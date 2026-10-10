//! The walker threads of a folder search and what they share.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use super::rules::{Refusal, Rules, Siblings, plain};

/// The most walker threads one search starts.
const MAX_THREADS: usize = 8;

/// The most walker threads all searches together have running.
const MAX_LIVE: usize = 32;

/// How often a walker with nothing to do looks again (the deadline, the end).
const IDLE: Duration = Duration::from_millis(50);

/// How many entries of a folder a walker lists between looks at the end, the deadline and the
/// folders waiting to be listed.
const CHECK_EVERY: u64 = 1024;

/// How a search ends when every walker thread stopped on a bug.
const INTERNAL: &str = "every search thread stopped on an internal error (please report this bug)";

/// What a search looks for.
pub trait Visitor: Send + Sync {
    /// Whether the regular file named `name` is to be read: asked of the files a folder lists until
    /// it has one more of them to read than [`Limits::files`].
    fn wants(&self, name: &str) -> bool;
    /// Whether the file at `path`, which [`Self::wants`] took by its name, is to be read after all,
    /// by what the file itself is. A file turned down here doesn't count among the files read.
    fn worth_reading(&self, _path: &Path) -> bool {
        true
    }
    /// The wanted items (indexes below the search's item count) the file at `path` is.
    fn items(&self, path: &Path) -> Vec<usize>;
}

/// The caps on a search.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    /// Seconds before it ends (0 to 3600; 60 when not finite).
    pub seconds: f64,
    /// Entries listed, in every folder.
    pub entries: u64,
    /// Folders waiting to be listed.
    pub folders: usize,
    /// Folders below the picked one.
    pub depth: u32,
    /// Files read.
    pub files: u64,
    /// Files kept for each item.
    pub per_item: usize,
    /// Files kept in all.
    pub hits: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { seconds: 60.0, entries: 10_000_000, folders: 1_000_000, depth: 64, files: 10_000, per_item: 10, hits: 1_000 }
    }
}

/// How a search ended.
#[derive(Clone, Debug, PartialEq)]
pub enum End {
    /// Every folder it could enter was searched.
    Done,
    /// Every item has a file.
    Found,
    /// [`Search::stop`].
    Stopped,
    /// [`Limits::seconds`] passed.
    Time,
    /// The entries listed, the folders waiting to be listed, the files read or the files kept
    /// reached their [`Limits`]. Folders deeper than [`Limits::depth`] are skipped and an item
    /// keeps at most [`Limits::per_item`] files, without ending the search.
    Limit,
    /// It couldn't start: the folder can't be read, isn't a folder or isn't one to search.
    Failed(String),
}

/// Where a search stands ([`Search::progress`]).
#[derive(Clone, Debug, Default)]
pub struct Progress {
    /// `None` while it runs.
    pub end: Option<End>,
    /// The folder searched (canonical, through [`plain`]), once it was checked. The walk reads the
    /// folder and the files below it by the paths with the `\\?\` prefix that Windows gives them.
    pub folder: PathBuf,
    /// The files found, each with the items it is, in the order found (through [`plain`]).
    pub hits: Vec<(PathBuf, Vec<usize>)>,
    /// Folders listed.
    pub folders: u64,
    /// Files listed.
    pub files: u64,
    /// Files read.
    pub read: u64,
    /// Folders not entered (other apps', the system's, hidden ones, ones too deep, ones whose
    /// contents aren't on the disk, ones whose names can't be read).
    pub skipped: u64,
    /// Folders and files that couldn't be read.
    pub unreadable: u64,
    /// How long it ran, or has been running.
    pub seconds: f64,
}

impl Progress {
    /// `searching`, `done` ([`End::Done`], [`End::Found`]), `stopped` ([`End::Stopped`],
    /// [`End::Time`], [`End::Limit`]) or `failed`.
    pub fn state(&self) -> &'static str {
        match &self.end {
            None => "searching",
            Some(End::Done | End::Found) => "done",
            Some(End::Stopped | End::Time | End::Limit) => "stopped",
            Some(End::Failed(_)) => "failed",
        }
    }

    /// Why a stopped search stopped: `stop`, `time` or `limit`.
    pub fn stopped(&self) -> Option<&'static str> {
        match &self.end {
            Some(End::Stopped) => Some("stop"),
            Some(End::Time) => Some("time"),
            Some(End::Limit) => Some("limit"),
            _ => None,
        }
    }

    /// Why a failed search failed.
    pub fn error(&self) -> Option<&str> {
        match &self.end {
            Some(End::Failed(e)) => Some(e),
            _ => None,
        }
    }
}

