import React from "react";
import { Globe, PanelRight, PanelRightClose, Plus, X } from "lucide-react";
import type { AgentInstance } from "../shared/protocol";
import type { WebSession } from "../state/webSessions";

interface WorkspaceHeaderProps {
  rightSidebarOpen: boolean;
  onToggleRightSidebar: () => void;
  openTabs: string[];
  activeTabId: string | null;
  agents: AgentInstance[];
  webSessions: WebSession[];
  onSelectTab: (tabId: string) => void;
  onCloseTab: (tabId: string) => void;
  onNewTab: () => void;
}

export const WorkspaceHeader: React.FC<WorkspaceHeaderProps> = ({
  rightSidebarOpen,
  onToggleRightSidebar,
  openTabs,
  activeTabId,
  agents,
  webSessions,
  onSelectTab,
  onCloseTab,
  onNewTab,
}) => {
  return (
    <header className="flex h-9 shrink-0 items-stretch justify-between border-b border-line-subtle bg-tab-bar select-none">
      {/* Left: Unified tab strip (Agent / Shell / Browser / DSH tabs) */}
      <div className="mux-tab-strip flex min-w-0 flex-1 items-stretch overflow-x-auto">
        {openTabs.map((tabId) => {
          const web = webSessions.find((w) => w.id === tabId);
          if (web) {
            const active = tabId === activeTabId;
            return (
              <div
                key={tabId}
                onClick={() => onSelectTab(tabId)}
                title={web.url || web.title}
                className={`group relative flex max-w-[210px] min-w-[110px] shrink-0 items-center justify-between gap-2 border-r border-line-subtle px-3 text-[12px] transition-colors cursor-pointer ${
                  active
                    ? "bg-tab-active text-ink-strong"
                    : "bg-tab-bar text-ink-muted hover:bg-hover/60 hover:text-ink"
                }`}
              >
                {active && (
                  <span className="pointer-events-none absolute inset-x-0 -bottom-[1px] h-[1px] bg-tab-active" />
                )}
                <div className="flex min-w-0 items-center gap-1.5">
                  <Globe
                    className={`h-3 w-3 shrink-0 ${
                      web.kind === "dsh" ? "text-working" : "text-ink-muted"
                    }`}
                    strokeWidth={1.85}
                  />
                  <span className={`truncate ${active ? "font-medium" : ""}`}>{web.title}</span>
                </div>
                <button
                  type="button"
                  aria-label={`关闭 ${web.title}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    onCloseTab(tabId);
                  }}
                  className={`flex h-4 w-4 shrink-0 items-center justify-center text-ink-muted hover:bg-active hover:text-ink-strong transition-opacity cursor-pointer ${
                    active ? "opacity-55 hover:opacity-100" : "opacity-0 group-hover:opacity-100"
                  }`}
                >
                  <X className="h-3 w-3" />
                </button>
              </div>
            );
          }

          const agent = agents.find((a) => a.id === tabId);
          if (!agent) return null;
          const active = tabId === activeTabId;
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
                <span className={`truncate ${active ? "font-medium" : ""}`}>{agent.title}</span>
              </div>
              <button
                type="button"
                aria-label={`关闭 ${agent.title}`}
                onClick={(e) => {
                  e.stopPropagation();
                  onCloseTab(tabId);
                }}
                className={`flex h-4 w-4 shrink-0 items-center justify-center text-ink-muted hover:bg-active hover:text-ink-strong transition-opacity cursor-pointer ${
                  active ? "opacity-55 hover:opacity-100" : "opacity-0 group-hover:opacity-100"
                }`}
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

      {/* Right: Orca Right Sidebar Toggle */}
      <div className="flex shrink-0 items-stretch">
        <button
          type="button"
          onClick={onToggleRightSidebar}
          title={rightSidebarOpen ? "收起右侧栏 (Ctrl+])" : "展开右侧栏 (Ctrl+])"}
          aria-label={rightSidebarOpen ? "收起右侧栏" : "展开右侧栏"}
          className={`flex w-9 shrink-0 items-center justify-center border-l border-line-subtle transition-colors cursor-pointer ${
            rightSidebarOpen
              ? "bg-active text-ink-strong"
              : "text-ink-muted hover:bg-hover hover:text-ink-strong"
          }`}
        >
          {rightSidebarOpen ? (
            <PanelRightClose className="h-[15px] w-[15px]" strokeWidth={1.75} />
          ) : (
            <PanelRight className="h-[15px] w-[15px]" strokeWidth={1.75} />
          )}
        </button>
      </div>
    </header>
  );
};
