import React, { useEffect, useMemo, useState } from "react";
import { Globe, MessageSquare, Terminal as TerminalIcon, X } from "lucide-react";
import {
  presetChatDriver,
  type AgentPreset,
  type Project,
  type SessionViewMode,
} from "../shared/protocol";
import { orderPresetsPiFirst } from "../shared/timeline";
import { normalizeWebUrl } from "../state/webSessions";

const PRESET_CUSTOM_WEB = "__muxlane_web_url__";

interface NewSessionModalProps {
  open: boolean;
  projects: Project[];
  initialProjectId: string | null;
  defaultSurface: SessionViewMode;
  presetsByProject: Record<string, AgentPreset[]>;
  onLoadPresets: (projectId: string) => Promise<AgentPreset[]>;
  onClose: () => void;
  onSpawn: (opts: {
    projectId: string;
    preset: AgentPreset;
    surface: SessionViewMode;
    initialPrompt?: string;
  }) => Promise<unknown>;
  onCreateWebSession: (opts: { projectId: string; url: string; title?: string }) => unknown;
}

export const NewSessionModal: React.FC<NewSessionModalProps> = ({
  open,
  projects,
  initialProjectId,
  defaultSurface,
  presetsByProject,
  onLoadPresets,
  onClose,
  onSpawn,
  onCreateWebSession,
}) => {
  const [projectId, setProjectId] = useState(initialProjectId || projects[0]?.id || "");
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

  useEffect(() => {
    if (!open) return;
    const pid = initialProjectId || projects[0]?.id || "";
    setProjectId(pid);
    setSelectedPresetId("");
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
    if (!open || !projectId) return;
    setLoadingPresets(true);
    void onLoadPresets(projectId).finally(() => setLoadingPresets(false));
  }, [onLoadPresets, open, projectId]);

  const presets = presetsByProject[projectId] ?? [];
  const { agents, shell } = useMemo(() => orderPresetsPiFirst(presets), [presets]);
  const allOrdered = useMemo(() => (shell ? [...agents, shell] : agents), [agents, shell]);

  useEffect(() => {
    if (!open) return;
    if (
      selectedPresetId !== PRESET_CUSTOM_WEB &&
      allOrdered.length > 0 &&
      !allOrdered.some((p) => p.id === selectedPresetId)
    ) {
      setSelectedPresetId(allOrdered[0].id);
    }
  }, [allOrdered, open, selectedPresetId]);

  if (!open) return null;

  const isCustomWeb = selectedPresetId === PRESET_CUSTOM_WEB;
  const activePreset = isCustomWeb
    ? null
    : (allOrdered.find((p) => p.id === selectedPresetId) ?? allOrdered[0] ?? null);
  const isShell = activePreset?.agent_type === "shell";
  const supportsNativeChat = activePreset
    ? presetChatDriver(activePreset.agent_type) !== null
    : false;

  const handleLaunch = async () => {
    if (!projectId || spawning) return;
    setFormError(null);

    if (isCustomWeb) {
      try {
        const trimmedUrl = webUrl.trim();
        if (!trimmedUrl) {
          setFormError("请输入要访问的网页地址，例如 https://github.com 或 http://localhost:3000");
          return;
        }
        const normalized = normalizeWebUrl(trimmedUrl);
        onCreateWebSession({
          projectId,
          url: normalized,
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
          {/* Project select */}
          <div>
            <label className="mb-1.5 block text-[11.5px] text-ink-muted">目标项目</label>
            <select
              value={projectId}
              onChange={(e) => {
                setFormError(null);
                setProjectId(e.target.value);
              }}
              className="h-9 w-full border border-line bg-canvas px-3 text-[13px] text-ink-strong focus:border-line-strong focus:outline-none"
            >
              {projects.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name} — {p.path}
                </option>
              ))}
            </select>
          </div>

          {/* Agent / Shell presets */}
          <div>
            <label className="mb-1.5 block text-[11.5px] text-ink-muted">Agent / Shell</label>
            {loadingPresets && allOrdered.length === 0 ? (
              <div className="border border-line-subtle bg-canvas p-4 text-center text-[12.5px] text-ink-muted">
                正在检测已安装的 CLI 工具…
              </div>
            ) : (
              <div className="grid grid-cols-2 gap-1.5">
                {allOrdered.map((preset) => {
                  const selected = !isCustomWeb && preset.id === activePreset?.id;
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

          {/* Browser / Webpage preset */}
          <div>
            <label className="mb-1.5 block text-[11.5px] text-ink-muted">浏览器 / 网页</label>
            <button
              type="button"
              onClick={() => {
                setFormError(null);
                setSelectedPresetId(PRESET_CUSTOM_WEB);
              }}
              className={`flex w-full items-center gap-2.5 border px-3 py-2 text-left transition-colors cursor-pointer ${
                isCustomWeb
                  ? "border-line-strong bg-active text-ink-strong"
                  : "border-line-subtle bg-canvas text-ink-2 hover:border-line hover:bg-hover hover:text-ink"
              }`}
            >
              <Globe className="h-4 w-4 shrink-0 text-ink-muted" strokeWidth={1.75} />
              <div className="min-w-0">
                <div className="truncate text-[12.5px] font-medium">网页浏览器</div>
                <div className="truncate font-mono text-[10.5px] text-ink-muted">
                  在当前项目标签页中访问任意 HTTP / HTTPS 网址
                </div>
              </div>
            </button>
          </div>

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
          {!isCustomWeb && !isShell && supportsNativeChat && activePreset && (
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
          {!isCustomWeb && !isShell && (
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
            disabled={!projectId || (!isCustomWeb && !activePreset) || spawning}
            onClick={() => void handleLaunch()}
            className="h-8 min-w-[88px] bg-primary-btn px-4 text-[12.5px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover disabled:opacity-45 transition-colors cursor-pointer"
          >
            {spawning ? "启动中…" : isCustomWeb ? "打开网页" : "启动会话"}
          </button>
        </div>
      </div>
    </div>
  );
};
