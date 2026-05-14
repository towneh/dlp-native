#!/usr/bin/env bash
# Cross-compile for Android arm64-v8a and armeabi-v7a using cargo-ndk.
# Requires: cargo-ndk installed, Android NDK available.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

: "${ANDROID_NDK_HOME:?ANDROID_NDK_HOME must be set}"

echo "==> Building Android arm64-v8a..."
cargo ndk \
  --target aarch64-linux-android \
  --platform 26 \
  -- build -p unity_dlp_core --release

echo "==> Building Android armeabi-v7a..."
cargo ndk \
  --target armv7-linux-androideabi \
  --platform 26 \
  -- build -p unity_dlp_core --release

echo "==> Staging .so files..."
ARM64_DEST="unity_package/Plugins/Android/libs/arm64-v8a"
ARMV7_DEST="unity_package/Plugins/Android/libs/armeabi-v7a"
mkdir -p "$ARM64_DEST" "$ARMV7_DEST"

cp target/aarch64-linux-android/release/libunity_dlp.so "$ARM64_DEST/"
cp target/armv7-linux-androideabi/release/libunity_dlp.so "$ARMV7_DEST/"
echo "==> Android .so files staged."
