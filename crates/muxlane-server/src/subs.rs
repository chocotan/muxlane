//! 订阅注册表：term.data 无损边界；lag/backpressure 后显式 term.resync。
//! 每个订阅一个独立转发任务，无数据时完全挂起零唤醒
//! （替代原 250Hz 轮询泵：空闲时每秒 250 次唤醒 + 锁竞争）。
use bytes::Bytes;
use muxlane_core::model::AgentId;
use muxlane_core::protocol::{EventMsg, TermDataEvent, TermResyncEvent};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, Mutex, Notify};

const RESYNC_MIN_INTERVAL: Duration = Duration::from_secs(1);

struct SubHandle {
    agent: AgentId,
    ending: Arc<AtomicBool>,
    wake: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Default)]
pub struct SubRegistry {
    subs: HashMap<String, SubHandle>,
    /// 注册表自身的 Weak，供转发任务退出时自行清理条目。
    me: Option<Weak<Mutex<SubRegistry>>>,
}

impl SubRegistry {
    /// 绑定注册表自身的 Arc（构造后调用一次），转发任务结束时可自行摘除条目。
    pub fn bind_self(&mut self, me: &Arc<Mutex<SubRegistry>>) {
        self.me = Some(Arc::downgrade(me));
    }

    pub fn add(
        &mut self,
        sub_id: &str,
        agent: &AgentId,
        sink: mpsc::Sender<EventMsg>,
        rx: broadcast::Receiver<Bytes>,
        session: Arc<muxlane_term::PtySession>,
    ) {
        let ending = Arc::new(AtomicBool::new(false));
        let wake = Arc::new(Notify::new());
        let task = tokio::spawn(forward(
            sub_id.to_string(),
            agent.clone(),
            session,
            sink,
            rx,
            Arc::clone(&ending),
            Arc::clone(&wake),
            self.me.clone(),
        ));
        self.subs.insert(
            sub_id.to_string(),
            SubHandle {
                agent: agent.clone(),
                ending,
                wake,
                task,
            },
        );
    }

    pub fn remove(&mut self, sub_id: &str) {
        if let Some(handle) = self.subs.remove(sub_id) {
            handle.task.abort();
        }
    }

    pub fn remove_agent(&mut self, agent: &AgentId) {
        let ids: Vec<String> = self
            .subs
            .iter()
            .filter(|(_, handle)| &handle.agent == agent)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.remove(&id);
        }
    }

    /// 标记 agent 结束；转发任务会在 sink 可写时先发 term.exit，再自行摘除订阅。
    pub fn mark_agent_exit(&mut self, agent: &AgentId) {
        for handle in self.subs.values().filter(|handle| &handle.agent == agent) {
            handle.ending.store(true, Ordering::Relaxed);
            handle.wake.notify_one();
        }
    }

    pub fn len(&self) -> usize {
        self.subs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.subs.is_empty()
    }
}

