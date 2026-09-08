# Muxlane

> **原生 Rust + GPUI 多 Agent 终端工作台**
> 在一个工作台中管理本地与 SSH 远端的项目、终端和 AI Coding Agent 会话。桌面进程同时提供本机 Unix socket 服务，也可用 `--headless` 单独运行服务端；每条远程连接读取该实例的项目与会话，不会递归发现它连接的其他机器。

---

![Muxlane Workbench Split View](docs/images/workbench-split.png)

---

## 核心特性

### 1. 原生工作台与通知
- **Rust + GPUI**：原生桌面界面与终端渲染，控件、菜单和通知容器统一使用方角。
- **直观层级侧边树**：机器（Machine）→ 项目（Project）→ 会话（Session）清晰拓扑结构展示，支持项目多选展开折叠与会话实时 OSC 动态标题。
- **方角通知浮层**：
  - 侧栏底部轻盈的通知入口，实时呈现未读计数与呼吸微光状态（Working 脉冲 / Blocked 高警示微光）；
  - 浮层式通知中心，记录各 Agent 任务流转（执行中、等待输入、任务完成）与相对时间，支持一键点击直达对应终端窗格。
- **命令面板（Command Palette）**：应用内 `Win/Cmd+K` 唤起，检索并启动预设 Agent（Claude Code、Codex、Pi、Agy、OpenCode、Qwen 等）或执行分屏命令；Agent CLI 需自行安装。

| 命令面板 (Win/Cmd+K) | 会话交互与菜单管理 |
|:---:|:---:|
| ![Command Palette](docs/images/command-palette.png) | ![Session Menu](docs/images/session-menu.png) |

---

### 2. 终端引擎与输入
- **真终端架构**：底层由 `portable-pty` 与 `alacritty_terminal` 协同驱动：
  - **Shell 与色彩**：支持自定义 Shell、ANSI 256 色与 TrueColor；设置 `TERM=xterm-256color`、`COLORTERM=truecolor`。
  - **排版与输入法**：按字体度量与终端单元格宽度绘制，处理 CJK 双宽字符、结合符及 IME 组合/提交；字体与 Fcitx5、IBus 等输入法的实际表现仍取决于桌面环境。
  - **主屏与备用屏（Main Screen / Alt Screen）**：支持历史滚动、选区松开复制与全屏 TUI 重绘。应用启用鼠标报告时可转发点击和滚轮，`Shift` 拖拽可强制本地选区。tmux 历史上限配置为 50,000 行，不代表每次重连都能完整回放这些内容。

![Terminal Rendering and Font Metrics](docs/images/terminal-core.png)

---

### 3. 递归窗格布局（Recursive PaneTree）
- **显式控制，告别意外分屏**：终端标签栏右侧 `＋` 或快捷键 `Platform+Alt+↑` 在同 Pane 创建默认终端预设的标签页；`Platform+Alt+→` / `Platform+Alt+↓` 在右侧 / 下方新建默认终端。
- **递归分屏与自适应比例**：
  - 支持水平（Horizontal）与垂直（Vertical）任意层级嵌套分屏；
  - 2px 精密分割线拖拽实时调整比例，自适应视口尺寸，窗格关闭后自动向内折叠父级，布局比例重启持久化保留；
  - 支持单窗格一键全屏聚焦（Maximize）与还原。

---

### 4. 会话弹出为独立 OS 窗口

主窗口始终是「侧栏 + 内容区」，应用不会主动改变主窗口尺寸；每个会话都可以在主窗口内平铺，也可以单独弹出为一个真实的系统窗口，由窗口管理器负责移动、缩放、最大化、跨屏与 Alt-Tab 切换。

- **弹出 / 收回**：侧栏会话右键、内容区 tab 右键都有「弹出到独立窗口」/「收回主窗口」。弹出后该会话从内容区消失，侧栏上带一个弹出标记；点击侧栏项或 `Platform+1..9` 会置顶对应系统窗口，而不是在主窗口打开。
- **全部弹出 / 全部收回**：侧栏底部一个按钮，作用于所有机器、所有项目的会话；对应 `DetachAllSessions` / `ReattachAllSessions` 两个可绑定动作，在主窗口和任何弹出窗口里都能触发。
- **关闭弹出窗口 = 收回**：系统关闭按钮、窗口内的「关闭标签页」快捷键（默认 `Ctrl+W`）或被系统关掉，都只把会话收回主窗口，不终止后台会话；窗口位置和大小会记住，下次弹出还在原位。真正终止会话仍走右键菜单。
- 弹出窗口里的分屏、新建会话、命令面板、切换 tab 等快捷键会转发回主窗口执行。
- 弹出状态随工作区保存；旧版本的 Tiled/Floating 全局开关和内部浮动桌面已移除，旧配置可直接加载。

