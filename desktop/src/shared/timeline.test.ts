import { describe, expect, it } from "vitest";
import type { ChatHistoryResult, TimelineEntry } from "./protocol";
import {
  applyChatDelta,
  buildDisplayItems,
  computeDiffRows,
  defaultChatMeta,
  orderPresetsPiFirst,
  summarizeToolInput,
  timelineFromHistory,
} from "./timeline";

describe("timeline reducer & display helpers", () => {
  it("applies deltas idempotently, streams text, and handles remove_entry", () => {
    const history: ChatHistoryResult = {
      agent: "pi_1",
      seq: 2,
      meta: { ...defaultChatMeta(), driver: "terminal_pty", supports_chat_input: true, has_pty: true },
      entries: [
        {
          type: "user_message",
          id: "__bridge_pending_user__:1",
          text: "hello",
          attachments: [],
          created_at_ms: 100,
        },
        {
          type: "assistant_text",
          id: "a1",
          text: "Checking",
          streaming: true,
          created_at_ms: 101,
        },
      ],
      pending_approvals: [],
    };

    let state = timelineFromHistory(history);
    // Stale seq must be ignored
    state = applyChatDelta(state, {
      agent: "pi_1",
      seq: 2,
      delta: { op: "reset" },
    });
    expect(state.entries).toHaveLength(2);

    // Append text
    state = applyChatDelta(state, {
      agent: "pi_1",
      seq: 3,
      delta: { op: "append_text", entry_id: "a1", delta: " Cargo.toml", kind: "assistant" },
    });
    expect(state.entries[1]).toMatchObject({ type: "assistant_text", text: "Checking Cargo.toml" });

    // Remove optimistic user echo cleanly
    state = applyChatDelta(state, {
      agent: "pi_1",
      seq: 4,
      delta: { op: "remove_entry", entry_id: "__bridge_pending_user__:1" },
    });
    expect(state.entries).toHaveLength(1);
    expect(state.entries[0].id).toBe("a1");

    // TurnFinished clears streaming flags
    state = applyChatDelta(state, {
      agent: "pi_1",
      seq: 5,
      delta: { op: "turn_finished" },
    });
    expect(state.meta.is_streaming).toBe(false);
    expect(state.entries[0]).toMatchObject({ type: "assistant_text", streaming: false });
  });

  it("clusters consecutive non-error exploration tool calls", () => {
    const mkTool = (id: string, kind: "read" | "search" | "edit"): TimelineEntry => ({
      type: "tool_call",
      id,
      call_id: id,
      name: kind,
      kind,
      input: { path: "src/main.rs" },
      locations: [],
      state: "completed",
      is_error: false,
      created_at_ms: 1,
    });
    const items = buildDisplayItems([
      mkTool("t1", "read"),
      mkTool("t2", "search"),
      mkTool("t3", "edit"),
    ]);
    expect(items).toHaveLength(2);
    expect(items[0].kind).toBe("exploration_cluster");
    expect(items[1].kind).toBe("single");
    if (items[1].kind === "single" && items[1].entry.type === "tool_call") {
      expect(summarizeToolInput(items[1].entry)).toBe("src/main.rs");
    }
  });

  it("computes unified diff additions, deletions, and line numbers", () => {
    const res = computeDiffRows("a\nold\nc\n", "a\nnew1\nnew2\nc\n");
    expect(res.additions).toBe(2);
    expect(res.deletions).toBe(1);
    expect(res.rows.some((r) => r.type === "del" && r.text === "old")).toBe(true);
    expect(res.rows.some((r) => r.type === "add" && r.text === "new2")).toBe(true);
  });

  it("orders Pi and generic ACP presets first and separates Shell", () => {
    const { agents, shell } = orderPresetsPiFirst([
      { id: "shell", label: "Shell", agent_type: "shell", program: "zsh", args: [], env: {}, require_installed: false },
      { id: "claude", label: "Claude", agent_type: "claude", program: "claude", args: [], env: {}, require_installed: true },
      { id: "opencode", label: "OpenCode", agent_type: "opencode", program: "opencode", args: [], env: {}, require_installed: true },
      { id: "pi", label: "Pi", agent_type: "pi", program: "pi", args: [], env: {}, require_installed: true },
    ]);
    expect(shell?.id).toBe("shell");
    expect(agents.map((a) => a.id)).toEqual(["pi", "opencode", "claude"]);
  });
});
