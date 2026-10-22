//! muxlane-core: 数据模型
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub type MachineId = String;
pub type ProjectId = String;
pub type AgentId = String;

pub fn new_id(prefix: &str) -> String {
    format!("{}_{}", prefix, ulid::Ulid::new())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MachineInfo {
    pub machine_id: MachineId,
    pub name: String,
    pub os: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    #[serde(default)]
    pub agents: Vec<AgentId>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    Claude,
    Codex,
    Opencode,
    Pi,
    /// Pi-compatible CLI with its own `~/.pie` config directory.
    Pie,
    /// Kept for deserializing older state; never offered or auto-detected.
    Gemini,
    Agy,
    Qwen,
    Kimi,
    Shell,
    #[serde(other)]
    Unknown,
}

impl AgentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentType::Claude => "claude",
            AgentType::Codex => "codex",
            AgentType::Opencode => "opencode",
            AgentType::Pi => "pi",
            AgentType::Pie => "pie",
            AgentType::Gemini => "gemini",
            AgentType::Agy => "agy",
            AgentType::Qwen => "qwen",
            AgentType::Kimi => "kimi",
            AgentType::Shell => "shell",
            AgentType::Unknown => "unknown",
        }
    }
    pub fn program(&self) -> &'static str {
        match self {
            AgentType::Claude => "claude",
            AgentType::Codex => "codex",
            AgentType::Opencode => "opencode",
            AgentType::Pi => "pi",
            AgentType::Pie => "pie",
            AgentType::Gemini => "gemini",
            AgentType::Agy => "agy",
            AgentType::Qwen => "qwen",
            AgentType::Kimi => "kimi",
            AgentType::Shell => "bash",
            AgentType::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Working,
    Blocked,
    Idle,
    Done,
    Failed,
    #[serde(other)]
    Unknown,
}

impl AgentStatus {
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            AgentStatus::Working => "working",
            AgentStatus::Blocked => "blocked",
            AgentStatus::Idle => "idle",
            AgentStatus::Done => "done",
            AgentStatus::Failed => "failed",
            AgentStatus::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentInstance {
    pub id: AgentId,
    pub project: ProjectId,
    pub agent_type: AgentType,
    pub title: String,
    pub status: AgentStatus,
    /// 状态起始时间（unix 秒）
    pub status_since: u64,
    /// done 是否已被用户查看；查看后 done -> idle
    #[serde(default = "default_true")]
    pub seen: bool,
    /// 所在 tmux 会话名（若经 tmux 启动）
    #[serde(default)]
    pub tmux_session: Option<String>,
}

fn default_true() -> bool {
    true
}

/// 服务端全量状态快照（state.list 的返回）
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Snapshot {
    #[serde(default)]
    pub machine: Option<MachineInfo>,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub agents: Vec<AgentInstance>,
}

impl Snapshot {
    pub fn agent(&self, id: &AgentId) -> Option<&AgentInstance> {
        self.agents.iter().find(|a| &a.id == id)
    }
    pub fn agent_mut(&mut self, id: &AgentId) -> Option<&mut AgentInstance> {
        self.agents.iter_mut().find(|a| &a.id == id)
    }
    pub fn project(&self, id: &ProjectId) -> Option<&Project> {
        self.projects.iter().find(|p| &p.id == id)
    }
    pub fn agents_of(&self, project: &ProjectId) -> Vec<&AgentInstance> {
        self.agents
            .iter()
            .filter(|a| &a.project == project)
            .collect()
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_roundtrip() {
        let snap = Snapshot {
            machine: Some(MachineInfo {
                machine_id: "m_1".into(),
                name: "local".into(),
                os: "linux".into(),
                version: "0.1.0".into(),
            }),
            projects: vec![Project {
                id: "p_1".into(),
                name: "zbase".into(),
                path: "/tmp/zbase".into(),
                branch: Some("main".into()),
                agents: vec!["a_1".into()],
            }],
            agents: vec![AgentInstance {
                id: "a_1".into(),
                project: "p_1".into(),
                agent_type: AgentType::Claude,
                title: "refactor".into(),
                status: AgentStatus::Working,
                status_since: 1700000000,
                seen: true,
                tmux_session: None,
            }],
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snap);
    }

    #[test]
    fn unknown_protocol_enums_are_tolerated() {
        assert_eq!(
            serde_json::from_str::<AgentType>(r#""future_agent""#).unwrap(),
            AgentType::Unknown
        );
        assert_eq!(
            serde_json::from_str::<AgentStatus>(r#""future_status""#).unwrap(),
            AgentStatus::Unknown
        );
    }

    #[test]
    fn id_prefix() {
        let id = new_id("agent");
        assert!(id.starts_with("agent_"));
    }
}
