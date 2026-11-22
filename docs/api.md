# API

Semua endpoint `allow()`-gated kecuali yang ditandai publik. Auth lewat cookie
sesi yang ditandatangani HMAC, atau klien hasil pairing.

## Status & konfigurasi

| Method | Path | Keterangan |
|---|---|---|
| `GET` | `/api/status` | capture, input, session, cwd, viewer |
| `GET` | `/api/system` | CPU, RAM, disk, network |
| `GET` | `/api/identity` | nama + ID perangkat |
| `GET` | `/api/ping` | probe ringan |
| `GET` | `/api/config` | konfigurasi efektif |
| `GET` | `/api/setup` | semua setting + `pairing` + `permissions` |
| `POST` | `/api/setup` | simpan setting |
| `POST` | `/api/setup/restart` | restart proses |
| `POST` | `/api/setup/open` | buka panel izin macOS |
| `GET`/`POST` | `/api/setup/autostart` | status / set LaunchAgent |

## Sesi

| Method | Path | Keterangan |
|---|---|---|
| `POST` | `/api/login` | login dengan ID atau token |
| `POST` | `/api/logout` | hapus sesi browser ini |
| `POST` | `/api/logout-all` | rotasi token, semua keluar |
| `GET` | `/api/sessions` | daftar sesi omp |
| `GET` | `/api/project-sessions?path=` | sesi tersimpan untuk project |
| `POST` | `/api/session-label` | ganti nama sesi (`{path,label}`) |
| `POST` | `/api/session-delete` | hapus satu sesi (`{path}`) |

## Pairing

| Method | Path | Akses |
|---|---|---|
| `POST` | `/api/pair/new` | loopback / paired |
| `POST` | `/api/pair/claim` | publik, butuh `ra_device` |
| `GET` | `/api/pair/clients` | loopback / paired |
| `POST` | `/api/pair/revoke` | loopback / paired |
| `POST` | `/api/pair/request` | perangkat meminta persetujuan |
| `POST` | `/api/pair/approve` | Mac menyetujui |

## Project & file

| Method | Path | Keterangan |
|---|---|---|
| `GET`/`POST` | `/api/projects` | daftar / tambah project (`name` opsional) |
| `DELETE` | `/api/projects/{id}` | hapus dari daftar; folder tidak disentuh |
| `GET` | `/api/browse?path=` | penjelajah folder untuk "tambah project" |
| `GET` | `/api/fs` | daftar direktori dalam `ALLOWED_ROOTS` |
| `GET` | `/api/files` | pohon berkas project |
| `GET` | `/api/files/read` | isi berkas (teks) |
| `GET` | `/api/files/raw` | isi mentah (biner/gambar) |

Semua path dikanonikalisasi lebih dulu; path di luar `ALLOWED_ROOTS` ditolak.

## Halaman publik

| Path | Isi |
|---|---|
| `/` | shell PWA (satu-satunya UI: pairing, perangkat, sesi) |
| `/auth?id=` / `/auth?token=` | 302 ke `/` (kompatibilitas tautan lama) |
| `/pair` | 302 ke `/?k=<key>`, mempertahankan `#c=<kode>` |
| `/desktop` | UI Setup desktop |
| `/desktop/qr.svg` | QR berisi ID |
| `/desktop/setup-qr.svg?c=` | QR pairing |

## WebSocket

| Path | Auth | Fungsi |
|---|---|---|
| `/ws/screen` | cookie | frame JPEG biner |
| `/ws/term` | cookie | terminal (PTY + tmux) |
| `/ws/rpc` | cookie | chat omp |

### `/ws/term`

Query:

```
cols, rows, project, resume
```

Pesan masuk (JSON teks):

```json
{"type":"input","data":"ls\r"}
{"type":"resize","cols":78,"rows":37}
{"type":"scroll","dir":"up","lines":10}
```

Keluaran dikirim sebagai biner (byte terminal).

## Relay (nginx, bukan app)

| Path | Fungsi |
|---|---|
| `/active` | kunci perangkat aktif |
| `/switch?key=` | pindah perangkat (same-origin) |
| `/probe/<key>` | status online |
| `/selector.html` | 302 ke `/?picker=1` (pemilih ada di app) |
| `/devices.json` | daftar perangkat (404 di relay; hanya lewat `/api/devices`) |

Ketika cookie `ra_device` tidak ada, `/api/*` dibalas `401 JSON`
`{"ok":false,"error":"no_device"}` — bukan halaman HTML, supaya klien bisa
memulihkan cookie alih-alih macet di status `connecting`.

Semua respons `/api/*` membawa `Cache-Control: no-store` (middleware di
`src/main.rs`). Tanpa itu, body lama bisa diputar ulang dari cache browser dan
aplikasi boot ke layar yang salah.

## Format sesi omp

`/api/project-sessions` membaca `~/.omp/agent/sessions/<proyek>/*.jsonl`. Dua
baris pertama menentukan yang ditampilkan:

```json
{"type":"title","title":"...","source":"auto"}
{"type":"session","id":"...","title":"...","titleSource":"auto"}
```

Rename tidak mengganti nama berkas (omp memakainya untuk resume); hanya nilai
`title` yang ditulis ulang, dengan `source`/`titleSource` menjadi `user`.