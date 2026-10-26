//! 状态检测引擎：hook 权威上报 + manifest 屏幕规则兜底 + 防抖
pub mod manifest;

use crate::model::{AgentId, AgentStatus, AgentType};
use manifest::CompiledManifest;
use std::collections::HashMap;
use std::time::Instant;

/// 一次屏幕采样的输入
#[derive(Debug, Clone, Default)]
pub struct ScreenInput {
    /// 终端屏幕底部可见行（含 prompt 区域），已按行切分
    pub bottom_lines: Vec<String>,
    /// OSC 标题（若有）
    pub osc_title: Option<String>,
    /// 最近一次输出距现在多少秒（输出静默时长）
    pub secs_since_output: Option<f64>,
    /// 最近一个采样周期内是否响铃
    pub bell: bool,
}

/// hook 上报事件（对应 wire 协议 agent.report 的 event 字段）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    Done,
    Failed,
    Blocked,
    Working,
    Idle,
}

impl HookEvent {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "done" => Some(HookEvent::Done),
            "failed" | "error" => Some(HookEvent::Failed),
            "blocked" => Some(HookEvent::Blocked),
            "working" => Some(HookEvent::Working),
            "idle" => Some(HookEvent::Idle),
            _ => None,
        }
    }
    pub fn to_status(self) -> AgentStatus {
        match self {
            HookEvent::Done => AgentStatus::Done,
            HookEvent::Failed => AgentStatus::Failed,
            HookEvent::Blocked => AgentStatus::Blocked,
            HookEvent::Working => AgentStatus::Working,
            HookEvent::Idle => AgentStatus::Idle,
        }
    }
}

#[derive(Debug, Clone)]
struct AgentDetectState {
    status: AgentStatus,
    /// 当前权威源
    authority: Authority,
    /// hook 最近一次上报时刻
    hook_last_seen: Option<Instant>,
    /// 防抖：候选状态与已连续次数
    pending: Option<(AgentStatus, u32)>,
    /// 状态序号，单调递增，乱序/回退丢弃
    seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Authority {
    Screen,
    Hook,
}

/// 引擎产出的状态更新（None = 无变化）
#[derive(Debug, Clone, PartialEq)]
pub struct StatusUpdate {
    pub agent: AgentId,
    pub to: AgentStatus,
    pub seq: u64,
}

/// 按键触发（非 hook、非屏幕规则）的状态更新
pub struct ScreenStatusUpdate {
    pub to: AgentStatus,
}

pub struct DetectionEngine {
    manifests: HashMap<String, CompiledManifest>,
    states: HashMap<AgentId, AgentDetectState>,
    /// 防抖需要连续一致的采样次数
    debounce_ticks: u32,
}

/// hook 权威窗口：hook 上报后该时长内 hook 独占状态判定
const HOOK_AUTHORITY_WINDOW: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// 静默兜底阈值：屏幕规则全部失配且当前被认为是 working 时，
/// 输出静默超过该时长则回落 idle。
/// TUI agent（pi/claude 等）工作时会持续重绘 spinner，不会真正静默；
/// 该兜底专用于修复「误标 working / hook 漏报 done」后的状态卡死。
const SILENCE_IDLE_FALLBACK_SECS: f64 = 10.0;

impl DetectionEngine {
    pub fn new() -> Self {
        DetectionEngine {
            manifests: HashMap::new(),
            states: HashMap::new(),
            debounce_ticks: 2,
        }
    }

    /// 注册/覆盖某 agent 类型的 manifest（内置或 ~/.config/muxlane/manifests）
    pub fn register_manifest(&mut self, m: CompiledManifest) {
        self.manifests.insert(m.agent_type.clone(), m);
    }

    pub fn has_hook_authority(&self, agent: &AgentId) -> bool {
        self.states
            .get(agent)
            .map(|s| matches!(s.authority, Authority::Hook))
            .unwrap_or(false)
    }

    /// 按键触发的 working 标记：立即生效，绕过屏幕防抖，
    /// 但与屏幕采样共享同一份引擎状态（screen authority）。
    /// hook 权威窗口内不覆写 hook 状态。
    pub fn mark_working(
        &mut self,
        agent: &AgentId,
        current: AgentStatus,
    ) -> Option<ScreenStatusUpdate> {
        let st = self.state_mut(agent, current);
        if matches!(st.authority, Authority::Hook) {
            return None;
        }
        st.pending = None;
        st.status = AgentStatus::Working;
        Some(ScreenStatusUpdate {
            to: AgentStatus::Working,
        })
    }

