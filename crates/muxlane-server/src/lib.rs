//! muxlane-server：本机 Unix socket 服务端（client 也能连它：对端机器的 muxlane、或本机 hook 脚本）
mod api;
mod state;
mod subs;
mod supervisor;

pub use api::ProjectAddError;
pub use state::ServerState;
pub use subs::SubRegistry;

use fs2::FileExt;
use muxlane_core::protocol::{
    methods, read_frame, write_frame, AgentReportParams, EventMsg, Request, Response,
    TermSubscribeParams, TermSubscribeResult,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock as StdRwLock};
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
    subs: Arc<Mutex<SubRegistry>>,
    socket_path: PathBuf,
    dirty: DirtyFlag,
    /// 全局状态事件广播（agent.status_changed 等）
    events: tokio::sync::broadcast::Sender<EventMsg>,
    auth: muxlane_core::AuthSecret,
    lifecycle: Arc<Mutex<()>>,
    persistence_path: StdRwLock<Option<PathBuf>>,
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
        let server = Arc::new(MuxlaneServer {
            runtime,
            state,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            subs: Arc::new(Mutex::new(SubRegistry::default())),
            socket_path,
            dirty,
            events,
            auth,
            lifecycle: Arc::new(Mutex::new(())),
            persistence_path: StdRwLock::new(None),
        });
        // 转发任务退出时自行摘除条目需要注册表自身的 Weak。
        if let Ok(mut subs) = server.subs.try_lock() {
            subs.bind_self(&server.subs);
        }
        server
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
        if let Ok(mut slot) = self.persistence_path.write() {
            *slot = Some(path);
        }
    }

    async fn persist_runtime_state(&self) -> anyhow::Result<()> {
        let path = self
            .persistence_path
            .read()
            .ok()
            .and_then(|path| path.clone());
        let Some(path) = path else { return Ok(()) };
        let snapshot = self.state.read().await.snapshot();
        let previous = muxlane_store::load(&path).unwrap_or_default();
        let persisted =
            muxlane_store::PersistedApp::from_snapshot(&snapshot).with_ui_prefs_from(&previous);
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
                    muxlane_core::protocol::features::TERM_INPUT.into(),
                    muxlane_core::protocol::features::TERM_RESIZE.into(),
                ],
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
                let (snapshot, rx) = session.subscribe();
                (
                    muxlane_core::model::new_id("sub"),
                    muxlane_core::protocol::b64_encode(&snapshot),
                    rx,
                    session,
                )
            })
        };
        let Some((sub_id, replay_b64, rx, session)) = prepared else {
            return Ok(Response::err(
                req.id,
                "no_such_agent",
                format!("agent {} not running", params.agent),
            ));
        };
        self.subs
            .lock()
            .await
            .add(&sub_id, &params.agent, ev_tx.clone(), rx, session);
        connection_subs.push(sub_id.clone());
        Ok(Response::ok(
            req.id,
            serde_json::to_value(TermSubscribeResult { sub_id, replay_b64 })?,
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
        session.resize(params.cols, params.rows)?;
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
                let code = if message.starts_with("no such project:") {
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
        let _ = self.state.write().await.report_hook(&params).await;
        self.dirty.bump();
        Ok(Response::ok(req.id, serde_json::json!({"ok": true})))
    }

    async fn handle_unknown_method(
        &self,
        request_id: u64,
        method: &str,
    ) -> anyhow::Result<Response> {
        Ok(Response::method_not_found(request_id, method))
    }

    async fn handle_conn(self: Arc<Self>, stream: UnixStream) -> anyhow::Result<()> {
        let (read_half, mut write_half) = stream.into_split();
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

        loop {
            tokio::select! {
                frame = frame_rx.recv() => match frame {
                    Some(Ok(value)) => {
                        let req: Request = match serde_json::from_value(value) {
                            Ok(req) => req,
                            Err(error) => {
                                write_frame(&mut write_half, &Response::err(0, "bad_request", error.to_string())).await?;
                                continue;
                            }
                        };
                        let response = match req.method.as_str() {
                            methods::SYSTEM_HELLO => self.handle_system_hello(req).await?,
                            methods::STATE_LIST => self.handle_state_list(req).await?,
                            methods::EVENTS_SUBSCRIBE => self.handle_events_subscribe(req, &mut status_rx, &ev_tx).await?,
                            methods::TERM_SUBSCRIBE => self.handle_term_subscribe(req, &ev_tx, &mut connection_subs).await?,
                            methods::TERM_UNSUBSCRIBE => self.handle_term_unsubscribe(req, &mut connection_subs).await?,
                            methods::TERM_INPUT => self.handle_term_input(req).await?,
                            methods::TERM_RESIZE => self.handle_term_resize(req).await?,
                            methods::AGENT_SPAWN => self.handle_agent_spawn(req).await?,
                            methods::AGENT_DELETE => self.handle_agent_delete(req).await?,
                            methods::PROJECT_ADD => self.handle_project_add(req).await?,
                            methods::PROJECT_DELETE => self.handle_project_delete(req).await?,
                            methods::AGENT_REPORT => self.handle_agent_report(req).await?,
                            other => self.handle_unknown_method(req.id, other).await?,
                        };
                        write_frame(&mut write_half, &response).await?;
                    }
                    Some(Err(muxlane_core::Error::Eof)) | None => break,
                    Some(Err(error)) => return Err(error.into()),
                },
                event = ev_rx.recv() => match event {
                    Some(message) => write_frame(&mut write_half, &message).await?,
                    None => break,
                },
                status = async {
                    match status_rx.as_mut() {
                        Some(receiver) => receiver.recv().await,
                        None => std::future::pending().await,
                    }
                } => match status {
                    Ok(message) => write_frame(&mut write_half, &message).await?,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
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
        Ok(())
    }
}
