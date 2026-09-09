//! In-process GPUI tests. The server has no agents and is never served; no PTY or SSH is started.

#[path = "project_creation_tests.rs"]
mod project_creation_tests;

#[path = "floating_tests.rs"]
mod floating_tests;

#[path = "notification_tests.rs"]
mod notification_tests;
#[path = "remote_operation_tests.rs"]
mod remote_operation_tests;

mod sidebar_navigation {
    use super::*;
    use muxlane_core::model::{AgentInstance, AgentType, MachineInfo, Project, Snapshot};

    fn project(id: &str) -> Project {
        Project {
            id: id.into(),
            name: id.into(),
            path: "/unused".into(),
            branch: None,
            agents: vec![],
        }
    }

    fn terminal(id: &str, project: &str) -> AgentInstance {
        AgentInstance {
            id: id.into(),
            project: project.into(),
            agent_type: AgentType::Shell,
            title: id.into(),
            status: muxlane_core::model::AgentStatus::Idle,
            status_since: 0,
            seen: true,
            tmux_session: None,
        }
    }

    fn populate(app: &mut MuxlaneApp) {
        app.last_snapshot.projects = vec![project("a"), project("empty"), project("b")];
        app.last_snapshot.agents = vec![
            terminal("a-2", "a"),
            terminal("b-1", "b"),
            terminal("a-1", "a"),
        ];
        // Config order deliberately differs from alphabetical host order.
        for host in ["z-host", "a-host"] {
            app.remotes.push(muxlane_client::RemoteHost::new(
                muxlane_client::HostCfg {
                    name: host.into(),
                    target: muxlane_client::Target::Socket("/unused.sock".into()),
                    auth: muxlane_client::SshAuth::default(),
                    retry_base_ms: 500,
                },
                app.remote_event_tx.clone(),
            ));
            app.remote_snaps.insert(
                host.into(),
                Snapshot {
                    machine: Some(MachineInfo {
                        machine_id: host.into(),
                        name: host.into(),
                        os: "test".into(),
                        version: "test".into(),
                    }),
                    projects: vec![project("empty"), project("r1"), project("r2")],
                    agents: vec![
                        terminal(&format!("{host}-1"), "r1"),
                        terminal(&format!("{host}-2"), "r2"),
                    ],
                },
            );
            app.remote_states.insert(
                host.into(),
                muxlane_client::RemoteState::Offline("test".into()),
            );
        }
    }

    fn assert_order(app: &mut MuxlaneApp, expected: &[&str]) {
        for (index, id) in expected.iter().enumerate() {
            app.active = Some((*id).into());
            assert_eq!(
                app.adjacent_sidebar_session(true).unwrap().agent,
                expected[(index + 1) % expected.len()]
            );
            assert_eq!(
                app.adjacent_sidebar_session(false).unwrap().agent,
                expected[(index + expected.len() - 1) % expected.len()]
            );
        }
        for active in [None, Some("missing".into())] {
            app.active = active;
            assert_eq!(
                app.adjacent_sidebar_session(true).unwrap().agent,
                expected[0]
            );
            assert_eq!(
                app.adjacent_sidebar_session(false).unwrap().agent,
                expected[expected.len() - 1]
            );
        }
    }

    #[test]
    fn sidebar_order_is_stable_across_projects_remotes_and_collapse() {
        with_app(|cx, _window, view| {
            cx.update(|cx| {
                view.update(cx, |app, _| {
                    populate(app);
                    // Neither pane tab order nor expansion state participates in traversal.
                    for id in ["b-1", "a-1", "a-2"] {
                        app.pane_tree.open_tab(&app.active_pane, id.into());
                    }
                    app.collapsed_machines
                        .extend(["local", "remote:z-host", "remote:a-host"].map(String::from));
                    app.collapsed_projects.extend(
                        ["local:a", "local:b", "remote:z-host:r1", "remote:z-host:r2"]
                            .map(String::from),
                    );
                    assert_order(
                        app,
                        &[
                            "a-2", "a-1", "b-1", "z-host-1", "z-host-2", "a-host-1", "a-host-2",
                        ],
                    );
                    assert!(app.move_project_order(&app.local_machine_id(), "b", "a"));
                    assert!(app.move_project_order("z-host", "r2", "r1"));
                    assert_order(
                        app,
                        &[
                            "b-1", "a-2", "a-1", "z-host-2", "z-host-1", "a-host-1", "a-host-2",
                        ],
                    );
                    app.remotes.clear();
                    assert_order(app, &["b-1", "a-2", "a-1"]);
                })
            });
        });
    }

