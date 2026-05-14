use std::env;
use std::io::Write as _;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let workspace_root = PathBuf::from(&crate_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();

    generate_header(&crate_dir, &workspace_root, &out_dir);
    bundle_yt_dlp(&workspace_root, &out_dir);
    emit_python_prefix(&workspace_root);

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/ffi.rs");
    // Rebuild when yt-dlp submodule is bumped (HEAD moves).
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root
            .join(".git/modules/vendor/yt-dlp/HEAD")
            .display()
    );
}

// ── cbindgen C header ─────────────────────────────────────────────────────────

fn generate_header(crate_dir: &str, workspace_root: &PathBuf, out_dir: &PathBuf) {
    let cbindgen_toml = workspace_root.join("crates/unity_dlp_capi/cbindgen.toml");
    let config = cbindgen::Config::from_file(&cbindgen_toml).expect("cbindgen.toml not found");

    // Write into OUT_DIR (for cargo bookkeeping).
    cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(config.clone())
        .generate()
        .expect("cbindgen failed")
        .write_to_file(out_dir.join("unity_dlp.h"));

    // Also write next to the workspace root for easy consumption by callers.
    cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(config)
        .generate()
        .expect("cbindgen failed")
        .write_to_file(workspace_root.join("unity_dlp.h"));
}

// ── yt-dlp zip bundle ─────────────────────────────────────────────────────────

/// Zip the yt-dlp Python package into OUT_DIR/yt_dlp.zip so it can be embedded
/// via `include_bytes!` in python_host.rs.
///
/// The zip root mirrors vendor/yt-dlp/ so that adding the zip file to sys.path
/// makes `import yt_dlp` work directly.
fn bundle_yt_dlp(workspace_root: &PathBuf, out_dir: &PathBuf) {
    let vendor_dir = workspace_root.join("vendor/yt-dlp");
    if !vendor_dir.exists() {
        // Submodule not initialised (e.g. shallow CI clone). Emit an empty zip
        // so the crate still compiles; extraction will fail at runtime.
        eprintln!("cargo:warning=vendor/yt-dlp not found — embedding empty yt_dlp.zip");
        let zip_path = out_dir.join("yt_dlp.zip");
        let file = std::fs::File::create(&zip_path).unwrap();
        zip::ZipWriter::new(file).finish().unwrap();
        return;
    }

    let zip_path = out_dir.join("yt_dlp.zip");
    let file = std::fs::File::create(&zip_path).expect("create yt_dlp.zip");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);

    for entry in walkdir::WalkDir::new(&vendor_dir)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();

        // Only include Python source and data files; skip __pycache__ and
        // compiled .pyc artefacts (we'll let Python create them on first run).
        if path.components().any(|c| c.as_os_str() == "__pycache__") {
            continue;
        }
        match path.extension().and_then(|e| e.to_str()) {
            Some("py") | Some("json") | Some("html") => {}
            _ => continue,
        }

        let rel = path.strip_prefix(&vendor_dir).unwrap();
        // Normalise to forward slashes (zip spec requires this).
        let entry_name = rel.to_string_lossy().replace('\\', "/");

        zip.start_file(&entry_name, options).unwrap();
        let content = std::fs::read(path).unwrap();
        zip.write_all(&content).unwrap();
    }

    zip.finish().expect("finalise yt_dlp.zip");
}

// ── Python prefix ─────────────────────────────────────────────────────────────

/// Query the Python interpreter for its sys.prefix and bake it into the binary
/// as UNITY_DLP_PYTHON_PREFIX so the embedded interpreter can set PYTHONHOME.
///
/// Only called when PYO3_PYTHON is set in the build environment. On cross-compile
/// targets (iOS, Android) PYO3_PYTHON is not set and UNITY_DLP_PYTHON_PREFIX is
/// left empty — those platforms bundle the stdlib differently.
fn emit_python_prefix(workspace_root: &PathBuf) {
    // Allow the build to succeed even without PYO3_PYTHON (cross-compile case).
    let python = match env::var("PYO3_PYTHON") {
        Ok(p) => p,
        Err(_) => {
            // Try uv as a fallback so `cargo build` without env setup still works.
            let uv_out = std::process::Command::new("uv")
                .args(["python", "find", "3.12"])
                .current_dir(workspace_root)
                .output();
            match uv_out {
                Ok(o) if o.status.success() => {
                    String::from_utf8(o.stdout).unwrap_or_default().trim().to_owned()
                }
                _ => {
                    println!("cargo:rustc-env=UNITY_DLP_PYTHON_PREFIX=");
                    return;
                }
            }
        }
    };

    let output = std::process::Command::new(&python)
        .args(["-c", "import sys; print(sys.prefix, end='')"])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let prefix = String::from_utf8(o.stdout).unwrap_or_default();
            println!("cargo:rustc-env=UNITY_DLP_PYTHON_PREFIX={}", prefix);
        }
        _ => {
            eprintln!("cargo:warning=Could not query Python prefix from {python}");
            println!("cargo:rustc-env=UNITY_DLP_PYTHON_PREFIX=");
        }
    }
}
