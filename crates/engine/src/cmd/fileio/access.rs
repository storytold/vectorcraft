#[cfg(not(target_arch = "wasm32"))]
use super::write_atomic;
use super::{EngineError, Result, Session, Value, bad, json};

// ---------- the file system (none on the web, where commands take and return bytes) ----------
//
// Every path a command reads or writes goes through these, which check it against the automation
// roots in force ([`crate::file_access`]): a path outside them is an error (`file_stamp` and
// `file_created`: no file), as is any path once roots are in force without its kind of access.

/// `path` checked for reading against the automation roots in force.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn may_read(path: &str) -> Result<()> {
    #[cfg(test)]
    touch(path);
    crate::file_access::check_read(path).map_err(EngineError::Other)
}

/// Paths read while [`touched`] runs, on any thread (the SVG importer reads on one of its own).
#[cfg(test)]
static TOUCHED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
static RECORDING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
fn touch(path: &str) {
    if RECORDING.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        TOUCHED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(path.to_string());
    }
}

/// Run `f` and list the paths read through this module meanwhile (by any test running then).
#[cfg(test)]
pub(crate) fn touched<T>(f: impl FnOnce() -> T) -> (T, Vec<String>) {
    use std::sync::atomic::Ordering::SeqCst;
    let start = TOUCHED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).len();
    RECORDING.fetch_add(1, SeqCst);
    let out = f();
    RECORDING.fetch_sub(1, SeqCst);
    let seen = TOUCHED.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(start..).unwrap_or_default().to_vec();
    (out, seen)
}

/// `path` checked for writing against the automation roots in force.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn may_write(path: &str) -> Result<()> {
    crate::file_access::check_write(path).map_err(EngineError::Other)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_file(path: &str) -> Result<Vec<u8>> {
    may_read(path)?;
    // A device or pipe may never end: only a regular file is read to its end.
    if !std::fs::metadata(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?.is_file() {
        return Err(EngineError::Other(format!("{path}: not a file")));
    }
    std::fs::read(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))
}

/// The first `max` bytes of the file at `path` (all of it when it is shorter). Anything but a
/// regular file (a folder, a pipe, a device) is refused, also through a link.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_file_head(path: &str, max: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    may_read(path)?;
    if !std::fs::metadata(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?.is_file() {
        return Err(EngineError::Other(format!("{path}: not a file")));
    }
    let mut out = vec![];
    std::fs::File::open(path).and_then(|f| f.take(max as u64).read_to_end(&mut out)).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    Ok(out)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_file(path: &str, bytes: &[u8]) -> Result<()> {
    // Work that stops early for a cancel can leave a truncated result: none of it is written.
    vectorcraft_doc::cancel::check()?;
    may_write(path)?;
    // Never a half-written file: see [`write_atomic`].
    write_atomic(std::path::Path::new(path), bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))
}

/// [`write_file`] with what `fill` writes into the file (a save streamed into it).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_file_with(path: &str, fill: &mut dyn FnMut(&mut dyn std::io::Write) -> std::io::Result<()>) -> Result<()> {
    vectorcraft_doc::cancel::check()?;
    may_write(path)?;
    vectorcraft_format::write_atomic_with(std::path::Path::new(path), |f| fill(f)).map_err(|e| EngineError::Other(format!("{path}: {e}")))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn create_dir(path: &str) -> Result<()> {
    may_write(path)?;
    std::fs::create_dir_all(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))
}

/// Write `bytes` to a new file at `path`. A file already at `path` is an error and is never
/// replaced. When the write fails part way, the file it started is removed.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_new_file(path: &str, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    may_write(path)?;
    let err = |e: std::io::Error| EngineError::Other(format!("{path}: {e}"));
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(err)?;
    let written = f.write_all(bytes).and_then(|()| vectorcraft_format::sync_file(&f));
    drop(f);
    written.map_err(|e| {
        // Best effort: the partly written file is all there is to clean up.
        let _ = std::fs::remove_file(path);
        err(e)
    })
}

/// The highest number [`copy_new`] gives a copy's name (`Name 99.png`).
#[cfg(not(target_arch = "wasm32"))]
const MAX_COPY_NUMBER: u32 = 99;

