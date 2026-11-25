//! Safe PTY write bridge (Orca-style Lane A): sanitizes paste payloads, wraps multiline/long
//! input in Bracketed Paste (`\x1b[200~...\x1b[201~`), submits `\r` after a TUI settle delay,
//! and verifies live agent confirmation so chat text is never injected into a bare shell.
use base64::Engine as _;
use muxlane_core::chat::{ApprovalReply, ChatAttachment};
use muxlane_core::model::{AgentId, AgentType};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

pub const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
pub const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";
pub const BRACKETED_PASTE_THRESHOLD_BYTES: usize = 64;
pub const SUBMIT_ENTER_DELAY: Duration = Duration::from_millis(60);
pub const ATTACHMENT_SETTLE_DELAY: Duration = Duration::from_millis(30);

/// Server-implemented bridge for safely writing chat prompts/approvals/aborts into a live PTY.
pub trait PtyBridgeWriter: Send + Sync {
    /// Returns `true` ONLY when a live PTY exists and its confirmed foreground process is an
    /// interactive AI agent (never a bare shell).
    fn is_agent_confirmed(&self, agent: &AgentId) -> Pin<Box<dyn Future<Output = bool> + Send>>;

    /// Writes raw bytes into the agent's live PTY stdin.
    fn write_bytes(
        &self,
        agent: &AgentId,
        bytes: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;

    /// Notifies the host server that a chat turn was submitted into the PTY.
    fn notify_submitted(&self, agent: &AgentId) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

/// Normalizes line endings and strips ANSI escape sequences and C0/C1 control characters
/// (neutralizing embedded `\x1b[201~` bracketed-paste breakout attempts) while preserving
/// `\n`, `\t`, and all printable Unicode text.
pub fn sanitize_pty_paste_text(raw: &str) -> String {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(normalized.len());
    let mut chars = normalized.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Strip CSI sequences (`ESC [ ... <final>`) such as `\x1b[201~` or SGR codes.
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            } else if chars.peek() == Some(&']') {
                // Strip OSC sequences (`ESC ] ... BEL` or `ESC \`).
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            continue;
        }
        let code = ch as u32;
        if ch == '\n'
            || ch == '\t'
            || (code >= 0x20 && code != 0x7f && !(0x80..=0x9f).contains(&code))
        {
            out.push(ch);
        }
    }
    out
}

/// Frames sanitized text with Bracketed Paste when multiline or longer than 64 bytes.
pub fn build_pty_paste_payload(sanitized: &str) -> Vec<u8> {
    if sanitized.is_empty() {
        return Vec::new();
    }
    if sanitized.contains('\n') || sanitized.len() > BRACKETED_PASTE_THRESHOLD_BYTES {
        let mut buf = Vec::with_capacity(
            BRACKETED_PASTE_START.len() + sanitized.len() + BRACKETED_PASTE_END.len(),
        );
        buf.extend_from_slice(BRACKETED_PASTE_START);
        buf.extend_from_slice(sanitized.as_bytes());
        buf.extend_from_slice(BRACKETED_PASTE_END);
        buf
    } else {
        sanitized.as_bytes().to_vec()
    }
}

/// Always frames attachment file paths in Bracketed Paste so TUI agents detect them cleanly.
pub fn build_attachment_paste_payload(paths: &[String]) -> Vec<u8> {
    let cleaned: Vec<String> = paths
        .iter()
        .map(|p| sanitize_pty_paste_text(p).trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if cleaned.is_empty() {
        return Vec::new();
    }
    let joined = format!("{} ", cleaned.join(" "));
    let mut buf =
        Vec::with_capacity(BRACKETED_PASTE_START.len() + joined.len() + BRACKETED_PASTE_END.len());
    buf.extend_from_slice(BRACKETED_PASTE_START);
    buf.extend_from_slice(joined.as_bytes());
    buf.extend_from_slice(BRACKETED_PASTE_END);
    buf
}

fn extension_for_mime(mime: &str) -> &'static str {
    let lower = mime.to_ascii_lowercase();
    if lower.contains("png") {
        "png"
    } else if lower.contains("jpeg") || lower.contains("jpg") {
        "jpg"
    } else if lower.contains("webp") {
        "webp"
    } else if lower.contains("gif") {
        "gif"
    } else if lower.contains("pdf") {
        "pdf"
    } else if lower.contains("text") {
        "txt"
    } else {
        "bin"
    }
}