    #[test]
    fn empty_sidebar_navigation_is_a_noop_and_single_session_wraps() {
        with_app(|cx, window, view| {
            cx.update_window(window, |_, window, cx| {
                view.update(cx, |app, cx| {
                    app.last_snapshot.projects = vec![project("empty")];
                    app.active = Some("missing".into());
                    let pane = app.active_pane.clone();
                    app.focus.focus(window, cx);
                    app.next_tab(window, cx);
                    app.prev_tab(window, cx);
                    assert_eq!(app.active.as_deref(), Some("missing"));
                    assert_eq!(app.active_pane, pane);
                    assert!(app.focus.is_focused(window));
                    assert!(app.workspace.current_project().is_none());
                    app.last_snapshot.agents.push(terminal("only", "empty"));
                    assert_order(app, &["only"]);
                })
            })
            .unwrap();
        });
    }

    #[test]
    fn navigation_reveals_and_focuses_the_target_project_and_existing_pane() {
        with_app(|cx, window, view| {
            cx.update_window(window, |_, window, cx| {
                view.update(cx, |app, cx| {
                    populate(app);
                    // Cached in-memory terminals exercise focus without a PTY or network connection.
                    for id in [
                        "a-2", "a-1", "b-1", "z-host-1", "z-host-2", "a-host-1", "a-host-2",
                    ] {
                        let (vterm, clipboard) = muxlane_term::VTerm::new_with_clipboard(80, 24);
                        let (input, _rx) = tokio::sync::mpsc::unbounded_channel();
                        let term = MuxlaneApp::create_remote_term(
                            id.into(),
                            (vterm, clipboard),
                            input,
                            &app.font_family,
                            Theme::for_mode(app.theme_mode),
                            false,
                            cx,
                        );
                        app.terms.insert(id.into(), term);
                    }
                    app.select_project_workspace_inner(
                        ProjectKey::new(app.local_machine_id(), "a"),
                        cx,
                    );
                    app.pane_tree.open_tab(&app.active_pane, "a-2".into());
                    let split = app
                        .pane_tree
                        .split(
                            &app.active_pane,
                            muxlane_core::SplitAxis::Horizontal,
                            "a-1".into(),
                        )
                        .unwrap();
                    app.active = Some("a-2".into());
                    app.maximized_pane = Some(app.active_pane.clone());
                    app.collapsed_machines
                        .extend(["local", "remote:z-host", "remote:a-host"].map(String::from));
                    app.collapsed_projects.extend(
                        [
                            "local:a",
                            "local:b",
                            "local:empty",
                            "remote:z-host:r1",
                            "remote:z-host:r2",
                            "remote:a-host:r1",
                            "remote:a-host:r2",
                        ]
                        .map(String::from),
                    );

                    for (next, id, machine, project, collapse_machine, collapse_project) in [
                        (true, "a-1", "ux-test", "a", "local", "local:a"),
                        (true, "b-1", "ux-test", "b", "local", "local:b"),
                        (
                            true,
                            "z-host-1",
                            "z-host",
                            "r1",
                            "remote:z-host",
                            "remote:z-host:r1",
                        ),
                        (false, "b-1", "ux-test", "b", "local", "local:b"),
                    ] {
                        app.collapsed_machines.insert(collapse_machine.into());
                        app.collapsed_projects.insert(collapse_project.into());
                        if next {
                            app.next_tab(window, cx);
                        } else {
                            app.prev_tab(window, cx);
                        }
                        assert_eq!(app.active.as_deref(), Some(id));
                        assert_eq!(
                            app.workspace.current_project(),
                            Some(&ProjectKey::new(machine, project))
                        );
                        assert_eq!(
                            app.pane_tree
                                .group(&app.active_pane)
                                .unwrap()
                                .active
                                .as_deref(),
                            Some(id)
                        );
                        assert!(!app.collapsed_machines.contains(collapse_machine));
                        assert!(!app.collapsed_projects.contains(collapse_project));
                        assert!(app.collapsed_projects.contains("local:empty"));
                        assert!(app.collapsed_machines.contains("remote:a-host"));
                        assert!(app.maximized_pane.is_none());
                        if id == "a-1" {
                            assert_eq!(app.active_pane, split);
                        }
                        let focus = app.terms[id].focus_handle(cx);
                        assert!(focus.is_focused(window), "{id} must receive focus");
                    }
                    // Missing active chooses the edges, then wraps across all machines.
                    app.active = None;
                    app.prev_tab(window, cx);
                    assert_eq!(app.active.as_deref(), Some("a-host-2"));
                    app.next_tab(window, cx);
                    assert_eq!(app.active.as_deref(), Some("a-2"));
                    app.active = Some("missing".into());
                    app.next_tab(window, cx);
                    assert_eq!(app.active.as_deref(), Some("a-2"));
                })
            })
            .unwrap();
        });
    }
}
use super::*;
use gpui::{AnyWindowHandle, TestAppContext};

