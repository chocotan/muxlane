import React, { useEffect, useRef, useState } from "react";
import {
  ArrowUp,
  Brain,
  Check,
  ChevronDown,
  Image as ImageIcon,
  Slash,
  Square,
  Terminal as TerminalIcon,
  X,
} from "lucide-react";
import type {
  ChatAttachment,
  ChatSessionMeta,
  StreamingBehavior,
} from "../shared/protocol";
import { formatTokens } from "../shared/timeline";

interface ChatComposerProps {
  meta: ChatSessionMeta;
  draft: string;
  onDraftChange: (next: string) => void;
  onSend: (
    text: string,
    attachments: ChatAttachment[],
    behavior?: StreamingBehavior,
  ) => Promise<void>;
  onAbort: () => void;
  onConfigure: (opts: {
    provider?: string;
    model_id?: string;
    thinking_level?: string;
  }) => void;
  onOpenTerminal: () => void;
  onTakeOverNative: () => void;
}

const SLASH_COMMANDS = [
  { cmd: "/compact", desc: "压缩当前上下文历史" },
  { cmd: "/clear", desc: "清空输入框草稿" },
  { cmd: "/terminal", desc: "切换到交互式终端视图" },
  { cmd: "/abort", desc: "停止当前 Agent 回合" },
];

function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const res = typeof reader.result === "string" ? reader.result : "";
      const comma = res.indexOf(",");
      resolve(comma >= 0 ? res.slice(comma + 1) : res);
    };
    reader.onerror = () => reject(reader.error ?? new Error("read image failed"));
    reader.readAsDataURL(file);
  });
}