/// Where [`copy_new`] put a file.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) enum Placed {
    /// Copied to this new file.
    Copied(std::path::PathBuf),
    /// Kept as this file, which holds the same contents.
    Kept(std::path::PathBuf),
}

/// Put a copy of the file `from` into the folder `dir`, never replacing a file there. It tries
/// `Name.ext`, then `Name 2.ext` up to `Name 99.ext`, and stops at the first name that is free,
/// where `create` makes the copy, or that is a file holding the same contents (`same`, given that
/// file and its metadata), which is kept. When `create` fails with `AlreadyExists` (the name was
/// taken meanwhile), it tries the next name. `None` when no name is free or holds the same
/// contents; a name whose metadata can't be read counts as taken.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn copy_new(
    from: &std::path::Path,
    dir: &std::path::Path,
    create: impl Fn(&std::path::Path) -> std::io::Result<()>,
    same: impl Fn(&std::path::Path, &std::fs::Metadata) -> bool,
) -> std::io::Result<Option<Placed>> {
    use std::io::ErrorKind;
    let (Some(name), Some(stem)) = (from.file_name(), from.file_stem()) else { return Err(ErrorKind::InvalidInput.into()) };
    let ext = from.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for n in 1..=MAX_COPY_NUMBER {
        let to = if n == 1 { dir.join(name) } else { dir.join(format!("{} {n}{ext}", stem.to_string_lossy())) };
        match std::fs::symlink_metadata(&to) {
            Err(e) if e.kind() == ErrorKind::NotFound => match create(&to) {
                Ok(()) => return Ok(Some(Placed::Copied(to))),
                // Taken meanwhile: the next name.
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            },
            Ok(m) if m.is_file() && same(&to, &m) => return Ok(Some(Placed::Kept(to))),
            _ => {}
        }
    }
    Ok(None)
}

/// The most entries `file.list` returns.
const LIST_MAX: usize = 5000;

pub(super) fn file_list(_: &mut Session, p: &Value) -> Result<Value> {
    let path = p.get("path").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| bad("file.list", "missing `path`"))?;
    let (entries, truncated) = list_dir(path, LIST_MAX)?;
    Ok(json!({ "path": absolute_path(path), "entries": entries, "truncated": truncated }))
}

/// The entries of folder `path`, sorted by name: at most `max`, and whether there were more. A
/// link isn't followed: its target may lie outside the automation roots.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn list_dir(path: &str, max: usize) -> Result<(Vec<Value>, bool)> {
    may_read(path)?;
    let dir = std::fs::read_dir(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let mut entries: Vec<(String, Value)> = vec![];
    let mut truncated = false;
    for e in dir.flatten() {
        if entries.len() == max {
            truncated = true;
            break;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(t) = e.file_type() else { continue };
        let entry = if t.is_symlink() {
            json!({ "name": name, "kind": "link" })
        } else if t.is_dir() {
            json!({ "name": name, "kind": "folder" })
        } else {
            json!({ "name": name, "kind": "file", "size": e.metadata().map(|m| m.len()).ok() })
        };
        entries.push((name, entry));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((entries.into_iter().map(|(_, v)| v).collect(), truncated))
}

#[cfg(target_arch = "wasm32")]
pub(super) fn list_dir(_: &str, _: usize) -> Result<(Vec<Value>, bool)> {
    Err(EngineError::Other("no file system here".into()))
}

/// A file's size (bytes) and modification time (ms since the Unix epoch, when the file system
/// keeps one); `None` when there is no file at `path` (or automation may not read it).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn file_stamp(path: &str) -> Option<(u64, Option<u64>)> {
    let m = file_metadata(path).filter(std::fs::Metadata::is_file)?;
    let modified = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64);
    Some((m.len(), modified))
}

/// When the file at `path` was created (ms since the Unix epoch), when the file system keeps it
/// (and automation may read it).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn file_created(path: &str) -> Option<u64> {
    let t = file_metadata(path)?.created().ok()?;
    t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis() as u64)
}

/// The file system's metadata of `path`, following links (when automation may read it).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn file_metadata(path: &str) -> Option<std::fs::Metadata> {
    may_read(path).ok()?;
    std::fs::metadata(path).ok()
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn file_created(_: &str) -> Option<u64> {
    None
}

