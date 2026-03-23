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
    init_rustls_provider();
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

    if let Some(bridge_cfg) = &state.config.bridge {
        if bridge_cfg.enabled {
            spawn_bridge(bridge_cfg.clone());
        }
    }

    run(state).await
}

fn spawn_bridge(cfg: AppConfigBridge) {
    tokio::spawn(async move {
        // Build provider using bridge lib logic
        let request_timeout = std::time::Duration::from_secs(cfg.request_timeout_secs.max(1));
        let provider = match cfg.provider.as_str() {
            "copilot_cli_pool" => {
                let p_cfg = clawbridge::CopilotCliConfig {
                    bin: cfg.copilot_bin.into(),
                    model: cfg.copilot_model,
                    config_dir: Some(cfg.copilot_config_dir),
                    session_mode: cfg.session_mode,
                    request_timeout,
                };
                let pool = clawbridge::CopilotCliPool::new(
                    cfg.copilot_pool_size.max(1),
                    cfg.copilot_worker_queue.max(1),
                    p_cfg,
                );
                clawbridge::Provider::CopilotCliPool(std::sync::Arc::new(pool))
            }
            "mock" => clawbridge::Provider::Mock,
            _ => {
                tracing::error!("embedded bridge only supports copilot_cli_pool or mock currently");
                return;
            }
        };

        let state = std::sync::Arc::new(clawbridge::AppState { provider });
        let app = ax_router::new()
            .route("/healthz", ax_get(clawbridge::healthz))
            .route("/v1/respond", ax_post(clawbridge::respond))
            .with_state(state);

        let listener = match tokio::net::TcpListener::bind(&cfg.bind).await {
            Ok(l) => l,
            Err(err) => {
                tracing::error!(bind = %cfg.bind, error = %err, "bridge failed to bind");
                return;
            }
        };

        tracing::info!(bind = %cfg.bind, "embedded clawbridge listening");
        if let Err(err) = axum::serve(listener, app).await {
            tracing::error!(error = %err, "embedded bridge exited with error");
        }
    });
}

use axum::{Router as ax_router, routing::{get as ax_get, post as ax_post}};
use clawlink::config::BridgeConfig as AppConfigBridge;

fn init_rustls_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
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
