# Architecture

```
Phone (PWA)  ──HTTPS/WSS──>  nginx relay  ──tunnel──>  remot-agent (Rust)  ──>  macOS
                                                       ├─ scap      (screen)
                                                       ├─ enigo     (input)
                                                       ├─ portable-pty + tmux
                                                       └─ omp --mode rpc-ui
```

## `src/` layout

| File | Responsibility |
|---|---|
| `main.rs` | HTTP/WS routes, auth, file & project API, PWA shell |
| `auth.rs` | HMAC cookies, session allowlist, login rate limit |
| `pairing.rs` | pairing codes, client keys, revoke |
| `rpc.rs` | omp RPC bridge (chat, model, sessions, ask forms) |
| `capture.rs` | ScreenCaptureKit loop; idle-aware + identical-frame detection |
| `input.rs` | enigo worker for mouse/keyboard |
| `terminal.rs` | PTY + tmux, one tmux session per target |
| `projects.rs` | project library with allowlist |
| `sysmon.rs` | CPU/RAM/disk/network sampler |
| `config.rs` | env + `settings.json` |
| `identity.rs` | device ID, secret |

## `public/` layout

PWA shell, no build step.

| File | Contents |
|---|---|
| `index.html` | app shell, all panes |
| `app.js` | client logic (nav, terminal, screen, files, pairing) |
| `style.css` | all styling |
| `sw.js` | service worker: shell cache only |
| `pair.js`, `setup.css` | pairing/onboarding pages |
| `vendor/` | xterm, marked, DOMPurify, jsQR |

## Screen flow

1. The client opens `/ws/screen`.
2. `capture.rs` grabs a frame, encodes JPEG, sends binary.
3. The client draws to `<canvas>`. Input (mouse/keyboard) is sent as JSON
   messages on the same `/ws/screen` WebSocket, then executed by `input.rs`.
4. Capture stops when there are no viewers, and skips identical frames.

Frames are sent as binary WebSocket — not base64 in JSON — to avoid extra
encode/decode cost on the phone.

## Terminal flow

1. The client opens `/ws/term?cols=&rows=&project=&resume=`.
2. The server runs `tmux new-session -A -s <name>` in a PTY.
3. Session name:

   ```
   without resume: omp-ai
   with resume   : omp-ai-r1a2b3c4d   ← session file hash
   ```

   Resume needs its own session. If the same name is used, `-A` will *attach*
   to the already-running omp and the `--resume` argument is ignored.

4. When the connection drops, only that tmux client process is killed.

> Don't use `tmux detach-client -s <session>` to close a connection: that
> command detaches **every** client on that session, so one stale phone closing
> would also disconnect the phone in use.

## Chat flow (omp RPC)

`rpc.rs` runs `omp --mode rpc-ui` as a child process and exchanges JSON:

```
client ──ws/rpc──> rpc.rs ──stdin/stdout──> omp
```

Supported messages include `get_state`, `get_messages_page`, `send_prompt`,
model selection, and `ask`/approval forms.

## Pairing flow

See [pairing.md](pairing.md). In short:

```
desktop: /api/pair/new  → code + QR
phone  : /pair?k=a      → nginx Set-Cookie ra_device=a
phone  : /api/pair/claim→ Mac replies with client key
phone  : cookie ra_c_…  → all subsequent requests
```

## Service worker

| Request | Strategy |
|---|---|
| navigation (`/`) | network-first, fallback to shell |
| `app.js`, `style.css` | network-first |
| vendor, icons | stale-while-revalidate |
| `/api/*`, `/ws/*`, `/sw.js` | untouched |

The cache version is derived from the shell hash at deploy time, so every code
change opens a new cache and deletes the old one.

## Battery

- capture stops when `screenClients == 0`
- identical frames aren't resent
- capture pauses while the Remote tab is inactive
- the terminal isn't resized without a real size change
