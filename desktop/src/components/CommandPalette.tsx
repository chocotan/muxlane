import React, { useEffect, useMemo, useState } from "react";
import { Bot, FolderPlus, Plus, Search, Terminal as TerminalIcon } from "lucide-react";
import type { AgentInstance, Project } from "../shared/protocol";
import { agentTypeLabel } from "../shared/timeline";

interface PaletteItem {
  id: string;
  label: string;
  detail: string;
  kind: "agent" | "action";
  run: () => void;
}

interface CommandPaletteProps {
  open: boolean;
  agents: AgentInstance[];
  projects: Project[];
  onClose: () => void;
  onSelectAgent: (agentId: string) => void;
  onNewSession: () => void;
  onAddProject: () => void;
  onOpenBots: () => void;
}

export const CommandPalette: React.FC<CommandPaletteProps> = ({
  open,
  agents,
  projects,
  onClose,
  onSelectAgent,
  onNewSession,
  onAddProject,
  onOpenBots,
}) => {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);

  useEffect(() => {
    if (open) {
      setQuery("");
      setSelected(0);
    }
  }, [open]);

  const items = useMemo<PaletteItem[]>(() => {
    const projectMap = new Map(projects.map((p) => [p.id, p.name]));
    const base: PaletteItem[] = [
      {
        id: "act:new",
        label: "新建会话 (Pi / OpenCode / ACP / Shell)",
        detail: "Ctrl+N",
        kind: "action",
        run: onNewSession,
      },
      {
        id: "act:project",
        label: "添加本地项目工作区",
        detail: "Folder",
        kind: "action",
        run: onAddProject,
      },
      {
        id: "act:bots",
        label: "打开自动化 Bots 面板",
        detail: "Bots",
        kind: "action",
        run: onOpenBots,
      },
      ...agents.map((a) => ({
        id: `agent:${a.id}`,
        label: a.title,
        detail: `${projectMap.get(a.project) ?? a.project} · ${agentTypeLabel(a.agent_type)} · ${a.status}`,
        kind: "agent" as const,
        run: () => onSelectAgent(a.id),
      })),
    ];

    const q = query.trim().toLowerCase();
    if (!q) return base;
    return base.filter(
      (i) => i.label.toLowerCase().includes(q) || i.detail.toLowerCase().includes(q),
    );
  }, [agents, onAddProject, onNewSession, onOpenBots, onSelectAgent, projects, query]);

  useEffect(() => {
    if (selected >= items.length) setSelected(0);
  }, [items.length, selected]);

  if (!open) return null;

  return (
    <div
      onClick={onClose}
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/55 pt-[14vh] px-4"
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="mux-popover-shadow w-full max-w-[540px] border border-line-strong bg-elevated"
      >
        <div className="flex h-11 items-center gap-2.5 border-b border-line-subtle px-3.5">
          <Search className="h-4 w-4 text-ink-muted" strokeWidth={1.75} />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                onClose();
              } else if (e.key === "ArrowDown") {
                e.preventDefault();
                setSelected((prev) => (items.length ? (prev + 1) % items.length : 0));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setSelected((prev) =>
                  items.length ? (prev - 1 + items.length) % items.length : 0,
                );
              } else if (e.key === "Enter" && items[selected]) {
                e.preventDefault();
                items[selected].run();
                onClose();
              }
            }}
            placeholder="搜索会话、项目或命令…"
            className="flex-1 bg-transparent text-[13.5px] text-ink-strong placeholder:text-ink-muted focus:outline-none select-text"
          />
          <span className="font-mono text-[10.5px] text-ink-muted">ESC</span>
        </div>

        <div className="max-h-80 overflow-y-auto py-1">
          {items.length === 0 ? (
            <div className="p-5 text-center text-[12.5px] text-ink-muted">无匹配结果</div>
          ) : (
            items.map((item, idx) => {
              const isSelected = idx === selected;
              return (
                <button
                  key={item.id}
                  type="button"
                  onMouseEnter={() => setSelected(idx)}
                  onClick={() => {
                    item.run();
                    onClose();
                  }}
                  className={`relative flex w-full items-center justify-between gap-3 px-3.5 py-2.5 text-left transition-colors cursor-pointer ${
                    isSelected
                      ? "bg-active text-ink-strong"
                      : "text-ink-2 hover:bg-hover hover:text-ink"
                  }`}
                >
                  {isSelected && (
                    <span className="absolute left-0 top-1 bottom-1 w-[2px] bg-ink-strong" />
                  )}
                  <div className="flex min-w-0 items-center gap-2.5">
                    {item.id === "act:new" ? (
                      <Plus className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
                    ) : item.id === "act:project" ? (
                      <FolderPlus className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
                    ) : item.id === "act:bots" ? (
                      <Bot className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
                    ) : (
                      <TerminalIcon className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
                    )}
                    <span className="truncate text-[13px] font-medium">{item.label}</span>
                  </div>
                  <span className="shrink-0 font-mono text-[11px] text-ink-muted">
                    {item.detail}
                  </span>
                </button>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
};
