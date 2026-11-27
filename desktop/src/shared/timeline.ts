import { diffLines } from "diff";
import type {
  AgentPreset,
  AgentType,
  ApprovalOption,
  ChatDeltaEvent,
  ChatDeltaOp,
  ChatHistoryResult,
  ChatSessionMeta,
  PendingApproval,
  SessionDriverMode,
  TimelineEntry,
  ToolCallKind,
} from "./protocol";

export interface ChatTimelineState {
  agent: string;
  seq: number;
  meta: ChatSessionMeta;
  entries: TimelineEntry[];
  pending_approvals: PendingApproval[];
}

export function defaultChatMeta(): ChatSessionMeta {
  return {
    driver: "terminal_pty",
    supports_chat_input: false,
    supports_terminal_switch: false,
    has_pty: false,
    available_models: [],
    available_thinking_levels: [],
    available_commands: [],
    queued_messages: [],
    tasks: [],
    subagents: [],
    background_jobs: [],
    is_streaming: false,
  };
}

export function timelineFromHistory(history: ChatHistoryResult): ChatTimelineState {
  return {
    agent: history.agent,
    seq: history.seq,
    meta: history.meta ?? defaultChatMeta(),
    entries: history.entries ?? [],
    pending_approvals: history.pending_approvals ?? [],
  };
}

export function applyChatDelta(
  state: ChatTimelineState,
  event: ChatDeltaEvent,
): ChatTimelineState {
  if (state.agent && event.agent !== state.agent) return state;
  if (event.seq <= state.seq) return state;
  const next = applyChatDeltaOp(state, event.delta);
  return { ...next, agent: state.agent || event.agent, seq: event.seq };
}

