#!/bin/zsh
# Keeps a Cloudflare quick tunnel alive; writes URL to data/tunnel-url.
# For a stable URL use a named tunnel instead (see tunnel.sh).
set -u
PORT="${PORT:-8788}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/data/tunnel-url"
LOG="$ROOT/data/tunnel.log"
mkdir -p "$ROOT/data"
while true; do
  cloudflared tunnel --url "http://127.0.0.1:$PORT" --no-autoupdate 2>&1 |
    while IFS= read -r line; do
      print -r -- "$line" >> "$LOG"
      if [[ "$line" =~ https://([a-z0-9-]+)\.trycloudflare\.com ]]; then
        print -r -- "https://${match[1]}.trycloudflare.com" > "$OUT"
      fi
    done
  sleep 3
done
