//! 服务端 Chat 会话注册表、增量事件分发、日志持久化与 Orca 风格 PTY 双向实时桥接协调。
use crate::driver::{ChatDriverHandle, DriverCmd, DriverEvent, DriverLaunchSpec};
use crate::pty_bridge::{self, PtyBridgeWriter};
use crate::{acp, journal, pi_rpc, transcript};
use muxlane_core::chat::{
    now_ms, ApprovalOption, ApprovalReply, ApprovalSource, ChatAttachment, ChatDeltaEvent,
    ChatDeltaOp, ChatDriverKind, ChatModelInfo, ChatSessionMeta, ChatTimelineState, NoticeLevel,
    PendingApproval, SessionDriverMode, StreamingBehavior, TimelineEntry,
};
use muxlane_core::model::{AgentId, AgentStatus, AgentType};
use muxlane_core::protocol::{
    events, AgentReportParams, ChatHistoryResult, ChatSessionSummary, EventMsg,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex, Notify};

const BRIDGE_LIVE_PREVIEW_ID: &str = "__bridge_live_preview__";
const BRIDGE_PENDING_USER_PREFIX: &str = "__bridge_pending_user__:";
const BRIDGE_POLL_INTERVAL: Duration = Duration::from_millis(250);

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

struct PtyBridgeState {
    agent_type: AgentType,
    cwd: PathBuf,
    fallback_cwd: Option<PathBuf>,
    started_secs: u64,
    proc_start_secs: Option<u64>,
    allow_older_fallback: bool,
    session_file_hint: Option<PathBuf>,
    write_lock: Arc<Mutex<()>>,
    wake_notify: Arc<Notify>,
    watcher_abort: Option<tokio::task::AbortHandle>,
    /// Optimistic user echoes awaiting disk JSONL flush: (entry_id, normalized_text, created_at_ms).
    pending_user_echoes: Vec<(String, String, u64)>,
    has_live_preview: bool,
    bridge_approval_id: Option<String>,
    last_auto_title: Option<String>,
}

impl Drop for PtyBridgeState {
    fn drop(&mut self) {
        if let Some(handle) = self.watcher_abort.take() {
            handle.abort();
        }
    }
}

struct ChatSessionEntry {
    spec: DriverLaunchSpec,
    timeline: ChatTimelineState,
    journal_path: PathBuf,
    driver: Option<ChatDriverHandle>,
    pty_bridge: Option<PtyBridgeState>,
    subs: Vec<ChatSub>,
}

pub struct ChatManager {
    sessions: Arc<Mutex<HashMap<AgentId, ChatSessionEntry>>>,
    journal_dir: std::sync::RwLock<Option<PathBuf>>,
    pty_writer: std::sync::RwLock<Option<Arc<dyn PtyBridgeWriter>>>,
    notices: mpsc::UnboundedSender<ChatHostNotice>,
}

fn normalize_echo_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_transient_bridge_op(op: &ChatDeltaOp) -> bool {
    match op {
        ChatDeltaOp::UpsertEntry { entry } => {
            entry.id() == BRIDGE_LIVE_PREVIEW_ID
                || entry.id().starts_with(BRIDGE_PENDING_USER_PREFIX)
        }
        ChatDeltaOp::RemoveEntry { entry_id } => {
            entry_id == BRIDGE_LIVE_PREVIEW_ID || entry_id.starts_with(BRIDGE_PENDING_USER_PREFIX)
        }
        _ => false,
    }
}

fn default_driver_kind_for(agent_type: AgentType) -> ChatDriverKind {
    match agent_type {
        AgentType::Pi | AgentType::Pie => ChatDriverKind::PiRpc,
        _ => ChatDriverKind::Acp,
    }
}

fn supports_native_switch_for(agent_type: AgentType) -> bool {
    muxlane_core::builtin_presets("")
        .into_iter()
        .find(|p| p.agent_type == agent_type)
        .and_then(|p| p.chat_driver_kind())
        .is_some()
}

fn model_info_from_string(raw: &str) -> Option<ChatModelInfo> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((provider, id)) = trimmed.split_once('/') {
        Some(ChatModelInfo {
            provider: provider.to_string(),
            id: id.to_string(),
            name: id.to_string(),
        })
    } else {
        Some(ChatModelInfo {
            provider: String::new(),
            id: trimmed.to_string(),
            name: trimmed.to_string(),
        })
    }
}