export function applyChatDeltaOp(
  state: ChatTimelineState,
  op: ChatDeltaOp,
): ChatTimelineState {
  switch (op.op) {
    case "upsert_entry": {
      const idx = state.entries.findIndex((e) => e.id === op.entry.id);
      if (idx >= 0) {
        const nextEntries = state.entries.slice();
        nextEntries[idx] = op.entry;
        return { ...state, entries: nextEntries };
      }
      return { ...state, entries: [...state.entries, op.entry] };
    }
    case "remove_entry": {
      if (!state.entries.some((e) => e.id === op.entry_id)) return state;
      return {
        ...state,
        entries: state.entries.filter((e) => e.id !== op.entry_id),
      };
    }
    case "append_text": {
      const idx = state.entries.findIndex((e) => e.id === op.entry_id);
      if (idx < 0) return state;
      const target = state.entries[idx];
      let updated: TimelineEntry = target;
      if (target.type === "assistant_text" && op.kind === "assistant") {
        updated = { ...target, text: target.text + op.delta, streaming: true };
      } else if (target.type === "thinking" && op.kind === "thinking") {
        updated = { ...target, text: target.text + op.delta, streaming: true };
      } else if (target.type === "tool_call" && op.kind === "tool_output") {
        updated = { ...target, output: (target.output ?? "") + op.delta };
      }
      if (updated === target) return state;
      const nextEntries = state.entries.slice();
      nextEntries[idx] = updated;
      return { ...state, entries: nextEntries };
    }
    case "replace_entry_text": {
      const idx = state.entries.findIndex((e) => e.id === op.entry_id);
      if (idx < 0) return state;
      const target = state.entries[idx];
      let updated: TimelineEntry = target;
      if (target.type === "assistant_text" && op.kind === "assistant") {
        updated = { ...target, text: op.text, streaming: !op.finish };
      } else if (target.type === "thinking" && op.kind === "thinking") {
        const duration =
          op.finish && target.duration_ms == null
            ? Math.max(0, Date.now() - target.created_at_ms)
            : target.duration_ms;
        updated = { ...target, text: op.text, streaming: !op.finish, duration_ms: duration };
      } else if (target.type === "tool_call" && op.kind === "tool_output") {
        updated = { ...target, output: op.text };
      }
      if (updated === target) return state;
      const nextEntries = state.entries.slice();
      nextEntries[idx] = updated;
      return { ...state, entries: nextEntries };
    }
    case "finish_entry": {
      const idx = state.entries.findIndex((e) => e.id === op.entry_id);
      if (idx < 0) return state;
      const target = state.entries[idx];
      let updated: TimelineEntry = target;
      if (target.type === "assistant_text") {
        updated = { ...target, streaming: false };
      } else if (target.type === "thinking") {
        updated = {
          ...target,
          streaming: false,
          duration_ms:
            op.duration_ms ??
            target.duration_ms ??
            Math.max(0, Date.now() - target.created_at_ms),
        };
      }
      if (updated === target) return state;
      const nextEntries = state.entries.slice();
      nextEntries[idx] = updated;
      return { ...state, entries: nextEntries };
    }
    case "set_assistant_usage": {
      const idx = state.entries.findIndex((e) => e.id === op.entry_id);
      if (idx < 0) return state;
      const target = state.entries[idx];
      if (target.type !== "assistant_text") return state;
      const nextEntries = state.entries.slice();
      nextEntries[idx] = { ...target, usage: op.usage };
      return { ...state, entries: nextEntries };
    }
    case "meta_updated":
      return { ...state, meta: op.meta };
    case "approval_requested": {
      const next = state.pending_approvals.filter((a) => a.id !== op.approval.id);
      next.push(op.approval);
      return { ...state, pending_approvals: next };
    }
    case "approval_resolved":
      return {
        ...state,
        pending_approvals: state.pending_approvals.filter((a) => a.id !== op.approval_id),
      };
    case "turn_finished": {
      const nextMeta: ChatSessionMeta = {
        ...state.meta,
        is_streaming: false,
        last_error: op.error ?? state.meta.last_error,
      };
      const nextEntries = state.entries.map((entry): TimelineEntry => {
        if (entry.type === "assistant_text" && entry.streaming) {
          return { ...entry, streaming: false };
        }
        if (entry.type === "thinking" && entry.streaming) {
          return { ...entry, streaming: false };
        }
        if (
          entry.type === "tool_call" &&
          (entry.state === "pending" || entry.state === "running")
        ) {
          return { ...entry, state: "completed" };
        }
        return entry;
      });
      return { ...state, meta: nextMeta, entries: nextEntries };
    }
    case "reset":
      return {
        ...state,
        entries: [],
        pending_approvals: [],
        meta: { ...state.meta, last_error: null },
      };
  }
}

export type DisplayTimelineItem =
  | { kind: "single"; key: string; entry: TimelineEntry }
  | {
      kind: "exploration_cluster";
      key: string;
      tools: Extract<TimelineEntry, { type: "tool_call" }>[];
    };

const EXPLORATION_KINDS: Set<ToolCallKind> = new Set(["read", "search", "fetch", "think"]);

export function isExplorationTool(entry: TimelineEntry): entry is Extract<TimelineEntry, { type: "tool_call" }> {
  return (
    entry.type === "tool_call" &&
    EXPLORATION_KINDS.has(entry.kind) &&
    !entry.is_error &&
    !entry.diff &&
    !(entry.attachments && entry.attachments.length > 0)
  );
}

export function buildDisplayItems(entries: TimelineEntry[]): DisplayTimelineItem[] {
  const out: DisplayTimelineItem[] = [];
  let i = 0;
  while (i < entries.length) {
    const cur = entries[i];
    if (isExplorationTool(cur)) {
      const cluster: Extract<TimelineEntry, { type: "tool_call" }>[] = [];
      while (i < entries.length) {
        const cand = entries[i];
        if (isExplorationTool(cand)) {
          cluster.push(cand);
          i++;
        } else {
          break;
        }
      }
      if (cluster.length >= 2) {
        out.push({
          kind: "exploration_cluster",
          key: `cluster:${cluster[0].id}`,
          tools: cluster,
        });
      } else {
        out.push({ kind: "single", key: cluster[0].id, entry: cluster[0] });
      }
    } else {
      out.push({ kind: "single", key: cur.id, entry: cur });
      i++;
    }
  }
  return out;
}

