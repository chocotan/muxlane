/**
 * Orca-ported slash command & skill autocomplete engine for ChatComposer.
 *
 * Mirrors Orca's `native-chat-composer-autocomplete.ts`, `native-chat-picker-items.ts`,
 * `native-chat-picker-triggers.ts`, and `native-chat-slash-commands.ts`:
 * - Leading `/` trigger (`dispatchable: true`) and mid-prompt `/` trigger (`dispatchable: false`)
 * - Grouped catalog: Commands -> Prompts -> Skills
 * - 5-tier ranking (exact -> prefix -> substring -> subsequence -> description) with alias support
 * - Preferred index selection across sections so an exact skill beats a fuzzy command
 * - Caret-preserving completion (`applySlashCompletion`) and dismissal tracking (`editReplacesTriggerToken`)
 */
import type { ChatSlashCommand, SessionDriverMode } from "./protocol";

export type SlashItemKind = "command" | "skill" | "prompt";
export type LocalSlashAction = "model" | "thinking" | "clear" | "abort";

export interface SlashPickerItem {
  id: string;
  kind: SlashItemKind;
  name: string;
  token: string;
  description?: string;
  argumentHint?: string;
  source?: string;
  aliases?: readonly string[];
  localAction?: LocalSlashAction;
  builtin?: boolean;
}

export type SlashAutocompleteState =
  | { open: false }
  | {
      open: true;
      query: string;
      triggerKey: string;
      triggerPosition: number;
      /** True only when `/` is at the very beginning of the composer (Enter can dispatch immediately). */
      dispatchable: boolean;
      items: SlashPickerItem[];
      commands: SlashPickerItem[];
      prompts: SlashPickerItem[];
      skills: SlashPickerItem[];
      preferredIndex: number;
    };

const LEADING_SLASH_TRIGGER = /^\/(\S*)$/;
const MID_PROMPT_SLASH_TRIGGER = /(?:^|\s)\/(\S*)$/;

const COMMAND_RESULT_LIMIT = 40;
const PROMPT_RESULT_LIMIT = 24;
const SKILL_RESULT_LIMIT = 40;

const LOCAL_COMMANDS: readonly SlashPickerItem[] = [
  {
    id: "command:compact",
    kind: "command",
    name: "compact",
    token: "/compact",
    description: "压缩当前上下文历史，减少 token 占用",
    argumentHint: "[自定义指令]",
    source: "builtin",
    builtin: true,
  },
  {
    id: "command:new",
    kind: "command",
    name: "new",
    token: "/new",
    description: "重置上下文并开启新的对话上下文",
    source: "builtin",
    builtin: true,
  },
  {
    id: "command:model",
    kind: "command",
    name: "model",
    token: "/model",
    description: "打开模型选择器并切换当前模型",
    aliases: ["models", "switch"],
    source: "builtin",
    localAction: "model",
    builtin: true,
  },
  {
    id: "command:thinking",
    kind: "command",
    name: "thinking",
    token: "/thinking",
    description: "打开思考等级选择器 (off / low / medium / high / xhigh)",
    aliases: ["effort", "reasoning"],
    source: "builtin",
    localAction: "thinking",
    builtin: true,
  },
  {
    id: "command:clear",
    kind: "command",
    name: "clear",
    token: "/clear",
    description: "清空当前输入框草稿与已附加图片",
    source: "builtin",
    localAction: "clear",
    builtin: true,
  },
  {
    id: "command:abort",
    kind: "command",
    name: "abort",
    token: "/abort",
    description: "停止当前正在执行的 Agent 回合",
    aliases: ["stop", "cancel"],
    source: "builtin",
    localAction: "abort",
    builtin: true,
  },
];

