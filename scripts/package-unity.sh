#!/usr/bin/env bash
# Assemble the Unity Package Manager tarball from the unity_package/ tree.
# Run after all platform builds have been staged.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# The Unity package's own version, not the Rust crate's: this tarball is what UPM
# installs, and it resolves to whatever package.json declares. Naming it after the
# workspace version instead produces a file whose name contradicts its contents.
VERSION=$(python3 -c \
  "import json; print(json.load(open('unity_package/package.json'))['version'])")

TARBALL="town.mr.ytdlp-${VERSION}.tgz"

echo "==> Packaging version $VERSION into $TARBALL..."
# Python~/ ships the stdlib staging script, so running it in the working tree
# leaves bytecode behind that has no business in a release tarball.
tar -czf "$TARBALL" \
  --exclude='__pycache__' \
  --transform 's|^unity_package|package|' \
  unity_package/

echo "==> Package created: $TARBALL"
