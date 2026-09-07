use super::*;

#[tokio::test]
async fn worker_completion_and_hook_isolation() {
    // Give only this test subprocess a fake parent-terminal identity, never mutate global env.
    const CHILD: &str = "MUXLANE_COMPLETION_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "completion_tests::worker_completion_and_hook_isolation",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .envs(TERMINAL_HOOK_ENV.map(|key| (key, "fixture-parent-value")))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated completion fixture failed: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("completion.py");
    std::fs::write(&script, r#"import json, os, sys
for key in ["MUXLANE_AGENT_ID", "MUXLANE_SOCKET", "MUXLANE_HOOK_TOKEN", "TMUX", "TMUX_PANE"]:
    assert not os.environ.get(key), "terminal hook identity leaked"
assert os.environ["FIXTURE_KEEP"] == "preserved"
pending = None
cancel_reason = None
for line in sys.stdin:
    message = json.loads(line)
    rid = message.get("id")
    method = message.get("method")
    if method == "session/cancel":
        assert pending is not None
        print(json.dumps({"jsonrpc": "2.0", "id": pending, "result": {"stopReason": cancel_reason}}), flush=True)
        pending = None
        continue
    if rid is None:
        continue
    if method == "initialize":
        result = {"protocolVersion": 1, "agentCapabilities": {"loadSession": True}}
    elif method == "session/new":
        result = {"sessionId": "protocol-id-not-a-ui-id"}
    elif method == "session/load":
        result = {}
    elif method == "session/prompt":
        text = message["params"]["prompt"][0]["text"]
        if text.startswith("cancel-"):
            pending = rid
            cancel_reason = "end_turn" if text == "cancel-ignoring-adapter" else "cancelled"
            continue
        if text == "error":
            print(json.dumps({"jsonrpc": "2.0", "id": rid, "error": {"code": -32603, "message": "fixture failure"}}), flush=True)
            continue
        result = {"stopReason": text}
    else:
        result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": result}), flush=True)
"#).unwrap();
    let profile = Profile::Custom(AgentDefinition {
        id: "completion-fixture".into(),
        label: "Completion fixture".into(),
        command: "python3".into(),
        args: vec![script.to_string_lossy().into_owned()],
        // Explicit config must not be able to reintroduce the parent's hook identity either.
        env: [
            ("FIXTURE_KEEP".into(), "preserved".into()),
            ("MUXLANE_AGENT_ID".into(), "fixture-override".into()),
        ]
        .into(),
    });
    for restored in [false, true] {
        let ActiveSession { handle, mut events } = spawn_on(
            &tokio::runtime::Handle::current(),
            profile.clone(),
            directory.path(),
            restored.then(|| "protocol-id-not-a-ui-id".into()),
        );
        let mut ready = false;
        loop {
            match next(&mut events).await {
                Event::Ready {
                    restored: actual, ..
                } => {
                    assert_eq!(actual, restored);
                    ready = true;
                }
                Event::Turn(TurnState::Idle) => {
                    assert!(ready);
                    break;
                }
                Event::PromptCompleted { .. } => {
                    panic!("initialization/restore must not complete a prompt")
                }
                Event::Error(error) => panic!("fixture initialization failed: {}", error.message),
                _ => {}
            }
        }
        for reason in [
            "end_turn",
            "error",
            "cancel-request",
            "cancel-ignoring-adapter",
            "cancelled",
            "refusal",
            "max_tokens",
            "max_turn_requests",
            "end_turn",
        ] {
            let id = handle
                .submit_submission(PromptSubmission::new(PromptPayload::new(reason)))
                .unwrap();
            let mut accepted = false;
            let mut completed = 0;
            let mut errors = 0;
            loop {
                match next(&mut events).await {
                    Event::PromptAccepted { id: actual } => {
                        assert_eq!(actual, id);
                        assert!(!accepted);
                        accepted = true;
                        if reason.starts_with("cancel-") {
                            handle.cancel().unwrap();
                        }
                    }
                    Event::PromptCompleted { id: actual } => {
                        assert!(accepted);
                        assert_eq!(actual, id);
                        completed += 1;
                    }
                    Event::Error(_) => errors += 1,
                    Event::Turn(TurnState::Idle) => break,
                    _ => {}
                }
            }
            assert!(accepted);
            assert_eq!(completed, usize::from(reason == "end_turn"), "{reason}");
            assert_eq!(errors, usize::from(reason == "error"), "{reason}");
        }
        handle.shutdown();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while let Some(event) = events.recv().await {
                assert!(
                    !matches!(event, Event::PromptCompleted { .. }),
                    "duplicate completion"
                );
            }
        })
        .await
        .unwrap();
    }
    let host = HostServices::new(directory.path()).unwrap();
    let created = host.create_terminal(v1::CreateTerminalRequest::new("fixture", "python3")
        .args(vec!["-c".into(), "import os; assert all(not os.environ.get(k) for k in ['MUXLANE_AGENT_ID', 'MUXLANE_SOCKET', 'MUXLANE_HOOK_TOKEN', 'TMUX', 'TMUX_PANE']); assert os.environ['FIXTURE_KEEP'] == 'preserved'; print('isolated')".into()])
        .env(TERMINAL_HOOK_ENV.map(|key| v1::EnvVariable::new(key, "fixture-request-value"))
            .into_iter().chain([v1::EnvVariable::new("FIXTURE_KEEP", "preserved")]).collect()))
        .await.unwrap();
    let id = created.terminal_id;
    host.wait_for_terminal_exit(v1::WaitForTerminalExitRequest::new("fixture", id.clone()))
        .await
        .unwrap();
    host.release_terminal(v1::ReleaseTerminalRequest::new("fixture", id.clone()))
        .await
        .unwrap();
    let result = host
        .query_terminal(v1::TerminalOutputRequest::new("fixture", id))
        .await;
    assert!(
        matches!(result.state, TerminalOutputState::Ready(snapshot) if snapshot.output == "isolated\n" && snapshot.exit_code == Some(0))
    );
    host.shutdown().await;
    for key in TERMINAL_HOOK_ENV {
        assert!(
            std::env::var(key).is_ok_and(|value| value == "fixture-parent-value"),
            "parent environment changed"
        );
    }
}

async fn next(events: &mut mpsc::UnboundedReceiver<Event>) -> Event {
    tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap()
}
