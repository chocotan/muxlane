//! Terminal frame diff: keyframes plus P-frame style patches.
//!
//! A full-screen TUI often repaints the whole viewport even when only one row
//! changed. Feeding that raw stream over the network costs bandwidth for pixels
//! that did not move. This mirrors the same PTY stream into a `VTerm` and turns
//! it into either a synthesized full frame (keyframe) or only the runs that
//! actually changed (patch).
//!
//! The client needs no change: patches are ordinary escape sequences, so the
//! viewer's VTerm applies them exactly like the original output.
use crate::vterm::{RenderRun, RenderSnapshot, RenderStyle, VTerm, DEFAULT_COLOR};
use std::io::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A full frame at least this often lets a drifted client self-heal.
const KEYFRAME_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub enum DiffFrame {
    /// Forward the original bytes unchanged: control sequences need to reach the
    /// viewer verbatim, and non full-screen sessions have scrollback to preserve.
    Raw(Vec<u8>),
    /// A synthesized full-screen redraw.
    Full(Vec<u8>),
    /// A synthesized patch covering only the runs that changed.
    Patch(Vec<u8>),
}

impl DiffFrame {
    pub fn into_bytes(self) -> Vec<u8> {
        match self {
            DiffFrame::Raw(bytes) | DiffFrame::Full(bytes) | DiffFrame::Patch(bytes) => bytes,
        }
    }
}

pub struct TermDiff {
    vterm: VTerm,
    prev: Option<Arc<RenderSnapshot>>,
    last_full: Option<Instant>,
    enabled: bool,
}

impl TermDiff {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            vterm: VTerm::new(cols, rows),
            prev: None,
            last_full: None,
            enabled: !matches!(
                std::env::var("MUXLANE_TERM_DIFF").as_deref(),
                Ok("off") | Ok("0") | Ok("false")
            ),
        }
    }

    pub fn size(&self) -> (u16, u16) {
        (self.vterm.cols, self.vterm.rows)
    }

    /// Resizing invalidates every cell: the next frame is a full keyframe.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        if self.size() == (cols, rows) {
            return;
        }
        self.vterm.resize(cols, rows);
        self.prev = None;
        self.last_full = None;
    }

    /// Seed the mirror from a replay/resync snapshot; the next frame is full.
    /// `alt_screen` is tracked by the PTY reader because a replay tail rarely
    /// contains the enter sequence, and scrolling must not look possible.
    pub fn reset_and_feed(&mut self, bytes: &[u8], alt_screen: bool) {
        self.vterm.feed(b"\x1bc");
        if alt_screen {
            self.vterm.feed(b"\x1b[?1049h");
        }
        self.vterm.feed_silent(bytes);
        self.prev = None;
        self.last_full = None;
    }

    /// Feed one live chunk and decide what the viewer should receive.
    pub fn feed(&mut self, bytes: &[u8]) -> DiffFrame {
        let history_before = self.vterm.history_len();
        self.vterm.feed(bytes);
        // Scrolling output must stay raw: the viewer needs it to grow scrollback,
        // and a patch stream only rewrites the visible screen. Full-screen TUIs
        // never scroll, so they still get patches.
        let scrolled = self.vterm.history_len() != history_before;
        if !self.enabled || scrolled || contains_control(bytes) {
            self.prev = Some(self.vterm.render_snapshot());
            return DiffFrame::Raw(bytes.to_vec());
        }
        let snapshot = self.vterm.render_snapshot();
        let keyframe = match self.last_full {
            None => true,
            Some(last) => last.elapsed() >= KEYFRAME_INTERVAL,
        };
        if keyframe {
            self.prev = Some(Arc::clone(&snapshot));
            self.last_full = Some(Instant::now());
            return DiffFrame::Full(render_full(&snapshot));
        }
        let previous = self
            .prev
            .as_ref()
            .expect("prev is set whenever a keyframe is not due");
        let patch = render_patch(previous, &snapshot);
        self.prev = Some(snapshot);
        DiffFrame::Patch(patch)
    }
}

/// Anything that is not plain text/SGR/cursor movement must go through untouched.
/// The viewer answers cursor/colour queries itself, so swallowing them here would
/// hang the program that asked.
fn contains_control(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != 0x1b {
            i += 1;
            continue;
        }
        let Some(&next) = bytes.get(i + 1) else {
            return true;
        };
        match next {
            // Charset selection (`ESC ( B`, `ESC ) 0`, ...) is decoded into the
            // mirror's cells and re-emitted as UTF-8, so skip it.
            b'(' | b')' | b'*' | b'+' => {
                i += 3;
                continue;
            }
            // OSC, DCS, APC, PM, SOS, RIS, DECALN.
            b']' | b'P' | b'_' | b'^' | b'X' | b'c' | b'#' => return true,
            b'[' => {
                if bytes.get(i + 2) == Some(&b'?') {
                    return true;
                }
                let mut j = i + 2;
                while j < bytes.len() {
                    let byte = bytes[j];
                    if (0x40..=0x7e).contains(&byte) {
                        // Mode set/reset, device queries and screen clears all carry
                        // state the mirror cannot safely hide from the viewer.
                        if matches!(byte, b'h' | b'l' | b'c' | b'n') {
                            return true;
                        }
                        if byte == b'J' {
                            let params = &bytes[i + 2..j];
                            if params.contains(&b'2') || params.contains(&b'3') {
                                return true;
                            }
                        }
                        break;
                    }
                    j += 1;
                }
                i = j.saturating_add(1);
                continue;
            }
            _ => return true,
        }
    }
    false
}

