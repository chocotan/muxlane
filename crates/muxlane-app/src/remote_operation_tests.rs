use super::*;
use muxlane_core::model::{AgentInstance, AgentStatus, AgentType, MachineInfo, Project, Snapshot};
use muxlane_core::protocol::{methods, read_frame, write_frame, Request, Response};
use std::time::{Duration, Instant};

fn wait_for(cx: &mut TestAppContext, mut condition: impl FnMut(&mut TestAppContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        if condition(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "remote operation timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn remote_mark_seen_writes_through_and_survives_client_restart() {
    // 真实远端 server（非手写 frame），验证 agent.mark_seen 能力协商、真实写回，
    // 且一个完全新的客户端连接（模拟本地客户端重启，无任何内存缓存）仍能看到
    // Idle/seen，不会重新闪烁。
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("peer.sock");
    let agent_id: muxlane_core::model::AgentId = "remote-done".into();
    let peer_state = Arc::new(tokio::sync::RwLock::new(muxlane_server::ServerState::new(
        MachineInfo {
            machine_id: "peer-machine".into(),
            name: "peer".into(),
            os: "test".into(),
            version: "test".into(),
        },
    )));
    let peer_server = muxlane_server::MuxlaneServer::new_with_runtime(
        socket.clone(),
        Arc::clone(&peer_state),
        muxlane_server::DirtyFlag::new(),
        runtime.handle().clone(),
    );
    {
        let srv = Arc::clone(&peer_server);
        runtime.spawn(async move { srv.serve().await.unwrap() });
    }
    runtime.block_on(async {
        for _ in 0..50 {
            if tokio::net::UnixStream::connect(&socket).await.is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("peer socket never became ready");
    });
    runtime.block_on(async {
        let mut state = peer_state.write().await;
        state.projects.push(Project {
            id: "p1".into(),
            name: "p1".into(),
            path: "/tmp/p1".into(),
            branch: None,
            agents: vec![agent_id.clone()],
        });
        state.agents.push(AgentInstance {
            id: agent_id.clone(),
            project: "p1".into(),
            agent_type: AgentType::Shell,
            title: "Terminal".into(),
            status: AgentStatus::Done,
            status_since: 10,
            seen: false,
            tmux_session: None,
        });
    });

    with_app_on_runtime(&runtime, |cx, window, app| {
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                let remote = muxlane_client::RemoteHost::new(
                    muxlane_client::HostCfg {
                        name: "peer".into(),
                        target: muxlane_client::Target::Socket(socket.display().to_string()),
                        auth: Default::default(),
                        retry_base_ms: 200,
                    },
                    app.remote_event_tx.clone(),
                );
                app.server.rt_spawn(Arc::clone(&remote).run_loop());
                app.remotes.push(remote);
                let _ = cx;
            });
        });
        wait_for(cx, |cx| {
            cx.update(|cx| {
                app.read(cx)
                    .remote_snaps
                    .get("peer")
                    .is_some_and(|snap| snap.agent(&agent_id).is_some())
            })
        });
        cx.update(|cx| {
            let app = app.read(cx);
            let remote = app.remotes.iter().find(|r| r.cfg.name == "peer").unwrap();
            assert!(
                remote.supports(muxlane_core::protocol::features::AGENT_MARK_SEEN),
                "real peer must advertise agent.mark_seen",
            );
        });
        cx.update_window(window, |_, window, cx| {
            app.update(cx, |app, cx| {
                let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
                let term = MuxlaneApp::create_remote_term(
                    agent_id.clone(),
                    muxlane_term::VTerm::new_with_clipboard(80, 24),
                    sender,
                    &app.font_family,
                    Theme::for_mode(app.theme_mode),
                    false,
                    cx,
                );
                app.terms.insert(agent_id.clone(), term);
                // 真实的“用户打开这个 tab”路径，不是直接调 mark_agent_seen。
                app.open_agent(&agent_id, window, cx);
            });
        })
        .unwrap();
        cx.update(|cx| {
            assert!(
                app.read(cx).remote_snaps["peer"]
                    .agent(&agent_id)
                    .unwrap()
                    .seen
            );
        });
    });

    // 等待 RPC 真正写到远端 server 自己的状态（不是客户端缓存）。
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let done = runtime.block_on(async {
            peer_state
                .read()
                .await
                .agents
                .iter()
                .find(|a| a.id == agent_id)
                .is_some_and(|a| a.status == AgentStatus::Idle && a.seen)
        });
        if done {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "remote server never recorded mark_seen"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    // 模拟客户端重启：全新 RemoteHost + 全新 channel，与上面那个 app 没有任何共享内存。
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let fresh_host = muxlane_client::RemoteHost::new(
        muxlane_client::HostCfg {
            name: "peer".into(),
            target: muxlane_client::Target::Socket(socket.display().to_string()),
            auth: Default::default(),
            retry_base_ms: 200,
        },
        tx,
    );
    runtime.spawn(Arc::clone(&fresh_host).run_loop());
    let snap = runtime.block_on(async {
        loop {
            match rx.recv().await {
                Some(muxlane_client::ClientEvent::StateChanged {
                    state: muxlane_client::RemoteState::Online(snap),
                    ..
                }) => break snap,
                Some(_) => continue,
                None => panic!("fresh client channel closed before Online"),
            }
        }
    });
    let agent = snap.agent(&agent_id).unwrap();
    assert_eq!(
        agent.status,
        AgentStatus::Idle,
        "restart must not resurrect Done/Failed"
    );
    assert!(agent.seen, "restart must not resurrect the unread flag");
}

