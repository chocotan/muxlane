use crate::icons::*;
use muxlane_acp::{Tool, ToolKind, ToolState, TerminalCommand, TerminalSnapshot};
use similar::{ChangeTag, TextDiff};
use std::{collections::HashMap, ops::Range, sync::Arc, time::Duration};

pub(super) fn kind_label(kind: &ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "Read", ToolKind::Edit => "Edit", ToolKind::Delete => "Delete",
        ToolKind::Move => "Move", ToolKind::Search => "Search", ToolKind::Execute => "Execute",
        ToolKind::Think => "Think", ToolKind::Fetch => "Fetch", ToolKind::SwitchMode => "Switch mode",
        ToolKind::Other => "Tool",
    }
}

pub(super) fn kind_icon(kind: &ToolKind) -> &'static [u8] {
    match kind {
        ToolKind::Read | ToolKind::Edit | ToolKind::Search => FOLDER_ICON,
        ToolKind::Delete => CLOSE_ICON,
        ToolKind::Move | ToolKind::Fetch => ARROW_DOWN_ICON,
        ToolKind::Execute => SEND_ICON,
        ToolKind::SwitchMode => SETTINGS_ICON,
        ToolKind::Think | ToolKind::Other => MORE_ICON,
    }
}

pub(super) fn tool_title(tool: &Tool) -> String {
    let first = tool.title.lines().next().unwrap_or_default();
    if first.trim().is_empty() {
        kind_label(&tool.kind).into()
    } else {
        format!("{first}{}", if tool.title.lines().count() > 1 { " ..." } else { "" })
    }
}

pub(super) fn state_label(state: &ToolState) -> String {
    match state {
        ToolState::Pending => "Pending".into(), ToolState::Running => "Running".into(),
        ToolState::Completed => "Completed".into(), ToolState::Failed => "Failed".into(),
        ToolState::Rejected => "Rejected".into(), ToolState::Canceled => "Canceled".into(),
        ToolState::Unknown(value) => format!("Unknown ({value})"),
    }
}

// Display argv as structured data, not as a reconstructed executable shell command.
pub(super) fn command_text(command: &TerminalCommand) -> String {
    format!("Command: {}\nArgs: {}\nCwd: {}", command.command,
        serde_json::to_string(&command.args).unwrap_or_default(), command.cwd)
}

pub(super) fn terminal_status(snapshot: &TerminalSnapshot) -> String {
    let mut parts = Vec::new();
    if let Some(code) = snapshot.exit_code { parts.push(format!("Exit {code}")); }
    if snapshot.truncated { parts.push("Output truncated".into()); }
    parts.join(" | ")
}

pub(super) fn bounded_text(text: &str, limit: usize) -> String {
    if text.len() <= limit { return text.into(); }
    let mut end = limit;
    while !text.is_char_boundary(end) { end -= 1; }
    format!("{}\n[Preview truncated; copy to inspect the complete original]", &text[..end])
}

pub(super) const MAX_DIFF_BYTES: usize = 64 * 1024;
const MAX_DIFF_LINES: usize = 2000;
const MAX_DIFF_ROWS: usize = 600;

#[derive(Debug)]
pub(super) struct DiffRow {
    pub range: Range<usize>,
    pub tag: Option<ChangeTag>,
}

#[derive(Debug, Default)]
pub(super) struct DiffPreview {
    pub text: String,
    pub rows: Vec<DiffRow>,
    pub added: usize,
    pub removed: usize,
    pub limited: bool,
}

impl DiffPreview {
    fn push(&mut self, text: &str, tag: Option<ChangeTag>) {
        let start = self.text.len();
        self.text.push_str(text);
        self.rows.push(DiffRow { range: start..self.text.len(), tag });
    }

    fn fallback(reason: &str) -> Self {
        let mut preview = Self { limited: true, ..Self::default() };
        preview.push(reason, None);
        preview
    }
}

pub(super) fn diff_preview(old: &str, new: &str) -> DiffPreview {
    if old.len().saturating_add(new.len()) > MAX_DIFF_BYTES
        || old.lines().count().saturating_add(new.lines().count()) > MAX_DIFF_LINES {
        return DiffPreview::fallback("Diff preview omitted: input exceeds 64 KiB or 2000 lines. Copy original/new below to inspect the full content.");
    }
    let diff = TextDiff::configure().timeout(Duration::from_millis(10)).diff_lines(old, new);
    let mut result = DiffPreview::default();
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => result.added += 1,
            ChangeTag::Delete => result.removed += 1,
            ChangeTag::Equal => {},
        }
    }
    for group in diff.grouped_ops(3) {
        let first = &group[0];
        let last = &group[group.len() - 1];
        let old_range = first.old_range().start..last.old_range().end;
        let new_range = first.new_range().start..last.new_range().end;
        result.push(&format!("@@ -{},{} +{},{} @@\n", old_range.start + usize::from(!old_range.is_empty()), old_range.len(), new_range.start + usize::from(!new_range.is_empty()), new_range.len()), None);
        for op in group {
            for change in diff.iter_changes(&op) {
                if result.rows.len() >= MAX_DIFF_ROWS {
                    result.limited = true;
                    result.push("[Diff preview truncated; copy original/new below for full content]", None);
                    return result;
                }
                let old_number = change.old_index().map(|n| (n + 1).to_string()).unwrap_or_default();
                let new_number = change.new_index().map(|n| (n + 1).to_string()).unwrap_or_default();
                let sign = match change.tag() { ChangeTag::Equal => ' ', ChangeTag::Delete => '-', ChangeTag::Insert => '+' };
                result.push(&format!("{old_number:>4} {new_number:>4} {sign}{}{}", change.value(), if change.missing_newline() { "\n" } else { "" }), Some(change.tag()));
                if change.missing_newline() {
                    result.push("\\ No newline at end of file\n", None);
                }
            }
        }
    }
    if result.rows.is_empty() { result.push("No changes", None); }
    result
}

