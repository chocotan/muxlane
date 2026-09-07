//! Context menus and confirmation dialogs.
use crate::app::MuxlaneApp;
use crate::i18n;
use crate::theme::Theme;
use crate::ui_scale::px as ui_px;
use crate::widgets::{format_upload_phase, semantic_button};
use gpui::{
    div, prelude::*, relative, rgba, Context, Focusable, ParentElement, Pixels, Point, Styled,
};
use muxlane_core::model::AgentId;

pub(crate) fn clamp_menu_position(
    anchor: Point<Pixels>,
    viewport: gpui::Size<Pixels>,
    menu_size: gpui::Size<Pixels>,
) -> Point<Pixels> {
    let x = if anchor.x + menu_size.width > viewport.width {
        anchor.x - menu_size.width
    } else {
        anchor.x
    };
    let y = if anchor.y + menu_size.height > viewport.height {
        anchor.y - menu_size.height
    } else {
        anchor.y
    };
    Point::new(
        x.max(Pixels::ZERO)
            .min((viewport.width - menu_size.width).max(Pixels::ZERO)),
        y.max(Pixels::ZERO)
            .min((viewport.height - menu_size.height).max(Pixels::ZERO)),
    )
}

#[derive(Clone)]
pub(crate) struct SessionMenu {
    pub(crate) agent: AgentId,
    pub(crate) position: Point<Pixels>,
    pub(crate) remote: bool,
}

#[derive(Clone)]
pub(crate) enum DeleteTarget {
    LocalProject {
        project: String,
        label: String,
    },
    RemoteProject {
        host: String,
        project: String,
        label: String,
    },
    RemoteMachine {
        host: String,
    },
}

#[derive(Clone)]
pub(crate) struct TreeMenu {
    pub(crate) target: DeleteTarget,
    pub(crate) position: Point<Pixels>,
}

pub(crate) fn dismiss_context_menus(
    session_menu: &mut Option<SessionMenu>,
    tree_menu: &mut Option<TreeMenu>,
) -> bool {
    let had_open_menu = session_menu.is_some() || tree_menu.is_some();
    *session_menu = None;
    *tree_menu = None;
    had_open_menu
}

#[derive(Clone)]
pub(crate) struct DeleteConfirm {
    pub(crate) target: DeleteTarget,
    pub(crate) affected_sessions: usize,
}

#[derive(Clone)]
pub(crate) enum ProjectCreationTarget {
    Local { path: String },
    Remote { host: String, path: String },
}

#[derive(Clone)]
pub(crate) struct PendingProjectCreation {
    pub(crate) target: ProjectCreationTarget,
    pub(crate) error: Option<String>,
}

#[derive(Clone)]
pub(crate) struct BootstrapConfirm {
    pub(crate) host: String,
    pub(crate) install: bool,
    pub(crate) upgrade: bool,
    pub(crate) binary: Option<String>,
}

