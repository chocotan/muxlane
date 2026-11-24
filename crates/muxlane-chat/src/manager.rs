//! 服务端 Chat 会话注册表、增量事件分发、日志持久化与 PTY Transcript 镜像协调。
use crate::driver::{ChatDriverHandle, DriverCmd, DriverEvent, DriverLaunchSpec};
use crate::{acp, journal, pi_rpc, transcript};
use muxlane_core::chat::{
    ApprovalReply, ChatAttachment, ChatDeltaEvent, ChatDeltaOp, ChatDriverKind, ChatSessionMeta,
    ChatTimelineState, NoticeLevel, PendingApproval, SessionDriverMode, StreamingBehavior,
    TimelineEntry,
};
use muxlane_core::model::{AgentId, AgentStatus, AgentType};
use muxlane_core::protocol::{events, ChatHistoryResult, ChatSessionSummary, EventMsg};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

#[derive(Debug, Clone)]
pub enum ChatHostNotice {
    StatusChanged {
        agent: AgentId,
        status: AgentStatus,
        message: Option<String>,
        reset_seen: bool,
    },
    TitleChanged {
        agent: AgentId,
        title: String,
    },
    MetaPersisted {
        agent: AgentId,
    },
    ApprovalChanged {
        agent: AgentId,
        approval: Option<Box<PendingApproval>>,
        resolved_id: Option<String>,
    },
}

#[derive(Clone)]
struct ChatSub {
    id: String,
    tx: mpsc::Sender<EventMsg>,
}

struct ChatSessionEntry {
    spec: DriverLaunchSpec,
    timeline: ChatTimelineState,
    journal_path: PathBuf,
    driver: Option<ChatDriverHandle>,
    subs: Vec<ChatSub>,
}

pub struct ChatManager {
    sessions: Arc<Mutex<HashMap<AgentId, ChatSessionEntry>>>,
    journal_dir: std::sync::RwLock<Option<PathBuf>>,
    notices: mpsc::UnboundedSender<ChatHostNotice>,
}

