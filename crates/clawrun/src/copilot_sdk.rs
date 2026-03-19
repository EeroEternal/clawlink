use std::time::Duration;

use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

use crate::{config::CopilotSdkConfig, types::InferenceRequest};

#[derive(Debug, Clone)]
pub struct CopilotSdkAgent {
    cfg: CopilotSdkConfig,
    client: reqwest::Client,
}

impl CopilotSdkAgent {
    pub fn new(cfg: CopilotSdkConfig) -> Result<Self, String> {
        let mut headers = HeaderMap::new();

        if let Some(token) = &cfg.bearer_token {
            let value = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| format!("invalid copilot bearer_token: {e}"))?;
            headers.insert(AUTHORIZATION, value);
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(cfg.timeout_secs.max(1)))
            .build()
            .map_err(|e| format!("failed to build copilot sdk http client: {e}"))?;

        Ok(Self { cfg, client })
    }

    pub async fn run(&self, agent_name: &str, req: &InferenceRequest) -> Result<String, String> {
        let payload = CopilotBridgeRequest {
            agent: agent_name.to_string(),
            prompt: req.text.clone(),
            channel_id: req.channel_id.clone(),
            session_id: req.session_id.clone(),
            system_prompt: if self.cfg.system_prompt.is_empty() {
                None
            } else {
                Some(self.cfg.system_prompt.clone())
            },
        };

        let rsp = self
            .client
            .post(&self.cfg.endpoint)
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("copilot sdk bridge request failed: {e}"))?;

        if !rsp.status().is_success() {
            let status = rsp.status();
            let body = rsp.text().await.unwrap_or_default();
            return Err(format!(
                "copilot sdk bridge error status={status}, body={body}"
            ));
        }

        let data: CopilotBridgeResponse = rsp
            .json()
            .await
            .map_err(|e| format!("copilot sdk bridge response decode failed: {e}"))?;

        if data.text.trim().is_empty() {
            return Err("copilot sdk bridge returned empty text".to_string());
        }

        Ok(data.text)
    }
}

#[derive(Debug, Serialize)]
struct CopilotBridgeRequest {
    agent: String,
    prompt: String,
    channel_id: String,
    session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CopilotBridgeResponse {
    text: String,
}
