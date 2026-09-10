//! Per-session native windows. The main window (sidebar + content) is never resized by the app.
//!
//! A session is either *docked* (rendered as a tab inside the main window's pane tree) or
//! *detached* (its own native OS window). `FloatWindow.hidden == false` means detached; the
//! persisted `normal` rect remembers native window geometry across restarts.
use crate::app::MuxlaneApp;
use crate::term_view::TermView;
use crate::ui_scale::px as ui_px;
use crate::workspace::ProjectKey;
use gpui::{div, prelude::*, size, Context, Window};
use gpui::{App, Bounds, Entity, Focusable, WeakEntity, WindowBounds, WindowKind, WindowOptions};
use muxlane_core::model::AgentId;
use muxlane_store::FloatingLayout;
use std::collections::{BTreeMap, HashSet};

pub(crate) struct SessionWindow {
    pub(crate) term: Entity<TermView>,
    controller: WeakEntity<MuxlaneApp>,
    agent: AgentId,
}

impl SessionWindow {
    /// Global actions (split, palette, tab navigation…) belong to the main window.
    fn forward_action(&self, action: &dyn gpui::Action, cx: &mut Context<Self>) {
        let action = action.boxed_clone();
        let owner = self.controller.clone();
        let agent = self.agent.clone();
        cx.defer(move |cx| {
            let Ok(main) = owner.update(cx, |app, cx| {
                if let Some(key) = app.project_key_for_agent(&agent) {
                    app.select_project_workspace_inner(key, cx);
                }
                app.active = Some(agent);
                app.floating.main.map(|main| (main, app.focus.clone()))
            }) else {
                return;
            };
            if let Some((main, focus)) = main {
                let _ = main.update(cx, |_, window, cx| {
                    window.activate_window();
                    focus.dispatch_action(action.as_ref(), window, cx);
                });
            }
        });
    }
}

impl Render for SessionWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(ui_px(16.));
        div()
            .size_full()
            .key_context("Muxlane")
            .on_action(
                cx.listener(|this, action: &crate::actions::SplitRight, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::actions::SplitDown, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::actions::NewShellTab, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::actions::TogglePalette, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::actions::NextTab, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::actions::PreviousTab, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::actions::ToggleTheme, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::actions::SelectTab1, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::actions::SelectTab2, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::actions::SelectTab3, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::actions::SelectTab4, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::actions::SelectTab5, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::actions::SelectTab6, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::actions::SelectTab7, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::actions::SelectTab8, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::actions::SelectTab9, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(
                cx.listener(|this, action: &crate::actions::DetachAllSessions, _, cx| {
                    this.forward_action(action, cx)
                }),
            )
            .on_action(cx.listener(
                |this, action: &crate::actions::ReattachAllSessions, _, cx| {
                    this.forward_action(action, cx)
                },
            ))
            // Close shortcut inside a detached window = reattach into the main window.
            .on_action(
                cx.listener(|this, _: &crate::actions::CloseTab, window, cx| {
                    let controller = this.controller.clone();
                    let agent = this.agent.clone();
                    let bounds = window.bounds();
                    window.defer(cx, move |window, cx| {
                        let _ = controller.update(cx, |app, cx| {
                            app.record_window_bounds(&agent, bounds);
                            app.reattach_from_window(
                                &agent,
                                window.window_handle().window_id(),
                                cx,
                            );
                        });
                        window.remove_window();
                    });
                }),
            )
            .child(self.term.clone())
    }
}

#[derive(Default)]
pub(crate) struct FloatingState {
    /// Per-project detach state and remembered native window geometry.
    pub(crate) layouts: BTreeMap<ProjectKey, FloatingLayout>,
    /// Live native windows keyed by agent.
    pub(crate) windows: BTreeMap<AgentId, gpui::WindowHandle<SessionWindow>>,
    /// Which detached window currently holds OS keyboard focus (drives read-state and toasts).
    pub(crate) focused: Option<(AgentId, gpui::WindowId)>,
    pending: bool,
    requested_focus: Option<AgentId>,
    /// The main window (sidebar + content). Never resized by the app.
    pub(crate) main: Option<gpui::AnyWindowHandle>,
    #[cfg(test)]
    pub(crate) fail_next_open: bool,
}

