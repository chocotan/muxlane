import type { AgentInstance, Project } from "../shared/protocol";

export interface TabGroupState {
  tabs: string[];
  activeAgentId: string | null;
  activeDshProjectId: string | null;
}

export interface HostWorkspaceTabsState {
  initialized: boolean;
  activeProjectId: string | null;
  shared: TabGroupState;
  byProject: Record<string, TabGroupState>;
}

export interface VisibleWorkspaceView {
  activeProjectId: string | null;
  openTabs: string[];
  activeAgentId: string | null;
  visibleDshProjects: string[];
  activeDshProjectId: string | null;
}

const STORAGE_PREFIX = "muxlane.workspace.tabs.v1:";

export const EMPTY_TAB_GROUP: TabGroupState = {
  tabs: [],
  activeAgentId: null,
  activeDshProjectId: null,
};

export function createEmptyWorkspaceTabsState(): HostWorkspaceTabsState {
  return {
    initialized: false,
    activeProjectId: null,
    shared: { ...EMPTY_TAB_GROUP, tabs: [] },
    byProject: {},
  };
}

function sanitizeGroup(raw: Partial<TabGroupState> | undefined): TabGroupState {
  const tabs = Array.isArray(raw?.tabs)
    ? Array.from(new Set(raw.tabs.filter((x): x is string => typeof x === "string" && x.length > 0)))
    : [];
  const activeAgentId =
    typeof raw?.activeAgentId === "string" && tabs.includes(raw.activeAgentId)
      ? raw.activeAgentId
      : tabs[0] ?? null;
  const activeDshProjectId =
    typeof raw?.activeDshProjectId === "string" && raw.activeDshProjectId.length > 0
      ? raw.activeDshProjectId
      : null;
  return { tabs, activeAgentId, activeDshProjectId };
}

export function loadHostWorkspaceTabs(hostId: string): HostWorkspaceTabsState {
  try {
    const raw = window.localStorage.getItem(STORAGE_PREFIX + hostId);
    if (!raw) return createEmptyWorkspaceTabsState();
    const parsed = JSON.parse(raw) as Partial<HostWorkspaceTabsState>;
    const byProject: Record<string, TabGroupState> = {};
    if (parsed.byProject && typeof parsed.byProject === "object") {
      for (const [pid, group] of Object.entries(parsed.byProject)) {
        if (pid) {
          byProject[pid] = sanitizeGroup(group);
        }
      }
    }
    return {
      initialized: parsed.initialized === true,
      activeProjectId: typeof parsed.activeProjectId === "string" ? parsed.activeProjectId : null,
      shared: sanitizeGroup(parsed.shared),
      byProject,
    };
  } catch {
    return createEmptyWorkspaceTabsState();
  }
}

export function saveHostWorkspaceTabs(hostId: string, state: HostWorkspaceTabsState): void {
  try {
    window.localStorage.setItem(STORAGE_PREFIX + hostId, JSON.stringify(state));
  } catch {
    // ignore storage errors
  }
}

export function getVisibleWorkspaceView(
  state: HostWorkspaceTabsState,
  projectWorkspacesEnabled: boolean,
  openDshProjects: readonly string[],
  agents: readonly AgentInstance[],
): VisibleWorkspaceView {
  const agentProjectMap = new Map<string, string>();
  for (const a of agents) {
    agentProjectMap.set(a.id, a.project);
  }

  if (projectWorkspacesEnabled) {
    const pid = state.activeProjectId;
    const group = pid ? state.byProject[pid] ?? EMPTY_TAB_GROUP : EMPTY_TAB_GROUP;
    const openTabs = pid
      ? group.tabs.filter((id) => agentProjectMap.get(id) === pid)
      : [];
    const visibleDshProjects = pid && openDshProjects.includes(pid) ? [pid] : [];
    const activeDshProjectId =
      group.activeDshProjectId && visibleDshProjects.includes(group.activeDshProjectId)
        ? group.activeDshProjectId
        : null;
    const activeAgentId = activeDshProjectId
      ? null
      : group.activeAgentId && openTabs.includes(group.activeAgentId)
        ? group.activeAgentId
        : openTabs[0] ?? null;
    return {
      activeProjectId: pid,
      openTabs,
      activeAgentId,
      visibleDshProjects,
      activeDshProjectId,
    };
  }

  const openTabs = state.shared.tabs.filter((id) => agentProjectMap.has(id));
  const visibleDshProjects = openDshProjects.slice();
  const activeDshProjectId =
    state.shared.activeDshProjectId && visibleDshProjects.includes(state.shared.activeDshProjectId)
      ? state.shared.activeDshProjectId
      : null;
  const activeAgentId = activeDshProjectId
    ? null
    : state.shared.activeAgentId && openTabs.includes(state.shared.activeAgentId)
      ? state.shared.activeAgentId
      : openTabs[0] ?? null;
  const derivedProjectId =
    activeDshProjectId ??
    (activeAgentId ? agentProjectMap.get(activeAgentId) ?? state.activeProjectId : state.activeProjectId);

  return {
    activeProjectId: derivedProjectId,
    openTabs,
    activeAgentId,
    visibleDshProjects,
    activeDshProjectId,
  };
}