/// Resolves an attachment to a local filesystem path, materializing inline base64 images to a
/// temporary file so CLI TUI agents can ingest them via pasted path.
pub fn materialize_attachment_path(att: &ChatAttachment) -> Option<String> {
    if let Some(path) = att.path.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return Some(path.to_string());
    }
    let b64 = att.data_b64.as_deref()?.trim();
    if b64.is_empty() {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let ext = extension_for_mime(&att.mime_type);
    let path =
        std::env::temp_dir().join(format!("muxlane-chat-attach-{}.{ext}", ulid::Ulid::new()));
    std::fs::write(&path, bytes).ok()?;
    Some(path.display().to_string())
}

/// Sends a chat prompt (plus any attachments) into a confirmed interactive agent PTY.
pub async fn submit_prompt_to_pty(
    writer: &dyn PtyBridgeWriter,
    agent: &AgentId,
    text: &str,
    attachments: &[ChatAttachment],
) -> anyhow::Result<()> {
    if !writer.is_agent_confirmed(agent).await {
        anyhow::bail!("terminal session returned to bare shell; chat input disabled");
    }

    let sanitized = sanitize_pty_paste_text(text);
    let trimmed = sanitized.trim();
    let attachment_paths: Vec<String> = attachments
        .iter()
        .filter_map(materialize_attachment_path)
        .collect();

    if trimmed.is_empty() && attachment_paths.is_empty() {
        return Ok(());
    }

    if !attachment_paths.is_empty() {
        let att_payload = build_attachment_paste_payload(&attachment_paths);
        if !att_payload.is_empty() {
            writer.write_bytes(agent, att_payload).await?;
            if !trimmed.is_empty() {
                tokio::time::sleep(ATTACHMENT_SETTLE_DELAY).await;
            }
        }
    }

    if !trimmed.is_empty() {
        let payload = build_pty_paste_payload(trimmed);
        writer.write_bytes(agent, payload).await?;
    }

    tokio::time::sleep(SUBMIT_ENTER_DELAY).await;
    writer.write_bytes(agent, b"\r".to_vec()).await?;
    writer.notify_submitted(agent).await;
    Ok(())
}

/// Abort sequence tailored to the interactive TUI agent (Esc for Pi/Pie, Ctrl+C for others).
pub fn abort_bytes_for_agent(agent_type: AgentType) -> Vec<u8> {
    match agent_type {
        AgentType::Pi | AgentType::Pie => vec![0x1b],
        _ => vec![0x03],
    }
}

