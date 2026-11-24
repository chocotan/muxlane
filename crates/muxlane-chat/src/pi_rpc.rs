//! Pi / Pie `--mode rpc` 结构化驱动实现（严格基于 Pi RPC v1 LF-JSONL 规范）。
use crate::driver::{
    extract_text_blocks, read_lf_line, ChatDriverHandle, DriverCmd, DriverEvent, DriverLaunchSpec,
};
use crate::transcript::infer_tool_diff;
use anyhow::Context;
use muxlane_core::chat::{
    now_ms, ApprovalOption, ApprovalReply, ApprovalSource, ChatDeltaOp, ChatModelInfo,
    ChatSessionMeta, NoticeLevel, PendingApproval, SessionDriverMode, StreamingBehavior,
    TextDeltaKind, TimelineEntry, ToolCallKind, ToolCallState,
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

    // 启动初始化：查询会话状态、模型列表与思考级别。
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

    let mut req_seq: u64 = 1;
    let mut recent_stderr: VecDeque<String> = VecDeque::with_capacity(20);
    let mut exit_requested = false;

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => match cmd {
                Some(DriverCmd::SendPrompt { text, attachments, streaming_behavior }) => {
                    translator.record_outgoing_prompt(&text, &attachments);
                    let id = format!("cmd-{req_seq}");
                    req_seq += 1;
                    let mut payload = serde_json::Map::new();
                    payload.insert("id".into(), Value::String(id));
                    payload.insert("type".into(), Value::String("prompt".into()));
                    payload.insert("message".into(), Value::String(text));
                    if !attachments.is_empty() {
                        let images: Vec<Value> = attachments
                            .iter()
                            .filter_map(|att| {
                                let data = att.data_b64.as_ref()?;
                                Some(json!({
                                    "type": "image",
                                    "data": data,
                                    "mimeType": att.mime_type,
                                }))
                            })
                            .collect();
                        if !images.is_empty() {
                            payload.insert("images".into(), Value::Array(images));
                        }
                    }
                    if translator.meta.is_streaming {
                        let behavior = match streaming_behavior.unwrap_or(StreamingBehavior::Steer) {
                            StreamingBehavior::Steer => "steer",
                            StreamingBehavior::FollowUp => "followUp",
                        };
                        payload.insert("streamingBehavior".into(), Value::String(behavior.into()));
                    }
                    send_json_line(&mut stdin, Value::Object(payload)).await?;
                }
                Some(DriverCmd::Abort) => {
                    let id = format!("cmd-{req_seq}");
                    req_seq += 1;
                    send_json_line(&mut stdin, json!({"id": id, "type": "abort"})).await?;
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
                }
                Some(DriverCmd::SetThinkingLevel { level }) => {
                    let id = format!("cmd-{req_seq}");
                    req_seq += 1;
                    send_json_line(
                        &mut stdin,
                        json!({"id": id, "type": "set_thinking_level", "level": level}),
                    )
                    .await?;
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

pub(crate) struct PiRpcTranslator {
    agent: String,
    pub meta: ChatSessionMeta,
    events: mpsc::UnboundedSender<DriverEvent>,
    turn_seq: u64,
    msg_seq: u64,
    active_turn_id: Option<String>,
    active_msg_id: Option<String>,
    active_blocks: HashMap<(usize, bool), String>,
    thinking_started_at: HashMap<String, Instant>,
    tool_started_at: HashMap<String, Instant>,
    last_error: Option<String>,
    blocked_pending: bool,
    pending_user_echos: VecDeque<String>,
}

impl PiRpcTranslator {
    pub fn new(
        agent: &str,
        meta: ChatSessionMeta,
        events: mpsc::UnboundedSender<DriverEvent>,
    ) -> Self {
        Self {
            agent: agent.to_string(),
            meta,
            events,
            turn_seq: 0,
            msg_seq: 0,
            active_turn_id: None,
            active_msg_id: None,
            active_blocks: HashMap::new(),
            thinking_started_at: HashMap::new(),
            tool_started_at: HashMap::new(),
            last_error: None,
            blocked_pending: false,
            pending_user_echos: VecDeque::new(),
        }
    }

    pub fn record_outgoing_prompt(
        &mut self,
        text: &str,
        attachments: &[muxlane_core::chat::ChatAttachment],
    ) {
        self.turn_seq += 1;
        let id = format!("user-{}", ulid::Ulid::new());
        let trimmed = text.trim().to_string();
        if !trimmed.is_empty() {
            self.pending_user_echos.push_back(trimmed);
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
        let turn_id = self.active_turn_id.take();
        self.active_msg_id = None;
        self.active_blocks.clear();

        let err = if aborted {
            None
        } else {
            self.last_error.take()
        };
        self.meta.last_error = err.clone();
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
        let Some(data) = record.get("data") else {
            return;
        };
        match cmd {
            "get_state" => {
                if let Some(m) = data.get("model").and_then(parse_pi_model) {
                    self.meta.current_model = Some(m);
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
                    self.emit_meta();
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
            let text = extract_text_blocks(msg).unwrap_or_default();
            if self
                .pending_user_echos
                .front()
                .is_some_and(|pending| pending == text.trim())
            {
                self.pending_user_echos.pop_front();
                return;
            }
            if !text.trim().is_empty() {
                let _ = self
                    .events
                    .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                        entry: TimelineEntry::UserMessage {
                            id: format!("pi-user-{}", ulid::Ulid::new()),
                            turn_id: self.active_turn_id.clone(),
                            text,
                            attachments: Vec::new(),
                            created_at_ms: now_ms(),
                        },
                    }));
            }
            return;
        }
        if role == "assistant" {
            self.msg_seq += 1;
            self.active_msg_id = Some(format!("pi-msg-{}", self.msg_seq));
            self.active_blocks.clear();
        }
    }

    fn handle_message_update(&mut self, record: &Value) {
        if let Some(usage) = record.get("usage") {
            let input = usage.get("input").and_then(|v| v.as_u64()).unwrap_or(0);
            let cache_read = usage.get("cacheRead").and_then(|v| v.as_u64()).unwrap_or(0);
            let total = input.saturating_add(cache_read);
            if total > 0 {
                self.meta.context_used_tokens = Some(total);
            }
            if let Some(cost) = usage.pointer("/cost/total").and_then(|v| v.as_f64()) {
                self.meta.total_cost_usd = Some(cost);
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
        if msg.get("role").and_then(|v| v.as_str()) != Some("assistant") {
            return;
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
        let input = record.get("args").cloned().unwrap_or(Value::Null);
        let diff = infer_tool_diff(&name, &input).map(Box::new);
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
        let args = record.get("args").cloned().unwrap_or(Value::Null);
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
        let diff = infer_tool_diff(&name, &args).map(Box::new);
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
            return id.clone();
        }
        let id = format!("{msg_prefix}-text-{idx}");
        self.active_blocks.insert((idx, false), id.clone());
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
}
