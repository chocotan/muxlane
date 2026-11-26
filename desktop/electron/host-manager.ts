import { spawn, type ChildProcess } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";
import type {
  AddHostInput,
  HostEntry,
  HostsState,
  RelayConfig,
  SshAuthKind,
} from "../src/shared/protocol";
import { resolveMuxlaneSocketPath } from "./muxlane-socket";

export const LOCAL_HOST_ID = "local";

const ASKPASS_SCRIPT_CONTENT =
  '#!/bin/sh\nsecret=$(cat -- "$MUXLANE_SSH_SECRET_FILE" 2>/dev/null) || exit 1\nrm -f -- "$MUXLANE_SSH_SECRET_FILE"\nprintf \'%s\\n\' "$secret"\n';

function muxlaneDataDir(): string {
  const dataHome = process.env.XDG_DATA_HOME || path.join(os.homedir(), ".local", "share");
  return path.join(dataHome, "muxlane");
}

function sshDataDir(): string {
  return path.join(muxlaneDataDir(), "ssh");
}

function hostsFilePath(): string {
  return path.join(muxlaneDataDir(), "desktop-hosts.json");
}

function desktopSecretsPath(): string {
  return path.join(muxlaneDataDir(), "desktop-secrets.json");
}

function stateJsonPath(): string {
  return path.join(muxlaneDataDir(), "state.json");
}

