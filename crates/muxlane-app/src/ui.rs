//! Zero-radius GPUI semantic palette and reusable UI primitives for Native Chat and Bot panels.
//! Strictly adheres to AGENTS.md: all elements use 0px corner radius.
use crate::theme::{Theme, ThemeMode};
use crate::ui_scale::px as ui_px;
use gpui::{
    div, prelude::*, rgba, Div, ElementId, IntoElement, ParentElement, SharedString, Stateful,
    Styled,
};

pub const CHAT_PROSE_PX: f32 = 13.5;
pub const CHAT_CODE_PX: f32 = 12.5;
pub const CHAT_META_PX: f32 = 11.5;
pub const CHAT_MICRO_PX: f32 = 10.5;
pub const CHAT_MEASURE_MAX_PX: f32 = 760.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ChatPalette {
    pub theme: Theme,
    pub canvas: u32,
    pub card: u32,
    pub inset: u32,
    pub user_surface: u32,
    pub code_surface: u32,
    pub inline_code_surface: u32,
    pub row_hover: u32,
    pub fg_strong: u32,
    pub fg_prose: u32,
    pub fg_muted: u32,
    pub fg_faint: u32,
    pub hairline: u32,
    pub subtle_line: u32,
    pub security_surface: u32,
    pub security_border: u32,
    pub diff_add_bg: u32,
    pub diff_add_fg: u32,
    pub diff_del_bg: u32,
    pub diff_del_fg: u32,
    pub syn_keyword: u32,
    pub syn_string: u32,
    pub syn_comment: u32,
    pub syn_number: u32,
    pub syn_type: u32,
}

impl ChatPalette {
    pub fn for_mode(mode: ThemeMode) -> Self {
        let theme = Theme::for_mode(mode);
        let is_dark = mode.is_dark();
        let canvas = theme.bg0;
        let card = theme.bg1;
        let inset = theme.bg2;
        let user_surface = mix(theme.accent, card, if is_dark { 0.14 } else { 0.08 });
        let code_surface = mix(inset, card, 0.55);
        let inline_code_surface = inset;
        let row_hover = Theme::with_alpha(theme.accent, 0x1e);
        let fg_strong = theme.fg0;
        let fg_prose = mix(theme.fg0, canvas, 0.93);
        let fg_muted = theme.fg1;
        let fg_faint = theme.fg2;
        let hairline = theme.line;
        let subtle_line = mix(theme.line, canvas, 0.55);
        let security_surface = mix(theme.yellow, card, if is_dark { 0.16 } else { 0.10 });
        let security_border = theme.yellow;
        let diff_add_bg = mix(theme.green, canvas, if is_dark { 0.12 } else { 0.08 });
        let diff_add_fg = mix(theme.green, theme.fg0, if is_dark { 0.30 } else { 0.28 });
        let diff_del_bg = mix(theme.red, canvas, if is_dark { 0.12 } else { 0.08 });
        let diff_del_fg = mix(theme.red, theme.fg0, if is_dark { 0.25 } else { 0.28 });

        let (syn_keyword, syn_string, syn_comment, syn_number, syn_type) = if is_dark {
            (
                mix(theme.accent, 0xe2e8f0ff, 0.72),
                mix(theme.green, 0xdcfce7ff, 0.78),
                theme.fg2,
                mix(theme.yellow, 0xfef3c7ff, 0.75),
                mix(theme.accent, 0xbae6fdff, 0.55),
            )
        } else {
            (
                theme.accent,
                mix(theme.green, theme.fg0, 0.75),
                theme.fg2,
                mix(theme.yellow, theme.fg0, 0.80),
                mix(theme.accent, theme.fg0, 0.60),
            )
        };

        Self {
            theme,
            canvas,
            card,
            inset,
            user_surface,
            code_surface,
            inline_code_surface,
            row_hover,
            fg_strong,
            fg_prose,
            fg_muted,
            fg_faint,
            hairline,
            subtle_line,
            security_surface,
            security_border,
            diff_add_bg,
            diff_add_fg,
            diff_del_bg,
            diff_del_fg,
            syn_keyword,
            syn_string,
            syn_comment,
            syn_number,
            syn_type,
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
    Ghost,
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
        ButtonVariant::Ghost => (0x00000000, palette.fg_muted, 0x00000000, palette.inset),
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

pub(crate) fn shortcut_chip(keys: impl Into<SharedString>, palette: ChatPalette) -> Div {
    div()
        .px_1()
        .py_0p5()
        .bg(rgba(palette.inset))
        .border_1()
        .border_color(rgba(palette.hairline))
        .text_size(ui_px(10.0))
        .text_color(rgba(palette.fg_faint))
        .child(keys.into())
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

pub(crate) fn disclosure_chevron(expanded: bool, palette: ChatPalette) -> Div {
    div()
        .w(ui_px(14.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .text_size(ui_px(CHAT_MICRO_PX))
        .text_color(rgba(palette.fg_faint))
        .child(if expanded { "▾" } else { "▸" })
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
            assert!(
                contrast(p.diff_add_fg, p.diff_add_bg) >= 4.5,
                "{} diff_add contrast",
                mode.id()
            );
            assert!(
                contrast(p.diff_del_fg, p.diff_del_bg) >= 4.5,
                "{} diff_del contrast",
                mode.id()
            );
        }
    }
}
