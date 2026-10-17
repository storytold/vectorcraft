//! The user's own PDF presets, ICC profiles and plug-ins, kept in folders next to the preferences
//! so that they outlive the process (#2168, #2174, #2541). The apps decide where
//! ([`Session::attach_pdf_preset_store`], [`Session::attach_profile_store`],
//! [`Session::attach_plugin_store`]); a session nobody attached a folder to keeps nothing, which
//! is how the engine's own tests and embedders run.
//!
//! * **PDF presets**: one `.vcpdfpresets` file per preset (the format `pdf.preset.export` writes,
//!   so a file can also be imported by hand), named after the preset. A session that attaches the
//!   folder replaces its saved presets with the ones in it; a folder that doesn't exist yet takes
//!   the presets the session has (the ones an older version kept in the preferences file).
//! * **ICC profiles**: the bytes of each profile `color.loadProfile` registered, as `.icc` files
//!   named after the profile, never the path they came from (a profile in a system folder may
//!   move or go away). Attaching the folder registers them again.
//! * **Plug-ins**: the module of each plug-in `plugin.install` installs, as a `.wasm` file named
//!   after its id (as Illustrator keeps plug-ins in its Plug-ins folder); `plugin.remove` deletes it.
//!   Attaching the folder installs them again. The Additional Plug-ins Folder preference is not
//!   copied: it is loaded from where it is.
//!
//! The files are untrusted input like any other: sizes and counts are capped, a file that does not
//! parse is skipped with a log record, and every write is atomic.

use std::io::Read;
use std::path::{Path, PathBuf};

use vectorcraft_pdf::{PdfPreset, builtin_preset};

use crate::Session;
use crate::cmd::pdfcmds::{PRESET_FORMAT, PresetFile};

/// Files read from one store folder, at most (the rest are ignored).
pub const MAX_STORE_FILES: usize = 1000;
/// The largest PDF presets file read (a preset is a few kilobytes).
pub const MAX_PRESET_FILE: u64 = 1 << 20;
/// The largest ICC profile file read or loaded (CMYK profiles with large tables are a few MB).
pub const MAX_PROFILE_FILE: u64 = 32 << 20;
/// The most stored ICC profile bytes read at startup, all files together (the rest are skipped).
pub const MAX_PROFILE_STORE: u64 = 256 << 20;
/// The extension of a stored ICC profile.
const PROFILE_EXT: &str = "icc";
/// The most stored plug-in bytes read at startup, all files together (the rest are skipped).
pub const MAX_PLUGIN_STORE: u64 = 512 << 20;
/// The extension of a stored plug-in.
const PLUGIN_EXT: &str = "wasm";

/// The file name (without extension) a stored item called `name` gets: its name made safe for any
/// file system, then a hash of the name, so that names that differ only in characters a file name
/// can't hold, or in case, can't share a file. Names equal in any case are one item.
pub(crate) fn file_stem(name: &str) -> String {
    let safe: String = name.trim().chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' }).take(48).collect();
    // FNV-1a over the lower-cased name: stable across versions and platforms, unlike `DefaultHasher`.
    let hash = name.trim().to_lowercase().bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{}-{hash:016x}", safe.trim())
}

/// The regular files of `dir` with the extension `ext` (any case), by file name, at most
/// [`MAX_STORE_FILES`]; none when the folder can't be read.
fn list_files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) && p.is_file())
        .collect();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    files.truncate(MAX_STORE_FILES);
    files
}

/// The content of `path` when it is at most `max` bytes; the file is never read past `max`.
pub(crate) fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = vec![];
    file.take(max + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err(too_large(max));
    }
    Ok(bytes)
}

/// What a file over its cap is told (a detail after the file's name, in the log or an error).
fn too_large(max: u64) -> String {
    format!("larger than {max} bytes")
}

/// Write `bytes` to `dir/file`, making the folder; nothing is written when the file has them
/// already. A failure is logged: remembering is best effort and never fails the command.
fn put(dir: &Path, file: &str, bytes: &[u8]) -> bool {
    let path = dir.join(file);
    if std::fs::read(&path).is_ok_and(|old| old == bytes) {
        return true;
    }
    let written = std::fs::create_dir_all(dir).and_then(|()| write_atomic(&path, bytes));
    if let Err(e) = &written {
        log::warn!("can't save {}: {e}", path.display());
    }
    written.is_ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    vectorcraft_format::write_atomic(path, bytes)
}

