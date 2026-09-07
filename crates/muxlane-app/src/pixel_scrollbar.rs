//! A pixel-based vertical scrollbar. Geometry is read after the scroll view's prepaint.
use crate::{theme::Theme, ui_scale::px as ui_px};
use gpui::{canvas, fill, point, prelude::*, px, rgba, size, Bounds, DispatchPhase, HitboxBehavior, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollHandle};
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Default)]
pub(crate) struct PixelScrollbar {
    drag: Rc<Cell<Option<f32>>>,
}

#[derive(Clone, Copy, Debug)]
struct Geometry {
    track: f32,
    thumb: f32,
    top: f32,
    maximum: f32,
}

impl Geometry {
    fn new(track: f32, viewport: f32, maximum: f32, offset: f32) -> Option<Self> {
        if track <= 0. || viewport <= 0. || maximum <= 0. { return None; }
        let thumb = (track * viewport / (viewport + maximum)).max(20.).min(track);
        Some(Self { track, thumb, top: (-offset / maximum).clamp(0., 1.) * (track - thumb), maximum })
    }

    fn offset(self, top: f32) -> f32 {
        if self.track <= self.thumb { return 0.; }
        -(top / (self.track - self.thumb)).clamp(0., 1.) * self.maximum
    }
}

impl PixelScrollbar {
    pub(crate) fn render(&self, scroll: &ScrollHandle, theme: Theme, on_scroll: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static) -> impl IntoElement {
        let scroll = scroll.clone();
        let drag = self.drag.clone();
        let on_scroll = Rc::new(on_scroll);
        let prepaint_scroll = scroll.clone();
        canvas(
            move |bounds, window, _| {
                let geometry = Geometry::new(bounds.size.height.into(), prepaint_scroll.bounds().size.height.into(), prepaint_scroll.max_offset().y.into(), prepaint_scroll.offset().y.into())?;
                Some((geometry, window.insert_hitbox(bounds, HitboxBehavior::Normal)))
            },
            move |bounds, state, window, _| {
                let Some((geometry, hitbox)) = state else { drag.set(None); return; };
                window.paint_quad(fill(bounds, rgba(theme.bg2)));
                window.paint_quad(fill(Bounds::new(point(bounds.left() + ui_px(2.), bounds.top() + px(geometry.top)), size((bounds.size.width - ui_px(4.)).max(px(1.)), px(geometry.thumb))), rgba(theme.fg2)));
                let scroll_down = scroll.clone();
                let drag_down = drag.clone();
                let callback = on_scroll.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble || event.button != MouseButton::Left || !hitbox.is_hovered(window) { return; }
                    let y = f32::from(event.position.y - bounds.top());
                    let grip = if y >= geometry.top && y <= geometry.top + geometry.thumb { y - geometry.top } else { geometry.thumb / 2. };
                    drag_down.set(Some(grip));
                    scroll_down.set_offset(point(scroll_down.offset().x, px(geometry.offset(y - grip))));
                    callback(window, cx);
                    window.invalidate_character_coordinates();
                    window.refresh();
                    cx.stop_propagation();
                });
                let scroll_move = scroll.clone();
                let drag_move = drag.clone();
                let callback = on_scroll.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble { return; }
                    let Some(grip) = drag_move.get() else { return; };
                    if !event.dragging() { drag_move.set(None); return; }
                    scroll_move.set_offset(point(scroll_move.offset().x, px(geometry.offset(f32::from(event.position.y - bounds.top()) - grip))));
                    callback(window, cx);
                    window.invalidate_character_coordinates();
                    window.refresh();
                    cx.stop_propagation();
                });
                let drag_up = drag.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, _| {
                    if phase == DispatchPhase::Capture && event.button == MouseButton::Left { drag_up.set(None); }
                });
            },
        ).absolute().right_0().top_0().bottom_0().w(ui_px(10.))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn track_click_and_thumb_drag_change_real_scroll_offset() {
        use gpui::{div, Context, Render, TestAppContext, VisualTestContext, Window};
        struct Fixture { scroll: ScrollHandle, bar: PixelScrollbar }
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().relative().size_full()
                    .child(div().id("scroll").size_full().overflow_y_scroll().track_scroll(&self.scroll)
                        .child(div().h(px(2000.)).w_full()))
                    .child(self.bar.render(&self.scroll, Theme::for_mode(crate::theme::ThemeMode::Light), |_, _| {}))
            }
        }
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|_, _| Fixture { scroll: ScrollHandle::new(), bar: Default::default() });
        let view = window.root(&mut cx).unwrap();
        let scroll = cx.update(|cx| view.read(cx).scroll.clone());
        let window: gpui::AnyWindowHandle = window.into();
        let mut visual = VisualTestContext::from_window(window, &mut cx);
        visual.simulate_resize(size(px(320.), px(400.)));
        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx)).unwrap();
        assert_eq!(scroll.max_offset().y, px(1600.));
        visual.simulate_click(point(px(315.), px(200.)), Default::default());
        assert_eq!(scroll.offset().y, px(-800.));
        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx)).unwrap();
        visual.simulate_mouse_down(point(px(315.), px(200.)), MouseButton::Left, Default::default());
        visual.simulate_mouse_move(point(px(315.), px(360.)), Some(MouseButton::Left), Default::default());
        visual.simulate_mouse_up(point(px(315.), px(360.)), MouseButton::Left, Default::default());
        assert_eq!(scroll.offset().y, px(-1600.));
        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx)).unwrap();
        visual.simulate_resize(size(px(320.), px(800.)));
        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx)).unwrap();
        visual.simulate_click(point(px(315.), px(160.)), Default::default());
        assert_eq!(scroll.offset().y, px(0.));
    }

    #[test]
    fn geometry_clamps_and_handles_empty_and_short_tracks() {
        assert!(Geometry::new(100., 100., 0., 0.).is_none());
        assert!(Geometry::new(0., 100., 100., 0.).is_none());
        let g = Geometry::new(100., 100., 900., -450.).unwrap();
        assert_eq!(g.thumb, 20.);
        assert_eq!(g.top, 40.);
        assert_eq!(g.offset(40.), -450.);
        assert_eq!(g.offset(-100.), 0.);
        assert_eq!(g.offset(1000.), -900.);
        assert_eq!(Geometry::new(10., 100., 900., 0.).unwrap().offset(5.), 0.);
    }
}
