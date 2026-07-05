#!/usr/bin/env bash
# Cross-compile for Android arm64-v8a and armeabi-v7a using cargo-ndk.
#
# Requirements:
#   - cargo-ndk installed  (cargo install cargo-ndk)
#   - Android NDK (set ANDROID_NDK_HOME)
#   - PYO3_CROSS_LIB_DIR pointing to a directory containing:
#       libpython3.12.so  — from Termux's python .deb for the target arch
#       _sysconfigdata*.py — from $termux_prefix/lib/python3.12/
#   - libclang-dev installed (for rquickjs-sys bindgen)
#   - BINDGEN_EXTRA_CLANG_ARGS set to --sysroot=<ndk_sysroot> --target=aarch64-linux-android<api>
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

: "${ANDROID_NDK_HOME:?ANDROID_NDK_HOME must be set}"
: "${PYO3_CROSS_LIB_DIR:?PYO3_CROSS_LIB_DIR must point to a dir with libpython3.12.so and _sysconfigdata*.py}"
export PYO3_CROSS_PYTHON_VERSION="3.12"

echo "==> Building Android arm64-v8a..."
cargo ndk \
  --target aarch64-linux-android \
  --platform 26 \
  -- build -p unity_dlp_core --profile release-with-debuginfo --no-default-features --features js-quickjs

echo "==> Building Android armeabi-v7a..."
cargo ndk \
  --target armv7-linux-androideabi \
  --platform 26 \
  -- build -p unity_dlp_core --profile release-with-debuginfo --no-default-features --features js-quickjs

echo "==> Staging .so files..."
ARM64_DEST="unity_package/Plugins/Android/libs/arm64-v8a"
ARMV7_DEST="unity_package/Plugins/Android/libs/armeabi-v7a"
mkdir -p "$ARM64_DEST" "$ARMV7_DEST"

cp target/aarch64-linux-android/release-with-debuginfo/libunity_dlp.so "$ARM64_DEST/"
cp target/armv7-linux-androideabi/release-with-debuginfo/libunity_dlp.so "$ARMV7_DEST/"
echo "==> Android .so files staged."
