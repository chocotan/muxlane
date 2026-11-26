import React, { useEffect, useState } from "react";
import {
  Check,
  Copy,
  Keyboard,
  Palette,
  RefreshCw,
  Sliders,
  X,
} from "lucide-react";
import type {
  DaemonConnectionStatus,
  MachineInfo,
  RelayConfig,
  SessionViewMode,
} from "../shared/protocol";
import {
  FONT_FAMILY_OPTIONS,
  type DesktopSettings,
  type TerminalFontFamily,
} from "../state/useSettings";
import { THEME_DEFINITIONS } from "../state/useTheme";

interface SettingsModalProps {
  open: boolean;
  settings: DesktopSettings;
  connectionStatus: DaemonConnectionStatus;
  machine: MachineInfo | undefined;
  onUpdateSettings: (patch: Partial<DesktopSettings>) => void;
  onResetSettings: () => void;
  onReconnect: () => void;
  onClose: () => void;
}

type SettingsTab = "appearance" | "general" | "shortcuts";

const UI_SCALES = [85, 90, 95, 100, 110, 125];

const SHORTCUT_ITEMS = [
  { label: "新建会话", keys: "Ctrl + N" },
  { label: "命令面板", keys: "Ctrl + K" },
  { label: "切换左侧栏", keys: "Ctrl + B" },
  { label: "切换右侧栏", keys: "Ctrl + ]" },
  { label: "关闭当前标签页", keys: "Ctrl + W" },
  { label: "远程机器管理", keys: "Ctrl + Shift + M" },
  { label: "打开设置", keys: "Ctrl + ," },
];

/** Fixed 28×28 square copy button so clicking never shifts adjacent layout. */
const FixedCopyIconButton: React.FC<{ text: string; title?: string }> = ({
  text,
  title = "复制",
}) => {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      onClick={() => {
        void navigator.clipboard.writeText(text);
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      }}
      title={copied ? "已复制" : title}
      aria-label={copied ? "已复制" : title}
      className="flex h-7 w-7 shrink-0 items-center justify-center border border-line bg-canvas text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
    >
      {copied ? (
        <Check className="h-3.5 w-3.5 text-success" />
      ) : (
        <Copy className="h-3.5 w-3.5" />
      )}
    </button>
  );
};

const SquareToggle: React.FC<{
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
}> = ({ checked, onChange, label }) => (
  <button
    type="button"
    role="switch"
    aria-checked={checked}
    aria-label={label}
    onClick={() => onChange(!checked)}
    className={`relative flex h-5 w-9 shrink-0 items-center border transition-colors cursor-pointer ${
      checked
        ? "border-primary-btn bg-primary-btn"
        : "border-line bg-canvas"
    }`}
  >
    <span
      className={`block h-3 w-3 transition-transform ${
        checked
          ? "translate-x-[18px] bg-primary-btn-fg"
          : "translate-x-[3px] bg-ink-muted"
      }`}
    />
  </button>
);