const CURATED_DESCRIPTIONS: Record<string, string> = {
  compact: "压缩当前上下文历史，减少 token 占用",
  new: "重置上下文并开启新的对话上下文",
  clear: "清空当前输入框草稿与已附加图片",
  model: "打开模型选择器并切换当前模型",
  thinking: "打开思考等级选择器",
  abort: "停止当前正在执行的 Agent 回合",
  help: "显示可用命令说明",
  review: "审查当前代码变更",
  context: "查看当前上下文占用情况",
  usage: "查看模型与 provider 的用量统计",
  jobs: "查看后台任务与输出",
  tasks: "查看当前会话任务列表",
  todos: "查看当前会话待办事项",
  agents: "查看或管理子代理任务",
  mcp: "管理 MCP 服务连接与开关",
  retry: "控制自动重试策略",
  goal: "设定、暂停或查看当前会话目标",
};

function sanitizeText(value: string | null | undefined, maxLen: number): string | undefined {
  if (!value) return undefined;
  const cleaned = value.replace(/[\u0000-\u001f\u007f]+/g, " ").trim();
  return cleaned ? cleaned.slice(0, maxLen) : undefined;
}

/**
 * Merge local Muxlane-owned slash commands with real session-reported commands/skills
 * from `pi --mode rpc` (`get_commands`) or ACP (`available_commands_update`).
 */
export function buildSlashCatalog(
  reported: readonly ChatSlashCommand[] | undefined,
  driver: SessionDriverMode,
): SlashPickerItem[] {
  const localOwned = new Set(
    LOCAL_COMMANDS.filter((c) => c.localAction).map((c) => c.name.toLowerCase()),
  );
  const items: SlashPickerItem[] = [];
  const seen = new Set<string>();

  // Include local commands first (for non-pi_rpc drivers, /compact & /new are still shown if reported or pi_rpc)
  for (const cmd of LOCAL_COMMANDS) {
    if ((cmd.name === "compact" || cmd.name === "new") && driver === "terminal_pty") {
      continue;
    }
    items.push(cmd);
    seen.add(`${cmd.kind}:${cmd.name.toLowerCase()}`);
  }

  for (const entry of reported ?? []) {
    const cleanName = entry.name.trim().replace(/^\/+/, "");
    if (!cleanName) continue;
    const lower = cleanName.toLowerCase();

    const kind: SlashItemKind =
      entry.kind === "skill" || entry.source === "skill" || lower.startsWith("skill:")
        ? "skill"
        : entry.kind === "prompt" || entry.source === "prompt"
          ? "prompt"
          : "command";

    // Local UI-owned commands keep their local action row.
    if (kind === "command" && localOwned.has(lower)) {
      continue;
    }

    const dedupeKey = `${kind}:${lower}`;
    if (seen.has(dedupeKey)) {
      // Enrich built-in command if session reported an argument hint
      const existing = items.find((i) => `${i.kind}:${i.name.toLowerCase()}` === dedupeKey);
      if (existing && !existing.argumentHint && entry.argument_hint) {
        existing.argumentHint = sanitizeText(entry.argument_hint, 80);
      }
      continue;
    }
    seen.add(dedupeKey);

    const description =
      sanitizeText(entry.description, 220) ??
      (kind === "command" ? CURATED_DESCRIPTIONS[lower] : undefined);

    const aliases =
      kind === "skill" && lower.startsWith("skill:")
        ? [cleanName.slice("skill:".length)]
        : undefined;

    items.push({
      id: `${kind}:${cleanName}`,
      kind,
      name: cleanName,
      token: `/${cleanName}`,
      description,
      argumentHint: sanitizeText(entry.argument_hint, 80),
      source: entry.source ?? (kind === "skill" ? "skill" : kind === "prompt" ? "prompt" : "extension"),
      aliases,
    });
  }

  return items;
}

function isSubsequence(query: string, value: string): boolean {
  let qi = 0;
  for (let i = 0; i < value.length && qi < query.length; i += 1) {
    if (value[i] === query[qi]) qi += 1;
  }
  return qi === query.length;
}