fn with_app(test: impl FnOnce(&mut TestAppContext, AnyWindowHandle, Entity<MuxlaneApp>)) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    with_app_on_runtime(&runtime, test);
}

fn with_app_on_runtime(
    runtime: &tokio::runtime::Runtime,
    test: impl FnOnce(&mut TestAppContext, AnyWindowHandle, Entity<MuxlaneApp>),
) {
    let directory = tempfile::tempdir().unwrap();
    let state = muxlane_server::ServerState::new(muxlane_core::model::MachineInfo {
        machine_id: "ux-test".into(),
        name: "test".into(),
        os: "test".into(),
        version: "test".into(),
    });
    let snapshot = state.snapshot();
    let server = MuxlaneServer::new_with_runtime(
        directory.path().join("unused.sock"),
        Arc::new(tokio::sync::RwLock::new(state)),
        muxlane_server::DirtyFlag::new(),
        runtime.handle().clone(),
    );
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        crate::shortcuts::install_keymap(cx, &Default::default()).unwrap();
    });
    let path = directory.path().join("state.json");
    let window = cx.add_window(move |window, cx| {
        let mut app = MuxlaneApp::new(
            window,
            cx,
            server,
            snapshot,
            vec![],
            Default::default(),
            path,
        );
        app.presets.clear();
        app
    });
    let view = window.root(&mut cx).unwrap();
    test(&mut cx, window.into(), view);
}

