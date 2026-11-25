import React, { useEffect, useState } from "react";
import { FolderPlus, Plus, RefreshCw, X } from "lucide-react";
import { useMuxlaneStore } from "../state/useMuxlaneStore";
import { useTheme } from "../state/useTheme";
import { AppRail } from "./AppRail";
import { BotsPanel } from "./BotsPanel";
import { ChatPane } from "./ChatPane";
import { CommandPalette } from "./CommandPalette";
import { NewSessionModal } from "./NewSessionModal";
import { Sidebar } from "./Sidebar";
import { SplitWorkspace } from "./SplitWorkspace";
import { TerminalPane } from "./TerminalPane";
import { WorkspaceHeader } from "./WorkspaceHeader";

export const App: React.FC = () => {
  const store = useMuxlaneStore();
  const { theme, toggleTheme } = useTheme();
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [sidebarWidth, setSidebarWidth] = useState(256);
  const [newSessionProjectId, setNewSessionProjectId] = useState<string | null>(null);
  const [newSessionOpen, setNewSessionOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);

  const projects = store.snapshot?.projects ?? [];
  const agents = store.snapshot?.agents ?? [];
  const activeAgent = agents.find((a) => a.id === store.activeAgentId);
  const activeTimeline = store.activeAgentId ? store.timelines[store.activeAgentId] : undefined;
  const activeMeta =
    activeTimeline?.meta ??
    (store.activeAgentId ? store.chatSummaries[store.activeAgentId]?.meta : undefined);
  const activeViewMode = store.activeAgentId ? store.getViewMode(store.activeAgentId) : "chat";

  const handleAddProject = async () => {
    const dir = await window.muxlane.pickDirectory();
    if (dir) {
      await store.addProject(dir, false);
    }
  };

  const openNewSession = (projectId?: string | null) => {
    if (projects.length === 0) {
      void handleAddProject();
      return;
    }
    setNewSessionProjectId(projectId ?? activeAgent?.project ?? projects[0]?.id ?? null);
    setNewSessionOpen(true);
  };

  // Global keyboard shortcuts
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (!mod) return;
      const key = e.key.toLowerCase();
      if (key === "k") {
        e.preventDefault();
        setPaletteOpen((v) => !v);
      } else if (key === "b") {
        e.preventDefault();
        setSidebarOpen((v) => !v);
      } else if (key === "n") {
        e.preventDefault();
        openNewSession();
      } else if (key === "w" && store.activeAgentId) {
        e.preventDefault();
        store.closeTab(store.activeAgentId);
      } else if (key === "1" && store.activeAgentId) {
        e.preventDefault();
        void store.setAgentViewMode(store.activeAgentId, "chat");
      } else if (key === "2" && store.activeAgentId) {
        e.preventDefault();
        void store.setAgentViewMode(store.activeAgentId, "split");
      } else if (key === "3" && store.activeAgentId) {
        e.preventDefault();
        void store.setAgentViewMode(store.activeAgentId, "terminal");
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-base text-ink">
      {/* Orca-style 44px Left App Rail */}
      <AppRail
        activeSection={store.activeSection}
        approvalsCount={store.globalApprovals.length}
        botsCount={store.bots.length}
        status={store.status}
        theme={theme}
        onSelectSection={store.setActiveSection}
        onOpenPalette={() => setPaletteOpen(true)}
        onToggleTheme={toggleTheme}
        onReconnect={() => void window.muxlane.reconnect()}
      />

      {/* Resizable Workspace Navigation Sidebar (hidden on Bots page or when collapsed) */}
      {store.activeSection === "workspace" && sidebarOpen && (
        <Sidebar
          width={sidebarWidth}
          onResize={setSidebarWidth}
          machineName={store.snapshot?.machine.name ?? "local"}
          projects={projects}
          agents={agents}
          chatSummaries={store.chatSummaries}
          activeAgentId={store.activeAgentId}
          activeSection={store.activeSection}
          globalApprovals={store.globalApprovals}
          onSelectAgent={(id) => store.openAgent(id)}
          onAddProject={() => void handleAddProject()}
          onDeleteProject={(pid) => void store.deleteProject(pid)}
          onNewSessionForProject={(pid) => openNewSession(pid)}
          onNewSession={() => openNewSession()}
          onRenameAgent={(id, title) => void store.renameAgent(id, title)}
          onDeleteAgent={(id) => void store.deleteAgent(id)}
        />
      )}

      {/* Primary Workspace Column */}
      <div className="flex min-w-0 flex-1 flex-col bg-canvas">
        {store.activeSection === "bots" ? (
          <BotsPanel
            projects={projects}
            bots={store.bots}
            runs={store.botRuns}
            onSaveBot={store.saveBot}
            onDeleteBot={store.deleteBot}
            onTriggerBot={store.triggerBot}
            onOpenAgent={(id) => store.openAgent(id, "chat")}
          />
        ) : (
          <>
            <WorkspaceHeader
              sidebarOpen={sidebarOpen}
              onToggleSidebar={() => setSidebarOpen((v) => !v)}
              openTabs={store.openTabs}
              activeAgentId={store.activeAgentId}
              agents={agents}
              activeMeta={activeMeta}
              activeViewMode={activeViewMode}
              onSelectTab={(id) => store.openAgent(id)}
              onCloseTab={store.closeTab}
              onNewTab={() => openNewSession()}
              onChangeViewMode={(mode) => {
                if (store.activeAgentId) void store.setAgentViewMode(store.activeAgentId, mode);
              }}
              onTakeOverNative={() => {
                if (store.activeAgentId) void store.takeOverNativeDriver(store.activeAgentId);
              }}
            />

            {!store.status.connected && (
              <div className="flex items-center justify-between border-b border-line-subtle bg-working-soft px-4 py-1.5 text-[12px] text-working">
                <span>
                  {store.status.connecting
                    ? "正在连接 muxlane-server 守护进程（若未运行将自动启动 muxlane --headless）…"
                    : store.status.error || "尚未连接到 muxlane-server"}
                </span>
                <button
                  type="button"
                  onClick={() => void window.muxlane.reconnect()}
                  className="flex items-center gap-1.5 border border-line bg-elevated px-2.5 py-0.5 text-[11.5px] text-ink hover:bg-hover transition-colors cursor-pointer"
                >
                  <RefreshCw className="h-3 w-3" />
                  <span>重新连接</span>
                </button>
              </div>
            )}

            <main className="min-h-0 flex-1">
              {!activeAgent ? (
                /* Serene Orca + DeepSeek Harness Editorial Landing */
                <div className="flex h-full items-center justify-center bg-canvas px-6">
                  <div className="w-full max-w-[440px]">
                    <div className="flex items-center gap-3">
                      <span className="flex h-7 w-7 items-center justify-center bg-primary-btn font-mono text-[13px] font-bold text-primary-btn-fg">
                        M
                      </span>
                      <h1 className="text-[20px] font-semibold tracking-tight text-ink-strong">
                        Muxlane
                      </h1>
                    </div>
                    <p className="mt-3 text-[13.5px] leading-[1.65] text-ink-2">
                      连接本地{" "}
                      <code className="border border-line-subtle bg-code-surface px-1.5 py-0.5 font-mono text-[12px] text-ink">
                        muxlane-server
                      </code>{" "}
                      守护进程，在统一工作台内编排 Pi / Pie / OpenCode / ACP 原生结构化对话与终端 PTY 双向实时分屏。
                    </p>

                    <div className="mt-6 flex flex-wrap items-center gap-2.5">
                      {projects.length === 0 ? (
                        <button
                          type="button"
                          onClick={() => void handleAddProject()}
                          className="flex h-9 items-center gap-2 bg-primary-btn px-4 text-[13px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover transition-colors cursor-pointer"
                        >
                          <FolderPlus className="h-4 w-4" />
                          <span>添加本地项目</span>
                        </button>
                      ) : (
                        <>
                          <button
                            type="button"
                            onClick={() => openNewSession()}
                            className="flex h-9 items-center gap-2 bg-primary-btn px-4 text-[13px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover transition-colors cursor-pointer"
                          >
                            <Plus className="h-4 w-4" />
                            <span>新建 Agent / 终端会话</span>
                          </button>
                          <button
                            type="button"
                            onClick={() => void handleAddProject()}
                            className="flex h-9 items-center gap-2 border border-line bg-canvas px-3.5 text-[13px] text-ink hover:bg-hover transition-colors cursor-pointer"
                          >
                            <FolderPlus className="h-4 w-4 text-ink-muted" />
                            <span>添加项目</span>
                          </button>
                        </>
                      )}
                    </div>

                    <div className="mt-8 grid grid-cols-2 gap-x-6 gap-y-2 border-t border-line-subtle pt-4 font-mono text-[11px] text-ink-muted">
                      <div className="flex items-center justify-between">
                        <span>新建会话</span>
                        <span className="text-ink-2">Ctrl+N</span>
                      </div>
                      <div className="flex items-center justify-between">
                        <span>命令面板</span>
                        <span className="text-ink-2">Ctrl+K</span>
                      </div>
                      <div className="flex items-center justify-between">
                        <span>侧边栏</span>
                        <span className="text-ink-2">Ctrl+B</span>
                      </div>
                      <div className="flex items-center justify-between">
                        <span>对话 / 分屏 / 终端</span>
                        <span className="text-ink-2">Ctrl+1/2/3</span>
                      </div>
                    </div>
                  </div>
                </div>
              ) : activeViewMode === "terminal" ? (
                <TerminalPane
                  agentId={activeAgent.id}
                  connected={store.status.connected}
                  theme={theme}
                  registerTermListener={store.registerTermListener}
                />
              ) : activeViewMode === "split" ? (
                <SplitWorkspace
                  left={
                    <ChatPane
                      agentId={activeAgent.id}
                      agentTitle={activeAgent.title}
                      theme={theme}
                      timeline={activeTimeline}
                      onSend={(text, atts, behavior) =>
                        store.sendChatPrompt(activeAgent.id, text, atts, behavior)
                      }
                      onAbort={() => void store.abortChat(activeAgent.id)}
                      onApprove={(apId, reply) => void store.approveChat(activeAgent.id, apId, reply)}
                      onConfigure={(opts) => void store.configureChat(activeAgent.id, opts)}
                      onOpenTerminal={() => void store.setAgentViewMode(activeAgent.id, "terminal")}
                      onTakeOverNative={() => void store.takeOverNativeDriver(activeAgent.id)}
                    />
                  }
                  right={
                    <TerminalPane
                      agentId={activeAgent.id}
                      connected={store.status.connected}
                      theme={theme}
                      registerTermListener={store.registerTermListener}
                    />
                  }
                />
              ) : (
                <ChatPane
                  agentId={activeAgent.id}
                  agentTitle={activeAgent.title}
                  theme={theme}
                  timeline={activeTimeline}
                  onSend={(text, atts, behavior) =>
                    store.sendChatPrompt(activeAgent.id, text, atts, behavior)
                  }
                  onAbort={() => void store.abortChat(activeAgent.id)}
                  onApprove={(apId, reply) => void store.approveChat(activeAgent.id, apId, reply)}
                  onConfigure={(opts) => void store.configureChat(activeAgent.id, opts)}
                  onOpenTerminal={() => void store.setAgentViewMode(activeAgent.id, "terminal")}
                  onTakeOverNative={() => void store.takeOverNativeDriver(activeAgent.id)}
                />
              )}
            </main>
          </>
        )}
      </div>

      {/* Modals */}
      <NewSessionModal
        open={newSessionOpen}
        projects={projects}
        initialProjectId={newSessionProjectId}
        presetsByProject={store.presetsByProject}
        onLoadPresets={store.loadPresets}
        onClose={() => setNewSessionOpen(false)}
        onSpawn={store.spawnSession}
      />

      <CommandPalette
        open={paletteOpen}
        agents={agents}
        projects={projects}
        onClose={() => setPaletteOpen(false)}
        onSelectAgent={(id) => store.openAgent(id)}
        onNewSession={() => openNewSession()}
        onAddProject={() => void handleAddProject()}
        onOpenBots={() => store.setActiveSection("bots")}
      />

      {/* Orca-style Lifted Monochrome Toast Stack */}
      {store.toasts.length > 0 && (
        <div className="fixed bottom-4 right-4 z-50 flex flex-col gap-2">
          {store.toasts.map((toast) => (
            <div
              key={toast.id}
              className={`mux-popover-shadow flex max-w-sm items-center justify-between gap-3 border border-line-strong border-l-2 bg-elevated px-3.5 py-2 text-[12.5px] text-ink-strong ${
                toast.level === "error"
                  ? "border-l-danger"
                  : toast.level === "success"
                    ? "border-l-success"
                    : "border-l-ink-2"
              }`}
            >
              <span className="min-w-0 break-words">{toast.message}</span>
              <button
                type="button"
                onClick={() => store.dismissToast(toast.id)}
                className="shrink-0 text-ink-muted hover:text-ink-strong cursor-pointer"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};
