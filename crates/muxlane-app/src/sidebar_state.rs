//! Resizable, persistable, and animated sidebar state.
use std::time::{Duration, Instant};

pub(crate) const DEFAULT_SIDEBAR_WIDTH: f32 = 230.0;
pub(crate) const MIN_SIDEBAR_WIDTH: f32 = 120.0;
pub(crate) const MAX_SIDEBAR_WIDTH: f32 = 480.0;
pub(crate) const SIDEBAR_RAIL_WIDTH: f32 = 5.0;

const REVEAL_DURATION: Duration = Duration::from_millis(220);
const HIDE_DURATION: Duration = Duration::from_millis(170);

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SidebarDrag {
    start_x: f32,
    start_width: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SidebarTransition {
    started_at: Instant,
    from: f32,
    to: f32,
    duration: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SidebarState {
    /// Persisted visibility target. The panel remains mounted while transitioning.
    pub(crate) visible: bool,
    /// Window logical pixels, independent of interface content scaling.
    pub(crate) width: f32,
    pub(crate) drag: Option<SidebarDrag>,
    pub(crate) reveal_progress: f32,
    pub(crate) transition: Option<SidebarTransition>,
}

impl SidebarState {
    pub(crate) fn new(visible: bool, width: f32) -> Self {
        Self {
            visible,
            width: clamp_width(width),
            drag: None,
            reveal_progress: if visible { 1.0 } else { 0.0 },
            transition: None,
        }
    }

    pub(crate) fn set_visible(&mut self, visible: bool, now: Instant, reduce_motion: bool) {
        self.advance_transition(now);
        self.visible = visible;
        let target = if visible { 1.0 } else { 0.0 };

        if reduce_motion || (self.reveal_progress - target).abs() < f32::EPSILON {
            self.reveal_progress = target;
            self.transition = None;
            return;
        }

        let full_duration = if visible {
            REVEAL_DURATION
        } else {
            HIDE_DURATION
        };
        let remaining = (target - self.reveal_progress).abs();
        self.transition = Some(SidebarTransition {
            started_at: now,
            from: self.reveal_progress,
            to: target,
            duration: full_duration.mul_f32(remaining),
        });
    }

    /// Advances the transition to `now`, returning whether the visual state changed.
    pub(crate) fn advance_transition(&mut self, now: Instant) -> bool {
        let Some(transition) = self.transition else {
            return false;
        };
        let elapsed = now.saturating_duration_since(transition.started_at);
        let linear = if transition.duration.is_zero() {
            1.0
        } else {
            elapsed.as_secs_f32() / transition.duration.as_secs_f32()
        }
        .clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - linear).powi(3);
        let next = transition.from + (transition.to - transition.from) * eased;
        let changed = (next - self.reveal_progress).abs() >= f32::EPSILON;
        self.reveal_progress = next.clamp(0.0, 1.0);

        if linear >= 1.0 {
            self.reveal_progress = transition.to;
            self.transition = None;
        }
        changed
    }

    pub(crate) fn is_transitioning(&self) -> bool {
        self.transition.is_some()
    }

    pub(crate) fn displayed_width(&self) -> f32 {
        SIDEBAR_RAIL_WIDTH + (self.width - SIDEBAR_RAIL_WIDTH) * self.reveal_progress
    }

    pub(crate) fn start_drag(&mut self, x: f32) {
        self.drag = Some(SidebarDrag {
            start_x: x,
            start_width: self.width,
        });
    }

    pub(crate) fn update_drag(&mut self, x: f32) -> bool {
        let Some(drag) = self.drag else {
            return false;
        };
        let next = clamp_width(drag.start_width + x - drag.start_x);
        if (next - self.width).abs() < f32::EPSILON {
            return false;
        }
        self.width = next;
        true
    }

    pub(crate) fn end_drag(&mut self) -> bool {
        self.drag.take().is_some()
    }
}

fn clamp_width(width: f32) -> f32 {
    if width.is_finite() {
        width.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH)
    } else {
        DEFAULT_SIDEBAR_WIDTH
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_approx(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.0001,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn initial_progress_matches_visibility() {
        assert_eq!(SidebarState::new(true, 230.0).reveal_progress, 1.0);
        assert_eq!(SidebarState::new(false, 230.0).reveal_progress, 0.0);
    }

    #[test]
    fn reveal_and_hide_complete_at_their_durations() {
        let now = Instant::now();
        let mut state = SidebarState::new(false, 230.0);
        state.set_visible(true, now, false);
        assert!(state.is_transitioning());
        state.advance_transition(now + REVEAL_DURATION);
        assert_approx(state.reveal_progress, 1.0);
        assert!(!state.is_transitioning());

        state.set_visible(false, now + REVEAL_DURATION, false);
        assert!(state.is_transitioning());
        state.advance_transition(now + REVEAL_DURATION + HIDE_DURATION);
        assert_approx(state.reveal_progress, 0.0);
        assert!(!state.is_transitioning());
    }

    #[test]
    fn reversing_transition_continues_from_current_progress() {
        let now = Instant::now();
        let mut state = SidebarState::new(false, 230.0);
        state.set_visible(true, now, false);
        state.advance_transition(now + Duration::from_millis(80));
        let before_reverse = state.reveal_progress;
        assert!(before_reverse > 0.0 && before_reverse < 1.0);

        let reversed_at = now + Duration::from_millis(80);
        state.set_visible(false, reversed_at, false);
        assert_approx(state.reveal_progress, before_reverse);
        state.advance_transition(reversed_at + HIDE_DURATION);
        assert_approx(state.reveal_progress, 0.0);
    }

    #[test]
    fn reduced_motion_completes_immediately() {
        let now = Instant::now();
        let mut state = SidebarState::new(false, 230.0);
        state.set_visible(true, now, true);
        assert_eq!(state.reveal_progress, 1.0);
        assert!(!state.is_transitioning());
        state.set_visible(false, now, true);
        assert_eq!(state.reveal_progress, 0.0);
        assert!(!state.is_transitioning());
    }

    #[test]
    fn width_is_clamped_on_restore_and_drag() {
        assert_eq!(SidebarState::new(true, 10.0).width, MIN_SIDEBAR_WIDTH);
        assert_eq!(
            SidebarState::new(true, f32::NAN).width,
            DEFAULT_SIDEBAR_WIDTH
        );
        let mut state = SidebarState::new(true, 230.0);
        state.start_drag(100.0);
        assert!(state.update_drag(1000.0));
        assert_eq!(state.width, MAX_SIDEBAR_WIDTH);
        assert!(state.end_drag());
        assert!(!state.end_drag());
    }

    #[test]
    fn drag_without_start_is_noop_and_same_width_reports_no_change() {
        let mut state = SidebarState::new(true, 230.0);
        assert!(!state.update_drag(500.0));
        state.start_drag(100.0);
        // Moving back to the start x keeps the width unchanged.
        assert!(!state.update_drag(100.0));
        assert_eq!(state.width, 230.0);
        assert!(state.end_drag());
    }

    #[test]
    fn drag_clamps_to_minimum_and_preserves_visibility() {
        let mut state = SidebarState::new(false, 300.0);
        state.start_drag(400.0);
        assert!(state.update_drag(0.0));
        assert_eq!(state.width, MIN_SIDEBAR_WIDTH);
        assert!(!state.visible);
    }
}
