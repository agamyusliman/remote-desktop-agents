#!/bin/zsh
# Build a double-clickable macOS .app bundle that needs zero terminal setup.
# Usage: ./scripts/package-macos.sh [--universal] [--sign "Developer ID Application: …"]
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
APP="dist/Remote Agents.app"
BIN="target/release/remot-agent"
UNIVERSAL=0
SIGN_ID=""
INSTALL=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --universal) UNIVERSAL=1; shift ;;
    --sign) SIGN_ID="${2:?--sign needs an identity}"; shift 2 ;;
    --install) INSTALL=1; shift ;;
    *) echo "unknown flag: $1"; exit 1 ;;
  esac
done

echo "Building release…"
if [[ "$UNIVERSAL" == "1" ]]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null 2>&1 || true
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  mkdir -p target/release
  lipo -create -output "$BIN" \
    target/aarch64-apple-darwin/release/remot-agent \
    target/x86_64-apple-darwin/release/remot-agent
else
  cargo build --release
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN" "$APP/Contents/MacOS/remot-agent"
cp public/icons/icon-512.png "$APP/Contents/Resources/icon.png"
cp public/icons/app.icns "$APP/Contents/Resources/AppIcon.icns"
cp public/icons/tray.png "$APP/Contents/Resources/tray.png"
cp public/icons/icon-96.png "$APP/Contents/Resources/icon-96.png"
cp public/icons/app.ico "$APP/Contents/Resources/app.ico" 2>/dev/null || true
cp -R public "$APP/Contents/Resources/public"
[[ -f .env.example ]] && cp .env.example "$APP/Contents/Resources/.env.example"

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Remote Agents</string>
  <key>CFBundleDisplayName</key><string>Remote Agents</string>
  <key>CFBundleIdentifier</key><string>dev.remoteagents.app</string>
  <key>CFBundleVersion</key><string>0.1.0</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleExecutable</key><string>remot-agent</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleIconName</key><string>AppIcon</string>
  <key>LSMinimumSystemVersion</key><string>12.3</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>Remote Agents</string>
  <key>NSScreenCaptureUsageDescription</key><string>Remote Agents mengirim layar ke perangkatmu sendiri.</string>
  <key>NSAccessibilityUsageDescription</key><string>Remote Agents memakai mouse/keyboard untuk mengendalikan perangkat ini.</string>
</dict>
</plist>
PLIST

# Bundle-scoped defaults read by the binary itself (see config::apply_bundle_defaults).
RELAY_URL="${REMOTE_AGENT_RELAY:-https://relay.example.com}"
printf '%s\n' "$RELAY_URL" > "$APP/Contents/Resources/relay-url"

# A stable signature is what lets macOS keep the Screen Recording /
# Accessibility grant across rebuilds. An ad-hoc signature changes its cdhash
# every build and silently loses the grant, so prefer the local dev identity.
LOCAL_ID="${REMOTE_AGENT_SIGN_ID:-}"
if [[ -z "$LOCAL_ID" ]]; then
  LOCAL_ID="$(security find-identity -v -p codesigning 2>/dev/null \
    | awk '/Remote Agents Dev/ {print $2; exit}')"
fi
if [[ -n "$LOCAL_ID" ]]; then
  echo "Signing with: $LOCAL_ID"
  # No hardened-runtime flag here: a bare binary has no resources, and
  # `--options runtime` makes codesign record an unsatisfiable requirement.
  codesign --force --sign "$LOCAL_ID" \
    --identifier "dev.remoteagents.app" "$APP/Contents/MacOS/remot-agent"
else
  echo "Warning: no stable identity; falling back to ad-hoc (grants reset each build)."
  echo "         Run ./scripts/setup-signing-cert.sh once to fix this."
  codesign --force --sign - --identifier "dev.remoteagents.app" \
    "$APP/Contents/MacOS/remot-agent"
fi

if [[ -n "$SIGN_ID" ]]; then
  echo "Signing with Developer ID…"
  codesign --force --options runtime --identifier "dev.remoteagents.app" \
    --timestamp --sign "$SIGN_ID" "$APP/Contents/MacOS/remot-agent"
  codesign --force --options runtime --timestamp --sign "$SIGN_ID" "$APP"
  codesign --verify --strict --verbose=2 "$APP"
  cat <<'NEXT'

Signed. To ship it to other Macs, also notarize:

  ditto -c -k --keepParent "dist/Remote Agents.app" dist/RemoteAgents.zip
  xcrun notarytool submit dist/RemoteAgents.zip \
    --keychain-profile "AC_NOTARY" --wait
  xcrun stapler staple "dist/Remote Agents.app"

Gatekeeper notes:
  - Screen Recording / Accessibility are per-app TCC permissions tied to the
    signing identity. Unsigned builds must be re-granted after each rebuild.
  - A DMG needs a Developer ID Application certificate + notary profile; an
    Ad-Hoc signature only works on the machine that built it.
NEXT
else
  echo
  echo "Built unsigned: dist/Remote Agents.app"
  echo "  - Runs on this Mac now (right-click → Open on first launch)."
  echo "  - Other Macs need: ./scripts/package-macos.sh --sign \"Developer ID Application: …\""
fi


if [[ "$INSTALL" == "1" ]]; then
  echo "Installing to /Applications…"
  rm -rf "/Applications/Remote Agents.app"
  cp -R "$APP" "/Applications/Remote Agents.app"
  xattr -dr com.apple.quarantine "/Applications/Remote Agents.app" 2>/dev/null || true
  # Keep only the installed copy: the build dir must not register a second app.
  rm -rf "$APP"
  echo "Installed: /Applications/Remote Agents.app"
fi

cat <<'DMG'

Create a DMG (drag-to-Applications):

  mkdir -p dist/dmg
  cp -R "dist/Remote Agents.app" dist/dmg/
  ln -sf /Applications "dist/dmg/Applications"
  hdiutil create -volname "Remote Agents" -srcfolder dist/dmg -ov -format UDZO \
    dist/RemoteAgents.dmg
DMG
