//! VTerm：alacritty_terminal 真彩色网格（本地/镜像共用）
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermDamage, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor, Rgb};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct VTerm {
    inner: Arc<Mutex<VTermInner>>,
    pub cols: u16,
    pub rows: u16,
}

struct VTermInner {
    term: Term<ClipboardBridge>,
    parser: Processor,
    cached: Option<Arc<RenderSnapshot>>,
    damage: ContentDamage,
}

#[derive(Clone)]
struct ClipboardBridge {
    tx: mpsc::UnboundedSender<String>,
}

impl EventListener for ClipboardBridge {
    fn send_event(&self, event: Event) {
        if let Event::ClipboardStore(_, text) = event {
            if !text.is_empty() {
                let _ = self.tx.send(text);
            }
        }
    }
}

#[derive(Debug, Clone)]
enum ContentDamage {
    Full,
    Partial(Vec<usize>),
    None,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VTermModes {
    pub mouse: bool,
    pub sgr_mouse: bool,
    pub alt_screen: bool,
    pub alternate_scroll: bool,
    pub app_cursor: bool,
    pub bracketed_paste: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderStyle {
    pub fg: u32,
    pub bg: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub dim: bool,
    pub selected: bool,
}

#[derive(Clone, Debug)]
pub struct RenderRun {
    pub text: String,
    pub start_col: usize,
    pub cells: usize,
    pub style: RenderStyle,
}

#[derive(Clone, Debug)]
pub struct RenderRow {
    pub runs: Vec<RenderRun>,
}

#[derive(Clone, Debug)]
pub struct RenderCursor {
    pub col: usize,
    pub row: usize,
}

#[derive(Clone, Debug)]
pub struct RenderSnapshot {
    pub rows: Vec<RenderRow>,
    pub cursor: Option<RenderCursor>,
    pub logical_cursor: Option<RenderCursor>,
    pub cols: usize,
    pub lines: usize,
}

/// 每个终端保留 5000 行回滚。
const SCROLLBACK_LINES: usize = 5000;

#[derive(Clone, Copy)]
struct TermDim {
    columns: usize,
    lines: usize,
    /// 回滚容量（screen 之外的行数）。total_lines = screen_lines + scrollback。
    scrollback: usize,
}
impl Dimensions for TermDim {
    fn columns(&self) -> usize {
        self.columns
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn total_lines(&self) -> usize {
        self.lines + self.scrollback
    }
}

impl VTerm {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self::new_with_clipboard(cols, rows).0
    }

    pub fn new_with_clipboard(cols: u16, rows: u16) -> (Self, mpsc::UnboundedReceiver<String>) {
        let size = TermDim {
            columns: cols as usize,
            lines: rows as usize,
            scrollback: SCROLLBACK_LINES,
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let term = Term::new(Default::default(), &size, ClipboardBridge { tx });
        (
            VTerm {
                inner: Arc::new(Mutex::new(VTermInner {
                    term,
                    parser: Processor::new(),
                    cached: None,
                    damage: ContentDamage::Full,
                })),
                cols,
                rows,
            },
            rx,
        )
    }

    fn lock_inner(&self) -> Option<MutexGuard<'_, VTermInner>> {
        match self.inner.lock() {
            Ok(guard) => Some(guard),
            Err(error) => {
                tracing::error!(%error, "VTerm mutex poisoned");
                None
            }
        }
    }

    pub fn feed(&self, data: &[u8]) {
        if let Some(mut guard) = self.lock_inner() {
            let VTermInner { term, parser, .. } = &mut *guard;
            parser.advance(term, data);
            let damage = match term.damage() {
                TermDamage::Full => ContentDamage::Full,
                TermDamage::Partial(lines) => {
                    let rows: Vec<usize> = lines.map(|d| d.line).collect();
                    if rows.is_empty() {
                        ContentDamage::None
                    } else {
                        ContentDamage::Partial(rows)
                    }
                }
            };
            term.reset_damage();
            guard.damage = merge_damage(
                std::mem::replace(&mut guard.damage, ContentDamage::None),
                damage,
            );
        }
    }

    /// 纯文本（检测/测试用）；UI 应使用 render_snapshot 保留颜色/光标。
    pub fn text_lines(&self) -> Vec<String> {
        let snap = self.render_snapshot();
        snap.rows
            .iter()
            .map(|r| {
                r.runs
                    .iter()
                    .map(|x| x.text.as_str())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// 真彩色渲染快照：相邻同样式 cell 合并为 run；光标独立返回。
    /// 返回 Arc：damage 为 None 时调用方 O(1) 共享缓存快照，不再整帧深拷贝。
    pub fn render_snapshot(&self) -> Arc<RenderSnapshot> {
        let mut guard = match self.lock_inner() {
            Some(guard) => guard,
            None => {
                return Arc::new(RenderSnapshot {
                    rows: vec![],
                    cursor: None,
                    logical_cursor: None,
                    cols: 0,
                    lines: 0,
                })
            }
        };
        let damage = std::mem::replace(&mut guard.damage, ContentDamage::None);
        let need_full = guard.cached.is_none()
            || matches!(damage, ContentDamage::Full)
            || guard.cached.as_ref().is_some_and(|c| {
                c.cols != guard.term.columns() || c.lines != guard.term.screen_lines()
            });

        let mut snap = if need_full {
            Arc::new(build_snapshot(&guard.term))
        } else {
            guard.cached.take().unwrap()
        };
        if let ContentDamage::Partial(rows) = damage {
            // copy-on-write：无其他持有者时原地更新，有持有者（渲染中）时克隆一份再改。
            let snap = Arc::make_mut(&mut snap);
            for row in rows {
                if row < snap.rows.len() {
                    snap.rows[row] = build_row(&guard.term, row);
                }
            }
            snap.cursor = cursor_of(&guard.term);
            snap.logical_cursor = logical_cursor_of(&guard.term);
        }
        guard.cached = Some(Arc::clone(&snap));
        snap
    }

    pub fn begin_selection(&self, line: i32, col: usize, right: bool) {
        if let Some(mut guard) = self.lock_inner() {
            guard.term.selection = Some(Selection::new(
                SelectionType::Simple,
                Point::new(Line(line), Column(col)),
                if right { Side::Right } else { Side::Left },
            ));
            guard.cached = None;
            guard.damage = ContentDamage::Full;
        }
    }

    /// 双击词选择：按语义边界选中整个词。
    pub fn select_word_at(&self, line: i32, col: usize) {
        self.select_at(SelectionType::Semantic, line, col);
    }

    /// 三击行选择：选中整行。
    pub fn select_lines_at(&self, line: i32, col: usize) {
        self.select_at(SelectionType::Lines, line, col);
    }

    fn select_at(&self, ty: SelectionType, line: i32, col: usize) {
        if let Some(mut guard) = self.lock_inner() {
            guard.term.selection = Some(Selection::new(
                ty,
                Point::new(Line(line), Column(col)),
                Side::Left,
            ));
            guard.cached = None;
            guard.damage = ContentDamage::Full;
        }
    }

    pub fn update_selection(&self, line: i32, col: usize, right: bool) {
        if let Some(mut guard) = self.lock_inner() {
            if let Some(selection) = guard.term.selection.as_mut() {
                selection.update(
                    Point::new(Line(line), Column(col)),
                    if right { Side::Right } else { Side::Left },
                );
                guard.cached = None;
                guard.damage = ContentDamage::Full;
            }
        }
    }

    pub fn stop_selection(&self) {
        if let Some(mut guard) = self.lock_inner() {
            if guard.term.selection.is_some() {
                guard.term.selection = None;
                guard.cached = None;
                guard.damage = ContentDamage::Full;
            }
        }
    }

    pub fn selection_to_string(&self) -> Option<String> {
        self.lock_inner()?.term.selection_to_string()
    }

    pub fn selection_active(&self) -> bool {
        self.lock_inner()
            .map(|guard| guard.term.selection.is_some())
            .unwrap_or(false)
    }

    pub fn mouse_motion_reporting(&self) -> bool {
        self.lock_inner()
            .map(|guard| {
                guard
                    .term
                    .mode()
                    .intersects(TermMode::MOUSE_MOTION | TermMode::MOUSE_DRAG)
            })
            .unwrap_or(false)
    }

    pub fn modes(&self) -> VTermModes {
        self.lock_inner()
            .map(|guard| {
                let mode = guard.term.mode();
                VTermModes {
                    mouse: mode.intersects(TermMode::MOUSE_MODE),
                    sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
                    alt_screen: mode.contains(TermMode::ALT_SCREEN),
                    alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
                    app_cursor: mode.contains(TermMode::APP_CURSOR),
                    bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
                }
            })
            .unwrap_or_default()
    }

    pub fn scroll_display(&self, lines: i32) -> bool {
        if lines == 0 {
            return false;
        }
        self.lock_inner()
            .map(|mut guard| {
                let before = guard.term.grid().display_offset();
                guard.term.scroll_display(Scroll::Delta(lines));
                let changed = before != guard.term.grid().display_offset();
                if changed {
                    guard.cached = None;
                    guard.damage = ContentDamage::Full;
                }
                changed
            })
            .unwrap_or(false)
    }

    pub fn scroll_metrics(&self) -> (usize, usize) {
        self.lock_inner()
            .map(|guard| {
                (
                    guard.term.grid().history_size(),
                    guard.term.grid().display_offset(),
                )
            })
            .unwrap_or_default()
    }

    pub fn mouse_reporting(&self) -> bool {
        self.lock_inner()
            .map(|guard| guard.term.mode().intersects(TermMode::MOUSE_MODE))
            .unwrap_or(false)
    }

    pub fn sgr_mouse(&self) -> bool {
        self.lock_inner()
            .map(|guard| guard.term.mode().contains(TermMode::SGR_MOUSE))
            .unwrap_or(false)
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        if let Some(mut guard) = self.lock_inner() {
            guard.term.resize(TermDim {
                columns: cols as usize,
                lines: rows as usize,
                scrollback: SCROLLBACK_LINES,
            });
            guard.cached = None;
            guard.damage = ContentDamage::Full;
        }
    }

    pub fn line_text(&self, visual: usize) -> Option<String> {
        let guard = self.lock_inner()?;
        let grid = guard.term.grid();
        if visual >= grid.screen_lines() {
            return None;
        }
        let buffer_line = visual as i32 - grid.display_offset() as i32;
        let mut line = String::new();
        for col in 0..grid.columns() {
            let cell = &grid[Point::new(Line(buffer_line), Column(col))];
            if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                line.push(cell.c);
            }
        }
        Some(line)
    }

    pub fn url_at(&self, visual: usize, col: usize) -> Option<String> {
        let text = self.line_text(visual)?;
        let mut start_idx = 0;
        for token in text.split_inclusive(|c: char| {
            c.is_whitespace()
                || c == '"'
                || c == '\''
                || c == '<'
                || c == '>'
                || c == '('
                || c == ')'
                || c == '['
                || c == ']'
        }) {
            let token_len = token.chars().count();
            let end_idx = start_idx + token_len;
            let trimmed = token.trim_matches(|c: char| {
                c.is_whitespace()
                    || c == '"'
                    || c == '\''
                    || c == '<'
                    || c == '>'
                    || c == '('
                    || c == ')'
                    || c == '['
                    || c == ']'
                    || c == ','
                    || c == ';'
            });
            if col >= start_idx
                && col < end_idx
                && (trimmed.starts_with("http://") || trimmed.starts_with("https://"))
            {
                return Some(trimmed.to_string());
            }
            start_idx = end_idx;
        }
        None
    }
}

fn merge_damage(a: ContentDamage, b: ContentDamage) -> ContentDamage {
    match (a, b) {
        (ContentDamage::Full, _) | (_, ContentDamage::Full) => ContentDamage::Full,
        (ContentDamage::None, x) | (x, ContentDamage::None) => x,
        (ContentDamage::Partial(mut a), ContentDamage::Partial(b)) => {
            a.extend(b);
            a.sort_unstable();
            a.dedup();
            ContentDamage::Partial(a)
        }
    }
}

fn build_snapshot(term: &Term<ClipboardBridge>) -> RenderSnapshot {
    let rows = (0..term.screen_lines())
        .map(|r| build_row(term, r))
        .collect();
    RenderSnapshot {
        rows,
        cursor: cursor_of(term),
        logical_cursor: logical_cursor_of(term),
        cols: term.columns(),
        lines: term.screen_lines(),
    }
}

fn cursor_of(term: &Term<ClipboardBridge>) -> Option<RenderCursor> {
    if !term.mode().contains(TermMode::SHOW_CURSOR) {
        return None;
    }
    let grid = term.grid();
    let visual = grid.cursor.point.line.0 + grid.display_offset() as i32;
    (visual >= 0 && visual < grid.screen_lines() as i32).then_some(RenderCursor {
        col: grid.cursor.point.column.0,
        row: visual as usize,
    })
}

fn logical_cursor_of(term: &Term<ClipboardBridge>) -> Option<RenderCursor> {
    let grid = term.grid();
    let p = grid.cursor.point;
    let visual = p.line.0 + grid.display_offset() as i32;
    let last = grid.screen_lines().saturating_sub(1) as i32;
    Some(RenderCursor {
        col: p.column.0,
        row: visual.clamp(0, last) as usize,
    })
}

fn build_row(term: &Term<ClipboardBridge>, visual: usize) -> RenderRow {
    let grid = term.grid();
    let columns = grid.columns();
    let buffer_line = visual as i32 - grid.display_offset() as i32;
    let default_fg = 0x2a2e38ff;
    let default_bg = 0xffffffff;
    // 选区范围与 cell 无关：每行只解析一次，不再逐 cell 调 to_range。
    let selection_range = term
        .selection
        .as_ref()
        .and_then(|selection| selection.to_range(term));
    let mut runs: Vec<RenderRun> = vec![];
    let mut current: Option<RenderRun> = None;
    for col in 0..columns {
        let cell = &grid[Point::new(Line(buffer_line), Column(col))];
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            if let Some(mut run) = current.take() {
                run.cells += 1;
                runs.push(run);
            }
            continue;
        }
        let mut fg = cell.fg;
        let mut bg = cell.bg;
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        let style = RenderStyle {
            fg: color_u32(fg, default_fg, default_bg),
            bg: color_u32(bg, default_fg, default_bg),
            bold: cell.flags.contains(Flags::BOLD),
            italic: cell.flags.contains(Flags::ITALIC),
            underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
            dim: cell.flags.contains(Flags::DIM) && !cell.flags.contains(Flags::BOLD),
            selected: selection_range.as_ref().is_some_and(|selection| {
                selection.contains(Point::new(Line(buffer_line), Column(col)))
            }),
        };
        let ch = if cell.flags.contains(Flags::HIDDEN) {
            ' '
        } else {
            cell.c
        };
        let append = current
            .as_ref()
            .is_some_and(|r| r.style == style && r.start_col + r.cells == col);
        if append {
            let r = current.as_mut().unwrap();
            r.text.push(ch);
            if let Some(zerowidth) = cell.zerowidth() {
                r.text.extend(zerowidth.iter());
            }
            r.cells += 1;
        } else {
            if let Some(r) = current.take() {
                runs.push(r);
            }
            let mut text = ch.to_string();
            if let Some(zerowidth) = cell.zerowidth() {
                text.extend(zerowidth.iter());
            }
            current = Some(RenderRun {
                text,
                start_col: col,
                cells: 1,
                style,
            });
        }
    }
    if let Some(r) = current {
        runs.push(r);
    }
    RenderRow { runs }
}

fn color_u32(c: Color, default_fg: u32, default_bg: u32) -> u32 {
    match c {
        Color::Spec(Rgb { r, g, b }) => rgba_u32(r, g, b),
        Color::Indexed(i) => indexed_color(i),
        Color::Named(n) => named_color(n, default_fg, default_bg),
    }
}
fn rgba_u32(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xff
}
fn named_color(n: NamedColor, fg: u32, bg: u32) -> u32 {
    use NamedColor::*;
    match n {
        Foreground | BrightForeground | DimForeground => fg,
        Background => bg,
        Cursor => 0x3d6cd8ff,
        Black | DimBlack => 0x2a2e38ff,
        Red | DimRed => 0xd64557ff,
        Green | DimGreen => 0x5c9e3aff,
        Yellow | DimYellow => 0xc08a2dff,
        Blue | DimBlue => 0x3d6cd8ff,
        Magenta | DimMagenta => 0x9b59b6ff,
        Cyan | DimCyan => 0x2a92b0ff,
        White | DimWhite => 0xcfcfd3ff,
        BrightBlack => 0x6f7480ff,
        BrightRed => 0xef5668ff,
        BrightGreen => 0x6fb84aff,
        BrightYellow => 0xd7a13eff,
        BrightBlue => 0x5b82e5ff,
        BrightMagenta => 0xb06ac4ff,
        BrightCyan => 0x45a8c4ff,
        BrightWhite => 0xffffffff,
    }
}
fn indexed_color(i: u8) -> u32 {
    const BASIC: [u32; 16] = [
        0x2a2e38ff, 0xd64557ff, 0x5c9e3aff, 0xc08a2dff, 0x3d6cd8ff, 0x9b59b6ff, 0x2a92b0ff,
        0xcfcfd3ff, 0x6f7480ff, 0xef5668ff, 0x6fb84aff, 0xd7a13eff, 0x5b82e5ff, 0xb06ac4ff,
        0x45a8c4ff, 0xffffffff,
    ];
    match i {
        0..=15 => BASIC[i as usize],
        16..=231 => {
            let n = i - 16;
            let r = n / 36;
            let g = (n % 36) / 6;
            let b = n % 6;
            let cv = |x: u8| if x == 0 { 0 } else { 55 + x * 40 };
            rgba_u32(cv(r), cv(g), cv(b))
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            rgba_u32(v, v, v)
        }
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    #[test]
    fn osc52_clipboard_store_is_emitted() {
        let (vterm, mut rx) = VTerm::new_with_clipboard(80, 24);
        let payload = muxlane_core::protocol::b64_encode(b"copied-from-tmux");
        vterm.feed(format!("\x1b]52;c;{payload}\x07").as_bytes());
        let text = rx.try_recv().expect("osc52 clipboard event");
        assert_eq!(text, "copied-from-tmux");
    }

    #[test]
    fn word_selection_grabs_semantic_word() {
        let vterm = VTerm::new(80, 24);
        vterm.feed(b"foo bar baz\r\n");
        vterm.select_word_at(0, 5);
        let text = vterm.selection_to_string().unwrap_or_default();
        assert_eq!(text, "bar");
    }

    #[test]
    fn line_selection_grabs_whole_line() {
        let vterm = VTerm::new(80, 24);
        vterm.feed(b"first line\r\nsecond line\r\n");
        vterm.select_lines_at(1, 3);
        let text = vterm.selection_to_string().unwrap_or_default();
        assert_eq!(text, "second line\n");
    }

    #[test]
    fn drag_selection_roundtrip() {
        let vterm = VTerm::new(80, 24);
        vterm.feed(b"HELLO-SELECTION-WORLD\r\nSECOND LINE\r\n");
        vterm.begin_selection(0, 0, false);
        // Side::Right 使终点列包含进选区：0..=20 共 21 列
        vterm.update_selection(0, 20, true);
        let text = vterm.selection_to_string().unwrap_or_default();
        assert_eq!(text, "HELLO-SELECTION-WORLD");
    }
}

#[cfg(test)]
mod alt_screen_tests {
    use super::*;

    #[test]
    fn feed_tracks_alt_screen_mode_transitions() {
        let vterm = VTerm::new(80, 24);
        assert!(!vterm.modes().alt_screen);
        vterm.feed(b"\x1b[?1049h");
        assert!(vterm.modes().alt_screen);
        vterm.feed(b"\x1b[?1049l");
        assert!(!vterm.modes().alt_screen);

        let vterm2 = VTerm::new(80, 24);
        vterm2.feed(b"\x1b[31mred\x1b[0m");
        assert!(vterm2.line_text(0).unwrap().contains("red"));
    }
}

#[cfg(test)]
mod scrollback_tests {
    use super::*;

    #[test]
    fn scrolling_output_accumulates_history() {
        let vterm = VTerm::new(80, 24);
        for i in 0..200 {
            vterm.feed(format!("line-{i}\r\n").as_bytes());
        }
        let (history, _offset) = vterm.scroll_metrics();
        assert!(history > 0, "history should accumulate, got {history}");
        assert!(
            vterm.scroll_display(3),
            "scroll_display should move viewport"
        );
    }
}
