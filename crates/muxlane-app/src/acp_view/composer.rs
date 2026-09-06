use super::*;
use gpui::MouseButton;

fn selector_popup_layer(menu: Entity<SelectorMenu>) -> impl IntoElement {
    deferred(
        anchored()
            .anchor(Anchor::TopLeft)
            .offset(gpui::point(ui_px(0.), ui_px(24.)))
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
        .px_1()
        .py_1()
        .text_size(ui_px(11.))
        .text_color(rgba(theme.fg1))
        .hover(|style| style.text_color(rgba(theme.fg0)))
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
        .child(format!("{label} ⌄"))
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
                self.selected_contexts.push(ContextItem {
                    relative_path: relative.to_string(),
                    path: std::path::PathBuf::new(),
                    kind: ContextKind::Url,
                    content: None,
                });
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
            SelectorTarget::Sessions => {
                if self.available_sessions.is_empty() {
                    items.push(SelectorMenuItem::informational(i18n::text(
                        self.language,
                        "acp.sessions_loading",
                    )));
                }
                for session in self.available_sessions.clone() {
                    let session_id = session.id.clone();
                    let title = session.title.clone();
                    let label = match session.updated_at {
                        Some(updated_at) if !updated_at.is_empty() => {
                            format!("{}  {}", title, updated_at)
                        }
                        _ => title.clone(),
                    };
                    let view = view.clone();
                    items.push(SelectorMenuItem::action(
                        label,
                        false,
                        move |_window, app| {
                            view.update(app, |this, cx| {
                                this.import_session(session_id.clone(), title.clone(), cx);
                            })
                            .ok();
                        },
                    ));
                }
            }
            SelectorTarget::Mode => {
                if let Some(modes) = snapshot.modes.as_ref() {
                    for mode in &modes.available {
                        let id = mode.id.clone();
                        let view = view.clone();
                        items.push(SelectorMenuItem::action(
                            mode.name.clone(),
                            mode.id == modes.current,
                            move |_window, app| {
                                view.update(app, |this, cx| {
                                    this.select_mode(id.clone(), cx);
                                })
                                .ok();
                            },
                        ));
                    }
                }
            }
            SelectorTarget::More => {
                let can_history = self.connection_phase == ConnectionPhase::Connected
                    && self
                        .capabilities
                        .as_ref()
                        .is_some_and(|capabilities| capabilities.sessions.list);
                let can_logout = self.connection_phase == ConnectionPhase::Connected
                    && self
                        .capabilities
                        .as_ref()
                        .is_some_and(|capabilities| capabilities.logout);
                let can_checkpoint_restore = self.can_restore_checkpoint();
                let can_checkpoint_undo = self.checkpoint_undo.is_some()
                    && self.turn_state == TurnState::Idle
                    && matches!(self.prompt_queue.phase, DispatchPhase::Idle)
                    && !self.checkpoint_busy;
                if can_checkpoint_restore {
                    let view = view.clone();
                    items.push(SelectorMenuItem::action(
                        i18n::text(self.language, "acp.checkpoint_restore"),
                        false,
                        move |_window, app| {
                            view.update(app, |this, cx| {
                                this.request_checkpoint_restore(cx);
                            })
                            .ok();
                        },
                    ));
                }
                if can_checkpoint_undo {
                    let view = view.clone();
                    items.push(SelectorMenuItem::action(
                        i18n::text(self.language, "acp.checkpoint_undo"),
                        false,
                        move |_window, app| {
                            view.update(app, |this, cx| {
                                this.undo_checkpoint_restore(cx);
                            })
                            .ok();
                        },
                    ));
                }
                if can_history {
                    let action_view = view.clone();
                    let reopen_view = view.clone();
                    let requested = Rc::new(std::cell::Cell::new(false));
                    let action_requested = requested.clone();
                    items.push(SelectorMenuItem::action(
                        i18n::text(self.language, "acp.session_history"),
                        false,
                        move |window, app| {
                            action_view
                                .update(app, |this, cx| {
                                    this.request_session_list(cx);
                                    action_requested
                                        .set(this.open_selector == Some(SelectorTarget::Sessions));
                                })
                                .ok();
                            if requested.get() {
                                let reopen_view = reopen_view.clone();
                                window.on_next_frame(move |_window, app| {
                                    reopen_view
                                        .update(app, |this, cx| {
                                            if this.open_selector.is_none() {
                                                this.open_selector = Some(SelectorTarget::Sessions);
                                                this.clear_selector_menu();
                                                cx.notify();
                                            }
                                        })
                                        .ok();
                                });
                            }
                        },
                    ));
                }
                if can_logout {
                    let view = view.clone();
                    items.push(SelectorMenuItem::action(
                        i18n::text(self.language, "acp.logout"),
                        false,
                        move |_window, app| {
                            view.update(app, |this, cx| this.logout(cx)).ok();
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
                                    move |_window, app| {
                                        view.update(app, |this, cx| {
                                            this.select_config(
                                                id.clone(),
                                                ConfigValue::Select(value.clone()),
                                                cx,
                                            );
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
                                    move |_window, app| {
                                        view.update(app, |this, cx| {
                                            this.select_config(
                                                id.clone(),
                                                ConfigValue::Boolean(value),
                                                cx,
                                            );
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
        let subscription = cx.subscribe(&menu, |this, menu, _: &gpui::DismissEvent, cx| {
            if this
                .selector_menu
                .as_ref()
                .is_some_and(|current| current.entity_id() == menu.entity_id())
            {
                this.close_selector();
                cx.notify();
            }
        });
        self.selector_menu = Some(menu.clone());
        self._selector_menu_subscription = Some(subscription);
        window.on_next_frame(move |window, cx| {
            menu.read(cx).focus_handle(cx).focus(window, cx);
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

    pub(crate) fn request_session_list(&mut self, cx: &mut Context<Self>) {
        if self.open_selector == Some(SelectorTarget::Sessions) {
            self.close_selector();
            cx.notify();
            return;
        }
        self.clear_selector_menu();
        if let Some(handle) = &self.handle {
            if let Err(error) = handle.list_sessions() {
                self.push_entry(Entry::Error(error.to_string()));
            } else {
                self.open_selector = Some(SelectorTarget::Sessions);
            }
        }
        cx.notify();
    }

    fn import_session(&mut self, session_id: String, title: String, cx: &mut Context<Self>) {
        self.close_selector();
        cx.emit(AcpViewEvent::ImportSession { session_id, title });
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
            .max_h(ui_px(220.))
            .overflow_y_scroll()
            .border_1()
            .border_color(rgba(theme.line))
            .bg(rgba(theme.bg1));
        for (index, item) in self.completions.clone().into_iter().enumerate() {
            let label = item.label.clone();
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
                .when(index == self.completion_index, |row| {
                    row.bg(rgba(theme.bg2))
                })
                .hover(|style| style.bg(rgba(theme.bg2)))
                .on_click(
                    cx.listener(move |this, _event, _window, cx| this.choose_completion(index, cx)),
                )
                .child(div().text_color(rgba(theme.fg0)).child(label))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(ui_px(10.))
                        .text_color(rgba(theme.fg2))
                        .child(item.description),
                )
                .child(
                    div()
                        .text_size(ui_px(9.))
                        .text_color(rgba(theme.fg2))
                        .child(item.source),
                ),
            );
        }
        let mut context_chips = div().flex().flex_wrap().gap_1().px_3().py_1();
        for context in self.selected_contexts.clone() {
            let context_kind = context.kind;
            let context_id = context.relative_path.clone();
            let label = format!("@{} ×", context.relative_path);
            context_chips = context_chips.child(
                semantic_button(
                    gpui::ElementId::Name(format!("acp-context-{}", context.relative_path).into()),
                    format!("Remove context {}", context.relative_path),
                    theme,
                )
                .px_2()
                .py_1()
                .border_1()
                .border_color(rgba(theme.line))
                .text_size(ui_px(10.))
                .text_color(rgba(theme.fg1))
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    this.remove_context(context_kind, &context_id, cx)
                }))
                .child(label),
            );
        }
        let mut attachment_chips = div().flex().flex_wrap().gap_1().px_3().py_1();
        for attachment in self.selected_attachments.clone() {
            let path = attachment.path.clone();
            let label = format!("{} · {} KB ×", attachment.name, attachment.size / 1024);
            attachment_chips = attachment_chips.child(
                semantic_button(
                    gpui::ElementId::Name(format!("acp-attachment-{}", attachment.name).into()),
                    format!("Remove attachment {}", attachment.name),
                    theme,
                )
                .px_2()
                .py_1()
                .border_1()
                .border_color(rgba(theme.line))
                .text_size(ui_px(10.))
                .text_color(rgba(theme.fg1))
                .on_click(
                    cx.listener(move |this, _event, _window, cx| this.remove_attachment(&path, cx)),
                )
                .child(label),
            );
        }
        let modes = self.thread.snapshot().modes.clone();
        let config_options = self.thread.snapshot().config_options.clone();
        let control_groups = footer_control_groups(&config_options);
        let can_history = self.connection_phase == ConnectionPhase::Connected
            && self
                .capabilities
                .as_ref()
                .is_some_and(|capabilities| capabilities.sessions.list);
        let can_logout = self.connection_phase == ConnectionPhase::Connected
            && self
                .capabilities
                .as_ref()
                .is_some_and(|capabilities| capabilities.logout);
        let can_checkpoint_restore = self.can_restore_checkpoint();
        let can_checkpoint_undo = self.checkpoint_undo.is_some()
            && self.turn_state == TurnState::Idle
            && matches!(self.prompt_queue.phase, DispatchPhase::Idle)
            && !self.checkpoint_busy;
        self.ensure_selector_menu(window, cx);
        let mut selector_menu = self.selector_menu.clone();
        let mut selector_controls = div().flex().items_center().flex_wrap().gap_2().ml_auto();
        let has_modes = modes
            .as_ref()
            .is_some_and(|modes| !modes.available.is_empty());
        if let Some(modes) = modes.as_ref().filter(|modes| !modes.available.is_empty()) {
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
        if can_checkpoint_restore || can_checkpoint_undo || can_history || can_logout {
            let popup = matches!(
                self.open_selector.as_ref(),
                Some(SelectorTarget::More | SelectorTarget::Sessions)
            )
            .then(|| selector_menu.take())
            .flatten();
            selector_controls = selector_controls.child(
                semantic_button(
                    "acp-more-options",
                    i18n::text(self.language, "acp.more"),
                    theme,
                )
                .px_2()
                .py_1()
                .text_size(ui_px(13.))
                .text_color(rgba(theme.fg2))
                .hover(|style| style.bg(rgba(theme.bg2)).text_color(rgba(theme.fg0)))
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
                    .flex()
                    .items_start()
                    .w_full()
                    .px_3()
                    .pt_2()
                    .child(div().flex_1().min_w_0().child(draft))
                    .child(
                        semantic_button(
                            "acp-expand-editor",
                            i18n::text(self.language, "palette.maximize"),
                            theme,
                        )
                        .w(ui_px(28.))
                        .h(ui_px(28.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgba(theme.fg2))
                        .hover(|style| style.bg(rgba(theme.bg2)).text_color(rgba(theme.fg0)))
                        .on_click(cx.listener(|_this, _event, _window, cx| {
                            cx.emit(AcpViewEvent::ToggleMaximize)
                        }))
                        .child(panel_icon(MAXIMIZE_ICON, theme.fg2)),
                    ),
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
                            semantic_button(
                                "acp-add-attachment",
                                i18n::text(self.language, "acp.add_attachment"),
                                theme,
                            )
                            .w(ui_px(30.))
                            .h(ui_px(30.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgba(theme.fg1))
                            .hover(|style| style.bg(rgba(theme.bg2)).text_color(rgba(theme.fg0)))
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.open_attachment_picker(cx)
                            }))
                            .child(panel_icon(PLUS_ICON, theme.fg1)),
                        )
                    })
                    .child(selector_controls)
                    .child(if generating {
                        semantic_button(
                            "acp-cancel",
                            i18n::text(self.language, "acp.cancel"),
                            theme,
                        )
                        .w(ui_px(30.))
                        .h(ui_px(30.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgba(theme.red))
                        .hover(|style| style.bg(rgba(theme.bg2)))
                        .on_click(cx.listener(|this, _event, _window, cx| this.cancel(cx)))
                        .child("■")
                    } else {
                        semantic_button("acp-send", i18n::text(self.language, "common.send"), theme)
                            .w(ui_px(30.))
                            .h(ui_px(30.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgba(theme.bg2))
                            .text_color(rgba(theme.fg1))
                            .hover(|style| style.bg(rgba(theme.line)).text_color(rgba(theme.fg0)))
                            .on_click(cx.listener(|this, _event, _window, cx| this.send_prompt(cx)))
                            .child(panel_icon(SEND_ICON, theme.fg1))
                    }),
            )
    }
}
