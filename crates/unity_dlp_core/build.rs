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
    bundle_zip(&workspace_root);

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/ffi.rs");
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join(".git/modules/vendor/yt-dlp/HEAD").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join(".git/modules/vendor/yt-dlp-ejs/HEAD").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join("python/unity_dlp_jsc").display()
    );
}

// ── cbindgen C header ─────────────────────────────────────────────────────────

fn generate_header(crate_dir: &str, workspace_root: &PathBuf, out_dir: &PathBuf) {
    let cbindgen_toml = workspace_root.join("crates/unity_dlp_capi/cbindgen.toml");
    let config = cbindgen::Config::from_file(&cbindgen_toml).expect("cbindgen.toml not found");

    cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(config.clone())
        .generate()
        .expect("cbindgen failed")
        .write_to_file(out_dir.join("unity_dlp.h"));

    cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(config)
        .generate()
        .expect("cbindgen failed")
        .write_to_file(workspace_root.join("unity_dlp.h"));
}

// ── Combined zip bundle ───────────────────────────────────────────────────────

/// Produce yt_dlp.zip containing three Python packages:
///   yt_dlp/          — from vendor/yt-dlp/
///   yt_dlp_ejs/      — from vendor/yt-dlp-ejs/yt_dlp_ejs/ + built JS
///   unity_dlp_jsc/   — from python/unity_dlp_jsc/unity_dlp_jsc/
///
/// The zip lands in unity_package/StreamingAssets/dlp/ so it can be read at
/// runtime by DlpBootstrap and passed to unity_dlp_init as packages_path.
fn bundle_zip(workspace_root: &PathBuf) {
    let dest_dir = workspace_root.join("unity_package/StreamingAssets/dlp");
    std::fs::create_dir_all(&dest_dir).expect("create StreamingAssets/dlp");
    let zip_path = dest_dir.join("yt_dlp.zip");
    let file = std::fs::File::create(&zip_path).expect("create yt_dlp.zip");
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);

    // ── yt_dlp ────────────────────────────────────────────────────────────────
    let yt_dlp_dir = workspace_root.join("vendor/yt-dlp/yt_dlp");
    if yt_dlp_dir.exists() {
        add_python_package(&mut zip, &yt_dlp_dir, "yt_dlp", opts);
    } else {
        eprintln!("cargo:warning=vendor/yt-dlp/yt_dlp not found — embedding empty yt_dlp");
    }

    // ── yt_dlp_ejs ────────────────────────────────────────────────────────────
    add_yt_dlp_ejs(&mut zip, workspace_root, opts);

    // ── unity_dlp_jsc ─────────────────────────────────────────────────────────
    let jsc_dir = workspace_root.join("python/unity_dlp_jsc/unity_dlp_jsc");
    if jsc_dir.exists() {
        add_python_package(&mut zip, &jsc_dir, "unity_dlp_jsc", opts);
    } else {
        eprintln!("cargo:warning=python/unity_dlp_jsc not found — YouTube JCP shim missing");
    }

    zip.finish().expect("finalise yt_dlp.zip");
    println!("cargo:warning=yt_dlp.zip staged to {}", zip_path.display());
}

/// Walk `src_dir` and add all `.py` / `.json` / `.html` files to the zip under
/// `zip_prefix/`, skipping `__pycache__`.
fn add_python_package(
    zip: &mut zip::ZipWriter<std::fs::File>,
    src_dir: &PathBuf,
    zip_prefix: &str,
    opts: zip::write::SimpleFileOptions,
) {
    for entry in walkdir::WalkDir::new(src_dir)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.components().any(|c| c.as_os_str() == "__pycache__") {
            continue;
        }
        match path.extension().and_then(|e| e.to_str()) {
            Some("py") | Some("json") | Some("html") => {}
            _ => continue,
        }
        let rel = path.strip_prefix(src_dir).unwrap();
        let name = format!("{zip_prefix}/{}", rel.to_string_lossy().replace('\\', "/"));
        zip.start_file(&name, opts).unwrap();
        zip.write_all(&std::fs::read(path).unwrap()).unwrap();
    }
}

fn add_yt_dlp_ejs(
    zip: &mut zip::ZipWriter<std::fs::File>,
    workspace_root: &PathBuf,
    opts: zip::write::SimpleFileOptions,
) {
    let ejs_dir = workspace_root.join("vendor/yt-dlp-ejs");
    if !ejs_dir.exists() {
        eprintln!("cargo:warning=vendor/yt-dlp-ejs not found — YouTube extraction will use built-in vendored scripts");
        return;
    }

    // Python sources
    add_python_package(zip, &ejs_dir.join("yt_dlp_ejs"), "yt_dlp_ejs", opts);

    // Synthetic _version.py (normally generated by hatch-vcs at install time)
    zip.start_file("yt_dlp_ejs/_version.py", opts).unwrap();
    zip.write_all(b"version = \"0.8.0\"\n").unwrap();

    // Build the JS bundles, then embed them so importlib.resources can read them.
    let js_built = run_hatch_build_py(workspace_root, &ejs_dir);
    if js_built {
        let dist = ejs_dir.join("dist");
        for (src, dst) in [
            ("yt.solver.core.min.js", "yt_dlp_ejs/yt/solver/core.min.js"),
            ("yt.solver.lib.min.js", "yt_dlp_ejs/yt/solver/lib.min.js"),
        ] {
            let p = dist.join(src);
            if p.exists() {
                zip.start_file(dst, opts).unwrap();
                zip.write_all(&std::fs::read(&p).unwrap()).unwrap();
            } else {
                eprintln!("cargo:warning=expected {src} in dist/ after hatch_build.py — not found");
            }
        }
    } else {
        eprintln!("cargo:warning=hatch_build.py failed; JS solver bundles not embedded (falling back to yt-dlp built-in vendored scripts)");
    }
}

/// Run `python vendor/yt-dlp-ejs/hatch_build.py` in `ejs_dir` to produce
/// `dist/yt.solver.{core,lib}.min.js`. Returns true on success.
fn run_hatch_build_py(workspace_root: &PathBuf, ejs_dir: &PathBuf) -> bool {
    let python = find_python(workspace_root);
    match std::process::Command::new(&python)
        .arg(ejs_dir.join("hatch_build.py"))
        .current_dir(ejs_dir)
        .status()
    {
        Ok(s) if s.success() => true,
        other => {
            let reason = other
                .map(|s| s.to_string())
                .unwrap_or_else(|e| e.to_string());
            eprintln!("cargo:warning=hatch_build.py exited with {reason}");
            false
        }
    }
}

/// Locate the Python interpreter: prefer `PYO3_PYTHON` env var, fall back to
/// `uv python find 3.12`, then `python3`.
fn find_python(workspace_root: &PathBuf) -> String {
    if let Ok(p) = env::var("PYO3_PYTHON") {
        return p;
    }
    let uv = std::process::Command::new("uv")
        .args(["python", "find", "3.12"])
        .current_dir(workspace_root)
        .output();
    if let Ok(o) = uv {
        if o.status.success() {
            return String::from_utf8(o.stdout).unwrap_or_default().trim().to_owned();
        }
    }
    "python3".to_owned()
}
