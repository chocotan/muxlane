//! Notification center state, policy, rendering, and toast lifecycle.
use crate::ui_scale::px as ui_px;

use crate::i18n::{self, Language};
use crate::icons::{panel_icon, NOTIFICATION_ICON};
use crate::sound::{self, SoundKind};
use crate::theme::{Theme, ThemeMode};
use crate::widgets::{format_relative_time, truncate};
use gpui::{
    div, prelude::*, rgba, Context, EventEmitter, MouseButton, ParentElement, Render, Styled, Task,
    Window,
};
use muxlane_core::model::{AgentId, AgentStatus, AgentType};
use std::collections::HashSet;
use std::time::Instant;

pub(crate) struct NotificationDraft {
    pub(crate) agent: AgentId,
    pub(crate) machine_name: String,
    pub(crate) project_name: String,
    pub(crate) agent_type: AgentType,
    pub(crate) focused: bool,
    pub(crate) from: AgentStatus,
    pub(crate) to: AgentStatus,
    pub(crate) message: Option<String>,
    pub(crate) sound_enabled: bool,
    pub(crate) desktop_enabled: bool,
}

pub(crate) enum NotificationCenterEvent {
    JumpToAgent(AgentId),
}

#[derive(Clone)]
pub struct Notification {
    pub agent: AgentId,
    pub machine_name: String,
    pub project_name: String,
    pub _agent_type: AgentType,
    pub to: AgentStatus,
    pub message: Option<String>,
    pub unread: bool,
    pub time_secs: u64,
}

#[derive(Clone)]
pub struct ToastNotification {
    pub id: u64,
    pub agent: AgentId,
    pub title: String,
    pub message: String,
    pub status: AgentStatus,
    pub created_at: Instant,
}

pub(crate) struct NotificationCenter {
    notifications: Vec<Notification>,
    toasts: Vec<ToastNotification>,
    toast_seq: u64,
    error_toast: Option<(String, Instant, bool)>,
    open: bool,
    theme_mode: ThemeMode,
    language: Language,
    _animation_task: Task<()>,
}

impl EventEmitter<NotificationCenterEvent> for NotificationCenter {}

impl NotificationCenter {
    pub(crate) fn new(theme_mode: ThemeMode, language: Language, cx: &mut Context<Self>) -> Self {
        let animation_task = cx.spawn(async move |this, cx| loop {
            let has_activity = match this.update(cx, |this, _| this.has_activity()) {
                Ok(has_activity) => has_activity,
                Err(_) => break,
            };
            cx.background_executor()
                .timer(if has_activity {
                    std::time::Duration::from_millis(100)
                } else {
                    std::time::Duration::from_millis(250)
                })
                .await;
            if !has_activity {
                continue;
            }
            if this
                .update(cx, |this, cx| {
                    this.toasts
                        .retain(|toast| toast.created_at.elapsed().as_secs() < 6);
                    if this
                        .error_toast
                        .as_ref()
                        .is_some_and(|(_, created, _)| created.elapsed().as_secs() >= 8)
                    {
                        this.error_toast = None;
                    }
                    cx.notify();
                })
                .is_err()
            {
                break;
            }
        });

        Self {
            notifications: Vec::new(),
            toasts: Vec::new(),
            toast_seq: 0,
            error_toast: None,
            open: false,
            theme_mode,
            language,
            _animation_task: animation_task,
        }
    }

