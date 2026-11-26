import React, { useMemo, useState } from "react";
import {
  BellRing,
  ChevronDown,
  ChevronRight,
  FolderGit2,
  FolderPlus,
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

interface SidebarProps {
  width: number;
  onResize: (nextWidth: number) => void;
  machineName: string;
  isRemoteHost: boolean;
  remoteHostsCount: number;
  projects: Project[];
  agents: AgentInstance[];
  chatSummaries: Record<string, ChatSessionSummary>;
  activeAgentId: string | null;
  activeSection: "workspace" | "bots";
  globalApprovals: PendingApproval[];
  onOpenRemotes: () => void;
  onSelectAgent: (agentId: string) => void;
  onAddProject: () => void;
  onDeleteProject: (projectId: string) => void;
  onNewSessionForProject: (projectId: string) => void;
  onNewSession: () => void;
  onRenameAgent: (agentId: string, nextTitle: string) => void;
  onDeleteAgent: (agentId: string) => void;
}

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
    case "qwen":
      return "QW";
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
  chatSummaries,
  activeAgentId,
  activeSection,
  globalApprovals,
  onOpenRemotes,
  onSelectAgent,
  onAddProject,
  onDeleteProject,
  onNewSessionForProject,
  onNewSession,
  onRenameAgent,
  onDeleteAgent,
}) => {
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [filterQuery, setFilterQuery] = useState("");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingTitle, setEditingTitle] = useState("");
  const [dragging, setDragging] = useState(false);

  const normalizedQuery = filterQuery.trim().toLowerCase();

  const filteredProjects = useMemo(() => {
    if (!normalizedQuery) {
      return projects.map((project) => ({
        project,
        agents: agents.filter((a) => a.project === project.id),
      }));
    }
    return projects
      .map((project) => {
        const projectMatch =
          project.name.toLowerCase().includes(normalizedQuery) ||
          (project.branch?.toLowerCase().includes(normalizedQuery) ?? false);
        const projectAgents = agents.filter((a) => a.project === project.id);
        const matchingAgents = projectMatch
          ? projectAgents
          : projectAgents.filter(
              (a) =>
                a.title.toLowerCase().includes(normalizedQuery) ||
                agentTypeLabel(a.agent_type).toLowerCase().includes(normalizedQuery),
            );
        return { project, agents: matchingAgents, projectMatch };
      })
      .filter((g) => g.projectMatch || g.agents.length > 0);
  }, [agents, normalizedQuery, projects]);

  const startRename = (agent: AgentInstance) => {
    setEditingId(agent.id);
    setEditingTitle(agent.title);
  };

  const commitRename = () => {
    if (editingId && editingTitle.trim()) {
      onRenameAgent(editingId, editingTitle.trim());
    }
    setEditingId(null);
  };

  const handleDragStart = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = width;
    setDragging(true);

    const onMove = (ev: PointerEvent) => {
      const next = Math.min(380, Math.max(216, startWidth + (ev.clientX - startX)));
      onResize(next);
    };
    const onUp = () => {
      setDragging(false);
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
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
                  onClick={() => onSelectAgent(ap.agent)}
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
      <div tabIndex={-1} className="mux-scroll-quiet flex-1 overflow-y-auto py-2 focus:outline-none">
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
            {filteredProjects.map(({ project, agents: projectAgents }) => {
              const isCollapsed = !normalizedQuery && collapsed[project.id] === true;
              return (
                <div key={project.id} className="px-1.5">
                  {/* Project Group Row — DeepSeek Harness style single 16px leading slot (Folder at rest, Chevron on hover) */}
                  <div className="group relative flex h-7 items-center px-2 hover:bg-hover transition-colors">
                    <button
                      type="button"
                      onClick={() =>
                        setCollapsed((prev) => ({ ...prev, [project.id]: !prev[project.id] }))
                      }
                      className="flex min-w-0 flex-1 items-center gap-1.5 text-left cursor-pointer"
                    >
                      <span className="inline-flex h-4 w-4 shrink-0 items-center justify-center text-ink-muted group-hover:hidden">
                        <FolderGit2 className="h-3.5 w-3.5" strokeWidth={1.7} />
                      </span>
                      <span className="hidden h-4 w-4 shrink-0 items-center justify-center text-ink-muted group-hover:inline-flex">
                        {isCollapsed ? (
                          <ChevronRight className="h-3.5 w-3.5" strokeWidth={1.9} />
                        ) : (
                          <ChevronDown className="h-3.5 w-3.5" strokeWidth={1.9} />
                        )}
                      </span>
                      <span className="truncate text-[12px] font-medium text-ink">
                        {project.name}
                      </span>
                      {project.branch && (
                        <span className="truncate font-mono text-[10px] text-ink-muted">
                          {project.branch}
                        </span>
                      )}
                    </button>
                    <div className="pointer-events-none absolute right-1 top-1/2 -translate-y-1/2 flex items-center gap-0.5 bg-hover pl-1.5 opacity-0 group-hover:pointer-events-auto group-hover:opacity-100 transition-opacity">
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

                  {/* Session Rows — exact same px-2 + 16px leading slot + gap-1.5 as Project Row */}
                  {!isCollapsed && (
                    <div className="mt-0.5 space-y-[1px]">
                      {projectAgents.length === 0 ? (
                        <button
                          type="button"
                          onClick={() => onNewSessionForProject(project.id)}
                          className="flex h-7 w-full items-center gap-1.5 px-2 text-left text-[11.5px] text-ink-muted hover:bg-hover hover:text-ink transition-colors cursor-pointer"
                        >
                          <span className="inline-flex h-4 w-4 shrink-0 items-center justify-center">
                            <Plus className="h-3 w-3" />
                          </span>
                          <span>新建会话</span>
                        </button>
                      ) : (
                        projectAgents.map((agent) => {
                          const isActive =
                            activeSection === "workspace" && activeAgentId === agent.id;
                          const summary = chatSummaries[agent.id];
                          const streaming = summary?.meta.is_streaming === true;
                          const unread =
                            !agent.seen &&
                            (agent.status === "done" ||
                              agent.status === "failed" ||
                              agent.status === "blocked");

                          return (
                            <div
                              key={agent.id}
                              onClick={() => onSelectAgent(agent.id)}
                              className={`group relative flex h-7 items-center justify-between gap-2 px-2 transition-colors cursor-pointer ${
                                isActive
                                  ? "bg-active text-ink-strong"
                                  : "text-ink-2 hover:bg-hover hover:text-ink"
                              }`}
                            >
                              {isActive && (
                                <span className="absolute left-0 top-1 bottom-1 w-[2px] bg-ink-strong" />
                              )}

                              <div className="flex min-w-0 flex-1 items-center gap-1.5">
                                <span className="inline-flex h-4 w-4 shrink-0 items-center justify-center">
                                  <span
                                    className={`h-1.5 w-1.5 ${statusSquareClass(
                                      agent.status,
                                      streaming,
                                    )}`}
                                  />
                                </span>
                                {editingId === agent.id ? (
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
                                    {agent.title}
                                  </span>
                                )}
                              </div>

                              {/* Resting tag — keeps constant layout; hover actions overlay smoothly on top */}
                              {editingId !== agent.id && (
                                <>
                                  <div className="flex shrink-0 items-center gap-1.5">
                                    {unread && (
                                      <span
                                        className="h-1.5 w-1.5 bg-unread"
                                        title="有未读更新"
                                      />
                                    )}
                                    <span className="font-mono text-[10px] text-ink-muted">
                                      {shortAgentTag(agent.agent_type)}
                                    </span>
                                  </div>

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
                                        startRename(agent);
                                      }}
                                      className="flex h-5 w-5 items-center justify-center text-ink-muted hover:text-ink-strong cursor-pointer"
                                    >
                                      <Pencil className="h-3 w-3" />
                                    </button>
                                    <button
                                      type="button"
                                      title="关闭会话"
                                      onClick={(e) => {
                                        e.stopPropagation();
                                        onDeleteAgent(agent.id);
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
                        })
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Thin Seam Resize Handle with Invisible Wide Grab Target */}
      <div
        onPointerDown={handleDragStart}
        title="拖拽调整侧边栏宽度"
        className={`absolute top-0 bottom-0 -right-[3px] z-20 w-[6px] cursor-col-resize transition-colors ${
          dragging ? "bg-ink-muted/40" : "hover:bg-ink-muted/25"
        }`}
      />
    </aside>
  );
};
