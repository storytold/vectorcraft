//! Windows: the font files in DirectWrite's system font collection and those of the fonts GDI has
//! loaded, scanned with the font folders (`vectorcraft_text::set_platform_font_files`). The desktop
//! app and `vectorcraft-cli` (exports, MCP) both install it; the CLI shares this file.
//!
//! Font services such as Adobe Fonts load their fonts with GDI at run time, from files in their
//! own folders, without installing them: the font folders and the registry don't list them. Since
//! Windows 10 (version 1607) DirectWrite's system font collection is meant to hold them too
//! (<https://learn.microsoft.com/windows/win32/directwrite/what-s-new-in-directwrite-for-windows-8-consumer-preview>,
//! "Support for Adobe Typekit and other font-service clients"), so VectorCraft lists them as other
//! apps do, reading the files in place (#579). It doesn't always: on some machines the fonts
//! Creative Cloud loads are in GDI's font list (Word and the font dialogs show them) but not in
//! the collection (#1258). So the families GDI lists that the collection doesn't name are asked
//! for one by one: a GDI font selected into a device context gives the DirectWrite font face
//! GDI uses (`IDWriteGdiInterop::CreateFontFaceFromHdc`), and that face its files.
//!
//! DirectWrite is COM and GDI a C API, neither with a safe API, hence the scoped `unsafe_code`
//! allowance.
#![allow(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use windows::Win32::Foundation::LPARAM;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_PROPERTY_ID_WIN32_FAMILY_NAME, DWriteCreateFactory, IDWriteFactory3, IDWriteFontFile, IDWriteFontSet,
    IDWriteGdiInterop, IDWriteLocalFontFileLoader, IDWriteStringList,
};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateFontIndirectW, DEFAULT_CHARSET, DeleteDC, DeleteObject, EnumFontFamiliesExW, HDC, HGDIOBJ, LOGFONTW, RASTER_FONTTYPE,
    SelectObject, TEXTMETRICW,
};
use windows::core::Interface;

/// The most fonts read from the collection (a system has a few thousand).
const MAX_FONTS: u32 = 1 << 16;
/// The most families and faces taken from GDI's font list.
const MAX_GDI_FONTS: usize = 1 << 16;
/// The most files one font face is made of (one, but for a few Type 1 fonts).
const MAX_FACE_FILES: u32 = 8;
/// The longest file path taken, in UTF-16 units (Windows' own limit).
const MAX_PATH: u32 = 32_767;

/// Have the font scans read the font files DirectWrite and GDI list.
pub fn install() {
    vectorcraft_text::set_platform_font_files(font_files);
}

/// The local files of the fonts in DirectWrite's system font collection as it is now (fonts
/// loaded since the last call included), with those of the fonts GDI lists that the collection
/// leaves out. None before Windows 10, or when DirectWrite fails: the font folders and the
/// registry still list the installed fonts.
fn font_files() -> Vec<String> {
    match listed_files() {
        Ok(files) => files.into_iter().collect(),
        Err(e) => {
            log::debug!("DirectWrite's system fonts aren't listed: {e}");
            Vec::new()
        }
    }
}

fn listed_files() -> windows::core::Result<BTreeSet<String>> {
    // SAFETY: plain DirectWrite calls on interfaces it returned, each checked for failure; the
    // collection is asked to check for fonts loaded since the factory last looked.
    let (factory, set) = unsafe {
        let factory: IDWriteFactory3 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let mut collection = None;
        factory.GetSystemFontCollection(false, &mut collection, true)?;
        (factory, collection.map(|c| c.GetFontSet()).transpose()?)
    };
    let mut files = set.as_ref().map(collection_files).unwrap_or_default();
    // GDI's own names for the collection's families, the names GDI lists them by.
    // SAFETY: a list DirectWrite returns, read by `strings`.
    let named = set.as_ref().and_then(|s| unsafe { s.GetPropertyValues(DWRITE_FONT_PROPERTY_ID_WIN32_FAMILY_NAME) }.ok());
    let named: BTreeSet<String> = named.as_ref().map(strings).unwrap_or_default().iter().map(|n| n.to_lowercase()).collect();
    match gdi_files(&factory, &named) {
        Ok(more) => files.extend(more),
        Err(e) => log::debug!("GDI's fonts aren't listed: {e}"),
    }
    Ok(files)
}

