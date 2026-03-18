# Tailscale Deployment Notes

ClawLink should bind only to loopback or a Tailscale address.

## 1) Find your Tailscale IP

```bash
tailscale ip -4
```

Set [gateway].bind in config.toml to the returned address, for example:

```toml
[gateway]
bind = "100.101.102.103:9443"
```

## 2) Lock down ACL

Use a restrictive ACL policy so only your trusted machines can reach port 9443.

## 3) TLS certificate strategy

For internal-only Tailscale traffic, you can:
- Use mkcert and distribute your local CA.
- Use Tailscale certs via `tailscale cert` if your setup supports HTTPS name routing.

## 4) Verify exposure

Run on another node:

```bash
nc -vz 100.101.102.103 9443
```

Expected: open only from ACL-allowed devices.
