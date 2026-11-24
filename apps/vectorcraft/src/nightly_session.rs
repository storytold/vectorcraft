//! App-private Nightly session health, not telemetry and not a Launcher protocol.
//! A live owner holds a separate persistent lease; secondary launches never rewrite its record.
//! Dropping the guard does not clear the marker. Only the app's controlled return path does.

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

const RECORD: &str = "nightly-session.json";
const LEASE: &str = "nightly-session.lock";
const MAX_METADATA_BYTES: usize = 256;

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
/// Set once a window has posted the interruption notice; until then the record must survive.
static NOTICE_SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub struct Session {
    _lease: File,
    path: PathBuf,
    own_record: Vec<u8>,
    previous_unclean: bool,
}

fn regular_or_missing(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(io::Error::other("Nightly session state must be a regular file, not an alias or directory")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Replace `path` with `bytes` without ever following a link at the destination: the temporary file
/// is created fresh in the same folder and renamed over the name, and a rename replaces the link
/// itself, never its target. A destination that is not a regular file is refused.
fn atomic_write_no_follow(path: &Path, bytes: &[u8]) -> io::Result<()> {
    // A destination that is a folder or a link is refused before anything is written.
    regular_or_missing(path)?;
    let dir = path.parent().ok_or_else(|| io::Error::other("Nightly session state has no folder"))?;
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let temporary = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&temporary);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options.open(&temporary).and_then(|mut file| {
        use std::io::Write as _;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    });
    if written.is_err() {
        // Best effort: the temporary file is all there is to clean up.
        let _ = std::fs::remove_file(&temporary);
    }
    if written.is_ok()
        && let Ok(handle) = File::open(dir)
    {
        // The rename is durable once its folder is.
        let _ = handle.sync_all();
    }
    written
}

impl Session {
    /// This launch's record: `None` for a stable build or when another window of the same profile
    /// owns it. The root must already be the validated private Nightly profile.
    pub fn begin(root: Option<&Path>, version: &str, commit: Option<&str>) -> io::Result<Option<Self>> {
        let Some(root) = root else { return Ok(None) };
        if version.is_empty() || version.len() > MAX_METADATA_BYTES || commit.is_some_and(|value| value.len() > MAX_METADATA_BYTES) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Nightly session provenance is missing or too large"));
        }
        let lease_path = root.join(LEASE);
        regular_or_missing(&lease_path)?;
        let lease = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lease_path)?;
        match lease.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        }
        let path = root.join(RECORD);
        let previous_unclean = regular_or_missing(&path)?;
        let own_record = serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "channel": "nightly-v1",
            "version": version,
            "commit": commit.filter(|value| !value.is_empty()),
        }))?;
        atomic_write_no_follow(&path, &own_record)?;
        Ok(Some(Self { _lease: lease, path, own_record, previous_unclean }))
    }

    pub fn previous_unclean(&self) -> bool {
        self.previous_unclean
    }

    /// Controlled return. If an earlier session's interruption was detected but no window showed it
    /// yet (a renderer failure before the app callback, or the hand-off to a second window), the
    /// record is left for the next owner to report; otherwise this is [`Self::complete`].
    pub fn finish(self, unclean_notice_shown: bool) -> io::Result<()> {
        if self.previous_unclean && !unclean_notice_shown {
            return Ok(());
        }
        self.complete()
    }

    /// Controlled event-loop completion, including a handled startup error. Clear before a successor
    /// can take the profile. An unwind, abort, forced exit or killed process never calls this.
    pub fn complete(self) -> io::Result<()> {
        if !regular_or_missing(&self.path)? {
            return Ok(());
        }
        let mut current = Vec::new();
        File::open(&self.path)?.take(self.own_record.len() as u64 + 1).read_to_end(&mut current)?;
        if current != self.own_record {
            return Err(io::Error::other("Nightly session state changed; leaving it intact"));
        }
        std::fs::remove_file(&self.path)
    }
}

/// Starts this launch's record, keeping it until [`finish_once`]. A profile another window already
/// owns gets no record (`Ok(false)` and no notice); a failure to record is reported, never fatal.
pub fn begin(root: Option<&Path>, version: &str, commit: Option<&str>) -> Result<bool, String> {
    let session = Session::begin(root, version, commit).map_err(|e| e.to_string())?;
    let previous_unclean = session.as_ref().is_some_and(Session::previous_unclean);
    *SESSION.lock().unwrap_or_else(PoisonError::into_inner) = session;
    Ok(previous_unclean)
}

