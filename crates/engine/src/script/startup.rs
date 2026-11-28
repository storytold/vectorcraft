//! The Startup Scripts folder: the `.jsx` files `script.startup.run` runs, one after another, when
//! the app starts and before a script chosen from File › Scripts runs ([`list`]), and the read of
//! one of them when its turn comes ([`read`]).
//!
//! Only the folder's own files are taken, never its subfolders. A listing follows no link: a
//! symbolic link or a Windows junction is left out, and so is a folder that is itself a link. Names
//! that start with a dot, files Windows marks hidden or system, names that are not Unicode, names
//! longer than [`folder::MAX_NAME_BYTES`] and files whose extension is not `.jsx` are left out too.
//! The files are sorted by name without regard to case, then by name.
//!
//! The limits are the Scripts folder's ([`folder::MAX_ENTRIES`], [`folder::MAX_SCRIPTS`] and
//! [`folder::MAX_SCAN`]), VectorCraft's own values. A listing that reaches one keeps what it found
//! and says so ([`Listing::truncated`]).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::folder;

/// What a Startup Scripts folder holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// The scripts, in the order they run.
    pub scripts: Vec<PathBuf>,
    /// Some scripts may be missing: a limit was reached or an entry could not be read.
    pub truncated: bool,
    /// The folder is there and could not be read, or is a link.
    pub unreadable: bool,
}

/// Does the file name `name` name a startup script: a `.jsx` file (in any case) whose name does not
/// start with a dot and has at most [`folder::MAX_NAME_BYTES`] bytes?
pub fn takes(name: &str) -> bool {
    !name.starts_with('.')
        && name.len() <= folder::MAX_NAME_BYTES
        && Path::new(name).extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("jsx"))
}

/// The startup scripts in `root`, within the Scripts folder's limits, with the time measured by
/// `clock`. A folder that does not exist gives an empty listing.
pub fn list(root: &Path, clock: folder::Clock) -> Listing {
    clock.time(|elapsed| list_with(root, &folder::Limits::default(), elapsed))
}

/// The startup scripts in `root`, within `limits` (the entries, scripts and time ones);
/// `elapsed` says how long the listing has taken.
pub fn list_with(root: &Path, limits: &folder::Limits, elapsed: &mut dyn FnMut() -> Duration) -> Listing {
    let unreadable = Listing { unreadable: true, ..Listing::default() };
    match std::fs::symlink_metadata(root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Listing::default(),
        Err(_) => return unreadable,
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => return unreadable,
        Ok(_) => {}
    }
    if root.to_str().is_none() {
        return unreadable;
    }
    let Ok(entries) = std::fs::read_dir(root) else { return unreadable };
    let mut listing = Listing::default();
    let mut names: Vec<String> = vec![];
    for (visited, entry) in entries.enumerate() {
        if visited >= limits.entries || elapsed() > limits.time {
            listing.truncated = true;
            break;
        }
        let Ok(entry) = entry else {
            listing.truncated = true;
            continue;
        };
        let Ok(name) = entry.file_name().into_string() else { continue };
        if !takes(&name) {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            listing.truncated = true;
            continue;
        };
        if !kind.is_file() || kind.is_symlink() || entry.metadata().is_ok_and(|m| folder::hidden_meta(&m)) {
            continue;
        }
        names.push(name);
    }
    names.sort_by(|a, b| (a.to_lowercase(), a).cmp(&(b.to_lowercase(), b)));
    if names.len() > limits.scripts {
        listing.truncated = true;
        names.truncate(limits.scripts);
    }
    listing.scripts = names.into_iter().map(|n| root.join(n)).collect();
    listing
}

/// Is `path` a startup script a listing of `root` can hold now: a file [`takes`] names, directly in
/// `root`, where neither `root` nor the file is a link and the file is not marked hidden or system?
pub fn holds(root: &Path, path: &Path) -> bool {
    if path.parent() != Some(root) {
        return false;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };
    takes(name) && folder::plain(root, true, false) && folder::plain(path, false, true)
}

/// The first `max` bytes of the startup script at `path`, or `None` when [`holds`] does not hold for
/// it before the read and again after it, or it cannot be read.
pub fn read(root: &Path, path: &Path, max: usize) -> Option<Vec<u8>> {
    if !holds(root, path) {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return None;
    }
    let mut bytes = vec![];
    file.take(max as u64).read_to_end(&mut bytes).ok()?;
    holds(root, path).then_some(bytes)
}

/// A run `script.startup.run` queued for the app, which starts it when the run before it ends.
#[derive(Clone, Debug, PartialEq)]
pub enum Queued {
    /// A startup script, read when its turn comes, with its `maxFuel`.
    Startup { path: PathBuf, fuel: u64 },
    /// The script that runs after the startup scripts, as `script.run`'s parameters.
    Then(serde_json::Value),
}
