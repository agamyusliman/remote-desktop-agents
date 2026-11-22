#!/bin/zsh
# Bring your own tunnel. Nothing here is tied to a specific account.
#
# A) Cloudflare quick tunnel (no account, random URL):
#      ./scripts/tunnel.sh
# B) Cloudflare named tunnel (stable hostname, your own account):
#      cloudflared tunnel login
#      cloudflared tunnel create remot-agent
#      cloudflared tunnel route dns remot-agent remot-agent.example.com
#      ./scripts/tunnel.sh --named remot-agent
# C) Tailscale (private, no public URL):
#      tailscale serve 8788
set -eu
PORT="${PORT:-8788}"

case "${1:-}" in
  --named)
    NAME="${2:?usage: tunnel.sh --named <tunnel-name>}"
    exec cloudflared tunnel --no-autoupdate run "$NAME" --url "http://127.0.0.1:$PORT"
    ;;
  "")
    exec cloudflared tunnel --url "http://127.0.0.1:$PORT" --no-autoupdate
    ;;
  *)
    echo "usage: $0 [--named <tunnel-name>]" >&2
    exit 2
    ;;
esac
