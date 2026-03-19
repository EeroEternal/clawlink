use std::{env, fs, io::ErrorKind, path::Path};

use clawrun::{AgentSpec, ClawRunConfig};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub gateway: GatewayConfig,
    pub operator: OperatorConfig,
    #[serde(default)]
    pub agents: Vec<AgentSpec>,
    #[serde(default)]
    pub clawrun: ClawRunConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let cfg = match fs::read_to_string(path) {
            Ok(raw) => toml::from_str::<Self>(&raw)
                .map_err(|e| format!("failed to parse config file {}: {e}", path.display()))?,
            Err(e) if e.kind() == ErrorKind::NotFound => Self::from_env()?,
            Err(e) => return Err(format!("failed to read config file {}: {e}", path.display())),
        };

        cfg.validate()?;
        Ok(cfg)
    }

    fn from_env() -> Result<Self, String> {
        let token = env::var("CLAWOPS_TOKEN")
            .map_err(|_| "CLAWOPS_TOKEN is required when config file is missing".to_string())?;

        let url = env::var("CLAWOPS_GATEWAY_URL")
            .unwrap_or_else(|_| "ws://127.0.0.1:9443/gateway/ws".to_string());

        let device_id = env::var("CLAWOPS_DEVICE_ID").unwrap_or_else(|_| "clawops-main".to_string());
        let reconnect_secs = env_u64("CLAWOPS_RECONNECT_SECS", 3);
        let ping_interval_secs = env_u64("CLAWOPS_PING_INTERVAL_SECS", 20);

        Ok(Self {
            gateway: GatewayConfig { url, token },
            operator: OperatorConfig {
                device_id,
                reconnect_secs,
                ping_interval_secs,
            },
            agents: vec![AgentSpec::default()],
            clawrun: ClawRunConfig::default(),
            logging: LoggingConfig::default(),
        })
    }

    fn validate(&self) -> Result<(), String> {
        if self.gateway.token.len() < 64 {
            return Err("gateway token must be >= 64 characters".to_string());
        }

        if !(self.gateway.url.starts_with("ws://") || self.gateway.url.starts_with("wss://")) {
            return Err("gateway.url must start with ws:// or wss://".to_string());
        }

        if self.operator.device_id.trim().is_empty() {
            return Err("operator.device_id cannot be empty".to_string());
        }

        if self.agents.is_empty() {
            return Err("at least one [[agents]] entry is required".to_string());
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    pub url: String,
    pub token: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OperatorConfig {
    #[serde(default = "default_device_id")]
    pub device_id: String,
    #[serde(default = "default_reconnect_secs")]
    pub reconnect_secs: u64,
    #[serde(default = "default_ping_interval_secs")]
    pub ping_interval_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_device_id() -> String {
    "clawops-main".to_string()
}

fn default_reconnect_secs() -> u64 {
    3
}

fn default_ping_interval_secs() -> u64 {
    20
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}
