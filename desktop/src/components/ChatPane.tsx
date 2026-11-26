import React, { useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  ArrowDown,
  ArrowUpRight,
  Brain,
  Check,
  ChevronRight,
  Copy,
  FileCode2,
  FileText,
  Layers,
  Loader2,
  Search,
  ShieldAlert,
  Terminal as TerminalIcon,
  Wrench,
  X,
  XCircle,
} from "lucide-react";
import type {
  ApprovalReply,
  ChatAttachment,
  PendingApproval,
  SessionStats,
  StreamingBehavior,
  TimelineEntry,
} from "../shared/protocol";
import {
  buildDisplayItems,
  defaultChatMeta,
  summarizeToolInput,
  type ChatTimelineState,
  type DisplayTimelineItem,
} from "../shared/timeline";
import type { AppTheme } from "../state/useTheme";
import { ChatComposer } from "./ChatComposer";
import { ChatSessionDock } from "./ChatSessionDock";
import { MessageUsageFooter } from "./ChatStatsBar";
import { DiffView } from "./DiffView";
import { MarkdownView } from "./MarkdownView";

interface ChatPaneProps {
  agentId: string;
  agentTitle?: string;
  theme: AppTheme;
  timeline: ChatTimelineState | undefined;
  onSend: (
    text: string,
    attachments: ChatAttachment[],
    behavior?: StreamingBehavior,
  ) => Promise<void>;
  onAbort: () => void;
  onDeleteQueuedMessage?: (messageId?: string) => void;
  onApprove: (approvalId: string, reply: ApprovalReply) => void;
  onConfigure: (opts: {
    provider?: string;
    model_id?: string;
    thinking_level?: string;
  }) => void;
}

const STARTER_PROMPTS = [
  "帮我梳理当前项目的目录结构与核心入口",
  "检查 git status 与最近一次改动是否存在潜在问题",
  "运行测试套件并定位任何失败用例",
];

function isExecutionStep(item: DisplayTimelineItem): boolean {
  if (item.kind === "exploration_cluster") return true;
  return item.entry.type === "thinking" || item.entry.type === "tool_call";
}

function itemTopSpacingClass(
  prev: DisplayTimelineItem | undefined,
  curr: DisplayTimelineItem,
): string {
  if (!prev) return "";
  if (curr.kind === "single" && curr.entry.type === "user_message") {
    return "mt-4";
  }
  if (prev.kind === "single" && prev.entry.type === "user_message") {
    return "mt-2.5";
  }
  if (isExecutionStep(prev) && isExecutionStep(curr)) {
    return "mt-0.5";
  }
  if (isExecutionStep(prev) && curr.kind === "single" && curr.entry.type === "assistant_text") {
    return "mt-2";
  }
  if (prev.kind === "single" && prev.entry.type === "assistant_text" && isExecutionStep(curr)) {
    return "mt-1.5";
  }
  return "mt-2";
}

