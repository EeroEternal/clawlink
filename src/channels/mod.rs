use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::broadcast;
use tracing::info;

use crate::{config::AppConfig, error::Result, protocol::OutboundMessage};

mod qq;

pub use qq::QqChannel;
use qq::spawn_qq_gateway_task;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Qq,
    Wecom,
    Dingtalk,
    Feishu,
}

#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    fn channel_id(&self) -> &'static str;
    fn kind(&self) -> ChannelKind;
    async fn send(&self, msg: &OutboundMessage) -> Result<()>;
}

#[derive(Debug)]
pub struct NoopChannel {
    id: &'static str,
    kind: ChannelKind,
}

impl NoopChannel {
    pub fn new(id: &'static str, kind: ChannelKind) -> Self {
        Self { id, kind }
    }
}

#[async_trait]
impl ChannelAdapter for NoopChannel {
    fn channel_id(&self) -> &'static str {
        self.id
    }

    fn kind(&self) -> ChannelKind {
        self.kind
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        info!(
            channel_id = self.id,
            session_id = %msg.session_id,
            "channel send requested"
        );
        Ok(())
    }
}

pub fn build_channels(config: &AppConfig) -> Vec<Arc<dyn ChannelAdapter>> {
    let mut channels: Vec<Arc<dyn ChannelAdapter>> = Vec::new();

    if config.channels.qq.enabled {
        channels.push(Arc::new(QqChannel::new(config.channels.qq.clone())));
    }
    if config.channels.wecom.enabled {
        channels.push(Arc::new(NoopChannel::new("wecom", ChannelKind::Wecom)));
    }
    if config.channels.dingtalk.enabled {
        channels.push(Arc::new(NoopChannel::new(
            "dingtalk",
            ChannelKind::Dingtalk,
        )));
    }
    if config.channels.feishu.enabled {
        channels.push(Arc::new(NoopChannel::new("feishu", ChannelKind::Feishu)));
    }

    channels
}

pub fn start_background_tasks(
    config: &AppConfig,
    events: &broadcast::Sender<crate::protocol::ServerMessage>,
) {
    let _ = spawn_qq_gateway_task(config.channels.qq.clone(), events.clone());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::OutboundMessage;

    fn sample_msg(channel_id: &str) -> OutboundMessage {
        OutboundMessage {
            session_id: "s1".to_string(),
            channel_id: channel_id.to_string(),
            text: Some("hello".to_string()),
            media: Vec::new(),
            quote: None,
            at: Vec::new(),
            revoke: false,
        }
    }

    #[tokio::test]
    async fn qq_channel_send_ok() {
        let ch = NoopChannel::new("qq", ChannelKind::Qq);
        let res = ch.send(&sample_msg("qq")).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn wecom_channel_send_ok() {
        let ch = NoopChannel::new("wecom", ChannelKind::Wecom);
        let res = ch.send(&sample_msg("wecom")).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn dingtalk_channel_send_ok() {
        let ch = NoopChannel::new("dingtalk", ChannelKind::Dingtalk);
        let res = ch.send(&sample_msg("dingtalk")).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn feishu_channel_send_ok() {
        let ch = NoopChannel::new("feishu", ChannelKind::Feishu);
        let res = ch.send(&sample_msg("feishu")).await;
        assert!(res.is_ok());
    }
}
