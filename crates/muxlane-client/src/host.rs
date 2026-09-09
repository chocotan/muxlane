//! RemoteHost：一台远端 muxlane 实例的连接管理（含 SSH 隧道与重连）
use muxlane_core::model::Snapshot;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::RwLock as StdRwLock;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{mpsc, Mutex, Notify};

/// 从 HostCfg 提取 SSH 目标（bootstrap 仅支持 SSH 远端）
fn ssh_target(cfg: &HostCfg) -> Result<&str, crate::tunnel::TunnelError> {
    match &cfg.target {
        Target::Ssh { host, .. } => Ok(host),
        Target::Socket(_) => Err(crate::tunnel::TunnelError::Other(
            "direct socket target cannot be installed".into(),
        )),
    }
}

#[derive(Clone, Default)]
pub enum SshAuth {
    #[default]
    SshConfig,
    PublicKey {
        username: Option<String>,
        identity_file: Option<String>,
    },
    Password {
        username: String,
        password: String,
    },
}

impl From<SshAuth> for muxlane_store::PersistedRemoteAuth {
    fn from(auth: SshAuth) -> Self {
        match auth {
            SshAuth::SshConfig => Self::SshConfig,
            SshAuth::PublicKey {
                username,
                identity_file,
            } => Self::PublicKey {
                username,
                identity_file,
            },
            SshAuth::Password { username, password } => Self::Password {
                username,
                password: (!password.is_empty()).then_some(password),
            },
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("password secret is missing")]
pub struct MissingPassword;

impl TryFrom<muxlane_store::PersistedRemoteAuth> for SshAuth {
    type Error = MissingPassword;

    fn try_from(auth: muxlane_store::PersistedRemoteAuth) -> Result<Self, Self::Error> {
        match auth {
            muxlane_store::PersistedRemoteAuth::SshConfig => Ok(Self::SshConfig),
            muxlane_store::PersistedRemoteAuth::PublicKey {
                username,
                identity_file,
            } => Ok(Self::PublicKey {
                username,
                identity_file,
            }),
            muxlane_store::PersistedRemoteAuth::Password { username, password } => {
                Ok(Self::Password {
                    username,
                    password: password.ok_or(MissingPassword)?,
                })
            }
        }
    }
}

impl std::fmt::Debug for SshAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SshAuth::SshConfig => f.write_str("SshConfig"),
            SshAuth::PublicKey {
                username,
                identity_file,
            } => f
                .debug_struct("PublicKey")
                .field("username", username)
                .field("identity_file", identity_file)
                .finish(),
            SshAuth::Password { username, .. } => f
                .debug_struct("Password")
                .field("username", username)
                .field("password", &"[redacted]")
                .finish(),
        }
    }
}

