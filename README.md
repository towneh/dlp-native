# dlp-native

> **WARNING: THIS PROJECT IS A PROOF OF CONCEPT AND HAS BEEN ENTIRELY VIBE CODED. Treat it with all due suspicion.**

A Unity 6.3+ native plugin that embeds CPython + yt-dlp to extract media metadata (URL resolution only — no download) without spawning subprocesses at runtime. Built in Rust, consumed by C# P/Invoke. The Python stdlib and yt-dlp source ship as StreamingAssets zips and are unpacked on first run — the native binary carries only the interpreter and JS engine, so yt-dlp updates don't require rebuilding the plugin.

## What it does

Given a URL (YouTube, Vimeo, SoundCloud, and anything else yt-dlp supports), it returns the resolved media metadata as JSON — stream URLs, title, duration, thumbnails, formats. No subprocess is spawned; the Python interpreter and yt-dlp run in-process inside the native plugin.

YouTube's JS signature challenges are solved via an in-process JS engine: V8 (via [rustyscript](https://github.com/rscarson/rustyscript)) on Windows/macOS, [QuickJS](https://github.com/DelSkayn/rquickjs) on Linux, Android and iOS.

## Quick start

### 1. Get the binaries

The native binaries and the platform stdlib bundles are not in the repo — grab them from a release. No Rust or Python toolchain needed, and no GitHub auth. Download the zip for your platform from the [latest release](https://github.com/towneh/dlp-native/releases/latest) and extract its `Plugins/` and `StreamingAssets/` folders into this repo's `unity_package/`:

| Asset | Platform |
|-------|----------|
| `unity_dlp-windows-x64.zip` | Windows x86_64 |
| `unity_dlp-macos-universal.zip` | macOS arm64 + x86_64 |
| `unity_dlp-linux-x64.zip` | Linux x86_64 |
| `unity_dlp-android-arm64.zip` | Android arm64-v8a |
| `unity_dlp-ios-arm64.zip` | iOS arm64 |

The C# scripts come with the package source itself, so a release only fills in the parts that have to be compiled.

Prefer a build newer than the last release? Fetch the latest CI artifacts instead — this needs the [GitHub CLI](https://cli.github.com/) authenticated to this repo (`gh auth login`):

```powershell
pwsh scripts/fetch-artifacts.ps1 windows     # Windows
```
```bash
bash scripts/fetch-artifacts.sh macos        # macOS
bash scripts/fetch-artifacts.sh linux        # Linux
```

Pass several platform names at once (e.g. `windows android`), or omit them all to fetch every platform. Files land directly in `unity_package/`. Add `-Run <id>` on PowerShell, or `-r <id>` on bash, to take a specific CI run rather than the latest successful one on `main` — which is what you want when the build you care about is on a branch.

Desktop players stage their own stdlib at build time from the host's Python. Android and iOS cannot — theirs is cross-compiled — so the asset carries it, and a player build for either fails outright if it is missing rather than shipping an interpreter with no standard library.

To build the binary yourself instead, see [Building](#building).

### 2. Add the package to your project

`unity_package/` is a UPM package (`town.mr.ytdlp`). Add it through **Window → Package Manager → + → Add package from disk**, pointing at `unity_package/package.json`, or add the path to your project's `Packages/manifest.json`:

```json
"town.mr.ytdlp": "file:C:/path/to/dlp-native/unity_package"
```

Use an absolute path. Unity resolves relative `file:` entries inconsistently, and a wrong one fails at import with little explanation.

It depends on `com.unity.nuget.newtonsoft-json` (3.2.1), which Package Manager will pull in for you.

### 3. Call it

Initialise once, then extract. Both are awaitable and the native work happens on a dedicated worker thread, so neither blocks your frame:

```csharp
using UnityEngine;
using YtDlp;

public class Example : MonoBehaviour
{
    private async void Start()
    {
        // Unpacks the Python bundle on first run, then starts the interpreter.
        await DlpBootstrap.EnsureInitAsync();

        var info = await YtDlpApi.ExtractAsync("https://vimeo.com/76979871");

        Debug.Log($"{info.Title} ({info.Duration}s)");
        foreach (var f in info.Formats)
            Debug.Log($"{f.FormatId}  {f.Width}x{f.Height}  {f.Ext}  {f.Url}");
    }
}
```

Init is not instant, and nothing is shown while it runs. The first launch unpacks the Python bundle, and every launch starts the interpreter and pre-imports yt-dlp so the first extraction doesn't pay that cost out of its own timeout. On desktop the whole thing is around a second; on mobile hardware a cold first run can take ten seconds or more, during which a user who has just pasted a URL sees nothing happening. If extraction is reachable from UI, call `EnsureInitAsync` early (app start is fine, it's off the main thread) and surface your own "starting up" state until it completes.

`ExtractAsync` optionally takes an `ExtractOptions` to pick a format (`Format`, `FormatSort`, `GeoBypassCountry`). The result is the sanitised yt-dlp `info_dict`: `VideoInfo` exposes the common fields (`Title`, `Duration`, `Thumbnail`, `Uploader`, `IsLive`, `Chapters`, `Formats`, …), and each `Format` carries `Url`, `Ext`, `Width`, `Height`, codecs and bitrates.

There is a runnable version of this in the **Player Test** sample (Package Manager → Samples → Import). It is an `OnGUI` MonoBehaviour, so it needs no canvas setup, and it is the quickest way to confirm init and extraction work on a device.

## Limits and failure modes

Extraction is deliberately bounded — a media URL can come from anywhere, and the plugin runs in-process with no sandbox, so a hostile or merely slow page must not be able to wedge the host. The ceilings:

| Bound | Value | What happens when it's hit |
|-------|-------|---------------------------|
| JS execution | 5 s, 256 MB heap (64 MB on mobile) | The script is terminated; surfaces as a JavaScript error |
| Extraction deadline | 15 s, hard-stopped at 20 s | `TimeoutException` |
| Result size | 64 MB, 16 MB on Android/iOS | `YtDlpException` — "Result too large" |
| Concurrent extractions | 4 | `YtDlpException` — "Extractor busy" |

Failures arrive as typed exceptions:

- `TimeoutException` — the page took too long. Retrying may work; the same URL failing repeatedly will not.
- `YtDlpException` — extraction failed. `NativeCode` carries the underlying status and `IsRetryable` is true only for the busy case, so you can back off and retry without parsing message text.
- `InvalidOperationException` — the library was not initialised, or a buffer could not be sized.

The concurrency cap is unreachable through the C# wrapper, which funnels every native call through a single worker thread on purpose: CPython pins its interpreter and GIL to whichever thread started it. The cap exists for anything else calling the C ABI directly.

## Supported platforms

| Platform | JS engine | Notes |
|----------|-----------|-------|
| Windows x86_64 | V8 (rustyscript) | |
| macOS universal | V8 (rustyscript) | arm64 + x86_64 merged into one binary (`lipo`) |
| Linux x86_64 | QuickJS (rquickjs) | |
| Android arm64-v8a | QuickJS (rquickjs) | Termux supplies libpython, the libraries its C extensions need and a CA bundle (see [VENDOR.md](VENDOR.md)); the player must target ARM64 |
| iOS arm64 | QuickJS (rquickjs) | packaged as an `.xcframework` covering device + simulator, iOS 16.0+ |

CI builds every platform in this table on `main`, on pull requests, and on demand (`gh workflow run Build --ref <branch>`) — a push to a topic branch on its own does not trigger it. Each run keeps the five as artifacts; a release publishes the same five as assets.

## Keeping yt-dlp current

yt-dlp ages fastest — YouTube changes its player JS and formats often, while the embedded CPython and the native ABI rarely move. So the bundled yt-dlp refreshes itself at runtime rather than waiting for a plugin rebuild.

In short: after init, the plugin checks PyPI for a newer yt-dlp and stages it for the next launch. It is on by default. Set `DlpBootstrap.AutoUpdate = false` before the first init call to pin to the bundled version.

<details>
<summary>How the update actually works</summary>

`DlpBootstrap` kicks off `DlpUpdater` after init — fire-and-forget, on by default via `DlpBootstrap.AutoUpdate`. A newer release is downloaded, sha256-verified against the PyPI digest, and checked for compatibility with the embedded interpreter's Python version and the bundled `yt-dlp-ejs` before being staged. The running interpreter keeps the package it booted with, because re-initialising it is not safe.

On the next launch both the staged update and the bundled zip are placed on Python's import path, the update first: `yt_dlp` resolves from the update, while `yt_dlp_ejs` and the `unity_dlp_jsc` shim — neither of which a PyPI package carries — resolve from the bundle behind it. Anything wrong with the staged update (missing, hash mismatch, incompatible Python or ejs version, or not a yt-dlp package at all) falls back to the bundle alone, and the check never throws.

What this does not cover:

- The Python standard library is tied to the embedded interpreter and only changes on a plugin rebuild.
- Compiled extensions (e.g. `curl_cffi`) can't ship as a zip and stay with the build.
- iOS is pinned to the bundled package — the App Store forbids downloading and running new code at runtime, so it refreshes via an app update.

</details>

## Building

Only needed if you want to build the native binary yourself; releases and CI artifacts cover the usual case.

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

Windows, macOS, and Linux scripts require [uv](https://github.com/astral-sh/uv) with Python 3.14 installed (`uv python install 3.14`). iOS uses a static Python framework from [python-apple-support](https://github.com/beeware/python-apple-support) and does not need uv.

## Architecture

```
Unity C# (DlpBootstrap.cs + YtDlp.cs)
    ├── StreamingAssets/dlp/stdlib/<platform>.zip  ─┐ unpacked on first run to
    ├── StreamingAssets/dlp/yt_dlp.zip             ─┘ persistentDataPath, or to
    │                                                getFilesDir() on Android, whose
    │                                                loader will not open a library
    │                                                from external storage
    └── P/Invoke → unity_dlp.{dll,dylib,so} / libunity_dlp.a
                       └── Rust (unity_dlp_core)
                               ├── PyO3 → CPython 3.14 (interpreter only)
                               │             └── yt-dlp + yt_dlp_ejs + unity_dlp_jsc (loaded from filesystem)
                               └── JS engine (feature-selected at build time)
                                       ├── js-v8: rustyscript → V8  (Windows, macOS)
                                       └── js-quickjs: rquickjs → QuickJS  (Linux, Android, iOS)
```

## Scope

Metadata / URL resolution only. No download API. The plugin resolves stream URLs; actual downloading is left to the caller (Unity's `UnityWebRequest`, FFmpeg, etc.).

## License

MIT. Originally written by [yewnyx](https://github.com/yewnyx); this repository is the
maintained continuation. See [LICENSE](LICENSE).
