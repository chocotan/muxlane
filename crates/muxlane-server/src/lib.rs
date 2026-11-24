//! muxlane-server：本机 Unix socket 服务端（client 也能连它：对端机器的 muxlane、或本机 hook 脚本）
mod api;
mod foreground;
mod relay;
mod state;
mod subs;
mod supervisor;

pub use api::ProjectAddError;
pub use relay::{validate_relay_credentials, PairOffer, RelayHandle};
pub use state::ServerState;
pub use subs::SubRegistry;

use fs2::FileExt;
use muxlane_core::protocol::{
    methods, read_frame, write_frame, AgentReportParams, EventMsg, PairBeginParams, Request,
    Response, TermSubscribeParams, TermSubscribeResult,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock as StdRwLock};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, Mutex, RwLock};

/// app 每次状态变更后调用 bump() → 所有连接的 events 订阅者收到 state.changed
#[derive(Clone)]
pub struct DirtyFlag(Arc<tokio::sync::watch::Sender<u64>>);

impl DirtyFlag {
    pub fn new() -> Self {
        DirtyFlag(Arc::new(tokio::sync::watch::channel(0u64).0))
    }
    pub fn bump(&self) {
        let cur = *self.0.borrow();
        let _ = self.0.send(cur + 1);
    }
    fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.0.subscribe()
    }
}

impl Default for DirtyFlag {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MuxlaneServer {
    state: Arc<RwLock<ServerState>>,
    runtime: tokio::runtime::Handle,
    /// 会话表独立（PtySession 非 Sync，不能进 RwLock 的共享读）
    sessions: Arc<Mutex<HashMap<muxlane_core::model::AgentId, Arc<muxlane_term::PtySession>>>>,
    chat: Arc<muxlane_chat::ChatManager>,
    bots: Arc<muxlane_bots::BotManager>,
    subs: Arc<Mutex<SubRegistry>>,
    socket_path: PathBuf,
    dirty: DirtyFlag,
    /// 全局状态事件广播（agent.status_changed 等）
    events: tokio::sync::broadcast::Sender<EventMsg>,
    auth: muxlane_core::AuthSecret,
    lifecycle: Arc<Mutex<()>>,
    /// Last foreground process sample; bounds `ps` usage across tick and hook paths.
    foreground_sampled_at: Mutex<Option<std::time::Instant>>,
    persistence_path: StdRwLock<Option<PathBuf>>,
    relay: RelayHandle,
    lan_ws_url: StdRwLock<Option<String>>,
    push_webhook_url: StdRwLock<Option<String>>,
    http_client: reqwest::Client,
}

impl MuxlaneServer {
    pub fn new(
        socket_path: PathBuf,
        state: Arc<RwLock<ServerState>>,
        dirty: DirtyFlag,
    ) -> Arc<Self> {
        Self::new_with_runtime(socket_path, state, dirty, tokio::runtime::Handle::current())
    }

    pub fn new_with_runtime(
        socket_path: PathBuf,
        state: Arc<RwLock<ServerState>>,
        dirty: DirtyFlag,
        runtime: tokio::runtime::Handle,
    ) -> Arc<Self> {
        Self::new_with_runtime_and_auth(
            socket_path,
            state,
            dirty,
            runtime,
            muxlane_core::AuthSecret::generate(),
        )
    }

    pub fn new_with_runtime_and_auth(
        socket_path: PathBuf,
        state: Arc<RwLock<ServerState>>,
        dirty: DirtyFlag,
        runtime: tokio::runtime::Handle,
        auth: muxlane_core::AuthSecret,
    ) -> Arc<Self> {
        // 复用 ServerState 的广播 channel（状态事件单源）
        let events = match state.try_read() {
            Ok(st) => st.events.clone(),
            Err(_) => state.blocking_read().events.clone(),
        };
        let env_webhook = std::env::var("MUXLANE_PUSH_WEBHOOK_URL")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        let (chat_notice_tx, mut chat_notice_rx) = mpsc::unbounded_channel();
        let chat = muxlane_chat::ChatManager::new(chat_notice_tx);
        let runtime_clone = runtime.clone();
        let server = Arc::new_cyclic(|weak_srv| {
            let bridge = Arc::new(ServerBotBridge(weak_srv.clone()));
            let bots = muxlane_bots::BotManager::new(Arc::clone(&chat), bridge);
            MuxlaneServer {
                runtime,
                state,
                sessions: Arc::new(Mutex::new(HashMap::new())),
                chat,
                bots,
                subs: Arc::new(Mutex::new(SubRegistry::default())),
                socket_path,
                dirty,
                events,
                auth,
                lifecycle: Arc::new(Mutex::new(())),
                foreground_sampled_at: Mutex::new(None),
                persistence_path: StdRwLock::new(None),
                relay: RelayHandle::new(),
                lan_ws_url: StdRwLock::new(None),
                push_webhook_url: StdRwLock::new(env_webhook),
                http_client,
            }
        });
        // 转发任务退出时自行摘除条目需要注册表自身的 Weak。
        if let Ok(mut subs) = server.subs.try_lock() {
            subs.bind_self(&server.subs);
        }
        let weak_srv = Arc::downgrade(&server);
        runtime_clone.spawn(async move {
            while let Some(notice) = chat_notice_rx.recv().await {
                let Some(srv) = weak_srv.upgrade() else {
                    break;
                };
                match notice {
                    muxlane_chat::ChatHostNotice::StatusChanged {
                        agent,
                        status,
                        message,
                        reset_seen,
                    } => {
                        let emitted = srv.state.write().await.apply_chat_status(
                            &agent,
                            status,
                            message.clone(),
                            reset_seen,
                        );
                        if !emitted.is_empty() {
                            srv.dirty.bump();
                        }
                        if matches!(
                            status,
                            muxlane_core::model::AgentStatus::Blocked
                                | muxlane_core::model::AgentStatus::Done
                                | muxlane_core::model::AgentStatus::Failed
                        ) {
                            srv.emit_push_webhook(serde_json::json!({
                                "machine_id": srv.machine_id(),
                                "kind": "status_changed",
                                "agent": agent,
                                "severity": status.as_str(),
                                "message": message,
                            }));
                        }
                    }
                    muxlane_chat::ChatHostNotice::TitleChanged { agent, title } => {
                        if srv.state.write().await.apply_chat_title(&agent, title) {
                            srv.dirty.bump();
                            let _ = srv.persist_runtime_state().await;
                        }
                    }
                    muxlane_chat::ChatHostNotice::MetaPersisted { .. } => {
                        let _ = srv.persist_runtime_state().await;
                    }
                    muxlane_chat::ChatHostNotice::ApprovalChanged {
                        agent,
                        approval,
                        resolved_id,
                    } => {
                        let approval = approval.map(|item| *item);
                        let payload = muxlane_core::protocol::ChatApprovalNoticeEvent {
                            agent: agent.clone(),
                            approval: approval.clone(),
                            resolved_id,
                        };
                        if let Ok(val) = serde_json::to_value(&payload) {
                            let _ = srv.events.send(EventMsg::new(
                                muxlane_core::protocol::events::CHAT_APPROVAL,
                                val,
                            ));
                        }
                        if let Some(app) = approval {
                            srv.emit_push_webhook(serde_json::json!({
                                "machine_id": srv.machine_id(),
                                "kind": "approval_requested",
                                "agent": agent,
                                "severity": "blocked",
                                "approval_id": app.id,
                                "title": app.title,
                                "message": app.message,
                                "tool_name": app.tool_name,
                                "command": app.command_or_input,
                            }));
                        }
                    }
                }
            }
        });
        server
    }