fn collection_files(set: &IDWriteFontSet) -> BTreeSet<String> {
    // SAFETY: a count of the set's fonts.
    let count = unsafe { set.GetFontCount() }.min(MAX_FONTS);
    // Faces of one file (a collection's, a variable font's instances) name it once.
    (0..count).filter_map(|i| local_path(set, i)).collect()
}

/// The path of the file holding font `i` of `set`, when it is a local file (not a downloadable
/// font, nor one in memory).
fn local_path(set: &IDWriteFontSet, i: u32) -> Option<String> {
    // SAFETY: `i` is below the set's count.
    let file = unsafe { set.GetFontFaceReference(i).ok()?.GetFontFile().ok()? };
    file_path(&file)
}

/// The path of `file`, when it is a local file.
fn file_path(file: &IDWriteFontFile) -> Option<String> {
    // SAFETY: the key DirectWrite hands back stays valid while `file` (which owns it) lives, and is
    // passed back with its own size; the path buffer is sized from DirectWrite's answer plus the
    // terminating NUL.
    unsafe {
        let loader: IDWriteLocalFontFileLoader = file.GetLoader().ok()?.cast().ok()?;
        let (mut key, mut size) = (std::ptr::null_mut(), 0u32);
        file.GetReferenceKey(&mut key, &mut size).ok()?;
        let len = loader.GetFilePathLengthFromKey(key, size).ok()?;
        if len == 0 || len > MAX_PATH {
            return None;
        }
        let mut path = vec![0u16; len as usize + 1];
        loader.GetFilePathFromKey(key, size, &mut path).ok()?;
        Some(String::from_utf16_lossy(path.get(..len as usize)?))
    }
}

/// The strings of `list`.
fn strings(list: &IDWriteStringList) -> Vec<String> {
    // SAFETY: indices below the list's count; each buffer is sized from DirectWrite's answer plus
    // the terminating NUL.
    let count = unsafe { list.GetCount() }.min(MAX_FONTS);
    (0..count)
        .filter_map(|i| unsafe {
            let len = list.GetStringLength(i).ok()?;
            if len > MAX_PATH {
                return None;
            }
            let mut s = vec![0u16; len as usize + 1];
            list.GetString(i, &mut s).ok()?;
            Some(String::from_utf16_lossy(s.get(..len as usize)?))
        })
        .collect()
}

/// A memory device context, deleted when dropped.
struct MemoryDc(HDC);

impl MemoryDc {
    fn new() -> windows::core::Result<Self> {
        // SAFETY: a device context compatible with the screen, owned by the guard.
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() { Err(windows::core::Error::from_thread()) } else { Ok(Self(dc)) }
    }
}

impl Drop for MemoryDc {
    fn drop(&mut self) {
        // SAFETY: the guard's own device context, with its first font back in it.
        // Failing to delete it leaks a handle, which there's nothing to do about.
        let _ = unsafe { DeleteDC(self.0) };
    }
}

