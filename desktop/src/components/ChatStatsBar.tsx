import React, { useEffect, useMemo, useRef, useState } from "react";
import { Clock, Database, Gauge, X } from "lucide-react";
import type { ChatSessionMeta, MessageUsage, SessionStats } from "../shared/protocol";
import {
  computeCacheHitPct,
  computeTps,
  formatCostUsd,
  formatDurationMs,
  formatTokensCompact,
  formatTokensExact,
} from "../shared/formatStats";

/**
 * Per-message token usage + duration inline badge & click-to-open DeepSeek Harness breakdown popover.
 * Rendered right after the model name below each completed AI message.
 */
export const MessageUsageFooter: React.FC<{
  usage?: MessageUsage | null;
  model?: string | null;
}> = ({ usage, model }) => {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  if (!usage) return null;
  const hasTokens = usage.total_tokens > 0;
  const hasDuration = usage.duration_ms != null && usage.duration_ms > 0;
  if (!hasTokens && !hasDuration) return null;

  const cacheHit = computeCacheHitPct(usage.input_tokens, usage.cache_read_tokens);
  const tps = computeTps(usage.output_tokens, usage.duration_ms);
  const costText = formatCostUsd(usage.cost_usd);

  return (
    <div ref={wrapRef} className="relative inline-flex items-center gap-1.5 font-mono text-[10.5px] text-ink-muted tabular-nums">
      {hasTokens && (
        <>
          <span className="text-ink-faint">·</span>
          <button
            type="button"
            onClick={() => setOpen((v) => !v)}
            title="查看本次 Token 明细"
            className={`inline-flex items-center gap-1 px-1 py-[1px] transition-colors cursor-pointer ${
              open
                ? "bg-hover text-ink-strong"
                : "text-ink-muted hover:bg-hover hover:text-ink"
            }`}
          >
            <span>{formatTokensCompact(usage.total_tokens)} tokens</span>
            {cacheHit != null && (
              <span className="text-ink-faint">({cacheHit}% 缓存)</span>
            )}
          </button>
        </>
      )}

      {hasDuration && (
        <>
          <span className="text-ink-faint">·</span>
          <span
            className="inline-flex items-center gap-1 text-ink-muted"
            title={
              usage.ttft_ms != null
                ? `总耗时 ${formatDurationMs(usage.duration_ms!)} · 首字 ${formatDurationMs(usage.ttft_ms)}`
                : `总耗时 ${formatDurationMs(usage.duration_ms!)}`
            }
          >
            <Clock className="h-2.5 w-2.5 text-ink-faint" />
            <span>{formatDurationMs(usage.duration_ms!)}</span>
            {tps && <span className="text-ink-faint">· {tps} tok/s</span>}
          </span>
        </>
      )}

      {open && hasTokens && (
        <div className="mux-popover-shadow absolute bottom-full left-0 z-30 mb-1.5 w-64 border border-line-strong bg-elevated text-[11px] text-ink">
          <div className="flex items-center justify-between border-b border-line-subtle bg-sidebar/60 px-2.5 py-1.5">
            <span className="font-sans text-[11px] font-medium text-ink-strong">
              本次回复统计
            </span>
            <button
              type="button"
              onClick={() => setOpen(false)}
              className="flex h-4 w-4 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink cursor-pointer"
              aria-label="关闭"
            >
              <X className="h-3 w-3" />
            </button>
          </div>
          <div className="space-y-1 px-2.5 py-2 font-mono text-[11px] tabular-nums">
            {model && (
              <StatRow label="模型" value={model} valueClass="truncate max-w-[140px]" />
            )}
            <StatRow label="总计" value={`${formatTokensExact(usage.total_tokens)} tok`} strong />
            {cacheHit != null && (
              <StatRow label="缓存命中" value={`${cacheHit}%`} valueClass="text-done" />
            )}
            <div className="my-1 border-t border-line-subtle" />
            <StatRow label="输入 (Input)" value={formatTokensExact(usage.input_tokens)} />
            {usage.cache_read_tokens > 0 && (
              <StatRow
                label="缓存读取"
                value={formatTokensExact(usage.cache_read_tokens)}
                valueClass="text-done"
              />
            )}
            {usage.cache_write_tokens > 0 && (
              <StatRow label="缓存写入" value={formatTokensExact(usage.cache_write_tokens)} />
            )}
            <StatRow label="输出 (Output)" value={formatTokensExact(usage.output_tokens)} />
            {usage.reasoning_tokens > 0 && (
              <StatRow label="思考 (Reasoning)" value={formatTokensExact(usage.reasoning_tokens)} />
            )}
            {(hasDuration || usage.ttft_ms != null || tps || costText) && (
              <>
                <div className="my-1 border-t border-line-subtle" />
                {hasDuration && (
                  <StatRow label="耗时" value={formatDurationMs(usage.duration_ms!)} />
                )}
                {usage.ttft_ms != null && (
                  <StatRow label="首字延迟 (TTFT)" value={formatDurationMs(usage.ttft_ms)} />
                )}
                {tps && <StatRow label="输出速度" value={`${tps} tok/s`} />}
                {costText && <StatRow label="估算费用" value={costText} />}
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

/**
 * DeepSeek Harness-style Session Stats Bar docked below the composer.
 * Shows clickable Activity & Usage rectangular badges with detail dialogs + context occupancy meter.
 */
export const SessionStatsStrip: React.FC<{
  meta: ChatSessionMeta;
  fallbackStats?: SessionStats | null;
}> = ({ meta, fallbackStats }) => {
  const [dialog, setDialog] = useState<"activity" | "usage" | null>(null);

  const stats: SessionStats | null = useMemo(() => {
    if (meta.stats && (meta.stats.turns > 0 || meta.stats.steps > 0 || meta.stats.total_tokens > 0)) {
      return meta.stats;
    }
    if (
      fallbackStats &&
      (fallbackStats.turns > 0 || fallbackStats.steps > 0 || fallbackStats.total_tokens > 0)
    ) {
      return fallbackStats;
    }
    return null;
  }, [meta.stats, fallbackStats]);

  useEffect(() => {
    if (!dialog) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setDialog(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [dialog]);

  const contextPct =
    meta.context_used_tokens != null &&
    meta.context_window_tokens != null &&
    meta.context_window_tokens > 0
      ? Math.min(100, Math.round((meta.context_used_tokens / meta.context_window_tokens) * 100))
      : null;

  if (!stats && meta.context_used_tokens == null && !meta.total_cost_usd) {
    return null;
  }

  const turns = stats?.turns ?? 0;
  const steps = stats?.steps ?? 0;
  const totalTokens = stats?.total_tokens ?? 0;
  const cacheHit = stats ? computeCacheHitPct(stats.input_tokens, stats.cache_read_tokens) : null;
  const avgTps = stats ? computeTps(stats.output_tokens, stats.llm_ms) : null;
  const totalActiveMs = stats ? stats.llm_ms + stats.tool_ms : 0;
  const avgTtftMs =
    stats && stats.ttft_steps > 0 ? Math.round(stats.ttft_ms / stats.ttft_steps) : null;
  const totalCostText = formatCostUsd(stats?.total_cost_usd ?? meta.total_cost_usd);

  return (
    <>
      <div className="mt-1.5 flex flex-wrap items-center justify-between gap-2 px-0.5 text-[10.5px] text-ink-muted select-none">
        {/* Left cluster: DeepSeek Harness Activity + Usage square badges */}
        <div className="flex flex-wrap items-center gap-1.5 font-mono tabular-nums">
          {stats && (turns > 0 || steps > 0) && (
            <button
              type="button"
              onClick={() => setDialog("activity")}
              title="查看会话活动与耗时统计"
              className="inline-flex h-5 items-center gap-1.5 border border-line-subtle bg-elevated px-2 text-ink-2 hover:border-line-strong hover:bg-hover hover:text-ink transition-colors cursor-pointer"
            >
              <Gauge className="h-3 w-3 text-ink-muted" />
              <span>
                {turns} 轮 {steps} 步
              </span>
              {avgTps ? (
                <>
                  <span className="text-ink-faint">·</span>
                  <span>{avgTps} tok/s</span>
                </>
              ) : totalActiveMs > 0 ? (
                <>
                  <span className="text-ink-faint">·</span>
                  <span>{formatDurationMs(totalActiveMs)}</span>
                </>
              ) : null}
            </button>
          )}

          {stats && totalTokens > 0 && (
            <button
              type="button"
              onClick={() => setDialog("usage")}
              title="查看会话 Token 用量明细"
              className="inline-flex h-5 items-center gap-1.5 border border-line-subtle bg-elevated px-2 text-ink-2 hover:border-line-strong hover:bg-hover hover:text-ink transition-colors cursor-pointer"
            >
              <Database className="h-3 w-3 text-ink-muted" />
              <span>总计 {formatTokensCompact(totalTokens)} tok</span>
              {cacheHit != null && (
                <>
                  <span className="text-ink-faint">·</span>
                  <span className="text-done">缓存 {cacheHit}%</span>
                </>
              )}
              {totalCostText && (
                <>
                  <span className="text-ink-faint">·</span>
                  <span>{totalCostText}</span>
                </>
              )}
            </button>
          )}
        </div>

        {/* Right cluster: Context occupancy meter */}
        {meta.context_used_tokens != null && (
          <div className="flex items-center gap-2 font-mono text-[10.5px] text-ink-muted tabular-nums">
            <span>
              上下文 {formatTokensCompact(meta.context_used_tokens)}
              {meta.context_window_tokens
                ? ` / ${formatTokensCompact(meta.context_window_tokens)}`
                : ""}
              {contextPct != null ? ` (${contextPct}%)` : ""}
            </span>
            {contextPct != null && (
              <div className="h-[4px] w-14 bg-hover overflow-hidden">
                <div
                  className={`h-full ${
                    contextPct > 85
                      ? "bg-danger"
                      : contextPct > 65
                        ? "bg-working"
                        : "bg-ink-2"
                  }`}
                  style={{ width: `${contextPct}%` }}
                />
              </div>
            )}
          </div>
        )}
      </div>

      {/* DeepSeek Harness Activity / Usage Detail Dialog (strict 0px square) */}
      {dialog && stats && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/45 p-4"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) setDialog(null);
          }}
        >
          <div className="mux-popover-shadow w-full max-w-sm border border-line-strong bg-elevated text-ink">
            <div className="flex items-center justify-between border-b border-line-subtle bg-sidebar/60 px-3.5 py-2.5">
              <div className="flex items-center gap-2">
                {dialog === "activity" ? (
                  <Gauge className="h-3.5 w-3.5 text-ink-2" />
                ) : (
                  <Database className="h-3.5 w-3.5 text-ink-2" />
                )}
                <span className="text-[12.5px] font-medium text-ink-strong">
                  {dialog === "activity" ? "会话活动统计" : "会话 Token 用量"}
                </span>
              </div>
              <button
                type="button"
                onClick={() => setDialog(null)}
                className="flex h-6 w-6 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink cursor-pointer"
                aria-label="关闭"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>

            {dialog === "activity" ? (
              <div className="space-y-2 px-3.5 py-3 font-mono text-[12px] tabular-nums">
                <StatRow label="对话轮次 (Turns)" value={String(stats.turns)} strong />
                <StatRow label="模型推理步数 (Steps)" value={String(stats.steps)} />
                <div className="my-1.5 border-t border-line-subtle" />
                <StatRow label="模型推理用时" value={formatDurationMs(stats.llm_ms)} />
                <StatRow
                  label={`工具调用用时 (${stats.tool_calls} 次)`}
                  value={formatDurationMs(stats.tool_ms)}
                />
                <StatRow label="总活跃耗时" value={formatDurationMs(totalActiveMs)} strong />
                <div className="my-1.5 border-t border-line-subtle" />
                <StatRow
                  label="首字平均延迟 (TTFT)"
                  value={avgTtftMs != null ? formatDurationMs(avgTtftMs) : "—"}
                />
                <StatRow
                  label="平均输出速度 (TPS)"
                  value={avgTps ? `${avgTps} tok/s` : "—"}
                />
              </div>
            ) : (
              <div className="space-y-2 px-3.5 py-3 font-mono text-[12px] tabular-nums">
                <StatRow
                  label="会话总计"
                  value={`${formatTokensExact(stats.total_tokens)} tok`}
                  strong
                />
                {cacheHit != null && (
                  <div className="space-y-1">
                    <StatRow label="缓存命中率" value={`${cacheHit}%`} valueClass="text-done" />
                    <div className="h-[4px] w-full bg-hover overflow-hidden">
                      <div className="h-full bg-done" style={{ width: `${cacheHit}%` }} />
                    </div>
                  </div>
                )}
                <div className="my-1.5 border-t border-line-subtle" />
                <StatRow label="输入 (Input)" value={formatTokensExact(stats.input_tokens)} />
                <StatRow
                  label="缓存读取 (Cache Read)"
                  value={formatTokensExact(stats.cache_read_tokens)}
                  valueClass={stats.cache_read_tokens > 0 ? "text-done" : undefined}
                />
                <StatRow
                  label="缓存写入 (Cache Write)"
                  value={formatTokensExact(stats.cache_write_tokens)}
                />
                <StatRow label="输出 (Output)" value={formatTokensExact(stats.output_tokens)} />
                {stats.reasoning_tokens > 0 && (
                  <StatRow
                    label="思考 (Reasoning)"
                    value={formatTokensExact(stats.reasoning_tokens)}
                  />
                )}
                {totalCostText && (
                  <>
                    <div className="my-1.5 border-t border-line-subtle" />
                    <StatRow label="累计估算费用" value={totalCostText} strong />
                  </>
                )}
              </div>
            )}
          </div>
        </div>
      )}
    </>
  );
};

const StatRow: React.FC<{
  label: string;
  value: string;
  strong?: boolean;
  valueClass?: string;
}> = ({ label, value, strong, valueClass }) => (
  <div className="flex items-center justify-between gap-3">
    <span className="font-sans text-[11px] text-ink-muted">{label}</span>
    <span
      className={`${strong ? "font-semibold text-ink-strong" : "text-ink"} ${valueClass ?? ""}`}
    >
      {value}
    </span>
  </div>
);