impl ChatManager {
    pub fn new(notices: mpsc::UnboundedSender<ChatHostNotice>) -> Arc<Self> {
        Arc::new(Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            journal_dir: std::sync::RwLock::new(None),
            notices,
        })
    }

    pub fn set_journal_dir(&self, dir: PathBuf) {
        if let Ok(mut slot) = self.journal_dir.write() {
            *slot = Some(dir);
        }
    }

    fn resolve_journal_path(&self, agent: &AgentId) -> PathBuf {
        let dir = self
            .journal_dir
            .read()
            .ok()
            .and_then(|slot| slot.clone())
            .unwrap_or_else(|| std::env::temp_dir().join("muxlane-chats"));
        journal::journal_path(&dir, agent)
    }

    /// 启动并注册一个新的结构化 Chat 会话（Pi RPC 或通用 ACP）。
    pub async fn start_session(
        self: &Arc<Self>,
        spec: DriverLaunchSpec,
        has_pty: bool,
        initial_prompt: Option<String>,
    ) -> anyhow::Result<ChatSessionMeta> {
        let agent = spec.agent.clone();
        let driver_mode = match spec.driver_kind {
            ChatDriverKind::PiRpc => SessionDriverMode::PiRpc,
            ChatDriverKind::Acp => SessionDriverMode::Acp,
        };
        let initial_meta = ChatSessionMeta {
            driver: driver_mode,
            supports_chat_input: true,
            supports_terminal_switch: true,
            has_pty,
            provider_session_id: spec.provider_session.clone(),
            ..Default::default()
        };
        let journal_path = self.resolve_journal_path(&agent);
        let mut timeline = if journal_path.exists() {
            journal::load_timeline(&journal_path).unwrap_or_default()
        } else {
            ChatTimelineState::default()
        };
        timeline.meta = initial_meta.clone();

        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let handle = match spec.driver_kind {
            ChatDriverKind::PiRpc => pi_rpc::spawn(spec.clone(), initial_meta.clone(), ev_tx),
            ChatDriverKind::Acp => acp::spawn(spec.clone(), initial_meta.clone(), ev_tx),
        };
        if let Some(prompt) = initial_prompt.filter(|p| !p.trim().is_empty()) {
            let _ = handle.send(DriverCmd::SendPrompt {
                text: prompt,
                attachments: Vec::new(),
                streaming_behavior: None,
            });
        }

        {
            let mut map = self.sessions.lock().await;
            map.insert(
                agent.clone(),
                ChatSessionEntry {
                    spec,
                    timeline,
                    journal_path,
                    driver: Some(handle),
                    subs: Vec::new(),
                },
            );
        }

        let this = Arc::clone(self);
        tokio::spawn(async move {
            this.pump_driver_events(agent, ev_rx).await;
        });

        Ok(initial_meta)
    }

    /// 启动时从磁盘 journal 恢复历史时间线（惰性唤醒子进程，秒开历史面板）。
    pub async fn restore_session(
        self: &Arc<Self>,
        spec: DriverLaunchSpec,
        provider_session_id: Option<String>,
        provider_session_file: Option<String>,
    ) {
        let agent = spec.agent.clone();
        let journal_path = self.resolve_journal_path(&agent);
        let mut timeline = journal::load_timeline(&journal_path).unwrap_or_default();
        timeline.meta.driver = match spec.driver_kind {
            ChatDriverKind::PiRpc => SessionDriverMode::PiRpc,
            ChatDriverKind::Acp => SessionDriverMode::Acp,
        };
        timeline.meta.supports_chat_input = true;
        timeline.meta.supports_terminal_switch = true;
        timeline.meta.has_pty = false;
        if provider_session_id.is_some() {
            timeline.meta.provider_session_id = provider_session_id;
        }
        if provider_session_file.is_some() {
            timeline.meta.provider_session_file = provider_session_file;
        }
        self.sessions.lock().await.insert(
            agent,
            ChatSessionEntry {
                spec,
                timeline,
                journal_path,
                driver: None,
                subs: Vec::new(),
            },
        );
    }

    pub fn try_persisted_records(
        &self,
    ) -> Vec<(AgentId, DriverLaunchSpec, Option<String>, Option<String>)> {
        let Ok(map) = self.sessions.try_lock() else {
            return Vec::new();
        };
        map.iter()
            .map(|(agent, entry)| {
                (
                    agent.clone(),
                    entry.spec.clone(),
                    entry.timeline.meta.provider_session_id.clone(),
                    entry.timeline.meta.provider_session_file.clone(),
                )
            })
            .collect()
    }

    /// 把已运行中的 PTY 会话（如交互式 `pi`）设为可切换，并标记 `has_pty`。
    pub async fn mark_has_pty(&self, agent: &AgentId, has_pty: bool) {
        let mut map = self.sessions.lock().await;
        if let Some(entry) = map.get_mut(agent) {
            entry.timeline.meta.has_pty = has_pty;
        }
    }

    /// 清空并移除会话（agent.delete 时调用）。
    pub async fn remove_session(&self, agent: &AgentId) {
        let removed = self.sessions.lock().await.remove(agent);
        if let Some(entry) = removed {
            if let Some(driver) = entry.driver {
                let _ = driver.send(DriverCmd::Shutdown);
            }
            journal::remove_journal(&entry.journal_path);
        }
    }

    pub async fn list_summaries(&self) -> Vec<ChatSessionSummary> {
        let map = self.sessions.lock().await;
        map.iter()
            .map(|(agent, entry)| ChatSessionSummary {
                agent: agent.clone(),
                meta: entry.timeline.meta.clone(),
                pending_approvals: entry.timeline.pending_approvals.len(),
                seq: entry.timeline.seq,
            })
            .collect()
    }

    pub async fn session_resume_info(
        &self,
        agent: &AgentId,
    ) -> Option<(DriverLaunchSpec, ChatSessionMeta)> {
        let map = self.sessions.lock().await;
        map.get(agent)
            .map(|entry| (entry.spec.clone(), entry.timeline.meta.clone()))
    }

    /// 停止活动驱动（用于切换进交互式 Terminal `--resume` 时释放 provider 锁）。
    pub async fn detach_driver(&self, agent: &AgentId) {
        let driver = {
            let mut map = self.sessions.lock().await;
            map.get_mut(agent).and_then(|entry| {
                entry.timeline.meta.is_streaming = false;
                entry.driver.take()
            })
        };
        if let Some(d) = driver {
            let _ = d.send(DriverCmd::Shutdown);
        }
    }

    /// 对仅存在 PTY 的 Pi/Pie 会话同步磁盘 JSONL Transcript。
    pub async fn refresh_pty_transcript_if_needed(
        &self,
        agent: &AgentId,
        agent_type: AgentType,
        cwd: PathBuf,
        started_secs: u64,
    ) -> Option<ChatHistoryResult> {
        {
            let map = self.sessions.lock().await;
            if let Some(existing) = map.get(agent) {
                return Some(ChatHistoryResult {
                    agent: agent.clone(),
                    seq: existing.timeline.seq,
                    meta: existing.timeline.meta.clone(),
                    entries: existing.timeline.entries.clone(),
                    pending_approvals: existing.timeline.pending_approvals.clone(),
                });
            }
        }
        if !matches!(agent_type, AgentType::Pi | AgentType::Pie) {
            return None;
        }
        let is_pie = agent_type == AgentType::Pie;
        let path = transcript::discover_pi_session_file(&cwd, is_pie, started_secs)?;
        let (meta, entries) = transcript::parse_pi_session_file(&path).ok()?;
        Some(ChatHistoryResult {
            agent: agent.clone(),
            seq: entries.len() as u64,
            meta,
            entries,
            pending_approvals: Vec::new(),
        })
    }

    pub async fn history(&self, agent: &AgentId) -> Option<ChatHistoryResult> {
        let map = self.sessions.lock().await;
        let entry = map.get(agent)?;
        Some(ChatHistoryResult {
            agent: agent.clone(),
            seq: entry.timeline.seq,
            meta: entry.timeline.meta.clone(),
            entries: entry.timeline.entries.clone(),
            pending_approvals: entry.timeline.pending_approvals.clone(),
        })
    }

    pub async fn subscribe(
        &self,
        agent: &AgentId,
        sub_id: String,
        tx: mpsc::Sender<EventMsg>,
    ) -> anyhow::Result<ChatHistoryResult> {
        let mut map = self.sessions.lock().await;
        let entry = map
            .get_mut(agent)
            .ok_or_else(|| anyhow::anyhow!("no chat session for agent {agent}"))?;
        entry.subs.push(ChatSub { id: sub_id, tx });
        Ok(ChatHistoryResult {
            agent: agent.clone(),
            seq: entry.timeline.seq,
            meta: entry.timeline.meta.clone(),
            entries: entry.timeline.entries.clone(),
            pending_approvals: entry.timeline.pending_approvals.clone(),
        })
    }

    pub async fn unsubscribe(&self, sub_id: &str) {
        let mut map = self.sessions.lock().await;
        for entry in map.values_mut() {
            entry.subs.retain(|s| s.id != sub_id);
        }
    }

    async fn ensure_driver_active(
        self: &Arc<Self>,
        agent: &AgentId,
    ) -> anyhow::Result<ChatDriverHandle> {
        let (spec, meta, resume_ref) = {
            let map = self.sessions.lock().await;
            let entry = map
                .get(agent)
                .ok_or_else(|| anyhow::anyhow!("no chat session for agent {agent}"))?;
            if let Some(handle) = &entry.driver {
                return Ok(handle.clone());
            }
            let resume = entry
                .timeline
                .meta
                .provider_session_file
                .clone()
                .or_else(|| entry.timeline.meta.provider_session_id.clone());
            (entry.spec.clone(), entry.timeline.meta.clone(), resume)
        };

        let mut launch_spec = spec;
        launch_spec.provider_session = resume_ref.clone();
        if launch_spec.driver_kind == ChatDriverKind::PiRpc {
            if let Some(session) = resume_ref.as_deref().filter(|s| !s.is_empty()) {
                if !launch_spec.args.iter().any(|a| a == "--session") {
                    launch_spec.args.push("--session".into());
                    launch_spec.args.push(session.to_string());
                }
            }
        }

        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let handle = match launch_spec.driver_kind {
            ChatDriverKind::PiRpc => pi_rpc::spawn(launch_spec, meta, ev_tx),
            ChatDriverKind::Acp => acp::spawn(launch_spec, meta, ev_tx),
        };
        {
            let mut map = self.sessions.lock().await;
            if let Some(entry) = map.get_mut(agent) {
                entry.driver = Some(handle.clone());
            }
        }
        let this = Arc::clone(self);
        let agent2 = agent.clone();
        tokio::spawn(async move {
            this.pump_driver_events(agent2, ev_rx).await;
        });
        Ok(handle)
    }

    pub async fn send_prompt(
        self: &Arc<Self>,
        agent: &AgentId,
        text: String,
        attachments: Vec<ChatAttachment>,
        streaming_behavior: Option<StreamingBehavior>,
    ) -> anyhow::Result<()> {
        let driver = self.ensure_driver_active(agent).await?;
        driver.send(DriverCmd::SendPrompt {
            text,
            attachments,
            streaming_behavior,
        })
    }

    pub async fn abort(&self, agent: &AgentId) -> anyhow::Result<()> {
        let driver = {
            let map = self.sessions.lock().await;
            map.get(agent).and_then(|e| e.driver.clone())
        };
        if let Some(driver) = driver {
            driver.send(DriverCmd::Abort)?;
        }
        Ok(())
    }

    pub async fn approve(
        &self,
        agent: &AgentId,
        approval_id: String,
        reply: ApprovalReply,
    ) -> anyhow::Result<()> {
        let driver = {
            let map = self.sessions.lock().await;
            map.get(agent)
                .and_then(|e| e.driver.clone())
                .ok_or_else(|| anyhow::anyhow!("driver not running for {agent}"))?
        };
        driver.send(DriverCmd::RespondApproval { approval_id, reply })
    }

    pub async fn configure(
        self: &Arc<Self>,
        agent: &AgentId,
        provider: Option<String>,
        model_id: Option<String>,
        thinking_level: Option<String>,
    ) -> anyhow::Result<()> {
        let driver = self.ensure_driver_active(agent).await?;
        if let Some(model_id) = model_id {
            driver.send(DriverCmd::SetModel { provider, model_id })?;
        }
        if let Some(level) = thinking_level {
            driver.send(DriverCmd::SetThinkingLevel { level })?;
        }
        Ok(())
    }

    async fn pump_driver_events(
        self: Arc<Self>,
        agent: AgentId,
        mut rx: mpsc::UnboundedReceiver<DriverEvent>,
    ) {
        while let Some(ev) = rx.recv().await {
            match ev {
                DriverEvent::Delta(op) => {
                    let meta_changed = matches!(op, ChatDeltaOp::MetaUpdated { .. });
                    let approval_notice = match &op {
                        ChatDeltaOp::ApprovalRequested { approval } => {
                            Some((Some(approval.clone()), None))
                        }
                        ChatDeltaOp::ApprovalResolved { approval_id } => {
                            Some((None, Some(approval_id.clone())))
                        }
                        _ => None,
                    };
                    let (event, subs, journal_path) = {
                        let mut map = self.sessions.lock().await;
                        let Some(entry) = map.get_mut(&agent) else {
                            break;
                        };
                        let next_seq = entry.timeline.seq + 1;
                        let ev = ChatDeltaEvent {
                            agent: agent.clone(),
                            seq: next_seq,
                            delta: op,
                        };
                        entry.timeline.apply(ev.clone());
                        entry.subs.retain(|s| !s.tx.is_closed());
                        (ev, entry.subs.clone(), entry.journal_path.clone())
                    };

                    let _ = journal::append_delta(&journal_path, &event);
                    if meta_changed {
                        let _ = self.notices.send(ChatHostNotice::MetaPersisted {
                            agent: agent.clone(),
                        });
                    }
                    if let Some((approval, resolved_id)) = approval_notice {
                        let _ = self.notices.send(ChatHostNotice::ApprovalChanged {
                            agent: agent.clone(),
                            approval: approval.map(Box::new),
                            resolved_id,
                        });
                    }
                    if let Ok(payload) = serde_json::to_value(&event) {
                        let rpc_ev = EventMsg::new(events::CHAT_DELTA, payload);
                        for sub in subs {
                            let _ = sub.tx.try_send(rpc_ev.clone());
                        }
                    }
                }
                DriverEvent::Status {
                    status,
                    message,
                    reset_seen,
                } => {
                    let _ = self.notices.send(ChatHostNotice::StatusChanged {
                        agent: agent.clone(),
                        status,
                        message,
                        reset_seen,
                    });
                }
                DriverEvent::Title(title) => {
                    let _ = self.notices.send(ChatHostNotice::TitleChanged {
                        agent: agent.clone(),
                        title,
                    });
                }
                DriverEvent::Exited { error } => {
                    let mut map = self.sessions.lock().await;
                    let Some(entry) = map.get_mut(&agent) else {
                        break;
                    };
                    entry.driver = None;
                    entry.timeline.meta.is_streaming = false;
                    if let Some(err) = &error {
                        let seq = entry.timeline.seq + 1;
                        let ev = ChatDeltaEvent {
                            agent: agent.clone(),
                            seq,
                            delta: ChatDeltaOp::UpsertEntry {
                                entry: TimelineEntry::Notice {
                                    id: format!("exit-{}", ulid::Ulid::new()),
                                    turn_id: None,
                                    level: NoticeLevel::Error,
                                    text: err.clone(),
                                    created_at_ms: muxlane_core::chat::now_ms(),
                                },
                            },
                        };
                        entry.timeline.apply(ev.clone());
                        let _ = journal::append_delta(&entry.journal_path, &ev);
                        if let Ok(payload) = serde_json::to_value(&ev) {
                            let rpc_ev = EventMsg::new(events::CHAT_DELTA, payload);
                            for sub in &entry.subs {
                                let _ = sub.tx.try_send(rpc_ev.clone());
                            }
                        }
                        let _ = self.notices.send(ChatHostNotice::StatusChanged {
                            agent: agent.clone(),
                            status: AgentStatus::Failed,
                            message: Some(err.clone()),
                            reset_seen: true,
                        });
                    }
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn node_available() -> bool {
        std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_ok()
    }

    #[tokio::test]
    async fn chat_manager_drives_pi_rpc_subprocess_end_to_end() {
        if !node_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("mock_pi.mjs");
        std::fs::write(
            &script,
            r#"
let buf = "";
process.stdin.setEncoding("utf8");
const send = (o) => process.stdout.write(JSON.stringify(o) + "\n");
process.stdin.on("data", (chunk) => {
  buf += chunk;
  let idx;
  while ((idx = buf.indexOf("\n")) !== -1) {
    const line = buf.slice(0, idx).trim();
    buf = buf.slice(idx + 1);
    if (!line) continue;
    const msg = JSON.parse(line);
    if (msg.type === "get_state") {
      send({ id: msg.id, type: "response", command: "get_state", success: true, data: { sessionId: "pi-s1", sessionFile: "/tmp/pi-s1.jsonl", thinkingLevel: "medium", isStreaming: false, model: { provider: "anthropic", id: "sonnet", name: "Sonnet" } } });
    } else if (msg.type === "get_available_models") {
      send({ id: msg.id, type: "response", command: "get_available_models", success: true, data: { models: [{ provider: "anthropic", id: "sonnet", name: "Sonnet" }] } });
    } else if (msg.type === "get_available_thinking_levels") {
      send({ id: msg.id, type: "response", command: "get_available_thinking_levels", success: true, data: { levels: ["off", "medium", "high"] } });
    } else if (msg.type === "prompt") {
      send({ id: msg.id, type: "response", command: "prompt", success: true, data: { disposition: "started" } });
      send({ type: "agent_start" });
      send({ type: "turn_start" });
      send({ type: "message_start", message: { role: "assistant", content: [] } });
      send({ type: "message_update", assistantMessageEvent: { type: "text_delta", contentIndex: 0, delta: "Need confirm." } });
      send({ type: "extension_ui_request", id: "perm-1", method: "confirm", title: "Proceed?", message: "Run check" });
    } else if (msg.type === "extension_ui_response" && msg.id === "perm-1") {
      send({ type: "message_update", assistantMessageEvent: { type: "text_delta", contentIndex: 0, delta: msg.confirmed ? " Confirmed!" : " Denied!" } });
      send({ type: "message_end", message: { role: "assistant", content: [{ type: "text", text: "Need confirm. Confirmed!" }] } });
      send({ type: "agent_settled", aborted: false });
    }
  }
});
"#,
        )
        .unwrap();

        let (notice_tx, mut notice_rx) = mpsc::unbounded_channel();
        let manager = ChatManager::new(notice_tx);
        manager.set_journal_dir(dir.path().join("journals"));
        manager
            .start_session(
                DriverLaunchSpec {
                    agent: "pi-1".into(),
                    agent_type: AgentType::Pi,
                    driver_kind: ChatDriverKind::PiRpc,
                    program: "node".into(),
                    args: vec![script.display().to_string()],
                    cwd: dir.path().to_path_buf(),
                    env: vec![],
                    provider_session: None,
                    model: None,
                    thinking_level: None,
                },
                false,
                Some("Hello Pi".into()),
            )
            .await
            .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let approval_id = loop {
            if let Some(hist) = manager.history(&"pi-1".into()).await {
                if let Some(first) = hist.pending_approvals.first() {
                    break first.id.clone();
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for extension_ui_request"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };

        manager
            .approve(
                &"pi-1".into(),
                approval_id,
                ApprovalReply::Confirm { confirmed: true },
            )
            .await
            .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let mut settled = false;
            while let Ok(notice) = notice_rx.try_recv() {
                if matches!(
                    notice,
                    ChatHostNotice::StatusChanged {
                        status: AgentStatus::Done,
                        ..
                    }
                ) {
                    settled = true;
                }
            }
            if settled {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for Done status"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let hist = manager.history(&"pi-1".into()).await.unwrap();
        assert_eq!(hist.meta.provider_session_id.as_deref(), Some("pi-s1"));
        assert!(hist.pending_approvals.is_empty());
        assert!(hist.entries.iter().any(|e| matches!(
            e,
            TimelineEntry::AssistantText { text, .. } if text.contains("Confirmed!")
        )));
    }

    #[tokio::test]
    async fn chat_manager_drives_acp_subprocess_end_to_end() {
        if !node_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("mock_acp.mjs");
        std::fs::write(
            &script,
            r#"
let buf = "";
let pendingPromptId = null;
process.stdin.setEncoding("utf8");
const send = (o) => process.stdout.write(JSON.stringify(o) + "\n");
process.stdin.on("data", (chunk) => {
  buf += chunk;
  let idx;
  while ((idx = buf.indexOf("\n")) !== -1) {
    const line = buf.slice(0, idx).trim();
    buf = buf.slice(idx + 1);
    if (!line) continue;
    const msg = JSON.parse(line);
    if (msg.method === "initialize") {
      send({ jsonrpc: "2.0", id: msg.id, result: { protocolVersion: 1, agentCapabilities: { loadSession: true } } });
    } else if (msg.method === "session/new") {
      send({ jsonrpc: "2.0", id: msg.id, result: { sessionId: "acp-s1", models: { currentModelId: "qwen-coder", availableModels: [{ modelId: "qwen-coder", name: "Qwen Coder" }] } } });
    } else if (msg.method === "session/prompt") {
      pendingPromptId = msg.id;
      send({ jsonrpc: "2.0", method: "session/update", params: { sessionId: "acp-s1", update: { sessionUpdate: "tool_call", toolCallId: "tc-1", title: "Write test", kind: "edit", status: "pending" } } });
      send({ jsonrpc: "2.0", id: 500, method: "session/request_permission", params: { sessionId: "acp-s1", toolCall: { toolCallId: "tc-1", title: "Allow write?" }, options: [{ optionId: "allow_once", name: "Allow", kind: "allow_once" }] } });
    } else if (msg.id === 500) {
      send({ jsonrpc: "2.0", method: "session/update", params: { sessionId: "acp-s1", update: { sessionUpdate: "tool_call_update", toolCallId: "tc-1", status: "completed", content: [{ type: "diff", path: "a.txt", oldText: "1", newText: "2" }] } } });
      send({ jsonrpc: "2.0", method: "session/update", params: { sessionId: "acp-s1", update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "All set via ACP!" } } } });
      send({ jsonrpc: "2.0", id: pendingPromptId, result: { stopReason: "end_turn" } });
    }
  }
});
"#,
        )
        .unwrap();

        let (notice_tx, mut notice_rx) = mpsc::unbounded_channel();
        let manager = ChatManager::new(notice_tx);
        manager.set_journal_dir(dir.path().join("journals"));
        manager
            .start_session(
                DriverLaunchSpec {
                    agent: "opencode-1".into(),
                    agent_type: AgentType::Opencode,
                    driver_kind: ChatDriverKind::Acp,
                    program: "node".into(),
                    args: vec![script.display().to_string()],
                    cwd: dir.path().to_path_buf(),
                    env: vec![],
                    provider_session: None,
                    model: None,
                    thinking_level: None,
                },
                false,
                Some("Edit a.txt".into()),
            )
            .await
            .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let approval_id = loop {
            if let Some(hist) = manager.history(&"opencode-1".into()).await {
                if let Some(first) = hist.pending_approvals.first() {
                    break first.id.clone();
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for ACP permission"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };

        manager
            .approve(
                &"opencode-1".into(),
                approval_id,
                ApprovalReply::SelectedOption {
                    option_id: "allow_once".into(),
                },
            )
            .await
            .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let mut done = false;
            while let Ok(notice) = notice_rx.try_recv() {
                if matches!(
                    notice,
                    ChatHostNotice::StatusChanged {
                        status: AgentStatus::Done,
                        ..
                    }
                ) {
                    done = true;
                }
            }
            if done {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for ACP Done"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let hist = manager.history(&"opencode-1".into()).await.unwrap();
        assert_eq!(hist.meta.provider_session_id.as_deref(), Some("acp-s1"));
        assert!(hist.entries.iter().any(|e| matches!(
            e,
            TimelineEntry::AssistantText { text, .. } if text == "All set via ACP!"
        )));
    }
}
