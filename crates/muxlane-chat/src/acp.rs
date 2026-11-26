//! 通用 ACP (Agent Client Protocol) JSON-RPC 2.0 over stdio 驱动。
//! 兼容 `opencode acp`、`qwen --acp`、`gemini --acp` 等一切标准 ACP Agent。
use crate::driver::{read_lf_line, ChatDriverHandle, DriverCmd, DriverEvent, DriverLaunchSpec};
use crate::transcript::infer_tool_diff;
use anyhow::Context;
use muxlane_core::chat::{
    now_ms, ApprovalOption, ApprovalReply, ApprovalSource, ChatAttachment, ChatDeltaOp,
    ChatModelInfo, ChatSessionMeta, ChatSlashCommand, MessageUsage, NoticeLevel, PendingApproval,
    PlanEntry, PlanEntryStatus, QueuedMessage, SessionDriverMode, SessionStats, StreamingBehavior,
    TextDeltaKind, TimelineEntry, ToolCallKind, ToolCallState, ToolDiff,
};
use muxlane_core::model::AgentStatus;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::time::Instant;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

const ACP_PROTOCOL_VERSION: u64 = 1;

pub fn spawn(
    spec: DriverLaunchSpec,
    initial_meta: ChatSessionMeta,
    events: mpsc::UnboundedSender<DriverEvent>,
) -> ChatDriverHandle {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        if let Err(error) = run_acp(spec, initial_meta, cmd_rx, events.clone()).await {
            let _ = events.send(DriverEvent::Exited {
                error: Some(error.to_string()),
            });
        }
    });
    ChatDriverHandle::new(cmd_tx)
}

#[derive(Debug)]
struct PendingPrompt {
    id: String,
    text: String,
    attachments: Vec<ChatAttachment>,
}

fn sync_acp_queue_meta(translator: &mut AcpTranslator, pending: &VecDeque<PendingPrompt>) {
    translator.meta.queued_messages = pending
        .iter()
        .map(|p| QueuedMessage {
            id: p.id.clone(),
            text: p.text.clone(),
            behavior: StreamingBehavior::FollowUp,
            attachments_count: p.attachments.len(),
        })
        .collect();
    translator.emit_meta();
}

