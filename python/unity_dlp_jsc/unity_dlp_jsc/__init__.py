"""
unity_dlp_jsc — JSCProvider shim for Phase 2.

In Phase 2 this module will subclass yt-dlp's JavaScriptInterpreter and
route JS execution back into the in-process Rust/V8 runtime registered
during unity_dlp_init().

Phase 0/1: empty placeholder so the package is importable.
"""
