//! Loads a staged `unity_dlp` binary the way a player does and initialises it
//! against the Python runtime and packages staged beside it.
//!
//! `unity_dlp_cli` links `unity_dlp_core` statically and finds Python through the
//! build machine's environment, so it passes whether or not the staged artifact
//! is complete. This resolves the library by path out of an unpacked artifact and
//! initialises it, so a runtime that did not travel with the plugin fails here
//! rather than in a consumer's editor.
//!
//! ```text
//! unity_dlp_loadcheck --package-root <dir> --python-home <dir> [--strict]
//! ```
//!
//! `--package-root` holds `Plugins/` and `StreamingAssets/` as the artifact lays
//! them out; `--python-home` is the stdlib zip already extracted, which is what
//! the package hands the library at runtime. `--strict` also fails on a degraded
//! init, where the interpreter is up but the JS shim did not register.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};

// Transcribed from unity_dlp_core::ffi rather than imported: depending on that
// crate would link it statically and defeat the point of the check.
const UNITY_DLP_OK: c_int = 0;
const UNITY_DLP_OK_DEGRADED: c_int = 1;

type InitFn = unsafe extern "C" fn(*const c_char, *const c_char) -> c_int;
type VersionFn = unsafe extern "C" fn() -> *const c_char;
type ShutdownFn = unsafe extern "C" fn() -> c_int;
type LastErrorFn = unsafe extern "C" fn(*mut u8, c_int, *mut c_int) -> c_int;

#[cfg(windows)]
mod sys {
    use std::ffi::{c_char, c_void, CString};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    // LoadLibraryExW resolves the *dependencies* of the library it loads against
    // the calling process's own directory, not the library's. The Python runtime
    // sits beside unity_dlp.dll rather than beside this binary, so without this
    // flag a complete artifact still fails to load.
    const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
        fn GetLastError() -> u32;
    }

    pub fn load(path: &Path) -> Result<*mut c_void, String> {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call, and null is the
        // documented value for the reserved file parameter.
        let handle = unsafe {
            LoadLibraryExW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                LOAD_WITH_ALTERED_SEARCH_PATH,
            )
        };
        if handle.is_null() {
            // SAFETY: nothing runs between the failed call and this read that
            // could reset the thread's last-error value.
            let code = unsafe { GetLastError() };
            return Err(format!("LoadLibraryExW failed with error {code}"));
        }
        Ok(handle)
    }

    pub fn symbol(handle: *mut c_void, name: &str) -> Result<*mut c_void, String> {
        let c_name =
            CString::new(name).map_err(|_| format!("symbol name {name} contains a NUL byte"))?;
        // SAFETY: `handle` came from a successful LoadLibraryExW above, and
        // `c_name` is NUL-terminated and outlives the call.
        let addr = unsafe { GetProcAddress(handle, c_name.as_ptr()) };
        if addr.is_null() {
            return Err(format!("symbol not exported: {name}"));
        }
        Ok(addr)
    }
}

#[cfg(unix)]
mod sys {
    use std::ffi::{c_char, c_int, c_void, CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    const RTLD_NOW: c_int = 2;

    // glibc folded libdl into libc in 2.34, but the stub stays linkable and older
    // runners still need the explicit link. macOS has no libdl at all.
    #[cfg_attr(target_os = "linux", link(name = "dl"))]
    extern "C" {
        fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlerror() -> *const c_char;
    }

    fn last_error() -> String {
        // SAFETY: dlerror returns null or a NUL-terminated string owned by the
        // loader and valid until the next dlerror call on this thread.
        let ptr = unsafe { dlerror() };
        if ptr.is_null() {
            return "unknown error".to_string();
        }
        // SAFETY: non-null as checked above, and NUL-terminated per the contract.
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }

    pub fn load(path: &Path) -> Result<*mut c_void, String> {
        let c_path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| "library path contains a NUL byte".to_string())?;
        // SAFETY: `c_path` is NUL-terminated and outlives the call.
        let handle = unsafe { dlopen(c_path.as_ptr(), RTLD_NOW) };
        if handle.is_null() {
            return Err(last_error());
        }
        Ok(handle)
    }

    pub fn symbol(handle: *mut c_void, name: &str) -> Result<*mut c_void, String> {
        let c_name =
            CString::new(name).map_err(|_| format!("symbol name {name} contains a NUL byte"))?;
        // SAFETY: `handle` came from a successful dlopen above, and `c_name` is
        // NUL-terminated and outlives the call.
        let addr = unsafe { dlsym(handle, c_name.as_ptr()) };
        if addr.is_null() {
            return Err(format!("symbol not exported: {name} ({})", last_error()));
        }
        Ok(addr)
    }
}

fn fail(message: &str) -> ! {
    eprintln!("loadcheck: {message}");
    std::process::exit(1);
}

/// Resolve `name` in `handle` and reinterpret it as `F`.
///
/// # Safety
///
/// `F` must be the signature the exported symbol was compiled with.
unsafe fn symbol_as<F: Copy>(handle: *mut c_void, name: &str) -> F {
    assert_eq!(
        std::mem::size_of::<F>(),
        std::mem::size_of::<*mut c_void>(),
        "F must be a bare function pointer"
    );
    let addr = sys::symbol(handle, name).unwrap_or_else(|e| fail(&e));
    // SAFETY: the assertion above rules out a size mismatch, and the caller
    // guarantees F matches the symbol's actual signature.
    unsafe { std::mem::transmute_copy(&addr) }
}

