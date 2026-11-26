import React, { useEffect, useState } from "react";
import { FolderPlus, Plus, RefreshCw, Server, X } from "lucide-react";
import { isChatOnlySession, useMuxlaneStore } from "../state/useMuxlaneStore";
import { resolveTerminalFontCss, useSettings } from "../state/useSettings";
import { AppRail } from "./AppRail";
import { BotsPanel } from "./BotsPanel";
import { ChatPane } from "./ChatPane";
import { CommandPalette } from "./CommandPalette";
import { NewSessionModal } from "./NewSessionModal";
import { RemoteMachinesModal } from "./RemoteMachinesModal";
import { RightSidebar, type RightSidebarTab } from "./RightSidebar";
import { SettingsModal } from "./SettingsModal";
import { Sidebar } from "./Sidebar";
import { TerminalPane } from "./TerminalPane";
import { WorkspaceHeader } from "./WorkspaceHeader";

const RIGHT_SIDEBAR_STORAGE_KEY = "muxlane.desktop.rightSidebar.v1";

interface RightSidebarState {
  open: boolean;
  width: number;
  tab: RightSidebarTab;
}

function loadRightSidebarState(): RightSidebarState {
  try {
    const raw = window.localStorage.getItem(RIGHT_SIDEBAR_STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<RightSidebarState>;
      const tab: RightSidebarTab =
        parsed.tab === "changes" || parsed.tab === "info" ? parsed.tab : "changes";
      const width =
        typeof parsed.width === "number" && parsed.width >= 280 && parsed.width <= 680
          ? Math.round(parsed.width)
          : 380;
      return {
        open: parsed.open === true,
        width,
        tab,
      };
    }
  } catch {
    // ignore storage errors
  }
  return { open: false, width: 380, tab: "changes" };
}

