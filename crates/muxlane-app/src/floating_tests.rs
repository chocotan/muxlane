//! Per-session detach/reattach: real in-process GPUI windows, inert terminals, no PTY/SSH.
use super::*;
use muxlane_core::model::{AgentInstance, AgentStatus, AgentType, Project};

fn populate(app: &mut MuxlaneApp, window: &mut Window, cx: &mut Context<MuxlaneApp>) {
    app.last_snapshot.projects = ["a", "b"]
        .into_iter()
        .map(|id| Project {
            id: id.into(),
            name: id.into(),
            path: "/unused".into(),
            branch: None,
            agents: vec![],
        })
        .collect();
    for (id, project) in [("a1", "a"), ("a2", "a"), ("b1", "b")] {
        app.last_snapshot.agents.push(AgentInstance {
            id: id.into(),
            project: project.into(),
            title: id.into(),
            agent_type: AgentType::Shell,
            status: AgentStatus::Idle,
            status_since: 0,
            seen: true,
            tmux_session: None,
        });
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
    app.select_project_workspace_inner(ProjectKey::new(app.local_machine_id(), "a"), cx);
    for id in ["a1", "a2"] {
        app.open_agent(&id.into(), window, cx);
    }
}

fn setup(cx: &mut TestAppContext, window: AnyWindowHandle, view: &Entity<MuxlaneApp>) {
    let visual = gpui::VisualTestContext::from_window(window, cx);
    visual.simulate_resize(size(px(1200.), px(800.)));
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |app, cx| populate(app, window, cx))
    })
    .unwrap();
    draw(cx, window);
}

fn main_size(cx: &mut TestAppContext, window: AnyWindowHandle) -> gpui::Size<gpui::Pixels> {
    cx.update_window(window, |_, window, _| window.viewport_size())
        .unwrap()
}

#[test]
fn detach_moves_session_to_native_window_and_keeps_main_window_size() {
    with_app(|cx, window, view| {
        setup(cx, window, &view);
        let before = main_size(cx, window);
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                assert!(!app.is_detached(&"a1".into()));
                assert!(app.pane_tree.pane_for_agent(&"a1".into()).is_some());
                app.detach_session(&"a1".into(), cx);
            })
        });
        draw(cx, window);
        cx.update(|cx| {
            let app = view.read(cx);
            assert!(app.is_detached(&"a1".into()));
            assert!(
                app.floating.windows.contains_key("a1"),
                "native window opened"
            );
            assert!(
                app.pane_tree.pane_for_agent(&"a1".into()).is_none(),
                "left the pane tree"
            );
            assert!(
                app.pane_tree.pane_for_agent(&"a2".into()).is_some(),
                "a2 still docked"
            );
            assert_eq!(app.active.as_deref(), Some("a2"));
            assert!(app.terms.contains_key("a1"), "terminal entity preserved");
        });
        // The app must never resize the main window on detach.
        assert_eq!(main_size(cx, window), before);
        // Sidebar marker is rendered for the detached session only.
        let mut visual = gpui::VisualTestContext::from_window(window, cx);
        assert!(visual.debug_bounds("detached-a1").is_some());
        assert!(visual.debug_bounds("detached-a2").is_none());
    });
}

#[test]
fn reattach_closes_native_window_and_restores_tab() {
    with_app(|cx, window, view| {
        setup(cx, window, &view);
        cx.update(|cx| view.update(cx, |app, cx| app.detach_session(&"a1".into(), cx)));
        draw(cx, window);
        let handle = cx.update(|cx| view.read(cx).floating.windows["a1"]);
        cx.update(|cx| view.update(cx, |app, cx| app.reattach_session(&"a1".into(), cx)));
        draw(cx, window);
        cx.update(|cx| {
            let app = view.read(cx);
            assert!(!app.is_detached(&"a1".into()));
            assert!(!app.floating.windows.contains_key("a1"));
            assert!(app.pane_tree.pane_for_agent(&"a1".into()).is_some());
            assert_eq!(app.active.as_deref(), Some("a1"));
            assert!(
                handle.update(cx, |_, _, _| ()).is_err(),
                "native window closed"
            );
        });
    });
}

#[test]
fn native_close_button_reattaches_and_remembers_bounds() {
    with_app(|cx, window, view| {
        setup(cx, window, &view);
        cx.update(|cx| view.update(cx, |app, cx| app.detach_session(&"a1".into(), cx)));
        draw(cx, window);
        let handle = cx.update(|cx| view.read(cx).floating.windows["a1"]);
        // Simulate the user pressing the OS close button.
        cx.update_window(handle.into(), |_, window, cx| {
            let bounds = window.bounds();
            let id = window.window_handle().window_id();
            view.update(cx, |app, cx| {
                app.record_window_bounds(&"a1".into(), bounds);
                app.reattach_from_window(&"a1".into(), id, cx);
            });
            window.remove_window();
        })
        .unwrap();
        draw(cx, window);
        cx.update(|cx| {
            let app = view.read(cx);
            assert!(!app.is_detached(&"a1".into()));
            assert!(app.pane_tree.pane_for_agent(&"a1".into()).is_some());
            // Geometry survives for the next detach.
            let rect = app
                .floating
                .layouts
                .values()
                .flat_map(|l| l.windows.iter())
                .find(|w| w.agent == "a1")
                .map(|w| w.normal)
                .expect("remembered rect");
            assert!(rect.width >= 280. && rect.height >= 160.);
        });
        // Stale window id must not reattach a *newly* detached window.
        cx.update(|cx| view.update(cx, |app, cx| app.detach_session(&"a1".into(), cx)));
        draw(cx, window);
        let fresh = cx.update(|cx| view.read(cx).floating.windows["a1"]);
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                app.reattach_from_window(&"a1".into(), handle.window_id(), cx);
            })
        });
        cx.update(|cx| {
            let app = view.read(cx);
            assert!(app.is_detached(&"a1".into()), "stale close ignored");
            assert_eq!(app.floating.windows["a1"].window_id(), fresh.window_id());
        });
    });
}