fn draw(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    cx.update_window(window, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

#[test]
fn terminal_shortcuts_dispatch_to_app_actions_and_disable_without_fixed_aliases() {
    use std::any::TypeId;
    with_app(|cx, window, view| {
        let actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let observed = actions.clone();
        let _subscription = cx.update(|cx| {
            cx.observe_keystrokes(move |event, _, _| {
                observed.borrow_mut().push(
                    event
                        .action
                        .as_ref()
                        .map(|action| action.as_any().type_id()),
                );
            })
        });
        cx.update_window(window, |_, window, cx| {
            let focus = view.read(cx).focus.clone();
            focus.focus(window, cx);
        })
        .unwrap();
        draw(cx, window);
        #[cfg(target_os = "macos")]
        let platform = "cmd";
        #[cfg(not(target_os = "macos"))]
        let platform = "super";
        for (key, expected) in [
            ("up", TypeId::of::<NewShellTab>()),
            ("right", TypeId::of::<SplitRight>()),
            ("down", TypeId::of::<SplitDown>()),
            ("k", TypeId::of::<PreviousTab>()),
            ("j", TypeId::of::<NextTab>()),
        ] {
            cx.simulate_keystrokes(window, &format!("{platform}-alt-{key}"));
            assert_eq!(actions.borrow().last(), Some(&Some(expected)));
        }
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                for action in ShortcutAction::ALL.into_iter().skip(1) {
                    action.set_binding(&mut app.shortcut_bindings, None);
                }
                crate::shortcuts::install_keymap(cx, &app.shortcut_bindings).unwrap();
            });
        });
        for key in ["up", "right", "down", "k", "j"] {
            actions.borrow_mut().clear();
            cx.simulate_keystrokes(window, &format!("{platform}-alt-{key}"));
            assert!(actions.borrow().iter().all(Option::is_none));
        }
        for key in ["ctrl-shift-t", "f6", "shift-f6"] {
            actions.borrow_mut().clear();
            cx.simulate_keystrokes(window, key);
            assert!(actions.borrow().iter().all(Option::is_none));
        }
        for (key, expected) in [
            ("ctrl-tab", TypeId::of::<NextTab>()),
            ("ctrl-shift-tab", TypeId::of::<PreviousTab>()),
        ] {
            cx.simulate_keystrokes(window, key);
            assert_eq!(actions.borrow().last(), Some(&Some(expected)));
        }
        for (action, key, expected) in [
            (
                ShortcutAction::NewTab,
                "ctrl-alt-t",
                TypeId::of::<NewShellTab>(),
            ),
            (
                ShortcutAction::SplitRight,
                "ctrl-alt-r",
                TypeId::of::<SplitRight>(),
            ),
            (
                ShortcutAction::SplitDown,
                "ctrl-alt-d",
                TypeId::of::<SplitDown>(),
            ),
        ] {
            cx.update(|cx| {
                view.update(cx, |app, cx| {
                    app.shortcut_bindings = crate::shortcuts::install_binding(
                        cx,
                        &app.shortcut_bindings,
                        action,
                        Some(key.into()),
                    )
                    .unwrap();
                });
            });
            actions.borrow_mut().clear();
            cx.simulate_keystrokes(window, key);
            assert_eq!(actions.borrow().last(), Some(&Some(expected)));
        }
    });
}

fn activate_settings_control(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    view: &Entity<MuxlaneApp>,
    id: &str,
) {
    cx.update_window(window, |_, window, cx| {
        let focus = view.read(cx).settings_focus.existing_control(id);
        focus.focus(window, cx);
    })
    .unwrap();
    draw(cx, window);
    cx.simulate_keystrokes(window, "enter");
    cx.update_window(window, |_, window, cx| {
        window.dispatch_event(
            gpui::PlatformInput::KeyUp(gpui::KeyUpEvent {
                keystroke: gpui::Keystroke::parse("enter").unwrap(),
            }),
            cx,
        );
    })
    .unwrap();
    draw(cx, window);
}

#[test]
fn default_terminal_settings_select_all_presets_and_persist_without_installed_filter() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("selected-preset.json");
    with_app(|cx, window, view| {
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |app, cx| {
                assert_eq!(app.default_terminal_preset, "shell");
                app.presets = muxlane_core::builtin_presets("/unused-shell");
                for preset in &mut app.presets {
                    if preset.id != "shell" {
                        preset.program = format!("/missing-test-program/{}", preset.id);
                    }
                }
                app.persistence = PersistenceWriter::new(path.clone());
                app.open_settings(window, cx);
            });
        })
        .unwrap();
        draw(cx, window);
        draw(cx, window);
        for id in [
            "shell", "claude", "codex", "pi", "opencode", "agy", "qwen", "kimi",
        ] {
            activate_settings_control(cx, window, &view, "settings-terminal-preset-select");
            activate_settings_control(
                cx,
                window,
                &view,
                &format!("settings-terminal-preset-option-{id}"),
            );
            cx.update(|cx| {
                let app = view.read(cx);
                assert_eq!(app.default_terminal_preset, id);
                assert_eq!(app.selected_terminal_preset().id, id);
                assert!(!app.settings_terminal_preset_menu);
            });
        }
        cx.update(|cx| {
            view.update(cx, |app, _| {
                // Dropping the writer drains queued saves before inspecting the state.
                app.persistence = PersistenceWriter::new(directory.path().join("unused.json"));
                app.default_terminal_preset = "unknown-preset".into();
                assert_eq!(app.selected_terminal_preset().id, "shell");
            });
        });
        assert_eq!(
            muxlane_store::load(&path).unwrap().default_terminal_preset,
            "kimi"
        );
    });
}

