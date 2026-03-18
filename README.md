# ClawLink

ClawLink is a minimal, secure, Rust-only IM connection gateway. It does not run agents, LLMs, plugins, tools, routines, or memory. It only normalizes official IM channels and forwards messages through an OpenClaw-compatible WebSocket operator interface.

## Scope

- In scope: connection layer, protocol adaptation, authenticated WebSocket gateway, webhook ingress, channel egress abstraction.
- Out of scope: AI logic, tool execution, web UI, plugin runtime, multi-tenant ACL platform, personal WeChat reverse-engineering.

## Current MVP Skeleton

- OpenClaw operator subset over WebSocket:
	- handshake: `challenge -> connect`
	- events: `chat.message`
	- methods: `chat.send`, `sessions_send`
- Security baseline:
	- bind must be loopback or tailscale range
	- token authentication (>=64 chars)
	- nonce anti-replay for connect
	- optional Ed25519 device signature verification
	- per-connection rate limit (governor)
	- payload size and JSON depth limit
	- JSON structured logs (tracing)
- Channel abstraction:
	- unified `ChannelAdapter` trait
	- QQ / WeCom / DingTalk / Feishu placeholders for official API integration
- Test baseline:
	- per-channel unit tests
	- end-to-end webhook-to-WS echo integration test

## Quick Start

### 1) Prepare config

```bash
cp config.toml.example config.toml
```

Edit `config.toml`:
- set a random token with length >= 64
- configure cert and key path
- enable channels you need

### 2) Generate local certs (mkcert example)

```bash
mkcert -install
mkdir -p certs
mkcert -cert-file certs/clawlink-cert.pem -key-file certs/clawlink-key.pem localhost 127.0.0.1
```

### 3) Run

```bash
cargo run -- --config config.toml
```

Health check:

```bash
curl -k https://127.0.0.1:9443/healthz
```

### 4) Verify with sample WS operator client

```bash
python3 -m pip install websockets
python3 examples/ws_client.py
```

## Config Example

See `config.toml.example`.

Security-critical options:

- `gateway.bind`: must be `127.0.0.1` / `::1` or tailscale range
- `gateway.require_wss`: should stay `true` in production
- `security.require_ed25519`: enable for device-level identity hardening
- `security.max_message_bytes`: enforced upper limit, must be <= 1MB

## Protocol Subset

Client first frame:

```json
{
	"op": "connect",
	"token": "<64+ random token>",
	"role": "operator",
	"device_id": "my-agent",
	"nonce": "<nonce from challenge>",
	"signature": "<optional ed25519 signature in base64>"
}
```

Server push message event:

```json
{
	"op": "chat.message",
	"session_id": "webhook:dingtalk",
	"channel_id": "dingtalk",
	"text": "hello"
}
```

Operator send:

```json
{
	"op": "chat.send",
	"session_id": "session-1",
	"channel_id": "dingtalk",
	"text": "reply from agent",
	"media": [],
	"at": [],
	"revoke": false
}
```

## Channel Integration Plan

Current implementation uses `NoopChannel` placeholders to validate protocol and runtime behavior.

To wire official APIs, implement `ChannelAdapter::send` per channel:

- `qq`: official bot platform WS long connection ingress + HTTP send (implemented)
- `wecom`: robot long-link WS ingress + WS command send
- `dingtalk`: webhook ingress + HTTP send
- `feishu`: webhook ingress + HTTP send

### QQ target routing

QQ send needs a concrete target OpenID. Use one of these session IDs when sending from operator:

- `qq:private:<user_openid>`
- `qq:group:<group_openid>`

If `channels.qq.bot_token` is set, ClawLink uses `QQBot <app_id>.<bot_token>` authorization.
If `bot_token` is empty, ClawLink requests app access tokens via `channels.qq.auth_url` using `app_id + app_secret`.

QQ inbound is now consumed from the official gateway WebSocket long connection (heartbeat + reconnect loop), then normalized to `chat.message`.

## Tests

Run all tests:

```bash
cargo test
```

The integration test `tests/echo_ws.rs` validates:
- `challenge -> connect`
- webhook inbound to `chat.message` push over WS

## Deployment

- Linux systemd unit: `deploy/systemd/clawlink.service`
- macOS launchd plist: `deploy/launchd/com.clawlink.gateway.plist`
- Tailscale hardening notes: `deploy/tailscale.md`

## Next Milestones

1. Implement real official API clients for DingTalk and Feishu first.
2. Add WeCom long-connection channel.
3. Add QQ official platform adapter.
4. Harden audit fields and redact secrets in logs.
5. Optimize release profile for binary size and startup latency.
