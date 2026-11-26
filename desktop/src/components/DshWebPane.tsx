import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  ExternalLink,
  Globe,
  Link2,
  Loader2,
  Power,
  RefreshCw,
  RotateCw,
  X,
} from "lucide-react";
import type { DshInstanceInfo } from "../shared/protocol";

interface WebviewElement extends HTMLElement {
  src: string;
  reload: () => void;
}

interface DshWebPaneProps {
  projectId: string;
  projectName: string;
  projectPath: string;
  visible: boolean;
  onCloseTab: () => void;
}

const CUSTOM_URL_KEY_PREFIX = "muxlane.dsh.customUrl.v1:";

function readSavedCustomUrl(projectId: string): string {
  try {
    return window.localStorage.getItem(CUSTOM_URL_KEY_PREFIX + projectId) || "";
  } catch {
    return "";
  }
}

function writeSavedCustomUrl(projectId: string, url: string): void {
  try {
    if (url.trim()) {
      window.localStorage.setItem(CUSTOM_URL_KEY_PREFIX + projectId, url.trim());
    } else {
      window.localStorage.removeItem(CUSTOM_URL_KEY_PREFIX + projectId);
    }
  } catch {
    // ignore
  }
}

export const DshWebPane: React.FC<DshWebPaneProps> = ({
  projectId,
  projectName,
  projectPath,
  visible,
  onCloseTab,
}) => {
  const webviewRef = useRef<WebviewElement | null>(null);
  const [info, setInfo] = useState<DshInstanceInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editingUrl, setEditingUrl] = useState(false);
  const [urlInput, setUrlInput] = useState(() => readSavedCustomUrl(projectId));

  const bootDsh = useCallback(
    async (customUrl?: string) => {
      setLoading(true);
      setError(null);
      try {
        const res = await window.muxlane.openDshWeb(projectId, projectPath, customUrl);
        setInfo(res);
      } catch (err) {
        setError((err as Error).message || "启动 DeepSeek Harness WebUI 失败");
      } finally {
        setLoading(false);
      }
    },
    [projectId, projectPath],
  );

  useEffect(() => {
    const saved = readSavedCustomUrl(projectId);
    void bootDsh(saved || undefined);
  }, [bootDsh, projectId]);

  const handleRestart = async () => {
    writeSavedCustomUrl(projectId, "");
    setUrlInput("");
    setEditingUrl(false);
    setLoading(true);
    setError(null);
    try {
      const res = await window.muxlane.restartDshWeb(projectId, projectPath);
      setInfo(res);
    } catch (err) {
      setError((err as Error).message || "重启 DeepSeek Harness WebUI 失败");
    } finally {
      setLoading(false);
    }
  };

  const handleApplyCustomUrl = async (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = urlInput.trim();
    writeSavedCustomUrl(projectId, trimmed);
    setEditingUrl(false);
    await bootDsh(trimmed || undefined);
  };

  const displayUrl = info?.url ? info.url.replace(/([?&]token=)[^&]+/i, "$1***") : "";

  return (
    <div
      className={`flex h-full min-h-0 flex-1 flex-col bg-canvas ${visible ? "" : "hidden"}`}
    >
      {/* Compact 30px DSH WebUI utility bar (0px square corners) */}
      <div className="flex h-[30px] shrink-0 items-center justify-between gap-2 border-b border-line-subtle bg-sidebar/70 px-3 text-[11px] text-ink-muted select-none">
        <div className="flex min-w-0 items-center gap-2">
          <Globe className="h-3.5 w-3.5 shrink-0 text-working" strokeWidth={1.75} />
          <span className="shrink-0 font-medium text-ink">DeepSeek Harness</span>
          <span className="shrink-0 text-ink-faint">·</span>
          <span className="truncate font-mono text-[10.5px] text-ink-2">{projectName}</span>
          {displayUrl && (
            <>
              <span className="shrink-0 text-ink-faint">·</span>
              <span className="truncate font-mono text-[10.5px] text-ink-muted">
                {displayUrl}
              </span>
            </>
          )}
        </div>

        <div className="flex shrink-0 items-center gap-1">
          <button
            type="button"
            onClick={() => setEditingUrl((v) => !v)}
            title="连接自定义/已运行的 DSH WebUI 地址"
            className={`inline-flex h-5 items-center gap-1 border px-1.5 text-[10.5px] transition-colors cursor-pointer ${
              editingUrl
                ? "border-line-strong bg-active text-ink-strong"
                : "border-line-subtle bg-canvas text-ink-2 hover:bg-hover hover:text-ink"
            }`}
          >
            <Link2 className="h-3 w-3" />
            <span>{info?.isCustomUrl ? "自定义地址" : "连接地址"}</span>
          </button>

          <button
            type="button"
            disabled={!info?.url}
            onClick={() => webviewRef.current?.reload()}
            title="刷新 WebUI 页面"
            className="flex h-5 w-5 items-center justify-center border border-line-subtle bg-canvas text-ink-2 hover:bg-hover hover:text-ink disabled:opacity-40 cursor-pointer"
          >
            <RefreshCw className="h-3 w-3" />
          </button>

          <button
            type="button"
            disabled={loading}
            onClick={() => void handleRestart()}
            title="重启此项目的本地 DSH WebUI 服务"
            className="flex h-5 w-5 items-center justify-center border border-line-subtle bg-canvas text-ink-2 hover:bg-hover hover:text-ink disabled:opacity-40 cursor-pointer"
          >
            <RotateCw className={`h-3 w-3 ${loading ? "animate-spin" : ""}`} />
          </button>

          {info?.url && (
            <button
              type="button"
              onClick={() => void window.muxlane.openExternal(info.url)}
              title="在系统浏览器中打开"
              className="flex h-5 w-5 items-center justify-center border border-line-subtle bg-canvas text-ink-2 hover:bg-hover hover:text-ink cursor-pointer"
            >
              <ExternalLink className="h-3 w-3" />
            </button>
          )}

          <button
            type="button"
            onClick={onCloseTab}
            title="关闭并停止此项目 DSH WebUI 标签"
            className="flex h-5 w-5 items-center justify-center border border-line-subtle bg-canvas text-ink-muted hover:border-danger/40 hover:bg-danger-soft hover:text-danger cursor-pointer"
          >
            <Power className="h-3 w-3" />
          </button>
        </div>
      </div>

      {/* Inline Custom URL Bar */}
      {editingUrl && (
        <form
          onSubmit={(e) => void handleApplyCustomUrl(e)}
          className="flex shrink-0 items-center gap-2 border-b border-line-subtle bg-elevated px-3 py-1.5"
        >
          <input
            value={urlInput}
            onChange={(e) => setUrlInput(e.target.value)}
            placeholder="留空自动启动本机 dsh web；或粘贴已有地址，例如 http://100.64.10.138:3080/?token=..."
            className="h-7 min-w-0 flex-1 border border-line bg-canvas px-2 font-mono text-[11.5px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
          />
          <button
            type="submit"
            className="h-7 bg-primary-btn px-2.5 text-[11.5px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover cursor-pointer"
          >
            应用
          </button>
          <button
            type="button"
            onClick={() => setEditingUrl(false)}
            className="flex h-7 w-7 items-center justify-center border border-line bg-canvas text-ink-muted hover:bg-hover hover:text-ink cursor-pointer"
            aria-label="取消"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </form>
      )}

      {/* Main Content */}
      <div className="relative min-h-0 flex-1 bg-canvas">
        {loading ? (
          <div className="flex h-full flex-col items-center justify-center gap-2.5 text-ink-muted">
            <Loader2 className="h-5 w-5 animate-spin text-working" />
            <div className="text-[12.5px] text-ink">
              正在为 <span className="font-mono text-ink-strong">{projectName}</span> 启动 DeepSeek
              Harness WebUI…
            </div>
            <div className="font-mono text-[11px] text-ink-muted">{projectPath}</div>
          </div>
        ) : error ? (
          <div className="flex h-full items-center justify-center p-6">
            <div className="w-full max-w-lg border border-line-strong bg-elevated p-4 space-y-3">
              <div className="text-[13px] font-semibold text-danger">
                无法启动 DeepSeek Harness WebUI
              </div>
              <div className="border-l-2 border-danger bg-danger-soft px-3 py-2 font-mono text-[11.5px] text-danger whitespace-pre-wrap break-words select-text">
                {error}
              </div>
              <div className="text-[11.5px] text-ink-muted">
                你可以重试自动启动，或点击右上角「连接地址」直接接入已运行的 `dsh web` URL。
              </div>
              <div className="flex items-center gap-2 pt-1">
                <button
                  type="button"
                  onClick={() => void handleRestart()}
                  className="h-7 bg-primary-btn px-3 text-[12px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover cursor-pointer"
                >
                  重试启动
                </button>
                <button
                  type="button"
                  onClick={() => setEditingUrl(true)}
                  className="h-7 border border-line bg-canvas px-3 text-[12px] text-ink hover:bg-hover cursor-pointer"
                >
                  填入已有 WebUI 地址
                </button>
              </div>
            </div>
          </div>
        ) : info?.url ? (
          React.createElement("webview", {
            ref: webviewRef,
            src: info.url,
            partition: `persist:dsh-${projectId}`,
            allowpopups: "true",
            style: { width: "100%", height: "100%", display: "inline-flex", border: "none" },
          })
        ) : null}
      </div>
    </div>
  );
};
