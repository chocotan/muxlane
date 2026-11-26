//! Pi / Pie `--mode rpc` 结构化驱动实现（严格基于 Pi RPC v1 LF-JSONL 规范）。
use crate::driver::{
    extract_text_blocks, read_lf_line, ChatDriverHandle, DriverCmd, DriverEvent, DriverLaunchSpec,
};
use crate::transcript::{infer_tool_diff, parse_pi_usage};
use anyhow::Context;
use muxlane_core::chat::{
    now_ms, ApprovalOption, ApprovalReply, ApprovalSource, BackgroundJob, ChatAttachment,
    ChatDeltaOp, ChatModelInfo, ChatSessionMeta, ChatSlashCommand, MessageUsage, NoticeLevel,
    PendingApproval, QueuedMessage, SessionDriverMode, SessionGoal, SessionStats, SessionSubagent,
    SessionTask, StreamingBehavior, TextDeltaKind, TimelineEntry, ToolCallKind, ToolCallState,
    ToolDiff,
};
use muxlane_core::model::AgentStatus;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::time::Instant;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

pub fn spawn(
    spec: DriverLaunchSpec,
    initial_meta: ChatSessionMeta,
    events: mpsc::UnboundedSender<DriverEvent>,
) -> ChatDriverHandle {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        if let Err(error) = run_pi_rpc(spec, initial_meta, cmd_rx, events.clone()).await {
            let _ = events.send(DriverEvent::Exited {
                error: Some(error.to_string()),
            });
        }
    });
    ChatDriverHandle::new(cmd_tx)
}

async fn run_pi_rpc(
    spec: DriverLaunchSpec,
    mut meta: ChatSessionMeta,
    mut cmd_rx: mpsc::UnboundedReceiver<DriverCmd>,
    events: mpsc::UnboundedSender<DriverEvent>,
) -> anyhow::Result<()> {
    meta.driver = SessionDriverMode::PiRpc;
    meta.supports_chat_input = true;
    meta.supports_terminal_switch = true;
    let _ = events.send(DriverEvent::Delta(ChatDeltaOp::MetaUpdated {
        meta: meta.clone(),
    }));

    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .env_remove("MUXLANE_KITTY_GRAPHICS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in &spec.env {
        command.env(k, v);
    }
    if let Some(model) = &spec.model {
        if !spec.args.iter().any(|a| a == "--model") {
            command.arg("--model").arg(model);
        }
    }
    if let Some(level) = &spec.thinking_level {
        if !spec.args.iter().any(|a| a == "--thinking") {
            command.arg("--thinking").arg(level);
        }
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to launch `{}` in --mode rpc", spec.program))?;
    let mut stdin = child.stdin.take().context("missing stdin")?;
    let stdout = child.stdout.take().context("missing stdout")?;
    let stderr = child.stderr.take();

    let (stderr_tx, mut stderr_rx) = mpsc::unbounded_channel::<String>();
    if let Some(stderr) = stderr {
        tokio::spawn(async move {
            let mut r = BufReader::new(stderr);
            while let Ok(Some(line)) = read_lf_line(&mut r).await {
                if !line.trim().is_empty() {
                    let _ = stderr_tx.send(line);
                }
            }
        });
    }

    let mut stdout_reader = BufReader::new(stdout);
    let mut translator = PiRpcTranslator::new(&spec.agent, meta, events.clone());

    // 启动初始化：查询会话状态、模型列表、思考级别与斜杠命令/技能目录。
    send_json_line(&mut stdin, json!({"id":"init-state","type":"get_state"})).await?;
    send_json_line(
        &mut stdin,
        json!({"id":"init-models","type":"get_available_models"}),
    )
    .await?;
    send_json_line(
        &mut stdin,
        json!({"id":"init-thinking","type":"get_available_thinking_levels"}),
    )
    .await?;
    send_json_line(
        &mut stdin,
        json!({"id":"init-commands","type":"get_commands"}),
    )
    .await?;

    let mut req_seq: u64 = 1;
    let mut recent_stderr: VecDeque<String> = VecDeque::with_capacity(20);
    let mut exit_requested = false;

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => match cmd {
                Some(DriverCmd::SendPrompt { text, attachments, streaming_behavior }) => {
                    let trimmed = text.trim();
                    if attachments.is_empty() && (trimmed == "/compact" || trimmed.starts_with("/compact ")) {
                        let id = format!("cmd-{req_seq}");
                        req_seq += 1;
                        let custom = trimmed.strip_prefix("/compact").map(str::trim).unwrap_or("");
                        let mut payload = serde_json::Map::new();
                        payload.insert("id".into(), Value::String(id));
                        payload.insert("type".into(), Value::String("compact".into()));
                        if !custom.is_empty() {
                            payload.insert("customInstructions".into(), Value::String(custom.to_string()));
                        }
                        translator.emit_notice(NoticeLevel::Info, "正在压缩上下文…".to_string());
                        send_json_line(&mut stdin, Value::Object(payload)).await?;
                        continue;
                    }
                    if attachments.is_empty() && trimmed == "/new" {
                        let id = format!("cmd-{req_seq}");
                        req_seq += 1;
                        send_json_line(&mut stdin, json!({"id": id, "type": "new_session"})).await?;
                        let state_id = format!("cmd-{req_seq}");
                        req_seq += 1;
                        send_json_line(&mut stdin, json!({"id": state_id, "type": "get_state"})).await?;
                        continue;
                    }

                    // Orca 规则：斜杠命令（非 /skill:...）作为控制命令执行，不预先插入乐观用户气泡；
                    // 若 Pi 将其视为真实对话回合，message_start(role="user") 会自然渲染真实消息。
                    let is_control_slash = attachments.is_empty()
                        && trimmed.starts_with('/')
                        && !trimmed.starts_with("/skill:");
                    let behavior = streaming_behavior.unwrap_or(StreamingBehavior::Steer);
                    if !is_control_slash {
                        if translator.meta.is_streaming {
                            translator.enqueue_outgoing_prompt(&text, &attachments, behavior);
                        } else {
                            translator.record_outgoing_prompt(&text, &attachments);
                        }
                    }
                    let id = format!("cmd-{req_seq}");
                    req_seq += 1;
                    let mut payload = serde_json::Map::new();
                    payload.insert("id".into(), Value::String(id));
                    payload.insert("type".into(), Value::String("prompt".into()));
                    payload.insert("message".into(), Value::String(text));
                    let images = attachments_to_pi_images(&attachments);
                    if !images.is_empty() {
                        payload.insert("images".into(), Value::Array(images));
                    }
                    if translator.meta.is_streaming {
                        let behavior_str = match behavior {
                            StreamingBehavior::Steer => "steer",
                            StreamingBehavior::FollowUp => "followUp",
                        };
                        payload.insert(
                            "streamingBehavior".into(),
                            Value::String(behavior_str.into()),
                        );
                    }
                    send_json_line(&mut stdin, Value::Object(payload)).await?;
                }
                Some(DriverCmd::Abort) => {
                    let id = format!("cmd-{req_seq}");
                    req_seq += 1;
                    send_json_line(&mut stdin, json!({"id": id, "type": "abort"})).await?;
                }
                Some(DriverCmd::DeleteQueuedMessage { message_id }) => {
                    let remaining = translator.remove_queued_message(message_id.as_deref());
                    let clear_id = format!("cmd-{req_seq}");
                    req_seq += 1;
                    send_json_line(&mut stdin, json!({"id": clear_id, "type": "clear_queue"}))
                        .await?;
                    if translator.meta.is_streaming {
                        for item in remaining {
                            let requeue_id = format!("cmd-{req_seq}");
                            req_seq += 1;
                            let cmd_type = match item.behavior {
                                StreamingBehavior::Steer => "steer",
                                StreamingBehavior::FollowUp => "follow_up",
                            };
                            let mut payload = serde_json::Map::new();
                            payload.insert("id".into(), Value::String(requeue_id));
                            payload.insert("type".into(), Value::String(cmd_type.into()));
                            payload.insert("message".into(), Value::String(item.text));
                            let images = attachments_to_pi_images(&item.attachments);
                            if !images.is_empty() {
                                payload.insert("images".into(), Value::Array(images));
                            }
                            send_json_line(&mut stdin, Value::Object(payload)).await?;
                        }
                    }
                }
                Some(DriverCmd::RespondApproval { approval_id, reply }) => {
                    translator.resolve_approval(&approval_id);
                    let mut resp = serde_json::Map::new();
                    resp.insert("type".into(), Value::String("extension_ui_response".into()));
                    resp.insert("id".into(), Value::String(approval_id));
                    match reply {
                        ApprovalReply::SelectedOption { option_id } => {
                            resp.insert("value".into(), Value::String(option_id));
                        }
                        ApprovalReply::Confirm { confirmed } => {
                            resp.insert("confirmed".into(), Value::Bool(confirmed));
                        }
                        ApprovalReply::Text { value } => {
                            resp.insert("value".into(), Value::String(value));
                        }
                        ApprovalReply::Cancel => {
                            resp.insert("cancelled".into(), Value::Bool(true));
                        }
                    }
                    send_json_line(&mut stdin, Value::Object(resp)).await?;
                }
                Some(DriverCmd::SetModel { provider, model_id }) => {
                    let id = format!("cmd-{req_seq}");
                    req_seq += 1;
                    let provider = provider.unwrap_or_default();
                    send_json_line(
                        &mut stdin,
                        json!({"id": id, "type": "set_model", "provider": provider, "modelId": model_id}),
                    )
                    .await?;
                    let state_id = format!("cmd-{req_seq}");
                    req_seq += 1;
                    send_json_line(&mut stdin, json!({"id": state_id, "type": "get_state"})).await?;
                    let think_id = format!("cmd-{req_seq}");
                    req_seq += 1;
                    send_json_line(
                        &mut stdin,
                        json!({"id": think_id, "type": "get_available_thinking_levels"}),
                    )
                    .await?;
                }
                Some(DriverCmd::SetThinkingLevel { level }) => {
                    let id = format!("cmd-{req_seq}");
                    req_seq += 1;
                    translator.meta.thinking_level = Some(level.clone());
                    let _ = events.send(DriverEvent::Delta(ChatDeltaOp::MetaUpdated {
                        meta: translator.meta.clone(),
                    }));
                    send_json_line(
                        &mut stdin,
                        json!({"id": id, "type": "set_thinking_level", "level": level}),
                    )
                    .await?;
                    let state_id = format!("cmd-{req_seq}");
                    req_seq += 1;
                    send_json_line(&mut stdin, json!({"id": state_id, "type": "get_state"})).await?;
                }
                Some(DriverCmd::Shutdown) | None => {
                    exit_requested = true;
                    break;
                }
            },
            line = read_lf_line(&mut stdout_reader) => match line {
                Ok(Some(raw)) => {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<Value>(trimmed) {
                        Ok(record) => translator.handle_record(record),
                        Err(error) => {
                            tracing::debug!(%error, "ignoring non-json stdout line from pi");
                        }
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = events.send(DriverEvent::Exited {
                        error: Some(format!("pi rpc read error: {error}")),
                    });
                    return Ok(());
                }
            },
            Some(err_line) = stderr_rx.recv() => {
                if recent_stderr.len() >= 20 {
                    recent_stderr.pop_front();
                }
                recent_stderr.push_back(err_line);
            }
        }
    }

    drop(stdin);
    if exit_requested {
        let _ = child.kill().await;
        let _ = events.send(DriverEvent::Exited { error: None });
        return Ok(());
    }
    let status = child.wait().await?;
    while let Ok(err_line) = stderr_rx.try_recv() {
        if recent_stderr.len() >= 20 {
            recent_stderr.pop_front();
        }
        recent_stderr.push_back(err_line);
    }
    let exit_err = if status.success() {
        None
    } else {
        let tail = recent_stderr.into_iter().collect::<Vec<_>>().join("\n");
        Some(if tail.is_empty() {
            format!("pi exited with status {status}")
        } else {
            tail
        })
    };
    let _ = events.send(DriverEvent::Exited { error: exit_err });
    Ok(())
}

