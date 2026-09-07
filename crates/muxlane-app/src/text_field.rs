use crate::theme::{Theme, ThemeMode};
use crate::ui_scale::px as ui_px;
use gpui::{
    canvas, div, point, prelude::*, rgba, size, Bounds, Context, ElementInputHandler,
    EntityInputHandler, FocusHandle, Focusable, Pixels, Point, Render, SharedString, Subscription,
    UTF16Selection, Window,
};
use std::ops::Range;

fn utf16_to_byte(content: &str, target: usize) -> usize {
    let mut utf16 = 0;
    for (byte, ch) in content.char_indices() {
        if utf16 >= target {
            return byte;
        }
        utf16 += ch.len_utf16();
        if utf16 > target {
            return byte;
        }
    }
    content.len()
}

fn byte_to_utf16(content: &str, target: usize) -> usize {
    let mut boundary = target.min(content.len());
    while !content.is_char_boundary(boundary) {
        boundary -= 1;
    }
    content[..boundary].encode_utf16().count()
}

fn range_from_utf16_in(content: &str, range: Range<usize>) -> Range<usize> {
    utf16_to_byte(content, range.start)..utf16_to_byte(content, range.end)
}

fn marked_selection(base: usize, new_text: &str, selected_utf16: Range<usize>) -> Range<usize> {
    let selected = range_from_utf16_in(new_text, selected_utf16);
    base + selected.start..base + selected.end
}

fn ime_caret_x(origin_x: f32, prefix_chars: usize, scale: f32) -> f32 {
    origin_x + (12.0 + prefix_chars as f32 * 7.2) * scale
}

pub struct TextField {
    focus: FocusHandle,
    content: String,
    placeholder: SharedString,
    selected_range: Range<usize>,
    marked_range: Option<Range<usize>>,
    secure: bool,
    theme_mode: ThemeMode,
    _focus_subscription: Subscription,
}

impl TextField {
    pub fn new(
        placeholder: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let focus_subscription = cx.on_focus_in(&focus, window, |_this, window, _cx| {
            window.invalidate_character_coordinates();
        });
        Self {
            focus,
            content: String::new(),
            placeholder: placeholder.into(),
            selected_range: 0..0,
            marked_range: None,
            secure: false,
            theme_mode: ThemeMode::Light,
            _focus_subscription: focus_subscription,
        }
    }

    pub fn new_secure(
        placeholder: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut field = Self::new(placeholder, window, cx);
        field.secure = true;
        field
    }