    pub(crate) fn push_notification(&mut self, draft: NotificationDraft, cx: &mut Context<Self>) {
        let body = effective_notification_body(draft.to, draft.message, self.language);
        let now_secs = muxlane_core::model::now_secs();

        if draft.from == draft.to {
            if let Some(existing) = self
                .notifications
                .iter_mut()
                .find(|item| item.agent == draft.agent && item.to == draft.to)
            {
                existing.message = Some(body);
                existing.unread = !draft.focused;
                existing.time_secs = now_secs;
                cx.notify();
            }
            return;
        }
        if !matches!(
            draft.to,
            AgentStatus::Blocked | AgentStatus::Done | AgentStatus::Failed
        ) {
            return;
        }

        self.toast_seq += 1;
        self.notifications.insert(
            0,
            Notification {
                agent: draft.agent.clone(),
                machine_name: draft.machine_name.clone(),
                project_name: draft.project_name.clone(),
                _agent_type: draft.agent_type,
                to: draft.to,
                message: Some(body.clone()),
                unread: !draft.focused,
                time_secs: now_secs,
            },
        );
        if self.notifications.len() > 50 {
            self.notifications.truncate(50);
        }

        let toast_title = match draft.to {
            AgentStatus::Blocked => i18n::text(self.language, "notification.title_input")
                .replace("{machine}", &draft.machine_name)
                .replace("{project}", &draft.project_name),
            AgentStatus::Done => i18n::text(self.language, "notification.title_done")
                .replace("{machine}", &draft.machine_name)
                .replace("{project}", &draft.project_name),
            AgentStatus::Failed => i18n::text(self.language, "notification.title_failed")
                .replace("{machine}", &draft.machine_name)
                .replace("{project}", &draft.project_name),
            _ => format!("{} · {}", draft.machine_name, draft.project_name),
        };

        if draft.focused {
            if draft.desktop_enabled {
                sound::send_desktop_notification(&toast_title, &body);
            }
            cx.notify();
            return;
        }

        self.toasts.insert(
            0,
            ToastNotification {
                id: self.toast_seq,
                agent: draft.agent,
                title: toast_title.clone(),
                message: body.clone(),
                status: draft.to,
                created_at: Instant::now(),
            },
        );
        if self.toasts.len() > 3 {
            self.toasts.truncate(3);
        }

        if draft.sound_enabled {
            match draft.to {
                AgentStatus::Blocked => sound::play_sound(SoundKind::Request),
                AgentStatus::Done | AgentStatus::Failed => sound::play_sound(SoundKind::Done),
                _ => {}
            }
        }
        if draft.desktop_enabled {
            sound::send_desktop_notification(&toast_title, &body);
        }
        cx.notify();
    }

    pub(crate) fn set_appearance(
        &mut self,
        theme_mode: ThemeMode,
        language: Language,
        cx: &mut Context<Self>,
    ) {
        self.theme_mode = theme_mode;
        self.language = language;
        cx.notify();
    }

    pub(crate) fn summary(&self) -> (usize, bool, bool) {
        let unread = self.notifications.iter().filter(|item| item.unread).count();
        let blocked = self
            .notifications
            .iter()
            .any(|item| item.unread && item.to == AgentStatus::Blocked);
        (unread, blocked, self.open)
    }

    pub(crate) fn has_activity(&self) -> bool {
        !self.toasts.is_empty() || self.error_toast.is_some()
    }

