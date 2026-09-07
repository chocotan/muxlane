//! Cached syntax foregrounds with byte-aligned selection backgrounds.
use crate::theme::Theme;
use gpui::{rgba, HighlightStyle};
use std::{collections::HashMap, ops::Range, sync::{Arc, LazyLock}};
use syntect::{easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet, util::LinesWithEndings};

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

type Spans = Arc<Vec<(Range<usize>, u32)>>;
struct Cached {
    source: String,
    language: String,
    dark: bool,
    spans: Spans,
}

#[derive(Default)]
pub(super) struct CodeHighlightCache(HashMap<String, Cached>);

impl CodeHighlightCache {
    pub(super) fn highlights(&mut self, id: &str, source: &str, language: Option<&str>, theme: Theme) -> Spans {
        let language = language.unwrap_or("").split_whitespace().next().unwrap_or("").to_lowercase();
        let dark = ((theme.bg0 >> 24) & 255) + ((theme.bg0 >> 16) & 255) + ((theme.bg0 >> 8) & 255) < 384;
        if let Some(cached) = self.0.get(id).filter(|cached| cached.source == source && cached.language == language && cached.dark == dark) {
            return cached.spans.clone();
        }
        let spans = Arc::new(highlight(source, &language, dark));
        // Bound the cache when a streaming response changes block structure.
        if self.0.len() >= 128 && !self.0.contains_key(id) { self.0.clear(); }
        self.0.insert(id.into(), Cached { source: source.into(), language, dark, spans: spans.clone() });
        spans
    }
}

fn highlight(source: &str, language: &str, dark: bool) -> Vec<(Range<usize>, u32)> {
    let token = match language { "js" => "javascript", "ts" => "typescript", "sh" | "shell" => "bash", other => other };
    let Some(syntax) = SYNTAXES.find_syntax_by_token(token) else { return Vec::new(); };
    let theme = &THEMES.themes[if dark { "base16-ocean.dark" } else { "InspiredGitHub" }];
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut offset = 0;
    let mut spans = Vec::new();
    for line in LinesWithEndings::from(source) {
        let Ok(regions) = highlighter.highlight_line(line, &SYNTAXES) else { return Vec::new(); };
        for (style, text) in regions {
            let end = offset + text.len();
            let color = style.foreground;
            spans.push((offset..end, u32::from_be_bytes([color.r, color.g, color.b, color.a])));
            offset = end;
        }
    }
    spans
}

pub(super) fn compose_highlights(source: &str, syntax: &[(Range<usize>, u32)], selection: Option<Range<usize>>, theme: Theme) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut boundaries = vec![0, source.len()];
    for (range, _) in syntax { boundaries.extend([range.start, range.end]); }
    if let Some(range) = &selection { boundaries.extend([range.start, range.end]); }
    boundaries.retain(|&offset| offset <= source.len() && source.is_char_boundary(offset));
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut syntax_index = 0;
    boundaries.windows(2).filter_map(|pair| {
        let range = pair[0]..pair[1];
        while syntax_index < syntax.len() && syntax[syntax_index].0.end <= range.start { syntax_index += 1; }
        let color = syntax.get(syntax_index).filter(|(span, _)| span.start <= range.start && span.end >= range.end).map(|(_, color)| rgba(*color).into());
        let selected = selection.as_ref().is_some_and(|selection| selection.start <= range.start && selection.end >= range.end);
        (color.is_some() || selected).then(|| (range, HighlightStyle { color, background_color: selected.then(|| rgba(theme.selection()).into()), ..Default::default() }))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_spans_and_selection_are_disjoint_byte_ranges() {
        let source = "fn main() {\n    let 中文 = \"😀\";\n}\n";
        let spans = highlight(source, "rust", false);
        assert!(!spans.is_empty());
        assert_eq!(spans.last().unwrap().0.end, source.len());
        let start = source.find('😀').unwrap();
        let result = compose_highlights(source, &spans, Some(start..start + 4), Theme::for_mode(crate::theme::ThemeMode::Light));
        for (range, _) in &result { assert!(source.is_char_boundary(range.start) && source.is_char_boundary(range.end)); }
        for pair in result.windows(2) { assert!(pair[0].0.end <= pair[1].0.start); }
        assert!(result.iter().any(|(range, style)| range == &(start..start + 4) && style.background_color.is_some() && style.color.is_some()));
        assert!(highlight(source, "not-a-language", false).is_empty());
    }
    #[test]
    fn long_and_unterminated_code_preserves_every_byte_and_cache_identity() {
        let source = format!("let s = \"{}", "中文".repeat(2048));
        let theme = Theme::for_mode(crate::theme::ThemeMode::Light);
        let mut cache = CodeHighlightCache::default();
        let a = cache.highlights("block", &source, Some("rust"), theme);
        let b = cache.highlights("block", &source, Some("rust"), theme);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(a.last().unwrap().0.end, source.len());
        let c = cache.highlights("block", &source, Some("unknown"), theme);
        assert!(c.is_empty());
    }
}
