use super::*;
use muxlane_core::model::{AgentInstance, AgentStatus, AgentType, Project};

fn project(id: &str) -> Project {
    Project {
        id: id.into(),
        name: format!("Project {id}"),
        path: "/unused".into(),
        branch: None,
        agents: vec![],
    }
}

fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
    draw(cx, window);
    let mut visual = gpui::VisualTestContext::from_window(window, cx);
    let bounds = visual
        .debug_bounds(id)
        .unwrap_or_else(|| panic!("missing {id}"));
    visual.simulate_click(bounds.center(), Default::default());
    draw(cx, window);
}

fn remote_snapshot(status: AgentStatus, since: u64) -> muxlane_core::model::Snapshot {
    muxlane_core::model::Snapshot {
        projects: vec![project("remote-project")],
        agents: ["remote-a", "remote-b"]
            .into_iter()
            .map(|id| AgentInstance {
                id: id.into(),
                project: "remote-project".into(),
                agent_type: AgentType::Shell,
                title: id.into(),
                status,
                status_since: since,
                seen: false,
                tmux_session: None,
            })
            .collect(),
        ..Default::default()
    }
}

fn remote_event(
    cx: &mut TestAppContext,
    app: &Entity<MuxlaneApp>,
    event: muxlane_client::ClientEvent,
) {
    cx.update(|cx| app.read(cx).remote_event_tx.try_send(event).unwrap());
    cx.run_until_parked();
}

fn remote_online(snapshot: muxlane_core::model::Snapshot) -> muxlane_client::ClientEvent {
    muxlane_client::ClientEvent::StateChanged {
        host: "remote".into(),
        state: muxlane_client::RemoteState::Online(snapshot),
    }
}

#[test]
fn remote_read_tabs_stay_read_across_unrelated_snapshot_refreshes() {
    with_app(|cx, _window, app| {
        let snapshot = remote_snapshot(AgentStatus::Done, 10);
        remote_event(cx, &app, remote_online(snapshot.clone()));
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                let pane = app.active_pane.clone();
                app.activate_tab(&pane, &"remote-a".into(), cx);
                app.activate_tab(&pane, &"remote-b".into(), cx);
                assert!(app.remote_snaps["remote"].agents.iter().all(|a| a.seen));
            });
        });
        for _ in 0..3 {
            remote_event(cx, &app, remote_online(snapshot.clone()));
            cx.update(|cx| {
                assert!(app.read(cx).remote_snaps["remote"]
                    .agents
                    .iter()
                    .all(|a| a.seen));
            });
        }
        // A new result missed during a disconnect must not inherit the old read flag.
        let mut next = snapshot;
        next.agents[0].status_since = 20;
        remote_event(cx, &app, remote_online(next));
        cx.update(|cx| {
            let app = app.read(cx);
            assert!(!app.remote_snaps["remote"].agents[0].seen);
            assert!(app.remote_snaps["remote"].agents[1].seen);
        });
    });
}

