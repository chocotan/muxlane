use super::*;
use gpui::MouseButton;

#[derive(Debug, PartialEq)]
struct UsageSummary {
    label: String,
    tooltip: String,
    fraction: Option<f32>,
    warning: bool,
}

fn compact_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.).replace(".0M", "M")
    } else if tokens >= 1_000 {
        format!("{:.0}k", tokens as f64 / 1_000.)
    } else {
        tokens.to_string()
    }
}

fn usage_summary(usage: Option<&muxlane_acp::Usage>) -> UsageSummary {
    let Some(usage) = usage else {
        return UsageSummary {
            label: "Context —".into(),
            tooltip: "Context usage has not been reported by this agent.".into(),
            fraction: None,
            warning: false,
        };
    };
    let ratio = (usage.size > 0).then(|| usage.used as f64 / usage.size as f64);
    let mut tooltip = match ratio {
        Some(ratio) => {
            let percentage = if ratio > 0. && ratio < 0.001 {
                "<0.1%".into()
            } else {
                format!("{:.1}%", ratio * 100.)
            };
            format!("Context: {} / {} tokens ({percentage})", usage.used, usage.size)
        }
        None => format!("Context: {} tokens used; total capacity has not been reported.", usage.used),
    };
    if let Some(cost) = &usage.cost {
        // Display the reported value without rounding small nonzero costs to zero.
        tooltip.push_str(&format!("\nCost: {} {}", cost.currency, cost.amount));
    }
    UsageSummary {
        label: format!("{} / {}", compact_tokens(usage.used),
            if usage.size > 0 { compact_tokens(usage.size) } else { "?".into() }),
        tooltip,
        fraction: ratio.map(|ratio| ratio.clamp(0., 1.) as f32),
        warning: ratio.is_some_and(|ratio| ratio >= 0.85),
    }
}

fn render_usage(usage: Option<&muxlane_acp::Usage>, theme: Theme) -> gpui::Stateful<gpui::Div> {
    let summary = usage_summary(usage);
    let color = if summary.warning { theme.yellow } else { theme.accent };
    div().id("acp-token-usage")
        .debug_selector(|| "acp-token-usage".into())
        .min_w_0().max_w(ui_px(140.)).flex_shrink(1.)
        .font_family("Noto Sans")
        .flex().flex_col().gap_1()
        .tooltip(crate::widgets::hover_tip(summary.tooltip, theme))
        .child(meta(theme, summary.label).min_w_0().overflow_hidden().text_ellipsis())
        .child(div().h(ui_px(2.)).w_full().bg(rgba(theme.line))
            .when_some(summary.fraction, |track, fraction| track.child(
                div().h_full().w(gpui::relative(fraction)).bg(rgba(color))
            )))
}

#[cfg(test)]
mod usage_tests {
    use super::*;

    #[test]
    fn absent_usage_is_unknown_not_zero() {
        let summary = usage_summary(None);
        assert_eq!(summary.label, "Context —");
        assert!(summary.tooltip.contains("not been reported"));
        assert_eq!(summary.fraction, None);
        assert!(!summary.warning);
    }

    #[test]
    fn unknown_zero_and_overflow_usage_remain_truthful() {
        let mut usage = muxlane_acp::Usage { used: 42, size: 0, cost: None };
        let summary = usage_summary(Some(&usage));
        assert_eq!(summary.label, "42 / ?");
        assert_eq!(summary.fraction, None);
        assert!(!summary.tooltip.contains('%'));
        usage.size = 100;
        usage.used = 0;
        let summary = usage_summary(Some(&usage));
        assert_eq!(summary.label, "0 / 100");
        assert_eq!(summary.fraction, Some(0.));
        assert!(!summary.warning);
        usage.used = 150;
        let summary = usage_summary(Some(&usage));
        assert_eq!(summary.label, "150 / 100");
        assert!(summary.tooltip.contains("150.0%"));
        assert_eq!(summary.fraction, Some(1.));
        assert!(summary.warning);
        usage.used = 84;
        assert!(!usage_summary(Some(&usage)).warning);
        usage.used = 85;
        assert!(usage_summary(Some(&usage)).warning);
    }

