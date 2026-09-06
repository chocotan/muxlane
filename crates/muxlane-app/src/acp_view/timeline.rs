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
    HighlightStyle, Image, ImageFormat, InteractiveElement, LayoutId, MouseButton, MouseDownEvent,
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
            .px_6()
            .py_1()
            .flex()
            .items_center()
            .gap_2()
            .text_color(rgba(theme.fg1))
            .child(render_status_indicator(
                AgentStatus::Working,
                "acp-generating",
                theme,
            ))
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
    terminal_outputs: HashMap<String, muxlane_acp::TerminalSnapshot>,
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
}

struct TimelineTextElement {
    layouts: Rc<RefCell<Vec<TimelineLayoutRecord>>>,
    text: StyledText,
    content: String,
    scope: String,
    range: Range<usize>,
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
        self.layouts.borrow_mut().push(TimelineLayoutRecord {
            scope: self.scope.clone(),
            range: self.range.clone(),
            bounds,
            layout: self.text.layout().clone(),
            content: self.content.clone(),
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
    terminal_outputs: HashMap<String, muxlane_acp::TerminalSnapshot>,
    diff_decisions: HashMap<(String, String), DiffDecision>,
    focus: FocusHandle,
    selection: TimelineSelection,
    layouts: Rc<RefCell<Vec<TimelineLayoutRecord>>>,
    texts: RefCell<HashMap<String, String>>,
    markdown_cache: RefCell<HashMap<String, MarkdownCacheEntry>>,
    image_cache: RefCell<HashMap<String, ImageCacheEntry>>,
    image_fingerprints: HashMap<String, u64>,
    image_generation: Cell<u64>,
}

impl TimelineItemView {
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
            terminal_outputs: HashMap::new(),
            diff_decisions: HashMap::new(),
            focus: cx.focus_handle(),
            selection: TimelineSelection::default(),
            layouts: Rc::new(RefCell::new(Vec::new())),
            texts: RefCell::new(HashMap::new()),
            markdown_cache: RefCell::new(HashMap::new()),
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
            terminal_outputs: self.terminal_outputs.clone(),
            diff_decisions: self.diff_decisions.clone(),
            selection: self.selection.clone(),
        }
    }

