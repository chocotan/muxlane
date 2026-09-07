//! ACP 对话视图的排版 / 间距常量与基础视觉原语。
//!
//! 所有数值都经 `ui_px` 缩放；颜色只来自 `Theme` token。

use crate::theme::Theme;
use crate::ui_scale::px as ui_px;
use crate::widgets::semantic_button;
use gpui::{div, prelude::*, rgba, ElementId, SharedString};

/// 正文字号（assistant 与用户消息同字号，比例字体）。
pub(super) const BODY_SIZE: f32 = 13.;
/// 正文行高。
pub(super) const BODY_LINE: f32 = 20.;
/// 元信息 / 标签 / 描述字号，也是全局最小字号。
pub(super) const META_SIZE: f32 = 11.;
/// 按钮文字字号。
pub(super) const BUTTON_SIZE: f32 = 12.;
/// 等宽字号：仅用于代码块 / diff / terminal / raw input-output。
pub(super) const CODE_SIZE: f32 = 12.;
/// 等宽行高。
pub(super) const CODE_LINE: f32 = 18.;

/// 所有正文文字左沿对齐位置。
pub(super) const CONTENT_INSET: f32 = 24.;
/// 卡片外边距；`CARD_MARGIN + CARD_PAD == CONTENT_INSET`。
pub(super) const CARD_MARGIN: f32 = 12.;
/// 卡片内边距。
pub(super) const CARD_PAD: f32 = 12.;
/// 带 2px 左色条的卡片内边距，保证文字仍落在 `CONTENT_INSET`。
pub(super) const BAR_PAD: f32 = CARD_PAD - 2.;

/// 折叠行 / 生成中指示器的左侧留白与字形槽宽：`GUTTER_PAD + GUTTER_WIDTH == CONTENT_INSET`。
pub(super) const GUTTER_PAD: f32 = 8.;
pub(super) const GUTTER_WIDTH: f32 = 16.;
/// 折叠体嵌套竖线的 x 位置；竖线 1px + `NEST_PAD` 后文字回到 `CONTENT_INSET`。
pub(super) const NEST_LINE_X: f32 = 11.;
pub(super) const NEST_PAD: f32 = CONTENT_INSET - NEST_LINE_X - 1.;

/// 图标按钮统一尺寸。
pub(super) const ICON_BUTTON: f32 = 28.;
/// 语义指示方块尺寸。
pub(super) const INDICATOR: f32 = 6.;
/// 弹层相对触发按钮的纵向偏移。
pub(super) const POPUP_OFFSET: f32 = 24.;
/// 补全弹窗最大高度。
pub(super) const POPUP_MAX_HEIGHT: f32 = 220.;
/// 折叠体 / 图片最大高度。
pub(super) const BODY_MAX_HEIGHT: f32 = 360.;
/// 终端输出最大高度。
pub(super) const TERMINAL_MAX_HEIGHT: f32 = 280.;

/// 语义淡底透明度。
pub(super) const TINT_ALPHA: u8 = 0x14;

/// 6×6 语义指示方块。
pub(super) fn indicator(color: u32) -> gpui::Div {
    div()
        .w(ui_px(INDICATOR))
        .h(ui_px(INDICATOR))
        .flex_none()
        .bg(rgba(color))
}

/// 带默认 hover（bg2）的按钮。
pub(super) fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    theme: Theme,
) -> gpui::Stateful<gpui::Div> {
    semantic_button(id, label, theme)
        .text_size(ui_px(BUTTON_SIZE))
        .hover(move |style| style.bg(rgba(theme.bg2)))
}

/// 主动作按钮：强调边框（accent），文字 fg0。
pub(super) fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    theme: Theme,
    border: u32,
) -> gpui::Stateful<gpui::Div> {
    button(id, label, theme)
        .px_3()
        .py_1()
        .border_color(rgba(border))
        .text_color(rgba(theme.fg0))
}

/// 次动作按钮：无强调边框，文字色由调用方决定。
pub(super) fn secondary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    theme: Theme,
    text: u32,
) -> gpui::Stateful<gpui::Div> {
    button(id, label, theme)
        .px_3()
        .py_1()
        .text_color(rgba(text))
}

/// 统一 28px 图标按钮：fg2，hover 时 bg2 + fg0。
pub(super) fn icon_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    theme: Theme,
) -> gpui::Stateful<gpui::Div> {
    semantic_button(id, label, theme)
        .w(ui_px(ICON_BUTTON))
        .h(ui_px(ICON_BUTTON))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgba(theme.fg2))
        .hover(move |style| style.bg(rgba(theme.bg2)).text_color(rgba(theme.fg0)))
}

/// 语义信号卡：2px 左色条 + 淡底，内容左沿对齐 `CONTENT_INSET`。
pub(super) fn signal_card(theme: Theme, color: u32) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .mx(ui_px(CARD_MARGIN))
        .my_2()
        .pl(ui_px(BAR_PAD))
        .pr(ui_px(CARD_PAD))
        .py(ui_px(CARD_PAD))
        .border_l_2()
        .border_color(rgba(color))
        .bg(rgba(Theme::with_alpha(color, TINT_ALPHA)))
        .text_size(ui_px(BODY_SIZE))
        .line_height(ui_px(BODY_LINE))
        .text_color(rgba(theme.fg1))
}

/// 卡片标题行：指示方块 + fg0 semibold 标题。
pub(super) fn card_title(theme: Theme, color: u32, title: impl Into<SharedString>) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(indicator(color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgba(theme.fg0))
                .child(title.into()),
        )
}

/// 元信息文本：fg2 + META_SIZE。
pub(super) fn meta(theme: Theme, text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(ui_px(META_SIZE))
        .text_color(rgba(theme.fg2))
        .child(text.into())
}