#[test]
fn remote_status_events_only_alert_background_tabs_and_preserve_reads_before_snapshot() {
    with_app(|cx, _window, app| {
        remote_event(
            cx,
            &app,
            remote_online(remote_snapshot(AgentStatus::Working, 10)),
        );
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                let pane = app.active_pane.clone();
                app.activate_tab(&pane, &"remote-a".into(), cx);
            });
        });
        for agent in ["remote-a", "remote-b"] {
            remote_event(
                cx,
                &app,
                muxlane_client::ClientEvent::StatusChanged {
                    host: "remote".into(),
                    agent: agent.into(),
                    agent_type: None,
                    from: AgentStatus::Working,
                    to: AgentStatus::Failed,
                    message: None,
                },
            );
        }
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                assert!(app.remote_snaps["remote"].agents[0].seen);
                assert!(!app.remote_snaps["remote"].agents[1].seen);
                let pane = app.active_pane.clone();
                app.activate_tab(&pane, &"remote-b".into(), cx);
            });
        });
        remote_event(
            cx,
            &app,
            remote_online(remote_snapshot(AgentStatus::Failed, 20)),
        );
        cx.update(|cx| {
            assert!(app.read(cx).remote_snaps["remote"]
                .agents
                .iter()
                .all(|a| a.seen));
        });
        // Even within the same timestamp second, a new completion event is unread.
        remote_event(
            cx,
            &app,
            muxlane_client::ClientEvent::StatusChanged {
                host: "remote".into(),
                agent: "remote-a".into(),
                agent_type: None,
                from: AgentStatus::Working,
                to: AgentStatus::Failed,
                message: None,
            },
        );
        remote_event(
            cx,
            &app,
            remote_online(remote_snapshot(AgentStatus::Failed, 20)),
        );
        cx.update(|cx| {
            assert!(!app.read(cx).remote_snaps["remote"].agents[0].seen);
        });
    });
}

#[test]
fn opening_remote_results_then_adding_shell_does_not_restore_attention() {
    use muxlane_core::model::MachineInfo;

    for detach_first in [false, true] {
        for status in [AgentStatus::Done, AgentStatus::Failed] {
            with_app(|cx, window, app| {
                let mode = if detach_first { "detached" } else { "docked" };
                let mut snapshot = remote_snapshot(status, 10);
                snapshot.machine = Some(MachineInfo {
                    machine_id: "remote-machine".into(),
                    name: "remote".into(),
                    os: "test".into(),
                    version: "test".into(),
                });
                remote_event(cx, &app, remote_online(snapshot.clone()));
                cx.update_window(window, |_, window, cx| {
                    app.update(cx, |app, cx| {
                        // Real activation must reach focus_agent, not just activate_tab.
                        for id in ["remote-a", "remote-b", "new-shell"] {
                            let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
                            let term = MuxlaneApp::create_remote_term(
                                id.into(),
                                muxlane_term::VTerm::new_with_clipboard(80, 24),
                                sender,
                                &app.font_family,
                                Theme::for_mode(app.theme_mode),
                                false,
                                cx,
                            );
                            app.terms.insert(id.into(), term);
                        }
                        app.select_project_workspace_inner(
                            ProjectKey::new("remote-machine", "remote-project"),
                            cx,
                        );
                        for id in ["remote-a", "remote-b"] {
                            app.open_agent(&id.into(), window, cx);
                        }
                        if detach_first {
                            app.detach_all_sessions(cx);
                        }
                        assert!(app.remote_snaps["remote"].agents.iter().all(|a| a.seen));
                    });
                })
                .unwrap();

                let shell = AgentInstance {
                    id: "new-shell".into(),
                    project: "remote-project".into(),
                    agent_type: AgentType::Shell,
                    title: "Shell".into(),
                    status: AgentStatus::Idle,
                    status_since: 20,
                    seen: true,
                    tmux_session: None,
                };
                // The spawn response opens the new shell before state.changed refreshes everything.
                cx.update_window(window, |_, window, cx| {
                    app.update(cx, |app, cx| {
                        app.remote_snaps
                            .get_mut("remote")
                            .unwrap()
                            .agents
                            .push(shell.clone());
                        app.open_agent(&shell.id, window, cx);
                    });
                })
                .unwrap();
                snapshot.agents.push(shell);
                for _ in 0..2 {
                    remote_event(cx, &app, remote_online(snapshot.clone()));
                    cx.update(|cx| {
                        let app = app.read(cx);
                        assert_eq!(app.active.as_deref(), Some("new-shell"));
                        for id in ["remote-a", "remote-b"] {
                            let agent = app.remote_snaps["remote"].agent(&id.into()).unwrap();
                            assert!(
                                agent.seen,
                                "{mode:?}: {id} became unread after adding a shell"
                            );
                            assert!(
                                !crate::widgets::compute_attention_style(
                                    agent.status,
                                    agent.seen,
                                    Theme::for_mode(app.theme_mode),
                                )
                                .is_alerting
                            );
                            // Read agents render as Idle (gray dot) without mutating the true status.
                            assert_eq!(
                                crate::widgets::display_status(agent.status, agent.seen),
                                AgentStatus::Idle,
                                "{mode:?}: {id} indicator did not turn gray after being read",
                            );
                            assert_eq!(
                                agent.status, status,
                                "{mode:?}: {id} true status was mutated locally"
                            );
                        }
                    });
                }
                // Genuine new results must still alert after an earlier result was read.
                snapshot.agents[0].status_since = 30;
                remote_event(cx, &app, remote_online(snapshot));
                cx.update(|cx| {
                    assert!(!app.read(cx).remote_snaps["remote"].agents[0].seen);
                });
            });
        }
    }
}