#[test]
fn remote_forget_then_late_spawn_success_does_not_restore_layout() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("late.sock");
    let listener = runtime.block_on(async { tokio::net::UnixListener::bind(&socket).unwrap() });
    let (release, gate) = tokio::sync::oneshot::channel();
    let (received, request_received) = std::sync::mpsc::channel();
    let peer = runtime.spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = tokio::io::split(stream);
        let request: Request = serde_json::from_value(
            read_frame(&mut tokio::io::BufReader::new(read))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(request.method, methods::AGENT_SPAWN);
        received.send(()).unwrap();
        gate.await.unwrap();
        let agent = AgentInstance {
            id: "late".into(),
            project: "r".into(),
            agent_type: AgentType::Shell,
            title: "late".into(),
            status: AgentStatus::Idle,
            status_since: 0,
            seen: true,
            tmux_session: None,
        };
        write_frame(
            &mut write,
            &Response::ok(request.id, serde_json::to_value(agent).unwrap()),
        )
        .await
        .unwrap();
    });
    with_app_on_runtime(&runtime, |cx, window, view| {
        let remote = cx
            .update_window(window, |_, window, cx| {
                view.update(cx, |app, cx| {
                    let remote = muxlane_client::RemoteHost::new(
                        muxlane_client::HostCfg {
                            name: "remote".into(),
                            target: muxlane_client::Target::Socket(socket.display().to_string()),
                            auth: Default::default(),
                            retry_base_ms: 200,
                        },
                        app.remote_event_tx.clone(),
                    );
                    app.remotes.push(remote.clone());
                    app.remote_snaps.insert(
                        "remote".into(),
                        Snapshot {
                            machine: Some(MachineInfo {
                                machine_id: "remote-machine".into(),
                                name: "remote".into(),
                                os: "test".into(),
                                version: "test".into(),
                            }),
                            projects: vec![Project {
                                id: "r".into(),
                                name: "r".into(),
                                path: "/unused".into(),
                                branch: None,
                                agents: vec![],
                            }],
                            agents: vec![],
                        },
                    );
                    app.select_project_workspace_inner(ProjectKey::new("remote-machine", "r"), cx);
                    app.spawn_remote_agent(
                        crate::remotes::RemoteAgentSpawnRequest {
                            host: "remote".into(),
                            project: "r".into(),
                            preset: None,
                            preferred_pane: None,
                            split_axis: None,
                        },
                        window,
                        cx,
                    );
                    remote
                })
            })
            .unwrap();
        wait_for(cx, |_| request_received.try_recv().is_ok());
        cx.update(|cx| {
            view.update(cx, |app, cx| {
                app.begin_delete(
                    crate::menus::DeleteTarget::RemoteMachine {
                        host: "remote".into(),
                    },
                    cx,
                );
                app.confirm_delete(cx);
            })
        });
        release.send(()).unwrap();
        runtime.block_on(peer).unwrap();
        wait_for(cx, |_| Arc::strong_count(&remote) == 1);
        cx.run_until_parked();
        cx.update(|cx| {
            let app = view.read(cx);
            assert!(app.remotes.is_empty());
            assert!(!app.remote_snaps.contains_key("remote"));
            assert!(!app
                .floating
                .layouts
                .contains_key(&ProjectKey::new("remote-machine", "r")));
            assert!(!app.terms.contains_key("late"));
            assert!(app.active.is_none());
        });
    });
}