    pub fn set_theme_mode(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
        self.theme_mode = mode;
        cx.notify();
    }

    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    pub fn text(&self) -> String {
        self.content.clone()
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.content.clear();
        self.selected_range = 0..0;
        self.marked_range = None;
        cx.notify();
    }

    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.content = text.into();
        let len = self.content.len();
        self.selected_range = len..len;
        self.marked_range = None;
        cx.notify();
    }

    fn utf16_to_byte(&self, target: usize) -> usize {
        utf16_to_byte(&self.content, target)
    }

    fn byte_to_utf16(&self, target: usize) -> usize {
        byte_to_utf16(&self.content, target)
    }

    fn range_from_utf16(&self, range: Range<usize>) -> Range<usize> {
        self.utf16_to_byte(range.start)..self.utf16_to_byte(range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.byte_to_utf16(range.start)..self.byte_to_utf16(range.end)
    }

    fn move_home(&mut self, select: bool, cx: &mut Context<Self>) {
        if select {
            self.selected_range = 0..self.selected_range.end;
        } else {
            self.selected_range = 0..0;
        }
        self.marked_range = None;
        cx.notify();
    }

    fn move_end(&mut self, select: bool, cx: &mut Context<Self>) {
        let len = self.content.len();
        if select {
            self.selected_range = self.selected_range.start..len;
        } else {
            self.selected_range = len..len;
        }
        self.marked_range = None;
        cx.notify();
    }

    fn move_left(&mut self, select: bool, by_word: bool, cx: &mut Context<Self>) {
        let target = if by_word {
            let cursor = self.selected_range.start;
            let slice = &self.content[..cursor];
            let mut non_space = false;
            let mut pos = 0;
            for (idx, ch) in slice.char_indices().rev() {
                if !ch.is_whitespace() && ch != '/' && ch != '\\' {
                    non_space = true;
                } else if non_space {
                    pos = idx + ch.len_utf8();
                    break;
                }
            }
            pos
        } else if !self.selected_range.is_empty() && !select {
            self.selected_range.start
        } else if self.selected_range.start > 0 {
            self.content[..self.selected_range.start]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0)
        } else {
            0
        };

        if select {
            self.selected_range = target..self.selected_range.end;
        } else {
            self.selected_range = target..target;
        }
        self.marked_range = None;
        cx.notify();
    }

    fn move_right(&mut self, select: bool, by_word: bool, cx: &mut Context<Self>) {
        let len = self.content.len();
        let target = if by_word {
            let cursor = self.selected_range.end;
            let slice = &self.content[cursor..];
            let mut non_sep = false;
            let mut pos = len;
            for (idx, ch) in slice.char_indices() {
                if !ch.is_whitespace() && ch != '/' && ch != '\\' {
                    non_sep = true;
                } else if non_sep {
                    pos = cursor + idx;
                    break;
                }
            }
            pos
        } else if !self.selected_range.is_empty() && !select {
            self.selected_range.end
        } else if self.selected_range.end < len {
            self.content[self.selected_range.end..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.selected_range.end + i)
                .unwrap_or(len)
        } else {
            len
        };

        if select {
            self.selected_range = self.selected_range.start..target;
        } else {
            self.selected_range = target..target;
        }
        self.marked_range = None;
        cx.notify();
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.marked_range = None;
        cx.notify();
    }

    fn backspace(&mut self, cx: &mut Context<Self>) {
        let range = if !self.selected_range.is_empty() {
            self.selected_range.clone()
        } else if self.selected_range.start > 0 {
            let end = self.selected_range.start;
            let start = self.content[..end]
                .char_indices()
                .next_back()
                .map(|(byte, _)| byte)
                .unwrap_or(0);
            start..end
        } else {
            return;
        };
        self.content.replace_range(range.clone(), "");
        self.selected_range = range.start..range.start;
        self.marked_range = None;
        cx.notify();
    }

    fn delete_forward(&mut self, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            let range = self.selected_range.clone();
            self.content.replace_range(range.clone(), "");
            self.selected_range = range.start..range.start;
            self.marked_range = None;
            cx.notify();
        } else if self.selected_range.start < self.content.len() {
            let start = self.selected_range.start;
            let end = self.content[start..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| start + i)
                .unwrap_or(self.content.len());
            self.content.replace_range(start..end, "");
            self.selected_range = start..start;
            self.marked_range = None;
            cx.notify();
        }
    }

    fn delete_to_start(&mut self, cx: &mut Context<Self>) {
        let end = self.selected_range.start;
        self.content.replace_range(0..end, "");
        self.selected_range = 0..0;
        self.marked_range = None;
        cx.notify();
    }

    fn delete_to_end(&mut self, cx: &mut Context<Self>) {
        let start = self.selected_range.start;
        self.content.truncate(start);
        self.selected_range = start..start;
        self.marked_range = None;
        cx.notify();
    }

    fn delete_word_backward(&mut self, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            self.backspace(cx);
            return;
        }
        let cursor = self.selected_range.start;
        let slice = &self.content[..cursor];
        let mut non_sep = false;
        let mut start = 0;
        for (idx, ch) in slice.char_indices().rev() {
            if !ch.is_whitespace() && ch != '/' && ch != '\\' {
                non_sep = true;
            } else if non_sep {
                start = idx + ch.len_utf8();
                break;
            }
        }
        self.content.replace_range(start..cursor, "");
        self.selected_range = start..start;
        self.marked_range = None;
        cx.notify();
    }

    fn copy(&self, cx: &mut Context<Self>) {
        let text = if !self.selected_range.is_empty() {
            let start = self.selected_range.start.min(self.content.len());
            let end = self.selected_range.end.min(self.content.len());
            self.content[start..end].to_string()
        } else {
            self.content.clone()
        };
        if !text.is_empty() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
    }

    fn cut(&mut self, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            let start = self.selected_range.start.min(self.content.len());
            let end = self.selected_range.end.min(self.content.len());
            let text = self.content[start..end].to_string();
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            self.backspace(cx);
        }
    }

    fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = cx.read_from_clipboard() {
            if let Some(text) = item.text() {
                self.replace_text_in_range(None, &text, window, cx);
            }
        }
    }
}

