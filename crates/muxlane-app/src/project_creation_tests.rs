//! Project creation must finish in the current UI, without reopening the application.
use super::*;
use std::time::{Duration, Instant};

fn wait_for_project_request(cx: &mut TestAppContext, view: &Entity<MuxlaneApp>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        if cx.update(|cx| !view.read(cx).project_add_busy) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "project request did not complete"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn project_submission_notifies_before_background_work_finishes() {
    with_app(|cx, _window, view| {
        let directory = tempfile::tempdir().unwrap();
        let notifications = std::rc::Rc::new(std::cell::Cell::new(0));
        let observed = notifications.clone();
        let _subscription =
            cx.update(|cx| cx.observe(&view, move |_, _| observed.set(observed.get() + 1)));
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                app.project_dialog = true;
                app.submit_local_project(directory.path().display().to_string(), false, cx);
                assert!(app.project_add_busy);
            });
        });
        assert!(
            notifications.get() > 0,
            "the Adding state must request a redraw immediately"
        );
        wait_for_project_request(cx, &view);
    });
}

#[test]
fn adding_project_reveals_it_without_restarting() {
    with_app(|cx, window, view| {
        let directory = tempfile::tempdir().unwrap();
        let project_path = directory.path().join("zbase");
        std::fs::create_dir(&project_path).unwrap();
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |app, cx| {
                app.collapsed_machines.insert("local".into());
                app.project_dialog = true;
                app.project_input.update(cx, |input, cx| {
                    input.set_text(project_path.display().to_string(), cx);
                });
                app.project_input.focus_handle(cx).focus(window, cx);
                cx.notify();
            });
        })
        .unwrap();
        draw(cx, window);
        let mut visual = gpui::VisualTestContext::from_window(window, cx);
        let submit = visual.debug_bounds("ux-project-submit").unwrap();
        visual.simulate_click(submit.center(), Default::default());
        wait_for_project_request(cx, &view);
        cx.update(|cx| {
            let app = view.read(cx);
            assert!(!app.project_dialog);
            assert!(app.dialog_error.is_none());
            assert!(app.project_input.read(cx).text().is_empty());
            assert!(!app.collapsed_machines.contains("local"));
            let project = &app.last_snapshot.projects[0];
            assert_eq!(project.path, project_path.canonicalize().unwrap());
            assert_eq!(
                app.workspace.current_project(),
                Some(&ProjectKey::new(app.local_machine_id(), project.id.clone()))
            );
        });
        draw(cx, window);
        let mut visual = gpui::VisualTestContext::from_window(window, cx);
        assert!(visual.debug_bounds("ux-project-zbase").is_some());
    });
}

#[test]
fn missing_project_requires_confirmation_and_then_appears() {
    with_app(|cx, window, view| {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("new/project");
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                app.project_dialog = true;
                app.submit_local_project(path.display().to_string(), false, cx);
            });
        });
        wait_for_project_request(cx, &view);
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                assert!(app.project_dialog);
                assert!(app.pending_project_creation.is_some());
                assert!(app.last_snapshot.projects.is_empty());
                assert!(!path.exists());
                app.confirm_project_create(cx);
            });
        });
        wait_for_project_request(cx, &view);
        cx.update(|cx| {
            let app = view.read(cx);
            assert!(!app.project_dialog);
            assert!(app.pending_project_creation.is_none());
            assert!(app.dialog_error.is_none());
            assert_eq!(
                app.last_snapshot.projects[0].path,
                path.canonicalize().unwrap()
            );
        });
        draw(cx, window);
    });
}

#[test]
fn invalid_project_keeps_dialog_open_and_reports_error() {
    with_app(|cx, _window, view| {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file");
        std::fs::write(&path, "not a directory").unwrap();
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                app.project_dialog = true;
                app.submit_local_project(path.display().to_string(), false, cx);
            });
        });
        wait_for_project_request(cx, &view);
        cx.update(|cx| {
            let app = view.read(cx);
            assert!(app.project_dialog);
            assert!(app.pending_project_creation.is_none());
            assert!(app
                .dialog_error
                .as_deref()
                .unwrap()
                .contains("not a directory"));
            assert!(app.last_snapshot.projects.is_empty());
        });
    });
}

#[test]
fn project_menu_offers_only_detected_local_editors() {
    with_app(|cx, window, view| {
        let directory = tempfile::tempdir().unwrap();
        let bin = directory.path().join("bin");
        let marker = directory.path().join("opened");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(
            bin.join("zed"),
            format!("#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n", marker.display()),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(bin.join("zed"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let project_path = directory.path().join("zbase");
        std::fs::create_dir(&project_path).unwrap();
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                app.local_editors = crate::editors::detect_with_for_test(&bin, directory.path());
                assert_eq!(app.local_editors.len(), 1);
                app.project_dialog = true;
                app.submit_local_project(project_path.display().to_string(), false, cx);
            });
        });
        wait_for_project_request(cx, &view);
        let open_menu = |cx: &mut gpui::TestAppContext, target| {
            cx.update(|cx| {
                view.update(cx, |app, cx| {
                    app.tree_menu = Some(crate::menus::TreeMenu {
                        target,
                        position: gpui::Point::new(px(40.), px(40.)),
                    });
                    cx.notify();
                });
            });
        };
        let project_id = cx.update(|cx| view.read(cx).last_snapshot.projects[0].id.clone());
        open_menu(
            cx,
            crate::menus::DeleteTarget::LocalProject {
                project: project_id,
                label: "zbase".into(),
            },
        );
        draw(cx, window);
        let mut visual = gpui::VisualTestContext::from_window(window, cx);
        assert!(visual.debug_bounds("tree-open-vscode").is_none());
        let zed = visual.debug_bounds("tree-open-zed").unwrap();
        visual.simulate_click(zed.center(), Default::default());
        draw(cx, window);
        assert!(cx.update(|cx| view.read(cx).tree_menu.is_none()));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !marker.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "editor was not launched"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            std::fs::read_to_string(&marker).unwrap(),
            project_path.canonicalize().unwrap().display().to_string()
        );
        // Remote projects must not offer local editors.
        open_menu(
            cx,
            crate::menus::DeleteTarget::RemoteProject {
                host: "peer".into(),
                project: "r".into(),
                label: "r".into(),
            },
        );
        draw(cx, window);
        let mut visual = gpui::VisualTestContext::from_window(window, cx);
        assert!(visual.debug_bounds("tree-open-zed").is_none());
    });
}
