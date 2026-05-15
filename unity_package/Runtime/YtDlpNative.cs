using System;
using System.Runtime.InteropServices;

namespace YtDlp.Native
{
    /// <summary>
    /// Raw P/Invoke bindings for the unity_dlp native library.
    /// All methods on this class are thread-safe with respect to the native
    /// library's own synchronization. Do not call Extract from the Unity main
    /// thread — it may block on network I/O for tens of seconds.
    /// </summary>
    internal static class NativeLib
    {
        public const int OK       =  0;
        public const int ERR_INIT = -1;
        public const int ERR_PY   = -2;
        public const int ERR_JS   = -3;
        public const int ERR_NET  = -4;
        public const int ERR_BUF  = -5;

#if UNITY_IOS && !UNITY_EDITOR
        private const string LibName = "__Internal";
#else
        private const string LibName = "unity_dlp";
#endif

        [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
        public static extern int unity_dlp_init(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string pythonHomeUtf8,
            [MarshalAs(UnmanagedType.LPUTF8Str)] string packagesPathUtf8);

        [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
        public static extern int unity_dlp_shutdown();

        [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
        public static extern IntPtr unity_dlp_version();

        [DllImport(LibName, CallingConvention = CallingConvention.Cdecl,
            CharSet = CharSet.Ansi, BestFitMapping = false, ThrowOnUnmappableChar = true)]
        public static extern int unity_dlp_extract(
            [MarshalAs(UnmanagedType.LPUTF8Str)] string urlUtf8,
            [MarshalAs(UnmanagedType.LPUTF8Str)] string optsJsonUtf8,
            [Out] byte[] outBuf,
            int outCap,
            out int outLen);

        [DllImport(LibName, CallingConvention = CallingConvention.Cdecl)]
        public static extern int unity_dlp_last_error(
            [Out] byte[] outBuf,
            int outCap,
            out int outLen);
    }
}
