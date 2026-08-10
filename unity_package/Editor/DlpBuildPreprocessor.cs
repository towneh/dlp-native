using System.IO;
using UnityEditor;
using UnityEditor.Build;
using UnityEditor.Build.Reporting;
using UnityEngine;
using PackageInfo = UnityEditor.PackageManager.PackageInfo;

namespace YtDlp.Editor
{
    /// <summary>
    /// Stages the Python stdlib zip for the active build target into
    /// StreamingAssets/dlp/stdlib/ before each player build.
    ///
    /// Skips if the zip already exists. Aborts the build if Python cannot
    /// be found or the script fails. Set DLP_PYTHON_HOME to override the
    /// Python prefix (otherwise uv python find 3.14 is used).
    ///
    /// Android and iOS stdlib zips must be staged by CI — this preprocessor
    /// only handles the three desktop targets automatically.
    /// </summary>
    public sealed class DlpBuildPreprocessor : IPreprocessBuildWithReport
    {
        public int callbackOrder => 0;

        public void OnPreprocessBuild(BuildReport report)
        {
            var platformId = ToPlatformId(report.summary.platform);
            if (platformId == null)
            {
                Debug.LogWarning(
                    $"[YtDlp] {report.summary.platform} stdlib must be staged by CI — " +
                    "auto-staging only supports Windows/macOS/Linux desktop builds.");
                return;
            }

            var pkg = PackageInfo.FindForAssembly(typeof(DlpBuildPreprocessor).Assembly);
            if (pkg == null)
                throw new BuildFailedException("[YtDlp] Cannot find YtDlp package path.");

            var pkgDlpDir  = Path.Combine(pkg.resolvedPath, "StreamingAssets", "dlp");
            var stdlibZip  = Path.Combine(pkgDlpDir, "stdlib", platformId + ".zip");
            var ytDlpZip   = Path.Combine(pkgDlpDir, "yt_dlp.zip");

            // Stage stdlib into the package if it's missing
            if (!File.Exists(stdlibZip))
            {
                var python = FindPython();
                if (python == null)
                    throw new BuildFailedException(
                        "[YtDlp] Python 3.x not found. " +
                        "Set DLP_PYTHON_HOME to your Python prefix, or run: uv python install 3.14");

                Debug.Log($"[YtDlp] Staging stdlib/{platformId}.zip using {python} …");
                RunStageScript(python, platformId, stdlibZip);

                if (!File.Exists(stdlibZip))
                    throw new BuildFailedException(
                        $"[YtDlp] Stdlib staging failed — {stdlibZip} was not created.");
            }

            if (!File.Exists(ytDlpZip))
                throw new BuildFailedException(
                    $"[YtDlp] yt_dlp.zip not found at {ytDlpZip}. Run the build script first.");

            // Copy both assets into the project's Assets/StreamingAssets/dlp/ so Unity
            // includes them in the player build (UPM package StreamingAssets are not
            // reliably copied into player builds by the build pipeline).
            CopyToProjectStreamingAssets(stdlibZip, ytDlpZip, platformId);
            AssetDatabase.Refresh();
            Debug.Log($"[YtDlp] DLP assets staged into project StreamingAssets.");
        }

        // ── Also expose as a menu item for manual / on-demand staging ─────────

        [MenuItem("Tools/YtDlp/Stage stdlib for current platform")]
        public static void StageManual()
        {
            var platformId = ToPlatformId(EditorUserBuildSettings.activeBuildTarget);
            if (platformId == null)
            {
                Debug.LogWarning("[YtDlp] No auto-staging for the current build target.");
                return;
            }

            var pkg = PackageInfo.FindForAssembly(typeof(DlpBuildPreprocessor).Assembly);
            if (pkg == null) { Debug.LogError("[YtDlp] Cannot find package path."); return; }

            var pkgDlpDir = Path.Combine(pkg.resolvedPath, "StreamingAssets", "dlp");
            var stdlibZip = Path.Combine(pkgDlpDir, "stdlib", platformId + ".zip");
            var ytDlpZip  = Path.Combine(pkgDlpDir, "yt_dlp.zip");

            if (!File.Exists(stdlibZip))
            {
                var python = FindPython();
                if (python == null) { Debug.LogError("[YtDlp] Python not found."); return; }

                RunStageScript(python, platformId, stdlibZip);

                if (!File.Exists(stdlibZip))
                {
                    Debug.LogError($"[YtDlp] Staging failed — {stdlibZip} not created.");
                    return;
                }
            }

            if (!File.Exists(ytDlpZip))
            {
                Debug.LogError($"[YtDlp] yt_dlp.zip not found at {ytDlpZip}. Run the build script first.");
                return;
            }

            CopyToProjectStreamingAssets(stdlibZip, ytDlpZip, platformId);
            AssetDatabase.Refresh();
            Debug.Log($"[YtDlp] DLP assets staged into project StreamingAssets.");
        }

