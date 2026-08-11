# Vendor Pins

This file records the pinned versions of vendored submodules.
Update both the submodule commit and this file when bumping.

| Submodule | Tag / Commit | Pinned date |
|-----------|-------------|-------------|
| `vendor/yt-dlp` | `2026.07.04` | 2026-07-05 |
| `vendor/yt-dlp-ejs` | `0.8.0` | 2026-05-14 |

`yt-dlp-ejs`'s version is read from its own tag when the bundle is built, so
bumping the submodule is enough — there is no second copy to keep in step.

Bumping either submodule also changes `unity_package/StreamingAssets/dlp/yt_dlp.zip`,
which is committed: its entries are stamped with the commit date of the newest
vendored source, so the archive moves with the pin even where the file contents
would not.

## Redistributed binaries (Android)

The Android artifact ships more than this project builds. Termux's aarch64 packages
supply the interpreter and the shared libraries the stdlib's C extensions link
against, and a CA bundle for them to verify TLS against. These are not submodules and
are deliberately not pinned here: CI resolves the current version of each from the
Termux package index at build time, because the pool keeps only the current build and
a pinned hash goes stale the moment Termux rebuilds. The index is checked against the
signed release, and each package against the checksum the index publishes.

| Package | Version at the last build | What ships | Why |
|---|---|---|---|
| `python` | 3.14.6-1 | `libpython3.14.so` | the interpreter itself |
| `libandroid-support` | 29-1 | `libandroid-support.so` | libc compatibility shim libpython links against |
| `zlib` | 1.3.2 | `libz.so.1` | `zlib`, `binascii` |
| `openssl` | 1:3.6.3 | `libssl.so.3`, `libcrypto.so.3` | `_ssl`, `_hashlib` |
| `libffi` | 3.5.2 | `libffi.so` | `_ctypes` |
| `libexpat` | 2.8.3 | `libexpat.so.1` | `pyexpat` |
| `ca-certificates` | 1:2026.07.16 | `etc/tls/cert.pem` | the trust store `SSL_CERT_FILE` points at |

Versions are what the last Android build resolved, recorded for reference rather than
as a pin — read the current ones out of a build log, or from
`dists/stable/main/binary-aarch64/Packages`.

The interpreter and its support shim are staged into `Plugins/Android/libs/arm64-v8a/`,
so they land in the APK's lib directory, which the loader searches. Everything else
goes inside the stdlib zip beside `lib-dynload`, where the extensions find them through
a rewritten `RUNPATH` — see `unity_package/Python~/stage_android_libs.py`.

Licence terms are the upstream projects' own and are not reproduced here:
[CPython](https://python.org/), [libandroid-support](https://github.com/termux/libandroid-support),
[zlib](https://www.zlib.net/), [OpenSSL](https://www.openssl.org/),
[libffi](https://sourceware.org/libffi/), [libexpat](https://libexpat.github.io/),
and the CA bundle, which is Mozilla's list [as published by curl](https://curl.se/docs/caextract.html).

## Vendored crate: `crates/deno_ast_patch`

| | |
|---|---|
| Real version | **0.50.3** — upstream `deno_ast`, source unmodified |
| Version it declares | **0.49.0** |
| Why | `rustyscript` pins `deno_ast` to an exact requirement that the real version does not satisfy |

Those two rows are the only place either number is written down here, so the
guidance below stays true across a bump without anyone having to remember to
edit it. CI compares both against the crate's manifests.

The declared version is deliberately wrong and is load-bearing: correcting it to
the real version breaks resolution. `Cargo.toml.orig` inside the crate still
carries the real number.

**When reading advisories, treat this crate as the real version above.** `cargo
audit` and `cargo deny` read the lockfile, which carries the declared version, so
any `deno_ast` result — a hit or a clean bill — is answering about the wrong one.
Neither can be pointed past the lockfile at the real version, so check it
directly at <https://rustsec.org/packages/deno_ast.html>.

Removing the need for this means moving the pin upstream to a requirement that
actually admits the version in use. Note that a `^` requirement will not
necessarily do it: for a `0.x` version Cargo treats the minor as the breaking
component, so `^0.49` resolves `>=0.49.0, <0.50.0` and still excludes 0.50.3 —
it would need `^0.50` if the API suits. Patching the consumer's requirement is
far less to carry than a vendored parser.

## Bumping yt-dlp

1. `cd vendor/yt-dlp && git fetch --tags && git checkout <new-tag>`
2. Update the table above.
3. Run `scripts/bump-yt-dlp.sh` (Phase 6) — it will rebuild and run URL tests.
4. If all tests pass, commit `vendor/yt-dlp` and `VENDOR.md` together.
5. Tag the commit `yt-dlp/<new-tag>`.

## Adding submodules (first time)

```sh
git submodule add https://github.com/yt-dlp/yt-dlp vendor/yt-dlp
git -C vendor/yt-dlp checkout 2025.01.15   # substitute pinned tag
git submodule add https://github.com/yt-dlp/yt-dlp-ejs vendor/yt-dlp-ejs
git -C vendor/yt-dlp-ejs checkout <tag matching yt-dlp's pyproject.toml>
```
