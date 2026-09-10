use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    Chinese,
    English,
}

impl Language {
    pub const ALL: [Self; 2] = [Self::Chinese, Self::English];

    pub fn id(self) -> &'static str {
        match self {
            Self::Chinese => "zh",
            Self::English => "en",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Chinese => "中文",
            Self::English => "English",
        }
    }

    pub fn from_id(value: &str) -> Option<Self> {
        match value {
            "zh" | "zh_cn" | "zh-CN" => Some(Self::Chinese),
            "en" | "en_us" | "en-US" => Some(Self::English),
            _ => None,
        }
    }

    pub fn detect() -> Self {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .filter_map(|key| std::env::var(key).ok())
            .find_map(|value| {
                let value = value.trim().to_ascii_lowercase();
                if value.starts_with("zh") {
                    Some(Self::Chinese)
                } else if value.starts_with("en") {
                    Some(Self::English)
                } else {
                    None
                }
            })
            .unwrap_or_default()
    }
}

const TRANSLATIONS: &[(&str, &str, &str)] = &[
    ("common.add", "添加", "Add"),
    ("common.adding", "添加中…", "Adding…"),
    ("common.cancel", "取消", "Cancel"),
    ("common.clear", "清空", "Clear"),
    ("common.connect", "连接", "Connect"),
    ("common.off", "关闭", "Off"),
    ("common.on", "开启", "On"),
    ("common.send", "发送", "Send"),
    ("common.settings", "设置", "Settings"),
    ("dialog.add_local_project", "添加本地项目", "Add Local Project"),
    (
        "dialog.add_local_project_help",
        "输入项目目录；目录不存在时可确认创建",
        "Enter a project directory. You can confirm creation if it does not exist.",
    ),
    (
        "dialog.add_remote_project",
        "在 {host} 添加项目",
        "Add Project on {host}",
    ),
    (
        "dialog.add_remote_project_help",
        "输入远端项目目录；目录不存在且远端支持时可确认创建",
        "Enter a remote project directory. If supported, you can confirm creation when it is missing.",
    ),
    ("dialog.auth_password", "用户名密码", "Password"),
    ("dialog.auth_public_key", "SSH 公钥", "SSH Key"),
    ("dialog.auth_ssh_config", "SSH 配置", "SSH Config"),
    ("dialog.connect_remote", "连接远程机器", "Connect to Remote"),
    (
        "dialog.connect_remote_help",
        "输入 SSH Host 或 ~/.ssh/config 别名；socket 自动发现",
        "Enter an SSH host or ~/.ssh/config alias. The socket is detected automatically.",
    ),
    (
        "error.create_session",
        "创建会话失败：{error}",
        "Failed to create session: {error}",
    ),
    (
        "error.create_shell",
        "创建 Shell 失败：{error}",
        "Failed to create shell: {error}",
    ),
    (
        "error.delete_remote_sessions",
        "{count} 个远端 tmux 会话未能销毁，项目仍保留",
        "Failed to destroy {count} remote tmux session(s). The project was kept.",
    ),
    (
        "error.delete_sessions_project",
        "{count} 个 tmux 会话未能销毁，项目仍保留",
        "Failed to destroy {count} tmux session(s). The project was kept.",
    ),
    (
        "error.delete_sessions_session",
        "{count} 个 tmux 会话未能销毁，会话仍保留",
        "Failed to destroy {count} tmux session(s). The session was kept.",
    ),
    (
        "error.delete_session",
        "删除会话失败：{error}",
        "Failed to delete session: {error}",
    ),
    (
        "error.local_project_missing",
        "本地项目已不存在，请重新选择项目",
        "The local project no longer exists. Select another project.",
    ),
    (
        "error.local_project_required",
        "请输入本地项目目录",
        "Enter a local project directory.",
    ),
    (
        "error.password_credentials_required",
        "密码连接需要用户名和密码",
        "A username and password are required.",
    ),
    (
        "error.remote_create_directory_unsupported",
        "远端版本不支持创建缺失目录，请先更新远端 Muxlane 或手动创建目录",
        "This remote version cannot create missing directories. Update Muxlane remotely or create the directory manually.",
    ),
    (
        "error.remote_create_session",
        "远程创建会话失败：{error}",
        "Failed to create remote session: {error}",
    ),
    (
        "error.remote_machine_missing",
        "远程机器已不存在",
        "The remote machine no longer exists.",
    ),
    (
        "error.remote_not_connected",
        "远端尚未连接",
        "The remote is not connected.",
    ),
    (
        "error.remote_project_required",
        "请输入远端项目目录",
        "Enter a remote project directory.",
    ),
    (
        "error.remote_unavailable_for_delete",
        "远端当前不可连接，未执行删除",
        "The remote is unavailable. Nothing was deleted.",
    ),
    (
        "error.ssh_target_required",
        "请输入 SSH host 或别名",
        "Enter an SSH host or alias.",
    ),
    (
        "main.state_load_failed",
        "muxlane: 无法读取状态文件 {path}: {error}\n为避免覆盖原数据，启动已中止。",
        "muxlane: failed to read state file {path}: {error}\nStartup was aborted to avoid overwriting existing data.",
    ),
    ("menu.add_remote_project", "添加远程项目…", "Add Remote Project…"),
    ("menu.open_in_vscode", "在 VS Code 中打开", "Open in VS Code"),
    ("menu.open_in_zed", "在 Zed 中打开", "Open in Zed"),
    (
        "menu.open_in_idea",
        "在 IntelliJ IDEA 中打开",
        "Open in IntelliJ IDEA",
    ),
    (
        "error.open_editor",
        "无法打开编辑器：{error}",
        "Failed to open editor: {error}",
    ),
    ("menu.confirm_delete", "确认删除", "Delete"),
    ("menu.delete_project", "删除项目", "Delete Project"),
    ("menu.delete_project_ellipsis", "删除项目…", "Delete Project…"),
    ("menu.delete_remote_machine", "删除远程机器…", "Delete Remote…"),
    (
        "menu.delete_remote_machine_title",
        "删除远程机器连接",
        "Delete Remote Connection",
    ),
    (
        "menu.delete_remote_project",
        "删除远程项目…",
        "Delete Remote Project…",
    ),
    (
        "menu.delete_remote_session",
        "删除远程会话",
        "Delete Remote Session",
    ),
    ("menu.delete_session", "删除会话", "Delete Session"),
    ("menu.detach_session", "弹出到独立窗口", "Detach to Window"),
    ("menu.reattach_session", "收回主窗口", "Reattach to Main Window"),
    ("sidebar.detach_all", "全部弹出", "Detach All"),
    ("sidebar.reattach_all", "全部收回", "Reattach All"),
    (
        "detached.placeholder",
        "{count} 个会话已弹出到独立窗口，从左侧选择或右键收回",
        "{count} session(s) detached to their own windows. Pick one on the left or right-click to reattach.",
    ),
    (
        "detached.empty",
        "没有打开的会话",
        "No open sessions",
    ),
    ("menu.deleting", "删除中…", "Deleting…"),
    (
        "menu.refresh_remote",
        "刷新项目和会话列表",
        "Refresh Projects and Sessions",
    ),
    ("menu.reconnect", "重新连接", "Reconnect"),
    (
        "error.refresh_remote",
        "刷新远端项目和会话失败：{error}",
        "Failed to refresh remote projects and sessions: {error}",
    ),
    (
        "menu.reinstall_remote",
        "重新部署 / 安装远端 Muxlane…",
        "Redeploy / Install Remote Muxlane…",
    ),
    (
        "menu.upgrade_remote",
        "更新远端 Muxlane…",
        "Update Remote Muxlane…",
    ),
    ("confirm.create_directory_title", "创建目录", "Create Directory"),
    (
        "confirm.create_directory_copy",
        "目录 {path} 不存在。是否递归创建并添加为项目？",
        "The directory {path} does not exist. Create it recursively and add it as a project?",
    ),
    ("confirm.creating", "创建中…", "Creating…"),
    ("confirm.create", "创建并添加", "Create and Add"),
    (
        "confirm.delete_project_copy",
        "将结束 {count} 个 Muxlane 会话（终端或 UI）。项目文件和用户默认 tmux 不会删除。",
        "This will end {count} Muxlane session(s), terminal or UI. Project files and user tmux sessions will not be deleted.",
    ),
    (
        "confirm.delete_remote_copy",
        "只删除本地连接、镜像和 tunnel；目标机器上的项目、session 与 tmux 全部保留。",
        "Only the local connection, mirror, and tunnel will be deleted. Projects, sessions, and tmux processes on the remote machine will be kept.",
    ),
    ("confirm.quit_title", "退出 Muxlane？", "Quit Muxlane?"),
    (
        "confirm.quit_copy",
        "确定要退出 Muxlane 吗？当前终端会话将保持运行。",
        "Quit Muxlane? Current terminal sessions will keep running.",
    ),
    ("confirm.quit", "退出 Muxlane", "Quit Muxlane"),
    ("bootstrap.action.install", "安装并启动", "Install and Start"),
    ("bootstrap.action.start", "启动并重连", "Start and Reconnect"),
    ("bootstrap.action.upgrade", "更新并重启", "Update and Restart"),
    (
        "bootstrap.confirm_title",
        "{action}远端 Muxlane",
        "{action} Remote Muxlane",
    ),
    (
        "bootstrap.description.install",
        "SSH 已连接到 {host}。将使用当前认证方式上传当前 Muxlane 并启动 headless 进程。",
        "SSH is connected to {host}. The current authentication method will be used to upload Muxlane and start the headless process.",
    ),
    (
        "bootstrap.description.start",
        "SSH 已连接到 {host}。将使用当前认证方式启动 headless 进程。",
        "SSH is connected to {host}. The current authentication method will be used to start the headless process.",
    ),
    (
        "bootstrap.description.upgrade",
        "SSH 已连接到 {host}。将使用当前认证方式上传新版本并重启 headless 进程。",
        "SSH is connected to {host}. The current authentication method will be used to upload the new version and restart the headless process.",
    ),
    ("bootstrap.install", "安装…", "Install…"),
    ("bootstrap.phase.install", "安装", "Installing"),
    ("bootstrap.phase.restart", "重启服务", "Restarting Service"),
    ("bootstrap.phase.upload", "上传二进制", "Uploading Binary"),
    ("bootstrap.start", "启动…", "Start…"),
    ("bootstrap.update", "更新…", "Update…"),
    ("notification.center", "通知中心", "Notifications"),
    ("notification.clear", "清空", "Clear"),
    (
        "notification.click_to_open",
        "点击直达会话",
        "Click to open session",
    ),
    ("notification.empty", "暂无通知", "No notifications"),
    (
        "notification.title_done",
        "{machine} · {project} 任务完成",
        "{machine} · {project} Task completed",
    ),
    (
        "notification.title_failed",
        "{machine} · {project} 任务失败",
        "{machine} · {project} Task failed",
    ),
    ("notification.title_input",
        "{machine} · {project} 等待输入",
        "{machine} · {project} Input required",
    ),
    (
        "pane.select_agent",
        "从左侧选择 agent 打开 tab",
        "Select an agent from the sidebar to open a tab",
    ),
    (
        "palette.clear_notifications",
        "清空所有通知",
        "Clear All Notifications",
    ),
    ("palette.close_split", "关闭当前分屏", "Close Current Pane"),
    (
        "palette.connect_remote",
        "连接远程开发机…",
        "Connect to Remote…",
    ),
    ("palette.horizontal_split", "水平分屏", "Split Right"),
    (
        "palette.maximize",
        "最大化 / 还原当前面板",
        "Maximize / Restore Current Pane",
    ),
    ("palette.new", "新建 {name}", "New {name}"),
    ("palette.no_results", "无匹配结果", "No Results"),
    (
        "palette.placeholder",
        "输入命令、项目名或 Agent 名…",
        "Type a command, project, or agent name…",
    ),
    ("palette.toggle_dark", "切换为深色模式", "Switch to Dark Mode"),
    (
        "palette.tab_switch_column",
        "Tab 切换项目 / Agent · Shift+Tab 切换 UI / 终端 · Enter 确认",
        "Tab: switch project / agent · Shift+Tab: UI / terminal · Enter: confirm",
    ),
    (
        "palette.toggle_light",
        "切换为浅色模式",
        "Switch to Light Mode",
    ),
    ("palette.vertical_split", "垂直分屏", "Split Down"),
    ("placeholder.password", "密码", "Password"),
    (
        "placeholder.private_key",
        "私钥路径（可选）",
        "Private key path (optional)",
    ),
    (
        "placeholder.remote_target",
        "nuc 或 192.168.1.20",
        "nuc or 192.168.1.20",
    ),
    (
        "placeholder.username",
        "用户名（可选）",
        "Username (optional)",
    ),
    ("relative.days_ago", "{count}天前", "{count}d ago"),
    ("relative.hours_ago", "{count}小时前", "{count}h ago"),
    ("relative.just_now", "刚刚", "just now"),
    ("relative.minutes_ago", "{count}分钟前", "{count}m ago"),
    ("relative.seconds_ago", "{count}秒前", "{count}s ago"),
    ("sidebar.connect_remote", "连接远程机器", "Connect to Remote"),
    ("sidebar.hide", "隐藏侧栏", "Hide Sidebar"),
    ("sidebar.show", "显示侧栏", "Show Sidebar"),
    (
        "sidebar.expand_project",
        "展开项目目录",
        "Expand Project Directory",
    ),
    (
        "sidebar.collapse_project",
        "隐藏项目目录",
        "Collapse Project Directory",
    ),
    ("sidebar.machines", "机器", "Machines"),
    ("sidebar.notifications", "通知", "Notifications"),
    ("status.auth_failed", "认证失败", "Authentication Failed"),
    ("status.connected", "已连接", "Connected"),
    ("status.connecting", "连接中", "Connecting"),
    ("status.disconnected", "已断开", "Disconnected"),
    ("status.failed", "失败", "Failed"),
    ("status.idle", "空闲", "Idle"),
    ("status.input_required", "等待输入", "Input Required"),
    ("status.needs_install", "未安装", "Not Installed"),
    ("status.needs_start", "未启动", "Not Running"),
    ("status.needs_update", "需要更新", "Update Required"),
    ("status.offline", "离线", "Offline"),
    ("status.task_completed", "任务完成", "Task Completed"),
    ("status.task_completed_body", "任务已完成", "Task completed"),
    ("status.working", "执行中", "Working"),
    ("status.remote_ssh_probe", "SSH 探测", "Checking SSH"),
    ("status.remote_subscribe", "订阅状态", "Subscribing"),
    ("status.remote_tunnel", "建立 tunnel", "Opening Tunnel"),
    ("settings.general", "通用", "General"),
    ("settings.appearance", "外观", "Appearance"),
    (
        "settings.project_workspaces",
        "一项目一工作区",
        "One Workspace per Project",
    ),
    (
        "settings.project_workspaces_help",
        "每个项目独立保存分屏和标签页；关闭后使用共享布局",
        "Keep panes and tabs separate for each project. Disabled mode uses a shared layout.",
    ),
    ("settings.shortcuts", "快捷键", "Shortcuts"),
    (
        "settings.shortcuts_help",
        "点击输入框后按下新快捷键，Esc 取消",
        "Click a field and press the new shortcut; Esc cancels",
    ),
    ("settings.shortcuts_restore", "恢复默认", "Restore Defaults"),
    (
        "settings.shortcut.close_tab",
        "关闭当前标签页/会话",
        "Close Current Tab/Session",
    ),
    (
        "settings.shortcut.previous_workspace",
        "上一个工作区",
        "Previous Workspace",
    ),
    (
        "settings.shortcut.next_workspace",
        "下一个工作区",
        "Next Workspace",
    ),
    (
        "settings.shortcut.previous_tab",
        "上一个标签页",
        "Previous Tab",
    ),
    (
        "settings.shortcut.next_tab",
        "下一个标签页",
        "Next Tab",
    ),
    (
        "settings.shortcut_recording",
        "请按一个组合键",
        "Press One Shortcut",
    ),
    ("settings.shortcut_disabled", "已禁用", "Disabled"),
    (
        "settings.shortcut_error_invalid",
        "该组合键无效",
        "That shortcut is invalid.",
    ),
    (
        "settings.shortcut_error_multiple",
        "仅支持单个组合键",
        "Only one shortcut chord is supported.",
    ),
    (
        "settings.shortcut_error_conflict",
        "组合键 {shortcut} 已被占用",
        "Shortcut {shortcut} is already assigned.",
    ),
    ("settings.interface_theme", "界面主题", "Interface Theme"),
    (
        "settings.interface_theme_help",
        "选择界面配色方案",
        "Choose the interface color scheme",
    ),
    ("settings.language", "语言", "Language"),
    ("settings.default_terminal_preset", "默认终端预设", "Default Terminal Preset"),
    ("settings.shortcut.new_tab", "新建默认终端标签页", "New Default Terminal Tab"),
    ("settings.shortcut.split_right", "右侧新建默认终端", "New Default Terminal on Right"),
    ("settings.shortcut.split_down", "下方新建默认终端", "New Default Terminal Below"),
    (
        "settings.language_help",
        "界面显示语言",
        "Interface display language",
    ),
    (
        "settings.notification_sound",
        "通知声音",
        "Notification Sound",
    ),
    (
        "settings.notification_sound_help",
        "任务完成或有通知时播放提示音",
        "Play a sound when tasks finish or notify",
    ),
    (
        "settings.osc52",
        "允许终端写入剪贴板 (OSC52)",
        "Allow Terminal Clipboard Writes (OSC52)",
    ),
    (
        "settings.osc52_help",
        "允许终端内程序读写系统剪贴板",
        "Allow terminal programs to use the system clipboard",
    ),
    ("settings.terminal_font", "终端字体", "Terminal Font"),
    (
        "settings.terminal_font_help",
        "终端使用的等宽字体",
        "Monospace font used in terminals",
    ),
    ("settings.ui_scale", "界面缩放", "Interface Scale"),
    (
        "settings.ui_scale_help",
        "输入 75–200 的整数百分比，按回车或应用；侧栏宽度独立调整",
        "Enter a whole percentage from 75–200, then press Enter or Apply; sidebar width is independent",
    ),
    ("settings.ui_scale_apply", "应用", "Apply"),
    (
        "settings.ui_scale_invalid",
        "请输入 75–200 之间的整数",
        "Enter a whole number from 75 to 200",
    ),
    (
        "terminal.attaching",
        "正在 attach 远程终端…",
        "Attaching to remote terminal…",
    ),
    (
        "terminal.reconnecting",
        "镜像流断开，正在重连…",
        "Mirror stream disconnected. Reconnecting…",
    ),
    ("theme.dark", "墨渊", "Ink"),
    ("theme.jade", "竹青", "Jade"),
    ("theme.light", "雾白瓷", "Porcelain"),
    ("theme.one_dark", "代码墨", "One Dark"),
    ("theme.paper", "纸暖", "Paper Warm"),
    ("theme.sakura", "樱粉", "Sakura"),
    ("theme.sky", "霁蓝", "Clear Sky"),
    ("theme.synthwave", "夜霓", "Synthwave"),
];

pub fn text(language: Language, key: &str) -> &str {
    for &(candidate, chinese, english) in TRANSLATIONS {
        if candidate == key {
            return match language {
                Language::Chinese => chinese,
                Language::English => english,
            };
        }
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_ids_translations_and_fallback_are_stable() {
        assert_eq!(Language::from_id("zh"), Some(Language::Chinese));
        assert_eq!(Language::from_id("en-US"), Some(Language::English));
        assert_eq!(text(Language::English, "common.settings"), "Settings");
        assert_eq!(text(Language::Chinese, "missing.key"), "missing.key");
    }
}
