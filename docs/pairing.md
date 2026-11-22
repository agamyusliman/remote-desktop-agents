# Pairing

The relay is only the entry path. After pairing, the access key lives on the
device.

## What is stored where

| Relay (VPS) | Device |
|---|---|
| routing key (`a`, `b`) | 32-byte client key |
| device name + ID | session `ra_session_<id>` |
| cookie `ra_device` | `clients.json` (allowlist, mode 600) |
| — | `ra_c_<device>_<prefix>` (HttpOnly) |

The relay **never** stores the client key. It only forwards bytes.

## Flow

1. Desktop app: **Create pairing code** → 6 characters, single use, 5 minutes.
2. The QR contains:

   ```
   https://relay/pair?k=<relay-key>#c=<code>&d=<device-id>
   ```

3. The phone opens `/pair?k=a#c=CODE`. The relay **does not render a page**: it
   sets the `ra_device=a` cookie, then `302`s to `/?k=a`. The browser keeps the
   `#c=` fragment because `Location` doesn't carry it.
4. The app (the only UI) reads `#c=`, calls `POST /api/pair/claim`, then cleans
   the URL with `history.replaceState`.
5. The Mac replies with the client key; the phone stores it as the HttpOnly
   `ra_c_<device-id>_<prefix>` cookie **and** in `localStorage` (`ra.keys`).
6. Subsequent requests use that cookie. The relay only sees the route.

## Why the code lives in the fragment (`#c=`)

Fragments are never sent to the server. The relay and access logs never see the
pairing code. The code is sent once, straight to the Mac, in the POST body.

## The `ra_device` cookie

This cookie **belongs to the relay**, not the app. Its job is to pick which Mac
a single hostname routes to when it serves several Macs.

The relay uses this cookie to route every `/api/*`. Without the cookie:

```
POST /api/status → 401 {"ok":false,"error":"no_device"}
```

The cookie is written by:

| Path | Written by |
|---|---|
| open `/pair?k=a` (from the QR) | nginx (`Set-Cookie`) |
| tap a device card in the app | JavaScript (`restoreTrustedCookies`) |
| successful pairing in the app | JavaScript (`rememberRelayDevice`) |

If the cookie is lost (clear site data, PWA uninstall), the app restores it
automatically: on `401 no_device` it reads `relayKey` from `/api/setup`, rewrites
the cookie, then retries the request. For single-device installs, the relay also
injects `window.__RA_DEVICE` into the shell so a phone without the cookie can
still recover.

## Remembered devices (`ra.keys`)

Besides the HttpOnly cookie, the phone stores its client keys in `localStorage`.
Effect: after **logging out**, tapping a device card logs straight back in
without a new pairing. Those keys are only deleted when you choose *Forget
device* or remove the device from the list in the desktop app.

## Strict mode

`requirePairing: true` disables 9-digit ID login. Only these are accepted:

- paired devices, or
- the long token (`data/token`)

## Revoke

```sh
# one device
POST /api/pair/revoke   {"id": "c…"}

# all
POST /api/pair/revoke   {"all": true}
```

Immediate effect: requests with old cookies receive `401`.

## Endpoints

| Endpoint | Access |
|---|---|
| `POST /api/pair/new` | loopback / paired |
| `GET /desktop/setup-qr.svg?c=` | loopback / session |
| `GET /pair` | public (with `?k=`) |
| `POST /api/pair/claim` | public (with the `ra_device` cookie) |
| `GET /api/pair/clients` | loopback / paired |
| `POST /api/pair/revoke` | loopback / paired |

## What the relay can still see

- the client key **during** `/api/pair/claim` (TLS terminates at the VPS)
- metadata: IP, time, traffic size, which Mac is active

To eliminate the first, the next step is to put the Mac's public key in the QR
and encrypt the claim so the relay becomes a blind pipe.

## Common problems

| Symptom | Cause |
|---|---|
| "Code is wrong or expired" even though it was just created | the code was already used by another process, or it's > 5 minutes old |
| phone gets in but stays at `connecting` | `ra_device` cookie missing → `/api/*` rejected |
| QR scan stops at a blank page | link without `?k=`, so the relay can't select a Mac |
| device card says "pairing needed" | `ra.keys` has no key for that device |
