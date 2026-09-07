//! 服务端集成测试：真 UnixListener + 真 PTY + 裸 TCP 客户端模拟
use muxlane_core::model::{AgentId, AgentInstance, AgentStatus, AgentType, MachineInfo, Project};
use muxlane_core::protocol::{
    events, methods, read_frame, write_frame, EventMsg, ProjectAddParams, Request, Response,
};
use muxlane_server::{DirtyFlag, MuxlaneServer, ProjectAddError, ServerState};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::BufReader;
use tokio::net::UnixStream;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct TestServer {
    server: Arc<MuxlaneServer>,
    _task: AbortOnDrop,
}

impl Deref for TestServer {
    type Target = Arc<MuxlaneServer>;

    fn deref(&self) -> &Self::Target {
        &self.server
    }
}

async fn spawn_server() -> (
    TestServer,
    PathBuf,
    Arc<RwLock<ServerState>>,
    DirtyFlag,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("muxlane.sock");
    let state = Arc::new(RwLock::new(ServerState::new(MachineInfo {
        machine_id: "m_test".into(),
        name: "test-box".into(),
        os: "linux".into(),
        version: "0.1.0".into(),
    })));
    let dirty = DirtyFlag::new();
    let server = MuxlaneServer::new(sock.clone(), Arc::clone(&state), dirty.clone());
    let srv = Arc::clone(&server);
    let task = AbortOnDrop(tokio::spawn(async move { srv.serve().await.unwrap() }));
    // 等 socket 就绪
    for _ in 0..50 {
        if UnixStream::connect(&sock).await.is_ok() {
            // 占位连接已建立了一个（无害，server 支持多连接）
            return (
                TestServer {
                    server,
                    _task: task,
                },
                sock,
                state,
                dirty,
                dir,
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("server socket never became ready");
}

async fn rpc_call(sock: &PathBuf, method: &str, params: serde_json::Value) -> Response {
    let mut conn = BufReader::new(UnixStream::connect(sock).await.unwrap());
    write_frame(
        &mut conn,
        &Request {
            id: 1,
            method: method.into(),
            params,
        },
    )
    .await
    .unwrap();
    loop {
        let value = read_frame(&mut conn).await.unwrap();
        if value.get("event").is_none() {
            return serde_json::from_value(value).unwrap();
        }
    }
}

fn project_params(path: impl Into<String>, create_if_missing: bool) -> ProjectAddParams {
    ProjectAddParams {
        path: path.into(),
        name: None,
        create_if_missing,
    }
}

async fn launch_agent(
    server: &Arc<MuxlaneServer>,
    _state: &Arc<RwLock<ServerState>>,
    script: &str,
) -> AgentId {
    let agent_id = muxlane_core::model::new_id("shell");
    let cfg = muxlane_term::LaunchCfg {
        agent: agent_id.clone(),
        agent_type: AgentType::Shell,
        cwd: std::env::temp_dir(),
        env: vec![],
        program_override: Some("bash".into()),
        args: vec!["-c".into(), script.into()],
        cols: 80,
        rows: 24,
        tmux_session: None,
    };
    let sess = muxlane_term::PtySession::spawn(cfg).unwrap();
    let project = Project {
        id: "p_test".into(),
        name: "testproj".into(),
        path: "/tmp/testproj".into(),
        branch: Some("main".into()),
        agents: vec![],
    };
    let instance = AgentInstance {
        id: agent_id.clone(),
        project: "p_test".into(),
        agent_type: AgentType::Shell,
        title: "test".into(),
        status: AgentStatus::Idle,
        status_since: muxlane_core::model::now_secs(),
        seen: true,
        tmux_session: None,
    };
    server.restore_agent(project, instance, sess).await;
    agent_id
}

#[tokio::test]
async fn local_state_changes_wake_dirty_subscribers() {
    let (server, _sock, state, _dirty, _dir) = spawn_server().await;
    let mut changes = server.subscribe_dirty();

    launch_agent(&server, &state, "sleep 5").await;

    tokio::time::timeout(std::time::Duration::from_secs(1), changes.changed())
        .await
        .expect("state change notification timed out")
        .expect("state change channel closed");
    assert_eq!(server.snapshot().await.agents.len(), 1);
}

#[tokio::test]
async fn project_add_api_creates_validates_and_deduplicates_paths() {
    let (server, _sock, state, _dirty, dir) = spawn_server().await;
    let store_path = dir.path().join("state.json");
    server.set_persistence_path(store_path.clone());

    let existing = dir.path().join("existing");
    std::fs::create_dir(&existing).unwrap();
    let first = server
        .add_project(project_params(existing.display().to_string(), false))
        .await
        .unwrap();
    assert_eq!(first.path, existing.canonicalize().unwrap());

    let missing = dir.path().join("missing");
    let error = server
        .add_project(project_params(missing.display().to_string(), false))
        .await
        .unwrap_err();
    let error = error.downcast_ref::<ProjectAddError>().unwrap();
    assert_eq!(
        error.code(),
        muxlane_core::protocol::error_codes::PATH_NOT_FOUND
    );
    assert!(error.to_string().contains("No such file or directory"));
    assert!(!missing.exists());
    assert_eq!(state.read().await.projects.len(), 1);

    let nested = dir.path().join("new/parent/project");
    let created = server
        .add_project(project_params(nested.display().to_string(), true))
        .await
        .unwrap();
    assert!(nested.is_dir());
    assert_eq!(created.path, nested.canonicalize().unwrap());

    let file = dir.path().join("project-file");
    std::fs::write(&file, "not a directory").unwrap();
    let error = server
        .add_project(project_params(file.display().to_string(), false))
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<ProjectAddError>().unwrap().code(),
        muxlane_core::protocol::error_codes::NOT_A_DIRECTORY
    );

    let child_of_file = file.join("child");
    let error = server
        .add_project(project_params(child_of_file.display().to_string(), true))
        .await
        .unwrap_err();
    let error = error.downcast_ref::<ProjectAddError>().unwrap();
    assert_eq!(
        error.code(),
        muxlane_core::protocol::error_codes::CREATE_DIRECTORY_FAILED
    );
    assert!(error.to_string().contains("Not a directory"));
    assert!(!child_of_file.exists());

    let error = server
        .add_project(project_params("~another-user/project", false))
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<ProjectAddError>().unwrap().code(),
        muxlane_core::protocol::error_codes::INVALID_PATH
    );

    let canonical = dir.path().join("concurrent");
    std::fs::create_dir(&canonical).unwrap();
    let alias = canonical.join(".");
    let (left, right) = tokio::join!(
        server.add_project(project_params(canonical.display().to_string(), false)),
        server.add_project(project_params(alias.display().to_string(), false)),
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.id, right.id);
    assert_eq!(
        state
            .read()
            .await
            .projects
            .iter()
            .filter(|project| project.path == canonical.canonicalize().unwrap())
            .count(),
        1
    );
    let expected = vec![first, created, left];
    assert_eq!(server.snapshot().await.projects, expected);
    assert_eq!(muxlane_store::load(&store_path).unwrap().projects, expected);
}

#[tokio::test]
async fn immediate_persistence_tracks_project_agent_and_supervisor_removal() {
    let (server, _sock, _state, _dirty, dir) = spawn_server().await;
    let store_path = dir.path().join("state.json");
    server.set_persistence_path(store_path.clone());

    let project_dir = dir.path().join("persisted-project");
    std::fs::create_dir(&project_dir).unwrap();
    let project = server
        .add_project(project_params(project_dir.display().to_string(), false))
        .await
        .unwrap();
    assert_eq!(muxlane_store::load(&store_path).unwrap().projects.len(), 1);

    let live = server
        .spawn_agent(muxlane_core::protocol::AgentSpawnParams {
            project: project.id.clone(),
            agent_type: Some(AgentType::Shell),
            program: Some("bash".into()),
            args: Some(vec!["-c".into(), "sleep 30".into()]),
            env: None,
            preset_name: None,
        })
        .await
        .unwrap();
    let live_tmux = live.tmux_session.clone().unwrap();
    let _live_cleanup = KillTmuxSessionOnDrop(live_tmux.clone());
    for _ in 0..100 {
        if tmux_session_exists(&live_tmux) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(muxlane_store::load(&store_path).unwrap().sessions.len(), 1);

    let deleted = server.delete_agent(&live.id).await.unwrap();
    assert!(deleted.failed_agents.is_empty());
    assert!(muxlane_store::load(&store_path)
        .unwrap()
        .sessions
        .is_empty());

    let exited = server
        .spawn_agent(muxlane_core::protocol::AgentSpawnParams {
            project: project.id.clone(),
            agent_type: Some(AgentType::Shell),
            program: Some("bash".into()),
            args: Some(vec!["-c".into(), "exit 0".into()]),
            env: None,
            preset_name: None,
        })
        .await
        .unwrap();
    assert_eq!(muxlane_store::load(&store_path).unwrap().sessions.len(), 1);
    for _ in 0..100 {
        server.maintain_sessions().await;
        if !server
            .snapshot()
            .await
            .agents
            .iter()
            .any(|agent| agent.id == exited.id)
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        !server
            .snapshot()
            .await
            .agents
            .iter()
            .any(|agent| agent.id == exited.id),
        "supervisor did not remove the exited session"
    );
    assert!(muxlane_store::load(&store_path)
        .unwrap()
        .sessions
        .is_empty());

    let deleted = server.delete_project(&project.id).await.unwrap();
    assert!(deleted.failed_agents.is_empty());
    assert!(muxlane_store::load(&store_path)
        .unwrap()
        .projects
        .is_empty());
}

#[tokio::test]
async fn project_add_rpc_advertises_and_enforces_create_capability() {
    let (_server, sock, state, _dirty, dir) = spawn_server().await;

    let hello = rpc_call(&sock, methods::SYSTEM_HELLO, serde_json::json!({})).await;
    let hello: muxlane_core::protocol::HelloResult =
        serde_json::from_value(hello.result.unwrap()).unwrap();
    assert!(hello
        .features
        .iter()
        .any(|feature| feature == muxlane_core::protocol::features::PROJECT_CREATE));

    let missing = dir.path().join("rpc-missing");
    let response = rpc_call(
        &sock,
        methods::PROJECT_ADD,
        serde_json::json!({"path": missing}),
    )
    .await;
    assert_eq!(
        response.error.unwrap().code,
        muxlane_core::protocol::error_codes::PATH_NOT_FOUND
    );
    assert!(!missing.exists());
    assert!(state.read().await.projects.is_empty());

    let nested = dir.path().join("rpc/new/nested");
    let response = rpc_call(
        &sock,
        methods::PROJECT_ADD,
        serde_json::json!({"path": nested, "create_if_missing": true}),
    )
    .await;
    let project: Project = serde_json::from_value(response.result.unwrap()).unwrap();
    assert_eq!(project.path, nested.canonicalize().unwrap());
    assert!(nested.is_dir());

    let file = dir.path().join("rpc-file");
    std::fs::write(&file, "not a directory").unwrap();
    let response = rpc_call(
        &sock,
        methods::PROJECT_ADD,
        serde_json::json!({"path": file, "create_if_missing": true}),
    )
    .await;
    assert_eq!(
        response.error.unwrap().code,
        muxlane_core::protocol::error_codes::NOT_A_DIRECTORY
    );

    let duplicate = rpc_call(
        &sock,
        methods::PROJECT_ADD,
        serde_json::json!({"path": nested, "create_if_missing": false}),
    )
    .await;
    let duplicate: Project = serde_json::from_value(duplicate.result.unwrap()).unwrap();
    assert_eq!(project.id, duplicate.id);
    assert_eq!(state.read().await.projects.len(), 1);
}

#[tokio::test]
async fn remote_term_input_reaches_pty_and_project_add_validates_path() {
    let (server, sock, state, _dirty, dir) = spawn_server().await;
    let agent = launch_agent(&server, &state, "read line; echo REMOTE:$line; sleep 5").await;
    let mut conn = BufReader::new(UnixStream::connect(&sock).await.unwrap());
    write_frame(
        &mut conn,
        &Request {
            id: 50,
            method: methods::TERM_INPUT.into(),
            params: serde_json::to_value(muxlane_core::protocol::TermInputParams {
                agent: agent.clone(),
                data_b64: muxlane_core::protocol::b64_encode(b"hello\n"),
            })
            .unwrap(),
        },
    )
    .await
    .unwrap();
    let _: Response = serde_json::from_value(read_frame(&mut conn).await.unwrap()).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let replay = server.session(&agent).await.unwrap().replay_snapshot();
    assert!(String::from_utf8_lossy(&replay).contains("REMOTE:hello"));

    let project_dir = dir.path().join("remote-project");
    std::fs::create_dir_all(&project_dir).unwrap();
    write_frame(
        &mut conn,
        &Request {
            id: 51,
            method: methods::PROJECT_ADD.into(),
            params: serde_json::json!({ "path": project_dir }),
        },
    )
    .await
    .unwrap();
    // 提交型输入会先广播 agent.status_changed（working）事件，读响应时跳过事件帧。
    let response: Response = loop {
        let value = read_frame(&mut conn).await.unwrap();
        if value.get("event").is_some() {
            continue;
        }
        break serde_json::from_value(value).unwrap();
    };
    let project: Project = serde_json::from_value(response.result.unwrap()).unwrap();
    assert_eq!(project.name, "remote-project");
    assert!(state
        .read()
        .await
        .projects
        .iter()
        .any(|item| item.id == project.id));
}

#[tokio::test]
async fn project_delete_destroys_scoped_sessions_and_state() {
    let (server, sock, state, _dirty, _dir) = spawn_server().await;
    let agent = launch_agent(&server, &state, "sleep 30").await;
    let mut conn = BufReader::new(UnixStream::connect(&sock).await.unwrap());
    write_frame(
        &mut conn,
        &Request {
            id: 40,
            method: methods::PROJECT_DELETE.into(),
            params: serde_json::json!({ "project": "p_test" }),
        },
    )
    .await
    .unwrap();
    let response: Response = serde_json::from_value(read_frame(&mut conn).await.unwrap()).unwrap();
    let result: muxlane_core::protocol::DeleteScopeResult =
        serde_json::from_value(response.result.unwrap()).unwrap();
    assert_eq!(result.destroyed_agents, vec![agent]);
    assert!(state.read().await.projects.is_empty());
    assert!(state.read().await.agents.is_empty());
    assert_eq!(server.session_count().await, 0);
}

#[tokio::test]
async fn state_list_returns_snapshot() {
    let (srv, sock, state, _dirty, _dir) = spawn_server().await;
    let _agent = launch_agent(&srv, &state, "sleep 5").await;

    let conn = BufReader::new(UnixStream::connect(&sock).await.unwrap());
    let mut conn = conn;
    write_frame(
        &mut conn,
        &Request {
            id: 1,
            method: methods::STATE_LIST.into(),
            params: serde_json::json!(null),
        },
    )
    .await
    .unwrap();
    let resp: Response = serde_json::from_value(read_frame(&mut conn).await.unwrap()).unwrap();
    assert_eq!(resp.id, 1);
    let snap: muxlane_core::model::Snapshot = serde_json::from_value(resp.result.unwrap()).unwrap();
    assert_eq!(snap.machine.as_ref().unwrap().name, "test-box");
    assert_eq!(snap.projects.len(), 1);
    assert_eq!(snap.agents.len(), 1);
}

#[tokio::test]
async fn term_subscribe_replay_and_incremental() {
    let (srv, sock, state, _dirty, _dir) = spawn_server().await;
    let agent = launch_agent(
        &srv,
        &state,
        "echo first-out; sleep 1; echo second-out; sleep 5",
    )
    .await;

    // 等 first-out 出现（回放缓冲里）
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let conn = BufReader::new(UnixStream::connect(&sock).await.unwrap());
    let mut conn = conn;
    write_frame(
        &mut conn,
        &Request {
            id: 2,
            method: methods::TERM_SUBSCRIBE.into(),
            params: serde_json::json!({ "agent": agent }),
        },
    )
    .await
    .unwrap();
    let resp: Response = serde_json::from_value(read_frame(&mut conn).await.unwrap()).unwrap();
    let result: muxlane_core::protocol::TermSubscribeResult =
        serde_json::from_value(resp.result.unwrap()).unwrap();
    let replay = muxlane_core::protocol::b64_decode(&result.replay_b64).unwrap();
    assert!(
        replay.windows(9).any(|w| w == b"first-out"),
        "replay covers history"
    );

    // 增量：等 second-out 通过订阅流到达
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut got_incremental = false;
    while std::time::Instant::now() < deadline {
        let frame =
            tokio::time::timeout(std::time::Duration::from_millis(500), read_frame(&mut conn))
                .await;
        match frame {
            Ok(Ok(v)) => {
                if v["event"] == events::TERM_DATA {
                    let data = muxlane_core::protocol::b64_decode(
                        v["params"]["data_b64"].as_str().unwrap(),
                    )
                    .unwrap();
                    if data.windows(10).any(|w| w == b"second-out") {
                        got_incremental = true;
                        break;
                    }
                }
            }
            Ok(Err(e)) => {
                eprintln!("DEBUG frame err: {e:?}");
                break;
            }
            Err(_) => {
                continue;
            } // timeout：继续等下一个事件
        }
    }
    assert!(
        got_incremental,
        "incremental term.data delivers later output"
    );
}

#[tokio::test]
async fn events_subscribe_receives_status_change() {
    let (srv, sock, state, _dirty, _dir) = spawn_server().await;
    let agent = launch_agent(&srv, &state, "sleep 10").await;

    let conn = BufReader::new(UnixStream::connect(&sock).await.unwrap());
    let mut conn = conn;
    write_frame(
        &mut conn,
        &Request {
            id: 3,
            method: methods::EVENTS_SUBSCRIBE.into(),
            params: serde_json::json!(null),
        },
    )
    .await
    .unwrap();
    let resp: Response = serde_json::from_value(read_frame(&mut conn).await.unwrap()).unwrap();
    assert!(resp.result.is_some());

    // 从另一连接上报 hook → 本连接应收到状态事件
    let token = srv.hook_token(&agent);
    let conn2 = BufReader::new(UnixStream::connect(&sock).await.unwrap());
    let mut conn2 = conn2;
    write_frame(&mut conn2, &Request {
        id: 9,
        method: methods::AGENT_REPORT.into(),
        params: serde_json::json!({ "token": token, "agent": agent, "event": "done", "message": "unit-test done" }),
    }).await.unwrap();
    let resp2: Response = serde_json::from_value(read_frame(&mut conn2).await.unwrap()).unwrap();
    assert!(resp2.result.is_some());

    // conn 收事件：先是订阅确认时的 state.changed，然后 agent.status_changed
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut got = None;
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), read_frame(&mut conn))
            .await
        {
            Ok(Ok(v)) => {
                if v["event"] == events::AGENT_STATUS {
                    got = Some(v);
                    break;
                }
            }
            Ok(Err(e)) => {
                eprintln!("DEBUG frame err: {e:?}");
                break;
            }
            Err(_) => {
                continue;
            }
        }
    }
    let ev: EventMsg =
        serde_json::from_value(got.expect("should receive agent.status_changed")).unwrap();
    assert_eq!(ev.params["to"], "done");
}