/// Walker threads shared out among searches: all of them together run at most `cap` threads.
pub(crate) struct Pool {
    live: AtomicUsize,
    cap: usize,
}

impl Pool {
    pub(crate) const fn new(cap: usize) -> Self {
        Self { live: AtomicUsize::new(0), cap }
    }

    /// Reserve up to `want` threads: as many as are free, none when none is.
    fn reserve(&self, want: usize) -> usize {
        let mut live = self.live.load(Ordering::SeqCst);
        loop {
            let n = want.min(self.cap.saturating_sub(live));
            if n == 0 {
                return 0;
            }
            match self.live.compare_exchange(live, live + n, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return n,
                Err(now) => live = now,
            }
        }
    }

    fn release(&self, n: usize) {
        self.live.fetch_sub(n, Ordering::SeqCst);
    }

    /// The threads running now.
    #[cfg(test)]
    pub(crate) fn live(&self) -> usize {
        self.live.load(Ordering::SeqCst)
    }
}

/// The threads of every search.
static POOL: Pool = Pool::new(MAX_LIVE);

/// The walker threads for a search: the logical CPUs less one and less the renderer's threads,
/// from 2 to 8.
pub fn threads() -> usize {
    let cpus = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
    threads_for(cpus, usize::from(vectorcraft_render::default_threads()))
}

/// [`threads`] on `cpus` logical CPUs with `render` renderer threads.
pub(crate) fn threads_for(cpus: usize, render: usize) -> usize {
    cpus.saturating_sub(1 + render).clamp(2, MAX_THREADS)
}

/// A running (or ended) search. Dropping it stops it.
pub struct Search {
    shared: Arc<Shared>,
}

impl Search {
    /// End the search ([`End::Stopped`]) unless it has ended. Walkers busy reading a folder or a
    /// file stop after it.
    pub fn stop(&self) {
        let mut w = self.shared.lock();
        self.shared.finish(&mut w, End::Stopped);
    }