impl MuxlaneApp {
    pub(crate) fn render_session_menu(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::for_mode(self.theme_mode);
        let Some(menu) = self.session_menu.clone() else {
            return div().into_any_element();
        };
        let is_acp = self.is_acp_session(&menu.agent);
        let (acp_view, can_logout) = self
            .acp_views
            .get(&menu.agent)
            .map(|view| {
                let state = view.read(cx);
                (
                    Some(view.clone()),
                    state.handle.is_some()
                        && state
                            .capabilities
                            .as_ref()
                            .is_some_and(|capabilities| capabilities.logout),
                )
            })
            .unwrap_or((None, false));
        div()
            .absolute()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _event, _window, cx| {
                if dismiss_context_menus(&mut this.session_menu, &mut this.tree_menu) {
                    cx.notify();
                }
            }))
            .on_any_mouse_down(
                cx.listener(|_this, _event: &gpui::MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                }),
            )
            .left(menu.position.x)
            .top(menu.position.y)
            .w(ui_px(180.))
            .bg(rgba(theme.bg1))
            .border_1()
            .border_color(rgba(theme.line))
            .shadow_lg()
            .when(is_acp && can_logout, |menu_element| {
                let view = acp_view.clone();
                menu_element.child(
                    semantic_button(
                        "session-logout",
                        i18n::text(self.language, "acp.logout"),
                        theme,
                    )
                    .px_3()
                    .py_2()
                    .text_size(ui_px(12.))
                    .text_color(rgba(theme.fg0))
                    .hover(|style| style.bg(rgba(theme.bg2)))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.session_menu = None;
                        if let Some(view) = &view {
                            view.update(cx, |view, cx| view.logout(cx));
                        }
                        cx.notify();
                    }))
                    .child(i18n::text(self.language, "acp.logout")),
                )
            })
            .child(
                semantic_button(
                    "session-delete",
                    if menu.remote {
                        i18n::text(self.language, "menu.delete_remote_session")
                    } else {
                        i18n::text(self.language, "menu.delete_session")
                    },
                    theme,
                )
                .px_3()
                .py_2()
                .text_size(ui_px(12.))
                .text_color(rgba(theme.red))
                .hover(|s| s.bg(rgba(theme.bg2)))
                .on_click(cx.listener({
                    let id = menu.agent.clone();
                    let remote = menu.remote;
                    move |this, _ev, window, cx| {
                        this.request_session_delete(&id, remote, window, cx)
                    }
                }))
                .child(if menu.remote {
                    i18n::text(self.language, "menu.delete_remote_session")
                } else {
                    i18n::text(self.language, "menu.delete_session")
                }),
            )
            .into_any_element()
    }

    pub(crate) fn render_acp_delete_confirm(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::for_mode(self.theme_mode);
        let Some(agent) = self.acp_delete_confirm.clone() else {
            return div().into_any_element();
        };
        let title = self
            .acp_records
            .get(&agent)
            .map(|record| record.metadata.title.clone())
            .unwrap_or_else(|| "Agent Thread".into());
        div()
            .absolute()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(theme.overlay()))
            .child(
                div()
                    .w(ui_px(420.))
                    .max_w(relative(0.9))
                    .p_4()
                    .bg(rgba(theme.bg1))
                    .border_1()
                    .border_color(rgba(theme.line))
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(i18n::text(self.language, "acp.delete_title")),
                    )
                    .child(
                        div()
                            .py_3()
                            .text_size(ui_px(11.))
                            .text_color(rgba(theme.fg1))
                            .child(
                                i18n::text(self.language, "acp.delete_copy")
                                    .replace("{title}", &title),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                semantic_button(
                                    "acp-delete-cancel",
                                    i18n::text(self.language, "common.cancel"),
                                    theme,
                                )
                                .px_3()
                                .py_2()
                                .border_1()
                                .border_color(rgba(theme.line))
                                .on_click(cx.listener(|this, _event, window, cx| {
                                    this.acp_delete_confirm = None;
                                    this.focus.focus(window, cx);
                                    cx.notify();
                                }))
                                .child(i18n::text(self.language, "common.cancel")),
                            )
                            .child(
                                semantic_button(
                                    "acp-delete-confirm",
                                    i18n::text(self.language, "menu.delete_session"),
                                    theme,
                                )
                                .px_3()
                                .py_2()
                                .bg(rgba(theme.red))
                                .text_color(rgba(theme.on_accent))
                                .on_click(cx.listener(|this, _event, window, cx| {
                                    this.confirm_acp_session_delete(window, cx)
                                }))
                                .child(i18n::text(self.language, "menu.delete_session")),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn render_tree_menu(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::for_mode(self.theme_mode);
        let Some(menu) = self.tree_menu.clone() else {
            return div().into_any_element();
        };
        let menu_el = match &menu.target {
            DeleteTarget::RemoteMachine { host } => {
                let host_name = host.clone();
                let host_name_2 = host.clone();
                let host_name_3 = host.clone();
                let host_obj = self.remotes.iter().find(|r| r.cfg.name == *host).cloned();
                div()
                    .w(ui_px(200.))
                    .bg(rgba(theme.bg1))
                    .border_1()
                    .border_color(rgba(theme.line))
                    .shadow_lg()
                    .child(
                        semantic_button(
                            "tree-reconnect",
                            i18n::text(self.language, "menu.reconnect"),
                            theme,
                        )
                        .px_3()
                        .py_2()
                        .text_size(ui_px(12.))
                        .text_color(rgba(theme.fg0))
                        .hover(|style| style.bg(rgba(theme.bg2)))
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            if let Some(h) = &host_obj {
                                h.reconnect();
                                this.focus.focus(window, cx);
                            }
                            this.tree_menu = None;
                            cx.notify();
                        }))
                        .child(i18n::text(self.language, "menu.reconnect")),
                    )
                    .child(
                        semantic_button(
                            "tree-add-project",
                            i18n::text(self.language, "menu.add_remote_project"),
                            theme,
                        )
                        .px_3()
                        .py_2()
                        .text_size(ui_px(12.))
                        .text_color(rgba(theme.fg0))
                        .hover(|style| style.bg(rgba(theme.bg2)))
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            this.tree_menu = None;
                            this.remote_project_dialog = Some(host_name.clone());
                            this.dialog_error = None;
                            this.remote_project_input
                                .update(cx, |input, cx| input.reset(cx));
                            this.remote_project_input.focus_handle(cx).focus(window, cx);
                            cx.notify();
                        }))
                        .child(i18n::text(self.language, "menu.add_remote_project")),
                    )
                    .child(
                        semantic_button(
                            "tree-upgrade-muxlane",
                            i18n::text(self.language, "menu.upgrade_remote"),
                            theme,
                        )
                        .px_3()
                        .py_2()
                        .text_size(ui_px(12.))
                        .text_color(rgba(theme.accent))
                        .hover(|style| style.bg(rgba(theme.bg2)))
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.tree_menu = None;
                            this.bootstrap_error = None;
                            this.bootstrap_confirm = Some(BootstrapConfirm {
                                host: host_name_2.clone(),
                                install: false,
                                upgrade: true,
                                binary: None,
                            });
                            cx.notify();
                        }))
                        .child(i18n::text(self.language, "menu.upgrade_remote")),
                    )
                    .child(
                        semantic_button(
                            "tree-reinstall-muxlane",
                            i18n::text(self.language, "menu.reinstall_remote"),
                            theme,
                        )
                        .px_3()
                        .py_2()
                        .text_size(ui_px(12.))
                        .text_color(rgba(theme.fg1))
                        .hover(|style| style.bg(rgba(theme.bg2)))
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.tree_menu = None;
                            this.bootstrap_error = None;
                            this.bootstrap_confirm = Some(BootstrapConfirm {
                                host: host_name_3.clone(),
                                install: true,
                                upgrade: false,
                                binary: None,
                            });
                            cx.notify();
                        }))
                        .child(i18n::text(self.language, "menu.reinstall_remote")),
                    )
                    .child(div().h(ui_px(1.)).bg(rgba(theme.line)).my_1())
                    .child(
                        semantic_button(
                            "tree-delete-machine",
                            i18n::text(self.language, "menu.delete_remote_machine"),
                            theme,
                        )
                        .px_3()
                        .py_2()
                        .text_size(ui_px(12.))
                        .text_color(rgba(theme.red))
                        .hover(|style| style.bg(rgba(theme.bg2)))
                        .on_click(cx.listener({
                            let target = menu.target.clone();
                            move |this, _event, _window, cx| {
                                this.tree_menu = None;
                                this.begin_delete(target.clone(), cx);
                            }
                        }))
                        .child(i18n::text(self.language, "menu.delete_remote_machine")),
                    )
            }
            DeleteTarget::LocalProject { .. } | DeleteTarget::RemoteProject { .. } => {
                let label = match &menu.target {
                    DeleteTarget::LocalProject { .. } => {
                        i18n::text(self.language, "menu.delete_project_ellipsis")
                    }
                    DeleteTarget::RemoteProject { .. } => {
                        i18n::text(self.language, "menu.delete_remote_project")
                    }
                    _ => unreachable!(),
                };
                div()
                    .w(ui_px(190.))
                    .bg(rgba(theme.bg1))
                    .border_1()
                    .border_color(rgba(theme.line))
                    .shadow_lg()
                    .child(
                        semantic_button("tree-delete-project", label, theme)
                            .px_3()
                            .py_2()
                            .text_size(ui_px(12.))
                            .text_color(rgba(theme.red))
                            .hover(|style| style.bg(rgba(theme.bg2)))
                            .on_click(cx.listener({
                                let target = menu.target.clone();
                                move |this, _event, _window, cx| {
                                    this.tree_menu = None;
                                    this.begin_delete(target.clone(), cx);
                                }
                            }))
                            .child(label),
                    )
            }
        };
        div()
            .absolute()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _event, _window, cx| {
                if dismiss_context_menus(&mut this.session_menu, &mut this.tree_menu) {
                    cx.notify();
                }
            }))
            .on_any_mouse_down(
                cx.listener(|_this, _event: &gpui::MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                }),
            )
            .left(menu.position.x)
            .top(menu.position.y)
            .child(menu_el)
            .into_any_element()
    }

    pub(crate) fn render_delete_confirm(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::for_mode(self.theme_mode);
        let busy = self.delete_busy;
        let Some(confirm) = self.delete_confirm.clone() else {
            return div().into_any_element();
        };
        let (title, label, destructive_copy) = match &confirm.target {
            DeleteTarget::LocalProject { label, .. }
            | DeleteTarget::RemoteProject { label, .. } => (
                i18n::text(self.language, "menu.delete_project"),
                label.clone(),
                i18n::text(self.language, "confirm.delete_project_copy")
                    .replace("{count}", &confirm.affected_sessions.to_string()),
            ),
            DeleteTarget::RemoteMachine { host } => (
                i18n::text(self.language, "menu.delete_remote_machine_title"),
                host.clone(),
                i18n::text(self.language, "confirm.delete_remote_copy").into(),
            ),
        };
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(theme.overlay()))
            .child(
                div()
                    .w(ui_px(460.))
                    .max_w(relative(0.92))
                    .bg(rgba(theme.bg1))
                    .border_1()
                    .border_color(rgba(theme.line))
                    .shadow_lg()
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .border_b_1()
                            .border_color(rgba(theme.line))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgba(theme.fg0))
                            .child(title),
                    )
                    .child(
                        div()
                            .px_4()
                            .pt_3()
                            .text_size(ui_px(12.))
                            .text_color(rgba(theme.fg1))
                            .child(format!("{} · {}", label, destructive_copy)),
                    )
                    .when_some(self.delete_error.clone(), |dialog, error| {
                        dialog.child(
                            div()
                                .px_4()
                                .pt_2()
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
                                    "delete-confirm-cancel",
                                    i18n::text(self.language, "common.cancel"),
                                    theme,
                                )
                                .px_3()
                                .py_1()
                                .text_color(rgba(if busy { theme.fg2 } else { theme.fg0 }))
                                .when(busy, |button| button.cursor_default())
                                .when(!busy, |button| {
                                    button
                                        .cursor_pointer()
                                        .hover(|style| style.bg(rgba(theme.bg2)))
                                })
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.cancel_delete(cx);
                                }))
                                .child(i18n::text(self.language, "common.cancel")),
                            )
                            .child({
                                let busy = self.delete_busy;
                                semantic_button(
                                    "delete-confirm-submit",
                                    i18n::text(self.language, "menu.confirm_delete"),
                                    theme,
                                )
                                .px_3()
                                .py_1()
                                .bg(rgba(theme.red))
                                .text_color(rgba(theme.on_accent))
                                .cursor_pointer()
                                .when(!busy, |el| {
                                    el.hover(|style| {
                                        style.bg(rgba(Theme::with_alpha(theme.red, 0xcc)))
                                    })
                                    .active(|style| {
                                        style.bg(rgba(Theme::with_alpha(theme.red, 0x99)))
                                    })
                                })
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    if !this.delete_busy {
                                        this.confirm_delete(cx);
                                    }
                                }))
                                .child(if busy {
                                    i18n::text(self.language, "menu.deleting")
                                } else {
                                    i18n::text(self.language, "menu.confirm_delete")
                                })
                            }),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn cancel_project_create(&mut self, cx: &mut Context<Self>) {
        if self.project_add_busy {
            return;
        }
        self.pending_project_creation = None;
        cx.notify();
    }

    pub(crate) fn confirm_project_create(&mut self, cx: &mut Context<Self>) {
        if self.project_add_busy {
            return;
        }
        let Some(pending) = self.pending_project_creation.as_mut() else {
            return;
        };
        pending.error = None;
        match pending.target.clone() {
            ProjectCreationTarget::Local { path } => self.submit_local_project(path, true, cx),
            ProjectCreationTarget::Remote { host, path } => {
                self.submit_remote_project_with_create(host, path, true, cx)
            }
        }
    }

    pub(crate) fn render_project_create_confirm(
        &mut self,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::for_mode(self.theme_mode);
        let Some(pending) = self.pending_project_creation.clone() else {
            return div().into_any_element();
        };
        let path = match &pending.target {
            ProjectCreationTarget::Local { path } | ProjectCreationTarget::Remote { path, .. } => {
                path
            }
        };
        let busy = self.project_add_busy;
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(theme.overlay()))
            .on_any_mouse_down(
                cx.listener(|_this, _event: &gpui::MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .w(ui_px(460.))
                    .max_w(relative(0.92))
                    .bg(rgba(theme.bg1))
                    .border_1()
                    .border_color(rgba(theme.line))
                    .shadow_lg()
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .border_b_1()
                            .border_color(rgba(theme.line))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgba(theme.fg0))
                            .child(i18n::text(self.language, "confirm.create_directory_title")),
                    )
                    .child(
                        div()
                            .px_4()
                            .pt_3()
                            .text_size(ui_px(12.))
                            .text_color(rgba(theme.fg1))
                            .child(
                                i18n::text(self.language, "confirm.create_directory_copy")
                                    .replace("{path}", path),
                            ),
                    )
                    .when_some(pending.error, |dialog, error| {
                        dialog.child(
                            div()
                                .px_4()
                                .pt_2()
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
                                    "project-create-cancel",
                                    i18n::text(self.language, "common.cancel"),
                                    theme,
                                )
                                .px_3()
                                .py_1()
                                .text_color(rgba(theme.fg0))
                                .when(!busy, |button| {
                                    button
                                        .hover(|style| style.bg(rgba(theme.bg2)))
                                        .cursor_pointer()
                                })
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.cancel_project_create(cx);
                                }))
                                .child(i18n::text(self.language, "common.cancel")),
                            )
                            .child(
                                semantic_button(
                                    "project-create-submit",
                                    i18n::text(self.language, "confirm.create"),
                                    theme,
                                )
                                .px_3()
                                .py_1()
                                .bg(rgba(theme.accent))
                                .text_color(rgba(theme.on_accent))
                                .when(!busy, |button| {
                                    button.cursor_pointer().hover(|style| {
                                        style.bg(rgba(Theme::with_alpha(theme.accent, 0xcc)))
                                    })
                                })
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.confirm_project_create(cx);
                                }))
                                .child(if busy {
                                    i18n::text(self.language, "confirm.creating")
                                } else {
                                    i18n::text(self.language, "confirm.create")
                                }),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn render_bootstrap_confirm(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::for_mode(self.theme_mode);
        let Some(confirm) = self.bootstrap_confirm.clone() else {
            return div().into_any_element();
        };
        let action = if confirm.upgrade {
            i18n::text(self.language, "bootstrap.action.upgrade")
        } else if confirm.install {
            i18n::text(self.language, "bootstrap.action.install")
        } else {
            i18n::text(self.language, "bootstrap.action.start")
        };
        let description_key = if confirm.upgrade {
            "bootstrap.description.upgrade"
        } else if confirm.install {
            "bootstrap.description.install"
        } else {
            "bootstrap.description.start"
        };
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(theme.overlay()))
            .child(
                div()
                    .w(ui_px(480.))
                    .max_w(relative(0.92))
                    .bg(rgba(theme.bg1))
                    .border_1()
                    .border_color(rgba(theme.line))
                    .shadow_lg()
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .border_b_1()
                            .border_color(rgba(theme.line))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgba(theme.fg0))
                            .child(
                                i18n::text(self.language, "bootstrap.confirm_title")
                                    .replace("{action}", action),
                            ),
                    )
                    .child(
                        div()
                            .px_4()
                            .pt_3()
                            .text_size(ui_px(12.))
                            .text_color(rgba(theme.fg1))
                            .child(
                                i18n::text(self.language, description_key)
                                    .replace("{host}", &confirm.host),
                            ),
                    )
                    .when_some(self.bootstrap_error.clone(), |dialog, error| {
                        dialog.child(
                            div()
                                .px_4()
                                .pt_2()
                                .text_size(ui_px(11.))
                                .text_color(rgba(theme.red))
                                .child(error),
                        )
                    })
                    .when_some(
                        self.bootstrap_progress.get(&confirm.host).cloned(),
                        |dialog, progress| {
                            let overall = progress.phase.overall(progress.percent);
                            let phase_text = format_upload_phase(&progress, self.language);
                            dialog
                                .child(
                                    div()
                                        .px_4()
                                        .pt_3()
                                        .text_size(ui_px(11.))
                                        .text_color(rgba(theme.accent))
                                        .child(phase_text),
                                )
                                .child(
                                    div().mx_4().mt_2().h(ui_px(4.)).bg(rgba(theme.bg2)).child(
                                        div()
                                            .w(relative(overall as f32 / 100.0))
                                            .h_full()
                                            .bg(rgba(theme.accent)),
                                    ),
                                )
                        },
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .px_4()
                            .py_3()
                            .child(
                                semantic_button(
                                    "bootstrap-cancel",
                                    i18n::text(self.language, "common.cancel"),
                                    theme,
                                )
                                .px_3()
                                .py_1()
                                .text_color(rgba(theme.fg0))
                                .hover(|style| style.bg(rgba(theme.bg2)))
                                .on_click(cx.listener({
                                    let host = confirm.host.clone();
                                    move |this, _event, _window, cx| {
                                        this.cancel_bootstrap_for_host(&host, cx);
                                    }
                                }))
                                .child(i18n::text(self.language, "common.cancel")),
                            )
                            .child(
                                semantic_button("bootstrap-submit", action, theme)
                                    .px_3()
                                    .py_1()
                                    .bg(rgba(theme.accent))
                                    .text_color(rgba(theme.on_accent))
                                    .on_click(cx.listener(|this, _event, _window, cx| {
                                        this.confirm_bootstrap(cx)
                                    }))
                                    .child(action),
                            ),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_menu_position_flips_and_clamps_at_viewport_edges() {
        let viewport = gpui::size(ui_px(800.), ui_px(600.));
        let menu = gpui::size(ui_px(200.), ui_px(120.));
        assert_eq!(
            clamp_menu_position(Point::new(ui_px(100.), ui_px(100.)), viewport, menu),
            Point::new(ui_px(100.), ui_px(100.))
        );
        assert_eq!(
            clamp_menu_position(Point::new(ui_px(790.), ui_px(590.)), viewport, menu),
            Point::new(ui_px(590.), ui_px(470.))
        );
        assert_eq!(
            clamp_menu_position(Point::new(ui_px(-10.), ui_px(-20.)), viewport, menu),
            Point::new(ui_px(0.), ui_px(0.))
        );
    }

    #[test]
    fn dismissing_context_menus_clears_session_and_tree_menus() {
        let mut session_menu = Some(SessionMenu {
            agent: "agent-1".into(),
            position: Point::new(ui_px(10.), ui_px(20.)),
            remote: false,
        });
        let mut tree_menu = Some(TreeMenu {
            target: DeleteTarget::LocalProject {
                project: "project-1".into(),
                label: "demo".into(),
            },
            position: Point::new(ui_px(30.), ui_px(40.)),
        });

        assert!(dismiss_context_menus(&mut session_menu, &mut tree_menu));
        assert!(session_menu.is_none());
        assert!(tree_menu.is_none());
        assert!(!dismiss_context_menus(&mut session_menu, &mut tree_menu));
    }
}
