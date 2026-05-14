#if UNITY_EDITOR
using System;
using System.Threading.Tasks;
using UnityEditor;
using UnityEngine;

namespace YtDlp.Editor
{
    [InitializeOnLoad]
    internal static class YtDlpSmokeTest
    {
        // Test URLs — none of these require JS challenge solving (Phase 1 scope).
        private const string VimeoUrl     = "https://vimeo.com/76979871";
        private const string SoundCloudUrl = "https://soundcloud.com/forss/flickermood";

        static YtDlpSmokeTest()
        {
            try
            {
                var version = YtDlpApi.Version();
                Debug.Log($"[YtDlp] native library loaded — version: {version}");
            }
            catch (Exception e)
            {
                Debug.LogError($"[YtDlp] failed to load native library: {e.Message}");
            }
        }

        // ── Menu items ────────────────────────────────────────────────────────

        [MenuItem("Tools/YtDlp/1 – Init only")]
        public static void RunInitOnly()
        {
            try
            {
                YtDlpApi.EnsureInit();
                Debug.Log($"[YtDlp] EnsureInit OK — version: {YtDlpApi.Version()}");
            }
            catch (Exception e)
            {
                Debug.LogError($"[YtDlp] Init failed: {e}");
            }
        }

        [MenuItem("Tools/YtDlp/2 – Extract Vimeo (Phase 1)")]
        public static void RunVimeoExtract() => RunExtract(VimeoUrl);

        [MenuItem("Tools/YtDlp/3 – Extract SoundCloud (Phase 1)")]
        public static void RunSoundCloudExtract() => RunExtract(SoundCloudUrl);

        // ── Helpers ───────────────────────────────────────────────────────────

        private static async void RunExtract(string url)
        {
            Debug.Log($"[YtDlp] extracting: {url}");
            try
            {
                // EnsureInit is idempotent; calling it here guarantees Python is
                // ready even if the static constructor ran before the DLL was staged.
                YtDlpApi.EnsureInit();

                var info = await YtDlpApi.ExtractAsync(url);
                LogResult(url, info);
            }
            catch (Exception e)
            {
                Debug.LogError($"[YtDlp] extraction failed: {e}");
            }
        }

        private static void LogResult(string url, VideoInfo info)
        {
            var fmtCount = info.Formats?.Count ?? 0;
            string bestUrl = null;
            if (info.Formats != null && fmtCount > 0)
                bestUrl = info.Formats[fmtCount - 1].Url; // last = highest quality

            Debug.Log(
                $"[YtDlp] PASS\n" +
                $"  URL      : {url}\n" +
                $"  id       : {info.Id}\n" +
                $"  title    : {info.Title}\n" +
                $"  duration : {info.Duration}s\n" +
                $"  formats  : {fmtCount}\n" +
                $"  best url : {bestUrl ?? info.DirectUrl ?? "(none)"}");

            if (fmtCount == 0 && string.IsNullOrEmpty(info.DirectUrl))
                Debug.LogWarning("[YtDlp] WARNING: no playable URLs found in result");
        }
    }
}
#endif
