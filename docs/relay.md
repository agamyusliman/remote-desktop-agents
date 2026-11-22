# Relay

One hostname, one active device. The relay lets several Macs be reached from a
single address without a subdomain per machine.

```
Phone ──HTTPS──> nginx (VPS) ──SSH tunnel──> Mac A / Mac B / …
```

## Rules

- The relay stores one active device key in
  `/var/lib/remote-agents/active`.
- Inactive devices receive **409** and a "Session moved" page.
- Switch devices through the device list in the app; the old session stops.
- Device history (name, ID, last used) is stored in the browser.
- Each device has its own session cookie (`ra_session_<id>`), so after logging
  in once you don't need to log in again when it's selected again.

## Flow

1. Open `https://<relay>`.
2. No `ra_device` cookie yet → the relay serves the app with `?picker=1`.
3. Tap a device → `POST /switch?key=<key>` → `ra_device` is set.
4. `/auth?id=<id>` → device session cookie → enter the app.
5. Other devices still open automatically show "Session moved".

## Mapping

`data/relay-devices.txt`

```
a 127.0.0.1:8788
b 127.0.0.1:8790
```

Deploy:

```sh
./scripts/relay-setup-nginx.sh root@<host> <domain>
```

That command generates the nginx config, copies the app shell, and fills
`/var/lib/remote-agents/keys`.

## Relay endpoints

| Endpoint | Function |
|---|---|
| `GET /selector.html` | 302 to `/?picker=1` (legacy page, no longer used) |
| `GET /devices.json` | device list (name + ID) |
| `GET /active` | active device key |
| `GET /switch?key=a` | move the session (same-origin only) |
| `GET /probe/<key>` | device online status |
| `GET /pair?k=a` | pairing entry, sets `ra_device` |

## Adding a device

```
# data/relay-devices.txt
c 127.0.0.1:8792
```

Add it to `relay/devices.json`, rerun `relay-setup-nginx.sh`, then create a
tunnel `-R 127.0.0.1:8792:127.0.0.1:<port>`.

## PWA caching and updates

The relay decides whether the phone receives a new version. The rules:

| File | Header | Reason |
|---|---|---|
| `/app.js`, `/style.css` | `no-cache, must-revalidate` | the PWA has no reload button |
| `/sw.js` | `no-store` | the worker script is the update mechanism itself |
| `/` (shell) | `no-cache` | HTML is always fresh |

Asset versions in `index.html` use the **MD5 of file contents**, not size:

```
/app.js?v=0a90da595a-84139
    └ 10-char hash  └ size
```

Size alone isn't enough: two different versions can have the same size, and the
phone will keep using the old file.

The service worker cache version is also derived from the shell hash at deploy
time:

```js
const VERSION = "ra-26607a5c1c52";   // app.js + style.css + index.html
```

## nginx configuration

The heredoc in `relay-setup-nginx.sh` **is quoted** (`<<'CONF'`). Without quotes,
the shell expands nginx variables (`$ra_backend`, `$cookie_ra_device`) and the
config breaks. Dynamic values come in through the `@HOST@` and `@DIR@`
placeholders.