/// The browser has no folders to keep anything in (the apps there never attach a store).
#[cfg(target_arch = "wasm32")]
fn write_atomic(_: &Path, _: &[u8]) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no file system"))
}

/// Remove `dir/file`; a file that isn't there is what was wanted.
fn take(dir: &Path, file: &str) {
    let path = dir.join(file);
    if let Err(e) = std::fs::remove_file(&path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("can't remove {}: {e}", path.display());
    }
}

impl Session {
    /// Keep the PDF presets this session saves in `dir`, and take the ones already there as the
    /// saved presets (see the module docs). Call it once, after the preferences were restored.
    pub fn attach_pdf_preset_store(&mut self, dir: PathBuf) {
        if dir.is_dir() {
            self.prefs.pdf_presets = load_presets(&dir);
            self.pdf_preset_dir = Some(dir);
        } else {
            self.pdf_preset_dir = Some(dir);
            for preset in self.prefs.pdf_presets.clone() {
                self.store_pdf_preset(None, &preset);
            }
        }
    }

    /// Write the saved preset `preset` to the store, which held it as `old_name` when that is set
    /// (a rename). A rename writes the new file first and removes the old one only when that write
    /// succeeded: after a failed write the store still holds the preset under its old name. When
    /// the two names differ only in case, the old file is written and then renamed to the new name;
    /// when that rename fails, the old file gets its content back. Without a store, nothing happens.
    pub(crate) fn store_pdf_preset(&self, old_name: Option<&str>, preset: &PdfPreset) {
        let Some(dir) = &self.pdf_preset_dir else { return };
        let mut clean = preset.clone();
        clean.settings.clear_passwords();
        let file = format!("{}.{PRESET_FORMAT}", file_stem(&clean.name));
        let bytes = match serde_json::to_vec_pretty(&PresetFile::new(vec![clean])) {
            Ok(bytes) => bytes,
            Err(e) => {
                log::warn!("can't save the PDF preset `{}`: {e}", preset.name);
                return;
            }
        };
        match old_name.map(|o| format!("{}.{PRESET_FORMAT}", file_stem(o))).filter(|old| *old != file) {
            // Names equal in any case get file names that differ only in case. A file system that
            // ignores case holds them as one file, which removing the old name would delete.
            Some(old) if old.to_lowercase() == file.to_lowercase() => match read_capped(&dir.join(&old), MAX_PRESET_FILE) {
                Ok(before) => {
                    if put(dir, &old, &bytes)
                        && let Err(e) = std::fs::rename(dir.join(&old), dir.join(&file))
                    {
                        log::warn!("can't rename {} to {file}: {e}", dir.join(&old).display());
                        put(dir, &old, &before);
                    }
                }
                Err(_) => {
                    put(dir, &file, &bytes);
                }
            },
            Some(old) => {
                if put(dir, &file, &bytes) {
                    take(dir, &old);
                }
            }
            None => {
                put(dir, &file, &bytes);
            }
        }
    }

    /// Remove the preset `name` from the store.
    pub(crate) fn unstore_pdf_preset(&self, name: &str) {
        if let Some(dir) = &self.pdf_preset_dir {
            take(dir, &format!("{}.{PRESET_FORMAT}", file_stem(name)));
        }
    }

    /// Keep the ICC profiles this session loads in `dir`, and register the ones already there.
    /// Call it before anything that names a profile (restoring the preferences, the first command).
    pub fn attach_profile_store(&mut self, dir: PathBuf) {
        let mut left = MAX_PROFILE_STORE;
        for path in list_files(&dir, PROFILE_EXT) {
            if left == 0 {
                log::warn!("skipping the other stored colour profiles: they hold more than {MAX_PROFILE_STORE} bytes together");
                break;
            }
            let loaded = read_capped(&path, MAX_PROFILE_FILE.min(left)).and_then(|bytes| {
                left -= bytes.len() as u64;
                vectorcraft_color::cms::register_icc(&bytes, None).map_err(|e| e.to_string())
            });
            if let Err(e) = loaded {
                log::warn!("skipping the stored colour profile {}: {e}", path.display());
            }
        }
        self.profile_dir = Some(dir);
    }

