use super::*;

#[derive(Debug)]
pub(super) struct ElicitationFormState {
    pub(super) fields: Vec<crate::acp_elicitation::ElicitationField>,
    pub(super) values: std::collections::HashMap<String, serde_json::Value>,
    pub(super) editors: std::collections::HashMap<String, Entity<PromptEditor>>,
}

impl ElicitationFormState {
    fn from_request(request: &ElicitationRequest) -> Option<Self> {
        let ElicitationMode::Form { schema } = &request.mode else {
            return None;
        };
        Some(Self {
            fields: fields_from_schema(schema),
            values: std::collections::HashMap::new(),
            editors: std::collections::HashMap::new(),
        })
    }
}

impl AcpView {
    pub(super) fn add_elicitation(&mut self, request: ElicitationRequest) {
        if let Some(form) = ElicitationFormState::from_request(&request) {
            self.elicitation_forms
                .entry(request.id.clone())
                .or_insert(form);
        }
        if !self
            .pending_elicitations
            .iter()
            .any(|pending| pending.id == request.id)
        {
            self.pending_elicitations.push(request);
        }
    }

    pub(super) fn render_panels(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        self.prepare_elicitation_inputs(window, cx);
        let theme = Theme::for_mode(self.theme_mode);
        let mut panels = div().flex().flex_col();
        if let Some(permission) = self.pending_permissions.first().cloned() {
            let id = permission.id.clone();
            let has_reject = permission.options.iter().any(|option| {
                matches!(
                    option.kind,
                    PermissionKind::RejectOnce | PermissionKind::RejectAlways
                )
            });
            let pending_more = self.pending_permissions.len().saturating_sub(1);
            let mut options = div().flex().flex_row().flex_wrap().items_center().gap_2();
            for option in permission.options {
                let permission_id = id.clone();
                let option_id = option.id.clone();
                let option_label = option.label.clone();
                let option_button = match option.kind {
                    PermissionKind::AllowOnce | PermissionKind::AllowAlways => primary_button(
                        format!("acp-permission-{id}-{}", option.id),
                        option_label.clone(),
                        theme,
                        theme.green,
                    ),
                    PermissionKind::RejectOnce | PermissionKind::RejectAlways => secondary_button(
                        format!("acp-permission-{id}-{}", option.id),
                        option_label.clone(),
                        theme,
                        theme.red,
                    ),
                    PermissionKind::Unknown => secondary_button(
                        format!("acp-permission-{id}-{}", option.id),
                        option_label.clone(),
                        theme,
                        theme.fg1,
                    ),
                };
                options = options.child(
                    option_button
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.choose_permission(
                                permission_id.clone(),
                                Some(option_id.clone()),
                                cx,
                            )
                        }))
                        .child(option_label),
                );
            }
            if !has_reject {
                let reject_id = id.clone();
                options = options.child(
                    secondary_button(
                        gpui::ElementId::Name(format!("acp-permission-{id}-reject").into()),
                        i18n::text(self.language, "acp.reject"),
                        theme,
                        theme.red,
                    )
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.choose_permission(reject_id.clone(), None, cx)
                    }))
                    .child(i18n::text(self.language, "acp.reject")),
                );
            }
            let card = signal_card(theme, theme.yellow).min_w_0()
                .child(card_title(theme, theme.yellow, permission.title).when(
                    pending_more > 0,
                    |title| {
                        title.child(meta(theme, format!("+{pending_more} pending")).flex_none())
                    },
                ))
                .child(meta(theme, format!("Awaiting permission | {}", permission.tool_id)))
                .when(!permission.context.is_empty(), |card| card.child(
                    div().id(format!("permission-context-{id}"))
                        .min_w_0().w_full().max_h(ui_px(180.))
                        .overflow_x_scroll().overflow_y_scroll().flex().font_family("monospace")
                        .text_size(ui_px(CODE_SIZE)).line_height(ui_px(CODE_LINE))
                        .whitespace_nowrap().child(div().w_auto().flex_none().child(permission.context.clone()))
                ))
                .child(options);
            panels = panels.child(card);
        }
        if let Some(request) = self.pending_elicitations.first().cloned() {
            let request_id = request.id.clone();
            let mut panel = signal_card(theme, theme.yellow).child(card_title(
                theme,
                theme.yellow,
                request.message,
            ));
            match request.mode {
                ElicitationMode::Form { .. } => {
                    let fields = self
                        .elicitation_forms
                        .get(&request_id)
                        .map(|form| form.fields.clone())
                        .unwrap_or_default();
                    for field in fields {
                        let label = if field.required {
                            format!("{} *", field.title)
                        } else {
                            field.title.clone()
                        };
                        let mut field_view = div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .py_1()
                            .child(div().text_color(rgba(theme.fg0)).child(label));
                        if let Some(description) = field.description {
                            field_view = field_view.child(meta(theme, description));
                        }
                        let value_key = format!("{request_id}:{}", field.name);
                        match field.kind {
                            ElicitationFieldKind::String { choices } if !choices.is_empty() => {
                                let mut choices_view = div().flex().flex_wrap().gap_1();
                                for choice in choices {
                                    let name = field.name.clone();
                                    let choice_value = choice.clone();
                                    let choice_request_id = request_id.clone();
                                    let selected = self
                                        .elicitation_forms
                                        .get(&request_id)
                                        .and_then(|form| form.values.get(&value_key))
                                        == Some(&serde_json::Value::String(choice.clone()));
                                    choices_view = choices_view.child(
                                        choice_button(
                                            gpui::ElementId::Name(
                                                format!("elicitation-{request_id}-{name}-{choice}")
                                                    .into(),
                                            ),
                                            choice.clone(),
                                            theme,
                                            selected,
                                            false,
                                        )
                                        .on_click(cx.listener(move |this, _event, _window, cx| {
                                            this.select_elicitation_value(
                                                &choice_request_id,
                                                name.clone(),
                                                serde_json::Value::String(choice_value.clone()),
                                                false,
                                                cx,
                                            )
                                        }))
                                        .child(choice.clone()),
                                    );
                                }
                                field_view = field_view.child(choices_view);
                            }
                            ElicitationFieldKind::Boolean => {
                                let current = self
                                    .elicitation_forms
                                    .get(&request_id)
                                    .and_then(|form| form.values.get(&value_key))
                                    .and_then(serde_json::Value::as_bool);
                                let mut choices_view = div().flex().gap_1();
                                for value in [true, false] {
                                    let name = field.name.clone();
                                    let choice_request_id = request_id.clone();
                                    let label = i18n::text(
                                        self.language,
                                        if value { "common.on" } else { "common.off" },
                                    );
                                    choices_view = choices_view.child(
                                        choice_button(
                                            gpui::ElementId::Name(
                                                format!("elicitation-{request_id}-{name}-{value}")
                                                    .into(),
                                            ),
                                            label,
                                            theme,
                                            current == Some(value),
                                            false,
                                        )
                                        .on_click(cx.listener(move |this, _event, _window, cx| {
                                            this.select_elicitation_value(
                                                &choice_request_id,
                                                name.clone(),
                                                serde_json::Value::Bool(value),
                                                false,
                                                cx,
                                            )
                                        }))
                                        .child(label),
                                    );
                                }
                                field_view = field_view.child(choices_view);
                            }
                            ElicitationFieldKind::MultiSelect { choices } => {
                                let mut choices_view = div().flex().flex_wrap().gap_1();
                                for choice in choices {
                                    let name = field.name.clone();
                                    let choice_value = choice.clone();
                                    let choice_request_id = request_id.clone();
                                    let selected = self
                                        .elicitation_forms
                                        .get(&request_id)
                                        .and_then(|form| form.values.get(&value_key))
                                        .and_then(serde_json::Value::as_array)
                                        .is_some_and(|values| {
                                            values.contains(&serde_json::Value::String(
                                                choice.clone(),
                                            ))
                                        });
                                    choices_view = choices_view.child(
                                        choice_button(
                                            gpui::ElementId::Name(
                                                format!("elicitation-{request_id}-{name}-{choice}")
                                                    .into(),
                                            ),
                                            choice.clone(),
                                            theme,
                                            selected,
                                            true,
                                        )
                                        .on_click(cx.listener(move |this, _event, _window, cx| {
                                            this.select_elicitation_value(
                                                &choice_request_id,
                                                name.clone(),
                                                serde_json::Value::String(choice_value.clone()),
                                                true,
                                                cx,
                                            )
                                        }))
                                        .child(choice.clone()),
                                    );
                                }
                                field_view = field_view.child(choices_view);
                            }
                            ElicitationFieldKind::Unsupported(kind) => {
                                field_view = field_view.child(
                                    div()
                                        .pl_2()
                                        .border_l_2()
                                        .border_color(rgba(theme.red))
                                        .text_color(rgba(theme.fg1))
                                        .child(format!("Unsupported field type: {kind}")),
                                );
                            }
                            _ => {
                                let key = format!("{request_id}:{}", field.name);
                                if let Some(editor) = self
                                    .elicitation_forms
                                    .get(&request_id)
                                    .and_then(|form| form.editors.get(&key))
                                {
                                    field_view = field_view.child(editor.clone());
                                }
                            }
                        }
                        panel = panel.child(field_view);
                    }
                    let accept_id = request_id.clone();
                    let decline_id = request_id.clone();
                    panel = panel.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .pt_1()
                            .child(
                                primary_button(
                                    "elicitation-accept",
                                    i18n::text(self.language, "acp.elicitation_accept"),
                                    theme,
                                    theme.green,
                                )
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    this.submit_elicitation(accept_id.clone(), cx)
                                }))
                                .child(i18n::text(self.language, "acp.elicitation_accept")),
                            )
                            .child(
                                secondary_button(
                                    "elicitation-decline",
                                    i18n::text(self.language, "acp.elicitation_decline"),
                                    theme,
                                    theme.red,
                                )
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    this.respond_elicitation(
                                        decline_id.clone(),
                                        ElicitationResponse::Decline,
                                        cx,
                                    )
                                }))
                                .child(i18n::text(self.language, "acp.elicitation_decline")),
                            ),
                    );
                }
                ElicitationMode::Url { url, .. } => {
                    let valid = valid_elicitation_url(&url);
                    let open_url = url.clone();
                    let done_id = request_id.clone();
                    let decline_id = request_id.clone();
                    panel = panel
                        .child(
                            div()
                                .font_family("monospace")
                                .text_size(ui_px(CODE_SIZE))
                                .line_height(ui_px(CODE_LINE))
                                .text_color(rgba(if valid { theme.accent } else { theme.red }))
                                .child(url),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_2()
                                .pt_1()
                                .when(valid, |actions| {
                                    actions.child(
                                        primary_button(
                                            "elicitation-open-url",
                                            i18n::text(self.language, "acp.elicitation_open_url"),
                                            theme,
                                            theme.accent,
                                        )
                                        .on_click(cx.listener(move |this, _event, _window, cx| {
                                            this.open_elicitation_url(&open_url, cx)
                                        }))
                                        .child(i18n::text(
                                            self.language,
                                            "acp.elicitation_open_url",
                                        )),
                                    )
                                })
                                .child(
                                    secondary_button(
                                        "elicitation-done",
                                        i18n::text(self.language, "acp.elicitation_done"),
                                        theme,
                                        theme.fg1,
                                    )
                                    .on_click(cx.listener(move |this, _event, _window, cx| {
                                        this.respond_elicitation(
                                            done_id.clone(),
                                            ElicitationResponse::Accept(serde_json::Map::new()),
                                            cx,
                                        )
                                    }))
                                    .child(i18n::text(self.language, "acp.elicitation_done")),
                                )
                                .child(
                                    secondary_button(
                                        "elicitation-url-decline",
                                        i18n::text(self.language, "acp.elicitation_decline"),
                                        theme,
                                        theme.red,
                                    )
                                    .on_click(cx.listener(move |this, _event, _window, cx| {
                                        this.respond_elicitation(
                                            decline_id.clone(),
                                            ElicitationResponse::Decline,
                                            cx,
                                        )
                                    }))
                                    .child(i18n::text(self.language, "acp.elicitation_decline")),
                                ),
                        );
                }
                ElicitationMode::Unsupported => {
                    let cancel_id = request_id.clone();
                    panel = panel.child(
                        div().flex().pt_1().child(
                            secondary_button(
                                "elicitation-cancel-unsupported",
                                i18n::text(self.language, "common.cancel"),
                                theme,
                                theme.red,
                            )
                            .on_click(cx.listener(move |this, _event, _window, cx| {
                                this.respond_elicitation(
                                    cancel_id.clone(),
                                    ElicitationResponse::Cancel,
                                    cx,
                                )
                            }))
                            .child(i18n::text(self.language, "common.cancel")),
                        ),
                    );
                }
            }
            panels = panels.child(panel);
        }
        if matches!(self.status, Status::Failed | Status::Disconnected) {
            if let Some(capabilities) = self
                .capabilities
                .as_ref()
                .filter(|capabilities| !capabilities.auth_methods.is_empty())
            {
                let mut auth_panel = signal_card(theme, theme.yellow).gap_0().child(
                    card_title(
                        theme,
                        theme.yellow,
                        i18n::text(self.language, "acp.authentication_required"),
                    )
                    .pb_2(),
                );
                for method in &capabilities.auth_methods {
                    let method_id = method.id.clone();
                    auth_panel = auth_panel.child(
                        button(
                            gpui::ElementId::Name(format!("acp-auth-{method_id}").into()),
                            method.name.clone(),
                            theme,
                        )
                        .w_full()
                        .px_3()
                        .py_2()
                        .border_0()
                        .border_t_1()
                        .border_color(rgba(theme.line))
                        .text_color(rgba(theme.fg0))
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.begin_authentication(method_id.clone(), cx)
                        }))
                        .child(method.name.clone()),
                    );
                }
                panels = panels.child(auth_panel);
            }
        }
        if self.profile.is_some()
            && self.handle.is_none()
            && matches!(self.status, Status::Failed | Status::Disconnected)
        {
            let can_start_fresh = self.protocol_session_id.is_some();
            panels =
                panels.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px(ui_px(CARD_MARGIN))
                        .py_2()
                        .border_t_1()
                        .border_color(rgba(theme.line))
                        .child(
                            primary_button(
                                "acp-retry",
                                i18n::text(self.language, "acp.retry"),
                                theme,
                                theme.accent,
                            )
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.request_restart(false, cx)
                            }))
                            .child(i18n::text(self.language, "acp.retry")),
                        )
                        .when(can_start_fresh, |controls| {
                            controls.child(
                                secondary_button(
                                    "acp-start-fresh",
                                    i18n::text(self.language, "acp.start_fresh"),
                                    theme,
                                    theme.fg1,
                                )
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.request_restart(true, cx)
                                }))
                                .child(i18n::text(self.language, "acp.start_fresh")),
                            )
                        }),
                );
        }
        panels
    }

    pub(super) fn render_queue_panels(&self, cx: &mut Context<Self>) -> gpui::Div {
        let theme = Theme::for_mode(self.theme_mode);
        div()
            .when(!self.prompt_queue.submissions.is_empty(), |thread| {
                let count = self.prompt_queue.submissions.len();
                let preview = self
                    .prompt_queue
                    .submissions
                    .front()
                    .map(|submission| crate::widgets::truncate(&submission.payload.text, 72))
                    .unwrap_or_default();
                let send_now_label = i18n::text(
                    self.language,
                    if self.turn_state == TurnState::Generating {
                        "acp.queue_cancel_and_send"
                    } else {
                        "acp.queue_send_now"
                    },
                );
                thread.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px(ui_px(CARD_MARGIN))
                        .py_2()
                        .border_t_1()
                        .border_color(rgba(theme.line))
                        .bg(rgba(theme.bg1))
                        .child(
                            meta(
                                theme,
                                format!(
                                    "{} · {preview}",
                                    i18n::text(
                                        self.language,
                                        if self.prompt_queue.paused {
                                            "acp.queue_paused"
                                        } else {
                                            "acp.queue_count"
                                        }
                                    )
                                    .replace("{count}", &count.to_string())
                                ),
                            )
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(rgba(theme.fg1)),
                        )
                        .child(
                            primary_button(
                                "acp-queue-send-now",
                                send_now_label,
                                theme,
                                theme.accent,
                            )
                            .on_click(
                                cx.listener(|this, _event, _window, cx| this.send_next_now(cx)),
                            )
                            .child(send_now_label),
                        )
                        .child(
                            icon_button(
                                "acp-queue-remove",
                                i18n::text(self.language, "acp.queue_remove"),
                                theme,
                            )
                            .on_click(
                                cx.listener(|this, _event, _window, cx| {
                                    this.remove_first_queued(cx)
                                }),
                            )
                            .child(panel_icon(CLOSE_ICON, theme.fg2)),
                        )
                        .when(count > 1, |queue| {
                            queue.child(
                                secondary_button(
                                    "acp-queue-clear",
                                    i18n::text(self.language, "acp.queue_clear"),
                                    theme,
                                    theme.red,
                                )
                                .on_click(
                                    cx.listener(|this, _event, _window, cx| this.clear_queue(cx)),
                                )
                                .child(i18n::text(self.language, "common.clear")),
                            )
                        }),
                )
            })
            .when(
                self.checkpoint_restore_confirm && !self.checkpoint_busy,
                |thread| {
                    thread.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .pl(ui_px(BAR_PAD))
                            .pr(ui_px(CARD_MARGIN))
                            .py_2()
                            .border_l_2()
                            .border_color(rgba(theme.yellow))
                            .bg(rgba(Theme::with_alpha(theme.yellow, TINT_ALPHA)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(ui_px(BODY_SIZE))
                                    .line_height(ui_px(BODY_LINE))
                                    .text_color(rgba(theme.fg0))
                                    .child(i18n::text(
                                        self.language,
                                        "acp.checkpoint_confirm_copy",
                                    )),
                            )
                            .child(
                                secondary_button(
                                    "acp-restore-checkpoint-cancel",
                                    i18n::text(self.language, "common.cancel"),
                                    theme,
                                    theme.fg1,
                                )
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.checkpoint_restore_confirm = false;
                                    cx.notify();
                                }))
                                .child(i18n::text(self.language, "common.cancel")),
                            )
                            .child(
                                primary_button(
                                    "acp-restore-checkpoint-confirm",
                                    i18n::text(self.language, "acp.checkpoint_confirm"),
                                    theme,
                                    theme.yellow,
                                )
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.restore_last_checkpoint(cx)
                                }))
                                .child(i18n::text(self.language, "acp.checkpoint_confirm")),
                            ),
                    )
                },
            )
    }

    pub(super) fn prepare_elicitation_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let requests = self.pending_elicitations.clone();
        for request in requests {
            let ElicitationMode::Form { .. } = request.mode else {
                continue;
            };
            let Some(form) = self.elicitation_forms.get_mut(&request.id) else {
                continue;
            };
            for field in form.fields.clone() {
                let key = format!("{}:{}", request.id, field.name);
                match &field.kind {
                    ElicitationFieldKind::String { choices } if !choices.is_empty() => {
                        if let Some(default) = field.default {
                            form.values.entry(key).or_insert(default);
                        }
                    }
                    ElicitationFieldKind::Boolean | ElicitationFieldKind::MultiSelect { .. } => {
                        if let Some(default) = field.default {
                            form.values.entry(key).or_insert(default);
                        }
                    }
                    ElicitationFieldKind::Unsupported(_) => {}
                    _ => {
                        if form.editors.contains_key(&key) {
                            continue;
                        }
                        let title = field.title.clone();
                        let default = field.default.clone();
                        let editor = cx.new(|cx| {
                            let mut editor = PromptEditor::new(title, window, cx);
                            editor.set_theme_mode(self.theme_mode, cx);
                            if let Some(default) = default {
                                let value = default
                                    .as_str()
                                    .map(str::to_string)
                                    .unwrap_or_else(|| default.to_string());
                                editor.set_text(value, cx);
                            }
                            editor
                        });
                        form.editors.insert(key, editor);
                    }
                }
            }
        }
    }

    pub(super) fn select_elicitation_value(
        &mut self,
        request_id: &str,
        name: String,
        value: serde_json::Value,
        multiple: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.elicitation_forms.get_mut(request_id) else {
            return;
        };
        let key = format!("{request_id}:{name}");
        if multiple {
            let values = form
                .values
                .entry(key)
                .or_insert_with(|| serde_json::Value::Array(Vec::new()));
            let Some(values) = values.as_array_mut() else {
                return;
            };
            if let Some(index) = values.iter().position(|candidate| candidate == &value) {
                values.remove(index);
            } else {
                values.push(value);
            }
        } else {
            form.values.insert(key, value);
        }
        cx.notify();
    }

    pub(super) fn submit_elicitation(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(form) = self.elicitation_forms.get(&id) else {
            return;
        };
        let fields = form.fields.clone();
        let text_values = fields
            .iter()
            .filter_map(|field| {
                let key = format!("{id}:{}", field.name);
                form.editors
                    .get(&key)
                    .map(|editor| (field.name.clone(), editor.read(cx).text()))
            })
            .collect();
        let choice_values = form
            .values
            .iter()
            .filter_map(|(key, value)| {
                key.strip_prefix(&format!("{id}:"))
                    .map(|name| (name.to_string(), value.clone()))
            })
            .collect();
        let values = match values_map(&fields, &text_values, &choice_values) {
            Ok(values) => values,
            Err(error) => {
                self.push_entry(Entry::Error(error));
                cx.notify();
                return;
            }
        };
        self.respond_elicitation(id, ElicitationResponse::Accept(values), cx);
    }

    pub(super) fn respond_elicitation(
        &mut self,
        id: String,
        response: ElicitationResponse,
        cx: &mut Context<Self>,
    ) {
        let Some(handle) = &self.handle else {
            self.push_entry(Entry::Error(
                i18n::text(self.language, "acp.disconnected").into(),
            ));
            cx.notify();
            return;
        };
        if let Err(error) = handle.respond_elicitation(id.clone(), response) {
            self.push_entry(Entry::Error(error.to_string()));
            cx.notify();
            return;
        }
        self.pending_elicitations.retain(|request| request.id != id);
        self.elicitation_forms.remove(&id);
        self.refresh_status();
        cx.notify();
    }

    fn open_elicitation_url(&mut self, url: &str, cx: &mut Context<Self>) {
        if valid_elicitation_url(url) {
            cx.open_url(url);
        } else {
            self.push_entry(Entry::Error(
                i18n::text(self.language, "acp.elicitation_invalid_url").into(),
            ));
            cx.notify();
        }
    }
}

