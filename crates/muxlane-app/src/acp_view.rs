mod composer;
mod panels;
mod state;
mod timeline;

use self::state::{CaptureContext, CaptureToken, DispatchPhase, PromptQueue, SelectorTarget};
use crate::acp_checkpoint::{capture_checkpoint, restore_checkpoint, ProjectCheckpoint};
use crate::acp_composer::{
    active_token, attachment_from_path, context_items, discover_skills, filter_context,
    load_attachment, load_project_diagnostics, load_skill, merged_completions, Attachment,
    AttachmentKind, CompletionItem, CompletionKind, ContextItem, ContextKind, Skill,
    MAX_CONTEXT_BYTES,
};
use crate::acp_elicitation::{
    fields_from_schema, valid_elicitation_url, values_map, ElicitationFieldKind,
};
use crate::i18n;
use crate::icons::{panel_icon, MAXIMIZE_ICON, PLUS_ICON, SEND_ICON};
use crate::prompt_editor::{PromptEditor, PromptEditorEvent};
use crate::selector_menu::{SelectorMenu, SelectorMenuItem};
use crate::theme::{Theme, ThemeMode};
use crate::ui_scale::px as ui_px;
use crate::widgets::semantic_button;
use gpui::{
    anchored, deferred, div, prelude::*, rgba, Anchor, App, Context, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement, ParentElement, Render, ScrollHandle, Subscription,
    Window,
};
use muxlane_acp::{
    AcpHandle, Capabilities, ConfigKind, ConfigValue, ConnectionPhase, ElicitationMode,
    ElicitationRequest, ElicitationResponse, Event, PermissionKind, PermissionOption, Profile,
    PromptSubmission, Status, ThreadDelta, ThreadReducer, TurnState,
};
use muxlane_core::model::{AgentId, AgentStatus};
use std::rc::Rc;

#[cfg(test)]
pub(crate) use self::timeline::reject_diff_change;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Entry {
    Status(String),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingPermission {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) options: Vec<PermissionOption>,
}

fn queue_permission(permissions: &mut Vec<PendingPermission>, permission: PendingPermission) {
    permissions.push(permission);
}

fn remove_permission(permissions: &mut Vec<PendingPermission>, id: &str) {
    permissions.retain(|permission| permission.id != id);
}

pub(crate) fn agent_status(status: Status) -> AgentStatus {
    match status {
        Status::Connecting => AgentStatus::Working,
        Status::Generating => AgentStatus::Working,
        Status::Permission => AgentStatus::Blocked,
        Status::Failed | Status::Disconnected => AgentStatus::Failed,
        Status::Idle => AgentStatus::Idle,
    }
}

struct ThreadCheckpoint {
    project: ProjectCheckpoint,
    snapshot: muxlane_acp::ThreadSnapshot,
}

fn placeholder_text(language: i18n::Language, profile: Option<&Profile>) -> String {
    i18n::text(language, "acp.placeholder").replace(
        "{profile}",
        profile.map(|profile| profile.label()).unwrap_or("Agent"),
    )
}

pub(crate) struct AcpViewInit {
    pub(crate) ui_id: AgentId,
    pub(crate) project_id: String,
    pub(crate) project_path: std::path::PathBuf,
    pub(crate) profile: Option<Profile>,
    pub(crate) protocol_session_id: Option<String>,
    pub(crate) parent_ui_id: Option<AgentId>,
    pub(crate) title: String,
    pub(crate) draft: String,
    pub(crate) snapshot: muxlane_acp::ThreadSnapshot,
    pub(crate) queued_prompts: Vec<PromptSubmission>,
    pub(crate) queue_paused: bool,
    pub(crate) theme_mode: crate::theme::ThemeMode,
    pub(crate) language: i18n::Language,
}

pub(crate) enum AcpViewEvent {
    ThreadChanged(Box<muxlane_store::PersistedAcpThreadData>),
    RefreshContexts,
    RestartRequested,
    OpenSubagent(String),
    ImportSession { session_id: String, title: String },
    ToggleMaximize,
}

impl EventEmitter<AcpViewEvent> for AcpView {}

pub(crate) struct AcpView {
    pub(crate) ui_id: AgentId,
    pub(crate) project_id: String,
    pub(crate) project_path: std::path::PathBuf,
    pub(crate) profile: Option<Profile>,
    pub(crate) protocol_session_id: Option<String>,
    pub(crate) parent_ui_id: Option<AgentId>,
    pub(crate) title: String,
    pub(crate) draft: Entity<PromptEditor>,
    pub(crate) draft_text: String,
    pub(crate) skills: Vec<Skill>,
    pub(crate) context_items: Vec<ContextItem>,
    pub(crate) selected_contexts: Vec<ContextItem>,
    selected_attachments: Vec<Attachment>,
    pub(crate) completions: Vec<CompletionItem>,
    pub(crate) completion_index: usize,
    open_selector: Option<SelectorTarget>,
    selector_menu: Option<Entity<SelectorMenu>>,
    _selector_menu_subscription: Option<Subscription>,
    prompt_queue: PromptQueue,
    checkpoint: Option<ThreadCheckpoint>,
    checkpoint_undo: Option<ThreadCheckpoint>,
    checkpoint_restore_confirm: bool,
    checkpoint_busy: bool,
    pub(crate) handle: Option<AcpHandle>,
    pub(crate) generation: u64,
    session_epoch: u64,
    pub(crate) start_allowed: bool,
    pub(crate) status: Status,
    connection_phase: ConnectionPhase,
    turn_state: TurnState,
    pub(crate) capabilities: Option<Capabilities>,
    available_sessions: Vec<muxlane_acp::SessionSummary>,
    pub(crate) pending_auth_method: Option<String>,
    pub(crate) thread: ThreadReducer,
    pub(crate) entries: Vec<Entry>,
    pub(crate) timeline_items: Vec<Entity<timeline::TimelineItemView>>,
    generating_indicator: Entity<timeline::GeneratingIndicator>,
    pub(crate) pending_permissions: Vec<PendingPermission>,
    pub(crate) pending_elicitations: Vec<ElicitationRequest>,
    elicitation_forms: std::collections::HashMap<String, panels::ElicitationFormState>,
    theme_mode: crate::theme::ThemeMode,
    language: i18n::Language,
    scroll: ScrollHandle,
    follow_tail: bool,
    unseen_entries: usize,
    _draft_subscription: Subscription,
    _draft_event_subscription: Subscription,
    thread_changed_pending: bool,
}

