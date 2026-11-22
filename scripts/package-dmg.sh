#!/bin/zsh
# Create a drag-to-Applications DMG from the built .app bundle.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
APP="dist/Remote Agents.app"
OUT="dist/RemoteAgents.dmg"

if [[ ! -d "$APP" ]]; then
  echo "Run ./scripts/package-macos.sh first." >&2
  exit 1
fi

rm -rf dist/dmg "$OUT"
mkdir -p dist/dmg
cp -R "$APP" dist/dmg/
ln -sf /Applications dist/dmg/Applications
hdiutil create -volname "Remote Agents" -srcfolder dist/dmg -ov -format UDZO "$OUT" >/dev/null
rm -rf dist/dmg
echo "Built $OUT ($(du -h "$OUT" | cut -f1))"
