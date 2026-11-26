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
import {
  computeCacheHitPct,
  computeTps,
  formatDurationMs,
  formatTokensCompact,
} from "./formatStats";

describe("usage reducer & formatStats helpers", () => {
  it("applies set_assistant_usage onto assistant_text entries", () => {
    const history: ChatHistoryResult = {
      agent: "pie_1",
      seq: 1,
      meta: defaultChatMeta(),
      entries: [
        {
          type: "assistant_text",
          id: "a1",
          text: "Done.",
          streaming: false,
          model: "DeepSeek",
          created_at_ms: 100,
        },
      ],
      pending_approvals: [],
    };
    let state = timelineFromHistory(history);
    state = applyChatDelta(state, {
      agent: "pie_1",
      seq: 2,
      delta: {
        op: "set_assistant_usage",
        entry_id: "a1",
        usage: {
          input_tokens: 1000,
          output_tokens: 200,
          cache_read_tokens: 3000,
          cache_write_tokens: 0,
          reasoning_tokens: 0,
          total_tokens: 4200,
          duration_ms: 2000,
          ttft_ms: 350,
          cost_usd: 0.004,
        },
      },
    });
    expect(state.entries[0]).toMatchObject({
      type: "assistant_text",
      usage: { total_tokens: 4200, duration_ms: 2000 },
    });
  });

  it("formats compact tokens, durations, cache hit %, and TPS cleanly", () => {
    expect(formatTokensCompact(850)).toBe("850");
    expect(formatTokensCompact(4200)).toBe("4.2K");
    expect(formatTokensCompact(150_000)).toBe("150K");
    expect(formatTokensCompact(1_250_000)).toBe("1.25M");
    expect(formatDurationMs(420)).toBe("420ms");
    expect(formatDurationMs(2450)).toBe("2.5s");
    expect(formatDurationMs(75_000)).toBe("1m 15s");
    expect(computeCacheHitPct(1000, 3000)).toBe(75);
    expect(computeTps(200, 2000)).toBe("100");
    expect(computeTps(50, 2000)).toBe("25.0");
  });
});
