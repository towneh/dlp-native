using System;
using System.IO;
using System.IO.Compression;
using System.Threading;
using System.Threading.Tasks;
using UnityEngine;
using UnityEngine.Networking;

namespace YtDlp
{
    /// <summary>
    /// Unpacks Python stdlib and yt_dlp.zip from StreamingAssets to
    /// persistentDataPath on first run, then calls unity_dlp_init.
    ///
    /// Usage (MonoBehaviour Start / Awake):
    ///   await DlpBootstrap.EnsureInitAsync();
    /// </summary>
    public static class DlpBootstrap
    {
        // Bump this when the stdlib bundle or yt-dlp version changes so the
        // next launch re-extracts to a fresh directory.
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
        /// Extract assets and return the resulting DlpPaths.
        /// Skips extraction if the version marker already exists.
        /// </summary>
        public static async Task<DlpPaths> PrepareAsync()
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
            ExtractZip(bytes, destDir);
        }

        private static async Task CopyPackagesAsync(string baseDir)
        {
            var srcUri   = Path.Combine(Application.streamingAssetsPath, "dlp", "yt_dlp.zip");
            var destPath = Path.Combine(baseDir, "yt_dlp.zip");

            var bytes = await ReadStreamingAssetAsync(srcUri);
            Directory.CreateDirectory(Path.GetDirectoryName(destPath)!);
            File.WriteAllBytes(destPath, bytes);
        }

        // On Android, Application.streamingAssetsPath is a jar:// URI.
        // UnityWebRequest handles both file:// and jar:// transparently.
        private static async Task<byte[]> ReadStreamingAssetAsync(string path)
        {
            using var req = UnityWebRequest.Get(path);
            var op = req.SendWebRequest();
            while (!op.isDone)
                await Task.Yield();

            if (req.result != UnityWebRequest.Result.Success)
                throw new IOException($"Failed to read streaming asset '{path}': {req.error}");

            return req.downloadHandler.data;
        }

        private static void ExtractZip(byte[] zipBytes, string destDir)
        {
            Directory.CreateDirectory(destDir);
            using var ms      = new MemoryStream(zipBytes);
            using var archive = new ZipArchive(ms, ZipArchiveMode.Read);
            foreach (var entry in archive.Entries)
            {
                if (string.IsNullOrEmpty(entry.Name))
                    continue; // directory entry

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