function legacySecretsPath(): string {
  return path.join(muxlaneDataDir(), "secrets.json");
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

interface PersistedDesktopSecrets {
  version: number;
  passwords: Record<string, string>;
}

export type RemoteProbeResult =
  | { kind: "ready"; socket: string }
  | { kind: "stopped"; socket: string; binary: string }
  | { kind: "missing"; socket: string };

export function shQuote(s: string): string {
  if (/^[A-Za-z0-9/._-]+$/.test(s)) {
    return s;
  }
  return `'${s.replace(/'/g, "'\\''")}'`;
}

export function sanitizeControlName(s: string): string {
  return s.replace(/[^A-Za-z0-9.-]/g, "_");
}

export function expandTilde(filePath: string): string {
  if (filePath.startsWith("~/")) {
    return path.join(os.homedir(), filePath.slice(2));
  }
  return filePath;
}

/**
 * Port of Rust `parse_target` + `SshAuth::destination`:
 * Splits optional `host:/remote/socket.sock` and prepends `username@` when needed.
 */
export function resolveSshTarget(host: Pick<HostEntry, "target" | "username" | "remoteSocketPath">): {
  destination: string;
  explicitSocket: string | null;
} {
  let rawHost = host.target.trim();
  let explicitSocket = host.remoteSocketPath?.trim() || null;

  const colonIdx = rawHost.indexOf(":");
  if (colonIdx > 0 && rawHost.slice(colonIdx + 1).startsWith("/")) {
    const parsedHost = rawHost.slice(0, colonIdx).trim();
    const parsedSocket = rawHost.slice(colonIdx + 1).trim();
    if (parsedHost && parsedSocket) {
      rawHost = parsedHost;
      if (!explicitSocket) {
        explicitSocket = parsedSocket;
      }
    }
  }

  const user = host.username?.trim();
  const destination = !rawHost.includes("@") && user ? `${user}@${rawHost}` : rawHost;
  return { destination, explicitSocket };
}

export function parseProbeOutput(stdout: string): RemoteProbeResult {
  const line = stdout.trim();
  const fields = line.split("\t");
  if (fields[0] === "READY" && fields[1]) {
    return { kind: "ready", socket: fields[1] };
  }
  if (fields[0] === "STOPPED" && fields[1] && fields[2]) {
    return { kind: "stopped", socket: fields[1], binary: fields[2] };
  }
  if (fields[0] === "MISSING" && fields[1]) {
    return { kind: "missing", socket: fields[1] };
  }
  throw new Error(`无法解析远端探测结果: ${line || "(空输出)"}`);
}

export function classifySshFailure(stderr: string, fallback = "SSH 连接失败"): Error {
  const message = stderr.trim();
  if (
    message.includes("Permission denied") ||
    message.includes("Authentication failed") ||
    message.includes("Too many authentication failures")
  ) {
    return new Error(`SSH 认证失败（请检查用户名、密码或私钥）: ${message}`);
  }
  if (message.includes("REMOTE HOST IDENTIFICATION HAS CHANGED")) {
    return new Error(`远端主机指纹已变更，已拒绝连接: ${message}`);
  }
  return new Error(message || fallback);
}

export function ensureAskpassScript(): string {
  const dir = sshDataDir();
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  const scriptPath = path.join(dir, "muxlane-askpass.sh");
  let current: string | null = null;
  try {
    current = fs.readFileSync(scriptPath, "utf8");
  } catch {
    current = null;
  }
  if (current !== ASKPASS_SCRIPT_CONTENT) {
    fs.writeFileSync(scriptPath, ASKPASS_SCRIPT_CONTENT, { mode: 0o700 });
  }
  try {
    fs.chmodSync(scriptPath, 0o700);
  } catch {
    // ignore on platforms without chmod
  }
  return scriptPath;
}

function writeOneShotSecretFile(password: string): string {
  const runtimeDir = path.join(process.env.XDG_RUNTIME_DIR || os.tmpdir(), "muxlane");
  fs.mkdirSync(runtimeDir, { recursive: true, mode: 0o700 });
  const secretFile = path.join(
    runtimeDir,
    `ssh-secret-${Date.now()}-${crypto.randomBytes(8).toString("hex")}`,
  );
  // Create atomically with 0600 (eliminates write-then-chmod plaintext window, matching Rust tunnel.rs)
  let fd: number | null = null;
  try {
    fd = fs.openSync(secretFile, "wx", 0o600);
    fs.writeFileSync(fd, password, "utf8");
    fs.fsyncSync(fd);
    fs.closeSync(fd);
    fd = null;
    return secretFile;
  } catch (err) {
    if (fd !== null) {
      try {
        fs.closeSync(fd);
      } catch {
        // ignore
      }
    }
    try {
      fs.unlinkSync(secretFile);
    } catch {
      // ignore
    }
    throw err;
  }
}

function loadDesktopSecrets(): Record<string, string> {
  try {
    const raw = fs.readFileSync(desktopSecretsPath(), "utf8");
    const parsed = JSON.parse(raw) as Partial<PersistedDesktopSecrets>;
    if (parsed && typeof parsed.passwords === "object" && parsed.passwords !== null) {
      return { ...parsed.passwords };
    }
    return {};
  } catch {
    return {};
  }
}

function persistDesktopSecrets(passwords: Record<string, string>): void {
  const dir = muxlaneDataDir();
  fs.mkdirSync(dir, { recursive: true });
  const target = desktopSecretsPath();
  const tmp = `${target}.tmp.${process.pid}.${crypto.randomBytes(3).toString("hex")}`;
  const payload: PersistedDesktopSecrets = { version: 1, passwords };
  let fd: number | null = null;
  try {
    fd = fs.openSync(tmp, "wx", 0o600);
    fs.writeFileSync(fd, JSON.stringify(payload, null, 2), "utf8");
    fs.fsyncSync(fd);
    fs.closeSync(fd);
    fd = null;
    fs.renameSync(tmp, target);
    try {
      fs.chmodSync(target, 0o600);
    } catch {
      // ignore
    }
  } catch (err) {
    if (fd !== null) {
      try {
        fs.closeSync(fd);
      } catch {
        // ignore
      }
    }
    try {
      fs.unlinkSync(tmp);
    } catch {
      // ignore
    }
    throw err;
  }
}

function loadLegacyRustSecrets(): Record<string, string> {
  try {
    const raw = fs.readFileSync(legacySecretsPath(), "utf8");
    const parsed = JSON.parse(raw) as { remote_passwords?: Record<string, string> };
    return parsed.remote_passwords ?? {};
  } catch {
    return {};
  }
}

function importLegacyGpuiRemotes(): { remotes: HostEntry[]; passwords: Record<string, string> } {
  try {
    const raw = fs.readFileSync(stateJsonPath(), "utf8");
    const legacyPasswords = loadLegacyRustSecrets();
    const parsed = JSON.parse(raw) as {
      remote_configs?: Array<{
        target?: string;
        auth?: {
          auth?: string;
          kind?: string;
          username?: string;
          password?: string | null;
          identity_file?: string;
        };
      }>;
    };
    const items = parsed.remote_configs ?? [];
    const imported: HostEntry[] = [];
    const passwords: Record<string, string> = {};
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
      } else if (!target.startsWith("relay://") && !target.startsWith("ws://") && !target.startsWith("wss://")) {
        const rawAuth = item.auth?.auth || item.auth?.kind || "ssh_config";
        const authKind: SshAuthKind =
          rawAuth === "password"
            ? "password"
            : rawAuth === "public_key"
              ? "public_key"
              : "ssh_config";
        const id = `remote_${crypto.randomBytes(4).toString("hex")}`;
        const secret = item.auth?.password || legacyPasswords[target];
        if (authKind === "password" && secret) {
          passwords[id] = secret;
        }
        imported.push({
          id,
          name: target.replace(/^[^@]+@/, "").replace(/:\/.*$/, ""),
          kind: "ssh",
          target,
          authKind,
          username: item.auth?.username || undefined,
          identityFile: item.auth?.identity_file || undefined,
          hasPassword: authKind === "password" ? Boolean(secret) : undefined,
        });
      }
    }
    return { remotes: imported, passwords };
  } catch {
    return { remotes: [], passwords: {} };
  }
}