### 5. 会话持久化与重连
- **tmux 常驻会话**：应用创建的本地会话由独立的 `tmux -L muxlane` server 承载。退出 GUI 会分离客户端，后台会话可继续运行；这不等于终止标签页，也不保证机器重启、tmux 被杀或会话自身退出后任务仍存活。
- **恢复与回放**：启动时恢复仍存活的 tmux 会话，使用 `capture-pane` 和有限容量的回放缓冲回填终端内容。它不是无限历史存档，断线期间的完整输出不作保证。

---

### 6. SSH 远程机器与 Agent
- **SSH 连接**：支持 `~/.ssh/config` 别名、指定私钥文件或界面中的账号密码认证，通过 SSH 转发远端 Unix socket；需要已有 SSH 网络可达性与认证配置，不提供额外的网络穿透服务。
- **远程服务管理**：
  - 自动探测远程机器环境（支持区分离线、未启动、未安装等细粒度状态）；
  - 用户确认后可上传兼容的 Muxlane 二进制并以 `--headless` 启动，具体限制见下文；
  - 离线保留缓存快照和布局以便重连；移除远程机器连接清理本地记录，不发送会话终止请求。删除远程项目或会话则是终止操作。
- **Agent Hook**：集成 Claude / Codex / OpenCode / Pi 等状态上报与结果通知。可在设置中控制桌面通知与声音，Hook 可用 `MUXLANE_HOOKS=off` 禁用；报告内容取决于各 Agent 的事件支持。
- **远程已读写回**：查看远端会话的完成/失败结果后，通过 `agent.mark_seen` RPC 真正写回远端服务并随其持久化，本地客户端重启后不会重新提醒；这需要远端 Muxlane 也是支持该能力的版本（`system.hello` 协商），旧版远端会自动降级为仅本次会话内已读。

---

## 快速开始

### 依赖环境
- **平台**：面向 Linux（X11 / Wayland）与 macOS。常规 CI 在 Ubuntu 上执行检查；发布工作流配置了 Linux 归档/AppImage 和 macOS arm64、x86_64 归档，不代表每种桌面环境均已验收。当前不发布 Windows 版本，服务端/客户端依赖 Unix socket，运行依赖 tmux。
- **构建工具链**：安装 rustup，使用仓库 [rust-toolchain.toml](rust-toolchain.toml) 固定的 **Rust 1.97.1**（含 rustfmt、clippy），与 CI 一致；不声明另一个未经验证的最低 Rust 版本。首次构建需下载 crates 与固定 revision 的 GPUI Git 依赖。
- **运行依赖**：tmux（建议使用较新版本）、可执行的 Shell、系统字体。Linux GUI 还需要图形会话、可用的 GPU/渲染驱动及对应动态库；Headless 不创建窗口，但仍使用同一二进制，其动态库依赖不会因此消失。
- **可选功能依赖**：SSH 连接需本地 OpenSSH 客户端与远端 SSH 服务；Claude/Codex 等 Hook 报告脚本需要 Node.js，Agent CLI 及其登录配置需在实际运行会话的机器上准备。

Ubuntu / Debian 构建依赖示例（合并两份 Linux CI 的依赖；其他发行版需换成对应包名）：

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential clang lld cmake pkg-config tmux \
  libfontconfig1-dev libfreetype6-dev libx11-dev libx11-xcb-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libasound2-dev \
  libwayland-dev wayland-protocols libxcb1-dev libxcb-render0-dev \
  libxcb-shape0-dev libxcb-xfixes0-dev libvulkan-dev libdbus-1-dev
```

macOS 源码构建需 Xcode Command Line Tools、rustup 与 tmux（例如通过 Homebrew 安装）；Linux 的 apt 包与安装脚本不适用于 macOS。

### 构建与本地运行

```bash
# 调试运行
cargo run -p muxlane-app

# 使用指定 Shell 启动
MUXLANE_SHELL=/usr/bin/zsh cargo run -p muxlane-app

