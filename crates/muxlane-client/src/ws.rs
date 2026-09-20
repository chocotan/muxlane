//! WebSocket transport to a self-hosted muxlane-relay (desktop as "phone").
use crate::Connection;
use anyhow::Context as _;
use futures_util::{Sink, SinkExt, StreamExt};
use muxlane_core::model::MachineInfo;
use muxlane_core::protocol::{PairBeginParams, PairBeginResult};
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_tungstenite::tungstenite::Message;

/// Pair this desktop with a relay host via its 8-digit code.
/// Returns the long-lived token and the host's machine identity.
pub async fn pair_relay(
    url: &str,
    code: &str,
    device: &str,
) -> anyhow::Result<(String, MachineInfo)> {
    let url = join(url, &format!("pair/{code}"));
    let mut conn = ws_connection(&url).await?;
    let value = conn
        .call(
            muxlane_core::protocol::methods::PAIR_BEGIN,
            serde_json::to_value(PairBeginParams {
                code: Some(code.to_string()),
                token: None,
                device: Some(device.to_string()),
            })?,
        )
        .await?;
    let result: PairBeginResult = serde_json::from_value(value)?;
    Ok((result.token, result.machine))
}

/// Open an RPC connection to a relay host using a previously issued token.
pub async fn connect_relay(url: &str, host_id: &str, token: &str) -> anyhow::Result<Connection> {
    let url = join(url, &format!("phone/{host_id}"));
    let mut conn = ws_connection(&url).await?;
    conn.call(
        muxlane_core::protocol::methods::PAIR_BEGIN,
        serde_json::to_value(PairBeginParams {
            code: None,
            token: Some(token.to_string()),
            device: None,
        })?,
    )
    .await?;
    Ok(conn)
}

fn join(base: &str, path: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    format!("{base}/{path}")
}

async fn ws_connection(url: &str) -> anyhow::Result<Connection> {
    let (ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .with_context(|| format!("connect {url}"))?;
    let stream = WsStream::new(ws);
    Ok(Connection::new(stream))
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// newline-JSON frames over WebSocket text messages.
struct WsStream {
    sink: futures_util::stream::SplitSink<Ws, Message>,
    incoming: Arc<std::sync::Mutex<Incoming>>,
    reader_task: tokio::task::JoinHandle<()>,
}

struct Incoming {
    buf: VecDeque<u8>,
    closed: bool,
    wakers: Vec<Waker>,
}

impl WsStream {
    fn new(ws: Ws) -> Self {
        let (sink, mut stream) = ws.split();
        let incoming = Arc::new(std::sync::Mutex::new(Incoming {
            buf: VecDeque::new(),
            closed: false,
            wakers: Vec::new(),
        }));
        let reader_task = {
            let incoming = Arc::clone(&incoming);
            tokio::spawn(async move {
                while let Some(message) = stream.next().await {
                    match message {
                        Ok(Message::Text(text)) => {
                            feed(&incoming, text.as_bytes());
                        }
                        Ok(Message::Binary(data)) => feed(&incoming, data.as_ref()),
                        Ok(Message::Close(_)) => break,
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
                let mut guard = incoming.lock().expect("incoming poisoned");
                guard.closed = true;
                for waker in guard.wakers.drain(..) {
                    waker.wake();
                }
            })
        };
        Self {
            sink,
            incoming,
            reader_task,
        }
    }
}

fn feed(incoming: &Arc<std::sync::Mutex<Incoming>>, bytes: &[u8]) {
    let mut guard = incoming.lock().expect("incoming poisoned");
    // 每条 WS 文本消息是一帧；newline-JSON 读端靠 '\n' 分隔，补回帧尾。
    guard.buf.extend(bytes);
    guard.buf.push_back(b'\n');
    for waker in guard.wakers.drain(..) {
        waker.wake();
    }
}

impl Drop for WsStream {
    fn drop(&mut self) {
        self.reader_task.abort();
    }
}

impl AsyncRead for WsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let mut incoming = self.incoming.lock().expect("incoming poisoned");
        if !incoming.buf.is_empty() {
            let n = incoming.buf.len().min(buf.remaining());
            for byte in incoming.buf.drain(..n) {
                buf.put_slice(&[byte]);
            }
            return Poll::Ready(Ok(()));
        }
        if incoming.closed {
            return Poll::Ready(Ok(()));
        }
        incoming.wakers.push(cx.waker().clone());
        Poll::Pending
    }
}

impl AsyncWrite for WsStream {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let text = String::from_utf8_lossy(buf).trim_end().to_string();
        let this = self.get_mut();
        match Sink::start_send(
            std::pin::Pin::new(&mut this.sink),
            Message::Text(text.into()),
        ) {
            Ok(()) => Poll::Ready(Ok(buf.len())),
            Err(error) => Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                error,
            ))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut().sink.poll_flush_unpin(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(error)) => Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                error,
            ))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut().sink.poll_close_unpin(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(error)) => Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                error,
            ))),
            Poll::Pending => Poll::Pending,
        }
    }
}
