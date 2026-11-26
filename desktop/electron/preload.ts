import { contextBridge, ipcRenderer, webFrame } from "electron";
import type {
  AddHostInput,
  DaemonConnectionStatus,
  HostsState,
  MuxlaneDesktopBridge,
  RelayConfig,
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
  pickFile(title?: string): Promise<string | null> {
    return ipcRenderer.invoke("muxlane:pick-file", title);
  },
  openExternal(url: string): Promise<void> {
    return ipcRenderer.invoke("muxlane:open-external", url);
  },
  setZoomFactor(factor: number): void {
    const clamped = Math.min(1.5, Math.max(0.75, factor));
    webFrame.setZoomFactor(clamped);
  },
  listHosts(): Promise<HostsState> {
    return ipcRenderer.invoke("muxlane:hosts-list");
  },
  addHost(input: AddHostInput): Promise<HostsState> {
    return ipcRenderer.invoke("muxlane:hosts-add", input);
  },
  removeHost(hostId: string): Promise<HostsState> {
    return ipcRenderer.invoke("muxlane:hosts-remove", hostId);
  },
  selectHost(hostId: string): Promise<HostsState> {
    return ipcRenderer.invoke("muxlane:hosts-select", hostId);
  },
  getRelayConfig(): Promise<RelayConfig> {
    return ipcRenderer.invoke("muxlane:relay-get");
  },
  saveRelayConfig(cfg: RelayConfig): Promise<RelayConfig> {
    return ipcRenderer.invoke("muxlane:relay-save", cfg);
  },
  getGitChanges(cwd: string) {
    return ipcRenderer.invoke("muxlane:git-changes", cwd);
  },
  getGitFileDiff(cwd: string, filePath: string, status: string) {
    return ipcRenderer.invoke("muxlane:git-file-diff", cwd, filePath, status);
  },
  openDshWeb(projectId: string, projectPath: string, customUrl?: string) {
    return ipcRenderer.invoke("muxlane:dsh-open", projectId, projectPath, customUrl);
  },
  restartDshWeb(projectId: string, projectPath: string) {
    return ipcRenderer.invoke("muxlane:dsh-restart", projectId, projectPath);
  },
  stopDshWeb(projectId: string) {
    return ipcRenderer.invoke("muxlane:dsh-stop", projectId);
  },
  listDshWeb() {
    return ipcRenderer.invoke("muxlane:dsh-list");
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
  onHostsChanged(handler: (state: HostsState) => void): () => void {
    const listener = (_ev: Electron.IpcRendererEvent, state: HostsState) => handler(state);
    ipcRenderer.on("muxlane:hosts-changed", listener);
    return () => ipcRenderer.removeListener("muxlane:hosts-changed", listener);
  },
};

contextBridge.exposeInMainWorld("muxlane", api);