#[test]
fn detach_all_and_reattach_all_cover_every_project() {
    with_app(|cx, window, view| {
        setup(cx, window, &view);
        cx.update(|cx| view.update(cx, |app, cx| app.detach_all_sessions(cx)));
        draw(cx, window);
        cx.update(|cx| {
            let app = view.read(cx);
            assert!(app.is_detached(&"a1".into()));
            assert!(app.is_detached(&"a2".into()));
            assert!(app.is_detached(&"b1".into()), "other projects are included");
            assert!(app.pane_tree.has_no_tabs());
            assert_eq!(app.floating.windows.len(), 3);
        });
        // Content area shows the placeholder, sidebar shows Reattach All.
        let mut visual = gpui::VisualTestContext::from_window(window, cx);
        assert!(visual.debug_bounds("detached-placeholder").is_some());
        assert!(visual.debug_bounds("sidebar-reattach-all").is_some());
        assert!(visual.debug_bounds("sidebar-detach-all").is_none());

        cx.update(|cx| view.update(cx, |app, cx| app.reattach_all_sessions(cx)));
        draw(cx, window);
        cx.update(|cx| {
            let app = view.read(cx);
            for id in ["a1", "a2", "b1"] {
                assert!(!app.is_detached(&id.into()));
                assert!(
                    app.pane_tree.pane_for_agent(&id.into()).is_some(),
                    "{id} back as tab"
                );
            }
            assert!(app.floating.windows.is_empty());
        });
        let mut visual = gpui::VisualTestContext::from_window(window, cx);
        assert!(visual.debug_bounds("detached-placeholder").is_none());
        assert!(visual.debug_bounds("sidebar-detach-all").is_some());
    });
}

#[test]
fn switching_projects_never_restores_a_detached_session_as_a_tab() {
    with_app(|cx, window, view| {
        setup(cx, window, &view);
        // Per-project workspaces so project "a" has its own saved layout containing a1.
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |app, cx| {
                app.set_project_workspaces_enabled(true, window, cx)
            })
        })
        .unwrap();
        draw(cx, window);
        cx.update(|cx| view.update(cx, |app, cx| app.detach_session(&"a1".into(), cx)));
        draw(cx, window);
        let before = cx.update(|cx| view.read(cx).terms["a1"].read(cx).render_count());
        // Leave and come back: the saved layout for "a" used to contain a1.
        let local = cx.update(|cx| view.read(cx).local_machine_id());
        for project in ["b", "a", "b", "a"] {
            cx.update(|cx| {
                view.update(cx, |app, cx| {
                    app.select_project_workspace_inner(ProjectKey::new(local.clone(), project), cx)
                })
            });
            draw(cx, window);
            cx.update(|cx| {
                let app = view.read(cx);
                assert!(app.is_detached(&"a1".into()));
                assert!(
                    app.pane_tree.pane_for_agent(&"a1".into()).is_none(),
                    "a1 must not reappear as a tab in project {project}"
                );
            });
        }
        // The detached terminal only ever renders in its own window: bound_window must not
        // flip back to the main window, which is what made the two copies flicker.
        let handle = cx.update(|cx| view.read(cx).floating.windows["a1"]);
        cx.update(|cx| {
            let app = view.read(cx);
            assert_eq!(
                app.terms["a1"].read(cx).bound_window,
                Some(handle.window_id())
            );
        });
        let after = cx.update(|cx| view.read(cx).terms["a1"].read(cx).render_count());
        assert!(
            after - before <= 2,
            "a1 re-rendered {} times while switching projects",
            after - before
        );
    });
}

#[test]
fn sidebar_click_on_detached_session_focuses_native_window_not_main() {
    with_app(|cx, window, view| {
        setup(cx, window, &view);
        cx.update(|cx| view.update(cx, |app, cx| app.detach_session(&"a1".into(), cx)));
        draw(cx, window);
        // Activate a2 in the main window first.
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |app, cx| app.open_agent(&"a2".into(), window, cx))
        })
        .unwrap();
        draw(cx, window);
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |app, cx| app.open_agent(&"a1".into(), window, cx))
        })
        .unwrap();
        draw(cx, window);
        cx.update(|cx| {
            let app = view.read(cx);
            // a1 stays detached; it did not get re-docked by a sidebar click.
            assert!(app.is_detached(&"a1".into()));
            assert!(app.pane_tree.pane_for_agent(&"a1".into()).is_none());
            // The main window's own active tab is still a2.
            assert_eq!(
                app.pane_tree
                    .group(&app.active_pane)
                    .and_then(|g| g.active.clone())
                    .as_deref(),
                Some("a2")
            );
        });
    });
}

