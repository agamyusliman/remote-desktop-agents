# Pairing

Relay hanya jalur awal. Setelah pairing, kunci akses hidup di perangkat.

## Apa yang disimpan di mana

| Relay (VPS) | Perangkat |
|---|---|
| routing key (`a`, `b`) | client key 32 byte |
| nama + ID perangkat | sesi `ra_session_<id>` |
| cookie `ra_device` | `clients.json` (allowlist, mode 600) |
| — | `ra_c_<device>_<prefix>` (HttpOnly) |

Relay **tidak pernah** menyimpan client key. Ia hanya meneruskan byte.

## Alur

1. App desktop: **Buat kode pairing** → 6 karakter, sekali pakai, 5 menit.
2. QR berisi:

   ```
   https://relay/pair?k=<relay-key>#c=<kode>&d=<device-id>
   ```

3. HP membuka `/pair?k=a#c=KODE`. Relay **tidak merender halaman**: ia
   menyetel cookie `ra_device=a` lalu `302` ke `/?k=a`. Browser mempertahankan
   fragment `#c=` karena `Location` tidak membawanya.
4. App (satu-satunya UI) membaca `#c=`, memanggil `POST /api/pair/claim`, lalu
   membersihkan URL dengan `history.replaceState`.
5. Mac membalas client key; HP menyimpannya sebagai cookie HttpOnly
   `ra_c_<device-id>_<prefix>` **dan** di `localStorage` (`ra.keys`).
6. Request berikutnya memakai cookie itu. Relay hanya melihat route.

## Kenapa kode ada di fragment (`#c=`)

Fragment tidak pernah dikirim ke server. Relay dan access log tidak melihat
kode pairing. Kode dikirim sekali, langsung ke Mac, di body POST.

## Cookie `ra_device`

Ini cookie **milik relay**, bukan milik app. Fungsinya memilih Mac mana yang
dituju saat satu hostname melayani beberapa Mac.

Relay memakai cookie ini untuk merutekan setiap `/api/*`. Tanpa cookie:

```
POST /api/status → 401 {"ok":false,"error":"no_device"}
```

Cookie ditulis oleh:

| Jalur | Ditulis oleh |
|---|---|
| buka `/pair?k=a` (dari QR) | nginx (`Set-Cookie`) |
| tap kartu perangkat di app | JavaScript (`restoreTrustedCookies`) |
| pairing sukses di app | JavaScript (`rememberRelayDevice`) |

Bila cookie hilang (clear site data, uninstall PWA), app memulihkannya otomatis:
pada `401 no_device` ia membaca `relayKey` dari `/api/setup`, menulis ulang
cookie, lalu mengulang request. Untuk instalasi satu perangkat, relay juga
menyisipkan `window.__RA_DEVICE` ke shell agar HP tanpa cookie tetap bisa pulih.

## Perangkat yang diingat (`ra.keys`)

Selain cookie HttpOnly, HP menyimpan kunci kliennya di `localStorage`. Efeknya:
setelah **keluar**, tap kartu perangkat langsung masuk lagi tanpa pairing baru.
Kunci itu hanya dihapus bila kamu memilih *Lupakan perangkat* atau menghapus
perangkat dari daftar di app desktop.

## Mode ketat

`requirePairing: true` menonaktifkan login ID 9 digit. Yang diterima hanya:

- perangkat hasil pairing, atau
- token panjang (`data/token`)

## Revoke

```sh
# satu perangkat
POST /api/pair/revoke   {"id": "c…"}

# semua
POST /api/pair/revoke   {"all": true}
```

Efek langsung: request dengan cookie lama menerima `401`.

## Endpoint

| Endpoint | Akses |
|---|---|
| `POST /api/pair/new` | loopback / paired |
| `GET /desktop/setup-qr.svg?c=` | loopback / session |
| `GET /pair` | publik (dengan `?k=`) |
| `POST /api/pair/claim` | publik (dengan cookie `ra_device`) |
| `GET /api/pair/clients` | loopback / paired |
| `POST /api/pair/revoke` | loopback / paired |

## Yang masih terlihat relay

- client key **saat** `/api/pair/claim` (TLS berhenti di VPS)
- metadata: IP, waktu, ukuran trafik, Mac mana yang aktif

Untuk menghilangkan yang pertama, langkah berikutnya adalah menaruh public key
Mac di QR dan mengenkripsi claim sehingga relay menjadi pipa buta.

## Masalah umum

| Gejala | Penyebab |
|---|---|
| "Kode salah atau kedaluwarsa" padahal baru dibuat | kode sudah dipakai proses lain, atau > 5 menit |
| HP masuk tapi berhenti di `connecting` | cookie `ra_device` tidak ada → `/api/*` ditolak |
| Scan QR berhenti di halaman kosong | link tanpa `?k=` sehingga relay tidak bisa memilih Mac |
| Kartu perangkat tertulis "perlu pairing" | `ra.keys` tidak punya kunci untuk perangkat itu |
