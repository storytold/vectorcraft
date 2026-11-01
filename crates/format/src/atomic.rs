//! Safe saving: a file is written to a temporary file beside it, then renamed over it, so a failed
//! or interrupted write never leaves a damaged file behind (the old one stays as it was).
//! [`write_new_with`] creates a file the same way without ever replacing one.
//!
//! Where a folder refuses the temporary file or the rename (some network shares, a file another
//! program holds open), the new content is first written and flushed elsewhere and only then copied
//! into the file itself. That copy isn't atomic, so the file as it was is copied and flushed beside
//! the temporary file first (#1695): if the copy into the file fails or is cut short, both versions
//! are on disk, and the error names them. Where that copy of the old file can't be made, the file
//! is left as it was and the save fails.
//!
//! On macOS `File::sync_all` is a full flush (`F_FULLFSYNC`), which SMB shares refuse with
//! `ENOTSUP` while a plain `fsync` works: [`sync_file`] falls back to the plain one there (#1620).

use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Write `bytes` to `path` atomically (see the module docs).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with(path, |f| f.write_all(bytes))
}

/// Write `path` atomically with `fill`, which writes the content into the temporary file. When
/// `fill` or flushing fails the temporary file is removed and `path` is left untouched. When the
/// folder refuses the temporary file or the rename, the content is copied into `path` instead (see
/// the module docs).
pub fn write_atomic_with(path: &Path, fill: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    // Replacing a file: its staging copy starts owner-only, so a private document is never readable
    // beside the target before `keep_permissions` has run (#1712). A new file keeps the default mode.
    let replacing = path.exists();
    let create = |tmp: &Path| if replacing { create_private(tmp) } else { create_new(tmp) };
    write_atomic_stepping(path, fill, create, |tmp, target| std::fs::rename(tmp, target))
}

