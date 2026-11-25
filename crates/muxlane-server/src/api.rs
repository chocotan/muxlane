use crate::MuxlaneServer;
use anyhow::Context as _;
use muxlane_core::model::{AgentId, AgentInstance, AgentStatus, Project, Snapshot};
use muxlane_core::protocol::{AgentSpawnParams, DeleteScopeResult, EventMsg, ProjectAddParams};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug)]
pub enum ProjectAddError {
    PathNotFound {
        path: String,
        source: std::io::Error,
    },
    NotDirectory(String),
    CreateDirectoryFailed {
        path: String,
        source: std::io::Error,
    },
    InvalidPath {
        path: String,
        source: std::io::Error,
    },
}

impl ProjectAddError {
    pub fn code(&self) -> &'static str {
        use muxlane_core::protocol::error_codes;
        match self {
            Self::PathNotFound { .. } => error_codes::PATH_NOT_FOUND,
            Self::NotDirectory(_) => error_codes::NOT_A_DIRECTORY,
            Self::CreateDirectoryFailed { .. } => error_codes::CREATE_DIRECTORY_FAILED,
            Self::InvalidPath { .. } => error_codes::INVALID_PATH,
        }
    }
}

impl std::fmt::Display for ProjectAddError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PathNotFound { path, source } => {
                write!(formatter, "project path does not exist: {path}: {source}")
            }
            Self::NotDirectory(path) => {
                write!(formatter, "project path is not a directory: {path}")
            }
            Self::CreateDirectoryFailed { path, source } => {
                write!(
                    formatter,
                    "failed to create project directory {path}: {source}"
                )
            }
            Self::InvalidPath { path, source } => {
                write!(formatter, "invalid project path {path}: {source}")
            }
        }
    }
}

impl std::error::Error for ProjectAddError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PathNotFound { source, .. }
            | Self::CreateDirectoryFailed { source, .. }
            | Self::InvalidPath { source, .. } => Some(source),
            Self::NotDirectory(_) => None,
        }
    }
}

fn invalid_path(path: &str, reason: impl Into<String>) -> ProjectAddError {
    ProjectAddError::InvalidPath {
        path: path.to_string(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidInput, reason.into()),
    }
}

fn expand_project_path(requested_path: &str) -> Result<PathBuf, ProjectAddError> {
    let requested_path = requested_path.trim();
    if requested_path.is_empty() {
        return Err(invalid_path(requested_path, "path is empty"));
    }
    if requested_path == "~" {
        return std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            invalid_path(
                requested_path,
                "HOME is not set for the muxlane server process",
            )
        });
    }
    if let Some(rest) = requested_path.strip_prefix("~/") {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(rest))
            .ok_or_else(|| {
                invalid_path(
                    requested_path,
                    "HOME is not set for the muxlane server process",
                )
            });
    }
    if requested_path.starts_with('~') {
        return Err(invalid_path(
            requested_path,
            "~user paths are not supported; use ~ or ~/...",
        ));
    }
    Ok(PathBuf::from(requested_path))
}

fn resolve_project_path(
    requested_path: String,
    create_if_missing: bool,
) -> Result<PathBuf, ProjectAddError> {
    let path = expand_project_path(&requested_path)?;
    match std::fs::metadata(&path) {
        Ok(_) => {}
        Err(source) if source.kind() == std::io::ErrorKind::NotFound && !create_if_missing => {
            return Err(ProjectAddError::PathNotFound {
                path: requested_path,
                source,
            });
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(&path).map_err(|source| {
                ProjectAddError::CreateDirectoryFailed {
                    path: requested_path.clone(),
                    source,
                }
            })?;
        }
        Err(source) if create_if_missing && source.kind() == std::io::ErrorKind::NotADirectory => {
            return Err(ProjectAddError::CreateDirectoryFailed {
                path: requested_path,
                source,
            });
        }
        Err(source) => {
            return Err(ProjectAddError::InvalidPath {
                path: requested_path,
                source,
            });
        }
    }

    let canonical = path
        .canonicalize()
        .map_err(|source| ProjectAddError::InvalidPath {
            path: requested_path.clone(),
            source,
        })?;
    let metadata =
        std::fs::metadata(&canonical).map_err(|source| ProjectAddError::InvalidPath {
            path: requested_path.clone(),
            source,
        })?;
    if !metadata.is_dir() {
        return Err(ProjectAddError::NotDirectory(requested_path));
    }
    Ok(canonical)
}

