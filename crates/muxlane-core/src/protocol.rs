//! muxlane wire 协议 v1：newline-JSON 帧
use crate::model::{AgentId, AgentStatus, ProjectId};
use crate::{Error, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_FRAME: usize = 1024 * 1024; // 1 MiB
pub const PROTOCOL_VERSION: u32 = 2;

/// base64 编解码快捷方式（wire 协议用）
pub fn b64_encode(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

pub fn b64_decode(s: &str) -> std::result::Result<Vec<u8>, base64::DecodeError> {
    base64::engine::general_purpose::STANDARD.decode(s)
}

/// 从原始字节流提取 OSC 标题（终端标题变化序列 `ESC ]0;title BEL`）
pub fn extract_osc_title(buf: &[u8]) -> Option<String> {
    const OSC_START: &[u8] = b"\x1b]";
    const BEL: u8 = 0x07;
    const ST: &[u8] = b"\x1b\\";
    let mut rest = buf;
    let mut last = None;
    while let Some(i) = find(rest, OSC_START) {
        let after = &rest[i + OSC_START.len()..];
        let Some(semi) = after.iter().position(|&b| b == b';') else {
            break;
        };
        let valid = matches!(after.first(), Some(b'0' | b'2'));
        let end_bel = after.iter().position(|&b| b == BEL);
        let end_st = find(after, ST);
        let Some(end) = (match (end_bel, end_st) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }) else {
            break;
        };
        if valid && semi < end {
            last = Some(String::from_utf8_lossy(&after[semi + 1..end]).into_owned());
        }
        let terminator_len = if end_bel == Some(end) { 1 } else { ST.len() };
        rest = &after[(end + terminator_len).min(after.len())..];
    }
    last
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// 去除 ANSI 转义序列（粗略），返回纯文本行（屏幕检测用）
pub fn strip_ansi(buf: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(buf);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c2 in chars.by_ref() {
                    if c2.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else if chars.peek() == Some(&'(') || chars.peek() == Some(&')') {
                // ISO-2022 字符集切换，例如 \x1b(B；跳过 ESC + 括号 + 一个字符。
                chars.next();
                chars.next();
            } else if chars.peek() == Some(&']') {
                chars.next();
                // OSC: 直到 BEL 或 ESC \\
                while let Some(&c2) = chars.peek() {
                    if c2 == '\x07' {
                        chars.next();
                        break;
                    }
                    if c2 == '\x1b' {
                        chars.next();
                        chars.next();
                        break;
                    }
                    chars.next();
                }
            }
            continue;
        }
        out.push(c);
    }
    out.lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.trim().is_empty())
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Response {
            id,
            result: Some(result),
            error: None,
        }
    }
    pub fn err(id: u64, code: &str, message: impl Into<String>) -> Self {
        Response {
            id,
            result: None,
            error: Some(RpcError {
                code: code.into(),
                message: message.into(),
                method: None,
            }),
        }
    }

    pub fn method_not_found(id: u64, method: &str) -> Self {
        Response {
            id,
            result: None,
            error: Some(RpcError {
                code: "unknown_method".into(),
                message: format!("unknown method: {method}"),
                method: Some(method.into()),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMsg {
    pub event: String,
    pub params: Value,
}

impl EventMsg {
    pub fn new(event: &str, params: Value) -> Self {
        EventMsg {
            event: event.into(),
            params,
        }
    }
}

/// 稳定的 RPC 错误码。客户端可据此分支处理，不能依赖错误文案。
pub mod error_codes {
    pub const PATH_NOT_FOUND: &str = "path_not_found";
    pub const NOT_A_DIRECTORY: &str = "not_a_directory";
    pub const CREATE_DIRECTORY_FAILED: &str = "create_directory_failed";
    pub const INVALID_PATH: &str = "invalid_path";
    pub const PERSISTENCE_FAILED: &str = "persistence_failed";
}

/// 事件名常量
pub mod events {
    pub const STATE_CHANGED: &str = "state.changed";
    pub const AGENT_STATUS: &str = "agent.status_changed";
    pub const TERM_DATA: &str = "term.data";
    pub const TERM_RESYNC: &str = "term.resync";
    pub const TERM_EXIT: &str = "term.exit";
}

/// 特性名常量
pub mod features {
    pub const PROJECT_ADD: &str = "project.add";
    pub const PROJECT_CREATE: &str = "project.create_missing";
    pub const AGENT_SPAWN: &str = "agent.spawn";
    pub const TERM_INPUT: &str = "term.input";
    pub const TERM_RESIZE: &str = "term.resize";
    /// 远端支持 agent.mark_seen：已读状态真正写回服务端（Done/Failed→Idle），
    /// 与本地终端同一语义，不依赖客户端会话内存。旧版本远端不会广播此特性，
    /// 客户端应在调用前先检查 supports()。
    pub const AGENT_MARK_SEEN: &str = "agent.mark_seen";
}
/// 方法名常量
pub mod methods {
    pub const STATE_LIST: &str = "state.list";
    pub const SYSTEM_HELLO: &str = "system.hello";
    pub const TERM_SUBSCRIBE: &str = "term.subscribe";
    pub const TERM_UNSUBSCRIBE: &str = "term.unsubscribe";
    pub const TERM_INPUT: &str = "term.input";
    pub const TERM_RESIZE: &str = "term.resize";
    pub const EVENTS_SUBSCRIBE: &str = "events.subscribe";
    pub const AGENT_REPORT: &str = "agent.report";
    pub const AGENT_SPAWN: &str = "agent.spawn";
    pub const AGENT_DELETE: &str = "agent.delete";
    pub const AGENT_MARK_SEEN: &str = "agent.mark_seen";
    pub const PROJECT_ADD: &str = "project.add";
    pub const PROJECT_DELETE: &str = "project.delete";
    pub const PAIR_BEGIN: &str = "pair.begin";
}

// ── 方法参数/结果 ─────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloResult {
    pub version: String,
    pub protocol: u32,
    pub features: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermSubscribeParams {
    pub agent: AgentId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermSubscribeResult {
    pub sub_id: String,
    /// base64 的回放快照（历史字节，客户端喂入 Term 快进）
    pub replay_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermInputParams {
    pub agent: AgentId,
    pub data_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermResizeParams {
    pub agent: AgentId,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermDataEvent {
    pub agent: AgentId,
    /// base64 PTY 字节增量
    pub data_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermResyncEvent {
    pub agent: AgentId,
    pub replay_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStatusEvent {
    pub agent: AgentId,
    /// Identity at emission time; older peers may omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<crate::model::AgentType>,
    pub from: AgentStatus,
    pub to: AgentStatus,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDeleteParams {
    pub agent: AgentId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMarkSeenParams {
    pub agent: AgentId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpawnParams {
    pub project: ProjectId,
    #[serde(default)]
    pub agent_type: Option<crate::model::AgentType>,
    #[serde(default)]
    pub program: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub env: Option<Vec<(String, String)>>,
    #[serde(default)]
    pub preset_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectAddParams {
    pub path: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub create_if_missing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectDeleteParams {
    pub project: ProjectId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteScopeResult {
    pub destroyed_agents: Vec<AgentId>,
    #[serde(default)]
    pub failed_agents: Vec<AgentId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentReportParams {
    pub token: String,
    pub agent: AgentId,
    /// "done" | "blocked" | "working"
    pub event: String,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermExitEvent {
    pub agent: AgentId,
}

// ── newline-JSON 帧编解码 ─────────────────────

/// 从 AsyncBufRead 读一行 JSON 帧。
pub async fn read_frame<R>(reader: &mut R) -> Result<Value>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    use tokio::io::{AsyncBufReadExt, AsyncReadExt};
    let mut line = Vec::with_capacity(256);
    let read = reader
        .take((MAX_FRAME + 2) as u64)
        .read_until(b'\n', &mut line)
        .await?;
    if read == 0 {
        return Err(Error::Eof);
    }
    let terminated = line.last() == Some(&b'\n');
    if terminated {
        line.pop();
    }
    if line.len() > MAX_FRAME {
        return Err(Error::FrameTooLarge {
            size: line.len(),
            max: MAX_FRAME,
        });
    }
    if !terminated {
        return Err(Error::Eof);
    }
    if line.is_empty() {
        // 空行跳过由 caller 处理；这里视为协议错误
        return Err(Error::Json(serde_json::Error::io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "empty frame",
        ))));
    }
    Ok(serde_json::from_slice(&line)?)
}

/// 写一行 JSON 帧（自动追加 \n）
pub async fn write_frame<W>(writer: &mut W, value: &impl Serialize) -> Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt;
    let mut buf = serde_json::to_vec(value)?;
    if buf.len() > MAX_FRAME {
        return Err(Error::FrameTooLarge {
            size: buf.len(),
            max: MAX_FRAME,
        });
    }
    buf.push(b'\n');
    writer.write_all(&buf).await?;
    writer.flush().await?;
    Ok(())
}

/// 同步版本（供 hook 脚本协议测试等）
pub fn encode_line(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut buf = serde_json::to_vec(value)?;
    buf.push(b'\n');
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Snapshot;

    #[test]
    fn project_add_defaults_create_if_missing_to_false() {
        let params: ProjectAddParams =
            serde_json::from_value(serde_json::json!({"path": "/tmp/project"})).unwrap();
        assert!(!params.create_if_missing);
    }

    #[test]
    fn osc_title_extraction() {
        let data = b"\x1b]0;my title\x07some output\x1b]2;second\x1b\\more";
        assert_eq!(extract_osc_title(data).as_deref(), Some("second"));
    }

    #[test]
    fn strip_ansi_basic() {
        let data =
            b"\x1b[31mHello\x1b[0m world\r\n\x1b(B$ \r\n\x1b]0;title\x07\x1b[1;32mnext line\x1b[0m";
        let lines = strip_ansi(data);
        assert_eq!(lines, vec!["Hello world", "$", "next line"]);
    }

    #[test]
    fn b64_roundtrip() {
        let s = b"hello \xe4\xbd\xa0\xe5\xa5\xbd";
        assert_eq!(b64_decode(&b64_encode(s)).unwrap(), s);
    }
    #[tokio::test]
    async fn frame_roundtrip() {
        let mut cursor =
            tokio::io::BufReader::new(std::io::Cursor::new(b"{\"a\":1}\n{\"b\":[2,3]}\n".to_vec()));
        let v1 = read_frame(&mut cursor).await.unwrap();
        assert_eq!(v1["a"], 1);
        let v2 = read_frame(&mut cursor).await.unwrap();
        assert_eq!(v2["b"][1], 3);
        let eof = read_frame(&mut cursor).await;
        assert!(matches!(eof, Err(Error::Eof)));
    }

    #[tokio::test]
    async fn request_response_roundtrip() {
        let req = Request {
            id: 7,
            method: methods::STATE_LIST.into(),
            params: Value::Null,
        };
        let mut buf = Vec::new();
        {
            let mut w = tokio::io::BufWriter::new(&mut buf);
            write_frame(&mut w, &req).await.unwrap();
        }
        let mut r = tokio::io::BufReader::new(std::io::Cursor::new(buf));
        let v = read_frame(&mut r).await.unwrap();
        let back: Request = serde_json::from_value(v).unwrap();
        assert_eq!(back.id, 7);
        assert_eq!(back.method, "state.list");
    }

    #[tokio::test]
    async fn snapshot_in_response() {
        let snap = Snapshot::default();
        let resp = Response::ok(1, serde_json::to_value(&snap).unwrap());
        let line = encode_line(&resp).unwrap();
        let mut r = tokio::io::BufReader::new(std::io::Cursor::new(line));
        let v = read_frame(&mut r).await.unwrap();
        let back: Response = serde_json::from_value(v).unwrap();
        let snap2: Snapshot = serde_json::from_value(back.result.unwrap()).unwrap();
        assert_eq!(snap2, Snapshot::default());
    }

    #[tokio::test]
    async fn oversized_write_rejected() {
        let mut output = Vec::new();
        let error = write_frame(&mut output, &"x".repeat(MAX_FRAME))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            Error::FrameTooLarge { size, max } if size > max && max == MAX_FRAME
        ));
    }

    #[tokio::test]
    async fn oversized_frame_rejected() {
        let big = "x".repeat(MAX_FRAME + 10);
        let data = format!("\"{}\"\n", big);
        let mut r = tokio::io::BufReader::new(std::io::Cursor::new(data.into_bytes()));
        let err = read_frame(&mut r).await.unwrap_err();
        assert!(matches!(err, Error::FrameTooLarge { .. }));
    }
}
