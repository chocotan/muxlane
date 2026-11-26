import { execFile, spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import https from "node:https";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import type { DshInstanceInfo } from "../src/shared/protocol";
import {
  classifySshFailure,
  execSshScript,
  LOCAL_HOST_ID,
  sharedMasterArgs,
  shQuote,
  spawnSshProcess,
  type HostConnectionContext,
} from "./host-manager";

interface SharedHostDsh {
  hostId: string;
  proc: ChildProcess | null; // local dsh process OR remote SSH port-forward process
  cleanupSecret?: () => void;
  url: string;
  cwd: string;
  isCustomUrl: boolean;
}

interface PersistedDshMap {
  byHost: Record<string, { url: string; isCustomUrl?: boolean }>;
}

const DSH_WEB_LINE_RE = /dsh\s+web:\s*(https?:\/\/[^\s"'<>]+)/i;
const LOOPBACK_URL_RE = /https?:\/\/(?:127\.0\.0\.1|localhost|\[::1\]):\d+[^\s"'<>]*/i;
const DEFAULT_LOCAL_CANDIDATES = ["http://127.0.0.1:3080/"];

let cachedLoginEnvPromise: Promise<NodeJS.ProcessEnv> | null = null;

function muxlaneDataDir(): string {
  const custom = process.env.MUXLANE_DATA_DIR?.trim();
  if (custom) return custom;
  const xdg = process.env.XDG_DATA_HOME?.trim();
  if (xdg) return path.join(xdg, "muxlane");
  return path.join(os.homedir(), ".local", "share", "muxlane");
}

function persistedDshFile(): string {
  return path.join(muxlaneDataDir(), "dsh-web.json");
}

function readPersistedDshMap(): PersistedDshMap {
  try {
    const raw = fs.readFileSync(persistedDshFile(), "utf8");
    const parsed = JSON.parse(raw) as Partial<PersistedDshMap> & {
      url?: string;
      isCustomUrl?: boolean;
    };
    if (parsed.byHost && typeof parsed.byHost === "object") {
      return { byHost: parsed.byHost };
    }
    // Migrate single-host v1 format
    if (typeof parsed.url === "string" && /^https?:\/\//i.test(parsed.url)) {
      return {
        byHost: {
          [LOCAL_HOST_ID]: { url: parsed.url, isCustomUrl: parsed.isCustomUrl === true },
        },
      };
    }
  } catch {
    // ignore missing/corrupt cache
  }
  return { byHost: {} };
}

function readPersistedDshForHost(hostId: string): { url: string; isCustomUrl?: boolean } | null {
  const entry = readPersistedDshMap().byHost[hostId];
  if (entry && typeof entry.url === "string" && /^https?:\/\//i.test(entry.url)) {
    return entry;
  }
  return null;
}

function writePersistedDshForHost(
  hostId: string,
  state: { url: string; isCustomUrl?: boolean } | null,
): void {
  try {
    const map = readPersistedDshMap();
    if (!state) {
      delete map.byHost[hostId];
    } else {
      map.byHost[hostId] = state;
    }
    const file = persistedDshFile();
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, JSON.stringify(map, null, 2), { mode: 0o600 });
    fs.chmodSync(file, 0o600);
  } catch {
    // ignore persistence errors
  }
}

function pickLoginShell(): string {
  const zshPath = "/usr/bin/zsh";
  const zshrc = path.join(os.homedir(), ".zshrc");
  if (fs.existsSync(zshPath) && fs.existsSync(zshrc)) {
    return zshPath;
  }
  return process.env.SHELL || "/bin/bash";
}

function loadLoginShellEnv(): Promise<NodeJS.ProcessEnv> {
  if (cachedLoginEnvPromise) return cachedLoginEnvPromise;
  if (process.platform === "win32") {
    cachedLoginEnvPromise = Promise.resolve({ ...process.env });
    return cachedLoginEnvPromise;
  }

  const shell = pickLoginShell();
  cachedLoginEnvPromise = new Promise<NodeJS.ProcessEnv>((resolve) => {
    const mergeAndResolve = (raw: string) => {
      const merged: NodeJS.ProcessEnv = { ...process.env };
      for (const entry of raw.split("\0")) {
        if (!entry) continue;
        const eq = entry.indexOf("=");
        if (eq <= 0) continue;
        const key = entry.slice(0, eq);
        const val = entry.slice(eq + 1);
        if (key === "PWD" || key === "SHLVL" || key === "_") continue;
        merged[key] = val;
      }
      resolve(merged);
    };

    execFile(
      shell,
      ["-ilc", "env -0"],
      { timeout: 3000, maxBuffer: 4 * 1024 * 1024, env: process.env },
      (err, stdout) => {
        if (!err && stdout && stdout.includes("\0")) {
          mergeAndResolve(stdout);
          return;
        }
        execFile(
          shell,
          ["-lc", "env -0"],
          { timeout: 2500, maxBuffer: 4 * 1024 * 1024, env: process.env },
          (err2, stdout2) => {
            if (!err2 && stdout2 && stdout2.includes("\0")) {
              mergeAndResolve(stdout2);
            } else {
              resolve({ ...process.env });
            }
          },
        );
      },
    );
  });

  return cachedLoginEnvPromise;
}

/**
 * Probe whether a DSH WebUI URL is currently alive AND accepts auth
 * (200 OK for open/authenticated endpoints, 303/302 for valid `?token=...` exchange).
 */
export function probeDshUrl(url: string, timeoutMs = 1500): Promise<boolean> {
  return new Promise((resolve) => {
    let settled = false;
    const done = (ok: boolean) => {
      if (settled) return;
      settled = true;
      resolve(ok);
    };

    try {
      const parsed = new URL(url);
      const client = parsed.protocol === "https:" ? https : http;
      const req = client.request(
        parsed,
        {
          method: "GET",
          timeout: timeoutMs,
          rejectUnauthorized: false,
        },
        (res) => {
          const code = res.statusCode ?? 0;
          res.resume();
          done(code === 200 || code === 302 || code === 303);
        },
      );
      req.on("timeout", () => {
        req.destroy();
        done(false);
      });
      req.on("error", () => done(false));
      req.end();
    } catch {
      done(false);
    }
  });
}

function allocateLocalPort(preferredPort = 0): Promise<number> {
  const tryPort = (port: number): Promise<number> =>
    new Promise((resolve, reject) => {
      const srv = net.createServer();
      srv.unref();
      srv.once("error", reject);
      srv.listen(port, "127.0.0.1", () => {
        const addr = srv.address();
        const bound = typeof addr === "object" && addr ? addr.port : 0;
        srv.close(() => resolve(bound));
      });
    });

  if (preferredPort > 0) {
    return tryPort(preferredPort).catch(() => tryPort(0));
  }
  return tryPort(0);
}

function waitForLocalTcpPort(
  port: number,
  proc: ChildProcess,
  timeoutMs = 12_000,
): Promise<void> {
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
      finish(classifySshFailure(stderrBuf, `远程 DSH 端口转发已退出 (code ${code ?? "unknown"})`));
    });
    proc.once("error", (err) => finish(err));

    const poller = setInterval(() => {
      if (Date.now() - started > timeoutMs) {
        finish(classifySshFailure(stderrBuf, "建立远程 DSH 端口转发超时"));
        return;
      }
      const sock = net.createConnection({ host: "127.0.0.1", port });
      sock.setTimeout(400);
      sock.once("connect", () => {
        sock.destroy();
        finish();
      });
      sock.once("error", () => sock.destroy());
      sock.once("timeout", () => sock.destroy());
    }, 120);
  });
}