    #[test]
    fn compact_context_keeps_exact_values_and_cost_in_tooltip() {
        let usage = muxlane_acp::Usage { used: 75_786, size: 300_000,
            cost: Some(muxlane_acp::Cost { currency: "USD".into(), amount: 0.000000123 }) };
        let summary = usage_summary(Some(&usage));
        assert_eq!(summary.label, "76k / 300k");
        assert!(summary.tooltip.contains("75786 / 300000"));
        assert!(summary.tooltip.contains("USD 0.000000123"));
        assert!(!summary.label.contains("USD"));
        assert!(!usage_summary(Some(&muxlane_acp::Usage { cost: None, ..usage })).tooltip.contains("Cost:"));
        for value in [1, 999, 1_000, 10_000, 999_999, 1_000_000, u64::MAX] {
            assert!(!compact_tokens(value).starts_with('0'));
        }
    }
}


fn selector_popup_layer(menu: Entity<SelectorMenu>) -> impl IntoElement {
    deferred(
        anchored()
            .anchor(Anchor::TopLeft)
            .offset(gpui::point(ui_px(0.), ui_px(POPUP_OFFSET)))
            .child(div().occlude().child(menu)),
    )
    .priority(2)
}
fn selector_button(
    id: impl Into<gpui::ElementId>,
    label: String,
    target: SelectorTarget,
    menu: Option<Entity<SelectorMenu>>,
    theme: Theme,
    cx: &mut Context<AcpView>,
) -> gpui::Stateful<gpui::Div> {
    let mouse_target = target.clone();
    semantic_button(id, label.clone(), theme)
        .text_size(ui_px(BUTTON_SIZE))
        .min_w_0()
        .max_w(ui_px(180.))
        .px_2()
        .py_1()
        .text_color(rgba(theme.fg1))
        .hover(move |style| style.bg(rgba(theme.bg2)).text_color(rgba(theme.fg0)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _event, _window, cx| {
                this.toggle_selector(mouse_target.clone(), cx);
                cx.stop_propagation();
            }),
        )
        .on_click(
            cx.listener(move |this, event: &gpui::ClickEvent, _window, cx| {
                if event.is_keyboard() {
                    this.toggle_selector(target.clone(), cx);
                }
            }),
        )
        .when_some(menu, |button, menu| {
            button.child(selector_popup_layer(menu))
        })
        .child(
            div()
                .flex()
                .items_center()
                .min_w_0()
                .w_full()
                .gap_1()
                .child(div().min_w_0().flex_1().overflow_hidden().text_ellipsis().child(label))
                .child(div().flex_none().text_color(rgba(theme.fg2)).child("⌄")),
        )
}

fn is_mode_config(option: &muxlane_acp::ConfigOption) -> bool {
    option.id.eq_ignore_ascii_case("mode") || option.name.eq_ignore_ascii_case("mode")
}

fn is_model_config(option: &muxlane_acp::ConfigOption) -> bool {
    let text = format!("{} {}", option.id, option.name).to_lowercase();
    text.contains("model")
}

fn is_thinking_config(option: &muxlane_acp::ConfigOption) -> bool {
    let text = format!("{} {}", option.id, option.name).to_lowercase();
    text.contains("think") || text.contains("reason")
}

