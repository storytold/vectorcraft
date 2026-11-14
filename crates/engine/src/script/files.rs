//! `File` and `Folder`: a script's objects for files and folders, as the help pages describe them
//! (<https://extendscript.docsforadobe.dev/file-system-access/using-file-and-folder-objects/>,
//! <https://extendscript.docsforadobe.dev/file-system-access/file-object/> and
//! <https://extendscript.docsforadobe.dev/file-system-access/folder-object/>). The classes are
//! built-ins of the realm ([`Realm::define_file_classes`]); the host functions here make every
//! path check and every read.
//!
//! An object holds an absolute path: a relative path is taken from `Folder.current`, `~` is the home
//! folder, `%xx` escapes are read as UTF-8, and on Windows both `/` and `\` separate names and
//! `/c/…` names drive C. Naming a path does not look anything up.
//!
//! VectorCraft's own rules:
//!
//! - A script reads only inside its [`ScriptScope`]: a script run from a file reads in that file's
//!   folder and the folders below it, and a script given as text reads nothing. A path whose names
//!   lie outside, a Windows network or device path (`\\host\share`, `\\?\`, `\\.\`), and a path that
//!   leads out through a link are refused before or instead of opening anything: `exists` is false,
//!   the dates are `null`, `getFiles` gives `null`, and `error` is [`ACCESS_DENIED`]. The automation
//!   read roots apply as well, so a run with roots in force reads only inside both.
//! - `File(path)` without `new` gives a `Folder` for an existing folder, and `Folder(path)` gives a
//!   `File` for an existing file, only where the path may be read.
//! - Opening reads the whole file, at most [`MAX_FILE_BYTES`], checked from its size before any of
//!   it is read; a run reads at most [`MAX_RUN_BYTES`] in all and ends past that, and has at most
//!   [`MAX_OPEN`] files open at once. A file opened again after its objects closed counts only the
//!   bytes it has grown by; a file open in another object counts again.
//! - The encoding of an opened file is the one its byte order mark gives (UTF-8, UTF-16LE or
//!   UTF-16BE), otherwise the one the script set, otherwise UTF-8 when the bytes are valid UTF-8,
//!   otherwise CP1252. `read`, `readln` and `readch` count characters and `seek` and `tell` bytes;
//!   `readln` and `readch` take CR, LF, CRLF and LFCR as one line end, and `readch` gives a line end
//!   as `"\n"`. A character the encoding cannot decode fails the read with [`UNDECODABLE`] and
//!   leaves the position where it was.
//! - `getFiles` lists at most [`MAX_ENTRIES`] entries, sorted by name, and looks at most
//!   [`crate::file_access::MAX_LISTED`] names; a mask matches `*` and `?`, without regard to case
//!   on Windows and macOS, in at most [`MAX_MASK_STEPS`] steps for one listing.
//! - A path longer than [`MAX_PATH_CHARS`] characters is refused before it is split into names.
//! - A script writes only in its run's temporary folder (`Folder.temp`, made when it is first
//!   named and deleted when the run ends), its own data folder (`Folder.userData`: for a script run
//!   from a file a folder of its own beside the settings, kept between runs, and for a script given
//!   as text a folder inside the temporary one), and the files and folders a person granted it.
//!   Its own folder is only read. The automation write roots apply as well, and the folder that
//!   holds the settings, with the Scripts and Plug-ins folders, is kept out of every grant but the
//!   script's data folder.
//! - In the app, `File.openDialog`, `File.saveDialog`, `Folder.selectDialog` and the objects'
//!   `openDlg`, `saveDlg` and `selectDlg` wait for the person, who picks a file or a folder and then
//!   allows the script that one, once or always, or denies it ([`ScriptPicker`]). A picked file is
//!   granted for reading (open) or for reading and writing (save), and a picked folder for reading
//!   and writing with everything inside it. A `filter` that is a function is called with each file
//!   picked, and the files it refuses are left out of what the dialog gives. Always keeps the grant in the grants file beside the
//!   settings, for that script file with that text (its SHA-256): a changed or moved script has none,
//!   and a kept grant whose path now leads elsewhere through a link is not used.
//!   The grant also holds the SHA-256 of each source the run evaluated (a file `$.evalFile` ran, a
//!   string given to `eval` or `Function`), up to the run's end: a later run that evaluates other
//!   code, or whose named engine holds code its earlier runs compiled that the grant was not kept
//!   for, loses the grant before that code runs. A run with nobody to ask gets `null` and
//!   [`NO_PICKER`].
//! - The commands a script runs with `vc.run` read and write only in the places above and in the
//!   files of the open documents ([`crate::file_access::ScriptGate`]).
//! - `open('w')` empties the file at once; what a script writes is kept in memory, at most
//!   [`MAX_FILE_BYTES`] a file, and written when it closes the file or the run ends; a run writes at
//!   most [`MAX_RUN_BYTES`] in all and ends past that. `write` encodes in the file's encoding (an
//!   ASCII or ISO-8859-1 file refuses a character it cannot hold, BINARY writes each character's low
//!   byte) and `writeln` adds the `lineFeed`'s line end. `copy` replaces a file that is there,
//!   `remove` deletes at once and takes a folder only when it is empty, `rename` takes a name without
//!   a folder and refuses one in use, and `length` and `readonly` change a file (on Unix, clearing
//!   `readonly` gives back the write bits the run cleared, or else the owner's). Each object that has
//!   a file open holds its own bytes, so what `length` adds and what `copy` writes count once for the
//!   file and once more for each such object. `write` joins its values only for an open file and
//!   only up to twice [`MAX_FILE_BYTES`] UTF-16 code units. Each file written is named in the
//!   report's `written` (at most [`super::MAX_WRITTEN`]), also after a run that failed.
//! - Hidden settings, aliases and `execute` are not available: they return `false` and set `error`.
//!   `execute` is counted in the report's `fileExecutes`.
//! - In a wasm build every lookup fails with [`NO_WEB`].

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use vectorcraft_js::{Data, HostError, HostItem, Realm, RunError};

use super::{Host, bump};
use crate::file_access::{ScriptRead, ScriptScope, path_inside};

/// The most bytes of one file a script opens.
pub const MAX_FILE_BYTES: usize = 8 << 20;
/// The most files a run remembers the bytes counted for, so opening one again counts only what it
/// grew by; a file past them counts in full each time it is opened.
const MAX_CHARGED: usize = 4096;
/// The most bytes of files one run opens, all of them counted together.
pub const MAX_RUN_BYTES: usize = 32 << 20;
/// The most entries `getFiles` gives.
pub const MAX_ENTRIES: usize = 2048;
/// The most files a run has open at once.
pub const MAX_OPEN: usize = 256;
/// The most characters of a path a script gives.
pub const MAX_PATH_CHARS: usize = 32_767;
/// The most steps one `getFiles` listing spends matching its mask against names.
pub const MAX_MASK_STEPS: usize = 1 << 24;

pub(crate) const ACCESS_DENIED: &str = "Script file access denied";
pub(crate) const TOO_LARGE: &str = "Script file exceeds 8 MiB";
pub(crate) const NOT_OPEN: &str = "Script file is not open";
pub(crate) const UNDECODABLE: &str = "Script file cannot be decoded";
pub(crate) const NOT_REGULAR: &str = "Script file is not a regular file";
pub(crate) const NOT_FOUND: &str = "Script file does not exist";
pub(crate) const READ_FAILED: &str = "Script file could not be read";
pub(crate) const LISTING_TOO_LARGE: &str = "Script folder has more than 2,048 entries";
pub(crate) const BAD_MODE: &str = "Script file open mode must be r, w, e or a";
pub(crate) const NO_PICKER: &str = "Script file dialogs need a person to answer them";
pub(crate) const NO_EXECUTE: &str = "Script files cannot be opened in another application";
pub(crate) const NO_WEB: &str = "Script file access is unavailable on the web";
pub(crate) const TOO_MANY_OPEN: &str = "Script has 256 files open";
pub(crate) const PATH_TOO_LONG: &str = "Script file path exceeds 32,767 characters";
pub(crate) const MASK_TOO_COSTLY: &str = "Script folder mask takes too many steps to match";
pub(crate) const NOT_WRITABLE: &str = "Script file is not open for writing";
pub(crate) const UNENCODABLE: &str = "Script file cannot encode the text";
pub(crate) const WRITE_FAILED: &str = "Script file could not be written";
pub(crate) const NOT_EMPTY: &str = "Script folder is not empty";
pub(crate) const NAME_TAKEN: &str = "Script file name is already in use";
pub(crate) const BAD_NAME: &str = "Script file name must be a name without a folder";
pub(crate) const NO_ALIAS: &str = "Script files cannot make aliases";
pub(crate) const NO_HIDDEN: &str = "Script file hidden setting cannot be changed";
pub(crate) const PROTECTED: &str = "Script file access to the app's own folders is not allowed";
pub(crate) const NOT_SAVED: &str = "Script file permission could not be saved";
pub(crate) const NO_ALWAYS: &str = "Script file permission can be kept only for a script run from a file";
const OVER_RUN_BYTES: &str = "the script opened more file data than one run may open (32 MiB)";
const OVER_RUN_WRITES: &str = "the script wrote more file data than one run may write (32 MiB)";
/// The most grants the grants file keeps.
pub const MAX_STORED_GRANTS: usize = 4096;
/// The most bytes of the grants file that are read.
const MAX_STORE_BYTES: usize = 4 << 20;
/// The name of the grants file, beside the folder of the scripts' data folders.
pub const GRANTS_FILE: &str = "Script Grants.json";
/// The most distinct sources a run evaluates whose SHA-256 is kept; past them no kept grant is in
/// force for the rest of the run, and the run's grants keep the sources counted before.
pub const MAX_RUN_CODE: usize = 256;

/// The SHA-256 of `units`, source text a script evaluated, as UTF-16 code units in little-endian
/// order.
pub(crate) fn code_hash(units: &[u16]) -> String {
    let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    vectorcraft_plugins::permissions::sha256_hex(&bytes)
}

/// The id of the next file a script opens. Ids are unique across runs, so a `File` that a named
/// engine kept from an earlier run does not name a file a later run opened.
static NEXT_HANDLE: AtomicU32 = AtomicU32::new(1);

/// The number in the name of the next run's temporary folder.
static NEXT_TEMP: AtomicU32 = AtomicU32::new(1);

/// Which file dialog a script opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKind {
    /// `File.openDialog` and `openDlg`: a file to read.
    Open,
    /// `File.saveDialog` and `saveDlg`: a file to write, which may not exist yet.
    Save,
    /// `Folder.selectDialog` and `selectDlg`: a folder to read and write.
    Folder,
}

impl PickerKind {
    /// The kind's name: `open`, `save` or `folder`.
    pub fn id(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Save => "save",
            Self::Folder => "folder",
        }
    }
}

/// A file dialog a running script waits at: the person picks a file or a folder, then allows the
/// script that one, once or always, or denies it ([`PickAnswer`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptPicker {
    pub kind: PickerKind,
    /// The script's prompt, empty when it gave none.
    pub prompt: String,
    /// An open dialog that may pick more than one file.
    pub multiple: bool,
    /// The file or folder the dialog starts at, when the script's object gave one.
    pub start: Option<String>,
    /// The file types the dialog shows, from the script's filter ([`dialog_filters`]): each one's
    /// name and extensions (`*` for every file). Empty: every file.
    pub filters: Vec<(String, Vec<String>)>,
    /// Whether the grant can be kept for later runs: the script was run from a file, and the app
    /// has a settings folder for the grants file.
    pub can_always: bool,
}

/// The answer to a [`ScriptPicker`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickAnswer {
    /// Cancel or Deny: the script gets `null`.
    Deny,
    /// The paths picked, allowed for this run, or with `always` for later runs of the same script too.
    Allow { paths: Vec<String>, always: bool },
}

/// What a run's `File` and `Folder` objects start with.
#[derive(Clone, Debug, Default)]
pub(crate) struct Setup {
    /// The run is in the app, where a person answers its file dialogs.
    pub framed: bool,
    /// The file the script was read from.
    pub script: Option<String>,
    /// The script's text with the files it includes, whose SHA-256 the grants it keeps are for.
    pub source: String,
    /// The folder that holds each script's data folder ([`crate::Session::attach_script_data`]).
    pub data_dir: Option<std::path::PathBuf>,
    /// The code the run's named engine kept from its earlier runs: the SHA-256 of each source
    /// ([`code_hash`] for evaluated code), and whether there was more than it keeps.
    pub engine_code: (Vec<String>, bool),
}

/// A grant the grants file keeps that is in force for the run: the real path, whether it may be
/// written, whether it is a folder's, and the evaluated code the script ran in the run that kept it.
struct Stored {
    path: std::path::PathBuf,
    write: bool,
    folder: bool,
    code: std::collections::BTreeSet<String>,
}

