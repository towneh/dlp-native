#!/usr/bin/env bash
# Build the native library for the host platform (Linux / macOS).
# Outputs the shared library and copies it to unity_package/Plugins/x86_64/.
#
# Requirements:
#   - Rust toolchain (see rust-toolchain.toml)
#   - uv with Python 3.12 installed  (`uv python install 3.12`)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# ── Locate Python 3.12 via uv ─────────────────────────────────────────────────
echo "==> Locating Python 3.12 via uv..."
PY_EXE="$(uv python find 3.12)"
if [[ -z "$PY_EXE" ]]; then
  echo "ERROR: Python 3.12 not found via uv. Run: uv python install 3.12" >&2
  exit 1
fi
echo "    Python: $PY_EXE"
export PYO3_PYTHON="$PY_EXE"

# ── Build ─────────────────────────────────────────────────────────────────────
echo "==> Building unity_dlp_core (release, host target)..."
cargo build -p unity_dlp_core --release

# ── Stage to Unity Plugins ────────────────────────────────────────────────────
case "$(uname -s)" in
  Linux*)  LIB="target/release/libunity_dlp.so" ;;
  Darwin*) LIB="target/release/libunity_dlp.dylib" ;;
  MINGW*|MSYS*|CYGWIN*) LIB="target/release/unity_dlp.dll" ;;
  *)
    echo "ERROR: Unknown OS '$(uname -s)'" >&2
    exit 1
    ;;
esac

DEST="unity_package/Plugins/x86_64"
mkdir -p "$DEST"
cp "$LIB" "$DEST/"
echo "==> Staged: $LIB → $DEST/"
echo ""
echo "Build complete."