    pub fn chat_manager(&self) -> &Arc<muxlane_chat::ChatManager> {
        &self.chat
    }

    pub fn bot_manager(&self) -> &Arc<muxlane_bots::BotManager> {
        &self.bots
    }

    pub async fn restore_bots(&self, persisted: &muxlane_store::PersistedApp) {
        self.bots
            .restore_snapshot(persisted.bots.clone(), persisted.bot_runs.clone())
            .await;
    }

    pub fn start_bot_daemons(self: &Arc<Self>) {
        let mgr = Arc::clone(&self.bots);
        let bind = if std::env::var("MUXLANE_BOT_WEBHOOK").as_deref() == Ok("0") {
            None
        } else {
            Some(
                std::env::var("MUXLANE_BOT_WEBHOOK_BIND")
                    .unwrap_or_else(|_| "127.0.0.1:0".to_string()),
            )
        };
        self.runtime.spawn(async move {
            mgr.start_background_daemons(bind.as_deref()).await;
        });
    }

    pub fn hook_token(&self, agent: &muxlane_core::model::AgentId) -> String {
        self.auth.token(agent, 30 * 24 * 60 * 60)
    }

    /// 从同步上下文往 runtime 投递任务
    pub fn runtime_handle(&self) -> tokio::runtime::Handle {
        self.runtime.clone()
    }

    pub fn rt_spawn<F>(&self, fut: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.runtime.spawn(fut);
    }

    pub fn set_persistence_path(&self, path: PathBuf) {
        if let Some(parent) = path.parent() {
            self.chat
                .set_journal_dir(muxlane_chat::journal::journal_dir_from_state_root(parent));
        }
        if path.exists() {
            if let Ok(persisted) = muxlane_store::load(&path) {
                if !persisted.bots.is_empty() || !persisted.bot_runs.is_empty() {
                    let bots = Arc::clone(&self.bots);
                    self.runtime.spawn(async move {
                        bots.restore_snapshot(persisted.bots, persisted.bot_runs)
                            .await;
                    });
                }
            }
        }
        if let Ok(mut slot) = self.persistence_path.write() {
            *slot = Some(path);
        }
    }

    async fn persist_runtime_state(&self) -> anyhow::Result<()> {
        let snapshot = self.state.read().await.snapshot();
        let bots_snap = self.bots.export_snapshot().await;
        self.persist_snapshot_with_bots(&snapshot, Some(bots_snap))
    }

    fn persist_snapshot(&self, snapshot: &muxlane_core::model::Snapshot) -> anyhow::Result<()> {
        self.persist_snapshot_with_bots(snapshot, None)
    }

    fn persist_snapshot_with_bots(
        &self,
        snapshot: &muxlane_core::model::Snapshot,
        bot_snapshot: Option<(
            Vec<muxlane_core::BotDefinition>,
            Vec<muxlane_core::BotRunRecord>,
        )>,
    ) -> anyhow::Result<()> {
        let path = self
            .persistence_path
            .read()
            .ok()
            .and_then(|path| path.clone());
        let Some(path) = path else { return Ok(()) };
        // 损坏/未来版本的 state.json 必须报错，禁止当空文档再写（会连 secrets.json 一起覆盖）。
        let previous = muxlane_store::load(&path)?;
        let mut persisted =
            muxlane_store::PersistedApp::from_snapshot(snapshot).with_ui_prefs_from(&previous);
        if let Some((bots, runs)) = bot_snapshot {
            persisted.bots = bots;
            persisted.bot_runs = runs;
        }
        let records = self.chat.try_persisted_records();
        persisted.chat_sessions = records
            .into_iter()
            .filter_map(|(agent_id, spec, prov_id, prov_file)| {
                let agent = snapshot.agent(&agent_id)?;
                Some(muxlane_store::PersistedChatSession {
                    agent_id,
                    project_id: agent.project.clone(),
                    agent_type: agent.agent_type,
                    title: agent.title.clone(),
                    title_is_custom: agent.title_is_custom,
                    driver_kind: spec.driver_kind,
                    program: spec.program,
                    args: spec.args,
                    provider_session_id: prov_id,
                    provider_session_file: prov_file,
                })
            })
            .collect();
        muxlane_store::save(&path, &persisted)
    }

