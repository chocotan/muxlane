//! muxlane-client：连接远端 muxlane 实例（本地 socket 直连或 SSH 隧道）
mod host;
mod tunnel;

pub use host::{
    parse_target, BootstrapPhase, BootstrapProgress, ClientEvent, HostCfg, MissingPassword,
    RemoteHost, RemoteStage, RemoteState, SshAuth, Target, UploadProgress,
};

pub async fn release_remote_tunnel(host: &str) {
    tunnel::release_tunnel(host).await;
}

use anyhow::Result;
use muxlane_core::model::Snapshot;
use muxlane_core::protocol::{
    b64_decode, b64_encode, read_frame, write_frame, EventMsg, Request, Response,
    TermSubscribeParams, TermSubscribeResult,
};
use tokio::io::{BufReader, ReadHalf, WriteHalf};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum RemoteCompatError {
    #[error(
        "远端 Muxlane 版本过旧：请求创建 {expected}，远端实际创建了 {actual}，请先更新远端 Muxlane"
    )]
    VersionSkew { expected: String, actual: String },
    #[error("{method} failed: unknown_method: unknown method: {method}")]
    MethodUnsupported { method: String },
    #[error("remote does not support capability {feature}")]
    FeatureUnsupported { feature: String },
}

#[derive(Debug, thiserror::Error)]
#[error("{method} failed: {code}: {message}")]
pub struct RpcCallError {
    pub method: String,
    pub code: String,
    pub message: String,
}

/// 到单个对端的连接抽象（newline-JSON RPC）
pub struct Connection {
    reader: BufReader<ReadHalf<UnixStream>>,
    writer: WriteHalf<UnixStream>,
    next_id: u64,
    events: Option<mpsc::UnboundedSender<EventMsg>>,
}

impl Connection {
    pub fn new(stream: UnixStream) -> Self {
        let (reader, writer) = tokio::io::split(stream);
        Connection {
            reader: BufReader::new(reader),
            writer,
            next_id: 1,
            events: None,
        }
    }

    pub fn set_event_handler(&mut self, events: mpsc::UnboundedSender<EventMsg>) {
        self.events = Some(events);
    }

    pub fn into_split(self) -> (RequestWriter, ResponseReader) {
        (
            RequestWriter {
                writer: self.writer,
                next_id: self.next_id,
            },
            ResponseReader {
                reader: self.reader,
            },
        )
    }

    pub async fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = async {
            write_frame(
                &mut self.writer,
                &Request {
                    id,
                    method: method.into(),
                    params,
                },
            )
            .await?;
            loop {
                let v = read_frame(&mut self.reader).await?;
                if v.get("event").is_some() {
                    let event: EventMsg = serde_json::from_value(v)?;
                    if let Some(events) = &self.events {
                        let _ = events.send(event);
                    }
                    continue;
                }
                let resp: Response = serde_json::from_value(v)?;
                if resp.id != id {
                    tracing::warn!(
                        expected_id = id,
                        response_id = resp.id,
                        "discarding unmatched RPC response"
                    );
                    continue;
                }
                return match resp.result {
                    Some(v) => Ok(v),
                    None => {
                        let err = resp.error.unwrap_or(muxlane_core::protocol::RpcError {
                            code: "unknown".into(),
                            message: "no error detail".into(),
                            method: None,
                        });
                        if err.code == "unknown_method" {
                            if let Some(method) = err.method {
                                return Err(RemoteCompatError::MethodUnsupported { method }.into());
                            }
                        }
                        return Err(RpcCallError {
                            method: method.to_string(),
                            code: err.code,
                            message: err.message,
                        }
                        .into());
                    }
                };
            }
        };
        tokio::time::timeout(CALL_TIMEOUT, request)
            .await
            .map_err(|_| {
                anyhow::anyhow!("{} timed out after {}s", method, CALL_TIMEOUT.as_secs())
            })?
    }
}

pub struct RequestWriter {
    writer: WriteHalf<UnixStream>,
    next_id: u64,
}

impl RequestWriter {
    pub async fn call(&mut self, method: &str, params: serde_json::Value) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        write_frame(
            &mut self.writer,
            &Request {
                id,
                method: method.into(),
                params,
            },
        )
        .await?;
        Ok(id)
    }
}

pub struct ResponseReader {
    reader: BufReader<ReadHalf<UnixStream>>,
}

impl ResponseReader {
    /// 读下一帧（Response 或 EventMsg）
    pub async fn next(&mut self) -> Result<Frame> {
        let v = read_frame(&mut self.reader).await?;
        if v.get("event").is_some() {
            Ok(Frame::Event(serde_json::from_value(v)?))
        } else {
            Ok(Frame::Response(serde_json::from_value(v)?))
        }
    }
}

pub enum Frame {
    Response(Response),
    Event(EventMsg),
}

#[derive(Debug, Clone)]
pub enum TermUpdate {
    /// 首帧或 lag/backpressure 后重同步；消费者必须 reset 后重放。
    Resync(Vec<u8>),
    Data(Vec<u8>),
}