fn create_new(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// [`write_atomic_with`], with `create` making the temporary file beside the target and `rename`
/// moving it over the target.
fn write_atomic_stepping(
    path: &Path,
    fill: impl FnOnce(&mut File) -> io::Result<()>,
    create: impl FnOnce(&Path) -> io::Result<File>,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let target = resolve_link(path)?;
    let beside = temp_path(&target)?;
    // `refused`: why the folder takes no temporary file (it is in the system's folder then).
    let (tmp, mut f, refused) = match create(&beside) {
        Ok(f) => (beside, f, None),
        // A folder that isn't there, a full volume: nothing can be saved there.
        Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded) => return Err(e),
        Err(e) => {
            let Some(tmp) = beside.file_name().map(|n| std::env::temp_dir().join(n)) else { return Err(e) };
            let Ok(f) = create_private(&tmp) else { return Err(e) };
            (tmp, f, Some(e))
        }
    };
    let filled = fill(&mut f).and_then(|()| sync_file(&f));
    drop(f);
    if let Err(e) = filled {
        // Best effort: the temporary file is all there is to clean up.
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    let refused = match refused {
        Some(e) => e,
        None => {
            // Best effort: a volume without Unix permissions (SMB, FAT) refuses them, and has none
            // to keep.
            let _ = keep_permissions(&target, &tmp);
            let Err(e) = rename(&tmp, &target) else { return Ok(()) };
            e
        }
    };
    write_over(&tmp, &target, refused)
}

/// The content of `tmp`, which is whole and flushed, copied into `target` itself, and `tmp`
/// removed: for a folder that `refused` the atomic replacement. When `target` can't be opened for
/// writing it is untouched: `tmp` is removed and `refused` returned. When what it holds can't be
/// copied aside first ([`keep_old`]) it is untouched too: `tmp` stays and the error names it. When
/// the copy into `target` fails it is damaged: `tmp` and the copy of the old content stay and the
/// error says where they are.
fn write_over(tmp: &Path, target: &Path, refused: io::Error) -> io::Result<()> {
    let opened = File::open(tmp).and_then(|from| Ok((from, OpenOptions::new().write(true).create(true).truncate(false).open(target)?)));
    let Ok((mut from, mut to)) = opened else {
        // Best effort: the temporary file is all there is to clean up.
        let _ = std::fs::remove_file(tmp);
        return Err(refused);
    };
    // Nothing to keep of a file that is new or empty.
    let old = to.metadata().map_or(true, |m| m.len() > 0).then(|| tmp.with_extension("old"));
    if let Some(old) = &old
        && let Err(e) = keep_old(target, old)
    {
        // Best effort: a partial copy is of no use.
        let _ = std::fs::remove_file(old);
        let why =
            format!("{refused}, and the file can't be copied aside before it is overwritten ({e}); the new content is kept in {}", tmp.display());
        return Err(io::Error::new(refused.kind(), why));
    }
    let copied = to.set_len(0).and_then(|()| io::copy(&mut from, &mut to)).and_then(|_| sync_file(&to));
    drop((from, to));
    match (copied, old) {
        (Ok(()), old) => {
            // Best effort: the file is saved, so the copies are no longer needed.
            let _ = std::fs::remove_file(tmp);
            if let Some(old) = old {
                let _ = std::fs::remove_file(old);
            }
            Ok(())
        }
        (Err(e), Some(old)) => {
            Err(io::Error::new(e.kind(), format!("{e} (the new content is kept in {}, the file as it was in {})", tmp.display(), old.display())))
        }
        (Err(e), None) => Err(io::Error::new(e.kind(), format!("{e} (the new content is kept in {})", tmp.display()))),
    }
}

/// A flushed copy of `target` as it is, at `old` (for its owner only).
fn keep_old(target: &Path, old: &Path) -> io::Result<()> {
    let mut to = create_private(old)?;
    io::copy(&mut File::open(target)?, &mut to)?;
    sync_file(&to)
}

/// A new file for its owner only (Unix): the system's temporary folder is shared.
fn create_private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

/// Create `path` with the content `fill` writes, never replacing a file or a link of that name. The
/// content goes to a temporary file beside it, which is then linked in under `path`
/// (`std::fs::hard_link`, which fails with `AlreadyExists` when the name is taken, also when it was
/// taken meanwhile). The file appears whole or not at all. Where hard links fail for another
/// reason, `path` is created with `create_new` and the content copied in; a failed copy removes
/// it. The temporary file is always removed.
pub fn write_new_with(path: &Path, fill: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    write_new_linking(path, fill, |tmp, path| std::fs::hard_link(tmp, path))
}

/// [`write_new_with`], with `link` linking the temporary file in under `path`.
fn write_new_linking(
    path: &Path,
    fill: impl FnOnce(&mut File) -> io::Result<()>,
    link: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let tmp = temp_path(path)?;
    let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
    let filled = fill(&mut f).and_then(|()| sync_file(&f));
    drop(f);
    let placed = filled.and_then(|()| match link(&tmp, path) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => copy_new(&tmp, path),
        linked => linked,
    });
    // Best effort: the temporary file is all there is to clean up.
    let _ = std::fs::remove_file(&tmp);
    placed
}

/// Create `path` (never replacing anything) with the content of the file `from`; a failed copy
/// removes it again.
fn copy_new(from: &Path, path: &Path) -> io::Result<()> {
    let mut to = OpenOptions::new().write(true).create_new(true).open(path)?;
    let copied = File::open(from).and_then(|mut f| io::copy(&mut f, &mut to)).and_then(|_| sync_file(&to));
    drop(to);
    if copied.is_err() {
        // Best effort: the part copied is all there is to clean up.
        let _ = std::fs::remove_file(path);
    }
    copied
}

/// Flush `file` to stable storage (`sync_all`). On Apple platforms that is a full flush
/// (`F_FULLFSYNC`); where the volume refuses it with `ENOTSUP` (SMB shares, #1620) this falls back
/// to a plain `fsync`. Any other error, and a failed `fsync`, is returned as it is.
pub fn sync_file(file: &File) -> io::Result<()> {
    match full_sync(file) {
        Err(e) if full_flush_unsupported(&e) => plain_fsync(file),
        r => r,
    }
}

/// Whether `e` is a volume refusing a full flush. Never off Apple platforms: there `sync_all` is
/// already a plain `fsync`. Checked by its code because std maps `ENOTSUP` to `Uncategorized`.
fn full_flush_unsupported(e: &io::Error) -> bool {
    cfg!(target_vendor = "apple") && e.raw_os_error() == Some(ENOTSUP)
}