/// The script's file state: where it reads and writes, where `Folder.current` starts, the files it
/// has open, how much it has read and written, and the file dialog it waits at.
pub(crate) struct Disk {
    scope: ScriptScope,
    /// Where the script writes: its temporary and data folders and what was granted for writing.
    writes: ScriptScope,
    home: String,
    start: String,
    open: HashMap<u32, Open>,
    temps: u32,
    bytes: usize,
    /// The most bytes counted in `bytes` for each file read, by its real path ([`Disk::charge_read`]).
    charged: HashMap<std::path::PathBuf, usize>,
    written: usize,
    framed: bool,
    /// The script's file with its links followed, and the SHA-256 of its text.
    script: Option<(std::path::PathBuf, String)>,
    /// The SHA-256 of the script's text with the files it includes, also for a script given as
    /// text, which its named engine keeps as code the engine holds.
    own: String,
    data_dir: Option<std::path::PathBuf>,
    /// The folder that holds the settings, with its links followed, which no grant reaches.
    fenced: Option<std::path::PathBuf>,
    /// The run's temporary folder and the script's data folder, with their links followed, once made.
    temp: Option<std::path::PathBuf>,
    user_data: Option<std::path::PathBuf>,
    /// The file dialog the script waits at.
    pub(crate) picker: Option<ScriptPicker>,
    /// The error that ended the run when a dialog's answer could not reach the script.
    pub(crate) failed: Option<vectorcraft_js::RunError>,
    /// The files written since the host last took them ([`Disk::take_written`]).
    wrote: Vec<String>,
    /// The places granted at the start (the script's own folder) and by the run itself (its
    /// temporary and data folders and what a person allowed in a dialog), each with whether it
    /// may be written and whether it is a folder: a withdrawn kept grant leaves these in force.
    held: Vec<(std::path::PathBuf, bool, bool)>,
    /// The kept grants in force: each is withdrawn once the run evaluates code it was not kept for.
    stored: Vec<Stored>,
    /// The SHA-256 of each source other than the script itself that the run's code may run: what
    /// it evaluated ([`code_hash`]) and what its named engine kept from earlier runs.
    ran: std::collections::BTreeSet<String>,
    /// The run evaluated more distinct sources than [`MAX_RUN_CODE`]: no kept grant is in force.
    ran_too_much: bool,
    /// The grants this run kept with Always (path, write, folder), and how many sources `ran` held
    /// when they were last written to the grants file.
    kept: Vec<(std::path::PathBuf, bool, bool)>,
    recorded: usize,
    /// The write bits setting `readonly` cleared from each file in this run, by its device and
    /// inode, which a rename keeps, at most [`MAX_ENTRIES`] files: clearing `readonly` gives them back.
    #[cfg(unix)]
    cleared: HashMap<(u64, u64), u32>,
}

impl Drop for Disk {
    fn drop(&mut self) {
        // The run's end has flushed these already and reported what it could not write.
        let _ = self.flush_all();
        if let Some(temp) = self.temp.take() {
            let _ = std::fs::remove_dir_all(temp);
        }
    }
}

impl Disk {
    /// The state of a run whose own files are read in `scope`.
    pub(crate) fn new(scope: &ScriptScope, setup: Setup) -> Self {
        let home = home_folder();
        let start = scope.folder().map_or_else(|| home.clone(), |p| p.to_string_lossy().into_owned());
        let web = cfg!(target_arch = "wasm32");
        let script = setup.script.as_deref().filter(|_| !web).and_then(|p| std::fs::canonicalize(p).ok());
        let own = vectorcraft_plugins::permissions::sha256_hex(setup.source.as_bytes());
        let script = script.map(|p| (p, own.clone()));
        let fenced = setup.data_dir.as_deref().filter(|_| !web).and_then(Path::parent).and_then(real_or_planned);
        let held = scope.roots().iter().map(|r| (r.clone(), false, true)).collect();
        let mut d = Self {
            scope: scope.clone(),
            writes: ScriptScope::default(),
            home,
            start,
            open: HashMap::new(),
            temps: 0,
            bytes: 0,
            charged: HashMap::new(),
            written: 0,
            framed: setup.framed,
            script,
            own,
            data_dir: setup.data_dir,
            fenced: fenced.clone(),
            temp: None,
            user_data: None,
            picker: None,
            failed: None,
            wrote: Vec::new(),
            held,
            stored: Vec::new(),
            ran: std::collections::BTreeSet::new(),
            ran_too_much: setup.engine_code.1,
            kept: Vec::new(),
            recorded: 0,
            #[cfg(unix)]
            cleared: HashMap::new(),
        };
        for hash in setup.engine_code.0 {
            let _ = d.saw(hash);
        }
        if let Some(f) = fenced {
            // A script's folder inside the settings folder stays open; the settings folder itself
            // does not, only the script's own file there.
            let roots = d.scope.roots();
            let mut own: Vec<std::path::PathBuf> = roots.iter().filter(|r| !path_inside(&f, r)).cloned().collect();
            if roots.iter().any(|r| path_inside(r, &f) && path_inside(&f, r)) {
                own.extend(d.script.as_ref().map(|(p, _)| p.clone()));
            }
            d.scope.fence(f.clone(), &own);
            d.writes.fence(f, &[]);
        }
        if d.framed {
            for g in d.stored() {
                for (scope, granted) in [(&mut d.writes, g.write), (&mut d.scope, true)] {
                    match (granted, g.folder) {
                        (false, _) => {}
                        (true, true) => scope.add(g.path.clone()),
                        (true, false) => scope.add_file(g.path.clone()),
                    }
                }
                d.stored.push(g);
            }
            d.withdraw();
        }
        d
    }

    /// Notes that the run evaluates the source `units` (`eval`, `Function`, `$.evalFile`), before
    /// it runs: the kept grants that were not kept for it are withdrawn.
    pub(crate) fn saw_code(&mut self, units: &[u16]) {
        if self.saw(code_hash(units)) {
            self.withdraw();
        }
    }

    /// Adds the source with SHA-256 `hash` to what the run's code may run. → whether it was new.
    fn saw(&mut self, hash: String) -> bool {
        if self.ran_too_much || self.ran.contains(&hash) || self.own == hash {
            return false;
        }
        if self.ran.len() >= MAX_RUN_CODE {
            self.ran_too_much = true;
        } else {
            self.ran.insert(hash);
        }
        true
    }

    /// What the run's code may run, for its named engine to keep: each source's SHA-256 (the
    /// script's own included, also for a script given as text) and whether there was more than
    /// [`MAX_RUN_CODE`].
    pub(crate) fn code(&self) -> (Vec<String>, bool) {
        (std::iter::once(self.own.clone()).chain(self.ran.iter().cloned()).collect(), self.ran_too_much)
    }

    /// Withdraws each kept grant whose run did not evaluate every source this run has: its place
    /// is no longer read or written, unless the run holds it some other way, and files opened for
    /// writing there are written now and take no more writes.
    fn withdraw(&mut self) {
        let (ran, all) = (&self.ran, self.ran_too_much);
        let (stay, gone): (Vec<Stored>, Vec<Stored>) = std::mem::take(&mut self.stored).into_iter().partition(|g| !all && ran.is_subset(&g.code));
        self.stored = stay;
        if gone.is_empty() {
            return;
        }
        for g in &gone {
            let held = |write: bool| {
                self.held.iter().any(|(p, w, f)| *p == g.path && *f == g.folder && (*w || !write))
                    || self.stored.iter().any(|s| s.path == g.path && s.folder == g.folder && (s.write || !write))
            };
            if !held(false) {
                self.scope.remove(&g.path, g.folder);
            }
            if g.write && !held(true) {
                self.writes.remove(&g.path, g.folder);
            }
        }
        let ids: Vec<u32> =
            self.open.iter().filter(|(_, o)| o.target.as_deref().is_some_and(|t| !self.writes.allows(t))).map(|(id, _)| *id).collect();
        for id in ids {
            let written = self.flush(id).is_ok();
            if let Some(o) = self.open.get_mut(&id) {
                // Changes that could not be written stay, so the run's end tries them again and
                // reports the file when it still cannot.
                o.withdrawn = true;
                if written {
                    o.target = None;
                }
            }
        }
        // Objects that hold a file the run may no longer read give none of it.
        for o in self.open.values_mut() {
            if o.read_from.as_deref().is_some_and(|p| !self.scope.allows(p)) {
                o.unreadable = true;
            }
        }
    }

    /// Writes the evaluated code this run ran into the grants it kept with Always, so a later run
    /// of the same script that evaluates the same code keeps them in force. A grant that cannot be
    /// updated keeps the code it was kept with.
    pub(crate) fn record_code(&mut self) {
        if self.kept.is_empty() || self.ran_too_much || self.ran.len() == self.recorded {
            return;
        }
        let (Some((script, hash)), Some(store)) = (&self.script, self.store_path()) else { return };
        let (script, code) = (shown(script), self.ran.iter().cloned().collect::<Vec<_>>());
        let Ok(_lock) = lock_store(&store) else { return };
        let Ok(mut grants) = try_read_store(&store) else { return };
        for g in &mut grants {
            let ours = g["script"] == *script
                && g["sha256"] == *hash
                && self.kept.iter().any(|(p, w, f)| g["path"] == *shown(p) && g["write"] == *w && g["folder"] == *f);
            if ours {
                g["code"] = serde_json::json!(code);
            }
        }
        if write_store(&store, grants).is_ok() {
            self.recorded = self.ran.len();
        }
    }

    /// The gate for a command the script runs: the places it reads and writes now, and the files of
    /// the open documents, `documents`.
    pub(crate) fn gate(&self, documents: Vec<String>) -> crate::file_access::ScriptGate {
        crate::file_access::ScriptGate { read: self.scope.clone(), write: self.writes.clone(), documents }
    }

    /// The files written since this was last asked, to name in the report.
    pub(crate) fn take_written(&mut self) -> Vec<String> {
        std::mem::take(&mut self.wrote)
    }

    fn wrote(&mut self, real: &Path) {
        let shown = crate::cmd::findfiles::plain(real.to_path_buf()).to_string_lossy().into_owned();
        if !self.wrote.contains(&shown) {
            self.wrote.push(shown);
        }
    }

    /// Grants the real path `real`, a folder with everything in it (`folder`) or a file, for
    /// reading, and with `write` for writing too.
    fn grant(&mut self, real: std::path::PathBuf, write: bool, folder: bool) {
        let scopes = if write { vec![&mut self.writes, &mut self.scope] } else { vec![&mut self.scope] };
        for scope in scopes {
            if folder { scope.add(real.clone()) } else { scope.add_file(real.clone()) }
        }
        self.held.push((real, write, folder));
    }

    /// The run's temporary folder, made the first time it is asked for; `None` when it cannot be
    /// made, when the automation write roots do not take it, and on the web.
    fn temp_dir(&mut self) -> Option<std::path::PathBuf> {
        if cfg!(target_arch = "wasm32") {
            return None;
        }
        if self.temp.is_none() {
            let base = std::env::temp_dir();
            let made = (0..64).find_map(|_| {
                let n = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
                let dir = base.join(format!("VectorCraft Script {}-{n}", std::process::id()));
                crate::file_access::check_write(&dir.to_string_lossy()).ok()?;
                std::fs::create_dir(&dir).ok().and_then(|()| std::fs::canonicalize(&dir).ok())
            })?;
            self.grant(made.clone(), true, true);
            self.temp = Some(made);
        }
        self.temp.clone().map(crate::cmd::findfiles::plain)
    }

    /// The script's data folder, made the first time it is asked for: for a script run from a file,
    /// a folder named by the SHA-256 of its path in the data folder, and otherwise a folder in the
    /// run's temporary folder. `None` when that folder is a link, cannot be made, or is outside the
    /// automation write roots.
    fn user_data_dir(&mut self) -> Option<std::path::PathBuf> {
        if cfg!(target_arch = "wasm32") {
            return None;
        }
        if self.user_data.is_none() {
            let dir = match (&self.script, &self.data_dir) {
                (Some((path, _)), Some(data)) => {
                    let key = vectorcraft_plugins::permissions::sha256_hex(path.to_string_lossy().as_bytes());
                    data.join(key.get(..16).unwrap_or(&key))
                }
                _ => self.temp_dir()?.join("userData"),
            };
            crate::file_access::check_write(&dir.to_string_lossy()).ok()?;
            // Each missing folder above it that is made on the way must be inside the write roots too.
            for missing in dir.ancestors().skip(1).take_while(|a| !a.as_os_str().is_empty() && std::fs::symlink_metadata(a).is_err()) {
                crate::file_access::check_write(&missing.to_string_lossy()).ok()?;
            }
            // The data folder that holds the folder, and then the folder, are folders where they are
            // named, not links; the folder is made only once the data folder is checked.
            let direct = |d: &Path| -> Option<std::path::PathBuf> {
                let (parent, name) = (std::fs::canonicalize(d.parent()?).ok()?, d.file_name()?);
                let real = std::fs::canonicalize(d).ok()?;
                (real == parent.join(name) && std::fs::symlink_metadata(d).is_ok_and(|m| m.is_dir())).then_some(real)
            };
            std::fs::create_dir_all(dir.parent()?).ok()?;
            direct(dir.parent()?)?;
            std::fs::create_dir_all(&dir).ok()?;
            let real = direct(&dir)?;
            self.scope.fence_open(real.clone());
            self.writes.fence_open(real.clone());
            self.grant(real.clone(), true, true);
            self.user_data = Some(real);
        }
        self.user_data.clone().map(crate::cmd::findfiles::plain)
    }

