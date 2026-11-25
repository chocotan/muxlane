import { diffLines } from "diff";
import type {
  AgentPreset,
  AgentType,
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
    !entry.diff
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
