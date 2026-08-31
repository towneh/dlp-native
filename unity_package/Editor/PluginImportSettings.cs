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

        // All three desktop binaries live in one directory, so each has to name the
        // targets it is *not* for as well. Clearing Any Platform leaves the individual
        // standalone flags alone, and their default is enabled, so a Windows DLL stays
        // marked for the Linux and macOS players unless each is disabled by hand.
        private static readonly BuildTarget[] DesktopTargets =
        {
            BuildTarget.StandaloneWindows64,
            BuildTarget.StandaloneOSX,
            BuildTarget.StandaloneLinux64,
        };

        private static void ConfigureDesktop(
            PluginImporter imp, BuildTarget target, string cpu, bool editor)
        {
            imp.SetCompatibleWithAnyPlatform(false);
            imp.SetCompatibleWithEditor(editor);
            foreach (var other in DesktopTargets)
                imp.SetCompatibleWithPlatform(other, other == target);
            imp.SetPlatformData(target, "CPU", cpu);
            imp.SaveAndReimport();
        }

        // Every .dll in the directory, because the plugin is not the only one that has
        // to reach the player: unity_dlp.dll imports python3.dll, which forwards to the
        // full runtime staged beside it. An unconfigured plugin defaults to Any Platform,
        // which offers a Windows DLL to every other target as well.
        private static void ConfigureWindowsX64()
        {
            const string dir = "Packages/town.mr.ytdlp/Plugins/x86_64";
            var fullDir = Path.GetFullPath(dir);
            if (!Directory.Exists(fullDir)) return;

            foreach (var file in Directory.GetFiles(fullDir, "*.dll"))
            {
                var imp = AssetImporter.GetAtPath($"{dir}/{Path.GetFileName(file)}") as PluginImporter;
                if (imp == null) continue;
                ConfigureDesktop(imp, BuildTarget.StandaloneWindows64, "x86_64", editor: true);
            }
        }

        private static void ConfigureMacOsUniversal()
        {
            var path = "Packages/town.mr.ytdlp/Plugins/x86_64/unity_dlp.dylib";
            if (!File.Exists(Path.GetFullPath(path))) return;
            var imp = AssetImporter.GetAtPath(path) as PluginImporter;
            if (imp == null) return;
            ConfigureDesktop(imp, BuildTarget.StandaloneOSX, "AnyCPU", editor: true);
        }

        private static void ConfigureLinuxX64()
        {
            var path = "Packages/town.mr.ytdlp/Plugins/x86_64/libunity_dlp.so";
            if (!File.Exists(Path.GetFullPath(path))) return;
            var imp = AssetImporter.GetAtPath(path) as PluginImporter;
            if (imp == null) return;
            ConfigureDesktop(imp, BuildTarget.StandaloneLinux64, "x86_64", editor: false);
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
