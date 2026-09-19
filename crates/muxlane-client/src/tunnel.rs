//! SSH 隧道：复用 ControlMaster，把远端 muxlane.sock 转发到本地。
use crate::host::{HostCfg, SshAuth, Target};
use crate::UploadProgress;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum TunnelError {
    #[error("SSH authentication failed: {0}")]
    Authentication(String),
    #[error("Muxlane is not installed; expected socket {remote_socket}")]
    NeedsInstall { remote_socket: String },
    #[error("Muxlane is installed but not running ({binary})")]
    NeedsStart {
        remote_socket: String,
        binary: String,
    },
    #[error("SSH/tunnel failed: {0}")]
    Other(String),
}

fn askpass_script() -> PathBuf {
    let path = muxlane_core::paths::data_dir().join("ssh/muxlane-askpass.sh");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let content = "#!/bin/sh\nsecret=$(cat -- \"$MUXLANE_SSH_SECRET_FILE\" 2>/dev/null) || exit 1\nrm -f -- \"$MUXLANE_SSH_SECRET_FILE\"\nprintf '%s\\n' \"$secret\"\n";
    if std::fs::read_to_string(&path).ok().as_deref() != Some(content) {
        let _ = std::fs::write(&path, content);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
        }
    }
    path
}

fn ssh_command(auth: &SshAuth) -> Command {
    let mut command = match auth {
        SshAuth::Password { password, .. } => {
            let runtime = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("muxlane");
            let _ = std::fs::create_dir_all(&runtime);
            let secret_file = runtime.join(format!(
                "ssh-secret-{}",
                muxlane_core::model::new_id("password")
            ));
            let _ = std::fs::write(&secret_file, password);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&secret_file, std::fs::Permissions::from_mode(0o600));
            }
            let mut command = Command::new("setsid");
            command
                .arg("sh")
                .arg("-c")
                .arg("ssh \"$@\"; status=$?; rm -f -- \"$MUXLANE_SSH_SECRET_FILE\"; exit $status")
                .arg("muxlane-ssh");
            command.env("SSH_ASKPASS", askpass_script());
            command.env("SSH_ASKPASS_REQUIRE", "force");
            command.env("MUXLANE_SSH_SECRET_FILE", secret_file);
            command.env(
                "DISPLAY",
                std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into()),
            );
            command
        }
        _ => Command::new("ssh"),
    };
    command.args([
        "-o",
        "ConnectTimeout=5",
        "-o",
        "NumberOfPasswordPrompts=1",
        // 对端死掉（断网/休眠/宕机）时 ~60s 内让 ssh 主动退出，
        // 否则要靠 TCP 默认 keepalive（约 2 小时）才发现隧道已死。
        "-o",
        "ServerAliveInterval=20",
        "-o",
        "ServerAliveCountMax=3",
    ]);
    match auth {
        SshAuth::SshConfig => {
            command.args(["-o", "BatchMode=yes"]);
        }
        SshAuth::PublicKey { identity_file, .. } => {
            command.args(["-o", "BatchMode=yes"]);
            if let Some(identity) = identity_file
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                let identity = if let Some(rest) = identity.strip_prefix("~/") {
                    std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .unwrap_or_default()
                        .join(rest)
                        .display()
                        .to_string()
                } else {
                    identity.to_string()
                };
                command
                    .arg("-o")
                    .arg("IdentitiesOnly=yes")
                    .arg("-i")
                    .arg(identity);
            }
        }
        SshAuth::Password { .. } => {
            command.args([
                "-o",
                "BatchMode=no",
                "-o",
                "PreferredAuthentications=password,keyboard-interactive",
            ]);
        }
    }
    command
}

/// exec 类 ssh 调用（probe/upload/start）共享的每台主机一条 ControlMaster。
/// 只有第一次调用做 TCP+认证握手，后续复用。
/// 注意：隧道进程（start_tunnel）不能用这个 socket——`-L` 转发只在 master
/// 连接上生效，复用已有 master 时转发会被静默丢弃。
fn shared_ctl(destination: &str) -> PathBuf {
    let dir = muxlane_core::paths::data_dir().join("ssh");
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!("cm-{}.ctl", sanitize(destination)))
}

fn shared_master_args(destination: &str) -> [String; 6] {
    [
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        "ControlPersist=600".into(),
        "-o".into(),
        format!("ControlPath={}", shared_ctl(destination).display()),
    ]
}

