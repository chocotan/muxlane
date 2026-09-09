//! Agent session opening, focus, deletion, and terminal caching.
use crate::app::palette::NewSessionTarget;
use crate::app::MuxlaneApp;
use crate::i18n;
use crate::term_view::TermView;
use crate::theme::Theme;
use crate::workspace::ProjectKey;
use gpui::{App, AppContext, Context, Entity, Focusable, Window};
use muxlane_core::model::{AgentId, Snapshot};
use muxlane_term::VTerm;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq)]
enum TerminalLocation {
    Local,
    Remote(String),
}

fn terminal_location(
    local: &Snapshot,
    remotes: &HashMap<String, Snapshot>,
    agent: &AgentId,
) -> Option<TerminalLocation> {
    if local.agent(agent).is_some() {
        return Some(TerminalLocation::Local);
    }
    remotes.iter().find_map(|(host, snapshot)| {
        snapshot
            .agent(agent)
            .is_some()
            .then(|| TerminalLocation::Remote(host.clone()))
    })
}

impl MuxlaneApp {
    pub(crate) fn session_summary(
        &self,
        agent: &AgentId,
    ) -> Option<(String, muxlane_core::model::AgentStatus, bool)> {
        self.find_agent_terminal(agent)
            .map(|a| (a.title, a.status, a.seen))
    }

    fn find_agent_terminal(&self, agent: &AgentId) -> Option<muxlane_core::model::AgentInstance> {
        self.last_snapshot.agent(agent).cloned().or_else(|| {
            self.remote_snaps
                .values()
                .find_map(|snapshot| snapshot.agent(agent).cloned())
        })
    }

    pub(crate) fn mark_agent_working(&mut self, agent: &AgentId, cx: &mut Context<Self>) {
        let mut local = false;
        if let Some(a) = self.last_snapshot.agent_mut(agent) {
            local = true;
            if a.status != muxlane_core::model::AgentStatus::Working {
                a.status = muxlane_core::model::AgentStatus::Working;
                a.status_since = muxlane_core::model::now_secs();
                cx.notify();
            }
        } else {
            // 远程没有 mark_working RPC，先更新本地镜像，避免输入后仍显示 Idle。
            for snapshot in self.remote_snaps.values_mut() {
                if let Some(a) = snapshot.agent_mut(agent) {
                    a.status = muxlane_core::model::AgentStatus::Working;
                    a.status_since = muxlane_core::model::now_secs();
                    cx.notify();
                    break;
                }
            }
        }
        // 与屏幕采样同走 DetectionEngine：既避免与引擎内部状态互斥（否则引擎
        // 推导出的候选 idle 会因等于陈旧内部状态而永不提交，spinner 卡死），
        // 又保证 Idle 状态下输入命令立即显示 working 反馈。
        let agent_id = agent.clone();
        if local {
            let server = Arc::clone(&self.server);
            server.rt_spawn({
                let server = Arc::clone(&server);
                async move { server.mark_working(&agent_id).await }
            });
        }
    }

    pub(crate) fn create_local_term(
        agent: AgentId,
        session: Arc<muxlane_term::PtySession>,
        font_family: &str,
        theme: Theme,
        osc52_clipboard_enabled: bool,
        cx: &mut Context<Self>,
    ) -> Entity<TermView> {
        let font_family = font_family.to_string();
        let term = cx.new(|cx| {
            TermView::new_local(
                agent.clone(),
                session,
                font_family,
                theme,
                osc52_clipboard_enabled,
                cx,
            )
        });
        cx.subscribe(
            &term,
            |this, _term, ev: &crate::term_view::TermEnterEvent, cx| {
                this.mark_agent_working(&ev.0, cx);
            },
        )
        .detach();
        cx.subscribe(
            &term,
            |this, _, event: &crate::term_view::TermFocusEvent, cx| {
                this.handle_terminal_focus(event, cx);
            },
        )
        .detach();
        term
    }