async fn run_acp(
    spec: DriverLaunchSpec,
    mut meta: ChatSessionMeta,
    mut cmd_rx: mpsc::UnboundedReceiver<DriverCmd>,
    events: mpsc::UnboundedSender<DriverEvent>,
) -> anyhow::Result<()> {
    meta.driver = SessionDriverMode::Acp;
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

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to launch `{}` in ACP mode", spec.program))?;
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
    let mut translator = AcpTranslator::new(&spec.agent, meta, events.clone());
    let mut rpc_seq: u64 = 1;
    let mut inflight: HashMap<u64, String> = HashMap::new();
    let mut pending_prompts: VecDeque<PendingPrompt> = VecDeque::new();
    let mut recent_stderr: VecDeque<String> = VecDeque::with_capacity(20);
    let mut exit_requested = false;

    // 1. 发送 ACP initialize 握手
    let init_id = rpc_seq;
    rpc_seq += 1;
    inflight.insert(init_id, "initialize".into());
    send_rpc_request(
        &mut stdin,
        init_id,
        "initialize",
        json!({
            "protocolVersion": ACP_PROTOCOL_VERSION,
            "clientCapabilities": {
                "fs": { "readTextFile": false, "writeTextFile": false },
                "terminal": false
            },
            "clientInfo": {
                "name": "muxlane",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
    )
    .await?;

    let mut session_ready = false;
    let mut supports_image = false;

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => match cmd {
                Some(DriverCmd::SendPrompt { text, attachments, .. }) => {
                    let prompt = PendingPrompt {
                        id: format!("queued-{}", ulid::Ulid::new()),
                        text,
                        attachments,
                    };
                    if !session_ready || translator.meta.is_streaming {
                        pending_prompts.push_back(prompt);
                        sync_acp_queue_meta(&mut translator, &pending_prompts);
                    } else {
                        dispatch_acp_prompt(
                            &mut stdin,
                            &mut rpc_seq,
                            &mut inflight,
                            &mut translator,
                            prompt,
                            supports_image,
                        ).await?;
                    }
                }
                Some(DriverCmd::Abort) => {
                    if let Some(sid) = translator.meta.provider_session_id.clone() {
                        send_rpc_notification(
                            &mut stdin,
                            "session/cancel",
                            json!({ "sessionId": sid }),
                        )
                        .await?;
                    }
                    pending_prompts.clear();
                    sync_acp_queue_meta(&mut translator, &pending_prompts);
                }
                Some(DriverCmd::DeleteQueuedMessage { message_id }) => {
                    match message_id {
                        Some(target_id) => {
                            pending_prompts.retain(|p| p.id != target_id);
                        }
                        None => {
                            pending_prompts.clear();
                        }
                    }
                    sync_acp_queue_meta(&mut translator, &pending_prompts);
                }
                Some(DriverCmd::RespondApproval { approval_id, reply }) => {
                    if let Some(rpc_req_id) = translator.take_permission_rpc_id(&approval_id) {
                        let outcome = match reply {
                            ApprovalReply::SelectedOption { option_id } => json!({
                                "outcome": { "outcome": "selected", "optionId": option_id }
                            }),
                            ApprovalReply::Confirm { confirmed: true } => {
                                if let Some(first) = translator.first_allow_option(&approval_id) {
                                    json!({ "outcome": { "outcome": "selected", "optionId": first } })
                                } else {
                                    json!({ "outcome": { "outcome": "cancelled" } })
                                }
                            }
                            ApprovalReply::Confirm { confirmed: false }
                            | ApprovalReply::Cancel
                            | ApprovalReply::Text { .. } => {
                                json!({ "outcome": { "outcome": "cancelled" } })
                            }
                        };
                        translator.resolve_permission_ui(&approval_id);
                        send_rpc_result(&mut stdin, rpc_req_id, outcome).await?;
                    }
                }
                Some(DriverCmd::SetModel { model_id, .. }) => {
                    if let Some(sid) = translator.meta.provider_session_id.clone() {
                        let id = rpc_seq;
                        rpc_seq += 1;
                        inflight.insert(id, format!("session/set_model:{model_id}"));
                        send_rpc_request(
                            &mut stdin,
                            id,
                            "session/set_model",
                            json!({ "sessionId": sid, "modelId": model_id }),
                        )
                        .await?;
                    }
                }
                Some(DriverCmd::SetThinkingLevel { .. }) => {
                    // ACP 当前标准模型通过 session/set_config_option 暴露，无统一独立思考等级方法。
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
                    let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
                        tracing::debug!(line = %trimmed, "ignoring non-json stdout line from ACP");
                        continue;
                    };

                    // 区分：1) Agent 发起的 Request（含 id + method）2) Notification（仅含 method）3) Response（含 id 无 method）
                    let has_method = msg.get("method").and_then(|v| v.as_str()).is_some();
                    let has_id = msg.get("id").is_some() && !msg.get("id").unwrap().is_null();

                    if has_method && has_id {
                        let rpc_req_id = msg.get("id").cloned().unwrap_or(Value::Null);
                        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
                        let params = msg.get("params").cloned().unwrap_or(Value::Null);
                        match method {
                            "session/request_permission" => {
                                if !translator.handle_permission_request(rpc_req_id.clone(), &params) {
                                    send_rpc_result(
                                        &mut stdin,
                                        rpc_req_id,
                                        json!({ "outcome": { "outcome": "cancelled" } }),
                                    )
                                    .await?;
                                }
                            }
                            _ => {
                                // 不支持的可选 client capability 返回 method not found
                                send_rpc_error(
                                    &mut stdin,
                                    rpc_req_id,
                                    -32601,
                                    &format!("unsupported client method: {method}"),
                                )
                                .await?;
                            }
                        }
                    } else if has_method {
                        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
                        if method == "session/update" {
                            if let Some(params) = msg.get("params") {
                                translator.handle_session_update(params);
                            }
                        }
                    } else if has_id {
                        let id = msg.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                        let kind = inflight.remove(&id).unwrap_or_default();
                        if let Some(err) = msg.get("error") {
                            let message = err
                                .get("message")
                                .and_then(|v| v.as_str())
                                .unwrap_or("ACP error")
                                .to_string();
                            let _ = events.send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                                entry: TimelineEntry::Notice {
                                    id: format!("acp-err-{}", ulid::Ulid::new()),
                                    turn_id: translator.active_turn_id.clone(),
                                    level: NoticeLevel::Error,
                                    text: format!("{kind}: {message}"),
                                    created_at_ms: now_ms(),
                                },
                            }));
                            if kind == "session/prompt" {
                                translator.finish_turn(Some(message));
                            }
                        } else if let Some(result) = msg.get("result") {
                            match kind.as_str() {
                                "initialize" => {
                                    let caps = result.get("agentCapabilities");
                                    supports_image = caps
                                        .and_then(|c| c.pointer("/promptCapabilities/image"))
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false);
                                    let load_session_cap = caps
                                        .and_then(|c| c.get("loadSession"))
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false);
                                    let resume_session_cap = caps
                                        .and_then(|c| c.pointer("/sessionCapabilities/resume"))
                                        .is_some_and(|v| !v.is_null());

                                    let cwd_str = spec.cwd.display().to_string();
                                    let next_id = rpc_seq;
                                    rpc_seq += 1;
                                    if let Some(existing_sid) = spec.provider_session.as_ref().filter(|s| !s.is_empty()) {
                                        if load_session_cap {
                                            inflight.insert(next_id, "session/load".into());
                                            send_rpc_request(
                                                &mut stdin,
                                                next_id,
                                                "session/load",
                                                json!({ "sessionId": existing_sid, "cwd": cwd_str, "mcpServers": [] }),
                                            ).await?;
                                        } else if resume_session_cap {
                                            inflight.insert(next_id, "session/resume".into());
                                            send_rpc_request(
                                                &mut stdin,
                                                next_id,
                                                "session/resume",
                                                json!({ "sessionId": existing_sid, "cwd": cwd_str, "mcpServers": [] }),
                                            ).await?;
                                        } else {
                                            inflight.insert(next_id, "session/new".into());
                                            send_rpc_request(
                                                &mut stdin,
                                                next_id,
                                                "session/new",
                                                json!({ "cwd": cwd_str, "mcpServers": [] }),
                                            ).await?;
                                        }
                                    } else {
                                        inflight.insert(next_id, "session/new".into());
                                        send_rpc_request(
                                            &mut stdin,
                                            next_id,
                                            "session/new",
                                            json!({ "cwd": cwd_str, "mcpServers": [] }),
                                        ).await?;
                                    }
                                }
                                "session/new" | "session/load" | "session/resume" => {
                                    if let Some(sid) = result.get("sessionId").and_then(|v| v.as_str()) {
                                        translator.meta.provider_session_id = Some(sid.to_string());
                                    } else if let Some(sid) = &spec.provider_session {
                                        translator.meta.provider_session_id = Some(sid.clone());
                                    }
                                    translator.update_models_from_session_result(result);
                                    session_ready = true;
                                    if let Some(model) = &spec.model {
                                        if let Some(sid) = translator.meta.provider_session_id.clone() {
                                            let mid = rpc_seq;
                                            rpc_seq += 1;
                                            inflight.insert(mid, format!("session/set_model:{model}"));
                                            send_rpc_request(
                                                &mut stdin,
                                                mid,
                                                "session/set_model",
                                                json!({ "sessionId": sid, "modelId": model }),
                                            ).await?;
                                        }
                                    }
                                    if let Some(next_prompt) = pending_prompts.pop_front() {
                                        sync_acp_queue_meta(&mut translator, &pending_prompts);
                                        dispatch_acp_prompt(
                                            &mut stdin,
                                            &mut rpc_seq,
                                            &mut inflight,
                                            &mut translator,
                                            next_prompt,
                                            supports_image,
                                        ).await?;
                                    }
                                }
                                "session/prompt" => {
                                    let stop_reason = result
                                        .get("stopReason")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("end_turn");
                                    if stop_reason == "cancelled" {
                                        translator.finish_turn_cancelled();
                                    } else {
                                        translator.finish_turn(None);
                                    }
                                    if let Some(next_prompt) = pending_prompts.pop_front() {
                                        sync_acp_queue_meta(&mut translator, &pending_prompts);
                                        dispatch_acp_prompt(
                                            &mut stdin,
                                            &mut rpc_seq,
                                            &mut inflight,
                                            &mut translator,
                                            next_prompt,
                                            supports_image,
                                        ).await?;
                                    }
                                }
                                _ if kind.starts_with("session/set_model:") => {
                                    let model_id = kind.trim_start_matches("session/set_model:");
                                    if let Some(found) = translator
                                        .meta
                                        .available_models
                                        .iter()
                                        .find(|m| m.id == model_id)
                                        .cloned()
                                    {
                                        translator.meta.current_model = Some(found);
                                    } else {
                                        translator.meta.current_model = Some(ChatModelInfo {
                                            provider: String::new(),
                                            id: model_id.to_string(),
                                            name: model_id.to_string(),
                                        });
                                    }
                                    translator.emit_meta();
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = events.send(DriverEvent::Exited {
                        error: Some(format!("ACP stdout read error: {error}")),
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
            format!("ACP agent exited with status {status}")
        } else {
            tail
        })
    };
    let _ = events.send(DriverEvent::Exited { error: exit_err });
    Ok(())
}

async fn dispatch_acp_prompt(
    stdin: &mut tokio::process::ChildStdin,
    rpc_seq: &mut u64,
    inflight: &mut HashMap<u64, String>,
    translator: &mut AcpTranslator,
    prompt: PendingPrompt,
    supports_image: bool,
) -> anyhow::Result<()> {
    let Some(sid) = translator.meta.provider_session_id.clone() else {
        return Ok(());
    };
    translator.begin_turn(&prompt.text, &prompt.attachments);
    let mut blocks = vec![json!({ "type": "text", "text": prompt.text })];
    if supports_image {
        for att in &prompt.attachments {
            if let Some(data) = &att.data_b64 {
                blocks.push(json!({
                    "type": "image",
                    "data": data,
                    "mimeType": att.mime_type,
                }));
            }
        }
    }
    let id = *rpc_seq;
    *rpc_seq += 1;
    inflight.insert(id, "session/prompt".into());
    send_rpc_request(
        stdin,
        id,
        "session/prompt",
        json!({
            "sessionId": sid,
            "prompt": blocks,
        }),
    )
    .await
}

async fn send_rpc_request(
    stdin: &mut tokio::process::ChildStdin,
    id: u64,
    method: &str,
    params: Value,
) -> anyhow::Result<()> {
    write_json_line(
        stdin,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }),
    )
    .await
}

