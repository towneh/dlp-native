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
# a module dropped from the zip from dragging its libraries in here, and the version
# ranking from disagreeing about which lib-dynload is staged.
from stage_stdlib import ANDROID_EXCLUDE_MODULES, PY_LIB_DIR_RE

# Libraries Android itself provides, so they resolve from the platform. Mirrors the
# list the CI verify step accepts, minus libc++_shared: that is an NDK runtime, and
# anything needing it has to be staged rather than borrowed from the host app.
PLATFORM = re.compile(
    r"^(libc|libm|libdl|liblog|libandroid|libz|libGLESv[123]|libEGL|libOpenSLES|libvulkan)\.so$"
)

# Staged into Plugins/Android/libs/, which lands in the APK's lib directory and is on
# the classloader namespace's search path, so extensions resolve these by soname. The
# interpreter is matched by pattern rather than by version: pinning one would, after a
# CPython bump, copy libpython itself in here and ship a second one inside the zip.
#
# [0-9] rather than \d in both patterns here: the CI check reads them out of this module
# and hands them to grep -E, which has no \d, and would then quietly match nothing.
IN_APK = re.compile(r"^(libpython3\.[0-9]+|libandroid-support)\.so$")

ORIGIN = b"$ORIGIN\0"

DT_NULL, DT_NEEDED, DT_RPATH, DT_SONAME, DT_RUNPATH = 0, 1, 15, 14, 29
_STRING_TAGS = (DT_NEEDED, DT_SONAME, DT_RPATH, DT_RUNPATH)
SHT_DYNAMIC = 6


def _dynamic(data):
    """(entries, dynstr_offset) for an ELF64 little-endian image.

    entries is a list of (tag, value, entry_offset); dynstr_offset is where the
    dynamic string table starts in the file.
    """
    # EI_CLASS=ELFCLASS64 and EI_DATA=ELFDATA2LSB: every offset below assumes both, and
    # set_runpath_origin writes back at those offsets, so a 32-bit or big-endian image
    # has to be refused rather than parsed into arbitrary positions.
    if data[:6] != b"\x7fELF\x02\x01":
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


def siblings_of(path):
    """What this object needs that only a library staged beside it can satisfy."""
    return [n for n in needed_of(path) if not PLATFORM.match(n) and not IN_APK.match(n)]


def set_runpath_origin(path):
    """Point DT_RUNPATH at $ORIGIN, in place. Returns False if there is none to set.

    DT_RPATH is deliberately not accepted. Bionic reads DT_RUNPATH only -- it consumes
    it when resolving a NEEDED entry, and logs DT_RPATH as an "unused DT entry
    (ignoring)". Rewriting one would produce a file that looks patched and is ignored
    at load, which is worse than reporting that it has no usable RUNPATH.

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

    value = next((v for t, v, _ in entries if t == DT_RUNPATH), None)
    if value is None:
        return False

    existing = _string(data, stroff, value)
    if len(existing) + 1 < len(ORIGIN):
        sys.exit(f"ERROR: {path}: RUNPATH {existing!r} is too short to overwrite")

    clobbered = range(value, value + len(ORIGIN))
    for tag, other, _ in entries:
        if tag not in _STRING_TAGS or other == value:
            continue
        # A tail can be shared from either side: an entry starting inside the bytes
        # being written, or one starting earlier whose string runs through them.
        if other in clobbered or other < value <= other + len(_string(data, stroff, other)):
            sys.exit(f"ERROR: {path}: rewriting RUNPATH would corrupt a shared string at +{other}")

    data[stroff + value : stroff + value + len(ORIGIN)] = ORIGIN
    with open(path, "wb") as f:
        f.write(data)
    return True


def find_lib_dynload(prefix):
    lib = os.path.join(prefix, "lib")
    # Rank X.Y numerically; sorting the names puts python3.9 above python3.14, which
    # would stage libraries into a different tree from the one stage_stdlib.py zips.
    candidates = [
        (int(m[1]), m[0])
        for name in os.listdir(lib)
        if (m := PY_LIB_DIR_RE.match(name))
        and os.path.isdir(os.path.join(lib, name, "lib-dynload"))
    ]
    if not candidates:
        sys.exit(f"ERROR: no lib/python3.x/lib-dynload under {prefix!r}")
    return os.path.join(lib, max(candidates)[1], "lib-dynload")


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
            if need in have or IN_APK.match(need) or PLATFORM.match(need):
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

    patched, stranded = 0, []
    for f in sorted(have):
        path = os.path.join(dynload, f)
        if set_runpath_origin(path):
            patched += 1
        elif siblings_of(path):
            # Nothing to overwrite, and it needs a library from this directory, so the
            # loader would never look for it. Objects needing only platform libraries
            # or what the APK carries are fine without one.
            stranded.append(f)

    listed = ", ".join(sorted(copied)) or "(none)"
    print(f"Copied {len(copied)} libraries into lib-dynload: {listed}")
    print(f"Set RUNPATH=$ORIGIN on {patched} shared objects")

    if stranded:
        # patchelf --set-rpath writes DT_RUNPATH, which is the tag bionic reads.
        sys.exit(
            "ERROR: no DT_RUNPATH to rewrite, but a library staged beside them is needed "
            f"by: {', '.join(stranded)}. Add one with patchelf --set-rpath '$ORIGIN'."
        )


if __name__ == "__main__":
    main()
