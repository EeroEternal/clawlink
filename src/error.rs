use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClawError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml decode error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("auth failed: {0}")]
    Auth(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("channel error: {0}")]
    Channel(String),
}

pub type Result<T> = std::result::Result<T, ClawError>;
