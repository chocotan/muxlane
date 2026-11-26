// 1:1 TypeScript wire protocol definitions mirroring crates/muxlane-core.

export type AgentType =
  | "claude"
  | "codex"
  | "opencode"
  | "pi"
  | "pie"
  | "gemini"
  | "agy"
  | "qwen"
  | "kimi"
  | "shell"
  | "unknown";

export type AgentStatus = "working" | "blocked" | "idle" | "done" | "failed" | "unknown";

export type SessionDriverMode = "terminal_pty" | "pi_rpc" | "acp";
export type SessionViewMode = "terminal" | "chat" | "split";
export type ChatDriverKind = "pi_rpc" | "acp";
export type StreamingBehavior = "steer" | "follow_up";

export interface MachineInfo {
  machine_id: string;
  name: string;
  os: string;
  version: string;
}

export interface Project {
  id: string;
  name: string;
  path: string;
  branch?: string | null;
  agents: string[];
}

export interface AgentInstance {
  id: string;
  project: string;
  agent_type: AgentType;
  title: string;
  title_is_custom: boolean;
  status: AgentStatus;
  status_since: number;
  seen: boolean;
  tmux_session?: string | null;
}

export interface Snapshot {
  machine: MachineInfo;
  projects: Project[];
  agents: AgentInstance[];
  seq: number;
}

export interface AgentPreset {
  id: string;
  label: string;
  agent_type: AgentType;
  program: string;
  args: string[];
  env: Record<string, string>;
  require_installed: boolean;
}

export interface ChatAttachment {
  name: string;
  path?: string | null;
  mime_type: string;
  data_b64?: string | null;
}

export type ToolCallKind =
  | "read"
  | "edit"
  | "write"
  | "bash"
  | "search"
  | "fetch"
  | "think"
  | "other";

export type ToolCallState = "pending" | "running" | "completed" | "failed";

export interface ToolDiff {
  path?: string | null;
  old_text: string;
  new_text: string;
}

export type NoticeLevel = "info" | "warning" | "error" | "compaction";

export type PlanEntryStatus = "pending" | "in_progress" | "completed";

export interface PlanEntry {
  content: string;
  status: PlanEntryStatus;
  priority?: string | null;
}

export type TimelineEntry =
  | {
      type: "user_message";
      id: string;
      turn_id?: string | null;
      text: string;
      attachments: ChatAttachment[];
      created_at_ms: number;
    }
  | {
      type: "thinking";
      id: string;
      turn_id?: string | null;
      text: string;
      streaming: boolean;
      duration_ms?: number | null;
      created_at_ms: number;
    }
  | {
      type: "assistant_text";
      id: string;
      turn_id?: string | null;
      text: string;
      streaming: boolean;
      model?: string | null;
      usage?: MessageUsage | null;
      created_at_ms: number;
    }
  | {
      type: "tool_call";
      id: string;
      turn_id?: string | null;
      call_id: string;
      name: string;
      kind: ToolCallKind;
      title?: string | null;
      input: unknown;
      output?: string | null;
      diff?: ToolDiff | null;
      exit_code?: number | null;
      locations: string[];
      state: ToolCallState;
      is_error: boolean;
      duration_ms?: number | null;
      created_at_ms: number;
    }
  | {
      type: "plan";
      id: string;
      turn_id?: string | null;
      entries: PlanEntry[];
      created_at_ms: number;
    }
  | {
      type: "notice";
      id: string;
      turn_id?: string | null;
      level: NoticeLevel;
      text: string;
      created_at_ms: number;
    };

export type ApprovalSource = "acp_permission" | "pi_extension_ui";

export interface ApprovalOption {
  option_id: string;
  label: string;
  kind?: string | null;
}

export interface PendingApproval {
  id: string;
  agent: string;
  source: ApprovalSource;
  method: string;
  title: string;
  message?: string | null;
  tool_name?: string | null;
  command_or_input?: string | null;
  options: ApprovalOption[];
  prefill?: string | null;
  created_at_ms: number;
}

export type ApprovalReply =
  | { kind: "selected_option"; option_id: string }
  | { kind: "confirm"; confirmed: boolean }
  | { kind: "text"; value: string }
  | { kind: "cancel" };

export interface ChatModelInfo {
  provider: string;
  id: string;
  name: string;
}

