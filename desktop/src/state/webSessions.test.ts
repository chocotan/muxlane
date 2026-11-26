import { describe, expect, it } from "vitest";
import { deriveTitleFromUrl, isWebSessionId, normalizeWebUrl } from "./webSessions";

describe("webSessions helpers", () => {
  it("normalizes local, IP, port, and domain URLs cleanly", () => {
    expect(normalizeWebUrl("https://github.com/chocotan/muxlane")).toBe(
      "https://github.com/chocotan/muxlane",
    );
    expect(normalizeWebUrl("github.com/chocotan")).toBe("https://github.com/chocotan");
    expect(normalizeWebUrl("localhost:5173")).toBe("http://localhost:5173");
    expect(normalizeWebUrl("127.0.0.1:3080/?token=abc")).toBe("http://127.0.0.1:3080/?token=abc");
    expect(normalizeWebUrl("100.64.10.138:3080")).toBe("http://100.64.10.138:3080");
    expect(normalizeWebUrl("   ")).toBe("");
    expect(() => normalizeWebUrl("not a valid url!!")).toThrow();
  });

  it("derives readable titles and identifies web session ids", () => {
    expect(deriveTitleFromUrl("http://localhost:3000/app", "web")).toBe("localhost:3000");
    expect(deriveTitleFromUrl("http://127.0.0.1:3080/?token=x", "dsh")).toBe("DeepSeek Harness");
    expect(isWebSessionId("web_abc_123")).toBe(true);
    expect(isWebSessionId("shell_abc")).toBe(false);
    expect(isWebSessionId(null)).toBe(false);
  });
});
