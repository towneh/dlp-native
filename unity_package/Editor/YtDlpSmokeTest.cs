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
        private const string VimeoUrl      = "https://vimeo.com/76979871";
        private const string SoundCloudUrl = "https://soundcloud.com/forss/flickermood";
        private const string YouTubeUrl    = "https://www.youtube.com/watch?v=2n_Ae9DGC0U";
        private const string YouTubePlaylistUrl = "https://www.youtube.com/watch?v=wJUXLqNHCaI&list=PLFs4vir_WsTySi9F8v5pvCi6zQj7Cwneu";

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

        [MenuItem("Tools/YtDlp/4 – Extract YouTube (Phase 2)")]
        public static void RunYouTubeExtract() => RunExtract(YouTubeUrl);

        [MenuItem("Tools/YtDlp/5 – Extract YouTube URL with playlist parameter")]
        public static void RunYouTubePlaylistExtract() => RunExtract(YouTubePlaylistUrl);

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
