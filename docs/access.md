# Access & tokens

A token grants full control of the Mac. Treat it like a password.

## Viewing current access

From the Mac (not from the phone):

```sh
./scripts/show-access.sh
```

Output: 9-digit ID, token, LAN URL, WAN URL.

## Login

Three ways:

| Way | Suited for |
|---|---|
| Scan QR from the desktop app | normal use |
| 9-digit ID | no camera |
| Long token (`data/token`) | emergency / automation |

Open `/auth?id=<id>` or `/auth?token=<token>` once; the HttpOnly cookie is set
and valid for `SESSION_TTL_DAYS`.

## Logging in through pairing

With `requirePairing: true`, the ID does not work. Only clients that completed
pairing or the long token are accepted. See [pairing.md](pairing.md).

## Rotating the token (log out all devices)

```sh
./scripts/rotate-token.sh --restart
```

What happens:

1. `data/token` is rewritten
2. `data/secret` is deleted → all old cookies become invalid immediately
3. the server restarts
4. the new login URL is printed

## Revoking access

Security is managed from the **desktop app**, not from the phone. Reason: a lost
phone can't be trusted to revoke itself.

| Action | Where | Effect |
|---|---|---|
| Change device | Phone Settings → **Device** → Change | switch to another Mac on the relay |
| Forget device | desktop app → device list | that phone's key is revoked |
| Disconnect all | desktop app → device list | all keys revoked + token rotated |
| Manual rotation | `./scripts/rotate-token.sh --restart` | all cookies & pairings become invalid |

Clearing site data on the phone does **not** revoke anything on the server — the
key is still stored in `clients.json`. To really disconnect, remove it from the
desktop app.

Display preferences (`ra.view`, `ra.sidebar`, `ra.prefs`, `ra.project`) and
stored keys (`ra.keys`) are device state, not session credentials, so they
survive when a session ends.

## Expiry

Cookies are HMAC-signed and valid for `SESSION_TTL_DAYS` (default 365).
Logging in again extends the expiry. Change it in
Settings → Sessions & devices.

## Lost everything

If the token and ID are lost and no phone is still logged in:

```sh
rm ~/Library/Application\ Support/Remote\ Agents/secret
./scripts/rotate-token.sh --restart
```

The app creates a new secret and prints new access details.