export function extractDshWebUrl(rawText: string): string | null {
  const clean = rawText.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, "");
  for (const line of clean.split(/\r?\n/)) {
    const tagged = line.match(DSH_WEB_LINE_RE);
    if (tagged?.[1]) return tagged[1];
    const loopback = line.match(LOOPBACK_URL_RE);
    if (loopback?.[0]) return loopback[0];
  }
  return null;
}

export function normalizeCustomDshUrl(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) return "";
  if (/^https?:\/\//i.test(trimmed)) return trimmed;
  if (/^[a-zA-Z0-9.-]+(?::\d+)?(?:\/.*)?$/.test(trimmed)) {
    return `http://${trimmed}`;
  }
  throw new Error("请输入有效的 http:// 或 https:// DeepSeek Harness WebUI 地址");
}

function resolveDshCommand(env: NodeJS.ProcessEnv): { command: string; prefixArgs: string[] } {
  const envBin = env.DSH_BIN?.trim() || process.env.DSH_BIN?.trim();
  if (envBin && fs.existsSync(envBin)) {
    return { command: envBin, prefixArgs: [] };
  }

  const pathEnv = env.PATH || process.env.PATH || "";
  for (const dir of pathEnv.split(path.delimiter)) {
    if (!dir) continue;
    const candidate = path.join(dir, "dsh");
    try {
      if (fs.statSync(candidate).isFile()) {
        return { command: candidate, prefixArgs: [] };
      }
    } catch {
      // continue
    }
  }

  const npxRoot = path.join(os.homedir(), ".npm", "_npx");
  try {
    if (fs.existsSync(npxRoot)) {
      const candidates: Array<{ file: string; mtime: number }> = [];
      for (const entry of fs.readdirSync(npxRoot)) {
        const candidate = path.join(npxRoot, entry, "node_modules", ".bin", "dsh");
        try {
          const st = fs.statSync(candidate);
          if (st.isFile()) {
            candidates.push({ file: candidate, mtime: st.mtimeMs });
          }
        } catch {
          // continue
        }
      }
      candidates.sort((a, b) => b.mtime - a.mtime);
      if (candidates.length > 0) {
        return { command: candidates[0].file, prefixArgs: [] };
      }
    }
  } catch {
    // ignore
  }

  const npxCmd = process.platform === "win32" ? "npx.cmd" : "npx";
  return { command: npxCmd, prefixArgs: ["-y", "@deepseek-ai/dsh"] };
}

