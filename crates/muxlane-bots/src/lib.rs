//! muxlane-bots: Background autonomous Bot scheduler, SOUL/MEMORY context engine,
//! git worktree isolation, Webhook/IM connectors, and permission policy orchestrator.
pub mod connectors;
pub mod cron;
pub mod manager;
pub mod soul;
pub mod worktree;

pub use manager::{decide_bot_approval, BotHostBridge, BotManager};