export function selectProjectWorkspace(
  state: HostWorkspaceTabsState,
  projectId: string,
): HostWorkspaceTabsState {
  if (state.activeProjectId === projectId && state.byProject[projectId]) {
    return state;
  }
  const existing = state.byProject[projectId];
  return {
    ...state,
    initialized: true,
    activeProjectId: projectId,
    byProject: existing
      ? state.byProject
      : {
          ...state.byProject,
          [projectId]: { ...EMPTY_TAB_GROUP, tabs: [] },
        },
  };
}

export function openAgentInWorkspace(
  state: HostWorkspaceTabsState,
  agentId: string,
  agentProjectId: string | null,
  projectWorkspacesEnabled: boolean,
): HostWorkspaceTabsState {
  const targetProjectId = agentProjectId ?? state.activeProjectId;

  if (projectWorkspacesEnabled && targetProjectId) {
    const prevGroup = state.byProject[targetProjectId] ?? EMPTY_TAB_GROUP;
    const nextTabs = prevGroup.tabs.includes(agentId)
      ? prevGroup.tabs
      : [...prevGroup.tabs, agentId];
    return {
      ...state,
      initialized: true,
      activeProjectId: targetProjectId,
      byProject: {
        ...state.byProject,
        [targetProjectId]: {
          tabs: nextTabs,
          activeAgentId: agentId,
          activeDshProjectId: null,
        },
      },
    };
  }

  const nextSharedTabs = state.shared.tabs.includes(agentId)
    ? state.shared.tabs
    : [...state.shared.tabs, agentId];
  return {
    ...state,
    initialized: true,
    activeProjectId: targetProjectId,
    shared: {
      tabs: nextSharedTabs,
      activeAgentId: agentId,
      activeDshProjectId: null,
    },
  };
}

function removeAgentFromGroup(
  group: TabGroupState,
  agentId: string,
  fallbackDshProjectId: string | null,
): TabGroupState {
  const idx = group.tabs.indexOf(agentId);
  if (idx < 0) return group;
  const nextTabs = group.tabs.filter((id) => id !== agentId);
  let nextActive = group.activeAgentId;
  let nextDsh = group.activeDshProjectId;
  if (group.activeAgentId === agentId) {
    nextActive = nextTabs[idx] ?? nextTabs[nextTabs.length - 1] ?? null;
    if (!nextActive && fallbackDshProjectId) {
      nextDsh = fallbackDshProjectId;
    }
  }
  return {
    tabs: nextTabs,
    activeAgentId: nextActive,
    activeDshProjectId: nextDsh,
  };
}