fn classify_failure(stderr: &[u8]) -> TunnelError {
    let message = String::from_utf8_lossy(stderr).trim().to_string();
    if message.contains("Permission denied") || message.contains("Authentication failed") {
        TunnelError::Authentication(message)
    } else {
        TunnelError::Other(if message.is_empty() {
            "unknown SSH failure".into()
        } else {
            message
        })
    }
}

#[derive(Debug, Clone)]
struct TunnelEntry {
    local: String,
    control_path: PathBuf,
    destination: String,
}

/// 隧道表：host → 精确 local/control/destination。
static TUNNELS: LazyLock<Mutex<Option<std::collections::HashMap<String, TunnelEntry>>>> =
    LazyLock::new(|| Mutex::new(None));

pub async fn release_tunnel(host: &str) {
    if let Some(entry) = TUNNELS
        .lock()
        .await
        .as_mut()
        .and_then(|tunnels| tunnels.remove(host))
    {
        let _ = std::fs::remove_file(entry.local);
        let _ = Command::new("ssh")
            .arg("-S")
            .arg(&entry.control_path)
            .args(["-O", "exit"])
            .arg(&entry.destination)
            .output()
            .await;
        let _ = std::fs::remove_file(entry.control_path);
        // 一并关掉该主机的共享 exec master，断开即不再保留已认证连接。
        let shared = shared_ctl(&entry.destination);
        let _ = Command::new("ssh")
            .arg("-S")
            .arg(&shared)
            .args(["-O", "exit"])
            .arg(&entry.destination)
            .output()
            .await;
        let _ = std::fs::remove_file(shared);
    }
}

/// 确保到 host 的隧道，返回本地可连的 unix socket 路径。
pub async fn ensure_tunnel(
    host: &str,
    remote_socket: &str,
    auth: &SshAuth,
) -> Result<String, TunnelError> {
    let destination = auth.destination(host);
    let discovered;
    let remote_socket = if remote_socket.trim().is_empty() {
        discovered = match probe_remote(host, auth).await? {
            RemoteProbe::Ready { remote_socket } => remote_socket,
        };
        discovered.as_str()
    } else {
        remote_socket
    };
    let cached = {
        let guard = TUNNELS.lock().await;
        guard.as_ref().and_then(|map| map.get(host).cloned())
    };
    if let Some(entry) = cached {
        if wait_healthy(std::path::Path::new(&entry.local)).await {
            return Ok(entry.local);
        }
        release_tunnel(host).await;
    }
    let entry = start_tunnel(&destination, remote_socket, auth).await?;
    let local = entry.local.clone();
    TUNNELS
        .lock()
        .await
        .get_or_insert_with(Default::default)
        .insert(host.to_string(), entry);
    Ok(local)
}

#[derive(Debug, Clone)]
pub enum RemoteProbe {
    Ready { remote_socket: String },
}

fn parse_probe_output(line: &str) -> Result<RemoteProbe, TunnelError> {
    let fields: Vec<_> = line.trim().split('\t').collect();
    match fields.as_slice() {
        ["READY", socket] => Ok(RemoteProbe::Ready {
            remote_socket: (*socket).into(),
        }),
        ["STOPPED", socket, binary] => Err(TunnelError::NeedsStart {
            remote_socket: (*socket).into(),
            binary: (*binary).into(),
        }),
        ["MISSING", socket] => Err(TunnelError::NeedsInstall {
            remote_socket: (*socket).into(),
        }),
        _ => Err(TunnelError::Other(format!(
            "invalid remote probe: {}",
            line.trim()
        ))),
    }
}

/// SSH 成功后区分 ready / missing / stopped，不再全部折叠为 offline。
pub async fn probe_remote(host: &str, auth: &SshAuth) -> Result<RemoteProbe, TunnelError> {
    let destination = auth.destination(host);
    let script = r#"data=${XDG_DATA_HOME:-$HOME/.local/share}/muxlane
socket=$data/muxlane.sock
lock=$data/muxlane.lock
managed=$data/bin/muxlane
running=0
if [ -S "$socket" ] && command -v flock >/dev/null 2>&1 && ! flock -n "$lock" -c true 2>/dev/null; then running=1; fi
if [ "$running" -eq 1 ]; then printf 'READY\t%s\n' "$socket"
elif command -v muxlane >/dev/null 2>&1; then printf 'STOPPED\t%s\t%s\n' "$socket" "$(command -v muxlane)"
elif [ -x "$managed" ]; then printf 'STOPPED\t%s\t%s\n' "$socket" "$managed"
else printf 'MISSING\t%s\n' "$socket"
fi"#;
    let output = ssh_command(auth)
        .args(shared_master_args(&destination))
        .arg(&destination)
        .arg(script)
        .output()
        .await
        .map_err(|error| TunnelError::Other(error.to_string()))?;
    if !output.status.success() {
        return Err(classify_failure(&output.stderr));
    }
    let line = String::from_utf8_lossy(&output.stdout);
    parse_probe_output(&line)
}

