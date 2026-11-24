//! Theme-aware syntax highlighting and unified line-level diff engine for Native Chat cards.
use crate::ui::ChatPalette;
use gpui::{rgba, HighlightStyle};
use similar::{ChangeTag, TextDiff};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiffLineKind {
    Context,
    Add,
    Del,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
}

/// Compute unified diff hunks with context lines bounded so large file edits stay fast.
pub(crate) fn compute_diff_lines(old: &str, new: &str, max_lines: usize) -> (Vec<DiffLine>, bool) {
    let diff = TextDiff::from_lines(old, new);
    let mut out = Vec::new();
    let mut truncated = false;
    let ops = diff.ops();
    for group in diff.grouped_ops(3) {
        if !out.is_empty() && ops.len() > 1 {
            // Separator marker between disjoint hunks
            out.push(DiffLine {
                kind: DiffLineKind::Context,
                old_line: None,
                new_line: None,
                text: "···".to_string(),
            });
        }
        for op in group {
            for change in diff.iter_changes(&op) {
                if out.len() >= max_lines {
                    truncated = true;
                    return (out, truncated);
                }
                let kind = match change.tag() {
                    ChangeTag::Equal => DiffLineKind::Context,
                    ChangeTag::Delete => DiffLineKind::Del,
                    ChangeTag::Insert => DiffLineKind::Add,
                };
                let text = change
                    .to_string_lossy()
                    .trim_end_matches(['\r', '\n'])
                    .to_string();
                out.push(DiffLine {
                    kind,
                    old_line: change.old_index().map(|i| i + 1),
                    new_line: change.new_index().map(|i| i + 1),
                    text,
                });
            }
        }
    }
    if out.is_empty() && (old != new) {
        for (idx, line) in new.lines().take(max_lines).enumerate() {
            out.push(DiffLine {
                kind: DiffLineKind::Add,
                old_line: None,
                new_line: Some(idx + 1),
                text: line.to_string(),
            });
        }
        truncated = new.lines().count() > max_lines;
    }
    (out, truncated)
}

/// Lightweight deterministic tokenizer emitting sorted non-overlapping byte-range highlights.
pub(crate) fn highlight_code(
    code: &str,
    lang: &str,
    palette: ChatPalette,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let lang = lang.trim().to_ascii_lowercase();
    let hash_comments = matches!(
        lang.as_str(),
        "py" | "python" | "sh" | "bash" | "zsh" | "fish" | "toml" | "yaml" | "yml" | "ruby" | "rb"
    );
    let slash_comments = !hash_comments
        && !matches!(
            lang.as_str(),
            "json" | "md" | "markdown" | "txt" | "text" | ""
        );
    let is_code_lang = !matches!(lang.as_str(), "txt" | "text" | "md" | "markdown");
    if !is_code_lang || code.is_empty() {
        return Vec::new();
    }

    let bytes = code.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut runs = Vec::new();

    while i < len {
        // Line comments (`//` or `#`)
        if (slash_comments && i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/')
            || (hash_comments && bytes[i] == b'#')
        {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            runs.push((
                start..i,
                HighlightStyle {
                    color: Some(rgba(palette.syn_comment).into()),
                    ..Default::default()
                },
            ));
            continue;
        }

        // Block comments `/* ... */`
        if slash_comments && i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(len);
            while !code.is_char_boundary(i) && i < len {
                i += 1;
            }
            runs.push((
                start..i,
                HighlightStyle {
                    color: Some(rgba(palette.syn_comment).into()),
                    ..Default::default()
                },
            ));
            continue;
        }

        // Quoted strings (`"`, `'`, `` ` ``)
        if bytes[i] == b'"' || bytes[i] == b'\'' || bytes[i] == b'`' {
            let quote = bytes[i];
            let start = i;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                if bytes[i] == b'\n' && quote != b'`' {
                    break;
                }
                i += 1;
            }
            while !code.is_char_boundary(i) && i < len {
                i += 1;
            }
            runs.push((
                start..i,
                HighlightStyle {
                    color: Some(rgba(palette.syn_string).into()),
                    ..Default::default()
                },
            ));
            continue;
        }

        // Numbers
        if bytes[i].is_ascii_digit()
            && (i == 0 || (!bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_'))
        {
            let start = i;
            while i < len
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
            {
                i += 1;
            }
            runs.push((
                start..i,
                HighlightStyle {
                    color: Some(rgba(palette.syn_number).into()),
                    ..Default::default()
                },
            ));
            continue;
        }

        // Identifiers / Keywords / Types
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &code[start..i];
            if is_keyword(word) {
                runs.push((
                    start..i,
                    HighlightStyle {
                        color: Some(rgba(palette.syn_keyword).into()),
                        font_weight: Some(gpui::FontWeight::MEDIUM),
                        ..Default::default()
                    },
                ));
            } else if word.chars().next().is_some_and(char::is_uppercase) && word.len() > 1 {
                runs.push((
                    start..i,
                    HighlightStyle {
                        color: Some(rgba(palette.syn_type).into()),
                        ..Default::default()
                    },
                ));
            }
            continue;
        }

        // Advance safely by UTF-8 char
        let ch_len = code[i..].chars().next().map_or(1, char::len_utf8);
        i += ch_len;
    }

    runs
}

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "fn" | "let"
            | "mut"
            | "pub"
            | "struct"
            | "enum"
            | "impl"
            | "trait"
            | "for"
            | "while"
            | "loop"
            | "if"
            | "else"
            | "match"
            | "return"
            | "async"
            | "await"
            | "use"
            | "mod"
            | "const"
            | "type"
            | "where"
            | "move"
            | "ref"
            | "true"
            | "false"
            | "None"
            | "Some"
            | "Ok"
            | "Err"
            | "function"
            | "export"
            | "import"
            | "from"
            | "class"
            | "interface"
            | "def"
            | "lambda"
            | "with"
            | "as"
            | "try"
            | "except"
            | "catch"
            | "finally"
            | "throw"
            | "in"
            | "self"
            | "Self"
            | "super"
            | "fun"
            | "val"
            | "var"
            | "package"
            | "null"
            | "undefined"
            | "break"
            | "continue"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeMode;

    #[test]
    fn highlight_produces_sorted_non_overlapping_char_boundary_ranges() {
        let palette = ChatPalette::for_mode(ThemeMode::Dark);
        let sample = "pub fn hello(x: String) -> usize {\n    // comment 中文\n    let s = \"hi 中文\";\n    return 42;\n}";
        let runs = highlight_code(sample, "rust", palette);
        assert!(!runs.is_empty());
        let mut prev_end = 0;
        for (range, _) in &runs {
            assert!(range.start >= prev_end);
            assert!(sample.is_char_boundary(range.start));
            assert!(sample.is_char_boundary(range.end));
            prev_end = range.end;
        }
    }

    #[test]
    fn diff_computes_line_numbers_and_tags() {
        let (lines, truncated) = compute_diff_lines("a\nb\nc\n", "a\nb2\nc\nd\n", 100);
        assert!(!truncated);
        assert!(lines
            .iter()
            .any(|l| l.kind == DiffLineKind::Del && l.text == "b"));
        assert!(lines
            .iter()
            .any(|l| l.kind == DiffLineKind::Add && l.text == "b2"));
        assert!(lines
            .iter()
            .any(|l| l.kind == DiffLineKind::Add && l.text == "d"));
    }
}
