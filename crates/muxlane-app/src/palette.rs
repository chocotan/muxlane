//! Command palette item discovery, keyboard handling, execution, and rendering.
use crate::app::MuxlaneApp;
use crate::i18n;
use crate::icons::*;
use crate::theme::Theme;
use crate::ui_scale::px as ui_px;
use crate::workspace::ProjectKey;
use gpui::{
    div, prelude::*, relative, rgba, Context, Focusable, MouseButton, ParentElement, Styled, Window,
};
use muxlane_core::SplitAxis;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NewSessionTarget {
    Local(muxlane_core::model::ProjectId),
    Remote {
        host: String,
        project: muxlane_core::model::ProjectId,
    },
}

/// 新建会话面板当前键盘焦点所在的栏。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaletteColumn {
    Projects,
    Presets,
}

#[derive(Clone)]
enum PaletteItem {
    Project {
        key: ProjectKey,
        label: String,
        path: String,
    },
    Preset {
        preset: muxlane_core::AgentPreset,
    },
    Action {
        id: &'static str,
        label: &'static str,
        shortcut: Option<&'static str>,
        icon: &'static [u8],
    },
}

impl MuxlaneApp {
    pub(crate) fn default_palette_project(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<ProjectKey> {
        self.terms
            .iter()
            .find_map(|(agent, term)| {
                term.focus_handle(cx)
                    .is_focused(window)
                    .then(|| self.project_key_for_agent(agent))
                    .flatten()
            })
            .or_else(|| {
                self.pane_tree
                    .group(&self.active_pane)
                    .and_then(|group| group.active.as_ref())
                    .and_then(|agent| self.project_key_for_agent(agent))
            })
            .or_else(|| self.workspace.current_project().cloned())
            .or_else(|| self.available_project_keys().into_iter().next())
    }

    fn palette_project_label(&self, key: &ProjectKey) -> Option<(String, String)> {
        if key.machine_id == self.local_machine_id() {
            return self
                .last_snapshot
                .project(&key.project_id)
                .map(|project| (project.name.clone(), project.path.display().to_string()));
        }
        self.remote_snaps.values().find_map(|snapshot| {
            snapshot
                .machine
                .as_ref()
                .is_some_and(|machine| machine.machine_id == key.machine_id)
                .then(|| snapshot.project(&key.project_id))
                .flatten()
                .map(|project| (project.name.clone(), project.path.display().to_string()))
        })
    }

    /// 新建会话面板左栏：项目列表（查询过滤后）。
    fn palette_projects(&self, cx: &Context<Self>) -> Vec<(ProjectKey, String, String)> {
        let query = self.palette_input.read(cx).text().trim().to_lowercase();
        self.available_project_keys()
            .into_iter()
            .filter_map(|key| {
                self.palette_project_label(&key)
                    .map(|(label, path)| (key, label, path))
            })
            .filter(|(_, label, path)| {
                query.is_empty() || format!("{label} {path}").to_lowercase().contains(&query)
            })
            .collect()
    }

    /// 新建会话面板右栏：当前项目可用的 Agent 预设（查询过滤后）。
    fn palette_presets(&self, cx: &Context<Self>) -> Vec<muxlane_core::AgentPreset> {
        let query = self.palette_input.read(cx).text().trim().to_lowercase();
        // 远端不做本机 PATH 过滤：program 绝对路径跨机无意义。
        let filter_installed = !matches!(
            self.new_session_target,
            Some(NewSessionTarget::Remote { .. })
        );
        let project_path = self.palette_project_path();
        self.presets
            .clone()
            .into_iter()
            .filter(|preset| {
                !filter_installed
                    || project_path
                        .as_deref()
                        .map_or_else(|| preset.installed(), |path| preset.installed_in(path))
            })
            .filter(|preset| {
                query.is_empty()
                    || format!("{} {}", preset.label, preset.program)
                        .to_lowercase()
                        .contains(&query)
            })
            .collect()
    }

    pub(crate) fn select_palette_project(&mut self, key: ProjectKey, cx: &mut Context<Self>) {
        self.palette_project_index = self
            .available_project_keys()
            .iter()
            .position(|candidate| *candidate == key)
            .unwrap_or(0);
        self.palette_column = PaletteColumn::Presets;
        self.palette_project = Some(key.clone());
        self.new_session_target = if key.machine_id == self.local_machine_id() {
            Some(NewSessionTarget::Local(key.project_id))
        } else {
            self.remote_host_for_key(&key)
                .map(|host| NewSessionTarget::Remote {
                    host,
                    project: key.project_id,
                })
        };
        self.palette_index = 0;
        self.palette_input.update(cx, |input, cx| input.reset(cx));
        cx.notify();
    }

    fn palette_project_path(&self) -> Option<std::path::PathBuf> {
        if let Some(key) = &self.palette_project {
            if key.machine_id == self.local_machine_id() {
                return self
                    .last_snapshot
                    .project(&key.project_id)
                    .map(|p| p.path.clone());
            }
            return self.remote_snaps.values().find_map(|snapshot| {
                snapshot
                    .machine
                    .as_ref()
                    .is_some_and(|machine| machine.machine_id == key.machine_id)
                    .then(|| snapshot.project(&key.project_id))
                    .flatten()
                    .map(|project| project.path.clone())
            });
        }
        match &self.new_session_target {
            Some(NewSessionTarget::Local(id)) => {
                self.last_snapshot.project(id).map(|p| p.path.clone())
            }
            Some(NewSessionTarget::Remote { host, project }) => self
                .remote_snaps
                .get(host)
                .and_then(|s| s.project(project))
                .map(|p| p.path.clone()),
            None => self
                .active
                .as_ref()
                .and_then(|id| self.find_agent(id))
                .and_then(|a| {
                    self.last_snapshot
                        .project(&a.project)
                        .map(|p| p.path.clone())
                        .or_else(|| {
                            for snap in self.remote_snaps.values() {
                                if let Some(p) = snap.project(&a.project) {
                                    return Some(p.path.clone());
                                }
                            }
                            None
                        })
                })
                .or_else(|| {
                    self.last_snapshot
                        .projects
                        .first()
                        .map(|project| project.path.clone())
                }),
        }
    }

    fn compute_palette_items(&self, cx: &Context<Self>) -> Vec<PaletteItem> {
        let query = self.palette_input.read(cx).text().trim().to_lowercase();
        let mut items = Vec::new();
        for key in self.available_project_keys() {
            if let Some((label, path)) = self.palette_project_label(&key) {
                items.push(PaletteItem::Project { key, label, path });
            }
        }

        if self.new_session_target.is_none() {
            // 全局命令面板 (Ctrl+K)：预设 + 操作，不含会话列表。
            // （新建会话走两栏布局，见 render_new_session_palette。）
            let project_path = self.palette_project_path();
            for preset in self.presets.clone().into_iter().filter(|p| {
                project_path
                    .as_deref()
                    .map_or_else(|| p.installed(), |path| p.installed_in(path))
            }) {
                items.push(PaletteItem::Preset { preset });
            }

            // 3. 操作指令
            items.push(PaletteItem::Action {
                id: "cmd-split-h",
                label: i18n::text(self.language, "palette.horizontal_split"),
                shortcut: None,
                icon: SPLIT_HORIZONTAL_ICON,
            });
            items.push(PaletteItem::Action {
                id: "cmd-split-v",
                label: i18n::text(self.language, "palette.vertical_split"),
                shortcut: None,
                icon: SPLIT_VERTICAL_ICON,
            });
            items.push(PaletteItem::Action {
                id: "cmd-max",
                label: i18n::text(self.language, "palette.maximize"),
                shortcut: None,
                icon: MAXIMIZE_ICON,
            });
            if self.pane_tree.leaf_count() > 1 {
                items.push(PaletteItem::Action {
                    id: "cmd-close-pane",
                    label: i18n::text(self.language, "palette.close_split"),
                    shortcut: None,
                    icon: CLOSE_ICON,
                });
            }
            items.push(PaletteItem::Action {
                id: "cmd-connect",
                label: i18n::text(self.language, "palette.connect_remote"),
                shortcut: None,
                icon: CONNECT_ICON,
            });
            items.push(PaletteItem::Action {
                id: "cmd-toggle-theme",
                label: if self.theme_mode.is_dark() {
                    i18n::text(self.language, "palette.toggle_light")
                } else {
                    i18n::text(self.language, "palette.toggle_dark")
                },
                shortcut: None,
                icon: THEME_ICON,
            });
            items.push(PaletteItem::Action {
                id: "cmd-clear-notifs",
                label: i18n::text(self.language, "palette.clear_notifications"),
                shortcut: None,
                icon: NOTIFICATION_ICON,
            });
        }

        if query.is_empty() {
            items
        } else {
            items
                .into_iter()
                .filter(|item| match item {
                    PaletteItem::Project { label, path, .. } => {
                        format!("{label} {path}").to_lowercase().contains(&query)
                    }
                    PaletteItem::Preset { preset } => {
                        let text = i18n::text(self.language, "palette.new")
                            .replace("{name}", &format!("{} {}", preset.label, preset.program))
                            .to_lowercase();
                        text.contains(&query)
                    }
                    PaletteItem::Action { label, .. } => label.to_lowercase().contains(&query),
                })
                .collect()
        }
    }

    fn execute_palette_item(
        &mut self,
        item: PaletteItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match item {
            PaletteItem::Project { key, .. } => {
                self.select_palette_project(key, cx);
                self.palette_open = true;
                return;
            }
            PaletteItem::Preset { preset } => {
                self.palette_open = false;
                self.spawn_preset(&preset, window, cx);
            }
            PaletteItem::Action { id, .. } => {
                self.palette_open = false;
                self.new_session_target = None;
                self.palette_project = None;
                match id {
                    "cmd-split-h" => {
                        let pane = self.active_pane.clone();
                        self.split_pane(&pane, SplitAxis::Horizontal, window, cx);
                    }
                    "cmd-split-v" => {
                        let pane = self.active_pane.clone();
                        self.split_pane(&pane, SplitAxis::Vertical, window, cx);
                    }
                    "cmd-max" => {
                        let pane = self.active_pane.clone();
                        self.toggle_maximize(&pane, cx);
                    }
                    "cmd-close-pane" => {
                        let pane = self.active_pane.clone();
                        self.close_split_pane(&pane, window, cx);
                    }
                    "cmd-connect" => self.open_connect_dialog(window, cx),
                    "cmd-toggle-theme" => {
                        self.toggle_theme(cx);
                    }
                    "cmd-clear-notifs" => {
                        self.notifications.update(cx, |center, cx| center.clear(cx));
                    }
                    _ => {}
                }
            }
        }
        cx.notify();
    }

    /// 返回是否消费了该按键（消费才 stop_propagation）
    pub(super) fn handle_palette_key(
        &mut self,
        ks: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // 有输入框时不抢编辑键：输入框的 bubble listener 先处理编辑/自插字符；
        // 导航/确认键仍由 palette 统一处理（Enter/上下/Escape 在 TextField 中
        // 本就不消费，会冒泡到这里）。
        if self.new_session_target.is_some() {
            return self.handle_new_session_palette_key(ks, window, cx);
        }
        let items = self.compute_palette_items(cx);
        match ks.key.as_str() {
            "up" => {
                self.palette_index = self.palette_index.saturating_sub(1);
                self.palette_scroll.scroll_to_item(self.palette_index);
                cx.notify();
                true
            }
            "down" => {
                if !items.is_empty() {
                    self.palette_index = (self.palette_index + 1).min(items.len() - 1);
                    self.palette_scroll.scroll_to_item(self.palette_index);
                    cx.notify();
                }
                true
            }
            "enter" => {
                if let Some(item) = items.get(self.palette_index).cloned() {
                    self.execute_palette_item(item, window, cx);
                    return true;
                }
                false
            }
            "escape" => {
                self.palette_open = false;
                self.new_session_target = None;
                self.palette_project = None;
                if let Some(active) = self.active.clone() {
                    self.focus_agent(&active, window, cx);
                }
                cx.notify();
                true
            }
            _ => false,
        }
    }

    /// 新建会话模式：左栏项目、右栏 Agent 类型，Tab 切栏。
    fn handle_new_session_palette_key(
        &mut self,
        ks: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let projects = self.palette_projects(cx);
        let presets = self.palette_presets(cx);
        match ks.key.as_str() {
            "tab" => {
                self.palette_column = match self.palette_column {
                    PaletteColumn::Projects => PaletteColumn::Presets,
                    PaletteColumn::Presets => PaletteColumn::Projects,
                };
                cx.notify();
                true
            }
            "up" => {
                match self.palette_column {
                    PaletteColumn::Projects => {
                        self.palette_project_index = self.palette_project_index.saturating_sub(1);
                        self.palette_project_scroll
                            .scroll_to_item(self.palette_project_index);
                        self.sync_palette_project_from_index(cx);
                    }
                    PaletteColumn::Presets => {
                        self.palette_index = self.palette_index.saturating_sub(1);
                        self.palette_scroll.scroll_to_item(self.palette_index);
                    }
                }
                cx.notify();
                true
            }
            "down" => {
                match self.palette_column {
                    PaletteColumn::Projects => {
                        if !projects.is_empty() {
                            self.palette_project_index =
                                (self.palette_project_index + 1).min(projects.len() - 1);
                            self.palette_project_scroll
                                .scroll_to_item(self.palette_project_index);
                            self.sync_palette_project_from_index(cx);
                        }
                    }
                    PaletteColumn::Presets => {
                        if !presets.is_empty() {
                            self.palette_index = (self.palette_index + 1).min(presets.len() - 1);
                            self.palette_scroll.scroll_to_item(self.palette_index);
                        }
                    }
                }
                cx.notify();
                true
            }
            "enter" => match self.palette_column {
                PaletteColumn::Projects => {
                    if let Some((key, _, _)) = projects.get(self.palette_project_index).cloned() {
                        self.select_palette_project(key, cx);
                        return true;
                    }
                    false
                }
                PaletteColumn::Presets => {
                    if let Some(preset) = presets.get(self.palette_index).cloned() {
                        self.palette_open = false;
                        // 不能在此清 new_session_target：spawn_preset 要 take 它决定项目。
                        self.spawn_preset(&preset, window, cx);
                        cx.notify();
                        return true;
                    }
                    false
                }
            },
            "escape" => {
                self.palette_open = false;
                self.new_session_target = None;
                self.palette_project = None;
                if let Some(active) = self.active.clone() {
                    self.focus_agent(&active, window, cx);
                }
                cx.notify();
                true
            }
            _ => false,
        }
    }

    /// 左栏上下移动时联动选中项目（右栏预设随项目过滤）。
    fn sync_palette_project_from_index(&mut self, cx: &mut Context<Self>) {
        let Some((key, _, _)) = self
            .palette_projects(cx)
            .get(self.palette_project_index)
            .cloned()
        else {
            return;
        };
        self.palette_project = Some(key.clone());
        self.new_session_target = if key.machine_id == self.local_machine_id() {
            Some(NewSessionTarget::Local(key.project_id))
        } else {
            self.remote_host_for_key(&key)
                .map(|host| NewSessionTarget::Remote {
                    host,
                    project: key.project_id,
                })
        };
        self.palette_index = 0;
    }

    pub(super) fn render_palette(
        &mut self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if self.new_session_target.is_some() {
            return self.render_new_session_palette(theme, cx);
        }
        let items = self.compute_palette_items(cx);
        let current_index = self.palette_index;
        let mut list_container = div()
            .id("palette-items-scroll")
            .flex()
            .flex_col()
            .max_h(ui_px(324.))
            .overflow_y_scroll()
            .track_scroll(&self.palette_scroll);

        if items.is_empty() {
            list_container = list_container.child(
                div()
                    .px_4()
                    .py_6()
                    .text_size(ui_px(12.))
                    .text_color(rgba(theme.fg2))
                    .child(i18n::text(self.language, "palette.no_results")),
            );
        } else {
            for (index, item) in items.into_iter().enumerate() {
                let is_selected = index == current_index;
                let item_for_click = item.clone();
                let row = match item {
                    PaletteItem::Project { key, label, path } => {
                        let selected = self.palette_project.as_ref() == Some(&key);
                        let item_for_click = item_for_click.clone();
                        div()
                            .id(gpui::ElementId::Name(
                                format!("pal-project-{}-{}", key.machine_id, key.project_id).into(),
                            ))
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .text_size(ui_px(12.))
                            .text_color(rgba(theme.fg0))
                            .when(is_selected, |el| el.bg(rgba(theme.bg2)))
                            .when(selected, |el| {
                                el.border_l_2().border_color(rgba(theme.accent))
                            })
                            .hover(|s| s.bg(rgba(theme.bg2)))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _ev, _window, cx| {
                                if let PaletteItem::Project { key, .. } = item_for_click.clone() {
                                    this.select_palette_project(key, cx);
                                }
                            }))
                            .child(panel_icon(CONNECT_ICON, theme.accent))
                            .child(label)
                            .child(
                                div()
                                    .ml_auto()
                                    .max_w(ui_px(260.))
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .text_size(ui_px(10.))
                                    .text_color(rgba(theme.fg2))
                                    .child(path),
                            )
                    }
                    PaletteItem::Preset { preset } => div()
                        .id(gpui::ElementId::Name(
                            format!("pal-preset-{}", preset.id).into(),
                        ))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .text_size(ui_px(12.))
                        .text_color(rgba(theme.fg0))
                        .when(is_selected, |el| el.bg(rgba(theme.bg2)))
                        .hover(|s| s.bg(rgba(theme.bg2)))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _ev, window, cx| {
                            this.execute_palette_item(item_for_click.clone(), window, cx);
                        }))
                        .child(panel_icon(PLUS_ICON, theme.accent))
                        .child(
                            i18n::text(self.language, "palette.new")
                                .replace("{name}", &preset.label),
                        )
                        .child(
                            div()
                                .ml_auto()
                                .text_size(ui_px(10.))
                                .text_color(rgba(theme.fg2))
                                .child(preset.program),
                        ),
                    PaletteItem::Action {
                        label,
                        shortcut,
                        icon,
                        ..
                    } => div()
                        .id(gpui::ElementId::Name(format!("pal-action-{index}").into()))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .text_size(ui_px(12.))
                        .text_color(rgba(theme.fg0))
                        .when(is_selected, |el| el.bg(rgba(theme.bg2)))
                        .hover(|s| s.bg(rgba(theme.bg2)))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _ev, window, cx| {
                            this.execute_palette_item(item_for_click.clone(), window, cx);
                        }))
                        .child(panel_icon(icon, theme.accent))
                        .child(label)
                        .when_some(shortcut, |row, sc| {
                            row.child(
                                div()
                                    .ml_auto()
                                    .px_1p5()
                                    .py_0p5()
                                    .border_1()
                                    .border_color(rgba(theme.line))
                                    .text_size(ui_px(9.5))
                                    .text_color(rgba(theme.fg2))
                                    .child(format!("[{sc}]")),
                            )
                        }),
                };
                list_container = list_container.child(row);
            }
        }

        let panel = div()
            .occlude()
            .w(ui_px(560.))
            .max_w(relative(0.92))
            .max_h(relative(0.75))
            .overflow_hidden()
            .bg(rgba(theme.bg1))
            .border_1()
            .border_color(rgba(theme.line))
            .shadow_xl()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _ev, _window, cx| {
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .p_3()
                    .border_b_1()
                    .border_color(rgba(theme.line))
                    .child(self.palette_input.clone()),
            )
            .child(list_container);

        div()
            .id("palette-backdrop")
            .absolute()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(theme.overlay()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _ev, window, cx| {
                    this.palette_open = false;
                    this.new_session_target = None;
                    this.palette_project = None;
                    if let Some(active) = this.active.clone() {
                        this.focus_agent(&active, window, cx);
                    }
                    cx.notify();
                }),
            )
            .child(panel)
            .into_any_element()
    }

    /// 新建会话面板：左栏项目、右栏 Agent 类型，不再是混合列表。
    fn render_new_session_palette(
        &mut self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let projects = self.palette_projects(cx);
        let presets = self.palette_presets(cx);
        let focused_column = self.palette_column;
        let project_index = self.palette_project_index;
        let preset_index = self.palette_index;
        let selected_project = self.palette_project.clone();

        let mut project_list = div()
            .id("palette-projects-scroll")
            .flex()
            .flex_col()
            .w(ui_px(220.))
            .flex_none()
            .max_h(ui_px(324.))
            .overflow_y_scroll()
            .track_scroll(&self.palette_project_scroll)
            .border_r_1()
            .border_color(rgba(theme.line));
        if projects.is_empty() {
            project_list = project_list.child(
                div()
                    .px_3()
                    .py_4()
                    .text_size(ui_px(11.))
                    .text_color(rgba(theme.fg2))
                    .child(i18n::text(self.language, "palette.no_results")),
            );
        }
        for (index, (key, label, path)) in projects.into_iter().enumerate() {
            let is_focused = focused_column == PaletteColumn::Projects && index == project_index;
            let is_selected = selected_project.as_ref() == Some(&key);
            project_list = project_list.child(
                div()
                    .id(gpui::ElementId::Name(
                        format!("pal-proj-{}-{}", key.machine_id, key.project_id).into(),
                    ))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .text_size(ui_px(12.))
                    .text_color(rgba(theme.fg0))
                    .when(is_focused && !is_selected, |el| el.bg(rgba(theme.bg2)))
                    .when(is_selected, |el| el.bg(rgba(theme.selection())))
                    .hover(|s| s.bg(rgba(theme.bg2)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.select_palette_project(key.clone(), cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(label),
                    )
                    .child(
                        div()
                            .max_w(ui_px(80.))
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_size(ui_px(9.5))
                            .text_color(rgba(theme.fg2))
                            .child(path),
                    ),
            );
        }

        let mut preset_list = div()
            .id("palette-presets-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .max_h(ui_px(324.))
            .overflow_y_scroll()
            .track_scroll(&self.palette_scroll);
        if presets.is_empty() {
            preset_list = preset_list.child(
                div()
                    .px_3()
                    .py_4()
                    .text_size(ui_px(11.))
                    .text_color(rgba(theme.fg2))
                    .child(i18n::text(self.language, "palette.no_results")),
            );
        }
        for (index, preset) in presets.into_iter().enumerate() {
            let is_focused = focused_column == PaletteColumn::Presets && index == preset_index;
            let label = i18n::text(self.language, "palette.new").replace("{name}", &preset.label);
            let program = preset.program.clone();
            preset_list = preset_list.child(
                div()
                    .id(gpui::ElementId::Name(
                        format!("pal-preset-{}", preset.id).into(),
                    ))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .text_size(ui_px(12.))
                    .text_color(rgba(theme.fg0))
                    .when(is_focused, |el| el.bg(rgba(theme.bg2)))
                    .hover(|s| s.bg(rgba(theme.bg2)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _ev, window, cx| {
                        this.palette_open = false;
                        // 不能在此清 new_session_target：spawn_preset 要 take 它决定项目。
                        this.spawn_preset(&preset, window, cx);
                        cx.notify();
                    }))
                    .child(panel_icon(PLUS_ICON, theme.accent))
                    .child(label)
                    .child(
                        div()
                            .ml_auto()
                            .text_size(ui_px(10.))
                            .text_color(rgba(theme.fg2))
                            .child(program),
                    ),
            );
        }

        let hint = i18n::text(self.language, "palette.tab_switch_column");
        let panel = div()
            .occlude()
            .w(ui_px(560.))
            .max_w(relative(0.92))
            .max_h(relative(0.75))
            .overflow_hidden()
            .bg(rgba(theme.bg1))
            .border_1()
            .border_color(rgba(theme.line))
            .shadow_xl()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _ev, _window, cx| {
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .p_3()
                    .border_b_1()
                    .border_color(rgba(theme.line))
                    .child(self.palette_input.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_start()
                    .child(project_list)
                    .child(preset_list),
            )
            .child(
                div()
                    .px_3()
                    .py_1p5()
                    .border_t_1()
                    .border_color(rgba(theme.line))
                    .text_size(ui_px(10.))
                    .text_color(rgba(theme.fg2))
                    .child(hint),
            );

        div()
            .id("palette-backdrop")
            .absolute()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(theme.overlay()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _ev, window, cx| {
                    this.palette_open = false;
                    this.new_session_target = None;
                    this.palette_project = None;
                    if let Some(active) = this.active.clone() {
                        this.focus_agent(&active, window, cx);
                    }
                    cx.notify();
                }),
            )
            .child(panel)
            .into_any_element()
    }
}
