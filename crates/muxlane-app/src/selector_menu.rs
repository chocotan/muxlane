use crate::theme::Theme;
use crate::ui_scale::px as ui_px;
use gpui::{
    div, prelude::*, rgba, App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable,
    KeyDownEvent, Render, ScrollHandle, SharedString, Window,
};
use std::rc::Rc;

pub(crate) struct SelectorMenuItem {
    pub(crate) label: SharedString,
    pub(crate) selected: bool,
    #[allow(clippy::type_complexity)]
    pub(crate) action: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl SelectorMenuItem {
    pub(crate) fn action(
        label: impl Into<SharedString>,
        selected: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            selected,
            action: Some(Rc::new(action)),
        }
    }

    pub(crate) fn informational(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            selected: false,
            action: None,
        }
    }
}

pub(crate) struct SelectorMenu {
    items: Vec<SelectorMenuItem>,
    selected_index: usize,
    focus_handle: FocusHandle,
    theme: Theme,
    scroll: ScrollHandle,
}

impl SelectorMenu {
    pub(crate) fn new(items: Vec<SelectorMenuItem>, cx: &mut Context<Self>, theme: Theme) -> Self {
        let selected_index = items
            .iter()
            .position(|item| item.selected && item.action.is_some())
            .or_else(|| items.iter().position(|item| item.action.is_some()))
            .unwrap_or(0);
        Self {
            items,
            selected_index,
            focus_handle: cx.focus_handle(),
            theme,
            scroll: ScrollHandle::new(),
        }
    }

    pub(crate) fn set_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    fn select_next(&mut self, direction: isize, cx: &mut Context<Self>) {
        if let Some(index) = next_actionable_index(&self.items, self.selected_index, direction) {
            self.selected_index = index;
            cx.notify();
        }
    }

    fn activate_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self
            .items
            .get(self.selected_index)
            .and_then(|item| item.action.clone())
        else {
            cx.emit(DismissEvent);
            return;
        };
        action(window, cx);
        cx.emit(DismissEvent);
    }
}

impl Focusable for SelectorMenu {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DismissEvent> for SelectorMenu {}

impl Render for SelectorMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let selected_index = self.selected_index;
        let items = std::mem::take(&mut self.items);
        let mut rows = div().flex().flex_col();
        for (index, item) in items.iter().enumerate() {
            let actionable = item.action.is_some();
            let selected = index == selected_index;
            let action = item.action.clone();
            let row = div()
                .id(("selector-menu-item", index))
                .flex()
                .items_center()
                .w_full()
                .px_3()
                .py_2()
                .text_size(ui_px(11.))
                .text_color(rgba(if actionable { theme.fg0 } else { theme.fg2 }))
                .when(selected, |row| row.bg(rgba(theme.bg2)))
                .when(actionable, |row| {
                    row.cursor_pointer()
                        .hover(|style| style.bg(rgba(theme.bg2)))
                        .on_hover(cx.listener(move |this, hovered, _window, cx| {
                            if *hovered
                                && this
                                    .items
                                    .get(index)
                                    .is_some_and(|item| item.action.is_some())
                            {
                                this.selected_index = index;
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            cx.stop_propagation();
                            this.selected_index = index;
                            if let Some(action) = action.clone() {
                                action(window, cx);
                                cx.emit(DismissEvent);
                            }
                        }))
                })
                .child(item.label.clone());
            rows = rows.child(row);
        }
        self.items = items;

        div()
            .id("selector-menu")
            .min_w(ui_px(160.))
            .max_h(ui_px(360.))
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .track_focus(&self.focus_handle)
            .border_1()
            .border_color(rgba(theme.line))
            .bg(rgba(theme.bg1))
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|_this, _event, _window, cx| {
                cx.emit(DismissEvent);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "up" => this.select_next(-1, cx),
                    "down" => this.select_next(1, cx),
                    "enter" => this.activate_selected(window, cx),
                    "escape" => cx.emit(DismissEvent),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(rows)
    }
}

fn next_actionable_index(
    items: &[SelectorMenuItem],
    current: usize,
    direction: isize,
) -> Option<usize> {
    if items.is_empty() || direction == 0 {
        return None;
    }
    let length = items.len();
    let current = current % length;
    for offset in 1..=length {
        let offset = offset % length;
        let index = if direction.is_positive() {
            (current + offset) % length
        } else {
            (current + length - offset) % length
        };
        if items.get(index).is_some_and(|item| item.action.is_some()) {
            return Some(index);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{next_actionable_index, SelectorMenuItem};

    #[test]
    fn navigation_wraps_and_skips_informational_rows() {
        let items = vec![
            SelectorMenuItem::informational("loading"),
            SelectorMenuItem::action("first", false, |_, _| {}),
            SelectorMenuItem::informational("separator"),
            SelectorMenuItem::action("last", false, |_, _| {}),
        ];
        assert_eq!(next_actionable_index(&items, 1, 1), Some(3));
        assert_eq!(next_actionable_index(&items, 3, 1), Some(1));
        assert_eq!(next_actionable_index(&items, 1, -1), Some(3));
        assert_eq!(next_actionable_index(&items, 3, -1), Some(1));
    }

    #[test]
    fn navigation_returns_none_without_actionable_rows() {
        let items = vec![SelectorMenuItem::informational("loading")];
        assert_eq!(next_actionable_index(&items, 0, 1), None);
        assert_eq!(next_actionable_index(&[], 0, -1), None);
        assert_eq!(next_actionable_index(&items, 0, 0), None);
    }
}