    pub(crate) fn toggle_open(&mut self, cx: &mut Context<Self>) {
        self.open = !self.open;
        cx.notify();
    }

    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            cx.notify();
        }
    }

    pub(crate) fn mark_agent_read(&mut self, agent: &AgentId, cx: &mut Context<Self>) {
        self.toasts.retain(|toast| &toast.agent != agent);
        for notification in self
            .notifications
            .iter_mut()
            .filter(|notification| &notification.agent == agent)
        {
            notification.unread = false;
        }
        cx.notify();
    }

    pub(crate) fn remove_agent(&mut self, agent: &AgentId, cx: &mut Context<Self>) {
        self.toasts.retain(|toast| &toast.agent != agent);
        self.notifications
            .retain(|notification| &notification.agent != agent);
        cx.notify();
    }

    pub(crate) fn remove_agents(&mut self, agents: &HashSet<AgentId>, cx: &mut Context<Self>) {
        self.toasts.retain(|toast| !agents.contains(&toast.agent));
        self.notifications
            .retain(|notification| !agents.contains(&notification.agent));
        cx.notify();
    }

    pub(crate) fn clear(&mut self, cx: &mut Context<Self>) {
        self.notifications.clear();
        cx.notify();
    }

    pub(crate) fn show_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.error_toast = Some((message, Instant::now(), true));
        cx.notify();
    }

    fn render_notifications_popover(
        &mut self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let unread_count = self.notifications.iter().filter(|n| n.unread).count();

        div()
            .id("notifications-backdrop")
            .absolute()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event, _window, cx| this.close(cx)),
            )
            .child(
                div()
                    .id("notifications-popover")
                    .occlude()
                    .absolute()
                    .bottom(ui_px(40.))
                    .left(ui_px(8.))
                    .w(ui_px(320.))
                    .max_h(ui_px(420.))
                    .flex()
                    .flex_col()
                    .bg(rgba(theme.bg1))
                    .border_1()
                    .border_color(rgba(theme.line))
                    .shadow_xl()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_this, _event, _window, cx| cx.stop_propagation()),
                    )
                    .child(
                        div()
                            .h(ui_px(34.))
                            .px_3()
                            .flex()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(rgba(theme.line))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .child(panel_icon(NOTIFICATION_ICON, theme.fg1))
                                    .child(
                                        div()
                                            .text_size(ui_px(12.))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgba(theme.fg0))
                                            .child(i18n::text(
                                                self.language,
                                                "notification.center",
                                            )),
                                    )
                                    .when(unread_count > 0, |header| {
                                        header.child(
                                            div()
                                                .px_1p5()
                                                .py(ui_px(1.))
                                                .bg(rgba(theme.accent))
                                                .text_color(rgba(theme.on_accent))
                                                .text_size(ui_px(9.))
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .child(format!("{unread_count}")),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .when(!self.notifications.is_empty(), |el| {
                                        el.child(
                                            div()
                                                .id("clear-notifications")
                                                .cursor_pointer()
                                                .px_1p5()
                                                .py_0p5()
                                                .text_size(ui_px(10.))
                                                .text_color(rgba(theme.fg2))
                                                .hover(|s| {
                                                    s.bg(rgba(theme.bg2))
                                                        .text_color(rgba(theme.fg0))
                                                })
                                                .on_click(cx.listener(|this, _ev, _window, cx| {
                                                    this.clear(cx)
                                                }))
                                                .child(i18n::text(
                                                    self.language,
                                                    "notification.clear",
                                                )),
                                        )
                                    })
                                    .child(
                                        div()
                                            .id("close-notifications")
                                            .cursor_pointer()
                                            .px_1()
                                            .text_size(ui_px(14.))
                                            .text_color(rgba(theme.fg2))
                                            .hover(|s| s.text_color(rgba(theme.fg0)))
                                            .on_click(
                                                cx.listener(|this, _ev, _window, cx| {
                                                    this.close(cx)
                                                }),
                                            )
                                            .child("×"),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .id("notifications-scroll")
                            .flex_1()
                            .overflow_y_scroll()
                            .when(self.notifications.is_empty(), |list| {
                                list.child(
                                    div()
                                        .py_8()
                                        .flex()
                                        .flex_col()
                                        .items_center()
                                        .justify_center()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_size(ui_px(11.))
                                                .text_color(rgba(theme.fg2))
                                                .child(i18n::text(
                                                    self.language,
                                                    "notification.empty",
                                                )),
                                        ),
                                )
                            })
                            .children(self.notifications.iter().enumerate().map(|(idx, n)| {
                                let dot_color = match n.to {
                                    AgentStatus::Blocked => theme.yellow,
                                    AgentStatus::Done => theme.green,
                                    AgentStatus::Failed => theme.red,
                                    AgentStatus::Working => theme.accent,
                                    _ => theme.fg2,
                                };
                                let status_text = match n.to {
                                    AgentStatus::Blocked => {
                                        i18n::text(self.language, "status.input_required")
                                    }
                                    AgentStatus::Done => {
                                        i18n::text(self.language, "status.task_completed")
                                    }
                                    AgentStatus::Failed => {
                                        i18n::text(self.language, "status.failed")
                                    }
                                    AgentStatus::Working => {
                                        i18n::text(self.language, "status.working")
                                    }
                                    _ => i18n::text(self.language, "status.idle"),
                                };
                                let agent_id = n.agent.clone();
                                let is_unread = n.unread;
                                let time_str = format_relative_time(n.time_secs, self.language);

                                div()
                                    .id(gpui::ElementId::Name(
                                        format!("notif-popover-item-{idx}").into(),
                                    ))
                                    .when(cfg!(test), |el| {
                                        el.debug_selector(move || {
                                            format!("notif-popover-item-{idx}")
                                        })
                                    })
                                    .relative()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .px_3()
                                    .py_2()
                                    .border_b_1()
                                    .border_color(rgba(theme.line))
                                    .when(is_unread, |el| {
                                        el.bg(rgba(Theme::with_alpha(theme.accent, 0x0f)))
                                    })
                                    .hover(|s| s.bg(rgba(theme.bg2)))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                                        this.open = false;
                                        cx.emit(NotificationCenterEvent::JumpToAgent(
                                            agent_id.clone(),
                                        ));
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap_1p5()
                                                    .child(
                                                        div()
                                                            .w(ui_px(6.))
                                                            .h(ui_px(6.))
                                                            .bg(rgba(dot_color)),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(ui_px(11.))
                                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                                            .text_color(rgba(theme.fg0))
                                                            .child(format!(
                                                                "{} · {}",
                                                                n.machine_name, n.project_name
                                                            )),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(ui_px(10.))
                                                            .text_color(rgba(dot_color))
                                                            .child(status_text),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_size(ui_px(10.))
                                                    .text_color(rgba(theme.fg2))
                                                    .child(time_str),
                                            ),
                                    )
                                    .when_some(n.message.clone(), |row, msg| {
                                        row.child(
                                            div()
                                                .text_size(ui_px(11.))
                                                .text_color(rgba(if is_unread {
                                                    theme.fg0
                                                } else {
                                                    theme.fg1
                                                }))
                                                .child(truncate(&msg, 90)),
                                        )
                                    })
                            })),
                    ),
            )
            .into_any_element()
    }
}