pub async fn open(path_or_target: &str) -> Result<Connection> {
    let stream = UnixStream::connect(path_or_target).await?;
    Ok(Connection::new(stream))
}

/// 运行一次终端订阅，直到断线/TermExit。无隐藏后台任务；调用方负责重连循环。
pub async fn stream_term(
    socket: &str,
    agent: &muxlane_core::model::AgentId,
    mut on_update: impl FnMut(TermUpdate) + Send,
) -> Result<()> {
    let mut conn = open(socket).await?;
    let result: TermSubscribeResult = serde_json::from_value(
        conn.call(
            muxlane_core::protocol::methods::TERM_SUBSCRIBE,
            serde_json::to_value(TermSubscribeParams {
                agent: agent.clone(),
            })?,
        )
        .await?,
    )?;
    on_update(TermUpdate::Resync(b64_decode(&result.replay_b64)?));
    let sub_id = result.sub_id;
    let (mut writer, mut reader) = conn.into_split();
    let outcome = loop {
        match reader.next().await? {
            Frame::Event(ev) if ev.event == muxlane_core::protocol::events::TERM_DATA => {
                let d: muxlane_core::protocol::TermDataEvent = serde_json::from_value(ev.params)?;
                if d.agent == *agent {
                    on_update(TermUpdate::Data(b64_decode(&d.data_b64)?));
                }
            }
            Frame::Event(ev) if ev.event == muxlane_core::protocol::events::TERM_RESYNC => {
                let d: muxlane_core::protocol::TermResyncEvent = serde_json::from_value(ev.params)?;
                if d.agent == *agent {
                    on_update(TermUpdate::Resync(b64_decode(&d.replay_b64)?));
                }
            }
            Frame::Event(ev) if ev.event == muxlane_core::protocol::events::TERM_EXIT => {
                break Ok(())
            }
            Frame::Event(_) | Frame::Response(_) => {}
        }
    };
    let _ = writer
        .call(
            muxlane_core::protocol::methods::TERM_UNSUBSCRIBE,
            serde_json::json!({ "sub_id": sub_id }),
        )
        .await;
    outcome
}

/// 拉取一次快照
pub async fn fetch_snapshot(conn: &mut Connection) -> Result<Snapshot> {
    let v = conn
        .call(
            muxlane_core::protocol::methods::STATE_LIST,
            serde_json::json!(null),
        )
        .await?;
    Ok(serde_json::from_value(v)?)
}

pub async fn spawn_agent(
    conn: &mut Connection,
    project: &muxlane_core::model::ProjectId,
    preset: Option<&muxlane_core::AgentPreset>,
) -> anyhow::Result<muxlane_core::model::AgentInstance> {
    let value = conn
        .call(
            muxlane_core::protocol::methods::AGENT_SPAWN,
            serde_json::to_value(muxlane_core::protocol::AgentSpawnParams {
                project: project.clone(),
                agent_type: preset.map(|p| p.agent_type),
                // Shell 预设的 program 来自本机 default_shell_program；远程应
                // 使用远端自己的默认 shell，不能把本机路径（或 basename）当作
                // 远程 override 传过去。
                program: preset.and_then(|p| {
                    (p.agent_type != muxlane_core::model::AgentType::Shell)
                        .then(|| p.program.clone())
                }),
                args: preset.map(|p| p.args.clone()),
                env: preset.map(|p| p.env.clone().into_iter().collect()),
                preset_name: preset.map(|p| p.label.clone()),
            })?,
        )
        .await?;
    let agent: muxlane_core::model::AgentInstance = serde_json::from_value(value)?;
    if let Some(expected) = preset.map(|p| p.agent_type) {
        if agent.agent_type != expected {
            // 旧版远端会忽略 agent_type，表面返回成功但实际创建 Shell；
            // 清理误创建的会话，并把版本不兼容明确反馈给 UI。
            let _ = delete_agent(conn, &agent.id).await;
            return Err(RemoteCompatError::VersionSkew {
                expected: expected.as_str().into(),
                actual: agent.agent_type.as_str().into(),
            }
            .into());
        }
    }
    Ok(agent)
}

pub async fn spawn_shell_agent(
    conn: &mut Connection,
    project: &muxlane_core::model::ProjectId,
) -> anyhow::Result<muxlane_core::model::AgentInstance> {
    spawn_agent(conn, project, None).await
}

pub async fn send_term_input(
    conn: &mut Connection,
    agent: &muxlane_core::model::AgentId,
    data: &[u8],
) -> anyhow::Result<()> {
    conn.call(
        muxlane_core::protocol::methods::TERM_INPUT,
        serde_json::to_value(muxlane_core::protocol::TermInputParams {
            agent: agent.clone(),
            data_b64: b64_encode(data),
        })?,
    )
    .await?;
    Ok(())
}

pub async fn resize_term(
    conn: &mut Connection,
    agent: &muxlane_core::model::AgentId,
    cols: u16,
    rows: u16,
) -> anyhow::Result<()> {
    conn.call(
        muxlane_core::protocol::methods::TERM_RESIZE,
        serde_json::to_value(muxlane_core::protocol::TermResizeParams {
            agent: agent.clone(),
            cols,
            rows,
        })?,
    )
    .await?;
    Ok(())
}

