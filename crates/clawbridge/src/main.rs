use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

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
use tokio::sync::{Mutex, mpsc, oneshot};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "clawbridge")]
#[command(about = "Bridge service for clawrun copilot_sdk engine")]
struct Cli {
    #[arg(long, env = "CLAWBRIDGE_BIND", default_value = "127.0.0.1:8787")]
    bind: String,
    #[arg(long, env = "CLAWBRIDGE_PROVIDER", default_value = "copilot_cli_pool")]
    provider: String,
    #[arg(long, env = "CLAWBRIDGE_COPILOT_BIN", default_value = "copilot")]
    copilot_bin: String,
    #[arg(long, env = "CLAWBRIDGE_COPILOT_MODEL")]
    copilot_model: Option<String>,
    #[arg(long, env = "CLAWBRIDGE_COPILOT_CONFIG_DIR")]
    copilot_config_dir: Option<PathBuf>,
    #[arg(long, env = "CLAWBRIDGE_SESSION_MODE", default_value_t = true)]
    session_mode: bool,
    #[arg(long, env = "CLAWBRIDGE_COPILOT_POOL_SIZE", default_value_t = 2)]
    copilot_pool_size: usize,
    #[arg(long, env = "CLAWBRIDGE_COPILOT_WORKER_QUEUE", default_value_t = 64)]
    copilot_worker_queue: usize,
    #[arg(long, env = "CLAWBRIDGE_REQUEST_TIMEOUT_SECS", default_value_t = 180)]
    request_timeout_secs: u64,
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
    CopilotCliPool(Arc<CopilotCliPool>),
    CopilotCli {
        bin: PathBuf,
        model: Option<String>,
        config_dir: Option<PathBuf>,
        session_mode: bool,
        request_timeout: Duration,
    },
    Mock,
    Command { bin: PathBuf, args: Vec<String> },
}

#[derive(Debug, Clone)]
struct CopilotCliConfig {
    bin: PathBuf,
    model: Option<String>,
    config_dir: Option<PathBuf>,
    session_mode: bool,
    request_timeout: Duration,
}

#[derive(Debug)]
struct CopilotCliPool {
    workers: Vec<mpsc::Sender<WorkerTask>>,
    affinity: Mutex<HashMap<String, usize>>,
    rr: AtomicUsize,
}

#[derive(Debug)]
struct WorkerTask {
    req: BridgeRequest,
    reply_tx: oneshot::Sender<Result<String, String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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
    let request_timeout = Duration::from_secs(cli.request_timeout_secs.max(1));

    match cli.provider.as_str() {
        "copilot_cli_pool" => {
            let cfg = CopilotCliConfig {
                bin: PathBuf::from(&cli.copilot_bin),
                model: cli.copilot_model.clone(),
                config_dir: cli.copilot_config_dir.clone(),
                session_mode: cli.session_mode,
                request_timeout,
            };
            let pool = CopilotCliPool::new(
                cli.copilot_pool_size.max(1),
                cli.copilot_worker_queue.max(1),
                cfg,
            );
            Ok(Provider::CopilotCliPool(Arc::new(pool)))
        }
        "copilot_cli" => Ok(Provider::CopilotCli {
            bin: PathBuf::from(&cli.copilot_bin),
            model: cli.copilot_model.clone(),
            config_dir: cli.copilot_config_dir.clone(),
            session_mode: cli.session_mode,
            request_timeout,
        }),
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
        other => Err(format!(
            "unsupported provider '{other}', use copilot_cli_pool, copilot_cli, mock, or command"
        )),
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
        Provider::CopilotCliPool(pool) => pool.run(req.clone()).await,
        Provider::CopilotCli {
            bin,
            model,
            config_dir,
            session_mode,
            request_timeout,
        } => {
            run_copilot_cli_provider(bin, model, config_dir, *session_mode, *request_timeout, req)
                .await
        }
        Provider::Mock => Ok(format!("[{}] {}", req.agent, req.prompt)),
        Provider::Command { bin, args } => run_command_provider(bin, args, req).await,
    }
}

