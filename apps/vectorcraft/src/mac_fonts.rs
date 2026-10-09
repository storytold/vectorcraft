//! macOS: the font files CoreText's font manager lists, scanned with the font folders
//! (`vectorcraft_text::set_platform_font_files`). The desktop app and `vectorcraft-cli` (exports,
//! MCP) both install it; the CLI shares this file.
//!
//! Apps and font managers can register fonts with the font manager from files in any folder,
//! outside the font folders
//! (<https://developer.apple.com/documentation/coretext/ctfontmanagerregisterfontsforurl(_:_:_:)>).
//! The font manager's list of available fonts holds them
//! (<https://developer.apple.com/documentation/coretext/ctfontmanagercopyavailablefonturls()>), so
//! VectorCraft lists them as it does on Windows (`system_fonts.rs`), reading the files in place
//! (#579).
//!
//! CoreText is a C API with no safe binding, hence the scoped `unsafe_code` allowance.
#![allow(unsafe_code)]

use std::collections::BTreeSet;

use objc2_core_foundation::{CFType, CFURL};
use objc2_core_text::CTFontManagerCopyAvailableFontURLs;

/// The most fonts read from the list (a system has a few thousand faces).
const MAX_FONTS: usize = 1 << 16;

/// Have the font scans read the font files CoreText lists.
pub fn install() {
    vectorcraft_text::set_platform_font_files(font_files);
}

/// The files of the fonts available to the app as CoreText lists them now (fonts registered since
/// the last call included).
fn font_files() -> Vec<String> {
    // SAFETY: a CoreText call without arguments; it returns a new array.
    let list = unsafe { CTFontManagerCopyAvailableFontURLs() };
    // SAFETY: the array holds CF objects (CFURLs, as CoreText documents); each one is checked to be
    // a CFURL before it is read.
    let fonts = unsafe { list.cast_unchecked::<CFType>() };
    // Faces of one file (a collection's) name it once.
    let files: BTreeSet<String> = fonts
        .iter()
        .take(MAX_FONTS)
        .filter_map(|f| f.downcast::<CFURL>().ok())
        // Only file URLs, as in `open_documents.rs` (#433).
        .filter(|u| u.scheme().is_some_and(|s| s.to_string().eq_ignore_ascii_case("file")))
        .filter_map(|u| u.to_file_path()?.into_os_string().into_string().ok())
        .collect();
    files.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::CFURL;
    use objc2_core_text::{CTFontManagerRegisterFontsForURL, CTFontManagerScope, CTFontManagerUnregisterFontsForURL};

    /// CoreText lists the installed fonts, by their files' full paths.
    #[test]
    fn coretext_lists_the_installed_font_files() {
        let files = super::font_files();
        assert!(files.iter().all(|f| std::path::Path::new(f).is_absolute()), "{:?}", files.first());
        assert!(files.iter().any(|f| f.starts_with("/System/Library/Fonts/")), "{} files, {:?}", files.len(), files.first());
    }

    /// A font registered with CoreText from another folder, as apps and font managers register
    /// fonts, is listed while it is registered.
    #[test]
    fn fonts_registered_from_other_folders_are_listed() {
        let dir = std::env::temp_dir().join(format!("vectorcraft-mac-fonts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A space in the name: the URL's escapes are decoded. The bundled serif isn't a font macOS
        // installs.
        let file = dir.join("Registered Font.ttf");
        std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/SourceSerif4-Regular.ttf"), &file).unwrap();
        let want = std::fs::canonicalize(&file).unwrap();
        let listed = || super::font_files().iter().any(|f| f.ends_with("/Registered Font.ttf") && std::fs::canonicalize(f).is_ok_and(|p| p == want));
        assert!(!listed());
        let url = CFURL::from_file_path(&file).unwrap();
        // SAFETY: a file URL, a scope constant and no error out-parameter.
        assert!(unsafe { CTFontManagerRegisterFontsForURL(&url, CTFontManagerScope::Process, std::ptr::null_mut()) });
        let registered = listed();
        // SAFETY: as above.
        unsafe { CTFontManagerUnregisterFontsForURL(&url, CTFontManagerScope::Process, std::ptr::null_mut()) };
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(registered);
    }
}
