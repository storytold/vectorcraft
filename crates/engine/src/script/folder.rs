//! The Scripts folder: the `.js` and `.jsx` files File › Scripts lists, with each subfolder as a
//! submenu ([`list`]), and the read of one of them when it is chosen ([`read`]).
//!
//! A listing reads names and file types only, never a script's text. It follows no link: a symbolic
//! link or a Windows junction is left out, whether it is a file, a folder or the Scripts folder
//! itself, so a listed script is a plain file in plain folders under the Scripts folder. Names that
//! start with a dot, files and folders Windows marks hidden or system, names that are not Unicode
//! and names longer than [`MAX_NAME_BYTES`] are left out too, and so is a folder with no script in
//! it.
//!
//! The values here are VectorCraft's own limits: [`MAX_ENTRIES`], [`MAX_SCRIPTS`], [`MAX_FOLDERS`],
//! [`MAX_DEPTH`], [`MAX_NAME_BYTES`], [`MAX_LABEL_CHARS`] and [`MAX_SCAN`]. A listing that reaches
//! one of them, or that cannot read a subfolder or an entry of a folder, keeps what it found and
//! says so ([`Listing::truncated`]).

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use crate::cmd::scripts::runs;

/// The most folder entries one listing looks at, in the Scripts folder and its subfolders together.
pub const MAX_ENTRIES: usize = 2048;
/// The most scripts one listing holds.
pub const MAX_SCRIPTS: usize = 512;
/// The most subfolders one listing reads.
pub const MAX_FOLDERS: usize = 128;
/// How many levels of subfolders a listing reads below the Scripts folder.
pub const MAX_DEPTH: usize = 4;
/// The longest file or folder name a listing takes, in UTF-8 bytes.
pub const MAX_NAME_BYTES: usize = 255;
/// The most characters of a label; a longer one ends in `…`.
pub const MAX_LABEL_CHARS: usize = 128;
/// How long one listing may take before it stops with what it found.
pub const MAX_SCAN: Duration = Duration::from_millis(250);

/// The limits of one listing.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub entries: usize,
    pub scripts: usize,
    pub folders: usize,
    pub depth: usize,
    pub time: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { entries: MAX_ENTRIES, scripts: MAX_SCRIPTS, folders: MAX_FOLDERS, depth: MAX_DEPTH, time: MAX_SCAN }
    }
}

/// One row of a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A script: its label (the file name without its extension) and the file's path as the folder
    /// gave it.
    Script { label: String, path: PathBuf },
    /// A subfolder with at least one script in it or below it: its label (the folder's name) and
    /// its rows.
    Folder { label: String, entries: Vec<Entry> },
}

/// What a Scripts folder holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// The rows of the folder itself, sorted ([`list_with`]).
    pub entries: Vec<Entry>,
    /// Some scripts may be missing: a limit was reached or a subfolder could not be read.
    pub truncated: bool,
    /// The folder is there and could not be read, or is a link.
    pub unreadable: bool,
}

impl Listing {
    /// Every script with the labels of the folders it is in, in menu order.
    pub fn scripts(&self) -> Vec<(Vec<&str>, &str, &Path)> {
        fn walk<'a>(entries: &'a [Entry], folders: &mut Vec<&'a str>, out: &mut Vec<(Vec<&'a str>, &'a str, &'a Path)>) {
            for e in entries {
                match e {
                    Entry::Script { label, path } => out.push((folders.clone(), label, path)),
                    Entry::Folder { label, entries } => {
                        folders.push(label);
                        walk(entries, folders, out);
                        folders.pop();
                    }
                }
            }
        }
        let mut out = vec![];
        walk(&self.entries, &mut vec![], &mut out);
        out
    }
}

/// What the folder `root` holds, within the default [`Limits`]. A folder that does not exist gives
/// an empty listing.
pub fn list(root: &Path) -> Listing {
    let start = std::time::Instant::now();
    list_with(root, &Limits::default(), &mut || start.elapsed())
}

/// What the folder `root` holds, within `limits`; `elapsed` says how long the listing has taken.
/// Each level is sorted by label without regard to case, then by label, then by file name, with
/// scripts and subfolders mixed. Two rows of one level never share a label unless both were cut:
/// a script whose label another row has shows its whole file name.
pub fn list_with(root: &Path, limits: &Limits, elapsed: &mut dyn FnMut() -> Duration) -> Listing {
    let unreadable = Listing { unreadable: true, ..Listing::default() };
    match std::fs::symlink_metadata(root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Listing::default(),
        Err(_) => return unreadable,
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => return unreadable,
        Ok(_) => {}
    }
    // Paths leave as text (menu params, `script.listInstalled`).
    if root.to_str().is_none() {
        return unreadable;
    }
    let mut scan = Scan { limits, elapsed, visited: 0, scripts: 0, folders: 0, truncated: false, stopped: false };
    match scan.level(root, 0) {
        Some(entries) => Listing { entries, truncated: scan.truncated, unreadable: false },
        None => unreadable,
    }
}

/// A script or subfolder found in one folder, before the subfolders are read.
struct Found {
    name: String,
    label: String,
    folder: bool,
}

struct Scan<'a> {
    limits: &'a Limits,
    elapsed: &'a mut dyn FnMut() -> Duration,
    visited: usize,
    scripts: usize,
    folders: usize,
    truncated: bool,
    /// The entry or time limit was reached: nothing more is read.
    stopped: bool,
}

