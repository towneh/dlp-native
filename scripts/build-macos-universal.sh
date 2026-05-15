#!/usr/bin/env bash
# Build a macOS universal (arm64 + x86_64) dylib.
# Must run on a macOS host with both Rust targets installed.
#
# PyO3 cross-compilation notes:
#   arm64 build : native; PYO3_PYTHON is used if set, otherwise uv python find 3.12.
#   x86_64 build: cross-compiled from arm64 host. PYO3_CROSS_PYTHON_VERSION=3.12 tells
#                 PyO3 not to interrogate the arm64 Python binary for the version.
#                 PYO3_CROSS_LIB_DIR is derived from the Python prefix so the linker
#                 can find libpython3.12 (works for both the Python.org universal2
#                 framework and Homebrew/uv installs).
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

# Derive the lib directory from the Python prefix for PYO3_CROSS_LIB_DIR.
# This ensures the linker's -lpython3.12 search has a valid -L path during
# the x86_64 cross-compile step.
PY_PREFIX="$("${PYO3_PYTHON:-python3}" -c 'import sys; print(sys.prefix)' 2>/dev/null || true)"
PY_LIB_DIR="${PY_PREFIX}/lib"
echo "==> Python prefix : $PY_PREFIX"
echo "==> PYO3_CROSS_LIB_DIR : $PY_LIB_DIR"

echo "==> Building arm64 (native)..."
cargo build -p unity_dlp_core --release --target "$ARM"

echo "==> Building x86_64 (cross)..."
# PYO3_CROSS_PYTHON_VERSION: skip arm64 Python binary interrogation.
# PYO3_CROSS_LIB_DIR: tell the linker where libpython3.12 lives.
# The Python.org universal2 framework dylib at $PY_PREFIX/lib/libpython3.12.dylib
# contains both arm64 and x86_64 slices, so the linker picks the right one.
PYO3_CROSS_PYTHON_VERSION=3.12 PYO3_CROSS_LIB_DIR="${PY_LIB_DIR}" \
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
