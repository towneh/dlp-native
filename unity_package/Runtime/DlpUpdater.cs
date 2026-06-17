using System;
using System.IO;
using System.IO.Compression;
using System.Net.Http;
using System.Security.Cryptography;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;
using Newtonsoft.Json.Linq;
using UnityEngine;

namespace YtDlp
{
    /// <summary>
    /// Keeps the bundled yt-dlp package current without rebuilding the native host.
    /// yt-dlp is the part of the engine that ages fastest (YouTube changes its player
    /// JS / formats often); the embedded CPython and the C ABI do not. So this fetches a
    /// newer pure-Python yt-dlp from PyPI, verifies it, and stages it for the next launch
    /// — the running interpreter keeps the zip it booted with (re-init is unsafe), and
    /// <see cref="DlpBootstrap"/> prefers the staged copy the next time it builds DlpPaths.
    ///
    /// Only the yt-dlp package is updatable this way. The Python stdlib is tied to the
    /// embedded interpreter and only changes on a host rebuild (a <c>DlpVersion</c> bump),
    /// and compiled extensions (e.g. curl_cffi for impersonation) can't ship as a zip —
    /// both stay with the native host build.
    /// </summary>
    public static class DlpUpdater
    {
        // The embedded interpreter's minor version. A candidate yt-dlp whose
        // requires-python excludes this is not installable here, so it's skipped rather
        // than staged into a host that can't run it. Bump alongside the host's CPython.
        internal const string EmbeddedPython = "3.12";

        private const string PyPiJsonUrl = "https://pypi.org/pypi/yt-dlp/json";
        private const string UpdatesDir  = "updates";
        private const string PointerFile = "current.json";

        private static readonly HttpClient Http = new HttpClient
        {
            Timeout = TimeSpan.FromMinutes(2),
        };

        public enum Outcome { Disabled, UpToDate, Incompatible, Staged, Failed }

        /// <summary>
        /// Resolves the yt-dlp zip <see cref="DlpBootstrap"/> should hand to init: a
        /// previously-staged update when one is valid for this host, otherwise the bundled
        /// zip at <paramref name="bundledBaseDir"/>. "Valid" means the pointer names this
        /// <paramref name="dlpVersion"/> and <see cref="EmbeddedPython"/>, the file exists,
        /// and its bytes still hash to the recorded digest (guards a corrupted/tampered
        /// cache). Any mismatch falls back to the bundled zip — never throws.
        /// </summary>
        internal static string ResolvePackagesPath(string bundledBaseDir, string dlpVersion)
        {
            var bundled = Path.Combine(bundledBaseDir, "yt_dlp.zip");
            try
            {
                var pointerPath = Path.Combine(UpdatesRoot(), PointerFile);
                if (!File.Exists(pointerPath)) return bundled;

                var rec = JObject.Parse(File.ReadAllText(pointerPath));
                if ((string)rec["forDlpVersion"] != dlpVersion) return bundled;
                if ((string)rec["forPython"]     != EmbeddedPython) return bundled;

                var file   = Path.Combine(UpdatesRoot(), (string)rec["fileName"] ?? string.Empty);
                var sha256 = (string)rec["sha256"];
                if (string.IsNullOrEmpty(sha256) || !File.Exists(file)) return bundled;
                if (!Sha256Hex(File.ReadAllBytes(file)).Equals(sha256, StringComparison.OrdinalIgnoreCase))
                    return bundled;

                return file;
            }
            catch (Exception e)
            {
                Debug.LogWarning($"[YtDlp] update pointer unreadable, using bundled yt-dlp: {e.Message}");
                return bundled;
            }
        }

