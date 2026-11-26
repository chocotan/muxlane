import fs from "node:fs";
import { describe, expect, it } from "vitest";
import {
  classifySshFailure,
  ensureAskpassScript,
  parseProbeOutput,
  resolveSshTarget,
  sanitizeControlName,
  shQuote,
} from "./host-manager";

describe("host-manager Rust tunnel.rs port", () => {
  it("resolves SSH destination and optional inline socket path", () => {
    expect(resolveSshTarget({ target: "192.168.1.20", username: "ubuntu" })).toEqual({
      destination: "ubuntu@192.168.1.20",
      explicitSocket: null,
    });
    expect(resolveSshTarget({ target: "alice@dev-box", username: "ignored" })).toEqual({
      destination: "alice@dev-box",
      explicitSocket: null,
    });
    expect(
      resolveSshTarget({ target: "192.168.1.20:/run/user/1000/muxlane.sock", username: "choco" }),
    ).toEqual({
      destination: "choco@192.168.1.20",
      explicitSocket: "/run/user/1000/muxlane.sock",
    });
  });

  it("parses remote probe outputs (READY / STOPPED / MISSING)", () => {
    expect(parseProbeOutput("READY\t/run/user/1000/muxlane.sock\n")).toEqual({
      kind: "ready",
      socket: "/run/user/1000/muxlane.sock",
    });
    expect(parseProbeOutput("STOPPED\t/tmp/muxlane.sock\t/usr/local/bin/muxlane\n")).toEqual({
      kind: "stopped",
      socket: "/tmp/muxlane.sock",
      binary: "/usr/local/bin/muxlane",
    });
    expect(parseProbeOutput("MISSING\t/home/u/.local/share/muxlane/muxlane.sock\n")).toEqual({
      kind: "missing",
      socket: "/home/u/.local/share/muxlane/muxlane.sock",
    });
  });

  it("writes askpass script with 0700 perms and never embeds any password", () => {
    const scriptPath = ensureAskpassScript();
    const content = fs.readFileSync(scriptPath, "utf8");
    expect(content).toContain("MUXLANE_SSH_SECRET_FILE");
    expect(content).toContain("rm -f --");
    const mode = fs.statSync(scriptPath).mode & 0o777;
    expect(mode).toBe(0o700);
  });

  it("quotes shell arguments safely and sanitizes control socket names", () => {
    expect(shQuote("/tmp/a'b")).toBe("'/tmp/a'\\''b'");
    expect(shQuote("/tmp/a b;rm")).toBe("'/tmp/a b;rm'");
    expect(sanitizeControlName("user@host.example:22")).toBe("user_host.example_22");
  });

  it("classifies SSH authentication errors clearly", () => {
    const err = classifySshFailure("Permission denied (publickey,password).");
    expect(err.message).toContain("SSH 认证失败");
  });
});
