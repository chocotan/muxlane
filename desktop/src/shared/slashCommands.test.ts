import { describe, expect, it } from "vitest";
import {
  applySlashCompletion,
  buildSlashCatalog,
  deriveSlashAutocomplete,
  editReplacesTriggerToken,
} from "./slashCommands";

describe("slashCommands (Orca port)", () => {
  const reported = [
    { name: "ponytail", description: "Set ponytail mode", kind: "command", source: "extension" },
    { name: "usage", description: "Token usage per provider", kind: "command", source: "extension" },
    {
      name: "skill:ponytail-review",
      description: "Quality review of a change",
      kind: "skill",
      source: "skill",
    },
    {
      name: "skill:ui-ux-pro-max",
      description: "UI/UX design intelligence",
      kind: "skill",
      source: "skill",
    },
  ];

  it("merges local builtin commands with reported extension commands and skills", () => {
    const catalog = buildSlashCatalog(reported, "pi_rpc");
    expect(catalog.some((i) => i.name === "compact" && i.kind === "command")).toBe(true);
    expect(catalog.some((i) => i.name === "model" && i.localAction === "model")).toBe(true);
    expect(catalog.some((i) => i.name === "thinking" && i.localAction === "thinking")).toBe(true);
    expect(catalog.some((i) => i.name === "ponytail" && i.source === "extension")).toBe(true);
    expect(catalog.some((i) => i.name === "skill:ponytail-review" && i.kind === "skill")).toBe(
      true,
    );
  });

  it("opens dispatchable menu on leading `/` and non-dispatchable menu on mid-prompt `/`", () => {
    const catalog = buildSlashCatalog(reported, "pi_rpc");
    const leading = deriveSlashAutocomplete("/usa", 4, catalog);
    expect(leading.open).toBe(true);
    if (leading.open) {
      expect(leading.dispatchable).toBe(true);
      expect(leading.triggerKey).toBe("/:0");
      expect(leading.items[0]?.name).toBe("usage");
    }

    const mid = deriveSlashAutocomplete("请用 /ui", 6, catalog);
    expect(mid.open).toBe(true);
    if (mid.open) {
      expect(mid.dispatchable).toBe(false);
      expect(mid.triggerKey).toBe("/:3");
      expect(mid.items[0]?.name).toBe("skill:ui-ux-pro-max");
    }
  });

  it("prefers exact skill alias match over fuzzy command match", () => {
    const catalog = buildSlashCatalog(reported, "pi_rpc");
    const state = deriveSlashAutocomplete("/ponytail-review", 16, catalog);
    expect(state.open).toBe(true);
    if (state.open) {
      expect(state.items[state.preferredIndex]?.name).toBe("skill:ponytail-review");
    }
  });

  it("matches /effort alias to /thinking", () => {
    const catalog = buildSlashCatalog(reported, "pi_rpc");
    const state = deriveSlashAutocomplete("/effort", 7, catalog);
    expect(state.open).toBe(true);
    if (state.open) {
      expect(state.items[0]?.name).toBe("thinking");
      expect(state.items[0]?.localAction).toBe("thinking");
    }
  });

  it("replaces slash token cleanly while preserving surrounding text and caret", () => {
    const catalog = buildSlashCatalog(reported, "pi_rpc");
    const skill = catalog.find((i) => i.name === "skill:ui-ux-pro-max")!;
    const completed = applySlashCompletion("请帮我 /ui 优化页面", 7, skill);
    expect(completed.draft).toBe("请帮我 /skill:ui-ux-pro-max 优化页面");
    expect(completed.caret).toBe("请帮我 /skill:ui-ux-pro-max ".length);
  });

  it("respects dismissedTriggerKey until a different trigger or replacement occurs", () => {
    const catalog = buildSlashCatalog(reported, "pi_rpc");
    expect(deriveSlashAutocomplete("/com", 4, catalog, "/:0").open).toBe(false);
    expect(editReplacesTriggerToken("/com", "/comp", "/:0")).toBe(false);
    expect(editReplacesTriggerToken("/com", "/new", "/:0")).toBe(true);
  });
});
