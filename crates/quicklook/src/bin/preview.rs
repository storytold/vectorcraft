//! The preview extension's executable (`VectorCraftPreview.appex`): the Space-bar preview of native
//! files. A stub on systems other than macOS.
#![forbid(unsafe_code)]

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "macos")]
    {
        vectorcraft_quicklook::mac::run(vectorcraft_quicklook::mac::Extension::Preview)
    }
    #[cfg(not(target_os = "macos"))]
    {
        vectorcraft_quicklook::not_macos("vectorcraft-ql-preview")
    }
}