export function buildDshWebArgs(prefixArgs: readonly string[], minimal = false): string[] {
  return minimal
    ? [...prefixArgs, "web"]
    : [...prefixArgs, "web", "--host", "127.0.0.1", "--port", "0"];
}

/**
 * Build POSIX shell script executed over SSH on a remote machine to detect an existing
 * Muxlane-managed `dsh web` daemon or start one cleanly on the remote machine.
 */
export function buildRemoteDshScript(
  remoteProjectPath: string,
  preferredPort: number,
  forceRestart: boolean,
): string {
  const qCwd = shQuote(remoteProjectPath || "");
  const restartFlag = forceRestart ? "1" : "0";
  return [
    'set -u',
    'NVM_LATEST=""',
    'if [ -d "$HOME/.nvm/versions/node" ]; then NVM_LATEST="$(ls -1 "$HOME/.nvm/versions/node" 2>/dev/null | sort -V | tail -n 1)"; fi',
    'if [ -n "$NVM_LATEST" ]; then PATH="$HOME/.nvm/versions/node/$NVM_LATEST/bin:$PATH"; fi',
    'export PATH="$HOME/.local/bin:$HOME/.bun/bin:$HOME/.bin:/usr/local/bin:/usr/bin:/bin:$PATH"',
    'DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/muxlane"',
    'mkdir -p "$DATA_DIR"',
    'PID_FILE="$DATA_DIR/dsh-web.pid"',
    'LOG_FILE="$DATA_DIR/dsh-web.log"',
    'extract_url() {',
    '  [ -f "$LOG_FILE" ] || return 1',
    '  tr -d "\\r" < "$LOG_FILE" | sed -E "s/\\x1b\\[[0-9;]*[a-zA-Z]//g" | grep -Eo "https?://(127\\.0\\.0\\.1|localhost|\\[::1\\]):[0-9]+[^[:space:]\\"\\x27<>]*" | head -n 1',
    '}',
    `if [ "${restartFlag}" = "1" ]; then`,
    '  if [ -f "$PID_FILE" ]; then OLD="$(cat "$PID_FILE" 2>/dev/null || true)"; [ -n "$OLD" ] && kill "$OLD" 2>/dev/null || true; fi',
    '  rm -f "$PID_FILE" "$LOG_FILE"',
    'fi',
    'if [ -f "$PID_FILE" ]; then',
    '  OLD="$(cat "$PID_FILE" 2>/dev/null || true)"',
    '  if [ -n "$OLD" ] && kill -0 "$OLD" 2>/dev/null; then',
    '    URL="$(extract_url || true)"',
    '    if [ -n "$URL" ]; then printf "DSH_URL\\t%s\\n" "$URL"; exit 0; fi',
    '  fi',
    'fi',
    `WORK_DIR=${qCwd}`,
    '[ -n "$WORK_DIR" ] && [ -d "$WORK_DIR" ] || WORK_DIR="$HOME"',
    'cd "$WORK_DIR" || cd "$HOME"',
    'DSH_CMD=""',
    'if [ -n "${DSH_BIN:-}" ] && [ -x "$DSH_BIN" ]; then DSH_CMD="$DSH_BIN"; fi',
    'if [ -z "$DSH_CMD" ] && command -v dsh >/dev/null 2>&1; then DSH_CMD="$(command -v dsh)"; fi',
    'if [ -z "$DSH_CMD" ] && [ -d "$HOME/.npm/_npx" ]; then',
    '  DSH_CMD="$(ls -1t "$HOME"/.npm/_npx/*/node_modules/.bin/dsh 2>/dev/null | head -n 1 || true)"',
    'fi',
    'start_dsh() {',
    '  rm -f "$LOG_FILE" "$PID_FILE"',
    '  if [ -n "$DSH_CMD" ]; then',
    '    NO_COLOR=1 BROWSER=none SSH_CONNECTION="127.0.0.1 1 127.0.0.1 22" nohup "$DSH_CMD" "$@" >"$LOG_FILE" 2>&1 </dev/null &',
    '  else',
    '    if ! command -v npx >/dev/null 2>&1; then echo "远程主机未安装 dsh 或 npx/node" >&2; exit 127; fi',
    '    NO_COLOR=1 BROWSER=none SSH_CONNECTION="127.0.0.1 1 127.0.0.1 22" nohup npx -y @deepseek-ai/dsh "$@" >"$LOG_FILE" 2>&1 </dev/null &',
    '  fi',
    '  DPID=$!',
    '  echo "$DPID" > "$PID_FILE"',
    '  i=0',
    '  while [ "$i" -lt 120 ]; do',
    '    URL="$(extract_url || true)"',
    '    if [ -n "$URL" ]; then printf "DSH_URL\\t%s\\n" "$URL"; return 0; fi',
    '    if ! kill -0 "$DPID" 2>/dev/null; then return 1; fi',
    '    sleep 0.25',
    '    i=$((i + 1))',
    '  done',
    '  return 2',
    '}',
    `if start_dsh web --host 127.0.0.1 --port ${preferredPort} --trusted-host 127.0.0.1:${preferredPort}; then exit 0; fi`,
    `if start_dsh web --host 127.0.0.1 --port 0 --trusted-host 127.0.0.1:${preferredPort}; then exit 0; fi`,
    'if grep -qi "unknown option" "$LOG_FILE" 2>/dev/null; then',
    '  if start_dsh web; then exit 0; fi',
    'fi',
    'tail -c 400 "$LOG_FILE" 2>/dev/null >&2 || true',
    'exit 1',
  ].join("\n");
}

