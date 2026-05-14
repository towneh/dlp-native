use std::ffi::CString;

use unity_dlp::{
    unity_dlp_extract, unity_dlp_init, unity_dlp_last_error, unity_dlp_version,
    UNITY_DLP_ERR_BUF, UNITY_DLP_OK,
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

            let rc = unity_dlp_init();
            if rc != UNITY_DLP_OK {
                eprintln!("init failed: {rc}");
                std::process::exit(1);
            }

            let url_c = CString::new(url.as_str()).expect("url contains NUL byte");
            let mut buf = vec![0u8; 1 << 16];
            let mut out_len: i32 = 0;

            let mut rc = unity_dlp_extract(
                url_c.as_ptr(),
                std::ptr::null(),
                buf.as_mut_ptr(),
                buf.len() as i32,
                &mut out_len,
            );

            if rc == UNITY_DLP_ERR_BUF {
                buf.resize(out_len as usize, 0);
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