    fn state_mut(&mut self, agent: &AgentId, current: AgentStatus) -> &mut AgentDetectState {
        let st = self
            .states
            .entry(agent.clone())
            .or_insert_with(|| AgentDetectState {
                status: current,
                authority: Authority::Screen,
                hook_last_seen: None,
                pending: None,
                seq: 0,
            });
        if st.status != current {
            st.status = current;
            st.pending = None;
        }
        st
    }

    pub fn forget(&mut self, agent: &AgentId) {
        self.states.remove(agent);
    }

    /// hook 上报入口（服务端线程投递）。权威窗口内独占。
    pub fn report(
        &mut self,
        agent: &AgentId,
        current: AgentStatus,
        ev: HookEvent,
    ) -> Option<StatusUpdate> {
        let st = self.state_mut(agent, current);
        st.hook_last_seen = Some(Instant::now());
        st.authority = Authority::Hook;
        st.seq += 1;
        let seq = st.seq;
        if st.status != ev.to_status() {
            st.status = ev.to_status();
            st.pending = None;
            Some(StatusUpdate {
                agent: agent.clone(),
                to: ev.to_status(),
                seq,
            })
        } else {
            st.pending = None;
            None
        }
    }

    /// 每 tick 屏幕采样。返回状态变化（已经过防抖）。
    pub fn observe(
        &mut self,
        agent: &AgentId,
        agent_type: AgentType,
        current: AgentStatus,
        input: &ScreenInput,
    ) -> Option<StatusUpdate> {
        // 1) hook 权威窗口内不采信屏幕规则
        if let Some(st) = self.states.get(agent) {
            if matches!(st.authority, Authority::Hook) {
                let fresh = st
                    .hook_last_seen
                    .map(|t| t.elapsed() < HOOK_AUTHORITY_WINDOW)
                    .unwrap_or(false);
                if fresh {
                    return None;
                }
                // 窗口过期 → 权威回落到屏幕
                let st = self.state_mut(agent, current);
                st.authority = Authority::Screen;
            }
        }

        // 2) 屏幕规则推导候选状态
        let mut candidate = self.screen_candidate(agent_type, input);
        let need = self.debounce_ticks;

        let st = self.state_mut(agent, current);
        // 2b) 静默兜底：规则全失配 + 当前 stuck 在 working + 输出长时间静默
        // → 候选 idle。只兜 working→idle，不覆盖 blocked（避免掩掉等待确认）。
        if candidate.is_none() && matches!(st.status, AgentStatus::Working) {
            let silent_long = input
                .secs_since_output
                .map(|s| s >= SILENCE_IDLE_FALLBACK_SECS)
                .unwrap_or(false);
            if silent_long {
                candidate = Some(AgentStatus::Idle);
            }
        }
        let Some(candidate) = candidate else {
            st.pending = None;
            return None;
        };
        if candidate == st.status {
            st.pending = None;
            return None;
        }

        // 3) 防抖：连续 debounce_ticks 次一致才提交
        match &mut st.pending {
            Some((pend, n)) if *pend == candidate => {
                *n += 1;
                if *n >= need {
                    st.pending = None;
                    st.status = candidate;
                    st.seq += 1;
                    return Some(StatusUpdate {
                        agent: agent.clone(),
                        to: candidate,
                        seq: st.seq,
                    });
                }
                None
            }
            _ => {
                st.pending = Some((candidate, 1));
                None
            }
        }
    }