        /// <summary>
        /// Checks PyPI for a newer yt-dlp and, if one is compatible and verified, stages it
        /// for the next launch. Safe to fire-and-forget after init — it never throws and
        /// never touches the running interpreter. <paramref name="currentVersion"/> is the
        /// version actually loaded this run (<c>YtDlpApi.Version()</c>); nothing is staged
        /// unless the candidate is strictly newer.
        /// </summary>
        public static async Task<Outcome> CheckAndStageAsync(
            string dlpVersion, string currentVersion, CancellationToken cancellationToken = default)
        {
#if UNITY_IOS && !UNITY_EDITOR
            // The App Store forbids downloading and executing new code at runtime, so iOS is
            // pinned to whatever yt-dlp shipped in the build; refresh comes via an app update.
            await Task.CompletedTask;
            return Outcome.Disabled;
#else
            try
            {
                var meta = await FetchLatestAsync(cancellationToken).ConfigureAwait(false);
                if (meta == null) return Outcome.Failed;

                if (!string.IsNullOrEmpty(currentVersion)
                    && CompareVersions(meta.Version, currentVersion) <= 0)
                    return Outcome.UpToDate;

                if (!PythonSatisfies(meta.RequiresPython, EmbeddedPython))
                {
                    Debug.Log($"[YtDlp] yt-dlp {meta.Version} needs Python {meta.RequiresPython}; " +
                              $"host has {EmbeddedPython} — skipping until the host is rebuilt.");
                    return Outcome.Incompatible;
                }

                var bytes = await Http.GetByteArrayAsync(meta.Url).ConfigureAwait(false);
                var hash  = Sha256Hex(bytes);
                if (!hash.Equals(meta.Sha256, StringComparison.OrdinalIgnoreCase))
                {
                    Debug.LogError($"[YtDlp] update checksum mismatch for yt-dlp {meta.Version} " +
                                   $"(expected {meta.Sha256}, got {hash}) — discarded.");
                    return Outcome.Failed;
                }

                Stage(bytes, meta, dlpVersion);
                Debug.Log($"[YtDlp] staged yt-dlp {meta.Version}; active on next launch.");
                return Outcome.Staged;
            }
            catch (OperationCanceledException) { return Outcome.Failed; }
            catch (Exception e)
            {
                Debug.LogWarning($"[YtDlp] update check failed (keeping current yt-dlp): {e.Message}");
                return Outcome.Failed;
            }
#endif
        }

        // The PyPI wheel (a `yt_dlp/`-rooted zip) is used directly on sys.path, exactly like
        // the bundled yt_dlp.zip — the dist-info alongside it is inert to zipimport. Stored
        // under .zip so its provenance is unambiguous to ResolvePackagesPath.
        private static void Stage(byte[] bytes, ReleaseMeta meta, string dlpVersion)
        {
            var root = UpdatesRoot();
            Directory.CreateDirectory(root);

            var fileName = $"yt_dlp-{meta.Version}.zip";
            var dest     = Path.Combine(root, fileName);
            var tmp      = dest + ".tmp";

            File.WriteAllBytes(tmp, bytes);
            if (File.Exists(dest)) File.Delete(dest);
            File.Move(tmp, dest);

            var previous = TryReadPointerFile();

            var pointer = new JObject
            {
                ["version"]       = meta.Version,
                ["sha256"]        = meta.Sha256,
                ["fileName"]      = fileName,
                ["forDlpVersion"] = dlpVersion,
                ["forPython"]     = EmbeddedPython,
            };
            File.WriteAllText(Path.Combine(root, PointerFile), pointer.ToString());

            // Prune the file the pointer used to name (a superseded update), now that the
            // pointer no longer references it.
            if (previous != null && previous != fileName)
            {
                var stale = Path.Combine(root, previous);
                if (File.Exists(stale)) { try { File.Delete(stale); } catch { /* best effort */ } }
            }
        }

        private static string TryReadPointerFile()
        {
            try
            {
                var p = Path.Combine(UpdatesRoot(), PointerFile);
                return File.Exists(p) ? (string)JObject.Parse(File.ReadAllText(p))["fileName"] : null;
            }
            catch { return null; }
        }

        private sealed class ReleaseMeta
        {
            public string Version;
            public string Url;
            public string Sha256;
            public string RequiresPython;
        }

        private static async Task<ReleaseMeta> FetchLatestAsync(CancellationToken cancellationToken)
        {
            var json = await Http.GetStringAsync(PyPiJsonUrl).ConfigureAwait(false);
            cancellationToken.ThrowIfCancellationRequested();

            var root    = JObject.Parse(json);
            var version = (string)root["info"]?["version"];
            if (string.IsNullOrEmpty(version)) return null;

            // The pure-Python wheel for the latest release; its digest anchors integrity.
            foreach (var url in root["urls"] ?? new JArray())
            {
                if ((string)url["packagetype"] != "bdist_wheel") continue;
                var fileUrl = (string)url["url"];
                var sha256  = (string)url["digests"]?["sha256"];
                if (string.IsNullOrEmpty(fileUrl) || string.IsNullOrEmpty(sha256)) continue;

                return new ReleaseMeta
                {
                    Version        = version,
                    Url            = fileUrl,
                    Sha256         = sha256,
                    // Prefer the file's own constraint, falling back to the project's.
                    RequiresPython = (string)url["requires_python"]
                                  ?? (string)root["info"]?["requires_python"],
                };
            }
            return null;
        }