impl FloatingState {
    pub(crate) fn from_persisted(app: &muxlane_store::PersistedApp) -> Self {
        Self {
            layouts: app
                .floating_workspaces
                .iter()
                .map(|record| {
                    let mut layout = record.layout.clone();
                    layout.normalize();
                    (
                        ProjectKey::new(
                            record.key.machine_id.clone(),
                            record.key.project_id.clone(),
                        ),
                        layout,
                    )
                })
                .collect(),
            ..Default::default()
        }
    }

    pub(crate) fn write_persisted(&self, app: &mut muxlane_store::PersistedApp) {
        app.floating_workspaces = self
            .layouts
            .iter()
            .filter(|(_, layout)| !layout.windows.is_empty())
            .map(|(key, layout)| muxlane_store::PersistedFloatingWorkspace {
                key: muxlane_store::PersistedProjectKey {
                    machine_id: key.machine_id.clone(),
                    project_id: key.project_id.clone(),
                },
                layout: layout.clone(),
            })
            .collect();
    }

    pub(crate) fn remove_agents(&mut self, removed: &HashSet<AgentId>) {
        if self
            .focused
            .as_ref()
            .is_some_and(|(agent, _)| removed.contains(agent))
        {
            self.focused = None;
        }
        for layout in self.layouts.values_mut() {
            layout.remove_agents(removed);
        }
        self.layouts.retain(|_, layout| !layout.windows.is_empty());
    }

    pub(crate) fn known_agents(&self, machine: &str) -> HashSet<AgentId> {
        self.layouts
            .iter()
            .filter(|(key, _)| key.machine_id == machine)
            .flat_map(|(_, layout)| layout.windows.iter().map(|w| w.agent.clone()))
            .collect()
    }

    pub(crate) fn is_detached(&self, agent: &AgentId) -> bool {
        self.layouts.values().any(|layout| {
            layout
                .windows
                .iter()
                .any(|w| &w.agent == agent && !w.hidden)
        })
    }

    pub(crate) fn detached_agents(&self) -> Vec<AgentId> {
        self.layouts
            .values()
            .flat_map(|layout| layout.windows.iter())
            .filter(|w| !w.hidden)
            .map(|w| w.agent.clone())
            .collect()
    }

    fn window_mut(&mut self, agent: &AgentId) -> Option<&mut muxlane_store::FloatWindow> {
        self.layouts
            .values_mut()
            .flat_map(|layout| layout.windows.iter_mut())
            .find(|w| &w.agent == agent)
    }
}