#[test]
fn remote_seen_does_not_leak_to_replaced_machine() {
    use muxlane_core::model::MachineInfo;
    let mut previous = remote_snapshot(AgentStatus::Done, 10);
    previous.machine = Some(MachineInfo {
        machine_id: "old".into(),
        name: "remote".into(),
        os: "test".into(),
        version: "test".into(),
    });
    previous.agents[0].seen = true;
    let mut next = previous.clone();
    next.machine.as_mut().unwrap().machine_id = "new".into();
    next.agents[0].seen = false;
    crate::remotes::merge_remote_seen(&mut next, Some(&previous), None, None);
    assert!(!next.agents[0].seen);
}

#[test]
fn terminal_notification_click_keeps_terminal_routing() {
    with_app(|cx, window, app| {
        let terminal = cx.update(|cx| {
            app.update(cx, |app, cx| {
                app.last_snapshot.projects = vec![project("a"), project("b")];
                app.last_snapshot.agents.push(AgentInstance {
                    id: "terminal-notification".into(),
                    project: "b".into(),
                    agent_type: AgentType::Shell,
                    title: "Terminal".into(),
                    status: AgentStatus::Done,
                    status_since: 0,
                    seen: false,
                    tmux_session: None,
                });
                let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
                let terminal = MuxlaneApp::create_remote_term(
                    "terminal-notification".into(),
                    muxlane_term::VTerm::new_with_clipboard(80, 24),
                    sender,
                    &app.font_family,
                    Theme::for_mode(app.theme_mode),
                    false,
                    cx,
                );
                app.terms
                    .insert("terminal-notification".into(), terminal.clone());
                app.select_project_workspace_inner(
                    ProjectKey::new(app.local_machine_id(), "a"),
                    cx,
                );
                let mut draft = app.notification_draft(
                    "terminal-notification".into(),
                    AgentStatus::Working,
                    AgentStatus::Done,
                    Some("Terminal completed".into()),
                );
                assert_eq!(draft.project_name, "Project b");
                assert_eq!(draft.agent_type, AgentType::Shell);
                assert!(draft.desktop_enabled);
                draft.desktop_enabled = false;
                draft.sound_enabled = false;
                app.notifications
                    .update(cx, |center, cx| center.push_notification(draft, cx));
                terminal
            })
        });
        click(cx, window, "toast-1");
        cx.update_window(window, |_, window, cx| {
            let app = app.read(cx);
            assert_eq!(app.active.as_deref(), Some("terminal-notification"));
            assert_eq!(
                app.workspace.current_project(),
                Some(&ProjectKey::new(app.local_machine_id(), "b"))
            );
            assert_eq!(
                app.pane_tree
                    .group(&app.active_pane)
                    .unwrap()
                    .active
                    .as_deref(),
                Some("terminal-notification")
            );
            assert!(terminal.focus_handle(cx).is_focused(window));
            assert_eq!(app.terms.len(), 1);
        })
        .unwrap();
    });
}