# 编译优化发布版本
cargo build --release -p muxlane-app
./target/release/muxlane
```

### 运行 Headless 服务端

在依赖齐全的无桌面服务器或远程工作站上运行（前台进程）：

```bash
muxlane --headless
```

数据目录为 `$XDG_DATA_HOME/muxlane`，未设置时使用 `~/.local/share/muxlane`（macOS 也遵循此规则），其中包含 `state.json`、`muxlane.sock` 和认证数据。不要公开或随意删除该目录。需要长期运行时由服务管理器托管；此命令不会自行安装开机启动服务。

GUI 自带本机服务端，不要让 GUI 与 Headless 同时占用同一数据目录/socket。`--headless --connect ...` 不会启动远程连接管理，远程连接在 GUI 启动路径中建立。

### 连接远程机器

先确保 `ssh my-dev-server` 能成功连接并完成主机密钥确认。CLI 使用 SSH config 的非交互认证（可配合 ssh-agent）；自定义端口放在 `~/.ssh/config` 的 `Port` 中，不要写成 `host:端口`。

```bash
muxlane --connect my-dev-server
muxlane --connect user@192.168.1.100

# 显式远端 socket（路径按远端实际数据目录填写）
muxlane --connect user@host:/home/user/.local/share/muxlane/muxlane.sock

# 本地/已转发的 Unix socket，以及逗号分隔的多个目标
muxlane --connect /tmp/forwarded-muxlane.sock
muxlane --connect my-dev-server,other-server
```

仅传主机别名或 `user@host` 时会自动探测远端默认数据目录。也可在桌面 **「连接」** 弹窗选择 SSH config、公钥认证（私钥路径）或密码认证；当前密码路径调用本地 `setsid`，macOS 默认不具备该工具，建议使用 SSH config/密钥。

**远程部署限制**：自动安装上传本地二进制（优先候选 `target/release/muxlane`，否则当前可执行文件），不做跨平台构建或自动下载匹配架构的版本。远端必须兼容其 OS、CPU 架构和动态库；必要时先构建匹配产物，并通过 `MUXLANE_BOOTSTRAP_BINARY=/path/to/muxlane` 指定上传文件。此变量指向原始可执行文件，不是 tar.gz 或 AppImage。

探测/启动脚本依赖远端 `flock`，上传需要 `gzip` 或 `gunzip`，启动使用 `nohup`；会话仍需远端 tmux 与 Shell。部署到远端数据目录的 `bin/muxlane`，日志在 `logs/headless.log`，不自动安装这些系统依赖。macOS 默认缺少 `flock`，不能将 Linux 自动部署流程视为开箱即用。升级会重启远端 Muxlane 服务，且缺少 `fuser` 时回退到按进程名 `pkill`，可能影响同用户的其他 Muxlane 实例，执行前应检查远端环境。

---

## 打包与安装

```bash
# 在仓库根目录制作 Linux 分发归档（脚本会构建 release）
scripts/package-linux.sh
# 当前 x86_64 主机产物：dist/muxlane-0.0.4-linux-x86_64.tar.gz
# 版本读取 Cargo.toml，架构读取 uname -m；同时生成 .sha256

# 安装到本地用户环境 (~/.local/bin)
PREFIX="$HOME/.local" packaging/install.sh target/release/muxlane
```

Linux 归档解压后，也可在解压目录运行 `PREFIX="$HOME/.local" ./install.sh`，安装二进制、桌面入口与图标；确保 `$HOME/.local/bin` 在 `PATH` 中。下载发布产物后先在归档所在目录运行 `sha256sum -c <归档名>.sha256` 校验。

`scripts/package-appimage.sh` 生成 AppImage，首次会下载 appimagetool；它会复用已有 release 二进制，需要先自行重新构建以避免打包旧代码。当前脚本不打包 tmux 或完整动态库依赖，tar.gz/AppImage 都不代表任意 Linux 系统可直接运行。

macOS 发布归档只有命令行可执行文件与 README，没有 `.app` 安装包；解压后可直接运行 `./muxlane`，或用 `install -m755 muxlane "$HOME/.local/bin/muxlane"` 安装到已创建的用户目录。

---

## 开发与验证

以下是本地检查入口；CI 配置见 [.github/workflows/ci.yml](.github/workflows/ci.yml) 与 [check.yml](.github/workflows/check.yml)。命令列出不表示当前工作区已全部运行通过。

```bash
# 代码风格格式化校验
cargo fmt --all -- --check

# 全仓 Clippy 静态代码检查
cargo clippy --workspace --all-targets -- -D warnings

# 全工作区单元与集成测试（覆盖 core / term / client / server / store / app）
cargo test --workspace -- --test-threads=1

# Linux 打包、安装及 headless socket 冒烟检查
scripts/release-smoke.sh