export function closeAgentTabInWorkspace(
  state: HostWorkspaceTabsState,
  agentId: string,
  agentProjectId: string | null,
  projectWorkspacesEnabled: boolean,
  openDshProjects: readonly string[],
): HostWorkspaceTabsState {
  if (projectWorkspacesEnabled) {
    const pid = agentProjectId ?? state.activeProjectId;
    if (!pid || !state.byProject[pid]) return state;
    const fallbackDsh = openDshProjects.includes(pid) ? pid : null;
    const nextGroup = removeAgentFromGroup(state.byProject[pid], agentId, fallbackDsh);
    return {
      ...state,
      byProject: {
        ...state.byProject,
        [pid]: nextGroup,
      },
    };
  }

  const fallbackDsh = openDshProjects[0] ?? null;
  return {
    ...state,
    shared: removeAgentFromGroup(state.shared, agentId, fallbackDsh),
  };
}

export function openDshInWorkspace(
  state: HostWorkspaceTabsState,
  projectId: string,
  projectWorkspacesEnabled: boolean,
): HostWorkspaceTabsState {
  if (projectWorkspacesEnabled) {
    const prevGroup = state.byProject[projectId] ?? EMPTY_TAB_GROUP;
    return {
      ...state,
      initialized: true,
      activeProjectId: projectId,
      byProject: {
        ...state.byProject,
        [projectId]: {
          ...prevGroup,
          activeDshProjectId: projectId,
        },
      },
    };
  }

  return {
    ...state,
    initialized: true,
    activeProjectId: projectId,
    shared: {
      ...state.shared,
      activeDshProjectId: projectId,
    },
  };
}

export function closeDshInWorkspace(
  state: HostWorkspaceTabsState,
  projectId: string,
): HostWorkspaceTabsState {
  const nextShared =
    state.shared.activeDshProjectId === projectId
      ? { ...state.shared, activeDshProjectId: null }
      : state.shared;

  const prevProjGroup = state.byProject[projectId];
  const nextByProject =
    prevProjGroup && prevProjGroup.activeDshProjectId === projectId
      ? {
          ...state.byProject,
          [projectId]: { ...prevProjGroup, activeDshProjectId: null },
        }
      : state.byProject;

  return {
    ...state,
    shared: nextShared,
    byProject: nextByProject,
  };
}

export function toggleProjectWorkspacesMode(
  state: HostWorkspaceTabsState,
  nextEnabled: boolean,
  agents: readonly AgentInstance[],
): HostWorkspaceTabsState {
  const agentProjectMap = new Map<string, string>();
  for (const a of agents) {
    agentProjectMap.set(a.id, a.project);
  }

  if (nextEnabled) {
    // Project shared tabs into per-project groups (mirrors Rust WorkspaceController::set_enabled)
    const nextByProject: Record<string, TabGroupState> = { ...state.byProject };
    const groupedFromShared = new Map<string, string[]>();
    for (const id of state.shared.tabs) {
      const pid = agentProjectMap.get(id);
      if (!pid) continue;
      const list = groupedFromShared.get(pid) ?? [];
      list.push(id);
      groupedFromShared.set(pid, list);
    }

    const derivedActiveProject =
      state.shared.activeDshProjectId ??
      (state.shared.activeAgentId ? agentProjectMap.get(state.shared.activeAgentId) ?? null : null) ??
      state.activeProjectId;

    for (const [pid, ids] of groupedFromShared.entries()) {
      const existing = nextByProject[pid];
      if (!existing || existing.tabs.length === 0) {
        const activeInProj =
          state.shared.activeAgentId && ids.includes(state.shared.activeAgentId)
            ? state.shared.activeAgentId
            : ids[0] ?? null;
        nextByProject[pid] = {
          tabs: ids,
          activeAgentId: activeInProj,
          activeDshProjectId: state.shared.activeDshProjectId === pid ? pid : null,
        };
      }
    }

    if (derivedActiveProject && state.shared.activeDshProjectId === derivedActiveProject) {
      const cur = nextByProject[derivedActiveProject] ?? { ...EMPTY_TAB_GROUP, tabs: [] };
      nextByProject[derivedActiveProject] = {
        ...cur,
        activeDshProjectId: derivedActiveProject,
      };
    }

    return {
      ...state,
      activeProjectId: derivedActiveProject,
      byProject: nextByProject,
    };
  }

  // Switching to shared mode: keep currently focused tab visible in shared layout
  const currentGroup = state.activeProjectId
    ? state.byProject[state.activeProjectId] ?? EMPTY_TAB_GROUP
    : EMPTY_TAB_GROUP;
  let nextSharedTabs = state.shared.tabs.slice();
  if (currentGroup.activeAgentId && !nextSharedTabs.includes(currentGroup.activeAgentId)) {
    nextSharedTabs.push(currentGroup.activeAgentId);
  }
  return {
    ...state,
    shared: {
      tabs: nextSharedTabs,
      activeAgentId: currentGroup.activeAgentId ?? state.shared.activeAgentId,
      activeDshProjectId: currentGroup.activeDshProjectId,
    },
  };
}