    pub(crate) fn create_remote_term(
        agent: AgentId,
        terminal: (
            VTerm,
            tokio::sync::mpsc::UnboundedReceiver<muxlane_term::TermSideEffect>,
        ),
        remote_input: tokio::sync::mpsc::UnboundedSender<crate::term_view::RemoteTermCommand>,
        font_family: &str,
        theme: Theme,
        osc52_clipboard_enabled: bool,
        cx: &mut Context<Self>,
    ) -> Entity<TermView> {
        let font_family = font_family.to_string();
        let term = cx.new(|cx| {
            TermView::new_remote(
                agent.clone(),
                terminal,
                remote_input,
                font_family,
                theme,
                osc52_clipboard_enabled,
                cx,
            )
        });
        cx.subscribe(
            &term,
            |this, _term, ev: &crate::term_view::TermEnterEvent, cx| {
                this.mark_agent_working(&ev.0, cx);
            },
        )
        .detach();
        cx.subscribe(
            &term,
            |this, _, event: &crate::term_view::TermFocusEvent, cx| {
                this.handle_terminal_focus(event, cx);
            },
        )
        .detach();
        term
    }

    fn ensure_local_terminal(&mut self, agent: &AgentId, cx: &mut Context<Self>) -> bool {
        if self.terms.contains_key(agent) {
            return true;
        }
        let Some(sess) = self.server.try_session(agent) else {
            return false;
        };
        let term = Self::create_local_term(
            agent.clone(),
            sess,
            &self.font_family,
            Theme::for_mode(self.theme_mode),
            self.osc52_clipboard_enabled,
            cx,
        );
        self.terms.insert(agent.clone(), term);
        true
    }

    fn target_pane_for_agent(&self, agent: &AgentId) -> muxlane_core::PaneId {
        self.pane_tree
            .pane_for_agent(agent)
            .unwrap_or_else(|| self.active_pane.clone())
    }

    pub(crate) fn open_agent(
        &mut self,
        agent: &AgentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane = self.target_pane_for_agent(agent);
        self.activate_agent(&pane, agent, window, cx);
    }

    fn ensure_remote_terminal(&mut self, agent: &AgentId, cx: &mut Context<Self>) -> bool {
        if !self.terms.contains_key(agent) {
            let (vterm, clipboard_rx) = VTerm::new_with_clipboard(120, 32);
            vterm.feed(
                format!(
                    "\u{1b}[2m{}\u{1b}[0m\r\n",
                    i18n::text(self.language, "terminal.attaching")
                )
                .as_bytes(),
            );
            let (command_tx, mut command_rx) = tokio::sync::mpsc::unbounded_channel();
            let term = Self::create_remote_term(
                agent.clone(),
                (vterm.clone(), clipboard_rx),
                command_tx,
                &self.font_family,
                Theme::for_mode(self.theme_mode),
                self.osc52_clipboard_enabled,
                cx,
            );
            let weak_term = term.downgrade();
            self.terms.insert(agent.clone(), term);
            // agent → 所属 RemoteHost，禁止全局 endpoint 串台。
            let host_name = self
                .remote_snaps
                .iter()
                .find(|(_, snap)| snap.agents.iter().any(|a| &a.id == agent))
                .map(|(host, _)| host.clone());
            let remote = host_name
                .and_then(|name| self.remotes.iter().find(|h| h.cfg.name == name).cloned());
            if let Some(remote) = remote {
                let (mirror_notify, mut mirror_notify_rx) = tokio::sync::mpsc::channel(1);
                cx.spawn(async move |_this, cx| {
                    while mirror_notify_rx.recv().await.is_some() {
                        if weak_term.update(cx, |_term, cx| cx.notify()).is_err() {
                            break;
                        }
                    }
                })
                .detach();

                let command_remote = Arc::clone(&remote);
                let command_agent = agent.clone();
                let command_vterm = vterm.clone();
                self.server.rt_spawn(async move {
                    while let Some(first) = command_rx.recv().await {
                        let mut input = Vec::new();
                        let mut resize = None;
                        let mut collect =
                            |command: crate::term_view::RemoteTermCommand| match command {
                                crate::term_view::RemoteTermCommand::Input(bytes) => {
                                    input.extend(bytes)
                                }
                                crate::term_view::RemoteTermCommand::Resize(cols, rows) => {
                                    resize = Some((cols, rows));
                                }
                            };
                        collect(first);
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                        while let Ok(command) = command_rx.try_recv() {
                            collect(command);
                        }
                        if !input.is_empty() {
                            if let Err(error) =
                                command_remote.send_term_input(&command_agent, &input).await
                            {
                                command_vterm.feed(
                                    format!("\r\n\x1b[31mremote input failed: {error}\x1b[0m\r\n")
                                        .as_bytes(),
                                );
                            }
                        }
                        if let Some((cols, rows)) = resize {
                            let _ = command_remote.resize_term(&command_agent, cols, rows).await;
                        }
                    }
                });

                let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
                self.mirror_cancel
                    .insert(agent.clone(), Arc::clone(&cancelled));
                let agent2 = agent.clone();
                let vterm2 = vterm.clone();
                let language = self.language;
                self.server.rt_spawn(async move {
                    let mut backoff = 250u64;
                    loop {
                        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                            break;
                        }
                        let Some(sock) = remote.endpoint_now() else {
                            tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
                            backoff = (backoff * 2).min(5_000);
                            continue;
                        };
                        let vterm3 = vterm2.clone();
                        let notify = mirror_notify.clone();
                        let result = muxlane_client::stream_term(&sock, &agent2, move |update| {
                            match update {
                                muxlane_client::TermUpdate::Resync(bytes) => {
                                    vterm3.feed(b"\x1bc");
                                    // 历史回放，里面的终端查询早已被回答过，不能再答一次。
                                    vterm3.feed_silent(&bytes);
                                }
                                muxlane_client::TermUpdate::Data(bytes) => vterm3.feed(&bytes),
                            }
                            let _ = notify.try_send(());
                        })
                        .await;
                        if result.is_ok() || cancelled.load(std::sync::atomic::Ordering::Acquire) {
                            break;
                        }
                        vterm2.feed(
                            format!(
                                "\u{1b}[31m{}\u{1b}[0m\r\n",
                                i18n::text(language, "terminal.reconnecting")
                            )
                            .as_bytes(),
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
                        backoff = (backoff * 2).min(5_000);
                    }
                });
            }
        }
        self.terms.contains_key(agent)
    }

