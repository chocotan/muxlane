import { describe, expect, it } from "vitest";
import { extractDshWebUrl, normalizeCustomDshUrl } from "./dsh-manager";

describe("dsh-manager helpers", () => {
  it("extracts dsh web URL while ignoring unrelated warning links", () => {
    const noisyOutput = [
      "(node:123) Warning: see https://nodejs.org/api/cli.html for details",
      "\x1b[32mdsh web:\x1b[0m http://127.0.0.1:41527/?token=abc_123-XYZ",
    ].join("\n");
    expect(extractDshWebUrl(noisyOutput)).toBe("http://127.0.0.1:41527/?token=abc_123-XYZ");
    expect(extractDshWebUrl("Warning: https://example.com/docs")).toBeNull();
  });

  it("normalizes custom DSH WebUI URLs with or without http prefix", () => {
    expect(normalizeCustomDshUrl("http://100.64.10.138:3080/?token=xyz")).toBe(
      "http://100.64.10.138:3080/?token=xyz",
    );
    expect(normalizeCustomDshUrl("100.64.10.138:3080/?token=xyz")).toBe(
      "http://100.64.10.138:3080/?token=xyz",
    );
    expect(normalizeCustomDshUrl("   ")).toBe("");
    expect(() => normalizeCustomDshUrl("ftp://invalid")).toThrow();
  });
});
