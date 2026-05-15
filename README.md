# dlp-native

> **WARNING: THIS PROJECT IS A PROOF OF CONCEPT AND HAS BEEN ENTIRELY VIBE CODED. Treat it with all due suspicion.**

A Unity 6.3+ native plugin that embeds CPython + yt-dlp to extract media metadata (URL resolution only — no download) without spawning subprocesses at runtime. Built in Rust, consumed by C# P/Invoke.

## What it does

Given a URL (YouTube, Vimeo, SoundCloud, and anything else yt-dlp supports), it returns the resolved media metadata as JSON — stream URLs, title, duration, thumbnails, formats. No subprocess is spawned; the Python interpreter and yt-dlp run in-process inside the native plugin.

YouTube's JS signature challenges are solved via an in-process JS engine: V8 (via [rustyscript](https://github.com/nicholasgasior/rustyscript)) on Windows/macOS, [QuickJS](https://github.com/DelSkayn/rquickjs) on Linux/iOS.

## Platform status

| Platform | Status | Notes |
|----------|--------|-------|
| Windows x86_64 | ✅ Working | V8 (rustyscript) |
| macOS universal | ✅ Working | arm64 + x86_64, lipo'd, V8 (rustyscript) |
| Linux x86_64 | ✅ Working | QuickJS (rquickjs) |
| Android arm64-v8a | 🔧 In progress | QuickJS (rquickjs), libpython via Termux .deb |
| iOS arm64 | ✅ Working | QuickJS (rquickjs), static lib, iOS 16.0+ |

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
Unity C# (YtDlp.cs)
    └── P/Invoke → unity_dlp.{dll,dylib,so} / libunity_dlp.a
                       └── Rust (unity_dlp_core)
                               ├── PyO3 → embedded CPython 3.12
                               │             └── yt-dlp (zip, embedded)
                               │                   └── unity_dlp_jsc (JCP shim)
                               └── JS engine (feature-selected at build time)
                                       ├── js-v8: rustyscript → V8  (Windows, macOS)
                                       └── js-quickjs: rquickjs → QuickJS  (Linux, iOS)
```

## Scope

Metadata / URL resolution only. No download API. The plugin resolves stream URLs; actual downloading is left to the caller (Unity's `UnityWebRequest`, FFmpeg, etc.).

## License

MIT
