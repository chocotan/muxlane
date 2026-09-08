//! Persisted per-session detach state and remembered native window geometry (logical pixels).
use muxlane_core::model::AgentId;
use serde::{Deserialize, Serialize};

/// Kept only so older `state.json` files still deserialize. The app no longer has a global mode.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutMode {
    #[default]
    Tiled,
    Floating,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FloatRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Default for FloatRect {
    fn default() -> Self {
        Self {
            x: 24.,
            y: 24.,
            width: 640.,
            height: 400.,
        }
    }
}

impl FloatRect {
    pub fn valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .into_iter()
            .all(|v| v.is_finite() && v >= 0.)
            && self.width > 0.
            && self.height > 0.
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloatWindow {
    pub agent: AgentId,
    /// Native window geometry recorded on close; restored on the next detach.
    #[serde(default)]
    pub normal: FloatRect,
    /// `false` = detached into its own native window; `true` = docked in the main window.
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloatingLayout {
    /// Back to front. No session or terminal entity is owned by the layout.
    #[serde(default)]
    pub windows: Vec<FloatWindow>,
    /// Most recently detached/focused session of this project.
    #[serde(default)]
    pub active: Option<AgentId>,
}

impl FloatingLayout {
    pub fn normalize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.windows.retain(|w| seen.insert(w.agent.clone()));
        for window in &mut self.windows {
            if !window.normal.valid() {
                window.normal = FloatRect::default();
            }
        }
        if !self
            .windows
            .iter()
            .any(|w| !w.hidden && Some(&w.agent) == self.active.as_ref())
        {
            self.active = self
                .windows
                .iter()
                .rev()
                .find(|w| !w.hidden)
                .map(|w| w.agent.clone());
        }
        if let Some(index) = self
            .windows
            .iter()
            .position(|w| Some(&w.agent) == self.active.as_ref())
        {
            let active = self.windows.remove(index);
            self.windows.push(active);
        }
    }

    pub fn ensure(&mut self, agent: AgentId) {
        if !self.windows.iter().any(|w| w.agent == agent) {
            let offset = (self.windows.len() % 8) as f32 * 28.;
            self.windows.push(FloatWindow {
                agent,
                normal: FloatRect {
                    x: 24. + offset,
                    y: 24. + offset,
                    ..Default::default()
                },
                hidden: false,
            });
        }
        self.normalize();
    }

    /// Mark detached and bring to front.
    pub fn activate(&mut self, agent: AgentId) {
        self.ensure(agent.clone());
        let index = self.windows.iter().position(|w| w.agent == agent).unwrap();
        let mut window = self.windows.remove(index);
        window.hidden = false;
        self.windows.push(window);
        self.active = Some(agent);
    }

    /// Mark docked (kept so geometry survives for the next detach).
    pub fn hide(&mut self, agent: &AgentId) {
        if let Some(w) = self.windows.iter_mut().find(|w| &w.agent == agent) {
            w.hidden = true;
        }
        self.normalize();
    }

    pub fn remove_agents(&mut self, removed: &std::collections::HashSet<AgentId>) {
        self.windows.retain(|w| !removed.contains(&w.agent));
        self.normalize();
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedFloatingWorkspace {
    pub key: crate::PersistedProjectKey,
    #[serde(default)]
    pub layout: FloatingLayout,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_keeps_active_above_nine_overlapping_windows() {
        let mut layout = FloatingLayout::default();
        for index in 0..9 {
            layout.ensure(format!("agent-{index}"));
        }
        assert_eq!(layout.active.as_deref(), Some("agent-0"));
        assert_eq!(layout.windows.last().unwrap().agent, "agent-0");
        assert_eq!(layout.windows[7].normal, layout.windows[8].normal);
    }

    #[test]
    fn normalize_restores_active_to_front_without_changing_geometry() {
        let mut layout = FloatingLayout::default();
        for index in 0..9 {
            layout.activate(format!("agent-{index}"));
        }
        layout.active = Some("agent-2".into());
        let expected = layout.windows[2].clone();
        let mut restored: FloatingLayout =
            serde_json::from_slice(&serde_json::to_vec(&layout).unwrap()).unwrap();
        restored.normalize();
        assert_eq!(restored.windows.last(), Some(&expected));
        assert_eq!(restored.active, layout.active);
    }

    #[test]
    fn hide_restore_and_delete_keep_order_and_active_valid() {
        let mut layout = FloatingLayout::default();
        layout.activate("a".into());
        layout.activate("b".into());
        layout.hide(&"b".into());
        assert_eq!(layout.windows.len(), 2);
        assert_eq!(layout.active, Some("a".into()));
        layout.activate("b".into());
        assert!(!layout.windows[1].hidden);
        layout.remove_agents(&std::collections::HashSet::from(["b".into()]));
        assert_eq!(layout.active, Some("a".into()));
    }

    #[test]
    fn legacy_presentation_field_is_ignored_on_load() {
        // Older files carried a `presentation` snap state; it must not break deserialization.
        let json = r#"{"windows":[{"agent":"a","normal":{"x":1,"y":2,"width":300,"height":200},"hidden":false,"presentation":"left"}],"active":"a"}"#;
        let layout: FloatingLayout = serde_json::from_str(json).unwrap();
        assert_eq!(layout.windows[0].agent, "a");
        assert!(!layout.windows[0].hidden);
    }
}