function loadSavedRemotes(): {
  activeHostId: string;
  remotes: HostEntry[];
  passwords: Record<string, string>;
} {
  const filePath = hostsFilePath();
  const passwords = loadDesktopSecrets();
  if (fs.existsSync(filePath)) {
    try {
      const parsed = JSON.parse(fs.readFileSync(filePath, "utf8")) as PersistedHostsFile;
      const remotes = (parsed.hosts ?? [])
        .filter((h) => h.id !== LOCAL_HOST_ID && h.target)
        .map((h) => ({
          ...h,
          hasPassword: h.authKind === "password" ? Boolean(passwords[h.id]) : undefined,
        }));
      const activeHostId =
        parsed.activeHostId &&
        (parsed.activeHostId === LOCAL_HOST_ID || remotes.some((r) => r.id === parsed.activeHostId))
          ? parsed.activeHostId
          : LOCAL_HOST_ID;
      return { activeHostId, remotes, passwords };
    } catch {
      return { activeHostId: LOCAL_HOST_ID, remotes: [], passwords };
    }
  }
  const imported = importLegacyGpuiRemotes();
  if (Object.keys(imported.passwords).length > 0) {
    try {
      persistDesktopSecrets({ ...passwords, ...imported.passwords });
    } catch {
      // non-fatal
    }
  }
  return {
    activeHostId: LOCAL_HOST_ID,
    remotes: imported.remotes,
    passwords: { ...passwords, ...imported.passwords },
  };
}