/// `path` made absolute against the working directory (as given when absolute already, or when
/// that fails).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn absolute_path(path: &str) -> String {
    if std::path::Path::new(path).is_absolute() {
        return path.to_string();
    }
    std::path::absolute(path).map_or_else(|_| path.to_string(), |p| p.to_string_lossy().into_owned())
}

/// The file to open for `stored`, a path or `file:` URL stored in a document (a linked file, an
/// SVG image's href), or why it is not followed. `document` is the path of the document it is
/// stored in, when it has one: a relative path is taken from its folder. Only files on this
/// computer, or on the network share the document is on, are followed (see `craft_docpath`); a
/// path that is still relative (no document) is refused, as the working directory isn't the
/// document's choice. On Windows, a document on a mapped drive is on the share behind the drive
/// ([`share_path`]): a path refused as on another computer is checked again against the
/// document's path on that share. A path on a drive mapped to a network share other than the
/// document's ([`drive_share_path`]) is refused as on another computer. Every stored path
/// reaches the file system through this.
pub(crate) fn document_path(stored: &str, document: Option<&str>) -> std::result::Result<String, craft_docpath::Refused> {
    document_path_on(stored, document, share_path, drive_share_path)
}

/// [`document_path`], with `share` giving a document's path on the network share behind its drive.
pub(crate) fn document_path_with(
    stored: &str,
    document: Option<&str>,
    share: impl FnOnce(&str) -> Option<String>,
) -> std::result::Result<String, craft_docpath::Refused> {
    use craft_docpath::Refused;
    let document = document.map(absolute_path);
    let resolve = |document: Option<&str>| craft_docpath::resolve_absolute(stored, document.map(std::path::Path::new));
    let path = match resolve(document.as_deref()) {
        Err(Refused::Network) => {
            let share = document.as_deref().and_then(share).ok_or(Refused::Network)?;
            resolve(Some(share.as_str()))?
        }
        path => path?,
    };
    path.into_os_string().into_string().map_err(|_| Refused::Invalid)
}

