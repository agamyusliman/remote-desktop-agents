# Konfigurasi

Tiga sumber pengaturan, berurutan menang:

1. `settings.json` di direktori data (ditulis UI Setup)
2. environment variable
3. default bawaan

Direktori data:

- app terpasang: `~/Library/Application Support/Remote Agents/`
- mode developer: `./data/` (override dengan `REMOTE_DATA`)

## Environment variable

| Variabel | Default | Arti |
|---|---|---|
| `HOST` | `0.0.0.0` | alamat bind |
| `PORT` | `8788` | port |
| `OMP_CWD` | `~/Documents/Ai` | folder awal omp |
| `ALLOWED_ROOTS` | `OMP_CWD` | root yang boleh diakses, dipisah `:` |
| `OMP_BIN` | `/opt/homebrew/bin/omp` | binary omp |
| `TMUX_BIN` | `/opt/homebrew/bin/tmux` | binary tmux |
| `CAPTURE_FPS` | `5` | batas frame per detik |
| `JPEG_QUALITY` | `62` | 20–95 |
| `CAPTURE_RESOLUTION` | `720p` | `720p` / `1080p` |
| `REMOTE_TOKEN` | acak → `data/token` | secret panjang |
| `PUBLIC_URL` | kosong | URL yang ditampilkan di Settings |
| `REMOTE_DATA` | `./data` | direktori data |
| `SESSION_TTL_DAYS` | `365` | umur cookie login (1–3650) |
| `REMOT_AGENT_DESKTOP` | `0` | `1` = mode tray + onboarding QR |
| `REMOT_AGENT_TUNNEL` | `0` | autostart tunnel saat login |

## Berkas di direktori data

| Berkas | Isi | Mode |
|---|---|---|
| `id` | ID perangkat 9 digit | 600 |
| `token` | token panjang | 600 |
| `secret` | kunci HMAC cookie | 600 |
| `clients.json` | allowlist klien hasil pairing | 600 |
| `projects.json` | library project | 644 |
| `settings.json` | pengaturan dari UI | 644 |
| `relay-devices.txt` | mapping relay: `<key> <host:port>` per baris | 644 |
| `tunnel-url` | URL publik hasil tunnel (opsional) | 644 |

Hapus `secret` untuk membatalkan semua cookie. Hapus `clients.json` untuk
membatalkan semua pairing.

## Nilai settings dari UI

`/api/setup` mengembalikan daftar ini; `POST /api/setup` menyimpannya.

```
deviceName  host  port  publicUrl  captureFps  jpegQuality
resolution  ompCwd  ompBin  allowedRoots  sessionTtlDays
requirePairing  relayUrl  relayKey
```

`relayKey` adalah kunci perangkat di relay (`a`, `b`, …). Dipakai klien untuk
menulis cookie `ra_device` setelah pairing.

## Preferensi klien (localStorage)

Disimpan di HP, bukan di server:

| Kunci | Isi |
|---|---|
| `ra.view` | layout: `term`, `screen`, `files`, `split` |
| `ra.sidebar` | `expanded` / `collapsed` |
| `ra.prefs` | ukuran font, opsi terminal |
| `ra.project` | project terakhir dibuka |
| `ra.keys` | kunci klien per perangkat, agar tap kartu = masuk tanpa pairing ulang |

Karena disimpan lokal, HP baru atau clear-site-data mengembalikan ke default.

`ra.keys` sengaja **tidak** dihapus saat logout: logout mengakhiri sesi,
sedangkan kunci perangkat tetap tersimpan di HP itu. Untuk benar-benar memutus
akses, hapus perangkat di daftar app desktop — server mencabut kuncinya.
