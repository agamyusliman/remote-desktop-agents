#!/bin/zsh
# Launch server + optional quick-tunnel at login.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
[[ -f .env ]] && { set -a; source .env; set +a; }
export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"

if ! pgrep -f 'target/release/remot-agent' >/dev/null 2>&1; then
  nohup "$ROOT/target/release/remot-agent" >> "$ROOT/data/server.log" 2>&1 &
fi

if [[ "${REMOT_AGENT_TUNNEL:-0}" == "1" ]] && ! pgrep -f 'scripts/tunnel-daemon.sh' >/dev/null 2>&1; then
  nohup "$ROOT/scripts/tunnel-daemon.sh" >> "$ROOT/data/tunnel.log" 2>&1 &
fi