    /// The grants file, beside the scripts' data folders.
    fn store_path(&self) -> Option<std::path::PathBuf> {
        self.data_dir.as_ref().map(|d| d.with_file_name(GRANTS_FILE))
    }

    /// The grants the grants file keeps for this script with this text: each path, whether it may
    /// be written, whether it is a folder, and the evaluated code it was kept for. A grant whose
    /// path no longer is its own real path is left out.
    fn stored(&self) -> Vec<Stored> {
        let (Some((script, hash)), Some(store)) = (&self.script, self.store_path()) else { return Vec::new() };
        let script = shown(script);
        read_store(&store)
            .into_iter()
            .filter(|g| g["script"] == *script && g["sha256"] == *hash)
            .filter_map(|g| {
                let code = g["code"].as_array()?.iter().map(|c| c.as_str().map(str::to_string)).collect::<Option<_>>()?;
                let (write, folder) = (g["write"].as_bool()?, g["folder"].as_bool()?);
                // The grant stays on the real path it was kept for: a path that now leads elsewhere,
                // through a link put in its place or in a folder above it, is not granted.
                let saved = g["path"].as_str()?;
                let path = real_of(Path::new(saved)).filter(|real| shown(real) == saved)?;
                Some(Stored { path, write, folder, code })
            })
            .filter(|g| self.fenced.as_ref().is_none_or(|f| !crate::file_access::path_inside(&g.path, f)))
            .collect()
    }

    /// Adds grants of `paths` (with `write`, folders when `folder`) for this script to the grants
    /// file, with the code the run has evaluated so far, written whole to a file beside it that then
    /// takes its place.
    fn keep(&self, paths: &[std::path::PathBuf], write: bool, folder: bool) -> Result<(), &'static str> {
        let (Some((script, hash)), Some(store)) = (&self.script, self.store_path()) else { return Err(NO_ALWAYS) };
        // A grants file that is there but cannot be read is left as it is.
        let _lock = lock_store(&store).map_err(|_| NOT_SAVED)?;
        let mut grants = try_read_store(&store).map_err(|_| NOT_SAVED)?;
        let script = shown(script);
        let code: Vec<&String> = self.ran.iter().collect();
        for p in paths {
            let path = shown(p);
            grants
                .retain(|g| !(g["script"] == *script && g["sha256"] == *hash && g["path"] == *path && g["write"] == write && g["folder"] == folder));
            grants.push(serde_json::json!({"script": script, "sha256": hash, "code": code, "path": path, "write": write, "folder": folder}));
        }
        if grants.len() > MAX_STORED_GRANTS {
            return Err(NOT_SAVED);
        }
        write_store(&store, grants).map_err(|_| NOT_SAVED)
    }

    /// The paths a person picked in the dialog the script waits at, allowed (`always`: kept for
    /// later runs too), or denied; → what the dialog's call gives the script (a path, a list of
    /// paths for an open dialog that picks more than one, `null`, or `{error}`), `None` when no
    /// dialog waits.
    pub(crate) fn answer(&mut self, answer: PickAnswer) -> Option<Data> {
        let picker = self.picker.take()?;
        let PickAnswer::Allow { paths, always } = answer else { return Some(Data::Null) };
        Some(self.allow(&picker, &paths, always).unwrap_or_else(error))
    }

    fn allow(&mut self, picker: &ScriptPicker, paths: &[String], always: bool) -> Result<Data, &'static str> {
        let wanted = if picker.multiple { paths.len().min(MAX_ENTRIES) } else { 1 };
        let mut reals = Vec::new();
        for p in paths.iter().take(wanted) {
            reals.push(self.picked(picker.kind, p)?);
        }
        if reals.is_empty() {
            return Ok(Data::Null);
        }
        let (write, folder) = (picker.kind != PickerKind::Open, picker.kind == PickerKind::Folder);
        if always {
            self.keep(&reals, write, folder)?;
            self.kept.extend(reals.iter().map(|r| (r.clone(), write, folder)));
            self.recorded = self.ran.len();
        }
        let mut picked = Vec::new();
        for real in reals {
            picked.push(Data::String(self.pathname(&shown(&real), &self.start, false).fs()));
            self.grant(real, write, folder);
        }
        Ok(match picked.len() {
            1 if !picker.multiple => picked.swap_remove(0),
            _ => Data::Array(picked),
        })
    }

    /// The real path of `path`, which a person picked in a dialog of `kind`: a file that is there
    /// (open), a folder that is there (folder), or a file whose folder is there (save). A network or
    /// device path, and a place in the folder that holds the settings, are refused.
    fn picked(&self, kind: PickerKind, path: &str) -> Result<std::path::PathBuf, &'static str> {
        let p = self.checked(path)?;
        if ScriptScope::refused_by_name(p) {
            return Err(ACCESS_DENIED);
        }
        let real = match kind {
            PickerKind::Save if std::fs::symlink_metadata(p).is_err() => {
                let (Some(dir), Some(name)) = (p.parent(), p.file_name()) else { return Err(ACCESS_DENIED) };
                std::fs::canonicalize(dir).map_err(|_| NOT_FOUND)?.join(name)
            }
            _ => std::fs::canonicalize(p).map_err(|_| NOT_FOUND)?,
        };
        match kind {
            PickerKind::Folder if !real.is_dir() => return Err(NOT_FOUND),
            PickerKind::Open if !real.is_file() => return Err(NOT_REGULAR),
            PickerKind::Save if real.is_dir() => return Err(NOT_REGULAR),
            _ => {}
        }
        if self.fenced.as_ref().is_some_and(|f| crate::file_access::path_inside(&real, f))
            && !self.user_data.as_ref().is_some_and(|u| crate::file_access::path_inside(&real, u))
        {
            return Err(PROTECTED);
        }
        Ok(real)
    }

    /// The real path writing to `path` writes, or why it may not ([`ScriptScope::target`]).
    fn target(&self, path: &str, follow: bool) -> Result<std::path::PathBuf, &'static str> {
        let p = self.checked(path)?;
        match self.writes.target(p, follow) {
            Ok(real) => Ok(real),
            Err(ScriptRead::Failed(_)) => Err(WRITE_FAILED),
            Err(_) => Err(ACCESS_DENIED),
        }
    }

    /// Counts `n` more bytes written in the run, which ends past [`MAX_RUN_BYTES`].
    fn charge_write(&mut self, n: usize) -> Result<(), HostError> {
        self.written = self.written.saturating_add(n);
        if self.written > MAX_RUN_BYTES {
            return Err(HostError::Abort(OVER_RUN_WRITES.to_string()));
        }
        Ok(())
    }

    /// Opens the file at `path` for writing in `mode` (`w`, `e` or `a`).
    fn open_write(&mut self, path: &str, encoding: &str, mode: &str) -> Result<u32, HostError> {
        let fail = |m: &str| Err(HostError::Error(m.to_string()));
        if self.open.len() >= MAX_OPEN {
            return fail(TOO_MANY_OPEN);
        }
        let real = match self.target(path, true) {
            Ok(real) => real,
            Err(m) => return fail(m),
        };
        let meta = std::fs::metadata(&real).ok();
        if meta.as_ref().is_some_and(|m| !m.is_file()) {
            return fail(NOT_REGULAR);
        }
        if mode == "e" && meta.is_none() {
            return fail(NOT_FOUND);
        }
        // Changes other open objects hold for the file are written before it is opened again, so
        // they do not later replace what this one writes.
        let ids: Vec<u32> = self.open.iter().filter(|(_, o)| o.target.as_deref() == Some(real.as_path())).map(|(id, _)| *id).collect();
        if let Err(m) = ids.iter().try_for_each(|id| self.flush(*id)) {
            return fail(m);
        }
        let bytes = if mode == "w" || meta.is_none() {
            // An encoding that is not one is refused before the file is emptied.
            if Open::new(Vec::new(), encoding).is_none() {
                return Err(HostError::Error(UNDECODABLE.to_string()));
            }
            if std::fs::File::create(&real).is_err() {
                return fail(WRITE_FAILED);
            }
            self.wrote(&real);
            Vec::new()
        } else {
            if meta.as_ref().is_some_and(|m| m.len() > MAX_FILE_BYTES as u64) {
                return fail(TOO_LARGE);
            }
            // The bytes kept are read under the rules for reading.
            let bytes = match self.scope.read(Path::new(path), MAX_FILE_BYTES) {
                Ok((at, bytes)) if at == real => bytes,
                Ok(_) | Err(ScriptRead::Refused) => return fail(ACCESS_DENIED),
                Err(ScriptRead::TooLarge) => return fail(TOO_LARGE),
                Err(ScriptRead::Missing) => return fail(NOT_FOUND),
                Err(ScriptRead::Failed(_)) => return fail(READ_FAILED),
            };
            if bytes.len() > MAX_FILE_BYTES {
                return fail(TOO_LARGE);
            }
            self.charge_read(&real, bytes.len())?;
            bytes
        };
        let mut file = Open::new(bytes, encoding).ok_or_else(|| HostError::Error(UNDECODABLE.to_string()))?;
        if mode == "a" {
            file.pos = file.bytes.len();
        }
        file.read_from = Some(real.clone());
        file.target = Some(real);
        Ok(self.insert(file))
    }

    fn insert(&mut self, file: Open) -> u32 {
        let mut id = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        while id == 0 || self.open.contains_key(&id) {
            id = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        }
        self.open.insert(id, file);
        id
    }

    /// Writes the open file `id`'s bytes to its file when it has changed. `Err` when it could not.
    fn flush(&mut self, id: u32) -> Result<(), &'static str> {
        let Some(file) = self.open.get_mut(&id) else { return Ok(()) };
        let Some(real) = file.target.clone().filter(|_| file.dirty) else { return Ok(()) };
        // A link put in the file's place, or in place of a folder above it, since it was opened is
        // not written through.
        let moved = real.parent().is_some_and(|p| std::fs::canonicalize(p).is_ok_and(|c| c != p));
        if moved || std::fs::symlink_metadata(&real).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(ACCESS_DENIED);
        }
        // The file stays changed until its bytes are written, so the run's end tries it again.
        std::fs::write(&real, &file.bytes).map_err(|_| WRITE_FAILED)?;
        file.dirty = false;
        self.wrote(&real);
        Ok(())
    }

    /// Writes every open file that has changed: at the end of the run. → the files that could not
    /// be written, as the report gives them.
    pub(crate) fn flush_all(&mut self) -> Vec<String> {
        // In the order the files were opened, so the object opened last writes a file last.
        let mut ids: Vec<u32> = self.open.keys().copied().collect();
        ids.sort_unstable();
        let mut failed = Vec::new();
        for id in ids {
            if self.flush(id).is_err()
                && let Some(real) = self.open.get(&id).and_then(|o| o.target.as_deref())
            {
                failed.push(shown(real));
            }
        }
        failed
    }

    /// The path `text` names from the folder `base`, a platform path; `text`'s URI escapes are
    /// decoded when `uri` is set.
    fn pathname(&self, text: &str, base: &str, uri: bool) -> Pathname {
        let root = Pathname::root(WINDOWS);
        let home = Pathname::parse(&self.home, &root, &root, WINDOWS, false);
        let base = Pathname::parse(base, &home, &home, WINDOWS, false);
        Pathname::parse(text, &base, &home, WINDOWS, uri)
    }

    /// The path `path` as the scope takes it, or why it is refused by its form alone.
    fn checked<'a>(&self, path: &'a str) -> Result<&'a Path, &'static str> {
        if cfg!(target_arch = "wasm32") {
            return Err(NO_WEB);
        }
        if too_long(path) {
            return Err(PATH_TOO_LONG);
        }
        let p = Path::new(path);
        if !p.is_absolute() || (WINDOWS && (path.starts_with(r"\\") || path.starts_with("//"))) {
            return Err(ACCESS_DENIED);
        }
        Ok(p)
    }

    fn stat(&self, path: &str) -> Result<Option<crate::file_access::ScriptEntry>, &'static str> {
        match self.scope.stat(self.checked(path)?) {
            Ok(e) => Ok(Some(e)),
            Err(ScriptRead::Missing) => Ok(None),
            Err(ScriptRead::Refused | ScriptRead::TooLarge) => Err(ACCESS_DENIED),
            Err(ScriptRead::Failed(_)) => Err(READ_FAILED),
        }
    }

    fn open_file(&mut self, path: &str, encoding: &str) -> Result<u32, HostError> {
        let fail = |m: &str| Err(HostError::Error(m.to_string()));
        if self.open.len() >= MAX_OPEN {
            return fail(TOO_MANY_OPEN);
        }
        let entry = match self.stat(path) {
            Ok(Some(e)) => e,
            Ok(None) => return fail(NOT_FOUND),
            Err(m) => return fail(m),
        };
        if !entry.file {
            return fail(NOT_REGULAR);
        }
        if entry.len > MAX_FILE_BYTES as u64 {
            return fail(TOO_LARGE);
        }
        let (real, bytes) = match self.scope.read(Path::new(path), MAX_FILE_BYTES) {
            Ok(read) => read,
            Err(ScriptRead::TooLarge) => return fail(TOO_LARGE),
            Err(ScriptRead::Missing) => return fail(NOT_FOUND),
            Err(ScriptRead::Refused) => return fail(ACCESS_DENIED),
            Err(ScriptRead::Failed(_)) => return fail(READ_FAILED),
        };
        self.charge_read(&real, bytes.len())?;
        let mut file = Open::new(bytes, encoding).ok_or_else(|| HostError::Error(UNDECODABLE.to_string()))?;
        file.read_from = Some(real);
        Ok(self.insert(file))
    }

    /// Counts `n` bytes read from the file `real` in the run, which ends past [`MAX_RUN_BYTES`]. A
    /// file opened again once no open object holds it counts only the bytes past the most read from
    /// it before; while another object holds it, all of them count, as each holds its own copy.
    fn charge_read(&mut self, real: &Path, n: usize) -> Result<(), HostError> {
        let held = self.open.values().any(|o| o.read_from.as_deref() == Some(real));
        let before = if held { 0 } else { self.charged.get(real).copied().unwrap_or(0) };
        let more = n.saturating_sub(before);
        if self.bytes.saturating_add(more) > MAX_RUN_BYTES {
            return Err(HostError::Abort(OVER_RUN_BYTES.to_string()));
        }
        self.bytes += more;
        if !held && n > before && (self.charged.contains_key(real) || self.charged.len() < MAX_CHARGED) {
            self.charged.insert(real.to_path_buf(), n);
        }
        Ok(())
    }
}