/**
 * Host-aware machine-level singleton manager for DeepSeek Harness (`dsh web`).
 * - On local host: detects a running local `dsh web` or starts ONE shared local instance.
 * - On remote SSH host: detects or starts ONE shared `dsh web` ON THE REMOTE MACHINE and
 *   forwards it over SSH to a local loopback port for Electron's `<webview>`.
 */
export class DshWebManager {
  private byHost = new Map<string, SharedHostDsh>();
  private inflight = new Map<string, Promise<SharedHostDsh>>();

  list(): DshInstanceInfo[] {
    return Array.from(this.byHost.values()).map((inst) => ({
      projectId: inst.hostId,
      projectPath: inst.cwd,
      url: inst.url,
      status: "ready",
      error: null,
      isCustomUrl: inst.isCustomUrl,
    }));
  }

  async open(
    projectId: string,
    projectPath: string,
    hostContext: HostConnectionContext,
    customUrl?: string,
  ): Promise<DshInstanceInfo> {
    const hostId = hostContext.host.id || LOCAL_HOST_ID;
    const normalizedCustom = customUrl ? normalizeCustomDshUrl(customUrl) : "";
    if (normalizedCustom) {
      this.killHostManaged(hostId);
      const custom: SharedHostDsh = {
        hostId,
        proc: null,
        url: normalizedCustom,
        cwd: projectPath,
        isCustomUrl: true,
      };
      this.byHost.set(hostId, custom);
      writePersistedDshForHost(hostId, { url: normalizedCustom, isCustomUrl: true });
      return this.toProjectInfo(projectId, projectPath, custom);
    }

    const inst = await this.ensureRunningForHost(hostContext, projectPath);
    return this.toProjectInfo(projectId, projectPath, inst);
  }