impl AcpView {
    pub(crate) fn new(init: AcpViewInit, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let AcpViewInit {
            ui_id,
            project_id,
            project_path,
            profile,
            protocol_session_id,
            parent_ui_id,
            title,
            draft,
            snapshot,
            queued_prompts,
            queue_paused,
            theme_mode,
            language,
        } = init;
        let context_root = project_path.clone();
        cx.spawn(async move |this, cx| {
            let (contexts, discovery) = cx
                .background_spawn(async move {
                    (context_items(&context_root), discover_skills(&context_root))
                })
                .await;
            this.update(cx, |view, cx| {
                view.skills = discovery.skills;
                match contexts {
                    Ok(contexts) => view.set_thread_contexts(contexts, cx),
                    Err(error) => {
                        view.push_entry(Entry::Error(error));
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
        let placeholder = placeholder_text(language, profile.as_ref());
        let draft_field = cx.new(|cx| {
            let mut field = PromptEditor::new(placeholder.clone(), window, cx);
            field.set_theme_mode(theme_mode, cx);
            field.set_chrome_visible(false, cx);
            field.set_text(draft.clone(), cx);
            field
        });
        let draft_subscription = cx.observe(&draft_field, |this, field, cx| {
            this.draft_text = field.read(cx).text();
            this.refresh_completions();
            this.schedule_thread_changed(cx);
            cx.notify();
        });
        let draft_event_subscription = cx.subscribe(
            &draft_field,
            |this, _field, event: &PromptEditorEvent, cx| match event {
                PromptEditorEvent::Submit => this.submit_or_confirm(cx),
                PromptEditorEvent::Edited => this.refresh_completions(),
                PromptEditorEvent::CompletionPrevious => {
                    this.move_completion(-1);
                    cx.notify();
                }
                PromptEditorEvent::CompletionNext => {
                    this.move_completion(1);
                    cx.notify();
                }
                PromptEditorEvent::CompletionDismiss => {
                    this.clear_completions();
                    cx.notify();
                }
            },
        );
        let parent = cx.entity().downgrade();
        let generating_indicator =
            cx.new(|_| timeline::GeneratingIndicator::new(theme_mode, language));
        let thread = ThreadReducer::from_snapshot(snapshot);
        let mut view = Self {
            ui_id,
            project_id,
            project_path: project_path.clone(),
            profile,
            protocol_session_id,
            parent_ui_id,
            title,
            draft: draft_field,
            draft_text: draft,
            skills: Vec::new(),
            context_items: Vec::new(),
            selected_contexts: Vec::new(),
            selected_attachments: Vec::new(),
            completions: Vec::new(),
            completion_index: 0,
            open_selector: None,
            selector_menu: None,
            _selector_menu_subscription: None,
            prompt_queue: PromptQueue::new(queued_prompts, queue_paused),
            checkpoint: None,
            checkpoint_undo: None,
            checkpoint_restore_confirm: false,
            checkpoint_busy: false,
            handle: None,
            generation: 0,
            session_epoch: 0,
            start_allowed: true,
            status: Status::Disconnected,
            connection_phase: ConnectionPhase::Disconnected,
            turn_state: TurnState::Idle,
            capabilities: None,
            available_sessions: Vec::new(),
            pending_auth_method: None,
            thread,
            entries: Vec::new(),
            timeline_items: Vec::new(),
            generating_indicator,
            pending_permissions: Vec::new(),
            pending_elicitations: Vec::new(),
            elicitation_forms: std::collections::HashMap::new(),
            theme_mode,
            language,
            scroll: ScrollHandle::new(),
            follow_tail: true,
            unseen_entries: 0,
            _draft_subscription: draft_subscription,
            _draft_event_subscription: draft_event_subscription,
            thread_changed_pending: false,
        };
        view.rebuild_timeline(parent, cx);
        view
    }

    fn rebuild_timeline(&mut self, parent: gpui::WeakEntity<Self>, cx: &mut Context<Self>) {
        let old_views: std::collections::HashMap<_, _> = self
            .timeline_items
            .iter()
            .map(|view| {
                let view = view.read(cx);
                (
                    view.item_key(),
                    (view.item().clone(), view.transient_state()),
                )
            })
            .collect();
        self.timeline_items = self
            .thread
            .snapshot()
            .items
            .iter()
            .map(|item| {
                let project_path = self.project_path.clone();
                let theme_mode = self.theme_mode;
                let language = self.language;
                let view = cx.new(|cx| {
                    timeline::TimelineItemView::new(
                        item,
                        parent.clone(),
                        project_path,
                        theme_mode,
                        language,
                        cx,
                    )
                });
                if let Some((old_item, state)) =
                    old_views.get(&timeline::TimelineItemView::key(item))
                {
                    if let Some(state) = timeline::preserve_transient_state(old_item, item, state) {
                        view.update(cx, |view, _cx| view.restore_transient_state(state));
                    }
                }
                view
            })
            .collect();
    }

    fn reconcile_timeline(
        &mut self,
        change: muxlane_acp::ThreadChange,
        cx: &mut Context<Self>,
    ) -> bool {
        let muxlane_acp::ThreadChange::Item { index, appended } = change else {
            return false;
        };
        let Some(item) = self.thread.snapshot().items.get(index).cloned() else {
            self.rebuild_timeline(cx.entity().downgrade(), cx);
            return true;
        };
        let key = timeline::TimelineItemView::key(&item);
        if appended {
            if index != self.timeline_items.len() {
                self.rebuild_timeline(cx.entity().downgrade(), cx);
                return true;
            }
            let project_path = self.project_path.clone();
            let theme_mode = self.theme_mode;
            let language = self.language;
            let parent = cx.entity().downgrade();
            self.timeline_items.push(cx.new(|cx| {
                timeline::TimelineItemView::new(
                    &item,
                    parent,
                    project_path,
                    theme_mode,
                    language,
                    cx,
                )
            }));
            return true;
        }
        let Some(existing) = self.timeline_items.get(index) else {
            self.rebuild_timeline(cx.entity().downgrade(), cx);
            return true;
        };
        if existing.read(cx).item_key() != key {
            self.rebuild_timeline(cx.entity().downgrade(), cx);
            return true;
        }
        existing.update(cx, |view, cx| view.set_item(item, cx));
        false
    }

    pub(crate) fn set_thread_contexts(
        &mut self,
        contexts: Vec<ContextItem>,
        cx: &mut Context<Self>,
    ) {
        let mut next_contexts: Vec<_> = self
            .context_items
            .iter()
            .filter(|context| !matches!(context.kind, ContextKind::Thread | ContextKind::Terminal))
            .cloned()
            .collect();
        next_contexts.extend(contexts);
        next_contexts.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        if next_contexts == self.context_items {
            return;
        }
        self.context_items = next_contexts;
        self.refresh_completions();
        cx.notify();
    }

    pub(crate) fn set_theme_mode(
        &mut self,
        theme_mode: crate::theme::ThemeMode,
        cx: &mut Context<Self>,
    ) {
        if self.theme_mode == theme_mode {
            return;
        }
        self.theme_mode = theme_mode;
        self.draft
            .update(cx, |field, cx| field.set_theme_mode(theme_mode, cx));
        if let Some(menu) = &self.selector_menu {
            menu.update(cx, |menu, cx| {
                menu.set_theme(Theme::for_mode(theme_mode), cx)
            });
        }
        for form in self.elicitation_forms.values() {
            for editor in form.editors.values() {
                editor.update(cx, |editor, cx| editor.set_theme_mode(theme_mode, cx));
            }
        }
        for item in &self.timeline_items {
            item.update(cx, |item, cx| item.set_theme_mode(theme_mode, cx));
        }
        self.generating_indicator
            .update(cx, |indicator, cx| indicator.set_theme_mode(theme_mode, cx));
        cx.notify();
    }

    pub(crate) fn set_language(&mut self, language: i18n::Language, cx: &mut Context<Self>) {
        self.language = language;
        let placeholder = placeholder_text(language, self.profile.as_ref());
        self.draft
            .update(cx, |field, cx| field.set_placeholder(placeholder, cx));
        for item in &self.timeline_items {
            item.update(cx, |item, cx| item.set_language(language, cx));
        }
        self.generating_indicator
            .update(cx, |indicator, cx| indicator.set_language(language, cx));
        cx.notify();
    }

    pub(crate) fn start(&mut self, active: muxlane_acp::ActiveSession, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.session_epoch = self.session_epoch.wrapping_add(1);
        self.prompt_queue.suspend_uncertain_submission();
        self.checkpoint_restore_confirm = false;
        let generation = self.generation;
        let session_epoch = self.session_epoch;
        let mut events = active.events;
        self.handle = Some(active.handle);
        self.start_allowed = false;
        self.connection_phase = ConnectionPhase::Connecting;
        self.turn_state = TurnState::Idle;
        self.refresh_status();
        self.pending_permissions.clear();
        self.pending_elicitations.clear();
        let entity = cx.entity().downgrade();
        cx.spawn(async move |_this, cx| {
            while let Some(event) = events.recv().await {
                if entity
                    .update(cx, |view, cx| {
                        if view.generation == generation && view.session_epoch == session_epoch {
                            view.apply(event, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn apply(&mut self, event: Event, cx: &mut Context<Self>) {
        let changes_thread = matches!(&event, Event::Delta(_));
        let mut notify = true;
        let mut timeline_changed = false;
        let mut adds_timeline_entry = false;
        let _change = match event {
            Event::Ready {
                protocol_session_id,
                restored,
                capabilities,
                modes,
                config_options,
            } => {
                self.protocol_session_id = Some(protocol_session_id);
                self.capabilities = Some(capabilities);
                if let Some(modes) = modes {
                    self.apply_delta(ThreadDelta::Modes(modes), cx);
                }
                if !config_options.is_empty() {
                    self.apply_delta(ThreadDelta::ConfigOptions(config_options), cx);
                }
                if restored {
                    self.push_entry(Entry::Status(
                        i18n::text(self.language, "acp.restored").into(),
                    ));
                }
                self.flush_thread_changed(cx);
                None
            }
            Event::Capabilities(capabilities) => {
                self.capabilities = Some(capabilities);
                None
            }
            Event::Authenticated => {
                self.pending_auth_method = None;
                None
            }
            Event::LoggedOut => {
                self.request_restart(true, cx);
                None
            }
            Event::SessionList(sessions) => {
                self.available_sessions = sessions;
                if self.open_selector == Some(SelectorTarget::Sessions) {
                    self.clear_selector_menu();
                }
                None
            }
            Event::Connection(phase) => {
                self.connection_phase = phase;
                if phase == ConnectionPhase::Disconnected {
                    self.handle = None;
                    self.prompt_queue.suspend_uncertain_submission();
                    self.pending_permissions.clear();
                    self.pending_elicitations.clear();
                    self.elicitation_forms.clear();
                }
                self.refresh_status();
                None
            }
            Event::Turn(turn_state) => {
                self.turn_state = turn_state;
                self.refresh_status();
                if turn_state == TurnState::Idle {
                    self.dispatch_next_queued(cx);
                    self.flush_thread_changed(cx);
                    cx.emit(AcpViewEvent::RefreshContexts);
                }
                None
            }
            Event::PromptAccepted { id } => {
                if self.prompt_queue.accept(&id) {
                    self.flush_thread_changed(cx);
                }
                None
            }
            Event::PromptRejected { id, error } => {
                if matches!(
                    &self.prompt_queue.phase,
                    DispatchPhase::AwaitingAcceptance(pending_id) if pending_id == &id
                ) {
                    self.prompt_queue.clear_pending();
                    self.prompt_queue.paused = true;
                    self.push_entry(Entry::Error(error.message));
                    self.flush_thread_changed(cx);
                }
                None
            }
            Event::Delta(delta) => {
                let (change, structure_changed) = self.apply_delta(delta, cx);
                timeline_changed = matches!(
                    change,
                    muxlane_acp::ThreadChange::Item { .. } | muxlane_acp::ThreadChange::Plan
                );
                adds_timeline_entry = matches!(
                    change,
                    muxlane_acp::ThreadChange::Item { appended: true, .. }
                        | muxlane_acp::ThreadChange::Plan
                );
                notify = structure_changed
                    || !matches!(
                        change,
                        muxlane_acp::ThreadChange::Item {
                            appended: false,
                            ..
                        }
                    );
                Some(change)
            }
            Event::Permission { id, title, options } => {
                queue_permission(
                    &mut self.pending_permissions,
                    PendingPermission { id, title, options },
                );
                self.refresh_status();
                None
            }
            Event::Elicitation(request) => {
                self.add_elicitation(request);
                self.refresh_status();
                None
            }
            Event::TerminalOutput(snapshot) => {
                let terminal_id = snapshot.id.clone();
                for item in &self.timeline_items {
                    if item.read(cx).contains_terminal(&terminal_id) {
                        let snapshot = snapshot.clone();
                        item.update(cx, |item, cx| item.set_terminal_snapshot(snapshot, cx));
                    }
                }
                notify = false;
                None
            }
            Event::Error(error) => {
                self.push_entry(Entry::Error(error.message));
                None
            }
        };
        if changes_thread {
            self.schedule_thread_changed(cx);
        }
        if timeline_changed {
            if self.follow_tail {
                self.scroll.scroll_to_bottom();
            } else if adds_timeline_entry {
                self.unseen_entries = self.unseen_entries.saturating_add(1);
                notify = true;
            }
        }
        if notify {
            cx.notify();
        }
    }

    fn refresh_status(&mut self) {
        self.status =
            if !self.pending_permissions.is_empty() || !self.pending_elicitations.is_empty() {
                Status::Permission
            } else {
                match self.connection_phase {
                    ConnectionPhase::Connecting => Status::Connecting,
                    ConnectionPhase::Connected => match self.turn_state {
                        TurnState::Idle => Status::Idle,
                        TurnState::Generating => Status::Generating,
                    },
                    ConnectionPhase::Failed => Status::Failed,
                    ConnectionPhase::Disconnected => Status::Disconnected,
                }
            };
    }

    fn apply_delta(
        &mut self,
        delta: ThreadDelta,
        cx: &mut Context<Self>,
    ) -> (muxlane_acp::ThreadChange, bool) {
        let terminal_ids = match &delta {
            ThreadDelta::ToolUpsert {
                content: Some(content),
                ..
            } => content
                .iter()
                .filter_map(|content| match content {
                    muxlane_acp::ToolContent::Terminal { id } => Some(id.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let session_title = match &delta {
            ThreadDelta::SessionInfo(value) => value
                .get("title")
                .and_then(serde_json::Value::as_str)
                .filter(|title| !title.trim().is_empty())
                .map(str::to_string),
            _ => None,
        };
        let change = self.thread.apply(delta);
        let structure_changed = self.reconcile_timeline(change, cx);
        if let Some(title) = session_title {
            self.title = title;
            self.schedule_thread_changed(cx);
        }
        if matches!(change, muxlane_acp::ThreadChange::AvailableCommands) {
            self.refresh_completions();
        }
        if let Some(handle) = &self.handle {
            for id in terminal_ids {
                if let Err(error) = handle.poll_terminal(id) {
                    self.push_entry(Entry::Error(error.to_string()));
                    break;
                }
            }
        }
        (change, structure_changed)
    }

    fn push_entry(&mut self, entry: Entry) {
        self.entries.push(entry);
    }

    pub(crate) fn send_prompt(&mut self, cx: &mut Context<Self>) {
        if self.parent_ui_id.is_some() || self.checkpoint_restore_confirm {
            return;
        }
        let text = self.draft.read(cx).text().trim().to_string();
        if text.is_empty() {
            return;
        }
        if self.handle.is_none() {
            self.push_entry(Entry::Error(
                i18n::text(self.language, "acp.disconnected").into(),
            ));
            cx.notify();
            return;
        }
        let Some(payload) = self.build_prompt_payload(text.clone(), cx) else {
            return;
        };
        self.prompt_queue.enqueue(PromptSubmission::new(payload));
        self.draft_text.clear();
        self.draft.update(cx, |field, cx| field.reset(cx));
        self.selected_contexts.clear();
        self.selected_attachments.clear();
        self.follow_tail = true;
        self.unseen_entries = 0;
        self.scroll.scroll_to_bottom();
        self.dispatch_next_queued(cx);
        cx.emit(AcpViewEvent::ThreadChanged(Box::new(self.thread_data())));
        cx.notify();
    }

    fn dispatch_next_queued(&mut self, cx: &mut Context<Self>) {
        if self.prompt_queue.paused
            || !matches!(self.prompt_queue.phase, DispatchPhase::Idle)
            || self.turn_state != TurnState::Idle
            || self.connection_phase != ConnectionPhase::Connected
            || self.checkpoint_busy
            || self.checkpoint_restore_confirm
        {
            return;
        }
        let Some(submission) = self.prompt_queue.front().cloned() else {
            return;
        };
        let token = CaptureToken {
            session_epoch: self.session_epoch,
            prompt_id: submission.id.clone(),
        };
        self.prompt_queue.begin_capture(token.clone());
        let root = self.project_path.clone();
        let snapshot = self.thread.snapshot().clone();
        cx.spawn(async move |this, cx| {
            let checkpoint = cx
                .background_spawn(async move { capture_checkpoint(&root) })
                .await;
            this.update(cx, |this, cx| {
                let valid = token.is_valid(CaptureContext {
                    session_epoch: this.session_epoch,
                    front_id: this.prompt_queue.front().map(|item| &item.id),
                    paused: this.prompt_queue.paused,
                    connected: this.connection_phase == ConnectionPhase::Connected,
                    turn_state: this.turn_state,
                    phase: &this.prompt_queue.phase,
                    checkpoint_busy: this.checkpoint_busy,
                    restore_confirmed: this.checkpoint_restore_confirm,
                });
                if !valid {
                    this.prompt_queue.cancel_capture(&token);
                    return;
                }
                match checkpoint {
                    Ok(Some(project)) => {
                        this.checkpoint = Some(ThreadCheckpoint { project, snapshot });
                    }
                    Ok(None) => this.checkpoint = None,
                    Err(error) => {
                        this.checkpoint = None;
                        this.push_entry(Entry::Error(error));
                    }
                }
                this.send_queued_submission(submission, token, cx);
            })
            .ok();
        })
        .detach();
        cx.emit(AcpViewEvent::ThreadChanged(Box::new(self.thread_data())));
        cx.notify();
    }

    fn send_queued_submission(
        &mut self,
        submission: PromptSubmission,
        token: CaptureToken,
        cx: &mut Context<Self>,
    ) {
        let valid = token.is_valid(CaptureContext {
            session_epoch: self.session_epoch,
            front_id: self.prompt_queue.front().map(|item| &item.id),
            paused: self.prompt_queue.paused,
            connected: self.connection_phase == ConnectionPhase::Connected,
            turn_state: self.turn_state,
            phase: &self.prompt_queue.phase,
            checkpoint_busy: self.checkpoint_busy,
            restore_confirmed: self.checkpoint_restore_confirm,
        });
        if !valid {
            self.prompt_queue.cancel_capture(&token);
            return;
        }
        let Some(handle) = &self.handle else {
            self.prompt_queue.clear_pending();
            return;
        };
        match handle.submit_submission(submission.clone()) {
            Ok(id) => self.prompt_queue.await_acceptance(id),
            Err(error) => {
                self.prompt_queue.clear_pending();
                self.prompt_queue.paused = true;
                self.push_entry(Entry::Error(error.to_string()));
            }
        }
        cx.emit(AcpViewEvent::ThreadChanged(Box::new(self.thread_data())));
        cx.notify();
    }

    fn can_restore_checkpoint(&self) -> bool {
        self.checkpoint.is_some()
            && !self.checkpoint_restore_confirm
            && self.turn_state == TurnState::Idle
            && matches!(self.prompt_queue.phase, DispatchPhase::Idle)
            && !self.checkpoint_busy
            && self.connection_phase == ConnectionPhase::Connected
    }

    fn request_checkpoint_restore(&mut self, cx: &mut Context<Self>) {
        if self.can_restore_checkpoint() {
            self.prompt_queue.invalidate();
            self.checkpoint_restore_confirm = true;
            cx.notify();
        }
    }

    fn restore_last_checkpoint(&mut self, cx: &mut Context<Self>) {
        if self.turn_state != TurnState::Idle
            || !matches!(self.prompt_queue.phase, DispatchPhase::Idle)
            || self.checkpoint_busy
            || self.connection_phase != ConnectionPhase::Connected
        {
            return;
        }
        let Some(checkpoint) = self.checkpoint.take() else {
            return;
        };
        self.checkpoint_restore_confirm = false;
        self.checkpoint_busy = true;
        self.prompt_queue.paused = true;
        self.prompt_queue.invalidate();
        let project = checkpoint.project.clone();
        let undo_snapshot = self.thread.snapshot().clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { restore_checkpoint(&project) })
                .await;
            this.update(cx, |this, cx| {
                this.checkpoint_busy = false;
                match result {
                    Ok(undo_project) => {
                        this.checkpoint_undo = undo_project.map(|project| ThreadCheckpoint {
                            project,
                            snapshot: undo_snapshot,
                        });
                        this.thread = ThreadReducer::from_snapshot(checkpoint.snapshot);
                        this.rebuild_timeline(cx.entity().downgrade(), cx);
                        this.request_restart(true, cx);
                        this.push_entry(Entry::Status(
                            i18n::text(this.language, "acp.checkpoint_restored").into(),
                        ));
                    }
                    Err(error) => {
                        this.checkpoint = Some(checkpoint);
                        this.push_entry(Entry::Error(error));
                    }
                }
                cx.emit(AcpViewEvent::ThreadChanged(Box::new(this.thread_data())));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn undo_checkpoint_restore(&mut self, cx: &mut Context<Self>) {
        if self.turn_state != TurnState::Idle
            || !matches!(self.prompt_queue.phase, DispatchPhase::Idle)
            || self.checkpoint_busy
            || self.connection_phase != ConnectionPhase::Connected
        {
            return;
        }
        let Some(checkpoint) = self.checkpoint_undo.take() else {
            return;
        };
        self.checkpoint_busy = true;
        let project = checkpoint.project.clone();
        let redo_snapshot = self.thread.snapshot().clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { restore_checkpoint(&project) })
                .await;
            this.update(cx, |this, cx| {
                this.checkpoint_busy = false;
                match result {
                    Ok(redo_project) => {
                        this.checkpoint = redo_project.map(|project| ThreadCheckpoint {
                            project,
                            snapshot: redo_snapshot,
                        });
                        this.thread = ThreadReducer::from_snapshot(checkpoint.snapshot);
                        this.rebuild_timeline(cx.entity().downgrade(), cx);
                        this.request_restart(true, cx);
                        this.push_entry(Entry::Status(
                            i18n::text(this.language, "acp.checkpoint_undo_restored").into(),
                        ));
                    }
                    Err(error) => {
                        this.checkpoint_undo = Some(checkpoint);
                        this.push_entry(Entry::Error(error));
                    }
                }
                cx.emit(AcpViewEvent::ThreadChanged(Box::new(this.thread_data())));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn remove_first_queued(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.prompt_queue.phase,
            DispatchPhase::AwaitingAcceptance(_)
        ) {
            if let Some(handle) = &self.handle {
                if let Err(error) = handle.cancel() {
                    self.push_entry(Entry::Error(error.to_string()));
                }
            }
        }
        self.prompt_queue.remove_front();
        cx.emit(AcpViewEvent::ThreadChanged(Box::new(self.thread_data())));
        cx.notify();
    }

    fn clear_queue(&mut self, cx: &mut Context<Self>) {
        self.prompt_queue.clear();
        cx.emit(AcpViewEvent::ThreadChanged(Box::new(self.thread_data())));
        cx.notify();
    }

    fn send_next_now(&mut self, cx: &mut Context<Self>) {
        self.prompt_queue.paused = false;
        if self.turn_state == TurnState::Generating {
            if let Some(handle) = &self.handle {
                if let Err(error) = handle.cancel() {
                    self.push_entry(Entry::Error(error.to_string()));
                }
            }
        } else {
            self.dispatch_next_queued(cx);
        }
        cx.emit(AcpViewEvent::ThreadChanged(Box::new(self.thread_data())));
        cx.notify();
    }

    fn begin_authentication(&mut self, method_id: String, cx: &mut Context<Self>) {
        self.pending_auth_method = Some(method_id);
        self.request_restart(false, cx);
    }

    pub(crate) fn logout(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = &self.handle {
            if let Err(error) = handle.logout() {
                self.push_entry(Entry::Error(error.to_string()));
            }
        }
        cx.notify();
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        self.prompt_queue.paused = true;
        cx.emit(AcpViewEvent::ThreadChanged(Box::new(self.thread_data())));
        if let Some(handle) = &self.handle {
            if let Err(error) = handle.cancel() {
                self.push_entry(Entry::Error(error.to_string()));
            }
        }
        cx.notify();
    }

    pub(crate) fn choose_permission(
        &mut self,
        id: String,
        option: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(handle) = &self.handle else {
            self.push_entry(Entry::Error("ACP session is not connected".into()));
            cx.notify();
            return;
        };
        if let Err(error) = handle.choose_permission(id.clone(), option) {
            self.push_entry(Entry::Error(error.to_string()));
            cx.notify();
            return;
        }
        remove_permission(&mut self.pending_permissions, &id);
        self.refresh_status();
        cx.notify();
    }

    pub(crate) fn delete_agent_session(&mut self) {
        let can_delete = self
            .capabilities
            .as_ref()
            .is_some_and(|capabilities| capabilities.sessions.delete);
        if can_delete {
            self.session_epoch = self.session_epoch.wrapping_add(1);
            self.prompt_queue.invalidate();
            if let (Some(handle), Some(session_id)) =
                (self.handle.take(), self.protocol_session_id.as_ref())
            {
                if let Err(error) = handle.delete_session(session_id.clone()) {
                    tracing::warn!(%error, %session_id, "delete ACP session failed");
                }
                handle.shutdown();
            }
            self.pending_permissions.clear();
            self.pending_elicitations.clear();
            self.start_allowed = false;
            self.connection_phase = ConnectionPhase::Disconnected;
            self.refresh_status();
        } else {
            self.shutdown();
        }
    }

    pub(crate) fn shutdown(&mut self) {
        self.session_epoch = self.session_epoch.wrapping_add(1);
        self.prompt_queue.suspend_uncertain_submission();
        if let Some(handle) = self.handle.take() {
            if let Some(session_id) = self.protocol_session_id.as_ref().filter(|_| {
                self.capabilities
                    .as_ref()
                    .is_some_and(|capabilities| capabilities.sessions.close)
            }) {
                if let Err(error) = handle.close_session(session_id.clone()) {
                    tracing::warn!(%error, %session_id, "close ACP session failed");
                }
            }
            handle.shutdown();
        }
        self.pending_permissions.clear();
        self.pending_elicitations.clear();
        self.start_allowed = false;
        self.connection_phase = ConnectionPhase::Disconnected;
        self.refresh_status();
    }

    pub(crate) fn request_restart(&mut self, fresh: bool, cx: &mut Context<Self>) {
        self.shutdown();
        self.generation = self.generation.wrapping_add(1);
        if fresh {
            self.protocol_session_id = None;
            self.entries.clear();
            cx.emit(AcpViewEvent::ThreadChanged(Box::new(self.thread_data())));
        }
        self.start_allowed = true;
        self.connection_phase = ConnectionPhase::Connecting;
        self.refresh_status();
        cx.emit(AcpViewEvent::RestartRequested);
        cx.notify();
    }

    fn schedule_thread_changed(&mut self, cx: &mut Context<Self>) {
        if self.thread_changed_pending {
            return;
        }
        self.thread_changed_pending = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(200))
                .await;
            this.update(cx, |view, cx| {
                if view.thread_changed_pending {
                    view.thread_changed_pending = false;
                    cx.emit(AcpViewEvent::ThreadChanged(Box::new(view.thread_data())));
                }
            })
            .ok();
        })
        .detach();
    }

    fn flush_thread_changed(&mut self, cx: &mut Context<Self>) {
        self.thread_changed_pending = false;
        cx.emit(AcpViewEvent::ThreadChanged(Box::new(self.thread_data())));
    }

    pub(crate) fn thread_data(&self) -> muxlane_store::PersistedAcpThreadData {
        let mut data = muxlane_store::PersistedAcpThreadData::new(self.metadata());
        data.snapshot = self.thread.snapshot().clone();
        data.queued_prompts = self.prompt_queue.submissions.iter().cloned().collect();
        data.queue_paused = self.prompt_queue.paused;
        data
    }

    pub(crate) fn metadata(&self) -> muxlane_store::PersistedAcpThread {
        muxlane_store::PersistedAcpThread {
            ui_id: self.ui_id.clone(),
            project_id: self.project_id.clone(),
            profile_id: self
                .profile
                .map(|profile| profile.id())
                .unwrap_or_default()
                .into(),
            protocol_session_id: self.protocol_session_id.clone(),
            title: self.title.clone(),
            parent_ui_id: self.parent_ui_id.clone(),
            draft: self.draft_text.clone(),
        }
    }

    fn resume_follow_tail(&mut self, cx: &mut Context<Self>) {
        self.follow_tail = true;
        self.unseen_entries = 0;
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    fn pause_follow_tail(&mut self, cx: &mut Context<Self>) {
        if self.follow_tail {
            self.follow_tail = false;
            cx.notify();
        }
    }

    fn render_notice(entry: &Entry, theme: Theme, language: i18n::Language) -> gpui::Div {
        let (label, color, body) = match entry {
            Entry::Status(text) => (i18n::text(language, "acp.status"), theme.fg2, text.clone()),
            Entry::Error(text) => (i18n::text(language, "acp.error"), theme.red, text.clone()),
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(rgba(theme.line))
            .child(
                div()
                    .text_size(ui_px(10.))
                    .text_color(rgba(color))
                    .child(label),
            )
            .child(
                div()
                    .text_size(ui_px(12.))
                    .text_color(rgba(theme.fg0))
                    .child(body),
            )
    }
}

impl Focusable for AcpView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.draft.focus_handle(cx)
    }
}
impl Render for AcpView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let completion_active = !self.checkpoint_restore_confirm && !self.completions.is_empty();
        let submit_enabled = !self.checkpoint_restore_confirm;
        self.draft.update(cx, |editor, _cx| {
            editor.set_completion_active(completion_active);
            editor.set_submit_enabled(submit_enabled);
        });
        let theme = Theme::for_mode(self.theme_mode);
        let page_bg = if self.theme_mode == ThemeMode::Light {
            0xecececff
        } else {
            theme.bg1
        };
        let snapshot = self.thread.snapshot();
        let plan = snapshot.plan.as_ref();
        let mut timeline = div().flex().flex_col();
        if self.timeline_items.is_empty() && self.entries.is_empty() && plan.is_none() {
            timeline = timeline.child(
                div()
                    .px_3()
                    .py_4()
                    .text_color(rgba(theme.fg2))
                    .child(i18n::text(self.language, "acp.empty")),
            );
        } else {
            for item in &self.timeline_items {
                timeline = timeline.child(item.clone());
            }
            if let Some(plan) = plan {
                let mut plan_view = div()
                    .mx_3()
                    .my_2()
                    .border_1()
                    .border_color(rgba(theme.line))
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .bg(rgba(theme.bg2))
                            .text_color(rgba(theme.green))
                            .child(i18n::text(self.language, "acp.plan")),
                    );
                for entry in &plan.entries {
                    plan_view = plan_view.child(
                        div()
                            .flex()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .child(entry.content.clone())
                            .child(
                                div()
                                    .ml_auto()
                                    .text_size(ui_px(9.))
                                    .text_color(rgba(theme.fg2))
                                    .child(format!("{} · {}", entry.priority, entry.status)),
                            ),
                    );
                }
                timeline = timeline.child(plan_view);
            }
            for entry in &self.entries {
                timeline = timeline.child(Self::render_notice(entry, theme, self.language));
            }
        }
        if self.turn_state == TurnState::Generating {
            timeline = timeline.child(self.generating_indicator.clone());
        }
        timeline = timeline.child(self.render_panels(window, cx));
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgba(page_bg))
            .child(
                div()
                    .id("acp-timeline-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .on_scroll_wheel(cx.listener(
                        |this, _event: &gpui::ScrollWheelEvent, _window, cx| {
                            this.pause_follow_tail(cx)
                        },
                    ))
                    .child(timeline),
            )
            .when(self.unseen_entries > 0, |thread| {
                let label = i18n::text(self.language, "acp.new_content")
                    .replace("{count}", &self.unseen_entries.to_string());
                thread.child(
                    semantic_button("acp-new-content", label.clone(), theme)
                        .mx_3()
                        .mb_1()
                        .px_3()
                        .py_1()
                        .border_1()
                        .border_color(rgba(theme.accent))
                        .text_color(rgba(theme.accent))
                        .on_click(
                            cx.listener(|this, _event, _window, cx| this.resume_follow_tail(cx)),
                        )
                        .child(label),
                )
            })
            .child(self.render_queue_panels(cx))
            .child(self.render_composer(window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::composer::footer_control_groups;
    use super::*;

    #[test]
    fn rejecting_diff_restores_only_an_unchanged_agent_result() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("file.txt");
        std::fs::write(&file, "new").unwrap();
        reject_diff_change(
            directory.path(),
            &file.display().to_string(),
            Some("old"),
            "new",
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "old");

        std::fs::write(&file, "user edit").unwrap();
        assert!(reject_diff_change(
            directory.path(),
            &file.display().to_string(),
            Some("old"),
            "new",
        )
        .is_err());
        assert_eq!(std::fs::read_to_string(file).unwrap(), "user edit");
    }

    #[test]
    fn permission_queue_keeps_parallel_requests_visible() {
        let option = PermissionOption {
            id: "allow".into(),
            label: "Allow".into(),
            kind: PermissionKind::AllowOnce,
        };
        let mut permissions = Vec::new();
        queue_permission(
            &mut permissions,
            PendingPermission {
                id: "first".into(),
                title: "First".into(),
                options: vec![option.clone()],
            },
        );
        queue_permission(
            &mut permissions,
            PendingPermission {
                id: "second".into(),
                title: "Second".into(),
                options: vec![option],
            },
        );

        remove_permission(&mut permissions, "first");

        assert_eq!(permissions.len(), 1);
        assert_eq!(permissions[0].id, "second");
    }

    #[test]
    fn footer_controls_group_secondary_selects_before_booleans() {
        fn select(id: &str, name: &str) -> muxlane_acp::ConfigOption {
            muxlane_acp::ConfigOption {
                id: id.into(),
                name: name.into(),
                description: None,
                category: None,
                kind: ConfigKind::Select {
                    current: "a".into(),
                    choices: vec![muxlane_acp::ConfigChoice {
                        id: "a".into(),
                        name: "A".into(),
                    }],
                },
            }
        }
        fn boolean(id: &str, name: &str) -> muxlane_acp::ConfigOption {
            muxlane_acp::ConfigOption {
                id: id.into(),
                name: name.into(),
                description: None,
                category: None,
                kind: ConfigKind::Boolean { current: true },
            }
        }

        let options = vec![
            boolean("fast", "Fast mode"),
            select("approval", "Approval"),
            select("model", "Model"),
            boolean("verbose", "Verbose"),
            select("effort", "Thinking effort"),
            select("sandbox", "Sandbox"),
        ];

        let groups = footer_control_groups(&options);

        assert_eq!(groups.secondary_selects, vec![1, 5]);
        assert_eq!(groups.model, Some(2));
        assert_eq!(groups.thinking, Some(4));
        assert_eq!(groups.secondary_booleans, vec![0, 3]);
    }
}
