use craft::font_catalog::{Catalog, FontEnv, font_dirs};

use super::{Arc, Path, PathBuf};

/// A font file holding `tables` (tag, data), sorted by tag as table directories are.
#[cfg(any(test, not(target_arch = "wasm32")))]
pub(crate) fn sfnt_of(tables: &[(&[u8; 4], &[u8])]) -> Option<Vec<u8>> {
    let mut tables = tables.to_vec();
    tables.sort_by_key(|t| *t.0);
    let n = u16::try_from(tables.len()).ok()?;
    let mut font = Vec::new();
    font.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    for v in [n, 0, 0, 0] {
        font.extend_from_slice(&v.to_be_bytes());
    }
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        font.extend_from_slice(*tag);
        for v in [0, u32::try_from(offset).ok()?, u32::try_from(data.len()).ok()?] {
            font.extend_from_slice(&v.to_be_bytes());
        }
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        font.extend_from_slice(data);
        font.resize(font.len().next_multiple_of(4), 0);
    }
    Some(font)
}

/// Lists the font files the platform's font service knows, for [`set_platform_font_files`].
pub type PlatformFontFiles = fn() -> Vec<String>;

/// What lists the platform's font files, set by the app ([`set_platform_font_files`]).
#[cfg(not(target_arch = "wasm32"))]
static PLATFORM_FONT_FILES: std::sync::OnceLock<PlatformFontFiles> = std::sync::OnceLock::new();

/// Have [`system_font_dirs`] and [`FontDb::global`]'s scan add the font files `list` gives (full
/// paths), asked again by each scan: fonts the platform knows that its font service, which the
/// scan asks itself, does not list. The desktop app lists on Windows the files of the families GDI
/// lists that DirectWrite's system collection doesn't name (#1258). Only the first call counts.
pub fn set_platform_font_files(list: PlatformFontFiles) {
    #[cfg(not(target_arch = "wasm32"))]
    // A second call keeps the first lister, as documented.
    let _ = PLATFORM_FONT_FILES.set(list);
    #[cfg(target_arch = "wasm32")]
    let _ = list;
}

/// The most files [`platform_font_files`] takes from the lister.
#[cfg(not(target_arch = "wasm32"))]
const MAX_PLATFORM_FILES: usize = 1 << 16;

/// The files [`set_platform_font_files`]'s lister gives now: full paths only, each once, at most
/// [`MAX_PLATFORM_FILES`]. None before it is set, and none on wasm.
fn platform_font_files() -> Vec<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(list) = PLATFORM_FONT_FILES.get() {
        let mut files: Vec<PathBuf> = list().into_iter().take(MAX_PLATFORM_FILES).map(PathBuf::from).filter(|p| p.is_absolute()).collect();
        files.sort();
        files.dedup();
        return files;
    }
    Vec::new()
}

/// The catalog of [`FontDb::global`]: the platform's font folders and font service, the font
/// store, VectorCraft's own folders ([`app_dirs`]) and the files [`set_platform_font_files`] lists.
pub(super) fn system_catalog() -> Catalog {
    let catalog = Catalog::new(FontEnv::from_process()).with_lister(Arc::new(platform_font_files));
    catalog.set_app_dirs(app_dirs());
    catalog
}

/// The folders VectorCraft adds to the platform's: its own Fonts folder ([`set_app_font_dir`]),
/// then the user's ([`set_user_font_dirs`]).
fn app_dirs() -> Vec<PathBuf> {
    app_font_dir().map(Path::to_path_buf).into_iter().chain(user_font_dirs()).collect()
}

/// Hands [`app_dirs`] and [`confined_font_dirs`] to [`FontDb::global`]'s catalog when it has been
/// made; one made later reads them itself. Its next check for installed fonts
/// ([`FontDb::installed_fonts_changed`]) reports the change.
#[cfg(not(target_arch = "wasm32"))]
fn follow_app_dirs() {
    if let Some(db) = super::FontDb::global_if_made() {
        db.catalog.set_app_dirs(app_dirs());
        db.catalog.set_confined_dirs(confined_font_dirs());
    }
}

