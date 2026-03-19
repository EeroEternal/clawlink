# clawbridge

clawbridge is an HTTP bridge service for clawrun.

It exposes:

- `GET /healthz`
- `POST /v1/respond`

`clawrun` calls this service when an agent uses engine `copilot_sdk`.

## Quick Start

Run in mock mode (for end-to-end smoke test):

```bash
cargo run --manifest-path crates/clawbridge/Cargo.toml -- --provider mock --bind 127.0.0.1:8787
```

Run with GitHub Copilot CLI resident worker pool (default provider):

```bash
cargo run --manifest-path crates/clawbridge/Cargo.toml -- --bind 127.0.0.1:8787
```

Optional model override:

```bash
CLAWBRIDGE_COPILOT_MODEL=gpt-5.3-codex \
cargo run --manifest-path crates/clawbridge/Cargo.toml --
```

Test endpoint:

```bash
curl -s http://127.0.0.1:8787/v1/respond \
  -H 'content-type: application/json' \
  -d '{
    "agent": "github-copilot-sdk",
    "prompt": "hello",
    "channel_id": "qq",
    "session_id": "qq:private:u1"
  }'
```

## Provider Modes

- `copilot_cli_pool` (default): keeps a resident async worker pool and routes each session to a sticky worker.
- `copilot_cli`: direct mode without the pool, still spawns `copilot` per request.
- `mock`: returns `[agent] prompt` for fast workflow validation.
- `command`: delegates to an external command.

`copilot_cli_pool` and `copilot_cli` require:

- Copilot CLI installed and available in PATH
- successful login state (`copilot login`)

By default, session mode is enabled (`CLAWBRIDGE_SESSION_MODE=true`):

- clawbridge maps each incoming `session_id + channel_id + agent` to a deterministic Copilot session UUID.
- requests are executed with `--resume <uuid>`, so multi-turn context is retained by Copilot CLI.
- in `copilot_cli_pool`, the same mapped session is routed to the same worker for ordered processing.

Pool tuning:

- `CLAWBRIDGE_COPILOT_POOL_SIZE` (default: `2`)
- `CLAWBRIDGE_COPILOT_WORKER_QUEUE` (default: `64`)
- `CLAWBRIDGE_REQUEST_TIMEOUT_SECS` (default: `180`)

Optional persistent CLI state directory:

```bash
CLAWBRIDGE_COPILOT_CONFIG_DIR=/data/copilot \
cargo run -p clawbridge
```

### command mode contract

Configure:

- `CLAWBRIDGE_PROVIDER=command`
- `CLAWBRIDGE_CMD=/path/to/provider`
- `CLAWBRIDGE_CMD_ARG=...` (repeatable)

Behavior:

- `clawbridge` writes request JSON to provider stdin.
- provider writes reply text to stdout.
- non-zero exit code is treated as error.

This mode is where you can plug official GitHub Copilot SDK implementations (Python/Node service wrappers).