impl SshAuth {
    pub fn destination(&self, host: &str) -> String {
        if host.contains('@') {
            return host.to_string();
        }
        match self {
            SshAuth::PublicKey {
                username: Some(username),
                ..
            }
            | SshAuth::Password { username, .. }
                if !username.trim().is_empty() =>
            {
                format!("{}@{}", username.trim(), host)
            }
            _ => host.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HostCfg {
    pub name: String,
    /// 直连 socket 路径（本机/已转发）或 SSH 目标（user@host）
    pub target: Target,
    pub auth: SshAuth,
    /// 重连初始退避
    pub retry_base_ms: u64,
}

#[derive(Debug, Clone)]
pub enum Target {
    /// 本机 unix socket 直连
    Socket(String),
    /// SSH：host + 远端 socket 路径（远端和本地路径通常一致）
    Ssh { host: String, socket: String },
}

/// 用户输入 → 连接目标：
/// - `/path/muxlane.sock`：本地/已转发 Unix socket
/// - `user@host:/path/muxlane.sock`：SSH StreamLocalForward
pub fn parse_target(input: &str) -> Target {
    let input = input.trim();
    if input.starts_with('/') {
        return Target::Socket(input.into());
    }
    if let Some((host, socket)) = input.split_once(':') {
        if !host.is_empty() && socket.starts_with('/') {
            return Target::Ssh {
                host: host.into(),
                socket: socket.into(),
            };
        }
    }
    // 普通用户只填 SSH config alias / user@host；socket 自动发现。
    Target::Ssh {
        host: input.into(),
        socket: String::new(),
    }
}

#[cfg(test)]
mod target_tests {
    use super::*;

    #[test]
    fn ssh_auth_builds_destination_without_exposing_password() {
        let password = SshAuth::Password {
            username: "alice".into(),
            password: "super-secret".into(),
        };
        assert_eq!(password.destination("server"), "alice@server");
        assert!(!format!("{password:?}").contains("super-secret"));
        let key = SshAuth::PublicKey {
            username: Some("bob".into()),
            identity_file: Some("~/.ssh/id_ed25519".into()),
        };
        assert_eq!(key.destination("nuc"), "bob@nuc");
    }

    #[test]
    fn ssh_auth_persistence_conversion_roundtrips() {
        let auth = SshAuth::Password {
            username: "alice".into(),
            password: "secret".into(),
        };
        let persisted: muxlane_store::PersistedRemoteAuth = auth.into();
        let restored = SshAuth::try_from(persisted).unwrap();

        assert!(matches!(
            restored,
            SshAuth::Password { username, password }
                if username == "alice" && password == "secret"
        ));
        assert!(matches!(
            SshAuth::try_from(muxlane_store::PersistedRemoteAuth::Password {
                username: "alice".into(),
                password: None,
            }),
            Err(MissingPassword)
        ));
    }

    #[test]
    fn ssh_alias_does_not_require_internal_socket_path() {
        match parse_target("choco@192.168.1.20") {
            Target::Ssh { host, socket } => {
                assert_eq!(host, "choco@192.168.1.20");
                assert!(socket.is_empty());
            }
            Target::Socket(_) => panic!("expected SSH target"),
        }
    }

    #[test]
    fn advanced_socket_targets_remain_supported() {
        assert!(matches!(
            parse_target("/tmp/muxlane.sock"),
            Target::Socket(_)
        ));
        assert!(matches!(
            parse_target("nuc:/run/user/1000/muxlane.sock"),
            Target::Ssh { socket, .. } if socket == "/run/user/1000/muxlane.sock"
        ));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteStage {
    SshProbe,
    Tunnel,
    Subscribe,
}

impl RemoteStage {
    pub fn label(self) -> &'static str {
        match self {
            RemoteStage::SshProbe => "SSH 探测",
            RemoteStage::Tunnel => "建立 tunnel",
            RemoteStage::Subscribe => "订阅状态",
        }
    }
}

#[derive(Debug, Clone)]
pub enum RemoteState {
    Connecting(RemoteStage),
    NeedsInstall {
        remote_socket: String,
    },
    NeedsStart {
        remote_socket: String,
        binary: String,
    },
    NeedsUpgrade {
        remote_socket: String,
    },
    AuthenticationFailed(String),
    Online(Snapshot),
    Offline(String),
}

/// 远端安装/升级的阶段性进度（上传/安装/重启）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootstrapProgress {
    pub phase: BootstrapPhase,
    /// 0..=100，None 表示该阶段无细分进度
    pub percent: Option<u8>,
    /// 上传阶段的已传输字节数（gzip 压缩后）
    pub done_bytes: Option<u64>,
    /// 上传阶段的总量字节数（gzip 压缩后）
    pub total_bytes: Option<u64>,
}

/// 上传回调携带的字节进度（done/total 均为传输流字节，通常已 gzip 压缩）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadProgress {
    pub done: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapPhase {
    Upload,
    Install,
    Restart,
}

impl BootstrapPhase {
    pub fn label(self) -> &'static str {
        match self {
            BootstrapPhase::Upload => "上传二进制",
            BootstrapPhase::Install => "安装",
            BootstrapPhase::Restart => "重启服务",
        }
    }

    /// 阶段在总进度里的起始百分比（上传 0-70，安装 70-85，重启 85-100）
    fn base(self) -> f32 {
        match self {
            BootstrapPhase::Upload => 0.0,
            BootstrapPhase::Install => 70.0,
            BootstrapPhase::Restart => 85.0,
        }
    }

    fn span(self) -> f32 {
        match self {
            BootstrapPhase::Upload => 70.0,
            BootstrapPhase::Install => 15.0,
            BootstrapPhase::Restart => 15.0,
        }
    }

    /// 换算为总进度 0..=100；未知细分进度时取阶段中点
    pub fn overall(self, percent: Option<u8>) -> u8 {
        let ratio = percent
            .map(|p| (p.clamp(0, 100) as f32) / 100.0)
            .unwrap_or(0.5);
        (self.base() + self.span() * ratio)
            .round()
            .clamp(0.0, 100.0) as u8
    }
}

#[derive(Debug, Clone)]
pub enum ClientEvent {
    StateChanged {
        host: String,
        state: RemoteState,
    },
    BootstrapProgress {
        host: String,
        progress: BootstrapProgress,
    },
    StatusChanged {
        host: String,
        agent: muxlane_core::model::AgentId,
        agent_type: Option<muxlane_core::model::AgentType>,
        from: muxlane_core::model::AgentStatus,
        to: muxlane_core::model::AgentStatus,
        message: Option<String>,
    },
    TermData {
        agent: muxlane_core::model::AgentId,
        data: Vec<u8>,
    },
}

pub struct RemoteHost {
    pub cfg: HostCfg,
    state: Arc<Mutex<RemoteState>>,
    events_tx: mpsc::Sender<ClientEvent>,
    endpoint: StdRwLock<Option<String>>,
    stopped: AtomicBool,
    retry: Notify,
    latency_ms: AtomicU64,
    /// UI 操作共用的串行 RPC 长连接；事件订阅使用另一条专职读连接。
    rpc: Mutex<Option<crate::Connection>>,
    /// 键盘输入专用连接（fire-and-forget，不等响应）：
    /// 避免慢 RPC（网络抖动时一个 30s 超时的调用）队头阻塞所有按键。
    /// 已知取舍：input 与 rpc（如 resize）走不同连接，极端时序下可能乱序
    /// （按键先于 SIGWINCH 到达），表现为一帧错帧，可自恢复。
    input: Mutex<Option<crate::RequestWriter>>,
    /// 完整进度快照；访问极短，不跨 await。
    progress: StdMutex<Option<BootstrapProgress>>,
    bootstrap_cancel: Arc<AtomicBool>,
    /// Stable machine identity learned from an authoritative Online snapshot.
    machine_id: StdRwLock<Option<String>>,
    /// Capabilities advertised by the latest successful system.hello.
    capabilities: StdRwLock<HashSet<String>>,
}

impl RemoteHost {
    pub fn new(cfg: HostCfg, events_tx: mpsc::Sender<ClientEvent>) -> Arc<Self> {
        Arc::new(RemoteHost {
            cfg,
            state: Arc::new(Mutex::new(RemoteState::Connecting(RemoteStage::SshProbe))),
            events_tx,
            endpoint: StdRwLock::new(None),
            stopped: AtomicBool::new(false),
            retry: Notify::new(),
            latency_ms: AtomicU64::new(0),
            rpc: Mutex::new(None),
            input: Mutex::new(None),
            progress: StdMutex::new(None),
            bootstrap_cancel: Arc::new(AtomicBool::new(false)),
            machine_id: StdRwLock::new(None),
            capabilities: StdRwLock::new(HashSet::new()),
        })
    }

    pub fn cancel_bootstrap(&self) {
        self.bootstrap_cancel.store(true, Ordering::Relaxed);
        self.clear_progress();
    }

    pub async fn state(&self) -> RemoteState {
        self.state.lock().await.clone()
    }

    fn emit_progress(&self, phase: BootstrapPhase, percent: Option<u8>) {
        self.emit_progress_bytes(phase, percent, None, None);
    }

    fn emit_progress_bytes(
        &self,
        phase: BootstrapPhase,
        percent: Option<u8>,
        done_bytes: Option<u64>,
        total_bytes: Option<u64>,
    ) {
        let progress = BootstrapProgress {
            phase,
            percent,
            done_bytes,
            total_bytes,
        };
        if let Ok(mut current) = self.progress.lock() {
            *current = Some(progress);
        }
        let _ = self.events_tx.try_send(ClientEvent::BootstrapProgress {
            host: self.cfg.name.clone(),
            progress,
        });
    }

    pub fn progress_now(&self) -> Option<BootstrapProgress> {
        self.progress.lock().ok().and_then(|progress| *progress)
    }

    fn clear_progress(&self) {
        if let Ok(mut progress) = self.progress.lock() {
            *progress = None;
        }
        self.retry.notify_one();
    }

    fn drop_progress(&self) {
        if let Ok(mut progress) = self.progress.lock() {
            *progress = None;
        }
    }

    /// 主动上报 NeedsUpgrade（例如远端对已知方法返回 unknown_method）
    pub async fn mark_needs_upgrade(&self) {
        let socket = self.endpoint_now().unwrap_or_default();
        self.set_state(
            RemoteState::NeedsUpgrade {
                remote_socket: socket,
            },
            &self.events_tx,
        )
        .await;
        self.retry.notify_one();
    }

    /// 本地可连的 socket 路径（直连或隧道端口对应的 socket）
    async fn local_socket(&self) -> Result<String, crate::tunnel::TunnelError> {
        let endpoint = match &self.cfg.target {
            Target::Socket(p) => p.clone(),
            Target::Ssh { host, socket } => {
                crate::tunnel::ensure_tunnel(host, socket, &self.cfg.auth).await?
            }
        };
        if let Ok(mut slot) = self.endpoint.write() {
            *slot = Some(endpoint.clone());
        }
        Ok(endpoint)
    }

    pub fn endpoint_now(&self) -> Option<String> {
        self.endpoint.read().ok().and_then(|v| v.clone())
    }

    async fn rpc(&self) -> anyhow::Result<tokio::sync::MutexGuard<'_, Option<crate::Connection>>> {
        let mut rpc = self.rpc.lock().await;
        if rpc.is_none() {
            let socket = self.local_socket().await?;
            *rpc = Some(crate::open(&socket).await?);
        }
        Ok(rpc)
    }

    fn rpc_failed(&self, rpc: &mut Option<crate::Connection>) {
        *rpc = None;
        self.retry.notify_one();
    }

    fn rpc_error_requires_reconnect(error: &anyhow::Error) -> bool {
        error.downcast_ref::<crate::RpcCallError>().is_none()
            && error.downcast_ref::<crate::RemoteCompatError>().is_none()
    }

    fn handle_rpc_result<T>(
        &self,
        rpc: &mut Option<crate::Connection>,
        result: &anyhow::Result<T>,
    ) {
        if result
            .as_ref()
            .is_err_and(Self::rpc_error_requires_reconnect)
        {
            self.rpc_failed(rpc);
        }
    }

    pub async fn fetch_snapshot(&self) -> anyhow::Result<Snapshot> {
        let mut rpc = self.rpc().await?;
        let result = crate::fetch_snapshot(rpc.as_mut().expect("RPC initialized")).await;
        self.handle_rpc_result(&mut rpc, &result);
        result
    }

    pub async fn send_term_input(
        &self,
        agent: &muxlane_core::model::AgentId,
        data: &[u8],
    ) -> anyhow::Result<()> {
        let mut input = self.input.lock().await;
        if input.is_none() {
            let socket = self.local_socket().await?;
            let connection = crate::open(&socket).await?;
            let (writer, mut reader) = connection.into_split();
            // 后台排空响应/事件帧，防止对端 socket 缓冲堆积。
            tokio::spawn(async move { while reader.next().await.is_ok() {} });
            *input = Some(writer);
        }
        let writer = input.as_mut().expect("input connection");
        let params = serde_json::to_value(muxlane_core::protocol::TermInputParams {
            agent: agent.clone(),
            data_b64: muxlane_core::protocol::b64_encode(data),
        })?;
        match writer
            .call(muxlane_core::protocol::methods::TERM_INPUT, params)
            .await
        {
            Ok(_) => Ok(()),
            Err(error) => {
                // 连接作废：下次按键时懒重建。不触发 retry（rpc 连接可能完全健康，
                // 避免单次按键失败引发全量重连/订阅重建）。
                *input = None;
                Err(error)
            }
        }
    }

    pub async fn resize_term(
        &self,
        agent: &muxlane_core::model::AgentId,
        cols: u16,
        rows: u16,
    ) -> anyhow::Result<()> {
        let mut rpc = self.rpc().await?;
        let result =
            crate::resize_term(rpc.as_mut().expect("RPC initialized"), agent, cols, rows).await;
        self.handle_rpc_result(&mut rpc, &result);
        result
    }

    pub async fn spawn_agent(
        &self,
        project: &muxlane_core::model::ProjectId,
        preset: Option<&muxlane_core::AgentPreset>,
    ) -> anyhow::Result<muxlane_core::model::AgentInstance> {
        let mut rpc = self.rpc().await?;
        let result =
            crate::spawn_agent(rpc.as_mut().expect("RPC initialized"), project, preset).await;
        self.handle_rpc_result(&mut rpc, &result);
        result
    }

    pub async fn delete_agent(&self, agent: &muxlane_core::model::AgentId) -> anyhow::Result<()> {
        let mut rpc = self.rpc().await?;
        let result = crate::delete_agent(rpc.as_mut().expect("RPC initialized"), agent).await;
        self.handle_rpc_result(&mut rpc, &result);
        result
    }

    /// 将远端 agent 标记为已读（Done/Failed→Idle），真实写回服务端，并随远端
    /// 自身的状态广播与持久化。调用前请先确认 supports(AGENT_MARK_SEEN)，
    /// 旧版本远端会返回 method_not_found。
    pub async fn mark_agent_seen(
        &self,
        agent: &muxlane_core::model::AgentId,
    ) -> anyhow::Result<()> {
        let mut rpc = self.rpc().await?;
        let result = crate::mark_agent_seen(rpc.as_mut().expect("RPC initialized"), agent).await;
        self.handle_rpc_result(&mut rpc, &result);
        result
    }

    pub fn machine_id(&self) -> Option<String> {
        self.machine_id.read().ok().and_then(|value| value.clone())
    }

    /// Returns true when the cached identity changed and should be persisted.
    pub fn cache_machine_id(&self, machine_id: Option<&str>) -> bool {
        let Some(machine_id) = machine_id else {
            return false;
        };
        let Ok(mut current) = self.machine_id.write() else {
            return false;
        };
        if current.as_deref() == Some(machine_id) {
            return false;
        }
        *current = Some(machine_id.to_string());
        true
    }

    pub fn restore_machine_id(&self, machine_id: Option<String>) {
        if let Some(machine_id) = machine_id {
            let _ = self.cache_machine_id(Some(&machine_id));
        }
    }

    pub fn supports(&self, feature: &str) -> bool {
        self.capabilities
            .read()
            .is_ok_and(|features| features.contains(feature))
    }

    pub async fn add_project(
        &self,
        path: &str,
        create_if_missing: bool,
    ) -> anyhow::Result<muxlane_core::model::Project> {
        if create_if_missing && !self.supports(muxlane_core::protocol::features::PROJECT_CREATE) {
            return Err(crate::RemoteCompatError::FeatureUnsupported {
                feature: muxlane_core::protocol::features::PROJECT_CREATE.into(),
            }
            .into());
        }
        let mut rpc = self.rpc().await?;
        let result = crate::add_project(
            rpc.as_mut().expect("RPC initialized"),
            path,
            create_if_missing,
        )
        .await;
        self.handle_rpc_result(&mut rpc, &result);
        result
    }

    pub async fn delete_project(
        &self,
        project: &muxlane_core::model::ProjectId,
    ) -> anyhow::Result<muxlane_core::protocol::DeleteScopeResult> {
        let mut rpc = self.rpc().await?;
        let result = crate::delete_project(rpc.as_mut().expect("RPC initialized"), project).await;
        self.handle_rpc_result(&mut rpc, &result);
        result
    }

    /// 上传字节进度 → 带字节数的 BootstrapProgress 事件
    fn upload_reporter(me: Arc<Self>) -> impl Fn(crate::UploadProgress) + Send + Sync + 'static {
        move |upload: crate::UploadProgress| {
            let percent = upload
                .done
                .min(upload.total)
                .checked_mul(100)
                .and_then(|v| v.checked_div(upload.total))
                .unwrap_or(100) as u8;
            me.emit_progress_bytes(
                BootstrapPhase::Upload,
                Some(percent),
                Some(upload.done),
                Some(upload.total),
            );
        }
    }

