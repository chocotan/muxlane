import { contextBridge, ipcRenderer } from "electron";
import type {
  DaemonConnectionStatus,
  MuxlaneDesktopBridge,
  RpcEventMessage,
} from "../src/shared/protocol";

const api: MuxlaneDesktopBridge = {
  rpc<T = unknown>(method: string, params?: unknown): Promise<T> {
    return ipcRenderer.invoke("muxlane:rpc", method, params ?? {});
  },
  getStatus(): Promise<DaemonConnectionStatus> {
    return ipcRenderer.invoke("muxlane:status");
  },
  reconnect(): Promise<void> {
    return ipcRenderer.invoke("muxlane:reconnect");
  },
  pickDirectory(): Promise<string | null> {
    return ipcRenderer.invoke("muxlane:pick-directory");
  },
  openExternal(url: string): Promise<void> {
    return ipcRenderer.invoke("muxlane:open-external", url);
  },
  onEvent(handler: (msg: RpcEventMessage) => void): () => void {
    const listener = (_ev: Electron.IpcRendererEvent, msg: RpcEventMessage) => handler(msg);
    ipcRenderer.on("muxlane:event", listener);
    return () => ipcRenderer.removeListener("muxlane:event", listener);
  },
  onStatus(handler: (status: DaemonConnectionStatus) => void): () => void {
    const listener = (_ev: Electron.IpcRendererEvent, status: DaemonConnectionStatus) =>
      handler(status);
    ipcRenderer.on("muxlane:status", listener);
    return () => ipcRenderer.removeListener("muxlane:status", listener);
  },
};

contextBridge.exposeInMainWorld("muxlane", api);
