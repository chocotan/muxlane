//! 服务端状态：快照 + agent 会话表 + hook 上报处理
use muxlane_core::detect::{DetectionEngine, HookEvent};
use muxlane_core::model::{
    AgentId, AgentInstance, AgentStatus, AgentType, MachineInfo, Project, Snapshot,
};
use muxlane_core::protocol::{AgentReportParams, EventMsg};

struct ShellIdentity {
    title: String,
    last_osc: Option<String>,
}

pub struct ServerState {
    pub machine: MachineInfo,
    pub projects: Vec<Project>,
    pub agents: Vec<AgentInstance>,
    /// 状态检测
    pub detector: DetectionEngine,
    shell_identities: std::collections::HashMap<AgentId, ShellIdentity>,
    /// 全局状态事件广播（app 内部路径与 wire 协议共用）
    pub events: tokio::sync::broadcast::Sender<muxlane_core::protocol::EventMsg>,
}

impl ServerState {
    pub fn new(machine: MachineInfo) -> Self {
        let (events, _) = tokio::sync::broadcast::channel(256);
        let mut detector = DetectionEngine::new();
        for manifest in muxlane_core::detect::builtin_manifests() {
            detector.register_manifest(manifest);
        }
        ServerState {
            machine,
            projects: vec![],
            agents: vec![],
            detector,
            shell_identities: Default::default(),
            events,
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            machine: Some(self.machine.clone()),
            projects: self.projects.clone(),
            agents: self.agents.clone(),
        }
    }

    pub fn add_project(&mut self, mut project: Project) -> bool {
        if self
            .projects
            .iter()
            .any(|existing| existing.id == project.id || existing.path == project.path)
        {
            return false;
        }
        project.agents.clear();
        self.projects.push(project);
        true
    }

    /// 注册一个新 agent（启动 PTY 后调用）
    pub fn add_agent(&mut self, project: Project, mut instance: AgentInstance) {
        instance.project = project.id.clone();
        if !self.projects.iter().any(|p| p.id == project.id) {
            self.projects.push(project.clone());
        }
        let proj = self
            .projects
            .iter_mut()
            .find(|p| p.id == project.id)
            .unwrap();
        if !proj.agents.contains(&instance.id) {
            proj.agents.push(instance.id.clone());
            proj.branch = project.branch.clone();
        }
        instance.seen = true;
        instance.status = AgentStatus::Idle;
        instance.status_since = muxlane_core::model::now_secs();
        self.agents.push(instance);
    }

    pub fn remove_agent(&mut self, agent: &AgentId) {
        if let Some(inst) = self.agents.iter().find(|a| &a.id == agent) {
            let pid = inst.project.clone();
            if let Some(p) = self.projects.iter_mut().find(|p| p.id == pid) {
                p.agents.retain(|a| a != agent);
            }
        }
        self.agents.retain(|a| &a.id != agent);
        self.detector.forget(agent);
        self.shell_identities.remove(agent);
        // 项目空了也保留（历史项目还在）；v1 不做清理
    }

    pub fn remove_project(&mut self, project: &muxlane_core::model::ProjectId) -> Vec<AgentId> {
        let agents: Vec<AgentId> = self
            .agents
            .iter()
            .filter(|agent| &agent.project == project)
            .map(|agent| agent.id.clone())
            .collect();
        for agent in &agents {
            self.remove_agent(agent);
        }
        self.projects.retain(|item| &item.id != project);
        agents
    }

    /// Shell presets, plus shells currently running an Agent in the foreground.
    pub(crate) fn is_shell_origin(&self, agent: &AgentId) -> bool {
        self.shell_identities.contains_key(agent)
            || self
                .agents
                .iter()
                .any(|instance| &instance.id == agent && instance.agent_type == AgentType::Shell)
    }