    pub async fn install_and_start(self: Arc<Self>) -> Result<(), crate::tunnel::TunnelError> {
        self.bootstrap_cancel.store(false, Ordering::Relaxed);
        self.emit_progress(BootstrapPhase::Upload, Some(0));
        let reporter = Self::upload_reporter(Arc::clone(&self));
        let host = ssh_target(&self.cfg)?.to_string();
        let auth = self.cfg.auth.clone();
        let cancel = Arc::clone(&self.bootstrap_cancel);
        let result = crate::tunnel::install_and_start(&host, &auth, reporter, cancel).await;
        if let Err(error) = result {
            self.clear_progress();
            return Err(error);
        }
        if self.bootstrap_cancel.load(Ordering::Relaxed) {
            self.clear_progress();
            return Err(crate::tunnel::TunnelError::Other("已取消上传".into()));
        }
        self.emit_progress(BootstrapPhase::Restart, Some(0));
        let result =
            crate::tunnel::start_remote(ssh_target(&self.cfg)?, &self.cfg.auth, None).await;
        if let Err(error) = result {
            self.clear_progress();
            return Err(error);
        }
        self.clear_progress();
        Ok(())
    }

    pub async fn start_and_retry(
        self: Arc<Self>,
        binary: &str,
    ) -> Result<(), crate::tunnel::TunnelError> {
        self.emit_progress(BootstrapPhase::Restart, Some(0));
        let result =
            crate::tunnel::start_remote(ssh_target(&self.cfg)?, &self.cfg.auth, Some(binary)).await;
        if result.is_err() {
            self.drop_progress();
        } else {
            self.clear_progress();
        }
        result
    }

