//! Where a folder search goes: not into other apps' or the system's folders. They are told by
//! their names and places (an app package, a home folder's `Library` or `AppData`, the system
//! folders at the root of a volume), never by the name of an app. Names and paths compare in
//! lowercase.

use std::path::{Component, Path, PathBuf};

/// Extensions of the packages apps and media libraries keep their files in.
const PACKAGE_EXTENSIONS: &[&str] = &[
    "app",
    "appex",
    "bundle",
    "component",
    "fcpbundle",
    "framework",
    "imovielibrary",
    "kext",
    "musiclibrary",
    "photolibrary",
    "photoslibrary",
    "plugin",
    "tvlibrary",
    "xpc",
];

/// The folders Windows installs programs and keeps their data and the system's in, wherever they
/// are.
const INSTALL_FOLDERS: &[&str] =
    &["program files", "program files (x86)", "program files (arm)", "programdata", "$recycle.bin", "system volume information"];

/// The folders a home folder keeps apps and their data in (besides its hidden ones).
const HOME_APP_FOLDERS: &[&str] = &["library", "appdata", "applications", "snap"];

/// The folders a `Library` folder keeps apps' data in (macOS).
const LIBRARY_APP_FOLDERS: &[&str] = &["application support", "containers", "group containers", "caches", "preferences"];

/// The folders beside the home folders that apps share (macOS `Shared`, Windows `Public`).
const SHARED_FOLDERS: &[&str] = &["shared", "public"];

/// The system's folders at the root of a macOS volume (or a copy of one).
const MAC_SYSTEM: &[&str] =
    &["applications", "bin", "cores", "dev", "etc", "home", "library", "net", "network", "opt", "private", "sbin", "system", "tmp", "usr", "var"];

/// The system's folders at the root of a Linux or BSD system (or a copy of one).
const UNIX_SYSTEM: &[&str] = &[
    "bin",
    "boot",
    "dev",
    "efi",
    "etc",
    "gnu",
    "lib",
    "lib32",
    "lib64",
    "libx32",
    "lost+found",
    "nix",
    "opt",
    "proc",
    "root",
    "run",
    "sbin",
    "snap",
    "sys",
    "usr",
    "var",
];

/// The system's folders at the root of a Windows drive (or a copy of one), besides the install
/// folders.
const WINDOWS_SYSTEM: &[&str] =
    &["windows", "windows.old", "recovery", "perflogs", "boot", "config.msi", "msocache", "$windows.~bt", "$windows.~ws", "$winreagent", "$sysreset"];

/// The most entries of a folder read to tell what kind of folder it is.
const MAX_SIBLINGS: usize = 100_000;

/// The names of the folders that show a folder is the root of a system ([`system_folders`]).
const SYSTEM_SIGNS: &[&str] = &["users", "system", "library", "applications", "windows", "program files", "etc", "usr", "bin", "sbin"];

/// Which of [`SYSTEM_SIGNS`] are among the names of a folder's folders and links.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Siblings(u16);

impl Siblings {
    /// The folders and links named `names` (in any case).
    #[cfg(test)]
    pub(crate) fn of(names: &[&str]) -> Self {
        let mut s = Self::default();
        names.iter().for_each(|n| s.note(n));
        s
    }

    /// Note a folder or link named `name` (in any case).
    pub(crate) fn note(&mut self, name: &str) {
        if let Some(i) = SYSTEM_SIGNS.iter().position(|s| s.eq_ignore_ascii_case(name)) {
            self.0 |= 1 << i;
        }
    }

    /// Whether a folder or link named `name`, one of [`SYSTEM_SIGNS`], was noted.
    fn has(self, name: &str) -> bool {
        SYSTEM_SIGNS.iter().position(|s| *s == name).is_some_and(|i| self.0 & (1 << i) != 0)
    }
}

/// Why a search may not start in a folder ([`Rules::refusal`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The folder is in other apps' or the system's folders, or in an app package.
    Apps,
    /// The folder is in the Shared or Public folder of a `Users` folder.
    Shared,
}

/// A folder a walk lists, as the rules see it ([`Rules::listed`]).
pub(crate) struct Listed {
    /// The folder, in lowercase.
    dir: PathBuf,
    /// Its name, in lowercase.
    name: String,
    /// The system folders in it, when it is the root of a system.
    system: Option<&'static [&'static str]>,
    /// It is a home folder.
    home: bool,
}

