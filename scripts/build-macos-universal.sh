#!/usr/bin/env bash
# Build a macOS universal (arm64 + x86_64) dylib.
# Must run on a macOS host with both Rust targets installed.
#
# PyO3 cross-compilation notes:
#   arm64 build : native; PYO3_PYTHON is used if set, otherwise uv python find 3.12.
#   x86_64 build: cross-compiled from arm64 host. Requires either a universal2
#                 Python (actions/setup-python provides one on macos-latest) or
#                 PYO3_CROSS_LIB_DIR pointing at an x86_64 Python lib directory.
#                 PYO3_CROSS_PYTHON_VERSION=3.12 is set here so PyO3 does not
#                 attempt to detect the version by running the arm64 Python binary.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

ARM="aarch64-apple-darwin"
X86="x86_64-apple-darwin"

# If PYO3_PYTHON is not already set (local dev), locate Python 3.12 via uv.
if [[ -z "${PYO3_PYTHON:-}" ]]; then
  if command -v uv &>/dev/null; then
    PY_EXE="$(uv python find 3.12 2>/dev/null || true)"
    if [[ -n "$PY_EXE" ]]; then
      export PYO3_PYTHON="$PY_EXE"
      echo "==> Python (uv): $PY_EXE"
    fi
  fi
fi

echo "==> Building arm64 (native)..."
cargo build -p unity_dlp_core --release --target "$ARM"

echo "==> Building x86_64 (cross)..."
# PYO3_CROSS_PYTHON_VERSION prevents PyO3 from running the arm64 Python binary
# to detect the version.  The linker uses whichever Python lib is in PYO3_PYTHON
# (universal2 on CI) or PYO3_CROSS_LIB_DIR if set explicitly.
PYO3_CROSS_PYTHON_VERSION=3.12 \
  cargo build -p unity_dlp_core --release --target "$X86"

echo "==> Lipo into universal binary..."
lipo -create \
  "target/$ARM/release/libunity_dlp.dylib" \
  "target/$X86/release/libunity_dlp.dylib" \
  -output "unity_dlp.dylib"

DEST="unity_package/Plugins/x86_64"
mkdir -p "$DEST"
mv unity_dlp.dylib "$DEST/"
echo "==> Universal dylib staged to $DEST/unity_dlp.dylib"
