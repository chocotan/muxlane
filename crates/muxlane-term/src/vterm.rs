//! VTerm：alacritty_terminal 真彩色网格（本地/镜像共用）
use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Term, TermDamage, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor, Rgb};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::mpsc;

use crate::kitty_graphics::{KittyGraphicsScanner, StoredImage};

#[derive(Clone)]
pub struct VTerm {
    inner: Arc<Mutex<VTermInner>>,
    /// (cols, rows, cell_width_px, cell_height_px) 打包进一个 u64，给 XTWINOPS 查询（如
    /// `CSI 14t` 报窗口像素尺寸）现押答。不进锁：写频率低（只在 resize 时），
    /// 读频率也低（只在程序主动查询时），用 Mutex 不划算。
    window_size: Arc<AtomicU64>,
    silent: Arc<AtomicBool>,
    pub cols: u16,
    pub rows: u16,
}

/// 终端主动产生的副作用：剪贴板写入，或需要回写进 PTY 的字节（光标位置报告、
/// 窗口像素尺寸查询等标准 xterm 查询/应答协议，alacritty_terminal 自己会解码请求，
/// 但必须由嵌入方把结果写回 PTY，否则客户端程序（如 `kitten icat`）会一直等超时。
#[derive(Debug, Clone)]
pub enum TermSideEffect {
    ClipboardStore(String),
    PtyWrite(Vec<u8>),
}

struct VTermInner {
    term: Term<ClipboardBridge>,
    parser: Processor,
    kitty: KittyGraphicsScanner,
    cached: Option<Arc<RenderSnapshot>>,
    damage: ContentDamage,
}

/// Kitty Unicode Placeholder 用的 PUA 占位符（协议固定值，见 kitty graphics-protocol 文档）。
const KITTY_PLACEHOLDER: char = '\u{10EEEE}';

