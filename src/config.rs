use std::{collections::HashMap, env, fs, io::ErrorKind, net::IpAddr, path::Path};

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
        let cfg = match fs::read_to_string(path) {
            Ok(raw) => toml::from_str(&raw)?,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                tracing::warn!(
                    config_path = %path.display(),
                    "config file not found, falling back to environment variables"
                );
                Self::from_env()?
            }
            Err(e) => return Err(e.into()),
        };

        cfg.validate()?;
        Ok(cfg)
    }

    pub fn from_env() -> Result<Self> {
        let token = env::var("CLAWLINK_GATEWAY_TOKEN").map_err(|_| {
            ClawError::InvalidConfig(
                "CLAWLINK_GATEWAY_TOKEN is required when config.toml is missing".to_string(),
            )
        })?;

        let port = env::var("PORT").unwrap_or_else(|_| "9443".to_string());
        let bind = env::var("CLAWLINK_GATEWAY_BIND").unwrap_or_else(|_| format!("0.0.0.0:{port}"));

        let require_wss = env_bool("CLAWLINK_GATEWAY_REQUIRE_WSS", false);
        let allow_public_bind = env_bool("CLAWLINK_ALLOW_PUBLIC_BIND", true);
        let logging_level = env::var("CLAWLINK_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());

        let qq_enabled = env_bool("CLAWLINK_CHANNEL_QQ_ENABLED", false);
        let qq_cfg = QqChannelConfig {
            enabled: qq_enabled,
            app_id: env::var("CLAWLINK_QQ_APP_ID").unwrap_or_default(),
            app_secret: env::var("CLAWLINK_QQ_APP_SECRET").unwrap_or_default(),
            bot_token: env::var("CLAWLINK_QQ_BOT_TOKEN").unwrap_or_default(),
            auth_url: env::var("CLAWLINK_QQ_AUTH_URL").unwrap_or_else(|_| default_qq_auth_url()),
            api_base: env::var("CLAWLINK_QQ_API_BASE").unwrap_or_else(|_| default_qq_api_base()),
            gateway_url: env::var("CLAWLINK_QQ_GATEWAY_URL").ok(),
            ws_enabled: env_bool("CLAWLINK_QQ_WS_ENABLED", true),
            ws_intents: env_u64("CLAWLINK_QQ_WS_INTENTS", default_qq_ws_intents()),
            ws_reconnect_seconds: env_u64(
                "CLAWLINK_QQ_WS_RECONNECT_SECONDS",
                default_qq_reconnect_seconds(),
            ),
            endpoint: env::var("CLAWLINK_QQ_ENDPOINT").ok(),
        };

        Ok(Self {
            gateway: GatewayConfig {
                bind,
                token,
                require_wss,
                tls_cert_path: env::var("CLAWLINK_TLS_CERT_PATH").unwrap_or_default(),
                tls_key_path: env::var("CLAWLINK_TLS_KEY_PATH").unwrap_or_default(),
                allow_public_bind,
                device_public_keys: HashMap::new(),
            },
            security: SecurityConfig {
                rate_limit_per_sec: env_u32("CLAWLINK_RATE_LIMIT_PER_SEC", default_rate_limit()),
                max_message_bytes: env_usize("CLAWLINK_MAX_MESSAGE_BYTES", default_max_message()),
                max_json_depth: env_usize("CLAWLINK_MAX_JSON_DEPTH", default_max_depth()),
                require_ed25519: env_bool("CLAWLINK_REQUIRE_ED25519", false),
            },
            channels: ChannelsConfig {
                qq: qq_cfg,
                wecom: ChannelToggleConfig {
                    enabled: env_bool("CLAWLINK_CHANNEL_WECOM_ENABLED", false),
                    endpoint: env::var("CLAWLINK_WECOM_ENDPOINT").ok(),
                },
                dingtalk: ChannelToggleConfig {
                    enabled: env_bool("CLAWLINK_CHANNEL_DINGTALK_ENABLED", false),
                    endpoint: env::var("CLAWLINK_DINGTALK_ENDPOINT").ok(),
                },
                feishu: ChannelToggleConfig {
                    enabled: env_bool("CLAWLINK_CHANNEL_FEISHU_ENABLED", false),
                    endpoint: env::var("CLAWLINK_FEISHU_ENDPOINT").ok(),
                },
            },
            logging: LoggingConfig {
                level: logging_level,
            },
        })
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

        if !self.gateway.allow_public_bind && !is_local_or_tailscale(addr.ip()) {
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
    pub allow_public_bind: bool,
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

fn env_bool(name: &str, default: bool) -> bool {
    match env::var(name) {
        Ok(v) => matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => default,
    }
}

fn env_u32(name: &str, default: u32) -> u32 {
    env::var(name)
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}