#[tokio::test]
async fn unknown_agent_subscribe_errors() {
    let (_srv, sock, _state, _dirty, _dir) = spawn_server().await;
    let conn = BufReader::new(UnixStream::connect(&sock).await.unwrap());
    let mut conn = conn;
    write_frame(
        &mut conn,
        &Request {
            id: 4,
            method: methods::TERM_SUBSCRIBE.into(),
            params: serde_json::json!({ "agent": "shell_nope" }),
        },
    )
    .await
    .unwrap();
    let resp: Response = serde_json::from_value(read_frame(&mut conn).await.unwrap()).unwrap();
    assert_eq!(resp.error.unwrap().code, "no_such_agent");
}

#[tokio::test]
async fn hook_rejects_invalid_token() {
    let (srv, sock, state, _dirty, _dir) = spawn_server().await;
    let agent = launch_agent(&srv, &state, "sleep 10").await;
    let mut conn = BufReader::new(UnixStream::connect(&sock).await.unwrap());
    write_frame(
        &mut conn,
        &Request {
            id: 10,
            method: methods::AGENT_REPORT.into(),
            params: serde_json::json!({
                "token": "v1:9999999999:invalid",
                "agent": agent,
                "event": "done"
            }),
        },
    )
    .await
    .unwrap();
    let resp: Response = serde_json::from_value(read_frame(&mut conn).await.unwrap()).unwrap();
    assert_eq!(resp.error.unwrap().code, "unauthorized");
}

