//! muxlane 持久化：原子写 + 版本字段 + 安全默认值。
use muxlane_core::model::{AgentId, AgentType, Project, ProjectId, Snapshot};
use muxlane_core::{PaneId, PaneNode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn temporary_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state.json");
    path.with_file_name(format!(
        "{name}.tmp.{}",
        muxlane_core::model::new_id("write")
    ))
}

fn cleanup_temporary_files(path: &Path) {
    let Some(parent) = path.parent() else {
        return;
    };
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state.json");
    let prefix = format!("{name}.tmp.");
    let legacy = path.with_file_name(format!("{name}.tmp"));
    let _ = std::fs::remove_file(legacy);
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|candidate| candidate.starts_with(&prefix))
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

pub const STORE_VERSION: u32 = 3;
const SECRETS_VERSION: u32 = 1;

fn default_sidebar_visible() -> bool {
    true
}

fn default_sidebar_width() -> f32 {
    230.0
}

fn default_ui_scale() -> u32 {
    100
}

fn default_close_tab_shortcut() -> Option<String> {
    Some("ctrl-w".into())
}

fn default_previous_workspace_shortcut() -> Option<String> {
    None
}

fn default_next_workspace_shortcut() -> Option<String> {
    None
}

fn default_previous_tab_shortcut() -> Option<String> {
    Some("platform-alt-up".into())
}

fn default_next_tab_shortcut() -> Option<String> {
    Some("platform-alt-down".into())
}

fn default_new_tab_shortcut() -> Option<String> {
    Some("platform-alt-t".into())
}

fn default_split_right_shortcut() -> Option<String> {
    Some("platform-alt-r".into())
}

fn default_split_down_shortcut() -> Option<String> {
    Some("platform-alt-d".into())
}