impl ChatManager {
    pub fn new(notices: mpsc::UnboundedSender<ChatHostNotice>) -> Arc<Self> {
        Arc::new(Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            journal_dir: std::sync::RwLock::new(None),
            pty_writer: std::sync::RwLock::new(None),
            notices,
        })
    }

    pub fn set_journal_dir(&self, dir: PathBuf) {
        if let Ok(mut slot) = self.journal_dir.write() {
            *slot = Some(dir);
        }
    }

    pub fn set_pty_writer(&self, writer: Arc<dyn PtyBridgeWriter>) {
        if let Ok(mut slot) = self.pty_writer.write() {
            *slot = Some(writer);
        }
    }

    fn get_pty_writer(&self) -> Option<Arc<dyn PtyBridgeWriter>> {
        self.pty_writer.read().ok().and_then(|slot| slot.clone())
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

    async fn emit_delta_op(&self, agent: &AgentId, op: ChatDeltaOp) {
        let op = match op {
            ChatDeltaOp::UpsertEntry { entry } => ChatDeltaOp::UpsertEntry {
                entry: compact_entry_for_wire(entry),
            },
            other => other,
        };
        let persist = !is_transient_bridge_op(&op);
        let meta_changed = matches!(op, ChatDeltaOp::MetaUpdated { .. });
        let approval_notice = match &op {
            ChatDeltaOp::ApprovalRequested { approval } => Some((Some(approval.clone()), None)),
            ChatDeltaOp::ApprovalResolved { approval_id } => {
                Some((None, Some(approval_id.clone())))
            }
            _ => None,
        };
        let prepared = {
            let mut map = self.sessions.lock().await;
            let Some(entry) = map.get_mut(agent) else {
                return;
            };
            let next_seq = entry.timeline.seq + 1;
            let ev = ChatDeltaEvent {
                agent: agent.clone(),
                seq: next_seq,
                delta: op,
            };
            entry.timeline.apply(ev.clone());
            entry.subs.retain(|s| !s.tx.is_closed());
            let is_structured = entry.pty_bridge.is_none();
            (
                ev,
                entry.subs.clone(),
                entry.journal_path.clone(),
                is_structured,
            )
        };
        let (event, subs, journal_path, is_structured) = prepared;
        if persist {
            let _ = journal::append_delta(&journal_path, &event);
        }
        if meta_changed && is_structured {
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
            ChatDriverKind::Acp if spec.agent_type == AgentType::Agy => {
                crate::agy_stream::spawn(spec.clone(), initial_meta.clone(), ev_tx)
            }
            ChatDriverKind::Acp => acp::spawn(spec.clone(), initial_meta.clone(), ev_tx),
        };
        if let Some(prompt) = initial_prompt.filter(|p| !p.trim().is_empty()) {
            let _ = handle.send(DriverCmd::SendPrompt {
                text: prompt,
                attachments: Vec::new(),
                streaming_behavior: None,
            });
        }

        let preserved_subs = {
            let mut map = self.sessions.lock().await;
            let subs = map
                .remove(&agent)
                .map(|mut old| {
                    if let Some(old_driver) = old.driver.take() {
                        let _ = old_driver.send(DriverCmd::Shutdown);
                    }
                    // Dropping `old.pty_bridge` aborts any active watcher task cleanly.
                    old.subs
                })
                .unwrap_or_default();
            map.insert(
                agent.clone(),
                ChatSessionEntry {
                    spec,
                    timeline,
                    journal_path,
                    driver: Some(handle),
                    pty_bridge: None,
                    subs: subs.clone(),
                },
            );
            subs
        };

        if !preserved_subs.is_empty() {
            self.emit_delta_op(
                &agent,
                ChatDeltaOp::MetaUpdated {
                    meta: initial_meta.clone(),
                },
            )
            .await;
        }

        let this = Arc::clone(self);
        tokio::spawn(async move {
            this.pump_driver_events(agent, ev_rx).await;
        });

        Ok(initial_meta)
    }

    /// 为运行在交互式 PTY/tmux 中的 AI Agent 注册或激活 Orca 风格双向实时桥接会话（零重启）。
    pub async fn ensure_pty_bridge(
        self: &Arc<Self>,
        agent: &AgentId,
        agent_type: AgentType,
        cwd: PathBuf,
        started_secs: u64,
        alive: bool,
        session_file_hint: Option<PathBuf>,
    ) -> ChatSessionMeta {
        self.ensure_pty_bridge_with_context(
            agent,
            agent_type,
            cwd,
            None,
            started_secs,
            None,
            false,
            alive,
            session_file_hint,
        )
        .await
    }

    /// 携带实时 pane cwd、回退项目目录、前台进程启动时间与是否允许匹配历史会话标志注册/更新 PTY Bridge。
    #[allow(clippy::too_many_arguments)]
    pub async fn ensure_pty_bridge_with_context(
        self: &Arc<Self>,
        agent: &AgentId,
        agent_type: AgentType,
        cwd: PathBuf,
        fallback_cwd: Option<PathBuf>,
        started_secs: u64,
        proc_start_secs: Option<u64>,
        allow_older_fallback: bool,
        alive: bool,
        session_file_hint: Option<PathBuf>,
    ) -> ChatSessionMeta {
        let mut emit_meta_update: Option<ChatSessionMeta> = None;
        let (existing_meta, claimed_files) = {
            let mut map = self.sessions.lock().await;
            let claimed: Vec<PathBuf> = map
                .iter()
                .filter(|(id, _)| *id != agent)
                .filter_map(|(_, e)| {
                    e.pty_bridge
                        .as_ref()
                        .and_then(|b| b.session_file_hint.clone())
                        .or_else(|| {
                            e.timeline
                                .meta
                                .provider_session_file
                                .as_deref()
                                .map(PathBuf::from)
                        })
                })
                .collect();
            let existing = if let Some(entry) = map.get_mut(agent) {
                // Lane B structured driver owns this session; do not override it with PTY bridge.
                if entry.driver.is_some() {
                    return entry.timeline.meta.clone();
                }
                if let Some(bridge) = entry.pty_bridge.as_mut() {
                    let relaunched_fresh = !allow_older_fallback
                        && match (bridge.proc_start_secs, proc_start_secs) {
                            (Some(prev), Some(next)) => next > prev.saturating_add(5),
                            (None, Some(next)) => next > bridge.started_secs.saturating_add(5),
                            _ => false,
                        };
                    bridge.agent_type = agent_type;
                    bridge.cwd = cwd.clone();
                    bridge.fallback_cwd = fallback_cwd.clone();
                    if relaunched_fresh {
                        bridge.started_secs = proc_start_secs.unwrap_or(started_secs);
                        bridge.proc_start_secs = proc_start_secs;
                        bridge.allow_older_fallback = false;
                        bridge.session_file_hint = session_file_hint.clone();
                        entry.timeline.meta.provider_session_file = None;
                        entry.timeline.meta.provider_session_id = None;
                    } else {
                        if proc_start_secs.is_some() {
                            bridge.proc_start_secs = proc_start_secs;
                        }
                        if let Some(hint) = session_file_hint.clone() {
                            bridge.session_file_hint = Some(hint);
                        }
                    }
                    let mut changed = relaunched_fresh;
                    if entry.timeline.meta.supports_chat_input != alive {
                        entry.timeline.meta.supports_chat_input = alive;
                        changed = true;
                    }
                    if !entry.timeline.meta.has_pty {
                        entry.timeline.meta.has_pty = true;
                        changed = true;
                    }
                    if !alive && entry.timeline.meta.is_streaming {
                        entry.timeline.meta.is_streaming = false;
                        changed = true;
                    }
                    let supports_switch = supports_native_switch_for(agent_type);
                    if entry.timeline.meta.supports_terminal_switch != supports_switch {
                        entry.timeline.meta.supports_terminal_switch = supports_switch;
                        changed = true;
                    }
                    bridge.wake_notify.notify_one();
                    if changed {
                        emit_meta_update = Some(entry.timeline.meta.clone());
                    }
                    Some(entry.timeline.meta.clone())
                } else {
                    None
                }
            } else {
                None
            };
            (existing, claimed)
        };

        if let Some(meta) = existing_meta {
            if let Some(updated) = emit_meta_update {
                self.emit_delta_op(agent, ChatDeltaOp::MetaUpdated { meta: updated })
                    .await;
            }
            return meta;
        }

        // Hydrate initial transcript synchronously (outside lock) if a matching Pi/Pie JSONL exists.
        // Freshly spawned sessions (`allow_older_fallback == false`) strictly require a file created
        // at/after spawn time so they never inherit an old session's transcript.
        let initial_file = session_file_hint.clone().or_else(|| {
            if matches!(agent_type, AgentType::Pi | AgentType::Pie) {
                let effective_floor = if allow_older_fallback {
                    started_secs.saturating_sub(3600)
                } else {
                    proc_start_secs.unwrap_or(started_secs)
                };
                transcript::discover_pi_session_file_with_context(
                    &cwd,
                    fallback_cwd.as_deref(),
                    agent_type == AgentType::Pie,
                    effective_floor,
                    proc_start_secs,
                    allow_older_fallback,
                    &claimed_files,
                )
            } else {
                None
            }
        });
        let mut initial_watcher = initial_file
            .clone()
            .map(transcript::IncrementalPiTranscript::new);
        let initial_poll = initial_watcher.as_mut().and_then(|w| w.poll_deltas().ok());

        let journal_path = self.resolve_journal_path(agent);
        let wake_notify = Arc::new(Notify::new());
        let supports_switch = supports_native_switch_for(agent_type);

        let (result_meta, should_emit_meta, initial_title) = {
            let mut map = self.sessions.lock().await;
            // Re-check after lock acquisition in case of concurrent ensure_pty_bridge call.
            if let Some(existing) = map.get(agent) {
                if existing.driver.is_some() || existing.pty_bridge.is_some() {
                    return existing.timeline.meta.clone();
                }
            }

            let preserved = map.remove(agent);
            let mut timeline = preserved
                .as_ref()
                .map(|old| old.timeline.clone())
                .unwrap_or_default();
            let preserved_subs = preserved.map(|old| old.subs).unwrap_or_default();

            let mut meta = ChatSessionMeta {
                driver: SessionDriverMode::TerminalPty,
                supports_chat_input: alive,
                supports_terminal_switch: supports_switch,
                has_pty: true,
                provider_session_id: timeline.meta.provider_session_id.clone(),
                provider_session_file: initial_file
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .or_else(|| timeline.meta.provider_session_file.clone()),
                current_model: timeline.meta.current_model.clone(),
                thinking_level: timeline.meta.thinking_level.clone(),
                ..Default::default()
            };

            let mut initial_title = None;
            if let Some(poll) = initial_poll {
                merge_transcript_meta(&mut meta, &poll.meta);
                initial_title = poll.session_title;
                for entry in poll.upserted_entries {
                    let seq = timeline.seq + 1;
                    timeline.apply(ChatDeltaEvent {
                        agent: agent.clone(),
                        seq,
                        delta: ChatDeltaOp::UpsertEntry { entry },
                    });
                }
            }
            timeline.meta = meta.clone();

            let this = Arc::clone(self);
            let agent_for_watch = agent.clone();
            let wake_for_watch = Arc::clone(&wake_notify);
            let watcher_handle = tokio::spawn(async move {
                this.watch_pty_bridge_transcript(agent_for_watch, wake_for_watch, initial_watcher)
                    .await;
            })
            .abort_handle();

            let spec = DriverLaunchSpec {
                agent: agent.clone(),
                agent_type,
                driver_kind: default_driver_kind_for(agent_type),
                program: agent_type.program().to_string(),
                args: Vec::new(),
                cwd: cwd.clone(),
                env: Vec::new(),
                provider_session: meta
                    .provider_session_file
                    .clone()
                    .or_else(|| meta.provider_session_id.clone()),
                model: None,
                thinking_level: None,
            };

            let had_subs = !preserved_subs.is_empty();
            map.insert(
                agent.clone(),
                ChatSessionEntry {
                    spec,
                    timeline,
                    journal_path,
                    driver: None,
                    pty_bridge: Some(PtyBridgeState {
                        agent_type,
                        cwd,
                        fallback_cwd,
                        started_secs,
                        proc_start_secs,
                        allow_older_fallback,
                        session_file_hint: initial_file,
                        write_lock: Arc::new(Mutex::new(())),
                        wake_notify,
                        watcher_abort: Some(watcher_handle),
                        pending_user_echoes: Vec::new(),
                        has_live_preview: false,
                        bridge_approval_id: None,
                        last_auto_title: initial_title.clone(),
                    }),
                    subs: preserved_subs,
                },
            );
            (meta, had_subs, initial_title)
        };

        if let Some(title) = initial_title {
            let _ = self.notices.send(ChatHostNotice::TitleChanged {
                agent: agent.clone(),
                title,
            });
        }

        if should_emit_meta {
            self.emit_delta_op(
                agent,
                ChatDeltaOp::MetaUpdated {
                    meta: result_meta.clone(),
                },
            )
            .await;
        }
        result_meta
    }

    /// 切换 PTY Bridge 写入锁状态（前台进程退回裸 Shell 时立刻降为只读，重新启动 Agent 时自动解锁）。
    pub async fn set_pty_bridge_alive(&self, agent: &AgentId, alive: bool) {
        let next_meta = {
            let mut map = self.sessions.lock().await;
            let Some(entry) = map.get_mut(agent) else {
                return;
            };
            if entry.driver.is_some() || entry.pty_bridge.is_none() {
                return;
            }
            let mut changed = false;
            if entry.timeline.meta.supports_chat_input != alive {
                entry.timeline.meta.supports_chat_input = alive;
                changed = true;
            }
            if !alive && entry.timeline.meta.is_streaming {
                entry.timeline.meta.is_streaming = false;
                changed = true;
            }
            if changed {
                Some(entry.timeline.meta.clone())
            } else {
                None
            }
        };
        if let Some(meta) = next_meta {
            self.emit_delta_op(agent, ChatDeltaOp::MetaUpdated { meta })
                .await;
        }
    }

    /// 唤醒 PTY Bridge 监听任务（例如用户在终端手动回车后立即同步磁盘 Transcript）。
    pub async fn wake_pty_bridge(&self, agent: &AgentId) {
        let wake = {
            let map = self.sessions.lock().await;
            map.get(agent)
                .and_then(|entry| entry.pty_bridge.as_ref())
                .map(|b| Arc::clone(&b.wake_notify))
        };
        if let Some(wake) = wake {
            wake.notify_one();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(120)).await;
                wake.notify_one();
            });
        }
    }

    /// 接收来自 Pi/OpenCode Extension 的 `agent.report` 侧信道事件（会话文件/模型/实时预览/审批/完成）。
    pub async fn notify_pty_hook(&self, params: &AgentReportParams) {
        let (ops, wake) = {
            let mut map = self.sessions.lock().await;
            let Some(entry) = map.get_mut(&params.agent) else {
                return;
            };
            if entry.driver.is_some() {
                return;
            }
            let Some(bridge) = entry.pty_bridge.as_mut() else {
                return;
            };

            let mut ops = Vec::new();
            let mut meta = entry.timeline.meta.clone();
            let mut meta_changed = false;

            if let Some(file) = params
                .session_file
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let path = PathBuf::from(file);
                if bridge.session_file_hint.as_ref() != Some(&path) {
                    bridge.session_file_hint = Some(path);
                }
                if meta.provider_session_file.as_deref() != Some(file) {
                    meta.provider_session_file = Some(file.to_string());
                    meta_changed = true;
                }
            }
            if let Some(sid) = params
                .session_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if meta.provider_session_id.as_deref() != Some(sid) {
                    meta.provider_session_id = Some(sid.to_string());
                    meta_changed = true;
                }
            }
            if let Some(model_raw) = params.model.as_deref() {
                if let Some(model_info) = model_info_from_string(model_raw) {
                    if meta.current_model.as_ref() != Some(&model_info) {
                        meta.current_model = Some(model_info);
                        meta_changed = true;
                    }
                }
            }

            let is_jsonl_backed = matches!(bridge.agent_type, AgentType::Pi | AgentType::Pie)
                || bridge.session_file_hint.is_some();

            match params.event.as_str() {
                "meta" => {}
                "preview" => {
                    if let Some(preview) = params
                        .streaming_preview
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        bridge.has_live_preview = true;
                        if !meta.is_streaming {
                            meta.is_streaming = true;
                            meta_changed = true;
                        }
                        ops.push(ChatDeltaOp::UpsertEntry {
                            entry: TimelineEntry::AssistantText {
                                id: BRIDGE_LIVE_PREVIEW_ID.to_string(),
                                turn_id: None,
                                text: preview.to_string(),
                                streaming: true,
                                model: meta.current_model.as_ref().map(|m| m.name.clone()),
                                usage: None,
                                created_at_ms: now_ms(),
                            },
                        });
                    }
                }
                "working" => {
                    if !meta.is_streaming {
                        meta.is_streaming = true;
                        meta_changed = true;
                    }
                    if let Some(old_approval) = bridge.bridge_approval_id.take() {
                        ops.push(ChatDeltaOp::ApprovalResolved {
                            approval_id: old_approval,
                        });
                    }
                }
                "blocked" => {
                    if meta.is_streaming {
                        meta.is_streaming = false;
                        meta_changed = true;
                    }
                    let question = params
                        .message
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or("等待用户确认")
                        .to_string();
                    if let Some(old_approval) = bridge.bridge_approval_id.take() {
                        ops.push(ChatDeltaOp::ApprovalResolved {
                            approval_id: old_approval,
                        });
                    }
                    let approval_id = format!("pty-approval-{}", ulid::Ulid::new());
                    bridge.bridge_approval_id = Some(approval_id.clone());
                    ops.push(ChatDeltaOp::ApprovalRequested {
                        approval: PendingApproval {
                            id: approval_id,
                            agent: params.agent.clone(),
                            source: ApprovalSource::PiExtensionUi,
                            method: "confirm".to_string(),
                            title: "终端 Agent 等待确认".to_string(),
                            message: Some(question),
                            tool_name: None,
                            command_or_input: None,
                            options: vec![
                                ApprovalOption {
                                    option_id: "allow_once".to_string(),
                                    label: "允许 (y)".to_string(),
                                    kind: Some("allow_once".to_string()),
                                },
                                ApprovalOption {
                                    option_id: "reject_once".to_string(),
                                    label: "拒绝 (n)".to_string(),
                                    kind: Some("reject_once".to_string()),
                                },
                            ],
                            prefill: Some(String::new()),
                            created_at_ms: now_ms(),
                        },
                    });
                }
                "done" => {
                    if meta.is_streaming {
                        meta.is_streaming = false;
                        meta_changed = true;
                    }
                    if let Some(old_approval) = bridge.bridge_approval_id.take() {
                        ops.push(ChatDeltaOp::ApprovalResolved {
                            approval_id: old_approval,
                        });
                    }
                    if !is_jsonl_backed {
                        if bridge.has_live_preview {
                            bridge.has_live_preview = false;
                            ops.push(ChatDeltaOp::RemoveEntry {
                                entry_id: BRIDGE_LIVE_PREVIEW_ID.to_string(),
                            });
                        }
                        let final_text = params
                            .streaming_preview
                            .as_deref()
                            .or(params.message.as_deref())
                            .map(str::trim)
                            .filter(|s| !s.is_empty() && *s != "任务已完成");
                        if let Some(text) = final_text {
                            ops.push(ChatDeltaOp::UpsertEntry {
                                entry: TimelineEntry::AssistantText {
                                    id: format!("bridge-done-{}", ulid::Ulid::new()),
                                    turn_id: None,
                                    text: text.to_string(),
                                    streaming: false,
                                    model: meta.current_model.as_ref().map(|m| m.name.clone()),
                                    usage: None,
                                    created_at_ms: now_ms(),
                                },
                            });
                        }
                    }
                    ops.push(ChatDeltaOp::TurnFinished {
                        turn_id: None,
                        error: None,
                    });
                }
                "idle" => {
                    if meta.is_streaming {
                        meta.is_streaming = false;
                        meta_changed = true;
                    }
                    if let Some(old_approval) = bridge.bridge_approval_id.take() {
                        ops.push(ChatDeltaOp::ApprovalResolved {
                            approval_id: old_approval,
                        });
                    }
                    if bridge.has_live_preview {
                        bridge.has_live_preview = false;
                        ops.push(ChatDeltaOp::RemoveEntry {
                            entry_id: BRIDGE_LIVE_PREVIEW_ID.to_string(),
                        });
                    }
                    ops.push(ChatDeltaOp::TurnFinished {
                        turn_id: None,
                        error: None,
                    });
                }
                "failed" | "error" => {
                    if meta.is_streaming {
                        meta.is_streaming = false;
                        meta_changed = true;
                    }
                    if let Some(old_approval) = bridge.bridge_approval_id.take() {
                        ops.push(ChatDeltaOp::ApprovalResolved {
                            approval_id: old_approval,
                        });
                    }
                    if bridge.has_live_preview {
                        bridge.has_live_preview = false;
                        ops.push(ChatDeltaOp::RemoveEntry {
                            entry_id: BRIDGE_LIVE_PREVIEW_ID.to_string(),
                        });
                    }
                    ops.push(ChatDeltaOp::TurnFinished {
                        turn_id: None,
                        error: params.message.clone(),
                    });
                }
                _ => {}
            }

            if meta_changed {
                ops.insert(0, ChatDeltaOp::MetaUpdated { meta });
            }
            (ops, Arc::clone(&bridge.wake_notify))
        };

        for op in ops {
            self.emit_delta_op(&params.agent, op).await;
        }
        wake.notify_one();
    }

    async fn watch_pty_bridge_transcript(
        self: Arc<Self>,
        agent: AgentId,
        wake: Arc<Notify>,
        mut watcher: Option<transcript::IncrementalPiTranscript>,
    ) {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(BRIDGE_POLL_INTERVAL) => {}
                _ = wake.notified() => {}
            }

            let probe = {
                let map = self.sessions.lock().await;
                let Some(entry) = map.get(&agent) else {
                    break;
                };
                if entry.driver.is_some() {
                    break;
                }
                let Some(bridge) = entry.pty_bridge.as_ref() else {
                    break;
                };
                let claimed: Vec<PathBuf> = map
                    .iter()
                    .filter(|(id, _)| *id != &agent)
                    .filter_map(|(_, e)| {
                        e.pty_bridge
                            .as_ref()
                            .and_then(|b| b.session_file_hint.clone())
                            .or_else(|| {
                                e.timeline
                                    .meta
                                    .provider_session_file
                                    .as_deref()
                                    .map(PathBuf::from)
                            })
                    })
                    .collect();
                (
                    bridge.agent_type,
                    bridge.cwd.clone(),
                    bridge.fallback_cwd.clone(),
                    bridge.started_secs,
                    bridge.proc_start_secs,
                    bridge.allow_older_fallback,
                    bridge.session_file_hint.clone(),
                    claimed,
                )
            };

            let (
                agent_type,
                cwd,
                fallback_cwd,
                started_secs,
                proc_start_secs,
                allow_older_fallback,
                hint,
                claimed_files,
            ) = probe;
            let resolved_file = hint.or_else(|| {
                if matches!(agent_type, AgentType::Pi | AgentType::Pie) {
                    let effective_floor = if allow_older_fallback {
                        started_secs.saturating_sub(3600)
                    } else {
                        proc_start_secs.unwrap_or(started_secs)
                    };
                    transcript::discover_pi_session_file_with_context(
                        &cwd,
                        fallback_cwd.as_deref(),
                        agent_type == AgentType::Pie,
                        effective_floor,
                        proc_start_secs,
                        allow_older_fallback,
                        &claimed_files,
                    )
                } else {
                    None
                }
            });

            let Some(file_path) = resolved_file else {
                continue;
            };

            // Pin the discovered file on the bridge so sibling sessions do not claim it.
            {
                let mut map = self.sessions.lock().await;
                if let Some(bridge) = map.get_mut(&agent).and_then(|e| e.pty_bridge.as_mut()) {
                    if bridge.session_file_hint.is_none() {
                        bridge.session_file_hint = Some(file_path.clone());
                    }
                }
            }

            match watcher.as_mut() {
                None => {
                    watcher = Some(transcript::IncrementalPiTranscript::new(file_path));
                }
                Some(existing) if existing.file_path() != file_path => {
                    existing.switch_file(file_path);
                }
                Some(_) => {}
            }

            let Some(active_watcher) = watcher.as_mut() else {
                continue;
            };
            let poll = match active_watcher.poll_deltas() {
                Ok(p) => p,
                Err(err) => {
                    tracing::debug!(%agent, %err, "incremental transcript poll error");
                    continue;
                }
            };

            if !poll.reset
                && !poll.meta_changed
                && poll.upserted_entries.is_empty()
                && poll.turn_state.is_none()
            {
                continue;
            }

            self.apply_bridge_transcript_poll(&agent, poll).await;
        }
    }

    /// Marks a PTY-bridge session as no longer streaming (e.g. when terminal screen detection
    /// observes the agent returning to Done / Idle / Failed without an extension hook).
    pub async fn mark_pty_bridge_idle(&self, agent: &AgentId) {
        let ops = {
            let mut map = self.sessions.lock().await;
            let Some(entry) = map.get_mut(agent) else {
                return;
            };
            let Some(bridge) = entry.pty_bridge.as_mut() else {
                return;
            };
            if !entry.timeline.meta.is_streaming && !bridge.has_live_preview {
                return;
            }
            let mut ops = Vec::new();
            if bridge.has_live_preview {
                bridge.has_live_preview = false;
                ops.push(ChatDeltaOp::RemoveEntry {
                    entry_id: BRIDGE_LIVE_PREVIEW_ID.to_string(),
                });
            }
            let mut next_meta = entry.timeline.meta.clone();
            next_meta.is_streaming = false;
            ops.push(ChatDeltaOp::MetaUpdated { meta: next_meta });
            ops.push(ChatDeltaOp::TurnFinished {
                turn_id: None,
                error: None,
            });
            ops
        };
        for op in ops {
            self.emit_delta_op(agent, op).await;
        }
    }

    async fn apply_bridge_transcript_poll(
        &self,
        agent: &AgentId,
        poll: transcript::TranscriptPoll,
    ) {
        let (ops, title_notice) = {
            let mut map = self.sessions.lock().await;
            let Some(entry) = map.get_mut(agent) else {
                return;
            };
            let Some(bridge) = entry.pty_bridge.as_mut() else {
                return;
            };

            let mut ops = Vec::new();
            if poll.reset {
                bridge.pending_user_echoes.clear();
                bridge.has_live_preview = false;
                bridge.last_auto_title = None;
                ops.push(ChatDeltaOp::Reset);
            }

            let title_notice = poll.session_title.and_then(|title| {
                if bridge.last_auto_title.as_deref() != Some(title.as_str()) {
                    bridge.last_auto_title = Some(title.clone());
                    Some(title)
                } else {
                    None
                }
            });

            let mut merged_meta = entry.timeline.meta.clone();
            let mut meta_dirty =
                merge_transcript_meta(&mut merged_meta, &poll.meta) && poll.meta_changed;
            let mut emit_turn_finished = false;

            if poll.reset && merged_meta.is_streaming {
                merged_meta.is_streaming = false;
                meta_dirty = true;
            }

            match poll.turn_state {
                Some(transcript::TranscriptTurnState::Completed) => {
                    if bridge.has_live_preview {
                        bridge.has_live_preview = false;
                        ops.push(ChatDeltaOp::RemoveEntry {
                            entry_id: BRIDGE_LIVE_PREVIEW_ID.to_string(),
                        });
                    }
                    if merged_meta.is_streaming {
                        merged_meta.is_streaming = false;
                        meta_dirty = true;
                        emit_turn_finished = true;
                    }
                }
                Some(transcript::TranscriptTurnState::Active)
                    if !poll.initial_load && !poll.reset && !merged_meta.is_streaming =>
                {
                    merged_meta.is_streaming = true;
                    meta_dirty = true;
                }
                _ => {}
            }

            if meta_dirty {
                ops.push(ChatDeltaOp::MetaUpdated { meta: merged_meta });
            }

            let now = now_ms();
            // Prune stale optimistic user echoes older than 60s.
            let mut stale_ids = Vec::new();
            bridge.pending_user_echoes.retain(|(id, _, ts)| {
                if now.saturating_sub(*ts) > 60_000 {
                    stale_ids.push(id.clone());
                    false
                } else {
                    true
                }
            });
            for id in stale_ids {
                ops.push(ChatDeltaOp::RemoveEntry { entry_id: id });
            }

            for item in poll.upserted_entries {
                match &item {
                    TimelineEntry::UserMessage { text, .. } => {
                        let norm = normalize_echo_text(text);
                        if let Some(pos) = bridge
                            .pending_user_echoes
                            .iter()
                            .position(|(_, pending_norm, _)| *pending_norm == norm)
                        {
                            let (pending_id, _, _) = bridge.pending_user_echoes.remove(pos);
                            ops.push(ChatDeltaOp::RemoveEntry {
                                entry_id: pending_id,
                            });
                        }
                    }
                    TimelineEntry::AssistantText { .. } if bridge.has_live_preview => {
                        bridge.has_live_preview = false;
                        ops.push(ChatDeltaOp::RemoveEntry {
                            entry_id: BRIDGE_LIVE_PREVIEW_ID.to_string(),
                        });
                    }
                    _ => {}
                }
                ops.push(ChatDeltaOp::UpsertEntry { entry: item });
            }

            if emit_turn_finished {
                ops.push(ChatDeltaOp::TurnFinished {
                    turn_id: None,
                    error: None,
                });
            }
            (ops, title_notice)
        };

        for op in ops {
            self.emit_delta_op(agent, op).await;
        }
        if let Some(title) = title_notice {
            let _ = self.notices.send(ChatHostNotice::TitleChanged {
                agent: agent.clone(),
                title,
            });
        }
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
                pty_bridge: None,
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
            .filter(|(_, entry)| {
                entry.pty_bridge.is_none()
                    && entry.timeline.meta.driver != SessionDriverMode::TerminalPty
            })
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
            // Dropping `entry.pty_bridge` aborts any running watcher task.
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
                entry.pty_bridge = None;
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
        Some(build_wire_history(agent, &entry.timeline))
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
        Ok(build_wire_history(agent, &entry.timeline))
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
            ChatDriverKind::Acp if launch_spec.agent_type == AgentType::Agy => {
                crate::agy_stream::spawn(launch_spec, meta, ev_tx)
            }
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
        let bridge_ctx = {
            let map = self.sessions.lock().await;
            map.get(agent).and_then(|entry| {
                if entry.driver.is_none() {
                    entry.pty_bridge.as_ref().map(|b| {
                        (
                            Arc::clone(&b.write_lock),
                            Arc::clone(&b.wake_notify),
                            b.agent_type,
                            b.session_file_hint.is_some(),
                        )
                    })
                } else {
                    None
                }
            })
        };

        if let Some((write_lock, wake_notify, agent_type, has_file_hint)) = bridge_ctx {
            let writer = self
                .get_pty_writer()
                .ok_or_else(|| anyhow::anyhow!("PTY bridge writer unavailable"))?;
            if !writer.is_agent_confirmed(agent).await {
                self.set_pty_bridge_alive(agent, false).await;
                anyhow::bail!("terminal session returned to bare shell; chat input disabled");
            }

            let _guard = write_lock.lock().await;
            let sanitized = pty_bridge::sanitize_pty_paste_text(&text);
            let display_text = sanitized.trim().to_string();
            let is_jsonl_backed =
                matches!(agent_type, AgentType::Pi | AgentType::Pie) || has_file_hint;

            let pending_id = if !display_text.is_empty() || !attachments.is_empty() {
                let id = if is_jsonl_backed {
                    format!("{BRIDGE_PENDING_USER_PREFIX}{}", ulid::Ulid::new())
                } else {
                    format!("bridge-user-{}", ulid::Ulid::new())
                };
                let created_at_ms = now_ms();
                if is_jsonl_backed {
                    let mut map = self.sessions.lock().await;
                    if let Some(bridge) = map.get_mut(agent).and_then(|e| e.pty_bridge.as_mut()) {
                        bridge.pending_user_echoes.push((
                            id.clone(),
                            normalize_echo_text(&display_text),
                            created_at_ms,
                        ));
                    }
                }
                self.emit_delta_op(
                    agent,
                    ChatDeltaOp::UpsertEntry {
                        entry: TimelineEntry::UserMessage {
                            id: id.clone(),
                            turn_id: None,
                            text: display_text.clone(),
                            attachments: attachments.clone(),
                            created_at_ms,
                        },
                    },
                )
                .await;
                Some(id)
            } else {
                None
            };

            if let Err(err) =
                pty_bridge::submit_prompt_to_pty(writer.as_ref(), agent, &text, &attachments).await
            {
                if let Some(id) = pending_id {
                    let mut map = self.sessions.lock().await;
                    if let Some(bridge) = map.get_mut(agent).and_then(|e| e.pty_bridge.as_mut()) {
                        bridge.pending_user_echoes.retain(|(pid, _, _)| pid != &id);
                    }
                    drop(map);
                    self.emit_delta_op(agent, ChatDeltaOp::RemoveEntry { entry_id: id })
                        .await;
                }
                if err.to_string().contains("bare shell") {
                    self.set_pty_bridge_alive(agent, false).await;
                }
                return Err(err);
            }

            let next_meta = {
                let mut map = self.sessions.lock().await;
                map.get_mut(agent).and_then(|e| {
                    if !e.timeline.meta.is_streaming {
                        e.timeline.meta.is_streaming = true;
                        Some(e.timeline.meta.clone())
                    } else {
                        None
                    }
                })
            };
            if let Some(meta) = next_meta {
                self.emit_delta_op(agent, ChatDeltaOp::MetaUpdated { meta })
                    .await;
            }

            wake_notify.notify_one();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(160)).await;
                wake_notify.notify_one();
            });
            return Ok(());
        }

        let driver = self.ensure_driver_active(agent).await?;
        driver.send(DriverCmd::SendPrompt {
            text,
            attachments,
            streaming_behavior,
        })
    }

    pub async fn abort(&self, agent: &AgentId) -> anyhow::Result<()> {
        let (driver, bridge_ctx) = {
            let map = self.sessions.lock().await;
            let entry = map.get(agent);
            let driver = entry.and_then(|e| e.driver.clone());
            let bridge = entry.and_then(|e| {
                if e.driver.is_none() {
                    e.pty_bridge.as_ref().map(|b| {
                        (
                            Arc::clone(&b.write_lock),
                            Arc::clone(&b.wake_notify),
                            b.agent_type,
                        )
                    })
                } else {
                    None
                }
            });
            (driver, bridge)
        };
        if let Some(driver) = driver {
            driver.send(DriverCmd::Abort)?;
            return Ok(());
        }
        if let Some((write_lock, wake_notify, agent_type)) = bridge_ctx {
            let writer = self
                .get_pty_writer()
                .ok_or_else(|| anyhow::anyhow!("PTY bridge writer unavailable"))?;
            if !writer.is_agent_confirmed(agent).await {
                self.set_pty_bridge_alive(agent, false).await;
                anyhow::bail!("terminal session returned to bare shell; abort disabled");
            }
            let _guard = write_lock.lock().await;
            writer
                .write_bytes(agent, pty_bridge::abort_bytes_for_agent(agent_type))
                .await?;
            wake_notify.notify_one();
        }
        Ok(())
    }

    pub async fn delete_queued_message(
        &self,
        agent: &AgentId,
        message_id: Option<String>,
    ) -> anyhow::Result<()> {
        let driver = {
            let map = self.sessions.lock().await;
            map.get(agent).and_then(|e| e.driver.clone())
        };
        if let Some(driver) = driver {
            driver.send(DriverCmd::DeleteQueuedMessage { message_id })?;
        }
        Ok(())
    }

    pub async fn approve(
        &self,
        agent: &AgentId,
        approval_id: String,
        reply: ApprovalReply,
    ) -> anyhow::Result<()> {
        let (driver, bridge_ctx) = {
            let map = self.sessions.lock().await;
            let entry = map.get(agent);
            let driver = entry.and_then(|e| e.driver.clone());
            let bridge = entry.and_then(|e| {
                if e.driver.is_none() {
                    e.pty_bridge
                        .as_ref()
                        .map(|b| (Arc::clone(&b.write_lock), Arc::clone(&b.wake_notify)))
                } else {
                    None
                }
            });
            (driver, bridge)
        };
        if let Some(driver) = driver {
            return driver.send(DriverCmd::RespondApproval { approval_id, reply });
        }
        if let Some((write_lock, wake_notify)) = bridge_ctx {
            let writer = self
                .get_pty_writer()
                .ok_or_else(|| anyhow::anyhow!("PTY bridge writer unavailable"))?;
            let _guard = write_lock.lock().await;
            if let Err(err) =
                pty_bridge::submit_approval_to_pty(writer.as_ref(), agent, &reply).await
            {
                if err.to_string().contains("bare shell") {
                    self.set_pty_bridge_alive(agent, false).await;
                }
                return Err(err);
            }
            {
                let mut map = self.sessions.lock().await;
                if let Some(bridge) = map.get_mut(agent).and_then(|e| e.pty_bridge.as_mut()) {
                    if bridge.bridge_approval_id.as_deref() == Some(approval_id.as_str()) {
                        bridge.bridge_approval_id = None;
                    }
                }
            }
            self.emit_delta_op(agent, ChatDeltaOp::ApprovalResolved { approval_id })
                .await;
            wake_notify.notify_one();
            return Ok(());
        }
        Err(anyhow::anyhow!("driver not running for {agent}"))
    }

    pub async fn configure(
        self: &Arc<Self>,
        agent: &AgentId,
        provider: Option<String>,
        model_id: Option<String>,
        thinking_level: Option<String>,
    ) -> anyhow::Result<()> {
        let is_pty_bridge = {
            let map = self.sessions.lock().await;
            map.get(agent)
                .is_some_and(|e| e.driver.is_none() && e.pty_bridge.is_some())
        };
        if is_pty_bridge {
            anyhow::bail!("model configuration requires structured chat driver");
        }
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
                    self.emit_delta_op(&agent, op).await;
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

/// Merges transcript-derived metadata (session id/file, model, thinking level) into live bridge
/// metadata without clobbering `supports_chat_input`, `has_pty`, or `is_streaming`.
fn merge_transcript_meta(dst: &mut ChatSessionMeta, src: &ChatSessionMeta) -> bool {
    let mut changed = false;
    if src.provider_session_id.is_some() && dst.provider_session_id != src.provider_session_id {
        dst.provider_session_id = src.provider_session_id.clone();
        changed = true;
    }
    if src.provider_session_file.is_some() && dst.provider_session_file != src.provider_session_file
    {
        dst.provider_session_file = src.provider_session_file.clone();
        changed = true;
    }
    if src.current_model.is_some() && dst.current_model != src.current_model {
        dst.current_model = src.current_model.clone();
        changed = true;
    }
    if src.thinking_level.is_some() && dst.thinking_level != src.thinking_level {
        dst.thinking_level = src.thinking_level.clone();
        changed = true;
    }
    changed
}

const WIRE_HISTORY_BUDGET_BYTES: usize = 700 * 1024;
const WIRE_TOOL_OUTPUT_MAX_BYTES: usize = 16 * 1024;
const WIRE_TOOL_INPUT_MAX_BYTES: usize = 8 * 1024;
const WIRE_DIFF_SIDE_MAX_BYTES: usize = 24 * 1024;
const WIRE_THINKING_MAX_BYTES: usize = 24 * 1024;
const WIRE_TEXT_MAX_BYTES: usize = 96 * 1024;
const WIRE_ATTACHMENT_B64_MAX_BYTES: usize = 260 * 1024;

fn truncate_utf8(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = text[..end].to_string();
    out.push_str("\n… [内容过长已截断]");
    out
}

/// Keeps small image attachments lossless and downscales oversized photos into a crisp JPEG
/// preview so every photo displays over 1 MiB RPC frames (local, remote SSH, and mobile Relay)
/// without ever being stripped to `None`.
fn compact_attachment_for_wire(mut att: ChatAttachment) -> ChatAttachment {
    let Some(raw_b64) = att.data_b64.as_deref() else {
        return att;
    };
    if raw_b64.len() <= WIRE_ATTACHMENT_B64_MAX_BYTES {
        return att;
    }
    if !att.mime_type.starts_with("image/") {
        att.data_b64 = None;
        return att;
    }
    use base64::Engine as _;
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(raw_b64.trim()) else {
        att.data_b64 = None;
        return att;
    };
    let Ok(img) = image::load_from_memory(&bytes) else {
        att.data_b64 = None;
        return att;
    };
    for (max_dim, quality) in [(1280u32, 82u8), (900, 74), (640, 65)] {
        let resized = if img.width() > max_dim || img.height() > max_dim {
            img.resize(max_dim, max_dim, image::imageops::FilterType::Triangle)
        } else {
            img.clone()
        };
        let mut jpeg_bytes = Vec::new();
        let mut encoder =
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, quality);
        if encoder.encode_image(&resized.to_rgb8()).is_ok() {
            let encoded_b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg_bytes);
            if encoded_b64.len() <= WIRE_ATTACHMENT_B64_MAX_BYTES {
                att.mime_type = "image/jpeg".to_string();
                att.data_b64 = Some(encoded_b64);
                return att;
            }
        }
    }
    att.data_b64 = None;
    att
}