#[test]
fn deleting_a_detached_session_closes_its_window() {
    with_app(|cx, window, view| {
        setup(cx, window, &view);
        cx.update(|cx| view.update(cx, |app, cx| app.detach_session(&"a1".into(), cx)));
        draw(cx, window);
        let handle = cx.update(|cx| view.read(cx).floating.windows["a1"]);
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |app, cx| {
                app.finish_delete_session(&"a1".into(), window, cx)
            })
        })
        .unwrap();
        draw(cx, window);
        cx.update(|cx| {
            let app = view.read(cx);
            assert!(!app.floating.windows.contains_key("a1"));
            assert!(!app.terms.contains_key("a1"));
            assert!(app
                .floating
                .known_agents(&app.local_machine_id())
                .is_empty());
            assert!(handle.update(cx, |_, _, _| ()).is_err());
        });
    });
}

#[test]
fn open_failure_falls_back_to_docked_without_losing_terminal() {
    with_app(|cx, window, view| {
        setup(cx, window, &view);
        let entity = cx.update(|cx| view.read(cx).terms["a1"].entity_id());
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                app.floating.fail_next_open = true;
                app.detach_session(&"a1".into(), cx);
            })
        });
        draw(cx, window);
        cx.update(|cx| {
            let app = view.read(cx);
            assert!(!app.is_detached(&"a1".into()), "fell back to docked");
            assert!(app.pane_tree.pane_for_agent(&"a1".into()).is_some());
            assert!(app.floating.windows.is_empty());
            assert_eq!(app.terms["a1"].entity_id(), entity);
            assert!(app.notifications.read(cx).has_activity(), "user was told");
        });
    });
}

#[test]
fn detach_state_round_trips_through_persistence() {
    with_app(|cx, window, view| {
        setup(cx, window, &view);
        cx.update(|cx| view.update(cx, |app, cx| app.detach_session(&"a1".into(), cx)));
        draw(cx, window);
        let persisted = cx.update(|cx| {
            let mut app = muxlane_store::PersistedApp::default();
            view.read(cx).floating.write_persisted(&mut app);
            app
        });
        let restored = crate::floating::FloatingState::from_persisted(&persisted);
        assert!(restored.is_detached(&"a1".into()));
        assert!(!restored.is_detached(&"a2".into()));
        // Reattach → hidden=true still persisted (geometry kept), but no longer detached.
        cx.update(|cx| view.update(cx, |app, cx| app.reattach_session(&"a1".into(), cx)));
        let persisted = cx.update(|cx| {
            let mut app = muxlane_store::PersistedApp::default();
            view.read(cx).floating.write_persisted(&mut app);
            app
        });
        let restored = crate::floating::FloatingState::from_persisted(&persisted);
        assert!(!restored.is_detached(&"a1".into()));
        assert!(restored.known_agents("ux-test").contains("a1"));
    });
}

#[test]
fn quit_closes_every_native_window() {
    with_app(|cx, window, view| {
        setup(cx, window, &view);
        cx.update(|cx| view.update(cx, |app, cx| app.detach_all_sessions(cx)));
        draw(cx, window);
        let handles: Vec<_> =
            cx.update(|cx| view.read(cx).floating.windows.values().copied().collect());
        assert_eq!(handles.len(), 3);
        cx.update(|cx| view.update(cx, |app, cx| app.close_all_session_windows(cx)));
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(view.read(cx).floating.windows.is_empty());
            for handle in handles {
                assert!(handle.update(cx, |_, _, _| ()).is_err());
            }
        });
    });
}

#[test]
fn detached_windows_settle_and_do_not_redraw_every_frame() {
    with_app(|cx, window, view| {
        setup(cx, window, &view);
        cx.update(|cx| view.update(cx, |app, cx| app.detach_session(&"a1".into(), cx)));
        draw(cx, window);
        let handle = cx.update(|cx| view.read(cx).floating.windows["a1"]);
        for _ in 0..3 {
            draw(cx, window);
            draw(cx, handle.into());
        }
        let before = cx.update(|cx| {
            let app = view.read(cx);
            (
                app.terms["a1"].read(cx).render_count(),
                app.terms["a2"].read(cx).render_count(),
            )
        });
        // With no input and no state change, nothing should re-render.
        for _ in 0..5 {
            cx.run_until_parked();
        }
        let after = cx.update(|cx| {
            let app = view.read(cx);
            (
                app.terms["a1"].read(cx).render_count(),
                app.terms["a2"].read(cx).render_count(),
            )
        });
        assert_eq!(
            before, after,
            "terminals re-rendered with no input: {before:?} -> {after:?}"
        );
    });
}
