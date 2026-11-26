import React, { useEffect, useMemo, useState } from "react";
import { Globe, MessageSquare, Server, Terminal as TerminalIcon, X } from "lucide-react";
import {
  presetChatDriver,
  type AgentPreset,
  type Project,
  type SessionViewMode,
} from "../shared/protocol";
import { orderPresetsPiFirst } from "../shared/timeline";
import {
  MACHINE_DEFAULT_WORKSPACE_ID,
  normalizeWebUrl,
  type WebSessionKind,
} from "../state/webSessions";

const PRESET_DSH_WEB = "__muxlane_web_dsh__";
const PRESET_CUSTOM_WEB = "__muxlane_web_url__";

interface NewSessionModalProps {
  open: boolean;
  projects: Project[];
  initialProjectId: string | null;
  defaultSurface: SessionViewMode;
  machineName: string;
  isRemoteHost: boolean;
  presetsByProject: Record<string, AgentPreset[]>;
  onLoadPresets: (projectId: string) => Promise<AgentPreset[]>;
  onClose: () => void;
  onSpawn: (opts: {
    projectId: string;
    preset: AgentPreset;
    surface: SessionViewMode;
    initialPrompt?: string;
  }) => Promise<unknown>;
  onCreateWebSession: (opts: {
    projectId: string;
    kind: WebSessionKind;
    url?: string;
    title?: string;
  }) => unknown;
}