#[cfg(target_vendor = "apple")]
fn plain_fsync(file: &File) -> io::Result<()> {
    #[cfg(test)]
    {
        PLAIN_FSYNCS.with(|c| c.set(c.get() + 1));
        if let Some(errno) = PLAIN_FSYNC_FAILS.with(std::cell::Cell::get) {
            return Err(io::Error::from_raw_os_error(errno));
        }
    }
    rustix::fs::fsync(file).map_err(io::Error::from)
}

#[cfg(not(target_vendor = "apple"))]
fn plain_fsync(file: &File) -> io::Result<()> {
    file.sync_all()
}

fn full_sync(file: &File) -> io::Result<()> {
    #[cfg(test)]
    {
        if REFUSE_FULL_SYNC.with(std::cell::Cell::get) {
            return Err(io::Error::from_raw_os_error(ENOTSUP));
        }
        match FLUSHES_LEFT.with(std::cell::Cell::get) {
            Some(0) => return Err(io::Error::other("the connection was lost")),
            Some(n) => FLUSHES_LEFT.with(|c| c.set(Some(n - 1))),
            None => {}
        }
    }
    file.sync_all()
}

/// What macOS answers when a volume can't make a full flush (SMB shares).
#[cfg(target_vendor = "apple")]
const ENOTSUP: i32 = 45;
#[cfg(not(target_vendor = "apple"))]
const ENOTSUP: i32 = 95;

#[cfg(test)]
thread_local! {
    /// Tests: every full flush on this thread fails as on an SMB share mounted by macOS.
    static REFUSE_FULL_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Tests: the plain `fsync` fallback fails too, with this OS error.
    static PLAIN_FSYNC_FAILS: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
    /// Tests: after this many more flushes on this thread, every flush fails.
    static FLUSHES_LEFT: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
    /// Tests: how many plain `fsync` fallbacks were made on this thread.
    static PLAIN_FSYNCS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// A symbolic link keeps pointing at its file: write the file it names.
fn resolve_link(path: &Path) -> io::Result<PathBuf> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => std::fs::canonicalize(path),
        _ => Ok(path.to_path_buf()),
    }
}

/// A fresh hidden name beside `target` (the same folder, so the rename stays on one volume).
fn temp_path(target: &Path) -> io::Result<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = target.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{}: not a file path", target.display())))?;
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    Ok(target.with_file_name(format!(".{}.{}-{n}.tmp", name.to_string_lossy(), std::process::id())))
}

/// The replacement keeps the replaced file's permissions, group and (Linux) access ACL: the same
/// mode bits mean other people when the group differs, and an ACL can narrow who reads the file.
/// When the group or the ACL can't be kept, the replacement is readable by its owner alone rather
/// than by people the old file didn't let in. (Unix; Windows files have none to keep beyond
/// read-only, which makes the save fail as writing in place would.)
#[cfg(unix)]
fn keep_permissions(target: &Path, tmp: &Path) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    // A new file: the default permissions.
    let Ok(m) = std::fs::metadata(target) else { return Ok(()) };
    let mut mode = m.permissions().mode();
    let kept_group = std::fs::metadata(tmp).is_ok_and(|t| t.gid() == m.gid()) || std::os::unix::fs::chown(tmp, None, Some(m.gid())).is_ok();
    if !kept_group || !keep_acl(target, tmp) {
        mode &= !0o077;
    }
    // After the ACL: the mode's group bits are its mask, kept as they were.
    std::fs::set_permissions(tmp, std::fs::Permissions::from_mode(mode))
}