/// The plugin's staged filename, matching what the build scripts produce.
fn plugin_file_name() -> &'static str {
    if cfg!(windows) {
        "unity_dlp.dll"
    } else if cfg!(target_os = "macos") {
        "unity_dlp.dylib"
    } else {
        "libunity_dlp.so"
    }
}

fn read_last_error(last_error: LastErrorFn) -> String {
    let mut buf = vec![0u8; 4096];
    let mut len: c_int = 0;
    let cap = c_int::try_from(buf.len()).unwrap_or(c_int::MAX);
    // SAFETY: `buf` holds `cap` writable bytes and `len` points to a live c_int.
    let rc = unsafe { last_error(buf.as_mut_ptr(), cap, &mut len) };
    if rc != UNITY_DLP_OK {
        return "<error message unavailable>".to_string();
    }
    // On anything but OK the call reports the length it needed rather than what
    // it wrote, so clamp to the allocation before slicing.
    let written = usize::try_from(len.max(0)).unwrap_or(0).min(buf.len());
    String::from_utf8_lossy(&buf[..written]).into_owned()
}

/// Name what is actually in `dir`. A staging step that renamed or dropped a file
/// is the usual cause of a failure here, and the listing identifies it directly.
fn list_dir(dir: &Path) {
    eprintln!("loadcheck: contents of {}:", dir.display());
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                eprintln!("    {}", entry.file_name().to_string_lossy());
            }
        }
        Err(e) => eprintln!("    <unreadable: {e}>"),
    }
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path)
        .unwrap_or_else(|e| fail(&format!("could not resolve {}: {e}", path.display())))
}

fn to_c_string(path: &Path, label: &str) -> CString {
    CString::new(path.to_string_lossy().as_bytes().to_vec())
        .unwrap_or_else(|_| fail(&format!("{label} contains a NUL byte")))
}

fn main() {
    let mut package_root: Option<PathBuf> = None;
    let mut python_home: Option<PathBuf> = None;
    let mut strict = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--package-root" => package_root = args.next().map(PathBuf::from),
            "--python-home" => python_home = args.next().map(PathBuf::from),
            "--strict" => strict = true,
            other => fail(&format!("unknown argument: {other}")),
        }
    }

    let Some(package_root) = package_root else {
        fail("missing --package-root")
    };
    let Some(python_home) = python_home else {
        fail("missing --python-home")
    };

    if !python_home.is_dir() {
        fail(&format!(
            "python home is not a directory: {}",
            python_home.display()
        ));
    }

    let plugin = package_root
        .join("Plugins")
        .join("x86_64")
        .join(plugin_file_name());
    if !plugin.is_file() {
        list_dir(plugin.parent().unwrap_or(package_root.as_path()));
        fail(&format!("plugin not staged: {}", plugin.display()));
    }

    let packages = package_root
        .join("StreamingAssets")
        .join("dlp")
        .join("yt_dlp.zip");
    if !packages.is_file() {
        list_dir(packages.parent().unwrap_or(package_root.as_path()));
        fail(&format!("packages zip not staged: {}", packages.display()));
    }

    // LOAD_WITH_ALTERED_SEARCH_PATH is what lets the loader find the Python
    // runtime beside the plugin, and Windows only honours it for a fully
    // qualified path. A relative one silently searches this binary's directory
    // instead and reports the plugin itself as missing.
    let plugin = absolute(&plugin);

    let handle = sys::load(&plugin)
        .unwrap_or_else(|e| fail(&format!("could not load {}: {e}", plugin.display())));
    println!("loadcheck: loaded {}", plugin.display());

    // SAFETY: every signature below is transcribed from the exported
    // declarations in unity_dlp_core::ffi.
    let init_fn = unsafe { symbol_as::<InitFn>(handle, "unity_dlp_init") };
    // SAFETY: as above.
    let version_fn = unsafe { symbol_as::<VersionFn>(handle, "unity_dlp_version") };
    // SAFETY: as above.
    let shutdown_fn = unsafe { symbol_as::<ShutdownFn>(handle, "unity_dlp_shutdown") };
    // SAFETY: as above.
    let last_error_fn = unsafe { symbol_as::<LastErrorFn>(handle, "unity_dlp_last_error") };

    // SAFETY: unity_dlp_version returns a static NUL-terminated string valid for
    // the lifetime of the process.
    let version = unsafe { CStr::from_ptr(version_fn()) }
        .to_string_lossy()
        .into_owned();
    println!("loadcheck: version {version}");

    // PYTHONHOME and sys.path entries are resolved by the interpreter, not by
    // this process, so both have to be absolute by the time they are handed over.
    let home_c = to_c_string(&absolute(&python_home), "python home");
    let packages_c = to_c_string(&absolute(&packages), "packages path");

    // SAFETY: both strings are NUL-terminated and outlive the call, which is the
    // whole of unity_dlp_init's documented pointer contract.
    let rc = unsafe { init_fn(home_c.as_ptr(), packages_c.as_ptr()) };
    if rc == UNITY_DLP_OK {
        println!("loadcheck: init ok");
    } else if rc == UNITY_DLP_OK_DEGRADED {
        let reason = read_last_error(last_error_fn);
        if strict {
            fail(&format!("init degraded under --strict: {reason}"));
        }
        println!("loadcheck: init degraded, JS shim unregistered: {reason}");
    } else {
        fail(&format!(
            "init failed ({rc}): {}",
            read_last_error(last_error_fn)
        ));
    }

    // SAFETY: init reported success above, which is shutdown's precondition.
    let rc = unsafe { shutdown_fn() };
    if rc != UNITY_DLP_OK {
        fail(&format!("shutdown failed ({rc})"));
    }
    println!("loadcheck: shutdown ok");
}
