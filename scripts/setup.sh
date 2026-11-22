#!/bin/zsh
# First-run setup for a fresh clone. Creates data/, token, and prints access URLs.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

mkdir -p data
chmod 700 data

if [[ ! -f data/token ]]; then
  python3 - <<'PY'
import secrets, base64, pathlib
p = pathlib.Path("data/token")
if not p.exists():
    p.write_text(base64.urlsafe_b64encode(secrets.token_bytes(24)).decode().rstrip("="))
    p.chmod(0o600)
PY
fi

if [[ ! -f .env ]]; then
  cp .env.example .env
  echo "Created .env from .env.example — edit paths before running."
fi

echo
echo "Build:  cargo build --release"
echo "Run:    set -a; source .env; set +a; ./target/release/remot-agent"
echo
./scripts/show-access.sh