#[test]
fn settings_capture_rebinds_and_disables_all_default_terminal_actions() {
    with_app(|cx, window, view| {
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |app, cx| {
                app.settings_page = SettingsPage::Shortcuts;
                app.open_settings(window, cx);
            });
        })
        .unwrap();
        draw(cx, window);
        draw(cx, window);
        #[cfg(target_os = "macos")]
        let chord = "cmd-alt-v";
        #[cfg(not(target_os = "macos"))]
        let chord = "super-alt-v";
        for action in [
            ShortcutAction::NewTab,
            ShortcutAction::SplitRight,
            ShortcutAction::SplitDown,
        ] {
            activate_settings_control(
                cx,
                window,
                &view,
                &format!("settings-shortcut-record-{action:?}"),
            );
            cx.simulate_keystrokes(window, chord);
            draw(cx, window);
            cx.update(|cx| {
                let app = view.read(cx);
                assert_eq!(
                    action.binding(&app.shortcut_bindings).as_deref(),
                    Some("alt-platform-v")
                );
                assert_eq!(app.shortcut_error, None);
            });
            activate_settings_control(
                cx,
                window,
                &view,
                &format!("settings-shortcut-clear-{action:?}"),
            );
            assert!(cx.update(|cx| action.binding(&view.read(cx).shortcut_bindings).is_none()));
        }
    });
}

// Render the production sidebar and settings around a real editor without a PTY process.
struct SidebarEditorFixture {
    app: Entity<MuxlaneApp>,
    editor: Entity<TextField>,
    _subscription: Subscription,
}

impl Render for SidebarEditorFixture {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.app.update(cx, |app, cx| {
            let theme = Theme::for_mode(app.theme_mode);
            let mut root = div()
                .id("ux-editor-root")
                .track_focus(&app.focus)
                .size_full()
                .relative()
                .flex()
                .flex_col()
                .child(app.render_sidebar_footer(theme, cx))
                .child(self.editor.clone());
            if app.settings_open {
                root = root.child(app.render_settings(window, cx));
            }
            root
        })
    }
}

#[test]
fn mouse_opened_settings_restores_the_original_draft_editor() {
    with_app(|cx, window, view| {
        let editor = cx
            .update_window(window, |_, window, cx| {
                let editor = cx.new(|cx| TextField::new("Draft", window, cx));
                window.replace_root(cx, |_, cx| SidebarEditorFixture {
                    app: view.clone(),
                    editor: editor.clone(),
                    _subscription: cx.observe(&view, |_, _, cx| cx.notify()),
                });
                editor
            })
            .unwrap();
        draw(cx, window);
        cx.update_window(window, |_, window, cx| {
            editor.focus_handle(cx).focus(window, cx)
        })
        .unwrap();
        draw(cx, window);
        let mut visual = gpui::VisualTestContext::from_window(window, cx);
        let button = visual.debug_bounds("ux-open-settings").unwrap();
        visual.simulate_click(button.center(), Default::default());
        draw(cx, window);
        assert!(cx.update(|cx| view.read(cx).settings_open));
        cx.simulate_keystrokes(window, "escape");
        draw(cx, window);
        assert!(cx
            .update_window(window, |_, window, cx| editor
                .focus_handle(cx)
                .is_focused(window))
            .unwrap());
        assert!(
            cx.update(|cx| view.read(cx).active.is_none()),
            "restoration must not rely on active-session fallback"
        );
        cx.simulate_keystrokes(window, "z");
        assert_eq!(cx.update(|cx| editor.read(cx).text()), "z");
    });
}

