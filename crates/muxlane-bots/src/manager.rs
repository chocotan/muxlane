//! BotManager: Bot CRUD, Cron/Interval scheduler, concurrency guard, permission policy engine,
//! ChatManager run orchestrator, worktree teardown, and MEMORY.md distillation.
use crate::connectors::{run_telegram_poller, send_im_reply, start_webhook_http_server};
use crate::cron::CronSchedule;
use crate::soul::{compose_bot_prompt, distill_memory};
use crate::worktree::{prepare_worktree, release_worktree_if_clean};
use anyhow::bail;
use muxlane_chat::ChatManager;
use muxlane_core::bot::{
    BotDefinition, BotId, BotPermissionProfile, BotRunRecord, BotRunStatus, BotTrigger,
    ImConnectorConfig,
};
use muxlane_core::chat::{ChatDeltaEvent, ChatDeltaOp, PendingApproval, TimelineEntry};
use muxlane_core::model::{now_secs, AgentId};
use muxlane_core::protocol::{events, EventMsg};
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, RwLock as StdRwLock};
use tokio::sync::{mpsc, Mutex};

const MAX_RETAINED_RUNS: usize = 120;

pub trait BotHostBridge: Send + Sync {
    fn resolve_project_dir(
        &self,
        project_id: String,
    ) -> Pin<Box<dyn Future<Output = Option<PathBuf>> + Send>>;

    fn spawn_chat_agent(
        &self,
        bot: BotDefinition,
        cwd: PathBuf,
        prompt: String,
        title: String,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<AgentId>> + Send>>;

    fn emit_bot_event(&self, event: EventMsg);

    fn persist_bots(&self);
}

pub struct BotManager {
    chat: Arc<ChatManager>,
    bridge: Arc<dyn BotHostBridge>,
    http_client: reqwest::Client,
    bots: Mutex<BTreeMap<BotId, BotDefinition>>,
    runs: Mutex<Vec<BotRunRecord>>,
    last_cron_minute: Mutex<HashMap<BotId, u64>>,
    last_interval_run: Mutex<HashMap<BotId, u64>>,
    webhook_addr: StdRwLock<Option<SocketAddr>>,
}

impl BotManager {
    pub fn new(chat: Arc<ChatManager>, bridge: Arc<dyn BotHostBridge>) -> Arc<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();
        Arc::new(Self {
            chat,
            bridge,
            http_client,
            bots: Mutex::new(BTreeMap::new()),
            runs: Mutex::new(Vec::new()),
            last_cron_minute: Mutex::new(HashMap::new()),
            last_interval_run: Mutex::new(HashMap::new()),
            webhook_addr: StdRwLock::new(None),
        })
    }

    pub fn webhook_addr(&self) -> Option<SocketAddr> {
        self.webhook_addr.read().ok().and_then(|g| *g)
    }

    pub async fn restore_snapshot(&self, bots: Vec<BotDefinition>, runs: Vec<BotRunRecord>) {
        let mut map = self.bots.lock().await;
        map.clear();
        for bot in bots {
            map.insert(bot.id.clone(), bot);
        }
        drop(map);

        let mut run_list = self.runs.lock().await;
        *run_list = runs.into_iter().take(MAX_RETAINED_RUNS).collect();
    }

    pub async fn export_snapshot(&self) -> (Vec<BotDefinition>, Vec<BotRunRecord>) {
        let bots = self.bots.lock().await.values().cloned().collect();
        let runs = self.runs.lock().await.clone();
        (bots, runs)
    }

    pub async fn list_bots(&self) -> Vec<BotDefinition> {
        self.bots.lock().await.values().cloned().collect()
    }

    pub async fn get_bot(&self, bot_id: &str) -> Option<BotDefinition> {
        self.bots.lock().await.get(bot_id).cloned()
    }

