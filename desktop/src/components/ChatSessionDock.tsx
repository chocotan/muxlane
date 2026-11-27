import React, { useEffect, useMemo, useRef, useState } from "react";
import {
  Bot,
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  Layers,
  ListChecks,
  Pause,
  Play,
  Square,
  Target,
  Trash2,
} from "lucide-react";
import type {
  BackgroundJob,
  ChatSessionMeta,
  SessionGoal,
  SessionSubagent,
  SessionTask,
} from "../shared/protocol";
import { formatBytes, formatDurationMs } from "../shared/formatStats";

interface ChatSessionDockProps {
  meta: ChatSessionMeta;
  onRunCommand: (commandText: string) => void;
}

/**
 * Compact bottom composer dock: only GoalBar and TodoPanel remain anchored
 * above the composer so background jobs and subagents never crowd the input area.
 * Strictly 0px square corners throughout.
 */
export const ChatSessionDock: React.FC<ChatSessionDockProps> = ({ meta, onRunCommand }) => {
  const goal = meta.goal && meta.goal.status !== "complete" ? meta.goal : null;
  const tasks = meta.tasks ?? [];

  if (!goal && tasks.length === 0) {
    return null;
  }

  return (
    <div className="shrink-0 px-5 pt-2">
      <div className="mx-auto w-full max-w-[68rem] space-y-1.5">
        {goal && <GoalBarStrip goal={goal} onRunCommand={onRunCommand} />}
        {tasks.length > 0 && <TodoPanelStrip tasks={tasks} />}
      </div>
    </div>
  );
};

/**
 * DeepSeek Harness (`dsh`) style session header actions for Subagents & Background Jobs:
 * Compact top-right trigger buttons with live pulsing status dots and counts,
 * opening floating square-cornered dropdown popovers (live section first, collapsible
 * settled section with clear action below).
 */
export const ChatSessionHeaderActions: React.FC<ChatSessionDockProps> = ({
  meta,
  onRunCommand,
}) => {
  const subagents = meta.subagents ?? [];
  const bgJobs = meta.background_jobs ?? [];
  const [openMenu, setOpenMenu] = useState<"subagents" | "jobs" | null>(null);
  const barRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!openMenu) return;
    const onPointerDown = (e: MouseEvent) => {
      if (barRef.current && !barRef.current.contains(e.target as Node)) {
        setOpenMenu(null);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpenMenu(null);
      }
    };
    document.addEventListener("mousedown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [openMenu]);

  useEffect(() => {
    if (openMenu === "subagents" && subagents.length === 0) setOpenMenu(null);
    if (openMenu === "jobs" && bgJobs.length === 0) setOpenMenu(null);
  }, [openMenu, subagents.length, bgJobs.length]);

  if (subagents.length === 0 && bgJobs.length === 0) {
    return null;
  }

  return (
    <div
      ref={barRef}
      className="relative z-20 shrink-0 border-b border-line-subtle bg-canvas px-5 py-1.5"
    >
      <div className="mx-auto flex w-full max-w-[68rem] items-center justify-end gap-2">
        {subagents.length > 0 && (
          <SubagentsHeaderAction
            subagents={subagents}
            open={openMenu === "subagents"}
            onToggle={() => setOpenMenu((cur) => (cur === "subagents" ? null : "subagents"))}
            onClose={() => setOpenMenu(null)}
            onRunCommand={onRunCommand}
          />
        )}
        {bgJobs.length > 0 && (
          <BackgroundJobsHeaderAction
            jobs={bgJobs}
            open={openMenu === "jobs"}
            onToggle={() => setOpenMenu((cur) => (cur === "jobs" ? null : "jobs"))}
            onClose={() => setOpenMenu(null)}
            onRunCommand={onRunCommand}
          />
        )}
      </div>
    </div>
  );
};

// ── 1. DeepSeek Harness GoalBar ─────────────────────────────────────────────

