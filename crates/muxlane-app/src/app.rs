//! MuxlaneApp：根组件。侧栏（机器树+通知）+ 贴边终端网格。
use crate::ui_scale::px as ui_px;
#[path = "palette.rs"]
pub(crate) mod palette;
#[path = "panes.rs"]
mod panes;
#[path = "sidebar.rs"]
mod sidebar;
use self::sidebar::SidebarDividerDrag;
use crate::acp_view::{AcpView, AcpViewInit};
use crate::actions::*;
use crate::dialogs::ConnectAuthMode;
use crate::i18n::{self, Language};
use crate::icons::svg_asset;
use crate::menus::{
    dismiss_context_menus, BootstrapConfirm, DeleteConfirm, PendingProjectCreation, SessionMenu,
    TreeMenu,
};
use crate::notifications::{NotificationCenter, NotificationCenterEvent, NotificationDraft};
use crate::persistence::PersistenceWriter;
use crate::settings::{SettingsPage, DEFAULT_FONT_FAMILY, FONT_FAMILIES};
use crate::shortcuts::{ShortcutAction, ShortcutError};
use crate::sidebar_state::{SidebarState, SIDEBAR_RAIL_WIDTH};
use crate::term_view::TermView;
use crate::text_field::TextField;
use crate::theme::{Theme, ThemeMode};
use crate::widgets::{hover_tip, semantic_button};
use crate::workspace::{ProjectKey, WorkspaceController};
use gpui::{
    div, prelude::*, px, rgba, size, App, AssetSource, Bounds, Context, Entity, FocusHandle,
    Focusable, MouseButton, ParentElement, Render, ScrollHandle, SharedString, Styled,
    Subscription, Window, WindowBounds, WindowOptions,
};
use muxlane_core::model::{AgentId, Snapshot};
use muxlane_core::{PaneId, PaneNode, SplitAxis};
use muxlane_server::MuxlaneServer;
use palette::{NewSessionTarget, PaletteColumn, SessionCreationMode};
use panes::{DividerDrag, SplitDrag};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

struct Assets;

fn select_initial_project(
    persisted_current: Option<ProjectKey>,
    local_machine_id: &str,
    available_local_projects: &[String],
    fallback_agent_project: Option<String>,
) -> Option<ProjectKey> {
    match persisted_current {
        Some(key)
            if key.machine_id != local_machine_id
                || available_local_projects.contains(&key.project_id) =>
        {
            Some(key)
        }
        _ => fallback_agent_project
            .filter(|project_id| available_local_projects.contains(project_id))
            .or_else(|| available_local_projects.first().cloned())
            .map(|project_id| ProjectKey::new(local_machine_id, project_id)),
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(svg_asset(path).map(Cow::Borrowed))
    }

    fn list(&self, _path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

pub fn launch(
    server: Arc<MuxlaneServer>,
    initial_snapshot: Snapshot,
    connect_to: Vec<String>,
    persisted: muxlane_store::PersistedApp,
    store_path: PathBuf,
) {
    gpui_platform::application()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            crate::shortcuts::install_keymap_or_defaults(cx, &persisted.shortcut_bindings);
            let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
            let _ = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    window.set_window_title("Muxlane");
                    cx.new(|cx| {
                        MuxlaneApp::new(
                            window,
                            cx,
                            Arc::clone(&server),
                            initial_snapshot.clone(),
                            connect_to.clone(),
                            persisted.clone(),
                            store_path.clone(),
                        )
                    })
                },
            );
            cx.activate(true);
        });
}

pub struct MuxlaneApp {
    // 环境/基础设施
    pub(crate) focus: FocusHandle,
    pub(crate) server: Arc<MuxlaneServer>,
    pub(crate) persistence: PersistenceWriter,
    pub(crate) remote_event_tx: tokio::sync::mpsc::Sender<muxlane_client::ClientEvent>,

    // Pane/Tab 布局
    /// 递归 pane tree；每个叶节点持有一个 TabGroup
    pub(crate) pane_tree: PaneNode,
    pub(crate) active_pane: PaneId,
    pub(crate) maximized_pane: Option<PaneId>,
    pub(crate) active: Option<AgentId>,
    pub(crate) workspace: WorkspaceController,
    pub(crate) split_drag: Option<SplitDrag>,
    pub(crate) split_metrics: Arc<std::sync::Mutex<HashMap<String, f32>>>,

    // 终端缓存
    pub(crate) terms: HashMap<AgentId, Entity<TermView>>,
    pub(crate) acp_views: HashMap<AgentId, Entity<AcpView>>,
    pub(crate) acp_metadata: HashMap<AgentId, muxlane_store::PersistedAcpThread>,
    pub(crate) acp_records: HashMap<AgentId, muxlane_store::PersistedAcpThreadData>,
    pub(crate) acp_deleted: Arc<std::sync::Mutex<std::collections::HashSet<AgentId>>>,
    pub(crate) acp_dirty: std::collections::HashSet<AgentId>,
    pub(crate) acp_persist_pending: bool,
    pub(crate) mirror_cancel: HashMap<AgentId, Arc<std::sync::atomic::AtomicBool>>,

    // 快照/远程镜像
    pub(crate) last_snapshot: Snapshot,
    /// 远程机器（本地快照缓存：host name → snapshot）
    pub(crate) remotes: Vec<Arc<muxlane_client::RemoteHost>>,
    pub(crate) remote_snaps: HashMap<String, Snapshot>,
    pub(crate) remote_states: HashMap<String, muxlane_client::RemoteState>,
    /// 远端安装/升级进度（host → 进度）
    pub(crate) bootstrap_progress: HashMap<String, muxlane_client::BootstrapProgress>,