function formatDuration(ms?: number | null): string | null {
  if (ms == null || ms <= 0) return null;
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`;
  return `${ms}ms`;
}

function toolVerb(entry: Extract<TimelineEntry, { type: "tool_call" }>): string {
  const running = entry.state === "running" || entry.state === "pending";
  switch (entry.kind) {
    case "read":
      return running ? "正在读取" : "读取";
    case "search":
      return running ? "正在搜索" : "搜索";
    case "edit":
      return running ? "正在编辑" : "编辑";
    case "write":
      return running ? "正在写入" : "写入";
    case "bash":
      return running ? "正在运行" : "运行";
    default:
      return entry.name;
  }
}

function extractBashCommand(input: unknown): string | null {
  if (typeof input === "string") return input.trim() || null;
  if (input && typeof input === "object") {
    const obj = input as Record<string, unknown>;
    const cmd = obj.command ?? obj.cmd ?? obj.script;
    if (typeof cmd === "string" && cmd.trim()) return cmd.trim();
  }
  return null;
}

function summarizeCluster(tools: Extract<TimelineEntry, { type: "tool_call" }>[]): string {
  let reads = 0;
  let searches = 0;
  let other = 0;
  for (const t of tools) {
    if (t.kind === "read") reads++;
    else if (t.kind === "search") searches++;
    else other++;
  }
  const parts: string[] = [];
  if (reads > 0) parts.push(`读取 ${reads} 个文件`);
  if (searches > 0) parts.push(`搜索 ${searches} 次`);
  if (other > 0) parts.push(`${other} 项检查`);
  return parts.join(" · ") || `${tools.length} 个步骤`;
}

/** Fixed 24×24 square icon-only copy button: zero horizontal width shift on click. */
const QuietCopyButton: React.FC<{ text: string; alwaysVisible?: boolean }> = ({
  text,
  alwaysVisible = true,
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
      title={copied ? "已复制" : "复制"}
      aria-label={copied ? "已复制" : "复制"}
      className={`flex h-6 w-6 shrink-0 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer ${
        alwaysVisible || copied ? "opacity-100" : "opacity-0 group-hover:opacity-100"
      }`}
    >
      {copied ? (
        <Check className="h-3.5 w-3.5 text-success" />
      ) : (
        <Copy className="h-3.5 w-3.5" />
      )}
    </button>
  );
};

/** Quiet single-line reasoning disclosure (Orca-style). */
const ReasoningRow: React.FC<{
  entry: Extract<TimelineEntry, { type: "thinking" }>;
}> = ({ entry }) => {
  const [expanded, setExpanded] = useState(entry.streaming);
  const prevStreaming = useRef(entry.streaming);

  useEffect(() => {
    if (prevStreaming.current && !entry.streaming) {
      setExpanded(false);
    }
    prevStreaming.current = entry.streaming;
  }, [entry.streaming]);

  const durationLabel =
    entry.duration_ms != null && entry.duration_ms > 0
      ? `已思考 ${(entry.duration_ms / 1000).toFixed(1)}s`
      : entry.streaming
        ? "正在思考…"
        : "思考过程";

  return (
    <div className="min-w-0">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="group/reason -mx-1.5 flex min-h-[24px] w-[calc(100%+0.75rem)] items-center gap-2 px-1.5 py-[2px] text-left text-[12.5px] leading-[18px] text-ink-muted hover:bg-hover/60 hover:text-ink transition-colors cursor-pointer select-none"
      >
        {entry.streaming ? (
          <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-working" />
        ) : (
          <Brain className="h-3.5 w-3.5 shrink-0 text-ink-muted" strokeWidth={1.7} />
        )}
        <span className="truncate leading-[18px]">{durationLabel}</span>
        <ChevronRight
          className={`h-3 w-3 shrink-0 text-ink-muted transition-transform ${
            expanded
              ? "rotate-90 opacity-100"
              : "opacity-0 group-hover/reason:opacity-100"
          }`}
        />
      </button>

      {expanded && (
        <div className="mt-0.5 mb-1 ml-[6.5px] max-h-72 overflow-y-auto border-l border-line pl-3 text-[12.5px] leading-[1.42] text-ink-muted select-text">
          <pre className="m-0 whitespace-pre-wrap font-sans">{entry.text}</pre>
        </div>
      )}
    </div>
  );
};

function toolRowIcon(entry: Extract<TimelineEntry, { type: "tool_call" }>) {
  if (entry.state === "running" || entry.state === "pending") {
    return <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-working" />;
  }
  if (entry.is_error || entry.state === "failed") {
    return <XCircle className="h-3.5 w-3.5 shrink-0 text-danger" />;
  }
  switch (entry.kind) {
    case "bash":
      return <TerminalIcon className="h-3.5 w-3.5 shrink-0 text-ink-muted" strokeWidth={1.7} />;
    case "edit":
    case "write":
      return <FileCode2 className="h-3.5 w-3.5 shrink-0 text-ink-2" strokeWidth={1.7} />;
    case "read":
      return <FileText className="h-3.5 w-3.5 shrink-0 text-ink-muted" strokeWidth={1.7} />;
    case "search":
      return <Search className="h-3.5 w-3.5 shrink-0 text-ink-muted" strokeWidth={1.7} />;
    default:
      return <Wrench className="h-3.5 w-3.5 shrink-0 text-ink-muted" strokeWidth={1.7} />;
  }
}

const OUTPUT_PREVIEW_LIMIT = 4000;

/**
 * Orca-style NativeChatToolLine:
 * Single-line verb + monospace target + status/duration badge;
 * expands cleanly into command block / inline DiffView / truncated output with copy.
 */
const ToolCallRow: React.FC<{
  entry: Extract<TimelineEntry, { type: "tool_call" }>;
}> = ({ entry }) => {
  const [expanded, setExpanded] = useState(Boolean(entry.is_error || entry.diff));
  const [showFullOutput, setShowFullOutput] = useState(false);

  const summary = useMemo(() => summarizeToolInput(entry), [entry]);
  const verb = toolVerb(entry);
  const bashCmd = entry.kind === "bash" ? extractBashCommand(entry.input) : null;
  const duration = formatDuration(entry.duration_ms);
  const isRunning = entry.state === "running" || entry.state === "pending";
  const failed = entry.is_error || entry.state === "failed" || (entry.exit_code != null && entry.exit_code !== 0);

  const rawOutput = entry.output ?? "";
  const isTruncated = !showFullOutput && rawOutput.length > OUTPUT_PREVIEW_LIMIT;
  const displayedOutput = isTruncated
    ? `${rawOutput.slice(0, OUTPUT_PREVIEW_LIMIT)}\n…（已省略 ${rawOutput.length - OUTPUT_PREVIEW_LIMIT} 字符）`
    : rawOutput;

  const hasDetail = Boolean(entry.diff || bashCmd || entry.output || (entry.input != null && !entry.diff));

  return (
    <div className="min-w-0">
      <button
        type="button"
        onClick={() => hasDetail && setExpanded((v) => !v)}
        className={`group/tool -mx-1.5 flex min-h-[24px] w-[calc(100%+0.75rem)] items-center justify-between gap-3 px-1.5 py-[2px] text-left leading-[18px] transition-colors select-none ${
          hasDetail ? "hover:bg-hover/70 cursor-pointer" : "cursor-default"
        }`}
      >
        <div className="flex min-w-0 flex-1 items-center gap-2">
          {toolRowIcon(entry)}
          <span
            className={`shrink-0 text-[12.5px] font-medium ${
              failed ? "text-danger" : "text-ink-2"
            }`}
          >
            {verb}
          </span>
          {summary && (
            <span
              className={`min-w-0 truncate font-mono text-[12px] ${
                failed
                  ? "text-danger/90"
                  : "text-ink group-hover/tool:text-ink-strong"
              }`}
            >
              {summary}
            </span>
          )}
          {hasDetail && (
            <ChevronRight
              className={`h-3 w-3 shrink-0 text-ink-muted transition-transform ${
                expanded
                  ? "rotate-90 opacity-100"
                  : "opacity-0 group-hover/tool:opacity-100"
              }`}
            />
          )}
        </div>

        <div className="flex shrink-0 items-center gap-2 font-mono text-[10.5px] text-ink-muted tabular-nums">
          {entry.exit_code != null && entry.exit_code !== 0 && (
            <span className="border border-danger/35 bg-danger-soft px-1 text-danger">
              exit {entry.exit_code}
            </span>
          )}
          {!isRunning && !failed && entry.state === "completed" && (
            <Check className="h-3 w-3 text-success/80 opacity-0 group-hover/tool:opacity-100 transition-opacity" />
          )}
          {duration && <span>{duration}</span>}
        </div>
      </button>

      {expanded && hasDetail && (
        <div className="mt-1 mb-1.5 ml-[6.5px] space-y-1.5 border-l border-line pl-3 select-text">
          {/* Bash command line */}
          {bashCmd && (
            <div className="group relative border border-line-subtle bg-code-surface">
              <div className="flex items-center justify-between border-b border-line-subtle bg-code-header px-2.5 py-0.5 text-[10.5px] font-mono text-ink-muted">
                <span>命令</span>
                <QuietCopyButton text={bashCmd} alwaysVisible />
              </div>
              <pre className="m-0 max-h-40 overflow-auto whitespace-pre-wrap break-words px-2.5 py-1.5 font-mono text-[12px] leading-[1.4] text-ink-strong">
                $ {bashCmd}
              </pre>
            </div>
          )}

          {/* Non-bash, non-diff structured input */}
          {!bashCmd && !entry.diff && entry.input != null && (
            <div className="border border-line-subtle bg-code-surface">
              <div className="flex items-center justify-between border-b border-line-subtle bg-code-header px-2.5 py-0.5 text-[10.5px] font-mono text-ink-muted">
                <span>参数 · {entry.name}</span>
                <QuietCopyButton
                  text={
                    typeof entry.input === "string"
                      ? entry.input
                      : JSON.stringify(entry.input, null, 2)
                  }
                  alwaysVisible
                />
              </div>
              <pre className="m-0 max-h-44 overflow-auto whitespace-pre-wrap break-words px-2.5 py-1.5 font-mono text-[11.5px] leading-[1.4] text-ink-2">
                {typeof entry.input === "string"
                  ? entry.input
                  : JSON.stringify(entry.input, null, 2)}
              </pre>
            </div>
          )}

          {/* Inline DiffView for edit / write */}
          {entry.diff && <DiffView diff={entry.diff} defaultExpanded />}

          {/* Tool Output */}
          {rawOutput.length > 0 && (
            <div
              className={`border ${
                failed
                  ? "border-danger/35 bg-danger-soft/30"
                  : "border-line-subtle bg-code-surface"
              }`}
            >
              <div className="flex items-center justify-between border-b border-line-subtle bg-code-header px-2.5 py-0.5 text-[10.5px] font-mono text-ink-muted">
                <span>{failed ? "错误输出" : "输出"}</span>
                <QuietCopyButton text={rawOutput} alwaysVisible />
              </div>
              <pre
                className={`m-0 max-h-72 overflow-auto whitespace-pre-wrap break-words px-2.5 py-1.5 font-mono text-[11.5px] leading-[1.4] ${
                  failed ? "text-danger" : "text-ink-2"
                }`}
              >
                {displayedOutput}
              </pre>
              {isTruncated && (
                <div className="border-t border-line-subtle px-2.5 py-1">
                  <button
                    type="button"
                    onClick={() => setShowFullOutput(true)}
                    className="text-[11px] font-medium text-ink-2 hover:text-ink-strong cursor-pointer"
                  >
                    展开完整输出 ({rawOutput.length} 字符)
                  </button>
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
};

/** Orca-style NativeChatToolRun: collapsed exploration summary opening to an indented rail. */
const ExplorationClusterRow: React.FC<{
  tools: Extract<TimelineEntry, { type: "tool_call" }>[];
}> = ({ tools }) => {
  const [expanded, setExpanded] = useState(false);
  const running = tools.some((t) => t.state === "running" || t.state === "pending");
  const hasError = tools.some((t) => t.is_error || t.state === "failed");
  const summary = useMemo(() => summarizeCluster(tools), [tools]);

  return (
    <div className="min-w-0">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="group/cluster -mx-1.5 flex min-h-[24px] w-[calc(100%+0.75rem)] items-center justify-between gap-2 px-1.5 py-[2px] text-left text-[12.5px] leading-[18px] text-ink-muted hover:bg-hover/70 hover:text-ink transition-colors cursor-pointer select-none"
      >
        <div className="flex min-w-0 flex-1 items-center gap-2">
          {running ? (
            <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-working" />
          ) : hasError ? (
            <XCircle className="h-3.5 w-3.5 shrink-0 text-danger" />
          ) : (
            <Layers className="h-3.5 w-3.5 shrink-0 text-ink-muted" strokeWidth={1.7} />
          )}
          <span className="shrink-0 font-medium text-ink-2">
            {running ? "正在探索" : "已完成探索"}
          </span>
          <span className="truncate font-mono text-[11.5px] text-ink-muted">
            {summary}
          </span>
          <ChevronRight
            className={`h-3 w-3 shrink-0 text-ink-muted transition-transform ${
              expanded
                ? "rotate-90 opacity-100"
                : "opacity-0 group-hover/cluster:opacity-100"
            }`}
          />
        </div>
        <span className="shrink-0 font-mono text-[10.5px] text-ink-muted tabular-nums">
          {tools.length} 步
        </span>
      </button>

      {expanded && (
        <div className="mt-0.5 mb-1.5 ml-[6.5px] space-y-0.5 border-l border-line pl-3.5">
          {tools.map((t) => (
            <ToolCallRow key={t.id} entry={t} />
          ))}
        </div>
      )}
    </div>
  );
};

/** Docked architectural approval bar right above the composer. */
const InlineApprovalCard: React.FC<{
  approval: PendingApproval;
  onReply: (reply: ApprovalReply) => void;
}> = ({ approval, onReply }) => {
  const [replyText, setReplyText] = useState(approval.prefill ?? "");

  return (
    <div className="shrink-0 px-5 pt-2">
      <div className="mx-auto w-full max-w-[68rem]">
        <div className="mux-composer-shadow border border-line border-l-2 border-l-question bg-elevated p-3.5">
          <div className="flex items-start gap-2.5">
            <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0 text-question" strokeWidth={1.8} />
            <div className="min-w-0 flex-1 space-y-2">
              <div className="flex items-center justify-between gap-2">
                <span className="text-[13px] font-semibold text-ink-strong">
                  {approval.title}
                </span>
                {approval.tool_name && (
                  <span className="border border-line-subtle bg-canvas px-1.5 py-0.5 font-mono text-[10.5px] text-question">
                    {approval.tool_name}
                  </span>
                )}
              </div>

              {approval.message && (
                <p className="text-[12.5px] text-ink-2 select-text">{approval.message}</p>
              )}

              {approval.command_or_input && (
                <pre className="m-0 max-h-28 overflow-auto border border-line-subtle bg-code-surface px-2.5 py-2 font-mono text-[11.5px] text-ink select-text">
                  {approval.command_or_input}
                </pre>
              )}

              <div className="flex flex-wrap items-center gap-2 pt-0.5">
                {approval.options.length > 0 ? (
                  approval.options.map((opt) => {
                    const isReject =
                      opt.kind?.startsWith("reject") ||
                      opt.option_id.toLowerCase().includes("deny") ||
                      opt.option_id.toLowerCase().includes("reject");
                    return (
                      <button
                        key={opt.option_id}
                        type="button"
                        onClick={() =>
                          onReply({ kind: "selected_option", option_id: opt.option_id })
                        }
                        className={`h-7 px-3 text-[12px] font-medium transition-colors cursor-pointer ${
                          isReject
                            ? "border border-danger/40 bg-danger-soft text-danger hover:bg-danger hover:text-white"
                            : "bg-primary-btn text-primary-btn-fg hover:bg-primary-btn-hover"
                        }`}
                      >
                        {opt.label}
                      </button>
                    );
                  })
                ) : (
                  <>
                    <button
                      type="button"
                      onClick={() => onReply({ kind: "confirm", confirmed: true })}
                      className="h-7 bg-primary-btn px-3 text-[12px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover transition-colors cursor-pointer"
                    >
                      允许
                    </button>
                    <button
                      type="button"
                      onClick={() => onReply({ kind: "confirm", confirmed: false })}
                      className="h-7 border border-danger/40 bg-danger-soft px-3 text-[12px] font-medium text-danger hover:bg-danger hover:text-white transition-colors cursor-pointer"
                    >
                      拒绝
                    </button>
                  </>
                )}
                <button
                  type="button"
                  onClick={() => onReply({ kind: "cancel" })}
                  className="h-7 border border-line bg-canvas px-2.5 text-[12px] text-ink-2 hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
                >
                  取消
                </button>
              </div>

              {(approval.prefill != null || approval.options.length === 0) && (
                <div className="flex items-center gap-2 pt-0.5">
                  <input
                    type="text"
                    value={replyText}
                    onChange={(e) => setReplyText(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && replyText.trim()) {
                        onReply({ kind: "text", value: replyText.trim() });
                      }
                    }}
                    placeholder="输入自定义回复，按 Enter 提交…"
                    className="h-8 flex-1 border border-line bg-canvas px-2.5 font-mono text-[12px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
                  />
                  <button
                    type="button"
                    disabled={!replyText.trim()}
                    onClick={() => onReply({ kind: "text", value: replyText.trim() })}
                    className="h-8 border border-line bg-canvas px-3 text-[12px] text-ink hover:bg-hover disabled:opacity-45 cursor-pointer"
                  >
                    提交
                  </button>
                </div>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export const ChatPane: React.FC<ChatPaneProps> = ({
  agentId,
  agentTitle,
  theme,
  timeline,
  onSend,
  onAbort,
  onDeleteQueuedMessage,
  onApprove,
  onConfigure,
}) => {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [atBottom, setAtBottom] = useState(true);
  const [draft, setDraft] = useState("");

  const meta = timeline?.meta ?? defaultChatMeta();
  const entries = timeline?.entries ?? [];
  const approvals = timeline?.pending_approvals ?? [];
  const displayItems = useMemo(() => buildDisplayItems(entries), [entries]);

  const fallbackStats = useMemo<SessionStats | null>(() => {
    if (entries.length === 0) return null;
    const acc: SessionStats = {
      turns: 0,
      steps: 0,
      tool_calls: 0,
      input_tokens: 0,
      output_tokens: 0,
      cache_read_tokens: 0,
      cache_write_tokens: 0,
      reasoning_tokens: 0,
      total_tokens: 0,
      total_cost_usd: null,
      llm_ms: 0,
      tool_ms: 0,
      ttft_ms: 0,
      ttft_steps: 0,
    };
    let costSum = 0;
    for (const entry of entries) {
      if (entry.type === "user_message") {
        acc.turns += 1;
      } else if (entry.type === "tool_call") {
        acc.tool_calls += 1;
        if (entry.duration_ms) acc.tool_ms += entry.duration_ms;
      } else if (entry.type === "assistant_text" && !entry.streaming) {
        acc.steps += 1;
        if (entry.usage) {
          acc.input_tokens += entry.usage.input_tokens;
          acc.output_tokens += entry.usage.output_tokens;
          acc.cache_read_tokens += entry.usage.cache_read_tokens;
          acc.cache_write_tokens += entry.usage.cache_write_tokens;
          acc.reasoning_tokens += entry.usage.reasoning_tokens;
          acc.total_tokens += entry.usage.total_tokens;
          if (entry.usage.cost_usd) costSum += entry.usage.cost_usd;
          if (entry.usage.duration_ms) acc.llm_ms += entry.usage.duration_ms;
          if (entry.usage.ttft_ms != null) {
            acc.ttft_ms += entry.usage.ttft_ms;
            acc.ttft_steps += 1;
          }
        }
      }
    }
    if (costSum > 0) acc.total_cost_usd = costSum;
    return acc.turns > 0 || acc.steps > 0 || acc.total_tokens > 0 ? acc : null;
  }, [entries]);

  useEffect(() => {
    setAtBottom(true);
    requestAnimationFrame(() => {
      const el = scrollRef.current;
      if (el) {
        el.scrollTop = el.scrollHeight;
        el.focus({ preventScroll: true });
      }
    });
  }, [agentId]);

  const handlePaneKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "PageDown" && e.key !== "PageUp") return;
    const target = e.target as HTMLElement | null;
    // Let overflowing multiline inputs (like a tall composer textarea) scroll themselves first.
    if (target && (target.tagName === "TEXTAREA" || target.tagName === "INPUT")) {
      const canScrollDown = target.scrollTop + target.clientHeight < target.scrollHeight - 1;
      const canScrollUp = target.scrollTop > 1;
      if (
        target.scrollHeight > target.clientHeight + 2 &&
        ((e.key === "PageDown" && canScrollDown) || (e.key === "PageUp" && canScrollUp))
      ) {
        return;
      }
    }
    const el = scrollRef.current;
    if (!el) return;
    e.preventDefault();
    const delta = Math.max(120, Math.round(el.clientHeight * 0.85));
    el.scrollBy({ top: e.key === "PageDown" ? delta : -delta, behavior: "smooth" });
  };

  const lastEntryFingerprint = useMemo(() => {
    const last = entries[entries.length - 1];
    if (!last) return "empty";
    if (last.type === "assistant_text" || last.type === "thinking") {
      return `${last.id}:${last.text.length}:${last.streaming}`;
    }
    if (last.type === "tool_call") {
      return `${last.id}:${last.state}:${last.output?.length ?? 0}`;
    }
    return `${last.id}:${entries.length}`;
  }, [entries]);

  useEffect(() => {
    if (!atBottom) return;
    const el = scrollRef.current;
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
  }, [atBottom, displayItems.length, lastEntryFingerprint]);

  const handleScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 72;
    if (nearBottom !== atBottom) setAtBottom(nearBottom);
  };

  const jumpToBottom = () => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
    setAtBottom(true);
  };

  const lastEntry = entries[entries.length - 1];
  const showWorkingTail =
    meta.is_streaming &&
    (!lastEntry ||
      (lastEntry.type !== "thinking" &&
        !(lastEntry.type === "assistant_text" && lastEntry.streaming)));
  const lastDisplayItem = displayItems[displayItems.length - 1];
  const workingTailSpacing = !lastDisplayItem
    ? ""
    : lastDisplayItem.kind === "single" && lastDisplayItem.entry.type === "user_message"
      ? "mt-2.5"
      : isExecutionStep(lastDisplayItem)
        ? "mt-0.5"
        : "mt-2";

  return (
    <div
      onKeyDown={handlePaneKeyDown}
      className="relative flex h-full w-full min-h-0 min-w-0 flex-col overflow-hidden bg-canvas"
    >
      {/* Scroll Viewport Wrapper */}
      <div className="relative min-h-0 min-w-0 flex-1">
        <div
          ref={scrollRef}
          tabIndex={-1}
          onScroll={handleScroll}
          className="h-full overflow-y-auto overflow-x-hidden px-5 pt-5 pb-5 focus:outline-none"
        >
          <div className="mx-auto flex w-full min-w-0 max-w-[68rem] flex-col">
            {displayItems.length === 0 && !showWorkingTail ? (
              <div className="mx-auto mt-12 w-full max-w-[44rem]">
                <div className="flex items-center gap-2.5">
                  <span className="flex h-6 w-6 items-center justify-center bg-primary-btn font-mono text-[11px] font-bold text-primary-btn-fg">
                    M
                  </span>
                  <h2 className="text-[18px] font-semibold tracking-tight text-ink-strong">
                    {agentTitle || "准备就绪"}
                  </h2>
                </div>
                <p className="mt-2.5 text-[13.5px] leading-[1.55] text-ink-2">
                  输入任务或指令开始协作；支持流式思考、模型与思考深度切换、代码高亮、内联文件 Diff 与粘贴截图。
                </p>

                <div className="mt-7 border-t border-line-subtle pt-4">
                  <div className="mb-2 text-[11px] text-ink-muted">快速开始</div>
                  <div className="divide-y divide-line-subtle border border-line-subtle">
                    {STARTER_PROMPTS.map((prompt) => (
                      <button
                        key={prompt}
                        type="button"
                        onClick={() => setDraft(prompt)}
                        className="group flex w-full items-center justify-between gap-3 bg-canvas px-3.5 py-2.5 text-left text-[13px] text-ink-2 hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
                      >
                        <span className="truncate">{prompt}</span>
                        <ArrowUpRight className="h-3.5 w-3.5 shrink-0 text-ink-muted opacity-0 group-hover:opacity-100 transition-opacity" />
                      </button>
                    ))}
                  </div>
                </div>
              </div>
            ) : (
              displayItems.map((item, idx) => {
                const spacing = itemTopSpacingClass(displayItems[idx - 1], item);

                if (item.kind === "exploration_cluster") {
                  return (
                    <div key={item.key} className={`min-w-0 ${spacing}`}>
                      <ExplorationClusterRow tools={item.tools} />
                    </div>
                  );
                }

                const entry = item.entry;
                switch (entry.type) {
                  case "user_message":
                    return (
                      <div
                        key={entry.id}
                        className={`group flex items-end justify-end gap-1.5 ${spacing}`}
                      >
                        {entry.text && (
                          <div className="select-none pb-0.5">
                            <QuietCopyButton text={entry.text} />
                          </div>
                        )}
                        <div className="max-w-[82%] border border-line-subtle bg-user-surface px-3 py-1.5 text-[13.5px] leading-[1.45] text-ink-strong select-text">
                          {entry.text && (
                            <div className="whitespace-pre-wrap break-words">{entry.text}</div>
                          )}
                          {entry.attachments.length > 0 && (
                            <div
                              className={`flex flex-wrap gap-2 ${entry.text ? "mt-2" : ""}`}
                            >
                              {entry.attachments.map((att, attIdx) => (
                                <div
                                  key={attIdx}
                                  className="border border-line-subtle bg-canvas p-1 font-mono text-[11px] text-ink-2"
                                >
                                  {att.data_b64 ? (
                                    <img
                                      src={`data:${att.mime_type};base64,${att.data_b64}`}
                                      alt={att.name}
                                      className="max-h-40 object-contain"
                                    />
                                  ) : (
                                    <span>{att.path || att.name}</span>
                                  )}
                                </div>
                              ))}
                            </div>
                          )}
                        </div>
                      </div>
                    );

                  case "thinking":
                    return (
                      <div key={entry.id} className={`min-w-0 ${spacing}`}>
                        <ReasoningRow entry={entry} />
                      </div>
                    );

                  case "assistant_text":
                    return (
                      <div key={entry.id} className={`group min-w-0 ${spacing}`}>
                        <MarkdownView
                          text={entry.text}
                          theme={theme}
                          streaming={entry.streaming}
                        />
                        {!entry.streaming && entry.text.trim().length > 0 && (
                          <div className="mt-1 flex min-h-[24px] flex-wrap items-center gap-1.5 select-none">
                            <QuietCopyButton text={entry.text} />
                            {entry.model && (
                              <span className="font-mono text-[10.5px] text-ink-muted">
                                {entry.model}
                              </span>
                            )}
                            <MessageUsageFooter
                              usage={entry.usage}
                              model={entry.model}
                            />
                          </div>
                        )}
                      </div>
                    );

                  case "tool_call":
                    return (
                      <div key={entry.id} className={`min-w-0 ${spacing}`}>
                        <ToolCallRow entry={entry} />
                      </div>
                    );

                  case "notice": {
                    const tone =
                      entry.level === "error"
                        ? "border-l-danger bg-danger-soft text-danger"
                        : entry.level === "warning"
                          ? "border-l-working bg-working-soft text-working"
                          : "border-l-line-strong bg-elevated text-ink-2";
                    return (
                      <div
                        key={entry.id}
                        className={`flex items-start gap-2 border-l-2 px-3 py-2 text-[12.5px] select-text ${tone} ${spacing}`}
                      >
                        <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                        <span className="whitespace-pre-wrap">{entry.text}</span>
                      </div>
                    );
                  }
                }
              })
            )}

            {/* Live Turn Activity Line */}
            {showWorkingTail && (
              <div
                className={`flex min-h-[24px] items-center gap-2 py-[2px] text-[12.5px] leading-[18px] text-ink-muted select-none ${workingTailSpacing}`}
              >
                <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-working" />
                <span className="leading-[18px]">处理中…</span>
              </div>
            )}
          </div>
        </div>

        {/* Scroll-to-Latest Button */}
        {!atBottom && displayItems.length > 0 && (
          <button
            type="button"
            onClick={jumpToBottom}
            className="mux-popover-shadow absolute bottom-3 left-1/2 -translate-x-1/2 z-10 flex h-7 items-center gap-1.5 border border-line-strong bg-elevated px-3 text-[11.5px] text-ink hover:bg-hover transition-colors cursor-pointer"
          >
            <ArrowDown className="h-3 w-3" />
            <span>回到最新</span>
          </button>
        )}
      </div>

      {/* Docked Inline Approval Card */}
      {approvals[0] && (
        <InlineApprovalCard
          approval={approvals[0]}
          onReply={(reply) => onApprove(approvals[0].id, reply)}
        />
      )}

      {/* Error Footer Banner */}
      {meta.last_error && (
        <div className="shrink-0 px-5 pt-2">
          <div className="mx-auto w-full max-w-[68rem] border-l-2 border-danger bg-danger-soft px-3 py-1.5 text-[12px] text-danger select-text">
            {meta.last_error}
          </div>
        </div>
      )}

      {/* Orca-style Queued Steer / Follow-Up Strip */}
      {meta.queued_messages && meta.queued_messages.length > 0 && (
        <div className="shrink-0 px-5 pt-2">
          <div className="mx-auto w-full max-w-[68rem] border border-line bg-elevated">
            <div className="flex items-center justify-between gap-2 border-b border-line-subtle bg-sidebar/60 px-3 py-1 text-[11px] text-ink-muted select-none">
              <div className="flex items-center gap-2">
                <span>待发送队列 ({meta.queued_messages.length})</span>
                <span className="font-mono text-[10.5px]">
                  {meta.queued_messages.some((m) => m.behavior === "steer")
                    ? "将在下个工具边界插队注入"
                    : "当前回合结束后按序发送"}
                </span>
              </div>
              {onDeleteQueuedMessage && (
                <button
                  type="button"
                  onClick={() => onDeleteQueuedMessage(undefined)}
                  title="清空全部待发送消息"
                  className="border border-line-subtle bg-canvas px-1.5 py-[1px] font-mono text-[10px] text-ink-muted hover:border-danger/40 hover:bg-danger-soft hover:text-danger transition-colors cursor-pointer"
                >
                  清空
                </button>
              )}
            </div>
            <div className="divide-y divide-line-subtle">
              {meta.queued_messages.map((item) => (
                <div
                  key={item.id}
                  className="flex items-center gap-2 px-3 py-1.5 text-[12.5px] leading-[1.4]"
                >
                  <span
                    className={`shrink-0 border px-1 py-[1px] font-mono text-[10px] ${
                      item.behavior === "steer"
                        ? "border-working/40 bg-working-soft text-working"
                        : "border-line bg-canvas text-ink-muted"
                    }`}
                  >
                    {item.behavior === "steer" ? "插队" : "排队"}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-ink select-text">
                    {item.text || `[${item.attachments_count ?? 1} 个附件]`}
                  </span>
                  {onDeleteQueuedMessage && (
                    <button
                      type="button"
                      onClick={() => onDeleteQueuedMessage(item.id)}
                      title="删除此条排队消息"
                      aria-label="删除此条排队消息"
                      className="flex h-5 w-5 shrink-0 items-center justify-center text-ink-muted hover:bg-danger-soft hover:text-danger transition-colors cursor-pointer"
                    >
                      <X className="h-3 w-3" />
                    </button>
                  )}
                </div>
              ))}
            </div>
          </div>
        </div>
      )}

      {/* DeepSeek Harness-style Pie Goal / Tasks / Subagents / Background Jobs Dock */}
      <ChatSessionDock
        meta={meta}
        onRunCommand={(cmd) => void onSend(cmd, [])}
      />

      {/* Docked Composer */}
      <ChatComposer
        meta={meta}
        fallbackStats={fallbackStats}
        draft={draft}
        onDraftChange={setDraft}
        onSend={onSend}
        onAbort={onAbort}
        onConfigure={onConfigure}
      />
    </div>
  );
};