/// The real path `real` as the grants file and the report give it.
fn shown(real: &Path) -> String {
    crate::cmd::findfiles::plain(real.to_path_buf()).to_string_lossy().into_owned()
}

/// The path `p` with its links followed; for a missing file whose folder is there, that folder's
/// real path and the file's name.
fn real_of(p: &Path) -> Option<std::path::PathBuf> {
    std::fs::canonicalize(p).ok().or_else(|| Some(std::fs::canonicalize(p.parent()?).ok()?.join(p.file_name()?)))
}

/// The path `p` with its links followed, also when it is not there yet: the real path of the
/// nearest folder above it that is there, and the names below that folder.
fn real_or_planned(p: &Path) -> Option<std::path::PathBuf> {
    let mut names = Vec::new();
    let mut at = p;
    loop {
        if let Ok(real) = std::fs::canonicalize(at) {
            return Some(names.iter().rev().fold(real, |real, name| real.join(name)));
        }
        names.push(at.file_name()?);
        at = at.parent()?;
    }
}

/// The first `max` bytes of the file at `real`, and one more when it has more.
fn read_head(real: &Path, max: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(real)?.take(max as u64 + 1).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The grants the grants file at `store` holds; none when it is missing or cannot be read. A grant
/// that is not well formed is left out. It is read under [`lock_store`]'s lock when that can be
/// taken, so it is read between two changes.
fn read_store(store: &Path) -> Vec<serde_json::Value> {
    let _lock = lock_store(store).ok();
    load_store(store, false).unwrap_or_default()
}

/// The grants the grants file at `store` holds, to change and write back; none when it is missing,
/// and `Err` when it is there but cannot be read, is too large, is not a grants file of this
/// version, or holds a grant that is not well formed or more grants than may be kept, so that a
/// change is not written over it.
fn try_read_store(store: &Path) -> std::io::Result<Vec<serde_json::Value>> {
    load_store(store, true)
}

/// The grants file at `store` read as [`read_store`] reads it, or with `strict` as
/// [`try_read_store`] does.
fn load_store(store: &Path, strict: bool) -> std::io::Result<Vec<serde_json::Value>> {
    let bytes = match read_head(store, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    if bytes.len() > MAX_STORE_BYTES {
        return Err(std::io::Error::other("the grants file is too large"));
    }
    let v = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(std::io::Error::other)?;
    if v["version"] != 1 {
        return Err(std::io::Error::other("the grants file has another version"));
    }
    let valid = |g: &serde_json::Value| {
        g["script"].is_string()
            && g["sha256"].is_string()
            && g["code"].as_array().is_some_and(|c| c.iter().all(serde_json::Value::is_string))
            && g["path"].is_string()
            && g["write"].is_boolean()
            && g["folder"].is_boolean()
    };
    if strict {
        let grants = v["grants"].as_array().ok_or_else(|| std::io::Error::other("the grants file holds no list of grants"))?;
        if grants.len() > MAX_STORED_GRANTS || !grants.iter().all(valid) {
            return Err(std::io::Error::other("the grants file holds a grant that cannot be read"));
        }
        return Ok(grants.clone());
    }
    Ok(v["grants"].as_array().map(|a| a.iter().filter(|g| valid(g)).take(MAX_STORED_GRANTS).cloned().collect()).unwrap_or_default())
}

/// The number in the name of the next file [`write_store`] writes before it takes the grants file's
/// place.
pub(crate) static NEXT_PART: AtomicU32 = AtomicU32::new(0);

/// Writes `grants` as the grants file at `store`: to a new file beside it, named for this process
/// and this write, which then takes its place. `Err`, with the file left as it was, when the grants
/// take more than [`MAX_STORE_BYTES`] or the new file cannot be written or take its place.
fn write_store(store: &Path, grants: Vec<serde_json::Value>) -> std::io::Result<()> {
    let text = serde_json::to_vec_pretty(&serde_json::json!({"version": 1, "grants": grants})).map_err(std::io::Error::other)?;
    if text.len() > MAX_STORE_BYTES {
        return Err(std::io::Error::other("the grants file would be too large"));
    }
    if let Some(dir) = store.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // A name an earlier write left behind is passed over, up to 64 times, and only the file made here
    // is removed.
    let mut tries = 0;
    let (part, mut file) = loop {
        let n = NEXT_PART.fetch_add(1, Ordering::Relaxed);
        let part = store.with_extension(format!("json.{}-{n}.part", std::process::id()));
        match std::fs::File::options().write(true).create_new(true).open(&part) {
            Ok(file) => break (part, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && tries < 63 => tries += 1,
            Err(e) => return Err(e),
        }
    };
    let written = std::io::Write::write_all(&mut file, &text);
    drop(file);
    // `rename` replaces a grants file that is there, on Windows too (`MOVEFILE_REPLACE_EXISTING`).
    written.and_then(|()| std::fs::rename(&part, store)).inspect_err(|_| {
        let _ = std::fs::remove_file(&part);
    })
}

/// The lock file beside the grants file at `store` (`Script Grants.lock`), locked until it is
/// dropped and waiting while another process holds it, so that processes that read and change the
/// grants file take turns. A change is not made when the lock cannot be taken.
#[cfg(not(target_arch = "wasm32"))]
fn lock_store(store: &Path) -> std::io::Result<std::fs::File> {
    let lock = store.with_extension("lock");
    let opened = store
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::File::options().create(true).truncate(false).read(true).write(true).open(&lock));
    let file = opened.and_then(|f| f.lock().map(|()| f));
    if let Err(e) = &file {
        log::warn!("can't lock {}: {e}", lock.display());
    }
    file
}

/// The browser has no folders to keep anything in.
#[cfg(target_arch = "wasm32")]
fn lock_store(_: &Path) -> std::io::Result<std::fs::File> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no file system"))
}

/// The grants the grants file beside the data folder `data_dir` keeps: each script, the SHA-256 of
/// its text, the path and whether it may be written (`script.grants`).
pub(crate) fn stored_grants(data_dir: &Path) -> Vec<serde_json::Value> {
    read_store(&data_dir.with_file_name(GRANTS_FILE))
}

/// Removes from the grants file beside `data_dir` the grants of the script `script` and of the path
/// `path`, of both when both are given and of every grant when neither is (`script.grants.revoke`).
/// → how many were removed.
pub(crate) fn revoke_grants(data_dir: &Path, script: Option<&str>, path: Option<&str>) -> std::io::Result<usize> {
    let store = data_dir.with_file_name(GRANTS_FILE);
    let _lock = lock_store(&store)?;
    let grants = try_read_store(&store)?;
    let before = grants.len();
    let kept: Vec<serde_json::Value> =
        grants.into_iter().filter(|g| !(script.is_none_or(|s| g["script"] == *s) && path.is_none_or(|p| g["path"] == *p))).collect();
    let removed = before - kept.len();
    if removed > 0 {
        write_store(&store, kept)?;
    }
    Ok(removed)
}

/// Whether paths are read as Windows reads them.
const WINDOWS: bool = cfg!(windows);

/// Does `path` have more than [`MAX_PATH_CHARS`] characters?
fn too_long(path: &str) -> bool {
    path.len() > MAX_PATH_CHARS && path.chars().nth(MAX_PATH_CHARS).is_some()
}

/// The home folder, as the environment gives it.
fn home_folder() -> String {
    let var = if WINDOWS { "USERPROFILE" } else { "HOME" };
    std::env::var(var).unwrap_or_else(|_| if WINDOWS { r"C:\".to_string() } else { "/".to_string() })
}

/// The folders `Folder.temp`, `Folder.desktop` and the others name, `null` where there is none:
/// `temp` and `user_data` are the run's own. On the web every one is `null`, and no platform folder
/// is looked up.
fn locations(home: &str, temp: Option<String>, user_data: Option<String>) -> Data {
    if cfg!(target_arch = "wasm32") {
        return Data::Object(PLACES.iter().map(|k| (k.to_string(), Data::Null)).collect());
    }
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let join = |a: &str, b: &str| Path::new(a).join(b).to_string_lossy().into_owned();
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned()));
    let (app_data, common, system, trash) = if WINDOWS {
        (env("PROGRAMDATA"), env("CommonProgramFiles"), env("SystemRoot"), None)
    } else if cfg!(target_os = "macos") {
        (
            Some("/Library/Application Support".to_string()),
            Some("/Library/Application Support".to_string()),
            Some("/System".to_string()),
            Some(join(home, ".Trash")),
        )
    } else {
        let data = env("XDG_DATA_HOME").unwrap_or_else(|| join(home, ".local/share"));
        (Some("/usr/share".to_string()), Some("/usr/share".to_string()), Some("/".to_string()), Some(join(&data, "Trash")))
    };
    let s = |v: Option<String>| v.map_or(Data::Null, Data::String);
    let places = vec![
        ("appData", app_data),
        ("appPackage", exe.clone()),
        ("commonFiles", common),
        ("desktop", Some(join(home, "Desktop"))),
        ("myDocuments", Some(join(home, "Documents"))),
        ("startup", exe),
        ("system", system),
        ("temp", temp),
        ("trash", trash),
        ("userData", user_data),
    ];
    Data::Object(places.into_iter().map(|(k, v)| (k.to_string(), s(v))).collect())
}

/// The names [`locations`] gives.
const PLACES: [&str; 10] = ["appData", "appPackage", "commonFiles", "desktop", "myDocuments", "startup", "system", "temp", "trash", "userData"];

/// The folder for temporary files when the run's own cannot be made: the platform's, or on the web
/// the root, where nothing is read.
fn temp_folder() -> std::path::PathBuf {
    if cfg!(target_arch = "wasm32") { std::path::PathBuf::from("/") } else { std::env::temp_dir() }
}

/// The start of an absolute path.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Root {
    /// `/`.
    Unix,
    /// A Windows drive letter, in upper case.
    Drive(char),
    /// A Windows share: `\\host\share`.
    Share(String, String),
    /// A Windows `\\?\` or `\\.\` path, kept as given.
    Device(String),
}

/// An absolute path: its root and its names, with `.` and `..` resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Pathname {
    root: Root,
    names: Vec<String>,
}

impl Pathname {
    fn root(windows: bool) -> Self {
        Self { root: if windows { Root::Drive('C') } else { Root::Unix }, names: Vec::new() }
    }

    /// The path `text` names, relative ones taken from `base` and `~` from `home`, with its URI
    /// escapes decoded when `uri` is set.
    fn parse(text: &str, base: &Pathname, home: &Pathname, windows: bool, uri: bool) -> Self {
        let decoded = if uri { decode(text) } else { text.to_string() };
        let t = if windows { decoded.replace('\\', "/") } else { decoded };
        let (start, rest): (Pathname, &str) = if windows && (t.starts_with("//?/") || t.starts_with("//./")) {
            return Self { root: Root::Device(t.replace('/', "\\")), names: Vec::new() };
        } else if let Some(r) = t.strip_prefix("//").filter(|_| windows) {
            let mut parts = r.splitn(3, '/');
            let host = parts.next().unwrap_or_default().to_string();
            let share = parts.next().unwrap_or_default().to_string();
            (Self { root: Root::Share(host, share), names: Vec::new() }, parts.next().unwrap_or_default())
        } else if windows && t.len() >= 2 && t.as_bytes()[0].is_ascii_alphabetic() && t.as_bytes()[1] == b':' {
            (Self { root: Root::Drive(t.as_bytes()[0].to_ascii_uppercase() as char), names: Vec::new() }, t.get(2..).unwrap_or_default())
        } else if let Some(r) = t.strip_prefix('/') {
            let (first, after) = r.split_once('/').unwrap_or((r, ""));
            if windows && first.len() == 1 && first.as_bytes()[0].is_ascii_alphabetic() {
                (Self { root: Root::Drive(first.as_bytes()[0].to_ascii_uppercase() as char), names: Vec::new() }, after)
            } else if windows {
                let root = match &base.root {
                    Root::Drive(_) | Root::Share(..) => base.root.clone(),
                    _ => Root::Drive('C'),
                };
                (Self { root, names: Vec::new() }, r)
            } else {
                (Self { root: Root::Unix, names: Vec::new() }, r)
            }
        } else if t == "~" || t.starts_with("~/") {
            (home.clone(), t.get(1..).unwrap_or_default())
        } else {
            (base.clone(), t.as_str())
        };
        let mut out = start;
        for name in rest.split('/') {
            match name {
                "" | "." => {}
                ".." => {
                    out.names.pop();
                }
                n => out.names.push(n.to_string()),
            }
        }
        out
    }