async fn send_json_line(
    stdin: &mut tokio::process::ChildStdin,
    value: Value,
) -> anyhow::Result<()> {
    let mut line = serde_json::to_vec(&value)?;
    line.push(b'\n');
    stdin.write_all(&line).await?;
    stdin.flush().await?;
    Ok(())
}

fn attachments_to_pi_images(attachments: &[ChatAttachment]) -> Vec<Value> {
    attachments
        .iter()
        .filter_map(|att| {
            let data = att.data_b64.as_ref()?;
            Some(json!({
                "type": "image",
                "data": data,
                "mimeType": att.mime_type,
            }))
        })
        .collect()
}

#[derive(Debug, Clone)]
pub(crate) struct StoredQueuedPrompt {
    pub id: String,
    pub text: String,
    pub attachments: Vec<ChatAttachment>,
    pub behavior: StreamingBehavior,
}

pub(crate) struct PiRpcTranslator {
    agent: String,
    pub meta: ChatSessionMeta,
    events: mpsc::UnboundedSender<DriverEvent>,
    turn_seq: u64,
    msg_seq: u64,
    active_turn_id: Option<String>,
    active_msg_id: Option<String>,
    active_msg_started_at: Option<Instant>,
    active_msg_first_token_ms: Option<u64>,
    last_text_entry_id: Option<String>,
    active_blocks: HashMap<(usize, bool), String>,
    thinking_started_at: HashMap<String, Instant>,
    tool_started_at: HashMap<String, Instant>,
    tool_inputs: HashMap<String, (Value, Option<Box<ToolDiff>>)>,
    session_stats: SessionStats,
    goal_armed: bool,
    last_error: Option<String>,
    blocked_pending: bool,
    pending_user_echos: VecDeque<String>,
    stored_queue: Vec<StoredQueuedPrompt>,
    has_derived_title: bool,
    has_explicit_title: bool,
}

impl PiRpcTranslator {
    pub fn new(
        agent: &str,
        meta: ChatSessionMeta,
        events: mpsc::UnboundedSender<DriverEvent>,
    ) -> Self {
        let session_stats = meta
            .stats
            .as_deref()
            .cloned()
            .unwrap_or_default();
        let goal_armed = meta.goal.as_deref().is_some_and(|g| g.armed);
        Self {
            agent: agent.to_string(),
            meta,
            events,
            turn_seq: 0,
            msg_seq: 0,
            active_turn_id: None,
            active_msg_id: None,
            active_msg_started_at: None,
            active_msg_first_token_ms: None,
            last_text_entry_id: None,
            active_blocks: HashMap::new(),
            thinking_started_at: HashMap::new(),
            tool_started_at: HashMap::new(),
            tool_inputs: HashMap::new(),
            session_stats,
            goal_armed,
            last_error: None,
            blocked_pending: false,
            pending_user_echos: VecDeque::new(),
            stored_queue: Vec::new(),
            has_derived_title: false,
            has_explicit_title: false,
        }
    }