    /// Where the search stands. Past its deadline it has ended ([`End::Time`]), even while a walker
    /// is still waiting for a slow folder or file.
    pub fn progress(&self) -> Progress {
        let s = &self.shared;
        let mut w = s.lock();
        if w.end.is_none() && Instant::now() >= s.deadline {
            s.finish(&mut w, End::Time);
        }
        Progress {
            end: w.end.clone(),
            folder: plain(w.folder.clone()),
            hits: w.hits.iter().map(|(path, items)| (plain(path.clone()), items.clone())).collect(),
            folders: w.folders,
            files: w.files,
            read: s.read.load(Ordering::Relaxed).min(s.limits.files),
            skipped: w.skipped,
            unreadable: w.unreadable,
            seconds: if w.end.is_some() { w.seconds } else { s.started.elapsed().as_secs_f64() },
        }
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Search `folder` and its subfolders on up to `threads` walker threads (at most 8, as many as the
/// process-wide cap leaves) for the `items` a `visitor` looks for, where `rules` allow (this
/// computer's when `None`). Returns at once; the folder is checked on a walker thread.
pub fn start(
    folder: PathBuf,
    rules: Option<Rules>,
    limits: Limits,
    threads: usize,
    items: usize,
    visitor: Arc<dyn Visitor>,
) -> Result<Search, String> {
    start_in(&POOL, folder, rules, limits, threads, items, visitor)
}

/// [`start`] with the threads of `pool`.
pub(crate) fn start_in(
    pool: &'static Pool,
    folder: PathBuf,
    rules: Option<Rules>,
    limits: Limits,
    threads: usize,
    items: usize,
    visitor: Arc<dyn Visitor>,
) -> Result<Search, String> {
    if cfg!(target_arch = "wasm32") {
        return Err("searching a folder isn't available on the web".into());
    }
    // The walkers follow no links, so a folder automation may read holds all they read.
    crate::file_access::check_read(&folder.to_string_lossy())?;
    let seconds = if limits.seconds.is_finite() { limits.seconds.clamp(0.0, 3600.0) } else { 60.0 };
    let n = pool.reserve(threads.clamp(1, MAX_THREADS));
    if n == 0 {
        return Err("the search couldn't start: try again in a moment".into());
    }
    let started = Instant::now();
    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        entries: AtomicU64::new(0),
        read: AtomicU64::new(0),
        started,
        deadline: started + Duration::from_secs_f64(seconds),
        limits,
        items,
        given: rules,
        rules: OnceLock::new(),
        visitor,
        walk: Mutex::new(Walk { root: Some(folder), alive: n, found: vec![0; items], ..Walk::default() }),
        wake: Condvar::new(),
        pool,
    });
    let mut spawned = 0;
    for _ in 0..n {
        let s = shared.clone();
        let thread = std::thread::Builder::new().name("vectorcraft-find".into()).spawn(move || {
            let _alive = Alive(s.clone());
            // A bug in the walk ends this walker and the others go on; when every walker ends this
            // way, the search fails ([`Alive`]).
            let _ = crate::guard::catch_panic(|| walker(&s));
        });
        if thread.is_ok() {
            spawned += 1;
        }
    }
    if spawned < n {
        pool.release(n - spawned);
        let mut w = shared.lock();
        w.alive = w.alive.saturating_sub(n - spawned);
        if spawned == 0 {
            return Err("the search couldn't start: try again in a moment".into());
        }
        if w.alive == 0 && w.end.is_none() {
            shared.finish(&mut w, End::Failed(INTERNAL.into()));
        }
    }
    Ok(Search { shared })
}

/// What a search's walkers share.
struct Shared {
    /// Set once the search ended: walkers stop listing and reading.
    stop: AtomicBool,
    entries: AtomicU64,
    read: AtomicU64,
    started: Instant,
    deadline: Instant,
    limits: Limits,
    items: usize,
    /// The rules given to [`start`], else this computer's, read by the first walker.
    given: Option<Rules>,
    rules: OnceLock<Rules>,
    visitor: Arc<dyn Visitor>,
    walk: Mutex<Walk>,
    /// Woken when folders are queued, a walker finishes a folder, or the search ends.
    wake: Condvar,
    pool: &'static Pool,
}

/// The state walkers change under the lock.
#[derive(Default)]
struct Walk {
    /// The picked folder, until a walker takes it to check it.
    root: Option<PathBuf>,
    /// A walker is checking the picked folder.
    preparing: bool,
    folder: PathBuf,
    /// Folders waiting to be listed, with their depth below the picked one.
    stack: Vec<(PathBuf, u32)>,
    /// Folders being listed (and their candidates read) now.
    listing: usize,
    /// Walker threads still running.
    alive: usize,
    end: Option<End>,
    seconds: f64,
    hits: Vec<(PathBuf, Vec<usize>)>,
    /// Files kept for each item.
    found: Vec<usize>,
    folders: u64,
    files: u64,
    skipped: u64,
    unreadable: u64,
    /// The home folder was queued from a system's root ([`Rules::home_below`]).
    home_queued: bool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Walk> {
        self.walk.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// End the search as `end` unless it has ended, and wake every walker to see it.
    fn finish(&self, w: &mut Walk, end: End) {
        if w.end.is_none() {
            w.end = Some(end);
            w.seconds = self.started.elapsed().as_secs_f64();
        }
        self.stop.store(true, Ordering::Relaxed);
        self.wake.notify_all();
    }

    fn end_with(&self, end: End) {
        let mut w = self.lock();
        self.finish(&mut w, end);
    }

    /// Whether to stop listing and reading: the search ended or its time is up.
    fn should_stop(&self) -> bool {
        self.stop.load(Ordering::Relaxed) || Instant::now() >= self.deadline
    }

    /// Whether the folders waiting to be listed and `more` folders found in the folder being listed
    /// come to more than [`Limits::folders`], which ends the search ([`End::Limit`]).
    fn too_many_folders(&self, more: usize) -> bool {
        let mut w = self.lock();
        let over = w.stack.len().saturating_add(more) > self.limits.folders;
        if over {
            self.finish(&mut w, End::Limit);
        }
        over
    }

    /// Check the picked folder, then queue it.
    fn prepare(&self, root: &Path) {
        let rules = self.rules.get_or_init(|| self.given.clone().unwrap_or_else(Rules::system));
        let path = root.display();
        let checked = match std::fs::canonicalize(root) {
            Err(e) => Err(format!("{path}: {e}")),
            Ok(folder) if !folder.is_dir() => Err(format!("{path} is not a folder")),
            Ok(folder) => match rules.refusal(&folder) {
                None => Ok(folder),
                Some(Refusal::Shared) => Err(format!("VectorCraft doesn't search the Shared or Public folder in Users: {path}")),
                Some(Refusal::Apps) => Err(format!("VectorCraft doesn't search other apps' or the system's folders: {path}")),
            },
        };
        let mut w = self.lock();
        match checked {
            Ok(folder) => {
                w.folder = folder.clone();
                w.stack.push((folder, 0));
            }
            Err(e) => self.finish(&mut w, End::Failed(e)),
        }
    }

    /// List one folder: queue its subfolders, then read the files [`Visitor::wants`] accepts.
    fn list(&self, dir: &Path, depth: u32) {
        let Some(rules) = self.rules.get() else { return };
        let (mut skipped, mut unreadable, mut files) = (0, 0, 0);
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                let mut w = self.lock();
                // The picked folder itself: the search can't run, and says why.
                if depth == 0 {
                    self.finish(&mut w, End::Failed(format!("{}: {e}", plain(dir.to_path_buf()).display())));
                } else {
                    w.unreadable += 1;
                    w.folders += 1;
                }
                return;
            }
        };
        let (mut folders, mut siblings, mut candidates) = (vec![], Siblings::default(), vec![]);
        for (n, entry) in entries.enumerate() {
            if n as u64 % CHECK_EVERY == CHECK_EVERY - 1 && (self.should_stop() || self.too_many_folders(folders.len())) {
                break;
            }
            if self.entries.fetch_add(1, Ordering::Relaxed) >= self.limits.entries {
                self.end_with(End::Limit);
                break;
            }
            let Ok(entry) = entry else {
                unreadable += 1;
                break;
            };
            let Ok(kind) = entry.file_type() else {
                unreadable += 1;
                continue;
            };
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                // A name that isn't Unicode: its folder is left out, its file can't be wanted.
                if kind.is_dir() {
                    skipped += 1;
                }
                continue;
            };
            if kind.is_dir() || kind.is_symlink() {
                siblings.note(name);
            }
            // Links are neither followed nor read.
            if kind.is_dir() {
                folders.push((entry, name.to_string()));
            } else if kind.is_file() {
                files += 1;
                // At most one file more than the search reads is kept to be read.
                if (candidates.len() as u64) <= self.limits.files && self.visitor.wants(name) && self.visitor.worth_reading(&entry.path()) {
                    if hidden_and_offline(&entry).1 {
                        // Its contents are in the cloud: counted as unreadable, never read.
                        unreadable += 1;
                    } else {
                        candidates.push(entry);
                    }
                }
            }
        }
        // The folder is told once for all its folders.
        let listed = rules.listed(dir, siblings);
        let mut queue = vec![];
        for (n, (entry, name)) in folders.into_iter().enumerate() {
            if n as u64 % CHECK_EVERY == CHECK_EVERY - 1 && self.should_stop() {
                break;
            }
            let path = entry.path();
            let (hidden, offline) = hidden_and_offline(&entry);
            if depth >= self.limits.depth || name.starts_with('.') || hidden || offline || rules.never_in(&listed, &name) || rules.is_refused(&path) {
                skipped += 1;
                continue;
            }
            queue.push((path, depth + 1));
        }
        let home = rules.home_below(&listed);
        {
            let mut w = self.lock();
            w.files += files;
            w.skipped += skipped;
            w.unreadable += unreadable;
            if w.end.is_none() {
                if w.stack.len() + queue.len() > self.limits.folders {
                    self.finish(&mut w, End::Limit);
                } else {
                    w.stack.extend(queue);
                    if let Some(home) = home.filter(|_| !w.home_queued) {
                        w.home_queued = true;
                        w.stack.push((home, 1));
                    }
                    self.wake.notify_all();
                }
            }
        }
        let mut unreadable = 0;
        for entry in candidates {
            if self.should_stop() {
                break;
            }
            if self.read.fetch_add(1, Ordering::Relaxed) >= self.limits.files {
                self.end_with(End::Limit);
                break;
            }
            let path = entry.path();
            match crate::guard::catch_panic(|| self.visitor.items(&path)) {
                Ok(items) if !items.is_empty() => self.keep(path, items),
                Ok(_) => {}
                // A file that trips a bug: as one that can't be read, and the search goes on.
                Err(_) => unreadable += 1,
            }
        }
        let mut w = self.lock();
        w.folders += 1;
        w.unreadable += unreadable;
    }

    /// Keep a file the visitor gave `items` for, for those of its items that take more files, as
    /// soon as it is read: a file read as the search ends is kept too. The search ends at the most
    /// files it keeps, and once every item has a file.
    fn keep(&self, path: PathBuf, mut items: Vec<usize>) {
        let mut w = self.lock();
        items.sort_unstable();
        items.dedup();
        items.retain(|i| w.found.get(*i).is_some_and(|n| *n < self.limits.per_item));
        if items.is_empty() || w.hits.len() >= self.limits.hits {
            return;
        }
        for i in &items {
            if let Some(n) = w.found.get_mut(*i) {
                *n += 1;
            }
        }
        w.hits.push((path, items));
        if w.hits.len() >= self.limits.hits {
            self.finish(&mut w, End::Limit);
        }
        if self.items > 0 && w.found.iter().all(|n| *n > 0) {
            self.finish(&mut w, End::Found);
        }
    }
}

