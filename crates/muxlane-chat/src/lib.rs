//! muxlane-chat：结构化对话驱动（Pi RPC / 通用 ACP）、PTY 双向桥接、Transcript 增量监听与 Timeline 持久化。
pub mod acp;
pub mod agy_stream;
pub mod driver;
pub mod journal;
pub mod manager;
pub mod pi_rpc;
pub mod pty_bridge;
pub mod transcript;

pub use driver::{ChatDriverHandle, DriverCmd, DriverEvent, DriverLaunchSpec};
pub use manager::{ChatHostNotice, ChatManager};
pub use pty_bridge::PtyBridgeWriter;
