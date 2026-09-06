use crate::i18n::{self, Language};
use crate::theme::Theme;
use crate::ui_scale::px as ui_px;
use gpui::{
    div, prelude::*, rgba, Animation, AnimationExt, Context, ElementId, Pixels, Point, Render,
    Role, SharedString, Window,
};

use std::time::Duration;
pub(crate) struct DragGhost {
    pub(crate) label: SharedString,
    pub(crate) offset: Point<Pixels>,
    pub(crate) theme: Theme,
}
impl Render for DragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .pl(self.offset.x.max(ui_px(0.0)))
            .pt(self.offset.y.max(ui_px(0.0)))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .bg(rgba(self.theme.bg2))
                    .border_1()
                    .border_color(rgba(self.theme.line))
                    .text_size(ui_px(11.))
                    .text_color(rgba(self.theme.fg0))
                    .child(self.label.clone()),
            )
    }
}

pub(crate) struct DividerDragGhost;

impl Render for DividerDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().w(ui_px(1.)).h(ui_px(1.))
    }
}

pub(crate) fn semantic_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    theme: Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .focusable()
        .tab_stop(true)
        .cursor_pointer()
        .role(Role::Button)
        .aria_label(label)
        .focus_visible(|style| style.border_1().border_color(rgba(theme.accent)))
        .active(|style| style.bg(rgba(theme.bg3)))
}

struct HoverTip {
    text: SharedString,
}

impl Render for HoverTip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .bg(rgba(0x1a1d24f2))
            .border_1()
            .border_color(rgba(0x00000066))
            .text_size(ui_px(11.))
            .text_color(rgba(0xffffffff))
            .child(self.text.clone())
    }
}

pub(crate) fn hover_tip(
    text: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut gpui::App) -> gpui::AnyView {
    let text = text.into();
    move |_, cx| cx.new(|_| HoverTip { text: text.clone() }).into()
}
pub(crate) fn format_relative_time(then: u64, lang: Language) -> String {
    let now = muxlane_core::model::now_secs();
    let diff = now.saturating_sub(then);
    if diff < 10 {
        i18n::text(lang, "relative.just_now").to_string()
    } else if diff < 60 {
        i18n::text(lang, "relative.seconds_ago").replace("{count}", &diff.to_string())
    } else if diff < 3600 {
        i18n::text(lang, "relative.minutes_ago").replace("{count}", &(diff / 60).to_string())
    } else if diff < 86400 {
        i18n::text(lang, "relative.hours_ago").replace("{count}", &(diff / 3600).to_string())
    } else {
        i18n::text(lang, "relative.days_ago").replace("{count}", &(diff / 86400).to_string())
    }
}

fn render_pi_loading_spinner_frame(frame: usize, theme: Theme) -> gpui::Div {
    let empty_index = frame % 8;
    let render_col = |indices: [usize; 4]| {
        let mut col = div().flex().flex_col().gap(ui_px(1.5));
        for index in indices {
            col = col.child(div().w(ui_px(2.5)).h(ui_px(2.5)).bg(rgba(Theme::with_alpha(
                theme.accent,
                if index == empty_index { 0x25 } else { 0xff },
            ))));
        }
        col
    };

    div()
        .flex()
        .gap(ui_px(2.5))
        .items_center()
        .justify_center()
        .child(render_col([0, 7, 6, 5]))
        .child(render_col([1, 2, 3, 4]))
}