/// `rowcolumn-diacritics.txt`（Unicode 6.0.0 冻结版）：下标即编码的行/列/MSB 数值。
/// 来源：https://sw.kovidgoyal.net/kitty/_downloads/f0a0de9ec8d9ff4456206db8e0814937/rowcolumn-diacritics.txt
#[rustfmt::skip]
const ROWCOLUMN_DIACRITICS: [u32; 297] = [
    0x305, 0x30D, 0x30E, 0x310, 0x312, 0x33D, 0x33E, 0x33F,
    0x346, 0x34A, 0x34B, 0x34C, 0x350, 0x351, 0x352, 0x357,
    0x35B, 0x363, 0x364, 0x365, 0x366, 0x367, 0x368, 0x369,
    0x36A, 0x36B, 0x36C, 0x36D, 0x36E, 0x36F, 0x483, 0x484,
    0x485, 0x486, 0x487, 0x592, 0x593, 0x594, 0x595, 0x597,
    0x598, 0x599, 0x59C, 0x59D, 0x59E, 0x59F, 0x5A0, 0x5A1,
    0x5A8, 0x5A9, 0x5AB, 0x5AC, 0x5AF, 0x5C4, 0x610, 0x611,
    0x612, 0x613, 0x614, 0x615, 0x616, 0x617, 0x657, 0x658,
    0x659, 0x65A, 0x65B, 0x65D, 0x65E, 0x6D6, 0x6D7, 0x6D8,
    0x6D9, 0x6DA, 0x6DB, 0x6DC, 0x6DF, 0x6E0, 0x6E1, 0x6E2,
    0x6E4, 0x6E7, 0x6E8, 0x6EB, 0x6EC, 0x730, 0x732, 0x733,
    0x735, 0x736, 0x73A, 0x73D, 0x73F, 0x740, 0x741, 0x743,
    0x745, 0x747, 0x749, 0x74A, 0x7EB, 0x7EC, 0x7ED, 0x7EE,
    0x7EF, 0x7F0, 0x7F1, 0x7F3, 0x816, 0x817, 0x818, 0x819,
    0x81B, 0x81C, 0x81D, 0x81E, 0x81F, 0x820, 0x821, 0x822,
    0x823, 0x825, 0x826, 0x827, 0x829, 0x82A, 0x82B, 0x82C,
    0x82D, 0x951, 0x953, 0x954, 0xF82, 0xF83, 0xF86, 0xF87,
    0x135D, 0x135E, 0x135F, 0x17DD, 0x193A, 0x1A17, 0x1A75, 0x1A76,
    0x1A77, 0x1A78, 0x1A79, 0x1A7A, 0x1A7B, 0x1A7C, 0x1B6B, 0x1B6D,
    0x1B6E, 0x1B6F, 0x1B70, 0x1B71, 0x1B72, 0x1B73, 0x1CD0, 0x1CD1,
    0x1CD2, 0x1CDA, 0x1CDB, 0x1CE0, 0x1DC0, 0x1DC1, 0x1DC3, 0x1DC4,
    0x1DC5, 0x1DC6, 0x1DC7, 0x1DC8, 0x1DC9, 0x1DCB, 0x1DCC, 0x1DD1,
    0x1DD2, 0x1DD3, 0x1DD4, 0x1DD5, 0x1DD6, 0x1DD7, 0x1DD8, 0x1DD9,
    0x1DDA, 0x1DDB, 0x1DDC, 0x1DDD, 0x1DDE, 0x1DDF, 0x1DE0, 0x1DE1,
    0x1DE2, 0x1DE3, 0x1DE4, 0x1DE5, 0x1DE6, 0x1DFE, 0x20D0, 0x20D1,
    0x20D4, 0x20D5, 0x20D6, 0x20D7, 0x20DB, 0x20DC, 0x20E1, 0x20E7,
    0x20E9, 0x20F0, 0x2CEF, 0x2CF0, 0x2CF1, 0x2DE0, 0x2DE1, 0x2DE2,
    0x2DE3, 0x2DE4, 0x2DE5, 0x2DE6, 0x2DE7, 0x2DE8, 0x2DE9, 0x2DEA,
    0x2DEB, 0x2DEC, 0x2DED, 0x2DEE, 0x2DEF, 0x2DF0, 0x2DF1, 0x2DF2,
    0x2DF3, 0x2DF4, 0x2DF5, 0x2DF6, 0x2DF7, 0x2DF8, 0x2DF9, 0x2DFA,
    0x2DFB, 0x2DFC, 0x2DFD, 0x2DFE, 0x2DFF, 0xA66F, 0xA67C, 0xA67D,
    0xA6F0, 0xA6F1, 0xA8E0, 0xA8E1, 0xA8E2, 0xA8E3, 0xA8E4, 0xA8E5,
    0xA8E6, 0xA8E7, 0xA8E8, 0xA8E9, 0xA8EA, 0xA8EB, 0xA8EC, 0xA8ED,
    0xA8EE, 0xA8EF, 0xA8F0, 0xA8F1, 0xAAB0, 0xAAB2, 0xAAB3, 0xAAB7,
    0xAAB8, 0xAABE, 0xAABF, 0xAAC1, 0xFE20, 0xFE21, 0xFE22, 0xFE23,
    0xFE24, 0xFE25, 0xFE26, 0x10A0F, 0x10A38, 0x1D185, 0x1D186, 0x1D187,
    0x1D188, 0x1D189, 0x1D1AA, 0x1D1AB, 0x1D1AC, 0x1D1AD, 0x1D242, 0x1D243,
    0x1D244,
];

fn diacritic_index(c: char) -> Option<u32> {
    ROWCOLUMN_DIACRITICS
        .binary_search(&(c as u32))
        .ok()
        .map(|i| i as u32)
}

#[derive(Clone)]
struct ClipboardBridge {
    tx: mpsc::UnboundedSender<TermSideEffect>,
    window_size: Arc<AtomicU64>,
    /// 喂历史回放时置位：终端查询的应答不能再写回 PTY（当时已经答过了）。
    silent: Arc<AtomicBool>,
}