#[test]
fn new_tab_button_follows_last_tab_when_tabs_fit() {
    with_app(|cx, window, view| {
        let mut visual = gpui::VisualTestContext::from_window(window, cx);
        visual.simulate_resize(size(px(1200.), px(640.)));
        for count in 1..=3 {
            let pane = cx.update(|cx| {
                view.update(cx, |app, cx| {
                    app.pane_tree
                        .open_tab(&app.active_pane, format!("adjacent-tab-{count}"));
                    cx.notify();
                    app.active_pane.clone()
                })
            });
            draw(cx, window);
            let scroll = cx.update(|cx| view.read(cx).pane_tab_scrolls[&pane].scroll.clone());
            let last = scroll.bounds_for_item(count - 1).unwrap();
            let button = visual.debug_bounds("ux-new-tab").unwrap();
            assert_eq!(scroll.max_offset().x, px(0.));
            assert!((button.left() - last.right()).abs() <= px(1.));
            assert_eq!(button.size.width, ui_px(28.));
            assert!(button.right() <= visual.debug_bounds("ux-pane-tools").unwrap().left());
        }
    });
}

#[test]
fn active_tab_scrolls_into_view_without_resetting_manual_scroll_or_moving_tools() {
    with_app(|cx, window, view| {
        let (ids, pane) = cx.update(|cx| {
            view.update(cx, |app, cx| {
                let ids: Vec<_> = (0..8).map(|index| format!("ux-tab-{index}")).collect();
                for id in &ids {
                    app.last_snapshot
                        .agents
                        .push(muxlane_core::model::AgentInstance {
                            id: id.clone(),
                            project: "ux-project".into(),
                            agent_type: muxlane_core::model::AgentType::Shell,
                            title: format!(
                                "{id}: a deliberately long session title for tab scrolling"
                            ),
                            status: muxlane_core::model::AgentStatus::Idle,
                            status_since: 0,
                            seen: true,
                            tmux_session: None,
                        });
                    app.pane_tree.open_tab(&app.active_pane, id.clone());
                }
                app.pane_tree.open_tab(&app.active_pane, ids[0].clone());
                cx.notify();
                (ids, app.active_pane.clone())
            })
        });
        let mut visual = gpui::VisualTestContext::from_window(window, cx);
        visual.simulate_resize(size(px(720.), px(640.)));
        draw(cx, window);
        let tools = visual.debug_bounds("ux-pane-tools").unwrap();
        let new_tab = visual.debug_bounds("ux-new-tab").unwrap();
        let scroll = cx.update(|cx| view.read(cx).pane_tab_scrolls[&pane].scroll.clone());
        assert_eq!(new_tab.left(), scroll.bounds().right());
        assert_eq!(new_tab.size.width, ui_px(28.));
        assert!(new_tab.right() <= tools.left());
        assert!(scroll.bounds().size.width > px(0.));
        assert!(
            scroll.max_offset().x > px(0.),
            "long tabs must overflow the narrow pane"
        );
        for (index, expected_negative) in [(7, true), (0, false)] {
            cx.update(|cx| {
                view.update(cx, |app, cx| {
                    app.pane_tree.open_tab(&pane, ids[index].clone());
                    cx.notify();
                })
            });
            draw(cx, window);
            let tab = scroll.bounds_for_item(index).unwrap();
            let viewport = scroll.bounds();
            assert!(tab.left() + scroll.offset().x >= viewport.left() - px(1.));
            assert!(tab.right() + scroll.offset().x <= viewport.right() + px(1.));
            assert_eq!(scroll.offset().x < px(0.), expected_negative);
            assert_eq!(visual.debug_bounds("ux-pane-tools").unwrap(), tools);
            assert_eq!(visual.debug_bounds("ux-new-tab").unwrap(), new_tab);
        }
        scroll.set_offset(gpui::point(-px(50.), px(0.)));
        cx.update(|cx| view.update(cx, |_, cx| cx.notify()));
        draw(cx, window);
        assert_eq!(scroll.offset().x, -px(50.));
        for width in [480., 600., 900.] {
            visual.simulate_resize(size(px(width), px(640.)));
            draw(cx, window);
            let button = visual.debug_bounds("ux-new-tab").unwrap();
            let tools = visual.debug_bounds("ux-pane-tools").unwrap();
            assert!(scroll.bounds().size.width > px(0.));
            assert!(scroll.max_offset().x > px(0.));
            assert_eq!(button.left(), scroll.bounds().right());
            assert_eq!(button.size.width, ui_px(28.));
            assert!(button.right() <= tools.left());
            assert!(tools.right() <= px(width));
        }
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                app.pane_tree = PaneNode::empty();
                app.active_pane = app.pane_tree.first_pane_id();
                cx.notify();
            })
        });
        draw(cx, window);
        assert!(!cx.update(|cx| view.read(cx).pane_tab_scrolls.contains_key(&pane)));
    });
}