function persistRemotes(activeHostId: string, remotes: HostEntry[]): void {
  const dir = muxlaneDataDir();
  fs.mkdirSync(dir, { recursive: true });
  const sanitized = remotes.map(({ hasPassword: _unused, ...rest }) => rest);
  const tmp = `${hostsFilePath()}.tmp.${process.pid}`;
  fs.writeFileSync(tmp, JSON.stringify({ activeHostId, hosts: sanitized }, null, 2), "utf8");
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

function sharedControlPath(destination: string): string {
  const dir = sshDataDir();
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  return path.join(dir, `cm-${sanitizeControlName(destination)}.ctl`);
}

function sharedMasterArgs(destination: string): string[] {
  return [
    "-o",
    "ControlMaster=auto",
    "-o",
    "ControlPersist=600",
    "-o",
    `ControlPath=${sharedControlPath(destination)}`,
  ];
}

function buildSshBaseArgs(host: HostEntry): string[] {
  const args: string[] = [
    "-o",
    "ConnectTimeout=10",
    "-o",
    "NumberOfPasswordPrompts=1",
    "-o",
    "ServerAliveInterval=20",
    "-o",
    "ServerAliveCountMax=3",
    "-o",
    "StrictHostKeyChecking=accept-new",
  ];
  if (host.port && Number.isFinite(host.port) && host.port > 0) {
    args.push("-p", String(host.port));
  }
  if (host.authKind === "password") {
    args.push(
      "-o",
      "BatchMode=no",
      "-o",
      "PreferredAuthentications=password,keyboard-interactive",
    );
  } else {
    args.push("-o", "BatchMode=yes");
    if (host.authKind === "public_key" && host.identityFile?.trim()) {
      args.push("-o", "IdentitiesOnly=yes", "-i", expandTilde(host.identityFile.trim()));
    }
  }
  return args;
}

function spawnSshProcess(
  host: HostEntry,
  password: string | undefined,
  sshArgs: string[],
  stdio: ["ignore" | "pipe", "ignore" | "pipe", "pipe"],
): { proc: ChildProcess; cleanupSecret: () => void } {
  const allSshArgs = [...buildSshBaseArgs(host), ...sshArgs];
  if (host.authKind === "password") {
    if (!password) {
      throw new Error("未找到该主机的登录密码，请移除后重新添加并输入密码");
    }
    const askpass = ensureAskpassScript();
    const secretFile = writeOneShotSecretFile(password);
    const cleanupSecret = () => {
      try {
        fs.unlinkSync(secretFile);
      } catch {
        // already removed by askpass or wrapper
      }
    };
    const proc = spawn(
      "setsid",
      [
        "sh",
        "-c",
        'ssh "$@"; status=$?; rm -f -- "$MUXLANE_SSH_SECRET_FILE"; exit $status',
        "muxlane-ssh",
        ...allSshArgs,
      ],
      {
        stdio,
        env: {
          ...process.env,
          SSH_ASKPASS: askpass,
          SSH_ASKPASS_REQUIRE: "force",
          MUXLANE_SSH_SECRET_FILE: secretFile,
          DISPLAY: process.env.DISPLAY || ":0",
        },
      },
    );
    proc.once("exit", cleanupSecret);
    proc.once("error", cleanupSecret);
    return { proc, cleanupSecret };
  }

  const proc = spawn("ssh", allSshArgs, { stdio });
  return { proc, cleanupSecret: () => {} };
}

function execSshScript(
  host: HostEntry,
  password: string | undefined,
  destination: string,
  script: string,
  timeoutMs = 20_000,
): Promise<{ stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    let settled = false;
    let stdout = "";
    let stderr = "";
    let cleanup = () => {};
    let proc: ChildProcess;
    try {
      const spawned = spawnSshProcess(
        host,
        password,
        [...sharedMasterArgs(destination), destination, script],
        ["ignore", "pipe", "pipe"],
      );
      proc = spawned.proc;
      cleanup = spawned.cleanupSecret;
    } catch (err) {
      reject(err);
      return;
    }

    const timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      try {
        proc.kill("SIGKILL");
      } catch {
        // ignore
      }
      cleanup();
      reject(classifySshFailure(stderr, `SSH 执行超时 (${destination})`));
    }, timeoutMs);

    proc.stdout?.on("data", (chunk: Buffer) => {
      stdout += chunk.toString("utf8");
    });
    proc.stderr?.on("data", (chunk: Buffer) => {
      stderr += chunk.toString("utf8");
    });
    proc.once("error", (err) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      cleanup();
      reject(err);
    });
    proc.once("exit", (code) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      cleanup();
      if (code === 0) {
        resolve({ stdout, stderr });
      } else {
        reject(classifySshFailure(stderr, `SSH 执行失败 (exit ${code ?? "unknown"})`));
      }
    });
  });
}

function findLocalBootstrapBinary(repoRoot?: string): string | null {
  const candidates = [
    process.env.MUXLANE_BOOTSTRAP_BINARY,
    path.join(muxlaneDataDir(), "bin", "muxlane"),
    repoRoot ? path.join(repoRoot, "target", "release", "muxlane") : null,
    repoRoot ? path.join(repoRoot, "target", "debug", "muxlane") : null,
    path.join(os.homedir(), ".cargo", "bin", "muxlane"),
    "/usr/local/bin/muxlane",
    "/usr/bin/muxlane",
  ].filter((p): p is string => Boolean(p && p.trim()));

  for (const candidate of candidates) {
    try {
      if (fs.statSync(candidate).isFile()) {
        return candidate;
      }
    } catch {
      // continue
    }
  }
  return null;
}