    /// The path as the platform writes it.
    fn fs(&self) -> String {
        match &self.root {
            Root::Unix => format!("/{}", self.names.join("/")),
            Root::Drive(c) => format!("{c}:\\{}", self.names.join("\\")),
            Root::Share(host, share) => self.names.iter().fold(format!(r"\\{host}\{share}"), |s, n| format!("{s}\\{n}")),
            Root::Device(raw) => raw.clone(),
        }
    }

    /// The path in URI notation, each name escaped.
    fn uri(&self) -> String {
        let start = match &self.root {
            Root::Unix => String::new(),
            Root::Drive(c) => format!("/{}", c.to_ascii_lowercase()),
            Root::Share(host, share) => format!("//{}/{}", encode(host), encode(share)),
            Root::Device(raw) => return raw.replace('\\', "/"),
        };
        let names: String = self.names.iter().map(|n| format!("/{}", encode(n))).collect();
        if start.is_empty() && names.is_empty() { "/".to_string() } else { start + &names }
    }

    fn parent(&self) -> Option<Self> {
        let mut p = self.clone();
        p.names.pop().map(|_| p)
    }

    /// `self` relative to the folder `base`, in URI notation; the whole URI when they have different
    /// roots.
    fn relative_to(&self, base: &Pathname, windows: bool) -> String {
        if self.root != base.root {
            return self.uri();
        }
        let same = |a: &String, b: &String| if windows { a.to_lowercase() == b.to_lowercase() } else { a == b };
        let common = self.names.iter().zip(&base.names).take_while(|(a, b)| same(a, b)).count();
        let mut parts: Vec<String> = vec!["..".to_string(); base.names.len() - common];
        parts.extend(self.names.iter().skip(common).map(|n| encode(n)));
        if parts.is_empty() { ".".to_string() } else { parts.join("/") }
    }

    /// The parts of `File` and `Folder` objects that come from the path alone.
    fn data(&self) -> Data {
        let s = |v: String| Data::String(v);
        Data::Object(vec![
            ("fs".into(), s(self.fs())),
            ("uri".into(), s(self.uri())),
            ("name".into(), s(self.names.last().map(|n| encode(n)).unwrap_or_default())),
            ("display".into(), s(self.names.last().cloned().unwrap_or_default())),
            ("path".into(), s(self.parent().map(|p| p.uri()).unwrap_or_default())),
            ("parent".into(), self.parent().map_or(Data::Null, |p| Data::String(p.fs()))),
        ])
    }
}

/// `s` with every character but letters, digits and `/-_.!~*'()` written as `%xx` escapes of its
/// UTF-8 bytes (RFC 2396).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `s` with its `%xx` escapes read as UTF-8 bytes; `s` as it is when they do not make UTF-8.
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match (b[i], b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (c, ..) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Do file names match a mask without regard to case: on Windows and macOS, whose file systems
/// usually do not tell names apart by case. Elsewhere case matters.
const MASK_IGNORES_CASE: bool = cfg!(any(windows, target_os = "macos"));

/// A `getFiles` mask, where `*` stands for any run of characters and `?` for one, without regard
/// to case where the file system does not tell names apart by case ([`MASK_IGNORES_CASE`]), read
/// once for a whole listing.
struct Mask {
    /// The mask, in lower case when case does not matter, each run of `*` written as one.
    chars: Vec<char>,
    /// How many of its characters are not `*`: a name with fewer characters does not match.
    fixed: usize,
    /// Names are compared without regard to case.
    fold: bool,
}

impl Mask {
    fn new(mask: &str) -> Self {
        Self::with_case(mask, MASK_IGNORES_CASE)
    }

    /// The mask `mask`, which compares names without regard to case when `fold`.
    fn with_case(mask: &str, fold: bool) -> Self {
        let mut chars: Vec<char> = Vec::new();
        let text = if fold { mask.to_lowercase() } else { mask.to_string() };
        for c in text.chars() {
            if !(c == '*' && chars.last() == Some(&'*')) {
                chars.push(c);
            }
        }
        let fixed = chars.iter().filter(|c| **c != '*').count();
        Self { chars, fixed, fold }
    }

    /// Does `name` match, taking a step from `steps` for each character compared? `None` when the
    /// steps run out first.
    fn matches(&self, name: &str, steps: &mut usize) -> Option<bool> {
        let n: Vec<char> = if self.fold { name.to_lowercase().chars().collect() } else { name.chars().collect() };
        if n.len() < self.fixed {
            return Some(false);
        }
        matched(&self.chars, &n, steps)
    }
}

/// Does `n` match the mask `m`, both in the same case? `None` when `steps` run out first.
fn matched(m: &[char], n: &[char], steps: &mut usize) -> Option<bool> {
    let (mut i, mut j, mut star, mut mark) = (0usize, 0usize, None, 0usize);
    while j < n.len() {
        *steps = steps.checked_sub(1)?;
        match m.get(i) {
            Some('*') => {
                star = Some(i);
                mark = j;
                i += 1;
            }
            Some(c) if *c == '?' || *c == n[j] => {
                i += 1;
                j += 1;
            }
            _ => match star {
                Some(s) => {
                    i = s + 1;
                    mark += 1;
                    j = mark;
                }
                None => return Some(false),
            },
        }
    }
    Some(m[i..].iter().all(|c| *c == '*'))
}

/// A character encoding an opened file is read in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoding {
    Utf8,
    Ascii,
    Binary,
    Latin1,
    Cp1252,
    Utf16Le,
    Utf16Be,
}

/// The characters CP1252 gives bytes 0x80 to 0x9F; the five it leaves undefined stand for
/// themselves.
const CP1252_HIGH: [u16; 32] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008D, 0x017D, 0x008F, 0x0090, 0x2018,
    0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178,
];

impl Encoding {
    fn parse(name: &str) -> Option<Self> {
        Some(match name.trim().to_ascii_uppercase().as_str() {
            "UTF-8" | "UTF8" => Self::Utf8,
            "ASCII" | "US-ASCII" => Self::Ascii,
            "BINARY" => Self::Binary,
            "ISO-8859-1" | "ISO8859-1" | "LATIN1" | "LATIN-1" => Self::Latin1,
            "CP1252" | "WINDOWS-1252" => Self::Cp1252,
            "UTF-16LE" | "UTF16LE" => Self::Utf16Le,
            "UTF-16" | "UTF16" | "UTF-16BE" | "UTF16BE" | "UCS-2" => Self::Utf16Be,
            _ => return None,
        })
    }

    /// `text` in this encoding; `None` when a character has no form in it. BINARY writes each
    /// character's low byte.
    fn encode(self, text: &str) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(text.len());
        for c in text.chars() {
            let code = u32::from(c);
            match self {
                Self::Utf8 => out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
                Self::Ascii if code < 0x80 => out.push(code as u8),
                Self::Binary => out.push((code & 0xFF) as u8),
                Self::Latin1 if code <= 0xFF => out.push(code as u8),
                Self::Cp1252 if code < 0x80 || (0xA0..=0xFF).contains(&code) => out.push(code as u8),
                Self::Cp1252 => out.push(0x80 + CP1252_HIGH.iter().position(|h| u32::from(*h) == code)? as u8),
                Self::Utf16Le | Self::Utf16Be => {
                    for unit in c.encode_utf16(&mut [0; 2]) {
                        out.extend_from_slice(&if self == Self::Utf16Le { unit.to_le_bytes() } else { unit.to_be_bytes() });
                    }
                }
                Self::Ascii | Self::Latin1 => return None,
            }
        }
        Some(out)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Ascii => "ASCII",
            Self::Binary => "BINARY",
            Self::Latin1 => "ISO-8859-1",
            Self::Cp1252 => "CP1252",
            Self::Utf16Le => "UTF-16LE",
            Self::Utf16Be => "UTF-16BE",
        }
    }
}

/// An opened file: its bytes, the byte position and the encoding it is read in; for a file opened
/// for writing, the real path its bytes are written to and whether they changed since.
struct Open {
    bytes: Vec<u8>,
    pos: usize,
    encoding: Encoding,
    name: String,
    target: Option<std::path::PathBuf>,
    /// The real path of the file the object was opened on, whose bytes it holds.
    read_from: Option<std::path::PathBuf>,
    dirty: bool,
    /// The grant the file was opened under was withdrawn while its changes could not be written:
    /// they are written again when the run ends, and the object takes no more writes.
    withdrawn: bool,
    /// The run may no longer read the file the object holds: `read`, `readln`, `readch` and `seek`
    /// give [`ACCESS_DENIED`], and `eof` is true.
    unreadable: bool,
}

impl Open {
    /// `bytes` opened with the encoding `asked` (empty: none asked); `None` when `asked` is not an
    /// encoding and the file has no byte order mark.
    fn new(bytes: Vec<u8>, asked: &str) -> Option<Self> {
        let (encoding, pos) = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            (Encoding::Utf8, 3)
        } else if bytes.starts_with(&[0xFF, 0xFE]) {
            (Encoding::Utf16Le, 2)
        } else if bytes.starts_with(&[0xFE, 0xFF]) {
            (Encoding::Utf16Be, 2)
        } else if !asked.is_empty() {
            (Encoding::parse(asked)?, 0)
        } else if std::str::from_utf8(&bytes).is_ok() {
            (Encoding::Utf8, 0)
        } else {
            (Encoding::Cp1252, 0)
        };
        Some(Self {
            bytes,
            pos,
            encoding,
            name: encoding.name().to_string(),
            target: None,
            read_from: None,
            dirty: false,
            withdrawn: false,
            unreadable: false,
        })
    }

    /// Writes `text` at the position, over what is there and past the end, when it takes at most
    /// `left` bytes; → the bytes written.
    fn write(&mut self, text: &str, left: usize) -> Result<usize, &'static str> {
        if self.target.is_none() || self.withdrawn {
            return Err(NOT_WRITABLE);
        }
        let bytes = self.encoding.encode(text).ok_or(UNENCODABLE)?;
        if bytes.len() > left {
            return Err(OVER_RUN_WRITES);
        }
        let end = self.pos.saturating_add(bytes.len());
        if end.max(self.bytes.len()) > MAX_FILE_BYTES {
            return Err(TOO_LARGE);
        }
        let over = end.min(self.bytes.len());
        self.bytes.splice(self.pos..over, bytes.iter().copied());
        self.pos = end;
        self.dirty = true;
        Ok(bytes.len())
    }

    /// The character at byte `at` and its length in bytes; `Ok(None)` at the end, `Err` where the
    /// encoding cannot decode it.
    fn char_at(&self, at: usize) -> Result<Option<(char, usize)>, ()> {
        let Some(&b) = self.bytes.get(at) else { return Ok(None) };
        let encoding = self.encoding;
        let unit = |i: usize| -> Option<u16> {
            let pair = self.bytes.get(i..i + 2)?;
            Some(if encoding == Encoding::Utf16Le { u16::from_le_bytes([pair[0], pair[1]]) } else { u16::from_be_bytes([pair[0], pair[1]]) })
        };
        let c = match encoding {
            Encoding::Ascii if b < 0x80 => (b as char, 1),
            Encoding::Ascii => return Err(()),
            Encoding::Binary | Encoding::Latin1 => (b as char, 1),
            Encoding::Cp1252 => {
                let code = if (0x80..0xA0).contains(&b) { u32::from(CP1252_HIGH[usize::from(b - 0x80)]) } else { u32::from(b) };
                (char::from_u32(code).ok_or(())?, 1)
            }
            Encoding::Utf8 => {
                let len = match b {
                    0x00..=0x7F => 1,
                    0xC2..=0xDF => 2,
                    0xE0..=0xEF => 3,
                    0xF0..=0xF4 => 4,
                    _ => return Err(()),
                };
                let s = self.bytes.get(at..at + len).and_then(|s| std::str::from_utf8(s).ok()).ok_or(())?;
                (s.chars().next().ok_or(())?, len)
            }
            Encoding::Utf16Le | Encoding::Utf16Be => {
                let first = unit(at).ok_or(())?;
                if (0xD800..0xDC00).contains(&first) {
                    let second = unit(at + 2).filter(|u| (0xDC00..0xE000).contains(u)).ok_or(())?;
                    let code = 0x10000 + ((u32::from(first) - 0xD800) << 10) + (u32::from(second) - 0xDC00);
                    (char::from_u32(code).ok_or(())?, 4)
                } else {
                    (char::from_u32(u32::from(first)).ok_or(())?, 2)
                }
            }
        };
        Ok(Some(c))
    }

    /// Up to `max` characters from the position (all of them for `None`).
    fn read(&mut self, max: Option<usize>) -> Result<String, ()> {
        let (mut at, mut out) = (self.pos, String::new());
        let mut n = 0;
        while max.is_none_or(|m| n < m) {
            let Some((c, len)) = self.char_at(at)? else { break };
            out.push(c);
            at += len;
            n += 1;
        }
        self.pos = at;
        Ok(out)
    }

    /// The position after a line end that began with `c` and continues at `at` (CR, LF, CRLF or
    /// LFCR). A character there that cannot be decoded is left for the next read.
    fn past_line_end(&self, c: char, at: usize) -> usize {
        let other = if c == '\r' { '\n' } else { '\r' };
        match self.char_at(at) {
            Ok(Some((n, len))) if n == other => at + len,
            _ => at,
        }
    }

    /// The characters up to the next line end, which is passed over.
    fn read_line(&mut self) -> Result<String, ()> {
        let (mut at, mut out) = (self.pos, String::new());
        while let Some((c, len)) = self.char_at(at)? {
            at += len;
            if c == '\r' || c == '\n' {
                at = self.past_line_end(c, at);
                break;
            }
            out.push(c);
        }
        self.pos = at;
        Ok(out)
    }

    /// The next character; a line end is one `"\n"`.
    fn read_char(&mut self) -> Result<String, ()> {
        let Some((c, len)) = self.char_at(self.pos)? else { return Ok(String::new()) };
        let mut at = self.pos + len;
        let c = if c == '\r' || c == '\n' {
            at = self.past_line_end(c, at);
            '\n'
        } else {
            c
        };
        self.pos = at;
        Ok(c.to_string())
    }
}

