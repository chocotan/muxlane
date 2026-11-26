//! Zero-radius GPUI semantic palette and reusable UI primitives for Bot panels.
//! Strictly adheres to AGENTS.md: all elements use 0px corner radius.
use crate::theme::{Theme, ThemeMode};
use crate::ui_scale::px as ui_px;
use gpui::{
    div, prelude::*, rgba, Div, ElementId, IntoElement, ParentElement, SharedString, Stateful,
    Styled,
};

pub const CHAT_PROSE_PX: f32 = 13.5;
pub const CHAT_META_PX: f32 = 11.5;
pub const CHAT_MICRO_PX: f32 = 10.5;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ChatPalette {
    pub theme: Theme,
    pub canvas: u32,
    pub card: u32,
    pub inset: u32,
    pub user_surface: u32,
    pub fg_strong: u32,
    pub fg_prose: u32,
    pub fg_muted: u32,
    pub fg_faint: u32,
    pub hairline: u32,
    pub diff_del_fg: u32,
}

impl ChatPalette {
    pub fn for_mode(mode: ThemeMode) -> Self {
        let theme = Theme::for_mode(mode);
        let is_dark = mode.is_dark();
        let canvas = theme.bg0;
        let card = theme.bg1;
        let inset = theme.bg2;
        let user_surface = mix(theme.accent, card, if is_dark { 0.14 } else { 0.08 });
        let fg_strong = theme.fg0;
        let fg_prose = mix(theme.fg0, canvas, 0.93);
        let fg_muted = theme.fg1;
        let fg_faint = theme.fg2;
        let hairline = theme.line;
        let diff_del_fg = mix(theme.red, theme.fg0, if is_dark { 0.25 } else { 0.28 });

        Self {
            theme,
            canvas,
            card,
            inset,
            user_surface,
            fg_strong,
            fg_prose,
            fg_muted,
            fg_faint,
            hairline,
            diff_del_fg,
        }
    }
}

/// Mix `a` toward `b`: `t = 1.0` returns `a`, `t = 0.0` returns `b`.
pub(crate) fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let lerp = |shift: u32| -> u32 {
        let ca = ((a >> shift) & 0xff) as f32;
        let cb = ((b >> shift) & 0xff) as f32;
        ((ca * t + cb * (1.0 - t)).round() as u32).min(255)
    };
    (lerp(24) << 24) | (lerp(16) << 16) | (lerp(8) << 8) | 0xff
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ButtonVariant {
    Primary,
    Secondary,
    Outline,
    Danger,
}

pub(crate) fn square_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    variant: ButtonVariant,
    palette: ChatPalette,
) -> Stateful<Div> {
    let (bg, fg, border, hover_bg) = match variant {
        ButtonVariant::Primary => (
            palette.theme.accent,
            palette.theme.on_accent,
            palette.theme.accent,
            mix(palette.theme.accent, palette.fg_strong, 0.85),
        ),
        ButtonVariant::Secondary => (
            palette.inset,
            palette.fg_strong,
            palette.hairline,
            mix(palette.inset, palette.fg_strong, 0.90),
        ),
        ButtonVariant::Outline => (
            palette.card,
            palette.fg_muted,
            palette.hairline,
            palette.inset,
        ),
        ButtonVariant::Danger => (
            mix(palette.theme.red, palette.card, 0.14),
            palette.diff_del_fg,
            palette.theme.red,
            mix(palette.theme.red, palette.card, 0.24),
        ),
    };
    div()
        .id(id.into())
        .h(ui_px(26.))
        .px_2p5()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap_1()
        .bg(rgba(bg))
        .text_color(rgba(fg))
        .border_1()
        .border_color(rgba(border))
        .text_size(ui_px(CHAT_META_PX))
        .font_weight(gpui::FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(move |s| s.bg(rgba(hover_bg)).text_color(rgba(fg)))
        .child(label.into())
}

pub(crate) fn square_badge(label: impl Into<SharedString>, fg: u32, bg: u32, border: u32) -> Div {
    div()
        .px_1p5()
        .py_0p5()
        .flex_none()
        .bg(rgba(bg))
        .border_1()
        .border_color(rgba(border))
        .text_size(ui_px(CHAT_MICRO_PX))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgba(fg))
        .child(label.into())
}

pub(crate) fn rail_card(
    palette: ChatPalette,
    left_rail: Option<u32>,
    body: impl IntoElement,
) -> Div {
    let mut card = div()
        .w_full()
        .flex()
        .bg(rgba(palette.card))
        .border_1()
        .border_color(rgba(palette.hairline));
    if let Some(rail) = left_rail {
        card = card.child(
            div()
                .w(ui_px(2.5))
                .flex_none()
                .self_stretch()
                .bg(rgba(rail)),
        );
    }
    card.child(div().flex_1().min_w_0().child(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn srgb_luminance(color: u32) -> f64 {
        let ch = |shift: u32| {
            let v = f64::from((color >> shift) & 0xff) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(24) + 0.7152 * ch(16) + 0.0722 * ch(8)
    }

    fn contrast(fg: u32, bg: u32) -> f64 {
        let a = srgb_luminance(fg);
        let b = srgb_luminance(bg);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn chat_palette_preserves_contrast_across_all_ten_themes() {
        for mode in ThemeMode::ALL {
            let p = ChatPalette::for_mode(mode);
            assert!(
                contrast(p.fg_prose, p.canvas) >= 4.5,
                "{} fg_prose contrast",
                mode.id()
            );
            assert!(
                contrast(p.fg_muted, p.card) >= 4.5,
                "{} fg_muted contrast",
                mode.id()
            );
        }
    }
}
