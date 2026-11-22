# Remote Agents

Self-hosted remote desktop **dan** chat/terminal `omp` untuk HP atau tablet.
Server Rust + PWA. Tanpa akun cloud, tanpa layanan pihak ketiga.

Kendalikan satu Mac dari mana saja: lihat layar, gerakkan mouse dan keyboard,
jalankan `omp`, kelola project, dan jelajahi berkas — semuanya dari browser HP
yang bisa dipasang sebagai app.

> **Repo publik.** Tidak ada token, kunci perangkat, atau alamat server di sini.
> Konfigurasi nyata hidup di `data/` dan `relay/devices.json`, keduanya
> di-`.gitignore`. Lihat [docs/security.md](docs/security.md) sebelum deploy.

## Fitur

| Fitur | Keterangan |
|---|---|
| **Layar** | ScreenCaptureKit via `scap`, JPEG ~5 fps lewat WebSocket, input via `enigo` |
| **Chat omp** | jembatan RPC (`omp --mode rpc-ui`), pemilih model, resume sesi, form ask |
| **Terminal** | PTY nyata via `portable-pty` + tmux + xterm.js |
| **Sesi** | daftar, resume, rename, dan hapus sesi omp tanpa keluar project |
| **Project** | library ber-allowlist; tambah dengan memilih folder, hapus dari daftar |
| **Berkas** | penjelajah pohon, penampil kode, pratinjau markdown |
| **Sistem** | CPU, RAM, disk, jaringan, proses teratas |
| **PWA** | install di iOS/Android; service worker hanya menyimpan shell |
| **Hemat baterai** | capture berhenti tanpa penonton, melewati frame identik |

## Kebutuhan

- macOS 12.3+ (ScreenCaptureKit), Rust 1.85+
- [`omp`](https://github.com/) terpasang (default `/opt/homebrew/bin/omp`)
- `tmux`
- Opsional: VPS untuk relay (nginx), `cloudflared` atau `tailscale`

## Mulai cepat

```sh
git clone https://github.com/agamyusliman/remote-desktop-agents.git
cd remote-desktop-agents
./scripts/setup.sh          # buat data/, token, .env
$EDITOR .env                # set OMP_CWD dan ALLOWED_ROOTS
cargo build --release
set -a; source .env; set +a
./target/release/remot-agent
```

Atau pakai app desktop:

```sh
./scripts/package-macos.sh --install   # → /Applications/Remote Agents.app
```

Lalu:

1. Buka app, izinkan **Screen Recording** dan **Accessibility**, jalankan ulang.
2. Tab **Hubungkan** → **Buat kode pairing**.
3. Scan QR dari HP, atau ketik kodenya.

Panduan lengkap: [docs/getting-started.md](docs/getting-started.md).

## Akses dari luar rumah

Jangan pernah membuka port `8788` ke internet. Pilihan:

| Cara | Catatan |
|---|---|
| Relay nginx di VPS | satu hostname untuk beberapa Mac — [docs/relay.md](docs/relay.md) |
| Tailscale | `tailscale serve 8788`, privat tanpa domain |
| Cloudflare named tunnel | `./scripts/setup-named-tunnel.sh remote.domainmu.com` |
| Quick tunnel | `./scripts/tunnel.sh`, URL acak untuk uji coba |

## Keamanan singkat

- Cookie sesi ditandatangani HMAC (`HttpOnly`, `Secure`); klien pairing memakai
  kunci bearer yang disimpan di HP dan di `clients.json` Mac.
- Setiap endpoint berkas dibatasi `ALLOWED_ROOTS`.
- Relay menolak `/api/*` tanpa cookie perangkat dengan `401 JSON`, bukan halaman.
- Hapus perangkat dari daftar di app desktop untuk memutus akses.
- Rotasi semua akses: `./scripts/rotate-token.sh --restart`.

Detail model ancaman: [docs/security.md](docs/security.md).

## Dokumentasi

| Dokumen | Isi |
|---|---|
| [docs/getting-started.md](docs/getting-started.md) | install, izin macOS, alur UI, langkah pertama |
| [docs/pairing.md](docs/pairing.md) | alur pairing, isi QR, cookie, revoke |
| [docs/relay.md](docs/relay.md) | satu hostname untuk beberapa Mac |
| [docs/access.md](docs/access.md) | token, rotasi, akses darurat |
| [docs/configuration.md](docs/configuration.md) | env var, isi direktori data, preferensi klien |
| [docs/architecture.md](docs/architecture.md) | struktur kode dan alur data |
| [docs/api.md](docs/api.md) | endpoint HTTP, WebSocket, format sesi |
| [docs/security.md](docs/security.md) | model ancaman dan batasan |
| [docs/troubleshooting.md](docs/troubleshooting.md) | gejala umum dan penyebabnya |
| [docs/contributing.md](docs/contributing.md) | aturan kode dan alur PR |

## Struktur singkat

```
src/        server Rust (axum): capture, input, terminal, rpc, pairing, auth
public/     PWA: index.html, app.js, style.css, sw.js, vendor/
relay/      konfigurasi nginx + contoh devices
scripts/    setup, packaging, tunnel, relay, rotasi token
docs/       dokumentasi
data/       state runtime — TIDAK di-commit
```

## Lisensi

MIT — lihat [LICENSE](LICENSE).
