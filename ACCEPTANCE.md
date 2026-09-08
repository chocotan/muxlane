# Muxlane 验收报告

日期：2026-09-03

## Task39 输入法失效根因与工作区/收回修复（v0.0.5）

日期：2026-09-08。本节为本次实际执行结果。

- **IME 概率性失效（XIMClientError 后进程内永久丢失输入法）**：用隔离数据目录、真实开窗口的冷启动基准（每轮 32~48 次）定位。结论：不是某个提交引入的逻辑错误，而是启动时序竞争——恢复每个 tmux 会话要 fork 一次 `tmux`，与开窗口后 GPUI/X11 的 XIM 握手并发；握手包交错后 fcitx 回包解析失败，GPUI X11 后端直接丢弃 XIM 连接且不重连。触发条件是"启动时有会话要恢复"：空配置 0/32；5 个会话时 v0.0.3 也会中（1~7/32），当前版本 5/32。此前误判为"既有问题/与本次无关"及"删 preedit 清理即修复"均不成立，已在此更正。
- **绕开修复（不触碰任何 IME 逻辑）**：GUI 启动不再在开窗口前阻塞 `restore_sessions`；先用 `PersistedApp::provisional_snapshot` 把保存的会话作为占位 agent 摆出布局，首帧绘制后延迟 300ms 再在 Tokio runtime 上真正 attach。`restore_sessions` 改为整批只 bump 一次 dirty（`restore_agent_quiet` + `bump_dirty`），UI 用完整快照做 reconcile；`apply_local_snapshot` 快照到达后主动挂上活动终端。Headless 无窗口，保持同步恢复。同条件复测：修改前 5/32，修改后 0/32、0/48；会话在启动后 1.0s 内全部挂上。代价：启动后终端内容晚 0.3~1s 出现。
- **TermView 窗口重绑不再清 preedit / 推光标坐标**：按"IME 交给系统"原则移除；经基准验证它与本次 XIM 失效无关，但保留删除。对应单测改为断言 preedit 原样保留。
- **"一项目一工作区"下收回会话进错项目**：`reattach_session` / `reattach_all_sessions` 原先无条件放进当前屏幕 pane 树。改为按会话归属：当前项目（或未开工作区）放屏幕 pane，否则写入所属项目保存的布局（`dock_into_owning_project`）。内容区空白占位在工作区模式下只统计当前项目。
- **已损坏的持久化布局自愈**：新增 `WorkspaceController::repair_ownership` + `MuxlaneApp::repair_workspace_ownership`，启动及每次本地/远程快照后把每个项目布局里不属于它的会话送回所属项目；幂等。用真实 `state.json` 验证：当前项目原 4 tab（3 个外来）→ 1 tab，外来会话各归其位。
- **全部弹出/收回改为全机器全项目**；侧栏页脚按钮的"是否还有会话在主窗口"判断同步改为全局。
- 测试：新增 `restore_sessions` 单次批量通知（真 tmux 再 attach）、`provisional_snapshot` 去重/孤儿/往返、工作区收回归属、损坏布局自愈、切换项目不复活已弹出会话（bound_window 稳定、渲染 ≤2 次）等回归；全仓 287 项通过，fmt/clippy 干净。
- 启动脚本 `scripts/demo-instance.sh`（隔离数据目录的演示实例）本次不纳入提交。

## Task38 会话弹出重构与远程已读/崩溃修复（v0.0.4）

日期：2026-09-08。本节为本次实际执行结果；下方 Task37/Task36 描述的 Dock 与全局 Floating 模式已被本次移除，仅作历史记录。

