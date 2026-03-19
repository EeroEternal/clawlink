use std::{path::PathBuf, process::Stdio, sync::Arc};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "clawbridge")]
#[command(about = "Bridge service for clawrun copilot_sdk engine")]
struct Cli {
    #[arg(long, env = "CLAWBRIDGE_BIND", default_value = "127.0.0.1:8787")]
    bind: String,
    #[arg(long, env = "CLAWBRIDGE_PROVIDER", default_value = "mock")]
    provider: String,
    #[arg(long, env = "CLAWBRIDGE_CMD")]
    cmd: Option<String>,
    #[arg(long, env = "CLAWBRIDGE_CMD_ARG")]
    cmd_arg: Vec<String>,
    #[arg(long, env = "CLAWBRIDGE_LOG_LEVEL", default_value = "info")]
    log_level: String,
}

#[derive(Debug, Clone)]
struct AppState {
    provider: Provider,
}

#[derive(Debug, Clone)]
enum Provider {
    Mock,
    Command { bin: PathBuf, args: Vec<String> },
}

#[derive(Debug, Deserialize, Serialize)]
struct BridgeRequest {
    agent: String,
    prompt: String,
    channel_id: String,
    session_id: String,
    #[serde(default)]
    system_prompt: Option<String>,
}

#[derive(Debug, Serialize)]
struct BridgeResponse {
    text: String,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    init_logging(&cli.log_level);

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

fn init_logging(level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .with_current_span(true)
        .with_span_list(false)
        .init();
}

fn build_provider(cli: &Cli) -> Result<Provider, String> {
    match cli.provider.as_str() {
        "mock" => Ok(Provider::Mock),
        "command" => {
            let Some(bin) = &cli.cmd else {
                return Err("CLAWBRIDGE_CMD is required when provider=command".to_string());
            };
            Ok(Provider::Command {
                bin: PathBuf::from(bin),
                args: cli.cmd_arg.clone(),
            })
        }
        other => Err(format!("unsupported provider '{other}', use mock or command")),
    }
}

async fn healthz() -> &'static str {
    "ok"
}

async fn respond(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BridgeRequest>,
) -> Result<Json<BridgeResponse>, (StatusCode, Json<ErrorResponse>)> {
    let text = run_provider(&state.provider, &req).await.map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse { error: err }),
        )
    })?;

    Ok(Json(BridgeResponse { text }))
}

async fn run_provider(provider: &Provider, req: &BridgeRequest) -> Result<String, String> {
    match provider {
        Provider::Mock => Ok(format!("[{}] {}", req.agent, req.prompt)),
        Provider::Command { bin, args } => run_command_provider(bin, args, req).await,
    }
}

async fn run_command_provider(bin: &PathBuf, args: &[String], req: &BridgeRequest) -> Result<String, String> {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn command provider: {e}"))?;

    let payload = serde_json::to_vec(req)
        .map_err(|e| format!("failed to encode bridge request payload: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&payload)
            .await
            .map_err(|e| format!("failed to write provider stdin: {e}"))?;
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| format!("failed waiting provider output: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(format!(
            "provider exited with status {}: {}",
            output.status,
            stderr
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|e| format!("provider stdout is not utf-8: {e}"))?;

    let text = stdout.trim().to_string();
    if text.is_empty() {
        return Err("provider returned empty response".to_string());
    }

    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_provider_returns_text() {
        let req = BridgeRequest {
            agent: "test-agent".to_string(),
            prompt: "hello".to_string(),
            channel_id: "qq".to_string(),
            session_id: "s1".to_string(),
            system_prompt: None,
        };

        let text = run_provider(&Provider::Mock, &req).await.unwrap();
        assert_eq!(text, "[test-agent] hello");
    }
}
