//! Muxlane relay: WebSocket rooms that dial host-side channels per phone.
//! Does not parse RPC. TLS is terminated by the deployer.
//!
//! Host stays on a control socket at `/host/{id}`. A phone on `/pair/{code}`
//! or `/phone/{id}` gets a channel id; the relay sends the host a
//! `__muxlane_dial__:{chan}` control frame, the host connects to
//! `/chan/{chan}`, and that socket is spliced to the phone. Any number of
//! channels per host.
use anyhow::Context as _;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

const PAIR_TTL: Duration = Duration::from_secs(5 * 60);
const IDLE_HOST_TTL: Duration = Duration::from_secs(5 * 60);
const CHAN_TTL: Duration = Duration::from_secs(60);
const PAIR_MAX_ATTEMPTS: u32 = 5;
pub const DIAL_FRAME_PREFIX: &str = "__muxlane_dial__:";

static NEXT_CHAN: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind {
    Host,
    Pair,
    Phone,
    Chan,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParsedPath {
    pub kind: PathKind,
    pub id: String,
}

pub fn parse_path(path: &str) -> Option<ParsedPath> {
    let path = path.split('?').next().unwrap_or(path).trim_end_matches('/');
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let kind = match parts.next()? {
        "host" => PathKind::Host,
        "pair" => PathKind::Pair,
        "phone" => PathKind::Phone,
        "chan" => PathKind::Chan,
        _ => return None,
    };
    let id = parts.next()?.to_string();
    if id.is_empty() || parts.next().is_some() {
        return None;
    }
    if !id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return None;
    }
    Some(ParsedPath { kind, id })
}

struct PairOffer {
    host_id: String,
    expires_at: Instant,
    attempts: u32,
}

struct HostSlot {
    tx: mpsc::UnboundedSender<String>,
    last_seen: Instant,
}

struct ChanSlot {
    inbound: mpsc::UnboundedReceiver<Message>,
    outbound: mpsc::UnboundedSender<Message>,
    created: Instant,
}

struct Inner {
    hosts: HashMap<String, HostSlot>,
    pairs: HashMap<String, PairOffer>,
    chans: HashMap<String, ChanSlot>,
}

#[derive(Clone)]
pub struct Relay {
    inner: Arc<Mutex<Inner>>,
    auth_token: Option<Arc<str>>,
}

impl Relay {
    pub fn new() -> Self {
        Self::with_auth_token(None)
    }