    pub fn record_outgoing_prompt(
        &mut self,
        text: &str,
        attachments: &[ChatAttachment],
    ) {
        self.turn_seq += 1;
        let id = format!("user-{}", ulid::Ulid::new());
        let trimmed = text.trim().to_string();
        if !trimmed.is_empty() {
            self.pending_user_echos.push_back(trimmed);
        }
        if !self.has_explicit_title && !self.has_derived_title {
            if let Some(title) = crate::driver::derive_prompt_title(text) {
                self.has_derived_title = true;
                let _ = self.events.send(DriverEvent::Title(title));
            }
        }
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                entry: TimelineEntry::UserMessage {
                    id,
                    turn_id: None,
                    text: text.to_string(),
                    attachments: attachments.to_vec(),
                    created_at_ms: now_ms(),
                },
            }));
    }

    pub fn enqueue_outgoing_prompt(
        &mut self,
        text: &str,
        attachments: &[ChatAttachment],
        behavior: StreamingBehavior,
    ) {
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() && attachments.is_empty() {
            return;
        }
        let id = format!("queued-{}", ulid::Ulid::new());
        self.stored_queue.push(StoredQueuedPrompt {
            id: id.clone(),
            text: text.to_string(),
            attachments: attachments.to_vec(),
            behavior,
        });
        self.sync_queued_meta();
        self.emit_meta();
    }

    pub fn remove_queued_message(&mut self, message_id: Option<&str>) -> Vec<StoredQueuedPrompt> {
        match message_id {
            Some(target_id) => {
                self.stored_queue.retain(|item| item.id != target_id);
            }
            None => {
                self.stored_queue.clear();
            }
        }
        self.sync_queued_meta();
        self.emit_meta();
        self.stored_queue.clone()
    }

    fn sync_queued_meta(&mut self) {
        self.meta.queued_messages = self
            .stored_queue
            .iter()
            .map(|item| QueuedMessage {
                id: item.id.clone(),
                text: item.text.clone(),
                behavior: item.behavior,
                attachments_count: item.attachments.len(),
            })
            .collect();
    }

    pub fn resolve_approval(&mut self, approval_id: &str) {
        self.blocked_pending = false;
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::ApprovalResolved {
                approval_id: approval_id.to_string(),
            }));
        if self.meta.is_streaming {
            let _ = self.events.send(DriverEvent::Status {
                status: AgentStatus::Working,
                message: None,
                reset_seen: false,
            });
        }
    }

    pub fn handle_record(&mut self, record: Value) {
        let Some(rec_type) = record.get("type").and_then(|v| v.as_str()) else {
            return;
        };
        match rec_type {
            "response" => self.handle_response(&record),
            "extension_ui_request" => self.handle_extension_ui(&record),
            "agent_start" => {
                self.turn_seq += 1;
                self.active_turn_id = Some(format!("pi-turn-{}", self.turn_seq));
                self.last_error = None;
                self.meta.last_error = None;
                self.meta.is_streaming = true;
                self.emit_meta();
                let _ = self.events.send(DriverEvent::Status {
                    status: AgentStatus::Working,
                    message: None,
                    reset_seen: false,
                });
            }
            "turn_start" => {
                if self.active_turn_id.is_none() {
                    self.turn_seq += 1;
                    self.active_turn_id = Some(format!("pi-turn-{}", self.turn_seq));
                }
                if !self.meta.is_streaming {
                    self.meta.is_streaming = true;
                    self.emit_meta();
                }
            }
            "message_start" => self.handle_message_start(&record),
            "message_update" => self.handle_message_update(&record),
            "message_end" => self.handle_message_end(&record),
            "tool_execution_start" => self.handle_tool_start(&record),
            "tool_execution_update" => self.handle_tool_update(&record),
            "tool_execution_end" => self.handle_tool_end(&record),
            "compaction_start" => {
                let reason = record
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("threshold");
                let _ = self
                    .events
                    .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                        entry: TimelineEntry::Notice {
                            id: format!("compact-start-{}", ulid::Ulid::new()),
                            turn_id: self.active_turn_id.clone(),
                            level: NoticeLevel::Compaction,
                            text: format!("Compacting context ({reason})…"),
                            created_at_ms: now_ms(),
                        },
                    }));
            }
            "compaction_end" => {
                if let Some(res) = record.get("result") {
                    let before = res
                        .get("tokensBefore")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let after = res
                        .get("estimatedTokensAfter")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let _ = self
                        .events
                        .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                            entry: TimelineEntry::Notice {
                                id: format!("compact-end-{}", ulid::Ulid::new()),
                                turn_id: self.active_turn_id.clone(),
                                level: NoticeLevel::Compaction,
                                text: format!("Context compacted: {before} -> {after} tokens"),
                                created_at_ms: now_ms(),
                            },
                        }));
                }
            }
            "auto_retry_start" => {
                let attempt = record.get("attempt").and_then(|v| v.as_u64()).unwrap_or(1);
                let max = record
                    .get("maxAttempts")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(3);
                let err = record
                    .get("errorMessage")
                    .and_then(|v| v.as_str())
                    .unwrap_or("transient error");
                let _ = self
                    .events
                    .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                        entry: TimelineEntry::Notice {
                            id: format!("retry-{}", ulid::Ulid::new()),
                            turn_id: self.active_turn_id.clone(),
                            level: NoticeLevel::Warning,
                            text: format!("Retrying ({attempt}/{max}): {err}"),
                            created_at_ms: now_ms(),
                        },
                    }));
            }
            "thinking_level_changed" => {
                if let Some(level) = record.get("level").and_then(|v| v.as_str()) {
                    self.meta.thinking_level = Some(level.to_string());
                    self.emit_meta();
                }
            }
            "session_info_changed" => {
                if let Some(name) = record.get("name").and_then(|v| v.as_str()) {
                    let _ = self.events.send(DriverEvent::Title(name.to_string()));
                }
            }
            "agent_end" => {
                let will_retry = record.get("willRetry").and_then(|v| v.as_bool());
                if will_retry == Some(false) {
                    self.finish_run(false);
                }
            }
            "agent_settled" => {
                let aborted = record
                    .get("aborted")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                self.finish_run(aborted);
            }
            _ => {}
        }
    }

    fn finish_run(&mut self, aborted: bool) {
        self.meta.is_streaming = false;
        self.meta.queued_messages.clear();
        self.stored_queue.clear();
        let turn_id = self.active_turn_id.take();
        self.active_msg_id = None;
        self.active_msg_started_at = None;
        self.active_msg_first_token_ms = None;
        self.last_text_entry_id = None;
        self.active_blocks.clear();

        let err = if aborted {
            None
        } else {
            self.last_error.take()
        };
        self.meta.last_error = err.clone();
        self.sync_pie_extension_state_from_session_file();
        self.emit_meta();

        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::TurnFinished {
                turn_id,
                error: err.clone(),
            }));

        if !self.blocked_pending {
            let status = if aborted {
                AgentStatus::Idle
            } else if err.is_some() {
                AgentStatus::Failed
            } else {
                AgentStatus::Done
            };
            let msg = if aborted {
                None
            } else {
                err.clone().or_else(|| Some("Task completed".to_string()))
            };
            let _ = self.events.send(DriverEvent::Status {
                status,
                message: msg,
                reset_seen: !aborted,
            });
        }
    }

    fn handle_response(&mut self, record: &Value) {
        let cmd = record.get("command").and_then(|v| v.as_str()).unwrap_or("");
        let ok = record
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if !ok {
            let err = record
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("command failed")
                .to_string();
            let _ = self
                .events
                .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                    entry: TimelineEntry::Notice {
                        id: format!("err-{}", ulid::Ulid::new()),
                        turn_id: self.active_turn_id.clone(),
                        level: NoticeLevel::Error,
                        text: format!("{cmd}: {err}"),
                        created_at_ms: now_ms(),
                    },
                }));
            return;
        }
        if cmd == "prompt" {
            if self.sync_pie_extension_state_from_session_file() {
                self.emit_meta();
            }
            return;
        }
        let Some(data) = record.get("data") else {
            return;
        };
        match cmd {
            "get_state" => {
                if let Some(m) = data.get("model").and_then(parse_pi_model) {
                    self.meta.current_model = Some(m);
                }
                if let Some(cw) = data
                    .pointer("/model/contextWindow")
                    .and_then(|v| v.as_u64())
                    .filter(|&v| v > 0)
                {
                    self.meta.context_window_tokens = Some(cw);
                }
                if let Some(lvl) = data.get("thinkingLevel").and_then(|v| v.as_str()) {
                    self.meta.thinking_level = Some(lvl.to_string());
                }
                if let Some(sf) = data.get("sessionFile").and_then(|v| v.as_str()) {
                    self.meta.provider_session_file = Some(sf.to_string());
                }
                if let Some(sid) = data.get("sessionId").and_then(|v| v.as_str()) {
                    self.meta.provider_session_id = Some(sid.to_string());
                }
                if let Some(s) = data.get("isStreaming").and_then(|v| v.as_bool()) {
                    self.meta.is_streaming = s;
                }
                self.sync_pie_extension_state_from_session_file();
                self.emit_meta();
            }
            "get_available_models" => {
                if let Some(arr) = data.get("models").and_then(|v| v.as_array()) {
                    self.meta.available_models = arr.iter().filter_map(parse_pi_model).collect();
                    self.emit_meta();
                }
            }
            "get_available_thinking_levels" | "cycle_thinking_level" => {
                if let Some(arr) = data.get("levels").and_then(|v| v.as_array()) {
                    self.meta.available_thinking_levels = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect();
                    self.emit_meta();
                } else if let Some(lvl) = data.get("level").and_then(|v| v.as_str()) {
                    self.meta.thinking_level = Some(lvl.to_string());
                    self.emit_meta();
                }
            }
            "set_model" => {
                if let Some(m) = parse_pi_model(data) {
                    self.meta.current_model = Some(m);
                }
                if let Some(cw) = data
                    .get("contextWindow")
                    .and_then(|v| v.as_u64())
                    .filter(|&v| v > 0)
                {
                    self.meta.context_window_tokens = Some(cw);
                }
                self.emit_meta();
            }
            "get_commands" => {
                if let Some(arr) = data.get("commands").and_then(|v| v.as_array()) {
                    self.meta.available_commands =
                        arr.iter().filter_map(parse_pi_command).collect();
                    self.emit_meta();
                }
            }
            "compact" => {
                let summary = data
                    .get("tokensBefore")
                    .and_then(|v| v.as_u64())
                    .map(|before| format!("上下文压缩完成（压缩前 {before} tokens）"))
                    .unwrap_or_else(|| "上下文压缩完成".to_string());
                self.emit_notice(NoticeLevel::Info, summary);
            }
            "new_session" => {
                let cancelled = data
                    .get("cancelled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if !cancelled {
                    self.session_stats = SessionStats::default();
                    self.goal_armed = false;
                    self.meta.stats = None;
                    self.meta.goal = None;
                    self.meta.tasks.clear();
                    self.meta.subagents.clear();
                    self.meta.background_jobs.clear();
                    self.meta.context_used_tokens = None;
                    self.meta.total_cost_usd = None;
                    self.emit_meta();
                    self.emit_notice(NoticeLevel::Info, "已开启新会话上下文".to_string());
                }
            }
            _ => {}
        }
    }

    fn handle_extension_ui(&mut self, record: &Value) {
        let id = record
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let method = record.get("method").and_then(|v| v.as_str()).unwrap_or("");
        match method {
            "select" | "confirm" | "input" | "editor" => {
                let title = record
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Agent requires input")
                    .to_string();
                let message = record
                    .get("message")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let prefill = record
                    .get("prefill")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let options = if method == "select" {
                    record
                        .get("options")
                        .and_then(|v| v.as_array())
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str())
                        .map(|opt| ApprovalOption {
                            option_id: opt.to_string(),
                            label: opt.to_string(),
                            kind: Some("select".into()),
                        })
                        .collect()
                } else if method == "confirm" {
                    vec![
                        ApprovalOption {
                            option_id: "confirm".into(),
                            label: "Allow".into(),
                            kind: Some("allow_once".into()),
                        },
                        ApprovalOption {
                            option_id: "deny".into(),
                            label: "Deny".into(),
                            kind: Some("reject_once".into()),
                        },
                    ]
                } else {
                    Vec::new()
                };
                self.blocked_pending = true;
                let _ = self
                    .events
                    .send(DriverEvent::Delta(ChatDeltaOp::ApprovalRequested {
                        approval: PendingApproval {
                            id,
                            agent: self.agent.clone(),
                            source: ApprovalSource::PiExtensionUi,
                            method: method.to_string(),
                            title: title.clone(),
                            message: message.clone(),
                            tool_name: None,
                            command_or_input: None,
                            options,
                            prefill,
                            created_at_ms: now_ms(),
                        },
                    }));
                let _ = self.events.send(DriverEvent::Status {
                    status: AgentStatus::Blocked,
                    message: Some(message.unwrap_or(title)),
                    reset_seen: true,
                });
            }
            "notify" => {
                let msg = record
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !msg.is_empty() {
                    let level = match record.get("notifyType").and_then(|v| v.as_str()) {
                        Some("warning") => NoticeLevel::Warning,
                        Some("error") => NoticeLevel::Error,
                        _ => NoticeLevel::Info,
                    };
                    let _ = self
                        .events
                        .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                            entry: TimelineEntry::Notice {
                                id: format!("notify-{}", ulid::Ulid::new()),
                                turn_id: self.active_turn_id.clone(),
                                level,
                                text: msg,
                                created_at_ms: now_ms(),
                            },
                        }));
                }
            }
            "setTitle" => {
                if let Some(title) = record.get("title").and_then(|v| v.as_str()) {
                    let _ = self.events.send(DriverEvent::Title(title.to_string()));
                }
            }
            _ => {}
        }
    }

    fn handle_message_start(&mut self, record: &Value) {
        let Some(msg) = record.get("message") else {
            return;
        };
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role == "user" {
            self.session_stats.turns = self.session_stats.turns.saturating_add(1);
            self.meta.stats = Some(Box::new(self.session_stats.clone()));
            let text = extract_text_blocks(msg).unwrap_or_default();
            let trimmed = text.trim();
            if self
                .pending_user_echos
                .front()
                .is_some_and(|pending| pending == trimmed)
            {
                self.pending_user_echos.pop_front();
                self.emit_meta();
                return;
            }
            let mut dequeued_attachments = Vec::new();
            if !self.stored_queue.is_empty() {
                if let Some(pos) = self
                    .stored_queue
                    .iter()
                    .position(|q| q.text.trim() == trimmed)
                {
                    let removed = self.stored_queue.remove(pos);
                    dequeued_attachments = removed.attachments;
                } else if !trimmed.is_empty() {
                    let removed = self.stored_queue.remove(0);
                    dequeued_attachments = removed.attachments;
                }
                self.sync_queued_meta();
            }
            self.emit_meta();
            if !trimmed.is_empty() || !dequeued_attachments.is_empty() {
                let _ = self
                    .events
                    .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                        entry: TimelineEntry::UserMessage {
                            id: format!("pi-user-{}", ulid::Ulid::new()),
                            turn_id: self.active_turn_id.clone(),
                            text,
                            attachments: dequeued_attachments,
                            created_at_ms: now_ms(),
                        },
                    }));
            }
            return;
        }
        if role == "custom" {
            self.handle_custom_message(msg);
            return;
        }
        if role == "assistant" {
            self.msg_seq += 1;
            self.active_msg_id = Some(format!("pi-msg-{}", self.msg_seq));
            self.active_msg_started_at = Some(Instant::now());
            self.active_msg_first_token_ms = None;
            self.last_text_entry_id = None;
            self.active_blocks.clear();
        }
    }

    fn handle_message_update(&mut self, record: &Value) {
        if let Some(usage) = record.get("usage") {
            let input = usage.get("input").and_then(|v| v.as_u64()).unwrap_or(0);
            let cache_read = usage.get("cacheRead").and_then(|v| v.as_u64()).unwrap_or(0);
            let cache_write = usage.get("cacheWrite").and_then(|v| v.as_u64()).unwrap_or(0);
            let total = input
                .saturating_add(cache_read)
                .saturating_add(cache_write);
            if total > 0 {
                self.meta.context_used_tokens = Some(total);
            }
        }
        let Some(ev) = record.get("assistantMessageEvent") else {
            return;
        };
        let ev_type = ev.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let idx = ev.get("contentIndex").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let msg_prefix = self
            .active_msg_id
            .clone()
            .unwrap_or_else(|| format!("pi-msg-{}", self.msg_seq.max(1)));

        match ev_type {
            "thinking_start" => {
                let id = format!("{msg_prefix}-think-{idx}");
                self.active_blocks.insert((idx, true), id.clone());
                self.thinking_started_at.insert(id.clone(), Instant::now());
                let _ = self
                    .events
                    .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                        entry: TimelineEntry::Thinking {
                            id,
                            turn_id: self.active_turn_id.clone(),
                            text: String::new(),
                            streaming: true,
                            duration_ms: None,
                            created_at_ms: now_ms(),
                        },
                    }));
            }
            "thinking_delta" => {
                let delta = ev
                    .get("delta")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if delta.is_empty() {
                    return;
                }
                if self.active_msg_first_token_ms.is_none() {
                    self.active_msg_first_token_ms =
                        self.active_msg_started_at.map(|t| t.elapsed().as_millis() as u64);
                }
                let id = self.ensure_thinking_block(idx, &msg_prefix);
                let _ = self
                    .events
                    .send(DriverEvent::Delta(ChatDeltaOp::AppendText {
                        entry_id: id,
                        delta,
                        kind: TextDeltaKind::Thinking,
                    }));
            }
            "thinking_end" => {
                let id = self.ensure_thinking_block(idx, &msg_prefix);
                if let Some(content) = ev.get("content").and_then(|v| v.as_str()) {
                    let _ = self
                        .events
                        .send(DriverEvent::Delta(ChatDeltaOp::ReplaceEntryText {
                            entry_id: id.clone(),
                            text: content.to_string(),
                            kind: TextDeltaKind::Thinking,
                            finish: true,
                        }));
                }
                let dur = self
                    .thinking_started_at
                    .remove(&id)
                    .map(|t| t.elapsed().as_millis() as u64);
                let _ = self
                    .events
                    .send(DriverEvent::Delta(ChatDeltaOp::FinishEntry {
                        entry_id: id,
                        duration_ms: dur,
                    }));
            }
            "text_start" => {
                let id = format!("{msg_prefix}-text-{idx}");
                self.active_blocks.insert((idx, false), id.clone());
                self.last_text_entry_id = Some(id.clone());
                let model = self.meta.current_model.as_ref().map(|m| m.name.clone());
                let _ = self
                    .events
                    .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                        entry: TimelineEntry::AssistantText {
                            id,
                            turn_id: self.active_turn_id.clone(),
                            text: String::new(),
                            streaming: true,
                            model,
                            usage: None,
                            created_at_ms: now_ms(),
                        },
                    }));
            }
            "text_delta" => {
                let delta = ev
                    .get("delta")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if delta.is_empty() {
                    return;
                }
                if self.active_msg_first_token_ms.is_none() {
                    self.active_msg_first_token_ms =
                        self.active_msg_started_at.map(|t| t.elapsed().as_millis() as u64);
                }
                let id = self.ensure_text_block(idx, &msg_prefix);
                let _ = self
                    .events
                    .send(DriverEvent::Delta(ChatDeltaOp::AppendText {
                        entry_id: id,
                        delta,
                        kind: TextDeltaKind::Assistant,
                    }));
            }
            "text_end" => {
                let id = self.ensure_text_block(idx, &msg_prefix);
                if let Some(content) = ev.get("content").and_then(|v| v.as_str()) {
                    let _ = self
                        .events
                        .send(DriverEvent::Delta(ChatDeltaOp::ReplaceEntryText {
                            entry_id: id.clone(),
                            text: content.to_string(),
                            kind: TextDeltaKind::Assistant,
                            finish: true,
                        }));
                } else {
                    let _ = self
                        .events
                        .send(DriverEvent::Delta(ChatDeltaOp::FinishEntry {
                            entry_id: id,
                            duration_ms: None,
                        }));
                }
            }
            "error" => {
                if let Some(msg) = ev.get("error").and_then(|v| v.as_str()) {
                    self.last_error = Some(msg.to_string());
                }
            }
            _ => {}
        }
    }

    fn handle_message_end(&mut self, record: &Value) {
        let Some(msg) = record.get("message") else {
            return;
        };
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role == "custom" {
            self.handle_custom_message(msg);
            return;
        }
        if role != "assistant" {
            return;
        }
        // Harvest toolCall arguments from the assistant message as a fallback cache for tool_execution_start/end.
        if let Some(content) = msg.get("content").and_then(|v| v.as_array()) {
            for block in content {
                if block.get("type").and_then(|v| v.as_str()) == Some("toolCall") {
                    let Some(cid) = block.get("id").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                    if let Some(args) = block
                        .get("arguments")
                        .or_else(|| block.get("args"))
                        .filter(|v| !v.is_null())
                    {
                        let diff = infer_tool_diff(name, args).map(Box::new);
                        self.tool_inputs
                            .entry(cid.to_string())
                            .or_insert_with(|| (args.clone(), diff));
                    }
                }
            }
        }
        if msg.get("stopReason").and_then(|v| v.as_str()) == Some("error") {
            let err = msg
                .get("errorMessage")
                .and_then(|v| v.as_str())
                .unwrap_or("Execution error")
                .to_string();
            self.last_error = Some(err);
        }
        // 确保未收到 *_end 的块也能收尾
        let ids: Vec<String> = self.active_blocks.values().cloned().collect();
        for id in ids {
            let dur = self
                .thinking_started_at
                .remove(&id)
                .map(|t| t.elapsed().as_millis() as u64);
            let _ = self
                .events
                .send(DriverEvent::Delta(ChatDeltaOp::FinishEntry {
                    entry_id: id,
                    duration_ms: dur,
                }));
        }
        self.active_blocks.clear();

        let duration_ms = self
            .active_msg_started_at
            .take()
            .map(|t| t.elapsed().as_millis() as u64);
        let ttft_ms = self.active_msg_first_token_ms.take();
        let parsed_usage = msg.get("usage").and_then(parse_pi_usage);

        self.session_stats.steps = self.session_stats.steps.saturating_add(1);
        if self.session_stats.turns == 0 {
            self.session_stats.turns = 1;
        }
        if let Some(dur) = duration_ms {
            self.session_stats.llm_ms = self.session_stats.llm_ms.saturating_add(dur);
        }
        if let Some(ttft) = ttft_ms {
            self.session_stats.ttft_ms = self.session_stats.ttft_ms.saturating_add(ttft);
            self.session_stats.ttft_steps = self.session_stats.ttft_steps.saturating_add(1);
        }

        let entry_usage = if let Some(mut u) = parsed_usage {
            u.duration_ms = duration_ms;
            u.ttft_ms = ttft_ms;
            self.session_stats.input_tokens =
                self.session_stats.input_tokens.saturating_add(u.input_tokens);
            self.session_stats.output_tokens = self
                .session_stats
                .output_tokens
                .saturating_add(u.output_tokens);
            self.session_stats.cache_read_tokens = self
                .session_stats
                .cache_read_tokens
                .saturating_add(u.cache_read_tokens);
            self.session_stats.cache_write_tokens = self
                .session_stats
                .cache_write_tokens
                .saturating_add(u.cache_write_tokens);
            self.session_stats.reasoning_tokens = self
                .session_stats
                .reasoning_tokens
                .saturating_add(u.reasoning_tokens);
            self.session_stats.total_tokens =
                self.session_stats.total_tokens.saturating_add(u.total_tokens);
            if let Some(cost) = u.cost_usd {
                self.session_stats.total_cost_usd =
                    Some(self.session_stats.total_cost_usd.unwrap_or(0.0) + cost);
            }
            let ctx = u
                .input_tokens
                .saturating_add(u.cache_read_tokens)
                .saturating_add(u.cache_write_tokens);
            if ctx > 0 {
                self.meta.context_used_tokens = Some(ctx);
            }
            if let Some(total_cost) = self.session_stats.total_cost_usd {
                self.meta.total_cost_usd = Some(total_cost);
            }
            Some(u)
        } else if duration_ms.is_some() || ttft_ms.is_some() {
            Some(MessageUsage {
                duration_ms,
                ttft_ms,
                ..Default::default()
            })
        } else {
            None
        };

        if let (Some(entry_id), Some(usage)) = (self.last_text_entry_id.take(), entry_usage) {
            let _ = self
                .events
                .send(DriverEvent::Delta(ChatDeltaOp::SetAssistantUsage {
                    entry_id,
                    usage: Box::new(usage),
                }));
        } else {
            self.last_text_entry_id = None;
        }
        self.meta.stats = Some(Box::new(self.session_stats.clone()));
        self.emit_meta();
    }

    fn handle_tool_start(&mut self, record: &Value) {
        let call_id = record
            .get("toolCallId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if call_id.is_empty() {
            return;
        }
        let name = record
            .get("toolName")
            .and_then(|v| v.as_str())
            .unwrap_or("tool")
            .to_string();
        let raw_args = record
            .get("args")
            .or_else(|| record.get("arguments"))
            .filter(|v| !v.is_null())
            .cloned();
        let (input, diff) = match raw_args {
            Some(args) => {
                let d = infer_tool_diff(&name, &args).map(Box::new);
                self.tool_inputs
                    .insert(call_id.clone(), (args.clone(), d.clone()));
                (args, d)
            }
            None => self
                .tool_inputs
                .get(&call_id)
                .cloned()
                .unwrap_or((Value::Null, None)),
        };
        self.tool_started_at.insert(call_id.clone(), Instant::now());
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                entry: TimelineEntry::ToolCall {
                    id: format!("tool:{call_id}"),
                    turn_id: self.active_turn_id.clone(),
                    call_id,
                    kind: ToolCallKind::from_tool_name(&name, None),
                    name,
                    title: None,
                    input,
                    output: None,
                    diff,
                    exit_code: None,
                    locations: Vec::new(),
                    state: ToolCallState::Running,
                    is_error: false,
                    duration_ms: None,
                    created_at_ms: now_ms(),
                },
            }));
    }

    fn handle_tool_update(&mut self, record: &Value) {
        let Some(call_id) = record.get("toolCallId").and_then(|v| v.as_str()) else {
            return;
        };
        if let Some(partial) = record.get("partialResult") {
            if let Some(text) = extract_text_blocks(partial) {
                let _ = self
                    .events
                    .send(DriverEvent::Delta(ChatDeltaOp::ReplaceEntryText {
                        entry_id: format!("tool:{call_id}"),
                        text,
                        kind: TextDeltaKind::ToolOutput,
                        finish: false,
                    }));
            }
        }
    }

    fn handle_tool_end(&mut self, record: &Value) {
        let call_id = record
            .get("toolCallId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if call_id.is_empty() {
            return;
        }
        let name = record
            .get("toolName")
            .and_then(|v| v.as_str())
            .unwrap_or("tool")
            .to_string();
        let cached = self.tool_inputs.remove(&call_id);
        let args = record
            .get("args")
            .or_else(|| record.get("arguments"))
            .filter(|v| !v.is_null())
            .cloned()
            .or_else(|| cached.as_ref().map(|(input, _)| input.clone()))
            .unwrap_or(Value::Null);
        let is_error = record
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let duration_ms = record
            .get("durationMs")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                self.tool_started_at
                    .remove(&call_id)
                    .map(|t| t.elapsed().as_millis() as u64)
            });
        let result = record.get("result");
        let output = result.and_then(extract_text_blocks);
        let exit_code = result
            .and_then(|r| r.pointer("/details/exitCode"))
            .and_then(|v| v.as_i64())
            .map(|v| v as i32);
        let diff = infer_tool_diff(&name, &args)
            .map(Box::new)
            .or_else(|| cached.and_then(|(_, d)| d));
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                entry: TimelineEntry::ToolCall {
                    id: format!("tool:{call_id}"),
                    turn_id: self.active_turn_id.clone(),
                    call_id,
                    kind: ToolCallKind::from_tool_name(&name, None),
                    name: name.clone(),
                    title: None,
                    input: args,
                    output,
                    diff,
                    exit_code,
                    locations: Vec::new(),
                    state: if is_error {
                        ToolCallState::Failed
                    } else {
                        ToolCallState::Completed
                    },
                    is_error,
                    duration_ms,
                    created_at_ms: now_ms(),
                },
            }));

        self.session_stats.tool_calls = self.session_stats.tool_calls.saturating_add(1);
        self.session_stats.tool_ms = self
            .session_stats
            .tool_ms
            .saturating_add(duration_ms.unwrap_or(0));
        self.meta.stats = Some(Box::new(self.session_stats.clone()));
        if !is_error {
            self.handle_pie_tool_result(&name, result);
        }
        self.emit_meta();
    }

    fn handle_custom_message(&mut self, msg: &Value) {
        let custom_type = msg
            .get("customType")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let details = msg.get("details");
        let mut changed = false;
        match custom_type {
            "background-task-notification" => {
                if let Some(job) = details.and_then(parse_background_job) {
                    self.upsert_background_job(job);
                    changed = true;
                }
                if self.sync_pie_extension_state_from_session_file() {
                    changed = true;
                }
            }
            "pie.background-list" => {
                if let Some(raw) = msg.get("content").and_then(|v| v.as_str()) {
                    if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(raw) {
                        for item in arr {
                            if let Some(job) = parse_background_job(&item) {
                                self.upsert_background_job(job);
                                changed = true;
                            }
                        }
                    }
                }
            }
            "pie.subagent-completion" | "pie.subagent-result" | "pie.subagent-stalled" => {
                if let Some(sub) = details.and_then(parse_subagent_record) {
                    self.upsert_subagent(sub);
                    changed = true;
                }
                if self.sync_pie_extension_state_from_session_file() {
                    changed = true;
                }
            }
            "pie.goal-status" | "pie.goal-round" => {
                if self.sync_pie_extension_state_from_session_file() {
                    changed = true;
                }
            }
            _ => {}
        }
        if changed {
            self.emit_meta();
        }
    }

    fn handle_pie_tool_result(&mut self, tool_name: &str, result: Option<&Value>) {
        let details = result.and_then(|r| r.get("details"));
        match tool_name {
            "GoalSet" | "GoalGet" | "GoalUpdate" => {
                if let Some(det) = details {
                    if let Some(armed) = det.get("armed").and_then(|v| v.as_bool()) {
                        self.goal_armed = armed;
                    }
                    if let Some(goal_val) = det.get("goal") {
                        if goal_val.is_null() {
                            self.meta.goal = None;
                        } else if let Some(mut goal) = parse_goal_record(goal_val) {
                            goal.armed = self.goal_armed && goal.status == "active";
                            self.meta.goal = Some(Box::new(goal));
                        }
                    }
                }
                self.sync_pie_extension_state_from_session_file();
            }
            "bg_run" | "bg_logs" => {
                if let Some(job) = details
                    .and_then(|d| d.get("task").or(Some(d)))
                    .and_then(parse_background_job)
                {
                    self.upsert_background_job(job);
                }
                if tool_name == "bg_run" {
                    self.sync_pie_extension_state_from_session_file();
                }
            }
            "bg_status" | "bg_kill" => {
                if let Some(det) = details {
                    if let Some(arr) = det.as_array() {
                        for item in arr {
                            if let Some(job) = parse_background_job(item) {
                                self.upsert_background_job(job);
                            }
                        }
                    } else if let Some(job) = parse_background_job(det) {
                        self.upsert_background_job(job);
                    }
                }
            }
            "Agent" | "get_subagent_result" | "steer_subagent" | "stop_subagent"
            | "TaskOutput" | "TaskStop" => {
                if let Some(sub) = details.and_then(parse_subagent_record) {
                    self.upsert_subagent(sub);
                }
                self.sync_pie_extension_state_from_session_file();
            }
            "list_subagents" => {
                if let Some(text) = result.and_then(extract_text_blocks) {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                        if let Some(runs) = parsed.get("runs").and_then(|v| v.as_array()) {
                            for item in runs {
                                if let Some(sub) = parse_subagent_record(item) {
                                    self.upsert_subagent(sub);
                                }
                            }
                        }
                    }
                }
                self.sync_pie_extension_state_from_session_file();
            }
            "TaskCreate" | "TaskUpdate" | "TaskList" | "TaskExecute" => {
                self.sync_pie_extension_state_from_session_file();
            }
            _ => {}
        }
    }

    fn upsert_background_job(&mut self, job: BackgroundJob) {
        if let Some(existing) = self
            .meta
            .background_jobs
            .iter_mut()
            .find(|item| item.id == job.id)
        {
            *existing = job;
        } else {
            self.meta.background_jobs.push(job);
        }
        sort_background_jobs(&mut self.meta.background_jobs);
    }

    fn upsert_subagent(&mut self, sub: SessionSubagent) {
        if let Some(existing) = self
            .meta
            .subagents
            .iter_mut()
            .find(|item| item.id == sub.id)
        {
            *existing = sub;
        } else {
            self.meta.subagents.push(sub);
        }
        sort_subagents(&mut self.meta.subagents);
    }

    fn sync_pie_extension_state_from_session_file(&mut self) -> bool {
        let Some(path) = self.meta.provider_session_file.as_deref() else {
            return false;
        };
        let Ok(raw) = std::fs::read_to_string(path) else {
            return false;
        };
        let mut saw_goal = false;
        let mut latest_goal: Option<SessionGoal> = None;
        let mut saw_tasks = false;
        let mut latest_tasks: Vec<SessionTask> = Vec::new();
        let mut saw_subagents = false;
        let mut subagent_map: HashMap<String, SessionSubagent> = HashMap::new();

        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || !trimmed.contains("\"custom\"") {
                continue;
            }
            let Ok(entry) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };
            if entry.get("type").and_then(|v| v.as_str()) != Some("custom") {
                continue;
            }
            let custom_type = entry
                .get("customType")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let Some(data) = entry.get("data") else {
                continue;
            };
            match custom_type {
                "pie.goal" => {
                    saw_goal = true;
                    if let Some(goal_val) = data.get("goal") {
                        if goal_val.is_null() {
                            latest_goal = None;
                        } else if let Some(mut goal) = parse_goal_record(goal_val) {
                            goal.armed = self.goal_armed && goal.status == "active";
                            latest_goal = Some(goal);
                        }
                    }
                }
                "pie.tasks" => {
                    if let Some(arr) = data.get("tasks").and_then(|v| v.as_array()) {
                        saw_tasks = true;
                        latest_tasks = arr.iter().filter_map(parse_task_record).collect();
                        sort_tasks(&mut latest_tasks);
                    }
                }
                "pie.subagent" => {
                    if let Some(sub) = parse_subagent_record(data) {
                        saw_subagents = true;
                        subagent_map.insert(sub.id.clone(), sub);
                    }
                }
                "pie.subagent-removed" => {
                    if let Some(id) = data.as_str() {
                        saw_subagents = true;
                        subagent_map.remove(id);
                    }
                }
                _ => {}
            }
        }

        let mut changed = false;
        if saw_goal {
            let boxed = latest_goal.map(Box::new);
            if self.meta.goal != boxed {
                self.meta.goal = boxed;
                changed = true;
            }
        }
        if saw_tasks && self.meta.tasks != latest_tasks {
            self.meta.tasks = latest_tasks;
            changed = true;
        }
        if saw_subagents {
            for existing in &self.meta.subagents {
                if let Some(disk_entry) = subagent_map.get_mut(&existing.id) {
                    let existing_terminal = !matches!(existing.status.as_str(), "queued" | "running");
                    let disk_terminal = !matches!(disk_entry.status.as_str(), "queued" | "running");
                    if (existing_terminal && !disk_terminal)
                        || (existing.finished_at_ms.unwrap_or(0)
                            > disk_entry.finished_at_ms.unwrap_or(0))
                        || (existing_terminal == disk_terminal && existing.turns > disk_entry.turns)
                    {
                        *disk_entry = existing.clone();
                    }
                }
            }
            let mut list: Vec<SessionSubagent> = subagent_map.into_values().collect();
            sort_subagents(&mut list);
            if self.meta.subagents != list {
                self.meta.subagents = list;
                changed = true;
            }
        }
        changed
    }

    fn ensure_thinking_block(&mut self, idx: usize, msg_prefix: &str) -> String {
        if let Some(id) = self.active_blocks.get(&(idx, true)) {
            return id.clone();
        }
        let id = format!("{msg_prefix}-think-{idx}");
        self.active_blocks.insert((idx, true), id.clone());
        self.thinking_started_at.insert(id.clone(), Instant::now());
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                entry: TimelineEntry::Thinking {
                    id: id.clone(),
                    turn_id: self.active_turn_id.clone(),
                    text: String::new(),
                    streaming: true,
                    duration_ms: None,
                    created_at_ms: now_ms(),
                },
            }));
        id
    }

    fn ensure_text_block(&mut self, idx: usize, msg_prefix: &str) -> String {
        if let Some(id) = self.active_blocks.get(&(idx, false)) {
            self.last_text_entry_id = Some(id.clone());
            return id.clone();
        }
        let id = format!("{msg_prefix}-text-{idx}");
        self.active_blocks.insert((idx, false), id.clone());
        self.last_text_entry_id = Some(id.clone());
        let model = self.meta.current_model.as_ref().map(|m| m.name.clone());
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                entry: TimelineEntry::AssistantText {
                    id: id.clone(),
                    turn_id: self.active_turn_id.clone(),
                    text: String::new(),
                    streaming: true,
                    model,
                    usage: None,
                    created_at_ms: now_ms(),
                },
            }));
        id
    }

    fn emit_meta(&self) {
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::MetaUpdated {
                meta: self.meta.clone(),
            }));
    }

    pub fn emit_notice(&self, level: NoticeLevel, text: String) {
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                entry: TimelineEntry::Notice {
                    id: format!("notice-{}", ulid::Ulid::new()),
                    turn_id: self.active_turn_id.clone(),
                    level,
                    text,
                    created_at_ms: now_ms(),
                },
            }));
    }
}