- **远程通知误闪烁**：`focus_agent` 对远程终端本地把 Done/Failed 改写为 Idle 与服务端不一致，下一次快照合并误判为新结果重新提醒；改为远程只标记 seen、不改 status，指示灯灰色由渲染层 `display_status(status, seen)` 决定。新增 `agent.mark_seen` RPC 与 `AGENT_MARK_SEEN` 能力，远端支持时真正写回服务端并随远端持久化，旧远端自动降级为会话内已读。端到端测试起真实远程 server，走真实聚焦路径，再用零共享内存的新客户端重连验证已读不复活。
- **远程新增/删除终端闪退**：远程 RPC 在 GPUI 后台线程执行，无 Tokio reactor 导致 `there is no reactor running` abort。统一经 `spawn_remote_operation` 投递到应用 Tokio runtime；覆盖新增/删除会话、新增/删除项目、安装/启动/升级远端 5 处。测试从无 Tokio 上下文的 GPUI 入口发起真实 Unix socket 请求验证冷/热连接。
- **Dock 重构为逐会话弹出**：删除自绘 Dock/dashboard/布局弹层、主窗口自动 `resize`、全局 Tiled/Floating 开关、store 内无调用的 `Presentation/.projected()/arrange` 模型及 `dock_navigation_tests`/`native_window_tests`。主窗口固定为侧栏+内容区。新增逐会话弹出/收回、全部弹出/全部收回（全机器全项目）、侧栏弹出标记与置顶、tab 右键菜单、内容区全空占位、直角图标与中英文案；`FloatWindow.hidden` 语义重解释为是否收回，旧配置 `presentation` 字段忽略。
- **弹出后闪烁/双份终端**：弹出只从当前 pane 树摘 tab，未从 workspace 各项目保存布局中删除，切换项目恢复布局把它重新变成 tab，主窗口与原生窗口同时渲染同一 `TermView` 并争抢 `bound_window`。修复为弹出时从所有布局移除，`apply_workspace_layout` 恢复时过滤已弹出会话。回归测试在按项目工作区下来回切换 4 次，断言不回到 tab、`bound_window` 稳定、渲染次数 ≤2。
- **快捷键默认值**：上一/下一标签 `Platform+Alt+K/J`，新建标签 `Platform+Alt+↑`，右侧/下方分屏 `Platform+Alt+→/↓`。旧默认值自动迁移，自定义值保留。
- 测试：app 116、store 30、其他 crate 全部通过（app 需单线程运行，并行存在既有窗口尺寸类 flaky 与本次无关）。改动 UI 面无 `rounded`/`rx`/`ry`。启动日志中的 `XIMClientError` 在打开任何弹出窗口前即出现，为 GPUI/fcitx5 XIM 握手既有问题，与本次无关。

## Task37 Dock 两级导航补充验收

本节记录此次两级 Dock 导航收尾，下面 Task36 和其他章节保留为历史记录，不代表此次执行了真实桌面或 release 验证。

```text
MUXLANE_DISABLE_NOTIFY=1 MUXLANE_DISABLE_SOUND=1 cargo test --workspace --offline
                                      PASS: 292 passed, 0 failed, 0 ignored
cargo check --workspace --offline      PASS
git diff --check                       PASS
方角及旧 projects_open 扫描            PASS（仅 rx 接收器变量匹配，无圆角容器）
```

- Dock 进入 Floating 默认展示可横向滚动的项目列表，点击项目进入其 tab 列表；返回图标回到项目列表，不再提供项目 popover。项目名带机器上下文、会话数和未读提示；布局、通知和设置工具保留。
- 项目浏览和返回不打开、激活或关闭任何会话 OS 窗口，不终止后台会话。显式 tab 点击复用已有窗口和 TermView Entity，或恢复隐藏窗口。真实 GPUI TestPlatform 测试验证窗口 registry 不变、列表隔离及返回。
- 根层新建打开现有本地添加项目对话框，空项目列表保留明确 Add project 入口。tab 层新建会话使用浏览项目的完整机器/项目键，不使用通知或其他窗口改变后的当前工作区。
- 通知继续恢复并聚焦目标 OS 窗口，但保持 Dock 浏览项目；原会话键盘导航未绑定到 Dock 浏览状态。跨机器相同 project ID 保持隔离；权威删除浏览项目或忘记机器回到根层，单纯离线保留浏览状态。
- 浏览只为未登记 tab 初始化隐藏记录，已有可见/隐藏记录保持原样。新增序列化往返回归验证模式仍为 Floating、已有窗口状态不变、dock_project 不持久化，恢复后默认项目列表；隐藏 tab 仍可显式恢复。
- 新增 root 层 320/480 像素、200% 缩放多项目滚动测试，并保留 tab 层 200% 工具可达、滚动和点击测试。会话右键菜单保留原终止路径，新增回归验证菜单目标及 Escape 关闭；未把菜单目标测试声称为真实后台终止验证。
- 本次仅使用进程内 GPUI TestPlatform 和工作区测试，没有操作用户 GUI、Fcitx 或既有进程，没有应用重启、提交或发布。仅定向格式化相关文件；真实 X11/Wayland、跨屏与 IME 视觉验收仍未执行。

## Task36 独立 OS 窗口补充验收

