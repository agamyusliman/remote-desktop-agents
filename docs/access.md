# Akses & token

Token memberi kendali penuh atas Mac. Perlakukan seperti password.

## Melihat akses saat ini

Dari Mac (bukan dari HP):

```sh
./scripts/show-access.sh
```

Output: ID 9 digit, token, URL LAN, URL WAN.

## Login

Tiga cara:

| Cara | Cocok untuk |
|---|---|
| Scan QR dari app desktop | pemakaian normal |
| ID 9 digit | tanpa kamera |
| Token panjang (`data/token`) | darurat / otomatis |

Buka `/auth?id=<id>` atau `/auth?token=<token>` sekali; cookie HttpOnly
dipasang dan berlaku `SESSION_TTL_DAYS`.

## Login lewat pairing

Kalau `requirePairing: true`, ID tidak berlaku. Yang diterima hanya klien hasil
pairing atau token panjang. Lihat [pairing.md](pairing.md).

## Rotasi token (logout semua perangkat)

```sh
./scripts/rotate-token.sh --restart
```

Yang terjadi:

1. `data/token` ditulis ulang
2. `data/secret` dihapus → semua cookie lama langsung tidak valid
3. server dijalankan ulang
4. URL login baru dicetak

## Mencabut akses

Keamanan dikelola dari **app desktop**, bukan dari HP. Alasannya: HP yang hilang
tidak bisa dipercaya untuk mencabut dirinya sendiri.

| Aksi | Di mana | Efek |
|---|---|---|
| Ganti perangkat | Settings HP → **Perangkat** → Ganti | pindah ke Mac lain di relay |
| Lupakan perangkat | app desktop → daftar perangkat | kunci HP itu dicabut |
| Putuskan semua | app desktop → daftar perangkat | semua kunci dicabut + token dirotasi |
| Rotasi manual | `./scripts/rotate-token.sh --restart` | semua cookie & pairing tidak valid |

Menghapus data situs di HP **tidak** mencabut apa pun di server — kuncinya masih
tersimpan di `clients.json`. Untuk benar-benar memutus, hapus dari app desktop.

Preferensi tampilan (`ra.view`, `ra.sidebar`, `ra.prefs`, `ra.project`) dan kunci
tersimpan (`ra.keys`) adalah state perangkat, bukan kredensial sesi, jadi tidak
ikut terhapus saat sesi berakhir.

## Masa berlaku

Cookie ditandatangani HMAC dan berlaku `SESSION_TTL_DAYS` (default 365).
Login ulang memperpanjang masa berlaku. Ubah di
Settings → Sesi & perangkat.

## Lupa semuanya

Kalau token dan ID hilang dan tidak ada HP yang masih login:

```sh
rm ~/Library/Application\ Support/Remote\ Agents/secret
./scripts/rotate-token.sh --restart
```

App akan membuat secret baru dan mencetak akses baru.