    pub(crate) fn ensure_agent_terminal(
        &mut self,
        agent: &AgentId,
        cx: &mut Context<Self>,
    ) -> bool {
        match terminal_location(&self.last_snapshot, &self.remote_snaps, agent) {
            Some(TerminalLocation::Local) => self.ensure_local_terminal(agent, cx),
            Some(TerminalLocation::Remote(_)) => self.ensure_remote_terminal(agent, cx),
            None => false,
        }
    }

    pub(crate) fn ensure_active_terminal(&mut self, cx: &mut Context<Self>) {
        if let Some(active) = self.active.clone() {
            self.ensure_agent_terminal(&active, cx);
        }
    }

    pub(crate) fn sync_active_terminal_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.terminal_focus_suspended(cx) {
            return;
        }
        let Some(focus) = self
            .active
            .as_ref()
            .filter(|agent| !self.is_detached(agent))
            .and_then(|agent| self.terms.get(agent))
            .map(|term| term.focus_handle(cx))
        else {
            return;
        };
        if focus.is_focused(window) {
            return;
        }
        let focus_is_managed = window.focused(cx).is_none()
            || self.focus.is_focused(window)
            || self
                .terms
                .iter()
                // A detached session's handle lives in its own native window; it must not
                // count as "managed" here or the two windows fight over the shared handle.
                .filter(|(agent, _)| !self.is_detached(agent))
                .any(|(_, term)| term.focus_handle(cx).is_focused(window))
            || [
                &self.palette_input,
                &self.connect_input,
                &self.connect_username,
                &self.connect_password,
                &self.connect_key_path,
                &self.project_input,
                &self.remote_project_input,
            ]
            .into_iter()
            .any(|input| input.focus_handle(cx).is_focused(window));
        if focus_is_managed {
            focus.focus(window, cx);
            window.invalidate_character_coordinates();
        }
    }

    pub(crate) fn terminal_focus_suspended(&self, cx: &App) -> bool {
        self.palette_open
            || self.connect_dialog
            || self.project_dialog
            || self.remote_project_dialog.is_some()
            || self.settings_open
            || self.session_menu.is_some()
            || self.tree_menu.is_some()
            || self.delete_confirm.is_some()
            || self.pending_project_creation.is_some()
            || self.bootstrap_confirm.is_some()
            || self.notifications.read(cx).summary().2
            || self.split_drag.is_some()
            || self.sidebar.drag.is_some()
    }

    pub(crate) fn activate_agent(
        &mut self,
        pane: &muxlane_core::PaneId,
        agent: &AgentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_detached(agent) {
            self.focus_detached_window(agent, cx);
            return;
        }
        self.activate_tab(pane, agent, cx);
        if self.ensure_agent_terminal(agent, cx) {
            self.focus_agent(agent, window, cx);
        }
        cx.notify();
    }

    pub(crate) fn focus_agent(
        &mut self,
        agent: &AgentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A detached session only takes focus from its own native window.
        if self.is_detached(agent)
            && !self
                .floating
                .windows
                .get(agent)
                .is_some_and(|handle| handle.window_id() == window.window_handle().window_id())
        {
            return;
        }
        let focus = self.terms.get(agent).map(|term| term.focus_handle(cx));
        if let Some(focus) = &focus {
            focus.focus(window, cx);
            window.invalidate_character_coordinates();
            tracing::debug!(
                agent = %agent,
                handle_focused = focus.is_focused(window),
                active = ?self.active,
                "focus agent requested"
            );
        }
        self.mark_agent_seen(agent, cx);
    }

    pub(crate) fn mark_agent_seen(&mut self, agent: &AgentId, cx: &mut Context<Self>) {
        // 清理当前 agent 的 Toast 与标记通知已读
        self.notifications
            .update(cx, |center, cx| center.mark_agent_read(agent, cx));
        if let Some(a) = self.last_snapshot.agent_mut(agent) {
            a.seen = true;
            if a.status.is_finished() {
                a.status = muxlane_core::model::AgentStatus::Idle;
            }
        } else {
            // 远端支持 agent.mark_seen 时真正写回服务端（Done/Failed→Idle），随远端
            // 自身持久化，不怕本地客户端重启丢失；旧版本远端不支持时降级为仅本
            // 地标记已读，且不能同步翻转 status：服务端仍保留 Done/Failed，
            // 下一次全量快照刷新时若本地 status 与服务端不一致，合并会误
            // 判为新结果而重新提醒。
            let host_name = self
                .remote_snaps
                .iter()
                .find(|(_, snap)| snap.agents.iter().any(|a| &a.id == agent))
                .map(|(host, _)| host.clone());
            let remote = host_name.as_ref().and_then(|host_name| {
                self.remotes
                    .iter()
                    .find(|host| host.cfg.name == *host_name)
                    .cloned()
            });
            if let Some(remote) = remote
                .filter(|remote| remote.supports(muxlane_core::protocol::features::AGENT_MARK_SEEN))
            {
                let agent_for_rpc = agent.clone();
                let task = self.spawn_remote_operation(async move {
                    remote.mark_agent_seen(&agent_for_rpc).await
                });
                cx.spawn(async move |_, _| {
                    // 最大努力：失败不阻塞 UI，下次查看会重试。
                    let _ = task.await;
                })
                .detach();
            }
            for snapshot in self.remote_snaps.values_mut() {
                if let Some(a) = snapshot.agent_mut(agent) {
                    a.seen = true;
                    break;
                }
            }
        }
        if self.last_snapshot.agent(agent).is_some() {
            let server = Arc::clone(&self.server);
            let agent = agent.clone();
            server.rt_spawn({
                let server = Arc::clone(&server);
                async move { server.mark_seen(&agent).await }
            });
        }
        cx.notify();
    }

    pub(crate) fn jump_to_agent(
        &mut self,
        agent: &AgentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.find_agent_terminal(agent).is_none() {
            return;
        }
        if let Some(key) = self.project_key_for_agent(agent) {
            self.select_project_workspace_inner(key, cx);
        }
        self.open_agent(agent, window, cx);
    }

    pub(crate) fn request_session_delete(
        &mut self,
        agent: &AgentId,
        remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_session(agent, remote, window, cx);
    }

    pub(crate) fn delete_session(
        &mut self,
        agent: &AgentId,
        remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if remote {
            let host_name = self
                .remote_snaps
                .iter()
                .find(|(_, snap)| snap.agents.iter().any(|a| &a.id == agent))
                .map(|(host, _)| host.clone());
            let remote = host_name.as_ref().and_then(|host_name| {
                self.remotes
                    .iter()
                    .find(|host| host.cfg.name == *host_name)
                    .cloned()
            });
            let (Some(host_name), Some(remote)) = (host_name, remote) else {
                self.notifications.update(cx, |center, cx| {
                    center.show_error(
                        i18n::text(self.language, "error.remote_unavailable_for_delete").into(),
                        cx,
                    )
                });
                cx.notify();
                return;
            };
            let agent = agent.clone();
            let agent_for_delete = agent.clone();
            let task =
                self.spawn_remote_operation(
                    async move { remote.delete_agent(&agent_for_delete).await },
                );
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                let _ = this.update_in(cx, |this, window, cx| match result {
                    Ok(()) => {
                        if let Some(snapshot) = this.remote_snaps.get_mut(&host_name) {
                            snapshot.agents.retain(|candidate| candidate.id != agent);
                            for project in &mut snapshot.projects {
                                project.agents.retain(|candidate| candidate != &agent);
                            }
                        }
                        this.finish_delete_session(&agent, window, cx);
                    }
                    Err(error) => {
                        this.notifications.update(cx, |center, cx| {
                            center.show_error(
                                i18n::text(this.language, "error.delete_session")
                                    .replace("{error}", &error.to_string()),
                                cx,
                            )
                        });
                        cx.notify();
                    }
                });
            })
            .detach();
            return;
        }

        let server = Arc::clone(&self.server);
        let agent = agent.clone();
        cx.spawn_in(window, async move |this, cx| {
            let agent_for_delete = agent.clone();
            let (result, snapshot) = cx
                .background_spawn(async move {
                    let result = server.delete_agent(&agent_for_delete).await;
                    let snapshot = server.snapshot().await;
                    (result, snapshot)
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.last_snapshot = snapshot;
                match result {
                    Ok(result) if result.failed_agents.is_empty() => {
                        this.finish_delete_session(&agent, window, cx);
                    }
                    Ok(result) => {
                        this.notifications.update(cx, |center, cx| {
                            center.show_error(
                                i18n::text(this.language, "error.delete_sessions_session")
                                    .replace("{count}", &result.failed_agents.len().to_string()),
                                cx,
                            )
                        });
                        cx.notify();
                    }
                    Err(error) => {
                        if this.last_snapshot.agent(&agent).is_none() {
                            this.finish_delete_session(&agent, window, cx);
                        }
                        this.notifications.update(cx, |center, cx| {
                            center.show_error(
                                i18n::text(this.language, "error.delete_session")
                                    .replace("{error}", &error.to_string()),
                                cx,
                            )
                        });
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub(crate) fn finish_delete_session(
        &mut self,
        agent: &AgentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let removed = std::collections::HashSet::from([agent.clone()]);
        self.floating.remove_agents(&removed);
        self.schedule_session_windows(cx);
        self.workspace.remove_agents(&removed);
        if let Some(pane) = self.pane_tree.pane_for_agent(agent) {
            self.pane_tree.close_tab(&pane, agent);
        }
        self.terms.remove(agent);
        if let Some(cancelled) = self.mirror_cancel.remove(agent) {
            cancelled.store(true, std::sync::atomic::Ordering::Release);
        }
        self.notifications
            .update(cx, |center, cx| center.remove_agent(agent, cx));
        if self.active.as_ref() == Some(agent) {
            self.active = self
                .pane_tree
                .group(&self.active_pane)
                .and_then(|group| group.active.clone());
        }
        self.session_menu = None;
        if let Some(active) = self.active.clone() {
            let pane = self.active_pane.clone();
            self.activate_agent(&pane, &active, window, cx);
        }
        self.persist();
        cx.notify();
    }

    pub(crate) fn cleanup_removed_agents(&mut self, removed: &[AgentId], cx: &mut Context<Self>) {
        let removed: std::collections::HashSet<_> = removed.iter().cloned().collect();
        self.floating.remove_agents(&removed);
        self.schedule_session_windows(cx);
        self.terms.retain(|agent, _| !removed.contains(agent));
        for agent in &removed {
            if let Some(cancelled) = self.mirror_cancel.remove(agent) {
                cancelled.store(true, std::sync::atomic::Ordering::Release);
            }
        }
        self.notifications
            .update(cx, |center, cx| center.remove_agents(&removed, cx));
        self.workspace.remove_agents(&removed);
        let valid: std::collections::HashSet<_> = self
            .last_snapshot
            .agents
            .iter()
            .map(|agent| agent.id.clone())
            .chain(
                self.remote_snaps
                    .values()
                    .flat_map(|snapshot| snapshot.agents.iter().map(|agent| agent.id.clone())),
            )
            .filter(|agent| !removed.contains(agent))
            .collect();
        self.pane_tree.retain_agents(&valid);
        if self
            .active
            .as_ref()
            .is_some_and(|agent| removed.contains(agent))
        {
            self.active = self
                .pane_tree
                .group(&self.active_pane)
                .and_then(|group| group.active.clone());
        }
        let clear_maximized = self
            .maximized_pane
            .as_ref()
            .and_then(|pane| self.pane_tree.group(pane))
            .map(|group| group.active.is_none())
            .unwrap_or(true);
        if clear_maximized {
            self.maximized_pane = None;
        }
        self.ensure_active_terminal(cx);
    }
}

impl MuxlaneApp {
    pub(crate) fn spawn_preset(
        &mut self,
        preset: &muxlane_core::AgentPreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.new_session_target.take();
        let target_key = match target {
            Some(NewSessionTarget::Remote { host, project }) => {
                self.palette_open = false;
                self.spawn_remote_agent(
                    crate::remotes::RemoteAgentSpawnRequest {
                        host,
                        project,
                        preset: Some(preset.clone()),
                        preferred_pane: None,
                        split_axis: None,
                    },
                    window,
                    cx,
                );
                return;
            }
            Some(NewSessionTarget::Local(project)) => {
                ProjectKey::new(self.local_machine_id(), project)
            }
            None => {
                self.workspace
                    .current_project()
                    .filter(|_| self.workspace.enabled())
                    .cloned()
                    .or_else(|| {
                        self.active
                            .as_ref()
                            .and_then(|agent| self.project_key_for_agent(agent))
                    })
                    .or_else(|| {
                        self.last_snapshot.projects.first().map(|project| {
                            ProjectKey::new(self.local_machine_id(), project.id.clone())
                        })
                    })
                    .unwrap_or_else(|| ProjectKey::new(self.local_machine_id(), String::new()))
            }
        };
        if target_key.project_id.is_empty() {
            return;
        }
        if target_key.machine_id != self.local_machine_id() {
            let Some(host) = self.remote_host_for_key(&target_key) else {
                return;
            };
            self.palette_open = false;
            self.spawn_remote_agent(
                crate::remotes::RemoteAgentSpawnRequest {
                    host,
                    project: target_key.project_id,
                    preset: Some(preset.clone()),
                    preferred_pane: None,
                    split_axis: None,
                },
                window,
                cx,
            );
            return;
        }
        let Some(project) = self.last_snapshot.project(&target_key.project_id).cloned() else {
            return;
        };
        let params = local_preset_params(preset, &project);
        let server = Arc::clone(&self.server);
        let preferred_pane = self.active_pane.clone();
        let pane = self.capture_spawn_target(&target_key, Some(&preferred_pane));
        let collapse_key = format!("local:{}", project.id);
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let agent = server.spawn_agent(params).await?;
                    let session = server
                        .session(&agent.id)
                        .await
                        .ok_or_else(|| anyhow::anyhow!("spawned agent has no session"))?;
                    Ok::<_, anyhow::Error>((agent, session))
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok((agent, session)) => {
                    if !this.project_owner_exists(&target_key)
                        || agent.project != target_key.project_id
                    {
                        return;
                    }
                    let agent_id = agent.id.clone();
                    let term = this.terms.get(&agent_id).cloned().unwrap_or_else(|| {
                        Self::create_local_term(
                            agent_id.clone(),
                            session,
                            &this.font_family,
                            Theme::for_mode(this.theme_mode),
                            this.osc52_clipboard_enabled,
                            cx,
                        )
                    });
                    if !this.last_snapshot.agents.iter().any(|a| a.id == agent_id) {
                        this.last_snapshot.agents.push(agent);
                    }
                    this.collapsed_projects.remove(&collapse_key);
                    this.terms.insert(agent_id.clone(), term);
                    this.palette_open = false;
                    this.new_session_target = None;
                    this.jump_to_project_if_needed(&target_key, cx);
                    this.place_async_agent(&target_key, agent_id, Some(pane), None, window, cx);
                }
                Err(error) => {
                    this.notifications.update(cx, |center, cx| {
                        center.show_error(
                            i18n::text(this.language, "error.create_session")
                                .replace("{error}", &error.to_string()),
                            cx,
                        )
                    });
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

pub(crate) fn local_preset_params(
    preset: &muxlane_core::AgentPreset,
    project: &muxlane_core::model::Project,
) -> muxlane_core::protocol::AgentSpawnParams {
    muxlane_core::protocol::AgentSpawnParams {
        project: project.id.clone(),
        agent_type: Some(preset.agent_type),
        program: (preset.agent_type != muxlane_core::model::AgentType::Shell).then(|| {
            preset
                .executable_in(&project.path)
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|| preset.program.clone())
        }),
        args: Some(preset.args.clone()),
        env: Some(preset.env.clone().into_iter().collect()),
        preset_name: Some(preset.label.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxlane_core::model::{AgentInstance, AgentStatus, AgentType};

    fn snapshot_with_agent(id: &str) -> Snapshot {
        Snapshot {
            agents: vec![AgentInstance {
                id: id.into(),
                project: "project".into(),
                agent_type: AgentType::Shell,
                title: "shell".into(),
                status: AgentStatus::Idle,
                status_since: 0,
                seen: true,
                tmux_session: Some(format!("muxlane-{id}")),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn terminal_location_distinguishes_local_remote_and_missing_agents() {
        let local = snapshot_with_agent("local-agent");
        let remotes = HashMap::from([("host".into(), snapshot_with_agent("remote-agent"))]);
        assert_eq!(
            terminal_location(&local, &remotes, &"local-agent".into()),
            Some(TerminalLocation::Local)
        );
        assert_eq!(
            terminal_location(&local, &remotes, &"remote-agent".into()),
            Some(TerminalLocation::Remote("host".into()))
        );
        assert_eq!(terminal_location(&local, &remotes, &"missing".into()), None);
    }

    #[test]
    fn local_terminal_preset_params_resolve_project_binary_without_changing_raw_preset() {
        let directory = tempfile::tempdir().unwrap();
        let bin_dir = directory.path().join("node_modules/.bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let executable = bin_dir.join("muxlane-test-preset");
        std::fs::write(&executable, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let project = muxlane_core::model::Project {
            id: "params-test".into(),
            name: "Params Test".into(),
            path: directory.path().into(),
            branch: None,
            agents: vec![],
        };
        let presets = muxlane_core::builtin_presets("/unused-shell");
        let shell = local_preset_params(&presets[0], &project);
        assert_eq!(shell.program, None);
        assert_eq!(shell.agent_type, Some(AgentType::Shell));
        for id in ["claude", "codex", "pi", "opencode"] {
            let mut preset = presets
                .iter()
                .find(|preset| preset.id == id)
                .unwrap()
                .clone();
            preset.program = "muxlane-test-preset".into();
            preset.args = vec!["--test".into(), "literal argument".into()];
            preset.env.insert("PRESET_TEST".into(), "value".into());
            let params = local_preset_params(&preset, &project);
            assert_eq!(params.project, project.id);
            assert_eq!(params.program.as_deref(), executable.to_str());
            assert_eq!(params.agent_type, Some(preset.agent_type));
            assert_eq!(params.preset_name, Some(preset.label.clone()));
            assert_eq!(params.args, Some(preset.args.clone()));
            assert_eq!(
                params.env,
                Some(vec![("PRESET_TEST".into(), "value".into())])
            );
            assert_eq!(preset.program, "muxlane-test-preset");
            preset.program = "muxlane-definitely-missing-test-preset".into();
            let missing = local_preset_params(&preset, &project);
            assert_eq!(missing.program, Some(preset.program.clone()));
            assert_eq!(missing.agent_type, Some(preset.agent_type));
        }
    }
}