本节替代早先应用内浮动桌面的错误范围及验收结论。以下为此次实际执行，后续章节是历史记录，并非本次重跑的 UI/release 证明。

```text
MUXLANE_DISABLE_NOTIFY=1 MUXLANE_DISABLE_SOUND=1 cargo test --workspace --offline
                                      PASS: 286 passed, 0 failed
cargo check --workspace --offline      PASS
git diff --check                       PASS
cfg(any()) / cx.test_window 扫描        PASS（无匹配）
方角样式扫描（本次修改测试表面）        PASS（rx 接收器变量非 SVG 属性）
```

- 新增 OS 焦点回归：TermFocusEvent 携带 agent、WindowId 和 focus/blur，窗口激活/失活同样更新；旧窗口失焦不清新焦点。重开时强制 TermView 重绘以重绑订阅。Floating 的远端 seen 合并、状态事件、通知 focused 和 Dock 状态均使用实际窗口焦点，Tiled 保留原语义。
- 验证两个 OS 窗口与主 Dock 之间的激活、窗口内 blur、未聚焦新通知不被抑制、聚焦清未读、后台创建不标记已读、输入不丢失、旧 WindowId 事件隔离、关闭/删除/切回 Tiled 清焦点。最新测试命令设置 `MUXLANE_DISABLE_NOTIFY=1 MUXLANE_DISABLE_SOUND=1`，禁用测试的系统通知和声音副作用。
- 每个会话是独立 GPUI Normal Window / SessionWindow root，共享唯一 MuxlaneApp 控制器和原 TermView Entity。主窗口只提供 Dock 和管理面板；无内部拖缩、吸附或伪最大化。
- 真实 GPUI TestPlatform 多窗口测试覆盖不同 WindowId/root、OS close 后 Dock 重开保留 Entity、后台会话不被删除、Dock 项目筛选不关闭其他 OS 窗口、通知恢复及目标窗口焦点和输入、主 Dock 不接收终端输入。
- Tiled/Floating 往返测试验证旧 OS 句柄关闭、Entity 不重建、TermView 绑定 WindowId 迁移；实现先卸载旧承载再挂载新承载。测试终端为惰性输入通道，未以真实 PTY 跨窗口运行作为本次验收依据。
- 覆盖打开失败注入后隐藏保留、Dock 重试恢复、无效句柄与 agent 删除清理；保留原后台 placement、项目删除、远端离线/online 清理及通知回归测试。
- OS 会话关闭或在会话窗口按 `Ctrl+W` 仅隐藏；会话菜单可终止，Tiled 的关闭标签页快捷键保留原有终止语义。Floating 分屏仅提示只支持 Tiled，不自动切换布局或修改 pane tree。主窗口确认退出时清理注册的会话窗口。原生创建不激活后台窗口，显式 Dock/通知打开才激活目标；系统焦点事件不切换 Dock 的项目筛选。
- TermView 模块内使用薄 root 的 GPUI TestPlatform 测试，先从 A 卸载再在 B 渲染同一 Entity，验证 preedit 清除、bound_window 更新及 B 的实际布局写回 last_bounds。未验证真实平台 IME handler、候选窗或 Fcitx 跨窗口行为；无生产 capture hook 或禁用的占位测试。
- 模式/项目/隐藏状态保持兼容，旧 FloatRect 画布坐标不映射为 OS 坐标。本版本不持久化原生位置/大小。Wayland 坐标、系统装饰和移动能力取决于 compositor；尚未做真实 X11/Wayland 多屏视觉验收。
- Dock 是普通、不透明、非置顶的主窗口，无系统面板/置顶/透明平台 hack；控件保留方角。未操作用户 GUI、Fcitx 或既有进程，未重启、提交或发布；release 由父协调者执行。

## 自动化门槛

```text
cargo fmt --all -- --check                            PASS
cargo clippy --workspace --all-targets -- -D warnings PASS
cargo test --workspace                                256 passed, 0 failed
scripts/ui-smoke.sh                                    PASS
scripts/release-smoke.sh                               PASS
```

## UI 自动化覆盖

