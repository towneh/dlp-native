use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicI32, Ordering};

use once_cell::sync::OnceCell;

use crate::{extract, logging, python_host};

// ── Error storage ─────────────────────────────────────────────────────────────

static LAST_ERROR: OnceCell<std::sync::Mutex<String>> = OnceCell::new();

fn last_error_mutex() -> &'static std::sync::Mutex<String> {
    LAST_ERROR.get_or_init(|| std::sync::Mutex::new(String::new()))
}

/// Upper bound on the stored error string, in bytes. The text is built from
/// remote-derived exception messages, so it is capped here rather than left to
/// grow unbounded in a process-global slot. Matches the buffer the C# wrapper
/// rents, so a well-behaved reader never has to retry.
const MAX_LAST_ERROR_BYTES: usize = 4096;
const LAST_ERROR_TRUNCATED_SUFFIX: &str = "... [truncated]";

fn clamp_error_message(mut msg: String) -> String {
    if msg.len() > MAX_LAST_ERROR_BYTES {
        let keep = MAX_LAST_ERROR_BYTES - LAST_ERROR_TRUNCATED_SUFFIX.len();
        // floor_char_boundary is unstable, so walk back to one by hand.
        let mut cut = keep;
        while cut > 0 && !msg.is_char_boundary(cut) {
            cut -= 1;
        }
        msg.truncate(cut);
        msg.push_str(LAST_ERROR_TRUNCATED_SUFFIX);
    }
    msg
}

fn set_last_error(msg: impl Into<String>) {
    let msg = clamp_error_message(msg.into());
    if let Ok(mut guard) = last_error_mutex().lock() {
        *guard = msg;
    }
}

// ── Result type ───────────────────────────────────────────────────────────────

pub type UnityDlpResult = i32;

pub const UNITY_DLP_OK: UnityDlpResult = 0;
/// Init succeeded but the yt-dlp JCP shim (`unity_dlp_jsc`) did not register.
/// The interpreter is up and extraction works, but the YouTube JS-challenge path
/// is unavailable. `unity_dlp_last_error` holds the import error.
pub const UNITY_DLP_OK_DEGRADED: UnityDlpResult = 1;
pub const UNITY_DLP_ERR_INIT: UnityDlpResult = -1;
pub const UNITY_DLP_ERR_PYTHON: UnityDlpResult = -2;
pub const UNITY_DLP_ERR_JS: UnityDlpResult = -3;
pub const UNITY_DLP_ERR_NET: UnityDlpResult = -4;
/// out_buf too small; out_len holds the required byte count.
pub const UNITY_DLP_ERR_BUF: UnityDlpResult = -5;
/// The extraction deadline expired and the worker was abandoned. Distinct from
/// ERR_NET so a caller can tell "this URL is slow or hostile" from "the network
/// failed", and can decline to retry immediately.
pub const UNITY_DLP_ERR_TIMEOUT: UnityDlpResult = -6;
/// Too many extractions already in flight. The call did no work; retrying after
/// one of the in-flight extractions finishes is the expected response.
pub const UNITY_DLP_ERR_BUSY: UnityDlpResult = -7;
/// The result was larger than the library is willing to materialise. Distinct
/// from ERR_BUF because a bigger buffer will not help: retrying re-runs the whole
/// extraction and produces the same oversized result.
pub const UNITY_DLP_ERR_TOO_LARGE: UnityDlpResult = -8;

// ── Init / shutdown ───────────────────────────────────────────────────────────

// Readiness is tri-state rather than a bool: the interpreter is not usable
// during the window between claiming initialisation and Py_Initialize actually
// returning, and a caller entering that window would trip PyO3's
// uninitialised-interpreter assert, which aborts the host process.
const STATE_UNINIT: i32 = 0;
const STATE_INITIALISING: i32 = 1;
const STATE_READY: i32 = 2;
/// Interpreter start-up failed, permanently. `python_host::init` caches its
/// outcome in a `OnceCell`, so once it has returned `Err` every later call gets
/// that same clone back. Re-running initialisation could only fail identically,
/// so this state is terminal: it reports the original failure instead of
/// advertising a retry that cannot succeed.
const STATE_FAILED: i32 = 3;

