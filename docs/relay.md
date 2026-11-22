# Relay

Satu hostname, satu perangkat aktif. Relay memungkinkan beberapa Mac dijangkau
dari satu alamat tanpa subdomain per mesin.

```
HP ──HTTPS──> nginx (VPS) ──tunnel SSH──> Mac A / Mac B / …
```

## Aturan

- Relay menyimpan satu kunci perangkat aktif di
  `/var/lib/remote-agents/active`.
- Perangkat yang tidak aktif menerima **409** dan halaman "Sesi pindah".
- Pindah perangkat lewat daftar perangkat di app; sesi lama berhenti.
- Riwayat perangkat (nama, ID, terakhir dipakai) disimpan di browser.
- Tiap perangkat punya cookie sesi sendiri (`ra_session_<id>`), jadi setelah
  login sekali tidak perlu login ulang saat dipilih kembali.

## Alur

1. Buka `https://<relay>`.
2. Belum ada cookie `ra_device` → relay menyajikan app dengan `?picker=1`.
3. Tap perangkat → `POST /switch?key=<key>` → `ra_device` diset.
4. `/auth?id=<id>` → cookie sesi perangkat → masuk ke app.
5. Perangkat lain yang masih terbuka otomatis menampilkan "Sesi pindah".

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

Perintah itu men-generate konfigurasi nginx, menyalin app shell, dan mengisi
`/var/lib/remote-agents/keys`.

## Endpoint relay

| Endpoint | Fungsi |
|---|---|
| `GET /selector.html` | 302 ke `/?picker=1` (halaman lama, sudah tidak dipakai) |
| `GET /devices.json` | daftar perangkat (nama + ID) |
| `GET /active` | kunci perangkat aktif |
| `GET /switch?key=a` | pindahkan sesi (same-origin saja) |
| `GET /probe/<key>` | status online perangkat |
| `GET /pair?k=a` | entry pairing, menyetel `ra_device` |

## Menambah perangkat

```
# data/relay-devices.txt
c 127.0.0.1:8792
```

Tambahkan ke `relay/devices.json`, jalankan ulang `relay-setup-nginx.sh`, lalu
buat tunnel `-R 127.0.0.1:8792:127.0.0.1:<port>`.

## Cache dan update PWA

Relay menentukan apakah HP menerima versi baru. Aturan yang dipakai:

| Berkas | Header | Alasan |
|---|---|---|
| `/app.js`, `/style.css` | `no-cache, must-revalidate` | PWA tidak punya tombol reload |
| `/sw.js` | `no-store` | skrip worker adalah mekanisme update itu sendiri |
| `/` (shell) | `no-cache` | HTML selalu segar |

Versi aset di `index.html` memakai **MD5 isi berkas**, bukan ukuran:

```
/app.js?v=0a90da595a-84139
    └ hash 10 char  └ ukuran
```

Ukuran saja tidak cukup: dua versi berbeda bisa berukuran sama, dan HP akan
terus memakai berkas lama.

Versi cache service worker juga diturunkan dari hash shell saat deploy:

```js
const VERSION = "ra-26607a5c1c52";   // app.js + style.css + index.html
```

## Konfigurasi nginx

Heredoc di `relay-setup-nginx.sh` **di-quote** (`<<'CONF'`). Tanpa quote, shell
mengekspansi variabel nginx (`$ra_backend`, `$cookie_ra_device`) dan konfigurasi
rusak. Nilai dinamis masuk lewat placeholder `@HOST@` dan `@DIR@`.
