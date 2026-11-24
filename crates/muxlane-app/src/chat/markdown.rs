//! 流式 Markdown 解析与 GPUI 零圆角富文本渲染器（基于 pulldown-cmark 0.13）。
use crate::chat::syntax::highlight_code;
use crate::ui::{
    square_button, ButtonVariant, ChatPalette, CHAT_CODE_PX, CHAT_MICRO_PX, CHAT_PROSE_PX,
};
use crate::ui_scale::px as ui_px;
use gpui::{
    div, prelude::*, rgba, AnyElement, App, ClipboardItem, ElementId, FontStyle, FontWeight,
    HighlightStyle, InteractiveText, ParentElement, SharedString, Styled, StyledText,
    UnderlineStyle,
};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SpanRun {
    pub range: Range<usize>,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RichLine {
    pub text: String,
    pub runs: Vec<SpanRun>,
}

impl RichLine {
    fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListItemBlock {
    pub checked: Option<bool>,
    pub content: RichLine,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MdBlock {
    Heading {
        level: u8,
        content: RichLine,
    },
    Paragraph {
        content: RichLine,
    },
    CodeBlock {
        lang: String,
        code: String,
    },
    Blockquote {
        lines: Vec<RichLine>,
    },
    List {
        ordered: bool,
        start: u64,
        items: Vec<ListItemBlock>,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Rule,
}

#[derive(Default)]
struct InlineBuilder {
    text: String,
    runs: Vec<SpanRun>,
    bold_depth: usize,
    italic_depth: usize,
    strike_depth: usize,
    link_stack: Vec<String>,
}

impl InlineBuilder {
    fn push_text(&mut self, fragment: &str, is_code: bool) {
        if fragment.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(fragment);
        let end = self.text.len();
        let link_url = self.link_stack.last().cloned();
        if is_code
            || self.bold_depth > 0
            || self.italic_depth > 0
            || self.strike_depth > 0
            || link_url.is_some()
        {
            self.runs.push(SpanRun {
                range: start..end,
                bold: self.bold_depth > 0,
                italic: self.italic_depth > 0,
                strike: self.strike_depth > 0,
                code: is_code,
                link_url,
            });
        }
    }

    fn finish(&mut self) -> RichLine {
        RichLine {
            text: std::mem::take(&mut self.text),
            runs: std::mem::take(&mut self.runs),
        }
    }
}

pub(crate) fn parse_markdown_blocks(input: &str) -> Vec<MdBlock> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(input, options);

    let mut blocks = Vec::new();
    let mut inline = InlineBuilder::default();

    let mut heading_level: Option<u8> = None;
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_buf = String::new();

    let mut blockquote_depth = 0usize;
    let mut quote_lines: Vec<RichLine> = Vec::new();

    let mut list_stack: Vec<(bool, u64)> = Vec::new();
    let mut list_items: Vec<ListItemBlock> = Vec::new();
    let mut current_task_checked: Option<bool> = None;
    let mut in_list_item = false;

    let mut in_table = false;
    let mut in_table_head = false;
    let mut table_headers: Vec<String> = Vec::new();
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut current_cell = String::new();
    let mut in_table_cell = false;

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    let lvl = match level {
                        pulldown_cmark::HeadingLevel::H1 => 1,
                        pulldown_cmark::HeadingLevel::H2 => 2,
                        pulldown_cmark::HeadingLevel::H3 => 3,
                        _ => 4,
                    };
                    heading_level = Some(lvl);
                    inline = InlineBuilder::default();
                }
                Tag::Paragraph => {
                    if !in_list_item {
                        inline = InlineBuilder::default();
                    }
                }
                Tag::CodeBlock(kind) => {
                    in_code_block = true;
                    code_lang = match kind {
                        CodeBlockKind::Fenced(info) => {
                            info.split_whitespace().next().unwrap_or("").to_string()
                        }
                        CodeBlockKind::Indented => String::new(),
                    };
                    code_buf.clear();
                }
                Tag::BlockQuote(_) => {
                    blockquote_depth += 1;
                }
                Tag::List(start) => {
                    if list_stack.is_empty() {
                        list_items.clear();
                    }
                    let ordered = start.is_some();
                    list_stack.push((ordered, start.unwrap_or(1)));
                }
                Tag::Item => {
                    in_list_item = true;
                    current_task_checked = None;
                    inline = InlineBuilder::default();
                }
                Tag::Table(_) => {
                    in_table = true;
                    table_headers.clear();
                    table_rows.clear();
                }
                Tag::TableHead => {
                    in_table_head = true;
                    current_row.clear();
                }
                Tag::TableRow => {
                    current_row.clear();
                }
                Tag::TableCell => {
                    in_table_cell = true;
                    current_cell.clear();
                }
                Tag::Strong => inline.bold_depth += 1,
                Tag::Emphasis => inline.italic_depth += 1,
                Tag::Strikethrough => inline.strike_depth += 1,
                Tag::Link { dest_url, .. } => inline.link_stack.push(dest_url.to_string()),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) => {
                    let content = inline.finish();
                    if let Some(level) = heading_level.take() {
                        if !content.is_empty() {
                            blocks.push(MdBlock::Heading { level, content });
                        }
                    }
                }
                TagEnd::Paragraph => {
                    if in_list_item {
                        if !inline.text.is_empty() {
                            inline.push_text(" ", false);
                        }
                    } else if blockquote_depth > 0 {
                        let line = inline.finish();
                        if !line.is_empty() {
                            quote_lines.push(line);
                        }
                    } else {
                        let content = inline.finish();
                        if !content.is_empty() {
                            blocks.push(MdBlock::Paragraph { content });
                        }
                    }
                }
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    let code = code_buf.trim_end_matches('\n').to_string();
                    blocks.push(MdBlock::CodeBlock {
                        lang: std::mem::take(&mut code_lang),
                        code,
                    });
                    code_buf.clear();
                }
                TagEnd::BlockQuote(_) => {
                    blockquote_depth = blockquote_depth.saturating_sub(1);
                    if blockquote_depth == 0 {
                        let line = inline.finish();
                        if !line.is_empty() {
                            quote_lines.push(line);
                        }
                        if !quote_lines.is_empty() {
                            blocks.push(MdBlock::Blockquote {
                                lines: std::mem::take(&mut quote_lines),
                            });
                        }
                    }
                }
                TagEnd::Item => {
                    in_list_item = false;
                    let content = inline.finish();
                    if !content.is_empty() || current_task_checked.is_some() {
                        list_items.push(ListItemBlock {
                            checked: current_task_checked.take(),
                            content,
                        });
                    }
                }
                TagEnd::List(_) => {
                    if let Some((ordered, start)) = list_stack.pop() {
                        if list_stack.is_empty() && !list_items.is_empty() {
                            blocks.push(MdBlock::List {
                                ordered,
                                start,
                                items: std::mem::take(&mut list_items),
                            });
                        }
                    }
                }
                TagEnd::TableCell => {
                    in_table_cell = false;
                    current_row.push(std::mem::take(&mut current_cell).trim().to_string());
                }
                TagEnd::TableHead => {
                    in_table_head = false;
                    if !current_row.is_empty() {
                        table_headers = std::mem::take(&mut current_row);
                    }
                }
                TagEnd::TableRow => {
                    if !in_table_head && !current_row.is_empty() {
                        table_rows.push(std::mem::take(&mut current_row));
                    }
                }
                TagEnd::Table => {
                    in_table = false;
                    if !table_headers.is_empty() || !table_rows.is_empty() {
                        blocks.push(MdBlock::Table {
                            headers: std::mem::take(&mut table_headers),
                            rows: std::mem::take(&mut table_rows),
                        });
                    }
                }
                TagEnd::Strong => inline.bold_depth = inline.bold_depth.saturating_sub(1),
                TagEnd::Emphasis => inline.italic_depth = inline.italic_depth.saturating_sub(1),
                TagEnd::Strikethrough => {
                    inline.strike_depth = inline.strike_depth.saturating_sub(1)
                }
                TagEnd::Link => {
                    inline.link_stack.pop();
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_code_block {
                    code_buf.push_str(&text);
                } else if in_table && in_table_cell {
                    current_cell.push_str(&text);
                } else {
                    inline.push_text(&text, false);
                }
            }
            Event::Code(code) => {
                if in_code_block {
                    code_buf.push_str(&code);
                } else if in_table && in_table_cell {
                    current_cell.push_str(&code);
                } else {
                    inline.push_text(&code, true);
                }
            }
            Event::SoftBreak => {
                if in_code_block {
                    code_buf.push('\n');
                } else if in_table && in_table_cell {
                    current_cell.push(' ');
                } else {
                    inline.push_text(" ", false);
                }
            }
            Event::HardBreak => {
                if in_code_block {
                    code_buf.push('\n');
                } else {
                    inline.push_text("\n", false);
                }
            }
            Event::Rule => blocks.push(MdBlock::Rule),
            Event::TaskListMarker(checked) => {
                current_task_checked = Some(checked);
            }
            _ => {}
        }
    }

    if in_code_block && !code_buf.is_empty() {
        blocks.push(MdBlock::CodeBlock {
            lang: code_lang,
            code: code_buf,
        });
    } else {
        let trailing = inline.finish();
        if !trailing.is_empty() {
            blocks.push(MdBlock::Paragraph { content: trailing });
        }
    }

    blocks
}

