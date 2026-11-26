import { app, BrowserWindow, dialog, ipcMain, Menu, nativeImage, shell } from "electron";
import { execFile } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";
import type {
  AddHostInput,
  RelayConfig,
  WorkspaceGitChanges,
  WorkspaceGitFile,
} from "../src/shared/protocol";
import { DshWebManager } from "./dsh-manager";
import { HostManager, readRelayConfig, writeRelayConfig } from "./host-manager";
import { MuxlaneSocketClient, resolveMuxlaneSocketPath } from "./muxlane-socket";

let mainWindow: BrowserWindow | null = null;
let rpcClient: MuxlaneSocketClient | null = null;
let hostManager: HostManager | null = null;
const dshManager = new DshWebManager();

function resolveAppIcon() {
  const iconPath = path.join(__dirname, "..", "assets", "muxlane-icon.png");
  const img = nativeImage.createFromPath(iconPath);
  return img.isEmpty() ? undefined : img;
}

function createWindow(): void {
  app.setName("Muxlane");
  if (process.platform !== "darwin") {
    Menu.setApplicationMenu(null);
  }
  const appIcon = resolveAppIcon();
  mainWindow = new BrowserWindow({
    width: 1440,
    height: 920,
    minWidth: 960,
    minHeight: 600,
    backgroundColor: "#0c0c0d",
    title: "Muxlane",
    icon: appIcon,
    autoHideMenuBar: true,
    webPreferences: {
      preload: path.join(__dirname, "preload.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
      webviewTag: true,
    },
  });
  if (process.platform !== "darwin") {
    mainWindow.removeMenu();
  }

  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    if (url.startsWith("http://") || url.startsWith("https://")) {
      void shell.openExternal(url);
    }
    return { action: "deny" };
  });

  mainWindow.webContents.on("did-attach-webview", (_event, webContents) => {
    webContents.setWindowOpenHandler(({ url }) => {
      if (url.startsWith("http://") || url.startsWith("https://")) {
        void shell.openExternal(url);
      }
      return { action: "deny" };
    });
  });

  const devUrl = process.env.MUXLANE_DEV_URL;
  if (devUrl) {
    void mainWindow.loadURL(devUrl);
  } else {
    void mainWindow.loadFile(path.join(__dirname, "..", "dist", "index.html"));
  }

  mainWindow.on("closed", () => {
    mainWindow = null;
  });
}

function setupRpcAndHosts(): void {
  const socketPath = resolveMuxlaneSocketPath();
  // `out/main.cjs` lives at `<repo>/desktop/out/main.cjs` -> repoRoot is `../..`
  const repoRoot = path.resolve(__dirname, "..", "..");
  rpcClient = new MuxlaneSocketClient(
    socketPath,
    repoRoot,
    (eventMsg) => {
      if (mainWindow && !mainWindow.isDestroyed()) {
        mainWindow.webContents.send("muxlane:event", eventMsg);
      }
    },
    (status) => {
      if (mainWindow && !mainWindow.isDestroyed()) {
        mainWindow.webContents.send("muxlane:status", status);
      }
    },
  );

  hostManager = new HostManager(
    (resolvedSocketPath, isLocal) => {
      rpcClient?.switchTarget(resolvedSocketPath, isLocal);
    },
    (hostsState) => {
      if (mainWindow && !mainWindow.isDestroyed()) {
        mainWindow.webContents.send("muxlane:hosts-changed", hostsState);
      }
    },
    repoRoot,
  );

  void hostManager.start().catch(() => {
    // Tunnel error is already surfaced via hostsState.tunnelStatus
  });
}

