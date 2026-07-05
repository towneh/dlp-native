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

fn do_init(python_home: &str, packages_path: &str) -> InitOutcome {
    // Set PYTHONHOME before Py_Initialize so the embedded interpreter can locate
    // its stdlib and C-extension modules (.pyd / .so in the DLLs / lib-dynload dir).
    if !python_home.is_empty() {
        // SAFETY: Python has not been initialised yet, so no Python threads exist
        // that might race on getenv.
        unsafe { std::env::set_var("PYTHONHOME", python_home) };
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
            let sys = py.import_bound("sys").map_err(|e| format!("import sys: {e}"))?;
            let path = sys.getattr("path").map_err(|e| format!("sys.path get: {e}"))?;
            for (i, segment) in segments.iter().enumerate() {
                path.call_method1("insert", (i as i32, *segment))
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
            if degraded.is_none() { "registered" } else { "FAILED (degraded)" }
        );
        Ok(degraded)
    })
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
        assert_eq!(parse_packages_path("\na.zip\n\n\nb.zip\n"), vec!["a.zip", "b.zip"]);
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
