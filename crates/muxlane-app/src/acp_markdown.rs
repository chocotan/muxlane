use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MarkdownBlock {
    Paragraph(String),
    Heading {
        level: u8,
        text: String,
    },
    Code {
        language: Option<String>,
        text: String,
    },
    ListItem(String),
    Quote(String),
    Rule,
}

enum OpenBlock {
    Paragraph,
    Heading(u8),
    Code(Option<String>),
    ListItem,
    Quote,
}

pub(crate) fn parse_markdown(source: &str) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let mut open = None;
    let mut text = String::new();
    let parser = Parser::new_ext(
        source,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS,
    );

    for event in parser {
        match event {
            Event::Start(Tag::Paragraph) => {
                start_block(&mut blocks, &mut open, &mut text, OpenBlock::Paragraph)
            }
            Event::Start(Tag::Heading { level, .. }) => start_block(
                &mut blocks,
                &mut open,
                &mut text,
                OpenBlock::Heading(heading_level(level)),
            ),
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match kind {
                    CodeBlockKind::Indented => None,
                    CodeBlockKind::Fenced(language) if language.is_empty() => None,
                    CodeBlockKind::Fenced(language) => Some(language.into_string()),
                };
                start_block(&mut blocks, &mut open, &mut text, OpenBlock::Code(language));
            }
            Event::Start(Tag::Item) => {
                start_block(&mut blocks, &mut open, &mut text, OpenBlock::ListItem)
            }
            Event::Start(Tag::BlockQuote(_)) => {
                start_block(&mut blocks, &mut open, &mut text, OpenBlock::Quote)
            }
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::CodeBlock
                | TagEnd::Item
                | TagEnd::BlockQuote(_),
            ) => flush(&mut blocks, &mut open, &mut text),
            Event::Text(value) => text.push_str(&value),
            Event::Code(value) => {
                text.push('`');
                text.push_str(&value);
                text.push('`');
            }
            Event::SoftBreak => {
                if matches!(open, Some(OpenBlock::Code(_))) {
                    text.push('\n');
                } else {
                    text.push(' ');
                }
            }
            Event::HardBreak => text.push('\n'),
            Event::Rule => {
                flush(&mut blocks, &mut open, &mut text);
                blocks.push(MarkdownBlock::Rule);
            }
            _ => {}
        }
    }
    flush(&mut blocks, &mut open, &mut text);
    blocks
}

fn start_block(
    blocks: &mut Vec<MarkdownBlock>,
    open: &mut Option<OpenBlock>,
    text: &mut String,
    next: OpenBlock,
) {
    if open.is_some() && !text.is_empty() {
        flush(blocks, open, text);
    }
    *open = Some(next);
}

fn flush(blocks: &mut Vec<MarkdownBlock>, open: &mut Option<OpenBlock>, text: &mut String) {
    let Some(kind) = open.take() else {
        return;
    };
    let value = std::mem::take(text);
    if value.is_empty() {
        return;
    }
    blocks.push(match kind {
        OpenBlock::Paragraph => MarkdownBlock::Paragraph(value),
        OpenBlock::Heading(level) => MarkdownBlock::Heading { level, text: value },
        OpenBlock::Code(language) => MarkdownBlock::Code {
            language,
            text: value,
        },
        OpenBlock::ListItem => MarkdownBlock::ListItem(value),
        OpenBlock::Quote => MarkdownBlock::Quote(value),
    });
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_primary_markdown_blocks() {
        let blocks = parse_markdown(
            "# Title\n\nParagraph with `code`.\n\n- one\n- two\n\n```rust\nfn main() {}\n```",
        );

        assert!(matches!(
            &blocks[0],
            MarkdownBlock::Heading { level: 1, text } if text == "Title"
        ));
        assert!(blocks
            .iter()
            .any(|block| matches!(block, MarkdownBlock::ListItem(text) if text == "one")));
        assert!(blocks.iter().any(|block| matches!(
            block,
            MarkdownBlock::Code { language: Some(language), text }
                if language == "rust" && text.contains("fn main")
        )));
    }
}