/// `{text}` for a read that worked, `{error}` for one that did not.
fn text_result(r: Result<String, ()>) -> Data {
    match r {
        Ok(t) => Data::Object(vec![("text".into(), Data::String(t))]),
        Err(()) => error(UNDECODABLE),
    }
}

fn error(message: &str) -> Data {
    Data::Object(vec![("error".into(), Data::String(message.to_string()))])
}

fn arg_text(a: &[Data], i: usize) -> String {
    match a.get(i) {
        Some(Data::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn arg_number(a: &[Data], i: usize) -> Option<f64> {
    match a.get(i) {
        Some(Data::Number(n)) => Some(*n),
        _ => None,
    }
}

fn millis(t: Option<std::time::SystemTime>) -> Data {
    t.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(Data::Null, |d| Data::Number(d.as_millis() as f64))
}

/// One host function of `__vcFiles`, counted and charged as a host call of the run `link` holds.
fn call(link: &super::Link, f: fn(&Host, &[Data]) -> Result<Data, HostError>) -> HostItem {
    let l = link.clone();
    HostItem::Function(Rc::new(move |a| {
        let h = super::current(&l)?;
        bump(&h.counts.file_calls);
        let r = f(&h, &a);
        h.note_written(h.disk.borrow_mut().take_written());
        let r = r?;
        h.charge_call(super::data_size(&r).saturating_add(a.iter().map(super::data_size).sum()))?;
        Ok(r)
    }))
}

/// `Ok(true)`, or `{error}` for a failure.
fn done(r: Result<(), &'static str>) -> Data {
    match r {
        Ok(()) => Data::Bool(true),
        Err(m) => error(m),
    }
}

fn handle(a: &[Data]) -> u32 {
    arg_number(a, 0).filter(|n| n.is_finite() && *n >= 0.0 && *n <= f64::from(u32::MAX)).map_or(0, |n| n as u32)
}

fn with_open(host: &Host, a: &[Data], f: impl FnOnce(&mut Open) -> Data) -> Data {
    match host.disk.borrow_mut().open.get_mut(&handle(a)) {
        Some(file) => f(file),
        None => error(NOT_OPEN),
    }
}

/// [`with_open`] for a call that reads what the file holds: `{error}` [`ACCESS_DENIED`] when the
/// run may no longer read it.
fn with_readable(host: &Host, a: &[Data], f: impl FnOnce(&mut Open) -> Data) -> Data {
    with_open(host, a, |file| if file.unreadable { error(ACCESS_DENIED) } else { f(file) })
}

/// The first two arguments as path text, refused with a `RangeError` when either is too long.
fn two_paths(a: &[Data]) -> Result<(String, String), HostError> {
    let (path, base) = (arg_text(a, 0), arg_text(a, 1));
    if too_long(&path) || too_long(&base) {
        return Err(HostError::Range(PATH_TOO_LONG.to_string()));
    }
    Ok((path, base))
}

fn parse(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let (path, base) = two_paths(a)?;
    let platform = matches!(a.get(2), Some(Data::Bool(true)));
    let disk = host.disk.borrow();
    Ok(disk.pathname(&path, &base, !platform).data())
}

/// The platform path `a[0]` relative to the platform path `a[1]`.
fn relative(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let (path, base) = two_paths(a)?;
    let disk = host.disk.borrow();
    let base = disk.pathname(&base, &disk.start, false);
    Ok(Data::String(disk.pathname(&path, &disk.start, false).relative_to(&base, WINDOWS)))
}

fn stat(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    Ok(match host.disk.borrow().stat(&arg_text(a, 0)) {
        Err(m) => error(m),
        Ok(None) => Data::Object(vec![("exists".into(), Data::Bool(false))]),
        Ok(Some(e)) => Data::Object(vec![
            ("exists".into(), Data::Bool(true)),
            ("dir".into(), Data::Bool(e.dir)),
            ("file".into(), Data::Bool(e.file)),
            ("length".into(), Data::Number(e.len as f64)),
            ("created".into(), millis(e.created)),
            ("modified".into(), millis(e.modified)),
            ("hidden".into(), Data::Bool(e.hidden)),
            ("readonly".into(), Data::Bool(e.readonly)),
            ("alias".into(), Data::Bool(e.link)),
        ]),
    })
}

/// `open(path, encoding, mode)`: `r` reads, `w`, `e` and `a` write.
fn open(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let (path, encoding, mode) = (arg_text(a, 0), arg_text(a, 1), arg_text(a, 2));
    let mut disk = host.disk.borrow_mut();
    let opened = match mode.as_str() {
        "w" | "e" | "a" => disk.open_write(&path, &encoding, &mode),
        _ => disk.open_file(&path, &encoding),
    };
    match opened {
        Ok(id) => Ok(Data::Object(vec![("id".into(), Data::Number(f64::from(id)))])),
        Err(HostError::Error(m)) => Ok(error(&m)),
        Err(e) => Err(e),
    }
}

/// Closes the open file, writing what changed: `true`, `false` when it was not open, or `{error}`
/// when it could not be written. A file that could not be written is kept for the run's end to
/// write again.
fn close(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let mut disk = host.disk.borrow_mut();
    let id = handle(a);
    Ok(match disk.flush(id) {
        Err(m) => error(m),
        Ok(()) => Data::Bool(disk.open.remove(&id).is_some()),
    })
}

/// `write(id, text)`. A write past the run's limit ends the run and leaves the file as it was.
fn write(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let text = arg_text(a, 1);
    let mut disk = host.disk.borrow_mut();
    let left = MAX_RUN_BYTES.saturating_sub(disk.written);
    let r = match disk.open.get_mut(&handle(a)) {
        Some(file) => file.write(&text, left),
        None => Err(NOT_OPEN),
    };
    match r {
        Ok(n) => {
            disk.charge_write(n)?;
            Ok(Data::Bool(true))
        }
        Err(OVER_RUN_WRITES) => Err(HostError::Abort(OVER_RUN_WRITES.to_string())),
        Err(m) => Ok(error(m)),
    }
}

/// `copy(from, to)`: the file `from` to `to`, replacing a file that is there.
fn copy(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let (from, to) = (arg_text(a, 0), arg_text(a, 1));
    let mut disk = host.disk.borrow_mut();
    let source = match disk.stat(&from) {
        Ok(Some(e)) if e.file => disk.checked(&from).ok().and_then(|p| disk.scope.locate(p).ok()),
        Ok(Some(_)) => return Ok(error(NOT_REGULAR)),
        Ok(None) => return Ok(error(NOT_FOUND)),
        Err(m) => return Ok(error(m)),
    };
    let Some(source) = source else { return Ok(error(ACCESS_DENIED)) };
    let target = match disk.target(&to, true) {
        Ok(t) => t,
        Err(m) => return Ok(error(m)),
    };
    if std::fs::metadata(&target).is_ok_and(|m| !m.is_file()) {
        return Ok(error(NOT_REGULAR));
    }
    // A file copied onto itself, by its own name or by another link to it, is already its copy once
    // its open objects are written, and copying could empty it.
    let one = |t: &std::fs::Metadata| std::fs::metadata(&source).is_ok_and(|s| same_file((&source, &s), (&target, t)));
    let itself = real_of(&target).is_some_and(|t| t == source) || std::fs::metadata(&target).is_ok_and(|t| one(&t));
    // The source's open objects write what they hold first, so the copy has it.
    let ids: Vec<u32> = disk.open.iter().filter(|(_, o)| o.target.as_deref() == Some(source.as_path())).map(|(id, _)| *id).collect();
    if let Err(m) = ids.iter().try_for_each(|id| disk.flush(*id)) {
        return Ok(error(m));
    }
    #[cfg(test)]
    if let Some(hook) = BEFORE_COPY_READ.with(|h| h.borrow_mut().take()) {
        hook(Path::new(&from), &source);
    }
    // The bytes are read once, no more than the limit, under the rules for reading, and those bytes
    // are what is charged and written: a source that grows after it is read does not copy more,
    // and a path that leads somewhere else than when it was looked up is refused.
    let copied = match disk.scope.read(Path::new(&from), MAX_FILE_BYTES) {
        Ok((at, bytes)) if at == source => bytes,
        Ok(_) | Err(ScriptRead::Refused) => return Ok(error(ACCESS_DENIED)),
        Err(ScriptRead::TooLarge) => return Ok(error(TOO_LARGE)),
        Err(ScriptRead::Missing) => return Ok(error(NOT_FOUND)),
        Err(ScriptRead::Failed(_)) => return Ok(error(READ_FAILED)),
    };
    // The target's open objects each take a copy of the bytes, which count as written too, as does
    // the copy itself when it is written.
    let holders = disk.open.values().filter(|o| o.target.as_deref() == Some(target.as_path())).count();
    disk.charge_write(copied.len().saturating_mul(holders.saturating_add(usize::from(!itself))))?;
    // The copy takes the source's permissions, as `std::fs::copy` gives them.
    let permissions = std::fs::metadata(&source).map(|m| m.permissions());
    let r =
        if itself { Ok(()) } else { std::fs::write(&target, &copied).and_then(|()| permissions.and_then(|p| std::fs::set_permissions(&target, p))) };
    if !itself && (r.is_ok() || target.exists()) {
        disk.wrote(&target);
    }
    // Open objects of the target hold the copy, so closing them does not write their older bytes.
    if r.is_ok() {
        for file in disk.open.values_mut().filter(|o| o.target.as_deref() == Some(target.as_path())) {
            file.bytes.clone_from(&copied);
            file.pos = file.pos.min(copied.len());
            file.dirty = false;
        }
    }
    Ok(done(r.map_err(|_| WRITE_FAILED)))
}

/// `remove(path)`: the file, or the folder when it is empty; a link itself, never what it leads to.
fn remove(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let mut disk = host.disk.borrow_mut();
    let target = match disk.target(&arg_text(a, 0), false) {
        Ok(t) => t,
        Err(m) => return Ok(error(m)),
    };
    let r = match std::fs::symlink_metadata(&target) {
        Err(_) => Err(NOT_FOUND),
        Ok(m) if m.is_dir() => std::fs::remove_dir(&target)
            .map_err(|_| if std::fs::read_dir(&target).is_ok_and(|mut d| d.next().is_some()) { NOT_EMPTY } else { WRITE_FAILED }),
        Ok(m) => std::fs::remove_file(&target).map_err(|_| WRITE_FAILED).inspect(|()| {
            // A file made later in its place, which may take its inode, gets none of its write bits.
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if m.nlink() <= 1 {
                    disk.cleared.remove(&(m.dev(), m.ino()));
                }
            }
            #[cfg(not(unix))]
            let _ = m;
        }),
    };
    if r.is_ok() {
        // Other open objects of the file drop their changes and take no more writes, so neither
        // their close nor the run's end makes the file again.
        for file in disk.open.values_mut().filter(|o| o.target.as_deref() == Some(target.as_path())) {
            file.dirty = false;
            file.target = None;
        }
        disk.wrote(&target);
    }
    Ok(done(r))
}

/// `rename(path, name)`: the file or folder named `name` in the same folder; → `{fs}`, its new path.
fn rename(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let (path, name) = (arg_text(a, 0), arg_text(a, 1));
    let bad = name.is_empty() || name == "." || name == ".." || name.contains('/') || (WINDOWS && (name.contains('\\') || name.contains(':')));
    if bad || name.contains('\0') {
        return Ok(error(BAD_NAME));
    }
    let Some(new) = Path::new(&path).parent().map(|p| p.join(&name).to_string_lossy().into_owned()) else { return Ok(error(ACCESS_DENIED)) };
    let mut disk = host.disk.borrow_mut();
    let (from, to) = match (disk.target(&path, false), disk.target(&new, false)) {
        (Ok(f), Ok(t)) => (f, t),
        (Err(m), _) | (_, Err(m)) => return Ok(error(m)),
    };
    let Ok(was) = std::fs::symlink_metadata(&from) else { return Ok(error(NOT_FOUND)) };
    // A folder that holds the settings folder is not moved, also when its name is given in another
    // case where case does not matter.
    let holds = |f: &Path| std::fs::canonicalize(&from).map_or(true, |kept| crate::file_access::path_inside(f, &kept));
    if was.is_dir() && disk.fenced.as_deref().is_some_and(holds) {
        return Ok(error(ACCESS_DENIED));
    }
    // A name that differs in case alone may name the same file, which then takes the new name.
    if let Ok(there) = std::fs::symlink_metadata(&to)
        && !(from.to_string_lossy().eq_ignore_ascii_case(&to.to_string_lossy()) && same_file((&from, &was), (&to, &there)))
    {
        return Ok(error(NAME_TAKEN));
    }
    if std::fs::rename(&from, &to).is_err() {
        return Ok(error(WRITE_FAILED));
    }
    // Open objects of the file, or of files in the folder, write to the new path.
    for file in disk.open.values_mut() {
        if let Some(rest) = file.target.as_deref().and_then(|t| t.strip_prefix(&from).ok()) {
            file.target = Some(if rest.as_os_str().is_empty() { to.clone() } else { to.join(rest) });
        }
    }
    // The old path no longer holds the file, so the report names it as well as the new one.
    disk.wrote(&from);
    disk.wrote(&to);
    Ok(Data::Object(vec![("fs".into(), Data::String(new))]))
}

/// Do the paths `a` and `b`, with their metadata, name one file? On Unix by device and inode; on
/// Windows by volume and file index, with links followed; elsewhere by the paths with their links
/// followed, which give the name as the file system keeps it.
fn same_file(a: (&Path, &std::fs::Metadata), b: (&Path, &std::fs::Metadata)) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.1.dev() == b.1.dev() && a.1.ino() == b.1.ino()
    }
    #[cfg(windows)]
    {
        ::same_file::is_same_file(a.0, b.0).unwrap_or(false)
    }
    #[cfg(not(any(unix, windows)))]
    {
        matches!((std::fs::canonicalize(a.0), std::fs::canonicalize(b.0)), (Ok(x), Ok(y)) if x == y)
    }
}

