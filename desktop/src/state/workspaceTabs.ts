import type { AgentInstance, Project } from "../shared/protocol";
import type { WebSession } from "./webSessions";

export interface TabGroupState {
  tabs: string[];
  activeTabId: string | null;
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
  activeTabId: string | null;
}

const STORAGE_PREFIX = "muxlane.workspace.tabs.v1:";

export const EMPTY_TAB_GROUP: TabGroupState = {
  tabs: [],
  activeTabId: null,
};

export function createEmptyWorkspaceTabsState(): HostWorkspaceTabsState {
  return {
    initialized: false,
    activeProjectId: null,
    shared: { tabs: [], activeTabId: null },
    byProject: {},
  };
}

function sanitizeGroup(
  raw: (Partial<TabGroupState> & { activeAgentId?: string | null }) | undefined,
): TabGroupState {
  const tabs = Array.isArray(raw?.tabs)
    ? Array.from(new Set(raw.tabs.filter((x): x is string => typeof x === "string" && x.length > 0)))
    : [];
  const rawActive =
    typeof raw?.activeTabId === "string"
      ? raw.activeTabId
      : typeof raw?.activeAgentId === "string"
        ? raw.activeAgentId
        : null;
  const activeTabId = rawActive && tabs.includes(rawActive) ? rawActive : tabs[0] ?? null;
  return { tabs, activeTabId };
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

export function buildTabOwnerMap(
  agents: readonly AgentInstance[],
  webSessions: readonly WebSession[] = [],
): Map<string, string> {
  const map = new Map<string, string>();
  for (const a of agents) {
    map.set(a.id, a.project);
  }
  for (const w of webSessions) {
    map.set(w.id, w.projectId);
  }
  return map;
}

export function getVisibleWorkspaceView(
  state: HostWorkspaceTabsState,
  projectWorkspacesEnabled: boolean,
  agents: readonly AgentInstance[],
  webSessions: readonly WebSession[] = [],
): VisibleWorkspaceView {
  const ownerMap = buildTabOwnerMap(agents, webSessions);

  if (projectWorkspacesEnabled) {
    const pid = state.activeProjectId;
    const group = pid ? state.byProject[pid] ?? EMPTY_TAB_GROUP : EMPTY_TAB_GROUP;
    const openTabs = pid ? group.tabs.filter((id) => ownerMap.get(id) === pid) : [];
    const activeTabId =
      group.activeTabId && openTabs.includes(group.activeTabId)
        ? group.activeTabId
        : openTabs[0] ?? null;
    return {
      activeProjectId: pid,
      openTabs,
      activeTabId,
    };
  }

  const openTabs = state.shared.tabs.filter((id) => ownerMap.has(id));
  const activeTabId =
    state.shared.activeTabId && openTabs.includes(state.shared.activeTabId)
      ? state.shared.activeTabId
      : openTabs[0] ?? null;
  const derivedProjectId =
    (activeTabId ? ownerMap.get(activeTabId) ?? state.activeProjectId : state.activeProjectId) ??
    null;

  return {
    activeProjectId: derivedProjectId,
    openTabs,
    activeTabId,
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
          [projectId]: { tabs: [], activeTabId: null },
        },
  };
}

export function openTabInWorkspace(
  state: HostWorkspaceTabsState,
  tabId: string,
  ownerProjectId: string | null,
  projectWorkspacesEnabled: boolean,
): HostWorkspaceTabsState {
  const targetProjectId = ownerProjectId ?? state.activeProjectId;

  if (projectWorkspacesEnabled && targetProjectId) {
    const prevGroup = state.byProject[targetProjectId] ?? EMPTY_TAB_GROUP;
    const nextTabs = prevGroup.tabs.includes(tabId) ? prevGroup.tabs : [...prevGroup.tabs, tabId];
    return {
      ...state,
      initialized: true,
      activeProjectId: targetProjectId,
      byProject: {
        ...state.byProject,
        [targetProjectId]: {
          tabs: nextTabs,
          activeTabId: tabId,
        },
      },
    };
  }

  const nextSharedTabs = state.shared.tabs.includes(tabId)
    ? state.shared.tabs
    : [...state.shared.tabs, tabId];
  return {
    ...state,
    initialized: true,
    activeProjectId: targetProjectId,
    shared: {
      tabs: nextSharedTabs,
      activeTabId: tabId,
    },
  };
}

function removeTabFromGroup(group: TabGroupState, tabId: string): TabGroupState {
  const idx = group.tabs.indexOf(tabId);
  if (idx < 0) return group;
  const nextTabs = group.tabs.filter((id) => id !== tabId);
  const nextActive =
    group.activeTabId === tabId
      ? nextTabs[idx] ?? nextTabs[nextTabs.length - 1] ?? null
      : group.activeTabId;
  return {
    tabs: nextTabs,
    activeTabId: nextActive,
  };
}

export function closeTabInWorkspace(
  state: HostWorkspaceTabsState,
  tabId: string,
  ownerProjectId: string | null,
  projectWorkspacesEnabled: boolean,
): HostWorkspaceTabsState {
  if (projectWorkspacesEnabled) {
    const pid = ownerProjectId ?? state.activeProjectId;
    if (!pid || !state.byProject[pid]) return state;
    return {
      ...state,
      byProject: {
        ...state.byProject,
        [pid]: removeTabFromGroup(state.byProject[pid], tabId),
      },
    };
  }

  return {
    ...state,
    shared: removeTabFromGroup(state.shared, tabId),
  };
}