/// Give `tmp` the access ACL of `target`; `true` when it has none (or its volume keeps none).
#[cfg(target_os = "linux")]
fn keep_acl(target: &Path, tmp: &Path) -> bool {
    use rustix::io::Errno;
    const ACL: &str = "system.posix_acl_access";
    let mut buf = vec![0u8; 64 << 10];
    match rustix::fs::getxattr(target, ACL, &mut buf[..]) {
        Ok(n) => buf.get(..n).is_some_and(|acl| rustix::fs::setxattr(tmp, ACL, acl, rustix::fs::XattrFlags::empty()).is_ok()),
        Err(e) => e == Errno::NODATA || e == Errno::NOTSUP,
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn keep_acl(_: &Path, _: &Path) -> bool {
    true
}

#[cfg(not(unix))]
fn keep_permissions(_: &Path, _: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vc-atomic-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The other files in `d` besides `keep`.
    fn leftovers(d: &Path, keep: &str) -> Vec<String> {
        std::fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).filter(|n| n != keep).collect()
    }

    /// Replacing a private file (#1712): the staging file is owner-only while it is written, whatever
    /// the umask, and the saved file keeps the old permissions.
    #[cfg(unix)]
    #[test]
    fn staging_of_a_private_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let d = dir("private-staging");
        let path = d.join("doc.vectorcraft");
        std::fs::write(&path, b"secret").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut staging_mode = 0;
        write_atomic_with(&path, |f| {
            staging_mode = f.metadata()?.permissions().mode() & 0o777;
            f.write_all(b"new")
        })
        .unwrap();
        assert_eq!(staging_mode, 0o600, "the staging file must not be group or world readable");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_original_file_survives_a_failed_write() {
        let d = dir("fail");
        let path = d.join("doc.vectorcraft");
        std::fs::write(&path, b"original").unwrap();
        let r = write_atomic_with(&path, |f| {
            f.write_all(b"half a fi")?;
            Err(io::Error::other("disk full"))
        });
        assert!(r.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert!(leftovers(&d, "doc.vectorcraft").is_empty(), "the temporary file is removed");
        write_atomic(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert!(leftovers(&d, "doc.vectorcraft").is_empty());
        // A folder that doesn't exist: an error, nothing written.
        assert!(write_atomic(&d.join("missing").join("x.vectorcraft"), b"x").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A save to an SMB share (#1620): macOS refuses the full flush there with `ENOTSUP`, so the
    /// file is flushed with a plain `fsync` and saved. When that fails too the save fails as before,
    /// the old file stays and no temporary file is left. Elsewhere `sync_all` is already a plain
    /// `fsync`: a refusal is an error like any other.
    #[test]
    fn a_volume_that_refuses_the_full_flush_still_saves() {
        let d = dir("no-full-sync");
        let path = d.join("doc.vectorcraft");
        std::fs::write(&path, b"original").unwrap();
        let apple = cfg!(target_vendor = "apple");
        REFUSE_FULL_SYNC.with(|c| c.set(true));
        let before = PLAIN_FSYNCS.with(std::cell::Cell::get);
        let saved = write_atomic(&path, b"new");
        let created = write_new_with(&d.join("new.otf"), |f| f.write_all(b"created"));
        let fsyncs = PLAIN_FSYNCS.with(std::cell::Cell::get) - before;
        PLAIN_FSYNC_FAILS.with(|c| c.set(Some(5)));
        let failed = write_atomic(&path, b"lost");
        PLAIN_FSYNC_FAILS.with(|c| c.set(None));
        REFUSE_FULL_SYNC.with(|c| c.set(false));
        assert_eq!(saved.is_ok(), apple, "{saved:?}");
        assert_eq!(created.is_ok(), apple, "{created:?}");
        assert_eq!(fsyncs, if apple { 2 } else { 0 }, "each refused full flush falls back to fsync");
        assert!(failed.is_err(), "a failed fsync still fails the save");
        assert_eq!(std::fs::read(&path).unwrap(), if apple { &b"new"[..] } else { b"original" });
        let mut left = leftovers(&d, "doc.vectorcraft");
        left.retain(|n| n != "new.otf");
        assert!(left.is_empty(), "no temporary file is left: {left:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    fn denied<T>() -> io::Result<T> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "refused by the share"))
    }

    /// A folder that refuses the rename over the file (#1620: some network shares): the new content,
    /// whole in the temporary file, is copied into the file itself. A new file is created.
    #[test]
    fn a_refused_rename_copies_the_content_into_the_file() {
        let d = dir("no-rename");
        let path = d.join("doc.vectorcraft");
        std::fs::write(&path, b"original, and longer").unwrap();
        write_atomic_stepping(&path, |f| f.write_all(b"new"), create_new, |_, _| denied()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        write_atomic_stepping(&d.join("other.vectorcraft"), |f| f.write_all(b"other"), create_new, |_, _| denied()).unwrap();
        assert_eq!(std::fs::read(d.join("other.vectorcraft")).unwrap(), b"other");
        assert_eq!(leftovers(&d, "doc.vectorcraft"), ["other.vectorcraft"], "the temporary files are removed");
        // A failed write never gets as far as the file.
        let r = write_atomic_stepping(&path, |f| f.write_all(b"half").and(Err(io::Error::other("disk full"))), create_new, |_, _| denied());
        assert_eq!(r.unwrap_err().to_string(), "disk full");
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A refused rename where the file can't be written either (here: a folder of that name): the
    /// rename's error, nothing changed and no temporary file left.
    #[test]
    fn a_refused_rename_over_a_file_that_cannot_be_written_changes_nothing() {
        let d = dir("no-rename-no-write");
        let path = d.join("doc.vectorcraft");
        std::fs::create_dir(&path).unwrap();
        let r = write_atomic_stepping(&path, |f| f.write_all(b"new"), create_new, |_, _| denied());
        assert_eq!(r.unwrap_err().to_string(), "refused by the share");
        assert!(path.is_dir() && leftovers(&d, "doc.vectorcraft").is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// #1695: a refused rename and a copy into the file that fails (here: its flush, after the copy
    /// of the old content was flushed): the file is damaged, so the temporary file with the whole
    /// new content and the copy of the old one stay, and the error names both.
    #[test]
    fn a_failed_copy_into_the_file_keeps_the_old_content_and_the_new() {
        let d = dir("no-rename-copy-fails-late");
        let path = d.join("doc.vectorcraft");
        std::fs::write(&path, b"original").unwrap();
        let r = write_atomic_stepping(
            &path,
            |f| f.write_all(b"new"),
            create_new,
            |_, _| {
                // One more flush works (the copy of the old content), the next doesn't.
                FLUSHES_LEFT.with(|c| c.set(Some(1)));
                denied()
            },
        );
        FLUSHES_LEFT.with(|c| c.set(None));
        let message = r.unwrap_err().to_string();
        let mut left = leftovers(&d, "doc.vectorcraft");
        left.sort();
        let [old, tmp] = left.as_slice() else { panic!("the old content and the new: {left:?}") };
        assert!(old.ends_with(".old") && tmp.ends_with(".tmp"), "{left:?}");
        assert!(message.contains(tmp.as_str()) && message.contains(old.as_str()), "{message}");
        assert_eq!((std::fs::read(d.join(old)).unwrap(), std::fs::read(d.join(tmp)).unwrap()), (b"original".to_vec(), b"new".to_vec()));
        // A new file has nothing to keep, and a save that works leaves neither copy.
        std::fs::remove_file(d.join(old)).unwrap();
        std::fs::remove_file(d.join(tmp)).unwrap();
        let fresh = d.join("fresh.vectorcraft");
        write_atomic_stepping(&fresh, |f| f.write_all(b"new"), create_new, |_, _| denied()).unwrap();
        write_atomic_stepping(&fresh, |f| f.write_all(b"newer"), create_new, |_, _| denied()).unwrap();
        assert_eq!(std::fs::read(&fresh).unwrap(), b"newer");
        assert_eq!(leftovers(&d, "doc.vectorcraft"), ["fresh.vectorcraft"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A refused rename where the old content can't be copied aside (here: no flush works): the
    /// file is left as it was, the temporary file with the whole new content stays and the error
    /// names it.
    #[test]
    fn a_failed_copy_into_the_file_keeps_the_temporary_file() {
        let d = dir("no-rename-copy-fails");
        let path = d.join("doc.vectorcraft");
        std::fs::write(&path, b"original").unwrap();
        let r = write_atomic_stepping(
            &path,
            |f| f.write_all(b"new"),
            create_new,
            |_, _| {
                // From here on flushing fails (also the plain `fsync` macOS falls back to).
                REFUSE_FULL_SYNC.with(|c| c.set(true));
                PLAIN_FSYNC_FAILS.with(|c| c.set(Some(5)));
                denied()
            },
        );
        REFUSE_FULL_SYNC.with(|c| c.set(false));
        PLAIN_FSYNC_FAILS.with(|c| c.set(None));
        let message = r.unwrap_err().to_string();
        let left = leftovers(&d, "doc.vectorcraft");
        let [tmp] = left.as_slice() else { panic!("one temporary file: {left:?}") };
        assert!(message.contains(tmp.as_str()), "{message}");
        assert_eq!(std::fs::read(d.join(tmp)).unwrap(), b"new");
        assert_eq!(std::fs::read(&path).unwrap(), b"original", "not overwritten without a copy of it");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A folder that takes no temporary file (#1620: a share that refuses hidden or new names) but
    /// lets the file be written: the content goes through the system's temporary folder, and no
    /// rename is tried. A folder that isn't there is still an error.
    #[test]
    fn a_folder_that_refuses_the_temporary_file_still_saves() {
        let d = dir("no-temp");
        let path = d.join("doc.vectorcraft");
        std::fs::write(&path, b"original, and longer").unwrap();
        let renamed = std::cell::Cell::new(false);
        let rename = |_: &Path, _: &Path| {
            renamed.set(true);
            Ok(())
        };
        write_atomic_stepping(&path, |f| f.write_all(b"new"), |_| denied(), rename).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert!(!renamed.get() && leftovers(&d, "doc.vectorcraft").is_empty());
        // The file can't be written either (a folder of that name): the folder's error.
        let folder = d.join("folder");
        std::fs::create_dir(&folder).unwrap();
        let r = write_atomic_stepping(&folder, |f| f.write_all(b"new"), |_| denied(), |_, _| Ok(()));
        assert_eq!(r.unwrap_err().to_string(), "refused by the share");
        let r = write_atomic_stepping(&path, |f| f.write_all(b"lost"), |_| Err(io::Error::from(io::ErrorKind::NotFound)), |_, _| Ok(()));
        assert_eq!(r.map_err(|e| e.kind()), Err(io::ErrorKind::NotFound));
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn write_new_never_replaces_what_is_there() {
        let d = dir("new");
        let path = d.join("a.otf");
        write_new_with(&path, |f| f.write_all(b"first")).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        assert!(leftovers(&d, "a.otf").is_empty(), "the temporary file is removed");
        let r = write_new_with(&path, |f| f.write_all(b"second"));
        assert_eq!(r.map_err(|e| e.kind()), Err(io::ErrorKind::AlreadyExists));
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        assert!(leftovers(&d, "a.otf").is_empty());
        // A failed write leaves nothing.
        let failed = d.join("b.otf");
        assert!(write_new_with(&failed, |f| f.write_all(b"half").and(Err(io::Error::other("disk full")))).is_err());
        assert!(!failed.exists() && leftovers(&d, "a.otf").is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Where hard links fail (a file system without them), the content is copied in, and a taken
    /// name stays as it was. A link that fails because the name is taken copies nothing.
    #[test]
    fn write_new_copies_where_hard_links_fail() {
        let d = dir("new-copy");
        let unsupported = |_: &Path, _: &Path| Err(io::Error::from(io::ErrorKind::Unsupported));
        let path = d.join("c.otf");
        write_new_linking(&path, |f| f.write_all(b"copied"), unsupported).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"copied");
        assert!(leftovers(&d, "c.otf").is_empty(), "the temporary file is removed");
        let r = write_new_linking(&path, |f| f.write_all(b"second"), unsupported);
        assert_eq!(r.map_err(|e| e.kind()), Err(io::ErrorKind::AlreadyExists));
        assert_eq!(std::fs::read(&path).unwrap(), b"copied");
        let taken = |_: &Path, _: &Path| Err(io::Error::from(io::ErrorKind::AlreadyExists));
        let other = d.join("d.otf");
        let r = write_new_linking(&other, |f| f.write_all(b"other"), taken);
        assert_eq!(r.map_err(|e| e.kind()), Err(io::ErrorKind::AlreadyExists));
        assert!(!other.exists() && leftovers(&d, "c.otf").is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A link of that name, even one to nothing, is never replaced or written through.
    #[cfg(unix)]
    #[test]
    fn write_new_leaves_links_alone() {
        let d = dir("new-links");
        let target = d.join("target.otf");
        std::fs::write(&target, b"target").unwrap();
        std::os::unix::fs::symlink(&target, d.join("link.otf")).unwrap();
        std::os::unix::fs::symlink(d.join("nothing.otf"), d.join("dangling.otf")).unwrap();
        for name in ["link.otf", "dangling.otf"] {
            let r = write_new_with(&d.join(name), |f| f.write_all(b"new"));
            assert_eq!(r.map_err(|e| e.kind()), Err(io::ErrorKind::AlreadyExists), "{name}");
        }
        assert_eq!(std::fs::read(&target).unwrap(), b"target");
        assert!(!d.join("nothing.otf").exists());
        let mut names: Vec<String> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        names.sort();
        assert_eq!(names, ["dangling.otf", "link.otf", "target.otf"], "no temporary file is left");
        let _ = std::fs::remove_dir_all(&d);
    }
}
