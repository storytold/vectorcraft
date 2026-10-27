use super::*;

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

/// Have [`system_font_dirs`] add the font files `list` gives (full paths) outside the font
/// folders, asked again by each scan and each check for installed fonts
/// ([`FontDb::installed_fonts_changed`]). The desktop app lists DirectWrite's system font
/// collection on Windows: fonts a font service such as Adobe Fonts loads in place, from files
/// outside the font folders and unknown to the registry (#579). On macOS it lists the fonts
/// CoreText's font manager has available, which apps and font managers can register from any
/// folder. Only the first call counts.
pub fn set_platform_font_files(list: PlatformFontFiles) {
    #[cfg(not(target_arch = "wasm32"))]
    // A second call keeps the first lister, as documented.
    let _ = PLATFORM_FONT_FILES.set(list);
    #[cfg(target_arch = "wasm32")]
    let _ = list;
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
    #[cfg(not(target_arch = "wasm32"))]
    // A second call keeps the first folder, as documented.
    let _ = APP_FONT_DIR.set(dir);
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

/// The platform's font folders (the system's and the user's), VectorCraft's own Fonts folder
/// ([`set_app_font_dir`]), the font files known to the platform outside them (on Windows those
/// registered with it, and those [`set_platform_font_files`] lists) and the folders the user added
/// ([`set_user_font_dirs`]), scanned by [`FontDb::global`].
pub fn system_font_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(target_arch = "wasm32") {
        return dirs;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        dirs.extend(["/System/Library/Fonts", "/Library/Fonts", "/Network/Library/Fonts"].map(Into::into));
        if let Some(h) = &home {
            dirs.push(h.join("Library/Fonts"));
        }
        // Fonts macOS downloads on demand (Yu Gothic, Yu Mincho, Tsukushi Maru Gothic…): one
        // asset folder per font, under a catalog folder whose number changes with macOS.
        for e in std::fs::read_dir("/System/Library/AssetsV2").into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().starts_with("com_apple_MobileAsset_Font") {
                dirs.push(e.path());
            }
        }
    } else if cfg!(windows) {
        let root = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
        dirs.push(root.join("Fonts"));
        // Fonts installed for the current user only (what Install does without administrator rights).
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(l).join("Microsoft\\Windows\\Fonts"));
        }
        #[cfg(windows)]
        dirs.extend(registered_font_files(&dirs));
    } else {
        dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(Into::into));
        // The XDG data folders' `fonts` (fontconfig's user folder; Flatpak, Snap, NixOS and Guix
        // list their own data folders), relative paths ignored as the XDG spec says.
        let xdg = |var| std::env::var_os(var).map(|v| std::env::split_paths(&v).filter(|p| p.is_absolute()).collect::<Vec<_>>()).unwrap_or_default();
        let data_home = xdg("XDG_DATA_HOME").into_iter().next().or_else(|| home.as_ref().map(|h| h.join(".local/share")));
        dirs.extend(data_home.iter().cloned().chain(xdg("XDG_DATA_DIRS")).map(|d| d.join("fonts")));
        if let Some(h) = &home {
            dirs.push(h.join(".fonts"));
        }
        // In a Flatpak sandbox, the host's fonts (system, local and the user's).
        dirs.extend(["/run/host/fonts", "/run/host/local-fonts", "/run/host/user-fonts"].map(Into::into));
        // The folders fontconfig is set up with outside those: packages add their own (TeX fonts
        // under `/usr/share/texmf/fonts`, #1529), and `fc-list` and other apps show their fonts.
        let config_home = xdg("XDG_CONFIG_HOME").into_iter().next().or_else(|| home.as_ref().map(|h| h.join(".config")));
        let places = crate::fontconfig::Places { home: home.clone(), data_home, config_home };
        for dir in crate::fontconfig::font_dirs(&crate::fontconfig::root(), &places) {
            if !dirs.iter().any(|known| dir.starts_with(known)) {
                dirs.push(dir);
            }
        }
    }
    dirs.extend(app_font_dir().map(Path::to_path_buf));
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(list) = PLATFORM_FONT_FILES.get() {
        let files = fonts_outside(list(), &dirs);
        dirs.extend(files);
    }
    dirs.extend(user_font_dirs());
    dirs
}

/// The registry key listing the fonts installed on Windows (for all users under the local
/// machine's key, for the current user under theirs): one value per face, holding its file's
/// name in the Windows font folder or its full path.
#[cfg(windows)]
const REGISTERED_FONTS: &str = r"Software\Microsoft\Windows NT\CurrentVersion\Fonts";

/// The font files registered with Windows outside `font_dirs` (fonts installed as shortcuts to
/// where they are, or by programs into their own folders), which apps find by the registry.
#[cfg(windows)]
fn registered_font_files(font_dirs: &[PathBuf]) -> Vec<PathBuf> {
    use windows_registry::{CURRENT_USER, LOCAL_MACHINE};
    let values = [LOCAL_MACHINE, CURRENT_USER].into_iter().filter_map(|root| root.open(REGISTERED_FONTS).ok()).flat_map(|key| {
        // A value that isn't a string isn't a font file.
        key.values().map(|vs| vs.filter_map(|(_, v)| String::try_from(v).ok()).collect::<Vec<_>>()).unwrap_or_default()
    });
    fonts_outside(values, font_dirs)
}

/// The full paths among `registered` (font registrations' data) that aren't in `font_dirs`
/// (ignoring case, as Windows paths do), sorted and deduplicated. File names alone are files in
/// the Windows font folder, which is scanned anyway. The count is capped.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn fonts_outside(registered: impl IntoIterator<Item = String>, font_dirs: &[PathBuf]) -> Vec<PathBuf> {
    const MAX_REGISTERED: usize = 1 << 16;
    let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
    let dirs: Vec<PathBuf> = font_dirs.iter().map(|d| lower(d)).collect();
    let mut files: Vec<PathBuf> = registered
        .into_iter()
        .take(MAX_REGISTERED)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && !dirs.iter().any(|d| lower(p).starts_with(d)))
        .collect();
    files.sort();
    files.dedup();
    files
}
