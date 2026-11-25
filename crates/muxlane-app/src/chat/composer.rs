//! Native Chat composer with `@file` mentions, `/commands`, image paste chips, model/thinking pickers,
//! and strict 0px rectangular chrome.
use crate::i18n::{self, Language};
use crate::text_field::{TextField, TextFieldEvent};
use crate::theme::ThemeMode;
use crate::ui::{
    shortcut_chip, square_badge, square_button, ButtonVariant, ChatPalette, CHAT_MEASURE_MAX_PX,
    CHAT_META_PX, CHAT_MICRO_PX,
};
use crate::ui_scale::px as ui_px;
use base64::Engine as _;
use gpui::{
    div, prelude::*, rgba, Context, Entity, EventEmitter, FocusHandle, Focusable, Render,
    SharedString, Subscription, Window,
};
use muxlane_core::chat::{ChatAttachment, ChatModelInfo, ChatSessionMeta, StreamingBehavior};
use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub(crate) enum ComposerEvent {
    Submit {
        text: String,
        attachments: Vec<ChatAttachment>,
        behavior: Option<StreamingBehavior>,
    },
    Slash(SlashCmd),
    Abort,
    SelectModel {
        provider: Option<String>,
        model_id: String,
    },
    SelectThinking(String),
    SwitchTerminal,
    TakeOverChat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlashCmd {
    Clear,
    Model,
    Thinking,
    Terminal,
    Abort,
}

impl SlashCmd {
    fn all() -> &'static [(SlashCmd, &'static str, &'static str)] {
        &[
            (SlashCmd::Model, "/model", "Switch model"),
            (SlashCmd::Thinking, "/thinking", "Set thinking depth"),
            (SlashCmd::Terminal, "/terminal", "Open interactive terminal"),
            (SlashCmd::Abort, "/abort", "Stop current turn"),
            (SlashCmd::Clear, "/clear", "Clear composer draft"),
        ]
    }
}

#[derive(Clone, Debug)]
struct AutocompleteItem {
    label: String,
    detail: Option<String>,
    insert_text: String,
    slash: Option<SlashCmd>,
}

#[derive(Clone, Debug)]
enum ComposerMenu {
    Autocomplete {
        range: Range<usize>,
        trigger: char,
        items: Vec<AutocompleteItem>,
        selected: usize,
    },
    ModelPicker {
        selected: usize,
    },
    ThinkingPicker {
        selected: usize,
    },
}

pub(crate) struct ChatComposer {
    pub(crate) input: Entity<TextField>,
    focus: FocusHandle,
    project_root: Option<PathBuf>,
    file_index: Vec<String>,
    attachments: Vec<ChatAttachment>,
    meta: ChatSessionMeta,
    theme_mode: ThemeMode,
    language: Language,
    active_menu: Option<ComposerMenu>,
    _sub: Subscription,
}

impl EventEmitter<ComposerEvent> for ChatComposer {}

impl Focusable for ChatComposer {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl ChatComposer {
    pub(crate) fn new(
        project_root: Option<PathBuf>,
        theme_mode: ThemeMode,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = i18n::text(language, "chat.placeholder").to_string();
        let input = cx.new(|cx| {
            let mut field = TextField::new_chat_composer(placeholder, window, cx);
            field.set_theme_mode(theme_mode, cx);
            field
        });
        let sub = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            TextFieldEvent::Changed => {
                this.refresh_autocomplete(cx);
                cx.notify();
            }
            TextFieldEvent::Submit { steer } => {
                this.submit_from_input(*steer, window, cx);
            }
            TextFieldEvent::PasteImage { bytes, format } => {
                if bytes.is_empty() || this.attachments.len() >= 8 {
                    return;
                }
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                let ext = format.mime_type().rsplit('/').next().unwrap_or("png");
                let name = format!("pasted-{}.{ext}", this.attachments.len() + 1);
                this.attachments.push(ChatAttachment {
                    name,
                    path: None,
                    mime_type: format.mime_type().to_string(),
                    data_b64: Some(encoded),
                });
                cx.notify();
            }
        });