/// spawn 目标项目不存在；RPC 层映射为 no_such_project，调用方可 downcast 判断。
#[derive(Debug)]
pub struct NoSuchProject(pub muxlane_core::model::ProjectId);

impl std::fmt::Display for NoSuchProject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no such project: {}", self.0)
    }
}

impl std::error::Error for NoSuchProject {}

#[derive(Debug)]
pub(crate) enum AgentRenameError {
    InvalidTitle(&'static str),
    NoSuchAgent(AgentId),
}

impl AgentRenameError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::InvalidTitle(_) => "bad_params",
            Self::NoSuchAgent(_) => "no_such_agent",
        }
    }
}

impl std::fmt::Display for AgentRenameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTitle(message) => f.write_str(message),
            Self::NoSuchAgent(agent) => write!(f, "no such agent: {agent}"),
        }
    }
}

impl std::error::Error for AgentRenameError {}

impl MuxlaneServer {
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<EventMsg> {
        self.events.subscribe()
    }

    pub fn subscribe_dirty(&self) -> tokio::sync::watch::Receiver<u64> {
        self.dirty.subscribe()
    }

    pub async fn snapshot(&self) -> Snapshot {
        self.state.read().await.snapshot()
    }

    pub fn try_session(&self, agent: &AgentId) -> Option<Arc<muxlane_term::PtySession>> {
        self.sessions
            .try_lock()
            .ok()
            .and_then(|sessions| sessions.get(agent).cloned())
    }

    pub async fn session(&self, agent: &AgentId) -> Option<Arc<muxlane_term::PtySession>> {
        self.sessions.lock().await.get(agent).cloned()
    }

    pub async fn session_count(&self) -> usize {
        self.sessions.lock().await.len()
    }

    /// Drop the in-process handle for a session and forget the agent, leaving the tmux
    /// session running. Lets tests exercise `restore_sessions` re-attach against a live
    /// tmux server without a real process restart.
    #[doc(hidden)]
    pub async fn sessions_forget_for_test(&self, agent: &AgentId) {
        self.sessions.lock().await.remove(agent);
        self.state.write().await.remove_agent(agent);
    }

    pub async fn subscription_count(&self) -> usize {
        self.subs.lock().await.len()
    }

    pub async fn spawn_agent(&self, params: AgentSpawnParams) -> anyhow::Result<AgentInstance> {
        let _lifecycle = self.lifecycle.lock().await;
        let project = self
            .state
            .read()
            .await
            .projects
            .iter()
            .find(|project| project.id == params.project)
            .cloned()
            .ok_or_else(|| NoSuchProject(params.project.clone()))?;
        let agent_type = params
            .agent_type
            .unwrap_or(muxlane_core::model::AgentType::Shell);
        let agent_id = muxlane_core::model::new_id(agent_type.as_str());
        let tmux_name = format!("muxlane-{agent_id}");
        let title = params.preset_name.unwrap_or_else(|| {
            if agent_type == muxlane_core::model::AgentType::Shell {
                muxlane_term::default_shell_program()
                    .rsplit('/')
                    .next()
                    .unwrap_or("shell")
                    .into()
            } else {
                agent_type.as_str().to_string()
            }
        });
        let mut cfg =
            if agent_type == muxlane_core::model::AgentType::Shell && params.program.is_none() {
                muxlane_term::LaunchCfg::shell(agent_id.clone(), project.path.clone())
            } else {
                muxlane_term::LaunchCfg {
                    agent: agent_id.clone(),
                    agent_type,
                    cwd: project.path.clone(),
                    env: params.env.unwrap_or_default(),
                    program_override: Some(
                        params
                            .program
                            .unwrap_or_else(|| agent_type.as_str().to_string()),
                    ),
                    args: params.args.unwrap_or_default(),
                    cols: 120,
                    rows: 32,
                    tmux_session: Some(tmux_name.clone()),
                }
            };
        cfg.tmux_session = Some(tmux_name);
        cfg.env.push(("MUXLANE_AGENT_ID".into(), agent_id.clone()));
        cfg.env.push((
            "MUXLANE_SOCKET".into(),
            self.socket_path.display().to_string(),
        ));
        cfg.env
            .push(("MUXLANE_HOOK_TOKEN".into(), self.hook_token(&agent_id)));

        // 注意：调用方可能在非 Tokio 上下文（app 侧 GPUI background executor），
        // 不能用 tokio::task::spawn_blocking（依赖线程本地 Handle::current()），
        // 必须显式走 server 持有的 runtime handle。
        let session = self
            .runtime
            .spawn_blocking(move || muxlane_term::PtySession::spawn(cfg))
            .await
            .context("spawn agent task")?
            .context("spawn agent")?;
        let instance = AgentInstance {
            id: agent_id.clone(),
            project: project.id.clone(),
            agent_type,
            title,
            title_is_custom: false,
            status: AgentStatus::Idle,
            status_since: muxlane_core::model::now_secs(),
            seen: true,
            tmux_session: session.tmux_session_name().map(str::to_string),
        };
        self.sessions
            .lock()
            .await
            .insert(agent_id.clone(), Arc::clone(&session));
        self.state
            .write()
            .await
            .add_agent(project, instance.clone());
        if agent_type.supports_chat_bridge() {
            let _ = self.ensure_pty_bridge_for_agent(&agent_id).await;
        }
        self.dirty.bump();
        self.persist_runtime_state().await?;
        Ok(instance)
    }