const REMOTE_PROBE_SCRIPT = `data=\${XDG_DATA_HOME:-$HOME/.local/share}/muxlane
socket=$data/muxlane.sock
lock=$data/muxlane.lock
managed=$data/bin/muxlane
running=0
if [ -S "$socket" ] && command -v flock >/dev/null 2>&1 && ! flock -n "$lock" -c true 2>/dev/null; then running=1; fi
if [ "$running" -eq 1 ]; then printf 'READY\\t%s\\n' "$socket"
elif command -v muxlane >/dev/null 2>&1; then printf 'STOPPED\\t%s\\t%s\\n' "$socket" "$(command -v muxlane)"
elif [ -x "$managed" ]; then printf 'STOPPED\\t%s\\t%s\\n' "$socket" "$managed"
else printf 'MISSING\\t%s\\n' "$socket"
fi`;

async function probeRemote(
  host: HostEntry,
  password: string | undefined,
  destination: string,
): Promise<RemoteProbeResult> {
  const { stdout } = await execSshScript(host, password, destination, REMOTE_PROBE_SCRIPT, 15_000);
  return parseProbeOutput(stdout);
}

async function startRemoteDaemon(
  host: HostEntry,
  password: string | undefined,
  destination: string,
  binary?: string,
): Promise<void> {
  const explicit = binary ?? "";
  const script = `set -eu
data=\${XDG_DATA_HOME:-$HOME/.local/share}/muxlane
socket=$data/muxlane.sock
lock=$data/muxlane.lock
bin=${shQuote(explicit)}
if [ -z "$bin" ]; then bin=$data/bin/muxlane; fi
mkdir -p "$data/logs"
running=0
if [ -S "$socket" ] && command -v flock >/dev/null 2>&1 && ! flock -n "$lock" -c true 2>/dev/null; then running=1; fi
if [ "$running" -eq 0 ]; then
  rm -f -- "$socket"
  nohup "$bin" --headless </dev/null >>"$data/logs/headless.log" 2>&1 &
fi
for i in 1 2 3 4 5 6 7 8 9 10; do
  if [ -S "$socket" ] && ! flock -n "$lock" -c true 2>/dev/null; then exit 0; fi
  sleep .2
done
printf 'Muxlane headless did not create %s\\n' "$socket" >&2
exit 1`;
  await execSshScript(host, password, destination, script, 20_000);
}

async function uploadRemoteBinary(
  host: HostEntry,
  password: string | undefined,
  destination: string,
  binaryPath: string,
  onProgress: (percent: number) => void,
): Promise<void> {
  const rawBytes = fs.readFileSync(binaryPath);
  const compressed = zlib.gzipSync(rawBytes, { level: zlib.constants.Z_BEST_SPEED });

  const script = `set -eu
data=\${XDG_DATA_HOME:-$HOME/.local/share}/muxlane
mkdir -p "$data/bin" "$data/logs"
tmp=$data/bin/.muxlane-upload-$$
if command -v gzip >/dev/null 2>&1; then
  gzip -dc > "$tmp"
elif command -v gunzip >/dev/null 2>&1; then
  gunzip -c > "$tmp"
else
  cat > "$tmp"
fi
chmod 700 "$tmp"
"$tmp" --version >/dev/null
mv "$tmp" "$data/bin/muxlane"`;

  await new Promise<void>((resolve, reject) => {
    let stderr = "";
    let settled = false;
    const { proc, cleanupSecret } = spawnSshProcess(
      host,
      password,
      [...sharedMasterArgs(destination), destination, script],
      ["pipe", "ignore", "pipe"],
    );

    proc.stderr?.on("data", (chunk: Buffer) => {
      stderr += chunk.toString("utf8");
    });
    proc.once("error", (err) => {
      if (settled) return;
      settled = true;
      cleanupSecret();
      reject(err);
    });
    proc.once("exit", (code) => {
      if (settled) return;
      settled = true;
      cleanupSecret();
      if (code === 0) {
        onProgress(100);
        resolve();
      } else {
        reject(classifySshFailure(stderr, `上传远端 muxlane 失败 (exit ${code ?? "unknown"})`));
      }
    });

    const stdin = proc.stdin;
    if (!stdin) {
      cleanupSecret();
      reject(new Error("SSH upload stdin unavailable"));
      return;
    }

    const total = Math.max(1, compressed.length);
    const chunkSize = 128 * 1024;
    let offset = 0;
    let lastPercent = -1;

    const pump = () => {
      while (offset < compressed.length) {
        const end = Math.min(offset + chunkSize, compressed.length);
        const slice = compressed.subarray(offset, end);
        offset = end;
        const percent = Math.min(100, Math.floor((offset * 100) / total));
        if (percent !== lastPercent) {
          lastPercent = percent;
          onProgress(percent);
        }
        if (!stdin.write(slice)) {
          stdin.once("drain", pump);
          return;
        }
      }
      stdin.end();
    };
    pump();
  });
}

