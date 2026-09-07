use crate::MuxlaneServer;
use anyhow::Context as _;
use muxlane_core::detect::ScreenInput;
use muxlane_core::model::{AgentId, AgentInstance, AgentStatus, Project};
use muxlane_store::PersistedApp;
use std::sync::Arc;
use std::time::Duration;

impl MuxlaneServer {
    pub fn start_supervisor(self: &Arc<Self>) {
        let server = Arc::clone(self);
        self.rt_spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                server.maintain_sessions().await;
            }
        });
    }

    pub async fn restore_sessions(&self, persisted: &PersistedApp) {
        // 并发 attach：每个会话的 tmux 探活/attach 都是独立子进程 + spawn_blocking，
        // 串行恢复会让首窗时间随会话数线性放大。
        let mut pending = Vec::new();
        for saved in &persisted.sessions {
            let Some(project) = persisted
                .projects
                .iter()
                .find(|project| project.id == saved.project_id)
                .cloned()
            else {
                continue;
            };
            pending.push(async move {
                if !tmux_session_alive(&saved.tmux_session).await {
                    return None;
                }
                self.attach_tmux(
                    &saved.agent_id,
                    saved.agent_type,
                    &saved.title,
                    &saved.tmux_session,
                    &project,
                )
                .await
                .ok()
                .map(|(instance, session)| (project, instance, session))
            });
        }
        for restored in futures::future::join_all(pending).await {
            if let Some((project, instance, session)) = restored {
                self.restore_agent(project, instance, session).await;
            }
        }
    }

    pub async fn maintain_sessions(&self) {
        let sessions: Vec<_> = self
            .sessions
            .lock()
            .await
            .iter()
            .map(|(id, session)| (id.clone(), Arc::clone(session)))
            .collect();
        let mut exited = Vec::new();
        let mut recovered = Vec::new();
        let mut screens = Vec::new();

        for (id, session) in sessions {
            if session.try_take_exit().is_some() {
                if let Some(name) = session.tmux_session_name() {
                    if tmux_session_alive(name).await {
                        recovered.push((id, name.to_string()));
                        continue;
                    }
                }
                exited.push(id);
                continue;
            }
            // 屏幕采样只需要尾部（8 行上下文 + OSC 标题）：锁内只拷尾部 16KB，
            // 避免每 500ms × 每会话全量拷贝 512KB replay。
            let tail = session.replay_tail(16 * 1024);
            let truncated = tail.len() == 16 * 1024;
            let tail = &tail[..];
            let mut lines = muxlane_core::protocol::strip_ansi(tail);
            // 截断可能从转义序列中间切开：丢弃首行残留，避免乱码进入采样。
            if truncated && !lines.is_empty() {
                lines.remove(0);
            }
            if lines.len() > 8 {
                lines = lines.split_off(lines.len() - 8);
            }
            screens.push((
                id,
                ScreenInput {
                    bottom_lines: lines,
                    osc_title: muxlane_core::protocol::extract_osc_title(tail),
                    secs_since_output: session.secs_since_output(),
                    bell: tail.last() == Some(&0x07),
                },
            ));
        }

        for (id, tmux_session) in recovered {
            if let Err(error) = self.recover_session(&id, tmux_session).await {
                tracing::warn!(agent = %id, %error, "reattach live tmux session failed");
            }
        }
        self.remove_exited_sessions(&exited).await;
        self.observe_screens(&screens).await;
    }

    async fn recover_session(&self, id: &AgentId, tmux_session: String) -> anyhow::Result<()> {
        let (agent, project) = {
            let state = self.state.read().await;
            let agent = state
                .agents
                .iter()
                .find(|agent| &agent.id == id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("agent state missing"))?;
            let project = state
                .projects
                .iter()
                .find(|project| project.id == agent.project)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("project state missing"))?;
            (agent, project)
        };
        let (_, session) = self
            .attach_tmux(
                &agent.id,
                agent.agent_type,
                &agent.title,
                &tmux_session,
                &project,
            )
            .await?;
        self.sessions.lock().await.insert(id.clone(), session);
        Ok(())
    }

    async fn attach_tmux(
        &self,
        agent_id: &AgentId,
        agent_type: muxlane_core::model::AgentType,
        title: &str,
        tmux_session: &str,
        project: &Project,
    ) -> anyhow::Result<(AgentInstance, Arc<muxlane_term::PtySession>)> {
        let cfg = muxlane_term::LaunchCfg {
            agent: agent_id.clone(),
            agent_type,
            cwd: project.path.clone(),
            env: vec![
                ("MUXLANE_AGENT_ID".into(), agent_id.clone()),
                (
                    "MUXLANE_SOCKET".into(),
                    self.socket_path().display().to_string(),
                ),
                ("MUXLANE_HOOK_TOKEN".into(), self.hook_token(agent_id)),
            ],
            program_override: None,
            args: vec![],
            cols: 120,
            rows: 32,
            tmux_session: Some(tmux_session.to_string()),
        };
        let session = tokio::task::spawn_blocking(move || muxlane_term::PtySession::spawn(cfg))
            .await
            .context("attach tmux task")??;
        let instance = AgentInstance {
            id: agent_id.clone(),
            project: project.id.clone(),
            agent_type,
            title: title.to_string(),
            status: AgentStatus::Idle,
            status_since: muxlane_core::model::now_secs(),
            seen: true,
            tmux_session: Some(tmux_session.to_string()),
        };
        Ok((instance, session))
    }

    async fn remove_exited_sessions(&self, exited: &[AgentId]) {
        if exited.is_empty() {
            return;
        }
        let mut sessions = self.sessions.lock().await;
        for id in exited {
            sessions.remove(id);
        }
        drop(sessions);
        let mut subs = self.subs.lock().await;
        for id in exited {
            subs.mark_agent_exit(id);
        }
        drop(subs);
        let mut state = self.state.write().await;
        for id in exited {
            for event in state.agent_exit(id) {
                let _ = self.events.send(event);
            }
        }
        drop(state);
        self.dirty.bump();
        if let Err(error) = self.persist_runtime_state().await {
            tracing::warn!(%error, "persist exited sessions failed");
        }
    }

    async fn observe_screens(&self, screens: &[(AgentId, ScreenInput)]) {
        if screens.is_empty() {
            return;
        }
        let mut state = self.state.write().await;
        let mut changed = false;
        for (id, screen) in screens {
            if !state.observe_screen(id, screen).is_empty() {
                changed = true;
            }
        }
        drop(state);
        if changed {
            self.dirty.bump();
        }
    }
}

async fn tmux_session_alive(name: &str) -> bool {
    let name = name.to_string();
    tokio::task::spawn_blocking(move || {
        let target = format!("={name}");
        std::process::Command::new("tmux")
            .args(["-L", "muxlane", "has-session", "-t", &target])
            .status()
            .is_ok_and(|status| status.success())
    })
    .await
    .unwrap_or(false)
}
