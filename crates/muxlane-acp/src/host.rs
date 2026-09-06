use agent_client_protocol::schema::v1;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncReadExt as _;
use tokio::process::Child;
use tokio::sync::Mutex;

const DEFAULT_OUTPUT_LIMIT: usize = 1024 * 1024;
const MAX_OUTPUT_LIMIT: usize = 10 * 1024 * 1024;
const MAX_FILE_READ_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct HostServices {
    root: Arc<PathBuf>,
    terminals: Arc<Mutex<HashMap<String, HostTerminal>>>,
}

struct HostTerminal {
    child: Arc<Mutex<Child>>,
    output: Arc<Mutex<OutputBuffer>>,
    exit_status: Arc<Mutex<Option<v1::TerminalExitStatus>>>,
    readers: Arc<std::sync::atomic::AtomicUsize>,
}

struct OutputBuffer {
    bytes: VecDeque<u8>,
    limit: usize,
    truncated: bool,
}

impl OutputBuffer {
    fn new(limit: usize) -> Self {
        Self {
            bytes: VecDeque::new(),
            limit,
            truncated: false,
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        self.bytes.extend(bytes.iter().copied());
        if self.bytes.len() > self.limit {
            let remove = self.bytes.len() - self.limit;
            self.bytes.drain(..remove);
            self.truncated = true;
        }
    }

    fn text(&self) -> String {
        let bytes: Vec<_> = self.bytes.iter().copied().collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

impl HostServices {
    pub(crate) fn new(root: &Path) -> Result<Self, agent_client_protocol::Error> {
        let root = std::fs::canonicalize(root).map_err(internal_error)?;
        Ok(Self {
            root: Arc::new(root),
            terminals: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub(crate) async fn read_text_file(
        &self,
        request: v1::ReadTextFileRequest,
    ) -> Result<v1::ReadTextFileResponse, agent_client_protocol::Error> {
        let path = secure_existing_path(&self.root, &request.path)?;
        let metadata = tokio::fs::metadata(&path).await.map_err(internal_error)?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_READ_BYTES as u64 {
            return Err(invalid_request("file is not readable or exceeds 2 MiB"));
        }
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(internal_error)?;
        let start = request.line.unwrap_or(1).saturating_sub(1) as usize;
        let limit = request.limit.map(|value| value as usize);
        let content = content
            .lines()
            .skip(start)
            .take(limit.unwrap_or(usize::MAX))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(v1::ReadTextFileResponse::new(content))
    }

    pub(crate) async fn write_text_file(
        &self,
        request: v1::WriteTextFileRequest,
    ) -> Result<v1::WriteTextFileResponse, agent_client_protocol::Error> {
        let path = secure_write_path(&self.root, &request.path)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(internal_error)?;
        }
        let temporary = path.with_extension(format!("muxlane-acp-{}.tmp", ulid::Ulid::new()));
        tokio::fs::write(&temporary, request.content)
            .await
            .map_err(internal_error)?;
        if let Err(error) = tokio::fs::rename(&temporary, &path).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(internal_error(error));
        }
        Ok(v1::WriteTextFileResponse::new())
    }

    pub(crate) async fn create_terminal(
        &self,
        request: v1::CreateTerminalRequest,
    ) -> Result<v1::CreateTerminalResponse, agent_client_protocol::Error> {
        let cwd = match request.cwd {
            Some(cwd) => secure_existing_path(&self.root, &cwd)?,
            None => self.root.as_ref().clone(),
        };
        if !cwd.is_dir() {
            return Err(invalid_request("terminal cwd is not a directory"));
        }
        let limit = request
            .output_byte_limit
            .and_then(|limit| usize::try_from(limit).ok())
            .unwrap_or(DEFAULT_OUTPUT_LIMIT)
            .clamp(1, MAX_OUTPUT_LIMIT);
        let mut command = tokio::process::Command::new(request.command);
        command
            .args(request.args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        for variable in request.env {
            command.env(variable.name, variable.value);
        }
        let mut child = command.spawn().map_err(internal_error)?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let output = Arc::new(Mutex::new(OutputBuffer::new(limit)));
        let readers = Arc::new(std::sync::atomic::AtomicUsize::new(
            usize::from(stdout.is_some()) + usize::from(stderr.is_some()),
        ));
        if let Some(stdout) = stdout {
            read_output(stdout, output.clone(), readers.clone());
        }
        if let Some(stderr) = stderr {
            read_output(stderr, output.clone(), readers.clone());
        }
        let id = format!("terminal_{}", ulid::Ulid::new());
        let child = Arc::new(Mutex::new(child));
        let exit_status = Arc::new(Mutex::new(None));
        self.terminals.lock().await.insert(
            id.clone(),
            HostTerminal {
                child,
                output,
                exit_status,
                readers,
            },
        );
        Ok(v1::CreateTerminalResponse::new(id))
    }

    pub(crate) async fn terminal_output_if_present(
        &self,
        request: v1::TerminalOutputRequest,
    ) -> Result<Option<v1::TerminalOutputResponse>, agent_client_protocol::Error> {
        let Some(terminal) = self
            .terminals
            .lock()
            .await
            .get(&request.terminal_id.to_string())
            .map(|terminal| HostTerminal {
                child: terminal.child.clone(),
                output: terminal.output.clone(),
                exit_status: terminal.exit_status.clone(),
                readers: terminal.readers.clone(),
            })
        else {
            return Ok(None);
        };
        refresh_exit_status(&terminal).await?;
        let exit_status = terminal.exit_status.lock().await.clone();
        let output = terminal.output.lock().await;
        Ok(Some(
            v1::TerminalOutputResponse::new(output.text(), output.truncated)
                .exit_status(exit_status),
        ))
    }

    pub(crate) async fn terminal_output(
        &self,
        request: v1::TerminalOutputRequest,
    ) -> Result<v1::TerminalOutputResponse, agent_client_protocol::Error> {
        let terminal = self.terminal(&request.terminal_id.to_string()).await?;
        refresh_exit_status(&terminal).await?;
        let exit_status = terminal.exit_status.lock().await.clone();
        let output = terminal.output.lock().await;
        Ok(
            v1::TerminalOutputResponse::new(output.text(), output.truncated)
                .exit_status(exit_status),
        )
    }

    pub(crate) async fn wait_for_terminal_exit(
        &self,
        request: v1::WaitForTerminalExitRequest,
    ) -> Result<v1::WaitForTerminalExitResponse, agent_client_protocol::Error> {
        let terminal = self.terminal(&request.terminal_id.to_string()).await?;
        let mut drain_attempts = 0;
        loop {
            refresh_exit_status(&terminal).await?;
            if let Some(status) = terminal.exit_status.lock().await.clone() {
                if terminal.readers.load(std::sync::atomic::Ordering::Acquire) == 0
                    || drain_attempts >= 4
                {
                    return Ok(v1::WaitForTerminalExitResponse::new(status));
                }
                drain_attempts += 1;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    pub(crate) async fn kill_terminal(
        &self,
        request: v1::KillTerminalRequest,
    ) -> Result<v1::KillTerminalResponse, agent_client_protocol::Error> {
        let terminal = self.terminal(&request.terminal_id.to_string()).await?;
        terminal
            .child
            .lock()
            .await
            .start_kill()
            .map_err(internal_error)?;
        Ok(v1::KillTerminalResponse::new())
    }

    pub(crate) async fn release_terminal(
        &self,
        request: v1::ReleaseTerminalRequest,
    ) -> Result<v1::ReleaseTerminalResponse, agent_client_protocol::Error> {
        let terminal = self
            .terminals
            .lock()
            .await
            .remove(&request.terminal_id.to_string())
            .ok_or_else(|| invalid_request("unknown terminal"))?;
        let mut child = terminal.child.lock().await;
        if child.try_wait().map_err(internal_error)?.is_none() {
            child.start_kill().map_err(internal_error)?;
        }
        Ok(v1::ReleaseTerminalResponse::new())
    }

    async fn terminal(&self, id: &str) -> Result<HostTerminal, agent_client_protocol::Error> {
        self.terminals
            .lock()
            .await
            .get(id)
            .map(|terminal| HostTerminal {
                child: terminal.child.clone(),
                output: terminal.output.clone(),
                exit_status: terminal.exit_status.clone(),
                readers: terminal.readers.clone(),
            })
            .ok_or_else(|| invalid_request("unknown terminal"))
    }

    pub(crate) async fn shutdown(&self) {
        let terminals = std::mem::take(&mut *self.terminals.lock().await);
        for (_, terminal) in terminals {
            let mut child = terminal.child.lock().await;
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.start_kill();
            }
        }
    }
}

async fn refresh_exit_status(terminal: &HostTerminal) -> Result<(), agent_client_protocol::Error> {
    if terminal.exit_status.lock().await.is_some() {
        return Ok(());
    }
    let status = terminal
        .child
        .lock()
        .await
        .try_wait()
        .map_err(internal_error)?;
    if let Some(status) = status {
        *terminal.exit_status.lock().await = Some(exit_status(status));
    }
    Ok(())
}

fn read_output<R>(
    mut reader: R,
    output: Arc<Mutex<OutputBuffer>>,
    readers: Arc<std::sync::atomic::AtomicUsize>,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(length) => output.lock().await.append(&buffer[..length]),
            }
        }
        readers.fetch_sub(1, std::sync::atomic::Ordering::Release);
    });
}

fn secure_existing_path(root: &Path, path: &Path) -> Result<PathBuf, agent_client_protocol::Error> {
    if !path.is_absolute() {
        return Err(invalid_request("path must be absolute"));
    }
    let path = std::fs::canonicalize(path).map_err(internal_error)?;
    if path == root || path.starts_with(root) {
        Ok(path)
    } else {
        Err(invalid_request("path escapes the project root"))
    }
}

fn secure_write_path(root: &Path, path: &Path) -> Result<PathBuf, agent_client_protocol::Error> {
    if !path.is_absolute() {
        return Err(invalid_request("path must be absolute"));
    }
    if path.exists() {
        return secure_existing_path(root, path);
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid_request("path has no parent"))?;
    let parent = std::fs::canonicalize(parent).map_err(internal_error)?;
    if parent == root || parent.starts_with(root) {
        Ok(parent.join(
            path.file_name()
                .ok_or_else(|| invalid_request("invalid file name"))?,
        ))
    } else {
        Err(invalid_request("path escapes the project root"))
    }
}

fn exit_status(status: std::process::ExitStatus) -> v1::TerminalExitStatus {
    let code = status.code().and_then(|code| u32::try_from(code).ok());
    v1::TerminalExitStatus::new().exit_code(code)
}

fn invalid_request(message: impl Into<String>) -> agent_client_protocol::Error {
    agent_client_protocol::Error::invalid_request().data(message.into())
}

fn internal_error(error: impl std::fmt::Display) -> agent_client_protocol::Error {
    agent_client_protocol::Error::internal_error().data(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_buffer_keeps_latest_bytes() {
        let mut output = OutputBuffer::new(3);
        output.append(b"ab");
        output.append(b"cdef");

        assert_eq!(output.text(), "def");
        assert!(output.truncated);
    }
    #[tokio::test]
    async fn released_terminal_is_absent_from_best_effort_output_poll() {
        let directory = tempfile::tempdir().unwrap();
        let host = HostServices::new(directory.path()).unwrap();
        let created = host
            .create_terminal(v1::CreateTerminalRequest::new("session", "/bin/sh"))
            .await
            .unwrap();
        let terminal_id = created.terminal_id.clone();
        host.release_terminal(v1::ReleaseTerminalRequest::new(
            "session",
            terminal_id.clone(),
        ))
        .await
        .unwrap();

        assert!(host
            .terminal_output_if_present(v1::TerminalOutputRequest::new(
                "session",
                terminal_id.clone(),
            ))
            .await
            .unwrap()
            .is_none());
        let error = host
            .terminal_output(v1::TerminalOutputRequest::new("session", terminal_id))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "Invalid request: \"unknown terminal\"");
    }

    #[tokio::test]
    async fn file_access_stays_inside_project_and_terminal_captures_output() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("file.txt");
        std::fs::write(&file, "one\ntwo\nthree").unwrap();
        let host = HostServices::new(&root).unwrap();
        let read = host
            .read_text_file(
                v1::ReadTextFileRequest::new("session", &file)
                    .line(2)
                    .limit(1),
            )
            .await
            .unwrap();
        assert_eq!(read.content, "two");
        assert!(host
            .read_text_file(v1::ReadTextFileRequest::new("session", "/etc/passwd"))
            .await
            .is_err());

        let created = host
            .create_terminal(
                v1::CreateTerminalRequest::new("session", "/bin/sh")
                    .args(vec!["-c".into(), "printf terminal-output".into()])
                    .cwd(root.clone()),
            )
            .await
            .unwrap();
        host.wait_for_terminal_exit(v1::WaitForTerminalExitRequest::new(
            "session",
            created.terminal_id.clone(),
        ))
        .await
        .unwrap();
        let output = host
            .terminal_output(v1::TerminalOutputRequest::new(
                "session",
                created.terminal_id,
            ))
            .await
            .unwrap();
        assert_eq!(output.output, "terminal-output");

        let inherited_pipe = host
            .create_terminal(
                v1::CreateTerminalRequest::new("session", "/bin/sh")
                    .args(vec!["-c".into(), "sleep 2 &".into()])
                    .cwd(root),
            )
            .await
            .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_millis(500),
            host.wait_for_terminal_exit(v1::WaitForTerminalExitRequest::new(
                "session",
                inherited_pipe.terminal_id,
            )),
        )
        .await
        .expect("wait must not depend indefinitely on inherited output pipes")
        .unwrap();
    }
}