function testUnixSocketConnect(filePath: string): Promise<boolean> {
  return new Promise((resolve) => {
    const conn = net.createConnection(filePath);
    const done = (ok: boolean) => {
      conn.removeAllListeners();
      conn.destroy();
      resolve(ok);
    };
    conn.setTimeout(600, () => done(false));
    conn.once("connect", () => done(true));
    conn.once("error", () => done(false));
  });
}

function waitForHealthySocket(
  filePath: string,
  proc: ChildProcess,
  timeoutMs = 15_000,
): Promise<void> {
  return new Promise((resolve, reject) => {
    let settled = false;
    let stderrBuf = "";
    let probing = false;
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
      finish(classifySshFailure(stderrBuf, `SSH 隧道已退出 (code ${code ?? "unknown"})`));
    });
    proc.once("error", (err) => {
      finish(err);
    });

    const poller = setInterval(() => {
      if (Date.now() - started > timeoutMs) {
        finish(classifySshFailure(stderrBuf, "建立 SSH 隧道超时"));
        return;
      }
      if (probing || !fs.existsSync(filePath)) return;
      probing = true;
      void testUnixSocketConnect(filePath).then((ok) => {
        probing = false;
        if (ok) finish();
      });
    }, 100);
  });
}

function closeSharedControlMaster(destination: string): void {
  const ctl = sharedControlPath(destination);
  if (!fs.existsSync(ctl)) return;
  try {
    const exitProc = spawn("ssh", ["-S", ctl, "-O", "exit", destination], {
      stdio: "ignore",
    });
    exitProc.once("exit", () => {
      try {
        fs.unlinkSync(ctl);
      } catch {
        // ignore
      }
    });
    exitProc.once("error", () => {
      try {
        fs.unlinkSync(ctl);
      } catch {
        // ignore
      }
    });
  } catch {
    try {
      fs.unlinkSync(ctl);
    } catch {
      // ignore
    }
  }
}

interface ActiveTunnel {
  hostId: string;
  destination: string;
  proc: ChildProcess;
  localSock: string;
}

export class HostManager {
  private activeHostId: string;
  private remotes: HostEntry[];
  private passwords: Record<string, string>;
  /** Persistent per-host SSH tunnel pool (matching Rust `static TUNNELS`): switching hosts reuses live tunnels instantly. */
  private tunnels = new Map<string, ActiveTunnel>();
  private tunnelStatus: string | null = null;
  private tunnelPhase: "idle" | "connecting" | "error" = "idle";

  constructor(
    private readonly onTargetResolved: (socketPath: string, isLocal: boolean) => void,
    private readonly onHostsChanged: (state: HostsState) => void,
    private readonly repoRoot?: string,
  ) {
    const loaded = loadSavedRemotes();
    this.activeHostId = loaded.activeHostId;
    this.remotes = loaded.remotes;
    this.passwords = loaded.passwords;
  }

