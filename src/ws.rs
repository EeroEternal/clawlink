use std::{num::NonZeroU32, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use axum_server::tls_rustls::RustlsConfig;
use futures::{SinkExt, StreamExt};
use governor::{Quota, RateLimiter, clock::DefaultClock, state::InMemoryState, state::NotKeyed};
use nonzero_ext::nonzero;
use tokio::{net::TcpListener, sync::broadcast};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    channels::ChannelAdapter,
    config::AppConfig,
    error::{ClawError, Result},
    protocol::{ClientMessage, OutboundMessage, ServerMessage},
    security::{NonceStore, json_depth, random_challenge, verify_ed25519_signature},
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub channels: Arc<Vec<Arc<dyn ChannelAdapter>>>,
    pub events: broadcast::Sender<ServerMessage>,
    pub nonce_store: Arc<NonceStore>,
}

impl AppState {
    pub fn new(config: Arc<AppConfig>, channels: Arc<Vec<Arc<dyn ChannelAdapter>>>) -> Self {
        let (events, _rx) = broadcast::channel(512);
        Self {
            config,
            channels,
            events,
            nonce_store: Arc::new(NonceStore::new()),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/gateway/ws", get(ws_upgrade))
        .route("/webhook/{channel}", post(webhook_inbound))
        .with_state(state)
}

pub async fn run(state: AppState) -> Result<()> {
    let addr = state
        .config
        .gateway
        .bind
        .parse::<std::net::SocketAddr>()
        .map_err(|e| ClawError::InvalidConfig(format!("invalid bind addr: {e}")))?;

    let app = router(state.clone());

    if state.config.gateway.require_wss {
        let tls = RustlsConfig::from_pem_file(
            state.config.gateway.tls_cert_path.clone(),
            state.config.gateway.tls_key_path.clone(),
        )
        .await
        .map_err(|e| ClawError::InvalidConfig(format!("failed to load TLS cert/key: {e}")))?;

        info!(bind = %addr, "starting WSS gateway");
        axum_server::bind_rustls(addr, tls)
            .serve(app.into_make_service())
            .await
            .map_err(ClawError::Io)?;
    } else {
        warn!("WSS disabled; use only for local development");
        let listener = TcpListener::bind(addr).await?;
        info!(bind = %addr, "starting WS gateway");
        axum::serve(listener, app).await.map_err(ClawError::Io)?;
    }

    Ok(())
}

async fn healthz() -> &'static str {
    "ok"
}

async fn webhook_inbound(
    Path(channel): Path<String>,
    State(state): State<AppState>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let (session_id, text) = if channel == "qq" {
        parse_qq_webhook_payload(&payload).unwrap_or_else(|| {
            (
                "webhook:qq".to_string(),
                payload
                    .get("d")
                    .and_then(|d| d.get("content"))
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned)
                    .or_else(|| Some(payload.to_string())),
            )
        })
    } else {
        (
            format!("webhook:{channel}"),
            payload
                .get("text")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned)
                .or_else(|| {
                    payload
                        .get("content")
                        .and_then(|v| v.as_str())
                        .map(ToOwned::to_owned)
                })
                .or_else(|| Some(payload.to_string())),
        )
    };

    let event = ServerMessage::ChatMessage {
        session_id,
        channel_id: channel,
        text,
        media: Vec::new(),
        quote: None,
        at: Vec::new(),
    };

    if let Err(e) = state.events.send(event) {
        warn!(error = %e, "no operator connected for inbound webhook message");
    }

    (StatusCode::ACCEPTED, Json(serde_json::json!({"ok": true})))
}

fn parse_qq_webhook_payload(payload: &serde_json::Value) -> Option<(String, Option<String>)> {
    let data = payload.get("d")?;
    let text = data
        .get("content")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);

    if let Some(group_openid) = data
        .get("group_openid")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
    {
        return Some((format!("qq:group:{group_openid}"), text));
    }

    if let Some(user_openid) = data
        .get("author")
        .and_then(|v| v.get("id"))
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
    {
        return Some((format!("qq:private:{user_openid}"), text));
    }

    None
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.max_frame_size(state.config.security.max_message_bytes)
        .on_upgrade(move |socket| async move {
            if let Err(err) = handle_socket(socket, state).await {
                error!(error = %err, "websocket connection closed with error");
            }
        })
}