        let focus = input.focus_handle(cx);
        let mut composer = Self {
            input,
            focus,
            project_root: project_root.clone(),
            file_index: Vec::new(),
            attachments: Vec::new(),
            meta: ChatSessionMeta::default(),
            theme_mode,
            language,
            active_menu: None,
            _sub: sub,
        };
        if let Some(root) = project_root {
            composer.reload_file_index(root, cx);
        }
        composer
    }

    pub(crate) fn set_theme_mode(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
        if self.theme_mode == mode {
            return;
        }
        self.theme_mode = mode;
        self.input.update(cx, |f, cx| f.set_theme_mode(mode, cx));
        cx.notify();
    }

    pub(crate) fn set_language(&mut self, language: Language, cx: &mut Context<Self>) {
        if self.language == language {
            return;
        }
        self.language = language;
        let placeholder = i18n::text(language, "chat.placeholder").to_string();
        self.input
            .update(cx, |f, cx| f.set_placeholder(placeholder, cx));
        cx.notify();
    }

    pub(crate) fn set_meta(&mut self, meta: ChatSessionMeta, cx: &mut Context<Self>) {
        if self.meta != meta {
            self.meta = meta;
            cx.notify();
        }
    }

    pub(crate) fn set_draft(
        &mut self,
        text: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = text.into();
        self.input.update(cx, |f, cx| f.set_text(text, cx));
        self.input.focus_handle(cx).focus(window, cx);
        self.refresh_autocomplete(cx);
        cx.notify();
    }

    pub(crate) fn reload_file_index(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.project_root = Some(root.clone());
        cx.spawn(async move |this, cx| {
            let files = cx
                .background_executor()
                .spawn(async move { collect_project_files(&root, 4000) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.file_index = files;
                this.refresh_autocomplete(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_autocomplete(&mut self, cx: &gpui::App) {
        let token = self.input.read(cx).cursor_token_before_caret();
        let Some((range, trigger, query)) = token else {
            if matches!(self.active_menu, Some(ComposerMenu::Autocomplete { .. })) {
                self.active_menu = None;
            }
            return;
        };

        let items: Vec<AutocompleteItem> = if trigger == '@' {
            let q = query.to_ascii_lowercase();
            let mut scored: Vec<(i32, &String)> = self
                .file_index
                .iter()
                .filter_map(|path| score_file_match(path, &q).map(|score| (score, path)))
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.len().cmp(&b.1.len())));
            scored
                .into_iter()
                .take(8)
                .map(|(_, path)| AutocompleteItem {
                    label: path.clone(),
                    detail: None,
                    insert_text: format!("@{path} "),
                    slash: None,
                })
                .collect()
        } else {
            let q = query.to_ascii_lowercase();
            SlashCmd::all()
                .iter()
                .filter(|(_, cmd, desc)| {
                    q.is_empty()
                        || cmd[1..].to_ascii_lowercase().contains(&q)
                        || desc.to_ascii_lowercase().contains(&q)
                })
                .map(|(variant, cmd, desc)| AutocompleteItem {
                    label: (*cmd).to_string(),
                    detail: Some((*desc).to_string()),
                    insert_text: format!("{cmd} "),
                    slash: Some(*variant),
                })
                .collect()
        };

        if items.is_empty() {
            self.active_menu = None;
        } else {
            self.active_menu = Some(ComposerMenu::Autocomplete {
                range,
                trigger,
                items,
                selected: 0,
            });
        }
    }

    fn accept_menu_selection(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(menu) = self.active_menu.clone() else {
            return false;
        };
        match menu {
            ComposerMenu::Autocomplete {
                range,
                items,
                selected,
                ..
            } => {
                let idx = index.unwrap_or(selected).min(items.len().saturating_sub(1));
                let Some(item) = items.get(idx).cloned() else {
                    self.active_menu = None;
                    return false;
                };
                self.active_menu = None;
                if let Some(cmd) = item.slash {
                    let current = self.input.read(cx).text();
                    if current.trim().starts_with('/') && !current.contains('\n') {
                        self.input.update(cx, |f, cx| f.reset(cx));
                        self.run_slash_command(cmd, window, cx);
                        return true;
                    }
                }
                self.input.update(cx, |f, cx| {
                    f.replace_byte_range(range, &item.insert_text, window, cx);
                });
                true
            }
            ComposerMenu::ModelPicker { selected } => {
                let idx = index.unwrap_or(selected);
                if let Some(model) = self.meta.available_models.get(idx).cloned() {
                    self.active_menu = None;
                    cx.emit(ComposerEvent::SelectModel {
                        provider: (!model.provider.is_empty()).then_some(model.provider),
                        model_id: model.id,
                    });
                    cx.notify();
                    return true;
                }
                self.active_menu = None;
                false
            }
            ComposerMenu::ThinkingPicker { selected } => {
                let levels = self.thinking_levels();
                let idx = index.unwrap_or(selected);
                if let Some(level) = levels.get(idx).cloned() {
                    self.active_menu = None;
                    cx.emit(ComposerEvent::SelectThinking(level));
                    cx.notify();
                    return true;
                }
                self.active_menu = None;
                false
            }
        }
    }

    fn run_slash_command(&mut self, cmd: SlashCmd, _window: &mut Window, cx: &mut Context<Self>) {
        match cmd {
            SlashCmd::Clear => {
                self.attachments.clear();
                self.input.update(cx, |f, cx| f.reset(cx));
                cx.emit(ComposerEvent::Slash(SlashCmd::Clear));
            }
            SlashCmd::Model => {
                if !self.meta.available_models.is_empty() {
                    let selected = self
                        .meta
                        .current_model
                        .as_ref()
                        .and_then(|cur| {
                            self.meta
                                .available_models
                                .iter()
                                .position(|m| m.id == cur.id)
                        })
                        .unwrap_or(0);
                    self.active_menu = Some(ComposerMenu::ModelPicker { selected });
                }
            }
            SlashCmd::Thinking => {
                let levels = self.thinking_levels();
                if !levels.is_empty() {
                    let selected = self
                        .meta
                        .thinking_level
                        .as_deref()
                        .and_then(|cur| levels.iter().position(|l| l == cur))
                        .unwrap_or(0);
                    self.active_menu = Some(ComposerMenu::ThinkingPicker { selected });
                }
            }
            SlashCmd::Terminal => cx.emit(ComposerEvent::SwitchTerminal),
            SlashCmd::Abort => cx.emit(ComposerEvent::Abort),
        }
        cx.notify();
    }

    fn thinking_levels(&self) -> Vec<String> {
        if !self.meta.available_thinking_levels.is_empty() {
            return self.meta.available_thinking_levels.clone();
        }
        if let Some(cur) = &self.meta.thinking_level {
            return vec!["off".into(), "low".into(), "medium".into(), "high".into()]
                .into_iter()
                .chain(
                    (!matches!(cur.as_str(), "off" | "low" | "medium" | "high"))
                        .then_some(cur.clone()),
                )
                .collect();
        }
        Vec::new()
    }

    fn submit_from_input(&mut self, steer: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.meta.supports_chat_input {
            return;
        }
        let raw = self.input.read(cx).text();
        let trimmed = raw.trim();
        if let Some((cmd, _, _)) = SlashCmd::all().iter().find(|(_, name, _)| *name == trimmed) {
            self.input.update(cx, |f, cx| f.reset(cx));
            self.active_menu = None;
            self.run_slash_command(*cmd, window, cx);
            return;
        }

        if trimmed.is_empty() && self.attachments.is_empty() {
            return;
        }

        let mut attachments = std::mem::take(&mut self.attachments);
        if let Some(root) = &self.project_root {
            let mut seen_paths: HashSet<String> =
                attachments.iter().filter_map(|a| a.path.clone()).collect();
            for mention in extract_at_mentions(&raw) {
                let full = root.join(&mention);
                if full.is_file() {
                    let canonical = full.to_string_lossy().to_string();
                    if seen_paths.insert(canonical.clone()) {
                        attachments.push(ChatAttachment {
                            name: mention,
                            path: Some(canonical),
                            mime_type: "text/plain".to_string(),
                            data_b64: None,
                        });
                    }
                }
            }
        }

        let behavior = if self.meta.is_streaming {
            Some(if steer {
                StreamingBehavior::Steer
            } else {
                StreamingBehavior::FollowUp
            })
        } else {
            None
        };

        self.input.update(cx, |f, cx| f.reset(cx));
        self.active_menu = None;
        cx.emit(ComposerEvent::Submit {
            text: raw,
            attachments,
            behavior,
        });
        cx.notify();
    }

    fn render_menu(
        &mut self,
        palette: ChatPalette,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let menu = self.active_menu.as_ref()?;
        let mut list = div()
            .w_full()
            .flex()
            .flex_col()
            .bg(rgba(palette.card))
            .border_1()
            .border_color(rgba(palette.hairline))
            .mb_1();

        match menu {
            ComposerMenu::Autocomplete {
                trigger,
                items,
                selected,
                ..
            } => {
                list = list.child(
                    div()
                        .px_2p5()
                        .py_1()
                        .border_b_1()
                        .border_color(rgba(palette.subtle_line))
                        .bg(rgba(palette.inset))
                        .text_size(ui_px(CHAT_MICRO_PX))
                        .text_color(rgba(palette.fg_faint))
                        .child(if *trigger == '@' {
                            "FILES IN PROJECT"
                        } else {
                            "SLASH COMMANDS"
                        }),
                );
                for (idx, item) in items.iter().enumerate() {
                    let is_sel = idx == *selected;
                    list = list.child(
                        div()
                            .id(gpui::ElementId::Name(format!("comp-ac-{idx}").into()))
                            .px_2p5()
                            .py_1()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .cursor_pointer()
                            .when(is_sel, |el| el.bg(rgba(palette.row_hover)))
                            .hover(|s| s.bg(rgba(palette.row_hover)))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.accept_menu_selection(Some(idx), window, cx);
                                    cx.stop_propagation();
                                }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .text_size(ui_px(CHAT_META_PX))
                                    .text_color(rgba(palette.fg_strong))
                                    .child(item.label.clone()),
                            )
                            .when_some(item.detail.clone(), |el, detail| {
                                el.child(
                                    div()
                                        .text_size(ui_px(CHAT_MICRO_PX))
                                        .text_color(rgba(palette.fg_faint))
                                        .child(detail),
                                )
                            }),
                    );
                }
            }
            ComposerMenu::ModelPicker { selected } => {
                list = list.child(
                    div()
                        .px_2p5()
                        .py_1()
                        .border_b_1()
                        .border_color(rgba(palette.subtle_line))
                        .bg(rgba(palette.inset))
                        .text_size(ui_px(CHAT_MICRO_PX))
                        .text_color(rgba(palette.fg_faint))
                        .child(i18n::text(self.language, "chat.model").to_string()),
                );
                let current_id = self.meta.current_model.as_ref().map(|m| m.id.as_str());
                for (idx, model) in self.meta.available_models.iter().enumerate().take(12) {
                    let is_sel = idx == *selected;
                    let is_current = current_id == Some(model.id.as_str());
                    let display = model_label(model);
                    list = list.child(
                        div()
                            .id(gpui::ElementId::Name(format!("comp-model-{idx}").into()))
                            .px_2p5()
                            .py_1()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .cursor_pointer()
                            .when(is_sel, |el| el.bg(rgba(palette.row_hover)))
                            .hover(|s| s.bg(rgba(palette.row_hover)))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.accept_menu_selection(Some(idx), window, cx);
                                    cx.stop_propagation();
                                }),
                            )
                            .child(
                                div()
                                    .text_size(ui_px(CHAT_META_PX))
                                    .text_color(rgba(palette.fg_strong))
                                    .child(display),
                            )
                            .when(is_current, |el| {
                                el.child(square_badge(
                                    "ACTIVE",
                                    palette.theme.on_accent,
                                    palette.theme.accent,
                                    palette.theme.accent,
                                ))
                            }),
                    );
                }
            }
            ComposerMenu::ThinkingPicker { selected } => {
                let levels = self.thinking_levels();
                list = list.child(
                    div()
                        .px_2p5()
                        .py_1()
                        .border_b_1()
                        .border_color(rgba(palette.subtle_line))
                        .bg(rgba(palette.inset))
                        .text_size(ui_px(CHAT_MICRO_PX))
                        .text_color(rgba(palette.fg_faint))
                        .child(i18n::text(self.language, "chat.thinking_level").to_string()),
                );
                let current = self.meta.thinking_level.as_deref();
                for (idx, level) in levels.iter().enumerate() {
                    let is_sel = idx == *selected;
                    let is_current = current == Some(level.as_str());
                    list = list.child(
                        div()
                            .id(gpui::ElementId::Name(format!("comp-think-{idx}").into()))
                            .px_2p5()
                            .py_1()
                            .flex()
                            .items_center()
                            .justify_between()
                            .cursor_pointer()
                            .when(is_sel, |el| el.bg(rgba(palette.row_hover)))
                            .hover(|s| s.bg(rgba(palette.row_hover)))
                            .on_mouse_down(
                                gpui::MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.accept_menu_selection(Some(idx), window, cx);
                                    cx.stop_propagation();
                                }),
                            )
                            .child(
                                div()
                                    .text_size(ui_px(CHAT_META_PX))
                                    .text_color(rgba(palette.fg_strong))
                                    .child(level.clone()),
                            )
                            .when(is_current, |el| {
                                el.child(square_badge(
                                    "ACTIVE",
                                    palette.theme.on_accent,
                                    palette.theme.accent,
                                    palette.theme.accent,
                                ))
                            }),
                    );
                }
            }
        }
        Some(list.into_any_element())
    }
}

