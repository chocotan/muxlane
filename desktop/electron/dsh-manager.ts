import { execFile, spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import https from "node:https";
import os from "node:os";
import path from "node:path";
import type { DshInstanceInfo } from "../src/shared/protocol";

interface SharedDshProcess {
  proc: ChildProcess | null;
  url: string;
  cwd: string;
  isCustomUrl: boolean;
}

interface PersistedDshState {
  url: string;
  isCustomUrl?: boolean;
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

function readPersistedDsh(): PersistedDshState | null {
  try {
    const raw = fs.readFileSync(persistedDshFile(), "utf8");
    const parsed = JSON.parse(raw) as Partial<PersistedDshState>;
    if (typeof parsed.url === "string" && /^https?:\/\//i.test(parsed.url)) {
      return { url: parsed.url, isCustomUrl: parsed.isCustomUrl === true };
    }
  } catch {
    // ignore missing/corrupt cache
  }
  return null;
}

function writePersistedDsh(state: PersistedDshState | null): void {
  try {
    const file = persistedDshFile();
    if (!state) {
      if (fs.existsSync(file)) fs.unlinkSync(file);
      return;
    }
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, JSON.stringify(state, null, 2), { mode: 0o600 });
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
 * (200 OK for open/authenticated endpoints, 303 See Other for valid `?token=...` exchange).
 * Returns false on connection refused, timeout, or 401 Unauthorized.
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
          // 200 OK or 303/302 redirect (valid token bootstrap) means ready & usable.
          // 401 means a different process with an unknown secret token is listening.
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
 * Machine-level singleton manager for DeepSeek Harness (`dsh web`).
 * Detects if a usable `dsh web` server is already running on the machine and reuses it;
 * otherwise starts ONE shared `dsh web` process and shares it across all project tabs.
 */
export class DshWebManager {
  private shared: SharedDshProcess | null = null;
  private inflight: Promise<SharedDshProcess> | null = null;

  list(): DshInstanceInfo[] {
    if (!this.shared) return [];
    return [
      {
        projectId: "shared",
        projectPath: this.shared.cwd,
        url: this.shared.url,
        status: "ready",
        error: null,
        isCustomUrl: this.shared.isCustomUrl,
      },
    ];
  }

  async open(
    projectId: string,
    projectPath: string,
    customUrl?: string,
  ): Promise<DshInstanceInfo> {
    const normalizedCustom = customUrl ? normalizeCustomDshUrl(customUrl) : "";
    if (normalizedCustom) {
      this.killManagedProcess();
      this.shared = {
        proc: null,
        url: normalizedCustom,
        cwd: projectPath,
        isCustomUrl: true,
      };
      writePersistedDsh({ url: normalizedCustom, isCustomUrl: true });
      return this.toProjectInfo(projectId, projectPath, this.shared);
    }

    const inst = await this.ensureRunning(projectPath);
    return this.toProjectInfo(projectId, projectPath, inst);
  }

  async restart(projectId: string, projectPath: string): Promise<DshInstanceInfo> {
    this.killManagedProcess();
    this.shared = null;
    writePersistedDsh(null);
    const inst = await this.startNewShared(projectPath);
    return this.toProjectInfo(projectId, projectPath, inst);
  }

  /**
   * Closing a project's DSH tab keeps the machine-level `dsh web` running for instant reuse.
   */
  stop(_projectId: string): void {
    // Keep shared machine-level dsh web warm; call restart() or dispose() to stop it.
  }

  dispose(): void {
    this.killManagedProcess();
    this.shared = null;
  }

  private toProjectInfo(
    projectId: string,
    projectPath: string,
    inst: SharedDshProcess,
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

  private killManagedProcess(): void {
    const proc = this.shared?.proc;
    if (!proc) return;
    this.shared = null;
    try {
      proc.kill("SIGTERM");
    } catch {
      // ignore
    }
  }

  private async ensureRunning(projectPath: string): Promise<SharedDshProcess> {
    // 1. Reuse in-memory shared instance if still alive and reachable
    if (this.shared) {
      const procAlive = !this.shared.proc || this.shared.proc.exitCode === null;
      if (procAlive && (await probeDshUrl(this.shared.url))) {
        return this.shared;
      }
      this.killManagedProcess();
      this.shared = null;
    }

    if (this.inflight) return this.inflight;

    const promise = this.detectOrStart(projectPath).finally(() => {
      this.inflight = null;
    });
    this.inflight = promise;
    return promise;
  }

  private async detectOrStart(projectPath: string): Promise<SharedDshProcess> {
    const shellEnv = await loadLoginShellEnv();

    // 2. Detect already-running dsh web on this machine:
    //    (a) persisted URL from previous Muxlane run or user-configured custom URL
    //    (b) DSH_WEB_URL environment variable
    //    (c) default local http://127.0.0.1:3080/ (if auth-free)
    const candidates: Array<{ url: string; isCustomUrl: boolean }> = [];
    const saved = readPersistedDsh();
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
        const reused: SharedDshProcess = {
          proc: null,
          url: cand.url,
          cwd: projectPath,
          isCustomUrl: cand.isCustomUrl,
        };
        this.shared = reused;
        return reused;
      }
    }

    // 3. No usable running dsh detected -> start one shared local dsh web instance
    return this.startNewShared(projectPath, shellEnv);
  }

  private async startNewShared(
    projectPath: string,
    preloadedEnv?: NodeJS.ProcessEnv,
  ): Promise<SharedDshProcess> {
    const cwd = fs.existsSync(projectPath) ? projectPath : os.homedir();
    const shellEnv = preloadedEnv ?? (await loadLoginShellEnv());
    const { command, prefixArgs } = resolveDshCommand(shellEnv);

    try {
      return await this.spawnOnce(cwd, command, buildDshWebArgs(prefixArgs, false), shellEnv);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      if (/unknown option/i.test(msg)) {
        return await this.spawnOnce(cwd, command, buildDshWebArgs(prefixArgs, true), shellEnv);
      }
      throw err;
    }
  }

  private async spawnOnce(
    cwd: string,
    command: string,
    args: string[],
    shellEnv: NodeJS.ProcessEnv,
  ): Promise<SharedDshProcess> {
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
        if (this.shared?.proc === proc) {
          this.shared = null;
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

    const record: SharedDshProcess = {
      proc,
      url,
      cwd,
      isCustomUrl: false,
    };
    this.shared = record;
    writePersistedDsh({ url, isCustomUrl: false });
    return record;
  }
}
