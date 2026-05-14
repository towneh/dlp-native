use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Emit the C header via cbindgen.
    let config = cbindgen::Config::from_file(
        PathBuf::from(&crate_dir)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("crates/unity_dlp_capi/cbindgen.toml"),
    )
    .expect("cbindgen.toml not found");

    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .expect("cbindgen failed")
        .write_to_file(out_dir.join("unity_dlp.h"));

    // Also write header alongside the library for easy consumption.
    let header_out = PathBuf::from(&crate_dir).join("../../unity_dlp.h");
    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(
            cbindgen::Config::from_file(
                PathBuf::from(&crate_dir)
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("crates/unity_dlp_capi/cbindgen.toml"),
            )
            .unwrap(),
        )
        .generate()
        .expect("cbindgen failed")
        .write_to_file(header_out);

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/ffi.rs");
}
