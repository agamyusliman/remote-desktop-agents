# Keamanan

Remote Agents memberi kendali penuh atas host kepada siapa pun yang memegang
kredensial. Perlakukan seperti remote shell.

## Model ancaman

| Aset | Pelindung |
|---|---|
| layar & input | cookie sesi bertanda tangan |
| terminal & chat | cookie sesi |
| berkas project | `ALLOWED_ROOTS` + kanonikalisasi |
| client key | hanya di perangkat, mode 600 |

## Aturan wajib

1. Bind ke `127.0.0.1` dan pakai tunnel, atau biarkan di LAN tepercaya.
2. **Jangan** port-forward port ke internet.
3. Token dan ID 9 digit setara password.
4. Rotasi setelah perangkat hilang: `./scripts/rotate-token.sh --restart`.
5. Review `clients.json` secara berkala; revoke yang tidak dikenal.

## Yang dilakukan sistem

- Cookie ditandatangani HMAC-SHA256, `HttpOnly`, `Secure`, `SameSite=Lax`.
- `secret` dibuat acak saat pertama jalan, mode 600.
- Kode pairing: sekali pakai, 5 menit, satu perangkat.
- Rate limit login per IP (`allow_attempt`).
- Path berkas dikanonikalisasi; symlink keluar `ALLOWED_ROOTS` ditolak.
- Service worker tidak pernah menyimpan `/api/*` atau `/ws/*`.
- Kode pairing tidak pernah dikirim ke relay (ada di fragment URL).

## Yang tidak dilindungi

| Risiko | Kenyataan |
|---|---|
| relay melihat client key saat claim | TLS berhenti di VPS |
| metadata relay | IP, waktu, ukuran trafik, Mac mana yang aktif |
| satu perangkat aktif | perangkat lain diputus, bukan ditolak diam-diam |
| `ra_device` bisa dipalsukan | cookie itu hanya memilih Mac, bukan autentikasi |
| TCC macOS | rebuild app memerlukan izin ulang |

## Distribusi

Tanpa Developer ID + notarization, Gatekeeper memblokir app di Mac lain:

```sh
./scripts/package-macos.sh --sign "Developer ID Application: Nama (TEAMID)"
```

Izin TCC (Screen Recording, Accessibility) terikat pada tanda tangan biner,
jadi setiap rebuild meminta izin ulang.

## Melaporkan masalah

Laporkan secara privat, jangan di issue publik.
