//! muxlane GPUI 主程序：三区极简 UI（侧栏机器树 / 贴边终端网格 / 浮层）
mod acp_checkpoint;
mod acp_composer;
mod acp_elicitation;
mod acp_markdown;
mod acp_view;
mod actions;
mod app;
mod bootstrap;
mod dialogs;
mod i18n;
mod icons;
mod menus;
mod notifications;
mod persistence;
mod prompt_editor;
mod remotes;
mod selector_menu;
mod sessions;
mod settings;
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
    rt.block_on(server.restore_sessions(&persisted));
    let initial_snapshot = rt.block_on(server.snapshot());
    persisted = muxlane_store::PersistedApp::from_snapshot(&initial_snapshot)
        .with_ui_prefs_from(&persisted);

    if headless {
        // Headless state changes persist synchronously through MuxlaneServer.
        server.set_persistence_path(store_path);
        tracing::info!("muxlane headless server running");
        rt.block_on(std::future::pending::<()>());
        return;
    }

    app::launch(server, initial_snapshot, connect_to, persisted, store_path);
}
