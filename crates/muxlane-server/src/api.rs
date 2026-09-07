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
            .with_context(|| format!("no such project: {}", params.project))?;
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
            status: AgentStatus::Idle,
            status_since: muxlane_core::model::now_secs(),
            seen: true,
            tmux_session: session.tmux_session_name().map(str::to_string),
        };
        self.sessions
            .lock()
            .await
            .insert(agent_id, Arc::clone(&session));
        self.state
            .write()
            .await
            .add_agent(project, instance.clone());
        self.dirty.bump();
        self.persist_runtime_state().await?;
        Ok(instance)
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
        self.sessions
            .lock()
            .await
            .insert(instance.id.clone(), session);
        self.state.write().await.add_agent(project, instance);
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
        (destroyed, failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