impl Render for ChatComposer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ChatPalette::for_mode(self.theme_mode);
        let measure = div()
            .w_full()
            .max_w(ui_px(CHAT_MEASURE_MAX_PX))
            .mx_auto()
            .px_4()
            .pb_3();

        if !self.meta.supports_chat_input {
            let banner_key = if self.meta.driver
                == muxlane_core::chat::SessionDriverMode::TerminalPty
                && self.meta.has_pty
            {
                "chat.bridge_paused_banner"
            } else {
                "chat.mirror_banner"
            };
            return measure.child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_3()
                    .py_2p5()
                    .bg(rgba(palette.card))
                    .border_1()
                    .border_color(rgba(palette.hairline))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(ui_px(CHAT_META_PX))
                            .text_color(rgba(palette.fg_muted))
                            .child(i18n::text(self.language, banner_key)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .when(self.meta.has_pty, |el| {
                                el.child(
                                    square_button(
                                        "mirror-open-term",
                                        i18n::text(self.language, "chat.open_terminal"),
                                        ButtonVariant::Outline,
                                        palette,
                                    )
                                    .on_click(cx.listener(
                                        |_this, _, _, cx| {
                                            cx.emit(ComposerEvent::SwitchTerminal);
                                        },
                                    )),
                                )
                            })
                            .when(self.meta.supports_terminal_switch, |el| {
                                el.child(
                                    square_button(
                                        "mirror-take-over",
                                        i18n::text(self.language, "chat.take_over_native"),
                                        ButtonVariant::Primary,
                                        palette,
                                    )
                                    .on_click(cx.listener(
                                        |_this, _, _, cx| {
                                            cx.emit(ComposerEvent::TakeOverChat);
                                        },
                                    )),
                                )
                            }),
                    ),
            );
        }

        let focused = self.focus.is_focused(window);
        let text_nonempty = !self.input.read(cx).text().trim().is_empty();
        let can_send = text_nonempty || !self.attachments.is_empty();
        let menu_el = self.render_menu(palette, cx);
        let model_text: SharedString = self
            .meta
            .current_model
            .as_ref()
            .map(model_label)
            .unwrap_or_else(|| "default".to_string())
            .into();
        let thinking_levels = self.thinking_levels();
        let thinking_text = self.meta.thinking_level.clone();
        let context_summary = match (
            self.meta.context_used_tokens,
            self.meta.context_window_tokens,
        ) {
            (Some(used), Some(win)) if win > 0 => Some(format!(
                "{}/{} ({}%)",
                format_token_count(used),
                format_token_count(win),
                ((used as f64 / win as f64) * 100.0).round() as u64
            )),
            (Some(used), _) => Some(format_token_count(used)),
            _ => None,
        };
        let cost_summary = self
            .meta
            .total_cost_usd
            .filter(|c| *c > 0.0001)
            .map(|c| format!("${c:.3}"));

        let outer = measure
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                let shift = event.keystroke.modifiers.shift;
                let model_len = this.meta.available_models.len().min(12);
                let think_len = this.thinking_levels().len();
                if let Some(menu) = &mut this.active_menu {
                    let count = match menu {
                        ComposerMenu::Autocomplete { items, .. } => items.len(),
                        ComposerMenu::ModelPicker { .. } => model_len,
                        ComposerMenu::ThinkingPicker { .. } => think_len,
                    };
                    match key {
                        "up" if count > 0 => {
                            let sel = match menu {
                                ComposerMenu::Autocomplete { selected, .. }
                                | ComposerMenu::ModelPicker { selected }
                                | ComposerMenu::ThinkingPicker { selected } => selected,
                            };
                            *sel = if *sel == 0 { count - 1 } else { *sel - 1 };
                            cx.stop_propagation();
                            cx.notify();
                            return;
                        }
                        "down" if count > 0 => {
                            let sel = match menu {
                                ComposerMenu::Autocomplete { selected, .. }
                                | ComposerMenu::ModelPicker { selected }
                                | ComposerMenu::ThinkingPicker { selected } => selected,
                            };
                            *sel = (*sel + 1) % count;
                            cx.stop_propagation();
                            cx.notify();
                            return;
                        }
                        "escape" => {
                            this.active_menu = None;
                            cx.stop_propagation();
                            cx.notify();
                            return;
                        }
                        "tab" => {
                            if this.accept_menu_selection(None, window, cx) {
                                cx.stop_propagation();
                            }
                            return;
                        }
                        "enter" if !shift => {
                            if this.accept_menu_selection(None, window, cx) {
                                cx.stop_propagation();
                            }
                            return;
                        }
                        _ => {}
                    }
                }
                if key == "escape" && this.meta.is_streaming {
                    cx.emit(ComposerEvent::Abort);
                    cx.stop_propagation();
                }
            }))
            .when_some(menu_el, |el, menu| el.child(menu));

        let mut box_el = div()
            .w_full()
            .flex()
            .flex_col()
            .bg(rgba(palette.card))
            .border_1()
            .border_color(rgba(if focused {
                palette.theme.accent
            } else {
                palette.hairline
            }));

        if self.meta.is_streaming {
            box_el = box_el.child(
                div()
                    .w_full()
                    .px_2p5()
                    .py_1()
                    .bg(rgba(palette.inset))
                    .border_b_1()
                    .border_color(rgba(palette.subtle_line))
                    .text_size(ui_px(CHAT_MICRO_PX))
                    .text_color(rgba(palette.fg_muted))
                    .child(i18n::text(self.language, "chat.queue_notice")),
            );
        }

        if !self.attachments.is_empty() {
            let mut chips = div().w_full().flex().flex_wrap().gap_1().px_2p5().pt_2();
            for (idx, att) in self.attachments.iter().enumerate() {
                let kind_tag = if att.data_b64.is_some() {
                    "IMG"
                } else {
                    "FILE"
                };
                chips = chips.child(
                    div()
                        .id(gpui::ElementId::Name(format!("att-chip-{idx}").into()))
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_1p5()
                        .py_0p5()
                        .bg(rgba(palette.inset))
                        .border_1()
                        .border_color(rgba(palette.hairline))
                        .text_size(ui_px(CHAT_MICRO_PX))
                        .text_color(rgba(palette.fg_strong))
                        .child(div().text_color(rgba(palette.theme.accent)).child(kind_tag))
                        .child(att.name.clone())
                        .child(
                            div()
                                .id(gpui::ElementId::Name(format!("att-rm-{idx}").into()))
                                .cursor_pointer()
                                .text_color(rgba(palette.fg_faint))
                                .hover(|s| s.text_color(rgba(palette.fg_strong)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if idx < this.attachments.len() {
                                        this.attachments.remove(idx);
                                        cx.notify();
                                    }
                                }))
                                .child("×"),
                        ),
                );
            }
            box_el = box_el.child(chips);
        }

        box_el = box_el
            .child(
                div()
                    .w_full()
                    .min_h(ui_px(48.))
                    .max_h(ui_px(180.))
                    .px_0p5()
                    .py_0p5()
                    .child(self.input.clone()),
            )
            .child(
                div()
                    .w_full()
                    .px_2p5()
                    .py_1p5()
                    .bg(rgba(palette.inset))
                    .border_t_1()
                    .border_color(rgba(palette.subtle_line))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .min_w_0()
                            .child(
                                square_button(
                                    "composer-model-btn",
                                    format!("{model_text} ▾"),
                                    ButtonVariant::Outline,
                                    palette,
                                )
                                .when(self.meta.available_models.is_empty(), |el| el.opacity(0.65))
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        if this.meta.available_models.is_empty() {
                                            return;
                                        }
                                        if matches!(
                                            this.active_menu,
                                            Some(ComposerMenu::ModelPicker { .. })
                                        ) {
                                            this.active_menu = None;
                                        } else {
                                            this.active_menu =
                                                Some(ComposerMenu::ModelPicker { selected: 0 });
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                            .when(
                                !thinking_levels.is_empty() || thinking_text.is_some(),
                                |el| {
                                    let label = format!(
                                        "{}: {} ▾",
                                        i18n::text(self.language, "chat.thinking_level"),
                                        thinking_text.as_deref().unwrap_or("default")
                                    );
                                    el.child(
                                        square_button(
                                            "composer-thinking-btn",
                                            label,
                                            ButtonVariant::Outline,
                                            palette,
                                        )
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                if this.thinking_levels().is_empty() {
                                                    return;
                                                }
                                                if matches!(
                                                    this.active_menu,
                                                    Some(ComposerMenu::ThinkingPicker { .. })
                                                ) {
                                                    this.active_menu = None;
                                                } else {
                                                    this.active_menu =
                                                        Some(ComposerMenu::ThinkingPicker {
                                                            selected: 0,
                                                        });
                                                }
                                                cx.notify();
                                            }),
                                        ),
                                    )
                                },
                            )
                            .when(
                                self.meta.driver
                                    == muxlane_core::chat::SessionDriverMode::TerminalPty,
                                |el| {
                                    el.child(shortcut_chip(
                                        i18n::text(self.language, "chat.bridge_sync_chip"),
                                        palette,
                                    ))
                                },
                            )
                            .when_some(context_summary, |el, ctx| {
                                el.child(shortcut_chip(ctx, palette))
                            })
                            .when_some(cost_summary, |el, cost| {
                                el.child(shortcut_chip(cost, palette))
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .when(self.meta.supports_terminal_switch, |el| {
                                el.child(
                                    square_button(
                                        "composer-switch-term",
                                        i18n::text(self.language, "chat.open_terminal"),
                                        ButtonVariant::Ghost,
                                        palette,
                                    )
                                    .on_click(cx.listener(
                                        |_this, _, _, cx| {
                                            cx.emit(ComposerEvent::SwitchTerminal);
                                        },
                                    )),
                                )
                            })
                            .when(self.meta.is_streaming, |el| {
                                el.child(
                                    square_button(
                                        "composer-abort-btn",
                                        i18n::text(self.language, "chat.abort"),
                                        ButtonVariant::Danger,
                                        palette,
                                    )
                                    .on_click(cx.listener(
                                        |_this, _, _, cx| {
                                            cx.emit(ComposerEvent::Abort);
                                        },
                                    )),
                                )
                            })
                            .child(
                                square_button(
                                    "composer-send-btn",
                                    i18n::text(self.language, "chat.send"),
                                    if can_send {
                                        ButtonVariant::Primary
                                    } else {
                                        ButtonVariant::Secondary
                                    },
                                    palette,
                                )
                                .when(!can_send, |el| el.opacity(0.5))
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.submit_from_input(false, window, cx);
                                    },
                                )),
                            ),
                    ),
            );

        outer.child(box_el)
    }
}

