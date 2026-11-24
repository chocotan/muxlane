//! muxlane-chat：结构化对话驱动（Pi RPC / 通用 ACP）、Transcript 解析与 Timeline 持久化。
pub mod acp;
pub mod driver;
pub mod journal;
pub mod manager;
pub mod pi_rpc;
pub mod transcript;

pub use driver::{ChatDriverHandle, DriverCmd, DriverEvent, DriverLaunchSpec};
pub use manager::{ChatHostNotice, ChatManager};