    pub async fn upgrade_and_retry(self: Arc<Self>) -> Result<(), crate::tunnel::TunnelError> {
        self.bootstrap_cancel.store(false, Ordering::Relaxed);
        self.emit_progress(BootstrapPhase::Upload, Some(0));
        let reporter = Self::upload_reporter(Arc::clone(&self));
        let host = ssh_target(&self.cfg)?.to_string();
        let auth = self.cfg.auth.clone();
        let cancel = Arc::clone(&self.bootstrap_cancel);
        let result = crate::tunnel::install_and_restart(&host, &auth, reporter, cancel).await;
        if let Err(error) = result {
            self.clear_progress();
            return Err(error);
        }
        if self.bootstrap_cancel.load(Ordering::Relaxed) {
            self.clear_progress();
            return Err(crate::tunnel::TunnelError::Other("已取消上传".into()));
        }
        self.emit_progress(BootstrapPhase::Restart, Some(0));
        let result =
            crate::tunnel::start_remote(ssh_target(&self.cfg)?, &self.cfg.auth, None).await;
        if result.is_err() {
            self.drop_progress();
        } else {
            self.clear_progress();
        }
        result
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(mut rpc) = self.rpc.try_lock() {
            *rpc = None;
        }
        self.retry.notify_waiters();
    }

