#![forbid(unsafe_code)]

mod host;
mod thread;

use host::HostServices;

use agent_client_protocol::{
    schema::{v1, ProtocolVersion},
    util::MatchDispatch,
    AcpAgent, Agent, Client, ConnectionTo, Dispatch, SessionMessage,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};
use tokio::sync::{mpsc, oneshot, Mutex};

pub use thread::{
    AvailableCommand, ConfigChoice, ConfigKind, ConfigOption, ConfigValue, ContentItem, Cost,
    Message, MessageRole, Modes, Plan, PlanEntry, SessionMode, Thought, ThreadChange, ThreadDelta,
    ThreadItem, ThreadReducer, ThreadSnapshot, Tool, ToolContent, ToolKind, ToolLocation,
    ToolState, Usage,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptBlock {
    Text(String),
    Image {
        data: String,
        mime_type: String,
    },
    Audio {
        data: String,
        mime_type: String,
    },
    Resource {
        name: String,
        uri: String,
        mime_type: Option<String>,
        text: String,
    },
    ResourceLink {
        name: String,
        uri: String,
        mime_type: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PromptId(String);

impl PromptId {
    pub fn new() -> Self {
        Self(ulid::Ulid::new().to_string())
    }
}

impl Default for PromptId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for PromptId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromptSubmission {
    pub id: PromptId,
    pub payload: PromptPayload,
}

impl PromptSubmission {
    pub fn new(payload: PromptPayload) -> Self {
        Self {
            id: PromptId::new(),
            payload,
        }
    }

    pub fn with_id(id: PromptId, payload: PromptPayload) -> Self {
        Self { id, payload }
    }
}

impl<'de> Deserialize<'de> for PromptSubmission {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Current {
            #[serde(default)]
            id: PromptId,
            payload: PromptPayload,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Current(Current),
            Legacy(PromptPayload),
        }

        match Wire::deserialize(deserializer)? {
            Wire::Current(current) => Ok(Self {
                id: current.id,
                payload: current.payload,
            }),
            Wire::Legacy(payload) => Ok(Self::new(payload)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptPayload {
    pub text: String,
    pub blocks: Vec<PromptBlock>,
}

impl PromptPayload {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            blocks: Vec::new(),
        }
    }

    pub fn with_block(mut self, block: PromptBlock) -> Self {
        self.blocks.push(block);
        self
    }

    pub fn into_content_blocks(self) -> Vec<v1::ContentBlock> {
        let mut blocks = vec![v1::ContentBlock::Text(v1::TextContent::new(self.text))];
        blocks.extend(self.blocks.into_iter().map(project_prompt_block));
        blocks
    }
}

fn project_prompt_block(block: PromptBlock) -> v1::ContentBlock {
    match block {
        PromptBlock::Text(text) => v1::ContentBlock::Text(v1::TextContent::new(text)),
        PromptBlock::Image { data, mime_type } => {
            v1::ContentBlock::Image(v1::ImageContent::new(data, mime_type))
        }
        PromptBlock::Audio { data, mime_type } => {
            v1::ContentBlock::Audio(v1::AudioContent::new(data, mime_type))
        }
        PromptBlock::Resource {
            uri,
            mime_type,
            text,
            ..
        } => {
            let resource = v1::TextResourceContents::new(text, uri).mime_type(mime_type);
            v1::ContentBlock::Resource(v1::EmbeddedResource::new(
                v1::EmbeddedResourceResource::TextResourceContents(resource),
            ))
        }
        PromptBlock::ResourceLink {
            name,
            uri,
            mime_type,
        } => v1::ContentBlock::ResourceLink(v1::ResourceLink::new(name, uri).mime_type(mime_type)),
    }
}

pub fn project_prompt(payload: PromptPayload) -> Vec<v1::ContentBlock> {
    payload.into_content_blocks()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    Claude,
    Codex,
    Pi,
}

impl Profile {
    pub fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Pi => "pi",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Pi => "Pi",
        }
    }

    pub fn command(self) -> &'static str {
        match self {
            Self::Claude => "npx -y @agentclientprotocol/claude-agent-acp@latest",
            Self::Codex => "npx -y @agentclientprotocol/codex-acp@latest",
            Self::Pi => "npx pi-acp",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "pi" => Some(Self::Pi),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Connecting,
    Idle,
    Generating,
    Permission,
    Failed,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionPhase {
    Connecting,
    Connected,
    Failed,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnState {
    Idle,
    Generating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKind {
    Connection,
    Authentication,
    Recovery,
    Request,
    Protocol,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionError {
    pub kind: ErrorKind,
    pub message: String,
}

impl SessionError {
    fn request(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Request,
            message: message.into(),
        }
    }

    fn worker(error: impl ToString, recovery: bool) -> Self {
        let message = error.to_string();
        let lower = message.to_ascii_lowercase();
        let kind = if lower.contains("auth") || lower.contains("unauthorized") {
            ErrorKind::Authentication
        } else if recovery {
            ErrorKind::Recovery
        } else {
            ErrorKind::Connection
        };
        Self { kind, message }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCapabilities {
    pub image: bool,
    pub audio: bool,
    pub embedded_context: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCapabilities {
    pub list: bool,
    pub delete: bool,
    pub resume: bool,
    pub close: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthMethod {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub terminal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub load_session: bool,
    pub prompt: PromptCapabilities,
    pub sessions: SessionCapabilities,
    pub auth_methods: Vec<AuthMethod>,
    pub logout: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionOption {
    pub id: String,
    pub label: String,
    pub kind: PermissionKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ElicitationMode {
    Form { schema: Value },
    Url { id: String, url: String },
    Unsupported,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ElicitationRequest {
    pub id: String,
    pub message: String,
    pub mode: ElicitationMode,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ElicitationResponse {
    Accept(serde_json::Map<String, Value>),
    Decline,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSnapshot {
    pub id: String,
    pub output: String,
    pub truncated: bool,
    pub exit_code: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Event {
    Ready {
        protocol_session_id: String,
        restored: bool,
        capabilities: Capabilities,
        modes: Option<Modes>,
        config_options: Vec<ConfigOption>,
    },
    Capabilities(Capabilities),
    Authenticated,
    LoggedOut,
    SessionList(Vec<SessionSummary>),
    Connection(ConnectionPhase),
    Turn(TurnState),
    PromptAccepted {
        id: PromptId,
    },
    PromptRejected {
        id: PromptId,
        error: SessionError,
    },
    Delta(ThreadDelta),
    Permission {
        id: String,
        title: String,
        options: Vec<PermissionOption>,
    },
    Elicitation(ElicitationRequest),
    TerminalOutput(TerminalSnapshot),
    Error(SessionError),
}

#[derive(Debug)]
enum Command {
    Prompt(PromptSubmission),
    Cancel,
    Permission {
        id: String,
        option: Option<String>,
    },
    Elicitation {
        id: String,
        response: ElicitationResponse,
    },
    PollTerminal(String),
    Logout,
    ListSessions,
    CloseSession(String),
    DeleteSession(String),
    SetMode(String),
    SetConfig {
        id: String,
        value: ConfigValue,
    },
    Shutdown,
}

#[derive(Clone)]
pub struct AcpHandle {
    tx: mpsc::UnboundedSender<Command>,
}

impl AcpHandle {
    pub fn submit_submission(&self, submission: PromptSubmission) -> Result<PromptId> {
        let id = submission.id.clone();
        self.tx
            .send(Command::Prompt(submission))
            .context("ACP worker stopped")?;
        Ok(id)
    }

    pub fn cancel(&self) -> Result<()> {
        self.tx
            .send(Command::Cancel)
            .context("ACP worker stopped")?;
        Ok(())
    }

    pub fn choose_permission(&self, id: impl Into<String>, option: Option<String>) -> Result<()> {
        self.tx
            .send(Command::Permission {
                id: id.into(),
                option,
            })
            .context("ACP worker stopped")?;
        Ok(())
    }

    pub fn respond_elicitation(
        &self,
        id: impl Into<String>,
        response: ElicitationResponse,
    ) -> Result<()> {
        self.tx
            .send(Command::Elicitation {
                id: id.into(),
                response,
            })
            .context("ACP worker stopped")?;
        Ok(())
    }

    pub fn poll_terminal(&self, id: impl Into<String>) -> Result<()> {
        self.tx
            .send(Command::PollTerminal(id.into()))
            .context("ACP worker stopped")?;
        Ok(())
    }

    pub fn logout(&self) -> Result<()> {
        self.tx
            .send(Command::Logout)
            .context("ACP worker stopped")?;
        Ok(())
    }

    pub fn list_sessions(&self) -> Result<()> {
        self.tx
            .send(Command::ListSessions)
            .context("ACP worker stopped")?;
        Ok(())
    }

    pub fn close_session(&self, id: impl Into<String>) -> Result<()> {
        self.tx
            .send(Command::CloseSession(id.into()))
            .context("ACP worker stopped")?;
        Ok(())
    }

    pub fn delete_session(&self, id: impl Into<String>) -> Result<()> {
        self.tx
            .send(Command::DeleteSession(id.into()))
            .context("ACP worker stopped")?;
        Ok(())
    }

    pub fn set_mode(&self, id: impl Into<String>) -> Result<()> {
        self.tx
            .send(Command::SetMode(id.into()))
            .context("ACP worker stopped")?;
        Ok(())
    }

    pub fn set_config_option(&self, id: impl Into<String>, value: ConfigValue) -> Result<()> {
        self.tx
            .send(Command::SetConfig {
                id: id.into(),
                value,
            })
            .context("ACP worker stopped")?;
        Ok(())
    }

    pub fn shutdown(&self) {
        if self.tx.send(Command::Shutdown).is_err() {
            tracing::debug!("ACP worker already stopped during shutdown");
        }
    }
}

pub struct ActiveSession {
    pub handle: AcpHandle,
    pub events: mpsc::UnboundedReceiver<Event>,
}

pub fn spawn_on(
    runtime: &tokio::runtime::Handle,
    profile: Profile,
    cwd: impl AsRef<Path>,
    protocol_session_id: Option<String>,
) -> ActiveSession {
    spawn_on_with_auth(runtime, profile, cwd, protocol_session_id, None)
}

pub fn spawn_on_with_auth(
    runtime: &tokio::runtime::Handle,
    profile: Profile,
    cwd: impl AsRef<Path>,
    protocol_session_id: Option<String>,
    auth_method: Option<String>,
) -> ActiveSession {
    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let (event_tx, events) = mpsc::unbounded_channel();
    let agent = match profile_agent(profile) {
        Ok(agent) => agent,
        Err(error) => {
            let _ = event_tx.send(Event::Connection(ConnectionPhase::Failed));
            let _ = event_tx.send(Event::Error(SessionError::worker(error, false)));
            let _ = event_tx.send(Event::Connection(ConnectionPhase::Disconnected));
            return ActiveSession {
                handle: AcpHandle { tx: command_tx },
                events,
            };
        }
    };
    runtime.spawn(run(
        agent,
        cwd.as_ref().to_path_buf(),
        protocol_session_id,
        auth_method,
        command_rx,
        event_tx,
    ));
    ActiveSession {
        handle: AcpHandle { tx: command_tx },
        events,
    }
}

fn profile_agent(profile: Profile) -> Result<AcpAgent> {
    let override_name = match profile {
        Profile::Claude => "MUXLANE_ACP_CLAUDE_COMMAND",
        Profile::Codex => "MUXLANE_ACP_CODEX_COMMAND",
        Profile::Pi => "MUXLANE_ACP_PI_COMMAND",
    };
    if let Ok(command) = std::env::var(override_name) {
        match AcpAgent::from_str(&command) {
            Ok(agent) => return Ok(agent),
            Err(error) => {
                tracing::warn!(%error, variable = override_name, "invalid ACP command override");
            }
        }
    }
    AcpAgent::from_str(profile.command()).context("invalid default ACP command")
}

pub fn spawn(
    profile: Profile,
    cwd: impl AsRef<Path>,
    protocol_session_id: Option<String>,
) -> ActiveSession {
    spawn_on(
        &tokio::runtime::Handle::current(),
        profile,
        cwd,
        protocol_session_id,
    )
}

type PermissionWaiters = Arc<Mutex<HashMap<String, oneshot::Sender<Option<String>>>>>;
type ElicitationWaiters = Arc<Mutex<HashMap<String, oneshot::Sender<ElicitationResponse>>>>;

async fn run(
    agent: AcpAgent,
    cwd: PathBuf,
    protocol_session_id: Option<String>,
    auth_method: Option<String>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<Event>,
) {
    let _ = events.send(Event::Connection(ConnectionPhase::Connecting));
    let host = match HostServices::new(&cwd) {
        Ok(host) => host,
        Err(error) => {
            let _ = events.send(Event::Connection(ConnectionPhase::Failed));
            let _ = events.send(Event::Error(SessionError::worker(error, false)));
            let _ = events.send(Event::Connection(ConnectionPhase::Disconnected));
            return;
        }
    };
    let waiters: PermissionWaiters = Default::default();
    let elicitation_waiters: ElicitationWaiters = Default::default();
    let permission_events = events.clone();
    let permission_waiters = waiters.clone();
    let elicitation_events = events.clone();
    let elicitation_waiters_for_handler = elicitation_waiters.clone();
    let read_host = host.clone();
    let write_host = host.clone();
    let create_terminal_host = host.clone();
    let output_terminal_host = host.clone();
    let wait_terminal_host = host.clone();
    let kill_terminal_host = host.clone();
    let release_terminal_host = host.clone();
    let session_host = host.clone();
    let session_waiters = waiters.clone();
    let session_elicitation_waiters = elicitation_waiters.clone();
    let session_events = events.clone();
    let recovery_attempted = protocol_session_id.is_some();
    let result = Client
        .builder()
        .on_receive_request(
            move |request: v1::ReadTextFileRequest,
                  responder: agent_client_protocol::Responder<v1::ReadTextFileResponse>,
                  _cx: ConnectionTo<Agent>| {
                let host = read_host.clone();
                async move { respond_result(responder, host.read_text_file(request).await) }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            move |request: v1::WriteTextFileRequest,
                  responder: agent_client_protocol::Responder<v1::WriteTextFileResponse>,
                  _cx: ConnectionTo<Agent>| {
                let host = write_host.clone();
                async move { respond_result(responder, host.write_text_file(request).await) }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            move |request: v1::CreateTerminalRequest,
                  responder: agent_client_protocol::Responder<v1::CreateTerminalResponse>,
                  _cx: ConnectionTo<Agent>| {
                let host = create_terminal_host.clone();
                async move { respond_result(responder, host.create_terminal(request).await) }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            move |request: v1::TerminalOutputRequest,
                  responder: agent_client_protocol::Responder<v1::TerminalOutputResponse>,
                  _cx: ConnectionTo<Agent>| {
                let host = output_terminal_host.clone();
                async move { respond_result(responder, host.terminal_output(request).await) }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            move |request: v1::WaitForTerminalExitRequest,
                  responder: agent_client_protocol::Responder<v1::WaitForTerminalExitResponse>,
                  _cx: ConnectionTo<Agent>| {
                let host = wait_terminal_host.clone();
                async move { respond_result(responder, host.wait_for_terminal_exit(request).await) }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            move |request: v1::KillTerminalRequest,
                  responder: agent_client_protocol::Responder<v1::KillTerminalResponse>,
                  _cx: ConnectionTo<Agent>| {
                let host = kill_terminal_host.clone();
                async move { respond_result(responder, host.kill_terminal(request).await) }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            move |request: v1::ReleaseTerminalRequest,
                  responder: agent_client_protocol::Responder<v1::ReleaseTerminalResponse>,
                  _cx: ConnectionTo<Agent>| {
                let host = release_terminal_host.clone();
                async move { respond_result(responder, host.release_terminal(request).await) }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            move |request: v1::RequestPermissionRequest,
                  responder: agent_client_protocol::Responder<v1::RequestPermissionResponse>,
                  _cx: ConnectionTo<Agent>| {
                let events = permission_events.clone();
                let waiters = permission_waiters.clone();
                async move {
                    let id = ulid::Ulid::new().to_string();
                    let options = request
                        .options
                        .iter()
                        .map(|option| PermissionOption {
                            id: option.option_id.to_string(),
                            label: option.name.clone(),
                            kind: match option.kind {
                                v1::PermissionOptionKind::AllowOnce => PermissionKind::AllowOnce,
                                v1::PermissionOptionKind::AllowAlways => PermissionKind::AllowAlways,
                                v1::PermissionOptionKind::RejectOnce => PermissionKind::RejectOnce,
                                v1::PermissionOptionKind::RejectAlways => PermissionKind::RejectAlways,
                                _ => PermissionKind::Unknown,
                            },
                        })
                        .collect();
                    let title = request
                        .tool_call
                        .fields
                        .title
                        .clone()
                        .unwrap_or_else(|| "Permission required".into());
                    let (tx, rx) = oneshot::channel();
                    waiters.lock().await.insert(id.clone(), tx);
                    if events
                        .send(Event::Permission {
                            id: id.clone(),
                            title,
                            options,
                        })
                        .is_err()
                    {
                        if let Some(waiter) = waiters.lock().await.remove(&id) {
                            let _ = waiter.send(None);
                        }
                    }
                    let selected = rx.await.unwrap_or(None);
                    let outcome = selected
                        .map(|id| {
                            v1::RequestPermissionOutcome::Selected(
                                v1::SelectedPermissionOutcome::new(id),
                            )
                        })
                        .unwrap_or(v1::RequestPermissionOutcome::Cancelled);
                    responder.respond(v1::RequestPermissionResponse::new(outcome))
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            move |request: v1::CreateElicitationRequest,
                  responder: agent_client_protocol::Responder<v1::CreateElicitationResponse>,
                  _cx: ConnectionTo<Agent>| {
                let events = elicitation_events.clone();
                let waiters = elicitation_waiters_for_handler.clone();
                async move {
                    let id = ulid::Ulid::new().to_string();
                    let mode = match request.mode {
                        v1::ElicitationMode::Form(form) => ElicitationMode::Form {
                            schema: serde_json::to_value(form.requested_schema)
                                .unwrap_or(Value::Null),
                        },
                        v1::ElicitationMode::Url(url) => ElicitationMode::Url {
                            id: url.elicitation_id.to_string(),
                            url: url.url,
                        },
                        _ => ElicitationMode::Unsupported,
                    };
                    let (tx, rx) = oneshot::channel();
                    waiters.lock().await.insert(id.clone(), tx);
                    if events
                        .send(Event::Elicitation(ElicitationRequest {
                            id: id.clone(),
                            message: request.message,
                            mode,
                        }))
                        .is_err()
                    {
                        if let Some(waiter) = waiters.lock().await.remove(&id) {
                            let _ = waiter.send(ElicitationResponse::Cancel);
                        }
                    }
                    let response = rx.await.unwrap_or(ElicitationResponse::Cancel);
                    respond_result(responder, project_elicitation_response(response))
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, move |connection: ConnectionTo<Agent>| async move {
            let client_capabilities = v1::ClientCapabilities::new()
                .fs(
                    v1::FileSystemCapabilities::new()
                        .read_text_file(true)
                        .write_text_file(true),
                )
                .terminal(true)
                .elicitation(
                v1::ElicitationCapabilities::new()
                    .form(v1::ElicitationFormCapabilities::new())
                    .url(v1::ElicitationUrlCapabilities::new()),
            );
            let init = connection
                .send_request(
                    v1::InitializeRequest::new(ProtocolVersion::V1)
                        .client_capabilities(client_capabilities),
                )
                .block_task()
                .await?;
            let capabilities = project_capabilities(&init);
            let _ = session_events.send(Event::Capabilities(capabilities.clone()));
            if let Some(method_id) = auth_method {
                connection
                    .send_request(v1::AuthenticateRequest::new(method_id))
                    .block_task()
                    .await?;
                let _ = session_events.send(Event::Authenticated);
            }
            let can_load = capabilities.load_session;
            let can_resume = capabilities.sessions.resume;
            let (mut session, restored, modes, config_options) =
                if let Some(id) = protocol_session_id.as_ref().filter(|_| can_load) {
                    let restored = connection
                        .load_session(id.clone(), &cwd)
                        .block_task()
                        .start_session()
                        .await?;
                    let modes = restored.response().modes.clone();
                    let config_options = restored
                        .response()
                        .config_options
                        .clone()
                        .unwrap_or_default();
                    (restored.into_session(), true, modes, config_options)
                } else if let Some(id) = protocol_session_id.as_ref().filter(|_| can_resume) {
                    let resumed = connection
                        .resume_session(id.clone(), &cwd)
                        .block_task()
                        .start_session()
                        .await?;
                    let modes = resumed.response().modes.clone();
                    let config_options = resumed
                        .response()
                        .config_options
                        .clone()
                        .unwrap_or_default();
                    (resumed.into_session(), true, modes, config_options)
                } else if protocol_session_id.is_some() {
                    return Err(agent_client_protocol::Error::invalid_request().data(
                        "ACP agent does not advertise session/load or session/resume; saved session cannot be restored",
                    ));
                } else {
                    let created = connection
                        .build_session(&cwd)
                        .block_task()
                        .start_session()
                        .await?;
                    let modes = created.response().modes;
                    let config_options = created.response().config_options.unwrap_or_default();
                    (created, false, modes, config_options)
                };
            let session_id = session.session_id().to_string();
            let session_connection = session.connection().clone();
            let _ = session_events.send(Event::Ready {
                protocol_session_id: session_id,
                restored,
                capabilities,
                modes: modes.map(project_modes),
                config_options: config_options.iter().map(project_config).collect(),
            });
            let _ = session_events.send(Event::Connection(ConnectionPhase::Connected));
            let _ = session_events.send(Event::Turn(TurnState::Idle));

            let mut generating = false;
            let (finished_tx, mut finished_rx) = mpsc::unbounded_channel::<Result<(), String>>();
            let finished_sender = finished_tx.clone();
            loop {
                tokio::select! {
                    command = commands.recv() => match command {
                        Some(Command::Prompt(submission)) if !generating => {
                            let id = submission.id.clone();
                            let text = submission.payload.text.clone();
                            let blocks = submission.payload.into_content_blocks();
                            let finished_sender = finished_sender.clone();
                            let sent = session_connection.send_request_to(
                                Agent,
                                v1::PromptRequest::new(session.session_id().clone(), blocks),
                            );
                            let callback_sender = finished_sender.clone();
                            match session_connection.spawn(async move {
                                let result = sent
                                    .block_task()
                                    .await
                                    .map(|_| ())
                                    .map_err(|error| error.to_string());
                                if callback_sender.send(result).is_err() {
                                    tracing::debug!("ACP prompt completion receiver dropped");
                                }
                                Ok(())
                            }) {
                                Ok(()) => {
                                    generating = true;
                                    let _ = session_events.send(Event::PromptAccepted { id });
                                    let _ = session_events.send(Event::Delta(ThreadDelta::MessageChunk { id: None, role: MessageRole::User, text }));
                                    let _ = session_events.send(Event::Turn(TurnState::Generating));
                                }
                                Err(error) => {
                                    let _ = session_events.send(Event::PromptRejected {
                                        id,
                                        error: SessionError::worker(error, false),
                                    });
                                }
                            }
                        }
                        Some(Command::Prompt(submission)) => {
                            let _ = session_events.send(Event::PromptRejected {
                                id: submission.id,
                                error: SessionError::request("A prompt is already running"),
                            });
                        }
                        Some(Command::Cancel) => {
                            if let Err(error) = session_connection.send_notification_to(
                                Agent,
                                v1::CancelNotification::new(session.session_id().clone()),
                            ) {
                                let _ = session_events.send(Event::Error(SessionError::request(error.to_string())));
                            }
                        }
                        Some(Command::Permission { id, option }) => {
                            resolve_permission(&session_waiters, &id, option).await;
                        }
                        Some(Command::Elicitation { id, response }) => {
                            resolve_elicitation(&session_elicitation_waiters, &id, response).await;
                        }
                        Some(Command::PollTerminal(id)) => {
                            match session_host
                                .terminal_output_if_present(v1::TerminalOutputRequest::new(
                                    session.session_id().clone(),
                                    id.clone(),
                                ))
                                .await
                            {
                                Ok(Some(output)) => {
                                    let _ = session_events.send(Event::TerminalOutput(
                                        TerminalSnapshot {
                                            id,
                                            output: output.output,
                                            truncated: output.truncated,
                                            exit_code: output
                                                .exit_status
                                                .and_then(|status| status.exit_code),
                                        },
                                    ));
                                }
                                Ok(None) => {
                                    tracing::debug!(terminal_id = %id, "skipping poll for released terminal");
                                }
                                Err(error) => {
                                    let _ = session_events.send(Event::Error(
                                        SessionError::request(error.to_string()),
                                    ));
                                }
                            }
                        }
                        Some(Command::Logout) => {
                            let connection = session_connection.clone();
                            let events = session_events.clone();
                            if let Err(error) = session_connection.spawn(async move {
                                match connection
                                    .send_request_to(Agent, v1::LogoutRequest::new())
                                    .block_task()
                                    .await
                                {
                                    Ok(_) => {
                                        let _ = events.send(Event::LoggedOut);
                                    }
                                    Err(error) => {
                                        let _ = events.send(Event::Error(
                                            SessionError::request(error.to_string()),
                                        ));
                                    }
                                }
                                Ok(())
                            }) {
                                let _ = session_events.send(Event::Error(
                                    SessionError::request(error.to_string()),
                                ));
                            }
                        }
                        Some(Command::ListSessions) => {
                            let connection = session_connection.clone();
                            let events = session_events.clone();
                            let cwd = cwd.clone();
                            if let Err(error) = session_connection.spawn(async move {
                                let mut cursor = None;
                                let mut seen_cursors = std::collections::HashSet::new();
                                let mut sessions = Vec::new();
                                for _ in 0..100 {
                                    let request = v1::ListSessionsRequest::new()
                                        .cwd(cwd.clone())
                                        .cursor(cursor.clone());
                                    match connection
                                        .send_request_to(Agent, request)
                                        .block_task()
                                        .await
                                    {
                                        Ok(response) => {
                                            sessions.extend(response.sessions.into_iter().map(|item| {
                                                SessionSummary {
                                                    id: item.session_id.to_string(),
                                                    title: item.title.unwrap_or_else(|| "Agent Thread".into()),
                                                    cwd: item.cwd,
                                                    updated_at: item.updated_at,
                                                }
                                            }));
                                            let Some(next_cursor) = response.next_cursor else {
                                                break;
                                            };
                                            if !seen_cursors.insert(next_cursor.to_string()) {
                                                let _ = events.send(Event::Error(SessionError::request(
                                                    "agent returned a repeated session/list cursor",
                                                )));
                                                break;
                                            }
                                            cursor = Some(next_cursor);
                                        }
                                        Err(error) => {
                                            let _ = events.send(Event::Error(
                                                SessionError::request(error.to_string()),
                                            ));
                                            break;
                                        }
                                    }
                                }
                                let _ = events.send(Event::SessionList(sessions));
                                Ok(())
                            }) {
                                let _ = session_events.send(Event::Error(
                                    SessionError::request(error.to_string()),
                                ));
                            }
                        }
                        Some(Command::CloseSession(id)) => {
                            let connection = session_connection.clone();
                            let events = session_events.clone();
                            if let Err(error) = session_connection.spawn(async move {
                                if let Err(error) = connection
                                    .send_request_to(Agent, v1::CloseSessionRequest::new(id))
                                    .block_task()
                                    .await
                                {
                                    let _ = events.send(Event::Error(
                                        SessionError::request(error.to_string()),
                                    ));
                                }
                                Ok(())
                            }) {
                                let _ = session_events.send(Event::Error(
                                    SessionError::request(error.to_string()),
                                ));
                            }
                        }
                        Some(Command::DeleteSession(id)) => {
                            let connection = session_connection.clone();
                            let events = session_events.clone();
                            if let Err(error) = session_connection.spawn(async move {
                                if let Err(error) = connection
                                    .send_request_to(Agent, v1::DeleteSessionRequest::new(id))
                                    .block_task()
                                    .await
                                {
                                    let _ = events.send(Event::Error(
                                        SessionError::request(error.to_string()),
                                    ));
                                }
                                Ok(())
                            }) {
                                let _ = session_events.send(Event::Error(
                                    SessionError::request(error.to_string()),
                                ));
                            }
                        }
                        Some(Command::SetMode(id)) => {
                            let connection = session_connection.clone();
                            let events = session_events.clone();
                            let mode_id = id.clone();
                            let request =
                                v1::SetSessionModeRequest::new(session.session_id().clone(), id);
                            if let Err(error) = session_connection.spawn(async move {
                                match connection
                                    .send_request_to(Agent, request)
                                    .block_task()
                                    .await
                                {
                                    Ok(_) => {
                                        let _ = events.send(Event::Delta(ThreadDelta::Modes(
                                            Modes {
                                                current: mode_id,
                                                available: Vec::new(),
                                            },
                                        )));
                                    }
                                    Err(error) => {
                                        let _ = events.send(Event::Error(SessionError::request(
                                            error.to_string(),
                                        )));
                                    }
                                }
                                Ok(())
                            }) {
                                let _ = session_events.send(Event::Error(
                                    SessionError::request(error.to_string()),
                                ));
                            }
                        }
                        Some(Command::SetConfig { id, value }) => {
                            let value = match value {
                                ConfigValue::Select(value) => {
                                    v1::SessionConfigOptionValue::value_id(value)
                                }
                                ConfigValue::Boolean(value) => {
                                    v1::SessionConfigOptionValue::boolean(value)
                                }
                            };
                            let request = v1::SetSessionConfigOptionRequest::new(
                                session.session_id().clone(),
                                id,
                                value,
                            );
                            let connection = session_connection.clone();
                            let events = session_events.clone();
                            if let Err(error) = session_connection.spawn(async move {
                                match connection
                                    .send_request_to(Agent, request)
                                    .block_task()
                                    .await
                                {
                                    Ok(response) => {
                                        let _ = events.send(Event::Delta(
                                            ThreadDelta::ConfigOptions(
                                                response
                                                    .config_options
                                                    .iter()
                                                    .map(project_config)
                                                    .collect(),
                                            ),
                                        ));
                                    }
                                    Err(error) => {
                                        let _ = events.send(Event::Error(SessionError::request(
                                            error.to_string(),
                                        )));
                                    }
                                }
                                Ok(())
                            }) {
                                let _ = session_events.send(Event::Error(
                                    SessionError::request(error.to_string()),
                                ));
                            }
                        }
                        Some(Command::Shutdown) | None => {
                            cancel_pending_permissions(&session_waiters).await;
                            cancel_pending_elicitations(&session_elicitation_waiters).await;
                            session_host.shutdown().await;
                            return Ok(());
                        }
                    },
                    finished = finished_rx.recv() => match finished {
                        Some(Ok(())) => {
                            if generating {
                                generating = false;
                                let _ = session_events.send(Event::Turn(TurnState::Idle));
                            }
                        }
                        Some(Err(error)) => {
                            let was_generating = generating;
                            generating = false;
                            let _ = session_events.send(Event::Error(SessionError::request(error)));
                            if was_generating {
                                let _ = session_events.send(Event::Turn(TurnState::Idle));
                            }
                        }
                        None => return Ok(()),
                    },
                    update = session.read_update() => match update? {
                        SessionMessage::StopReason(_) => {
                            if generating {
                                generating = false;
                                let _ = session_events.send(Event::Turn(TurnState::Idle));
                            }
                        }
                        SessionMessage::SessionMessage(dispatch) => {
                            project_dispatch(dispatch, &session_events).await?;
                        }
                        _ => tracing::debug!("unknown ACP session message"),
                    }
                }
            }
        })
        .await;

    cancel_pending_permissions(&waiters).await;
    cancel_pending_elicitations(&elicitation_waiters).await;
    host.shutdown().await;
    if let Err(error) = result {
        let _ = events.send(Event::Connection(ConnectionPhase::Failed));
        let _ = events.send(Event::Error(SessionError::worker(
            error,
            recovery_attempted,
        )));
    }
    let _ = events.send(Event::Connection(ConnectionPhase::Disconnected));
}

fn respond_result<T: agent_client_protocol::JsonRpcResponse>(
    responder: agent_client_protocol::Responder<T>,
    result: Result<T, agent_client_protocol::Error>,
) -> Result<(), agent_client_protocol::Error> {
    match result {
        Ok(response) => responder.respond(response),
        Err(error) => responder.respond_with_error(error),
    }
}

async fn resolve_permission(waiters: &PermissionWaiters, id: &str, option: Option<String>) {
    if let Some(waiter) = waiters.lock().await.remove(id) {
        let _ = waiter.send(option);
    }
}

async fn cancel_pending_permissions(waiters: &PermissionWaiters) {
    let pending = std::mem::take(&mut *waiters.lock().await);
    for (_, waiter) in pending {
        let _ = waiter.send(None);
    }
}

async fn resolve_elicitation(
    waiters: &ElicitationWaiters,
    id: &str,
    response: ElicitationResponse,
) {
    if let Some(waiter) = waiters.lock().await.remove(id) {
        let _ = waiter.send(response);
    }
}

async fn cancel_pending_elicitations(waiters: &ElicitationWaiters) {
    let pending = std::mem::take(&mut *waiters.lock().await);
    for (_, waiter) in pending {
        let _ = waiter.send(ElicitationResponse::Cancel);
    }
}

fn project_elicitation_response(
    response: ElicitationResponse,
) -> Result<v1::CreateElicitationResponse, agent_client_protocol::Error> {
    let action = match response {
        ElicitationResponse::Accept(values) => {
            let values = values
                .into_iter()
                .map(|(name, value)| project_elicitation_value(value).map(|value| (name, value)))
                .collect::<Result<std::collections::BTreeMap<_, _>, _>>()?;
            v1::ElicitationAction::Accept(v1::ElicitationAcceptAction::new().content(values))
        }
        ElicitationResponse::Decline => v1::ElicitationAction::Decline,
        ElicitationResponse::Cancel => v1::ElicitationAction::Cancel,
    };
    Ok(v1::CreateElicitationResponse::new(action))
}

fn project_elicitation_value(
    value: Value,
) -> Result<v1::ElicitationContentValue, agent_client_protocol::Error> {
    match value {
        Value::String(value) => Ok(v1::ElicitationContentValue::String(value)),
        Value::Number(value) => value
            .as_i64()
            .map(v1::ElicitationContentValue::Integer)
            .or_else(|| value.as_f64().map(v1::ElicitationContentValue::Number))
            .ok_or_else(|| agent_client_protocol::Error::invalid_params().data("invalid number")),
        Value::Bool(value) => Ok(v1::ElicitationContentValue::Boolean(value)),
        Value::Array(values) => values
            .into_iter()
            .map(|value| value.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .map(v1::ElicitationContentValue::StringArray)
            .ok_or_else(|| {
                agent_client_protocol::Error::invalid_params()
                    .data("elicitation arrays must contain only strings")
            }),
        Value::Null | Value::Object(_) => Err(agent_client_protocol::Error::invalid_params()
            .data("unsupported elicitation value type")),
    }
}

async fn project_dispatch(
    dispatch: Dispatch,
    events: &mpsc::UnboundedSender<Event>,
) -> agent_client_protocol::Result<()> {
    MatchDispatch::new(dispatch)
        .if_notification(async |notification: v1::SessionNotification| {
            for event in project_update(notification.update) {
                let _ = events.send(event);
            }
            Ok::<(), agent_client_protocol::Error>(())
        })
        .await
        .otherwise_ignore()
}

pub fn project_update(update: v1::SessionUpdate) -> Vec<Event> {
    use v1::SessionUpdate;

    match update {
        SessionUpdate::UserMessageChunk(chunk) => chunk_delta(chunk, MessageRole::User),
        SessionUpdate::AgentMessageChunk(chunk) => chunk_delta(chunk, MessageRole::Assistant),
        SessionUpdate::AgentThoughtChunk(chunk) => thought_delta(chunk),
        SessionUpdate::ToolCall(tool) => vec![Event::Delta(ThreadDelta::ToolUpsert {
            id: tool.tool_call_id.to_string(),
            title: Some(tool.title),
            kind: Some(project_tool_kind(tool.kind)),
            state: Some(project_tool_state(tool.status)),
            content: Some(project_tool_content(tool.content)),
            locations: Some(project_tool_locations(tool.locations)),
            raw_input: tool.raw_input,
            raw_output: tool.raw_output,
            subagent_session_id: project_subagent_session_id(&tool.meta),
        })],
        SessionUpdate::ToolCallUpdate(tool) => vec![Event::Delta(ThreadDelta::ToolUpsert {
            id: tool.tool_call_id.to_string(),
            title: tool.fields.title,
            kind: tool.fields.kind.map(project_tool_kind),
            state: tool.fields.status.map(project_tool_state),
            content: tool.fields.content.map(project_tool_content),
            locations: tool.fields.locations.map(project_tool_locations),
            raw_input: tool.fields.raw_input,
            raw_output: tool.fields.raw_output,
            subagent_session_id: project_subagent_session_id(&tool.meta),
        })],
        SessionUpdate::Plan(plan) => vec![Event::Delta(ThreadDelta::Plan(project_plan(plan)))],
        SessionUpdate::AvailableCommandsUpdate(update) => {
            vec![Event::Delta(ThreadDelta::AvailableCommands(
                update
                    .available_commands
                    .into_iter()
                    .map(project_command)
                    .collect(),
            ))]
        }
        SessionUpdate::CurrentModeUpdate(update) => vec![Event::Delta(ThreadDelta::Modes(Modes {
            current: update.current_mode_id.to_string(),
            available: Vec::new(),
        }))],
        SessionUpdate::ConfigOptionUpdate(update) => vec![Event::Delta(
            ThreadDelta::ConfigOptions(update.config_options.iter().map(project_config).collect()),
        )],
        SessionUpdate::SessionInfoUpdate(update) => vec![Event::Delta(ThreadDelta::SessionInfo(
            serde_json::to_value(update).unwrap_or(Value::Null),
        ))],
        SessionUpdate::UsageUpdate(update) => vec![Event::Delta(ThreadDelta::Usage(Usage {
            used: update.used,
            size: update.size,
            cost: update.cost.map(|cost| Cost {
                amount: cost.amount,
                currency: cost.currency,
            }),
        }))],
        _ => vec![Event::Delta(ThreadDelta::Unknown(
            serde_json::to_value(update).unwrap_or(Value::Null),
        ))],
    }
}

fn chunk_delta(chunk: v1::ContentChunk, role: MessageRole) -> Vec<Event> {
    let id = chunk.message_id.map(|id| id.to_string());
    match chunk.content {
        v1::ContentBlock::Text(text) => vec![Event::Delta(ThreadDelta::MessageChunk {
            id,
            role,
            text: text.text,
        })],
        content => vec![Event::Delta(ThreadDelta::ContentBlock {
            id,
            role,
            content: project_content_block(content),
        })],
    }
}

fn thought_delta(chunk: v1::ContentChunk) -> Vec<Event> {
    let id = chunk.message_id.map(|id| id.to_string());
    match chunk.content {
        v1::ContentBlock::Text(text) => vec![Event::Delta(ThreadDelta::ThoughtChunk {
            id,
            text: text.text,
        })],
        content => vec![Event::Delta(ThreadDelta::Unknown(
            serde_json::to_value(content).unwrap_or(Value::Null),
        ))],
    }
}

fn project_tool_state(status: v1::ToolCallStatus) -> ToolState {
    match status {
        v1::ToolCallStatus::Pending => ToolState::Pending,
        v1::ToolCallStatus::InProgress => ToolState::Running,
        v1::ToolCallStatus::Completed => ToolState::Completed,
        v1::ToolCallStatus::Failed => ToolState::Failed,
        _ => ToolState::Unknown(format!("{status:?}")),
    }
}

fn project_tool_kind(kind: v1::ToolKind) -> ToolKind {
    match kind {
        v1::ToolKind::Read => ToolKind::Read,
        v1::ToolKind::Edit => ToolKind::Edit,
        v1::ToolKind::Delete => ToolKind::Delete,
        v1::ToolKind::Move => ToolKind::Move,
        v1::ToolKind::Search => ToolKind::Search,
        v1::ToolKind::Execute => ToolKind::Execute,
        v1::ToolKind::Think => ToolKind::Think,
        v1::ToolKind::Fetch => ToolKind::Fetch,
        v1::ToolKind::SwitchMode => ToolKind::SwitchMode,
        _ => ToolKind::Other,
    }
}

fn project_tool_content(content: Vec<v1::ToolCallContent>) -> Vec<ToolContent> {
    content.into_iter().map(project_tool_content_item).collect()
}

fn project_tool_content_item(content: v1::ToolCallContent) -> ToolContent {
    match content {
        v1::ToolCallContent::Content(content) => project_content_block(content.content),
        v1::ToolCallContent::Diff(diff) => ToolContent::Diff {
            path: diff.path.display().to_string(),
            old_text: diff.old_text,
            new_text: diff.new_text,
        },
        v1::ToolCallContent::Terminal(terminal) => ToolContent::Terminal {
            id: terminal.terminal_id.to_string(),
        },
        content => ToolContent::Unknown(serde_json::to_value(content).unwrap_or(Value::Null)),
    }
}

fn project_content_block(content: v1::ContentBlock) -> ToolContent {
    match content {
        v1::ContentBlock::Text(text) => ToolContent::Text(text.text),
        v1::ContentBlock::Image(image) => ToolContent::Image {
            data: image.data,
            mime_type: image.mime_type,
        },
        v1::ContentBlock::Audio(audio) => ToolContent::Audio {
            data: audio.data,
            mime_type: audio.mime_type,
        },
        v1::ContentBlock::ResourceLink(resource) => ToolContent::ResourceLink {
            name: resource.name,
            uri: resource.uri,
        },
        v1::ContentBlock::Resource(resource) => match resource.resource {
            v1::EmbeddedResourceResource::TextResourceContents(resource) => ToolContent::Resource {
                uri: resource.uri,
                text: Some(resource.text),
            },
            v1::EmbeddedResourceResource::BlobResourceContents(resource) => ToolContent::Resource {
                uri: resource.uri,
                text: None,
            },
            _ => ToolContent::Unknown(Value::Null),
        },
        content => ToolContent::Unknown(serde_json::to_value(content).unwrap_or(Value::Null)),
    }
}

fn project_tool_locations(locations: Vec<v1::ToolCallLocation>) -> Vec<ToolLocation> {
    locations
        .into_iter()
        .map(|location| ToolLocation {
            path: location.path.display().to_string(),
            line: location.line,
        })
        .collect()
}

fn project_subagent_session_id(meta: &Option<v1::Meta>) -> Option<String> {
    let meta = meta.as_ref()?;
    ["subagentSessionId", "subagent_session_id"]
        .into_iter()
        .find_map(|key| meta.get(key).and_then(Value::as_str).map(str::to_string))
}

fn project_plan(plan: v1::Plan) -> Plan {
    Plan {
        entries: plan
            .entries
            .into_iter()
            .map(|entry| PlanEntry {
                content: entry.content,
                priority: format!("{:?}", entry.priority),
                status: format!("{:?}", entry.status),
            })
            .collect(),
    }
}

fn project_command(command: v1::AvailableCommand) -> AvailableCommand {
    AvailableCommand {
        name: command.name,
        description: command.description,
        input_hint: command.input.map(|input| match input {
            v1::AvailableCommandInput::Unstructured(input) => input.hint,
            _ => String::new(),
        }),
    }
}

fn project_modes(modes: v1::SessionModeState) -> Modes {
    Modes {
        current: modes.current_mode_id.to_string(),
        available: modes
            .available_modes
            .into_iter()
            .map(|mode| SessionMode {
                id: mode.id.to_string(),
                name: mode.name,
                description: mode.description,
            })
            .collect(),
    }
}

fn project_config(option: &v1::SessionConfigOption) -> ConfigOption {
    let kind = match &option.kind {
        v1::SessionConfigKind::Select(select) => ConfigKind::Select {
            current: select.current_value.to_string(),
            choices: config_choices(&select.options),
        },
        v1::SessionConfigKind::Boolean(boolean) => ConfigKind::Boolean {
            current: boolean.current_value,
        },
        _ => ConfigKind::Select {
            current: String::new(),
            choices: Vec::new(),
        },
    };
    ConfigOption {
        id: option.id.to_string(),
        name: option.name.clone(),
        description: option.description.clone(),
        category: option
            .category
            .as_ref()
            .map(|category| format!("{category:?}")),
        kind,
    }
}

fn config_choices(options: &v1::SessionConfigSelectOptions) -> Vec<ConfigChoice> {
    match options {
        v1::SessionConfigSelectOptions::Ungrouped(options) => options
            .iter()
            .map(|option| ConfigChoice {
                id: option.value.to_string(),
                name: option.name.clone(),
            })
            .collect(),
        v1::SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| group.options.iter())
            .map(|option| ConfigChoice {
                id: option.value.to_string(),
                name: option.name.clone(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn project_capabilities(init: &v1::InitializeResponse) -> Capabilities {
    let auth_methods = init
        .auth_methods
        .iter()
        .map(|method| match method {
            v1::AuthMethod::Agent(method) => AuthMethod {
                id: method.id.to_string(),
                name: method.name.clone(),
                description: method.description.clone(),
                terminal: false,
            },
            v1::AuthMethod::Terminal(method) => AuthMethod {
                id: method.id.to_string(),
                name: method.name.clone(),
                description: method.description.clone(),
                terminal: true,
            },
            _ => AuthMethod {
                id: String::new(),
                name: String::from("Unknown authentication method"),
                description: None,
                terminal: false,
            },
        })
        .collect();
    Capabilities {
        load_session: init.agent_capabilities.load_session,
        prompt: PromptCapabilities {
            image: init.agent_capabilities.prompt_capabilities.image,
            audio: init.agent_capabilities.prompt_capabilities.audio,
            embedded_context: init.agent_capabilities.prompt_capabilities.embedded_context,
        },
        sessions: SessionCapabilities {
            list: init.agent_capabilities.session_capabilities.list.is_some(),
            delete: init
                .agent_capabilities
                .session_capabilities
                .delete
                .is_some(),
            resume: init
                .agent_capabilities
                .session_capabilities
                .resume
                .is_some(),
            close: init.agent_capabilities.session_capabilities.close.is_some(),
        },
        auth_methods,
        logout: init.agent_capabilities.auth.logout.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use v1::{ContentBlock, ContentChunk, ImageContent, SessionUpdate, TextContent, ToolCall};

    #[test]
    fn profile_ids_round_trip() {
        for profile in [Profile::Claude, Profile::Codex, Profile::Pi] {
            assert_eq!(Profile::from_id(profile.id()), Some(profile));
        }
        assert_eq!(Profile::from_id("unknown"), None);
    }

    #[test]
    fn profile_commands_parse_to_expected_launchers() {
        for profile in [Profile::Claude, Profile::Codex, Profile::Pi] {
            let agent = AcpAgent::from_str(profile.command()).expect("profile command parses");
            assert_eq!(agent.config().command(), std::path::Path::new("npx"));
        }
        let pi = AcpAgent::from_str(Profile::Pi.command()).expect("Pi command parses");
        assert_eq!(pi.config().arguments(), ["pi-acp"]);
    }

    #[test]
    fn profiles_serialize_and_deserialize_without_changing_legacy_values() {
        for profile in [Profile::Claude, Profile::Codex, Profile::Pi] {
            let json = serde_json::to_string(&profile).expect("profile serializes");
            assert_eq!(
                serde_json::from_str::<Profile>(&json).expect("profile deserializes"),
                profile
            );
        }
        assert_eq!(
            serde_json::from_str::<Profile>(r#""Claude""#).expect("legacy Claude profile"),
            Profile::Claude
        );
        assert_eq!(
            serde_json::from_str::<Profile>(r#""Codex""#).expect("legacy Codex profile"),
            Profile::Codex
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn worker_creates_session_and_streams_typed_prompt_response() {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("fake_acp.py");
        let request_log = directory.path().join("requests.log");
        std::fs::write(
            &script,
            r#"import json, sys
for line in sys.stdin:
    message = json.loads(line)
    request_id = message.get("id")
    method = message.get("method")
    with open(sys.argv[1], "a") as log:
        log.write(str(method) + "\n")
    if request_id is None:
        continue
    if method == "initialize":
        result = {"protocolVersion": 1, "agentCapabilities": {"sessionCapabilities": {"list": {}}}}
    elif method == "session/new":
        result = {"sessionId": "fake-session", "modes": {"currentModeId": "fast", "availableModes": [{"id": "fast", "name": "Fast"}]}, "configOptions": [{"id": "model", "name": "Model", "type": "select", "currentValue": "small", "options": [{"value": "small", "name": "Small"}]}]}
    elif method == "session/list":
        if message.get("params", {}).get("cursor"):
            result = {"sessions": [{"sessionId": "history-2", "cwd": sys.argv[2], "title": "Older History"}]}
        else:
            result = {"sessions": [{"sessionId": "history-1", "cwd": sys.argv[2], "title": "History"}], "nextCursor": "page-2"}
    elif method == "session/set_mode" or method == "session/set_config_option":
        result = {}
    elif method == "session/prompt":
        for update in [
            {"sessionUpdate": "agent_message_chunk", "messageId": "m1", "content": {"type": "text", "text": "fake reply"}},
            {"sessionUpdate": "tool_call", "toolCallId": "t1", "title": "run", "status": "in_progress", "kind": "execute"},
            {"sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed"},
            {"sessionUpdate": "available_commands_update", "availableCommands": [{"name": "test", "description": "Test"}]},
        ]:
            print(json.dumps({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "fake-session", "update": update}}), flush=True)
        result = {"stopReason": "end_turn"}
    else:
        result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": request_id, "result": result}), flush=True)
"#,
        )
        .unwrap();
        let agent = AcpAgent::from_str(&format!(
            "python3 {} {} {}",
            script.display(),
            request_log.display(),
            directory.path().display()
        ))
        .unwrap();
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, mut events) = mpsc::unbounded_channel();
        tokio::spawn(run(
            agent,
            directory.path().to_path_buf(),
            None,
            None,
            command_rx,
            event_tx,
        ));
        let handle = AcpHandle { tx: command_tx };

        let ready = loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap();
            if let Event::Ready { .. } = event {
                break event;
            }
        };
        assert!(matches!(ready, Event::Ready { modes: Some(_), .. }));

        let prompt_id = handle
            .submit_submission(PromptSubmission::new(PromptPayload::new("hello")))
            .unwrap();
        let mut reducer = ThreadReducer::new();
        let mut saw_prompt_accepted = false;
        let mut saw_tool = false;
        let mut saw_command = false;
        for _ in 0..12 {
            let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
                .await
                .unwrap()
                .unwrap();
            match &event {
                Event::PromptAccepted { id } => {
                    assert_eq!(id, &prompt_id);
                    saw_prompt_accepted = true;
                }
                Event::Delta(ThreadDelta::MessageChunk {
                    role: MessageRole::User,
                    ..
                }) => assert!(saw_prompt_accepted),
                Event::Turn(TurnState::Generating) => assert!(saw_prompt_accepted),
                _ => {}
            }
            if let Event::Delta(delta) = event {
                if matches!(delta, ThreadDelta::ToolUpsert { .. }) {
                    saw_tool = true;
                }
                if matches!(delta, ThreadDelta::AvailableCommands(_)) {
                    saw_command = true;
                }
                reducer.apply(delta);
                if saw_tool && saw_command {
                    break;
                }
            }
        }
        handle.set_mode("fast").unwrap();
        handle
            .set_config_option("model", ConfigValue::Select("small".into()))
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let log = std::fs::read_to_string(&request_log).unwrap_or_default();
                if log.contains("session/set_mode") && log.contains("session/set_config_option") {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        handle.list_sessions().unwrap();
        let listed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(Event::SessionList(sessions)) = events.recv().await {
                    break sessions;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "history-1");
        assert_eq!(listed[1].id, "history-2");
        handle.shutdown();
        assert!(saw_prompt_accepted);
        assert!(saw_tool && saw_command);
        assert!(reducer.snapshot().items.iter().any(|item| matches!(item, ThreadItem::Tool(tool) if tool.id == "t1" && tool.state == ToolState::Completed)));
    }

    #[test]
    fn elicitation_response_projects_supported_json_values() {
        let response = project_elicitation_response(ElicitationResponse::Accept(
            serde_json::Map::from_iter([
                ("name".into(), serde_json::json!("Muxlane")),
                ("count".into(), serde_json::json!(3)),
                ("enabled".into(), serde_json::json!(true)),
                ("tags".into(), serde_json::json!(["a", "b"])),
            ]),
        ))
        .unwrap();
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["action"], "accept");
        assert_eq!(value["content"]["count"], 3);
        assert_eq!(value["content"]["tags"], serde_json::json!(["a", "b"]));
        assert!(project_elicitation_response(ElicitationResponse::Accept(
            serde_json::Map::from_iter([("nested".into(), serde_json::json!({"x": 1}))]),
        ))
        .is_err());
    }

    #[test]
    fn prompt_submissions_read_legacy_payloads_with_generated_ids() {
        let legacy: PromptSubmission =
            serde_json::from_str(r#"{"text":"old prompt","blocks":[]}"#).unwrap();
        assert_eq!(legacy.payload.text, "old prompt");
        assert!(!legacy.id.to_string().is_empty());

        let current: PromptSubmission = serde_json::from_str(
            r#"{"id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","payload":{"text":"current","blocks":[]}}"#,
        )
        .unwrap();
        assert_eq!(current.payload.text, "current");
        assert_eq!(current.id.to_string(), "01ARZ3NDEKTSV4RRFFQ69G5FAV");
    }

    #[test]
    fn prompt_payload_projects_text_resources_and_links() {
        let blocks = project_prompt(
            PromptPayload::new("question")
                .with_block(PromptBlock::Resource {
                    name: "src/main.rs".into(),
                    uri: "file:///tmp/src/main.rs".into(),
                    mime_type: Some("text/plain".into()),
                    text: "fn main() {}".into(),
                })
                .with_block(PromptBlock::ResourceLink {
                    name: "src".into(),
                    uri: "file:///tmp/src".into(),
                    mime_type: None,
                }),
        );

        assert_eq!(blocks.len(), 3);
        assert!(matches!(blocks[0], v1::ContentBlock::Text(_)));
        assert!(matches!(blocks[1], v1::ContentBlock::Resource(_)));
        assert!(matches!(blocks[2], v1::ContentBlock::ResourceLink(_)));
    }

    #[test]
    fn projects_typed_stream_chunks_and_tool_updates() {
        let events = project_update(SessionUpdate::AgentMessageChunk(
            ContentChunk::new(ContentBlock::Text(TextContent::new("hi"))).message_id("m1"),
        ));
        assert!(
            matches!(&events[0], Event::Delta(ThreadDelta::MessageChunk { id: Some(id), text, .. }) if id == "m1" && text == "hi")
        );

        let image_events = project_update(SessionUpdate::AgentMessageChunk(
            ContentChunk::new(ContentBlock::Image(ImageContent::new(
                "aGVsbG8=",
                "image/png",
            )))
            .message_id("m2"),
        ));
        assert!(matches!(
            &image_events[0],
            Event::Delta(ThreadDelta::ContentBlock {
                id: Some(id),
                role: MessageRole::Assistant,
                content: ToolContent::Image { mime_type, .. },
            }) if id == "m2" && mime_type == "image/png"
        ));

        let tool = ToolCall::new("t1", "run").meta(v1::Meta::from_iter([(
            "subagentSessionId".into(),
            serde_json::Value::String("child-session".into()),
        )]));
        let events = project_update(SessionUpdate::ToolCall(tool));
        assert!(
            matches!(&events[0], Event::Delta(ThreadDelta::ToolUpsert { id, title: Some(title), subagent_session_id: Some(session_id), .. }) if id == "t1" && title == "run" && session_id == "child-session")
        );
    }
}