fn default_terminal_preset() -> String {
    "shell".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedShortcutBindings {
    #[serde(default = "default_close_tab_shortcut")]
    pub close_tab: Option<String>,
    #[serde(default = "default_previous_workspace_shortcut")]
    pub previous_workspace: Option<String>,
    #[serde(default = "default_next_workspace_shortcut")]
    pub next_workspace: Option<String>,
    #[serde(default = "default_previous_tab_shortcut")]
    pub previous_tab: Option<String>,
    #[serde(default = "default_next_tab_shortcut")]
    pub next_tab: Option<String>,
    #[serde(default = "default_new_tab_shortcut")]
    pub new_tab: Option<String>,
    #[serde(default = "default_split_right_shortcut")]
    pub split_right: Option<String>,
    #[serde(default = "default_split_down_shortcut")]
    pub split_down: Option<String>,
}

impl Default for PersistedShortcutBindings {
    fn default() -> Self {
        Self {
            close_tab: default_close_tab_shortcut(),
            previous_workspace: default_previous_workspace_shortcut(),
            next_workspace: default_next_workspace_shortcut(),
            previous_tab: default_previous_tab_shortcut(),
            next_tab: default_next_tab_shortcut(),
            new_tab: default_new_tab_shortcut(),
            split_right: default_split_right_shortcut(),
            split_down: default_split_down_shortcut(),
        }
    }
}

impl PersistedShortcutBindings {
    /// Migrate the old defaults written before the platform-key scheme.
    /// User-defined shortcuts other than those exact legacy defaults are preserved.
    fn migrate_legacy_defaults(&mut self) {
        if matches!(
            self.previous_workspace.as_deref(),
            Some("platform-left") | Some("alt-left") | Some("platform-up")
        ) {
            self.previous_workspace = default_previous_workspace_shortcut();
        }
        if matches!(
            self.next_workspace.as_deref(),
            Some("platform-right") | Some("alt-right") | Some("platform-down")
        ) {
            self.next_workspace = default_next_workspace_shortcut();
        }
        if matches!(
            self.previous_tab.as_deref(),
            Some("platform-up" | "platform-left")
        ) {
            self.previous_tab = default_previous_tab_shortcut();
        }
        if matches!(
            self.next_tab.as_deref(),
            Some("platform-down" | "platform-right")
        ) {
            self.next_tab = default_next_tab_shortcut();
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedSecrets {
    version: u32,
    #[serde(default)]
    remote_passwords: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersistedApp {
    pub version: u32,
    #[serde(default)]
    pub initialized: bool,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub remotes: Vec<String>,
    #[serde(default)]
    pub remote_configs: Vec<PersistedRemote>,
    #[serde(default)]
    pub sessions: Vec<PersistedAgent>,
    #[serde(default)]
    pub acp_threads: Vec<PersistedAcpThread>,
    #[serde(default = "PaneNode::empty")]
    pub pane_tree: PaneNode,
    #[serde(default)]
    pub active_pane: Option<PaneId>,
    #[serde(default)]
    pub maximized_pane: Option<PaneId>,
    #[serde(default)]
    pub window: Option<WindowGeometry>,
    #[serde(default)]
    pub dark_mode: Option<bool>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub font_family: Option<String>,
    #[serde(default)]
    pub sound_enabled: Option<bool>,
    #[serde(default)]
    pub osc52_clipboard_enabled: Option<bool>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default = "default_terminal_preset")]
    pub default_terminal_preset: String,
    #[serde(default)]
    pub project_workspaces_enabled: bool,
    #[serde(default)]
    pub project_workspaces: Vec<PersistedWorkspace>,
    #[serde(default)]
    pub active_project_workspace: Option<PersistedProjectKey>,
    #[serde(default = "default_sidebar_visible")]
    pub sidebar_visible: bool,
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    #[serde(default = "default_ui_scale")]
    pub ui_scale: u32,
    #[serde(default)]
    pub shortcut_bindings: PersistedShortcutBindings,
    /// 侧栏项目自定义排序：machine_id -> 按显示顺序排列的 project_id。
    #[serde(default)]
    pub project_order: std::collections::BTreeMap<String, Vec<String>>,
}

impl PersistedApp {
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        let mut projects = snapshot.projects.clone();
        for project in &mut projects {
            project.agents.clear();
        }
        Self {
            initialized: true,
            projects,
            sessions: snapshot
                .agents
                .iter()
                .filter_map(|agent| {
                    Some(PersistedAgent {
                        agent_id: agent.id.clone(),
                        project_id: agent.project.clone(),
                        agent_type: agent.agent_type,
                        title: agent.title.clone(),
                        tmux_session: agent.tmux_session.clone()?,
                    })
                })
                .collect(),
            ..Self::default()
        }
    }

    pub fn with_ui_prefs_from(mut self, previous: &Self) -> Self {
        self.remotes = previous.remotes.clone();
        self.remote_configs = previous.remote_configs.clone();
        self.pane_tree = previous.pane_tree.clone();
        self.active_pane = previous.active_pane.clone();
        self.window = previous.window;
        self.dark_mode = previous.dark_mode;
        self.theme = previous.theme.clone();
        self.font_family = previous.font_family.clone();
        self.sound_enabled = previous.sound_enabled;
        self.osc52_clipboard_enabled = previous.osc52_clipboard_enabled;
        self.language = previous.language.clone();
        self.default_terminal_preset = previous.default_terminal_preset.clone();
        self.project_workspaces_enabled = previous.project_workspaces_enabled;
        self.project_workspaces = previous.project_workspaces.clone();
        self.active_project_workspace = previous.active_project_workspace.clone();
        self.sidebar_visible = previous.sidebar_visible;
        self.sidebar_width = previous.sidebar_width;
        self.ui_scale = previous.ui_scale;
        self.shortcut_bindings = previous.shortcut_bindings.clone();
        self.shortcut_bindings.migrate_legacy_defaults();
        self.project_order = previous.project_order.clone();
        self.acp_threads = previous.acp_threads.clone();
        self
    }
}

impl Default for PersistedApp {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            initialized: false,
            projects: vec![],
            remotes: vec![],
            remote_configs: vec![],
            sessions: vec![],
            acp_threads: vec![],
            pane_tree: PaneNode::empty(),
            active_pane: None,
            maximized_pane: None,
            window: None,
            dark_mode: None,
            theme: None,
            font_family: None,
            sound_enabled: None,
            osc52_clipboard_enabled: None,
            language: None,
            default_terminal_preset: default_terminal_preset(),
            project_workspaces_enabled: false,
            project_workspaces: vec![],
            active_project_workspace: None,
            sidebar_visible: default_sidebar_visible(),
            sidebar_width: default_sidebar_width(),
            ui_scale: default_ui_scale(),
            shortcut_bindings: PersistedShortcutBindings::default(),
            project_order: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PersistedProjectKey {
    pub machine_id: String,
    pub project_id: ProjectId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersistedWorkspace {
    pub key: PersistedProjectKey,
    #[serde(default = "PaneNode::empty")]
    pub pane_tree: PaneNode,
    #[serde(default)]
    pub active_pane: Option<PaneId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "auth", rename_all = "snake_case")]
pub enum PersistedRemoteAuth {
    SshConfig,
    PublicKey {
        #[serde(default)]
        username: Option<String>,
        #[serde(default)]
        identity_file: Option<String>,
    },
    Password {
        #[serde(default)]
        username: String,
        #[serde(default)]
        password: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedRemote {
    pub target: String,
    pub auth: PersistedRemoteAuth,
    #[serde(default)]
    pub machine_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedAgent {
    pub agent_id: AgentId,
    pub project_id: ProjectId,
    pub agent_type: AgentType,
    pub title: String,
    pub tmux_session: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PersistedAcpThread {
    #[serde(default)]
    pub ui_id: AgentId,
    #[serde(default)]
    pub project_id: ProjectId,
    #[serde(default)]
    pub profile_id: String,
    #[serde(default)]
    pub protocol_session_id: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub parent_ui_id: Option<AgentId>,
    #[serde(default)]
    pub draft: String,
}

pub const ACP_THREAD_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpThreadSaveOutcome {
    Written,
    SkippedStale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpThreadDeleteOutcome {
    Deleted,
    SkippedNewer,
    NotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersistedAcpThreadData {
    pub schema_version: u32,
    pub metadata: PersistedAcpThread,
    #[serde(default)]
    pub snapshot: muxlane_acp::ThreadSnapshot,
    #[serde(default)]
    pub queued_prompts: Vec<muxlane_acp::PromptSubmission>,
    #[serde(default)]
    pub queue_paused: bool,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub write_revision: u64,
}

impl PersistedAcpThreadData {
    pub fn new(metadata: PersistedAcpThread) -> Self {
        let now = muxlane_core::model::now_secs();
        Self {
            schema_version: ACP_THREAD_SCHEMA_VERSION,
            metadata,
            snapshot: muxlane_acp::ThreadSnapshot::default(),
            queued_prompts: Vec::new(),
            queue_paused: false,
            created_at: now,
            updated_at: now,
            write_revision: 1,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct AcpThreadLoad {
    pub records: Vec<PersistedAcpThreadData>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct WindowGeometry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub maximized: bool,
}

pub fn load(path: &Path) -> anyhow::Result<PersistedApp> {
    let result = (|| {
        if !path.exists() {
            return Ok(PersistedApp::default());
        }
        let bytes = std::fs::read(path)?;
        let mut app: PersistedApp = serde_json::from_slice(&bytes)?;
        migrate(&mut app)?;
        app.shortcut_bindings.migrate_legacy_defaults();
        let secrets_path = secrets_path(path);
        let mut secrets = load_secrets(&secrets_path)?;
        let mut migrated_passwords = false;
        for remote in &mut app.remote_configs {
            if let PersistedRemoteAuth::Password { password, .. } = &mut remote.auth {
                if let Some(inline) = password.take() {
                    secrets
                        .remote_passwords
                        .insert(remote.target.clone(), inline);
                    migrated_passwords = true;
                }
            }
        }
        if migrated_passwords {
            write_secrets(&secrets_path, &secrets)?;
            write_state(path, &app)?;
        }
        restore_passwords(&mut app, &secrets);
        Ok(app)
    })();
    cleanup_temporary_files(path);
    result
}

pub fn save(path: &Path, app: &PersistedApp) -> anyhow::Result<()> {
    let mut state = app.clone();
    let mut secrets = PersistedSecrets {
        version: SECRETS_VERSION,
        ..Default::default()
    };
    for remote in &mut state.remote_configs {
        if let PersistedRemoteAuth::Password { password, .. } = &mut remote.auth {
            if let Some(password) = password.take() {
                secrets
                    .remote_passwords
                    .insert(remote.target.clone(), password);
            }
        }
    }
    write_secrets(&secrets_path(path), &secrets)?;
    write_state(path, &state)
}

pub fn save_acp_thread(state_path: &Path, record: &PersistedAcpThreadData) -> anyhow::Result<()> {
    save_acp_thread_if_newer(state_path, record).map(|_| ())
}

pub fn save_acp_thread_if_newer(
    state_path: &Path,
    record: &PersistedAcpThreadData,
) -> anyhow::Result<AcpThreadSaveOutcome> {
    validate_thread_id(&record.metadata.ui_id)?;
    validate_thread_schema(record.schema_version)?;
    let mut record = record.clone();
    record.schema_version = ACP_THREAD_SCHEMA_VERSION;
    let path = acp_thread_path(state_path, &record.metadata.ui_id);
    if let Some(existing) = read_acp_thread_file(&path)? {
        if existing.write_revision >= record.write_revision {
            return Ok(AcpThreadSaveOutcome::SkippedStale);
        }
    }
    write_atomic(&path, &serde_json::to_vec_pretty(&record)?, Some(0o600))?;
    Ok(AcpThreadSaveOutcome::Written)
}

pub fn delete_acp_thread(state_path: &Path, ui_id: &str) -> anyhow::Result<()> {
    delete_acp_thread_if_not_newer(state_path, ui_id, u64::MAX).map(|_| ())
}

pub fn delete_acp_thread_if_not_newer(
    state_path: &Path,
    ui_id: &str,
    delete_revision: u64,
) -> anyhow::Result<AcpThreadDeleteOutcome> {
    validate_thread_id(ui_id)?;
    let path = acp_thread_path(state_path, ui_id);
    let Some(existing) = read_acp_thread_file(&path)? else {
        return Ok(AcpThreadDeleteOutcome::NotFound);
    };
    if existing.write_revision > delete_revision {
        return Ok(AcpThreadDeleteOutcome::SkippedNewer);
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(AcpThreadDeleteOutcome::Deleted),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(AcpThreadDeleteOutcome::NotFound)
        }
        Err(error) => Err(error.into()),
    }
}

pub fn load_acp_threads(state_path: &Path) -> AcpThreadLoad {
    let directory = acp_threads_dir(state_path);
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return AcpThreadLoad::default();
    };
    let mut result = AcpThreadLoad::default();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let loaded = read_acp_thread_file(&path);
        match loaded {
            Ok(Some(record)) => result.records.push(record),
            Ok(None) => {}
            Err(error) => result.errors.push(format!("{}: {error}", path.display())),
        }
    }
    result.records.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.metadata.ui_id.cmp(&right.metadata.ui_id))
    });
    result
}

fn acp_threads_dir(state_path: &Path) -> PathBuf {
    state_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("threads")
}

fn acp_thread_path(state_path: &Path, ui_id: &str) -> PathBuf {
    acp_threads_dir(state_path).join(format!("{ui_id}.json"))
}

fn validate_thread_schema(schema_version: u32) -> anyhow::Result<()> {
    if schema_version > ACP_THREAD_SCHEMA_VERSION {
        anyhow::bail!(
            "thread schema {} is newer than supported {}",
            schema_version,
            ACP_THREAD_SCHEMA_VERSION
        );
    }
    Ok(())
}

fn read_acp_thread_file(path: &Path) -> anyhow::Result<Option<PersistedAcpThreadData>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let record: PersistedAcpThreadData = serde_json::from_slice(&bytes)?;
    validate_thread_id(&record.metadata.ui_id)?;
    validate_thread_schema(record.schema_version)?;
    let expected_name = format!("{}.json", record.metadata.ui_id);
    if path.file_name().and_then(|name| name.to_str()) != Some(expected_name.as_str()) {
        anyhow::bail!("thread file name does not match its ui_id");
    }
    Ok(Some(record))
}
fn validate_thread_id(ui_id: &str) -> anyhow::Result<()> {
    if ui_id.starts_with("acp_")
        && ui_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        Ok(())
    } else {
        anyhow::bail!("invalid ACP thread id")
    }
}

fn secrets_path(state_path: &Path) -> PathBuf {
    state_path.with_file_name("secrets.json")
}

fn load_secrets(path: &Path) -> anyhow::Result<PersistedSecrets> {
    if !path.exists() {
        return Ok(PersistedSecrets {
            version: SECRETS_VERSION,
            ..Default::default()
        });
    }
    let secrets: PersistedSecrets = serde_json::from_slice(&std::fs::read(path)?)?;
    if secrets.version > SECRETS_VERSION {
        anyhow::bail!(
            "secrets version {} is newer than supported {}",
            secrets.version,
            SECRETS_VERSION
        );
    }
    Ok(secrets)
}

fn restore_passwords(app: &mut PersistedApp, secrets: &PersistedSecrets) {
    for remote in &mut app.remote_configs {
        if let PersistedRemoteAuth::Password { password, .. } = &mut remote.auth {
            *password = secrets.remote_passwords.get(&remote.target).cloned();
        }
    }
}

fn write_state(path: &Path, app: &PersistedApp) -> anyhow::Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(app)?, None)
}

fn write_secrets(path: &Path, secrets: &PersistedSecrets) -> anyhow::Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(secrets)?, Some(0o600))
}

