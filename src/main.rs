use std::{path::PathBuf, sync::Arc};

use clap::Parser;
use clawlink::{
    channels::{build_channels, start_background_tasks},
    config::AppConfig,
    error::Result,
    ws::{AppState, run},
};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "clawlink")]
#[command(about = "Minimal secure IM connection gateway compatible with OpenClaw operator role")]
struct Cli {
    #[arg(long, env = "CLAWLINK_CONFIG", default_value = "config.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Arc::new(AppConfig::load(&cli.config)?);

    init_logging(&config.logging.level);
    info!(config_path = %cli.config.display(), "configuration loaded");

    let channels = Arc::new(build_channels(&config));
    info!(
        enabled_channels = channels.len(),
        "channel adapters initialized"
    );

    let state = AppState::new(config, channels);
    start_background_tasks(&state.config, &state.events);
    run(state).await
}

fn init_logging(level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .with_current_span(true)
        .with_span_list(false)
        .init();
}