function getNameRank(name: string, query: string): number | null {
  if (name === query) return 0;
  if (name.startsWith(query)) return 1;
  if (name.includes(query)) return 2;
  return isSubsequence(query, name) ? 3 : null;
}

function getMatchRank(item: SlashPickerItem, query: string, matchDescriptions: boolean): number | null {
  const q = query.toLowerCase();
  let rank = getNameRank(item.name.toLowerCase(), q);
  for (const alias of item.aliases ?? []) {
    const aliasRank = getNameRank(alias.toLowerCase(), q);
    if (aliasRank !== null && (rank === null || aliasRank + 0.5 < rank)) {
      rank = aliasRank + 0.5;
    }
  }
  if (rank !== null) return rank;
  if (matchDescriptions && item.description?.toLowerCase().includes(q)) {
    return 4;
  }
  return null;
}

function rankGroup(items: SlashPickerItem[], query: string, limit: number): SlashPickerItem[] {
  if (!query) return items.slice(0, limit);
  return items
    .map((item, idx) => ({ item, idx, rank: getMatchRank(item, query, true) }))
    .filter((entry): entry is { item: SlashPickerItem; idx: number; rank: number } => entry.rank !== null)
    .sort((a, b) => a.rank - b.rank || a.idx - b.idx)
    .slice(0, limit)
    .map((entry) => entry.item);
}

/**
 * Orca's `preferredNativeChatPickerIndex`: commands render before skills,
 * so an exact skill match still becomes the default highlighted row over a fuzzy command.
 */
export function preferredSlashPickerIndex(items: readonly SlashPickerItem[], query: string): number {
  if (!query || items.length === 0) return 0;
  let bestIndex = 0;
  let bestRank = Number.POSITIVE_INFINITY;
  for (let i = 0; i < items.length; i += 1) {
    const rank = getMatchRank(items[i], query, true);
    if (rank !== null && rank < bestRank) {
      bestRank = rank;
      bestIndex = i;
    }
  }
  return bestIndex;
}

/**
 * Derive Orca-style slash autocomplete state from the current draft & caret position.
 */
export function deriveSlashAutocomplete(
  draft: string,
  caret: number,
  catalog: readonly SlashPickerItem[],
  dismissedTriggerKey: string | null = null,
): SlashAutocompleteState {
  const safeCaret = Math.max(0, Math.min(caret, draft.length));
  const before = draft.slice(0, safeCaret);

  const leadingMatch = before.match(LEADING_SLASH_TRIGGER);
  let query: string;
  let triggerPosition: number;
  let dispatchable: boolean;

  if (leadingMatch) {
    query = leadingMatch[1];
    triggerPosition = 0;
    dispatchable = true;
  } else {
    const midMatch = before.match(MID_PROMPT_SLASH_TRIGGER);
    if (!midMatch) {
      return { open: false };
    }
    query = midMatch[1];
    triggerPosition = before.length - query.length - 1;
    dispatchable = false;
  }

  const triggerKey = `/:${triggerPosition}`;
  if (dismissedTriggerKey === triggerKey) {
    return { open: false };
  }

  // Mid-prompt slash only completes skills & prompts (or commands as text), keeping same clean grouping
  const rawCommands = catalog.filter((i) => i.kind === "command");
  const rawPrompts = catalog.filter((i) => i.kind === "prompt");
  const rawSkills = catalog.filter((i) => i.kind === "skill");

  const commands = rankGroup(rawCommands, query, COMMAND_RESULT_LIMIT);
  const prompts = rankGroup(rawPrompts, query, PROMPT_RESULT_LIMIT);
  const skills = rankGroup(rawSkills, query, SKILL_RESULT_LIMIT);
  const items = [...commands, ...prompts, ...skills];

  return {
    open: true,
    query,
    triggerKey,
    triggerPosition,
    dispatchable,
    items,
    commands,
    prompts,
    skills,
    preferredIndex: preferredSlashPickerIndex(items, query),
  };
}