    pub async fn rename_agent(
        &self,
        agent: &AgentId,
        title: String,
    ) -> anyhow::Result<AgentInstance> {
        if title.chars().any(char::is_control) {
            return Err(AgentRenameError::InvalidTitle(
                "title must not contain control characters",
            )
            .into());
        }
        let title = title.trim();
        if title.is_empty() {
            return Err(AgentRenameError::InvalidTitle("title must not be empty").into());
        }
        if title.chars().count() > 128 {
            return Err(
                AgentRenameError::InvalidTitle("title must be at most 128 characters").into(),
            );
        }
        let _lifecycle = self.lifecycle.lock().await;
        let mut state = self.state.write().await;
        let index = state
            .agents
            .iter()
            .position(|item| &item.id == agent)
            .ok_or_else(|| AgentRenameError::NoSuchAgent(agent.clone()))?;
        let mut candidate = state.snapshot();
        candidate.agents[index].title = title.to_string();
        candidate.agents[index].title_is_custom = true;
        self.persist_snapshot(&candidate)?;
        let renamed = candidate.agents[index].clone();
        state.agents[index] = renamed.clone();
        drop(state);
        self.dirty.bump();
        Ok(renamed)
    }

    pub async fn delete_agent(&self, agent: &AgentId) -> anyhow::Result<DeleteScopeResult> {
        let (destroyed_agents, failed_agents) =
            self.destroy_sessions(std::slice::from_ref(agent)).await;
        if !destroyed_agents.is_empty() {
            let mut state = self.state.write().await;
            for agent in &destroyed_agents {
                state.remove_agent(agent);
            }
            drop(state);
            self.dirty.bump();
            self.persist_runtime_state().await?;
        }
        Ok(DeleteScopeResult {
            destroyed_agents,
            failed_agents,
        })
    }

    pub async fn add_project(&self, params: ProjectAddParams) -> anyhow::Result<Project> {
        let _lifecycle = self.lifecycle.lock().await;
        let requested_path = params.path;
        let requested_name = params.name;
        let create_if_missing = params.create_if_missing;
        let path = self
            .runtime
            .spawn_blocking(move || resolve_project_path(requested_path, create_if_missing))
            .await
            .context("resolve project path task")??;

        let mut state = self.state.write().await;
        if let Some(existing) = state
            .projects
            .iter()
            .find(|project| project.path == path)
            .cloned()
        {
            return Ok(existing);
        }
        let name = requested_name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| {
                path.file_name()
                    .map(|value| value.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string())
            });
        let project = Project {
            id: muxlane_core::model::new_id("project"),
            name,
            path,
            branch: None,
            agents: vec![],
        };
        let inserted = state.add_project(project.clone());
        debug_assert!(inserted);
        drop(state);

