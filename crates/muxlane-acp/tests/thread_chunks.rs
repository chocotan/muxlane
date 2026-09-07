use muxlane_acp::{
    Event, MessageRole, ThreadChange, ThreadDelta, ThreadItem, ThreadReducer, ThreadSnapshot,
    ToolContent,
};

fn message(id: Option<&str>, role: MessageRole, text: &str) -> ThreadDelta {
    ThreadDelta::MessageChunk {
        id: id.map(str::to_string),
        role,
        text: text.into(),
    }
}

fn tool(id: &str) -> ThreadDelta {
    ThreadDelta::ToolUpsert {
        id: id.into(),
        title: None,
        kind: None,
        state: None,
        content: None,
        locations: None,
        raw_input: None,
        raw_output: None,
        subagent_session_id: None,
    }
}

fn message_at(reducer: &ThreadReducer, index: usize) -> &muxlane_acp::Message {
    let ThreadItem::Message(message) = &reducer.snapshot().items[index] else {
        panic!("expected message")
    };
    message
}

#[test]
fn opencode_shared_thought_message_id_forms_one_complete_response() {
    let mut reducer = ThreadReducer::new();
    for (kind, text) in [
        ("agent_thought_chunk", "Thinking"),
        ("agent_message_chunk", "你好"),
        ("agent_message_chunk", "，"),
        ("agent_message_chunk", "世界 😀。"),
    ] {
        // Exercise wire projection too: the provider reuses a messageId for thought and text.
        let update = serde_json::from_value(serde_json::json!({
            "sessionUpdate":kind, "messageId":"msg_fixture", "content":{"type":"text", "text":text}
        }))
        .unwrap();
        for event in muxlane_acp::project_update(update) {
            let Event::Delta(delta) = event else {
                panic!("expected chunk")
            };
            let change = reducer.apply(delta);
            assert_eq!(
                change,
                ThreadChange::Item {
                    index: if kind == "agent_thought_chunk" { 0 } else { 1 },
                    appended: text == "Thinking" || text == "你好",
                }
            );
        }
    }
    assert_eq!(reducer.snapshot().items.len(), 2);
    assert_eq!(message_at(&reducer, 1).text, "你好，世界 😀。");
    assert_eq!(
        message_at(&reducer, 1).protocol_id.as_deref(),
        Some("msg_fixture")
    );
    let thought_id = match &reducer.snapshot().items[0] {
        ThreadItem::Thought(t) => &t.id,
        _ => unreachable!(),
    };
    assert_ne!(thought_id, &message_at(&reducer, 1).id);
}

#[test]
fn anonymous_unicode_and_late_protocol_id_preserve_identity_and_index() {
    let mut reducer = ThreadReducer::new();
    for (index, (id, text)) in [
        (None, "你"),
        (None, "好"),
        (Some("a"), " 😀"),
        (None, "，"),
        (Some("a"), "世界。"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            reducer.apply(message(id, MessageRole::Assistant, text)),
            ThreadChange::Item {
                index: 0,
                appended: index == 0
            }
        );
        assert_eq!(message_at(&reducer, 0).id, "local-message-1");
    }
    assert_eq!(message_at(&reducer, 0).text, "你好 😀，世界。");
    assert_eq!(message_at(&reducer, 0).protocol_id.as_deref(), Some("a"));
    assert_eq!(
        reducer.apply(message(Some("b"), MessageRole::Assistant, "New message")),
        ThreadChange::Item {
            index: 1,
            appended: true
        }
    );
    let encoded = serde_json::to_string(reducer.snapshot()).unwrap();
    let restored = ThreadReducer::from_snapshot(serde_json::from_str(&encoded).unwrap());
    assert_eq!(restored.snapshot(), reducer.snapshot());
}