export interface ChatSlashCommand {
  name: string;
  description?: string | null;
  /** "command" | "skill" | "prompt" */
  kind: "command" | "skill" | "prompt" | string;
  source?: string | null;
  argument_hint?: string | null;
}

export interface QueuedMessage {
  id: string;
  text: string;
  behavior: StreamingBehavior;
  attachments_count?: number;
}

export interface MessageUsage {
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  reasoning_tokens: number;
  total_tokens: number;
  cost_usd?: number | null;
  duration_ms?: number | null;
  ttft_ms?: number | null;
}

export interface SessionStats {
  turns: number;
  steps: number;
  tool_calls: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  reasoning_tokens: number;
  total_tokens: number;
  total_cost_usd?: number | null;
  llm_ms: number;
  tool_ms: number;
  ttft_ms: number;
  ttft_steps: number;
}

export interface SessionGoal {
  objective: string;
  status: "active" | "paused" | "complete" | string;
  rounds: number;
  max_rounds: number;
  armed: boolean;
}

export interface SessionTask {
  id: string;
  subject: string;
  description?: string | null;
  status: "pending" | "in_progress" | "completed" | string;
  active_form?: string | null;
  agent_type?: string | null;
  owner?: string | null;
  blocked_by: string[];
}

export interface SessionSubagent {
  id: string;
  agent: string;
  description: string;
  status: "queued" | "running" | "completed" | "error" | "stopped" | "interrupted" | string;
  model?: string | null;
  turns: number;
  max_turns?: number | null;
  active_tool?: string | null;
  output?: string | null;
  error?: string | null;
  task_id?: string | null;
  started_at_ms?: number | null;
  finished_at_ms?: number | null;
}

export interface BackgroundJob {
  id: string;
  name: string;
  description: string;
  command?: string | null;
  status: "running" | "completed" | "failed" | "killed" | string;
  exit_code?: number | null;
  output_path?: string | null;
  bytes_written?: number | null;
  error?: string | null;
  started_at_ms?: number | null;
  finished_at_ms?: number | null;
}

export interface ChatSessionMeta {
  driver: SessionDriverMode;
  supports_chat_input: boolean;
  supports_terminal_switch: boolean;
  has_pty: boolean;
  provider_session_id?: string | null;
  provider_session_file?: string | null;
  current_model?: ChatModelInfo | null;
  available_models: ChatModelInfo[];
  thinking_level?: string | null;
  available_thinking_levels: string[];
  available_commands?: ChatSlashCommand[];
  queued_messages?: QueuedMessage[];
  is_streaming: boolean;
  context_used_tokens?: number | null;
  context_window_tokens?: number | null;
  total_cost_usd?: number | null;
  stats?: SessionStats | null;
  goal?: SessionGoal | null;
  tasks?: SessionTask[];
  subagents?: SessionSubagent[];
  background_jobs?: BackgroundJob[];
  last_error?: string | null;
}

export type TextDeltaKind = "assistant" | "thinking" | "tool_output";

export type ChatDeltaOp =
  | { op: "upsert_entry"; entry: TimelineEntry }
  | { op: "remove_entry"; entry_id: string }
  | { op: "append_text"; entry_id: string; delta: string; kind: TextDeltaKind }
  | {
      op: "replace_entry_text";
      entry_id: string;
      text: string;
      kind: TextDeltaKind;
      finish?: boolean;
    }
  | { op: "finish_entry"; entry_id: string; duration_ms?: number | null }
  | { op: "set_assistant_usage"; entry_id: string; usage: MessageUsage }
  | { op: "meta_updated"; meta: ChatSessionMeta }
  | { op: "approval_requested"; approval: PendingApproval }
  | { op: "approval_resolved"; approval_id: string }
  | { op: "turn_finished"; turn_id?: string | null; error?: string | null }
  | { op: "reset" };

export interface ChatDeltaEvent {
  agent: string;
  seq: number;
  delta: ChatDeltaOp;
}

export interface ChatHistoryResult {
  agent: string;
  seq: number;
  meta: ChatSessionMeta;
  entries: TimelineEntry[];
  pending_approvals: PendingApproval[];
}

export interface ChatSubscribeResult {
  sub_id: string;
  history: ChatHistoryResult;
}