pub(crate) fn render_rich_line(
    id: ElementId,
    line: &RichLine,
    palette: ChatPalette,
    mono_font: &str,
) -> AnyElement {
    let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    let mut font_overrides: Vec<(Range<usize>, SharedString)> = Vec::new();
    let mut click_ranges: Vec<Range<usize>> = Vec::new();
    let mut click_urls: Vec<String> = Vec::new();
    let mono_shared: SharedString = mono_font.to_string().into();

    for run in &line.runs {
        let mut style = HighlightStyle::default();
        if run.bold {
            style.font_weight = Some(FontWeight::SEMIBOLD);
            style.color = Some(rgba(palette.fg_strong).into());
        }
        if run.italic {
            style.font_style = Some(FontStyle::Italic);
        }
        if run.strike {
            style.strikethrough = Some(gpui::StrikethroughStyle {
                thickness: gpui::px(1.),
                color: Some(rgba(palette.fg_faint).into()),
            });
        }
        if run.code {
            style.background_color = Some(rgba(palette.inline_code_surface).into());
            style.color = Some(rgba(palette.fg_strong).into());
            font_overrides.push((run.range.clone(), mono_shared.clone()));
        }
        if let Some(url) = &run.link_url {
            style.color = Some(rgba(palette.theme.accent).into());
            style.underline = Some(UnderlineStyle {
                thickness: gpui::px(1.),
                color: Some(rgba(palette.theme.accent).into()),
                wavy: false,
            });
            click_ranges.push(run.range.clone());
            click_urls.push(url.clone());
        }
        highlights.push((run.range.clone(), style));
    }

    let styled = StyledText::new(line.text.clone())
        .with_highlights(highlights)
        .with_font_family_overrides(font_overrides);

    if click_ranges.is_empty() {
        styled.into_any_element()
    } else {
        InteractiveText::new(id, styled)
            .on_click(click_ranges, move |ix, _window, cx: &mut App| {
                if let Some(url) = click_urls.get(ix) {
                    if url.starts_with("http://") || url.starts_with("https://") {
                        cx.open_url(url);
                    } else {
                        cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                    }
                }
            })
            .into_any_element()
    }
}

