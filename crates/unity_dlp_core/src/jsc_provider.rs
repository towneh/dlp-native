use pyo3::prelude::*;

#[cfg(all(feature = "js-v8", feature = "js-quickjs"))]
compile_error!("features `js-v8` and `js-quickjs` are mutually exclusive");

#[cfg(not(any(feature = "js-v8", feature = "js-quickjs")))]
compile_error!("one of `js-v8` or `js-quickjs` must be enabled");

/// Evaluate `script` inside an isolated JS context and return the captured
/// console.log output as a string.
///
/// The EJS solver emits its JSON result via a single `console.log(JSON.stringify(…))`
/// call. We prepend a shim that redirects console to an array so the output can
/// be returned without a subprocess.
pub fn run_js(script: &str) -> Result<String, String> {
    let src = wrap_script(script);
    run_js_inner(&src)
}

fn wrap_script(script: &str) -> String {
    let mut src = String::with_capacity(script.len() + 300);
    src.push_str(
        "(function(){\
            var __out=[];\
            globalThis.console={\
                log:function(){__out.push([].slice.call(arguments).join(' '));},\
                warn:function(){__out.push([].slice.call(arguments).join(' '));},\
                error:function(){__out.push([].slice.call(arguments).join(' '));}\
            };",
    );
    src.push_str(script);
    // The leading newline matters: a script ending in a // line comment would
    // otherwise swallow this suffix and the whole wrapper would fail to parse.
    src.push_str("\n;return __out.join('\\n');})()");
    src
}

/// Wall-clock ceiling for one script. Deliberately below the extraction
/// timeout so a runaway script is reported as a JS failure rather than
/// surfacing later as a generic extraction timeout.
const JS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Heap ceiling for one script.
///
/// On the V8 backend this also arms rustyscript's near-heap-limit callback,
/// which terminates the isolate when a script allocates its way to the ceiling.
/// That covers allocation-driven runaways only — a script that spins without
/// allocating never trips it, which is what the watchdog below is for.
const JS_MAX_HEAP_BYTES: usize = 256 * 1024 * 1024;

// ── V8 backend (Windows, macOS) ───────────────────────────────────────────────

#[cfg(feature = "js-v8")]
fn run_js_inner(src: &str) -> Result<String, String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use rustyscript::{Error as JsError, Runtime, RuntimeOptions};

    fn inner(src: &str) -> Result<String, JsError> {
        let mut rt = Runtime::new(RuntimeOptions {
            timeout: JS_TIMEOUT,
            max_heap_size: Some(JS_MAX_HEAP_BYTES),
            ..Default::default()
        })?;

        // Neither option above stops a script that spins without allocating.
        // rustyscript's timeout is a tokio select against a sleep, which a
        // synchronous loop never yields to, and its terminate callback is
        // installed on the near-heap-limit hook, so it only fires if the script
        // allocates. Terminating the isolate from another thread is the only
        // thing that interrupts a non-allocating loop.
        let isolate = rt.deno_runtime().v8_isolate().thread_safe_handle();
        let finished = Arc::new(AtomicBool::new(false));
        // Fail closed: with no watchdog there is nothing to stop a
        // non-allocating loop, so refuse to evaluate rather than run unbounded.
        let watchdog = {
            let finished = Arc::clone(&finished);
            std::thread::Builder::new()
                .name("unity_dlp_js_watchdog".to_string())
                .spawn(move || {
                    // Parked rather than polled, so a script that finishes
                    // normally wakes this thread immediately instead of leaving
                    // the caller to wait out a sleep interval. park_timeout may
                    // also return spuriously, hence the re-checking loop.
                    let deadline = std::time::Instant::now() + JS_TIMEOUT;
                    loop {
                        if finished.load(Ordering::Acquire) {
                            return;
                        }
                        match deadline.checked_duration_since(std::time::Instant::now()) {
                            Some(remaining) => std::thread::park_timeout(remaining),
                            None => break,
                        }
                    }
                    if !finished.load(Ordering::Acquire) {
                        isolate.terminate_execution();
                    }
                })
                .map_err(|e| JsError::Runtime(format!("could not start the JS watchdog: {e}")))?
        };

        let result = rt.eval::<String>(src);
        finished.store(true, Ordering::Release);
        watchdog.thread().unpark();
        let _ = watchdog.join();
        result
    }

    inner(src).map_err(|e| format!("rustyscript: {e}"))
}