fn reconcile_windows(controller: WeakEntity<MuxlaneApp>, cx: &mut App) {
    // Never open/draw a window while the controller is borrowed: open_window draws immediately.
    let Ok((main, existing, desired, requested)) = controller.update(cx, |app, cx| {
        let desired = app.floating.detached_agents();
        let mut terminals = Vec::new();
        for agent in desired {
            app.ensure_agent_terminal(&agent, cx);
            if let Some(term) = app.terms.get(&agent) {
                let title = app
                    .session_summary(&agent)
                    .map(|s| s.0)
                    .unwrap_or_else(|| agent.clone());
                terminals.push((agent, term.clone(), title));
            }
        }
        (
            app.floating.main,
            app.floating.windows.clone(),
            terminals,
            app.floating.requested_focus.take(),
        )
    }) else {
        return;
    };
    for (agent, handle) in existing {
        if !desired.iter().any(|(id, _, _)| id == &agent) {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
            let _ = controller.update(cx, |app, _| {
                app.floating.windows.remove(&agent);
                if app
                    .floating
                    .focused
                    .as_ref()
                    .is_some_and(|(id, _)| id == &agent)
                {
                    app.floating.focused = None;
                }
            });
        } else if handle.update(cx, |_, _, _| ()).is_err() {
            // The OS closed the window behind our back: treat as reattach.
            let _ = controller.update(cx, |app, cx| {
                app.reattach_from_window(&agent, handle.window_id(), cx);
            });
        }
    }
    for (agent, term, title) in desired {
        let Ok((existing, still_detached)) = controller.update(cx, |app, _| {
            (
                app.floating.windows.get(&agent).copied(),
                app.floating.is_detached(&agent),
            )
        }) else {
            break;
        };
        if !still_detached {
            continue;
        }
        let handle = if let Some(handle) = existing {
            Some(handle)
        } else {
            let owner = controller.clone();
            let close_owner = controller.clone();
            let id = agent.clone();
            let close_id = agent.clone();
            let display_id = main.and_then(|main| {
                main.update(cx, |_, window, cx| {
                    window.display(cx).map(|display| display.id())
                })
                .ok()
                .flatten()
            });
            let saved_rect = controller
                .update(cx, |app, _| {
                    app.floating
                        .layouts
                        .values()
                        .flat_map(|l| l.windows.iter())
                        .find(|w| w.agent == agent)
                        .map(|w| w.normal)
                })
                .ok()
                .flatten();
            let bounds = match saved_rect {
                Some(rect) if rect.valid() && rect.x < 10000. && rect.y < 10000. => Bounds::new(
                    gpui::point(gpui::px(rect.x), gpui::px(rect.y)),
                    size(gpui::px(rect.width), gpui::px(rect.height)),
                ),
                _ => Bounds::centered(display_id, size(gpui::px(900.), gpui::px(600.)), cx),
            };
            #[cfg(test)]
            let fail_open = controller
                .update(cx, |app, _| {
                    std::mem::take(&mut app.floating.fail_next_open)
                })
                .unwrap_or(false);
            #[cfg(not(test))]
            let fail_open = false;
            let opened = if fail_open {
                Err(anyhow::anyhow!("injected native window open failure"))
            } else {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        kind: WindowKind::Normal,
                        display_id,
                        focus: false,
                        app_id: Some("muxlane".into()),
                        window_decorations: Some(gpui::WindowDecorations::Server),
                        ..Default::default()
                    },
                    move |window, cx| {
                        window.set_window_title(&title);
                        // Keyboard focus is local to this window; creation must not activate the OS window.
                        term.update(cx, |_, cx| cx.notify());
                        term.focus_handle(cx).focus(window, cx);
                        let window_id = window.window_handle().window_id();
                        window.on_window_should_close(cx, move |window, cx| {
                            let owner = close_owner.clone();
                            let agent = close_id.clone();
                            let bounds = window.bounds();
                            cx.defer(move |cx| {
                                let _ = owner.update(cx, |app, cx| {
                                    app.record_window_bounds(&agent, bounds);
                                    app.reattach_from_window(&agent, window_id, cx);
                                });
                            });
                            true
                        });
                        cx.new(|_| SessionWindow {
                            term,
                            controller: owner,
                            agent: id,
                        })
                    },
                )
            };
            match opened {
                Ok(handle) => {
                    let _ = controller.update(cx, |app, _| {
                        app.floating.windows.insert(agent.clone(), handle);
                    });
                    Some(handle)
                }
                Err(error) => {
                    // A failed open falls back to docked; do not retry every frame.
                    let _ = controller.update(cx, |app, cx| {
                        app.reattach_session(&agent, cx);
                        app.notifications.update(cx, |center, cx| {
                            center.show_error(format!("Could not open session window: {error}"), cx)
                        });
                    });
                    None
                }
            }
        };
        if requested.as_ref() == Some(&agent) {
            if let Some(handle) = handle {
                let focus_owner = controller.clone();
                let _ = handle.update(cx, |root, window, cx| {
                    window.activate_window();
                    root.term.focus_handle(cx).focus(window, cx);
                    let _ = focus_owner.update(cx, |app, cx| app.focus_agent(&agent, window, cx));
                });
            }
        }
    }
    let _ = controller.update(cx, |app, _| {
        app.floating.pending = false;
    });
}

impl MuxlaneApp {
    pub(crate) fn schedule_session_windows(&mut self, cx: &mut Context<Self>) {
        if self.floating.pending {
            return;
        }
        self.floating.pending = true;
        let controller = cx.entity().downgrade();
        cx.defer(move |cx| reconcile_windows(controller, cx));
    }

    pub(crate) fn is_detached(&self, agent: &AgentId) -> bool {
        self.floating.is_detached(agent)
    }

    /// Pop the session out of the main window into its own native window.
    pub(crate) fn detach_session(&mut self, agent: &AgentId, cx: &mut Context<Self>) {
        let Some(key) = self.project_key_for_agent(agent) else {
            return;
        };
        self.remove_from_all_layouts(agent);
        self.floating
            .layouts
            .entry(key)
            .or_default()
            .activate(agent.clone());
        if self.active.as_ref() == Some(agent) {
            self.active = self
                .pane_tree
                .group(&self.active_pane)
                .and_then(|group| group.active.clone());
        }
        self.ensure_agent_terminal(agent, cx);
        self.floating.requested_focus = Some(agent.clone());
        self.mark_agent_seen(agent, cx);
        self.schedule_session_windows(cx);
        self.persist();
        cx.notify();
    }