/// Selection lives in the marker; the outer border remains a focus indicator.
fn choice_button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<gpui::SharedString>,
    theme: Theme,
    selected: bool,
    multiple: bool,
) -> gpui::Stateful<gpui::Div> {
    button(id, label, theme)
        .role(if multiple {
            gpui::Role::CheckBox
        } else {
            gpui::Role::RadioButton
        })
        .aria_toggled(if selected {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .border_color(rgba(theme.line))
        .text_color(rgba(theme.fg0))
        .child(
            div()
                .flex_none()
                .size(ui_px(14.))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(rgba(if selected { theme.accent } else { theme.fg2 }))
                .text_size(ui_px(11.))
                .line_height(ui_px(12.))
                .text_color(rgba(theme.fg0))
                .child(if selected { "✓" } else { "" }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_expose_radio_or_checkbox_and_explicit_toggled_state() {
        use gpui::Element;
        let theme = Theme::for_mode(crate::theme::ThemeMode::Light);
        for multiple in [false, true] {
            for selected in [false, true] {
                let choice = choice_button("choice", "Choice", theme, selected, multiple);
                let role = if multiple {
                    gpui::Role::CheckBox
                } else {
                    gpui::Role::RadioButton
                };
                assert_eq!(choice.a11y_role(), Some(role));
                let mut node = gpui::accesskit::Node::new(role);
                choice.write_a11y_info(&mut node);
                assert_eq!(
                    node.toggled(),
                    Some(if selected {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                );
                assert_eq!(node.label(), Some("Choice"));
            }
        }
    }

    fn request(id: &str) -> ElicitationRequest {
        ElicitationRequest {
            id: id.into(),
            message: format!("Request {id}"),
            mode: ElicitationMode::Form {
                schema: serde_json::json!({
                    "type": "object",
                    "required": ["name"],
                    "properties": {"name": {"type": "string"}}
                }),
            },
        }
    }

    #[test]
    fn form_state_parses_once_and_keeps_requests_isolated() {
        let request_a = request("a");
        let request_b = request("b");
        let mut forms = std::collections::HashMap::from([
            (
                request_a.id.clone(),
                ElicitationFormState::from_request(&request_a).unwrap(),
            ),
            (
                request_b.id.clone(),
                ElicitationFormState::from_request(&request_b).unwrap(),
            ),
        ]);
        assert_eq!(forms["a"].fields.len(), 1);
        assert_eq!(forms["b"].fields.len(), 1);
        forms
            .get_mut("a")
            .unwrap()
            .values
            .insert("a:name".into(), serde_json::json!("Alice"));
        forms
            .get_mut("b")
            .unwrap()
            .values
            .insert("b:name".into(), serde_json::json!("Bob"));

        forms.remove("a");

        assert!(!forms.contains_key("a"));
        assert_eq!(
            forms["b"].values.get("b:name"),
            Some(&serde_json::json!("Bob"))
        );
    }
}
