import { useCallback, useEffect, useState } from "react";

export type DropPosition = "before" | "after";

export interface SidebarOrderState {
  projectOrder: string[];
  sessionOrder: Record<string, string[]>;
}

const STORAGE_PREFIX = "muxlane.sidebar.order.v1:";

export function applyOrder<T>(
  items: readonly T[],
  savedIds: readonly string[] | undefined,
  getId: (item: T) => string,
): T[] {
  if (!savedIds || savedIds.length === 0 || items.length <= 1) {
    return items.slice();
  }
  const rank = new Map<string, number>();
  savedIds.forEach((id, idx) => rank.set(id, idx));
  return items
    .map((item, originalIdx) => ({
      item,
      rank: rank.get(getId(item)) ?? Number.MAX_SAFE_INTEGER,
      originalIdx,
    }))
    .sort((a, b) => a.rank - b.rank || a.originalIdx - b.originalIdx)
    .map((x) => x.item);
}

export function reorderIds(
  currentIds: readonly string[],
  draggedId: string,
  targetId: string,
  position: DropPosition,
): string[] {
  if (!draggedId || !targetId || draggedId === targetId) {
    return currentIds.slice();
  }
  const filtered = currentIds.filter((id) => id !== draggedId);
  const targetIdx = filtered.indexOf(targetId);
  if (targetIdx < 0) {
    return currentIds.slice();
  }
  const insertIdx = position === "before" ? targetIdx : targetIdx + 1;
  filtered.splice(insertIdx, 0, draggedId);
  return filtered;
}

function loadOrder(hostKey: string): SidebarOrderState {
  try {
    const raw = window.localStorage.getItem(STORAGE_PREFIX + hostKey);
    if (!raw) return { projectOrder: [], sessionOrder: {} };
    const parsed = JSON.parse(raw) as Partial<SidebarOrderState>;
    return {
      projectOrder: Array.isArray(parsed.projectOrder) ? parsed.projectOrder : [],
      sessionOrder:
        parsed.sessionOrder && typeof parsed.sessionOrder === "object" ? parsed.sessionOrder : {},
    };
  } catch {
    return { projectOrder: [], sessionOrder: {} };
  }
}

function saveOrder(hostKey: string, state: SidebarOrderState): void {
  try {
    window.localStorage.setItem(STORAGE_PREFIX + hostKey, JSON.stringify(state));
  } catch {
    // ignore storage quota errors
  }
}

export function useSidebarOrder(hostKey: string) {
  const [order, setOrder] = useState<SidebarOrderState>(() => loadOrder(hostKey));

  useEffect(() => {
    setOrder(loadOrder(hostKey));
  }, [hostKey]);

  const reorderProjects = useCallback(
    (visibleProjectIds: string[], draggedId: string, targetId: string, position: DropPosition) => {
      setOrder((prev) => {
        const nextProjectOrder = reorderIds(visibleProjectIds, draggedId, targetId, position);
        const next: SidebarOrderState = {
          ...prev,
          projectOrder: nextProjectOrder,
        };
        saveOrder(hostKey, next);
        return next;
      });
    },
    [hostKey],
  );

  const reorderSessions = useCallback(
    (
      projectId: string,
      visibleSessionIds: string[],
      draggedId: string,
      targetId: string,
      position: DropPosition,
    ) => {
      setOrder((prev) => {
        const nextIds = reorderIds(visibleSessionIds, draggedId, targetId, position);
        const next: SidebarOrderState = {
          ...prev,
          sessionOrder: {
            ...prev.sessionOrder,
            [projectId]: nextIds,
          },
        };
        saveOrder(hostKey, next);
        return next;
      });
    },
    [hostKey],
  );

  return {
    projectOrder: order.projectOrder,
    sessionOrder: order.sessionOrder,
    reorderProjects,
    reorderSessions,
  };
}
