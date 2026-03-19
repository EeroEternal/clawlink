use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentEngine {
    Template,
    CopilotSdk,
}

impl Default for AgentEngine {
    fn default() -> Self {
        Self::Template
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentSpec {
    pub name: String,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default = "default_reply_template")]
    pub reply_template: String,
    #[serde(default)]
    pub engine: AgentEngine,
}

impl Default for AgentSpec {
    fn default() -> Self {
        Self {
            name: "default-agent".to_string(),
            channels: Vec::new(),
            keywords: Vec::new(),
            reply_template: default_reply_template(),
            engine: AgentEngine::Template,
        }
    }
}

#[derive(Debug, Clone)]
pub struct InferenceRequest {
    pub session_id: String,
    pub channel_id: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct InferenceResult {
    pub agent_name: String,
    pub output_text: String,
}

fn default_reply_template() -> String {
    "[{agent}] {text}".to_string()
}