/// The files of the faces of the font families GDI lists that DirectWrite's collection doesn't
/// name (`named`, lower case).
fn gdi_files(factory: &IDWriteFactory3, named: &BTreeSet<String>) -> windows::core::Result<BTreeSet<String>> {
    // SAFETY: a DirectWrite call checked for failure.
    let interop = unsafe { factory.GetGdiInterop()? };
    let dc = MemoryDc::new()?;
    let mut families = BTreeSet::new();
    enum_fonts(&dc, [0; 32], add_family, &mut families);
    let mut faces = Vec::new();
    for family in families.iter().filter(|f| !named.contains(&face_name(f).to_lowercase())) {
        enum_fonts(&dc, *family, add_face, &mut faces);
    }
    // A face listed once for each of its character sets is read once.
    faces.sort_by_key(face_key);
    faces.dedup_by_key(|f| face_key(f));
    // Asking for a face's files takes about a third of a millisecond, and the fonts are looked at
    // every few seconds (#2576): the faces still listed keep the files they had while those exist.
    // A face no longer listed is forgotten, so one activated again is asked for anew, and one
    // whose files couldn't be read (GDI or DirectWrite failed, or it has no local file) is asked
    // again at the next look.
    let mut known = KNOWN_FACES.lock().unwrap_or_else(|e| e.into_inner());
    let mut now = BTreeMap::new();
    for f in &faces {
        let key = face_key(f);
        let files = match known.remove(&key) {
            Some(files) if !files.is_empty() && files.iter().all(|p| std::path::Path::new(p).is_file()) => files,
            _ => face_files(&interop, &dc, f),
        };
        now.insert(key, files);
    }
    *known = now;
    Ok(known.values().flatten().cloned().collect())
}

/// A GDI face: its name (NUL-padded), weight and whether it's italic.
type FaceKey = ([u16; 32], i32, u8);

fn face_key(f: &LOGFONTW) -> FaceKey {
    (f.lfFaceName, f.lfWeight, f.lfItalic)
}

/// The files of the faces GDI listed at the last look ([`gdi_files`]).
static KNOWN_FACES: Mutex<BTreeMap<FaceKey, Vec<String>>> = Mutex::new(BTreeMap::new());

/// Have GDI call `add` with `out` for each font of the family named `family` (NUL-padded), or for
/// one font of each family when it's empty.
fn enum_fonts<T>(
    dc: &MemoryDc,
    family: [u16; 32],
    add: unsafe extern "system" fn(*const LOGFONTW, *const TEXTMETRICW, u32, LPARAM) -> i32,
    out: &mut T,
) {
    let query = LOGFONTW { lfCharSet: DEFAULT_CHARSET, lfFaceName: family, ..Default::default() };
    // SAFETY: `add` reads the fonts GDI passes and casts the LPARAM back to the `T` it is, which
    // outlives the call (GDI calls back before returning). Its answer is the last callback's.
    let _ = unsafe { EnumFontFamiliesExW(dc.0, &query, Some(add), LPARAM(std::ptr::from_mut(out) as isize), 0) };
}

/// A GDI font list callback adding the font's family name to the `BTreeSet<[u16; 32]>` in `out`.
unsafe extern "system" fn add_family(font: *const LOGFONTW, _: *const TEXTMETRICW, kind: u32, out: LPARAM) -> i32 {
    // SAFETY: GDI passes a valid font, and `out` is the set `enum_fonts` was given.
    let (Some(font), Some(out)) = (unsafe { font.as_ref() }, unsafe { (out.0 as *mut BTreeSet<[u16; 32]>).as_mut() }) else {
        return 0;
    };
    // Bitmap fonts have no file DirectWrite reads; `@` names are vertical writing's copies.
    if kind & RASTER_FONTTYPE == 0 && font.lfFaceName.first().is_some_and(|&c| c != 0 && c != u16::from(b'@')) {
        out.insert(font.lfFaceName);
    }
    i32::from(out.len() < MAX_GDI_FONTS)
}

/// A GDI font list callback adding the font to the `Vec<LOGFONTW>` in `out`.
unsafe extern "system" fn add_face(font: *const LOGFONTW, _: *const TEXTMETRICW, kind: u32, out: LPARAM) -> i32 {
    // SAFETY: GDI passes a valid font, and `out` is the list `enum_fonts` was given.
    let (Some(font), Some(out)) = (unsafe { font.as_ref() }, unsafe { (out.0 as *mut Vec<LOGFONTW>).as_mut() }) else {
        return 0;
    };
    if kind & RASTER_FONTTYPE == 0 {
        out.push(*font);
    }
    i32::from(out.len() < MAX_GDI_FONTS)
}