export interface ChatSessionSummary {
  agent: string;
  meta: ChatSessionMeta;
  pending_approvals: number;
  seq: number;
}

export interface ChatListResult {
  sessions: ChatSessionSummary[];
}

export interface ChatApprovalNoticeEvent {
  agent: string;
  approval?: PendingApproval | null;
  resolved_id?: string | null;
}

export interface TermSubscribeResult {
  sub_id: string;
  replay_b64: string;
}

export interface TermDataEvent {
  agent: string;
  data_b64: string;
}

export interface TermResyncEvent {
  agent: string;
  replay_b64: string;
}

export interface TermReplayChunkEvent {
  agent: string;
  sub_id: string;
  replay_id: number;
  chunk_index: number;
  data_b64: string;
}

export type BotPermissionProfile = "read_only" | "workspace_safe" | "ask_first" | "full_auto";

export type BotTrigger =
  | { kind: "manual" }
  | { kind: "interval"; every_secs: number }
  | { kind: "cron"; expr: string }
  | { kind: "webhook"; secret?: string | null }
  | {
      kind: "im";
      connector: {
        platform: "telegram" | "slack" | "discord" | "feishu" | "ding_talk" | "generic_webhook";
        webhook_url?: string | null;
        bot_token?: string | null;
        signing_secret?: string | null;
        channel_or_chat_id?: string | null;
        poll_interval_secs: number;
      };
      allow_from: string[];
    };

export interface BotDefinition {
  id: string;
  name: string;
  project_id: string;
  enabled: boolean;
  agent_type: AgentType;
  program?: string | null;
  args: string[];
  driver: ChatDriverKind;
  model?: string | null;
  thinking_level?: string | null;
  prompt_template: string;
  soul_markdown: string;
  memory_markdown: string;
  permission: BotPermissionProfile;
  isolated_worktree: boolean;
  max_concurrency: number;
  timeout_secs: number;
  triggers: BotTrigger[];
  notify_on_finish: boolean;
  created_at: number;
  updated_at: number;
}

export type BotRunStatus =
  | "queued"
  | "running"
  | "needs_approval"
  | "succeeded"
  | "failed"
  | "timed_out"
  | "cancelled";

export interface BotRunRecord {
  id: string;
  bot_id: string;
  agent_id?: string | null;
  project_id: string;
  trigger_source: string;
  prompt: string;
  worktree_path?: string | null;
  status: BotRunStatus;
  summary?: string | null;
  error?: string | null;
  started_at: number;
  finished_at?: number | null;
}

export interface RpcEventMessage {
  event: string;
  params: unknown;
}

export interface DaemonConnectionStatus {
  connected: boolean;
  connecting: boolean;
  socketPath: string;
  error?: string | null;
}

export const Methods = {
  SYSTEM_HELLO: "system.hello",
  STATE_LIST: "state.list",
  EVENTS_SUBSCRIBE: "events.subscribe",
  TERM_SUBSCRIBE: "term.subscribe",
  TERM_UNSUBSCRIBE: "term.unsubscribe",
  TERM_INPUT: "term.input",
  TERM_RESIZE: "term.resize",
  AGENT_SPAWN: "agent.spawn",
  AGENT_DELETE: "agent.delete",
  AGENT_RENAME: "agent.rename",
  AGENT_MARK_SEEN: "agent.mark_seen",
  PROJECT_ADD: "project.add",
  PROJECT_DELETE: "project.delete",
  PRESET_LIST: "preset.list",
  CHAT_SPAWN: "chat.spawn",
  CHAT_LIST: "chat.list",
  CHAT_HISTORY: "chat.history",
  CHAT_SUBSCRIBE: "chat.subscribe",
  CHAT_UNSUBSCRIBE: "chat.unsubscribe",
  CHAT_SEND: "chat.send",
  CHAT_ABORT: "chat.abort",
  CHAT_QUEUE_DELETE: "chat.queue.delete",
  CHAT_APPROVE: "chat.approve",
  CHAT_CONFIGURE: "chat.configure",
  CHAT_SWITCH_MODE: "chat.switch_mode",
  BOT_LIST: "bot.list",
  BOT_SAVE: "bot.save",
  BOT_DELETE: "bot.delete",
  BOT_TRIGGER: "bot.trigger",
  BOT_RUNS: "bot.runs",
} as const;

