use muxlane_client::{
    stream_term, ClientEvent, Connection, HostCfg, RemoteHost, RemoteState, SshAuth, Target,
    TermUpdate,
};
use muxlane_core::protocol::b64_encode;
use muxlane_core::protocol::{
    read_frame, write_frame, EventMsg, Request, Response, TermSubscribeResult,
};
use std::sync::{Arc, Mutex};
use tokio::net::UnixListener;

#[tokio::test]
async fn connection_routes_events_and_keeps_waiting_for_matching_response() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("routing.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (r, mut w) = stream.into_split();
        let mut r = tokio::io::BufReader::new(r);
        let req: Request = serde_json::from_value(read_frame(&mut r).await.unwrap()).unwrap();
        write_frame(
            &mut w,
            &EventMsg::new("test.event", serde_json::json!({"n": 1})),
        )
        .await
        .unwrap();
        write_frame(
            &mut w,
            &Response::ok(req.id + 99, serde_json::json!("stale")),
        )
        .await
        .unwrap();
        write_frame(&mut w, &Response::ok(req.id, serde_json::json!("ok")))
            .await
            .unwrap();
    });

    let mut conn = Connection::new(tokio::net::UnixStream::connect(&sock).await.unwrap());
    let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
    conn.set_event_handler(events_tx);
    assert_eq!(
        conn.call("test.call", serde_json::Value::Null)
            .await
            .unwrap(),
        "ok"
    );
    assert_eq!(events_rx.recv().await.unwrap().event, "test.event");
    server.await.unwrap();
}

#[tokio::test]
async fn remote_host_reuses_rpc_connection() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("reuse.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (r, mut w) = stream.into_split();
        let mut r = tokio::io::BufReader::new(r);
        for _ in 0..2 {
            let req: Request = serde_json::from_value(read_frame(&mut r).await.unwrap()).unwrap();
            write_frame(
                &mut w,
                &Response::ok(
                    req.id,
                    serde_json::to_value(muxlane_core::model::Snapshot::default()).unwrap(),
                ),
            )
            .await
            .unwrap();
        }
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
    let (events_tx, _events_rx) = tokio::sync::mpsc::channel(1);
    let host = RemoteHost::new(
        HostCfg {
            name: "test".into(),
            target: Target::Socket(sock.to_string_lossy().into_owned()),
            auth: SshAuth::default(),
            retry_base_ms: 200,
        },
        events_tx,
    );
    host.fetch_snapshot().await.unwrap();
    host.fetch_snapshot().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn refresh_snapshot_publishes_the_fetched_list() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("refresh.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let mut expected = muxlane_core::model::Snapshot::default();
    expected.projects.push(muxlane_core::model::Project {
        id: "project-1".into(),
        name: "Project 1".into(),
        path: "/tmp/project-1".into(),
        branch: None,
        agents: Vec::new(),
    });
    let response_snapshot = expected.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (r, mut w) = stream.into_split();
        let mut r = tokio::io::BufReader::new(r);
        let req: Request = serde_json::from_value(read_frame(&mut r).await.unwrap()).unwrap();
        write_frame(
            &mut w,
            &Response::ok(req.id, serde_json::to_value(response_snapshot).unwrap()),
        )
        .await
        .unwrap();
    });
    let (events_tx, mut events_rx) = tokio::sync::mpsc::channel(1);
    let host = RemoteHost::new(
        HostCfg {
            name: "test".into(),
            target: Target::Socket(sock.to_string_lossy().into_owned()),
            auth: SshAuth::default(),
            retry_base_ms: 200,
        },
        events_tx,
    );

    host.refresh_snapshot().await.unwrap();

    let event = events_rx.recv().await.unwrap();
    assert!(matches!(
        event,
        ClientEvent::StateChanged {
            host,
            state: RemoteState::Online(snapshot),
        } if host == "test" && snapshot == expected
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn stream_term_handles_replay_resync_and_exit() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("fake.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (r, mut w) = stream.into_split();
        let mut r = tokio::io::BufReader::new(r);
        let req: Request = serde_json::from_value(read_frame(&mut r).await.unwrap()).unwrap();
        let first = b64_encode(b"FIRST");
        write_frame(
            &mut w,
            &Response::ok(
                req.id,
                serde_json::to_value(TermSubscribeResult {
                    sub_id: "s1".into(),
                    replay_b64: first,
                })
                .unwrap(),
            ),
        )
        .await
        .unwrap();
        let second = b64_encode(b"SECOND");
        write_frame(
            &mut w,
            &EventMsg::new(
                muxlane_core::protocol::events::TERM_RESYNC,
                serde_json::json!({"agent":"a1","replay_b64":second}),
            ),
        )
        .await
        .unwrap();
        write_frame(
            &mut w,
            &EventMsg::new(
                muxlane_core::protocol::events::TERM_EXIT,
                serde_json::json!({"agent":"a1"}),
            ),
        )
        .await
        .unwrap();
    });
    let updates = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let out = Arc::clone(&updates);
    stream_term(sock.to_str().unwrap(), &"a1".into(), move |update| {
        if let TermUpdate::Resync(bytes) = update {
            out.lock().unwrap().push(bytes);
        }
    })
    .await
    .unwrap();
    server.await.unwrap();
    assert_eq!(
        *updates.lock().unwrap(),
        vec![b"FIRST".to_vec(), b"SECOND".to_vec()]
    );
}