/// What a walker does next.
enum Job {
    Prepare(PathBuf),
    List(PathBuf, u32),
}

/// A walker thread: check the picked folder, or list the next folder, until the search ends.
fn walker(s: &Shared) {
    loop {
        let job = {
            let mut w = s.lock();
            loop {
                if w.end.is_some() {
                    return;
                }
                if Instant::now() >= s.deadline {
                    s.finish(&mut w, End::Time);
                    return;
                }
                if let Some(root) = w.root.take() {
                    w.preparing = true;
                    break Job::Prepare(root);
                }
                if let Some((dir, depth)) = w.stack.pop() {
                    w.listing += 1;
                    break Job::List(dir, depth);
                }
                if !w.preparing && w.listing == 0 {
                    s.finish(&mut w, End::Done);
                    return;
                }
                w = s.wake.wait_timeout(w, IDLE).map_or_else(|e| e.into_inner().0, |(w, _)| w);
            }
        };
        // The guard lowers `preparing` or `listing` when the job ends, on a bug too.
        let _busy = Busy(s, matches!(job, Job::Prepare(_)));
        let done = crate::guard::catch_panic(|| match &job {
            Job::Prepare(root) => s.prepare(root),
            Job::List(dir, depth) => s.list(dir, *depth),
        });
        if done.is_err() {
            let mut w = s.lock();
            match job {
                Job::Prepare(_) => s.finish(&mut w, End::Failed(INTERNAL.into())),
                Job::List(..) => w.unreadable += 1,
            }
        }
    }
}

