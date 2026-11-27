import React, { useEffect, useRef, useState } from "react";
import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import { Methods, type TermSubscribeResult } from "../shared/protocol";
import {
  decodeKittyImageBitmap,
  KittyGraphicsScanner,
  scanKittyPlaceholderSlices,
} from "../shared/kittyGraphics";
import { parseOsc52Payload } from "../shared/osc52";
import { resolveCustomTerminalSequence } from "../shared/terminalKeys";
import { encodeStringToBase64, type TermStreamChunk } from "../state/useMuxlaneStore";
import type { AppTheme } from "../state/useTheme";

async function writeSystemClipboard(text: string, alsoSelection = true): Promise<void> {
  if (!text) return;
  try {
    if (window.muxlane?.writeClipboardText) {
      await window.muxlane.writeClipboardText(text, { alsoSelection });
      return;
    }
  } catch {
    // fall through to browser clipboard API
  }
  await navigator.clipboard?.writeText(text).catch(() => {});
}

async function readSystemClipboard(): Promise<string> {
  try {
    if (window.muxlane?.readClipboardText) {
      return await window.muxlane.readClipboardText();
    }
  } catch {
    // fall through to browser clipboard API
  }
  return (await navigator.clipboard?.readText().catch(() => "")) ?? "";
}

interface TerminalPaneProps {
  agentId: string;
  connected: boolean;
  theme: AppTheme;
  fontSize?: number;
  fontFamily?: string;
  visible?: boolean;
  registerTermListener: (agentId: string, fn: (chunk: TermStreamChunk) => void) => () => void;
}

const LIGHT_ANSI = {
  selectionBackground: "rgba(0, 0, 0, 0.14)",
  black: "#242831",
  red: "#d13e50",
  green: "#2f855a",
  yellow: "#b7791f",
  blue: "#2563eb",
  magenta: "#7e22ce",
  cyan: "#0e7490",
  white: "#71717a",
  brightBlack: "#52525b",
  brightRed: "#e11d48",
  brightGreen: "#16a34a",
  brightYellow: "#ca8a04",
  brightBlue: "#3b82f6",
  brightMagenta: "#9333ea",
  brightCyan: "#0891b2",
  brightWhite: "#09090b",
};

const DARK_ANSI = {
  selectionBackground: "rgba(255, 255, 255, 0.18)",
  black: "#21252b",
  red: "#e06c75",
  green: "#98c379",
  yellow: "#e5c07b",
  blue: "#61afef",
  magenta: "#c678dd",
  cyan: "#56b6c2",
  white: "#d7dae0",
  brightBlack: "#5c6370",
  brightRed: "#f28b92",
  brightGreen: "#b5e095",
  brightYellow: "#f0d399",
  brightBlue: "#82c4f8",
  brightMagenta: "#dc9bf0",
  brightCyan: "#7ad4df",
  brightWhite: "#ffffff",
};

const XTERM_THEMES: Record<AppTheme, ITheme> = {
  light: { ...LIGHT_ANSI, background: "#faf9f6", foreground: "#242831", cursor: "#3d6cd8", cursorAccent: "#faf9f6" },
  moonstone: { ...LIGHT_ANSI, background: "#faf9fc", foreground: "#292633", cursor: "#6650a4", cursorAccent: "#faf9fc" },
  paper: { ...LIGHT_ANSI, background: "#f4ecdf", foreground: "#3d3027", cursor: "#b35f2a", cursorAccent: "#f4ecdf" },
  ash: { ...LIGHT_ANSI, background: "#f4f4f2", foreground: "#1a1a18", cursor: "#3a3a36", cursorAccent: "#f4f4f2" },
  sky: { ...LIGHT_ANSI, background: "#f2f7ff", foreground: "#1e293b", cursor: "#2563eb", cursorAccent: "#f2f7ff" },
  jade: { ...LIGHT_ANSI, background: "#eff8f2", foreground: "#1f3529", cursor: "#059669", cursorAccent: "#eff8f2" },
  sakura: { ...LIGHT_ANSI, background: "#fff2f5", foreground: "#452530", cursor: "#e11d48", cursorAccent: "#fff2f5" },
  dark: { ...DARK_ANSI, background: "#181a1f", foreground: "#d7dae0", cursor: "#528bff", cursorAccent: "#181a1f" },
  onedark: { ...DARK_ANSI, background: "#282c34", foreground: "#abb2bf", cursor: "#61afef", cursorAccent: "#282c34" },
  synthwave: {
    ...DARK_ANSI,
    background: "#1b1029",
    foreground: "#f9eaff",
    cursor: "#e879f9",
    cursorAccent: "#1b1029",
    green: "#7ee2b8",
    yellow: "#f5ca7a",
    red: "#fb7185",
    magenta: "#e879f9",
  },
};