/// Folders the user adds to the font scan (Preferences › Type › Additional Fonts Folder, #683),
/// read with their subfolders after the platform's.
#[cfg(not(target_arch = "wasm32"))]
static USER_FONT_DIRS: std::sync::RwLock<Vec<PathBuf>> = std::sync::RwLock::new(Vec::new());

/// Have [`system_font_dirs`] also read `dirs` (#683), so the fonts in them are listed and used as
/// if installed. The next check for installed fonts ([`FontDb::installed_fonts_changed`]) then
/// reports the change, and a scan catalogs them.
pub fn set_user_font_dirs(dirs: Vec<PathBuf>) {
    set_user_font_dirs_confined(dirs, false);
}

/// [`set_user_font_dirs`]; `confined` when a caller confined to some folders (automation) chose
/// them: the scan then reads only what is inside them, not files or folders that links in them
/// lead to elsewhere, since the caller could plant such a link.
pub fn set_user_font_dirs_confined(dirs: Vec<PathBuf>, confined: bool) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        *USER_FONT_DIRS.write().unwrap_or_else(std::sync::PoisonError::into_inner) = dirs;
        USER_FONT_DIRS_CONFINED.store(confined, std::sync::atomic::Ordering::Relaxed);
        follow_app_dirs();
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (dirs, confined);
}

/// Whether [`USER_FONT_DIRS`] were chosen by a confined caller ([`set_user_font_dirs_confined`]).
#[cfg(not(target_arch = "wasm32"))]
static USER_FONT_DIRS_CONFINED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The folders, as found on disk, that the scan reads only the inside of
/// ([`set_user_font_dirs_confined`]).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn confined_font_dirs() -> Vec<PathBuf> {
    if !USER_FONT_DIRS_CONFINED.load(std::sync::atomic::Ordering::Relaxed) {
        return vec![];
    }
    user_font_dirs().iter().filter_map(|d| std::fs::canonicalize(d).ok()).collect()
}

/// The folders [`set_user_font_dirs`] added (none on wasm).
pub fn user_font_dirs() -> Vec<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    return USER_FONT_DIRS.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    #[cfg(target_arch = "wasm32")]
    Vec::new()
}

/// VectorCraft's own Fonts folder, set by the app ([`set_app_font_dir`]).
#[cfg(not(target_arch = "wasm32"))]
static APP_FONT_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Have [`system_font_dirs`] read `dir` with its subfolders: VectorCraft's own Fonts folder, which
/// `text.addFontFiles` copies font files into. The desktop app and `vectorcraft-cli` set it before
/// the first scan, as they install [`set_platform_font_files`]. A folder that doesn't exist yet is
/// read once it does. Only the first call counts. Ignored on wasm.
pub fn set_app_font_dir(dir: PathBuf) {
    // A second call keeps the first folder, as documented.
    #[cfg(not(target_arch = "wasm32"))]
    if APP_FONT_DIR.set(dir).is_ok() {
        follow_app_dirs();
    }
    #[cfg(target_arch = "wasm32")]
    let _ = dir;
}

/// The folder [`set_app_font_dir`] set: none before that, and none on wasm.
pub fn app_font_dir() -> Option<&'static Path> {
    #[cfg(not(target_arch = "wasm32"))]
    return APP_FONT_DIR.get().map(PathBuf::as_path);
    #[cfg(target_arch = "wasm32")]
    None
}

/// The platform's font folders (the system's and the user's, [`font_dirs`]), VectorCraft's own
/// Fonts folder ([`set_app_font_dir`]), the font files [`set_platform_font_files`] lists and the
/// folders the user added ([`set_user_font_dirs`]), scanned by [`FontDb::global`] with the files
/// the platform's font service and, on Windows, the registry list. None on wasm.
pub fn system_font_dirs() -> Vec<PathBuf> {
    if cfg!(target_arch = "wasm32") {
        return Vec::new();
    }
    let mut dirs = font_dirs(&FontEnv::from_process());
    dirs.extend(app_font_dir().map(Path::to_path_buf));
    dirs.extend(platform_font_files());
    dirs.extend(user_font_dirs());
    dirs
}
