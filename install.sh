#!/bin/sh
# install.sh — build and install xsh.
#
# Usage:
#   ./install.sh                 install to ~/.local/bin (no sudo)
#   ./install.sh --system        also install to /usr/local/bin and
#                                 register it in /etc/shells (needs sudo)
#
# Can also be run remotely:
#   curl -fsSL https://raw.githubusercontent.com/furaxdev/xsh/master/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/furaxdev/xsh/master/install.sh | sh -s -- --system

set -eu

REPO_URL="https://github.com/furaxdev/xsh.git"
USER_BIN="$HOME/.local/bin"
SYSTEM_BIN="/usr/local/bin"
SYSTEM_INSTALL=0

for arg in "$@"; do
  case "$arg" in
    --system) SYSTEM_INSTALL=1 ;;
    *) echo "install.sh: unknown option: $arg" >&2; exit 1 ;;
  esac
done

say() { printf '\033[36m==>\033[0m %s\n' "$1"; }
warn() { printf '\033[33m!!\033[0m %s\n' "$1" >&2; }

# --- 1. make sure Rust is available ---------------------------------------
if ! command -v cargo >/dev/null 2>&1; then
  say "Rust n'est pas installé, installation via rustup..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
else
  say "Rust déjà présent ($(cargo --version))."
fi

# --- 2. locate (or fetch) the source ---------------------------------------
if [ -f "Cargo.toml" ] && grep -q '^name = "xsh"' Cargo.toml 2>/dev/null; then
  SRC_DIR="$(pwd)"
  say "Build depuis le répertoire courant ($SRC_DIR)."
  CLEANUP_SRC=0
else
  SRC_DIR="$(mktemp -d)"
  CLEANUP_SRC=1
  say "Récupération des sources dans $SRC_DIR..."
  if command -v git >/dev/null 2>&1; then
    git clone --depth 1 "$REPO_URL" "$SRC_DIR"
  else
    warn "git introuvable, impossible de récupérer les sources."
    exit 1
  fi
fi

# --- 3. build ----------------------------------------------------------------
say "Compilation en mode release (ça prend une minute)..."
( cd "$SRC_DIR" && cargo build --release )
BIN_PATH="$SRC_DIR/target/release/xsh"

if [ ! -x "$BIN_PATH" ]; then
  warn "Le binaire n'a pas été produit, abandon."
  exit 1
fi

# --- 4. install for the current user ----------------------------------------
mkdir -p "$USER_BIN"
install -m 755 "$BIN_PATH" "$USER_BIN/xsh"
say "Installé dans $USER_BIN/xsh"

case ":$PATH:" in
  *":$USER_BIN:"*) ;;
  *) warn "$USER_BIN n'est pas dans ton PATH — ajoute-le à ton .bashrc/.zshrc :"
     printf '     export PATH="%s:$PATH"\n' "$USER_BIN" ;;
esac

# --- 5. optional system-wide install ----------------------------------------
if [ "$SYSTEM_INSTALL" -eq 1 ]; then
  say "Installation système dans $SYSTEM_BIN (sudo requis)..."
  sudo install -m 755 "$BIN_PATH" "$SYSTEM_BIN/xsh"
  if ! grep -qx "$SYSTEM_BIN/xsh" /etc/shells 2>/dev/null; then
    echo "$SYSTEM_BIN/xsh" | sudo tee -a /etc/shells >/dev/null
    say "Ajouté $SYSTEM_BIN/xsh à /etc/shells."
  else
    say "$SYSTEM_BIN/xsh est déjà dans /etc/shells."
  fi
  say "Pour en faire ton shell de connexion : chsh -s $SYSTEM_BIN/xsh"
fi

if [ "$CLEANUP_SRC" -eq 1 ]; then
  rm -rf "$SRC_DIR"
fi

say "Terminé. Lance-le avec: xsh"
