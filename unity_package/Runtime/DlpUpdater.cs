using System;
using System.IO;
using System.IO.Compression;
using System.Linq;
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

        // sys.path entries are joined with '\n' rather than a platform separator
        // because ';' and ':' are each legal path characters on one OS or the other.
        // Kept in lock-step with the native host's parsing (python_host::init).
        internal const char PathListSeparator = '\n';

        // The three top-level packages the bundled zip must carry. A staged yt-dlp
        // wheel replaces only yt_dlp/; yt_dlp_ejs/ and unity_dlp_jsc/ always load
        // from the bundle behind it.
        private static readonly string[] BundledPackages = { "yt_dlp/", "yt_dlp_ejs/", "unity_dlp_jsc/" };

        private static readonly HttpClient Http = new HttpClient
        {
            Timeout = TimeSpan.FromMinutes(2),
        };

        public enum Outcome { Disabled, UpToDate, Incompatible, Staged, Failed }

        /// <summary>
        /// Resolves the packages list <see cref="DlpBootstrap"/> should hand to init.
        /// When a previously-staged update is valid for this host, returns the staged
        /// wheel and the bundled zip joined by <see cref="PathListSeparator"/> — yt_dlp
        /// loads from the staged wheel while yt_dlp_ejs and unity_dlp_jsc keep loading
        /// from the bundle behind it. Otherwise returns the bundled zip at
        /// <paramref name="bundledBaseDir"/> alone. "Valid" means the pointer names this
        /// <paramref name="dlpVersion"/> and <see cref="EmbeddedPython"/>, the file exists,
        /// its bytes still hash to the recorded digest (guards a corrupted/tampered cache),
        /// and it is yt-dlp-shaped (contains <c>yt_dlp/version.py</c>). Any mismatch falls
        /// back to the bundled zip — never throws.
        /// </summary>
        internal static string ResolvePackagesPath(string bundledBaseDir, string dlpVersion)
        {
            var bundled = Path.Combine(bundledBaseDir, "yt_dlp.zip");
            VerifyBundleContents(bundled);
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

                // A staged update must be a yt-dlp-shaped wheel. Anything else — a wrong
                // artifact, or a wheel staged under an older scheme that replaced the whole
                // bundle — falls back to the bundle, self-healing installs broken in the field.
                if (!ZipHasEntry(file, "yt_dlp/version.py")) return bundled;

                return file + PathListSeparator + bundled;
            }
            catch (Exception e)
            {
                Debug.LogWarning($"[YtDlp] update pointer unreadable, using bundled yt-dlp: {e.Message}");
                return bundled;
            }
        }

        // Warns if the bundled zip is missing any of the three top-level packages, so a
        // build that dropped one is self-diagnosing rather than surfacing later as an
        // opaque init failure. Best-effort: a read error is a warning, not fatal.
        private static void VerifyBundleContents(string bundledZip)
        {
            try
            {
                if (!File.Exists(bundledZip))
                {
                    Debug.LogError($"[YtDlp] bundled yt_dlp.zip is missing at {bundledZip}.");
                    return;
                }
                using var fs      = File.OpenRead(bundledZip);
                using var archive = new ZipArchive(fs, ZipArchiveMode.Read);
                foreach (var pkg in BundledPackages)
                    if (!archive.Entries.Any(e => e.FullName.StartsWith(pkg, StringComparison.Ordinal)))
                        Debug.LogError($"[YtDlp] bundled yt_dlp.zip is missing the '{pkg}' package — rebuild the native host.");
            }
            catch (Exception e)
            {
                Debug.LogWarning($"[YtDlp] could not verify bundled yt_dlp.zip contents: {e.Message}");
            }
        }

        // True if a zip contains an entry with exactly this name. Swallows read errors
        // (corrupt/unopenable zip) as false so callers can treat it as "not present".
        private static bool ZipHasEntry(string zipPath, string entryName)
        {
            try
            {
                using var fs      = File.OpenRead(zipPath);
                using var archive = new ZipArchive(fs, ZipArchiveMode.Read);
                return archive.GetEntry(entryName) != null;
            }
            catch { return false; }
        }

        // Drops the active staged update by deleting its pointer, so the next launch boots
        // pure-bundle. Called when init failed or degraded while a staged wheel was on the
        // path — the matched bundled pair is the safe fallback. Best-effort: never throws.
        internal static void QuarantineStagedUpdate(string reason)
        {
            try
            {
                var pointer = Path.Combine(UpdatesRoot(), PointerFile);
                if (!File.Exists(pointer)) return;
                File.Delete(pointer);
                Debug.LogWarning($"[YtDlp] quarantined staged yt-dlp update ({reason}); " +
                                 $"next launch uses the bundled package.");
            }
            catch (Exception e)
            {
                Debug.LogWarning($"[YtDlp] failed to quarantine staged update: {e.Message}");
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

                if (!EjsRequirementSatisfied(bytes, meta.Version, dlpVersion, out var incompatReason))
                {
                    Debug.Log($"[YtDlp] {incompatReason}");
                    return Outcome.Incompatible;
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

        // Reads yt-dlp's __version__ (yt_dlp/version.py) from the first entry of a
        // <see cref="PathListSeparator"/>-delimited packages list that actually carries it —
        // i.e. the yt-dlp that will load — so the updater compares against the right version.
        // Accepts a single path too (no separator). Null if none can be read, in which case
        // any candidate counts as newer.
        internal static string ReadPackagesVersion(string packagesPath)
        {
            if (string.IsNullOrEmpty(packagesPath)) return null;
            foreach (var entry in packagesPath.Split(PathListSeparator))
            {
                var version = ReadYtDlpVersionFromZip(entry.Trim());
                if (version != null) return version;
            }
            return null;
        }

        private static string ReadYtDlpVersionFromZip(string zipPath)
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

        // The bundle's yt_dlp_ejs is fixed at build time. A staged yt-dlp that needs a newer
        // yt-dlp-ejs than the bundle carries would fail its YouTube path, so it's skipped
        // until the host is rebuilt. Returns true (stage) when no ejs requirement is declared
        // or the bundled version satisfies it; false (skip, with a reason) when unsatisfied or
        // the requirement can't be read/parsed — mirroring PythonSatisfies: never let through
        // a possibly-incompatible package.
        private static bool EjsRequirementSatisfied(
            byte[] wheelBytes, string ytDlpVersion, string dlpVersion, out string reason)
        {
            reason = null;

            string requirement;
            try { requirement = ReadEjsRequirement(wheelBytes); }
            catch (Exception e)
            {
                reason = $"could not read yt-dlp {ytDlpVersion} wheel METADATA ({e.Message}) — skipping.";
                return false;
            }

            if (string.IsNullOrEmpty(requirement)) return true; // no ejs dependency declared

            var have = ReadBundledEjsVersion(dlpVersion);
            if (string.IsNullOrEmpty(have))
            {
                reason = $"yt-dlp {ytDlpVersion} needs yt-dlp-ejs {requirement}; " +
                         $"bundled ejs version unreadable — skipping.";
                return false;
            }

            if (!EjsSatisfies(requirement, have))
            {
                reason = $"yt-dlp {ytDlpVersion} needs yt-dlp-ejs {requirement}; host bundles {have} " +
                         $"— skipping until the host is rebuilt.";
                return false;
            }
            return true;
        }

        // Parses the "Requires-Dist: yt-dlp-ejs …" version specifier from the wheel's
        // yt_dlp-*.dist-info/METADATA. Returns the specifier (e.g. ">=0.8.0"), an empty
        // string if the dependency is declared without a constraint, or null if the wheel
        // declares no yt-dlp-ejs dependency at all. Name normalisation treats '-'/'_' alike.
        private static string ReadEjsRequirement(byte[] wheelBytes)
        {
            using var ms      = new MemoryStream(wheelBytes);
            using var archive = new ZipArchive(ms, ZipArchiveMode.Read);
            var metadata = archive.Entries.FirstOrDefault(e =>
                Regex.IsMatch(e.FullName, @"^yt_dlp-[^/]+\.dist-info/METADATA$"));
            if (metadata == null) return null;

            using var reader = new StreamReader(metadata.Open());
            string line;
            while ((line = reader.ReadLine()) != null)
            {
                // "Requires-Dist: yt-dlp-ejs>=0.8.0" or "…: yt-dlp-ejs (>=0.8.0)", optionally
                // followed by "; extra == '…'" markers we ignore.
                var m = Regex.Match(line,
                    @"^Requires-Dist:\s*yt[-_]dlp[-_]ejs\s*(?:\(([^)]*)\)|([^;]*))",
                    RegexOptions.IgnoreCase);
                if (!m.Success) continue;
                return (m.Groups[1].Success ? m.Groups[1].Value : m.Groups[2].Value).Trim();
            }
            return null;
        }

        private static string ReadBundledEjsVersion(string dlpVersion)
        {
            try
            {
                var bundled = Path.Combine(DlpBootstrap.PersistentDataPath, "dlp", dlpVersion, "yt_dlp.zip");
                if (!File.Exists(bundled)) return null;
                using var fs      = File.OpenRead(bundled);
                using var archive = new ZipArchive(fs, ZipArchiveMode.Read);
                var entry = archive.GetEntry("yt_dlp_ejs/_version.py");
                if (entry == null) return null;
                using var reader = new StreamReader(entry.Open());
                var m = Regex.Match(reader.ReadToEnd(), @"(?m)^\s*version\s*=\s*['""]([^'""]+)['""]");
                return m.Success ? m.Groups[1].Value : null;
            }
            catch { return null; }
        }

        // Conservative PEP 440 check that bundled yt-dlp-ejs `have` satisfies `requirement`
        // (comma-separated clauses, all of which must hold). Handles ==, !=, >=, <=, >, <, ~=
        // and trailing ".*" wildcards on dotted-numeric versions. Any unparseable clause
        // returns false so a possibly-incompatible update is never staged.
        private static bool EjsSatisfies(string requirement, string have)
        {
            if (!TryParseVersion(have, out var hv)) return false;

            foreach (var raw in requirement.Split(','))
            {
                var clause = raw.Trim();
                if (clause.Length == 0) continue;

                string op = clause.StartsWith(">=") || clause.StartsWith("<=") || clause.StartsWith("==") ||
                            clause.StartsWith("!=") || clause.StartsWith("~=")
                    ? clause.Substring(0, 2)
                    : (clause[0] == '>' || clause[0] == '<') ? clause.Substring(0, 1) : null;
                if (op == null) return false;

                var rest     = clause.Substring(op.Length).Trim();
                bool wildcard = rest.EndsWith(".*", StringComparison.Ordinal);
                if (!TryParseVersion(rest.TrimEnd('*', '.'), out var cv)) return false;

                int c = CompareVersions(hv, cv);
                bool ok = op switch
                {
                    ">=" => c >= 0,
                    "<=" => c <= 0,
                    ">"  => c >  0,
                    "<"  => c <  0,
                    "==" => wildcard ? StartsWithPrefix(hv, cv) : c == 0,
                    "!=" => wildcard ? !StartsWithPrefix(hv, cv) : c != 0,
                    "~=" => c >= 0,   // compatible-release lower bound
                    _    => false,
                };
                if (!ok) return false;
            }
            return true;
        }

        // "0.8.0" → [0, 8, 0]. Fails on any non-numeric segment (e.g. "0.8.0rc1"), so an
        // unparseable version is treated conservatively by the caller.
        private static bool TryParseVersion(string v, out int[] parts)
        {
            parts = null;
            if (string.IsNullOrEmpty(v)) return false;
            var segs   = v.Split('.');
            var result = new int[segs.Length];
            for (int i = 0; i < segs.Length; i++)
                if (!int.TryParse(segs[i], out result[i])) return false;
            parts = result;
            return true;
        }

        private static int CompareVersions(int[] a, int[] b)
        {
            int n = Math.Max(a.Length, b.Length);
            for (int i = 0; i < n; i++)
            {
                int ai = i < a.Length ? a[i] : 0;
                int bi = i < b.Length ? b[i] : 0;
                if (ai != bi) return ai.CompareTo(bi);
            }
            return 0;
        }

        private static bool StartsWithPrefix(int[] have, int[] prefix)
        {
            for (int i = 0; i < prefix.Length; i++)
                if (i >= have.Length || have[i] != prefix[i]) return false;
            return true;
        }

        // DlpBootstrap.PersistentDataPath is captured on the main thread; this runs from
        // thread-pool continuations where Application.persistentDataPath would throw.
        private static string UpdatesRoot()
            => Path.Combine(DlpBootstrap.PersistentDataPath, "dlp", UpdatesDir);

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
