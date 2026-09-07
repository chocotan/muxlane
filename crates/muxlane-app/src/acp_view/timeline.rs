use super::style::*;
use super::code_highlight::{CodeHighlightCache, compose_highlights};
use super::tool_display::{self, DiffCache};
use super::{AcpView, Entry};
use crate::acp_markdown::{parse_markdown, MarkdownBlock};
use crate::i18n;
use crate::icons::{panel_icon, ARROW_DOWN_ICON, COPY_ICON};
use crate::theme::{Theme, ThemeMode};
use crate::ui_scale::px as ui_px;
use crate::widgets::{render_status_indicator, semantic_button};
use base64::Engine as _;
use gpui::{
    div, img, prelude::*, rgba, App, Bounds, Context, Element, FocusHandle, GlobalElementId,
    Image, ImageFormat, InteractiveElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, SharedString,
    StatefulInteractiveElement, Styled, StyledText, TextLayout, WeakEntity, Window,
};
use muxlane_acp::{MessageRole, ThreadItem, ToolContent};
use muxlane_core::model::AgentStatus;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffDecision {
    Kept,
    Rejected,
}

pub(crate) struct GeneratingIndicator {
    theme_mode: ThemeMode,
    language: i18n::Language,
}

impl GeneratingIndicator {
    pub(crate) fn new(theme_mode: ThemeMode, language: i18n::Language) -> Self {
        Self {
            theme_mode,
            language,
        }
    }

    pub(crate) fn set_theme_mode(&mut self, theme_mode: ThemeMode, cx: &mut Context<Self>) {
        self.theme_mode = theme_mode;
        cx.notify();
    }

    pub(crate) fn set_language(&mut self, language: i18n::Language, cx: &mut Context<Self>) {
        self.language = language;
        cx.notify();
    }
}

