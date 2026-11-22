#!/bin/zsh
# Finish a named Cloudflare tunnel for this app.
# Usage: ./scripts/setup-named-tunnel.sh <hostname> [tunnel-name]
# Requires: cloudflared tunnel login (once, opens browser)
set -eu
HOSTNAME="${1:?usage: setup-named-tunnel.sh remote.example.com [name]}"
NAME="${2:-remote-agents}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT="${PORT:-8788}"

if [[ ! -f "$HOME/.cloudflared/cert.pem" ]]; then
  echo "Belum login. Jalankan dulu: cloudflared tunnel login"
  exit 1
fi

cloudflared tunnel create "$NAME" 2>/dev/null || true
TUNNEL_ID="$(cloudflared tunnel list --output json | python3 -c "import json,sys; d=json.load(sys.stdin); print(next((t['id'] for t in d if t['name']=='$NAME'),''))")"
[[ -n "$TUNNEL_ID" ]] || { echo "tunnel $NAME tidak ditemukan"; exit 1; }

cloudflared tunnel route dns "$NAME" "$HOSTNAME"

mkdir -p "$HOME/.cloudflared"
cat > "$HOME/.cloudflared/$NAME.yml" <<YAML
tunnel: $TUNNEL_ID
credentials-file: $HOME/.cloudflared/$TUNNEL_ID.json
ingress:
  - hostname: $HOSTNAME
    service: http://127.0.0.1:$PORT
  - service: http_status:404
YAML

echo "Tunnel $NAME -> $HOSTNAME"
echo "Jalankan: cloudflared tunnel run $NAME"
echo "Atau:     ./scripts/tunnel.sh --named $NAME"
echo
echo "Set PUBLIC_URL=https://$HOSTNAME di .env agar tampil di Settings."