    /// A detached session must not survive in any saved per-project layout, otherwise
    /// switching projects restores it as a tab while its native window is still open and
    /// the two windows fight over the same terminal.
    fn remove_from_all_layouts(&mut self, agent: &AgentId) {
        if let Some(pane) = self.pane_tree.pane_for_agent(agent) {
            let closes_maximized = self.maximized_pane.as_ref() == Some(&pane);
            self.pane_tree.close_tab(&pane, agent);
            if closes_maximized {
                self.maximized_pane = None;
            }
            if self.pane_tree.group(&self.active_pane).is_none() {
                self.active_pane = self.pane_tree.first_pane_id();
                self.maximized_pane = None;
            }
        }
        self.workspace
            .remove_agents(&HashSet::from([agent.clone()]));
    }

    /// Bring the session back into the main window's pane tree.
    pub(crate) fn reattach_session(&mut self, agent: &AgentId, cx: &mut Context<Self>) {
        if let Some(w) = self.floating.window_mut(agent) {
            w.hidden = true;
        }
        for layout in self.floating.layouts.values_mut() {
            layout.normalize();
        }
        if self
            .floating
            .focused
            .as_ref()
            .is_some_and(|(id, _)| id == agent)
        {
            self.floating.focused = None;
        }
        if self.dock_into_owning_project(agent) {
            self.active = Some(agent.clone());
        }
        self.schedule_session_windows(cx);
        self.persist();
        cx.notify();
    }

    /// Put a session back as a tab in the layout that owns it. With per-project workspaces
    /// the session goes into *its* project's saved layout, not whatever is on screen.
    /// Returns true when it landed in the visible pane tree.
    fn dock_into_owning_project(&mut self, agent: &AgentId) -> bool {
        let Some(key) = self.project_key_for_agent(agent) else {
            return false;
        };
        let visible = !self.workspace.enabled() || self.workspace.current_project() == Some(&key);
        if visible {
            if self.pane_tree.pane_for_agent(agent).is_none() {
                let pane = self.active_pane.clone();
                self.pane_tree.open_tab(&pane, agent.clone());
            }
        } else {
            self.workspace
                .place_agent_in_project(&key, agent.clone(), None, None);
        }
        visible
    }

    /// Called when a native window closes (close button, Ctrl+W inside it, or OS-side close).
    pub(crate) fn reattach_from_window(
        &mut self,
        agent: &AgentId,
        window_id: gpui::WindowId,
        cx: &mut Context<Self>,
    ) {
        if !self
            .floating
            .windows
            .get(agent)
            .is_some_and(|h| h.window_id() == window_id)
        {
            return;
        }
        self.floating.windows.remove(agent);
        self.reattach_session(agent, cx);
    }

    /// Detach every docked session across all projects.
    pub(crate) fn detach_all_sessions(&mut self, cx: &mut Context<Self>) {
        let agents: Vec<(ProjectKey, AgentId)> = self
            .all_known_agents()
            .into_iter()
            .filter(|(_, agent)| !self.is_detached(agent))
            .collect();
        if agents.is_empty() {
            return;
        }
        for (key, agent) in &agents {
            self.remove_from_all_layouts(agent);
            self.floating
                .layouts
                .entry(key.clone())
                .or_default()
                .activate(agent.clone());
            self.ensure_agent_terminal(agent, cx);
        }
        if let Some(active) = self.active.clone() {
            if agents.iter().any(|(_, agent)| agent == &active) {
                self.floating.requested_focus = Some(active);
            }
        }
        self.active = self
            .pane_tree
            .group(&self.active_pane)
            .and_then(|group| group.active.clone());
        self.schedule_session_windows(cx);
        self.persist();
        cx.notify();
    }

    /// Bring every detached session back into the main window.
    pub(crate) fn reattach_all_sessions(&mut self, cx: &mut Context<Self>) {
        let agents = self.floating.detached_agents();
        if agents.is_empty() {
            return;
        }
        let mut last_visible = None;
        for agent in &agents {
            if let Some(w) = self.floating.window_mut(agent) {
                w.hidden = true;
            }
            if self.dock_into_owning_project(agent) {
                last_visible = Some(agent.clone());
            }
        }
        for layout in self.floating.layouts.values_mut() {
            layout.normalize();
        }
        self.floating.focused = None;
        if let Some(agent) = last_visible {
            self.active = Some(agent);
        }
        self.schedule_session_windows(cx);
        self.persist();
        cx.notify();
    }

