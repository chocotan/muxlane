import { describe, expect, it } from "vitest";
import { applyOrder, reorderIds } from "./useSidebarOrder";

describe("sidebar drag-and-drop ordering helpers", () => {
  it("applies saved order and appends unseen items in original order", () => {
    const items = [
      { id: "a" },
      { id: "b" },
      { id: "c" },
      { id: "d" },
    ];
    const ordered = applyOrder(items, ["c", "a", "ghost"], (x) => x.id);
    expect(ordered.map((x) => x.id)).toEqual(["c", "a", "b", "d"]);
  });

  it("returns original order when saved order is empty or missing", () => {
    const items = [{ id: "a" }, { id: "b" }];
    expect(applyOrder(items, undefined, (x) => x.id).map((x) => x.id)).toEqual(["a", "b"]);
    expect(applyOrder(items, [], (x) => x.id).map((x) => x.id)).toEqual(["a", "b"]);
  });

  it("reorders dragged id before or after target id", () => {
    expect(reorderIds(["a", "b", "c", "d"], "d", "b", "before")).toEqual(["a", "d", "b", "c"]);
    expect(reorderIds(["a", "b", "c", "d"], "a", "c", "after")).toEqual(["b", "c", "a", "d"]);
    expect(reorderIds(["a", "b", "c"], "b", "b", "before")).toEqual(["a", "b", "c"]);
    expect(reorderIds(["a", "b", "c"], "x", "b", "before")).toEqual(["a", "x", "b", "c"]);
    expect(reorderIds(["a", "b", "c"], "a", "missing", "after")).toEqual(["a", "b", "c"]);
  });
});
