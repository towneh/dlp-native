use std::env;
use std::io::Write as _;
use std::path::{Path, PathBuf};

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
        workspace_root
            .join(".git/modules/vendor/yt-dlp/HEAD")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root
            .join(".git/modules/vendor/yt-dlp-ejs/HEAD")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root.join("python/unity_dlp_jsc").display()
    );
}

// ── cbindgen C header ─────────────────────────────────────────────────────────

fn generate_header(crate_dir: &str, workspace_root: &Path, out_dir: &Path) {
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
    // Entries are stamped with the date of the sources rather than of the build. This
    // archive is committed, and stamping it with "now" gives identical sources different
    // bytes every time, so a rebuild or a fetch of the CI artifacts always reads as a
    // change to a 9 MB binary and a real one cannot be told from the noise.
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .last_modified_time(vendored_source_date(workspace_root));

    // A bundle missing any of the three packages produces an interpreter that
    // aborts (or degrades) at init, so an absent source is a build failure, not a
    // warning that ships a broken artifact.

    // ── yt_dlp ────────────────────────────────────────────────────────────────
    let yt_dlp_dir = workspace_root.join("vendor/yt-dlp/yt_dlp");
    assert!(
        yt_dlp_dir.exists(),
        "vendor/yt-dlp/yt_dlp not found — run `git submodule update --init` before building"
    );
    add_python_package(&mut zip, &yt_dlp_dir, "yt_dlp", opts);

    // ── yt_dlp_ejs ────────────────────────────────────────────────────────────
    add_yt_dlp_ejs(&mut zip, workspace_root, opts);

    // ── unity_dlp_jsc ─────────────────────────────────────────────────────────
    let jsc_dir = workspace_root.join("python/unity_dlp_jsc/unity_dlp_jsc");
    assert!(
        jsc_dir.exists(),
        "python/unity_dlp_jsc not found — the YouTube JCP shim is required in the bundle"
    );
    add_python_package(&mut zip, &jsc_dir, "unity_dlp_jsc", opts);

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

/// The commit date of the newest vendored source, used to stamp the bundle's entries.
///
/// Reads as a real date — the day the newest thing in the bundle was written — while
/// staying identical across machines and rebuilds, because a commit carries its date
/// and timezone in the object rather than taking the reader's.
///
/// Only the submodules are consulted. Taking this repo's HEAD instead would move the
/// stamp on every commit made here, which is the churn this exists to remove.
fn vendored_source_date(workspace_root: &Path) -> zip::DateTime {
    let newest = ["vendor/yt-dlp", "vendor/yt-dlp-ejs"]
        .iter()
        .map(|relative| {
            let dir = workspace_root.join(relative);
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(["log", "-1", "--format=%cd", "--date=format:%Y %m %d %H %M %S"])
                .output()
                .unwrap_or_else(|e| panic!("could not run git to date {relative}: {e}"));
            assert!(
                out.status.success(),
                "git could not read a commit date from {relative}. A submodule that has \
                 not been initialised has no date to read — run `git submodule update --init`."
            );
            let stdout = String::from_utf8_lossy(&out.stdout);
            let parts: Vec<u16> = stdout
                .split_whitespace()
                .map(|f| f.parse().expect("git returned a non-numeric date field"))
                .collect();
            assert_eq!(parts.len(), 6, "unexpected git date output: {stdout:?}");
            (parts[0], parts[1], parts[2], parts[3], parts[4], parts[5])
        })
        .max()
        .expect("no vendored sources to date");

    // The zip format stores MS-DOS timestamps, which start in 1980. Nothing vendored
    // here is anywhere near that, so a date below it means the fields were misread.
    zip::DateTime::from_date_and_time(
        newest.0,
        newest.1 as u8,
        newest.2 as u8,
        newest.3 as u8,
        newest.4 as u8,
        newest.5 as u8,
    )
    .expect("vendored commit date is not representable in a zip entry")
}

/// The version hatch-vcs would generate for the pinned `yt-dlp-ejs`: its most
/// recent tag.
///
/// Fails the build rather than guessing. A wrong version here is worse than no
/// build, because the runtime update guard treats it as fact and a too-new value
/// stages an update the bundle cannot satisfy.
fn ejs_version_from_tag(ejs_dir: &Path) -> String {
    let git = |args: &[&str]| -> Option<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(ejs_dir)
            .args(args)
            .output()
            .expect("could not run git to read the yt-dlp-ejs version");
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };

    // Tags on the checked-out commit first. The submodule is pinned to a release
    // tag, and this reads it from a shallow clone, where `describe` cannot walk
    // back far enough to find anything. Taking the first release-shaped tag also
    // steps over any moving name that happens to point at the same commit.
    let pointed = git(&["tag", "--points-at", "HEAD"]).unwrap_or_default();
    let tag = pointed
        .lines()
        .map(str::trim)
        .find(|candidate| is_release_version(candidate.strip_prefix('v').unwrap_or(candidate)))
        .map(str::to_string)
        // Otherwise the nearest tag behind HEAD, which covers a pin made after a
        // release and needs history to resolve. Restricted to release-shaped tags:
        // an unfiltered describe returns whichever tag is nearest, so a moving one
        // in between would be picked and then rejected as a version, failing the
        // build even though a usable release tag sits further back.
        .or_else(|| {
            git(&[
                "describe",
                "--tags",
                "--abbrev=0",
                "--match=[0-9]*",
                "--match=v[0-9]*",
            ])
        });

    let tag = tag.unwrap_or_else(|| {
        panic!(
            "could not read a release tag from vendor/yt-dlp-ejs. A shallow submodule \
             checkout has no tags — fetch them with `git -C vendor/yt-dlp-ejs fetch --tags`, \
             or check out a release tag if it is on an untagged commit."
        )
    });

    assert!(!tag.is_empty(), "vendor/yt-dlp-ejs reported an empty tag");
    // hatch-vcs drops a leading `v`, so a `v0.8.0` tag means version 0.8.0.
    let version = tag.strip_prefix('v').unwrap_or(&tag).to_string();
    assert!(
        is_release_version(&version),
        "vendor/yt-dlp-ejs is on tag {version:?}, which is not a release version. \
         The value is written into _version.py, where yt-dlp reads it to decide \
         whether the bundled solver is supported and DlpUpdater reads it to decide \
         whether an update is safe to stage — both by parsing dot-separated numbers. \
         It therefore has to look like 0.8.0, not a moving name like `nightly` and \
         not a prerelease. Check out a release tag."
    );
    version
}

