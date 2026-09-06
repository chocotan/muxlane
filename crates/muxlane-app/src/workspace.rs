//! Project-scoped UI layout state. Agent and tmux lifecycles stay outside this module.
use muxlane_core::model::AgentId;
use muxlane_core::{PaneId, PaneNode};
use std::collections::{BTreeMap, HashSet};

use crate::app::MuxlaneApp;
use gpui::{Context, Window};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ProjectKey {
    pub(crate) machine_id: String,
    pub(crate) project_id: String,
}

impl ProjectKey {
    pub(crate) fn new(machine_id: impl Into<String>, project_id: impl Into<String>) -> Self {
        Self {
            machine_id: machine_id.into(),
            project_id: project_id.into(),
        }
    }
}

pub(crate) fn adjacent_project_target(
    ordered_projects: &[ProjectKey],
    current: Option<&ProjectKey>,
    next: bool,
) -> Option<ProjectKey> {
    if ordered_projects.len() < 2 {
        return None;
    }
    let index = current.and_then(|current| {
        ordered_projects
            .iter()
            .position(|candidate| candidate == current)
    });
    let target = match (index, next) {
        (Some(index), true) => (index + 1) % ordered_projects.len(),
        (Some(index), false) => (index + ordered_projects.len() - 1) % ordered_projects.len(),
        (None, true) => 0,
        (None, false) => ordered_projects.len() - 1,
    };
    Some(ordered_projects[target].clone())
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WorkspaceLayout {
    pub(crate) pane_tree: PaneNode,
    pub(crate) active_pane: PaneId,
}

impl WorkspaceLayout {
    pub(crate) fn new(mut pane_tree: PaneNode, active_pane: Option<PaneId>) -> Self {
        // Re-applying the tree's own tab set normalizes malformed persisted
        // group.active values without changing the pane structure.
        let tabs = pane_tree
            .all_groups()
            .into_iter()
            .flat_map(|group| group.tabs.iter().cloned())
            .collect();
        pane_tree.retain_agents(&tabs);
        let active_pane = active_pane
            .filter(|pane| pane_tree.group(pane).is_some())
            .unwrap_or_else(|| pane_tree.first_pane_id());
        Self {
            pane_tree,
            active_pane,
        }
    }

    fn projected(&self, agents: &HashSet<AgentId>) -> Self {
        let mut pane_tree = self.pane_tree.clone();
        pane_tree.retain_agents(agents);
        Self::new(pane_tree, Some(self.active_pane.clone()))
    }

    fn remove_agents(&mut self, agents: &HashSet<AgentId>) {
        let keep: HashSet<_> = self
            .pane_tree
            .all_groups()
            .into_iter()
            .flat_map(|group| group.tabs.iter().cloned())
            .filter(|agent| !agents.contains(agent))
            .collect();
        self.pane_tree.retain_agents(&keep);
        if self.pane_tree.group(&self.active_pane).is_none() {
            self.active_pane = self.pane_tree.first_pane_id();
        }
    }

    fn target_pane(&self, preferred: Option<&PaneId>) -> PaneId {
        preferred
            .filter(|pane| self.pane_tree.group(pane).is_some())
            .cloned()
            .unwrap_or_else(|| self.active_pane.clone())
    }

    fn place_agent(
        &mut self,
        agent: AgentId,
        preferred_pane: Option<&PaneId>,
        split_axis: Option<muxlane_core::SplitAxis>,
    ) {
        let pane = self.target_pane(preferred_pane);
        if let Some(axis) = split_axis {
            if let Some(new_pane) = self.pane_tree.split(&pane, axis, agent.clone()) {
                self.active_pane = new_pane;
                return;
            }
        }
        self.pane_tree.open_tab(&pane, agent);
        self.active_pane = pane;
    }
}

fn empty_layout() -> WorkspaceLayout {
    WorkspaceLayout::new(PaneNode::empty(), None)
}

fn active_tab_in_layout(pane_tree: &PaneNode, active_pane: &PaneId) -> Option<AgentId> {
    pane_tree
        .group(active_pane)
        .and_then(|group| group.active.clone().or_else(|| group.tabs.first().cloned()))
}

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceController {
    enabled: bool,
    shared: WorkspaceLayout,
    projects: BTreeMap<ProjectKey, WorkspaceLayout>,
    current_project: Option<ProjectKey>,
}

impl WorkspaceController {
    pub(crate) fn from_persisted(persisted: &muxlane_store::PersistedApp) -> Self {
        let shared =
            WorkspaceLayout::new(persisted.pane_tree.clone(), persisted.active_pane.clone());
        let projects = persisted
            .project_workspaces
            .iter()
            .map(|record| {
                (
                    ProjectKey::new(record.key.machine_id.clone(), record.key.project_id.clone()),
                    WorkspaceLayout::new(record.pane_tree.clone(), record.active_pane.clone()),
                )
            })
            .collect();
        Self {
            enabled: persisted.project_workspaces_enabled,
            shared,
            projects,
            current_project: persisted
                .active_project_workspace
                .as_ref()
                .map(|key| ProjectKey::new(key.machine_id.clone(), key.project_id.clone())),
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn current_project(&self) -> Option<&ProjectKey> {
        self.current_project.as_ref()
    }

    pub(crate) fn initial_layout(&mut self, selected: Option<ProjectKey>) -> WorkspaceLayout {
        self.current_project = selected.clone();
        if self.enabled {
            if let Some(key) = selected {
                return self
                    .projects
                    .entry(key)
                    .or_insert_with(empty_layout)
                    .clone();
            }
        }
        self.shared.clone()
    }

    pub(crate) fn switch_project(
        &mut self,
        key: ProjectKey,
        current: WorkspaceLayout,
    ) -> Option<WorkspaceLayout> {
        self.save_current(current);
        if self.current_project.as_ref() == Some(&key) {
            return None;
        }
        self.current_project = Some(key.clone());
        self.layout_for_project(key)
    }

    fn layout_for_project(&mut self, key: ProjectKey) -> Option<WorkspaceLayout> {
        self.enabled.then(|| {
            self.projects
                .entry(key)
                .or_insert_with(empty_layout)
                .clone()
        })
    }

    pub(crate) fn set_enabled(
        &mut self,
        enabled: bool,
        current: WorkspaceLayout,
        selected: Option<ProjectKey>,
        project_agents: &HashSet<AgentId>,
    ) -> Option<WorkspaceLayout> {
        self.save_current(current);
        if self.enabled == enabled {
            return None;
        }
        self.enabled = enabled;
        if enabled {
            if selected.is_some() {
                self.current_project = selected;
            }
            self.current_project.clone().map(|key| {
                self.projects
                    .entry(key)
                    .or_insert_with(|| self.shared.projected(project_agents))
                    .clone()
            })
        } else {
            Some(self.shared.clone())
        }
    }

    pub(crate) fn save_current(&mut self, layout: WorkspaceLayout) {
        if self.enabled {
            if let Some(key) = self.current_project.clone() {
                self.projects.insert(key, layout);
            }
        } else {
            self.shared = layout;
        }
    }

    pub(crate) fn remove_agents(&mut self, agents: &HashSet<AgentId>) {
        self.shared.remove_agents(agents);
        for layout in self.projects.values_mut() {
            layout.remove_agents(agents);
        }
    }

    pub(crate) fn remove_project(
        &mut self,
        key: &ProjectKey,
        current: WorkspaceLayout,
        next: Option<ProjectKey>,
    ) -> Option<WorkspaceLayout> {
        self.save_current(current);
        let removed_current = self.current_project.as_ref() == Some(key);
        self.projects.remove(key);
        if !removed_current {
            return None;
        }
        self.current_project = None;
        if !self.enabled {
            return None;
        }
        Some(self.select_after_removal(next))
    }

    pub(crate) fn reconcile_machine_projects(
        &mut self,
        machine_id: &str,
        valid_projects: &HashSet<String>,
        current: WorkspaceLayout,
        next: Option<ProjectKey>,
    ) -> (bool, Option<WorkspaceLayout>) {
        self.save_current(current);
        let previous_len = self.projects.len();
        self.projects.retain(|key, _| {
            key.machine_id != machine_id || valid_projects.contains(&key.project_id)
        });
        let changed = self.projects.len() != previous_len;
        let removed_current = self.current_project.as_ref().is_some_and(|key| {
            key.machine_id == machine_id && !valid_projects.contains(&key.project_id)
        });
        if !removed_current {
            return (changed, None);
        }
        self.current_project = None;
        if !self.enabled {
            return (true, None);
        }
        (true, Some(self.select_after_removal(next)))
    }

    pub(crate) fn remove_machine(
        &mut self,
        machine_id: &str,
        current: WorkspaceLayout,
        next: Option<ProjectKey>,
    ) -> Option<WorkspaceLayout> {
        self.save_current(current);
        let removed_current = self
            .current_project
            .as_ref()
            .is_some_and(|key| key.machine_id == machine_id);
        self.projects.retain(|key, _| key.machine_id != machine_id);
        if !removed_current {
            return None;
        }
        self.current_project = None;
        if !self.enabled {
            return None;
        }
        Some(self.select_after_removal(next))
    }

    fn select_after_removal(&mut self, next: Option<ProjectKey>) -> WorkspaceLayout {
        let Some(key) = next else {
            return empty_layout();
        };
        self.current_project = Some(key.clone());
        self.projects
            .entry(key)
            .or_insert_with(empty_layout)
            .clone()
    }

    pub(crate) fn prepare_spawn_target(
        &mut self,
        key: &ProjectKey,
        current: WorkspaceLayout,
        preferred_pane: Option<&PaneId>,
    ) -> PaneId {
        if !self.enabled {
            return current.target_pane(preferred_pane);
        }
        if self.current_project.as_ref() == Some(key) {
            self.projects.insert(key.clone(), current.clone());
            return current.target_pane(preferred_pane);
        }
        self.projects
            .entry(key.clone())
            .or_insert_with(empty_layout)
            .target_pane(preferred_pane)
    }

    pub(crate) fn place_agent_in_project(
        &mut self,
        key: &ProjectKey,
        agent: AgentId,
        preferred_pane: Option<&PaneId>,
        split_axis: Option<muxlane_core::SplitAxis>,
    ) {
        let layout = self
            .projects
            .entry(key.clone())
            .or_insert_with(|| WorkspaceLayout::new(PaneNode::empty(), None));
        layout.place_agent(agent, preferred_pane, split_axis);
    }

    pub(crate) fn should_activate_async_result(&self, key: &ProjectKey) -> bool {
        !self.enabled || self.current_project.as_ref() == Some(key)
    }

    pub(crate) fn known_agents_for_machine(&self, machine_id: &str) -> HashSet<AgentId> {
        self.projects
            .iter()
            .filter(|(key, _)| key.machine_id == machine_id)
            .flat_map(|(_, layout)| layout.pane_tree.all_groups())
            .flat_map(|group| group.tabs.iter().cloned())
            .collect()
    }

    pub(crate) fn stale_agents_for_machine(
        &self,
        machine_id: &str,
        valid: &HashSet<AgentId>,
        previously_known: impl IntoIterator<Item = AgentId>,
    ) -> HashSet<AgentId> {
        let mut known = self.known_agents_for_machine(machine_id);
        known.extend(previously_known);
        known.difference(valid).cloned().collect()
    }

    pub(crate) fn write_persisted(
        &self,
        app: &mut muxlane_store::PersistedApp,
        current: WorkspaceLayout,
    ) {
        let mut controller = self.clone();
        controller.save_current(current);
        app.project_workspaces_enabled = controller.enabled;
        app.active_project_workspace =
            controller
                .current_project
                .map(|key| muxlane_store::PersistedProjectKey {
                    machine_id: key.machine_id,
                    project_id: key.project_id,
                });
        app.pane_tree = controller.shared.pane_tree;
        app.active_pane = Some(controller.shared.active_pane);
        app.project_workspaces = controller
            .projects
            .into_iter()
            .map(|(key, layout)| muxlane_store::PersistedWorkspace {
                key: muxlane_store::PersistedProjectKey {
                    machine_id: key.machine_id,
                    project_id: key.project_id,
                },
                pane_tree: layout.pane_tree,
                active_pane: Some(layout.active_pane),
            })
            .collect();
    }
}

impl MuxlaneApp {
    pub(crate) fn current_workspace_layout(&self) -> WorkspaceLayout {
        WorkspaceLayout::new(self.pane_tree.clone(), Some(self.active_pane.clone()))
    }

    pub(crate) fn local_machine_id(&self) -> String {
        self.last_snapshot
            .machine
            .as_ref()
            .map(|machine| machine.machine_id.clone())
            .unwrap_or_else(|| "local".into())
    }

    pub(crate) fn project_key_for_agent(&self, agent: &AgentId) -> Option<ProjectKey> {
        if let Some(metadata) = self.acp_metadata.get(agent) {
            return Some(ProjectKey::new(
                self.local_machine_id(),
                metadata.project_id.clone(),
            ));
        }
        if let Some(instance) = self.last_snapshot.agent(agent) {
            return Some(ProjectKey::new(
                self.local_machine_id(),
                instance.project.clone(),
            ));
        }
        self.remote_snaps.values().find_map(|snapshot| {
            let instance = snapshot.agent(agent)?;
            let machine_id = snapshot.machine.as_ref()?.machine_id.clone();
            Some(ProjectKey::new(machine_id, instance.project.clone()))
        })
    }

    pub(crate) fn project_agents(&self, key: &ProjectKey) -> HashSet<AgentId> {
        let local_machine_id = self.local_machine_id();
        if key.machine_id == local_machine_id {
            return self
                .last_snapshot
                .agents
                .iter()
                .filter(|agent| agent.project == key.project_id)
                .map(|agent| agent.id.clone())
                .chain(
                    self.acp_metadata
                        .iter()
                        .filter(|(_, thread)| thread.project_id == key.project_id)
                        .map(|(id, _)| id.clone()),
                )
                .collect();
        }
        self.remote_snaps
            .values()
            .find(|snapshot| {
                snapshot
                    .machine
                    .as_ref()
                    .is_some_and(|machine| machine.machine_id == key.machine_id)
            })
            .map(|snapshot| {
                snapshot
                    .agents
                    .iter()
                    .filter(|agent| agent.project == key.project_id)
                    .map(|agent| agent.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn apply_workspace_layout(&mut self, layout: WorkspaceLayout) {
        self.pane_tree = layout.pane_tree;
        self.active_pane = layout.active_pane;
        self.active = active_tab_in_layout(&self.pane_tree, &self.active_pane);
        self.maximized_pane = None;
        self.split_drag = None;
        self.sidebar.end_drag();
        if let Ok(mut metrics) = self.split_metrics.lock() {
            metrics.clear();
        }
    }

    pub(crate) fn select_project_workspace(
        &mut self,
        key: ProjectKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_project_workspace_inner(key, cx);
        self.focus_current_workspace(window, cx);
    }

    fn focus_current_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(active) = self
            .active
            .clone()
            .filter(|agent| self.terms.contains_key(agent) || self.acp_views.contains_key(agent))
        {
            self.focus_agent(&active, window, cx);
        } else {
            self.focus.focus(window, cx);
        }
    }

    pub(crate) fn select_project_workspace_inner(
        &mut self,
        key: ProjectKey,
        cx: &mut Context<Self>,
    ) {
        let current = self.current_workspace_layout();
        if let Some(layout) = self.workspace.switch_project(key, current) {
            self.apply_workspace_layout(layout);
        }
        self.persist();
        cx.notify();
    }

    pub(crate) fn capture_spawn_target(
        &mut self,
        key: &ProjectKey,
        preferred_pane: Option<&PaneId>,
    ) -> PaneId {
        let current = self.current_workspace_layout();
        self.workspace
            .prepare_spawn_target(key, current, preferred_pane)
    }

    pub(crate) fn remote_host_for_key(&self, key: &ProjectKey) -> Option<String> {
        self.remote_snaps.iter().find_map(|(host, snapshot)| {
            snapshot
                .machine
                .as_ref()
                .is_some_and(|machine| machine.machine_id == key.machine_id)
                .then(|| host.clone())
        })
    }

    pub(crate) fn available_project_keys(&self) -> Vec<ProjectKey> {
        let local_machine_id = self.local_machine_id();
        let mut keys: Vec<_> = self
            .ordered_projects(&local_machine_id, &self.last_snapshot.projects)
            .into_iter()
            .map(|project| ProjectKey::new(local_machine_id.clone(), project.id.clone()))
            .collect();
        for remote in &self.remotes {
            let host = &remote.cfg.name;
            if !matches!(
                self.remote_states.get(host),
                Some(muxlane_client::RemoteState::Online(_))
            ) {
                continue;
            }
            let Some(snapshot) = self.remote_snaps.get(host) else {
                continue;
            };
            let Some(machine) = snapshot.machine.as_ref() else {
                continue;
            };
            keys.extend(
                self.ordered_projects(&machine.machine_id, &snapshot.projects)
                    .into_iter()
                    .map(|project| ProjectKey::new(machine.machine_id.clone(), project.id.clone())),
            );
        }
        keys
    }

    pub(crate) fn remove_project_workspace(&mut self, key: &ProjectKey) {
        let next = self
            .available_project_keys()
            .into_iter()
            .find(|candidate| candidate != key);
        let current = self.current_workspace_layout();
        if let Some(layout) = self.workspace.remove_project(key, current, next) {
            self.apply_workspace_layout(layout);
        }
    }

    pub(crate) fn reconcile_machine_workspaces(
        &mut self,
        machine_id: &str,
        valid_projects: &HashSet<String>,
    ) -> bool {
        let next = self.available_project_keys().into_iter().find(|candidate| {
            candidate.machine_id != machine_id || valid_projects.contains(&candidate.project_id)
        });
        let current = self.current_workspace_layout();
        let (changed, layout) =
            self.workspace
                .reconcile_machine_projects(machine_id, valid_projects, current, next);
        if let Some(layout) = layout {
            self.apply_workspace_layout(layout);
        }
        changed
    }

    pub(crate) fn remove_machine_workspaces(&mut self, machine_id: &str) {
        let next = self
            .available_project_keys()
            .into_iter()
            .find(|candidate| candidate.machine_id != machine_id);
        let current = self.current_workspace_layout();
        if let Some(layout) = self.workspace.remove_machine(machine_id, current, next) {
            self.apply_workspace_layout(layout);
        }
    }

    /// 用户主动 spawn 完成后的跳转：目标项目不是当前工作区时先切过去，
    /// 否则 place_async_agent 会把新会话放到后台不激活。
    pub(crate) fn jump_to_project_if_needed(&mut self, key: &ProjectKey, cx: &mut Context<Self>) {
        if self.workspace.enabled() && self.workspace.current_project() != Some(key) {
            self.select_project_workspace_inner(key.clone(), cx);
        }
    }

    pub(crate) fn place_async_agent(
        &mut self,
        key: &ProjectKey,
        agent: AgentId,
        preferred_pane: Option<PaneId>,
        split_axis: Option<muxlane_core::SplitAxis>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workspace.should_activate_async_result(key) {
            self.workspace
                .place_agent_in_project(key, agent, preferred_pane.as_ref(), split_axis);
            self.persist();
            cx.notify();
            return;
        }

        let pane = preferred_pane
            .filter(|pane| self.pane_tree.group(pane).is_some())
            .unwrap_or_else(|| self.active_pane.clone());
        let activation_pane = if let Some(axis) = split_axis {
            if let Some(new_pane) = self.pane_tree.split(&pane, axis, agent.clone()) {
                self.maximized_pane = None;
                new_pane
            } else {
                pane
            }
        } else {
            pane
        };
        self.activate_agent(&activation_pane, &agent, window, cx);
        self.persist();
        cx.notify();
    }

    pub(crate) fn set_project_workspaces_enabled(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = self
            .workspace
            .current_project()
            .cloned()
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
        let agents = selected
            .as_ref()
            .map(|key| self.project_agents(key))
            .unwrap_or_default();
        let current = self.current_workspace_layout();
        if let Some(layout) = self
            .workspace
            .set_enabled(enabled, current, selected, &agents)
        {
            self.apply_workspace_layout(layout);
            if let Some(active) = self.active.clone() {
                let pane = self.active_pane.clone();
                self.activate_agent(&pane, &active, window, cx);
            }
        }
        self.persist();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxlane_core::{SplitAxis, TabGroup};

    fn layout_with(agent: &str) -> WorkspaceLayout {
        let tree = PaneNode::with_tab(agent.into());
        WorkspaceLayout::new(tree.clone(), Some(tree.first_pane_id()))
    }

    fn controller(enabled: bool, current: Option<ProjectKey>) -> WorkspaceController {
        WorkspaceController {
            enabled,
            shared: layout_with("shared"),
            projects: BTreeMap::new(),
            current_project: current,
        }
    }

    fn contains(layout: &WorkspaceLayout, agent: &str) -> bool {
        layout.pane_tree.pane_for_agent(&agent.into()).is_some()
    }

    #[test]
    fn adjacent_projects_wrap_at_both_boundaries_in_available_order() {
        let projects = vec![
            ProjectKey::new("local", "first"),
            ProjectKey::new("local", "middle"),
            ProjectKey::new("remote", "last"),
        ];
        assert_eq!(
            adjacent_project_target(&projects, Some(&projects[0]), false),
            Some(projects[2].clone())
        );
        assert_eq!(
            adjacent_project_target(&projects, Some(&projects[2]), true),
            Some(projects[0].clone())
        );
        assert_eq!(
            adjacent_project_target(&projects, Some(&projects[1]), false),
            Some(projects[0].clone())
        );
        assert_eq!(
            adjacent_project_target(&projects, Some(&projects[1]), true),
            Some(projects[2].clone())
        );
    }

    #[test]
    fn adjacent_project_target_handles_single_or_empty_project_lists() {
        let single = vec![ProjectKey::new("local", "only")];
        assert_eq!(
            adjacent_project_target(&single, Some(&single[0]), true),
            None
        );
        assert_eq!(adjacent_project_target(&[], None, false), None);
        assert_eq!(
            adjacent_project_target(&[ProjectKey::new("local", "only")], None, false),
            None
        );
    }

    #[test]
    fn switching_to_an_empty_project_restores_an_empty_active_pane_safely() {
        let first = ProjectKey::new("local", "first");
        let empty = ProjectKey::new("local", "empty");
        let mut controller = controller(true, Some(first.clone()));
        controller
            .projects
            .insert(first.clone(), layout_with("first"));
        controller
            .projects
            .insert(empty.clone(), WorkspaceLayout::new(PaneNode::empty(), None));

        let layout = controller
            .switch_project(empty.clone(), layout_with("first"))
            .unwrap();
        assert_eq!(controller.current_project(), Some(&empty));
        let group = layout.pane_tree.group(&layout.active_pane).unwrap();
        assert!(group.tabs.is_empty());
        assert!(group.active.is_none());
    }

    #[test]
    fn focus_target_uses_only_the_current_layout() {
        let layout = PaneNode::with_tab("layout-agent".into());
        let pane = layout.first_pane_id();
        assert_eq!(
            active_tab_in_layout(&layout, &pane),
            Some("layout-agent".into())
        );

        let empty = PaneNode::empty();
        let pane = empty.first_pane_id();
        assert_eq!(active_tab_in_layout(&empty, &pane), None);
    }

    #[test]
    fn toggling_roundtrips_shared_and_project_layouts() {
        let key = ProjectKey::new("m", "p");
        let mut controller = controller(false, Some(key.clone()));
        let project = controller
            .set_enabled(
                true,
                layout_with("shared-edited"),
                Some(key.clone()),
                &HashSet::from(["shared-edited".into()]),
            )
            .unwrap();
        assert!(contains(&project, "shared-edited"));

        let shared = controller
            .set_enabled(false, layout_with("project-edited"), None, &HashSet::new())
            .unwrap();
        assert!(contains(&shared, "shared-edited"));

        let restored = controller
            .set_enabled(true, shared, Some(key), &HashSet::new())
            .unwrap();
        assert!(contains(&restored, "project-edited"));
    }

    #[test]
    fn projects_are_independent_and_machine_is_part_of_the_key() {
        let local = ProjectKey::new("local", "same-id");
        let remote = ProjectKey::new("remote", "same-id");
        let mut controller = controller(true, Some(local.clone()));
        controller
            .projects
            .insert(local.clone(), layout_with("local-agent"));
        controller
            .projects
            .insert(remote.clone(), layout_with("remote-agent"));

        let remote_layout = controller
            .switch_project(remote.clone(), layout_with("local-edited"))
            .unwrap();
        assert!(contains(&remote_layout, "remote-agent"));
        assert!(!contains(&remote_layout, "local-edited"));

        let local_layout = controller
            .switch_project(local.clone(), layout_with("remote-edited"))
            .unwrap();
        assert!(contains(&local_layout, "local-edited"));
        assert!(contains(
            controller.projects.get(&remote).unwrap(),
            "remote-edited"
        ));
    }

    #[test]
    fn first_project_layout_starts_empty() {
        let mut shared_tree = PaneNode::with_tab("keep".into());
        let first = shared_tree.first_pane_id();
        shared_tree
            .split(&first, SplitAxis::Horizontal, "drop".into())
            .unwrap();
        let shared = WorkspaceLayout::new(shared_tree, None);
        let key = ProjectKey::new("m", "p");
        let mut controller = WorkspaceController {
            enabled: true,
            shared,
            projects: BTreeMap::new(),
            current_project: None,
        };

        let initial = controller.initial_layout(Some(key));
        assert_eq!(initial.pane_tree.leaf_count(), 1);
        assert!(initial.pane_tree.all_groups()[0].tabs.is_empty());
    }

    #[test]
    fn removing_agent_cleans_shared_and_every_project_layout() {
        let mut controller = WorkspaceController {
            enabled: true,
            shared: layout_with("gone"),
            projects: BTreeMap::from([
                (ProjectKey::new("m", "a"), layout_with("gone")),
                (ProjectKey::new("m", "b"), layout_with("kept")),
            ]),
            current_project: Some(ProjectKey::new("m", "a")),
        };
        controller.remove_agents(&HashSet::from(["gone".into()]));
        assert!(!contains(&controller.shared, "gone"));
        assert!(!contains(
            controller.projects.get(&ProjectKey::new("m", "a")).unwrap(),
            "gone"
        ));
        assert!(contains(
            controller.projects.get(&ProjectKey::new("m", "b")).unwrap(),
            "kept"
        ));
    }

    #[test]
    fn removing_current_project_selects_next_or_empty_layout() {
        let a = ProjectKey::new("m", "a");
        let b = ProjectKey::new("m", "b");
        let mut controller = controller(true, Some(a.clone()));
        controller.projects.insert(a.clone(), layout_with("a"));
        controller.projects.insert(b.clone(), layout_with("b"));

        let next = controller
            .remove_project(&a, layout_with("a-edited"), Some(b.clone()))
            .unwrap();
        assert_eq!(controller.current_project(), Some(&b));
        assert!(contains(&next, "b"));
        assert!(!controller.projects.contains_key(&a));

        let empty = controller
            .remove_project(&b, next, None)
            .expect("enabled mode needs an empty fallback");
        assert!(controller.current_project().is_none());
        assert_eq!(empty.pane_tree.leaf_count(), 1);
        assert!(empty.pane_tree.all_groups()[0].tabs.is_empty());
    }

    #[test]
    fn removing_machine_only_drops_its_project_records() {
        let a = ProjectKey::new("machine-a", "p");
        let b = ProjectKey::new("machine-b", "p");
        let mut controller = controller(true, Some(a.clone()));
        controller.projects.insert(a, layout_with("a"));
        controller.projects.insert(b.clone(), layout_with("b"));

        let next = controller
            .remove_machine("machine-a", layout_with("a-edited"), Some(b.clone()))
            .unwrap();
        assert_eq!(controller.current_project(), Some(&b));
        assert!(contains(&next, "b"));
        assert!(controller
            .projects
            .keys()
            .all(|key| key.machine_id != "machine-a"));
    }

    #[test]
    fn async_result_for_background_project_keeps_current_project_untouched() {
        let a = ProjectKey::new("m", "a");
        let b = ProjectKey::new("m", "b");
        let mut controller = controller(true, Some(a.clone()));
        let pane = controller.prepare_spawn_target(&a, layout_with("a"), None);
        let b_layout = controller
            .switch_project(b.clone(), layout_with("a"))
            .unwrap();
        assert!(!controller.should_activate_async_result(&a));

        controller.place_agent_in_project(&a, "new-a".into(), Some(&pane), None);
        assert_eq!(controller.current_project(), Some(&b));
        assert!(!contains(&b_layout, "new-a"));
        assert!(contains(controller.projects.get(&a).unwrap(), "new-a"));
    }

    #[test]
    fn background_split_uses_captured_pane() {
        let a = ProjectKey::new("m", "a");
        let mut controller = controller(true, Some(ProjectKey::new("m", "b")));
        controller
            .projects
            .insert(a.clone(), layout_with("existing"));
        let captured = controller.projects.get(&a).unwrap().active_pane.clone();
        controller.place_agent_in_project(
            &a,
            "split-agent".into(),
            Some(&captured),
            Some(SplitAxis::Vertical),
        );
        let layout = controller.projects.get(&a).unwrap();
        assert_eq!(layout.pane_tree.leaf_count(), 2);
        assert!(contains(layout, "split-agent"));
        assert_ne!(layout.active_pane, captured);
    }

    #[test]
    fn invalid_active_pane_and_tab_are_normalized() {
        let tree = PaneNode::Leaf {
            group: TabGroup {
                id: "pane-valid".into(),
                tabs: vec!["agent".into()],
                active: Some("missing".into()),
            },
        };
        let layout = WorkspaceLayout::new(tree, Some("pane-missing".into()));
        assert_eq!(layout.active_pane, "pane-valid");
        assert_eq!(
            layout
                .pane_tree
                .group(&layout.active_pane)
                .and_then(|group| group.active.as_deref()),
            Some("agent")
        );
    }

    #[test]
    fn authoritative_machine_agents_only_mark_that_machines_stale_tabs() {
        let mut controller = controller(true, Some(ProjectKey::new("machine-a", "p")));
        controller
            .projects
            .insert(ProjectKey::new("machine-a", "p"), layout_with("stale-a"));
        controller
            .projects
            .insert(ProjectKey::new("machine-b", "p"), layout_with("keep-b"));
        let stale = controller.stale_agents_for_machine(
            "machine-a",
            &HashSet::from(["live-a".into()]),
            ["prior-a".into()],
        );
        assert_eq!(
            stale,
            HashSet::from(["stale-a".to_string(), "prior-a".to_string()])
        );
        assert!(!stale.contains("keep-b"));
    }

    #[test]
    fn authoritative_project_reconcile_removes_stale_current_and_applies_fallback() {
        let removed = ProjectKey::new("remote", "deleted");
        let kept = ProjectKey::new("remote", "kept");
        let local = ProjectKey::new("local", "local-project");
        let mut controller = controller(true, Some(removed.clone()));
        controller
            .projects
            .insert(removed.clone(), layout_with("deleted-agent"));
        controller
            .projects
            .insert(kept.clone(), layout_with("kept-agent"));
        controller
            .projects
            .insert(local.clone(), layout_with("local-agent"));

        let (changed, fallback) = controller.reconcile_machine_projects(
            "remote",
            &HashSet::from(["kept".to_string()]),
            layout_with("deleted-hot"),
            Some(local.clone()),
        );

        assert!(changed);
        assert!(!controller.projects.contains_key(&removed));
        assert!(controller.projects.contains_key(&kept));
        assert_eq!(controller.current_project(), Some(&local));
        assert!(contains(&fallback.unwrap(), "local-agent"));
    }

    #[test]
    fn authoritative_project_reconcile_keeps_valid_current_layout_hot() {
        let current = ProjectKey::new("remote", "kept");
        let stale = ProjectKey::new("remote", "deleted");
        let mut controller = controller(true, Some(current.clone()));
        controller
            .projects
            .insert(current.clone(), layout_with("old-current"));
        controller
            .projects
            .insert(stale.clone(), layout_with("stale"));

        let (changed, fallback) = controller.reconcile_machine_projects(
            "remote",
            &HashSet::from(["kept".to_string()]),
            layout_with("current-hot"),
            None,
        );

        assert!(changed);
        assert!(fallback.is_none());
        assert_eq!(controller.current_project(), Some(&current));
        assert!(contains(
            controller.projects.get(&current).unwrap(),
            "current-hot"
        ));
        assert!(!controller.projects.contains_key(&stale));
    }

    #[test]
    fn disabled_mode_keeps_async_results_on_the_shared_hot_layout() {
        let target = ProjectKey::new("m", "p");
        let mut controller = controller(false, Some(ProjectKey::new("m", "other")));
        let hot = layout_with("shared-hot");
        let pane = controller.prepare_spawn_target(&target, hot.clone(), Some(&hot.active_pane));
        assert_eq!(pane, hot.active_pane);
        assert!(controller.should_activate_async_result(&target));
        assert!(!controller.projects.contains_key(&target));
    }

    #[test]
    fn persistence_stashes_hot_layout_in_the_correct_slot() {
        let key = ProjectKey::new("machine", "project");
        let controller = controller(true, Some(key.clone()));
        let mut persisted = muxlane_store::PersistedApp::default();
        controller.write_persisted(&mut persisted, layout_with("hot"));

        assert!(persisted.project_workspaces_enabled);
        assert_eq!(
            persisted.active_project_workspace,
            Some(muxlane_store::PersistedProjectKey {
                machine_id: "machine".into(),
                project_id: "project".into(),
            })
        );
        assert_eq!(persisted.project_workspaces.len(), 1);
        assert!(persisted.project_workspaces[0]
            .pane_tree
            .pane_for_agent(&"hot".into())
            .is_some());

        let restored = WorkspaceController::from_persisted(&persisted);
        assert_eq!(restored.current_project(), Some(&key));
        assert!(contains(restored.projects.get(&key).unwrap(), "hot"));
    }
}