fn push_color(out: &mut Vec<u8>, color: u32, foreground: bool) {
    if color == DEFAULT_COLOR {
        out.extend_from_slice(if foreground { b";39" } else { b";49" });
        return;
    }
    let red = (color >> 24) & 0xff;
    let green = (color >> 16) & 0xff;
    let blue = (color >> 8) & 0xff;
    let _ = write!(
        out,
        ";{};2;{};{};{}",
        if foreground { 38 } else { 48 },
        red,
        green,
        blue
    );
}

fn write_sgr(out: &mut Vec<u8>, style: &RenderStyle) {
    out.extend_from_slice(b"\x1b[0");
    if style.bold {
        out.extend_from_slice(b";1");
    }
    if style.dim {
        out.extend_from_slice(b";2");
    }
    if style.italic {
        out.extend_from_slice(b";3");
    }
    if style.underline {
        out.extend_from_slice(b";4");
    }
    // `build_row` already swapped fg/bg for INVERSE cells; swap back and let the
    // viewer's VTerm apply INVERSE itself, so its cell matches the mirror.
    if style.inverse {
        out.extend_from_slice(b";7");
        push_color(out, style.bg, true);
        push_color(out, style.fg, false);
    } else {
        push_color(out, style.fg, true);
        push_color(out, style.bg, false);
    }
    out.push(b'm');
}

/// Trailing spaces only need cells when they carry a visible background; the
/// line was just erased to the default background otherwise.
fn visible_text(run: &RenderRun) -> &str {
    if run.style.bg == DEFAULT_COLOR && !run.style.inverse {
        run.text.trim_end_matches(' ')
    } else {
        &run.text
    }
}

fn render_row(out: &mut Vec<u8>, row: usize, runs: &[RenderRun]) {
    // Reset SGR, then erase to the default background. Positioned writes never
    // scroll, so the app's scroll region is left alone.
    let _ = write!(out, "\x1b[0m\x1b[{};1H\x1b[2K", row + 1);
    for run in runs {
        let text = visible_text(run);
        if text.is_empty() {
            continue;
        }
        let _ = write!(out, "\x1b[{};{}H", row + 1, run.start_col + 1);
        write_sgr(out, &run.style);
        out.extend_from_slice(text.as_bytes());
    }
}

fn push_cursor(out: &mut Vec<u8>, snapshot: &RenderSnapshot) {
    if let Some(cursor) = snapshot.cursor.as_ref() {
        let _ = write!(out, "\x1b[{};{}H", cursor.row + 1, cursor.col + 1);
    }
}

fn render_full(snapshot: &RenderSnapshot) -> Vec<u8> {
    let mut out = Vec::with_capacity(snapshot.rows.len() * 64);
    // Select ASCII so the UTF-8 runs below are not read through a legacy charset.
    out.extend_from_slice(b"\x1b(B\x1b[0m");
    for (row, line) in snapshot.rows.iter().enumerate() {
        render_row(&mut out, row, &line.runs);
    }
    push_cursor(&mut out, snapshot);
    out
}

