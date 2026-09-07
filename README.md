# Muxlane

> **原生 Rust + GPUI 多 Agent 工作台与高性能终端客户端**
> 专为 AI Coding Agent、多项目协同、分布式远程机器管理而生。每台机器既是轻量客户端也是高内聚服务端；一次连接，自动发现远端实例上的所有机器、项目与常驻会话。

---

![Muxlane Workbench Split View](docs/images/workbench-split.png)

---

## ✨ 核心特性

### 1. 现代工作台设计与极致视觉体验
- **原生 GPUI 驱动**：毫秒级响应，告别 Electron 臃肿内存开销，提供极度顺滑的 60/120 FPS 交互动画。
- **直观层级侧边树**：机器（Machine）→ 项目（Project）→ 会话（Session）清晰拓扑结构展示，支持项目多选展开折叠与会话实时 OSC 动态标题。
- **灵动胶囊通知浮岛（Dynamic Capsule Popover）**：
  - 侧栏底部轻盈的通知入口，实时呈现未读计数与呼吸微光状态（Working 脉冲 / Blocked 高警示微光）；
  - 浮层式通知中心，记录各 Agent 任务流转（执行中、等待输入、任务完成）与相对时间，支持一键点击直达对应终端窗格。
- **命令面板（Command Palette）**：全局 `Win/Cmd+K` 极速唤起，秒级检索并启动预设 Agent（Claude Code、Codex、Pi、Agy、OpenCode、Qwen 等）或快速分屏。

| 全局命令面板 (Win/Cmd+K) | 会话交互与菜单管理 |
|:---:|:---:|
| ![Command Palette](docs/images/command-palette.png) | ![Session Menu](docs/images/session-menu.png) |

---

### 2. 纯粹、可靠的硬核终端引擎
- **真终端架构**：底层由 `portable-pty` 与 `alacritty_terminal` 协同驱动：
  - **原生终端体验**：支持 `$SHELL` / `zsh`、`TERM=xterm-256color`、`COLORTERM=truecolor`，完美呈现 ANSI 256 色与 TrueColor 真彩色高亮。
  - **精确排版与输入法兼容**：真实字体度量（Font Metrics）+ Force-width Shaping；精确对齐 CJK 双宽字符与结合符（Combining Marks），完美支持 Fcitx5 / IBus 等 Linux 桌面中文 IME 提交。
  - **优雅的屏幕状态自适应（Main Screen vs Alt Screen）**：
    - **普通 Shell**：50,000 行平滑本地历史视口滚动（Scrollback），鼠标左键划词即选、松开即复制；
    - **全屏 TUI 应用（Vim / Htop / Codex / Pi）**：进入独立的备用屏模式（Alt Screen），鼠标滚轮与点击直接透传给应用（浏览对话、光标点选、列表滚动）；窗口尺寸改变（Resize）时原位重绘，从根本上解决旧终端画布撕裂与历史重影。

![Terminal Rendering and Font Metrics](docs/images/terminal-core.png)

---

### 3. 灵活递归窗格布局（Recursive PaneTree）
- **显式控制，告别意外分屏**：终端标签栏右侧 `＋` 或快捷键 `Platform+Alt+T` 在同 Pane 创建默认终端预设的标签页；`Platform+Alt+R` / `Platform+Alt+D` 在右侧 / 下方新建默认终端。ACP 标签栏 `＋` 仍继承当前 UI profile。
- **递归分屏与自适应比例**：
  - 支持水平（Horizontal）与垂直（Vertical）任意层级嵌套分屏；
  - 2px 精密分割线拖拽实时调整比例，自适应视口尺寸，窗格关闭后自动向内折叠父级，布局比例重启持久化保留；
  - 支持单窗格一键全屏聚焦（Maximize）与还原。

---

### 4. 会话持久化与零阻断重连
- **无感常驻后台**：本地所有会话运行在隔离的 muxlane 专属 tmux server 中。关闭桌面 GUI 仅仅是 Detach 客户端，后台编译、开发测试或 Agent 任务永不中断。
- **冷启动与重连历史自动回填**：创建窗口或断线重连时，通过 `capture-pane` 机制秒级回填完整的历史上下文缓冲区，告别重连后终端白板。

---

### 5. 分布式多机器互联与远程 Agent
- **零配置穿透连接**：支持通过 `~/.ssh/config` 别名、指定公钥或标准账号直连远程机器。
- **智能远程服务纳管**：
  - 自动探测远程机器环境（支持区分离线、未启动、未安装等细粒度状态）；
  - 支持一键将本地 `muxlane --headless` 上传并静默拉起远程守护服务；
  - 远程连接断开后仅清除本地缓存视图，绝对安全，绝不误杀远端运行中的任务会话。
