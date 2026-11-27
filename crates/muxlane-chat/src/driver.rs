//! 统一结构化 Agent Chat 驱动抽象（Pi RPC / 通用 ACP 共用命令与事件信道）。
use anyhow::Result;
use muxlane_core::chat::{
    ApprovalReply, ChatAttachment, ChatDeltaOp, ChatDriverKind, StreamingBehavior,
};
use muxlane_core::model::{AgentId, AgentStatus, AgentType};
use std::path::PathBuf;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct DriverLaunchSpec {
    pub agent: AgentId,
    pub agent_type: AgentType,
    pub driver_kind: ChatDriverKind,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub provider_session: Option<String>,
    pub model: Option<String>,
    pub thinking_level: Option<String>,
}

#[derive(Debug)]
pub enum DriverCmd {
    SendPrompt {
        text: String,
        attachments: Vec<ChatAttachment>,
        streaming_behavior: Option<StreamingBehavior>,
    },
    Abort,
    DeleteQueuedMessage {
        message_id: Option<String>,
    },
    RespondApproval {
        approval_id: String,
        reply: ApprovalReply,
    },
    SetModel {
        provider: Option<String>,
        model_id: String,
    },
    SetThinkingLevel {
        level: String,
    },
    Shutdown,
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum DriverEvent {
    Delta(ChatDeltaOp),
    Status {
        status: AgentStatus,
        message: Option<String>,
        reset_seen: bool,
    },
    Title(String),
    Exited {
        error: Option<String>,
    },
}

#[derive(Clone)]
pub struct ChatDriverHandle {
    tx: mpsc::UnboundedSender<DriverCmd>,
}

impl ChatDriverHandle {
    pub fn new(tx: mpsc::UnboundedSender<DriverCmd>) -> Self {
        Self { tx }
    }

    pub fn send(&self, cmd: DriverCmd) -> Result<()> {
        self.tx
            .send(cmd)
            .map_err(|_| anyhow::anyhow!("chat driver process is no longer running"))
    }
}

/// 严格按 LF (\n) 读取一行 JSON，去掉可选前置 CR (\r)。
/// 避免通用按 Unicode 行分隔符拆行导致 JSON 字符串内 U+2028/U+2029 被错误截断。
pub async fn read_lf_line<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<String>> {
    use tokio::io::AsyncBufReadExt;
    let mut buf = Vec::with_capacity(512);
    let n = reader.read_until(b'\n', &mut buf).await?;
    if n == 0 {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

pub fn extract_text_blocks(value: &serde_json::Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = value.as_array() {
        let mut parts = Vec::new();
        for item in arr {
            if item.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                    parts.push(t.to_string());
                }
            }
        }
        if !parts.is_empty() {
            return Some(parts.join("\n"));
        }
    }
    if let Some(arr) = value.get("content").and_then(|v| v.as_array()) {
        return extract_text_blocks(&serde_json::Value::Array(arr.clone()));
    }
    if let Some(s) = value.get("content").and_then(|v| v.as_str()) {
        return Some(s.to_string());
    }
    None
}

/// Derive a clean single-line session title from the first non-slash user prompt.
pub fn derive_prompt_title(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('/') && !trimmed.starts_with("/skill:") {
        return None;
    }
    let first_line = trimmed
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let collapsed: String = first_line
        .chars()
        .filter(|ch| !ch.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.is_empty() {
        return None;
    }
    let mut chars = collapsed.chars();
    let head: String = chars.by_ref().take(48).collect();
    if chars.next().is_some() {
        Some(format!("{head}…"))
    } else {
        Some(head)
    }
}
