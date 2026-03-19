use std::{path::PathBuf, time::Duration};

use clap::Parser;
use config::AppConfig;
use hub::OperatorHub;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

mod config;
mod hub;
mod protocol;

#[derive(Debug, Parser)]
#[command(name = "clawops")]
#[command(about = "Operator-hub service for ClawLink")]
struct Cli {
    #[arg(long, env = "CLAWOPS_CONFIG", default_value = "clawops.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let cfg = match AppConfig::load(&cli.config) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("failed to load config: {err}");
            std::process::exit(2);
        }
    };

    init_logging(&cfg.logging.level);

    info!(
        config_path = %cli.config.display(),
        gateway_url = %cfg.gateway.url,
        agents = cfg.agents.len(),
        "clawops starting"
    );

    let hub = OperatorHub::new(cfg.clone());

    loop {
        if let Err(err) = hub.run_once().await {
            error!(error = %err, "operator-hub disconnected");
        }

        warn!(
            reconnect_after_secs = cfg.operator.reconnect_secs,
            "reconnecting to clawlink"
        );
        tokio::time::sleep(Duration::from_secs(cfg.operator.reconnect_secs.max(1))).await;
    }
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