impl Render for GeneratingIndicator {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_mode(self.theme_mode);
        div()
            .pl(ui_px(GUTTER_PAD))
            .pr_3()
            .py_1()
            .flex()
            .items_center()
            .text_size(ui_px(BODY_SIZE))
            .line_height(ui_px(BODY_LINE))
            .text_color(rgba(theme.fg1))
            .child(
                div()
                    .w(ui_px(GUTTER_WIDTH))
                    .flex_none()
                    .flex()
                    .items_center()
                    .child(render_status_indicator(
                        AgentStatus::Working,
                        "acp-generating",
                        theme,
                    )),
            )
            .child(i18n::text(self.language, "acp.generating"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct TimelineSelection {
    scope: Option<String>,
    anchor: usize,
    active: usize,
    dragging: bool,
}

impl TimelineSelection {
    fn begin(&mut self, scope: &str, offset: usize) {
        self.scope = Some(scope.to_string());
        self.anchor = offset;
        self.active = offset;
        self.dragging = true;
    }

    fn extend(&mut self, scope: &str, offset: usize) {
        if self.scope.as_deref() == Some(scope) && self.dragging {
            self.active = offset;
        }
    }

    fn select_range(&mut self, scope: &str, range: Range<usize>) {
        self.scope = Some(scope.to_string());
        self.anchor = range.start;
        self.active = range.end;
        self.dragging = false;
    }

    fn end_drag(&mut self) {
        self.dragging = false;
    }

    fn normalized(&self) -> Range<usize> {
        self.anchor.min(self.active)..self.anchor.max(self.active)
    }

    fn selected_text(&self, text: &str) -> Option<String> {
        let range = self.normalized();
        let start = nearest_char_boundary(text, range.start);
        let end = nearest_char_boundary(text, range.end);
        (start < end)
            .then(|| text.get(start..end).map(str::to_string))
            .flatten()
            .filter(|text| !text.is_empty())
    }

    fn highlight(&self, scope: &str, range: &Range<usize>) -> Option<Range<usize>> {
        if self.scope.as_deref() != Some(scope) {
            return None;
        }
        let selection = self.normalized();
        let start = selection.start.max(range.start);
        let end = selection.end.min(range.end);
        (start < end).then(|| start - range.start..end - range.start)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct TimelineItemTransientState {
    expanded: bool,
    raw_expanded: HashSet<String>,
    terminal_outputs: HashMap<String, muxlane_acp::TerminalOutputState>,
    diff_decisions: HashMap<(String, String), DiffDecision>,
    selection: TimelineSelection,
}

pub(crate) fn preserve_transient_state(
    old_item: &ThreadItem,
    new_item: &ThreadItem,
    state: &TimelineItemTransientState,
) -> Option<TimelineItemTransientState> {
    (old_item == new_item).then(|| state.clone())
}

struct TimelineLayoutRecord {
    scope: String,
    range: Range<usize>,
    bounds: Bounds<Pixels>,
    layout: TextLayout,
    content: String,
    #[cfg(test)]
    painted_highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
}

struct TimelineTextElement {
    layouts: Rc<RefCell<Vec<TimelineLayoutRecord>>>,
    text: StyledText,
    content: String,
    scope: String,
    range: Range<usize>,
    #[cfg(test)]
    highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
}

impl IntoElement for TimelineTextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TimelineTextElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.text.request_layout(None, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.text
            .prepaint(None, inspector_id, bounds, request_layout, window, cx)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.text.paint(
            None,
            inspector_id,
            bounds,
            request_layout,
            &mut (),
            window,
            cx,
        );
        let mut layouts = self.layouts.borrow_mut();
        // Repainting after resize need not rerender the entity. Replace this block's geometry.
        layouts.retain(|record| record.scope != self.scope || record.range.start != self.range.start);
        layouts.push(TimelineLayoutRecord {
            scope: self.scope.clone(),
            range: self.range.clone(),
            bounds,
            layout: self.text.layout().clone(),
            content: self.content.clone(),
            #[cfg(test)]
            painted_highlights: self.highlights.clone(),
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlattenedMarkdown {
    text: String,
    ranges: Vec<Option<Range<usize>>>,
}

#[derive(Clone)]
enum ImageCacheState {
    Loading,
    Ready(std::sync::Arc<Image>),
    Error(String),
}

struct ImageCacheEntry {
    fingerprint: u64,
    generation: u64,
    state: ImageCacheState,
}

struct MarkdownCacheEntry {
    source: String,
    blocks: std::sync::Arc<Vec<MarkdownBlock>>,
    flattened: std::sync::Arc<FlattenedMarkdown>,
}

fn cached_markdown(
    cache: &mut HashMap<String, MarkdownCacheEntry>,
    key: &str,
    source: &str,
) -> (
    std::sync::Arc<Vec<MarkdownBlock>>,
    std::sync::Arc<FlattenedMarkdown>,
) {
    let entry = cache
        .entry(key.to_string())
        .or_insert_with(|| MarkdownCacheEntry {
            source: String::new(),
            blocks: std::sync::Arc::new(Vec::new()),
            flattened: std::sync::Arc::new(FlattenedMarkdown {
                text: String::new(),
                ranges: Vec::new(),
            }),
        });
    if entry.source != source {
        let blocks = std::sync::Arc::new(parse_markdown(source));
        entry.source.clear();
        entry.source.push_str(source);
        entry.flattened = std::sync::Arc::new(markdown_selection_parts(&blocks));
        entry.blocks = blocks;
    }
    (entry.blocks.clone(), entry.flattened.clone())
}

fn markdown_selection_parts(blocks: &[MarkdownBlock]) -> FlattenedMarkdown {
    let mut text = String::new();
    let mut ranges = Vec::with_capacity(blocks.len());
    for block in blocks {
        let block_text = match block {
            MarkdownBlock::Paragraph(value)
            | MarkdownBlock::Heading { text: value, .. }
            | MarkdownBlock::Code { text: value, .. }
            | MarkdownBlock::ListItem(value)
            | MarkdownBlock::Quote(value) => value,
            MarkdownBlock::Rule => {
                ranges.push(None);
                continue;
            }
        };
        if block_text.is_empty() {
            ranges.push(None);
            continue;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        let start = text.len();
        text.push_str(block_text);
        ranges.push(Some(start..text.len()));
    }
    FlattenedMarkdown { text, ranges }
}

fn nearest_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn clicked_char_start(text: &str, offset: usize) -> Option<usize> {
    if text.is_empty() {
        return None;
    }
    let offset = nearest_char_boundary(text, offset);
    if offset == text.len() {
        return text.char_indices().next_back().map(|(start, _)| start);
    }
    Some(offset)
}

fn surrounding_word_range(text: &str, offset: usize) -> Range<usize> {
    let Some(start) = clicked_char_start(text, offset) else {
        return 0..0;
    };
    let character = text[start..].chars().next().unwrap_or_default();
    let is_word = |character: char| character.is_alphanumeric() || character == '_';
    let is_word_character = is_word(character);
    let is_selectable = |candidate: char| {
        if is_word_character {
            is_word(candidate)
        } else if character.is_whitespace() {
            candidate.is_whitespace()
        } else {
            false
        }
    };

    let mut range_start = start;
    while range_start > 0 {
        let Some((previous_start, previous)) = text[..range_start].char_indices().next_back()
        else {
            break;
        };
        if !is_selectable(previous) {
            break;
        }
        range_start = previous_start;
    }

    let mut range_end = start + character.len_utf8();
    while range_end < text.len() {
        let next = text[range_end..].chars().next().unwrap_or_default();
        if !is_selectable(next) {
            break;
        }
        range_end += next.len_utf8();
    }
    range_start..range_end
}

fn surrounding_line_range(text: &str, offset: usize) -> Range<usize> {
    let boundary = nearest_char_boundary(text, offset);
    let start = text[..boundary]
        .rfind('\n')
        .map_or(0, |newline| newline + 1);
    let end = text[boundary..]
        .find('\n')
        .map_or(text.len(), |newline| boundary + newline + 1);
    start..end
}

fn timeline_point_to_offset(
    records: &[TimelineLayoutRecord],
    scope: &str,
    position: Point<Pixels>,
) -> Option<usize> {
    let mut scoped = records
        .iter()
        .filter(|record| record.scope == scope)
        .collect::<Vec<_>>();
    scoped.sort_by(|left, right| {
        left.bounds
            .top()
            .partial_cmp(&right.bounds.top())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left.bounds
                    .left()
                    .partial_cmp(&right.bounds.left())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    let first = *scoped.first()?;
    if position.y < first.bounds.top() {
        return Some(first.range.start);
    }
    let mut last = first;
    for record in scoped {
        if position.y < record.bounds.top() {
            return Some(record.range.start);
        }
        if position.y <= record.bounds.bottom() {
            let offset = match record.layout.index_for_position(position) {
                Ok(offset) | Err(offset) => offset.min(record.range.end - record.range.start),
            };
            return Some(nearest_char_boundary(&record.content, offset) + record.range.start);
        }
        last = record;
    }
    Some(last.range.end)
}

pub(crate) struct TimelineItemView {
    item: ThreadItem,
    parent: WeakEntity<AcpView>,
    project_path: std::path::PathBuf,
    theme_mode: ThemeMode,
    language: i18n::Language,
    expanded: bool,
    raw_expanded: HashSet<String>,
    raw_cache: RefCell<HashMap<String, String>>,
    diff_cache: RefCell<DiffCache>,
    terminal_outputs: HashMap<String, muxlane_acp::TerminalOutputState>,
    diff_decisions: HashMap<(String, String), DiffDecision>,
    focus: FocusHandle,
    selection: TimelineSelection,
    layouts: Rc<RefCell<Vec<TimelineLayoutRecord>>>,
    texts: RefCell<HashMap<String, String>>,
    markdown_cache: RefCell<HashMap<String, MarkdownCacheEntry>>,
    code_cache: RefCell<CodeHighlightCache>,
    code_scrolls: RefCell<HashMap<String, gpui::ScrollHandle>>,
    image_cache: RefCell<HashMap<String, ImageCacheEntry>>,
    image_fingerprints: HashMap<String, u64>,
    image_generation: Cell<u64>,
}

impl TimelineItemView {
    #[cfg(test)]
    pub(super) fn expand_tool_fixture(&mut self, cx: &mut Context<Self>) {
        self.expanded = true;
        cx.notify();
    }

    #[cfg(test)]
    pub(super) fn assert_tool_fixture(&self, scope: &str, raw_scope: &str) {
        assert!(!self.texts.borrow().contains_key(raw_scope), "raw input stays collapsed");
        let scrolls = self.code_scrolls.borrow();
        let scroll = scrolls.get(scope).expect("tool code has a persistent scroll handle");
        assert!(scroll.max_offset().x > ui_px(0.), "long tool lines must scroll horizontally: {scope}");
        let texts = self.texts.borrow();
        assert!(texts.get(scope).unwrap().contains("  indented"), "whitespace is preserved");
    }

    #[cfg(test)]
    pub(super) fn terminal_state(&self, id: &str) -> muxlane_acp::TerminalOutputState {
        self.terminal_outputs.get(id).cloned().unwrap_or(muxlane_acp::TerminalOutputState::Unavailable)
    }

    #[cfg(test)]
    pub(super) fn assert_current_text_geometry(&self) -> Bounds<Pixels> {
        let records = self.layouts.borrow();
        let keys: HashSet<_> = records.iter().map(|record| (&record.scope, record.range.start)).collect();
        assert_eq!(keys.len(), records.len(), "a repaint must replace old block geometry");
        assert_eq!(records.len(), 3, "fixture contains two paragraphs and one code block");
        for record in records.iter() {
            assert_eq!(record.bounds, record.layout.bounds());
            for index in record.content.char_indices().map(|(index, _)| index).filter(|index| *index > 0).take(24) {
                let point = record.layout.position_for_index(index).unwrap() + gpui::point(ui_px(0.1), ui_px(1.));
                assert_eq!(timeline_point_to_offset(&records, &record.scope, point), Some(record.range.start + index));
            }
        }
        let scrolls = self.code_scrolls.borrow();
        assert_eq!(scrolls.len(), 1);
        let scroll = scrolls.values().next().unwrap();
        assert!(scroll.max_offset().x > ui_px(0.), "long code must overflow only its own viewport");
        records.iter().find(|record| record.range.start == 0).unwrap().bounds
    }

    pub(crate) fn new(
        item: &ThreadItem,
        parent: WeakEntity<AcpView>,
        project_path: std::path::PathBuf,
        theme_mode: ThemeMode,
        language: i18n::Language,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            item: item.clone(),
            parent,
            project_path,
            theme_mode,
            language,
            expanded: false,
            raw_expanded: HashSet::new(),
            raw_cache: RefCell::new(HashMap::new()),
            diff_cache: RefCell::new(DiffCache::default()),
            terminal_outputs: HashMap::new(),
            diff_decisions: HashMap::new(),
            focus: cx.focus_handle(),
            selection: TimelineSelection::default(),
            layouts: Rc::new(RefCell::new(Vec::new())),
            texts: RefCell::new(HashMap::new()),
            markdown_cache: RefCell::new(HashMap::new()),
            code_cache: RefCell::new(CodeHighlightCache::default()),
            code_scrolls: RefCell::new(HashMap::new()),
            image_cache: RefCell::new(HashMap::new()),
            image_fingerprints: image_fingerprints(item),
            image_generation: Cell::new(0),
        }
    }

    pub(crate) fn key(item: &ThreadItem) -> String {
        match item {
            ThreadItem::Message(item) => format!("message:{}", item.id),
            ThreadItem::Content(item) => format!("content:{}", item.id),
            ThreadItem::Thought(item) => format!("thought:{}", item.id),
            ThreadItem::Tool(item) => format!("tool:{}", item.id),
        }
    }

    pub(crate) fn item_key(&self) -> String {
        Self::key(&self.item)
    }

    pub(crate) fn item(&self) -> &ThreadItem {
        &self.item
    }

    pub(crate) fn transient_state(&self) -> TimelineItemTransientState {
        TimelineItemTransientState {
            expanded: self.expanded,
            raw_expanded: self.raw_expanded.clone(),
            terminal_outputs: self.terminal_outputs.clone(),
            diff_decisions: self.diff_decisions.clone(),
            selection: self.selection.clone(),
        }
    }

    pub(crate) fn restore_transient_state(&mut self, state: TimelineItemTransientState) {
        self.expanded = state.expanded;
        self.raw_expanded = state.raw_expanded;
        self.terminal_outputs = state.terminal_outputs;
        self.diff_decisions = state.diff_decisions;
        self.selection = state.selection;
    }

    pub(crate) fn contains_terminal(&self, id: &str) -> bool {
        match &self.item {
            ThreadItem::Content(item) => content_contains_terminal(&item.content, id),
            ThreadItem::Tool(item) => item
                .content
                .iter()
                .any(|content| content_contains_terminal(content, id)),
            _ => false,
        }
    }

    pub(crate) fn set_item(&mut self, item: ThreadItem, cx: &mut Context<Self>) {
        if self.item != item {
            self.raw_cache.borrow_mut().clear();
            self.diff_decisions.clear();
            self.selection = TimelineSelection::default();
        }
        let fingerprints = image_fingerprints(&item);
        let markdown_scopes = markdown_scopes(&item);
        self.item = item;
        self.texts.borrow_mut().clear();
        self.image_cache
            .borrow_mut()
            .retain(|scope, entry| fingerprints.get(scope) == Some(&entry.fingerprint));
        self.image_fingerprints = fingerprints;
        self.markdown_cache
            .borrow_mut()
            .retain(|scope, _| markdown_scopes.contains(scope));
        cx.notify();
    }

    pub(crate) fn set_theme_mode(&mut self, theme_mode: ThemeMode, cx: &mut Context<Self>) {
        self.theme_mode = theme_mode;
        cx.notify();
    }

    pub(crate) fn set_language(&mut self, language: i18n::Language, cx: &mut Context<Self>) {
        self.language = language;
        cx.notify();
    }

    pub(crate) fn set_terminal_result(
        &mut self,
        result: muxlane_acp::TerminalQueryResult,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.terminal_outputs.get(&result.id), Some(muxlane_acp::TerminalOutputState::Ready(_)))
            || matches!(&result.state, muxlane_acp::TerminalOutputState::Ready(_)) {
            self.terminal_outputs.insert(result.id, result.state);
        }
        cx.notify();
    }

    fn theme(&self) -> Theme {
        Theme::for_mode(self.theme_mode)
    }

    fn timeline_mouse_down(
        &mut self,
        scope: &str,
        position: Point<Pixels>,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window, cx);
        window.invalidate_character_coordinates();
        let offset = {
            let layouts = self.layouts.borrow();
            timeline_point_to_offset(&layouts, scope, position)
        };
        if let Some(offset) = offset {
            if click_count == 1 {
                self.selection.begin(scope, offset);
            } else {
                let range = self.texts.borrow().get(scope).map(|text| {
                    if click_count >= 4 {
                        0..text.len()
                    } else if click_count >= 3 {
                        surrounding_line_range(text, offset)
                    } else {
                        surrounding_word_range(text, offset)
                    }
                });
                if let Some(range) = range {
                    self.selection.select_range(scope, range);
                }
            }
            cx.notify();
        }
        cx.stop_propagation();
    }

    fn timeline_mouse_move(
        &mut self,
        position: Point<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(scope) = self.selection.scope.clone() else {
            return;
        };
        let offset = {
            let layouts = self.layouts.borrow();
            timeline_point_to_offset(&layouts, &scope, position)
        };
        if let Some(offset) = offset {
            let previous = self.selection.active;
            self.selection.extend(&scope, offset);
            if self.selection.active != previous {
                cx.notify();
            }
        }
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let text = self
            .selection
            .scope
            .as_ref()
            .and_then(|scope| self.texts.borrow().get(scope).cloned())
            .and_then(|text| self.selection.selected_text(&text));
        let Some(text) = text else {
            return false;
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        true
    }

    fn register_text(&self, scope: &str, text: String) {
        self.texts.borrow_mut().insert(scope.to_string(), text);
    }

    fn render_selectable_text(
        &self,
        scope: &str,
        text: &str,
        range: Range<usize>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        self.render_text_with_syntax(scope, text, range, theme, &[], cx)
    }

    fn render_text_with_syntax(
        &self,
        scope: &str,
        text: &str,
        range: Range<usize>,
        theme: Theme,
        syntax: &[(Range<usize>, u32)],
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let display_text: SharedString = text.to_string().into();
        let highlights = compose_highlights(text, syntax, self.selection.highlight(scope, &range), theme);
        #[cfg(test)]
        let painted_highlights = highlights.clone();
        let styled_text = StyledText::new(display_text.clone()).with_highlights(highlights);
        let scope_for_event = scope.to_string();
        let layouts = self.layouts.clone();
        div()
            .min_w_0()
            .w_full()
            .flex_shrink(1.)
            .cursor(gpui::CursorStyle::IBeam)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.timeline_mouse_down(
                        &scope_for_event,
                        event.position,
                        event.click_count,
                        window,
                        cx,
                    );
                }),
            )
            .child(TimelineTextElement {
                layouts,
                text: styled_text,
                content: display_text.to_string(),
                scope: scope.to_string(),
                range,
                #[cfg(test)]
                highlights: painted_highlights,
            })
    }

    fn render_cached_markdown(
        &self,
        key: &str,
        text: &str,
        theme: Theme,
        scope: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let (blocks, flattened) = {
            let mut cache = self.markdown_cache.borrow_mut();
            cached_markdown(&mut cache, key, text)
        };
        self.register_text(scope, flattened.text.clone());
        self.render_markdown_blocks(&blocks, &flattened, theme, scope, cx)
    }

    fn render_markdown_blocks(
        &self,
        blocks: &[MarkdownBlock],
        flattened: &FlattenedMarkdown,
        theme: Theme,
        scope: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let mut ranges = flattened.ranges.iter().cloned();
        let mut content = div()
            .debug_selector(|| "acp-markdown".into())
            .min_w_0()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .whitespace_normal()
            .text_size(ui_px(BODY_SIZE))
            .line_height(ui_px(BODY_LINE));
        for block in blocks {
            content = content.child(match block {
                MarkdownBlock::Paragraph(text) => {
                    div()
                        .text_size(ui_px(BODY_SIZE))
                        .line_height(ui_px(BODY_LINE))
                        .child(self.render_selectable_text(
                            scope,
                            text,
                            ranges.next().flatten().unwrap_or_default(),
                            theme,
                            cx,
                        ))
                }
                MarkdownBlock::Heading { level, text } => div()
                    .text_size(ui_px(match level {
                        1 => 18.,
                        2 => 16.,
                        _ => 14.,
                    }))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(self.render_selectable_text(
                        scope,
                        text,
                        ranges.next().flatten().unwrap_or_default(),
                        theme,
                        cx,
                    )),
                MarkdownBlock::Code { language, text } => {
                    let range = ranges.next().flatten().unwrap_or_default();
                    let id = format!("{scope}:code:{}", range.start);
                    let syntax = self.code_cache.borrow_mut().highlights(&id, text, language.as_deref(), theme);
                    let scroll = {
                        let mut scrolls = self.code_scrolls.borrow_mut();
                        if scrolls.len() >= 128 && !scrolls.contains_key(&id) { scrolls.clear(); }
                        scrolls.entry(id.clone()).or_default().clone()
                    };
                    let copy_text = text.clone();
                    div()
                        .min_w_0().w_full().flex().flex_col()
                        .border_1().border_color(rgba(theme.line)).bg(rgba(theme.bg0))
                        .child(div().min_w_0().flex().items_center().justify_between().px_3()
                            .child(meta(theme, language.clone().unwrap_or_default()).min_w_0().flex_1().overflow_hidden().text_ellipsis())
                            .child(icon_button(format!("{id}:copy"), i18n::text(self.language, "acp.copy_response"), theme)
                                .on_click(move |_, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy_text.clone())))
                                .child(panel_icon(COPY_ICON, theme.fg2))))
                        .child(div().id(gpui::ElementId::Name(id.into())).debug_selector(|| "acp-code-scroll".into()).min_w_0().w_full().flex().overflow_x_scroll().track_scroll(&scroll)
                            .px_3().pb_2().font_family("monospace")
                            .text_size(ui_px(CODE_SIZE)).line_height(ui_px(CODE_LINE)).whitespace_nowrap()
                            .child(self.render_text_with_syntax(scope, text, range, theme, &syntax, cx).w_auto().flex_none()))
                },
                MarkdownBlock::ListItem(text) => {
                    div()
                        .flex()
                        .gap_2()
                        .child("▪")
                        .child(self.render_selectable_text(
                            scope,
                            text,
                            ranges.next().flatten().unwrap_or_default(),
                            theme,
                            cx,
                        ))
                }
                MarkdownBlock::Quote(text) => div()
                    .border_l_2()
                    .border_color(rgba(theme.line))
                    .pl_3()
                    .text_color(rgba(theme.fg1))
                    .child(self.render_selectable_text(
                        scope,
                        text,
                        ranges.next().flatten().unwrap_or_default(),
                        theme,
                        cx,
                    )),
                MarkdownBlock::Rule => {
                    let _ = ranges.next();
                    div().h(ui_px(1.)).w_full().bg(rgba(theme.line))
                }
            }.min_w_0().flex_shrink(1.));
        }
        content
    }

    fn render_image(
        &self,
        data: &str,
        mime_type: &str,
        theme: Theme,
        scope: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let fingerprint = self
            .image_fingerprints
            .get(scope)
            .copied()
            .unwrap_or_else(|| image_fingerprint(mime_type, data));
        let valid_input =
            image_format(mime_type).is_some() && data.len() <= MAX_ENCODED_IMAGE_BYTES;
        let (state, generation) = {
            let mut cache = self.image_cache.borrow_mut();
            if let Some(entry) = cache
                .get(scope)
                .filter(|entry| entry.fingerprint == fingerprint)
            {
                (entry.state.clone(), None)
            } else {
                let generation = self.next_image_generation();
                let state = if valid_input {
                    ImageCacheState::Loading
                } else {
                    ImageCacheState::Error(image_error_text(mime_type))
                };
                insert_image_cache_entry(&mut cache, scope, fingerprint, generation, state.clone());
                (state, valid_input.then_some(generation))
            }
        };
        if let Some(generation) = generation {
            self.schedule_image_decode(
                scope.to_string(),
                data.to_string(),
                mime_type.to_string(),
                fingerprint,
                generation,
                cx,
            );
        }
        match state {
            ImageCacheState::Loading => div()
                .w(ui_px(48.))
                .h(ui_px(24.))
                .border_1()
                .border_color(rgba(theme.line))
                .bg(rgba(theme.bg1)),
            ImageCacheState::Ready(image) => div()
                .border_1()
                .border_color(rgba(theme.line))
                .child(img(image).max_w_full().max_h(ui_px(BODY_MAX_HEIGHT))),
            ImageCacheState::Error(error) => div()
                .pl_2()
                .border_l_2()
                .border_color(rgba(theme.red))
                .text_color(rgba(theme.fg1))
                .child(error),
        }
    }

    fn next_image_generation(&self) -> u64 {
        let generation = self.image_generation.get().wrapping_add(1);
        self.image_generation.set(generation);
        generation
    }

    fn schedule_image_decode(
        &self,
        scope: String,
        data: String,
        mime_type: String,
        fingerprint: u64,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let decode_mime_type = mime_type.clone();
            let result = cx
                .background_spawn(async move { decode_image(&data, &decode_mime_type) })
                .await;
            this.update(cx, |view, cx| {
                let mut image_cache = view.image_cache.borrow_mut();
                let Some(entry) = image_cache.get_mut(&scope) else {
                    return;
                };
                if !image_cache_entry_matches(entry, fingerprint, generation) {
                    return;
                }
                entry.state = match result {
                    Ok(bytes) => match image_format(&mime_type) {
                        Some(format) => ImageCacheState::Ready(std::sync::Arc::new(
                            Image::from_bytes(format, bytes),
                        )),
                        None => ImageCacheState::Error(image_error_text(&mime_type)),
                    },
                    Err(_) => ImageCacheState::Error(image_error_text(&mime_type)),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn render_tool_content(
        &self,
        content: &ToolContent,
        theme: Theme,
        scope: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        match content {
            ToolContent::Text(text) => self.render_cached_markdown(scope, text, theme, scope, cx),
            ToolContent::Image { data, mime_type } => {
                self.render_image(data, mime_type, theme, scope, cx)
            }
            ToolContent::Audio { data, mime_type } => div().text_color(rgba(theme.fg1)).child(
                format!("Audio · {mime_type} · {} bytes encoded", data.len()),
            ),
            ToolContent::Resource { uri, text } => div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_color(rgba(theme.accent)).child(uri.clone()))
                .when_some(text.clone(), |resource, text| {
                    resource.child(self.render_cached_markdown(scope, &text, theme, scope, cx))
                }),
            ToolContent::ResourceLink { name, uri } => div()
                .flex()
                .gap_2()
                .text_color(rgba(theme.accent))
                .child(name.clone())
                .child(uri.clone()),
            ToolContent::Diff {
                path,
                old_text,
                new_text,
            } => self.render_diff(path, old_text.as_deref(), new_text, theme, scope, cx),
            ToolContent::Terminal { id } => self.render_terminal(id, theme, scope, cx),
            ToolContent::Unknown(value) => self.render_raw("Content", value, theme, scope, cx),
        }
    }

    fn render_raw(
        &self,
        label: &str,
        value: &serde_json::Value,
        theme: Theme,
        scope: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let expanded = self.raw_expanded.contains(scope);
        let toggle_scope = scope.to_string();
        let mut view = div().min_w_0().child(
            semantic_button(format!("{scope}:toggle"), label.to_string(), theme)
                .flex().items_center().py_1()
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.raw_expanded.remove(&toggle_scope) {
                        this.raw_expanded.insert(toggle_scope.clone());
                    }
                    cx.notify();
                }))
                .child(disclosure_glyph(theme, expanded))
                .child(label.to_string()),
        );
        if expanded {
            let text = self.raw_cache.borrow_mut().entry(scope.to_string()).or_insert_with(|| {
                tool_display::bounded_text(&serde_json::to_string_pretty(value).unwrap_or_default(), 16 * 1024)
            }).clone();
            let raw = value.clone();
            view = view.child(icon_button(format!("{scope}:copy"), "Copy raw data", theme)
                .on_click(move |_, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(raw.to_string())))
                .child(panel_icon(COPY_ICON, theme.fg2)))
                .child(self.render_plain_code(scope, &text, theme, cx));
        }
        view
    }

    fn code_scroll(&self, scope: &str) -> gpui::ScrollHandle {
        let mut scrolls = self.code_scrolls.borrow_mut();
        if scrolls.len() >= 128 && !scrolls.contains_key(scope) { scrolls.clear(); }
        scrolls.entry(scope.into()).or_default().clone()
    }

    fn render_plain_code(&self, scope: &str, text: &str, theme: Theme, cx: &mut Context<Self>) -> gpui::Div {
        self.register_text(scope, text.to_string());
        let scroll = self.code_scroll(scope);
        div().min_w_0().w_full().child(div().id(format!("{scope}:scroll")).debug_selector(|| "acp-tool-code-scroll".into()).min_w_0().w_full().max_h(ui_px(TERMINAL_MAX_HEIGHT))
            .track_scroll(&scroll).flex()
            .overflow_y_scroll().overflow_x_scroll().px_3().py_2()
            .font_family("monospace").text_size(ui_px(CODE_SIZE)).line_height(ui_px(CODE_LINE))
            .whitespace_nowrap()
            .child(self.render_selectable_text(scope, text, 0..text.len(), theme, cx).w_auto().flex_none()))
    }

    fn render_diff(
        &self,
        path: &str,
        old_text: Option<&str>,
        new_text: &str,
        theme: Theme,
        scope: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let key = diff_key(scope, path);
        let decision = self.diff_decisions.get(&key).copied();
        let preview = self.diff_cache.borrow_mut().get(scope, old_text.unwrap_or_default(), new_text);
        self.register_text(scope, preview.text.clone());
        let mut lines = div().w_auto().flex_none().flex().flex_col();
        for row in &preview.rows {
            let color = match row.tag {
                Some(similar::ChangeTag::Insert) => theme.green,
                Some(similar::ChangeTag::Delete) => theme.red,
                _ => theme.bg0,
            };
            lines = lines.child(div().px_3().bg(rgba(Theme::with_alpha(color, TINT_ALPHA)))
                .child(self.render_selectable_text(scope, &preview.text[row.range.clone()].trim_end_matches('\n'), row.range.clone(), theme, cx).w_auto().flex_none()));
        }
        let copy_old = old_text.unwrap_or_default().to_string();
        let copy_new = new_text.to_string();
        let keep_key = key.clone();
        let reject_key = key;
        let reject_path = path.to_string();
        let reject_old = old_text.map(str::to_string);
        let reject_new = new_text.to_string();
        div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(rgba(theme.line))
            .child(
                div()
                    .px_3()
                    .py_1()
                    .bg(rgba(theme.bg2))
                    .text_size(ui_px(META_SIZE))
                    .text_color(rgba(theme.fg1))
                    .child(path.to_string())
                    .child(meta(theme, if preview.limited && preview.added == 0 && preview.removed == 0 {
                        "Diff preview unavailable".into()
                    } else {
                        format!("+{} -{}{}", preview.added, preview.removed, if preview.limited { " | Preview truncated" } else { "" })
                    })),
            )
            .child(div().id(format!("{scope}:diff-scroll")).debug_selector(|| "acp-tool-diff-scroll".into()).track_scroll(&self.code_scroll(scope)).min_w_0().w_full().max_h(ui_px(BODY_MAX_HEIGHT))
                .flex().overflow_x_scroll().overflow_y_scroll().font_family("monospace")
                .text_size(ui_px(CODE_SIZE)).line_height(ui_px(CODE_LINE)).whitespace_nowrap().child(lines))
            .child(div().flex().flex_wrap().gap_2().px_3()
                .child(button(format!("{scope}:copy-old"), "Copy original", theme)
                    .on_click(move |_, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy_old.clone())))
                    .child("Copy original"))
                .child(button(format!("{scope}:copy-new"), "Copy new", theme)
                    .on_click(move |_, _, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy_new.clone())))
                    .child("Copy new")))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .when(decision.is_none(), |actions| {
                        actions
                            .child(
                                primary_button(
                                    gpui::ElementId::Name(format!("diff-keep-{scope}-{path}").into()),
                                    i18n::text(self.language, "acp.diff_keep"),
                                    theme,
                                    theme.green,
                                )
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    this.keep_diff(keep_key.clone(), cx)
                                }))
                                .child(i18n::text(self.language, "acp.diff_keep")),
                            )
                            .child(
                                secondary_button(
                                    gpui::ElementId::Name(format!("diff-reject-{scope}-{path}").into()),
                                    i18n::text(self.language, "acp.diff_reject"),
                                    theme,
                                    theme.red,
                                )
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    this.reject_diff(
                                        reject_key.clone(),
                                        reject_path.clone(),
                                        reject_old.clone(),
                                        reject_new.clone(),
                                        cx,
                                    )
                                }))
                                .child(i18n::text(self.language, "acp.diff_reject")),
                            )
                    })
                    .when_some(decision, |actions, decision| {
                        actions.child(meta(
                            theme,
                            match decision {
                                DiffDecision::Kept => i18n::text(self.language, "acp.diff_kept"),
                                DiffDecision::Rejected => {
                                    i18n::text(self.language, "acp.diff_rejected")
                                }
                            },
                        ))
                    }),
            )
    }

    fn render_terminal(
        &self,
        id: &str,
        theme: Theme,
        scope: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        use muxlane_acp::TerminalOutputState;
        let terminal_id = id.to_string();
        let output = match self.terminal_outputs.get(id) {
            Some(TerminalOutputState::Pending) => i18n::text(self.language, "acp.terminal_loading").to_string(),
            None | Some(TerminalOutputState::Unavailable) => i18n::text(self.language, "acp.terminal_unavailable").to_string(),
            Some(TerminalOutputState::Failed(error)) => format!("{}: {error}", i18n::text(self.language, "acp.terminal_failed")),
            Some(TerminalOutputState::Ready(snapshot)) => {
                if snapshot.output.is_empty() {
                    i18n::text(self.language, "acp.terminal_empty").into()
                } else {
                    tool_display::bounded_text(&snapshot.output, 64 * 1024)
                }
            }
        };
        let snapshot = match self.terminal_outputs.get(id) {
            Some(TerminalOutputState::Ready(snapshot)) => Some(snapshot),
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .border_1()
            .border_color(rgba(theme.line))
            .when_some(snapshot.and_then(|snapshot| snapshot.command.as_ref()), |view, command| {
                view.child(self.render_plain_code(&format!("{scope}:command"), &tool_display::command_text(command), theme, cx))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .pl_3()
                    .pr_1()
                    .bg(rgba(theme.bg2))
                    .text_size(ui_px(META_SIZE))
                    .text_color(rgba(theme.fg1))
                    .child(format!("Output | {id}"))
                    .child(
                        icon_button(
                            gpui::ElementId::Name(format!("terminal-refresh-{id}").into()),
                            i18n::text(self.language, "acp.terminal_refresh"),
                            theme,
                        )
                        .ml_auto()
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.poll_terminal(terminal_id.clone(), cx)
                        }))
                        .child("↻"),
                    ),
            )
            .child(self.render_plain_code(scope, &output, theme, cx))
            .when_some(snapshot, |view, snapshot| {
                let copy_id = id.to_string();
                view.child(div().px_3().flex().items_center().justify_between()
                    .child(meta(theme, tool_display::terminal_status(snapshot)))
                    .child(icon_button(format!("{scope}:copy-output"), "Copy retained output", theme)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(TerminalOutputState::Ready(snapshot)) = this.terminal_outputs.get(&copy_id) {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(snapshot.output.clone()));
                            }
                        }))
                        .child(panel_icon(COPY_ICON, theme.fg2))))
            })
    }

    fn keep_diff(&mut self, key: (String, String), cx: &mut Context<Self>) {
        self.diff_decisions.insert(key, DiffDecision::Kept);
        cx.notify();
    }

    fn reject_diff(
        &mut self,
        key: (String, String),
        path: String,
        old_text: Option<String>,
        new_text: String,
        cx: &mut Context<Self>,
    ) {
        match reject_diff_change(&self.project_path, &path, old_text.as_deref(), &new_text) {
            Ok(()) => {
                self.diff_decisions.insert(key, DiffDecision::Rejected);
                cx.notify();
            }
            Err(error) => self.parent_error(error, cx),
        }
    }

    fn toggle_expanded(&mut self, cx: &mut Context<Self>) {
        self.expanded = !self.expanded;
        cx.notify();
    }

    fn poll_terminal(&mut self, id: String, cx: &mut Context<Self>) {
        let parent = self.parent.clone();
        cx.spawn(async move |_, cx| {
            parent.update(cx, |view, cx| view.poll_terminal(id, cx)).ok();
        }).detach();
    }

    fn open_subagent(&mut self, session_id: String, cx: &mut Context<Self>) {
        self.parent
            .update(cx, |_, cx| {
                cx.emit(super::AcpViewEvent::OpenSubagent(session_id))
            })
            .ok();
    }

    fn resume_follow_tail(&mut self, cx: &mut Context<Self>) {
        self.parent
            .update(cx, |view, cx| view.resume_follow_tail(cx))
            .ok();
    }

    fn parent_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.parent
            .update(cx, |view, cx| {
                view.push_entry(Entry::Error(error));
                cx.notify();
            })
            .ok();
    }

    fn render_thread_item(&self, cx: &mut Context<Self>) -> gpui::Div {
        let theme = self.theme();
        match &self.item {
            ThreadItem::Message(message) => match message.role {
                MessageRole::User => user_card(theme).child(self.render_cached_markdown(
                    &format!("message:{}", message.id),
                    &message.text,
                    theme,
                    &format!("message:{}", message.id),
                    cx,
                )),
                MessageRole::Assistant => {
                    let body = self.render_cached_markdown(
                        &format!("message:{}", message.id),
                        &message.text,
                        theme,
                        &format!("message:{}", message.id),
                        cx,
                    );
                    let body_text = message.text.clone();
                    let id = message.id.clone();
                    div()
                        .debug_selector(|| "acp-assistant-message".into())
                        .relative()
                        .group("msg")
                        .pl(ui_px(CONTENT_INSET))
                        .pr(ui_px(76.))
                        .min_h(ui_px(36.))
                        .py_1()
                        .text_size(ui_px(BODY_SIZE))
                        .line_height(ui_px(BODY_LINE))
                        .text_color(rgba(theme.fg0))
                        .child(body)
                        .child(
                            div()
                                .absolute()
                                .top_1()
                                .right_3()
                                .flex()
                                .justify_end()
                                .gap_1()
                                .child(
                                    icon_button(
                                        gpui::ElementId::Name(format!("acp-copy-{id}").into()),
                                        i18n::text(self.language, "acp.copy_response"),
                                        theme,
                                    )
                                    .debug_selector(|| "acp-copy-response".into())
                                    .opacity(0.)
                                    .group_hover("msg", |style| style.opacity(1.))
                                    .focus(|style| style.opacity(1.))
                                    .in_focus(|style| style.opacity(1.))
                                    .on_click(cx.listener(move |_this, _event, _window, cx| {
                                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                            body_text.clone(),
                                        ))
                                    }))
                                    .child(panel_icon(COPY_ICON, theme.fg2)),
                                )
                                .child(
                                    icon_button(
                                        gpui::ElementId::Name(format!("acp-bottom-{id}").into()),
                                        i18n::text(self.language, "acp.scroll_bottom"),
                                        theme,
                                    )
                                    .opacity(0.)
                                    .group_hover("msg", |style| style.opacity(1.))
                                    .focus(|style| style.opacity(1.))
                                    .in_focus(|style| style.opacity(1.))
                                    .on_click(cx.listener(|this, _event, _window, cx| {
                                        this.resume_follow_tail(cx)
                                    }))
                                    .child(panel_icon(ARROW_DOWN_ICON, theme.fg2)),
                                ),
                        )
                }
            },
            ThreadItem::Content(content) => {
                let scope = format!("content:{}", content.id);
                let body = self.render_tool_content(&content.content, theme, &scope, cx);
                match content.role {
                    MessageRole::User => user_card(theme).child(body),
                    MessageRole::Assistant => div()
                        .pl(ui_px(CONTENT_INSET))
                        .pr_3()
                        .py_2()
                        .text_size(ui_px(BODY_SIZE))
                        .line_height(ui_px(BODY_LINE))
                        .child(body),
                }
            }
            ThreadItem::Thought(thought) => {
                let copy_text = thought.text.clone();
                let id = thought.id.clone();
                let body = self.render_cached_markdown(
                    &format!("thought:{id}"),
                    &thought.text,
                    theme,
                    &format!("thought:{id}"),
                    cx,
                );
                div()
                    .my_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .min_w_0()
                            .pl(ui_px(GUTTER_PAD))
                            .pr_3()
                            .child(
                                semantic_button(
                                    gpui::ElementId::Name(
                                        format!("acp-thought-toggle-{id}").into(),
                                    ),
                                    i18n::text(self.language, "acp.thought"),
                                    theme,
                                )
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .px_0()
                                .py_1()
                                .text_size(ui_px(BODY_SIZE))
                                .line_height(ui_px(BODY_LINE))
                                .text_color(rgba(theme.fg1))
                                .on_click(
                                    cx.listener(|this, _event, _window, cx| {
                                        this.toggle_expanded(cx)
                                    }),
                                )
                                .child(disclosure_glyph(theme, self.expanded))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(i18n::text(self.language, "acp.thinking")),
                                ),
                            )
                            .child(
                                icon_button(
                                    gpui::ElementId::Name(format!("acp-thought-copy-{id}").into()),
                                    i18n::text(self.language, "acp.copy_thought"),
                                    theme,
                                )
                                .on_click(cx.listener(move |_this, _event, _window, cx| {
                                    cx.stop_propagation();
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        copy_text.clone(),
                                    ));
                                }))
                                .child(panel_icon(COPY_ICON, theme.fg2)),
                            ),
                    )
                    .when(self.expanded, |view| {
                        view.child(
                            nested_body(theme)
                                .id(format!("acp-thought-body-{id}"))
                                .max_h(ui_px(BODY_MAX_HEIGHT))
                                .overflow_y_scroll()
                                .child(body),
                        )
                    })
            }
            ThreadItem::Tool(tool) => {
                let id = tool.id.clone();
                let state_color = match tool.state {
                    muxlane_acp::ToolState::Failed | muxlane_acp::ToolState::Rejected => theme.red,
                    muxlane_acp::ToolState::Completed => theme.green,
                    muxlane_acp::ToolState::Running | muxlane_acp::ToolState::Pending => theme.yellow,
                    _ => theme.fg2,
                };
                let mut view = div().my_1().child(
                    div()
                        .flex()
                        .items_center()
                        .min_w_0()
                        .pl(ui_px(GUTTER_PAD))
                        .pr_3()
                        .child(
                            semantic_button(
                                gpui::ElementId::Name(format!("acp-tool-toggle-{id}").into()),
                                tool.title.clone(),
                                theme,
                            )
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .px_0()
                            .py_1()
                            .text_size(ui_px(BODY_SIZE))
                            .line_height(ui_px(BODY_LINE))
                            .text_color(rgba(theme.fg1))
                            .on_click(
                                cx.listener(|this, _event, _window, cx| this.toggle_expanded(cx)),
                            )
                            .child(disclosure_glyph(theme, self.expanded))
                            .child(panel_icon(tool_display::kind_icon(&tool.kind), theme.fg2))
                            .child(
                                div()
                                    .pl_2()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(tool_display::tool_title(tool)),
                            ),
                        )
                        .child(div().flex_none().px_2().text_size(ui_px(META_SIZE)).text_color(rgba(state_color))
                            .child(tool_display::state_label(&tool.state)))
                        .child(
                            icon_button(
                                gpui::ElementId::Name(format!("acp-tool-copy-{id}").into()),
                                i18n::text(self.language, "acp.copy_tool"),
                                theme,
                            )
                            .on_click(cx.listener(move |this, _event, _window, cx| {
                                cx.stop_propagation();
                                if let ThreadItem::Tool(tool) = &this.item {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(tool_copy_text(tool)));
                                }
                            }))
                            .child(panel_icon(COPY_ICON, theme.fg2)),
                        ),
                );
                if let Some(session_id) = tool.subagent_session_id.clone() {
                    view = view.child(
                        div().flex().justify_end().py_1().pr_3().child(
                            button(
                                gpui::ElementId::Name(format!("open-subagent-{}", tool.id).into()),
                                i18n::text(self.language, "acp.open_subagent"),
                                theme,
                            )
                            .px_3()
                            .py_1()
                            .border_color(rgba(theme.line))
                            .text_color(rgba(theme.fg1))
                            .on_click(cx.listener(move |this, _event, _window, cx| {
                                this.open_subagent(session_id.clone(), cx)
                            }))
                            .child(i18n::text(self.language, "acp.open_subagent")),
                        ),
                    );
                }
                if self.expanded {
                    let mut expanded = nested_body(theme)
                        .child(meta(theme, tool_display::kind_label(&tool.kind)))
                        .child(self.render_plain_code(&format!("tool:{id}:title"), &tool.title, theme, cx));
                    for location in &tool.locations {
                        expanded = expanded.child(
                            div()
                                .py_1()
                                .text_size(ui_px(META_SIZE))
                                .text_color(rgba(theme.accent))
                                .child(match location.line {
                                    Some(line) => format!("{}:{line}", location.path),
                                    None => location.path.clone(),
                                }),
                        );
                    }
                    for (index, content) in tool.content.iter().enumerate() {
                        let scope = format!("tool:{id}:content:{index}");
                        expanded = expanded.child(
                            div()
                                .id(format!("acp-tool-content-{id}-{index}"))
                                .py_1()
                                .min_w_0()
                                .max_h(ui_px(BODY_MAX_HEIGHT))
                                .overflow_y_scroll()
                                .child(self.render_tool_content(content, theme, &scope, cx)),
                        );
                    }
                    if let Some(input) = &tool.raw_input {
                        expanded = expanded.child(self.render_raw("Raw input", input, theme, &format!("tool:{id}:input"), cx));
                    }
                    if let Some(output) = &tool.raw_output {
                        expanded = expanded.child(self.render_raw("Raw output", output, theme, &format!("tool:{id}:output"), cx));
                    }
                    view = view.child(expanded);
                }
                view
            }
        }
    }
}

