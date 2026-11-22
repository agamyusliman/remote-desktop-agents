#!/bin/zsh
# Create a stable self-signed code-signing identity for local builds.
#
# Why: macOS TCC keys Accessibility/Screen-Recording grants to the code
# signature. An ad-hoc signature changes its cdhash on every rebuild, so the
# grant is silently invalidated. A self-signed certificate gives a stable
# designated requirement, so permission survives rebuilds.
set -eu
NAME="Remote Agents Dev"
KEYCHAIN="$HOME/Library/Keychains/remote-agents-signing.keychain-db"
PASS="remote-agents-local"

if security find-identity -v -p codesigning 2>/dev/null | grep -q "$NAME"; then
  echo "identity already present: $NAME"
  exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

cat > "$TMP/openssl.cnf" <<CNF
[ req ]
distinguished_name = dn
x509_extensions = ext
prompt = no
[ dn ]
CN = $NAME
[ ext ]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
CNF

openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -keyout "$TMP/key.pem" -out "$TMP/cert.pem" -config "$TMP/openssl.cnf" >/dev/null 2>&1
openssl pkcs12 -export -inkey "$TMP/key.pem" -in "$TMP/cert.pem" \
  -out "$TMP/id.p12" -passout pass:"$PASS" -name "$NAME" >/dev/null 2>&1

if [[ ! -f "$KEYCHAIN" ]]; then
  security create-keychain -p "$PASS" "$KEYCHAIN"
fi
security unlock-keychain -p "$PASS" "$KEYCHAIN"
security import "$TMP/id.p12" -k "$KEYCHAIN" -P "$PASS" -T /usr/bin/codesign -A >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$PASS" "$KEYCHAIN" >/dev/null 2>&1 || true

# Make it usable by codesign without unlocking manually.
if ! security list-keychains -d user | grep -q "remote-agents-signing"; then
  security list-keychains -d user -s $(security list-keychains -d user | tr -d '"') "$KEYCHAIN"
fi
security find-identity -v -p codesigning "$KEYCHAIN" | tail -2
echo
echo "Signed builds will now keep their Accessibility grant across rebuilds."
