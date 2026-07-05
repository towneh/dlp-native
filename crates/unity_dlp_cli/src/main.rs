use std::ffi::CString;

use unity_dlp::{
    unity_dlp_extract, unity_dlp_init, unity_dlp_last_error, unity_dlp_version,
    UNITY_DLP_ERR_BUF, UNITY_DLP_OK, UNITY_DLP_OK_DEGRADED,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(String::as_str) {
        Some("version") | None => {
            let ver = unsafe {
                std::ffi::CStr::from_ptr(unity_dlp_version())
                    .to_string_lossy()
                    .into_owned()
            };
            println!("{ver}");
        }
        Some("extract") => {
            let url = match args.get(2) {
                Some(u) => u,
                None => {
                    eprintln!("Usage: unity_dlp_cli extract <url>");
                    std::process::exit(1);
                }
            };

            // DLP_PYTHON_HOME / DLP_PACKAGES_PATH let callers point the CLI at a
            // local Python prefix and packages without rebuilding. DLP_PACKAGES_PATH
            // is a `\n`-delimited list (each entry a .zip or a directory); earlier
            // entries win sys.path resolution — e.g. a staged yt-dlp wheel followed
            // by the bundled zip that carries yt_dlp_ejs and unity_dlp_jsc.
            let python_home = std::env::var("DLP_PYTHON_HOME").unwrap_or_default();
            let packages_path = std::env::var("DLP_PACKAGES_PATH").unwrap_or_default();
            let home_c = CString::new(python_home).expect("DLP_PYTHON_HOME contains NUL");
            let pkgs_c = CString::new(packages_path).expect("DLP_PACKAGES_PATH contains NUL");

            let rc = unity_dlp_init(home_c.as_ptr(), pkgs_c.as_ptr());
            if rc != UNITY_DLP_OK && rc != UNITY_DLP_OK_DEGRADED {
                eprintln!("init failed: {rc}");
                std::process::exit(1);
            }
            if rc == UNITY_DLP_OK_DEGRADED {
                eprintln!("warning: init degraded — unity_dlp_jsc shim not registered; \
                           YouTube JS-challenge path unavailable");
            }

            let url_c = CString::new(url.as_str()).expect("url contains NUL byte");
            // 8 MB initial buffer — avoids a second network round-trip for large
            // YouTube responses. Double on ERR_BUF in case the response is unusually large.
            let mut buf = vec![0u8; 8 << 20];
            let mut out_len: i32 = 0;

            let mut rc = unity_dlp_extract(
                url_c.as_ptr(),
                std::ptr::null(),
                buf.as_mut_ptr(),
                buf.len() as i32,
                &mut out_len,
            );

            if rc == UNITY_DLP_ERR_BUF {
                buf.resize((out_len as usize).max(buf.len() * 2), 0);
                rc = unity_dlp_extract(
                    url_c.as_ptr(),
                    std::ptr::null(),
                    buf.as_mut_ptr(),
                    buf.len() as i32,
                    &mut out_len,
                );
            }

            if rc != UNITY_DLP_OK {
                let mut err_buf = vec![0u8; 4096];
                let mut err_len: i32 = 0;
                unity_dlp_last_error(
                    err_buf.as_mut_ptr(),
                    err_buf.len() as i32,
                    &mut err_len,
                );
                let err = std::str::from_utf8(&err_buf[..err_len as usize])
                    .unwrap_or("<invalid utf-8>");
                eprintln!("extract failed ({rc}): {err}");
                std::process::exit(1);
            }

            let json = std::str::from_utf8(&buf[..out_len as usize]).unwrap_or("{}");
            println!("{json}");
        }
        Some(cmd) => {
            eprintln!("Unknown command: {cmd}");
            eprintln!("Usage: unity_dlp_cli [version|extract <url>]");
            std::process::exit(1);
        }
    }
}
