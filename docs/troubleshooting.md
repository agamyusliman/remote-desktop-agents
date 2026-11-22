# Troubleshooting

Symptom → cause → fix.

## Phone stuck at `connecting`

**Most common cause:** the `ra_device` cookie is missing. The relay uses that
cookie to route `/api/*`; without it, every request fails.

Check from the Mac:

```sh
curl -s -o /dev/null -w "%{http_code}\n" https://<relay>/api/status
# 401 {"ok":false,"error":"no_device"} → cookie missing
```

**Fix:** re-pair via QR, or open `/pair?k=a`. The app also restores the cookie
automatically on `401 no_device`.

---

## Pairing code says "wrong or expired" but was just created

The code is **single use** and valid for 5 minutes. If another process (test,
another tab) uses it first, the code is used up.

Create a new code, use it right away, don't use it for anything else.

---

## QR scan doesn't respond

The camera may be open but nothing appears in a screenshot — that's normal.
The camera video uses a protected surface that `adb screencap` can't capture.

What to check:

1. Camera permission is granted for the site.
2. The `hint` changes to "Point the camera at the QR code in the desktop app."
3. The **Choose QR photo** button is available as a fallback.

If permission is denied, the message is "Camera permission denied" — not silence.

---

## Terminal exits by itself after a few seconds

Two causes seen before:

| Cause | Fix |
|---|---|
| `tmux detach-client -s <session>` detaches all clients | only `child.kill()` |
| resume uses the same tmux session name, `-A` attaches to the old omp | session name `omp-ai-r<hash>` |
| two terminal sockets alive at once | `onclose` only reconnects if still the owner |

Check existing tmux sessions:

```sh
tmux list-sessions
tmux list-panes -a -F "#{session_name} :: #{pane_current_command}"
```

---

## Resume doesn't continue the session (you get a new one)

omp refuses to write to a session file that another process has open:

```
Warning: Session … is open for writing in another omp process,
so this session now saves to … instead
```

That's omp behavior, not a bug. Resume a session that is **not** currently
active in another omp.

---

## Relay deploy fails: `parameter not set`

```
./scripts/relay-setup-nginx.sh: ra_backend: parameter not set
```

The config heredoc isn't quoted, so the shell expands `$ra_backend`. It must be
`<<'CONF'`, with `@HOST@` / `@DIR@` placeholders for dynamic values.

---

## nginx: `invalid condition "$arg_k"`

There's a leftover `\$` escape in the template. Once the heredoc is quoted,
nginx variables must be written without a backslash.

---

## PWA keeps using the old version after deploy

Three causes:

1. `/app.js` and `/sw.js` without `cache-control`
2. asset versions based on file size, not content hash
3. hardcoded service worker `VERSION` that was forgotten

Now: `app.js`/`style.css` → `no-cache, must-revalidate`; `sw.js` →
`no-store`; asset versions and `VERSION` derived from content hash.

Verify from the phone:

```js
(await navigator.serviceWorker.getRegistration()).active
caches.keys()          // must be "ra-<hash>", not "ra-v10"
```

---

## UI resets itself after logout

Previously `localStorage.clear()` also wiped the layout. Now only credentials
are deleted; `ra.view`, `ra.sidebar`, `ra.prefs`, `ra.project` are preserved.

If it still resets, the phone really cleared site data — prefs are gone, and
defaults are restored.

---

## Black screen / `capture.error`

Screen Recording isn't allowed. After allowing it, **restart the app** — TCC
doesn't apply to an already-running process.

---

## Clicks have no effect

Accessibility isn't allowed. `/api/status` reports `input.trusted: false` and
the app shows a toast.

---

## "Session moved" / HTTP 409

The relay only allows one active device. Other devices receive 409 and the
device picker. Tap the device you want to use; the old device stops
automatically. See [relay.md](relay.md).

---

## Log out all has no effect

Old cookies only become invalid if `secret` is actually deleted. Make sure
`rotate-token.sh` runs with `--restart`, or delete it manually:

```sh
rm ~/Library/Application\ Support/Remote\ Agents/secret
```

---

## Collecting evidence

```sh
# from the Mac
./scripts/show-access.sh
curl -s localhost:8788/api/status | jq .

# from the phone (USB debugging)
adb forward tcp:9222 localabstract:chrome_devtools_remote
curl -s localhost:9222/json/list | jq -r '.[].url'
```

The phone debugger gives you the real cookies, `caches.keys()` output, and
WebSocket events — much faster than guessing from screenshots.
