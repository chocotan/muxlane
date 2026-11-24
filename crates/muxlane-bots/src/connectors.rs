//! Webhook HTTP ingress (GitHub/GitLab/CI/Feishu/Slack/DingTalk), HMAC-SHA256 verification,
//! outbound IM reply dispatchers (Telegram/Slack/Discord/Feishu/DingTalk), and Telegram polling.
use crate::BotManager;
use hmac::{Hmac, Mac};
use muxlane_core::bot::{BotTrigger, ImConnectorConfig, ImPlatform};
use sha2::Sha256;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

const MAX_WEBHOOK_BYTES: usize = 1024 * 1024;

pub fn verify_webhook_hmac_sha256(secret: &str, signature_header: &str, body: &[u8]) -> bool {
    let clean_sig = signature_header
        .trim()
        .strip_prefix("sha256=")
        .unwrap_or(signature_header.trim());
    let Ok(expected_bytes) = hex_decode(clean_sig) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    let computed = mac.finalize().into_bytes();
    if computed.len() != expected_bytes.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in computed.iter().zip(expected_bytes.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

fn hex_decode(raw: &str) -> Result<Vec<u8>, ()> {
    if !raw.len().is_multiple_of(2) {
        return Err(());
    }
    let mut out = Vec::with_capacity(raw.len() / 2);
    let bytes = raw.as_bytes();
    for i in (0..bytes.len()).step_by(2) {
        let s = std::str::from_utf8(&bytes[i..i + 2]).map_err(|_| ())?;
        let val = u8::from_str_radix(s, 16).map_err(|_| ())?;
        out.push(val);
    }
    Ok(out)
}

pub async fn send_im_reply(
    client: &reqwest::Client,
    connector: &ImConnectorConfig,
    text: &str,
) -> anyhow::Result<()> {
    let clipped: String = text.chars().take(3800).collect();
    match connector.platform {
        ImPlatform::Telegram => {
            let Some(token) = connector
                .bot_token
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return Ok(());
            };
            let Some(chat_id) = connector
                .channel_or_chat_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return Ok(());
            };
            let url = format!("https://api.telegram.org/bot{token}/sendMessage");
            let mut body = serde_json::json!({
                "chat_id": chat_id,
                "text": clipped,
            });
            if let Ok(num) = chat_id.parse::<i64>() {
                body["chat_id"] = serde_json::json!(num);
            }
            client
                .post(url)
                .json(&body)
                .send()
                .await?
                .error_for_status()?;
        }
        ImPlatform::Slack => {
            if let Some(url) = connector
                .webhook_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                client
                    .post(url)
                    .json(&serde_json::json!({ "text": clipped }))
                    .send()
                    .await?
                    .error_for_status()?;
            } else if let (Some(token), Some(chan)) = (
                connector
                    .bot_token
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty()),
                connector
                    .channel_or_chat_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty()),
            ) {
                client
                    .post("https://slack.com/api/chat.postMessage")
                    .bearer_auth(token)
                    .json(&serde_json::json!({ "channel": chan, "text": clipped }))
                    .send()
                    .await?
                    .error_for_status()?;
            }
        }
        ImPlatform::Discord => {
            if let Some(url) = connector
                .webhook_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let discord_text: String = clipped.chars().take(1950).collect();
                client
                    .post(url)
                    .json(&serde_json::json!({ "content": discord_text }))
                    .send()
                    .await?
                    .error_for_status()?;
            }
        }
        ImPlatform::Feishu => {
            if let Some(url) = connector
                .webhook_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                client
                    .post(url)
                    .json(&serde_json::json!({
                        "msg_type": "text",
                        "content": { "text": clipped }
                    }))
                    .send()
                    .await?
                    .error_for_status()?;
            }
        }
        ImPlatform::DingTalk => {
            if let Some(url) = connector
                .webhook_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                client
                    .post(url)
                    .json(&serde_json::json!({
                        "msgtype": "text",
                        "text": { "content": clipped }
                    }))
                    .send()
                    .await?
                    .error_for_status()?;
            }
        }
        ImPlatform::GenericWebhook => {
            if let Some(url) = connector
                .webhook_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                client
                    .post(url)
                    .json(&serde_json::json!({ "text": clipped }))
                    .send()
                    .await?
                    .error_for_status()?;
            }
        }
    }
    Ok(())
}

pub async fn start_webhook_http_server(
    bind_addr: &str,
    manager: Arc<BotManager>,
) -> anyhow::Result<SocketAddr> {
    let listener = TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    tokio::spawn(async move {
        while let Ok((stream, _peer)) = listener.accept().await {
            let mgr = Arc::clone(&manager);
            tokio::spawn(async move {
                if let Err(err) = handle_http_conn(stream, mgr).await {
                    tracing::debug!(%err, "bot webhook connection error");
                }
            });
        }
    });
    Ok(local_addr)
}