/// 用户消息 / 用户内容统一卡片：左 accent 色条 + bg1，文字落在 `CONTENT_INSET`。
fn user_card(theme: Theme) -> gpui::Div {
    div()
        .mx(ui_px(CARD_MARGIN))
        .my_2()
        .pl(ui_px(BAR_PAD))
        .pr(ui_px(CARD_PAD))
        .py(ui_px(CARD_PAD))
        .border_l_2()
        .border_color(rgba(theme.accent))
        .bg(rgba(theme.bg0))
        .text_size(ui_px(BODY_SIZE))
        .line_height(ui_px(BODY_LINE))
        .text_color(rgba(theme.fg0))
}

/// 折叠行字形槽：▸ 收起 / ▾ 展开。
fn disclosure_glyph(theme: Theme, expanded: bool) -> gpui::Div {
    div()
        .w(ui_px(GUTTER_WIDTH))
        .flex_none()
        .text_color(rgba(theme.fg2))
        .child(if expanded { "▾" } else { "▸" })
}

/// 折叠体：嵌套竖线，文字回到 `CONTENT_INSET`。
fn nested_body(theme: Theme) -> gpui::Div {
    div()
        .ml(ui_px(NEST_LINE_X))
        .mr_3()
        .pl(ui_px(NEST_PAD))
        .pb_2()
        .min_w_0()
        .border_l_1()
        .border_color(rgba(theme.line))
        .text_size(ui_px(BODY_SIZE))
        .line_height(ui_px(BODY_LINE))
        .text_color(rgba(theme.fg1))
}

