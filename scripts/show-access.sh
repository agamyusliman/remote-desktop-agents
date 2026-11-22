#!/bin/zsh
# Print the short device ID, PIN, and access URLs (run on the Mac).
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ID_FILE="$ROOT/data/id"
TUNNEL_FILE="$ROOT/data/tunnel-url"
PORT="${PORT:-8788}"

[[ -f "$ID_FILE" ]] || { echo "Belum ada ID. Jalankan server sekali."; exit 1; }
ID="$(cat "$ID_FILE")"
LAN="$(ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1 2>/dev/null || echo 127.0.0.1)"

echo "Device ID : $ID"
echo "LAN       : http://$LAN:$PORT"
if [[ -f "$TUNNEL_FILE" ]]; then
  echo "WAN       : $(cat "$TUNNEL_FILE")"
fi
echo
echo "Login: buka URL, masukkan ID (atau scan QR di app desktop). Connect langsung."
