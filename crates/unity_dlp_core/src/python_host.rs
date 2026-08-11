use once_cell::sync::OnceCell;
use pyo3::prelude::*;

/// Outcome of a successful init. `Some(msg)` means the interpreter is up and
/// extraction works, but the `unity_dlp_jsc` shim did not register (YouTube
/// JS-challenge path degraded); `msg` is the import error for `last_error`.
type InitOutcome = Result<Option<String>, String>;

static INIT_RESULT: OnceCell<InitOutcome> = OnceCell::new();

/// Initialise the embedded Python interpreter.
///
/// `python_home`   — path to the unpacked Python prefix (sets PYTHONHOME).
///                   Empty string skips PYTHONHOME configuration.
/// `packages_path` — a `\n`-delimited list of paths added to sys.path (each a
///                   .zip or a directory). Earlier entries win resolution.
///                   Empty string skips sys.path modification. Newlines are used
///                   as the separator because `;`/`:` are legal path characters.
///
/// Returns `Ok(None)` on a clean init, `Ok(Some(msg))` when the interpreter came
/// up but the `unity_dlp_jsc` shim failed to import (degraded — extraction still
/// works, YouTube JS-challenge path does not), and `Err` on a hard failure.
///
/// Idempotent: the first call runs initialisation; subsequent calls return the
/// cached result regardless of the arguments passed. Never calls Py_Finalize.
pub fn init(python_home: &str, packages_path: &str) -> InitOutcome {
    INIT_RESULT
        .get_or_init(|| do_init(python_home, packages_path))
        .clone()
}

/// Split a `\n`-delimited packages path into its non-empty segments, in order.
/// The first segment ends up first in `sys.path`, so it wins resolution.
fn parse_packages_path(packages_path: &str) -> Vec<&str> {
    packages_path
        .split('\n')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Set an environment variable so the embedded CPython actually sees it.
///
/// On Windows this is not just `std::env::set_var`. `set_var` calls
/// `SetEnvironmentVariableW`, which updates the Win32 environment block but *not*
/// the UCRT's cached wide environment (`_wenviron`). CPython's path configuration
/// reads `PYTHONHOME` via `_wgetenv`, which serves that CRT cache — and the cache
/// is materialised lazily on first access, then never re-synced with the Win32
/// block. In a short-lived process nothing touches the CRT env before this runs,
/// so the lazy cache picks the value up; but in a long-lived host (e.g. the Unity
/// Editor) the CRT env was frozen long before, `SetEnvironmentVariableW` lands too
/// late, `_wgetenv` returns NULL, and `Py_Initialize` fails to find the stdlib —
/// which aborts the whole host process. So we also push the value through the CRT
/// (`_wputenv_s`), which updates `_wenviron` (and syncs the Win32 block). Rust and
/// python3.dll share `ucrtbase.dll` here (no `crt-static`), so this reaches the
/// same `_wenviron` the interpreter reads.
fn set_env_for_python(name: &str, value: &str) {
    // SAFETY: `set_var` is unsound if any other thread in the *process* touches
    // the environment concurrently — not just any other Python thread. Nothing
    // here can enforce that, because this code runs inside a long-lived,
    // many-threaded host (see the note above about the Unity Editor). What
    // bounds it is the call site: this runs exactly once, from `do_init`, behind
    // the `INIT_RESULT` OnceCell, before `Py_Initialize`, and the exported
    // `unity_dlp_init` is documented as the first call a host may make. The
    // residual risk is a host that mutates the environment from another thread
    // while it is calling `unity_dlp_init`.
    unsafe { std::env::set_var(name, value) };

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        extern "C" {
            // ucrtbase: errno_t _wputenv_s(const wchar_t* name, const wchar_t* value);
            fn _wputenv_s(name: *const u16, value: *const u16) -> i32;
        }
        let to_wide = |s: &str| -> Vec<u16> {
            std::ffi::OsStr::new(s)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        };
        let wname = to_wide(name);
        let wvalue = to_wide(value);
        // SAFETY: both pointers are NUL-terminated UTF-16 buffers that outlive the
        // call; _wputenv_s copies them.
        unsafe { _wputenv_s(wname.as_ptr(), wvalue.as_ptr()) };
    }
}