pub(crate) fn render_markdown_blocks(
    entry_id: &str,
    blocks: &[MdBlock],
    palette: ChatPalette,
    mono_font: &str,
) -> AnyElement {
    let mut stack = div()
        .flex()
        .flex_col()
        .gap_2()
        .w_full()
        .min_w_0()
        .text_size(ui_px(CHAT_PROSE_PX))
        .line_height(ui_px(21.0))
        .text_color(rgba(palette.fg_prose));

    for (idx, block) in blocks.iter().enumerate() {
        let block_el = match block {
            MdBlock::Heading { level, content } => {
                let (size, pt) = match level {
                    1 => (17.5, 4.0),
                    2 => (15.5, 3.0),
                    3 => (14.5, 2.0),
                    _ => (CHAT_PROSE_PX, 1.0),
                };
                div()
                    .pt(ui_px(pt))
                    .text_size(ui_px(size))
                    .line_height(ui_px(size * 1.4))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgba(palette.fg_strong))
                    .child(render_rich_line(
                        ElementId::Name(format!("md-{entry_id}-h-{idx}").into()),
                        content,
                        palette,
                        mono_font,
                    ))
                    .into_any_element()
            }
            MdBlock::Paragraph { content } => div()
                .w_full()
                .min_w_0()
                .child(render_rich_line(
                    ElementId::Name(format!("md-{entry_id}-p-{idx}").into()),
                    content,
                    palette,
                    mono_font,
                ))
                .into_any_element(),
            MdBlock::CodeBlock { lang, code } => {
                let header_lang = if lang.trim().is_empty() {
                    "TEXT".to_string()
                } else {
                    lang.trim().to_ascii_uppercase()
                };
                let copy_payload = code.clone();
                let runs = highlight_code(code, lang, palette);
                let highlighted = StyledText::new(code.clone()).with_highlights(runs);
                div()
                    .w_full()
                    .min_w_0()
                    .bg(rgba(palette.code_surface))
                    .border_1()
                    .border_color(rgba(palette.hairline))
                    .child(
                        div()
                            .h(ui_px(26.))
                            .px_2p5()
                            .flex()
                            .items_center()
                            .justify_between()
                            .bg(rgba(palette.card))
                            .border_b_1()
                            .border_color(rgba(palette.hairline))
                            .child(
                                div()
                                    .text_size(ui_px(CHAT_MICRO_PX))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgba(palette.fg_faint))
                                    .child(header_lang),
                            )
                            .child(
                                square_button(
                                    ElementId::Name(format!("copy-{entry_id}-{idx}").into()),
                                    "Copy",
                                    ButtonVariant::Ghost,
                                    palette,
                                )
                                .h(ui_px(20.))
                                .px_1p5()
                                .text_size(ui_px(CHAT_MICRO_PX))
                                .on_click(
                                    move |_ev, _window, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy_payload.clone(),
                                        ));
                                    },
                                ),
                            ),
                    )
                    .child(
                        div()
                            .id(ElementId::Name(
                                format!("code-scroll-{entry_id}-{idx}").into(),
                            ))
                            .w_full()
                            .overflow_x_scroll()
                            .px_3()
                            .py_2p5()
                            .font_family(SharedString::from(mono_font.to_string()))
                            .text_size(ui_px(CHAT_CODE_PX))
                            .line_height(ui_px(18.5))
                            .text_color(rgba(palette.fg_strong))
                            .child(highlighted),
                    )
                    .into_any_element()
            }
            MdBlock::Blockquote { lines } => {
                let mut quote = div()
                    .w_full()
                    .flex()
                    .bg(rgba(palette.card))
                    .border_1()
                    .border_color(rgba(palette.subtle_line))
                    .child(
                        div()
                            .w(ui_px(2.5))
                            .flex_none()
                            .bg(rgba(palette.theme.accent)),
                    );
                let mut inner = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .flex_1()
                    .min_w_0()
                    .text_color(rgba(palette.fg_muted));
                for (q_idx, line) in lines.iter().enumerate() {
                    inner = inner.child(render_rich_line(
                        ElementId::Name(format!("md-{entry_id}-q-{idx}-{q_idx}").into()),
                        line,
                        palette,
                        mono_font,
                    ));
                }
                quote = quote.child(inner);
                quote.into_any_element()
            }
            MdBlock::List {
                ordered,
                start,
                items,
            } => {
                let mut list_col = div().flex().flex_col().gap_1().w_full().pl_1();
                for (item_idx, item) in items.iter().enumerate() {
                    let marker = match item.checked {
                        Some(true) => "[x]".to_string(),
                        Some(false) => "[ ]".to_string(),
                        None if *ordered => format!("{}.", start + item_idx as u64),
                        None => "▪".to_string(),
                    };
                    let marker_color = match item.checked {
                        Some(true) => palette.theme.green,
                        _ => palette.fg_faint,
                    };
                    list_col = list_col.child(
                        div()
                            .flex()
                            .items_start()
                            .gap_2()
                            .w_full()
                            .min_w_0()
                            .child(
                                div()
                                    .flex_none()
                                    .min_w(ui_px(18.))
                                    .text_color(rgba(marker_color))
                                    .font_family(SharedString::from(mono_font.to_string()))
                                    .text_size(ui_px(CHAT_CODE_PX))
                                    .child(marker),
                            )
                            .child(div().flex_1().min_w_0().child(render_rich_line(
                                ElementId::Name(
                                    format!("md-{entry_id}-li-{idx}-{item_idx}").into(),
                                ),
                                &item.content,
                                palette,
                                mono_font,
                            ))),
                    );
                }
                list_col.into_any_element()
            }
            MdBlock::Table { headers, rows } => {
                let mut tbl = div()
                    .w_full()
                    .border_1()
                    .border_color(rgba(palette.hairline))
                    .bg(rgba(palette.card));
                if !headers.is_empty() {
                    let mut hrow = div()
                        .flex()
                        .w_full()
                        .bg(rgba(palette.inset))
                        .border_b_1()
                        .border_color(rgba(palette.hairline));
                    for (ci, cell) in headers.iter().enumerate() {
                        hrow = hrow.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .px_2()
                                .py_1p5()
                                .when(ci > 0, |c| {
                                    c.border_l_1().border_color(rgba(palette.hairline))
                                })
                                .text_size(ui_px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgba(palette.fg_strong))
                                .child(cell.clone()),
                        );
                    }
                    tbl = tbl.child(hrow);
                }
                for (ri, row) in rows.iter().enumerate() {
                    let mut rdiv = div().flex().w_full().when(ri + 1 < rows.len(), |r| {
                        r.border_b_1().border_color(rgba(palette.subtle_line))
                    });
                    for (ci, cell) in row.iter().enumerate() {
                        rdiv = rdiv.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .px_2()
                                .py_1p5()
                                .when(ci > 0, |c| {
                                    c.border_l_1().border_color(rgba(palette.subtle_line))
                                })
                                .text_size(ui_px(12.))
                                .text_color(rgba(palette.fg_prose))
                                .child(cell.clone()),
                        );
                    }
                    tbl = tbl.child(rdiv);
                }
                tbl.into_any_element()
            }
            MdBlock::Rule => div()
                .w_full()
                .h(ui_px(1.))
                .my_1()
                .bg(rgba(palette.hairline))
                .into_any_element(),
        };
        stack = stack.child(block_el);
    }

    stack.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_inline_styles_into_disjoint_char_boundary_runs() {
        let md = "Hello **[link text](https://example.com)** and `code` plus 中文!";
        let blocks = parse_markdown_blocks(md);
        assert_eq!(blocks.len(), 1);
        let MdBlock::Paragraph { content } = &blocks[0] else {
            panic!("expected paragraph");
        };
        let mut prev_end = 0;
        for run in &content.runs {
            assert!(run.range.start >= prev_end);
            assert!(content.text.is_char_boundary(run.range.start));
            assert!(content.text.is_char_boundary(run.range.end));
            prev_end = run.range.end;
        }
        assert!(content
            .runs
            .iter()
            .any(|r| r.code && &content.text[r.range.clone()] == "code"));
        assert!(content
            .runs
            .iter()
            .any(|r| r.link_url.as_deref() == Some("https://example.com")));
    }

    #[test]
    fn parses_fenced_code_tables_and_tasklists() {
        let md = "## Title\n\n```rust\nlet x = 1;\n```\n\n- [x] Done\n- [ ] Todo\n\n| A | B |\n| - | - |\n| 1 | 2 |\n";
        let blocks = parse_markdown_blocks(md);
        assert!(matches!(blocks[0], MdBlock::Heading { level: 2, .. }));
        assert!(matches!(&blocks[1], MdBlock::CodeBlock { lang, .. } if lang == "rust"));
        assert!(
            matches!(&blocks[2], MdBlock::List { items, .. } if items.len() == 2 && items[0].checked == Some(true))
        );
        assert!(
            matches!(&blocks[3], MdBlock::Table { headers, rows } if headers == &["A", "B"] && rows.len() == 1)
        );
    }
}
