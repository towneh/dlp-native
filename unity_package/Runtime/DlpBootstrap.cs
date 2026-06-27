using System;
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
    /// Both editor and runtime use the same extraction + init path.
    /// The only editor-specific step is resolving the source directory via
    /// PackageInfo (since the package's StreamingAssets are not at
    /// Application.streamingAssetsPath for local UPM packages).
    ///
    /// Usage: await DlpBootstrap.EnsureInitAsync() from MonoBehaviour Start
    /// or any editor script.
    /// </summary>
    public static class DlpBootstrap
    {
        // Bump when the stdlib bundle or yt-dlp version changes to force
        // re-extraction on the next launch.
        private const string DlpVersion = "0.1.0";

        /// <summary>
        /// When true, after init the engine checks PyPI for a newer yt-dlp and stages it for
        /// the next launch (see <see cref="DlpUpdater"/>). No-op on iOS. Set false before the
        /// first init call to pin to the bundled package.
        /// </summary>
        public static bool AutoUpdate = true;

        // Application.persistentDataPath is main-thread-only, but extraction and the
        // fire-and-forget update check run their continuations on the thread pool. Capture
        // it once at the (main-thread) entry points and read the cache off-thread.
        internal static string PersistentDataPath { get; private set; }

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
                {
                    PersistentDataPath = Application.persistentDataPath;
                    _initTask = RunInitAsync();
                }
                return _initTask;
            }
        }

        private static async Task RunInitAsync()
        {
            var paths = await PrepareAsync().ConfigureAwait(false);
            Debug.Log($"[YtDlp] pythonHome={paths.PythonHome} packagesPath={paths.PackagesPath}");
            YtDlpApi.EnsureInit(paths);

            // Fire-and-forget: stage a newer yt-dlp for the next launch. Never throws and
            // never touches the interpreter that just booted (re-init is unsafe).
            if (AutoUpdate)
                _ = DlpUpdater.CheckAndStageAsync(DlpVersion, DlpUpdater.ReadPackagesVersion(paths.PackagesPath));
        }

        /// <summary>
        /// Resolve DlpPaths without calling unity_dlp_init.
        /// Extracts stdlib and yt_dlp.zip to persistentDataPath if needed.
        /// </summary>
        public static async Task<DlpPaths> PrepareAsync()
        {
            // Runs synchronously on the caller's (main) thread up to the first await.
            PersistentDataPath = Application.persistentDataPath;
#if UNITY_EDITOR
            // PackageInfo.FindForAssembly must run on the main thread.
            // Resolve it synchronously here, before any await, while we are
            // still on the calling thread.
            var info = UnityEditor.PackageManager.PackageInfo
                .FindForAssembly(typeof(DlpBootstrap).Assembly);
            if (info == null)
                throw new InvalidOperationException(
                    "Cannot locate the YtDlp package — is it installed via UPM?");
            var srcDlpDir = Path.Combine(info.resolvedPath, "StreamingAssets", "dlp");
#else
            var srcDlpDir = Path.Combine(Application.streamingAssetsPath, "dlp");
#endif
            return await PrepareFromDirAsync(srcDlpDir).ConfigureAwait(false);
        }

        // Shared extraction path used by both editor and runtime.
        private static async Task<DlpPaths> PrepareFromDirAsync(string srcDlpDir)
        {
            var baseDir    = Path.Combine(PersistentDataPath, "dlp", DlpVersion);
            var markerPath = Path.Combine(baseDir, ".ready");

            if (!File.Exists(markerPath))
            {
                await ExtractStdlibAsync(srcDlpDir, baseDir).ConfigureAwait(false);
                await CopyPackagesAsync(srcDlpDir, baseDir).ConfigureAwait(false);
                File.WriteAllText(markerPath, DlpVersion);
            }

            return new DlpPaths(
                pythonHome:   Path.Combine(baseDir, "python"),
                packagesPath: DlpUpdater.ResolvePackagesPath(baseDir, DlpVersion));
        }

        private static async Task ExtractStdlibAsync(string srcDlpDir, string baseDir)
        {
            var platformId = GetPlatformId();
            var srcPath    = Path.Combine(srcDlpDir, "stdlib", platformId + ".zip");
            var destDir    = Path.Combine(baseDir, "python");

            var bytes = await ReadFileAsync(srcPath).ConfigureAwait(false);
            await Task.Run(() => ExtractZip(bytes, destDir)).ConfigureAwait(false);
        }

        private static async Task CopyPackagesAsync(string srcDlpDir, string baseDir)
        {
            var srcPath  = Path.Combine(srcDlpDir, "yt_dlp.zip");
            var destPath = Path.Combine(baseDir, "yt_dlp.zip");

            var bytes = await ReadFileAsync(srcPath).ConfigureAwait(false);
            await Task.Run(() =>
            {
                Directory.CreateDirectory(Path.GetDirectoryName(destPath)!);
                File.WriteAllBytes(destPath, bytes);
            }).ConfigureAwait(false);
        }

        private static Task<byte[]> ReadFileAsync(string path)
        {
#if UNITY_ANDROID && !UNITY_EDITOR
            return ReadViaWebRequestAsync(path);
#else
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
                throw new IOException($"Failed to read '{path}': {req.error}");
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
