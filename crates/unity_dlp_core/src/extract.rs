use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::python_host;

/// How long the caller waits for a result before abandoning the worker.
///
/// Deliberately longer than the in-snippet Python timeout so that, in the
/// ordinary case, the snippet reports its own richer error first. This deadline
/// is the backstop for the case the snippet cannot handle: work that never
/// returns to Python bytecode, where the cancellation it attempts can never be
/// delivered.
const EXTRACT_DEADLINE: Duration = Duration::from_secs(20);

/// Deadline the Python snippet applies to its own worker, in seconds.
///
/// Declared here rather than inside the snippet so all three budgets have one
/// home and their ordering can be checked. It is injected as a local, the same
/// way the result ceiling is.
const PY_EXTRACT_TIMEOUT_SECS: u64 = 15;

/// Per-socket timeout applied to yt-dlp's network reads, in seconds.
///
/// Reapplied after caller options are merged rather than left as a default a
/// caller could raise, because it is the main thing bounding how long an
/// abandoned worker keeps working.
///
/// It is not an end-to-end deadline. It covers socket operations only: Python
/// resolves names through `getaddrinfo`, which takes no timeout argument and
/// ignores the default socket timeout, so a stalled resolver can outlive it.
/// Such a worker is a daemon thread and cannot block process exit, but it is
/// not counted by `IN_FLIGHT` either, so repeated timeouts against a
/// non-resolving host can accumulate Python threads. Bounding that properly
/// needs a killable boundary this library deliberately does not have — running
/// in-process, with no subprocess, is the point of the plugin.
const PY_SOCKET_TIMEOUT_SECS: u64 = 10;

/// The budgets must fire innermost-first, or an outer one masks the better
/// error from an inner one: a runaway script should surface as a JS failure and
/// a stuck extraction as the snippet's own timeout, rather than both arriving as
/// the Rust backstop. Nothing enforced that while the values lived in three
/// places, so it is now checked at compile time.
const _: () = assert!(
    crate::jsc_provider::JS_TIMEOUT.as_secs() < PY_EXTRACT_TIMEOUT_SECS,
    "the JS budget must expire before the Python extraction timeout"
);
const _: () = assert!(
    PY_EXTRACT_TIMEOUT_SECS < EXTRACT_DEADLINE.as_secs(),
    "the Python extraction timeout must expire before the Rust deadline"
);

/// Ceiling on extractions running at once. Each one owns an OS thread and a
/// Python thread state, and a timed-out extraction is abandoned rather than
/// killed, so without a cap a run of hostile URLs would accumulate threads for
/// the lifetime of the process.
///
/// The shipped Unity wrapper cannot reach this cap: it funnels every native
/// call through a single worker thread, because CPython pins its interpreter,
/// GIL and thread state to whichever thread called `Py_Initialize`. That is
/// deliberate and must stay. The cap exists for every other consumer of this C
/// ABI — the CLI, and anything else that calls `unity_dlp_extract` concurrently
/// — which the library cannot assume serialise their calls.
const MAX_IN_FLIGHT: usize = 4;

/// Ceiling on the serialised result, in bytes.
///
/// `out_cap` bounds the copy into the caller's buffer but never the allocation
/// behind it: by the time it is consulted, the whole serialised `info_dict`
/// exists as a Python string and again as a Rust one. A remote side that
/// inflates the format list therefore drives memory here regardless of what the
/// caller asked for, and the buffer-too-small retry re-runs the entire
/// extraction to rebuild the same oversized result.
///
/// Lower on mobile, where there is far less headroom and an OOM kill is least
/// survivable.
#[cfg(any(target_os = "android", target_os = "ios"))]
const MAX_RESULT_BYTES: usize = 16 * 1024 * 1024;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const MAX_RESULT_BYTES: usize = 64 * 1024 * 1024;

static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// Why an extraction did not produce a result.
///
/// The caller maps these onto C ABI result codes. They are distinguished here,
/// where the failure is still structured, rather than by inspecting rendered
/// text further up.
pub enum ExtractError {
    /// yt-dlp or the interpreter raised something not covered below.
    Python(String),
    /// The failure was an `OSError` — `URLError`, `ConnectionError`,
    /// `HTTPError` and `socket` failures all derive from it.
    Network(String),
    /// The embedded JS engine failed.
    Js(String),
    /// The deadline expired, or Python raised `TimeoutError`.
    Timeout(String),
    /// Too many extractions already in flight.
    Busy(String),
    /// The serialised result exceeded `MAX_RESULT_BYTES`.
    TooLarge(String),
}