async fn handle_http_conn(stream: TcpStream, manager: Arc<BotManager>) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await? == 0 {
        return Ok(());
    }
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return write_http_response(reader.get_mut(), 400, r#"{"error":"bad_request"}"#).await;
    }
    let method = parts[0];
    let path = parts[1].split('?').next().unwrap_or(parts[1]);

    let mut headers = HashMap::new();
    let mut content_len = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim().to_string();
            if key == "content-length" {
                content_len = val.parse().unwrap_or(0);
            }
            headers.insert(key, val);
        }
    }

    if content_len > MAX_WEBHOOK_BYTES {
        return write_http_response(reader.get_mut(), 413, r#"{"error":"payload_too_large"}"#)
            .await;
    }
    let mut body = vec![0u8; content_len];
    if content_len > 0 {
        reader.read_exact(&mut body).await?;
    }
    if method != "POST" {
        return write_http_response(reader.get_mut(), 405, r#"{"error":"method_not_allowed"}"#)
            .await;
    }

    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match segments.as_slice() {
        ["hooks", bot_id] => {
            handle_generic_webhook(reader.get_mut(), &manager, bot_id, &headers, &body).await
        }
        ["im", platform, bot_id] => {
            handle_im_webhook(
                reader.get_mut(),
                &manager,
                platform,
                bot_id,
                &headers,
                &body,
            )
            .await
        }
        _ => write_http_response(reader.get_mut(), 404, r#"{"error":"not_found"}"#).await,
    }
}

async fn handle_generic_webhook(
    stream: &mut TcpStream,
    manager: &Arc<BotManager>,
    bot_id: &str,
    headers: &HashMap<String, String>,
    body: &[u8],
) -> anyhow::Result<()> {
    let Some(bot) = manager.get_bot(bot_id).await else {
        return write_http_response(stream, 404, r#"{"error":"unknown_bot"}"#).await;
    };
    let secret = bot.triggers.iter().find_map(|t| match t {
        BotTrigger::Webhook { secret } => Some(secret.clone()),
        _ => None,
    });
    let Some(secret_opt) = secret else {
        return write_http_response(stream, 404, r#"{"error":"webhook_not_configured"}"#).await;
    };
    if let Some(expected_secret) = secret_opt.as_deref().filter(|s| !s.is_empty()) {
        let sig_ok = headers
            .get("x-hub-signature-256")
            .is_some_and(|sig| verify_webhook_hmac_sha256(expected_secret, sig, body));
        let token_ok = headers
            .get("x-muxlane-secret")
            .is_some_and(|s| s == expected_secret)
            || headers.get("authorization").is_some_and(|a| {
                a.strip_prefix("Bearer ")
                    .is_some_and(|t| t == expected_secret)
            });
        if !sig_ok && !token_ok {
            return write_http_response(stream, 401, r#"{"error":"unauthorized"}"#).await;
        }
    }
    let payload_text = String::from_utf8_lossy(body).into_owned();
    let event_name = headers
        .get("x-github-event")
        .map(|ev| format!("webhook:github:{ev}"))
        .unwrap_or_else(|| "webhook".into());
    match manager
        .trigger_bot(bot_id, event_name, Some(payload_text), None)
        .await
    {
        Ok(run) => {
            let json = serde_json::json!({ "ok": true, "run_id": run.id }).to_string();
            write_http_response(stream, 202, &json).await
        }
        Err(err) => {
            let json = serde_json::json!({ "error": err.to_string() }).to_string();
            write_http_response(stream, 409, &json).await
        }
    }
}

async fn handle_im_webhook(
    stream: &mut TcpStream,
    manager: &Arc<BotManager>,
    platform: &str,
    bot_id: &str,
    headers: &HashMap<String, String>,
    body: &[u8],
) -> anyhow::Result<()> {
    let Some(bot) = manager.get_bot(bot_id).await else {
        return write_http_response(stream, 404, r#"{"error":"unknown_bot"}"#).await;
    };
    let parsed_json: serde_json::Value =
        serde_json::from_slice(body).unwrap_or(serde_json::Value::Null);

    // Handle Slack / Feishu challenge handshakes before triggering a run.
    if let Some(challenge) = parsed_json.get("challenge").and_then(|v| v.as_str()) {
        let json = serde_json::json!({ "challenge": challenge }).to_string();
        return write_http_response(stream, 200, &json).await;
    }

    let Some((connector, allow_from)) = bot.triggers.iter().find_map(|t| match t {
        BotTrigger::Im {
            connector,
            allow_from,
        } if platform_matches(platform, connector.platform) => {
            Some((connector.clone(), allow_from.clone()))
        }
        _ => None,
    }) else {
        return write_http_response(stream, 404, r#"{"error":"im_trigger_not_configured"}"#).await;
    };

    if let Some(secret) = connector
        .signing_secret
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        let token_ok = headers.get("x-muxlane-secret").is_some_and(|s| s == secret)
            || headers.get("x-slack-signature").is_some()
            || parsed_json.get("token").and_then(|t| t.as_str()) == Some(secret);
        if !token_ok {
            return write_http_response(stream, 401, r#"{"error":"unauthorized"}"#).await;
        }
    }

    let sender = parsed_json
        .pointer("/event/user")
        .or_else(|| parsed_json.pointer("/event/sender/sender_id/open_id"))
        .or_else(|| parsed_json.get("senderId"))
        .or_else(|| parsed_json.get("user_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    if !allow_from.is_empty()
        && !allow_from
            .iter()
            .any(|allowed| allowed == "*" || allowed == &sender)
    {
        return write_http_response(stream, 403, r#"{"error":"sender_not_allowed"}"#).await;
    }

    let text = parsed_json
        .pointer("/event/text")
        .or_else(|| parsed_json.pointer("/event/message/content"))
        .or_else(|| parsed_json.pointer("/text/content"))
        .or_else(|| parsed_json.get("text"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| String::from_utf8_lossy(body).into_owned());

    match manager
        .trigger_bot(
            bot_id,
            format!("im:{platform}:{sender}"),
            Some(text),
            Some(connector),
        )
        .await
    {
        Ok(run) => {
            let json = serde_json::json!({ "ok": true, "run_id": run.id }).to_string();
            write_http_response(stream, 202, &json).await
        }
        Err(err) => {
            let json = serde_json::json!({ "error": err.to_string() }).to_string();
            write_http_response(stream, 409, &json).await
        }
    }
}

fn platform_matches(slug: &str, platform: ImPlatform) -> bool {
    matches!(
        (slug.to_ascii_lowercase().as_str(), platform),
        ("slack", ImPlatform::Slack)
            | ("feishu", ImPlatform::Feishu)
            | ("dingtalk", ImPlatform::DingTalk)
            | ("discord", ImPlatform::Discord)
            | ("telegram", ImPlatform::Telegram)
            | ("webhook", ImPlatform::GenericWebhook)
    )
}

async fn write_http_response(
    stream: &mut TcpStream,
    status: u16,
    json_body: &str,
) -> anyhow::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        _ => "Internal Server Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        json_body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(json_body.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

/// Polls Telegram Bot `getUpdates` for enabled Bots with Telegram `bot_token`.
pub async fn run_telegram_poller(manager: Arc<BotManager>) {
    let mut offsets: HashMap<String, i64> = HashMap::new();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .unwrap_or_default();

    loop {
        let bots = manager.list_bots().await;
        let mut any_polled = false;
        for bot in bots {
            if !bot.enabled {
                continue;
            }
            for trig in &bot.triggers {
                let BotTrigger::Im {
                    connector,
                    allow_from,
                } = trig
                else {
                    continue;
                };
                if connector.platform != ImPlatform::Telegram {
                    continue;
                }
                let Some(token) = connector
                    .bot_token
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                any_polled = true;
                let offset = offsets.get(&bot.id).copied().unwrap_or(0);
                let url = format!(
                    "https://api.telegram.org/bot{token}/getUpdates?offset={offset}&timeout=5"
                );
                let Ok(resp) = client.get(url).send().await else {
                    continue;
                };
                let Ok(json) = resp.json::<serde_json::Value>().await else {
                    continue;
                };
                let Some(updates) = json.get("result").and_then(|v| v.as_array()) else {
                    continue;
                };
                for upd in updates {
                    if let Some(uid) = upd.get("update_id").and_then(|v| v.as_i64()) {
                        offsets.insert(bot.id.clone(), uid + 1);
                    }
                    let Some(msg) = upd.get("message") else {
                        continue;
                    };
                    let Some(text) = msg.get("text").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    let chat_id = msg
                        .pointer("/chat/id")
                        .and_then(|v| v.as_i64())
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    let username = msg
                        .pointer("/from/username")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if !allow_from.is_empty()
                        && !allow_from.iter().any(|a| {
                            a == "*" || a == &chat_id || (!username.is_empty() && a == &username)
                        })
                    {
                        continue;
                    }
                    let mut reply_conn = connector.clone();
                    if !chat_id.is_empty() {
                        reply_conn.channel_or_chat_id = Some(chat_id.clone());
                    }
                    let mgr = Arc::clone(&manager);
                    let bid = bot.id.clone();
                    let body = text.to_string();
                    tokio::spawn(async move {
                        let _ = mgr
                            .trigger_bot(
                                &bid,
                                format!("im:telegram:{chat_id}"),
                                Some(body),
                                Some(reply_conn),
                            )
                            .await;
                    });
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(if any_polled {
            2
        } else {
            5
        }))
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_hmac_verification_accepts_valid_and_rejects_tamper() {
        let secret = "super-secret";
        let body = br#"{"action":"opened","number":42}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let hex: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let header = format!("sha256={hex}");
        assert!(verify_webhook_hmac_sha256(secret, &header, body));
        assert!(!verify_webhook_hmac_sha256(secret, &header, b"tampered"));
        assert!(!verify_webhook_hmac_sha256(secret, "sha256=deadbeef", body));
    }
}
