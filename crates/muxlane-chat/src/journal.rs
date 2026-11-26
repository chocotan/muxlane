//! 会话时间线 JSONL 持久化日志（支持重启恢复与断线增量重放）。
use anyhow::Result;
use muxlane_core::chat::{ChatDeltaEvent, ChatTimelineState};
use muxlane_core::model::AgentId;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn journal_dir_from_state_root(root: &Path) -> PathBuf {
    root.join("chats")
}

pub fn journal_path(dir: &Path, agent: &AgentId) -> PathBuf {
    let safe: String = agent
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("{safe}.jsonl"))
}

pub fn append_delta(path: &Path, event: &ChatDeltaEvent) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(event)?;
    writeln!(file, "{line}")?;
    Ok(())
}

pub fn load_timeline(path: &Path) -> Result<ChatTimelineState> {
    let mut state = ChatTimelineState::default();
    if !path.exists() {
        return Ok(state);
    }
    let raw = std::fs::read_to_string(path)?;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<ChatDeltaEvent>(trimmed) {
            Ok(event) => {
                state.apply(event);
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "ignoring corrupt chat journal line");
            }
        }
    }
    // 历史回放加载完成时，若上次进程崩溃导致某条目停在 streaming=true，统一收敛闭合。
    state.meta.is_streaming = false;
    state.pending_approvals.clear();
    state.apply_op(muxlane_core::ChatDeltaOp::TurnFinished {
        turn_id: None,
        error: None,
    });
    Ok(state)
}

pub fn remove_journal(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxlane_core::chat::{ChatDeltaOp, TextDeltaKind, TimelineEntry};

    #[test]
    fn journal_roundtrips_and_converges_unfinished_streams() {
        let dir = tempfile::tempdir().unwrap();
        let path = journal_path(dir.path(), &"pi-1".into());
        append_delta(
            &path,
            &ChatDeltaEvent {
                agent: "pi-1".into(),
                seq: 1,
                delta: ChatDeltaOp::UpsertEntry {
                    entry: TimelineEntry::AssistantText {
                        id: "msg-1".into(),
                        turn_id: Some("t1".into()),
                        text: "hi ".into(),
                        streaming: true,
                        model: None,
                        usage: None,
                        created_at_ms: 100,
                    },
                },
            },
        )
        .unwrap();
        append_delta(
            &path,
            &ChatDeltaEvent {
                agent: "pi-1".into(),
                seq: 2,
                delta: ChatDeltaOp::AppendText {
                    entry_id: "msg-1".into(),
                    delta: "there".into(),
                    kind: TextDeltaKind::Assistant,
                },
            },
        )
        .unwrap();

        let restored = load_timeline(&path).unwrap();
        assert_eq!(restored.seq, 2);
        assert!(!restored.meta.is_streaming);
        assert_eq!(restored.entries.len(), 1);
        match &restored.entries[0] {
            TimelineEntry::AssistantText {
                text, streaming, ..
            } => {
                assert_eq!(text, "hi there");
                assert!(!streaming);
            }
            other => panic!("unexpected entry: {other:?}"),
        }
    }
}