- **标准化 Hook 链路**：深度集成 Claude / Codex / OpenCode / Pi 等 Agent 的状态汇报机制，精准抓取 Assistant 任务结果并推送全局桌面提醒。

---

## 🚀 快速开始

### 依赖环境
- Linux / macOS (Unix)
- Rust 1.80+ (推荐最新 stable)
- tmux 3.2+
- 系统字体与渲染依赖：`fontconfig`, `freetype`, `libxkbcommon`, `wayland` / `xcb` 相关开发包

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

### ACP 工具调用

工具行显示协议状态和工具类型；标题保留协议原文，折叠时只显示首行。Raw input/output 独立折叠，不根据工具名称或原始 JSON 推断执行命令。终端只在本机 HostServices 提供元数据时显示真实 command、argv 和 cwd；输出保留空白并横向滚动，退出码与截断提示可同时显示，不显示推测耗时。输出预览上限为 64 KiB，可复制完整的已保留输出（不包含上游已截去的字节）。

文件修改使用逐行 diff、三行上下文、旧新行号与增删统计。超过合计 64 KiB / 2000 行时不计算 diff；单个预览最多展示 600 行，并明确标记限制。可复制完整原文和新内容，路径仅展示，不自动打开远端路径。回滚仍受项目根目录与当前文件内容一致性检查约束。授权卡关联工具 ID 和已有 typed 内容，一次/始终/拒绝选项保持协议含义，重复提交不会再次发送。

### ACP Agent 发现与安装

原生 UI 会话支持用户自行安装的 **ACP stdio CLI 或 adapter**，普通交互式终端 CLI 不一定支持 ACP。新建会话的 UI 模式会在后台刷新 [ACP 官方目录](https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json)，仅获取元数据，不下载图标、归档或运行目录中的命令。目录响应限制为 2 MiB / 10 秒，仅接受 1.x schema；离线或刷新失败时保留已有目录与原子写入的 `acp-registry-cache.json` 缓存。

离线也提供 Claude、Codex、Pi、OpenCode 四项本地 recipe。OpenCode 无需手填配置，检测到 `opencode` 后用 `opencode acp` 启动。前三项分别检测 `claude-agent-acp`、`codex-acp`、`pi-acp`，不再默认使用可能下载软件的 `npx`。未找到命令时点击或按 Enter 只显示安装详情，不创建会话；安装命令只能复制，用户自行执行后点击“重新检测”。“刷新官方目录”和“重新检测”独立工作，没有自动安装或 shell 安装入口。

检测只查看当前 GUI 进程 PATH 中绝对目录下的文件（或显式绝对命令路径），检查文件及可执行位，不执行 `--help`、`--version`、`npx`、`uvx`，不搜索项目的 `node_modules`，忽略空和相对 PATH 项。“已找到”不代表已认证、已验证 ACP 能力或所有依赖可用。当前文件检测支持 Unix 平台。GUI 继承的 PATH 若在外部改变，需重启应用；同一 PATH 目录内新安装的命令可直接重新检测。

### 自定义 ACP CLI

未知官方目录条目显示“需手工配置”，Muxlane 不会猜测 npm bin、归档 basename 或启动参数。

配置文件为用户级 `$XDG_DATA_HOME/muxlane/agents.json`；未设置 `XDG_DATA_HOME` 时使用 `~/.local/share/muxlane/agents.json`。文件不存在时仍提供四项本地 recipe，不会读取或执行项目目录中的 `agents.json`。配置在启动和“重新检测”时读取；运行中的进程不变，后续启动/重连按当前配置解析。

```json
{
  "agents": [
    {
      "id": "my-acp-agent",
      "label": "My ACP Agent",
      "command": "/absolute/path/to/acp-stdio-adapter",
      "args": ["--config", "/absolute/path with spaces/adapter.json"],
      "env": {
        "MY_ADAPTER_MODE": "development"
      }
    }
  ]
}
```

此例仅展示配置结构，请按实际 adapter 文档填写程序、参数与环境变量。`args`、`env` 可省略。`command` 是单个可执行文件路径或 PATH 中的程序名；`args` 每项是一个原样传递的参数，`env` 覆盖子进程继承的环境变量。不经 shell 拼接，不展开 `$HOME`、`~`、管道或命令替换。只添加自己信任的 CLI，避免将密钥放入可公开分享的配置文件。

命令面板 UI 模式按 Claude、Codex、Pi、OpenCode、额外用户定义、其他官方目录条目排列。旧 id `claude`、`codex`、`pi` 保持兼容；官方 `claude-acp`、`codex-acp`、`pi-acp` 映射到旧 id。同名用户定义优先于本地 recipe 和目录，合并后不重复；自定义 id（含别名归一后）必须唯一且非空。配置有误时显示文件路径及错误，整份自定义列表不加载，本地 recipe 与目录发现仍可用。