impl Focusable for TextField {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(range_utf16);
        *adjusted_range = Some(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        window.invalidate_character_coordinates();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let start = range.start.min(self.content.len());
        let end = range.end.min(self.content.len());
        self.content.replace_range(start..end, text);
        let cursor = start + text.len();
        self.selected_range = cursor..cursor;
        self.marked_range = None;
        window.invalidate_character_coordinates();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let start = range.start.min(self.content.len());
        let end = range.end.min(self.content.len());
        self.content.replace_range(start..end, new_text);
        let marked = start..start + new_text.len();
        self.marked_range = (!new_text.is_empty()).then_some(marked.clone());
        self.selected_range = new_selected_range_utf16
            .map(|selected| marked_selection(start, new_text, selected))
            .unwrap_or(marked.end..marked.end);
        window.invalidate_character_coordinates();
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let start = utf16_to_byte(&self.content, range_utf16.start).min(self.content.len());
        let prefix = self.content[..start].chars().count();
        let x = ime_caret_x(
            f32::from(element_bounds.origin.x),
            prefix,
            crate::ui_scale::factor(),
        );
        Some(Bounds {
            origin: point(gpui::px(x), element_bounds.origin.y),
            size: size(ui_px(2.), element_bounds.size.height),
        })
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.byte_to_utf16(self.content.len()))
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().clone();
        let focus = self.focus.clone();
        let focused = focus.is_focused(window);

        let theme = Theme::for_mode(self.theme_mode);
        let border_c = if focused { theme.accent } else { theme.line };
        let bg_c = theme.bg0;
        let text_c = if self.content.is_empty() {
            theme.fg2
        } else {
            theme.fg0
        };
        let cursor_c = theme.accent;

        let content_len = self.content.len();
        let sel_start = self.selected_range.start.min(content_len);
        let sel_end = self.selected_range.end.min(content_len);
        let has_selection = sel_start < sel_end;

        let content_el = if self.content.is_empty() {
            div()
                .flex()
                .items_center()
                .child(
                    div()
                        .text_color(rgba(text_c))
                        .child(self.placeholder.clone()),
                )
                .when(focused, |el| {
                    el.child(
                        div()
                            .ml(ui_px(1.))
                            .w(ui_px(1.5))
                            .h(ui_px(16.))
                            .bg(rgba(cursor_c)),
                    )
                })
        } else if !focused {
            let display_text = if self.secure {
                "•".repeat(self.content.chars().count())
            } else {
                self.content.clone()
            };
            div().text_color(rgba(text_c)).child(display_text)
        } else {
            let before = if self.secure {
                "•".repeat(self.content[..sel_start].chars().count())
            } else {
                self.content[..sel_start].to_string()
            };
            let selected = if self.secure {
                "•".repeat(self.content[sel_start..sel_end].chars().count())
            } else {
                self.content[sel_start..sel_end].to_string()
            };
            let after = if self.secure {
                "•".repeat(self.content[sel_end..].chars().count())
            } else {
                self.content[sel_end..].to_string()
            };

            let mut row = div().flex().items_center().text_color(rgba(text_c));
            if !before.is_empty() {
                row = row.child(div().child(before));
            }
            if has_selection {
                row = row.child(div().bg(rgba(theme.selection())).child(selected));
            } else {
                row = row.child(div().w(ui_px(1.5)).h(ui_px(16.)).bg(rgba(cursor_c)));
            }
            if !after.is_empty() {
                row = row.child(div().child(after));
            }
            row
        };

