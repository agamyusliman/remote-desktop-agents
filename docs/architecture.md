# Arsitektur

```
HP (PWA)  ──HTTPS/WSS──>  relay nginx  ──tunnel──>  remot-agent (Rust)  ──>  macOS
                                                     ├─ scap      (layar)
                                                     ├─ enigo     (input)
                                                     ├─ portable-pty + tmux
                                                     └─ omp --mode rpc-ui
```

## Struktur `src/`

| Berkas | Tanggung jawab |
|---|---|
| `main.rs` | route HTTP/WS, auth, API file & project, PWA shell |
| `auth.rs` | cookie HMAC, allowlist sesi, rate limit login |
| `pairing.rs` | kode pairing, client key, revoke |
| `rpc.rs` | jembatan omp RPC (chat, model, sesi, form ask) |
| `capture.rs` | loop ScreenCaptureKit; idle-aware + deteksi frame identik |
| `input.rs` | worker enigo untuk mouse/keyboard |
| `terminal.rs` | PTY + tmux, satu sesi tmux per target |
| `projects.rs` | library project dengan allowlist |
| `sysmon.rs` | sampler CPU/RAM/disk/jaringan |
| `config.rs` | env + `settings.json` |
| `identity.rs` | ID perangkat, secret |

## Struktur `public/`

Shell PWA, tanpa build step.

| Berkas | Isi |
|---|---|
| `index.html` | shell app, semua pane |
| `app.js` | logika klien (nav, terminal, layar, file, pairing) |
| `style.css` | seluruh styling |
| `sw.js` | service worker: cache shell saja |
| `pair.js`, `setup.css` | halaman pairing/onboarding |
| `vendor/` | xterm, marked, DOMPurify, jsQR |

## Alur layar

1. Klien membuka `/ws/screen`.
2. `capture.rs` mengambil frame, meng-encode JPEG, mengirim biner.
3. Klien menggambar ke `<canvas>`. Input (mouse/keyboard) dikirim sebagai pesan
   JSON pada WebSocket `/ws/screen` yang sama, lalu dieksekusi `input.rs`.
4. Capture berhenti saat tidak ada penonton, dan melewati frame yang identik.

Frame dikirim sebagai WebSocket biner — bukan base64 di JSON — supaya tidak ada
biaya encode/decode tambahan di HP.

## Alur terminal

1. Klien membuka `/ws/term?cols=&rows=&project=&resume=`.
2. Server menjalankan `tmux new-session -A -s <nama>` di PTY.
3. Nama sesi:

   ```
   tanpa resume : omp-ai
   dengan resume: omp-ai-r1a2b3c4d   ← hash file sesi
   ```

   Resume perlu sesi sendiri. Kalau memakai nama yang sama, `-A` akan
   *attach* ke omp yang sudah jalan dan argumen `--resume` diabaikan.

4. Saat koneksi putus, hanya proses klien tmux itu yang dimatikan.

> Jangan memakai `tmux detach-client -s <sesi>` untuk menutup koneksi:
> perintah itu melepas **semua** klien pada sesi tersebut, sehingga satu HP
> lama yang tertutup ikut memutus HP yang sedang dipakai.

## Alur chat (omp RPC)

`rpc.rs` menjalankan `omp --mode rpc-ui` sebagai proses anak dan bertukar JSON:

```
klien ──ws/rpc──> rpc.rs ──stdin/stdout──> omp
```

Pesan yang didukung antara lain `get_state`, `get_messages_page`, `send_prompt`,
pemilihan model, serta form `ask`/approval.

## Alur pairing

Lihat [pairing.md](pairing.md). Ringkas:

```
desktop: /api/pair/new  → kode + QR
HP     : /pair?k=a      → nginx Set-Cookie ra_device=a
HP     : /api/pair/claim→ Mac membalas client key
HP     : cookie ra_c_…  → semua request berikutnya
```

## Service worker

| Permintaan | Strategi |
|---|---|
| navigasi (`/`) | network-first, fallback shell |
| `app.js`, `style.css` | network-first |
| vendor, ikon | stale-while-revalidate |
| `/api/*`, `/ws/*`, `/sw.js` | tidak disentuh |

Versi cache diturunkan dari hash shell saat deploy, sehingga setiap perubahan
kode membuka cache baru dan menghapus yang lama.

## Baterai

- capture berhenti bila `screenClients == 0`
- frame identik tidak dikirim ulang
- capture dijeda saat tab Remote tidak aktif
- terminal tidak di-resize tanpa perubahan ukuran nyata