/// `create(path)`: the folder, with the folders above it that are missing.
fn create(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let disk = host.disk.borrow();
    let target = match disk.target(&arg_text(a, 0), true) {
        Ok(t) => t,
        Err(m) => return Ok(error(m)),
    };
    // Each missing folder above it that is made on the way must be a place the script may write too.
    let outside = target.ancestors().skip(1).take_while(|a| !a.as_os_str().is_empty() && std::fs::symlink_metadata(a).is_err()).any(|a| {
        !disk.writes.allows(a) || crate::file_access::check_write(&crate::cmd::findfiles::plain(a.to_path_buf()).to_string_lossy()).is_err()
    });
    if outside {
        return Ok(error(ACCESS_DENIED));
    }
    Ok(done(std::fs::create_dir_all(&target).map_err(|_| WRITE_FAILED)))
}

/// `setLength(path, length)`: the file cut or padded with zero bytes to `length` bytes.
fn set_length(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let n = arg_number(a, 1).filter(|n| n.is_finite() && *n >= 0.0).map_or(0, |n| n as u64);
    if n > MAX_FILE_BYTES as u64 {
        return Ok(error(TOO_LARGE));
    }
    let mut disk = host.disk.borrow_mut();
    let target = match disk.target(&arg_text(a, 0), true) {
        Ok(t) => t,
        Err(m) => return Ok(error(m)),
    };
    let r = match std::fs::metadata(&target) {
        Err(_) => Err(NOT_FOUND),
        Ok(m) if !m.is_file() => Err(NOT_REGULAR),
        Ok(m) => {
            // Each open object of the file holds its own bytes, so what each one grows by counts too.
            let len = usize::try_from(n).unwrap_or(usize::MAX);
            let held = disk
                .open
                .values()
                .filter(|o| o.target.as_deref() == Some(target.as_path()))
                .fold(0usize, |sum, o| sum.saturating_add(len.saturating_sub(o.bytes.len())));
            disk.charge_write(usize::try_from(n.saturating_sub(m.len())).unwrap_or(usize::MAX).saturating_add(held))?;
            // The file's open objects write what they hold first, and then hold the new length.
            let ids: Vec<u32> = disk.open.iter().filter(|(_, o)| o.target.as_deref() == Some(target.as_path())).map(|(id, _)| *id).collect();
            ids.iter()
                .try_for_each(|id| disk.flush(*id))
                .and_then(|()| std::fs::OpenOptions::new().write(true).open(&target).and_then(|f| f.set_len(n)).map_err(|_| WRITE_FAILED))
        }
    };
    if r.is_ok() {
        let len = usize::try_from(n).unwrap_or(usize::MAX);
        for file in disk.open.values_mut().filter(|o| o.target.as_deref() == Some(target.as_path())) {
            file.bytes.resize(len, 0);
            file.pos = file.pos.min(len);
        }
        disk.wrote(&target);
    }
    Ok(done(r))
}

/// `setReadonly(path, readonly)`.
fn set_readonly(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let on = matches!(a.get(1), Some(Data::Bool(true)));
    let mut disk = host.disk.borrow_mut();
    let target = match disk.target(&arg_text(a, 0), true) {
        Ok(t) => t,
        Err(m) => return Ok(error(m)),
    };
    let r = std::fs::metadata(&target).map_err(|_| NOT_FOUND).and_then(|m| {
        let mut p = m.permissions();
        // On Unix setting it clears every write bit; clearing it gives back the bits this run
        // cleared from the file, or else the owner's write bit.
        #[cfg(unix)]
        let (back, file) = {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let (mode, file) = (p.mode(), (m.dev(), m.ino()));
            // Saved bits are given back only to a file that is still without write bits.
            let saved = disk.cleared.get(&file).copied().filter(|_| mode & 0o222 == 0);
            let back = if on { mode & 0o222 } else { saved.unwrap_or(0o200) };
            p.set_mode(if on { mode & !0o222 } else { mode | back });
            (back, file)
        };
        #[cfg(not(unix))]
        p.set_readonly(on);
        std::fs::set_permissions(&target, p).map_err(|_| WRITE_FAILED)?;
        #[cfg(unix)]
        if !on {
            disk.cleared.remove(&file);
        } else if back != 0 && (disk.cleared.len() < MAX_ENTRIES || disk.cleared.contains_key(&file)) {
            disk.cleared.insert(file, back);
        }
        Ok(())
    });
    if r.is_ok() {
        disk.wrote(&target);
    }
    Ok(done(r))
}

/// `pick(kind, prompt, multiple, start)`: the file dialog of `kind` (`open`, `save` or `folder`),
/// which the run waits at in the app; elsewhere `{error}`.
fn pick(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let kind = match arg_text(a, 0).as_str() {
        "open" => PickerKind::Open,
        "save" => PickerKind::Save,
        _ => PickerKind::Folder,
    };
    if cfg!(target_arch = "wasm32") {
        return Ok(error(NO_WEB));
    }
    let mut disk = host.disk.borrow_mut();
    if !disk.framed {
        return Ok(error(NO_PICKER));
    }
    if !host.can_wait.get() {
        let name = match kind {
            PickerKind::Open => "File.openDialog",
            PickerKind::Save => "File.saveDialog",
            PickerKind::Folder => "Folder.selectDialog",
        };
        return Err(HostError::Error(format!(
            "{name}: a file dialog cannot open inside a callback of a built-in function, a getter, a conversion or a file $.evalFile runs; call it from the script's own code"
        )));
    }
    let prompt = super::dialogs::cut_to(&arg_text(a, 1), super::dialogs::MAX_MESSAGE_BYTES).to_string();
    let start = Some(arg_text(a, 3)).filter(|s| !s.is_empty() && !too_long(s));
    let can_always = disk.script.is_some() && disk.data_dir.is_some();
    let multiple = kind == PickerKind::Open && matches!(a.get(2), Some(Data::Bool(true)));
    let filters = if kind == PickerKind::Folder { Vec::new() } else { dialog_filters(&arg_text(a, 4)) };
    disk.picker = Some(ScriptPicker { kind, prompt, multiple, start, filters, can_always });
    Err(HostError::Wait)
}

/// The most file types a dialog's filter gives, and the most extensions of one.
const MAX_FILTERS: usize = 32;
/// The most characters of a dialog's filter that are read.
const MAX_FILTER_CHARS: usize = 4096;

/// The file types of a dialog's `filter`, a string such as `"JavaScript:*.jsx;All files:*.*"`:
/// each `name:pattern` starts a type, a pattern without a name adds to the type before it (the
/// first one starts a type named by the patterns), and patterns are separated by `,` or `;`. A
/// `*.ext` pattern gives the extension `ext`, and `*` and `*.*` give `*`, every file; other patterns
/// are left out, and so is a type left without extensions. Only types that give every file: none
/// (every file).
pub(crate) fn dialog_filters(filter: &str) -> Vec<(String, Vec<String>)> {
    let filter: String = filter.chars().take(MAX_FILTER_CHARS).collect();
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    // Whether the last type was started by a pattern without a name, and is named by its patterns.
    let mut unnamed = false;
    for piece in filter.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()) {
        let (name, pattern) = match piece.split_once(':') {
            Some((n, p)) => (Some(n.trim()), p.trim()),
            None => (None, piece),
        };
        let ext = match pattern {
            "*" | "*.*" => Some("*".to_string()),
            p => p
                .strip_prefix("*.")
                .filter(|e| !e.is_empty() && e.len() <= 32 && e.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.')))
                .map(str::to_string),
        };
        match (name, out.last_mut()) {
            (Some(n), _) => {
                out.push((if n.is_empty() { pattern.to_string() } else { n.to_string() }, Vec::new()));
                unnamed = false;
            }
            (None, None) => {
                out.push((pattern.to_string(), Vec::new()));
                unnamed = true;
            }
            (None, Some((name, _))) if unnamed && ext.is_some() && name.len() < 256 => {
                name.push(';');
                name.push_str(pattern);
            }
            (None, Some(_)) => {}
        }
        if let (Some(ext), Some((_, exts))) = (ext, out.last_mut())
            && exts.len() < MAX_FILTERS
            && !exts.contains(&ext)
        {
            exts.push(ext);
        }
    }
    out.retain(|(_, exts)| !exts.is_empty());
    out.truncate(MAX_FILTERS);
    if out.iter().all(|(_, exts)| exts.iter().all(|e| e == "*")) {
        return Vec::new();
    }
    out
}

fn read(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let max = arg_number(a, 1).filter(|n| *n >= 0.0).map(|n| n as usize);
    Ok(with_readable(host, a, |f| text_result(f.read(max))))
}

fn readln(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    Ok(with_readable(host, a, |f| text_result(f.read_line())))
}

fn readch(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    Ok(with_readable(host, a, |f| text_result(f.read_char())))
}

fn seek(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let (Some(pos), Some(mode)) = (arg_number(a, 1), arg_number(a, 2)) else { return Err(HostError::Type(BAD_SEEK.into())) };
    Ok(with_readable(host, a, |f| {
        let base = match mode as u8 {
            1 => f.pos as f64,
            2 => f.bytes.len() as f64,
            _ => 0.0,
        };
        let to = base + pos.trunc();
        if !(0.0..=f.bytes.len() as f64).contains(&to) {
            return Data::Bool(false);
        }
        f.pos = to as usize;
        Data::Bool(true)
    }))
}

const BAD_SEEK: &str = "File.seek: takes a position and a mode";

fn tell(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    Ok(with_open(host, a, |f| Data::Number(f.pos as f64)))
}

fn eof(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    Ok(Data::Bool(host.disk.borrow().open.get(&handle(a)).is_none_or(|f| f.unreadable || f.pos >= f.bytes.len())))
}

fn encoding(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    Ok(with_open(host, a, |f| Data::String(f.name.clone())))
}

fn set_encoding(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let name = arg_text(a, 1);
    Ok(with_open(host, a, |f| match Encoding::parse(&name) {
        Some(e) => {
            f.encoding = e;
            f.name = e.name().to_string();
            Data::Object(vec![])
        }
        None => error(UNDECODABLE),
    }))
}

fn list(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let disk = host.disk.borrow();
    let path = arg_text(a, 0);
    let mask = Mask::new(&arg_text(a, 1));
    let checked = match disk.checked(&path) {
        Ok(p) => p,
        Err(m) => return Ok(error(m)),
    };
    let steps = std::cell::Cell::new(MAX_MASK_STEPS);
    let spent = std::cell::Cell::new(false);
    let keep = |name: &str| {
        let mut left = steps.get();
        let kept = mask.matches(name, &mut left);
        steps.set(left);
        spent.set(spent.get() || kept.is_none());
        kept.unwrap_or(false)
    };
    let listed = disk.scope.list(checked, MAX_ENTRIES, keep);
    if spent.get() {
        return Ok(error(MASK_TOO_COSTLY));
    }
    let found = match listed {
        Ok(found) => found,
        Err(ScriptRead::TooLarge) => return Ok(error(LISTING_TOO_LARGE)),
        Err(ScriptRead::Missing) => return Ok(error(NOT_FOUND)),
        Err(ScriptRead::Refused) => return Ok(error(ACCESS_DENIED)),
        Err(ScriptRead::Failed(_)) => return Ok(error(READ_FAILED)),
    };
    let entries = found
        .into_iter()
        .map(|(name, dir)| {
            let fs = Path::new(&path).join(&name).to_string_lossy().into_owned();
            Data::Object(vec![("fs".into(), Data::String(fs)), ("dir".into(), Data::Bool(dir))])
        })
        .collect();
    Ok(Data::Object(vec![("entries".into(), Data::Array(entries))]))
}