fn model_label(model: &ChatModelInfo) -> String {
    if model.provider.is_empty() || model.name.starts_with(&model.provider) {
        model.name.clone()
    } else {
        format!("{}/{}", model.provider, model.name)
    }
}

fn format_token_count(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{}k", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

pub(crate) fn extract_at_mentions(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw_token in text.split_whitespace() {
        if let Some(rest) = raw_token.strip_prefix('@') {
            let clean = rest.trim_end_matches([',', '.', ';', ':', ')', ']']);
            if !clean.is_empty() && !out.iter().any(|existing| existing == clean) {
                out.push(clean.to_string());
            }
        }
    }
    out
}

fn score_file_match(path: &str, query_lower: &str) -> Option<i32> {
    if query_lower.is_empty() {
        return Some(0);
    }
    let lower = path.to_ascii_lowercase();
    let filename = lower.rsplit('/').next().unwrap_or(&lower);
    if filename == query_lower {
        Some(1000)
    } else if filename.starts_with(query_lower) {
        Some(800 - path.len() as i32)
    } else if filename.contains(query_lower) {
        Some(500 - path.len() as i32)
    } else if lower.contains(query_lower) {
        Some(200 - path.len() as i32)
    } else {
        None
    }
}

pub(crate) fn collect_project_files(root: &Path, limit: usize) -> Vec<String> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                if matches!(
                    name.as_str(),
                    "target" | "node_modules" | "dist" | "build" | "__pycache__" | ".git"
                ) {
                    continue;
                }
                stack.push(path);
            } else if path.is_file() {
                if let Ok(rel) = path.strip_prefix(root) {
                    files.push(rel.to_string_lossy().replace('\\', "/"));
                    if files.len() >= limit {
                        files.sort();
                        return files;
                    }
                }
            }
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_mentions_strips_trailing_punctuation_and_dedups() {
        assert_eq!(
            extract_at_mentions(
                "Inspect @crates/muxlane-app/src/main.rs, and @Cargo.toml and @Cargo.toml."
            ),
            vec!["crates/muxlane-app/src/main.rs", "Cargo.toml"]
        );
    }

    #[test]
    fn file_match_scoring_prioritizes_filename_prefixes() {
        assert!(
            score_file_match("crates/app/src/view.rs", "view")
                > score_file_match("crates/view_helper/src/other.rs", "view")
        );
        assert_eq!(score_file_match("src/main.rs", "nonexistent"), None);
    }

    #[test]
    fn token_formatter_scales_cleanly() {
        assert_eq!(format_token_count(850), "850");
        assert_eq!(format_token_count(42_500), "42k");
        assert_eq!(format_token_count(1_500_000), "1.5M");
    }
}