    // 通知 Entity
    pub(crate) notifications: Entity<NotificationCenter>,

    // 外观设置
    pub(crate) theme_mode: ThemeMode,
    pub(crate) font_family: String,
    pub(crate) sound_enabled: bool,
    pub(crate) osc52_clipboard_enabled: bool,
    pub(crate) language: Language,

    // 设置面板
    pub(crate) settings_open: bool,
    pub(crate) settings_page: SettingsPage,
    pub(crate) settings_theme_menu: bool,
    pub(crate) settings_font_menu: bool,
    pub(crate) settings_language_menu: bool,
    pub(crate) settings_scale_menu: bool,
    pub(crate) shortcut_bindings: muxlane_store::PersistedShortcutBindings,
    pub(crate) shortcut_capture: Option<ShortcutAction>,
    pub(crate) shortcut_capture_subscription: Option<Subscription>,
    pub(crate) shortcut_error: Option<ShortcutError>,

    // 命令面板
    pub(crate) palette_open: bool,
    palette_index: usize,
    palette_scroll: ScrollHandle,
    pub(crate) palette_input: Entity<TextField>,
    pub(crate) palette_project: Option<ProjectKey>,
    palette_project_index: usize,
    palette_project_scroll: ScrollHandle,
    palette_column: PaletteColumn,
    presets: Vec<muxlane_core::AgentPreset>,
    pub(crate) new_session_target: Option<NewSessionTarget>,
    pub(crate) session_creation_mode: SessionCreationMode,

    // 退出确认
    pub(crate) quit_confirm_open: bool,
    pub(crate) quit_confirmed: bool,
    pub(crate) quit_cancel_focus: FocusHandle,
    pub(crate) quit_exit_focus: FocusHandle,

    // 对话框
    pub(crate) connect_dialog: bool,
    pub(crate) connect_input: Entity<TextField>,
    pub(crate) connect_auth_mode: ConnectAuthMode,
    pub(crate) connect_username: Entity<TextField>,
    pub(crate) connect_password: Entity<TextField>,
    pub(crate) connect_key_path: Entity<TextField>,
    pub(crate) project_dialog: bool,
    pub(crate) remote_project_dialog: Option<String>,
    pub(crate) remote_project_input: Entity<TextField>,
    pub(crate) project_input: Entity<TextField>,
    pub(crate) dialog_error: Option<String>,

    // 菜单/确认框
    pub(crate) session_menu: Option<SessionMenu>,
    pub(crate) acp_delete_confirm: Option<AgentId>,
    pub(crate) tree_menu: Option<TreeMenu>,
    pub(crate) delete_confirm: Option<DeleteConfirm>,
    pub(crate) delete_error: Option<String>,
    pub(crate) delete_busy: bool,
    pub(crate) bootstrap_confirm: Option<BootstrapConfirm>,
    pub(crate) bootstrap_error: Option<String>,
    pub(crate) pending_project_creation: Option<PendingProjectCreation>,
    pub(crate) project_add_busy: bool,

    // 侧栏
    pub(crate) sidebar: SidebarState,
    sidebar_frame_pending: bool,
    pub(crate) collapsed_machines: std::collections::HashSet<String>,
    pub(crate) collapsed_projects: std::collections::HashSet<String>,
    /// 侧栏项目自定义排序：machine_id -> 按显示顺序排列的 project_id。
    pub(crate) project_order: std::collections::BTreeMap<String, Vec<String>>,
}