impl Scan<'_> {
    /// The rows of the folder `dir`, which is `depth` levels below the Scripts folder; `None` when
    /// it cannot be read.
    fn level(&mut self, dir: &Path, depth: usize) -> Option<Vec<Entry>> {
        let mut found: Vec<Found> = vec![];
        for entry in std::fs::read_dir(dir).ok()? {
            if self.visited >= self.limits.entries || (self.elapsed)() > self.limits.time {
                (self.truncated, self.stopped) = (true, true);
                break;
            }
            self.visited += 1;
            let Ok(entry) = entry else {
                self.truncated = true;
                continue;
            };
            let Ok(name) = entry.file_name().into_string() else { continue };
            let Ok(kind) = entry.file_type() else {
                self.truncated = true;
                continue;
            };
            if name.len() > MAX_NAME_BYTES || name.starts_with('.') || kind.is_symlink() || hidden(&entry) {
                continue;
            }
            if kind.is_dir() {
                found.push(Found { label: shown(&name), name, folder: true });
            } else if kind.is_file() && runs(&name) {
                if self.scripts >= self.limits.scripts {
                    self.truncated = true;
                    continue;
                }
                self.scripts += 1;
                let stem = name.rsplit_once('.').map_or(name.as_str(), |(stem, _)| stem);
                found.push(Found { label: shown(stem), name, folder: false });
            }
        }
        // A script whose label another row of this folder has shows its whole file name, until no
        // label is shared. Each pass gives at least one more script its whole file name, and names
        // are unique within a folder, so this ends within `found.len()` passes.
        loop {
            let mut count = std::collections::HashMap::<String, usize>::new();
            for f in &found {
                *count.entry(f.label.clone()).or_default() += 1;
            }
            let mut renamed = false;
            for f in found.iter_mut().filter(|f| !f.folder) {
                if count.get(&f.label).is_some_and(|n| *n > 1) {
                    let whole = shown(&f.name);
                    if f.label != whole {
                        f.label = whole;
                        renamed = true;
                    }
                }
            }
            if !renamed {
                break;
            }
        }
        found.sort_by(|a, b| (a.label.to_lowercase(), &a.label, &a.name).cmp(&(b.label.to_lowercase(), &b.label, &b.name)));
        let mut out = Vec::with_capacity(found.len());
        for f in found {
            let path = dir.join(&f.name);
            if !f.folder {
                out.push(Entry::Script { label: f.label, path });
                continue;
            }
            if self.stopped {
                continue;
            }
            if depth >= self.limits.depth || self.folders >= self.limits.folders {
                self.truncated = true;
                continue;
            }
            self.folders += 1;
            match self.level(&path, depth + 1) {
                Some(entries) if entries.is_empty() => {}
                Some(entries) => out.push(Entry::Folder { label: f.label, entries }),
                None => self.truncated = true,
            }
        }
        Some(out)
    }
}

/// `name` as a menu shows it: control and bidirectional formatting characters written as escapes,
/// and at most [`MAX_LABEL_CHARS`] characters.
fn shown(name: &str) -> String {
    vectorcraft_plugins::permissions::display_text(name, MAX_LABEL_CHARS)
}

/// Does Windows mark `entry` hidden or system?
fn hidden(entry: &std::fs::DirEntry) -> bool {
    entry.metadata().is_ok_and(|m| hidden_meta(&m))
}

/// Does Windows mark the file or folder of `m` hidden or system?
#[cfg(windows)]
fn hidden_meta(m: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    const HIDDEN_OR_SYSTEM: u32 = 0x2 | 0x4;
    m.file_attributes() & HIDDEN_OR_SYSTEM != 0
}

/// Elsewhere a hidden file is one whose name starts with a dot, which the listing leaves out by name.
#[cfg(not(windows))]
fn hidden_meta(_: &std::fs::Metadata) -> bool {
    false
}

/// Is `path` a folder (`folder`) or a file, and not a link? With `shown`, also not one Windows
/// marks hidden or system, which a listing leaves out.
fn plain(path: &Path, folder: bool, shown: bool) -> bool {
    std::fs::symlink_metadata(path)
        .is_ok_and(|m| !m.file_type().is_symlink() && (if folder { m.is_dir() } else { m.is_file() }) && !(shown && hidden_meta(&m)))
}

/// Is `path` a script a listing of `root` can hold now: a `.js` or `.jsx` file at most
/// [`MAX_DEPTH`] folders below `root`, where `root`, each folder on the way and the file are there
/// and none is a link, and neither the file nor a folder below `root` is marked hidden or system?
pub fn holds(root: &Path, path: &Path) -> bool {
    let Ok(rest) = path.strip_prefix(root) else { return false };
    let names: Option<Vec<&str>> = rest
        .components()
        .map(|c| match c {
            Component::Normal(name) => name.to_str().filter(|n| !n.starts_with('.') && n.len() <= MAX_NAME_BYTES),
            _ => None,
        })
        .collect();
    let Some(names) = names else { return false };
    let Some((file, folders)) = names.split_last() else { return false };
    if folders.len() > MAX_DEPTH || !runs(file) || !plain(root, true, false) {
        return false;
    }
    let mut at = root.to_path_buf();
    for folder in folders {
        at.push(folder);
        if !plain(&at, true, true) {
            return false;
        }
    }
    at.push(file);
    plain(&at, false, true)
}

/// The first `max` bytes of the script at `path`, or `None` when [`holds`] does not hold for it
/// before the read and again after it, or it cannot be read. The two checks leave no time in which
/// a link stood in for the file or one of its folders for the whole read; a program that swaps one
/// in and back out between a check and the open can still have another file read.
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
