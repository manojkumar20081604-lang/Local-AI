#!/usr/bin/env bash
# Install the Local-AI CLI so `local-ai` works from any directory.
#
#   ./install.sh              # user install → ~/.local/bin (no sudo needed)
#   ./install.sh --system     # system install → /usr/local/bin (needs sudo)
#   BINDIR=/custom/dir ./install.sh   # install anywhere
#   ./install.sh --uninstall  # remove the installed binary
#
# Alternative (same result, needs Rust): cargo install --path cli
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_NAME="local-ai"

usage() {
    echo "Usage: ./install.sh [--system] [--uninstall] [--help]"
    echo "  (default)   install to ~/.local/bin (add it to PATH if needed)"
    echo "  --system    install to /usr/local/bin (needs sudo)"
    echo "  --uninstall remove the installed binary"
}

UNINSTALL=0
SYSTEM=0
for arg in "$@"; do
    case "$arg" in
        --uninstall) UNINSTALL=1 ;;
        --system) SYSTEM=1 ;;
        --help|-h) usage; exit 0 ;;
        *) echo "Unknown option: $arg"; usage; exit 1 ;;
    esac
done

if [ -n "${BINDIR:-}" ]; then
    DEST="$BINDIR"
elif [ "$SYSTEM" -eq 1 ]; then
    DEST="/usr/local/bin"
else
    DEST="$HOME/.local/bin"
fi

if [ "$UNINSTALL" -eq 1 ]; then
    rm -f "$DEST/$BIN_NAME"
    echo "Removed $DEST/$BIN_NAME (if it existed)"
    exit 0
fi

command -v cargo >/dev/null 2>&1 || {
    echo "error: 'cargo' not found. Install Rust first: https://rustup.rs" >&2
    exit 1
}

echo "Building $BIN_NAME (release, one-time, a few minutes)…"
cargo build --release --manifest-path "$REPO_ROOT/cli/Cargo.toml"

mkdir -p "$DEST"
cp "$REPO_ROOT/cli/target/release/$BIN_NAME" "$DEST/$BIN_NAME"
echo "Installed → $DEST/$BIN_NAME"

# PATH hint for user installs
if ! command -v "$BIN_NAME" >/dev/null 2>&1; then
    echo ""
    echo "'$DEST' is not on your PATH. Add it once, then restart the terminal:"
    echo "  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc"
    echo "  (fish: fish_add_path ~/.local/bin)"
    echo ""
    echo "Or run it directly meanwhile: $DEST/$BIN_NAME --help"
else
    "$BIN_NAME" --help >/dev/null
    echo "Verified: '$BIN_NAME' runs from anywhere. Try: $BIN_NAME doctor"
fi
