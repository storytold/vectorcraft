//! The folders Save for Web's and SVG Options' browser previews go to: one new folder per preview in
//! a private root inside the system's temporary folder, with the old ones removed.

/// The age at which a later browser preview removes an earlier preview's folder.
#[cfg(not(target_arch = "wasm32"))]
pub(super) const PREVIEW_KEEP: std::time::Duration = std::time::Duration::from_secs(3600);

/// The folder Save for Web's browser previews go to inside the system's temporary folder.
pub(super) const ROOT: &str = "VectorCraft Save for Web";

/// The preview page's name from the document's title `stem`: one file name in the preview folder.
/// `file_stem` gives back a title it can't take a stem of ("../.."), so one with a separator or of
/// dots only is "Untitled".
pub(super) fn preview_name(stem: &str) -> &str {
    if stem.trim().is_empty() || stem.contains(['/', '\\']) || stem.chars().all(|c| c == '.') { "Untitled" } else { stem }
}

/// A new folder for a browser preview in the system's temporary folder, under the folder named
/// `root` ([`preview_folder_under`]).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn temp_folder_under(root: &str) -> Result<String, String> {
    #[cfg(test)]
    if let Some(tmp) = TEST_TMP.with_borrow(Clone::clone) {
        return preview_folder_under(&tmp, root, PREVIEW_KEEP);
    }
    preview_folder_under(&std::env::temp_dir(), root, PREVIEW_KEEP)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
thread_local! {
    /// The folder this test thread's browser previews go to instead of the system's temporary
    /// folder, when set.
    pub(super) static TEST_TMP: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Remove the folder `dir` made for a browser preview whose files could not all be written, so
/// a failed preview leaves no folder behind. The log says when it can't be removed.
pub(super) fn remove_preview_folder(dir: &str) {
    if let Err(e) = std::fs::remove_dir_all(dir) {
        log::warn!("vectorcraft: the browser preview folder {dir} can't be removed: {e}");
    }
}

/// A new folder for a Save for Web browser preview in the temporary folder `tmp`
/// ([`preview_folder_under`] with `VectorCraft Save for Web`).
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(super) fn preview_folder_in(tmp: &std::path::Path, keep: std::time::Duration) -> Result<String, String> {
    preview_folder_under(tmp, ROOT, keep)
}

/// A new folder for a browser preview in the temporary folder `tmp`: inside the folder named `root`
/// ([`preview_folder`]), or, when that folder can't be used, directly in `tmp`, named `root`, a
/// space, the process id and a count. The log says why the shared folder wasn't used. Each preview
/// first removes the folders of that name in `tmp` last changed `keep` or longer ago
/// ([`remove_old`]). Previews under one `root` never remove the folders of another.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn preview_folder_under(tmp: &std::path::Path, root: &str, keep: std::time::Duration) -> Result<String, String> {
    let beside = format!("{root} ");
    remove_old(tmp, &beside, keep);
    preview_folder(&tmp.join(root), keep).or_else(|e| {
        log::warn!("vectorcraft: a browser preview can't use {e}, so the preview goes to a new folder of its own");
        new_folder(tmp, &beside)
    })
}

/// A new folder in `root` ([`new_folder`]). `root` is made when it is missing. An existing `root`
/// is refused when it is a link or not a folder, or, on unix, when it belongs to another user or
/// users other than its owner can write to it. Before the new folder is made, the preview folders
/// in `root` last changed `keep` or longer ago are removed ([`remove_old`]).
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn preview_folder(root: &std::path::Path, keep: std::time::Duration) -> Result<String, String> {
    match private_folder().recursive(true).create(root) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(format!("{}: {e}", root.display())),
    }
    let m = std::fs::symlink_metadata(root).map_err(|e| format!("{}: {e}", root.display()))?;
    if let Some(why) = refused(&m) {
        return Err(format!("{}: {}", root.display(), why));
    }
    remove_old(root, "", keep);
    new_folder(root, "")
}