- zsh 会话正确启动（sidebar/tab 显示 `zsh`）
- xterm-256color + truecolor，cell foreground/background/bold/italic/underline
- 可见 block cursor
- UI 键盘输入 → GPUI InputHandler → 同步 PTY write → shell 输出 → `term.subscribe` replay
- Fcitx preedit/commit/unmark 中文链路；IME candidate 使用逻辑 cursor cell bounds
- 实测 monospace cell + force-width GPUI canvas；宽字符 spacer 断 run，combining marks 保留
- 本地 VTerm scrollback + mouse/alternate-screen mode routing
- terminal viewport 4px 内边距与保守 cols/rows 计算
- Ctrl+K 命令面板
- 本地机器 `＋` 打开真实 TextField，目录 canonicalize/去重后持久化空项目
- 项目 `＋` 创建会话菜单
- 会话右键“删除会话”菜单
- tab strip `＋` / Ctrl+Shift+T 创建同 pane Shell tab，不隐式 split
- sidebar machine/project/session 树层级和动态 OSC title
- 显式水平/垂直 split；新 panel 固定创建普通 Shell，不复制当前 Agent 类型
- 2px 可见分隔线（扩展命中区）拖动调整比例并持久化
- 关闭 split 删除 leaf、递归折叠父树，但不终止对应 tmux session
- 最大化/还原；tab 拖动同组重排/跨 pane
- 远端状态聚合、term.subscribe replay + incremental
- Pi/OpenCode/Claude/Codex 通知提取最终消息；左下与桌面通知复用同一 normalized body
- 同状态 done hook 可以补全先到的 screen-detection 空消息
- OpenCode `session.idle` 与 Pi `agent_settled` plugin 幂等安装
- Pi 真实 `pi -p` 短任务通过 tmux env 上报，`state.list` 最终为 `done`
- tmux `mouse on` 支持滚轮/copy-mode，`status off` 保持无底栏
- SSH config/公钥/用户名密码认证；密码不持久化、不进入 argv/ControlMaster 环境
- missing/stopped remote 探测与确认式自动安装/启动 headless
- `system.hello` capability/version handshake；旧远端显示需要更新，不能把协议不兼容误报为离线
- 远端可创建 Shell 项目/会话；`term.input`/`term.resize` 经过 SSH tunnel 路由到对应 host
- 删除远程机器只清本地连接/tunnel；删除项目才销毁 scoped muxlane tmux sessions

证据：[`artifacts/ui-smoke/`](./artifacts/ui-smoke/)

## 协议与安全

- `state.list`：通过
- `term.subscribe` replay + 增量：通过
- 多订阅者同流：通过
- `events.subscribe` 跨连接广播：通过
- `agent.report`：有效 HMAC 接受；伪造/过期 token 拒绝
- `agent.delete`：kill 持久 tmux session + 状态清理
- Node ESM hook 脚本：真实 server 集成测试通过
- Unix socket：`0600`
- secret：32 bytes，`0600`

## 发布产物

```text
target/release/muxlane                           ~24 MB
dist/muxlane-0.0.2-linux-x86_64.tar.gz          ~9.7 MB
dist/muxlane-0.0.2-linux-x86_64.tar.gz.sha256
```

安装脚本和 desktop entry 在临时 `PREFIX` 下通过验收。

## 独立代码审查修复

三个并行 reviewer（正确性/测试/维护性）审查后修复并补回归：

- 协议 reader 改为独立任务，半帧不会被 `select!` 取消
- Unix socket 单实例 lock，第二进程不能 unlink 活跃 listener
- PTY `ChildKiller` 与阻塞 `wait` 分离；删除不会死锁
- replay + broadcast 原子交接；Lagged/背压触发 `term.resync`，不再静默丢 ANSI 字节
- 订阅按连接清理；客户端 stream 无隐藏泄漏任务；断线自动重订阅
- RemoteHost `state.changed` 重拉快照；多远端按 host endpoint 路由
- SSH StreamLocalForward 使用 `-L local_socket:remote_socket`；不启用未鉴权的 socat TCP fallback
- DetectionEngine manifests 接入生产 tick；Done/seen 查看后回 Idle
- Claude hooks 改为追加/去重并支持卸载，不覆盖用户同类 hooks
- HMAC secret 载入时强制修复 `0600`
- PaneTree tab 末位拖动索引、非活动 pane 关闭、持久化边界等修复

## 已知后续项

1. 在两台独立实体机器上做 SSH StreamLocalForward 长时间稳定性测试
2. Relay（可选）及其显式配对/SAS 流程
3. macOS/Windows GPUI 构建与签名
4. 行级 retained GPU view（当前已经有 TermDamage 行缓存；只有压测证明 submit 成瓶颈后再做）