function setupIpc(): void {
  ipcMain.handle("muxlane:rpc", async (_ev, method: string, params: unknown) => {
    if (!rpcClient) throw new Error("Muxlane client not initialized");
    return rpcClient.call(method, params ?? {});
  });

  ipcMain.handle("muxlane:status", () => {
    return (
      rpcClient?.getStatus() ?? {
        connected: false,
        connecting: false,
        socketPath: resolveMuxlaneSocketPath(),
        error: null,
      }
    );
  });

  ipcMain.handle("muxlane:reconnect", async () => {
    if (!hostManager) {
      rpcClient?.reconnectNow();
      return;
    }
    const active = hostManager.getState().activeHostId;
    if (active === "local") {
      rpcClient?.reconnectNow();
    } else {
      await hostManager.selectHost(active).catch(() => {});
    }
  });

  ipcMain.handle("muxlane:pick-directory", async () => {
    if (!mainWindow) return null;
    const res = await dialog.showOpenDialog(mainWindow, {
      title: "选择项目目录",
      properties: ["openDirectory", "createDirectory"],
    });
    if (res.canceled || res.filePaths.length === 0) return null;
    return res.filePaths[0];
  });

  ipcMain.handle("muxlane:pick-file", async (_ev, title?: string) => {
    if (!mainWindow) return null;
    const res = await dialog.showOpenDialog(mainWindow, {
      title: title || "选择文件",
      properties: ["openFile", "showHiddenFiles"],
    });
    if (res.canceled || res.filePaths.length === 0) return null;
    return res.filePaths[0];
  });

  ipcMain.handle("muxlane:open-external", async (_ev, url: string) => {
    if (typeof url === "string" && (url.startsWith("http://") || url.startsWith("https://"))) {
      await shell.openExternal(url);
    }
  });

  ipcMain.handle("muxlane:hosts-list", () => {
    return (
      hostManager?.getState() ?? {
        activeHostId: "local",
        hosts: [],
        tunnelStatus: null,
      }
    );
  });

  ipcMain.handle("muxlane:hosts-add", async (_ev, input: AddHostInput) => {
    if (!hostManager) throw new Error("Host manager not initialized");
    return hostManager.addHost(input);
  });

  ipcMain.handle("muxlane:hosts-remove", async (_ev, hostId: string) => {
    if (!hostManager) throw new Error("Host manager not initialized");
    return hostManager.removeHost(hostId);
  });

  ipcMain.handle("muxlane:hosts-select", async (_ev, hostId: string) => {
    if (!hostManager) throw new Error("Host manager not initialized");
    return hostManager.selectHost(hostId);
  });

  ipcMain.handle("muxlane:relay-get", () => {
    return readRelayConfig();
  });

  ipcMain.handle("muxlane:relay-save", (_ev, cfg: RelayConfig) => {
    return writeRelayConfig(cfg);
  });

  ipcMain.handle("muxlane:git-changes", async (_ev, cwd: string) => {
    return readWorkspaceGitChanges(cwd);
  });

  ipcMain.handle(
    "muxlane:git-file-diff",
    async (_ev, cwd: string, filePath: string, status: string) => {
      return readWorkspaceGitFileDiff(cwd, filePath, status);
    },
  );

  ipcMain.handle(
    "muxlane:dsh-open",
    async (_ev, projectId: string, projectPath: string, customUrl?: string) => {
      return dshManager.open(projectId, projectPath, customUrl);
    },
  );

  ipcMain.handle("muxlane:dsh-restart", async (_ev, projectId: string, projectPath: string) => {
    return dshManager.restart(projectId, projectPath);
  });

  ipcMain.handle("muxlane:dsh-stop", (_ev, projectId: string) => {
    dshManager.stop(projectId);
  });

  ipcMain.handle("muxlane:dsh-list", () => {
    return dshManager.list();
  });
}

app.whenReady().then(() => {
  setupIpc();
  setupRpcAndHosts();
  createWindow();

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

app.on("window-all-closed", () => {
  rpcClient?.dispose();
  hostManager?.dispose();
  dshManager.dispose();
  if (process.platform !== "darwin") {
    app.quit();
  }
});

function runGit(cwd: string, args: string[], timeoutMs = 4000): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(
      "git",
      ["-c", "core.quotepath=false", "-C", cwd, ...args],
      { timeout: timeoutMs, maxBuffer: 4 * 1024 * 1024 },
      (err, stdout, stderr) => {
        if (err) {
          reject(new Error(stderr?.trim() || err.message));
          return;
        }
        resolve(stdout);
      },
    );
  });
}