impl ExtractError {
    pub fn message(&self) -> &str {
        match self {
            Self::Python(m)
            | Self::Network(m)
            | Self::Js(m)
            | Self::Timeout(m)
            | Self::Busy(m)
            | Self::TooLarge(m) => m,
        }
    }
}

/// Classify a raised exception by type.
///
/// The rendered text of an exception embeds the caller's URL and remote
/// response bodies, so it is not something to branch on — a URL containing the
/// word "Network" would otherwise select the result code. The exception class
/// is not attacker-controlled in the same way.
fn classify(py: Python<'_>, err: &PyErr) -> fn(String) -> ExtractError {
    use pyo3::exceptions::{PyOSError, PyTimeoutError};

    // TimeoutError derives from OSError, so it has to be tested first or every
    // timeout would be reported as a network failure.
    if err.is_instance_of::<crate::jsc_provider::JsError>(py) {
        ExtractError::Js
    } else if err.is_instance_of::<PyTimeoutError>(py) {
        ExtractError::Timeout
    } else if err.is_instance_of::<PyOSError>(py) {
        ExtractError::Network
    } else {
        ExtractError::Python
    }
}

/// Decrements the in-flight count when the Rust worker thread ends, not when
/// the caller stops waiting.
///
/// This bounds Rust worker threads, not the Python threads beneath them. When
/// the in-snippet timeout fires, the snippet raises, `run_extract` returns and
/// the permit is released while that extraction's Python thread may still be
/// running — the snippet logs exactly that case. Those linger until their
/// socket timeout expires and are not counted here — and a worker stalled in
/// name resolution is not bounded even by that; see PY_SOCKET_TIMEOUT_SECS.
struct InFlightPermit;

impl InFlightPermit {
    fn acquire() -> Result<Self, ExtractError> {
        let mut current = IN_FLIGHT.load(Ordering::Acquire);
        loop {
            if current >= MAX_IN_FLIGHT {
                return Err(ExtractError::Busy(format!(
                    "{MAX_IN_FLIGHT} extractions already in flight; try again later"
                )));
            }
            match IN_FLIGHT.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(Self),
                Err(observed) => current = observed,
            }
        }
    }
}

impl Drop for InFlightPermit {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Call yt-dlp to extract metadata for `url` and return the result as a JSON string.
///
/// `opts_json` — optional JSON object forwarded to `YoutubeDL` options dict.
///
/// Runs on a dedicated OS thread and waits on a channel, so the deadline is
/// enforced outside the interpreter. Waiting on the GIL instead would make the
/// timeout unenforceable: the snippet's cancellation is delivered at Python
/// bytecode boundaries, which CPU-bound C code inside yt-dlp — regex
/// backtracking, decompression, JSON parsing of a large body — never reaches.
pub fn extract(url: &str, opts_json: Option<&str>) -> Result<String, ExtractError> {
    let permit = InFlightPermit::acquire()?;

    let (tx, rx) = mpsc::channel();
    let url = url.to_string();
    let opts_json = opts_json.map(str::to_string);

    std::thread::Builder::new()
        .name("unity_dlp_extract".to_string())
        .spawn(move || {
            // Held by the worker, so the slot is released when this thread
            // finishes rather than when the caller gives up waiting. Note this
            // tracks the Rust thread only; see InFlightPermit.
            let _permit = permit;
            let result = python_host::with_python(|py| run_extract(py, &url, opts_json.as_deref()));
            // The receiver is gone if the caller already timed out; the result
            // is simply dropped in that case.
            let _ = tx.send(result);
        })
        .map_err(|e| ExtractError::Python(format!("could not start extraction thread: {e}")))?;

    match rx.recv_timeout(EXTRACT_DEADLINE) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(ExtractError::Timeout(format!(
            "extraction exceeded {}s and was abandoned",
            EXTRACT_DEADLINE.as_secs()
        ))),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(ExtractError::Python(
            "extraction thread ended without producing a result".to_string(),
        )),
    }
}