会话持久化仍保存稳定的 `profile_id`。恢复、重连和子会话使用原 agent；删除或改名后的未知 id 保留历史、草稿与原 id，并显示不可用，不会回退启动其他 agent。重新添加同一 id 并“重新检测”后可重新解析，再重连会话。启动前重新检查本机命令并固定绝对路径。

已有 `MUXLANE_ACP_CLAUDE_COMMAND`、`MUXLANE_ACP_CODEX_COMMAND`、`MUXLANE_ACP_PI_COMMAND` 环境覆盖继续支持原命令字符串及 SDK 的结构化 JSON 格式；无效覆盖显示配置错误，不再静默改用默认程序。它们与用户 `agents.json` 都是显式可信启动配置，不受官方目录控制：用户自行配置 `npx`、shell 或其他 launcher 时仍可能下载或执行程序，“不自动安装”仅保证 Muxlane 的默认 recipe、发现/检测和安装详情不会自动安装。

### 运行 Headless 服务端

在无桌面界面的云服务器或远程工作站上直接运行：

```bash
muxlane --headless
```

### 连接远程机器

通过命令行快速附加：

```bash
muxlane --connect my-dev-server
muxlane --connect user@192.168.1.100
```

或直接在桌面界面中点击左下角 **「连接」** 按钮，根据弹窗输入 SSH 主机名与端口即可直观管理。

---

## 📦 打包与安装

```bash
# 制作 Linux 独立分发归档
scripts/package-linux.sh
# 产物输出至：dist/muxlane-0.1.0-linux-x86_64.tar.gz

# 安装到本地用户环境 (~/.local/bin)
PREFIX="$HOME/.local" packaging/install.sh target/release/muxlane
```

---

## 🧪 质量保障与自动化测试

本项目拥有严苛的自动化测试套件与全工作区流水线验证：

```bash
# 代码风格格式化校验
cargo fmt --all -- --check

# 全仓 Clippy 静态代码检查
cargo clippy --workspace --all-targets -- -D warnings

# 全工作区单元与集成测试（覆盖 core / term / client / server / store / app）
cargo test --workspace

# 运行独立桌面端无损 UI 端到端 Smoke 自动化验证
scripts/ui-smoke.sh
```

---

## ⌨️ 常用快捷键

| 快捷键 | 功能描述 |
| :--- | :--- |
| `Platform + K` | 唤起全局命令面板（启动 Agent / 分屏命令） |
| `Ctrl + W` | 关闭当前活动标签页并结束其会话（可配置） |
| `Platform + Alt + ↑` / `Platform + Alt + ↓` | 沿现有侧栏会话顺序切换上一个 / 下一个标签页（可配置） |
| `Ctrl + Shift + Tab` / `Ctrl + Tab` | 上一个 / 下一个标签页 |
| `Platform + Alt + T` | 在当前窗格新建默认终端标签页（可配置） |
| `Platform + Alt + R` / `Platform + Alt + D` | 右侧 / 下方新建默认终端（可配置） |
| `Platform + 1` ... `Platform + 9` | 选择当前窗格的对应标签页 |
| `Ctrl + C` (选区存在时) | 复制选中文本到系统剪贴板 |
| `Ctrl + C` (无选区时) | 向终端进程发送原生中断信号 (`SIGINT`) |
| `Ctrl + V` | 将剪贴板内容安全粘贴至当前终端（带 Bracketed Paste 保护） |
| `Shift + 鼠标左键拖拽` | 强制使用本地终端文本划词，松开自动复制（即使在 Vim/Htop 等应用内） |
| `Esc` | 快速退出通知中心浮层、下拉菜单或弹出层 |

`Platform` 在 Linux / Windows 上为 Win（Super），macOS 上为 Command。设置页的“快捷键”区域可录入单个组合键、清空以禁用单项，或整组恢复默认；修改会立即生效并在重启后保留。应用级快捷键优先于终端输入，清空或改绑后，对应按键会恢复为终端输入。没有全局 F6 / Shift+F6 工作区域切换键，设置页内原有 Tab / F6 焦点循环保留。

设置页“通用”的“默认终端预设”可选择 Shell、Claude Code、Codex、Pi、OpenCode 等内置终端预设，缺省为 Shell；这不是 bash / zsh 可执行文件设置，也不改变 ACP UI profiles。选择适用于新建终端标签页、终端 `＋` 和显式分屏；远端使用同一预设在远端解析程序。列表不按本地安装状态过滤，未安装的有效预设启动失败时会显示错误，不会静默换成 Shell。

---

## 📄 开源许可

本项目遵循 MIT 开源许可证。欢迎提 Issue 与 PR 共同演进！
