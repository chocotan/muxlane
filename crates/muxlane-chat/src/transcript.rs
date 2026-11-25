//! Pi / Pie 磁盘 JSONL Session 实时解析（用于 PTY/tmux 会话向 Chat 视图实时增量同步与续接）。
use muxlane_core::chat::{
    ChatModelInfo, ChatSessionMeta, NoticeLevel, SessionDriverMode, TimelineEntry, ToolCallKind,
    ToolCallState, ToolDiff,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const BOUNDARY_WINDOW_BYTES: u64 = 512;

/// 按 Pi 官方规范编码项目路径：`--<path>--`，去掉前导分隔符并将 `/`, `\`, `:` 替换为 `-`。
pub fn pi_session_dir_name(cwd: &Path) -> String {
    let raw = cwd.to_string_lossy();
    let trimmed = raw.trim_start_matches(['/', '\\']);
    let safe: String = trimmed
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' => '-',
            other => other,
        })
        .collect();
    format!("--{safe}--")
}

pub fn pi_sessions_root(is_pie: bool) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(
        home.join(if is_pie { ".pie" } else { ".pi" })
            .join("agent/sessions"),
    )
}

/// 根据 cwd 与启动时间戳发现匹配的 Pi session JSONL 文件。
pub fn discover_pi_session_file(
    cwd: &Path,
    is_pie: bool,
    started_after_unix_secs: u64,
) -> Option<PathBuf> {
    let root = pi_sessions_root(is_pie)?;
    let dir = root.join(pi_session_dir_name(cwd));
    let entries = std::fs::read_dir(&dir).ok()?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let mod_secs = modified
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if mod_secs + 2 < started_after_unix_secs {
            continue;
        }
        if best.as_ref().is_none_or(|(prev, _)| modified > *prev) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, path)| path)
}

#[derive(Debug, Clone)]
struct PiTranscriptParserState {
    meta: ChatSessionMeta,
    tool_calls: HashMap<String, TimelineEntry>,
}

impl PiTranscriptParserState {
    fn new(path: &Path) -> Self {
        Self {
            meta: ChatSessionMeta {
                driver: SessionDriverMode::TerminalPty,
                supports_chat_input: false,
                supports_terminal_switch: true,
                has_pty: true,
                provider_session_file: Some(path.display().to_string()),
                ..Default::default()
            },
            tool_calls: HashMap::new(),
        }
    }