  async restart(
    projectId: string,
    projectPath: string,
    hostContext: HostConnectionContext,
  ): Promise<DshInstanceInfo> {
    const hostId = hostContext.host.id || LOCAL_HOST_ID;
    this.killHostManaged(hostId);
    writePersistedDshForHost(hostId, null);

    if (hostContext.host.kind === "ssh") {
      const inst = await this.ensureRemoteDsh(hostContext, projectPath, true);
      return this.toProjectInfo(projectId, projectPath, inst);
    }

    const inst = await this.startNewLocalShared(projectPath);
    return this.toProjectInfo(projectId, projectPath, inst);
  }

  /**
   * Closing a project tab keeps the machine-level `dsh web` warm for instant reuse.
   */
  stop(_projectId: string): void {
    // Keep shared per-host dsh web alive until restart() or dispose().
  }

  dispose(): void {
    for (const hostId of Array.from(this.byHost.keys())) {
      this.killHostManaged(hostId);
    }
  }

  private toProjectInfo(
    projectId: string,
    projectPath: string,
    inst: SharedHostDsh,
  ): DshInstanceInfo {
    return {
      projectId,
      projectPath,
      url: inst.url,
      status: "ready",
      error: null,
      isCustomUrl: inst.isCustomUrl,
    };
  }

  private killHostManaged(hostId: string): void {
    const existing = this.byHost.get(hostId);
    if (!existing) return;
    this.byHost.delete(hostId);
    if (existing.proc) {
      try {
        existing.proc.kill("SIGTERM");
      } catch {
        // ignore
      }
    }
    try {
      existing.cleanupSecret?.();
    } catch {
      // ignore
    }
  }

  private async ensureRunningForHost(
    hostContext: HostConnectionContext,
    projectPath: string,
  ): Promise<SharedHostDsh> {
    const hostId = hostContext.host.id || LOCAL_HOST_ID;
    const existing = this.byHost.get(hostId);
    if (existing) {
      const procAlive = !existing.proc || existing.proc.exitCode === null;
      if (procAlive && (await probeDshUrl(existing.url))) {
        return existing;
      }
      this.killHostManaged(hostId);
    }

    const pending = this.inflight.get(hostId);
    if (pending) return pending;

    const promise = (
      hostContext.host.kind === "ssh"
        ? this.ensureRemoteDsh(hostContext, projectPath, false)
        : this.detectOrStartLocal(projectPath)
    ).finally(() => {
      this.inflight.delete(hostId);
    });
    this.inflight.set(hostId, promise);
    return promise;
  }