    /// Only a verified foreground process sample may change a shell's identity.
    /// Identity changes are state updates, not task completion notifications.
    pub(crate) fn observe_foreground(
        &mut self,
        agent: &AgentId,
        foreground: &crate::foreground::ShellForeground,
        osc_title: Option<&str>,
    ) -> bool {
        let Some(instance) = self
            .agents
            .iter_mut()
            .find(|instance| &instance.id == agent)
        else {
            return false;
        };
        let identity = self
            .shell_identities
            .entry(agent.clone())
            .or_insert_with(|| ShellIdentity {
                title: if instance.agent_type == AgentType::Shell {
                    instance.title.clone()
                } else {
                    foreground.shell.clone()
                },
                last_osc: osc_title.map(str::to_owned),
            });
        if instance.agent_type == foreground.agent_type {
            return false;
        }
        instance.agent_type = foreground.agent_type;
        instance.title = if foreground.agent_type == AgentType::Shell {
            identity.title.clone()
        } else {
            muxlane_core::builtin_presets("")
                .into_iter()
                .find(|preset| preset.agent_type == foreground.agent_type)
                .map(|preset| preset.label)
                .unwrap_or_else(|| foreground.agent_type.as_str().into())
        };
        // Replay still contains the previous application's OSC and prompt. Do not
        // immediately overwrite the new title with that old output.
        identity.last_osc = osc_title.map(str::to_owned);
        instance.status = AgentStatus::Idle;
        instance.seen = true;
        instance.status_since = muxlane_core::model::now_secs();
        self.detector.forget(agent);
        true
    }

    /// 屏幕检测兜底（hook 权威窗口内 DetectionEngine 会自动忽略屏幕结果）。
    pub fn observe_screen(
        &mut self,
        agent: &AgentId,
        input: &muxlane_core::detect::ScreenInput,
    ) -> Vec<EventMsg> {
        let fresh_title = if let Some(identity) = self.shell_identities.get_mut(agent) {
            let fresh = identity.last_osc.as_ref() != input.osc_title.as_ref();
            identity.last_osc = input.osc_title.clone();
            fresh
        } else {
            true
        };
        let title_changed = fresh_title
            && input.osc_title.as_deref().is_some_and(|raw| {
                let title: String = raw
                    .trim()
                    .chars()
                    .filter(|ch| !ch.is_control())
                    .take(80)
                    .collect();
                if title.is_empty() || looks_like_tmux_copy_title(&title) {
                    return false;
                }
                let Some(instance) = self.agents.iter_mut().find(|item| &item.id == agent) else {
                    return false;
                };
                if instance.title == title {
                    false
                } else {
                    instance.title = title;
                    true
                }
            });
        let Some((current, agent_type)) = self
            .agents
            .iter()
            .find(|a| &a.id == agent)
            .map(|a| (a.status, a.agent_type))
        else {
            return vec![];
        };
        let mut events =
            if let Some(update) = self.detector.observe(agent, agent_type, current, input) {
                self.apply_status(agent, update.to, None)
            } else {
                vec![]
            };
        if title_changed && events.is_empty() {
            events.push(EventMsg::new(
                muxlane_core::protocol::events::STATE_CHANGED,
                serde_json::json!({}),
            ));
        }
        events
    }

    /// 查看 Done agent：seen=true，Done→Idle。
    pub fn mark_seen(&mut self, agent: &AgentId) -> Vec<EventMsg> {
        let Some(inst) = self.agents.iter_mut().find(|a| &a.id == agent) else {
            return vec![];
        };
        inst.seen = true;
        if inst.status.is_finished() {
            return self.apply_status(agent, AgentStatus::Idle, None);
        }
        vec![]
    }

