use std::{collections::HashMap, fs, net::IpAddr, path::Path};

use serde::Deserialize;

use crate::error::{ClawError, Result};

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub channels: ChannelsConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&raw)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        if self.gateway.token.len() < 64 {
            return Err(ClawError::InvalidConfig(
                "gateway.token must be >= 64 characters".to_string(),
            ));
        }

        let addr = self
            .gateway
            .bind
            .parse::<std::net::SocketAddr>()
            .map_err(|e| ClawError::InvalidConfig(format!("invalid gateway.bind address: {e}")))?;

        if !is_local_or_tailscale(addr.ip()) {
            return Err(ClawError::InvalidConfig(
                "gateway.bind must be loopback or tailscale range".to_string(),
            ));
        }

        if self.gateway.require_wss {
            if self.gateway.tls_cert_path.is_empty() || self.gateway.tls_key_path.is_empty() {
                return Err(ClawError::InvalidConfig(
                    "tls_cert_path and tls_key_path are required when require_wss=true".to_string(),
                ));
            }
        }

        if self.security.max_message_bytes > 1_048_576 {
            return Err(ClawError::InvalidConfig(
                "security.max_message_bytes must be <= 1048576".to_string(),
            ));
        }

        Ok(())
    }
}

fn is_local_or_tailscale(ip: IpAddr) -> bool {
    if ip.is_loopback() {
        return true;
    }

    match ip {
        IpAddr::V4(v4) => {
            let oct = v4.octets();
            // Tailscale CGNAT range: 100.64.0.0/10
            oct[0] == 100 && (64..=127).contains(&oct[1])
        }
        IpAddr::V6(v6) => {
            // Accept ULA for tailscale v6 routes.
            let seg0 = v6.segments()[0];
            (seg0 & 0xfe00) == 0xfc00
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    pub token: String,
    #[serde(default = "default_true")]
    pub require_wss: bool,
    #[serde(default)]
    pub tls_cert_path: String,
    #[serde(default)]
    pub tls_key_path: String,
    #[serde(default)]
    pub device_public_keys: HashMap<String, String>,
}

fn default_bind() -> String {
    "127.0.0.1:9443".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_rate_limit")]
    pub rate_limit_per_sec: u32,
    #[serde(default = "default_max_message")]
    pub max_message_bytes: usize,
    #[serde(default = "default_max_depth")]
    pub max_json_depth: usize,
    #[serde(default)]
    pub require_ed25519: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            rate_limit_per_sec: default_rate_limit(),
            max_message_bytes: default_max_message(),
            max_json_depth: default_max_depth(),
            require_ed25519: false,
        }
    }
}

fn default_rate_limit() -> u32 {
    20
}

fn default_max_message() -> usize {
    1_048_576
}

fn default_max_depth() -> usize {
    64
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ChannelsConfig {
    #[serde(default)]
    pub qq: QqChannelConfig,
    #[serde(default)]
    pub wecom: ChannelToggleConfig,
    #[serde(default)]
    pub dingtalk: ChannelToggleConfig,
    #[serde(default)]
    pub feishu: ChannelToggleConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QqChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub app_id: String,
    #[serde(default)]
    pub app_secret: String,
    #[serde(default)]
    pub bot_token: String,
    #[serde(default = "default_qq_auth_url")]
    pub auth_url: String,
    #[serde(default = "default_qq_api_base")]
    pub api_base: String,
    #[serde(default)]
    pub gateway_url: Option<String>,
    #[serde(default = "default_true")]
    pub ws_enabled: bool,
    #[serde(default = "default_qq_ws_intents")]
    pub ws_intents: u64,
    #[serde(default = "default_qq_reconnect_seconds")]
    pub ws_reconnect_seconds: u64,
    #[serde(default)]
    pub endpoint: Option<String>,
}

impl Default for QqChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            app_id: String::new(),
            app_secret: String::new(),
            bot_token: String::new(),
            auth_url: default_qq_auth_url(),
            api_base: default_qq_api_base(),
            gateway_url: None,
            ws_enabled: true,
            ws_intents: default_qq_ws_intents(),
            ws_reconnect_seconds: default_qq_reconnect_seconds(),
            endpoint: None,
        }
    }
}

fn default_qq_auth_url() -> String {
    "https://bots.qq.com/app/getAppAccessToken".to_string()
}

fn default_qq_api_base() -> String {
    "https://api.sgroup.qq.com".to_string()
}

fn default_qq_ws_intents() -> u64 {
    // GROUP_AND_C2C_EVENT
    1 << 30
}

fn default_qq_reconnect_seconds() -> u64 {
    3
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ChannelToggleConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: Option<String>,
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

fn default_true() -> bool {
    true
}