    pub(crate) fn restore_transient_state(&mut self, state: TimelineItemTransientState) {
        self.expanded = state.expanded;
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

    pub(crate) fn set_terminal_snapshot(
        &mut self,
        snapshot: muxlane_acp::TerminalSnapshot,
        cx: &mut Context<Self>,
    ) {
        self.terminal_outputs.insert(snapshot.id.clone(), snapshot);
        cx.notify();
    }

    fn theme(&self) -> Theme {
        Theme::for_mode(self.theme_mode)
    }

    fn timeline_mouse_down(
        &mut self,
        scope: &str,
        position: Point<Pixels>,
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
            self.selection.begin(scope, offset);
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
        let display_text: SharedString = text.to_string().into();
        let mut styled_text = StyledText::new(display_text.clone());
        if let Some(selection) = self.selection.highlight(scope, &range) {
            styled_text = styled_text.with_highlights(vec![(
                selection,
                HighlightStyle {
                    background_color: Some(rgba(theme.selection()).into()),
                    ..Default::default()
                },
            )]);
        }
        let scope_for_event = scope.to_string();
        let layouts = self.layouts.clone();
        div()
            .cursor(gpui::CursorStyle::IBeam)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.timeline_mouse_down(&scope_for_event, event.position, window, cx);
                }),
            )
            .child(TimelineTextElement {
                layouts,
                text: styled_text,
                content: display_text.to_string(),
                scope: scope.to_string(),
                range,
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
        let mut content = div().flex().flex_col().gap_2().whitespace_normal();
        for block in blocks {
            content = content.child(match block {
                MarkdownBlock::Paragraph(text) => {
                    div()
                        .line_height(ui_px(19.))
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
                MarkdownBlock::Code { language, text } => div()
                    .flex()
                    .flex_col()
                    .border_1()
                    .border_color(rgba(theme.line))
                    .bg(rgba(theme.bg1))
                    .px_3()
                    .py_2()
                    .font_family("monospace")
                    .text_size(ui_px(11.))
                    .whitespace_normal()
                    .when_some(language.clone(), |code, language| {
                        code.child(
                            div()
                                .pb_1()
                                .text_size(ui_px(9.))
                                .text_color(rgba(theme.fg2))
                                .child(language),
                        )
                    })
                    .child(self.render_selectable_text(
                        scope,
                        text,
                        ranges.next().flatten().unwrap_or_default(),
                        theme,
                        cx,
                    )),
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
                    .border_color(rgba(theme.fg2))
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
            });
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
                .child(img(image).max_w_full().max_h(ui_px(360.))),
            ImageCacheState::Error(error) => div().text_color(rgba(theme.red)).child(error),
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
            ToolContent::Unknown(value) => {
                let output = value.to_string();
                self.register_text(scope, output.clone());
                div()
                    .font_family("monospace")
                    .text_color(rgba(theme.fg2))
                    .child(self.render_selectable_text(scope, &output, 0..output.len(), theme, cx))
            }
        }
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
        if let Some(old_text) = old_text {
            self.register_text(&format!("{scope}:old"), old_text.to_string());
        }
        self.register_text(&format!("{scope}:new"), new_text.to_string());
        let keep_key = key.clone();
        let reject_key = key;
        let reject_path = path.to_string();
        let reject_old = old_text.map(str::to_string);
        let reject_new = new_text.to_string();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .border_1()
            .border_color(rgba(theme.line))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .bg(rgba(theme.bg2))
                    .text_color(rgba(theme.fg1))
                    .child(path.to_string()),
            )
            .when_some(old_text.map(str::to_string), |diff, old_text| {
                diff.child(
                    div()
                        .px_2()
                        .py_1()
                        .bg(rgba(Theme::with_alpha(theme.red, 0x18)))
                        .font_family("monospace")
                        .whitespace_normal()
                        .child(self.render_selectable_text(
                            &format!("{scope}:old"),
                            &old_text,
                            0..old_text.len(),
                            theme,
                            cx,
                        )),
                )
            })
            .child(
                div()
                    .px_2()
                    .py_1()
                    .bg(rgba(Theme::with_alpha(theme.green, 0x18)))
                    .font_family("monospace")
                    .whitespace_normal()
                    .child(self.render_selectable_text(
                        &format!("{scope}:new"),
                        new_text,
                        0..new_text.len(),
                        theme,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .when(decision.is_none(), |actions| {
                        actions
                            .child(
                                semantic_button(
                                    gpui::ElementId::Name(format!("diff-keep-{path}").into()),
                                    i18n::text(self.language, "acp.diff_keep"),
                                    theme,
                                )
                                .px_2()
                                .py_1()
                                .border_1()
                                .border_color(rgba(theme.green))
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    this.keep_diff(keep_key.clone(), cx)
                                }))
                                .child(i18n::text(self.language, "acp.diff_keep")),
                            )
                            .child(
                                semantic_button(
                                    gpui::ElementId::Name(format!("diff-reject-{path}").into()),
                                    i18n::text(self.language, "acp.diff_reject"),
                                    theme,
                                )
                                .px_2()
                                .py_1()
                                .text_color(rgba(theme.red))
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
                        actions.child(match decision {
                            DiffDecision::Kept => i18n::text(self.language, "acp.diff_kept"),
                            DiffDecision::Rejected => {
                                i18n::text(self.language, "acp.diff_rejected")
                            }
                        })
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
        let snapshot = self.terminal_outputs.get(id).cloned();
        let terminal_id = id.to_string();
        let output = snapshot.map_or_else(
            || i18n::text(self.language, "acp.terminal_loading").to_string(),
            |snapshot| {
                let suffix = match snapshot.exit_code {
                    Some(code) => format!("\n[exit {code}]"),
                    None if snapshot.truncated => "\n[output truncated]".into(),
                    None => String::new(),
                };
                format!("{}{suffix}", snapshot.output)
            },
        );
        self.register_text(scope, output.clone());
        div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(rgba(theme.line))
            .child(
                div()
                    .flex()
                    .items_center()
                    .px_2()
                    .py_1()
                    .bg(rgba(theme.bg2))
                    .font_family("monospace")
                    .child(format!("Terminal {id}"))
                    .child(
                        semantic_button(
                            gpui::ElementId::Name(format!("terminal-refresh-{id}").into()),
                            i18n::text(self.language, "acp.terminal_refresh"),
                            theme,
                        )
                        .ml_auto()
                        .px_2()
                        .py_1()
                        .on_click(cx.listener(move |this, _event, _window, cx| {
                            this.poll_terminal(terminal_id.clone(), cx)
                        }))
                        .child("↻"),
                    ),
            )
            .child(
                div()
                    .id(gpui::ElementId::Name(
                        format!("terminal-output-{id}").into(),
                    ))
                    .max_h(ui_px(280.))
                    .overflow_y_scroll()
                    .px_2()
                    .py_1()
                    .font_family("monospace")
                    .whitespace_normal()
                    .child(self.render_selectable_text(scope, &output, 0..output.len(), theme, cx)),
            )
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
        parent
            .update(cx, |view, cx| {
                if let Some(handle) = &view.handle {
                    if let Err(error) = handle.poll_terminal(id) {
                        view.push_entry(Entry::Error(error.to_string()));
                        cx.notify();
                    }
                }
            })
            .ok();
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
                MessageRole::User => div()
                    .mx_3()
                    .my_2()
                    .p_3()
                    .border_1()
                    .border_color(rgba(theme.line))
                    .bg(rgba(theme.bg0))
                    .font_family("monospace")
                    .text_size(ui_px(12.))
                    .child(self.render_cached_markdown(
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
                    div().px_6().py_2().child(body).child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .pt_1()
                            .child(
                                semantic_button(
                                    gpui::ElementId::Name(format!("acp-copy-{id}").into()),
                                    i18n::text(self.language, "acp.copy_response"),
                                    theme,
                                )
                                .w(ui_px(26.))
                                .h(ui_px(26.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(rgba(theme.fg2))
                                .hover(|style| {
                                    style.bg(rgba(theme.bg2)).text_color(rgba(theme.fg0))
                                })
                                .on_click(cx.listener(move |_this, _event, _window, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        body_text.clone(),
                                    ))
                                }))
                                .child(panel_icon(COPY_ICON, theme.fg2)),
                            )
                            .child(
                                semantic_button(
                                    gpui::ElementId::Name(format!("acp-bottom-{id}").into()),
                                    i18n::text(self.language, "acp.scroll_bottom"),
                                    theme,
                                )
                                .w(ui_px(26.))
                                .h(ui_px(26.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(rgba(theme.fg2))
                                .hover(|style| {
                                    style.bg(rgba(theme.bg2)).text_color(rgba(theme.fg0))
                                })
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
                    MessageRole::User => div()
                        .mx_3()
                        .my_2()
                        .p_3()
                        .border_1()
                        .border_color(rgba(theme.line))
                        .bg(rgba(theme.bg1))
                        .child(body),
                    MessageRole::Assistant => div().px_3().py_2().child(body),
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
                    .px_6()
                    .my_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .min_w_0()
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
                                .gap_2()
                                .px_0()
                                .py_1()
                                .text_color(rgba(theme.fg1))
                                .on_click(
                                    cx.listener(|this, _event, _window, cx| {
                                        this.toggle_expanded(cx)
                                    }),
                                )
                                .child(div().w(ui_px(14.)).flex_none().child(if self.expanded {
                                    "▾"
                                } else {
                                    "◇"
                                }))
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
                                semantic_button(
                                    gpui::ElementId::Name(format!("acp-thought-copy-{id}").into()),
                                    i18n::text(self.language, "acp.copy_thought"),
                                    theme,
                                )
                                .w(ui_px(26.))
                                .h(ui_px(26.))
                                .flex()
                                .items_center()
                                .justify_center()
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
                            div()
                                .id(format!("acp-thought-body-{id}"))
                                .max_h(ui_px(360.))
                                .overflow_y_scroll()
                                .min_w_0()
                                .pb_2()
                                .text_color(rgba(theme.fg1))
                                .child(body),
                        )
                    })
            }
            ThreadItem::Tool(tool) => {
                let id = tool.id.clone();
                let copy_text = tool_copy_text(tool);
                let mut view = div().px_6().my_1().child(
                    div()
                        .flex()
                        .items_center()
                        .min_w_0()
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
                            .gap_2()
                            .px_0()
                            .py_1()
                            .text_color(rgba(theme.fg1))
                            .on_click(
                                cx.listener(|this, _event, _window, cx| this.toggle_expanded(cx)),
                            )
                            .child(div().w(ui_px(14.)).flex_none().child(if self.expanded {
                                "▾"
                            } else {
                                "▸"
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(tool.title.clone()),
                            ),
                        )
                        .child(
                            semantic_button(
                                gpui::ElementId::Name(format!("acp-tool-copy-{id}").into()),
                                i18n::text(self.language, "acp.copy_tool"),
                                theme,
                            )
                            .w(ui_px(26.))
                            .h(ui_px(26.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_click(cx.listener(move |_this, _event, _window, cx| {
                                cx.stop_propagation();
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    copy_text.clone(),
                                ));
                            }))
                            .child(panel_icon(COPY_ICON, theme.fg2)),
                        ),
                );
                if let Some(session_id) = tool.subagent_session_id.clone() {
                    view = view.child(
                        div().flex().justify_end().py_1().child(
                            semantic_button(
                                gpui::ElementId::Name(format!("open-subagent-{}", tool.id).into()),
                                i18n::text(self.language, "acp.open_subagent"),
                                theme,
                            )
                            .px_2()
                            .py_1()
                            .border_1()
                            .border_color(rgba(theme.line))
                            .on_click(cx.listener(move |this, _event, _window, cx| {
                                this.open_subagent(session_id.clone(), cx)
                            }))
                            .child(i18n::text(self.language, "acp.open_subagent")),
                        ),
                    );
                }
                if self.expanded {
                    for location in &tool.locations {
                        view = view.child(div().py_1().text_color(rgba(theme.accent)).child(
                            match location.line {
                                Some(line) => format!("{}:{line}", location.path),
                                None => location.path.clone(),
                            },
                        ));
                    }
                    for (index, content) in tool.content.iter().enumerate() {
                        let scope = format!("tool:{id}:content:{index}");
                        view = view.child(
                            div()
                                .id(format!("acp-tool-content-{id}-{index}"))
                                .py_1()
                                .min_w_0()
                                .max_h(ui_px(360.))
                                .overflow_y_scroll()
                                .child(self.render_tool_content(content, theme, &scope, cx)),
                        );
                    }
                    if let Some(input) = &tool.raw_input {
                        let scope = format!("tool:{id}:input");
                        let text = format!("Input: {input}");
                        self.register_text(&scope, text.clone());
                        view = view.child(
                            div()
                                .py_1()
                                .min_w_0()
                                .font_family("monospace")
                                .whitespace_normal()
                                .child(self.render_selectable_text(
                                    &scope,
                                    &text,
                                    0..text.len(),
                                    theme,
                                    cx,
                                )),
                        );
                    }
                    if let Some(output) = &tool.raw_output {
                        let scope = format!("tool:{id}:output");
                        let text = format!("Output: {output}");
                        self.register_text(&scope, text.clone());
                        view = view.child(
                            div()
                                .py_1()
                                .min_w_0()
                                .font_family("monospace")
                                .whitespace_normal()
                                .child(self.render_selectable_text(
                                    &scope,
                                    &text,
                                    0..text.len(),
                                    theme,
                                    cx,
                                )),
                        );
                    }
                }
                view
            }
        }
    }
}

impl Render for TimelineItemView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.texts.borrow_mut().clear();
        self.layouts.borrow_mut().clear();
        self.render_thread_item(cx)
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
            id: "thought-1".into(),
            text: "same".into(),
        });
        let changed_item = ThreadItem::Thought(muxlane_acp::Thought {
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
            muxlane_acp::TerminalSnapshot {
                id: "terminal-1".into(),
                output: "output".into(),
                truncated: false,
                exit_code: None,
            },
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