#[derive(Default)]
pub(super) struct DiffCache(HashMap<String, (String, String, Arc<DiffPreview>)>);

impl DiffCache {
    pub fn get(&mut self, scope: &str, old: &str, new: &str) -> Arc<DiffPreview> {
        if let Some((cached_old, cached_new, preview)) = self.0.get(scope) {
            if cached_old == old && cached_new == new { return preview.clone(); }
        }
        if self.0.len() >= 16 { self.0.clear(); }
        let preview = Arc::new(diff_preview(old, new));
        // Large sources are already owned by the tool; don't duplicate them in this cache.
        if old.len().saturating_add(new.len()) <= MAX_DIFF_BYTES {
            self.0.insert(scope.into(), (old.into(), new.into(), preview.clone()));
        }
        preview
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn true_diff_has_context_numbers_counts_and_unicode() {
        let diff = diff_preview("same\nold\n尾\n", "same\nnew\n尾\n");
        assert_eq!((diff.added, diff.removed), (1, 1));
        assert!(diff.text.contains("   1    1  same"));
        assert!(diff.text.contains("   2      -old"));
        assert!(diff.text.contains("        2 +new"));
        assert!(diff.text.contains("尾"));
        assert!(diff.text.starts_with("@@ -1,3 +1,3 @@"));
    }
    #[test]
    fn diff_no_newline_and_empty_creation() {
        let diff = diff_preview("a", "a\n");
        assert!(diff.text.contains("No newline at end of file"));
        assert_eq!((diff.added, diff.removed), (1, 1));
        assert!(diff_preview("", "new\n").text.starts_with("@@ -0,0 +1,1 @@"));
        assert_eq!(diff_preview("same", "same").text, "No changes");
    }
    #[test]
    fn diff_bounds_and_cache() {
        assert!(diff_preview("", &"x".repeat(MAX_DIFF_BYTES + 1)).limited);
        assert!(diff_preview("", &"x\n".repeat(2001)).limited);
        assert!(diff_preview("", &"x\n".repeat(800)).limited);
        let mut cache = DiffCache::default();
        let a = cache.get("x", "a", "b");
        assert!(Arc::ptr_eq(&a, &cache.get("x", "a", "b")));
        assert!(!Arc::ptr_eq(&a, &cache.get("x", "a", "c")));
    }
    #[test]
    fn terminal_metadata_and_both_statuses_are_preserved() {
        let command = TerminalCommand { command: "sh".into(), args: vec!["-c".into(), "echo 'a b'".into()], cwd: "/tmp/project".into() };
        assert!(command_text(&command).contains("[\"-c\",\"echo 'a b'\"]"));
        let snapshot = TerminalSnapshot { id: "t".into(), output: "  a\n\tb".into(), truncated: true, exit_code: Some(7), command: Some(command) };
        assert_eq!(terminal_status(&snapshot), "Exit 7 | Output truncated");
        let legacy: TerminalSnapshot = serde_json::from_str(r#"{"id":"t","output":"","truncated":false,"exit_code":null}"#).unwrap();
        assert!(legacy.command.is_none());
        assert!(bounded_text("中文", 4).starts_with("中\n"));
    }
    #[test]
    fn all_kinds_and_states_have_explicit_labels() {
        for kind in [ToolKind::Read, ToolKind::Edit, ToolKind::Delete, ToolKind::Move, ToolKind::Search, ToolKind::Execute, ToolKind::Think, ToolKind::Fetch, ToolKind::SwitchMode, ToolKind::Other] {
            assert!(!kind_label(&kind).is_empty()); assert!(!kind_icon(&kind).is_empty());
        }
        let labels: std::collections::HashSet<_> = [ToolState::Pending, ToolState::Running, ToolState::Completed, ToolState::Failed, ToolState::Rejected, ToolState::Canceled].iter().map(state_label).collect();
        assert_eq!(labels.len(), 6);
        let tool = Tool { id: "x".into(), title: "Keep protocol title\nsecond line".into(), kind: ToolKind::Other, state: ToolState::Pending, content: vec![], locations: vec![], raw_input: Some(serde_json::json!("{incomplete")), raw_output: None, subagent_session_id: None };
        assert_eq!(tool_title(&tool), "Keep protocol title ...");
    }
}
