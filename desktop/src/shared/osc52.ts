/**
 * OSC 52 clipboard sequence parser for xterm.js (`ESC ] 52 ; <selection> ; <payload> BEL/ST`).
 *
 * Remote tmux (`set-clipboard external/on`) and remote TUI apps (vim/neovim/claude/pi)
 * emit OSC 52 to copy text back to the local desktop clipboard over SSH/daemon streams.
 */

export const MAX_OSC52_DECODED_BYTES = 1024 * 1024; // 1 MiB safety cap

export interface ParsedOsc52 {
  /** Target selections specified in the OSC 52 parameter (e.g. "c", "p", "pc"). */
  selection: string;
  /** Decoded UTF-8 clipboard text. */
  text: string;
  /** Whether to also mirror into X11 primary selection on Linux. */
  alsoSelection: boolean;
}

const BASE64_RE = /^[A-Za-z0-9+/]*={0,2}$/;

function decodeBase64ToUtf8(cleanB64: string, maxDecodedBytes: number): string | null {
  if (!cleanB64) return null;

  // Pad missing trailing '=' if an emitter omits padding
  const rem = cleanB64.length % 4;
  if (rem === 1) return null;
  const padded = rem === 0 ? cleanB64 : cleanB64 + "=".repeat(4 - rem);

  if (!BASE64_RE.test(padded)) return null;

  const estimatedBytes = Math.floor((padded.length * 3) / 4);
  if (estimatedBytes > maxDecodedBytes + 2) return null;

  let binary: string;
  try {
    binary = atob(padded);
  } catch {
    return null;
  }

  if (binary.length === 0 || binary.length > maxDecodedBytes) return null;

  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }

  const text = new TextDecoder("utf-8", { fatal: false }).decode(bytes);
  return text.length > 0 ? text : null;
}

/**
 * Parse the body passed by xterm.js to `term.parser.registerOscHandler(52, (data) => ...)`.
 *
 * Returns `null` for:
 * - clipboard read queries (`?`) — never leak local clipboard to remote processes
 * - empty clears (`c;`)
 * - invalid / oversized base64 payloads
 */
export function parseOsc52Payload(
  data: string,
  maxDecodedBytes = MAX_OSC52_DECODED_BYTES,
): ParsedOsc52 | null {
  const semi = data.indexOf(";");
  if (semi < 0) return null;

  const selection = data.slice(0, semi).trim();
  const rawPayload = data.slice(semi + 1);

  // Clipboard read query (`\x1b]52;c;?\x07`) — ignore for security.
  if (rawPayload.trim() === "?") return null;

  // Strip ASCII whitespace/line wraps that some base64 encoders insert.
  const cleanB64 = rawPayload.replace(/[\r\n\t ]+/g, "");
  if (!cleanB64) return null;

  const text = decodeBase64ToUtf8(cleanB64, maxDecodedBytes);
  if (text === null) return null;

  // Always write to the system clipboard and mirror into Linux primary selection
  // (matching GPUI's TermSideEffect::ClipboardStore behavior).
  return {
    selection: selection || "c",
    text,
    alsoSelection: true,
  };
}