#[test]
fn remote_spawn_and_delete_from_gpui_without_tokio_context() {
    assert!(tokio::runtime::Handle::try_current().is_err());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("remote.sock");
    let listener = runtime.block_on(async { tokio::net::UnixListener::bind(&socket).unwrap() });
    let agent = AgentInstance {
        id: "remote-terminal".into(),
        project: "remote-project".into(),
        agent_type: AgentType::Shell,
        title: "Terminal".into(),
        status: AgentStatus::Idle,
        status_since: 0,
        seen: true,
        tmux_session: None,
    };
    let response_agent = agent.clone();
    let peer = runtime.spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = tokio::io::split(stream);
        let mut read = tokio::io::BufReader::new(read);
        // Exercise both a cold RPC connection and its reuse (which also needs a Tokio timer).
        for method in [methods::AGENT_SPAWN, methods::AGENT_DELETE] {
            let request: Request =
                serde_json::from_value(read_frame(&mut read).await.unwrap()).unwrap();
            assert_eq!(request.method, method);
            let result = if method == methods::AGENT_SPAWN {
                assert_eq!(request.params["project"], "remote-project");
                serde_json::to_value(&response_agent).unwrap()
            } else {
                assert_eq!(request.params["agent"], "remote-terminal");
                serde_json::json!({"ok": true})
            };
            write_frame(&mut write, &Response::ok(request.id, result))
                .await
                .unwrap();
        }
    });
    with_app_on_runtime(&runtime, |cx, window, app| {
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                let snapshot = Snapshot {
                    machine: Some(MachineInfo {
                        machine_id: "remote-machine".into(),
                        name: "remote".into(),
                        os: "test".into(),
                        version: "test".into(),
                    }),
                    projects: vec![Project {
                        id: agent.project.clone(),
                        name: "Remote project".into(),
                        path: "/unused".into(),
                        branch: None,
                        agents: vec![],
                    }],
                    agents: vec![],
                };
                app.remote_snaps.insert("remote".into(), snapshot.clone());
                app.remote_states.insert(
                    "remote".into(),
                    muxlane_client::RemoteState::Online(snapshot),
                );
                app.remotes.push(muxlane_client::RemoteHost::new(
                    muxlane_client::HostCfg {
                        name: "remote".into(),
                        target: muxlane_client::Target::Socket(socket.display().to_string()),
                        auth: Default::default(),
                        retry_base_ms: 200,
                    },
                    app.remote_event_tx.clone(),
                ));
                // Keep the test scoped to RPC and UI cleanup, without starting a terminal mirror.
                let (input, _receiver) = tokio::sync::mpsc::unbounded_channel();
                let term = MuxlaneApp::create_remote_term(
                    agent.id.clone(),
                    muxlane_term::VTerm::new_with_clipboard(80, 24),
                    input,
                    &app.font_family,
                    Theme::for_mode(app.theme_mode),
                    false,
                    cx,
                );
                app.terms.insert(agent.id.clone(), term);
            });
        });
        cx.update_window(window, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.spawn_remote_agent(
                    crate::remotes::RemoteAgentSpawnRequest {
                        host: "remote".into(),
                        project: agent.project.clone(),
                        preset: None,
                        preferred_pane: None,
                        split_axis: None,
                    },
                    window,
                    cx,
                );
            });
        })
        .unwrap();
        wait_for(cx, |cx| {
            cx.update(|cx| {
                app.read(cx).remote_snaps["remote"]
                    .agent(&agent.id)
                    .is_some()
            })
        });
        cx.update_window(window, |_, window, cx| {
            app.update(cx, |app, cx| {
                assert_eq!(app.active.as_ref(), Some(&agent.id));
                app.request_session_delete(&agent.id, true, window, cx);
            });
        })
        .unwrap();
        wait_for(cx, |cx| {
            cx.update(|cx| {
                app.read(cx).remote_snaps["remote"]
                    .agent(&agent.id)
                    .is_none()
            })
        });
        cx.update(|cx| {
            let app = app.read(cx);
            assert!(!app.terms.contains_key(&agent.id));
            assert_ne!(app.active.as_ref(), Some(&agent.id));
            assert!(app.remote_snaps["remote"].projects[0].agents.is_empty());
        });
    });
    runtime.block_on(peer).unwrap();
}

#[test]
fn remote_operations_support_reactor_io_and_preserve_typed_errors() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    with_app_on_runtime(&runtime, |cx, _window, app| {
        assert!(tokio::runtime::Handle::try_current().is_err());
        let (tx, rx) = std::sync::mpsc::channel();
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                let task = app.spawn_remote_operation(async {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    let directory = tempfile::tempdir()?;
                    let path = directory.path().join("probe");
                    tokio::fs::write(&path, b"reactor").await?;
                    assert_eq!(tokio::fs::read(path).await?, b"reactor");
                    assert!(tokio::process::Command::new("true")
                        .status()
                        .await?
                        .success());
                    Err::<(), _>(
                        muxlane_client::RpcCallError {
                            method: methods::PROJECT_ADD.into(),
                            code: muxlane_core::protocol::error_codes::PATH_NOT_FOUND.into(),
                            message: "missing directory".into(),
                        }
                        .into(),
                    )
                });
                cx.spawn(async move |_, _| {
                    tx.send(task.await).unwrap();
                })
                .detach();
            });
        });
        wait_for(cx, |_| match rx.try_recv() {
            Ok(result) => {
                let error = result.unwrap_err();
                let error = error
                    .downcast_ref::<muxlane_client::RpcCallError>()
                    .unwrap();
                assert_eq!(
                    error.code,
                    muxlane_core::protocol::error_codes::PATH_NOT_FOUND
                );
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(error) => panic!("operation result lost: {error}"),
        });
    });
}
