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

static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// Why an extraction did not produce a result.
///
/// The caller maps these onto C ABI result codes. They are distinguished here,
/// where the failure is still structured, rather than by inspecting rendered
/// text further up.
pub enum ExtractError {
    /// yt-dlp or the interpreter raised.
    Python(String),
    /// The deadline expired and the worker was abandoned.
    Timeout(String),
    /// Too many extractions already in flight.
    Busy(String),
}

impl ExtractError {
    pub fn message(&self) -> &str {
        match self {
            Self::Python(m) | Self::Timeout(m) | Self::Busy(m) => m,
        }
    }
}

/// Decrements the in-flight count when the Rust worker thread ends, not when
/// the caller stops waiting.
///
/// This bounds Rust worker threads, not the Python threads beneath them. When
/// the in-snippet timeout fires, the snippet raises, `run_extract` returns and
/// the permit is released while that extraction's Python thread may still be
/// running — the snippet logs exactly that case. Those linger until their
/// socket timeout expires and are not counted here.
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
        Ok(result) => result.map_err(ExtractError::Python),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(ExtractError::Timeout(format!(
            "extraction exceeded {}s and was abandoned",
            EXTRACT_DEADLINE.as_secs()
        ))),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(ExtractError::Python(
            "extraction thread ended without producing a result".to_string(),
        )),
    }
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

# daemon=True: a timed-out extraction is deliberately abandoned rather than
# killed, and a non-daemon thread is joined during interpreter shutdown, which
# would let one hostile URL stall host teardown.
extract_thread = _threading.Thread(
    target=_extract,
    args=(_url, _opts, _result_object),
    daemon=True,
)
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