async fn send_rpc_notification(
    stdin: &mut tokio::process::ChildStdin,
    method: &str,
    params: Value,
) -> anyhow::Result<()> {
    write_json_line(
        stdin,
        json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }),
    )
    .await
}

async fn send_rpc_result(
    stdin: &mut tokio::process::ChildStdin,
    id: Value,
    result: Value,
) -> anyhow::Result<()> {
    write_json_line(
        stdin,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }),
    )
    .await
}

async fn send_rpc_error(
    stdin: &mut tokio::process::ChildStdin,
    id: Value,
    code: i64,
    message: &str,
) -> anyhow::Result<()> {
    write_json_line(
        stdin,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message },
        }),
    )
    .await
}

async fn write_json_line(
    stdin: &mut tokio::process::ChildStdin,
    value: Value,
) -> anyhow::Result<()> {
    let mut line = serde_json::to_vec(&value)?;
    line.push(b'\n');
    stdin.write_all(&line).await?;
    stdin.flush().await?;
    Ok(())
}

pub(crate) struct AcpTranslator {
    agent: String,
    pub meta: ChatSessionMeta,
    events: mpsc::UnboundedSender<DriverEvent>,
    turn_seq: u64,
    block_seq: u64,
    pub active_turn_id: Option<String>,
    active_think_entry: Option<String>,
    active_text_entry: Option<String>,
    last_text_entry_id: Option<String>,
    turn_started_at: Option<Instant>,
    turn_first_token_ms: Option<u64>,
    turn_tool_ms: u64,
    session_stats: SessionStats,
    thinking_started_at: Option<Instant>,
    tool_started_at: HashMap<String, Instant>,
    tool_cache: HashMap<String, TimelineEntry>,
    pending_permissions: HashMap<String, (Value, Vec<ApprovalOption>)>,
    has_derived_title: bool,
    has_explicit_title: bool,
}