fn pack_window_size(cols: u16, rows: u16, cell_width: u16, cell_height: u16) -> u64 {
    ((cols as u64) << 48) | ((rows as u64) << 32) | ((cell_width as u64) << 16) | cell_height as u64
}

fn unpack_window_size(packed: u64) -> WindowSize {
    WindowSize {
        num_cols: (packed >> 48) as u16,
        num_lines: (packed >> 32) as u16,
        cell_width: (packed >> 16) as u16,
        cell_height: packed as u16,
    }
}

impl EventListener for ClipboardBridge {
    fn send_event(&self, event: Event) {
        match event {
            Event::ClipboardStore(_, text) if !text.is_empty() => {
                let _ = self.tx.send(TermSideEffect::ClipboardStore(text));
            }
            // 光标位置报告、DA1/DA2 设备属性、CSI 8t 字符格尺寸……alacritty_terminal 自己
            // 组好了应答文本，只需要回写进 PTY。
            Event::PtyWrite(text) if !self.silent.load(Ordering::Relaxed) => {
                let _ = self.tx.send(TermSideEffect::PtyWrite(text.into_bytes()));
            }
            // CSI 14t 报窗口像素尺寸：用最近一次 set_cell_pixel_size 写入的尺寸回答。拿不到
            // 真实像素尺寸的话（pixel_width/height=0）kitten icat 这类工具会直接拒绝发图。
            Event::TextAreaSizeRequest(formatter) if !self.silent.load(Ordering::Relaxed) => {
                let window_size = unpack_window_size(self.window_size.load(Ordering::Relaxed));
                let text = formatter(window_size);
                let _ = self.tx.send(TermSideEffect::PtyWrite(text.into_bytes()));
            }
            _ => {}
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
    /// cell 带 INVERSE 标志（fg/bg 已互换）。tmux copy-mode 的选区用这个表示。
    pub inverse: bool,
}

/// 占位符 cell 指向的图片子区域（Kitty Unicode Placeholder 协议解码结果）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageCellRef {
    pub image_id: u32,
    pub row: u32,
    pub col: u32,
}

#[derive(Clone, Debug)]
pub struct RenderRun {
    pub text: String,
    pub start_col: usize,
    pub cells: usize,
    pub style: RenderStyle,
    /// 非空时，这个 run 是一个图片占位符 cell（cells 总是 1），调用方应画图而不是画字形。
    pub image: Option<ImageCellRef>,
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

    pub fn new_with_clipboard(
        cols: u16,
        rows: u16,
    ) -> (Self, mpsc::UnboundedReceiver<TermSideEffect>) {
        let size = TermDim {
            columns: cols as usize,
            lines: rows as usize,
            scrollback: SCROLLBACK_LINES,
        };
        let (tx, rx) = mpsc::unbounded_channel();
        // 初始像素尺寸用个合理估值（和 term_view 的 FALLBACK_CELL_W/H 同数量级），
        // 真实字体度量出来后会很快通过 set_cell_pixel_size 更新。
        let window_size = Arc::new(AtomicU64::new(pack_window_size(cols, rows, 8, 17)));
        let silent = Arc::new(AtomicBool::new(false));
        let term = Term::new(
            Default::default(),
            &size,
            ClipboardBridge {
                tx,
                window_size: Arc::clone(&window_size),
                silent: Arc::clone(&silent),
            },
        );
        (
            VTerm {
                inner: Arc::new(Mutex::new(VTermInner {
                    term,
                    parser: Processor::new(),
                    kitty: KittyGraphicsScanner::new(),
                    cached: None,
                    damage: ContentDamage::Full,
                })),
                window_size,
                silent,
                cols,
                rows,
            },
            rx,
        )
    }

    /// 喂历史回放字节：和 feed 一样解析，但不把终端查询（CSI 6n / 14t / DA…）的应答写回 PTY。
    /// 回放里的查询在它第一次被输出时就已经回答过了；再答一次会变成一串垃圾字符落进
    /// 前台程序（比如 pi 的输入框里冒出 `?6c>0;2600;1c8;32;120t4;544;960t`）。
    pub fn feed_silent(&self, data: &[u8]) {
        self.silent.store(true, Ordering::Relaxed);
        self.feed(data);
        self.silent.store(false, Ordering::Relaxed);
    }

