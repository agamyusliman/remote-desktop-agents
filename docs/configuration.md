# Configuration

Three sources of settings, in order of precedence:

1. `settings.json` in the data directory (written by the Setup UI)
2. environment variables
3. built-in defaults

Data directory:

- installed app: `~/Library/Application Support/Remote Agents/`
- developer mode: `./data/` (override with `REMOTE_DATA`)

## Environment variables

| Variable | Default | Meaning |
|---|---|---|
| `HOST` | `0.0.0.0` | bind address |
| `PORT` | `8788` | port |
| `OMP_CWD` | `~/Documents/Ai` | omp's starting folder |
| `ALLOWED_ROOTS` | `OMP_CWD` | roots that may be accessed, separated by `:` |
| `OMP_BIN` | `/opt/homebrew/bin/omp` | omp binary |
| `TMUX_BIN` | `/opt/homebrew/bin/tmux` | tmux binary |
| `CAPTURE_FPS` | `5` | frame cap per second |
| `JPEG_QUALITY` | `62` | 20–95 |
| `CAPTURE_RESOLUTION` | `720p` | `720p` / `1080p` |
| `REMOTE_TOKEN` | random → `data/token` | long secret |
| `PUBLIC_URL` | empty | URL shown in Settings |
| `REMOTE_DATA` | `./data` | data directory |
| `SESSION_TTL_DAYS` | `365` | login cookie lifetime (1–3650) |
| `REMOT_AGENT_DESKTOP` | `0` | `1` = tray mode + QR onboarding |
| `REMOT_AGENT_TUNNEL` | `0` | autostart tunnel at login |

## Files in the data directory

| File | Contents | Mode |
|---|---|---|
| `id` | 9-digit device ID | 600 |
| `token` | long token | 600 |
| `secret` | cookie HMAC key | 600 |
| `clients.json` | paired-client allowlist | 600 |
| `projects.json` | project library | 644 |
| `settings.json` | settings from the UI | 644 |
| `relay-devices.txt` | relay mapping: `<key> <host:port>` per line | 644 |
| `tunnel-url` | public tunnel URL (optional) | 644 |

Delete `secret` to invalidate all cookies. Delete `clients.json` to invalidate
all pairings.

## Settings values from the UI

`/api/setup` returns this list; `POST /api/setup` saves it.

```
deviceName  host  port  publicUrl  captureFps  jpegQuality
resolution  ompCwd  ompBin  allowedRoots  sessionTtlDays
requirePairing  relayUrl  relayKey
```

`relayKey` is the device key on the relay (`a`, `b`, …). Clients use it to
write the `ra_device` cookie after pairing.

## Client preferences (localStorage)

Stored on the phone, not on the server:

| Key | Contents |
|---|---|
| `ra.view` | layout: `term`, `screen`, `files`, `split` |
| `ra.sidebar` | `expanded` / `collapsed` |
| `ra.prefs` | font size, terminal options |
| `ra.project` | last opened project |
| `ra.keys` | client keys per device, so tapping a card logs in without pairing again |

Because they're stored locally, a new phone or clearing site data returns to
defaults.

`ra.keys` is deliberately **not** deleted on logout: logout ends the session,
while the device key stays stored on that phone. To truly revoke access, remove
the device in the desktop app's device list — the server revokes its key.