// ── QuickJS backend (Linux, Android, iOS) ────────────────────────────────────

#[cfg(feature = "js-quickjs")]
fn run_js_inner(src: &str) -> Result<String, String> {
    use rquickjs::{Context, Runtime};

    let rt = Runtime::new().map_err(|e| format!("rquickjs init: {e}"))?;
    rt.set_memory_limit(JS_MAX_HEAP_BYTES);

    // QuickJS has no watchdog thread: without an interrupt handler a script
    // that enters a loop cannot be stopped at all. The handler is polled by the
    // interpreter, so returning true unwinds it back out to the eval call.
    let deadline = std::time::Instant::now() + JS_TIMEOUT;
    rt.set_interrupt_handler(Some(Box::new(move || {
        std::time::Instant::now() >= deadline
    })));

    let ctx = Context::full(&rt).map_err(|e| format!("rquickjs context: {e}"))?;
    ctx.with(|ctx| {
        ctx.eval::<String, _>(src.as_bytes())
            .map_err(|e| format!("rquickjs: {e}"))
    })
}

// ── PyO3 surface ──────────────────────────────────────────────────────────────

#[pyfunction]
#[pyo3(name = "run_js")]
fn py_run_js(py: Python<'_>, script: String) -> PyResult<String> {
    // The JS engine touches no Python objects, so the GIL is released for the
    // duration of the call. Holding it here would mean one slow or hostile
    // script blocks every other thread that needs the interpreter, including
    // every subsequent extraction.
    py.allow_threads(|| run_js(&script))
        .map_err(pyo3::exceptions::PyRuntimeError::new_err)
}

/// Register `unity_dlp_js` into `sys.modules` so the Python JCP shim can do
/// `import unity_dlp_js; unity_dlp_js.run_js(stdin)`.
pub fn register_module(py: Python<'_>) -> Result<(), String> {
    (|| -> PyResult<()> {
        let m = PyModule::new_bound(py, "unity_dlp_js")?;
        m.add_function(wrap_pyfunction!(py_run_js, &m)?)?;
        py.import_bound("sys")?.getattr("modules")?.set_item("unity_dlp_js", &m)?;
        Ok(())
    })()
    .map_err(|e| format!("register unity_dlp_js: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_console_output() {
        let out = run_js("console.log('ok')").expect("script should evaluate");
        assert_eq!(out.trim(), "ok");
    }

    #[test]
    fn a_trailing_line_comment_does_not_swallow_the_wrapper() {
        let out = run_js("console.log('ok') // trailing comment")
            .expect("a script ending in a line comment must still evaluate");
        assert_eq!(out.trim(), "ok");
    }

    /// The point of the engine budget: a script that never returns has to be
    /// stopped by the engine, because nothing above it can interrupt a thread
    /// parked in native JS.
    #[test]
    fn a_runaway_script_is_terminated() {
        // Run on a worker and wait with a timeout: if the budget ever stops
        // working, this has to fail rather than hang the test binary until CI
        // kills the job.
        let (tx, rx) = std::sync::mpsc::channel();
        let started = std::time::Instant::now();
        std::thread::spawn(move || {
            let _ = tx.send(run_js("while (true) {}").is_err());
        });
        let errored = rx
            .recv_timeout(JS_TIMEOUT * 4)
            .expect("the engine budget must stop the script");
        let elapsed = started.elapsed();

        assert!(errored, "an infinite loop must not return Ok");
        assert!(
            elapsed < JS_TIMEOUT * 4,
            "expected termination near the {JS_TIMEOUT:?} budget, took {elapsed:?}"
        );
    }
}