        // Reads yt-dlp's __version__ from a packages zip (yt_dlp/version.py) without importing
        // it — lets the updater compare against whatever is on disk, before init completes.
        // Null if the zip can't be read, in which case any candidate counts as newer.
        internal static string ReadPackagesVersion(string zipPath)
        {
            try
            {
                if (string.IsNullOrEmpty(zipPath) || !File.Exists(zipPath)) return null;
                using var fs      = File.OpenRead(zipPath);
                using var archive = new ZipArchive(fs, ZipArchiveMode.Read);
                var entry = archive.GetEntry("yt_dlp/version.py");
                if (entry == null) return null;
                using var reader = new StreamReader(entry.Open());
                var match = Regex.Match(reader.ReadToEnd(), @"__version__\s*=\s*['""]([^'""]+)['""]");
                return match.Success ? match.Groups[1].Value : null;
            }
            catch { return null; }
        }

        private static string UpdatesRoot()
            => Path.Combine(Application.persistentDataPath, "dlp", UpdatesDir);

        private static string Sha256Hex(byte[] bytes)
        {
            using var sha = SHA256.Create();
            var hash = sha.ComputeHash(bytes);
            var sb = new System.Text.StringBuilder(hash.Length * 2);
            foreach (var b in hash) sb.Append(b.ToString("x2"));
            return sb.ToString();
        }

        // yt-dlp versions are date-stamped (e.g. 2025.06.09[.suffix]); compare field by
        // field, numerically where both fields are numeric, else lexically. Returns
        // negative if a < b, positive if a > b, zero if equal.
        private static int CompareVersions(string a, string b)
        {
            var pa = (a ?? string.Empty).Split('.');
            var pb = (b ?? string.Empty).Split('.');
            int n = Math.Max(pa.Length, pb.Length);
            for (int i = 0; i < n; i++)
            {
                var sa = i < pa.Length ? pa[i] : "0";
                var sb = i < pb.Length ? pb[i] : "0";
                int cmp = int.TryParse(sa, out var ia) && int.TryParse(sb, out var ib)
                    ? ia.CompareTo(ib)
                    : string.CompareOrdinal(sa, sb);
                if (cmp != 0) return cmp;
            }
            return 0;
        }

        // Conservative PEP 440 check over (major, minor): every comma-separated clause must
        // hold for `have` to be installable. Unrecognised input returns false so an
        // unparseable constraint never lets through a possibly-incompatible package.
        private static bool PythonSatisfies(string requiresPython, string have)
        {
            if (string.IsNullOrWhiteSpace(requiresPython)) return true; // unconstrained
            if (!TryParseMinor(have, out var hv)) return false;

            foreach (var raw in requiresPython.Split(','))
            {
                var clause = raw.Trim();
                if (clause.Length == 0) continue;

                string op = clause.StartsWith(">=") || clause.StartsWith("<=") || clause.StartsWith("==") ||
                            clause.StartsWith("!=") || clause.StartsWith("~=")
                    ? clause.Substring(0, 2)
                    : (clause[0] == '>' || clause[0] == '<') ? clause.Substring(0, 1) : null;
                if (op == null) return false;

                if (!TryParseMinor(clause.Substring(op.Length).Trim().TrimEnd('*', '.'), out var cv))
                    return false;

                int c = hv.CompareTo(cv);
                bool ok = op switch
                {
                    ">=" => c >= 0,
                    "<=" => c <= 0,
                    ">"  => c >  0,
                    "<"  => c <  0,
                    "==" => c == 0,
                    "!=" => c != 0,
                    "~=" => c >= 0,   // compatible-release lower bound, good enough on (major,minor)
                    _    => false,
                };
                if (!ok) return false;
            }
            return true;
        }

        // Parses "3", "3.12", "3.12.1" → comparable (major*1000 + minor). Patch is ignored.
        private static bool TryParseMinor(string v, out int packed)
        {
            packed = 0;
            if (string.IsNullOrEmpty(v)) return false;
            var parts = v.Split('.');
            if (!int.TryParse(parts[0], out var major)) return false;
            int minor = 0;
            if (parts.Length > 1 && !int.TryParse(parts[1], out minor)) return false;
            packed = major * 1000 + minor;
            return true;
        }
    }
}
