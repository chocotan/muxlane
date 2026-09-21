//! Outbound WebSocket client to a self-hosted muxlane-relay.
use crate::MuxlaneServer;
use anyhow::Context as _;
use futures_util::{SinkExt, StreamExt};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

const PAIR_TTL: Duration = Duration::from_secs(5 * 60);
const PAIR_MAX_ATTEMPTS: u32 = 5;
const DIAL_FRAME_PREFIX: &str = "__muxlane_dial__:";

#[derive(Clone, Debug)]
pub struct PairOffer {
    pub code: String,
    pub relay_url: String,
    pub expires_at: Instant,
}

struct PairState {
    code: String,
    expires_at: Instant,
    attempts: u32,
}

#[derive(Clone)]
pub struct RelayHandle {
    inner: Arc<Mutex<RelayInner>>,
    started: Arc<AtomicBool>,
}

struct RelayInner {
    url: Option<String>,
    token: Option<String>,
    pair: Option<PairState>,
    code_tx: Option<mpsc::UnboundedSender<String>>,
}

impl Default for RelayHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl RelayHandle {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RelayInner {
                url: None,
                token: None,
                pair: None,
                code_tx: None,
            })),
            started: Arc::new(AtomicBool::new(false)),
        }
    }

    pub async fn set_url(&self, url: Option<String>) {
        self.inner.lock().await.url = url.filter(|value| !value.trim().is_empty());
    }

    pub async fn set_token(&self, token: Option<String>) {
        self.inner.lock().await.token = token.filter(|value| !value.trim().is_empty());
    }

    pub async fn token(&self) -> Option<String> {
        self.inner.lock().await.token.clone()
    }

    pub async fn url(&self) -> Option<String> {
        self.inner.lock().await.url.clone()
    }

    pub async fn bind_code_sender(&self, tx: mpsc::UnboundedSender<String>) {
        self.inner.lock().await.code_tx = Some(tx);
    }

    pub async fn offer_code(&self) -> anyhow::Result<PairOffer> {
        let mut inner = self.inner.lock().await;
        let relay_url = inner.url.clone().context("relay URL is not configured")?;
        let code = random_code();
        let expires_at = Instant::now() + PAIR_TTL;
        inner.pair = Some(PairState {
            code: code.clone(),
            expires_at,
            attempts: 0,
        });
        if let Some(tx) = &inner.code_tx {
            let _ = tx.send(code.clone());
        }
        Ok(PairOffer {
            code,
            relay_url,
            expires_at,
        })
    }

    pub async fn consume_code(&self, code: &str) -> bool {
        let mut inner = self.inner.lock().await;
        let Some(pair) = inner.pair.as_mut() else {
            return false;
        };
        if pair.expires_at <= Instant::now() {
            inner.pair = None;
            return false;
        }
        if pair.code != code {
            pair.attempts += 1;
            if pair.attempts >= PAIR_MAX_ATTEMPTS {
                inner.pair = None;
            }
            return false;
        }
        inner.pair = None;
        true
    }

    async fn pending_code(&self) -> Option<String> {
        self.inner
            .lock()
            .await
            .pair
            .as_ref()
            .map(|pair| pair.code.clone())
    }

    pub(crate) fn mark_started(&self) -> bool {
        self.started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
}

fn random_code() -> String {
    use rand::Rng;
    let n: u32 = rand::rng().random_range(0..100_000_000);
    format!("{n:08}")
}