#[tokio::test]
async fn node_hook_script_reports_with_env_identity() {
    if std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    let (srv, sock, state, _dirty, dir) = spawn_server().await;
    let agent = launch_agent(&srv, &state, "sleep 10").await;
    let script = dir.path().join("report.mjs");
    std::fs::write(&script, muxlane_core::hook::REPORT_SCRIPT).unwrap();
    let token = srv.hook_token(&agent);
    let mut child = tokio::process::Command::new("node")
        .arg(&script)
        .arg("done")
        .env("MUXLANE_SOCKET", &sock)
        .env("MUXLANE_AGENT_ID", &agent)
        .env("MUXLANE_HOOK_TOKEN", token)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use tokio::io::AsyncWriteExt;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"{}\n")
        .await
        .unwrap();
    child.stdin.take();
    let status = tokio::time::timeout(std::time::Duration::from_secs(3), child.wait())
        .await
        .expect("hook exits")
        .unwrap();
    assert!(status.success());
    for _ in 0..20 {
        if state.read().await.agents[0].status == AgentStatus::Done {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("node hook did not update agent state");
}

#[tokio::test]
async fn partial_request_survives_interleaved_event() {
    use tokio::io::AsyncWriteExt;
    let (srv, sock, _state, dirty, _dir) = spawn_server().await;
    let mut conn = BufReader::new(UnixStream::connect(&sock).await.unwrap());
    // 开事件订阅。
    write_frame(
        &mut conn,
        &Request {
            id: 1,
            method: methods::EVENTS_SUBSCRIBE.into(),
            params: serde_json::json!(null),
        },
    )
    .await
    .unwrap();
    let _resp: Response = serde_json::from_value(read_frame(&mut conn).await.unwrap()).unwrap();
    // 消耗初始 state.changed。
    let _ = read_frame(&mut conn).await.unwrap();

    // 分两段写 state.list，在中间触发 dirty event；半帧必须留在 reader task 的缓冲中。
    conn.write_all(b"{\"id\":42,\"method\":\"state.")
        .await
        .unwrap();
    dirty.bump();
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    conn.write_all(b"list\"}\n").await.unwrap();

    let mut got_response = false;
    for _ in 0..4 {
        let v = tokio::time::timeout(std::time::Duration::from_secs(1), read_frame(&mut conn))
            .await
            .unwrap()
            .unwrap();
        if v.get("id").and_then(|v| v.as_u64()) == Some(42) {
            let resp: Response = serde_json::from_value(v).unwrap();
            assert!(resp.result.is_some());
            got_response = true;
            break;
        }
    }
    assert!(
        got_response,
        "partial state.list response arrives after interleaved event"
    );
    drop(srv);
}

#[tokio::test]
async fn second_server_cannot_unlink_active_socket() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("same.sock");
    let mk_state = || {
        Arc::new(RwLock::new(ServerState::new(MachineInfo {
            machine_id: muxlane_core::model::new_id("m"),
            name: "box".into(),
            os: "linux".into(),
            version: "0.1".into(),
        })))
    };
    let first = MuxlaneServer::new(sock.clone(), mk_state(), DirtyFlag::new());
    let _first_task = AbortOnDrop(tokio::spawn(async move {
        let _ = first.serve().await;
    }));
    for _ in 0..50 {
        if UnixStream::connect(&sock).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let second = MuxlaneServer::new(sock.clone(), mk_state(), DirtyFlag::new());
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), second.serve())
        .await
        .expect("second returns promptly");
    assert!(result.is_err());
    // 第一台仍可连接。
    assert!(UnixStream::connect(&sock).await.is_ok());
}

