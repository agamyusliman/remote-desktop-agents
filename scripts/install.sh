#!/bin/zsh
# One-command setup for non-developers (macOS).
# Usage: ./scripts/install.sh
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

say() { printf "\n\033[1;36m%s\033[0m\n" "$*"; }
ok()  { printf "  \033[32m✓\033[0m %s\n" "$*"; }
warn(){ printf "  \033[33m!\033[0m %s\n" "$*"; }
die() { printf "\n\033[31m%s\033[0m\n" "$*"; exit 1; }

say "Remote Agents — installer"
echo "Folder: $ROOT"

# 1. Homebrew
if ! command -v brew >/dev/null 2>&1; then
  die "Homebrew belum ada. Install dari https://brew.sh lalu jalankan lagi."
fi
ok "Homebrew"

# 2. Dependencies
for pkg in omp tmux; do
  if command -v "$pkg" >/dev/null 2>&1; then
    ok "$pkg"
  else
    warn "$pkg belum ada — mencoba brew install…"
    brew install "$pkg" || die "Gagal install $pkg"
  fi
done

# 3. Data + config
mkdir -p data && chmod 700 data
if [[ ! -f .env ]]; then
  cp .env.example .env
  # Sensible defaults for a normal user
  PROJECT_DIR="$HOME/Documents/Ai"
  [[ -d "$PROJECT_DIR" ]] || PROJECT_DIR="$HOME"
  /usr/bin/sed -i '' "s|^OMP_CWD=.*|OMP_CWD=$PROJECT_DIR|" .env
  /usr/bin/sed -i '' "s|^ALLOWED_ROOTS=.*|ALLOWED_ROOTS=$HOME|" .env
  ok ".env dibuat (project: $PROJECT_DIR)"
else
  ok ".env sudah ada"
fi

# 4. Build
say "Build (beberapa menit pertama kali)…"
cargo build --release
ok "Binary: target/release/remot-agent"

# 5. Token / ID
set -a; source .env; set +a
./target/release/remot-agent --print-identity 2>/dev/null || true
ID="$(cat data/id 2>/dev/null || echo '—')"
ok "Device ID: $ID"

# 6. macOS app bundle
if [[ "$(uname)" == "Darwin" ]]; then
  ./scripts/package-macos.sh >/dev/null 2>&1 && ok "App: dist/Remote Agents.app" || warn "Bundle .app gagal dibuat"
fi

say "Selesai."
cat <<TXT
  1. Buka app:  open "dist/Remote Agents.app"
     (atau: ./target/release/remot-agent)
  2. Di Mac, izinkan Screen Recording + Accessibility saat diminta.
  3. Scan QR dari HP, atau buka URL LAN dan masukkan ID.
  4. Untuk akses dari luar rumah, lihat README bagian "Remote access".
TXT
