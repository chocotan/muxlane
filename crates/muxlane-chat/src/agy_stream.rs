//! Antigravity CLI (`agy`) 原生 `--input-format stream-json --output-format stream-json` 多轮对话驱动。
use crate::driver::{read_lf_line, ChatDriverHandle, DriverCmd, DriverEvent, DriverLaunchSpec};
use crate::transcript::infer_tool_diff;
use anyhow::Context;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use muxlane_core::chat::{
    now_ms, ChatAttachment, ChatDeltaOp, ChatModelInfo, ChatSessionMeta, MessageUsage,
    QueuedMessage, SessionDriverMode, SessionStats, StreamingBehavior, TextDeltaKind,
    TimelineEntry, ToolCallKind, ToolCallState, ToolDiff,
};
use muxlane_core::model::AgentStatus;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::process::Stdio;
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
        if let Err(error) = run_agy_stream(spec, initial_meta, cmd_rx, events.clone()).await {
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

fn sync_queue_meta(translator: &mut AgyStreamTranslator, pending: &VecDeque<PendingPrompt>) {
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

async fn fetch_agy_models(program: &str) -> Vec<ChatModelInfo> {
    let output = Command::new(program)
        .arg("models")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await;
    let Ok(out) = output else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut models = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("Fetching") {
            continue;
        }
        if let Some((id, name)) = trimmed.split_once('\t') {
            let id = id.trim();
            let name = name.trim();
            if !id.is_empty() {
                models.push(ChatModelInfo {
                    provider: "antigravity".to_string(),
                    id: id.to_string(),
                    name: if name.is_empty() {
                        id.to_string()
                    } else {
                        name.to_string()
                    },
                });
            }
        }
    }
    models
}

fn build_agy_args(spec: &DriverLaunchSpec, conversation_id: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = spec
        .args
        .iter()
        .filter(|a| *a != "--acp" && *a != "--experimental-acp")
        .cloned()
        .collect();

    if !args.iter().any(|a| a == "--input-format") {
        args.push("--input-format".into());
        args.push("stream-json".into());
    }
    if !args.iter().any(|a| a == "--output-format") {
        args.push("--output-format".into());
        args.push("stream-json".into());
    }
    if !args.iter().any(|a| a == "--dangerously-skip-permissions") {
        args.push("--dangerously-skip-permissions".into());
    }
    if let Some(conv) = conversation_id.filter(|s| !s.trim().is_empty()) {
        if !args.iter().any(|a| a == "--conversation") {
            args.push("--conversation".into());
            args.push(conv.to_string());
        }
    }
    if let Some(model) = spec.model.as_deref().filter(|s| !s.trim().is_empty()) {
        if !args.iter().any(|a| a == "--model") {
            args.push("--model".into());
            args.push(model.to_string());
        }
    }
    if let Some(effort) = spec
        .thinking_level
        .as_deref()
        .filter(|s| matches!(*s, "low" | "medium" | "high"))
    {
        if !args.iter().any(|a| a == "--effort") {
            args.push("--effort".into());
            args.push(effort.to_string());
        }
    }
    if !args
        .iter()
        .any(|a| a == "--print" || a == "-p" || a.starts_with("--print="))
    {
        args.push("--print=".into());
    }
    args
}

async fn run_agy_stream(
    mut spec: DriverLaunchSpec,
    mut meta: ChatSessionMeta,
    mut cmd_rx: mpsc::UnboundedReceiver<DriverCmd>,
    events: mpsc::UnboundedSender<DriverEvent>,
) -> anyhow::Result<()> {
    meta.driver = SessionDriverMode::Acp;
    meta.supports_chat_input = true;
    meta.supports_terminal_switch = false;
    meta.available_thinking_levels = vec!["low".into(), "medium".into(), "high".into()];
    if let Some(model_id) = spec.model.as_deref().filter(|s| !s.is_empty()) {
        meta.current_model = Some(ChatModelInfo {
            provider: "antigravity".into(),
            id: model_id.to_string(),
            name: model_id.to_string(),
        });
    }
    if let Some(level) = spec.thinking_level.clone() {
        meta.thinking_level = Some(level);
    }

    let mut translator = AgyStreamTranslator::new(&spec.agent, meta, events.clone());
    translator.emit_meta();

    let (models_tx, mut models_rx) = mpsc::unbounded_channel::<Vec<ChatModelInfo>>();
    let prog_for_models = spec.program.clone();
    tokio::spawn(async move {
        let list = fetch_agy_models(&prog_for_models).await;
        let _ = models_tx.send(list);
    });

    let mut pending_prompts: VecDeque<PendingPrompt> = VecDeque::new();
    let mut exit_requested = false;

    loop {
        let launch_args = build_agy_args(&spec, translator.meta.provider_session_id.as_deref());
        let mut command = Command::new(&spec.program);
        command
            .args(&launch_args)
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
            .with_context(|| format!("failed to launch `{}` in stream-json mode", spec.program))?;
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
        let mut recent_stderr: VecDeque<String> = VecDeque::with_capacity(20);
        let mut session_ready = false;
        let mut respawn_requested = false;

        loop {
            tokio::select! {
                Some(models) = models_rx.recv() => {
                    if !models.is_empty() {
                        translator.meta.available_models = models;
                        translator.emit_meta();
                    }
                }
                cmd = cmd_rx.recv() => match cmd {
                    Some(DriverCmd::SendPrompt { text, attachments, .. }) => {
                        let prompt = PendingPrompt {
                            id: format!("queued-{}", ulid::Ulid::new()),
                            text,
                            attachments,
                        };
                        if !session_ready || translator.meta.is_streaming {
                            pending_prompts.push_back(prompt);
                            sync_queue_meta(&mut translator, &pending_prompts);
                        } else {
                            dispatch_agy_prompt(&mut stdin, &mut translator, prompt).await?;
                        }
                    }
                    Some(DriverCmd::DeleteQueuedMessage { message_id }) => {
                        match message_id {
                            Some(id) => pending_prompts.retain(|p| p.id != id),
                            None => pending_prompts.clear(),
                        }
                        sync_queue_meta(&mut translator, &pending_prompts);
                    }
                    Some(DriverCmd::Abort) => {
                        if translator.meta.is_streaming {
                            let _ = child.kill().await;
                            translator.finish_aborted();
                            respawn_requested = true;
                            break;
                        }
                    }
                    Some(DriverCmd::SetModel { model_id, .. }) => {
                        let mid = model_id.trim().to_string();
                        if !mid.is_empty() {
                            let display_name = translator
                                .meta
                                .available_models
                                .iter()
                                .find(|m| m.id == mid)
                                .map(|m| m.name.clone())
                                .unwrap_or_else(|| mid.clone());
                            spec.model = Some(mid.clone());
                            translator.meta.current_model = Some(ChatModelInfo {
                                provider: "antigravity".into(),
                                id: mid,
                                name: display_name,
                            });
                            translator.emit_meta();
                            if !translator.meta.is_streaming {
                                let _ = child.kill().await;
                                respawn_requested = true;
                                break;
                            }
                        }
                    }
                    Some(DriverCmd::SetThinkingLevel { level }) => {
                        let lv = level.trim().to_string();
                        if !lv.is_empty() {
                            spec.thinking_level = Some(lv.clone());
                            translator.meta.thinking_level = Some(lv);
                            translator.emit_meta();
                            if !translator.meta.is_streaming {
                                let _ = child.kill().await;
                                respawn_requested = true;
                                break;
                            }
                        }
                    }
                    Some(DriverCmd::RespondApproval { .. }) => {}
                    Some(DriverCmd::Shutdown) | None => {
                        exit_requested = true;
                        let _ = child.kill().await;
                        break;
                    }
                },
                Some(line) = stderr_rx.recv() => {
                    if recent_stderr.len() >= 20 {
                        recent_stderr.pop_front();
                    }
                    recent_stderr.push_back(line);
                }
                line_res = read_lf_line(&mut stdout_reader) => match line_res {
                    Ok(Some(line)) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
                            continue;
                        };
                        let outcome = translator.handle_event(&val);
                        if outcome.initialized && !session_ready {
                            session_ready = true;
                            if !translator.meta.is_streaming {
                                if let Some(next) = pending_prompts.pop_front() {
                                    sync_queue_meta(&mut translator, &pending_prompts);
                                    dispatch_agy_prompt(&mut stdin, &mut translator, next).await?;
                                }
                            }
                        }
                        if outcome.turn_finished {
                            if let Some(next) = pending_prompts.pop_front() {
                                sync_queue_meta(&mut translator, &pending_prompts);
                                dispatch_agy_prompt(&mut stdin, &mut translator, next).await?;
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        if !exit_requested {
                            let _ = events.send(DriverEvent::Exited {
                                error: Some(format!("agy stream-json stdout read error: {error}")),
                            });
                        }
                        return Ok(());
                    }
                }
            }
        }

        if exit_requested {
            let _ = events.send(DriverEvent::Exited { error: None });
            return Ok(());
        }
        if respawn_requested {
            continue;
        }

        let status = child.wait().await.ok();
        let err = if status.is_some_and(|s| s.success()) {
            None
        } else {
            let tail = recent_stderr
                .iter()
                .rev()
                .take(4)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join(" | ");
            Some(if tail.is_empty() {
                format!("agy stream-json exited with status {status:?}")
            } else {
                format!("agy stream-json exited ({status:?}): {tail}")
            })
        };
        if let Some(msg) = err.clone() {
            translator.meta.is_streaming = false;
            translator.meta.last_error = Some(msg.clone());
            translator.emit_meta();
            let _ = events.send(DriverEvent::Delta(ChatDeltaOp::TurnFinished {
                turn_id: translator.active_turn_id.take(),
                error: Some(msg),
            }));
        }
        let _ = events.send(DriverEvent::Exited { error: err });
        return Ok(());
    }
}

async fn dispatch_agy_prompt(
    stdin: &mut tokio::process::ChildStdin,
    translator: &mut AgyStreamTranslator,
    prompt: PendingPrompt,
) -> anyhow::Result<()> {
    let turn_id = format!("agy-turn-{}", ulid::Ulid::new());
    let mut prompt_text = prompt.text.clone();
    let mut staged_paths = Vec::new();

    for (idx, att) in prompt.attachments.iter().enumerate() {
        if let Some(path) = att.path.as_deref().filter(|s| !s.trim().is_empty()) {
            staged_paths.push(path.to_string());
        } else if let Some(b64) = att.data_b64.as_deref().filter(|s| !s.trim().is_empty()) {
            if let Ok(bytes) = BASE64.decode(b64.as_bytes()) {
                let ext = match att.mime_type.as_str() {
                    "image/jpeg" | "image/jpg" => "jpg",
                    "image/webp" => "webp",
                    "image/gif" => "gif",
                    _ => "png",
                };
                let tmp_path = std::env::temp_dir()
                    .join(format!("muxlane-agy-{}-{idx}.{ext}", ulid::Ulid::new()));
                if std::fs::write(&tmp_path, bytes).is_ok() {
                    staged_paths.push(tmp_path.display().to_string());
                }
            }
        }
    }
    if !staged_paths.is_empty() {
        let note = staged_paths
            .iter()
            .map(|p| format!("[Attached image file: {p}]"))
            .collect::<Vec<_>>()
            .join("\n");
        if prompt_text.trim().is_empty() {
            prompt_text = note;
        } else {
            prompt_text = format!("{prompt_text}\n\n{note}");
        }
    }

    translator.start_turn(turn_id.clone(), prompt.text, prompt.attachments);
    let payload = json!({
        "event": "user",
        "message": {
            "role": "user",
            "content": prompt_text,
        }
    });
    let mut raw = serde_json::to_string(&payload)?;
    raw.push('\n');
    stdin.write_all(raw.as_bytes()).await?;
    stdin.flush().await?;
    Ok(())
}

#[derive(Debug, Default)]
struct EventOutcome {
    initialized: bool,
    turn_finished: bool,
}

struct AgyStreamTranslator {
    agent: String,
    meta: ChatSessionMeta,
    events: mpsc::UnboundedSender<DriverEvent>,
    active_turn_id: Option<String>,
    turn_seq: u64,
    active_text_steps: HashSet<String>,
    active_thinking_steps: HashSet<String>,
    tool_inputs: HashMap<String, (Value, Option<Box<ToolDiff>>)>,
    session_stats: SessionStats,
}

impl AgyStreamTranslator {
    fn new(
        agent: impl Into<String>,
        meta: ChatSessionMeta,
        events: mpsc::UnboundedSender<DriverEvent>,
    ) -> Self {
        Self {
            agent: agent.into(),
            meta,
            events,
            active_turn_id: None,
            turn_seq: 0,
            active_text_steps: HashSet::new(),
            active_thinking_steps: HashSet::new(),
            tool_inputs: HashMap::new(),
            session_stats: SessionStats::default(),
        }
    }

    fn emit_meta(&self) {
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::MetaUpdated {
                meta: self.meta.clone(),
            }));
    }

    fn start_turn(&mut self, turn_id: String, text: String, attachments: Vec<ChatAttachment>) {
        self.turn_seq += 1;
        self.active_turn_id = Some(turn_id.clone());
        self.active_text_steps.clear();
        self.active_thinking_steps.clear();
        self.meta.is_streaming = true;
        self.meta.last_error = None;
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                entry: TimelineEntry::UserMessage {
                    id: format!("user-{}", ulid::Ulid::new()),
                    turn_id: Some(turn_id),
                    text,
                    attachments,
                    created_at_ms: now_ms(),
                },
            }));
        self.emit_meta();
        let _ = self.events.send(DriverEvent::Status {
            status: AgentStatus::Working,
            message: None,
            reset_seen: false,
        });
    }

    fn finish_aborted(&mut self) {
        self.meta.is_streaming = false;
        let turn_id = self.active_turn_id.take();
        self.active_text_steps.clear();
        self.active_thinking_steps.clear();
        self.emit_meta();
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::TurnFinished {
                turn_id,
                error: Some("已中止当前生成".to_string()),
            }));
        let _ = self.events.send(DriverEvent::Status {
            status: AgentStatus::Idle,
            message: None,
            reset_seen: false,
        });
    }

    fn handle_event(&mut self, record: &Value) -> EventOutcome {
        let mut outcome = EventOutcome::default();
        let event = record.get("event").and_then(Value::as_str).unwrap_or("");
        match event {
            "init" => {
                if let Some(conv_id) = record
                    .get("conversation_id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    self.meta.provider_session_id = Some(conv_id.to_string());
                }
                self.emit_meta();
                outcome.initialized = true;
            }
            "step_update" => {
                if let Some(upd) = record.get("step_update") {
                    self.handle_step_update(upd);
                }
            }
            "result" => {
                if let Some(res) = record.get("result") {
                    self.handle_result(res);
                }
                outcome.turn_finished = true;
            }
            _ => {}
        }
        outcome
    }

    fn handle_step_update(&mut self, upd: &Value) {
        let step_type = upd.get("step_type").and_then(Value::as_str).unwrap_or("");
        let state = upd.get("state").and_then(Value::as_str).unwrap_or("");
        let step_index = upd.get("step_index").and_then(Value::as_u64).unwrap_or(0);
        let duration_ms = upd
            .get("duration_seconds")
            .and_then(Value::as_f64)
            .map(|s| (s * 1000.0).round() as u64);

        match step_type {
            "agent_response" => {
                let text_id = format!("{}-t{}-text-{step_index}", self.agent, self.turn_seq);
                let think_id = format!("{}-t{}-think-{step_index}", self.agent, self.turn_seq);

                if let Some(td) = upd
                    .get("thinking_delta")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    if self.active_thinking_steps.insert(think_id.clone()) {
                        let _ = self
                            .events
                            .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                                entry: TimelineEntry::Thinking {
                                    id: think_id.clone(),
                                    turn_id: self.active_turn_id.clone(),
                                    text: String::new(),
                                    streaming: true,
                                    duration_ms: None,
                                    created_at_ms: now_ms(),
                                },
                            }));
                    }
                    let _ = self
                        .events
                        .send(DriverEvent::Delta(ChatDeltaOp::AppendText {
                            entry_id: think_id.clone(),
                            delta: td.to_string(),
                            kind: TextDeltaKind::Thinking,
                        }));
                }

                if let Some(delta) = upd
                    .get("text_delta")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    if self.active_text_steps.insert(text_id.clone()) {
                        let model = self.meta.current_model.as_ref().map(|m| m.name.clone());
                        let _ = self
                            .events
                            .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                                entry: TimelineEntry::AssistantText {
                                    id: text_id.clone(),
                                    turn_id: self.active_turn_id.clone(),
                                    text: String::new(),
                                    streaming: true,
                                    model,
                                    usage: None,
                                    created_at_ms: now_ms(),
                                },
                            }));
                    }
                    let _ = self
                        .events
                        .send(DriverEvent::Delta(ChatDeltaOp::AppendText {
                            entry_id: text_id.clone(),
                            delta: delta.to_string(),
                            kind: TextDeltaKind::Assistant,
                        }));
                }

                if state == "DONE" || state == "ERROR" {
                    if self.active_thinking_steps.remove(&think_id) {
                        let _ = self
                            .events
                            .send(DriverEvent::Delta(ChatDeltaOp::FinishEntry {
                                entry_id: think_id,
                                duration_ms,
                            }));
                    }
                    if self.active_text_steps.remove(&text_id) {
                        let _ = self
                            .events
                            .send(DriverEvent::Delta(ChatDeltaOp::FinishEntry {
                                entry_id: text_id.clone(),
                                duration_ms,
                            }));
                        if let Some(usage) = parse_agy_usage(upd.get("usage"), duration_ms) {
                            let _ = self.events.send(DriverEvent::Delta(
                                ChatDeltaOp::SetAssistantUsage {
                                    entry_id: text_id,
                                    usage: Box::new(usage),
                                },
                            ));
                        }
                    }
                    self.session_stats.steps = self.session_stats.steps.saturating_add(1);
                }
            }
            "tool" => {
                let tool_info = upd.get("tool_info");
                let name = upd
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .or_else(|| {
                        tool_info
                            .and_then(|ti| ti.get("name"))
                            .and_then(Value::as_str)
                    })
                    .unwrap_or("tool")
                    .to_string();
                let call_id = format!("t{}-step-{step_index}", self.turn_seq);
                let entry_id = format!("agy-tool-{call_id}");

                let params = tool_info
                    .and_then(|ti| ti.get("parameters"))
                    .filter(|v| !v.is_null())
                    .cloned();
                let (input, diff) = match params {
                    Some(p) => {
                        let d = infer_agy_tool_diff(&name, &p)
                            .or_else(|| infer_tool_diff(&name, &p))
                            .map(Box::new);
                        self.tool_inputs
                            .insert(call_id.clone(), (p.clone(), d.clone()));
                        (p, d)
                    }
                    None => self
                        .tool_inputs
                        .get(&call_id)
                        .cloned()
                        .unwrap_or((Value::Null, None)),
                };

                let output = tool_info
                    .and_then(|ti| ti.get("output").or_else(|| ti.get("error")))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let is_error = state == "ERROR"
                    || tool_info
                        .and_then(|ti| ti.get("error"))
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.is_empty());
                let tc_state = match state {
                    "DONE" => {
                        self.tool_inputs.remove(&call_id);
                        self.session_stats.tool_calls =
                            self.session_stats.tool_calls.saturating_add(1);
                        if let Some(ms) = duration_ms {
                            self.session_stats.tool_ms =
                                self.session_stats.tool_ms.saturating_add(ms);
                        }
                        if is_error {
                            ToolCallState::Failed
                        } else {
                            ToolCallState::Completed
                        }
                    }
                    "ERROR" => {
                        self.tool_inputs.remove(&call_id);
                        ToolCallState::Failed
                    }
                    _ => ToolCallState::Running,
                };

                let _ = self
                    .events
                    .send(DriverEvent::Delta(ChatDeltaOp::UpsertEntry {
                        entry: TimelineEntry::ToolCall {
                            id: entry_id,
                            turn_id: self.active_turn_id.clone(),
                            call_id,
                            kind: ToolCallKind::from_tool_name(&name, None),
                            name,
                            title: None,
                            input,
                            output,
                            attachments: Vec::new(),
                            diff,
                            exit_code: None,
                            locations: Vec::new(),
                            state: tc_state,
                            is_error,
                            duration_ms,
                            created_at_ms: now_ms(),
                        },
                    }));
            }
            _ => {}
        }
    }

    fn handle_result(&mut self, res: &Value) {
        if let Some(conv_id) = res
            .get("conversation_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            self.meta.provider_session_id = Some(conv_id.to_string());
        }
        let status = res.get("status").and_then(Value::as_str).unwrap_or("");
        let err = if status == "ERROR" {
            res.get("error")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| Some("agy returned error".to_string()))
        } else {
            None
        };

        let duration_ms = res
            .get("duration_seconds")
            .and_then(Value::as_f64)
            .map(|s| (s * 1000.0).round() as u64);
        if let Some(u) = parse_agy_usage(res.get("usage"), duration_ms) {
            self.session_stats.input_tokens = u.input_tokens;
            self.session_stats.output_tokens = u.output_tokens;
            self.session_stats.cache_read_tokens = u.cache_read_tokens;
            self.session_stats.reasoning_tokens = u.reasoning_tokens;
            self.session_stats.total_tokens = u.total_tokens;
            let ctx = u.input_tokens.saturating_add(u.cache_read_tokens);
            if ctx > 0 {
                self.meta.context_used_tokens = Some(ctx);
            }
        }
        self.session_stats.turns = res
            .get("num_turns")
            .and_then(Value::as_u64)
            .map(|n| n as u32)
            .unwrap_or(self.session_stats.turns.saturating_add(1));
        self.meta.stats = Some(Box::new(self.session_stats.clone()));
        self.meta.is_streaming = false;
        self.meta.last_error = err.clone();
        let turn_id = self.active_turn_id.take();
        self.emit_meta();
        let _ = self
            .events
            .send(DriverEvent::Delta(ChatDeltaOp::TurnFinished {
                turn_id,
                error: err.clone(),
            }));
        let _ = self.events.send(DriverEvent::Status {
            status: if err.is_some() {
                AgentStatus::Failed
            } else {
                AgentStatus::Done
            },
            message: err,
            reset_seen: true,
        });
    }
}

