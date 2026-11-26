import React, { useMemo, useState } from "react";
import {
  BellRing,
  ChevronDown,
  ChevronRight,
  FolderPlus,
  Globe,
  Pencil,
  Plus,
  Search,
  Server,
  Trash2,
  X,
} from "lucide-react";
import type {
  AgentInstance,
  AgentStatus,
  ChatSessionSummary,
  PendingApproval,
  Project,
} from "../shared/protocol";
import { agentTypeLabel } from "../shared/timeline";
import { applyOrder, type DropPosition } from "../state/useSidebarOrder";
import { isWebSessionId, type WebSession } from "../state/webSessions";
import { ProjectFolderIcon } from "./MuxlaneIcons";

interface SidebarProps {
  width: number;
  onResize: (nextWidth: number) => void;
  machineName: string;
  isRemoteHost: boolean;
  remoteHostsCount: number;
  projects: Project[];
  agents: AgentInstance[];
  webSessions: WebSession[];
  projectOrder: string[];
  sessionOrder: Record<string, string[]>;
  onReorderProjects: (
    visibleProjectIds: string[],
    draggedId: string,
    targetId: string,
    position: DropPosition,
  ) => void;
  onReorderSessions: (
    projectId: string,
    visibleSessionIds: string[],
    draggedId: string,
    targetId: string,
    position: DropPosition,
  ) => void;
  chatSummaries: Record<string, ChatSessionSummary>;
  activeProjectId: string | null;
  projectWorkspacesEnabled: boolean;
  activeTabId: string | null;
  activeSection: "workspace" | "dsh" | "bots";
  globalApprovals: PendingApproval[];
  onOpenRemotes: () => void;
  onSelectProject: (projectId: string) => void;
  onSelectTab: (tabId: string) => void;
  onAddProject: () => void;
  onDeleteProject: (projectId: string) => void;
  onNewSessionForProject: (projectId: string) => void;
  onNewSession: () => void;
  onRenameAgent: (agentId: string, nextTitle: string) => void;
  onDeleteAgent: (agentId: string) => void;
  onRenameWebSession: (id: string, nextTitle: string) => void;
  onDeleteWebSession: (id: string) => void;
}

type SidebarSessionItem =
  | { kind: "agent"; id: string; agent: AgentInstance }
  | { kind: "web"; id: string; web: WebSession };

type DragPayload =
  | { kind: "project"; projectId: string }
  | { kind: "session"; projectId: string; itemId: string };

type DropIndicator =
  | { kind: "project"; targetId: string; position: DropPosition }
  | { kind: "session"; projectId: string; targetId: string; position: DropPosition };

function statusSquareClass(status: AgentStatus, streaming: boolean): string {
  if (status === "blocked") return "bg-question";
  if (status === "failed") return "bg-danger";
  if (status === "working" || streaming) return "bg-working animate-pulse";
  if (status === "done") return "bg-success";
  return "bg-ink-faint";
}

function shortAgentTag(type: AgentInstance["agent_type"]): string {
  switch (type) {
    case "pi":
      return "Pi";
    case "pie":
      return "Pie";
    case "opencode":
      return "OC";
    case "claude":
      return "CC";
    case "codex":
      return "CX";
    case "gemini":
      return "GM";
    case "kimi":
      return "KM";
    case "agy":
      return "Agy";
    case "shell":
      return "Sh";
    default:
      return agentTypeLabel(type);
  }
}

