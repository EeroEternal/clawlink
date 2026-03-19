# Session and Worker Pool Flow

This document explains how conversation context is kept stable across the full ClawLink stack.

## Components

- ClawLink: channel gateway and normalized message bus.
- clawops: operator hub that consumes `chat.message` and sends `chat.send`.
- clawrun: routing/runtime layer that chooses an agent engine.
- clawbridge: HTTP bridge that talks to Copilot CLI.

## End-to-End Flow

1. A channel message enters ClawLink and is normalized to `chat.message`.
2. clawops receives the event over WebSocket.
3. clawops calls clawrun with `(session_id, channel_id, text)`.
4. clawrun chooses an agent and calls clawbridge `/v1/respond` for `copilot_sdk`.
5. clawbridge maps the business session key to a deterministic Copilot session id.
6. clawbridge routes the request to a sticky worker in the pool.
7. The worker calls Copilot CLI with `--resume <copilot_session_id>`.
8. The response is returned to clawops.
9. clawops sends `chat.send` back to ClawLink.

## Session Semantics

Goal: preserve multi-turn context for the same business conversation.

Business session key:

- `session_id + channel_id + agent`

Deterministic mapping:

- same input key always maps to the same `copilot_session_id`.
- implementation uses UUID v5.

Why this matters:

- request N and request N+1 for the same business session both use the same Copilot conversation history.
- no random per-request session creation.

## Worker Pool Semantics

Default provider in clawbridge is `copilot_cli_pool`.

Pool behavior:

- a fixed number of async workers is created at startup.
- each worker has a bounded queue.
- session affinity map keeps `copilot_session_id -> worker_index`.
- first request of a session is assigned once; later requests for the same session go to the same worker.

Why affinity is important:

- preserves per-session request ordering.
- avoids same-session race conditions across workers.
- keeps state handling predictable under concurrency.

## Runtime Controls

Main environment variables:

- `CLAWBRIDGE_PROVIDER` default: `copilot_cli_pool`
- `CLAWBRIDGE_COPILOT_POOL_SIZE` default: `2`
- `CLAWBRIDGE_COPILOT_WORKER_QUEUE` default: `64`
- `CLAWBRIDGE_REQUEST_TIMEOUT_SECS` default: `180`
- `CLAWBRIDGE_SESSION_MODE` default: `true`
- `CLAWBRIDGE_COPILOT_CONFIG_DIR` optional persistent Copilot state directory

## Failure and Fallback Notes

- If Copilot CLI exits non-zero, clawbridge returns an upstream error to caller.
- If a request times out, clawbridge returns a timeout error.
- clawrun can still apply template fallback behavior when bridge calls fail.

## Operational Checklist

1. Ensure Copilot CLI is installed and logged in.
2. Keep `CLAWBRIDGE_SESSION_MODE=true` for multi-turn behavior.
3. Set `CLAWBRIDGE_COPILOT_CONFIG_DIR` to persistent storage in production.
4. Tune pool size and queue based on latency and throughput.
5. Monitor error rate and timeout rate per worker.
