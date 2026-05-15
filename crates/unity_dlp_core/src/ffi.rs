use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, Ordering};

use once_cell::sync::OnceCell;

use crate::{extract, logging, python_host};

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

pub type UnityDlpResult = i32;

pub const UNITY_DLP_OK: UnityDlpResult = 0;
pub const UNITY_DLP_ERR_INIT: UnityDlpResult = -1;
pub const UNITY_DLP_ERR_PYTHON: UnityDlpResult = -2;
pub const UNITY_DLP_ERR_JS: UnityDlpResult = -3;
pub const UNITY_DLP_ERR_NET: UnityDlpResult = -4;
/// out_buf too small; out_len holds the required byte count.
pub const UNITY_DLP_ERR_BUF: UnityDlpResult = -5;

// ── Init / shutdown ───────────────────────────────────────────────────────────

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialise the native library.
///
/// `python_home_utf8`   — NUL-terminated path to the unpacked Python prefix
///                        (sets PYTHONHOME). Nullable; null or empty skips it.
/// `packages_path_utf8` — NUL-terminated path added to sys.path (a .zip or a
///                        directory). Nullable; null or empty skips it.
///
/// Must succeed before calling any other function. Safe to call from multiple
/// threads — only the first call runs initialisation; subsequent calls are
/// no-ops that return UNITY_DLP_OK.
#[no_mangle]
pub extern "C" fn unity_dlp_init(
    python_home_utf8: *const c_char,
    packages_path_utf8: *const c_char,
) -> UnityDlpResult {
    if INITIALIZED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return UNITY_DLP_OK;
    }

    logging::init();

    let python_home = if python_home_utf8.is_null() {
        ""
    } else {
        match unsafe { CStr::from_ptr(python_home_utf8) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                set_last_error("python_home is not valid UTF-8");
                INITIALIZED.store(false, Ordering::SeqCst);
                return UNITY_DLP_ERR_INIT;
            }
        }
    };

    let packages_path = if packages_path_utf8.is_null() {
        ""
    } else {
        match unsafe { CStr::from_ptr(packages_path_utf8) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                set_last_error("packages_path is not valid UTF-8");
                INITIALIZED.store(false, Ordering::SeqCst);
                return UNITY_DLP_ERR_INIT;
            }
        }
    };

    if let Err(e) = python_host::init(python_home, packages_path) {
        log::error!("unity_dlp_init: Python init failed: {e}");
        set_last_error(e);
        INITIALIZED.store(false, Ordering::SeqCst);
        return UNITY_DLP_ERR_INIT;
    }

    log::info!("unity_dlp_init: library initialised");
    UNITY_DLP_OK
}

/// Shut down the native library and release resources.
///
/// After this call the library is uninitialised. Do not call other functions
/// until `unity_dlp_init` succeeds again.
#[no_mangle]
pub extern "C" fn unity_dlp_shutdown() -> UnityDlpResult {
    if INITIALIZED
        .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return UNITY_DLP_OK;
    }

    log::info!("unity_dlp_shutdown: library shut down");
    UNITY_DLP_OK
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
#[no_mangle]
pub extern "C" fn unity_dlp_extract(
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
    if !INITIALIZED.load(Ordering::SeqCst) {
        set_last_error("library not initialised; call unity_dlp_init first");
        return UNITY_DLP_ERR_INIT;
    }

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
        match unsafe { CStr::from_ptr(opts_json_utf8) }.to_str() {
            Ok(s) if !s.is_empty() => Some(s),
            _ => None,
        }
    };

    log::debug!("unity_dlp_extract: url={url}");

    let json = match extract::extract(url, opts_json) {
        Ok(j) => j,
        Err(e) => {
            log::error!("unity_dlp_extract: {e}");
            set_last_error(&e);
            // Classify the error so C# can give a typed exception.
            let code = if e.contains("URLError")
                || e.contains("ConnectionError")
                || e.contains("HTTP Error")
                || e.contains("Network")
            {
                UNITY_DLP_ERR_NET
            } else {
                UNITY_DLP_ERR_PYTHON
            };
            return code;
        }
    };

    let bytes = json.as_bytes();
    let needed = bytes.len() as i32;
    // SAFETY: out_len is non-null (checked above).
    unsafe { *out_len = needed };

    if out_buf.is_null() || out_cap < needed {
        return UNITY_DLP_ERR_BUF;
    }

    // SAFETY: out_buf points to at least out_cap bytes (caller guarantee).
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len()) };
    UNITY_DLP_OK
}

// ── Last error ────────────────────────────────────────────────────────────────

/// Copy the last error message (UTF-8, no NUL terminator) into `out_buf`.
///
/// Returns UNITY_DLP_OK on success, UNITY_DLP_ERR_BUF if the buffer is too
/// small (with `*out_len` set to the required byte count).
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

    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len()) };
    UNITY_DLP_OK
}