/// 小文件直传（剪贴板图片粘贴到远端）：stdin `cat` 到远端路径，复用 ControlMaster。
pub async fn upload_bytes(
    cfg: &HostCfg,
    bytes: &[u8],
    remote_path: &str,
) -> Result<(), TunnelError> {
    let host = match &cfg.target {
        Target::Ssh { host, .. } => host.clone(),
        Target::Socket(_) => {
            return Err(TunnelError::Other(
                "direct socket target cannot receive uploads".into(),
            ))
        }
    };
    let destination = cfg.auth.destination(&host);
    let script = format!("cat > {}", sh_quote(remote_path));
    let mut child = ssh_command(&cfg.auth)
        .args(shared_master_args(&destination))
        .arg(&destination)
        .arg(&script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| TunnelError::Other(error.to_string()))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| TunnelError::Other("SSH upload stdin unavailable".into()))?;
    stdin
        .write_all(bytes)
        .await
        .map_err(|error| TunnelError::Other(error.to_string()))?;
    stdin
        .flush()
        .await
        .map_err(|error| TunnelError::Other(error.to_string()))?;
    drop(stdin);
    let output = child
        .wait_with_output()
        .await
        .map_err(|error| TunnelError::Other(error.to_string()))?;
    if !output.status.success() {
        return Err(classify_failure(&output.stderr));
    }
    Ok(())
}

async fn upload_binary(
    host: &str,
    auth: &SshAuth,
    on_progress: impl Fn(UploadProgress) + Send + Sync + 'static,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), TunnelError> {
    let destination = auth.destination(host);
    let binary = if let Some(path) = std::env::var_os("MUXLANE_BOOTSTRAP_BINARY") {
        PathBuf::from(path)
    } else {
        let current =
            std::env::current_exe().map_err(|error| TunnelError::Other(error.to_string()))?;
        let release = current
            .parent()
            .and_then(|debug| debug.parent())
            .map(|target| target.join("release/muxlane"));
        release.filter(|path| path.is_file()).unwrap_or(current)
    };
    let mut raw_bytes = tokio::fs::read(&binary)
        .await
        .map_err(|error| TunnelError::Other(error.to_string()))?;

    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(TunnelError::Other("已取消上传".into()));
    }

    // 如果二进制大于 50MB (例如未 strip 的 debug 产物)，尝试就地 strip 去掉调试符号
    if raw_bytes.len() > 50 * 1024 * 1024 {
        if let Ok(temp_file) = tempfile::NamedTempFile::new() {
            let temp_path = temp_file.path();
            if tokio::fs::write(temp_path, &raw_bytes).await.is_ok() {
                if let Ok(status) = std::process::Command::new("strip")
                    .arg("-s")
                    .arg(temp_path)
                    .status()
                {
                    if status.success() {
                        if let Ok(stripped) = tokio::fs::read(temp_path).await {
                            if !stripped.is_empty() {
                                raw_bytes = stripped;
                            }
                        }
                    }
                }
            }
        }
    }

    // 采用 Gzip 高效压缩流传输
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut encoder, &raw_bytes)
        .map_err(|e| TunnelError::Other(format!("Compression error: {e}")))?;
    let compressed_bytes = encoder
        .finish()
        .map_err(|e| TunnelError::Other(format!("Compression error: {e}")))?;

    let script = r#"set -eu
data=${XDG_DATA_HOME:-$HOME/.local/share}/muxlane
mkdir -p "$data/bin" "$data/logs"
tmp=$data/bin/.muxlane-upload-$$
if command -v gzip >/dev/null 2>&1; then
  gzip -dc > "$tmp"
elif command -v gunzip >/dev/null 2>&1; then
  gunzip -c > "$tmp"
else
  cat > "$tmp"
