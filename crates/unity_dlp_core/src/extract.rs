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
const EXTRACT_PY: &str = r#"
import yt_dlp as _ydl_mod
import json as _json
import threading as _threading
import ctypes as _ctypes

_opts = {
    'quiet': True,
    'no_warnings': True,
    'extract_flat': False,
}

if _opts_json is not None:
    _opts.update(_json.loads(_opts_json))

_result_object = {
    'result': ""
}

def _extract(_url, _opts, _result_object):
    global _ydl_mod, _json
    with _ydl_mod.YoutubeDL(_opts) as _ydl:
        _info = _ydl.extract_info(_url, download=False)
        _info = _ydl_mod.YoutubeDL.sanitize_info(_info)
        _result_object['result'] = _json.dumps(_info)

extract_thread = _threading.Thread(target=_extract, args=(_url, _opts, _result_object))
extract_thread.start()
extract_thread.join(timeout=15)

if not extract_thread.is_alive():
    # The extract thread successfully finished or raised exception
    _result = _result_object['result']
else:
    # The extract thread didn't finish in time

    # Get a thread id for the extract thread
    if hasattr(extract_thread, '_thread_id'):
        _thread_id = extract_thread._thread_id
    else:
        for id, thread in _threading._active.items():
            if thread is extract_thread:
                _thread_id = id

    # Raise exception in the extract thread forcibly
    _exception_result = _ctypes.pythonapi.PyThreadState_SetAsyncExc(_ctypes.c_long(_thread_id), _ctypes.py_object(SystemExit))

    if _exception_result != 1:
        # Couldn't find the thread
        raise Exception("Timeout", "Timeout occured, but failed to destroy thread id " + str(_thread_id));

    # In case of the exeception was catched
    _exception_result = _ctypes.pythonapi.PyThreadState_SetAsyncExc(_ctypes.c_long(_thread_id), _ctypes.py_object(SystemExit))

    if _exception_result != 1:
        raise Exception("Timeout", "Timeout occured, but failed to destroy thread id " + str(_thread_id));

    raise Exception("Timeout", "Forcibly raised exception in thread id " + str(_thread_id))
"#;
