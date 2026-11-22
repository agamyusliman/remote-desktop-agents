# Mulai

Tujuan: dari install sampai melihat layar Mac di HP, tanpa terminal.

## 1. Install app desktop

Build sendiri:

```sh
./scripts/package-macos.sh --install   # → /Applications/Remote Agents.app
```

Atau buka DMG hasil `./scripts/package-dmg.sh`, lalu drag ke Applications.

## 2. Beri izin macOS

Saat pertama dibuka, macOS meminta dua izin. Keduanya wajib:

| Izin | Untuk apa | Kalau ditolak |
|---|---|---|
| **Screen Recording** | menangkap layar | layar hitam, `capture.error` di `/api/status` |
| **Accessibility** | mouse & keyboard | `input.trusted: false`, klik tidak berpengaruh |

UI Setup menyediakan tombol "Buka pengaturan" untuk keduanya. Setelah mengubah
izin, **app harus dijalankan ulang** — TCC tidak berlaku pada proses yang sudah
hidup.

## 3. Buat kode pairing

Di app desktop, tab **Hubungkan** → **Buat kode pairing**.

Muncul QR dan kode 6 karakter. Kode:

- sekali pakai
- berlaku 5 menit
- hanya bisa dipakai satu perangkat

## 4. Hubungkan dari HP

Buka URL relay, lalu salah satu:

- **Scan QR** — kamera, paling andal karena QR membawa `?k=` yang menentukan Mac mana
- **Ketik kode** — `ABC-123`

Setelah terhubung, HP menyimpan kunci akses sendiri. Pairing ulang tidak perlu.

## 5. Install sebagai app di HP

- Android Chrome: menu ⋮ → **Install app**
- iPhone Safari: Share → **Add to Home Screen**

PWA punya service worker yang menyimpan hanya shell UI (bukan API, layar, atau
terminal), jadi tetap bisa dibuka saat Mac sedang mati.

## Alur UI di HP

Semua ada di satu halaman — tidak ada halaman terpisah untuk pairing.

| Layar | Isi |
|---|---|
| **Home** | status perangkat, CPU/RAM/disk/net, project terakhir |
| **Projects** | daftar project + tombol `+` di kanan atas header |
| **Remote** | toolbar `CLI / Screen / Files / Split`, ikon sesi, nama project, reload, keyboard |
| **System** | proses dan metrik |
| **Settings** | FPS, kualitas, ukuran font, ganti perangkat |

Beberapa kebiasaan yang perlu diketahui:

- **Tambah project** = tap `+` lalu pilih folder. Nama project diambil otomatis
  dari nama folder; tidak ada kolom nama.
- **Hapus project** = swipe kartu ke kiri, tap ikon tempat sampah. Hanya dihapus
  dari daftar, folder di disk tidak disentuh.
- **Ganti sesi** = tap ikon jam di toolbar Remote. Daftar sesi muncul menimpa
  terminal, jadi tidak perlu keluar project atau mengetik `/resume`.
- **Rename/hapus sesi** = ikon pensil / tempat sampah di setiap baris sesi.
  Rename menulis ke berkas sesi, jadi omp juga memakai nama baru itu.

## Di mana data disimpan

| Hal | Lokasi |
|---|---|
| Data app desktop | `~/Library/Application Support/Remote Agents/` |
| Binary relay-link | `…/Remote Agents/bin/` |
| Kunci perangkat (`clients.json`) | `…/Remote Agents/clients.json` |
| Token panjang | `…/Remote Agents/token` |
| Secret HMAC | `…/Remote Agents/secret` |
| Mode developer | `./data/` di root project |

`./data` lama disalin otomatis saat pertama kali app dijalankan.

## Mode developer

```sh
./scripts/setup.sh          # buat data/, token, .env
$EDITOR .env                # atur OMP_CWD / ALLOWED_ROOTS
cargo build --release
set -a; source .env; set +a
./target/release/remot-agent
```

Buka URL yang dicetak. Lihat [configuration.md](configuration.md) untuk semua
variabel.

## Membuka dari luar rumah

Jangan buka port `8788` ke internet. Pakai salah satu:

| Cara | Perintah | Catatan |
|---|---|---|
| Relay sendiri | `./scripts/relay-setup-nginx.sh root@vps domainmu.com` | satu hostname untuk beberapa Mac, lihat [relay.md](relay.md) |
| Tailscale | `tailscale serve 8788` | privat, tanpa domain |
| Cloudflare named tunnel | `./scripts/setup-named-tunnel.sh remote.domainmu.com` | URL tetap |
| Quick tunnel | `./scripts/tunnel.sh` | URL acak, untuk uji coba |

## Autostart

Toggle di UI Setup. Manual:

```sh
cp scripts/sh.remot-agent.plist.example ~/Library/LaunchAgents/
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/sh.remot-agent.plist
```

## Berikutnya

Pairing gagal? Lihat [pairing.md](pairing.md) dan
[troubleshooting.md](troubleshooting.md).