# 有桌面环境时，运行交互 UI 冒烟检查
scripts/ui-smoke.sh
```

UI smoke 面向可操作的 X11 桌面，需要 `wmctrl`、`xdotool`、ImageMagick 的 `import`、Python 3 + Pillow、tmux、Node.js、Fcitx5（含拼音）以及默认 `/usr/bin/zsh`（可用 `MUXLANE_TEST_SHELL` 覆盖）。脚本会切换工作区、注入键鼠和切换输入法，使用临时数据目录并将截图写入 `artifacts/ui-smoke`；请在专用桌面会话中运行，不视作“无损”或 Wayland/macOS 验证。

会话弹出/收回的进程内 GPUI 测试见 [floating_tests.rs](crates/muxlane-app/src/floating_tests.rs)，远程 RPC 与运行时的回归见 [remote_operation_tests.rs](crates/muxlane-app/src/remote_operation_tests.rs)，通知已读合并见 [notification_tests.rs](crates/muxlane-app/src/notification_tests.rs)：它们使用模拟终端输入与进程内 server，不启动真实 PTY/SSH，不能替代窗口管理器、跨屏与 IME 的真实桌面检查。`muxlane-app` 的 GPUI 测试建议以 `--test-threads=1` 运行，并行时存在与窗口尺寸相关的既有偶发失败。[ACCEPTANCE.md](ACCEPTANCE.md) 收录分阶段验收记录与待验事项，需按日期和范围阅读，不能视为当前工作区全部功能的验证结论。

---

## 常用快捷键与设置

| 快捷键 | 功能描述 |
| :--- | :--- |
| `Platform + K` | 唤起应用内命令面板（启动 Agent / 分屏命令） |
| `Ctrl + W` | 主窗口：关闭活动标签页并终止会话；弹出窗口内：仅收回主窗口（同一可配置绑定） |
| `Platform + Alt + K` / `Platform + Alt + J` | 沿侧栏会话顺序切换上一个 / 下一个标签页（可配置） |
| `Ctrl + Shift + Tab` / `Ctrl + Tab` | 沿侧栏顺序切换上一个 / 下一个会话 |
| `Platform + Alt + ↑` | 当前窗格新建默认终端标签页（可配置） |
| `Platform + Alt + →` / `Platform + Alt + ↓` | 右侧 / 下方新建默认终端（可配置） |
| `Platform + 1` ... `Platform + 9` | 选择当前窗格的对应标签页；若已弹出则置顶其系统窗口 |
| `Ctrl + C` (选区存在时) | 复制选中文本到系统剪贴板 |
| `Ctrl + C` (无选区时) | 发送终端控制字符，由 Shell/TUI 决定是否作为中断处理 |
| `Ctrl + V` | 粘贴剪贴板；应用启用 Bracketed Paste 时发送对应包裹序列 |
| `Shift + 鼠标左键拖拽` | 强制使用本地终端文本划词，松开自动复制（即使在 Vim/Htop 等应用内） |
| `Esc` | 快速退出通知中心浮层、下拉菜单或弹出层 |

`Platform` 在 Linux 上为 Win（Super），macOS 上为 Command；这些是应用内快捷键，桌面环境可能先拦截某些组合键。设置页的“快捷键”区域可录入六项可配置动作的单个组合键、清空以禁用单项，或整组恢复默认；修改会立即生效并在重启后保留。固定的命令面板、Ctrl+Tab 与数字选项卡快捷键不在该配置范围内，冲突绑定会被拒绝。应用级快捷键优先于终端输入，清空或改绑后，对应按键会恢复为终端输入。没有全局 F6 / Shift+F6 工作区域切换键，设置页内原有 Tab / F6 焦点循环保留。

设置页“通用”的“默认终端预设”可选择 Shell、Claude Code、Codex、Pi、OpenCode 等内置终端预设，缺省为 Shell。选择适用于新建终端标签页、终端 `＋` 和显式分屏；远端使用同一预设在远端解析程序。列表不按本地安装状态过滤，未安装的有效预设启动失败时会显示错误，不会静默换成 Shell。

---

## 仓库入口

- [muxlane-app](crates/muxlane-app)：GPUI 界面、布局、设置、通知及 CLI 入口。
- [muxlane-core](crates/muxlane-core)：模型、协议、PaneTree、Agent 预设与 Hook。
- [muxlane-term](crates/muxlane-term)：PTY、tmux、终端状态及回放。
- [muxlane-server](crates/muxlane-server) / [muxlane-client](crates/muxlane-client)：本机 socket 服务与 SSH 远程客户端。
- [muxlane-store](crates/muxlane-store)：持久化状态和迁移；[scripts](scripts) / [packaging](packaging)：检查、打包和安装脚本。
- [发布工作流](.github/workflows/release.yml)：平台产物与版本校验规则。

## 开源许可

本项目遵循 [MIT 开源许可证](LICENSE)。欢迎提 Issue 与 PR 共同演进！