const GoalBarStrip: React.FC<{
  goal: SessionGoal;
  onRunCommand: (cmd: string) => void;
}> = ({ goal, onRunCommand }) => {
  const [expanded, setExpanded] = useState(false);
  const isPaused = goal.status === "paused";
  const isArmed = goal.status === "active" && goal.armed;
  const phaseLabel = isPaused ? "已暂停" : isArmed ? "持续推进中" : "已激活";
  const phaseTone = isPaused
    ? "border-line bg-canvas text-ink-muted"
    : "border-working/40 bg-working-soft text-working";

  return (
    <div className="border border-line bg-elevated">
      <div className="flex items-center gap-2 px-3 py-1.5">
        <Target
          className={`h-3.5 w-3.5 shrink-0 ${isPaused ? "text-ink-muted" : "text-working"}`}
        />
        <span className={`shrink-0 border px-1.5 py-[1px] font-mono text-[10px] ${phaseTone}`}>
          {phaseLabel}
        </span>
        <span className="shrink-0 font-mono text-[10.5px] text-ink-muted tabular-nums">
          轮次 {goal.rounds}/{goal.max_rounds}
        </span>
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          className="min-w-0 flex-1 text-left text-[12px] text-ink hover:text-ink-strong transition-colors cursor-pointer"
          title={goal.objective}
        >
          <span className={expanded ? "whitespace-pre-wrap break-words" : "block truncate"}>
            {goal.objective}
          </span>
        </button>

        <div className="flex shrink-0 items-center gap-1">
          {isPaused ? (
            <button
              type="button"
              onClick={() => onRunCommand("/goal resume")}
              title="恢复目标执行"
              className="inline-flex h-5 items-center gap-1 border border-line bg-canvas px-1.5 text-[10.5px] text-ink-2 hover:border-line-strong hover:bg-hover hover:text-ink cursor-pointer"
            >
              <Play className="h-2.5 w-2.5" />
              <span>恢复</span>
            </button>
          ) : (
            <button
              type="button"
              onClick={() => onRunCommand("/goal pause")}
              title="暂停目标执行"
              className="inline-flex h-5 items-center gap-1 border border-line bg-canvas px-1.5 text-[10.5px] text-ink-2 hover:border-line-strong hover:bg-hover hover:text-ink cursor-pointer"
            >
              <Pause className="h-2.5 w-2.5" />
              <span>暂停</span>
            </button>
          )}
          <button
            type="button"
            onClick={() => onRunCommand("/goal clear")}
            title="清除当前目标"
            className="inline-flex h-5 w-5 items-center justify-center border border-line bg-canvas text-ink-muted hover:border-danger/40 hover:bg-danger-soft hover:text-danger cursor-pointer"
          >
            <Trash2 className="h-2.5 w-2.5" />
          </button>
        </div>
      </div>
    </div>
  );
};

// ── 2. DeepSeek Harness TodoPanel (Pie Tasks) ───────────────────────────────