fi
chmod 700 "$tmp"
"$tmp" --version >/dev/null
mv "$tmp" "$data/bin/muxlane""#;

    let mut child = ssh_command(auth)
        .args(shared_master_args(&destination))
        .arg(&destination)
        .arg(script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| TunnelError::Other(error.to_string()))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| TunnelError::Other("SSH upload stdin unavailable".into()))?;

    let total = compressed_bytes.len().max(1);
    let mut done: u64 = 0;
    let mut last_percent: Option<u8> = None;
    const CHUNK: usize = 128 * 1024;
    for chunk in compressed_bytes.chunks(CHUNK) {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = child.kill().await;
            return Err(TunnelError::Other("已取消上传".into()));
        }
        stdin
            .write_all(chunk)
            .await
            .map_err(|error| TunnelError::Other(error.to_string()))?;
        done += chunk.len() as u64;
        // 按百分比变化节流，避免每个 128KB 块都发一次事件
        let percent = Some(((done.min(total as u64) * 100) / total as u64) as u8);
        if percent != last_percent {
            last_percent = percent;
            on_progress(UploadProgress {
                done: done.min(total as u64),
                total: total as u64,
            });
        }
    }
    on_progress(UploadProgress {
        done: total as u64,
        total: total as u64,
    });
    stdin
        .flush()
        .await
        .map_err(|error| TunnelError::Other(error.to_string()))?;
    drop(stdin);

    let output = child
        .wait_with_output()
        .await
        .map_err(|error| TunnelError::Other(error.to_string()))?;
    if !output.status.success() {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(TunnelError::Other("已取消上传".into()));
        }
        return Err(classify_failure(&output.stderr));
    }
    Ok(())
}

pub async fn install_and_start(
    host: &str,
    auth: &SshAuth,
    on_upload_progress: impl Fn(UploadProgress) + Send + Sync + 'static,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), TunnelError> {
    upload_binary(host, auth, on_upload_progress, cancel).await?;
    start_remote(host, auth, None).await
}

pub async fn install_and_restart(
    host: &str,
    auth: &SshAuth,
    on_upload_progress: impl Fn(UploadProgress) + Send + Sync + 'static,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), TunnelError> {
    upload_binary(host, auth, on_upload_progress, cancel).await?;
    let destination = auth.destination(host);
    let command = r#"data=${XDG_DATA_HOME:-$HOME/.local/share}/muxlane
if command -v fuser >/dev/null 2>&1; then
  fuser -k -TERM "$data/muxlane.lock" >/dev/null 2>&1 || true
else
  pkill -TERM -x muxlane 2>/dev/null || true
fi
stopped=0
for i in 1 2 3 4 5 6 7 8 9 10; do
  if flock -n "$data/muxlane.lock" -c true 2>/dev/null; then stopped=1; break; fi
  sleep .2
done
if [ "$stopped" -eq 0 ]; then
  if command -v fuser >/dev/null 2>&1; then
    fuser -k -KILL "$data/muxlane.lock" >/dev/null 2>&1 || true
  else
    pkill -KILL -x muxlane 2>/dev/null || true
  fi
  for i in 1 2 3 4 5; do
    if flock -n "$data/muxlane.lock" -c true 2>/dev/null; then break; fi
    sleep .2
  done
fi
rm -f -- "$data/muxlane.sock""#;
    let output = ssh_command(auth)
        .args(shared_master_args(&destination))
        .arg(&destination)
        .arg(command)
        .output()
        .await
        .map_err(|error| TunnelError::Other(error.to_string()))?;
    if !output.status.success() {
        return Err(classify_failure(&output.stderr));
    }
    start_remote(host, auth, None).await
}

