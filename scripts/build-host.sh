#!/usr/bin/env bash
# Build the native library for the host platform.
# Outputs the shared library next to this script in ../target/release/.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "==> Building unity_dlp_core (release, host target)..."
cargo build -p unity_dlp_core --release

# Determine output name by OS.
case "$(uname -s)" in
  Linux*)  LIB="target/release/libunity_dlp.so" ;;
  Darwin*) LIB="target/release/libunity_dlp.dylib" ;;
  MINGW*|MSYS*|CYGWIN*) LIB="target/release/unity_dlp.dll" ;;
  *)
    echo "ERROR: Unknown OS '$(uname -s)'" >&2
    exit 1
    ;;
esac

echo "==> Built: $LIB"

# Copy to unity_package Plugins folder.
DEST="unity_package/Plugins/x86_64"
mkdir -p "$DEST"
cp "$LIB" "$DEST/"
echo "==> Staged to $DEST/"
