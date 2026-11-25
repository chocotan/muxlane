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
  XCircle,
} from "lucide-react";
import type {
  ApprovalReply,
  ChatAttachment,
  PendingApproval,
  StreamingBehavior,
  TimelineEntry,
} from "../shared/protocol";
import {
  buildDisplayItems,
  defaultChatMeta,
  summarizeToolInput,
  type ChatTimelineState,
} from "../shared/timeline";
import type { AppTheme } from "../state/useTheme";
import { ChatComposer } from "./ChatComposer";
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
  onApprove: (approvalId: string, reply: ApprovalReply) => void;
  onConfigure: (opts: {
    provider?: string;
    model_id?: string;
    thinking_level?: string;
  }) => void;
  onOpenTerminal: () => void;
  onTakeOverNative: () => void;
}

const STARTER_PROMPTS = [
  "先梳理这个仓库的核心架构、关键模块和入口文件",
  "运行当前测试套件，定位任何失败或警告原因",
  "检查当前未提交的 git diff，排查潜在 bug 或边界情况",
];

/** Orca-style quiet single-line reasoning disclosure with subtle left rail when open. */
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
      ? `Thought for ${(entry.duration_ms / 1000).toFixed(1)}s`
      : entry.streaming
        ? "Thinking…"
        : "Thought";

  return (
    <div className="min-w-0">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="group/reason flex min-h-6 w-full items-center gap-1.5 py-0.5 text-left text-[13px] text-ink-muted hover:text-ink transition-colors cursor-pointer select-none"
      >
        {entry.streaming ? (
          <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-working" />
        ) : (
          <Brain className="h-3.5 w-3.5 shrink-0 text-ink-muted" strokeWidth={1.7} />
        )}
        <span className="truncate">{durationLabel}</span>
        <ChevronRight
          className={`h-3.5 w-3.5 shrink-0 text-ink-muted transition-transform ${
            expanded
              ? "rotate-90 opacity-100"
              : "opacity-0 group-hover/reason:opacity-100"
          }`}
        />
      </button>

      {expanded && (
        <div className="mt-1.5 ml-[6px] max-h-72 overflow-y-auto border-l border-line pl-3.5 text-[12.5px] leading-[1.6] text-ink-muted select-text">
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

/** Orca + DeepSeek Harness quiet inline tool trajectory row (borderless when collapsed). */
const ToolCallRow: React.FC<{
  entry: Extract<TimelineEntry, { type: "tool_call" }>;
}> = ({ entry }) => {
  const [expanded, setExpanded] = useState(Boolean(entry.is_error || entry.diff));
  const summary = useMemo(() => summarizeToolInput(entry), [entry]);

  return (
    <div className="min-w-0">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className={`group/tool flex min-h-7 w-full items-center justify-between gap-3 px-1.5 -mx-1.5 text-left transition-colors hover:bg-hover/70 cursor-pointer select-none ${
          entry.is_error ? "text-danger" : "text-ink-2 hover:text-ink-strong"
        }`}
      >
        <div className="flex min-w-0 flex-1 items-center gap-2">
          {toolRowIcon(entry)}
          <span className="shrink-0 font-mono text-[12px] font-medium text-ink">
            {entry.name}
          </span>
          {summary && (
            <span className="truncate font-mono text-[11.5px] text-ink-muted">
              {summary}
            </span>
          )}
          <ChevronRight
            className={`h-3 w-3 shrink-0 text-ink-muted transition-transform ${
              expanded
                ? "rotate-90 opacity-100"
                : "opacity-0 group-hover/tool:opacity-100"
            }`}
          />
        </div>

        <div className="flex shrink-0 items-center gap-2 font-mono text-[10.5px] text-ink-muted tabular-nums">
          {entry.exit_code != null && (
            <span className={entry.exit_code === 0 ? "text-ink-muted" : "text-danger"}>
              exit {entry.exit_code}
            </span>
          )}
          {entry.duration_ms != null && <span>{entry.duration_ms}ms</span>}
        </div>
      </button>

      {expanded && (
        <div className="mt-1.5 ml-[6px] space-y-2 border-l border-line pl-3.5 select-text">
          {entry.input != null && !entry.diff && (
            <div>
              <div className="mb-1 font-mono text-[10px] uppercase tracking-wider text-ink-muted">
                Input
              </div>
              <pre className="m-0 max-h-48 overflow-auto border border-line-subtle bg-code-surface px-2.5 py-2 font-mono text-[11.5px] leading-[1.5] text-ink-2">
                {typeof entry.input === "string"
                  ? entry.input
                  : JSON.stringify(entry.input, null, 2)}
              </pre>
            </div>
          )}

          {entry.diff && <DiffView diff={entry.diff} defaultExpanded />}

          {entry.output && (
            <div>
              <div className="mb-1 font-mono text-[10px] uppercase tracking-wider text-ink-muted">
                Output
              </div>
              <pre className="m-0 max-h-64 overflow-auto border border-line-subtle bg-code-surface px-2.5 py-2 font-mono text-[11.5px] leading-[1.5] text-ink-2 whitespace-pre-wrap">
                {entry.output}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
};

/** Orca-style collapsed exploration summary line opening to a quiet vertical rail. */
const ExplorationClusterRow: React.FC<{
  tools: Extract<TimelineEntry, { type: "tool_call" }>[];
}> = ({ tools }) => {
  const [expanded, setExpanded] = useState(false);
  const running = tools.some((t) => t.state === "running" || t.state === "pending");
  const hasError = tools.some((t) => t.is_error || t.state === "failed");

  return (
    <div className="min-w-0">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="group/cluster flex min-h-7 w-full items-center gap-2 px-1.5 -mx-1.5 text-left text-[12.5px] text-ink-muted hover:bg-hover/70 hover:text-ink transition-colors cursor-pointer select-none"
      >
        {running ? (
          <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-working" />
        ) : hasError ? (
          <XCircle className="h-3.5 w-3.5 shrink-0 text-danger" />
        ) : (
          <Layers className="h-3.5 w-3.5 shrink-0 text-ink-muted" strokeWidth={1.7} />
        )}
        <span className="shrink-0 text-ink-2">
          {running ? `正在探索 (${tools.length})` : `探索了 ${tools.length} 个步骤`}
        </span>
        <span className="truncate font-mono text-[11px] text-ink-muted">
          {tools
            .map((t) => t.name)
            .slice(0, 5)
            .join(" · ")}
          {tools.length > 5 ? " …" : ""}
        </span>
        <ChevronRight
          className={`h-3.5 w-3.5 shrink-0 text-ink-muted transition-transform ${
            expanded
              ? "rotate-90 opacity-100"
              : "opacity-0 group-hover/cluster:opacity-100"
          }`}
        />
      </button>

      {expanded && (
        <div className="mt-1 ml-[6px] space-y-0.5 border-l border-line pl-3.5">
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
    <div className="shrink-0 px-4 pt-2">
      <div className="mx-auto w-full max-w-[46rem]">
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

const QuietCopyButton: React.FC<{ text: string; alwaysVisible?: boolean }> = ({
  text,
  alwaysVisible = false,
}) => {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      onClick={() => {
        void navigator.clipboard.writeText(text);
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      }}
      title="复制全文"
      className={`flex h-6 items-center gap-1 px-1.5 font-mono text-[10.5px] text-ink-muted hover:bg-hover hover:text-ink-strong transition-all cursor-pointer ${
        alwaysVisible ? "opacity-100" : "opacity-0 group-hover:opacity-100"
      }`}
    >
      {copied ? (
        <Check className="h-3 w-3 text-success" />
      ) : (
        <Copy className="h-3 w-3" />
      )}
      <span>{copied ? "已复制" : "复制"}</span>
    </button>
  );
};

export const ChatPane: React.FC<ChatPaneProps> = ({
  agentId,
  agentTitle,
  theme,
  timeline,
  onSend,
  onAbort,
  onApprove,
  onConfigure,
  onOpenTerminal,
  onTakeOverNative,
}) => {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [atBottom, setAtBottom] = useState(true);
  const [draft, setDraft] = useState("");

  const meta = timeline?.meta ?? defaultChatMeta();
  const entries = timeline?.entries ?? [];
  const approvals = timeline?.pending_approvals ?? [];
  const displayItems = useMemo(() => buildDisplayItems(entries), [entries]);

  useEffect(() => {
    setAtBottom(true);
    requestAnimationFrame(() => {
      const el = scrollRef.current;
      if (el) el.scrollTop = el.scrollHeight;
    });
  }, [agentId]);

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

  // Orca live tail row: show "Working…" only when turn is streaming and tail isn't already streaming text/thinking
  const lastEntry = entries[entries.length - 1];
  const showWorkingTail =
    meta.is_streaming &&
    (!lastEntry ||
      (lastEntry.type !== "thinking" &&
        !(lastEntry.type === "assistant_text" && lastEntry.streaming)));

  return (
    <div className="relative flex h-full min-h-0 flex-col bg-canvas">
      {/* Main Serene Scrollable Transcript Viewport */}
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className="flex-1 overflow-y-auto px-5 pt-8 pb-6"
      >
        <div className="mx-auto flex w-full max-w-[46rem] flex-col gap-5">
          {displayItems.length === 0 ? (
            /* Editorial Empty Conversation Canvas (DeepSeek Harness + Orca style) */
            <div className="mx-auto mt-14 w-full max-w-[36rem]">
              <div className="flex items-center gap-2.5">
                <span className="flex h-6 w-6 items-center justify-center bg-primary-btn font-mono text-[11px] font-bold text-primary-btn-fg">
                  M
                </span>
                <h2 className="text-[18px] font-semibold tracking-tight text-ink-strong">
                  {agentTitle || "准备就绪"}
                </h2>
              </div>
              <p className="mt-2.5 text-[13.5px] leading-[1.65] text-ink-2">
                {meta.driver === "terminal_pty"
                  ? "当前会话已通过双通道 PTY 桥接同步。此处发送的指令会安全注入终端，终端内的输出、代码改动与工具调用也会实时呈现。"
                  : "已连接结构化原生驱动。支持流式推理展开、代码语法高亮、内联文件 Diff 与截图粘贴。"}
              </p>

              <div className="mt-7 border-t border-line-subtle pt-4">
                <div className="mb-2 font-mono text-[10.5px] uppercase tracking-wider text-ink-muted">
                  快速开始
                </div>
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
              const isLatestItem = idx === displayItems.length - 1;

              if (item.kind === "exploration_cluster") {
                return <ExplorationClusterRow key={item.key} tools={item.tools} />;
              }

              const entry = item.entry;
              switch (entry.type) {
                case "user_message":
                  return (
                    <div key={entry.id} className="group flex flex-col items-end gap-1">
                      <div className="max-w-[82%] border border-line-subtle bg-user-surface px-3.5 py-2.5 text-[14px] leading-[1.62] text-ink-strong select-text">
                        {entry.text && (
                          <div className="whitespace-pre-wrap break-words">{entry.text}</div>
                        )}
                        {entry.attachments.length > 0 && (
                          <div
                            className={`flex flex-wrap gap-2 ${entry.text ? "mt-2.5" : ""}`}
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
                      <div className="flex items-center gap-1 select-none">
                        {entry.text && <QuietCopyButton text={entry.text} />}
                      </div>
                    </div>
                  );

                case "thinking":
                  return <ReasoningRow key={entry.id} entry={entry} />;

                case "assistant_text":
                  return (
                    <div key={entry.id} className="group min-w-0">
                      <MarkdownView
                        text={entry.text}
                        theme={theme}
                        streaming={entry.streaming}
                      />
                      {!entry.streaming && entry.text.trim().length > 0 && (
                        <div className="mt-1.5 flex items-center gap-2 select-none">
                          <QuietCopyButton
                            text={entry.text}
                            alwaysVisible={isLatestItem}
                          />
                          {entry.model && (
                            <span
                              className={`font-mono text-[10.5px] text-ink-muted transition-opacity ${
                                isLatestItem
                                  ? "opacity-75"
                                  : "opacity-0 group-hover:opacity-75"
                              }`}
                            >
                              {entry.model}
                            </span>
                          )}
                        </div>
                      )}
                    </div>
                  );

                case "tool_call":
                  return <ToolCallRow key={entry.id} entry={entry} />;

                case "plan":
                  return (
                    <div
                      key={entry.id}
                      className="border-l-2 border-line-strong bg-elevated/50 px-3.5 py-2.5"
                    >
                      <div className="mb-2 font-mono text-[10.5px] uppercase tracking-wider text-ink-muted">
                        Execution Plan
                      </div>
                      <div className="space-y-1.5">
                        {entry.entries.map((step, sIdx) => (
                          <div key={sIdx} className="flex items-start gap-2.5 text-[13px]">
                            <span
                              className={`mt-[7px] h-1.5 w-1.5 shrink-0 ${
                                step.status === "completed"
                                  ? "bg-success"
                                  : step.status === "in_progress"
                                    ? "bg-working animate-pulse"
                                    : "bg-ink-faint"
                              }`}
                            />
                            <span
                              className={
                                step.status === "completed"
                                  ? "text-ink-muted line-through"
                                  : "text-ink"
                              }
                            >
                              {step.content}
                            </span>
                          </div>
                        ))}
                      </div>
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
                      className={`flex items-start gap-2 border-l-2 px-3 py-2 text-[12.5px] select-text ${tone}`}
                    >
                      <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                      <span className="whitespace-pre-wrap">{entry.text}</span>
                    </div>
                  );
                }
              }
            })
          )}

          {/* Orca Live Turn Activity Line */}
          {showWorkingTail && (
            <div className="flex min-h-6 items-center gap-2 text-[13px] text-ink-muted select-none">
              <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-working" />
              <span>Working…</span>
            </div>
          )}
        </div>
      </div>

      {/* Subtle Scroll-to-Latest Button */}
      {!atBottom && displayItems.length > 0 && (
        <button
          type="button"
          onClick={jumpToBottom}
          className="mux-popover-shadow absolute bottom-36 left-1/2 -translate-x-1/2 z-10 flex h-7 items-center gap-1.5 border border-line-strong bg-elevated px-3 font-mono text-[11px] text-ink hover:bg-hover transition-colors cursor-pointer"
        >
          <ArrowDown className="h-3 w-3" />
          <span>回到最新</span>
        </button>
      )}

      {/* Docked Inline Approval Card */}
      {approvals[0] && (
        <InlineApprovalCard
          approval={approvals[0]}
          onReply={(reply) => onApprove(approvals[0].id, reply)}
        />
      )}

      {/* Error Footer Banner */}
      {meta.last_error && (
        <div className="shrink-0 px-4 pt-2">
          <div className="mx-auto w-full max-w-[46rem] border-l-2 border-danger bg-danger-soft px-3 py-1.5 text-[12px] text-danger select-text">
            {meta.last_error}
          </div>
        </div>
      )}

      {/* Docked Composer */}
      <ChatComposer
        meta={meta}
        draft={draft}
        onDraftChange={setDraft}
        onSend={onSend}
        onAbort={onAbort}
        onConfigure={onConfigure}
        onOpenTerminal={onOpenTerminal}
        onTakeOverNative={onTakeOverNative}
      />
    </div>
  );
};
