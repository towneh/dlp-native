#!/usr/bin/env bash
# Build iOS device (arm64) and simulator (arm64) static libs,
# then package into an xcframework.
# Must run on a macOS host.
#
# Requirements:
#   - Rust toolchain with aarch64-apple-ios and aarch64-apple-ios-sim targets
#     (rustup target add aarch64-apple-ios aarch64-apple-ios-sim)
#   - Xcode command-line tools
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

DEVICE="aarch64-apple-ios"
SIM="aarch64-apple-ios-sim"
export IPHONEOS_DEPLOYMENT_TARGET="16.0"
export PYO3_CROSS_PYTHON_VERSION="3.12"

# ── Fetch python-apple-support if needed ──────────────────────────────────────
if [[ ! -d python-ios ]]; then
  echo "==> Downloading Python 3.12 for iOS (BeeWare python-apple-support)..."
  curl -fsSL -o /tmp/Python-3.12-iOS-support.b8.tar.gz \
    "https://github.com/beeware/python-apple-support/releases/download/3.12-b8/Python-3.12-iOS-support.b8.tar.gz"
  mkdir python-ios
  tar -xzf /tmp/Python-3.12-iOS-support.b8.tar.gz -C python-ios
fi

# ── Set up Python lib dirs ────────────────────────────────────────────────────
DEVICE_FW="$(pwd)/python-ios/Python.xcframework/ios-arm64/Python.framework"
SIM_FW_DIR="$(ls -d "$(pwd)/python-ios/Python.xcframework/"*simulator*/ | head -1)"
SIM_FW="${SIM_FW_DIR%/}/Python.framework"

mkdir -p python-ios-device-lib python-ios-sim-lib
cp "$DEVICE_FW/Python" python-ios-device-lib/libpython3.12.a
cp "$SIM_FW/Python"    python-ios-sim-lib/libpython3.12.a
export CFLAGS="-I$DEVICE_FW/Headers"

# ── Build device (arm64) ──────────────────────────────────────────────────────
echo "==> Building iOS device (arm64)..."
export SDKROOT="$(xcrun --sdk iphoneos --show-sdk-path)"
PYO3_NO_PYTHON=1 PYO3_CROSS_LIB_DIR="$(pwd)/python-ios-device-lib" \
  cargo build -p unity_dlp_core --profile release-with-debuginfo --target "$DEVICE" \
  --no-default-features --features js-quickjs

# ── Build simulator (arm64) ───────────────────────────────────────────────────
echo "==> Building iOS simulator (arm64)..."
export SDKROOT="$(xcrun --sdk iphonesimulator --show-sdk-path)"
PYO3_NO_PYTHON=1 PYO3_CROSS_LIB_DIR="$(pwd)/python-ios-sim-lib" \
  cargo build -p unity_dlp_core --profile release-with-debuginfo --target "$SIM" \
  --no-default-features --features js-quickjs

# ── Package xcframework ───────────────────────────────────────────────────────
echo "==> Creating xcframework..."
DEST="unity_package/Plugins/iOS"
mkdir -p "$DEST"
rm -rf "$DEST/unity_dlp.xcframework"
xcodebuild -create-xcframework \
  -library "target/$DEVICE/release-with-debuginfo/libunity_dlp.a" \
  -library "target/$SIM/release-with-debuginfo/libunity_dlp.a" \
  -output "$DEST/unity_dlp.xcframework"
echo "==> xcframework staged to $DEST/unity_dlp.xcframework"