#[test]
fn palette_letters_are_search_text_and_do_not_change_layout() {
    with_app(|cx, window, view| {
        for letter in ["h", "v", "x", "m"] {
            cx.update_window(window, |_, window, cx| {
                view.update(cx, |app, cx| {
                    app.palette_open = true;
                    app.palette_input.update(cx, |input, cx| input.reset(cx));
                    app.palette_input.focus_handle(cx).focus(window, cx);
                    let before = serde_json::to_value(&app.pane_tree).unwrap();
                    assert!(!app.handle_palette_key(
                        &gpui::Keystroke::parse(letter).unwrap(),
                        window,
                        cx
                    ));
                    assert_eq!(serde_json::to_value(&app.pane_tree).unwrap(), before);
                    assert!(app.palette_open);
                    assert!(app.maximized_pane.is_none());
                })
            })
            .unwrap();
            draw(cx, window);
            cx.simulate_keystrokes(window, letter);
            cx.update(|cx| {
                let app = view.read(cx);
                assert_eq!(app.palette_input.read(cx).text(), letter);
                assert!(app.palette_open);
                assert_eq!(app.pane_tree.leaf_count(), 1);
                assert!(app.maximized_pane.is_none());
            });
        }
    });
}

#[test]
fn pending_delete_cannot_be_cancelled_or_retargeted() {
    with_app(|cx, _window, view| {
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                let target =
                    |name: &str| crate::menus::DeleteTarget::RemoteMachine { host: name.into() };
                app.begin_delete(target("original"), cx);
                app.delete_busy = true;
                app.delete_error = Some("pending error".into());
                app.cancel_delete(cx);
                app.begin_delete(target("replacement"), cx);
                app.confirm_delete(cx);
                assert!(app.delete_busy);
                assert_eq!(app.delete_error.as_deref(), Some("pending error"));
                assert!(matches!(&app.delete_confirm.as_ref().unwrap().target,
            crate::menus::DeleteTarget::RemoteMachine { host } if host == "original"));
                app.delete_busy = false;
                app.cancel_delete(cx);
                assert!(app.delete_confirm.is_none());
                assert!(app.delete_error.is_none());
                app.begin_delete(
                    crate::menus::DeleteTarget::RemoteProject {
                        host: "missing".into(),
                        project: "project".into(),
                        label: "Project".into(),
                    },
                    cx,
                );
                app.confirm_delete(cx);
                assert!(!app.delete_busy);
                assert!(app.delete_confirm.is_some());
                assert!(app.delete_error.is_some());
            })
        })
    });
}