/// [`document_path_with`], also refusing a path on a drive that is mapped to a network share other
/// than the document's (or, for a document on this computer or without a path, to any share) as
/// [`craft_docpath::Refused::Network`]. `drive_share` gives a path's path on the share behind its
/// drive (`Y:\art\x.png` on `\\server\other\art\x.png`), or `None` for a path on this computer. A
/// path on the document's own drive is not looked up. A document written `\\?\Z:\…` is looked up
/// on its share as `Z:\…` is.
pub(crate) fn document_path_on(
    stored: &str,
    document: Option<&str>,
    share: impl Fn(&str) -> Option<String>,
    drive_share: impl FnOnce(&str) -> Option<String>,
) -> std::result::Result<String, craft_docpath::Refused> {
    let share = |d: &str| share(d.strip_prefix(r"\\?\").filter(|d| drive_letter(d).is_some()).unwrap_or(d));
    let path = document_path_with(stored, document, share)?;
    let document = document.map(absolute_path);
    let Some(drive) = drive_letter(&path) else { return Ok(path) };
    if document.as_deref().and_then(drive_letter).is_some_and(|d| d.eq_ignore_ascii_case(&drive)) {
        return Ok(path);
    }
    let Some(on_share) = drive_share(&path) else { return Ok(path) };
    let home = document.map(|d| share(&d).unwrap_or(d));
    craft_docpath::resolve(&on_share, home.as_deref().map(std::path::Path::new))?;
    Ok(path)
}

/// The drive letter of an absolute path on a drive (`C:\…`, `C:/…` or `\\?\C:\…`).
fn drive_letter(path: &str) -> Option<char> {
    let mut chars = path.strip_prefix(r"\\?\").unwrap_or(path).chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some(l), Some(':'), Some('\\' | '/')) if l.is_ascii_alphabetic() => Some(l),
        _ => None,
    }
}

/// The path of `path` (on a drive, `Y:\art\x.png`) on the network share the drive is mapped to
/// (`\\server\other\art\x.png`), when the drive's root folder is on one and automation may read
/// `path`. The root folder's final path ([`std::fs::canonicalize`]) names the share.
#[cfg(windows)]
pub(crate) fn drive_share_path(path: &str) -> Option<String> {
    may_read(path).ok()?;
    drive_share_of(path, |root| std::fs::canonicalize(root).ok())
}

#[cfg(not(windows))]
pub(crate) fn drive_share_path(_: &str) -> Option<String> {
    None
}

/// The path of `path` on the network share its drive is mapped to, given `final_path`, which
/// returns the final path of the drive's root folder (`Y:\`): as [`share_of`] gives it for a
/// document, also for a path written `\\?\Y:\…`.
#[cfg(any(windows, test))]
pub(crate) fn drive_share_of(path: &str, final_path: impl FnOnce(&str) -> Option<std::path::PathBuf>) -> Option<String> {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    share_of(path, |folder| {
        let (root, rest) = (folder.get(..3)?, folder.get(3..)?);
        let root = final_path(root)?;
        let root = root.to_str()?.trim_end_matches('\\');
        Some(if rest.is_empty() { root.into() } else { format!(r"{root}\{rest}").into() })
    })
}

/// The path of `document` (on a drive, `Z:\jobs\doc.vectorcraft`) on the network share the drive is
/// mapped to (`\\server\share\jobs\doc.vectorcraft`), when its folder is on one and automation may
/// read the folder. The folder's final path ([`std::fs::canonicalize`]) names the share as
/// `\\?\UNC\server\share\…`.
#[cfg(windows)]
pub(crate) fn share_path(document: &str) -> Option<String> {
    share_of(document, |folder| {
        may_read(folder).ok()?;
        std::fs::canonicalize(folder).ok()
    })
}

#[cfg(not(windows))]
pub(crate) fn share_path(_: &str) -> Option<String> {
    None
}

/// The path of `document` on the network share its drive is mapped to, given `final_path`, which
/// returns the final path of a folder: `None` when `document` is not on a drive (`C:\` or `C:/`)
/// or `final_path` gives no share (`\\?\UNC\server\share\…`), such as a folder on a local volume.
#[cfg(any(windows, test))]
pub(crate) fn share_of(document: &str, final_path: impl FnOnce(&str) -> Option<std::path::PathBuf>) -> Option<String> {
    let mut chars = document.chars();
    if !matches!((chars.next(), chars.next(), chars.next()), (Some(l), Some(':'), Some('\\' | '/')) if l.is_ascii_alphabetic()) {
        return None;
    }
    // The drive's root folder keeps its separator (`Z:\`).
    let split = document.rfind(['\\', '/'])?;
    let (folder, name) = (document.get(..split.max(3))?, document.get(split + 1..)?);
    if name.is_empty() {
        return None;
    }
    let target = final_path(folder)?;
    let target = target.to_str()?;
    let unc = target.get(..8).filter(|p| p.eq_ignore_ascii_case(r"\\?\UNC\")).and(target.get(8..))?.trim_end_matches('\\');
    if unc.split('\\').filter(|c| !c.is_empty()).count() < 2 {
        return None;
    }
    Some(format!(r"\\{unc}\{name}"))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn file_stamp(_: &str) -> Option<(u64, Option<u64>)> {
    None
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn absolute_path(path: &str) -> String {
    path.to_string()
}

#[cfg(target_arch = "wasm32")]
fn no_fs(path: &str) -> EngineError {
    EngineError::Other(format!("{path}: no file system here (pass dataBase64 to open; omit the path to get dataBase64)"))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn read_file(path: &str) -> Result<Vec<u8>> {
    Err(no_fs(path))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn read_file_head(path: &str, _: usize) -> Result<Vec<u8>> {
    Err(no_fs(path))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn write_file(path: &str, _: &[u8]) -> Result<()> {
    Err(no_fs(path))
}

#[cfg(target_arch = "wasm32")]
pub fn write_file_with(path: &str, _: &mut dyn FnMut(&mut dyn std::io::Write) -> std::io::Result<()>) -> Result<()> {
    Err(no_fs(path))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn create_dir(path: &str) -> Result<()> {
    Err(no_fs(path))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn write_new_file(path: &str, _: &[u8]) -> Result<()> {
    Err(no_fs(path))
}
