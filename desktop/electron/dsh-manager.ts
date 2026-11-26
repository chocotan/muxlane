import { execFile, spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import type { DshInstanceInfo } from "../src/shared/protocol";

interface ManagedDshProcess {
  projectId: string;
  projectPath: string;
  proc: ChildProcess | null;
  url: string;
  status: "starting" | "ready" | "error";
  error: string | null;
  isCustomUrl: boolean;
}

const DSH_WEB_LINE_RE = /dsh\s+web:\s*(https?:\/\/[^\s"'<>]+)/i;
const LOOPBACK_URL_RE = /https?:\/\/(?:127\.0\.0\.1|localhost|\[::1\]):\d+[^\s"'<>]*/i;

let cachedLoginEnvPromise: Promise<NodeJS.ProcessEnv> | null = null;

/**
 * Resolve the user's interactive login shell environment (PATH, nvm node, http_proxy,
 * API keys, etc.) so `dsh` spawned from a systemd-started Electron process has the
 * same runtime environment as a normal terminal.
 */
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
        // Never overwrite Electron/session critical vars if shell left them blank
        if (key === "PWD" || key === "SHLVL" || key === "_") continue;
        merged[key] = val;
      }
      resolve(merged);
    };

    // Try interactive login shell first (loads ~/.bashrc / ~/.zshrc where nvm, proxies, and API keys live).
    execFile(
      shell,
      ["-ilc", "env -0"],
      { timeout: 3000, maxBuffer: 4 * 1024 * 1024, env: process.env },
      (err, stdout) => {
        if (!err && stdout && stdout.includes("\0")) {
          mergeAndResolve(stdout);
          return;
        }
        // Fallback to non-interactive login shell.
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

  // Search PATH (including login shell PATH: nvm, bun, ~/.local/bin, etc.) for `dsh`
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

  // Search ~/.npm/_npx/*/node_modules/.bin/dsh (newest first, e.g. already cached npx install)
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

  // Fallback to npx
  return { command: "npx", prefixArgs: ["-y", "@deepseek-ai/dsh"] };
}

function toInfo(inst: ManagedDshProcess): DshInstanceInfo {
  return {
    projectId: inst.projectId,
    projectPath: inst.projectPath,
    url: inst.url,
    status: inst.status,
    error: inst.error,
    isCustomUrl: inst.isCustomUrl,
  };
}

export class DshWebManager {
  private instances = new Map<string, ManagedDshProcess>();
  private inflight = new Map<string, Promise<DshInstanceInfo>>();

  list(): DshInstanceInfo[] {
    return Array.from(this.instances.values()).map(toInfo);
  }

  async open(
    projectId: string,
    projectPath: string,
    customUrl?: string,
  ): Promise<DshInstanceInfo> {
    const normalizedCustom = customUrl ? normalizeCustomDshUrl(customUrl) : "";
    if (normalizedCustom) {
      this.stop(projectId);
      const custom: ManagedDshProcess = {
        projectId,
        projectPath,
        proc: null,
        url: normalizedCustom,
        status: "ready",
        error: null,
        isCustomUrl: true,
      };
      this.instances.set(projectId, custom);
      return toInfo(custom);
    }

    const existing = this.instances.get(projectId);
    if (existing && existing.status === "ready") {
      if (existing.isCustomUrl || (existing.proc && existing.proc.exitCode === null)) {
        return toInfo(existing);
      }
      this.instances.delete(projectId);
    }

    const pending = this.inflight.get(projectId);
    if (pending) return pending;

    const promise = this.spawnForProject(projectId, projectPath).finally(() => {
      this.inflight.delete(projectId);
    });
    this.inflight.set(projectId, promise);
    return promise;
  }

  async restart(projectId: string, projectPath: string): Promise<DshInstanceInfo> {
    this.stop(projectId);
    return this.open(projectId, projectPath);
  }

  stop(projectId: string): void {
    const existing = this.instances.get(projectId);
    if (!existing) return;
    this.instances.delete(projectId);
    if (existing.proc) {
      try {
        existing.proc.kill("SIGTERM");
      } catch {
        // ignore
      }
    }
  }

  dispose(): void {
    for (const projectId of Array.from(this.instances.keys())) {
      this.stop(projectId);
    }
  }

  private async spawnForProject(
    projectId: string,
    projectPath: string,
  ): Promise<DshInstanceInfo> {
    const cwd = fs.existsSync(projectPath) ? projectPath : os.homedir();
    const shellEnv = await loadLoginShellEnv();
    const { command, prefixArgs } = resolveDshCommand(shellEnv);
    const args = [...prefixArgs, "--profile", "web", "--no-open", "--host", "127.0.0.1", "--port", "0"];

    const spawnEnv: NodeJS.ProcessEnv = {
      ...shellEnv,
      NO_COLOR: "1",
    };
    delete spawnEnv.FORCE_COLOR;
    delete spawnEnv.ELECTRON_RUN_AS_NODE;

    const proc = spawn(command, args, {
      cwd,
      stdio: ["ignore", "pipe", "pipe"],
      env: spawnEnv,
    });

    const record: ManagedDshProcess = {
      projectId,
      projectPath: cwd,
      proc,
      url: "",
      status: "starting",
      error: null,
      isCustomUrl: false,
    };
    this.instances.set(projectId, record);

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
        // Keep only the incomplete tail line so URLs split across chunks still match
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
        if (this.instances.get(projectId) === record && record.status === "ready") {
          this.instances.delete(projectId);
        }
        finish(
          new Error(
            `DeepSeek Harness WebUI 进程已退出 (code ${code ?? "unknown"}): ${
              logTail.trim().slice(-400) || "请确认已安装 @deepseek-ai/dsh"
            }`,
          ),
        );
      });
    }).catch((err: Error) => {
      record.status = "error";
      record.error = err.message;
      this.instances.delete(projectId);
      throw err;
    });

    record.url = url;
    record.status = "ready";
    record.error = null;
    return toInfo(record);
  }
}
