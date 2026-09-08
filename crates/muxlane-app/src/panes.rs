//! Pane tree, tab navigation, split layout, and divider drag handling.
use crate::app::MuxlaneApp;
use crate::i18n;
use crate::icons::*;
use crate::theme::Theme;
use crate::ui_scale::px as ui_px;
use crate::widgets::*;
use crate::workspace::ProjectKey;
use gpui::{
    canvas, div, prelude::*, relative, rgba, Context, MouseButton, Pixels, Point, SharedString,
    Window,
};
use muxlane_core::model::AgentId;
use muxlane_core::{PaneId, PaneNode, SplitAxis};
use std::sync::Arc;

#[derive(Clone)]
struct DragTab {
    agent: AgentId,
    from_pane: PaneId,
}

#[derive(Clone)]
pub(super) struct DividerDrag;

#[derive(Default)]
pub(crate) struct PaneTabScroll {
    pub(super) scroll: gpui::ScrollHandle,
    active: Option<AgentId>,
}

#[derive(Clone)]
pub(crate) struct SplitDrag {
    split_id: String,
    divider: usize,
    pub(super) axis: SplitAxis,
    start: Point<Pixels>,
    sizes: Vec<f32>,
}

impl MuxlaneApp {
    pub(crate) fn activate_tab(&mut self, pane: &PaneId, agent: &AgentId, cx: &mut Context<Self>) {
        self.pane_tree.open_tab(pane, agent.clone());
        // 跨 pane 的显式导航必须揭示目标 pane；同 pane 切 tab 保留 zoom。
        if self
            .maximized_pane
            .as_ref()
            .is_some_and(|maximized| maximized != pane)
        {
            self.maximized_pane = None;
        }
        self.active_pane = pane.clone();
        self.active = Some(agent.clone());
        // 清理当前 agent 的 Toast 与标记通知已读
        self.notifications
            .update(cx, |center, cx| center.mark_agent_read(agent, cx));
        if let Some(a) = self.last_snapshot.agent_mut(agent) {
            a.seen = true;
            if a.status.is_finished() {
                a.status = muxlane_core::model::AgentStatus::Idle;
            }
        }
        // 查看本地 Done agent 后回到 Idle。
        if self.last_snapshot.agent(agent).is_some() {
            let server = Arc::clone(&self.server);
            let agent = agent.clone();
            server.rt_spawn({
                let server = Arc::clone(&server);
                async move { server.mark_seen(&agent).await }
            });
        }
        self.persist();
    }

    pub(super) fn select_tab_n(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(group) = self.pane_tree.group(&self.active_pane) {
            if let Some(agent) = group.tabs.get(index).cloned() {
                let pane = self.active_pane.clone();
                self.activate_agent(&pane, &agent, window, cx);
            }
        }
    }