fn run_extract(py: Python<'_>, url: &str, opts_json: Option<&str>) -> Result<String, ExtractError> {
    let locals = PyDict::new_bound(py);
    locals
        .set_item("_url", url)
        .map_err(|e| ExtractError::Python(format!("set _url: {e}")))?;
    locals
        .set_item("_opts_json", opts_json)
        .map_err(|e| ExtractError::Python(format!("set _opts_json: {e}")))?;
    locals
        .set_item("_max_result_bytes", MAX_RESULT_BYTES)
        .map_err(|e| ExtractError::Python(format!("set _max_result_bytes: {e}")))?;
    locals
        .set_item("_extract_timeout_seconds", PY_EXTRACT_TIMEOUT_SECS)
        .map_err(|e| ExtractError::Python(format!("set _extract_timeout_seconds: {e}")))?;
    locals
        .set_item("_socket_timeout_seconds", PY_SOCKET_TIMEOUT_SECS)
        .map_err(|e| ExtractError::Python(format!("set _socket_timeout_seconds: {e}")))?;

    py.run_bound(EXTRACT_PY, None, Some(&locals)).map_err(|e| {
        let variant = classify(py, &e);
        variant(format!("yt-dlp extraction failed: {e}"))
    })?;

    if let Ok(Some(size)) = locals.get_item("_too_large") {
        let size = size.extract::<usize>().unwrap_or(0);
        return Err(ExtractError::TooLarge(format!(
            "extraction result is {size} bytes, over the {MAX_RESULT_BYTES}-byte limit"
        )));
    }

    locals
        .get_item("_result")
        .map_err(|e| ExtractError::Python(format!("read _result: {e}")))?
        .ok_or_else(|| ExtractError::Python("extraction produced no result".to_string()))?
        .extract::<String>()
        .map_err(|e| ExtractError::Python(format!("_result is not a string: {e}")))
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
// The extraction timeout and result ceiling are deliberately not part of the
// public ABI. They are DoS controls, so a caller that could widen them could
// also disable them, and their ordering against the JS budget is an invariant
// this module owns. Platform differences are handled by the cfg above.
const EXTRACT_PY: &str = r#"
import threading as _threading
import ctypes as _ctypes
import logging as _logging

_opts = {
    'quiet': True,
    'no_warnings': True,
    'extract_flat': False,
    'noplaylist': True,
}

if _opts_json is not None:
    import json as _json
    _opts.update(_json.loads(_opts_json))

# Applied after the merge, not before: this bounds blocking reads so an injected
# cancellation can reach Python bytecode, and it is what limits how long an
# abandoned worker keeps running. A caller must not be able to raise or remove
# it, so it is reasserted here rather than set as a default above.
_opts['socket_timeout'] = _socket_timeout_seconds

# _extract_timeout_seconds and _max_result_bytes are injected by the caller so
# the budgets have a single home in Rust; see PY_EXTRACT_TIMEOUT_SECS.
_post_cancel_join_seconds = 1
_result_object = {}

def _extract(_url, _opts, _result_object, _max_result_bytes):
    # Keep these imports local: pyo3 executes this snippet with distinct globals
    # and locals dictionaries, while Python caches imports process-wide.
    import json
    import yt_dlp

    try:
        with yt_dlp.YoutubeDL(_opts) as _ydl:
            _info = _ydl.extract_info(_url, download=False)
            _info = yt_dlp.YoutubeDL.sanitize_info(_info)
            _serialised = json.dumps(_info)
            # Report an oversized result rather than handing it back: the caller
            # would otherwise copy it again and, on a buffer miss, re-run the
            # whole extraction to produce the same oversized result.
            if len(_serialised) > _max_result_bytes:
                _result_object['too_large'] = len(_serialised)
            else:
                _result_object['result'] = _serialised
    except Exception as _error:
        # Thread exceptions are otherwise only printed to stderr. Carry normal
        # extraction failures back to the calling thread.
        _result_object['error'] = _error

# daemon=True: a timed-out extraction is deliberately abandoned rather than
# killed, and a non-daemon thread is joined during interpreter shutdown, which
# would let one hostile URL stall host teardown.
extract_thread = _threading.Thread(
    target=_extract,
    args=(_url, _opts, _result_object, _max_result_bytes),
    daemon=True,
)
extract_thread.start()
extract_thread.join(timeout=_extract_timeout_seconds)

if not extract_thread.is_alive():
    if 'error' in _result_object:
        raise _result_object['error']
    if 'too_large' in _result_object:
        _too_large = _result_object['too_large']
    else:
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
    # socket_timeout bounds how long it can remain in the background, except
    # while resolving a name, which that timeout does not cover.
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