/// Where a search may go.
#[derive(Clone, Debug, Default)]
pub struct Rules {
    /// The home folder (canonical).
    pub home: Option<PathBuf>,
    /// Folders searched when picked, whatever holds them (canonical): the temporary folders, and
    /// tests' folders.
    pub user_content: Vec<PathBuf>,
    /// Other apps' and the system's folders the environment names (canonical): never entered, and a
    /// search of a folder inside one fails.
    pub refused: Vec<PathBuf>,
}

/// `path` in lowercase and, on Windows, without its `\\?\` prefix ([`unprefixed`]), for comparing
/// paths.
fn lower(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let plain = if cfg!(windows) { unprefixed(&text) } else { None };
    PathBuf::from(plain.as_deref().unwrap_or(&text).to_lowercase())
}

/// The system's folders a volume's root holds, when its folders (`siblings`) show it is the root of
/// an operating system.
fn system_folders(siblings: Siblings) -> Option<&'static [&'static str]> {
    let has = |name: &str| siblings.has(name);
    if has("users") && (has("system") || has("library") || has("applications")) {
        Some(MAC_SYSTEM)
    } else if has("windows") && (has("users") || has("program files")) {
        Some(WINDOWS_SYSTEM)
    } else if has("etc") && has("usr") && (has("bin") || has("sbin")) {
        Some(UNIX_SYSTEM)
    } else {
        None
    }
}

/// What the names of the folders and links in `dir` show ([`Siblings`]).
fn folder_siblings(dir: &Path) -> Siblings {
    let mut siblings = Siblings::default();
    for e in std::fs::read_dir(dir).into_iter().flatten().take(MAX_SIBLINGS).flatten() {
        let name = e.file_name();
        if let Some(name) = name.to_str()
            && e.file_type().is_ok_and(|t| t.is_dir() || t.is_symlink())
        {
            siblings.note(name);
        }
    }
    siblings
}

/// The first `n` components of `path`.
fn prefix(path: &Path, n: usize) -> PathBuf {
    path.components().take(n).collect()
}

/// `path` without the `\\?\` prefix that `std::fs::canonicalize` gives paths on Windows: `C:\…`
/// for `\\?\C:\…` and `\\server\share\…` for `\\?\UNC\server\share\…`, when every name in it can
/// be written without the prefix ([`without_verbatim`]). Other paths, and paths on other systems,
/// are returned as they are.
pub fn plain(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    match path.to_str().and_then(without_verbatim) {
        Some(p) => PathBuf::from(p),
        None => path,
    }
}

