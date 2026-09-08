//! Sidebar machine tree, project/session rows, and footer controls.
use super::palette::NewSessionTarget;
use super::MuxlaneApp;
use crate::dialogs::ConnectAuthMode;
use crate::i18n;
use crate::icons::*;
use crate::menus::{
    clamp_menu_position, dismiss_context_menus, BootstrapConfirm, DeleteTarget, SessionMenu,
    TreeMenu,
};
use crate::theme::Theme;
use crate::ui_scale::px as ui_px;
use crate::widgets::*;
use crate::workspace::ProjectKey;
use gpui::{
    div, prelude::*, relative, rgba, size, Context, Focusable, MouseButton, ParentElement, Styled,
};
use muxlane_core::model::AgentId;
use std::sync::Arc;

struct SessionRowData {
    id: AgentId,
    title: String,
    status: muxlane_core::model::AgentStatus,
    seen: bool,
    remote: bool,
}

struct SidebarProject<'a> {
    project: &'a muxlane_core::model::Project,
    collapse_key: String,
    sessions: Vec<&'a muxlane_core::model::AgentInstance>,
}

struct SidebarMachine<'a> {
    remote: Option<&'a Arc<muxlane_client::RemoteHost>>,
    collapse_key: String,
    projects: Vec<SidebarProject<'a>>,
}

pub(super) struct SidebarSessionTarget {
    pub(super) agent: AgentId,
    pub(super) machine_collapse_key: String,
    pub(super) project_collapse_key: String,
}

/// 侧栏项目行拖拽负载：仅同机器内重排。
#[derive(Clone)]
pub(crate) struct DragProject {
    pub(crate) machine_id: String,
    pub(crate) project_id: String,
    pub(crate) label: String,
}

fn reorder_project_ids(ids: &mut Vec<String>, dragged: &str, target: &str) -> bool {
    if dragged == target {
        return false;
    }
    let Some(from) = ids.iter().position(|id| id == dragged) else {
        return false;
    };
    let Some(target_index) = ids.iter().position(|id| id == target) else {
        return false;
    };

    let dragged_id = ids.remove(from);
    let target_index_after_remove = if from < target_index {
        target_index - 1
    } else {
        target_index
    };
    let insertion_index = if from < target_index {
        target_index_after_remove + 1
    } else {
        target_index_after_remove
    };
    ids.insert(insertion_index, dragged_id);
    true
}

#[cfg(test)]
mod tests {
    use super::reorder_project_ids;

    #[test]
    fn reorder_project_ids_moves_projects_in_both_directions() {
        let cases = [
            ("A", "B", vec!["B", "A", "C"]),
            ("A", "C", vec!["B", "C", "A"]),
            ("C", "B", vec!["A", "C", "B"]),
            ("C", "A", vec!["C", "A", "B"]),
        ];

        for (dragged, target, expected) in cases {
            let mut ids = vec!["A".to_string(), "B".to_string(), "C".to_string()];
            assert!(reorder_project_ids(&mut ids, dragged, target));
            assert_eq!(
                ids,
                expected.into_iter().map(String::from).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn reorder_project_ids_ignores_self_and_missing_projects() {
        for (dragged, target) in [("A", "A"), ("missing", "A"), ("A", "missing")] {
            let mut ids = vec!["A".to_string(), "B".to_string(), "C".to_string()];
            assert!(!reorder_project_ids(&mut ids, dragged, target));
            assert_eq!(ids, ["A", "B", "C"]);
        }
    }
}

impl MuxlaneApp {
    // This logical tree is shared by rendering and navigation, independent of collapse state.
    fn sidebar_machines(&self) -> Vec<SidebarMachine<'_>> {
        let mut machines = vec![SidebarMachine {
            remote: None,
            collapse_key: "local".into(),
            projects: self.sidebar_projects(&self.last_snapshot, &self.local_machine_id(), None),
        }];
        for remote in &self.remotes {
            let host = &remote.cfg.name;
            machines.push(SidebarMachine {
                remote: Some(remote),
                collapse_key: format!("remote:{host}"),
                projects: self
                    .remote_snaps
                    .get(host)
                    .map(|snapshot| {
                        let machine_id = snapshot
                            .machine
                            .as_ref()
                            .map(|machine| machine.machine_id.as_str())
                            .unwrap_or_default();
                        self.sidebar_projects(snapshot, machine_id, Some(host))
                    })
                    .unwrap_or_default(),
            });
        }
        machines
    }

    fn sidebar_projects<'a>(
        &'a self,
        snapshot: &'a muxlane_core::model::Snapshot,
        machine_id: &str,
        remote_host: Option<&str>,
    ) -> Vec<SidebarProject<'a>> {
        self.ordered_projects(machine_id, &snapshot.projects)
            .into_iter()
            .map(|project| {
                let sessions = snapshot.agents_of(&project.id);
                SidebarProject {
                    project,
                    collapse_key: match remote_host {
                        Some(host) => format!("remote:{host}:{}", project.id),
                        None => format!("local:{}", project.id),
                    },
                    sessions,
                }
            })
            .collect()
    }

