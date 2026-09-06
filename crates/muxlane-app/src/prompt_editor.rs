use crate::theme::{Theme, ThemeMode};
use crate::ui_scale::px as ui_px;
use gpui::{
    div, fill, point, prelude::*, rgba, size, Bounds, Context, CursorStyle, Element,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    GlobalElementId, HighlightStyle, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, Render, SharedString, StyledText, Subscription,
    TextLayout, UTF16Selection, UnderlineStyle, Window,
};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PromptEditorEvent {
    Submit,
    Edited,
    CompletionPrevious,
    CompletionNext,
    CompletionDismiss,
}

fn utf16_to_byte(content: &str, target: usize) -> usize {
    let mut units = 0;
    for (byte, character) in content.char_indices() {
        if units >= target {
            return byte;
        }
        units += character.len_utf16();
        if units > target {
            return byte;
        }
    }
    content.len()
}

fn byte_to_utf16(content: &str, target: usize) -> usize {
    let mut boundary = target.min(content.len());
    while !content.is_char_boundary(boundary) {
        boundary = boundary.saturating_sub(1);
    }
    content[..boundary].encode_utf16().count()
}

fn range_from_utf16(content: &str, range: Range<usize>) -> Range<usize> {
    utf16_to_byte(content, range.start)..utf16_to_byte(content, range.end)
}

fn line_column(content: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(content.len());
    let before = &content[..offset];
    let line = before.matches('\n').count();
    let column = before
        .rsplit_once('\n')
        .map_or(before, |(_, current)| current)
        .chars()
        .count();
    (line, column)
}

fn byte_for_line_column(content: &str, target_line: usize, target_column: usize) -> usize {
    let mut line_start = 0;
    let mut line = 0;
    for (index, character) in content.char_indices() {
        if line == target_line {
            return content[line_start..]
                .char_indices()
                .take_while(|(_, character)| *character != '\n')
                .nth(target_column)
                .map_or_else(
                    || {
                        content[line_start..]
                            .find('\n')
                            .map_or(content.len(), |end| line_start + end)
                    },
                    |(column, _)| line_start + column,
                );
        }
        if character == '\n' {
            line += 1;
            line_start = index + 1;
        }
    }
    if line == target_line {
        content[line_start..]
            .char_indices()
            .nth(target_column)
            .map_or(content.len(), |(column, _)| line_start + column)
    } else {
        content.len()
    }
}

fn marked_selection(base: usize, text: &str, range: Range<usize>) -> Range<usize> {
    let range = range_from_utf16(text, range);
    base + range.start..base + range.end
}

pub(crate) struct PromptEditor {
    focus: FocusHandle,
    content: String,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    completion_active: bool,
    submit_enabled: bool,
    chrome_visible: bool,
    theme_mode: ThemeMode,
    last_layout: Option<TextLayout>,
    last_bounds: Option<Bounds<Pixels>>,
    last_line_height: Option<Pixels>,
    is_selecting: bool,
    _focus_subscription: Subscription,
}

