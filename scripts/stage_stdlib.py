#!/usr/bin/env python3
"""
Stage a Python stdlib zip into unity_package/StreamingAssets/dlp/stdlib/.

Usage:
  python3 scripts/stage_stdlib.py PLATFORM [--python PYTHON_EXE]
                                            [--prefix PREFIX_DIR]
                                            [--bases BASE [BASE ...]]

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

The zip is stored uncompressed (ZIP_STORED) so the OS can page individual
files directly after extraction rather than decompressing everything upfront.
"""
import argparse
import os
import subprocess
import sys
import zipfile


def main():
    p = argparse.ArgumentParser(description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('platform',
                   help='Target platform id, e.g. windows-x86_64')
    p.add_argument('--python',
                   help='Python interpreter to query sys.prefix from')
    p.add_argument('--prefix',
                   help='Explicit Python prefix dir (for cross-compiled targets)')
    p.add_argument('--bases', nargs='+',
                   help='Sub-dirs of prefix to bundle (default: auto-detected)')
    p.add_argument('--exclude-dirs', nargs='+', metavar='DIR', default=[],
                   help='Directory names to skip (e.g. lib-dynload test)')
    args = p.parse_args()

    # Resolve prefix
    if args.prefix:
        prefix = os.path.abspath(args.prefix)
    elif args.python:
        prefix = subprocess.check_output(
            [args.python, '-c', 'import sys; print(sys.prefix, end="")'],
            text=True).strip()
    else:
        prefix = sys.prefix

    # Resolve bases
    if args.bases:
        bases = args.bases
    elif args.platform.startswith('windows'):
        bases = ['Lib', 'DLLs']
    else:
        lib_dir = os.path.join(prefix, 'lib')
        if not os.path.isdir(lib_dir):
            sys.exit(f'ERROR: lib/ not found under prefix {prefix!r}')
        py_dirs = sorted(d for d in os.listdir(lib_dir) if d.startswith('python3.'))
        if not py_dirs:
            sys.exit(f'ERROR: no python3.x directory found in {lib_dir!r}')
        bases = [os.path.join('lib', py_dirs[-1])]

    out = os.path.join('unity_package', 'StreamingAssets', 'dlp', 'stdlib',
                       args.platform + '.zip')
    os.makedirs(os.path.dirname(out), exist_ok=True)

    total = 0
    with zipfile.ZipFile(out, 'w', zipfile.ZIP_STORED) as z:
        for base in bases:
            base_dir = os.path.join(prefix, base)
            if not os.path.isdir(base_dir):
                print(f'WARNING: {base_dir!r} not found, skipping', file=sys.stderr)
                continue
            exclude = set(args.exclude_dirs) | {'__pycache__'}
            for root, dirs, files in os.walk(base_dir):
                dirs[:] = [d for d in dirs if d not in exclude]
                for f in files:
                    full = os.path.join(root, f)
                    arc = os.path.relpath(full, prefix).replace(os.sep, '/')
                    z.write(full, arc)
                    total += 1

    size_mb = os.path.getsize(out) / 1_048_576
    print(f'Staged {total} files from {prefix!r} -> {out!r} ({size_mb:.1f} MB)')


if __name__ == '__main__':
    main()