/**
 * Replace the active `/` trigger token (and any trailing word characters belonging to it)
 * with `${item.token} `, returning the next draft and caret offset.
 * Direct port of Orca's `applyPickerSuggestion`.
 */
export function applySlashCompletion(
  draft: string,
  caret: number,
  item: SlashPickerItem,
): { draft: string; caret: number } {
  const safeCaret = Math.max(0, Math.min(caret, draft.length));
  const before = draft.slice(0, safeCaret);
  const match = before.match(LEADING_SLASH_TRIGGER) ?? before.match(MID_PROMPT_SLASH_TRIGGER);
  if (!match) {
    const trimmed = draft.trimStart();
    if (!trimmed) {
      const next = `${item.token} `;
      return { draft: next, caret: next.length };
    }
    const prefix = draft.slice(0, safeCaret);
    const needsSpace = prefix.length > 0 && !/\s$/.test(prefix);
    const nextBefore = `${prefix}${needsSpace ? " " : ""}${item.token} `;
    return { draft: nextBefore + draft.slice(safeCaret), caret: nextBefore.length };
  }

  const query = match[1];
  const tokenStart = before.length - query.length - 1;
  const names = [item.token.slice(1), ...(item.aliases ?? [])];
  const rest = draft.slice(safeCaret).match(/^[\w:-]*/)?.[0] ?? "";
  const typedName = `${query}${rest}`.toLowerCase();
  const completesName =
    query !== "" && names.some((n) => n.toLowerCase().includes(typedName));
  const nameEnd = safeCaret + (completesName ? rest.length : 0);
  const replacedEnd = nameEnd + (draft[nameEnd] === " " ? 1 : 0);
  const nextBefore = `${before.slice(0, tokenStart)}${item.token} `;
  return {
    draft: nextBefore + draft.slice(replacedEnd),
    caret: nextBefore.length,
  };
}

/**
 * Direct port of Orca's `editReplacesTriggerToken`:
 * Replacing across an entire dismissed token starts a fresh occurrence;
 * typing/deleting within the same token keeps it dismissed until a new `/` is typed.
 */
export function editReplacesTriggerToken(
  previous: string,
  next: string,
  triggerKey: string,
): boolean {
  const triggerPosition = Number.parseInt(triggerKey.slice(triggerKey.indexOf(":") + 1), 10);
  if (!Number.isFinite(triggerPosition) || previous === next) {
    return false;
  }
  let commonPrefix = 0;
  const maxPrefix = Math.min(previous.length, next.length);
  while (commonPrefix < maxPrefix && previous[commonPrefix] === next[commonPrefix]) {
    commonPrefix += 1;
  }
  let commonSuffix = 0;
  while (
    commonSuffix < previous.length - commonPrefix &&
    commonSuffix < next.length - commonPrefix &&
    previous[previous.length - 1 - commonSuffix] === next[next.length - 1 - commonSuffix]
  ) {
    commonSuffix += 1;
  }
  const removed = previous.length - commonPrefix - commonSuffix;
  const inserted = next.length - commonPrefix - commonSuffix;
  if (removed === 0 || inserted === 0) {
    return false;
  }
  let tokenEnd = triggerPosition + 1;
  while (tokenEnd < previous.length && !/\s/.test(previous[tokenEnd])) {
    tokenEnd += 1;
  }
  return commonPrefix < tokenEnd && previous.length - commonSuffix > triggerPosition;
}

export function slashSourceLabel(item: SlashPickerItem): string {
  if (item.localAction) return "快捷";
  switch (item.source) {
    case "builtin":
      return "内置";
    case "extension":
      return "扩展";
    case "skill":
      return "技能";
    case "prompt":
      return "模板";
    case "acp":
      return "Agent";
    default:
      return item.kind === "skill" ? "技能" : item.kind === "prompt" ? "模板" : "命令";
  }
}
