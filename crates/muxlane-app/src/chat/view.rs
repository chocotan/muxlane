//! Native Chat view combining a virtualized GPUI timeline (`ListState`), cached Markdown/diff
//! cards, inline security approval cards, and `ChatComposer`.
use crate::chat::composer::{ChatComposer, ComposerEvent, SlashCmd};
use crate::chat::markdown::{parse_markdown_blocks, render_markdown_blocks, MdBlock};
use crate::chat::syntax::{compute_diff_lines, DiffLineKind};
use crate::i18n::{self, Language};
use crate::text_field::TextField;
use crate::theme::ThemeMode;
use crate::ui::{
    disclosure_chevron, rail_card, shortcut_chip, square_badge, square_button, ButtonVariant,
    ChatPalette, CHAT_CODE_PX, CHAT_MEASURE_MAX_PX, CHAT_META_PX, CHAT_MICRO_PX, CHAT_PROSE_PX,
};
use crate::ui_scale::px as ui_px;
use gpui::{
    div, list, prelude::*, rgba, AnyElement, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FollowMode, FontWeight, ListAlignment, ListState, Render, SharedString, Subscription, Window,
};
use muxlane_client::RemoteHost;
use muxlane_core::chat::{
    ApprovalOption, ApprovalReply, ChatAttachment, ChatDeltaEvent, ChatDeltaOp, ChatTimelineState,
    NoticeLevel, PendingApproval, PlanEntryStatus, SessionDriverMode, StreamingBehavior,
    TimelineEntry, ToolCallKind, ToolCallState, ToolDiff,
};
use muxlane_core::model::AgentId;
use muxlane_core::protocol::{
    events, ChatApproveParams, ChatConfigureParams, ChatHistoryResult, ChatSendParams,
};
use muxlane_server::MuxlaneServer;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Clone)]
pub(crate) enum ChatBackend {
    Local {
        server: Arc<MuxlaneServer>,
    },
    Remote {
        server: Arc<MuxlaneServer>,
        host: Arc<RemoteHost>,
    },
}

