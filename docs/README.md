# Dokumentasi Remote Agents

Remote Agents adalah server Rust + PWA untuk mengendalikan satu Mac dari HP:
lihat layar, gerakkan mouse/keyboard, jalankan omp (chat & terminal), dan
menjelajahi file project.

## Mulai dari mana

| Kalau kamu… | Baca |
|---|---|
| baru pertama kali pakai | [getting-started.md](getting-started.md) |
| mau menghubungkan HP ke Mac | [pairing.md](pairing.md) |
| punya lebih dari satu Mac | [relay.md](relay.md) |
| ingin kontribusi kode | [contributing.md](contributing.md) |
| sedang ada masalah | [troubleshooting.md](troubleshooting.md) |

## Daftar isi

- **[getting-started.md](getting-started.md)** — install app, izin macOS, langkah pertama di HP.
- **[pairing.md](pairing.md)** — alur pairing, isi QR, cookie, revoke perangkat.
- **[relay.md](relay.md)** — satu hostname untuk beberapa Mac, routing, pemilih perangkat.
- **[access.md](access.md)** — token, rotasi, logout, akses darurat dari Mac.
- **[configuration.md](configuration.md)** — semua environment variable dan berkas di `data/`.
- **[architecture.md](architecture.md)** — struktur `src/`, alur data, WebSocket.
- **[api.md](api.md)** — daftar endpoint HTTP dan WebSocket.
- **[security.md](security.md)** — model ancaman dan batasan.
- **[troubleshooting.md](troubleshooting.md)** — gejala umum dan penyebabnya.
- **[contributing.md](contributing.md)** — aturan kode dan alur PR.
- **[agent-conventions.md](agent-conventions.md)** — konvensi tooling Bun untuk agen otomatis.

## Prinsip desain

1. **Tidak butuh akun cloud.** Relay hanya jalur awal; kunci akses disimpan di
   perangkat, bukan di server.
2. **Satu perangkat aktif.** Satu waktu hanya satu HP/laptop yang mengemudikan
   satu Mac, supaya dua pengendali tidak bertabrakan.
3. **PWA tanpa build step.** `public/` disajikan apa adanya; tidak ada bundler.
4. **Baterai diutamakan.** Capture berhenti saat tak ada penonton layar dan
   melewati frame yang identik.
