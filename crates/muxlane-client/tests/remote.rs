//! 客户端集成测试：对端起真 muxlane-server，RemoteHost 完整走 连接→快照→事件→term 流
use muxlane_client::{ClientEvent, HostCfg, RemoteHost, Target};
use muxlane_core::model::{AgentInstance, AgentStatus, AgentType, MachineInfo, Project};
use muxlane_server::{DirtyFlag, MuxlaneServer, ServerState};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

async fn spawn_peer(
    dir: &tempfile::TempDir,
) -> (Arc<MuxlaneServer>, Arc<RwLock<ServerState>>, String) {
    let sock = dir.path().join("peer.sock");
    let state = Arc::new(RwLock::new(ServerState::new(MachineInfo {
        machine_id: "m_peer".into(),
        name: "peer-box".into(),
        os: "linux".into(),
        version: "0.1.0".into(),
    })));
    let dirty = DirtyFlag::new();
    let server = MuxlaneServer::new(sock.clone(), Arc::clone(&state), dirty);

    let srv = Arc::clone(&server);
    tokio::spawn(async move { srv.serve().await.unwrap() });

    // 等 socket 就绪
    for _ in 0..50 {
        if tokio::net::UnixStream::connect(&sock).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    (server, state, sock.display().to_string())
}

#[tokio::test]
async fn remote_host_connects_and_receives_events() {
    let dir = tempfile::tempdir().unwrap();
    let (server, state, sock) = spawn_peer(&dir).await;

    // 对端注册一个项目+agent
    let agent_id = muxlane_core::model::new_id("shell");
    let inst = AgentInstance {
        id: agent_id.clone(),
        project: "p1".into(),
        agent_type: AgentType::Shell,
        title: "peer bash".into(),
        status: AgentStatus::Idle,
        status_since: muxlane_core::model::now_secs(),
        seen: true,
        tmux_session: None,
    };
    let proj = Project {
        id: "p1".into(),
        name: "peer-proj".into(),
        path: "/tmp/peer-proj".into(),
        branch: Some("main".into()),
        agents: vec![agent_id.clone()],
    };
    state.write().await.add_agent(proj, inst);

    // 客户端连上去
    let (tx, mut rx) = mpsc::channel(64);
    let host = RemoteHost::new(
        HostCfg {
            name: "peer".into(),
            target: Target::Socket(sock),
            auth: muxlane_client::SshAuth::SshConfig,
            retry_base_ms: 200,
        },
        tx,
    );
    tokio::spawn(std::clone::Clone::clone(&host).run_loop());

    // 收到 Online 状态，且快照含 peer-box
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut got_online = false;
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await {
            Ok(Some(ClientEvent::StateChanged {
                state: RemoteState::Online(snap),
                ..
            })) => {
                assert_eq!(snap.machine.as_ref().unwrap().name, "peer-box");
                assert_eq!(snap.agents.len(), 1);
                got_online = true;
                break;
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    assert!(got_online, "should receive Online snapshot");
    assert!(host.supports(muxlane_core::protocol::features::PROJECT_CREATE));

    let created_dir = dir.path().join("created/remotely");
    let created = host
        .add_project(&created_dir.display().to_string(), true)
        .await
        .unwrap();
    assert_eq!(created.path, created_dir.canonicalize().unwrap());
    assert!(created_dir.is_dir());

    // 远端新增会话只发 state.changed；RemoteHost 必须重拉 state.list。
    let second = muxlane_core::model::AgentInstance {
        id: "shell_second".into(),
        project: "p1".into(),
        agent_type: AgentType::Shell,
        title: "second".into(),
        status: AgentStatus::Idle,
        status_since: muxlane_core::model::now_secs(),
        seen: true,
        tmux_session: None,
    };
    let second_id = second.id.clone();
    state.write().await.agents.push(second);
    state.write().await.projects[0]
        .agents
        .push(second_id.clone());
    server
        .add_project(muxlane_core::protocol::ProjectAddParams {
            path: dir.path().display().to_string(),
            name: Some("refresh-trigger".into()),
            create_if_missing: false,
        })
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut refreshed = false;
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await {
            Ok(Some(ClientEvent::StateChanged {
                state: RemoteState::Online(snap),
                ..
            })) if snap.agents.len() == 2 => {
                refreshed = true;
                break;
            }
            Ok(_) => continue,
            Err(_) => continue,
        }
    }
    assert!(refreshed, "state.changed refreshes remote snapshot");

    // 对端 hook 上报 → 客户端应收到 StatusChanged
    state
        .write()
        .await
        .report_hook(&muxlane_core::protocol::AgentReportParams {
            token: String::new(),
            agent: agent_id.clone(),
            event: "done".into(),
            message: Some("remote done".into()),
        })
        .await;

    let mut got_status = None;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await {
            Ok(Some(ClientEvent::StatusChanged { to, .. })) => {
                got_status = Some(to);
                break;
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    assert_eq!(
        got_status,
        Some(AgentStatus::Done),
        "client receives remote status change"
    );

    drop(server);
    let _ = agent_id;
}

use muxlane_client::RemoteState;

#[test]
fn parses_socket_and_ssh_targets() {
    assert!(matches!(
        muxlane_client::parse_target("/tmp/muxlane.sock"),
        muxlane_client::Target::Socket(p) if p == "/tmp/muxlane.sock"
    ));
    assert!(matches!(
        muxlane_client::parse_target("nuc"),
        muxlane_client::Target::Ssh { host, socket } if host == "nuc" && socket.is_empty()
    ));
    assert!(matches!(
        muxlane_client::parse_target("choco@192.168.1.20"),
        muxlane_client::Target::Ssh { host, socket } if host == "choco@192.168.1.20" && socket.is_empty()
    ));
    assert!(matches!(
        muxlane_client::parse_target("choco@nuc:/home/choco/.local/share/muxlane/muxlane.sock"),
        muxlane_client::Target::Ssh { host, socket }
            if host == "choco@nuc" && socket.ends_with("/muxlane.sock")
    ));
}

/// 真机集成检查：MUXLANE_TEST_SSH_HOST=host cargo test -- --ignored
/// 验证 upload_bytes 落盘远端且内容一致（粘贴图片链路的核心）。
#[tokio::test]
#[ignore = "set MUXLANE_TEST_SSH_HOST to run against a real host"]
async fn upload_bytes_lands_file_on_remote_host() {
    use muxlane_client::HostCfg;
    use sha2::Digest;
    let host = std::env::var("MUXLANE_TEST_SSH_HOST").expect("MUXLANE_TEST_SSH_HOST not set");
    let cfg = HostCfg {
        name: host.clone(),
        target: muxlane_client::Target::Ssh {
            host,
            socket: String::new(),
        },
        auth: muxlane_client::SshAuth::default(),
        retry_base_ms: 200,
    };
    let payload: Vec<u8> = (0u8..=255).cycle().take(1024 * 33).collect();
    let path = format!("/tmp/muxlane-upload-test-{}", std::process::id());
    muxlane_client::upload_bytes_to_remote(&cfg, &payload, &path)
        .await
        .unwrap();
    let output = std::process::Command::new("ssh")
        .args(["-o", "BatchMode=yes", &format!("{}", cfg_target(&cfg))])
        .arg("sha256sum")
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let remote_hash = stdout.split_whitespace().next().unwrap();
    let mut hasher = sha2::Sha256::new();
    hasher.update(&payload);
    let local_hash = format!("{:x}", hasher.finalize());
    assert_eq!(remote_hash, local_hash, "remote file content mismatch");
    std::process::Command::new("ssh")
        .args(["-o", "BatchMode=yes", &cfg_target(&cfg)])
        .arg("rm")
        .arg(&path)
        .output()
        .unwrap();
}

fn cfg_target(cfg: &HostCfg) -> String {
    match &cfg.target {
        muxlane_client::Target::Ssh { host, .. } => host.clone(),
        muxlane_client::Target::Socket(path) => path.clone(),
    }
}
