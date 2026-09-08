//! Connection, local project, and remote project dialogs.
use crate::app::MuxlaneApp;
use crate::i18n;
use crate::theme::Theme;
use crate::ui_scale::px as ui_px;
use crate::widgets::semantic_button;
use gpui::{
    div, prelude::*, relative, rgba, Context, Focusable, MouseButton, ParentElement, Styled, Window,
};
use std::sync::Arc;

pub(crate) fn should_prompt_project_creation(
    error_code: Option<&str>,
    create_if_missing: bool,
) -> bool {
    !create_if_missing && error_code == Some(muxlane_core::protocol::error_codes::PATH_NOT_FOUND)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectAuthMode {
    SshConfig,
    PublicKey,
    Password,
}

impl MuxlaneApp {
    fn handle_connect_key(
        &mut self,
        keystroke: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match keystroke.key.as_str() {
            "escape" => {
                self.connect_dialog = false;
                self.dialog_error = None;
                cx.notify();
                true
            }
            "enter"
                if [
                    &self.connect_input,
                    &self.connect_username,
                    &self.connect_password,
                    &self.connect_key_path,
                ]
                .into_iter()
                .any(|input| input.focus_handle(cx).is_focused(window)) =>
            {
                let target = self.connect_input.read(cx).text();
                self.add_remote_target(target, cx);
                true
            }
            _ => false,
        }
    }

    fn add_local_project(&mut self, raw_path: String, cx: &mut Context<Self>) {
        if self.project_add_busy {
            return;
        }
        let path = raw_path.trim();
        if path.is_empty() {
            self.dialog_error =
                Some(i18n::text(self.language, "error.local_project_required").into());
            cx.notify();
            return;
        }
        self.submit_local_project(path.to_string(), false, cx);
    }

    pub(crate) fn submit_local_project(
        &mut self,
        path: String,
        create_if_missing: bool,
        cx: &mut Context<Self>,
    ) {
        if self.project_add_busy {
            return;
        }
        self.project_add_busy = true;
        let server = Arc::clone(&self.server);
        let requested_path = path.clone();
        let params = muxlane_core::protocol::ProjectAddParams {
            path,
            name: None,
            create_if_missing,
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    server.add_project(params).await?;
                    Ok::<_, anyhow::Error>(server.snapshot().await)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.project_add_busy = false;
                match result {
                    Ok(snapshot) => {
                        this.last_snapshot = snapshot;
                        this.project_dialog = false;
                        this.pending_project_creation = None;
                        this.dialog_error = None;
                        this.project_input.update(cx, |input, cx| input.reset(cx));
                        this.persist();
                    }
                    Err(error) => {
                        let error_code = error
                            .downcast_ref::<muxlane_server::ProjectAddError>()
                            .map(muxlane_server::ProjectAddError::code);
                        if should_prompt_project_creation(error_code, create_if_missing) {
                            this.pending_project_creation =
                                Some(crate::menus::PendingProjectCreation {
                                    target: crate::menus::ProjectCreationTarget::Local {
                                        path: requested_path,
                                    },
                                    error: None,
                                });
                            this.dialog_error = None;
                        } else if let Some(pending) = this.pending_project_creation.as_mut() {
                            pending.error = Some(error.to_string());
                        } else {
                            this.dialog_error = Some(error.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn handle_project_key(&mut self, ks: &gpui::Keystroke, cx: &mut Context<Self>) {
        if self.pending_project_creation.is_some() {
            match ks.key.as_str() {
                "escape" => self.cancel_project_create(cx),
                "enter" => self.confirm_project_create(cx),
                _ => {}
            }
            return;
        }
        match ks.key.as_str() {
            "escape" if !self.project_add_busy => {
                self.project_dialog = false;
                self.dialog_error = None;
            }
            "enter" => {
                let path = self.project_input.read(cx).text();
                self.add_local_project(path, cx);
                return;
            }
            _ => return,
        }
        cx.notify();
    }

    pub(crate) fn render_connect_dialog(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::for_mode(self.theme_mode);
        let input = self.connect_input.clone();
        let username = self.connect_username.clone();
        let password = self.connect_password.clone();
        let key_path = self.connect_key_path.clone();
        let auth_mode = self.connect_auth_mode;
        let error = self.dialog_error.clone();
        div()
            .id("connect-dialog-backdrop")
            .absolute()
            .size_full()
            .flex()
            .items_start()
            .justify_center()
            .pt(ui_px(90.))
            .bg(rgba(theme.overlay()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _ev, _window, cx| {
                    this.connect_dialog = false;
                    this.dialog_error = None;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .occlude()
                    .w(ui_px(480.))
                    .max_w(relative(0.92))
                    .bg(rgba(theme.bg1))
                    .border_1()
                    .border_color(rgba(theme.line))
                    .shadow_lg()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_this, _ev, _window, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, window, cx| {
                        if this.handle_connect_key(&ev.keystroke, window, cx) {
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
                            .child(i18n::text(self.language, "dialog.connect_remote")),
                    )
                    .child(
                        div()
                            .px_4()
                            .pt_3()
                            .text_size(ui_px(11.))
                            .text_color(rgba(theme.fg1))
                            .child(i18n::text(self.language, "dialog.connect_remote_help")),
                    )
                    .child(div().mx_4().mt_3().child(input))
                    .child(
                        div()
                            .mx_4()
                            .mt_2()
                            .flex()
                            .border_1()
                            .border_color(rgba(theme.line))
                            .overflow_hidden()
                            .child(
                                semantic_button(
                                    "auth-config",
                                    i18n::text(self.language, "dialog.auth_ssh_config"),
                                    theme,
                                )
                                .flex_1()
                                .px_2()
                                .py_1()
                                .text_size(ui_px(11.))
                                .text_color(rgba(theme.fg1))
                                .when(auth_mode == ConnectAuthMode::SshConfig, |item| {
                                    item.bg(rgba(theme.selection())).text_color(rgba(theme.fg0))
                                })
                                .hover(|item| item.bg(rgba(theme.bg2)))
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.connect_auth_mode = ConnectAuthMode::SshConfig;
                                    cx.notify();
                                }))
                                .child(i18n::text(self.language, "dialog.auth_ssh_config")),
                            )
                            .child(
                                semantic_button(
                                    "auth-key",
                                    i18n::text(self.language, "dialog.auth_public_key"),
                                    theme,
                                )
                                .flex_1()
                                .px_2()
                                .py_1()
                                .text_size(ui_px(11.))
                                .text_color(rgba(theme.fg1))
                                .when(auth_mode == ConnectAuthMode::PublicKey, |item| {
                                    item.bg(rgba(theme.selection())).text_color(rgba(theme.fg0))
                                })
                                .hover(|item| item.bg(rgba(theme.bg2)))
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.connect_auth_mode = ConnectAuthMode::PublicKey;
                                    cx.notify();
                                }))
                                .child(i18n::text(self.language, "dialog.auth_public_key")),
                            )
                            .child(
                                semantic_button(
                                    "auth-password",
                                    i18n::text(self.language, "dialog.auth_password"),
                                    theme,
                                )
                                .flex_1()
                                .px_2()
                                .py_1()
                                .text_size(ui_px(11.))
                                .text_color(rgba(theme.fg1))
                                .when(auth_mode == ConnectAuthMode::Password, |item| {
                                    item.bg(rgba(theme.selection())).text_color(rgba(theme.fg0))
                                })
                                .hover(|item| item.bg(rgba(theme.bg2)))
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.connect_auth_mode = ConnectAuthMode::Password;
                                    cx.notify();
                                }))
                                .child(i18n::text(self.language, "dialog.auth_password")),
                            ),
                    )
                    .when(auth_mode == ConnectAuthMode::PublicKey, |dialog| {
                        dialog
                            .child(div().mx_4().mt_2().child(username.clone()))
                            .child(div().mx_4().mt_2().child(key_path))
                    })
                    .when(auth_mode == ConnectAuthMode::Password, |dialog| {
                        dialog
                            .child(div().mx_4().mt_2().child(username))
                            .child(div().mx_4().mt_2().child(password))
                    })
                    .when_some(error, |dialog, error| {
                        dialog.child(
                            div()
                                .mx_4()
                                .mt_2()
                                .text_size(ui_px(11.))
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
                                    "connect-cancel",
                                    i18n::text(self.language, "common.cancel"),
                                    theme,
                                )
                                .px_3()
                                .py_1()
                                .text_color(rgba(theme.fg0))
                                .hover(|s| s.bg(rgba(theme.bg2)))
                                .on_click(cx.listener(|this, _ev, _window, cx| {
                                    this.connect_dialog = false;
                                    this.dialog_error = None;
                                    cx.notify();
                                }))
                                .child(i18n::text(self.language, "common.cancel")),
                            )
                            .child(
                                semantic_button(
                                    "connect-submit",
                                    i18n::text(self.language, "common.connect"),
                                    theme,
                                )
                                .px_3()
                                .py_1()
                                .bg(rgba(theme.accent))
                                .text_color(rgba(theme.on_accent))
                                .hover(|s| s.bg(rgba(Theme::with_alpha(theme.accent, 0xcc))))
                                .active(|s| s.bg(rgba(Theme::with_alpha(theme.accent, 0x99))))
                                .on_click(cx.listener(|this, _ev, _window, cx| {
                                    let target = this.connect_input.read(cx).text();
                                    this.add_remote_target(target, cx);
                                }))
                                .child(i18n::text(self.language, "common.connect")),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn render_remote_project_dialog(
        &mut self,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::for_mode(self.theme_mode);
        let Some(host) = self.remote_project_dialog.clone() else {
            return div().into_any_element();
        };
        let input = self.remote_project_input.clone();
        let busy = self.project_add_busy;
        div()
            .id("remote-project-backdrop")
            .absolute()
            .size_full()
            .flex()
            .items_start()
            .justify_center()
            .pt(ui_px(90.))
            .bg(rgba(theme.overlay()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event, _window, cx| {
                    if !this.project_add_busy && this.pending_project_creation.is_none() {
                        this.remote_project_dialog = None;
                        this.dialog_error = None;
                        cx.notify();
                    }
                }),
            )
            .child(
                div()
                    .occlude()
                    .w(ui_px(480.))
                    .max_w(relative(0.92))
                    .bg(rgba(theme.bg1))
                    .border_1()
                    .border_color(rgba(theme.line))
                    .shadow_lg()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_this, _ev, _window, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .on_key_down(
                        cx.listener(|this, event: &gpui::KeyDownEvent, _window, cx| {
                            match event.keystroke.key.as_str() {
                                "escape" if this.pending_project_creation.is_some() => {
                                    this.cancel_project_create(cx);
                                    cx.stop_propagation();
                                }
                                "enter" if this.pending_project_creation.is_some() => {
                                    this.confirm_project_create(cx);
                                    cx.stop_propagation();
                                }
                                "escape" if !this.project_add_busy => {
                                    this.remote_project_dialog = None;
                                    this.dialog_error = None;
                                    cx.stop_propagation();
                                    cx.notify();
                                }
                                "enter" => {
                                    if let Some(host) = this.remote_project_dialog.clone() {
                                        let path = this.remote_project_input.read(cx).text();
                                        this.submit_remote_project(host, path, cx);
                                    }
                                    cx.stop_propagation();
                                }
                                _ => {}
                            }
                        }),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .border_b_1()
                            .border_color(rgba(theme.line))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgba(theme.fg0))
                            .child(
                                i18n::text(self.language, "dialog.add_remote_project")
                                    .replace("{host}", &host),
                            ),
                    )
                    .child(
                        div()
                            .px_4()
                            .pt_3()
                            .text_size(ui_px(11.))
                            .text_color(rgba(theme.fg1))
                            .child(i18n::text(self.language, "dialog.add_remote_project_help")),
                    )
                    .child(div().mx_4().mt_3().child(input))
                    .when_some(self.dialog_error.clone(), |dialog, error| {
                        dialog.child(
                            div()
                                .mx_4()
                                .mt_2()
                                .text_size(ui_px(11.))
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
                                    "remote-project-cancel",
                                    i18n::text(self.language, "common.cancel"),
                                    theme,
                                )
                                .px_3()
                                .py_1()
                                .text_color(rgba(theme.fg0))
                                .when(!busy, |button| {
                                    button.hover(|style| style.bg(rgba(theme.bg2)))
                                })
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    if !this.project_add_busy {
                                        this.remote_project_dialog = None;
                                        this.dialog_error = None;
                                        cx.notify();
                                    }
                                }))
                                .child(i18n::text(self.language, "common.cancel")),
                            )
                            .child(
                                semantic_button(
                                    "remote-project-submit",
                                    if busy {
                                        i18n::text(self.language, "common.adding")
                                    } else {
                                        i18n::text(self.language, "common.add")
                                    },
                                    theme,
                                )
                                .px_3()
                                .py_1()
                                .bg(rgba(theme.accent))
                                .text_color(rgba(theme.on_accent))
                                .when(!busy, |button| {
                                    button.hover(|style| {
                                        style.bg(rgba(Theme::with_alpha(theme.accent, 0xcc)))
                                    })
                                })
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    if let Some(host) = this.remote_project_dialog.clone() {
                                        let path = this.remote_project_input.read(cx).text();
                                        this.submit_remote_project(host, path, cx);
                                    }
                                }))
                                .child(if busy {
                                    i18n::text(self.language, "common.adding")
                                } else {
                                    i18n::text(self.language, "common.add")
                                }),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn render_project_dialog(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::for_mode(self.theme_mode);
        let input = self.project_input.clone();
        let error = self.dialog_error.clone();
        let busy = self.project_add_busy;
        div()
            .id("project-dialog-backdrop")
            .absolute()
            .size_full()
            .flex()
            .items_start()
            .justify_center()
            .pt(ui_px(90.))
            .bg(rgba(theme.overlay()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _ev, _window, cx| {
                    if !this.project_add_busy && this.pending_project_creation.is_none() {
                        this.project_dialog = false;
                        this.dialog_error = None;
                        cx.notify();
                    }
                }),
            )
            .child(
                div()
                    .occlude()
                    .w(ui_px(480.))
                    .max_w(relative(0.92))
                    .bg(rgba(theme.bg1))
                    .border_1()
                    .border_color(rgba(theme.line))
                    .shadow_lg()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_this, _ev, _window, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _window, cx| {
                        if matches!(ev.keystroke.key.as_str(), "enter" | "escape") {
                            this.handle_project_key(&ev.keystroke, cx);
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
                            .child(i18n::text(self.language, "dialog.add_local_project")),
                    )
                    .child(
                        div()
                            .px_4()
                            .pt_3()
                            .text_size(ui_px(11.))
                            .text_color(rgba(theme.fg1))
                            .child(i18n::text(self.language, "dialog.add_local_project_help")),
                    )
                    .child(div().mx_4().mt_3().child(input))
                    .when_some(error, |dialog, error| {
                        dialog.child(
                            div()
                                .mx_4()
                                .mt_2()
                                .text_size(ui_px(11.))
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
                                    "project-cancel",
                                    i18n::text(self.language, "common.cancel"),
                                    theme,
                                )
                                .px_3()
                                .py_1()
                                .text_color(rgba(theme.fg0))
                                .when(!busy, |button| {
                                    button.hover(|style| style.bg(rgba(theme.bg2)))
                                })
                                .on_click(cx.listener(|this, _ev, _window, cx| {
                                    if !this.project_add_busy {
                                        this.project_dialog = false;
                                        this.dialog_error = None;
                                        cx.notify();
                                    }
                                }))
                                .child(i18n::text(self.language, "common.cancel")),
                            )
                            .child(
                                semantic_button(
                                    "project-submit",
                                    if busy {
                                        i18n::text(self.language, "common.adding")
                                    } else {
                                        i18n::text(self.language, "common.add")
                                    },
                                    theme,
                                )
                                .px_3()
                                .py_1()
                                .bg(rgba(theme.accent))
                                .text_color(rgba(theme.on_accent))
                                .when(!busy, |button| {
                                    button.hover(|style| {
                                        style.bg(rgba(Theme::with_alpha(theme.accent, 0xcc)))
                                    })
                                })
                                .on_click(cx.listener(|this, _ev, _window, cx| {
                                    let path = this.project_input.read(cx).text();
                                    this.add_local_project(path, cx);
                                }))
                                .child(if busy {
                                    i18n::text(self.language, "common.adding")
                                } else {
                                    i18n::text(self.language, "common.add")
                                }),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn show_quit_confirmation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !request_quit_confirmation(&mut self.quit_confirm_open, self.quit_confirmed) {
            return;
        }
        self.palette_open = false;
        self.new_session_target = None;
        self.connect_dialog = false;
        self.project_dialog = false;
        self.remote_project_dialog = None;
        self.dialog_error = None;
        crate::menus::dismiss_context_menus(&mut self.session_menu, &mut self.tree_menu);
        self.cancel_delete(cx);
        self.bootstrap_confirm = None;
        self.bootstrap_error = None;
        self.pending_project_creation = None;
        if self.settings_open {
            self.close_settings(window, cx);
        }
        self.notifications.update(cx, |center, cx| center.close(cx));
        self.quit_cancel_focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn handle_quit_confirmation_key(
        &mut self,
        keystroke: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match keystroke.key.as_str() {
            "escape" => {
                cancel_quit_confirmation(&mut self.quit_confirm_open);
                self.focus.focus(window, cx);
                cx.notify();
                true
            }
            "enter" if self.quit_exit_focus.is_focused(window) => {
                self.confirm_quit(window, cx);
                true
            }
            "enter" => {
                cancel_quit_confirmation(&mut self.quit_confirm_open);
                self.focus.focus(window, cx);
                cx.notify();
                true
            }
            "tab" => {
                let focus = if keystroke.modifiers.shift {
                    if self.quit_cancel_focus.is_focused(window) {
                        &self.quit_exit_focus
                    } else {
                        &self.quit_cancel_focus
                    }
                } else if self.quit_exit_focus.is_focused(window) {
                    &self.quit_cancel_focus
                } else {
                    &self.quit_exit_focus
                };
                focus.focus(window, cx);
                true
            }
            _ => false,
        }
    }

    fn confirm_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quit_confirmed || !self.quit_confirm_open {
            return;
        }
        self.close_all_session_windows(cx);
        self.quit_confirmed = true;
        self.quit_confirm_open = false;
        self.persist();
        window.remove_window();
    }

    pub(crate) fn render_quit_confirmation(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::for_mode(self.theme_mode);
        let cancel_focus = self.quit_cancel_focus.clone();
        let exit_focus = self.quit_exit_focus.clone();
        div()
            .id("quit-confirm-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(theme.overlay()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _event, _window, cx| {
                    cx.stop_propagation();
                }),
            )
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if this.handle_quit_confirmation_key(&event.keystroke, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .id("quit-confirm-dialog")
                    .w(ui_px(420.))
                    .max_w(relative(0.92))
                    .bg(rgba(theme.bg1))
                    .border_1()
                    .border_color(rgba(theme.line))
                    .shadow_lg()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_this, _event, _window, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .border_b_1()
                            .border_color(rgba(theme.line))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgba(theme.fg0))
                            .child(i18n::text(self.language, "confirm.quit_title")),
                    )
                    .child(
                        div()
                            .px_4()
                            .pt_3()
                            .text_size(ui_px(12.))
                            .text_color(rgba(theme.fg1))
                            .child(i18n::text(self.language, "confirm.quit_copy")),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                semantic_button(
                                    "quit-confirm-cancel",
                                    i18n::text(self.language, "common.cancel"),
                                    theme,
                                )
                                .track_focus(&cancel_focus)
                                .tab_index(1)
                                .px_3()
                                .py_1()
                                .border_1()
                                .border_color(rgba(theme.line))
                                .text_color(rgba(theme.fg1))
                                .hover(|style| style.bg(rgba(theme.bg2)))
                                .on_click(cx.listener(|this, _event, window, cx| {
                                    cancel_quit_confirmation(&mut this.quit_confirm_open);
                                    this.focus.focus(window, cx);
                                    cx.notify();
                                }))
                                .child(i18n::text(self.language, "common.cancel")),
                            )
                            .child(
                                semantic_button(
                                    "quit-confirm-submit",
                                    i18n::text(self.language, "confirm.quit"),
                                    theme,
                                )
                                .track_focus(&exit_focus)
                                .tab_index(2)
                                .px_3()
                                .py_1()
                                .bg(rgba(theme.red))
                                .text_color(rgba(theme.bg0))
                                .hover(|style| style.bg(rgba(Theme::with_alpha(theme.red, 0xcc))))
                                .on_click(cx.listener(|this, _event, window, cx| {
                                    this.confirm_quit(window, cx);
                                }))
                                .child(i18n::text(self.language, "confirm.quit")),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn open_connect_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.connect_dialog = true;
        self.project_dialog = false;
        self.dialog_error = None;
        self.connect_input.update(cx, |input, cx| input.reset(cx));
        self.connect_username
            .update(cx, |input, cx| input.reset(cx));
        self.connect_password
            .update(cx, |input, cx| input.reset(cx));
        self.connect_key_path
            .update(cx, |input, cx| input.reset(cx));
        self.connect_input.focus_handle(cx).focus(window, cx);
        cx.notify();
    }
}

fn request_quit_confirmation(open: &mut bool, confirmed: bool) -> bool {
    if confirmed || *open {
        false
    } else {
        *open = true;
        true
    }
}

fn cancel_quit_confirmation(open: &mut bool) {
    *open = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_confirmation_intercepts_repeated_requests_and_can_cancel() {
        let mut open = false;
        assert!(request_quit_confirmation(&mut open, false));
        assert!(open);
        assert!(!request_quit_confirmation(&mut open, false));
        cancel_quit_confirmation(&mut open);
        assert!(!open);
    }

    #[test]
    fn confirmed_quit_request_is_ignored_by_confirmation_state() {
        let mut open = false;
        assert!(!request_quit_confirmation(&mut open, true));
    }

    #[test]
    fn project_creation_prompt_requires_first_typed_not_found_error() {
        assert!(should_prompt_project_creation(
            Some(muxlane_core::protocol::error_codes::PATH_NOT_FOUND),
            false
        ));
        assert!(!should_prompt_project_creation(
            Some(muxlane_core::protocol::error_codes::PATH_NOT_FOUND),
            true
        ));
        assert!(!should_prompt_project_creation(
            Some(muxlane_core::protocol::error_codes::NOT_A_DIRECTORY),
            false
        ));
        assert!(!should_prompt_project_creation(None, false));
    }
}