export const ChatComposer: React.FC<ChatComposerProps> = ({
  meta,
  draft,
  onDraftChange,
  onSend,
  onAbort,
  onConfigure,
  onOpenTerminal,
  onTakeOverNative,
}) => {
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const [attachments, setAttachments] = useState<ChatAttachment[]>([]);
  const [sending, setSending] = useState(false);
  const [slashOpen, setSlashOpen] = useState(false);
  const [modelOpen, setModelOpen] = useState(false);
  const [thinkingOpen, setThinkingOpen] = useState(false);

  // Auto-grow textarea smoothly up to 220px
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 220)}px`;
  }, [draft]);

  const isPtyBridge = meta.driver === "terminal_pty";
  const isReadOnly = !meta.supports_chat_input;
  const canSubmit = !isReadOnly && !sending && (draft.trim().length > 0 || attachments.length > 0);

  const handleSubmit = async (steer = false) => {
    if (!canSubmit) return;
    const text = draft.trim();
    if (text === "/clear") {
      onDraftChange("");
      setAttachments([]);
      return;
    }
    if (text === "/abort") {
      onDraftChange("");
      onAbort();
      return;
    }
    if (text === "/terminal") {
      onDraftChange("");
      onOpenTerminal();
      return;
    }
    const atts = attachments.slice();
    setSending(true);
    try {
      const behavior: StreamingBehavior | undefined = meta.is_streaming
        ? steer
          ? "steer"
          : "follow_up"
        : undefined;
      onDraftChange("");
      setAttachments([]);
      await onSend(text, atts, behavior);
    } catch {
      onDraftChange(text);
      setAttachments(atts);
    } finally {
      setSending(false);
      textareaRef.current?.focus();
    }
  };

  const handlePaste = async (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const items = Array.from(e.clipboardData.items);
    const images = items.filter((item) => item.type.startsWith("image/"));
    if (images.length === 0) return;
    e.preventDefault();
    const added: ChatAttachment[] = [];
    for (const img of images) {
      const file = img.getAsFile();
      if (!file) continue;
      try {
        const b64 = await fileToBase64(file);
        const ext = file.type.split("/")[1] || "png";
        added.push({
          name: file.name || `paste-${attachments.length + added.length + 1}.${ext}`,
          mime_type: file.type || "image/png",
          data_b64: b64,
        });
      } catch {
        // ignore broken image item
      }
    }
    if (added.length > 0) {
      setAttachments((prev) => [...prev, ...added].slice(0, 8));
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.nativeEvent.isComposing) return;
    if (e.key === "Escape") {
      if (slashOpen || modelOpen || thinkingOpen) {
        setSlashOpen(false);
        setModelOpen(false);
        setThinkingOpen(false);
        return;
      }
      if (meta.is_streaming) {
        e.preventDefault();
        onAbort();
      }
      return;
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      const steer = e.ctrlKey || e.metaKey;
      void handleSubmit(steer);
    }
  };

  if (isReadOnly) {
    return (
      <div className="mux-composer-scrim shrink-0 px-4 pb-4 pt-3">
        <div className="mx-auto w-full max-w-[46rem]">
          <div className="flex items-center justify-between gap-3 border border-line border-l-2 border-l-working bg-elevated px-3.5 py-2.5 text-[12.5px] mux-composer-shadow">
            <div className="min-w-0 text-ink-2">
              {isPtyBridge && meta.has_pty
                ? "终端前台已退回裸 Shell，对话输入已自动锁定以防命令误注入。重启 Agent 后自动解锁。"
                : "当前处于只读镜像模式，终端会话正在运行中。"}
            </div>
            <div className="flex shrink-0 items-center gap-2">
              {meta.has_pty && (
                <button
                  type="button"
                  onClick={onOpenTerminal}
                  className="flex h-7 items-center gap-1.5 border border-line bg-canvas px-2.5 text-[12px] text-ink hover:bg-hover transition-colors cursor-pointer"
                >
                  <TerminalIcon className="h-3.5 w-3.5" />
                  <span>查看终端</span>
                </button>
              )}
              {meta.supports_terminal_switch && (
                <button
                  type="button"
                  onClick={onTakeOverNative}
                  className="flex h-7 items-center bg-primary-btn px-2.5 text-[12px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover transition-colors cursor-pointer"
                >
                  接管原生对话
                </button>
              )}
            </div>
          </div>
        </div>
      </div>
    );
  }

  const contextPct =
    meta.context_used_tokens && meta.context_window_tokens && meta.context_window_tokens > 0
      ? Math.min(100, Math.round((meta.context_used_tokens / meta.context_window_tokens) * 100))
      : null;

  return (
    <div className="mux-composer-scrim shrink-0 px-4 pb-4 pt-2">
      <div className="relative mx-auto w-full max-w-[46rem]">
        {/* Slash Command Popover */}
        {slashOpen && (
          <div className="mux-popover-shadow absolute bottom-full left-0 right-0 mb-1.5 border border-line-strong bg-elevated z-20">
            <div className="border-b border-line-subtle px-3 py-1.5 font-mono text-[10.5px] uppercase tracking-wider text-ink-muted">
              Commands
            </div>
            <div className="py-1">
              {SLASH_COMMANDS.map((item) => (
                <button
                  key={item.cmd}
                  type="button"
                  onClick={() => {
                    setSlashOpen(false);
                    if (item.cmd === "/clear") {
                      onDraftChange("");
                      setAttachments([]);
                    } else if (item.cmd === "/abort") {
                      onAbort();
                    } else if (item.cmd === "/terminal") {
                      onOpenTerminal();
                    } else {
                      void onSend(item.cmd, []);
                    }
                  }}
                  className="flex w-full items-center justify-between px-3 py-2 text-left text-[12.5px] hover:bg-hover transition-colors cursor-pointer"
                >
                  <span className="font-mono font-medium text-ink-strong">{item.cmd}</span>
                  <span className="text-ink-muted">{item.desc}</span>
                </button>
              ))}
            </div>
          </div>
        )}

        {/* Orca + DeepSeek Harness Quiet Elevated Composer Box */}
        <div className="mux-composer-shadow border border-line bg-elevated focus-within:border-line-strong transition-colors">
          {/* Subtle streaming hint strip */}
          {meta.is_streaming && (
            <div className="flex items-center justify-between border-b border-line-subtle bg-hover/50 px-3.5 py-1.5 text-[11.5px] text-ink-2">
              <div className="flex items-center gap-2 min-w-0">
                <span className="h-1.5 w-1.5 shrink-0 bg-working animate-pulse" />
                <span className="truncate">
                  {isPtyBridge
                    ? "Agent 运行中 · 按 Esc 中断，发送将排入终端队列"
                    : "Agent 运行中 · Enter 追加排队，Ctrl+Enter 立即引导 (Steer)"}
                </span>
              </div>
              <button
                type="button"
                onClick={onAbort}
                className="flex shrink-0 items-center gap-1 px-2 py-0.5 text-[11px] font-medium text-danger hover:bg-danger-soft transition-colors cursor-pointer"
              >
                <Square className="h-2.5 w-2.5 fill-current" />
                <span>停止</span>
              </button>
            </div>
          )}

          {/* Pasted image attachment chips */}
          {attachments.length > 0 && (
            <div className="flex flex-wrap gap-2 border-b border-line-subtle px-3.5 py-2">
              {attachments.map((att, idx) => (
                <div
                  key={`${att.name}-${idx}`}
                  className="flex items-center gap-2 border border-line-subtle bg-canvas p-1 pr-2 text-[11.5px]"
                >
                  {att.data_b64 ? (
                    <img
                      src={`data:${att.mime_type};base64,${att.data_b64}`}
                      alt={att.name}
                      className="h-7 w-7 object-cover border border-line-subtle"
                    />
                  ) : (
                    <ImageIcon className="h-4 w-4 text-ink-2 ml-1" />
                  )}
                  <span className="max-w-[140px] truncate font-mono text-ink-2">
                    {att.name}
                  </span>
                  <button
                    type="button"
                    onClick={() => setAttachments((prev) => prev.filter((_, i) => i !== idx))}
                    className="text-ink-muted hover:text-danger cursor-pointer"
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                </div>
              ))}
            </div>
          )}

          {/* Spacious Input Field */}
          <textarea
            ref={textareaRef}
            rows={2}
            value={draft}
            onChange={(e) => onDraftChange(e.target.value)}
            onPaste={(e) => void handlePaste(e)}
            onKeyDown={handleKeyDown}
            placeholder="输入任务或问题…（Shift+Enter 换行，支持粘贴截图与 / 快捷命令）"
            className="block w-full resize-none bg-transparent px-3.5 pt-3 pb-2 text-[14px] leading-[1.6] text-ink-strong placeholder:text-ink-muted focus:outline-none select-text"
          />

          {/* Borderless Quiet Control Strip */}
          <div className="flex flex-wrap items-center justify-between gap-2 px-2.5 pb-2 pt-0.5">
            <div className="flex flex-wrap items-center gap-1">
              {/* Slash command trigger */}
              <button
                type="button"
                onClick={() => {
                  setSlashOpen((v) => !v);
                  setModelOpen(false);
                  setThinkingOpen(false);
                }}
                title="快捷命令 (/)"
                className={`flex h-7 w-7 items-center justify-center transition-colors cursor-pointer ${
                  slashOpen
                    ? "bg-active text-ink-strong"
                    : "text-ink-muted hover:bg-hover hover:text-ink-strong"
                }`}
              >
                <Slash className="h-3.5 w-3.5" strokeWidth={1.85} />
              </button>

              {/* Model Picker (ghost borderless button) */}
              <div className="relative">
                <button
                  type="button"
                  disabled={meta.available_models.length === 0}
                  onClick={() => {
                    setModelOpen((v) => !v);
                    setThinkingOpen(false);
                    setSlashOpen(false);
                  }}
                  className="flex h-7 items-center gap-1 px-2 font-mono text-[11.5px] text-ink-2 hover:bg-hover hover:text-ink-strong disabled:opacity-60 disabled:hover:bg-transparent disabled:cursor-default transition-colors cursor-pointer"
                >
                  <span className="max-w-[190px] truncate">
                    {meta.current_model?.name || meta.current_model?.id || "default"}
                  </span>
                  {meta.available_models.length > 0 && (
                    <ChevronDown className="h-3 w-3 text-ink-muted" />
                  )}
                </button>
                {modelOpen && meta.available_models.length > 0 && (
                  <div className="mux-popover-shadow absolute bottom-full left-0 mb-1.5 max-h-64 w-72 overflow-y-auto border border-line-strong bg-elevated py-1 z-30">
                    {meta.available_models.map((m) => {
                      const isCurrent = meta.current_model?.id === m.id;
                      return (
                        <button
                          key={`${m.provider}/${m.id}`}
                          type="button"
                          onClick={() => {
                            setModelOpen(false);
                            onConfigure({ provider: m.provider || undefined, model_id: m.id });
                          }}
                          className="flex w-full items-center justify-between gap-2 px-2.5 py-1.5 text-left font-mono text-[11.5px] hover:bg-hover transition-colors cursor-pointer"
                        >
                          <div className="flex min-w-0 items-center gap-2">
                            <span className="w-3 shrink-0">
                              {isCurrent && <Check className="h-3 w-3 text-ink-strong" />}
                            </span>
                            <span
                              className={`truncate ${
                                isCurrent ? "font-medium text-ink-strong" : "text-ink"
                              }`}
                            >
                              {m.name || m.id}
                            </span>
                          </div>
                          <span className="shrink-0 text-[10px] text-ink-muted">
                            {m.provider}
                          </span>
                        </button>
                      );
                    })}
                  </div>
                )}
              </div>

              {/* Thinking effort selector */}
              {(meta.thinking_level || meta.available_thinking_levels.length > 0) && (
                <div className="relative">
                  <button
                    type="button"
                    disabled={meta.available_thinking_levels.length === 0}
                    onClick={() => {
                      setThinkingOpen((v) => !v);
                      setModelOpen(false);
                      setSlashOpen(false);
                    }}
                    className="flex h-7 items-center gap-1 px-2 font-mono text-[11.5px] text-ink-2 hover:bg-hover hover:text-ink-strong disabled:opacity-60 disabled:hover:bg-transparent transition-colors cursor-pointer"
                  >
                    <Brain className="h-3 w-3 text-ink-muted" strokeWidth={1.75} />
                    <span>{meta.thinking_level ?? "default"}</span>
                    {meta.available_thinking_levels.length > 0 && (
                      <ChevronDown className="h-3 w-3 text-ink-muted" />
                    )}
                  </button>
                  {thinkingOpen && meta.available_thinking_levels.length > 0 && (
                    <div className="mux-popover-shadow absolute bottom-full left-0 mb-1.5 w-36 border border-line-strong bg-elevated py-1 z-30">
                      {meta.available_thinking_levels.map((lvl) => {
                        const isCurrent = meta.thinking_level === lvl;
                        return (
                          <button
                            key={lvl}
                            type="button"
                            onClick={() => {
                              setThinkingOpen(false);
                              onConfigure({ thinking_level: lvl });
                            }}
                            className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left font-mono text-[11.5px] text-ink hover:bg-hover transition-colors cursor-pointer"
                          >
                            <span className="w-3 shrink-0">
                              {isCurrent && <Check className="h-3 w-3 text-ink-strong" />}
                            </span>
                            <span className={isCurrent ? "font-medium text-ink-strong" : ""}>
                              {lvl}
                            </span>
                          </button>
                        );
                      })}
                    </div>
                  )}
                </div>
              )}

              {/* Quiet PTY Sync live indicator */}
              {isPtyBridge && (
                <div className="flex h-7 items-center gap-1.5 px-2 font-mono text-[10.5px] text-ink-muted">
                  <span className="h-1.5 w-1.5 bg-success" />
                  <span>PTY Sync</span>
                </div>
              )}
            </div>

            {/* Right cluster: quiet context meter + crisp monochrome Submit/Stop square */}
            <div className="flex items-center gap-2.5">
              {meta.context_used_tokens != null && (
                <div className="flex items-center gap-2 font-mono text-[10.5px] text-ink-muted tabular-nums">
                  <span>
                    {formatTokens(meta.context_used_tokens)}
                    {meta.context_window_tokens
                      ? ` / ${formatTokens(meta.context_window_tokens)}`
                      : ""}
                  </span>
                  {contextPct != null && (
                    <div className="h-[3px] w-10 bg-hover overflow-hidden">
                      <div
                        className={`h-full ${
                          contextPct > 85
                            ? "bg-danger"
                            : contextPct > 65
                              ? "bg-working"
                              : "bg-ink-2"
                        }`}
                        style={{ width: `${contextPct}%` }}
                      />
                    </div>
                  )}
                </div>
              )}

              {meta.total_cost_usd != null && meta.total_cost_usd > 0.0001 && (
                <span className="font-mono text-[10.5px] text-ink-muted tabular-nums">
                  ${meta.total_cost_usd.toFixed(3)}
                </span>
              )}

              {meta.is_streaming && !draft.trim() ? (
                <button
                  type="button"
                  onClick={onAbort}
                  title="停止生成 (Esc)"
                  className="flex h-7 w-7 items-center justify-center border border-danger/40 bg-danger-soft text-danger hover:bg-danger hover:text-white transition-colors cursor-pointer"
                >
                  <Square className="h-3 w-3 fill-current" />
                </button>
              ) : (
                <button
                  type="button"
                  disabled={!canSubmit}
                  onClick={() => void handleSubmit(false)}
                  title={meta.is_streaming ? "排队发送 (Enter)" : "发送 (Enter)"}
                  className="flex h-7 w-7 items-center justify-center bg-primary-btn text-primary-btn-fg hover:bg-primary-btn-hover disabled:bg-hover disabled:text-ink-faint disabled:cursor-not-allowed transition-colors cursor-pointer"
                >
                  <ArrowUp className="h-3.5 w-3.5" strokeWidth={2.2} />
                </button>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};