    /// hook 上报：应用状态并产出事件（agent.status_changed）
    pub async fn report_hook(&mut self, params: &AgentReportParams) -> Vec<EventMsg> {
        let Some(ev) = HookEvent::parse(&params.event) else {
            return vec![];
        };
        let Some(inst) = self.agents.iter().find(|a| a.id == params.agent) else {
            return vec![];
        };
        // Late/background hook messages must not put a returned shell back into
        // the old Agent's Working/Done state. Foreground polling owns identity.
        if self.shell_identities.contains_key(&params.agent) && inst.agent_type == AgentType::Shell
        {
            return vec![];
        }
        let current = inst.status;
        let target = ev.to_status();
        if let Some(upd) = self.detector.report(&params.agent, current, ev) {
            self.apply_status(&params.agent, upd.to, params.message.clone())
        } else if current == target
            && params
                .message
                .as_deref()
                .is_some_and(|message| !message.trim().is_empty())
        {
            let msg = EventMsg::new(
                muxlane_core::protocol::events::AGENT_STATUS,
                serde_json::to_value(muxlane_core::protocol::AgentStatusEvent {
                    agent: params.agent.clone(),
                    agent_type: Some(inst.agent_type),
                    from: current,
                    to: current,
                    message: params.message.clone(),
                })
                .unwrap_or_default(),
            );
            let _ = self.events.send(msg.clone());
            vec![msg]
        } else {
            vec![]
        }
    }

    /// 按键触发的 working 标记：与屏幕采样同走 DetectionEngine，
    /// 保证引擎内部状态与服务器状态一致（否则屏幕推导的 idle 候选
    /// 会因等于引擎陈旧内部状态而永不提交，状态卡死）。
    pub fn mark_screen_working(&mut self, agent: &AgentId) -> Vec<EventMsg> {
        let Some(current) = self
            .agents
            .iter()
            .find(|a| &a.id == agent)
            .map(|a| a.status)
        else {
            return vec![];
        };
        if let Some(update) = self.detector.mark_working(agent, current) {
            self.apply_status(agent, update.to, None)
        } else {
            vec![]
        }
    }

    fn apply_status(
        &mut self,
        agent: &AgentId,
        to: AgentStatus,
        message: Option<String>,
    ) -> Vec<EventMsg> {
        let Some(inst) = self.agents.iter_mut().find(|a| a.id == *agent) else {
            return vec![];
        };
        let from = inst.status;
        if from == to {
            return vec![];
        }
        inst.status = to;
        inst.status_since = muxlane_core::model::now_secs();
        if to.is_finished() {
            inst.seen = false;
        }
        let msg = EventMsg::new(
            muxlane_core::protocol::events::AGENT_STATUS,
            serde_json::to_value(muxlane_core::protocol::AgentStatusEvent {
                agent: agent.clone(),
                agent_type: Some(inst.agent_type),
                from,
                to,
                message,
            })
            .unwrap_or_default(),
        );
        let _ = self.events.send(msg.clone()); // 广播给 wire 订阅者
        vec![msg]
    }

    /// agent 会话退出 → 移除并返回 TermExit 事件
    pub fn agent_exit(&mut self, agent: &AgentId) -> Vec<EventMsg> {
        self.remove_agent(agent);
        vec![EventMsg::new(
            muxlane_core::protocol::events::TERM_EXIT,
            serde_json::to_value(muxlane_core::protocol::TermExitEvent {
                agent: agent.clone(),
            })
            .unwrap_or_default(),
        )]
    }
}