export const App: React.FC = () => {
  const { settings, updateSettings, toggleTheme, resetSettings } = useSettings();
  const store = useMuxlaneStore({
    notificationsEnabled: settings.notificationsEnabled,
  });

  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [sidebarWidth, setSidebarWidth] = useState(252);
  const [rightSidebar, setRightSidebar] = useState<RightSidebarState>(loadRightSidebarState);
  const [newSessionProjectId, setNewSessionProjectId] = useState<string | null>(null);
  const [newSessionOpen, setNewSessionOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [remotesOpen, setRemotesOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [remoteProjectOpen, setRemoteProjectOpen] = useState(false);
  const [remoteProjectPath, setRemoteProjectPath] = useState("");
  const [remoteProjectCreate, setRemoteProjectCreate] = useState(false);

  useEffect(() => {
    try {
      window.localStorage.setItem(RIGHT_SIDEBAR_STORAGE_KEY, JSON.stringify(rightSidebar));
    } catch {
      // ignore storage errors
    }
  }, [rightSidebar]);

  const projects = store.snapshot?.projects ?? [];
  const agents = store.snapshot?.agents ?? [];
  const activeAgent = agents.find((a) => a.id === store.activeAgentId);
  const activeProject = activeAgent
    ? projects.find((p) => p.id === activeAgent.project)
    : undefined;
  const activeTimeline = store.activeAgentId ? store.timelines[store.activeAgentId] : undefined;
  const activeIsChat = isChatOnlySession(activeAgent);

  const activeHost = store.hostsState.hosts.find((h) => h.id === store.hostsState.activeHostId);
  const isRemoteHost = Boolean(activeHost && activeHost.kind !== "local");
  const remoteHostsCount = store.hostsState.hosts.filter((h) => h.kind !== "local").length;
  const displayMachineName =
    activeHost?.name || store.snapshot?.machine.name || "本机";
  const terminalFontCss = resolveTerminalFontCss(settings.terminalFontFamily);

  const handleAddProject = async () => {
    if (isRemoteHost) {
      setRemoteProjectPath("");
      setRemoteProjectCreate(false);
      setRemoteProjectOpen(true);
      return;
    }
    const dir = await window.muxlane.pickDirectory();
    if (dir) {
      await store.addProject(dir, false);
    }
  };

  const handleSubmitRemoteProject = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!remoteProjectPath.trim()) return;
    const proj = await store.addProject(remoteProjectPath.trim(), remoteProjectCreate);
    if (proj) {
      setRemoteProjectOpen(false);
      setRemoteProjectPath("");
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

  // Global keyboard shortcuts (do not hijack plain Ctrl+B/K/N/W when focused inside terminal)
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (!mod || e.altKey) return;
      const target = e.target as HTMLElement | null;
      const inTerminal = Boolean(target?.closest(".xterm"));
      const key = e.key.toLowerCase();
      if (key === ",") {
        e.preventDefault();
        setSettingsOpen((v) => !v);
      } else if (e.shiftKey && key === "m") {
        e.preventDefault();
        setRemotesOpen((v) => !v);
      } else if (key === "]") {
        e.preventDefault();
        setRightSidebar((prev) => ({ ...prev, open: !prev.open }));
      } else if (key === "k" && (!inTerminal || e.shiftKey)) {
        e.preventDefault();
        setPaletteOpen((v) => !v);
      } else if (key === "b" && (!inTerminal || e.shiftKey)) {
        e.preventDefault();
        setSidebarOpen((v) => !v);
      } else if (key === "n" && (!inTerminal || e.shiftKey)) {
        e.preventDefault();
        openNewSession();
      } else if (key === "w" && (!inTerminal || e.shiftKey) && store.activeAgentId) {
        e.preventDefault();
        store.closeTab(store.activeAgentId);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-base text-ink">
      {/* 44px Left App Rail */}
      <AppRail
        activeSection={store.activeSection}
        sidebarOpen={sidebarOpen}
        approvalsCount={store.globalApprovals.length}
        botsCount={store.bots.length}
        isRemoteHost={isRemoteHost}
        remoteHostsCount={remoteHostsCount}
        status={store.status}
        theme={settings.theme}
        onSelectSection={store.setActiveSection}
        onToggleSidebar={() => setSidebarOpen((v) => !v)}
        onOpenRemotes={() => setRemotesOpen(true)}
        onOpenSettings={() => setSettingsOpen(true)}
        onOpenPalette={() => setPaletteOpen(true)}
        onToggleTheme={toggleTheme}
        onReconnect={() => void window.muxlane.reconnect()}
      />

      {/* Resizable Left Workspace Navigation Sidebar */}
      {store.activeSection === "workspace" && sidebarOpen && (
        <Sidebar
          width={sidebarWidth}
          onResize={setSidebarWidth}
          machineName={displayMachineName}
          isRemoteHost={isRemoteHost}
          remoteHostsCount={remoteHostsCount}
          projects={projects}
          agents={agents}
          chatSummaries={store.chatSummaries}
          activeAgentId={store.activeAgentId}
          activeSection={store.activeSection}
          globalApprovals={store.globalApprovals}
          onOpenRemotes={() => setRemotesOpen(true)}
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
            onOpenAgent={(id) => store.openAgent(id)}
          />
        ) : (
          <>
            <WorkspaceHeader
              rightSidebarOpen={rightSidebar.open}
              onToggleRightSidebar={() =>
                setRightSidebar((prev) => ({ ...prev, open: !prev.open }))
              }
              openTabs={store.openTabs}
              activeAgentId={store.activeAgentId}
              agents={agents}
              onSelectTab={(id) => store.openAgent(id)}
              onCloseTab={store.closeTab}
              onNewTab={() => openNewSession()}
            />

            {store.status.connected && store.outdatedDaemon && (
              <div className="flex items-center justify-between gap-3 border-b border-line-subtle bg-question-soft px-4 py-1.5 text-[12px] text-question">
                <span>
                  当前连接的 muxlane 守护进程为旧版本，请退出旧版 muxlane 后重启守护进程（所有运行中的 tmux 会话会自动保留）。
                </span>
                <button
                  type="button"
                  onClick={() => void window.muxlane.reconnect()}
                  className="flex shrink-0 items-center gap-1.5 border border-line bg-elevated px-2.5 py-0.5 text-[11.5px] text-ink hover:bg-hover transition-colors cursor-pointer"
                >
                  <RefreshCw className="h-3 w-3" />
                  <span>重新检测</span>
                </button>
              </div>
            )}

            {!store.status.connected && (
              <div className="flex items-center justify-between gap-3 border-b border-line-subtle bg-working-soft px-4 py-1.5 text-[12px] text-working">
                <span className="truncate">
                  {store.hostsState.tunnelStatus ||
                    (store.status.connecting
                      ? `正在连接 ${displayMachineName} 守护进程…`
                      : store.status.error || "尚未连接到 muxlane-server")}
                </span>
                <button
                  type="button"
                  onClick={() => void window.muxlane.reconnect()}
                  className="flex shrink-0 items-center gap-1.5 border border-line bg-elevated px-2.5 py-0.5 text-[11.5px] text-ink hover:bg-hover transition-colors cursor-pointer"
                >
                  <RefreshCw className="h-3 w-3" />
                  <span>重新连接</span>
                </button>
              </div>
            )}

            {/* Main Session Viewport — pure fixed surface per session (Chat OR Terminal) */}
            <main className="relative min-h-0 min-w-0 flex-1 overflow-hidden">
              {!activeAgent ? (
                <div className="flex h-full items-center justify-center bg-canvas px-6">
                  <div className="w-full max-w-[420px]">
                    <div className="flex items-center gap-3">
                      <span className="flex h-7 w-7 items-center justify-center bg-primary-btn font-mono text-[13px] font-bold text-primary-btn-fg">
                        M
                      </span>
                      <h1 className="text-[20px] font-semibold tracking-tight text-ink-strong">
                        Muxlane
                      </h1>
                    </div>
                    <p className="mt-3 text-[13.5px] leading-[1.55] text-ink-2">
                      选择左侧项目或新建对话 / 终端会话开始工作，也支持通过 SSH 连接远程主机。
                    </p>

                    <div className="mt-6 flex flex-wrap items-center gap-2">
                      {projects.length === 0 ? (
                        <button
                          type="button"
                          onClick={() => void handleAddProject()}
                          className="flex h-9 items-center gap-2 bg-primary-btn px-4 text-[13px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover transition-colors cursor-pointer"
                        >
                          <FolderPlus className="h-4 w-4" />
                          <span>添加项目</span>
                        </button>
                      ) : (
                        <>
                          <button
                            type="button"
                            onClick={() => openNewSession()}
                            className="flex h-9 items-center gap-2 bg-primary-btn px-4 text-[13px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover transition-colors cursor-pointer"
                          >
                            <Plus className="h-4 w-4" />
                            <span>新建会话</span>
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
                      <button
                        type="button"
                        onClick={() => setRemotesOpen(true)}
                        className="flex h-9 items-center gap-2 border border-line bg-canvas px-3.5 text-[13px] text-ink hover:bg-hover transition-colors cursor-pointer"
                      >
                        <Server className="h-4 w-4 text-ink-muted" />
                        <span>远程机器</span>
                      </button>
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
                        <span>切换右侧栏</span>
                        <span className="text-ink-2">Ctrl+]</span>
                      </div>
                      <div className="flex items-center justify-between">
                        <span>设置</span>
                        <span className="text-ink-2">Ctrl+,</span>
                      </div>
                    </div>
                  </div>
                </div>
              ) : activeIsChat ? (
                <ChatPane
                  key={activeAgent.id}
                  agentId={activeAgent.id}
                  agentTitle={activeAgent.title}
                  theme={settings.theme}
                  timeline={activeTimeline}
                  onSend={(text, atts, behavior) =>
                    store.sendChatPrompt(activeAgent.id, text, atts, behavior)
                  }
                  onAbort={() => void store.abortChat(activeAgent.id)}
                  onApprove={(apId, reply) =>
                    void store.approveChat(activeAgent.id, apId, reply)
                  }
                  onConfigure={(opts) => void store.configureChat(activeAgent.id, opts)}
                />
              ) : (
                <TerminalPane
                  key={activeAgent.id}
                  agentId={activeAgent.id}
                  connected={store.status.connected}
                  theme={settings.theme}
                  fontSize={settings.terminalFontSize}
                  fontFamily={terminalFontCss}
                  visible
                  registerTermListener={store.registerTermListener}
                />
              )}
            </main>
          </>
        )}
      </div>

      {/* Orca-style Collapsible & Resizable Right Sidebar */}
      {store.activeSection === "workspace" && (
        <RightSidebar
          open={rightSidebar.open}
          width={rightSidebar.width}
          tab={rightSidebar.tab}
          onChangeTab={(tab) => setRightSidebar((prev) => ({ ...prev, tab }))}
          onResize={(width) => setRightSidebar((prev) => ({ ...prev, width }))}
          onClose={() => setRightSidebar((prev) => ({ ...prev, open: false }))}
          activeAgent={activeAgent}
          activeProject={activeProject}
          isRemoteHost={isRemoteHost}
          timeline={activeTimeline}
        />
      )}

      {/* Modals */}
      <NewSessionModal
        open={newSessionOpen}
        projects={projects}
        initialProjectId={newSessionProjectId}
        defaultSurface={settings.defaultViewMode}
        presetsByProject={store.presetsByProject}
        onLoadPresets={store.loadPresets}
        onClose={() => setNewSessionOpen(false)}
        onSpawn={store.spawnSession}
      />

      <RemoteMachinesModal
        open={remotesOpen}
        hostsState={store.hostsState}
        connectionStatus={store.status}
        onClose={() => setRemotesOpen(false)}
        onSelectHost={store.selectHost}
        onAddHost={store.addHost}
        onRemoveHost={store.removeHost}
      />

      <SettingsModal
        open={settingsOpen}
        settings={settings}
        connectionStatus={store.status}
        machine={store.snapshot?.machine}
        onUpdateSettings={updateSettings}
        onResetSettings={resetSettings}
        onReconnect={() => void window.muxlane.reconnect()}
        onClose={() => setSettingsOpen(false)}
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
        onOpenRemotes={() => setRemotesOpen(true)}
        onOpenSettings={() => setSettingsOpen(true)}
      />

      {/* Remote Project Path Input Modal */}
      {remoteProjectOpen && (
        <div
          onClick={() => setRemoteProjectOpen(false)}
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4"
        >
          <form
            onClick={(e) => e.stopPropagation()}
            onSubmit={(e) => void handleSubmitRemoteProject(e)}
            className="mux-popover-shadow w-full max-w-[460px] border border-line-strong bg-elevated"
          >
            <div className="flex h-11 items-center justify-between border-b border-line-subtle px-4">
              <span className="text-[13.5px] font-semibold text-ink-strong">
                在 {displayMachineName} 添加项目
              </span>
              <button
                type="button"
                onClick={() => setRemoteProjectOpen(false)}
                className="flex h-6 w-6 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink-strong cursor-pointer"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="space-y-3 p-4">
              <div>
                <label className="mb-1.5 block text-[11.5px] text-ink-muted">
                  远端项目目录路径
                </label>
                <input
                  autoFocus
                  required
                  value={remoteProjectPath}
                  onChange={(e) => setRemoteProjectPath(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") setRemoteProjectOpen(false);
                  }}
                  placeholder="例如：~/workspace/my-repo"
                  className="h-9 w-full border border-line bg-canvas px-3 font-mono text-[12.5px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
                />
              </div>
              <label className="flex items-center gap-2 text-[12px] text-ink-2 cursor-pointer select-none">
                <input
                  type="checkbox"
                  checked={remoteProjectCreate}
                  onChange={(e) => setRemoteProjectCreate(e.target.checked)}
                />
                <span>如果目录不存在则自动创建</span>
              </label>
            </div>
            <div className="flex items-center justify-end gap-2 border-t border-line-subtle px-4 py-3">
              <button
                type="button"
                onClick={() => setRemoteProjectOpen(false)}
                className="h-8 border border-line bg-canvas px-3 text-[12px] text-ink-2 hover:bg-hover hover:text-ink-strong cursor-pointer"
              >
                取消
              </button>
              <button
                type="submit"
                disabled={!remoteProjectPath.trim()}
                className="h-8 bg-primary-btn px-3.5 text-[12px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover disabled:opacity-45 cursor-pointer"
              >
                添加项目
              </button>
            </div>
          </form>
        </div>
      )}

      {/* Toast Stack */}
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
                className="flex h-5 w-5 shrink-0 items-center justify-center text-ink-muted hover:text-ink-strong cursor-pointer"
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