impl ChatBackend {
    fn server(&self) -> &Arc<MuxlaneServer> {
        match self {
            Self::Local { server } | Self::Remote { server, .. } => server,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum ChatViewEvent {
    OpenTerminal,
    TakeOverChat,
    SwitchToTerminalDriver,
}

enum TimelineStreamMsg {
    History(ChatHistoryResult),
    Delta(ChatDeltaEvent),
    Disconnected(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TimelineDisplayItem {
    Entry {
        index: usize,
    },
    ToolCluster {
        cluster_id: String,
        indices: Vec<usize>,
    },
}

#[derive(Clone)]
struct CachedMarkdown {
    len: usize,
    hash: u64,
    blocks: Arc<Vec<MdBlock>>,
}

pub(crate) struct ChatView {
    agent: AgentId,
    backend: ChatBackend,
    mono_font: SharedString,
    theme_mode: ThemeMode,
    language: Language,
    timeline: ChatTimelineState,
    display_items: Vec<TimelineDisplayItem>,
    list_state: ListState,
    is_following_tail: bool,
    composer: Entity<ChatComposer>,
    md_cache: Arc<Mutex<HashMap<String, CachedMarkdown>>>,
    expanded_thinking: HashSet<String>,
    expanded_tools: HashSet<String>,
    expanded_clusters: HashSet<String>,
    collapsed_diffs: HashSet<String>,
    approval_inputs: HashMap<String, Entity<TextField>>,
    stream_abort: Option<tokio::task::AbortHandle>,
    local_sub_id: Option<String>,
    connection_note: Option<String>,
    _composer_sub: Subscription,
}

impl EventEmitter<ChatViewEvent> for ChatView {}

impl Focusable for ChatView {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.composer.focus_handle(cx)
    }
}

impl Drop for ChatView {
    fn drop(&mut self) {
        if let Some(handle) = self.stream_abort.take() {
            handle.abort();
        }
        if let (ChatBackend::Local { server }, Some(sub_id)) =
            (&self.backend, self.local_sub_id.take())
        {
            let chat = Arc::clone(server.chat_manager());
            server.rt_spawn(async move {
                chat.unsubscribe(&sub_id).await;
            });
        }
    }
}

impl ChatView {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        agent: AgentId,
        backend: ChatBackend,
        project_root: Option<PathBuf>,
        mono_font: SharedString,
        theme_mode: ThemeMode,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let composer =
            cx.new(|cx| ChatComposer::new(project_root, theme_mode, language, window, cx));
        let _composer_sub = cx.subscribe(&composer, |this, _, event, cx| match event {
            ComposerEvent::Submit {
                text,
                attachments,
                behavior,
            } => this.send_prompt(text.clone(), attachments.clone(), *behavior, cx),
            ComposerEvent::Slash(SlashCmd::Clear) => {
                // Composer draft already cleared; timeline stays intact.
            }
            ComposerEvent::Slash(_) => {}
            ComposerEvent::Abort => this.abort_turn(cx),
            ComposerEvent::SelectModel { provider, model_id } => {
                this.configure_model(provider.clone(), Some(model_id.clone()), None, cx);
            }
            ComposerEvent::SelectThinking(level) => {
                this.configure_model(None, None, Some(level.clone()), cx);
            }
            ComposerEvent::SwitchTerminal => cx.emit(ChatViewEvent::OpenTerminal),
            ComposerEvent::TakeOverChat => cx.emit(ChatViewEvent::TakeOverChat),
        });

        let list_state = ListState::new(0, ListAlignment::Bottom, ui_px(220.0));
        list_state.set_follow_mode(FollowMode::Tail);
        let weak_self = cx.weak_entity();
        list_state.set_scroll_handler(move |ev, _window, cx| {
            let following = ev.is_following_tail;
            let _ = weak_self.update(cx, |this, cx| {
                if this.is_following_tail != following {
                    this.is_following_tail = following;
                    cx.notify();
                }
            });
        });

        let sub_id = format!("ui-chat-{agent}-{}", uuid_v4_short());
        let mut view = Self {
            agent,
            backend,
            mono_font,
            theme_mode,
            language,
            timeline: ChatTimelineState::default(),
            display_items: Vec::new(),
            list_state,
            is_following_tail: true,
            composer,
            md_cache: Arc::new(Mutex::new(HashMap::new())),
            expanded_thinking: HashSet::new(),
            expanded_tools: HashSet::new(),
            expanded_clusters: HashSet::new(),
            collapsed_diffs: HashSet::new(),
            approval_inputs: HashMap::new(),
            stream_abort: None,
            local_sub_id: Some(sub_id.clone()),
            connection_note: None,
            _composer_sub,
        };
        view.start_stream(sub_id, cx);
        view
    }

    pub(crate) fn set_theme_mode(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
        if self.theme_mode == mode {
            return;
        }
        self.theme_mode = mode;
        self.composer.update(cx, |c, cx| c.set_theme_mode(mode, cx));
        for field in self.approval_inputs.values() {
            field.update(cx, |f, cx| f.set_theme_mode(mode, cx));
        }
        cx.notify();
    }

    pub(crate) fn set_language(&mut self, language: Language, cx: &mut Context<Self>) {
        if self.language == language {
            return;
        }
        self.language = language;
        self.composer
            .update(cx, |c, cx| c.set_language(language, cx));
        cx.notify();
    }

    pub(crate) fn set_mono_font(&mut self, font: SharedString, cx: &mut Context<Self>) {
        if self.mono_font != font {
            self.mono_font = font;
            cx.notify();
        }
    }

    pub(crate) fn refresh_after_driver_change(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.stream_abort.take() {
            handle.abort();
        }
        if let (ChatBackend::Local { server }, Some(old_sub)) =
            (&self.backend, self.local_sub_id.take())
        {
            let chat = Arc::clone(server.chat_manager());
            server.rt_spawn(async move {
                chat.unsubscribe(&old_sub).await;
            });
        }
        let sub_id = format!("ui-chat-{}-{}", self.agent, uuid_v4_short());
        self.local_sub_id = Some(sub_id.clone());
        self.start_stream(sub_id, cx);
    }

    fn start_stream(&mut self, sub_id: String, cx: &mut Context<Self>) {
        let (tx, mut rx) = mpsc::unbounded_channel::<TimelineStreamMsg>();
        let agent = self.agent.clone();
        let backend = self.backend.clone();
        let rt_handle = self.backend.server().runtime_handle();

        let join = rt_handle.spawn(async move {
            match backend {
                ChatBackend::Local { server } => {
                    let _ = server.ensure_pty_bridge_for_agent(&agent).await;
                    let chat = server.chat_manager();
                    // Poll briefly until ChatManager has registered the session or PTY bridge.
                    for _ in 0..20 {
                        let (ev_tx, mut ev_rx) = mpsc::channel(256);
                        match chat.subscribe(&agent, sub_id.clone(), ev_tx).await {
                            Ok(history) => {
                                let _ = tx.send(TimelineStreamMsg::History(history));
                                while let Some(ev) = ev_rx.recv().await {
                                    if ev.event == events::CHAT_DELTA {
                                        if let Ok(delta) =
                                            serde_json::from_value::<ChatDeltaEvent>(ev.params)
                                        {
                                            if delta.agent == agent
                                                && tx.send(TimelineStreamMsg::Delta(delta)).is_err()
                                            {
                                                break;
                                            }
                                        }
                                    }
                                }
                                return;
                            }
                            Err(_) => {
                                let _ = server.ensure_pty_bridge_for_agent(&agent).await;
                                tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                            }
                        }
                    }
                    let _ = tx.send(TimelineStreamMsg::Disconnected(
                        "chat session unavailable".into(),
                    ));
                }
                ChatBackend::Remote { host, .. } => loop {
                    let Some(endpoint) = host.endpoint_now() else {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        continue;
                    };
                    let tx_inner = tx.clone();
                    let res = muxlane_client::stream_chat(&endpoint, &agent, move |update| {
                        let msg = match update {
                            muxlane_client::ChatStreamUpdate::History(h) => {
                                TimelineStreamMsg::History(h)
                            }
                            muxlane_client::ChatStreamUpdate::Delta(d) => {
                                TimelineStreamMsg::Delta(d)
                            }
                        };
                        let _ = tx_inner.send(msg);
                    })
                    .await;
                    if let Err(err) = res {
                        if tx
                            .send(TimelineStreamMsg::Disconnected(err.to_string()))
                            .is_err()
                        {
                            return;
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                },
            }
        });
        self.stream_abort = Some(join.abort_handle());

        cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.recv().await {
                let mut batch = vec![msg];
                while let Ok(more) = rx.try_recv() {
                    batch.push(more);
                }
                let ok = this
                    .update(cx, |this, cx| {
                        for item in batch {
                            match item {
                                TimelineStreamMsg::History(history) => {
                                    this.apply_history(history, cx);
                                }
                                TimelineStreamMsg::Delta(delta) => {
                                    this.apply_delta(delta, cx);
                                }
                                TimelineStreamMsg::Disconnected(err) => {
                                    this.connection_note = Some(err);
                                    cx.notify();
                                }
                            }
                        }
                    })
                    .is_ok();
                if !ok {
                    break;
                }
            }
        })
        .detach();
    }

    fn apply_history(&mut self, history: ChatHistoryResult, cx: &mut Context<Self>) {
        self.connection_note = None;
        self.timeline = ChatTimelineState::from_snapshot(
            history.seq,
            history.meta,
            history.entries,
            history.pending_approvals,
        );
        self.composer
            .update(cx, |c, cx| c.set_meta(self.timeline.meta.clone(), cx));
        self.rebuild_display_items(true);
        self.retain_approval_inputs();
        cx.notify();
    }

    fn apply_delta(&mut self, delta: ChatDeltaEvent, cx: &mut Context<Self>) {
        if delta.seq <= self.timeline.seq {
            return;
        }
        self.connection_note = None;
        let is_reset = matches!(delta.delta, ChatDeltaOp::Reset);
        let is_in_place_text = matches!(
            delta.delta,
            ChatDeltaOp::AppendText { .. }
                | ChatDeltaOp::ReplaceEntryText { .. }
                | ChatDeltaOp::FinishEntry { .. }
        );
        self.timeline.apply(delta);
        if is_reset {
            if let Ok(mut cache) = self.md_cache.lock() {
                cache.clear();
            }
        }
        self.composer
            .update(cx, |c, cx| c.set_meta(self.timeline.meta.clone(), cx));
        self.rebuild_display_items(!is_in_place_text);
        self.retain_approval_inputs();
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn apply_test_history(
        &mut self,
        history: ChatHistoryResult,
        cx: &mut Context<Self>,
    ) {
        self.apply_history(history, cx);
    }

    fn retain_approval_inputs(&mut self) {
        let active: HashSet<&str> = self
            .timeline
            .pending_approvals
            .iter()
            .map(|a| a.id.as_str())
            .collect();
        self.approval_inputs
            .retain(|id, _| active.contains(id.as_str()));
    }

    fn rebuild_display_items(&mut self, allow_structural_update: bool) {
        let new_items = build_display_items(&self.timeline.entries);
        let old_len = self.display_items.len();
        let new_len = new_items.len();

        if old_len == new_len && !allow_structural_update && new_len > 0 {
            self.display_items = new_items;
            self.list_state.remeasure_items(new_len - 1..new_len);
            return;
        }

        if old_len == new_len && self.display_items == new_items && new_len > 0 {
            self.list_state.remeasure_items(new_len - 1..new_len);
            return;
        }

        if old_len > 0
            && new_len >= old_len
            && self.display_items[..old_len - 1] == new_items[..old_len - 1]
        {
            self.display_items = new_items;
            self.list_state
                .splice(old_len - 1..old_len, new_len - (old_len - 1));
            return;
        }

        self.display_items = new_items;
        self.list_state.reset(self.display_items.len());
        self.list_state.set_follow_mode(FollowMode::Tail);
    }

    fn send_prompt(
        &mut self,
        text: String,
        attachments: Vec<ChatAttachment>,
        behavior: Option<StreamingBehavior>,
        cx: &mut Context<Self>,
    ) {
        let agent = self.agent.clone();
        let backend = self.backend.clone();
        self.backend.server().rt_spawn(async move {
            let res = match backend {
                ChatBackend::Local { server } => {
                    server
                        .chat_manager()
                        .send_prompt(&agent, text, attachments, behavior)
                        .await
                }
                ChatBackend::Remote { host, .. } => {
                    host.chat_send(ChatSendParams {
                        agent,
                        text,
                        attachments,
                        streaming_behavior: behavior,
                        client_msg_id: None,
                    })
                    .await
                }
            };
            if let Err(err) = res {
                tracing::warn!(%err, "chat send failed");
            }
        });
        cx.notify();
    }

    fn abort_turn(&mut self, _cx: &mut Context<Self>) {
        let agent = self.agent.clone();
        let backend = self.backend.clone();
        self.backend.server().rt_spawn(async move {
            let res = match backend {
                ChatBackend::Local { server } => server.chat_manager().abort(&agent).await,
                ChatBackend::Remote { host, .. } => host.chat_abort(&agent).await,
            };
            if let Err(err) = res {
                tracing::warn!(%err, "chat abort failed");
            }
        });
    }

    fn configure_model(
        &mut self,
        provider: Option<String>,
        model_id: Option<String>,
        thinking_level: Option<String>,
        _cx: &mut Context<Self>,
    ) {
        let agent = self.agent.clone();
        let backend = self.backend.clone();
        self.backend.server().rt_spawn(async move {
            let res = match backend {
                ChatBackend::Local { server } => {
                    server
                        .chat_manager()
                        .configure(&agent, provider, model_id, thinking_level)
                        .await
                }
                ChatBackend::Remote { host, .. } => {
                    host.chat_configure(ChatConfigureParams {
                        agent,
                        provider,
                        model_id,
                        thinking_level,
                    })
                    .await
                }
            };
            if let Err(err) = res {
                tracing::warn!(%err, "chat configure failed");
            }
        });
    }

    fn respond_approval(
        &mut self,
        approval_id: String,
        reply: ApprovalReply,
        cx: &mut Context<Self>,
    ) {
        self.approval_inputs.remove(&approval_id);
        let agent = self.agent.clone();
        let backend = self.backend.clone();
        self.backend.server().rt_spawn(async move {
            let res = match backend {
                ChatBackend::Local { server } => {
                    server
                        .chat_manager()
                        .approve(&agent, approval_id, reply)
                        .await
                }
                ChatBackend::Remote { host, .. } => {
                    host.chat_approve(ChatApproveParams {
                        agent,
                        approval_id,
                        reply,
                    })
                    .await
                }
            };
            if let Err(err) = res {
                tracing::warn!(%err, "chat approve failed");
            }
        });
        cx.notify();
    }

    fn cached_markdown(&self, id: &str, text: &str) -> Arc<Vec<MdBlock>> {
        let len = text.len();
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();
        if let Ok(cache) = self.md_cache.lock() {
            if let Some(hit) = cache.get(id) {
                if hit.len == len && hit.hash == hash {
                    return Arc::clone(&hit.blocks);
                }
            }
        }
        let blocks = Arc::new(parse_markdown_blocks(text));
        if let Ok(mut cache) = self.md_cache.lock() {
            if cache.len() > 400 {
                cache.clear();
            }
            cache.insert(
                id.to_string(),
                CachedMarkdown {
                    len,
                    hash,
                    blocks: Arc::clone(&blocks),
                },
            );
        }
        blocks
    }

    fn render_header(&mut self, palette: ChatPalette, cx: &mut Context<Self>) -> AnyElement {
        let driver = self.timeline.meta.driver;
        let is_live_bridge =
            driver == SessionDriverMode::TerminalPty && self.timeline.meta.supports_chat_input;
        let driver_label = match driver {
            SessionDriverMode::PiRpc => "PI RPC",
            SessionDriverMode::Acp => "ACP",
            SessionDriverMode::TerminalPty if is_live_bridge => "PTY SYNC",
            SessionDriverMode::TerminalPty => "PTY READONLY",
        };
        let is_structured = driver != SessionDriverMode::TerminalPty;
        let is_interactive = is_structured || is_live_bridge;
        let model_label = self
            .timeline
            .meta
            .current_model
            .as_ref()
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "auto".to_string());
        let session_short = self.timeline.meta.provider_session_id.as_deref().map(|s| {
            if s.len() > 8 {
                &s[..8]
            } else {
                s
            }
        });
        let streaming = self.timeline.meta.is_streaming;
        let has_approval = !self.timeline.pending_approvals.is_empty();

        div()
            .w_full()
            .h(ui_px(32.))
            .px_3()
            .bg(rgba(palette.card))
            .border_b_1()
            .border_color(rgba(palette.hairline))
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(square_badge(
                        driver_label,
                        palette.theme.on_accent,
                        if is_interactive {
                            palette.theme.accent
                        } else {
                            palette.fg_faint
                        },
                        palette.hairline,
                    ))
                    .child(
                        div()
                            .text_size(ui_px(CHAT_META_PX))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgba(palette.fg_strong))
                            .child(model_label),
                    )
                    .when_some(session_short, |el, short| {
                        el.child(shortcut_chip(format!("#{short}"), palette))
                    })
                    .when(has_approval, |el| {
                        el.child(square_badge(
                            i18n::text(self.language, "chat.approval_required").to_string(),
                            palette.fg_strong,
                            palette.security_surface,
                            palette.security_border,
                        ))
                    })
                    .when(streaming && !has_approval, |el| {
                        el.child(square_badge(
                            "STREAMING",
                            palette.theme.accent,
                            palette.user_surface,
                            palette.theme.accent,
                        ))
                    }),
            )
            .child(div().flex().items_center().gap_1p5().when(
                self.timeline.meta.supports_terminal_switch,
                |el| {
                    let label = if is_structured {
                        i18n::text(self.language, "chat.switch_to_term")
                    } else {
                        i18n::text(self.language, "chat.switch_to_chat")
                    };
                    el.child(
                        square_button(
                            "chat-hdr-switch-driver",
                            label,
                            ButtonVariant::Outline,
                            palette,
                        )
                        .on_click(cx.listener(move |_this, _, _, cx| {
                            if is_structured {
                                cx.emit(ChatViewEvent::SwitchToTerminalDriver);
                            } else {
                                cx.emit(ChatViewEvent::TakeOverChat);
                            }
                        })),
                    )
                },
            ))
            .into_any_element()
    }

    fn render_empty_state(&self, palette: ChatPalette, cx: &mut Context<Self>) -> AnyElement {
        let starters = [
            "Summarize this repository architecture and entry points.",
            "Run tests in the workspace and diagnose any failures.",
            "Review current uncommitted changes for bugs or edge cases.",
        ];
        div()
            .flex_1()
            .w_full()
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .child(
                div()
                    .w_full()
                    .max_w(ui_px(560.))
                    .bg(rgba(palette.card))
                    .border_1()
                    .border_color(rgba(palette.hairline))
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_size(ui_px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgba(palette.fg_strong))
                            .child(i18n::text(self.language, "chat.empty_title")),
                    )
                    .child(
                        div()
                            .text_size(ui_px(CHAT_PROSE_PX))
                            .text_color(rgba(palette.fg_muted))
                            .child(i18n::text(self.language, "chat.empty_body")),
                    )
                    .when(self.timeline.meta.supports_chat_input, |el| {
                        let mut chips = div().flex().flex_col().gap_1p5().pt_1();
                        for (idx, prompt) in starters.into_iter().enumerate() {
                            let text_str = prompt.to_string();
                            chips = chips.child(
                                div()
                                    .id(gpui::ElementId::Name(format!("chat-starter-{idx}").into()))
                                    .px_2p5()
                                    .py_1p5()
                                    .bg(rgba(palette.inset))
                                    .border_1()
                                    .border_color(rgba(palette.hairline))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgba(palette.row_hover)))
                                    .text_size(ui_px(CHAT_META_PX))
                                    .text_color(rgba(palette.fg_strong))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        let p = text_str.clone();
                                        this.composer.update(cx, |c, cx| {
                                            c.set_draft(p, window, cx);
                                        });
                                    }))
                                    .child(prompt),
                            );
                        }
                        el.child(chips)
                    }),
            )
            .into_any_element()
    }

    fn render_approval_cards(
        &mut self,
        palette: ChatPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.timeline.pending_approvals.is_empty() {
            return None;
        }
        let approvals = self.timeline.pending_approvals.clone();
        for approval in &approvals {
            if matches!(approval.method.as_str(), "input" | "editor")
                && !self.approval_inputs.contains_key(&approval.id)
            {
                let prefill = approval.prefill.clone().unwrap_or_default();
                let mode = self.theme_mode;
                let field = cx.new(|cx| {
                    let mut f = TextField::new("Enter value…", window, cx);
                    f.set_theme_mode(mode, cx);
                    if !prefill.is_empty() {
                        f.set_text(prefill, cx);
                    }
                    f
                });
                self.approval_inputs.insert(approval.id.clone(), field);
            }
        }

        let mut wrap = div()
            .w_full()
            .max_w(ui_px(CHAT_MEASURE_MAX_PX))
            .mx_auto()
            .px_4()
            .pb_2()
            .flex()
            .flex_col()
            .gap_2();

        for approval in approvals {
            wrap = wrap.child(self.render_single_approval(&approval, palette, cx));
        }
        Some(wrap.into_any_element())
    }

    fn render_single_approval(
        &self,
        approval: &PendingApproval,
        palette: ChatPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = approval.id.clone();
        let text_field = self.approval_inputs.get(&id).cloned();

        let mut body = div()
            .p_3()
            .bg(rgba(palette.security_surface))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(square_badge(
                                i18n::text(self.language, "chat.approval_required").to_string(),
                                palette.theme.on_accent,
                                palette.security_border,
                                palette.security_border,
                            ))
                            .child(
                                div()
                                    .text_size(ui_px(CHAT_PROSE_PX))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgba(palette.fg_strong))
                                    .child(approval.title.clone()),
                            ),
                    )
                    .when_some(approval.tool_name.clone(), |el, tool| {
                        el.child(shortcut_chip(tool, palette))
                    }),
            )
            .when_some(approval.message.clone(), |el, msg| {
                el.child(
                    div()
                        .text_size(ui_px(CHAT_META_PX))
                        .text_color(rgba(palette.fg_prose))
                        .child(msg),
                )
            })
            .when_some(approval.command_or_input.clone(), |el, cmd| {
                el.child(
                    div()
                        .w_full()
                        .p_2()
                        .bg(rgba(palette.code_surface))
                        .border_1()
                        .border_color(rgba(palette.hairline))
                        .font_family(self.mono_font.clone())
                        .text_size(ui_px(CHAT_CODE_PX))
                        .text_color(rgba(palette.fg_strong))
                        .child(cmd),
                )
            });

        if let Some(field) = text_field {
            let aid_submit = id.clone();
            let aid_cancel = id.clone();
            let field_read = field.clone();
            body = body.child(field).child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1p5()
                    .child(
                        square_button(
                            format!("app-cancel-{id}"),
                            i18n::text(self.language, "common.cancel"),
                            ButtonVariant::Outline,
                            palette,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.respond_approval(aid_cancel.clone(), ApprovalReply::Cancel, cx);
                        })),
                    )
                    .child(
                        square_button(
                            format!("app-submit-{id}"),
                            i18n::text(self.language, "chat.reply_submit"),
                            ButtonVariant::Primary,
                            palette,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let val = field_read.read(cx).text();
                            this.respond_approval(
                                aid_submit.clone(),
                                ApprovalReply::Text { value: val },
                                cx,
                            );
                        })),
                    ),
            );
        } else if !approval.options.is_empty() {
            let mut actions = div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_end()
                .gap_1p5();
            for (idx, opt) in approval.options.iter().enumerate() {
                let aid = id.clone();
                let oid = opt.option_id.clone();
                let variant = approval_option_variant(opt);
                actions = actions.child(
                    square_button(
                        format!("app-opt-{id}-{idx}"),
                        opt.label.clone(),
                        variant,
                        palette,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.respond_approval(
                            aid.clone(),
                            ApprovalReply::SelectedOption {
                                option_id: oid.clone(),
                            },
                            cx,
                        );
                    })),
                );
            }
            body = body.child(actions);
        } else {
            let aid_deny = id.clone();
            let aid_allow = id.clone();
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1p5()
                    .child(
                        square_button(
                            format!("app-deny-{id}"),
                            i18n::text(self.language, "chat.deny"),
                            ButtonVariant::Danger,
                            palette,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.respond_approval(
                                aid_deny.clone(),
                                ApprovalReply::Confirm { confirmed: false },
                                cx,
                            );
                        })),
                    )
                    .child(
                        square_button(
                            format!("app-allow-{id}"),
                            i18n::text(self.language, "chat.allow"),
                            ButtonVariant::Primary,
                            palette,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.respond_approval(
                                aid_allow.clone(),
                                ApprovalReply::Confirm { confirmed: true },
                                cx,
                            );
                        })),
                    ),
            );
        }

        rail_card(palette, Some(palette.security_border), body).into_any_element()
    }

    fn render_timeline_item(
        &mut self,
        ix: usize,
        palette: ChatPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(display) = self.display_items.get(ix).cloned() else {
            return div().into_any_element();
        };
        let row_body = match display {
            TimelineDisplayItem::Entry { index } => {
                let Some(entry) = self.timeline.entries.get(index).cloned() else {
                    return div().into_any_element();
                };
                self.render_entry(&entry, palette, cx)
            }
            TimelineDisplayItem::ToolCluster {
                cluster_id,
                indices,
            } => self.render_tool_cluster(&cluster_id, &indices, palette, cx),
        };
        div()
            .w_full()
            .px_4()
            .py_1()
            .child(
                div()
                    .w_full()
                    .max_w(ui_px(CHAT_MEASURE_MAX_PX))
                    .mx_auto()
                    .child(row_body),
            )
            .into_any_element()
    }

    fn render_entry(
        &self,
        entry: &TimelineEntry,
        palette: ChatPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match entry {
            TimelineEntry::UserMessage {
                id,
                text,
                attachments,
                ..
            } => {
                let blocks = self.cached_markdown(id, text);
                div()
                    .w_full()
                    .p_3()
                    .bg(rgba(palette.user_surface))
                    .border_1()
                    .border_color(rgba(palette.hairline))
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(ui_px(CHAT_MICRO_PX))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgba(palette.theme.accent))
                                    .child(i18n::text(self.language, "chat.user_label")),
                            )
                            .when(!attachments.is_empty(), |el| {
                                let mut row = div().flex().flex_wrap().gap_1();
                                for att in attachments {
                                    row = row.child(shortcut_chip(att.name.clone(), palette));
                                }
                                el.child(row)
                            }),
                    )
                    .child(render_markdown_blocks(
                        id,
                        &blocks,
                        palette,
                        &self.mono_font,
                    ))
                    .into_any_element()
            }
            TimelineEntry::AssistantText {
                id,
                text,
                streaming,
                ..
            } => {
                let blocks = self.cached_markdown(id, text);
                div()
                    .w_full()
                    .py_1p5()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(render_markdown_blocks(
                        id,
                        &blocks,
                        palette,
                        &self.mono_font,
                    ))
                    .when(*streaming, |el| {
                        el.child(
                            div().flex().items_center().gap_1().child(
                                div()
                                    .w(ui_px(7.))
                                    .h(ui_px(13.))
                                    .bg(rgba(palette.theme.accent)),
                            ),
                        )
                    })
                    .into_any_element()
            }
            TimelineEntry::Thinking {
                id,
                text,
                streaming,
                duration_ms,
                ..
            } => {
                let expanded = self.expanded_thinking.contains(id) || *streaming;
                let preview = one_line_preview(text, 72);
                let dur = duration_ms.map(format_duration);
                let toggle_id = id.clone();

                let mut header = div()
                    .id(gpui::ElementId::Name(format!("think-hdr-{id}").into()))
                    .px_2p5()
                    .py_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgba(palette.row_hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded_thinking.remove(&toggle_id) {
                            this.expanded_thinking.insert(toggle_id.clone());
                        }
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .min_w_0()
                            .child(disclosure_chevron(expanded, palette))
                            .child(
                                div()
                                    .text_size(ui_px(CHAT_META_PX))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgba(palette.fg_muted))
                                    .child(i18n::text(self.language, "chat.thinking_label")),
                            )
                            .when(!expanded && !preview.is_empty(), |el| {
                                el.child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .text_size(ui_px(CHAT_MICRO_PX))
                                        .text_color(rgba(palette.fg_faint))
                                        .child(preview),
                                )
                            }),
                    );
                if let Some(d) = dur {
                    header = header.child(
                        div()
                            .text_size(ui_px(CHAT_MICRO_PX))
                            .text_color(rgba(palette.fg_faint))
                            .child(d),
                    );
                }

                div()
                    .w_full()
                    .bg(rgba(palette.inset))
                    .border_1()
                    .border_color(rgba(palette.hairline))
                    .flex()
                    .flex_col()
                    .child(header)
                    .when(expanded && !text.trim().is_empty(), |el| {
                        el.child(
                            div()
                                .px_3()
                                .py_2()
                                .border_t_1()
                                .border_color(rgba(palette.subtle_line))
                                .text_size(ui_px(CHAT_META_PX))
                                .line_height(ui_px(18.0))
                                .text_color(rgba(palette.fg_muted))
                                .child(text.clone()),
                        )
                    })
                    .into_any_element()
            }
            TimelineEntry::ToolCall {
                id,
                name,
                kind,
                title,
                input,
                output,
                diff,
                exit_code,
                locations,
                state,
                duration_ms,
                ..
            } => {
                if let Some(diff) = diff.as_deref() {
                    return self.render_diff_tool_card(
                        id,
                        name,
                        title.as_deref(),
                        diff,
                        *state,
                        *duration_ms,
                        palette,
                        cx,
                    );
                }
                if *kind == ToolCallKind::Bash {
                    return self.render_bash_tool_card(
                        id,
                        title.as_deref(),
                        input,
                        output.as_deref(),
                        *exit_code,
                        *state,
                        *duration_ms,
                        palette,
                        cx,
                    );
                }
                self.render_generic_tool_row(
                    id,
                    name,
                    title.as_deref(),
                    input,
                    output.as_deref(),
                    locations,
                    *state,
                    *duration_ms,
                    palette,
                    cx,
                )
            }
            TimelineEntry::Plan { entries, .. } => {
                let mut box_el = div()
                    .w_full()
                    .p_3()
                    .bg(rgba(palette.card))
                    .border_1()
                    .border_color(rgba(palette.hairline))
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        div()
                            .text_size(ui_px(CHAT_MICRO_PX))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgba(palette.fg_faint))
                            .child("PLAN"),
                    );
                for item in entries {
                    let (mark, fg) = match item.status {
                        PlanEntryStatus::Completed => ("✓", palette.theme.green),
                        PlanEntryStatus::InProgress => ("●", palette.theme.accent),
                        PlanEntryStatus::Pending => ("○", palette.fg_faint),
                    };
                    box_el = box_el.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(ui_px(CHAT_META_PX))
                            .child(div().text_color(rgba(fg)).child(mark))
                            .child(
                                div()
                                    .flex_1()
                                    .text_color(rgba(palette.fg_prose))
                                    .child(item.content.clone()),
                            )
                            .when_some(item.priority.clone(), |el, p| {
                                el.child(shortcut_chip(p, palette))
                            }),
                    );
                }
                box_el.into_any_element()
            }
            TimelineEntry::Notice { level, text, .. } => {
                let (bg, border, fg) = match level {
                    NoticeLevel::Error => {
                        (palette.diff_del_bg, palette.theme.red, palette.diff_del_fg)
                    }
                    NoticeLevel::Warning => (
                        palette.security_surface,
                        palette.security_border,
                        palette.fg_strong,
                    ),
                    NoticeLevel::Compaction | NoticeLevel::Info => {
                        (palette.inset, palette.hairline, palette.fg_muted)
                    }
                };
                div()
                    .w_full()
                    .px_3()
                    .py_1p5()
                    .bg(rgba(bg))
                    .border_1()
                    .border_color(rgba(border))
                    .text_size(ui_px(CHAT_META_PX))
                    .text_color(rgba(fg))
                    .child(text.clone())
                    .into_any_element()
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_diff_tool_card(
        &self,
        id: &str,
        name: &str,
        title: Option<&str>,
        diff: &ToolDiff,
        state: ToolCallState,
        duration_ms: Option<u64>,
        palette: ChatPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = !self.collapsed_diffs.contains(id);
        let path_label = diff.path.as_deref().or(title).unwrap_or(name).to_string();
        let (lines, truncated) = compute_diff_lines(&diff.old_text, &diff.new_text, 140);
        let added = lines.iter().filter(|l| l.kind == DiffLineKind::Add).count();
        let removed = lines.iter().filter(|l| l.kind == DiffLineKind::Del).count();
        let toggle_id = id.to_string();
        let rail = tool_state_color(state, palette);

        let mut content = div().flex().flex_col().child(
            div()
                .id(gpui::ElementId::Name(format!("diff-hdr-{id}").into()))
                .px_2p5()
                .py_1p5()
                .bg(rgba(palette.inset))
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .cursor_pointer()
                .hover(|s| s.bg(rgba(palette.row_hover)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.collapsed_diffs.remove(&toggle_id) {
                        this.collapsed_diffs.insert(toggle_id.clone());
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .min_w_0()
                        .child(disclosure_chevron(expanded, palette))
                        .child(square_badge(
                            "EDIT",
                            palette.theme.accent,
                            palette.user_surface,
                            palette.theme.accent,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .font_family(self.mono_font.clone())
                                .text_size(ui_px(CHAT_META_PX))
                                .text_color(rgba(palette.fg_strong))
                                .child(path_label),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .when(added > 0, |el| {
                            el.child(
                                div()
                                    .text_size(ui_px(CHAT_MICRO_PX))
                                    .text_color(rgba(palette.diff_add_fg))
                                    .child(format!("+{added}")),
                            )
                        })
                        .when(removed > 0, |el| {
                            el.child(
                                div()
                                    .text_size(ui_px(CHAT_MICRO_PX))
                                    .text_color(rgba(palette.diff_del_fg))
                                    .child(format!("-{removed}")),
                            )
                        })
                        .when_some(duration_ms.map(format_duration), |el, d| {
                            el.child(
                                div()
                                    .text_size(ui_px(CHAT_MICRO_PX))
                                    .text_color(rgba(palette.fg_faint))
                                    .child(d),
                            )
                        }),
                ),
        );

        if expanded && !lines.is_empty() {
            let mut diff_body = div()
                .w_full()
                .border_t_1()
                .border_color(rgba(palette.hairline))
                .bg(rgba(palette.code_surface))
                .font_family(self.mono_font.clone())
                .text_size(ui_px(CHAT_CODE_PX))
                .line_height(ui_px(18.0))
                .flex()
                .flex_col();
            for line in &lines {
                let (bg, fg, prefix) = match line.kind {
                    DiffLineKind::Add => (palette.diff_add_bg, palette.diff_add_fg, "+"),
                    DiffLineKind::Del => (palette.diff_del_bg, palette.diff_del_fg, "-"),
                    DiffLineKind::Context => (palette.code_surface, palette.fg_muted, " "),
                };
                let ln = line
                    .new_line
                    .or(line.old_line)
                    .map(|n| format!("{n:>4}"))
                    .unwrap_or_else(|| "   ·".to_string());
                diff_body = diff_body.child(
                    div()
                        .w_full()
                        .px_2()
                        .bg(rgba(bg))
                        .flex()
                        .gap_2()
                        .child(
                            div()
                                .w(ui_px(36.))
                                .flex_none()
                                .text_color(rgba(palette.fg_faint))
                                .child(ln),
                        )
                        .child(div().flex_none().text_color(rgba(fg)).child(prefix))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_color(rgba(fg))
                                .child(line.text.clone()),
                        ),
                );
            }
            if truncated {
                diff_body = diff_body.child(
                    div()
                        .px_2p5()
                        .py_1()
                        .text_size(ui_px(CHAT_MICRO_PX))
                        .text_color(rgba(palette.fg_faint))
                        .child("… diff truncated"),
                );
            }
            content = content.child(diff_body);
        }

        rail_card(palette, Some(rail), content).into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_bash_tool_card(
        &self,
        id: &str,
        title: Option<&str>,
        input: &serde_json::Value,
        output: Option<&str>,
        exit_code: Option<i32>,
        state: ToolCallState,
        duration_ms: Option<u64>,
        palette: ChatPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cmd = input
            .get("command")
            .and_then(|v| v.as_str())
            .or(title)
            .unwrap_or("bash")
            .to_string();
        let has_output = output.is_some_and(|o| !o.trim().is_empty());
        let expanded = self.expanded_tools.contains(id)
            || state == ToolCallState::Running
            || exit_code.is_some_and(|c| c != 0);
        let toggle_id = id.to_string();
        let rail = tool_state_color(state, palette);

        let mut card_body = div().flex().flex_col().child(
            div()
                .id(gpui::ElementId::Name(format!("bash-hdr-{id}").into()))
                .px_2p5()
                .py_1p5()
                .bg(rgba(palette.inset))
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .cursor_pointer()
                .hover(|s| s.bg(rgba(palette.row_hover)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.expanded_tools.remove(&toggle_id) {
                        this.expanded_tools.insert(toggle_id.clone());
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .min_w_0()
                        .child(disclosure_chevron(expanded, palette))
                        .child(
                            div()
                                .text_size(ui_px(CHAT_META_PX))
                                .font_family(self.mono_font.clone())
                                .text_color(rgba(palette.theme.accent))
                                .child("$"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .font_family(self.mono_font.clone())
                                .text_size(ui_px(CHAT_META_PX))
                                .text_color(rgba(palette.fg_strong))
                                .child(one_line_preview(&cmd, 96)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .when_some(exit_code, |el, code| {
                            let (bg, fg, border) = if code == 0 {
                                (
                                    palette.diff_add_bg,
                                    palette.diff_add_fg,
                                    palette.theme.green,
                                )
                            } else {
                                (palette.diff_del_bg, palette.diff_del_fg, palette.theme.red)
                            };
                            el.child(square_badge(format!("exit {code}"), fg, bg, border))
                        })
                        .when_some(duration_ms.map(format_duration), |el, d| {
                            el.child(
                                div()
                                    .text_size(ui_px(CHAT_MICRO_PX))
                                    .text_color(rgba(palette.fg_faint))
                                    .child(d),
                            )
                        }),
                ),
        );

        if expanded && has_output {
            let trimmed = truncate_tail(output.unwrap_or_default(), 2400);
            card_body = card_body.child(
                div()
                    .id(gpui::ElementId::Name(format!("bash-out-{id}").into()))
                    .w_full()
                    .max_h(ui_px(260.))
                    .overflow_y_scroll()
                    .p_2p5()
                    .border_t_1()
                    .border_color(rgba(palette.hairline))
                    .bg(rgba(palette.code_surface))
                    .font_family(self.mono_font.clone())
                    .text_size(ui_px(CHAT_CODE_PX))
                    .line_height(ui_px(18.0))
                    .text_color(rgba(palette.fg_prose))
                    .child(trimmed),
            );
        }

        rail_card(palette, Some(rail), card_body).into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_generic_tool_row(
        &self,
        id: &str,
        name: &str,
        title: Option<&str>,
        input: &serde_json::Value,
        output: Option<&str>,
        locations: &[String],
        state: ToolCallState,
        duration_ms: Option<u64>,
        palette: ChatPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = self.expanded_tools.contains(id);
        let toggle_id = id.to_string();
        let dot = tool_state_color(state, palette);
        let detail = title
            .map(str::to_string)
            .or_else(|| locations.first().cloned())
            .or_else(|| compact_input_hint(input))
            .unwrap_or_default();

        let mut box_el = div()
            .w_full()
            .bg(rgba(palette.card))
            .border_1()
            .border_color(rgba(palette.hairline))
            .flex()
            .flex_col()
            .child(
                div()
                    .id(gpui::ElementId::Name(format!("tool-hdr-{id}").into()))
                    .px_2p5()
                    .py_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgba(palette.row_hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded_tools.remove(&toggle_id) {
                            this.expanded_tools.insert(toggle_id.clone());
                        }
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .min_w_0()
                            .child(disclosure_chevron(expanded, palette))
                            .child(div().w(ui_px(6.)).h(ui_px(6.)).flex_none().bg(rgba(dot)))
                            .child(
                                div()
                                    .text_size(ui_px(CHAT_META_PX))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgba(palette.fg_strong))
                                    .child(name.to_string()),
                            )
                            .when(!detail.is_empty(), |el| {
                                el.child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .font_family(self.mono_font.clone())
                                        .text_size(ui_px(CHAT_MICRO_PX))
                                        .text_color(rgba(palette.fg_faint))
                                        .child(detail),
                                )
                            }),
                    )
                    .when_some(duration_ms.map(format_duration), |el, d| {
                        el.child(
                            div()
                                .text_size(ui_px(CHAT_MICRO_PX))
                                .text_color(rgba(palette.fg_faint))
                                .child(d),
                        )
                    }),
            );

        if expanded {
            let input_str = serde_json::to_string_pretty(input).unwrap_or_default();
            let mut details = div()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(rgba(palette.subtle_line))
                .bg(rgba(palette.code_surface))
                .font_family(self.mono_font.clone())
                .text_size(ui_px(CHAT_MICRO_PX))
                .text_color(rgba(palette.fg_muted))
                .flex()
                .flex_col()
                .gap_1p5();
            if input_str != "{}" && input_str != "null" {
                details = details.child(truncate_tail(&input_str, 800));
            }
            if let Some(out) = output.filter(|o| !o.trim().is_empty()) {
                details = details.child(
                    div()
                        .pt_1()
                        .border_t_1()
                        .border_color(rgba(palette.subtle_line))
                        .text_color(rgba(palette.fg_prose))
                        .child(truncate_tail(out, 1400)),
                );
            }
            box_el = box_el.child(details);
        }

        box_el.into_any_element()
    }

    fn render_tool_cluster(
        &self,
        cluster_id: &str,
        indices: &[usize],
        palette: ChatPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = self.expanded_clusters.contains(cluster_id);
        let toggle_id = cluster_id.to_string();
        let mut running = 0usize;
        let mut failed = 0usize;
        let mut total_ms = 0u64;
        for &idx in indices {
            if let Some(TimelineEntry::ToolCall {
                state, duration_ms, ..
            }) = self.timeline.entries.get(idx)
            {
                if matches!(state, ToolCallState::Running | ToolCallState::Pending) {
                    running += 1;
                } else if *state == ToolCallState::Failed {
                    failed += 1;
                }
                total_ms += duration_ms.unwrap_or(0);
            }
        }
        let dot = if failed > 0 {
            palette.theme.red
        } else if running > 0 {
            palette.theme.yellow
        } else {
            palette.theme.green
        };
        let summary = format!(
            "{} {}",
            indices.len(),
            i18n::text(self.language, "chat.tools_cluster")
        );

        let mut card = div()
            .w_full()
            .bg(rgba(palette.inset))
            .border_1()
            .border_color(rgba(palette.hairline))
            .flex()
            .flex_col()
            .child(
                div()
                    .id(gpui::ElementId::Name(
                        format!("cluster-{cluster_id}").into(),
                    ))
                    .px_2p5()
                    .py_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgba(palette.row_hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded_clusters.remove(&toggle_id) {
                            this.expanded_clusters.insert(toggle_id.clone());
                        }
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(disclosure_chevron(expanded, palette))
                            .child(div().w(ui_px(6.)).h(ui_px(6.)).flex_none().bg(rgba(dot)))
                            .child(
                                div()
                                    .text_size(ui_px(CHAT_META_PX))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgba(palette.fg_muted))
                                    .child(summary),
                            ),
                    )
                    .when(total_ms > 0, |el| {
                        el.child(
                            div()
                                .text_size(ui_px(CHAT_MICRO_PX))
                                .text_color(rgba(palette.fg_faint))
                                .child(format_duration(total_ms)),
                        )
                    }),
            );

        if expanded {
            let mut inner = div()
                .p_1p5()
                .border_t_1()
                .border_color(rgba(palette.subtle_line))
                .flex()
                .flex_col()
                .gap_1();
            for &idx in indices {
                if let Some(entry) = self.timeline.entries.get(idx) {
                    inner = inner.child(self.render_entry(entry, palette, cx));
                }
            }
            card = card.child(inner);
        }

        card.into_any_element()
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ChatPalette::for_mode(self.theme_mode);
        let header = self.render_header(palette, cx);
        let approval_el = self.render_approval_cards(palette, window, cx);
        let show_jump = !self.is_following_tail && !self.display_items.is_empty();

        let timeline_body = if self.display_items.is_empty() {
            self.render_empty_state(palette, cx)
        } else {
            let entity = cx.entity().clone();
            let list_el = list(self.list_state.clone(), move |ix, _window, cx| {
                entity.update(cx, |this, cx| this.render_timeline_item(ix, palette, cx))
            })
            .w_full()
            .h_full();
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .w_full()
                .py_2()
                .child(list_el)
                .when(show_jump, |el| {
                    el.child(
                        div().absolute().right_4().bottom_2().child(
                            square_button(
                                "chat-jump-bottom",
                                format!("↓ {}", i18n::text(self.language, "chat.scroll_bottom")),
                                ButtonVariant::Secondary,
                                palette,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.is_following_tail = true;
                                this.list_state.scroll_to_end();
                                cx.notify();
                            })),
                        ),
                    )
                })
                .into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgba(palette.canvas))
            .text_color(rgba(palette.fg_prose))
            .child(header)
            .when_some(self.connection_note.clone(), |el, note| {
                el.child(
                    div()
                        .w_full()
                        .px_3()
                        .py_1()
                        .bg(rgba(palette.security_surface))
                        .border_b_1()
                        .border_color(rgba(palette.security_border))
                        .text_size(ui_px(CHAT_MICRO_PX))
                        .text_color(rgba(palette.fg_strong))
                        .child(note),
                )
            })
            .child(timeline_body)
            .when_some(approval_el, |el, cards| el.child(cards))
            .child(self.composer.clone())
    }
}

fn build_display_items(entries: &[TimelineEntry]) -> Vec<TimelineDisplayItem> {
    let mut items = Vec::with_capacity(entries.len());
    let mut i = 0;
    while i < entries.len() {
        if is_clusterable_tool(&entries[i]) {
            let start = i;
            while i < entries.len() && is_clusterable_tool(&entries[i]) {
                i += 1;
            }
            if i - start >= 2 {
                items.push(TimelineDisplayItem::ToolCluster {
                    cluster_id: entries[start].id().to_string(),
                    indices: (start..i).collect(),
                });
            } else {
                items.push(TimelineDisplayItem::Entry { index: start });
            }
        } else {
            items.push(TimelineDisplayItem::Entry { index: i });
            i += 1;
        }
    }
    items
}

fn is_clusterable_tool(entry: &TimelineEntry) -> bool {
    matches!(
        entry,
        TimelineEntry::ToolCall {
            kind: ToolCallKind::Read | ToolCallKind::Search | ToolCallKind::Fetch,
            diff: None,
            ..
        }
    )
}

fn approval_option_variant(opt: &ApprovalOption) -> ButtonVariant {
    match opt.kind.as_deref() {
        Some("allow_always") | Some("allow_once") => ButtonVariant::Primary,
        Some("reject_once") | Some("reject_always") => ButtonVariant::Danger,
        _ => ButtonVariant::Secondary,
    }
}

fn tool_state_color(state: ToolCallState, palette: ChatPalette) -> u32 {
    match state {
        ToolCallState::Completed => palette.theme.green,
        ToolCallState::Running => palette.theme.yellow,
        ToolCallState::Failed => palette.theme.red,
        ToolCallState::Pending => palette.fg_faint,
    }
}

fn one_line_preview(text: &str, max_chars: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= max_chars {
        joined
    } else {
        let head: String = joined.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

fn truncate_tail(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut start = text.len() - max_bytes;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    format!("…\n{}", &text[start..])
}

fn compact_input_hint(input: &serde_json::Value) -> Option<String> {
    for key in ["path", "file_path", "pattern", "query", "url", "command"] {
        if let Some(val) = input.get(key).and_then(|v| v.as_str()) {
            return Some(one_line_preview(val, 72));
        }
    }
    None
}

fn format_duration(ms: u64) -> String {
    if ms >= 1000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{ms}ms")
    }
}

fn uuid_v4_short() -> String {
    let mut hasher = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut hasher);
    std::thread::current().id().hash(&mut hasher);
    format!("{:08x}", hasher.finish() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxlane_core::chat::{ToolCallKind, ToolCallState};

    fn dummy_tool(id: &str, kind: ToolCallKind) -> TimelineEntry {
        TimelineEntry::ToolCall {
            id: id.into(),
            turn_id: None,
            call_id: id.into(),
            name: format!("{kind:?}"),
            kind,
            title: None,
            input: serde_json::json!({}),
            output: None,
            diff: None,
            exit_code: None,
            locations: Vec::new(),
            state: ToolCallState::Completed,
            is_error: false,
            duration_ms: Some(12),
            created_at_ms: 0,
        }
    }

    #[test]
    fn clusters_two_or_more_consecutive_read_search_tools() {
        let entries = vec![
            dummy_tool("t1", ToolCallKind::Read),
            dummy_tool("t2", ToolCallKind::Search),
            dummy_tool("t3", ToolCallKind::Edit),
            dummy_tool("t4", ToolCallKind::Read),
        ];
        let items = build_display_items(&entries);
        assert_eq!(
            items,
            vec![
                TimelineDisplayItem::ToolCluster {
                    cluster_id: "t1".into(),
                    indices: vec![0, 1],
                },
                TimelineDisplayItem::Entry { index: 2 },
                TimelineDisplayItem::Entry { index: 3 },
            ]
        );
    }
}
