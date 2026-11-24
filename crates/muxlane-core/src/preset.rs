//! Agent preset 模型，保持最小而可扩展。
use crate::model::AgentType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentPreset {
    pub id: String,
    pub label: String,
    pub agent_type: AgentType,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// 二进制不存在时 UI 隐藏（Shell 永远可用）
    #[serde(default = "default_true")]
    pub require_installed: bool,
}
fn default_true() -> bool {
    true
}

impl AgentPreset {
    /// Check the process PATH. Kept for callers without project context.
    pub fn installed(&self) -> bool {
        if self.agent_type == AgentType::Shell {
            return true;
        }
        which(&self.program).is_some()
    }

    /// Resolve Pi from PATH; other agents may use project-local bins.
    pub fn executable_in(&self, cwd: &Path) -> Option<PathBuf> {
        if self.agent_type == AgentType::Shell {
            return Some(PathBuf::from(&self.program));
        }
        if self.agent_type == AgentType::Pi {
            return which(&self.program);
        }
        project_binary(cwd, &self.program).or_else(|| which(&self.program))
    }

    /// Check installation using the same resolution rules as launching.
    pub fn installed_in(&self, cwd: &Path) -> bool {
        self.executable_in(cwd).is_some()
    }

    /// Which structured native chat driver this preset supports (Pi RPC or generic ACP).
    pub fn chat_driver_kind(&self) -> Option<crate::chat::ChatDriverKind> {
        match self.agent_type {
            AgentType::Pi | AgentType::Pie => Some(crate::chat::ChatDriverKind::PiRpc),
            AgentType::Opencode
            | AgentType::Gemini
            | AgentType::Qwen
            | AgentType::Kimi
            | AgentType::Agy => Some(crate::chat::ChatDriverKind::Acp),
            AgentType::Claude | AgentType::Codex | AgentType::Shell | AgentType::Unknown => None,
        }
    }

    /// Build CLI arguments for starting this agent in structured chat mode.
    pub fn chat_launch_args(&self, session_id_or_file: Option<&str>) -> Option<Vec<String>> {
        let mut args = self.args.clone();
        match self.chat_driver_kind()? {
            crate::chat::ChatDriverKind::PiRpc => {
                if !args.iter().any(|a| a == "--mode") {
                    args.push("--mode".into());
                    args.push("rpc".into());
                }
                if let Some(session) = session_id_or_file.filter(|s| !s.is_empty()) {
                    args.push("--session".into());
                    args.push(session.to_string());
                }
            }
            crate::chat::ChatDriverKind::Acp => match self.agent_type {
                AgentType::Opencode => {
                    if !args.iter().any(|a| a == "acp") {
                        args.insert(0, "acp".into());
                    }
                }
                _ => {
                    if !args
                        .iter()
                        .any(|a| a == "--acp" || a == "--experimental-acp")
                    {
                        args.push("--acp".into());
                    }
                }
            },
        }
        Some(args)
    }

    /// Build interactive TUI CLI arguments to resume an existing provider session in terminal mode.
    pub fn terminal_resume_args(&self, session_id_or_file: &str) -> Vec<String> {
        let mut args = self.args.clone();
        match self.agent_type {
            AgentType::Pi | AgentType::Pie => {
                args.push("--session".into());
                args.push(session_id_or_file.to_string());
            }
            AgentType::Opencode => {
                args.push("-s".into());
                args.push(session_id_or_file.to_string());
            }
            AgentType::Gemini | AgentType::Qwen => {
                args.push("-r".into());
                args.push(session_id_or_file.to_string());
            }
            AgentType::Kimi | AgentType::Agy => {
                args.push("--resume".into());
                args.push(session_id_or_file.to_string());
            }
            _ => {}
        }
        args
    }
}

