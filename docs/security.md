# Security

Remote Agents gives full control of the host to anyone holding the credentials.
Treat it like a remote shell.

## Threat model

| Asset | Protection |
|---|---|
| screen & input | signed session cookie |
| terminal & chat | session cookie |
| project files | `ALLOWED_ROOTS` + canonicalization |
| client key | only on the device, mode 600 |

## Required rules

1. Bind to `127.0.0.1` and use a tunnel, or keep it on a trusted LAN.
2. **Don't** port-forward the port to the internet.
3. The token and the 9-digit ID are equivalent to a password.
4. Rotate after losing a device: `./scripts/rotate-token.sh --restart`.
5. Review `clients.json` regularly; revoke anything you don't recognize.

## What the system does

- Cookies are signed HMAC-SHA256, `HttpOnly`, `Secure`, `SameSite=Lax`.
- `secret` is generated randomly on first run, mode 600.
- Pairing codes: single use, 5 minutes, one device.
- Per-IP login rate limit (`allow_attempt`).
- File paths are canonicalized; symlinks pointing outside `ALLOWED_ROOTS` are rejected.
- The service worker never caches `/api/*` or `/ws/*`.
- Pairing codes are never sent to the relay (they live in the URL fragment).

## What is not protected

| Risk | Reality |
|---|---|
| relay sees the client key during claim | TLS terminates at the VPS |
| relay metadata | IP, time, traffic size, which Mac is active |
| one active device | other devices are disconnected, not silently rejected |
| `ra_device` can be forged | that cookie only selects a Mac, it isn't authentication |
| macOS TCC | rebuilding the app requires re-granting permissions |

## Distribution

Without a Developer ID + notarization, Gatekeeper blocks the app on other Macs:

```sh
./scripts/package-macos.sh --sign "Developer ID Application: Name (TEAMID)"
```

TCC permissions (Screen Recording, Accessibility) are tied to the binary
signature, so every rebuild asks for permission again.

## Reporting issues

Report privately, not in a public issue.
