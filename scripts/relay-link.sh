#!/bin/zsh
# Keep a reverse SSH tunnel to the relay host.
# Usage: ./scripts/relay-link.sh [relay-host] [remote-port] [local-port]
set -eu
RELAY="${1:-root@relay.example.com}"
RPORT="${2:-8788}"
LPORT="${3:-8788}"
KEY="${RELAY_SSH_KEY:-$HOME/.ssh/ra_tunnel}"
while true; do
  ssh -N \
    -o ExitOnForwardFailure=yes \
    -o ServerAliveInterval=30 \
    -o ServerAliveCountMax=3 \
    -o StrictHostKeyChecking=accept-new \
    -i "$KEY" \
    -R "127.0.0.1:${RPORT}:127.0.0.1:${LPORT}" \
    "$RELAY" || true
  # The relay or the network dropped us: back off briefly, then reconnect.
  sleep 3
done