static INIT_STATE: AtomicI32 = AtomicI32::new(STATE_UNINIT);
// The result code the first successful init returned (UNITY_DLP_OK or
// UNITY_DLP_OK_DEGRADED). Later no-op calls echo it so every caller sees the
// same verdict rather than an unconditional OK. Written before INIT_STATE is
// released to STATE_READY, so anyone who observes READY also observes this.
static INIT_CODE: AtomicI32 = AtomicI32::new(UNITY_DLP_OK);

/// The reason interpreter start-up failed, kept for the terminal state.
///
/// `LAST_ERROR` cannot be relied on to still hold it: any later call to
/// `unity_dlp_extract` overwrites it with "library not initialised", so by the
/// time a second `unity_dlp_init` arrives the actionable reason would be gone.
static INIT_FAILURE: OnceCell<String> = OnceCell::new();

/// True once `unity_dlp_init` has completed successfully.
fn is_ready() -> bool {
    INIT_STATE.load(Ordering::Acquire) == STATE_READY
}

/// Run an exported entry point with a panic barrier.
///
/// `extern "C"` functions abort the process if a panic escapes them, and this
/// library is loaded in-process by the Unity Editor and shipped clients, so an
/// abort takes the host down. Remote data flows through PyO3, V8 and QuickJS
/// below these entry points; a panic anywhere in that stack is turned into an
/// error code here instead.
fn ffi_guard<F>(on_panic: UnityDlpResult, f: F) -> UnityDlpResult
where
    F: FnOnce() -> UnityDlpResult,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(code) => code,
        Err(payload) => {
            let detail = panic_detail(payload.as_ref());
            log::error!("panic crossing the C ABI boundary: {detail}");
            set_last_error(format!("panic in native library: {detail}"));
            on_panic
        }
    }
}