    pub async fn save_bot(&self, mut bot: BotDefinition) -> anyhow::Result<BotDefinition> {
        if bot.name.trim().is_empty() {
            bail!("bot name cannot be empty");
        }
        if bot.id.trim().is_empty() {
            bot.id = muxlane_core::model::new_id("bot");
        }
        // Validate any Cron expressions before saving.
        for trig in &bot.triggers {
            if let BotTrigger::Cron { expr } = trig {
                CronSchedule::parse(expr)?;
            }
        }
        let now = now_secs();
        if bot.created_at == 0 {
            bot.created_at = now;
        }
        bot.updated_at = now;
        bot.max_concurrency = bot.max_concurrency.max(1);
        bot.timeout_secs = bot.timeout_secs.max(15);

        self.bots.lock().await.insert(bot.id.clone(), bot.clone());
        if let Ok(payload) = serde_json::to_value(&bot) {
            self.bridge
                .emit_bot_event(EventMsg::new(events::BOT_UPDATED, payload));
        }
        self.bridge.persist_bots();
        Ok(bot)
    }

    pub async fn delete_bot(&self, bot_id: &str) -> bool {
        let removed = self.bots.lock().await.remove(bot_id).is_some();
        if removed {
            self.bridge.emit_bot_event(EventMsg::new(
                events::BOT_UPDATED,
                serde_json::json!({ "deleted": bot_id }),
            ));
            self.bridge.persist_bots();
        }
        removed
    }

    pub async fn list_runs(&self, bot_id: Option<&str>, limit: usize) -> Vec<BotRunRecord> {
        let runs = self.runs.lock().await;
        runs.iter()
            .filter(|r| bot_id.is_none_or(|id| r.bot_id == id))
            .take(limit.max(1))
            .cloned()
            .collect()
    }

    pub async fn start_background_daemons(self: &Arc<Self>, webhook_bind: Option<&str>) {
        if let Some(bind) = webhook_bind {
            match start_webhook_http_server(bind, Arc::clone(self)).await {
                Ok(addr) => {
                    if let Ok(mut guard) = self.webhook_addr.write() {
                        *guard = Some(addr);
                    }
                    tracing::info!(%addr, "bot webhook ingress listening");
                }
                Err(err) => tracing::warn!(%err, "failed to bind bot webhook listener"),
            }
        }
        let sched_mgr = Arc::clone(self);
        tokio::spawn(async move {
            sched_mgr.scheduler_loop().await;
        });
        let tg_mgr = Arc::clone(self);
        tokio::spawn(async move {
            run_telegram_poller(tg_mgr).await;
        });
    }

