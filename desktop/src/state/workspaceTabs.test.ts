import { describe, expect, it } from "vitest";
import type { AgentInstance, Project } from "../shared/protocol";
import type { WebSession } from "./webSessions";
import {
  closeTabInWorkspace,
  createEmptyWorkspaceTabsState,
  getVisibleWorkspaceView,
  openTabInWorkspace,
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

const webSessions: WebSession[] = [
  { id: "web_doc_1", projectId: "p1", title: "localhost:5173", url: "http://localhost:5173", createdAt: 1 },
  {
    id: "web_doc_2",
    projectId: "p2",
    title: "localhost:3000",
    url: "http://localhost:3000",
    createdAt: 2,
  },
];

describe("project-scoped workspace tabs (agents + web/dsh sessions)", () => {
  it("hydrates only the active project's first session on initial load", () => {
    const initial = reconcileWorkspaceTabsWithSnapshot(
      createEmptyWorkspaceTabsState(),
      projects,
      agents,
      true,
      webSessions,
    );
    const view = getVisibleWorkspaceView(initial, true, agents, webSessions);
    expect(view.activeProjectId).toBe("p1");
    expect(view.openTabs).toEqual(["a1"]);
    expect(view.activeTabId).toBe("a1");
  });

  it("isolates open agent and web tabs per project and restores them when switching back", () => {
    let state = reconcileWorkspaceTabsWithSnapshot(
      createEmptyWorkspaceTabsState(),
      projects,
      agents,
      true,
      webSessions,
    );
    state = openTabInWorkspace(state, "web_doc_1", "p1", true);
    state = openTabInWorkspace(state, "web_doc_2", "p2", true);

    let view = getVisibleWorkspaceView(state, true, agents, webSessions);
    expect(view.activeProjectId).toBe("p2");
    expect(view.openTabs).toEqual(["web_doc_2"]);
    expect(view.activeTabId).toBe("web_doc_2");

    state = selectProjectWorkspace(state, "p1");
    view = getVisibleWorkspaceView(state, true, agents, webSessions);
    expect(view.activeProjectId).toBe("p1");
    expect(view.openTabs).toEqual(["a1", "web_doc_1"]);
    expect(view.activeTabId).toBe("web_doc_1");
  });

  it("closes a web tab and falls back to adjacent tab in the same project", () => {
    let state = reconcileWorkspaceTabsWithSnapshot(
      createEmptyWorkspaceTabsState(),
      projects,
      agents,
      true,
      webSessions,
    );
    state = openTabInWorkspace(state, "web_doc_1", "p1", true);
    state = closeTabInWorkspace(state, "web_doc_1", "p1", true);

    const view = getVisibleWorkspaceView(state, true, agents, webSessions);
    expect(view.openTabs).toEqual(["a1"]);
    expect(view.activeTabId).toBe("a1");
  });

  it("projects shared tabs into project groups when enabling project workspaces", () => {
    let state = createEmptyWorkspaceTabsState();
    state = openTabInWorkspace(state, "a1", "p1", false);
    state = openTabInWorkspace(state, "b1", "p2", false);
    state = openTabInWorkspace(state, "web_doc_2", "p2", false);

    state = toggleProjectWorkspacesMode(state, true, agents, webSessions);
    let view = getVisibleWorkspaceView(state, true, agents, webSessions);
    expect(view.activeProjectId).toBe("p2");
    expect(view.openTabs).toEqual(["b1", "web_doc_2"]);
    expect(view.activeTabId).toBe("web_doc_2");

    state = selectProjectWorkspace(state, "p1");
    view = getVisibleWorkspaceView(state, true, agents, webSessions);
    expect(view.openTabs).toEqual(["a1"]);
    expect(view.activeTabId).toBe("a1");
  });
});
