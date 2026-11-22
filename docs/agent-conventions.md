# Konvensi agen otomatis

Catatan ini berlaku untuk agen otomatis (omp, Claude, dan sejenisnya) yang
bekerja di repo ini. Proyek ini **Rust + JavaScript tanpa bundler**, jadi aturan
di bawah hanya dipakai bila kamu menambahkan tooling JS/TS baru.

## Default tooling

Pakai Bun, bukan Node/npm.

- `bun <file>` alih-alih `node <file>` atau `ts-node <file>`
- `bun test` alih-alih `jest` / `vitest`
- `bun build <file>` alih-alih `webpack` / `esbuild`
- `bun install` alih-alih `npm install` / `yarn` / `pnpm`
- `bun run <script>` alih-alih `npm run`
- `bunx <pkg>` alih-alih `npx`
- Bun memuat `.env` otomatis; jangan tambahkan dotenv.

## API

- `Bun.serve()` mendukung WebSocket, HTTPS, dan routing. Jangan pakai `express`.
- `bun:sqlite` alih-alih `better-sqlite3`.
- `Bun.redis` alih-alih `ioredis`.
- `Bun.sql` alih-alih `pg` / `postgres.js`.
- `WebSocket` sudah bawaan; jangan pakai `ws`.
- `Bun.file` lebih disukai daripada `readFile`/`writeFile` dari `node:fs`.
- `Bun.$` alih-alih `execa`.

## Frontend

Kalau menambah UI baru, utamakan HTML import dari `Bun.serve()`; jangan pakai
Vite. Untuk shell PWA yang ada, tetap sunting `public/` langsung — proyek ini
tidak memakai build step.

## Testing

`bun test` untuk tes JS.

```ts
import { test, expect } from "bun:test";

test("hello world", () => {
  expect(1).toBe(1);
});
```

Untuk kode Rust, gunakan `cargo test` dan `cargo clippy`.

## Yang tidak boleh dilakukan

- Menambah bundler atau build step ke `public/`.
- Mengubah `sw.js` tanpa menaikkan versi (versi kini diturunkan dari hash saat
  deploy, tetapi jangan menonaktifkan mekanisme itu).
- Menyimpan kredensial, token, atau path pribadi di repo.
