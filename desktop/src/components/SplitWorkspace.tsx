import React, { useRef, useState } from "react";

interface SplitWorkspaceProps {
  left: React.ReactNode;
  right: React.ReactNode;
}

/** Orca-style thin 1px seam with wide invisible pointer grab reach. */
export const SplitWorkspace: React.FC<SplitWorkspaceProps> = ({ left, right }) => {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [ratio, setRatio] = useState(0.55);
  const [dragging, setDragging] = useState(false);

  const startDrag = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    const container = containerRef.current;
    if (!container) return;
    const rect = container.getBoundingClientRect();
    setDragging(true);

    const onMove = (ev: PointerEvent) => {
      const next = (ev.clientX - rect.left) / rect.width;
      setRatio(Math.min(0.78, Math.max(0.24, next)));
    };
    const onUp = () => {
      setDragging(false);
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  return (
    <div ref={containerRef} className="flex h-full min-h-0 w-full">
      <div className="h-full min-w-0" style={{ width: `${ratio * 100}%` }}>
        {left}
      </div>
      <div
        onPointerDown={startDrag}
        title="拖拽调整对话与终端分屏比例"
        className="group relative z-10 w-[1px] shrink-0 cursor-col-resize bg-line-subtle"
      >
        {/* Invisible wide hit region reaching into adjacent pane */}
        <div className="absolute inset-y-0 -left-[3px] -right-[4px]" />
        {/* Highlighted hairline on hover or active drag */}
        <div
          className={`pointer-events-none absolute inset-y-0 left-0 w-[1px] transition-colors ${
            dragging
              ? "bg-ink-2"
              : "bg-transparent group-hover:bg-ink-muted"
          }`}
        />
      </div>
      <div className="h-full min-w-0 flex-1">{right}</div>
    </div>
  );
};