/// `name`, NUL-padded, as a string.
fn face_name(name: &[u16; 32]) -> String {
    String::from_utf16_lossy(name.split(|&c| c == 0).next().unwrap_or_default())
}

/// The local files of the DirectWrite font face GDI uses for `font`.
fn face_files(interop: &IDWriteGdiInterop, dc: &MemoryDc, font: &LOGFONTW) -> Vec<String> {
    // SAFETY: the font is selected into the guard's device context only while DirectWrite reads
    // it, then the context's own font goes back and the font is deleted; the files array is
    // sized from DirectWrite's count.
    unsafe {
        let gdi_font = CreateFontIndirectW(font);
        if gdi_font.is_invalid() {
            return Vec::new();
        }
        let first = SelectObject(dc.0, HGDIOBJ::from(gdi_font));
        let face = interop.CreateFontFaceFromHdc(dc.0);
        SelectObject(dc.0, first);
        // Failing to delete it leaks a handle, which there's nothing to do about.
        let _ = DeleteObject(HGDIOBJ::from(gdi_font));
        let Ok(face) = face else { return Vec::new() };
        let mut count = 0;
        if face.GetFiles(&mut count, None).is_err() || count == 0 || count > MAX_FACE_FILES {
            return Vec::new();
        }
        let mut files = vec![None; count as usize];
        if face.GetFiles(&mut count, Some(files.as_mut_ptr())).is_err() {
            return Vec::new();
        }
        files.iter().flatten().filter_map(file_path).collect()
    }
}

#[cfg(test)]
mod tests {
    /// DirectWrite lists the installed fonts, by their files' full paths.
    #[test]
    fn directwrite_lists_the_installed_font_files() {
        let files = super::font_files();
        assert!(files.iter().all(|f| std::path::Path::new(f).is_absolute()), "{:?}", files.first());
        let fonts = std::path::Path::new(&std::env::var("WINDIR").unwrap()).join("Fonts").to_string_lossy().to_lowercase();
        assert!(files.iter().any(|f| f.to_lowercase().starts_with(&fonts)), "{} files, {:?}", files.len(), files.first());
    }

    /// A font loaded with GDI from a file without an extension outside the font folders, as
    /// Creative Cloud loads Adobe Fonts, is listed by its file even though DirectWrite's collection
    /// doesn't hold it (#1258): fonts loaded for one process never are.
    #[test]
    fn fonts_only_gdi_has_loaded_are_listed_by_their_files() {
        use windows::Win32::Graphics::Gdi::{AddFontResourceExW, FR_PRIVATE, RemoveFontResourceExW};
        let dir = std::env::temp_dir().join(format!("vc-gdi-fonts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("29457");
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/SourceSans3-Regular.ttf");
        let mut data = std::fs::read(source).unwrap();
        // Renamed (to a name as long) so no installed Source Sans 3 stands in for it.
        let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
        for (from, to) in [(utf16("Source Sans 3"), utf16("GdiOnly Sans3")), (b"Source Sans 3".to_vec(), b"GdiOnly Sans3".to_vec())] {
            let mut i = 0;
            while let Some(at) = data[i..].windows(from.len()).position(|w| w == from) {
                data[i + at..i + at + to.len()].copy_from_slice(&to);
                i += at + to.len();
            }
        }
        std::fs::write(&file, data).unwrap();
        let path = windows::core::HSTRING::from(file.as_os_str());
        assert_eq!(unsafe { AddFontResourceExW(&path, FR_PRIVATE, None) }, 1);
        let files = super::font_files();
        let _ = unsafe { RemoveFontResourceExW(&path, FR_PRIVATE.0, None) };
        let _ = std::fs::remove_dir_all(&dir);
        let wanted = file.to_string_lossy().to_lowercase();
        assert!(files.iter().any(|f| f.to_lowercase() == wanted), "{wanted} not in {} files", files.len());
    }
}