    /// Write the profile `name` (its file's `bytes`) to the store; `false` when it didn't go there.
    pub(crate) fn store_profile(&self, name: &str, bytes: &[u8]) -> bool {
        self.profile_dir.as_deref().is_some_and(|dir| put(dir, &format!("{}.{PROFILE_EXT}", file_stem(name)), bytes))
    }

    /// Remove the profile `name` from the store.
    pub(crate) fn unstore_profile(&self, name: &str) {
        if let Some(dir) = &self.profile_dir {
            take(dir, &format!("{}.{PROFILE_EXT}", file_stem(name)));
        }
    }

    /// Keep the plug-ins this session installs in `dir`, and install the ones already there (a
    /// module that doesn't load is skipped with a log record).
    pub fn attach_plugin_store(&mut self, dir: PathBuf) {
        let limits = vectorcraft_plugins::Limits::default();
        let mut left = MAX_PLUGIN_STORE;
        for path in list_files(&dir, PLUGIN_EXT) {
            if left == 0 {
                log::warn!("skipping the other stored plug-ins: they hold more than {MAX_PLUGIN_STORE} bytes together");
                break;
            }
            let loaded = read_capped(&path, (limits.max_module_bytes as u64).min(left)).and_then(|bytes| {
                left -= bytes.len() as u64;
                vectorcraft_plugins::Plugin::load(&bytes, limits.clone()).map_err(|e| e.to_string())
            });
            match loaded {
                Ok(plugin) => {
                    vectorcraft_plugins::registry::install(plugin.with_source(path.display().to_string()));
                }
                Err(e) => log::warn!("skipping the stored plug-in {}: {e}", path.display()),
            }
        }
        self.plugin_dir = Some(dir);
    }

    /// Write the module `bytes` of the plug-in `id` to the store.
    pub(crate) fn store_plugin(&self, id: &str, bytes: &[u8]) {
        if let Some(dir) = &self.plugin_dir {
            put(dir, &format!("{}.{PLUGIN_EXT}", file_stem(id)), bytes);
        }
    }

    /// Remove the plug-in `id` from the store.
    pub(crate) fn unstore_plugin(&self, id: &str) {
        if let Some(dir) = &self.plugin_dir {
            take(dir, &format!("{}.{PLUGIN_EXT}", file_stem(id)));
        }
    }
}

/// The presets of the files in `dir`: valid ones only, no name twice (any case), none called like a
/// built-in preset, at most [`MAX_STORE_FILES`].
fn load_presets(dir: &Path) -> Vec<PdfPreset> {
    let mut presets: Vec<PdfPreset> = vec![];
    for path in list_files(dir, PRESET_FORMAT) {
        let file = read_capped(&path, MAX_PRESET_FILE).and_then(|bytes| serde_json::from_slice::<PresetFile>(&bytes).map_err(|e| e.to_string()));
        let file = match file {
            Ok(f) if f.format == PRESET_FORMAT => f,
            Ok(f) => {
                log::warn!("skipping {}: not a PDF presets file (format `{}`)", path.display(), f.format);
                continue;
            }
            Err(e) => {
                log::warn!("skipping the stored PDF preset {}: {e}", path.display());
                continue;
            }
        };
        for mut preset in file.presets {
            let name = preset.name.trim().to_string();
            let usable = !name.is_empty()
                && builtin_preset(&name).is_none()
                && !presets.iter().any(|q| q.name.eq_ignore_ascii_case(&name))
                && preset.settings.check_values().is_ok()
                && presets.len() < MAX_STORE_FILES;
            if usable {
                preset.name = name;
                preset.settings.clear_passwords();
                presets.push(preset);
            } else {
                log::warn!("skipping the PDF preset `{name}` of {}", path.display());
            }
        }
    }
    presets
}