pub fn builtin_presets(shell: impl Into<String>) -> Vec<AgentPreset> {
    let empty = || BTreeMap::new();
    vec![
        AgentPreset {
            id: "shell".into(),
            label: "Shell".into(),
            agent_type: AgentType::Shell,
            program: shell.into(),
            args: vec![],
            env: empty(),
            require_installed: false,
        },
        AgentPreset {
            id: "claude".into(),
            label: "Claude Code".into(),
            agent_type: AgentType::Claude,
            program: "claude".into(),
            args: vec![],
            env: empty(),
            require_installed: true,
        },
        AgentPreset {
            id: "codex".into(),
            label: "Codex".into(),
            agent_type: AgentType::Codex,
            program: "codex".into(),
            args: vec![],
            env: empty(),
            require_installed: true,
        },
        AgentPreset {
            id: "opencode".into(),
            label: "OpenCode".into(),
            agent_type: AgentType::Opencode,
            program: "opencode".into(),
            args: vec![],
            env: empty(),
            require_installed: true,
        },
        AgentPreset {
            id: "pi".into(),
            label: "Pi".into(),
            agent_type: AgentType::Pi,
            program: "pi".into(),
            args: vec![],
            env: empty(),
            require_installed: true,
        },
        AgentPreset {
            id: "pie".into(),
            label: "Pie".into(),
            agent_type: AgentType::Pie,
            program: "pie".into(),
            args: vec![],
            env: empty(),
            require_installed: true,
        },
        AgentPreset {
            id: "agy".into(),
            label: "Agy".into(),
            agent_type: AgentType::Agy,
            program: "agy".into(),
            args: vec![],
            env: empty(),
            require_installed: true,
        },
        AgentPreset {
            id: "qwen".into(),
            label: "Qwen Code".into(),
            agent_type: AgentType::Qwen,
            program: "qwen".into(),
            args: vec![],
            env: empty(),
            require_installed: true,
        },
        AgentPreset {
            id: "kimi".into(),
            label: "Kimi CLI".into(),
            agent_type: AgentType::Kimi,
            program: "kimi".into(),
            args: vec![],
            env: empty(),
            require_installed: true,
        },
    ]
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn project_binary(cwd: &Path, program: &str) -> Option<PathBuf> {
    let program = Path::new(program);
    if program.components().count() > 1 {
        let candidate = if program.is_absolute() {
            program.to_path_buf()
        } else {
            cwd.join(program)
        };
        return is_executable(&candidate).then_some(candidate);
    }

    ["node_modules/.bin", ".venv/bin", ".local/bin"]
        .into_iter()
        .map(|dir| cwd.join(dir).join(program))
        .find(|candidate| is_executable(candidate))
}

fn which(program: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(program);
    if p.components().count() > 1 && p.is_file() {
        return is_executable(p).then_some(p.to_path_buf());
    }
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(program))
            .find(|candidate| is_executable(candidate))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtins_include_core_agents() {
        let ps = builtin_presets("/bin/zsh");
        assert_eq!(ps[0].program, "/bin/zsh");
        for id in [
            "shell", "claude", "codex", "opencode", "pi", "pie", "agy", "qwen", "kimi",
        ] {
            assert!(ps.iter().any(|p| p.id == id));
        }
        assert!(!ps.iter().any(|p| p.id == "gemini"));
        assert_eq!(
            ps.iter()
                .filter(|p| p.agent_type != AgentType::Shell)
                .count(),
            8
        );
    }
    #[test]
    fn shell_always_visible() {
        assert!(builtin_presets("/not/actually/installed")[0].installed());
    }

    #[test]
    #[cfg(unix)]
    fn pi_uses_path_and_ignores_project_local_bins() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let global = which("sh").unwrap();
        let local_only = format!(
            "muxlane-local-pi-{}",
            temp.path().file_name().unwrap().to_string_lossy()
        );
        let mut preset = builtin_presets("/bin/sh")
            .into_iter()
            .find(|p| p.id == "pi")
            .unwrap();
        for dir in ["node_modules/.bin", ".venv/bin", ".local/bin"] {
            let bin_dir = temp.path().join(dir);
            std::fs::create_dir_all(&bin_dir).unwrap();
            for program in ["sh", local_only.as_str()] {
                let executable = bin_dir.join(program);
                std::fs::write(&executable, "").unwrap();
                std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))
                    .unwrap();
            }
        }

        preset.program = "sh".into();
        assert_eq!(preset.executable_in(temp.path()), Some(global));
        assert!(preset.installed_in(temp.path()));
        preset.program = local_only;
        assert_eq!(preset.executable_in(temp.path()), None);
        assert!(!preset.installed_in(temp.path()));
    }

    #[test]
    fn project_local_bins_are_installed() {
        let temp = tempfile::tempdir().unwrap();
        let preset = builtin_presets("/bin/sh")
            .into_iter()
            .find(|p| p.id == "qwen")
            .unwrap();

        for dir in ["node_modules/.bin", ".venv/bin", ".local/bin"] {
            let project = temp.path().join(dir.replace('/', "_"));
            let bin_dir = project.join(dir);
            std::fs::create_dir_all(&bin_dir).unwrap();
            std::fs::write(bin_dir.join("qwen"), "").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    bin_dir.join("qwen"),
                    std::fs::Permissions::from_mode(0o755),
                )
                .unwrap();
            }
            assert!(preset.installed_in(&project), "local bin under {dir}");
        }
    }
}
