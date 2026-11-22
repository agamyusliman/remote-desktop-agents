# Troubleshooting

Gejala → penyebab → perbaikan.

## HP berhenti di `connecting`

**Penyebab paling umum:** cookie `ra_device` tidak ada. Relay memakai cookie itu
untuk merutekan `/api/*`; tanpa cookie, semua request gagal.

Periksa dari Mac:

```sh
curl -s -o /dev/null -w "%{http_code}\n" https://<relay>/api/status
# 401 {"ok":false,"error":"no_device"} → cookie hilang
```

**Perbaikan:** pairing ulang lewat QR, atau buka `/pair?k=a`. App juga
memulihkan cookie otomatis pada `401 no_device`.

---

## Kode pairing "salah atau kedaluwarsa" padahal baru dibuat

Kode **sekali pakai** dan berlaku 5 menit. Kalau ada proses lain (tes, tab lain)
yang memakainya lebih dulu, kode itu habis.

Buat kode baru, langsung pakai, jangan dipakai untuk hal lain.

---

## Scan QR tidak bereaksi

Kamera mungkin terbuka tapi tidak ada yang terlihat di screenshot — itu normal.
Video kamera memakai protected surface yang tidak bisa ditangkap `adb screencap`.

Yang perlu dipastikan:

1. Izin kamera untuk situs diberikan.
2. `hint` berubah menjadi "Arahkan kamera ke QR di app desktop."
3. Tombol **Pilih foto QR** tersedia sebagai cadangan.

Kalau izin ditolak, pesannya "Izin kamera ditolak" — bukan diam.

---

## Terminal keluar sendiri setelah beberapa detik

Dua penyebab yang pernah terjadi:

| Penyebab | Perbaikan |
|---|---|
| `tmux detach-client -s <sesi>` melepas semua klien | hanya `child.kill()` |
| resume memakai nama sesi tmux yang sama, `-A` attach ke omp lama | nama sesi `omp-ai-r<hash>` |
| dua socket terminal hidup bersamaan | `onclose` hanya reconnect bila masih pemilik |

Cek sesi tmux yang ada:

```sh
tmux list-sessions
tmux list-panes -a -F "#{session_name} :: #{pane_current_command}"
```

---

## Resume tidak melanjutkan sesi (dapat sesi baru)

omp menolak menulis ke file sesi yang sedang dibuka proses lain:

```
Warning: Session … is open for writing in another omp process,
so this session now saves to … instead
```

Itu perilaku omp, bukan bug. Resume sesi yang **tidak** sedang aktif di omp lain.

---

## Deploy relay gagal: `parameter not set`

```
./scripts/relay-setup-nginx.sh: ra_backend: parameter not set
```

Heredoc konfigurasi tidak di-quote, sehingga shell mengekspansi `$ra_backend`.
Harus `<<'CONF'`, dengan placeholder `@HOST@` / `@DIR@` untuk nilai dinamis.

---

## nginx: `invalid condition "$arg_k"`

Ada escape `\$` tersisa di template. Setelah heredoc di-quote, variabel nginx
harus ditulis tanpa backslash.

---

## PWA tetap memakai versi lama setelah deploy

Tiga penyebab:

1. `/app.js` dan `/sw.js` tanpa `cache-control`
2. versi aset memakai ukuran berkas, bukan hash isi
3. `VERSION` service worker hardcode dan lupa dinaikkan

Sekarang: `app.js`/`style.css` → `no-cache, must-revalidate`; `sw.js` →
`no-store`; versi aset dan `VERSION` diturunkan dari hash isi.

Verifikasi dari HP:

```js
(await navigator.serviceWorker.getRegistration()).active
caches.keys()          // harus "ra-<hash>", bukan "ra-v10"
```

---

## UI berubah sendiri setelah logout

Dulu `localStorage.clear()` menghapus layout juga. Sekarang hanya kredensial
yang dihapus; `ra.view`, `ra.sidebar`, `ra.prefs`, `ra.project` dipertahankan.

Kalau tetap berubah, berarti HP benar-benar clear site data — prefs memang
hilang, dan default dikembalikan.

---

## Layar hitam / `capture.error`

Screen Recording belum diizinkan. Setelah mengizinkan, **jalankan ulang app** —
TCC tidak berlaku pada proses yang sudah berjalan.

---

## Klik tidak berpengaruh

Accessibility belum diizinkan. `/api/status` melaporkan `input.trusted: false`
dan app menampilkan toast.

---

## "Sesi pindah" / HTTP 409

Relay hanya mengizinkan satu perangkat aktif. Perangkat lain menerima 409 dan
halaman pilihan. Tap perangkat yang ingin dipakai; perangkat lama otomatis
berhenti. Lihat [relay.md](relay.md).

---

## Logout semua tidak berpengaruh

Cookie lama hanya tidak valid kalau `secret` benar-benar dihapus. Pastikan
`rotate-token.sh` dijalankan dengan `--restart`, atau hapus manual:

```sh
rm ~/Library/Application\ Support/Remote\ Agents/secret
```

---

## Mengumpulkan bukti

```sh
# dari Mac
./scripts/show-access.sh
curl -s localhost:8788/api/status | jq .

# dari HP (USB debugging)
adb forward tcp:9222 localabstract:chrome_devtools_remote
curl -s localhost:9222/json/list | jq -r '.[].url'
```

Debugger HP memberi cookie nyata, hasil `caches.keys()`, dan event WebSocket —
jauh lebih cepat daripada menebak dari screenshot.