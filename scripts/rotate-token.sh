#!/bin/zsh
# Rotate device token, keep the short ID, revoke all cookies, optionally restart.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
mkdir -p data

python3 - <<'PY'
import secrets, base64, pathlib
d = pathlib.Path("data"); d.mkdir(exist_ok=True)
(d / "token").write_text(base64.urlsafe_b64encode(secrets.token_bytes(24)).decode().rstrip("="))
(d / "secret").unlink(missing_ok=True)
for p in (d / "token",):
    p.chmod(0o600)
print("ID   :", (d / "id").read_text().strip() if (d / "id").exists() else "—")
print("Token:", (d / "token").read_text())
PY

echo "All old cookies are no longer valid."
if [[ "${1:-}" == "--restart" ]]; then
  pkill -f 'target/release/remot-agent' 2>/dev/null || true
  sleep 1
  nohup ./target/release/remot-agent >> data/server.log 2>&1 &
fi
