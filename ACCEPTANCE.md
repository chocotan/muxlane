# Muxlane 验收报告

日期：2026-09-03

## 自动化门槛

```text
cargo fmt --all -- --check                            PASS
cargo clippy --workspace --all-targets -- -D warnings PASS
cargo test --workspace                                274 passed, 0 failed
scripts/ui-smoke.sh                                    PASS（像素+状态断言；含 ACP 创建、流式回复、持久化与归档）
scripts/release-smoke.sh                               PASS
```

## UI 自动化覆盖

- Agent Thread 页面按目标截图收敛为 timeline + 无边框 composer；底栏仅保留附件、`@`、模型/thinking 与发送
- PromptEditor 使用 GPUI `TextLayout` 实际塑形结果处理光标、点击、拖选与 IME 坐标，不再使用硬编码字符网格
- History 可从 tab `…` 菜单打开，重复选择、Escape 或点击外部均可关闭；响应不会重新打开已关闭弹层
- checkpoint Restore/Undo 不再常驻底栏，仅在 `⋯` 更多菜单中按需出现
- 新建会话显式支持 `UI / Terminal`，本地与远程默认均为 Terminal，远程保持 Terminal-only
- Agent Thread 使用原生 GPUI timeline/composer；支持 Markdown、thinking、plan、tool、diff、terminal content 和 follow-tail
- Composer 支持多行编辑、IME、`/` ACP command 与本地 skill、`@` 文件/目录/符号/thread/terminal/URL/diagnostic context
- 支持图片/音频附件 capability gating，以及 mode/model/thinking/config selector
- permission FIFO、form/URL elicitation、生成中 prompt queue/pause/Send Now
- ACP host 文件访问限制在项目根目录；terminal 自动回收并在 wait 后保证 stdout/stderr 已 drain
- typed thread snapshot、draft、queue、usage/plan 持久化；History、恢复与 replay 去重
- 关闭 Agent Thread tab 永久删除对应线程记录与线程文件
- checkpoint 恢复前二次确认并保存 Undo；未捕获的大型 untracked 文件保持不动
- subagent session 关联打开；ACP `session/list` 分页带重复 cursor 与页数保护
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

- ACP `session/list` 分页、replay 分歧、permission FIFO、elicitation value、terminal drain/reap 均有定向测试
- 线程文件名与 JSON `ui_id` 必须一致；损坏/错名文件隔离报告，不复活已删除 thread
- checkpoint 覆盖 tracked/index/untracked/Undo 与超限大文件保留
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