export const SettingsModal: React.FC<SettingsModalProps> = ({
  open,
  settings,
  connectionStatus,
  machine,
  onUpdateSettings,
  onResetSettings,
  onReconnect,
  onClose,
}) => {
  const [tab, setTab] = useState<SettingsTab>("appearance");
  const [relayCfg, setRelayCfg] = useState<RelayConfig>({
    relayEnabled: false,
    relayUrl: "",
  });
  const [relaySaved, setRelaySaved] = useState(false);

  useEffect(() => {
    if (!open) return;
    void window.muxlane
      .getRelayConfig()
      .then((cfg) => setRelayCfg(cfg))
      .catch(() => {});

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose, open]);

  if (!open) return null;

  const handleToggleNotifications = async (next: boolean) => {
    if (next && typeof Notification !== "undefined" && Notification.permission === "default") {
      try {
        await Notification.requestPermission();
      } catch {
        // ignore
      }
    }
    onUpdateSettings({ notificationsEnabled: next });
  };

  const handleSaveRelay = async (next: RelayConfig) => {
    setRelayCfg(next);
    const saved = await window.muxlane.saveRelayConfig(next);
    setRelayCfg(saved);
    setRelaySaved(true);
    setTimeout(() => setRelaySaved(false), 1600);
  };

  const navItems: Array<{ id: SettingsTab; label: string; icon: React.ComponentType<{ className?: string; strokeWidth?: number }> }> = [
    { id: "appearance", label: "外观与终端", icon: Palette },
    { id: "general", label: "通用与中继", icon: Sliders },
    { id: "shortcuts", label: "快捷键", icon: Keyboard },
  ];

  return (
    <div
      onClick={onClose}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4"
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="mux-popover-shadow flex h-[520px] w-full max-w-[680px] flex-col border border-line-strong bg-elevated"
      >
        {/* Header */}
        <div className="flex h-11 shrink-0 items-center justify-between border-b border-line-subtle px-4">
          <span className="text-[13.5px] font-semibold text-ink-strong">设置</span>
          <button
            type="button"
            onClick={onClose}
            aria-label="关闭"
            className="flex h-6 w-6 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Body */}
        <div className="flex min-h-0 flex-1">
          {/* Left Navigation Column */}
          <div className="flex w-44 shrink-0 flex-col justify-between border-r border-line-subtle bg-sidebar p-2 select-none">
            <div className="space-y-0.5">
              {navItems.map((item) => {
                const Icon = item.icon;
                const active = tab === item.id;
                return (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => setTab(item.id)}
                    className={`relative flex h-8 w-full items-center gap-2 px-2.5 text-left text-[12.5px] transition-colors cursor-pointer ${
                      active
                        ? "bg-active font-medium text-ink-strong"
                        : "text-ink-2 hover:bg-hover hover:text-ink"
                    }`}
                  >
                    {active && (
                      <span className="absolute left-0 top-1 bottom-1 w-[2px] bg-ink-strong" />
                    )}
                    <Icon className="h-3.5 w-3.5 shrink-0 text-ink-muted" strokeWidth={1.75} />
                    <span>{item.label}</span>
                  </button>
                );
              })}
            </div>

            <button
              type="button"
              onClick={onResetSettings}
              className="h-7 w-full border border-line-subtle bg-canvas px-2 text-[11.5px] text-ink-muted hover:bg-hover hover:text-ink transition-colors cursor-pointer"
            >
              恢复默认设置
            </button>
          </div>

          {/* Right Settings Content */}
          <div className="min-w-0 flex-1 overflow-y-auto p-5">
            {tab === "appearance" && (
              <div className="divide-y divide-line-subtle">
                {/* Theme */}
                <div className="pb-4">
                  <div className="flex items-center justify-between gap-4">
                    <div>
                      <div className="text-[13px] font-medium text-ink-strong">界面主题</div>
                      <div className="mt-0.5 text-[11.5px] text-ink-muted">
                        同步切换对话画布、代码高亮与内置终端配色（共 {THEME_DEFINITIONS.length} 套主题）
                      </div>
                    </div>
                  </div>
                  <div className="mt-3 grid grid-cols-5 gap-1.5">
                    {THEME_DEFINITIONS.map((opt) => {
                      const active = settings.theme === opt.id;
                      return (
                        <button
                          key={opt.id}
                          type="button"
                          onClick={() => onUpdateSettings({ theme: opt.id })}
                          title={`${opt.label} (${opt.englishLabel})`}
                          className={`group flex flex-col border p-1.5 text-left transition-colors cursor-pointer ${
                            active
                              ? "border-primary-btn bg-active"
                              : "border-line bg-canvas hover:border-line-strong hover:bg-hover"
                          }`}
                        >
                          {/* Compact 0px architectural miniature theme preview */}
                          <div
                            className="flex h-6 w-full border border-black/15 overflow-hidden"
                            style={{ backgroundColor: opt.preview.canvas }}
                          >
                            <div
                              className="h-full w-2.5 shrink-0"
                              style={{ backgroundColor: opt.preview.sidebar }}
                            />
                            <div className="flex flex-1 items-center justify-between px-1.5">
                              <span
                                className="h-1 w-4"
                                style={{ backgroundColor: opt.preview.ink, opacity: 0.7 }}
                              />
                              <span
                                className="h-2 w-2 shrink-0"
                                style={{ backgroundColor: opt.preview.accent }}
                              />
                            </div>
                          </div>
                          <div className="mt-1 flex items-center justify-between gap-1">
                            <span
                              className={`truncate text-[11.5px] ${
                                active ? "font-semibold text-ink-strong" : "text-ink"
                              }`}
                            >
                              {opt.label}
                            </span>
                            {active && (
                              <Check className="h-3 w-3 shrink-0 text-ink-strong" strokeWidth={2.2} />
                            )}
                          </div>
                        </button>
                      );
                    })}
                  </div>
                </div>

                {/* UI Scale */}
                <div className="flex items-center justify-between gap-4 py-4">
                  <div>
                    <div className="text-[13px] font-medium text-ink-strong">界面缩放</div>
                    <div className="mt-0.5 text-[11.5px] text-ink-muted">
                      等比缩放整个桌面工作台
                    </div>
                  </div>
                  <div className="flex border border-line bg-base p-[2px]">
                    {UI_SCALES.map((pct) => (
                      <button
                        key={pct}
                        type="button"
                        onClick={() => onUpdateSettings({ uiScale: pct })}
                        className={`h-6 px-2 font-mono text-[11.5px] tabular-nums transition-colors cursor-pointer ${
                          settings.uiScale === pct
                            ? "bg-active font-medium text-ink-strong"
                            : "text-ink-muted hover:text-ink"
                        }`}
                      >
                        {pct}%
                      </button>
                    ))}
                  </div>
                </div>

                {/* Terminal Font Family */}
                <div className="flex items-center justify-between gap-4 py-4">
                  <div>
                    <div className="text-[13px] font-medium text-ink-strong">终端字体</div>
                    <div className="mt-0.5 text-[11.5px] text-ink-muted">
                      实时应用到所有交互式终端面板
                    </div>
                  </div>
                  <select
                    value={settings.terminalFontFamily}
                    onChange={(e) =>
                      onUpdateSettings({
                        terminalFontFamily: e.target.value as TerminalFontFamily,
                      })
                    }
                    className="h-8 w-44 border border-line bg-canvas px-2.5 font-mono text-[12px] text-ink-strong focus:border-line-strong focus:outline-none"
                  >
                    {FONT_FAMILY_OPTIONS.map((opt) => (
                      <option key={opt.value} value={opt.value}>
                        {opt.value}
                      </option>
                    ))}
                  </select>
                </div>

                {/* Terminal Font Size */}
                <div className="flex items-center justify-between gap-4 py-4">
                  <div>
                    <div className="text-[13px] font-medium text-ink-strong">终端字号</div>
                    <div className="mt-0.5 text-[11.5px] text-ink-muted">
                      支持 10px – 20px，调整后自动重新适配行列
                    </div>
                  </div>
                  <div className="flex items-center border border-line bg-canvas">
                    <button
                      type="button"
                      onClick={() =>
                        onUpdateSettings({
                          terminalFontSize: Math.max(10, settings.terminalFontSize - 1),
                        })
                      }
                      className="flex h-7 w-7 items-center justify-center text-ink-2 hover:bg-hover hover:text-ink-strong cursor-pointer"
                    >
                      -
                    </button>
                    <span className="w-12 text-center font-mono text-[12px] text-ink-strong tabular-nums">
                      {settings.terminalFontSize}px
                    </span>
                    <button
                      type="button"
                      onClick={() =>
                        onUpdateSettings({
                          terminalFontSize: Math.min(20, settings.terminalFontSize + 1),
                        })
                      }
                      className="flex h-7 w-7 items-center justify-center text-ink-2 hover:bg-hover hover:text-ink-strong cursor-pointer"
                    >
                      +
                    </button>
                  </div>
                </div>

                {/* Default View Mode */}
                <div className="flex items-center justify-between gap-4 pt-4">
                  <div>
                    <div className="text-[13px] font-medium text-ink-strong">新建会话默认类型</div>
                    <div className="mt-0.5 text-[11.5px] text-ink-muted">
                      新建 AI Agent 会话时默认选中的类型（创建后不再运行中切换）
                    </div>
                  </div>
                  <div className="flex border border-line bg-base p-[2px]">
                    {(
                      [
                        { id: "chat", label: "对话" },
                        { id: "terminal", label: "终端" },
                      ] as Array<{ id: SessionViewMode; label: string }>
                    ).map((opt) => (
                      <button
                        key={opt.id}
                        type="button"
                        onClick={() => onUpdateSettings({ defaultViewMode: opt.id })}
                        className={`h-6 w-14 text-[12px] transition-colors cursor-pointer ${
                          settings.defaultViewMode === opt.id
                            ? "bg-active font-medium text-ink-strong"
                            : "text-ink-muted hover:text-ink"
                        }`}
                      >
                        {opt.label}
                      </button>
                    ))}
                  </div>
                </div>
              </div>
            )}

            {tab === "general" && (
              <div className="divide-y divide-line-subtle">
                {/* Project-scoped workspace tabs (matches Rust project_workspaces) */}
                <div className="flex items-center justify-between gap-4 pb-4">
                  <div>
                    <div className="text-[13px] font-medium text-ink-strong">按项目隔离标签页</div>
                    <div className="mt-0.5 text-[11.5px] text-ink-muted">
                      每个项目独立保存右侧打开的会话与 DSH 标签；关闭后所有项目共享同一组标签栏
                    </div>
                  </div>
                  <SquareToggle
                    checked={settings.projectWorkspacesEnabled}
                    onChange={(next) => onUpdateSettings({ projectWorkspacesEnabled: next })}
                    label="按项目隔离标签页"
                  />
                </div>

                {/* Desktop Notifications */}
                <div className="flex items-center justify-between gap-4 py-4">
                  <div>
                    <div className="text-[13px] font-medium text-ink-strong">系统桌面通知</div>
                    <div className="mt-0.5 text-[11.5px] text-ink-muted">
                      后台会话请求审批确认时弹出系统通知
                    </div>
                  </div>
                  <SquareToggle
                    checked={settings.notificationsEnabled}
                    onChange={(next) => void handleToggleNotifications(next)}
                    label="系统桌面通知"
                  />
                </div>

                {/* Daemon Socket & Machine ID */}
                <div className="space-y-3 py-4">
                  <div className="flex items-center justify-between gap-4">
                    <div>
                      <div className="text-[13px] font-medium text-ink-strong">守护进程连接</div>
                      <div className="mt-0.5 font-mono text-[11px] text-ink-muted break-all select-text">
                        {connectionStatus.socketPath || "未配置"}
                      </div>
                    </div>
                    <button
                      type="button"
                      onClick={onReconnect}
                      className="flex h-7 shrink-0 items-center gap-1.5 border border-line bg-canvas px-2.5 text-[12px] text-ink hover:bg-hover transition-colors cursor-pointer"
                    >
                      <RefreshCw className="h-3 w-3" />
                      <span>重连</span>
                    </button>
                  </div>

                  {machine && (
                    <div className="flex items-center justify-between gap-3 border border-line-subtle bg-canvas px-3 py-2">
                      <div className="min-w-0">
                        <div className="text-[11px] text-ink-muted">当前机器 ID</div>
                        <div className="truncate font-mono text-[12px] text-ink-strong select-text">
                          {machine.machine_id}
                        </div>
                      </div>
                      <FixedCopyIconButton text={machine.machine_id} title="复制机器 ID" />
                    </div>
                  )}
                </div>

                {/* Mobile Relay Settings (synced with ~/.local/share/muxlane/state.json) */}
                <div className="space-y-3 pt-4">
                  <div className="flex items-center justify-between gap-4">
                    <div>
                      <div className="text-[13px] font-medium text-ink-strong">手机中继 (Relay)</div>
                      <div className="mt-0.5 text-[11.5px] text-ink-muted">
                        与本地 Muxlane state.json 同步，供移动端远程访问
                      </div>
                    </div>
                    <SquareToggle
                      checked={relayCfg.relayEnabled}
                      onChange={(next) =>
                        void handleSaveRelay({ ...relayCfg, relayEnabled: next })
                      }
                      label="手机中继"
                    />
                  </div>

                  <div className="flex items-center gap-2">
                    <input
                      value={relayCfg.relayUrl}
                      onChange={(e) =>
                        setRelayCfg((prev) => ({ ...prev, relayUrl: e.target.value }))
                      }
                      placeholder="wss://your-relay.example.com"
                      className="h-8 min-w-0 flex-1 border border-line bg-canvas px-2.5 font-mono text-[12px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
                    />
                    <button
                      type="button"
                      onClick={() => void handleSaveRelay(relayCfg)}
                      className="h-8 w-16 shrink-0 border border-line bg-canvas text-[12px] text-ink hover:bg-hover transition-colors cursor-pointer"
                    >
                      {relaySaved ? "已保存" : "应用"}
                    </button>
                  </div>
                </div>
              </div>
            )}

            {tab === "shortcuts" && (
              <div className="divide-y divide-line-subtle border border-line-subtle bg-canvas">
                {SHORTCUT_ITEMS.map((item) => (
                  <div
                    key={item.label}
                    className="flex items-center justify-between px-3.5 py-2.5 text-[12.5px]"
                  >
                    <span className="text-ink">{item.label}</span>
                    <kbd className="border border-line bg-elevated px-2 py-0.5 font-mono text-[11px] text-ink-2">
                      {item.keys}
                    </kbd>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};