impl AcpTranslator {
    pub fn new(
        agent: &str,
        meta: ChatSessionMeta,
        events: mpsc::UnboundedSender<DriverEvent>,
    ) -> Self {
        let session_stats = meta.stats.as_deref().cloned().unwrap_or_default();
        Self {
            agent: agent.to_string(),
            meta,
            events,
            turn_seq: 0,
            block_seq: 0,
            active_turn_id: None,
            active_think_entry: None,
            active_text_entry: None,
            last_text_entry_id: None,
            turn_started_at: None,
            turn_first_token_ms: None,
            turn_tool_ms: 0,
            session_stats,
            thinking_started_at: None,
            tool_started_at: HashMap::new(),
            tool_cache: HashMap::new(),
            pending_permissions: HashMap::new(),
            has_derived_title: false,
            has_explicit_title: false,
        }
    }

    pub fn begin_turn(&mut self, text: &str, attachments: &[ChatAttachment]) {
        self.turn_seq += 1;
        let turn_id = format!("acp-turn-{}", self.turn_seq);
        self.active_turn_id = Some(turn_id.clone());
        self.active_think_entry = None;
        self.active_text_entry = None;
        self.last_text_entry_id = None;
        self.turn_started_at = Some(Instant::now());
        self.turn_first_token_ms = None;
        self.turn_tool_ms = 0;
        self.session_stats.turns = self.session_stats.turns.saturating_add(1);
        self.meta.stats = Some(Box::new(self.session_stats.clone()));
        self.meta.is_streaming = true;
        self.meta.last_error = None;
        self.emit_meta();
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
                    id: format!("acp-user-{}", ulid::Ulid::new()),
                    turn_id: Some(turn_id),
                    text: text.to_string(),
                    attachments: attachments.to_vec(),
                    created_at_ms: now_ms(),
                },
            }));
        let _ = self.events.send(DriverEvent::Status {
            status: AgentStatus::Working,
            message: None,
            reset_seen: false,
        });
    }

    pub fn finish_turn(&mut self, error: Option<String>) {
        self.close_active_text();
        self.close_active_thinking();
        let total_turn_ms = self
            .turn_started_at
            .take()
            .map(|t| t.elapsed().as_millis() as u64);
        let ttft_ms = self.turn_first_token_ms.take();
        if let Some(total_ms) = total_turn_ms {
            let llm_ms = total_ms.saturating_sub(self.turn_tool_ms);
            self.session_stats.steps = self.session_stats.steps.saturating_add(1);
            self.session_stats.llm_ms = self.session_stats.llm_ms.saturating_add(llm_ms);
            if let Some(ttft) = ttft_ms {
                self.session_stats.ttft_ms = self.session_stats.ttft_ms.saturating_add(ttft);
                self.session_stats.ttft_steps = self.session_stats.ttft_steps.saturating_add(1);
            }
            if let Some(entry_id) = self.last_text_entry_id.take() {
                let _ = self
                    .events
                    .send(DriverEvent::Delta(ChatDeltaOp::SetAssistantUsage {
                        entry_id,
                        usage: Box::new(MessageUsage {
                            duration_ms: Some(total_ms),
                            ttft_ms,
                            ..Default::default()
                        }),
                    }));
            }
        } else {
            self.last_text_entry_id = None;
        }
        self.meta.stats = Some(Box::new(self.session_stats.clone()));
        let turn_id = self.active_turn_id.take();
        self.meta.is_streaming = false;
        self.meta.last_error = error.clone();
        self.emit_meta();
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::TurnFinished {
                turn_id,
                error: error.clone(),
            }));
        if self.pending_permissions.is_empty() {
            let status = if error.is_some() {
                AgentStatus::Failed
            } else {
                AgentStatus::Done
            };
            let _ = self.events.send(DriverEvent::Status {
                status,
                message: error.or_else(|| Some("Task completed".to_string())),
                reset_seen: true,
            });
        }
    }

    pub fn finish_turn_cancelled(&mut self) {
        self.close_active_text();
        self.close_active_thinking();
        self.turn_started_at = None;
        self.turn_first_token_ms = None;
        self.last_text_entry_id = None;
        let turn_id = self.active_turn_id.take();
        self.meta.is_streaming = false;
        self.emit_meta();
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::TurnFinished {
                turn_id,
                error: None,
            }));
        if self.pending_permissions.is_empty() {
            let _ = self.events.send(DriverEvent::Status {
                status: AgentStatus::Idle,
                message: None,
                reset_seen: false,
            });
        }
    }

    pub fn update_models_from_session_result(&mut self, result: &Value) {
        if let Some(models) = result.get("models") {
            let current = models
                .get("currentModelId")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let mut list = Vec::new();
            if let Some(arr) = models.get("availableModels").and_then(|v| v.as_array()) {
                for item in arr {
                    if let Some(id) = item.get("modelId").and_then(|v| v.as_str()) {
                        let name = item
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or(id)
                            .to_string();
                        list.push(ChatModelInfo {
                            provider: String::new(),
                            id: id.to_string(),
                            name,
                        });
                    }
                }
            }
            if let Some(cur_id) = current {
                self.meta.current_model =
                    list.iter()
                        .find(|m| m.id == cur_id)
                        .cloned()
                        .or(Some(ChatModelInfo {
                            provider: String::new(),
                            id: cur_id.clone(),
                            name: cur_id,
                        }));
            }
            if !list.is_empty() {
                self.meta.available_models = list;
            }
        }
        self.emit_meta();
    }

    pub fn handle_permission_request(&mut self, rpc_req_id: Value, params: &Value) -> bool {
        let Some(options_arr) = params.get("options").and_then(|v| v.as_array()) else {
            return false;
        };
        let options: Vec<ApprovalOption> = options_arr
            .iter()
            .filter_map(|opt| {
                let option_id = opt.get("optionId")?.as_str()?.to_string();
                let label = opt
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&option_id)
                    .to_string();
                let kind = opt.get("kind").and_then(|v| v.as_str()).map(str::to_string);
                Some(ApprovalOption {
                    option_id,
                    label,
                    kind,
                })
            })
            .collect();
        if options.is_empty() {
            return false;
        }
        let tool = params.get("toolCall");
        let tool_name = tool
            .and_then(|t| t.get("name").or_else(|| t.get("title")))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let title = tool
            .and_then(|t| t.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("Agent permission request")
            .to_string();
        let command_or_input = tool
            .and_then(|t| t.get("rawInput"))
            .map(|raw| serde_json::to_string_pretty(raw).unwrap_or_else(|_| raw.to_string()));

        let approval_id = format!("acp-perm-{}", ulid::Ulid::new());
        self.pending_permissions
            .insert(approval_id.clone(), (rpc_req_id, options.clone()));

        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::ApprovalRequested {
                approval: PendingApproval {
                    id: approval_id,
                    agent: self.agent.clone(),
                    source: ApprovalSource::AcpPermission,
                    method: "permission".into(),
                    title: title.clone(),
                    message: None,
                    tool_name,
                    command_or_input,
                    options,
                    prefill: None,
                    created_at_ms: now_ms(),
                },
            }));
        let _ = self.events.send(DriverEvent::Status {
            status: AgentStatus::Blocked,
            message: Some(title),
            reset_seen: true,
        });
        true
    }

    pub fn take_permission_rpc_id(&mut self, approval_id: &str) -> Option<Value> {
        self.pending_permissions
            .get(approval_id)
            .map(|(rpc_id, _)| rpc_id.clone())
    }

    pub fn first_allow_option(&self, approval_id: &str) -> Option<String> {
        let (_, options) = self.pending_permissions.get(approval_id)?;
        options
            .iter()
            .find(|opt| opt.kind.as_deref().is_some_and(|k| k.starts_with("allow")))
            .or_else(|| options.first())
            .map(|opt| opt.option_id.clone())
    }

    pub fn resolve_permission_ui(&mut self, approval_id: &str) {
        self.pending_permissions.remove(approval_id);
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::ApprovalResolved {
                approval_id: approval_id.to_string(),
            }));
        if self.pending_permissions.is_empty() && self.meta.is_streaming {
            let _ = self.events.send(DriverEvent::Status {
                status: AgentStatus::Working,
                message: None,
                reset_seen: false,
            });
        }
    }

    pub fn handle_session_update(&mut self, params: &Value) {
        let Some(update) = params.get("update") else {
            return;
        };
        let kind = update
            .get("sessionUpdate")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        match kind {
            "agent_thought_chunk" => {
                if let Some(text) = update.pointer("/content/text").and_then(|v| v.as_str()) {
                    if text.is_empty() {
                        return;
                    }
                    if self.turn_first_token_ms.is_none() {
                        self.turn_first_token_ms =
                            self.turn_started_at.map(|t| t.elapsed().as_millis() as u64);
                    }
                    self.close_active_text();
                    let entry_id = if let Some(id) = &self.active_think_entry {
                        id.clone()
                    } else {
                        self.block_seq += 1;
                        let id = format!("acp-think-{}", self.block_seq);
                        self.active_think_entry = Some(id.clone());
                        self.thinking_started_at = Some(Instant::now());
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
                    };
                    let _ = self
                        .events
                        .send(DriverEvent::Delta(ChatDeltaOp::AppendText {
                            entry_id,
                            delta: text.to_string(),
                            kind: TextDeltaKind::Thinking,
                        }));
                }
            }
            "agent_message_chunk" => {
                if let Some(text) = update.pointer("/content/text").and_then(|v| v.as_str()) {
                    if text.is_empty() {
                        return;
                    }
                    if self.turn_first_token_ms.is_none() {
                        self.turn_first_token_ms =
                            self.turn_started_at.map(|t| t.elapsed().as_millis() as u64);
                    }
                    self.close_active_thinking();
                    let entry_id = if let Some(id) = &self.active_text_entry {
                        id.clone()
                    } else {
                        self.block_seq += 1;
                        let id = format!("acp-text-{}", self.block_seq);
                        self.active_text_entry = Some(id.clone());
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
                    };
                    let _ = self
                        .events
                        .send(DriverEvent::Delta(ChatDeltaOp::AppendText {
                            entry_id,
                            delta: text.to_string(),
                            kind: TextDeltaKind::Assistant,
                        }));
                }
            }
            "tool_call" | "tool_call_update" => {
                // 工具调用会切分前后文案块，使工具卡片按真实时间顺序插入文本中间。
                self.close_active_thinking();
                self.close_active_text();
                self.handle_tool_update(update);
            }
            "plan" => {
                if let Some(arr) = update.get("entries").and_then(|v| v.as_array()) {
                    let entries = arr
                        .iter()
                        .filter_map(|item| {
                            let content = item.get("content")?.as_str()?.to_string();
                            let status = match item.get("status").and_then(|v| v.as_str()) {
                                Some("completed") => PlanEntryStatus::Completed,
                                Some("in_progress") => PlanEntryStatus::InProgress,
                                _ => PlanEntryStatus::Pending,
                            };
                            let priority = item
                                .get("priority")
                                .and_then(|v| v.as_str())
                                .map(str::to_string);
                            Some(PlanEntry {
                                content,
                                status,
                                priority,
                            })
                        })
                        .collect();
                    let plan_id = format!(
                        "plan:{}",
                        self.active_turn_id.as_deref().unwrap_or("thread")
                    );
                    let _ = self
                        .events
                        .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                            entry: TimelineEntry::Plan {
                                id: plan_id,
                                turn_id: self.active_turn_id.clone(),
                                entries,
                                created_at_ms: now_ms(),
                            },
                        }));
                }
            }
            "usage_update" => {
                if let Some(used) = update.get("used").and_then(|v| v.as_u64()) {
                    self.meta.context_used_tokens = Some(used);
                }
                if let Some(size) = update.get("size").and_then(|v| v.as_u64()) {
                    self.meta.context_window_tokens = Some(size);
                }
                if let Some(cost) = update.pointer("/cost/amount").and_then(|v| v.as_f64()) {
                    self.meta.total_cost_usd = Some(cost);
                }
                self.emit_meta();
            }
            "session_info_update" => {
                if let Some(title) = update.get("title").and_then(|v| v.as_str()) {
                    let trimmed = title.trim();
                    if !trimmed.is_empty() {
                        self.has_explicit_title = true;
                        let _ = self.events.send(DriverEvent::Title(trimmed.to_string()));
                    }
                }
            }
            "available_commands_update" => {
                if let Some(arr) = update.get("availableCommands").and_then(|v| v.as_array()) {
                    let commands: Vec<ChatSlashCommand> = arr
                        .iter()
                        .filter_map(|item| {
                            let raw_name = item.get("name")?.as_str()?.trim();
                            if raw_name.is_empty() {
                                return None;
                            }
                            let name = raw_name.trim_start_matches('/').to_string();
                            let description = item
                                .get("description")
                                .and_then(|v| v.as_str())
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_string);
                            let argument_hint = item
                                .pointer("/input/hint")
                                .and_then(|v| v.as_str())
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_string);
                            let kind = if name.starts_with("skill:") {
                                "skill".to_string()
                            } else {
                                "command".to_string()
                            };
                            Some(ChatSlashCommand {
                                name,
                                description,
                                kind,
                                source: Some("acp".to_string()),
                                argument_hint,
                            })
                        })
                        .collect();
                    self.meta.available_commands = commands;
                    self.emit_meta();
                }
            }
            _ => {}
        }
    }

    fn handle_tool_update(&mut self, update: &Value) {
        let Some(call_id) = update.get("toolCallId").and_then(|v| v.as_str()) else {
            return;
        };
        let prev = self.tool_cache.get(call_id).cloned();

        let (
            prev_name,
            prev_title,
            prev_kind,
            prev_input,
            prev_output,
            prev_diff,
            prev_exit,
            prev_locs,
            prev_created,
        ) = match prev {
            Some(TimelineEntry::ToolCall {
                name,
                title,
                kind,
                input,
                output,
                diff,
                exit_code,
                locations,
                created_at_ms,
                ..
            }) => (
                name,
                title,
                kind,
                input,
                output,
                diff,
                exit_code,
                locations,
                created_at_ms,
            ),
            _ => (
                "tool".to_string(),
                None,
                ToolCallKind::Other,
                Value::Null,
                None,
                None,
                None,
                Vec::new(),
                now_ms(),
            ),
        };

        let name = update
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                update
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or(prev_name)
            });
        let title = update
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or(prev_title);
        let acp_kind = update.get("kind").and_then(|v| v.as_str());
        let kind = if acp_kind.is_some() || prev_kind == ToolCallKind::Other {
            ToolCallKind::from_tool_name(&name, acp_kind)
        } else {
            prev_kind
        };
        let input = update
            .get("rawInput")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or(prev_input);

        let mut output = extract_acp_tool_output(update).or(prev_output);
        let diff = extract_acp_tool_diff(update)
            .or_else(|| infer_tool_diff(&name, &input))
            .map(Box::new)
            .or(prev_diff);
        let exit_code = extract_acp_exit_code(update).or(prev_exit);
        let locations = extract_acp_locations(update).unwrap_or(prev_locs);

        let state = match update.get("status").and_then(|v| v.as_str()) {
            Some("completed") => ToolCallState::Completed,
            Some("failed") => ToolCallState::Failed,
            Some("in_progress") => ToolCallState::Running,
            Some("pending") => ToolCallState::Pending,
            _ => ToolCallState::Running,
        };
        if matches!(state, ToolCallState::Pending | ToolCallState::Running) {
            self.tool_started_at
                .entry(call_id.to_string())
                .or_insert_with(Instant::now);
        }
        let duration_ms = if matches!(state, ToolCallState::Completed | ToolCallState::Failed) {
            let dur = self
                .tool_started_at
                .remove(call_id)
                .map(|t| t.elapsed().as_millis() as u64);
            if dur.is_some() {
                self.session_stats.tool_calls = self.session_stats.tool_calls.saturating_add(1);
                let ms = dur.unwrap_or(0);
                self.session_stats.tool_ms = self.session_stats.tool_ms.saturating_add(ms);
                self.turn_tool_ms = self.turn_tool_ms.saturating_add(ms);
                self.meta.stats = Some(Box::new(self.session_stats.clone()));
            }
            dur
        } else {
            None
        };
        let is_error = state == ToolCallState::Failed || exit_code.is_some_and(|c| c != 0);
        if output.as_deref().is_some_and(str::is_empty) {
            output = None;
        }

        let entry = TimelineEntry::ToolCall {
            id: format!("tool:{call_id}"),
            turn_id: self.active_turn_id.clone(),
            call_id: call_id.to_string(),
            name,
            kind,
            title,
            input,
            output,
            diff,
            exit_code,
            locations,
            state,
            is_error,
            duration_ms,
            created_at_ms: prev_created,
        };
        self.tool_cache.insert(call_id.to_string(), entry.clone());
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry { entry }));
    }

    fn close_active_text(&mut self) {
        if let Some(id) = self.active_text_entry.take() {
            let _ = self
                .events
                .send(DriverEvent::Delta(ChatDeltaOp::FinishEntry {
                    entry_id: id,
                    duration_ms: None,
                }));
        }
    }

    fn close_active_thinking(&mut self) {
        if let Some(id) = self.active_think_entry.take() {
            let dur = self
                .thinking_started_at
                .take()
                .map(|t| t.elapsed().as_millis() as u64);
            let _ = self
                .events
                .send(DriverEvent::Delta(ChatDeltaOp::FinishEntry {
                    entry_id: id,
                    duration_ms: dur,
                }));
        }
    }

    fn emit_meta(&self) {
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::MetaUpdated {
                meta: self.meta.clone(),
            }));
    }
}

