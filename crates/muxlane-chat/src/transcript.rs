//! Pi / Pie 磁盘 JSONL Session 实时解析（用于 PTY/tmux 会话向 Chat 视图实时镜像与续接）。
use muxlane_core::chat::{
    ChatModelInfo, ChatSessionMeta, NoticeLevel, SessionDriverMode, TimelineEntry, ToolCallKind,
    ToolCallState, ToolDiff,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

/// 全量解析一个 Pi session `.jsonl` 文件为统一的 `TimelineEntry` 列表与会话元数据。
pub fn parse_pi_session_file(path: &Path) -> anyhow::Result<(ChatSessionMeta, Vec<TimelineEntry>)> {
    let raw = std::fs::read_to_string(path)?;
    let mut meta = ChatSessionMeta {
        driver: SessionDriverMode::TerminalPty,
        supports_chat_input: false,
        supports_terminal_switch: true,
        has_pty: true,
        provider_session_file: Some(path.display().to_string()),
        ..Default::default()
    };
    let mut entries: Vec<TimelineEntry> = Vec::new();
    let mut tool_index: HashMap<String, usize> = HashMap::new();

    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = record.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "session" => {
                if let Some(id) = record.get("id").and_then(|v| v.as_str()) {
                    meta.provider_session_id = Some(id.to_string());
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
                    meta.current_model = Some(ChatModelInfo {
                        provider,
                        id: model_id.clone(),
                        name: model_id,
                    });
                }
            }
            "thinking_level_change" => {
                if let Some(level) = record.get("thinkingLevel").and_then(|v| v.as_str()) {
                    meta.thinking_level = Some(level.to_string());
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
                entries.push(TimelineEntry::Notice {
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
                    continue;
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
                            entries.push(TimelineEntry::UserMessage {
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
                            meta.current_model = Some(ChatModelInfo {
                                provider: message
                                    .get("provider")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                id: m.clone(),
                                name: m.clone(),
                            });
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
                                            entries.push(TimelineEntry::Thinking {
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
                                            entries.push(TimelineEntry::AssistantText {
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
                                        let pos = entries.len();
                                        tool_index.insert(call_id.clone(), pos);
                                        entries.push(TimelineEntry::ToolCall {
                                            id: format!("tool:{call_id}"),
                                            turn_id: None,
                                            call_id,
                                            kind: ToolCallKind::from_tool_name(&name, None),
                                            name,
                                            title: None,
                                            input,
                                            output: None,
                                            diff,
                                            exit_code: None,
                                            locations: Vec::new(),
                                            state: ToolCallState::Completed,
                                            is_error: false,
                                            duration_ms: None,
                                            created_at_ms: ts,
                                        });
                                    }
                                    _ => {}
                                }
                            }
                        } else {
                            let text = extract_message_text(message);
                            if !text.is_empty() {
                                entries.push(TimelineEntry::AssistantText {
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
                        if let Some(&idx) = tool_index.get(call_id) {
                            if let Some(TimelineEntry::ToolCall {
                                output,
                                is_error: err_slot,
                                state,
                                ..
                            }) = entries.get_mut(idx)
                            {
                                *output = Some(output_text);
                                *err_slot = is_error;
                                *state = if is_error {
                                    ToolCallState::Failed
                                } else {
                                    ToolCallState::Completed
                                };
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    Ok((meta, entries))
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
}
