# Automated agent conventions

These notes apply to automated agents (omp, Claude, and similar) working in this
repo. This project is **Rust + JavaScript with no bundler**, so the rules below
only apply when you add new JS/TS tooling.

## Default tooling

Use Bun, not Node/npm.

- `bun <file>` instead of `node <file>` or `ts-node <file>`
- `bun test` instead of `jest` / `vitest`
- `bun build <file>` instead of `webpack` / `esbuild`
- `bun install` instead of `npm install` / `yarn` / `pnpm`
- `bun run <script>` instead of `npm run`
- `bunx <pkg>` instead of `npx`
- Bun loads `.env` automatically; don't add dotenv.

## API

- `Bun.serve()` supports WebSocket, HTTPS, and routing. Don't use `express`.
- `bun:sqlite` instead of `better-sqlite3`.
- `Bun.redis` instead of `ioredis`.
- `Bun.sql` instead of `pg` / `postgres.js`.
- `WebSocket` is built in; don't use `ws`.
- `Bun.file` is preferred over `readFile`/`writeFile` from `node:fs`.
- `Bun.$` instead of `execa`.

## Frontend

If you add new UI, prefer HTML imports from `Bun.serve()`; don't use Vite. For
the existing PWA shell, keep editing `public/` directly — this project has no
build step.

## Testing

`bun test` for JS tests.

```ts
import { test, expect } from "bun:test";

test("hello world", () => {
  expect(1).toBe(1);
});
```

For Rust code, use `cargo test` and `cargo clippy`.

## What not to do

- Add a bundler or build step to `public/`.
- Change `sw.js` without bumping the version (the version is now derived from a
  hash at deploy time, but don't disable that mechanism).
- Store credentials, tokens, or personal paths in the repo.