fn extract_acp_tool_output(update: &Value) -> Option<String> {
    if let Some(arr) = update.get("content").and_then(|v| v.as_array()) {
        let mut texts = Vec::new();
        for item in arr {
            if item.get("type").and_then(|v| v.as_str()) == Some("content") {
                if let Some(t) = item.pointer("/content/text").and_then(|v| v.as_str()) {
                    texts.push(t.to_string());
                }
            }
        }
        if !texts.is_empty() {
            return Some(texts.join("\n"));
        }
    }
    let raw = update.get("rawOutput")?;
    if raw.is_null() {
        return None;
    }
    if let Some(s) = raw.as_str() {
        return Some(s.to_string());
    }
    if let Some(s) = raw.get("output").and_then(|v| v.as_str()) {
        return Some(s.to_string());
    }
    if let Some(s) = raw.pointer("/metadata/output").and_then(|v| v.as_str()) {
        return Some(s.to_string());
    }
    let stdout = raw.get("stdout").and_then(|v| v.as_str());
    let stderr = raw.get("stderr").and_then(|v| v.as_str());
    if stdout.is_some() || stderr.is_some() {
        let parts: Vec<&str> = [stdout, stderr].into_iter().flatten().collect();
        return Some(parts.join("\n"));
    }
    Some(raw.to_string())
}