fn render_patch(previous: &RenderSnapshot, snapshot: &RenderSnapshot) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"\x1b(B");
    for (row, (old, new)) in previous
        .rows
        .iter()
        .zip(snapshot.rows.iter())
        .enumerate()
    {
        if old == new {
            continue;
        }
        // Same run layout: overwrite only the runs whose text or style moved.
        let aligned = old.runs.len() == new.runs.len()
            && old
                .runs
                .iter()
                .zip(new.runs.iter())
                .all(|(a, b)| a.start_col == b.start_col && a.cells == b.cells);
        if aligned {
            for (a, b) in old.runs.iter().zip(new.runs.iter()) {
                if a == b {
                    continue;
                }
                let _ = write!(out, "\x1b[{};{}H", row + 1, b.start_col + 1);
                write_sgr(&mut out, &b.style);
                out.extend_from_slice(b.text.as_bytes());
            }
        } else {
            render_row(&mut out, row, &new.runs);
        }
    }
    push_cursor(&mut out, snapshot);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed `raw` into a mirror and a viewer, applying whatever the diff emits,
    /// then compare the two screens. This is the whole contract.
    fn assert_viewer_matches(raw: &[&[u8]]) -> (usize, usize) {
        let mut diff = TermDiff::new(40, 12);
        let viewer = VTerm::new(40, 12);
        let mut sent = 0;
        for chunk in raw {
            let frame = diff.feed(chunk);
            let bytes = frame.into_bytes();
            sent += bytes.len();
            viewer.feed(&bytes);
        }
        let mirror = diff.vterm.render_snapshot();
        let seen = viewer.render_snapshot();
        assert_eq!(seen.rows, mirror.rows, "viewer screen must match mirror");
        (sent, raw.iter().map(|chunk| chunk.len()).sum())
    }

    #[test]
    fn full_screen_repaints_become_small_patches() {
        // Enter the alternate screen and paint a 12-row status block.
        let mut enter = Vec::new();
        enter.extend_from_slice(b"\x1b[?1049h\x1b[H\x1b[2J");
        for row in 0..12 {
            let _ = write!(enter, "\x1b[{};1HTodo 0/10  row {}", row + 1, row);
        }
        let _ = write!(&mut enter, "\x1b[12;1H");
        let chunk = enter.as_slice();

        // Each frame repaints the whole viewport but only the last row's counter moves.
        let frames: Vec<Vec<u8>> = (0..8)
            .map(|tick| {
                let mut frame = Vec::new();
                frame.extend_from_slice(b"\x1b[H");
                for row in 0..12 {
                    let _ = write!(frame, "\x1b[{};1HTodo 0/10  row {}\x1b[K", row + 1, row);
                }
                let _ = write!(frame, "\x1b[12;1Htick {tick:02}\x1b[K");
                frame
            })
            .collect();
        let mut chunks: Vec<&[u8]> = vec![chunk];
        chunks.extend(frames.iter().map(|frame| frame.as_slice()));

        let (sent, raw) = assert_viewer_matches(&chunks);
        // The keyframe plus seven one-row patches must beat eight full repaints.
        assert!(sent < raw, "diff sent {sent} vs raw {raw}");
        assert!(sent * 2 < raw, "expected a large win, sent {sent} vs raw {raw}");
    }

    #[test]
    fn control_sequences_pass_through_untouched() {
        let mut diff = TermDiff::new(20, 6);
        diff.feed(b"\x1b[?1049h");
        let osc = b"\x1b]0;title\x07";
        match diff.feed(osc) {
            DiffFrame::Raw(bytes) => assert_eq!(bytes, osc),
            other => panic!("OSC must be raw, got {other:?}"),
        }
        let query = b"\x1b[6n";
        match diff.feed(query) {
            DiffFrame::Raw(bytes) => assert_eq!(bytes, query),
            other => panic!("DSR must be raw, got {other:?}"),
        }
    }

    #[test]
    fn scrolling_output_keeps_raw_bytes() {
        let mut diff = TermDiff::new(20, 6);
        // Enough newlines to push lines into the scrollback.
        let text = b"\r\n\r\n\r\n\r\n\r\n\r\n\r\n\r\n";
        match diff.feed(text) {
            DiffFrame::Raw(bytes) => assert_eq!(bytes, text),
            other => panic!("scrolling sessions must stay raw, got {other:?}"),
        }
    }

    #[test]
    fn alternate_screen_seed_keeps_newlines_patchable() {
        let mut diff = TermDiff::new(20, 6);
        diff.reset_and_feed(b"prompt$ pie", true);
        let first = b"\x1b[Hline1\r\nline2\r\nline3\r\nline4\r\nline5\r\nline6\r\nline7";
        match diff.feed(first) {
            DiffFrame::Full(_) => {}
            other => panic!("first frame after seed must be full, got {other:?}"),
        }
        // Same number of newlines would scroll a normal screen; alt-screen must not.
        let second =
            b"\x1b[Hline1\r\nline2\r\nline3\r\nline4\r\nline5\r\nline6\r\nline8";
        match diff.feed(second) {
            DiffFrame::Patch(_) => {}
            other => panic!("alt-screen frame must be a patch, got {other:?}"),
        }
    }

    #[test]
    fn charset_selection_does_not_force_raw() {
        let mut diff = TermDiff::new(20, 6);
        diff.feed(b"\x1b[?1049h");
        diff.feed(b"\x1b[Hhello\x1b(B");
        match diff.feed(b"\x1b[Hhello\x1b(B") {
            DiffFrame::Patch(_) => {}
            other => panic!("charset-only chunks must stay patchable, got {other:?}"),
        }
    }

    #[test]
    fn resize_forces_a_full_keyframe() {
        let mut diff = TermDiff::new(20, 6);
        diff.feed(b"\x1b[?1049h");
        diff.feed(b"\x1b[Hhello");
        let _ = diff.feed(b"\x1b[Hworld");
        diff.resize(30, 8);
        match diff.feed(b"\x1b[Hresized") {
            DiffFrame::Full(_) => {}
            other => panic!("first frame after resize must be full, got {other:?}"),
        }
    }
}
