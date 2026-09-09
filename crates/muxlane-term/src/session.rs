//! PtySession：本地 PTY 包装，唯一输出分叉点（tap broadcast + replay 环形缓冲）
use crate::ReplayBuffer;
use anyhow::{Context as _, Result};
use muxlane_core::model::{AgentId, AgentType};
use muxlane_core::protocol::b64_encode;
use portable_pty::{native_pty_system, CommandBuilder};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};

/// 回放缓冲 512KB
const REPLAY_CAP: usize = 512 * 1024;
/// broadcast channel 容量（慢订阅者丢最旧帧）
const BROADCAST_CAP: usize = 256;

/// 解析用户 shell：优先显式配置，再尝试常用 shell、$SHELL 和登录 shell。
pub fn default_shell_program() -> String {
    // 显式配置优先。
    if let Ok(s) = std::env::var("MUXLANE_SHELL") {
        let s = s.trim();
        if !s.is_empty() && std::path::Path::new(s).exists() {
            return s.to_string();
        }
    }
    // 用户有 zsh 配置且 zsh 已安装：按其真实日常环境启动 zsh。
    // （本机桌面登录 shell 可能是 bash，但终端应用配置实际使用 zsh。）
    let home = std::env::var("HOME").unwrap_or_default();
    for zsh in ["/usr/bin/zsh", "/bin/zsh"] {
        if std::path::Path::new(zsh).exists() && std::path::Path::new(&home).join(".zshrc").exists()
        {
            return zsh.to_string();
        }
    }
    if let Ok(s) = std::env::var("SHELL") {
        let s = s.trim();
        if !s.is_empty() && std::path::Path::new(s).exists() {
            return s.to_string();
        }
    }
    #[cfg(unix)]
    {
        // /etc/passwd 登录 shell
        if let Ok(passwd) = std::fs::read_to_string("/etc/passwd") {
            let uid = unsafe { libc_getuid() };
            for line in passwd.lines() {
                let fields: Vec<&str> = line.split(':').collect();
                if fields.len() >= 7 && fields[2].parse::<u32>().ok() == Some(uid) {
                    let sh = fields[6];
                    if std::path::Path::new(sh).exists() {
                        return sh.to_string();
                    }
                }
            }
        }
        "/bin/bash".into()
    }
    #[cfg(not(unix))]
    {
        "sh".into()
    }
}

#[cfg(unix)]
extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

#[derive(Debug, Clone)]
pub struct LaunchCfg {
    pub agent: AgentId,
    pub agent_type: AgentType,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    /// 默认按 agent_type 取程序；override 优先
    pub program_override: Option<String>,
    pub args: Vec<String>,
    pub cols: u16,
    pub rows: u16,
    /// tmux session：Some 时 `new-session -A`，GUI 关闭只 detach，进程继续运行。
    pub tmux_session: Option<String>,
}

impl LaunchCfg {
    pub fn shell(agent: AgentId, cwd: PathBuf) -> Self {
        let tmux_session = Some(format!("muxlane-{}", sanitize_tmux_name(&agent)));
        LaunchCfg {
            agent,
            agent_type: AgentType::Shell,
            cwd,
            env: vec![],
            program_override: None,
            args: vec![],
            cols: 120,
            rows: 32,
            tmux_session,
        }
    }
}

#[derive(Debug, Clone)]
pub enum SessionEvent {
    Exit { code: Option<u32> },
}

struct Shared {
    replay: std::sync::Mutex<ReplayBuffer>,
    child: std::sync::Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    /// 独立 killer：read thread 可在 child.wait() 阻塞，kill 也不争 child 锁。
    killer: std::sync::Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
    stream_gate: std::sync::Mutex<()>,
    /// 最近一次收到 PTY 输出的时刻（毫秒）；0 = 尚无输出。
    /// 读线程在每个 chunk 上更新，供状态检测做静默兜底。
    last_output_ms: AtomicU64,
}