/// Sends an approval response into a confirmed interactive agent PTY.
pub async fn submit_approval_to_pty(
    writer: &dyn PtyBridgeWriter,
    agent: &AgentId,
    reply: &ApprovalReply,
) -> anyhow::Result<()> {
    if !writer.is_agent_confirmed(agent).await {
        anyhow::bail!("terminal session returned to bare shell; approval input disabled");
    }
    match reply {
        ApprovalReply::Confirm { confirmed: true } => {
            writer.write_bytes(agent, b"y\r".to_vec()).await?;
        }
        ApprovalReply::Confirm { confirmed: false } => {
            writer.write_bytes(agent, b"n\r".to_vec()).await?;
        }
        ApprovalReply::Cancel => {
            writer.write_bytes(agent, vec![0x1b]).await?;
        }
        ApprovalReply::SelectedOption { option_id } => {
            let lower = option_id.trim().to_ascii_lowercase();
            if lower.starts_with("allow") || lower == "y" || lower == "yes" {
                writer.write_bytes(agent, b"y\r".to_vec()).await?;
            } else if lower.starts_with("reject")
                || lower.starts_with("deny")
                || lower == "n"
                || lower == "no"
            {
                writer.write_bytes(agent, b"n\r".to_vec()).await?;
            } else {
                let sanitized = sanitize_pty_paste_text(option_id);
                let payload = build_pty_paste_payload(sanitized.trim());
                if !payload.is_empty() {
                    writer.write_bytes(agent, payload).await?;
                    tokio::time::sleep(SUBMIT_ENTER_DELAY).await;
                }
                writer.write_bytes(agent, b"\r".to_vec()).await?;
            }
        }
        ApprovalReply::Text { value } => {
            let sanitized = sanitize_pty_paste_text(value);
            let payload = build_pty_paste_payload(sanitized.trim());
            if !payload.is_empty() {
                writer.write_bytes(agent, payload).await?;
                tokio::time::sleep(SUBMIT_ENTER_DELAY).await;
            }
            writer.write_bytes(agent, b"\r".to_vec()).await?;
        }
    }
    writer.notify_submitted(agent).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    struct MockPtyWriter {
        confirmed: AtomicBool,
        writes: Mutex<Vec<Vec<u8>>>,
        notified: AtomicBool,
    }

    impl MockPtyWriter {
        fn new(confirmed: bool) -> Arc<Self> {
            Arc::new(Self {
                confirmed: AtomicBool::new(confirmed),
                writes: Mutex::new(Vec::new()),
                notified: AtomicBool::new(false),
            })
        }
    }

    impl PtyBridgeWriter for MockPtyWriter {
        fn is_agent_confirmed(
            &self,
            _agent: &AgentId,
        ) -> Pin<Box<dyn Future<Output = bool> + Send>> {
            let ok = self.confirmed.load(Ordering::Acquire);
            Box::pin(async move { ok })
        }

        fn write_bytes(
            &self,
            _agent: &AgentId,
            bytes: Vec<u8>,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> {
            self.writes.lock().unwrap().push(bytes);
            Box::pin(async { Ok(()) })
        }

        fn notify_submitted(&self, _agent: &AgentId) -> Pin<Box<dyn Future<Output = ()> + Send>> {
            self.notified.store(true, Ordering::Release);
            Box::pin(async {})
        }
    }

    #[test]
    fn sanitizes_escape_sequences_and_control_bytes() {
        let hostile = "hello\x1b[201~rm -rf /\r\n\x03next\tline\x1b[31mred\x1b[0m";
        let clean = sanitize_pty_paste_text(hostile);
        assert_eq!(clean, "hellorm -rf /\nnext\tlinered");
        assert!(!clean.contains('\x1b'));
        assert!(!clean.contains('\x03'));
        assert!(!clean.contains('\r'));
    }

    #[test]
    fn bracketed_paste_applies_to_multiline_and_long_payloads() {
        assert_eq!(build_pty_paste_payload("short prompt"), b"short prompt");
        let multiline = build_pty_paste_payload("line 1\nline 2");
        assert!(multiline.starts_with(BRACKETED_PASTE_START));
        assert!(multiline.ends_with(BRACKETED_PASTE_END));
        let long = "a".repeat(BRACKETED_PASTE_THRESHOLD_BYTES + 1);
        let framed = build_pty_paste_payload(&long);
        assert!(framed.starts_with(BRACKETED_PASTE_START));
        assert!(framed.ends_with(BRACKETED_PASTE_END));
    }

    #[tokio::test]
    async fn submit_prompt_writes_payload_then_separate_enter_and_blocks_bare_shell() {
        let writer = MockPtyWriter::new(true);
        let agent: AgentId = "pi_1".into();
        submit_prompt_to_pty(writer.as_ref(), &agent, "line one\nline two", &[])
            .await
            .unwrap();
        let writes = writer.writes.lock().unwrap().clone();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0], b"\x1b[200~line one\nline two\x1b[201~");
        assert_eq!(writes[1], b"\r");
        assert!(writer.notified.load(Ordering::Acquire));

        writer.confirmed.store(false, Ordering::Release);
        let err = submit_prompt_to_pty(writer.as_ref(), &agent, "ls", &[])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("bare shell"));
        assert_eq!(writer.writes.lock().unwrap().len(), 2);
    }

    #[test]
    fn abort_bytes_use_escape_for_pi_and_ctrl_c_for_others() {
        assert_eq!(abort_bytes_for_agent(AgentType::Pi), vec![0x1b]);
        assert_eq!(abort_bytes_for_agent(AgentType::Pie), vec![0x1b]);
        assert_eq!(abort_bytes_for_agent(AgentType::Opencode), vec![0x03]);
    }
}
