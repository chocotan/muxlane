export interface WebSession {
  id: string;
  projectId: string;
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

export function deriveTitleFromUrl(url: string): string {
  try {
    const parsed = new URL(url);
    return parsed.host || url;
  } catch {
    return url || "网页";
  }
}

export function extractDshToken(url: string): string | null {
  try {
    const parsed = new URL(url);
    const token = parsed.searchParams.get("token")?.trim();
    return token || null;
  } catch {
    return null;
  }
}

/**
 * Keep the DSH `?token=...` query parameter visible after `dsh web` performs its
 * post-login 303 redirect to `./`, without leaking the token to unrelated origins.
 */
export function preserveDshToken(navigatedUrl: string, resolvedUrl: string): string {
  if (!navigatedUrl || !resolvedUrl) return navigatedUrl || resolvedUrl;
  try {
    const nav = new URL(navigatedUrl);
    const base = new URL(resolvedUrl);
    const token = base.searchParams.get("token");
    if (token && nav.origin === base.origin && !nav.searchParams.has("token")) {
      nav.searchParams.set("token", token);
      return nav.toString();
    }
    return navigatedUrl;
  } catch {
    return navigatedUrl;
  }
}

export function loadWebSessions(hostId: string): WebSession[] {
  try {
    const raw = window.localStorage.getItem(STORAGE_PREFIX + hostId);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter(
        (item): item is WebSession & { kind?: string } =>
          Boolean(item) &&
          typeof item.id === "string" &&
          typeof item.projectId === "string" &&
          item.projectId !== "__machine_default__" &&
          item.kind !== "dsh" &&
          typeof item.title === "string" &&
          typeof item.url === "string" &&
          item.url.trim().length > 0,
      )
      .map(({ id, projectId, title, titleIsCustom, url, createdAt }) => ({
        id,
        projectId,
        title,
        titleIsCustom,
        url,
        createdAt: typeof createdAt === "number" ? createdAt : Date.now(),
      }));
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