    pub fn from_env() -> anyhow::Result<Self> {
        let token = std::env::var("MUXLANE_RELAY_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("MUXLANE_RELAY_TOKEN is required"))?;
        Ok(Self::with_auth_token(Some(token)))
    }

    pub fn with_auth_token(token: Option<String>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                hosts: HashMap::new(),
                pairs: HashMap::new(),
                chans: HashMap::new(),
            })),
            auth_token: token.map(Arc::<str>::from),
        }
    }

    pub async fn serve(self, bind: &str) -> anyhow::Result<()> {
        let listener = TcpListener::bind(bind)
            .await
            .with_context(|| format!("bind {bind}"))?;
        tracing::info!(bind, "muxlane-relay listening");
        let janitor = self.clone();
        tokio::spawn(async move { janitor.janitor().await });
        loop {
            let (stream, addr) = listener.accept().await?;
            let relay = self.clone();
            tokio::spawn(async move {
                if let Err(error) = relay.handle_stream(stream).await {
                    tracing::debug!(%addr, %error, "relay connection closed");
                }
            });
        }
    }

    async fn janitor(self) {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let now = Instant::now();
            let mut inner = self.inner.lock().await;
            inner.pairs.retain(|_, offer| offer.expires_at > now);
            inner
                .chans
                .retain(|_, chan| chan.created + CHAN_TTL > now && !chan.outbound.is_closed());
            inner.hosts.retain(|_, slot| {
                !slot.tx.is_closed() && now.duration_since(slot.last_seen) < IDLE_HOST_TTL
            });
        }
    }

    #[allow(clippy::result_large_err)] // tungstenite 握手回调签名固定
    async fn handle_stream(&self, stream: TcpStream) -> anyhow::Result<()> {
        let mut uri = None;
        let mut auth = None;
        let ws = tokio_tungstenite::accept_hdr_async(
            stream,
            |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                uri = Some(request.uri().path().to_string());
                auth = request
                    .headers()
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.strip_prefix("Bearer "))
                    .map(str::to_owned);
                Ok(response)
            },
        )
        .await?;
        let path = uri.unwrap_or_else(|| "/".into());
        let parsed = parse_path(&path).ok_or_else(|| anyhow::anyhow!("unknown path {path}"))?;
        if matches!(parsed.kind, PathKind::Host | PathKind::Chan)
            && self.auth_token.as_deref().is_some_and(|expected| {
                auth.as_deref() != Some(expected)
            })
        {
            anyhow::bail!("relay authentication failed");
        }
        match parsed.kind {
            PathKind::Host => self.handle_host(parsed.id, ws).await,
            PathKind::Pair => self.handle_phone(PhoneKind::Pair(parsed.id), ws).await,
            PathKind::Phone => self.handle_phone(PhoneKind::Host(parsed.id), ws).await,
            PathKind::Chan => self.handle_chan(parsed.id, ws).await,
        }
    }

    async fn handle_host(
        &self,
        host_id: String,
        ws: WebSocketStream<TcpStream>,
    ) -> anyhow::Result<()> {
        let (mut sink, mut stream) = ws.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        {
            let mut inner = self.inner.lock().await;
            // 旧控制连接顶掉：新连接生效，旧连接的写端关闭后自然退出。
            inner.hosts.insert(
                host_id.clone(),
                HostSlot {
                    tx,
                    last_seen: Instant::now(),
                },
            );
        }
        tracing::info!(%host_id, "host connected");

        let result = async {
            loop {
                tokio::select! {
                    control = rx.recv() => {
                        let Some(text) = control else { break };
                        sink.send(Message::Text(text.into())).await?;
                    }
                    message = stream.next() => {
                        match message {
                            Some(Ok(Message::Text(text))) => {
                                self.register_pair_code(&host_id, text.as_str().trim()).await?;
                                sink.send(Message::Text("ok".into())).await?;
                            }
                            Some(Ok(Message::Ping(payload))) => {
                                sink.send(Message::Pong(payload)).await?;
                            }
                            Some(Ok(Message::Close(_))) | None => break,
                            Some(Ok(_)) => {}
                            Some(Err(error)) => return Err(error.into()),
                        }
                        if let Some(slot) = self.inner.lock().await.hosts.get_mut(&host_id) {
                            slot.last_seen = Instant::now();
                        }
                    }
                }
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;

        self.drop_host(&host_id).await;
        result
    }

    async fn drop_host(&self, host_id: &str) {
        let mut inner = self.inner.lock().await;
        inner.hosts.remove(host_id);
        inner.pairs.retain(|_, offer| offer.host_id != host_id);
        // 关掉挂起的通道：手机端 sink 关闭，通道侧 recv 得到 None 自然结束。
        inner.chans.clear();
    }

    async fn register_pair_code(&self, host_id: &str, code: &str) -> anyhow::Result<()> {
        if code.len() != 8 || !code.chars().all(|ch| ch.is_ascii_digit()) {
            anyhow::bail!("invalid pair code");
        }
        let mut inner = self.inner.lock().await;
        inner
            .pairs
            .retain(|_, offer| offer.expires_at > Instant::now());
        inner.pairs.insert(
            code.to_string(),
            PairOffer {
                host_id: host_id.to_string(),
                expires_at: Instant::now() + PAIR_TTL,
                attempts: 0,
            },
        );
        Ok(())
    }

    async fn handle_phone(
        &self,
        kind: PhoneKind,
        ws: WebSocketStream<TcpStream>,
    ) -> anyhow::Result<()> {
        let host_id = match &kind {
            PhoneKind::Host(id) => id.clone(),
            PhoneKind::Pair(code) => self.take_pair(code).await?,
        };
        let (mut sink, mut stream) = ws.split();
        let (to_host, from_phone) = mpsc::unbounded_channel();
        let (to_phone, mut from_host) = mpsc::unbounded_channel();
        let chan_id = format!("chan_{}", NEXT_CHAN.fetch_add(1, Ordering::Relaxed));
        {
            let inner = self.inner.lock().await;
            let Some(slot) = inner.hosts.get(&host_id) else {
                anyhow::bail!("host {host_id} is offline");
            };
            slot.tx
                .send(format!("{DIAL_FRAME_PREFIX}{chan_id}"))
                .map_err(|_| anyhow::anyhow!("host {host_id} is offline"))?;
        }
        self.inner.lock().await.chans.insert(
            chan_id,
            ChanSlot {
                inbound: from_phone,
                outbound: to_phone,
                created: Instant::now(),
            },
        );

        loop {
            tokio::select! {
                message = stream.next() => {
                    match message {
                        Some(Ok(message @ (Message::Text(_) | Message::Binary(_)))) => {
                            if to_host.send(message).is_err() {
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
                message = from_host.recv() => {
                    let Some(message) = message else { break };
                    sink.send(message).await?;
                }
            }
        }
        Ok(())
    }

    async fn handle_chan(
        &self,
        chan_id: String,
        ws: WebSocketStream<TcpStream>,
    ) -> anyhow::Result<()> {
        let Some(mut slot) = self.inner.lock().await.chans.remove(&chan_id) else {
            anyhow::bail!("unknown or expired channel");
        };
        let (mut sink, mut stream) = ws.split();
        loop {
            tokio::select! {
                message = stream.next() => {
                    match message {
                        Some(Ok(message @ (Message::Text(_) | Message::Binary(_)))) => {
                            if slot.outbound.send(message).is_err() {
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
                message = slot.inbound.recv() => {
                    let Some(message) = message else { break };
                    sink.send(message).await?;
                }
            }
        }
        Ok(())
    }

    async fn take_pair(&self, code: &str) -> anyhow::Result<String> {
        let mut inner = self.inner.lock().await;
        let Some(offer) = inner.pairs.get_mut(code) else {
            anyhow::bail!("unknown or expired pair code");
        };
        if offer.expires_at <= Instant::now() {
            inner.pairs.remove(code);
            anyhow::bail!("unknown or expired pair code");
        }
        offer.attempts += 1;
        if offer.attempts > PAIR_MAX_ATTEMPTS {
            inner.pairs.remove(code);
            anyhow::bail!("pair code locked");
        }
        let host_id = offer.host_id.clone();
        if inner
            .hosts
            .get(&host_id)
            .is_none_or(|slot| slot.tx.is_closed())
        {
            anyhow::bail!("host {host_id} is offline");
        }
        inner.pairs.remove(code);
        Ok(host_id)
    }
}

enum PhoneKind {
    Pair(String),
    Host(String),
}

impl Default for Relay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_pair_phone_and_chan_paths() {
        assert_eq!(
            parse_path("/host/machine_01abc"),
            Some(ParsedPath {
                kind: PathKind::Host,
                id: "machine_01abc".into(),
            })
        );
        assert_eq!(
            parse_path("/pair/12345678?x=1"),
            Some(ParsedPath {
                kind: PathKind::Pair,
                id: "12345678".into(),
            })
        );
        assert_eq!(
            parse_path("/phone/machine_01abc/"),
            Some(ParsedPath {
                kind: PathKind::Phone,
                id: "machine_01abc".into(),
            })
        );
        assert_eq!(
            parse_path("/chan/chan_7"),
            Some(ParsedPath {
                kind: PathKind::Chan,
                id: "chan_7".into(),
            })
        );
        assert!(parse_path("/host/").is_none());
        assert!(parse_path("/other/x").is_none());
        assert!(parse_path("/host/bad/id").is_none());
    }
}
