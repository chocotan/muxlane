//! Manual session titles, shared by the sidebar and terminal tab menus.
use crate::app::MuxlaneApp;
use crate::i18n;
use crate::text_field::TextField;
use crate::theme::Theme;
use crate::ui_scale::px as ui_px;
use crate::widgets::semantic_button;
use gpui::{div, prelude::*, relative, rgba, Context, Entity, Focusable, MouseButton, Window};
use muxlane_core::model::AgentId;
use std::sync::Arc;

pub(crate) struct SessionRename {
    pub(crate) agent: AgentId,
    pub(crate) input: Entity<TextField>,
    pub(crate) error: Option<String>,
    pub(crate) busy: bool,
    cancel_focus: gpui::FocusHandle,
    submit_focus: gpui::FocusHandle,
}

impl MuxlaneApp {
    pub(crate) fn begin_session_rename(
        &mut self,
        agent: &AgentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((title, _, _)) = self.session_summary(agent) else {
            return;
        };
        let input = cx.new(|cx| {
            let mut input = TextField::new("", window, cx);
            input.set_theme_mode(self.theme_mode, cx);
            input.set_text(title, cx);
            input
        });
        input.focus_handle(cx).focus(window, cx);
        self.session_rename = Some(SessionRename {
            agent: agent.clone(),
            input,
            error: None,
            busy: false,
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
        });
        self.session_menu = None;
        self.tree_menu = None;
        cx.notify();
    }

