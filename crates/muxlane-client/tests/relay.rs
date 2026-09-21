//! 桌面经自建中继连接远端：pair_relay 拿 token，RemoteHost 走 Target::Relay。
use muxlane_client::{ClientEvent, HostCfg, RemoteHost, RemoteState, Target};
use muxlane_core::model::{AgentInstance, AgentStatus, AgentType, MachineInfo, Project};
use muxlane_server::{DirtyFlag, MuxlaneServer, ServerState};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

async fn spawn_relay() -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let bind = format!("127.0.0.1:{port}");
    let task = tokio::spawn({
        let bind = bind.clone();
        async move { muxlane_relay::Relay::new().serve(&bind).await.unwrap() }
    });
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(&bind).await.is_ok() {
            return (format!("ws://{bind}"), task);
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("relay never became ready");
}

async fn spawn_server_with_relay(
    dir: &tempfile::TempDir,
    relay_url: &str,
) -> (Arc<MuxlaneServer>, Arc<RwLock<ServerState>>) {
    let sock = dir.path().join("peer.sock");
    let state = Arc::new(RwLock::new(ServerState::new(MachineInfo {
        machine_id: "m_relay_peer".into(),
        name: "relay-box".into(),
        os: "linux".into(),
        version: "0.1.0".into(),
    })));
    let server = MuxlaneServer::new(sock.clone(), Arc::clone(&state), DirtyFlag::new());
    let srv = Arc::clone(&server);
    tokio::spawn(async move { srv.serve().await.unwrap() });
    for _ in 0..50 {
        if tokio::net::UnixStream::connect(&sock).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    server
        .relay_handle()
        .set_url(Some(relay_url.to_string()))
        .await;
    server.start_relay(relay_url.to_string(), None);
    (server, state)
}

#[tokio::test]
async fn desktop_pairs_and_connects_over_relay() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("muxlane_server=debug,muxlane_relay=debug,muxlane_client=debug")
        .try_init();
    let (relay_url, _relay_task) = spawn_relay().await;
    let dir = tempfile::tempdir().unwrap();
    let (server, state) = spawn_server_with_relay(&dir, &relay_url).await;

    // 对端注册一个项目+agent
    let agent_id = muxlane_core::model::new_id("shell");
    state.write().await.add_agent(
        Project {
            id: "p1".into(),
            name: "relay-proj".into(),
            path: "/tmp/relay-proj".into(),
            branch: None,
            agents: vec![agent_id.clone()],
        },
        AgentInstance {
            id: agent_id.clone(),
            project: "p1".into(),
            agent_type: AgentType::Shell,
            title: "relay bash".into(),
            status: AgentStatus::Idle,
            status_since: muxlane_core::model::now_secs(),
            seen: true,
            tmux_session: None,
        },
    );

    // 桌面「配对」：拿码 → pair_relay 得 token + machine
    // host WS 注册码到 relay 是异步的，先等它就绪
    let offer = server.begin_pair_offer().await.unwrap();
    let mut paired = None;
    for _ in 0..20 {
        match muxlane_client::pair_relay(&relay_url, &offer.code, "desktop-test").await {
            Ok(pair) => {
                paired = Some(pair);
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
        }
    }
    let (token, machine) = paired.expect("pair_relay should succeed once host registered the code");
    assert_eq!(machine.machine_id, "m_relay_peer");
    assert_eq!(machine.name, "relay-box");

    // RemoteHost 走中继：连上、hello、快照
    let (tx, mut rx) = mpsc::channel(64);
    let host = RemoteHost::new(
        HostCfg {
            name: machine.name.clone(),
            target: Target::Relay {
                url: relay_url.clone(),
                host_id: machine.machine_id.clone(),
            },
            auth: muxlane_client::SshAuth::RelayToken {
                token: Some(token.clone()),
            },
            retry_base_ms: 200,
        },
        tx,
    );
    let snapshot = host.fetch_snapshot().await.unwrap();
    assert_eq!(snapshot.machine.unwrap().name, "relay-box");
    assert_eq!(snapshot.agents.len(), 1);
    assert_eq!(snapshot.agents[0].title, "relay bash");

    // 事件通道（events.subscribe → state.changed → 快照）也走中继
    tokio::spawn(std::clone::Clone::clone(&host).run_loop());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    let mut got_online = false;
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await {
            Ok(Some(ClientEvent::StateChanged {
                state: RemoteState::Online(snap),
                ..
            })) => {
                assert_eq!(snap.machine.as_ref().unwrap().machine_id, "m_relay_peer");
                got_online = true;
                break;
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    assert!(got_online, "should receive Online snapshot over relay");
    host.stop();

    // token 重连（杀连接后第二次 pair.begin）
    let mut conn = muxlane_client::connect_relay(&relay_url, &machine.machine_id, &token)
        .await
        .unwrap();
    let hello = conn
        .call(
            muxlane_core::protocol::methods::SYSTEM_HELLO,
            serde_json::json!({}),
        )
        .await
        .unwrap();
    let hello: muxlane_core::protocol::HelloResult = serde_json::from_value(hello).unwrap();
    assert!(hello
        .features
        .iter()
        .any(|feature| feature == muxlane_core::protocol::features::PAIR));

    // 错误码被拒
    assert!(
        muxlane_client::pair_relay(&relay_url, "00000000", "desktop-test")
            .await
            .is_err()
    );
}
