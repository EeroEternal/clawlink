# clawops

clawops is an operator-hub service for ClawLink.

It connects to ClawLink over WebSocket as role=operator, receives normalized chat.message events, routes them through `clawrun`, and sends chat.send replies back to the original session.

## Relationship with ClawLink

- ClawLink: channel gateway (QQ/Feishu/WeCom adapters, protocol normalization, security)
- clawops: business orchestration layer (gateway session handling, reply delivery)
- clawrun: agent runtime library (selection + template/copilot_sdk engines)

ClawLink and clawops are independent processes.

## Run

1. Copy config template:

   cp clawops.toml.example clawops.toml

2. Set token to match clawlink gateway token.

3. Start clawops:

   cargo run --release -- --config clawops.toml

## Config

See clawops.toml.example.

- gateway.url: ClawLink WS endpoint, for example ws://127.0.0.1:9443/gateway/ws
- gateway.token: same token as ClawLink gateway.token
- operator.device_id: operator identity visible in ClawLink logs
- agents: routing rules by channel + keyword + engine (`template` or `copilot_sdk`)
- clawrun.copilot.endpoint: HTTP bridge endpoint connected to a backend that runs the official GitHub Copilot SDK

## Copilot SDK Flow

1. ClawLink pushes `chat.message` to clawops.
2. clawops forwards message context to clawrun.
3. clawrun selects agent based on channel + keyword.
4. If selected engine is `copilot_sdk`, clawrun calls configured bridge endpoint.
5. Bridge backend returns text from GitHub Copilot SDK runtime.
6. clawops sends `chat.send` back to ClawLink.

## Notes

This crate now delegates reply generation to clawrun and supports a bridge mode for GitHub Copilot SDK.
If bridge call fails, clawrun falls back to template reply for reliability.
