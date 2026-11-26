/**
 * Terminal key sequence overrides for `@xterm/xterm` on Linux/Electron.
 *
 * Fixes known `@xterm/xterm` encoding bugs and aligns with `crates/muxlane-app/src/terminal_keys.rs`:
 * 1. On Linux (`!isMac`), `@xterm/xterm` rewrites `Alt+Arrow*` (`\x1b[1;3A..D`) into `Ctrl+Arrow*`
 *    (`\x1b[1;5A..D`), breaking `Alt+Up` / `Alt+Down` / `Alt+Left` / `Alt+Right` in pi/pie/agy/zsh/tmux.
 * 2. `@xterm/xterm` drops the `Alt` modifier on `PageUp` / `PageDown`.
 * 3. `@xterm/xterm` sends `\r` for `Shift+Enter` instead of `\n` (newline in agent TUIs).
 */

export interface MinimalKeyboardEvent {
  key: string;
  shiftKey: boolean;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
}

const ARROW_FINALS: Record<string, string> = {
  ArrowUp: "A",
  ArrowDown: "B",
  ArrowRight: "C",
  ArrowLeft: "D",
};

const HOME_END_FINALS: Record<string, string> = {
  Home: "H",
  End: "F",
};

const PAGE_PARAMS: Record<string, number> = {
  PageUp: 5,
  PageDown: 6,
};

function xtermModifierCode(e: MinimalKeyboardEvent): number {
  return 1 + (e.shiftKey ? 1 : 0) + (e.altKey ? 2 : 0) + (e.ctrlKey ? 4 : 0);
}

/**
 * Return an explicit escape sequence when `@xterm/xterm`'s built-in Linux encoder is wrong
 * or incomplete; return `null` to let `@xterm/xterm` handle the event natively.
 */
export function resolveCustomTerminalSequence(e: MinimalKeyboardEvent): string | null {
  if (e.metaKey) return null;

  // 1. Alt-modified arrow keys (xterm.js rewrites Alt+Arrow to Ctrl+Arrow on Linux!)
  const arrowFinal = ARROW_FINALS[e.key];
  if (arrowFinal && e.altKey) {
    return `\x1b[1;${xtermModifierCode(e)}${arrowFinal}`;
  }

  // 2. Alt-modified Home/End
  const homeEndFinal = HOME_END_FINALS[e.key];
  if (homeEndFinal && e.altKey) {
    return `\x1b[1;${xtermModifierCode(e)}${homeEndFinal}`;
  }

  // 3. Alt-modified PageUp/PageDown (xterm.js drops Alt on PageUp/PageDown)
  const pageParam = PAGE_PARAMS[e.key];
  if (pageParam !== undefined && e.altKey && !e.shiftKey) {
    return `\x1b[${pageParam};${xtermModifierCode(e)}~`;
  }

  // 4. Shift+Enter -> LF newline (matches crates/muxlane-app/src/terminal_keys.rs)
  if (e.key === "Enter" && e.shiftKey && !e.ctrlKey && !e.altKey) {
    return "\n";
  }

  // 5. Ctrl+Enter -> CSI u (supported by tmux extended-keys=on, format=csi-u)
  if (e.key === "Enter" && e.ctrlKey && !e.shiftKey && !e.altKey) {
    return "\x1b[13;5u";
  }

  return null;
}
