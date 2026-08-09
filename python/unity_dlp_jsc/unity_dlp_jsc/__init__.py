"""
unity_dlp_jsc — JsChallengeProvider shim that routes yt-dlp YouTube JS
challenges into the in-process V8 runtime (rustyscript) registered by Rust
during unity_dlp_init().

The `unity_dlp_js` module is a PyO3-backed module registered into sys.modules
by jsc_provider::register_module() before this package is imported.
"""

from yt_dlp.extractor.youtube.jsc._builtin.ejs import EJSBaseJCP
from yt_dlp.extractor.youtube.jsc.provider import (
    JsChallengeProvider,
    JsChallengeProviderError,
    register_preference,
    register_provider,
)

import unity_dlp_js


@register_provider
class UnityDlpJCP(EJSBaseJCP):
    """Routes EJS JS-challenge solving through the embedded V8 (rustyscript)."""

    PROVIDER_NAME = "unity-dlp"
    JS_RUNTIME_NAME = "unity-dlp"

    def is_available(self) -> bool:
        # _available is True unless EJSBaseJCP failed to load a solver script.
        # Skip the parent's binary-detection check — we have no external binary.
        return self._available

    def _run_js_runtime(self, stdin: str, /) -> str:
        try:
            return unity_dlp_js.run_js(stdin)
        except Exception as exc:
            raise JsChallengeProviderError(f"unity-dlp V8 runtime error: {exc}") from exc


@register_preference(UnityDlpJCP)
def _preference(provider: JsChallengeProvider, requests) -> int:
    # 900 > built-in quickjs subprocess (850) so we are preferred on every platform.
    return 900
