import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  ChevronDown,
  Copy,
  ExternalLink,
  Globe,
  KeyRound,
  Loader2,
  RefreshCw,
  RotateCw,
  Server,
} from "lucide-react";
import { extractDshToken, normalizeWebUrl, preserveDshToken } from "../state/webSessions";

interface WebviewElement extends HTMLElement {
  src: string;
  reload: () => void;
  goBack: () => void;
  goForward: () => void;
  canGoBack: () => boolean;
  canGoForward: () => boolean;
}

interface WebviewFailLoadEvent extends Event {
  errorCode: number;
  errorDescription: string;
  validatedURL: string;
  isMainFrame?: boolean;
}

interface WebviewNavigateEvent extends Event {
  url: string;
}

interface DshWorkspaceViewProps {
  hostId: string;
  hostName: string;
  isRemoteHost: boolean;
  remoteHostsCount: number;
  defaultProjectPath: string;
  visible: boolean;
  onOpenRemotes: () => void;
}

const CUSTOM_DSH_URL_PREFIX = "muxlane.dsh.hostUrl.v1:";

function readHostCustomDshUrl(hostId: string): string {
  try {
    return window.localStorage.getItem(CUSTOM_DSH_URL_PREFIX + hostId) || "";
  } catch {
    return "";
  }
}

function writeHostCustomDshUrl(hostId: string, url: string): void {
  try {
    if (url.trim()) {
      window.localStorage.setItem(CUSTOM_DSH_URL_PREFIX + hostId, url.trim());
    } else {
      window.localStorage.removeItem(CUSTOM_DSH_URL_PREFIX + hostId);
    }
  } catch {
    // ignore
  }
}

