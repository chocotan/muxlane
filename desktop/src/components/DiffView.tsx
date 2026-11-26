import React, { useMemo, useState } from "react";
import { ChevronDown, ChevronRight, FileDiff } from "lucide-react";
import type { ToolDiff } from "../shared/protocol";
import { computeDiffRows } from "../shared/timeline";

export const DiffView: React.FC<{ diff: ToolDiff; defaultExpanded?: boolean }> = ({
  diff,
  defaultExpanded = true,
}) => {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const { rows, additions, deletions } = useMemo(
    () => computeDiffRows(diff.old_text, diff.new_text),
    [diff.new_text, diff.old_text],
  );
  const truncated = rows.length > 260;
  const visibleRows = truncated ? rows.slice(0, 260) : rows;

  return (
    <div className="mt-2 border border-line-subtle bg-code-surface select-text">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className={`flex h-8 w-full items-center justify-between bg-code-header px-2.5 text-left font-mono text-[11.5px] hover:bg-hover/60 transition-colors cursor-pointer select-none ${
          expanded ? "border-b border-line-subtle" : ""
        }`}
      >
        <div className="flex min-w-0 items-center gap-2">
          {expanded ? (
            <ChevronDown className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
          ) : (
            <ChevronRight className="h-3.5 w-3.5 shrink-0 text-ink-muted" />
          )}
          <FileDiff className="h-3.5 w-3.5 shrink-0 text-ink-2" strokeWidth={1.7} />
          <span className="truncate text-ink">{diff.path || "变更差异"}</span>
        </div>
        <div className="flex shrink-0 items-center gap-2 font-mono text-[11px] tabular-nums">
          <span style={{ color: "var(--diff-add-fg)" }}>+{additions}</span>
          <span style={{ color: "var(--diff-del-fg)" }}>-{deletions}</span>
        </div>
      </button>

      {expanded && (
        <div className="max-h-[380px] overflow-auto font-mono text-[12px] leading-[1.55]">
          <table className="w-full border-collapse">
            <tbody>
              {visibleRows.map((row, idx) => {
                const isAdd = row.type === "add";
                const isDel = row.type === "del";
                const rowStyle: React.CSSProperties | undefined = isAdd
                  ? { backgroundColor: "var(--diff-add-bg)" }
                  : isDel
                    ? { backgroundColor: "var(--diff-del-bg)" }
                    : undefined;
                const gutterStyle: React.CSSProperties = isAdd
                  ? {
                      backgroundColor: "var(--diff-add-gutter)",
                      color: "var(--diff-add-fg)",
                    }
                  : isDel
                    ? {
                        backgroundColor: "var(--diff-del-gutter)",
                        color: "var(--diff-del-fg)",
                      }
                    : { color: "var(--ink-faint)" };
                const textStyle: React.CSSProperties = isAdd
                  ? { color: "var(--diff-add-fg)" }
                  : isDel
                    ? { color: "var(--diff-del-fg)" }
                    : { color: "var(--ink-secondary)" };
                const marker = isAdd ? "+" : isDel ? "-" : " ";

                return (
                  <tr key={idx} style={rowStyle}>
                    <td
                      style={gutterStyle}
                      className="w-9 select-none border-r border-line-subtle px-1.5 text-right text-[11px] tabular-nums"
                    >
                      {row.oldLine ?? ""}
                    </td>
                    <td
                      style={gutterStyle}
                      className="w-9 select-none border-r border-line-subtle px-1.5 text-right text-[11px] tabular-nums"
                    >
                      {row.newLine ?? ""}
                    </td>
                    <td
                      style={gutterStyle}
                      className="w-5 select-none text-center font-semibold"
                    >
                      {marker}
                    </td>
                    <td style={textStyle} className="whitespace-pre px-2.5 py-[1px]">
                      {row.text}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          {truncated && (
            <div className="border-t border-line-subtle bg-code-header px-3 py-1 text-[11px] text-ink-muted">
              已截断显示前 260 行（共 {rows.length} 行）
            </div>
          )}
        </div>
      )}
    </div>
  );
};
