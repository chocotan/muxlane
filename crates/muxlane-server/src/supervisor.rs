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
                    saved.title_is_custom,
                    &saved.tmux_session,
                    &project,
                )
                .await
                .ok()
                .map(|(instance, session)| (project, instance, session))
            });
        }
        for (project, instance, session) in futures::future::join_all(pending)
            .await
            .into_iter()
            .flatten()
        {
            self.restore_agent_quiet(project, instance, session).await;
        }
        for saved in &persisted.chat_sessions {
            let Some(project) = persisted
                .projects
                .iter()
                .find(|project| project.id == saved.project_id)
                .cloned()
            else {
                continue;
            };
            if self
                .state
                .read()
                .await
                .agents
                .iter()
                .any(|a| a.id == saved.agent_id)
            {
                continue;
            }
            let spec = muxlane_chat::DriverLaunchSpec {
                agent: saved.agent_id.clone(),
                agent_type: saved.agent_type,
                driver_kind: saved.driver_kind,
                program: saved.program.clone(),
                args: saved.args.clone(),
                cwd: project.path.clone(),
                env: Vec::new(),
                provider_session: saved
                    .provider_session_file
                    .clone()
                    .or_else(|| saved.provider_session_id.clone()),
                model: None,
                thinking_level: None,
            };
            self.chat
                .restore_session(
                    spec,
                    saved.provider_session_id.clone(),
                    saved.provider_session_file.clone(),
                )
                .await;
            let instance = AgentInstance {
                id: saved.agent_id.clone(),
                project: saved.project_id.clone(),
                agent_type: saved.agent_type,
                title: saved.title.clone(),
                title_is_custom: saved.title_is_custom,
                status: AgentStatus::Idle,
                status_since: muxlane_core::model::now_secs(),
                seen: true,
                tmux_session: None,
            };
            self.state.write().await.add_agent(project, instance);
        }
        // Attach live PTY bridges immediately for all restored Pi/Pie/OpenCode/etc. tmux sessions.
        self.ensure_all_pty_bridges().await;
        // One notification for the whole batch: the UI reconciles against a complete
        // snapshot instead of a half-restored one.
        self.bump_dirty();
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
        let foregrounds = self.sample_foregrounds_if_due(&screens).await;
        self.observe_screens(&screens, &foregrounds).await;
    }

    /// One shared sampler for the periodic tick and hook path. Sampling happens
    /// while a Shell-origin or Agy session exists, at most once per second.
    async fn sample_foregrounds_if_due(
        &self,
        screens: &[(AgentId, ScreenInput)],
    ) -> crate::foreground::ForegroundSample {
        const INTERVAL: Duration = Duration::from_millis(1000);
        if screens.is_empty() {
            return Default::default();
        }
        let needs_sample = {
            let state = self.state.read().await;
            screens.iter().any(|(id, _)| {
                state.is_shell_origin(id)
                    || state.agents.iter().any(|agent| {
                        &agent.id == id && agent.agent_type == muxlane_core::model::AgentType::Agy
                    })
            })
        };
        if !needs_sample {
            return Default::default();
        }
        let mut last = self.foreground_sampled_at.lock().await;
        if last.is_some_and(|instant| instant.elapsed() < INTERVAL) {
            return Default::default();
        }
        let sample = crate::foreground::sample_all().await;
        *last = Some(std::time::Instant::now());
        sample
    }

    /// A first hook can arrive before the periodic sampler; resolve identity before
    /// emitting its notification. Native Agent presets need no additional query.
    pub(crate) async fn refresh_shell_identity_for_hook(&self, id: &AgentId) {
        let name = {
            let state = self.state.read().await;
            state
                .agents
                .iter()
                .find(|agent| &agent.id == id)
                .filter(|agent| agent.agent_type == muxlane_core::AgentType::Shell)
                .and_then(|agent| agent.tmux_session.clone())
        };
        let Some(name) = name else {
            return;
        };
        // Only the first hook of a fresh Shell-origin session needs an early sample;
        // once identity is known the periodic sampler owns it.
        let mut last = self.foreground_sampled_at.lock().await;
        if last.is_some_and(|instant| instant.elapsed() < Duration::from_millis(1000)) {
            return;
        }
        let foregrounds = crate::foreground::sample().await;
        *last = Some(std::time::Instant::now());
        drop(last);
        let Some(foreground) = foregrounds.get(&name) else {
            return;
        };
        let title = self.sessions.lock().await.get(id).and_then(|session| {
            muxlane_core::protocol::extract_osc_title(&session.replay_tail(16 * 1024))
        });
        if self
            .state
            .write()
            .await
            .observe_foreground(id, foreground, title.as_deref())
        {
            if foreground.agent_type == muxlane_core::model::AgentType::Shell {
                self.chat.set_pty_bridge_alive(id, false).await;
            } else if foreground.agent_type.supports_chat_bridge() {
                let _ = self.ensure_pty_bridge_for_agent(id).await;
            }
            self.dirty.bump();
        }
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
                agent.title_is_custom,
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
        title_is_custom: bool,
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
            title_is_custom,
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
        for id in exited {
            self.chat.set_pty_bridge_alive(id, false).await;
            self.chat.remove_session(id).await;
        }
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

    async fn observe_screens(
        &self,
        screens: &[(AgentId, ScreenInput)],
        sample: &crate::foreground::ForegroundSample,
    ) {
        if screens.is_empty() {
            return;
        }
        let mut state = self.state.write().await;
        let mut changed = false;
        let mut title_updated = false;
        let mut foreground_transitions: Vec<(AgentId, muxlane_core::model::AgentType)> = Vec::new();
        let mut idle_transitions: Vec<AgentId> = Vec::new();
        for (id, screen) in screens {
            let tmux_name = state
                .agents
                .iter()
                .find(|agent| &agent.id == id)
                .and_then(|agent| agent.tmux_session.clone());
            let foreground = tmux_name
                .as_deref()
                .and_then(|name| sample.shell_foregrounds.get(name));
            if let Some(foreground) = foreground {
                if state.observe_foreground(id, foreground, screen.osc_title.as_deref()) {
                    changed = true;
                    foreground_transitions.push((id.clone(), foreground.agent_type));
                    continue; // The sampled screen may still belong to the previous process.
                }
            }
            if let Some(agy_title) = tmux_name
                .as_deref()
                .and_then(|name| sample.agy_titles.get(name))
            {
                if state.apply_chat_title(id, agy_title.clone()) {
                    changed = true;
                    title_updated = true;
                }
            }
            let events = state.observe_screen(id, screen);
            if !events.is_empty() {
                changed = true;
                for ev in &events {
                    if ev.event == muxlane_core::protocol::events::AGENT_STATUS {
                        if let Ok(status_ev) = serde_json::from_value::<
                            muxlane_core::protocol::AgentStatusEvent,
                        >(ev.params.clone())
                        {
                            if matches!(
                                status_ev.to,
                                muxlane_core::model::AgentStatus::Done
                                    | muxlane_core::model::AgentStatus::Idle
                                    | muxlane_core::model::AgentStatus::Failed
                            ) {
                                idle_transitions.push(id.clone());
                            }
                        }
                    }
                }
            }
        }
        drop(state);
        for (id, next_type) in foreground_transitions {
            if next_type == muxlane_core::model::AgentType::Shell {
                self.chat.set_pty_bridge_alive(&id, false).await;
            } else if next_type.supports_chat_bridge() {
                let _ = self.ensure_fresh_pty_bridge_for_agent(&id).await;
                self.chat.set_pty_bridge_alive(&id, true).await;
            }
        }
        for id in idle_transitions {
            self.chat.mark_pty_bridge_idle(&id).await;
        }
        if changed {
            self.dirty.bump();
            if title_updated {
                let _ = self.persist_runtime_state().await;
            }
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