export const DshWorkspaceView: React.FC<DshWorkspaceViewProps> = ({
  hostId,
  hostName,
  isRemoteHost,
  remoteHostsCount,
  defaultProjectPath,
  visible,
  onOpenRemotes,
}) => {
  const webviewRef = useRef<WebviewElement | null>(null);
  const [resolvedUrl, setResolvedUrl] = useState<string>("");
  const [addressInput, setAddressInput] = useState<string>(() => readHostCustomDshUrl(hostId));
  const [reusedExisting, setReusedExisting] = useState(false);
  const [copiedKind, setCopiedKind] = useState<"url" | "token" | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [webviewError, setWebviewError] = useState<string | null>(null);
  const [canBack, setCanBack] = useState(false);
  const [canForward, setCanForward] = useState(false);

  const bootDsh = useCallback(
    async (customUrl?: string) => {
      setLoading(true);
      setError(null);
      setWebviewError(null);
      try {
        const res = await window.muxlane.openDshWeb(
          `host:${hostId}`,
          defaultProjectPath,
          customUrl,
        );
        setResolvedUrl(res.url);
        setAddressInput(res.url);
        setReusedExisting(Boolean(res.reusedExisting));
      } catch (err) {
        setError((err as Error).message || `无法启动 ${hostName} 的 DeepSeek Harness WebUI`);
      } finally {
        setLoading(false);
      }
    },
    [defaultProjectPath, hostId, hostName],
  );

  useEffect(() => {
    const saved = readHostCustomDshUrl(hostId);
    void bootDsh(saved || undefined);
    // Boot once per mounted host instance
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hostId]);

  useEffect(() => {
    const wv = webviewRef.current;
    if (!wv || !resolvedUrl) return;

    const syncNav = () => {
      try {
        setCanBack(wv.canGoBack());
        setCanForward(wv.canGoForward());
      } catch {
        // webview not attached yet
      }
    };

    const onNavigate = (ev: Event) => {
      const e = ev as WebviewNavigateEvent;
      if (e.url) setAddressInput(preserveDshToken(e.url, resolvedUrl));
      syncNav();
    };

    const onFailLoad = (ev: Event) => {
      const e = ev as WebviewFailLoadEvent;
      if (e.errorCode === -3 || e.isMainFrame === false) return;
      setWebviewError(
        `页面加载失败 (${e.errorCode} ${e.errorDescription || "ERR"}): ${e.validatedURL || resolvedUrl}`,
      );
    };

    const onFinishLoad = () => {
      setWebviewError(null);
      syncNav();
    };

    const onCrashed = () => {
      setWebviewError("WebView 渲染进程异常退出，请点击刷新或重启服务。");
    };

    wv.addEventListener("did-navigate", onNavigate);
    wv.addEventListener("did-navigate-in-page", onNavigate);
    wv.addEventListener("did-fail-load", onFailLoad);
    wv.addEventListener("did-finish-load", onFinishLoad);
    wv.addEventListener("crashed", onCrashed);
    wv.addEventListener("render-process-gone", onCrashed);
    return () => {
      wv.removeEventListener("did-navigate", onNavigate);
      wv.removeEventListener("did-navigate-in-page", onNavigate);
      wv.removeEventListener("did-fail-load", onFailLoad);
      wv.removeEventListener("did-finish-load", onFinishLoad);
      wv.removeEventListener("crashed", onCrashed);
      wv.removeEventListener("render-process-gone", onCrashed);
    };
  }, [resolvedUrl]);

  const handleRestart = async () => {
    writeHostCustomDshUrl(hostId, "");
    setLoading(true);
    setError(null);
    setWebviewError(null);
    try {
      const res = await window.muxlane.restartDshWeb(`host:${hostId}`, defaultProjectPath);
      setResolvedUrl(res.url);
      setAddressInput(res.url);
      setReusedExisting(Boolean(res.reusedExisting));
    } catch (err) {
      setError((err as Error).message || `重启 ${hostName} 的 DeepSeek Harness WebUI 失败`);
    } finally {
      setLoading(false);
    }
  };

  const handleAddressSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setWebviewError(null);
    try {
      const normalized = addressInput.trim() ? normalizeWebUrl(addressInput) : "";
      writeHostCustomDshUrl(hostId, normalized);
      await bootDsh(normalized || undefined);
    } catch (err) {
      setError((err as Error).message || "请输入有效的 http:// 或 https:// 地址");
    }
  };

  const activeError = error || webviewError;
  const dshToken = extractDshToken(resolvedUrl) || extractDshToken(addressInput);
  const copyValue = async (text: string, kind: "url" | "token") => {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopiedKind(kind);
      window.setTimeout(() => {
        setCopiedKind((prev) => (prev === kind ? null : prev));
      }, 1500);
    } catch {
      // ignore clipboard errors
    }
  };
  const externalOpenUrl = (() => {
    if (!resolvedUrl) return addressInput;
    if (!addressInput) return resolvedUrl;
    try {
      return new URL(addressInput).origin === new URL(resolvedUrl).origin
        ? resolvedUrl
        : addressInput;
    } catch {
      return resolvedUrl;
    }
  })();

  return (
    <div
      className={`absolute inset-0 flex flex-col bg-canvas ${
        visible ? "z-10 visible" : "z-0 invisible pointer-events-none"
      }`}
    >
      {/* 36px Machine-Aware DSH Top Bar (aligned with AppRail & WorkspaceHeader, 0px corners) */}
      <header className="flex h-9 shrink-0 items-center gap-2 border-b border-line-subtle bg-tab-bar px-2.5 text-[12px] text-ink-muted select-none">
        {/* Clickable Machine Switcher */}
        <button
          type="button"
          onClick={onOpenRemotes}
          title={
            remoteHostsCount > 0
              ? `切换当前机器的 DeepSeek Harness（已保存 ${remoteHostsCount} 台远程主机）`
              : "连接或切换远程机器"
          }
          className="flex h-6 shrink-0 items-center gap-1.5 border border-line-subtle bg-canvas px-2 text-left hover:border-line hover:bg-hover transition-colors cursor-pointer"
        >
          <Server
            className={`h-3.5 w-3.5 shrink-0 ${isRemoteHost ? "text-success" : "text-ink-muted"}`}
            strokeWidth={1.75}
          />
          <span className="max-w-[140px] truncate text-[12px] font-semibold text-ink-strong">
            {hostName}
          </span>
          <ChevronDown className="h-3 w-3 shrink-0 text-ink-muted" />
        </button>

        <div className="flex shrink-0 items-center gap-1.5 pr-1">
          <Globe className="h-3.5 w-3.5 text-working" strokeWidth={1.8} />
          <span className="font-medium text-ink">DeepSeek Harness</span>
        </div>

        {/* Navigation controls */}
        <div className="flex shrink-0 items-center gap-0.5">
          <button
            type="button"
            disabled={!canBack}
            onClick={() => webviewRef.current?.goBack()}
            title="后退"
            className="flex h-6 w-6 items-center justify-center text-ink-2 hover:bg-hover hover:text-ink-strong disabled:opacity-30 cursor-pointer"
          >
            <ArrowLeft className="h-3.5 w-3.5" />
          </button>
          <button
            type="button"
            disabled={!canForward}
            onClick={() => webviewRef.current?.goForward()}
            title="前进"
            className="flex h-6 w-6 items-center justify-center text-ink-2 hover:bg-hover hover:text-ink-strong disabled:opacity-30 cursor-pointer"
          >
            <ArrowRight className="h-3.5 w-3.5" />
          </button>
          <button
            type="button"
            disabled={!resolvedUrl}
            onClick={() => {
              setWebviewError(null);
              webviewRef.current?.reload();
            }}
            title="刷新页面"
            className="flex h-6 w-6 items-center justify-center text-ink-2 hover:bg-hover hover:text-ink-strong disabled:opacity-30 cursor-pointer"
          >
            <RefreshCw className="h-3.5 w-3.5" />
          </button>
        </div>

        {/* Editable URL bar (always displays full URL including ?token=...) */}
        <form
          onSubmit={(e) => void handleAddressSubmit(e)}
          className="flex min-w-0 flex-1 items-center"
        >
          <input
            value={addressInput}
            onChange={(e) => setAddressInput(e.target.value)}
            title={addressInput || resolvedUrl}
            placeholder={`留空回车自动检测/启动 ${hostName} 的 dsh web；或粘贴已有地址…`}
            className="h-6 min-w-0 flex-1 border border-line-subtle bg-canvas px-2 font-mono text-[11px] text-ink placeholder:text-ink-muted focus:border-line-strong focus:text-ink-strong focus:outline-none select-text"
          />
        </form>

        {/* Actions */}
        <div className="flex shrink-0 items-center gap-1">
          {reusedExisting && !dshToken && resolvedUrl && (
            <span
              title={`已自动复用 ${hostName} 上正在运行的 DSH WebUI 实例（避免多实例会话占用冲突）`}
              className="flex h-6 items-center border border-line-subtle bg-canvas px-2 text-[11px] text-success"
            >
              已复用已有实例
            </span>
          )}

          {resolvedUrl && (
            <button
              type="button"
              onClick={() => void copyValue(resolvedUrl, "url")}
              title={`复制完整地址（含 token）：${resolvedUrl}`}
              className="flex h-6 items-center gap-1 border border-line-subtle bg-canvas px-2 text-[11px] text-ink-2 hover:bg-hover hover:text-ink cursor-pointer"
            >
              {copiedKind === "url" ? (
                <Check className="h-3 w-3 text-success" />
              ) : (
                <Copy className="h-3 w-3" />
              )}
              <span>{copiedKind === "url" ? "已复制" : "复制地址"}</span>
            </button>
          )}

          {dshToken && (
            <button
              type="button"
              onClick={() => void copyValue(dshToken, "token")}
              title={`复制 Token：${dshToken}`}
              className="flex h-6 items-center gap-1 border border-line-subtle bg-canvas px-2 text-[11px] text-ink-2 hover:bg-hover hover:text-ink cursor-pointer"
            >
              {copiedKind === "token" ? (
                <Check className="h-3 w-3 text-success" />
              ) : (
                <KeyRound className="h-3 w-3" />
              )}
              <span>{copiedKind === "token" ? "已复制" : "复制 Token"}</span>
            </button>
          )}

          <button
            type="button"
            disabled={loading}
            onClick={() => void handleRestart()}
            title={`重启 ${hostName} 的 DSH WebUI 服务`}
            className="flex h-6 items-center gap-1 border border-line-subtle bg-canvas px-2 text-[11px] text-ink-2 hover:bg-hover hover:text-ink disabled:opacity-40 cursor-pointer"
          >
            <RotateCw className={`h-3 w-3 ${loading ? "animate-spin" : ""}`} />
            <span>重启</span>
          </button>

          {resolvedUrl && (
            <button
              type="button"
              onClick={() => void window.muxlane.openExternal(externalOpenUrl)}
              title="在系统浏览器中打开（含 token）"
              className="flex h-6 w-6 items-center justify-center border border-line-subtle bg-canvas text-ink-2 hover:bg-hover hover:text-ink cursor-pointer"
            >
              <ExternalLink className="h-3 w-3" />
            </button>
          )}
        </div>
      </header>

      {/* Main Full-Bleed DSH Viewport */}
      <div className="relative min-h-0 flex-1 bg-canvas">
        {loading ? (
          <div className="flex h-full flex-col items-center justify-center gap-2.5 text-ink-muted">
            <Loader2 className="h-5 w-5 animate-spin text-working" />
            <div className="text-[13px] text-ink">
              正在检测并连接 <span className="font-mono text-ink-strong">{hostName}</span> 的
              DeepSeek Harness WebUI…
            </div>
            <div className="text-[11.5px] text-ink-muted">
              {isRemoteHost
                ? "若远程主机已启动将直接复用；未启动则在远程主机自动启动并经 SSH 转发"
                : "若本机已启动将直接复用；未启动则自动执行 dsh web"}
            </div>
          </div>
        ) : activeError ? (
          <div className="flex h-full items-center justify-center p-6">
            <div className="w-full max-w-lg border border-line-strong bg-elevated p-4 space-y-3">
              <div className="text-[13px] font-semibold text-danger">
                无法连接 {hostName} 的 DeepSeek Harness WebUI
              </div>
              <div className="border-l-2 border-danger bg-danger-soft px-3 py-2 font-mono text-[11.5px] text-danger whitespace-pre-wrap break-words select-text">
                {activeError}
              </div>
              <div className="text-[11.5px] text-ink-muted">
                你可以点击重试，或在顶部地址栏直接粘贴已运行的 `dsh web` 地址（含 `?token=...`）后回车接入。
              </div>
              <div className="flex items-center gap-2 pt-1">
                <button
                  type="button"
                  onClick={() => void handleRestart()}
                  className="h-7 bg-primary-btn px-3 text-[12px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover cursor-pointer"
                >
                  重启 {hostName} DSH
                </button>
                <button
                  type="button"
                  onClick={onOpenRemotes}
                  className="h-7 border border-line bg-canvas px-3 text-[12px] text-ink hover:bg-hover cursor-pointer"
                >
                  切换机器
                </button>
              </div>
            </div>
          </div>
        ) : resolvedUrl ? (
          React.createElement("webview", {
            key: `${hostId}:${resolvedUrl}`,
            ref: webviewRef,
            src: resolvedUrl,
            partition: `persist:dsh-${hostId}`,
            allowpopups: "true",
            style: { width: "100%", height: "100%", display: "inline-flex", border: "none" },
          })
        ) : null}
      </div>
    </div>
  );
};
