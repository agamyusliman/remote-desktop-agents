# Getting started

Goal: from install to seeing the Mac screen on your phone, no terminal needed.

## 1. Install the desktop app

Build it yourself:

```sh
./scripts/package-macos.sh --install   # → /Applications/Remote Agents.app
```

Or open the DMG produced by `./scripts/package-dmg.sh`, then drag it to Applications.

## 2. Grant macOS permissions

On first launch, macOS asks for two permissions. Both are required:

| Permission | For | If denied |
|---|---|---|
| **Screen Recording** | capturing the screen | black screen, `capture.error` in `/api/status` |
| **Accessibility** | mouse & keyboard | `input.trusted: false`, clicks have no effect |

The Setup UI provides an "Open settings" button for both. After changing
permissions, **the app must be restarted** — TCC doesn't apply to a process that
is already running.

## 3. Create a pairing code

In the desktop app, **Connect** tab → **Create pairing code**.

A QR code and a 6-character code appear. The code:

- is single use
- is valid for 5 minutes
- can be used by one device only

## 4. Connect from your phone

Open the relay URL, then either:

- **Scan QR** — camera, most reliable because the QR carries `?k=` which selects which Mac
- **Enter code** — `ABC-123`

Once connected, the phone stores its own access key. No re-pairing needed.

## 5. Install as an app on your phone

- Android Chrome: ⋮ menu → **Install app**
- iPhone Safari: Share → **Add to Home Screen**

The PWA has a service worker that caches only the UI shell (not API, screen, or
terminal), so it still opens while the Mac is off.

## Phone UI flow

Everything is on one page — there are no separate pages for pairing.

| Screen | Contents |
|---|---|
| **Home** | device status, CPU/RAM/disk/net, last project |
| **Projects** | project list + `+` button at the top right of the header |
| **Remote** | `CLI / Screen / Files / Split` toolbar, session icon, project name, reload, keyboard |
| **System** | processes and metrics |
| **Settings** | FPS, quality, font size, change device |

A few habits to know:

- **Add project** = tap `+` then choose a folder. The project name is taken
  automatically from the folder name; there's no name field.
- **Delete project** = swipe the card left, tap the trash icon. It's only removed
  from the list; the folder on disk is untouched.
- **Switch session** = tap the clock icon in the Remote toolbar. The session list
  appears over the terminal, so you don't need to leave the project or type
  `/resume`.
- **Rename/delete session** = pencil / trash icon on each session row.
  Rename writes to the session file, so omp uses the new name too.
- **Attach to a running terminal** = the same panel lists tmux sessions already
  open on the Mac under "Running on this Mac". Tapping one joins that session
  instead of starting a new agent, so the phone drives the same context and any
  tool it launches — an emulator, a dev server — is visible on the screen
  stream. Sessions you attach to skip `/resume`; tmux would ignore it anyway.

## Where data is stored

| Item | Location |
|---|---|
| Desktop app data | `~/Library/Application Support/Remote Agents/` |
| relay-link binary | `…/Remote Agents/bin/` |
| Device keys (`clients.json`) | `…/Remote Agents/clients.json` |
| Long token | `…/Remote Agents/token` |
| HMAC secret | `…/Remote Agents/secret` |
| Developer mode | `./data/` at the project root |

An old `./data` is copied automatically the first time the app runs.

## Developer mode

```sh
./scripts/setup.sh          # create data/, token, .env
$EDITOR .env                # set OMP_CWD / ALLOWED_ROOTS
cargo build --release
set -a; source .env; set +a
./target/release/remot-agent
```

Open the printed URL. See [configuration.md](configuration.md) for all
variables.

## Accessing from outside your home

Don't expose port `8788` to the internet. Use one of:

| Method | Command | Note |
|---|---|---|
| Your own relay | `./scripts/relay-setup-nginx.sh root@vps yourdomain.com` | one hostname for several Macs, see [relay.md](relay.md) |
| Tailscale | `tailscale serve 8788` | private, no domain |
| Cloudflare named tunnel | `./scripts/setup-named-tunnel.sh remote.yourdomain.com` | fixed URL |
| Quick tunnel | `./scripts/tunnel.sh` | random URL, for testing |

## Autostart

Toggle in the Setup UI. Manually:

```sh
cp scripts/sh.remot-agent.plist.example ~/Library/LaunchAgents/
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/sh.remot-agent.plist
```

## Next

Pairing failed? See [pairing.md](pairing.md) and
[troubleshooting.md](troubleshooting.md).
