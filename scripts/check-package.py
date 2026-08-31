#!/usr/bin/env python3
"""
Assert the Unity package tarball carries a loadable payload for every platform.

package-unity.sh tars whatever happens to be in unity_package/ at the time, so a
platform whose upload path stops matching drops out of the package without
failing anything. The only symptom is a plugin that will not load on that
platform, discovered by whoever installed it.

Usage:
  python3 scripts/check-package.py [TARBALL]

TARBALL defaults to the single .tgz in the working directory.
"""

from __future__ import annotations

import fnmatch
import sys
import tarfile
from pathlib import Path

PLUGINS = "package/Plugins/x86_64"

# (what it is, patterns that satisfy it, how many members must match).
# Alternatives exist where the staged filename depends on what the build found:
# the macOS runtime is named after the dylib the plugin actually links against.
REQUIRED: list[tuple[str, list[str], int]] = [
    ("package manifest", ["package/package.json"], 1),
    ("runtime scripts", ["package/Runtime/YtDlp.cs"], 1),
    ("yt-dlp bundle", ["package/StreamingAssets/dlp/yt_dlp.zip"], 1),
    ("Windows plugin", [f"{PLUGINS}/unity_dlp.dll"], 1),
    # The stable-ABI forwarder and the runtime behind it: two files, and the
    # plugin cannot be loaded on Windows without both.
    ("Windows Python runtime", [f"{PLUGINS}/python3*.dll"], 2),
    ("macOS plugin", [f"{PLUGINS}/unity_dlp.dylib"], 1),
    ("macOS Python runtime", [f"{PLUGINS}/Python", f"{PLUGINS}/libpython*.dylib"], 1),
    ("Linux plugin", [f"{PLUGINS}/libunity_dlp.so"], 1),
    ("Linux Python runtime", [f"{PLUGINS}/libpython*.so*"], 1),
    ("Android plugin", ["package/Plugins/Android/libs/arm64-v8a/libunity_dlp.so"], 1),
    # The static library in both slices, not just anything under the xcframework:
    # Info.plist alone would otherwise satisfy this, and a device-only framework
    # would pass while failing to link against the simulator.
    ("iOS slice libraries", ["package/Plugins/iOS/unity_dlp.xcframework/*/libunity_dlp.a"], 2),
]

STDLIB_PLATFORMS = [
    "windows-x86_64",
    "macos-universal",
    "linux-x86_64",
    "android-arm64-v8a",
    "ios-arm64",
]


def resolve_tarball(argv: list[str]) -> Path:
    if argv:
        return Path(argv[0])
    candidates = sorted(Path.cwd().glob("*.tgz"))
    if not candidates:
        print("no .tgz found; run scripts/package-unity.sh first")
        sys.exit(1)
    if candidates[1:]:
        found = ", ".join(path.name for path in candidates)
        print(f"expected one .tgz, found several: {found}")
        sys.exit(1)
    return candidates[0]


def main() -> int:
    tarball = resolve_tarball(sys.argv[1:])
    with tarfile.open(tarball) as tar:
        members = tar.getnames()

    failures: list[str] = []

    for description, patterns, minimum in REQUIRED:
        matched = sum(1 for name in members if any(fnmatch.fnmatch(name, p) for p in patterns))
        if matched < minimum:
            alternatives = " or ".join(patterns)
            failures.append(f"{description}: matched {matched} of {minimum} ({alternatives})")

    for platform_id in STDLIB_PLATFORMS:
        name = f"package/StreamingAssets/dlp/stdlib/{platform_id}.zip"
        if name not in members:
            failures.append(f"stdlib for {platform_id}: {name} is absent")

    print(f"{tarball.name}: {len(members)} entries")
    if failures:
        for failure in failures:
            print(f"  missing  {failure}")
        return 1
    print("  every platform present")
    return 0


if __name__ == "__main__":
    sys.exit(main())
