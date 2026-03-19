use std::time::Duration;

use clawrun::{ClawRun, InferenceRequest};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

use crate::{config::AppConfig, protocol::{ClientMessage, OutboundMessage, ServerMessage}};

#[derive(Debug, Clone)]
pub struct OperatorHub {
    cfg: AppConfig,
}

impl OperatorHub {
    pub fn new(cfg: AppConfig) -> Self {
        Self { cfg }
    }

    pub async fn run_once(&self) -> Result<(), String> {
        let runner = ClawRun::new(self.cfg.clawrun.clone())?;

        let (ws, _) = tokio_tungstenite::connect_async(&self.cfg.gateway.url)
            .await
            .map_err(|e| format!("failed to connect to clawlink: {e}"))?;

        let (mut writer, mut reader) = ws.split();

        let challenge_raw = reader
            .next()
            .await
            .ok_or_else(|| "clawlink closed before challenge".to_string())?
            .map_err(|e| format!("failed to read challenge: {e}"))?;

        let challenge_msg = read_server_message(challenge_raw)?;
        let nonce = match challenge_msg {
            ServerMessage::Challenge { nonce, .. } => nonce,
            other => {
                return Err(format!(
                    "expected challenge as first frame, got {other:?}"
                ));
            }
        };

        let connect = ClientMessage::Connect {
            token: self.cfg.gateway.token.clone(),
            role: "operator".to_string(),
            device_id: self.cfg.operator.device_id.clone(),
            nonce,
            signature: None,
        };
        send_client_message(&mut writer, &connect).await?;

        let connect_ack_raw = reader
            .next()
            .await
            .ok_or_else(|| "clawlink closed before connect.ok".to_string())?
            .map_err(|e| format!("failed to read connect ack: {e}"))?;
        let ack = read_server_message(connect_ack_raw)?;

        match ack {
            ServerMessage::ConnectOk { session_id } => {
                info!(%session_id, "operator connected");
            }
            ServerMessage::Error { code, message } => {
                return Err(format!("connect rejected: code={code}, message={message}"));
            }
            other => {
                return Err(format!("expected connect.ok frame, got {other:?}"));
            }
        }

        let mut ping_tick = tokio::time::interval(Duration::from_secs(
            self.cfg.operator.ping_interval_secs.max(5),
        ));

        loop {
            tokio::select! {
                _ = ping_tick.tick() => {
                    send_client_message(&mut writer, &ClientMessage::Ping).await?;
                }
                maybe_frame = reader.next() => {
                    let frame = maybe_frame
                        .ok_or_else(|| "clawlink closed websocket".to_string())?
                        .map_err(|e| format!("failed to read websocket frame: {e}"))?;

                    match read_server_message(frame) {
                        Ok(ServerMessage::ChatMessage {
                            session_id,
                            channel_id,
                            text,
                            quote,
                            ..
                        }) => {
                            let Some(input_text) = text else {
                                continue;
                            };

                            let reply = runner
                                .generate_reply(
                                    &self.cfg.agents,
                                    &InferenceRequest {
                                        session_id: session_id.clone(),
                                        channel_id: channel_id.clone(),
                                        text: input_text,
                                    },
                                )
                                .await?;

                            let out = OutboundMessage {
                                session_id,
                                channel_id,
                                text: Some(reply.output_text),
                                media: Vec::new(),
                                quote,
                                at: Vec::new(),
                                revoke: false,
                            };

                            send_client_message(&mut writer, &ClientMessage::ChatSend(out)).await?;
                        }
                        Ok(ServerMessage::Pong) => {}
                        Ok(ServerMessage::Error { code, message }) => {
                            warn!(%code, %message, "gateway returned protocol error");
                        }
                        Ok(_) => {}
                        Err(err) => {
                            warn!(error = %err, "ignored non-protocol frame");
                        }
                    }
                }
            }
        }
    }
}

fn read_server_message(frame: Message) -> Result<ServerMessage, String> {
    match frame {
        Message::Text(raw) => {
            serde_json::from_str::<ServerMessage>(&raw)
                .map_err(|e| format!("failed to decode server message: {e}; raw={raw}"))
        }
        Message::Close(close) => match close {
            Some(frame) => Err(format!(
                "websocket closed by peer: code={}, reason={} ",
                frame.code, frame.reason
            )),
            None => Err("websocket closed by peer".to_string()),
        },
        other => Err(format!("unexpected websocket frame: {other:?}")),
    }
}

async fn send_client_message(
    writer: &mut futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    msg: &ClientMessage,
) -> Result<(), String> {
    let raw = serde_json::to_string(msg)
        .map_err(|e| format!("failed to encode client message: {e}"))?;

    writer
        .send(Message::Text(raw.into()))
        .await
        .map_err(|e| format!("failed to send websocket frame: {e}"))
}

#[cfg(test)]
mod tests {
    use super::read_server_message;

    #[test]
    fn parse_server_message_text() {
        let raw = tokio_tungstenite::tungstenite::Message::Text(
            r#"{"op":"pong"}"#.to_string().into(),
        );
        let parsed = read_server_message(raw).unwrap();
        assert!(matches!(parsed, crate::protocol::ServerMessage::Pong));
    }
}