#[tokio::test]
async fn connection_drop_cleans_terminal_subscriptions() {
    let (srv, sock, state, _dirty, _dir) = spawn_server().await;
    let agent = launch_agent(&srv, &state, "sleep 10").await;
    let mut conn = BufReader::new(UnixStream::connect(&sock).await.unwrap());
    write_frame(
        &mut conn,
        &Request {
            id: 77,
            method: methods::TERM_SUBSCRIBE.into(),
            params: serde_json::json!({"agent": agent}),
        },
    )
    .await
    .unwrap();
    let _ = read_frame(&mut conn).await.unwrap();
    assert_eq!(srv.subscription_count().await, 1);
    drop(conn);
    for _ in 0..20 {
        if srv.subscription_count().await == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("subscription leaked after connection close");
}

#[tokio::test]
async fn agent_delete_kills_and_cleans_state() {
    let (srv, sock, state, _dirty, _dir) = spawn_server().await;
    let agent = launch_agent(&srv, &state, "sleep 60").await;
    let mut events = BufReader::new(UnixStream::connect(&sock).await.unwrap());
    write_frame(
        &mut events,
        &Request {
            id: 1,
            method: methods::EVENTS_SUBSCRIBE.into(),
            params: serde_json::json!(null),
        },
    )
    .await
    .unwrap();
    let _ = read_frame(&mut events).await.unwrap(); // response
    let _ = read_frame(&mut events).await.unwrap(); // initial dirty

    let mut rpc = BufReader::new(UnixStream::connect(&sock).await.unwrap());
    write_frame(
        &mut rpc,
        &Request {
            id: 2,
            method: methods::AGENT_DELETE.into(),
            params: serde_json::json!({"agent": agent}),
        },
    )
    .await
    .unwrap();
    let resp: Response = serde_json::from_value(read_frame(&mut rpc).await.unwrap()).unwrap();
    assert!(resp.result.is_some());
    assert!(srv.session(&agent).await.is_none());
    assert!(!state.read().await.agents.iter().any(|a| a.id == agent));
    assert!(!state.read().await.projects[0].agents.contains(&agent));
    let ev = tokio::time::timeout(std::time::Duration::from_secs(1), read_frame(&mut events))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ev["event"], muxlane_core::protocol::events::STATE_CHANGED);
}

