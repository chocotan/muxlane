import { app, BrowserWindow, dialog, ipcMain, shell } from "electron";
import path from "node:path";
import { MuxlaneSocketClient, resolveMuxlaneSocketPath } from "./muxlane-socket";

let mainWindow: BrowserWindow | null = null;
let rpcClient: MuxlaneSocketClient | null = null;

function createWindow(): void {
  mainWindow = new BrowserWindow({
    width: 1440,
    height: 920,
    minWidth: 960,
    minHeight: 600,
    backgroundColor: "#0c0c0d",
    title: "Muxlane",
    autoHideMenuBar: true,
    webPreferences: {
      preload: path.join(__dirname, "preload.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  });

  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    if (url.startsWith("http://") || url.startsWith("https://")) {
      void shell.openExternal(url);
    }
    return { action: "deny" };
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

function setupRpcClient(): void {
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
  rpcClient.start();
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

  ipcMain.handle("muxlane:reconnect", () => {
    rpcClient?.reconnectNow();
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

  ipcMain.handle("muxlane:open-external", async (_ev, url: string) => {
    if (typeof url === "string" && (url.startsWith("http://") || url.startsWith("https://"))) {
      await shell.openExternal(url);
    }
  });
}

app.whenReady().then(() => {
  setupIpc();
  setupRpcClient();
  createWindow();

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

app.on("window-all-closed", () => {
  rpcClient?.dispose();
  if (process.platform !== "darwin") {
    app.quit();
  }
});
