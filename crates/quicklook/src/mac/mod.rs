//! The extensions' Objective-C side (macOS only): a `QLThumbnailProvider` subclass
//! ([`thumbnail::ThumbnailProvider`]) and a data-based `QLPreviewProvider` subclass
//! ([`preview::PreviewProvider`]), written with objc2, and the executable's entry point
//! ([`run`]), which hands control to Foundation's `NSExtensionMain` as an app extension's `main`
//! does.
//!
//! Defining Objective-C classes, calling framework functions and handing pointers to Quick Look's
//! completion blocks needs `unsafe` (as `apps/vectorcraft/src/open_documents.rs` does for AppKit):
//! hence this module's `unsafe_code` allowance. Every call into the read path runs under
//! [`guarded`], so a bug there becomes an error for Quick Look (Finder shows the document icon)
//! rather than a panic unwinding into Objective-C.
#![allow(unsafe_code)]

pub mod preview;
pub mod thumbnail;

use std::ffi::{CString, c_char, c_int};
use std::os::unix::ffi::OsStringExt as _;
use std::path::PathBuf;
use std::process::ExitCode;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread as _, ClassType as _};
use objc2_foundation::{NSDictionary, NSError, NSErrorUserInfoKey, NSLocalizedDescriptionKey, NSString, NSURL};

use crate::{QuickLookError, Result};

/// Which extension an executable is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extension {
    Thumbnail,
    Preview,
}

/// The `NSError` domain of the errors the extensions give Quick Look.
pub const ERROR_DOMAIN: &str = "ai.storyteller.vectorcraft.quicklook";

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    /// Foundation's entry point for app extensions: it reads the bundle's `NSExtension`
    /// dictionary, instantiates `NSExtensionPrincipalClass` and serves Quick Look's requests. It
    /// doesn't return while the extension runs.
    fn NSExtensionMain(argc: c_int, argv: *mut *mut c_char) -> c_int;
}

/// The extension executable's `main`: register the principal class (named in the bundle's
/// Info.plist), then run `NSExtensionMain` with the process's arguments.
pub fn run(which: Extension) -> ExitCode {
    // Registers the class with the Objective-C runtime, where NSExtensionMain looks it up by name.
    let _ = match which {
        Extension::Thumbnail => thumbnail::ThumbnailProvider::class(),
        Extension::Preview => preview::PreviewProvider::class(),
    };
    // An argument with a NUL byte can't be passed on: leave it out.
    let args: Vec<CString> = std::env::args_os().filter_map(|a| CString::new(a.into_vec()).ok()).collect();
    let mut argv: Vec<*mut c_char> = args.iter().map(|a| a.as_ptr().cast_mut()).collect();
    argv.push(std::ptr::null_mut());
    let argc = c_int::try_from(args.len()).unwrap_or(0);
    // SAFETY: argv holds `argc` NUL-terminated strings, then a null pointer, as C's `main` gets
    // them; `args` outlives the call.
    let code = unsafe { NSExtensionMain(argc, argv.as_mut_ptr()) };
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

/// `f`, with a panic (a bug) turned into an error.
pub(crate) fn guarded<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|p| {
        let why = p.downcast_ref::<&str>().map(|s| (*s).to_owned()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
        Err(QuickLookError::Render(format!("internal error: {why}")))
    })
}

/// The `NSError` Quick Look gets for `e`: its code and message ([`ERROR_DOMAIN`]).
pub(crate) fn ns_error(e: &QuickLookError) -> Retained<NSError> {
    let message: Retained<AnyObject> = Retained::into_super(Retained::into_super(NSString::from_str(&e.to_string())));
    // SAFETY: NSLocalizedDescriptionKey is a constant NSString Foundation exports.
    let key: &NSErrorUserInfoKey = unsafe { NSLocalizedDescriptionKey };
    let info = NSDictionary::<NSErrorUserInfoKey, AnyObject>::from_retained_objects(&[key], &[message]);
    // SAFETY: the user info maps an NSString key to an NSString, as NSError expects.
    unsafe { NSError::initWithDomain_code_userInfo(NSError::alloc(), &NSString::from_str(ERROR_DOMAIN), e.code(), Some(&info)) }
}

/// The path of a `file:` URL.
pub(crate) fn file_path(url: &NSURL) -> Result<PathBuf> {
    if !url.isFileURL() {
        return Err(QuickLookError::Read("not a file".into()));
    }
    url.path().map(|p| PathBuf::from(p.to_string())).ok_or_else(|| QuickLookError::Read("the file has no path".into()))
}