    pub(crate) fn has_any_docked_session(&self) -> bool {
        self.all_known_agents()
            .iter()
            .any(|(_, agent)| !self.is_detached(agent))
    }

    pub(crate) fn all_known_agents(&self) -> Vec<(ProjectKey, AgentId)> {
        let local = self.local_machine_id();
        let mut out: Vec<(ProjectKey, AgentId)> = self
            .last_snapshot
            .agents
            .iter()
            .map(|agent| {
                (
                    ProjectKey::new(local.clone(), agent.project.clone()),
                    agent.id.clone(),
                )
            })
            .collect();
        for snapshot in self.remote_snaps.values() {
            let Some(machine) = snapshot.machine.as_ref() else {
                continue;
            };
            out.extend(snapshot.agents.iter().map(|agent| {
                (
                    ProjectKey::new(machine.machine_id.clone(), agent.project.clone()),
                    agent.id.clone(),
                )
            }));
        }
        out
    }

    /// Focus a detached session's native window (sidebar click / Alt+N on a detached session).
    pub(crate) fn focus_detached_window(&mut self, agent: &AgentId, cx: &mut Context<Self>) {
        self.floating.requested_focus = Some(agent.clone());
        self.mark_agent_seen(agent, cx);
        self.schedule_session_windows(cx);
        cx.notify();
    }

    pub(crate) fn record_window_bounds(&mut self, agent: &AgentId, bounds: Bounds<gpui::Pixels>) {
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        let x = f32::from(bounds.origin.x);
        let y = f32::from(bounds.origin.y);
        if width >= 280. && height >= 160. && x.is_finite() && y.is_finite() {
            if let Some(w) = self.floating.window_mut(agent) {
                w.normal = muxlane_store::FloatRect {
                    x,
                    y,
                    width,
                    height,
                };
            }
        }
    }

    /// Which agent "the user is looking at" for read-state and toast suppression.
    pub(crate) fn notification_focused_agent(&self) -> Option<&AgentId> {
        self.floating
            .focused
            .as_ref()
            .map(|(agent, _)| agent)
            .or(self.active.as_ref())
    }

    pub(crate) fn handle_terminal_focus(
        &mut self,
        event: &crate::term_view::TermFocusEvent,
        cx: &mut Context<Self>,
    ) {
        let identity = (event.agent.clone(), event.window);
        if !event.focused {
            if self.floating.focused.as_ref() == Some(&identity) {
                self.floating.focused = None;
                cx.notify();
            }
            return;
        }
        if !self
            .floating
            .windows
            .get(&event.agent)
            .is_some_and(|h| h.window_id() == event.window)
        {
            return;
        }
        self.floating.focused = Some(identity);
        self.mark_agent_seen(&event.agent, cx);
    }

    /// Placeholder for the content area when every session of the project is detached.
    pub(crate) fn render_detached_placeholder(&self) -> gpui::AnyElement {
        let theme = crate::theme::Theme::for_mode(self.theme_mode);
        // With per-project workspaces only count this project's sessions; the shared
        // layout counts everything.
        let detached = match self.workspace.current_project() {
            Some(key) if self.workspace.enabled() => self
                .floating
                .layouts
                .get(key)
                .map(|layout| layout.windows.iter().filter(|w| !w.hidden).count())
                .unwrap_or(0),
            _ => self.floating.detached_agents().len(),
        };
        div()
            .id("detached-placeholder")
            .debug_selector(|| "detached-placeholder".into())
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(theme.bg0))
            .text_color(gpui::rgba(theme.fg2))
            .text_size(ui_px(12.))
            .child(if detached > 0 {
                crate::i18n::text(self.language, "detached.placeholder")
                    .replace("{count}", &detached.to_string())
            } else {
                crate::i18n::text(self.language, "detached.empty").to_string()
            })
            .into_any_element()
    }

    pub(crate) fn close_all_session_windows(&mut self, cx: &mut Context<Self>) {
        self.floating.focused = None;
        let windows = std::mem::take(&mut self.floating.windows);
        cx.defer(move |cx| {
            for (_, handle) in windows {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        });
    }
}