    pub fn reconnect(&self) {
        if let Ok(mut rpc) = self.rpc.try_lock() {
            *rpc = None;
        }
        self.retry.notify_one();
    }

    pub fn latency_ms(&self) -> Option<u64> {
        let latency = self.latency_ms.load(Ordering::Relaxed);
        (latency > 0).then_some(latency)
    }

    async fn wait_retry(&self, delay: std::time::Duration) {
        tokio::select! {
            _ = tokio::time::sleep(delay) => {},
            _ = self.retry.notified() => {},
        }
    }

    /// 连接循环（由调用方 spawn：tokio::spawn(host.run_loop()) 或 runtime.spawn）
    pub async fn run_loop(self: Arc<Self>) {
        let this = self;
        let mut backoff = this.cfg.retry_base_ms.max(200);
        loop {
            if this.stopped.load(Ordering::Acquire) {
                break;
            }
            this.set_state(
                RemoteState::Connecting(RemoteStage::SshProbe),
                &this.events_tx,
            )
            .await;
            let socket = match this.local_socket().await {
                Ok(socket) => socket,
                Err(crate::tunnel::TunnelError::NeedsInstall { remote_socket }) => {
                    this.set_state(RemoteState::NeedsInstall { remote_socket }, &this.events_tx)
                        .await;
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {},
                        _ = this.retry.notified() => {},
                    }
                    continue;
                }
                Err(crate::tunnel::TunnelError::NeedsStart {
                    remote_socket,
                    binary,
                }) => {
                    this.set_state(
                        RemoteState::NeedsStart {
                            remote_socket,
                            binary,
                        },
                        &this.events_tx,
                    )
                    .await;
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {},
                        _ = this.retry.notified() => {},
                    }
                    continue;
                }
                Err(crate::tunnel::TunnelError::Authentication(error)) => {
                    this.set_state(RemoteState::AuthenticationFailed(error), &this.events_tx)
                        .await;
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {},
                        _ = this.retry.notified() => {},
                    }
                    continue;
                }
                Err(error) => {
                    this.set_state(RemoteState::Offline(error.to_string()), &this.events_tx)
                        .await;
                    this.wait_retry(std::time::Duration::from_millis(backoff))
                        .await;
                    backoff = (backoff * 2).min(30_000);
                    continue;
                }
            };
            this.set_state(
                RemoteState::Connecting(RemoteStage::Tunnel),
                &this.events_tx,
            )
            .await;
            match crate::open(&socket).await {
                Ok(mut conn) => {
                    let started = std::time::Instant::now();
                    let hello = conn
                        .call(
                            muxlane_core::protocol::methods::SYSTEM_HELLO,
                            serde_json::json!({}),
                        )
                        .await
                        .and_then(|value| {
                            serde_json::from_value::<muxlane_core::protocol::HelloResult>(value)
                                .map_err(Into::into)
                        });
                    if let Ok(mut capabilities) = this.capabilities.write() {
                        capabilities.clear();
                        if let Ok(hello) = &hello {
                            capabilities.extend(hello.features.iter().cloned());
                        }
                    }
                    let compatible = hello.as_ref().is_ok_and(|hello| {
                        hello.protocol >= muxlane_core::protocol::PROTOCOL_VERSION
                            && hello.features.iter().any(|feature| {
                                feature == muxlane_core::protocol::features::TERM_INPUT
                            })
                            && hello.features.iter().any(|feature| {
                                feature == muxlane_core::protocol::features::PROJECT_ADD
                            })
                    });
                    if !compatible {
                        this.set_state(
                            RemoteState::NeedsUpgrade {
                                remote_socket: socket.clone(),
                            },
                            &this.events_tx,
                        )
                        .await;
                        tokio::select! {
                            _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {},
                            _ = this.retry.notified() => {},
                        }
                        continue;
                    }
                    this.set_state(
                        RemoteState::Connecting(RemoteStage::Subscribe),
                        &this.events_tx,
                    )
                    .await;
                    // 订阅事件流
                    if let Err(e) = conn
                        .call(
                            muxlane_core::protocol::methods::EVENTS_SUBSCRIBE,
                            serde_json::json!(null),
                        )
                        .await
                    {
                        this.set_state(RemoteState::Offline(e.to_string()), &this.events_tx)
                            .await;
                        this.wait_retry(std::time::Duration::from_millis(backoff))
                            .await;
                        backoff = (backoff * 2).min(30_000);
                        continue;
                    }
                    // 拉全量快照（复用 RemoteHost RPC 长连接）
                    match this.fetch_snapshot().await {
                        Ok(snap) => {
                            this.latency_ms.store(
                                started.elapsed().as_millis().clamp(1, u64::MAX as u128) as u64,
                                Ordering::Relaxed,
                            );
                            this.set_state(RemoteState::Online(snap), &this.events_tx)
                                .await;
                            backoff = this.cfg.retry_base_ms.max(200);
                        }
                        Err(e) => {
                            this.set_state(RemoteState::Offline(e.to_string()), &this.events_tx)
                                .await;
                            this.wait_retry(std::time::Duration::from_millis(backoff))
                                .await;
                            backoff = (backoff * 2).min(30_000);
                            continue;
                        }
                    }
                    // 读循环：事件推送。state.changed 使用 200ms 滑动窗口合并。
                    let (_writer, mut reader) = conn.into_split();
                    let mut refresh_deadline = None;
                    loop {
                        if this.stopped.load(Ordering::Acquire) {
                            return;
                        }
                        let next = tokio::select! {
                            frame = reader.next() => Some(frame),
                            _ = async {
                                match refresh_deadline {
                                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                                    None => std::future::pending().await,
                                }
                            } => None,
                            _ = this.retry.notified() => {
                                if this.stopped.load(Ordering::Acquire) {
                                    return;
                                }
                                break;
                            }
                        };
                        let Some(next) = next else {
                            refresh_deadline = None;
                            if let Ok(snap) = this.fetch_snapshot().await {
                                this.set_state(RemoteState::Online(snap), &this.events_tx)
                                    .await;
                            }
                            continue;
                        };
                        match next {
                            Ok(crate::Frame::Event(ev)) => match ev.event.as_str() {
                                muxlane_core::protocol::events::STATE_CHANGED => {
                                    refresh_deadline = Some(
                                        tokio::time::Instant::now()
                                            + std::time::Duration::from_millis(200),
                                    );
                                }
                                muxlane_core::protocol::events::AGENT_STATUS => {
                                    if let Ok(s) = serde_json::from_value::<
                                        muxlane_core::protocol::AgentStatusEvent,
                                    >(ev.params)
                                    {
                                        let _ = this
                                            .events_tx
                                            .send(ClientEvent::StatusChanged {
                                                host: this.cfg.name.clone(),
                                                agent: s.agent,
                                                agent_type: s.agent_type,
                                                from: s.from,
                                                to: s.to,
                                                message: s.message,
                                            })
                                            .await;
                                    }
                                }
                                _ => {}
                            },
                            Ok(crate::Frame::Response(_)) => {}
                            Err(e) => {
                                this.set_state(
                                    RemoteState::Offline(e.to_string()),
                                    &this.events_tx,
                                )
                                .await;
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    this.set_state(RemoteState::Offline(e.to_string()), &this.events_tx)
                        .await;
                }
            }
            this.wait_retry(std::time::Duration::from_millis(backoff))
                .await;
            if this.stopped.load(Ordering::Acquire) {
                break;
            }
            backoff = (backoff * 2).min(30_000);
        }
    }

    async fn set_state(&self, s: RemoteState, tx: &mpsc::Sender<ClientEvent>) {
        *self.state.lock().await = s.clone();
        let _ = tx
            .send(ClientEvent::StateChanged {
                host: self.cfg.name.clone(),
                state: s,
            })
            .await;
    }
}

