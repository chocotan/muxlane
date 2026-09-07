//! Command palette item discovery, keyboard handling, execution, and rendering.
use crate::app::MuxlaneApp;
use crate::i18n;
use crate::icons::*;
use crate::theme::Theme;
use crate::ui_scale::px as ui_px;
use crate::widgets::semantic_button;
use crate::workspace::ProjectKey;
use gpui::{
    div, prelude::*, relative, rgba, Context, Focusable, MouseButton, ParentElement, Styled, Window,
};
use muxlane_core::SplitAxis;

fn availability_key(availability: &muxlane_acp::Availability) -> &'static str {
    match availability {
        muxlane_acp::Availability::Found(_) => "agents.found",
        muxlane_acp::Availability::Missing => "agents.missing",
        muxlane_acp::Availability::Unmapped => "agents.unmapped",
        muxlane_acp::Availability::Unsupported => "agents.unsupported",
        muxlane_acp::Availability::Invalid(_) => "agents.invalid",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NewSessionTarget {
    Local(muxlane_core::model::ProjectId),
    Remote {
        host: String,
        project: muxlane_core::model::ProjectId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionCreationMode {
    Ui,
    Terminal,
}

impl SessionCreationMode {
    pub(crate) fn for_target(target: &NewSessionTarget) -> Self {
        match target {
            NewSessionTarget::Local(_) | NewSessionTarget::Remote { .. } => Self::Terminal,
        }
    }
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

    fn palette_acp_profiles(&self, cx: &Context<Self>) -> Vec<muxlane_acp::AgentEntry> {
        let query = self.palette_input.read(cx).text().trim().to_lowercase();
        self.acp_entries
            .iter()
            .filter(|entry| {
                query.is_empty()
                    || format!("{} {}", entry.id, entry.label)
                        .to_lowercase()
                        .contains(&query)
            })
            .cloned()
            .collect()
    }

    /// Both pointer and keyboard activation, and non-palette creation, use this gate.
    pub(crate) fn prepare_acp_creation(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        if self.acp_detecting {
            return false;
        }
        let result = self.acp_launch_profile(id);
        if result.is_ok() {
            return true;
        }
        if let Some(error) = &self.acp_registry_error {
            self.notifications.update(cx, |center, cx| center.show_error(error.clone(), cx));
            self.palette_open = true;
            cx.notify();
            return false;
        }
        if let Some(entry) = self.acp_entries.iter_mut().find(|entry| entry.id == id) {
            if let Some(profile) = &entry.profile {
                entry.availability = match muxlane_acp::effective_definition(profile) {
                    Ok(definition) => muxlane_acp::LocalProbe::current().definition(&definition),
                    Err(error) => muxlane_acp::Availability::Invalid(format!("{error:#}")),
                };
            }
        }
        self.acp_install_detail = Some(id.into());
        self.palette_open = true;
        cx.notify();
        false
    }

    fn activate_acp_entry(
        &mut self,
        entry: muxlane_acp::AgentEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.prepare_acp_creation(&entry.id, cx) {
            return;
        }
        if let Some(profile) = self.acp_registry.resolve(&entry.id) {
            self.acp_install_detail = None;
            self.spawn_acp_view(profile, window, cx);
        }
    }

    pub(crate) fn select_palette_project(&mut self, key: ProjectKey, cx: &mut Context<Self>) {
        self.acp_install_detail = None;
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
        if let Some(target) = self.new_session_target.as_ref() {
            self.session_creation_mode = SessionCreationMode::for_target(target);
        }
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
            // Global command palette: presets and actions.
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
        if self.session_creation_mode == SessionCreationMode::Ui {
            if let Some(id) = self.acp_install_detail.clone() {
                match ks.key.as_str() {
                    "escape" | "up" | "down" => {
                        self.acp_install_detail = None;
                        cx.notify();
                        return true;
                    }
                    "enter" => {
                        if let Some(entry) = self
                            .acp_entries
                            .iter()
                            .find(|entry| entry.id == id)
                            .cloned()
                        {
                            self.activate_acp_entry(entry, window, cx);
                        }
                        return true;
                    }
                    _ => {}
                }
            }
        }
        let projects = self.palette_projects(cx);
        let presets = self.palette_presets(cx);
        let profiles = self.palette_acp_profiles(cx);
        let session_count = match self.session_creation_mode {
            SessionCreationMode::Ui => profiles.len(),
            SessionCreationMode::Terminal => presets.len(),
        };
        match ks.key.as_str() {
            "tab" if ks.modifiers.shift => {
                self.session_creation_mode = match self.session_creation_mode {
                    SessionCreationMode::Ui => SessionCreationMode::Terminal,
                    SessionCreationMode::Terminal
                        if matches!(self.new_session_target, Some(NewSessionTarget::Local(_))) =>
                    {
                        SessionCreationMode::Ui
                    }
                    SessionCreationMode::Terminal => SessionCreationMode::Terminal,
                };
                self.palette_index = 0;
                self.palette_scroll.scroll_to_item(0);
                cx.notify();
                true
            }
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
                        if session_count > 0 {
                            self.palette_index = (self.palette_index + 1).min(session_count - 1);
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
                PaletteColumn::Presets => match self.session_creation_mode {
                    SessionCreationMode::Ui => {
                        if let Some(profile) = profiles.get(self.palette_index).cloned() {
                            self.activate_acp_entry(profile, window, cx);
                            cx.notify();
                            return true;
                        }
                        false
                    }
                    SessionCreationMode::Terminal => {
                        if let Some(preset) = presets.get(self.palette_index).cloned() {
                            self.palette_open = false;
                            // spawn_preset takes the target to resolve the project.
                            self.spawn_preset(&preset, window, cx);
                            cx.notify();
                            return true;
                        }
                        false
                    }
                },
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
        if let Some(target) = self.new_session_target.as_ref() {
            self.session_creation_mode = SessionCreationMode::for_target(target);
        }
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

    fn render_agent_discovery_actions(
        &self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let refresh = i18n::text(
            self.language,
            if self.acp_catalog_loading {
                "agents.refreshing"
            } else {
                "agents.refresh"
            },
        );
        let detect = i18n::text(
            self.language,
            if self.acp_detecting {
                "agents.detecting"
            } else {
                "agents.detect"
            },
        );
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .p_2()
            .text_size(ui_px(11.))
            .child(
                semantic_button("acp-catalog-refresh", refresh, theme)
                    .px_2()
                    .py_1()
                    .when(!self.acp_catalog_loading, |button| {
                        button.on_click(cx.listener(|this, _, _, cx| this.refresh_acp_catalog(cx)))
                    })
                    .child(panel_icon(CONNECT_ICON, theme.fg2))
                    .child(refresh),
            )
            .child(
                semantic_button("acp-local-detect", detect, theme)
                    .px_2()
                    .py_1()
                    .when(!self.acp_detecting, |button| {
                        button.on_click(cx.listener(|this, _, _, cx| this.detect_acp_agents(cx)))
                    })
                    .child(panel_icon(FOLDER_ICON, theme.fg2))
                    .child(detect),
            )
            .into_any_element()
    }

    fn render_agent_install_detail(
        &self,
        entry: muxlane_acp::AgentEntry,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let back = i18n::text(self.language, "agents.back");
        let mut detail = div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap_2()
            .p_3()
            .text_size(ui_px(11.))
            .text_color(rgba(theme.fg0))
            .child(
                semantic_button("acp-install-back", back, theme)
                    .px_2()
                    .py_1()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.acp_install_detail = None;
                        cx.notify();
                    }))
                    .child(panel_icon(CLOSE_ICON, theme.fg2))
                    .child(back),
            )
            .child(entry.label.clone())
            .child(i18n::text(
                self.language,
                availability_key(&entry.availability),
            ));
        if let muxlane_acp::Availability::Found(path) = &entry.availability {
            detail = detail
                .child(
                    div()
                        .id("acp-found-path")
                        .overflow_x_scroll()
                        .child(path.display().to_string()),
                )
                .child(i18n::text(self.language, "agents.found_note"));
            let start = i18n::text(self.language, "agents.create");
            let selected = entry.clone();
            detail = detail.child(
                semantic_button("acp-install-create", start, theme)
                    .px_2()
                    .py_1()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_acp_entry(selected.clone(), window, cx)
                    }))
                    .child(panel_icon(PLUS_ICON, theme.fg2))
                    .child(start),
            );
        }
        if let muxlane_acp::Availability::Invalid(error) = &entry.availability {
            detail = detail.child(error.clone());
        }
        if let Some(command) = entry.install.filter(|_| !entry.can_start()) {
            detail = detail
                .child(i18n::text(self.language, "agents.manual_install"))
                .child(
                    div()
                        .id("acp-install-command")
                        .w_full()
                        .overflow_x_scroll()
                        .child(command),
                )
                .child(
                    semantic_button(
                        "acp-copy-install",
                        i18n::text(self.language, "agents.copy"),
                        theme,
                    )
                    .px_2()
                    .py_1()
                    .when(cfg!(test), |button| {
                        button.debug_selector(|| "acp-copy-install".into())
                    })
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(command.into()))
                    }))
                    .child(panel_icon(COPY_ICON, theme.fg2))
                    .child(i18n::text(self.language, "agents.copy")),
                );
        } else if !entry.can_start() {
            detail = detail.child(i18n::text(self.language, "agents.configure"));
        }
        if entry.user_configured {
            detail = detail.child(i18n::text(self.language, "agents.custom_warning"));
        }
        if let Some(url) = entry.docs {
            detail = detail.child(
                semantic_button(
                    "acp-install-docs",
                    i18n::text(self.language, "agents.docs"),
                    theme,
                )
                .px_2()
                .py_1()
                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)))
                .child(panel_icon(CONNECT_ICON, theme.fg2))
                .child(i18n::text(self.language, "agents.docs")),
            );
        }
        detail.into_any_element()
    }

    /// 新建会话面板：左栏项目、右栏 Agent 类型，不再是混合列表。
    fn render_new_session_palette(
        &mut self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let projects = self.palette_projects(cx);
        let presets = self.palette_presets(cx);
        let profiles = self.palette_acp_profiles(cx);
        let mode = self.session_creation_mode;
        let local_target = matches!(self.new_session_target, Some(NewSessionTarget::Local(_)));
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
        let session_count = match mode {
            SessionCreationMode::Ui => profiles.len(),
            SessionCreationMode::Terminal => presets.len(),
        };
        if session_count == 0 {
            preset_list = preset_list.child(
                div()
                    .px_3()
                    .py_4()
                    .text_size(ui_px(11.))
                    .text_color(rgba(theme.fg2))
                    .child(i18n::text(self.language, "palette.no_results")),
            );
        }
        match mode {
            SessionCreationMode::Ui => {
                preset_list = preset_list.child(self.render_agent_discovery_actions(theme, cx));
                if let Some(error) = &self.acp_catalog_error {
                    preset_list = preset_list.child(
                        div()
                            .px_3()
                            .py_2()
                            .text_size(ui_px(11.))
                            .text_color(rgba(theme.red))
                            .child(format!(
                                "{}: {error}",
                                i18n::text(self.language, "agents.catalog_error")
                            )),
                    );
                }
                if let Some(error) = &self.acp_registry_error {
                    preset_list = preset_list.child(
                        div()
                            .px_3()
                            .py_2()
                            .text_size(ui_px(11.))
                            .text_color(rgba(theme.red))
                            .child(error.clone()),
                    );
                }
                let detail = self
                    .acp_install_detail
                    .as_ref()
                    .and_then(|id| self.acp_entries.iter().find(|entry| &entry.id == id))
                    .cloned();
                if let Some(detail) = detail {
                    preset_list =
                        preset_list.child(self.render_agent_install_detail(detail, theme, cx));
                }
                for (index, profile) in profiles
                    .into_iter()
                    .enumerate()
                    .filter(|_| self.acp_install_detail.is_none())
                {
                    let is_focused =
                        focused_column == PaletteColumn::Presets && index == preset_index;
                    let label = format!("{} UI", profile.label);
                    let command = format!(
                        "{}{}",
                        i18n::text(
                            self.language,
                            if self.acp_detecting {
                                "agents.detecting"
                            } else {
                                availability_key(&profile.availability)
                            }
                        ),
                        if profile.user_configured {
                            i18n::text(self.language, "agents.user_command")
                        } else {
                            ""
                        }
                    );
                    preset_list = preset_list.child(
                        div()
                            .id(gpui::ElementId::Name(
                                format!("pal-acp-{}", profile.id).into(),
                            ))
                            .when(cfg!(test), |row| {
                                row.debug_selector(|| "acp-agent-row".into())
                            })
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .text_size(ui_px(12.))
                            .text_color(rgba(theme.fg0))
                            .when(is_focused, |el| el.bg(rgba(theme.bg2)))
                            .hover(|style| style.bg(rgba(theme.bg2)))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                this.activate_acp_entry(profile.clone(), window, cx);
                                cx.notify();
                            }))
                            .child(panel_icon(CONNECT_ICON, theme.accent))
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
                                    .ml_auto()
                                    .max_w(ui_px(120.))
                                    .text_size(ui_px(10.))
                                    .text_color(rgba(theme.fg2))
                                    .child(command),
                            ),
                    );
                }
            }
            SessionCreationMode::Terminal => {
                for (index, preset) in presets.into_iter().enumerate() {
                    let is_focused =
                        focused_column == PaletteColumn::Presets && index == preset_index;
                    let label =
                        i18n::text(self.language, "palette.new").replace("{name}", &preset.label);
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
                            .hover(|style| style.bg(rgba(theme.bg2)))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                this.palette_open = false;
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
            }
        }

        let ui_selected = mode == SessionCreationMode::Ui;
        let terminal_selected = mode == SessionCreationMode::Terminal;
        let mode_switch = div()
            .flex()
            .border_1()
            .border_color(rgba(theme.line))
            .child(
                semantic_button(
                    "session-mode-ui",
                    i18n::text(self.language, "palette.ui_session"),
                    theme,
                )
                .px_3()
                .py_1()
                .text_size(ui_px(11.))
                .when(ui_selected, |button| {
                    button
                        .bg(rgba(theme.accent))
                        .text_color(rgba(theme.on_accent))
                })
                .when(!local_target, |button| button.text_color(rgba(theme.fg2)))
                .when(local_target, |button| {
                    button.on_click(cx.listener(|this, _event, _window, cx| {
                        this.session_creation_mode = SessionCreationMode::Ui;
                        this.palette_index = 0;
                        cx.notify();
                    }))
                })
                .child(i18n::text(self.language, "palette.ui_session")),
            )
            .child(
                semantic_button(
                    "session-mode-terminal",
                    i18n::text(self.language, "palette.terminal_session"),
                    theme,
                )
                .px_3()
                .py_1()
                .border_0()
                .border_l_1()
                .border_color(rgba(theme.line))
                .text_size(ui_px(11.))
                .when(terminal_selected, |button| {
                    button
                        .bg(rgba(theme.accent))
                        .text_color(rgba(theme.on_accent))
                })
                .on_click(cx.listener(|this, _event, _window, cx| {
                    this.session_creation_mode = SessionCreationMode::Terminal;
                    this.palette_index = 0;
                    cx.notify();
                }))
                .child(i18n::text(self.language, "palette.terminal_session")),
            );

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
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .border_b_1()
                    .border_color(rgba(theme.line))
                    .child(self.palette_input.clone())
                    .child(mode_switch),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_agent_enter_click_and_copy_never_create_or_execute() {
        use gpui::{AppContext, TestAppContext, VisualTestContext};
        use std::sync::Arc;
        let directory = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let state = muxlane_server::ServerState::new(muxlane_core::model::MachineInfo {
            machine_id: "catalog-test".into(),
            name: "test".into(),
            os: "test".into(),
            version: "test".into(),
        });
        let mut snapshot = state.snapshot();
        snapshot.projects.push(muxlane_core::model::Project {
            id: "project".into(),
            name: "Project".into(),
            path: directory.path().into(),
            branch: None,
            agents: vec![],
        });
        let server = muxlane_server::MuxlaneServer::new_with_runtime(
            directory.path().join("unused.sock"),
            Arc::new(tokio::sync::RwLock::new(state)),
            muxlane_server::DirtyFlag::new(),
            runtime.handle().clone(),
        );
        let missing = muxlane_acp::AgentDefinition {
            id: "missing".into(),
            label: "Missing Agent".into(),
            command: directory
                .path()
                .join("must-not-exist")
                .to_str()
                .unwrap()
                .into(),
            args: vec![],
            env: Default::default(),
        };
        let store = directory.path().join("state.json");
        let mut cx = TestAppContext::single();
        let window = cx.add_window(move |window, cx| {
            let mut app = MuxlaneApp::new(
                window,
                cx,
                server,
                snapshot,
                vec![],
                Default::default(),
                store,
            );
            app.acp_registry = muxlane_acp::AgentRegistry::from_definitions(vec![missing]).unwrap();
            app.acp_entries = app
                .acp_registry
                .entries(None, &muxlane_acp::LocalProbe::new(""))
                .into_iter()
                .filter(|entry| entry.id == "missing")
                .collect();
            // A fixed local recipe command, never a remote distribution value.
            app.acp_entries[0].install = Some("npm install -g opencode-ai");
            app.acp_entries[0].user_configured = false;
            app.new_session_target = Some(NewSessionTarget::Local("project".into()));
            app.session_creation_mode = SessionCreationMode::Ui;
            app.palette_open = true;
            app
        });
        let view = window.root(&mut cx).unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            view.update(cx, |app, cx| {
                assert!(app.handle_new_session_palette_key(
                    &gpui::Keystroke::parse("enter").unwrap(),
                    window,
                    cx
                ));
                assert_eq!(app.acp_install_detail.as_deref(), Some("missing"));
                assert!(app.acp_views.is_empty());
                assert!(app.acp_records.is_empty());
                assert!(app.new_session_target.is_some());
                app.acp_install_detail = None;
                cx.notify();
            });
        })
        .unwrap();
        let draw = |cx: &mut TestAppContext| {
            cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
            cx.update_window(window.into(), |_, window, cx| {
                window.simulate_next_frame(cx)
            })
            .unwrap();
            cx.run_until_parked();
        };
        draw(&mut cx);
        {
            let mut visual = VisualTestContext::from_window(window.into(), &mut cx);
            let bounds = visual.debug_bounds("acp-agent-row").unwrap();
            visual.simulate_click(bounds.center(), Default::default());
        }
        draw(&mut cx);
        cx.update(|cx| {
            let app = view.read(cx);
            assert_eq!(app.acp_install_detail.as_deref(), Some("missing"));
            assert!(app.acp_views.is_empty());
            assert!(app.acp_records.is_empty());
        });
        {
            let mut visual = VisualTestContext::from_window(window.into(), &mut cx);
            let bounds = visual.debug_bounds("acp-copy-install").unwrap();
            visual.simulate_click(bounds.center(), Default::default());
        }
        draw(&mut cx);
        cx.update(|cx| {
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                "npm install -g opencode-ai"
            );
            assert!(view.read(cx).acp_views.is_empty());
            assert!(view.read(cx).acp_records.is_empty());
        });
        assert!(!directory.path().join("must-not-exist").exists());
    }

    #[test]
    fn all_targets_default_to_terminal_sessions() {
        assert_eq!(
            SessionCreationMode::for_target(&NewSessionTarget::Remote {
                host: "host".into(),
                project: "project".into(),
            }),
            SessionCreationMode::Terminal
        );
        assert_eq!(
            SessionCreationMode::for_target(&NewSessionTarget::Local("project".into())),
            SessionCreationMode::Terminal
        );
    }
}
