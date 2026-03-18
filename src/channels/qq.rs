use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, warn};

use crate::{
    config::QqChannelConfig,
    error::{ClawError, Result},
    protocol::{OutboundMessage, ServerMessage},
};

use super::{ChannelAdapter, ChannelKind};

#[derive(Debug, Clone)]
pub struct QqChannel {
    cfg: QqChannelConfig,
    client: reqwest::Client,
    token_cache: std::sync::Arc<tokio::sync::Mutex<Option<CachedAccessToken>>>,
}

#[derive(Debug, Clone)]
struct CachedAccessToken {
    token: String,
    expires_at: Instant,
}

impl QqChannel {
    pub fn new(cfg: QqChannelConfig) -> Self {
        Self {
            cfg,
            client: reqwest::Client::new(),
            token_cache: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    async fn authorization_header(&self) -> Result<String> {
        if !self.cfg.bot_token.is_empty() {
            if self.cfg.app_id.is_empty() {
                return Err(ClawError::InvalidConfig(
                    "channels.qq.app_id is required when bot_token is set".to_string(),
                ));
            }
            return Ok(format!("QQBot {}.{}", self.cfg.app_id, self.cfg.bot_token));
        }

        if self.cfg.app_id.is_empty() || self.cfg.app_secret.is_empty() {
            return Err(ClawError::InvalidConfig(
                "channels.qq requires bot_token or app_id+app_secret".to_string(),
            ));
        }

        let token = self.get_app_access_token().await?;
        Ok(format!("Bearer {token}"))
    }

    async fn get_app_access_token(&self) -> Result<String> {
        {
            let guard = self.token_cache.lock().await;
            if let Some(cached) = &*guard {
                if cached.expires_at > Instant::now() + Duration::from_secs(60) {
                    return Ok(cached.token.clone());
                }
            }
        }

        let req = AccessTokenRequest {
            app_id: self.cfg.app_id.clone(),
            client_secret: self.cfg.app_secret.clone(),
        };

        let rsp = self
            .client
            .post(&self.cfg.auth_url)
            .json(&req)
            .send()
            .await
            .map_err(|e| ClawError::Channel(format!("qq access token request failed: {e}")))?;

        if !rsp.status().is_success() {
            let status = rsp.status();
            let body = rsp.text().await.unwrap_or_default();
            return Err(ClawError::Channel(format!(
                "qq access token request failed with status {status}: {body}"
            )));
        }

        let payload: AccessTokenResponse = rsp.json().await.map_err(|e| {
            ClawError::Channel(format!("qq access token response decode failed: {e}"))
        })?;

        let token = payload.access_token;
        let expires_in = payload.expires_in.max(120);

        let mut guard = self.token_cache.lock().await;
        *guard = Some(CachedAccessToken {
            token: token.clone(),
            expires_at: Instant::now() + Duration::from_secs(expires_in),
        });

        Ok(token)
    }

    fn resolve_target(&self, msg: &OutboundMessage) -> Result<QqTarget> {
        parse_qq_target(&msg.session_id).or_else(|| parse_qq_target(&msg.channel_id)).ok_or_else(
            || {
                ClawError::Channel(
                    "qq target missing, use session_id format qq:private:<openid> or qq:group:<openid>"
                        .to_string(),
                )
            },
        )
    }

    fn build_message_body(&self, msg: &OutboundMessage) -> Result<QqSendBody> {
        let mut content = msg.text.clone().unwrap_or_default();

        if !msg.media.is_empty() {
            let links = msg
                .media
                .iter()
                .map(|m| m.url.as_str())
                .collect::<Vec<_>>()
                .join("\n");

            if !links.is_empty() {
                if !content.is_empty() {
                    content.push_str("\n");
                }
                content.push_str(&links);
            }
        }

        if content.is_empty() {
            return Err(ClawError::Channel(
                "qq send requires text or media url".to_string(),
            ));
        }

        Ok(QqSendBody {
            content,
            msg_type: 0,
            msg_id: msg.quote.clone(),
        })
    }
}

pub fn spawn_qq_gateway_task(
    cfg: QqChannelConfig,
    events: broadcast::Sender<ServerMessage>,
) -> Option<tokio::task::JoinHandle<()>> {
    if !cfg.enabled || !cfg.ws_enabled {
        return None;
    }

    if cfg.app_id.is_empty() || cfg.bot_token.is_empty() {
        warn!("qq ws gateway is enabled but app_id/bot_token is missing; skip qq ws task");
        return None;
    }

    Some(tokio::spawn(async move {
        loop {
            if let Err(err) = run_gateway_once(&cfg, &events).await {
                error!(error = %err, "qq ws gateway loop failed");
            }
            tokio::time::sleep(Duration::from_secs(cfg.ws_reconnect_seconds.max(1))).await;
        }
    }))
}

async fn run_gateway_once(
    cfg: &QqChannelConfig,
    events: &broadcast::Sender<ServerMessage>,
) -> Result<()> {
    let ws_url = resolve_gateway_ws_url(cfg).await?;
    info!(ws_url = %ws_url, "connecting to qq gateway ws");

    let (socket, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| ClawError::Channel(format!("qq ws connect failed: {e}")))?;

    let (mut writer, mut reader) = socket.split();

    let hello_frame = reader
        .next()
        .await
        .ok_or_else(|| ClawError::Channel("qq ws closed before hello".to_string()))?
        .map_err(|e| ClawError::Channel(format!("qq ws hello read failed: {e}")))?;

    let hello_text = frame_to_text(hello_frame)?;
    let hello: GatewayFrame = serde_json::from_str(&hello_text)
        .map_err(|e| ClawError::Channel(format!("qq ws hello decode failed: {e}")))?;

    if hello.op != 10 {
        return Err(ClawError::Channel(format!(
            "qq ws expected op=10 hello, got op={}",
            hello.op
        )));
    }

    let heartbeat_ms = hello
        .d
        .as_ref()
        .and_then(|d| d.get("heartbeat_interval"))
        .and_then(|v| v.as_u64())
        .unwrap_or(30_000);

    let identify = serde_json::json!({
        "op": 2,
        "d": {
            "token": format!("QQBot {}.{}", cfg.app_id, cfg.bot_token),
            "intents": cfg.ws_intents,
            "shard": [0, 1],
            "properties": {
                "$os": std::env::consts::OS,
                "$browser": "clawlink",
                "$device": "clawlink"
            }
        }
    });

    writer
        .send(Message::Text(identify.to_string().into()))
        .await
        .map_err(|e| ClawError::Channel(format!("qq ws identify send failed: {e}")))?;

    let mut heartbeat = tokio::time::interval(Duration::from_millis(heartbeat_ms.max(5_000)));
    let mut seq: Option<i64> = None;

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                let hb = serde_json::json!({"op": 1, "d": seq});
                writer
                    .send(Message::Text(hb.to_string().into()))
                    .await
                    .map_err(|e| ClawError::Channel(format!("qq ws heartbeat failed: {e}")))?;
            }
            maybe_frame = reader.next() => {
                let frame = match maybe_frame {
                    Some(Ok(frame)) => frame,
                    Some(Err(e)) => {
                        return Err(ClawError::Channel(format!("qq ws read failed: {e}")));
                    }
                    None => {
                        return Err(ClawError::Channel("qq ws closed by peer".to_string()));
                    }
                };

                match frame {
                    Message::Text(txt) => {
                        let payload = txt.to_string();
                        let gateway: GatewayFrame = serde_json::from_str(&payload).map_err(|e| {
                            ClawError::Channel(format!("qq ws frame decode failed: {e}"))
                        })?;

                        if let Some(s) = gateway.s {
                            seq = Some(s);
                        }

                        if gateway.op == 0 {
                            if let Some(event) = parse_dispatch_event(&gateway) {
                                let _ = events.send(event);
                            }
                        }
                    }
                    Message::Ping(data) => {
                        writer
                            .send(Message::Pong(data))
                            .await
                            .map_err(|e| ClawError::Channel(format!("qq ws pong send failed: {e}")))?;
                    }
                    Message::Close(_) => {
                        return Err(ClawError::Channel("qq ws closed".to_string()));
                    }
                    Message::Binary(_) | Message::Pong(_) | Message::Frame(_) => {}
                }
            }
        }
    }
}