    pub(super) fn adjacent_sidebar_session(&self, next: bool) -> Option<SidebarSessionTarget> {
        let machines = self.sidebar_machines();
        let sessions: Vec<_> = machines
            .iter()
            .flat_map(|machine| {
                machine.projects.iter().flat_map(move |project| {
                    project
                        .sessions
                        .iter()
                        .map(move |session| (machine, project, session))
                })
            })
            .collect();
        if sessions.is_empty() {
            return None;
        }
        let current = self.active.as_ref().and_then(|active| {
            sessions
                .iter()
                .position(|(_, _, session)| &session.id == active)
        });
        let index = match (current, next) {
            (Some(index), true) => (index + 1) % sessions.len(),
            (Some(index), false) => (index + sessions.len() - 1) % sessions.len(),
            (None, true) => 0,
            (None, false) => sessions.len() - 1,
        };
        let (machine, project, session) = sessions[index];
        Some(SidebarSessionTarget {
            agent: session.id.clone(),
            machine_collapse_key: machine.collapse_key.clone(),
            project_collapse_key: project.collapse_key.clone(),
        })
    }

    /// 按自定义顺序返回项目；未记录的保持原顺序排在尾部。
    pub(crate) fn ordered_projects<'a>(
        &self,
        machine_id: &str,
        projects: &'a [muxlane_core::model::Project],
    ) -> Vec<&'a muxlane_core::model::Project> {
        let Some(order) = self.project_order.get(machine_id) else {
            return projects.iter().collect();
        };
        let mut indexed: Vec<(usize, &muxlane_core::model::Project)> =
            projects.iter().enumerate().collect();
        indexed.sort_by_key(|(original, project)| {
            let pos = order
                .iter()
                .position(|id| id == &project.id)
                .unwrap_or(usize::MAX);
            (pos, *original)
        });
        indexed.into_iter().map(|(_, project)| project).collect()
    }

    /// 把 dragged 项目移动到 target 项目附近（同机器）。
    pub(crate) fn move_project_order(
        &mut self,
        machine_id: &str,
        dragged: &str,
        target: &str,
    ) -> bool {
        let snapshot = if machine_id == self.local_machine_id() {
            Some(&self.last_snapshot)
        } else {
            self.remote_snaps.values().find(|snapshot| {
                snapshot
                    .machine
                    .as_ref()
                    .is_some_and(|machine| machine.machine_id == machine_id)
            })
        };
        let Some(snapshot) = snapshot else {
            return false;
        };
        let mut ids: Vec<String> = self
            .ordered_projects(machine_id, &snapshot.projects)
            .into_iter()
            .map(|project| project.id.clone())
            .collect();
        if !reorder_project_ids(&mut ids, dragged, target) {
            return false;
        }
        self.project_order.insert(machine_id.to_string(), ids);
        self.persist();
        true
    }
}

#[derive(Clone)]
pub(super) struct SidebarDividerDrag;