    pub(super) fn next_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_sidebar_session(true, window, cx);
    }

    pub(super) fn prev_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_sidebar_session(false, window, cx);
    }

    fn navigate_sidebar_session(
        &mut self,
        next: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.adjacent_sidebar_session(next) else {
            return;
        };
        self.collapsed_machines.remove(&target.machine_collapse_key);
        self.collapsed_projects.remove(&target.project_collapse_key);
        self.jump_to_agent(&target.agent, window, cx);
    }

    pub(super) fn close_tab(
        &mut self,
        pane: &PaneId,
        agent: &AgentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let clears_zoom = self.maximized_pane.as_ref() == Some(pane)
            && self
                .pane_tree
                .group(pane)
                .and_then(|group| group.active.as_ref())
                == Some(agent);
        let remote = self.remote_snaps.values().any(|snapshot| {
            snapshot
                .agents
                .iter()
                .any(|candidate| &candidate.id == agent)
        });
        self.delete_session(agent, remote, window, cx);
        if clears_zoom {
            // 关闭 zoom owner 的选中会话不能把 zoom 转移给下一个会话。
            self.maximized_pane = None;
        }
    }

    fn move_dragged_tab(
        &mut self,
        drag: &DragTab,
        target_pane: &PaneId,
        target_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .pane_tree
            .move_tab(&drag.from_pane, target_pane, &drag.agent, target_index)
        {
            self.activate_agent(target_pane, &drag.agent, window, cx);
        }
    }

    pub(crate) fn selected_terminal_preset(&self) -> muxlane_core::AgentPreset {
        self.presets
            .iter()
            .find(|preset| preset.id == self.default_terminal_preset)
            .cloned()
            .unwrap_or_else(|| {
                muxlane_core::builtin_presets(muxlane_term::default_shell_program()).remove(0)
            })
    }

    pub(super) fn spawn_shell_for_pane(
        &mut self,
        pane: &PaneId,
        split_axis: Option<SplitAxis>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane_agent = self
            .pane_tree
            .group(pane)
            .and_then(|group| group.active.clone().or_else(|| group.tabs.first().cloned()));
        let target_key = self
            .workspace
            .current_project()
            .filter(|_| self.workspace.enabled())
            .cloned()
            .or_else(|| {
                pane_agent
                    .as_ref()
                    .and_then(|agent| self.project_key_for_agent(agent))
            })
            .or_else(|| {
                self.active
                    .as_ref()
                    .and_then(|agent| self.project_key_for_agent(agent))
            })
            .or_else(|| {
                self.last_snapshot
                    .projects
                    .first()
                    .map(|project| ProjectKey::new(self.local_machine_id(), project.id.clone()))
            });
        let Some(target_key) = target_key else { return };
        let preset = self.selected_terminal_preset();
        if target_key.machine_id != self.local_machine_id() {
            let Some(host) = self.remote_host_for_key(&target_key) else {
                return;
            };
            self.spawn_remote_agent(
                crate::remotes::RemoteAgentSpawnRequest {
                    host,
                    project: target_key.project_id,
                    preset: Some(preset),
                    preferred_pane: Some(pane.clone()),
                    split_axis,
                },
                window,
                cx,
            );
            return;
        }
        let Some(project) = self.last_snapshot.project(&target_key.project_id).cloned() else {
            return;
        };
        let params = crate::sessions::local_preset_params(&preset, &project);
        let server = Arc::clone(&self.server);
        let pane = self.capture_spawn_target(&target_key, Some(pane));
        let collapse_key = format!("local:{}", project.id);
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let agent = server.spawn_agent(params).await?;
                    let session = server
                        .session(&agent.id)
                        .await
                        .ok_or_else(|| anyhow::anyhow!("spawned agent has no session"))?;
                    Ok::<_, anyhow::Error>((agent, session))
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok((agent, session)) => {
                    let agent_id = agent.id.clone();
                    let term = Self::create_local_term(
                        agent_id.clone(),
                        session,
                        &this.font_family,
                        Theme::for_mode(this.theme_mode),
                        this.osc52_clipboard_enabled,
                        cx,
                    );
                    this.collapsed_projects.remove(&collapse_key);
                    this.terms.insert(agent_id.clone(), term);
                    this.jump_to_project_if_needed(&target_key, cx);
                    this.place_async_agent(
                        &target_key,
                        agent_id,
                        Some(pane),
                        split_axis,
                        window,
                        cx,
                    );
                }
                Err(error) => {
                    this.notifications.update(cx, |center, cx| {
                        center.show_error(
                            i18n::text(this.language, "error.create_session")
                                .replace("{error}", &error.to_string()),
                            cx,
                        )
                    });
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn new_contextual_tab(
        &mut self,
        pane: &PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_shell_tab(pane, window, cx);
    }

    pub(super) fn new_shell_tab(
        &mut self,
        pane: &PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.spawn_shell_for_pane(pane, None, window, cx);
    }

    /// 显式分屏：新 pane 使用默认终端预设，不复制当前 agent 类型。
    pub(super) fn split_pane(
        &mut self,
        pane: &PaneId,
        axis: SplitAxis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.spawn_shell_for_pane(pane, Some(axis), window, cx);
    }

    pub(crate) fn toggle_maximize(&mut self, pane: &PaneId, cx: &mut Context<Self>) {
        self.maximized_pane = if self.maximized_pane.as_ref() == Some(pane) {
            None
        } else {
            Some(pane.clone())
        };
        self.persist();
        cx.notify();
    }

    pub(super) fn close_split_pane(
        &mut self,
        pane: &PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pane_tree.leaf_count() <= 1 {
            return;
        }
        let agents = self
            .pane_tree
            .group(pane)
            .map(|group| group.tabs.clone())
            .unwrap_or_default();
        if let Some(next) = self.pane_tree.without_pane(pane) {
            self.pane_tree = next;
            for agent in agents {
                let remote = self.remote_snaps.values().any(|snapshot| {
                    snapshot
                        .agents
                        .iter()
                        .any(|candidate| candidate.id == agent)
                });
                self.delete_session(&agent, remote, window, cx);
            }
            if let Ok(mut metrics) = self.split_metrics.lock() {
                metrics.clear();
            }
            self.maximized_pane = None;
            self.active_pane = self.pane_tree.first_pane_id();
            self.active = self
                .pane_tree
                .group(&self.active_pane)
                .and_then(|g| g.active.clone());
            if let Some(active) = self.active.clone() {
                let pane = self.active_pane.clone();
                self.activate_agent(&pane, &active, window, cx);
            }
            self.persist();
            cx.notify();
        }
    }

    pub(super) fn start_split_drag(
        &mut self,
        split_id: String,
        divider: usize,
        axis: SplitAxis,
        start: Point<Pixels>,
    ) {
        if let Some((_, sizes)) = self.pane_tree.split_info(&split_id) {
            self.split_drag = Some(SplitDrag {
                split_id,
                divider,
                axis,
                start,
                sizes,
            });
        }
    }

    pub(super) fn update_split_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.split_drag.clone() else {
            return;
        };
        let extent = self
            .split_metrics
            .lock()
            .ok()
            .and_then(|m| m.get(&drag.split_id).copied())
            .unwrap_or(1.0)
            .max(1.0);
        if drag.divider + 1 >= drag.sizes.len() {
            return;
        }
        let delta_px = if drag.axis == SplitAxis::Horizontal {
            f32::from(position.x - drag.start.x)
        } else {
            f32::from(position.y - drag.start.y)
        };
        let delta = delta_px / extent;
        let pair_total = drag.sizes[drag.divider] + drag.sizes[drag.divider + 1];
        if !pair_total.is_finite() || pair_total <= 0.0 {
            return;
        }
        let min = 0.05_f32.min(pair_total / 2.0);
        let left = (drag.sizes[drag.divider] + delta).clamp(min, pair_total - min);
        let mut next = drag.sizes.clone();
        next[drag.divider] = left;
        next[drag.divider + 1] = pair_total - left;
        if self.pane_tree.update_split_sizes(&drag.split_id, next) {
            cx.notify();
        }
    }

    pub(super) fn end_split_drag(&mut self) {
        if self.split_drag.take().is_some() {
            self.persist();
        }
    }

    pub(super) fn render_pane_node(
        &mut self,
        node: PaneNode,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = Theme::for_mode(self.theme_mode);
        match node {
            PaneNode::Split {
                id,
                axis,
                children,
                sizes,
            } => {
                let metrics = Arc::clone(&self.split_metrics);
                let metric_id = id.clone();
                let num_children = children.len();
                let mut container = div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .when(axis == SplitAxis::Horizontal, |el| el.flex_row())
                    .when(axis == SplitAxis::Vertical, |el| el.flex_col())
                    .child(
                        canvas(
                            move |bounds, _window, _cx| {
                                let extent = if axis == SplitAxis::Horizontal {
                                    f32::from(bounds.size.width)
                                } else {
                                    f32::from(bounds.size.height)
                                };
                                if let Ok(mut map) = metrics.lock() {
                                    map.insert(metric_id.clone(), extent);
                                }
                            },
                            |_bounds, _state, _window, _cx| {},
                        )
                        .absolute()
                        .size_full(),
                    );
                for (index, child) in children.into_iter().enumerate() {
                    if index > 0 {
                        let split_id = id.clone();
                        container = container.child(
                            div()
                                .id(gpui::ElementId::Name(
                                    format!("divider-{id}-{index}").into(),
                                ))
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .when(axis == SplitAxis::Horizontal, |el| {
                                    el.w(ui_px(2.))
                                        .h_full()
                                        .ml(ui_px(-1.))
                                        .mr(ui_px(-1.))
                                        .cursor_col_resize()
                                })
                                .when(axis == SplitAxis::Vertical, |el| {
                                    el.h(ui_px(2.))
                                        .w_full()
                                        .mt(ui_px(-1.))
                                        .mb(ui_px(-1.))
                                        .cursor_row_resize()
                                })
                                .on_click(cx.listener({
                                    let split_id = split_id.clone();
                                    move |this, ev: &gpui::ClickEvent, _window, cx| {
                                        if ev.click_count() >= 2 {
                                            let equal_size = 1.0 / num_children.max(1) as f32;
                                            let next_sizes = vec![equal_size; num_children];
                                            if this
                                                .pane_tree
                                                .update_split_sizes(&split_id, next_sizes)
                                            {
                                                this.persist();
                                                cx.notify();
                                            }
                                        }
                                    }
                                }))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(
                                        move |this, ev: &gpui::MouseDownEvent, _window, _cx| {
                                            this.start_split_drag(
                                                split_id.clone(),
                                                index - 1,
                                                axis,
                                                ev.position,
                                            );
                                        },
                                    ),
                                )
                                .on_drag(DividerDrag, |_, _offset, _window, cx| {
                                    cx.new(|_| DividerDragGhost)
                                })
                                .on_mouse_move(cx.listener(
                                    |this, ev: &gpui::MouseMoveEvent, _window, cx| {
                                        if this.split_drag.is_some() {
                                            this.update_split_drag(ev.position, cx);
                                        }
                                    },
                                ))
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(|this, _ev, _window, _cx| this.end_split_drag()),
                                ),
                        );
                    }
                    let rendered = self.render_pane_node(child, cx);
                    let share = sizes
                        .get(index)
                        .copied()
                        .unwrap_or(1.0 / sizes.len().max(1) as f32);
                    container = container.child(
                        div()
                            .flex()
                            .flex_basis(relative(share))
                            .flex_grow_0()
                            .flex_shrink_1()
                            .min_w_0()
                            .min_h_0()
                            .child(rendered),
                    );
                }
                container.into_any_element()
            }
            PaneNode::Leaf { group } => {
                let pane_id = group.id.clone();
                let active_id = group.active.clone();
                let is_focused_pane = group.id == self.active_pane;
                let tab_scroll = self.pane_tab_scrolls.entry(pane_id.clone()).or_default();
                if tab_scroll.active != active_id {
                    if let Some(index) = group
                        .tabs
                        .iter()
                        .position(|id| Some(id) == active_id.as_ref())
                    {
                        tab_scroll.scroll.scroll_to_item(index);
                    }
                    tab_scroll.active = active_id.clone();
                }
                let scroll = tab_scroll.scroll.clone();
                let mut tabs = div()
                    .id(gpui::ElementId::Name(format!("pane-tabs-{pane_id}").into()))
                    .track_scroll(&scroll)
                    .flex()
                    .flex_shrink_1()
                    .min_w_0()
                    .items_center()
                    .h_full()
                    .overflow_x_scroll();
                for tab_id in group.tabs.clone() {
                    let is_active = active_id.as_ref() == Some(&tab_id);
                    let pane_for_tab = pane_id.clone();
                    let summary = self.session_summary(&tab_id);
                    let status = summary
                        .as_ref()
                        .map(|(_, status, _)| *status)
                        .unwrap_or(muxlane_core::model::AgentStatus::Idle);
                    let seen = summary.as_ref().map(|(_, _, seen)| *seen).unwrap_or(true)
                        || self.active.as_ref() == Some(&tab_id);
                    let att = compute_attention_style(status, seen, theme);
                    let tab_title = summary
                        .as_ref()
                        .map(|(title, _, _)| title.trim())
                        .filter(|title| !title.is_empty())
                        .unwrap_or("session")
                        .to_string();
                    let drag_label: SharedString = format!("Terminal · {}", status.as_str()).into();
                    let tab = div()
                        .id(gpui::ElementId::Name(format!("tab-{tab_id}").into()))
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1p5()
                        .h_full()
                        .px_2()
                        .text_size(ui_px(11.5))
                        .when(att.is_alerting && att.text_color.is_some(), |el| {
                            el.text_color(rgba(att.text_color.unwrap()))
                        })
                        .when(!att.is_alerting, |el| {
                            el.text_color(rgba(if is_active { theme.fg0 } else { theme.fg1 }))
                        })
                        .when(att.is_alerting && att.bg_color.is_some(), |el| {
                            el.bg(rgba(att.bg_color.unwrap()))
                        })
                        .when(is_active, |el| {
                            el.bg(rgba(theme.bg0))
                                .border_t_2()
                                .border_color(rgba(theme.accent))
                        })
                        .border_r_1()
                        .border_color(rgba(theme.line))
                        .when(!is_active, |el| el.hover(|s| s.bg(rgba(theme.bg2))))
                        .on_click(cx.listener({
                            let id = tab_id.clone();
                            let pane = pane_for_tab.clone();
                            move |this, _ev, window, cx| {
                                this.activate_agent(&pane, &id, window, cx);
                            }
                        }))
                        // 鼠标中键直接关闭 Tab
                        .on_mouse_down(
                            MouseButton::Middle,
                            cx.listener({
                                let id = tab_id.clone();
                                let pane = pane_for_tab.clone();
                                move |this, _ev, window, cx| {
                                    cx.stop_propagation();
                                    this.close_tab(&pane, &id, window, cx);
                                }
                            }),
                        )
                        .on_drag(
                            DragTab {
                                agent: tab_id.clone(),
                                from_pane: pane_for_tab.clone(),
                            },
                            {
                                let label = drag_label;
                                move |_, offset, _, cx| {
                                    let label = label.clone();
                                    cx.new(move |_| DragGhost {
                                        label,
                                        offset,
                                        theme,
                                    })
                                }
                            },
                        )
                        // drop on tab = insert before it（同组重排/跨 pane）
                        .on_drop::<DragTab>(cx.listener({
                            let pane = pane_id.clone();
                            let slot = group.tabs.iter().position(|a| a == &tab_id).unwrap_or(0);
                            move |this, drag: &DragTab, window, cx| {
                                this.move_dragged_tab(drag, &pane, slot, window, cx)
                            }
                        }))
                        .child(div().flex_none().child(render_status_indicator(
                            status,
                            format!("pane-{pane_id}-agent-{tab_id}"),
                            theme,
                        )))
                        .child(
                            div()
                                .max_w(ui_px(180.))
                                .overflow_hidden()
                                .text_ellipsis()
                                .line_height(ui_px(14.))
                                .child(tab_title.clone()),
                        )
                        .tooltip(hover_tip(tab_title.clone(), theme))
                        .child(
                            semantic_button(
                                gpui::ElementId::Name(format!("tab-close-{tab_id}").into()),
                                format!("Close tab {tab_title}"),
                                theme,
                            )
                            .text_color(rgba(theme.fg2))
                            .flex_none()
                            .px_1()
                            .hover(|s| s.bg(rgba(theme.bg2)).text_color(rgba(theme.fg0)))
                            .tooltip(hover_tip(format!("Close tab {tab_title}"), theme))
                            .on_click(cx.listener({
                                let id = tab_id.clone();
                                let pane = pane_id.clone();
                                move |this, _ev, window, cx| {
                                    cx.stop_propagation();
                                    this.close_tab(&pane, &id, window, cx);
                                }
                            }))
                            .child("×"),
                        );
                    tabs = tabs.child(tab);
                }
                let new_tab_label =
                    i18n::text(self.language, "settings.shortcut.new_tab").to_string();
                let new_tab = div().flex().flex_none().h_full().items_center().child(
                    semantic_button(
                        gpui::ElementId::Name(format!("new-tab-{pane_id}").into()),
                        new_tab_label.clone(),
                        theme,
                    )
                    .when(cfg!(test), |button| {
                        button.debug_selector(|| "ux-new-tab".into())
                    })
                    .w(ui_px(28.))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(ui_px(14.))
                    .text_color(rgba(theme.fg1))
                    .hover(|s| s.bg(rgba(theme.bg2)).text_color(rgba(theme.fg0)))
                    .tooltip(hover_tip(new_tab_label, theme))
                    .on_click(cx.listener({
                        let pane = pane_id.clone();
                        move |this, _ev, window, cx| this.new_contextual_tab(&pane, window, cx)
                    }))
                    .child(panel_icon(PLUS_ICON, theme.fg1)),
                );
                // 显式分屏/最大化 controls：没有隐式 split。
                let tools = div()
                    .flex()
                    .flex_none()
                    .h_full()
                    .items_center()
                    .when(cfg!(test), |tools| {
                        tools.debug_selector(|| "ux-pane-tools".into())
                    })
                    .child(
                        semantic_button(
                            gpui::ElementId::Name(format!("split-h-{pane_id}").into()),
                            i18n::text(self.language, "palette.horizontal_split"),
                            theme,
                        )
                        .w(ui_px(28.))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(rgba(theme.bg2)).text_color(rgba(theme.fg0)))
                        .tooltip(hover_tip(
                            i18n::text(self.language, "palette.horizontal_split"),
                            theme,
                        ))
                        .on_click(cx.listener({
                            let pane = pane_id.clone();
                            move |this, _ev, window, cx| {
                                this.split_pane(&pane, SplitAxis::Horizontal, window, cx)
                            }
                        }))
                        .child(panel_icon(SPLIT_HORIZONTAL_ICON, theme.fg1)),
                    )
                    .child(
                        semantic_button(
                            gpui::ElementId::Name(format!("split-v-{pane_id}").into()),
                            i18n::text(self.language, "palette.vertical_split"),
                            theme,
                        )
                        .w(ui_px(28.))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(rgba(theme.bg2)).text_color(rgba(theme.fg0)))
                        .tooltip(hover_tip(
                            i18n::text(self.language, "palette.vertical_split"),
                            theme,
                        ))
                        .on_click(cx.listener({
                            let pane = pane_id.clone();
                            move |this, _ev, window, cx| {
                                this.split_pane(&pane, SplitAxis::Vertical, window, cx)
                            }
                        }))
                        .child(panel_icon(SPLIT_VERTICAL_ICON, theme.fg1)),
                    )
                    .child(
                        semantic_button(
                            gpui::ElementId::Name(format!("maximize-{pane_id}").into()),
                            i18n::text(self.language, "palette.maximize"),
                            theme,
                        )
                        .w(ui_px(28.))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(rgba(theme.bg2)).text_color(rgba(theme.fg0)))
                        .tooltip(hover_tip(
                            i18n::text(self.language, "palette.maximize"),
                            theme,
                        ))
                        .on_click(cx.listener({
                            let pane = pane_id.clone();
                            move |this, _ev, _window, cx| this.toggle_maximize(&pane, cx)
                        }))
                        .child(panel_icon(
                            if self.maximized_pane.as_ref() == Some(&pane_id) {
                                RESTORE_ICON
                            } else {
                                MAXIMIZE_ICON
                            },
                            theme.fg1,
                        )),
                    )
                    .when(self.pane_tree.leaf_count() > 1, |controls| {
                        controls.child(
                            semantic_button(
                                gpui::ElementId::Name(format!("close-pane-{pane_id}").into()),
                                i18n::text(self.language, "palette.close_split"),
                                theme,
                            )
                            .w(ui_px(28.))
                            .h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgba(theme.fg1))
                            .hover(|s| s.bg(rgba(theme.bg2)).text_color(rgba(theme.red)))
                            .tooltip(hover_tip(
                                i18n::text(self.language, "palette.close_split"),
                                theme,
                            ))
                            .on_click(cx.listener({
                                let pane = pane_id.clone();
                                move |this, _ev, window, cx| {
                                    this.close_split_pane(&pane, window, cx)
                                }
                            }))
                            .child(panel_icon(CLOSE_ICON, theme.red)),
                        )
                    });

                let tabs = div()
                    .flex()
                    .flex_none()
                    .min_w_0()
                    .h(ui_px(28.))
                    .bg(rgba(theme.bg1))
                    .border_b_1()
                    .border_color(rgba(theme.line))
                    .child(tabs)
                    .child(new_tab)
                    .child(div().flex_1())
                    .child(tools);

                let active_summary = active_id.as_ref().and_then(|id| self.session_summary(id));
                let active_status = active_summary
                    .as_ref()
                    .map(|(_, status, _)| *status)
                    .unwrap_or(muxlane_core::model::AgentStatus::Idle);
                let active_seen = active_summary
                    .as_ref()
                    .map(|(_, _, seen)| *seen)
                    .unwrap_or(true)
                    || self.active.as_ref() == active_id.as_ref();
                let pane_att = compute_attention_style(active_status, active_seen, theme);

                let content = active_id.as_ref().and_then(|id| {
                    self.terms
                        .get(id)
                        .cloned()
                        .map(IntoElement::into_any_element)
                });
                let tab_count = group.tabs.len();
                let target_pane = pane_id.clone();
                let pane_click_id = pane_id.clone();
                let pane_click_active = active_id.clone();
                let pane_drop_bg = Theme::with_alpha(theme.accent, 0x1a);
                let mut pane = div()
                    .id(gpui::ElementId::Name(
                        format!("pane-container-{}", pane_id).into(),
                    ))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    // 所有 pane 画全边框（交界处双线叠加，简单可靠）
                    .border_1()
                    .border_color(rgba(
                        if let Some(alert_color) =
                            pane_att.border_color.filter(|_| pane_att.is_alerting)
                        {
                            alert_color
                        } else if is_focused_pane {
                            theme.accent
                        } else {
                            theme.line
                        },
                    ))
                    .when(is_focused_pane || pane_att.is_alerting, |el| el.shadow_md())
                    .on_hover(cx.listener({
                        let pane_id = pane_click_id.clone();
                        move |this, hovered: &bool, window, cx| {
                            if !*hovered || this.terminal_focus_suspended(cx) {
                                return;
                            }
                            if this.active_pane != pane_id {
                                if let Some(agent_id) = this
                                    .pane_tree
                                    .group(&pane_id)
                                    .and_then(|group| group.active.clone())
                                {
                                    this.activate_agent(&pane_id, &agent_id, window, cx);
                                } else {
                                    this.active_pane = pane_id.clone();
                                    this.active = None;
                                }
                                cx.notify();
                            }
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener({
                            let pane_id = pane_click_id.clone();
                            let active_id = pane_click_active.clone();
                            move |this, _ev: &gpui::MouseDownEvent, window, cx| {
                                if let Some(agent_id) = &active_id {
                                    this.activate_agent(&pane_id, agent_id, window, cx);
                                } else {
                                    this.active_pane = pane_id.clone();
                                    this.active = None;
                                }
                                cx.notify();
                            }
                        }),
                    )
                    .on_drop::<DragTab>(cx.listener(move |this, drag: &DragTab, window, cx| {
                        this.move_dragged_tab(drag, &target_pane, tab_count, window, cx)
                    }))
                    .drag_over::<DragTab>(move |s, _, _, _| s.bg(rgba(pane_drop_bg)))
                    .child(tabs);
                if let Some(content) = content {
                    pane = pane.child(div().flex_1().min_h_0().child(content));
                } else {
                    pane = pane.child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(rgba(theme.fg2))
                            .child(i18n::text(self.language, "pane.select_agent")),
                    );
                }
                pane.into_any_element()
            }
        }
    }
}