export const NewSessionModal: React.FC<NewSessionModalProps> = ({
  open,
  projects,
  initialProjectId,
  defaultSurface,
  machineName,
  isRemoteHost,
  presetsByProject,
  onLoadPresets,
  onClose,
  onSpawn,
  onCreateWebSession,
}) => {
  const [projectId, setProjectId] = useState(
    initialProjectId || projects[0]?.id || MACHINE_DEFAULT_WORKSPACE_ID,
  );
  const [selectedPresetId, setSelectedPresetId] = useState<string>("");
  const [surface, setSurface] = useState<SessionViewMode>(
    defaultSurface === "terminal" ? "terminal" : "chat",
  );
  const [initialPrompt, setInitialPrompt] = useState("");
  const [webUrl, setWebUrl] = useState("");
  const [webTitle, setWebTitle] = useState("");
  const [formError, setFormError] = useState<string | null>(null);
  const [loadingPresets, setLoadingPresets] = useState(false);
  const [spawning, setSpawning] = useState(false);

  const isMachineDefaultWorkspace = projectId === MACHINE_DEFAULT_WORKSPACE_ID;

  useEffect(() => {
    if (!open) return;
    const pid = initialProjectId || projects[0]?.id || MACHINE_DEFAULT_WORKSPACE_ID;
    setProjectId(pid);
    setSelectedPresetId(pid === MACHINE_DEFAULT_WORKSPACE_ID ? PRESET_DSH_WEB : "");
    setSurface(defaultSurface === "terminal" ? "terminal" : "chat");
    setInitialPrompt("");
    setWebUrl("");
    setWebTitle("");
    setFormError(null);

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [defaultSurface, initialProjectId, onClose, open, projects]);

  useEffect(() => {
    if (!open || !projectId || projectId === MACHINE_DEFAULT_WORKSPACE_ID) return;
    setLoadingPresets(true);
    void onLoadPresets(projectId).finally(() => setLoadingPresets(false));
  }, [onLoadPresets, open, projectId]);

  const presets = isMachineDefaultWorkspace ? [] : (presetsByProject[projectId] ?? []);
  const { agents, shell } = useMemo(() => orderPresetsPiFirst(presets), [presets]);
  const allOrdered = useMemo(() => (shell ? [...agents, shell] : agents), [agents, shell]);

  useEffect(() => {
    if (!open) return;
    if (isMachineDefaultWorkspace) {
      if (selectedPresetId !== PRESET_DSH_WEB && selectedPresetId !== PRESET_CUSTOM_WEB) {
        setSelectedPresetId(PRESET_DSH_WEB);
      }
      return;
    }
    // Project workspace: DSH is machine-level only; keep custom web or default to first CLI preset
    if (selectedPresetId === PRESET_DSH_WEB) {
      setSelectedPresetId(allOrdered[0]?.id ?? PRESET_CUSTOM_WEB);
      return;
    }
    if (
      selectedPresetId !== PRESET_CUSTOM_WEB &&
      allOrdered.length > 0 &&
      !allOrdered.some((p) => p.id === selectedPresetId)
    ) {
      setSelectedPresetId(allOrdered[0].id);
    }
  }, [allOrdered, isMachineDefaultWorkspace, open, selectedPresetId]);

  if (!open) return null;

  const isDshWeb = isMachineDefaultWorkspace && selectedPresetId === PRESET_DSH_WEB;
  const isCustomWeb = selectedPresetId === PRESET_CUSTOM_WEB;
  const isWebSelection = isDshWeb || isCustomWeb;
  const activePreset = isWebSelection
    ? null
    : (allOrdered.find((p) => p.id === selectedPresetId) ?? allOrdered[0] ?? null);
  const isShell = activePreset?.agent_type === "shell";
  const supportsNativeChat = activePreset
    ? presetChatDriver(activePreset.agent_type) !== null
    : false;

  const handleLaunch = async () => {
    if (!projectId || spawning) return;
    setFormError(null);

    if (isWebSelection) {
      try {
        const trimmedUrl = webUrl.trim();
        if (isCustomWeb) {
          if (!trimmedUrl) {
            setFormError("请输入要访问的网页地址，例如 https://github.com 或 http://localhost:3000");
            return;
          }
          normalizeWebUrl(trimmedUrl);
        } else if (trimmedUrl) {
          normalizeWebUrl(trimmedUrl);
        }
        onCreateWebSession({
          projectId,
          kind: isDshWeb ? "dsh" : "web",
          url: trimmedUrl || undefined,
          title: webTitle.trim() || undefined,
        });
        onClose();
      } catch (err) {
        setFormError((err as Error).message || "网页地址格式无效");
      }
      return;
    }

    if (!activePreset) return;
    setSpawning(true);
    try {
      await onSpawn({
        projectId,
        preset: activePreset,
        surface:
          isShell || !supportsNativeChat
            ? "terminal"
            : surface === "terminal"
              ? "terminal"
              : "chat",
        initialPrompt: initialPrompt.trim() || undefined,
      });
      onClose();
    } finally {
      setSpawning(false);
    }
  };

  return (
    <div
      onClick={onClose}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4"
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="mux-popover-shadow w-full max-w-[540px] border border-line-strong bg-elevated"
      >
        <div className="flex h-11 items-center justify-between border-b border-line-subtle px-4">
          <span className="text-[13.5px] font-semibold text-ink-strong">新建会话</span>
          <button
            type="button"
            onClick={onClose}
            aria-label="关闭"
            className="flex h-6 w-6 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="space-y-4 p-4">
          {/* Workspace / Project select */}
          <div>
            <label className="mb-1.5 block text-[11.5px] text-ink-muted">目标工作区</label>
            <select
              value={projectId}
              onChange={(e) => {
                setFormError(null);
                setProjectId(e.target.value);
              }}
              className="h-9 w-full border border-line bg-canvas px-3 text-[13px] text-ink-strong focus:border-line-strong focus:outline-none"
            >
              <option value={MACHINE_DEFAULT_WORKSPACE_ID}>
                默认工作区（机器级 · {machineName}）
              </option>
              {projects.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name} — {p.path}
                </option>
              ))}
            </select>
          </div>

          {/* Agent / Shell presets (only for project workspaces) */}
          {!isMachineDefaultWorkspace && (
            <div>
              <label className="mb-1.5 block text-[11.5px] text-ink-muted">Agent / Shell</label>
              {loadingPresets && allOrdered.length === 0 ? (
                <div className="border border-line-subtle bg-canvas p-4 text-center text-[12.5px] text-ink-muted">
                  正在检测已安装的 CLI 工具…
                </div>
              ) : (
                <div className="grid grid-cols-2 gap-1.5">
                  {allOrdered.map((preset) => {
                    const selected = !isWebSelection && preset.id === activePreset?.id;
                    return (
                      <button
                        key={preset.id}
                        type="button"
                        onClick={() => {
                          setFormError(null);
                          setSelectedPresetId(preset.id);
                        }}
                        className={`flex items-center justify-between border px-3 py-2 text-left transition-colors cursor-pointer ${
                          selected
                            ? "border-line-strong bg-active text-ink-strong"
                            : "border-line-subtle bg-canvas text-ink-2 hover:border-line hover:bg-hover hover:text-ink"
                        }`}
                      >
                        <div className="min-w-0">
                          <div className="truncate text-[12.5px] font-medium">{preset.label}</div>
                          <div className="truncate font-mono text-[10.5px] text-ink-muted">
                            {preset.program}
                          </div>
                        </div>
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          )}

          {/* Browser / Web presets: DSH lives ONLY in machine default workspace; custom web works everywhere */}
          <div>
            <label className="mb-1.5 block text-[11.5px] text-ink-muted">浏览器 / 网页</label>
            <div className={isMachineDefaultWorkspace ? "grid grid-cols-2 gap-1.5" : "grid grid-cols-1 gap-1.5"}>
              {isMachineDefaultWorkspace && (
                <button
                  type="button"
                  onClick={() => {
                    setFormError(null);
                    setSelectedPresetId(PRESET_DSH_WEB);
                  }}
                  className={`flex items-center gap-2.5 border px-3 py-2 text-left transition-colors cursor-pointer ${
                    isDshWeb
                      ? "border-line-strong bg-active text-ink-strong"
                      : "border-line-subtle bg-canvas text-ink-2 hover:border-line hover:bg-hover hover:text-ink"
                  }`}
                >
                  <Globe className="h-4 w-4 shrink-0 text-working" strokeWidth={1.75} />
                  <div className="min-w-0">
                    <div className="truncate text-[12.5px] font-medium">DeepSeek Harness</div>
                    <div className="truncate font-mono text-[10.5px] text-ink-muted">
                      dsh web · {machineName}
                    </div>
                  </div>
                </button>
              )}

              <button
                type="button"
                onClick={() => {
                  setFormError(null);
                  setSelectedPresetId(PRESET_CUSTOM_WEB);
                }}
                className={`flex items-center gap-2.5 border px-3 py-2 text-left transition-colors cursor-pointer ${
                  isCustomWeb
                    ? "border-line-strong bg-active text-ink-strong"
                    : "border-line-subtle bg-canvas text-ink-2 hover:border-line hover:bg-hover hover:text-ink"
                }`}
              >
                <Globe className="h-4 w-4 shrink-0 text-ink-muted" strokeWidth={1.75} />
                <div className="min-w-0">
                  <div className="truncate text-[12.5px] font-medium">网页浏览器</div>
                  <div className="truncate font-mono text-[10.5px] text-ink-muted">
                    访问任意 HTTP / HTTPS 网址
                  </div>
                </div>
              </button>
            </div>
          </div>

          {/* DSH WebUI options (machine default workspace only) */}
          {isDshWeb && (
            <div className="space-y-3 border border-line-subtle bg-canvas/60 p-3">
              <div className="flex items-center gap-2 text-[11.5px] text-ink-2">
                <Server
                  className={`h-3.5 w-3.5 shrink-0 ${isRemoteHost ? "text-success" : "text-ink-muted"}`}
                />
                <span>
                  {isRemoteHost
                    ? `将检测并在远程主机「${machineName}」复用或启动 dsh web（自动经 SSH 端口转发）`
                    : `将检测并在「${machineName}」复用或启动 dsh web`}
                </span>
              </div>
              <div>
                <label className="mb-1 block text-[11px] text-ink-muted">
                  自定义 DSH 地址（可选，留空自动检测/启动）
                </label>
                <input
                  value={webUrl}
                  onChange={(e) => setWebUrl(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void handleLaunch();
                    }
                  }}
                  placeholder="例如 http://100.64.10.138:3080/?token=..."
                  className="h-8 w-full border border-line bg-canvas px-2.5 font-mono text-[12px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
                />
              </div>
            </div>
          )}

          {/* Custom Webpage URL options */}
          {isCustomWeb && (
            <div className="space-y-3 border border-line-subtle bg-canvas/60 p-3">
              <div>
                <label className="mb-1 block text-[11px] text-ink-muted">网页地址</label>
                <input
                  autoFocus
                  value={webUrl}
                  onChange={(e) => setWebUrl(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void handleLaunch();
                    }
                  }}
                  placeholder="例如 https://github.com 或 http://localhost:5173"
                  className="h-8 w-full border border-line bg-canvas px-2.5 font-mono text-[12px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
                />
              </div>
              <div>
                <label className="mb-1 block text-[11px] text-ink-muted">
                  标签名称（可选，默认使用页面标题或域名）
                </label>
                <input
                  value={webTitle}
                  onChange={(e) => setWebTitle(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void handleLaunch();
                    }
                  }}
                  placeholder="例如 本地预览 / 文档 / 监控面板"
                  className="h-8 w-full border border-line bg-canvas px-2.5 text-[12px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
                />
              </div>
            </div>
          )}

          {/* Session Type (fixed at creation) */}
          {!isWebSelection && !isShell && supportsNativeChat && activePreset && (
            <div>
              <label className="mb-1.5 block text-[11.5px] text-ink-muted">会话类型</label>
              <div className="grid grid-cols-2 border border-line bg-base p-[2px]">
                <button
                  type="button"
                  onClick={() => setSurface("chat")}
                  className={`flex h-8 items-center justify-center gap-1.5 text-[12px] transition-colors cursor-pointer ${
                    surface !== "terminal"
                      ? "bg-active font-medium text-ink-strong"
                      : "text-ink-muted hover:text-ink"
                  }`}
                >
                  <MessageSquare className="h-3.5 w-3.5" />
                  <span>对话会话</span>
                </button>
                <button
                  type="button"
                  onClick={() => setSurface("terminal")}
                  className={`flex h-8 items-center justify-center gap-1.5 text-[12px] transition-colors cursor-pointer ${
                    surface === "terminal"
                      ? "bg-active font-medium text-ink-strong"
                      : "text-ink-muted hover:text-ink"
                  }`}
                >
                  <TerminalIcon className="h-3.5 w-3.5" />
                  <span>终端会话</span>
                </button>
              </div>
            </div>
          )}

          {/* Optional Initial Prompt */}
          {!isWebSelection && !isShell && (
            <div>
              <label className="mb-1.5 block text-[11.5px] text-ink-muted">初始任务（可选）</label>
              <textarea
                rows={2}
                value={initialPrompt}
                onChange={(e) => setInitialPrompt(e.target.value)}
                placeholder="例如：帮我检查当前分支的未提交变更并跑一遍测试"
                className="w-full resize-none border border-line bg-canvas px-3 py-2 text-[13px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
              />
            </div>
          )}

          {formError && (
            <div className="border-l-2 border-danger bg-danger-soft px-3 py-1.5 text-[12px] text-danger">
              {formError}
            </div>
          )}
        </div>

        <div className="flex items-center justify-end gap-2 border-t border-line-subtle px-4 py-3">
          <button
            type="button"
            onClick={onClose}
            className="h-8 border border-line bg-canvas px-3.5 text-[12.5px] text-ink-2 hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
          >
            取消
          </button>
          <button
            type="button"
            disabled={!projectId || (!isWebSelection && !activePreset) || spawning}
            onClick={() => void handleLaunch()}
            className="h-8 min-w-[88px] bg-primary-btn px-4 text-[12.5px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover disabled:opacity-45 transition-colors cursor-pointer"
          >
            {spawning ? "启动中…" : isWebSelection ? "打开网页" : "启动会话"}
          </button>
        </div>
      </div>
    </div>
  );
};