/// The Windows path `p` without its `\\?\` prefix, when it has one for a drive or a share: `C:\…`
/// for `\\?\C:\…` and `\\server\…` for `\\?\UNC\server\…`.
pub(crate) fn unprefixed(p: &str) -> Option<String> {
    if let Some(unc) = p.strip_prefix(r"\\?\UNC\") {
        return Some(format!(r"\\{unc}"));
    }
    let disk = p.strip_prefix(r"\\?\")?;
    matches!(disk.as_bytes(), [d, b':', b'\\', ..] if d.is_ascii_alphabetic()).then(|| disk.to_string())
}

/// [`unprefixed`], when each name in the path can be written without the prefix ([`portable`]):
/// a share's server and share names, and the names below a drive's root or a share.
pub(crate) fn without_verbatim(p: &str) -> Option<String> {
    let plain = unprefixed(p)?;
    let (names, at_least) = match plain.strip_prefix(r"\\") {
        Some(unc) => (unc, 2),
        None => (plain.get(3..)?, 0),
    };
    let mut names: Vec<&str> = if names.is_empty() { vec![] } else { names.split('\\').collect() };
    // A trailing separator.
    if names.len() > at_least && names.last() == Some(&"") {
        names.pop();
    }
    let ok = names.len() >= at_least && names.iter().all(|n| portable(n));
    ok.then_some(plain)
}

/// Whether `name` can be a name in a Windows path without the `\\?\` prefix: not `.` or `..`, not
/// ending in a dot or a space, without `<>:"/\|?*` or control characters, and not a device name
/// (`CON`, `PRN`, `AUX`, `NUL`, `COM0` to `COM9`, `LPT0` to `LPT9`, these with a superscript digit,
/// `CONIN$`, `CONOUT$`), with an extension or not.
fn portable(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end().to_ascii_lowercase();
    let numbered = |device: &str| {
        stem.strip_prefix(device).is_some_and(|n| {
            let mut n = n.chars();
            n.next().is_some_and(|c| c.is_ascii_digit() || "¹²³".contains(c)) && n.next().is_none()
        })
    };
    let device = ["con", "prn", "aux", "nul", "conin$", "conout$"].contains(&stem.as_str()) || numbered("com") || numbered("lpt");
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.ends_with(['.', ' '])
        && !name.chars().any(|c| c < ' ' || "<>:\"/\\|?*".contains(c))
        && !device
}

impl Rules {
    /// This computer's: the home folder, the temporary folders, and the app and system folders the
    /// environment names.
    pub fn system() -> Self {
        let canonical = |p: PathBuf| p.is_absolute().then(|| std::fs::canonicalize(&p).ok()).flatten();
        let var = |name: &str| std::env::var_os(name).map(PathBuf::from).and_then(canonical);
        let home = var(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
        let user_content = [std::env::temp_dir(), PathBuf::from("/tmp")].into_iter().filter_map(canonical).collect();
        let names: &[&str] = if cfg!(windows) {
            &["APPDATA", "LOCALAPPDATA", "ProgramData", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "SystemRoot"]
        } else {
            &[
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_CACHE_HOME",
                "XDG_STATE_HOME",
                "HOST_XDG_CONFIG_HOME",
                "HOST_XDG_DATA_HOME",
                "HOST_XDG_CACHE_HOME",
                "HOST_XDG_STATE_HOME",
            ]
        };
        let refused = names.iter().filter_map(|n| var(n)).collect();
        Rules { home, user_content, refused }
    }

    /// Whether `dir` is a home folder: the home folder, a folder in a `Users` or `home` folder, or
    /// a folder the Flatpak document portal exports (`/run/user/<uid>/doc/<id>/<name>`).
    fn is_home(&self, dir: &Path) -> bool {
        if self.home.as_ref().is_some_and(|h| lower(h) == lower(dir)) {
            return true;
        }
        let above = dir.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().to_lowercase());
        if matches!(above.as_deref(), Some("users" | "home")) {
            return true;
        }
        let parts: Vec<String> = dir.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect();
        matches!(parts.as_slice(), [_, run, user, uid, doc, _, _] if run == "run" && user == "user" && is_number(uid) && doc == "doc")
    }

    /// The folder `dir`, whose folders show `siblings`, as the rules see it when a walk lists it:
    /// told once for all the folders in it.
    pub(crate) fn listed(&self, dir: &Path, siblings: Siblings) -> Listed {
        Listed {
            dir: lower(dir),
            name: dir.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default(),
            system: system_folders(siblings),
            home: self.is_home(dir),
        }
    }

    /// Whether a search never enters the folder `name` in the listed folder `parent`: an app's
    /// package, a Windows install folder, a system folder at the root of a volume, the folders apps
    /// share beside the home folders, a `Library` folder's app data, or a home folder's hidden
    /// folders and app folders.
    pub(crate) fn never_in(&self, parent: &Listed, name: &str) -> bool {
        let name = name.to_lowercase();
        let name = name.as_str();
        if Path::new(name).extension().and_then(|e| e.to_str()).is_some_and(|e| PACKAGE_EXTENSIONS.contains(&e)) || INSTALL_FOLDERS.contains(&name) {
            return true;
        }
        if parent.system.is_some_and(|system| system.contains(&name)) {
            return true;
        }
        match parent.name.as_str() {
            "users" if SHARED_FOLDERS.contains(&name) => return true,
            "library" if LIBRARY_APP_FOLDERS.contains(&name) => return true,
            _ => {}
        }
        parent.home && (name.starts_with('.') || HOME_APP_FOLDERS.contains(&name))
    }

    /// [`Self::never_in`] the folder `parent`, whose folders show `siblings`.
    #[cfg(test)]
    pub(crate) fn never(&self, parent: &Path, siblings: Siblings, name: &str) -> bool {
        self.never_in(&self.listed(parent, siblings), name)
    }

    /// The home folder, when the listed folder `parent` is the root of a system whose rules skip
    /// the folder the home folder is in (`/usr/home/me` on FreeBSD, `/var/home/me`): a walk of the
    /// root reaches it this way.
    pub(crate) fn home_below(&self, parent: &Listed) -> Option<PathBuf> {
        let home = self.home.as_ref()?;
        let system = parent.system?;
        let below = lower(home).strip_prefix(&parent.dir).ok()?.components().next()?.as_os_str().to_string_lossy().into_owned();
        system.contains(&below.as_str()).then(|| home.clone())
    }

    /// Whether `path` is one of the refused folders.
    pub(crate) fn is_refused(&self, path: &Path) -> bool {
        let path = lower(path);
        self.refused.iter().any(|r| lower(r) == path)
    }

    /// The folders that hold user content whatever holds them, as far as they hold `folder`: the
    /// home folder, [`Self::user_content`], the cloud storage folders in the home folder's
    /// `Library` (an app's own iCloud `Documents` folder too), removable media and a user's runtime
    /// folder on Linux (network shares, the document portal).
    fn bases(&self, folder: &Path) -> Vec<PathBuf> {
        let mut v = self.user_content.clone();
        let lowered = lower(folder);
        if let Some(home) = &self.home {
            v.push(home.clone());
            let library = home.join("Library");
            v.push(library.join("CloudStorage"));
            let mobile = library.join("Mobile Documents");
            v.push(mobile.join("com~apple~CloudDocs"));
            let depth = mobile.components().count();
            if lowered.starts_with(lower(&mobile)) && lowered.components().nth(depth + 1).is_some_and(|c| c.as_os_str() == "documents") {
                v.push(prefix(folder, depth + 2));
            }
        }
        v.push(PathBuf::from("/run/media"));
        let parts: Vec<String> = folder.components().take(4).map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect();
        if matches!(parts.as_slice(), [_, run, user, uid] if run == "run" && user == "user" && is_number(uid)) {
            v.push(prefix(folder, 4));
        }
        v
    }

    /// Why a search may not start in `folder` (canonical), or `None` when it may: the deepest base
    /// ([`Self::bases`]) or refused folder that holds it is a refused one, or a folder below that
    /// base (or below the root, without one) is one a walk doesn't enter.
    pub(crate) fn refusal(&self, folder: &Path) -> Option<Refusal> {
        self.refusal_with(folder, folder_siblings)
    }

    /// [`Self::refusal`] without listing a folder: the root of a system, told by the folders it
    /// holds, isn't recognized.
    pub(crate) fn refusal_unlisted(&self, folder: &Path) -> Option<Refusal> {
        self.refusal_with(folder, |_| Siblings::default())
    }

    /// [`Self::refusal`], with `siblings` telling what a folder's folders show.
    fn refusal_with(&self, folder: &Path, siblings: impl Fn(&Path) -> Siblings) -> Option<Refusal> {
        let lowered = lower(folder);
        let held = |p: &PathBuf| lowered.starts_with(lower(p)).then(|| p.components().count());
        let base = self.bases(folder).iter().filter_map(held).max();
        let refused = self.refused.iter().filter_map(held).max();
        let start = match (base, refused) {
            (_, Some(r)) if base.is_none_or(|b| r >= b) => return Some(Refusal::Apps),
            (Some(b), _) => b,
            // From the root of the folder's volume.
            (None, _) => folder.components().take_while(|c| matches!(c, Component::Prefix(_) | Component::RootDir)).count(),
        };
        let mut parent = prefix(folder, start);
        for c in folder.components().skip(start) {
            let child = parent.join(c);
            let listed = self.listed(&parent, siblings(&parent));
            let name = c.as_os_str().to_string_lossy().to_lowercase();
            if listed.name == "users" && SHARED_FOLDERS.contains(&name.as_str()) {
                return Some(Refusal::Shared);
            }
            if self.never_in(&listed, &name) || self.is_refused(&child) {
                return Some(Refusal::Apps);
            }
            parent = child;
        }
        None
    }
}

/// Whether `s` is a number (a user id).
fn is_number(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}
