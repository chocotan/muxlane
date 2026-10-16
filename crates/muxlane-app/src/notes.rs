//! Per-machine and per-project Markdown notes. Files stay on this computer.

use crate::theme::Theme;
use crate::ui_scale::px as ui_px;
use gpui::{div, prelude::*, rgba, FontWeight, SharedString};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::path::{Path, PathBuf};

pub(crate) fn sanitize(id: &str) -> String {
    let cleaned: String = id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "local".into()
    } else {
        cleaned
    }
}

pub(crate) fn scope_key(kind: &str, id: &str) -> String {
    format!("{kind}:{}", sanitize(id))
}

pub(crate) fn notes_dir(data_dir: &Path, scope: &str) -> PathBuf {
    data_dir.join("notes").join(sanitize(scope))
}

pub(crate) fn list_notes(dir: &Path) -> Vec<PathBuf> {
    let mut notes = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect::<Vec<_>>();
    notes.sort();
    notes
}

pub(crate) fn create_note(dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.md", muxlane_core::model::new_id("note")));
    std::fs::write(&path, "")?;
    Ok(path)
}

pub(crate) fn note_title(path: &Path, fallback: &str) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().trim_start_matches('#').trim())
        .filter(|line| !line.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

#[derive(Clone)]
struct Run {
    text: String,
    bold: bool,
    code: bool,
}

pub(crate) fn render_markdown(source: &str, theme: Theme) -> gpui::AnyElement {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let mut blocks: Vec<gpui::AnyElement> = Vec::new();
    let mut runs = Vec::new();
    let mut chunk = String::new();
    let mut bold = false;
    let code = false;
    let mut heading = false;
    let mut in_code_block = false;
    let mut code_block = String::new();
    let mut ordered = false;
    let mut list_index = 0u64;
    let flush_chunk = |runs: &mut Vec<Run>, chunk: &mut String, bold: bool, code: bool| {
        if !chunk.is_empty() {
            runs.push(Run {
                text: std::mem::take(chunk),
                bold,
                code,
            });
        }
    };
    for event in Parser::new_ext(source, options) {
        match event {
            Event::Start(Tag::Heading { .. }) => heading = true,
            Event::End(TagEnd::Heading(_)) => {
                flush_chunk(&mut runs, &mut chunk, bold, code);
                blocks.push(render_runs(&runs, theme, true));
                runs.clear();
                heading = false;
            }
            Event::Start(Tag::Paragraph) | Event::Start(Tag::BlockQuote(_)) => {}
            Event::End(TagEnd::Paragraph) | Event::End(TagEnd::BlockQuote(_)) => {
                flush_chunk(&mut runs, &mut chunk, bold, code);
                if !runs.is_empty() {
                    blocks.push(render_runs(&runs, theme, heading));
                    runs.clear();
                }
            }
            Event::Start(Tag::List(start)) => {
                ordered = start.is_some();
                list_index = start.unwrap_or(1);
            }
            Event::Start(Tag::Item) => {
                if ordered {
                    chunk.push_str(&format!("{list_index}. "));
                    list_index += 1;
                } else {
                    chunk.push_str("- ");
                }
            }
            Event::End(TagEnd::Item) => {
                flush_chunk(&mut runs, &mut chunk, bold, code);
                if !runs.is_empty() {
                    blocks.push(render_runs(&runs, theme, false));
                    runs.clear();
                }
            }
            Event::Start(Tag::CodeBlock(_)) => in_code_block = true,
            Event::End(TagEnd::CodeBlock) => {
                blocks.push(
                    div()
                        .w_full()
                        .p_2()
                        .bg(rgba(theme.bg2))
                        .text_size(ui_px(12.))
                        .text_color(rgba(theme.fg0))
                        .child(std::mem::take(&mut code_block))
                        .into_any_element(),
                );
                in_code_block = false;
            }
            Event::Start(Tag::Strong) => {
                flush_chunk(&mut runs, &mut chunk, bold, code);
                bold = true;
            }
            Event::End(TagEnd::Strong) => {
                flush_chunk(&mut runs, &mut chunk, bold, code);
                bold = false;
            }
            Event::Code(text) => {
                flush_chunk(&mut runs, &mut chunk, bold, code);
                runs.push(Run {
                    text: text.to_string(),
                    bold: false,
                    code: true,
                });
            }
            Event::Text(text) if in_code_block => code_block.push_str(&text),
            Event::Text(text) => chunk.push_str(&text),
            Event::SoftBreak | Event::HardBreak if in_code_block => code_block.push('\n'),
            Event::SoftBreak | Event::HardBreak => chunk.push('\n'),
            Event::Rule => {
                blocks.push(div().h(ui_px(1.)).bg(rgba(theme.line)).into_any_element());
            }
            Event::Html(_) | Event::InlineHtml(_) | Event::Start(Tag::Image { .. }) => {}
            _ => {}
        }
    }
    flush_chunk(&mut runs, &mut chunk, bold, code);
    if !runs.is_empty() {
        blocks.push(render_runs(&runs, theme, heading));
    }
    let mut column = div().flex().flex_col().gap_2().size_full().p_4();
    for block in blocks {
        column = column.child(block);
    }
    column.into_any_element()
}

fn render_runs(runs: &[Run], theme: Theme, heading: bool) -> gpui::AnyElement {
    let mut row = div()
        .flex()
        .flex_wrap()
        .text_size(ui_px(if heading { 16. } else { 13. }))
        .text_color(rgba(theme.fg0));
    if heading {
        row = row.font_weight(FontWeight::BOLD);
    }
    for run in runs {
        let mut piece = div().child(SharedString::from(run.text.clone()));
        if run.bold {
            piece = piece.font_weight(FontWeight::BOLD);
        }
        if run.code {
            piece = piece.bg(rgba(theme.bg2)).px_1();
        }
        row = row.child(piece);
    }
    row.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_names_stay_filesystem_safe() {
        assert_eq!(sanitize("local"), "local");
        assert_eq!(sanitize("host/name"), "host_name");
        assert_eq!(sanitize(""), "local");
        assert_eq!(scope_key("project", "a/b"), "project:a_b");
    }

    #[test]
    fn title_uses_the_first_nonempty_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.md");
        std::fs::write(&path, "\n# 部署备忘\n- 9843\n").unwrap();
        assert_eq!(note_title(&path, "未命名"), "部署备忘");
        std::fs::write(&path, "\n\n").unwrap();
        assert_eq!(note_title(&path, "未命名"), "未命名");
    }
}
