//! 统一结构化对话时间线模型与确定性状态归约器（Pi RPC / ACP / Transcript 共享）。
use crate::model::AgentId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionDriverMode {
    #[default]
    TerminalPty,
    PiRpc,
    Acp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionViewMode {
    #[default]
    Terminal,
    Chat,
    Split,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatDriverKind {
    PiRpc,
    Acp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StreamingBehavior {
    #[default]
    Steer,
    FollowUp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatAttachment {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub mime_type: String,
    /// Optional base64 payload (used for inline images).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_b64: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallKind {
    Read,
    Edit,
    Write,
    Bash,
    Search,
    Fetch,
    Think,
    #[default]
    Other,
}

impl ToolCallKind {
    pub fn from_tool_name(name: &str, acp_kind: Option<&str>) -> Self {
        if let Some(kind) = acp_kind {
            match kind {
                "read" => return Self::Read,
                "edit" => return Self::Edit,
                "delete" => return Self::Edit,
                "move" => return Self::Edit,
                "search" => return Self::Search,
                "execute" => return Self::Bash,
                "think" => return Self::Think,
                "fetch" => return Self::Fetch,
                _ => {}
            }
        }
        let lower = name.to_ascii_lowercase();
        if lower.contains("read") || lower == "view_file" {
            Self::Read
        } else if lower.contains("edit") || lower.contains("replace") || lower.contains("patch") {
            Self::Edit
        } else if lower.contains("write") || lower.contains("create") {
            Self::Write
        } else if lower == "bash"
            || lower == "sh"
            || lower == "shell"
            || lower.contains("exec")
            || lower.contains("command")
            || lower.contains("run")
        {
            Self::Bash
        } else if lower.contains("grep")
            || lower.contains("glob")
            || lower.contains("find")
            || lower.contains("search")
            || lower == "ls"
        {
            Self::Search
        } else if lower.contains("fetch") || lower.contains("web") || lower.contains("http") {
            Self::Fetch
        } else {
            Self::Other
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallState {
    #[default]
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolDiff {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub old_text: String,
    #[serde(default)]
    pub new_text: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
    Compaction,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanEntryStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanEntry {
    pub content: String,
    #[serde(default)]
    pub status: PlanEntryStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineEntry {
    UserMessage {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        text: String,
        #[serde(default)]
        attachments: Vec<ChatAttachment>,
        created_at_ms: u64,
    },
    Thinking {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        text: String,
        #[serde(default)]
        streaming: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        created_at_ms: u64,
    },
    AssistantText {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        text: String,
        #[serde(default)]
        streaming: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Box<MessageUsage>>,
        created_at_ms: u64,
    },
    ToolCall {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        call_id: String,
        name: String,
        #[serde(default)]
        kind: ToolCallKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default)]
        input: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<Box<ToolDiff>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        #[serde(default)]
        locations: Vec<String>,
        #[serde(default)]
        state: ToolCallState,
        #[serde(default)]
        is_error: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        created_at_ms: u64,
    },
    Plan {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        #[serde(default)]
        entries: Vec<PlanEntry>,
        created_at_ms: u64,
    },
    Notice {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        level: NoticeLevel,
        text: String,
        created_at_ms: u64,
    },
}

impl TimelineEntry {
    pub fn id(&self) -> &str {
        match self {
            Self::UserMessage { id, .. }
            | Self::Thinking { id, .. }
            | Self::AssistantText { id, .. }
            | Self::ToolCall { id, .. }
            | Self::Plan { id, .. }
            | Self::Notice { id, .. } => id,
        }
    }

    pub fn created_at_ms(&self) -> u64 {
        match self {
            Self::UserMessage { created_at_ms, .. }
            | Self::Thinking { created_at_ms, .. }
            | Self::AssistantText { created_at_ms, .. }
            | Self::ToolCall { created_at_ms, .. }
            | Self::Plan { created_at_ms, .. }
            | Self::Notice { created_at_ms, .. } => *created_at_ms,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalSource {
    AcpPermission,
    PiExtensionUi,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalOption {
    pub option_id: String,
    pub label: String,
    /// ACP kind e.g. allow_once / allow_always / reject_once / reject_always, or confirm/select
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingApproval {
    pub id: String,
    pub agent: AgentId,
    pub source: ApprovalSource,
    /// "permission" | "confirm" | "select" | "input" | "editor"
    pub method: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_or_input: Option<String>,
    #[serde(default)]
    pub options: Vec<ApprovalOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefill: Option<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApprovalReply {
    SelectedOption { option_id: String },
    Confirm { confirmed: bool },
    Text { value: String },
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatModelInfo {
    #[serde(default)]
    pub provider: String,
    pub id: String,
    pub name: String,
}

fn default_slash_command_kind() -> String {
    "command".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatSlashCommand {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// "command" | "skill" | "prompt"
    #[serde(default = "default_slash_command_kind")]
    pub kind: String,
    /// Optional source tag such as "extension" | "skill" | "prompt" | "builtin"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueuedMessage {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub behavior: StreamingBehavior,
    #[serde(default)]
    pub attachments_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MessageUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttft_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SessionStats {
    #[serde(default)]
    pub turns: u32,
    #[serde(default)]
    pub steps: u32,
    #[serde(default)]
    pub tool_calls: u32,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_cost_usd: Option<f64>,
    #[serde(default)]
    pub llm_ms: u64,
    #[serde(default)]
    pub tool_ms: u64,
    #[serde(default)]
    pub ttft_ms: u64,
    #[serde(default)]
    pub ttft_steps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionGoal {
    pub objective: String,
    /// "active" | "paused" | "complete"
    pub status: String,
    #[serde(default)]
    pub rounds: u32,
    #[serde(default)]
    pub max_rounds: u32,
    #[serde(default)]
    pub armed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionTask {
    pub id: String,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// "pending" | "in_progress" | "completed"
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSubagent {
    pub id: String,
    pub agent: String,
    pub description: String,
    /// "queued" | "running" | "completed" | "error" | "stopped" | "interrupted"
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub turns: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackgroundJob {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// "running" | "completed" | "failed" | "killed"
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_written: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ChatSessionMeta {
    #[serde(default)]
    pub driver: SessionDriverMode,
    #[serde(default)]
    pub supports_chat_input: bool,
    #[serde(default)]
    pub supports_terminal_switch: bool,
    #[serde(default)]
    pub has_pty: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_model: Option<ChatModelInfo>,
    #[serde(default)]
    pub available_models: Vec<ChatModelInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
    #[serde(default)]
    pub available_thinking_levels: Vec<String>,
    #[serde(default)]
    pub available_commands: Vec<ChatSlashCommand>,
    #[serde(default)]
    pub queued_messages: Vec<QueuedMessage>,
    #[serde(default)]
    pub is_streaming: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_used_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<Box<SessionStats>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<Box<SessionGoal>>,
    #[serde(default)]
    pub tasks: Vec<SessionTask>,
    #[serde(default)]
    pub subagents: Vec<SessionSubagent>,
    #[serde(default)]
    pub background_jobs: Vec<BackgroundJob>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextDeltaKind {
    Assistant,
    Thinking,
    ToolOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ChatDeltaOp {
    UpsertEntry {
        entry: TimelineEntry,
    },
    RemoveEntry {
        entry_id: String,
    },
    AppendText {
        entry_id: String,
        delta: String,
        kind: TextDeltaKind,
    },
    ReplaceEntryText {
        entry_id: String,
        text: String,
        kind: TextDeltaKind,
        #[serde(default)]
        finish: bool,
    },
    FinishEntry {
        entry_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
    },
    SetAssistantUsage {
        entry_id: String,
        usage: Box<MessageUsage>,
    },
    MetaUpdated {
        meta: ChatSessionMeta,
    },
    ApprovalRequested {
        approval: PendingApproval,
    },
    ApprovalResolved {
        approval_id: String,
    },
    TurnFinished {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Reset,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatDeltaEvent {
    pub agent: AgentId,
    pub seq: u64,
    pub delta: ChatDeltaOp,
}

/// 确定性的对话时间线状态容器（服务端与客户端共享同一归约逻辑）。
#[derive(Debug, Clone, Default)]
pub struct ChatTimelineState {
    pub seq: u64,
    pub meta: ChatSessionMeta,
    pub entries: Vec<TimelineEntry>,
    pub pending_approvals: Vec<PendingApproval>,
    index: HashMap<String, usize>,
}

impl ChatTimelineState {
    pub fn from_snapshot(
        seq: u64,
        meta: ChatSessionMeta,
        entries: Vec<TimelineEntry>,
        pending_approvals: Vec<PendingApproval>,
    ) -> Self {
        let mut state = Self {
            seq,
            meta,
            entries,
            pending_approvals,
            index: HashMap::new(),
        };
        state.rebuild_index();
        state
    }

    pub fn rebuild_index(&mut self) {
        self.index.clear();
        for (idx, entry) in self.entries.iter().enumerate() {
            self.index.insert(entry.id().to_string(), idx);
        }
    }

    pub fn get(&self, id: &str) -> Option<&TimelineEntry> {
        self.index.get(id).and_then(|&idx| self.entries.get(idx))
    }

    /// 按顺序应用增量事件；重复或过期 seq 自动幂等跳过。
    pub fn apply(&mut self, event: ChatDeltaEvent) -> bool {
        if event.seq <= self.seq {
            return false;
        }
        self.seq = event.seq;
        self.apply_op(event.delta);
        true
    }

    pub fn apply_op(&mut self, op: ChatDeltaOp) {
        match op {
            ChatDeltaOp::UpsertEntry { entry } => {
                if let Some(&idx) = self.index.get(entry.id()) {
                    self.entries[idx] = entry;
                } else {
                    let idx = self.entries.len();
                    self.index.insert(entry.id().to_string(), idx);
                    self.entries.push(entry);
                }
            }
            ChatDeltaOp::RemoveEntry { entry_id } => {
                if let Some(idx) = self.index.get(&entry_id).copied() {
                    self.entries.remove(idx);
                    self.rebuild_index();
                }
            }
            ChatDeltaOp::AppendText {
                entry_id,
                delta,
                kind,
            } => {
                if let Some(&idx) = self.index.get(&entry_id) {
                    match (&mut self.entries[idx], kind) {
                        (TimelineEntry::AssistantText { text, .. }, TextDeltaKind::Assistant) => {
                            text.push_str(&delta);
                        }
                        (TimelineEntry::Thinking { text, .. }, TextDeltaKind::Thinking) => {
                            text.push_str(&delta);
                        }
                        (TimelineEntry::ToolCall { output, .. }, TextDeltaKind::ToolOutput) => {
                            match output {
                                Some(existing) => existing.push_str(&delta),
                                None => *output = Some(delta),
                            }
                        }
                        _ => {}
                    }
                }
            }
            ChatDeltaOp::ReplaceEntryText {
                entry_id,
                text: replacement,
                kind,
                finish,
            } => {
                if let Some(&idx) = self.index.get(&entry_id) {
                    match (&mut self.entries[idx], kind) {
                        (
                            TimelineEntry::AssistantText {
                                text, streaming, ..
                            },
                            TextDeltaKind::Assistant,
                        ) => {
                            *text = replacement;
                            if finish {
                                *streaming = false;
                            }
                        }
                        (
                            TimelineEntry::Thinking {
                                text,
                                streaming,
                                duration_ms,
                                created_at_ms,
                                ..
                            },
                            TextDeltaKind::Thinking,
                        ) => {
                            *text = replacement;
                            if finish {
                                *streaming = false;
                                if duration_ms.is_none() {
                                    *duration_ms = Some(now_ms().saturating_sub(*created_at_ms));
                                }
                            }
                        }
                        (TimelineEntry::ToolCall { output, .. }, TextDeltaKind::ToolOutput) => {
                            *output = Some(replacement);
                        }
                        _ => {}
                    }
                }
            }
            ChatDeltaOp::FinishEntry {
                entry_id,
                duration_ms,
            } => {
                if let Some(&idx) = self.index.get(&entry_id) {
                    match &mut self.entries[idx] {
                        TimelineEntry::AssistantText { streaming, .. } => {
                            *streaming = false;
                        }
                        TimelineEntry::Thinking {
                            streaming,
                            duration_ms: slot,
                            created_at_ms,
                            ..
                        } => {
                            *streaming = false;
                            *slot = duration_ms
                                .or(*slot)
                                .or_else(|| Some(now_ms().saturating_sub(*created_at_ms)));
                        }
                        _ => {}
                    }
                }
            }
            ChatDeltaOp::SetAssistantUsage { entry_id, usage } => {
                if let Some(&idx) = self.index.get(&entry_id) {
                    if let TimelineEntry::AssistantText { usage: slot, .. } = &mut self.entries[idx]
                    {
                        *slot = Some(usage);
                    }
                }
            }
            ChatDeltaOp::MetaUpdated { meta } => {
                self.meta = meta;
            }
            ChatDeltaOp::ApprovalRequested { approval } => {
                if let Some(existing) = self
                    .pending_approvals
                    .iter_mut()
                    .find(|item| item.id == approval.id)
                {
                    *existing = approval;
                } else {
                    self.pending_approvals.push(approval);
                }
            }
            ChatDeltaOp::ApprovalResolved { approval_id } => {
                self.pending_approvals.retain(|item| item.id != approval_id);
            }
            ChatDeltaOp::TurnFinished { error, .. } => {
                self.meta.is_streaming = false;
                if let Some(err) = error {
                    self.meta.last_error = Some(err);
                }
                for entry in &mut self.entries {
                    match entry {
                        TimelineEntry::AssistantText { streaming, .. }
                        | TimelineEntry::Thinking { streaming, .. } => {
                            *streaming = false;
                        }
                        TimelineEntry::ToolCall { state, .. }
                            if matches!(state, ToolCallState::Pending | ToolCallState::Running) =>
                        {
                            *state = ToolCallState::Completed;
                        }
                        _ => {}
                    }
                }
            }
            ChatDeltaOp::Reset => {
                self.entries.clear();
                self.index.clear();
                self.pending_approvals.clear();
                self.meta.last_error = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reducer_applies_streaming_text_and_tools_in_order() {
        let mut state = ChatTimelineState::default();
        assert!(state.apply(ChatDeltaEvent {
            agent: "pi-1".into(),
            seq: 1,
            delta: ChatDeltaOp::UpsertEntry {
                entry: TimelineEntry::Thinking {
                    id: "t1".into(),
                    turn_id: Some("turn-1".into()),
                    text: String::new(),
                    streaming: true,
                    duration_ms: None,
                    created_at_ms: 1000,
                },
            },
        }));
        // Stale seq is ignored
        assert!(!state.apply(ChatDeltaEvent {
            agent: "pi-1".into(),
            seq: 1,
            delta: ChatDeltaOp::Reset,
        }));
        assert!(state.apply(ChatDeltaEvent {
            agent: "pi-1".into(),
            seq: 2,
            delta: ChatDeltaOp::AppendText {
                entry_id: "t1".into(),
                delta: "planning...".into(),
                kind: TextDeltaKind::Thinking,
            },
        }));
        assert!(state.apply(ChatDeltaEvent {
            agent: "pi-1".into(),
            seq: 3,
            delta: ChatDeltaOp::FinishEntry {
                entry_id: "t1".into(),
                duration_ms: Some(420),
            },
        }));
        assert!(state.apply(ChatDeltaEvent {
            agent: "pi-1".into(),
            seq: 4,
            delta: ChatDeltaOp::UpsertEntry {
                entry: TimelineEntry::AssistantText {
                    id: "a1".into(),
                    turn_id: Some("turn-1".into()),
                    text: "Hello ".into(),
                    streaming: true,
                    model: Some("sonnet".into()),
                    usage: None,
                    created_at_ms: 1500,
                },
            },
        }));
        assert!(state.apply(ChatDeltaEvent {
            agent: "pi-1".into(),
            seq: 5,
            delta: ChatDeltaOp::AppendText {
                entry_id: "a1".into(),
                delta: "world".into(),
                kind: TextDeltaKind::Assistant,
            },
        }));
        assert!(state.apply(ChatDeltaEvent {
            agent: "pi-1".into(),
            seq: 6,
            delta: ChatDeltaOp::SetAssistantUsage {
                entry_id: "a1".into(),
                usage: Box::new(MessageUsage {
                    input_tokens: 120,
                    output_tokens: 45,
                    cache_read_tokens: 80,
                    total_tokens: 245,
                    duration_ms: Some(1350),
                    ttft_ms: Some(210),
                    ..Default::default()
                }),
            },
        }));
        assert!(state.apply(ChatDeltaEvent {
            agent: "pi-1".into(),
            seq: 7,
            delta: ChatDeltaOp::TurnFinished {
                turn_id: Some("turn-1".into()),
                error: None,
            },
        }));

        assert_eq!(state.entries.len(), 2);
        match &state.entries[0] {
            TimelineEntry::Thinking {
                text,
                streaming,
                duration_ms,
                ..
            } => {
                assert_eq!(text, "planning...");
                assert!(!streaming);
                assert_eq!(*duration_ms, Some(420));
            }
            other => panic!("unexpected entry: {other:?}"),
        }
        match &state.entries[1] {
            TimelineEntry::AssistantText {
                text,
                streaming,
                usage,
                ..
            } => {
                assert_eq!(text, "Hello world");
                assert!(!streaming);
                assert_eq!(usage.as_deref().map(|u| u.total_tokens), Some(245));
                assert_eq!(usage.as_deref().and_then(|u| u.duration_ms), Some(1350));
            }
            other => panic!("unexpected entry: {other:?}"),
        }
    }

    #[test]
    fn tool_kind_classification_covers_pi_and_acp() {
        assert_eq!(
            ToolCallKind::from_tool_name("read", None),
            ToolCallKind::Read
        );
        assert_eq!(
            ToolCallKind::from_tool_name("edit", None),
            ToolCallKind::Edit
        );
        assert_eq!(
            ToolCallKind::from_tool_name("write", None),
            ToolCallKind::Write
        );
        assert_eq!(
            ToolCallKind::from_tool_name("bash", None),
            ToolCallKind::Bash
        );
        assert_eq!(
            ToolCallKind::from_tool_name("custom_tool", Some("execute")),
            ToolCallKind::Bash
        );
        assert_eq!(
            ToolCallKind::from_tool_name("ripgrep", Some("search")),
            ToolCallKind::Search
        );
    }

    #[test]
    fn reducer_removes_entry_and_rebuilds_index() {
        let mut state = ChatTimelineState::default();
        assert!(state.apply(ChatDeltaEvent {
            agent: "pi-1".into(),
            seq: 1,
            delta: ChatDeltaOp::UpsertEntry {
                entry: TimelineEntry::UserMessage {
                    id: "__bridge_pending_user__:1".into(),
                    turn_id: None,
                    text: "hello".into(),
                    attachments: Vec::new(),
                    created_at_ms: 100,
                },
            },
        }));
        assert!(state.apply(ChatDeltaEvent {
            agent: "pi-1".into(),
            seq: 2,
            delta: ChatDeltaOp::UpsertEntry {
                entry: TimelineEntry::AssistantText {
                    id: "a1".into(),
                    turn_id: None,
                    text: "hi".into(),
                    streaming: true,
                    model: None,
                    usage: None,
                    created_at_ms: 200,
                },
            },
        }));
        assert!(state.apply(ChatDeltaEvent {
            agent: "pi-1".into(),
            seq: 3,
            delta: ChatDeltaOp::RemoveEntry {
                entry_id: "__bridge_pending_user__:1".into(),
            },
        }));
        assert_eq!(state.entries.len(), 1);
        assert!(state.get("__bridge_pending_user__:1").is_none());
        assert!(state.get("a1").is_some());
        assert!(state.apply(ChatDeltaEvent {
            agent: "pi-1".into(),
            seq: 4,
            delta: ChatDeltaOp::AppendText {
                entry_id: "a1".into(),
                delta: "!".into(),
                kind: TextDeltaKind::Assistant,
            },
        }));
        match state.get("a1") {
            Some(TimelineEntry::AssistantText { text, .. }) => assert_eq!(text, "hi!"),
            other => panic!("expected updated a1 after removal, got {other:?}"),
        }
    }
}
