#if UNITY_EDITOR
using System.IO;
using UnityEditor;
using UnityEngine;

namespace YtDlp.Editor
{
    /// <summary>
    /// Configures per-platform plugin import settings so Unity routes each
    /// native library binary to the correct target at build time.
    ///
    /// Run via the menu: Tools → YtDlp → Configure Plugin Import Settings.
    /// Also runs automatically via <see cref="InitializeOnLoadMethod"/>.
    /// </summary>
    [InitializeOnLoad]
    internal static class PluginImportSettings
    {
        static PluginImportSettings() => Configure();

        [MenuItem("Tools/YtDlp/Configure Plugin Import Settings")]
        public static void Configure()
        {
            ConfigureWindowsX64();
            ConfigureMacOsUniversal();
            ConfigureLinuxX64();
            ConfigureIos();
            ConfigureAndroidAbi("arm64-v8a", "ARM64");
            ConfigureAndroidAbi("armeabi-v7a", "ARMv7");
            AssetDatabase.Refresh();
        }

        private static void ConfigureWindowsX64()
        {
            var path = "Packages/town.mr.ytdlp/Plugins/x86_64/unity_dlp.dll";
            if (!File.Exists(Path.GetFullPath(path))) return;
            var imp = AssetImporter.GetAtPath(path) as PluginImporter;
            if (imp == null) return;
            imp.SetCompatibleWithAnyPlatform(false);
            imp.SetCompatibleWithEditor(true);
            imp.SetCompatibleWithPlatform(BuildTarget.StandaloneWindows64, true);
            imp.SetPlatformData(BuildTarget.StandaloneWindows64, "CPU", "x86_64");
            imp.SaveAndReimport();
        }

        private static void ConfigureMacOsUniversal()
        {
            var path = "Packages/town.mr.ytdlp/Plugins/x86_64/unity_dlp.dylib";
            if (!File.Exists(Path.GetFullPath(path))) return;
            var imp = AssetImporter.GetAtPath(path) as PluginImporter;
            if (imp == null) return;
            imp.SetCompatibleWithAnyPlatform(false);
            imp.SetCompatibleWithEditor(true);
            imp.SetCompatibleWithPlatform(BuildTarget.StandaloneOSX, true);
            imp.SetPlatformData(BuildTarget.StandaloneOSX, "CPU", "AnyCPU");
            imp.SaveAndReimport();
        }

        private static void ConfigureLinuxX64()
        {
            var path = "Packages/town.mr.ytdlp/Plugins/x86_64/unity_dlp.so";
            if (!File.Exists(Path.GetFullPath(path))) return;
            var imp = AssetImporter.GetAtPath(path) as PluginImporter;
            if (imp == null) return;
            imp.SetCompatibleWithAnyPlatform(false);
            imp.SetCompatibleWithEditor(false);
            imp.SetCompatibleWithPlatform(BuildTarget.StandaloneLinux64, true);
            imp.SetPlatformData(BuildTarget.StandaloneLinux64, "CPU", "x86_64");
            imp.SaveAndReimport();
        }

        private static void ConfigureIos()
        {
            var path = "Packages/town.mr.ytdlp/Plugins/iOS/libunity_dlp.a";
            if (!File.Exists(Path.GetFullPath(path))) return;
            var imp = AssetImporter.GetAtPath(path) as PluginImporter;
            if (imp == null) return;
            imp.SetCompatibleWithAnyPlatform(false);
            imp.SetCompatibleWithEditor(false);
            imp.SetCompatibleWithPlatform(BuildTarget.iOS, true);
            imp.SetPlatformData(BuildTarget.iOS, "CPU", "ARM64");
            imp.SaveAndReimport();
        }

        // Every .so in the ABI directory, because the plugin is not the only one that
        // has to reach the device: it links against the Termux libpython, which in turn
        // needs libandroid-support. An unconfigured .so is left out of the APK silently,
        // and the loader then reports the plugin as missing rather than the library that
        // actually is.
        private static void ConfigureAndroidAbi(string abi, string cpu)
        {
            var dir = $"Packages/town.mr.ytdlp/Plugins/Android/libs/{abi}";
            var fullDir = Path.GetFullPath(dir);
            if (!Directory.Exists(fullDir)) return;

            foreach (var file in Directory.GetFiles(fullDir, "*.so"))
            {
                var imp = AssetImporter.GetAtPath($"{dir}/{Path.GetFileName(file)}") as PluginImporter;
                if (imp == null) continue;
                imp.SetCompatibleWithAnyPlatform(false);
                imp.SetCompatibleWithEditor(false);
                imp.SetCompatibleWithPlatform(BuildTarget.Android, true);
                imp.SetPlatformData(BuildTarget.Android, "CPU", cpu);
                imp.SaveAndReimport();
            }
        }
    }
}
#endif
