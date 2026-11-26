import { spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import type { DaemonConnectionStatus, RpcEventMessage } from "../src/shared/protocol";
import { Methods } from "../src/shared/protocol";

interface PendingRequest {
  resolve: (value: unknown) => void;
  reject: (err: Error) => void;
  timer: NodeJS.Timeout;
}

export function resolveMuxlaneSocketPath(): string {
  if (process.env.MUXLANE_SOCKET && process.env.MUXLANE_SOCKET.trim()) {
    return process.env.MUXLANE_SOCKET.trim();
  }
  const dataHome = process.env.XDG_DATA_HOME || path.join(os.homedir(), ".local", "share");
  return path.join(dataHome, "muxlane", "muxlane.sock");
}

function findMuxlaneBinary(repoRoot: string): string | null {
  if (process.env.MUXLANE_BIN && fs.existsSync(process.env.MUXLANE_BIN)) {
    return process.env.MUXLANE_BIN;
  }
  const localBuilds = [
    path.join(repoRoot, "target", "debug", "muxlane"),
    path.join(repoRoot, "target", "release", "muxlane"),
  ]
    .filter((candidate) => fs.existsSync(candidate))
    .sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs);
  if (localBuilds.length > 0) return localBuilds[0];

  const fallbacks = [
    path.join(os.homedir(), ".cargo", "bin", "muxlane"),
    "/usr/local/bin/muxlane",
    "/usr/bin/muxlane",
  ];
  for (const candidate of fallbacks) {
    if (fs.existsSync(candidate)) return candidate;
  }
  return "muxlane";
}

/**
 * Persistent line-delimited JSON-RPC client over Muxlane's Unix domain socket.
 * Automatically boots `muxlane --headless` when no daemon is currently listening.
 */
export class MuxlaneSocketClient {
  private socket: net.Socket | null = null;
  private readBuf: Buffer = Buffer.alloc(0);
  private nextId = 1;
  private pending = new Map<number, PendingRequest>();
  private reconnectTimer: NodeJS.Timeout | null = null;
  private backoffMs = 300;
  private disposed = false;
  private daemonSpawnAttempted = false;
  private spawnedDaemon: ChildProcess | null = null;
  private status: DaemonConnectionStatus;

  private socketPath: string;
  private allowAutoSpawnLocal = true;

  constructor(
    socketPath: string,
    private readonly repoRoot: string,
    private readonly onEvent: (ev: RpcEventMessage) => void,
    private readonly onStatus: (status: DaemonConnectionStatus) => void,
  ) {
    this.socketPath = socketPath;
    this.status = {
      connected: false,
      connecting: false,
      socketPath,
      error: null,
    };
  }

  getStatus(): DaemonConnectionStatus {
    return { ...this.status };
  }

  start(): void {
    this.disposed = false;
    this.connectNow();
  }

  switchTarget(socketPath: string, allowAutoSpawnLocal: boolean): void {
    this.socketPath = socketPath;
    this.allowAutoSpawnLocal = allowAutoSpawnLocal;
    this.daemonSpawnAttempted = false;
    this.backoffMs = 300;
    this.clearReconnect();
    if (this.socket) {
      this.socket.destroy();
      this.socket = null;
    }
    for (const req of this.pending.values()) {
      clearTimeout(req.timer);
      req.reject(new Error("已切换连接主机"));
    }
    this.pending.clear();
    this.updateStatus({
      connected: false,
      connecting: true,
      socketPath,
      error: null,
    });
    this.connectNow();
  }

  reconnectNow(): void {
    this.clearReconnect();
    this.backoffMs = 300;
    this.daemonSpawnAttempted = false;
    if (this.socket) {
      this.socket.destroy();
      this.socket = null;
    }
    this.connectNow();
  }

  dispose(): void {
    this.disposed = true;
    this.clearReconnect();
    if (this.socket) {
      this.socket.destroy();
      this.socket = null;
    }
    for (const req of this.pending.values()) {
      clearTimeout(req.timer);
      req.reject(new Error("Muxlane desktop closed"));
    }
    this.pending.clear();
  }

