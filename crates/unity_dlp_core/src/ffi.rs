use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, Ordering};

use once_cell::sync::OnceCell;

use crate::logging;

// ── Error storage ─────────────────────────────────────────────────────────────

static LAST_ERROR: OnceCell<std::sync::Mutex<String>> = OnceCell::new();

fn last_error_mutex() -> &'static std::sync::Mutex<String> {
    LAST_ERROR.get_or_init(|| std::sync::Mutex::new(String::new()))
}

fn set_last_error(msg: impl Into<String>) {
    if let Ok(mut guard) = last_error_mutex().lock() {
        *guard = msg.into();
    }
}

// ── Result type ───────────────────────────────────────────────────────────────

/// Return type for all C ABI functions.
///
/// Non-negative = success, negative = error code.
pub type UnityDlpResult = i32;

pub const UNITY_DLP_OK: UnityDlpResult = 0;
pub const UNITY_DLP_ERR_INIT: UnityDlpResult = -1;
pub const UNITY_DLP_ERR_PYTHON: UnityDlpResult = -2;
pub const UNITY_DLP_ERR_JS: UnityDlpResult = -3;
pub const UNITY_DLP_ERR_NET: UnityDlpResult = -4;
/// out_buf too small; out_len holds required byte count.
pub const UNITY_DLP_ERR_BUF: UnityDlpResult = -5;

// ── Init / shutdown ───────────────────────────────────────────────────────────

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialize the native library.
///
/// Must be called once before any other function. Safe to call from any thread;
/// subsequent calls are no-ops and return UNITY_DLP_OK.
#[no_mangle]
pub extern "C" fn unity_dlp_init() -> UnityDlpResult {
    if INITIALIZED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        // Already initialized.
        return UNITY_DLP_OK;
    }

    logging::init();
    log::info!("unity_dlp_init: library initialized (phase-0 stub)");
    UNITY_DLP_OK
}

/// Shut down the native library and release resources.
///
/// After this call the library is in an uninitialized state. Do not call any
/// other function until `unity_dlp_init` is called again.
#[no_mangle]
pub extern "C" fn unity_dlp_shutdown() -> UnityDlpResult {
    if INITIALIZED
        .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        // Not initialized — harmless.
        return UNITY_DLP_OK;
    }

    log::info!("unity_dlp_shutdown: library shut down");
    UNITY_DLP_OK
}

// ── Version ───────────────────────────────────────────────────────────────────

/// Return a static, null-terminated UTF-8 version string.
///
/// The returned pointer is valid for the lifetime of the process; callers must
/// not free it.
#[no_mangle]
pub extern "C" fn unity_dlp_version() -> *const c_char {
    // SAFETY: literal is valid UTF-8 and NUL-terminated.
    c"unity_dlp/0.1.0 (phase-0)".as_ptr()
}

// ── Extract ───────────────────────────────────────────────────────────────────

/// Extract media metadata for the given URL.
///
/// `url_utf8`      — NUL-terminated URL string (required).
/// `opts_json_utf8`— NUL-terminated JSON options (nullable).
/// `out_buf`       — caller-allocated output buffer.
/// `out_cap`       — capacity of `out_buf` in bytes.
/// `out_len`       — receives the number of bytes written (or bytes needed on
///                   ERR_BUF).
///
/// Returns UNITY_DLP_OK on success, or an error code. On ERR_BUF the caller
/// should reallocate `out_buf` to at least `*out_len` bytes and retry.
#[no_mangle]
pub extern "C" fn unity_dlp_extract(
    url_utf8: *const c_char,
    opts_json_utf8: *const c_char,
    out_buf: *mut u8,
    out_cap: i32,
    out_len: *mut i32,
) -> UnityDlpResult {
    // Phase 0: opts_json_utf8, out_buf, out_cap are unused until Phase 1.
    let _ = (opts_json_utf8, out_buf, out_cap);
    if url_utf8.is_null() || out_len.is_null() {
        set_last_error("unity_dlp_extract: null pointer argument");
        return UNITY_DLP_ERR_INIT;
    }

    let _url = match unsafe { CStr::from_ptr(url_utf8) }.to_str() {
        Ok(s) => s,
        Err(_) => {
            set_last_error("unity_dlp_extract: url is not valid UTF-8");
            return UNITY_DLP_ERR_INIT;
        }
    };

    // Phase-0 stub: extraction is not yet implemented.
    set_last_error("unity_dlp_extract: not implemented in phase-0");
    unsafe { *out_len = 0 };
    UNITY_DLP_ERR_INIT
}

// ── Last error ────────────────────────────────────────────────────────────────

/// Copy the last error message into `out_buf`.
///
/// Returns UNITY_DLP_OK on success, UNITY_DLP_ERR_BUF if the buffer is too
/// small (with `*out_len` set to the required size).
#[no_mangle]
pub extern "C" fn unity_dlp_last_error(
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
    let needed = bytes.len() as i32;
    unsafe { *out_len = needed };

    if out_buf.is_null() || out_cap < needed {
        return UNITY_DLP_ERR_BUF;
    }

    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len());
    }
    UNITY_DLP_OK
}
