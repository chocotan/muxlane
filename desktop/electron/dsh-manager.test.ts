import http from "node:http";
import { describe, expect, it } from "vitest";
import {
  buildDshWebArgs,
  extractDshWebUrl,
  normalizeCustomDshUrl,
  probeDshUrl,
} from "./dsh-manager";

describe("dsh-manager helpers", () => {
  it("builds dsh web args without --no-open or --profile", () => {
    const full = buildDshWebArgs(["-y", "@deepseek-ai/dsh"], false);
    expect(full).toEqual(["-y", "@deepseek-ai/dsh", "web", "--host", "127.0.0.1", "--port", "0"]);
    expect(full).not.toContain("--no-open");
    expect(full).not.toContain("--profile");

    expect(buildDshWebArgs(["-y", "@deepseek-ai/dsh"], true)).toEqual([
      "-y",
      "@deepseek-ai/dsh",
      "web",
    ]);
  });

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

  it("probes live DSH endpoints and rejects 401 unauthorized", async () => {
    const server = http.createServer((req, res) => {
      if (req.url === "/ok") {
        res.writeHead(200);
        res.end("ok");
      } else if (req.url === "/?token=good") {
        res.writeHead(303, { Location: "./" });
        res.end();
      } else {
        res.writeHead(401);
        res.end("unauthorized");
      }
    });
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const addr = server.address();
    const port = typeof addr === "object" && addr ? addr.port : 0;
    try {
      expect(await probeDshUrl(`http://127.0.0.1:${port}/ok`)).toBe(true);
      expect(await probeDshUrl(`http://127.0.0.1:${port}/?token=good`)).toBe(true);
      expect(await probeDshUrl(`http://127.0.0.1:${port}/`)).toBe(false);
    } finally {
      await new Promise<void>((resolve) => server.close(() => resolve()));
    }
  });
});
