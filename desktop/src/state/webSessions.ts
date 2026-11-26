export type WebSessionKind = "dsh" | "web";

export const MACHINE_DEFAULT_WORKSPACE_ID = "__machine_default__";

export interface WebSession {
  id: string;
  projectId: string;
  kind: WebSessionKind;
  title: string;
  titleIsCustom?: boolean;
  url: string;
  createdAt: number;
}

const STORAGE_PREFIX = "muxlane.web.sessions.v1:";

export function isWebSessionId(id: string | null | undefined): boolean {
  return Boolean(id && id.startsWith("web_"));
}

export function createWebSessionId(): string {
  const rand = Math.random().toString(36).slice(2, 8);
  return `web_${Date.now().toString(36)}_${rand}`;
}

/**
 * Normalize user-entered webpage URL:
 * - Preserves explicit http:// or https://
 * - Uses http:// for localhost, loopback, IPv4 literals, or explicit host:port
 * - Uses https:// for domain names
 */
export function normalizeWebUrl(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) return "";
  if (/^https?:\/\//i.test(trimmed)) return trimmed;
  if (/^(localhost|127\.0\.0\.1|\[::1\]|\d{1,3}(?:\.\d{1,3}){3})(?::\d+)?(?:\/.*)?$/i.test(trimmed)) {
    return `http://${trimmed}`;
  }
  if (/^[a-zA-Z0-9.-]+:\d+(?:\/.*)?$/.test(trimmed)) {
    return `http://${trimmed}`;
  }
  if (/^[a-zA-Z0-9-]+(?:\.[a-zA-Z0-9-]+)+(?:\/.*)?$/.test(trimmed)) {
    return `https://${trimmed}`;
  }
  throw new Error("请输入有效的网页地址，例如 https://example.com 或 http://localhost:3000");
}

export function deriveTitleFromUrl(url: string, kind: WebSessionKind): string {
  if (kind === "dsh") return "DeepSeek Harness";
  try {
    const parsed = new URL(url);
    return parsed.host || url;
  } catch {
    return url || "网页";
  }
}

export function loadWebSessions(hostId: string): WebSession[] {
  try {
    const raw = window.localStorage.getItem(STORAGE_PREFIX + hostId);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (item): item is WebSession =>
        Boolean(item) &&
        typeof item.id === "string" &&
        typeof item.projectId === "string" &&
        (item.kind === "dsh" || item.kind === "web") &&
        typeof item.title === "string" &&
        typeof item.url === "string",
    );
  } catch {
    return [];
  }
}

export function saveWebSessions(hostId: string, sessions: readonly WebSession[]): void {
  try {
    window.localStorage.setItem(STORAGE_PREFIX + hostId, JSON.stringify(sessions));
  } catch {
    // ignore storage errors
  }
}
