use super::*;
use muxlane_acp::{Event, PromptId, TurnState};
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

fn apply(cx: &mut TestAppContext, acp: &Entity<AcpView>, event: Event) {
    cx.update(|cx| acp.update(cx, |view, cx| view.apply(event, cx)));
    cx.run_until_parked();
}

fn prompt_id(id: &str) -> PromptId {
    serde_json::from_value(serde_json::json!(id)).unwrap()
}

fn complete(cx: &mut TestAppContext, acp: &Entity<AcpView>, id: &str) {
    let id = prompt_id(id);
    apply(cx, acp, Event::PromptAccepted { id: id.clone() });
    apply(cx, acp, Event::Turn(TurnState::Generating));
    apply(cx, acp, Event::PromptCompleted { id: id.clone() });
    apply(cx, acp, Event::Turn(TurnState::Idle));
    // Duplicate and stale completion events cannot create a second notification.
    apply(cx, acp, Event::PromptCompleted { id });
}

fn count(cx: &mut TestAppContext, app: &Entity<MuxlaneApp>) -> usize {
    cx.update(|cx| app.read(cx).notifications.read(cx).entries().len())
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

#[test]
fn acp_notification_click_targets_ui_session_across_projects_without_terminal_fallback() {
    for profile in [None, Some(muxlane_acp::Profile::Pi)] {
        with_app(|cx, window, app| {
            let (acp, target_pane) = cx
                .update_window(window, |_, window, cx| {
                    app.update(cx, |app, cx| {
                        app.last_snapshot.projects = vec![project("a"), project("b")];
                        let acp = cx.new(|cx| {
                            let mut view = AcpView::new(
                                AcpViewInit {
                                    profile_id: "unresolved-notification-profile".into(),
                                    ui_id: "acp-notification-ui".into(),
                                    project_id: "b".into(),
                                    project_path: "/unused".into(),
                                    profile: profile.clone(),
                                    protocol_session_id: Some("unrelated-protocol-id".into()),
                                    parent_ui_id: None,
                                    title: "ACP notification title".into(),
                                    draft: String::new(),
                                    snapshot: Default::default(),
                                    queued_prompts: vec![],
                                    queue_paused: false,
                                    theme_mode: app.theme_mode,
                                    language: app.language,
                                },
                                window,
                                cx,
                            );
                            view.start_allowed = false;
                            view
                        });
                        app.register_acp_view(
                            "acp-notification-ui".into(),
                            acp.clone(),
                            window,
                            cx,
                        );
                        app.select_project_workspace_inner(
                            ProjectKey::new(app.local_machine_id(), "b"),
                            cx,
                        );
                        let target_pane = app
                            .pane_tree
                            .split(
                                &app.active_pane,
                                muxlane_core::SplitAxis::Horizontal,
                                "acp-notification-ui".into(),
                            )
                            .unwrap();
                        app.select_project_workspace_inner(
                            ProjectKey::new(app.local_machine_id(), "a"),
                            cx,
                        );
                        (acp, target_pane)
                    })
                })
                .unwrap();
            apply(cx, &acp, Event::Turn(TurnState::Idle));
            apply(
                cx,
                &acp,
                Event::PromptCompleted {
                    id: prompt_id("not-accepted"),
                },
            );
            assert_eq!(count(cx, &app), 0);
            complete(cx, &acp, "first");
            assert_eq!(count(cx, &app), 1);
            cx.update(|cx| {
                let app = app.read(cx);
                let entries = app.notifications.read(cx).entries();
                assert_eq!(entries[0].agent, "acp-notification-ui");
                assert_eq!(entries[0].project_name, "Project b");
                let message = entries[0].message.as_ref().unwrap();
                assert!(message.contains("ACP notification title"));
                assert!(message.contains("unresolved-notification-profile"));
                assert!(entries[0].unread);
                let draft = app.notification_draft(
                    "acp-notification-ui".into(),
                    AgentStatus::Working,
                    AgentStatus::Done,
                    None,
                );
                assert!(!draft.desktop_enabled && !draft.sound_enabled);
            });
            click(cx, window, "toast-1");
            cx.update_window(window, |_, window, cx| {
                let app = app.read(cx);
                assert_eq!(
                    app.workspace.current_project(),
                    Some(&ProjectKey::new(app.local_machine_id(), "b"))
                );
                assert_eq!(app.active.as_deref(), Some("acp-notification-ui"));
                assert_eq!(app.active_pane, target_pane);
                assert_eq!(
                    app.pane_tree
                        .group(&app.active_pane)
                        .unwrap()
                        .active
                        .as_deref(),
                    Some("acp-notification-ui")
                );
                assert!(acp.focus_handle(cx).is_focused(window));
                assert!(app.terms.is_empty());
                assert!(!acp.read(cx).start_allowed);
                assert!(acp.read(cx).handle.is_none());
                assert!(!app.notifications.read(cx).entries()[0].unread);
            })
            .unwrap();

            // Failed and cancelled turns do not manufacture completion from Idle.
            apply(
                cx,
                &acp,
                Event::PromptAccepted {
                    id: prompt_id("failed"),
                },
            );
            apply(cx, &acp, Event::Turn(TurnState::Generating));
            apply(
                cx,
                &acp,
                Event::Error(muxlane_acp::SessionError {
                    kind: muxlane_acp::ErrorKind::Request,
                    message: "fixture failure".into(),
                }),
            );
            apply(cx, &acp, Event::Turn(TurnState::Idle));
            apply(
                cx,
                &acp,
                Event::PromptCompleted {
                    id: prompt_id("failed"),
                },
            );
            apply(
                cx,
                &acp,
                Event::PromptAccepted {
                    id: prompt_id("cancelled"),
                },
            );
            apply(cx, &acp, Event::Turn(TurnState::Generating));
            cx.update(|cx| acp.update(cx, |view, cx| view.cancel(cx)));
            apply(
                cx,
                &acp,
                Event::PromptCompleted {
                    id: prompt_id("cancelled"),
                },
            );
            apply(cx, &acp, Event::Turn(TurnState::Idle));
            assert_eq!(count(cx, &app), 1);

            complete(cx, &acp, "second");
            assert_eq!(count(cx, &app), 2);
            cx.update(|cx| {
                app.update(cx, |app, cx| {
                    app.select_project_workspace_inner(
                        ProjectKey::new(app.local_machine_id(), "a"),
                        cx,
                    );
                    app.notifications
                        .update(cx, |center, cx| center.toggle_open(cx));
                });
            });
            click(cx, window, "notif-popover-item-0");
            assert!(cx
                .update_window(window, |_, window, cx| acp
                    .focus_handle(cx)
                    .is_focused(window))
                .unwrap());
            assert!(cx.update(|cx| app.read(cx).terms.is_empty()));

            // Simulate an old notification whose target was removed, without purging its row.
            cx.update(|cx| {
                app.update(cx, |app, cx| {
                    app.acp_views.remove("acp-notification-ui");
                    app.acp_metadata.remove("acp-notification-ui");
                    app.acp_records.remove("acp-notification-ui");
                    app.acp_deleted
                        .lock()
                        .unwrap()
                        .insert("acp-notification-ui".into());
                    app.select_project_workspace_inner(
                        ProjectKey::new(app.local_machine_id(), "a"),
                        cx,
                    );
                    app.notifications
                        .update(cx, |center, cx| center.toggle_open(cx));
                });
            });
            let before = cx.update(|cx| {
                let app = app.read(cx);
                (
                    app.active.clone(),
                    app.active_pane.clone(),
                    app.workspace.current_project().cloned(),
                )
            });
            click(cx, window, "notif-popover-item-0");
            cx.update(|cx| {
                let app = app.read(cx);
                assert_eq!(
                    (
                        app.active.clone(),
                        app.active_pane.clone(),
                        app.workspace.current_project().cloned()
                    ),
                    before
                );
                assert!(app.terms.is_empty());
            });
            // A retained old view cannot publish a notification after deletion.
            complete(cx, &acp, "deleted");
            assert_eq!(count(cx, &app), 2);
        });
    }
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
            assert!(app.acp_views.is_empty());
        })
        .unwrap();
    });
}