impl PromptEditor {
    pub(crate) fn new(
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
            selection_reversed: false,
            marked_range: None,
            completion_active: false,
            submit_enabled: true,
            chrome_visible: true,
            theme_mode: ThemeMode::Light,
            last_layout: None,
            last_bounds: None,
            last_line_height: None,
            is_selecting: false,
            _focus_subscription: focus_subscription,
        }
    }

    pub(crate) fn text(&self) -> String {
        self.content.clone()
    }

    pub(crate) fn reset(&mut self, cx: &mut Context<Self>) {
        self.content.clear();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.edited(cx);
    }

    pub(crate) fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.content = text.into();
        let end = self.content.len();
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        self.edited(cx);
    }

    pub(crate) fn set_completion_active(&mut self, active: bool) {
        self.completion_active = active;
    }

    pub(crate) fn set_submit_enabled(&mut self, enabled: bool) {
        self.submit_enabled = enabled;
    }

    pub(crate) fn set_chrome_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.chrome_visible = visible;
        cx.notify();
    }

    pub(crate) fn set_theme_mode(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
        self.theme_mode = mode;
        cx.notify();
    }

    pub(crate) fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    fn edited(&mut self, cx: &mut Context<Self>) {
        cx.emit(PromptEditorEvent::Edited);
        cx.notify();
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn selection_utf16(&self) -> Range<usize> {
        byte_to_utf16(&self.content, self.selected_range.start)
            ..byte_to_utf16(&self.content, self.selected_range.end)
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content.len());
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content.len());
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn index_for_point(&self, position: Point<Pixels>) -> Option<usize> {
        if self.content.is_empty() {
            return Some(0);
        }
        let layout = self.last_layout.as_ref()?;
        Some(
            layout
                .index_for_position(position)
                .unwrap_or_else(|index| index)
                .min(self.content.len()),
        )
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window, cx);
        window.invalidate_character_coordinates();
        self.is_selecting = true;
        if let Some(index) = self.index_for_point(event.position) {
            if event.modifiers.shift {
                self.select_to(index, cx);
            } else {
                self.move_to(index, cx);
            }
        }
        cx.stop_propagation();
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_selecting {
            if let Some(index) = self.index_for_point(event.position) {
                self.select_to(index, cx);
            }
        }
    }

    fn on_mouse_up(&mut self, _event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = false;
        cx.notify();
    }

    fn replace_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .map(|range| range_from_utf16(&self.content, range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let start = range.start.min(self.content.len());
        let end = range.end.min(self.content.len());
        self.content.replace_range(start..end, text);
        let cursor = start + text.len();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.marked_range = None;
        self.edited(cx);
    }

    fn move_home(&mut self, select: bool, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let target = if let (Some(layout), Some(bounds)) = (&self.last_layout, &self.last_bounds) {
            layout
                .position_for_index(cursor)
                .map(|position| {
                    layout
                        .index_for_position(point(bounds.left(), position.y))
                        .unwrap_or_else(|index| index)
                })
                .unwrap_or_else(|| {
                    self.content[..cursor]
                        .rfind('\n')
                        .map_or(0, |index| index + 1)
                })
        } else {
            self.content[..cursor]
                .rfind('\n')
                .map_or(0, |index| index + 1)
        };
        if select {
            self.select_to(target, cx);
        } else {
            self.move_to(target, cx);
        }
    }

    fn move_end(&mut self, select: bool, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let target = if let (Some(layout), Some(bounds)) = (&self.last_layout, &self.last_bounds) {
            layout
                .position_for_index(cursor)
                .map(|position| {
                    layout
                        .index_for_position(point(bounds.right(), position.y))
                        .unwrap_or_else(|index| index)
                })
                .unwrap_or_else(|| {
                    self.content[cursor..]
                        .find('\n')
                        .map_or(self.content.len(), |index| cursor + index)
                })
        } else {
            self.content[cursor..]
                .find('\n')
                .map_or(self.content.len(), |index| cursor + index)
        };
        if select {
            self.select_to(target, cx);
        } else {
            self.move_to(target, cx);
        }
    }

    fn move_horizontal(&mut self, right: bool, select: bool, cx: &mut Context<Self>) {
        let target = if right {
            if !select && !self.selected_range.is_empty() {
                self.selected_range.end
            } else {
                self.content[self.cursor_offset()..]
                    .char_indices()
                    .nth(1)
                    .map_or(self.content.len(), |(index, _)| {
                        self.cursor_offset() + index
                    })
            }
        } else if !select && !self.selected_range.is_empty() {
            self.selected_range.start
        } else {
            self.content[..self.cursor_offset()]
                .char_indices()
                .next_back()
                .map_or(0, |(index, _)| index)
        };
        if select {
            self.select_to(target, cx);
        } else {
            self.move_to(target, cx);
        }
    }

    fn move_vertical(&mut self, down: bool, select: bool, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let target =
            if let (Some(layout), Some(line_height)) = (&self.last_layout, self.last_line_height) {
                layout
                    .position_for_index(cursor)
                    .map(|position| {
                        let y = if down {
                            position.y + line_height
                        } else {
                            position.y - line_height
                        };
                        layout
                            .index_for_position(point(position.x, y))
                            .unwrap_or_else(|index| index)
                            .min(self.content.len())
                    })
                    .unwrap_or(cursor)
            } else {
                let (line, column) = line_column(&self.content, cursor);
                let target_line = if down {
                    line.saturating_add(1)
                } else {
                    line.saturating_sub(1)
                };
                if target_line == line {
                    cursor
                } else {
                    byte_for_line_column(&self.content, target_line, column)
                }
            };
        if target == cursor {
            return;
        }
        if select {
            self.select_to(target, cx);
        } else {
            self.move_to(target, cx);
        }
    }

    fn backspace(&mut self, cx: &mut Context<Self>) {
        let range = if !self.selected_range.is_empty() {
            self.selected_range.clone()
        } else if self.selected_range.start > 0 {
            let end = self.selected_range.start;
            self.content[..end]
                .char_indices()
                .next_back()
                .map_or(0..end, |(start, _)| start..end)
        } else {
            return;
        };
        self.content.replace_range(range.clone(), "");
        self.selected_range = range.start..range.start;
        self.selection_reversed = false;
        self.marked_range = None;
        self.edited(cx);
    }

    fn delete_forward(&mut self, cx: &mut Context<Self>) {
        let range = if !self.selected_range.is_empty() {
            self.selected_range.clone()
        } else if self.selected_range.start < self.content.len() {
            let start = self.selected_range.start;
            let end = self.content[start..]
                .char_indices()
                .nth(1)
                .map_or(self.content.len(), |(index, _)| start + index);
            start..end
        } else {
            return;
        };
        self.content.replace_range(range.clone(), "");
        self.selected_range = range.start..range.start;
        self.selection_reversed = false;
        self.marked_range = None;
        self.edited(cx);
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    fn copy(&self, cx: &mut Context<Self>) {
        let text = if self.selected_range.is_empty() {
            self.content.clone()
        } else {
            self.content[self.selected_range.clone()].to_string()
        };
        if !text.is_empty() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
    }

    fn cut(&mut self, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            let text = self.content[self.selected_range.clone()].to_string();
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            self.backspace(cx);
        }
    }

    fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = cx.read_from_clipboard() {
            if let Some(text) = item.text() {
                self.replace_range(None, &text, cx);
                window.invalidate_character_coordinates();
            }
        }
    }
}

