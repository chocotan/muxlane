import React, { useEffect, useState } from "react";
import {
  Check,
  Eye,
  EyeOff,
  FolderOpen,
  Laptop,
  Loader2,
  Plus,
  Server,
  Trash2,
  X,
} from "lucide-react";
import type {
  AddHostInput,
  DaemonConnectionStatus,
  HostEntry,
  HostsState,
  SshAuthKind,
} from "../shared/protocol";

interface RemoteMachinesModalProps {
  open: boolean;
  hostsState: HostsState;
  connectionStatus: DaemonConnectionStatus;
  onClose: () => void;
  onSelectHost: (hostId: string) => Promise<unknown>;
  onAddHost: (input: AddHostInput) => Promise<unknown>;
  onRemoveHost: (hostId: string) => Promise<unknown>;
}

function hostAuthBadge(host: HostEntry): string {
  if (host.kind === "local") return "local";
  if (host.kind === "socket") return "socket";
  switch (host.authKind) {
    case "password":
      return "ssh 密码";
    case "public_key":
      return "ssh 私钥";
    default:
      return "ssh config";
  }
}

function hostTargetDisplay(host: HostEntry): string {
  if (host.kind === "ssh" && host.username && !host.target.includes("@")) {
    const portSuffix = host.port ? `:${host.port}` : "";
    return `${host.username}@${host.target}${portSuffix}`;
  }
  if (host.kind === "ssh" && host.port) {
    return `${host.target}:${host.port}`;
  }
  return host.target;
}

