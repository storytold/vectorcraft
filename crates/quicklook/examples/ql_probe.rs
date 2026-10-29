//! Test tool for the Quick Look workflow (`.github/workflows/quicklook-macos.yml`): ask Quick Look
//! for a file's thumbnail the way Finder does (QuickLookThumbnailing's `QLThumbnailGenerator`),
//! accepting only a thumbnail (not the file's icon), and save it as a PNG. Not shipped.
//!
//! `cargo run --release -p vectorcraft-quicklook --example ql_probe -- FILE OUT.png [SIZE] [SECONDS]`
//!
//! Exit status: 0 a thumbnail was saved, 1 Quick Look answered with an error (printed), 124 no
//! answer within SECONDS (default 60), 2 bad arguments. `qlmanage -t` doesn't always reach
//! app-extension thumbnailers on current macOS; this does.
// A test tool: Objective-C calls need `unsafe`.
#![allow(unsafe_code)]

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "macos")]
    {
        mac::main()
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("ql_probe asks macOS's Quick Look for thumbnails: macOS only");
        std::process::ExitCode::from(2)
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::process::ExitCode;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use block2::RcBlock;
    use objc2::AnyThread as _;
    use objc2_core_foundation::CGSize;
    use objc2_foundation::{NSError, NSURL};
    use objc2_quick_look_thumbnailing::{QLThumbnailGenerationRequest, QLThumbnailGenerationRequestRepresentationTypes, QLThumbnailGenerator};
    use objc2_uniform_type_identifiers::UTTypePNG;

    pub fn main() -> ExitCode {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let (Some(file), Some(out)) = (args.first(), args.get(1)) else {
            eprintln!("usage: ql_probe FILE OUT.png [SIZE] [SECONDS]");
            return ExitCode::from(2);
        };
        let size: f64 = args.get(2).and_then(|s| s.parse().ok()).filter(|s: &f64| s.is_finite() && *s >= 1.0).unwrap_or(512.0);
        let secs: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(60);
        let (Some(url), Some(out_url)) = (NSURL::from_file_path(file), NSURL::from_file_path(out)) else {
            eprintln!("ql_probe: not a path: {file} or {out}");
            return ExitCode::from(2);
        };
        let _ = std::fs::remove_file(out);
        // SAFETY: plain object creation; the request asks for a thumbnail only (no icon).
        let request = unsafe {
            QLThumbnailGenerationRequest::initWithFileAtURL_size_scale_representationTypes(
                QLThumbnailGenerationRequest::alloc(),
                &url,
                CGSize::new(size, size),
                1.0,
                QLThumbnailGenerationRequestRepresentationTypes::Thumbnail,
            )
        };
        let (tx, rx) = mpsc::channel::<Option<String>>();
        let done = RcBlock::new(move |error: *mut NSError| {
            // SAFETY: Quick Look passes a valid NSError or null.
            let why = unsafe { error.as_ref() }.map(|e| format!("{} {}: {}", e.domain(), e.code(), e.localizedDescription()));
            // The waiting side may have given up.
            let _ = tx.send(why);
        });
        let start = Instant::now();
        // SAFETY: the generator copies what it needs; the block lives until it is called.
        unsafe {
            QLThumbnailGenerator::sharedGenerator()
                .saveBestRepresentationForRequest_toFileAtURL_asContentType_completionHandler(&request, &out_url, UTTypePNG, &done)
        };
        match rx.recv_timeout(Duration::from_secs(secs)) {
            Ok(None) => {
                println!("ql_probe: {file}: thumbnail saved to {out} in {:.2} s", start.elapsed().as_secs_f64());
                ExitCode::SUCCESS
            }
            Ok(Some(why)) => {
                println!("ql_probe: {file}: no thumbnail ({why}) after {:.2} s", start.elapsed().as_secs_f64());
                ExitCode::from(1)
            }
            Err(_) => {
                println!("ql_probe: {file}: no answer within {secs} s");
                ExitCode::from(124)
            }
        }
    }
}