        div()
            .id("text-field")
            .relative()
            .track_focus(&focus)
            .flex()
            .items_center()
            .w_full()
            .min_h(ui_px(34.))
            .px_3()
            .border_1()
            .border_color(rgba(border_c))
            .bg(rgba(bg_c))
            .text_size(ui_px(12.))
            .on_click(cx.listener(|this, _event, window, cx| {
                this.focus.focus(window, cx);
                window.invalidate_character_coordinates();
                cx.stop_propagation();
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                let ctrl = event.keystroke.modifiers.control;
                let alt = event.keystroke.modifiers.alt;
                let shift = event.keystroke.modifiers.shift;
                let platform = event.keystroke.modifiers.platform;
                let ctrl_or_cmd = ctrl || platform;

                let handled = match key {
                    "home" => {
                        this.move_home(shift, cx);
                        true
                    }
                    "end" => {
                        this.move_end(shift, cx);
                        true
                    }
                    "left" => {
                        this.move_left(shift, alt || ctrl, cx);
                        true
                    }
                    "right" => {
                        this.move_right(shift, alt || ctrl, cx);
                        true
                    }
                    "backspace" => {
                        if ctrl_or_cmd {
                            this.delete_to_start(cx);
                        } else if alt {
                            this.delete_word_backward(cx);
                        } else {
                            this.backspace(cx);
                        }
                        true
                    }
                    "delete" => {
                        this.delete_forward(cx);
                        true
                    }
                    "a" if ctrl_or_cmd => {
                        this.select_all(cx);
                        true
                    }
                    "a" if ctrl && !platform => {
                        this.move_home(shift, cx);
                        true
                    }
                    "e" if ctrl && !platform => {
                        this.move_end(shift, cx);
                        true
                    }
                    "b" if ctrl && !platform => {
                        this.move_left(shift, false, cx);
                        true
                    }
                    "f" if ctrl && !platform => {
                        this.move_right(shift, false, cx);
                        true
                    }
                    "u" if ctrl && !platform => {
                        this.delete_to_start(cx);
                        true
                    }
                    "k" if ctrl && !platform => {
                        this.delete_to_end(cx);
                        true
                    }
                    "w" if ctrl && !platform => {
                        this.delete_word_backward(cx);
                        true
                    }
                    "d" if ctrl && !platform => {
                        this.delete_forward(cx);
                        true
                    }
                    "c" if ctrl_or_cmd => {
                        this.copy(cx);
                        true
                    }
                    "x" if ctrl_or_cmd => {
                        this.cut(cx);
                        true
                    }
                    "v" if ctrl_or_cmd => {
                        this.paste(window, cx);
                        true
                    }
                    // 普通字符不在此处自插：字符插入统一走平台层
                    // key_char → replace_text_in_range 路径（前提是事件
                    // 未被祖先 stop_propagation，见 app.rs 根节点监听器）。
                    // 祖先导航处理器也应放行普通字符，确保输入框可以正常搜索。
                    _ => false,
                };

                if handled {
                    window.invalidate_character_coordinates();
                    cx.stop_propagation();
                }
            }))
            .child(content_el)
            .child(
                canvas(
                    |_bounds, _window, _cx| {},
                    move |bounds, _state, window, cx| {
                        window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
                    },
                )
                .absolute()
                .size_full(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_conversion_handles_chinese_and_surrogate_pairs() {
        let content = "A中😀B";
        assert_eq!(utf16_to_byte(content, 2), "A中".len());
        assert_eq!(byte_to_utf16(content, "A中😀".len()), 4);
        assert_eq!(byte_to_utf16(content, 2), 1);
    }

    #[test]
    fn marked_selection_is_relative_to_new_text() {
        assert_eq!(marked_selection(3, "中文", 2..2), 9..9);
        assert_eq!(marked_selection(3, "😀x", 2..3), 7..8);
    }

    #[test]
    fn ime_caret_coordinates_follow_ui_scale() {
        assert!((ime_caret_x(20.0, 2, 1.0) - 46.4).abs() < f32::EPSILON);
        assert!((ime_caret_x(20.0, 2, 1.5) - 59.6).abs() < f32::EPSILON);
        assert!((ime_caret_x(20.0, 2, 2.0) - 72.8).abs() < f32::EPSILON);
    }
}