/// 单订阅转发循环：事件驱动，无数据时挂起在 rx.recv()/wake 上，零空转。
#[allow(clippy::too_many_arguments)]
async fn forward(
    sub_id: String,
    agent: AgentId,
    session: Arc<muxlane_term::PtySession>,
    sink: mpsc::Sender<EventMsg>,
    mut rx: broadcast::Receiver<Bytes>,
    ending: Arc<AtomicBool>,
    wake: Arc<Notify>,
    registry: Option<Weak<Mutex<SubRegistry>>>,
) {
    let mut last_resync: Option<Instant> = None;
    loop {
        if ending.load(Ordering::Relaxed) {
            // 等 sink 可写再发 term.exit（await 容量，等同原泵的重试语义），然后结束。
            let msg = EventMsg::new(
                muxlane_core::protocol::events::TERM_EXIT,
                serde_json::to_value(muxlane_core::protocol::TermExitEvent {
                    agent: agent.clone(),
                })
                .unwrap_or_default(),
            );
            let _ = sink.send(msg).await;
            break;
        }
        tokio::select! {
            _ = wake.notified() => {}
            result = rx.recv() => match result {
                Ok(bytes) => {
                    let msg = EventMsg::new(
                        muxlane_core::protocol::events::TERM_DATA,
                        serde_json::to_value(TermDataEvent {
                            agent: agent.clone(),
                            data_b64: muxlane_core::protocol::b64_encode(&bytes),
                        })
                        .unwrap_or_default(),
                    );
                    // 背压：等待 sink 容量；等得太久导致 Lagged 时由下方 resync 兜底。
                    if sink.send(msg).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if let Some(last) = last_resync {
                        let elapsed = last.elapsed();
                        if elapsed < RESYNC_MIN_INTERVAL {
                            // 重 lag 合并：节流窗口内不立即重发（睡眠可被 exit 唤醒打断）。
                            // 注意：睡醒后必须 fall through 发 resync——节流期间被覆盖
                            // 丢弃的字节已形成空洞，继续发增量会损坏对端终端状态。
                            tokio::select! {
                                _ = tokio::time::sleep(RESYNC_MIN_INTERVAL - elapsed) => {}
                                _ = wake.notified() => continue
                            }
                        }
                    }
                    // 队列满/Lagged 不是可忽略条件：以完整 replay resync 重建无损边界。
                    let (snapshot, new_rx) = session.subscribe();
                    rx = new_rx;
                    let msg = EventMsg::new(
                        muxlane_core::protocol::events::TERM_RESYNC,
                        serde_json::to_value(TermResyncEvent {
                            agent: agent.clone(),
                            replay_b64: muxlane_core::protocol::b64_encode(&snapshot),
                        })
                        .unwrap_or_default(),
                    );
                    if sink.send(msg).await.is_err() {
                        break;
                    }
                    last_resync = Some(Instant::now());
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
    // 自行摘除条目（remove()/remove_agent() 先行移除时为 no-op）。
    if let Some(registry) = registry.and_then(|weak| weak.upgrade()) {
        registry.lock().await.remove(&sub_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxlane_core::model::AgentType;
    use std::time::Duration;

    fn spawn_session(agent: &str, script: &str) -> Arc<muxlane_term::PtySession> {
        muxlane_term::PtySession::spawn(muxlane_term::LaunchCfg {
            agent: agent.into(),
            agent_type: AgentType::Shell,
            cwd: std::env::temp_dir(),
            env: vec![],
            program_override: Some("bash".into()),
            args: vec!["-c".into(), script.into()],
            cols: 80,
            rows: 24,
            tmux_session: None,
        })
        .unwrap()
    }

    fn bound_registry() -> Arc<Mutex<SubRegistry>> {
        let registry = Arc::new(Mutex::new(SubRegistry::default()));
        registry
            .try_lock()
            .expect("fresh registry")
            .bind_self(&registry);
        registry
    }

    #[tokio::test]
    async fn forwarder_delivers_all_frames_in_order() {
        let agent = "shell_burst".to_string();
        let session = spawn_session(&agent, "sleep 2");
        let (tap_tx, rx) = broadcast::channel(128);
        let (sink, mut sink_rx) = mpsc::channel(256);
        let registry = bound_registry();
        registry
            .lock()
            .await
            .add("burst", &agent, sink, rx, Arc::clone(&session));

        for byte in 0..64u32 {
            tap_tx.send(Bytes::from(vec![byte as u8])).unwrap();
        }
        for expected in 0..64u32 {
            let message = tokio::time::timeout(Duration::from_secs(2), sink_rx.recv())
                .await
                .expect("frame within deadline")
                .expect("frame");
            assert_eq!(message.event, muxlane_core::protocol::events::TERM_DATA);
            let event: TermDataEvent = serde_json::from_value(message.params).unwrap();
            assert_eq!(
                muxlane_core::protocol::b64_decode(&event.data_b64).unwrap(),
                vec![expected as u8]
            );
        }
        session.kill();
    }

    #[tokio::test]
    async fn lagged_receiver_recovers_with_explicit_resync() {
        let agent = "shell_resync".to_string();
        let session = spawn_session(&agent, "sleep .1; echo RESYNC-MARK; sleep 1");
        let (tap_tx, rx) = broadcast::channel(8);
        let (tx, mut sink_rx) = mpsc::channel(1);
        // 预填满 connection queue，逼转发任务阻塞在 sink.send 上。
        tx.try_send(EventMsg::new("dummy", serde_json::json!({})))
            .unwrap();
        let registry = bound_registry();
        registry
            .lock()
            .await
            .add("s1", &agent, tx, rx, Arc::clone(&session));

        // 等 replay 包含 RESYNC-MARK，然后灌 20 条（容量 8 → 必 Lagged）。
        tokio::time::sleep(Duration::from_millis(250)).await;
        for index in 0..20u8 {
            tap_tx.send(Bytes::from(vec![index])).unwrap();
        }

        // dummy 出队 → 被阻的 term.data 完成 → Lagged 分支发 resync。
        assert_eq!(sink_rx.recv().await.unwrap().event, "dummy");
        let mut resync = None;
        for _ in 0..4 {
            let msg = tokio::time::timeout(Duration::from_secs(2), sink_rx.recv())
                .await
                .expect("frame within deadline")
                .expect("frame");
            if msg.event == muxlane_core::protocol::events::TERM_RESYNC {
                resync = Some(msg);
                break;
            }
        }
        let resync = resync.expect("resync after lag");
        let event: TermResyncEvent = serde_json::from_value(resync.params).unwrap();
        let replay = muxlane_core::protocol::b64_decode(&event.replay_b64).unwrap();
        assert!(replay.windows(11).any(|w| w == b"RESYNC-MARK"));
        session.kill();
    }

    #[tokio::test]
    async fn repeated_lag_within_throttle_window_still_resyncs() {
        // 交互式 bash：按需产生输出，逼出节流窗口内的第二次 Lagged。
        let agent = "shell_double_lag".to_string();
        let session = muxlane_term::PtySession::spawn(muxlane_term::LaunchCfg {
            agent: agent.clone(),
            agent_type: AgentType::Shell,
            cwd: std::env::temp_dir(),
            env: vec![],
            program_override: Some("bash".into()),
            args: vec!["-i".into()],
            cols: 80,
            rows: 24,
            tmux_session: None,
        })
        .unwrap();
        let (tap_tx, rx) = broadcast::channel(8);
        let (tx, mut sink_rx) = mpsc::channel(8);
        let registry = bound_registry();
        registry
            .lock()
            .await
            .add("dl", &agent, tx, rx, Arc::clone(&session));

        // Lagged #1：本地 tap 溢出（sink 容量 8，不读 → 转发阻住 → tap 溢出）。
        tokio::time::sleep(Duration::from_millis(300)).await;
        for index in 0..20u8 {
            tap_tx.send(Bytes::from(vec![index])).unwrap();
        }
        // 边读边推进，直到 resync #1。
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let msg = tokio::time::timeout(Duration::from_millis(500), sink_rx.recv())
                .await
                .expect("frame within deadline")
                .expect("frame");
            if msg.event == muxlane_core::protocol::events::TERM_RESYNC {
                break;
            }
            assert!(Instant::now() < deadline, "resync #1 not seen");
        }
        // Lagged #2：seq 输出 ~7MB（~850 个 8KB 帧）；不再读 sink → 转发阻住 →
        // session tap（256）溢出，距 resync #1 不到 1s，进入节流分支。
        session.write_input(b"seq 1 1000000\n");
        tokio::time::sleep(Duration::from_millis(1200)).await;

        // 节流醒来后必须 fall through 发第二个 resync；若继续发增量则有洞。
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut second = None;
        while Instant::now() < deadline {
            let msg = tokio::time::timeout(Duration::from_millis(500), sink_rx.recv())
                .await
                .expect("frame within deadline")
                .expect("frame");
            if msg.event == muxlane_core::protocol::events::TERM_RESYNC {
                second = Some(msg);
                break;
            }
        }
        let second = second.expect("throttled second lag must still resync after the wait");
        let event: TermResyncEvent = serde_json::from_value(second.params).unwrap();
        let replay = muxlane_core::protocol::b64_decode(&event.replay_b64).unwrap();
        assert!(
            replay.windows(5).any(|w| w == b"99999"),
            "second resync carries flooded replay tail"
        );
        session.kill();
    }

    #[tokio::test]
    async fn ending_agent_emits_term_exit_before_subscription_cleanup() {
        let agent = "shell_exit_notice".to_string();
        let session = spawn_session(&agent, "sleep 1");
        let (_, rx) = session.subscribe();
        let (tx, mut sink_rx) = mpsc::channel(2);
        let registry = bound_registry();
        registry
            .lock()
            .await
            .add("ending", &agent, tx, rx, Arc::clone(&session));

        registry.lock().await.mark_agent_exit(&agent);

        let msg = tokio::time::timeout(Duration::from_secs(2), sink_rx.recv())
            .await
            .expect("exit within deadline")
            .expect("exit frame");
        assert_eq!(msg.event, muxlane_core::protocol::events::TERM_EXIT);
        let event: muxlane_core::protocol::TermExitEvent =
            serde_json::from_value(msg.params).unwrap();
        assert_eq!(event.agent, agent);
        // 清理是异步的：轮询等待转发任务自行摘除。
        let deadline = Instant::now() + Duration::from_secs(2);
        while !registry.lock().await.is_empty() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(registry.lock().await.is_empty());
        session.kill();
    }
}
