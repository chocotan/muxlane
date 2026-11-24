use super::*;
use muxlane_core::model::{AgentInstance, AgentStatus, AgentType, Project};

fn rename_fixture() -> AgentInstance {
    AgentInstance {
        id: "rename-agent".into(),
        project: "rename-project".into(),
        agent_type: AgentType::Pi,
        title: "Pi".into(),
        title_is_custom: false,
        status: AgentStatus::Working,
        status_since: 1,
        seen: true,
        tmux_session: None,
    }
}

#[test]
fn session_rename_keeps_input_focus_validates_and_cancels_with_escape() {
    with_app(|cx, window, view| {
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |app, cx| {
                let agent = rename_fixture();
                let id = agent.id.clone();
                app.last_snapshot.agents.push(agent);
                app.begin_session_rename(&id, window, cx);
            });
        })
        .unwrap();
        draw(cx, window);
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |app, cx| {
                app.sync_active_terminal_focus(window, cx);
                let rename = app.session_rename.as_ref().unwrap();
                assert!(rename.input.focus_handle(cx).is_focused(window));
                assert_eq!(rename.input.read(cx).text(), "Pi");
                assert!(app.terminal_focus_suspended(cx));
                rename
                    .input
                    .update(cx, |input, cx| input.set_text("  ", cx));
            });
        })
        .unwrap();
        cx.simulate_keystrokes(window, "enter");
        cx.update(|cx| {
            let app = view.read(cx);
            let rename = app.session_rename.as_ref().unwrap();
            assert!(rename.error.is_some());
            assert!(!rename.busy);
            assert_eq!(app.last_snapshot.agents[0].title, "Pi");
        });
        for key in ["tab", "shift-tab", "f6", "shift-f6"] {
            for _ in 0..3 {
                cx.simulate_keystrokes(window, key);
                draw(cx, window);
            }
            cx.update_window(window, |_, window, cx| {
                assert!(view
                    .read(cx)
                    .session_rename
                    .as_ref()
                    .unwrap()
                    .input
                    .focus_handle(cx)
                    .is_focused(window));
            })
            .unwrap();
        }
        cx.simulate_keystrokes(window, "escape");
        cx.update(|cx| assert!(view.read(cx).session_rename.is_none()));
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |app, cx| {
                app.begin_session_rename(&"rename-agent".into(), window, cx)
            });
        })
        .unwrap();
        draw(cx, window);
        cx.simulate_keystrokes(window, "tab");
        cx.simulate_keystrokes(window, "enter");
        cx.update(|cx| assert!(view.read(cx).session_rename.is_none()));
    });
}

#[test]
fn session_rename_saves_through_server_and_updates_display_title() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    with_app_on_runtime(&runtime, |cx, window, view| {
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |app, cx| {
                let agent = rename_fixture();
                let id = agent.id.clone();
                let mut state =
                    muxlane_server::ServerState::new(app.last_snapshot.machine.clone().unwrap());
                state.fixture_push(
                    Project {
                        id: agent.project.clone(),
                        name: "Rename".into(),
                        path: "/unused".into(),
                        branch: None,
                        agents: vec![],
                    },
                    agent,
                );
                app.last_snapshot = state.snapshot();
                app.server = MuxlaneServer::new_with_runtime(
                    app.store_path.with_extension("sock"),
                    Arc::new(tokio::sync::RwLock::new(state)),
                    muxlane_server::DirtyFlag::new(),
                    runtime.handle().clone(),
                );
                app.begin_session_rename(&id, window, cx);
                app.session_rename
                    .as_ref()
                    .unwrap()
                    .input
                    .update(cx, |input, cx| {
                        input.set_text("  修复登录问题  ", cx);
                    });
                app.submit_session_rename(window, cx);
            });
        })
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            cx.run_until_parked();
            if cx.update(|cx| view.read(cx).session_rename.is_none()) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "rename did not complete"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        cx.update(|cx| {
            let app = view.read(cx);
            assert_eq!(
                app.session_summary(&"rename-agent".into()).unwrap().0,
                "修复登录问题"
            );
            assert!(app.last_snapshot.agents[0].title_is_custom);
            let snapshot = runtime.block_on(app.server.snapshot());
            assert_eq!(snapshot.agents[0].title, "修复登录问题");
            assert_eq!(snapshot.agents[0].status, AgentStatus::Working);
        });
    });
}
