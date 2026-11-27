import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import {
  ArrowUp,
  Brain,
  Check,
  ChevronDown,
  Image as ImageIcon,
  Search,
  Slash,
  Square,
  X,
} from "lucide-react";
import type {
  ChatAttachment,
  ChatSessionMeta,
  SessionStats,
  StreamingBehavior,
} from "../shared/protocol";
import {
  applySlashCompletion,
  buildSlashCatalog,
  deriveSlashAutocomplete,
  editReplacesTriggerToken,
  type LocalSlashAction,
  type SlashPickerItem,
} from "../shared/slashCommands";
import { ChatSlashMenu } from "./ChatSlashMenu";
import { SessionStatsStrip } from "./ChatStatsBar";

interface ChatComposerProps {
  meta: ChatSessionMeta;
  fallbackStats?: SessionStats | null;
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
}

const DEFAULT_THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh"];

const THINKING_LABELS: Record<string, string> = {
  off: "关闭 (off)",
  minimal: "极低 (minimal)",
  low: "低 (low)",
  medium: "中 (medium)",
  high: "高 (high)",
  xhigh: "极高 (xhigh)",
  max: "最大 (max)",
};

const MAX_UPLOAD_IMAGE_B64_BYTES = 520 * 1024;

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

function loadImageElement(file: File): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => {
      URL.revokeObjectURL(url);
      resolve(img);
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("decode image failed"));
    };
    img.src = url;
  });
}

async function encodeImageFileForWire(
  file: File,
): Promise<{ mimeType: string; dataB64: string }> {
  const rawB64 = await fileToBase64(file);
  const rawMime = file.type || "image/png";
  if (rawB64.length <= MAX_UPLOAD_IMAGE_B64_BYTES) {
    return { mimeType: rawMime, dataB64: rawB64 };
  }
  try {
    const img = await loadImageElement(file);
    const attempts: Array<[number, number]> = [
      [1600, 0.84],
      [1200, 0.78],
      [880, 0.7],
      [640, 0.62],
    ];
    let best = rawB64;
    for (const [maxDim, quality] of attempts) {
      const longest = Math.max(img.naturalWidth || img.width, img.naturalHeight || img.height, 1);
      const scale = Math.min(1, maxDim / longest);
      const w = Math.max(1, Math.round((img.naturalWidth || img.width) * scale));
      const h = Math.max(1, Math.round((img.naturalHeight || img.height) * scale));
      const canvas = document.createElement("canvas");
      canvas.width = w;
      canvas.height = h;
      const ctx = canvas.getContext("2d");
      if (!ctx) break;
      ctx.fillStyle = "#ffffff";
      ctx.fillRect(0, 0, w, h);
      ctx.drawImage(img, 0, 0, w, h);
      const dataUrl = canvas.toDataURL("image/jpeg", quality);
      const comma = dataUrl.indexOf(",");
      const b64 = comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl;
      best = b64;
      if (b64.length <= MAX_UPLOAD_IMAGE_B64_BYTES) {
        return { mimeType: "image/jpeg", dataB64: b64 };
      }
    }
    return { mimeType: "image/jpeg", dataB64: best };
  } catch {
    return { mimeType: rawMime, dataB64: rawB64 };
  }
}

