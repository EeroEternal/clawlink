use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaRef {
    pub url: String,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub session_id: String,
    pub channel_id: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub media: Vec<MediaRef>,
    #[serde(default)]
    pub quote: Option<String>,
    #[serde(default)]
    pub at: Vec<String>,
    #[serde(default)]
    pub revoke: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum ClientMessage {
    #[serde(rename = "connect")]
    Connect {
        token: String,
        role: String,
        device_id: String,
        nonce: String,
        #[serde(default)]
        signature: Option<String>,
    },
    #[serde(rename = "chat.send")]
    ChatSend(OutboundMessage),
    #[serde(rename = "ping")]
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum ServerMessage {
    #[serde(rename = "challenge")]
    Challenge { challenge: String, nonce: String },
    #[serde(rename = "connect.ok")]
    ConnectOk { session_id: String },
    #[serde(rename = "chat.message")]
    ChatMessage {
        session_id: String,
        channel_id: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        media: Vec<MediaRef>,
        #[serde(default)]
        quote: Option<String>,
        #[serde(default)]
        at: Vec<String>,
    },
    #[serde(rename = "pong")]
    Pong,
    #[serde(rename = "error")]
    Error { code: String, message: String },
}