  getState(): HostsState {
    return {
      activeHostId: this.activeHostId,
      hosts: [
        buildLocalHost(),
        ...this.remotes.map((h) => {
          const t = this.tunnels.get(h.id);
          const ready = Boolean(t && t.proc.exitCode === null && fs.existsSync(t.localSock));
          return {
            ...h,
            hasPassword: h.authKind === "password" ? Boolean(this.passwords[h.id]) : undefined,
            tunnelReady: h.kind === "ssh" ? ready : undefined,
          };
        }),
      ],
      tunnelStatus: this.tunnelStatus,
      tunnelPhase: this.tunnelPhase,
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
    const authKind: SshAuthKind | undefined =
      input.kind === "ssh" ? (input.authKind ?? "ssh_config") : undefined;

    let username = input.username?.trim() || undefined;
    const password = input.password ?? "";
    if (authKind === "password") {
      if (!username && target.includes("@")) {
        username = target.split("@", 1)[0]?.trim() || undefined;
      }
      if (!username || !password) {
        throw new Error("用户名密码连接需要填写用户名和密码");
      }
    }

    const defaultName =
      input.kind === "socket"
        ? path.basename(target) || target
        : target.replace(/^[^@]+@/, "").replace(/:\/.*$/, "");
    const id = `remote_${crypto.randomBytes(4).toString("hex")}`;
    const entry: HostEntry = {
      id,
      name: input.name?.trim() || defaultName,
      kind: input.kind,
      target,
      authKind,
      username,
      port: input.port && Number.isFinite(input.port) ? input.port : undefined,
      identityFile: input.identityFile?.trim() || undefined,
      remoteSocketPath: input.remoteSocketPath?.trim() || undefined,
      hasPassword: authKind === "password" ? true : undefined,
    };

    const prevActiveId = this.activeHostId;
    const prevRemotes = this.remotes;
    const prevPasswords = { ...this.passwords };

    this.remotes = [...this.remotes, entry];
    if (authKind === "password") {
      this.passwords[id] = password;
      persistDesktopSecrets(this.passwords);
    }
    persistRemotes(this.activeHostId, this.remotes);
    this.onHostsChanged(this.getState());

    try {
      return await this.selectHost(entry.id);
    } catch (err) {
      // Roll back the failed newly-added host so user can fix typos/password directly without duplicates
      this.teardownHostTunnel(entry.id);
      this.remotes = prevRemotes;
      this.passwords = prevPasswords;
      this.activeHostId = prevActiveId;
      persistDesktopSecrets(this.passwords);
      persistRemotes(this.activeHostId, this.remotes);
      this.tunnelStatus = (err as Error).message;
      this.tunnelPhase = "error";
      this.onHostsChanged(this.getState());
      throw err;
    }
  }

  async removeHost(hostId: string): Promise<HostsState> {
    if (hostId === LOCAL_HOST_ID) {
      return this.getState();
    }
    this.teardownHostTunnel(hostId);
    const removed = this.remotes.find((h) => h.id === hostId);
    if (removed && removed.kind === "ssh") {
      const { destination } = resolveSshTarget(removed);
      closeSharedControlMaster(destination);
    }
    this.remotes = this.remotes.filter((h) => h.id !== hostId);
    if (hostId in this.passwords) {
      delete this.passwords[hostId];
      persistDesktopSecrets(this.passwords);
    }
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
    for (const hostId of Array.from(this.tunnels.keys())) {
      this.teardownHostTunnel(hostId);
    }
    this.tunnelStatus = null;
    this.tunnelPhase = "idle";
  }

  private teardownHostTunnel(hostId: string): void {
    const existing = this.tunnels.get(hostId);
    if (!existing) return;
    this.tunnels.delete(hostId);
    try {
      existing.proc.kill("SIGTERM");
    } catch {
      // ignore
    }
    try {
      fs.unlinkSync(existing.localSock);
    } catch {
      // ignore
    }
    closeSharedControlMaster(existing.destination);
  }

  private setProgress(status: string): void {
    this.tunnelStatus = status;
    this.tunnelPhase = "connecting";
    this.onHostsChanged(this.getState());
  }

  private async activateHost(hostId: string): Promise<void> {
    if (hostId === LOCAL_HOST_ID) {
      this.tunnelStatus = null;
      this.tunnelPhase = "idle";
      this.onHostsChanged(this.getState());
      this.onTargetResolved(resolveMuxlaneSocketPath(), true);
      return;
    }

    const host = this.remotes.find((h) => h.id === hostId);
    if (!host) {
      this.activeHostId = LOCAL_HOST_ID;
      this.tunnelStatus = null;
      this.tunnelPhase = "idle";
      this.onHostsChanged(this.getState());
      this.onTargetResolved(resolveMuxlaneSocketPath(), true);
      return;
    }

    if (host.kind === "socket") {
      this.tunnelStatus = null;
      this.tunnelPhase = "idle";
      this.onHostsChanged(this.getState());
      this.onTargetResolved(host.target, false);
      return;
    }

    // Fast path: reuse existing live SSH tunnel for this host without re-authenticating or re-probing!
    const cached = this.tunnels.get(host.id);
    if (cached && cached.proc.exitCode === null && fs.existsSync(cached.localSock)) {
      const healthy = await testUnixSocketConnect(cached.localSock);
      if (this.activeHostId !== host.id) return;
      if (healthy) {
        this.tunnelStatus = null;
        this.tunnelPhase = "idle";
        this.onHostsChanged(this.getState());
        this.onTargetResolved(cached.localSock, false);
        return;
      }
      this.teardownHostTunnel(host.id);
    } else if (cached) {
      this.teardownHostTunnel(host.id);
    }

    const password = this.passwords[host.id];
    const { destination, explicitSocket } = resolveSshTarget(host);
    this.setProgress(`正在探测远端主机 (${destination})…`);

    let createdProc: ChildProcess | null = null;
    let createdLocalSock: string | null = null;

    try {
      let remoteSock: string;
      if (explicitSocket) {
        remoteSock = explicitSocket;
      } else {
        const probe = await probeRemote(host, password, destination);
        if (this.activeHostId !== host.id) return;

        if (probe.kind === "ready") {
          remoteSock = probe.socket;
        } else if (probe.kind === "stopped") {
          this.setProgress(`远端 muxlane 未运行，正在启动 (${destination})…`);
          await startRemoteDaemon(host, password, destination, probe.binary);
          if (this.activeHostId !== host.id) return;
          remoteSock = probe.socket;
        } else {
          // probe.kind === "missing": auto-bootstrap by uploading local muxlane binary (exact Rust flow)
          const localBinary = findLocalBootstrapBinary(this.repoRoot);
          if (!localBinary) {
            throw new Error(
              `远端未安装 muxlane (${probe.socket})，且本机未找到可上传的 muxlane 二进制文件`,
            );
          }
          this.setProgress(`远端未安装 muxlane，正在上传并部署 (0%)…`);
          await uploadRemoteBinary(host, password, destination, localBinary, (pct) => {
            if (this.activeHostId === host.id) {
              this.setProgress(`远端未安装 muxlane，正在上传并部署 (${pct}%)…`);
            }
          });
          if (this.activeHostId !== host.id) return;
          this.setProgress(`部署完成，正在启动远端 muxlane 服务…`);
          await startRemoteDaemon(host, password, destination);
          if (this.activeHostId !== host.id) return;
          remoteSock = probe.socket;
        }
      }

      this.setProgress(`正在建立 SSH 隧道 (${destination})…`);
      const dir = sshDataDir();
      fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
      const localSock = path.join(
        dir,
        `fwd-${host.id}-${crypto.randomBytes(3).toString("hex")}.sock`,
      );
      try {
        fs.unlinkSync(localSock);
      } catch {
        // ignore
      }

      // Note: `-o ControlMaster=no` is critical so `-L` StreamLocalForward isn't silently dropped on a shared master.
      const tunnelArgs = [
        "-NT",
        "-o",
        "ControlMaster=no",
        "-o",
        "ExitOnForwardFailure=yes",
        "-o",
        "StreamLocalBindUnlink=yes",
        "-L",
        `${localSock}:${remoteSock}`,
        destination,
      ];
      const { proc } = spawnSshProcess(host, password, tunnelArgs, ["ignore", "ignore", "pipe"]);
      createdProc = proc;
      createdLocalSock = localSock;

      const entry: ActiveTunnel = { hostId: host.id, destination, proc, localSock };
      this.tunnels.set(host.id, entry);
      proc.once("exit", () => {
        if (this.tunnels.get(host.id) === entry) {
          this.tunnels.delete(host.id);
          try {
            fs.unlinkSync(localSock);
          } catch {
            // ignore
          }
          this.onHostsChanged(this.getState());
        }
      });

      await waitForHealthySocket(localSock, proc);

      // Guard against concurrent host switches while awaiting tunnel readiness
      if (this.activeHostId !== host.id || this.tunnels.get(host.id) !== entry) {
        return;
      }

      this.tunnelStatus = null;
      this.tunnelPhase = "idle";
      this.onHostsChanged(this.getState());
      this.onTargetResolved(localSock, false);
    } catch (err) {
      if (createdProc) {
        try {
          createdProc.kill("SIGTERM");
        } catch {
          // ignore
        }
      }
      if (createdLocalSock) {
        try {
          fs.unlinkSync(createdLocalSock);
        } catch {
          // ignore
        }
      }
      this.tunnels.delete(host.id);
      this.tunnelStatus = (err as Error).message;
      this.tunnelPhase = "error";
      this.onHostsChanged(this.getState());
      throw err;
    }
  }
}
