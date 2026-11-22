# Contributing

## Setup

```sh
git clone <repo> remote-desktop-agents
cd remote-desktop-agents
./scripts/setup.sh          # data/, token, .env
cargo build --release
set -a; source .env; set +a
./target/release/remot-agent
```

`./scripts/setup.sh` creates a private `data/` with a random token and copies
`.env.example` → `.env`. Don't commit `data/` or `.env`.

## Code rules

- No hardcoded hostnames, tunnel URLs, tokens, or personal paths.
- New configuration must be an env var documented in `.env.example` and
  [configuration.md](configuration.md).
- Every filesystem endpoint must check `ALLOWED_ROOTS`.
- The service worker must not cache `/api/*`, `/ws/*`, or screen frames.
- Run `cargo fmt` and `cargo clippy` before a PR.

## Client conventions

- `public/` is served as-is. No bundler, no build step.
- One function, one responsibility. Avoid abstractions you don't need yet.
- Don't add new `setInterval` calls without reason; battery is a real constraint.
- Every socket must have a single owner; avoid duplicate connections.

## Layout

```
src/main.rs       HTTP/WS routes, auth, file & project API
src/rpc.rs        omp RPC bridge
src/capture.rs    capture loop (idle-aware)
src/input.rs      enigo worker
src/projects.rs   project library
src/sysmon.rs     system sampler
src/terminal.rs   PTY + tmux
src/auth.rs       HMAC cookies, rate limit
src/pairing.rs    pairing codes, client keys
src/config.rs     env + settings
public/           PWA shell
scripts/          setup, token rotation, tunnel, packaging
docs/             documentation
```

## Testing changes

Minimum manual verification:

```sh
cargo build --release
./target/release/remot-agent
```

Then in the browser: open Remote, open the terminal, resume a session, and make
sure the terminal survives more than 30 seconds without falling back to the
picker.

For relay changes:

```sh
./scripts/relay-setup-nginx.sh root@<host> <domain>
curl -s -o /dev/null -w "%{http_code}\n" https://<domain>/api/status
# 401 JSON {"ok":false,"error":"no_device"} = correct
# 200 text/html = broken routing
```

## Commit

- One commit, one reason.
- State the symptom being fixed, not just the files changed.
- Don't mix refactors with bug fixes.
