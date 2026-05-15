use once_cell::sync::OnceCell;
use pyo3::prelude::*;

static INIT_RESULT: OnceCell<Result<(), String>> = OnceCell::new();

/// Initialise the embedded Python interpreter.
///
/// `python_home`   — path to the unpacked Python prefix (sets PYTHONHOME).
///                   Empty string skips PYTHONHOME configuration.
/// `packages_path` — path added to sys.path (a .zip or a directory).
///                   Empty string skips sys.path modification.
///
/// Idempotent: the first call runs initialisation; subsequent calls return the
/// cached result regardless of the arguments passed. Never calls Py_Finalize.
pub fn init(python_home: &str, packages_path: &str) -> Result<(), String> {
    INIT_RESULT
        .get_or_init(|| do_init(python_home, packages_path))
        .as_ref()
        .map(|_| ())
        .map_err(|e| e.clone())
}

fn do_init(python_home: &str, packages_path: &str) -> Result<(), String> {
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

    Python::with_gil(|py| -> Result<(), String> {
        if !packages_path.is_empty() {
            // packages_path may be a .zip or a directory; Python handles both.
            let sys = py.import_bound("sys").map_err(|e| format!("import sys: {e}"))?;
            sys.getattr("path")
                .map_err(|e| format!("sys.path get: {e}"))?
                .call_method1("insert", (0i32, packages_path))
                .map_err(|e| format!("sys.path.insert: {e}"))?;
        }

        // Register the unity_dlp_js PyO3 module so unity_dlp_jsc can import it.
        crate::jsc_provider::register_module(py)?;

        // Importing unity_dlp_jsc triggers @register_provider, which enrolls
        // UnityDlpJCP into yt-dlp's JCP registry before any extraction runs.
        py.run_bound("import unity_dlp_jsc", None, None)
            .map_err(|e| format!("import unity_dlp_jsc: {e}"))?;

        log::debug!(
            "python_host: interpreter ready (home={:?} packages={:?}) — unity_dlp_jsc registered",
            python_home, packages_path
        );
        Ok(())
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
