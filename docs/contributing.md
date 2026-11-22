# Kontribusi

## Setup

```sh
git clone <repo> remote-desktop-agents
cd remote-desktop-agents
./scripts/setup.sh          # data/, token, .env
cargo build --release
set -a; source .env; set +a
./target/release/remot-agent
```

`./scripts/setup.sh` membuat `data/` privat berisi token acak dan menyalin
`.env.example` → `.env`. Jangan commit `data/` atau `.env`.

## Aturan kode

- Tidak ada hostname, URL tunnel, token, atau path pribadi yang di-hardcode.
- Konfigurasi baru wajib berupa env var yang didokumentasikan di `.env.example`
  dan [configuration.md](configuration.md).
- Setiap endpoint filesystem harus memeriksa `ALLOWED_ROOTS`.
- Service worker tidak boleh meng-cache `/api/*`, `/ws/*`, atau frame layar.
- Jalankan `cargo fmt` dan `cargo clippy` sebelum PR.

## Konvensi klien

- `public/` disajikan apa adanya. Tidak ada bundler, tidak ada build step.
- Satu fungsi, satu tanggung jawab. Hindari abstraksi yang belum perlu.
- Jangan menambah `setInterval` baru tanpa alasan; baterai adalah batasan nyata.
- Setiap socket harus punya pemilik tunggal; hindari koneksi ganda.

## Struktur

```
src/main.rs       route HTTP/WS, auth, API file & project
src/rpc.rs        jembatan omp RPC
src/capture.rs    loop capture (idle-aware)
src/input.rs      worker enigo
src/projects.rs   library project
src/sysmon.rs     sampler sistem
src/terminal.rs   PTY + tmux
src/auth.rs       cookie HMAC, rate limit
src/pairing.rs    kode pairing, client key
src/config.rs     env + settings
public/           shell PWA
scripts/          setup, rotasi token, tunnel, packaging
docs/             dokumentasi
```

## Menguji perubahan

Minimal verifikasi manual:

```sh
cargo build --release
./target/release/remot-agent
```

Lalu di browser: buka Remote, buka terminal, resume satu sesi, dan pastikan
terminal bertahan lebih dari 30 detik tanpa kembali ke picker.

Untuk perubahan relay:

```sh
./scripts/relay-setup-nginx.sh root@<host> <domain>
curl -s -o /dev/null -w "%{http_code}\n" https://<domain>/api/status
# 401 JSON {"ok":false,"error":"no_device"} = benar
# 200 text/html = routing rusak
```

## Commit

- Satu commit, satu alasan.
- Sebutkan gejala yang diperbaiki, bukan hanya berkas yang diubah.
- Jangan campur refactor dengan perbaikan bug.