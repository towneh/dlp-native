#!/usr/bin/env bash
# Assemble the Unity Package Manager tarball from the unity_package/ tree.
# Run after all platform builds have been staged.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

VERSION=$(cargo metadata --no-deps --format-version 1 \
  | python3 -c "import sys,json; pkgs=json.load(sys.stdin)['packages']; \
    print(next(p['version'] for p in pkgs if p['name']=='unity_dlp_core'))")

TARBALL="com.yewnyx.ytdlp-${VERSION}.tgz"

echo "==> Packaging version $VERSION into $TARBALL..."
tar -czf "$TARBALL" \
  --transform 's|^unity_package|package|' \
  unity_package/

echo "==> Package created: $TARBALL"
