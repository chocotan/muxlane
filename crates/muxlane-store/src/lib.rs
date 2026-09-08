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
