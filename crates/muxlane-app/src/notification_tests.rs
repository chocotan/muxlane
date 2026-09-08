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