#[cfg(test)]
mod progress_tests {
    use super::*;

    #[tokio::test]
    async fn missing_create_capability_only_blocks_creation_requests() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("old-server.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = tokio::io::split(stream);
            let mut read = tokio::io::BufReader::new(read);
            let request: muxlane_core::protocol::Request = serde_json::from_value(
                muxlane_core::protocol::read_frame(&mut read).await.unwrap(),
            )
            .unwrap();
            let params: muxlane_core::protocol::ProjectAddParams =
                serde_json::from_value(request.params).unwrap();
            assert!(!params.create_if_missing);
            let project = muxlane_core::model::Project {
                id: "existing".into(),
                name: "existing".into(),
                path: "/existing".into(),
                branch: None,
                agents: vec![],
            };
            muxlane_core::protocol::write_frame(
                &mut write,
                &muxlane_core::protocol::Response::ok(
                    request.id,
                    serde_json::to_value(project).unwrap(),
                ),
            )
            .await
            .unwrap();
        });
        let (events_tx, _events_rx) = mpsc::channel(1);
        let host = RemoteHost::new(
            HostCfg {
                name: "old".into(),
                target: Target::Socket(socket.display().to_string()),
                auth: SshAuth::default(),
                retry_base_ms: 200,
            },
            events_tx,
        );

        let existing = host.add_project("/existing", false).await.unwrap();
        assert_eq!(existing.id, "existing");
        let error = host.add_project("/missing", true).await.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<crate::RemoteCompatError>(),
            Some(crate::RemoteCompatError::FeatureUnsupported { feature })
                if feature == muxlane_core::protocol::features::PROJECT_CREATE
        ));
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn rpc_business_error_keeps_connection_and_does_not_wake_retry() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("server.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = tokio::io::split(stream);
            let mut read = tokio::io::BufReader::new(read);

            let request: muxlane_core::protocol::Request = serde_json::from_value(
                muxlane_core::protocol::read_frame(&mut read).await.unwrap(),
            )
            .unwrap();
            let params: muxlane_core::protocol::ProjectAddParams =
                serde_json::from_value(request.params).unwrap();
            assert!(!params.create_if_missing);
            muxlane_core::protocol::write_frame(
                &mut write,
                &muxlane_core::protocol::Response::err(
                    request.id,
                    muxlane_core::protocol::error_codes::PATH_NOT_FOUND,
                    "missing directory",
                ),
            )
            .await
            .unwrap();

            let request: muxlane_core::protocol::Request = serde_json::from_value(
                muxlane_core::protocol::read_frame(&mut read).await.unwrap(),
            )
            .unwrap();
            let params: muxlane_core::protocol::ProjectAddParams =
                serde_json::from_value(request.params).unwrap();
            assert!(params.create_if_missing);
            let project = muxlane_core::model::Project {
                id: "created".into(),
                name: "created".into(),
                path: "/created".into(),
                branch: None,
                agents: vec![],
            };
            muxlane_core::protocol::write_frame(
                &mut write,
                &muxlane_core::protocol::Response::ok(
                    request.id,
                    serde_json::to_value(project).unwrap(),
                ),
            )
            .await
            .unwrap();
        });
        let (events_tx, _events_rx) = mpsc::channel(1);
        let host = RemoteHost::new(
            HostCfg {
                name: "server".into(),
                target: Target::Socket(socket.display().to_string()),
                auth: SshAuth::default(),
                retry_base_ms: 200,
            },
            events_tx,
        );
        host.capabilities
            .write()
            .unwrap()
            .insert(muxlane_core::protocol::features::PROJECT_CREATE.into());

        let error = host.add_project("/missing", false).await.unwrap_err();
        assert_eq!(
            error.downcast_ref::<crate::RpcCallError>().unwrap().code,
            muxlane_core::protocol::error_codes::PATH_NOT_FOUND
        );
        assert!(host.rpc.lock().await.is_some());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), host.retry.notified())
                .await
                .is_err(),
            "business errors must not wake the reconnect loop"
        );

        let project = host.add_project("/created", true).await.unwrap();
        assert_eq!(project.id, "created");
        peer.await.unwrap();
    }

    #[test]
    fn machine_identity_can_be_restored_and_updated() {
        let (events_tx, _events_rx) = mpsc::channel(1);
        let host = RemoteHost::new(
            HostCfg {
                name: "test".into(),
                target: Target::Socket("/tmp/unused.sock".into()),
                auth: SshAuth::default(),
                retry_base_ms: 200,
            },
            events_tx,
        );
        assert_eq!(host.machine_id(), None);
        host.restore_machine_id(Some("machine-old".into()));
        assert_eq!(host.machine_id().as_deref(), Some("machine-old"));
        assert!(!host.cache_machine_id(Some("machine-old")));
        assert!(host.cache_machine_id(Some("machine-new")));
        assert_eq!(host.machine_id().as_deref(), Some("machine-new"));
    }

    #[test]
    fn progress_snapshot_preserves_byte_counts() {
        let (events_tx, _events_rx) = mpsc::channel(1);
        let host = RemoteHost::new(
            HostCfg {
                name: "test".into(),
                target: Target::Socket("/tmp/unused.sock".into()),
                auth: SshAuth::default(),
                retry_base_ms: 200,
            },
            events_tx,
        );
        host.emit_progress_bytes(BootstrapPhase::Upload, Some(25), Some(10), Some(40));
        assert_eq!(
            host.progress_now(),
            Some(BootstrapProgress {
                phase: BootstrapPhase::Upload,
                percent: Some(25),
                done_bytes: Some(10),
                total_bytes: Some(40),
            })
        );
    }

    #[test]
    fn bootstrap_phase_overall_spans_upload_install_restart() {
        // 上传阶段：0% → 0，100% → 70
        assert_eq!(BootstrapPhase::Upload.overall(Some(0)), 0);
        assert_eq!(BootstrapPhase::Upload.overall(Some(100)), 70);
        assert_eq!(BootstrapPhase::Upload.overall(Some(50)), 35);
        // 安装阶段：70-85
        assert_eq!(BootstrapPhase::Install.overall(Some(0)), 70);
        assert_eq!(BootstrapPhase::Install.overall(Some(100)), 85);
        // 重启阶段：85-100
        assert_eq!(BootstrapPhase::Restart.overall(Some(0)), 85);
        assert_eq!(BootstrapPhase::Restart.overall(Some(100)), 100);
        // 无细分进度取阶段中点
        assert_eq!(BootstrapPhase::Install.overall(None), 78);
        // 边界钳制
        assert_eq!(BootstrapPhase::Upload.overall(Some(255)), 70);
    }
}