impl MuxlaneApp {
    fn render_project_row(
        &self,
        project: &muxlane_core::model::Project,
        remote_host: Option<&str>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (
            row_id,
            add_id,
            collapse_key,
            project_group,
            branch,
            delete_target,
            new_session_target,
            workspace_key,
        ) = if let Some(host) = remote_host {
            let workspace_key = self.remote_snaps.get(host).and_then(|snapshot| {
                snapshot
                    .machine
                    .as_ref()
                    .map(|machine| ProjectKey::new(machine.machine_id.clone(), project.id.clone()))
            });
            (
                format!("remote-project-row-{host}-{}", project.id),
                format!("remote-session-add-{host}-{}", project.id),
                format!("remote:{host}:{}", project.id),
                format!("remote-project-hover-{host}-{}", project.id),
                project.branch.clone(),
                DeleteTarget::RemoteProject {
                    host: host.to_string(),
                    project: project.id.clone(),
                    label: project.name.clone(),
                },
                NewSessionTarget::Remote {
                    host: host.to_string(),
                    project: project.id.clone(),
                },
                workspace_key,
            )
        } else {
            (
                format!("project-row-{}", project.id),
                format!("project-add-{}", project.id),
                format!("local:{}", project.id),
                format!("project-hover-{}", project.id),
                project
                    .branch
                    .clone()
                    .filter(|branch| !branch.trim().is_empty()),
                DeleteTarget::LocalProject {
                    project: project.id.clone(),
                    label: project.name.clone(),
                },
                NewSessionTarget::Local(project.id.clone()),
                Some(ProjectKey::new(self.local_machine_id(), project.id.clone())),
            )
        };
        let project_name = project.name.clone();
        let project_path = project.path.display().to_string();
        let collapse_key_for_click = collapse_key.clone();
        let palette_key = workspace_key.clone();
        // 仅在没有聚焦会话时高亮项目：会话行已有自己的选中态，避免双重高亮。
        let is_current = self.active.is_none()
            && self.workspace.enabled()
            && workspace_key
                .as_ref()
                .is_some_and(|key| self.workspace.current_project() == Some(key));
        let drag_payload = workspace_key.as_ref().map(|key| DragProject {
            machine_id: key.machine_id.clone(),
            project_id: key.project_id.clone(),
            label: project.name.clone(),
        });
        let drop_machine = workspace_key.as_ref().map(|key| key.machine_id.clone());
        let drop_project = workspace_key.as_ref().map(|key| key.project_id.clone());
        div()
            .id(gpui::ElementId::Name(row_id.into()))
            .flex()
            .items_center()
            .gap_1()
            .h(ui_px(28.))
            .pl_4()
            .pr_2()
            .text_size(ui_px(12.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(rgba(theme.fg0))
            .group(project_group.clone())
            .when(is_current, |row| row.bg(rgba(theme.bg2)))
            .hover(|style| style.bg(rgba(theme.bg2)))
            .when_some(drag_payload, |row, payload| {
                row.on_drag(payload, {
                    move |drag: &DragProject, offset, _, cx| {
                        let label = drag.label.clone();
                        cx.new(move |_| DragGhost {
                            label: label.into(),
                            offset,
                            theme,
                        })
                    }
                })
            })
            .on_drop::<DragProject>(cx.listener(move |this, drag: &DragProject, _window, cx| {
                if let (Some(machine), Some(target)) =
                    (drop_machine.as_ref(), drop_project.as_ref())
                {
                    if drag.machine_id == *machine
                        && this.move_project_order(machine, &drag.project_id, target)
                    {
                        cx.notify();
                    }
                }
                cx.stop_propagation();
            }))
            .tooltip(hover_tip(project_path, theme))
            .on_click(cx.listener(move |this, _event, window, cx| {
                if let Some(workspace_key) = workspace_key.clone() {
                    let is_current = this.workspace.current_project() == Some(&workspace_key);
                    if is_current && !this.collapsed_projects.remove(&collapse_key_for_click) {
                        this.collapsed_projects
                            .insert(collapse_key_for_click.clone());
                    }
                    this.select_project_workspace(workspace_key, window, cx);
                }
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    this.focus.focus(window, cx);
                    this.palette_open = false;
                    this.session_menu = None;
                    this.tree_menu = Some(TreeMenu {
                        target: delete_target.clone(),
                        position: clamp_menu_position(
                            event.position,
                            window.viewport_size(),
                            size(ui_px(190.), ui_px(150.)),
                        ),
                    });
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(panel_icon(FOLDER_ICON, theme.fg1))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(project_name),
            )
            .child(
                div()
                    .ml_auto()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .when_some(branch, |controls, branch| {
                        controls.child(
                            div()
                                .flex_none()
                                .max_w(ui_px(90.))
                                .overflow_hidden()
                                .text_ellipsis()
                                .px_1()
                                .bg(rgba(theme.bg2))
                                .text_size(ui_px(9.))
                                .font_weight(gpui::FontWeight::NORMAL)
                                .text_color(rgba(theme.fg1))
                                .child(branch),
                        )
                    })
                    .child(
                        div()
                            .id(gpui::ElementId::Name(add_id.into()))
                            .w(ui_px(20.))
                            .h(ui_px(20.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(ui_px(13.))
                            .font_weight(gpui::FontWeight::NORMAL)
                            .text_color(rgba(theme.fg1))
                            .invisible()
                            .group_hover(project_group, |style| style.visible())
                            .hover(|style| style.bg(rgba(theme.bg2)).text_color(rgba(theme.accent)))
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                cx.stop_propagation();
                                this.new_session_target = Some(new_session_target.clone());
                                this.palette_project = palette_key.clone();
                                this.palette_open = true;
                                this.palette_index = 0;
                                this.palette_scroll.scroll_to_item(0);
                                this.palette_column = super::palette::PaletteColumn::Presets;
                                this.palette_project_index = palette_key
                                    .as_ref()
                                    .and_then(|key| {
                                        this.available_project_keys()
                                            .iter()
                                            .position(|candidate| candidate == key)
                                    })
                                    .unwrap_or(0);
                                this.palette_input.update(cx, |input, cx| input.reset(cx));
                                this.palette_input.focus_handle(cx).focus(window, cx);
                                dismiss_context_menus(&mut this.session_menu, &mut this.tree_menu);
                                cx.notify();
                            }))
                            .child(panel_icon(PLUS_ICON, theme.fg1)),
                    ),
            )
    }

    fn render_sidebar_session(
        &self,
        agent: &muxlane_core::model::AgentInstance,
        remote: bool,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.render_agent_row(agent, remote, theme, cx)
            .into_any_element()
    }

    fn render_agent_row(
        &self,
        agent: &muxlane_core::model::AgentInstance,
        remote: bool,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        self.render_session_row(
            SessionRowData {
                id: agent.id.clone(),
                title: agent.title.clone(),
                status: agent.status,
                seen: agent.seen,
                remote,
            },
            theme,
            cx,
        )
    }

    fn render_session_row(
        &self,
        session: SessionRowData,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let SessionRowData {
            id,
            title,
            status,
            seen,
            remote,
        } = session;
        let active = self.active.as_ref() == Some(&id);
        let detached = self.is_detached(&id);
        let detached_marker_id = format!("detached-{id}");
        let project_key = self.project_key_for_agent(&id);
        let attention = compute_attention_style(status, seen || active, theme);
        let animation_id = format!(
            "sidebar-agent-{}-{id}",
            project_key
                .as_ref()
                .map(|key| key.machine_id.as_str())
                .unwrap_or(if remote { "remote" } else { "local" }),
        );
        div()
            .id(gpui::ElementId::Name(id.clone().into()))
            .flex()
            .items_center()
            .gap_1()
            .h(ui_px(26.))
            .pl_4()
            .pr_2()
            .text_size(ui_px(11.5))
            .when(
                attention.is_alerting && attention.text_color.is_some(),
                |el| el.text_color(rgba(attention.text_color.unwrap())),
            )
            .when(!attention.is_alerting, |el| {
                el.text_color(rgba(if active { theme.fg0 } else { theme.fg1 }))
            })
            .hover(|style| style.bg(rgba(theme.bg2)))
            .when(
                attention.is_alerting && attention.bg_color.is_some(),
                |el| el.bg(rgba(attention.bg_color.unwrap())),
            )
            .when(!attention.is_alerting && active, |el| {
                el.bg(rgba(theme.bg2))
            })
            .on_click(cx.listener({
                let id = id.clone();
                move |this, _event, window, cx| {
                    if let Some(project_key) = project_key.clone() {
                        this.select_project_workspace_inner(project_key, cx);
                    }
                    this.open_agent(&id, window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    this.focus.focus(window, cx);
                    this.tree_menu = None;
                    this.session_menu = Some(SessionMenu {
                        agent: id.clone(),
                        position: clamp_menu_position(
                            event.position,
                            window.viewport_size(),
                            size(ui_px(180.), ui_px(88.)),
                        ),
                        remote,
                    });
                    this.palette_open = false;
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(render_status_indicator(
                crate::widgets::display_status(status, seen || active),
                animation_id,
                theme,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(title),
            )
            .when(detached, |el| {
                el.child(
                    div()
                        .id(gpui::ElementId::Name(detached_marker_id.clone().into()))
                        .debug_selector(move || detached_marker_id.clone())
                        .flex_none()
                        .w(ui_px(14.))
                        .h(ui_px(14.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .tooltip(hover_tip(
                            i18n::text(self.language, "menu.detach_session"),
                            theme,
                        ))
                        .child(panel_icon(DETACH_ICON, theme.fg2)),
                )
            })
    }
}

impl MuxlaneApp {
    pub(crate) fn render_machine_tree(
        &mut self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let snap = self.last_snapshot.clone();
        let machine_name = snap
            .machine
            .as_ref()
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "local".into());

        // ── 侧栏：机器树（统一 Machines 树：Local Machine + Projects + Sessions）
        let machines = self.sidebar_machines();
        let local_machine = &machines[0];
        let local_machine_key = local_machine.collapse_key.clone();
        let local_collapsed = self.collapsed_machines.contains(&local_machine_key);
        let mut tree = div().flex().flex_col().py_1();
        tree = tree.child(
            div()
                .h(ui_px(28.))
                .px_3()
                .flex()
                .items_center()
                .text_size(ui_px(10.))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgba(theme.fg1))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(i18n::text(self.language, "sidebar.machines")),
                )
                .child(
                    semantic_button(
                        "connect-machine",
                        i18n::text(self.language, "sidebar.connect_remote"),
                        theme,
                    )
                    .ml_auto()
                    .w(ui_px(20.))
                    .h(ui_px(20.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_color(rgba(theme.fg1))
                    .hover(|s| s.bg(rgba(theme.bg2)).text_color(rgba(theme.accent)))
                    .active(|s| s.bg(rgba(theme.bg3)))
                    .tooltip(hover_tip(
                        i18n::text(self.language, "sidebar.connect_remote"),
                        theme,
                    ))
                    .on_click(cx.listener(|this, _ev, window, cx| {
                        this.open_connect_dialog(window, cx);
                    }))
                    .child(panel_icon(PLUS_ICON, theme.fg1)),
                ),
        );
        tree = tree.child(
            div()
                .id("machine-local")
                .flex()
                .items_center()
                .gap_1()
                .h(ui_px(32.))
                .pl_4()
                .pr_2()
                .text_size(ui_px(12.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgba(theme.fg0))
                .group("local-machine")
                .hover(|style| style.bg(rgba(theme.bg2)))
                .on_click(cx.listener({
                    let key = local_machine_key.clone();
                    move |this, _event, _window, cx| {
                        if !this.collapsed_machines.remove(&key) {
                            this.collapsed_machines.insert(key.clone());
                        }
                        cx.notify();
                    }
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(machine_name.clone()),
                )
                .child(
                    div()
                        .ml_auto()
                        .flex_none()
                        .px_1()
                        .bg(rgba(theme.bg2))
                        .text_size(ui_px(9.))
                        .font_weight(gpui::FontWeight::NORMAL)
                        .text_color(rgba(theme.fg1))
                        .child("local"),
                )
                .child(
                    semantic_button(
                        "add-local-project",
                        i18n::text(self.language, "dialog.add_local_project"),
                        theme,
                    )
                    .w(ui_px(20.))
                    .h(ui_px(20.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(ui_px(13.))
                    .font_weight(gpui::FontWeight::NORMAL)
                    .text_color(rgba(theme.fg1))
                    .invisible()
                    .group_hover("local-machine", |style| style.visible())
                    .hover(|s| s.bg(rgba(theme.bg2)).text_color(rgba(theme.accent)))
                    .on_click(cx.listener(|this, _ev, window, cx| {
                        cx.stop_propagation();
                        this.project_dialog = true;
                        this.connect_dialog = false;
                        this.dialog_error = None;
                        this.project_input.update(cx, |input, cx| input.reset(cx));
                        this.project_input.focus_handle(cx).focus(window, cx);
                        cx.notify();
                    }))
                    .child(panel_icon(PLUS_ICON, theme.fg1)),
                ),
        );
        if !local_collapsed {
            for node in &local_machine.projects {
                let project_collapsed = self.collapsed_projects.contains(&node.collapse_key);
                let mut pnode = div().flex().flex_col();
                pnode = pnode.child(self.render_project_row(node.project, None, theme, cx));
                if !project_collapsed {
                    for session in &node.sessions {
                        pnode = pnode.child(self.render_sidebar_session(session, false, theme, cx));
                    }
                }
                tree = tree.child(pnode);
            }
        }

        // ── 远程机器分组
        for machine in machines.iter().skip(1) {
            let host = machine.remote.expect("non-local sidebar machine");
            let name = host.cfg.name.clone();
            let machine_target = DeleteTarget::RemoteMachine { host: name.clone() };
            let (dot_color, status_text, remediation) = match self.remote_states.get(&name) {
                Some(muxlane_client::RemoteState::Online(_)) => {
                    if self.remote_snaps.contains_key(&name) {
                        (
                            theme.green,
                            i18n::text(self.language, "status.connected"),
                            None,
                        )
                    } else {
                        (
                            theme.fg1,
                            i18n::text(self.language, "status.connecting"),
                            None,
                        )
                    }
                }
                Some(muxlane_client::RemoteState::NeedsInstall { .. }) => (
                    theme.yellow,
                    i18n::text(self.language, "status.needs_install"),
                    Some((true, false, None)),
                ),
                Some(muxlane_client::RemoteState::NeedsStart { binary, .. }) => (
                    theme.yellow,
                    i18n::text(self.language, "status.needs_start"),
                    Some((false, false, Some(binary.clone()))),
                ),
                Some(muxlane_client::RemoteState::NeedsUpgrade { .. }) => (
                    theme.yellow,
                    i18n::text(self.language, "status.needs_update"),
                    Some((false, true, None)),
                ),
                Some(muxlane_client::RemoteState::AuthenticationFailed(_)) => (
                    theme.red,
                    i18n::text(self.language, "status.auth_failed"),
                    None,
                ),
                Some(muxlane_client::RemoteState::Connecting(stage)) => (
                    theme.yellow,
                    i18n::text(
                        self.language,
                        match stage {
                            muxlane_client::RemoteStage::SshProbe => "status.remote_ssh_probe",
                            muxlane_client::RemoteStage::Tunnel => "status.remote_tunnel",
                            muxlane_client::RemoteStage::Subscribe => "status.remote_subscribe",
                        },
                    ),
                    None,
                ),
                Some(muxlane_client::RemoteState::Offline(_)) => {
                    (theme.fg1, i18n::text(self.language, "status.offline"), None)
                }
                _ => (
                    theme.fg1,
                    i18n::text(self.language, "status.connecting"),
                    None,
                ),
            };
            let reconnectable = !matches!(
                self.remote_states.get(&name),
                Some(muxlane_client::RemoteState::Online(_))
            );
            let status_text = if matches!(
                self.remote_states.get(&name),
                Some(muxlane_client::RemoteState::Online(_))
            ) {
                host.latency_ms()
                    .map(|latency| format!("{latency} ms"))
                    .unwrap_or_else(|| i18n::text(self.language, "status.connected").into())
            } else if matches!(
                self.remote_states.get(&name),
                Some(muxlane_client::RemoteState::Offline(_))
            ) {
                i18n::text(self.language, "status.disconnected").into()
            } else {
                status_text.to_string()
            };
            let machine_key = machine.collapse_key.clone();
            let machine_collapsed = self.collapsed_machines.contains(&machine_key);
            let snap_ref = self.remote_snaps.get(&name);
            let machine_attention = snap_ref
                .map(|snapshot| {
                    if snapshot
                        .agents
                        .iter()
                        .any(|agent| agent.status == muxlane_core::model::AgentStatus::Blocked)
                    {
                        compute_attention_style(
                            muxlane_core::model::AgentStatus::Blocked,
                            false,
                            theme,
                        )
                    } else if snapshot.agents.iter().any(|agent| {
                        agent.status == muxlane_core::model::AgentStatus::Failed
                            && !agent.seen
                            && self.active.as_ref() != Some(&agent.id)
                    }) {
                        compute_attention_style(
                            muxlane_core::model::AgentStatus::Failed,
                            false,
                            theme,
                        )
                    } else if snapshot.agents.iter().any(|agent| {
                        agent.status == muxlane_core::model::AgentStatus::Done
                            && !agent.seen
                            && self.active.as_ref() != Some(&agent.id)
                    }) {
                        compute_attention_style(
                            muxlane_core::model::AgentStatus::Done,
                            false,
                            theme,
                        )
                    } else {
                        AttentionStyle::default()
                    }
                })
                .unwrap_or_default();
            let remediation_host = name.clone();
            let remote_project_host = name.clone();
            let mut rnode = div().flex().flex_col().mt_1().child(
                div()
                    .id(gpui::ElementId::Name(format!("machine-row-{name}").into()))
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(ui_px(32.))
                    .pl_4()
                    .pr_2()
                    .text_size(ui_px(12.5))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgba(theme.fg0))
                    .when(machine_attention.is_alerting, |row| {
                        row.bg(rgba(machine_attention.bg_color.unwrap_or(theme.bg2)))
                    })
                    .group(gpui::SharedString::from(format!("machine-hover-{name}")))
                    .hover(|style| style.bg(rgba(theme.bg2)))
                    .on_click(cx.listener({
                        let key = machine_key.clone();
                        move |this, _event, _window, cx| {
                            if !this.collapsed_machines.remove(&key) {
                                this.collapsed_machines.insert(key.clone());
                            }
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener({
                            let target = machine_target.clone();
                            move |this, event: &gpui::MouseDownEvent, window, cx| {
                                this.focus.focus(window, cx);
                                this.session_menu = None;
                                this.tree_menu = Some(TreeMenu {
                                    target: target.clone(),
                                    position: clamp_menu_position(
                                        event.position,
                                        window.viewport_size(),
                                        size(ui_px(200.), ui_px(190.)),
                                    ),
                                });
                                cx.stop_propagation();
                                cx.notify();
                            }
                        }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .id(gpui::ElementId::Name(
                                format!("remote-reconnect-{name}").into(),
                            ))
                            .ml_auto()
                            .flex_none()
                            .px_1()
                            .bg(rgba(theme.bg2))
                            .text_size(ui_px(9.))
                            .font_weight(gpui::FontWeight::NORMAL)
                            .text_color(rgba(dot_color))
                            .on_click(cx.listener({
                                let host = Arc::clone(host);
                                let host_cfg = host.cfg.clone();
                                let is_auth_failed = matches!(
                                    self.remote_states.get(&name),
                                    Some(muxlane_client::RemoteState::AuthenticationFailed(_))
                                );
                                move |this, _event, window, cx| {
                                    if is_auth_failed {
                                        this.connect_dialog = true;
                                        this.dialog_error = None;
                                        let target_str = match &host_cfg.target {
                                            muxlane_client::Target::Socket(path) => path.clone(),
                                            muxlane_client::Target::Ssh { host, socket }
                                                if socket.is_empty() =>
                                            {
                                                host.clone()
                                            }
                                            muxlane_client::Target::Ssh { host, socket } => {
                                                format!("{host}:{socket}")
                                            }
                                        };
                                        this.connect_input.update(cx, |input, cx| {
                                            input.set_text(&target_str, cx);
                                        });
                                        match &host_cfg.auth {
                                            muxlane_client::SshAuth::Password {
                                                username, ..
                                            } => {
                                                this.connect_auth_mode = ConnectAuthMode::Password;
                                                this.connect_username.update(cx, |input, cx| {
                                                    input.set_text(username, cx);
                                                });
                                                this.connect_password
                                                    .update(cx, |input, cx| input.reset(cx));
                                                this.connect_password
                                                    .focus_handle(cx)
                                                    .focus(window, cx);
                                            }
                                            muxlane_client::SshAuth::PublicKey {
                                                username,
                                                identity_file,
                                            } => {
                                                this.connect_auth_mode = ConnectAuthMode::PublicKey;
                                                if let Some(u) = username {
                                                    this.connect_username.update(
                                                        cx,
                                                        |input, cx| {
                                                            input.set_text(u, cx);
                                                        },
                                                    );
                                                }
                                                if let Some(k) = identity_file {
                                                    this.connect_key_path.update(
                                                        cx,
                                                        |input, cx| {
                                                            input.set_text(k, cx);
                                                        },
                                                    );
                                                }
                                                this.connect_key_path
                                                    .focus_handle(cx)
                                                    .focus(window, cx);
                                            }
                                            muxlane_client::SshAuth::SshConfig => {
                                                this.connect_auth_mode = ConnectAuthMode::SshConfig;
                                                this.connect_input
                                                    .focus_handle(cx)
                                                    .focus(window, cx);
                                            }
                                        }
                                        cx.notify();
                                    } else if reconnectable {
                                        host.reconnect();
                                        this.focus.focus(window, cx);
                                        cx.notify();
                                    }
                                    cx.stop_propagation();
                                }
                            }))
                            .child(status_text),
                    )
                    .child(
                        div()
                            .id(gpui::ElementId::Name(
                                format!("remote-project-add-{remote_project_host}").into(),
                            ))
                            .w(ui_px(20.))
                            .h(ui_px(20.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(ui_px(13.))
                            .font_weight(gpui::FontWeight::NORMAL)
                            .text_color(rgba(theme.fg1))
                            .invisible()
                            .group_hover(format!("machine-hover-{name}"), |style| style.visible())
                            .hover(|style| style.bg(rgba(theme.bg2)).text_color(rgba(theme.accent)))
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                cx.stop_propagation();
                                this.remote_project_dialog = Some(remote_project_host.clone());
                                this.dialog_error = None;
                                this.remote_project_input
                                    .update(cx, |input, cx| input.reset(cx));
                                this.remote_project_input.focus_handle(cx).focus(window, cx);
                                cx.notify();
                            }))
                            .child(panel_icon(PLUS_ICON, theme.fg1)),
                    )
                    .when_some(remediation, |row, (install, upgrade, binary)| {
                        row.child(
                            div()
                                .id(gpui::ElementId::Name(
                                    format!("remote-remediate-{remediation_host}").into(),
                                ))
                                .flex_none()
                                .px_2()
                                .py_1()
                                .text_size(ui_px(10.))
                                .text_color(rgba(theme.accent))
                                .hover(|style| style.bg(rgba(theme.bg2)))
                                .on_click(cx.listener({
                                    let host = remediation_host.clone();
                                    move |this, _event, _window, cx| {
                                        this.bootstrap_error = None;
                                        this.bootstrap_confirm = Some(BootstrapConfirm {
                                            host: host.clone(),
                                            install,
                                            upgrade,
                                            binary: binary.clone(),
                                        });
                                        cx.stop_propagation();
                                        cx.notify();
                                    }
                                }))
                                .child(if install {
                                    i18n::text(self.language, "bootstrap.install")
                                } else if upgrade {
                                    i18n::text(self.language, "bootstrap.update")
                                } else {
                                    i18n::text(self.language, "bootstrap.start")
                                }),
                        )
                    }),
            );
            // 进度条
            if let Some(progress) = self.bootstrap_progress.get(&name) {
                let overall = progress.phase.overall(progress.percent);
                let phase_text = format_upload_phase(progress, self.language);
                let host_to_cancel = name.clone();
                rnode = rnode.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .pl_4()
                        .pr_2()
                        .h(ui_px(22.))
                        .text_size(ui_px(10.))
                        .text_color(rgba(theme.accent))
                        .child(phase_text)
                        .child(
                            div().flex_1().h(ui_px(3.)).bg(rgba(theme.bg2)).child(
                                div()
                                    .w(relative(overall as f32 / 100.0))
                                    .h_full()
                                    .bg(rgba(theme.accent)),
                            ),
                        )
                        .child(
                            div()
                                .id(gpui::ElementId::Name(
                                    format!("sidebar-cancel-bootstrap-{host_to_cancel}").into(),
                                ))
                                .px_1()
                                .cursor_pointer()
                                .text_color(rgba(theme.fg2))
                                .hover(|s| s.text_color(rgba(theme.red)))
                                .on_click(cx.listener(move |this, _ev, _window, cx| {
                                    this.cancel_bootstrap_for_host(&host_to_cancel, cx);
                                }))
                                .child("×"),
                        ),
                );
            }
            if !machine_collapsed {
                for node in &machine.projects {
                    let project_collapsed = self.collapsed_projects.contains(&node.collapse_key);
                    let mut pnode = div().flex().flex_col();
                    pnode =
                        pnode.child(self.render_project_row(node.project, Some(&name), theme, cx));
                    if !project_collapsed {
                        for session in &node.sessions {
                            pnode =
                                pnode.child(self.render_sidebar_session(session, true, theme, cx));
                        }
                    }
                    rnode = rnode.child(pnode);
                }
            }
            tree = tree.child(rnode);
        }
        tree.into_any_element()
    }

    pub(crate) fn render_sidebar_footer(
        &mut self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let (unread_count, has_blocked, notifications_open) = self.notifications.read(cx).summary();
        div()
            .h(ui_px(40.))
            .px_2()
            .flex()
            .items_center()
            .gap_1()
            .border_t_1()
            .border_color(rgba(theme.line))
            .child({
                // One toggle: "Detach All" while anything is docked, "Reattach All" once all are out.
                let any_docked = self.has_any_docked_session();
                let (id, key, icon) = if any_docked {
                    ("sidebar-detach-all", "sidebar.detach_all", DETACH_ICON)
                } else {
                    (
                        "sidebar-reattach-all",
                        "sidebar.reattach_all",
                        REATTACH_ICON,
                    )
                };
                let label = i18n::text(self.language, key);
                semantic_button(id, label, theme)
                    .debug_selector(move || id.into())
                    .w(ui_px(32.))
                    .h(ui_px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .tooltip(hover_tip(label, theme))
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        if any_docked {
                            this.detach_all_sessions(cx);
                        } else {
                            this.reattach_all_sessions(cx);
                        }
                    }))
                    .child(panel_icon(icon, theme.fg1))
            })
            .child(
                semantic_button(
                    "sidebar-hide-button",
                    i18n::text(self.language, "sidebar.hide"),
                    theme,
                )
                .w(ui_px(32.))
                .h(ui_px(32.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(rgba(theme.bg2)))
                .active(|s| s.bg(rgba(theme.bg3)))
                .tooltip(hover_tip(i18n::text(self.language, "sidebar.hide"), theme))
                .on_click(cx.listener(|this, _ev, window, cx| {
                    this.set_sidebar_visible(false, window, cx);
                }))
                .child(panel_icon(SIDEBAR_COLLAPSE_ICON, theme.fg1)),
            )
            .child(div().flex_1())
            .child({
                let badge_color = if has_blocked {
                    theme.yellow
                } else if unread_count > 0 {
                    theme.accent
                } else {
                    theme.fg2
                };
                let badge_glow = if unread_count > 0 {
                    Some(Theme::with_alpha(badge_color, 0x60))
                } else {
                    None
                };

                semantic_button(
                    "sidebar-notification-button",
                    i18n::text(self.language, "sidebar.notifications"),
                    theme,
                )
                .relative()
                .w(ui_px(32.))
                .h(ui_px(32.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(rgba(theme.bg2)))
                .active(|s| s.bg(rgba(theme.bg3)))
                .tooltip(hover_tip(
                    i18n::text(self.language, "sidebar.notifications"),
                    theme,
                ))
                .when(unread_count > 0, |el| {
                    el.bg(rgba(Theme::with_alpha(badge_color, 0x18)))
                })
                .when(notifications_open, |el| el.bg(rgba(theme.bg2)))
                .on_click(cx.listener(|this, _ev, _window, cx| {
                    this.notifications
                        .update(cx, |center, cx| center.toggle_open(cx));
                    cx.notify();
                }))
                .child(panel_icon(
                    NOTIFICATION_ICON,
                    if notifications_open || unread_count > 0 {
                        badge_color
                    } else {
                        theme.fg1
                    },
                ))
                .when(unread_count > 0, |el| {
                    el.child(
                        div()
                            .absolute()
                            .top(ui_px(2.))
                            .right(ui_px(2.))
                            .min_w(ui_px(14.))
                            .h(ui_px(14.))
                            .px(ui_px(3.))
                            .bg(rgba(badge_color))
                            .when_some(badge_glow, |b, glow| b.border_1().border_color(rgba(glow)))
                            .text_size(ui_px(9.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgba(if has_blocked {
                                theme.on_warning
                            } else {
                                theme.on_accent
                            }))
                            .child(if unread_count > 99 {
                                "99+".to_string()
                            } else {
                                format!("{unread_count}")
                            }),
                    )
                })
            })
            .child(
                semantic_button(
                    "open-settings",
                    i18n::text(self.language, "common.settings"),
                    theme,
                )
                .when(cfg!(test), |button| {
                    button.debug_selector(|| "ux-open-settings".into())
                })
                .w(ui_px(32.))
                .h(ui_px(32.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(rgba(theme.bg2)))
                .active(|s| s.bg(rgba(theme.bg3)))
                .when(self.settings_open, |el| el.bg(rgba(theme.bg2)))
                .tooltip(hover_tip(
                    i18n::text(self.language, "common.settings"),
                    theme,
                ))
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(cx.listener(|this, _ev, window, cx| {
                    this.open_settings(window, cx);
                }))
                .child(panel_icon(
                    SETTINGS_ICON,
                    if self.settings_open {
                        theme.fg0
                    } else {
                        theme.fg1
                    },
                )),
            )
            .into_any_element()
    }
}
