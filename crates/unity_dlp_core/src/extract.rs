use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::python_host;

/// Call yt-dlp to extract metadata for `url` and return the result as a JSON string.
///
/// `opts_json` — optional JSON object forwarded to `YoutubeDL` options dict.
///
/// Errors are returned as descriptive strings; the caller maps them to C ABI
/// error codes and stores them in `unity_dlp_last_error`.
pub fn extract(url: &str, opts_json: Option<&str>) -> Result<String, String> {
    python_host::with_python(|py| run_extract(py, url, opts_json))
}

fn run_extract(py: Python<'_>, url: &str, opts_json: Option<&str>) -> Result<String, String> {
    let locals = PyDict::new_bound(py);
    locals
        .set_item("_url", url)
        .map_err(|e| format!("set _url: {e}"))?;
    locals
        .set_item("_opts_json", opts_json)
        .map_err(|e| format!("set _opts_json: {e}"))?;

    py.run_bound(EXTRACT_PY, None, Some(&locals))
        .map_err(|e| format!("yt-dlp extraction failed: {e}"))?;

    locals
        .get_item("_result")
        .map_err(|e| format!("read _result: {e}"))?
        .ok_or_else(|| "extraction produced no result".to_string())?
        .extract::<String>()
        .map_err(|e| format!("_result is not a string: {e}"))
}

// The Python snippet run for each extraction. Uses `exec` semantics (no return value);
// the result is communicated back via the `_result` local.
//
// Notes:
//  - `quiet` / `no_warnings` suppress yt-dlp's console output; log routing to
//    Unity Debug.Log is Phase-6 work.
//  - `sanitize_info` removes non-JSON-serialisable objects (e.g. datetime) that
//    appear in some extractors' info_dict.
//  - `extract_flat=False` ensures full format list resolution.
// TODO: Pass the extraction timeout through the public ABI and add a dedicated
// timeout result code instead of keeping it as an implementation detail here.
const EXTRACT_PY: &str = r#"
import threading as _threading
import ctypes as _ctypes
import logging as _logging

_opts = {
    'quiet': True,
    'no_warnings': True,
    'extract_flat': False,
    'noplaylist': True,
    # Bound blocking reads so an injected timeout can reach Python bytecode.
    'socket_timeout': 10,
}

if _opts_json is not None:
    import json as _json
    _opts.update(_json.loads(_opts_json))

_extract_timeout_seconds = 15
_post_cancel_join_seconds = 1
_result_object = {}

def _extract(_url, _opts, _result_object):
    # Keep these imports local: pyo3 executes this snippet with distinct globals
    # and locals dictionaries, while Python caches imports process-wide.
    import json
    import yt_dlp

    try:
        with yt_dlp.YoutubeDL(_opts) as _ydl:
            _info = _ydl.extract_info(_url, download=False)
            _info = yt_dlp.YoutubeDL.sanitize_info(_info)
            _result_object['result'] = json.dumps(_info)
    except Exception as _error:
        # Thread exceptions are otherwise only printed to stderr. Carry normal
        # extraction failures back to the calling thread.
        _result_object['error'] = _error

extract_thread = _threading.Thread(target=_extract, args=(_url, _opts, _result_object))
extract_thread.start()
extract_thread.join(timeout=_extract_timeout_seconds)

if not extract_thread.is_alive():
    if 'error' in _result_object:
        raise _result_object['error']
    _result = _result_object['result']
else:
    _thread_id = extract_thread.ident
    if _thread_id is None:
        raise TimeoutError(
            f"yt-dlp extraction exceeded {_extract_timeout_seconds}s; "
            "worker thread has no id"
        )

    # Ask the worker to stop when it next executes Python bytecode.
    _exception_result = _ctypes.pythonapi.PyThreadState_SetAsyncExc(_ctypes.c_ulong(_thread_id), _ctypes.py_object(SystemExit))

    if _exception_result > 1:
        # Undo the injection if CPython reports that more than one thread state
        # was modified; this should never happen for a valid thread id.
        _ctypes.pythonapi.PyThreadState_SetAsyncExc(_ctypes.c_ulong(_thread_id), None)

    # join() releases the GIL, giving a worker that is back in Python a chance
    # to receive SystemExit. A blocking read may survive briefly, but
    # socket_timeout bounds how long it can remain in the background.
    extract_thread.join(timeout=_post_cancel_join_seconds)
    if extract_thread.is_alive():
        _logging.getLogger("unity_dlp").warning(
            "Timed-out extraction thread %s is still alive after cancellation; "
            "waiting for its socket timeout",
            _thread_id,
        )

    _cancel_detail = (
        "cancellation requested"
        if _exception_result == 1
        else f"cancellation could not find exactly one thread (result={_exception_result})"
    )
    raise TimeoutError(
        f"yt-dlp extraction exceeded {_extract_timeout_seconds}s; {_cancel_detail}"
    )
"#;