fn parse_agy_usage(val: Option<&Value>, duration_ms: Option<u64>) -> Option<MessageUsage> {
    let obj = val?.as_object()?;
    let input = obj.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
    let output = obj
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let reasoning = obj
        .get("thinking_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_read = obj
        .get("cache_read_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total = obj
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(input.saturating_add(output));
    if total == 0 && input == 0 && output == 0 && duration_ms.is_none() {
        return None;
    }
    Some(MessageUsage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_write_tokens: 0,
        reasoning_tokens: reasoning,
        total_tokens: total,
        cost_usd: None,
        duration_ms,
        ttft_ms: None,
    })
}

fn infer_agy_tool_diff(name: &str, input: &Value) -> Option<ToolDiff> {
    let lower = name.to_ascii_lowercase();
    let path = input
        .get("TargetFile")
        .or_else(|| input.get("AbsolutePath"))
        .and_then(Value::as_str)
        .map(str::to_string);
    if lower == "replace_file_content" {
        let old = input
            .get("TargetContent")
            .and_then(Value::as_str)?
            .to_string();
        let new = input
            .get("ReplacementContent")
            .and_then(Value::as_str)?
            .to_string();
        return Some(ToolDiff {
            path,
            old_text: old,
            new_text: new,
        });
    }
    if lower == "write_to_file" {
        let content = input
            .get("CodeContent")
            .and_then(Value::as_str)?
            .to_string();
        return Some(ToolDiff {
            path,
            old_text: String::new(),
            new_text: content,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxlane_core::chat::ChatTimelineState;

    #[test]
    fn agy_stream_translator_handles_init_tools_text_and_result() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut tr = AgyStreamTranslator::new("agy-test", ChatSessionMeta::default(), tx);

        let init = tr.handle_event(&json!({
            "event": "init",
            "conversation_id": "conv-123",
            "init": { "cwd": "/repo", "tools": ["view_file", "run_command"] }
        }));
        assert!(init.initialized);
        assert_eq!(tr.meta.provider_session_id.as_deref(), Some("conv-123"));

        tr.start_turn("turn-1".into(), "check cargo".into(), Vec::new());

        tr.handle_event(&json!({
            "event": "step_update",
            "step_update": {
                "step_index": 2,
                "state": "ACTIVE",
                "step_type": "tool",
                "tool_name": "view_file",
                "tool_info": {
                    "name": "view_file",
                    "parameters": { "AbsolutePath": "/repo/Cargo.toml" }
                }
            }
        }));
        tr.handle_event(&json!({
            "event": "step_update",
            "step_update": {
                "step_index": 2,
                "state": "DONE",
                "step_type": "tool",
                "tool_name": "view_file",
                "duration_seconds": 0.042,
                "tool_info": {
                    "name": "view_file",
                    "parameters": { "AbsolutePath": "/repo/Cargo.toml" },
                    "output": "56 lines, 1399 bytes"
                }
            }
        }));
        tr.handle_event(&json!({
            "event": "step_update",
            "step_update": {
                "step_index": 3,
                "state": "ACTIVE",
                "step_type": "agent_response",
                "text_delta": "DONE"
            }
        }));
        tr.handle_event(&json!({
            "event": "step_update",
            "step_update": {
                "step_index": 3,
                "state": "DONE",
                "step_type": "agent_response",
                "text_delta": "!",
                "duration_seconds": 1.2,
                "usage": {
                    "input_tokens": 2400,
                    "output_tokens": 47,
                    "thinking_tokens": 45,
                    "cache_read_tokens": 15810,
                    "total_tokens": 2447
                }
            }
        }));
        let fin = tr.handle_event(&json!({
            "event": "result",
            "result": {
                "conversation_id": "conv-123",
                "status": "SUCCESS",
                "response": "DONE!",
                "duration_seconds": 2.4,
                "num_turns": 1,
                "usage": {
                    "input_tokens": 2400,
                    "output_tokens": 47,
                    "thinking_tokens": 45,
                    "cache_read_tokens": 15810,
                    "total_tokens": 2447
                }
            }
        }));
        assert!(fin.turn_finished);

        let mut state = ChatTimelineState::default();
        while let Ok(ev) = rx.try_recv() {
            if let DriverEvent::Delta(op) = ev {
                state.apply_op(op);
            }
        }
        assert!(!state.meta.is_streaming);
        assert_eq!(state.entries.len(), 3);
        match &state.entries[1] {
            TimelineEntry::ToolCall {
                name,
                kind,
                output,
                state,
                duration_ms,
                ..
            } => {
                assert_eq!(name, "view_file");
                assert_eq!(*kind, ToolCallKind::Read);
                assert_eq!(output.as_deref(), Some("56 lines, 1399 bytes"));
                assert_eq!(*state, ToolCallState::Completed);
                assert_eq!(*duration_ms, Some(42));
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
        match &state.entries[2] {
            TimelineEntry::AssistantText {
                text,
                streaming,
                usage,
                ..
            } => {
                assert_eq!(text, "DONE!");
                assert!(!streaming);
                assert_eq!(usage.as_deref().unwrap().total_tokens, 2447);
            }
            other => panic!("expected AssistantText, got {other:?}"),
        }
    }
}
