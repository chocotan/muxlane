import React, { useEffect, useMemo, useState } from "react";
import { MessageSquare, Terminal as TerminalIcon, X } from "lucide-react";
import {
  presetChatDriver,
  type AgentPreset,
  type Project,
  type SessionViewMode,
} from "../shared/protocol";
import { orderPresetsPiFirst } from "../shared/timeline";

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
}) => {
  const [projectId, setProjectId] = useState(initialProjectId || projects[0]?.id || "");
  const [selectedPresetId, setSelectedPresetId] = useState<string>("");
  const [surface, setSurface] = useState<SessionViewMode>(
    defaultSurface === "terminal" ? "terminal" : "chat",
  );
  const [initialPrompt, setInitialPrompt] = useState("");
  const [loadingPresets, setLoadingPresets] = useState(false);
  const [spawning, setSpawning] = useState(false);

  useEffect(() => {
    if (!open) return;
    const pid = initialProjectId || projects[0]?.id || "";
    setProjectId(pid);
    setSurface(defaultSurface === "terminal" ? "terminal" : "chat");
    setInitialPrompt("");

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
  const allOrdered = useMemo(
    () => (shell ? [...agents, shell] : agents),
    [agents, shell],
  );

  useEffect(() => {
    if (!open) return;
    if (allOrdered.length > 0 && !allOrdered.some((p) => p.id === selectedPresetId)) {
      setSelectedPresetId(allOrdered[0].id);
    }
  }, [allOrdered, open, selectedPresetId]);

  if (!open) return null;

  const activePreset = allOrdered.find((p) => p.id === selectedPresetId) ?? allOrdered[0] ?? null;
  const isShell = activePreset?.agent_type === "shell";
  const supportsNativeChat = activePreset
    ? presetChatDriver(activePreset.agent_type) !== null
    : false;

  const handleLaunch = async () => {
    if (!projectId || !activePreset || spawning) return;
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
              onChange={(e) => setProjectId(e.target.value)}
              className="h-9 w-full border border-line bg-canvas px-3 text-[13px] text-ink-strong focus:border-line-strong focus:outline-none"
            >
              {projects.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name} — {p.path}
                </option>
              ))}
            </select>
          </div>

          {/* Preset picker */}
          <div>
            <label className="mb-1.5 block text-[11.5px] text-ink-muted">Agent / Shell</label>
            {loadingPresets && allOrdered.length === 0 ? (
              <div className="border border-line-subtle bg-canvas p-4 text-center text-[12.5px] text-ink-muted">
                正在检测已安装的 CLI 工具…
              </div>
            ) : (
              <div className="grid grid-cols-2 gap-1.5">
                {allOrdered.map((preset) => {
                  const selected = preset.id === activePreset?.id;
                  return (
                    <button
                      key={preset.id}
                      type="button"
                      onClick={() => setSelectedPresetId(preset.id)}
                      className={`flex items-center justify-between border px-3 py-2 text-left transition-colors cursor-pointer ${
                        selected
                          ? "border-line-strong bg-active text-ink-strong"
                          : "border-line-subtle bg-canvas text-ink-2 hover:border-line hover:bg-hover hover:text-ink"
                      }`}
                    >
                      <div className="min-w-0">
                        <div className="truncate text-[12.5px] font-medium">
                          {preset.label}
                        </div>
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

          {/* Session Type (fixed at creation) */}
          {!isShell && supportsNativeChat && activePreset && (
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
          {!isShell && (
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
            disabled={!projectId || !activePreset || spawning}
            onClick={() => void handleLaunch()}
            className="h-8 min-w-[88px] bg-primary-btn px-4 text-[12.5px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover disabled:opacity-45 transition-colors cursor-pointer"
          >
            {spawning ? "启动中…" : "启动会话"}
          </button>
        </div>
      </div>
    </div>
  );
};