const TodoPanelStrip: React.FC<{ tasks: SessionTask[] }> = ({ tasks }) => {
  const completedCount = tasks.filter((t) => t.status === "completed").length;
  const inProgressCount = tasks.filter((t) => t.status === "in_progress").length;
  const pendingCount = tasks.filter(
    (t) => t.status !== "completed" && t.status !== "in_progress",
  ).length;
  const hasOpen = inProgressCount > 0 || pendingCount > 0;
  const [open, setOpen] = useState(hasOpen);

  return (
    <div className="border border-line bg-elevated">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center justify-between gap-2 bg-sidebar/60 px-3 py-1 text-left text-[11px] text-ink-2 hover:bg-hover/60 transition-colors cursor-pointer select-none"
      >
        <div className="flex items-center gap-1.5">
          {open ? (
            <ChevronDown className="h-3 w-3 text-ink-muted" />
          ) : (
            <ChevronRight className="h-3 w-3 text-ink-muted" />
          )}
          <ListChecks className="h-3.5 w-3.5 text-ink-2" />
          <span className="font-medium text-ink">任务清单</span>
          <span className="font-mono text-[10.5px] text-ink-muted">({tasks.length})</span>
        </div>
        <div className="flex items-center gap-2 font-mono text-[10.5px] text-ink-muted tabular-nums">
          <span className={completedCount > 0 ? "text-done" : ""}>已完成 {completedCount}</span>
          <span>·</span>
          <span className={inProgressCount > 0 ? "text-working" : ""}>
            进行中 {inProgressCount}
          </span>
          <span>·</span>
          <span>待办 {pendingCount}</span>
        </div>
      </button>

      {open && (
        <div className="max-h-40 overflow-y-auto divide-y divide-line-subtle border-t border-line-subtle">
          {tasks.map((task) => {
            const isDone = task.status === "completed";
            const isActive = task.status === "in_progress";
            return (
              <div
                key={task.id}
                className="flex items-start gap-2 px-3 py-1.5 text-[12px] leading-[1.4]"
              >
                <span
                  className={`mt-1.5 h-2 w-2 shrink-0 ${
                    isDone
                      ? "bg-done"
                      : isActive
                        ? "bg-working animate-pulse"
                        : "border border-line-strong bg-transparent"
                  }`}
                />
                <span className="shrink-0 font-mono text-[10.5px] text-ink-faint">
                  #{task.id}
                </span>
                <div className="min-w-0 flex-1">
                  <div className={isDone ? "text-ink-muted line-through" : "text-ink"}>
                    {isActive && task.active_form ? task.active_form : task.subject}
                  </div>
                  {(task.agent_type || task.owner || task.blocked_by.length > 0) && (
                    <div className="mt-0.5 flex flex-wrap items-center gap-1.5 font-mono text-[10px] text-ink-muted">
                      {task.agent_type && (
                        <span className="border border-line-subtle bg-canvas px-1">
                          {task.agent_type}
                        </span>
                      )}
                      {task.owner && <span>owner:{task.owner}</span>}
                      {task.blocked_by.length > 0 && (
                        <span className="text-working">
                          等待 {task.blocked_by.map((id) => `#${id}`).join(", ")}
                        </span>
                      )}
                    </div>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};

// ── 3. dsh-style Subagents Header Action + Floating Popover ─────────────────

function isLiveSubagent(sub: SessionSubagent): boolean {
  return sub.status === "running" || sub.status === "queued";
}

const SubagentsHeaderAction: React.FC<{
  subagents: SessionSubagent[];
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  onRunCommand: (cmd: string) => void;
}> = ({ subagents, open, onToggle, onClose, onRunCommand }) => {
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [settledOpen, setSettledOpen] = useState(false);
  const [clearedIds, setClearedIds] = useState<Record<string, boolean>>({});
  const [now, setNow] = useState(() => Date.now());

  const { liveRows, settledRows } = useMemo(() => {
    const live: SessionSubagent[] = [];
    const settled: SessionSubagent[] = [];
    for (const s of subagents) {
      if (isLiveSubagent(s)) {
        live.push(s);
      } else if (!clearedIds[s.id]) {
        settled.push(s);
      }
    }
    live.sort((a, b) => (a.started_at_ms ?? 0) - (b.started_at_ms ?? 0));
    settled.sort(
      (a, b) =>
        (b.finished_at_ms ?? b.started_at_ms ?? 0) - (a.finished_at_ms ?? a.started_at_ms ?? 0),
    );
    return { liveRows: live, settledRows: settled };
  }, [subagents, clearedIds]);

  useEffect(() => {
    if (!open || liveRows.length === 0) return;
    setNow(Date.now());
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [open, liveRows.length]);

  const visibleCount = liveRows.length + settledRows.length;
  const triggerLabel =
    liveRows.length > 0
      ? `子代理 · ${liveRows.length} 运行中`
      : `子代理 · ${visibleCount > 0 ? visibleCount : subagents.length}`;
  const showSettled = liveRows.length === 0 || settledOpen;

  const renderRow = (sub: SessionSubagent) => {
    const isRunning = sub.status === "running";
    const isQueued = sub.status === "queued";
    const isDone = sub.status === "completed";
    const isErr = sub.status === "error" || sub.status === "interrupted";
    const isExpanded = expandedId === sub.id;
    const elapsedMs =
      sub.started_at_ms != null
        ? Math.max(0, (sub.finished_at_ms ?? now) - sub.started_at_ms)
        : null;
    const hasDetail = Boolean(sub.output || sub.error);

    return (
      <div key={sub.id} className="px-3 py-2 text-[12px]">
        <div className="flex items-start gap-2">
          <span
            className={`mt-1.5 h-2 w-2 shrink-0 ${
              isRunning
                ? "bg-working animate-pulse"
                : isQueued
                  ? "border border-working bg-transparent"
                  : isDone
                    ? "bg-done"
                    : isErr
                      ? "bg-danger"
                      : "bg-ink-faint"
            }`}
          />
          <button
            type="button"
            disabled={!hasDetail}
            onClick={() => setExpandedId((cur) => (cur === sub.id ? null : sub.id))}
            className={`min-w-0 flex-1 text-left ${hasDetail ? "cursor-pointer" : "cursor-default"}`}
            title={sub.description}
          >
            <div className="flex items-center gap-1.5">
              <span className="truncate font-medium text-ink-strong">{sub.description}</span>
            </div>
            <div className="mt-0.5 flex flex-wrap items-center gap-1.5 font-mono text-[10.5px] text-ink-muted tabular-nums">
              <span className="border border-line-subtle bg-canvas px-1 text-[10px] text-ink-2">
                {sub.agent}
              </span>
              {sub.active_tool && isRunning && (
                <span className="border border-working/30 bg-working-soft px-1 text-[10px] text-working">
                  {sub.active_tool}
                </span>
              )}
              <span>
                {sub.turns > 0
                  ? `${sub.turns}${sub.max_turns ? `/${sub.max_turns}` : ""} 轮`
                  : subStatusLabel(sub.status)}
              </span>
              {elapsedMs != null && elapsedMs > 0 && <span>· {formatDurationMs(elapsedMs)}</span>}
            </div>
          </button>
          <div className="flex shrink-0 items-center gap-1 pt-0.5">
            {(isRunning || isQueued) && (
              <button
                type="button"
                onClick={() => {
                  onClose();
                  onRunCommand("/agents");
                }}
                title="管理子代理 (/agents)"
                className="inline-flex h-5 items-center gap-1 border border-line bg-canvas px-1.5 font-mono text-[10px] text-ink-muted hover:border-line-strong hover:bg-hover hover:text-ink cursor-pointer"
              >
                <Square className="h-2 w-2" />
                <span>管理</span>
              </button>
            )}
            {hasDetail && (
              <button
                type="button"
                onClick={() => setExpandedId((cur) => (cur === sub.id ? null : sub.id))}
                className="inline-flex h-5 w-5 items-center justify-center text-ink-muted hover:text-ink-strong cursor-pointer"
                aria-label="展开子代理详情"
              >
                <ChevronDown
                  className={`h-3 w-3 transition-transform ${isExpanded ? "rotate-180" : ""}`}
                />
              </button>
            )}
          </div>
        </div>
        {isExpanded && hasDetail && (
          <div className="mt-2 border-l-2 border-line-strong bg-canvas px-2.5 py-1.5 font-mono text-[11px] leading-[1.45] text-ink-2 whitespace-pre-wrap break-words select-text max-h-40 overflow-y-auto">
            {sub.error ? <span className="text-danger">{sub.error}</span> : sub.output}
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="relative">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className={`inline-flex h-6 items-center gap-1.5 border px-2 text-[11.5px] transition-colors cursor-pointer select-none ${
          open
            ? "border-line-strong bg-elevated text-ink-strong"
            : liveRows.length > 0
              ? "border-working/40 bg-working-soft/60 text-ink-strong hover:bg-working-soft"
              : "border-line bg-elevated text-ink-2 hover:border-line-strong hover:bg-hover hover:text-ink-strong"
        }`}
      >
        <span className="relative inline-flex items-center">
          <Bot className={`h-3.5 w-3.5 ${liveRows.length > 0 ? "text-working" : "text-ink-muted"}`} />
        </span>
        {liveRows.length > 0 && <span className="h-1.5 w-1.5 bg-working animate-pulse" />}
        <span className="font-mono text-[11px] tabular-nums">{triggerLabel}</span>
        <ChevronDown
          className={`h-3 w-3 text-ink-muted transition-transform ${open ? "rotate-180" : ""}`}
        />
      </button>

      {open && (
        <div className="mux-popover-shadow absolute top-full right-0 z-40 mt-1.5 flex w-[460px] max-w-[calc(100vw-3rem)] max-h-[60vh] flex-col border border-line-strong bg-elevated">
          <div className="flex items-center justify-between gap-2 border-b border-line-subtle bg-sidebar px-3 py-1.5 select-none">
            <div className="flex items-center gap-1.5 text-[11.5px] font-medium text-ink-strong">
              <Bot className="h-3.5 w-3.5 text-ink-2" />
              <span>子代理</span>
              <span className="font-mono text-[10.5px] text-ink-muted">({subagents.length})</span>
            </div>
            <button
              type="button"
              onClick={() => {
                onClose();
                onRunCommand("/agents");
              }}
              className="border border-line bg-canvas px-1.5 py-[1px] font-mono text-[10px] text-ink-2 hover:border-line-strong hover:bg-hover hover:text-ink-strong cursor-pointer"
            >
              /agents
            </button>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto divide-y divide-line-subtle">
            {liveRows.length > 0 && (
              <div>
                <div className="bg-sidebar/45 px-3 py-1 font-mono text-[10px] uppercase tracking-wider text-ink-muted select-none">
                  进行中 ({liveRows.length})
                </div>
                <div className="divide-y divide-line-subtle">{liveRows.map(renderRow)}</div>
              </div>
            )}

            {settledRows.length > 0 && (
              <div>
                <div className="flex items-center justify-between gap-2 bg-sidebar/45 px-3 py-1 select-none">
                  <button
                    type="button"
                    onClick={() => setSettledOpen((v) => !v)}
                    className="inline-flex items-center gap-1 font-mono text-[10px] text-ink-muted hover:text-ink-strong cursor-pointer"
                  >
                    <ChevronDown
                      className={`h-3 w-3 transition-transform ${showSettled ? "" : "-rotate-90"}`}
                    />
                    <span>已结束 ({settledRows.length})</span>
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      setClearedIds((prev) => {
                        const next = { ...prev };
                        for (const s of settledRows) next[s.id] = true;
                        return next;
                      });
                    }}
                    className="font-mono text-[10px] text-ink-muted hover:text-ink-strong cursor-pointer"
                  >
                    隐藏已结束
                  </button>
                </div>
                {showSettled && (
                  <div className="divide-y divide-line-subtle">{settledRows.map(renderRow)}</div>
                )}
              </div>
            )}

            {liveRows.length === 0 && settledRows.length === 0 && (
              <div className="flex items-center justify-between px-3 py-3 text-[11.5px] text-ink-muted">
                <span>已隐藏历史记录</span>
                <button
                  type="button"
                  onClick={() => setClearedIds({})}
                  className="border border-line bg-canvas px-2 py-0.5 font-mono text-[10.5px] text-ink-2 hover:bg-hover hover:text-ink-strong cursor-pointer"
                >
                  恢复显示
                </button>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

// ── 4. dsh-style Background Jobs (bg) Header Action + Floating Popover ──────

const BackgroundJobsHeaderAction: React.FC<{
  jobs: BackgroundJob[];
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  onRunCommand: (cmd: string) => void;
}> = ({ jobs, open, onToggle, onClose, onRunCommand }) => {
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [settledOpen, setSettledOpen] = useState(false);
  const [clearedIds, setClearedIds] = useState<Record<string, boolean>>({});
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());

  const { liveRows, settledRows } = useMemo(() => {
    const live: BackgroundJob[] = [];
    const settled: BackgroundJob[] = [];
    for (const j of jobs) {
      if (j.status === "running") {
        live.push(j);
      } else if (!clearedIds[j.id]) {
        settled.push(j);
      }
    }
    live.sort((a, b) => (a.started_at_ms ?? 0) - (b.started_at_ms ?? 0));
    settled.sort(
      (a, b) =>
        (b.finished_at_ms ?? b.started_at_ms ?? 0) - (a.finished_at_ms ?? a.started_at_ms ?? 0),
    );
    return { liveRows: live, settledRows: settled };
  }, [jobs, clearedIds]);

  useEffect(() => {
    if (!open || liveRows.length === 0) return;
    setNow(Date.now());
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [open, liveRows.length]);

  const copyPath = (job: BackgroundJob) => {
    if (!job.output_path) return;
    void navigator.clipboard?.writeText(job.output_path);
    setCopiedId(job.id);
    window.setTimeout(() => setCopiedId((cur) => (cur === job.id ? null : cur)), 1500);
  };

  const visibleCount = liveRows.length + settledRows.length;
  const triggerLabel =
    liveRows.length > 0
      ? `后台任务 · ${liveRows.length} 运行中`
      : `后台任务 · ${visibleCount > 0 ? visibleCount : jobs.length}`;
  const showSettled = liveRows.length === 0 || settledOpen;

  const renderRow = (job: BackgroundJob) => {
    const isRunning = job.status === "running";
    const isDone = job.status === "completed";
    const isErr = job.status === "failed" || job.status === "killed";
    const isExpanded = expandedId === job.id;
    const elapsedMs =
      job.started_at_ms != null
        ? Math.max(0, (job.finished_at_ms ?? now) - job.started_at_ms)
        : null;
    const sizeLabel = formatBytes(job.bytes_written);
    const labelText =
      job.description !== job.name ? job.description : job.command || job.description;

    return (
      <div key={job.id} className="px-3 py-2 text-[12px]">
        <div className="flex items-start gap-2">
          <span
            className={`mt-1.5 h-2 w-2 shrink-0 ${
              isRunning
                ? "bg-working animate-pulse"
                : isDone
                  ? "bg-done"
                  : isErr
                    ? "bg-danger"
                    : "bg-ink-faint"
            }`}
          />
          <button
            type="button"
            onClick={() => setExpandedId((cur) => (cur === job.id ? null : job.id))}
            className="min-w-0 flex-1 text-left cursor-pointer"
            title={job.command || job.description}
          >
            <div className="truncate font-medium text-ink-strong">{labelText}</div>
            <div className="mt-0.5 flex flex-wrap items-center gap-1.5 font-mono text-[10.5px] text-ink-muted tabular-nums">
              <span className="border border-line-subtle bg-canvas px-1 text-[10px] text-ink-2">
                {job.name}
              </span>
              {job.exit_code != null && (
                <span className={job.exit_code === 0 ? "text-done" : "text-danger"}>
                  exit {job.exit_code}
                </span>
              )}
              {isRunning && <span className="text-working">运行中</span>}
              {sizeLabel && <span>{sizeLabel}</span>}
              {elapsedMs != null && elapsedMs > 0 && <span>· {formatDurationMs(elapsedMs)}</span>}
            </div>
          </button>
          <div className="flex shrink-0 items-center gap-1 pt-0.5">
            {job.output_path && (
              <button
                type="button"
                onClick={() => copyPath(job)}
                title={`复制日志路径: ${job.output_path}`}
                className="inline-flex h-5 w-5 items-center justify-center border border-line-subtle bg-canvas text-ink-muted hover:border-line-strong hover:bg-hover hover:text-ink cursor-pointer"
              >
                {copiedId === job.id ? (
                  <Check className="h-2.5 w-2.5 text-done" />
                ) : (
                  <Copy className="h-2.5 w-2.5" />
                )}
              </button>
            )}
            <button
              type="button"
              onClick={() => setExpandedId((cur) => (cur === job.id ? null : job.id))}
              className="inline-flex h-5 w-5 items-center justify-center text-ink-muted hover:text-ink-strong cursor-pointer"
              aria-label="展开后台任务详情"
            >
              <ChevronDown
                className={`h-3 w-3 transition-transform ${isExpanded ? "rotate-180" : ""}`}
              />
            </button>
          </div>
        </div>

        {isExpanded && (
          <div className="mt-2 space-y-1 border-l-2 border-line-strong bg-canvas px-2.5 py-1.5 font-mono text-[11px] text-ink-2 select-text">
            {job.command && (
              <div className="break-all">
                <span className="text-ink-faint">$ </span>
                <span className="text-ink">{job.command}</span>
              </div>
            )}
            {job.output_path && (
              <div className="break-all text-[10.5px] text-ink-muted">log: {job.output_path}</div>
            )}
            {job.error && <div className="whitespace-pre-wrap text-danger">{job.error}</div>}
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="relative">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className={`inline-flex h-6 items-center gap-1.5 border px-2 text-[11.5px] transition-colors cursor-pointer select-none ${
          open
            ? "border-line-strong bg-elevated text-ink-strong"
            : liveRows.length > 0
              ? "border-working/40 bg-working-soft/60 text-ink-strong hover:bg-working-soft"
              : "border-line bg-elevated text-ink-2 hover:border-line-strong hover:bg-hover hover:text-ink-strong"
        }`}
      >
        <Layers className={`h-3.5 w-3.5 ${liveRows.length > 0 ? "text-working" : "text-ink-muted"}`} />
        {liveRows.length > 0 && <span className="h-1.5 w-1.5 bg-working animate-pulse" />}
        <span className="font-mono text-[11px] tabular-nums">{triggerLabel}</span>
        <ChevronDown
          className={`h-3 w-3 text-ink-muted transition-transform ${open ? "rotate-180" : ""}`}
        />
      </button>

      {open && (
        <div className="mux-popover-shadow absolute top-full right-0 z-40 mt-1.5 flex w-[480px] max-w-[calc(100vw-3rem)] max-h-[60vh] flex-col border border-line-strong bg-elevated">
          <div className="flex items-center justify-between gap-2 border-b border-line-subtle bg-sidebar px-3 py-1.5 select-none">
            <div className="flex items-center gap-1.5 text-[11.5px] font-medium text-ink-strong">
              <Layers className="h-3.5 w-3.5 text-ink-2" />
              <span>后台任务 (bg)</span>
              <span className="font-mono text-[10.5px] text-ink-muted">({jobs.length})</span>
            </div>
            <button
              type="button"
              onClick={() => {
                onClose();
                onRunCommand("/jobs");
              }}
              className="border border-line bg-canvas px-1.5 py-[1px] font-mono text-[10px] text-ink-2 hover:border-line-strong hover:bg-hover hover:text-ink-strong cursor-pointer"
            >
              /jobs
            </button>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto divide-y divide-line-subtle">
            {liveRows.length > 0 && (
              <div>
                <div className="bg-sidebar/45 px-3 py-1 font-mono text-[10px] uppercase tracking-wider text-ink-muted select-none">
                  进行中 ({liveRows.length})
                </div>
                <div className="divide-y divide-line-subtle">{liveRows.map(renderRow)}</div>
              </div>
            )}

            {settledRows.length > 0 && (
              <div>
                <div className="flex items-center justify-between gap-2 bg-sidebar/45 px-3 py-1 select-none">
                  <button
                    type="button"
                    onClick={() => setSettledOpen((v) => !v)}
                    className="inline-flex items-center gap-1 font-mono text-[10px] text-ink-muted hover:text-ink-strong cursor-pointer"
                  >
                    <ChevronDown
                      className={`h-3 w-3 transition-transform ${showSettled ? "" : "-rotate-90"}`}
                    />
                    <span>已结束 ({settledRows.length})</span>
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      setClearedIds((prev) => {
                        const next = { ...prev };
                        for (const j of settledRows) next[j.id] = true;
                        return next;
                      });
                    }}
                    className="font-mono text-[10px] text-ink-muted hover:text-ink-strong cursor-pointer"
                  >
                    隐藏已结束
                  </button>
                </div>
                {showSettled && (
                  <div className="divide-y divide-line-subtle">{settledRows.map(renderRow)}</div>
                )}
              </div>
            )}

            {liveRows.length === 0 && settledRows.length === 0 && (
              <div className="flex items-center justify-between px-3 py-3 text-[11.5px] text-ink-muted">
                <span>已隐藏历史任务</span>
                <button
                  type="button"
                  onClick={() => setClearedIds({})}
                  className="border border-line bg-canvas px-2 py-0.5 font-mono text-[10.5px] text-ink-2 hover:bg-hover hover:text-ink-strong cursor-pointer"
                >
                  恢复显示
                </button>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

function subStatusLabel(status: string): string {
  switch (status) {
    case "running":
      return "运行中";
    case "queued":
      return "排队中";
    case "completed":
      return "已完成";
    case "error":
      return "失败";
    case "stopped":
      return "已停止";
    case "interrupted":
      return "已中断";
    default:
      return status;
  }
}
