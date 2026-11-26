import { describe, expect, it } from "vitest";
import type { AgentInstance, Project } from "../shared/protocol";
import {
  closeAgentTabInWorkspace,
  closeDshInWorkspace,
  createEmptyWorkspaceTabsState,
  getVisibleWorkspaceView,
  openAgentInWorkspace,
  openDshInWorkspace,
  reconcileWorkspaceTabsWithSnapshot,
  selectProjectWorkspace,
  toggleProjectWorkspacesMode,
} from "./workspaceTabs";

const projects: Project[] = [
  { id: "p1", name: "alpha", path: "/tmp/alpha", branch: "main", agents: ["a1", "a2"] },
  { id: "p2", name: "beta", path: "/tmp/beta", branch: "dev", agents: ["b1", "b2"] },
];

function makeAgent(id: string, project: string): AgentInstance {
  return {
    id,
    project,
    agent_type: "shell",
    title: id,
    title_is_custom: false,
    status: "idle",
    status_since: 1,
    seen: true,
    tmux_session: `muxlane-${id}`,
  };
}

const agents: AgentInstance[] = [
  makeAgent("a1", "p1"),
  makeAgent("a2", "p1"),
  makeAgent("b1", "p2"),
  makeAgent("b2", "p2"),
];

describe("project-scoped workspace tabs", () => {
  it("hydrates only the active project's first session on initial load", () => {
    const initial = reconcileWorkspaceTabsWithSnapshot(
      createEmptyWorkspaceTabsState(),
      projects,
      agents,
      true,
    );
    const view = getVisibleWorkspaceView(initial, true, [], agents);
    expect(view.activeProjectId).toBe("p1");
    expect(view.openTabs).toEqual(["a1"]);
    expect(view.activeAgentId).toBe("a1");
  });

  it("isolates open tabs per project and restores them when switching back", () => {
    let state = reconcileWorkspaceTabsWithSnapshot(
      createEmptyWorkspaceTabsState(),
      projects,
      agents,
      true,
    );
    state = openAgentInWorkspace(state, "a2", "p1", true);
    state = openAgentInWorkspace(state, "b1", "p2", true);

    // Now on p2: only b1 is visible, p1's tabs are hidden
    let view = getVisibleWorkspaceView(state, true, [], agents);
    expect(view.activeProjectId).toBe("p2");
    expect(view.openTabs).toEqual(["b1"]);
    expect(view.activeAgentId).toBe("b1");

    // Switch back to p1 by clicking the project row: restores [a1, a2] with a2 active
    state = selectProjectWorkspace(state, "p1");
    view = getVisibleWorkspaceView(state, true, [], agents);
    expect(view.activeProjectId).toBe("p1");
    expect(view.openTabs).toEqual(["a1", "a2"]);
    expect(view.activeAgentId).toBe("a2");
  });

  it("scopes DSH WebUI tabs to their project and falls back on close", () => {
    let state = reconcileWorkspaceTabsWithSnapshot(
      createEmptyWorkspaceTabsState(),
      projects,
      agents,
      true,
    );
    state = openDshInWorkspace(state, "p1", true);

    let view = getVisibleWorkspaceView(state, true, ["p1", "p2"], agents);
    expect(view.visibleDshProjects).toEqual(["p1"]);
    expect(view.activeDshProjectId).toBe("p1");
    expect(view.activeAgentId).toBeNull();

    // Closing DSH on p1 returns focus to a1
    state = closeDshInWorkspace(state, "p1");
    view = getVisibleWorkspaceView(state, true, ["p2"], agents);
    expect(view.visibleDshProjects).toEqual([]);
    expect(view.activeDshProjectId).toBeNull();
    expect(view.activeAgentId).toBe("a1");
  });

  it("projects shared tabs into project groups when enabling project workspaces", () => {
    let state = createEmptyWorkspaceTabsState();
    state = openAgentInWorkspace(state, "a1", "p1", false);
    state = openAgentInWorkspace(state, "b1", "p2", false);
    state = openAgentInWorkspace(state, "b2", "p2", false);

    // Switch to project-scoped mode: active project is p2 with [b1, b2], p1 keeps [a1]
    state = toggleProjectWorkspacesMode(state, true, agents);
    let view = getVisibleWorkspaceView(state, true, [], agents);
    expect(view.activeProjectId).toBe("p2");
    expect(view.openTabs).toEqual(["b1", "b2"]);
    expect(view.activeAgentId).toBe("b2");

    state = selectProjectWorkspace(state, "p1");
    view = getVisibleWorkspaceView(state, true, [], agents);
    expect(view.openTabs).toEqual(["a1"]);
    expect(view.activeAgentId).toBe("a1");
  });

  it("closes tabs within the current project without disturbing other projects", () => {
    let state = reconcileWorkspaceTabsWithSnapshot(
      createEmptyWorkspaceTabsState(),
      projects,
      agents,
      true,
    );
    state = openAgentInWorkspace(state, "a2", "p1", true);
    state = closeAgentTabInWorkspace(state, "a2", "p1", true, []);
    const view = getVisibleWorkspaceView(state, true, [], agents);
    expect(view.openTabs).toEqual(["a1"]);
    expect(view.activeAgentId).toBe("a1");
  });
});
