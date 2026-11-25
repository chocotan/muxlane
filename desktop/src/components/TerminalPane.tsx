import React, { useEffect, useRef, useState } from "react";
import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import { Maximize2, Trash2 } from "lucide-react";
import { Methods, type TermSubscribeResult } from "../shared/protocol";
import { encodeStringToBase64, type TermStreamChunk } from "../state/useMuxlaneStore";
import type { AppTheme } from "../state/useTheme";

interface TerminalPaneProps {
  agentId: string;
  connected: boolean;
  theme: AppTheme;
  registerTermListener: (agentId: string, fn: (chunk: TermStreamChunk) => void) => () => void;
}

const DARK_XTERM_THEME: ITheme = {
  background: "#0e0e10",
  foreground: "#e4e4e7",
  cursor: "#e4e4e7",
  cursorAccent: "#0e0e10",
  selectionBackground: "rgba(255, 255, 255, 0.18)",
  black: "#18181b",
  red: "#f87171",
  green: "#74c991",
  yellow: "#eab308",
  blue: "#60a5fa",
  magenta: "#c084fc",
  cyan: "#22d3ee",
  white: "#d4d4d8",
  brightBlack: "#52525b",
  brightRed: "#fca5a5",
  brightGreen: "#86efac",
  brightYellow: "#fde047",
  brightBlue: "#93c5fd",
  brightMagenta: "#d8b4fe",
  brightCyan: "#67e8f9",
  brightWhite: "#fafafa",
};

const LIGHT_XTERM_THEME: ITheme = {
  background: "#fafafa",
  foreground: "#18181b",
  cursor: "#18181b",
  cursorAccent: "#fafafa",
  selectionBackground: "rgba(0, 0, 0, 0.14)",
  black: "#18181b",
  red: "#dc2626",
  green: "#15803d",
  yellow: "#b45309",
  blue: "#1d4ed8",
  magenta: "#7e22ce",
  cyan: "#0e7490",
  white: "#71717a",
  brightBlack: "#52525b",
  brightRed: "#ef4444",
  brightGreen: "#16a34a",
  brightYellow: "#ca8a04",
  brightBlue: "#2563eb",
  brightMagenta: "#9333ea",
  brightCyan: "#0891b2",
  brightWhite: "#09090b",
};

export const TerminalPane: React.FC<TerminalPaneProps> = ({
  agentId,
  connected,
  theme,
  registerTermListener,
}) => {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const themeRef = useRef<AppTheme>(theme);
  const [sizeText, setSizeText] = useState("120×32");
  const [exited, setExited] = useState(false);

  themeRef.current = theme;

  // Live-update xterm.js color palette without resetting active session
  useEffect(() => {
    const term = termRef.current;
    if (term) {
      term.options.theme = theme === "light" ? LIGHT_XTERM_THEME : DARK_XTERM_THEME;
    }
  }, [theme]);

  useEffect(() => {
    const container = hostRef.current;
    if (!container) return;

    const term = new Terminal({
      cursorBlink: true,
      cursorStyle: "block",
      fontFamily:
        '"SF Mono", "JetBrains Mono", "Fira Code", "Cascadia Code", ui-monospace, SFMono-Regular, Menlo, monospace',
      fontSize: 13,
      lineHeight: 1.28,
      scrollback: 10_000,
      allowProposedApi: true,
      theme: themeRef.current === "light" ? LIGHT_XTERM_THEME : DARK_XTERM_THEME,
    });

    const fit = new FitAddon();
    const links = new WebLinksAddon((_ev, uri) => {
      void window.muxlane.openExternal(uri);
    });
    term.loadAddon(fit);
    term.loadAddon(links);
    term.open(container);

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
        setSizeText(`${cols}×${rows}`);
        void window.muxlane
          .rpc(Methods.TERM_RESIZE, { agent: agentId, cols, rows })
          .catch(() => {});
      } catch {
        // ignore layout race before visible
      }
    };

    const dataDisposable = term.onData((data) => {
      void window.muxlane
        .rpc(Methods.TERM_INPUT, {
          agent: agentId,
          data_b64: encodeStringToBase64(data),
        })
        .catch(() => {});
    });

    const resizeObserver = new ResizeObserver(() => {
      requestAnimationFrame(fitAndNotify);
    });
    resizeObserver.observe(container);

    let subId: string | null = null;
    let disposed = false;
    setExited(false);

    const unregister = registerTermListener(agentId, (chunk) => {
      if (disposed) return;
      if (chunk.kind === "reset") {
        term.reset();
        if (chunk.bytes.length > 0) term.write(chunk.bytes);
      } else if (chunk.kind === "append") {
        if (chunk.bytes.length > 0) term.write(chunk.bytes);
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
            void window.muxlane.rpc(Methods.TERM_UNSUBSCRIBE, { sub_id: res.sub_id }).catch(() => {});
            return;
          }
          subId = res.sub_id;
          if (res.replay_b64) {
            const bin = atob(res.replay_b64);
            const bytes = new Uint8Array(bin.length);
            for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
            term.reset();
            term.write(bytes);
          }
          requestAnimationFrame(fitAndNotify);
        })
        .catch((err) => {
          if (!disposed) {
            term.writeln(`\x1b[31m终端连接失败: ${(err as Error).message}\x1b[0m`);
          }
        });
    }

    return () => {
      disposed = true;
      unregister();
      resizeObserver.disconnect();
      dataDisposable.dispose();
      if (subId) {
        void window.muxlane.rpc(Methods.TERM_UNSUBSCRIBE, { sub_id: subId }).catch(() => {});
      }
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [agentId, connected, registerTermListener]);

  return (
    <div className="flex h-full min-h-0 flex-col bg-terminal-surface">
      <div className="flex h-8 shrink-0 items-center justify-between border-b border-line-subtle bg-sidebar px-3 font-mono text-[10.5px] text-ink-muted select-none">
        <div className="flex items-center gap-2">
          <span className="uppercase tracking-wider text-ink-2">Terminal</span>
          <span className="text-ink-faint">·</span>
          <span className="tabular-nums">{sizeText}</span>
          {exited && (
            <>
              <span className="text-ink-faint">·</span>
              <span className="text-working">exited</span>
            </>
          )}
        </div>
        <div className="flex items-center gap-1">
          <button
            type="button"
            title="适配终端尺寸"
            onClick={() => {
              fitRef.current?.fit();
              const t = termRef.current;
              if (t) {
                setSizeText(`${t.cols}×${t.rows}`);
                void window.muxlane
                  .rpc(Methods.TERM_RESIZE, { agent: agentId, cols: t.cols, rows: t.rows })
                  .catch(() => {});
              }
            }}
            className="flex h-6 items-center gap-1 px-1.5 text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
          >
            <Maximize2 className="h-3 w-3" />
            <span>适配</span>
          </button>
          <button
            type="button"
            title="清屏"
            onClick={() => termRef.current?.clear()}
            className="flex h-6 items-center gap-1 px-1.5 text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
          >
            <Trash2 className="h-3 w-3" />
            <span>清屏</span>
          </button>
        </div>
      </div>
      <div ref={hostRef} className="min-h-0 flex-1 overflow-hidden bg-terminal-surface" />
    </div>
  );
};