pub async fn start_remote(
    host: &str,
    auth: &SshAuth,
    binary: Option<&str>,
) -> Result<(), TunnelError> {
    let destination = auth.destination(host);
    let explicit = binary.unwrap_or("");
    let command = format!(
        r#"set -eu
data=${{XDG_DATA_HOME:-$HOME/.local/share}}/muxlane
socket=$data/muxlane.sock
lock=$data/muxlane.lock
bin={}
if [ -z "$bin" ]; then bin=$data/bin/muxlane; fi
mkdir -p "$data/logs"
running=0
if [ -S "$socket" ] && command -v flock >/dev/null 2>&1 && ! flock -n "$lock" -c true 2>/dev/null; then running=1; fi
if [ "$running" -eq 0 ]; then
  rm -f -- "$socket"
  nohup "$bin" --headless </dev/null >>"$data/logs/headless.log" 2>&1 &
fi
for i in 1 2 3 4 5 6 7 8 9 10; do
  if [ -S "$socket" ] && ! flock -n "$lock" -c true 2>/dev/null; then exit 0; fi
  sleep .2
done
printf 'Muxlane headless did not create %s\n' "$socket" >&2
exit 1"#,
        sh_quote(explicit)
    );
    let output = ssh_command(auth)
        .args(shared_master_args(&destination))
        .arg(&destination)
        .arg(command)
        .output()
        .await
        .map_err(|error| TunnelError::Other(error.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(classify_failure(&output.stderr))
    }
}

/// 启动一条隧道：
///   ssh -M -S <ctl> -fN -L <local_sock>:<remote_sock> <host>   (OpenSSH ≥8.3 支持 unix 转发)
/// 失败则回退：远端 socat 转 TCP + 本地 socat TCP→unix
async fn start_tunnel(
    host: &str,
    remote_socket: &str,
    auth: &SshAuth,
) -> Result<TunnelEntry, TunnelError> {
    let dir = muxlane_core::paths::data_dir();
    std::fs::create_dir_all(dir.join("ssh")).ok();
    let id = muxlane_core::model::new_id("tunnel");
    let ctl = dir.join("ssh").join(format!("{id}.ctl"));
    let local_sock = dir.join("ssh").join(format!("{id}-fwd.sock"));
    let _ = std::fs::remove_file(&local_sock);

    // 方案 A：OpenSSH 原生 StreamLocalForward: -L local_socket:remote_socket。
    let out = ssh_command(auth)
        .args(["-S"])
        .arg(&ctl)
        .args([
            "-fN",
            "-o",
            "ControlMaster=auto",
            "-o",
            "ControlPersist=600",
            "-o",
            "StreamLocalBindUnlink=yes",
            "-o",
            "ConnectTimeout=5",
        ])
        .arg("-L")
        .arg(streamlocal_spec(&local_sock, remote_socket))
        .arg(host)
        .output()
        .await;

    if out.as_ref().is_ok_and(|o| o.status.success()) && wait_healthy(&local_sock).await {
        return Ok(TunnelEntry {
            local: local_sock.display().to_string(),
            control_path: ctl,
            destination: host.to_string(),
        });
    }
    match out {
        Ok(output) => Err(classify_failure(&output.stderr)),
        Err(error) => Err(TunnelError::Other(error.to_string())),
    }
}

async fn wait_healthy(socket: &std::path::Path) -> bool {
    for _ in 0..30 {
        if tokio::net::UnixStream::connect(socket).await.is_ok() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    false
}

fn sh_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

fn streamlocal_spec(local: &std::path::Path, remote: &str) -> String {
    format!("{}:{}", local.display(), remote)
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_probe_distinguishes_install_and_start() {
        assert!(matches!(
            parse_probe_output("READY\t/run/user/1000/muxlane.sock\n"),
            Ok(RemoteProbe::Ready { .. })
        ));
        assert!(matches!(
            parse_probe_output("MISSING\t/home/u/.local/share/muxlane/muxlane.sock\n"),
            Err(TunnelError::NeedsInstall { .. })
        ));
        assert!(matches!(
            parse_probe_output("STOPPED\t/tmp/muxlane.sock\t/usr/bin/muxlane\n"),
            Err(TunnelError::NeedsStart { .. })
        ));
    }

    #[test]
    fn askpass_script_never_contains_a_password() {
        let script = std::fs::read_to_string(askpass_script()).unwrap();
        assert!(script.contains("MUXLANE_SSH_SECRET_FILE"));
        assert!(!script.contains("super-secret"));
    }

    #[test]
    fn shell_quote_handles_apostrophes() {
        assert_eq!(sh_quote("/tmp/a'b"), "'/tmp/a'\\''b'");
    }

    #[test]
    fn shell_quote_blocks_injection() {
        assert_eq!(sh_quote("/tmp/a b;rm"), "'/tmp/a b;rm'");
    }

    #[test]
    fn native_streamlocal_spec_is_local_colon_remote() {
        assert_eq!(
            streamlocal_spec(std::path::Path::new("/tmp/local.sock"), "/run/remote.sock"),
            "/tmp/local.sock:/run/remote.sock"
        );
    }

    #[test]
    fn shared_master_uses_sanitized_per_host_control_path() {
        let args = shared_master_args("user@host.example");
        let rendered = args.join(" ");
        assert!(rendered.contains("ControlMaster=auto"));
        assert!(rendered.contains("ControlPersist=600"));
        assert!(rendered.contains("cm-user_host.example.ctl"));
        // 同主机两次调用拿到同一路径（才能复用 master）
        assert_eq!(
            shared_ctl("user@host.example"),
            shared_ctl("user@host.example")
        );
    }
}