/// A window showed the interruption notice: a controlled return now clears the record.
pub fn mark_notice_shown() {
    NOTICE_SHOWN.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// The controlled return, by whichever path comes first (the window's `on_exit`, which every quit
/// reaches, or the code after the event loop). Later calls do nothing.
pub fn finish_once() {
    let shown = NOTICE_SHOWN.load(std::sync::atomic::Ordering::Relaxed);
    let watch = SESSION.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(watch) = watch
        && let Err(error) = watch.finish(shown)
    {
        log::warn!("Nightly session state was left intact: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            loop {
                let path =
                    std::env::temp_dir().join(format!("vectorcraft-nightly-session-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("cannot create owned fixture: {error}"),
                }
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn stable_has_no_session_side_effects() {
        let fixture = Fixture::new();
        assert!(Session::begin(None, "0.24.1", None).unwrap().is_none());
        assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 0);
    }

    #[test]
    fn a_busy_secondary_reports_nothing_and_keeps_the_owner_record() {
        let fixture = Fixture::new();
        let first = Session::begin(Some(&fixture.0), "nightly", Some("owned-commit")).unwrap().unwrap();
        let record = std::fs::read(fixture.0.join(RECORD)).unwrap();
        assert!(Session::begin(Some(&fixture.0), "secondary", None).unwrap().is_none());
        assert_eq!(std::fs::read(fixture.0.join(RECORD)).unwrap(), record, "a busy secondary must not replace the owner's record");
        first.complete().unwrap();
        assert!(!fixture.0.join(RECORD).exists());
    }

    #[test]
    fn controlled_completion_is_clean_but_an_unfinished_owner_is_reported_next_time() {
        let fixture = Fixture::new();
        let first = Session::begin(Some(&fixture.0), "first", Some("recorded-commit")).unwrap().unwrap();
        assert!(!first.previous_unclean());
        first.complete().unwrap();
        assert!(!fixture.0.join(RECORD).exists());
        let unfinished = Session::begin(Some(&fixture.0), "second", None).unwrap().unwrap();
        assert!(!unfinished.previous_unclean());
        drop(unfinished); // Unwind or abnormal ownership release, not a controlled completion.
        let next = Session::begin(Some(&fixture.0), "third", None).unwrap().unwrap();
        assert!(next.previous_unclean());
        next.complete().unwrap();
        assert!(!fixture.0.join(RECORD).exists());
    }

    #[test]
    fn an_interruption_no_window_showed_survives_a_handled_startup_failure() {
        let fixture = Fixture::new();
        drop(Session::begin(Some(&fixture.0), "crashed", None).unwrap().unwrap());
        // The window failed before the app callback: no notice was shown.
        let failed = Session::begin(Some(&fixture.0), "failed", None).unwrap().unwrap();
        assert!(failed.previous_unclean());
        failed.finish(false).unwrap();
        let retry = Session::begin(Some(&fixture.0), "retry", None).unwrap().unwrap();
        assert!(retry.previous_unclean());
        retry.finish(true).unwrap();
        assert!(!fixture.0.join(RECORD).exists(), "a session that showed the notice clears its record");
        let clean = Session::begin(Some(&fixture.0), "clean", None).unwrap().unwrap();
        assert!(!clean.previous_unclean());
        clean.finish(false).unwrap();
        assert!(!fixture.0.join(RECORD).exists());
    }

    #[test]
    fn cleanup_never_removes_an_unknown_changed_record() {
        let fixture = Fixture::new();
        let watch = Session::begin(Some(&fixture.0), "nightly", None).unwrap().unwrap();
        std::fs::write(fixture.0.join(RECORD), b"replacement owned by someone else").unwrap();
        assert!(watch.complete().is_err());
        assert_eq!(std::fs::read(fixture.0.join(RECORD)).unwrap(), b"replacement owned by someone else");
    }

    #[test]
    fn provenance_is_bounded_and_never_collects_documents_paths_or_diagnostics() {
        let fixture = Fixture::new();
        assert!(Session::begin(Some(&fixture.0), &"x".repeat(MAX_METADATA_BYTES + 1), None).is_err());
        assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 0);
        let watch = Session::begin(Some(&fixture.0), "nightly", None).unwrap().unwrap();
        let data: serde_json::Value = serde_json::from_slice(&std::fs::read(fixture.0.join(RECORD)).unwrap()).unwrap();
        let fields = data.as_object().unwrap().keys().map(String::as_str).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(fields, ["schema_version", "channel", "version", "commit"].into_iter().collect());
        assert!(data["commit"].is_null(), "missing provenance must not be invented");
        watch.complete().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn reserved_state_aliases_do_not_modify_external_files() {
        let fixture = Fixture::new();
        let external = fixture.0.join("stable-document");
        std::fs::write(&external, b"stable bytes").unwrap();
        for name in [LEASE, RECORD] {
            let alias = fixture.0.join(name);
            std::os::unix::fs::symlink(&external, &alias).unwrap();
            assert!(Session::begin(Some(&fixture.0), "nightly", None).is_err(), "{name}");
            assert_eq!(std::fs::read(&external).unwrap(), b"stable bytes");
            std::fs::remove_file(alias).unwrap();
        }
    }
}