fn frame_to_text(frame: Message) -> Result<String> {
    match frame {
        Message::Text(txt) => Ok(txt.to_string()),
        other => Err(ClawError::Channel(format!(
            "qq ws expected text frame for hello, got {other:?}"
        ))),
    }
}

async fn resolve_gateway_ws_url(cfg: &QqChannelConfig) -> Result<String> {
    if let Some(url) = &cfg.gateway_url {
        if !url.is_empty() {
            return Ok(url.clone());
        }
    }

    let endpoint = format!("{}/gateway/bot", cfg.api_base.trim_end_matches('/'));
    let auth = format!("QQBot {}.{}", cfg.app_id, cfg.bot_token);

    let client = reqwest::Client::new();
    let rsp = client
        .get(endpoint)
        .header("Authorization", auth)
        .send()
        .await
        .map_err(|e| ClawError::Channel(format!("qq gateway url request failed: {e}")))?;

    if !rsp.status().is_success() {
        let status = rsp.status();
        let body = rsp.text().await.unwrap_or_default();
        return Err(ClawError::Channel(format!(
            "qq gateway url request failed with status {status}: {body}"
        )));
    }

    let data: GatewayBotResponse = rsp
        .json()
        .await
        .map_err(|e| ClawError::Channel(format!("qq gateway url response decode failed: {e}")))?;

    Ok(data.url)
}

