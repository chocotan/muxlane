import { execFile, spawn, type ChildProcess } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import type { AddHostInput, HostEntry, HostsState, RelayConfig } from "../src/shared/protocol";
import { resolveMuxlaneSocketPath } from "./muxlane-socket";

export const LOCAL_HOST_ID = "local";

function muxlaneDataDir(): string {
  const dataHome = process.env.XDG_DATA_HOME || path.join(os.homedir(), ".local", "share");
  return path.join(dataHome, "muxlane");
}

function hostsFilePath(): string {
  return path.join(muxlaneDataDir(), "desktop-hosts.json");
}

function stateJsonPath(): string {
  return path.join(muxlaneDataDir(), "state.json");
}

function buildLocalHost(): HostEntry {
  return {
    id: LOCAL_HOST_ID,
    name: os.hostname() || "本机",
    kind: "local",
    target: resolveMuxlaneSocketPath(),
  };
}

interface PersistedHostsFile {
  activeHostId?: string;
  hosts?: HostEntry[];
}

function importLegacyGpuiRemotes(): HostEntry[] {
  try {
    const raw = fs.readFileSync(stateJsonPath(), "utf8");
    const parsed = JSON.parse(raw) as {
      remote_configs?: Array<{
        target?: string;
        auth?: {
          kind?: string;
          username?: string;
          identity_file?: string;
        };
      }>;
    };
    const items = parsed.remote_configs ?? [];
    const imported: HostEntry[] = [];
    for (const item of items) {
      const target = item.target?.trim();
      if (!target) continue;
      if (target.startsWith("/") || target.startsWith("unix://")) {
        const clean = target.replace(/^unix:\/\//, "");
        imported.push({
          id: `remote_${crypto.randomBytes(4).toString("hex")}`,
          name: path.basename(clean) || clean,
          kind: "socket",
          target: clean,
        });
      } else if (!target.startsWith("relay://")) {
        imported.push({
          id: `remote_${crypto.randomBytes(4).toString("hex")}`,
          name: target.replace(/^[^@]+@/, ""),
          kind: "ssh",
          target,
          authKind: item.auth?.kind === "public_key" ? "public_key" : "ssh_config",
          username: item.auth?.username || undefined,
          identityFile: item.auth?.identity_file || undefined,
        });
      }
    }
    return imported;
  } catch {
    return [];
  }
}

function loadSavedRemotes(): { activeHostId: string; remotes: HostEntry[] } {
  const filePath = hostsFilePath();
  if (fs.existsSync(filePath)) {
    try {
      const parsed = JSON.parse(fs.readFileSync(filePath, "utf8")) as PersistedHostsFile;
      const remotes = (parsed.hosts ?? []).filter((h) => h.id !== LOCAL_HOST_ID && h.target);
      const activeHostId =
        parsed.activeHostId &&
        (parsed.activeHostId === LOCAL_HOST_ID || remotes.some((r) => r.id === parsed.activeHostId))
          ? parsed.activeHostId
          : LOCAL_HOST_ID;
      return { activeHostId, remotes };
    } catch {
      return { activeHostId: LOCAL_HOST_ID, remotes: [] };
    }
  }
  const imported = importLegacyGpuiRemotes();
  return { activeHostId: LOCAL_HOST_ID, remotes: imported };
}

function persistRemotes(activeHostId: string, remotes: HostEntry[]): void {
  const dir = muxlaneDataDir();
  fs.mkdirSync(dir, { recursive: true });
  const tmp = `${hostsFilePath()}.tmp.${process.pid}`;
  fs.writeFileSync(tmp, JSON.stringify({ activeHostId, hosts: remotes }, null, 2), "utf8");
  fs.renameSync(tmp, hostsFilePath());
}

export function readRelayConfig(): RelayConfig {
  try {
    const parsed = JSON.parse(fs.readFileSync(stateJsonPath(), "utf8")) as {
      relay_enabled?: boolean;
      relay_url?: string | null;
    };
    return {
      relayEnabled: Boolean(parsed.relay_enabled),
      relayUrl: typeof parsed.relay_url === "string" ? parsed.relay_url : "",
    };
  } catch {
    return { relayEnabled: false, relayUrl: "" };
  }
}

export function writeRelayConfig(cfg: RelayConfig): RelayConfig {
  const normalized: RelayConfig = {
    relayEnabled: Boolean(cfg.relayEnabled),
    relayUrl: cfg.relayUrl.trim().replace(/\/+$/, ""),
  };
  try {
    const filePath = stateJsonPath();
    if (fs.existsSync(filePath)) {
      const parsed = JSON.parse(fs.readFileSync(filePath, "utf8")) as Record<string, unknown>;
      parsed.relay_enabled = normalized.relayEnabled;
      parsed.relay_url = normalized.relayUrl || null;
      const tmp = `${filePath}.tmp.desktop.${process.pid}`;
      fs.writeFileSync(tmp, JSON.stringify(parsed, null, 2), "utf8");
      fs.renameSync(tmp, filePath);
    }
  } catch {
    // Non-fatal if state.json has not been created yet
  }
  return normalized;
}

function buildSshBaseArgs(host: HostEntry): string[] {
  const args: string[] = [
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=10",
    "-o",
    "ServerAliveInterval=15",
    "-o",
    "ServerAliveCountMax=3",
  ];
  if (host.port && Number.isFinite(host.port) && host.port > 0) {
    args.push("-p", String(host.port));
  }
  if (host.authKind === "public_key" && host.identityFile?.trim()) {
    args.push("-i", host.identityFile.trim(), "-o", "IdentitiesOnly=yes");
  }
  if (host.username?.trim() && !host.target.includes("@")) {
    args.push("-l", host.username.trim());
  }
  return args;
}

function probeRemoteSocketPath(host: HostEntry): Promise<string> {
  if (host.remoteSocketPath?.trim()) {
    return Promise.resolve(host.remoteSocketPath.trim());
  }
  const args = [
    ...buildSshBaseArgs(host),
    host.target,
    'printf "%s" "${XDG_DATA_HOME:-$HOME/.local/share}/muxlane/muxlane.sock"',
  ];
  return new Promise((resolve, reject) => {
    execFile("ssh", args, { timeout: 12_000 }, (err, stdout, stderr) => {
      if (err) {
        reject(new Error((stderr || err.message).trim() || `SSH 探测失败: ${host.target}`));
        return;
      }
      const discovered = stdout.trim();
      if (!discovered || !discovered.startsWith("/")) {
        reject(new Error(`未能自动发现远端 socket 路径 (${host.target})`));
        return;
      }
      resolve(discovered);
    });
  });
}

function waitForSocketFile(filePath: string, proc: ChildProcess, timeoutMs = 12_000): Promise<void> {
  return new Promise((resolve, reject) => {
    let settled = false;
    let stderrBuf = "";
    const started = Date.now();

    const finish = (err?: Error) => {
      if (settled) return;
      settled = true;
      clearInterval(poller);
      if (err) reject(err);
      else resolve();
    };

    proc.stderr?.on("data", (chunk: Buffer) => {
      stderrBuf += chunk.toString("utf8");
    });
    proc.once("exit", (code) => {
      finish(
        new Error(
          stderrBuf.trim() || `SSH 隧道已退出 (code ${code ?? "unknown"})`,
        ),
      );
    });
    proc.once("error", (err) => {
      finish(err);
    });

    const poller = setInterval(() => {
      if (fs.existsSync(filePath)) {
        finish();
        return;
      }
      if (Date.now() - started > timeoutMs) {
        finish(new Error(stderrBuf.trim() || "建立 SSH 隧道超时"));
      }
    }, 80);
  });
}

export class HostManager {
  private activeHostId: string;
  private remotes: HostEntry[];
  private tunnelProc: ChildProcess | null = null;
  private tunnelLocalSocket: string | null = null;
  private tunnelStatus: string | null = null;

  constructor(
    private readonly onTargetResolved: (socketPath: string, isLocal: boolean) => void,
    private readonly onHostsChanged: (state: HostsState) => void,
  ) {
    const loaded = loadSavedRemotes();
    this.activeHostId = loaded.activeHostId;
    this.remotes = loaded.remotes;
  }

  getState(): HostsState {
    return {
      activeHostId: this.activeHostId,
      hosts: [buildLocalHost(), ...this.remotes],
      tunnelStatus: this.tunnelStatus,
    };
  }

  async start(): Promise<void> {
    await this.activateHost(this.activeHostId);
  }

  async addHost(input: AddHostInput): Promise<HostsState> {
    const target = input.target.trim();
    if (!target) {
      throw new Error("请输入目标主机或 Socket 路径");
    }
    const defaultName =
      input.kind === "socket"
        ? path.basename(target) || target
        : target.replace(/^[^@]+@/, "");
    const entry: HostEntry = {
      id: `remote_${crypto.randomBytes(4).toString("hex")}`,
      name: input.name?.trim() || defaultName,
      kind: input.kind,
      target,
      authKind: input.kind === "ssh" ? (input.authKind ?? "ssh_config") : undefined,
      username: input.username?.trim() || undefined,
      port: input.port && Number.isFinite(input.port) ? input.port : undefined,
      identityFile: input.identityFile?.trim() || undefined,
      remoteSocketPath: input.remoteSocketPath?.trim() || undefined,
    };
    this.remotes = [...this.remotes, entry];
    persistRemotes(this.activeHostId, this.remotes);
    this.onHostsChanged(this.getState());
    return this.selectHost(entry.id);
  }

  async removeHost(hostId: string): Promise<HostsState> {
    if (hostId === LOCAL_HOST_ID) {
      return this.getState();
    }
    this.remotes = this.remotes.filter((h) => h.id !== hostId);
    const needSwitchLocal = this.activeHostId === hostId;
    if (needSwitchLocal) {
      this.activeHostId = LOCAL_HOST_ID;
    }
    persistRemotes(this.activeHostId, this.remotes);
    if (needSwitchLocal) {
      await this.activateHost(LOCAL_HOST_ID);
    } else {
      this.onHostsChanged(this.getState());
    }
    return this.getState();
  }

  async selectHost(hostId: string): Promise<HostsState> {
    const exists = hostId === LOCAL_HOST_ID || this.remotes.some((h) => h.id === hostId);
    if (!exists) {
      throw new Error(`未找到主机: ${hostId}`);
    }
    this.activeHostId = hostId;
    persistRemotes(this.activeHostId, this.remotes);
    await this.activateHost(hostId);
    return this.getState();
  }

  dispose(): void {
    this.teardownTunnel();
  }

  private teardownTunnel(): void {
    if (this.tunnelProc) {
      try {
        this.tunnelProc.kill("SIGTERM");
      } catch {
        // ignore
      }
      this.tunnelProc = null;
    }
    if (this.tunnelLocalSocket) {
      try {
        fs.unlinkSync(this.tunnelLocalSocket);
      } catch {
        // ignore
      }
      this.tunnelLocalSocket = null;
    }
    this.tunnelStatus = null;
  }

  private async activateHost(hostId: string): Promise<void> {
    this.teardownTunnel();

    if (hostId === LOCAL_HOST_ID) {
      this.tunnelStatus = null;
      this.onHostsChanged(this.getState());
      this.onTargetResolved(resolveMuxlaneSocketPath(), true);
      return;
    }

    const host = this.remotes.find((h) => h.id === hostId);
    if (!host) {
      this.activeHostId = LOCAL_HOST_ID;
      this.onHostsChanged(this.getState());
      this.onTargetResolved(resolveMuxlaneSocketPath(), true);
      return;
    }

    if (host.kind === "socket") {
      this.tunnelStatus = null;
      this.onHostsChanged(this.getState());
      this.onTargetResolved(host.target, false);
      return;
    }

    // SSH unix-domain socket forwarding tunnel
    this.tunnelStatus = `正在建立 SSH 隧道 (${host.target})…`;
    this.onHostsChanged(this.getState());
    try {
      const remoteSock = await probeRemoteSocketPath(host);
      const tmpDir = path.join(os.tmpdir(), "muxlane-ssh");
      fs.mkdirSync(tmpDir, { recursive: true, mode: 0o700 });
      const localSock = path.join(
        tmpDir,
        `fwd-${host.id}-${crypto.randomBytes(3).toString("hex")}.sock`,
      );
      try {
        fs.unlinkSync(localSock);
      } catch {
        // ignore
      }

      const args = [
        "-NT",
        "-o",
        "ExitOnForwardFailure=yes",
        ...buildSshBaseArgs(host),
        "-L",
        `${localSock}:${remoteSock}`,
        host.target,
      ];
      const proc = spawn("ssh", args, { stdio: ["ignore", "ignore", "pipe"] });
      this.tunnelProc = proc;
      this.tunnelLocalSocket = localSock;

      await waitForSocketFile(localSock, proc);

      // Guard against concurrent host switches while awaiting tunnel readiness
      if (this.activeHostId !== host.id || this.tunnelProc !== proc) {
        return;
      }

      this.tunnelStatus = null;
      this.onHostsChanged(this.getState());
      this.onTargetResolved(localSock, false);
    } catch (err) {
      this.teardownTunnel();
      this.tunnelStatus = (err as Error).message;
      this.onHostsChanged(this.getState());
      throw err;
    }
  }
}