#[test]
fn late_thought_id_keeps_identity_and_distinct_explicit_id_starts_item() {
    let mut reducer = ThreadReducer::new();
    for (id, text) in [(None, "思"), (Some("thought"), "考"), (None, "中")] {
        reducer.apply(ThreadDelta::ThoughtChunk {
            id: id.map(str::to_string),
            text: text.into(),
        });
        assert!(
            matches!(&reducer.snapshot().items[0], ThreadItem::Thought(t) if t.id == "local-thought-1")
        );
    }
    assert_eq!(reducer.snapshot().items.len(), 1);
    assert_eq!(
        reducer.apply(ThreadDelta::ThoughtChunk {
            id: Some("other".into()),
            text: "next".into()
        }),
        ThreadChange::Item {
            index: 1,
            appended: true
        }
    );
}

#[test]
fn user_assistant_tool_image_and_turn_boundaries_do_not_merge() {
    let mut reducer = ThreadReducer::new();
    for (index, role) in [
        MessageRole::User,
        MessageRole::Assistant,
        MessageRole::User,
        MessageRole::Assistant,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            reducer.apply(message(Some("shared"), role, "text")),
            ThreadChange::Item {
                index,
                appended: true
            }
        );
        assert_eq!(message_at(&reducer, index).text, "text");
        assert_eq!(message_at(&reducer, index).role, role);
    }
    assert_eq!(
        reducer.apply(tool("shared")),
        ThreadChange::Item {
            index: 4,
            appended: true
        }
    );
    assert_eq!(
        reducer.apply(tool("shared")),
        ThreadChange::Item {
            index: 4,
            appended: false
        }
    );
    assert_eq!(
        reducer.apply(message(
            Some("shared"),
            MessageRole::Assistant,
            "after tool"
        )),
        ThreadChange::Item {
            index: 5,
            appended: true
        }
    );
    reducer.apply(ThreadDelta::ContentBlock {
        id: Some("shared".into()),
        role: MessageRole::Assistant,
        content: ToolContent::Image {
            data: String::new(),
            mime_type: "image/png".into(),
        },
    });
    assert_eq!(
        reducer.apply(message(None, MessageRole::Assistant, "after image")),
        ThreadChange::Item {
            index: 7,
            appended: true
        }
    );
    assert_eq!(message_at(&reducer, 5).text, "after tool");
    assert_ne!(message_at(&reducer, 1).id, message_at(&reducer, 3).id);
}

#[test]
fn legacy_snapshot_repairs_only_adjacent_same_id_same_role_chunks() {
    // Real observed shape: one user, one thought, thirteen text fragments sharing
    // the thought's ID. All source text and IDs have been replaced with synthetic data.
    let snapshot: ThreadSnapshot =
        serde_json::from_str(include_str!("fixtures/opencode_shared_id.json")).unwrap();
    assert_eq!(snapshot.items.len(), 15);
    let mut reducer = ThreadReducer::from_snapshot(snapshot);
    assert_eq!(reducer.snapshot().items.len(), 3);
    assert_eq!(message_at(&reducer, 2).text, "你好。我是示例助手，请提问。");
    let stable_id = message_at(&reducer, 2).id.clone();
    assert_eq!(
        reducer.apply(message(
            Some("msg_fixture"),
            MessageRole::Assistant,
            "你好。我是示例助手，请提问。"
        )),
        ThreadChange::None
    );
    assert_eq!(message_at(&reducer, 2).id, stable_id);
    let snapshot = reducer.into_snapshot();
    assert_eq!(
        ThreadReducer::from_snapshot(snapshot.clone()).into_snapshot(),
        snapshot
    );
}

#[test]
fn restored_protocol_alias_replays_into_original_item_not_last_item() {
    let mut reducer = ThreadReducer::new();
    reducer.apply(message(None, MessageRole::Assistant, "hello"));
    reducer.apply(message(Some("a"), MessageRole::Assistant, " world"));
    reducer.apply(message(Some("b"), MessageRole::Assistant, "second"));
    let mut reducer = ThreadReducer::from_snapshot(reducer.into_snapshot());
    assert_eq!(
        reducer.apply(message(Some("a"), MessageRole::Assistant, "changed")),
        ThreadChange::Item {
            index: 0,
            appended: false
        }
    );
    assert_eq!(message_at(&reducer, 0).id, "local-message-1");
    assert_eq!(message_at(&reducer, 0).text, "changed");
    assert_eq!(message_at(&reducer, 1).text, "second");
}
