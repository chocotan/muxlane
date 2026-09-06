//! Agent session opening, focus, deletion, and terminal caching.
use crate::acp_composer::{ContextItem, ContextKind};
use crate::acp_view::{agent_status, AcpView, AcpViewEvent, AcpViewInit};
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

fn thread_context_text(record: &muxlane_store::PersistedAcpThreadData) -> String {
    let mut output = String::new();
    for item in &record.snapshot.items {
        let line = match item {
            muxlane_acp::ThreadItem::Message(message) => {
                format!("{:?}: {}\n", message.role, message.text)
            }
            muxlane_acp::ThreadItem::Content(content) => {
                format!("{:?}: [structured content]\n", content.role)
            }
            muxlane_acp::ThreadItem::Thought(_) => continue,
            muxlane_acp::ThreadItem::Tool(tool) => {
                format!("Tool: {} ({:?})\n", tool.title, tool.state)
            }
        };
        if output.len().saturating_add(line.len()) > 64 * 1024 {
            output.push_str("[thread context truncated]\n");
            break;
        }
        output.push_str(&line);
    }
    output
}

impl MuxlaneApp {
    pub(crate) fn is_acp_session(&self, agent: &AgentId) -> bool {
        self.acp_views.contains_key(agent)
    }

    pub(crate) fn session_summary(
        &self,
        agent: &AgentId,
        cx: &App,
    ) -> Option<(String, muxlane_core::model::AgentStatus, bool)> {
        if let Some(view) = self.acp_views.get(agent) {
            let view = view.read(cx);
            return Some((
                if view.title.is_empty() {
                    "ACP".into()
                } else {
                    view.title.clone()
                },
                agent_status(view.status),
                self.active.as_ref() == Some(agent),
            ));
        }
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

    pub(crate) fn register_acp_view(
        &mut self,
        id: AgentId,
        view: Entity<AcpView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let data = view.read(cx).thread_data();
        self.update_acp_record(id.clone(), data);
        cx.subscribe_in(&view, window, {
            let id = id.clone();
            move |this, _view, event: &AcpViewEvent, window, cx| match event {
                AcpViewEvent::ThreadChanged(data) => {
                    this.update_acp_record(id.clone(), data.as_ref().clone());
                    this.schedule_acp_persist(cx);
                    cx.notify();
                }
                AcpViewEvent::RefreshContexts => {
                    this.refresh_acp_thread_contexts(cx);
                }
                AcpViewEvent::OpenSubagent(session_id) => {
                    let parent_id = id.clone();
                    let session_id = session_id.clone();
                    cx.defer_in(window, move |this, window, cx| {
                        this.spawn_acp_subagent(&parent_id, session_id.clone(), window, cx);
                    });
                }
                AcpViewEvent::ImportSession { session_id, title } => {
                    let source_id = id.clone();
                    let session_id = session_id.clone();
                    let title = title.clone();
                    cx.defer_in(window, move |this, window, cx| {
                        this.spawn_acp_import(
                            &source_id,
                            session_id.clone(),
                            title.clone(),
                            window,
                            cx,
                        );
                    });
                }
                AcpViewEvent::ToggleMaximize => {
                    let id = id.clone();
                    cx.defer_in(window, move |this, _window, cx| {
                        if let Some(pane) = this.pane_tree.pane_for_agent(&id) {
                            this.toggle_maximize(&pane, cx);
                        }
                    });
                }
                AcpViewEvent::RestartRequested => {
                    let app = cx.entity().downgrade();
                    let id = id.clone();
                    cx.defer(move |cx| {
                        app.update(cx, |this, cx| {
                            this.ensure_acp_started(&id, cx);
                        })
                        .ok();
                    });
                }
            }
        })
        .detach();
        self.acp_views.insert(id, view);
        self.refresh_acp_thread_contexts(cx);
        self.schedule_acp_persist(cx);
    }

    fn refresh_acp_thread_contexts(&mut self, cx: &mut Context<Self>) {
        let records: Vec<_> = self
            .acp_records
            .values()
            .filter(|record| !record.archived)
            .cloned()
            .collect();
        let local_machine_id = self.local_machine_id();
        let terminal_contexts: Vec<_> = self
            .terms
            .iter()
            .filter_map(|(agent, terminal)| {
                let key = self.project_key_for_agent(agent)?;
                if key.machine_id != local_machine_id {
                    return None;
                }
                let terminal = terminal.read(cx);
                let mut content = terminal.vterm.selection_to_string().unwrap_or_else(|| {
                    let lines = terminal.vterm.text_lines();
                    lines[lines.len().saturating_sub(40)..].join("\n")
                });
                if content.len() > 64 * 1024 {
                    let mut boundary = 64 * 1024;
                    while !content.is_char_boundary(boundary) {
                        boundary = boundary.saturating_sub(1);
                    }
                    content.truncate(boundary);
                    content.push_str("\n[terminal context truncated]");
                }
                Some((
                    key.project_id,
                    ContextItem {
                        relative_path: format!("terminal:{agent}"),
                        path: std::path::PathBuf::new(),
                        kind: ContextKind::Terminal,
                        content: Some(content),
                    },
                ))
            })
            .collect();
        for (view_id, view) in &self.acp_views {
            let project_id = view.read(cx).project_id.clone();
            let mut contexts: Vec<_> = records
                .iter()
                .filter(|record| {
                    record.metadata.ui_id != *view_id && record.metadata.project_id == project_id
                })
                .map(|record| ContextItem {
                    relative_path: format!("thread:{}", record.metadata.title),
                    path: std::path::PathBuf::new(),
                    kind: ContextKind::Thread,
                    content: Some(thread_context_text(record)),
                })
                .collect();
            contexts.extend(
                terminal_contexts
                    .iter()
                    .filter(|(terminal_project, _)| terminal_project == &project_id)
                    .map(|(_, context)| context.clone()),
            );
            view.update(cx, |view, cx| view.set_thread_contexts(contexts, cx));
        }
    }

    fn update_acp_record(&mut self, id: AgentId, mut data: muxlane_store::PersistedAcpThreadData) {
        let is_deleted = self
            .acp_deleted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(&id);
        if is_deleted {
            return;
        }
        let changed = if let Some(existing) = self.acp_records.get(&id) {
            data.created_at = existing.created_at;
            data.archived = existing.archived;
            data.write_revision = existing.write_revision;
            data.updated_at = existing.updated_at;
            if data != *existing {
                data.write_revision = existing.write_revision.saturating_add(1);
                data.updated_at = muxlane_core::model::now_secs();
                true
            } else {
                false
            }
        } else {
            if data.write_revision == 0 {
                data.write_revision = 1;
            }
            true
        };
        self.acp_metadata.insert(id.clone(), data.metadata.clone());
        if changed {
            self.acp_dirty.insert(id.clone());
        }
        self.acp_records.insert(id, data);
    }

    fn schedule_acp_persist(&mut self, cx: &mut Context<Self>) {
        if self.acp_persist_pending {
            return;
        }
        self.acp_persist_pending = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(300))
                .await;
            this.update(cx, |this, cx| {
                this.acp_persist_pending = false;
                let deleted = this.acp_deleted.lock().ok();
                let dirty = std::mem::take(&mut this.acp_dirty);
                for id in dirty {
                    if deleted
                        .as_ref()
                        .is_some_and(|deleted| deleted.contains(&id))
                    {
                        continue;
                    }
                    if let Some(record) = this.acp_records.get(&id) {
                        this.persistence.upsert_acp(record.clone());
                    }
                }
                this.persist();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn ensure_acp_started(&mut self, agent: &AgentId, cx: &mut Context<Self>) -> bool {
        let Some(view) = self.acp_views.get(agent).cloned() else {
            return false;
        };
        let (profile, project_id, protocol_session_id, auth_method, started, start_allowed) = {
            let view = view.read(cx);
            (
                view.profile,
                view.project_id.clone(),
                view.protocol_session_id.clone(),
                view.pending_auth_method.clone(),
                view.handle.is_some(),
                view.start_allowed,
            )
        };
        if started {
            return true;
        }
        if !start_allowed {
            return false;
        }
        let (Some(profile), Some(project)) = (profile, self.last_snapshot.project(&project_id))
        else {
            return false;
        };
        let session = muxlane_acp::spawn_on_with_auth(
            &self.server.runtime_handle(),
            profile,
            &project.path,
            protocol_session_id,
            auth_method,
        );
        view.update(cx, |view, cx| view.start(session, cx));
        true
    }

    pub(crate) fn spawn_acp_view(
        &mut self,
        profile: muxlane_acp::Profile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project_id = match self.new_session_target.as_ref() {
            Some(NewSessionTarget::Local(project_id)) => project_id.clone(),
            _ => {
                self.palette_open = true;
                self.notifications.update(cx, |center, cx| {
                    center.show_error(
                        i18n::text(self.language, "error.local_project_required").into(),
                        cx,
                    )
                });
                cx.notify();
                return;
            }
        };
        self.new_session_target = None;
        self.spawn_acp_view_in_pane(project_id, profile, None, window, cx);
    }

    pub(crate) fn spawn_acp_view_in_pane(
        &mut self,
        project_id: String,
        profile: muxlane_acp::Profile,
        preferred_pane: Option<muxlane_core::PaneId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project_path) = self
            .last_snapshot
            .project(&project_id)
            .map(|project| project.path.clone())
        else {
            self.notifications.update(cx, |center, cx| {
                center.show_error(
                    i18n::text(self.language, "error.local_project_missing").into(),
                    cx,
                )
            });
            cx.notify();
            return;
        };
        let key = ProjectKey::new(self.local_machine_id(), project_id.clone());
        let preferred_pane = preferred_pane.unwrap_or_else(|| self.active_pane.clone());
        let pane = self.capture_spawn_target(&key, Some(&preferred_pane));
        let id = muxlane_core::model::new_id("acp");
        let title = format!("{} UI", profile.label());
        let view = cx.new(|cx| {
            AcpView::new(
                AcpViewInit {
                    ui_id: id.clone(),
                    project_id: project_id.clone(),
                    project_path,
                    profile: Some(profile),
                    protocol_session_id: None,
                    parent_ui_id: None,
                    title,
                    draft: String::new(),
                    snapshot: muxlane_acp::ThreadSnapshot::default(),
                    queued_prompts: Vec::new(),
                    queue_paused: false,
                    theme_mode: self.theme_mode,
                    language: self.language,
                },
                window,
                cx,
            )
        });
        self.register_acp_view(id.clone(), view, window, cx);
        self.collapsed_projects
            .remove(&format!("local:{project_id}"));
        self.palette_open = false;
        self.jump_to_project_if_needed(&key, cx);
        self.place_async_agent(&key, id, Some(pane), None, window, cx);
    }

    pub(crate) fn archive_acp_session(
        &mut self,
        agent: &AgentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(record) = self.acp_records.get_mut(agent) else {
            return;
        };
        record.archived = true;
        record.write_revision = record.write_revision.saturating_add(1);
        record.updated_at = muxlane_core::model::now_secs();
        self.acp_dirty.insert(agent.clone());
        if let Some(view) = self.acp_views.remove(agent) {
            view.update(cx, |view, _cx| view.shutdown());
        }
        let removed = std::collections::HashSet::from([agent.clone()]);
        self.workspace.remove_agents(&removed);
        if let Some(pane) = self.pane_tree.pane_for_agent(agent) {
            self.pane_tree.close_tab(&pane, agent);
        }
        self.acp_metadata.remove(agent);
        if self.active.as_ref() == Some(agent) {
            self.active = self
                .pane_tree
                .group(&self.active_pane)
                .and_then(|group| group.active.clone());
        }
        self.session_menu = None;
        self.schedule_acp_persist(cx);
        if let Some(active) = self.active.clone() {
            let pane = self.active_pane.clone();
            self.activate_agent(&pane, &active, window, cx);
        }
        self.persist();
        cx.notify();
    }

    pub(crate) fn unarchive_acp_session(
        &mut self,
        agent: &AgentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mut data) = self.acp_records.get(agent).cloned() else {
            return;
        };
        let Some(project) = self
            .last_snapshot
            .project(&data.metadata.project_id)
            .cloned()
        else {
            return;
        };
        let Some(profile) = muxlane_acp::Profile::from_id(&data.metadata.profile_id) else {
            return;
        };
        data.archived = false;
        data.write_revision = data.write_revision.saturating_add(1);
        data.updated_at = muxlane_core::model::now_secs();
        self.acp_dirty.insert(agent.clone());
        self.acp_records.insert(agent.clone(), data.clone());
        let key = ProjectKey::new(self.local_machine_id(), project.id.clone());
        let preferred_pane = self.active_pane.clone();
        let pane = self.capture_spawn_target(&key, Some(&preferred_pane));
        let metadata = data.metadata.clone();
        let view = cx.new(|cx| {
            AcpView::new(
                AcpViewInit {
                    ui_id: metadata.ui_id.clone(),
                    project_id: metadata.project_id.clone(),
                    project_path: project.path.clone(),
                    profile: Some(profile),
                    protocol_session_id: metadata.protocol_session_id.clone(),
                    parent_ui_id: metadata.parent_ui_id.clone(),
                    title: metadata.title.clone(),
                    draft: metadata.draft.clone(),
                    snapshot: data.snapshot.clone(),
                    queued_prompts: data.queued_prompts.clone(),
                    queue_paused: data.queue_paused,
                    theme_mode: self.theme_mode,
                    language: self.language,
                },
                window,
                cx,
            )
        });
        self.register_acp_view(agent.clone(), view, window, cx);
        self.jump_to_project_if_needed(&key, cx);
        self.place_async_agent(&key, agent.clone(), Some(pane), None, window, cx);
        self.schedule_acp_persist(cx);
    }

    pub(crate) fn spawn_acp_import(
        &mut self,
        source_id: &AgentId,
        protocol_session_id: String,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(existing_id) = self
            .acp_records
            .values()
            .find(|record| {
                record.metadata.protocol_session_id.as_deref() == Some(protocol_session_id.as_str())
            })
            .map(|record| record.metadata.ui_id.clone())
        {
            if self.acp_views.contains_key(&existing_id) {
                self.open_agent(&existing_id, window, cx);
            } else {
                self.unarchive_acp_session(&existing_id, window, cx);
            }
            return;
        }
        let Some(source) = self.acp_views.get(source_id).cloned() else {
            return;
        };
        let (project_id, project_path, profile) = {
            let source = source.read(cx);
            let Some(profile) = source.profile else {
                return;
            };
            (
                source.project_id.clone(),
                source.project_path.clone(),
                profile,
            )
        };
        let id = muxlane_core::model::new_id("acp");
        let view = cx.new(|cx| {
            AcpView::new(
                AcpViewInit {
                    ui_id: id.clone(),
                    project_id: project_id.clone(),
                    project_path,
                    profile: Some(profile),
                    protocol_session_id: Some(protocol_session_id),
                    parent_ui_id: None,
                    title,
                    draft: String::new(),
                    snapshot: muxlane_acp::ThreadSnapshot::default(),
                    queued_prompts: Vec::new(),
                    queue_paused: false,
                    theme_mode: self.theme_mode,
                    language: self.language,
                },
                window,
                cx,
            )
        });
        self.register_acp_view(id.clone(), view, window, cx);
        let key = ProjectKey::new(self.local_machine_id(), project_id);
        let pane = self.active_pane.clone();
        self.jump_to_project_if_needed(&key, cx);
        self.place_async_agent(&key, id, Some(pane), None, window, cx);
    }

    pub(crate) fn spawn_acp_subagent(
        &mut self,
        parent_id: &AgentId,
        protocol_session_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(parent) = self.acp_views.get(parent_id).cloned() else {
            return;
        };
        let (project_id, profile, project_path) = {
            let parent = parent.read(cx);
            let Some(profile) = parent.profile else {
                return;
            };
            (
                parent.project_id.clone(),
                profile,
                parent.project_path.clone(),
            )
        };
        if let Some(existing_id) = self
            .acp_records
            .values()
            .find(|record| {
                record.metadata.parent_ui_id.as_ref() == Some(parent_id)
                    && record.metadata.protocol_session_id.as_deref()
                        == Some(protocol_session_id.as_str())
            })
            .map(|record| record.metadata.ui_id.clone())
        {
            if self.acp_views.contains_key(&existing_id) {
                self.open_agent(&existing_id, window, cx);
            } else {
                self.unarchive_acp_session(&existing_id, window, cx);
            }
            return;
        }
        let id = muxlane_core::model::new_id("acp");
        let view = cx.new(|cx| {
            AcpView::new(
                AcpViewInit {
                    ui_id: id.clone(),
                    project_id: project_id.clone(),
                    project_path,
                    profile: Some(profile),
                    protocol_session_id: Some(protocol_session_id),
                    parent_ui_id: Some(parent_id.clone()),
                    title: i18n::text(self.language, "acp.subagent").into(),
                    draft: String::new(),
                    snapshot: muxlane_acp::ThreadSnapshot::default(),
                    queued_prompts: Vec::new(),
                    queue_paused: true,
                    theme_mode: self.theme_mode,
                    language: self.language,
                },
                window,
                cx,
            )
        });
        self.register_acp_view(id.clone(), view, window, cx);
        let key = ProjectKey::new(self.local_machine_id(), project_id);
        let pane = self
            .pane_tree
            .pane_for_agent(parent_id)
            .unwrap_or_else(|| self.active_pane.clone());
        self.jump_to_project_if_needed(&key, cx);
        self.place_async_agent(&key, id, Some(pane), None, window, cx);
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
        term
    }

    pub(crate) fn create_remote_term(
        agent: AgentId,
        terminal: (VTerm, tokio::sync::mpsc::UnboundedReceiver<String>),
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
                                    vterm3.feed(&bytes);
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
                .values()
                .any(|term| term.focus_handle(cx).is_focused(window))
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
            || self.acp_delete_confirm.is_some()
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
        self.activate_tab(pane, agent, cx);
        if self.is_acp_session(agent) {
            self.ensure_acp_started(agent, cx);
            self.focus_agent(agent, window, cx);
        } else if self.ensure_agent_terminal(agent, cx) {
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
        let focus = self
            .acp_views
            .get(agent)
            .map(|view| view.focus_handle(cx))
            .or_else(|| self.terms.get(agent).map(|term| term.focus_handle(cx)));
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
        // 清理当前 agent 的 Toast 与标记通知已读
        self.notifications
            .update(cx, |center, cx| center.mark_agent_read(agent, cx));
        if let Some(a) = self.last_snapshot.agent_mut(agent) {
            a.seen = true;
            if a.status.is_finished() {
                a.status = muxlane_core::model::AgentStatus::Idle;
            }
        } else {
            // 远端没有 mark_seen RPC，先同步本地镜像，避免点击后仍持续闪烁。
            for snapshot in self.remote_snaps.values_mut() {
                if let Some(a) = snapshot.agent_mut(agent) {
                    a.seen = true;
                    if a.status.is_finished() {
                        a.status = muxlane_core::model::AgentStatus::Idle;
                    }
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
        if self.is_acp_session(agent) {
            self.session_menu = None;
            self.acp_delete_confirm = Some(agent.clone());
            cx.notify();
        } else {
            self.delete_session(agent, remote, window, cx);
        }
    }

    pub(crate) fn confirm_acp_session_delete(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(agent) = self.acp_delete_confirm.take() else {
            return;
        };
        self.delete_session(&agent, false, window, cx);
    }

    pub(crate) fn delete_session(
        &mut self,
        agent: &AgentId,
        remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_acp_session(agent) {
            self.finish_delete_session(agent, window, cx);
            return;
        }
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
            cx.spawn_in(window, async move |this, cx| {
                let agent_for_delete = agent.clone();
                let result = cx
                    .background_spawn(async move { remote.delete_agent(&agent_for_delete).await })
                    .await;
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

    fn finish_delete_session(
        &mut self,
        agent: &AgentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let removed = std::collections::HashSet::from([agent.clone()]);
        self.workspace.remove_agents(&removed);
        if let Some(pane) = self.pane_tree.pane_for_agent(agent) {
            self.pane_tree.close_tab(&pane, agent);
        }
        self.terms.remove(agent);
        let removed_acp = self.acp_metadata.contains_key(agent)
            || self.acp_records.contains_key(agent)
            || self.acp_views.contains_key(agent);
        if let Some(view) = self.acp_views.remove(agent) {
            view.update(cx, |view, _cx| view.delete_agent_session());
        }
        let delete_revision = self
            .acp_records
            .get(agent)
            .map(|record| record.write_revision)
            .unwrap_or(0);
        self.acp_metadata.remove(agent);
        self.acp_records.remove(agent);
        self.acp_dirty.remove(agent);
        if removed_acp {
            if let Ok(mut deleted) = self.acp_deleted.lock() {
                deleted.insert(agent.clone());
            }
            self.persistence.delete_acp(agent.clone(), delete_revision);
        }
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

    pub(crate) fn remove_acp_project_sessions(&mut self, project_id: &str, cx: &mut Context<Self>) {
        let removed: std::collections::HashSet<_> = self
            .acp_metadata
            .iter()
            .filter(|(_, thread)| thread.project_id == project_id)
            .map(|(id, _)| id.clone())
            .collect();
        if removed.is_empty() {
            return;
        }
        for id in &removed {
            if let Some(view) = self.acp_views.remove(id) {
                view.update(cx, |view, _cx| view.delete_agent_session());
            }
            self.acp_metadata.remove(id);
            let delete_revision = self
                .acp_records
                .get(id)
                .map(|record| record.write_revision)
                .unwrap_or(0);
            self.acp_records.remove(id);
            self.acp_dirty.remove(id);
            if let Ok(mut deleted) = self.acp_deleted.lock() {
                deleted.insert(id.clone());
            }
            self.persistence.delete_acp(id.clone(), delete_revision);
        }
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
            .chain(self.acp_views.keys().cloned())
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
        self.persist();
        cx.notify();
    }

    pub(crate) fn cleanup_removed_agents(&mut self, removed: &[AgentId], cx: &mut Context<Self>) {
        let removed: std::collections::HashSet<_> = removed.iter().cloned().collect();
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
            .chain(self.acp_views.keys().cloned())
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
        let params = muxlane_core::protocol::AgentSpawnParams {
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
        };
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
                    let agent_id = agent.id.clone();
                    let term = Self::create_local_term(
                        agent_id.clone(),
                        session,
                        &this.font_family,
                        Theme::for_mode(this.theme_mode),
                        this.osc52_clipboard_enabled,
                        cx,
                    );
                    this.collapsed_projects.remove(&collapse_key);
                    this.terms.insert(agent_id.clone(), term);
                    this.palette_open = false;
                    this.new_session_target = None;
                    this.jump_to_project_if_needed(&target_key, cx);
                    this.place_async_agent(&target_key, agent_id, Some(pane), None, window, cx);
                    this.select_project_workspace(target_key.clone(), window, cx);
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
}
