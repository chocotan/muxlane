import { describe, expect, it } from "vitest";
import { resolveCustomTerminalSequence } from "./terminalKeys";

function ev(
  key: string,
  mods: { shift?: boolean; alt?: boolean; ctrl?: boolean; meta?: boolean } = {},
) {
  return {
    key,
    shiftKey: Boolean(mods.shift),
    altKey: Boolean(mods.alt),
    ctrlKey: Boolean(mods.ctrl),
    metaKey: Boolean(mods.meta),
  };
}

describe("resolveCustomTerminalSequence", () => {
  it("encodes Alt+Arrow keys using real xterm modifier 3 (not Ctrl 5)", () => {
    expect(resolveCustomTerminalSequence(ev("ArrowUp", { alt: true }))).toBe("\x1b[1;3A");
    expect(resolveCustomTerminalSequence(ev("ArrowDown", { alt: true }))).toBe("\x1b[1;3B");
    expect(resolveCustomTerminalSequence(ev("ArrowRight", { alt: true }))).toBe("\x1b[1;3C");
    expect(resolveCustomTerminalSequence(ev("ArrowLeft", { alt: true }))).toBe("\x1b[1;3D");
    expect(resolveCustomTerminalSequence(ev("ArrowUp", { alt: true, shift: true }))).toBe(
      "\x1b[1;4A",
    );
    expect(resolveCustomTerminalSequence(ev("ArrowUp", { alt: true, ctrl: true }))).toBe(
      "\x1b[1;7A",
    );
  });

  it("leaves plain and Ctrl-only arrows to xterm.js", () => {
    expect(resolveCustomTerminalSequence(ev("ArrowUp"))).toBeNull();
    expect(resolveCustomTerminalSequence(ev("ArrowUp", { ctrl: true }))).toBeNull();
  });

  it("encodes Alt+PageUp and Alt+PageDown without dropping Alt", () => {
    expect(resolveCustomTerminalSequence(ev("PageUp", { alt: true }))).toBe("\x1b[5;3~");
    expect(resolveCustomTerminalSequence(ev("PageDown", { alt: true }))).toBe("\x1b[6;3~");
    expect(resolveCustomTerminalSequence(ev("PageUp", { shift: true }))).toBeNull();
  });

  it("encodes Shift+Enter as LF newline and Ctrl+Enter as CSI u", () => {
    expect(resolveCustomTerminalSequence(ev("Enter", { shift: true }))).toBe("\n");
    expect(resolveCustomTerminalSequence(ev("Enter", { ctrl: true }))).toBe("\x1b[13;5u");
    expect(resolveCustomTerminalSequence(ev("Enter"))).toBeNull();
  });
});