fn resolve(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let disk = host.disk.borrow();
    let path = arg_text(a, 0);
    let target = match disk.stat(&path) {
        Err(m) => return Ok(error(m)),
        Ok(Some(e)) if e.link => disk.checked(&path).ok().and_then(|p| disk.scope.locate(p).ok()),
        Ok(_) => None,
    };
    Ok(match target {
        Some(real) => {
            let dir = real.is_dir();
            let fs = crate::cmd::findfiles::plain(real).to_string_lossy().into_owned();
            Data::Object(vec![("fs".into(), Data::String(fs)), ("dir".into(), Data::Bool(dir))])
        }
        None => Data::Object(vec![("fs".into(), Data::Null)]),
    })
}

fn execute(host: &Host, _: &[Data]) -> Result<Data, HostError> {
    bump(&host.counts.file_executes);
    Ok(error(NO_EXECUTE))
}

/// A name for a new file in the run's temporary folder.
fn temp_name(host: &Host, _: &[Data]) -> Result<Data, HostError> {
    let mut disk = host.disk.borrow_mut();
    disk.temps += 1;
    let name = format!("vc{:05}.tmp", disk.temps);
    let dir = disk.temp_dir().unwrap_or_else(temp_folder);
    Ok(Data::String(dir.join(name).to_string_lossy().into_owned()))
}

/// `locations(place)`: the folders [`locations`] gives. The run's temporary folder and the script's
/// data folder are made only when `place` names them.
fn locations_of(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let place = arg_text(a, 0);
    let mut disk = host.disk.borrow_mut();
    let shown = |p: Option<std::path::PathBuf>| p.map(|p| p.to_string_lossy().into_owned());
    let temp = if place == "temp" { shown(disk.temp_dir()) } else { None };
    let user_data = if place == "userData" { shown(disk.user_data_dir()) } else { None };
    Ok(locations(&disk.home, temp, user_data))
}

fn encode_text(_: &Host, a: &[Data]) -> Result<Data, HostError> {
    Ok(Data::String(encode(&arg_text(a, 0))))
}

fn decode_text(_: &Host, a: &[Data]) -> Result<Data, HostError> {
    Ok(Data::String(decode(&arg_text(a, 0))))
}

fn encoding_available(_: &Host, a: &[Data]) -> Result<Data, HostError> {
    Ok(Data::Bool(Encoding::parse(&arg_text(a, 0)).is_some()))
}

/// The folder `Folder.current` starts at in the run `host` serves: the script's own folder.
fn start_folder(host: &Host, _: &[Data]) -> Result<Data, HostError> {
    let disk = host.disk.borrow();
    Ok(Data::String(disk.pathname(&disk.start, &disk.start, false).fs()))
}

/// Adds `File` and `Folder` to the realm, their host functions acting for the run `link` holds.
pub(super) fn install(realm: &mut Realm, link: &super::Link) -> Result<(), RunError> {
    let unchecked = |f: fn(&Host, &[Data]) -> Result<Data, HostError>| {
        let l = link.clone();
        HostItem::Function(Rc::new(move |a| {
            let h = super::current(&l)?;
            f(&h, &a)
        }))
    };
    let value = |s: &str| HostItem::Value(Data::String(s.to_string()));
    let text = HostItem::Object(vec![
        ("notOpen".into(), value(NOT_OPEN)),
        ("badMode".into(), value(BAD_MODE)),
        ("noAlias".into(), value(NO_ALIAS)),
        ("noHidden".into(), value(NO_HIDDEN)),
        ("tooLarge".into(), value(TOO_LARGE)),
    ]);
    let system = if WINDOWS {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "Macintosh"
    } else {
        "Unix"
    };
    let items = vec![
        ("text".into(), text),
        ("maxWrite".into(), HostItem::Value(Data::Number(MAX_FILE_BYTES as f64))),
        ("system".into(), value(system)),
        ("lineFeed".into(), value(if WINDOWS { "Windows" } else { "Unix" })),
        ("current".into(), unchecked(start_folder)),
        ("locations".into(), call(link, locations_of)),
        ("encode".into(), call(link, encode_text)),
        ("decode".into(), call(link, decode_text)),
        ("encodingAvailable".into(), call(link, encoding_available)),
        ("parse".into(), call(link, parse)),
        ("relative".into(), call(link, relative)),
        ("tempName".into(), call(link, temp_name)),
        ("stat".into(), call(link, stat)),
        ("open".into(), call(link, open)),
        ("close".into(), call(link, close)),
        ("read".into(), call(link, read)),
        ("readln".into(), call(link, readln)),
        ("readch".into(), call(link, readch)),
        ("seek".into(), call(link, seek)),
        ("tell".into(), call(link, tell)),
        ("eof".into(), call(link, eof)),
        ("encoding".into(), call(link, encoding)),
        ("setEncoding".into(), call(link, set_encoding)),
        ("list".into(), call(link, list)),
        ("resolve".into(), call(link, resolve)),
        ("execute".into(), call(link, execute)),
        ("write".into(), call(link, write)),
        ("copy".into(), call(link, copy)),
        ("remove".into(), call(link, remove)),
        ("rename".into(), call(link, rename)),
        ("create".into(), call(link, create)),
        ("setLength".into(), call(link, set_length)),
        ("setReadonly".into(), call(link, set_readonly)),
        ("pick".into(), call(link, pick)),
    ];
    realm.define_file_classes(items)
}

/// Tests: what the next `copy` does with its source's path, as given and as its real path, after
/// looking it up and before reading it.
#[cfg(test)]
pub(crate) type CopyHook = Box<dyn FnOnce(&Path, &Path)>;

#[cfg(test)]
thread_local! {
    pub(crate) static BEFORE_COPY_READ: std::cell::RefCell<Option<CopyHook>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str, base: &str, windows: bool) -> Pathname {
        let root = Pathname::root(windows);
        let home = Pathname::parse(if windows { r"C:\Users\me" } else { "/home/me" }, &root, &root, windows, false);
        let base = Pathname::parse(base, &home, &home, windows, false);
        Pathname::parse(text, &base, &home, windows, true)
    }

    #[test]
    fn windows_paths_take_both_separators_drives_and_uri_names() {
        let f = p(r"C:\art\one two.txt", r"C:\work", true);
        assert_eq!((f.fs(), f.uri()), (r"C:\art\one two.txt".to_string(), "/c/art/one%20two.txt".to_string()));
        assert_eq!(p("/d/x/y.txt", r"C:\work", true).fs(), r"D:\x\y.txt");
        assert_eq!(p("c:/a/./b/../c", r"C:\work", true).fs(), r"C:\a\c");
        assert_eq!(p("sub/f.txt", r"C:\work", true).fs(), r"C:\work\sub\f.txt");
        assert_eq!(p("~/notes.txt", r"C:\work", true).fs(), r"C:\Users\me\notes.txt");
        assert_eq!(p(r"\\host\share\x.txt", r"C:\work", true).fs(), r"\\host\share\x.txt");
        assert_eq!(p(r"\\?\C:\x.txt", r"C:\work", true).root, Root::Device(r"\\?\C:\x.txt".into()));
        assert_eq!(p("caf%C3%A9.txt", r"C:\w", true).fs(), "C:\\w\\caf\u{e9}.txt");
        assert_eq!(p("..\\..\\..\\..", r"C:\w", true).fs(), r"C:\");
        assert_eq!(p("C:", r"D:\w", true).uri(), "/c");
    }

    #[test]
    fn unix_paths_keep_backslashes_in_names() {
        assert_eq!(p(r"a\b.txt", "/work", false).fs(), r"/work/a\b.txt");
        assert_eq!(p("/x/../y", "/work", false).fs(), "/y");
        assert_eq!(p("~", "/work", false).fs(), "/home/me");
        assert_eq!(p("/", "/work", false).uri(), "/");
    }

    #[test]
    fn relative_uris_climb_to_the_common_folder() {
        let base = p("/a/b", "/", false);
        assert_eq!(p("/a/b/c d.txt", "/", false).relative_to(&base, false), "c%20d.txt");
        assert_eq!(p("/a/x.txt", "/", false).relative_to(&base, false), "../x.txt");
        assert_eq!(p("/a/b", "/", false).relative_to(&base, false), ".");
        assert_eq!(p("D:/x", "C:/", true).relative_to(&p("C:/a", "C:/", true), true), "/d/x");
    }

    #[test]
    fn uri_escapes_are_utf8_both_ways() {
        assert_eq!(encode("caf\u{e9} 1"), "caf%C3%A9%201");
        assert_eq!(encode("a-_.!~*'()/b"), "a-_.!~*'()/b");
        assert_eq!(decode("caf%C3%A9%20x"), "caf\u{e9} x");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%E9"), "%E9", "a lone Latin-1 escape is not UTF-8");
    }

    #[test]
    fn masks_match_stars_and_question_marks_without_case() {
        let matches = |mask: &str, name: &str| Mask::with_case(mask, true).matches(name, &mut { usize::MAX }) == Some(true);
        assert!(matches("*.JSX", "a.jsx"));
        assert!(matches("a?c*", "abcdef"));
        assert!(!matches("*.txt", "a.jsx"));
        assert!(matches("*", ""));
        assert!(matches("*a*b", "xaxxb"));
        assert!(matches("a***b", "ab"));
        assert!(!matches("?", ""));
    }

    #[test]
    fn masks_match_case_where_the_file_system_tells_names_apart_by_case() {
        let matches = |mask: &str, name: &str| Mask::with_case(mask, false).matches(name, &mut { usize::MAX }) == Some(true);
        assert!(!matches("*.JSX", "a.jsx"));
        assert!(matches("*.JSX", "a.JSX"));
        assert!(matches("A?c*", "Abcdef"));
        assert!(!matches("A?c*", "abcdef"));
        assert_eq!(MASK_IGNORES_CASE, cfg!(any(windows, target_os = "macos")));
        assert_eq!(Mask::new("*.JSX").matches("a.jsx", &mut { usize::MAX }), Some(MASK_IGNORES_CASE));
    }

    #[test]
    fn masks_stop_when_their_steps_run_out() {
        let mask = Mask::new(&format!("*{}b", "a".repeat(100)));
        let name = "a".repeat(200);
        let mut steps = 1000;
        assert_eq!(mask.matches(&name, &mut steps), None);
        assert_eq!(steps, 0);
        let mut steps = 1000;
        assert_eq!(Mask::new("*.jsx").matches("a.jsx", &mut steps), Some(true));
        assert!(steps > 990);
    }

    #[test]
    fn decoding_counts_characters_and_takes_four_line_ends() {
        let mut f = Open::new(b"\xEF\xBB\xBFa\xC3\xA9\r\nb\n\rc\rd\ne".to_vec(), "").unwrap();
        assert_eq!((f.name.as_str(), f.pos), ("UTF-8", 3), "the byte order mark is passed over");
        assert_eq!(f.read(Some(2)).unwrap(), "a\u{e9}");
        assert_eq!(f.pos, 6, "two characters, three bytes");
        assert_eq!(f.read_line().unwrap(), "", "CRLF is one line end");
        assert_eq!(f.read_line().unwrap(), "b", "LFCR is one line end");
        assert_eq!(f.read_line().unwrap(), "c");
        assert_eq!(f.read_char().unwrap(), "d");
        assert_eq!(f.read_char().unwrap(), "\n");
        assert_eq!(f.read(None).unwrap(), "e");
        assert_eq!(f.read_char().unwrap(), "");
        let mut b = Open::new(vec![0xE8, 0x41], "BINARY").unwrap();
        assert_eq!(b.read(None).unwrap(), "\u{e8}A");
        let mut a = Open::new(vec![0x41, 0xE8], "ascii").unwrap();
        assert!(a.read(None).is_err());
        assert_eq!(a.pos, 0, "a failed read leaves the position");
        let mut c = Open::new(vec![0x80, 0xE9], "").unwrap();
        assert_eq!(c.name, "CP1252");
        assert_eq!(c.read(None).unwrap(), "\u{20ac}\u{e9}");
        let mut u = Open::new(vec![0xFF, 0xFE, 0x3D, 0xD8, 0x00, 0xDE, 0x41, 0x00], "").unwrap();
        assert_eq!(u.name, "UTF-16LE");
        assert_eq!(u.read(None).unwrap(), "\u{1f600}A");
        assert!(Open::new(b"x".to_vec(), "KLINGON").is_none());
        let mut l = Open::new(vec![b'a', b'\n', 0xE8, b'\r', 0xE8], "ASCII").unwrap();
        assert_eq!(l.read_line().unwrap(), "a", "a line end before a byte ASCII cannot read still ends the line");
        assert!(l.read_line().is_err());
        assert_eq!(l.pos, 2);
        l.pos = 3;
        assert_eq!(l.read_char().unwrap(), "\n");
        assert_eq!(l.pos, 4);
    }
}