fn render_pi_loading_spinner(animation_id: impl Into<ElementId>, theme: Theme) -> impl IntoElement {
    div()
        .w(ui_px(14.))
        .h(ui_px(14.))
        .flex()
        .items_center()
        .justify_center()
        .with_animation(
            animation_id,
            Animation::new(Duration::from_millis(800)).repeat(),
            move |container, delta| {
                let frame = ((delta * 8.0).floor() as usize).min(7);
                container.child(render_pi_loading_spinner_frame(frame, theme))
            },
        )
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AttentionStyle {
    pub(crate) bg_color: Option<u32>,
    pub(crate) text_color: Option<u32>,
    pub(crate) border_color: Option<u32>,
    pub(crate) is_alerting: bool,
}

pub(crate) fn compute_attention_style(
    status: muxlane_core::model::AgentStatus,
    seen: bool,
    theme: Theme,
) -> AttentionStyle {
    match status {
        muxlane_core::model::AgentStatus::Blocked => {
            let base_color = theme.yellow;
            AttentionStyle {
                bg_color: Some(Theme::with_alpha(base_color, 0x22)),
                text_color: Some(base_color),
                border_color: Some(Theme::with_alpha(base_color, 0xa0)),
                is_alerting: true,
            }
        }
        muxlane_core::model::AgentStatus::Done if !seen => {
            let base_color = theme.green;
            AttentionStyle {
                bg_color: Some(Theme::with_alpha(base_color, 0x1e)),
                text_color: Some(base_color),
                border_color: Some(Theme::with_alpha(base_color, 0x70)),
                is_alerting: true,
            }
        }
        muxlane_core::model::AgentStatus::Failed if !seen => {
            let base_color = theme.red;
            AttentionStyle {
                bg_color: Some(Theme::with_alpha(base_color, 0x1e)),
                text_color: Some(base_color),
                border_color: Some(Theme::with_alpha(base_color, 0x70)),
                is_alerting: true,
            }
        }
        _ => AttentionStyle::default(),
    }
}

pub(crate) fn render_status_indicator(
    status: muxlane_core::model::AgentStatus,
    animation_id: impl Into<ElementId>,
    theme: Theme,
) -> gpui::Div {
    let container = div()
        .w(ui_px(14.))
        .h(ui_px(14.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center();

    match status {
        muxlane_core::model::AgentStatus::Working => {
            container.child(render_pi_loading_spinner(animation_id, theme))
        }
        muxlane_core::model::AgentStatus::Blocked => {
            container.child(div().w(ui_px(6.)).h(ui_px(6.)).bg(rgba(theme.yellow)))
        }
        muxlane_core::model::AgentStatus::Done => {
            container.child(div().w(ui_px(6.)).h(ui_px(6.)).bg(rgba(theme.green)))
        }
        muxlane_core::model::AgentStatus::Failed => {
            container.child(div().w(ui_px(6.)).h(ui_px(6.)).bg(rgba(theme.red)))
        }
        muxlane_core::model::AgentStatus::Idle | muxlane_core::model::AgentStatus::Unknown => {
            container.child(div().w(ui_px(5.)).h(ui_px(5.)).bg(rgba(theme.fg2)))
        }
    }
}

pub(crate) fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    } else {
        s.to_string()
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// 上传阶段进度文本：如「上传二进制 12.3 MB / 45.6 MB (27%)」；
/// 无字节数时退化为「上传二进制 27%」/「上传二进制…」
pub(crate) fn format_upload_phase(
    progress: &muxlane_client::BootstrapProgress,
    language: Language,
) -> String {
    let label = i18n::text(
        language,
        match progress.phase {
            muxlane_client::BootstrapPhase::Upload => "bootstrap.phase.upload",
            muxlane_client::BootstrapPhase::Install => "bootstrap.phase.install",
            muxlane_client::BootstrapPhase::Restart => "bootstrap.phase.restart",
        },
    );
    match (progress.done_bytes, progress.total_bytes, progress.percent) {
        (Some(done), Some(total), Some(percent)) if total > 0 => {
            format!(
                "{label} {} / {} ({percent}%)",
                format_bytes(done),
                format_bytes(total)
            )
        }
        (_, _, Some(percent)) => format!("{label} {percent}%"),
        _ => format!("{label}…"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_progress_text_shows_bytes_and_percent() {
        use muxlane_client::BootstrapPhase;
        let progress = muxlane_client::BootstrapProgress {
            phase: BootstrapPhase::Upload,
            percent: Some(27),
            done_bytes: Some(12 * 1024 * 1024 + 300 * 1024),
            total_bytes: Some(45 * 1024 * 1024),
        };
        let text = format_upload_phase(&progress, Language::English);
        assert!(text.contains("12.3 MB"), "{text}");
        assert!(text.contains("45.0 MB"), "{text}");
        assert!(text.contains("27%"), "{text}");
        // 无字节数时退化为纯百分比
        let text = format_upload_phase(
            &muxlane_client::BootstrapProgress {
                phase: BootstrapPhase::Install,
                percent: Some(50),
                done_bytes: None,
                total_bytes: None,
            },
            Language::English,
        );
        assert_eq!(text, "Installing 50%");
        // 无细分进度
        let text = format_upload_phase(
            &muxlane_client::BootstrapProgress {
                phase: BootstrapPhase::Restart,
                percent: None,
                done_bytes: None,
                total_bytes: None,
            },
            Language::English,
        );
        assert_eq!(text, "Restarting Service…");
    }
}
