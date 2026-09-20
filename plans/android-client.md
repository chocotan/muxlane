# Android 客户端

## Context

Muxlane 是 Rust + GPUI 桌面工作台（Linux / macOS）。服务端只听本机 Unix socket；桌面连远端靠本机 `ssh` 把 socket 转过来。没有 Android 代码，没有网络监听，连接本身不鉴权（`AuthSecret` 只给 hook token）。`pair.begin` 只在协议常量里，服务端未实现。

用户要的是：**手机远程查看器**，Agent 仍在电脑上跑。公网靠 **用户自建中继**（官方不托管）。v1 **只走中继**，不做局域网直连端口。客户端 **Kotlin + Compose**。功能：**配对、列表、单会话终端、状态、重连、系统通知、新建/删除会话**。

不做：手机上跑 Agent、分屏/弹窗、SSH 部署远端、添加/删除项目、Kitty 图形、官方中继。

## Approach

三块，一次做完最小闭环：

```
手机 Compose  --WSS-->  muxlane-relay(用户 VPS 或 Host 本机)
                              ^
桌面/headless --WSS outbound--+     本机仍听 muxlane.sock
```

1. **`muxlane-relay`**：小二进制。WebSocket 房间。不解析 RPC。TLS 由部署方终止（caddy/nginx）。
2. **Host outbound**：`muxlane-server` 连中继，把每条手机通道当成一条 RPC 连接（与现有 `handle_conn` 同类）。落地 `pair.begin`。Unix socket 路径不鉴权，中继路径必须先配对。
3. **Android App**：填中继 URL + 配对码；之后用 `host_id` + token 重连。协议用现有 newline-JSON（每条 WS 文本消息 = 一条 JSON 帧）。终端用 Compose 网格 + 自有 MIT VT，不 JNI、不引 Termux（GPL）。

配对（最短）：

- Host 常驻 `wss://relay/host/{host_id}`（`host_id` = 已有 `machine_id`）。
- 「配对手机」：Host 向中继登记 8 位码，5 分钟过期、5 次失败作废。桌面弹窗显示码和中继 URL。
- 手机连 `wss://relay/pair/{code}`，中继把该 socket 接到对应 Host。
- 第一条 RPC：`pair.begin` `{code}`。Host 用现有 `AuthSecret` 签发长期 token（subject=`mobile:{device}`）。
- 之后手机连 `wss://relay/phone/{host_id}`，第一条 RPC：`pair.begin` `{token}`。失败则断开。
- 中继不验 token，只按路径转发。防滥用：码短 TTL + 单房间单手机通道（v1 同时一个手机；多设备可先后连）。

新建会话：协议没有预设列表。加 `preset.list`（按项目 PATH 过滤 `builtin_presets`），手机只展示 Host 上已安装的项，再调现有 `agent.spawn`。

通知：没有官方推送。App 用前台服务保活 WS，收到 `agent.status_changed` 且 `to` 为 Blocked/Done/Failed 时发系统通知。点通知打开对应会话并 `agent.mark_seen`。

UI：机器 → 项目 → 会话；方角（Compose `RectangleShape`，禁止 `RoundedCornerShape`）。中文为主，跟桌面 i18n。

## Files to modify

**新**

- `crates/muxlane-relay/` — 中继二进制（tokio + tungstenite）
- `android/` — Gradle / Compose 工程（不进 Cargo workspace）

**改**

- `Cargo.toml` — members 加上 `muxlane-relay`
- `crates/muxlane-core/src/protocol.rs` — `pair.begin` 参数/结果；`preset.list`；feature 名
- `crates/muxlane-core/src/auth.rs` — token subject 用于 `mobile:*`（现有 HMAC 格式够用则不改）
- `crates/muxlane-server/src/lib.rs` — 中继路径走 `pair.begin`；RPC 分发加上 `preset.list`
- `crates/muxlane-server/src/` 新 `relay.rs` — outbound WSS、配对码登记、每通道 `handle_conn`
- `crates/muxlane-server/src/api.rs` — `list_presets(project)` 复用 `builtin_presets` + `installed_in`
- `crates/muxlane-store/src/lib.rs` — 持久化 `relay_url`（可选启用）
- `crates/muxlane-app/src/settings.rs` / `dialogs.rs` / `i18n.rs` — 中继 URL、配对弹窗
- `crates/muxlane-app/src/main.rs` — headless：`--relay URL`，日志打出配对码
- `README.md` — 自建中继 + Android 查看器

