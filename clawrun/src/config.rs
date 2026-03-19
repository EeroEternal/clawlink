use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClawRunConfig {
    #[serde(default)]
    pub copilot: CopilotSdkConfig,
}

impl Default for ClawRunConfig {
    fn default() -> Self {
        Self {
            copilot: CopilotSdkConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CopilotSdkConfig {
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub bearer_token: Option<String>,
    #[serde(default)]
    pub system_prompt: String,
}

impl Default for CopilotSdkConfig {
    fn default() -> Self {
        Self {
            endpoint: default_endpoint(),
            timeout_secs: default_timeout_secs(),
            bearer_token: None,
            system_prompt: String::new(),
        }
    }
}

fn default_endpoint() -> String {
    "http://127.0.0.1:8787/v1/respond".to_string()
}

fn default_timeout_secs() -> u64 {
    30
}