fn panic_detail(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

/// Initialise the native library.
///
/// `python_home_utf8`   — NUL-terminated path to the unpacked Python prefix
///                        (sets PYTHONHOME). Nullable; null or empty skips it.
/// `packages_path_utf8` — NUL-terminated `\n`-delimited list of paths added to
///                        sys.path (each a .zip or a directory); earlier entries
///                        win resolution. Newlines separate because `;`/`:` are
///                        legal path characters. Nullable; null or empty skips it.
///
/// Must succeed before calling any other function. Returns UNITY_DLP_OK on a
/// clean init or UNITY_DLP_OK_DEGRADED when the interpreter came up but the JCP
/// shim did not register (see that constant). Safe to call from multiple threads
/// — only the first call runs initialisation; subsequent calls echo the first
/// call's result code.
///
/// # Safety
///
/// Each of `python_home_utf8` and `packages_path_utf8` must be either null or a
/// pointer to a NUL-terminated C string that stays valid and unmodified for the
/// duration of the call.
#[no_mangle]
pub unsafe extern "C" fn unity_dlp_init(
    python_home_utf8: *const c_char,
    packages_path_utf8: *const c_char,
) -> UnityDlpResult {
    // SAFETY: the pointer contract is this function's own, documented in its
    // # Safety section; it is forwarded unchanged to the inner call.
    ffi_guard(UNITY_DLP_ERR_INIT, || unsafe {
        unity_dlp_init_inner(python_home_utf8, packages_path_utf8)
    })
}

unsafe fn unity_dlp_init_inner(
    python_home_utf8: *const c_char,
    packages_path_utf8: *const c_char,
) -> UnityDlpResult {
    match INIT_STATE.compare_exchange(
        STATE_UNINIT,
        STATE_INITIALISING,
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => {}
        Err(STATE_INITIALISING) => {
            // Another thread is mid-initialisation. Reporting OK here would tell
            // the caller the interpreter is usable while Py_Initialize is still
            // running, so refuse instead.
            set_last_error("initialisation is already in progress on another thread");
            return UNITY_DLP_ERR_INIT;
        }
        Err(STATE_FAILED) => {
            // Terminal: python_host::init has cached its Err, so re-running it
            // could only fail identically. Restore the original reason, which a
            // later call may have overwritten in LAST_ERROR.
            let reason = INIT_FAILURE
                .get()
                .cloned()
                .unwrap_or_else(|| "interpreter initialisation failed".to_string());
            set_last_error(reason);
            return UNITY_DLP_ERR_INIT;
        }
        Err(_) => return INIT_CODE.load(Ordering::Acquire),
    }

    logging::init();

    let python_home = if python_home_utf8.is_null() {
        ""
    } else {
        // SAFETY: non-null (checked above); the caller guarantees a
        // NUL-terminated string valid for this call.
        match unsafe { CStr::from_ptr(python_home_utf8) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                set_last_error("python_home is not valid UTF-8");
                INIT_STATE.store(STATE_UNINIT, Ordering::Release);
                return UNITY_DLP_ERR_INIT;
            }
        }
    };

    let packages_path = if packages_path_utf8.is_null() {
        ""
    } else {
        // SAFETY: non-null (checked above); the caller guarantees a
        // NUL-terminated string valid for this call.
        match unsafe { CStr::from_ptr(packages_path_utf8) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                set_last_error("packages_path is not valid UTF-8");
                INIT_STATE.store(STATE_UNINIT, Ordering::Release);
                return UNITY_DLP_ERR_INIT;
            }
        }
    };

    match python_host::init(python_home, packages_path) {
        Ok(None) => {
            log::info!("unity_dlp_init: library initialised");
            INIT_CODE.store(UNITY_DLP_OK, Ordering::Relaxed);
            INIT_STATE.store(STATE_READY, Ordering::Release);
            UNITY_DLP_OK
        }
        Ok(Some(shim_error)) => {
            // Interpreter is up; only the JCP shim failed. Extraction works, so
            // stay initialised and surface the reason via last_error.
            log::error!("unity_dlp_init: JCP shim degraded: {shim_error}");
            set_last_error(shim_error);
            INIT_CODE.store(UNITY_DLP_OK_DEGRADED, Ordering::Relaxed);
            INIT_STATE.store(STATE_READY, Ordering::Release);
            UNITY_DLP_OK_DEGRADED
        }
        Err(e) => {
            log::error!("unity_dlp_init: Python init failed: {e}");
            let _ = INIT_FAILURE.set(e.clone());
            set_last_error(e);
            INIT_STATE.store(STATE_FAILED, Ordering::Release);
            UNITY_DLP_ERR_INIT
        }
    }
}

/// Shut down the native library and release resources.
///
/// After this call the library is uninitialised. Do not call other functions
/// until `unity_dlp_init` succeeds again.
#[no_mangle]
pub extern "C" fn unity_dlp_shutdown() -> UnityDlpResult {
    ffi_guard(UNITY_DLP_OK, || {
        if INIT_STATE
            .compare_exchange(
                STATE_READY,
                STATE_UNINIT,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return UNITY_DLP_OK;
        }

        log::info!("unity_dlp_shutdown: library shut down");
        UNITY_DLP_OK
    })
}

// ── Version ───────────────────────────────────────────────────────────────────

/// Return a static, NUL-terminated UTF-8 version string.
///
/// The pointer is valid for the lifetime of the process and must not be freed.
#[no_mangle]
pub extern "C" fn unity_dlp_version() -> *const c_char {
    c"unity_dlp/0.1.0 (phase-2)".as_ptr()
}

// ── Extract ───────────────────────────────────────────────────────────────────

