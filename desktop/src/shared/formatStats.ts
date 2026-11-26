export function formatTokensCompact(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 100_000) return `${Math.round(n / 1_000)}K`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(Math.round(n));
}

export function formatTokensExact(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "0";
  return Math.round(n).toLocaleString("en-US");
}

export function formatDurationMs(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "0ms";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const secs = ms / 1000;
  if (secs < 60) return `${secs.toFixed(1)}s`;
  const mins = Math.floor(secs / 60);
  const remSecs = Math.round(secs % 60);
  return `${mins}m ${remSecs}s`;
}

export function computeCacheHitPct(inputTokens: number, cacheReadTokens: number): number | null {
  const denom = Math.max(0, inputTokens) + Math.max(0, cacheReadTokens);
  if (denom <= 0 || cacheReadTokens <= 0) return null;
  return Math.round((cacheReadTokens / denom) * 100);
}

export function computeTps(outputTokens: number, durationMs: number | null | undefined): string | null {
  if (!durationMs || durationMs <= 0 || outputTokens <= 0) return null;
  const tps = outputTokens / (durationMs / 1000);
  if (!Number.isFinite(tps) || tps <= 0) return null;
  return tps >= 100 ? Math.round(tps).toString() : tps.toFixed(1);
}

export function formatCostUsd(cost: number | null | undefined): string | null {
  if (cost == null || !Number.isFinite(cost) || cost <= 0) return null;
  if (cost < 0.001) return `$${cost.toFixed(4)}`;
  if (cost < 0.1) return `$${cost.toFixed(3)}`;
  return `$${cost.toFixed(2)}`;
}

export function formatBytes(bytes: number | null | undefined): string | null {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return null;
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}
