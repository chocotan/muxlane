mod composer;
mod code_highlight;
mod panels;
mod state;
mod style;
mod timeline;
mod title;
mod tool_display;

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
use crate::icons::{panel_icon, CLOSE_ICON, MAXIMIZE_ICON, PLUS_ICON, SEND_ICON};
use crate::prompt_editor::{PromptEditor, PromptEditorEvent};
use crate::selector_menu::{SelectorMenu, SelectorMenuItem};
use crate::theme::Theme;
use crate::ui_scale::px as ui_px;
use crate::widgets::semantic_button;
use self::style::*;
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
    pub(crate) tool_id: String,
    pub(crate) context: String,
    pub(crate) options: Vec<PermissionOption>,
}

fn queue_permission(permissions: &mut Vec<PendingPermission>, permission: PendingPermission) {
    if !permissions.iter().any(|pending| pending.id == permission.id) {
        permissions.push(permission);
    }
}

fn remove_permission(permissions: &mut Vec<PendingPermission>, id: &str) {
    permissions.retain(|permission| permission.id != id);
}

fn permission_context(tool: Option<&muxlane_acp::Tool>, terminals: &std::collections::HashMap<String, muxlane_acp::TerminalOutputState>) -> String {
    use muxlane_acp::{ToolContent, TerminalOutputState};
    let mut context = String::new();
    if let Some(tool) = tool {
        for content in tool.content.iter().take(16) {
            let text = match content {
                ToolContent::Diff { path, old_text, new_text } => format!("{path}\n{}", tool_display::diff_preview(old_text.as_deref().unwrap_or_default(), new_text).text),
                ToolContent::Terminal { id } => match terminals.get(id) {
                    Some(TerminalOutputState::Ready(snapshot)) => snapshot.command.as_ref().map(tool_display::command_text).unwrap_or_else(|| format!("Terminal {id}")),
                    _ => format!("Terminal {id}"),
                },
                ToolContent::Text(text) => tool_display::bounded_text(text, 8 * 1024),
                ToolContent::Resource { uri, .. } | ToolContent::ResourceLink { uri, .. } => uri.clone(),
                _ => continue,
            };
            context.push_str(&text);
            context.push('\n');
            if context.len() > 16 * 1024 { break; }
        }
    }
    tool_display::bounded_text(&context, 16 * 1024)
}

fn permission_resolution(snapshot: &muxlane_acp::ThreadSnapshot, id: &str, state: muxlane_acp::ToolState) -> Option<ThreadDelta> {
    use muxlane_acp::{ThreadItem, ToolState};
    snapshot.items.iter().find_map(|item| match item {
        ThreadItem::Tool(tool) if tool.id == id && matches!(tool.state, ToolState::Pending | ToolState::Running) => Some(ThreadDelta::ToolUpsert {
            id: id.into(), title: None, kind: None, state: Some(state.clone()),
            content: None, locations: None, raw_input: None, raw_output: None, subagent_session_id: None,
        }),
        _ => None,
    })
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
    pub(crate) profile_id: String,
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
    PromptCompleted,
    RestartRequested,
    OpenSubagent(String),
    ToggleMaximize,
}

impl EventEmitter<AcpViewEvent> for AcpView {}

pub(crate) struct AcpView {
    // Keep unresolved ids intact when metadata is saved again.
    pub(crate) profile_id: String,
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
    checkpoint_restore_confirm: bool,
    checkpoint_busy: bool,
    pub(crate) handle: Option<AcpHandle>,
    pub(crate) generation: u64,
    session_epoch: u64,
    pub(crate) start_allowed: bool,
    pub(crate) status: Status,
    connection_phase: ConnectionPhase,
    turn_state: TurnState,
    completion_prompt: Option<muxlane_acp::PromptId>,
    pub(crate) capabilities: Option<Capabilities>,
    pub(crate) pending_auth_method: Option<String>,
    pub(crate) thread: ThreadReducer,
    pub(crate) entries: Vec<Entry>,
    pub(crate) timeline_items: Vec<Entity<timeline::TimelineItemView>>,
    terminal_results: std::collections::HashMap<String, muxlane_acp::TerminalOutputState>,
    terminal_result_order: std::collections::VecDeque<String>,
    terminal_queries: std::collections::HashMap<String, u64>,
    terminal_query_sequence: u64,
    generating_indicator: Entity<timeline::GeneratingIndicator>,
    pub(crate) pending_permissions: Vec<PendingPermission>,
    pub(crate) pending_elicitations: Vec<ElicitationRequest>,
    elicitation_forms: std::collections::HashMap<String, panels::ElicitationFormState>,
    theme_mode: crate::theme::ThemeMode,
    language: i18n::Language,
    scroll: ScrollHandle,
    scrollbar: crate::pixel_scrollbar::PixelScrollbar,
    follow_tail: bool,
    unseen_entries: usize,
    _draft_subscription: Subscription,
    _draft_event_subscription: Subscription,
    thread_changed_pending: bool,
    prompt_title_pending: bool,
}

