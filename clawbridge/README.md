# clawbridge

clawbridge is an HTTP bridge service for clawrun.

It exposes:

- `GET /healthz`
- `POST /v1/respond`

`clawrun` calls this service when an agent uses engine `copilot_sdk`.

## Quick Start

Run in mock mode (for end-to-end smoke test):

```bash
cargo run --manifest-path clawbridge/Cargo.toml -- --provider mock --bind 127.0.0.1:8787
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

- `mock` (default): returns `[agent] prompt` for fast workflow validation.
- `command`: delegates to an external command.

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
