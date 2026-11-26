import React from "react";
import {
  Bot,
  Command,
  Layers,
  Moon,
  PanelLeftClose,
  PanelLeftOpen,
  RefreshCw,
  Server,
  Settings,
  Sun,
} from "lucide-react";
import type { DaemonConnectionStatus } from "../shared/protocol";
import { isDarkTheme, type AppTheme } from "../state/useTheme";
import { MuxlaneLogoIcon } from "./MuxlaneIcons";

interface AppRailProps {
  activeSection: "workspace" | "bots";
  sidebarOpen: boolean;
  approvalsCount: number;
  botsCount: number;
  isRemoteHost: boolean;
  remoteHostsCount: number;
  status: DaemonConnectionStatus;
  theme: AppTheme;
  onSelectSection: (section: "workspace" | "bots") => void;
  onToggleSidebar: () => void;
  onOpenRemotes: () => void;
  onOpenSettings: () => void;
  onOpenPalette: () => void;
  onToggleTheme: () => void;
  onReconnect: () => void;
}

export const AppRail: React.FC<AppRailProps> = ({
  activeSection,
  sidebarOpen,
  approvalsCount,
  botsCount,
  isRemoteHost,
  remoteHostsCount,
  status,
  theme,
  onSelectSection,
  onToggleSidebar,
  onOpenRemotes,
  onOpenSettings,
  onOpenPalette,
  onToggleTheme,
  onReconnect,
}) => {
  const dark = isDarkTheme(theme);

  return (
    <nav
      aria-label="主导航"
      className="flex h-full w-[44px] shrink-0 flex-col items-center justify-between border-r border-line-subtle bg-rail select-none"
    >
      {/* Top: Brand Mark + Primary Views */}
      <div className="flex w-full flex-col items-center">
        <div
          title="Muxlane"
          className="flex h-9 w-full items-center justify-center border-b border-line-subtle"
        >
          <MuxlaneLogoIcon className="h-[22px] w-[22px]" />
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

          <button
            type="button"
            onClick={onOpenRemotes}
            title={
              remoteHostsCount > 0
                ? `远程机器 (${remoteHostsCount}) · Ctrl+Shift+M`
                : "添加远程机器 (Ctrl+Shift+M)"
            }
            className="relative flex h-9 w-full items-center justify-center text-ink-muted hover:bg-hover/70 hover:text-ink transition-colors cursor-pointer"
          >
            <Server className="h-[16px] w-[16px]" strokeWidth={1.75} />
            {isRemoteHost && (
              <span className="absolute right-2 top-2 h-1.5 w-1.5 bg-unread" />
            )}
          </button>
        </div>
      </div>

      {/* Bottom: Palette, Theme, Settings (reconnect button only shown when disconnected/connecting) */}
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
          title={dark ? "切换到浅色主题" : "切换到深色主题"}
          className="flex h-8 w-8 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink transition-colors cursor-pointer"
        >
          {dark ? (
            <Sun className="h-[15px] w-[15px]" strokeWidth={1.75} />
          ) : (
            <Moon className="h-[15px] w-[15px]" strokeWidth={1.75} />
          )}
        </button>

        <button
          type="button"
          onClick={onOpenSettings}
          title="设置 (Ctrl+,)"
          className="flex h-8 w-8 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink transition-colors cursor-pointer"
        >
          <Settings className="h-[15px] w-[15px]" strokeWidth={1.75} />
        </button>

        <button
          type="button"
          onClick={onToggleSidebar}
          title={sidebarOpen ? "隐藏左侧边栏 (Ctrl+B)" : "展开左侧边栏 (Ctrl+B)"}
          aria-label={sidebarOpen ? "隐藏左侧边栏" : "展开左侧边栏"}
          className={`flex h-8 w-8 items-center justify-center transition-colors cursor-pointer ${
            sidebarOpen
              ? "text-ink-muted hover:bg-hover hover:text-ink"
              : "bg-active text-ink-strong hover:bg-hover"
          }`}
        >
          {sidebarOpen ? (
            <PanelLeftClose className="h-[15px] w-[15px]" strokeWidth={1.75} />
          ) : (
            <PanelLeftOpen className="h-[15px] w-[15px]" strokeWidth={1.75} />
          )}
        </button>

        {(!status.connected || status.connecting) && (
          <button
            type="button"
            onClick={onReconnect}
            title={
              status.connecting
                ? "正在连接 muxlane-server…"
                : `未连接: ${status.error || status.socketPath}，点击重连`
            }
            className="flex h-8 w-8 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink transition-colors cursor-pointer"
          >
            <RefreshCw
              className={`h-3.5 w-3.5 ${
                status.connecting ? "animate-spin text-working" : "text-danger"
              }`}
              strokeWidth={1.8}
            />
          </button>
        )}
      </div>
    </nav>
  );
};