pub struct PtySession {
    pub agent: AgentId,
    pty_master: std::sync::Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    writer: std::sync::Mutex<Box<dyn Write + Send>>,
    tap_tx: broadcast::Sender<bytes::Bytes>,
    shared: Arc<Shared>,
    exit_rx: Mutex<mpsc::Receiver<SessionEvent>>,
    last_input_ms: AtomicU64,
    focused: AtomicBool,
    tmux_server_name: Option<String>,
    tmux_session_name: Option<String>,
}

impl PtySession {
    /// 启动 PTY 子进程；读线程自动开始泵数据到 tap/replay
    pub fn spawn(cfg: LaunchCfg) -> Result<Arc<PtySession>> {
        Self::spawn_with_tmux_socket(cfg, "muxlane")
    }

    fn spawn_with_tmux_socket(cfg: LaunchCfg, tmux_server_name: &str) -> Result<Arc<PtySession>> {
        #[cfg(target_os = "macos")]
        let cfg = {
            let mut cfg = cfg;
            // Existing tmux servers retain the environment from their first launch.
            // Pass the recovered GUI PATH to each new session as well as the client.
            if !cfg.env.iter().any(|(key, _)| key == "PATH") {
                if let Ok(path) = std::env::var("PATH") {
                    cfg.env.push(("PATH".into(), path));
                }
            }
            cfg
        };
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(portable_pty::PtySize {
                rows: cfg.rows,
                cols: cfg.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("openpty")?;

        let (program, args) = match (&cfg.agent_type, &cfg.program_override) {
            (AgentType::Shell, None) => (default_shell_program(), cfg.args.clone()),
            (_, Some(program)) => (program.clone(), cfg.args.clone()),
            (t, None) => (t.program().to_string(), cfg.args.clone()),
        };
        let mut cmd = if let Some(session) = &cfg.tmux_session {
            let tmux_config = tmux_config_path();
            configure_tmux_server(tmux_server_name, &tmux_config);
            update_tmux_environment(tmux_server_name, session, &cfg.env);
            let mut c = CommandBuilder::new("tmux");
            c.arg("-L");
            c.arg(tmux_server_name);
            c.arg("-f");
            c.arg(tmux_config);
            c.arg("new-session");
            c.arg("-A");
            c.arg("-s");
            c.arg(session);
            c.arg("-c");
            c.arg(&cfg.cwd);
            for (k, v) in &cfg.env {
                c.arg("-e");
                c.arg(format!("{k}={v}"));
            }
            c.arg(&program);
            for arg in &args {
                c.arg(arg);
            }
            c
        } else {
            let mut c = CommandBuilder::new(&program);
            for arg in &args {
                c.arg(arg);
            }
            c
        };
        cmd.cwd(&cfg.cwd);
        // portable-pty 默认环境近乎为空，必须显式补齐终端基础环境，否则无颜色
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env(
            "LANG",
            std::env::var("LANG").unwrap_or_else(|_| "en_US.UTF-8".into()),
        );
        // 继承关键会话环境（SSH_AUTH_SOCK 等由调用方通过 cfg.env 追加）
        for k in [
            "HOME",
            "USER",
            "PATH",
            "SSH_AUTH_SOCK",
            "GPG_TTY",
            "DISPLAY",
            "XDG_RUNTIME_DIR",
        ] {
            if let Ok(v) = std::env::var(k) {
                cmd.env(k, v);
            }
        }
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }
        cmd.env("MUXLANE_AGENT_ID", &cfg.agent);

        let child = pair.slave.spawn_command(cmd).context("spawn command")?;
        drop(pair.slave); // 父进程不再需要 slave

        let reader = pair.master.try_clone_reader().context("clone reader")?;
        let writer = pair.master.take_writer().context("take writer")?;

        let (tap_tx, _) = broadcast::channel(BROADCAST_CAP);
        let (exit_tx, exit_rx) = mpsc::channel(8);

        let killer = child.clone_killer();
        let shared = Arc::new(Shared {
            replay: std::sync::Mutex::new(ReplayBuffer::new(REPLAY_CAP)),
            child: std::sync::Mutex::new(child),
            killer: std::sync::Mutex::new(killer),
            stream_gate: std::sync::Mutex::new(()),
            last_output_ms: AtomicU64::new(0),
        });

        let session = Arc::new(PtySession {
            agent: cfg.agent.clone(),
            pty_master: std::sync::Mutex::new(pair.master),
            writer: std::sync::Mutex::new(writer),
            tap_tx: tap_tx.clone(),
            exit_rx: Mutex::new(exit_rx),
            shared: Arc::clone(&shared),
            last_input_ms: AtomicU64::new(0),
            focused: AtomicBool::new(false),
            tmux_server_name: cfg
                .tmux_session
                .as_ref()
                .map(|_| tmux_server_name.to_string()),
            tmux_session_name: cfg.tmux_session.clone(),
        });

        // ── 读线程：PTY → tap broadcast + replay；只持有 shared（不含 MasterPty）
        let agent_id = cfg.agent.clone();
        let tap_tx_reader = tap_tx.clone();
        let shared_ref = Arc::clone(&shared);
        std::thread::Builder::new()
            .name(format!("pty-read-{agent_id}"))
            .spawn(move || {
                let mut reader = BufReader::with_capacity(16 * 1024, reader);
                let mut chunk = vec![0u8; 8192];
                // 读到即发：8KB read 粒度即天然批量，避免小输出滞留聚合缓冲（延迟优先）
                loop {
                    match reader.read(&mut chunk) {
                        Ok(0) => break, // EOF
                        Ok(n) => {
                            shared_ref
                                .last_output_ms
                                .store(now_millis(), Ordering::Relaxed);
                            let bytes = bytes::Bytes::copy_from_slice(&chunk[..n]);
                            // replay push + broadcast send 与 subscribe 原子交接，消除首帧丢字节窗口。
                            if let Ok(_gate) = shared_ref.stream_gate.lock() {
                                if let Ok(mut r) = shared_ref.replay.lock() {
                                    r.push(&bytes[..]);
                                }
                                let _ = tap_tx_reader.send(bytes);
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                let code = shared_ref
                    .child
                    .lock()
                    .ok()
                    .and_then(|mut c| c.wait().ok())
                    .map(|st| st.exit_code());
                let _ = exit_tx.try_send(SessionEvent::Exit { code });
            })
            .context("spawn reader thread")?;

        Ok(session)
    }

    /// 新订阅者：返回（回放快照, 增量流接收端）
    pub fn subscribe(&self) -> (bytes::Bytes, broadcast::Receiver<bytes::Bytes>) {
        let _gate = self.shared.stream_gate.lock().ok();
        // gate 持有期间 reader 不能 publish：receiver 游标与 snapshot 边界完全一致。
        let rx = self.tap_tx.subscribe();
        let snap = self
            .shared
            .replay
            .lock()
            .map(|r| r.snapshot())
            .unwrap_or_default();
        (snap, rx)
    }

    pub fn replay_snapshot(&self) -> bytes::Bytes {
        self.shared
            .replay
            .lock()
            .map(|r| r.snapshot())
            .unwrap_or_default()
    }

    /// 回放尾部 n 字节：锁内只拷尾部，供屏幕采样等热路径使用
    pub fn replay_tail(&self, n: usize) -> bytes::Bytes {
        self.shared
            .replay
            .lock()
            .map(|r| r.tail(n))
            .unwrap_or_default()
    }

    /// 回放快照的 base64（wire 协议直发）
    pub fn replay_b64(&self) -> String {
        let snap = self
            .shared
            .replay
            .lock()
            .map(|r| r.snapshot())
            .unwrap_or_default();
        b64_encode(&snap)
    }

    /// 低延迟同步写输入：一个 mutex 临界区完成 write+flush，
    /// 不为每个按键创建异步任务。
    pub fn write_input(&self, input: &[u8]) {
        if input.is_empty() {
            return;
        }
        self.last_input_ms.store(now_millis(), Ordering::Relaxed);
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(input);
            let _ = w.flush();
        }
    }

    pub fn interaction_recent(&self) -> bool {
        now_millis().saturating_sub(self.last_input_ms.load(Ordering::Relaxed)) <= 75
    }

    /// 距最近一次 PTY 输出的秒数；尚无输出时返回 None。
    pub fn secs_since_output(&self) -> Option<f64> {
        let ms = self.shared.last_output_ms.load(Ordering::Relaxed);
        if ms == 0 {
            return None;
        }
        Some(now_millis().saturating_sub(ms) as f64 / 1000.0)
    }
    pub fn set_focused(&self, focused: bool) {
        self.focused.store(focused, Ordering::Relaxed);
    }
    pub fn is_focused(&self) -> bool {
        self.focused.load(Ordering::Relaxed)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.pty_master
            .lock()
            .map_err(|_| anyhow::anyhow!("pty master lock poisoned"))?
            .resize(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })?;
        Ok(())
    }

    /// 等待退出事件
    pub async fn next_event(&self) -> Option<SessionEvent> {
        let mut rx = self.exit_rx.lock().await;
        rx.recv().await
    }

    /// 非阻塞尝试取退出事件（轮询泵用）
    pub fn try_take_exit(&self) -> Option<SessionEvent> {
        let mut rx = match self.exit_rx.try_lock() {
            Ok(rx) => rx,
            Err(_) => return None,
        };
        rx.try_recv().ok()
    }

    pub fn tmux_session_name(&self) -> Option<&str> {
        self.tmux_session_name.as_deref()
    }

    pub fn kill_persistent(&self) -> bool {
        // 先杀 tmux server-side session，再终止本地 attach client。
        let destroyed =
            if let (Some(server), Some(name)) = (&self.tmux_server_name, &self.tmux_session_name) {
                let target = format!("={name}");
                let _ = std::process::Command::new("tmux")
                    .args(["-L", server, "kill-session", "-t"])
                    .arg(&target)
                    .status();
                std::process::Command::new("tmux")
                    .args(["-L", server, "has-session", "-t"])
                    .arg(&target)
                    .status()
                    .is_ok_and(|status| !status.success())
            } else {
                true
            };
        if destroyed {
            self.kill();
        }
        destroyed
    }

    pub fn kill(&self) {
        if let Ok(mut killer) = self.shared.killer.lock() {
            let _ = killer.kill();
        }
    }

    /// 从后台 tmux 会话捕获历史缓冲区（解决首次/重连 attach 时历史缺失）。
    pub fn capture_history(&self) -> Option<Vec<u8>> {
        let name = self.tmux_session_name.as_deref()?;
        let server = self.tmux_server_name.as_deref()?;
        let target = format!("={name}");
        let output = std::process::Command::new("tmux")
            .args([
                "-L",
                server,
                "capture-pane",
                "-p",
                "-e",
                "-S",
                "-50000",
                "-t",
                &target,
            ])
            .output()
            .ok()?;
        if output.status.success() && !output.stdout.is_empty() {
            Some(output.stdout)
        } else {
            None
        }
    }
}

fn update_tmux_environment(server: &str, session: &str, env: &[(String, String)]) {
    // 单次 tmux 调用批量设置（`\;` 分隔多条命令），避免每变量一次 fork+exec。
    if env.is_empty() {
        return;
    }
    let mut args: Vec<String> = vec!["-L".into(), server.into()];
    for (index, (key, value)) in env.iter().enumerate() {
        if index > 0 {
            args.push(";".into());
        }
        args.push("set-environment".into());
        args.push("-t".into());
        args.push(session.into());
        args.push(key.clone());
        args.push(value.clone());
    }
    let _ = std::process::Command::new("tmux")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// tmux server 全局配置是幂等的且与会话无关：每个 tmux server 每进程只配置一次，
/// 避免每次 spawn/attach 都 fork 8+ 个子进程（启动恢复 N 个会话时放大为 N×13 次）。
/// 仅在全部命令成功后才缓存；首次 spawn 时 server 尚未启动属预期失败，
/// 不缓存、下次 spawn 重试（此时 server 已由 new-session -f 拉起并读取配置）。
fn configure_tmux_server(server: &str, config_path: &Path) {
    use std::sync::{Mutex, OnceLock};
    static CONFIGURED: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    let configured = CONFIGURED.get_or_init(|| Mutex::new(std::collections::HashSet::new()));
    {
        let done = configured.lock().unwrap_or_else(|e| e.into_inner());
        if done.contains(server) {
            return;
        }
    }
    if configure_tmux_server_inner(server, config_path) {
        let mut done = configured.lock().unwrap_or_else(|e| e.into_inner());
        done.insert(server.to_string());
    }
}

/// 返回全部命令是否成功。
fn configure_tmux_server_inner(server: &str, config_path: &Path) -> bool {
    // tmux 接管鼠标：滚轮可浏览历史，并在全屏应用中透传；
    // 按住 Shift 时仍可在前端直接划词复制。
    let mut all_ok = true;
    for (option, value) in [
        ("status", "off"),
        ("mouse", "on"),
        ("extended-keys", "on"),
        ("extended-keys-format", "csi-u"),
        ("history-limit", "50000"),
        ("set-clipboard", "external"),
        ("set-titles", "off"),
    ] {
        let ok = std::process::Command::new("tmux")
            .args(["-L", server, "set-option", "-g", option, value])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        all_ok &= ok;
    }

    // `-f` 只在 server 首次启动时读取；常驻 server 需要主动刷新按键绑定。
    let sourced = std::process::Command::new("tmux")
        .args(["-L", server, "source-file"])
        .arg(config_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    all_ok && sourced
}

fn tmux_config_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("muxlane");
    let _ = std::fs::create_dir_all(&base);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700));
    }
    let path = base.join("tmux.conf");
    const CONFIG: &str = "\
set-option -g status off
set-option -g set-titles off
set-option -g default-terminal \"xterm-256color\"
set-option -g terminal-overrides \",xterm-256color:RGB,tmux-256color:RGB,*-256color:RGB\"
set-option -gu terminal-features
set-option -ga terminal-features \",xterm-256color:RGB:clipboard,tmux-256color:RGB:clipboard,*-256color:RGB:clipboard,*:clipboard\"
set-option -g xterm-keys on
set-option -g history-limit 50000
set-option -g window-size latest
set-option -g mouse on
set-option -g set-clipboard external
set-option -g copy-command \"if command -v pbcopy >/dev/null 2>&1; then pbcopy; elif command -v wl-copy >/dev/null 2>&1; then wl-copy; elif command -v xclip >/dev/null 2>&1; then xclip -selection clipboard; elif command -v xsel >/dev/null 2>&1; then xsel --clipboard --input; else cat >/dev/null; fi\"
set-option -sg escape-time 10
set-environment -gu NO_COLOR
set-environment -g COLORTERM truecolor
set-environment -g CLICOLOR 1
set-environment -g CLICOLOR_FORCE 1
set-environment -g FORCE_COLOR 1
set-window-option -g allow-rename on
set-window-option -g automatic-rename off
set-window-option -g mode-keys vi
bind-key v copy-mode
bind-key -T copy-mode-vi v send-keys -X begin-selection
bind-key -T copy-mode-vi y send-keys -X copy-selection-and-cancel
bind-key -T copy-mode-vi Enter send-keys -X copy-selection-and-cancel
bind-key -T copy-mode-vi Escape send-keys -X cancel
bind-key -n MouseDrag1Pane if-shell -F \"#{||:#{pane_in_mode},#{mouse_any_flag}}\" { send-keys -M } { copy-mode -M }
bind-key -T copy-mode-vi MouseDragEnd1Pane send-keys -X copy-selection-and-cancel
bind-key -T copy-mode MouseDragEnd1Pane send-keys -X copy-selection-and-cancel
bind-key -T copy-mode-vi MouseUp1Pane send-keys -X copy-selection-and-cancel
bind-key -T copy-mode MouseUp1Pane send-keys -X copy-selection-and-cancel
bind-key -T copy-mode-vi WheelDownPane select-pane \\; send-keys -X -N 5 scroll-down-and-cancel
bind-key -T copy-mode WheelDownPane select-pane \\; send-keys -X -N 5 scroll-down-and-cancel
bind-key -T copy-mode-vi DoubleClick1Pane send-keys -X select-word \\; send-keys -X copy-pipe
bind-key -n DoubleClick1Pane select-pane \\; copy-mode -H \\; send-keys -X select-word \\; send-keys -X copy-pipe
bind-key -T copy-mode-vi TripleClick1Pane send-keys -X select-line \\; send-keys -X copy-pipe
bind-key -n TripleClick1Pane select-pane \\; copy-mode -H \\; send-keys -X select-line \\; send-keys -X copy-pipe
";
    if std::fs::read_to_string(&path).ok().as_deref() != Some(CONFIG) {
        let _ = std::fs::write(&path, CONFIG);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
    path
}

fn sanitize_tmux_name(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "_-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct TmuxServerCleanup {
        name: String,
        socket_path: PathBuf,
    }

    impl TmuxServerCleanup {
        fn new(name: String) -> Self {
            let socket_path = std::env::var_os("TMUX_TMPDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(format!("tmux-{}", unsafe { libc_getuid() }))
                .join(&name);
            Self { name, socket_path }
        }
    }

    impl Drop for TmuxServerCleanup {
        fn drop(&mut self) {
            let _ = std::process::Command::new("tmux")
                .args(["-L", &self.name, "kill-server"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }

    #[test]
    fn muxlane_tmux_config_enables_scrollback_without_status_bar() {
        let config = std::fs::read_to_string(tmux_config_path()).unwrap();
        assert!(config.contains("set-option -g mouse on"));
        assert!(config.contains("set-option -g set-titles off"));
        assert!(config.contains("set-option -g status off"));
        assert!(config.contains("set-option -g copy-command"));
        assert!(config.contains("set-option -g xterm-keys on"));
        assert!(config.contains("set-option -g history-limit 50000"));
        assert!(config.contains("copy-mode -M"));
        assert!(config.contains("bind-key -T copy-mode-vi WheelDownPane"));
        assert!(config.contains("bind-key -T copy-mode WheelDownPane"));
        assert_eq!(config.matches("scroll-down-and-cancel").count(), 2);
        assert!(config.contains("MouseDragEnd1Pane"));
        assert!(config.contains("MouseUp1Pane"));
        assert!(config.contains(
            "bind-key -n DoubleClick1Pane select-pane \\; copy-mode -H \\; send-keys -X select-word \\; send-keys -X copy-pipe"
        ));
        assert!(config.contains(
            "bind-key -n TripleClick1Pane select-pane \\; copy-mode -H \\; send-keys -X select-line \\; send-keys -X copy-pipe"
        ));
    }

    #[test]
    fn tmux_config_reloads_idempotently_and_copy_mode_exits_at_bottom() {
        let socket = format!(
            "muxlane-config-test-{}-{}",
            std::process::id(),
            now_millis()
        );
        let config = tmux_config_path();
        let pane = "probe:0.0";

        let started = std::process::Command::new("tmux")
            .args([
                "-L",
                &socket,
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-x",
                "80",
                "-y",
                "24",
                "-s",
                "probe",
                "bash --noprofile --norc",
            ])
            .status()
            .unwrap();
        assert!(started.success());
        let _cleanup = TmuxServerCleanup::new(socket.clone());

        for _ in 0..2 {
            configure_tmux_server(&socket, &config);
        }

        for (event, selection) in [
            ("DoubleClick1Pane", "select-word"),
            ("TripleClick1Pane", "select-line"),
        ] {
            let output = std::process::Command::new("tmux")
                .args(["-L", &socket, "list-keys", "-T", "root"])
                .output()
                .unwrap();
            let bindings = String::from_utf8_lossy(&output.stdout);
            assert!(
                bindings.lines().any(|line| {
                    line.contains(event)
                        && line.contains("select-pane")
                        && line.contains("copy-mode -H")
                        && line.contains(selection)
                        && line.contains("copy-pipe")
                }),
                "missing {event} copy binding in root table: {bindings}"
            );
        }

        for table in ["copy-mode-vi", "copy-mode"] {
            let output = std::process::Command::new("tmux")
                .args(["-L", &socket, "list-keys", "-T", table])
                .output()
                .unwrap();
            let bindings = String::from_utf8_lossy(&output.stdout);
            assert!(
                bindings.lines().any(|line| line.contains("WheelDownPane")
                    && line.contains("scroll-down-and-cancel")),
                "missing WheelDownPane cancel binding in {table}: {bindings}"
            );
        }

        let output = std::process::Command::new("tmux")
            .args(["-L", &socket, "show-options", "-gqv", "terminal-features"])
            .output()
            .unwrap();
        let features = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            features.matches("xterm-256color:RGB:clipboard").count(),
            1,
            "terminal-features should stay idempotent: {features}"
        );

        std::process::Command::new("tmux")
            .args(["-L", &socket, "send-keys", "-t", pane, "seq 1 200", "Enter"])
            .status()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let output = std::process::Command::new("tmux")
                .args([
                    "-L",
                    &socket,
                    "display-message",
                    "-p",
                    "-t",
                    pane,
                    "#{history_size}",
                ])
                .output()
                .unwrap();
            let history_size = String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse::<usize>()
                .unwrap_or(0);
            if history_size >= 10 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for tmux history, size={history_size}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(std::process::Command::new("tmux")
            .args(["-L", &socket, "copy-mode", "-t", pane, "-H"])
            .status()
            .unwrap()
            .success());
        assert!(std::process::Command::new("tmux")
            .args([
                "-L",
                &socket,
                "send-keys",
                "-t",
                pane,
                "-X",
                "-N",
                "10",
                "scroll-up",
            ])
            .status()
            .unwrap()
            .success());

        for expected_mode in ["1", "0"] {
            assert!(std::process::Command::new("tmux")
                .args([
                    "-L",
                    &socket,
                    "send-keys",
                    "-t",
                    pane,
                    "-X",
                    "-N",
                    "5",
                    "scroll-down-and-cancel",
                ])
                .status()
                .unwrap()
                .success());
            let output = std::process::Command::new("tmux")
                .args([
                    "-L",
                    &socket,
                    "display-message",
                    "-p",
                    "-t",
                    pane,
                    "#{pane_in_mode}",
                ])
                .output()
                .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                expected_mode
            );
        }
    }

    #[tokio::test]
    async fn spawn_echo_and_tap() {
        let cfg = LaunchCfg {
            agent: "shell_t1".into(),
            agent_type: AgentType::Shell,
            cwd: std::env::temp_dir(),
            env: vec![],
            program_override: Some("bash".into()),
            args: vec!["-c".into(), "echo hello-muxlane; sleep 0.2".into()],
            cols: 80,
            rows: 24,
            tmux_session: None,
        };
        let s = PtySession::spawn(cfg).unwrap();
        let (snap, mut rx) = s.subscribe();
        assert!(snap.is_empty()); // 起步无历史

        let marker = b"hello-muxlane";
        let mut got = Vec::new();
        for _ in 0..50 {
            match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                Ok(Ok(bytes)) => {
                    got.extend_from_slice(&bytes);
                    if got.windows(marker.len()).any(|window| window == marker) {
                        break;
                    }
                }
                _ => break,
            }
        }
        assert!(
            got.windows(marker.len()).any(|window| window == marker),
            "tap should receive program output"
        );

        let ev = tokio::time::timeout(Duration::from_secs(3), s.next_event()).await;
        assert!(matches!(ev, Ok(Some(SessionEvent::Exit { .. }))));

        let replay = s.replay_b64();
        assert!(!replay.is_empty());
    }

    #[tokio::test]
    async fn multiple_subscribers_same_stream() {
        let cfg = LaunchCfg {
            agent: "shell_t2".into(),
            agent_type: AgentType::Shell,
            cwd: std::env::temp_dir(),
            env: vec![],
            program_override: Some("bash".into()),
            args: vec![
                "-c".into(),
                "for i in 1 2 3; do echo tick-$i; sleep 0.15; done".into(),
            ],
            cols: 80,
            rows: 24,
            tmux_session: None,
        };
        let s = PtySession::spawn(cfg).unwrap();
        let (_s1, mut rx1) = s.subscribe();
        let (_s2, mut rx2) = s.subscribe();

        let mut got1 = Vec::new();
        let mut got2 = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            tokio::select! {
                r = rx1.recv() => if let Ok(b) = r { got1.extend_from_slice(&b); },
                r = rx2.recv() => if let Ok(b) = r { got2.extend_from_slice(&b); },
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
            if got1.windows(6).any(|w| w == b"tick-3") && got2.windows(6).any(|w| w == b"tick-3") {
                break;
            }
        }
        assert!(
            got1.windows(6).any(|w| w == b"tick-3"),
            "sub1 got all (PTY 输出为 tick-3\r\n)"
        );
        assert!(got2.windows(6).any(|w| w == b"tick-3"), "sub2 got all");
        s.kill();
    }

    #[tokio::test]
    async fn write_input_reaches_program() {
        let cfg = LaunchCfg {
            agent: "shell_t3".into(),
            agent_type: AgentType::Shell,
            cwd: std::env::temp_dir(),
            env: vec![],
            program_override: Some("bash".into()),
            args: vec!["-c".into(), "read line; echo GOT:$line".into()],
            cols: 80,
            rows: 24,
            tmux_session: None,
        };
        let s = PtySession::spawn(cfg).unwrap();
        let (_snap, mut rx) = s.subscribe();
        tokio::time::sleep(Duration::from_millis(200)).await;
        s.write_input(b"ping\r\n");
        let mut got = Vec::new();
        for _ in 0..50 {
            match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                Ok(Ok(b)) => {
                    got.extend_from_slice(&b);
                    if got.windows(5).any(|w| w == b"GOT:p") {
                        break;
                    }
                }
                _ => break,
            }
        }
        assert!(
            got.windows(5).any(|w| w == b"GOT:p"),
            "input echoed through program"
        );
        s.kill();
    }

    #[tokio::test]
    async fn replay_covers_late_subscriber() {
        let cfg = LaunchCfg {
            agent: "shell_t4".into(),
            agent_type: AgentType::Shell,
            cwd: std::env::temp_dir(),
            env: vec![],
            program_override: Some("bash".into()),
            args: vec!["-c".into(), "echo early-output; sleep 0.5".into()],
            cols: 80,
            rows: 24,
            tmux_session: None,
        };
        let s = PtySession::spawn(cfg).unwrap();
        // 等第一个程序输出完成
        tokio::time::sleep(Duration::from_millis(400)).await;
        // 晚到的订阅者通过 replay 拿到历史
        let (snap, _rx) = s.subscribe();
        assert!(
            snap.windows(12).any(|w| w == b"early-output"),
            "late subscriber sees replay"
        );
        s.kill();
    }
    #[test]
    fn tmux_session_survives_client_detach_and_reattaches() {
        let socket = format!(
            "muxlane-persist-test-{}-{}",
            std::process::id(),
            now_millis()
        );
        let name = "persist".to_string();
        let cfg = LaunchCfg {
            agent: "persist-a".into(),
            agent_type: AgentType::Shell,
            cwd: std::env::current_dir().unwrap(),
            env: vec![],
            program_override: Some("/bin/sh".into()),
            args: vec![
                "-c".into(),
                "printf 'phase-one\n'; sleep 0.4; printf 'phase-two\n'; exec sleep 5".into(),
            ],
            cols: 80,
            rows: 24,
            tmux_session: Some(name.clone()),
        };
        let first = PtySession::spawn_with_tmux_socket(cfg.clone(), &socket).unwrap();
        let _cleanup = TmuxServerCleanup::new(socket.clone());
        std::thread::sleep(Duration::from_millis(180));
        assert!(String::from_utf8_lossy(&first.replay_snapshot()).contains("phase-one"));
        first.kill();
        std::thread::sleep(Duration::from_millis(500));
        assert!(std::process::Command::new("tmux")
            .args(["-L", &socket, "has-session", "-t", &name])
            .status()
            .unwrap()
            .success());

        let second = PtySession::spawn_with_tmux_socket(cfg, &socket).unwrap();
        std::thread::sleep(Duration::from_millis(180));
        let replay = String::from_utf8_lossy(&second.replay_snapshot()).into_owned();
        second.kill_persistent();
        assert!(replay.contains("phase-two"), "reattached replay={replay:?}");
    }
}