#[test]
fn invalid_replacement_credentials_preserve_existing_remote() {
    with_app(|cx, _window, view| {
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                let remote = muxlane_client::RemoteHost::new(
                    muxlane_client::HostCfg {
                        name: "existing".into(),
                        target: muxlane_client::parse_target("existing"),
                        auth: muxlane_client::SshAuth::SshConfig,
                        retry_base_ms: 500,
                    },
                    app.remote_event_tx.clone(),
                );
                remote.restore_machine_id(Some("original-machine".into()));
                app.remotes.push(remote.clone());
                app.remote_snaps
                    .insert("existing".into(), Default::default());
                app.connect_auth_mode = ConnectAuthMode::Password;
                for (username, password) in [("", "secret"), ("user", "")] {
                    app.connect_username
                        .update(cx, |input, cx| input.set_text(username, cx));
                    app.connect_password
                        .update(cx, |input, cx| input.set_text(password, cx));
                    app.add_remote_target("existing".into(), cx);
                    assert_eq!(app.remotes.len(), 1);
                    assert!(Arc::ptr_eq(&remote, &app.remotes[0]));
                    assert_eq!(remote.machine_id().as_deref(), Some("original-machine"));
                    assert!(app.remote_snaps.contains_key("existing"));
                    assert!(app.dialog_error.is_some());
                }
                // This fixture never drives the Tokio runtime, so replacement starts no network work.
                app.connect_username
                    .update(cx, |input, cx| input.set_text("user", cx));
                app.connect_password
                    .update(cx, |input, cx| input.set_text("  secret  ", cx));
                app.add_remote_target("existing".into(), cx);
                assert!(matches!(&app.remotes[0].cfg.auth,
            muxlane_client::SshAuth::Password { password, .. } if password == "  secret  "));
            })
        })
    });
}

#[test]
fn settings_tab_and_f6_cycle_locally_and_restore_focus() {
    with_app(|cx, window, view| {
        let previous = cx
            .update_window(window, |_, window, cx| {
                view.update(cx, |app, cx| {
                    app.focus.focus(window, cx);
                    app.open_settings(window, cx);
                    app.focus.clone()
                })
            })
            .unwrap();
        draw(cx, window);
        draw(cx, window);
        let first = cx
            .update_window(window, |_, window, cx| window.focused(cx).unwrap())
            .unwrap();
        assert_ne!(first, previous);
        for keys in ["tab", "shift-tab", "f6", "shift-f6"] {
            let mut seen = Vec::new();
            for _ in 0..8 {
                cx.simulate_keystrokes(window, keys);
                draw(cx, window);
                let focused = cx
                    .update_window(window, |_, window, cx| window.focused(cx).unwrap())
                    .unwrap();
                assert_ne!(focused, previous);
                assert!(!seen.contains(&focused));
                seen.push(focused);
            }
            assert!(cx
                .update_window(window, |_, window, _| first.is_focused(window))
                .unwrap());
        }
        cx.simulate_keystrokes(window, "shift-tab");
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                app.settings_page = SettingsPage::Appearance;
                cx.notify();
            })
        });
        draw(cx, window);
        assert!(cx
            .update_window(window, |_, window, _| first.is_focused(window))
            .unwrap());
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                app.settings_theme_menu = true;
                cx.notify();
            })
        });
        draw(cx, window);
        // Close, three navigation rows, theme trigger, then the first theme option.
        for _ in 0..5 {
            cx.simulate_keystrokes(window, "tab");
            draw(cx, window);
        }
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                app.settings_theme_menu = false;
                cx.notify();
            })
        });
        draw(cx, window);
        assert!(cx
            .update_window(window, |_, window, _| first.is_focused(window))
            .unwrap());
        cx.simulate_keystrokes(window, "escape");
        assert!(cx
            .update_window(window, |_, window, _| previous.is_focused(window))
            .unwrap());
        assert!(!cx.update(|cx| view.read(cx).settings_open));
    });
}