## Reuse

- 帧：`protocol::Request/Response/EventMsg`，方法 `system.hello`、`state.list`、`events.subscribe`、`term.subscribe/input/resize/unsubscribe`、`agent.spawn/delete/mark_seen`
- 模型：`Snapshot`、`Project`、`AgentInstance`、`AgentStatus`、`AgentSpawnParams`
- 终端字节：`term.data` / `term.replay_chunk` / `term.resync` / `term.exit`，base64；订阅时 `accept_replay_chunks: true`
- 鉴权算法：`AuthSecret::token` / `verify`（`v1:expiry:mac`）
- 预设：`muxlane_core::builtin_presets` + `AgentPreset::installed_in`
- 服务端连接循环：`MuxlaneServer::handle_conn`（中继通道复用，不要复制 RPC 分支）
- 侧栏信息架构：机器 → 项目 → 会话
- 通知语义：桌面 `notifications.rs` 的 Done/Blocked/Failed；手机只做系统通知，不做桌面 toast 动画

不复用：GPUI `TermView`、本机 ssh ControlMaster、tmux、桌面分屏/弹出窗口、`muxlane-client::Connection`（UnixStream，安卓重写一份 WS 客户端）。

## Steps

- [ ] **中继** `muxlane-relay`：`/host/{id}`、`/pair/{code}`、`/phone/{id}`；空房间 5 分钟回收；单通道 splice 或按消息转发。单测用两个 WS 客户端对发。
- [ ] **协议** 落地 `pair.begin`（code | token → token + machine）和 `preset.list`（`{project}` → 已安装预设）。`system.hello.features` 加上对应名。Unix socket 仍不要求 pair。
- [ ] **Server outbound**：设置里有 `relay_url` 则连中继；配对码 API 给 UI；每条手机通道进现有 `handle_conn`，入站先 `pair.begin` 否则断开。
- [ ] **桌面 UI**：设置填中继 URL；「配对手机」弹窗显示 URL + 码；方角。headless `--relay` 把码打到日志。
- [ ] **Android**：`android/` Compose。屏：中继 URL、配对、会话树、终端、新建会话（preset 列表）、删除确认。OkHttp/Ktor WS。前台服务 + 状态通知。方角。
- [ ] **终端**：Compose Canvas 网格 + 自有 VT（ANSI/256/真彩/CJK 宽字符/滚动）。不做到桌面 Kitty 图形。默认 80×24，按像素 `term.resize`。
- [ ] **README**：relay 部署（反代 WSS）、桌面填 URL、手机配对。

## Verification

1. 本机起 `muxlane --headless --relay ws://127.0.0.1:PORT` 和 `muxlane-relay`。
2. 日志里的码，用 `websocat`/小脚本 `pair.begin` → `state.list`，应看到项目/会话。
3. 模拟器：配对 → 列表 → 打开会话有回放和增量 → 软键盘输入进 Host PTY。
4. 新建 Claude/Shell（Host 已安装的）出现在树里；删除会话 Host 上消失。
5. App 进后台，Host 上 Agent 变 Done，系统通知出现；点开后 `seen`。
6. 杀 App 重开：用保存的 token 重连，不用新码。
7. 错误码拒绝、过期码拒绝、Unix socket 客户端不走 pair 仍能用。
8. `cargo test --workspace`；Android 仪器测试至少：帧编解码、pair token 存储、VT 基础 SGR。

## Defaults（未再问）

- 中继用户自建；局域网 = 把 relay 跑在 Host 上，手机填 `ws://192.168.x.x`
- 同时一条手机通道
- minSdk 26，中文 UI
- 不加项目增删、LAN TCP、FCM、JNI VTerm
- 中继不加密 RPC 明文（WSS + 自建；token 防匿名）

## Open → closed

| 项 | 决定 |
| --- | --- |
| 产品 | 远程查看器 |
| 穿透 | Host outbound 自建中继 |
| 官方托管 | 不做 |
| 局域网直连端口 | v1 不做 |
| 技术栈 | Kotlin + Compose |
| v1 功能 | 配对 + 终端 + 通知 + 新建/删除会话 |
