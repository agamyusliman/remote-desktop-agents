# Remote Agents

Self-hosted remote desktop **and** `omp` chat/terminal for phone or tablet.
Rust server + PWA. No cloud accounts, no third-party services.

Control one Mac from anywhere: view the screen, move the mouse and keyboard,
run `omp`, manage projects, and browse files — all from a phone browser
that installs as an app.

> **Public repo.** No tokens, device keys, or server addresses here.
> The real configuration lives in `data/` and `relay/devices.json`, both of
> which are `.gitignore`d. See [docs/security.md](docs/security.md) before deploying.

## Features

| Feature | Description |
|---|---|
| **Screen** | ScreenCaptureKit via `scap`, JPEG ~5 fps over WebSocket, input via `enigo` |
| **omp chat** | RPC bridge (`omp --mode rpc-ui`), model picker, session resume, ask form |
| **Terminal** | real PTY via `portable-pty` + tmux + xterm.js |
| **Sessions** | list, resume, rename, and delete omp sessions without leaving the project |
| **Projects** | allowlisted library; add by picking a folder, remove from the list |
| **Files** | tree explorer, code viewer, markdown preview |
| **System** | CPU, RAM, disk, network, top processes |
| **PWA** | installs on iOS/Android; service worker caches the shell only |
| **Battery saver** | capture stops when nobody is watching, identical frames are skipped |

## Requirements

- macOS 12.3+ (ScreenCaptureKit), Rust 1.85+
- [`omp`](https://github.com/) installed (default `/opt/homebrew/bin/omp`)
- `tmux`
- Optional: a VPS for the relay (nginx), `cloudflared` or `tailscale`

## Quick start

```sh
git clone https://github.com/agamyusliman/remote-desktop-agents.git
cd remote-desktop-agents
./scripts/setup.sh          # create data/, token, .env
$EDITOR .env                # set OMP_CWD and ALLOWED_ROOTS
cargo build --release
set -a; source .env; set +a
./target/release/remot-agent
```

Or use the desktop app:

```sh
./scripts/package-macos.sh --install   # → /Applications/Remote Agents.app
```

Then:

1. Open the app, allow **Screen Recording** and **Accessibility**, then relaunch.
2. **Connect** tab → **Create pairing code**.
3. Scan the QR from your phone, or type the code.

Full guide: [docs/getting-started.md](docs/getting-started.md).

## Access from outside your home

Never expose port `8788` to the internet. Options:

| Method | Notes |
|---|---|
| nginx relay on a VPS | one hostname for several Macs — [docs/relay.md](docs/relay.md) |
| Tailscale | `tailscale serve 8788`, private, no domain |
| Cloudflare named tunnel | `./scripts/setup-named-tunnel.sh remote.yourdomain.com` |
| Quick tunnel | `./scripts/tunnel.sh`, random URL for testing |

## Security in brief

- Session cookies are HMAC-signed (`HttpOnly`, `Secure`); pairing clients use
  a bearer key stored on the phone and in `clients.json` on the Mac.
- Every file endpoint is restricted by `ALLOWED_ROOTS`.
- The relay rejects `/api/*` without a device cookie with `401 JSON`, not a page.
- Remove a device from the list in the desktop app to cut off access.
- Rotate every credential: `./scripts/rotate-token.sh --restart`.

Threat model details: [docs/security.md](docs/security.md).

## Documentation

| Document | Contents |
|---|---|
| [docs/getting-started.md](docs/getting-started.md) | install, macOS permissions, UI flow, first steps |
| [docs/pairing.md](docs/pairing.md) | pairing flow, QR contents, cookies, revoke |
| [docs/relay.md](docs/relay.md) | one hostname for several Macs |
| [docs/access.md](docs/access.md) | token, rotation, emergency access |
| [docs/configuration.md](docs/configuration.md) | env vars, data directory contents, client preferences |
| [docs/architecture.md](docs/architecture.md) | code structure and data flow |
| [docs/api.md](docs/api.md) | HTTP endpoints, WebSocket, session format |
| [docs/security.md](docs/security.md) | threat model and limits |
| [docs/troubleshooting.md](docs/troubleshooting.md) | common symptoms and causes |
| [docs/contributing.md](docs/contributing.md) | code rules and PR flow |

## Layout

```
src/        Rust server (axum): capture, input, terminal, rpc, pairing, auth
public/     PWA: index.html, app.js, style.css, sw.js, vendor/
relay/      nginx config + example devices
scripts/    setup, packaging, tunnel, relay, token rotation
docs/       documentation
data/       runtime state — NOT committed
```

## License

MIT — see [LICENSE](LICENSE).