/// Extract media metadata for the given URL.
///
/// `url_utf8`       — NUL-terminated URL (required).
/// `opts_json_utf8` — NUL-terminated JSON options object (nullable).
/// `out_buf`        — caller-allocated output buffer.
/// `out_cap`        — capacity of `out_buf` in bytes.
/// `out_len`        — on success: bytes written; on ERR_BUF: bytes required.
///
/// Call this on a worker thread — it blocks on network I/O. The C# wrapper
/// already uses `Task.Run` for this purpose.
///
/// # Safety
///
/// `url_utf8` must point to a NUL-terminated C string, and `opts_json_utf8`
/// must be null or point to one; both must stay valid for the duration of the
/// call. `out_len` must point to a writable `i32`. `out_buf` must be null, or
/// point to at least `out_cap` writable bytes. `out_cap` must not overstate the
/// allocation behind `out_buf`.
#[no_mangle]
pub unsafe extern "C" fn unity_dlp_extract(
    url_utf8: *const c_char,
    opts_json_utf8: *const c_char,
    out_buf: *mut u8,
    out_cap: i32,
    out_len: *mut i32,
) -> UnityDlpResult {
    // SAFETY: the pointer contract is this function's own, documented in its
    // # Safety section; it is forwarded unchanged to the inner call.
    ffi_guard(UNITY_DLP_ERR_PYTHON, || unsafe {
        unity_dlp_extract_inner(url_utf8, opts_json_utf8, out_buf, out_cap, out_len)
    })
}

unsafe fn unity_dlp_extract_inner(
    url_utf8: *const c_char,
    opts_json_utf8: *const c_char,
    out_buf: *mut u8,
    out_cap: i32,
    out_len: *mut i32,
) -> UnityDlpResult {
    if url_utf8.is_null() || out_len.is_null() {
        set_last_error("null pointer argument");
        return UNITY_DLP_ERR_INIT;
    }
    if !is_ready() {
        set_last_error("library not initialised; call unity_dlp_init first");
        return UNITY_DLP_ERR_INIT;
    }

    // SAFETY: non-null (checked above); the caller guarantees a
    // NUL-terminated string valid for this call.
    let url = match unsafe { CStr::from_ptr(url_utf8) }.to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error("url is not valid UTF-8");
            return UNITY_DLP_ERR_INIT;
        }
    };

    let opts_json: Option<&str> = if opts_json_utf8.is_null() {
        None
    } else {
        // SAFETY: non-null (checked above); the caller guarantees a
        // NUL-terminated string valid for this call.
        match unsafe { CStr::from_ptr(opts_json_utf8) }.to_str() {
            Ok(s) if !s.is_empty() => Some(s),
            _ => None,
        }
    };

    log::debug!("unity_dlp_extract: url={url}");

    let json = match extract::extract(url, opts_json) {
        Ok(j) => j,
        Err(e) => {
            let message = e.message();
            log::error!("unity_dlp_extract: {message}");
            set_last_error(message);
            // The variant was decided from the raised exception's class, so the
            // code no longer depends on text the caller can influence.
            return match &e {
                extract::ExtractError::Timeout(_) => UNITY_DLP_ERR_TIMEOUT,
                extract::ExtractError::Busy(_) => UNITY_DLP_ERR_BUSY,
                extract::ExtractError::Network(_) => UNITY_DLP_ERR_NET,
                extract::ExtractError::Js(_) => UNITY_DLP_ERR_JS,
                extract::ExtractError::TooLarge(_) => UNITY_DLP_ERR_TOO_LARGE,
                extract::ExtractError::Python(_) => UNITY_DLP_ERR_PYTHON,
            };
        }
    };

    let bytes = json.as_bytes();
    // The ABI expresses lengths as int32_t. Reject anything it cannot describe
    // rather than narrowing: a wrapped length would be checked against out_cap
    // while the copy below still moved the full payload.
    let Ok(needed) = i32::try_from(bytes.len()) else {
        set_last_error("result exceeds the 2 GiB limit the ABI can express");
        // SAFETY: out_len is non-null (checked above).
        unsafe { *out_len = 0 };
        return UNITY_DLP_ERR_BUF;
    };
    // SAFETY: out_len is non-null (checked above).
    unsafe { *out_len = needed };

    if out_buf.is_null() || out_cap < needed {
        return UNITY_DLP_ERR_BUF;
    }

    // SAFETY: out_buf points to at least out_cap bytes (caller guarantee), and
    // out_cap >= needed == bytes.len() was just checked, so the copy stays
    // inside the caller's buffer.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len()) };
    UNITY_DLP_OK
}

