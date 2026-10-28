//! muxlane-term：PTY 会话 + 终端模拟（与 GPUI 解耦）
mod kitty_graphics;
mod diff;
mod replay;
mod session;
mod vterm;

pub use kitty_graphics::{KittyGraphicsScanner, StoredImage};
pub use diff::{DiffFrame, TermDiff};
pub use replay::ReplayBuffer;
pub use session::{default_shell_program, LaunchCfg, PtySession, SessionEvent};
pub use vterm::{
    ImageCellRef, RenderCursor, RenderSnapshot, TermSideEffect, VTerm, VTermModes, DEFAULT_COLOR,
};
