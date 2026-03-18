#!/usr/bin/env python3
"""Minimal ClawLink operator client for smoke testing."""

import asyncio
import json
import ssl

import websockets

URL = "wss://127.0.0.1:9443/gateway/ws"
TOKEN = "REPLACE_WITH_THE_SAME_64_PLUS_CHAR_TOKEN"
DEVICE_ID = "local-test-device"


async def main() -> None:
    ssl_ctx = ssl.create_default_context()
    # Local self-signed cert testing only.
    ssl_ctx.check_hostname = False
    ssl_ctx.verify_mode = ssl.CERT_NONE

    async with websockets.connect(URL, ssl=ssl_ctx) as ws:
        first = json.loads(await ws.recv())
        print("challenge:", first)

        await ws.send(
            json.dumps(
                {
                    "op": "connect",
                    "token": TOKEN,
                    "role": "operator",
                    "device_id": DEVICE_ID,
                    "nonce": first["nonce"],
                }
            )
        )

        print("connect ack:", await ws.recv())

        await ws.send(
            json.dumps(
                {
                    "op": "chat.send",
                    "session_id": "manual-test",
                    "channel_id": "dingtalk",
                    "text": "hello from python client",
                    "media": [],
                    "at": [],
                    "revoke": False,
                }
            )
        )

        while True:
            print("event:", await ws.recv())


if __name__ == "__main__":
    asyncio.run(main())