fn compact_entry_for_wire(entry: TimelineEntry) -> TimelineEntry {
    match entry {
        TimelineEntry::UserMessage {
            id,
            turn_id,
            text,
            attachments,
            created_at_ms,
        } => {
            let attachments = attachments
                .into_iter()
                .map(compact_attachment_for_wire)
                .collect();
            TimelineEntry::UserMessage {
                id,
                turn_id,
                text: truncate_utf8(&text, WIRE_TEXT_MAX_BYTES),
                attachments,
                created_at_ms,
            }
        }
        TimelineEntry::Thinking {
            id,
            turn_id,
            text,
            streaming,
            duration_ms,
            created_at_ms,
        } => TimelineEntry::Thinking {
            id,
            turn_id,
            text: truncate_utf8(&text, WIRE_THINKING_MAX_BYTES),
            streaming,
            duration_ms,
            created_at_ms,
        },
        TimelineEntry::AssistantText {
            id,
            turn_id,
            text,
            streaming,
            model,
            usage,
            created_at_ms,
        } => TimelineEntry::AssistantText {
            id,
            turn_id,
            text: truncate_utf8(&text, WIRE_TEXT_MAX_BYTES),
            streaming,
            model,
            usage,
            created_at_ms,
        },
        TimelineEntry::ToolCall {
            id,
            turn_id,
            call_id,
            name,
            kind,
            title,
            input,
            output,
            attachments,
            diff,
            exit_code,
            locations,
            state,
            is_error,
            duration_ms,
            created_at_ms,
        } => {
            let compact_input = match serde_json::to_string(&input) {
                Ok(raw) if raw.len() > WIRE_TOOL_INPUT_MAX_BYTES => {
                    serde_json::Value::String(truncate_utf8(&raw, WIRE_TOOL_INPUT_MAX_BYTES))
                }
                _ => input,
            };
            let compact_output = output.map(|o| truncate_utf8(&o, WIRE_TOOL_OUTPUT_MAX_BYTES));
            let compact_attachments = attachments
                .into_iter()
                .map(compact_attachment_for_wire)
                .collect();
            let compact_diff = diff.map(|mut d| {
                d.old_text = truncate_utf8(&d.old_text, WIRE_DIFF_SIDE_MAX_BYTES);
                d.new_text = truncate_utf8(&d.new_text, WIRE_DIFF_SIDE_MAX_BYTES);
                d
            });
            TimelineEntry::ToolCall {
                id,
                turn_id,
                call_id,
                name,
                kind,
                title,
                input: compact_input,
                output: compact_output,
                attachments: compact_attachments,
                diff: compact_diff,
                exit_code,
                locations,
                state,
                is_error,
                duration_ms,
                created_at_ms,
            }
        }
        other => other,
    }
}

