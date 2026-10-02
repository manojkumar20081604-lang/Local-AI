#!/usr/bin/env bash
# Install the Local-AI CLI so `local-ai` works from any directory.
#
#   ./install.sh              # user install → ~/.local/bin (no sudo needed)
#   ./install.sh --system     # system install → /usr/local/bin (needs sudo)
#   BINDIR=/custom/dir ./install.sh   # install anywhere
#   ./install.sh --uninstall  # remove the installed binary
#
# One-liner (no clone needed — the script fetches the repo itself):
#   curl -fsSL https://raw.githubusercontent.com/manojkumar20081604-lang/Local-AI/main/install.sh | sh
#
# Alternative (same result, needs Rust): cargo install --path cli
set -eu
if [ -n "${BASH_VERSION:-}" ]; then set -o pipefail; fi

REPO_URL="https://github.com/manojkumar20081604-lang/Local-AI.git"
# $0-based (POSIX-safe: works under bash AND sh, local file AND curl pipe).
REPO_ROOT=""
if [ -n "${0:-}" ] && [ -f "$(dirname "$0")/cli/Cargo.toml" ] 2>/dev/null; then
    REPO_ROOT="$(cd "$(dirname "$0")" && pwd)"
fi
if [ -z "$REPO_ROOT" ] && [ -f "cli/Cargo.toml" ]; then
    REPO_ROOT="$(pwd)"
fi
if [ ! -f "$REPO_ROOT/cli/Cargo.toml" ]; then
    # Piped via curl (no repo on disk) — clone to a temp dir first.
    command -v git >/dev/null 2>&1 || {
        echo "error: 'git' not found. Install git first: https://git-scm.com" >&2
        exit 1
    }
    REPO_ROOT="$(mktemp -d)/Local-AI"
    echo "Fetching Local-AI…"
    git clone --depth 1 "$REPO_URL" "$REPO_ROOT"
    CLEANUP_CLONE=1
else
    CLEANUP_CLONE=0
fi
trap 'if [ "${CLEANUP_CLONE:-0}" -eq 1 ]; then rm -rf "$REPO_ROOT"; fi' EXIT
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
