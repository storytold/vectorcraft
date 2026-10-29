//! The thumbnail extension's executable (`VectorCraftThumbnail.appex`): Finder's thumbnails of
//! native files. A stub on systems other than macOS.
#![forbid(unsafe_code)]

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "macos")]
    {
        vectorcraft_quicklook::mac::run(vectorcraft_quicklook::mac::Extension::Thumbnail)
    }
    #[cfg(not(target_os = "macos"))]
    {
        vectorcraft_quicklook::not_macos("vectorcraft-ql-thumbnail")
    }
}