// ── Last error ────────────────────────────────────────────────────────────────

/// Copy the last error message (UTF-8, no NUL terminator) into `out_buf`.
///
/// Returns UNITY_DLP_OK on success, UNITY_DLP_ERR_BUF if the buffer is too
/// small (with `*out_len` set to the required byte count).
///
/// # Safety
///
/// `out_len` must point to a writable `i32`. `out_buf` must be null, or point
/// to at least `out_cap` writable bytes, and `out_cap` must not overstate the
/// allocation behind it.
#[no_mangle]
pub unsafe extern "C" fn unity_dlp_last_error(
    out_buf: *mut u8,
    out_cap: i32,
    out_len: *mut i32,
) -> UnityDlpResult {
    // SAFETY: the pointer contract is this function's own, documented in its
    // # Safety section; it is forwarded unchanged to the inner call.
    ffi_guard(UNITY_DLP_ERR_INIT, || unsafe {
        unity_dlp_last_error_inner(out_buf, out_cap, out_len)
    })
}

unsafe fn unity_dlp_last_error_inner(
    out_buf: *mut u8,
    out_cap: i32,
    out_len: *mut i32,
) -> UnityDlpResult {
    if out_len.is_null() {
        return UNITY_DLP_ERR_INIT;
    }

    let msg = last_error_mutex()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();

    let bytes = msg.as_bytes();
    // set_last_error caps the stored message at MAX_LAST_ERROR_BYTES, so this
    // conversion cannot fail; it is written as a check rather than a cast so the
    // guard below can never be compared against a wrapped length.
    let Ok(needed) = i32::try_from(bytes.len()) else {
        // SAFETY: out_len is non-null (checked above).
        unsafe { *out_len = 0 };
        return UNITY_DLP_ERR_BUF;
    };
    // SAFETY: out_len is non-null (checked above).
    unsafe { *out_len = needed };

    if out_buf.is_null() || out_cap < needed {
        return UNITY_DLP_ERR_BUF;
    }

    // SAFETY: out_buf points to at least out_cap bytes (caller guarantee), and
    // out_cap >= needed == bytes.len() was just checked, so the copy stays
    // inside the caller's buffer.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len()) };
    UNITY_DLP_OK
}

#[cfg(test)]
mod last_error_tests {
    use super::*;

    #[test]
    fn short_messages_are_unchanged() {
        assert_eq!(clamp_error_message("boom".to_string()), "boom");
    }

    #[test]
    fn a_message_at_the_limit_is_unchanged() {
        let msg = "a".repeat(MAX_LAST_ERROR_BYTES);
        assert_eq!(clamp_error_message(msg.clone()), msg);
    }

    #[test]
    fn oversized_messages_are_capped() {
        let got = clamp_error_message("a".repeat(MAX_LAST_ERROR_BYTES * 3));
        assert_eq!(got.len(), MAX_LAST_ERROR_BYTES);
        assert!(got.ends_with(LAST_ERROR_TRUNCATED_SUFFIX));
    }

    #[test]
    fn truncation_lands_on_a_char_boundary() {
        // A multi-byte character straddling the cut must not be split.
        let got = clamp_error_message("é".repeat(MAX_LAST_ERROR_BYTES));
        assert!(got.len() <= MAX_LAST_ERROR_BYTES);
        assert!(got.ends_with(LAST_ERROR_TRUNCATED_SUFFIX));
    }
}
