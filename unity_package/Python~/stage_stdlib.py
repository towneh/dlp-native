#!/usr/bin/env python3
"""
Stage a Python stdlib zip into the package's StreamingAssets/dlp/stdlib/.

Lives inside the Unity package rather than under scripts/ because
DlpBuildPreprocessor runs it during player builds, and a consumer who installed
the package only has unity_package/ on disk. The trailing ~ stops Unity
importing the directory as assets. CI runs this same file, so the player build
and the CI build share one implementation instead of drifting apart.

Usage:
  python3 unity_package/Python~/stage_stdlib.py PLATFORM [--python PYTHON_EXE]
                                                         [--prefix PREFIX_DIR]
                                                         [--bases BASE ...]

PLATFORM    Target identifier, e.g.:
              windows-x86_64  macos-universal  linux-x86_64
              android-arm64-v8a  ios-arm64

--python    Path to a Python interpreter; its sys.prefix is used as the
            source root. Defaults to the interpreter running this script.

--prefix    Explicit prefix directory (overrides --python sys.prefix).
            Required for cross-compiled targets (Android, iOS) where the
            target Python cannot be executed on the host.

--bases     Sub-directories of the prefix to include (e.g. Lib DLLs).
            Defaults to ['Lib', 'DLLs'] on Windows, or the detected
            lib/pythonX.Y directory on POSIX.

--exclude-dirs
            Extra directory names to skip, on top of ALWAYS_EXCLUDE below.

--out-dir   Directory to write <platform>.zip into. Defaults to
            DEFAULT_OUT_DIR, which is anchored to this file rather than the
            working directory, so the script can be run from anywhere.
            DlpBuildPreprocessor passes it explicitly.

The zip is stored uncompressed (ZIP_STORED) so the OS can page individual
files directly after extraction rather than decompressing everything upfront.

Staging nothing is treated as a failure: an empty archive still satisfies the
"already staged, skip it" check in DlpBuildPreprocessor, so it would quietly
persist into a player build.
"""

import argparse
import os
import re
import subprocess
import sys
import zipfile

# Nothing under these is reachable at runtime: yt-dlp and its own dependencies
# come from the bundled zip on sys.path, never from the staged prefix, so pip's
# bootstrap and the build host's site-packages are dead weight. The rest is the
# Tk stack and dev-only trees, which the embedded interpreter never imports.
ALWAYS_EXCLUDE = frozenset(
    {
        "__pycache__",
        "ensurepip",
        "idlelib",
        "lib2to3",
        "pydoc_data",
        "site-packages",
        "test",
        "tkinter",
        "turtledemo",
    }
)

# Extension modules dropped from the Android bundle. Termux links each against a
# library from its own prefix, which is not on the device, so every one of these
# would fail to dlopen if it were imported at all. Their pure-Python wrappers stay:
# `import bz2` then raises ImportError, which is what the stdlib importers around
# them already handle. Verified against a real extraction — blocking _sqlite3, _bz2,
# _lzma and _zstd leaves it working, and the rest are never imported. Staging them
# anyway would also fail the CI check that every staged module's NEEDED resolves.
ANDROID_EXCLUDE_MODULES = frozenset(
    {
        "_bz2",
        "_curses",
        "_curses_panel",
        "_dbm",
        "_gdbm",
        "_lzma",
        "_multiprocessing",
        "_sqlite3",
        "_zstd",
        "readline",
    }
)

# lib/ entries the POSIX base auto-detection will consider. Shared with
# stage_android_libs.py so both rank interpreter versions the same way.
PY_LIB_DIR_RE = re.compile(r"^python3\.(\d+)$")

# Anchored to this file, not the working directory, so the destination does not
# depend on where the script was invoked from. This file sits at
# <package>/Python~/, so two levels up is the package root.
_PACKAGE_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_OUT_DIR = os.path.join(_PACKAGE_ROOT, "StreamingAssets", "dlp", "stdlib")


def write_tree(z, prefix, bases, exclude, drop_modules):
    """Write each base directory into the archive. Returns (files written, modules dropped)."""
    total = 0
    dropped = []
    for base in bases:
        base_dir = os.path.join(prefix, base)
        if not os.path.isdir(base_dir):
            print(f"WARNING: {base_dir!r} not found, skipping", file=sys.stderr)
            continue
        for root, dirs, files in os.walk(base_dir):
            dirs[:] = [d for d in dirs if d not in exclude]
            for f in files:
                # "_bz2.cpython-314-aarch64-linux-android.so" -> "_bz2"
                if f.endswith(".so") and f.split(".")[0] in drop_modules:
                    dropped.append(f)
                    continue
                full = os.path.join(root, f)
                arc = os.path.relpath(full, prefix).replace(os.sep, "/")
                z.write(full, arc)
                total += 1
    return total, dropped