        // ── Helpers ───────────────────────────────────────────────────────────

        private static void CopyToProjectStreamingAssets(
            string stdlibZip, string ytDlpZip, string platformId)
        {
            var projDlpDir    = Path.Combine(Application.dataPath, "StreamingAssets", "dlp");
            var projStdlibDir = Path.Combine(projDlpDir, "stdlib");
            Directory.CreateDirectory(projStdlibDir);

            var destStdlib = Path.Combine(projStdlibDir, platformId + ".zip");
            var destYtDlp  = Path.Combine(projDlpDir, "yt_dlp.zip");

            File.Copy(stdlibZip, destStdlib, overwrite: true);
            File.Copy(ytDlpZip,  destYtDlp,  overwrite: true);

            Debug.Log($"[YtDlp] Copied stdlib/{platformId}.zip → {destStdlib}");
            Debug.Log($"[YtDlp] Copied yt_dlp.zip → {destYtDlp}");
        }

        private static string ToPlatformId(BuildTarget t) => t switch
        {
            BuildTarget.StandaloneWindows or BuildTarget.StandaloneWindows64 => "windows-x86_64",
            BuildTarget.StandaloneOSX     => "macos-universal",
            BuildTarget.StandaloneLinux64 => "linux-x86_64",
            _                             => null,
        };

        private static string FindPython()
        {
            // 1. DLP_PYTHON_HOME is sys.prefix; derive the executable from it
            var home = System.Environment.GetEnvironmentVariable("DLP_PYTHON_HOME");
            if (!string.IsNullOrEmpty(home))
            {
                foreach (var rel in new[] { "python.exe", "bin/python3", "bin/python" })
                {
                    var p = Path.Combine(home, rel);
                    if (File.Exists(p)) return p;
                }
            }

            // 2. uv. --system keeps discovery off an active virtualenv, whose prefix has
            //    a Lib/ holding only site-packages; +gil keeps it off a free-threaded
            //    build. Matches PYTHON_REQUEST in .github/workflows/build.yml.
            var uv = Exec("uv", "python find --system 3.14+gil");
            if (!string.IsNullOrEmpty(uv) && File.Exists(uv)) return uv;

            return null;
        }

        /// <summary>
        /// Path to the staging script shipped inside the package. It lives under
        /// Python~/ so Unity does not import it as an asset, and it is the same file
        /// CI invokes — the player build and the CI build must not stage differently.
        /// </summary>
        private static string StageScriptPath()
        {
            var pkg = PackageInfo.FindForAssembly(typeof(DlpBuildPreprocessor).Assembly);
            if (pkg == null) return null;
            var path = Path.Combine(pkg.resolvedPath, "Python~", "stage_stdlib.py");
            return File.Exists(path) ? path : null;
        }

        private static void RunStageScript(string python, string platformId, string outZip)
        {
            var script = StageScriptPath();
            if (script == null)
                throw new BuildFailedException(
                    "[YtDlp] stage_stdlib.py not found in the package (expected Python~/stage_stdlib.py).");

            var outDir = Path.GetDirectoryName(outZip)!;
            Directory.CreateDirectory(outDir);

            // The script names the archive <platform>.zip itself, so it takes the
            // directory. Running it with the resolved interpreter is what selects the
            // prefix: with no --prefix or --python, it stages that interpreter's own.
            var ok = Exec(python, $"\"{script}\" {platformId} --out-dir \"{outDir}\"",
                          out var stdout, out var stderr);
            if (!string.IsNullOrEmpty(stdout)) Debug.Log($"[YtDlp] {stdout}");
            if (!ok)
                throw new BuildFailedException(
                    $"[YtDlp] stage_stdlib.py failed for {platformId}. {stderr}");
        }

        // stdout only, so a caller reading a path out of it is not handed diagnostics.
        private static string Exec(string exe, string args)
            => Exec(exe, args, out var stdout, out _) ? stdout : null;

        // False on a non-zero exit or a launch failure. stderr is returned separately
        // so a staging failure can say why rather than only that a file is absent.
        private static bool Exec(string exe, string args, out string stdout, out string stderr)
        {
            stdout = null;
            stderr = null;
            try
            {
                using var p = System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo
                {
                    FileName               = exe,
                    Arguments              = args,
                    RedirectStandardOutput = true,
                    RedirectStandardError  = true,
                    UseShellExecute        = false,
                    CreateNoWindow         = true,
                });
                stdout = p.StandardOutput.ReadToEnd().Trim();
                stderr = p.StandardError.ReadToEnd().Trim();
                p.WaitForExit(60_000);
                return p.ExitCode == 0;
            }
            catch (System.Exception e)
            {
                stderr = e.Message;
                return false;
            }
        }
    }
}
