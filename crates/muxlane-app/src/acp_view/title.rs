use muxlane_acp::{MessageRole, ThreadItem, ThreadSnapshot};

pub(super) fn has_user_prompt(snapshot: &ThreadSnapshot) -> bool {
    snapshot.items.iter().any(|item| match item {
        ThreadItem::Message(message) => message.role == MessageRole::User,
        ThreadItem::Content(content) => content.role == MessageRole::User,
        _ => false,
    })
}

/// Server titles win; local titles only replace placeholders on the first queued prompt.
pub(super) fn updated_title(
    current: &str,
    default: &str,
    first_prompt: Option<&str>,
    server_title: Option<&str>,
) -> Option<String> {
    if let Some(title) = server_title.filter(|title| !title.trim().is_empty()) {
        return Some(title.to_string());
    }
    if !current.trim().is_empty() && current != default && current != "ACP" {
        return None;
    }
    let prompt = first_prompt?;
    let title = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return None;
    }
    const MAX_CHARS: usize = 48;
    if title.chars().count() > MAX_CHARS {
        Some(format!(
            "{}...",
            title.chars().take(MAX_CHARS - 3).collect::<String>()
        ))
    } else {
        Some(title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_prompt_replaces_only_default_titles() {
        for current in ["", "  ", "ACP", "Claude UI"] {
            assert_eq!(
                updated_title(current, "Claude UI", Some("Fix startup"), None).as_deref(),
                Some("Fix startup")
            );
        }
        assert_eq!(
            updated_title("Recovered title", "Claude UI", Some("New question"), None),
            None
        );
        assert_eq!(updated_title("Claude UI", "Claude UI", None, None), None);
    }

    #[test]
    fn prompt_title_normalizes_whitespace_and_ignores_blank_input() {
        assert_eq!(
            updated_title("ACP", "Claude UI", Some(" \n\t\u{3000}"), None),
            None
        );
        assert_eq!(
            updated_title(
                "ACP",
                "Claude UI",
                Some("  fix\n\t startup \u{3000} now  "),
                None
            )
            .as_deref(),
            Some("fix startup now")
        );
    }

    #[test]
    fn long_multibyte_prompt_is_truncated_at_character_boundaries() {
        let prompt = "修复启动界面".repeat(20);
        let title = updated_title("Codex UI", "Codex UI", Some(&prompt), None).unwrap();
        assert_eq!(title.chars().count(), 48);
        assert_eq!(
            title,
            format!("{}...", prompt.chars().take(45).collect::<String>())
        );
        assert_eq!(
            updated_title("ACP", "Codex UI", Some("修复界面 𝄞"), None).as_deref(),
            Some("修复界面 𝄞")
        );
    }

    #[test]
    fn server_title_overrides_fallback_and_restored_titles() {
        for current in ["Claude UI", "Fix startup", "Recovered title"] {
            assert_eq!(
                updated_title(
                    current,
                    "Claude UI",
                    Some("Local prompt"),
                    Some("Server title")
                )
                .as_deref(),
                Some("Server title")
            );
            assert_eq!(
                updated_title(current, "Claude UI", None, Some(" \n ")),
                None
            );
        }
    }

    #[test]
    fn restored_user_history_consumes_first_prompt() {
        let mut snapshot = ThreadSnapshot::default();
        assert!(!has_user_prompt(&snapshot));
        snapshot
            .items
            .push(ThreadItem::Message(muxlane_acp::Message {
                protocol_id: None,
                id: "assistant".into(),
                role: MessageRole::Assistant,
                text: "Welcome".into(),
            }));
        assert!(!has_user_prompt(&snapshot));
        snapshot
            .items
            .push(ThreadItem::Message(muxlane_acp::Message {
                protocol_id: None,
                id: "user".into(),
                role: MessageRole::User,
                text: "First prompt".into(),
            }));
        assert!(has_user_prompt(&snapshot));
    }
}