/// A walker's job in progress: preparing (`true`) or listing a folder.
struct Busy<'a>(&'a Shared, bool);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        let mut w = self.0.lock();
        if self.1 {
            w.preparing = false;
        } else {
            w.listing = w.listing.saturating_sub(1);
        }
        self.0.wake.notify_all();
    }
}

/// A running walker thread, counted out of the search and the pool when it ends.
struct Alive(Arc<Shared>);

impl Drop for Alive {
    fn drop(&mut self) {
        let mut w = self.0.lock();
        w.alive = w.alive.saturating_sub(1);
        if w.alive == 0 && w.end.is_none() {
            self.0.finish(&mut w, End::Failed(INTERNAL.into()));
        }
        drop(w);
        self.0.pool.release(1);
    }
}

/// Whether a folder or file is hidden by its flags (macOS `UF_HIDDEN`; Windows hidden or system),
/// and whether its contents are in the cloud rather than on the disk (macOS `SF_DATALESS`; Windows
/// offline or recalled on access).
#[cfg(target_os = "macos")]
fn hidden_and_offline(entry: &std::fs::DirEntry) -> (bool, bool) {
    use std::os::macos::fs::MetadataExt;
    const UF_HIDDEN: u32 = 0x8000;
    const SF_DATALESS: u32 = 0x4000_0000;
    entry.metadata().map_or((false, false), |m| (m.st_flags() & UF_HIDDEN != 0, m.st_flags() & SF_DATALESS != 0))
}

#[cfg(windows)]
fn hidden_and_offline(entry: &std::fs::DirEntry) -> (bool, bool) {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    const OFFLINE: u32 = 0x1000;
    const RECALL_ON_OPEN: u32 = 0x4_0000;
    const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
    entry.metadata().map_or((false, false), |m| {
        let a = m.file_attributes();
        (a & (HIDDEN | SYSTEM) != 0, a & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0)
    })
}

#[cfg(not(any(target_os = "macos", windows)))]
fn hidden_and_offline(_: &std::fs::DirEntry) -> (bool, bool) {
    (false, false)
}