impl AcpView {
    pub(crate) fn new(init: AcpViewInit, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let AcpViewInit {
            profile_id,
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
            field.configure_composer(600., cx);
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
                PromptEditorEvent::Edited => {}
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
        let prompt_title_pending = queued_prompts.is_empty() && !title::has_user_prompt(&snapshot);
        let thread = ThreadReducer::from_snapshot(snapshot);
        let mut view = Self {
            profile_id,
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
            checkpoint_restore_confirm: false,
            checkpoint_busy: false,
            handle: None,
            generation: 0,
            session_epoch: 0,
            start_allowed: true,
            status: Status::Disconnected,
            connection_phase: ConnectionPhase::Disconnected,
            turn_state: TurnState::Idle,
            completion_prompt: None,
            capabilities: None,
            pending_auth_method: None,
            thread,
            entries: Vec::new(),
            timeline_items: Vec::new(),
            terminal_results: Default::default(),
            terminal_result_order: Default::default(),
            terminal_queries: Default::default(),
            terminal_query_sequence: 0,
            generating_indicator,
            pending_permissions: Vec::new(),
            pending_elicitations: Vec::new(),
            elicitation_forms: std::collections::HashMap::new(),
            theme_mode,
            language,
            scroll: ScrollHandle::new(),
            scrollbar: Default::default(),
            follow_tail: true,
            unseen_entries: 0,
            _draft_subscription: draft_subscription,
            _draft_event_subscription: draft_event_subscription,
            thread_changed_pending: false,
            prompt_title_pending,
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
                self.hydrate_terminal_results(&view, cx);
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
            let view = cx.new(|cx| {
                timeline::TimelineItemView::new(
                    &item,
                    parent,
                    project_path,
                    theme_mode,
                    language,
                    cx,
                )
            });
            self.hydrate_terminal_results(&view, cx);
            self.timeline_items.push(view);
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
        self.hydrate_terminal_results(existing, cx);
        false
    }

    fn merge_sorted_contexts(left: Vec<ContextItem>, right: Vec<ContextItem>) -> Vec<ContextItem> {
        let capacity = left.len() + right.len();
        let mut left = left.into_iter().peekable();
        let mut right = right.into_iter().peekable();
        let mut merged = Vec::with_capacity(capacity);
        while left.peek().is_some() || right.peek().is_some() {
            let take_left = match (left.peek(), right.peek()) {
                (Some(left), Some(right)) => left.relative_path <= right.relative_path,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => unreachable!(),
            };
            if take_left {
                merged.push(left.next().expect("peeked left context"));
            } else {
                merged.push(right.next().expect("peeked right context"));
            }
        }
        merged
    }

    pub(crate) fn set_thread_contexts(
        &mut self,
        contexts: Vec<ContextItem>,
        cx: &mut Context<Self>,
    ) {
        let mut existing_project = Vec::new();
        let mut existing_thread = Vec::new();
        for context in std::mem::take(&mut self.context_items) {
            if matches!(context.kind, ContextKind::Thread | ContextKind::Terminal) {
                existing_thread.push(context);
            } else {
                existing_project.push(context);
            }
        }

        let mut incoming_project = Vec::new();
        let mut incoming_thread = Vec::new();
        for context in contexts {
            if matches!(context.kind, ContextKind::Thread | ContextKind::Terminal) {
                incoming_thread.push(context);
            } else {
                incoming_project.push(context);
            }
        }
        incoming_thread.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        if incoming_project.is_empty() && existing_thread == incoming_thread {
            self.context_items = if existing_project.is_empty() {
                existing_thread
            } else if existing_thread.is_empty() {
                existing_project
            } else {
                Self::merge_sorted_contexts(existing_project, existing_thread)
            };
            return;
        }

        let project_contexts = if existing_project.is_empty() {
            incoming_project
        } else if incoming_project.is_empty() {
            existing_project
        } else {
            Self::merge_sorted_contexts(existing_project, incoming_project)
        };
        let thread_contexts = Self::merge_sorted_contexts(existing_thread, incoming_thread);
        let next_contexts = if project_contexts.is_empty() {
            thread_contexts
        } else if thread_contexts.is_empty() {
            project_contexts
        } else {
            Self::merge_sorted_contexts(project_contexts, thread_contexts)
        };
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
            Event::SessionList(_) => None,
            Event::Connection(phase) => {
                self.connection_phase = phase;
                if matches!(phase, ConnectionPhase::Disconnected | ConnectionPhase::Failed) {
                    self.completion_prompt = None;
                    self.handle = None;
                    self.finish_terminal_queries(cx);
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
                    self.completion_prompt = None;
                    self.dispatch_next_queued(cx);
                    self.flush_thread_changed(cx);
                    cx.emit(AcpViewEvent::RefreshContexts);
                }
                None
            }
            Event::PromptCompleted { id } => {
                if self.completion_prompt.as_ref() == Some(&id) {
                    self.completion_prompt = None;
                    self.flush_thread_changed(cx);
                    cx.emit(AcpViewEvent::PromptCompleted);
                }
                None
            }
            Event::PromptAccepted { id } => {
                self.completion_prompt = Some(id.clone());
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
            Event::Permission { id, title, tool_id, options } => {
                let tool = self.thread.snapshot().items.iter().find_map(|item| match item {
                    muxlane_acp::ThreadItem::Tool(tool) if tool.id == tool_id => Some(tool),
                    _ => None,
                });
                let title = tool.filter(|tool| !tool.title.is_empty()).map(|tool| tool.title.clone()).unwrap_or(title);
                let context = permission_context(tool, &self.terminal_results);
                queue_permission(
                    &mut self.pending_permissions,
                    PendingPermission { id, title, tool_id, context, options },
                );
                self.refresh_status();
                None
            }
            Event::Elicitation(request) => {
                self.add_elicitation(request);
                self.refresh_status();
                None
            }
            Event::TerminalOutput(result) => {
                self.terminal_queries.remove(&result.id);
                self.set_terminal_result(result, cx);
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
        let session_title = match &delta {
            ThreadDelta::SessionInfo(value) => value
                .get("title")
                .and_then(serde_json::Value::as_str)
                .filter(|title| !title.trim().is_empty())
                .map(str::to_string),
            _ => None,
        };
        let change = self.thread.apply(delta);
        let terminal_ids = match change {
            muxlane_acp::ThreadChange::Item { index, .. } => match self.thread.snapshot().items.get(index) {
                Some(muxlane_acp::ThreadItem::Tool(tool)) => tool.content.iter().filter_map(|content| match content {
                    muxlane_acp::ToolContent::Terminal { id } => Some(id.clone()),
                    _ => None,
                }).collect(),
                Some(muxlane_acp::ThreadItem::Content(content)) => match &content.content {
                    muxlane_acp::ToolContent::Terminal { id } => vec![id.clone()],
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        let structure_changed = self.reconcile_timeline(change, cx);
        if let Some(title) = title::updated_title(&self.title, "", None, session_title.as_deref()) {
            self.title = title;
            self.prompt_title_pending = false;
            self.schedule_thread_changed(cx);
        }
        if matches!(change, muxlane_acp::ThreadChange::AvailableCommands) {
            self.refresh_completions();
        }
        for id in terminal_ids {
            self.poll_terminal(id, cx);
        }
        (change, structure_changed)
    }

    fn hydrate_terminal_results(&self, item: &Entity<timeline::TimelineItemView>, cx: &mut Context<Self>) {
        for (id, state) in &self.terminal_results {
            if item.read(cx).contains_terminal(id) {
                let result = muxlane_acp::TerminalQueryResult { id: id.clone(), state: state.clone() };
                item.update(cx, |item, cx| item.set_terminal_result(result, cx));
            }
        }
    }

    fn set_terminal_result(&mut self, result: muxlane_acp::TerminalQueryResult, cx: &mut Context<Self>) {
        use muxlane_acp::TerminalOutputState;
        let id = result.id.clone();
        if !matches!(self.terminal_results.get(&id), Some(TerminalOutputState::Ready(_)))
            || matches!(&result.state, TerminalOutputState::Ready(_)) {
            self.terminal_results.insert(id.clone(), result.state.clone());
            self.terminal_result_order.retain(|key| key != &id);
            self.terminal_result_order.push_back(id.clone());
        }
        while self.terminal_results.len() > 32 || self.terminal_results.values().map(|state| match state {
            TerminalOutputState::Ready(snapshot) => snapshot.output.len(),
            _ => 0,
        }).sum::<usize>() > 16 * 1024 * 1024 {
            if let Some(key) = self.terminal_result_order.pop_front() { self.terminal_results.remove(&key); }
            else { break; }
        }
        for item in &self.timeline_items {
            if item.read(cx).contains_terminal(&id) {
                item.update(cx, |item, cx| item.set_terminal_result(result.clone(), cx));
            }
        }
    }

    fn finish_terminal_queries(&mut self, cx: &mut Context<Self>) {
        for (id, _) in std::mem::take(&mut self.terminal_queries) {
            self.set_terminal_result(muxlane_acp::TerminalQueryResult {
                id, state: muxlane_acp::TerminalOutputState::Unavailable,
            }, cx);
        }
    }

    fn poll_terminal(&mut self, id: String, cx: &mut Context<Self>) {
        use muxlane_acp::{TerminalOutputState, TerminalQueryResult};
        if self.terminal_queries.contains_key(&id) { return; }
        let state = match &self.handle {
            None => TerminalOutputState::Unavailable,
            Some(handle) => match handle.poll_terminal(id.clone()) {
                Ok(()) => TerminalOutputState::Pending,
                Err(error) => TerminalOutputState::Failed(error.to_string()),
            },
        };
        if state == TerminalOutputState::Pending {
            self.terminal_query_sequence = self.terminal_query_sequence.wrapping_add(1);
            let token = self.terminal_query_sequence;
            self.terminal_queries.insert(id.clone(), token);
            let id = id.clone();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(std::time::Duration::from_secs(3)).await;
                this.update(cx, |view, cx| {
                    if view.terminal_queries.get(&id) == Some(&token) {
                        view.terminal_queries.remove(&id);
                        view.set_terminal_result(TerminalQueryResult {
                            id, state: TerminalOutputState::Failed("Terminal output query timed out".into()),
                        }, cx);
                    }
                }).ok();
            }).detach();
        }
        self.set_terminal_result(TerminalQueryResult { id, state }, cx);
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
        if self.prompt_title_pending && !title::has_user_prompt(self.thread.snapshot()) {
            let default_title = format!(
                "{} UI",
                self.profile
                    .as_ref()
                    .map(|profile| profile.label())
                    .unwrap_or("Agent")
            );
            if let Some(title) = title::updated_title(&self.title, &default_title, Some(&text), None) {
                self.title = title;
            }
        }
        self.prompt_title_pending = false;
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
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { restore_checkpoint(&project) })
                .await;
            this.update(cx, |this, cx| {
                this.checkpoint_busy = false;
                match result {
                    Ok(_) => {
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
        self.completion_prompt = None;
        self.prompt_queue.paused = true;
        cx.emit(AcpViewEvent::ThreadChanged(Box::new(self.thread_data())));
        if let Some(handle) = self.handle.clone() {
            let pending_ids: Vec<_> = self.pending_permissions.iter().map(|permission| permission.id.clone()).collect();
            for id in pending_ids {
                self.choose_permission(id, None, cx);
            }
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
        let Some(permission) = self.pending_permissions.iter().find(|pending| pending.id == id).cloned() else {
            return;
        };
        let state = match option.as_ref() {
            None => Some(muxlane_acp::ToolState::Canceled),
            Some(id) => match permission.options.iter().find(|choice| &choice.id == id) {
                Some(choice) if matches!(choice.kind, PermissionKind::RejectOnce | PermissionKind::RejectAlways) => Some(muxlane_acp::ToolState::Rejected),
                Some(_) => None,
                None => return,
            },
        };
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
        if let Some(state) = state {
            if let Some(delta) = permission_resolution(self.thread.snapshot(), &permission.tool_id, state) {
                self.apply(Event::Delta(delta), cx);
            }
        }
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
        self.finish_terminal_queries(cx);
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
            profile_id: self.profile_id.clone(),
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
        let (label, bar, tint, body) = match entry {
            Entry::Status(text) => (
                i18n::text(language, "acp.status"),
                theme.line,
                None,
                text.clone(),
            ),
            Entry::Error(text) => (
                i18n::text(language, "acp.error"),
                theme.red,
                Some(Theme::with_alpha(theme.red, TINT_ALPHA)),
                text.clone(),
            ),
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .mx(ui_px(CARD_MARGIN))
            .my_2()
            .pl(ui_px(BAR_PAD))
            .pr(ui_px(CARD_PAD))
            .py_2()
            .border_l_2()
            .border_color(rgba(bar))
            .when_some(tint, |notice, tint| notice.bg(rgba(tint)))
            .child(meta(theme, label).font_weight(gpui::FontWeight::SEMIBOLD))
            .child(
                div()
                    .text_size(ui_px(BODY_SIZE))
                    .line_height(ui_px(BODY_LINE))
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
        let page_bg = theme.bg1;
        let snapshot = self.thread.snapshot();
        let plan = snapshot.plan.as_ref();
        let mut timeline = div().flex().flex_col().min_w_0().w_full();
        if self.timeline_items.is_empty() && self.entries.is_empty() && plan.is_none() {
            timeline = timeline.child(
                div()
                    .flex()
                    .justify_center()
                    .px(ui_px(CONTENT_INSET))
                    .py_8()
                    .text_size(ui_px(BODY_SIZE))
                    .line_height(ui_px(BODY_LINE))
                    .text_color(rgba(theme.fg2))
                    .child(i18n::text(self.language, "acp.empty")),
            );
        } else {
            for item in &self.timeline_items {
                timeline = timeline.child(item.clone());
            }
            if let Some(plan) = plan {
                let mut plan_view = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .mx(ui_px(CARD_MARGIN))
                    .my_2()
                    .py_2()
                    .border_1()
                    .border_color(rgba(theme.line))
                    .text_size(ui_px(BODY_SIZE))
                    .line_height(ui_px(BODY_LINE))
                    .child(
                        card_title(theme, theme.accent, i18n::text(self.language, "acp.plan"))
                            .px(ui_px(CARD_PAD)),
                    );
                for entry in &plan.entries {
                    let status = entry.status.as_str();
                    let status_color = match status {
                        "completed" | "done" => theme.green,
                        "in_progress" => theme.accent,
                        _ => theme.fg2,
                    };
                    plan_view = plan_view.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px(ui_px(CARD_PAD))
                            .child(indicator(status_color))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_color(rgba(theme.fg0))
                                    .child(entry.content.clone()),
                            )
                            .child(
                                meta(theme, format!("{} · {}", entry.priority, entry.status))
                                    .ml_auto()
                                    .flex_none(),
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
        let draft_for_bounds = self.draft.clone();
        let view_for_scroll = cx.entity().downgrade();
        div()
            .relative()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgba(page_bg))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .w_full()
                    .min_h_0()
                    .child(
                        div()
                            .id("acp-timeline-scroll")
                            .min_w_0()
                            .pr(ui_px(10.))
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .on_scroll_wheel(cx.listener(
                                |this, _event: &gpui::ScrollWheelEvent, _window, cx| {
                                    this.pause_follow_tail(cx)
                                },
                            ))
                            .child(timeline),
                    )
                    .child(self.scrollbar.render(&self.scroll, theme, move |_, cx| {
                        view_for_scroll.update(cx, |this, cx| this.pause_follow_tail(cx)).ok();
                    }))
                    .when(self.unseen_entries > 0, |scroll_area| {
                        let label = i18n::text(self.language, "acp.new_content")
                            .replace("{count}", &self.unseen_entries.to_string());
                        scroll_area.child(
                            button("acp-new-content", label.clone(), theme)
                                .absolute()
                                .bottom_2()
                                .right_3()
                                .px_3()
                                .py_1()
                                .border_color(rgba(theme.accent))
                                .bg(rgba(theme.bg0))
                                .text_color(rgba(theme.fg0))
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.resume_follow_tail(cx)
                                }))
                                .child(label),
                        )
                    }),
            )
            .child(self.render_queue_panels(cx))
            .child(self.render_composer(window, cx))
            .child(gpui::canvas(move |bounds, _, cx| {
                draft_for_bounds.update(cx, |editor, cx| {
                    editor.configure_composer(f32::from(bounds.size.height) / f32::from(ui_px(1.)), cx);
                });
            }, |_, _, _, _| {}).absolute().size_full())
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

    pub(super) fn terminal_test_view(window: &mut Window, cx: &mut Context<AcpView>) -> AcpView {
        let mut view = AcpView::new(AcpViewInit {
            profile_id: "terminal-test".into(), ui_id: "terminal-test".into(),
            project_id: "terminal-test".into(), project_path: "/unused".into(),
            profile: None, protocol_session_id: None, parent_ui_id: None,
            title: "Terminal".into(), draft: String::new(), snapshot: Default::default(),
            queued_prompts: vec![], queue_paused: false,
            theme_mode: crate::theme::ThemeMode::Light,
            language: i18n::Language::from_id("en").unwrap(),
        }, window, cx);
        view.start_allowed = false;
        view
    }

    fn terminal_tool(id: &str, state: muxlane_acp::ToolState) -> Event {
        Event::Delta(ThreadDelta::ToolUpsert {
            id: id.into(), title: Some("Terminal".into()), kind: Some(muxlane_acp::ToolKind::Execute),
            state: Some(state), content: Some(vec![muxlane_acp::ToolContent::Terminal { id: id.into() }]),
            locations: None, raw_input: None, raw_output: None, subagent_session_id: None,
        })
    }

    #[test]
    fn terminal_unknown_completed_and_assistant_do_not_leave_loading() {
        use muxlane_acp::{TerminalOutputState, TerminalQueryResult, ToolState};
        let mut cx = gpui::TestAppContext::single();
        let window = cx.add_window(terminal_test_view);
        let entity = window.root(&mut cx).unwrap();
        entity.update(&mut cx, |view, cx| {
            view.apply(terminal_tool("tool_fixture", ToolState::Running), cx);
            assert_eq!(view.timeline_items[0].read(cx).terminal_state("tool_fixture"), TerminalOutputState::Unavailable);
            view.set_terminal_result(TerminalQueryResult { id: "tool_fixture".into(), state: TerminalOutputState::Pending }, cx);
            view.apply(Event::TerminalOutput(TerminalQueryResult { id: "tool_fixture".into(), state: TerminalOutputState::Unavailable }), cx);
            view.apply(terminal_tool("tool_fixture", ToolState::Completed), cx);
            view.apply(Event::Delta(ThreadDelta::MessageChunk { id: Some("answer".into()), role: muxlane_acp::MessageRole::Assistant, text: "Done".into() }), cx);
            assert_eq!(view.timeline_items[0].read(cx).terminal_state("tool_fixture"), TerminalOutputState::Unavailable);
            assert_eq!(view.timeline_items.len(), 2);
            assert!(view.entries.is_empty());
        });
    }

    #[test]
    fn terminal_events_before_item_empty_ready_failure_and_disconnect() {
        use muxlane_acp::{TerminalOutputState, TerminalQueryResult, TerminalSnapshot, ToolState};
        let mut cx = gpui::TestAppContext::single();
        let window = cx.add_window(terminal_test_view);
        let entity = window.root(&mut cx).unwrap();
        entity.update(&mut cx, |view, cx| {
            for (id, output) in [("early", "final output"), ("empty", "")] {
                let state = TerminalOutputState::Ready(TerminalSnapshot { id: id.into(), output: output.into(), truncated: false, exit_code: Some(0), command: None });
                view.apply(Event::TerminalOutput(TerminalQueryResult { id: id.into(), state: state.clone() }), cx);
                view.apply(terminal_tool(id, ToolState::Completed), cx);
                let item = view.timeline_items.last().unwrap().clone();
                assert_eq!(item.read(cx).terminal_state(id), state);
                view.apply(Event::TerminalOutput(TerminalQueryResult { id: id.into(), state: TerminalOutputState::Unavailable }), cx);
                assert_eq!(item.read(cx).terminal_state(id), state, "unavailable must not erase output, including empty Ready");
                view.rebuild_timeline(cx.entity().downgrade(), cx);
                assert_eq!(view.timeline_items.last().unwrap().read(cx).terminal_state(id), state);
            }
            view.apply(terminal_tool("failed", ToolState::Running), cx);
            let failure = TerminalOutputState::Failed("query failed".into());
            view.apply(Event::TerminalOutput(TerminalQueryResult { id: "failed".into(), state: failure.clone() }), cx);
            assert_eq!(view.timeline_items[2].read(cx).terminal_state("failed"), failure);
            view.poll_terminal("failed".into(), cx);
            assert_eq!(view.timeline_items[2].read(cx).terminal_state("failed"), TerminalOutputState::Unavailable);
            view.set_terminal_result(TerminalQueryResult { id: "failed".into(), state: TerminalOutputState::Pending }, cx);
            view.terminal_queries.insert("failed".into(), 1);
            view.apply(Event::Connection(ConnectionPhase::Disconnected), cx);
            assert_eq!(view.timeline_items[2].read(cx).terminal_state("failed"), TerminalOutputState::Unavailable);
            assert!(view.terminal_queries.is_empty());
            assert!(view.entries.is_empty(), "terminal errors stay local");
            view.terminal_results.clear();
            view.timeline_items.clear();
            view.rebuild_timeline(cx.entity().downgrade(), cx);
            assert_eq!(view.timeline_items[0].read(cx).terminal_state("early"), TerminalOutputState::Unavailable, "restored transcript has no live terminal");
        });
    }

    #[test]
    fn streamed_response_keeps_one_entity_and_one_markdown_paragraph() {
        use gpui::{px, size, TestAppContext, VisualTestContext};
        use muxlane_acp::{MessageRole, ThreadItem};
        for first_id in [None, Some("shared")] {
            let mut cx = TestAppContext::single();
            let window = cx.add_window(terminal_test_view);
            let parent = window.root(&mut cx).unwrap();
            parent.update(&mut cx, |view, cx| {
                view.apply(Event::Delta(ThreadDelta::ThoughtChunk { id: Some("shared".into()), text: "Thinking".into() }), cx);
                view.apply(Event::Delta(ThreadDelta::MessageChunk { id: first_id.map(str::to_string), role: MessageRole::Assistant, text: "你".into() }), cx);
                let thought = view.timeline_items[0].entity_id();
                let response = view.timeline_items[1].entity_id();
                let key = view.timeline_items[1].read(cx).item_key();
                for text in ["好", "，", "世", "界", "。"] {
                    view.apply(Event::Delta(ThreadDelta::MessageChunk { id: Some("shared".into()), role: MessageRole::Assistant, text: text.into() }), cx);
                    assert_eq!(view.timeline_items.len(), 2);
                    assert_eq!(view.timeline_items[0].entity_id(), thought);
                    assert_eq!(view.timeline_items[1].entity_id(), response);
                    assert_eq!(view.timeline_items[1].read(cx).item_key(), key);
                }
                assert!(matches!(view.timeline_items[1].read(cx).item(), ThreadItem::Message(message) if message.text == "你好，世界。"));
            });
            let window: gpui::AnyWindowHandle = window.into();
            let mut visual = VisualTestContext::from_window(window, &mut cx);
            for width in [900., 320.] {
                visual.simulate_resize(size(px(width), px(640.)));
                for _ in 0..3 {
                    cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx)).unwrap();
                    cx.update_window(window, |_, window, cx| window.simulate_next_frame(cx)).unwrap();
                }
                let bounds = visual.debug_bounds("acp-assistant-message").unwrap();
                assert!(bounds.size.height <= px(44.), "a sentence must not reserve a toolbar row per token: {bounds:?}");
                assert!(bounds.left() >= px(0.) && bounds.right() <= px(width + 1.));
            }
        }
    }

    #[test]
    fn context_footer_is_present_and_bounded_for_every_profile_and_usage_state() {
        use gpui::{px, size, TestAppContext, VisualTestContext};
        use muxlane_acp::{Cost, Usage};
        let mut cx = TestAppContext::single();
        let window = cx.add_window(terminal_test_view);
        let parent = window.root(&mut cx).unwrap();
        let window: gpui::AnyWindowHandle = window.into();
        let mut visual = VisualTestContext::from_window(window, &mut cx);
        for profile in ["opencode", "pi", "claude", "codex", "custom-agent"] {
            for usage in [None, Some(Usage { used: 42, size: 0, cost: None }),
                Some(Usage { used: 0, size: 100, cost: Some(Cost { amount: 0., currency: "USD".into() }) }),
                Some(Usage { used: 75_786, size: 300_000, cost: Some(Cost { amount: 0.0000001, currency: "USD".into() }) }),
                Some(Usage { used: u64::MAX, size: 100, cost: None })]
            {
                parent.update(&mut cx, |view, cx| {
                    view.profile_id = profile.into();
                    let mut snapshot = view.thread.snapshot().clone();
                    snapshot.usage = usage;
                    snapshot.modes = Some(muxlane_acp::Modes { current: "model".into(), available: vec![muxlane_acp::SessionMode {
                        id: "model".into(), name: "Long model label ".repeat(40), description: None,
                    }] });
                    view.thread = ThreadReducer::from_snapshot(snapshot);
                    cx.notify();
                });
                for width in [900., 320., 240.] {
                    visual.simulate_resize(size(px(width), px(640.)));
                    for _ in 0..3 {
                        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx)).unwrap();
                        cx.update_window(window, |_, window, cx| window.simulate_next_frame(cx)).unwrap();
                    }
                    let usage = visual.debug_bounds("acp-token-usage").expect("all profiles show Context, including unreported usage");
                    let composer = visual.debug_bounds("acp-composer").unwrap();
                    assert!(usage.size.width > px(0.) && usage.size.width <= px(141.), "{profile}: {usage:?}");
                    assert!(usage.left() >= composer.left() && usage.right() <= composer.right());
                    assert!(usage.top() >= composer.top() && usage.bottom() <= composer.bottom());
                    assert!(composer.right() <= px(width + 1.));
                }
            }
        }
    }

    #[test]
    fn composer_and_markdown_stay_inside_resized_panes() {
        use gpui::{px, size, TestAppContext, VisualTestContext};
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|window, cx| {
            let mut view = AcpView::new(AcpViewInit {
                profile_id: "layout-only".into(), ui_id: "layout-only".into(),
                project_id: "layout-only".into(), project_path: "/unused".into(),
                profile: None, protocol_session_id: None, parent_ui_id: None,
                title: "Layout".into(), draft: "中文 😀 draft".into(),
                snapshot: muxlane_acp::ThreadSnapshot {
                    items: vec![muxlane_acp::ThreadItem::Message(muxlane_acp::Message {
                        protocol_id: None,
                        id: "layout".into(), role: muxlane_acp::MessageRole::Assistant,
                        text: format!("{}\n\n```rust\nlet value = \"{}\";\n```\n\n{}", "正文 Unicode 😀 with wrapping words ".repeat(30), "long code ".repeat(100), "Final paragraph ".repeat(40)),
                    })],
                    modes: Some(muxlane_acp::Modes { current: "long".into(), available: vec![muxlane_acp::SessionMode { id: "long".into(), name: "LongModelLabel".repeat(60), description: None }] }),
                    ..Default::default()
                },
                queued_prompts: vec![], queue_paused: false,
                theme_mode: crate::theme::ThemeMode::Light, language: i18n::Language::from_id("en").unwrap(),
            }, window, cx);
            view.start_allowed = false;
            view
        });
        let view = window.root(&mut cx).unwrap();
        let window: gpui::AnyWindowHandle = window.into();
        let mut visual = VisualTestContext::from_window(window, &mut cx);
        let mut previous_geometry: Option<(f32, gpui::Bounds<gpui::Pixels>)> = None;
        for (width, height) in [(900., 640.), (320., 640.), (900., 640.), (320., 260.)] {
            visual.simulate_resize(size(px(width), px(height)));
            for _ in 0..3 {
                cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx)).unwrap();
                cx.update_window(window, |_, window, cx| window.simulate_next_frame(cx)).unwrap();
            }
            for selector in ["acp-composer", "acp-prompt-editor", "acp-markdown", "acp-code-scroll"] {
                let bounds = visual.debug_bounds(selector).expect(selector);
                assert!(bounds.left() >= px(0.) && bounds.right() <= px(width + 1.), "{selector}: {bounds:?} at {width}");
                assert!(bounds.size.width > px(0.));
            }
            let editor = visual.debug_bounds("acp-prompt-editor").unwrap();
            assert!(editor.size.height >= px(99.) && editor.size.height <= px(241.));
            let composer = visual.debug_bounds("acp-composer").unwrap();
            assert!(composer.bottom() <= px(height + 1.));
            if width < 400. { assert!(cx.update(|cx| view.read(cx).scroll.max_offset().y) > px(0.)); }
            let geometry = cx.update(|cx| view.read(cx).timeline_items[0].read(cx).assert_current_text_geometry());
            if let Some((previous_width, previous)) = previous_geometry {
                if width < previous_width {
                    assert!(geometry.size.width < previous.size.width);
                    assert!(geometry.size.height > previous.size.height, "narrow paragraphs must rewrap");
                } else {
                    assert!(geometry.size.width > previous.size.width);
                    assert!(geometry.size.height < previous.size.height, "wide paragraphs must unwrap");
                }
            }
            previous_geometry = Some((width, geometry));
        }
    }

    #[test]
    fn tool_shell_and_diff_fixtures_preserve_raw_collapse_and_horizontal_scroll() {
        use gpui::{px, size, TestAppContext, VisualTestContext};
        use muxlane_acp::{ToolContent, ToolKind, ToolState, TerminalCommand, TerminalSnapshot, TerminalOutputState, TerminalQueryResult};
        let mut cx = TestAppContext::single();
        let window = cx.add_window(terminal_test_view);
        let entity = window.root(&mut cx).unwrap();
        entity.update(&mut cx, |view, cx| {
            let line = format!("  indented {}\n", "long output ".repeat(100));
            view.apply(Event::TerminalOutput(TerminalQueryResult { id: "shell".into(), state: TerminalOutputState::Ready(TerminalSnapshot {
                id: "shell".into(), output: line.clone(), truncated: true, exit_code: Some(3), command: Some(TerminalCommand {
                    command: "/bin/sh".into(), args: vec!["-c".into(), "display only".into()], cwd: "/unused".into(),
                }),
            }) }), cx);
            for (id, kind, content) in [
                ("shell", ToolKind::Execute, ToolContent::Terminal { id: "shell".into() }),
                ("diff", ToolKind::Edit, ToolContent::Diff { path: "/unused/remote.rs".into(), old_text: Some("old\n".into()), new_text: line.clone() }),
            ] {
                view.apply(Event::Delta(ThreadDelta::ToolUpsert { id: id.into(), title: Some(format!("{}\ncomplete title", "Long protocol title ".repeat(20))), kind: Some(kind), state: Some(ToolState::Completed), content: Some(vec![content]), locations: None, raw_input: Some(serde_json::json!("{partial")), raw_output: Some(serde_json::json!({"result":"large"})), subagent_session_id: None }), cx);
                view.timeline_items.last().unwrap().update(cx, |item, cx| item.expand_tool_fixture(cx));
            }
        });
        let window: gpui::AnyWindowHandle = window.into();
        let mut visual = VisualTestContext::from_window(window, &mut cx);
        for width in [900., 320.] {
            visual.simulate_resize(size(px(width), px(1200.)));
            for _ in 0..3 {
                cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx)).unwrap();
                cx.update_window(window, |_, window, cx| window.simulate_next_frame(cx)).unwrap();
            }
            cx.update(|cx| {
                let view = entity.read(cx);
                view.timeline_items[0].read(cx).assert_tool_fixture("tool:shell:content:0", "tool:shell:input");
                view.timeline_items[1].read(cx).assert_tool_fixture("tool:diff:content:0", "tool:diff:input");
            });
            for selector in ["acp-tool-code-scroll", "acp-tool-diff-scroll"] {
                let bounds = visual.debug_bounds(selector).expect(selector);
                assert!(bounds.left() >= px(0.) && bounds.right() <= px(width + 1.), "{selector}: {bounds:?}");
            }
        }
    }

    #[test]
    fn permission_outcomes_only_change_pending_or_running_associated_tools() {
        use muxlane_acp::{ToolState, ThreadItem};
        for state in [ToolState::Pending, ToolState::Running, ToolState::Completed, ToolState::Failed, ToolState::Rejected, ToolState::Canceled] {
            for outcome in [ToolState::Rejected, ToolState::Canceled] {
                let mut reducer = ThreadReducer::new();
                for id in ["t", "unrelated"] {
                    let Event::Delta(delta) = terminal_tool(id, state.clone()) else { unreachable!() };
                    reducer.apply(delta);
                }
                let before = reducer.snapshot().clone();
                let patch = permission_resolution(reducer.snapshot(), "t", outcome.clone());
                assert_eq!(patch.is_some(), matches!(state, ToolState::Pending | ToolState::Running));
                let expected = if let Some(patch) = patch {
                    reducer.apply(patch);
                    outcome
                } else {
                    state.clone()
                };
                let ThreadItem::Tool(tool) = &reducer.snapshot().items[0] else { unreachable!() };
                assert_eq!(tool.state, expected);
                let mut unchanged_fields = tool.clone();
                unchanged_fields.state = state.clone();
                assert_eq!(ThreadItem::Tool(unchanged_fields), before.items[0]);
                assert_eq!(reducer.snapshot().items[1], before.items[1]);
                assert!(permission_resolution(reducer.snapshot(), "missing", ToolState::Canceled).is_none());
            }
        }
    }

    #[test]
    fn permission_context_uses_typed_data() {
        use muxlane_acp::{ToolContent, ToolState};
        let mut cx = gpui::TestAppContext::single();
        let window = cx.add_window(terminal_test_view);
        let entity = window.root(&mut cx).unwrap();
        entity.update(&mut cx, |view, cx| {
            view.apply(terminal_tool("t", ToolState::Pending), cx);
            let Event::Delta(mut patch) = terminal_tool("t", ToolState::Pending) else { unreachable!() };
            if let ThreadDelta::ToolUpsert { content, raw_input, .. } = &mut patch {
                *content = Some(vec![ToolContent::Diff { path: "/remote/file".into(), old_text: Some("a\n".into()), new_text: "b\n".into() }]);
                *raw_input = Some(serde_json::json!({"command":"not a real command"}));
            }
            view.apply(Event::Delta(patch), cx);
            view.apply(Event::Permission { id: "request".into(), title: "Fallback".into(), tool_id: "t".into(), options: vec![] }, cx);
            let permission = &view.pending_permissions[0];
            assert_eq!(permission.title, "Terminal");
            assert!(permission.context.contains("/remote/file\n@@"));
            assert!(!permission.context.contains("not a real command"));
        });
    }

    fn permission_fixture(view: &mut AcpView, cx: &mut Context<AcpView>) {
        for id in ["t", "other"] {
            view.apply(Event::Delta(ThreadDelta::ToolUpsert {
                id: id.into(), title: Some("Read file".into()), kind: Some(muxlane_acp::ToolKind::Read),
                state: Some(muxlane_acp::ToolState::Pending), content: Some(vec![]),
                locations: None, raw_input: None, raw_output: None, subagent_session_id: None,
            }), cx);
            view.apply(Event::Permission {
                id: format!("request-{id}"), title: "Permission".into(), tool_id: id.into(),
                options: [
                    ("allow-once", PermissionKind::AllowOnce),
                    ("allow-always", PermissionKind::AllowAlways),
                    ("reject-once", PermissionKind::RejectOnce),
                    ("reject-always", PermissionKind::RejectAlways),
                ].into_iter().map(|(id, kind)| PermissionOption { id: id.into(), label: id.into(), kind }).collect(),
            }, cx);
        }
    }

    #[test]
    fn permission_sends_exact_choice_updates_target_and_prevents_duplicate_sends() {
        use muxlane_acp::{ToolState, ThreadItem, test_support::command_channel};
        for (option, expected) in [
            (Some("allow-once"), ToolState::Pending),
            (Some("allow-always"), ToolState::Pending),
            (Some("reject-once"), ToolState::Rejected),
            (Some("reject-always"), ToolState::Rejected),
            (None, ToolState::Canceled),
        ] {
            let mut cx = gpui::TestAppContext::single();
            let window = cx.add_window(terminal_test_view);
            let entity = window.root(&mut cx).unwrap();
            let (handle, mut receiver) = command_channel();
            entity.update(&mut cx, |view, cx| {
                permission_fixture(view, cx);
                view.handle = Some(handle);
                let other = view.thread.snapshot().items[1].clone();
                let option = option.map(str::to_string);
                view.choose_permission("request-t".into(), option.clone(), cx);
                assert_eq!(receiver.try_permission(), Some(("request-t".into(), option.clone())));
                assert_eq!(view.pending_permissions.len(), 1);
                assert_eq!(view.pending_permissions[0].id, "request-other");
                assert!(matches!(&view.thread.snapshot().items[0], ThreadItem::Tool(tool) if tool.state == expected));
                assert_eq!(view.timeline_items[0].read(cx).item(), &view.thread.snapshot().items[0]);
                assert_eq!(view.thread.snapshot().items[1], other);
                let resolved = view.thread.snapshot().clone();
                let remaining = view.pending_permissions.clone();
                view.choose_permission("request-t".into(), option, cx);
                view.choose_permission("request-t".into(), None, cx);
                assert_eq!(receiver.try_permission(), None, "resolved requests must not send again");
                assert_eq!(view.pending_permissions, remaining);
                assert_eq!(view.thread.snapshot(), &resolved);
                assert!(view.entries.is_empty());
            });
        }
    }

    #[test]
    fn permission_invalid_and_disconnected_sends_keep_pending_and_tool_state() {
        use muxlane_acp::test_support::command_channel;
        let mut cx = gpui::TestAppContext::single();
        let window = cx.add_window(terminal_test_view);
        let entity = window.root(&mut cx).unwrap();
        let (handle, mut receiver) = command_channel();
        entity.update(&mut cx, |view, cx| {
            permission_fixture(view, cx);
            let before = view.thread.snapshot().clone();
            let pending = view.pending_permissions.clone();
            view.handle = Some(handle);
            view.choose_permission("request-t".into(), Some("invalid".into()), cx);
            view.choose_permission("missing".into(), None, cx);
            assert_eq!(receiver.try_permission(), None);
            assert!(view.entries.is_empty());
            assert_eq!(view.pending_permissions, pending);
            assert_eq!(view.thread.snapshot(), &before);
            drop(receiver);
            for option in [Some("allow-once"), Some("reject-once"), None] {
                view.choose_permission("request-t".into(), option.map(str::to_string), cx);
                assert!(matches!(view.entries.last(), Some(Entry::Error(error)) if error.contains("ACP worker stopped")));
                assert_eq!(view.pending_permissions, pending);
                assert_eq!(view.thread.snapshot(), &before);
            }
            view.handle = None;
            view.choose_permission("request-t".into(), None, cx);
            assert!(matches!(view.entries.last(), Some(Entry::Error(error)) if error.contains("not connected")));
            assert_eq!(view.pending_permissions, pending);
            assert_eq!(view.thread.snapshot(), &before);
        });
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
                tool_id: "tool-first".into(),
                context: String::new(),
                options: vec![option.clone()],
            },
        );
        queue_permission(
            &mut permissions,
            PendingPermission {
                id: "second".into(),
                title: "Second".into(),
                tool_id: "tool-second".into(),
                context: String::new(),
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
