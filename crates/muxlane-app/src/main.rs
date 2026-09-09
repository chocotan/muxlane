//! muxlane GPUI 主程序：三区极简 UI（侧栏机器树 / 贴边终端网格 / 浮层）
mod actions;
mod app;
mod bootstrap;
mod dialogs;
mod editors;
mod floating;
mod i18n;
mod icons;
mod menus;
mod notifications;
mod persistence;
mod remotes;
mod sessions;
mod settings;
#[cfg(any(target_os = "macos", test))]
mod shell_environment;
mod shortcuts;
mod sidebar_state;
mod sound;
mod term_view;
mod terminal_keys;
mod text_field;
mod theme;
mod ui_scale;
mod widgets;
mod workspace;

use muxlane_core::model::MachineInfo;
use muxlane_server::{DirtyFlag, MuxlaneServer, ServerState};
use std::sync::Arc;
use tokio::sync::RwLock;

fn hostname() -> String {
    if let Ok(name) = std::env::var("HOSTNAME") {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "local".into())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("muxlane {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "muxlane {version}\n\nUSAGE:\n  muxlane [--headless] [--connect TARGET[,TARGET...]]\n\nOPTIONS:\n  --headless          Run Unix socket server without a window\n  --connect TARGET    Connect to /path/muxlane.sock or user@host:/path/muxlane.sock\n  -V, --version       Print version\n  -h, --help          Print help\n\nENV:\n  MUXLANE_SHELL=/path    Override default shell\n  MUXLANE_HOOKS=off      Disable agent hook injection",
            version = env!("CARGO_PKG_VERSION")
        );
        return;
    }
    let headless = args.iter().any(|a| a == "--headless");
    let mut connect_to: Vec<String> = args
        .iter()
        .position(|a| a == "--connect")
        .and_then(|i| args.get(i + 1).cloned())
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    let dir = muxlane_core::paths::data_dir();
    std::fs::create_dir_all(&dir).ok();
    let store_path = muxlane_store::default_path(&dir);
    let mut persisted = match muxlane_store::load(&store_path) {
        Ok(state) => state,
        Err(error) => {
            let message = i18n::text(i18n::Language::detect(), "main.state_load_failed")
                .replace("{path}", &store_path.display().to_string())
                .replace("{error}", &error.to_string());
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    for remote in &persisted.remotes {
        if !connect_to.contains(remote) {
            connect_to.push(remote.clone());
        }
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .try_init()
        .ok();

    #[cfg(target_os = "macos")]
    if let Err(error) = shell_environment::import_login_path(&muxlane_term::default_shell_program())
    {
        tracing::warn!(%error, "could not prepare macOS terminal PATH");
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let machine_id = std::fs::read_to_string(dir.join("machine_id"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| {
            let id = muxlane_core::model::new_id("machine");
            std::fs::write(dir.join("machine_id"), &id).ok();
            id
        });

    let mut initial_state = ServerState::new(MachineInfo {
        machine_id,
        name: hostname(),
        os: std::env::consts::OS.into(),
        version: env!("CARGO_PKG_VERSION").into(),
    });
    initial_state.projects = persisted.projects.clone();
    for project in &mut initial_state.projects {
        project.agents.clear();
    }
    let state = Arc::new(RwLock::new(initial_state));
    let dirty = DirtyFlag::new();
    let auth = muxlane_core::AuthSecret::load_or_create(&dir.join("secret"))
        .expect("load/create muxlane auth secret");
    let server = MuxlaneServer::new_with_runtime_and_auth(
        dir.join("muxlane.sock"),
        Arc::clone(&state),
        dirty,
        rt.handle().clone(),
        auth,
    );

    {
        let srv = Arc::clone(&server);
        rt.spawn(async move { srv.serve().await.expect("server died") });
    }
    server.start_supervisor();

    bootstrap::install(&dir);

    if headless {
        rt.block_on(server.restore_sessions(&persisted));
        // Headless state changes persist synchronously through MuxlaneServer.
        server.set_persistence_path(store_path);
        tracing::info!("muxlane headless server running");
        rt.block_on(std::future::pending::<()>());
        return;
    }

    // GUI: do not attach tmux sessions before the window is up. Restoring forks one
    // `tmux` per saved session and races the X11 input-method handshake that happens
    // right after the window opens; losing that race disables the IME for the whole
    // process. The app lays out the saved sessions provisionally and triggers the real
    // restore itself after its first frame.
    let base_snapshot = rt.block_on(server.snapshot());
    let initial_snapshot = persisted.provisional_snapshot(base_snapshot);
    persisted = muxlane_store::PersistedApp::from_snapshot(&initial_snapshot)
        .with_ui_prefs_from(&persisted);

    app::launch(server, initial_snapshot, connect_to, persisted, store_path);
}
