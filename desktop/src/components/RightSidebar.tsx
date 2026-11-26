import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  FileDiff,
  GitBranch,
  Info,
  Loader2,
  PanelRightClose,
  RefreshCw,
} from "lucide-react";
import type {
  AgentInstance,
  Project,
  ToolDiff,
  WorkspaceGitChanges,
  WorkspaceGitFile,
} from "../shared/protocol";
import type { ChatTimelineState } from "../shared/timeline";
import { DiffView } from "./DiffView";

export type RightSidebarTab = "changes" | "info";

interface RightSidebarProps {
  open: boolean;
  width: number;
  tab: RightSidebarTab;
  onChangeTab: (tab: RightSidebarTab) => void;
  onResize: (width: number) => void;
  onClose: () => void;
  activeAgent: AgentInstance | undefined;
  activeProject: Project | undefined;
  isRemoteHost?: boolean;
  timeline: ChatTimelineState | undefined;
}

const MIN_WIDTH = 280;
const MAX_WIDTH = 680;
const MAX_DIFF_LINES = 260;

const InlineCopyButton: React.FC<{ text: string; title?: string }> = ({
  text,
  title = "复制",
}) => {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      onClick={(e) => {
        e.stopPropagation();
        void navigator.clipboard.writeText(text);
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      }}
      title={copied ? "已复制" : title}
      aria-label={copied ? "已复制" : title}
      className="flex h-6 w-6 shrink-0 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
    >
      {copied ? <Check className="h-3 w-3 text-success" /> : <Copy className="h-3 w-3" />}
    </button>
  );
};