fn extract_acp_tool_diff(update: &Value) -> Option<ToolDiff> {
    let arr = update.get("content")?.as_array()?;
    for item in arr {
        if item.get("type").and_then(|v| v.as_str()) == Some("diff") {
            let path = item
                .get("path")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let old_text = item
                .get("oldText")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let new_text = item
                .get("newText")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            return Some(ToolDiff {
                path,
                old_text,
                new_text,
            });
        }
    }
    None
}

fn extract_acp_exit_code(update: &Value) -> Option<i32> {
    let raw = update.get("rawOutput")?;
    raw.get("exitCode")
        .or_else(|| raw.get("exit_code"))
        .or_else(|| raw.pointer("/metadata/exit"))
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)
}

fn extract_acp_locations(update: &Value) -> Option<Vec<String>> {
    let arr = update.get("locations")?.as_array()?;
    let locs: Vec<String> = arr
        .iter()
        .filter_map(|loc| {
            let path = loc.get("path")?.as_str()?;
            if let Some(line) = loc.get("line").and_then(|v| v.as_u64()) {
                Some(format!("{path}:{line}"))
            } else {
                Some(path.to_string())
            }
        })
        .collect();
    Some(locs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxlane_core::chat::ChatTimelineState;

    #[test]
    fn acp_translator_interleaves_text_tools_diffs_and_permissions() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut tr = AcpTranslator::new("opencode-1", ChatSessionMeta::default(), tx);

        tr.begin_turn("Refactor lib.rs", &[]);
        tr.handle_session_update(&json!({
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "Let me inspect." }
            }
        }));
        tr.handle_session_update(&json!({
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "tc-1",
                "title": "Edit src/lib.rs",
                "kind": "edit",
                "status": "pending"
            }
        }));
        assert!(tr.handle_permission_request(
            json!(99),
            &json!({
                "sessionId": "s1",
                "toolCall": { "toolCallId": "tc-1", "title": "Edit src/lib.rs" },
                "options": [
                    { "optionId": "allow_once", "name": "Allow once", "kind": "allow_once" },
                    { "optionId": "reject", "name": "Reject", "kind": "reject_once" }
                ]
            })
        ));
        tr.resolve_permission_ui(&tr.pending_permissions.keys().next().unwrap().clone());
        tr.handle_session_update(&json!({
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "tool_call_update",
                "toolCallId": "tc-1",
                "status": "completed",
                "content": [{ "type": "diff", "path": "src/lib.rs", "oldText": "a", "newText": "b" }]
            }
        }));
        tr.handle_session_update(&json!({
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "Updated cleanly." }
            }
        }));
        tr.finish_turn(None);

        let mut state = ChatTimelineState::default();
        while let Ok(ev) = rx.try_recv() {
            if let DriverEvent::Delta(op) = ev {
                state.apply_op(op);
            }
        }

        // User -> AssistantText("Let me inspect.") -> ToolCall(with Diff) -> AssistantText("Updated cleanly.")
        assert_eq!(state.entries.len(), 4);
        match &state.entries[2] {
            TimelineEntry::ToolCall { diff, .. } => {
                assert_eq!(diff.as_ref().unwrap().new_text, "b");
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(state.pending_approvals.is_empty());
    }
}