fn parse_dispatch_event(frame: &GatewayFrame) -> Option<ServerMessage> {
    let event_name = frame.t.as_deref()?;
    let data = frame.d.as_ref()?;

    let is_message_event = matches!(
        event_name,
        "AT_MESSAGE_CREATE" | "GROUP_AT_MESSAGE_CREATE" | "C2C_MESSAGE_CREATE" | "MESSAGE_CREATE"
    );

    if !is_message_event {
        return None;
    }

    let text = data
        .get("content")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);

    let msg_id = data
        .get("id")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);

    let at = data
        .get("mentions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let session_id = if let Some(group_openid) = data
        .get("group_openid")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
    {
        format!("qq:group:{group_openid}")
    } else if let Some(user_openid) = data
        .get("author")
        .and_then(|v| v.get("id"))
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
    {
        format!("qq:private:{user_openid}")
    } else {
        "qq:private:unknown".to_string()
    };

    Some(ServerMessage::ChatMessage {
        session_id,
        channel_id: "qq".to_string(),
        text,
        media: Vec::new(),
        quote: msg_id,
        at,
    })
}

#[async_trait]
impl ChannelAdapter for QqChannel {
    fn channel_id(&self) -> &'static str {
        "qq"
    }

    fn kind(&self) -> ChannelKind {
        ChannelKind::Qq
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        let target = self.resolve_target(msg)?;
        let body = self.build_message_body(msg)?;
        let auth = self.authorization_header().await?;

        let api_base = self.cfg.api_base.trim_end_matches('/');
        let endpoint = match target {
            QqTarget::Private(user_openid) => format!("{api_base}/v2/users/{user_openid}/messages"),
            QqTarget::Group(group_openid) => {
                format!("{api_base}/v2/groups/{group_openid}/messages")
            }
        };

        let rsp = self
            .client
            .post(endpoint)
            .header("Authorization", auth)
            .header("X-Union-Appid", &self.cfg.app_id)
            .json(&body)
            .send()
            .await
            .map_err(|e| ClawError::Channel(format!("qq send request failed: {e}")))?;

        if !rsp.status().is_success() {
            let status = rsp.status();
            let body = rsp.text().await.unwrap_or_default();
            return Err(ClawError::Channel(format!(
                "qq send failed with status {status}: {body}"
            )));
        }

        Ok(())
    }
}