export function removeTabEverywhere(
  state: HostWorkspaceTabsState,
  tabId: string,
): HostWorkspaceTabsState {
  const nextByProject: Record<string, TabGroupState> = {};
  for (const [pid, group] of Object.entries(state.byProject)) {
    nextByProject[pid] = removeTabFromGroup(group, tabId);
  }
  return {
    ...state,
    shared: removeTabFromGroup(state.shared, tabId),
    byProject: nextByProject,
  };
}

export function toggleProjectWorkspacesMode(
  state: HostWorkspaceTabsState,
  nextEnabled: boolean,
  agents: readonly AgentInstance[],
  webSessions: readonly WebSession[] = [],
): HostWorkspaceTabsState {
  const ownerMap = buildTabOwnerMap(agents, webSessions);

  if (nextEnabled) {
    const nextByProject: Record<string, TabGroupState> = { ...state.byProject };
    const groupedFromShared = new Map<string, string[]>();
    for (const id of state.shared.tabs) {
      const pid = ownerMap.get(id);
      if (!pid) continue;
      const list = groupedFromShared.get(pid) ?? [];
      list.push(id);
      groupedFromShared.set(pid, list);
    }

    const derivedActiveProject =
      (state.shared.activeTabId ? ownerMap.get(state.shared.activeTabId) ?? null : null) ??
      state.activeProjectId;

    for (const [pid, ids] of groupedFromShared.entries()) {
      const existing = nextByProject[pid];
      if (!existing || existing.tabs.length === 0) {
        const activeInProj =
          state.shared.activeTabId && ids.includes(state.shared.activeTabId)
            ? state.shared.activeTabId
            : ids[0] ?? null;
        nextByProject[pid] = {
          tabs: ids,
          activeTabId: activeInProj,
        };
      }
    }

    return {
      ...state,
      activeProjectId: derivedActiveProject,
      byProject: nextByProject,
    };
  }

  const currentGroup = state.activeProjectId
    ? state.byProject[state.activeProjectId] ?? EMPTY_TAB_GROUP
    : EMPTY_TAB_GROUP;
  const nextSharedTabs = state.shared.tabs.slice();
  if (currentGroup.activeTabId && !nextSharedTabs.includes(currentGroup.activeTabId)) {
    nextSharedTabs.push(currentGroup.activeTabId);
  }
  return {
    ...state,
    shared: {
      tabs: nextSharedTabs,
      activeTabId: currentGroup.activeTabId ?? state.shared.activeTabId,
    },
  };
}

export function reconcileWorkspaceTabsWithSnapshot(
  state: HostWorkspaceTabsState,
  projects: readonly Project[],
  agents: readonly AgentInstance[],
  projectWorkspacesEnabled: boolean,
  webSessions: readonly WebSession[] = [],
): HostWorkspaceTabsState {
  const validProjectIds = new Set(projects.map((p) => p.id));
  const ownerMap = buildTabOwnerMap(agents, webSessions);

  const sharedTabs = state.shared.tabs.filter((id) => ownerMap.has(id));
  const sharedActive =
    state.shared.activeTabId && sharedTabs.includes(state.shared.activeTabId)
      ? state.shared.activeTabId
      : sharedTabs[0] ?? null;

  const nextByProject: Record<string, TabGroupState> = {};
  for (const [pid, group] of Object.entries(state.byProject)) {
    if (!validProjectIds.has(pid)) continue;
    const filteredTabs = group.tabs.filter((id) => ownerMap.get(id) === pid);
    const active =
      group.activeTabId && filteredTabs.includes(group.activeTabId)
        ? group.activeTabId
        : filteredTabs[0] ?? null;
    nextByProject[pid] = {
      tabs: filteredTabs,
      activeTabId: active,
    };
  }

  let nextActiveProjectId =
    state.activeProjectId && validProjectIds.has(state.activeProjectId)
      ? state.activeProjectId
      : null;
  if (!nextActiveProjectId) {
    const firstAgent = agents.find((a) => validProjectIds.has(a.project));
    const firstWeb = webSessions.find((w) => validProjectIds.has(w.projectId));
    nextActiveProjectId = firstAgent?.project ?? firstWeb?.projectId ?? projects[0]?.id ?? null;
  }

  let nextShared: TabGroupState = {
    tabs: sharedTabs,
    activeTabId: sharedActive,
  };
  let initialized = state.initialized;

  if (!initialized && nextActiveProjectId) {
    const firstTabId =
      agents.find((a) => a.project === nextActiveProjectId)?.id ??
      webSessions.find((w) => w.projectId === nextActiveProjectId)?.id ??
      null;
    if (firstTabId) {
      const existingGroup = nextByProject[nextActiveProjectId];
      if (!existingGroup || existingGroup.tabs.length === 0) {
        nextByProject[nextActiveProjectId] = {
          tabs: [firstTabId],
          activeTabId: firstTabId,
        };
      }
      if (nextShared.tabs.length === 0) {
        nextShared = {
          tabs: [firstTabId],
          activeTabId: firstTabId,
        };
      }
    }
    initialized = true;
  } else if (
    !projectWorkspacesEnabled &&
    nextShared.tabs.length === 0 &&
    (agents.length > 0 || webSessions.length > 0) &&
    !initialized
  ) {
    const firstId = agents[0]?.id ?? webSessions[0]?.id ?? null;
    if (firstId) {
      nextShared = {
        tabs: [firstId],
        activeTabId: firstId,
      };
    }
    initialized = true;
  }

  return {
    initialized,
    activeProjectId: nextActiveProjectId,
    shared: nextShared,
    byProject: nextByProject,
  };
}