impl EventEmitter<PromptEditorEvent> for PromptEditor {}

impl Focusable for PromptEditor {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EntityInputHandler for PromptEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = range_from_utf16(&self.content, range_utf16);
        *adjusted_range = Some(
            byte_to_utf16(&self.content, range.start)..byte_to_utf16(&self.content, range.end),
        );
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.selection_utf16(),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|range| {
            byte_to_utf16(&self.content, range.start)..byte_to_utf16(&self.content, range.end)
        })
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
        self.replace_range(range_utf16, text, cx);
        window.invalidate_character_coordinates();
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
            .map(|range| range_from_utf16(&self.content, range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        let start = range.start.min(self.content.len());
        let end = range.end.min(self.content.len());
        self.content.replace_range(start..end, new_text);
        let marked = start..start + new_text.len();
        self.marked_range = (!new_text.is_empty()).then_some(marked.clone());
        self.selected_range = new_selected_range_utf16
            .map(|range| marked_selection(start, new_text, range))
            .unwrap_or(marked.end..marked.end);
        self.selection_reversed = false;
        window.invalidate_character_coordinates();
        self.edited(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let byte = utf16_to_byte(&self.content, range_utf16.start);
        let origin = self
            .last_layout
            .as_ref()
            .and_then(|layout| layout.position_for_index(byte))
            .unwrap_or(element_bounds.origin);
        Some(Bounds {
            origin,
            size: size(ui_px(2.), window.line_height()),
        })
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        self.index_for_point(point)
            .map(|byte| byte_to_utf16(&self.content, byte))
    }
}

struct PromptTextElement {
    input: Entity<PromptEditor>,
    text: StyledText,
    focused: bool,
    cursor: usize,
    selection: Range<usize>,
    cursor_color: gpui::Hsla,
}

struct PromptTextPrepaint {
    cursor: Option<PaintQuad>,
}

impl IntoElement for PromptTextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for PromptTextElement {
    type RequestLayoutState = ();
    type PrepaintState = PromptTextPrepaint;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.text.request_layout(None, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> Self::PrepaintState {
        self.text
            .prepaint(None, inspector_id, bounds, request_layout, window, cx);
        let cursor = if self.focused && self.selection.is_empty() {
            self.text
                .layout()
                .position_for_index(self.cursor)
                .map(|position| {
                    fill(
                        Bounds::new(position, size(ui_px(1.5), window.line_height())),
                        self.cursor_color,
                    )
                })
        } else {
            None
        };
        PromptTextPrepaint { cursor }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut gpui::App,
    ) {
        let focus = self.input.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        self.text.paint(
            None,
            inspector_id,
            bounds,
            request_layout,
            &mut (),
            window,
            cx,
        );
        if let Some(cursor) = prepaint.cursor.take() {
            window.paint_quad(cursor);
        }
        let layout = self.text.layout().clone();
        let line_height = window.line_height();
        self.input.update(cx, |input, _cx| {
            input.last_layout = Some(layout);
            input.last_bounds = Some(bounds);
            input.last_line_height = Some(line_height);
        });
    }
}

impl Render for PromptEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.focus.clone();
        let focused = focus.is_focused(window);
        let theme = Theme::for_mode(self.theme_mode);
        let text_color = if self.content.is_empty() {
            theme.fg2
        } else {
            theme.fg0
        };
        let display_text: SharedString = if self.content.is_empty() {
            self.placeholder.clone()
        } else {
            self.content.clone().into()
        };
        let mut text = StyledText::new(display_text);
        let mut highlights = Vec::new();
        if !self.selected_range.is_empty() {
            highlights.push((
                self.selected_range.clone(),
                HighlightStyle {
                    background_color: Some(rgba(theme.selection()).into()),
                    ..Default::default()
                },
            ));
        }
        if let Some(marked_range) = self.marked_range.clone() {
            highlights.push((
                marked_range,
                HighlightStyle {
                    underline: Some(UnderlineStyle {
                        color: Some(rgba(theme.accent).into()),
                        thickness: ui_px(1.),
                        wavy: false,
                    }),
                    ..Default::default()
                },
            ));
        }
        if !highlights.is_empty() {
            text = text.with_highlights(highlights);
        }
        let text_element = PromptTextElement {
            input: cx.entity().clone(),
            text,
            focused,
            cursor: self.cursor_offset(),
            selection: self.selected_range.clone(),
            cursor_color: rgba(theme.accent).into(),
        };
        let mut root = div()
            .id("prompt-editor")
            .relative()
            .track_focus(&focus)
            .w_full()
            .max_h(ui_px(176.))
            .overflow_y_scroll()
            .text_size(ui_px(12.))
            .font_family("monospace")
            .text_color(rgba(text_color))
            .whitespace_normal()
            .cursor(CursorStyle::IBeam)
            .on_scroll_wheel(
                cx.listener(|_this, _event: &gpui::ScrollWheelEvent, window, _cx| {
                    window.invalidate_character_coordinates();
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.on_mouse_down(event, window, cx)
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                this.on_mouse_move(event, window, cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.on_mouse_up(event, window, cx)
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.on_mouse_up(event, window, cx)
                }),
            );
        if self.chrome_visible {
            root = root
                .min_h(ui_px(64.))
                .px_3()
                .py_2()
                .border_1()
                .border_color(rgba(if focused { theme.accent } else { theme.line }))
                .bg(rgba(theme.bg0));
        } else {
            root = root.min_h(ui_px(28.));
        }
        root.on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
            let key = event.keystroke.key.as_str();
            let modifiers = event.keystroke.modifiers;
            let command = modifiers.control || modifiers.platform;
            let handled = match key {
                "enter" if modifiers.shift && this.submit_enabled => {
                    this.replace_range(None, "\n", cx);
                    true
                }
                "enter" if this.marked_range.is_none() && this.submit_enabled => {
                    cx.emit(PromptEditorEvent::Submit);
                    true
                }
                "home" => {
                    this.move_home(modifiers.shift, cx);
                    true
                }
                "end" => {
                    this.move_end(modifiers.shift, cx);
                    true
                }
                "left" => {
                    this.move_horizontal(false, modifiers.shift, cx);
                    true
                }
                "right" => {
                    this.move_horizontal(true, modifiers.shift, cx);
                    true
                }
                "up" if this.completion_active => {
                    cx.emit(PromptEditorEvent::CompletionPrevious);
                    true
                }
                "down" if this.completion_active => {
                    cx.emit(PromptEditorEvent::CompletionNext);
                    true
                }
                "escape" if this.completion_active => {
                    cx.emit(PromptEditorEvent::CompletionDismiss);
                    true
                }
                "up" => {
                    this.move_vertical(false, modifiers.shift, cx);
                    true
                }
                "down" => {
                    this.move_vertical(true, modifiers.shift, cx);
                    true
                }
                "backspace" => {
                    this.backspace(cx);
                    true
                }
                "delete" => {
                    this.delete_forward(cx);
                    true
                }
                "a" if command => {
                    this.select_all(cx);
                    true
                }
                "c" if command => {
                    this.copy(cx);
                    true
                }
                "x" if command => {
                    this.cut(cx);
                    true
                }
                "v" if command => {
                    this.paste(window, cx);
                    true
                }
                _ => false,
            };
            if handled {
                window.invalidate_character_coordinates();
                cx.stop_propagation();
            }
        }))
        .child(text_element)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_round_trip_handles_surrogates_and_newlines() {
        let text = "A😀\n中";
        assert_eq!(utf16_to_byte(text, 3), "A😀".len());
        assert_eq!(byte_to_utf16(text, "A😀".len()), 3);
        assert_eq!(range_from_utf16(text, 3..4), 5..6);
    }

    #[test]
    fn vertical_cursor_movement_preserves_character_column() {
        let text = "abc\n中x\n12345";
        let second_line_column_one = byte_for_line_column(text, 1, 1);
        assert_eq!(&text[second_line_column_one..], "x\n12345");
        assert_eq!(line_column(text, second_line_column_one), (1, 1));
        assert_eq!(byte_for_line_column(text, 2, 1), "abc\n中x\n1".len());
    }

    #[test]
    fn marked_selection_is_relative_to_replaced_text() {
        assert_eq!(marked_selection(2, "😀x", 2..3), 6..7);
    }

    #[test]
    fn newline_is_not_counted_as_a_submit() {
        let text = "first\nsecond";
        assert_eq!(text.lines().count(), 2);
        assert!(!range_from_utf16(text, 5..6).is_empty());
    }
}