fn write_atomic(path: &Path, data: &[u8], mode: Option<u32>) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = temporary_path(path);
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(mode);
        }
        let mut file = options.open(&tmp)?;
        use std::io::Write;
        file.write_all(data)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
        }
        Ok::<_, anyhow::Error>(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn migrate(app: &mut PersistedApp) -> anyhow::Result<()> {
    if app.version > STORE_VERSION {
        anyhow::bail!(
            "state version {} is newer than supported {}",
            app.version,
            STORE_VERSION
        );
    }
    // v1 -> v2: legacy pane_tree/active_pane already represent the shared layout.
    // New project workspaces and sidebar preferences use serde defaults.
    // v2 -> v3: ACP thread metadata is optional and restored lazily.
    if app.version < 3 {
        app.acp_threads = Vec::new();
    }
    if app.version < 2 {
        // OSC52 clipboard used to default to off, which silently broke tmux
        // mouse-selection copy. Flip it on once during the v1 -> v2 upgrade
        // (this may override a legacy explicit `false` a single time); in v2
        // users can still disable it and that choice is preserved.
        app.osc52_clipboard_enabled = Some(true);
    }
    if app.remote_configs.is_empty() && !app.remotes.is_empty() {
        app.remote_configs = app
            .remotes
            .drain(..)
            .map(|target| PersistedRemote {
                target,
                auth: PersistedRemoteAuth::SshConfig,
                machine_id: None,
            })
            .collect();
    }
    app.version = STORE_VERSION;
    Ok(())
}

pub fn default_path(data_dir: &Path) -> PathBuf {
    data_dir.join("state.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn from_snapshot_keeps_only_persistable_runtime_state() {
        let snapshot = Snapshot {
            machine: None,
            projects: vec![Project {
                id: "p".into(),
                name: "repo".into(),
                path: "/tmp/repo".into(),
                branch: Some("main".into()),
                agents: vec!["tmux-agent".into(), "plain-agent".into()],
            }],
            agents: vec![
                muxlane_core::model::AgentInstance {
                    id: "tmux-agent".into(),
                    project: "p".into(),
                    agent_type: AgentType::Claude,
                    title: "work".into(),
                    status: muxlane_core::model::AgentStatus::Working,
                    status_since: 1,
                    seen: false,
                    tmux_session: Some("muxlane-tmux-agent".into()),
                },
                muxlane_core::model::AgentInstance {
                    id: "plain-agent".into(),
                    project: "p".into(),
                    agent_type: AgentType::Shell,
                    title: "shell".into(),
                    status: muxlane_core::model::AgentStatus::Idle,
                    status_since: 2,
                    seen: true,
                    tmux_session: None,
                },
            ],
        };

        let mut previous = PersistedApp::default();
        let pane = previous.pane_tree.first_pane_id();
        previous.pane_tree.open_tab(&pane, "tmux-agent".into());
        previous.theme = Some("nord".into());
        previous.osc52_clipboard_enabled = Some(true);
        previous.project_workspaces_enabled = true;
        previous.active_project_workspace = Some(PersistedProjectKey {
            machine_id: "machine-a".into(),
            project_id: "p".into(),
        });
        previous.project_workspaces.push(PersistedWorkspace {
            key: previous.active_project_workspace.clone().unwrap(),
            pane_tree: previous.pane_tree.clone(),
            active_pane: Some(pane.clone()),
        });
        previous.sidebar_visible = false;
        previous.sidebar_width = 312.0;
        previous.shortcut_bindings.close_tab = Some("ctrl-q".into());
        previous.shortcut_bindings.next_tab = None;
        previous.maximized_pane = Some(pane);
        let app = PersistedApp::from_snapshot(&snapshot).with_ui_prefs_from(&previous);

        assert!(app.initialized);
        assert!(app.projects[0].agents.is_empty());
        assert_eq!(app.projects[0].branch.as_deref(), Some("main"));
        assert_eq!(
            app.sessions,
            vec![PersistedAgent {
                agent_id: "tmux-agent".into(),
                project_id: "p".into(),
                agent_type: AgentType::Claude,
                title: "work".into(),
                tmux_session: "muxlane-tmux-agent".into(),
            }]
        );
        assert_eq!(app.theme.as_deref(), Some("nord"));
        assert_eq!(app.osc52_clipboard_enabled, Some(true));
        assert_eq!(app.pane_tree, previous.pane_tree);
        assert!(app.project_workspaces_enabled);
        assert_eq!(
            app.active_project_workspace,
            previous.active_project_workspace
        );
        assert_eq!(app.project_workspaces, previous.project_workspaces);
        assert!(!app.sidebar_visible);
        assert_eq!(app.sidebar_width, 312.0);
        assert_eq!(app.shortcut_bindings, previous.shortcut_bindings);
        assert!(app.maximized_pane.is_none());
    }

    #[test]
    fn from_empty_snapshot_has_empty_projects_and_sessions() {
        let app = PersistedApp::from_snapshot(&Snapshot::default());
        assert!(app.initialized);
        assert!(app.projects.is_empty());
        assert!(app.sessions.is_empty());
    }

    #[test]
    fn concurrent_saves_leave_valid_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = std::sync::Arc::new(dir.path().join("state.json"));
        let app = std::sync::Arc::new(PersistedApp::default());
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let path = std::sync::Arc::clone(&path);
                let app = std::sync::Arc::clone(&app);
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        save(&path, &app).unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        load(&path).unwrap();
    }
    #[test]
    fn roundtrip_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state.json");
        let mut app = PersistedApp {
            ui_scale: 125,
            ..Default::default()
        };
        app.remote_configs.push(PersistedRemote {
            target: "user@nuc:/tmp/muxlane.sock".into(),
            auth: PersistedRemoteAuth::SshConfig,
            machine_id: Some("machine-nuc".into()),
        });
        app.sessions.push(PersistedAgent {
            agent_id: "a".into(),
            project_id: "p".into(),
            agent_type: AgentType::Shell,
            title: "zsh".into(),
            tmux_session: "muxlane-a".into(),
        });
        app.window = Some(WindowGeometry {
            x: 10.0,
            y: 20.0,
            width: 1200.0,
            height: 800.0,
            maximized: true,
        });
        let root = app.pane_tree.first_pane_id();
        app.pane_tree.open_tab(&root, "a".into());
        let second = app
            .pane_tree
            .split(&root, muxlane_core::SplitAxis::Horizontal, "b".into())
            .unwrap();
        app.active_pane = Some(second.clone());
        app.project_workspaces_enabled = true;
        app.active_project_workspace = Some(PersistedProjectKey {
            machine_id: "machine-a".into(),
            project_id: "p".into(),
        });
        app.project_workspaces.push(PersistedWorkspace {
            key: app.active_project_workspace.clone().unwrap(),
            pane_tree: app.pane_tree.clone(),
            active_pane: Some(second.clone()),
        });
        app.project_workspaces.push(PersistedWorkspace {
            key: PersistedProjectKey {
                machine_id: "machine-b".into(),
                project_id: "p".into(),
            },
            pane_tree: PaneNode::with_tab("remote-agent".into()),
            active_pane: None,
        });
        app.projects.push(Project {
            id: "p".into(),
            name: "repo".into(),
            path: "/tmp/repo".into(),
            branch: Some("main".into()),
            agents: vec![],
        });
        save(&p, &app).unwrap();
        let json = std::fs::read_to_string(&p).unwrap();
        assert!(json.contains("\"sessions\""));
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value["project_workspaces"].is_array());
        assert_eq!(
            value["project_workspaces"][0]["key"]["machine_id"],
            "machine-a"
        );
        assert_eq!(value["project_workspaces"][0]["key"]["project_id"], "p");
        let back = load(&p).unwrap();
        assert_eq!(back, app);
        assert!(std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .all(|entry| entry.file_name() != "state.json.tmp"));
    }

    #[test]
    fn passwords_are_stored_separately_and_restored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut app = PersistedApp::default();
        app.remote_configs.push(PersistedRemote {
            target: "alice@nuc".into(),
            auth: PersistedRemoteAuth::Password {
                username: "alice".into(),
                password: Some("correct horse battery staple".into()),
            },
            machine_id: None,
        });

        save(&path, &app).unwrap();

        let state = std::fs::read_to_string(&path).unwrap();
        assert!(!state.contains("correct horse battery staple"));
        assert_eq!(load(&path).unwrap(), app);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.path().join("secrets.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn inline_password_is_migrated_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(
            &path,
            r#"{"version":1,"remote_configs":[{"target":"alice@nuc","auth":{"auth":"password","username":"alice","password":"legacy-secret"}}]}"#,
        )
        .unwrap();

        let first = load(&path).unwrap();
        let second = load(&path).unwrap();

        assert_eq!(first, second);
        assert!(matches!(
            &first.remote_configs[0].auth,
            PersistedRemoteAuth::Password { password: Some(password), .. }
                if password == "legacy-secret"
        ));
        assert!(!std::fs::read_to_string(path)
            .unwrap()
            .contains("legacy-secret"));
    }

    #[test]
    fn legacy_remote_targets_migrate_to_remote_configs() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state.json");
        std::fs::write(
            &p,
            r#"{"version":1,"remotes":["user@nuc"],"projects":[],"sessions":[]}"#,
        )
        .unwrap();
        let app = load(&p).unwrap();
        assert!(app.remotes.is_empty());
        assert_eq!(app.remote_configs.len(), 1);
        assert_eq!(app.remote_configs[0].target, "user@nuc");
        assert_eq!(app.remote_configs[0].machine_id, None);
    }

    #[test]
    fn legacy_remote_config_without_machine_id_is_compatible() {
        let remote: PersistedRemote =
            serde_json::from_str(r#"{"target":"user@nuc","auth":{"auth":"ssh_config"}}"#).unwrap();
        assert_eq!(remote.target, "user@nuc");
        assert_eq!(remote.machine_id, None);
    }

    #[test]
    fn remote_machine_id_roundtrips() {
        let remote = PersistedRemote {
            target: "user@nuc".into(),
            auth: PersistedRemoteAuth::SshConfig,
            machine_id: Some("machine-stable".into()),
        };
        let json = serde_json::to_string(&remote).unwrap();
        assert_eq!(
            serde_json::from_str::<PersistedRemote>(&json).unwrap(),
            remote
        );
    }

    #[test]
    fn version_one_layout_migrates_to_shared_with_new_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(
            &path,
            r#"{"version":1,"pane_tree":{"kind":"leaf","group":{"id":"pane-old","tabs":["a"],"active":"a"}},"active_pane":"pane-old"}"#,
        )
        .unwrap();

        let app = load(&path).unwrap();
        assert_eq!(app.version, STORE_VERSION);
        assert_eq!(app.active_pane.as_deref(), Some("pane-old"));
        assert!(app.pane_tree.pane_for_agent(&"a".into()).is_some());
        assert!(!app.project_workspaces_enabled);
        assert!(app.project_workspaces.is_empty());
        assert!(app.sidebar_visible);
        assert_eq!(app.sidebar_width, 230.0);
    }

    #[test]
    fn v1_missing_osc52_field_is_flipped_on_during_migration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, r#"{"version":1}"#).unwrap();

        let app = load(&path).unwrap();
        assert_eq!(app.version, STORE_VERSION);
        assert_eq!(app.osc52_clipboard_enabled, Some(true));
    }

    #[test]
    fn v1_explicit_osc52_false_is_overwritten_once_during_migration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, r#"{"version":1,"osc52_clipboard_enabled":false}"#).unwrap();

        let app = load(&path).unwrap();
        assert_eq!(app.osc52_clipboard_enabled, Some(true));
    }

    #[test]
    fn v2_explicit_osc52_false_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, r#"{"version":2,"osc52_clipboard_enabled":false}"#).unwrap();

        let app = load(&path).unwrap();
        assert_eq!(app.osc52_clipboard_enabled, Some(false));
    }

    #[test]
    fn v2_missing_osc52_field_stays_unset_for_default_true() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, r#"{"version":2}"#).unwrap();

        let app = load(&path).unwrap();
        // None means "no explicit user choice": the app layer defaults to true.
        assert_eq!(app.osc52_clipboard_enabled, None);
    }

    #[test]
    fn shortcut_bindings_use_defaults_for_missing_fields_and_preserve_null() {
        let bindings: PersistedShortcutBindings =
            serde_json::from_str(r#"{"close_tab":"ctrl-q","next_tab":null}"#).unwrap();

        assert_eq!(bindings.close_tab.as_deref(), Some("ctrl-q"));
        assert_eq!(bindings.previous_workspace, None);
        assert_eq!(bindings.next_workspace, None);
        assert_eq!(bindings.previous_tab.as_deref(), Some("platform-alt-up"));
        assert_eq!(bindings.next_tab, None);
        assert_eq!(bindings.new_tab.as_deref(), Some("platform-alt-t"));
        assert_eq!(bindings.split_right.as_deref(), Some("platform-alt-r"));
        assert_eq!(bindings.split_down.as_deref(), Some("platform-alt-d"));
    }

    #[test]
    fn legacy_state_without_shortcuts_receives_defaults_without_rewriting_version() {
        let app: PersistedApp = serde_json::from_str(r#"{"version":2}"#).unwrap();
        assert_eq!(app.version, 2);
        assert_eq!(app.shortcut_bindings, PersistedShortcutBindings::default());
    }

    #[test]
    fn acp_thread_metadata_roundtrips_without_messages_or_secrets() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        let mut app = PersistedApp::default();
        app.acp_threads.push(PersistedAcpThread {
            ui_id: "acp_1".into(),
            project_id: "project_1".into(),
            profile_id: "codex".into(),
            protocol_session_id: Some("session_1".into()),
            title: "Codex UI".into(),
            parent_ui_id: None,
            draft: "continue the refactor".into(),
        });

        save(&path, &app).unwrap();
        let restored = load(&path).unwrap();

        assert_eq!(restored.acp_threads, app.acp_threads);
        let json = std::fs::read_to_string(path).unwrap();
        assert!(!json.contains("api_key"));
        assert!(!json.contains("messages"));
    }

    #[test]
    fn acp_thread_data_roundtrips_and_rejects_path_traversal() {
        let directory = tempfile::tempdir().unwrap();
        let state_path = directory.path().join("state.json");
        let metadata = PersistedAcpThread {
            ui_id: "acp_thread_1".into(),
            project_id: "project_1".into(),
            profile_id: "claude".into(),
            protocol_session_id: Some("session_1".into()),
            title: "Thread".into(),
            parent_ui_id: None,
            draft: "draft".into(),
        };
        let mut record = PersistedAcpThreadData::new(metadata);
        record.snapshot.revision = 7;
        record
            .queued_prompts
            .push(muxlane_acp::PromptSubmission::new(
                muxlane_acp::PromptPayload::new("next"),
            ));
        record.queue_paused = true;

        save_acp_thread(&state_path, &record).unwrap();
        let loaded = load_acp_threads(&state_path);
        assert!(loaded.errors.is_empty());
        assert_eq!(loaded.records.len(), 1);
        assert_eq!(loaded.records[0].snapshot.revision, 7);
        assert_eq!(loaded.records[0].write_revision, 1);
        assert_eq!(loaded.records[0].schema_version, ACP_THREAD_SCHEMA_VERSION);
        assert_eq!(loaded.records[0].queued_prompts[0].payload.text, "next");
        assert!(loaded.records[0].queue_paused);

        let mut invalid = record.clone();
        invalid.metadata.ui_id = "../escape".into();
        assert!(save_acp_thread(&state_path, &invalid).is_err());
        delete_acp_thread(&state_path, "acp_thread_1").unwrap();
        assert!(load_acp_threads(&state_path).records.is_empty());
    }

    #[test]
    fn legacy_acp_queue_payloads_are_migrated_to_stable_submissions() {
        let record: PersistedAcpThreadData = serde_json::from_str(
            r#"{
                "schema_version": 1,
                "archived": true,
                "metadata": {"ui_id":"acp_legacy"},
                "queued_prompts": [{"text":"keep me","blocks":[]}]
            }"#,
        )
        .unwrap();
        assert_eq!(record.write_revision, 0);
        assert_eq!(record.queued_prompts.len(), 1);
        assert_eq!(record.queued_prompts[0].payload.text, "keep me");
        assert!(!record.queued_prompts[0].id.to_string().is_empty());
        let encoded = serde_json::to_string(&record).unwrap();
        assert!(!encoded.contains("archived"));
    }

    #[test]
    fn legacy_archive_flag_does_not_discard_local_thread_data() {
        let mut record = PersistedAcpThreadData::new(PersistedAcpThread {
            ui_id: "acp_legacy_archive".into(),
            project_id: "project_1".into(),
            profile_id: "claude".into(),
            protocol_session_id: Some("restorable_session".into()),
            title: "Real restored title".into(),
            draft: "unsent draft".into(),
            ..Default::default()
        });
        record
            .snapshot
            .items
            .push(muxlane_acp::ThreadItem::Message(muxlane_acp::Message {
                protocol_id: None,
                id: "user_1".into(),
                role: muxlane_acp::MessageRole::User,
                text: "Original prompt".into(),
            }));
        record
            .queued_prompts
            .push(muxlane_acp::PromptSubmission::new(
                muxlane_acp::PromptPayload::new("queued prompt"),
            ));
        record.queue_paused = true;
        for archived in [None, Some(false), Some(true)] {
            let mut json = serde_json::to_value(&record).unwrap();
            if let Some(archived) = archived {
                json["archived"] = serde_json::json!(archived);
            }
            let restored: PersistedAcpThreadData = serde_json::from_value(json).unwrap();
            assert_eq!(restored, record);
            assert!(serde_json::to_value(&restored)
                .unwrap()
                .get("archived")
                .is_none());
        }
    }

    #[test]
    fn acp_protocol_aliases_survive_store_roundtrip_without_changing_item_ids() {
        let directory = tempfile::tempdir().unwrap();
        let state_path = directory.path().join("state.json");
        let mut reducer = muxlane_acp::ThreadReducer::new();
        for (id, text) in [(None, "hello"), (Some("protocol-answer"), " world")] {
            reducer.apply(muxlane_acp::ThreadDelta::MessageChunk {
                id: id.map(str::to_string), role: muxlane_acp::MessageRole::Assistant, text: text.into(),
            });
        }
        let mut record = PersistedAcpThreadData::new(PersistedAcpThread {
            ui_id: "acp_protocol_alias".into(), ..Default::default()
        });
        record.snapshot = reducer.into_snapshot();
        save_acp_thread(&state_path, &record).unwrap();
        let loaded = load_acp_threads(&state_path);
        assert!(loaded.errors.is_empty());
        assert_eq!(loaded.records, vec![record]);
        let muxlane_acp::ThreadItem::Message(message) = &loaded.records[0].snapshot.items[0] else { panic!("expected message") };
        assert_eq!(message.id, "local-message-1");
        assert_eq!(message.protocol_id.as_deref(), Some("protocol-answer"));
    }

    #[test]
    fn unknown_acp_agent_id_survives_restore_and_resave() {
        let directory = tempfile::tempdir().unwrap();
        let state_path = directory.path().join("state.json");
        let record = PersistedAcpThreadData::new(PersistedAcpThread {
            ui_id: "acp_unknown".into(),
            profile_id: "removed-custom-agent".into(),
            protocol_session_id: Some("original-session".into()),
            title: "Original title".into(),
            draft: "unsent prompt".into(),
            ..Default::default()
        });
        save_acp_thread(&state_path, &record).unwrap();
        let restored = load_acp_threads(&state_path).records.remove(0);
        assert!(muxlane_acp::AgentRegistry::default()
            .require(&restored.metadata.profile_id)
            .is_err());
        save_acp_thread(&state_path, &restored).unwrap();
        assert_eq!(load_acp_threads(&state_path).records, vec![record]);
    }

    #[test]
    fn acp_revision_guards_writes_and_deletes() {
        let directory = tempfile::tempdir().unwrap();
        let state_path = directory.path().join("state.json");
        let metadata = PersistedAcpThread {
            ui_id: "acp_revision".into(),
            ..Default::default()
        };
        let mut newer = PersistedAcpThreadData::new(metadata);
        newer.write_revision = 2;
        newer.updated_at = 42;
        newer.snapshot.revision = 2;
        assert_eq!(
            save_acp_thread_if_newer(&state_path, &newer).unwrap(),
            AcpThreadSaveOutcome::Written
        );

        let mut stale = newer.clone();
        stale.write_revision = 1;
        stale.updated_at = 99;
        stale.snapshot.revision = 1;
        assert_eq!(
            save_acp_thread_if_newer(&state_path, &stale).unwrap(),
            AcpThreadSaveOutcome::SkippedStale
        );
        assert_eq!(
            load_acp_threads(&state_path).records[0].snapshot.revision,
            2
        );
        assert_eq!(load_acp_threads(&state_path).records[0].updated_at, 42);

        let mut equal = newer.clone();
        equal.snapshot.revision = 3;
        assert_eq!(
            save_acp_thread_if_newer(&state_path, &equal).unwrap(),
            AcpThreadSaveOutcome::SkippedStale
        );

        let mut latest = newer.clone();
        latest.write_revision = 3;
        latest.snapshot.revision = 4;
        assert_eq!(
            save_acp_thread_if_newer(&state_path, &latest).unwrap(),
            AcpThreadSaveOutcome::Written
        );
        assert_eq!(
            load_acp_threads(&state_path).records[0].snapshot.revision,
            4
        );

        assert_eq!(
            delete_acp_thread_if_not_newer(&state_path, "acp_revision", 2).unwrap(),
            AcpThreadDeleteOutcome::SkippedNewer
        );
        assert_eq!(
            delete_acp_thread_if_not_newer(&state_path, "acp_revision", 3).unwrap(),
            AcpThreadDeleteOutcome::Deleted
        );
    }

    #[test]
    fn legacy_acp_schema_defaults_revision_and_roundtrips() {
        let record: PersistedAcpThreadData = serde_json::from_str(
            r#"{"schema_version":1,"metadata":{"ui_id":"acp_legacy_revision"}}"#,
        )
        .unwrap();
        assert_eq!(record.write_revision, 0);
        let encoded = serde_json::to_string(&record).unwrap();
        let decoded: PersistedAcpThreadData = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, record);
    }
    #[test]
    fn acp_thread_load_reports_corrupt_and_mismatched_files() {
        let directory = tempfile::tempdir().unwrap();
        let state_path = directory.path().join("state.json");
        let record = PersistedAcpThreadData::new(PersistedAcpThread {
            ui_id: "acp_valid".into(),
            ..Default::default()
        });
        save_acp_thread(&state_path, &record).unwrap();
        let threads = acp_threads_dir(&state_path);
        std::fs::write(threads.join("broken.json"), "not json").unwrap();
        std::fs::write(
            threads.join("wrong-name.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();

        let loaded = load_acp_threads(&state_path);
        assert_eq!(loaded.records.len(), 1);
        assert_eq!(loaded.errors.len(), 2);
    }

    #[test]
    fn v2_state_defaults_to_no_acp_threads() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        std::fs::write(&path, r#"{"version":2}"#).unwrap();

        let restored = load(&path).unwrap();

        assert_eq!(restored.version, STORE_VERSION);
        assert!(restored.acp_threads.is_empty());
    }

    #[test]
    fn legacy_alt_workspace_defaults_migrate_to_platform_keys() {
        let mut bindings = PersistedShortcutBindings {
            previous_workspace: Some("alt-left".into()),
            next_workspace: Some("alt-right".into()),
            previous_tab: Some("ctrl-alt-p".into()),
            ..Default::default()
        };
        bindings.migrate_legacy_defaults();
        assert_eq!(bindings.previous_workspace, None);
        assert_eq!(bindings.next_workspace, None);
    }

    #[test]
    fn legacy_platform_defaults_migrate_to_their_new_actions() {
        let mut bindings = PersistedShortcutBindings {
            previous_workspace: Some("platform-left".into()),
            next_workspace: Some("platform-right".into()),
            previous_tab: Some("platform-up".into()),
            next_tab: Some("platform-down".into()),
            ..Default::default()
        };
        bindings.migrate_legacy_defaults();
        assert_eq!(bindings.previous_workspace, None);
        assert_eq!(bindings.next_workspace, None);
        assert_eq!(bindings.previous_tab.as_deref(), Some("platform-alt-up"));
        assert_eq!(bindings.next_tab.as_deref(), Some("platform-alt-down"));

        let mut partial = PersistedShortcutBindings {
            previous_workspace: Some("platform-left".into()),
            next_workspace: None,
            previous_tab: Some("ctrl-alt-p".into()),
            next_tab: Some("platform-down".into()),
            ..Default::default()
        };
        partial.migrate_legacy_defaults();
        assert_eq!(partial.previous_workspace, None);
        assert_eq!(partial.next_workspace, None);
        assert_eq!(partial.previous_tab.as_deref(), Some("ctrl-alt-p"));
        assert_eq!(partial.next_tab.as_deref(), Some("platform-alt-down"));
    }

    #[test]
    fn shortcut_bindings_roundtrip_with_disabled_values() {
        let mut app = PersistedApp::default();
        app.shortcut_bindings.close_tab = None;
        app.shortcut_bindings.previous_tab = Some("ctrl-alt-b".into());
        app.shortcut_bindings.new_tab = None;
        app.shortcut_bindings.split_right = Some("ctrl-alt-r".into());
        app.shortcut_bindings.split_down = None;
        let json = serde_json::to_string(&app).unwrap();
        let restored: PersistedApp = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.shortcut_bindings, app.shortcut_bindings);
    }

    #[test]
    fn missing_is_default() {
        let dir = tempfile::tempdir().unwrap();
        let app = load(&dir.path().join("missing.json")).unwrap();
        assert_eq!(app.version, STORE_VERSION);
    }

    #[test]
    fn default_terminal_preset_survives_load_and_ui_preference_merge() {
        let legacy: PersistedApp = serde_json::from_str(r#"{"version":3}"#).unwrap();
        assert_eq!(legacy.default_terminal_preset, "shell");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        for id in [
            "shell",
            "claude",
            "codex",
            "pi",
            "opencode",
            "unknown-preset",
        ] {
            let previous = PersistedApp {
                default_terminal_preset: id.into(),
                ..Default::default()
            };
            save(&path, &previous).unwrap();
            let restored = load(&path).unwrap();
            let merged =
                PersistedApp::from_snapshot(&Snapshot::default()).with_ui_prefs_from(&restored);
            assert_eq!(merged.default_terminal_preset, id);
        }
    }

    #[test]
    fn left_right_defaults_migrate_but_custom_and_disabled_values_survive_load() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.json");
        std::fs::write(&path, r#"{"version":3,"shortcut_bindings":{"previous_tab":"platform-left","next_tab":"platform-right","new_tab":null,"split_right":"ctrl-alt-r","split_down":null}}"#).unwrap();
        let bindings = load(&path).unwrap().shortcut_bindings;
        assert_eq!(bindings.previous_tab.as_deref(), Some("platform-alt-up"));
        assert_eq!(bindings.next_tab.as_deref(), Some("platform-alt-down"));
        assert_eq!(bindings.new_tab, None);
        assert_eq!(bindings.split_right.as_deref(), Some("ctrl-alt-r"));
        assert_eq!(bindings.split_down, None);
    }
    #[test]
    fn future_version_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state.json");
        std::fs::write(&p, r#"{"version":999}"#).unwrap();
        assert!(load(&p).is_err());
    }

    #[test]
    fn corrupted_json_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state.json");
        std::fs::write(&p, b"{truncated").unwrap();
        assert!(load(&p).is_err());
    }
}
