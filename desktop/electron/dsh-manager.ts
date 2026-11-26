import { spawn, type ChildProcess } from "node:child_process";
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

const URL_LINE_RE = /https?:\/\/[^\s"'<>]+/i;

function resolveDshCommand(): { command: string; prefixArgs: string[] } {
  const envBin = process.env.DSH_BIN?.trim();
  if (envBin && fs.existsSync(envBin)) {
    return { command: envBin, prefixArgs: [] };
  }

  // Search PATH for `dsh`
  const pathEnv = process.env.PATH || "";
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
    const trimmedCustom = customUrl?.trim();
    if (trimmedCustom) {
      if (!/^https?:\/\//i.test(trimmedCustom)) {
        throw new Error("请输入有效的 http:// 或 https:// DeepSeek Harness WebUI 地址");
      }
      this.stop(projectId);
      const custom: ManagedDshProcess = {
        projectId,
        projectPath,
        proc: null,
        url: trimmedCustom,
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
    const { command, prefixArgs } = resolveDshCommand();
    const args = [...prefixArgs, "--profile", "web", "--no-open", "--port", "0"];

    const proc = spawn(command, args, {
      cwd,
      stdio: ["ignore", "pipe", "pipe"],
      env: {
        ...process.env,
        NO_COLOR: "1",
      },
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
        // Strip ANSI escape codes if any
        const clean = text.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, "");
        for (const line of clean.split(/\r?\n/)) {
          const match = line.match(URL_LINE_RE);
          if (match) {
            finish(undefined, match[0]);
            return;
          }
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
            `启动 DeepSeek Harness WebUI 超时。输出：${logTail.trim().slice(-300) || "(无输出)"}`,
          ),
        );
      }, 25_000);

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
              logTail.trim().slice(-300) || "请确认已安装 @deepseek-ai/dsh"
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