    fn parse_record(
        &mut self,
        record: &Value,
        live_tools_running: bool,
    ) -> (bool, Vec<TimelineEntry>) {
        let mut meta_changed = false;
        let mut out = Vec::new();
        let kind = record.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "session" => {
                if let Some(id) = record.get("id").and_then(|v| v.as_str()) {
                    if self.meta.provider_session_id.as_deref() != Some(id) {
                        self.meta.provider_session_id = Some(id.to_string());
                        meta_changed = true;
                    }
                }
            }
            "model_change" => {
                let provider = record
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let model_id = record
                    .get("modelId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !model_id.is_empty() {
                    let next = Some(ChatModelInfo {
                        provider,
                        id: model_id.clone(),
                        name: model_id,
                    });
                    if self.meta.current_model != next {
                        self.meta.current_model = next;
                        meta_changed = true;
                    }
                }
            }
            "thinking_level_change" => {
                if let Some(level) = record.get("thinkingLevel").and_then(|v| v.as_str()) {
                    let next = Some(level.to_string());
                    if self.meta.thinking_level != next {
                        self.meta.thinking_level = next;
                        meta_changed = true;
                    }
                }
            }
            "compaction" => {
                let id = record
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("compaction")
                    .to_string();
                let summary = record
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Context compacted")
                    .to_string();
                out.push(TimelineEntry::Notice {
                    id: format!("compact-{id}"),
                    turn_id: None,
                    level: NoticeLevel::Compaction,
                    text: summary,
                    created_at_ms: parse_timestamp_ms(record.get("timestamp")),
                });
            }
            "message" => {
                let entry_id = record
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("msg")
                    .to_string();
                let Some(message) = record.get("message") else {
                    return (meta_changed, out);
                };
                let role = message.get("role").and_then(|v| v.as_str()).unwrap_or("");
                let ts = message
                    .get("timestamp")
                    .and_then(|v| v.as_u64())
                    .unwrap_or_else(|| parse_timestamp_ms(record.get("timestamp")));
                match role {
                    "user" => {
                        let text = extract_message_text(message);
                        if !text.is_empty() {
                            out.push(TimelineEntry::UserMessage {
                                id: format!("pi-user-{entry_id}"),
                                turn_id: None,
                                text,
                                attachments: Vec::new(),
                                created_at_ms: ts,
                            });
                        }
                    }
                    "assistant" => {
                        let model = message
                            .get("model")
                            .and_then(|v| v.as_str())
                            .map(str::to_string);
                        if let Some(m) = &model {
                            let next = Some(ChatModelInfo {
                                provider: message
                                    .get("provider")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                id: m.clone(),
                                name: m.clone(),
                            });
                            if self.meta.current_model != next {
                                self.meta.current_model = next;
                                meta_changed = true;
                            }
                        }
                        if let Some(parts) = message.get("content").and_then(|v| v.as_array()) {
                            for (idx, part) in parts.iter().enumerate() {
                                let part_type =
                                    part.get("type").and_then(|v| v.as_str()).unwrap_or("");
                                match part_type {
                                    "thinking" => {
                                        let thinking = part
                                            .get("thinking")
                                            .or_else(|| part.get("text"))
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        if !thinking.is_empty() {
                                            out.push(TimelineEntry::Thinking {
                                                id: format!("pi-think-{entry_id}-{idx}"),
                                                turn_id: None,
                                                text: thinking,
                                                streaming: false,
                                                duration_ms: None,
                                                created_at_ms: ts,
                                            });
                                        }
                                    }
                                    "text" => {
                                        let text = part
                                            .get("text")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        if !text.is_empty() {
                                            out.push(TimelineEntry::AssistantText {
                                                id: format!("pi-text-{entry_id}-{idx}"),
                                                turn_id: None,
                                                text,
                                                streaming: false,
                                                model: model.clone(),
                                                created_at_ms: ts,
                                            });
                                        }
                                    }
                                    "toolCall" => {
                                        let call_id = part
                                            .get("id")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or(&entry_id)
                                            .to_string();
                                        let name = part
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("tool")
                                            .to_string();
                                        let input =
                                            part.get("arguments").cloned().unwrap_or(Value::Null);
                                        let diff = infer_tool_diff(&name, &input).map(Box::new);
                                        let tool_entry = TimelineEntry::ToolCall {
                                            id: format!("tool:{call_id}"),
                                            turn_id: None,
                                            call_id: call_id.clone(),
                                            kind: ToolCallKind::from_tool_name(&name, None),
                                            name,
                                            title: None,
                                            input,
                                            output: None,
                                            diff,
                                            exit_code: None,
                                            locations: Vec::new(),
                                            state: if live_tools_running {
                                                ToolCallState::Running
                                            } else {
                                                ToolCallState::Completed
                                            },
                                            is_error: false,
                                            duration_ms: None,
                                            created_at_ms: ts,
                                        };
                                        self.tool_calls.insert(call_id, tool_entry.clone());
                                        out.push(tool_entry);
                                    }
                                    _ => {}
                                }
                            }
                        } else {
                            let text = extract_message_text(message);
                            if !text.is_empty() {
                                out.push(TimelineEntry::AssistantText {
                                    id: format!("pi-text-{entry_id}"),
                                    turn_id: None,
                                    text,
                                    streaming: false,
                                    model,
                                    created_at_ms: ts,
                                });
                            }
                        }
                    }
                    "toolResult" => {
                        let call_id = message
                            .get("toolCallId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let output_text = extract_message_text(message);
                        let is_error = message
                            .get("isError")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if let Some(existing) = self.tool_calls.get_mut(call_id) {
                            if let TimelineEntry::ToolCall {
                                output,
                                is_error: err_slot,
                                state,
                                ..
                            } = existing
                            {
                                *output = Some(output_text);
                                *err_slot = is_error;
                                *state = if is_error {
                                    ToolCallState::Failed
                                } else {
                                    ToolCallState::Completed
                                };
                            }
                            out.push(existing.clone());
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        (meta_changed, out)
    }
}

/// Output of a single incremental poll cycle on a Pi/Pie JSONL session transcript.
#[derive(Debug, Default)]
pub struct TranscriptPoll {
    /// True if the file was truncated, rewritten, replaced, or switched to a different session file.
    pub reset: bool,
    pub meta_changed: bool,
    pub meta: ChatSessionMeta,
    pub upserted_entries: Vec<TimelineEntry>,
}

/// Orca-style byte-offset incremental JSONL transcript watcher with boundary SHA-256 fingerprint
/// validation to detect in-place truncation, rotation, rewrite, or session file switches.
pub struct IncrementalPiTranscript {
    file_path: PathBuf,
    offset: u64,
    partial_line: String,
    boundary_hash: Option<[u8; 32]>,
    pending_reset: bool,
    parser: PiTranscriptParserState,
}

impl IncrementalPiTranscript {
    pub fn new(file_path: PathBuf) -> Self {
        let parser = PiTranscriptParserState::new(&file_path);
        Self {
            file_path,
            offset: 0,
            partial_line: String::new(),
            boundary_hash: None,
            pending_reset: false,
            parser,
        }
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// Switches tracking to a new JSONL file (e.g. after `/new` or `/resume` inside Pi TUI).
    pub fn switch_file(&mut self, new_path: PathBuf) {
        if self.file_path == new_path {
            return;
        }
        self.file_path = new_path.clone();
        self.offset = 0;
        self.partial_line.clear();
        self.boundary_hash = None;
        self.pending_reset = true;
        self.parser = PiTranscriptParserState::new(&new_path);
    }

    fn compute_boundary_hash(file: &mut File, offset: u64) -> std::io::Result<Option<[u8; 32]>> {
        if offset == 0 {
            return Ok(None);
        }
        let start = offset.saturating_sub(BOUNDARY_WINDOW_BYTES);
        let len = (offset - start) as usize;
        file.seek(SeekFrom::Start(start))?;
        let mut buf = vec![0u8; len];
        file.read_exact(&mut buf)?;
        let digest = Sha256::digest(&buf);
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        Ok(Some(out))
    }

    pub fn poll_deltas(&mut self) -> anyhow::Result<TranscriptPoll> {
        let mut file = match File::open(&self.file_path) {
            Ok(f) => f,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(TranscriptPoll::default());
            }
            Err(err) => return Err(err.into()),
        };
        let file_len = file.metadata()?.len();

        let fingerprint_mismatch = if self.offset > 0 && file_len >= self.offset {
            match (
                self.boundary_hash,
                Self::compute_boundary_hash(&mut file, self.offset)?,
            ) {
                (Some(prev), Some(curr)) => prev != curr,
                _ => false,
            }
        } else {
            false
        };

        let is_reset = self.pending_reset || file_len < self.offset || fingerprint_mismatch;
        if is_reset {
            self.offset = 0;
            self.partial_line.clear();
            self.boundary_hash = None;
            self.pending_reset = false;
            self.parser = PiTranscriptParserState::new(&self.file_path);
        } else if file_len == self.offset {
            return Ok(TranscriptPoll::default());
        }

        let initial_load = self.offset == 0 && !is_reset;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut chunk = Vec::with_capacity((file_len - self.offset) as usize);
        file.read_to_end(&mut chunk)?;
        self.offset += chunk.len() as u64;
        self.boundary_hash = Self::compute_boundary_hash(&mut file, self.offset)?;

        let chunk_str = String::from_utf8_lossy(&chunk);
        self.partial_line.push_str(&chunk_str);

        let mut complete_lines = Vec::new();
        while let Some(newline_pos) = self.partial_line.find('\n') {
            let line = self.partial_line[..newline_pos].trim().to_string();
            self.partial_line.drain(..=newline_pos);
            if !line.is_empty() {
                complete_lines.push(line);
            }
        }
        // Also flush a trailing complete JSON object if writer omitted trailing newline.
        let trailing_trimmed = self.partial_line.trim();
        if trailing_trimmed.starts_with('{')
            && trailing_trimmed.ends_with('}')
            && serde_json::from_str::<Value>(trailing_trimmed).is_ok()
        {
            complete_lines.push(trailing_trimmed.to_string());
            self.partial_line.clear();
        }

        let mut poll = TranscriptPoll {
            reset: is_reset,
            meta_changed: initial_load || is_reset,
            meta: self.parser.meta.clone(),
            upserted_entries: Vec::new(),
        };

        for line in complete_lines {
            let Ok(record) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let (mc, entries) = self.parser.parse_record(&record, true);
            if mc {
                poll.meta_changed = true;
            }
            poll.upserted_entries.extend(entries);
        }
        poll.meta = self.parser.meta.clone();
        Ok(poll)
    }
}

/// 全量解析一个 Pi session `.jsonl` 文件为统一的 `TimelineEntry` 列表与会话元数据。
pub fn parse_pi_session_file(path: &Path) -> anyhow::Result<(ChatSessionMeta, Vec<TimelineEntry>)> {
    let raw = std::fs::read_to_string(path)?;
    let mut parser = PiTranscriptParserState::new(path);
    let mut entries: Vec<TimelineEntry> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let (_, emitted) = parser.parse_record(&record, false);
        for entry in emitted {
            if let Some(&pos) = index.get(entry.id()) {
                entries[pos] = entry;
            } else {
                let pos = entries.len();
                index.insert(entry.id().to_string(), pos);
                entries.push(entry);
            }
        }
    }

    Ok((parser.meta, entries))
}

fn parse_timestamp_ms(val: Option<&Value>) -> u64 {
    val.and_then(|v| v.as_u64()).unwrap_or(0)
}

pub(crate) fn extract_message_text(message: &Value) -> String {
    if let Some(s) = message.get("content").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(arr) = message.get("content").and_then(|v| v.as_array()) {
        let mut out = Vec::new();
        for item in arr {
            if item.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                    out.push(t.to_string());
                }
            }
        }
        return out.join("\n");
    }
    String::new()
}

pub(crate) fn infer_tool_diff(name: &str, input: &Value) -> Option<ToolDiff> {
    let lower = name.to_ascii_lowercase();
    let path = input
        .get("path")
        .or_else(|| input.get("file_path"))
        .or_else(|| input.get("filePath"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if lower.contains("edit") || lower.contains("replace") {
        let old = input
            .get("oldText")
            .or_else(|| input.get("old_string"))
            .or_else(|| input.get("oldString"))
            .and_then(|v| v.as_str())?
            .to_string();
        let new = input
            .get("newText")
            .or_else(|| input.get("new_string"))
            .or_else(|| input.get("newString"))
            .and_then(|v| v.as_str())?
            .to_string();
        return Some(ToolDiff {
            path,
            old_text: old,
            new_text: new,
        });
    }
    if lower.contains("write") {
        if let Some(content) = input.get("content").and_then(|v| v.as_str()) {
            return Some(ToolDiff {
                path,
                old_text: String::new(),
                new_text: content.to_string(),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn encodes_pi_session_dir_consistently() {
        assert_eq!(
            pi_session_dir_name(Path::new("/home/choco/Downloads/muxlane")),
            "--home-choco-Downloads-muxlane--"
        );
    }

    #[test]
    fn parses_pi_session_jsonl_entries_and_tool_results() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.jsonl");
        let lines = [
            r#"{"type":"session","version":3,"id":"sess-1","timestamp":"2026-11-23T00:00:00Z","cwd":"/tmp/app"}"#,
            r#"{"type":"message","id":"u1","parentId":null,"timestamp":"2026-11-23T00:00:01Z","message":{"role":"user","content":"Check Cargo.toml","timestamp":1000}}"#,
            r#"{"type":"message","id":"a1","parentId":"u1","timestamp":"2026-11-23T00:00:02Z","message":{"role":"assistant","model":"sonnet","content":[{"type":"thinking","thinking":"Inspecting file"},{"type":"toolCall","id":"tc1","name":"read","arguments":{"path":"Cargo.toml"}},{"type":"text","text":"Looks good."}],"timestamp":2000}}"#,
            r#"{"type":"message","id":"r1","parentId":"a1","timestamp":"2026-11-23T00:00:03Z","message":{"role":"toolResult","toolCallId":"tc1","toolName":"read","content":[{"type":"text","text":"[package]\nname = \"demo\""}],"isError":false,"timestamp":3000}}"#,
        ];
        std::fs::write(&file, lines.join("\n")).unwrap();
        let (meta, entries) = parse_pi_session_file(&file).unwrap();
        assert_eq!(meta.provider_session_id.as_deref(), Some("sess-1"));
        assert_eq!(entries.len(), 4);
        match &entries[2] {
            TimelineEntry::ToolCall {
                output, is_error, ..
            } => {
                assert_eq!(output.as_deref(), Some("[package]\nname = \"demo\""));
                assert!(!is_error);
            }
            other => panic!("unexpected tool entry: {other:?}"),
        }
    }

    #[test]
    fn incremental_watcher_handles_append_partial_lines_tool_results_rewrite_and_file_switch() {
        let dir = tempfile::tempdir().unwrap();
        let file1 = dir.path().join("s1.jsonl");
        std::fs::write(
            &file1,
            concat!(
                r#"{"type":"session","version":3,"id":"sess-1"}"#,
                "\n",
                r#"{"type":"message","id":"u1","message":{"role":"user","content":"Hi Pi","timestamp":100}}"#,
                "\n"
            ),
        )
        .unwrap();

        let mut watcher = IncrementalPiTranscript::new(file1.clone());
        let first = watcher.poll_deltas().unwrap();
        assert!(!first.reset);
        assert_eq!(first.meta.provider_session_id.as_deref(), Some("sess-1"));
        assert_eq!(first.upserted_entries.len(), 1);

        // No-op poll emits nothing.
        let idle = watcher.poll_deltas().unwrap();
        assert!(!idle.reset);
        assert!(idle.upserted_entries.is_empty());

        // Partial line write is buffered without corrupting JSON.
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&file1)
                .unwrap();
            f.write_all(br#"{"type":"message","id":"a1","message":{"role":"assistant","content":[{"type":"toolCall","id":"tc1","name":"bash","arguments":{"command":"ls"}}"#).unwrap();
        }
        let partial = watcher.poll_deltas().unwrap();
        assert!(partial.upserted_entries.is_empty());

        // Complete the line -> toolCall emits as Running.
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&file1)
                .unwrap();
            f.write_all(b"]}}\n").unwrap();
        }
        let tool_started = watcher.poll_deltas().unwrap();
        assert_eq!(tool_started.upserted_entries.len(), 1);
        assert!(matches!(
            &tool_started.upserted_entries[0],
            TimelineEntry::ToolCall {
                state: ToolCallState::Running,
                ..
            }
        ));

        // Append toolResult -> upserts same tool entry as Completed with output.
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&file1)
                .unwrap();
            f.write_all(
                concat!(
                    r#"{"type":"message","id":"r1","message":{"role":"toolResult","toolCallId":"tc1","content":"Cargo.toml","isError":false}}"#,
                    "\n"
                )
                .as_bytes(),
            )
            .unwrap();
        }
        let tool_done = watcher.poll_deltas().unwrap();
        assert_eq!(tool_done.upserted_entries.len(), 1);
        match &tool_done.upserted_entries[0] {
            TimelineEntry::ToolCall {
                id,
                output,
                state,
                is_error,
                ..
            } => {
                assert_eq!(id, "tool:tc1");
                assert_eq!(output.as_deref(), Some("Cargo.toml"));
                assert_eq!(*state, ToolCallState::Completed);
                assert!(!is_error);
            }
            other => panic!("unexpected entry: {other:?}"),
        }

        // In-place rewrite/truncation triggers full reset via size/fingerprint check.
        std::fs::write(
            &file1,
            concat!(
                r#"{"type":"session","version":3,"id":"sess-rewritten"}"#,
                "\n",
                r#"{"type":"message","id":"u2","message":{"role":"user","content":"Fresh start"}}"#,
                "\n"
            ),
        )
        .unwrap();
        let reset_poll = watcher.poll_deltas().unwrap();
        assert!(reset_poll.reset);
        assert_eq!(
            reset_poll.meta.provider_session_id.as_deref(),
            Some("sess-rewritten")
        );
        assert_eq!(reset_poll.upserted_entries.len(), 1);

        // Switching to a second session file (e.g. `/new` in Pi TUI) emits reset + new entries.
        let file2 = dir.path().join("s2.jsonl");
        std::fs::write(
            &file2,
            concat!(
                r#"{"type":"session","version":3,"id":"sess-2"}"#,
                "\n",
                r#"{"type":"message","id":"u3","message":{"role":"user","content":"Session two"}}"#,
                "\n"
            ),
        )
        .unwrap();
        watcher.switch_file(file2);
        let switched = watcher.poll_deltas().unwrap();
        assert!(switched.reset);
        assert_eq!(switched.meta.provider_session_id.as_deref(), Some("sess-2"));
        assert_eq!(switched.upserted_entries.len(), 1);
    }
}