/// Point OpenSSL at the CA bundle shipped beside the stdlib, when there is one.
///
/// A libssl built for another prefix looks for its trust store where that prefix would
/// have been, and finds nothing on a device that is not it. Every TLS connection then
/// fails verification — the handshake works, so this surfaces as
/// `CERTIFICATE_VERIFY_FAILED: unable to get local issuer certificate` from deep inside
/// an extractor rather than as anything resembling a missing file.
///
/// Only the platforms whose stdlib carries the bundle are affected: elsewhere the file
/// is absent and the variable stays unset, leaving Python's own defaults alone (on
/// Windows that is the system certificate store, which already works).
fn set_ca_bundle(python_home: &str) {
    let bundle = std::path::Path::new(python_home)
        .join("etc")
        .join("tls")
        .join("cert.pem");
    if bundle.is_file() {
        // Read by OpenSSL when the default verify paths are loaded, which
        // ssl.create_default_context does — so this reaches yt-dlp's own sessions.
        set_env_for_python("SSL_CERT_FILE", &bundle.to_string_lossy());
    }
}

fn do_init(python_home: &str, packages_path: &str) -> InitOutcome {
    // Set PYTHONHOME before Py_Initialize so the embedded interpreter can locate
    // its stdlib and C-extension modules (.pyd / .so in the DLLs / lib-dynload dir).
    // Goes through set_env_for_python, not std::env::set_var directly — see that
    // function for why the Win32-only path silently fails inside a long-lived host.
    if !python_home.is_empty() {
        set_env_for_python("PYTHONHOME", python_home);
        set_ca_bundle(python_home);
    }

    // pyo3::prepare_freethreaded_python calls Py_InitializeEx(0). We do this
    // manually (no auto-initialize feature) so we control the order: env vars
    // first, then init, then sys.path configuration.
    pyo3::prepare_freethreaded_python();

    Python::with_gil(|py| -> InitOutcome {
        // Insert each segment at an increasing index so the list order is
        // preserved in sys.path (segment 0 at index 0, segment 1 at index 1, …).
        // Each entry may be a .zip or a directory; Python handles both.
        let segments = parse_packages_path(packages_path);
        if !segments.is_empty() {
            let sys = py
                .import_bound("sys")
                .map_err(|e| format!("import sys: {e}"))?;
            let path = sys
                .getattr("path")
                .map_err(|e| format!("sys.path get: {e}"))?;
            for (i, segment) in segments.iter().enumerate() {
                path.call_method1("insert", (i, *segment))
                    .map_err(|e| format!("sys.path.insert: {e}"))?;
            }
        }

        // Register the unity_dlp_js PyO3 module so unity_dlp_jsc can import it.
        crate::jsc_provider::register_module(py)?;

        // Importing unity_dlp_jsc triggers @register_provider, which enrolls
        // UnityDlpJCP into yt-dlp's JCP registry before any extraction runs. A
        // failure here (missing from the loaded package, or API skew against a
        // staged yt-dlp) only knocks out the YouTube JS-challenge path — the rest
        // of yt-dlp still works — so it degrades rather than aborting init.
        let degraded = match py.run_bound("import unity_dlp_jsc", None, None) {
            Ok(()) => None,
            Err(e) => Some(format!("import unity_dlp_jsc: {e}")),
        };

        log::debug!(
            "python_host: interpreter ready (home={:?} packages={:?}) — unity_dlp_jsc {}",
            python_home,
            packages_path,
            if degraded.is_none() {
                "registered"
            } else {
                "FAILED (degraded)"
            }
        );
        Ok(degraded)
    })
}

/// Acquire the Python GIL and run `f`.
///
/// `init()` must succeed before calling this. Do not hold the GIL across an
/// `.await` point — release it before any async yield.
pub fn with_python<F, R>(f: F) -> R
where
    F: for<'py> FnOnce(Python<'py>) -> R,
{
    Python::with_gil(f)
}

#[cfg(test)]
mod tests {
    use super::parse_packages_path;

    #[test]
    fn splits_and_preserves_order() {
        assert_eq!(
            parse_packages_path("a.zip\nb.zip\nc"),
            vec!["a.zip", "b.zip", "c"]
        );
    }

    #[test]
    fn skips_empty_segments() {
        assert_eq!(
            parse_packages_path("\na.zip\n\n\nb.zip\n"),
            vec!["a.zip", "b.zip"]
        );
    }

    #[test]
    fn single_entry_unchanged() {
        assert_eq!(parse_packages_path("only.zip"), vec!["only.zip"]);
    }

    #[test]
    fn empty_yields_no_segments() {
        assert!(parse_packages_path("").is_empty());
        assert!(parse_packages_path("\n\n").is_empty());
    }
}