function formatTokens(n?: number | null): string {
  if (n == null) return "—";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

function gitStatusTone(status: string): string {
  switch (status) {
    case "A":
    case "?":
      return "text-success border-success/35 bg-success-soft";
    case "D":
      return "text-danger border-danger/35 bg-danger-soft";
    case "U":
      return "text-question border-question/35 bg-question-soft";
    case "R":
      return "text-ink-2 border-line bg-active";
    default:
      return "text-working border-working/35 bg-working-soft";
  }
}

function splitFilePath(filePath: string): { dir: string; base: string } {
  const idx = filePath.lastIndexOf("/");
  if (idx < 0) return { dir: "", base: filePath };
  return { dir: filePath.slice(0, idx + 1), base: filePath.slice(idx + 1) };
}

/** Compact unified diff renderer matching DiffView palette with strict 0px square corners. */
const UnifiedDiffBlock: React.FC<{ rawDiff: string }> = ({ rawDiff }) => {
  const lines = useMemo(() => {
    const raw = rawDiff.split("\n");
    // Drop git header metadata lines (`diff --git`, `index`, `---`, `+++`) to keep view compact
    return raw.filter(
      (line) =>
        !line.startsWith("diff --git ") &&
        !line.startsWith("index ") &&
        !line.startsWith("--- ") &&
        !line.startsWith("+++ ") &&
        !line.startsWith("new file mode ") &&
        !line.startsWith("deleted file mode ") &&
        !line.startsWith("old mode ") &&
        !line.startsWith("new mode "),
    );
  }, [rawDiff]);

  if (!rawDiff.trim()) {
    return (
      <div className="px-2.5 py-2 text-[11.5px] text-ink-muted">
        暂无文本差异（可能是空文件、权限变更或二进制文件）
      </div>
    );
  }

  const truncated = lines.length > MAX_DIFF_LINES;
  const visible = truncated ? lines.slice(0, MAX_DIFF_LINES) : lines;

  return (
    <div className="max-h-[360px] overflow-auto bg-code-surface font-mono text-[11.5px] leading-[1.45] select-text">
      <div className="min-w-max">
        {visible.map((line, idx) => {
          const isHunk = line.startsWith("@@");
          const isAdd = !isHunk && line.startsWith("+");
          const isDel = !isHunk && line.startsWith("-");
          const style: React.CSSProperties | undefined = isAdd
            ? { backgroundColor: "var(--diff-add-bg)", color: "var(--diff-add-fg)" }
            : isDel
              ? { backgroundColor: "var(--diff-del-bg)", color: "var(--diff-del-fg)" }
              : isHunk
                ? { backgroundColor: "var(--surface-code-header)", color: "var(--ink-muted)" }
                : { color: "var(--ink-secondary)" };
          return (
            <div key={idx} style={style} className="whitespace-pre px-2.5 py-[1px]">
              {line || " "}
            </div>
          );
        })}
      </div>
      {truncated && (
        <div className="border-t border-line-subtle bg-code-header px-2.5 py-1 text-[10.5px] text-ink-muted">
          已截断显示前 {MAX_DIFF_LINES} 行（共 {lines.length} 行）
        </div>
      )}
    </div>
  );
};

/**
 * Orca-style collapsible & resizable Right Sidebar with live Workspace Git Changes
 * (works for BOTH terminal and chat sessions) and Session Info panels.
 * Strictly 0px square corners per AGENTS.md.
 */
export const RightSidebar: React.FC<RightSidebarProps> = ({
  open,
  width,
  tab,
  onChangeTab,
  onResize,
  onClose,
  activeAgent,
  activeProject,
  isRemoteHost = false,
  timeline,
}) => {
  const [dragging, setDragging] = useState(false);
  const [expandedDiffId, setExpandedDiffId] = useState<string | null>(null);
  const [expandedGitPaths, setExpandedGitPaths] = useState<Record<string, boolean>>({});
  const [gitChanges, setGitChanges] = useState<WorkspaceGitChanges | null>(null);
  const [gitLoading, setGitLoading] = useState(false);
  const [gitDiffCache, setGitDiffCache] = useState<Record<string, string>>({});
  const [gitDiffLoading, setGitDiffLoading] = useState<Record<string, boolean>>({});

  const startXRef = useRef(0);
  const startWidthRef = useRef(width);

  const projectPath = activeProject?.path ?? null;

  const refreshGitChanges = useCallback(
    async (silent = false) => {
      if (!projectPath || isRemoteHost) {
        setGitChanges(null);
        return;
      }
      if (!silent) setGitLoading(true);
      try {
        const res = await window.muxlane.getGitChanges(projectPath);
        setGitChanges(res);
      } catch (err) {
        setGitChanges({
          isRepo: false,
          branch: null,
          files: [],
          error: (err as Error).message,
        });
      } finally {
        if (!silent) setGitLoading(false);
      }
    },
    [isRemoteHost, projectPath],
  );

  // Reset per-project expansion/diff cache when switching project
  useEffect(() => {
    setExpandedGitPaths({});
    setGitDiffCache({});
    setGitDiffLoading({});
  }, [projectPath]);

  // Fetch live workspace git changes whenever the Changes tab is visible (for BOTH terminal & chat)
  useEffect(() => {
    if (!open || tab !== "changes" || !projectPath || isRemoteHost) return;
    void refreshGitChanges(false);
    const timer = window.setInterval(() => {
      void refreshGitChanges(true);
    }, 5000);
    return () => window.clearInterval(timer);
  }, [open, tab, projectPath, isRemoteHost, refreshGitChanges]);

  const toggleGitFile = async (file: WorkspaceGitFile) => {
    const nextOpen = !expandedGitPaths[file.path];
    setExpandedGitPaths((prev) => ({ ...prev, [file.path]: nextOpen }));
    if (nextOpen && projectPath) {
      setGitDiffLoading((prev) => ({ ...prev, [file.path]: true }));
      try {
        const diff = await window.muxlane.getGitFileDiff(projectPath, file.path, file.status);
        setGitDiffCache((prev) => ({ ...prev, [file.path]: diff }));
      } finally {
        setGitDiffLoading((prev) => ({ ...prev, [file.path]: false }));
      }
    }
  };

  const fileDiffs = useMemo(() => {
    const entries = timeline?.entries ?? [];
    const diffs: Array<{
      id: string;
      path: string;
      name: string;
      diff: ToolDiff;
      isError: boolean;
    }> = [];
    for (const entry of entries) {
      if (entry.type === "tool_call" && entry.diff) {
        const path =
          entry.diff.path ||
          (entry.input &&
          typeof entry.input === "object" &&
          typeof (entry.input as Record<string, unknown>).path === "string"
            ? String((entry.input as Record<string, unknown>).path)
            : entry.name);
        diffs.push({
          id: entry.id,
          path,
          name: entry.name,
          diff: { ...entry.diff, path },
          isError: entry.is_error,
        });
      }
    }
    return diffs;
  }, [timeline?.entries]);

  const totalGitStats = useMemo(() => {
    let add = 0;
    let del = 0;
    for (const f of gitChanges?.files ?? []) {
      add += f.additions;
      del += f.deletions;
    }
    return { add, del };
  }, [gitChanges?.files]);

  const handleDragStart = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    startXRef.current = e.clientX;
    startWidthRef.current = width;
    setDragging(true);

    const onMove = (ev: PointerEvent) => {
      const delta = startXRef.current - ev.clientX;
      const next = Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, startWidthRef.current + delta));
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

  if (!open) return null;

  const isChatSession = Boolean(activeAgent && !activeAgent.tmux_session);
  const meta = timeline?.meta;
  const changesBadgeCount =
    gitChanges?.isRepo && !isRemoteHost ? gitChanges.files.length : fileDiffs.length;

  return (
    <aside
      style={{ width }}
      className="relative flex h-full shrink-0 flex-col border-l border-line-subtle bg-sidebar select-none"
    >
      {/* Left Resize Handle */}
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label="调整右侧栏宽度"
        onPointerDown={handleDragStart}
        className={`absolute top-0 bottom-0 -left-[2px] z-20 w-[5px] cursor-col-resize transition-colors ${
          dragging ? "bg-line-strong" : "hover:bg-line"
        }`}
      />

      {/* Orca-style Right Sidebar Tab Header (36px = h-9) */}
      <div className="flex h-9 shrink-0 items-stretch justify-between border-b border-line-subtle bg-tab-bar">
        <div className="flex min-w-0 items-stretch">
          <button
            type="button"
            onClick={() => onChangeTab("changes")}
            className={`relative flex items-center gap-1.5 border-r border-line-subtle px-3 text-[12px] transition-colors cursor-pointer ${
              tab === "changes"
                ? "bg-tab-active font-medium text-ink-strong"
                : "text-ink-muted hover:bg-hover/60 hover:text-ink"
            }`}
          >
            {tab === "changes" && (
              <span className="pointer-events-none absolute inset-x-0 -bottom-[1px] h-[1px] bg-tab-active" />
            )}
            <FileDiff className="h-3.5 w-3.5 shrink-0" strokeWidth={1.75} />
            <span>改动</span>
            {changesBadgeCount > 0 && (
              <span className="font-mono text-[10.5px] text-ink-muted tabular-nums">
                {changesBadgeCount}
              </span>
            )}
          </button>

          <button
            type="button"
            onClick={() => onChangeTab("info")}
            className={`relative flex items-center gap-1.5 border-r border-line-subtle px-3 text-[12px] transition-colors cursor-pointer ${
              tab === "info"
                ? "bg-tab-active font-medium text-ink-strong"
                : "text-ink-muted hover:bg-hover/60 hover:text-ink"
            }`}
          >
            {tab === "info" && (
              <span className="pointer-events-none absolute inset-x-0 -bottom-[1px] h-[1px] bg-tab-active" />
            )}
            <Info className="h-3.5 w-3.5 shrink-0" strokeWidth={1.75} />
            <span>详情</span>
          </button>
        </div>

        <button
          type="button"
          onClick={onClose}
          title="收起右侧栏 (Ctrl+])"
          aria-label="收起右侧栏"
          className="flex w-9 shrink-0 items-center justify-center border-l border-line-subtle text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
        >
          <PanelRightClose className="h-[15px] w-[15px]" strokeWidth={1.75} />
        </button>
      </div>

      {/* Panel Body */}
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden bg-canvas">
        {!activeAgent ? (
          <div className="flex flex-1 items-center justify-center px-4 text-center text-[12px] text-ink-muted">
            请先在左侧选择一个会话
          </div>
        ) : tab === "changes" ? (
          <div tabIndex={-1} className="flex-1 overflow-y-auto p-3 select-text focus:outline-none">
            {/* Live Workspace Git Changes (available for BOTH Terminal & Chat sessions) */}
            {!isRemoteHost && projectPath && (
              <div className="space-y-2">
                <div className="flex items-center justify-between gap-2 border-b border-line-subtle pb-2">
                  <div className="flex min-w-0 items-center gap-1.5 font-mono text-[11px] text-ink-2">
                    <GitBranch className="h-3.5 w-3.5 shrink-0 text-ink-muted" strokeWidth={1.75} />
                    <span className="truncate font-medium text-ink-strong">
                      {gitChanges?.branch || activeProject?.branch || activeProject?.name}
                    </span>
                    {gitChanges?.isRepo && gitChanges.files.length > 0 && (
                      <span className="flex shrink-0 items-center gap-1.5 tabular-nums">
                        <span style={{ color: "var(--diff-add-fg)" }}>+{totalGitStats.add}</span>
                        <span style={{ color: "var(--diff-del-fg)" }}>-{totalGitStats.del}</span>
                      </span>
                    )}
                  </div>
                  <button
                    type="button"
                    onClick={() => {
                      setGitDiffCache({});
                      void refreshGitChanges(false);
                    }}
                    title="刷新工作区改动"
                    className="flex h-6 w-6 shrink-0 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
                  >
                    <RefreshCw
                      className={`h-3 w-3 ${gitLoading ? "animate-spin text-working" : ""}`}
                    />
                  </button>
                </div>

                {gitLoading && !gitChanges ? (
                  <div className="flex items-center justify-center gap-2 py-8 text-[12px] text-ink-muted">
                    <Loader2 className="h-3.5 w-3.5 animate-spin text-working" />
                    <span>读取工作区 Git 改动…</span>
                  </div>
                ) : gitChanges && !gitChanges.isRepo ? (
                  <div className="py-6 text-center text-[12px] text-ink-muted">
                    {gitChanges.error
                      ? `读取 Git 状态失败: ${gitChanges.error}`
                      : "当前项目目录不是 Git 仓库"}
                  </div>
                ) : gitChanges && gitChanges.files.length === 0 ? (
                  <div className="py-8 text-center text-[12px] text-ink-muted">
                    工作区干净，暂无未提交改动
                  </div>
                ) : (
                  <div className="space-y-1.5">
                    {(gitChanges?.files ?? []).map((file) => {
                      const isOpen = Boolean(expandedGitPaths[file.path]);
                      const loadingDiff = Boolean(gitDiffLoading[file.path]);
                      const diffText = gitDiffCache[file.path];
                      const { dir, base } = splitFilePath(file.path);
                      return (
                        <div key={file.path} className="border border-line-subtle bg-elevated">
                          <button
                            type="button"
                            onClick={() => void toggleGitFile(file)}
                            className="flex w-full items-center justify-between gap-2 px-2 py-1.5 text-left hover:bg-hover transition-colors cursor-pointer"
                          >
                            <div className="flex min-w-0 flex-1 items-center gap-1.5">
                              {isOpen ? (
                                <ChevronDown className="h-3 w-3 shrink-0 text-ink-muted" />
                              ) : (
                                <ChevronRight className="h-3 w-3 shrink-0 text-ink-muted" />
                              )}
                              <span
                                className={`inline-flex h-4 w-4 shrink-0 items-center justify-center border font-mono text-[10px] font-semibold ${gitStatusTone(
                                  file.status,
                                )}`}
                              >
                                {file.status}
                              </span>
                              <span className="min-w-0 truncate font-mono text-[11.5px]">
                                {dir && <span className="text-ink-muted">{dir}</span>}
                                <span className="text-ink-strong">{base}</span>
                              </span>
                            </div>
                            <div className="flex shrink-0 items-center gap-1.5 font-mono text-[10.5px] tabular-nums">
                              {file.additions > 0 && (
                                <span style={{ color: "var(--diff-add-fg)" }}>
                                  +{file.additions}
                                </span>
                              )}
                              {file.deletions > 0 && (
                                <span style={{ color: "var(--diff-del-fg)" }}>
                                  -{file.deletions}
                                </span>
                              )}
                            </div>
                          </button>

                          {isOpen && (
                            <div className="border-t border-line-subtle">
                              <div className="flex items-center justify-between border-b border-line-subtle bg-code-header px-2.5 py-0.5 font-mono text-[10.5px] text-ink-muted">
                                <span className="truncate">{file.path}</span>
                                <InlineCopyButton text={file.path} title="复制路径" />
                              </div>
                              {loadingDiff ? (
                                <div className="flex items-center gap-2 px-2.5 py-3 text-[11.5px] text-ink-muted">
                                  <Loader2 className="h-3 w-3 animate-spin text-working" />
                                  <span>加载差异…</span>
                                </div>
                              ) : (
                                <UnifiedDiffBlock rawDiff={diffText ?? ""} />
                              )}
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            )}

            {/* Structured Chat Tool Diffs (shown on remote hosts or alongside workspace git) */}
            {(isRemoteHost || fileDiffs.length > 0) && (
              <div className={!isRemoteHost && projectPath ? "mt-4 border-t border-line-subtle pt-3" : ""}>
                <div className="mb-2 text-[11px] font-medium uppercase tracking-wider text-ink-muted">
                  本次对话文件操作 ({fileDiffs.length})
                </div>
                {fileDiffs.length === 0 ? (
                  <div className="py-4 text-center text-[12px] text-ink-muted">
                    {isRemoteHost
                      ? "远程主机暂仅展示对话内结构化文件改动"
                      : "本次对话暂无工具文件改动"}
                  </div>
                ) : (
                  <div className="space-y-1.5">
                    {fileDiffs.map((item) => {
                      const isOpen = expandedDiffId === item.id || fileDiffs.length <= 3;
                      return (
                        <div key={item.id} className="border border-line-subtle bg-elevated">
                          <button
                            type="button"
                            onClick={() =>
                              setExpandedDiffId((prev) => (prev === item.id ? null : item.id))
                            }
                            className="flex w-full items-center justify-between gap-2 px-2.5 py-1.5 text-left hover:bg-hover transition-colors cursor-pointer"
                          >
                            <span className="min-w-0 truncate font-mono text-[11.5px] text-ink-strong">
                              {item.path}
                            </span>
                            <span
                              className={`shrink-0 font-mono text-[10px] ${
                                item.isError ? "text-danger" : "text-ink-muted"
                              }`}
                            >
                              {item.name}
                            </span>
                          </button>
                          {isOpen && (
                            <div className="border-t border-line-subtle p-1.5">
                              <DiffView diff={item.diff} defaultExpanded />
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            )}
          </div>
        ) : (
          <div tabIndex={-1} className="flex-1 overflow-y-auto p-3.5 select-text focus:outline-none">
            <div className="space-y-3">
              {/* Session Header */}
              <div className="border-b border-line-subtle pb-3">
                <div className="text-[13px] font-semibold text-ink-strong break-words">
                  {activeAgent.title}
                </div>
                <div className="mt-1 flex flex-wrap items-center gap-2 font-mono text-[11px] text-ink-muted">
                  <span>{isChatSession ? "对话会话" : "终端会话"}</span>
                  <span>·</span>
                  <span>{activeAgent.agent_type}</span>
                  <span>·</span>
                  <span>{activeAgent.status}</span>
                </div>
              </div>

              {/* Context Meter */}
              {meta?.context_used_tokens != null && (
                <div className="border border-line-subtle bg-elevated p-2.5">
                  <div className="flex items-center justify-between text-[11px] text-ink-muted">
                    <span>上下文占用</span>
                    <span className="font-mono text-ink-strong tabular-nums">
                      {formatTokens(meta.context_used_tokens)}
                      {meta.context_window_tokens
                        ? ` / ${formatTokens(meta.context_window_tokens)}`
                        : ""}
                    </span>
                  </div>
                  {meta.context_window_tokens && meta.context_window_tokens > 0 && (
                    <div className="mt-1.5 h-1.5 w-full bg-hover overflow-hidden">
                      <div
                        className="h-full bg-ink-2"
                        style={{
                          width: `${Math.min(
                            100,
                            Math.round(
                              (meta.context_used_tokens / meta.context_window_tokens) * 100,
                            ),
                          )}%`,
                        }}
                      />
                    </div>
                  )}
                </div>
              )}

              {/* Metadata Grid */}
              <div className="divide-y divide-line-subtle border border-line-subtle bg-elevated text-[11.5px]">
                {activeProject && (
                  <div className="flex items-center justify-between gap-2 px-2.5 py-2">
                    <span className="shrink-0 text-ink-muted">工作区</span>
                    <div className="flex min-w-0 items-center gap-1">
                      <span className="truncate font-mono text-ink" title={activeProject.path}>
                        {activeProject.path}
                      </span>
                      <InlineCopyButton text={activeProject.path} title="复制目录" />
                    </div>
                  </div>
                )}

                {activeProject?.branch && (
                  <div className="flex items-center justify-between gap-2 px-2.5 py-2">
                    <span className="shrink-0 text-ink-muted">分支</span>
                    <span className="truncate font-mono text-ink">{activeProject.branch}</span>
                  </div>
                )}

                {meta?.current_model && (
                  <div className="flex items-center justify-between gap-2 px-2.5 py-2">
                    <span className="shrink-0 text-ink-muted">模型</span>
                    <span className="truncate font-mono text-ink">
                      {meta.current_model.name || meta.current_model.id}
                    </span>
                  </div>
                )}

                {meta?.thinking_level && (
                  <div className="flex items-center justify-between gap-2 px-2.5 py-2">
                    <span className="shrink-0 text-ink-muted">思考等级</span>
                    <span className="font-mono text-ink">{meta.thinking_level}</span>
                  </div>
                )}

                {meta?.driver && isChatSession && (
                  <div className="flex items-center justify-between gap-2 px-2.5 py-2">
                    <span className="shrink-0 text-ink-muted">驱动</span>
                    <span className="font-mono text-ink">{meta.driver}</span>
                  </div>
                )}

                {activeAgent.tmux_session && (
                  <div className="flex items-center justify-between gap-2 px-2.5 py-2">
                    <span className="shrink-0 text-ink-muted">tmux</span>
                    <div className="flex min-w-0 items-center gap-1">
                      <span className="truncate font-mono text-ink">
                        {activeAgent.tmux_session}
                      </span>
                      <InlineCopyButton text={activeAgent.tmux_session} title="复制 tmux 会话名" />
                    </div>
                  </div>
                )}

                <div className="flex items-center justify-between gap-2 px-2.5 py-2">
                  <span className="shrink-0 text-ink-muted">会话 ID</span>
                  <div className="flex min-w-0 items-center gap-1">
                    <span className="truncate font-mono text-ink-2">{activeAgent.id}</span>
                    <InlineCopyButton text={activeAgent.id} title="复制会话 ID" />
                  </div>
                </div>
              </div>
            </div>
          </div>
        )}
      </div>
    </aside>
  );
};