impl Focusable for MuxlaneApp {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl MuxlaneApp {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        server: Arc<MuxlaneServer>,
        initial_snapshot: Snapshot,
        connect_to: Vec<String>,
        persisted: muxlane_store::PersistedApp,
        store_path: std::path::PathBuf,
    ) -> Self {
        crate::ui_scale::set_percent(persisted.ui_scale);
        let loaded_acp_threads = muxlane_store::load_acp_threads(&store_path);
        for error in &loaded_acp_threads.errors {
            tracing::warn!(%error, "load ACP thread failed");
        }
        let stored_acp_records: HashMap<_, _> = loaded_acp_threads
            .records
            .into_iter()
            .map(|record| (record.metadata.ui_id.clone(), record))
            .collect();
        let mut persisted_acp_threads = persisted.acp_threads.clone();
        for record in stored_acp_records.values() {
            if !persisted_acp_threads
                .iter()
                .any(|thread| thread.ui_id == record.metadata.ui_id)
            {
                persisted_acp_threads.push(record.metadata.clone());
            }
        }
        // 本地状态事件 → 通知列表
        let mut local_rx = server.subscribe_events();
        cx.spawn(async move |this, cx| loop {
            match local_rx.recv().await {
                Ok(ev) => {
                    if ev.event == muxlane_core::protocol::events::AGENT_STATUS {
                        if let Ok(p) = serde_json::from_value::<
                            muxlane_core::protocol::AgentStatusEvent,
                        >(ev.params)
                        {
                            this.update(cx, |this, cx| {
                                let draft =
                                    this.notification_draft(p.agent, p.from, p.to, p.message);
                                this.notifications
                                    .update(cx, |center, cx| center.push_notification(draft, cx));
                                cx.notify();
                            })
                            .ok();
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        })
        .detach();

        // 本地状态变更时重拉快照；终端内容由 TermView 独立通知。
        let mut local_dirty = server.subscribe_dirty();
        let server_for_snapshot = Arc::clone(&server);
        cx.spawn(async move |this, cx| {
            while local_dirty.changed().await.is_ok() {
                let server = Arc::clone(&server_for_snapshot);
                let snap = cx
                    .background_spawn(async move { server.snapshot().await })
                    .await;
                if this
                    .update(cx, |this, cx| {
                        this.last_snapshot = snap;
                        if let Some(active) = this
                            .active
                            .clone()
                            .filter(|agent| this.is_acp_session(agent))
                        {
                            this.ensure_acp_started(&active, cx);
                        }
                        this.persist();
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        // ── 远程机器接入（socket 直连；SSH 隧道在 tunnel.rs）
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let remotes = Self::restore_remotes(&server, &persisted, &connect_to, &tx);
        let persistence = PersistenceWriter::new(store_path.clone());

        // 远程事件泵：StateChanged(Online) 更新快照缓存并触发 UI 刷新
        {
            cx.spawn(async move |this, cx| {
                let mut rx = rx;
                loop {
                    let Some(ev) = rx.recv().await else { break };
                    this.update(cx, |this, cx| {
                        match ev {
                            muxlane_client::ClientEvent::StateChanged { host, state } => {
                                this.remote_states.insert(host.clone(), state.clone());
                                if let muxlane_client::RemoteState::Online(snap) = &state {
                                    let mut snap = snap.clone();
                                    if let Some(active) = this.active.as_ref() {
                                        if let Some(agent) = snap.agent_mut(active) {
                                            agent.seen = true;
                                        }
                                    }
                                    let machine_id = snap
                                        .machine
                                        .as_ref()
                                        .map(|machine| machine.machine_id.clone());
                                    let remote = this
                                        .remotes
                                        .iter()
                                        .find(|remote| remote.cfg.name == host)
                                        .cloned();
                                    let previous_machine_id =
                                        remote.as_ref().and_then(|remote| remote.machine_id());
                                    let machine_id_changed =
                                        machine_id.as_deref().is_some_and(|id| {
                                            remote.as_ref().is_some_and(|remote| {
                                                remote.cache_machine_id(Some(id))
                                            })
                                        });
                                    let replaced_machine_id = crate::remotes::replaced_machine_id(
                                        previous_machine_id,
                                        machine_id.as_deref(),
                                    );
                                    let valid_agents: std::collections::HashSet<_> =
                                        snap.agents.iter().map(|agent| agent.id.clone()).collect();
                                    let mut stale_agents = machine_id
                                        .as_deref()
                                        .map(|machine_id| {
                                            let current = this.current_workspace_layout();
                                            this.workspace.save_current(current);
                                            let previously_known = this
                                                .remote_snaps
                                                .get(&host)
                                                .filter(|previous| {
                                                    previous.machine.as_ref().is_some_and(
                                                        |known_machine| {
                                                            known_machine.machine_id == machine_id
                                                        },
                                                    )
                                                })
                                                .into_iter()
                                                .flat_map(|previous| {
                                                    previous
                                                        .agents
                                                        .iter()
                                                        .map(|agent| agent.id.clone())
                                                });
                                            this.workspace.stale_agents_for_machine(
                                                machine_id,
                                                &valid_agents,
                                                previously_known,
                                            )
                                        })
                                        .unwrap_or_default();
                                    if let Some(previous) = replaced_machine_id.as_deref() {
                                        stale_agents.extend(
                                            this.workspace
                                                .known_agents_for_machine(previous)
                                                .difference(&valid_agents)
                                                .cloned(),
                                        );
                                    }
                                    let stale_agents: Vec<_> = stale_agents.into_iter().collect();
                                    let valid_projects: std::collections::HashSet<_> = snap
                                        .projects
                                        .iter()
                                        .map(|project| project.id.clone())
                                        .collect();
                                    this.remote_snaps.insert(host.clone(), snap);
                                    if let Some(previous) = replaced_machine_id.as_deref() {
                                        this.remove_machine_workspaces(previous);
                                    }
                                    let workspaces_changed =
                                        machine_id.as_deref().is_some_and(|id| {
                                            this.reconcile_machine_workspaces(id, &valid_projects)
                                        });
                                    if !stale_agents.is_empty() {
                                        this.cleanup_removed_agents(&stale_agents, cx);
                                    }
                                    this.ensure_active_terminal(cx);
                                    if machine_id_changed
                                        || workspaces_changed
                                        || !stale_agents.is_empty()
                                    {
                                        this.persist();
                                    }
                                }
                                // 到达稳态后清除进度显示
                                if !matches!(state, muxlane_client::RemoteState::Connecting(_)) {
                                    this.bootstrap_progress.remove(&host);
                                }
                            }
                            muxlane_client::ClientEvent::BootstrapProgress { host, progress } => {
                                this.bootstrap_progress.insert(host, progress);
                            }
                            muxlane_client::ClientEvent::StatusChanged {
                                host,
                                agent,
                                from,
                                to,
                                message,
                            } => {
                                if let Some(snap) = this.remote_snaps.get_mut(&host) {
                                    if let Some(a) = snap.agents.iter_mut().find(|a| a.id == agent)
                                    {
                                        a.status = to;
                                        if to.is_finished() {
                                            a.seen = this.active.as_ref() != Some(&agent);
                                        }
                                    }
                                }
                                let draft = this.notification_draft(agent, from, to, message);
                                this.notifications
                                    .update(cx, |center, cx| center.push_notification(draft, cx));
                            }
                            _ => {}
                        }
                        cx.notify();
                    })
                    .ok();
                }
            })
            .detach();
        }

        let thread_is_visible = |thread: &muxlane_store::PersistedAcpThread| {
            !stored_acp_records
                .get(&thread.ui_id)
                .is_some_and(|record| record.archived)
        };
        let first_agent = initial_snapshot
            .agents
            .first()
            .map(|agent| agent.id.clone())
            .or_else(|| {
                persisted_acp_threads
                    .iter()
                    .find(|thread| {
                        thread_is_visible(thread)
                            && initial_snapshot.project(&thread.project_id).is_some()
                    })
                    .map(|thread| thread.ui_id.clone())
            });
        let valid: std::collections::HashSet<AgentId> = initial_snapshot
            .agents
            .iter()
            .map(|agent| agent.id.clone())
            .chain(
                persisted_acp_threads
                    .iter()
                    .filter(|thread| {
                        thread_is_visible(thread)
                            && initial_snapshot.project(&thread.project_id).is_some()
                    })
                    .map(|thread| thread.ui_id.clone()),
            )
            .collect();
        let mut workspace = WorkspaceController::from_persisted(&persisted);
        let local_machine_id = initial_snapshot
            .machine
            .as_ref()
            .map(|machine| machine.machine_id.clone())
            .unwrap_or_else(|| "local".into());
        let mut missing_local: std::collections::HashSet<AgentId> = persisted
            .sessions
            .iter()
            .map(|session| session.agent_id.clone())
            .filter(|agent| !valid.contains(agent))
            .collect();
        missing_local.extend(
            workspace
                .known_agents_for_machine(&local_machine_id)
                .difference(&valid)
                .cloned(),
        );
        workspace.remove_agents(&missing_local);
        let available_local_projects: Vec<_> = initial_snapshot
            .projects
            .iter()
            .map(|project| project.id.clone())
            .collect();
        let fallback_agent_project = persisted
            .pane_tree
            .all_groups()
            .into_iter()
            .flat_map(|group| group.tabs.iter())
            .find_map(|agent| {
                initial_snapshot
                    .agent(agent)
                    .map(|instance| instance.project.clone())
                    .or_else(|| {
                        persisted_acp_threads
                            .iter()
                            .find(|thread| {
                                thread_is_visible(thread)
                                    && &thread.ui_id == agent
                                    && initial_snapshot.project(&thread.project_id).is_some()
                            })
                            .map(|thread| thread.project_id.clone())
                    })
            });
        let selected_project = select_initial_project(
            workspace.current_project().cloned(),
            &local_machine_id,
            &available_local_projects,
            fallback_agent_project,
        );
        let restored_layout = workspace.initial_layout(selected_project);
        let restored_tree = restored_layout.pane_tree;
        let restored_active = restored_layout.active_pane;
        let theme_mode = persisted
            .theme
            .as_deref()
            .and_then(ThemeMode::from_id)
            .unwrap_or_else(|| {
                if persisted.dark_mode.unwrap_or(false) {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                }
            });
        let font_family = persisted
            .font_family
            .as_deref()
            .filter(|font| FONT_FAMILIES.contains(font))
            .unwrap_or(DEFAULT_FONT_FAMILY)
            .to_string();
        let language = persisted
            .language
            .as_deref()
            .and_then(Language::from_id)
            .unwrap_or_else(Language::detect);
        let notifications = cx.new(|cx| NotificationCenter::new(theme_mode, language, cx));
        cx.subscribe_in(
            &notifications,
            window,
            |this, _center, event: &NotificationCenterEvent, window, cx| match event {
                NotificationCenterEvent::JumpToAgent(agent) => {
                    this.jump_to_agent(agent, window, cx)
                }
            },
        )
        .detach();
        let palette_input = cx.new(|cx| {
            let mut field = TextField::new(i18n::text(language, "palette.placeholder"), window, cx);
            field.set_theme_mode(theme_mode, cx);
            field
        });

        let connect_input = cx.new(|cx| {
            let mut field = TextField::new(
                i18n::text(language, "placeholder.remote_target"),
                window,
                cx,
            );
            field.set_theme_mode(theme_mode, cx);
            field
        });

        let connect_username = cx.new(|cx| {
            let mut field =
                TextField::new(i18n::text(language, "placeholder.username"), window, cx);
            field.set_theme_mode(theme_mode, cx);
            field
        });

        let connect_password = cx.new(|cx| {
            let mut field =
                TextField::new_secure(i18n::text(language, "placeholder.password"), window, cx);
            field.set_theme_mode(theme_mode, cx);
            field
        });

        let connect_key_path = cx.new(|cx| {
            let mut field =
                TextField::new(i18n::text(language, "placeholder.private_key"), window, cx);
            field.set_theme_mode(theme_mode, cx);
            field
        });

        let project_input = cx.new(|cx| {
            let mut field = TextField::new("~/projects/my-project", window, cx);
            field.set_theme_mode(theme_mode, cx);
            field
        });

        let remote_project_input = cx.new(|cx| {
            let mut field = TextField::new("~/projects/remote-project", window, cx);
            field.set_theme_mode(theme_mode, cx);
            field
        });

        let mut app = MuxlaneApp {
            focus: cx.focus_handle(),
            server,
            pane_tree: restored_tree,
            active_pane: restored_active,
            // 最大化是瞬时 UI 状态，不跨重启保留
            maximized_pane: None,
            terms: HashMap::new(),
            acp_views: HashMap::new(),
            acp_metadata: HashMap::new(),
            acp_records: stored_acp_records.clone(),
            acp_deleted: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            acp_dirty: std::collections::HashSet::new(),
            acp_persist_pending: false,
            mirror_cancel: HashMap::new(),
            active: None,
            workspace,
            last_snapshot: initial_snapshot,
            remotes,
            remote_snaps: HashMap::new(),
            remote_states: HashMap::new(),
            notifications,
            theme_mode,
            font_family,
            settings_open: false,
            settings_page: SettingsPage::General,
            settings_theme_menu: false,
            settings_font_menu: false,
            settings_language_menu: false,
            settings_scale_menu: false,
            shortcut_bindings: crate::shortcuts::normalize(&persisted.shortcut_bindings)
                .unwrap_or_default(),
            shortcut_capture: None,
            shortcut_capture_subscription: None,
            shortcut_error: None,
            sound_enabled: persisted.sound_enabled.unwrap_or(true),
            // Default on: tmux mouse-selection copy relies on OSC52 reaching the
            // system clipboard. Users can still disable it in settings.
            osc52_clipboard_enabled: persisted.osc52_clipboard_enabled.unwrap_or(true),
            language,
            palette_open: false,
            palette_index: 0,
            palette_scroll: ScrollHandle::new(),
            palette_input,
            palette_project: None,
            palette_project_index: 0,
            palette_project_scroll: ScrollHandle::new(),
            palette_column: PaletteColumn::Presets,
            presets: muxlane_core::builtin_presets(muxlane_term::default_shell_program()),
            connect_dialog: false,
            connect_input,
            connect_auth_mode: ConnectAuthMode::SshConfig,
            connect_username,
            connect_password,
            connect_key_path,
            project_dialog: false,
            remote_project_dialog: None,
            remote_project_input,
            project_input,
            dialog_error: None,
            quit_confirm_open: false,
            quit_confirmed: false,
            quit_cancel_focus: cx.focus_handle(),
            quit_exit_focus: cx.focus_handle(),
            remote_event_tx: tx.clone(),
            new_session_target: None,
            session_creation_mode: SessionCreationMode::Terminal,
            session_menu: None,
            acp_delete_confirm: None,
            tree_menu: None,
            delete_confirm: None,
            delete_error: None,
            delete_busy: false,
            bootstrap_confirm: None,
            bootstrap_error: None,
            pending_project_creation: None,
            project_add_busy: false,
            persistence,
            split_drag: None,
            split_metrics: Arc::new(std::sync::Mutex::new(HashMap::new())),
            sidebar: SidebarState::new(persisted.sidebar_visible, persisted.sidebar_width),
            sidebar_frame_pending: false,
            collapsed_machines: std::collections::HashSet::new(),
            collapsed_projects: std::collections::HashSet::new(),
            project_order: persisted.project_order.clone(),
            bootstrap_progress: HashMap::new(),
        };
        app.active = app
            .pane_tree
            .group(&app.active_pane)
            .and_then(|group| group.active.clone());
        let restorable_acp_threads: Vec<_> = persisted_acp_threads
            .iter()
            .filter(|record| {
                thread_is_visible(record) && app.last_snapshot.project(&record.project_id).is_some()
            })
            .cloned()
            .collect();
        for record in &restorable_acp_threads {
            let data = stored_acp_records
                .get(&record.ui_id)
                .cloned()
                .unwrap_or_else(|| muxlane_store::PersistedAcpThreadData::new(record.clone()));
            let Some(project_path) = app
                .last_snapshot
                .project(&record.project_id)
                .map(|project| project.path.clone())
            else {
                continue;
            };
            let profile = muxlane_acp::Profile::from_id(&record.profile_id);
            let view = cx.new(|cx| {
                AcpView::new(
                    AcpViewInit {
                        ui_id: record.ui_id.clone(),
                        project_id: record.project_id.clone(),
                        project_path: project_path.clone(),
                        profile,
                        protocol_session_id: record.protocol_session_id.clone(),
                        parent_ui_id: record.parent_ui_id.clone(),
                        title: if record.title.is_empty() {
                            "ACP".into()
                        } else {
                            record.title.clone()
                        },
                        draft: record.draft.clone(),
                        snapshot: data.snapshot.clone(),
                        queued_prompts: data.queued_prompts.clone(),
                        queue_paused: data.queue_paused,
                        theme_mode,
                        language,
                    },
                    window,
                    cx,
                )
            });
            if profile.is_none() {
                view.update(cx, |view, cx| {
                    view.apply(
                        muxlane_acp::Event::Error(muxlane_acp::SessionError {
                            kind: muxlane_acp::ErrorKind::Protocol,
                            message: "Unsupported ACP profile".into(),
                        }),
                        cx,
                    );
                });
            }
            app.register_acp_view(record.ui_id.clone(), view, window, cx);
        }
        if let Some(active) = app.active.clone().filter(|agent| app.is_acp_session(agent)) {
            app.ensure_acp_started(&active, cx);
        }
        if let Some(agent) = app
            .active
            .clone()
            .filter(|agent| app.last_snapshot.agent(agent).is_some())
        {
            let server = Arc::clone(&app.server);
            let session_agent = agent.clone();
            cx.spawn(async move |this, cx| {
                let session = cx
                    .background_spawn(async move { server.session(&session_agent).await })
                    .await;
                if let Some(session) = session {
                    let _ = this.update(cx, |this, cx| {
                        let term = Self::create_local_term(
                            agent.clone(),
                            session,
                            &this.font_family,
                            Theme::for_mode(this.theme_mode),
                            this.osc52_clipboard_enabled,
                            cx,
                        );
                        this.terms.insert(agent, term);
                        cx.notify();
                    });
                }
            })
            .detach();
        }
        let app_handle = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            app_handle
                .update(cx, |this, cx| {
                    if this.quit_confirmed {
                        true
                    } else {
                        this.show_quit_confirmation(window, cx);
                        false
                    }
                })
                .unwrap_or(true)
        });
        // 仅 UI 自动化使用；真实交互仍由用户点击 agent 打开 tab。
        if std::env::var("MUXLANE_TEST_AUTO_OPEN").as_deref() == Ok("1") {
            if let Some(id) = first_agent {
                app.open_agent(&id, window, cx);
            }
        }
        app.persist();
        app
    }

    pub(crate) fn persist(&self) {
        let remote_configs: Vec<muxlane_store::PersistedRemote> = self
            .remotes
            .iter()
            .map(|host| {
                let target = match &host.cfg.target {
                    muxlane_client::Target::Socket(path) => path.clone(),
                    muxlane_client::Target::Ssh { host, socket } if socket.is_empty() => {
                        host.clone()
                    }
                    muxlane_client::Target::Ssh { host, socket } => format!("{host}:{socket}"),
                };
                let auth = host.cfg.auth.clone().into();
                muxlane_store::PersistedRemote {
                    target,
                    auth,
                    machine_id: host.machine_id(),
                }
            })
            .collect();
        let mut app = muxlane_store::PersistedApp::from_snapshot(&self.last_snapshot);
        app.remotes = remote_configs
            .iter()
            .map(|remote| remote.target.clone())
            .collect();
        app.remote_configs = remote_configs;
        self.workspace
            .write_persisted(&mut app, self.current_workspace_layout());
        app.sidebar_visible = self.sidebar.visible;
        app.sidebar_width = self.sidebar.width;
        app.shortcut_bindings = self.shortcut_bindings.clone();
        app.dark_mode = Some(self.theme_mode.is_dark());
        app.ui_scale = crate::ui_scale::percent();
        app.project_order = self.project_order.clone();
        app.theme = Some(self.theme_mode.id().into());
        app.font_family = Some(self.font_family.clone());
        app.sound_enabled = Some(self.sound_enabled);
        app.osc52_clipboard_enabled = Some(self.osc52_clipboard_enabled);
        app.language = Some(self.language.id().into());
        app.acp_threads = self
            .acp_records
            .values()
            .map(|record| record.metadata.clone())
            .collect();
        app.acp_threads
            .sort_by(|left, right| left.ui_id.cmp(&right.ui_id));
        self.persistence.submit_app(app);
    }

    fn find_agent(&self, agent: &AgentId) -> Option<muxlane_core::model::AgentInstance> {
        if let Some(a) = self.last_snapshot.agent(agent) {
            return Some(a.clone());
        }
        for snap in self.remote_snaps.values() {
            if let Some(a) = snap.agent(agent) {
                return Some(a.clone());
            }
        }
        None
    }

    fn notification_draft(
        &self,
        agent: AgentId,
        from: muxlane_core::model::AgentStatus,
        to: muxlane_core::model::AgentStatus,
        message: Option<String>,
    ) -> NotificationDraft {
        let details = self
            .last_snapshot
            .agent(&agent)
            .map(|instance| {
                let project_name = self
                    .last_snapshot
                    .project(&instance.project)
                    .map(|project| project.name.clone())
                    .unwrap_or_else(|| "project".into());
                ("local".to_string(), project_name, instance.agent_type)
            })
            .or_else(|| {
                self.remote_snaps.iter().find_map(|(host, snapshot)| {
                    let instance = snapshot.agent(&agent)?;
                    let project_name = snapshot
                        .project(&instance.project)
                        .map(|project| project.name.clone())
                        .unwrap_or_else(|| "project".into());
                    Some((host.clone(), project_name, instance.agent_type))
                })
            })
            .unwrap_or_else(|| {
                (
                    "remote".into(),
                    "project".into(),
                    muxlane_core::model::AgentType::Unknown,
                )
            });

        NotificationDraft {
            focused: self.active.as_ref() == Some(&agent),
            agent,
            machine_name: details.0,
            project_name: details.1,
            agent_type: details.2,
            from,
            to,
            message,
            sound_enabled: self.sound_enabled,
        }
    }
}

impl MuxlaneApp {
    pub(crate) fn set_sidebar_visible(
        &mut self,
        visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar
            .set_visible(visible, Instant::now(), cx.reduce_motion());
        self.persist();
        cx.notify();
        self.schedule_sidebar_frame(window, cx);
    }

    fn schedule_sidebar_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_frame_pending || !self.sidebar.is_transitioning() {
            return;
        }
        self.sidebar_frame_pending = true;
        cx.on_next_frame(window, |this, window, cx| {
            this.sidebar_frame_pending = false;
            let now = Instant::now();
            let changed = if cx.reduce_motion() {
                this.sidebar.set_visible(this.sidebar.visible, now, true);
                true
            } else {
                this.sidebar.advance_transition(now)
            };
            if changed {
                cx.notify();
            }
            this.schedule_sidebar_frame(window, cx);
        });
    }
}

impl Render for MuxlaneApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(ui_px(16.));
        self.sync_active_terminal_focus(window, cx);
        let theme = Theme::for_mode(self.theme_mode);
        self.schedule_sidebar_frame(window, cx);
        // ── 终端网格：PaneTree 递归渲染。
        let render_tree = if let Some(max) = &self.maximized_pane {
            self.pane_tree
                .group(max)
                .cloned()
                .map(|group| PaneNode::Leaf { group })
                .unwrap_or_else(|| self.pane_tree.clone())
        } else {
            self.pane_tree.clone()
        };
        let grid = div()
            .relative()
            .flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(rgba(theme.bg0))
            .child(self.render_pane_node(render_tree, cx));

        // ── 根布局：侧栏 + 网格
        let mut root = div()
            .id("muxlane-root")
            .relative()
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &TogglePalette, window, cx| {
                this.palette_open = !this.palette_open;
                this.new_session_target = None;
                if this.palette_open {
                    if let Some(key) = this.default_palette_project(window, cx) {
                        this.palette_project_index = this
                            .available_project_keys()
                            .iter()
                            .position(|candidate| candidate == &key)
                            .unwrap_or(0);
                        this.palette_project = Some(key);
                    } else {
                        this.palette_project = None;
                    }
                    this.palette_index = 0;
                    this.palette_scroll.scroll_to_item(0);
                    dismiss_context_menus(&mut this.session_menu, &mut this.tree_menu);
                    this.palette_input.update(cx, |input, cx| input.reset(cx));
                    this.palette_input.focus_handle(cx).focus(window, cx);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                let pane = this.active_pane.clone();
                if let Some(agent) = this
                    .pane_tree
                    .group(&pane)
                    .and_then(|group| group.active.clone())
                {
                    this.close_tab(&pane, &agent, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NewShellTab, window, cx| {
                let pane = this.active_pane.clone();
                this.new_shell_tab(&pane, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                this.next_tab(window, cx);
            }))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| {
                this.prev_tab(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectTab1, window, cx| {
                this.select_tab_n(0, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectTab2, window, cx| {
                this.select_tab_n(1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectTab3, window, cx| {
                this.select_tab_n(2, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectTab4, window, cx| {
                this.select_tab_n(3, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectTab5, window, cx| {
                this.select_tab_n(4, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectTab6, window, cx| {
                this.select_tab_n(5, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectTab7, window, cx| {
                this.select_tab_n(6, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectTab8, window, cx| {
                this.select_tab_n(7, window, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectTab9, window, cx| {
                this.select_tab_n(8, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, _window, cx| {
                this.toggle_theme(cx);
            }))
            .on_action(cx.listener(|_this, _: &FocusNextPart, window, cx| {
                window.focus_next(cx);
            }))
            .on_action(cx.listener(|_this, _: &FocusPreviousPart, window, cx| {
                window.focus_prev(cx);
            }))
            .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, window, cx| {
                let terminal_is_focused = this
                    .terms
                    .values()
                    .any(|term| term.focus_handle(cx).is_focused(window));
                if this.quit_confirm_open {
                    if this.handle_quit_confirmation_key(&ev.keystroke, window, cx) {
                        cx.stop_propagation();
                    }
                } else if this.acp_delete_confirm.is_some() {
                    if ev.keystroke.key.as_str() == "escape" {
                        this.acp_delete_confirm = None;
                        this.focus.focus(window, cx);
                        cx.stop_propagation();
                        cx.notify();
                    }
                } else if this.palette_open {
                    // Palette owns Tab and its navigation keys while open; ordinary text still
                    // bubbles to the input handler.
                    let handled = this.handle_palette_key(&ev.keystroke, window, cx);
                    if handled {
                        cx.stop_propagation();
                    }
                } else if ev.keystroke.key.as_str() == "tab" && !terminal_is_focused {
                    if ev.keystroke.modifiers.shift {
                        window.focus_prev(cx);
                    } else {
                        window.focus_next(cx);
                    }
                    cx.stop_propagation();
                } else if this.notifications.read(cx).summary().2 {
                    if ev.keystroke.key.as_str() == "escape" {
                        this.notifications.update(cx, |center, cx| center.close(cx));
                        this.focus.focus(window, cx);
                        cx.stop_propagation();
                        cx.notify();
                    }
                } else if this.settings_open {
                    if ev.keystroke.key.as_str() == "escape" {
                        this.close_settings(window, cx);
                        cx.stop_propagation();
                    }
                } else if ev.keystroke.key.as_str() == "escape"
                    && (this.session_menu.is_some() || this.tree_menu.is_some())
                {
                    dismiss_context_menus(&mut this.session_menu, &mut this.tree_menu);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_drag_move::<DividerDrag>(cx.listener(
                |this, ev: &gpui::DragMoveEvent<DividerDrag>, _window, cx| {
                    if this.split_drag.is_some() {
                        this.update_split_drag(ev.event.position, cx);
                    }
                },
            ))
            .on_drag_move::<SidebarDividerDrag>(cx.listener(
                |this, ev: &gpui::DragMoveEvent<SidebarDividerDrag>, _window, cx| {
                    if this
                        .sidebar
                        .update_drag(f32::from(ev.event.position.x) / crate::ui_scale::factor())
                    {
                        cx.notify();
                    }
                },
            ))
            .on_drop::<DividerDrag>(cx.listener(|this, _drag, _window, _cx| {
                this.end_split_drag();
            }))
            .on_drop::<SidebarDividerDrag>(cx.listener(|this, _drag, _window, _cx| {
                if this.sidebar.end_drag() {
                    this.persist();
                }
            }))
            .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, _window, cx| {
                if this.split_drag.is_some() {
                    this.update_split_drag(ev.position, cx);
                }
                if this
                    .sidebar
                    .update_drag(f32::from(ev.position.x) / crate::ui_scale::factor())
                {
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _ev, _window, _cx| {
                    this.end_split_drag();
                    if this.sidebar.end_drag() {
                        this.persist();
                    }
                }),
            )
            .flex()
            .size_full()
            .bg(rgba(theme.bg0))
            .text_color(rgba(theme.fg0))
            .font_family("Noto Sans");
        let sidebar_width = self.sidebar.width;
        let sidebar_progress = self.sidebar.reveal_progress;
        let displayed_sidebar_width = self.sidebar.displayed_width();
        let sidebar_can_resize = self.sidebar.visible && !self.sidebar.is_transitioning();
        root = root.child(
            div()
                .id("sidebar-shell")
                .relative()
                .w(ui_px(displayed_sidebar_width))
                .h_full()
                .flex_none()
                .child(
                    div().size_full().overflow_hidden().child(
                        div()
                            .w(ui_px(sidebar_width))
                            .h_full()
                            .flex_none()
                            .flex()
                            .flex_col()
                            .opacity(sidebar_progress)
                            .bg(rgba(theme.bg1))
                            .child(
                                div()
                                    .id("sidebar-tree-scroll")
                                    .flex_1()
                                    .overflow_y_scroll()
                                    .child(self.render_machine_tree(theme, cx)),
                            )
                            .child(self.render_sidebar_footer(theme, cx)),
                    ),
                )
                .child(
                    semantic_button(
                        "sidebar-rail",
                        i18n::text(self.language, "sidebar.show"),
                        theme,
                    )
                    .absolute()
                    .top_0()
                    .right_0()
                    .w(ui_px(SIDEBAR_RAIL_WIDTH))
                    .h_full()
                    .bg(rgba(Theme::with_alpha(theme.line, 0x80)))
                    .hover(|style| style.bg(rgba(Theme::with_alpha(theme.accent, 0x70))))
                    .when(sidebar_can_resize, |rail| {
                        rail.tab_stop(false)
                            .cursor_col_resize()
                            .on_drag(SidebarDividerDrag, |_, _offset, _window, cx| {
                                cx.new(|_| crate::widgets::DividerDragGhost)
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(
                                    move |this, event: &gpui::MouseDownEvent, _window, cx| {
                                        this.sidebar.start_drag(
                                            f32::from(event.position.x) / crate::ui_scale::factor(),
                                        );
                                        cx.stop_propagation();
                                        cx.notify();
                                    },
                                ),
                            )
                    })
                    .when(!sidebar_can_resize, |rail| {
                        rail.cursor_pointer()
                            .tooltip(hover_tip(i18n::text(self.language, "sidebar.show")))
                            .on_click(cx.listener(|this, _event, window, cx| {
                                this.set_sidebar_visible(true, window, cx);
                            }))
                    }),
                ),
        );
        root = root.child(grid);
        if self.split_drag.is_some() || self.sidebar.drag.is_some() {
            let mut overlay = div()
                .id("layout-drag-overlay")
                .absolute()
                .size_full()
                .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, _window, cx| {
                    if this.split_drag.is_some() {
                        this.update_split_drag(ev.position, cx);
                    }
                    if this
                        .sidebar
                        .update_drag(f32::from(ev.position.x) / crate::ui_scale::factor())
                    {
                        cx.notify();
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _ev, _window, _cx| {
                        this.end_split_drag();
                        if this.sidebar.end_drag() {
                            this.persist();
                        }
                    }),
                );
            if self.sidebar.drag.is_some()
                || self.split_drag.as_ref().map(|drag| drag.axis) == Some(SplitAxis::Horizontal)
            {
                overlay = overlay.cursor_col_resize();
            } else {
                overlay = overlay.cursor_row_resize();
            }
            root = root.child(overlay);
        }

        if self.palette_open {
            root = root.child(self.render_palette(theme, cx));
        }
        if self.connect_dialog {
            root = root.child(self.render_connect_dialog(cx));
        }
        if self.project_dialog {
            root = root.child(self.render_project_dialog(cx));
        }
        if self.remote_project_dialog.is_some() {
            root = root.child(self.render_remote_project_dialog(cx));
        }
        if self.session_menu.is_some() || self.tree_menu.is_some() {
            root = root.child(
                div()
                    .id("context-menu-backdrop")
                    .absolute()
                    .size_full()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _event, window, cx| {
                            if dismiss_context_menus(&mut this.session_menu, &mut this.tree_menu) {
                                if let Some(active) = this.active.clone() {
                                    this.focus_agent(&active, window, cx);
                                }
                                cx.notify();
                            }
                            cx.stop_propagation();
                        }),
                    ),
            );
        }
        if self.session_menu.is_some() {
            root = root.child(self.render_session_menu(cx));
        }
        if self.acp_delete_confirm.is_some() {
            root = root.child(self.render_acp_delete_confirm(cx));
        }
        if self.tree_menu.is_some() {
            root = root.child(self.render_tree_menu(cx));
        }
        if self.delete_confirm.is_some() {
            root = root.child(self.render_delete_confirm(cx));
        }
        if self.pending_project_creation.is_some() {
            root = root.child(self.render_project_create_confirm(cx));
        }
        if self.bootstrap_confirm.is_some() {
            root = root.child(self.render_bootstrap_confirm(cx));
        }
        root = root.child(self.notifications.clone());
        if self.settings_open {
            root = root.child(self.render_settings(cx));
        }
        if self.quit_confirm_open {
            root = root.child(self.render_quit_confirmation(cx));
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_rejects_stale_local_current_and_uses_valid_agent_project() {
        let selected = select_initial_project(
            Some(ProjectKey::new("local", "deleted")),
            "local",
            &["first".into(), "agent".into()],
            Some("agent".into()),
        );

        assert_eq!(selected, Some(ProjectKey::new("local", "agent")));
    }

    #[test]
    fn startup_falls_back_to_first_local_project_when_agent_project_is_invalid() {
        let selected = select_initial_project(
            Some(ProjectKey::new("local", "deleted")),
            "local",
            &["first".into()],
            Some("also-deleted".into()),
        );

        assert_eq!(selected, Some(ProjectKey::new("local", "first")));
    }

    #[test]
    fn startup_keeps_valid_local_and_offline_remote_currents() {
        let local = ProjectKey::new("local", "kept");
        assert_eq!(
            select_initial_project(Some(local.clone()), "local", &["kept".into()], None,),
            Some(local)
        );

        let remote = ProjectKey::new("remote-machine", "offline-project");
        assert_eq!(
            select_initial_project(Some(remote.clone()), "local", &["first".into()], None,),
            Some(remote)
        );
    }
}