    async fn scheduler_loop(self: Arc<Self>) {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            let now = now_secs();
            let current_minute = now / 60;
            let bots = self.list_bots().await;
            for bot in bots {
                if !bot.enabled {
                    continue;
                }
                let mut due_source: Option<String> = None;
                for trig in &bot.triggers {
                    match trig {
                        BotTrigger::Interval { every_secs } if *every_secs > 0 => {
                            let last = self
                                .last_interval_run
                                .lock()
                                .await
                                .get(&bot.id)
                                .copied()
                                .unwrap_or(0);
                            if now.saturating_sub(last) >= *every_secs {
                                self.last_interval_run
                                    .lock()
                                    .await
                                    .insert(bot.id.clone(), now);
                                due_source = Some("interval".into());
                                break;
                            }
                        }
                        BotTrigger::Cron { expr } => {
                            let already = self
                                .last_cron_minute
                                .lock()
                                .await
                                .get(&bot.id)
                                .copied()
                                .is_some_and(|m| m == current_minute);
                            if !already {
                                if let Ok(sched) = CronSchedule::parse(expr) {
                                    if sched.matches_unix_secs(now) {
                                        self.last_cron_minute
                                            .lock()
                                            .await
                                            .insert(bot.id.clone(), current_minute);
                                        due_source = Some("cron".into());
                                        break;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(source) = due_source {
                    let mgr = Arc::clone(&self);
                    let id = bot.id.clone();
                    tokio::spawn(async move {
                        if let Err(err) = mgr.trigger_bot(&id, source, None, None).await {
                            tracing::debug!(bot_id = %id, %err, "scheduled bot skipped");
                        }
                    });
                }
            }
        }
    }

    pub async fn trigger_bot(
        self: &Arc<Self>,
        bot_id: &str,
        trigger_source: String,
        payload: Option<String>,
        reply_connector: Option<ImConnectorConfig>,
    ) -> anyhow::Result<BotRunRecord> {
        let bot = self
            .get_bot(bot_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("unknown bot: {bot_id}"))?;
        if trigger_source != "manual" && !bot.enabled {
            bail!("bot is disabled");
        }
        {
            let runs = self.runs.lock().await;
            let active_count = runs
                .iter()
                .filter(|r| r.bot_id == bot.id && !r.status.is_terminal())
                .count();
            if active_count >= bot.max_concurrency.max(1) as usize {
                bail!("bot max concurrency ({}) reached", bot.max_concurrency);
            }
        }

        let Some(project_dir) = self
            .bridge
            .resolve_project_dir(bot.project_id.clone())
            .await
        else {
            bail!("project {} not found", bot.project_id);
        };

        let run_id = muxlane_core::model::new_id("run");
        let lease = if bot.isolated_worktree {
            prepare_worktree(&project_dir, &bot.id, &run_id).await
        } else {
            crate::worktree::WorktreeLease {
                workdir: project_dir.clone(),
                repo_toplevel: None,
                isolated: false,
            }
        };

        let prompt = compose_bot_prompt(&bot, &trigger_source, payload.as_deref());
        let record = BotRunRecord {
            id: run_id.clone(),
            bot_id: bot.id.clone(),
            agent_id: None,
            project_id: bot.project_id.clone(),
            trigger_source: trigger_source.clone(),
            prompt: prompt.clone(),
            worktree_path: lease
                .isolated
                .then(|| lease.workdir.to_string_lossy().into_owned()),
            status: BotRunStatus::Running,
            summary: None,
            error: None,
            started_at: now_secs(),
            finished_at: None,
        };
        self.upsert_run(record.clone()).await;

        let title = format!("[Bot] {}", bot.name);
        let agent_id = match self
            .bridge
            .spawn_chat_agent(bot.clone(), lease.workdir.clone(), prompt.clone(), title)
            .await
        {
            Ok(id) => id,
            Err(err) => {
                release_worktree_if_clean(&lease).await;
                let mut failed = record.clone();
                failed.status = BotRunStatus::Failed;
                failed.error = Some(err.to_string());
                failed.finished_at = Some(now_secs());
                self.upsert_run(failed.clone()).await;
                return Err(err);
            }
        };

        let mut started_record = record;
        started_record.agent_id = Some(agent_id.clone());
        self.upsert_run(started_record.clone()).await;

        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            mgr.monitor_run(bot, started_record, agent_id, lease, reply_connector)
                .await;
        });

        Ok(self
            .runs
            .lock()
            .await
            .iter()
            .find(|r| r.id == run_id)
            .cloned()
            .unwrap_or_else(|| unreachable!()))
    }

    async fn monitor_run(
        self: Arc<Self>,
        bot: BotDefinition,
        mut record: BotRunRecord,
        agent_id: AgentId,
        lease: crate::worktree::WorktreeLease,
        reply_connector: Option<ImConnectorConfig>,
    ) {
        let sub_id = muxlane_core::model::new_id("bot-sub");
        let (tx, mut rx) = mpsc::channel::<EventMsg>(128);
        if let Err(err) = self.chat.subscribe(&agent_id, sub_id.clone(), tx).await {
            record.status = BotRunStatus::Failed;
            record.error = Some(err.to_string());
            record.finished_at = Some(now_secs());
            release_worktree_if_clean(&lease).await;
            self.upsert_run(record).await;
            return;
        }

        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_secs(bot.timeout_secs.max(15));
        let mut final_status = BotRunStatus::Succeeded;
        let mut final_error: Option<String> = None;

        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {
                    let _ = self.chat.abort(&agent_id).await;
                    final_status = BotRunStatus::TimedOut;
                    final_error = Some(format!("bot timed out after {}s", bot.timeout_secs));
                    break;
                }
                msg = rx.recv() => {
                    let Some(ev) = msg else {
                        break;
                    };
                    if ev.event != events::CHAT_DELTA {
                        continue;
                    }
                    let Ok(delta) = serde_json::from_value::<ChatDeltaEvent>(ev.params) else {
                        continue;
                    };
                    match delta.delta {
                        ChatDeltaOp::ApprovalRequested { approval } => {
                            if let Some(reply) = decide_bot_approval(bot.permission, &approval) {
                                let _ = self.chat.approve(&agent_id, approval.id, reply).await;
                            } else {
                                record.status = BotRunStatus::NeedsApproval;
                                self.upsert_run(record.clone()).await;
                            }
                        }
                        ChatDeltaOp::ApprovalResolved { .. } => {
                            if record.status == BotRunStatus::NeedsApproval {
                                record.status = BotRunStatus::Running;
                                self.upsert_run(record.clone()).await;
                            }
                        }
                        ChatDeltaOp::TurnFinished { error, .. } => {
                            if let Some(err) = error {
                                final_status = BotRunStatus::Failed;
                                final_error = Some(err);
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        self.chat.unsubscribe(&sub_id).await;

        let summary = self.extract_summary(&agent_id).await;
        let cleaned = release_worktree_if_clean(&lease).await;
        if cleaned {
            record.worktree_path = None;
        }
        record.status = final_status;
        record.error = final_error.clone();
        record.summary = summary.clone();
        record.finished_at = Some(now_secs());
        self.upsert_run(record.clone()).await;

        if let Some(sum_text) = summary.as_deref() {
            if let Some(mut latest_bot) = self.get_bot(&bot.id).await {
                latest_bot.memory_markdown = distill_memory(
                    &latest_bot.memory_markdown,
                    &record.trigger_source,
                    sum_text,
                );
                latest_bot.updated_at = now_secs();
                self.bots
                    .lock()
                    .await
                    .insert(latest_bot.id.clone(), latest_bot.clone());
                if let Ok(payload) = serde_json::to_value(&latest_bot) {
                    self.bridge
                        .emit_bot_event(EventMsg::new(events::BOT_UPDATED, payload));
                }
            }
        }
        self.bridge.persist_bots();

        let outbound_connectors: Vec<ImConnectorConfig> = if let Some(conn) = reply_connector {
            vec![conn]
        } else if bot.notify_on_finish {
            bot.triggers
                .iter()
                .filter_map(|t| match t {
                    BotTrigger::Im { connector, .. } => Some(connector.clone()),
                    _ => None,
                })
                .collect()
        } else {
            Vec::new()
        };

        if !outbound_connectors.is_empty() {
            let text = format!(
                "[Muxlane Bot: {}]\nStatus: {}\n{}",
                bot.name,
                record.status.as_str(),
                record
                    .summary
                    .as_deref()
                    .or(record.error.as_deref())
                    .unwrap_or("(no output)")
            );
            for conn in outbound_connectors {
                if let Err(err) = send_im_reply(&self.http_client, &conn, &text).await {
                    tracing::debug!(bot_id = %bot.id, %err, "failed to send IM reply");
                }
            }
        }
    }

    async fn extract_summary(&self, agent_id: &AgentId) -> Option<String> {
        let hist = self.chat.history(agent_id).await?;
        hist.entries.iter().rev().find_map(|entry| match entry {
            TimelineEntry::AssistantText { text, .. } if !text.trim().is_empty() => {
                Some(text.trim().chars().take(1200).collect())
            }
            _ => None,
        })
    }

    async fn upsert_run(&self, record: BotRunRecord) {
        {
            let mut runs = self.runs.lock().await;
            if let Some(existing) = runs.iter_mut().find(|r| r.id == record.id) {
                *existing = record.clone();
            } else {
                runs.insert(0, record.clone());
                if runs.len() > MAX_RETAINED_RUNS {
                    runs.truncate(MAX_RETAINED_RUNS);
                }
            }
        }
        if let Ok(payload) = serde_json::to_value(&record) {
            self.bridge
                .emit_bot_event(EventMsg::new(events::BOT_RUN_UPDATED, payload));
        }
    }
}

pub fn decide_bot_approval(
    profile: BotPermissionProfile,
    approval: &PendingApproval,
) -> Option<muxlane_core::chat::ApprovalReply> {
    match profile {
        BotPermissionProfile::AskFirst => None,
        BotPermissionProfile::FullAuto => Some(pick_option_or_confirm(approval, true)),
        BotPermissionProfile::ReadOnly => {
            let tool = approval
                .tool_name
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase();
            let title = approval.title.to_ascii_lowercase();
            let safe = (tool.contains("read")
                || tool.contains("search")
                || tool.contains("grep")
                || tool.contains("find")
                || tool.contains("list"))
                && !tool.contains("write")
                && !tool.contains("edit")
                && !tool.contains("bash")
                && !title.contains("exec");
            Some(pick_option_or_confirm(approval, safe))
        }
        BotPermissionProfile::WorkspaceSafe => {
            let tool = approval
                .tool_name
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase();
            let cmd = approval
                .command_or_input
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase();
            let destructive = tool.contains("bash")
                && (cmd.contains("rm -rf")
                    || cmd.contains("git push")
                    || cmd.contains("sudo ")
                    || cmd.contains("curl ")
                    || cmd.contains("mkfs"));
            if destructive {
                None // Escalate dangerous shell commands even in WorkspaceSafe!
            } else {
                Some(pick_option_or_confirm(approval, true))
            }
        }
    }
}

fn pick_option_or_confirm(
    approval: &PendingApproval,
    allow: bool,
) -> muxlane_core::chat::ApprovalReply {
    use muxlane_core::chat::ApprovalReply;

    if approval.options.is_empty() {
        return ApprovalReply::Confirm { confirmed: allow };
    }
    let matched = if allow {
        approval
            .options
            .iter()
            .find(|o| {
                let k = o.kind.as_deref().unwrap_or("").to_ascii_lowercase();
                let l = o.label.to_ascii_lowercase();
                k.contains("allow") || k.contains("once") || l.contains("allow")
            })
            .or_else(|| approval.options.first())
    } else {
        approval
            .options
            .iter()
            .find(|o| {
                let k = o.kind.as_deref().unwrap_or("").to_ascii_lowercase();
                let l = o.label.to_ascii_lowercase();
                k.contains("reject") || k.contains("deny") || l.contains("deny")
            })
            .or_else(|| approval.options.last())
    };
    match matched {
        Some(opt) => ApprovalReply::SelectedOption {
            option_id: opt.option_id.clone(),
        },
        None => ApprovalReply::Confirm { confirmed: allow },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxlane_chat::DriverLaunchSpec;
    use muxlane_core::bot::{BotPermissionProfile, BotTrigger};
    use muxlane_core::chat::{ApprovalSource, ChatDriverKind};
    use muxlane_core::model::AgentType;
    use std::time::Duration;

    struct DummyBridge {
        chat: Arc<ChatManager>,
        project_dir: PathBuf,
        driver_script: PathBuf,
        events: Arc<Mutex<Vec<EventMsg>>>,
    }

    impl BotHostBridge for DummyBridge {
        fn resolve_project_dir(
            &self,
            _project_id: String,
        ) -> Pin<Box<dyn Future<Output = Option<PathBuf>> + Send>> {
            let p = self.project_dir.clone();
            Box::pin(async move { Some(p) })
        }

        fn spawn_chat_agent(
            &self,
            bot: BotDefinition,
            cwd: PathBuf,
            prompt: String,
            _title: String,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<AgentId>> + Send>> {
            let chat = Arc::clone(&self.chat);
            let script = self.driver_script.clone();
            Box::pin(async move {
                let agent = muxlane_core::model::new_id("botagent");
                let spec = DriverLaunchSpec {
                    agent: agent.clone(),
                    agent_type: bot.agent_type,
                    driver_kind: ChatDriverKind::PiRpc,
                    program: "node".into(),
                    args: vec![script.to_string_lossy().into_owned()],
                    cwd,
                    env: vec![],
                    provider_session: None,
                    model: None,
                    thinking_level: None,
                };
                chat.start_session(spec, false, Some(prompt)).await?;
                Ok(agent)
            })
        }

        fn emit_bot_event(&self, event: EventMsg) {
            if let Ok(mut guard) = self.events.try_lock() {
                guard.push(event);
            }
        }

        fn persist_bots(&self) {}
    }

    #[test]
    fn bot_permission_policy_enforces_profiles() {
        let bash_rm = PendingApproval {
            id: "a1".into(),
            agent: "ag".into(),
            source: ApprovalSource::PiExtensionUi,
            method: "confirm".into(),
            title: "Run bash".into(),
            message: None,
            tool_name: Some("bash".into()),
            command_or_input: Some("rm -rf /tmp/x".into()),
            options: vec![],
            prefill: None,
            created_at_ms: 0,
        };
        assert!(decide_bot_approval(BotPermissionProfile::AskFirst, &bash_rm).is_none());
        assert!(decide_bot_approval(BotPermissionProfile::WorkspaceSafe, &bash_rm).is_none());
        assert_eq!(
            decide_bot_approval(BotPermissionProfile::FullAuto, &bash_rm),
            Some(muxlane_core::chat::ApprovalReply::Confirm { confirmed: true })
        );
        assert_eq!(
            decide_bot_approval(BotPermissionProfile::ReadOnly, &bash_rm),
            Some(muxlane_core::chat::ApprovalReply::Confirm { confirmed: false })
        );
    }

    #[tokio::test]
    async fn bot_manager_executes_run_auto_approves_and_distills_memory() {
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("mock_bot_pi.mjs");
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
      send({ id: msg.id, type: "response", command: "get_state", success: true, data: { sessionId: "b1", isStreaming: false } });
    } else if (msg.type === "get_available_models") {
      send({ id: msg.id, type: "response", command: "get_available_models", success: true, data: { models: [] } });
    } else if (msg.type === "get_available_thinking_levels") {
      send({ id: msg.id, type: "response", command: "get_available_thinking_levels", success: true, data: { levels: [] } });
    } else if (msg.type === "prompt") {
      send({ id: msg.id, type: "response", command: "prompt", success: true, data: { disposition: "started" } });
      send({ type: "agent_start" });
      send({ type: "turn_start" });
      send({ type: "extension_ui_request", id: "perm-bot", method: "confirm", title: "Allow check?" });
    } else if (msg.type === "extension_ui_response" && msg.id === "perm-bot") {
      send({ type: "message_start", message: { role: "assistant", content: [] } });
      send({ type: "message_update", assistantMessageEvent: { type: "text_delta", contentIndex: 0, delta: "All checks green." } });
      send({ type: "message_end", message: { role: "assistant", content: [{ type: "text", text: "All checks green." }] } });
      send({ type: "agent_settled", aborted: false });
    }
  }
});
"#,
        )
        .unwrap();

        let (ntx, _nrx) = mpsc::unbounded_channel();
        let chat = ChatManager::new(ntx);
        let bridge = Arc::new(DummyBridge {
            chat: Arc::clone(&chat),
            project_dir: dir.path().to_path_buf(),
            driver_script: script,
            events: Arc::new(Mutex::new(Vec::new())),
        });
        let mgr = BotManager::new(chat, bridge);
        let mut bot = BotDefinition::new("bot_ci".into(), "CI Watcher".into(), "proj_1".into());
        bot.agent_type = AgentType::Pi;
        bot.permission = BotPermissionProfile::FullAuto;
        bot.isolated_worktree = false;
        bot.triggers = vec![BotTrigger::Manual];
        mgr.save_bot(bot).await.unwrap();

        let run = mgr
            .trigger_bot("bot_ci", "manual".into(), Some("check build".into()), None)
            .await
            .unwrap();
        assert_eq!(run.status, BotRunStatus::Running);

        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let runs = mgr.list_runs(Some("bot_ci"), 10).await;
                if runs
                    .first()
                    .is_some_and(|r| r.status == BotRunStatus::Succeeded)
                {
                    assert_eq!(runs[0].summary.as_deref(), Some("All checks green."));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
        })
        .await
        .expect("bot run did not succeed in time");

        let saved_bot = mgr.get_bot("bot_ci").await.unwrap();
        assert!(saved_bot.memory_markdown.contains("All checks green."));
    }
}
