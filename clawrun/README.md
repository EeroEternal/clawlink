# clawrun

clawrun is the agent runtime library used by clawops.

It provides:

- Agent selection by channel and keyword
- Template engine for quick fallback replies
- Built-in `copilot_sdk` engine via HTTP bridge

## Why bridge mode

GitHub Copilot SDK currently has official SDKs for Node.js/TypeScript, Python, Go, .NET, and Java.
In this Rust workspace, clawrun uses a simple HTTP bridge contract so clawops can still route to a Copilot SDK-powered backend.

## Bridge API contract

Endpoint:

- POST `/v1/respond`

Request JSON:

```json
{
  "agent": "github-copilot-sdk",
  "prompt": "need refund",
  "channel_id": "qq",
  "session_id": "qq:private:openid",
  "system_prompt": "optional"
}
```

Response JSON:

```json
{
  "text": "Here is the answer from copilot sdk"
}
```

## Runtime config

`ClawRunConfig` includes:

- `copilot.endpoint` default: `http://127.0.0.1:8787/v1/respond`
- `copilot.timeout_secs` default: `30`
- `copilot.bearer_token` optional
- `copilot.system_prompt` optional

If `copilot_sdk` call fails, clawrun falls back to agent `reply_template`.