async fn run_copilot_cli_provider(
    bin: &PathBuf,
    model: &Option<String>,
    config_dir: &Option<PathBuf>,
    session_mode: bool,
    request_timeout: Duration,
    req: &BridgeRequest,
) -> Result<String, String> {
    let mut cmd = Command::new(bin);
    cmd.arg("-p").arg(build_copilot_prompt(req));
    cmd.arg("-s");
    cmd.arg("--allow-all-tools");
    cmd.arg("--no-color");
    cmd.arg("--stream").arg("off");

    if let Some(config_dir) = config_dir {
        cmd.arg("--config-dir").arg(config_dir);
    }

    if session_mode {
        cmd.arg("--resume").arg(map_to_copilot_session_id(req));
    }

    if let Some(model) = model {
        if !model.trim().is_empty() {
            cmd.arg("--model").arg(model);
        }
    }

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = tokio::time::timeout(request_timeout, cmd.output())
        .await
        .map_err(|_| format!("copilot cli timed out after {}s", request_timeout.as_secs()))?
        .map_err(|e| format!("failed to spawn copilot cli: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(format!(
            "copilot cli exited with status {}: {}",
            output.status,
            stderr
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|e| format!("copilot cli stdout is not utf-8: {e}"))?;

    let text = stdout.trim().to_string();
    if text.is_empty() {
        return Err("copilot cli returned empty response".to_string());
    }

    Ok(text)
}

fn build_copilot_prompt(req: &BridgeRequest) -> String {
    let mut prompt = String::new();

    if let Some(system_prompt) = &req.system_prompt {
        if !system_prompt.trim().is_empty() {
            prompt.push_str("System instruction:\n");
            prompt.push_str(system_prompt.trim());
            prompt.push_str("\n\n");
        }
    }

    prompt.push_str("You are responding for a chat bridge service.\n");
    prompt.push_str("Return only the final reply text, no markdown code fences.\n\n");
    prompt.push_str("Context:\n");
    prompt.push_str(&format!("- agent: {}\n", req.agent));
    prompt.push_str(&format!("- channel_id: {}\n", req.channel_id));
    prompt.push_str(&format!("- session_id: {}\n\n", req.session_id));
    prompt.push_str("User message:\n");
    prompt.push_str(req.prompt.trim());

    prompt
}

fn map_to_copilot_session_id(req: &BridgeRequest) -> String {
    // Keep one deterministic Copilot session per bridge session/channel/agent tuple.
    let key = format!("{}|{}|{}", req.session_id, req.channel_id, req.agent);
    Uuid::new_v5(&Uuid::NAMESPACE_URL, key.as_bytes()).to_string()
}

impl CopilotCliPool {
    fn new(worker_count: usize, queue_size: usize, cfg: CopilotCliConfig) -> Self {
        let mut workers = Vec::with_capacity(worker_count);

        for worker_id in 0..worker_count {
            let (tx, mut rx) = mpsc::channel::<WorkerTask>(queue_size);
            let cfg = cfg.clone();

            tokio::spawn(async move {
                while let Some(task) = rx.recv().await {
                    let result = run_copilot_cli_provider(
                        &cfg.bin,
                        &cfg.model,
                        &cfg.config_dir,
                        cfg.session_mode,
                        cfg.request_timeout,
                        &task.req,
                    )
                    .await;

                    if task.reply_tx.send(result).is_err() {
                        error!(worker_id, "worker response receiver dropped");
                    }
                }
            });

            workers.push(tx);
        }

        Self {
            workers,
            affinity: Mutex::new(HashMap::new()),
            rr: AtomicUsize::new(0),
        }
    }

    async fn run(&self, req: BridgeRequest) -> Result<String, String> {
        let key = map_to_copilot_session_id(&req);
        let idx = self.worker_for_key(&key).await;

        let (reply_tx, reply_rx) = oneshot::channel();
        let task = WorkerTask { req, reply_tx };

        self.workers[idx]
            .send(task)
            .await
            .map_err(|_| format!("worker {idx} channel closed"))?;

        reply_rx
            .await
            .map_err(|_| format!("worker {idx} dropped response"))?
    }

    async fn worker_for_key(&self, key: &str) -> usize {
        let mut map = self.affinity.lock().await;
        if let Some(idx) = map.get(key) {
            return *idx;
        }

        let idx = self.rr.fetch_add(1, Ordering::SeqCst) % self.workers.len();
        map.insert(key.to_string(), idx);
        idx
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

    #[test]
    fn session_id_mapping_is_stable() {
        let req = BridgeRequest {
            agent: "test-agent".to_string(),
            prompt: "hello".to_string(),
            channel_id: "qq".to_string(),
            session_id: "qq:private:u1".to_string(),
            system_prompt: None,
        };

        let s1 = map_to_copilot_session_id(&req);
        let s2 = map_to_copilot_session_id(&req);
        assert_eq!(s1, s2);
    }
}