pub async fn add_project(
    conn: &mut Connection,
    path: &str,
    create_if_missing: bool,
) -> anyhow::Result<muxlane_core::model::Project> {
    let value = conn
        .call(
            muxlane_core::protocol::methods::PROJECT_ADD,
            serde_json::to_value(muxlane_core::protocol::ProjectAddParams {
                path: path.into(),
                name: None,
                create_if_missing,
            })?,
        )
        .await?;
    Ok(serde_json::from_value(value)?)
}

pub async fn delete_project(
    conn: &mut Connection,
    project: &muxlane_core::model::ProjectId,
) -> anyhow::Result<muxlane_core::protocol::DeleteScopeResult> {
    let value = conn
        .call(
            muxlane_core::protocol::methods::PROJECT_DELETE,
            serde_json::to_value(muxlane_core::protocol::ProjectDeleteParams {
                project: project.clone(),
            })?,
        )
        .await?;
    Ok(serde_json::from_value(value)?)
}

/// 删除远端 agent 会话（server 同时 kill PTY + 更新 state.list）
pub async fn delete_agent(
    conn: &mut Connection,
    agent: &muxlane_core::model::AgentId,
) -> anyhow::Result<()> {
    conn.call(
        muxlane_core::protocol::methods::AGENT_DELETE,
        serde_json::to_value(muxlane_core::protocol::AgentDeleteParams {
            agent: agent.clone(),
        })?,
    )
    .await?;
    Ok(())
}

pub async fn mark_agent_seen(
    conn: &mut Connection,
    agent: &muxlane_core::model::AgentId,
) -> anyhow::Result<()> {
    conn.call(
        muxlane_core::protocol::methods::AGENT_MARK_SEEN,
        serde_json::to_value(muxlane_core::protocol::AgentMarkSeenParams {
            agent: agent.clone(),
        })?,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxlane_core::model::{AgentInstance, AgentStatus, AgentType};

    #[tokio::test]
    async fn rpc_failures_preserve_the_server_error_code() {
        let (client, peer) = UnixStream::pair().unwrap();
        let peer = tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(peer);
            let mut read = BufReader::new(read);
            let request: Request =
                serde_json::from_value(read_frame(&mut read).await.unwrap()).unwrap();
            write_frame(
                &mut write,
                &Response::err(
                    request.id,
                    muxlane_core::protocol::error_codes::PATH_NOT_FOUND,
                    "missing directory",
                ),
            )
            .await
            .unwrap();
        });

        let mut connection = Connection::new(client);
        let error = connection
            .call("project.add", serde_json::json!({"path": "/missing"}))
            .await
            .unwrap_err();
        let error = error.downcast_ref::<RpcCallError>().unwrap();
        assert_eq!(error.method, "project.add");
        assert_eq!(
            error.code,
            muxlane_core::protocol::error_codes::PATH_NOT_FOUND
        );
        assert_eq!(error.message, "missing directory");
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn compatibility_failures_are_typed() {
        let (client, peer) = UnixStream::pair().unwrap();
        let peer = tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(peer);
            let mut read = BufReader::new(read);

            let request: Request = serde_json::from_value(read_frame(&mut read).await.unwrap())
                .expect("unknown method request");
            write_frame(
                &mut write,
                &Response::method_not_found(request.id, &request.method),
            )
            .await
            .unwrap();

            let request: Request = serde_json::from_value(read_frame(&mut read).await.unwrap())
                .expect("spawn request");
            let agent = AgentInstance {
                id: "shell_old".into(),
                project: "p1".into(),
                agent_type: AgentType::Shell,
                title: "old remote".into(),
                status: AgentStatus::Idle,
                status_since: 0,
                seen: true,
                tmux_session: None,
            };
            write_frame(
                &mut write,
                &Response::ok(request.id, serde_json::to_value(agent).unwrap()),
            )
            .await
            .unwrap();

            let request: Request = serde_json::from_value(read_frame(&mut read).await.unwrap())
                .expect("cleanup request");
            write_frame(
                &mut write,
                &Response::ok(request.id, serde_json::json!({"ok": true})),
            )
            .await
            .unwrap();
        });

        let mut connection = Connection::new(client);
        let error = connection
            .call("project.add", serde_json::Value::Null)
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<RemoteCompatError>(),
            Some(RemoteCompatError::MethodUnsupported { method }) if method == "project.add"
        ));

        let preset = muxlane_core::builtin_presets("bash")
            .into_iter()
            .find(|preset| preset.agent_type == AgentType::Codex)
            .unwrap();
        let error = spawn_agent(&mut connection, &"p1".into(), Some(&preset))
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<RemoteCompatError>(),
            Some(RemoteCompatError::VersionSkew { expected, actual })
                if expected == "codex" && actual == "shell"
        ));
        peer.await.unwrap();
    }
}