/// Builds a wire-safe `ChatHistoryResult` guaranteed to serialize well below `MAX_FRAME` (1 MiB),
/// keeping the most recent timeline entries when long sessions exceed the single-frame budget.
fn build_wire_history(agent: &AgentId, timeline: &ChatTimelineState) -> ChatHistoryResult {
    let overhead = serde_json::to_vec(&(&timeline.meta, &timeline.pending_approvals))
        .map(|v| v.len())
        .unwrap_or(4096)
        + 4096;
    let budget = WIRE_HISTORY_BUDGET_BYTES.saturating_sub(overhead);

    let total = timeline.entries.len();
    let mut kept_rev: Vec<TimelineEntry> = Vec::new();
    let mut used_bytes: usize = 0;

    for raw in timeline.entries.iter().rev() {
        let compact = compact_entry_for_wire(raw.clone());
        let entry_bytes = serde_json::to_vec(&compact).map(|v| v.len()).unwrap_or(512) + 2;
        if !kept_rev.is_empty() && used_bytes + entry_bytes > budget {
            break;
        }
        used_bytes += entry_bytes;
        kept_rev.push(compact);
    }

    let omitted = total.saturating_sub(kept_rev.len());
    kept_rev.reverse();
    if omitted > 0 {
        kept_rev.insert(
            0,
            TimelineEntry::Notice {
                id: "wire-history-omitted".to_string(),
                turn_id: None,
                level: muxlane_core::chat::NoticeLevel::Info,
                text: format!(
                    "已加载最近 {} 条记录（更早的 {} 条历史已省略以适配传输上限）",
                    kept_rev.len(),
                    omitted
                ),
                created_at_ms: 0,
            },
        );
    }

    ChatHistoryResult {
        agent: agent.clone(),
        seq: timeline.seq,
        meta: timeline.meta.clone(),
        entries: kept_rev,
        pending_approvals: timeline.pending_approvals.clone(),
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

    #[tokio::test]
    async fn chat_manager_bridges_pty_two_way_with_incremental_jsonl_and_shell_guard() {
        use std::future::Future;
        use std::io::Write;
        use std::pin::Pin;
        use std::sync::atomic::{AtomicBool, Ordering};

        struct TestPtyWriter {
            confirmed: AtomicBool,
            writes: std::sync::Mutex<Vec<Vec<u8>>>,
        }

        impl PtyBridgeWriter for TestPtyWriter {
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

            fn notify_submitted(
                &self,
                _agent: &AgentId,
            ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
                Box::pin(async {})
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let jsonl = dir.path().join("pi-live.jsonl");
        std::fs::write(
            &jsonl,
            concat!(r#"{"type":"session","version":3,"id":"sess-live"}"#, "\n"),
        )
        .unwrap();

        let (notice_tx, _notice_rx) = mpsc::unbounded_channel();
        let manager = ChatManager::new(notice_tx);
        manager.set_journal_dir(dir.path().join("journals"));
        let writer = Arc::new(TestPtyWriter {
            confirmed: AtomicBool::new(true),
            writes: std::sync::Mutex::new(Vec::new()),
        });
        manager.set_pty_writer(writer.clone());

        let agent: AgentId = "pi-pty-1".into();
        let meta = manager
            .ensure_pty_bridge(
                &agent,
                AgentType::Pi,
                dir.path().to_path_buf(),
                0,
                true,
                None,
            )
            .await;
        assert_eq!(meta.driver, SessionDriverMode::TerminalPty);
        assert!(meta.supports_chat_input);
        assert!(meta.has_pty);
        // PTY bridge sessions must not be exported into headless chat persistence.
        assert!(manager.try_persisted_records().is_empty());

        let (sub_tx, mut sub_rx) = mpsc::channel(64);
        let initial = manager
            .subscribe(&agent, "sub-1".into(), sub_tx)
            .await
            .unwrap();
        assert_eq!(initial.meta.driver, SessionDriverMode::TerminalPty);

        // Hook reports exact session_file & model.
        manager
            .notify_pty_hook(&AgentReportParams {
                token: String::new(),
                agent: agent.clone(),
                event: "meta".into(),
                message: None,
                session_id: Some("sess-live".into()),
                session_file: Some(jsonl.display().to_string()),
                model: Some("anthropic/claude-sonnet".into()),
                streaming_preview: None,
            })
            .await;

        // Send multiline prompt from Chat UI -> writes Bracketed Paste + separate `\r` and emits optimistic echo.
        manager
            .send_prompt(&agent, "line 1\nline 2".into(), Vec::new(), None)
            .await
            .unwrap();
        {
            let writes = writer.writes.lock().unwrap();
            assert_eq!(writes.len(), 2);
            assert_eq!(writes[0], b"\x1b[200~line 1\nline 2\x1b[201~");
            assert_eq!(writes[1], b"\r");
        }
        let after_send = manager.history(&agent).await.unwrap();
        assert!(after_send.meta.is_streaming);
        assert!(after_send
            .entries
            .iter()
            .any(|e| e.id().starts_with(BRIDGE_PENDING_USER_PREFIX)));

        // Hook streams a live preview before disk flush.
        manager
            .notify_pty_hook(&AgentReportParams {
                token: String::new(),
                agent: agent.clone(),
                event: "preview".into(),
                streaming_preview: Some("Draft answer...".into()),
                ..Default::default()
            })
            .await;
        assert!(manager
            .history(&agent)
            .await
            .unwrap()
            .entries
            .iter()
            .any(|e| e.id() == BRIDGE_LIVE_PREVIEW_ID));

        // Simulate Pi flushing user + intermediate toolUse (turn still running -> is_streaming stays true).
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&jsonl)
                .unwrap();
            writeln!(
                f,
                r#"{{"type":"message","id":"u1","message":{{"role":"user","content":"line 1\nline 2","timestamp":1000}}}}"#
            )
            .unwrap();
            writeln!(
                f,
                r#"{{"type":"message","id":"a0","message":{{"role":"assistant","stopReason":"toolUse","content":[{{"type":"toolCall","id":"tc-live","name":"read","arguments":{{"path":"Cargo.toml"}}}}],"timestamp":1500}}}}"#
            )
            .unwrap();
        }
        manager.wake_pty_bridge(&agent).await;

        let tool_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let hist = manager.history(&agent).await.unwrap();
            if hist.entries.iter().any(|e| e.id() == "tool:tc-live") {
                assert!(
                    hist.meta.is_streaming,
                    "intermediate toolUse must keep is_streaming=true"
                );
                break;
            }
            assert!(tokio::time::Instant::now() < tool_deadline);
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        // Simulate toolResult + final assistant response with stopReason="stop" (NO hook "done" sent).
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&jsonl)
                .unwrap();
            writeln!(
                f,
                r#"{{"type":"message","id":"r0","message":{{"role":"toolResult","toolCallId":"tc-live","content":[{{"type":"text","text":"ok"}}],"isError":false,"timestamp":1800}}}}"#
            )
            .unwrap();
            writeln!(
                f,
                r#"{{"type":"message","id":"a1","message":{{"role":"assistant","stopReason":"stop","content":[{{"type":"text","text":"Final synced answer"}}],"timestamp":2000}}}}"#
            )
            .unwrap();
        }
        manager.wake_pty_bridge(&agent).await;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let hist = manager.history(&agent).await.unwrap();
            let has_real_user = hist.entries.iter().any(|e| e.id() == "pi-user-u1");
            let has_real_asst = hist.entries.iter().any(|e| {
                matches!(e, TimelineEntry::AssistantText { id, text, .. } if id == "pi-text-a1-0" && text == "Final synced answer")
            });
            let no_pending = !hist.entries.iter().any(|e| {
                e.id().starts_with(BRIDGE_PENDING_USER_PREFIX) || e.id() == BRIDGE_LIVE_PREVIEW_ID
            });
            if has_real_user && has_real_asst && no_pending && !hist.meta.is_streaming {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for bridged JSONL reconciliation + idle turn completion: is_streaming={}, entries={:?}",
                hist.meta.is_streaming,
                hist.entries
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(sub_rx.try_recv().is_ok());

        // Screen-idle fallback (`mark_pty_bridge_idle`) also clears is_streaming reliably.
        manager
            .notify_pty_hook(&AgentReportParams {
                token: String::new(),
                agent: agent.clone(),
                event: "working".into(),
                ..Default::default()
            })
            .await;
        assert!(manager.history(&agent).await.unwrap().meta.is_streaming);
        manager.mark_pty_bridge_idle(&agent).await;
        assert!(!manager.history(&agent).await.unwrap().meta.is_streaming);

        // Blocked hook creates inline approval; approving writes `y\r` into PTY.
        manager
            .notify_pty_hook(&AgentReportParams {
                token: String::new(),
                agent: agent.clone(),
                event: "blocked".into(),
                message: Some("Run tests?".into()),
                ..Default::default()
            })
            .await;
        let approval_id = manager
            .history(&agent)
            .await
            .unwrap()
            .pending_approvals
            .first()
            .unwrap()
            .id
            .clone();
        manager
            .approve(
                &agent,
                approval_id,
                ApprovalReply::Confirm { confirmed: true },
            )
            .await
            .unwrap();
        assert_eq!(writer.writes.lock().unwrap().last().unwrap(), b"y\r");

        // Abort sends Esc (`\x1b`) for Pi.
        manager.abort(&agent).await.unwrap();
        assert_eq!(writer.writes.lock().unwrap().last().unwrap(), b"\x1b");

        // Bare-shell guard rejects writes and locks `supports_chat_input = false`.
        writer.confirmed.store(false, Ordering::Release);
        let err = manager
            .send_prompt(&agent, "should fail".into(), Vec::new(), None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("bare shell"));
        assert!(
            !manager
                .history(&agent)
                .await
                .unwrap()
                .meta
                .supports_chat_input
        );
    }

    #[test]
    fn compact_attachment_for_wire_resizes_large_image_instead_of_dropping() {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
        use image::{ImageBuffer, Rgb};
        use std::io::Cursor;

        // Generate a noisy 1024x1024 PNG whose base64 exceeds WIRE_ATTACHMENT_B64_MAX_BYTES.
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(1024, 1024, |x, y| {
            let v = ((x.wrapping_mul(73) ^ y.wrapping_mul(151)) & 0xff) as u8;
            Rgb([v, v.wrapping_mul(3), v.wrapping_add(91)])
        });
        let mut png_bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)
            .unwrap();
        let raw_b64 = BASE64.encode(&png_bytes);
        assert!(
            raw_b64.len() > WIRE_ATTACHMENT_B64_MAX_BYTES,
            "test fixture should exceed wire budget (got {})",
            raw_b64.len()
        );

        let compacted = compact_attachment_for_wire(ChatAttachment {
            name: "photo.png".to_string(),
            path: None,
            mime_type: "image/png".to_string(),
            data_b64: Some(raw_b64),
        });
        let out_b64 = compacted
            .data_b64
            .expect("oversized image should be compressed, not dropped");
        assert!(out_b64.len() <= WIRE_ATTACHMENT_B64_MAX_BYTES);
        assert_eq!(compacted.mime_type, "image/jpeg");
        let decoded = BASE64.decode(out_b64.as_bytes()).unwrap();
        assert!(image::load_from_memory(&decoded).is_ok());
    }
}
