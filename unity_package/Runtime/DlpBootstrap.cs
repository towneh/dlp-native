using System;
using System.Diagnostics;
using System.IO;
using System.IO.Compression;
using System.Threading.Tasks;
using UnityEngine;
#if UNITY_ANDROID && !UNITY_EDITOR
using UnityEngine.Networking;
#endif

namespace YtDlp
{
    /// <summary>
    /// Unpacks Python stdlib and yt_dlp.zip from StreamingAssets to
    /// persistentDataPath on first run, then calls unity_dlp_init.
    ///
    /// Editor: skips unpacking — uses the package's own StreamingAssets
    /// directly and locates the host Python via DLP_PYTHON_HOME or uv.
    ///
    /// Runtime: await DlpBootstrap.EnsureInitAsync() from MonoBehaviour Start.
    /// </summary>
    public static class DlpBootstrap
    {
        // Bump when the stdlib bundle or yt-dlp version changes to force
        // re-extraction on the next launch.
        private const string DlpVersion = "0.1.0";

        private static Task _initTask;
        private static readonly object _lock = new object();

        /// <summary>
        /// Idempotent async init. Safe to call from multiple MonoBehaviours;
        /// only the first caller does real work.
        /// </summary>
        public static Task EnsureInitAsync()
        {
            lock (_lock)
            {
                if (_initTask == null)
                    _initTask = RunInitAsync();
                return _initTask;
            }
        }

        private static async Task RunInitAsync()
        {
            var paths = await PrepareAsync();
            YtDlpApi.EnsureInit(paths);
        }

        /// <summary>
        /// Resolve DlpPaths without calling unity_dlp_init.
        /// In the editor this is a fast synchronous-ish lookup; at runtime it
        /// may extract assets from StreamingAssets to persistentDataPath.
        /// </summary>
        public static async Task<DlpPaths> PrepareAsync()
        {
#if UNITY_EDITOR
            return await PrepareEditorAsync();
#else
            return await PrepareRuntimeAsync();
#endif
        }

#if UNITY_EDITOR
        private static Task<DlpPaths> PrepareEditorAsync()
        {
            // Run everything on a thread-pool thread so blocking calls (Process,
            // File I/O) don't stall the Unity main thread.
            return Task.Run(() =>
            {
                var info = UnityEditor.PackageManager.PackageInfo
                    .FindForAssembly(typeof(DlpBootstrap).Assembly);
                if (info == null)
                    throw new InvalidOperationException(
                        "Cannot locate the YtDlp package — is it installed via UPM?");

                var ytDlpZip = Path.Combine(
                    info.resolvedPath, "StreamingAssets", "dlp", "yt_dlp.zip");
                if (!File.Exists(ytDlpZip))
                    throw new FileNotFoundException(
                        "yt_dlp.zip not found in package StreamingAssets. " +
                        "Run scripts/build-host.ps1 (Windows) or scripts/build-host.sh (macOS/Linux) first.",
                        ytDlpZip);

                var pythonHome = FindEditorPythonHome();
                return new DlpPaths(pythonHome, ytDlpZip);
            });
        }

        private static string FindEditorPythonHome()
        {
            // 1. Explicit override via environment variable
            var home = Environment.GetEnvironmentVariable("DLP_PYTHON_HOME");
            if (!string.IsNullOrEmpty(home)) return home;

            // 2. Ask uv for the Python 3.12 executable, then query its prefix
            try
            {
                var pyExe = RunProcess("uv", "python find 3.12");
                if (!string.IsNullOrEmpty(pyExe))
                {
                    var prefix = RunProcess(pyExe, "-c \"import sys; print(sys.prefix, end='')\"");
                    if (!string.IsNullOrEmpty(prefix)) return prefix;
                }
            }
            catch { /* uv not installed — fall through */ }

            return string.Empty; // Python will try to locate its own prefix
        }

        private static string RunProcess(string exe, string args)
        {
            using var p = Process.Start(new ProcessStartInfo
            {
                FileName               = exe,
                Arguments              = args,
                RedirectStandardOutput = true,
                UseShellExecute        = false,
                CreateNoWindow         = true,
            });
            p.WaitForExit(10_000);
            return p.StandardOutput.ReadToEnd().Trim();
        }
#endif // UNITY_EDITOR

