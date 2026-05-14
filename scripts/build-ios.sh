#!/usr/bin/env bash
# Build iOS device (arm64) and simulator (arm64) static libs,
# then package into an xcframework.
# Must run on a macOS host.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

DEVICE="aarch64-apple-ios"
SIM="aarch64-apple-ios-sim"

echo "==> Building iOS device (arm64)..."
cargo build -p unity_dlp_core --release --target "$DEVICE"

echo "==> Building iOS simulator (arm64)..."
cargo build -p unity_dlp_core --release --target "$SIM"

echo "==> Creating xcframework..."
rm -rf unity_dlp.xcframework
xcodebuild -create-xcframework \
  -library "target/$DEVICE/release/libunity_dlp.a" \
  -library "target/$SIM/release/libunity_dlp.a" \
  -output "unity_dlp.xcframework"

echo "==> Copying static lib slices to Unity package..."
DEST="unity_package/Plugins/iOS"
mkdir -p "$DEST"
cp "target/$DEVICE/release/libunity_dlp.a" "$DEST/libunity_dlp.a"
echo "==> iOS libs staged."