export const Sidebar: React.FC<SidebarProps> = ({
  width,
  onResize,
  machineName,
  isRemoteHost,
  remoteHostsCount,
  projects,
  agents,
  webSessions,
  projectOrder,
  sessionOrder,
  onReorderProjects,
  onReorderSessions,
  chatSummaries,
  activeProjectId,
  projectWorkspacesEnabled,
  activeTabId,
  activeSection,
  globalApprovals,
  onOpenRemotes,
  onSelectProject,
  onSelectTab,
  onAddProject,
  onDeleteProject,
  onNewSessionForProject,
  onNewSession,
  onRenameAgent,
  onDeleteAgent,
  onRenameWebSession,
  onDeleteWebSession,
}) => {
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [filterQuery, setFilterQuery] = useState("");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingTitle, setEditingTitle] = useState("");
  const [draggingResize, setDraggingResize] = useState(false);
  const [dragPayload, setDragPayload] = useState<DragPayload | null>(null);
  const [dropIndicator, setDropIndicator] = useState<DropIndicator | null>(null);

  const normalizedQuery = filterQuery.trim().toLowerCase();
  const canDragReorder = !normalizedQuery && !editingId;

  const orderedProjects = useMemo(
    () => applyOrder(projects, projectOrder, (p) => p.id),
    [projectOrder, projects],
  );

  const filteredProjects = useMemo(() => {
    const groups = orderedProjects.map((project) => {
      const rawItems: SidebarSessionItem[] = [
        ...agents
          .filter((a) => a.project === project.id)
          .map((agent): SidebarSessionItem => ({ kind: "agent", id: agent.id, agent })),
        ...webSessions
          .filter((w) => w.projectId === project.id)
          .map((web): SidebarSessionItem => ({ kind: "web", id: web.id, web })),
      ];
      const items = applyOrder(rawItems, sessionOrder[project.id], (x) => x.id);
      return { project, items };
    });

    if (!normalizedQuery) {
      return groups;
    }
    return groups
      .map(({ project, items }) => {
        const projectMatch =
          project.name.toLowerCase().includes(normalizedQuery) ||
          (project.branch?.toLowerCase().includes(normalizedQuery) ?? false);
        const matchingItems = projectMatch
          ? items
          : items.filter((item) => {
              if (item.kind === "web") {
                return (
                  item.web.title.toLowerCase().includes(normalizedQuery) ||
                  item.web.url.toLowerCase().includes(normalizedQuery)
                );
              }
              return (
                item.agent.title.toLowerCase().includes(normalizedQuery) ||
                agentTypeLabel(item.agent.agent_type).toLowerCase().includes(normalizedQuery)
              );
            });
        return { project, items: matchingItems, projectMatch };
      })
      .filter((g) => g.projectMatch || g.items.length > 0);
  }, [agents, normalizedQuery, orderedProjects, sessionOrder, webSessions]);

  const startRename = (id: string, title: string) => {
    setEditingId(id);
    setEditingTitle(title);
  };

  const commitRename = () => {
    if (editingId && editingTitle.trim()) {
      if (isWebSessionId(editingId)) {
        onRenameWebSession(editingId, editingTitle.trim());
      } else {
        onRenameAgent(editingId, editingTitle.trim());
      }
    }
    setEditingId(null);
  };

  const handleResizeDragStart = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = width;
    setDraggingResize(true);

    const onMove = (ev: PointerEvent) => {
      const next = Math.min(380, Math.max(216, startWidth + (ev.clientX - startX)));
      onResize(next);
    };
    const onUp = () => {
      setDraggingResize(false);
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  const clearDragState = () => {
    setDragPayload(null);
    setDropIndicator(null);
  };

  const computeDropPos = (e: React.DragEvent<HTMLElement>): DropPosition => {
    const rect = e.currentTarget.getBoundingClientRect();
    return e.clientY < rect.top + rect.height / 2 ? "before" : "after";
  };

  const visibleProjectIds = useMemo(
    () => orderedProjects.map((p) => p.id),
    [orderedProjects],
  );

  const renderSessionRows = (workspaceId: string, items: SidebarSessionItem[]) => {
    const visibleIds = items.map((x) => x.id);
    return (
      <div className="mt-0.5 space-y-[1px]">
        {items.map((item) => {
          const isActive = activeSection === "workspace" && activeTabId === item.id;
          const isDraggingThisSession =
            dragPayload?.kind === "session" && dragPayload.itemId === item.id;
          const sessionDropBefore =
            dropIndicator?.kind === "session" &&
            dropIndicator.projectId === workspaceId &&
            dropIndicator.targetId === item.id &&
            dropIndicator.position === "before";
          const sessionDropAfter =
            dropIndicator?.kind === "session" &&
            dropIndicator.projectId === workspaceId &&
            dropIndicator.targetId === item.id &&
            dropIndicator.position === "after";

          const title = item.kind === "web" ? item.web.title : item.agent.title;
          const summary = item.kind === "agent" ? chatSummaries[item.agent.id] : undefined;
          const streaming = summary?.meta.is_streaming === true;
          const unread =
            item.kind === "agent" &&
            !item.agent.seen &&
            (item.agent.status === "done" ||
              item.agent.status === "failed" ||
              item.agent.status === "blocked");
          const badge = item.kind === "web" ? "Web" : shortAgentTag(item.agent.agent_type);

          return (
            <div
              key={item.id}
              draggable={canDragReorder && editingId !== item.id}
              onDragStart={(e) => {
                if (!canDragReorder || editingId === item.id) return;
                e.stopPropagation();
                e.dataTransfer.effectAllowed = "move";
                e.dataTransfer.setData("text/plain", `session:${item.id}`);
                setDragPayload({
                  kind: "session",
                  projectId: workspaceId,
                  itemId: item.id,
                });
              }}
              onDragOver={(e) => {
                if (
                  !canDragReorder ||
                  dragPayload?.kind !== "session" ||
                  dragPayload.projectId !== workspaceId ||
                  dragPayload.itemId === item.id
                ) {
                  return;
                }
                e.preventDefault();
                e.stopPropagation();
                e.dataTransfer.dropEffect = "move";
                const pos = computeDropPos(e);
                setDropIndicator((prev) =>
                  prev?.kind === "session" &&
                  prev.targetId === item.id &&
                  prev.position === pos
                    ? prev
                    : {
                        kind: "session",
                        projectId: workspaceId,
                        targetId: item.id,
                        position: pos,
                      },
                );
              }}
              onDrop={(e) => {
                if (
                  !canDragReorder ||
                  dragPayload?.kind !== "session" ||
                  dragPayload.projectId !== workspaceId
                ) {
                  return;
                }
                e.preventDefault();
                e.stopPropagation();
                const pos = computeDropPos(e);
                if (dragPayload.itemId !== item.id) {
                  onReorderSessions(workspaceId, visibleIds, dragPayload.itemId, item.id, pos);
                }
                clearDragState();
              }}
              onClick={() => onSelectTab(item.id)}
              className={`group relative flex h-7 items-center justify-between gap-2 px-2 transition-colors cursor-pointer ${
                isDraggingThisSession ? "opacity-45" : ""
              } ${
                isActive
                  ? "bg-active text-ink-strong"
                  : "text-ink-2 hover:bg-hover hover:text-ink"
              }`}
            >
              {sessionDropBefore && (
                <span className="pointer-events-none absolute inset-x-1 top-0 z-10 h-[2px] bg-ink-strong" />
              )}
              {sessionDropAfter && (
                <span className="pointer-events-none absolute inset-x-1 bottom-0 z-10 h-[2px] bg-ink-strong" />
              )}
              {isActive && (
                <span className="absolute left-0 top-1 bottom-1 w-[2px] bg-ink-strong" />
              )}

              <div className="flex min-w-0 flex-1 items-center gap-1.5">
                <span className="inline-flex h-4 w-4 shrink-0 items-center justify-center">
                  {item.kind === "web" ? (
                    <Globe className="h-3 w-3 text-ink-muted" strokeWidth={1.85} />
                  ) : (
                    <span
                      className={`h-1.5 w-1.5 ${statusSquareClass(item.agent.status, streaming)}`}
                    />
                  )}
                </span>
                {editingId === item.id ? (
                  <input
                    autoFocus
                    value={editingTitle}
                    onClick={(e) => e.stopPropagation()}
                    onChange={(e) => setEditingTitle(e.target.value)}
                    onBlur={commitRename}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitRename();
                      if (e.key === "Escape") setEditingId(null);
                    }}
                    className="w-full border border-line-strong bg-canvas px-1.5 py-0.5 text-[12px] text-ink-strong focus:outline-none select-text"
                  />
                ) : (
                  <span
                    className={`truncate text-[12.5px] ${
                      isActive ? "font-medium text-ink-strong" : ""
                    }`}
                  >
                    {title}
                  </span>
                )}
              </div>

              {editingId !== item.id && (
                <>
                  <div className="flex shrink-0 items-center gap-1.5">
                    {unread && (
                      <span className="h-1.5 w-1.5 bg-unread" title="有未读更新" />
                    )}
                    <span className="font-mono text-[10px] text-ink-muted">{badge}</span>
                  </div>

                  {/* Unified hover actions for ALL session types */}
                  <div
                    className={`pointer-events-none absolute right-1 top-1/2 -translate-y-1/2 flex items-center gap-0.5 pl-2 opacity-0 group-hover:pointer-events-auto group-hover:opacity-100 transition-opacity ${
                      isActive ? "bg-active" : "bg-hover"
                    }`}
                  >
                    <button
                      type="button"
                      title="重命名"
                      onClick={(e) => {
                        e.stopPropagation();
                        startRename(item.id, title);
                      }}
                      className="flex h-5 w-5 items-center justify-center text-ink-muted hover:text-ink-strong cursor-pointer"
                    >
                      <Pencil className="h-3 w-3" />
                    </button>
                    <button
                      type="button"
                      title="删除会话"
                      onClick={(e) => {
                        e.stopPropagation();
                        if (item.kind === "web") {
                          onDeleteWebSession(item.id);
                        } else {
                          onDeleteAgent(item.id);
                        }
                      }}
                      className="flex h-5 w-5 items-center justify-center text-ink-muted hover:text-danger cursor-pointer"
                    >
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </div>
                </>
              )}
            </div>
          );
        })}
      </div>
    );
  };

  return (
    <aside
      style={{ width }}
      className="relative flex h-full shrink-0 flex-col border-r border-line-subtle bg-sidebar select-none"
    >
      {/* Top 36px Header aligned with WorkspaceHeader — clickable machine switcher */}
      <div className="flex h-9 shrink-0 items-center justify-between gap-1 border-b border-line-subtle px-2">
        <button
          type="button"
          onClick={onOpenRemotes}
          title={
            remoteHostsCount > 0
              ? `切换或管理远程机器（已保存 ${remoteHostsCount} 台远程主机）`
              : "连接或管理远程机器"
          }
          className="flex min-w-0 items-center gap-1.5 px-1.5 py-1 text-left hover:bg-hover transition-colors cursor-pointer"
        >
          <Server
            className={`h-3.5 w-3.5 shrink-0 ${isRemoteHost ? "text-success" : "text-ink-muted"}`}
            strokeWidth={1.75}
          />
          <span className="truncate text-[12.5px] font-semibold tracking-tight text-ink-strong">
            {machineName || "本机"}
          </span>
          <ChevronDown className="h-3 w-3 shrink-0 text-ink-muted" />
        </button>

        <div className="flex shrink-0 items-center gap-0.5">
          <button
            type="button"
            onClick={onAddProject}
            title="添加项目工作区"
            className="flex h-6 w-6 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
          >
            <FolderPlus className="h-3.5 w-3.5" strokeWidth={1.75} />
          </button>
          <button
            type="button"
            onClick={onNewSession}
            title="新建会话 (Ctrl+N)"
            className="flex h-6 w-6 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
          >
            <Plus className="h-3.5 w-3.5" strokeWidth={1.85} />
          </button>
        </div>
      </div>

      {/* Quiet Search / Filter Field */}
      <div className="shrink-0 border-b border-line-subtle px-2.5 py-2">
        <div className="flex h-7 items-center gap-1.5 border border-line-subtle bg-canvas/60 px-2 focus-within:border-line-strong transition-colors">
          <Search className="h-3.5 w-3.5 shrink-0 text-ink-muted" strokeWidth={1.75} />
          <input
            value={filterQuery}
            onChange={(e) => setFilterQuery(e.target.value)}
            placeholder="筛选会话或项目…"
            className="min-w-0 flex-1 bg-transparent text-[12px] text-ink placeholder:text-ink-muted focus:outline-none select-text"
          />
          {filterQuery && (
            <button
              type="button"
              onClick={() => setFilterQuery("")}
              className="flex h-4 w-4 shrink-0 items-center justify-center text-ink-muted hover:text-ink cursor-pointer"
            >
              <X className="h-3 w-3" />
            </button>
          )}
        </div>
      </div>

      {/* Priority Needs-Attention Section */}
      {globalApprovals.length > 0 && (
        <div className="shrink-0 border-b border-line-subtle bg-question-soft px-2.5 py-2">
          <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold text-question">
            <BellRing className="h-3 w-3" />
            <span>待确认审批 ({globalApprovals.length})</span>
          </div>
          <div className="space-y-1">
            {globalApprovals.slice(0, 3).map((ap) => {
              const target = agents.find((a) => a.id === ap.agent);
              return (
                <button
                  key={ap.id}
                  type="button"
                  onClick={() => onSelectTab(ap.agent)}
                  className="block w-full truncate border-l-2 border-question bg-canvas/75 px-2 py-1 text-left text-[11.5px] text-ink hover:bg-canvas transition-colors cursor-pointer"
                >
                  <span className="font-medium text-ink-strong">
                    {target?.title ?? ap.agent}
                  </span>
                  <span className="ml-1.5 text-ink-2">{ap.title}</span>
                </button>
              );
            })}
          </div>
        </div>
      )}

      {/* Projects & Sessions Tree */}
      <div
        tabIndex={-1}
        onDragEnd={clearDragState}
        className="mux-scroll-quiet flex-1 overflow-y-auto py-2 focus:outline-none"
      >
        {projects.length === 0 ? (
          <div className="mx-3 mt-6 border border-dashed border-line p-4 text-center">
            <p className="text-[12px] text-ink-2">还没有项目工作区</p>
            <button
              type="button"
              onClick={onAddProject}
              className="mt-3 bg-primary-btn px-3 py-1.5 text-[12px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover transition-colors cursor-pointer"
            >
              添加项目文件夹
            </button>
          </div>
        ) : filteredProjects.length === 0 ? (
          <div className="px-4 py-6 text-center text-[12px] text-ink-muted">
            没有匹配「{filterQuery}」的会话
          </div>
        ) : (
          <div className="space-y-2">
            {filteredProjects.map(({ project, items }) => {
              const defaultCollapsed = items.length === 0;
              const isCollapsed =
                !normalizedQuery && (collapsed[project.id] ?? defaultCollapsed);
              const isDraggingThisProject =
                dragPayload?.kind === "project" && dragPayload.projectId === project.id;
              const projectDropBefore =
                dropIndicator?.kind === "project" &&
                dropIndicator.targetId === project.id &&
                dropIndicator.position === "before";
              const projectDropAfter =
                dropIndicator?.kind === "project" &&
                dropIndicator.targetId === project.id &&
                dropIndicator.position === "after";
              const isCurrentProject =
                activeSection === "workspace" &&
                projectWorkspacesEnabled &&
                activeProjectId === project.id;
              const isProjectRowHighlighted = isCurrentProject && !activeTabId;

              return (
                <div
                  key={project.id}
                  onDragOver={(e) => {
                    if (!canDragReorder || dragPayload?.kind !== "project") return;
                    if (dragPayload.projectId === project.id) return;
                    e.preventDefault();
                    e.dataTransfer.dropEffect = "move";
                    const pos = computeDropPos(e);
                    setDropIndicator((prev) =>
                      prev?.kind === "project" &&
                      prev.targetId === project.id &&
                      prev.position === pos
                        ? prev
                        : { kind: "project", targetId: project.id, position: pos },
                    );
                  }}
                  onDrop={(e) => {
                    if (!canDragReorder || dragPayload?.kind !== "project") return;
                    e.preventDefault();
                    const pos = computeDropPos(e);
                    if (dragPayload.projectId !== project.id) {
                      onReorderProjects(visibleProjectIds, dragPayload.projectId, project.id, pos);
                    }
                    clearDragState();
                  }}
                  className={`relative px-1.5 ${isDraggingThisProject ? "opacity-45" : ""}`}
                >
                  {projectDropBefore && (
                    <div className="pointer-events-none absolute inset-x-2 -top-1 z-10 h-[2px] bg-ink-strong" />
                  )}
                  {projectDropAfter && (
                    <div className="pointer-events-none absolute inset-x-2 -bottom-1 z-10 h-[2px] bg-ink-strong" />
                  )}

                  {/* Project Group Row */}
                  <div
                    draggable={canDragReorder}
                    onDragStart={(e) => {
                      if (!canDragReorder) return;
                      e.dataTransfer.effectAllowed = "move";
                      e.dataTransfer.setData("text/plain", `project:${project.id}`);
                      setDragPayload({ kind: "project", projectId: project.id });
                    }}
                    title={project.path}
                    className={`group relative flex h-7 items-center px-2 transition-colors ${
                      isProjectRowHighlighted
                        ? "bg-active text-ink-strong"
                        : "hover:bg-hover"
                    }`}
                  >
                    {isCurrentProject && (
                      <span
                        className={`pointer-events-none absolute left-0 top-1 bottom-1 w-[2px] ${
                          isProjectRowHighlighted ? "bg-ink-strong" : "bg-ink-muted/60"
                        }`}
                      />
                    )}
                    <button
                      type="button"
                      onClick={() => {
                        const toggleCollapse = () =>
                          setCollapsed((prev) => ({
                            ...prev,
                            [project.id]: !(prev[project.id] ?? defaultCollapsed),
                          }));
                        if (projectWorkspacesEnabled) {
                          if (activeProjectId === project.id) {
                            if (items.length > 0) toggleCollapse();
                          } else {
                            onSelectProject(project.id);
                          }
                        } else if (items.length > 0) {
                          toggleCollapse();
                        }
                      }}
                      className="flex min-w-0 flex-1 items-center gap-1.5 text-left cursor-pointer"
                    >
                      <span
                        className={`inline-flex h-4 w-4 shrink-0 items-center justify-center ${
                          isCurrentProject ? "text-ink-strong" : "text-ink-muted"
                        }`}
                      >
                        <ProjectFolderIcon className="h-[14px] w-[14px]" />
                      </span>
                      <span
                        className={`truncate text-[12px] font-medium ${
                          isCurrentProject ? "text-ink-strong" : "text-ink"
                        }`}
                      >
                        {project.name}
                      </span>
                      {project.branch && (
                        <span className="truncate font-mono text-[10px] text-ink-muted">
                          {project.branch}
                        </span>
                      )}
                      {isCollapsed && items.length > 0 && (
                        <span className="ml-auto flex shrink-0 items-center gap-0.5 font-mono text-[10px] text-ink-muted group-hover:opacity-0">
                          <span>{items.length}</span>
                          <ChevronRight className="h-3 w-3" strokeWidth={1.8} />
                        </span>
                      )}
                    </button>
                    <div
                      className={`pointer-events-none absolute right-1 top-1/2 -translate-y-1/2 flex items-center gap-0.5 pl-1.5 opacity-0 group-hover:pointer-events-auto group-hover:opacity-100 transition-opacity ${
                        isProjectRowHighlighted ? "bg-active" : "bg-hover"
                      }`}
                    >
                      <button
                        type="button"
                        title="在此项目新建会话"
                        onClick={() => onNewSessionForProject(project.id)}
                        className="flex h-5 w-5 items-center justify-center text-ink-muted hover:bg-active hover:text-ink-strong cursor-pointer"
                      >
                        <Plus className="h-3.5 w-3.5" />
                      </button>
                      <button
                        type="button"
                        title="移除项目"
                        onClick={() => onDeleteProject(project.id)}
                        className="flex h-5 w-5 items-center justify-center text-ink-muted hover:bg-active hover:text-danger cursor-pointer"
                      >
                        <Trash2 className="h-3 w-3" />
                      </button>
                    </div>
                  </div>

                  {!isCollapsed && items.length > 0 && renderSessionRows(project.id, items)}
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Thin Seam Resize Handle with Invisible Wide Grab Target */}
      <div
        onPointerDown={handleResizeDragStart}
        title="拖拽调整侧边栏宽度"
        className={`absolute top-0 bottom-0 -right-[3px] z-20 w-[6px] cursor-col-resize transition-colors ${
          draggingResize ? "bg-ink-muted/40" : "hover:bg-ink-muted/25"
        }`}
      />
    </aside>
  );
};