/// Whether a tag can stand in as the package version: dot-separated numeric
/// components and nothing else.
///
/// That is the shape every yt-dlp-ejs release tag has, and the shape both
/// readers of the value need. yt-dlp splits it on `[-.]` and reads the parts as
/// integers to decide whether the bundled solver is a version it supports, and
/// `DlpUpdater` parses it the same way before deciding whether an update is safe
/// to stage. Neither raises on a malformed value; they read it as a mismatch, so
/// a version like `1..0` would build and ship a bundle whose solver yt-dlp
/// declines to use and whose updates can never stage again. Restricting the tag
/// to digits and dots also leaves it unable to end the string literal it is
/// written into, which git's own ref rules would otherwise permit.
fn is_release_version(tag: &str) -> bool {
    tag.split('.')
        .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

fn add_yt_dlp_ejs(
    zip: &mut zip::ZipWriter<std::fs::File>,
    workspace_root: &PathBuf,
    opts: zip::write::SimpleFileOptions,
) {
    let ejs_dir = workspace_root.join("vendor/yt-dlp-ejs");
    assert!(
        ejs_dir.exists(),
        "vendor/yt-dlp-ejs not found — run `git submodule update --init` before building"
    );

    // Python sources
    add_python_package(zip, &ejs_dir.join("yt_dlp_ejs"), "yt_dlp_ejs", opts);

    // Synthetic _version.py (normally generated by hatch-vcs from the git tag).
    // Read the tag here for the same reason, so bumping the submodule cannot leave
    // this value behind: DlpUpdater compares it against a candidate yt-dlp's
    // yt-dlp-ejs requirement before staging an update, so a stale value decides
    // real staging outcomes rather than merely reporting the wrong number.
    let ejs_version = ejs_version_from_tag(&ejs_dir);
    zip.start_file("yt_dlp_ejs/_version.py", opts).unwrap();
    zip.write_all(format!("version = \"{ejs_version}\"\n").as_bytes())
        .unwrap();

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

/// Locate an interpreter to run `hatch_build.py` with: prefer the `PYO3_PYTHON`
/// env var, fall back to `uv python find --system 3.14+gil`, then `python3`.
///
/// This only builds the ejs JS bundles, so any working 3.x will do — it never
/// becomes the interpreter the extension links against. The request is still
/// constrained so it matches what the build scripts pick.
fn find_python(workspace_root: &PathBuf) -> String {
    if let Ok(p) = env::var("PYO3_PYTHON") {
        return p;
    }
    let uv = std::process::Command::new("uv")
        .args(["python", "find", "--system", "3.14+gil"])
        .current_dir(workspace_root)
        .output();
    if let Ok(o) = uv {
        if o.status.success() {
            return String::from_utf8(o.stdout)
                .unwrap_or_default()
                .trim()
                .to_owned();
        }
    }
    "python3".to_owned()
}
