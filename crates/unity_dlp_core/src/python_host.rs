use once_cell::sync::OnceCell;
use pyo3::prelude::*;
use std::path::PathBuf;

/// Embedded yt-dlp source zip produced by build.rs from vendor/yt-dlp.
static YT_DLP_ZIP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/yt_dlp.zip"));

/// sys.prefix of the CPython used at build time, baked in by build.rs.
/// Empty on cross-compile targets (iOS/Android) where PYTHONHOME is not used.
static PYTHON_PREFIX: &str = env!("UNITY_DLP_PYTHON_PREFIX");

static INIT_RESULT: OnceCell<Result<(), String>> = OnceCell::new();

/// Initialise the embedded Python interpreter and place yt-dlp on sys.path.
///
/// Idempotent: the first call runs initialisation; subsequent calls return the
/// cached result (success or failure). Never calls Py_Finalize.
pub fn init() -> Result<(), String> {
    INIT_RESULT
        .get_or_init(do_init)
        .as_ref()
        .map(|_| ())
        .map_err(|e| e.clone())
}

fn do_init() -> Result<(), String> {
    // Set PYTHONHOME before Py_Initialize so the embedded interpreter can locate
    // its stdlib and C-extension modules (.pyd / .so in the DLLs / lib-dynload dir).
    // On mobile targets the stdlib is bundled differently and PYTHON_PREFIX is empty.
    if !PYTHON_PREFIX.is_empty() {
        // SAFETY: Python has not been initialised yet, so no Python threads exist
        // that might race on getenv.
        unsafe { std::env::set_var("PYTHONHOME", PYTHON_PREFIX) };
    }

    // pyo3::prepare_freethreaded_python calls Py_InitializeEx(0). We do this
    // manually (no auto-initialize feature) so we control the order: env vars
    // first, then init, then sys.path configuration.
    pyo3::prepare_freethreaded_python();

    let zip_path = write_yt_dlp_zip()?;

    Python::with_gil(|py| -> Result<(), String> {
        let sys = py.import_bound("sys").map_err(|e| format!("import sys: {e}"))?;
        let path = sys
            .getattr("path")
            .map_err(|e| format!("sys.path get: {e}"))?;
        path.call_method1("insert", (0i32, zip_path.to_str().unwrap_or("")))
            .map_err(|e| format!("sys.path.insert: {e}"))?;

        log::debug!(
            "python_host: interpreter ready, yt-dlp zip on sys.path ({})",
            zip_path.display()
        );
        Ok(())
    })
}

fn write_yt_dlp_zip() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("unity_dlp");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create temp dir: {e}"))?;
    let path = dir.join("yt_dlp.zip");
    // Always overwrite so a plugin update refreshes the embedded source.
    std::fs::write(&path, YT_DLP_ZIP).map_err(|e| format!("write yt_dlp.zip: {e}"))?;
    Ok(path)
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