export const ChatComposer: React.FC<ChatComposerProps> = ({
  meta,
  fallbackStats,
  draft,
  onDraftChange,
  onSend,
  onAbort,
  onConfigure,
}) => {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const modelSearchRef = useRef<HTMLInputElement | null>(null);
  const prevDraftRef = useRef<string>(draft);
  const slashListboxId = `mux-slash-picker-${useId().replaceAll(":", "")}`;

  const [attachments, setAttachments] = useState<ChatAttachment[]>([]);
  const [sending, setSending] = useState(false);
  const [caret, setCaret] = useState<number>(draft.length);
  const [dismissedTriggerKey, setDismissedTriggerKey] = useState<string | null>(null);
  const [activeSlashIndex, setActiveSlashIndex] = useState<number>(0);
  const [modelOpen, setModelOpen] = useState(false);
  const [modelQuery, setModelQuery] = useState("");
  const [thinkingOpen, setThinkingOpen] = useState(false);

  const isReadOnly = !meta.supports_chat_input;
  const canSubmit = !isReadOnly && !sending && (draft.trim().length > 0 || attachments.length > 0);

  const thinkingOptions = useMemo(() => {
    if (meta.available_thinking_levels.length > 0) {
      return meta.available_thinking_levels;
    }
    if (meta.driver === "pi_rpc" || meta.thinking_level) {
      return DEFAULT_THINKING_LEVELS;
    }
    return [];
  }, [meta.available_thinking_levels, meta.driver, meta.thinking_level]);

  const filteredModels = useMemo(() => {
    const q = modelQuery.trim().toLowerCase();
    if (!q) return meta.available_models;
    return meta.available_models.filter(
      (m) =>
        m.id.toLowerCase().includes(q) ||
        m.name.toLowerCase().includes(q) ||
        m.provider.toLowerCase().includes(q),
    );
  }, [meta.available_models, modelQuery]);

  // Orca-ported slash command & skill catalog + autocomplete derivation
  const slashCatalog = useMemo(
    () => buildSlashCatalog(meta.available_commands, meta.driver),
    [meta.available_commands, meta.driver],
  );

  const slashAutocomplete = useMemo(
    () =>
      isReadOnly
        ? ({ open: false } as const)
        : deriveSlashAutocomplete(draft, caret, slashCatalog, dismissedTriggerKey),
    [caret, dismissedTriggerKey, draft, isReadOnly, slashCatalog],
  );

  // Sync preferred active row whenever query/trigger changes (Orca pattern)
  const slashQueryKey = slashAutocomplete.open
    ? `${slashAutocomplete.triggerKey}:${slashAutocomplete.query}`
    : "";
  useEffect(() => {
    if (slashAutocomplete.open) {
      setActiveSlashIndex(slashAutocomplete.preferredIndex);
      setModelOpen(false);
      setThinkingOpen(false);
    }
  }, [slashQueryKey, slashAutocomplete.open, slashAutocomplete.open ? slashAutocomplete.preferredIndex : 0]);

  // Keep caret in bounds when external draft changes occur (e.g. switching session)
  useEffect(() => {
    if (caret > draft.length) {
      setCaret(draft.length);
    }
    if (!draft) {
      setDismissedTriggerKey(null);
    }
    prevDraftRef.current = draft;
  }, [draft, caret]);

  // Auto-resize textarea
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(200, Math.max(44, el.scrollHeight))}px`;
  }, [draft]);

  // Focus model search when opened
  useEffect(() => {
    if (modelOpen) {
      setModelQuery("");
      requestAnimationFrame(() => modelSearchRef.current?.focus());
    }
  }, [modelOpen]);

  // Close popovers on outside click
  useEffect(() => {
    const onPointerDown = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        if (slashAutocomplete.open) {
          setDismissedTriggerKey(slashAutocomplete.triggerKey);
        }
        setModelOpen(false);
        setThinkingOpen(false);
      }
    };
    window.addEventListener("mousedown", onPointerDown);
    return () => window.removeEventListener("mousedown", onPointerDown);
  }, [slashAutocomplete]);

  const setDraftAndCaret = useCallback(
    (nextDraft: string, nextCaret: number) => {
      prevDraftRef.current = nextDraft;
      onDraftChange(nextDraft);
      setCaret(nextCaret);
      setDismissedTriggerKey(null);
      requestAnimationFrame(() => {
        const el = textareaRef.current;
        if (el) {
          el.focus();
          el.setSelectionRange(nextCaret, nextCaret);
        }
      });
    },
    [onDraftChange],
  );

  const runLocalAction = useCallback(
    (action: LocalSlashAction) => {
      onDraftChange("");
      setCaret(0);
      setDismissedTriggerKey(null);
      switch (action) {
        case "clear":
          setAttachments([]);
          break;
        case "abort":
          onAbort();
          break;
        case "model":
          setThinkingOpen(false);
          if (meta.available_models.length > 0) {
            setModelOpen(true);
          }
          break;
        case "thinking":
          setModelOpen(false);
          if (thinkingOptions.length > 0) {
            setThinkingOpen(true);
          }
          break;
      }
    },
    [meta.available_models.length, onAbort, onDraftChange, thinkingOptions.length],
  );

  const addImageFiles = async (files: FileList | File[]) => {
    const list = Array.from(files).filter((f) => f.type.startsWith("image/"));
    if (list.length === 0) return;
    const next: ChatAttachment[] = [];
    for (const file of list) {
      const { mimeType, dataB64 } = await encodeImageFileForWire(file);
      const ext = mimeType.split("/")[1] || "png";
      next.push({
        name: file.name || `image-${Date.now()}.${ext}`,
        mime_type: mimeType,
        data_b64: dataB64,
      });
    }
    setAttachments((prev) => [...prev, ...next]);
  };

  const defaultStreamingBehavior: StreamingBehavior =
    meta.driver === "pi_rpc" ? "steer" : "follow_up";

  const dispatchText = useCallback(
    async (text: string, atts: ChatAttachment[], behaviorOverride?: StreamingBehavior) => {
      setSending(true);
      try {
        const behavior: StreamingBehavior | undefined = meta.is_streaming
          ? (behaviorOverride ?? defaultStreamingBehavior)
          : undefined;
        await onSend(text, atts, behavior);
      } finally {
        setSending(false);
        textareaRef.current?.focus();
      }
    },
    [defaultStreamingBehavior, meta.is_streaming, onSend],
  );

  const handleChooseSlashItem = useCallback(
    (item: SlashPickerItem, mode: "dispatch" | "complete") => {
      if (mode === "dispatch" && item.localAction) {
        runLocalAction(item.localAction);
        return;
      }
      if (mode === "dispatch" && item.kind === "command") {
        onDraftChange("");
        setCaret(0);
        setAttachments([]);
        setDismissedTriggerKey(null);
        void dispatchText(item.token, []);
        return;
      }
      const el = textareaRef.current;
      const liveText = el ? el.value : draft;
      const liveCaret = el?.selectionStart ?? caret;
      const completed = applySlashCompletion(liveText, liveCaret, item);
      setDraftAndCaret(completed.draft, completed.caret);
    },
    [caret, dispatchText, draft, onDraftChange, runLocalAction, setDraftAndCaret],
  );

  const handleSubmit = async (behaviorOverride?: StreamingBehavior) => {
    if (!canSubmit) return;
    const trimmed = draft.trim();
    if (trimmed === "/clear") {
      runLocalAction("clear");
      return;
    }
    if (trimmed === "/abort" || trimmed === "/stop") {
      runLocalAction("abort");
      return;
    }
    if (trimmed === "/model" || trimmed === "/models") {
      runLocalAction("model");
      return;
    }
    if (trimmed === "/thinking" || trimmed === "/effort") {
      runLocalAction("thinking");
      return;
    }

    const toSend = draft;
    const atts = attachments;
    onDraftChange("");
    setCaret(0);
    setAttachments([]);
    setDismissedTriggerKey(null);
    await dispatchText(toSend, atts, behaviorOverride);
  };

  const handlePaste = async (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const items = Array.from(e.clipboardData?.items ?? []);
    const imageItems = items.filter((item) => item.type.startsWith("image/"));
    if (imageItems.length === 0) return;
    e.preventDefault();
    const files: File[] = [];
    for (const item of imageItems) {
      const f = item.getAsFile();
      if (f) files.push(f);
    }
    await addImageFiles(files);
  };

  const handleTextareaChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const nextValue = e.target.value;
    const nextCaret = e.target.selectionStart ?? nextValue.length;
    if (
      dismissedTriggerKey &&
      editReplacesTriggerToken(prevDraftRef.current, nextValue, dismissedTriggerKey)
    ) {
      setDismissedTriggerKey(null);
    }
    prevDraftRef.current = nextValue;
    onDraftChange(nextValue);
    setCaret(nextCaret);
  };

  const syncCaretFromTextarea = (e: React.SyntheticEvent<HTMLTextAreaElement>) => {
    const el = e.currentTarget;
    setCaret(el.selectionStart ?? el.value.length);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Orca IME guard: never steal Enter/Tab while IME composition is active
    if (e.nativeEvent.isComposing || e.keyCode === 229) {
      if (e.key === "Enter") {
        e.preventDefault();
      }
      return;
    }

    // Orca slash picker keyboard routing
    if (slashAutocomplete.open) {
      const count = slashAutocomplete.items.length;
      if (e.key === "ArrowDown" && count > 0) {
        e.preventDefault();
        setActiveSlashIndex((idx) => (idx + 1) % count);
        return;
      }
      if (e.key === "ArrowUp" && count > 0) {
        e.preventDefault();
        setActiveSlashIndex((idx) => (idx - 1 + count) % count);
        return;
      }
      if ((e.key === "Enter" || e.key === "Tab") && !e.shiftKey && count > 0) {
        e.preventDefault();
        const picked =
          slashAutocomplete.items[Math.min(activeSlashIndex, count - 1)] ??
          slashAutocomplete.items[0];
        const shouldDispatch =
          e.key === "Enter" &&
          slashAutocomplete.dispatchable &&
          picked.kind === "command" &&
          !picked.argumentHint;
        handleChooseSlashItem(picked, shouldDispatch ? "dispatch" : "complete");
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setDismissedTriggerKey(slashAutocomplete.triggerKey);
        return;
      }
    }

    if (e.key === "Escape") {
      if (modelOpen || thinkingOpen) {
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
      const override: StreamingBehavior | undefined =
        e.ctrlKey || e.metaKey
          ? defaultStreamingBehavior === "steer"
            ? "follow_up"
            : "steer"
          : undefined;
      void handleSubmit(override);
    }
  };

  const handleToggleSlashButton = () => {
    setModelOpen(false);
    setThinkingOpen(false);
    if (slashAutocomplete.open) {
      setDismissedTriggerKey(slashAutocomplete.triggerKey);
      textareaRef.current?.focus();
      return;
    }
    const el = textareaRef.current;
    const liveDraft = el ? el.value : draft;
    const liveCaret = el?.selectionStart ?? liveDraft.length;
    if (!liveDraft.trim()) {
      setDraftAndCaret("/", 1);
      return;
    }
    const before = liveDraft.slice(0, liveCaret);
    const after = liveDraft.slice(liveCaret);
    // If caret is already on a dismissed slash token, simply re-open it
    if (/(?:^|\s)\/\S*$/.test(before)) {
      setDismissedTriggerKey(null);
      el?.focus();
      return;
    }
    const needsSpace = before.length > 0 && !/\s$/.test(before);
    const inserted = `${needsSpace ? " " : ""}/`;
    const nextDraft = `${before}${inserted}${after}`;
    setDraftAndCaret(nextDraft, before.length + inserted.length);
  };

  if (isReadOnly) {
    return (
      <div className="shrink-0 px-5 pb-4 pt-2">
        <div className="mx-auto w-full max-w-[68rem]">
          <div className="border border-line border-l-2 border-l-working bg-elevated px-3.5 py-2 text-[12.5px] text-ink-2 mux-composer-shadow">
            当前会话处于只读视图。
          </div>
          <SessionStatsStrip meta={meta} fallbackStats={fallbackStats} />
        </div>
      </div>
    );
  }

  const hasModelDisplay =
    Boolean(meta.current_model?.name || meta.current_model?.id) ||
    meta.available_models.length > 0;

  return (
    <div className="shrink-0 px-5 pb-4 pt-2">
      <div ref={containerRef} className="relative mx-auto w-full max-w-[68rem]">
        <input
          ref={fileInputRef}
          type="file"
          accept="image/*"
          multiple
          className="hidden"
          onChange={(e) => {
            if (e.target.files) void addImageFiles(e.target.files);
            e.target.value = "";
          }}
        />

        {/* Orca-ported Slash Command & Skill Autocomplete Menu */}
        {slashAutocomplete.open && (
          <ChatSlashMenu
            autocomplete={slashAutocomplete}
            activeIndex={activeSlashIndex}
            listboxId={slashListboxId}
            onChoose={handleChooseSlashItem}
            onHover={setActiveSlashIndex}
          />
        )}

        {/* Quiet Elevated Composer Box */}
        <div className="mux-composer-shadow border border-line bg-elevated focus-within:border-line-strong transition-colors">
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
                    className="flex h-4 w-4 items-center justify-center text-ink-muted hover:text-danger cursor-pointer"
                  >
                    <X className="h-3 w-3" />
                  </button>
                </div>
              ))}
            </div>
          )}

          {/* Input Field */}
          <textarea
            ref={textareaRef}
            rows={2}
            value={draft}
            role="combobox"
            aria-expanded={slashAutocomplete.open}
            aria-controls={slashAutocomplete.open ? slashListboxId : undefined}
            aria-activedescendant={
              slashAutocomplete.open && slashAutocomplete.items.length > 0
                ? `${slashListboxId}-option-${Math.min(activeSlashIndex, slashAutocomplete.items.length - 1)}`
                : undefined
            }
            onChange={handleTextareaChange}
            onSelect={syncCaretFromTextarea}
            onClick={syncCaretFromTextarea}
            onKeyUp={syncCaretFromTextarea}
            onPaste={(e) => void handlePaste(e)}
            onKeyDown={handleKeyDown}
            placeholder={
              meta.is_streaming
                ? "当前正在回复… Enter 加入待发送队列，Ctrl+Enter 立即插队引导"
                : "输入任务或问题…（输入 / 唤起命令与技能，Shift+Enter 换行，支持粘贴截图）"
            }
            className="block w-full resize-none bg-transparent px-3.5 pt-2.5 pb-1.5 text-[13.5px] leading-[1.45] text-ink-strong placeholder:text-ink-muted focus:outline-none select-text"
          />

          {/* Orca-style Bottom Control Strip */}
          <div className="flex items-center justify-between gap-2 px-2.5 pb-2 pt-0.5">
            <div className="flex min-w-0 flex-1 items-center gap-1">
              {/* Image upload trigger */}
              <button
                type="button"
                onClick={() => fileInputRef.current?.click()}
                title="添加图片附件"
                className="flex h-7 w-7 shrink-0 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
              >
                <ImageIcon className="h-3.5 w-3.5" strokeWidth={1.85} />
              </button>

              {/* Slash command trigger */}
              <button
                type="button"
                onPointerDown={(e) => e.preventDefault()}
                onClick={handleToggleSlashButton}
                title="命令与技能 (/)"
                className={`flex h-7 w-7 shrink-0 items-center justify-center transition-colors cursor-pointer ${
                  slashAutocomplete.open
                    ? "bg-active text-ink-strong"
                    : "text-ink-muted hover:bg-hover hover:text-ink-strong"
                }`}
              >
                <Slash className="h-3.5 w-3.5" strokeWidth={1.85} />
              </button>

              {/* Orca-style Searchable Model Picker */}
              {hasModelDisplay && (
                <div className="relative min-w-0">
                  <button
                    type="button"
                    disabled={meta.available_models.length === 0}
                    onClick={() => {
                      setModelOpen((v) => !v);
                      setThinkingOpen(false);
                      if (slashAutocomplete.open) {
                        setDismissedTriggerKey(slashAutocomplete.triggerKey);
                      }
                    }}
                    title="切换模型 (/model)"
                    className={`flex h-7 max-w-full items-center gap-1 border px-2 font-mono text-[11.5px] transition-colors disabled:opacity-75 disabled:hover:bg-transparent disabled:cursor-default cursor-pointer ${
                      modelOpen
                        ? "border-line-strong bg-active text-ink-strong"
                        : "border-transparent text-ink-2 hover:border-line-subtle hover:bg-hover hover:text-ink-strong"
                    }`}
                  >
                    <span className="max-w-[240px] truncate">
                      {meta.current_model?.name || meta.current_model?.id || "选择模型"}
                    </span>
                    {meta.available_models.length > 0 && (
                      <ChevronDown className="h-3 w-3 shrink-0 text-ink-muted" />
                    )}
                  </button>
                  {modelOpen && meta.available_models.length > 0 && (
                    <div className="mux-popover-shadow absolute bottom-full left-0 mb-1.5 flex w-80 max-w-[85vw] flex-col border border-line-strong bg-elevated z-30">
                      {meta.available_models.length > 6 && (
                        <div className="flex items-center gap-1.5 border-b border-line-subtle px-2.5 py-1.5">
                          <Search className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
                          <input
                            ref={modelSearchRef}
                            type="text"
                            value={modelQuery}
                            onChange={(e) => setModelQuery(e.target.value)}
                            placeholder="搜索模型或 Provider…"
                            className="w-full bg-transparent text-[12px] text-ink-strong placeholder:text-ink-muted focus:outline-none select-text"
                          />
                        </div>
                      )}
                      <div className="max-h-64 overflow-y-auto py-1">
                        {filteredModels.length === 0 ? (
                          <div className="px-3 py-3 text-center text-[11.5px] text-ink-muted">
                            没有匹配的模型
                          </div>
                        ) : (
                          filteredModels.map((m) => {
                            const isCurrent =
                              meta.current_model?.id === m.id &&
                              (!meta.current_model.provider ||
                                !m.provider ||
                                meta.current_model.provider === m.provider);
                            return (
                              <button
                                key={`${m.provider}/${m.id}`}
                                type="button"
                                onClick={() => {
                                  setModelOpen(false);
                                  onConfigure({
                                    provider: m.provider || undefined,
                                    model_id: m.id,
                                  });
                                }}
                                className={`flex w-full items-center justify-between gap-2 px-2.5 py-1.5 text-left font-mono text-[11.5px] transition-colors cursor-pointer ${
                                  isCurrent ? "bg-active/70" : "hover:bg-hover"
                                }`}
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
                          })
                        )}
                      </div>
                    </div>
                  )}
                </div>
              )}

              {/* Orca-style Thinking Level Picker */}
              {thinkingOptions.length > 0 && (
                <div className="relative shrink-0">
                  <button
                    type="button"
                    onClick={() => {
                      setThinkingOpen((v) => !v);
                      setModelOpen(false);
                      if (slashAutocomplete.open) {
                        setDismissedTriggerKey(slashAutocomplete.triggerKey);
                      }
                    }}
                    title="切换思考深度 (/thinking)"
                    className={`flex h-7 items-center gap-1 border px-2 font-mono text-[11.5px] transition-colors cursor-pointer ${
                      thinkingOpen
                        ? "border-line-strong bg-active text-ink-strong"
                        : "border-transparent text-ink-2 hover:border-line-subtle hover:bg-hover hover:text-ink-strong"
                    }`}
                  >
                    <Brain className="h-3 w-3 text-ink-muted" strokeWidth={1.75} />
                    <span>{meta.thinking_level ?? "medium"}</span>
                    <ChevronDown className="h-3 w-3 text-ink-muted" />
                  </button>
                  {thinkingOpen && (
                    <div className="mux-popover-shadow absolute bottom-full left-0 mb-1.5 w-44 border border-line-strong bg-elevated py-1 z-30">
                      <div className="border-b border-line-subtle px-2.5 py-1 text-[10.5px] text-ink-muted">
                        思考强度 (Thinking)
                      </div>
                      {thinkingOptions.map((lvl) => {
                        const isCurrent = meta.thinking_level === lvl;
                        return (
                          <button
                            key={lvl}
                            type="button"
                            onClick={() => {
                              setThinkingOpen(false);
                              onConfigure({ thinking_level: lvl });
                            }}
                            className={`flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[11.5px] text-ink transition-colors cursor-pointer ${
                              isCurrent ? "bg-active/70" : "hover:bg-hover"
                            }`}
                          >
                            <span className="w-3 shrink-0">
                              {isCurrent && <Check className="h-3 w-3 text-ink-strong" />}
                            </span>
                            <span
                              className={`font-mono ${
                                isCurrent ? "font-medium text-ink-strong" : ""
                              }`}
                            >
                              {THINKING_LABELS[lvl] ?? lvl}
                            </span>
                          </button>
                        );
                      })}
                    </div>
                  )}
                </div>
              )}
            </div>

            {/* Right cluster: Submit/Stop square */}
            <div className="flex shrink-0 items-center gap-2">
              {meta.is_streaming && !draft.trim() ? (
                <button
                  type="button"
                  onClick={onAbort}
                  title="停止生成 (Esc)"
                  className="flex h-7 w-7 shrink-0 items-center justify-center border border-danger/40 bg-danger-soft text-danger hover:bg-danger hover:text-white transition-colors cursor-pointer"
                >
                  <Square className="h-3 w-3 fill-current" />
                </button>
              ) : (
                <button
                  type="button"
                  disabled={!canSubmit}
                  onClick={() => void handleSubmit()}
                  title={
                    meta.is_streaming
                      ? defaultStreamingBehavior === "steer"
                        ? "插队引导发送 (Enter)"
                        : "排队发送 (Enter)"
                      : "发送 (Enter)"
                  }
                  className="flex h-7 w-7 shrink-0 items-center justify-center bg-primary-btn text-primary-btn-fg hover:bg-primary-btn-hover disabled:bg-hover disabled:text-ink-faint disabled:cursor-not-allowed transition-colors cursor-pointer"
                >
                  <ArrowUp className="h-3.5 w-3.5" strokeWidth={2.2} />
                </button>
              )}
            </div>
          </div>
        </div>

        {/* DeepSeek Harness-style Session Total Stats & Context Occupancy Bar */}
        <SessionStatsStrip meta={meta} fallbackStats={fallbackStats} />
      </div>
    </div>
  );
};