export function summarizeToolInput(entry: Extract<TimelineEntry, { type: "tool_call" }>): string {
  if (entry.title && entry.title.trim()) return entry.title.trim();
  if (entry.input && typeof entry.input === "object" && !Array.isArray(entry.input)) {
    const rec = entry.input as Record<string, unknown>;
    for (const key of [
      "CommandLine",
      "AbsolutePath",
      "TargetFile",
      "Query",
      "SearchPath",
      "command",
      "cmd",
      "path",
      "file_path",
      "filePath",
      "pattern",
      "query",
      "url",
      "file",
    ]) {
      const v = rec[key];
      if (typeof v === "string" && v.trim()) {
        return v.trim();
      }
    }
    if (Array.isArray(rec.questions)) {
      for (const item of rec.questions) {
        if (item && typeof item === "object" && !Array.isArray(item)) {
          const q = (item as Record<string, unknown>).question;
          if (typeof q === "string" && q.trim()) {
            return q.trim();
          }
        }
      }
    }
  }
  if (entry.locations.length > 0) return entry.locations[0];
  return "";
}

export interface UnifiedDiffRow {
  type: "context" | "add" | "del";
  oldLine: number | null;
  newLine: number | null;
  text: string;
}

export function computeDiffRows(
  oldText: string,
  newText: string,
): { rows: UnifiedDiffRow[]; additions: number; deletions: number } {
  const changes = diffLines(oldText, newText);
  const rows: UnifiedDiffRow[] = [];
  let oldLine = 1;
  let newLine = 1;
  let additions = 0;
  let deletions = 0;

  for (const part of changes) {
    const rawLines = part.value.replace(/\r\n/g, "\n").split("\n");
    if (rawLines.length > 0 && rawLines[rawLines.length - 1] === "") {
      rawLines.pop();
    }
    for (const line of rawLines) {
      if (part.added) {
        additions++;
        rows.push({ type: "add", oldLine: null, newLine: newLine++, text: line });
      } else if (part.removed) {
        deletions++;
        rows.push({ type: "del", oldLine: oldLine++, newLine: null, text: line });
      } else {
        rows.push({ type: "context", oldLine: oldLine++, newLine: newLine++, text: line });
      }
    }
  }
  return { rows, additions, deletions };
}

export function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1_000) return `${Math.round(tokens / 1_000)}k`;
  return String(tokens);
}

export function driverBadgeInfo(
  meta: ChatSessionMeta | undefined,
  agentType: AgentType,
): { label: string; tone: "accent" | "live" | "warn" | "muted" } {
  if (!meta) {
    if (agentType === "shell") return { label: "SHELL", tone: "muted" };
    return { label: "PTY SYNC", tone: "live" };
  }
  switch (meta.driver) {
    case "pi_rpc":
      return { label: "PI RPC", tone: "accent" };
    case "acp":
      return { label: "ACP", tone: "accent" };
    case "terminal_pty":
      if (meta.supports_chat_input) return { label: "PTY SYNC", tone: "live" };
      return { label: "PTY READONLY", tone: "warn" };
  }
}

export function agentTypeLabel(agentType: AgentType): string {
  switch (agentType) {
    case "pi":
      return "Pi";
    case "pie":
      return "Pie";
    case "opencode":
      return "OpenCode";
    case "qwen":
      return "Qwen";
    case "kimi":
      return "Kimi";
    case "agy":
      return "AGY";
    case "gemini":
      return "Gemini";
    case "codex":
      return "Codex";
    case "claude":
      return "Claude";
    case "shell":
      return "Shell";
    default:
      return "Agent";
  }
}

export function orderPresetsPiFirst(presets: AgentPreset[]): {
  agents: AgentPreset[];
  shell: AgentPreset | null;
} {
  const priority: Record<AgentType, number> = {
    pi: 0,
    pie: 1,
    opencode: 2,
    qwen: 3,
    kimi: 4,
    agy: 5,
    gemini: 6,
    codex: 7,
    claude: 8,
    shell: 99,
    unknown: 99,
  };
  const shell = presets.find((p) => p.agent_type === "shell" || p.id === "shell") ?? null;
  const agents = presets
    .filter((p) => p.agent_type !== "shell" && p.id !== "shell")
    .sort((a, b) => (priority[a.agent_type] ?? 50) - (priority[b.agent_type] ?? 50));
  return { agents, shell };
}