def main():
    p = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    p.add_argument("platform", help="Target platform id, e.g. windows-x86_64")
    p.add_argument("--python", help="Python interpreter to query sys.prefix from")
    p.add_argument("--prefix", help="Explicit Python prefix dir (for cross-compiled targets)")
    p.add_argument(
        "--bases", nargs="+", help="Sub-dirs of prefix to bundle (default: auto-detected)"
    )
    p.add_argument(
        "--exclude-dirs",
        nargs="+",
        metavar="DIR",
        default=[],
        help="Extra directory names to skip, on top of the built-in list",
    )
    p.add_argument(
        "--out-dir",
        metavar="DIR",
        default=DEFAULT_OUT_DIR,
        help="Where to write <platform>.zip (default: the repo's staged stdlib dir)",
    )
    args = p.parse_args()

    # Resolve prefix
    if args.prefix:
        prefix = os.path.abspath(args.prefix)
    elif args.python:
        prefix = subprocess.check_output(
            [args.python, "-c", 'import sys; print(sys.prefix, end="")'], text=True
        ).strip()
    else:
        prefix = sys.prefix

    # Resolve bases
    if args.bases:
        bases = args.bases
    elif args.platform.startswith("windows"):
        bases = ["Lib", "DLLs"]
    else:
        lib_dir = os.path.join(prefix, "lib")
        if not os.path.isdir(lib_dir):
            sys.exit(f"ERROR: lib/ not found under prefix {prefix!r}")
        # Rank X.Y numerically; sorting the names puts python3.9 above python3.12.
        py_dirs = [
            (int(m[1]), m[0])
            for d in os.listdir(lib_dir)
            if (m := PY_LIB_DIR_RE.match(d)) and os.path.isdir(os.path.join(lib_dir, d))
        ]
        if not py_dirs:
            sys.exit(f"ERROR: no python3.x directory found in {lib_dir!r}")
        bases = [os.path.join("lib", max(py_dirs)[1])]

    # Android's libssl was built for another prefix, so it looks for its trust store
    # somewhere that does not exist on the device and every TLS connection fails
    # verification. Carrying the bundle puts it at <pythonHome>/etc/tls/, where the host
    # points SSL_CERT_FILE. Desktop hosts use their own store.
    #
    # Outside the resolution above, and so applied to explicit --bases too: the trust
    # store is not part of choosing a stdlib layout, and leaving it to the caller means
    # an Android bundle can be built without one. That failure surfaces on device as a
    # certificate error from inside an extractor, nowhere near the staging that caused it.
    if args.platform.startswith("android"):
        tls_base = os.path.join("etc", "tls")
        if not os.path.isdir(os.path.join(prefix, tls_base)):
            sys.exit(
                f"ERROR: no etc/tls under {prefix!r}. The Android bundle needs the "
                "Termux ca-certificates package, or TLS fails verification on device."
            )
        # Normalised, so a caller passing "etc/tls" on Windows does not add it twice
        # and leave the archive carrying every certificate two over.
        if not any(os.path.normpath(b) == os.path.normpath(tls_base) for b in bases):
            bases.append(tls_base)

    out = os.path.join(args.out_dir, args.platform + ".zip")
    os.makedirs(os.path.dirname(out), exist_ok=True)

    exclude = set(args.exclude_dirs) | ALWAYS_EXCLUDE
    drop_modules = ANDROID_EXCLUDE_MODULES if args.platform.startswith("android") else frozenset()

    with zipfile.ZipFile(out, "w", zipfile.ZIP_STORED) as z:
        total, dropped = write_tree(z, prefix, bases, exclude, drop_modules)

    if total == 0:
        # Leaving the empty archive behind would satisfy the "already staged"
        # check in DlpBuildPreprocessor and ship a stdlib-less zip.
        os.remove(out)
        sys.exit(f"ERROR: staged nothing from {prefix!r} (bases: {bases})")

    size_mb = os.path.getsize(out) / 1_048_576
    print(f"Staged {total} files from {prefix!r} -> {out!r} ({size_mb:.1f} MB)")
    if dropped:
        print(
            f"Dropped {len(dropped)} unsupported extension modules: "
            f"{', '.join(sorted(f.split('.')[0] for f in dropped))}"
        )


if __name__ == "__main__":
    main()
