import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  ExternalLink,
  Globe,
  Loader2,
  RefreshCw,
  RotateCw,
  Server,
} from "lucide-react";
import {
  deriveTitleFromUrl,
  normalizeWebUrl,
  type WebSession,
} from "../state/webSessions";

interface WebviewElement extends HTMLElement {
  src: string;
  reload: () => void;
  goBack: () => void;
  goForward: () => void;
  canGoBack: () => boolean;
  canGoForward: () => boolean;
  loadURL: (url: string) => Promise<void>;
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

interface WebviewTitleEvent extends Event {
  title: string;
}

interface WebBrowserPaneProps {
  session: WebSession;
  projectName: string;
  projectPath: string;
  hostId: string;
  hostName: string;
  isRemoteHost: boolean;
  visible: boolean;
  onUpdateSession: (patch: Partial<Pick<WebSession, "url" | "title" | "titleIsCustom">>) => void;
}

function maskToken(url: string): string {
  return url.replace(/([?&]token=)[^&]+/i, "$1***");
}

export const WebBrowserPane: React.FC<WebBrowserPaneProps> = ({
  session,
  projectName,
  projectPath,
  hostId,
  hostName,
  isRemoteHost,
  visible,
  onUpdateSession,
}) => {
  const webviewRef = useRef<WebviewElement | null>(null);
  const [resolvedUrl, setResolvedUrl] = useState<string>(session.url);
  const [addressInput, setAddressInput] = useState<string>(session.url);
  const [addressFocused, setAddressFocused] = useState(false);
  const [loading, setLoading] = useState<boolean>(session.kind === "dsh" && !session.url);
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
        const res = await window.muxlane.openDshWeb(session.projectId, projectPath, customUrl);
        setResolvedUrl(res.url);
        setAddressInput(res.url);
      } catch (err) {
        setError((err as Error).message || "启动 DeepSeek Harness WebUI 失败");
      } finally {
        setLoading(false);
      }
    },
    [projectPath, session.projectId],
  );

  useEffect(() => {
    if (session.kind === "dsh") {
      void bootDsh(session.url || undefined);
    } else {
      setResolvedUrl(session.url);
      setAddressInput(session.url);
      setLoading(false);
      setError(null);
    }
    // Boot once on mount or when host changes for this session
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session.id, hostId]);

  useEffect(() => {
    const wv = webviewRef.current;
    if (!wv || !resolvedUrl) return;

    const syncNavButtons = () => {
      try {
        setCanBack(wv.canGoBack());
        setCanForward(wv.canGoForward());
      } catch {
        // webview not yet ready
      }
    };

    const onNavigate = (ev: Event) => {
      const e = ev as WebviewNavigateEvent;
      if (e.url) {
        setAddressInput(e.url);
        if (session.kind === "web" && e.url !== session.url) {
          onUpdateSession({ url: e.url });
        }
      }
      syncNavButtons();
    };

    const onTitleUpdated = (ev: Event) => {
      const e = ev as WebviewTitleEvent;
      if (session.kind === "web" && !session.titleIsCustom && e.title?.trim()) {
        onUpdateSession({ title: e.title.trim().slice(0, 48) });
      }
    };

    const onFailLoad = (ev: Event) => {
      const e = ev as WebviewFailLoadEvent;
      // -3 is ERR_ABORTED (normal during HTTP 303 redirect from /?token=... to ./)
      if (e.errorCode === -3 || e.isMainFrame === false) return;
      setWebviewError(
        `页面加载失败 (${e.errorCode} ${e.errorDescription || "ERR"}): ${e.validatedURL || resolvedUrl}`,
      );
    };

    const onFinishLoad = () => {
      setWebviewError(null);
      syncNavButtons();
    };

    const onCrashed = () => {
      setWebviewError("浏览器页面渲染进程异常退出，请点击刷新重试。");
    };

    wv.addEventListener("did-navigate", onNavigate);
    wv.addEventListener("did-navigate-in-page", onNavigate);
    wv.addEventListener("page-title-updated", onTitleUpdated);
    wv.addEventListener("did-fail-load", onFailLoad);
    wv.addEventListener("did-finish-load", onFinishLoad);
    wv.addEventListener("crashed", onCrashed);
    wv.addEventListener("render-process-gone", onCrashed);
    return () => {
      wv.removeEventListener("did-navigate", onNavigate);
      wv.removeEventListener("did-navigate-in-page", onNavigate);
      wv.removeEventListener("page-title-updated", onTitleUpdated);
      wv.removeEventListener("did-fail-load", onFailLoad);
      wv.removeEventListener("did-finish-load", onFinishLoad);
      wv.removeEventListener("crashed", onCrashed);
      wv.removeEventListener("render-process-gone", onCrashed);
    };
  }, [onUpdateSession, resolvedUrl, session.kind, session.titleIsCustom, session.url]);

  const handleRestartDsh = async () => {
    setLoading(true);
    setError(null);
    setWebviewError(null);
    try {
      const res = await window.muxlane.restartDshWeb(session.projectId, projectPath);
      setResolvedUrl(res.url);
      setAddressInput(res.url);
      if (session.url) {
        onUpdateSession({ url: "" });
      }
    } catch (err) {
      setError((err as Error).message || "重启 DeepSeek Harness WebUI 失败");
    } finally {
      setLoading(false);
    }
  };

  const handleNavigateSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setWebviewError(null);
    try {
      const normalized = normalizeWebUrl(addressInput);
      if (!normalized) {
        if (session.kind === "dsh") {
          onUpdateSession({ url: "" });
          await bootDsh(undefined);
        }
        return;
      }
      setAddressInput(normalized);
      if (session.kind === "dsh") {
        onUpdateSession({ url: normalized });
        await bootDsh(normalized);
      } else {
        setResolvedUrl(normalized);
        onUpdateSession({
          url: normalized,
          ...(session.titleIsCustom ? {} : { title: deriveTitleFromUrl(normalized, "web") }),
        });
        const wv = webviewRef.current;
        if (wv && typeof wv.loadURL === "function") {
          try {
            await wv.loadURL(normalized);
          } catch {
            // fallback handled by src/key
          }
        }
      }
    } catch (err) {
      setError((err as Error).message || "无效的网页地址");
    }
  };

  const activeError = error || webviewError;
  const shownAddress = addressFocused ? addressInput : maskToken(addressInput);
  const partition = session.kind === "dsh" ? `persist:dsh-${hostId}` : "persist:web-browser";

  return (
    <div
      className={`absolute inset-0 flex flex-col bg-canvas ${
        visible ? "z-10 visible" : "z-0 invisible pointer-events-none"
      }`}
    >
      {/* Compact 32px Browser Navigation Bar (0px square corners) */}
      <div className="flex h-8 shrink-0 items-center gap-1.5 border-b border-line-subtle bg-sidebar/70 px-2 text-[11px] text-ink-muted select-none">
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

        {/* Editable URL Bar */}
        <form onSubmit={(e) => void handleNavigateSubmit(e)} className="flex min-w-0 flex-1 items-center">
          <div className="flex h-6 min-w-0 flex-1 items-center gap-1.5 border border-line-subtle bg-canvas px-2 focus-within:border-line-strong">
            <Globe
              className={`h-3 w-3 shrink-0 ${
                session.kind === "dsh" ? "text-working" : "text-ink-muted"
              }`}
              strokeWidth={1.8}
            />
            <input
              value={shownAddress}
              onFocus={() => setAddressFocused(true)}
              onBlur={() => setAddressFocused(false)}
              onChange={(e) => setAddressInput(e.target.value)}
              placeholder={
                session.kind === "dsh"
                  ? "留空回车自动连接当前机器 DSH；或输入自定义地址…"
                  : "输入任意网址后回车，例如 https://github.com 或 http://localhost:3000"
              }
              className="min-w-0 flex-1 bg-transparent font-mono text-[11px] text-ink focus:text-ink-strong focus:outline-none select-text"
            />
          </div>
        </form>

        {/* Right Controls */}
        <div className="flex shrink-0 items-center gap-1">
          {session.kind === "dsh" && (
            <>
              <span
                title={
                  isRemoteHost
                    ? `运行在远程主机 ${hostName}（经 SSH 端口转发）`
                    : `运行在${hostName}`
                }
                className="inline-flex h-6 items-center gap-1 border border-line-subtle bg-canvas px-1.5 font-mono text-[10px] text-ink-2"
              >
                <Server
                  className={`h-3 w-3 ${isRemoteHost ? "text-success" : "text-ink-muted"}`}
                  strokeWidth={1.75}
                />
                <span>{hostName}</span>
              </span>
              <button
                type="button"
                disabled={loading}
                onClick={() => void handleRestartDsh()}
                title={`重启 ${hostName} 的 DSH WebUI 服务`}
                className="flex h-6 w-6 items-center justify-center border border-line-subtle bg-canvas text-ink-2 hover:bg-hover hover:text-ink disabled:opacity-40 cursor-pointer"
              >
                <RotateCw className={`h-3 w-3 ${loading ? "animate-spin" : ""}`} />
              </button>
            </>
          )}

          {resolvedUrl && (
            <button
              type="button"
              onClick={() => void window.muxlane.openExternal(addressInput || resolvedUrl)}
              title="在系统浏览器中打开"
              className="flex h-6 w-6 items-center justify-center border border-line-subtle bg-canvas text-ink-2 hover:bg-hover hover:text-ink cursor-pointer"
            >
              <ExternalLink className="h-3 w-3" />
            </button>
          )}
        </div>
      </div>

      {/* Main Content */}
      <div className="relative min-h-0 flex-1 bg-canvas">
        {loading ? (
          <div className="flex h-full flex-col items-center justify-center gap-2.5 text-ink-muted">
            <Loader2 className="h-5 w-5 animate-spin text-working" />
            <div className="text-[12.5px] text-ink">
              正在检测并连接 <span className="font-mono text-ink-strong">{hostName}</span> 的
              DeepSeek Harness WebUI…
            </div>
            <div className="font-mono text-[11px] text-ink-muted">
              {projectName} · {projectPath}
            </div>
          </div>
        ) : activeError ? (
          <div className="flex h-full items-center justify-center p-6">
            <div className="w-full max-w-lg border border-line-strong bg-elevated p-4 space-y-3">
              <div className="text-[13px] font-semibold text-danger">
                {session.kind === "dsh" ? "DeepSeek Harness WebUI 加载失败" : "网页加载失败"}
              </div>
              <div className="border-l-2 border-danger bg-danger-soft px-3 py-2 font-mono text-[11.5px] text-danger whitespace-pre-wrap break-words select-text">
                {activeError}
              </div>
              <div className="text-[11.5px] text-ink-muted">
                {session.kind === "dsh"
                  ? `你可以点击重试，或在上方地址栏直接输入 ${hostName} 已运行的 dsh web URL 后回车。`
                  : "请检查网址是否正确或目标服务是否已启动，修改上方地址栏后按回车重试。"}
              </div>
              <div className="flex items-center gap-2 pt-1">
                {session.kind === "dsh" ? (
                  <button
                    type="button"
                    onClick={() => void handleRestartDsh()}
                    className="h-7 bg-primary-btn px-3 text-[12px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover cursor-pointer"
                  >
                    重启 {hostName} DSH
                  </button>
                ) : (
                  <button
                    type="button"
                    onClick={() => {
                      setError(null);
                      setWebviewError(null);
                      webviewRef.current?.reload();
                    }}
                    className="h-7 bg-primary-btn px-3 text-[12px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover cursor-pointer"
                  >
                    重新加载
                  </button>
                )}
              </div>
            </div>
          </div>
        ) : resolvedUrl ? (
          React.createElement("webview", {
            key: `${hostId}:${resolvedUrl}`,
            ref: webviewRef,
            src: resolvedUrl,
            partition,
            allowpopups: "true",
            style: { width: "100%", height: "100%", display: "inline-flex", border: "none" },
          })
        ) : null}
      </div>
    </div>
  );
};
