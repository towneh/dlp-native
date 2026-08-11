#!/usr/bin/env python3
"""
Make the Android stdlib's C extensions loadable on a device, in the Termux prefix,
before stage_stdlib.py zips it.

Termux builds each extension against libraries from its own prefix and records
RUNPATH=/data/data/com.termux/files/usr/lib. That path does not exist on a device
that is not Termux, and the loader does not search the directory an extension was
opened from, so a NEEDED entry like libz.so.1 resolves nowhere and the import fails:

    dlopen failed: library "libz.so.1" not found:
      needed by .../lib-dynload/binascii.cpython-314-aarch64-linux-android.so

Two changes fix that. Every library the extensions need is copied in beside them,
and RUNPATH is rewritten to $ORIGIN so the loader looks there. Libraries staged into
the APK (libpython3.14.so, libandroid-support.so) are already on the namespace's
search path and are left alone; so are the ones Android itself provides.

The dependency set is computed, not listed: whatever the modules declare is what gets
copied, transitively, so a Termux rebuild that adds a dependency is carried without
editing this file. A dependency that cannot be found in the prefix is an error --
staging it absent would only move the failure to the device.

Usage:
  python3 unity_package/Python~/stage_android_libs.py --prefix PREFIX_DIR
"""

import argparse
import os
import re
import shutil
import struct
import sys

# Same directory, so running this by path puts it on sys.path. Sharing the list keeps
# a module dropped from the zip from dragging its libraries in here.
from stage_stdlib import ANDROID_EXCLUDE_MODULES

# Libraries Android itself provides, so they resolve from the platform. Mirrors the
# list the CI verify step accepts, minus libc++_shared: that is an NDK runtime, and
# anything needing it has to be staged rather than borrowed from the host app.
PLATFORM = re.compile(
    r"^(libc|libm|libdl|liblog|libandroid|libz|libGLESv[123]|libEGL|libOpenSLES|libvulkan)\.so$"
)

# Staged into Plugins/Android/libs/, which lands in the APK's lib directory and is on
# the classloader namespace's search path, so extensions resolve these by soname.
IN_APK = frozenset({"libpython3.14.so", "libandroid-support.so"})

ORIGIN = b"$ORIGIN\0"

DT_NULL, DT_NEEDED, DT_RPATH, DT_SONAME, DT_RUNPATH = 0, 1, 15, 14, 29
_STRING_TAGS = (DT_NEEDED, DT_SONAME, DT_RPATH, DT_RUNPATH)
SHT_DYNAMIC = 6


def _dynamic(data):
    """(entries, dynstr_offset) for an ELF64 little-endian image.

    entries is a list of (tag, value, entry_offset); dynstr_offset is where the
    dynamic string table starts in the file.
    """
    if data[:4] != b"\x7fELF":
        return None, None
    (e_shoff,) = struct.unpack_from("<Q", data, 0x28)
    e_shentsize, e_shnum = struct.unpack_from("<HH", data, 0x3A)
    sections, dynamic = [], None
    for i in range(e_shnum):
        off = e_shoff + i * e_shentsize
        (sh_type,) = struct.unpack_from("<I", data, off + 4)
        sh_offset, sh_size = struct.unpack_from("<QQ", data, off + 0x18)
        (sh_link,) = struct.unpack_from("<I", data, off + 0x28)
        sections.append((sh_offset, sh_size))
        if sh_type == SHT_DYNAMIC:
            dynamic = (sh_offset, sh_size, sh_link)
    if dynamic is None:
        return None, None

    entries = []
    off, end = dynamic[0], dynamic[0] + dynamic[1]
    while off < end:
        tag, value = struct.unpack_from("<qQ", data, off)
        if tag == DT_NULL:
            break
        entries.append((tag, value, off))
        off += 16
    return entries, sections[dynamic[2]][0]


def _string(data, stroff, value):
    return data[stroff + value : data.index(b"\0", stroff + value)].decode()


def needed_of(path):
    with open(path, "rb") as f:
        data = f.read()
    entries, stroff = _dynamic(data)
    if entries is None:
        return []
    return [_string(data, stroff, v) for tag, v, _ in entries if tag == DT_NEEDED]


def set_runpath_origin(path):
    """Point RUNPATH at $ORIGIN, in place. Returns False if there is none to set.

    The replacement is shorter than what Termux records, so it is written over the
    existing string rather than relocating the string table. Any other dynamic entry
    pointing into the bytes being overwritten would be corrupted by that, so this
    refuses rather than risk it -- string tables can share tails between entries.
    """
    with open(path, "rb") as f:
        data = bytearray(f.read())
    entries, stroff = _dynamic(data)
    if entries is None:
        return False

    target = next(((t, v) for t, v, _ in entries if t in (DT_RUNPATH, DT_RPATH)), None)
    if target is None:
        return False
    _, value = target

    existing = _string(data, stroff, value)
    if len(existing) + 1 < len(ORIGIN):
        sys.exit(f"ERROR: {path}: RUNPATH {existing!r} is too short to overwrite")

    clobbered = range(value, value + len(ORIGIN))
    for tag, other, _ in entries:
        if tag in _STRING_TAGS and other != value and other in clobbered:
            sys.exit(f"ERROR: {path}: rewriting RUNPATH would corrupt a shared string at +{other}")

    data[stroff + value : stroff + value + len(ORIGIN)] = ORIGIN
    with open(path, "wb") as f:
        f.write(data)
    return True


def find_lib_dynload(prefix):
    lib = os.path.join(prefix, "lib")
    for name in sorted(os.listdir(lib), reverse=True):
        if re.match(r"^python3\.\d+$", name):
            path = os.path.join(lib, name, "lib-dynload")
            if os.path.isdir(path):
                return path
    sys.exit(f"ERROR: no lib/python3.x/lib-dynload under {prefix!r}")


def main():
    p = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    p.add_argument("--prefix", required=True, help="Extracted Termux prefix (…/files/usr)")
    args = p.parse_args()

    prefix = os.path.abspath(args.prefix)
    dynload = find_lib_dynload(prefix)
    libdir = os.path.join(prefix, "lib")

    # Breadth-first over what the modules need, then what those libraries need. Modules
    # stage_stdlib.py drops are excluded: their libraries would otherwise be copied in
    # to satisfy something that never ships.
    have = {
        f
        for f in os.listdir(dynload)
        if f.endswith(".so") and f.split(".")[0] not in ANDROID_EXCLUDE_MODULES
    }
    queue = [os.path.join(dynload, f) for f in sorted(have)]
    copied = []
    while queue:
        for need in needed_of(queue.pop()):
            if need in have or need in IN_APK or PLATFORM.match(need):
                continue
            src = os.path.join(libdir, need)
            if not os.path.exists(src):
                sys.exit(
                    f"ERROR: {need} is needed by the staged extensions but is not in "
                    f"{libdir!r}. Add the Termux package that ships it to the Android job."
                )
            dest = os.path.join(dynload, need)
            # Resolve symlinks: Termux ships libffi.so pointing at libffi.so.8, and a
            # dangling link in the zip is a library that is not there.
            shutil.copy(os.path.realpath(src), dest)
            have.add(need)
            copied.append(need)
            queue.append(dest)

    patched = sum(1 for f in sorted(have) if set_runpath_origin(os.path.join(dynload, f)))

    listed = ", ".join(sorted(copied)) or "(none)"
    print(f"Copied {len(copied)} libraries into lib-dynload: {listed}")
    print(f"Set RUNPATH=$ORIGIN on {patched} shared objects")


if __name__ == "__main__":
    main()
