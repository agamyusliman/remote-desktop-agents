# API

All endpoints are `allow()`-gated except those marked public. Auth is via an
HMAC-signed session cookie, or a paired client.

## Status & configuration

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/status` | capture, input, session, cwd, viewer |
| `GET` | `/api/system` | CPU, RAM, disk, network |
| `GET` | `/api/identity` | device name + ID |
| `GET` | `/api/ping` | lightweight probe |
| `GET` | `/api/config` | effective configuration |
| `GET` | `/api/setup` | all settings + `pairing` + `permissions` |
| `POST` | `/api/setup` | save settings |
| `POST` | `/api/setup/restart` | restart the process |
| `POST` | `/api/setup/open` | open the macOS permissions pane |
| `GET`/`POST` | `/api/setup/autostart` | status / set LaunchAgent |

## Sessions

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/login` | log in with an ID or token |
| `POST` | `/api/logout` | delete this browser session |
| `POST` | `/api/logout-all` | rotate the token, log everyone out |
| `GET` | `/api/sessions` | list omp sessions |
| `GET` | `/api/project-sessions?path=` | saved sessions for a project |
| `POST` | `/api/session-label` | rename a session (`{path,label}`) |
| `POST` | `/api/session-delete` | delete one session (`{path}`) |

## Pairing

| Method | Path | Access |
|---|---|---|
| `POST` | `/api/pair/new` | loopback / paired |
| `POST` | `/api/pair/claim` | public, requires `ra_device` |
| `GET` | `/api/pair/clients` | loopback / paired |
| `POST` | `/api/pair/revoke` | loopback / paired |
| `POST` | `/api/pair/request` | device requests approval |
| `POST` | `/api/pair/approve` | Mac approves |

## Projects & files

| Method | Path | Description |
|---|---|---|
| `GET`/`POST` | `/api/projects` | list / add project (`name` optional) |
| `DELETE` | `/api/projects/{id}` | remove from the list; the folder is untouched |
| `GET` | `/api/browse?path=` | folder browser for "Add project" |
| `GET` | `/api/fs` | list directories within `ALLOWED_ROOTS` |
| `GET` | `/api/files` | project file tree |
| `GET` | `/api/files/read` | file contents (text) |
| `GET` | `/api/files/raw` | raw contents (binary/image) |

All paths are canonicalized first; paths outside `ALLOWED_ROOTS` are rejected.

## Public pages

| Path | Contents |
|---|---|
| `/` | PWA shell (the only UI: pairing, devices, sessions) |
| `/auth?id=` / `/auth?token=` | 302 to `/` (backwards-compatible links) |
| `/pair` | 302 to `/?k=<key>`, preserving `#c=<code>` |
| `/desktop` | desktop Setup UI |
| `/desktop/qr.svg` | QR containing the ID |
| `/desktop/setup-qr.svg?c=` | pairing QR |

## WebSocket

| Path | Auth | Function |
|---|---|---|
| `/ws/screen` | cookie | binary JPEG frames |
| `/ws/term` | cookie | terminal (PTY + tmux) |
| `/ws/rpc` | cookie | omp chat |

### `/ws/term`

Query:

```
cols, rows, project, resume
```

Incoming messages (text JSON):

```json
{"type":"input","data":"ls\r"}
{"type":"resize","cols":78,"rows":37}
{"type":"scroll","dir":"up","lines":10}
```

Output is sent as binary (terminal bytes).

## Relay (nginx, not the app)

| Path | Function |
|---|---|
| `/active` | active device key |
| `/switch?key=` | switch device (same-origin) |
| `/probe/<key>` | online status |
| `/selector.html` | 302 to `/?picker=1` (the picker lives in the app) |
| `/devices.json` | device list (404 on the relay; only via `/api/devices`) |

When the `ra_device` cookie is missing, `/api/*` replies with `401 JSON`
`{"ok":false,"error":"no_device"}` — not an HTML page, so clients can recover
the cookie instead of getting stuck in the `connecting` state.

All `/api/*` responses carry `Cache-Control: no-store` (middleware in
`src/main.rs`). Without it, stale bodies can be replayed from the browser cache
and the app boots into the wrong screen.

## omp session format

`/api/project-sessions` reads `~/.omp/agent/sessions/<project>/*.jsonl`. The
first two lines determine what is shown:

```json
{"type":"title","title":"...","source":"auto"}
{"type":"session","id":"...","title":"...","titleSource":"auto"}
```

Renaming doesn't change the file name (omp uses it for resume); only the
`title` value is rewritten, with `source`/`titleSource` set to `user`.