  call<T = unknown>(method: string, params: unknown = {}): Promise<T> {
    if (!this.socket || !this.status.connected) {
      return Promise.reject(new Error("未连接到 muxlane-server 守护进程"));
    }
    const id = this.nextId++;
    const line = JSON.stringify({ id, method, params }) + "\n";
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`RPC timeout: ${method}`));
      }, 30_000);
      this.pending.set(id, {
        resolve: resolve as (v: unknown) => void,
        reject,
        timer,
      });
      this.socket?.write(line, (err) => {
        if (err) {
          const item = this.pending.get(id);
          if (item) {
            clearTimeout(item.timer);
            this.pending.delete(id);
            item.reject(err);
          }
        }
      });
    });
  }

  private updateStatus(partial: Partial<DaemonConnectionStatus>): void {
    this.status = { ...this.status, ...partial };
    this.onStatus(this.getStatus());
  }

  private clearReconnect(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }

  private trySpawnHeadlessDaemon(): void {
    if (this.daemonSpawnAttempted || this.spawnedDaemon) return;
    this.daemonSpawnAttempted = true;
    const bin = findMuxlaneBinary(this.repoRoot);
    if (!bin) return;
    try {
      const child = spawn(bin, ["--headless"], {
        detached: true,
        stdio: "ignore",
        env: { ...process.env },
      });
      child.unref();
      child.on("error", () => {
        this.spawnedDaemon = null;
      });
      child.on("exit", () => {
        this.spawnedDaemon = null;
      });
      this.spawnedDaemon = child;
    } catch {
      this.spawnedDaemon = null;
    }
  }

  private connectNow(): void {
    if (this.disposed) return;
    this.clearReconnect();
    this.updateStatus({ connecting: true, error: null });

    const sock = net.createConnection(this.socketPath);
    this.socket = sock;
    this.readBuf = Buffer.alloc(0);

    sock.on("connect", () => {
      if (this.socket !== sock) return;
      this.backoffMs = 300;
      this.updateStatus({ connected: true, connecting: false, error: null });
      // Subscribe to global state + agent status + bot + approval push events immediately.
      void this.call(Methods.EVENTS_SUBSCRIBE, null).catch(() => {});
    });

    sock.on("data", (chunk: Buffer) => {
      this.readBuf = this.readBuf.length === 0 ? chunk : Buffer.concat([this.readBuf, chunk]);
      let nlIdx = this.readBuf.indexOf(0x0a);
      while (nlIdx !== -1) {
        const line = this.readBuf.subarray(0, nlIdx).toString("utf8").trim();
        this.readBuf = this.readBuf.subarray(nlIdx + 1);
        if (line.length > 0) {
          this.handleFrame(line);
        }
        nlIdx = this.readBuf.indexOf(0x0a);
      }
    });

    sock.on("error", (err: NodeJS.ErrnoException) => {
      if (this.socket !== sock) return;
      if (this.allowAutoSpawnLocal && (err.code === "ENOENT" || err.code === "ECONNREFUSED")) {
        this.trySpawnHeadlessDaemon();
      }
      this.updateStatus({
        connected: false,
        connecting: false,
        error: `守护进程未连接 (${err.code ?? err.message})`,
      });
    });

    sock.on("close", () => {
      if (this.socket !== sock) return;
      this.socket = null;
      for (const req of this.pending.values()) {
        clearTimeout(req.timer);
        req.reject(new Error("muxlane-server 连接断开"));
      }
      this.pending.clear();
      this.updateStatus({ connected: false, connecting: false });
      if (!this.disposed) {
        const delay = this.backoffMs;
        this.backoffMs = Math.min(this.backoffMs * 1.6, 4000);
        this.reconnectTimer = setTimeout(() => this.connectNow(), delay);
      }
    });
  }

  private handleFrame(line: string): void {
    let parsed: Record<string, unknown>;
    try {
      parsed = JSON.parse(line) as Record<string, unknown>;
    } catch {
      return;
    }

    if (typeof parsed.id === "number") {
      const req = this.pending.get(parsed.id);
      if (!req) return;
      clearTimeout(req.timer);
      this.pending.delete(parsed.id);
      if (parsed.error && typeof parsed.error === "object") {
        const errObj = parsed.error as { code?: string; message?: string };
        req.reject(new Error(errObj.message || errObj.code || "RPC error"));
      } else {
        req.resolve(parsed.result);
      }
      return;
    }

    if (typeof parsed.event === "string") {
      this.onEvent({
        event: parsed.event,
        params: parsed.params,
      });
    }
  }
}
