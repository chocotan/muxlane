import { execFileSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import {
  buildDshWebArgs,
  buildRemoteDshForwardArgs,
  buildRemoteDshScript,
  extractDshBrowserSessionSecret,
  extractDshWebUrl,
  mintDshSessionCookie,
  normalizeCustomDshUrl,
  parseListenAddressToUrl,
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

  it("probes live DSH endpoints with token or signed Cookie and rejects 401 unauthorized", async () => {
    const server = http.createServer((req, res) => {
      if (req.url === "/ok" || req.headers.cookie === "dsh-auth-test=valid") {
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
      expect(await probeDshUrl(`http://127.0.0.1:${port}/`, 1500, "dsh-auth-test=valid")).toBe(
        true,
      );
      expect(await probeDshUrl(`http://127.0.0.1:${port}/`)).toBe(false);
    } finally {
      await new Promise<void>((resolve) => server.close(() => resolve()));
    }
  });

  it("extracts only browser-session secret and mints authority-bound dsh-auth cookie", () => {
    const rawSecret = crypto
      .randomBytes(32)
      .toString("base64")
      .replace(/\+/g, "-")
      .replace(/\//g, "_")
      .replace(/=+$/, "");
    const yaml = [
      "version: 1",
      "records:",
      "  client-connection/browser-session:",
      "    kind: grant",
      "    payload:",
      "      version: 1",
      `      secret: ${rawSecret}`,
      "  other/provider:",
      "    payload:",
      "      secret: DO_NOT_PICK_THIS_SECRET_VALUE",
      "refs:",
      "  JKL_API_KEY: super-secret-api-key",
    ].join("\n");

    expect(extractDshBrowserSessionSecret(yaml)).toBe(rawSecret);

    const cookie = mintDshSessionCookie("http://127.0.0.1:3080/", rawSecret, 1_700_000_000_000);
    expect(cookie).not.toBeNull();
    expect(cookie?.url).toBe("http://127.0.0.1:3080/");
    expect(cookie?.name.startsWith("dsh-auth-")).toBe(true);
    expect(cookie?.value.startsWith("v1.")).toBe(true);
    expect(cookie?.value.split(".")).toHaveLength(3);
    expect(cookie?.header).toBe(`${cookie?.name}=${cookie?.value}`);
  });

  it("normalizes ss/lsof listen addresses into connectable HTTP URLs", () => {
    expect(parseListenAddressToUrl("100.64.10.138:3080")).toBe("http://100.64.10.138:3080/");
    expect(parseListenAddressToUrl("127.0.0.1:45443")).toBe("http://127.0.0.1:45443/");
    expect(parseListenAddressToUrl("0.0.0.0:3080")).toBe("http://127.0.0.1:3080/");
    expect(parseListenAddressToUrl("*:3080")).toBe("http://127.0.0.1:3080/");
    expect(parseListenAddressToUrl("[::]:3080")).toBe("http://127.0.0.1:3080/");
    expect(parseListenAddressToUrl("[::ffff:100.64.10.138]:3080")).toBe(
      "http://100.64.10.138:3080/",
    );
    expect(parseListenAddressToUrl("[::1]:3080")).toBe("http://[::1]:3080/");
    expect(parseListenAddressToUrl("invalid")).toBeNull();
  });

  it("generates remote SSH dsh script without --no-open and with existing-process reuse", () => {
    const script = buildRemoteDshScript("/srv/repo", 41234, false);
    expect(script).toContain("dsh-web.pid");
    expect(script).toContain("find_existing_dsh_listen");
    expect(script).toContain("read_dsh_secret");
    expect(script).toContain("--port 41234 --trusted-host 127.0.0.1:41234");
    expect(script).toContain("SSH_CONNECTION=");
    expect(script).not.toContain("--no-open");
    expect(script).not.toContain("--profile");
    expect(script).not.toContain("\\x27");
  });

  it("remote script extract_url preserves full tokens containing x, 2, 7 and ANSI colors", () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "muxlane-dsh-test-"));
    try {
      const dataDir = path.join(tmpDir, "muxlane");
      fs.mkdirSync(dataDir, { recursive: true });
      const expectedUrl =
        "http://127.0.0.1:39409/?token=AbC_x27_0123456789-xyzXYZ_EndToken72x";
      fs.writeFileSync(path.join(dataDir, "dsh-web.pid"), `${process.pid}\n`);
      fs.writeFileSync(
        path.join(dataDir, "dsh-web.log"),
        `\x1b[32mdsh web:\x1b[0m ${expectedUrl}\r\n`,
      );
      const script = buildRemoteDshScript(tmpDir, 39409, false);
      const out = execFileSync("sh", ["-c", script], {
        env: { ...process.env, XDG_DATA_HOME: tmpDir },
        encoding: "utf8",
      });
      expect(out.trim()).toBe(`DSH_URL\t${expectedUrl}`);
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  it("disables ControlMaster and ControlPath on remote DSH port-forward tunnels", () => {
    const args = buildRemoteDshForwardArgs("127.0.0.1:41234:127.0.0.1:41234", "alice@remote");
    expect(args).toContain("-NT");
    expect(args).toContain("ControlMaster=no");
    expect(args).toContain("ControlPath=none");
    expect(args).toContain("ControlPersist=no");
    expect(args).toContain("ExitOnForwardFailure=yes");
    expect(args).toContain("-L");
    expect(args).toContain("127.0.0.1:41234:127.0.0.1:41234");
    expect(args[args.length - 1]).toBe("alice@remote");
    expect(args.join(" ")).not.toContain("ControlMaster=auto");
  });
});