fn config_value_label(option: &muxlane_acp::ConfigOption, language: i18n::Language) -> String {
    match &option.kind {
        ConfigKind::Select { current, choices } => choices
            .iter()
            .find(|choice| &choice.id == current)
            .map(|choice| choice.name.clone())
            .unwrap_or_else(|| current.clone()),
        ConfigKind::Boolean { current } => {
            i18n::text(language, if *current { "common.on" } else { "common.off" }).to_string()
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct FooterControlGroups {
    pub(super) secondary_selects: Vec<usize>,
    pub(super) model: Option<usize>,
    pub(super) thinking: Option<usize>,
    pub(super) secondary_booleans: Vec<usize>,
}

pub(super) fn footer_control_groups(
    config_options: &[muxlane_acp::ConfigOption],
) -> FooterControlGroups {
    let mut groups = FooterControlGroups {
        model: config_options.iter().position(is_model_config),
        thinking: config_options.iter().position(is_thinking_config),
        ..Default::default()
    };
    for (index, option) in config_options.iter().enumerate() {
        if Some(index) == groups.model || Some(index) == groups.thinking {
            continue;
        }
        match option.kind {
            ConfigKind::Select { .. } => groups.secondary_selects.push(index),
            ConfigKind::Boolean { .. } => groups.secondary_booleans.push(index),
        }
    }
    groups
}

impl AcpView {
    pub(super) fn refresh_completions(&mut self) {
        let Some((trigger, query)) = active_token(&self.draft_text) else {
            self.completions.clear();
            self.completion_index = 0;
            return;
        };
        self.completions = if trigger == '/' {
            merged_completions(
                &self.thread.snapshot().available_commands,
                &self.skills,
                &query,
            )
        } else {
            filter_context(&self.context_items, &query)
        };
        self.completion_index = self
            .completion_index
            .min(self.completions.len().saturating_sub(1));
        if !self.completions.is_empty() {
            self.close_selector();
        }
    }

    fn choose_completion(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(item) = self.completions.get(index).cloned() else {
            return;
        };
        let text = crate::acp_composer::replace_active_token(&self.draft_text, &item.insert_text);
        if item.kind == CompletionKind::Context {
            let relative = item.label.trim_start_matches('@');
            let context = self
                .context_items
                .iter()
                .find(|context| context.relative_path == relative)
                .cloned();
            if let Some(context) = context {
                if !self
                    .selected_contexts
                    .iter()
                    .any(|selected| selected.relative_path == context.relative_path)
                {
                    let load_diagnostics = context.kind == ContextKind::Diagnostic;
                    let context_name = context.relative_path.clone();
                    self.selected_contexts.push(context);
                    if load_diagnostics {
                        self.load_diagnostics_context(context_name, cx);
                    }
                }
            } else if valid_elicitation_url(relative)
                && !self
                    .selected_contexts
                    .iter()
                    .any(|selected| selected.relative_path == relative)
            {
                self.selected_contexts.push(ContextItem::new(
                    relative,
                    std::path::PathBuf::new(),
                    ContextKind::Url,
                    None,
                ));
            }
        }
        self.draft.update(cx, |field, cx| field.set_text(text, cx));
        self.completions.clear();
        self.completion_index = 0;
    }

    fn load_diagnostics_context(&mut self, name: String, cx: &mut Context<Self>) {
        let root = self.project_path.clone();
        cx.spawn(async move |this, cx| {
            let content = cx
                .background_spawn(async move { load_project_diagnostics(&root) })
                .await;
            this.update(cx, |this, cx| {
                if let Some(context) = this
                    .selected_contexts
                    .iter_mut()
                    .find(|context| context.relative_path == name)
                {
                    context.content = Some(content);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn submit_or_confirm(&mut self, cx: &mut Context<Self>) {
        if self.completions.is_empty() {
            self.send_prompt(cx);
        } else {
            self.choose_completion(self.completion_index, cx);
        }
    }

    pub(super) fn move_completion(&mut self, direction: isize) {
        if self.completions.is_empty() {
            return;
        }
        self.completion_index = (self.completion_index as isize + direction)
            .rem_euclid(self.completions.len() as isize) as usize;
    }

    pub(super) fn clear_completions(&mut self) {
        self.completions.clear();
        self.completion_index = 0;
    }

    pub(super) fn build_prompt_payload(
        &mut self,
        text: String,
        cx: &mut Context<Self>,
    ) -> Option<muxlane_acp::PromptPayload> {
        let mut payload = muxlane_acp::PromptPayload::new(text.clone());
        for token in text
            .split_whitespace()
            .filter(|token| token.starts_with("/:"))
        {
            let name = token
                .trim_start_matches("/:")
                .trim_end_matches(|character: char| character.is_ascii_punctuation());
            if let Some(skill) = self.skills.iter().find(|skill| skill.name == name) {
                match load_skill(skill) {
                    Ok(body) => {
                        payload = payload.with_block(muxlane_acp::PromptBlock::Text(format!(
                            "\n[Skill {}]\n{}",
                            skill.name, body
                        )))
                    }
                    Err(error) => {
                        self.push_entry(Entry::Error(error));
                        cx.notify();
                        return None;
                    }
                }
            }
        }
        let embedded = self
            .capabilities
            .as_ref()
            .is_some_and(|capabilities| capabilities.prompt.embedded_context);
        for context in &self.selected_contexts {
            if let Some(content) = &context.content {
                payload = payload.with_block(muxlane_acp::PromptBlock::Text(format!(
                    "\n[Context {}]\n{}",
                    context.relative_path, content
                )));
                continue;
            }
            if context.kind == ContextKind::Diagnostic {
                self.push_entry(Entry::Error(
                    i18n::text(self.language, "acp.diagnostics_loading").into(),
                ));
                cx.notify();
                return None;
            }
            if context.kind == ContextKind::Url {
                payload = payload.with_block(muxlane_acp::PromptBlock::ResourceLink {
                    name: context.relative_path.clone(),
                    uri: context.relative_path.clone(),
                    mime_type: None,
                });
                continue;
            }
            let uri = format!("file://{}", context.path.display());
            if context.kind == ContextKind::Directory {
                payload = if embedded {
                    payload.with_block(muxlane_acp::PromptBlock::ResourceLink {
                        name: context.relative_path.clone(),
                        uri,
                        mime_type: None,
                    })
                } else {
                    payload.with_block(muxlane_acp::PromptBlock::Text(format!(
                        "\n[Directory context {}]",
                        context.relative_path
                    )))
                };
                continue;
            }
            match std::fs::metadata(&context.path) {
                Ok(metadata) if metadata.len() <= MAX_CONTEXT_BYTES => {}
                Ok(_) => {
                    self.push_entry(Entry::Error(format!(
                        "Context exceeds {} KiB: {}",
                        MAX_CONTEXT_BYTES / 1024,
                        context.relative_path
                    )));
                    cx.notify();
                    return None;
                }
                Err(error) => {
                    self.push_entry(Entry::Error(format!("{}: {error}", context.relative_path)));
                    cx.notify();
                    return None;
                }
            }
            match std::fs::read_to_string(&context.path) {
                Ok(body) if embedded => {
                    payload = payload.with_block(muxlane_acp::PromptBlock::Resource {
                        name: context.relative_path.clone(),
                        uri,
                        mime_type: Some("text/plain".into()),
                        text: body,
                    })
                }
                Ok(body) => {
                    payload = payload.with_block(muxlane_acp::PromptBlock::Text(format!(
                        "\n[Context {}]\n{}",
                        context.relative_path, body
                    )))
                }
                Err(error) => {
                    self.push_entry(Entry::Error(format!("{}: {error}", context.relative_path)));
                    cx.notify();
                    return None;
                }
            }
        }
        for attachment in &self.selected_attachments {
            let supported = self
                .capabilities
                .as_ref()
                .is_some_and(|capabilities| match attachment.kind {
                    AttachmentKind::Image => capabilities.prompt.image,
                    AttachmentKind::Audio => capabilities.prompt.audio,
                });
            if !supported {
                self.push_entry(Entry::Error(
                    i18n::text(self.language, "acp.attachment_unsupported").into(),
                ));
                cx.notify();
                return None;
            }
            match load_attachment(attachment) {
                Ok(block) => payload = payload.with_block(block),
                Err(error) => {
                    self.push_entry(Entry::Error(error));
                    cx.notify();
                    return None;
                }
            }
        }
        Some(payload)
    }

    fn remove_context(&mut self, kind: ContextKind, id: &str, cx: &mut Context<Self>) {
        self.selected_contexts
            .retain(|context| context.kind != kind || context.relative_path != id);
        cx.notify();
    }

    fn remove_attachment(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        self.selected_attachments
            .retain(|attachment| attachment.path != path);
        cx.notify();
    }

    fn open_attachment_picker(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(i18n::text(self.language, "acp.add_attachment").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            this.update(cx, |this, cx| {
                for path in paths {
                    match attachment_from_path(&path) {
                        Ok(attachment)
                            if !this
                                .selected_attachments
                                .iter()
                                .any(|selected| selected.path == attachment.path) =>
                        {
                            this.selected_attachments.push(attachment);
                        }
                        Ok(_) => {}
                        Err(error) => this.push_entry(Entry::Error(error)),
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn clear_selector_menu(&mut self) {
        self.selector_menu = None;
        self._selector_menu_subscription = None;
    }

    pub(super) fn close_selector(&mut self) {
        self.open_selector = None;
        self.clear_selector_menu();
    }

    fn ensure_selector_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selector) = self.open_selector.as_ref() else {
            return;
        };
        if self.selector_menu.is_some() {
            return;
        }

        let snapshot = self.thread.snapshot();
        let mut items = Vec::new();
        let view = cx.entity().downgrade();
        match selector {
            SelectorTarget::Mode => {
                if let Some(modes) = snapshot.modes.as_ref() {
                    for mode in &modes.available {
                        let id = mode.id.clone();
                        let view = view.clone();
                        items.push(SelectorMenuItem::action(
                            mode.name.clone(),
                            mode.id == modes.current,
                            move |window, app| {
                                view.update(app, |this, cx| {
                                    this.select_mode(id.clone(), cx);
                                    this.draft.focus_handle(cx).focus(window, cx);
                                })
                                .ok();
                            },
                        ));
                    }
                }
            }
            SelectorTarget::More => {
                let can_logout = self.connection_phase == ConnectionPhase::Connected
                    && self
                        .capabilities
                        .as_ref()
                        .is_some_and(|capabilities| capabilities.logout);
                if can_logout {
                    let view = view.clone();
                    items.push(SelectorMenuItem::action(
                        i18n::text(self.language, "acp.logout"),
                        false,
                        move |window, app| {
                            view.update(app, |this, cx| {
                                this.logout(cx);
                                this.draft.focus_handle(cx).focus(window, cx);
                            })
                            .ok();
                        },
                    ));
                }
            }
            SelectorTarget::Config(config_id) => {
                if let Some(option) = snapshot
                    .config_options
                    .iter()
                    .find(|option| option.id == *config_id)
                {
                    match &option.kind {
                        ConfigKind::Select { current, choices } => {
                            for choice in choices {
                                let id = option.id.clone();
                                let value = choice.id.clone();
                                let view = view.clone();
                                items.push(SelectorMenuItem::action(
                                    choice.name.clone(),
                                    choice.id == *current,
                                    move |window, app| {
                                        view.update(app, |this, cx| {
                                            this.select_config(
                                                id.clone(),
                                                ConfigValue::Select(value.clone()),
                                                cx,
                                            );
                                            this.draft.focus_handle(cx).focus(window, cx);
                                        })
                                        .ok();
                                    },
                                ));
                            }
                        }
                        ConfigKind::Boolean { current } => {
                            for value in [false, true] {
                                let id = option.id.clone();
                                let label = if value {
                                    i18n::text(self.language, "common.on")
                                } else {
                                    i18n::text(self.language, "common.off")
                                };
                                let view = view.clone();
                                items.push(SelectorMenuItem::action(
                                    label,
                                    value == *current,
                                    move |window, app| {
                                        view.update(app, |this, cx| {
                                            this.select_config(
                                                id.clone(),
                                                ConfigValue::Boolean(value),
                                                cx,
                                            );
                                            this.draft.focus_handle(cx).focus(window, cx);
                                        })
                                        .ok();
                                    },
                                ));
                            }
                        }
                    }
                }
            }
        }

        let menu = cx.new(|cx| SelectorMenu::new(items, cx, Theme::for_mode(self.theme_mode)));
        let subscription = cx.subscribe_in(
            &menu,
            window,
            |this, menu, _: &gpui::DismissEvent, window, cx| {
                if this
                    .selector_menu
                    .as_ref()
                    .is_some_and(|current| current.entity_id() == menu.entity_id())
                {
                    let restore_focus = menu.read(cx).restore_focus_on_dismiss;
                    this.close_selector();
                    if restore_focus {
                        this.draft.focus_handle(cx).focus(window, cx);
                    }
                    cx.notify();
                }
            },
        );
        self.selector_menu = Some(menu.clone());
        self._selector_menu_subscription = Some(subscription);
        window.on_next_frame(move |window, cx| {
            view.update(cx, |this, cx| {
                if this
                    .selector_menu
                    .as_ref()
                    .is_some_and(|current| current.entity_id() == menu.entity_id())
                {
                    menu.read(cx).focus_handle(cx).focus(window, cx);
                }
            })
            .ok();
        });
    }

    fn toggle_selector(&mut self, selector: SelectorTarget, cx: &mut Context<Self>) {
        if self.open_selector.as_ref() == Some(&selector) {
            self.close_selector();
        } else {
            self.close_selector();
            self.open_selector = Some(selector);
        }
        cx.notify();
    }

    fn select_mode(&mut self, id: String, cx: &mut Context<Self>) {
        if let Some(handle) = &self.handle {
            if let Err(error) = handle.set_mode(id) {
                self.push_entry(Entry::Error(error.to_string()));
            }
        }
        self.close_selector();
        cx.notify();
    }

    fn select_config(&mut self, id: String, value: ConfigValue, cx: &mut Context<Self>) {
        if let Some(handle) = &self.handle {
            if let Err(error) = handle.set_config_option(id, value) {
                self.push_entry(Entry::Error(error.to_string()));
            }
        }
        self.close_selector();
        cx.notify();
    }

    pub(super) fn render_composer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let theme = Theme::for_mode(self.theme_mode);
        let draft = self.draft.clone();
        let generating = matches!(self.status, Status::Generating | Status::Permission);
        let supports_attachments = self
            .capabilities
            .as_ref()
            .map(|capabilities| capabilities.prompt.image || capabilities.prompt.audio)
            .unwrap_or(true);
        let mut completion_popup = div()
            .id("acp-completion-popup")
            .flex()
            .flex_col()
            .mx_3()
            .mb_1()
            .max_h(ui_px(POPUP_MAX_HEIGHT))
            .overflow_y_scroll()
            .border_1()
            .border_color(rgba(theme.line))
            .bg(rgba(theme.bg1));
        for (index, item) in self.completions.clone().into_iter().enumerate() {
            let label = item.label.clone();
            let selected = index == self.completion_index;
            completion_popup = completion_popup.child(
                semantic_button(
                    gpui::ElementId::Name(format!("acp-completion-{index}").into()),
                    label.clone(),
                    theme,
                )
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .border_0()
                .border_l_2()
                .border_color(rgba(if selected { theme.accent } else { 0x00000000 }))
                .text_size(ui_px(BODY_SIZE))
                .when(selected, |row| row.bg(rgba(theme.bg2)))
                .hover(|style| style.bg(rgba(theme.bg2)))
                .on_click(
                    cx.listener(move |this, _event, _window, cx| this.choose_completion(index, cx)),
                )
                .child(div().text_color(rgba(theme.fg0)).child(label))
                .child(
                    meta(theme, item.description)
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis(),
                )
                .child(meta(theme, item.source).flex_none()),
            );
        }
        let mut context_chips = div().flex().flex_wrap().gap_1().px_3().py_1();
        for context in self.selected_contexts.clone() {
            let context_kind = context.kind;
            let context_id = context.relative_path.clone();
            let label = format!("@{}", context.relative_path);
            context_chips = context_chips.child(
                chip(
                    gpui::ElementId::Name(format!("acp-context-{}", context.relative_path).into()),
                    format!("Remove context {}", context.relative_path),
                    label,
                    theme,
                )
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    this.remove_context(context_kind, &context_id, cx)
                })),
            );
        }
        let mut attachment_chips = div().flex().flex_wrap().gap_1().px_3().py_1();
        for attachment in self.selected_attachments.clone() {
            let path = attachment.path.clone();
            let label = format!("{} · {} KB", attachment.name, attachment.size / 1024);
            attachment_chips = attachment_chips.child(
                chip(
                    gpui::ElementId::Name(format!("acp-attachment-{}", attachment.name).into()),
                    format!("Remove attachment {}", attachment.name),
                    label,
                    theme,
                )
                .on_click(
                    cx.listener(move |this, _event, _window, cx| this.remove_attachment(&path, cx)),
                ),
            );
        }
        let modes = self.thread.snapshot().modes.clone();
        let config_options = self.thread.snapshot().config_options.clone();
        let control_groups = footer_control_groups(&config_options);
        let can_logout = self.connection_phase == ConnectionPhase::Connected
            && self
                .capabilities
                .as_ref()
                .is_some_and(|capabilities| capabilities.logout);
        self.ensure_selector_menu(window, cx);
        let mut selector_menu = self.selector_menu.clone();
        let usage = render_usage(self.thread.snapshot().usage.as_ref(), theme);
        let mut selector_controls = div().min_w_0().flex_1().flex().justify_end().items_center().flex_wrap().gap_1();
        let has_modes = config_options.is_empty()
            && modes
                .as_ref()
                .is_some_and(|modes| !modes.available.is_empty());
        if let Some(modes) = modes
            .as_ref()
            .filter(|modes| config_options.is_empty() && !modes.available.is_empty())
        {
            let current = modes
                .available
                .iter()
                .find(|mode| mode.id == modes.current)
                .map(|mode| mode.name.as_str())
                .unwrap_or(&modes.current)
                .to_string();
            let popup = self
                .open_selector
                .as_ref()
                .is_some_and(|target| target == &SelectorTarget::Mode)
                .then(|| selector_menu.take())
                .flatten();
            selector_controls = selector_controls.child(selector_button(
                "acp-mode-selector",
                current,
                SelectorTarget::Mode,
                popup,
                theme,
                cx,
            ));
        }
        for &index in &control_groups.secondary_selects {
            let option = &config_options[index];
            if has_modes && is_mode_config(option) {
                continue;
            }
            let id = option.id.clone();
            let label = format!(
                "{}: {}",
                option.name,
                config_value_label(option, self.language)
            );
            let popup = self
                .open_selector
                .as_ref()
                .is_some_and(|target| target.matches_id(&id))
                .then(|| selector_menu.take())
                .flatten();
            selector_controls = selector_controls.child(selector_button(
                gpui::ElementId::Name(format!("acp-config-selector-{id}").into()),
                label,
                SelectorTarget::config(id),
                popup,
                theme,
                cx,
            ));
        }
        if let Some(index) = control_groups.model {
            let option = &config_options[index];
            let id = option.id.clone();
            let label = config_value_label(option, self.language);
            let popup = self
                .open_selector
                .as_ref()
                .is_some_and(|target| target.matches_id(&id))
                .then(|| selector_menu.take())
                .flatten();
            selector_controls = selector_controls.child(selector_button(
                gpui::ElementId::Name(format!("acp-model-selector-{id}").into()),
                label,
                SelectorTarget::config(id),
                popup,
                theme,
                cx,
            ));
        }
        if let Some(index) = control_groups.thinking {
            let option = &config_options[index];
            let id = option.id.clone();
            let label = format!(
                "{}: {}",
                i18n::text(self.language, "acp.thinking"),
                config_value_label(option, self.language)
            );
            let popup = self
                .open_selector
                .as_ref()
                .is_some_and(|target| target.matches_id(&id))
                .then(|| selector_menu.take())
                .flatten();
            selector_controls = selector_controls.child(selector_button(
                gpui::ElementId::Name(format!("acp-thinking-selector-{id}").into()),
                label,
                SelectorTarget::config(id),
                popup,
                theme,
                cx,
            ));
        }
        for &index in &control_groups.secondary_booleans {
            let option = &config_options[index];
            let id = option.id.clone();
            let label = format!(
                "{}: {}",
                option.name,
                config_value_label(option, self.language)
            );
            let popup = self
                .open_selector
                .as_ref()
                .is_some_and(|target| target.matches_id(&id))
                .then(|| selector_menu.take())
                .flatten();
            selector_controls = selector_controls.child(selector_button(
                gpui::ElementId::Name(format!("acp-config-selector-{id}").into()),
                label,
                SelectorTarget::config(id),
                popup,
                theme,
                cx,
            ));
        }
        if can_logout {
            let popup = matches!(self.open_selector.as_ref(), Some(SelectorTarget::More))
                .then(|| selector_menu.take())
                .flatten();
            selector_controls = selector_controls.child(
                icon_button(
                    "acp-more-options",
                    i18n::text(self.language, "acp.more"),
                    theme,
                )
                .text_size(ui_px(BODY_SIZE))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _event, _window, cx| {
                        this.toggle_selector(SelectorTarget::More, cx);
                        cx.stop_propagation();
                    }),
                )
                .on_click(cx.listener(|this, event: &gpui::ClickEvent, _window, cx| {
                    if event.is_keyboard() {
                        this.toggle_selector(SelectorTarget::More, cx);
                    }
                }))
                .when_some(popup, |button, popup| {
                    button.child(selector_popup_layer(popup))
                })
                .child("⋯"),
            );
        }
        div()
            .debug_selector(|| "acp-composer".into())
            .min_w_0()
            .w_full()
            .flex_none()
            .bg(rgba(theme.bg0))
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(rgba(theme.line))
            .on_key_down(
                cx.listener(|this, event: &gpui::KeyDownEvent, _window, cx| {
                    if this.checkpoint_restore_confirm {
                        match event.keystroke.key.as_str() {
                            "escape" => this.checkpoint_restore_confirm = false,
                            "enter" => this.restore_last_checkpoint(cx),
                            _ => return,
                        }
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                    if event.keystroke.key.as_str() == "escape" && this.open_selector.is_some() {
                        this.close_selector();
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                    if this.completions.is_empty() {
                        return;
                    }
                    match event.keystroke.key.as_str() {
                        "up" => this.move_completion(-1),
                        "down" => this.move_completion(1),
                        "escape" => this.clear_completions(),
                        _ => return,
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .when(!self.completions.is_empty(), |composer| {
                composer.child(completion_popup)
            })
            .when(!self.selected_contexts.is_empty(), |composer| {
                composer.child(context_chips)
            })
            .when(!self.selected_attachments.is_empty(), |composer| {
                composer.child(attachment_chips)
            })
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .px_3()
                    .pt_2()
                    .child(draft),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_2()
                    .min_h(ui_px(44.))
                    .px_3()
                    .py_1()
                    .when(supports_attachments, |bar| {
                        bar.child(
                            icon_button(
                                "acp-add-attachment",
                                i18n::text(self.language, "acp.add_attachment"),
                                theme,
                            )
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.open_attachment_picker(cx)
                            }))
                            .child(panel_icon(PLUS_ICON, theme.fg1)),
                        )
                    })
                    .child(usage)
                    .child(selector_controls)
                    .child(
                        icon_button("acp-expand-editor", i18n::text(self.language, "palette.maximize"), theme)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(AcpViewEvent::ToggleMaximize)))
                            .child(panel_icon(MAXIMIZE_ICON, theme.fg2)),
                    )
                    .child(if generating {
                        icon_button("acp-cancel", i18n::text(self.language, "acp.cancel"), theme)
                            .text_size(ui_px(BUTTON_SIZE))
                            .text_color(rgba(theme.red))
                            .on_click(cx.listener(|this, _event, _window, cx| this.cancel(cx)))
                            .child("■")
                    } else {
                        icon_button("acp-send", i18n::text(self.language, "common.send"), theme)
                            .bg(rgba(theme.bg2))
                            .on_click(cx.listener(|this, _event, _window, cx| this.send_prompt(cx)))
                            .child(panel_icon(SEND_ICON, theme.fg1))
                    }),
            )
    }
}

/// 上下文 / 附件 chip：标签 + 独立关闭图标（hover 时 red）。
fn chip(
    id: impl Into<gpui::ElementId>,
    aria: impl Into<gpui::SharedString>,
    label: String,
    theme: Theme,
) -> gpui::Stateful<gpui::Div> {
    semantic_button(id, aria, theme)
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .border_color(rgba(theme.line))
        .text_size(ui_px(META_SIZE))
        .text_color(rgba(theme.fg1))
        .hover(move |style| style.bg(rgba(theme.bg2)))
        .child(label)
        .child(
            panel_icon(CLOSE_ICON, theme.fg2)
                .size(ui_px(12.))
                .hover(move |style| style.text_color(rgba(theme.red))),
        )
}