    /// 终端区域的实际行列数 + 每个格子的真实像素尺寸（GPUI 字体度量结果），
    /// 配合 resize 一起调。不调的话 `CSI 14t` 等查询回答的是默认估值，不准确但不会卡住。
    pub fn set_window_pixel_geometry(
        &self,
        cols: u16,
        rows: u16,
        cell_width: u16,
        cell_height: u16,
    ) {
        self.window_size.store(
            pack_window_size(cols, rows, cell_width, cell_height),
            Ordering::Relaxed,
        );
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
            let VTermInner {
                term,
                parser,
                kitty,
                ..
            } = &mut *guard;
            // Kitty 图片 APC 序列（ESC _G...ESC \）vte 0.13 不认识，会被直接吸掉；
            // 先拦下来自己解析，剩下的字节再交给 alacritty 正常处理。
            let filtered = kitty.process(data);
            parser.advance(term, &filtered);
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
        let fallback_id = guard.kitty.latest_placement_id();
        let need_full = guard.cached.is_none()
            || matches!(damage, ContentDamage::Full)
            || guard.cached.as_ref().is_some_and(|c| {
                c.cols != guard.term.columns() || c.lines != guard.term.screen_lines()
            });

        let mut snap = if need_full {
            Arc::new(build_snapshot(&guard.term, fallback_id))
        } else {
            guard.cached.take().unwrap()
        };
        if let ContentDamage::Partial(rows) = damage {
            // copy-on-write：无其他持有者时原地更新，有持有者（渲染中）时克隆一份再改。
            let snap = Arc::make_mut(&mut snap);
            for row in rows {
                if row < snap.rows.len() {
                    snap.rows[row] = build_row(&guard.term, row, fallback_id);
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

    /// 取一张已经接收完整的 Kitty 图片（用 Unicode Placeholder 解码出来的 image id 查）。
    pub fn kitty_image(&self, id: u32) -> Option<Arc<StoredImage>> {
        self.lock_inner()?.kitty.image(id)
    }

    /// 取一张图的虚拟占位网格尺寸（cols, rows），用于把整张图切块。
    pub fn kitty_placement_size(&self, id: u32) -> Option<(u32, u32)> {
        self.lock_inner()?.kitty.placement_size(id)
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

fn build_snapshot(term: &Term<ClipboardBridge>, fallback_id: Option<u32>) -> RenderSnapshot {
    let rows = (0..term.screen_lines())
        .map(|r| build_row(term, r, fallback_id))
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

fn build_row(term: &Term<ClipboardBridge>, visual: usize, fallback_id: Option<u32>) -> RenderRow {
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
    // Kitty Unicode Placeholder 的行/列/MSB 变音符可以省略，继承左侧 placeholder cell 的值。
    let mut prev_placeholder: Option<(Color, u32, u32, u32)> = None;
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
            inverse: cell.flags.contains(Flags::INVERSE),
        };
        let ch = if cell.flags.contains(Flags::HIDDEN) {
            ' '
        } else {
            cell.c
        };

        if ch == KITTY_PLACEHOLDER {
            // image id 编码在 cell 真实的前景色里；tmux 选区/反显用的是 INVERSE 标志，
            // 上面已经把 fg/bg 换了位，这里必须用 cell.fg 而不是换位后的 fg。
            if let Some((image_ref, id_fallback)) = decode_placeholder(
                cell.fg,
                cell.zerowidth(),
                &mut prev_placeholder,
                fallback_id,
            ) {
                if let Some(r) = current.take() {
                    runs.push(r);
                }
                // id 是从最近占位里兜底出来的（前景色被 tmux 选区改了）：
                // 把这个 cell 标记为选中，画上选区高亮。
                let mut style = style;
                style.selected |= id_fallback;
                runs.push(RenderRun {
                    text: String::new(),
                    start_col: col,
                    cells: 1,
                    style,
                    image: Some(image_ref),
                });
                continue;
            }
        }
        prev_placeholder = None;

        let append = current
            .as_ref()
            .is_some_and(|r| r.image.is_none() && r.style == style && r.start_col + r.cells == col);
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
                image: None,
            });
        }
    }
    if let Some(r) = current {
        runs.push(r);
    }
    RenderRow { runs }
}

/// Kitty Unicode Placeholder 解码：从占位符 cell 的前景色 + 变音符里换出 (image_id, row, col)。
/// `prev` 是左边上一个已解码的 placeholder 单元格（fg, row, col, msb），用于补全被省略的变音符；
/// 解码成功后会就地更新它，下一个 cell 继续继承。
fn decode_placeholder(
    fg: Color,
    zerowidth: Option<&[char]>,
    prev: &mut Option<(Color, u32, u32, u32)>,
    fallback_id: Option<u32>,
) -> Option<(ImageCellRef, bool)> {
    let diacritics = zerowidth.unwrap_or(&[]);
    // 前景色被外部改掉（tmux copy-mode 选区色）时 color_base_id 拿不到 id：
    // 只要行/列变音符还在，就用最近建过占位的 image id 兑底。
    let (base_id, id_fallback) = match color_base_id(fg) {
        Some(id) => (id, false),
        None => match (
            fallback_id,
            diacritics.first().copied().and_then(diacritic_index),
            diacritics.get(1).copied().and_then(diacritic_index),
        ) {
            (Some(id), Some(_), Some(_)) => (id, true),
            _ => {
                *prev = None;
                return None;
            }
        },
    };
    let row_d = diacritics.first().copied().and_then(diacritic_index);
    let col_d = diacritics.get(1).copied().and_then(diacritic_index);
    let msb_d = diacritics.get(2).copied().and_then(diacritic_index);

    let same_image = prev.as_ref().is_some_and(|(prev_fg, ..)| *prev_fg == fg);
    let row = match (row_d, same_image) {
        (Some(v), _) => v,
        (None, true) => prev.unwrap().1,
        (None, false) => return None,
    };
    let col = match (col_d, same_image) {
        (Some(v), _) => v,
        (None, true) => prev.unwrap().2 + 1,
        (None, false) => return None,
    };
    let msb = match (msb_d, same_image) {
        (Some(v), _) => v,
        (None, true) => prev.unwrap().3,
        (None, false) => 0,
    };

    *prev = Some((fg, row, col, msb));
    Some((
        ImageCellRef {
            image_id: base_id | (msb << 24),
            row,
            col,
        },
        id_fallback,
    ))
}

/// 前景色直接编码的 image id 低位部分：真彩色取 24 位 RGB，256 色取索引值。
/// `Color::Named` 无法表示图片 id，视为不是 placeholder。
fn color_base_id(c: Color) -> Option<u32> {
    match c {
        Color::Spec(Rgb { r, g, b }) => Some(((r as u32) << 16) | ((g as u32) << 8) | b as u32),
        Color::Indexed(i) => Some(i as u32),
        Color::Named(_) => None,
    }
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
    fn cursor_position_report_is_forwarded_as_pty_write() {
        // CSI 6n 是光标位置报告查询，alacritty_terminal 会自己组好 `ESC[row;colR` 并通过
        // Event::PtyWrite 回调，之前 ClipboardBridge 直接丢掉了这类事件，导致查询永远等不到回复。
        let (vterm, mut rx) = VTerm::new_with_clipboard(80, 24);
        vterm.feed(b"\x1b[6n");
        let effect = rx.try_recv().expect("cursor position report forwarded");
        match effect {
            TermSideEffect::PtyWrite(bytes) => {
                assert!(bytes.starts_with(b"\x1b["), "got {bytes:?}");
                assert!(bytes.ends_with(b"R"), "got {bytes:?}");
            }
            other => panic!("expected PtyWrite, got {other:?}"),
        }
    }

    #[test]
    fn window_pixel_size_query_uses_set_geometry() {
        // CSI 14t 报窗口像素尺寸：kitten icat 等 Kitty 图形客户端靠这个查询判断终端能不能显图，
        // 拿到 0x0 就直接拒绝。
        let (vterm, mut rx) = VTerm::new_with_clipboard(80, 24);
        vterm.set_window_pixel_geometry(80, 24, 9, 18);
        vterm.feed(b"\x1b[14t");
        let effect = rx.try_recv().expect("window size report forwarded");
        match effect {
            TermSideEffect::PtyWrite(bytes) => {
                // 格式 `ESC[4;height;widtht`，height=24*18=432, width=80*9=720。
                assert_eq!(bytes, b"\x1b[4;432;720t");
            }
            other => panic!("expected PtyWrite, got {other:?}"),
        }
    }

    #[test]
    fn feed_silent_parses_but_does_not_answer_queries() {
        // 历史回放里的 CSI 6n 不能再写回 PTY，否则应答会变成键盘输入落进前台程序。
        let (vterm, mut rx) = VTerm::new_with_clipboard(80, 24);
        vterm.feed_silent(b"hello\x1b[6n\x1b[14t");
        assert!(rx.try_recv().is_err(), "silent feed must not emit PtyWrite");
        assert_eq!(vterm.line_text(0).unwrap().trim_end(), "hello");
        // 之后的正常 feed 恢复应答。
        vterm.feed(b"\x1b[6n");
        assert!(matches!(rx.try_recv(), Ok(TermSideEffect::PtyWrite(_))));
    }

    #[test]
    fn osc52_clipboard_store_is_emitted() {
        let (vterm, mut rx) = VTerm::new_with_clipboard(80, 24);
        let payload = muxlane_core::protocol::b64_encode(b"copied-from-tmux");
        vterm.feed(format!("\x1b]52;c;{payload}\x07").as_bytes());
        let effect = rx.try_recv().expect("osc52 clipboard event");
        match effect {
            TermSideEffect::ClipboardStore(text) => assert_eq!(text, "copied-from-tmux"),
            other => panic!("expected ClipboardStore, got {other:?}"),
        }
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

#[cfg(test)]
mod kitty_placeholder_tests {
    use super::*;

    fn placeholder(diacritics: &[char]) -> String {
        let mut s = String::from(KITTY_PLACEHOLDER);
        s.extend(diacritics.iter());
        s
    }

    /// 直接搭 kitty 官方文档里的 2x2 示例（image id 42，256 色模式）。
    #[test]
    fn unicode_placeholder_2x2_grid_decodes_row_col_and_id() {
        let vterm = VTerm::new(80, 24);
        let row0 = format!(
            "\x1b[38;5;42m{}{}\x1b[39m\r\n",
            placeholder(&['\u{0305}', '\u{0305}']), // row=0, col=0
            placeholder(&['\u{0305}', '\u{030D}']), // row=0, col=1
        );
        let row1 = format!(
            "\x1b[38;5;42m{}{}\x1b[39m\r\n",
            placeholder(&['\u{030D}', '\u{0305}']), // row=1, col=0
            placeholder(&['\u{030D}', '\u{030D}']), // row=1, col=1
        );
        vterm.feed(row0.as_bytes());
        vterm.feed(row1.as_bytes());

        let snap = vterm.render_snapshot();
        let images_in = |row: usize| -> Vec<ImageCellRef> {
            snap.rows[row].runs.iter().filter_map(|r| r.image).collect()
        };
        assert_eq!(
            images_in(0),
            vec![
                ImageCellRef {
                    image_id: 42,
                    row: 0,
                    col: 0
                },
                ImageCellRef {
                    image_id: 42,
                    row: 0,
                    col: 1
                },
            ]
        );
        assert_eq!(
            images_in(1),
            vec![
                ImageCellRef {
                    image_id: 42,
                    row: 1,
                    col: 0
                },
                ImageCellRef {
                    image_id: 42,
                    row: 1,
                    col: 1
                },
            ]
        );
    }

    /// MSB 变音符（第三个）把 image id 扩展到 24 位以上：42 + (2<<24) = 33554474。
    #[test]
    fn msb_diacritic_extends_image_id_beyond_24_bits() {
        let vterm = VTerm::new(80, 24);
        let line = format!(
            "\x1b[38;5;42m{}\x1b[39m\r\n",
            placeholder(&['\u{0305}', '\u{0305}', '\u{030E}']), // row=0,col=0,msb=2
        );
        vterm.feed(line.as_bytes());

        let snap = vterm.render_snapshot();
        let image = snap.rows[0].runs.iter().find_map(|r| r.image).unwrap();
        assert_eq!(image.image_id, 42 + (2 << 24));
        assert_eq!((image.row, image.col), (0, 0));
    }

    /// 省略的变音符从左侧 placeholder cell 继承：行号继承，列号 = 左侧 + 1。
    #[test]
    fn omitted_diacritics_inherit_from_left_placeholder_cell() {
        let vterm = VTerm::new(80, 24);
        let line = format!(
            "\x1b[38;5;7m{}{}\x1b[39m\r\n",
            placeholder(&['\u{030D}', '\u{0305}']), // row=1, col=0
            placeholder(&[]),                       // 全部省略 -> row=1, col=1
        );
        vterm.feed(line.as_bytes());

        let snap = vterm.render_snapshot();
        let images: Vec<ImageCellRef> = snap.rows[0].runs.iter().filter_map(|r| r.image).collect();
        assert_eq!(
            images,
            vec![
                ImageCellRef {
                    image_id: 7,
                    row: 1,
                    col: 0
                },
                ImageCellRef {
                    image_id: 7,
                    row: 1,
                    col: 1
                },
            ]
        );
    }

    /// 普通文字不受影响，不会被误识别成图片占位符。
    #[test]
    fn plain_text_has_no_image_ref() {
        let vterm = VTerm::new(80, 24);
        vterm.feed(b"hello world\r\n");
        let snap = vterm.render_snapshot();
        assert!(snap.rows[0].runs.iter().all(|r| r.image.is_none()));
    }

    /// tmux copy-mode 选中用 SGR 7（INVERSE）反显，fg/bg 互换后 image id 不能丢。
    #[test]
    fn inverse_video_keeps_placeholder_image_id() {
        let vterm = VTerm::new(80, 24);
        let line = format!(
            "\x1b[7m\x1b[38;5;42m{}\x1b[0m\r\n",
            placeholder(&['\u{0305}', '\u{0305}']),
        );
        vterm.feed(line.as_bytes());
        let snap = vterm.render_snapshot();
        let run = snap.rows[0]
            .runs
            .iter()
            .find(|r| r.image.is_some())
            .expect("still an image run");
        assert_eq!(run.image.unwrap().image_id, 42);
        assert!(run.style.inverse);
    }

    /// tmux copy-mode 选区会把占位符 cell 的前景色改成选区色（Named），fg 里解不出 id：
    /// 只要行/列变音符还在，就用最近建过占位的 id 兜底，并标记为选中。
    #[test]
    fn clobbered_fg_falls_back_to_latest_placement_id() {
        let vterm = VTerm::new(80, 24);
        // 先有 U=1 占位命令登记 id=42，再用命名前景色（白色）发占位符。
        vterm.feed(b"\x1b_Ga=p,i=42,U=1,c=1,r=1,q=2;\x1b\\");
        let line = format!(
            "\x1b[37m{}\x1b[0m\r\n",
            placeholder(&['\u{0305}', '\u{0305}']),
        );
        vterm.feed(line.as_bytes());
        let snap = vterm.render_snapshot();
        let run = snap.rows[0]
            .runs
            .iter()
            .find(|r| r.image.is_some())
            .expect("still an image run");
        assert_eq!(run.image.unwrap().image_id, 42);
        assert!(run.style.selected);
    }
}