    pub(crate) fn cancel_session_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .session_rename
            .as_ref()
            .is_some_and(|rename| rename.busy)
        {
            return;
        }
        self.session_rename = None;
        self.focus.focus(window, cx);
        self.sync_active_terminal_focus(window, cx);
        cx.notify();
    }

    pub(crate) fn cycle_session_rename_focus(
        &self,
        reverse: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(rename) = &self.session_rename else {
            return;
        };
        let handles = [
            rename.input.focus_handle(cx),
            rename.cancel_focus.clone(),
            rename.submit_focus.clone(),
        ];
        let current = handles
            .iter()
            .position(|handle| handle.is_focused(window))
            .unwrap_or(0);
        let next = (current + if reverse { handles.len() - 1 } else { 1 }) % handles.len();
        handles[next].focus(window, cx);
    }

    pub(crate) fn handle_session_rename_key(
        &mut self,
        keystroke: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match keystroke.key.as_str() {
            "tab" | "f6" => self.cycle_session_rename_focus(keystroke.modifiers.shift, window, cx),
            "escape" => self.cancel_session_rename(window, cx),
            "enter" => {
                if self
                    .session_rename
                    .as_ref()
                    .is_some_and(|rename| rename.cancel_focus.is_focused(window))
                {
                    self.cancel_session_rename(window, cx);
                } else {
                    self.submit_session_rename(window, cx);
                }
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn submit_session_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.session_rename.as_mut().filter(|rename| !rename.busy) else {
            return;
        };
        let title = rename.input.read(cx).text().trim().to_string();
        if title.is_empty() || title.chars().count() > 128 || title.chars().any(char::is_control) {
            rename.error = Some(i18n::text(self.language, "error.session_title_invalid").into());
            cx.notify();
            return;
        }
        let agent = rename.agent.clone();
        let host = self
            .remote_snaps
            .iter()
            .find_map(|(host, snapshot)| snapshot.agent(&agent).is_some().then(|| host.clone()));
        let remote = host.as_ref().and_then(|host| {
            self.remotes
                .iter()
                .find(|remote| remote.cfg.name == *host)
                .cloned()
        });
        if host.is_some() {
            let error = match &remote {
                None => Some("error.remote_not_connected"),
                Some(remote)
                    if !remote.supports(muxlane_core::protocol::features::AGENT_RENAME) =>
                {
                    Some("error.remote_rename_unsupported")
                }
                _ => None,
            };
            if let Some(error) = error {
                rename.error = Some(i18n::text(self.language, error).into());
                cx.notify();
                return;
            }
        }
        rename.busy = true;
        rename.error = None;
        let server = Arc::clone(&self.server);
        let agent_for_rpc = agent.clone();
        let task = self.spawn_remote_operation(async move {
            match remote {
                Some(remote) => remote.rename_agent(&agent_for_rpc, title).await,
                None => server.rename_agent(&agent_for_rpc, title).await,
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(renamed) => {
                        let snapshot = match &host {
                            Some(host) => this.remote_snaps.get_mut(host),
                            None => Some(&mut this.last_snapshot),
                        };
                        if let Some(instance) =
                            snapshot.and_then(|snapshot| snapshot.agent_mut(&agent))
                        {
                            instance.title = renamed.title;
                            instance.title_is_custom = renamed.title_is_custom;
                        }
                        this.session_rename = None;
                        this.persist();
                        this.focus.focus(window, cx);
                        this.sync_active_terminal_focus(window, cx);
                    }
                    Err(error) => {
                        if let Some(rename) = this.session_rename.as_mut() {
                            rename.busy = false;
                            rename.error = Some(
                                i18n::text(this.language, "error.rename_session")
                                    .replace("{error}", &error.to_string()),
                            );
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn render_session_rename(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::for_mode(self.theme_mode);
        let Some(rename) = &self.session_rename else {
            return div().into_any_element();
        };
        let input = rename.input.clone();
        let busy = rename.busy;
        let error = rename.error.clone();
        let cancel_focus = rename.cancel_focus.clone();
        let submit_focus = rename.submit_focus.clone();
        div()
            .id("session-rename-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(theme.overlay()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.cancel_session_rename(window, cx);
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .id("session-rename-dialog")
                    .w(ui_px(460.))
                    .max_w(relative(0.92))
                    .bg(rgba(theme.bg1))
                    .border_1()
                    .border_color(rgba(theme.line))
                    .shadow_lg()
                    .on_any_mouse_down(cx.listener(|_, _: &gpui::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                    }))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                        if this.handle_session_rename_key(&event.keystroke, window, cx) {
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .border_b_1()
                            .border_color(rgba(theme.line))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgba(theme.fg0))
                            .child(i18n::text(self.language, "menu.rename_session")),
                    )
                    .child(
                        div()
                            .px_4()
                            .pt_3()
                            .text_size(ui_px(12.))
                            .text_color(rgba(theme.fg1))
                            .child(i18n::text(self.language, "dialog.session_title")),
                    )
                    .child(div().mx_4().mt_2().child(input))
                    .when_some(error, |dialog, error| {
                        dialog.child(
                            div()
                                .mx_4()
                                .mt_2()
                                .text_size(ui_px(12.))
                                .text_color(rgba(theme.red))
                                .child(error),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .px_4()
                            .py_3()
                            .child(
                                semantic_button(
                                    "session-rename-cancel",
                                    i18n::text(self.language, "common.cancel"),
                                    theme,
                                )
                                .track_focus(&cancel_focus)
                                .px_3()
                                .py_1()
                                .text_color(rgba(if busy { theme.fg2 } else { theme.fg0 }))
                                .when(busy, |button| button.cursor_default())
                                .when(!busy, |button| {
                                    button.hover(|style| style.bg(rgba(theme.bg2)))
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.cancel_session_rename(window, cx)
                                }))
                                .child(i18n::text(self.language, "common.cancel")),
                            )
                            .child(
                                semantic_button(
                                    "session-rename-submit",
                                    i18n::text(self.language, "common.save"),
                                    theme,
                                )
                                .track_focus(&submit_focus)
                                .px_3()
                                .py_1()
                                .bg(rgba(theme.accent))
                                .text_color(rgba(theme.on_accent))
                                .when(busy, |button| button.cursor_default().opacity(0.6))
                                .when(!busy, |button| {
                                    button
                                        .hover(|style| {
                                            style.bg(rgba(Theme::with_alpha(theme.accent, 0xcc)))
                                        })
                                        .active(|style| {
                                            style.bg(rgba(Theme::with_alpha(theme.accent, 0x99)))
                                        })
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.submit_session_rename(window, cx)
                                }))
                                .child(i18n::text(
                                    self.language,
                                    if busy { "common.saving" } else { "common.save" },
                                )),
                            ),
                    ),
            )
            .into_any_element()
    }
}