export function reconcileWorkspaceTabsWithSnapshot(
  state: HostWorkspaceTabsState,
  projects: readonly Project[],
  agents: readonly AgentInstance[],
  projectWorkspacesEnabled: boolean,
): HostWorkspaceTabsState {
  const validProjectIds = new Set(projects.map((p) => p.id));
  const agentProjectMap = new Map<string, string>();
  for (const a of agents) {
    agentProjectMap.set(a.id, a.project);
  }

  // Filter shared tabs to existing agents
  const sharedTabs = state.shared.tabs.filter((id) => agentProjectMap.has(id));
  const sharedActive =
    state.shared.activeAgentId && sharedTabs.includes(state.shared.activeAgentId)
      ? state.shared.activeAgentId
      : sharedTabs[0] ?? null;
  const sharedDsh =
    state.shared.activeDshProjectId && validProjectIds.has(state.shared.activeDshProjectId)
      ? state.shared.activeDshProjectId
      : null;

  // Filter per-project tabs to existing projects & agents belonging to that project
  const nextByProject: Record<string, TabGroupState> = {};
  for (const [pid, group] of Object.entries(state.byProject)) {
    if (!validProjectIds.has(pid)) continue;
    const filteredTabs = group.tabs.filter((id) => agentProjectMap.get(id) === pid);
    const active =
      group.activeAgentId && filteredTabs.includes(group.activeAgentId)
        ? group.activeAgentId
        : filteredTabs[0] ?? null;
    const activeDsh = group.activeDshProjectId === pid ? pid : null;
    nextByProject[pid] = {
      tabs: filteredTabs,
      activeAgentId: active,
      activeDshProjectId: activeDsh,
    };
  }

  // Determine valid activeProjectId
  let nextActiveProjectId =
    state.activeProjectId && validProjectIds.has(state.activeProjectId)
      ? state.activeProjectId
      : null;
  if (!nextActiveProjectId) {
    const firstAgent = agents.find((a) => validProjectIds.has(a.project));
    nextActiveProjectId = firstAgent?.project ?? projects[0]?.id ?? null;
  }

  let nextShared: TabGroupState = {
    tabs: sharedTabs,
    activeAgentId: sharedActive,
    activeDshProjectId: sharedDsh,
  };
  let initialized = state.initialized;

  // First-time hydration: open just the first session of the active project (never all projects!)
  if (!initialized && nextActiveProjectId) {
    const firstProjectAgent = agents.find((a) => a.project === nextActiveProjectId);
    if (firstProjectAgent) {
      const existingGroup = nextByProject[nextActiveProjectId];
      if (!existingGroup || existingGroup.tabs.length === 0) {
        nextByProject[nextActiveProjectId] = {
          tabs: [firstProjectAgent.id],
          activeAgentId: firstProjectAgent.id,
          activeDshProjectId: null,
        };
      }
      if (nextShared.tabs.length === 0) {
        nextShared = {
          tabs: [firstProjectAgent.id],
          activeAgentId: firstProjectAgent.id,
          activeDshProjectId: null,
        };
      }
    }
    initialized = true;
  } else if (!projectWorkspacesEnabled && nextShared.tabs.length === 0 && agents.length > 0 && !initialized) {
    nextShared = {
      tabs: [agents[0].id],
      activeAgentId: agents[0].id,
      activeDshProjectId: null,
    };
    initialized = true;
  }

  return {
    initialized,
    activeProjectId: nextActiveProjectId,
    shared: nextShared,
    byProject: nextByProject,
  };
}
