//! VectorCraft's macOS Quick Look extensions (#663): the executables of the thumbnail and preview
//! app extensions inside `VectorCraft.app/Contents/PlugIns` (packaging/macos/quicklook).
//!
//! What a file shows is decided by `vectorcraft-quicklook` (`crates/quicklook`), the safe read
//! path, which forbids `unsafe`. This crate is only the Objective-C side ([`mac`], macOS only):
//! the `QLThumbnailProvider` and `QLPreviewProvider` subclasses and the extension's entry point.
//! Those need `unsafe`, so they live here, among the apps, with the allowance scoped to that
//! module as `apps/vectorcraft` scopes its AppKit code. On other systems each executable is a
//! stub that says so and exits.
#![deny(unsafe_code)]

#[cfg(target_os = "macos")]
pub mod mac;

/// What an extension's executable does where it can't run: says so and fails.
pub fn not_macos(name: &str) -> std::process::ExitCode {
    eprintln!("{name} is a macOS Quick Look extension: it runs inside VectorCraft.app on macOS only");
    std::process::ExitCode::FAILURE
}
