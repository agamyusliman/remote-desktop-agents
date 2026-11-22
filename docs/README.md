# Remote Agents documentation

Remote Agents is a Rust + PWA server for controlling a Mac from a phone:
view the screen, move the mouse/keyboard, run omp (chat & terminal), and
browse project files.

## Where to start

| If you… | Read |
|---|---|
| are using it for the first time | [getting-started.md](getting-started.md) |
| want to connect your phone to a Mac | [pairing.md](pairing.md) |
| have more than one Mac | [relay.md](relay.md) |
| want to contribute code | [contributing.md](contributing.md) |
| are running into a problem | [troubleshooting.md](troubleshooting.md) |

## Contents

- **[getting-started.md](getting-started.md)** — install the app, macOS permissions, first steps on the phone.
- **[pairing.md](pairing.md)** — pairing flow, QR contents, cookies, device revoke.
- **[relay.md](relay.md)** — one hostname for several Macs, routing, device picker.
- **[access.md](access.md)** — token, rotation, logout, emergency access from the Mac.
- **[configuration.md](configuration.md)** — all environment variables and files in `data/`.
- **[architecture.md](architecture.md)** — `src/` layout, data flow, WebSocket.
- **[api.md](api.md)** — list of HTTP and WebSocket endpoints.
- **[security.md](security.md)** — threat model and limits.
- **[troubleshooting.md](troubleshooting.md)** — common symptoms and their causes.
- **[contributing.md](contributing.md)** — code rules and PR flow.
- **[agent-conventions.md](agent-conventions.md)** — Bun tooling conventions for automated agents.

## Design principles

1. **No cloud account needed.** The relay is only the entry path; access keys are
   stored on the device, not on the server.
2. **One active device.** Only one phone/laptop drives a Mac at a time, so two
   controllers don't collide.
3. **PWA with no build step.** `public/` is served as-is; there is no bundler.
4. **Battery first.** Capture stops when no one is watching the screen and
   skips identical frames.