async fn handle_socket(socket: WebSocket, state: AppState) -> Result<()> {
    let (mut sender, mut receiver) = socket.split();

    let nonce = state.nonce_store.issue().await;
    let challenge = random_challenge();

    send_server_message(
        &mut sender,
        &ServerMessage::Challenge {
            challenge: challenge.clone(),
            nonce: nonce.clone(),
        },
    )
    .await?;

    let first = tokio::time::timeout(Duration::from_secs(15), receiver.next())
        .await
        .map_err(|_| ClawError::Auth("connect timeout".to_string()))?;

    let msg = match first {
        Some(Ok(Message::Text(txt))) => txt.to_string(),
        Some(Ok(_)) => {
            return Err(ClawError::Protocol(
                "first websocket frame must be JSON text".to_string(),
            ));
        }
        Some(Err(e)) => {
            return Err(ClawError::Protocol(format!("ws receive error: {e}")));
        }
        None => {
            return Err(ClawError::Protocol(
                "peer closed before connect".to_string(),
            ));
        }
    };

    validate_payload_limits(&msg, &state.config)?;
    let connect: ClientMessage = serde_json::from_str(&msg)?;

    let (device_id, _role) = authenticate_connect(connect, &state, &nonce, &challenge).await?;
    let session_id = Uuid::new_v4().to_string();

    info!(%session_id, %device_id, "operator authenticated");

    send_server_message(
        &mut sender,
        &ServerMessage::ConnectOk {
            session_id: session_id.clone(),
        },
    )
    .await?;

    let rate_limit = state.config.security.rate_limit_per_sec.max(1);
    let limiter: RateLimiter<NotKeyed, InMemoryState, DefaultClock> = RateLimiter::direct(
        Quota::per_second(NonZeroU32::new(rate_limit).unwrap_or(nonzero!(1u32))),
    );

    let mut event_rx = state.events.subscribe();
    let writer = tokio::spawn(async move {
        while let Ok(event) = event_rx.recv().await {
            if send_server_message(&mut sender, &event).await.is_err() {
                break;
            }
        }
    });

    while let Some(frame) = receiver.next().await {
        let frame = frame.map_err(|e| ClawError::Protocol(format!("ws read error: {e}")))?;

        match frame {
            Message::Text(raw) => {
                limiter
                    .check()
                    .map_err(|_| ClawError::Protocol("rate limit exceeded".to_string()))?;

                let raw = raw.to_string();
                validate_payload_limits(&raw, &state.config)?;
                let msg: ClientMessage = serde_json::from_str(&raw)?;

                match msg {
                    ClientMessage::Ping => {
                        let _ = state.events.send(ServerMessage::Pong);
                    }
                    ClientMessage::ChatSend(outbound) | ClientMessage::SessionsSend(outbound) => {
                        handle_outbound(outbound, &state).await?;
                    }
                    ClientMessage::Connect { .. } => {
                        return Err(ClawError::Protocol(
                            "connect can only be sent once".to_string(),
                        ));
                    }
                }
            }
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => {}
        }
    }

    writer.abort();
    Ok(())
}

async fn authenticate_connect(
    msg: ClientMessage,
    state: &AppState,
    expected_nonce: &str,
    challenge: &str,
) -> Result<(String, String)> {
    let ClientMessage::Connect {
        token,
        role,
        device_id,
        nonce,
        signature,
    } = msg
    else {
        return Err(ClawError::Auth("first message must be connect".to_string()));
    };

    if role != "operator" {
        return Err(ClawError::Auth("role must be operator".to_string()));
    }

    if token != state.config.gateway.token {
        return Err(ClawError::Auth("token mismatch".to_string()));
    }

    if nonce != expected_nonce {
        return Err(ClawError::Auth("nonce mismatch".to_string()));
    }

    let consumed = state.nonce_store.consume(&nonce).await;
    if !consumed {
        return Err(ClawError::Auth("nonce already used".to_string()));
    }

    if state.config.security.require_ed25519 {
        let pub_key = state
            .config
            .gateway
            .device_public_keys
            .get(&device_id)
            .ok_or_else(|| ClawError::Auth("device public key missing".to_string()))?;
        let signature =
            signature.ok_or_else(|| ClawError::Auth("signature required".to_string()))?;
        let payload = format!("{challenge}:{nonce}:{device_id}");
        verify_ed25519_signature(pub_key, &signature, payload.as_bytes())?;
    }

    Ok((device_id, role))
}

async fn handle_outbound(outbound: OutboundMessage, state: &AppState) -> Result<()> {
    let channel = state
        .channels
        .iter()
        .find(|c| c.channel_id() == outbound.channel_id)
        .ok_or_else(|| {
            ClawError::Channel(format!(
                "channel '{}' is not configured",
                outbound.channel_id
            ))
        })?
        .clone();

    channel.send(&outbound).await
}

fn validate_payload_limits(raw: &str, config: &AppConfig) -> Result<()> {
    if raw.len() > config.security.max_message_bytes {
        return Err(ClawError::Protocol("payload too large".to_string()));
    }
    if json_depth(raw) > config.security.max_json_depth {
        return Err(ClawError::Protocol("json depth exceeded".to_string()));
    }
    Ok(())
}

async fn send_server_message(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    msg: &ServerMessage,
) -> Result<()> {
    let raw = serde_json::to_string(msg)?;
    sender
        .send(Message::Text(raw.into()))
        .await
        .map_err(|e| ClawError::Protocol(format!("ws send error: {e}")))
}