  private async ensureRemoteDsh(
    hostContext: HostConnectionContext,
    projectPath: string,
    forceRestart: boolean,
  ): Promise<SharedHostDsh> {
    const hostId = hostContext.host.id;
    const destination = hostContext.destination;
    if (!destination) {
      throw new Error("远程主机 SSH 目标无效");
    }

    // Check if user saved a direct custom URL for this remote host (and not forceRestart)
    if (!forceRestart) {
      const saved = readPersistedDshForHost(hostId);
      if (saved?.isCustomUrl && (await probeDshUrl(saved.url))) {
        const reused: SharedHostDsh = {
          hostId,
          proc: null,
          url: saved.url,
          cwd: projectPath,
          isCustomUrl: true,
        };
        this.byHost.set(hostId, reused);
        return reused;
      }
    }

    const preferredPort = await allocateLocalPort(0);
    const script = buildRemoteDshScript(projectPath, preferredPort, forceRestart);
    const { stdout, stderr } = await execSshScript(
      hostContext.host,
      hostContext.password,
      destination,
      script,
      35_000,
    );

    let remoteUrl = "";
    for (const line of stdout.split(/\r?\n/)) {
      if (line.startsWith("DSH_URL\t")) {
        remoteUrl = line.slice("DSH_URL\t".length).trim();
        break;
      }
    }
    if (!remoteUrl) {
      remoteUrl = extractDshWebUrl(stdout) ?? "";
    }
    if (!remoteUrl) {
      throw new Error(
        `远程主机 (${hostContext.host.name}) 启动 DSH WebUI 失败: ${
          stderr.trim().slice(-300) || stdout.trim().slice(-300) || "未返回 WebUI 地址"
        }`,
      );
    }

    const parsed = new URL(remoteUrl);
    const remoteHost = parsed.hostname || "127.0.0.1";
    const remotePort = Number(parsed.port || (parsed.protocol === "https:" ? 443 : 80));
    if (!Number.isFinite(remotePort) || remotePort <= 0) {
      throw new Error(`无法解析远程 DSH 端口: ${remoteUrl}`);
    }

    // Prefer binding the exact same port locally (so Host header matches natively even without --trusted-host).
    const localPort = await allocateLocalPort(remotePort === preferredPort ? preferredPort : remotePort);
    const forwardSpec = `127.0.0.1:${localPort}:${remoteHost}:${remotePort}`;

    const { proc, cleanupSecret } = spawnSshProcess(
      hostContext.host,
      hostContext.password,
      [
        ...sharedMasterArgs(destination),
        "-N",
        "-o",
        "ExitOnForwardFailure=yes",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=3",
        "-L",
        forwardSpec,
        destination,
      ],
      ["ignore", "ignore", "pipe"],
    );

    try {
      await waitForLocalTcpPort(localPort, proc, 12_000);
    } catch (err) {
      try {
        proc.kill("SIGTERM");
      } catch {
        // ignore
      }
      cleanupSecret();
      throw err;
    }

    const localUrl = `${parsed.protocol}//127.0.0.1:${localPort}${parsed.pathname}${parsed.search}`;
    const record: SharedHostDsh = {
      hostId,
      proc,
      cleanupSecret,
      url: localUrl,
      cwd: projectPath,
      isCustomUrl: false,
    };
    proc.once("exit", () => {
      if (this.byHost.get(hostId) === record) {
        this.byHost.delete(hostId);
      }
    });
    this.byHost.set(hostId, record);
    return record;
  }

  private async detectOrStartLocal(projectPath: string): Promise<SharedHostDsh> {
    const shellEnv = await loadLoginShellEnv();

    const candidates: Array<{ url: string; isCustomUrl: boolean }> = [];
    const saved = readPersistedDshForHost(LOCAL_HOST_ID);
    if (saved) {
      candidates.push({ url: saved.url, isCustomUrl: saved.isCustomUrl === true });
    }
    const envUrl = shellEnv.DSH_WEB_URL?.trim() || process.env.DSH_WEB_URL?.trim();
    if (envUrl && /^https?:\/\//i.test(envUrl)) {
      candidates.push({ url: envUrl, isCustomUrl: false });
    }
    for (const defUrl of DEFAULT_LOCAL_CANDIDATES) {
      candidates.push({ url: defUrl, isCustomUrl: false });
    }

    for (const cand of candidates) {
      if (await probeDshUrl(cand.url)) {
        const reused: SharedHostDsh = {
          hostId: LOCAL_HOST_ID,
          proc: null,
          url: cand.url,
          cwd: projectPath,
          isCustomUrl: cand.isCustomUrl,
        };
        this.byHost.set(LOCAL_HOST_ID, reused);
        return reused;
      }
    }

    return this.startNewLocalShared(projectPath, shellEnv);
  }