    /// 启动监听（阻塞当前 async 上下文）
    pub async fn serve(self: Arc<Self>) -> anyhow::Result<()> {
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // 单实例锁：只有持锁者才能清理 stale socket，禁止第二进程 unlink 活跃 listener。
        let lock_path = self.socket_path.with_extension("lock");
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        lock_file.try_lock_exclusive().map_err(|_| {
            anyhow::anyhow!("another muxlane instance owns {}", lock_path.display())
        })?;
        let _instance_lock = lock_file;
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
        let listener = UnixListener::bind(&self.socket_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.socket_path, std::fs::Permissions::from_mode(0o600))?;
        }
        tracing::info!(path = %self.socket_path.display(), "muxlane server listening");

        loop {
            let (stream, _addr) = listener.accept().await?;
            let server = Arc::clone(&self);
            tokio::spawn(async move {
                if let Err(e) = server.handle_conn(stream).await {
                    tracing::debug!(error = %e, "connection closed");
                }
            });
        }
    }

    async fn handle_system_hello(&self, req: Request) -> anyhow::Result<Response> {
        Ok(Response::ok(
            req.id,
            serde_json::to_value(muxlane_core::protocol::HelloResult {
                version: env!("CARGO_PKG_VERSION").into(),
                protocol: muxlane_core::protocol::PROTOCOL_VERSION,
                features: vec![
                    muxlane_core::protocol::features::PROJECT_ADD.into(),
                    muxlane_core::protocol::features::PROJECT_CREATE.into(),
                    muxlane_core::protocol::features::AGENT_SPAWN.into(),
                    muxlane_core::protocol::features::AGENT_RENAME.into(),
                    muxlane_core::protocol::features::TERM_INPUT.into(),
                    muxlane_core::protocol::features::TERM_RESIZE.into(),
                    muxlane_core::protocol::features::TERM_REPLAY_CHUNKS.into(),
                    muxlane_core::protocol::features::TERM_DIFF.into(),
                    muxlane_core::protocol::features::AGENT_MARK_SEEN.into(),
                    muxlane_core::protocol::features::PAIR.into(),
                    muxlane_core::protocol::features::PRESET_LIST.into(),
                    muxlane_core::protocol::features::CHAT_TIMELINE.into(),
                    muxlane_core::protocol::features::CHAT_SEND.into(),
                    muxlane_core::protocol::features::CHAT_APPROVAL.into(),
                    muxlane_core::protocol::features::E2EE.into(),
                    muxlane_core::protocol::features::BOTS.into(),
                ],
                lan_ws_url: self.lan_ws_url(),
            })?,
        ))
    }

    async fn handle_state_list(&self, req: Request) -> anyhow::Result<Response> {
        Ok(Response::ok(
            req.id,
            serde_json::to_value(self.snapshot().await)?,
        ))
    }

    async fn handle_events_subscribe(
        &self,
        req: Request,
        status_rx: &mut Option<tokio::sync::broadcast::Receiver<EventMsg>>,
        ev_tx: &mpsc::Sender<EventMsg>,
    ) -> anyhow::Result<Response> {
        *status_rx = Some(self.subscribe_events());
        let _ = ev_tx
            .send(EventMsg::new(
                muxlane_core::protocol::events::STATE_CHANGED,
                serde_json::json!({}),
            ))
            .await;
        Ok(Response::ok(req.id, serde_json::json!({"ok": true})))
    }

    async fn handle_term_subscribe(
        &self,
        req: Request,
        ev_tx: &mpsc::Sender<EventMsg>,
        connection_subs: &mut Vec<String>,
    ) -> anyhow::Result<Response> {
        let params = match serde_json::from_value::<TermSubscribeParams>(req.params) {
            Ok(params) => params,
            Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
        };
        let prepared = {
            let session = self.sessions.lock().await.get(&params.agent).cloned();
            session.map(|session| {
                let (_snapshot, rx) = session.subscribe();
                let snapshot = session.resync_snapshot();
                (muxlane_core::model::new_id("sub"), snapshot, rx, session)
            })
        };
        let Some((sub_id, snapshot, rx, session)) = prepared else {
            return Ok(Response::err(
                req.id,
                "no_such_agent",
                format!("agent {} not running", params.agent),
            ));
        };
        self.subs.lock().await.add(
            &sub_id,
            &params.agent,
            ev_tx.clone(),
            rx,
            session,
            snapshot.clone(),
            params.accept_replay_chunks,
            params.accept_replay_gzip,
            params.accept_data_gzip,
            params.accept_data_stream,
            params.accept_term_diff,
        );
        connection_subs.push(sub_id.clone());
        let replay_payload = if params.accept_replay_gzip {
            muxlane_core::protocol::gzip_encode(&snapshot)
        } else {
            snapshot.to_vec()
        };
        Ok(Response::ok(
            req.id,
            serde_json::to_value(TermSubscribeResult {
                sub_id,
                replay_b64: if params.accept_replay_chunks {
                    String::new()
                } else {
                    muxlane_core::protocol::b64_encode(&replay_payload)
                },
            })?,
        ))
    }

    async fn handle_term_unsubscribe(
        &self,
        req: Request,
        connection_subs: &mut Vec<String>,
    ) -> anyhow::Result<Response> {
        if let Some(sub_id) = req.params["sub_id"].as_str() {
            self.subs.lock().await.remove(sub_id);
            connection_subs.retain(|id| id != sub_id);
        }
        Ok(Response::ok(req.id, serde_json::json!({"ok": true})))
    }