#[derive(Debug, Serialize)]
struct QqSendBody {
    content: String,
    msg_type: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    msg_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct AccessTokenRequest {
    #[serde(rename = "appId")]
    app_id: String,
    #[serde(rename = "clientSecret")]
    client_secret: String,
}

#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    access_token: String,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct GatewayBotResponse {
    url: String,
}

#[derive(Debug, Deserialize)]
struct GatewayFrame {
    op: i64,
    #[serde(default)]
    s: Option<i64>,
    #[serde(default)]
    t: Option<String>,
    #[serde(default)]
    d: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum QqTarget {
    Private(String),
    Group(String),
}

fn parse_qq_target(input: &str) -> Option<QqTarget> {
    let private_prefix = "qq:private:";
    let group_prefix = "qq:group:";

    if let Some(id) = input.strip_prefix(private_prefix) {
        if !id.is_empty() {
            return Some(QqTarget::Private(id.to_string()));
        }
    }
    if let Some(id) = input.strip_prefix(group_prefix) {
        if !id.is_empty() {
            return Some(QqTarget::Group(id.to_string()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::OutboundMessage;

    fn sample_msg(session_id: &str) -> OutboundMessage {
        OutboundMessage {
            session_id: session_id.to_string(),
            channel_id: "qq".to_string(),
            text: Some("hello".to_string()),
            media: Vec::new(),
            quote: None,
            at: Vec::new(),
            revoke: false,
        }
    }

    #[test]
    fn parse_private_target_ok() {
        let parsed = parse_qq_target("qq:private:abc123");
        assert_eq!(parsed, Some(QqTarget::Private("abc123".to_string())));
    }

    #[test]
    fn parse_group_target_ok() {
        let parsed = parse_qq_target("qq:group:group001");
        assert_eq!(parsed, Some(QqTarget::Group("group001".to_string())));
    }

    #[test]
    fn build_content_with_media_links() {
        let ch = QqChannel::new(QqChannelConfig::default());
        let mut msg = sample_msg("qq:private:abc");
        msg.media = vec![
            crate::protocol::MediaRef {
                url: "https://example.com/a.png".to_string(),
                kind: Some("image".to_string()),
            },
            crate::protocol::MediaRef {
                url: "https://example.com/f.pdf".to_string(),
                kind: Some("file".to_string()),
            },
        ];

        let body = ch.build_message_body(&msg).unwrap();
        assert!(body.content.contains("https://example.com/a.png"));
        assert!(body.content.contains("https://example.com/f.pdf"));
    }

    #[test]
    fn parse_group_dispatch_event_ok() {
        let frame: GatewayFrame = serde_json::from_value(serde_json::json!({
            "op": 0,
            "s": 11,
            "t": "GROUP_AT_MESSAGE_CREATE",
            "d": {
                "id": "m001",
                "content": "hello group",
                "group_openid": "g_openid",
                "mentions": [{"id": "bot001"}]
            }
        }))
        .unwrap();

        let event = parse_dispatch_event(&frame).unwrap();
        match event {
            ServerMessage::ChatMessage {
                session_id,
                channel_id,
                text,
                quote,
                at,
                ..
            } => {
                assert_eq!(session_id, "qq:group:g_openid");
                assert_eq!(channel_id, "qq");
                assert_eq!(text.as_deref(), Some("hello group"));
                assert_eq!(quote.as_deref(), Some("m001"));
                assert_eq!(at, vec!["bot001".to_string()]);
            }
            _ => panic!("unexpected event type"),
        }
    }
}