  private async startNewLocalShared(
    projectPath: string,
    preloadedEnv?: NodeJS.ProcessEnv,
  ): Promise<SharedHostDsh> {
    const cwd = fs.existsSync(projectPath) ? projectPath : os.homedir();
    const shellEnv = preloadedEnv ?? (await loadLoginShellEnv());
    const { command, prefixArgs } = resolveDshCommand(shellEnv);

    try {
      return await this.spawnLocalOnce(cwd, command, buildDshWebArgs(prefixArgs, false), shellEnv);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      if (/unknown option/i.test(msg)) {
        return await this.spawnLocalOnce(cwd, command, buildDshWebArgs(prefixArgs, true), shellEnv);
      }
      throw err;
    }
  }

  private async spawnLocalOnce(
    cwd: string,
    command: string,
    args: string[],
    shellEnv: NodeJS.ProcessEnv,
  ): Promise<SharedHostDsh> {
    const spawnEnv: NodeJS.ProcessEnv = {
      ...shellEnv,
      NO_COLOR: "1",
      // Suppress dsh's automatic external browser launch without passing `--no-open`
      SSH_CONNECTION: shellEnv.SSH_CONNECTION || "127.0.0.1 1 127.0.0.1 22",
      BROWSER: "none",
    };
    delete spawnEnv.FORCE_COLOR;
    delete spawnEnv.ELECTRON_RUN_AS_NODE;

    const proc = spawn(command, args, {
      cwd,
      stdio: ["ignore", "pipe", "pipe"],
      env: spawnEnv,
    });

    const url = await new Promise<string>((resolve, reject) => {
      let settled = false;
      let logTail = "";
      let lineBuf = "";

      const finish = (err?: Error, foundUrl?: string) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        if (err) reject(err);
        else resolve(foundUrl || "");
      };

      const onData = (chunk: Buffer) => {
        const text = chunk.toString("utf8");
        logTail = (logTail + text).slice(-4000);
        lineBuf += text;
        const found = extractDshWebUrl(lineBuf);
        if (found) {
          finish(undefined, found);
          return;
        }
        const lastNl = lineBuf.lastIndexOf("\n");
        if (lastNl >= 0) {
          lineBuf = lineBuf.slice(lastNl + 1);
        }
      };

      const timer = setTimeout(() => {
        try {
          proc.kill("SIGTERM");
        } catch {
          // ignore
        }
        finish(
          new Error(
            `启动 DeepSeek Harness WebUI 超时 (${command})。输出：${logTail.trim().slice(-400) || "(无输出)"}`,
          ),
        );
      }, 30_000);

      proc.stdout?.on("data", onData);
      proc.stderr?.on("data", onData);
      proc.once("error", (err) => {
        finish(new Error(`无法启动 dsh (${command}): ${err.message}`));
      });
      proc.once("exit", (code) => {
        if (this.byHost.get(LOCAL_HOST_ID)?.proc === proc) {
          this.byHost.delete(LOCAL_HOST_ID);
        }
        finish(
          new Error(
            `DeepSeek Harness WebUI 进程已退出 (code ${code ?? "unknown"}): ${
              logTail.trim().slice(-400) || "请确认已安装 @deepseek-ai/dsh"
            }`,
          ),
        );
      });
    });

    const record: SharedHostDsh = {
      hostId: LOCAL_HOST_ID,
      proc,
      url,
      cwd,
      isCustomUrl: false,
    };
    this.byHost.set(LOCAL_HOST_ID, record);
    writePersistedDshForHost(LOCAL_HOST_ID, { url, isCustomUrl: false });
    return record;
  }
}