fn parse_pi_model(value: &Value) -> Option<ChatModelInfo> {
    let id = value.get("id")?.as_str()?.to_string();
    let provider = value
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| id.clone());
    Some(ChatModelInfo { provider, id, name })
}

fn parse_pi_command(value: &Value) -> Option<ChatSlashCommand> {
    let raw_name = value.get("name")?.as_str()?.trim();
    if raw_name.is_empty() {
        return None;
    }
    let name = raw_name.trim_start_matches('/').to_string();
    let description = value
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let source = value
        .get("source")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let kind = if source.as_deref() == Some("skill") || name.starts_with("skill:") {
        "skill".to_string()
    } else if source.as_deref() == Some("prompt") {
        "prompt".to_string()
    } else {
        "command".to_string()
    };
    let argument_hint = value
        .get("argumentHint")
        .or_else(|| value.get("argument_hint"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some(ChatSlashCommand {
        name,
        description,
        kind,
        source,
        argument_hint,
    })
}

fn parse_goal_record(value: &Value) -> Option<SessionGoal> {
    let objective = value.get("objective")?.as_str()?.trim().to_string();
    if objective.is_empty() {
        return None;
    }
    let status = value
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("active")
        .to_string();
    let rounds = value.get("rounds").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let max_rounds = value
        .get("maxRounds")
        .or_else(|| value.get("max_rounds"))
        .and_then(|v| v.as_u64())
        .unwrap_or(64) as u32;
    let armed = value
        .get("armed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Some(SessionGoal {
        objective,
        status,
        rounds,
        max_rounds,
        armed,
    })
}

fn parse_task_record(value: &Value) -> Option<SessionTask> {
    let id = value.get("id")?.as_str()?.to_string();
    let subject = value.get("subject")?.as_str()?.trim().to_string();
    if id.is_empty() || subject.is_empty() {
        return None;
    }
    let description = value
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let status = value
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("pending")
        .to_string();
    let active_form = value
        .get("activeForm")
        .or_else(|| value.get("active_form"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let agent_type = value
        .get("agentType")
        .or_else(|| value.get("agent_type"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let owner = value
        .get("owner")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let blocked_by = value
        .get("blockedBy")
        .or_else(|| value.get("blocked_by"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some(SessionTask {
        id,
        subject,
        description,
        status,
        active_form,
        agent_type,
        owner,
        blocked_by,
    })
}

fn parse_subagent_record(value: &Value) -> Option<SessionSubagent> {
    let id = value
        .get("id")
        .or_else(|| value.get("agentId"))
        .and_then(|v| v.as_str())?
        .to_string();
    if id.is_empty() {
        return None;
    }
    let agent = value
        .get("agent")
        .or_else(|| value.get("subagent_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("worker")
        .to_string();
    let description = value
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| agent.clone());
    let status = value
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("running")
        .to_string();
    let model = value
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let turns = value.get("turns").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let max_turns = value
        .get("maxTurns")
        .or_else(|| value.get("max_turns"))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    let active_tool = value
        .get("activeTool")
        .or_else(|| value.get("active_tool"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let output = value
        .get("output")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.len() > 2000 {
                let mut end = 2000;
                while end > 0 && !s.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}…", &s[..end])
            } else {
                s.to_string()
            }
        });
    let error = value
        .get("error")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let task_id = value
        .get("taskId")
        .or_else(|| value.get("task_id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let started_at_ms = value
        .get("startedAt")
        .or_else(|| value.get("started_at_ms"))
        .and_then(|v| v.as_u64());
    let finished_at_ms = value
        .get("finishedAt")
        .or_else(|| value.get("finished_at_ms"))
        .and_then(|v| v.as_u64());
    Some(SessionSubagent {
        id,
        agent,
        description,
        status,
        model,
        turns,
        max_turns,
        active_tool,
        output,
        error,
        task_id,
        started_at_ms,
        finished_at_ms,
    })
}

fn parse_background_job(value: &Value) -> Option<BackgroundJob> {
    let id = value
        .get("id")
        .or_else(|| value.get("taskId"))
        .and_then(|v| v.as_str())?
        .to_string();
    if id.is_empty() {
        return None;
    }
    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| id.clone());
    let description = value
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| name.clone());
    let command = value
        .get("command")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let status = value
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("running")
        .to_string();
    let exit_code = value
        .get("exitCode")
        .or_else(|| value.get("exit_code"))
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let output_path = value
        .get("outputPath")
        .or_else(|| value.get("output_path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let bytes_written = value
        .get("bytesWritten")
        .or_else(|| value.get("bytes_written"))
        .and_then(|v| v.as_u64());
    let error = value
        .get("error")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let started_at_ms = value
        .get("startedAt")
        .or_else(|| value.get("started_at_ms"))
        .and_then(|v| v.as_u64());
    let finished_at_ms = value
        .get("finishedAt")
        .or_else(|| value.get("finished_at_ms"))
        .and_then(|v| v.as_u64());
    Some(BackgroundJob {
        id,
        name,
        description,
        command,
        status,
        exit_code,
        output_path,
        bytes_written,
        error,
        started_at_ms,
        finished_at_ms,
    })
}

fn sort_tasks(tasks: &mut Vec<SessionTask>) {
    let mut in_progress = Vec::new();
    let mut pending = Vec::new();
    let mut completed = Vec::new();
    for task in tasks.drain(..) {
        match task.status.as_str() {
            "in_progress" => in_progress.push(task),
            "completed" => completed.push(task),
            _ => pending.push(task),
        }
    }
    completed.reverse();
    tasks.extend(in_progress);
    tasks.extend(pending);
    tasks.extend(completed);
}

fn sort_subagents(subagents: &mut [SessionSubagent]) {
    subagents.sort_by(|a, b| {
        let a_active = matches!(a.status.as_str(), "running" | "queued");
        let b_active = matches!(b.status.as_str(), "running" | "queued");
        b_active
            .cmp(&a_active)
            .then_with(|| b.started_at_ms.unwrap_or(0).cmp(&a.started_at_ms.unwrap_or(0)))
    });
}

fn sort_background_jobs(jobs: &mut [BackgroundJob]) {
    jobs.sort_by(|a, b| {
        let a_active = a.status == "running";
        let b_active = b.status == "running";
        b_active
            .cmp(&a_active)
            .then_with(|| b.started_at_ms.unwrap_or(0).cmp(&a.started_at_ms.unwrap_or(0)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxlane_core::chat::ChatTimelineState;

    #[test]
    fn pi_rpc_translator_handles_streaming_tools_and_extension_ui() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut tr = PiRpcTranslator::new("pi-1", ChatSessionMeta::default(), tx);

        tr.record_outgoing_prompt("Fix bug", &[]);
        tr.handle_record(json!({"type":"agent_start"}));
        tr.handle_record(json!({"type":"turn_start"}));
        tr.handle_record(
            json!({"type":"message_start","message":{"role":"user","content":"Fix bug"}}),
        );
        tr.handle_record(
            json!({"type":"message_start","message":{"role":"assistant","content":[]}}),
        );
        tr.handle_record(json!({
            "type":"message_update",
            "usage":{"input":100,"cacheRead":20,"cost":{"total":0.02}},
            "assistantMessageEvent":{"type":"thinking_delta","contentIndex":0,"delta":"Let me check"}
        }));
        tr.handle_record(json!({
            "type":"message_update",
            "assistantMessageEvent":{"type":"thinking_end","contentIndex":0,"content":"Let me check main.rs"}
        }));
        tr.handle_record(json!({
            "type":"tool_execution_start",
            "toolCallId":"call-1",
            "toolName":"edit",
            "args":{"path":"src/main.rs","oldText":"old","newText":"new"}
        }));
        tr.handle_record(json!({
            "type":"extension_ui_request",
            "id":"ui-1",
            "method":"confirm",
            "title":"Apply patch?",
            "message":"Modify src/main.rs"
        }));
        tr.resolve_approval("ui-1");
        tr.handle_record(json!({
            "type":"tool_execution_end",
            "toolCallId":"call-1",
            "toolName":"edit",
            "args":{"path":"src/main.rs","oldText":"old","newText":"new"},
            "result":{"content":[{"type":"text","text":"Edited src/main.rs"}]},
            "isError":false,
            "durationMs":12
        }));
        tr.handle_record(json!({
            "type":"message_update",
            "assistantMessageEvent":{"type":"text_delta","contentIndex":1,"delta":"Done!"}
        }));
        tr.handle_record(json!({"type":"agent_settled","aborted":false}));

        let mut state = ChatTimelineState::default();
        let mut statuses = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            match ev {
                DriverEvent::Delta(op) => state.apply_op(op),
                DriverEvent::Status { status, .. } => statuses.push(status),
                _ => {}
            }
        }

        assert!(statuses.contains(&AgentStatus::Blocked));
        assert_eq!(statuses.last(), Some(&AgentStatus::Done));
        assert!(state.pending_approvals.is_empty());
        // Duplicate user echo from Pi was ignored cleanly
        assert_eq!(
            state
                .entries
                .iter()
                .filter(|e| matches!(e, TimelineEntry::UserMessage { .. }))
                .count(),
            1
        );
        assert_eq!(state.entries.len(), 4);
    }

    #[test]
    fn pi_rpc_preserves_tool_args_and_orders_queued_followups_chronologically() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut tr = PiRpcTranslator::new("pi-2", ChatSessionMeta::default(), tx);

        // Turn 1 starts
        tr.record_outgoing_prompt("Inspect project", &[]);
        tr.handle_record(json!({"type":"agent_start"}));
        tr.handle_record(json!({"type":"turn_start"}));
        tr.handle_record(
            json!({"type":"message_start","message":{"role":"user","content":"Inspect project"}}),
        );
        tr.handle_record(
            json!({"type":"message_start","message":{"role":"assistant","content":[]}}),
        );

        // User sends a follow-up while Turn 1 is still streaming
        tr.enqueue_outgoing_prompt("Say hello next", &[], StreamingBehavior::FollowUp);
        assert_eq!(tr.meta.queued_messages.len(), 1);

        // Tool starts with args, ends WITHOUT args (real Pi RPC behavior)
        tr.handle_record(json!({
            "type":"tool_execution_start",
            "toolCallId":"call-bash-1",
            "toolName":"bash",
            "args":{"command":"cargo test --workspace"}
        }));
        tr.handle_record(json!({
            "type":"tool_execution_end",
            "toolCallId":"call-bash-1",
            "toolName":"bash",
            "result":{"content":[{"type":"text","text":"ok"}],"details":{"exitCode":0}},
            "isError":false
        }));
        tr.handle_record(json!({
            "type":"message_update",
            "assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Project inspected."}
        }));
        tr.handle_record(json!({"type":"message_end","message":{"role":"assistant","content":[]}}));

        // Now Pi dequeues the queued follow-up message for Turn 2
        tr.handle_record(json!({"type":"turn_start"}));
        tr.handle_record(
            json!({"type":"message_start","message":{"role":"user","content":"Say hello next"}}),
        );
        tr.handle_record(
            json!({"type":"message_start","message":{"role":"assistant","content":[]}}),
        );
        tr.handle_record(json!({
            "type":"message_update",
            "assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Hello!"}
        }));
        tr.handle_record(json!({"type":"agent_settled","aborted":false}));

        let mut state = ChatTimelineState::default();
        while let Ok(ev) = rx.try_recv() {
            if let DriverEvent::Delta(op) = ev {
                state.apply_op(op);
            }
        }

        // Queue is drained
        assert!(state.meta.queued_messages.is_empty());

        // Tool call preserved its bash command even though tool_execution_end omitted `args`
        let bash_tool = state.entries.iter().find_map(|e| match e {
            TimelineEntry::ToolCall { name, input, .. } if name == "bash" => Some(input),
            _ => None,
        });
        assert_eq!(
            bash_tool.and_then(|v| v.get("command")).and_then(|v| v.as_str()),
            Some("cargo test --workspace")
        );

        // Chronological order: User1 -> Tool -> Assistant1 -> User2 -> Assistant2
        let kinds: Vec<&str> = state
            .entries
            .iter()
            .map(|e| match e {
                TimelineEntry::UserMessage { .. } => "user",
                TimelineEntry::ToolCall { .. } => "tool",
                TimelineEntry::AssistantText { .. } => "assistant",
                TimelineEntry::Thinking { .. } => "thinking",
                TimelineEntry::Plan { .. } => "plan",
                TimelineEntry::Notice { .. } => "notice",
            })
            .collect();
        assert_eq!(kinds, vec!["user", "tool", "assistant", "user", "assistant"]);
    }

    #[test]
    fn pi_rpc_tracks_usage_stats_and_pie_goal_tasks_subagents_bg() {
        let dir = tempfile::tempdir().unwrap();
        let session_file = dir.path().join("pie-session.jsonl");
        std::fs::write(
            &session_file,
            [
                json!({
                    "type": "custom",
                    "customType": "pie.goal",
                    "data": {
                        "goal": {
                            "objective": "Ship dark mode cleanly",
                            "status": "active",
                            "rounds": 2,
                            "maxRounds": 16,
                            "createdAt": 1000,
                            "updatedAt": 2000
                        }
                    }
                })
                .to_string(),
                json!({
                    "type": "custom",
                    "customType": "pie.tasks",
                    "data": {
                        "nextId": 3,
                        "tasks": [
                            {
                                "id": "1",
                                "subject": "Audit CSS tokens",
                                "description": "Check all theme variables",
                                "status": "completed",
                                "blockedBy": []
                            },
                            {
                                "id": "2",
                                "subject": "Implement toggle",
                                "description": "",
                                "status": "in_progress",
                                "agentType": "worker",
                                "owner": "agent:sub-1",
                                "blockedBy": []
                            }
                        ]
                    }
                })
                .to_string(),
                json!({
                    "type": "custom",
                    "customType": "pie.subagent",
                    "data": {
                        "id": "sub-1",
                        "agent": "worker",
                        "description": "Implement toggle",
                        "prompt": "do it",
                        "status": "running",
                        "model": "deepseek-chat",
                        "thinking": "medium",
                        "turns": 3,
                        "maxTurns": 30,
                        "activeTool": "edit",
                        "startedAt": 1500,
                        "taskId": "2",
                        "output": ""
                    }
                })
                .to_string(),
            ]
            .join("\n")
                + "\n",
        )
        .unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut tr = PiRpcTranslator::new("pie-1", ChatSessionMeta::default(), tx);

        // Initial get_state hydrates model context window + Pie JSONL goal/tasks/subagents
        tr.handle_record(json!({
            "type": "response",
            "command": "get_state",
            "success": true,
            "data": {
                "model": { "provider": "deepseek", "id": "deepseek-chat", "name": "DeepSeek Chat", "contextWindow": 128000 },
                "sessionFile": session_file.to_string_lossy(),
                "sessionId": "sess-pie-1",
                "isStreaming": false
            }
        }));

        // Assistant turn with bg_run + usage
        tr.record_outgoing_prompt("Run build in background", &[]);
        tr.handle_record(json!({"type":"agent_start"}));
        tr.handle_record(json!({"type":"turn_start"}));
        tr.handle_record(json!({"type":"message_start","message":{"role":"user","content":"Run build in background"}}));
        tr.handle_record(json!({"type":"message_start","message":{"role":"assistant","content":[]}}));
        tr.handle_record(json!({
            "type":"tool_execution_start",
            "toolCallId":"bg-call-1",
            "toolName":"bg_run",
            "args":{"name":"cargo-build","command":"cargo build --release","timeoutSeconds":600}
        }));
        tr.handle_record(json!({
            "type":"tool_execution_end",
            "toolCallId":"bg-call-1",
            "toolName":"bg_run",
            "isError":false,
            "durationMs":45,
            "result":{
                "content":[{"type":"text","text":"started"}],
                "details":{
                    "task":{
                        "id":"bg-1",
                        "name":"cargo-build",
                        "description":"Release build",
                        "command":"cargo build --release",
                        "status":"running",
                        "startedAt":5000,
                        "outputPath":"/tmp/bg-1.log",
                        "bytesWritten":128
                    }
                }
            }
        }));
        tr.handle_record(json!({
            "type":"message_update",
            "assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Started background build."}
        }));
        tr.handle_record(json!({
            "type":"message_end",
            "message":{
                "role":"assistant",
                "content":[{"type":"text","text":"Started background build."}],
                "usage":{
                    "input":1200,
                    "output":300,
                    "cacheRead":800,
                    "cacheWrite":100,
                    "totalTokens":2400,
                    "cost":{"total":0.012}
                }
            }
        }));
        // Background job completes via custom message
        tr.handle_record(json!({
            "type":"message_start",
            "message":{
                "role":"custom",
                "customType":"background-task-notification",
                "details":{
                    "id":"bg-1",
                    "name":"cargo-build",
                    "description":"Release build",
                    "command":"cargo build --release",
                    "status":"completed",
                    "exitCode":0,
                    "startedAt":5000,
                    "finishedAt":9000,
                    "outputPath":"/tmp/bg-1.log",
                    "bytesWritten":4096
                }
            }
        }));
        // Subagent completes via custom message
        tr.handle_record(json!({
            "type":"message_start",
            "message":{
                "role":"custom",
                "customType":"pie.subagent-completion",
                "details":{
                    "id":"sub-1",
                    "agent":"worker",
                    "description":"Implement toggle",
                    "status":"completed",
                    "turns":4,
                    "maxTurns":30,
                    "startedAt":1500,
                    "finishedAt":6500,
                    "taskId":"2",
                    "output":"Toggle implemented and verified."
                }
            }
        }));
        tr.handle_record(json!({"type":"agent_settled","aborted":false}));

        let mut state = ChatTimelineState::default();
        while let Ok(ev) = rx.try_recv() {
            if let DriverEvent::Delta(op) = ev {
                state.apply_op(op);
            }
        }

        // Verify per-message usage on AssistantText
        let assistant_usage = state.entries.iter().find_map(|e| match e {
            TimelineEntry::AssistantText { usage, .. } => usage.as_deref(),
            _ => None,
        });
        let usage = assistant_usage.expect("assistant message should carry usage");
        assert_eq!(usage.input_tokens, 1200);
        assert_eq!(usage.output_tokens, 300);
        assert_eq!(usage.cache_read_tokens, 800);
        assert_eq!(usage.cache_write_tokens, 100);
        assert_eq!(usage.total_tokens, 2400);
        assert!(usage.duration_ms.is_some());

        // Verify session stats
        let stats = state.meta.stats.as_deref().expect("session stats populated");
        assert_eq!(stats.turns, 1);
        assert_eq!(stats.steps, 1);
        assert_eq!(stats.tool_calls, 1);
        assert_eq!(stats.tool_ms, 45);
        assert_eq!(stats.total_tokens, 2400);
        assert_eq!(state.meta.context_window_tokens, Some(128000));
        assert_eq!(state.meta.context_used_tokens, Some(2100));

        // Verify Pie goal, tasks, subagents, background_jobs
        let goal = state.meta.goal.as_deref().expect("goal hydrated");
        assert_eq!(goal.objective, "Ship dark mode cleanly");
        assert_eq!(goal.rounds, 2);
        assert_eq!(goal.max_rounds, 16);

        assert_eq!(state.meta.tasks.len(), 2);
        assert_eq!(state.meta.tasks[0].status, "in_progress");
        assert_eq!(state.meta.tasks[1].status, "completed");

        assert_eq!(state.meta.subagents.len(), 1);
        assert_eq!(state.meta.subagents[0].id, "sub-1");
        assert_eq!(state.meta.subagents[0].status, "completed");

        assert_eq!(state.meta.background_jobs.len(), 1);
        assert_eq!(state.meta.background_jobs[0].id, "bg-1");
        assert_eq!(state.meta.background_jobs[0].status, "completed");
        assert_eq!(state.meta.background_jobs[0].exit_code, Some(0));
    }

    #[test]
    fn pi_rpc_removes_single_and_all_queued_steer_messages() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut tr = PiRpcTranslator::new("pie-q", ChatSessionMeta::default(), tx);
        tr.enqueue_outgoing_prompt("First steer", &[], StreamingBehavior::Steer);
        tr.enqueue_outgoing_prompt("Second steer", &[], StreamingBehavior::Steer);
        tr.enqueue_outgoing_prompt("Third steer", &[], StreamingBehavior::Steer);
        assert_eq!(tr.meta.queued_messages.len(), 3);
        assert_eq!(tr.meta.queued_messages[0].behavior, StreamingBehavior::Steer);

        let second_id = tr.meta.queued_messages[1].id.clone();
        let remaining = tr.remove_queued_message(Some(&second_id));
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0].text, "First steer");
        assert_eq!(remaining[1].text, "Third steer");
        assert_eq!(tr.meta.queued_messages.len(), 2);

        let cleared = tr.remove_queued_message(None);
        assert!(cleared.is_empty());
        assert!(tr.meta.queued_messages.is_empty());
    }
}
