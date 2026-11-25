import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { MuxlaneSocketClient } from "./muxlane-socket";
import type { DaemonConnectionStatus, RpcEventMessage } from "../src/shared/protocol";

describe("MuxlaneSocketClient", () => {
  const cleanups: Array<() => void> = [];

  afterEach(() => {
    while (cleanups.length > 0) {
      cleanups.pop()?.();
    }
  });

  it("exchanges JSON-RPC frames over Unix socket and reassembles split push events", async () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "muxlane-sock-test-"));
    const sockPath = path.join(tmpDir, "muxlane.sock");
    cleanups.push(() => fs.rmSync(tmpDir, { recursive: true, force: true }));

    const server = net.createServer((conn) => {
      let buf = "";
      conn.on("data", (chunk) => {
        buf += chunk.toString("utf8");
        let idx = buf.indexOf("\n");
        while (idx !== -1) {
          const line = buf.slice(0, idx).trim();
          buf = buf.slice(idx + 1);
          if (line) {
            const req = JSON.parse(line) as { id: number; method: string };
            if (req.method === "events.subscribe") {
              conn.write(JSON.stringify({ id: req.id, result: { ok: true } }) + "\n");
              // Send a split push event frame across two chunks to test reassembly
              const push = JSON.stringify({
                event: "chat.delta",
                params: { agent: "pi_1", seq: 1, delta: { op: "reset" } },
              });
              conn.write(push.slice(0, 18));
              setTimeout(() => conn.write(push.slice(18) + "\n"), 10);
            } else if (req.method === "state.list") {
              conn.write(
                JSON.stringify({
                  id: req.id,
                  result: {
                    machine: { machine_id: "m1", name: "dev", os: "linux", version: "0.0.35" },
                    projects: [],
                    agents: [],
                    seq: 1,
                  },
                }) + "\n",
              );
            } else if (req.method === "fail.me") {
              conn.write(
                JSON.stringify({
                  id: req.id,
                  error: { code: "bad_params", message: "boom" },
                }) + "\n",
              );
            }
          }
          idx = buf.indexOf("\n");
        }
      });
    });

    await new Promise<void>((resolve) => server.listen(sockPath, resolve));
    cleanups.push(() => server.close());

    const events: RpcEventMessage[] = [];
    const statuses: DaemonConnectionStatus[] = [];

    const client = new MuxlaneSocketClient(
      sockPath,
      tmpDir,
      (ev) => events.push(ev),
      (st) => statuses.push(st),
    );
    cleanups.push(() => client.dispose());
    client.start();

    await new Promise<void>((resolve, reject) => {
      const deadline = Date.now() + 3000;
      const timer = setInterval(() => {
        if (client.getStatus().connected) {
          clearInterval(timer);
          resolve();
        } else if (Date.now() > deadline) {
          clearInterval(timer);
          reject(new Error("timed out connecting to unix socket"));
        }
      }, 15);
    });

    const snap = await client.call<{ machine: { name: string } }>("state.list");
    expect(snap.machine.name).toBe("dev");

    await expect(client.call("fail.me")).rejects.toThrow("boom");

    await new Promise<void>((resolve, reject) => {
      const deadline = Date.now() + 2000;
      const timer = setInterval(() => {
        if (events.some((e) => e.event === "chat.delta")) {
          clearInterval(timer);
          resolve();
        } else if (Date.now() > deadline) {
          clearInterval(timer);
          reject(new Error("timed out waiting for split push event"));
        }
      }, 15);
    });

    expect(events.some((e) => e.event === "chat.delta")).toBe(true);
    expect(statuses.some((s) => s.connected)).toBe(true);
  });
});