    /// 屏幕规则 → AgentStatus
    fn screen_candidate(&self, agent_type: AgentType, input: &ScreenInput) -> Option<AgentStatus> {
        let m = self.manifests.get(agent_type.as_str())?;
        m.evaluate(input)
    }
}

impl Default for DetectionEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// 内置 manifest TOML（编译期嵌入）
pub fn builtin_manifests() -> Vec<CompiledManifest> {
    let sources = [
        ("claude", include_str!("manifests/claude.toml")),
        ("codex", include_str!("manifests/codex.toml")),
        ("opencode", include_str!("manifests/opencode.toml")),
        ("pi", include_str!("manifests/pi.toml")),
        ("pie", include_str!("manifests/pi.toml")),
        ("agy", include_str!("manifests/agy.toml")),
        ("qwen", include_str!("manifests/qwen.toml")),
        ("kimi", include_str!("manifests/kimi.toml")),
        ("shell", include_str!("manifests/shell.toml")),
    ];
    sources
        .into_iter()
        .filter_map(|(name, src)| match CompiledManifest::parse(name, src) {
            Ok(m) => Some(m),
            Err(e) => {
                tracing::warn!(manifest = name, error = %e, "builtin manifest parse failed");
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> DetectionEngine {
        let mut e = DetectionEngine::new();
        for m in builtin_manifests() {
            e.register_manifest(m);
        }
        e
    }

    fn input<'a>(lines: impl IntoIterator<Item = &'a str>) -> ScreenInput {
        ScreenInput {
            bottom_lines: lines.into_iter().map(|s| s.to_string()).collect(),
            osc_title: None,
            secs_since_output: Some(0.1),
            bell: false,
        }
    }

    #[test]
    fn claude_blocked_by_permission_prompt() {
        let mut e = engine();
        let a: AgentId = "opaque_A01".into();
        assert_eq!(
            e.observe(
                &a,
                AgentType::Claude,
                AgentStatus::Working,
                &input(["❯ do it"])
            ),
            None
        ); // 第一次进 pending
        let lines = [" Do you want to proceed?", "❯ 1. Yes", "  2. No"];
        assert_eq!(
            e.observe(&a, AgentType::Claude, AgentStatus::Working, &input(lines)),
            None
        );
        let upd = e
            .observe(&a, AgentType::Claude, AgentStatus::Working, &input(lines))
            .unwrap();
        assert_eq!(upd.to, AgentStatus::Blocked);
    }

    #[test]
    fn debounce_rejects_flicker() {
        let mut e = engine();
        let a: AgentId = "claude_A01".into();
        // 单次出现 blocked 特征然后消失 → 不提交
        let lines = [" Do you want to proceed?"];
        e.observe(&a, AgentType::Claude, AgentStatus::Working, &input(lines));
        e.observe(
            &a,
            AgentType::Claude,
            AgentStatus::Working,
            &input(["working hard..."]),
        );
        let r = e.observe(
            &a,
            AgentType::Claude,
            AgentStatus::Working,
            &input(["working hard..."]),
        );
        assert_eq!(r, None);
        assert!(!e.has_hook_authority(&a));
    }

    #[test]
    fn failed_hook_event_is_explicit() {
        assert_eq!(HookEvent::parse("failed"), Some(HookEvent::Failed));
        assert_eq!(HookEvent::parse("error"), Some(HookEvent::Failed));
        assert_eq!(HookEvent::Failed.to_status(), AgentStatus::Failed);
    }

    #[test]
    fn hook_report_is_authoritative_and_immediate() {
        let mut e = engine();
        let a: AgentId = "codex_B02".into();
        let upd = e.report(&a, AgentStatus::Working, HookEvent::Done).unwrap();
        assert_eq!(upd.to, AgentStatus::Done);
        // hook 窗口内屏幕规则不生效
        let r = e.observe(
            &a,
            AgentType::Claude,
            AgentStatus::Done,
            &input(["some output"]),
        );
        assert_eq!(r, None);
    }

    #[test]
    fn hook_report_same_status_no_event() {
        let mut e = engine();
        let a: AgentId = "pi_C03".into();
        assert_eq!(e.report(&a, AgentStatus::Done, HookEvent::Done), None);
    }

    #[test]
    fn keypress_working_does_not_block_screen_idle() {
        // 回归：键入命令 → mark_working → 屏幕 idle 规则必须能正常提交回
        // idle，否则 spinner 永久卡死（引擎内部状态与服务器状态脱节）。
        let mut e = engine();
        let a: AgentId = "shell_S01".into();
        let prompt = input(["~/proj ❯"]);
        assert_eq!(
            e.observe(&a, AgentType::Shell, AgentStatus::Idle, &prompt),
            None
        );
        // 用户按 Enter，按键触发 working（无防抖，立即生效）
        let upd = e.mark_working(&a, AgentStatus::Idle).unwrap();
        assert_eq!(upd.to, AgentStatus::Working);
        // 命令执行完成，屏幕回到 prompt：防抖两拍后应提交 idle
        assert_eq!(
            e.observe(&a, AgentType::Shell, AgentStatus::Working, &prompt),
            None
        );
        let upd = e
            .observe(&a, AgentType::Shell, AgentStatus::Working, &prompt)
            .unwrap();
        assert_eq!(upd.to, AgentStatus::Idle);
    }

    #[test]
    fn keypress_working_respects_hook_authority() {
        // hook 窗口内按键不覆写 hook 状态，防止屏幕侧抢夺权威
        let mut e = engine();
        let a: AgentId = "claude_A01".into();
        e.report(&a, AgentStatus::Working, HookEvent::Working);
        assert!(e.mark_working(&a, AgentStatus::Working).is_none());
    }

    #[test]
    fn builtin_manifest_types_match_supported_agents() {
        let types = builtin_manifests()
            .into_iter()
            .map(|m| m.agent_type)
            .collect::<std::collections::BTreeSet<_>>();
        let expected = [
            "agy", "claude", "codex", "kimi", "opencode", "pi", "pie", "qwen", "shell",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        assert_eq!(types, expected);
        assert!(!types.contains("gemini"));
    }

    #[test]
    fn new_agent_manifests_recognize_screen_states() {
        for (agent_type, status, mut screen) in [
            (
                "agy",
                AgentStatus::Blocked,
                input(["Agy requires approval"]),
            ),
            (
                "qwen",
                AgentStatus::Working,
                input(std::iter::empty::<&str>()),
            ),
            ("kimi", AgentStatus::Idle, input(["(kimi) >"])),
        ] {
            if agent_type == "qwen" {
                screen.osc_title = Some("Qwen Code: thinking".into());
            }
            let manifest = builtin_manifests()
                .into_iter()
                .find(|m| m.agent_type == agent_type)
                .unwrap();
            assert_eq!(manifest.evaluate(&screen), Some(status));
        }
    }

    #[test]
    fn unknown_type_has_no_screen_fallback() {
        let mut e = engine();
        let a: AgentId = "claude_D04".into();
        let prompt = input(["$ "]);
        assert_eq!(
            e.observe(&a, AgentType::Unknown, AgentStatus::Working, &prompt),
            None
        );
        assert_eq!(
            e.observe(&a, AgentType::Unknown, AgentStatus::Working, &prompt),
            None
        );
    }

    /// 当前 pi 空闲底栏：不匹配任何现有屏幕规则
    fn pi_idle_screen(secs: f64) -> ScreenInput {
        ScreenInput {
            bottom_lines: vec![
                "────────────────────────".into(),
                "~/Downloads/some-proj".into(),
                "↑216k ↓23k R2.4M CH96.3% 13.2%/600k (auto)".into(),
            ],
            osc_title: None,
            secs_since_output: Some(secs),
            bell: false,
        }
    }

    #[test]
    fn stuck_working_recovers_via_silence() {
        // 回归：空回车误标 working，pi 屏幕规则全失配，hook 无事件；
        // 输出静默超过阈值后，防抖两拍应回落 idle（修复状态永久卡死）。
        let mut e = engine();
        let a: AgentId = "pi_P01".into();
        assert_eq!(
            e.observe(
                &a,
                AgentType::Pi,
                AgentStatus::Working,
                &pi_idle_screen(30.0)
            ),
            None
        ); // 防抖第一拍
        let upd = e
            .observe(
                &a,
                AgentType::Pi,
                AgentStatus::Working,
                &pi_idle_screen(31.0),
            )
            .unwrap();
        assert_eq!(upd.to, AgentStatus::Idle);
    }

    #[test]
    fn silence_fallback_requires_real_silence() {
        // 静默时长不足阈值（spinner 仍在重绘）→ 不触发回落
        let mut e = engine();
        let a: AgentId = "pi_P02".into();
        for secs in [0.5, 1.0, 2.0] {
            assert_eq!(
                e.observe(
                    &a,
                    AgentType::Pi,
                    AgentStatus::Working,
                    &pi_idle_screen(secs)
                ),
                None
            );
        }
    }

    #[test]
    fn silence_fallback_needs_output_timestamp() {
        // 拿不到输出时间戳（None）→ 保守不回落
        let mut e = engine();
        let a: AgentId = "pi_P03".into();
        let mut screen = pi_idle_screen(0.0);
        screen.secs_since_output = None;
        for _ in 0..3 {
            assert_eq!(
                e.observe(&a, AgentType::Pi, AgentStatus::Working, &screen),
                None
            );
        }
    }

    #[test]
    fn silence_fallback_does_not_override_blocked() {
        // blocked（等待确认）同样长时间静默，但不能被静默兜底掩掉
        let mut e = engine();
        let a: AgentId = "pi_P04".into();
        for _ in 0..3 {
            assert_eq!(
                e.observe(
                    &a,
                    AgentType::Pi,
                    AgentStatus::Blocked,
                    &pi_idle_screen(60.0)
                ),
                None
            );
        }
    }

    #[test]
    fn seq_monotonic() {
        let mut e = engine();
        let a: AgentId = "claude_E05".into();
        e.report(&a, AgentStatus::Working, HookEvent::Done);
        let lines = ["Do you want to proceed?"];
        e.observe(&a, AgentType::Claude, AgentStatus::Done, &input(lines)); // hook 权威窗口内，屏幕不生效
        let s1 = e.report(&a, AgentStatus::Done, HookEvent::Working).unwrap();
        let s2 = e
            .report(&a, AgentStatus::Working, HookEvent::Blocked)
            .unwrap();
        assert!(s2.seq > s1.seq);
        assert_eq!(s2.to, AgentStatus::Blocked);
    }
}