function parseNumstat(raw: string, stats: Map<string, { additions: number; deletions: number }>): void {
  for (const line of raw.split("\n")) {
    if (!line.trim()) continue;
    const parts = line.split("\t");
    if (parts.length < 3) continue;
    const add = Number.parseInt(parts[0], 10);
    const del = Number.parseInt(parts[1], 10);
    const filePath = parts.slice(2).join("\t");
    const prev = stats.get(filePath) ?? { additions: 0, deletions: 0 };
    stats.set(filePath, {
      additions: prev.additions + (Number.isFinite(add) ? add : 0),
      deletions: prev.deletions + (Number.isFinite(del) ? del : 0),
    });
  }
}

async function readWorkspaceGitChanges(cwd: string): Promise<WorkspaceGitChanges> {
  if (!cwd) {
    return { isRepo: false, branch: null, files: [] };
  }
  try {
    const [statusOut, unstagedNum, stagedNum] = await Promise.all([
      runGit(cwd, ["status", "--porcelain=v1", "-uall", "--branch"]),
      runGit(cwd, ["diff", "--numstat"]).catch(() => ""),
      runGit(cwd, ["diff", "--cached", "--numstat"]).catch(() => ""),
    ]);

    const numstats = new Map<string, { additions: number; deletions: number }>();
    parseNumstat(unstagedNum, numstats);
    parseNumstat(stagedNum, numstats);

    let branch: string | null = null;
    const files: WorkspaceGitFile[] = [];

    for (const line of statusOut.split("\n")) {
      if (!line) continue;
      if (line.startsWith("## ")) {
        const rest = line.slice(3).trim();
        const dotIdx = rest.indexOf("...");
        branch = (dotIdx >= 0 ? rest.slice(0, dotIdx) : rest.split(" ")[0]) || null;
        continue;
      }
      if (line.length < 4) continue;
      const x = line[0];
      const y = line[1];
      let filePath = line.slice(3);
      const renameArrow = filePath.indexOf(" -> ");
      if (renameArrow >= 0) {
        filePath = filePath.slice(renameArrow + 4);
      }
      if (filePath.startsWith('"') && filePath.endsWith('"')) {
        filePath = filePath.slice(1, -1);
      }

      const untracked = x === "?" && y === "?";
      const staged = !untracked && x !== " ";
      const unstaged = !untracked && y !== " ";
      const status = untracked
        ? "?"
        : x === "U" || y === "U"
          ? "U"
          : x === "R" || y === "R"
            ? "R"
            : x === "D" || y === "D"
              ? "D"
              : x === "A" || y === "A"
                ? "A"
                : "M";

      const stat = numstats.get(filePath);
      files.push({
        path: filePath,
        status,
        staged,
        unstaged,
        additions: stat?.additions ?? 0,
        deletions: stat?.deletions ?? 0,
      });
    }

    return { isRepo: true, branch, files };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    if (/not a git repository/i.test(message)) {
      return { isRepo: false, branch: null, files: [] };
    }
    return { isRepo: false, branch: null, files: [], error: message };
  }
}

async function readWorkspaceGitFileDiff(
  cwd: string,
  filePath: string,
  status: string,
): Promise<string> {
  if (!cwd || !filePath) return "";
  if (status === "?") {
    const resolvedRoot = path.resolve(cwd);
    const resolvedFile = path.resolve(resolvedRoot, filePath);
    if (!resolvedFile.startsWith(resolvedRoot + path.sep) && resolvedFile !== resolvedRoot) {
      return "";
    }
    try {
      const buf = await fs.readFile(resolvedFile);
      const slice = buf.subarray(0, 120 * 1024);
      if (slice.includes(0)) {
        return "[二进制文件，无法预览差异]";
      }
      const lines = slice.toString("utf8").split("\n");
      const header = `@@ -0,0 +1,${lines.length} @@`;
      const body = lines.map((l) => `+${l}`).join("\n");
      return `${header}\n${body}${buf.length > slice.length ? "\n…（文件过大，已截断）" : ""}`;
    } catch {
      return "";
    }
  }
  try {
    const diff = await runGit(cwd, ["diff", "--no-color", "HEAD", "--", filePath]);
    if (diff.trim()) return diff;
  } catch {
    // Unborn HEAD fallback
  }
  try {
    const cached = await runGit(cwd, ["diff", "--no-color", "--cached", "--", filePath]);
    if (cached.trim()) return cached;
  } catch {
    // ignore
  }
  try {
    return await runGit(cwd, ["diff", "--no-color", "--", filePath]);
  } catch {
    return "";
  }
}

