//! Headless background Bot models: SOUL.md/MEMORY.md config, triggers (Cron/Interval/Webhook/IM),
//! permission profiles, git worktree isolation, and execution history records.
use crate::chat::ChatDriverKind;
use crate::model::{AgentId, AgentType, ProjectId};
use serde::{Deserialize, Serialize};

pub type BotId = String;
pub type BotRunId = String;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BotPermissionProfile {
    ReadOnly,
    WorkspaceSafe,
    #[default]
    AskFirst,
    FullAuto,
}

impl BotPermissionProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            BotPermissionProfile::ReadOnly => "read_only",
            BotPermissionProfile::WorkspaceSafe => "workspace_safe",
            BotPermissionProfile::AskFirst => "ask_first",
            BotPermissionProfile::FullAuto => "full_auto",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImPlatform {
    Telegram,
    Slack,
    Discord,
    Feishu,
    DingTalk,
    #[default]
    GenericWebhook,
}

impl ImPlatform {
    pub fn label(&self) -> &'static str {
        match self {
            ImPlatform::Telegram => "Telegram",
            ImPlatform::Slack => "Slack",
            ImPlatform::Discord => "Discord",
            ImPlatform::Feishu => "Feishu",
            ImPlatform::DingTalk => "DingTalk",
            ImPlatform::GenericWebhook => "Webhook",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ImConnectorConfig {
    #[serde(default)]
    pub platform: ImPlatform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_or_chat_id: Option<String>,
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
}

fn default_poll_interval_secs() -> u64 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BotTrigger {
    Manual,
    Interval {
        every_secs: u64,
    },
    Cron {
        expr: String,
    },
    Webhook {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        secret: Option<String>,
    },
    Im {
        connector: ImConnectorConfig,
        #[serde(default)]
        allow_from: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BotDefinition {
    pub id: BotId,
    pub name: String,
    pub project_id: ProjectId,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_agent_type")]
    pub agent_type: AgentType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_driver")]
    pub driver: ChatDriverKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
    #[serde(default)]
    pub prompt_template: String,
    #[serde(default)]
    pub soul_markdown: String,
    #[serde(default)]
    pub memory_markdown: String,
    #[serde(default)]
    pub permission: BotPermissionProfile,
    #[serde(default = "default_true")]
    pub isolated_worktree: bool,
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: u32,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub triggers: Vec<BotTrigger>,
    #[serde(default = "default_true")]
    pub notify_on_finish: bool,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}

impl BotDefinition {
    pub fn new(id: BotId, name: String, project_id: ProjectId) -> Self {
        let now = crate::model::now_secs();
        Self {
            id,
            name,
            project_id,
            enabled: true,
            agent_type: default_agent_type(),
            program: None,
            args: Vec::new(),
            driver: default_driver(),
            model: None,
            thinking_level: None,
            prompt_template: "Inspect the repository task and report findings clearly:\n{{payload}}".into(),
            soul_markdown: "# Role\nYou are an autonomous Muxlane background engineer.\n# Principles\n- Work accurately in your isolated worktree.\n- Prefer small verified edits and run relevant checks.\n- Report concise summaries with exact file paths.".into(),
            memory_markdown: String::new(),
            permission: BotPermissionProfile::AskFirst,
            isolated_worktree: true,
            max_concurrency: 1,
            timeout_secs: 600,
            triggers: vec![BotTrigger::Manual],
            notify_on_finish: true,
            created_at: now,
            updated_at: now,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_agent_type() -> AgentType {
    AgentType::Pi
}

fn default_driver() -> ChatDriverKind {
    ChatDriverKind::PiRpc
}

fn default_max_concurrency() -> u32 {
    1
}

fn default_timeout_secs() -> u64 {
    600
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BotRunStatus {
    Queued,
    Running,
    NeedsApproval,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
}

impl BotRunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            BotRunStatus::Succeeded
                | BotRunStatus::Failed
                | BotRunStatus::TimedOut
                | BotRunStatus::Cancelled
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            BotRunStatus::Queued => "queued",
            BotRunStatus::Running => "running",
            BotRunStatus::NeedsApproval => "needs_approval",
            BotRunStatus::Succeeded => "succeeded",
            BotRunStatus::Failed => "failed",
            BotRunStatus::TimedOut => "timed_out",
            BotRunStatus::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BotRunRecord {
    pub id: BotRunId,
    pub bot_id: BotId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    pub project_id: ProjectId,
    pub trigger_source: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    pub status: BotRunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
}