pub async fn run(server: Arc<MuxlaneServer>, url: String) -> anyhow::Result<()> {
    let url = url.trim().trim_end_matches('/').to_string();
    if !url.is_empty() {
        server.relay().set_url(Some(url)).await;
    }
    loop {
        let Some(base) = server.relay().url().await else {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        };
        let host_id = server.machine_id();
        let ws_url = join_url(&base, &format!("host/{host_id}"));
        let token = server.relay().token().await;
        match connect_once(Arc::clone(&server), &ws_url, token).await {
            Ok(()) => tracing::info!("relay host disconnected, reconnecting"),
            Err(error) => tracing::warn!("relay host failed: {error:?}"),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

fn join_url(base: &str, path: &str) -> String {
    if base.ends_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

async fn connect_once(
    server: Arc<MuxlaneServer>,
    ws_url: &str,
    token: Option<String>,
) -> anyhow::Result<()> {
    let request = websocket_request(ws_url, token.as_deref())?;
    let (ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .with_context(|| format!("connect {ws_url}"))?;
    let (mut sink, mut stream) = ws.split();
    let (code_tx, mut code_rx) = mpsc::unbounded_channel();
    server.relay().bind_code_sender(code_tx).await;
    tracing::info!(%ws_url, "relay host connected");

    if let Some(code) = server.relay().pending_code().await {
        sink.send(Message::Text(code.into())).await?;
        wait_ack(&mut stream, &mut sink).await?;
        tracing::debug!("relay pair code registered");
    }

    loop {
        tokio::select! {
            code = code_rx.recv() => {
                let Some(code) = code else { break };
                sink.send(Message::Text(code.into())).await?;
                wait_ack(&mut stream, &mut sink).await?;
            }
            message = stream.next() => {
                match message {
                    Some(Ok(Message::Text(text))) if text.starts_with(DIAL_FRAME_PREFIX) => {
                        let chan = text[DIAL_FRAME_PREFIX.len()..].to_string();
                        tracing::debug!(%chan, "relay phone channel dialing");
                        let base = base_url(ws_url);
                        let host = Arc::clone(&server);
                        tokio::spawn(async move {
                            let token = host.relay().token().await;
                            if let Err(error) = run_channel(host, &base, &chan, token).await {
                                tracing::debug!(%chan, %error, "relay channel closed");
                            }
                        });
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        sink.send(Message::Pong(payload)).await?;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => return Err(error.into()),
                }
            }
        }
    }
    Ok(())
}

fn base_url(host_ws_url: &str) -> String {
    host_ws_url
        .rsplit_once("/host/")
        .map(|(base, _)| base.to_string())
        .unwrap_or_else(|| host_ws_url.to_string())
}

async fn run_channel(
    server: Arc<MuxlaneServer>,
    base: &str,
    chan: &str,
    token: Option<String>,
) -> anyhow::Result<()> {
    let url = join_url(base, &format!("chan/{chan}"));
    let request = websocket_request(&url, token.as_deref())?;
    let (ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .with_context(|| format!("connect {url}"))?;
    let (mut sink, mut stream) = ws.split();
    let session = RelaySession::new();
    let phone = session.clone();
    let host = Arc::clone(&server);
    let task = tokio::spawn(async move { host.handle_rpc(phone, true).await });
    pump_session(&mut sink, &mut stream, session).await?;
    let _ = task.await;
    Ok(())
}

fn websocket_request(
    url: &str,
    token: Option<&str>,
) -> anyhow::Result<tokio_tungstenite::tungstenite::http::Request<()>> {
    let mut request = url.into_client_request()?;
    if let Some(token) = token {
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {token}").parse()?,
        );
    }
    Ok(request)
}

/// Validate a relay token without taking over the real host registration.
pub async fn validate_relay_credentials(
    relay_url: &str,
    token: &str,
) -> anyhow::Result<()> {
    let probe = join_url(
        relay_url.trim().trim_end_matches('/'),
        "host/muxlane-auth-probe",
    );
    let request = websocket_request(&probe, Some(token))?;
    let (ws, _) = tokio_tungstenite::connect_async(request).await?;
    drop(ws);
    Ok(())
}

async fn wait_ack<S>(
    stream: &mut futures_util::stream::SplitStream<S>,
    sink: &mut futures_util::stream::SplitSink<S, Message>,
) -> anyhow::Result<()>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    loop {
        match stream.next().await {
            Some(Ok(Message::Text(text))) if text == "ok" => return Ok(()),
            Some(Ok(Message::Ping(payload))) => {
                sink.send(Message::Pong(payload)).await?;
            }
            other => anyhow::bail!("unexpected pair ack: {other:?}"),
        }
    }
}

async fn pump_session<S>(
    sink: &mut futures_util::stream::SplitSink<S, Message>,
    stream: &mut futures_util::stream::SplitStream<S>,
    session: RelaySession,
) -> anyhow::Result<()>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    loop {
        tokio::select! {
            message = stream.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        if session.push_in(text.as_bytes().to_vec()).is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        if session.push_in(data.as_ref().to_vec()).is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        sink.send(Message::Pong(payload)).await?;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => return Err(error.into()),
                }
            }
            outgoing = session.pop_out() => {
                let Some(bytes) = outgoing else { break };
                let text = String::from_utf8_lossy(&bytes).trim_end().to_string();
                sink.send(Message::Text(text.into())).await?;
            }
        }
    }
    session.close();
    Ok(())
}

#[derive(Clone)]
struct RelaySession {
    incoming: Arc<std::sync::Mutex<Incoming>>,
    outgoing: mpsc::UnboundedSender<Vec<u8>>,
    outgoing_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<Vec<u8>>>>>,
}

struct Incoming {
    buf: std::collections::VecDeque<u8>,
    closed: bool,
    wakers: Vec<std::task::Waker>,
}

impl RelaySession {
    fn new() -> Self {
        let (outgoing, outgoing_rx) = mpsc::unbounded_channel();
        Self {
            incoming: Arc::new(std::sync::Mutex::new(Incoming {
                buf: std::collections::VecDeque::new(),
                closed: false,
                wakers: Vec::new(),
            })),
            outgoing,
            outgoing_rx: Arc::new(Mutex::new(Some(outgoing_rx))),
        }
    }

    fn push_in(&self, mut bytes: Vec<u8>) -> Result<(), ()> {
        if !bytes.ends_with(b"\n") {
            bytes.push(b'\n');
        }
        let mut incoming = self.incoming.lock().expect("incoming poisoned");
        if incoming.closed {
            return Err(());
        }
        incoming.buf.extend(bytes);
        for waker in incoming.wakers.drain(..) {
            waker.wake();
        }
        Ok(())
    }

    async fn pop_out(&self) -> Option<Vec<u8>> {
        let mut slot = self.outgoing_rx.lock().await;
        slot.as_mut()?.recv().await
    }

    fn close(&self) {
        let mut incoming = self.incoming.lock().expect("incoming poisoned");
        incoming.closed = true;
        for waker in incoming.wakers.drain(..) {
            waker.wake();
        }
    }
}

impl AsyncRead for RelaySession {
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

impl AsyncWrite for RelaySession {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.outgoing
            .send(buf.to_vec())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "relay closed"))?;
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