export const RemoteMachinesModal: React.FC<RemoteMachinesModalProps> = ({
  open,
  hostsState,
  connectionStatus,
  onClose,
  onSelectHost,
  onAddHost,
  onRemoveHost,
}) => {
  const [adding, setAdding] = useState(false);
  const [kind, setKind] = useState<"ssh" | "socket">("ssh");
  const [authKind, setAuthKind] = useState<SshAuthKind>("ssh_config");
  const [target, setTarget] = useState("");
  const [name, setName] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [port, setPort] = useState("");
  const [identityFile, setIdentityFile] = useState("");
  const [remoteSocketPath, setRemoteSocketPath] = useState("");
  const [busyId, setBusyId] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setError(null);
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose, open]);

  if (!open) return null;

  const resetForm = () => {
    setTarget("");
    setName("");
    setUsername("");
    setPassword("");
    setShowPassword(false);
    setPort("");
    setIdentityFile("");
    setRemoteSocketPath("");
    setError(null);
  };

  const handleSelect = async (hostId: string) => {
    setBusyId(hostId);
    setError(null);
    try {
      await onSelectHost(hostId);
      onClose();
    } catch (err) {
      setError((err as Error).message || "切换主机失败");
    } finally {
      setBusyId(null);
    }
  };

  const handleRemove = async (hostId: string) => {
    setBusyId(hostId);
    setError(null);
    try {
      await onRemoveHost(hostId);
    } catch (err) {
      setError((err as Error).message || "移除主机失败");
    } finally {
      setBusyId(null);
    }
  };

  const effectiveUsername =
    username.trim() || (target.includes("@") ? target.trim().split("@", 1)[0]?.trim() || "" : "");

  const canSubmit =
    !submitting &&
    Boolean(target.trim()) &&
    (kind !== "ssh" ||
      authKind !== "password" ||
      (Boolean(effectiveUsername) && password.length > 0));

  const handleAdd = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    if (kind === "ssh" && authKind === "password" && (!effectiveUsername || !password)) {
      setError("用户名密码连接需要填写用户名和密码");
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      const parsedPort = port.trim() ? Number.parseInt(port.trim(), 10) : undefined;
      await onAddHost({
        kind,
        target: target.trim(),
        name: name.trim() || undefined,
        authKind: kind === "ssh" ? authKind : undefined,
        username: kind === "ssh" && effectiveUsername ? effectiveUsername : undefined,
        password: kind === "ssh" && authKind === "password" ? password : undefined,
        port: kind === "ssh" && parsedPort && !Number.isNaN(parsedPort) ? parsedPort : undefined,
        identityFile:
          kind === "ssh" && authKind === "public_key" && identityFile.trim()
            ? identityFile.trim()
            : undefined,
        remoteSocketPath:
          kind === "ssh" && remoteSocketPath.trim() ? remoteSocketPath.trim() : undefined,
      });
      resetForm();
      setAdding(false);
      onClose();
    } catch (err) {
      setError((err as Error).message || "连接远程机器失败");
    } finally {
      setSubmitting(false);
    }
  };

  const handlePickKey = async () => {
    const file = await window.muxlane.pickFile("选择 SSH 私钥文件");
    if (file) setIdentityFile(file);
  };

  const isConnectingStatus =
    hostsState.tunnelPhase === "connecting" && Boolean(hostsState.tunnelStatus);
  const displayError =
    error || (hostsState.tunnelPhase !== "connecting" ? hostsState.tunnelStatus : null);

  return (
    <div
      onClick={onClose}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4"
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="mux-popover-shadow flex max-h-[85vh] w-full max-w-[580px] flex-col border border-line-strong bg-elevated"
      >
        <div className="flex h-11 shrink-0 items-center justify-between border-b border-line-subtle px-4">
          <div className="flex items-center gap-2">
            <Server className="h-4 w-4 text-ink-muted" strokeWidth={1.75} />
            <span className="text-[13.5px] font-semibold text-ink-strong">远程机器</span>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="关闭"
            className="flex h-6 w-6 items-center justify-center text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto p-4 space-y-4">
          {isConnectingStatus && (
            <div className="flex items-center gap-2 border-l-2 border-working bg-working-soft px-3 py-2 text-[12px] text-working select-text">
              <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
              <span className="min-w-0 flex-1 break-all">{hostsState.tunnelStatus}</span>
            </div>
          )}

          {displayError && (
            <div className="border-l-2 border-danger bg-danger-soft px-3 py-2 text-[12px] text-danger select-text break-words">
              {displayError}
            </div>
          )}

          {/* Saved Hosts List */}
          <div>
            <div className="mb-2 flex items-center justify-between">
              <span className="text-[11.5px] font-medium text-ink-muted">已配置主机</span>
              {!adding && (
                <button
                  type="button"
                  onClick={() => {
                    resetForm();
                    setAdding(true);
                  }}
                  className="flex h-6 items-center gap-1 border border-line bg-canvas px-2 text-[11.5px] text-ink hover:bg-hover transition-colors cursor-pointer"
                >
                  <Plus className="h-3 w-3" />
                  <span>添加远程机器</span>
                </button>
              )}
            </div>

            <div className="divide-y divide-line-subtle border border-line-subtle bg-canvas">
              {hostsState.hosts.map((host) => {
                const isActive = host.id === hostsState.activeHostId;
                const isBusy = busyId === host.id;
                const isLocal = host.kind === "local";

                return (
                  <div
                    key={host.id}
                    className={`flex items-center justify-between gap-3 px-3 py-2.5 ${
                      isActive ? "bg-active/60" : "hover:bg-hover/50"
                    }`}
                  >
                    <button
                      type="button"
                      disabled={isBusy}
                      onClick={() => void handleSelect(host.id)}
                      className="flex min-w-0 flex-1 items-center gap-2.5 text-left cursor-pointer disabled:cursor-wait"
                    >
                      {isLocal ? (
                        <Laptop className="h-4 w-4 shrink-0 text-ink-muted" strokeWidth={1.7} />
                      ) : (
                        <Server className="h-4 w-4 shrink-0 text-ink-muted" strokeWidth={1.7} />
                      )}
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <span className="truncate text-[13px] font-medium text-ink-strong">
                            {host.name}
                          </span>
                          <span className="shrink-0 border border-line-subtle bg-elevated px-1 font-mono text-[10px] text-ink-muted uppercase">
                            {hostAuthBadge(host)}
                          </span>
                        </div>
                        <div className="truncate font-mono text-[11px] text-ink-muted">
                          {hostTargetDisplay(host)}
                        </div>
                      </div>
                    </button>

                    <div className="flex shrink-0 items-center gap-1.5">
                      {isBusy ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin text-working" />
                      ) : isActive ? (
                        <span
                          className={`flex items-center gap-1 font-mono text-[11px] ${
                            connectionStatus.connected ? "text-success" : "text-working"
                          }`}
                        >
                          <Check className="h-3.5 w-3.5" />
                          <span>{connectionStatus.connected ? "已连接" : "连接中"}</span>
                        </span>
                      ) : (
                        <button
                          type="button"
                          onClick={() => void handleSelect(host.id)}
                          className="h-6 border border-line bg-elevated px-2 text-[11.5px] text-ink hover:bg-hover transition-colors cursor-pointer"
                        >
                          切换
                        </button>
                      )}

                      {!isLocal && (
                        <button
                          type="button"
                          disabled={isBusy}
                          title="移除远程主机"
                          onClick={() => void handleRemove(host.id)}
                          className="flex h-6 w-6 items-center justify-center text-ink-muted hover:bg-hover hover:text-danger transition-colors cursor-pointer"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </button>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          </div>

          {/* Add Remote Host Form */}
          {adding && (
            <form
              onSubmit={(e) => void handleAdd(e)}
              className="space-y-3 border border-line bg-canvas p-3.5"
            >
              <div className="flex items-center justify-between">
                <span className="text-[12.5px] font-semibold text-ink-strong">连接远程机器</span>
                <button
                  type="button"
                  onClick={() => setAdding(false)}
                  className="text-[11.5px] text-ink-muted hover:text-ink cursor-pointer"
                >
                  取消
                </button>
              </div>

              <div className="grid grid-cols-2 border border-line bg-base p-[2px]">
                <button
                  type="button"
                  onClick={() => setKind("ssh")}
                  className={`h-7 text-[12px] transition-colors cursor-pointer ${
                    kind === "ssh"
                      ? "bg-active font-medium text-ink-strong"
                      : "text-ink-muted hover:text-ink"
                  }`}
                >
                  SSH 主机
                </button>
                <button
                  type="button"
                  onClick={() => setKind("socket")}
                  className={`h-7 text-[12px] transition-colors cursor-pointer ${
                    kind === "socket"
                      ? "bg-active font-medium text-ink-strong"
                      : "text-ink-muted hover:text-ink"
                  }`}
                >
                  Unix Socket
                </button>
              </div>

              <div>
                <label className="mb-1 block text-[11.5px] text-ink-muted">
                  {kind === "ssh"
                    ? "主机地址 / IP 或 ~/.ssh/config 别名"
                    : "本地 / 转发 Unix Socket 路径"}
                </label>
                <input
                  required
                  value={target}
                  onChange={(e) => setTarget(e.target.value)}
                  placeholder={
                    kind === "ssh"
                      ? "例如：192.168.1.20、ubuntu@192.168.1.20 或 dev-box"
                      : "例如：/tmp/remote-muxlane.sock"
                  }
                  className="h-8 w-full border border-line bg-elevated px-2.5 font-mono text-[12.5px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
                />
              </div>

              <div className="grid grid-cols-2 gap-2.5">
                <div>
                  <label className="mb-1 block text-[11.5px] text-ink-muted">显示名称（可选）</label>
                  <input
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder="默认使用主机名"
                    className="h-8 w-full border border-line bg-elevated px-2.5 text-[12.5px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
                  />
                </div>
                {kind === "ssh" && (
                  <div>
                    <label className="mb-1 block text-[11.5px] text-ink-muted">端口（可选）</label>
                    <input
                      value={port}
                      onChange={(e) => setPort(e.target.value)}
                      placeholder="22"
                      className="h-8 w-full border border-line bg-elevated px-2.5 font-mono text-[12.5px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
                    />
                  </div>
                )}
              </div>

              {kind === "ssh" && (
                <>
                  <div className="grid grid-cols-3 border border-line bg-base p-[2px]">
                    <button
                      type="button"
                      onClick={() => setAuthKind("ssh_config")}
                      className={`h-7 text-[12px] transition-colors cursor-pointer ${
                        authKind === "ssh_config"
                          ? "bg-active font-medium text-ink-strong"
                          : "text-ink-muted hover:text-ink"
                      }`}
                    >
                      SSH Config
                    </button>
                    <button
                      type="button"
                      onClick={() => setAuthKind("public_key")}
                      className={`h-7 text-[12px] transition-colors cursor-pointer ${
                        authKind === "public_key"
                          ? "bg-active font-medium text-ink-strong"
                          : "text-ink-muted hover:text-ink"
                      }`}
                    >
                      私钥文件
                    </button>
                    <button
                      type="button"
                      onClick={() => setAuthKind("password")}
                      className={`h-7 text-[12px] transition-colors cursor-pointer ${
                        authKind === "password"
                          ? "bg-active font-medium text-ink-strong"
                          : "text-ink-muted hover:text-ink"
                      }`}
                    >
                      用户名密码
                    </button>
                  </div>

                  {authKind === "password" && (
                    <div className="grid grid-cols-2 gap-2.5">
                      <div>
                        <label className="mb-1 block text-[11.5px] text-ink-muted">
                          用户名 {target.includes("@") ? "（已从地址读取）" : "*"}
                        </label>
                        <input
                          required={!target.includes("@")}
                          value={username}
                          onChange={(e) => setUsername(e.target.value)}
                          placeholder={
                            target.includes("@")
                              ? target.trim().split("@", 1)[0]
                              : "例如：root 或 ubuntu"
                          }
                          className="h-8 w-full border border-line bg-elevated px-2.5 font-mono text-[12px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
                        />
                      </div>
                      <div>
                        <label className="mb-1 block text-[11.5px] text-ink-muted">密码 *</label>
                        <div className="flex gap-1">
                          <input
                            required
                            type={showPassword ? "text" : "password"}
                            autoComplete="off"
                            value={password}
                            onChange={(e) => setPassword(e.target.value)}
                            placeholder="SSH 登录密码"
                            className="h-8 min-w-0 flex-1 border border-line bg-elevated px-2.5 font-mono text-[12px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
                          />
                          <button
                            type="button"
                            onClick={() => setShowPassword((v) => !v)}
                            title={showPassword ? "隐藏密码" : "显示密码"}
                            aria-label={showPassword ? "隐藏密码" : "显示密码"}
                            className="flex h-8 w-8 shrink-0 items-center justify-center border border-line bg-elevated text-ink-muted hover:bg-hover hover:text-ink-strong cursor-pointer"
                          >
                            {showPassword ? (
                              <EyeOff className="h-3.5 w-3.5" />
                            ) : (
                              <Eye className="h-3.5 w-3.5" />
                            )}
                          </button>
                        </div>
                      </div>
                    </div>
                  )}

                  {authKind === "public_key" && (
                    <div className="grid grid-cols-2 gap-2.5">
                      <div>
                        <label className="mb-1 block text-[11.5px] text-ink-muted">
                          用户名（可选）
                        </label>
                        <input
                          value={username}
                          onChange={(e) => setUsername(e.target.value)}
                          placeholder="如地址已含 user@ 可留空"
                          className="h-8 w-full border border-line bg-elevated px-2.5 font-mono text-[12px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
                        />
                      </div>
                      <div>
                        <label className="mb-1 block text-[11.5px] text-ink-muted">私钥路径</label>
                        <div className="flex gap-1">
                          <input
                            value={identityFile}
                            onChange={(e) => setIdentityFile(e.target.value)}
                            placeholder="~/.ssh/id_ed25519"
                            className="h-8 min-w-0 flex-1 border border-line bg-elevated px-2.5 font-mono text-[12px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
                          />
                          <button
                            type="button"
                            onClick={() => void handlePickKey()}
                            title="浏览私钥文件"
                            className="flex h-8 w-8 shrink-0 items-center justify-center border border-line bg-elevated text-ink-muted hover:bg-hover hover:text-ink-strong cursor-pointer"
                          >
                            <FolderOpen className="h-3.5 w-3.5" />
                          </button>
                        </div>
                      </div>
                    </div>
                  )}

                  <div>
                    <label className="mb-1 block text-[11.5px] text-ink-muted">
                      远端 Socket 路径（留空则自动探测并按需部署/启动 muxlane）
                    </label>
                    <input
                      value={remoteSocketPath}
                      onChange={(e) => setRemoteSocketPath(e.target.value)}
                      placeholder="自动发现 ~/.local/share/muxlane/muxlane.sock"
                      className="h-8 w-full border border-line bg-elevated px-2.5 font-mono text-[12px] text-ink-strong placeholder:text-ink-muted focus:border-line-strong focus:outline-none select-text"
                    />
                  </div>

                  {authKind === "password" && (
                    <div className="text-[11px] leading-[1.45] text-ink-muted">
                      密码以 0600 权限保存在本机密钥文件并通过一次性 SSH_ASKPASS 传递，不会写入命令行参数或普通配置文件。
                    </div>
                  )}
                </>
              )}

              <div className="flex items-center justify-end gap-2 pt-1">
                <button
                  type="button"
                  onClick={() => setAdding(false)}
                  className="h-8 border border-line bg-elevated px-3 text-[12px] text-ink-2 hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
                >
                  取消
                </button>
                <button
                  type="submit"
                  disabled={!canSubmit}
                  className="flex h-8 min-w-[104px] items-center justify-center gap-1.5 bg-primary-btn px-3.5 text-[12px] font-medium text-primary-btn-fg hover:bg-primary-btn-hover disabled:opacity-45 transition-colors cursor-pointer"
                >
                  {submitting && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                  <span>{submitting ? "正在连接…" : "添加并连接"}</span>
                </button>
              </div>
            </form>
          )}
        </div>
      </div>
    </div>
  );
};