fn looks_like_tmux_copy_title(title: &str) -> bool {
    let t = title.trim();
    t.contains("[tmux]")
        || t.contains("copy-mode")
        || t.contains("Copy mode")
        || (t.starts_with('[') && t.contains("tmux"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn shell_identity_changes_clear_authority_and_do_not_reuse_old_titles() {
        let mut state = ServerState::new(MachineInfo {
            machine_id: "test".into(),
            name: "test".into(),
            os: "test".into(),
            version: "test".into(),
        });
        let id = "shell_test".to_string();
        state.add_agent(
            Project {
                id: "p".into(),
                name: "p".into(),
                path: "/tmp".into(),
                branch: None,
                agents: vec![],
            },
            AgentInstance {
                id: id.clone(),
                project: "p".into(),
                agent_type: AgentType::Shell,
                title: "Shell".into(),
                status: AgentStatus::Idle,
                status_since: 0,
                seen: true,
                tmux_session: None,
            },
        );
        let mut foreground = crate::foreground::ShellForeground {
            shell: "zsh".into(),
            agent_type: AgentType::Pi,
        };
        assert!(state.observe_foreground(&id, &foreground, Some("old shell title")));
        assert_eq!(state.agents[0].title, "Pi");
        let report = AgentReportParams {
            token: "unused".into(),
            agent: id.clone(),
            event: "working".into(),
            message: None,
        };
        let events = state.report_hook(&report).await;
        assert_eq!(events[0].params["agent_type"], "pi");
        assert!(state.detector.has_hook_authority(&id));
        let screen = muxlane_core::detect::ScreenInput {
            osc_title: Some("Agent task title".into()),
            ..Default::default()
        };
        state.observe_screen(&id, &screen);
        assert_eq!(state.agents[0].title, "Agent task title");
        // A completed turn is not a process exit: identity remains Pi.
        state
            .report_hook(&AgentReportParams {
                event: "done".into(),
                ..report.clone()
            })
            .await;
        assert_eq!(state.agents[0].agent_type, AgentType::Pi);
        foreground.agent_type = AgentType::Shell;
        assert!(state.observe_foreground(&id, &foreground, screen.osc_title.as_deref()));
        assert_eq!(state.agents[0].title, "Shell");
        assert_eq!(state.agents[0].status, AgentStatus::Idle);
        assert!(state.agents[0].seen);
        assert!(!state.detector.has_hook_authority(&id));
        state.observe_screen(&id, &screen);
        assert_eq!(
            state.agents[0].title, "Shell",
            "old OSC in replay must not restore Agent title"
        );
        assert!(
            state.report_hook(&report).await.is_empty(),
            "late hooks must not reactivate an exited Agent"
        );
        foreground.agent_type = AgentType::Codex;
        assert!(state.observe_foreground(&id, &foreground, screen.osc_title.as_deref()));
        assert_eq!(state.agents[0].title, "Codex");
        assert_eq!(
            state.report_hook(&report).await[0].params["agent_type"],
            "codex"
        );
    }

    #[test]
    fn removing_project_keeps_unrelated_agents() {
        let mut st = ServerState::new(MachineInfo {
            machine_id: "m".into(),
            name: "box".into(),
            os: "linux".into(),
            version: "0.1".into(),
        });
        for (project_id, agent_id) in [("p1", "a1"), ("p2", "a2")] {
            st.add_agent(
                Project {
                    id: project_id.into(),
                    name: project_id.into(),
                    path: format!("/tmp/{project_id}").into(),
                    branch: None,
                    agents: vec![],
                },
                AgentInstance {
                    id: agent_id.into(),
                    project: project_id.into(),
                    agent_type: muxlane_core::model::AgentType::Shell,
                    title: agent_id.into(),
                    status: AgentStatus::Idle,
                    status_since: 0,
                    seen: true,
                    tmux_session: None,
                },
            );
        }
        assert_eq!(st.remove_project(&"p1".into()), vec!["a1"]);
        assert_eq!(st.projects.len(), 1);
        assert_eq!(st.agents[0].id, "a2");
    }

    #[tokio::test]
    async fn same_status_hook_enriches_message_and_osc_updates_title() {
        let mut st = ServerState::new(MachineInfo {
            machine_id: "m".into(),
            name: "box".into(),
            os: "linux".into(),
            version: "0.1".into(),
        });
        let agent = "pi_test".to_string();
        st.add_agent(
            Project {
                id: "p".into(),
                name: "repo".into(),
                path: "/tmp".into(),
                branch: None,
                agents: vec![],
            },
            AgentInstance {
                id: agent.clone(),
                project: "p".into(),
                agent_type: muxlane_core::model::AgentType::Pi,
                title: "Pi".into(),
                status: AgentStatus::Idle,
                status_since: 0,
                seen: true,
                tmux_session: None,
            },
        );
        st.apply_status(&agent, AgentStatus::Done, None);
        let events = st
            .report_hook(&AgentReportParams {
                token: "unused".into(),
                agent: agent.clone(),
                event: "done".into(),
                message: Some("完成了具体修复".into()),
            })
            .await;
        let payload: muxlane_core::protocol::AgentStatusEvent =
            serde_json::from_value(events[0].params.clone()).unwrap();
        assert_eq!(payload.from, AgentStatus::Done);
        assert_eq!(payload.to, AgentStatus::Done);
        assert_eq!(payload.message.as_deref(), Some("完成了具体修复"));

        let title_events = st.observe_screen(
            &agent,
            &muxlane_core::detect::ScreenInput {
                osc_title: Some("π - muxlane repo".into()),
                ..Default::default()
            },
        );
        assert!(!title_events.is_empty());
        assert_eq!(st.agents[0].title, "π - muxlane repo");

        let ignored = st.observe_screen(
            &"a1".into(),
            &muxlane_core::detect::ScreenInput {
                osc_title: Some("[tmux] 0:zsh  Copy mode".into()),
                ..Default::default()
            },
        );
        assert!(ignored.is_empty());
        assert_eq!(st.agents[0].title, "π - muxlane repo");
    }

    #[test]
    fn add_project_keeps_empty_project_and_rejects_duplicate_path() {
        let mut st = ServerState::new(MachineInfo {
            machine_id: "m".into(),
            name: "box".into(),
            os: "linux".into(),
            version: "0.1".into(),
        });
        let project = Project {
            id: "p1".into(),
            name: "repo".into(),
            path: "/tmp/repo".into(),
            branch: None,
            agents: vec!["stale".into()],
        };
        assert!(st.add_project(project));
        assert!(st.projects[0].agents.is_empty());
        assert!(!st.add_project(Project {
            id: "p2".into(),
            name: "same-path".into(),
            path: "/tmp/repo".into(),
            branch: None,
            agents: vec![],
        }));
        assert_eq!(st.projects.len(), 1);
    }

    #[test]
    fn screen_detection_is_registered_and_done_seen_returns_idle() {
        let mut st = ServerState::new(MachineInfo {
            machine_id: "m".into(),
            name: "box".into(),
            os: "linux".into(),
            version: "0.1".into(),
        });
        let agent = "claude_test".to_string();
        st.projects.push(Project {
            id: "p".into(),
            name: "p".into(),
            path: "/tmp".into(),
            branch: None,
            agents: vec![agent.clone()],
        });
        st.agents.push(AgentInstance {
            id: agent.clone(),
            project: "p".into(),
            agent_type: muxlane_core::model::AgentType::Claude,
            title: "Claude".into(),
            status: AgentStatus::Working,
            status_since: 0,
            seen: true,
            tmux_session: None,
        });
        let input = muxlane_core::detect::ScreenInput {
            bottom_lines: vec!["Do you want to proceed?".into(), "❯ 1. Yes".into()],
            ..Default::default()
        };
        assert!(st.observe_screen(&agent, &input).is_empty()); // debounce 1
        assert!(!st.observe_screen(&agent, &input).is_empty());
        assert_eq!(st.agents[0].status, AgentStatus::Blocked);
        st.apply_status(&agent, AgentStatus::Done, None);
        assert!(!st.mark_seen(&agent).is_empty());
        assert!(st.agents[0].seen);
        assert_eq!(st.agents[0].status, AgentStatus::Idle);

        st.apply_status(&agent, AgentStatus::Failed, None);
        assert!(!st.mark_seen(&agent).is_empty());
        assert!(st.agents[0].seen);
        assert_eq!(st.agents[0].status, AgentStatus::Idle);
    }
}