const DEFAULT_FONT_FAMILY =
  '"JetBrains Mono", "SF Mono", "Fira Code", ui-monospace, SFMono-Regular, Menlo, monospace';

export const TerminalPane: React.FC<TerminalPaneProps> = ({
  agentId,
  connected,
  theme,
  fontSize = 13,
  fontFamily = DEFAULT_FONT_FAMILY,
  visible = true,
  registerTermListener,
}) => {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const overlayCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const themeRef = useRef<AppTheme>(theme);
  const fontSizeRef = useRef(fontSize);
  const fontFamilyRef = useRef(fontFamily);
  const [exited, setExited] = useState(false);

  themeRef.current = theme;
  fontSizeRef.current = fontSize;
  fontFamilyRef.current = fontFamily;

  // Orca-style focus & fit handoff when TerminalPane is uncovered from Chat view
  useEffect(() => {
    if (!visible) return;
    const term = termRef.current;
    const fit = fitRef.current;
    if (!term || !fit) return;
    const raf = requestAnimationFrame(() => {
      try {
        fit.fit();
        const cols = Math.max(20, term.cols);
        const rows = Math.max(6, term.rows);
        void window.muxlane
          .rpc(Methods.TERM_RESIZE, { agent: agentId, cols, rows })
          .catch(() => {});
        term.focus();
      } catch {
        // ignore layout race
      }
    });
    return () => cancelAnimationFrame(raf);
  }, [agentId, visible]);

  // Live-update xterm.js color palette without resetting active session
  useEffect(() => {
    const term = termRef.current;
    if (term) {
      term.options.theme = XTERM_THEMES[theme] ?? XTERM_THEMES.dark;
    }
  }, [theme]);

  // Live-update terminal font size & family from Settings and refit cleanly
  useEffect(() => {
    const term = termRef.current;
    const fit = fitRef.current;
    if (!term || !fit) return;
    term.options.fontSize = fontSize;
    term.options.fontFamily = fontFamily;
    requestAnimationFrame(() => {
      try {
        fit.fit();
        const cols = Math.max(20, term.cols);
        const rows = Math.max(6, term.rows);
        void window.muxlane
          .rpc(Methods.TERM_RESIZE, { agent: agentId, cols, rows })
          .catch(() => {});
      } catch {
        // ignore
      }
    });
  }, [agentId, fontFamily, fontSize]);

  useEffect(() => {
    const container = hostRef.current;
    if (!container) return;

    const term = new Terminal({
      cursorBlink: true,
      cursorStyle: "block",
      fontFamily: fontFamilyRef.current,
      fontSize: fontSizeRef.current,
      lineHeight: 1.08,
      scrollback: 10_000,
      allowProposedApi: true,
      theme: XTERM_THEMES[themeRef.current] ?? XTERM_THEMES.dark,
    });

    const fit = new FitAddon();
    const links = new WebLinksAddon((_ev, uri) => {
      void window.muxlane.openExternal(uri);
    });
    term.loadAddon(fit);
    term.loadAddon(links);
    term.open(container);
    if (visible) {
      requestAnimationFrame(() => term.focus());
    }

    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        webgl.dispose();
      });
      term.loadAddon(webgl);
    } catch {
      // Canvas/DOM renderer remains active
    }

    termRef.current = term;
    fitRef.current = fit;

    const fitAndNotify = () => {
      try {
        fit.fit();
        const cols = Math.max(20, term.cols);
        const rows = Math.max(6, term.rows);
        void window.muxlane
          .rpc(Methods.TERM_RESIZE, { agent: agentId, cols, rows })
          .catch(() => {});
      } catch {
        // ignore layout race before visible
      }
    };

    const sendInput = (data: string) => {
      void window.muxlane
        .rpc(Methods.TERM_INPUT, {
          agent: agentId,
          data_b64: encodeStringToBase64(data),
        })
        .catch(() => {});
    };

    const kittyScanner = new KittyGraphicsScanner();
    const kittyBitmaps = new Map<number, ImageBitmap>();
    const directPlacements: Array<{
      id: number;
      absRow: number;
      col: number;
      cols?: number;
      rows?: number;
    }> = [];
    let overlayRaf: number | null = null;
    let disposed = false;

    const clearKittyBitmaps = () => {
      for (const bmp of kittyBitmaps.values()) {
        try {
          bmp.close();
        } catch {
          // ignore
        }
      }
      kittyBitmaps.clear();
      directPlacements.length = 0;
    };

    const drawKittyOverlay = () => {
      overlayRaf = null;
      if (disposed) return;
      const canvas = overlayCanvasRef.current;
      if (!canvas) return;
      const screenEl = container.querySelector(".xterm-screen") as HTMLElement | null;
      if (!screenEl) return;

      const hostRect = container.getBoundingClientRect();
      const screenRect = screenEl.getBoundingClientRect();
      const width = Math.max(0, Math.round(screenRect.width));
      const height = Math.max(0, Math.round(screenRect.height));
      if (width === 0 || height === 0 || term.cols <= 0 || term.rows <= 0) return;

      const left = screenRect.left - hostRect.left;
      const top = screenRect.top - hostRect.top;
      canvas.style.left = `${left}px`;
      canvas.style.top = `${top}px`;
      canvas.style.width = `${width}px`;
      canvas.style.height = `${height}px`;

      const dpr = window.devicePixelRatio || 1;
      const pixelW = Math.round(width * dpr);
      const pixelH = Math.round(height * dpr);
      if (canvas.width !== pixelW || canvas.height !== pixelH) {
        canvas.width = pixelW;
        canvas.height = pixelH;
      }

      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, width, height);

      if (kittyBitmaps.size === 0) return;

      const cellW = width / term.cols;
      const cellH = height / term.rows;
      ctx.imageSmoothingEnabled = true;
      ctx.imageSmoothingQuality = "high";

      // 1. Render Unicode Placeholder (U+10EEEE) placements (used by pi, pie, tmux passthrough)
      const slices = scanKittyPlaceholderSlices(term, kittyScanner);
      const renderedIds = new Set<number>();
      for (const slice of slices) {
        const bmp = kittyBitmaps.get(slice.imageId);
        if (!bmp) continue;
        renderedIds.add(slice.imageId);

        const sx = (slice.srcColStart / slice.totalCols) * bmp.width;
        const sy = (slice.srcRowStart / slice.totalRows) * bmp.height;
        const sw = (slice.spanCols / slice.totalCols) * bmp.width;
        const sh = (slice.spanRows / slice.totalRows) * bmp.height;
        const dx = slice.screenCol * cellW;
        const dy = slice.screenRow * cellH;
        const dw = slice.spanCols * cellW;
        const dh = slice.spanRows * cellH;
        ctx.drawImage(bmp, sx, sy, sw, sh, dx, dy, dw, dh);
      }

      // 2. Render direct `a=T` placements (non-placeholder Kitty tools)
      const viewportY = term.buffer.active.viewportY;
      for (const dp of directPlacements) {
        if (renderedIds.has(dp.id)) continue;
        const bmp = kittyBitmaps.get(dp.id);
        if (!bmp) continue;
        const drawCols = dp.cols ?? Math.max(1, Math.min(term.cols - dp.col, Math.ceil(bmp.width / cellW)));
        const drawRows = dp.rows ?? Math.max(1, Math.ceil(bmp.height / cellH));
        const screenRow = dp.absRow - viewportY;
        if (screenRow + drawRows <= 0 || screenRow >= term.rows) continue;
        ctx.drawImage(bmp, dp.col * cellW, screenRow * cellH, drawCols * cellW, drawRows * cellH);
      }
    };

    const scheduleOverlayDraw = () => {
      if (disposed || overlayRaf !== null) return;
      overlayRaf = requestAnimationFrame(drawKittyOverlay);
    };

    let isParsingReplay = false;
    const writeBytes = (bytes: Uint8Array, isReplay: boolean) => {
      if (bytes.length === 0) return;

      const kittyRes = kittyScanner.process(bytes);
      if (kittyRes.cleared) {
        clearKittyBitmaps();
        scheduleOverlayDraw();
      }
      if (!isReplay) {
        for (const reply of kittyRes.replies) {
          sendInput(reply);
        }
      }
      for (const dp of kittyRes.directPlacements) {
        directPlacements.push({
          id: dp.id,
          absRow: term.buffer.active.baseY + term.buffer.active.cursorY,
          col: term.buffer.active.cursorX,
          cols: dp.cols,
          rows: dp.rows,
        });
        if (directPlacements.length > 64) directPlacements.shift();
      }
      for (const id of kittyRes.updatedImageIds) {
        const stored = kittyScanner.getImage(id);
        if (!stored) continue;
        void decodeKittyImageBitmap(stored)
          .then((bmp) => {
            if (!bmp) return;
            if (disposed) {
              bmp.close();
              return;
            }
            const prev = kittyBitmaps.get(id);
            if (prev) prev.close();
            kittyBitmaps.set(id, bmp);
            scheduleOverlayDraw();
          })
          .catch(() => {
            // ignore corrupt/unsupported image payloads
          });
      }

      if (kittyRes.cleanBytes.length === 0) return;
      if (isReplay) {
        term.write("", () => {
          isParsingReplay = true;
        });
        term.write(kittyRes.cleanBytes, () => {
          isParsingReplay = false;
          scheduleOverlayDraw();
        });
      } else {
        term.write(kittyRes.cleanBytes, () => {
          if (kittyScanner.hasImages()) scheduleOverlayDraw();
        });
      }
    };

    // Handle OSC 52 clipboard writes from remote tmux / TUI apps (vim, neovim, pi, claude, etc.)
    const osc52Disposable = term.parser.registerOscHandler(52, (data) => {
      if (isParsingReplay) return true;
      const parsed = parseOsc52Payload(data);
      if (parsed) {
        void writeSystemClipboard(parsed.text, parsed.alsoSelection);
      }
      return true;
    });

    // Fix @xterm/xterm Linux key encoding bugs (Alt+Arrow rewritten to Ctrl+Arrow, Shift+Enter -> \r, etc.)
    term.attachCustomKeyEventHandler((e) => {
      if (e.isComposing || e.keyCode === 229) return true;

      const lower = e.key.toLowerCase();
      const isCtrlShift = e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey;
      const isCtrlOnly = e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey;
      const isMetaOnly = e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey;

      // Copy: Ctrl+Shift+C, Cmd+C, or Ctrl+C when terminal has an active selection
      if (lower === "c" && (isCtrlShift || isMetaOnly || (isCtrlOnly && term.hasSelection()))) {
        if (e.type === "keydown") {
          e.preventDefault();
          e.stopPropagation();
          if (term.hasSelection()) {
            void writeSystemClipboard(term.getSelection(), true);
            if (isCtrlOnly) {
              term.clearSelection();
            }
          }
        }
        return false;
      }

      // Paste: Ctrl+Shift+V or Cmd+V
      if (lower === "v" && (isCtrlShift || isMetaOnly)) {
        if (e.type === "keydown") {
          e.preventDefault();
          e.stopPropagation();
          void readSystemClipboard().then((text) => {
            if (text) term.paste(text);
          });
        }
        return false;
      }

      const customSeq = resolveCustomTerminalSequence(e);
      if (customSeq !== null) {
        if (e.type === "keydown") {
          e.preventDefault();
          e.stopPropagation();
          sendInput(customSeq);
        }
        return false;
      }

      return true;
    });

    const dataDisposable = term.onData(sendInput);
    const renderDisposable = term.onRender(() => {
      if (kittyBitmaps.size > 0) scheduleOverlayDraw();
    });
    const scrollDisposable = term.onScroll(() => {
      if (kittyBitmaps.size > 0) scheduleOverlayDraw();
    });

    const resizeObserver = new ResizeObserver(() => {
      requestAnimationFrame(() => {
        fitAndNotify();
        scheduleOverlayDraw();
      });
    });
    resizeObserver.observe(container);

    let subId: string | null = null;
    setExited(false);

    const unregister = registerTermListener(agentId, (chunk) => {
      if (disposed) return;
      if (chunk.kind === "reset") {
        term.reset();
        kittyScanner.reset();
        clearKittyBitmaps();
        scheduleOverlayDraw();
        writeBytes(chunk.bytes, chunk.isReplay !== false);
      } else if (chunk.kind === "append") {
        writeBytes(chunk.bytes, chunk.isReplay === true);
      } else if (chunk.kind === "exit") {
        setExited(true);
        term.writeln("\r\n\x1b[33m[终端会话已退出]\x1b[0m");
      }
    });

    if (connected) {
      void window.muxlane
        .rpc<TermSubscribeResult>(Methods.TERM_SUBSCRIBE, {
          agent: agentId,
          accept_replay_chunks: true,
          accept_replay_gzip: false,
          accept_data_gzip: false,
          accept_data_stream: false,
          accept_term_diff: false,
        })
        .then((res) => {
          if (disposed) {
            void window.muxlane
              .rpc(Methods.TERM_UNSUBSCRIBE, { sub_id: res.sub_id })
              .catch(() => {});
            return;
          }
          subId = res.sub_id;
          if (res.replay_b64) {
            const bin = atob(res.replay_b64);
            const bytes = new Uint8Array(bin.length);
            for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
            term.reset();
            kittyScanner.reset();
            clearKittyBitmaps();
            writeBytes(bytes, true);
          }
          requestAnimationFrame(() => {
            fitAndNotify();
            scheduleOverlayDraw();
          });
        })
        .catch((err) => {
          if (!disposed) {
            term.writeln(`\x1b[31m终端连接失败: ${(err as Error).message}\x1b[0m`);
          }
        });
    }

    return () => {
      disposed = true;
      if (overlayRaf !== null) cancelAnimationFrame(overlayRaf);
      unregister();
      resizeObserver.disconnect();
      osc52Disposable.dispose();
      renderDisposable.dispose();
      scrollDisposable.dispose();
      dataDisposable.dispose();
      clearKittyBitmaps();
      if (subId) {
        void window.muxlane.rpc(Methods.TERM_UNSUBSCRIBE, { sub_id: subId }).catch(() => {});
      }
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [agentId, connected, registerTermListener]);

  return (
    <div
      onMouseDown={() => termRef.current?.focus()}
      onKeyDown={(e) => {
        if (e.key === "PageDown" || e.key === "PageUp") {
          e.preventDefault();
        }
      }}
      className="relative flex h-full min-h-0 flex-col bg-terminal-surface"
    >
      <div className="relative min-h-0 flex-1 overflow-hidden px-1 py-0.5">
        <div ref={hostRef} className="h-full w-full overflow-hidden" />
        <canvas
          ref={overlayCanvasRef}
          className="pointer-events-none absolute top-0 left-0 z-10"
        />
      </div>
      {exited && (
        <div className="pointer-events-none absolute right-3 bottom-3 border border-line bg-elevated px-2 py-0.5 font-mono text-[11px] text-working select-none">
          终端已退出
        </div>
      )}
    </div>
  );
};
