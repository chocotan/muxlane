import React from "react";
import {
  Columns2,
  MessageSquare,
  PanelLeft,
  Plus,
  Terminal as TerminalIcon,
  X,
} from "lucide-react";
import type {
  AgentInstance,
  ChatSessionMeta,
  SessionViewMode,
} from "../shared/protocol";
import { supportsChatBridge } from "../shared/protocol";
import { driverBadgeInfo } from "../shared/timeline";

interface WorkspaceHeaderProps {
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  openTabs: string[];
  activeAgentId: string | null;
  agents: AgentInstance[];
  activeMeta: ChatSessionMeta | undefined;
  activeViewMode: SessionViewMode;
  onSelectTab: (agentId: string) => void;
  onCloseTab: (agentId: string) => void;
  onNewTab: () => void;
  onChangeViewMode: (mode: SessionViewMode) => void;
  onTakeOverNative: () => void;
}

export const WorkspaceHeader: React.FC<WorkspaceHeaderProps> = ({
  sidebarOpen,
  onToggleSidebar,
  openTabs,
  activeAgentId,
  agents,
  activeMeta,
  activeViewMode,
  onSelectTab,
  onCloseTab,
  onNewTab,
  onChangeViewMode,
  onTakeOverNative,
}) => {
  const activeAgent = agents.find((a) => a.id === activeAgentId);
  const canChat = activeAgent ? supportsChatBridge(activeAgent.agent_type) : false;
  const badge = activeAgent ? driverBadgeInfo(activeMeta, activeAgent.agent_type) : null;

  const badgeDotClass =
    badge?.tone === "live"
      ? "bg-success"
      : badge?.tone === "accent"
        ? "bg-ink-strong"
        : badge?.tone === "warn"
          ? "bg-working"
          : "bg-ink-faint";

  return (
    <header className="flex h-9 shrink-0 items-stretch justify-between border-b border-line-subtle bg-tab-bar select-none">
      {/* Left: Sidebar toggle + Orca-style continuous tab strip */}
      <div className="mux-tab-strip flex min-w-0 flex-1 items-stretch overflow-x-auto">
        <button
          type="button"
          onClick={onToggleSidebar}
          title="切换侧边栏 (Ctrl+B)"
          className="flex w-9 shrink-0 items-center justify-center border-r border-line-subtle text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
        >
          <PanelLeft
            className={`h-[15px] w-[15px] ${sidebarOpen ? "text-ink" : ""}`}
            strokeWidth={1.75}
          />
        </button>

        {openTabs.map((tabId) => {
          const agent = agents.find((a) => a.id === tabId);
          if (!agent) return null;
          const active = tabId === activeAgentId;
          return (
            <div
              key={tabId}
              onClick={() => onSelectTab(tabId)}
              className={`group relative flex max-w-[210px] min-w-[110px] shrink-0 items-center justify-between gap-2 border-r border-line-subtle px-3 text-[12px] transition-colors cursor-pointer ${
                active
                  ? "bg-tab-active text-ink-strong"
                  : "bg-tab-bar text-ink-muted hover:bg-hover/60 hover:text-ink"
              }`}
            >
              {/* Seamless merge over header bottom hairline for active tab */}
              {active && (
                <span className="pointer-events-none absolute inset-x-0 -bottom-[1px] h-[1px] bg-tab-active" />
              )}
              <div className="flex min-w-0 items-center gap-2">
                <span
                  className={`h-1.5 w-1.5 shrink-0 ${
                    agent.status === "blocked"
                      ? "bg-question"
                      : agent.status === "working"
                        ? "bg-working animate-pulse"
                        : agent.status === "failed"
                          ? "bg-danger"
                          : agent.status === "done"
                            ? "bg-success"
                            : "bg-ink-faint"
                  }`}
                />
                <span className={`truncate ${active ? "font-medium" : ""}`}>
                  {agent.title}
                </span>
              </div>
              <button
                type="button"
                aria-label={`关闭 ${agent.title}`}
                onClick={(e) => {
                  e.stopPropagation();
                  onCloseTab(tabId);
                }}
                className="flex h-4 w-4 shrink-0 items-center justify-center text-ink-muted opacity-0 group-hover:opacity-100 hover:bg-active hover:text-ink-strong transition-opacity cursor-pointer"
              >
                <X className="h-3 w-3" />
              </button>
            </div>
          );
        })}

        <button
          type="button"
          onClick={onNewTab}
          title="新建会话 (Ctrl+N)"
          className="flex w-9 shrink-0 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
        >
          <Plus className="h-3.5 w-3.5" strokeWidth={1.85} />
        </button>
      </div>

      {/* Right: Quiet Driver Indicator + Minimal Monochrome Mode Switcher */}
      <div className="flex shrink-0 items-center gap-2.5 px-3">
        {badge && (
          <div className="flex items-center gap-1.5 font-mono text-[10.5px] text-ink-2">
            <span className={`h-1.5 w-1.5 shrink-0 ${badgeDotClass}`} />
            <span>{badge.label}</span>
          </div>
        )}

        {activeMeta?.driver === "terminal_pty" && activeMeta.supports_terminal_switch && (
          <button
            type="button"
            onClick={onTakeOverNative}
            title="从 PTY 桥接切换为结构化原生驱动（支持模型与思考深度切换）"
            className="px-1.5 py-0.5 text-[11px] text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
          >
            切原生
          </button>
        )}

        {activeAgent && canChat && (
          <div className="flex items-center border border-line-subtle bg-base p-[2px]">
            {(
              [
                { id: "chat", label: "对话", icon: MessageSquare, title: "对话视图 (Ctrl+1)" },
                { id: "split", label: "分屏", icon: Columns2, title: "对话 + 终端双屏 (Ctrl+2)" },
                { id: "terminal", label: "终端", icon: TerminalIcon, title: "交互式终端 (Ctrl+3)" },
              ] as const
            ).map((item) => {
              const Icon = item.icon;
              const isCurrent = activeViewMode === item.id;
              return (
                <button
                  key={item.id}
                  type="button"
                  title={item.title}
                  onClick={() => onChangeViewMode(item.id)}
                  className={`flex h-6 items-center gap-1.5 px-2 text-[11.5px] transition-colors cursor-pointer ${
                    isCurrent
                      ? "bg-active font-medium text-ink-strong"
                      : "text-ink-muted hover:text-ink"
                  }`}
                >
                  <Icon className="h-3 w-3" strokeWidth={1.8} />
                  <span>{item.label}</span>
                </button>
              );
            })}
          </div>
        )}
      </div>
    </header>
  );
};