export function isLiveBridge(meta: ChatSessionMeta | undefined): boolean {
  return Boolean(meta && meta.driver === "terminal_pty" && meta.supports_chat_input);
}

export function driverSubtitle(driver: SessionDriverMode | undefined, agentType: AgentType): string {
  const base = agentTypeLabel(agentType);
  if (!driver) return base;
  if (driver === "pi_rpc") return `${base} · RPC`;
  if (driver === "acp") return `${base} · ACP`;
  return `${base} · PTY`;
}

export interface ParsedPromptOption {
  optionId: string;
  indexLabel: string | null;
  headline: string;
  description: string | null;
  isCustom: boolean;
  isReject: boolean;
}

export interface ParsedPromptPreview {
  title: string;
  body: string;
}

export interface ParsedApprovalPrompt {
  isQuestion: boolean;
  isInputOnly: boolean;
  isMultiSelectInput: boolean;
  useListLayout: boolean;
  badge: string;
  questionTitle: string;
  hint: string | null;
  previews: ParsedPromptPreview[];
  options: ParsedPromptOption[];
  embeddedOptions: ParsedPromptOption[];
}

const CUSTOM_SENTINEL_RE = /^(type something\.?|other|自定义输入|输入其他.*)$/i;
const LEADING_INDEX_RE = /^(\d+)[.)、]\s+(.+)$/s;
const HEADER_TAG_RE = /^\[([^\[\]\n]{1,32})\]\s*(.*)$/;
const PREVIEW_HEADER_RE = /^---\s*(.+?)\s*---$/;

function splitHeadlineAndDescription(body: string): {
  headline: string;
  description: string | null;
} {
  for (const sep of [" — ", " – "]) {
    const idx = body.indexOf(sep);
    if (idx > 0) {
      const left = body.slice(0, idx).trim();
      const right = body.slice(idx + sep.length).trim();
      if (left && right) {
        return { headline: left, description: right };
      }
    }
  }
  const dashIdx = body.indexOf(" - ");
  if (dashIdx > 0 && dashIdx <= 48) {
    const left = body.slice(0, dashIdx).trim();
    const right = body.slice(dashIdx + 3).trim();
    if (left && right) {
      return { headline: left, description: right };
    }
  }
  return { headline: body, description: null };
}

export function parsePromptOption(
  option: ApprovalOption,
  fallbackIndex: number,
  showFallbackIndex: boolean,
): ParsedPromptOption {
  const rawLabel = option.label.replace(/\r\n/g, "\n").trim();
  const idxMatch = rawLabel.match(LEADING_INDEX_RE);
  const indexLabel = idxMatch
    ? idxMatch[1]
    : showFallbackIndex
      ? String(fallbackIndex + 1)
      : null;
  const body = (idxMatch ? idxMatch[2] : rawLabel).trim();
  const isCustom = CUSTOM_SENTINEL_RE.test(body);
  const kindLower = (option.kind ?? "").toLowerCase();
  const idLower = option.option_id.toLowerCase();
  const isReject =
    kindLower.startsWith("reject") ||
    kindLower.includes("deny") ||
    idLower === "deny" ||
    idLower === "reject" ||
    idLower.includes("reject_") ||
    idLower.includes("deny_");

  if (isCustom) {
    return {
      optionId: option.option_id,
      indexLabel,
      headline: "自定义输入",
      description: "填写其他回答",
      isCustom: true,
      isReject: false,
    };
  }

  const { headline, description } = splitHeadlineAndDescription(body);
  return {
    optionId: option.option_id,
    indexLabel,
    headline: headline || rawLabel,
    description,
    isCustom: false,
    isReject,
  };
}