        self.dirty.bump();
        if let Err(error) = self.persist_runtime_state().await {
            self.state
                .write()
                .await
                .projects
                .retain(|item| item.id != project.id);
            self.dirty.bump();
            return Err(error);
        }
        Ok(project)
    }

    pub async fn delete_project(
        &self,
        project: &muxlane_core::model::ProjectId,
    ) -> anyhow::Result<DeleteScopeResult> {
        let _lifecycle = self.lifecycle.lock().await;
        let agents: Vec<_> = self
            .state
            .read()
            .await
            .agents
            .iter()
            .filter(|agent| &agent.project == project)
            .map(|agent| agent.id.clone())
            .collect();
        let (destroyed_agents, failed_agents) = self.destroy_sessions(&agents).await;
        {
            let mut state = self.state.write().await;
            for agent in &destroyed_agents {
                state.remove_agent(agent);
            }
            if failed_agents.is_empty() {
                state.projects.retain(|item| &item.id != project);
            }
        }
        self.dirty.bump();
        self.persist_runtime_state().await?;
        Ok(DeleteScopeResult {
            destroyed_agents,
            failed_agents,
        })
    }

    pub async fn list_presets(
        &self,
        project: &muxlane_core::model::ProjectId,
    ) -> anyhow::Result<Vec<muxlane_core::AgentPreset>> {
        let path = self
            .state
            .read()
            .await
            .projects
            .iter()
            .find(|item| &item.id == project)
            .map(|item| item.path.clone())
            .ok_or_else(|| anyhow::anyhow!("no such project: {project}"))?;
        let cwd = path.clone();
        self.runtime
            .spawn_blocking(move || {
                muxlane_core::builtin_presets(muxlane_term::default_shell_program())
                    .into_iter()
                    .filter(|preset| !preset.require_installed || preset.installed_in(&cwd))
                    .collect::<Vec<_>>()
            })
            .await
            .context("list presets")
    }

    pub async fn mark_seen(&self, agent: &AgentId) {
        if !self.state.write().await.mark_seen(agent).is_empty() {
            self.dirty.bump();
        }
    }

    pub async fn mark_working(&self, agent: &AgentId) {
        if !self
            .state
            .write()
            .await
            .mark_screen_working(agent)
            .is_empty()
        {
            self.dirty.bump();
        }
        self.chat.wake_pty_bridge(agent).await;
    }

    pub async fn persist(&self) -> anyhow::Result<()> {
        self.persist_runtime_state().await
    }

    pub async fn restore_agent(
        &self,
        project: Project,
        instance: AgentInstance,
        session: Arc<muxlane_term::PtySession>,
    ) {
        self.restore_agent_quiet(project, instance, session).await;
        self.dirty.bump();
    }

    /// Register a restored session without notifying subscribers; the caller batches.
    pub(crate) async fn restore_agent_quiet(
        &self,
        project: Project,
        instance: AgentInstance,
        session: Arc<muxlane_term::PtySession>,
    ) {
        let id = instance.id.clone();
        let agent_type = instance.agent_type;
        let cwd = project.path.clone();
        let started_secs = instance.status_since;
        self.sessions.lock().await.insert(id.clone(), session);
        self.state.write().await.add_agent(project, instance);
        if agent_type.supports_chat_bridge() {
            let _ = self
                .chat
                .ensure_pty_bridge(&id, agent_type, cwd, started_secs, true, None)
                .await;
        }
    }

    pub(crate) fn bump_dirty(&self) {
        self.dirty.bump();
    }

    async fn destroy_sessions(&self, agents: &[AgentId]) -> (Vec<AgentId>, Vec<AgentId>) {
        let sessions: HashMap<_, _> = {
            let mut live = self.sessions.lock().await;
            agents
                .iter()
                .filter_map(|agent| live.remove(agent).map(|session| (agent.clone(), session)))
                .collect()
        };
        let mut destroyed = Vec::new();
        let mut failed = Vec::new();
        for agent in agents {
            if let Some(session) = sessions.get(agent) {
                let session = Arc::clone(session);
                let killed = self
                    .runtime
                    .spawn_blocking(move || session.kill_persistent())
                    .await
                    .unwrap_or(false);
                if killed {
                    destroyed.push(agent.clone());
                } else {
                    failed.push(agent.clone());
                }
            } else {
                destroyed.push(agent.clone());
            }
        }
        if !failed.is_empty() {
            let mut live = self.sessions.lock().await;
            for agent in &failed {
                if let Some(session) = sessions.get(agent) {
                    live.insert(agent.clone(), Arc::clone(session));
                }
            }
        }
        let mut subs = self.subs.lock().await;
        for agent in &destroyed {
            subs.mark_agent_exit(agent);
        }
        drop(subs);
        for agent in &destroyed {
            self.chat.remove_session(agent).await;
        }
        (destroyed, failed)
    }

    pub async fn spawn_chat(
        &self,
        params: muxlane_core::protocol::ChatSpawnParams,
    ) -> anyhow::Result<AgentInstance> {
        self.spawn_chat_in_cwd(params, None).await
    }

    pub async fn spawn_chat_in_cwd(
        &self,
        params: muxlane_core::protocol::ChatSpawnParams,
        cwd_override: Option<PathBuf>,
    ) -> anyhow::Result<AgentInstance> {
        let _lifecycle = self.lifecycle.lock().await;
        let project = self
            .state
            .read()
            .await
            .projects
            .iter()
            .find(|project| project.id == params.project)
            .cloned()
            .ok_or_else(|| NoSuchProject(params.project.clone()))?;
        let agent_type = params
            .agent_type
            .unwrap_or(muxlane_core::model::AgentType::Pi);
        let agent_id = muxlane_core::model::new_id(agent_type.as_str());
        let title = params
            .preset_name
            .clone()
            .unwrap_or_else(|| agent_type.as_str().to_string());
        let working_dir = cwd_override.unwrap_or_else(|| project.path.clone());

        let preset = muxlane_core::builtin_presets(muxlane_term::default_shell_program())
            .into_iter()
            .find(|p| p.agent_type == agent_type);
        let driver_kind = params
            .driver
            .or_else(|| preset.as_ref().and_then(|p| p.chat_driver_kind()))
            .ok_or_else(|| {
                anyhow::anyhow!("agent type {:?} does not support native chat", agent_type)
            })?;
        let program = params
            .program
            .or_else(|| {
                preset
                    .as_ref()
                    .and_then(|p| p.executable_in(&working_dir))
                    .map(|path| path.display().to_string())
            })
            .unwrap_or_else(|| agent_type.program().to_string());
        let args = params
            .args
            .or_else(|| {
                preset
                    .as_ref()
                    .and_then(|p| p.chat_launch_args(params.provider_session.as_deref()))
            })
            .unwrap_or_else(|| match driver_kind {
                muxlane_core::ChatDriverKind::PiRpc => vec!["--mode".into(), "rpc".into()],
                muxlane_core::ChatDriverKind::Acp => {
                    if agent_type == muxlane_core::model::AgentType::Opencode {
                        vec!["acp".into()]
                    } else {
                        vec!["--acp".into()]
                    }
                }
            });

        let spec = muxlane_chat::DriverLaunchSpec {
            agent: agent_id.clone(),
            agent_type,
            driver_kind,
            program,
            args,
            cwd: working_dir,
            env: params.env.unwrap_or_default(),
            provider_session: params.provider_session,
            model: params.model,
            thinking_level: params.thinking_level,
        };
        self.chat
            .start_session(spec, false, params.initial_prompt)
            .await?;

        let instance = AgentInstance {
            id: agent_id.clone(),
            project: project.id.clone(),
            agent_type,
            title,
            title_is_custom: false,
            status: AgentStatus::Idle,
            status_since: muxlane_core::model::now_secs(),
            seen: true,
            tmux_session: None,
        };
        self.state
            .write()
            .await
            .add_agent(project, instance.clone());
        self.dirty.bump();
        self.persist_runtime_state().await?;
        Ok(instance)
    }

    /// 为存活的 PTY Agent 确保已注册双向实时桥接会话（若已处于结构化驱动模式则返回现有 meta）。
    pub async fn ensure_pty_bridge_for_agent(
        &self,
        agent: &AgentId,
    ) -> Option<muxlane_core::ChatSessionMeta> {
        self.session(agent).await?;
        let (inst, project) = {
            let state = self.state.read().await;
            let inst = state.agents.iter().find(|a| &a.id == agent)?.clone();
            let project = state
                .projects
                .iter()
                .find(|p| p.id == inst.project)?
                .clone();
            (inst, project)
        };
        if !inst.agent_type.supports_chat_bridge() {
            return None;
        }
        Some(
            self.chat
                .ensure_pty_bridge(
                    agent,
                    inst.agent_type,
                    project.path,
                    inst.status_since,
                    true,
                    None,
                )
                .await,
        )
    }

    /// 确保所有存活且支持桥接的 PTY Agent 都已向 ChatManager 注册摘要，供桌面与移动端秒切对话。
    pub async fn ensure_all_pty_bridges(&self) {
        let live_ids: std::collections::HashSet<AgentId> = {
            let guard = self.sessions.lock().await;
            guard.keys().cloned().collect()
        };
        if live_ids.is_empty() {
            return;
        }
        let candidates: Vec<(AgentId, muxlane_core::model::AgentType, PathBuf, u64)> = {
            let state = self.state.read().await;
            state
                .agents
                .iter()
                .filter(|a| live_ids.contains(&a.id) && a.agent_type.supports_chat_bridge())
                .filter_map(|a| {
                    let project = state.projects.iter().find(|p| p.id == a.project)?;
                    Some((
                        a.id.clone(),
                        a.agent_type,
                        project.path.clone(),
                        a.status_since,
                    ))
                })
                .collect()
        };
        for (agent_id, agent_type, cwd, started_secs) in candidates {
            let _ = self
                .chat
                .ensure_pty_bridge(&agent_id, agent_type, cwd, started_secs, true, None)
                .await;
        }
    }

    pub async fn chat_history(
        &self,
        agent: &AgentId,
    ) -> anyhow::Result<muxlane_core::protocol::ChatHistoryResult> {
        let _ = self.ensure_pty_bridge_for_agent(agent).await;
        if let Some(history) = self.chat.history(agent).await {
            return Ok(history);
        }
        let (inst, project) = {
            let state = self.state.read().await;
            let inst = state
                .agents
                .iter()
                .find(|a| &a.id == agent)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no such agent: {agent}"))?;
            let project = state
                .projects
                .iter()
                .find(|p| p.id == inst.project)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no such project: {}", inst.project))?;
            (inst, project)
        };
        if let Some(history) = self
            .chat
            .refresh_pty_transcript_if_needed(
                agent,
                inst.agent_type,
                project.path,
                inst.status_since.saturating_sub(3600),
            )
            .await
        {
            return Ok(history);
        }
        Ok(muxlane_core::protocol::ChatHistoryResult {
            agent: agent.clone(),
            seq: 0,
            meta: muxlane_core::ChatSessionMeta {
                driver: muxlane_core::SessionDriverMode::TerminalPty,
                has_pty: self.session(agent).await.is_some(),
                supports_chat_input: false,
                supports_terminal_switch: muxlane_core::builtin_presets("")
                    .into_iter()
                    .find(|p| p.agent_type == inst.agent_type)
                    .and_then(|p| p.chat_driver_kind())
                    .is_some(),
                ..Default::default()
            },
            entries: Vec::new(),
            pending_approvals: Vec::new(),
        })
    }

    pub async fn switch_chat_mode(
        &self,
        agent: &AgentId,
        target: muxlane_core::SessionDriverMode,
    ) -> anyhow::Result<muxlane_core::ChatSessionMeta> {
        let _lifecycle = self.lifecycle.lock().await;
        let (inst, project) = {
            let state = self.state.read().await;
            let inst = state
                .agents
                .iter()
                .find(|a| &a.id == agent)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no such agent: {agent}"))?;
            let project = state
                .projects
                .iter()
                .find(|p| p.id == inst.project)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no such project: {}", inst.project))?;
            (inst, project)
        };

        let preset = muxlane_core::builtin_presets(muxlane_term::default_shell_program())
            .into_iter()
            .find(|p| p.agent_type == inst.agent_type);

        match target {
            muxlane_core::SessionDriverMode::TerminalPty => {
                if self.session(agent).await.is_some() {
                    let _ = self.ensure_pty_bridge_for_agent(agent).await;
                    return Ok(self.chat_history(agent).await?.meta);
                }
                let resume_info = self.chat.session_resume_info(agent).await;
                self.chat.detach_driver(agent).await;

                let resume_ref = resume_info.as_ref().and_then(|(_, meta)| {
                    meta.provider_session_file
                        .clone()
                        .or_else(|| meta.provider_session_id.clone())
                });
                let program = resume_info
                    .as_ref()
                    .map(|(spec, _)| spec.program.clone())
                    .or_else(|| {
                        preset
                            .as_ref()
                            .and_then(|p| p.executable_in(&project.path))
                            .map(|p| p.display().to_string())
                    })
                    .unwrap_or_else(|| inst.agent_type.program().to_string());
                let args = match (&preset, resume_ref.as_deref()) {
                    (Some(p), Some(s)) => p.terminal_resume_args(s),
                    (Some(p), None) => p.args.clone(),
                    _ => Vec::new(),
                };
                let tmux_name = format!("muxlane-{agent}");
                let mut cfg = muxlane_term::LaunchCfg {
                    agent: agent.clone(),
                    agent_type: inst.agent_type,
                    cwd: project.path.clone(),
                    env: Vec::new(),
                    program_override: Some(program),
                    args,
                    cols: 120,
                    rows: 32,
                    tmux_session: Some(tmux_name),
                };
                cfg.env.push(("MUXLANE_AGENT_ID".into(), agent.clone()));
                cfg.env.push((
                    "MUXLANE_SOCKET".into(),
                    self.socket_path.display().to_string(),
                ));
                cfg.env
                    .push(("MUXLANE_HOOK_TOKEN".into(), self.hook_token(agent)));
                let session = self
                    .runtime
                    .spawn_blocking(move || muxlane_term::PtySession::spawn(cfg))
                    .await
                    .context("spawn terminal handoff task")?
                    .context("spawn terminal handoff")?;
                let tmux_actual = session.tmux_session_name().map(str::to_string);
                self.sessions.lock().await.insert(agent.clone(), session);
                if let Some(a) = self
                    .state
                    .write()
                    .await
                    .agents
                    .iter_mut()
                    .find(|a| &a.id == agent)
                {
                    a.tmux_session = tmux_actual;
                }
                let _ = self.ensure_pty_bridge_for_agent(agent).await;
                self.chat.mark_has_pty(agent, true).await;
                self.dirty.bump();
                self.persist_runtime_state().await?;
                Ok(self.chat_history(agent).await?.meta)
            }
            muxlane_core::SessionDriverMode::PiRpc | muxlane_core::SessionDriverMode::Acp => {
                let p =
                    preset.ok_or_else(|| anyhow::anyhow!("preset unavailable for chat switch"))?;
                let driver_kind = p.chat_driver_kind().ok_or_else(|| {
                    anyhow::anyhow!("preset {:?} does not support native chat", p.agent_type)
                })?;
                let mut resume_ref = self
                    .chat
                    .session_resume_info(agent)
                    .await
                    .and_then(|(_, m)| m.provider_session_file.or(m.provider_session_id));
                if resume_ref.is_none()
                    && matches!(
                        inst.agent_type,
                        muxlane_core::model::AgentType::Pi | muxlane_core::model::AgentType::Pie
                    )
                {
                    resume_ref = muxlane_chat::transcript::discover_pi_session_file(
                        &project.path,
                        inst.agent_type == muxlane_core::model::AgentType::Pie,
                        inst.status_since.saturating_sub(3600),
                    )
                    .map(|path| path.display().to_string());
                }

                // 结束原有 PTY 后接管为原生结构化会话
                if let Some(old_pty) = self.sessions.lock().await.remove(agent) {
                    let _ = self
                        .runtime
                        .spawn_blocking(move || old_pty.kill_persistent())
                        .await;
                    if let Some(a) = self
                        .state
                        .write()
                        .await
                        .agents
                        .iter_mut()
                        .find(|a| &a.id == agent)
                    {
                        a.tmux_session = None;
                    }
                    self.subs.lock().await.mark_agent_exit(agent);
                }

                let program = p
                    .executable_in(&project.path)
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| p.program.clone());
                let args = p
                    .chat_launch_args(resume_ref.as_deref())
                    .unwrap_or_default();
                let meta = self
                    .chat
                    .start_session(
                        muxlane_chat::DriverLaunchSpec {
                            agent: agent.clone(),
                            agent_type: inst.agent_type,
                            driver_kind,
                            program,
                            args,
                            cwd: project.path,
                            env: Vec::new(),
                            provider_session: resume_ref,
                            model: None,
                            thinking_level: None,
                        },
                        false,
                        None,
                    )
                    .await?;
                self.dirty.bump();
                self.persist_runtime_state().await?;
                Ok(meta)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rename_server() -> Arc<MuxlaneServer> {
        use muxlane_core::model::{AgentType, MachineInfo};
        let mut state = crate::ServerState::new(MachineInfo {
            machine_id: "test".into(),
            name: "test".into(),
            os: "test".into(),
            version: "test".into(),
        });
        state.fixture_push(
            Project {
                id: "p".into(),
                name: "p".into(),
                path: "/tmp".into(),
                branch: None,
                agents: vec!["a".into()],
            },
            AgentInstance {
                id: "a".into(),
                project: "p".into(),
                agent_type: AgentType::Shell,
                title: "Shell".into(),
                title_is_custom: false,
                status: AgentStatus::Working,
                status_since: 42,
                seen: false,
                tmux_session: Some("muxlane-a".into()),
            },
        );
        MuxlaneServer::new(
            "/tmp/unused-muxlane-rename.sock".into(),
            Arc::new(tokio::sync::RwLock::new(state)),
            crate::DirtyFlag::new(),
        )
    }

    #[tokio::test]
    async fn rename_validates_titles_and_returns_rpc_errors_without_changing_state() {
        use muxlane_core::protocol::{methods, Request};
        let server = rename_server();
        let before = server.snapshot().await;
        for title in [
            "".into(),
            "   ".into(),
            "a\nb".into(),
            "\tname".into(),
            "name\u{7f}".into(),
            "名".repeat(129),
        ] {
            let response = server
                .handle_agent_rename(Request {
                    id: 1,
                    method: methods::AGENT_RENAME.into(),
                    params: serde_json::json!({"agent": "a", "title": title}),
                })
                .await
                .unwrap();
            let error = response.error.unwrap();
            assert_eq!(error.code, "bad_params");
            assert!(error.message.contains("title"));
            assert_eq!(server.snapshot().await, before);
        }
        let missing = server
            .handle_agent_rename(Request {
                id: 2,
                method: methods::AGENT_RENAME.into(),
                params: serde_json::json!({"agent": "missing", "title": "valid"}),
            })
            .await
            .unwrap();
        assert_eq!(missing.error.unwrap().code, "no_such_agent");
        let title = "名".repeat(128);
        let renamed = server
            .rename_agent(&"a".into(), format!(" {title} "))
            .await
            .unwrap();
        assert_eq!(renamed.title, title);
        assert!(renamed.title_is_custom);
        let mut expected = before.agents[0].clone();
        expected.title = title;
        expected.title_is_custom = true;
        assert_eq!(renamed, expected);
    }

    #[tokio::test]
    async fn rename_persists_custom_titles_and_rolls_back_on_save_failure() {
        use muxlane_core::protocol::{methods, Request};
        let server = rename_server();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        server.set_persistence_path(path.clone());
        let mut changes = server.subscribe_dirty();
        for previous_custom in [false, true] {
            let before = server.snapshot().await;
            assert_eq!(before.agents[0].title_is_custom, previous_custom);
            std::fs::write(&path, "broken state").unwrap();
            let response = server
                .handle_agent_rename(Request {
                    id: 1,
                    method: methods::AGENT_RENAME.into(),
                    params: serde_json::json!({"agent": "a", "title": "lost rename"}),
                })
                .await
                .unwrap();
            assert_eq!(
                response.error.unwrap().code,
                muxlane_core::protocol::error_codes::PERSISTENCE_FAILED
            );
            assert_eq!(server.snapshot().await, before);
            assert!(!changes.has_changed().unwrap());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken state");
            std::fs::remove_file(&path).unwrap();
            server
                .rename_agent(&"a".into(), "修复登录".into())
                .await
                .unwrap();
            let saved = muxlane_store::load(&path).unwrap();
            assert_eq!(saved.sessions[0].title, "修复登录");
            assert!(saved.sessions[0].title_is_custom);
            assert!(changes.has_changed().unwrap());
            changes.borrow_and_update();
        }
    }

    #[test]
    fn project_paths_expand_only_current_user_home() {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
        assert_eq!(expand_project_path("~").unwrap(), home);
        assert_eq!(
            expand_project_path("~/projects/example").unwrap(),
            home.join("projects/example")
        );
        let error = expand_project_path("~someone/projects").unwrap_err();
        assert_eq!(
            error.code(),
            muxlane_core::protocol::error_codes::INVALID_PATH
        );
        assert!(error.to_string().contains("~user paths are not supported"));
    }
}