impl Render for TimelineItemView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.texts.borrow_mut().clear();
        self.layouts.borrow_mut().clear();
        self.render_thread_item(cx)
            .min_w_0()
            .flex_shrink(1.)
            .track_focus(&self.focus)
            .on_key_down(
                cx.listener(|this, event: &gpui::KeyDownEvent, _window, cx| {
                    let modifiers = event.keystroke.modifiers;
                    if event.keystroke.key.as_str() == "c"
                        && (modifiers.control || modifiers.platform)
                        && !modifiers.alt
                        && this.copy_selection(cx)
                    {
                        cx.stop_propagation();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                this.timeline_mouse_move(event.position, window, cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseUpEvent, _window, _cx| {
                    this.selection.end_drag();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _event: &MouseUpEvent, _window, _cx| this.selection.end_drag()),
            )
    }
}

fn content_contains_terminal(content: &ToolContent, id: &str) -> bool {
    matches!(content, ToolContent::Terminal { id: terminal_id } if terminal_id == id)
}

fn tool_copy_text(tool: &muxlane_acp::Tool) -> String {
    let mut output = format!("{}\n", tool.title);
    if let Some(input) = &tool.raw_input {
        output.push_str("Input: ");
        output.push_str(&input.to_string());
        output.push('\n');
    }
    for content in &tool.content {
        match content {
            ToolContent::Text(text) => output.push_str(text),
            ToolContent::Resource { uri, text } => {
                output.push_str(uri);
                if let Some(text) = text {
                    output.push('\n');
                    output.push_str(text);
                }
            }
            ToolContent::ResourceLink { name, uri } => {
                output.push_str(name);
                output.push(' ');
                output.push_str(uri);
            }
            ToolContent::Terminal { id } => output.push_str(id),
            ToolContent::Diff {
                path,
                old_text,
                new_text,
            } => {
                output.push_str(path);
                output.push('\n');
                if let Some(old_text) = old_text {
                    output.push_str(old_text);
                    output.push('\n');
                }
                output.push_str(new_text);
            }
            ToolContent::Audio { mime_type, .. } | ToolContent::Image { mime_type, .. } => {
                output.push_str(mime_type)
            }
            ToolContent::Unknown(value) => output.push_str(&value.to_string()),
        }
        output.push('\n');
    }
    if let Some(output_value) = &tool.raw_output {
        output.push_str("Output: ");
        output.push_str(&output_value.to_string());
    }
    output
}

fn diff_key(scope: &str, path: &str) -> (String, String) {
    (scope.to_string(), path.to_string())
}

pub(crate) fn reject_diff_change(
    project_root: &std::path::Path,
    path: &str,
    old_text: Option<&str>,
    new_text: &str,
) -> Result<(), String> {
    let root = std::fs::canonicalize(project_root)
        .map_err(|error| format!("{}: {error}", project_root.display()))?;
    let requested = std::path::Path::new(path);
    if !requested.is_absolute() {
        return Err("diff path must be absolute".into());
    }
    let current_path =
        std::fs::canonicalize(requested).map_err(|error| format!("{path}: {error}"))?;
    if current_path != root && !current_path.starts_with(&root) {
        return Err("diff path escapes the project root".into());
    }
    let current =
        std::fs::read_to_string(&current_path).map_err(|error| format!("{path}: {error}"))?;
    if current != new_text {
        return Err("file changed after the agent diff; refusing to overwrite it".into());
    }
    match old_text {
        Some(old_text) => {
            let temporary = current_path.with_extension(format!(
                "muxlane-reject-{}.tmp",
                muxlane_core::model::new_id("diff")
            ));
            std::fs::write(&temporary, old_text)
                .map_err(|error| format!("{}: {error}", temporary.display()))?;
            if let Err(error) = std::fs::rename(&temporary, &current_path) {
                std::fs::remove_file(&temporary)
                    .map_err(|cleanup| format!("{}: {cleanup}", temporary.display()))?;
                return Err(format!("{}: {error}", current_path.display()));
            }
            Ok(())
        }
        None => std::fs::remove_file(&current_path)
            .map_err(|error| format!("{}: {error}", current_path.display())),
    }
}

const MAX_ENCODED_IMAGE_BYTES: usize = 14 * 1024 * 1024;
const MAX_DECODED_IMAGE_BYTES: usize = 10 * 1024 * 1024;

fn image_fingerprint(mime_type: &str, data: &str) -> u64 {
    let mut fingerprint = 14695981039346656037_u64;
    for byte in mime_type.bytes().chain([0]).chain(data.bytes()) {
        fingerprint ^= u64::from(byte);
        fingerprint = fingerprint.wrapping_mul(1099511628211);
    }
    fingerprint
}

fn image_fingerprints(item: &ThreadItem) -> HashMap<String, u64> {
    let mut fingerprints = HashMap::new();
    match item {
        ThreadItem::Content(item) => {
            if let ToolContent::Image { data, mime_type } = &item.content {
                fingerprints.insert(
                    format!("content:{}", item.id),
                    image_fingerprint(mime_type, data),
                );
            }
        }
        ThreadItem::Tool(item) => {
            for (index, content) in item.content.iter().enumerate() {
                if let ToolContent::Image { data, mime_type } = content {
                    fingerprints.insert(
                        format!("tool:{}:content:{index}", item.id),
                        image_fingerprint(mime_type, data),
                    );
                }
            }
        }
        _ => {}
    }
    fingerprints
}

fn markdown_scopes(item: &ThreadItem) -> HashSet<String> {
    let mut scopes = HashSet::new();
    match item {
        ThreadItem::Message(item) => {
            scopes.insert(format!("message:{}", item.id));
        }
        ThreadItem::Thought(item) => {
            scopes.insert(format!("thought:{}", item.id));
        }
        ThreadItem::Content(item) => {
            if matches!(
                item.content,
                ToolContent::Text(_) | ToolContent::Resource { text: Some(_), .. }
            ) {
                scopes.insert(format!("content:{}", item.id));
            }
        }
        ThreadItem::Tool(item) => {
            for (index, content) in item.content.iter().enumerate() {
                if matches!(
                    content,
                    ToolContent::Text(_) | ToolContent::Resource { text: Some(_), .. }
                ) {
                    scopes.insert(format!("tool:{}:content:{index}", item.id));
                }
            }
        }
    }
    scopes
}

fn image_error_text(mime_type: &str) -> String {
    format!("Unsupported or oversized image · {mime_type}")
}

fn decode_image(data: &str, mime_type: &str) -> Result<Vec<u8>, String> {
    if image_format(mime_type).is_none() {
        return Err(image_error_text(mime_type));
    }
    if data.len() > MAX_ENCODED_IMAGE_BYTES {
        return Err(image_error_text(mime_type));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| image_error_text(mime_type))?;
    if bytes.len() > MAX_DECODED_IMAGE_BYTES {
        return Err(image_error_text(mime_type));
    }
    Ok(bytes)
}

fn insert_image_cache_entry(
    cache: &mut HashMap<String, ImageCacheEntry>,
    scope: &str,
    fingerprint: u64,
    generation: u64,
    state: ImageCacheState,
) -> bool {
    if cache
        .get(scope)
        .is_some_and(|entry| entry.fingerprint == fingerprint)
    {
        return false;
    }
    cache.insert(
        scope.to_string(),
        ImageCacheEntry {
            fingerprint,
            generation,
            state,
        },
    );
    true
}

fn image_cache_entry_matches(entry: &ImageCacheEntry, fingerprint: u64, generation: u64) -> bool {
    entry.fingerprint == fingerprint && entry.generation == generation
}

fn image_format(mime_type: &str) -> Option<ImageFormat> {
    match mime_type {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" | "image/jpg" => Some(ImageFormat::Jpeg),
        "image/webp" => Some(ImageFormat::Webp),
        "image/gif" => Some(ImageFormat::Gif),
        "image/svg+xml" => Some(ImageFormat::Svg),
        "image/bmp" => Some(ImageFormat::Bmp),
        "image/tiff" => Some(ImageFormat::Tiff),
        "image/x-icon" | "image/vnd.microsoft.icon" => Some(ImageFormat::Ico),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(cx: &mut gpui::TestAppContext, window: gpui::AnyWindowHandle) {
        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx)).unwrap();
        cx.update_window(window, |_, window, cx| window.simulate_next_frame(cx)).unwrap();
    }

    #[test]
    fn response_overlay_copy_is_mouse_and_keyboard_reachable_without_overlapping_text() {
        use gpui::{px, size, TestAppContext, VisualTestContext};
        use muxlane_acp::{Event, ThreadDelta};
        let mut cx = TestAppContext::single();
        let window = cx.add_window(super::super::tests::terminal_test_view);
        let parent = window.root(&mut cx).unwrap();
        parent.update(&mut cx, |view, cx| {
            view.apply(Event::Delta(ThreadDelta::MessageChunk { id: None,
                role: MessageRole::Assistant, text: "你好，世界。".into() }), cx);
        });
        let item = cx.update(|cx| parent.read(cx).timeline_items[0].clone());
        let window: gpui::AnyWindowHandle = window.into();
        let mut visual = VisualTestContext::from_window(window, &mut cx);
        visual.simulate_resize(size(px(320.), px(640.)));
        for _ in 0..3 { draw(&mut cx, window); }
        let copy = visual.debug_bounds("acp-copy-response").unwrap();
        cx.update(|cx| {
            assert!(item.read(cx).layouts.borrow().iter().all(|text| text.bounds.right() <= copy.left()));
        });
        visual.simulate_mouse_move(copy.center(), None, Default::default());
        draw(&mut cx, window);
        visual.simulate_click(copy.center(), Default::default());
        assert_eq!(cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()), "你好，世界。");
        let mouse_copy_focus = cx.update_window(window, |_, window, cx| window.focused(cx)).unwrap();
        assert!(mouse_copy_focus.is_some(), "mouse click must focus the Copy button");
        visual.simulate_mouse_move(gpui::point(px(0.), px(600.)), None, Default::default());
        cx.update_window(window, |_, window, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string("cleared".into()));
            item.read(cx).focus.clone().focus(window, cx);
            window.focus_next(cx);
            assert_eq!(window.focused(cx), mouse_copy_focus, "focus_next must reach the Copy button");
        }).unwrap();
        draw(&mut cx, window);
        // GPUI synthesizes button clicks on release; simulate_keystrokes sends only KeyDown.
        visual.simulate_event(gpui::KeyDownEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
            is_held: false,
            prefer_character_input: false,
        });
        visual.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
        });
        assert_eq!(cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()), "你好，世界。");
    }

    #[test]
    fn focused_multiclick_notifies_paints_highlight_and_copies_exact_unicode() {
        use gpui::{point, px, TestAppContext, VisualTestContext};
        use muxlane_acp::{Event, ThreadDelta};
        let mut cx = TestAppContext::single();
        let window = cx.add_window(super::super::tests::terminal_test_view);
        let parent = window.root(&mut cx).unwrap();
        parent.update(&mut cx, |view, cx| {
            view.apply(Event::Delta(ThreadDelta::MessageChunk {
                id: Some("click".into()), role: MessageRole::Assistant,
                text: "prefix 中文 😀 suffix\n\nsecond paragraph".into(),
            }), cx);
        });
        let item = cx.update(|cx| parent.read(cx).timeline_items[0].clone());
        let window: gpui::AnyWindowHandle = window.into();
        let mut visual = VisualTestContext::from_window(window, &mut cx);
        for _ in 0..3 { draw(&mut cx, window); }
        cx.update_window(window, |_, window, cx| item.read(cx).focus.clone().focus(window, cx)).unwrap();
        draw(&mut cx, window);
        let notifications = Rc::new(Cell::new(0));
        let _subscription = cx.update(|cx| cx.observe(&item, {
            let notifications = notifications.clone();
            move |_, _| notifications.set(notifications.get() + 1)
        }));
        let scope = "message:click";
        for (count, expected) in [
            (2, "中文"),
            (3, "prefix 中文 😀 suffix\n"),
            (4, "prefix 中文 😀 suffix\nsecond paragraph"),
            (1, ""),
        ] {
            let position = cx.update(|cx| {
                let item = item.read(cx);
                let layouts = item.layouts.borrow();
                let record = &layouts[0];
                let index = record.content.find('中').unwrap();
                record.layout.position_for_index(index).unwrap() + point(px(1.), px(3.))
            });
            assert!(cx.update_window(window, |_, window, cx| item.read(cx).focus.is_focused(window)).unwrap());
            let previous = notifications.get();
            visual.simulate_event(MouseDownEvent {
                button: MouseButton::Left, position, click_count: count, ..Default::default()
            });
            assert!(notifications.get() > previous, "click count {count} must notify even when already focused");
            draw(&mut cx, window);
            item.update(&mut cx, |item, cx| {
                assert_eq!(item.selection.dragging, count == 1);
                assert_eq!(item.selection.selected_text(&item.texts.borrow()[scope]).unwrap_or_default(), expected);
                for record in item.layouts.borrow().iter() {
                    let selected: Vec<_> = record.painted_highlights.iter()
                        .filter(|(_, style)| style.background_color.is_some())
                        .map(|(range, _)| range.clone()).collect();
                    assert_eq!(selected, item.selection.highlight(scope, &record.range).into_iter().collect::<Vec<_>>());
                }
                cx.write_to_clipboard(gpui::ClipboardItem::new_string("unchanged".into()));
            });
            cx.simulate_keystrokes(window, "ctrl-c");
            cx.update(|cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), if expected.is_empty() { "unchanged" } else { expected }));
            visual.simulate_mouse_up(position, MouseButton::Left, Default::default());
            draw(&mut cx, window);
        }
    }

    #[test]
    fn resized_and_horizontally_scrolled_unicode_uses_current_geometry_for_exact_copy() {
        use gpui::{point, px, size, TestAppContext, VisualTestContext};
        use muxlane_acp::{Event, ThreadDelta};
        let mut cx = TestAppContext::single();
        let window = cx.add_window(super::super::tests::terminal_test_view);
        let parent = window.root(&mut cx).unwrap();
        parent.update(&mut cx, |view, cx| {
            view.apply(Event::Delta(ThreadDelta::MessageChunk {
                id: Some("geometry".into()), role: MessageRole::Assistant,
                text: format!("{}\n\n```rust\nlet value = \"{}末尾😀 done\";\n```\n\nfinal paragraph",
                    "prefix 中文 😀 suffix ".repeat(4), "long code ".repeat(30)),
            }), cx);
        });
        let item = cx.update(|cx| parent.read(cx).timeline_items[0].clone());
        let window: gpui::AnyWindowHandle = window.into();
        let mut visual = VisualTestContext::from_window(window, &mut cx);
        let mut previous: Option<(f32, Bounds<Pixels>)> = None;
        for width in [900., 320., 900.] {
            visual.simulate_resize(size(px(width), px(1200.)));
            for _ in 0..3 { draw(&mut cx, window); }
            let paragraph = cx.update(|cx| item.read(cx).assert_current_text_geometry());
            if let Some((old_width, old_bounds)) = previous {
                assert_eq!(paragraph.size.width < old_bounds.size.width, width < old_width);
                assert_eq!(paragraph.size.height > old_bounds.size.height, width < old_width);
            }
            previous = Some((width, paragraph));
            let (start, end, range) = cx.update(|cx| {
                let item = item.read(cx);
                let records = item.layouts.borrow();
                let paragraph = records.iter().find(|record| record.range.start == 0).unwrap();
                let index = paragraph.content.rfind('中').unwrap();
                let end_index = index + "中文 😀".len();
                let start = paragraph.layout.position_for_index(index).unwrap() + point(px(0.1), px(3.));
                let end = paragraph.layout.position_for_index(end_index).unwrap() + point(px(0.1), px(3.));
                let range = index..end_index;
                assert_eq!(timeline_point_to_offset(&records, &paragraph.scope, start), Some(range.start));
                assert_eq!(timeline_point_to_offset(&records, &paragraph.scope, end), Some(range.end));
                (start, end, range)
            });
            visual.simulate_mouse_down(start, MouseButton::Left, Default::default());
            visual.simulate_mouse_move(end, Some(MouseButton::Left), Default::default());
            visual.simulate_mouse_up(end, MouseButton::Left, Default::default());
            draw(&mut cx, window);
            cx.simulate_keystrokes(window, "ctrl-c");
            cx.update(|cx| {
                assert_eq!(item.read(cx).selection.normalized(), range);
                assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "中文 😀");
            });

            let scroll = cx.update(|cx| item.read(cx).code_scrolls.borrow().values().next().unwrap().clone());
            scroll.set_offset(point(px(0.), px(0.)));
            draw(&mut cx, window);
            let unscrolled = cx.update(|cx| {
                let item = item.read(cx);
                let records = item.layouts.borrow();
                let code = records.iter().find(|record| record.content.contains("末尾")).unwrap();
                code.layout.position_for_index(code.content.find('末').unwrap()).unwrap()
            });
            scroll.set_offset(point(-scroll.max_offset().x, px(0.)));
            draw(&mut cx, window);
            let viewport = visual.debug_bounds("acp-code-scroll").unwrap();
            let (start, end, range) = cx.update(|cx| {
                let item = item.read(cx);
                let records = item.layouts.borrow();
                let code = records.iter().find(|record| record.content.contains("末尾")).unwrap();
                let index = code.content.find('末').unwrap();
                assert!(index > 0);
                let end_index = index + "末尾😀".len();
                assert!(code.content.is_char_boundary(index) && code.content.is_char_boundary(end_index));
                let start = code.layout.position_for_index(index).unwrap();
                let end = code.layout.position_for_index(end_index).unwrap();
                assert!((start.x - unscrolled.x - scroll.offset().x).abs() < px(0.1), "painted geometry must move by the actual scroll offset");
                let start = start + point(px(0.1), px(3.));
                let end = end + point(px(0.1), px(3.));
                assert!(viewport.contains(&start) && viewport.contains(&end), "Unicode drag endpoints must be visible after scrolling");
                let range = code.range.start + index..code.range.start + end_index;
                assert_eq!(timeline_point_to_offset(&records, &code.scope, start), Some(range.start));
                assert_eq!(timeline_point_to_offset(&records, &code.scope, end), Some(range.end));
                (start, end, range)
            });
            visual.simulate_mouse_down(start, MouseButton::Left, Default::default());
            visual.simulate_mouse_move(end, Some(MouseButton::Left), Default::default());
            visual.simulate_mouse_up(end, MouseButton::Left, Default::default());
            draw(&mut cx, window);
            cx.simulate_keystrokes(window, "ctrl-c");
            cx.update(|cx| {
                let item = item.read(cx);
                assert_eq!(item.selection.normalized(), range);
                assert!(!item.selection.dragging);
                assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "末尾😀");
            });
        }
    }

    #[test]
    fn surrounding_word_ranges_handle_unicode_whitespace_and_boundaries() {
        assert_eq!(surrounding_word_range("hello world", 1), 0..5);
        assert_eq!(surrounding_word_range("foo  bar", 4), 3..5);
        assert_eq!(surrounding_word_range("中文😀!", 0), 0..6);
        assert_eq!(surrounding_word_range("中文😀!", 6), 6..10);
        assert_eq!(surrounding_word_range("中文😀!", 10), 10..11);
        assert_eq!(surrounding_word_range("", 0), 0..0);
        assert_eq!(surrounding_word_range("abc", 99), 0..3);
    }

    #[test]
    fn surrounding_line_ranges_include_newlines_and_stay_within_content() {
        let text = "first\n中😀\nlast";
        assert_eq!(surrounding_line_range(text, 1), 0..6);
        assert_eq!(surrounding_line_range(text, 8), 6..14);
        assert_eq!(surrounding_line_range(text, text.len()), 14..text.len());
        assert_eq!(surrounding_line_range("", 4), 0..0);
    }

    #[test]
    fn click_ranges_apply_to_the_whole_scope_and_keep_other_scopes_isolated() {
        let text = "first\n中😀\nlast";
        let mut selection = TimelineSelection::default();
        selection.select_range("message", surrounding_word_range(text, 8));
        assert_eq!(selection.selected_text(text).as_deref(), Some("中"));
        assert!(selection.highlight("other", &(0..text.len())).is_none());
        selection.select_range("message", surrounding_line_range(text, 8));
        assert_eq!(selection.selected_text(text).as_deref(), Some("中😀\n"));
        selection.select_range("message", 0..text.len());
        assert_eq!(selection.selected_text(text).as_deref(), Some(text));
    }

    #[test]
    fn timeline_selection_normalizes_forward_and_reverse_multibyte_ranges() {
        let text = "aé🙂b";
        let mut selection = TimelineSelection::default();
        selection.begin("message", 1);
        selection.extend("message", 7);
        assert_eq!(selection.normalized(), 1..7);
        assert_eq!(selection.selected_text(text).as_deref(), Some("é🙂"));
        selection.begin("message", 7);
        selection.extend("message", 1);
        assert_eq!(selection.normalized(), 1..7);
        assert_eq!(selection.selected_text(text).as_deref(), Some("é🙂"));
    }

    #[test]
    fn structural_diff_keys_keep_scopes_and_paths_distinct() {
        assert_ne!(
            diff_key("tool:one:content:0", "/tmp/a"),
            diff_key("tool:two:content:0", "/tmp/a")
        );
        assert_ne!(
            diff_key("tool:one:content:0", "/tmp/a"),
            diff_key("tool:one:content:0", "/tmp/b")
        );
    }

    #[test]
    fn transient_state_is_preserved_only_for_equal_items() {
        let item = ThreadItem::Thought(muxlane_acp::Thought {
            protocol_id: None,
            id: "thought-1".into(),
            text: "same".into(),
        });
        let changed_item = ThreadItem::Thought(muxlane_acp::Thought {
            protocol_id: None,
            id: "thought-1".into(),
            text: "changed".into(),
        });
        let mut state = TimelineItemTransientState {
            expanded: true,
            ..Default::default()
        };
        state.diff_decisions.insert(
            ("tool:one:content:0".into(), "/tmp/a".into()),
            DiffDecision::Rejected,
        );
        state.terminal_outputs.insert(
            "terminal-1".into(),
            muxlane_acp::TerminalOutputState::Ready(muxlane_acp::TerminalSnapshot {
                id: "terminal-1".into(),
                output: "output".into(),
                truncated: false,
                exit_code: None,
                command: None,
            }),
        );

        assert_eq!(
            preserve_transient_state(&item, &item, &state),
            Some(state.clone())
        );
        assert_eq!(preserve_transient_state(&item, &changed_item, &state), None);
    }

    #[test]
    fn markdown_cache_reuses_unchanged_documents_and_replaces_changed_sources() {
        let mut cache = HashMap::new();
        let (first, _) = cached_markdown(&mut cache, "scope", "hello");
        let (same, _) = cached_markdown(&mut cache, "scope", "hello");
        assert!(std::sync::Arc::ptr_eq(&first, &same));

        let (changed, flattened) = cached_markdown(&mut cache, "scope", "goodbye");
        assert!(!std::sync::Arc::ptr_eq(&first, &changed));
        assert_eq!(flattened.text, "goodbye");
    }

    #[test]
    fn image_fingerprints_are_stable_and_change_with_content() {
        assert_eq!(
            image_fingerprint("image/png", "abc"),
            image_fingerprint("image/png", "abc")
        );
        assert_ne!(
            image_fingerprint("image/png", "abc"),
            image_fingerprint("image/png", "abd")
        );
        assert_ne!(
            image_fingerprint("image/png", "abc"),
            image_fingerprint("image/jpeg", "abc")
        );
    }

    #[test]
    fn decode_image_accepts_valid_base64_and_rejects_invalid_or_oversized_data() {
        assert_eq!(decode_image("aGVsbG8=", "image/png").unwrap(), b"hello");
        assert!(decode_image("not base64", "image/png").is_err());
        assert!(decode_image("aGVsbG8=", "text/plain").is_err());
        assert!(decode_image(&"a".repeat(MAX_ENCODED_IMAGE_BYTES + 1), "image/png").is_err());
        let oversized =
            base64::engine::general_purpose::STANDARD.encode(vec![0; MAX_DECODED_IMAGE_BYTES + 1]);
        assert!(decode_image(&oversized, "image/png").is_err());
    }

    #[test]
    fn matching_image_content_schedules_once_and_changed_content_replaces_entry() {
        let mut cache = HashMap::new();
        let first_fingerprint = image_fingerprint("image/png", "first");
        assert!(insert_image_cache_entry(
            &mut cache,
            "scope",
            first_fingerprint,
            1,
            ImageCacheState::Loading,
        ));
        assert!(!insert_image_cache_entry(
            &mut cache,
            "scope",
            first_fingerprint,
            1,
            ImageCacheState::Loading,
        ));
        let changed_fingerprint = image_fingerprint("image/png", "changed");
        assert!(insert_image_cache_entry(
            &mut cache,
            "scope",
            changed_fingerprint,
            2,
            ImageCacheState::Loading,
        ));
        assert!(cache
            .get("scope")
            .is_some_and(|entry| { image_cache_entry_matches(entry, changed_fingerprint, 2) }));
    }

    #[test]
    fn stale_image_results_do_not_match_a_new_generation() {
        let entry = ImageCacheEntry {
            fingerprint: image_fingerprint("image/png", "new"),
            generation: 2,
            state: ImageCacheState::Loading,
        };
        assert!(!image_cache_entry_matches(
            &entry,
            image_fingerprint("image/png", "old"),
            1
        ));
        assert!(image_cache_entry_matches(
            &entry,
            image_fingerprint("image/png", "new"),
            2
        ));
    }

    #[test]
    fn markdown_selection_parts_align_ranges_with_each_block() {
        let blocks = vec![
            MarkdownBlock::Paragraph("para".into()),
            MarkdownBlock::Heading {
                level: 2,
                text: "heading".into(),
            },
            MarkdownBlock::ListItem("item".into()),
            MarkdownBlock::Rule,
            MarkdownBlock::Code {
                language: Some("rust".into()),
                text: "code".into(),
            },
            MarkdownBlock::Quote("quote".into()),
        ];
        let parts = markdown_selection_parts(&blocks);
        assert_eq!(parts.text, "para\nheading\nitem\ncode\nquote");
        assert_eq!(
            parts.ranges,
            vec![
                Some(0..4),
                Some(5..12),
                Some(13..17),
                None,
                Some(18..22),
                Some(23..28)
            ]
        );
    }
}
