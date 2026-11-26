import React, { useState } from "react";
import {
  Bot,
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  ListChecks,
  Pause,
  Play,
  Square,
  Target,
  Terminal,
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
 * DeepSeek Harness-style composer dock for Pie extensions:
 * - GoalBar (objective, phase, rounds, pause/resume/clear)
 * - TodoPanel (Pie task ledger)
 * - SubagentsPanel (Pie delegated subagent runs)
 * - BackgroundJobsPanel (Pie background shell `bg` jobs)
 * Strictly 0px square corners throughout.
 */
export const ChatSessionDock: React.FC<ChatSessionDockProps> = ({ meta, onRunCommand }) => {
  const goal = meta.goal && meta.goal.status !== "complete" ? meta.goal : null;
  const tasks = meta.tasks ?? [];
  const subagents = meta.subagents ?? [];
  const bgJobs = meta.background_jobs ?? [];

  if (!goal && tasks.length === 0 && subagents.length === 0 && bgJobs.length === 0) {
    return null;
  }

  return (
    <div className="shrink-0 px-5 pt-2">
      <div className="mx-auto w-full max-w-[68rem] space-y-1.5">
        {goal && <GoalBarStrip goal={goal} onRunCommand={onRunCommand} />}
        {tasks.length > 0 && <TodoPanelStrip tasks={tasks} />}
        {subagents.length > 0 && (
          <SubagentsPanelStrip subagents={subagents} onRunCommand={onRunCommand} />
        )}
        {bgJobs.length > 0 && (
          <BackgroundJobsPanelStrip jobs={bgJobs} onRunCommand={onRunCommand} />
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
  const pendingCount = tasks.filter((t) => t.status !== "completed" && t.status !== "in_progress").length;
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
          <span className={inProgressCount > 0 ? "text-working" : ""}>进行中 {inProgressCount}</span>
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

// ── 3. Pie Subagents Panel ──────────────────────────────────────────────────

const SubagentsPanelStrip: React.FC<{
  subagents: SessionSubagent[];
  onRunCommand: (cmd: string) => void;
}> = ({ subagents, onRunCommand }) => {
  const runningCount = subagents.filter(
    (s) => s.status === "running" || s.status === "queued",
  ).length;
  const doneCount = subagents.filter((s) => s.status === "completed").length;
  const failedCount = subagents.filter(
    (s) => s.status === "error" || s.status === "stopped" || s.status === "interrupted",
  ).length;
  const [open, setOpen] = useState(runningCount > 0);
  const [expandedIds, setExpandedIds] = useState<Record<string, boolean>>({});

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
          <Bot className={`h-3.5 w-3.5 ${runningCount > 0 ? "text-working" : "text-ink-2"}`} />
          <span className="font-medium text-ink">子代理</span>
          <span className="font-mono text-[10.5px] text-ink-muted">({subagents.length})</span>
        </div>
        <div className="flex items-center gap-2 font-mono text-[10.5px] text-ink-muted tabular-nums">
          {runningCount > 0 && <span className="text-working">运行中 {runningCount}</span>}
          {runningCount > 0 && (doneCount > 0 || failedCount > 0) && <span>·</span>}
          {doneCount > 0 && <span className="text-done">已完成 {doneCount}</span>}
          {doneCount > 0 && failedCount > 0 && <span>·</span>}
          {failedCount > 0 && <span className="text-danger">已终止 {failedCount}</span>}
        </div>
      </button>

      {open && (
        <div className="max-h-44 overflow-y-auto divide-y divide-line-subtle border-t border-line-subtle">
          {subagents.map((sub) => {
            const isRunning = sub.status === "running";
            const isQueued = sub.status === "queued";
            const isDone = sub.status === "completed";
            const isErr = sub.status === "error" || sub.status === "interrupted";
            const isExpanded = !!expandedIds[sub.id];
            const elapsedMs =
              sub.started_at_ms != null
                ? Math.max(0, (sub.finished_at_ms ?? Date.now()) - sub.started_at_ms)
                : null;
            const hasDetail = Boolean(sub.output || sub.error);

            return (
              <div key={sub.id} className="px-3 py-1.5 text-[12px]">
                <div className="flex items-center gap-2">
                  <span
                    className={`h-2 w-2 shrink-0 ${
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
                  <span className="shrink-0 border border-line-subtle bg-canvas px-1 font-mono text-[10px] text-ink-2">
                    {sub.agent}
                  </span>
                  <button
                    type="button"
                    disabled={!hasDetail}
                    onClick={() =>
                      setExpandedIds((prev) => ({ ...prev, [sub.id]: !prev[sub.id] }))
                    }
                    className={`min-w-0 flex-1 truncate text-left text-ink ${
                      hasDetail ? "hover:text-ink-strong cursor-pointer" : "cursor-default"
                    }`}
                    title={sub.description}
                  >
                    {sub.description}
                  </button>
                  {sub.active_tool && isRunning && (
                    <span className="shrink-0 border border-working/30 bg-working-soft px-1 font-mono text-[10px] text-working">
                      {sub.active_tool}
                    </span>
                  )}
                  <span className="shrink-0 font-mono text-[10.5px] text-ink-muted tabular-nums">
                    {sub.turns > 0
                      ? `${sub.turns}${sub.max_turns ? `/${sub.max_turns}` : ""} 轮`
                      : subStatusLabel(sub.status)}
                    {elapsedMs != null && elapsedMs > 0 ? ` · ${formatDurationMs(elapsedMs)}` : ""}
                  </span>
                  {(isRunning || isQueued) && (
                    <button
                      type="button"
                      onClick={() => onRunCommand(`/agents`)}
                      title="管理子代理 (/agents)"
                      className="inline-flex h-5 items-center gap-1 border border-line bg-canvas px-1.5 font-mono text-[10px] text-ink-muted hover:border-line-strong hover:bg-hover hover:text-ink cursor-pointer"
                    >
                      <Square className="h-2 w-2" />
                      <span>管理</span>
                    </button>
                  )}
                </div>
                {isExpanded && hasDetail && (
                  <div className="mt-1.5 border-l-2 border-line-strong bg-canvas px-2.5 py-1.5 font-mono text-[11px] leading-[1.45] text-ink-2 whitespace-pre-wrap break-words select-text max-h-32 overflow-y-auto">
                    {sub.error ? (
                      <span className="text-danger">{sub.error}</span>
                    ) : (
                      sub.output
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};

// ── 4. Pie Background Shell Jobs (bg) Panel ─────────────────────────────────

const BackgroundJobsPanelStrip: React.FC<{
  jobs: BackgroundJob[];
  onRunCommand: (cmd: string) => void;
}> = ({ jobs, onRunCommand }) => {
  const runningCount = jobs.filter((j) => j.status === "running").length;
  const doneCount = jobs.filter((j) => j.status === "completed").length;
  const failedCount = jobs.filter((j) => j.status === "failed" || j.status === "killed").length;
  const [open, setOpen] = useState(runningCount > 0 || jobs.length <= 4);
  const [expandedIds, setExpandedIds] = useState<Record<string, boolean>>({});
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const copyPath = (job: BackgroundJob) => {
    if (!job.output_path) return;
    void navigator.clipboard?.writeText(job.output_path);
    setCopiedId(job.id);
    window.setTimeout(() => setCopiedId((cur) => (cur === job.id ? null : cur)), 1500);
  };

  return (
    <div className="border border-line bg-elevated">
      <div className="flex items-center justify-between gap-2 bg-sidebar/60 px-3 py-1 select-none">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="flex min-w-0 flex-1 items-center gap-1.5 text-left text-[11px] text-ink-2 hover:text-ink transition-colors cursor-pointer"
        >
          {open ? (
            <ChevronDown className="h-3 w-3 text-ink-muted" />
          ) : (
            <ChevronRight className="h-3 w-3 text-ink-muted" />
          )}
          <Terminal className={`h-3.5 w-3.5 ${runningCount > 0 ? "text-working" : "text-ink-2"}`} />
          <span className="font-medium text-ink">后台任务 (bg)</span>
          <span className="font-mono text-[10.5px] text-ink-muted">({jobs.length})</span>
        </button>

        <div className="flex shrink-0 items-center gap-2 font-mono text-[10.5px] text-ink-muted tabular-nums">
          {runningCount > 0 && <span className="text-working">运行中 {runningCount}</span>}
          {runningCount > 0 && (doneCount > 0 || failedCount > 0) && <span>·</span>}
          {doneCount > 0 && <span className="text-done">已完成 {doneCount}</span>}
          {doneCount > 0 && failedCount > 0 && <span>·</span>}
          {failedCount > 0 && <span className="text-danger">失败/终止 {failedCount}</span>}
          <button
            type="button"
            onClick={() => onRunCommand("/jobs")}
            title="在会话中查看所有后台任务 (/jobs)"
            className="ml-1 border border-line bg-canvas px-1.5 py-[1px] text-[10px] text-ink-2 hover:border-line-strong hover:bg-hover hover:text-ink cursor-pointer"
          >
            /jobs
          </button>
        </div>
      </div>

      {open && (
        <div className="max-h-44 overflow-y-auto divide-y divide-line-subtle border-t border-line-subtle">
          {jobs.map((job) => {
            const isRunning = job.status === "running";
            const isDone = job.status === "completed";
            const isErr = job.status === "failed" || job.status === "killed";
            const isExpanded = !!expandedIds[job.id];
            const elapsedMs =
              job.started_at_ms != null
                ? Math.max(0, (job.finished_at_ms ?? Date.now()) - job.started_at_ms)
                : null;
            const sizeLabel = formatBytes(job.bytes_written);

            return (
              <div key={job.id} className="px-3 py-1.5 text-[12px]">
                <div className="flex items-center gap-2">
                  <span
                    className={`h-2 w-2 shrink-0 ${
                      isRunning
                        ? "bg-working animate-pulse"
                        : isDone
                          ? "bg-done"
                          : isErr
                            ? "bg-danger"
                            : "bg-ink-faint"
                    }`}
                  />
                  <span className="shrink-0 border border-line-subtle bg-canvas px-1 font-mono text-[10px] text-ink-2">
                    {job.name}
                  </span>
                  <button
                    type="button"
                    onClick={() =>
                      setExpandedIds((prev) => ({ ...prev, [job.id]: !prev[job.id] }))
                    }
                    className="min-w-0 flex-1 truncate text-left text-ink hover:text-ink-strong cursor-pointer"
                    title={job.command || job.description}
                  >
                    {job.description !== job.name
                      ? job.description
                      : job.command || job.description}
                  </button>

                  <div className="flex shrink-0 items-center gap-2 font-mono text-[10.5px] text-ink-muted tabular-nums">
                    {job.exit_code != null && (
                      <span className={job.exit_code === 0 ? "text-done" : "text-danger"}>
                        exit {job.exit_code}
                      </span>
                    )}
                    {isRunning && <span className="text-working">运行中</span>}
                    {sizeLabel && <span>{sizeLabel}</span>}
                    {elapsedMs != null && elapsedMs > 0 && (
                      <span>{formatDurationMs(elapsedMs)}</span>
                    )}
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
                  </div>
                </div>

                {isExpanded && (
                  <div className="mt-1.5 space-y-1 border-l-2 border-line-strong bg-canvas px-2.5 py-1.5 font-mono text-[11px] text-ink-2 select-text">
                    {job.command && (
                      <div className="truncate">
                        <span className="text-ink-faint">$ </span>
                        <span className="text-ink">{job.command}</span>
                      </div>
                    )}
                    {job.output_path && (
                      <div className="truncate text-[10.5px] text-ink-muted">
                        log: {job.output_path}
                      </div>
                    )}
                    {job.error && (
                      <div className="whitespace-pre-wrap text-danger">{job.error}</div>
                    )}
                  </div>
                )}
              </div>
            );
          })}
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