/// Removes the preview folders in `dir` last changed `keep` or longer ago: folders (not links)
/// named `prefix` followed by `<digits>-<digits>`, on unix only those the current user owns. Other
/// entries are left alone.
#[cfg(not(target_arch = "wasm32"))]
fn remove_old(dir: &std::path::Path, prefix: &str, keep: std::time::Duration) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if !is_preview_name(&e.file_name(), prefix) {
            continue;
        }
        // `DirEntry::metadata` doesn't follow a link, so a link is never a folder here.
        let Ok(m) = e.metadata() else { continue };
        let old = m.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age >= keep);
        if old && m.is_dir() && mine(&m) {
            // Best effort: one that can't be removed now is tried again by the next preview.
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// A new folder in `parent` named `prefix` followed by the process id, `-` and a count of the
/// process's previews; on unix only its owner can open it. A name that is already taken, by an
/// earlier process with the same id or by anyone else, is skipped for the next count.
#[cfg(not(target_arch = "wasm32"))]
fn new_folder(parent: &std::path::Path, prefix: &str) -> Result<String, String> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    loop {
        let dir = parent.join(format!("{prefix}{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        match private_folder().create(&dir) {
            Ok(()) => return Ok(dir.to_string_lossy().replace('\\', "/")),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("{}: {e}", dir.display())),
        }
    }
}

/// A builder for folders that only their owner can read, write or open (mode 0o700).
#[cfg(unix)]
fn private_folder() -> std::fs::DirBuilder {
    let mut b = std::fs::DirBuilder::new();
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b
}

/// A builder for folders: a plain one outside unix.
#[cfg(all(not(unix), not(target_arch = "wasm32")))]
fn private_folder() -> std::fs::DirBuilder {
    std::fs::DirBuilder::new()
}

/// Why a preview root whose metadata (of the root itself, not what a link points to) is `m` can't
/// be used, or `None` when it can.
#[cfg(not(target_arch = "wasm32"))]
fn refused(m: &std::fs::Metadata) -> Option<&'static str> {
    if m.file_type().is_symlink() {
        Some("it is a link")
    } else if !m.is_dir() {
        Some("it is not a folder")
    } else if !mine(m) {
        Some("it belongs to another user")
    } else if others_can_write(m) {
        Some("other users can write to it")
    } else {
        None
    }
}

/// Whether the current user owns the file with metadata `m`.
#[cfg(unix)]
fn mine(m: &std::fs::Metadata) -> bool {
    std::os::unix::fs::MetadataExt::uid(m) == rustix::process::geteuid().as_raw()
}

/// Whether the current user owns the file with metadata `m`: always true outside unix.
#[cfg(all(not(unix), not(target_arch = "wasm32")))]
fn mine(_: &std::fs::Metadata) -> bool {
    true
}

/// Whether users other than the owner of the file with metadata `m` can write to it.
#[cfg(unix)]
fn others_can_write(m: &std::fs::Metadata) -> bool {
    std::os::unix::fs::MetadataExt::mode(m) & 0o022 != 0
}

/// Whether users other than the owner of the file with metadata `m` can write to it: always false
/// outside unix.
#[cfg(all(not(unix), not(target_arch = "wasm32")))]
fn others_can_write(_: &std::fs::Metadata) -> bool {
    false
}

/// Whether `name` is a preview folder's name: `prefix`, then digits, `-` and digits.
#[cfg(not(target_arch = "wasm32"))]
fn is_preview_name(name: &std::ffi::OsStr, prefix: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    name.to_str().and_then(|n| n.strip_prefix(prefix)).and_then(|n| n.split_once('-')).is_some_and(|(a, b)| digits(a) && digits(b))
}

#[cfg(target_arch = "wasm32")]
pub(super) fn temp_folder_under(_: &str) -> Result<String, String> {
    Err("no file system here".into())
}
