import React from "react";
import {
  Bot,
  Command,
  Layers,
  Moon,
  RefreshCw,
  Sun,
} from "lucide-react";
import type { DaemonConnectionStatus } from "../shared/protocol";
import type { AppTheme } from "../state/useTheme";

interface AppRailProps {
  activeSection: "workspace" | "bots";
  approvalsCount: number;
  botsCount: number;
  status: DaemonConnectionStatus;
  theme: AppTheme;
  onSelectSection: (section: "workspace" | "bots") => void;
  onOpenPalette: () => void;
  onToggleTheme: () => void;
  onReconnect: () => void;
}

export const AppRail: React.FC<AppRailProps> = ({
  activeSection,
  approvalsCount,
  botsCount,
  status,
  theme,
  onSelectSection,
  onOpenPalette,
  onToggleTheme,
  onReconnect,
}) => {
  return (
    <nav
      aria-label="主导航"
      className="flex h-full w-[44px] shrink-0 flex-col items-center justify-between border-r border-line-subtle bg-rail select-none"
    >
      {/* Top: Brand Mark + Primary Views */}
      <div className="flex w-full flex-col items-center">
        <div
          title="Muxlane Desktop"
          className="flex h-9 w-full items-center justify-center border-b border-line-subtle"
        >
          <span className="flex h-5 w-5 items-center justify-center bg-primary-btn font-mono text-[11px] font-bold tracking-tighter text-primary-btn-fg">
            M
          </span>
        </div>

        <div className="mt-2 flex w-full flex-col items-center gap-1">
          <button
            type="button"
            onClick={() => onSelectSection("workspace")}
            title="工作区与会话"
            className={`relative flex h-9 w-full items-center justify-center transition-colors cursor-pointer ${
              activeSection === "workspace"
                ? "bg-hover text-ink-strong"
                : "text-ink-muted hover:bg-hover/70 hover:text-ink"
            }`}
          >
            {activeSection === "workspace" && (
              <span className="absolute left-0 top-1.5 bottom-1.5 w-[2px] bg-ink-strong" />
            )}
            <Layers className="h-[17px] w-[17px]" strokeWidth={1.75} />
            {approvalsCount > 0 && (
              <span
                title={`${approvalsCount} 项待确认审批`}
                className="absolute right-1.5 top-1.5 flex h-3.5 min-w-3.5 items-center justify-center bg-question px-0.5 font-mono text-[9px] font-bold text-white"
              >
                {approvalsCount}
              </span>
            )}
          </button>

          <button
            type="button"
            onClick={() => onSelectSection("bots")}
            title={`自动化 Bots (${botsCount})`}
            className={`relative flex h-9 w-full items-center justify-center transition-colors cursor-pointer ${
              activeSection === "bots"
                ? "bg-hover text-ink-strong"
                : "text-ink-muted hover:bg-hover/70 hover:text-ink"
            }`}
          >
            {activeSection === "bots" && (
              <span className="absolute left-0 top-1.5 bottom-1.5 w-[2px] bg-ink-strong" />
            )}
            <Bot className="h-[17px] w-[17px]" strokeWidth={1.75} />
          </button>
        </div>
      </div>

      {/* Bottom: Utilities + Theme + Daemon Health */}
      <div className="mb-2 flex w-full flex-col items-center gap-1">
        <button
          type="button"
          onClick={onOpenPalette}
          title="命令面板 (Ctrl+K)"
          className="flex h-8 w-8 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink transition-colors cursor-pointer"
        >
          <Command className="h-[15px] w-[15px]" strokeWidth={1.75} />
        </button>

        <button
          type="button"
          onClick={onToggleTheme}
          title={theme === "dark" ? "切换到浅色主题" : "切换到深色主题"}
          className="flex h-8 w-8 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink transition-colors cursor-pointer"
        >
          {theme === "dark" ? (
            <Sun className="h-[15px] w-[15px]" strokeWidth={1.75} />
          ) : (
            <Moon className="h-[15px] w-[15px]" strokeWidth={1.75} />
          )}
        </button>

        <button
          type="button"
          onClick={onReconnect}
          title={
            status.connected
              ? `muxlane-server 已连接 · ${status.socketPath}`
              : status.connecting
                ? "正在连接 muxlane-server…"
                : `${status.error || "守护进程未连接"}（点击重连）`
          }
          className="flex h-8 w-8 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink transition-colors cursor-pointer"
        >
          {!status.connected && !status.connecting ? (
            <RefreshCw className="h-3.5 w-3.5 text-danger" />
          ) : (
            <span
              className={`h-2 w-2 ${
                status.connected
                  ? "bg-success"
                  : "bg-working animate-pulse"
              }`}
            />
          )}
        </button>
      </div>
    </nav>
  );
};
