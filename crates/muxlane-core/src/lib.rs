//! muxlane-core：协议、数据模型、状态检测、hook 注入
pub mod auth;
pub mod bot;
pub mod chat;
pub mod detect;
pub mod e2ee;
mod error;
pub mod hook;
pub mod model;
pub mod pane;
pub mod paths;
pub mod preset;
pub mod protocol;

pub use error::{Error, Result};

pub use bot::{
    BotDefinition, BotId, BotPermissionProfile, BotRunId, BotRunRecord, BotRunStatus, BotTrigger,
    ImConnectorConfig, ImPlatform,
};

pub use chat::{
    BackgroundJob, ChatAttachment, ChatDeltaEvent, ChatDeltaOp, ChatDriverKind, ChatModelInfo,
    ChatSessionMeta, ChatSlashCommand, ChatTimelineState, MessageUsage, PendingApproval,
    QueuedMessage, SessionDriverMode, SessionGoal, SessionStats, SessionSubagent, SessionTask,
    SessionViewMode, TimelineEntry,
};
pub use detect::DetectionEngine;
pub use model::{
    AgentId, AgentInstance, AgentStatus, AgentType, MachineId, MachineInfo, Project, ProjectId,
    Snapshot,
};

pub use auth::AuthSecret;
pub use pane::{PaneId, PaneNode, SplitAxis, TabGroup};
pub use preset::{builtin_presets, AgentPreset};