        private static async Task<DlpPaths> PrepareRuntimeAsync()
        {
            var baseDir    = Path.Combine(Application.persistentDataPath, "dlp", DlpVersion);
            var markerPath = Path.Combine(baseDir, ".ready");

            if (!File.Exists(markerPath))
            {
                await ExtractStdlibAsync(baseDir);
                await CopyPackagesAsync(baseDir);
                File.WriteAllText(markerPath, DlpVersion);
            }

            return new DlpPaths(
                pythonHome:   Path.Combine(baseDir, "python"),
                packagesPath: Path.Combine(baseDir, "yt_dlp.zip"));
        }

        private static async Task ExtractStdlibAsync(string baseDir)
        {
            var platformId = GetPlatformId();
            var srcUri     = Path.Combine(
                Application.streamingAssetsPath, "dlp", "stdlib", platformId + ".zip");
            var destDir    = Path.Combine(baseDir, "python");

            var bytes = await ReadStreamingAssetAsync(srcUri);
            await Task.Run(() => ExtractZip(bytes, destDir));
        }

        private static async Task CopyPackagesAsync(string baseDir)
        {
            var srcUri   = Path.Combine(Application.streamingAssetsPath, "dlp", "yt_dlp.zip");
            var destPath = Path.Combine(baseDir, "yt_dlp.zip");

            var bytes = await ReadStreamingAssetAsync(srcUri);
            await Task.Run(() =>
            {
                Directory.CreateDirectory(Path.GetDirectoryName(destPath)!);
                File.WriteAllBytes(destPath, bytes);
            });
        }

        private static Task<byte[]> ReadStreamingAssetAsync(string path)
        {
#if UNITY_ANDROID && !UNITY_EDITOR
            return ReadViaWebRequestAsync(path);
#else
            // On all non-Android platforms streamingAssetsPath is a real
            // filesystem path — use direct I/O to avoid UWR + Task.Yield()
            // deadlocking when called from a synchronous context.
            return Task.Run(() => File.ReadAllBytes(path));
#endif
        }

#if UNITY_ANDROID && !UNITY_EDITOR
        private static async Task<byte[]> ReadViaWebRequestAsync(string path)
        {
            using var req = UnityEngine.Networking.UnityWebRequest.Get(path);
            var op = req.SendWebRequest();
            while (!op.isDone)
                await Task.Yield();
            if (req.result != UnityEngine.Networking.UnityWebRequest.Result.Success)
                throw new IOException($"Failed to read streaming asset '{path}': {req.error}");
            return req.downloadHandler.data;
        }
#endif

        private static void ExtractZip(byte[] zipBytes, string destDir)
        {
            Directory.CreateDirectory(destDir);
            using var ms      = new MemoryStream(zipBytes);
            using var archive = new ZipArchive(ms, ZipArchiveMode.Read);
            foreach (var entry in archive.Entries)
            {
                if (string.IsNullOrEmpty(entry.Name)) continue;
                var relative = entry.FullName.Replace('/', Path.DirectorySeparatorChar);
                var destFile = Path.Combine(destDir, relative);
                Directory.CreateDirectory(Path.GetDirectoryName(destFile)!);
                using var src = entry.Open();
                using var dst = File.Create(destFile);
                src.CopyTo(dst);
            }
        }

        private static string GetPlatformId()
        {
#if UNITY_EDITOR_WIN || UNITY_STANDALONE_WIN
            return "windows-x86_64";
#elif UNITY_EDITOR_OSX || UNITY_STANDALONE_OSX
            return "macos-universal";
#elif UNITY_EDITOR_LINUX || UNITY_STANDALONE_LINUX
            return "linux-x86_64";
#elif UNITY_ANDROID
            return "android-arm64-v8a";
#elif UNITY_IOS
            return "ios-arm64";
#else
            throw new PlatformNotSupportedException("Unsupported platform for DlpBootstrap");
#endif
        }
    }
}