/// 无 Tokio 上下文的极简 block_on（noop waker + 有界轮询）。
/// 模拟 app 侧 GPUI background executor 线程：future 可正常驱动，
/// 但线程本地没有 Tokio runtime（Handle::current() 会 panic）。
fn block_on_detached<F: std::future::Future>(fut: F) -> F::Output {
    use std::task::{Context, Poll, Waker};
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut cx = Context::from_waker(Waker::noop());
    let mut fut = std::pin::pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "detached future timed out"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
}

fn tmux_session_exists(name: &str) -> bool {
    std::process::Command::new("tmux")
        .args(["-L", "muxlane", "has-session", "-t", &format!("={name}")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

struct KillTmuxSessionOnDrop(String);

impl Drop for KillTmuxSessionOnDrop {
    fn drop(&mut self) {
        let _ = std::process::Command::new("tmux")
            .args(["-L", "muxlane", "kill-session", "-t"])
            .arg(format!("={}", self.0))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// 回归：app 在 GPUI background executor（非 Tokio 线程）里直接调用
/// spawn_agent / delete_agent，内部必须用 server 持有的 runtime handle，
/// 不能 tokio::task::spawn_blocking（依赖 Handle::current()，进程直接 abort）。
#[test]
fn spawn_and_delete_agent_from_non_tokio_thread() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (server, state) = rt.block_on(async {
        let state = Arc::new(RwLock::new(ServerState::new(MachineInfo {
            machine_id: "m_test".into(),
            name: "test-box".into(),
            os: "linux".into(),
            version: "0.1.0".into(),
        })));
        state.write().await.add_project(Project {
            id: "p_test".into(),
            name: "testproj".into(),
            path: "/tmp/testproj".into(),
            branch: None,
            agents: vec![],
        });
        (
            MuxlaneServer::new(
                dir.path().join("muxlane.sock"),
                Arc::clone(&state),
                DirtyFlag::new(),
            ),
            state,
        )
    });

    // 在全新的 std 线程（保证无线程本地 runtime）上驱动 spawn_agent
    let srv = Arc::clone(&server);
    let spawned = std::thread::spawn(move || {
        block_on_detached(async move {
            srv.spawn_agent(muxlane_core::protocol::AgentSpawnParams {
                project: "p_test".into(),
                agent_type: Some(AgentType::Shell),
                program: Some("bash".into()),
                args: Some(vec!["-c".into(), "sleep 30".into()]),
                env: None,
                preset_name: None,
            })
            .await
        })
    })
    .join()
    .expect("caller thread panicked")
    .expect("spawn_agent failed");
    let agent = spawned.id.clone();
    let tmux_name = spawned.tmux_session.clone().expect("tmux session name");
    assert!(!tmux_name.is_empty());
    let _tmux_cleanup = KillTmuxSessionOnDrop(tmux_name.clone());

    // `new-session -A` 客户端是异步建立的：等 tmux server 真正建好 session，
    // 否则 kill-session 会抢在创建之前（真实 app 里用户不可能这么快删除）。
    rt.block_on(async {
        for _ in 0..100 {
            if tmux_session_exists(&tmux_name) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("tmux session never appeared");
    });

    // 同一线程模型驱动 delete_agent（destroy_sessions 内部也走 spawn_blocking）
    let srv = Arc::clone(&server);
    let id = agent.clone();
    let deleted =
        std::thread::spawn(move || block_on_detached(async move { srv.delete_agent(&id).await }))
            .join()
            .expect("caller thread panicked")
            .expect("delete_agent failed");
    assert!(deleted.failed_agents.is_empty());

    rt.block_on(async {
        assert!(server.session(&agent).await.is_none());
        assert!(!state.read().await.agents.iter().any(|a| a.id == agent));
    });
    assert!(!tmux_session_exists(&tmux_name));
}
