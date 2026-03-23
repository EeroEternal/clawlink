use std::sync::Arc;
use clawbridge::{Cli, build_provider, AppState, healthz, respond};
use clap::Parser;
use axum::{Router, routing::{get, post}};
use tracing::{info, error};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    clawbridge::init_logging(&cli.log_level);

    let provider = match build_provider(&cli) {
        Ok(p) => p,
        Err(err) => {
            eprintln!("invalid bridge config: {err}");
            std::process::exit(2);
        }
    };

    let state = Arc::new(AppState { provider });
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/respond", post(respond))
        .with_state(state);

    let listener = match tokio::net::TcpListener::bind(&cli.bind).await {
        Ok(l) => l,
        Err(err) => {
            eprintln!("failed to bind {}: {err}", cli.bind);
            std::process::exit(2);
        }
    };

    info!(bind = %cli.bind, "clawbridge listening");
    if let Err(err) = axum::serve(listener, app).await {
        error!(error = %err, "bridge server exited with error");
        std::process::exit(1);
    }
}