impl Render for NotificationCenter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_mode(self.theme_mode);
        let mut root = div().absolute().size_full();

        if !self.toasts.is_empty() {
            root = root.child(
                div()
                    .id("toast-overlay")
                    .absolute()
                    .bottom(ui_px(16.))
                    .right(ui_px(16.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .w(ui_px(320.))
                    .children(self.toasts.iter().map(|toast| {
                        let dot_color = match toast.status {
                            AgentStatus::Blocked => theme.yellow,
                            AgentStatus::Done => theme.green,
                            AgentStatus::Failed => theme.red,
                            AgentStatus::Working => theme.accent,
                            AgentStatus::Idle | AgentStatus::Unknown => theme.fg2,
                        };
                        let agent_id = toast.agent.clone();
                        let toast_id = toast.id;
                        div()
                            .id(gpui::ElementId::Name(format!("toast-{toast_id}").into()))
                            .when(cfg!(test), |el| {
                                el.debug_selector(move || format!("toast-{toast_id}"))
                            })
                            .relative()
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            .p_3()
                            .pl_4()
                            .bg(rgba(theme.bg2))
                            .border_1()
                            .border_color(rgba(theme.line))
                            .shadow_lg()
                            .cursor_pointer()
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .bottom_0()
                                    .w(ui_px(4.))
                                    .bg(rgba(dot_color)),
                            )
                            .on_click(cx.listener(move |_this, _ev, _window, cx| {
                                cx.emit(NotificationCenterEvent::JumpToAgent(agent_id.clone()));
                            }))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1p5()
                                            .child(
                                                div().w(ui_px(7.)).h(ui_px(7.)).bg(rgba(dot_color)),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .text_size(ui_px(12.))
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .text_color(rgba(theme.fg0))
                                                    .child(toast.title.clone()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id(gpui::ElementId::Name(
                                                format!("toast-close-{toast_id}").into(),
                                            ))
                                            .text_size(ui_px(11.))
                                            .text_color(rgba(theme.fg2))
                                            .hover(|s| s.text_color(rgba(theme.fg0)))
                                            .on_click(cx.listener(move |this, _ev, _window, cx| {
                                                cx.stop_propagation();
                                                this.toasts.retain(|item| item.id != toast_id);
                                                cx.notify();
                                            }))
                                            .child("×"),
                                    ),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .text_size(ui_px(11.5))
                                    .text_color(rgba(theme.fg1))
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(truncate(&toast.message, 120)),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .text_size(ui_px(9.5))
                                    .text_color(rgba(theme.fg2))
                                    .child(i18n::text(self.language, "notification.click_to_open")),
                            )
                    })),
            );
        }

        if let Some((message, _, is_error)) = self.error_toast.clone() {
            let color = if is_error { theme.red } else { theme.fg1 };
            root = root.child(
                div()
                    .id("error-toast")
                    .absolute()
                    .top(ui_px(16.))
                    .right(ui_px(16.))
                    .w(ui_px(420.))
                    .p_3()
                    .bg(rgba(theme.bg1))
                    .border_1()
                    .border_color(rgba(color))
                    .text_size(ui_px(11.5))
                    .text_color(rgba(color))
                    .child(message),
            );
        }

        if self.open {
            root = root.child(self.render_notifications_popover(theme, cx));
        }
        root
    }
}

fn effective_notification_body(
    status: AgentStatus,
    message: Option<String>,
    language: Language,
) -> String {
    let normalized = message
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        return match status {
            AgentStatus::Blocked => i18n::text(language, "status.input_required").into(),
            AgentStatus::Done => i18n::text(language, "status.task_completed_body").into(),
            AgentStatus::Failed => i18n::text(language, "status.failed").into(),
            _ => status.as_str().into(),
        };
    }
    let mut chars = normalized.chars();
    let body: String = chars.by_ref().take(180).collect();
    if chars.next().is_some() {
        format!("{body}…")
    } else {
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_body_preserves_content_and_has_status_fallbacks() {
        assert_eq!(
            effective_notification_body(
                AgentStatus::Done,
                Some("  Fixed the issue\nand passed tests  ".into()),
                Language::English,
            ),
            "Fixed the issue and passed tests"
        );
        assert_eq!(
            effective_notification_body(AgentStatus::Done, Some("  ".into()), Language::Chinese),
            i18n::text(Language::Chinese, "status.task_completed_body")
        );
        assert_eq!(
            effective_notification_body(AgentStatus::Blocked, None, Language::English),
            i18n::text(Language::English, "status.input_required")
        );
    }
}
