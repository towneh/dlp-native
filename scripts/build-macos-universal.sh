#!/usr/bin/env bash
# Build a macOS universal (arm64 + x86_64) dylib.
# Must run on a macOS host with both Rust targets installed.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

ARM="aarch64-apple-darwin"
X86="x86_64-apple-darwin"

echo "==> Building arm64..."
cargo build -p unity_dlp_core --release --target "$ARM"

echo "==> Building x86_64..."
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