export const Events = {
  STATE_CHANGED: "state.changed",
  AGENT_STATUS: "agent.status_changed",
  TERM_DATA: "term.data",
  TERM_RESYNC: "term.resync",
  TERM_REPLAY_CHUNK: "term.replay_chunk",
  TERM_EXIT: "term.exit",
  CHAT_DELTA: "chat.delta",
  CHAT_APPROVAL: "chat.approval",
  BOT_UPDATED: "bot.updated",
  BOT_RUN_UPDATED: "bot.run_updated",
} as const;

export function supportsChatBridge(agentType: AgentType): boolean {
  return agentType !== "shell" && agentType !== "unknown";
}

export function presetChatDriver(agentType: AgentType): ChatDriverKind | null {
  if (agentType === "pi" || agentType === "pie") return "pi_rpc";
  if (
    agentType === "opencode" ||
    agentType === "gemini" ||
    agentType === "qwen" ||
    agentType === "kimi" ||
    agentType === "agy"
  ) {
    return "acp";
  }
  return null;
}

export type HostKind = "local" | "ssh" | "socket";
export type SshAuthKind = "ssh_config" | "public_key" | "password";

export interface HostEntry {
  id: string;
  name: string;
  kind: HostKind;
  target: string;
  authKind?: SshAuthKind;
  username?: string;
  port?: number;
  identityFile?: string;
  remoteSocketPath?: string;
  hasPassword?: boolean;
  tunnelReady?: boolean;
}

export interface DshInstanceInfo {
  projectId: string;
  projectPath: string;
  url: string;
  status: "starting" | "ready" | "error";
  error?: string | null;
  isCustomUrl?: boolean;
  reusedExisting?: boolean;
}

export interface AddHostInput {
  name?: string;
  kind: "ssh" | "socket";
  target: string;
  authKind?: SshAuthKind;
  username?: string;
  password?: string;
  port?: number;
  identityFile?: string;
  remoteSocketPath?: string;
}

export interface HostsState {
  activeHostId: string;
  hosts: HostEntry[];
  tunnelStatus?: string | null;
  tunnelPhase?: "idle" | "connecting" | "error";
}

export interface RelayConfig {
  relayEnabled: boolean;
  relayUrl: string;
}

export interface WorkspaceGitFile {
  path: string;
  /** M | A | D | R | ? | U */
  status: string;
  staged: boolean;
  unstaged: boolean;
  additions: number;
  deletions: number;
}

export interface WorkspaceGitChanges {
  isRepo: boolean;
  branch: string | null;
  files: WorkspaceGitFile[];
  error?: string | null;
}

export interface MuxlaneDesktopBridge {
  rpc<T = unknown>(method: string, params?: unknown): Promise<T>;
  getStatus(): Promise<DaemonConnectionStatus>;
  reconnect(): Promise<void>;
  pickDirectory(): Promise<string | null>;
  pickFile(title?: string): Promise<string | null>;
  openExternal(url: string): Promise<void>;
  setZoomFactor(factor: number): void;
  listHosts(): Promise<HostsState>;
  addHost(input: AddHostInput): Promise<HostsState>;
  removeHost(hostId: string): Promise<HostsState>;
  selectHost(hostId: string): Promise<HostsState>;
  getRelayConfig(): Promise<RelayConfig>;
  saveRelayConfig(cfg: RelayConfig): Promise<RelayConfig>;
  getGitChanges(cwd: string): Promise<WorkspaceGitChanges>;
  getGitFileDiff(cwd: string, filePath: string, status: string): Promise<string>;
  openDshWeb(projectId: string, projectPath: string, customUrl?: string): Promise<DshInstanceInfo>;
  restartDshWeb(projectId: string, projectPath: string): Promise<DshInstanceInfo>;
  stopDshWeb(projectId: string): Promise<void>;
  listDshWeb(): Promise<DshInstanceInfo[]>;
  onEvent(handler: (msg: RpcEventMessage) => void): () => void;
  onStatus(handler: (status: DaemonConnectionStatus) => void): () => void;
  onHostsChanged(handler: (state: HostsState) => void): () => void;
}

declare global {
  interface Window {
    muxlane: MuxlaneDesktopBridge;
  }
}