    async fn handle_term_input(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::TermInputParams>(req.params) {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        let Some(session) = self.sessions.lock().await.get(&params.agent).cloned() else {
            return Ok(Response::err(req.id, "no_such_agent", params.agent));
        };
        let data = muxlane_core::protocol::b64_decode(&params.data_b64)?;
        session.write_input(&data);
        if data.iter().any(|byte| matches!(byte, b'\r' | b'\n')) {
            self.mark_working(&params.agent).await;
        }
        Ok(Response::ok(req.id, serde_json::json!({"ok": true})))
    }

    async fn handle_term_resize(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::TermResizeParams>(req.params) {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        let Some(session) = self.sessions.lock().await.get(&params.agent).cloned() else {
            return Ok(Response::err(req.id, "no_such_agent", params.agent));
        };
        // 远程 RPC 协议目前不携带真实像素尺寸；给个合理估值总比 0x0 强，
        // 否则 kitten icat 这类客户端会因为拿不到像素尺寸直接拒绝发图。
        session.resize(params.cols, params.rows, params.cols * 8, params.rows * 17)?;
        Ok(Response::ok(req.id, serde_json::json!({"ok": true})))
    }

    async fn handle_agent_spawn(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::AgentSpawnParams>(req.params) {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        match self.spawn_agent(params).await {
            Ok(instance) => Ok(Response::ok(req.id, serde_json::to_value(instance)?)),
            Err(error) => {
                let message = error.to_string();
                let code = if error.downcast_ref::<crate::api::NoSuchProject>().is_some() {
                    "no_such_project"
                } else {
                    "spawn_failed"
                };
                Ok(Response::err(req.id, code, message))
            }
        }
    }

    async fn handle_agent_delete(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::AgentDeleteParams>(req.params) {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        match self.delete_agent(&params.agent).await {
            Ok(result) if result.failed_agents.is_empty() => {
                Ok(Response::ok(req.id, serde_json::json!({"ok": true})))
            }
            Ok(result) => Ok(Response::err(
                req.id,
                "delete_failed",
                format!(
                    "failed to destroy agent {}",
                    result.failed_agents.join(", ")
                ),
            )),
            Err(error) => Ok(Response::err(
                req.id,
                muxlane_core::protocol::error_codes::PERSISTENCE_FAILED,
                error.to_string(),
            )),
        }
    }

    async fn handle_project_add(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::ProjectAddParams>(req.params) {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        match self.add_project(params).await {
            Ok(project) => Ok(Response::ok(req.id, serde_json::to_value(project)?)),
            Err(error) => {
                let code = error
                    .downcast_ref::<ProjectAddError>()
                    .map(ProjectAddError::code)
                    .unwrap_or(muxlane_core::protocol::error_codes::PERSISTENCE_FAILED);
                Ok(Response::err(req.id, code, error.to_string()))
            }
        }
    }

    async fn handle_project_delete(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::ProjectDeleteParams>(req.params)
            {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        match self.delete_project(&params.project).await {
            Ok(result) => Ok(Response::ok(req.id, serde_json::to_value(result)?)),
            Err(error) => Ok(Response::err(
                req.id,
                muxlane_core::protocol::error_codes::PERSISTENCE_FAILED,
                error.to_string(),
            )),
        }
    }

    async fn handle_agent_report(&self, req: Request) -> anyhow::Result<Response> {
        let params = match serde_json::from_value::<AgentReportParams>(req.params) {
            Ok(params) => params,
            Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
        };
        if !self.auth.verify(&params.agent, &params.token) {
            tracing::warn!(
                agent = %params.agent,
                reason = "invalid or expired token",
                "hook authentication rejected"
            );
            return Ok(Response::err(
                req.id,
                "unauthorized",
                "invalid or expired hook token",
            ));
        }
        self.refresh_shell_identity_for_hook(&params.agent).await;
        let _ = self.state.write().await.report_hook(&params).await;
        self.dirty.bump();
        Ok(Response::ok(req.id, serde_json::json!({"ok": true})))
    }

    async fn handle_agent_rename(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::AgentRenameParams>(req.params) {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        match self.rename_agent(&params.agent, params.title).await {
            Ok(agent) => Ok(Response::ok(req.id, serde_json::to_value(agent)?)),
            Err(error) => {
                let code = error
                    .downcast_ref::<api::AgentRenameError>()
                    .map(api::AgentRenameError::code)
                    .unwrap_or(muxlane_core::protocol::error_codes::PERSISTENCE_FAILED);
                Ok(Response::err(req.id, code, error.to_string()))
            }
        }
    }

    async fn handle_agent_mark_seen(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::AgentMarkSeenParams>(req.params)
            {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        self.mark_seen(&params.agent).await;
        Ok(Response::ok(req.id, serde_json::json!({"ok": true})))
    }

    async fn handle_unknown_method(
        &self,
        request_id: u64,
        method: &str,
    ) -> anyhow::Result<Response> {
        Ok(Response::method_not_found(request_id, method))
    }

    async fn handle_pair_begin(&self, req: Request) -> anyhow::Result<(Response, Option<String>)> {
        let params = match serde_json::from_value::<PairBeginParams>(req.params) {
            Ok(params) => params,
            Err(error) => {
                return Ok((Response::err(req.id, "bad_params", error.to_string()), None))
            }
        };
        let device = params
            .device
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("phone");
        if !device
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        {
            return Ok((
                Response::err(req.id, "bad_params", "invalid device id"),
                None,
            ));
        }
        tracing::debug!(
            device,
            has_host = params.host_id.is_some(),
            has_token = params.token.is_some(),
            "pair.begin"
        );
        let issued_subject = if let Some(token) = params
            .token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            match self.auth.verify_mobile(token) {
                Some(existing) => existing,
                None => {
                    return Ok((
                        Response::err(req.id, "unauthorized", "invalid or expired pairing"),
                        None,
                    ))
                }
            }
        } else if let Some(host_id) = params
            .host_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if host_id != self.machine_id() {
                return Ok((
                    Response::err(req.id, "unauthorized", "unknown machine"),
                    None,
                ));
            }
            format!("mobile:{device}")
        } else {
            return Ok((
                Response::err(req.id, "unauthorized", "invalid or expired pairing"),
                None,
            ));
        };
        let token = self.auth.token(&issued_subject, 180 * 24 * 60 * 60);
        tracing::debug!(%issued_subject, "pair.begin authorized");
        let machine = self.state.read().await.machine.clone();
        Ok((
            Response::ok(
                req.id,
                serde_json::to_value(muxlane_core::protocol::PairBeginResult {
                    token: token.clone(),
                    machine,
                    lan_ws_url: self.lan_ws_url(),
                })?,
            ),
            Some(token),
        ))
    }

    fn handle_e2ee_upgrade(
        &self,
        req: Request,
        session_token: Option<&str>,
    ) -> anyhow::Result<(Response, Option<muxlane_core::e2ee::E2eeSession>)> {
        let Some(token) = session_token else {
            return Ok((
                Response::err(
                    req.id,
                    "unauthorized",
                    "pair.begin required before e2ee.upgrade",
                ),
                None,
            ));
        };
        let params =
            match serde_json::from_value::<muxlane_core::protocol::E2eeUpgradeParams>(req.params) {
                Ok(p) => p,
                Err(err) => {
                    return Ok((Response::err(req.id, "bad_params", err.to_string()), None))
                }
            };
        let client_pub = match muxlane_core::protocol::b64_decode(&params.client_pubkey_b64) {
            Ok(b) => b,
            Err(err) => {
                return Ok((
                    Response::err(req.id, "bad_params", format!("bad client pubkey: {err}")),
                    None,
                ))
            }
        };
        let client_nonce = match muxlane_core::protocol::b64_decode(&params.client_nonce_b64) {
            Ok(b) => b,
            Err(err) => {
                return Ok((
                    Response::err(req.id, "bad_params", format!("bad client nonce: {err}")),
                    None,
                ))
            }
        };
        let server_kp = muxlane_core::e2ee::E2eeKeyPair::generate();
        let server_pub_b64 = muxlane_core::protocol::b64_encode(server_kp.public_bytes());
        let server_nonce = muxlane_core::e2ee::random_nonce16();
        let server_nonce_b64 = muxlane_core::protocol::b64_encode(&server_nonce);
        let session = match muxlane_core::e2ee::E2eeSession::derive(
            muxlane_core::e2ee::E2eeRole::Server,
            server_kp,
            &client_pub,
            token,
            &client_nonce,
            &server_nonce,
        ) {
            Ok(s) => s,
            Err(err) => return Ok((Response::err(req.id, "e2ee_failed", err.to_string()), None)),
        };
        Ok((
            Response::ok(
                req.id,
                serde_json::to_value(muxlane_core::protocol::E2eeUpgradeResult {
                    server_pubkey_b64: server_pub_b64,
                    server_nonce_b64,
                    cipher: "aes-256-gcm".into(),
                })?,
            ),
            Some(session),
        ))
    }

    async fn handle_preset_list(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::PresetListParams>(req.params) {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        match self.list_presets(&params.project).await {
            Ok(presets) => Ok(Response::ok(req.id, serde_json::to_value(presets)?)),
            Err(error) => Ok(Response::err(req.id, "no_such_project", error.to_string())),
        }
    }

    async fn handle_chat_spawn(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::ChatSpawnParams>(req.params) {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        match self.spawn_chat(params).await {
            Ok(instance) => Ok(Response::ok(req.id, serde_json::to_value(instance)?)),
            Err(error) => {
                let code = if error.downcast_ref::<api::NoSuchProject>().is_some() {
                    "no_such_project"
                } else {
                    "spawn_failed"
                };
                Ok(Response::err(req.id, code, error.to_string()))
            }
        }
    }

    async fn handle_chat_list(&self, req: Request) -> anyhow::Result<Response> {
        let sessions = self.chat.list_summaries().await;
        Ok(Response::ok(
            req.id,
            serde_json::to_value(muxlane_core::protocol::ChatListResult { sessions })?,
        ))
    }

    async fn handle_chat_history(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::ChatHistoryParams>(req.params) {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        match self.chat_history(&params.agent).await {
            Ok(history) => Ok(Response::ok(req.id, serde_json::to_value(history)?)),
            Err(error) => Ok(Response::err(req.id, "no_such_agent", error.to_string())),
        }
    }

    async fn handle_chat_subscribe(
        &self,
        req: Request,
        ev_tx: &mpsc::Sender<EventMsg>,
        connection_chat_subs: &mut Vec<String>,
    ) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::ChatSubscribeParams>(req.params)
            {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        let sub_id = muxlane_core::model::new_id("chat-sub");
        match self
            .chat
            .subscribe(&params.agent, sub_id.clone(), ev_tx.clone())
            .await
        {
            Ok(history) => {
                connection_chat_subs.push(sub_id.clone());
                Ok(Response::ok(
                    req.id,
                    serde_json::to_value(muxlane_core::protocol::ChatSubscribeResult {
                        sub_id,
                        history,
                    })?,
                ))
            }
            Err(error) => Ok(Response::err(req.id, "no_such_agent", error.to_string())),
        }
    }

    async fn handle_chat_unsubscribe(
        &self,
        req: Request,
        connection_chat_subs: &mut Vec<String>,
    ) -> anyhow::Result<Response> {
        let params = match serde_json::from_value::<muxlane_core::protocol::ChatUnsubscribeParams>(
            req.params,
        ) {
            Ok(params) => params,
            Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
        };
        self.chat.unsubscribe(&params.sub_id).await;
        connection_chat_subs.retain(|id| id != &params.sub_id);
        Ok(Response::ok(req.id, serde_json::json!({"ok": true})))
    }

    async fn handle_chat_send(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::ChatSendParams>(req.params) {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        match self
            .chat
            .send_prompt(
                &params.agent,
                params.text,
                params.attachments,
                params.streaming_behavior,
            )
            .await
        {
            Ok(()) => Ok(Response::ok(req.id, serde_json::json!({"ok": true}))),
            Err(error) => Ok(Response::err(req.id, "chat_send_failed", error.to_string())),
        }
    }

    async fn handle_chat_abort(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::ChatAbortParams>(req.params) {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        match self.chat.abort(&params.agent).await {
            Ok(()) => Ok(Response::ok(req.id, serde_json::json!({"ok": true}))),
            Err(error) => Ok(Response::err(
                req.id,
                "chat_abort_failed",
                error.to_string(),
            )),
        }
    }

    async fn handle_chat_approve(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::ChatApproveParams>(req.params) {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        match self
            .chat
            .approve(&params.agent, params.approval_id, params.reply)
            .await
        {
            Ok(()) => Ok(Response::ok(req.id, serde_json::json!({"ok": true}))),
            Err(error) => Ok(Response::err(
                req.id,
                "chat_approve_failed",
                error.to_string(),
            )),
        }
    }

    async fn handle_chat_configure(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::ChatConfigureParams>(req.params)
            {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        match self
            .chat
            .configure(
                &params.agent,
                params.provider,
                params.model_id,
                params.thinking_level,
            )
            .await
        {
            Ok(()) => Ok(Response::ok(req.id, serde_json::json!({"ok": true}))),
            Err(error) => Ok(Response::err(
                req.id,
                "chat_configure_failed",
                error.to_string(),
            )),
        }
    }

    async fn handle_chat_switch_mode(&self, req: Request) -> anyhow::Result<Response> {
        let params = match serde_json::from_value::<muxlane_core::protocol::ChatSwitchModeParams>(
            req.params,
        ) {
            Ok(params) => params,
            Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
        };
        match self.switch_chat_mode(&params.agent, params.target).await {
            Ok(meta) => Ok(Response::ok(req.id, serde_json::to_value(meta)?)),
            Err(error) => Ok(Response::err(
                req.id,
                "chat_switch_failed",
                error.to_string(),
            )),
        }
    }

    async fn handle_bot_list(&self, req: Request) -> anyhow::Result<Response> {
        let bots = self.bots.list_bots().await;
        Ok(Response::ok(
            req.id,
            serde_json::to_value(muxlane_core::protocol::BotListResult { bots })?,
        ))
    }

    async fn handle_bot_save(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::BotSaveParams>(req.params) {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        match self.bots.save_bot(params.bot).await {
            Ok(bot) => Ok(Response::ok(req.id, serde_json::to_value(bot)?)),
            Err(error) => Ok(Response::err(req.id, "bot_save_failed", error.to_string())),
        }
    }

    async fn handle_bot_delete(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::BotDeleteParams>(req.params) {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        let removed = self.bots.delete_bot(&params.bot_id).await;
        Ok(Response::ok(req.id, serde_json::json!({ "ok": removed })))
    }

    async fn handle_bot_trigger(&self, req: Request) -> anyhow::Result<Response> {
        let params =
            match serde_json::from_value::<muxlane_core::protocol::BotTriggerParams>(req.params) {
                Ok(params) => params,
                Err(error) => return Ok(Response::err(req.id, "bad_params", error.to_string())),
            };
        let source = params
            .source
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "manual".into());
        match self
            .bots
            .trigger_bot(&params.bot_id, source, params.payload, None)
            .await
        {
            Ok(run) => Ok(Response::ok(req.id, serde_json::to_value(run)?)),
            Err(error) => Ok(Response::err(
                req.id,
                "bot_trigger_failed",
                error.to_string(),
            )),
        }
    }

    async fn handle_bot_runs(&self, req: Request) -> anyhow::Result<Response> {
        let params = serde_json::from_value::<muxlane_core::protocol::BotRunsParams>(req.params)
            .unwrap_or(muxlane_core::protocol::BotRunsParams {
                bot_id: None,
                limit: None,
            });
        let runs = self
            .bots
            .list_runs(params.bot_id.as_deref(), params.limit.unwrap_or(40))
            .await;
        Ok(Response::ok(
            req.id,
            serde_json::to_value(muxlane_core::protocol::BotRunsResult { runs })?,
        ))
    }

    pub fn relay_handle(&self) -> RelayHandle {
        self.relay.clone()
    }

    pub(crate) fn relay(&self) -> &RelayHandle {
        &self.relay
    }

    pub fn machine_id(&self) -> String {
        match self.state.try_read() {
            Ok(state) => state.machine.machine_id.clone(),
            Err(_) => self.state.blocking_read().machine.machine_id.clone(),
        }
    }

    pub fn lan_ws_url(&self) -> Option<String> {
        self.lan_ws_url.read().ok().and_then(|guard| guard.clone())
    }

    pub fn set_push_webhook_url(&self, url: Option<String>) {
        let cleaned = url
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Ok(mut guard) = self.push_webhook_url.write() {
            *guard = cleaned;
        }
    }

    fn emit_push_webhook(&self, payload: serde_json::Value) {
        let Some(url) = self
            .push_webhook_url
            .read()
            .ok()
            .and_then(|guard| guard.clone())
        else {
            return;
        };
        let client = self.http_client.clone();
        self.runtime.spawn(async move {
            if let Err(err) = client.post(&url).json(&payload).send().await {
                tracing::debug!(%url, %err, "push webhook delivery failed");
            }
        });
    }

    pub fn start_lan_ws(self: &Arc<Self>) {
        if self.lan_ws_url().is_some() {
            return;
        }
        let port: u16 = std::env::var("MUXLANE_LAN_WS_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let server = Arc::clone(self);
        self.runtime.spawn(async move {
            if let Err(err) = server
                .start_lan_ws_on(&format!("0.0.0.0:{port}"), false)
                .await
            {
                tracing::debug!(%err, "lan ws listener stopped");
            }
        });
    }

    pub async fn start_lan_ws_on(
        self: &Arc<Self>,
        bind_addr: &str,
        allow_loopback: bool,
    ) -> anyhow::Result<Option<String>> {
        let listener = tokio::net::TcpListener::bind(bind_addr).await?;
        let local_addr = listener.local_addr()?;
        let local_port = local_addr.port();
        let advertised_ip = if local_addr.ip().is_loopback() {
            allow_loopback.then_some(local_addr.ip())
        } else {
            crate::relay::detect_lan_ipv4().or_else(|| {
                allow_loopback.then_some(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
            })
        };
        let url = advertised_ip.map(|ip| format!("ws://{ip}:{local_port}"));
        if let Ok(mut guard) = self.lan_ws_url.write() {
            *guard = url.clone();
        }
        let server = Arc::clone(self);
        self.runtime.spawn(async move {
            crate::relay::accept_lan_ws(server, listener).await;
        });
        Ok(url)
    }

    pub fn start_relay(self: &Arc<Self>, url: String, token: Option<String>) {
        let url = url.trim().trim_end_matches('/').to_string();
        self.relay
            .configure((!url.is_empty()).then_some(url), token);
        let should_start = self.relay.mark_started();
        if should_start {
            let server = Arc::clone(self);
            self.runtime.spawn(async move {
                if let Err(error) = crate::relay::run(server).await {
                    tracing::warn!(%error, "relay client stopped");
                }
            });
        }
    }

    pub async fn begin_pair_offer(self: &Arc<Self>) -> anyhow::Result<PairOffer> {
        self.relay.offer_code().await
    }

    async fn handle_conn(self: Arc<Self>, stream: UnixStream) -> anyhow::Result<()> {
        self.handle_rpc(stream, false).await
    }

    pub(crate) async fn handle_rpc<S>(
        self: Arc<Self>,
        stream: S,
        require_pair: bool,
    ) -> anyhow::Result<()>
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (read_half, mut write_half) = tokio::io::split(stream);
        let (frame_tx, mut frame_rx) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(read_half);
            loop {
                let frame = read_frame(&mut reader).await;
                let eof = matches!(frame, Err(muxlane_core::Error::Eof));
                if frame_tx.send(frame).await.is_err() || eof {
                    break;
                }
            }
        });

        let (ev_tx, mut ev_rx) = mpsc::channel::<EventMsg>(256);
        let mut status_rx: Option<tokio::sync::broadcast::Receiver<EventMsg>> = None;
        let mut dirty_rx = self.dirty.subscribe();
        let mut dirty_seen = *dirty_rx.borrow_and_update();
        let mut connection_subs = Vec::new();
        let mut connection_chat_subs = Vec::new();
        let mut paired = !require_pair;
        let mut session_token: Option<String> = None;
        let mut e2ee: Option<muxlane_core::e2ee::E2eeSession> = None;

        loop {
            tokio::select! {
                frame = frame_rx.recv() => match frame {
                    Some(Ok(raw_value)) => {
                        let value = if let Some(session) = e2ee.as_mut() {
                            let envelope: muxlane_core::e2ee::E2eeEnvelope = match serde_json::from_value(raw_value) {
                                Ok(env) => env,
                                Err(_) => {
                                    send_rpc_frame(
                                        &mut write_half,
                                        Some(session),
                                        &Response::err(0, "e2ee_required", "encrypted frame required"),
                                    )
                                    .await?;
                                    break;
                                }
                            };
                            match session.open(&envelope) {
                                Ok(pt) => match serde_json::from_slice(&pt) {
                                    Ok(v) => v,
                                    Err(error) => {
                                        send_rpc_frame(
                                            &mut write_half,
                                            Some(session),
                                            &Response::err(0, "bad_request", error.to_string()),
                                        )
                                        .await?;
                                        continue;
                                    }
                                },
                                Err(error) => {
                                    send_rpc_frame(
                                        &mut write_half,
                                        Some(session),
                                        &Response::err(0, "e2ee_decrypt_failed", error.to_string()),
                                    )
                                    .await?;
                                    break;
                                }
                            }
                        } else {
                            raw_value
                        };

                        let req: Request = match serde_json::from_value(value) {
                            Ok(req) => req,
                            Err(error) => {
                                send_rpc_frame(
                                    &mut write_half,
                                    e2ee.as_mut(),
                                    &Response::err(0, "bad_request", error.to_string()),
                                )
                                .await?;
                                continue;
                            }
                        };
                        if require_pair && !paired && req.method != methods::PAIR_BEGIN {
                            send_rpc_frame(
                                &mut write_half,
                                e2ee.as_mut(),
                                &Response::err(req.id, "unauthorized", "pair.begin required"),
                            )
                            .await?;
                            break;
                        }
                        let method = req.method.clone();
                        let mut pending_e2ee: Option<muxlane_core::e2ee::E2eeSession> = None;
                        let response = match method.as_str() {
                            methods::SYSTEM_HELLO => self.handle_system_hello(req).await?,
                            methods::STATE_LIST => self.handle_state_list(req).await?,
                            methods::EVENTS_SUBSCRIBE => self.handle_events_subscribe(req, &mut status_rx, &ev_tx).await?,
                            methods::TERM_SUBSCRIBE => self.handle_term_subscribe(req, &ev_tx, &mut connection_subs).await?,
                            methods::TERM_UNSUBSCRIBE => self.handle_term_unsubscribe(req, &mut connection_subs).await?,
                            methods::TERM_INPUT => self.handle_term_input(req).await?,
                            methods::TERM_RESIZE => self.handle_term_resize(req).await?,
                            methods::AGENT_SPAWN => self.handle_agent_spawn(req).await?,
                            methods::AGENT_DELETE => self.handle_agent_delete(req).await?,
                            methods::AGENT_RENAME => self.handle_agent_rename(req).await?,
                            methods::AGENT_MARK_SEEN => self.handle_agent_mark_seen(req).await?,
                            methods::PROJECT_ADD => self.handle_project_add(req).await?,
                            methods::PROJECT_DELETE => self.handle_project_delete(req).await?,
                            methods::AGENT_REPORT => self.handle_agent_report(req).await?,
                            methods::PAIR_BEGIN => {
                                let (resp, tok) = self.handle_pair_begin(req).await?;
                                if let Some(tok) = tok {
                                    session_token = Some(tok);
                                }
                                resp
                            }
                            methods::E2EE_UPGRADE => {
                                if e2ee.is_some() {
                                    Response::err(req.id, "already_e2ee", "e2ee session already active")
                                } else {
                                    let (resp, next_e2ee) =
                                        self.handle_e2ee_upgrade(req, session_token.as_deref())?;
                                    pending_e2ee = next_e2ee;
                                    resp
                                }
                            }
                            methods::PRESET_LIST => self.handle_preset_list(req).await?,
                            methods::CHAT_SPAWN => self.handle_chat_spawn(req).await?,
                            methods::CHAT_LIST => self.handle_chat_list(req).await?,
                            methods::CHAT_HISTORY => self.handle_chat_history(req).await?,
                            methods::CHAT_SUBSCRIBE => self.handle_chat_subscribe(req, &ev_tx, &mut connection_chat_subs).await?,
                            methods::CHAT_UNSUBSCRIBE => self.handle_chat_unsubscribe(req, &mut connection_chat_subs).await?,
                            methods::CHAT_SEND => self.handle_chat_send(req).await?,
                            methods::CHAT_ABORT => self.handle_chat_abort(req).await?,
                            methods::CHAT_APPROVE => self.handle_chat_approve(req).await?,
                            methods::CHAT_CONFIGURE => self.handle_chat_configure(req).await?,
                            methods::CHAT_SWITCH_MODE => self.handle_chat_switch_mode(req).await?,
                            methods::BOT_LIST => self.handle_bot_list(req).await?,
                            methods::BOT_SAVE => self.handle_bot_save(req).await?,
                            methods::BOT_DELETE => self.handle_bot_delete(req).await?,
                            methods::BOT_TRIGGER => self.handle_bot_trigger(req).await?,
                            methods::BOT_RUNS => self.handle_bot_runs(req).await?,
                            other => self.handle_unknown_method(req.id, other).await?,
                        };
                        let pair_failed = require_pair
                            && method == methods::PAIR_BEGIN
                            && response.error.is_some();
                        if method == methods::PAIR_BEGIN && response.error.is_none() {
                            paired = true;
                        }
                        send_rpc_frame(&mut write_half, e2ee.as_mut(), &response).await?;
                        if let Some(next_e2ee) = pending_e2ee {
                            e2ee = Some(next_e2ee);
                        }
                        if pair_failed {
                            break;
                        }
                    }
                    Some(Err(muxlane_core::Error::Eof)) | None => break,
                    Some(Err(error)) => return Err(error.into()),
                },
                event = ev_rx.recv() => match event {
                    Some(message) => send_rpc_frame(&mut write_half, e2ee.as_mut(), &message).await?,
                    None => break,
                },
                status = async {
                    match status_rx.as_mut() {
                        Some(receiver) => receiver.recv().await,
                        None => std::future::pending().await,
                    }
                } => match status {
                    Ok(message) => send_rpc_frame(&mut write_half, e2ee.as_mut(), &message).await?,
                    // 漏了边沿通知：补一次全量同步信号，让对端重拉快照。
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let _ = ev_tx.try_send(EventMsg::new(
                            muxlane_core::protocol::events::STATE_CHANGED,
                            serde_json::json!({}),
                        ));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                Ok(()) = dirty_rx.changed() => {
                    let current = *dirty_rx.borrow_and_update();
                    if current != dirty_seen {
                        dirty_seen = current;
                        let _ = ev_tx.try_send(EventMsg::new(
                            muxlane_core::protocol::events::STATE_CHANGED,
                            serde_json::json!({}),
                        ));
                    }
                }
                else => break,
            }
        }
        if !connection_subs.is_empty() {
            let mut subs = self.subs.lock().await;
            for id in connection_subs {
                subs.remove(&id);
            }
        }
        for id in connection_chat_subs {
            self.chat.unsubscribe(&id).await;
        }
        Ok(())
    }
}

struct ServerBotBridge(std::sync::Weak<MuxlaneServer>);

impl muxlane_bots::BotHostBridge for ServerBotBridge {
    fn resolve_project_dir(
        &self,
        project_id: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<PathBuf>> + Send>> {
        let weak = self.0.clone();
        Box::pin(async move {
            let srv = weak.upgrade()?;
            let guard = srv.state.read().await;
            guard
                .projects
                .iter()
                .find(|p| p.id == project_id)
                .map(|p| p.path.clone())
        })
    }

    fn spawn_chat_agent(
        &self,
        bot: muxlane_core::BotDefinition,
        cwd: PathBuf,
        prompt: String,
        title: String,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = anyhow::Result<muxlane_core::model::AgentId>> + Send>,
    > {
        let weak = self.0.clone();
        Box::pin(async move {
            let srv = weak
                .upgrade()
                .ok_or_else(|| anyhow::anyhow!("server shut down"))?;
            let params = muxlane_core::protocol::ChatSpawnParams {
                project: bot.project_id,
                agent_type: Some(bot.agent_type),
                program: bot.program,
                args: (!bot.args.is_empty()).then_some(bot.args),
                preset_name: Some(title),
                driver: Some(bot.driver),
                provider_session: None,
                initial_prompt: Some(prompt),
                model: bot.model,
                thinking_level: bot.thinking_level,
                env: None,
            };
            let inst = srv.spawn_chat_in_cwd(params, Some(cwd)).await?;
            Ok(inst.id)
        })
    }

    fn emit_bot_event(&self, event: EventMsg) {
        if let Some(srv) = self.0.upgrade() {
            let _ = srv.events.send(event);
        }
    }

    fn persist_bots(&self) {
        if let Some(srv) = self.0.upgrade() {
            let srv_clone = Arc::clone(&srv);
            srv.runtime.spawn(async move {
                let _ = srv_clone.persist_runtime_state().await;
            });
        }
    }
}

async fn send_rpc_frame<W, T>(
    writer: &mut W,
    e2ee: Option<&mut muxlane_core::e2ee::E2eeSession>,
    value: &T,
) -> anyhow::Result<()>
where
    W: AsyncWrite + Unpin,
    T: serde::Serialize,
{
    if let Some(session) = e2ee {
        let envelope = session.seal_json(value)?;
        write_frame(writer, &envelope).await?;
    } else {
        write_frame(writer, value).await?;
    }
    Ok(())
}
