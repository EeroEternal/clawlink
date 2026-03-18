use std::sync::Arc;

use axum::Router;
use clawlink::{
    channels::{ChannelAdapter, ChannelKind, NoopChannel},
    config::{AppConfig, ChannelsConfig, GatewayConfig, LoggingConfig, SecurityConfig},
    ws::{AppState, router},
};
use futures::{SinkExt, StreamExt};
use tokio::{net::TcpListener, time::Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::test]
async fn e2e_webhook_echo_to_operator() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let config = Arc::new(AppConfig {
        gateway: GatewayConfig {
            bind: addr.to_string(),
            token: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            require_wss: false,
            tls_cert_path: String::new(),
            tls_key_path: String::new(),
            device_public_keys: Default::default(),
            allow_public_bind: true,
        },
        security: SecurityConfig {
            rate_limit_per_sec: 50,
            max_message_bytes: 1_048_576,
            max_json_depth: 64,
            require_ed25519: false,
        },
        channels: ChannelsConfig::default(),
        logging: LoggingConfig {
            level: "debug".to_string(),
        },
    });

    let channels: Arc<Vec<Arc<dyn ChannelAdapter>>> = Arc::new(vec![Arc::new(NoopChannel::new(
        "dingtalk",
        ChannelKind::Dingtalk,
    ))]);

    let state = AppState::new(config, channels);
    let app: Router = router(state);

    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let url = format!("ws://{}/gateway/ws", addr);
    let (mut ws, _) = connect_async(url).await.unwrap();

    let challenge_raw = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let challenge_text = match challenge_raw {
        Message::Text(t) => t.to_string(),
        other => panic!("unexpected challenge frame: {other:?}"),
    };
    let challenge: serde_json::Value = serde_json::from_str(&challenge_text).unwrap();
    let nonce = challenge["nonce"].as_str().unwrap().to_string();

    let connect = serde_json::json!({
        "op": "connect",
        "token": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "role": "operator",
        "device_id": "test-device",
        "nonce": nonce
    });
    ws.send(Message::Text(connect.to_string().into()))
        .await
        .unwrap();

    let ack = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ack_text = match ack {
        Message::Text(t) => t.to_string(),
        other => panic!("unexpected ack frame: {other:?}"),
    };
    let ack_json: serde_json::Value = serde_json::from_str(&ack_text).unwrap();
    assert_eq!(ack_json["op"], "connect.ok");

    let client = reqwest::Client::new();
    let endpoint = format!("http://{}/webhook/dingtalk", addr);
    let res = client
        .post(endpoint)
        .json(&serde_json::json!({"text": "hello from webhook"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 202);

    let event_raw = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let event_text = match event_raw {
        Message::Text(t) => t.to_string(),
        other => panic!("unexpected event frame: {other:?}"),
    };
    let event_json: serde_json::Value = serde_json::from_str(&event_text).unwrap();
    assert_eq!(event_json["op"], "chat.message");
    assert_eq!(event_json["channel_id"], "dingtalk");
    assert_eq!(event_json["text"], "hello from webhook");

    server.abort();
}