export function parseApprovalPrompt(approval: PendingApproval): ParsedApprovalPrompt {
  const method = (approval.method || "").toLowerCase();
  const isInputOnly = method === "input" || method === "editor";
  const isQuestion = method === "select" || isInputOnly;

  const normalized = (approval.title || "").replace(/\r\n/g, "\n").trim();
  const blocks = normalized
    .split(/\n{2,}/)
    .map((b) => b.trim())
    .filter(Boolean);

  let badge = isQuestion ? (isInputOnly ? "等待输入" : "单选提问") : "需要确认";
  let questionLines: string[] = [];
  const previews: ParsedPromptPreview[] = [];
  const embeddedOptions: ParsedPromptOption[] = [];
  const hintLines: string[] = [];

  blocks.forEach((block, blockIndex) => {
    const lines = block.split("\n");
    const previewMatch = lines[0]?.trim().match(PREVIEW_HEADER_RE);
    if (previewMatch) {
      previews.push({
        title: previewMatch[1].trim(),
        body: lines.slice(1).join("\n").trim(),
      });
      return;
    }

    if (blockIndex === 0) {
      const firstLine = lines[0]?.trim() ?? "";
      const tagMatch = firstLine.match(HEADER_TAG_RE);
      if (tagMatch && tagMatch[1].trim()) {
        badge = tagMatch[1].trim();
        const restFirst = tagMatch[2].trim();
        if (restFirst) questionLines.push(restFirst);
        for (const extra of lines.slice(1)) {
          if (extra.trim()) questionLines.push(extra.trim());
        }
      } else {
        questionLines = lines.map((l) => l.trim()).filter(Boolean);
      }
      return;
    }

    const nonEmptyLines = lines.map((l) => l.trim()).filter(Boolean);
    const allNumbered =
      nonEmptyLines.length > 0 && nonEmptyLines.every((l) => LEADING_INDEX_RE.test(l));
    if (allNumbered && isInputOnly) {
      nonEmptyLines.forEach((line, i) => {
        embeddedOptions.push(
          parsePromptOption({ option_id: line, label: line, kind: "select" }, i, true),
        );
      });
      return;
    }

    if (isInputOnly && blockIndex === blocks.length - 1) {
      hintLines.push(nonEmptyLines.join("\n"));
    } else {
      questionLines.push(nonEmptyLines.join("\n"));
    }
  });

  const rawHint = hintLines.join("\n").trim() || null;
  const isMultiSelectInput =
    isInputOnly &&
    embeddedOptions.length > 0 &&
    Boolean(rawHint && /(comma-separated|all that apply|numbers|多选)/i.test(rawHint));

  let hint = rawHint;
  if (isMultiSelectInput) {
    hint = "可直接点选上方选项，或输入序号（如 1,3）与自定义内容";
    if (badge === "等待输入") badge = "多选提问";
  } else if (rawHint && /^type your answer:?$/i.test(rawHint)) {
    hint = "请输入你的自定义回答";
  }

  const parsedOptions = approval.options.map((opt, idx) =>
    parsePromptOption(opt, idx, method === "select"),
  );

  const useListLayout =
    method === "select" ||
    parsedOptions.length > 2 ||
    parsedOptions.some((o) => Boolean(o.description) || o.headline.length > 18);

  return {
    isQuestion,
    isInputOnly,
    isMultiSelectInput,
    useListLayout,
    badge,
    questionTitle: questionLines.join("\n").trim() || normalized || "等待回复",
    hint,
    previews,
    options: parsedOptions,
    embeddedOptions,
  };
}

export function toggleMultiSelectNumber(current: string, indexLabel: string): string {
  const trimmed = current.trim();
  if (!trimmed) return indexLabel;
  const tokens = trimmed.split(/[,\s]+/).filter(Boolean);
  if (!tokens.every((t) => /^\d+$/.test(t))) {
    return indexLabel;
  }
  const exists = tokens.includes(indexLabel);
  const next = exists ? tokens.filter((t) => t !== indexLabel) : [...tokens, indexLabel];
  return next.sort((a, b) => Number(a) - Number(b)).join(",");
}

