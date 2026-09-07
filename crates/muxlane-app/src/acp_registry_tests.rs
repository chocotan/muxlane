use super::*;
use gpui::{AnyWindowHandle, TestAppContext};
use muxlane_acp::{AgentDefinition, AgentRegistry, LocalProbe};

fn with_registry_app(
    registry: anyhow::Result<AgentRegistry>,
    restore: bool,
    test: impl FnOnce(&mut TestAppContext, AnyWindowHandle, Entity<MuxlaneApp>),
) {
    let directory = tempfile::tempdir().unwrap();
    // Never drive this runtime: assertions inspect launch handles without executing adapters.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let state = muxlane_server::ServerState::new(muxlane_core::model::MachineInfo {
        machine_id: "registry-test".into(),
        name: "test".into(),
        os: "test".into(),
        version: "test".into(),
    });
    let mut snapshot = state.snapshot();
    snapshot.projects.push(muxlane_core::model::Project {
        id: "project".into(),
        name: "Project".into(),
        path: directory.path().into(),
        branch: None,
        agents: vec![],
    });
    let server = MuxlaneServer::new_with_runtime(
        directory.path().join("unused.sock"),
        Arc::new(tokio::sync::RwLock::new(state)),
        muxlane_server::DirtyFlag::new(),
        runtime.handle().clone(),
    );
    let mut persisted = muxlane_store::PersistedApp::default();
    if restore {
        persisted
            .acp_threads
            .push(muxlane_store::PersistedAcpThread {
                ui_id: "restored".into(),
                project_id: "project".into(),
                profile_id: "codex".into(),
                ..Default::default()
            });
        persisted.pane_tree = PaneNode::with_tab("restored".into());
    }
    let mut cx = TestAppContext::single();
    let window = cx.add_window(|window, cx| {
        MuxlaneApp::new_with_acp_registry(
            window,
            cx,
            server,
            snapshot,
            vec![],
            persisted,
            directory.path().join("state.json"),
            registry,
        )
    });
    let app = window.root(&mut cx).unwrap();
    test(&mut cx, window.into(), app);
}

fn write_override(path: &std::path::Path) -> AgentDefinition {
    let definition = AgentDefinition {
        id: "codex".into(),
        label: "Local Codex Wrapper".into(),
        command: std::env::current_exe().unwrap().to_str().unwrap().into(),
        args: vec!["--local-wrapper".into()],
        env: Default::default(),
    };
    std::fs::write(
        path,
        serde_json::to_vec(&serde_json::json!({
            "agents": [definition.clone()]
        }))
        .unwrap(),
    )
    .unwrap();
    definition
}

fn detection(
    path: &std::path::Path,
) -> anyhow::Result<(AgentRegistry, Vec<muxlane_acp::AgentEntry>)> {
    let registry = AgentRegistry::load(path)?;
    let entries = registry.entries(None, &LocalProbe::current());
    Ok((registry, entries))
}

#[test]
fn bad_registry_blocks_restored_override_and_palette_without_fallback_launch() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("agents.json");
    let definition = write_override(&path);
    assert_eq!(
        AgentRegistry::load(&path)
            .unwrap()
            .require("codex")
            .unwrap()
            .definition(),
        definition
    );
    std::fs::write(&path, "bad JSON").unwrap();
    with_registry_app(AgentRegistry::load(&path), true, |cx, window, entity| {
        cx.update_window(window, |_, window, cx| {
            entity.update(cx, |app, cx| {
                assert_eq!(app.active.as_deref(), Some("restored"));
                assert!(app.acp_registry_error.as_ref().unwrap().contains("parse"));
                let restored = app.acp_views["restored"].clone();
                assert!(restored.read(cx).handle.is_none());
                assert!(!app.ensure_acp_started(&"restored".into(), cx));
                assert!(restored.read(cx).start_allowed);
                assert!(app
                    .acp_launch_profile("codex")
                    .unwrap_err()
                    .to_string()
                    .contains("configuration unavailable"));
                app.new_session_target = Some(NewSessionTarget::Local("project".into()));
                assert!(!app.prepare_acp_creation("codex", cx));
                app.spawn_acp_view(muxlane_acp::Profile::Codex, window, cx);
                assert_eq!(app.acp_views.len(), 1);
                assert!(app.new_session_target.is_some());
                assert!(restored.read(cx).handle.is_none());

                write_override(&path);
                app.apply_acp_detection(detection(&path), cx);
                assert!(app.acp_registry_error.is_none());
                assert_eq!(
                    app.acp_launch_profile("codex").unwrap().definition(),
                    definition
                );
                assert!(app.prepare_acp_creation("codex", cx));
                assert!(app.ensure_acp_started(&"restored".into(), cx));
                assert!(restored.read(cx).handle.is_some());
                assert_eq!(
                    restored.read(cx).profile.as_ref().unwrap().definition(),
                    definition
                );
            });
        })
        .unwrap();
    });
}

#[test]
fn failed_recheck_preserves_last_good_entries_and_running_handles_until_repaired() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("agents.json");
    let definition = write_override(&path);
    with_registry_app(AgentRegistry::load(&path), true, |cx, _, entity| {
        entity.update(cx, |app, cx| {
            let restored = app.acp_views["restored"].clone();
            assert!(restored.read(cx).handle.is_some());
            let entries = format!("{:?}", app.acp_entries);
            std::fs::write(&path, "bad JSON").unwrap();
            for _ in 0..2 {
                app.apply_acp_detection(detection(&path), cx);
                assert!(app.acp_registry_error.is_some());
                assert_eq!(
                    app.acp_registry.require("codex").unwrap().definition(),
                    definition
                );
                assert_eq!(format!("{:?}", app.acp_entries), entries);
                assert!(!app.prepare_acp_creation("codex", cx));
                assert_eq!(format!("{:?}", app.acp_entries), entries);
                assert!(app.ensure_acp_started(&"restored".into(), cx));
                assert!(restored.read(cx).handle.is_some());
                assert_eq!(
                    restored.read(cx).profile.as_ref().unwrap().definition(),
                    definition
                );
            }
            write_override(&path);
            // Repair alone does not clear the failure gate; a successful recheck does.
            assert!(!app.prepare_acp_creation("codex", cx));
            app.apply_acp_detection(detection(&path), cx);
            assert!(app.acp_registry_error.is_none());
            assert!(app.prepare_acp_creation("codex", cx));
            assert!(restored.read(cx).handle.is_some());
        });
    });
}

#[test]
fn missing_registry_file_uses_normal_defaults_without_error_gate() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("agents.json");
    with_registry_app(AgentRegistry::load(&path), false, |cx, _, entity| {
        entity.update(cx, |app, cx| {
            assert!(app.acp_registry_error.is_none());
            assert_eq!(
                app.acp_registry.profiles(),
                AgentRegistry::default().profiles()
            );
            app.apply_acp_detection(detection(&path), cx);
            assert!(app.acp_registry_error.is_none());
            assert_eq!(
                app.acp_registry.profiles(),
                AgentRegistry::default().profiles()
            );
        });
    });
}
