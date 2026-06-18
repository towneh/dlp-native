# dlp-native

> **WARNING: THIS PROJECT IS A PROOF OF CONCEPT AND HAS BEEN ENTIRELY VIBE CODED. Treat it with all due suspicion.**

A Unity 6.3+ native plugin that embeds CPython + yt-dlp to extract media metadata (URL resolution only — no download) without spawning subprocesses at runtime. Built in Rust, consumed by C# P/Invoke. The Python stdlib and yt-dlp source ship as StreamingAssets zips and are unpacked on first run — the native binary carries only the interpreter and JS engine, so yt-dlp updates don't require rebuilding the plugin.

## What it does

Given a URL (YouTube, Vimeo, SoundCloud, and anything else yt-dlp supports), it returns the resolved media metadata as JSON — stream URLs, title, duration, thumbnails, formats. No subprocess is spawned; the Python interpreter and yt-dlp run in-process inside the native plugin.

YouTube's JS signature challenges are solved via an in-process JS engine: V8 (via [rustyscript](https://github.com/nicholasgasior/rustyscript)) on Windows/macOS, [QuickJS](https://github.com/DelSkayn/rquickjs) on Linux/iOS.

## Getting started

The easiest way to get the plugin files is to download the latest CI artifacts — no Rust or Python toolchain required. You'll need the [GitHub CLI](https://cli.github.com/) authenticated to this repo (`gh auth login`).

**Windows:**
```powershell
pwsh scripts/fetch-artifacts.ps1 windows
```

**macOS:**
```bash
bash scripts/fetch-artifacts.sh macos
```

**Linux:**
```bash
bash scripts/fetch-artifacts.sh linux
```

Pass multiple platform names to fetch several at once (e.g. `windows android`), or omit all arguments to fetch every platform. The files are placed directly into `unity_package/`.

To build from source instead, see [Building](#building).

## Platform status

| Platform | Status | Notes |
|----------|--------|-------|
| Windows x86_64 | ✅ Working | V8 (rustyscript) |
| macOS universal | ✅ Working | arm64 + x86_64, lipo'd, V8 (rustyscript) |
| Linux x86_64 | ✅ Working | QuickJS (rquickjs) |
| Android arm64-v8a | 🔧 In progress | QuickJS (rquickjs), libpython via Termux .deb |
| iOS arm64 | ✅ Working | QuickJS (rquickjs), xcframework (device + arm64 simulator), iOS 16.0+ |

## Building

**Windows (PowerShell):**
```powershell
pwsh scripts/build-host.ps1
```

**macOS / Linux:**
```bash
bash scripts/build-host.sh
```

**macOS universal (arm64 + x86_64):**
```bash
bash scripts/build-macos-universal.sh
```

**Android (requires Android NDK + cargo-ndk):**
```bash
export ANDROID_NDK_HOME=/path/to/ndk
bash scripts/build-android.sh
```

**iOS (requires macOS host + Xcode):**
```bash
bash scripts/build-ios.sh
```

Windows, macOS, and Linux scripts require [uv](https://github.com/astral-sh/uv) with Python 3.12 installed (`uv python install 3.12`). iOS uses a static Python framework from [python-apple-support](https://github.com/beeware/python-apple-support) and does not need uv.

## Architecture

```
Unity C# (DlpBootstrap.cs + YtDlp.cs)
    ├── StreamingAssets/dlp/stdlib/<platform>.zip  ─┐ unpacked to
    ├── StreamingAssets/dlp/yt_dlp.zip             ─┘ persistentDataPath on first run
    └── P/Invoke → unity_dlp.{dll,dylib,so} / libunity_dlp.a
                       └── Rust (unity_dlp_core)
                               ├── PyO3 → CPython 3.12 (interpreter only)
                               │             └── yt-dlp + unity_dlp_jsc (loaded from filesystem)
                               └── JS engine (feature-selected at build time)
                                       ├── js-v8: rustyscript → V8  (Windows, macOS)
                                       └── js-quickjs: rquickjs → QuickJS  (Linux, Android, iOS)
```

## Keeping yt-dlp current

yt-dlp ages fastest — YouTube changes its player JS and formats often, while the embedded CPython and the native ABI rarely move. So the bundled yt-dlp can refresh itself at runtime rather than waiting for a plugin rebuild.

After init, `DlpBootstrap` checks PyPI for a newer yt-dlp (`DlpUpdater`, fire-and-forget, on by default via `DlpBootstrap.AutoUpdate`). A newer release is downloaded, sha256-verified against the PyPI digest, checked for compatibility with the embedded interpreter's Python version, and staged for the next launch — the running interpreter keeps the package it booted with, since re-init isn't safe. On the next launch the staged copy is used in place of the bundled zip; anything wrong with it (missing, hash mismatch, version-incompatible) falls back to the bundled zip, and the check never throws.

What this does not cover:

- The Python stdlib is tied to the embedded interpreter and only changes on a plugin rebuild.
- Compiled extensions (e.g. `curl_cffi`) can't ship as a zip and stay with the build.
- iOS is pinned to the bundled package — the App Store forbids downloading and running new code at runtime, so it refreshes via an app update.

Set `DlpBootstrap.AutoUpdate = false` before the first init call to pin to the bundled package.

## Scope

Metadata / URL resolution only. No download API. The plugin resolves stream URLs; actual downloading is left to the caller (Unity's `UnityWebRequest`, FFmpeg, etc.).

## License

MIT
