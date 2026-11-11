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
//!   [`MAX_OPEN`] files open at once.
//! - The encoding of an opened file is the one its byte order mark gives (UTF-8, UTF-16LE or
//!   UTF-16BE), otherwise the one the script set, otherwise UTF-8 when the bytes are valid UTF-8,
//!   otherwise CP1252. `read`, `readln` and `readch` count characters and `seek` and `tell` bytes;
//!   `readln` and `readch` take CR, LF, CRLF and LFCR as one line end, and `readch` gives a line end
//!   as `"\n"`. A character the encoding cannot decode fails the read with [`UNDECODABLE`] and
//!   leaves the position where it was.
//! - `getFiles` lists at most [`MAX_ENTRIES`] entries, sorted by name, and looks at most
//!   [`crate::file_access::MAX_LISTED`] names; a mask matches `*` and `?` without regard to case,
//!   in at most [`MAX_MASK_STEPS`] steps for one listing.
//! - A path longer than [`MAX_PATH_CHARS`] characters is refused before it is split into names.
//! - Writing (`open` with `w`, `e` or `a`, `write`, `copy`, `remove`, `rename`, `create` and the
//!   property setters), the file and folder dialogs and `execute` are not available: they return
//!   `false` or `null` and set `error`. `execute` is counted in the report's `fileExecutes`.
//! - In a wasm build every lookup fails with [`NO_WEB`].

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use vectorcraft_js::{Data, HostError, HostItem, Realm, RunError};

use super::{Host, bump};
use crate::file_access::{ScriptRead, ScriptScope};

/// The most bytes of one file a script opens.
pub const MAX_FILE_BYTES: usize = 8 << 20;
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
pub(crate) const NO_WRITES: &str = "Script file writing is not available";
pub(crate) const NO_PICKER: &str = "Script file dialogs are not available";
pub(crate) const NO_EXECUTE: &str = "Script files cannot be opened in another application";
pub(crate) const NO_WEB: &str = "Script file access is unavailable on the web";
pub(crate) const TOO_MANY_OPEN: &str = "Script has 256 files open";
pub(crate) const PATH_TOO_LONG: &str = "Script file path exceeds 32,767 characters";
pub(crate) const MASK_TOO_COSTLY: &str = "Script folder mask takes too many steps to match";
const OVER_RUN_BYTES: &str = "the script opened more file data than one run may open (32 MiB)";

/// The script's file state: its scope, where `Folder.current` starts, the files it has open and how
/// much it has read.
pub(crate) struct Disk {
    scope: ScriptScope,
    home: String,
    start: String,
    open: HashMap<u32, Open>,
    next: u32,
    temps: u32,
    bytes: usize,
}

impl Disk {
    /// The state of a run whose own files are read in `scope`.
    pub(crate) fn new(scope: &ScriptScope) -> Self {
        let home = home_folder();
        let start = scope.folder().map_or_else(|| home.clone(), |p| p.to_string_lossy().into_owned());
        Self { scope: scope.clone(), home, start, open: HashMap::new(), next: 1, temps: 0, bytes: 0 }
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
        if self.bytes.saturating_add(entry.len as usize) > MAX_RUN_BYTES {
            return Err(HostError::Abort(OVER_RUN_BYTES.to_string()));
        }
        let bytes = match self.scope.read(Path::new(path), MAX_FILE_BYTES) {
            Ok((_, bytes)) => bytes,
            Err(ScriptRead::TooLarge) => return fail(TOO_LARGE),
            Err(ScriptRead::Missing) => return fail(NOT_FOUND),
            Err(ScriptRead::Refused) => return fail(ACCESS_DENIED),
            Err(ScriptRead::Failed(_)) => return fail(READ_FAILED),
        };
        if self.bytes.saturating_add(bytes.len()) > MAX_RUN_BYTES {
            return Err(HostError::Abort(OVER_RUN_BYTES.to_string()));
        }
        self.bytes += bytes.len();
        let file = Open::new(bytes, encoding).ok_or_else(|| HostError::Error(UNDECODABLE.to_string()))?;
        let id = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        self.open.insert(id, file);
        Ok(id)
    }
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

/// The folders `Folder.temp`, `Folder.desktop` and the others name, `null` where there is none.
/// On the web every one is `null`, and no platform folder is looked up.
fn locations(home: &str) -> Data {
    if cfg!(target_arch = "wasm32") {
        return Data::Object(PLACES.iter().map(|k| (k.to_string(), Data::Null)).collect());
    }
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let join = |a: &str, b: &str| Path::new(a).join(b).to_string_lossy().into_owned();
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned()));
    let temp = temp_folder().to_string_lossy().into_owned();
    let (user_data, app_data, common, system, trash) = if WINDOWS {
        (env("APPDATA"), env("PROGRAMDATA"), env("CommonProgramFiles"), env("SystemRoot"), None)
    } else if cfg!(target_os = "macos") {
        (
            Some(join(home, "Library/Application Support")),
            Some("/Library/Application Support".to_string()),
            Some("/Library/Application Support".to_string()),
            Some("/System".to_string()),
            Some(join(home, ".Trash")),
        )
    } else {
        let data = env("XDG_DATA_HOME").unwrap_or_else(|| join(home, ".local/share"));
        (Some(data.clone()), Some("/usr/share".to_string()), Some("/usr/share".to_string()), Some("/".to_string()), Some(join(&data, "Trash")))
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
        ("temp", Some(temp)),
        ("trash", trash),
        ("userData", user_data),
    ];
    Data::Object(places.into_iter().map(|(k, v)| (k.to_string(), s(v))).collect())
}

/// The names [`locations`] gives.
const PLACES: [&str; 10] = ["appData", "appPackage", "commonFiles", "desktop", "myDocuments", "startup", "system", "temp", "trash", "userData"];

/// The folder for temporary files: the platform's, or on the web the root, where nothing is read.
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

/// A `getFiles` mask, where `*` stands for any run of characters and `?` for one, without regard
/// to case, read once for a whole listing.
struct Mask {
    /// The mask in lower case, each run of `*` written as one.
    chars: Vec<char>,
    /// How many of its characters are not `*`: a name with fewer characters does not match.
    fixed: usize,
}

impl Mask {
    fn new(mask: &str) -> Self {
        let mut chars: Vec<char> = Vec::new();
        for c in mask.to_lowercase().chars() {
            if !(c == '*' && chars.last() == Some(&'*')) {
                chars.push(c);
            }
        }
        let fixed = chars.iter().filter(|c| **c != '*').count();
        Self { chars, fixed }
    }

    /// Does `name` match, taking a step from `steps` for each character compared? `None` when the
    /// steps run out first.
    fn matches(&self, name: &str, steps: &mut usize) -> Option<bool> {
        let n: Vec<char> = name.to_lowercase().chars().collect();
        if n.len() < self.fixed {
            return Some(false);
        }
        matched(&self.chars, &n, steps)
    }
}

/// Does `n` match the lower-case mask `m`? `None` when `steps` run out first.
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

/// An opened file: its bytes, the byte position and the encoding it is read in.
struct Open {
    bytes: Vec<u8>,
    pos: usize,
    encoding: Encoding,
    name: String,
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
        Some(Self { bytes, pos, encoding, name: encoding.name().to_string() })
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

/// One host function of `__vcFiles`, counted and charged as a host call.
fn call(host: &Rc<Host>, f: fn(&Host, &[Data]) -> Result<Data, HostError>) -> HostItem {
    let h = host.clone();
    HostItem::Function(Rc::new(move |a| {
        bump(&h.counts.file_calls);
        let r = f(&h, &a)?;
        h.charge_call(super::data_size(&r).saturating_add(a.iter().map(super::data_size).sum()))?;
        Ok(r)
    }))
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

fn open(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    match host.disk.borrow_mut().open_file(&arg_text(a, 0), &arg_text(a, 1)) {
        Ok(id) => Ok(Data::Object(vec![("id".into(), Data::Number(f64::from(id)))])),
        Err(HostError::Error(m)) => Ok(error(&m)),
        Err(e) => Err(e),
    }
}

fn close(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    Ok(Data::Bool(host.disk.borrow_mut().open.remove(&handle(a)).is_some()))
}

fn read(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let max = arg_number(a, 1).filter(|n| *n >= 0.0).map(|n| n as usize);
    Ok(with_open(host, a, |f| text_result(f.read(max))))
}

fn readln(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    Ok(with_open(host, a, |f| text_result(f.read_line())))
}

fn readch(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    Ok(with_open(host, a, |f| text_result(f.read_char())))
}

fn seek(host: &Host, a: &[Data]) -> Result<Data, HostError> {
    let (Some(pos), Some(mode)) = (arg_number(a, 1), arg_number(a, 2)) else { return Err(HostError::Type(BAD_SEEK.into())) };
    Ok(with_open(host, a, |f| {
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
    Ok(Data::Bool(host.disk.borrow().open.get(&handle(a)).is_none_or(|f| f.pos >= f.bytes.len())))
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

fn temp_name(host: &Host, _: &[Data]) -> Result<Data, HostError> {
    let mut disk = host.disk.borrow_mut();
    disk.temps += 1;
    let name = format!("vc{:05}.tmp", disk.temps);
    Ok(Data::String(temp_folder().join(name).to_string_lossy().into_owned()))
}

fn locations_of(host: &Host, _: &[Data]) -> Result<Data, HostError> {
    Ok(locations(&host.disk.borrow().home))
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

/// Adds `File` and `Folder` to the realm.
pub(super) fn install(realm: &mut Realm, host: &Rc<Host>) -> Result<(), RunError> {
    let start = {
        let disk = host.disk.borrow();
        disk.pathname(&disk.start, &disk.start, false).fs()
    };
    let value = |s: &str| HostItem::Value(Data::String(s.to_string()));
    let text = HostItem::Object(vec![
        ("notOpen".into(), value(NOT_OPEN)),
        ("noWrites".into(), value(NO_WRITES)),
        ("noPicker".into(), value(NO_PICKER)),
        ("badMode".into(), value(BAD_MODE)),
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
        ("system".into(), value(system)),
        ("lineFeed".into(), value(if WINDOWS { "Windows" } else { "Unix" })),
        ("current".into(), HostItem::Function(Rc::new(move |_| Ok(Data::String(start.clone()))))),
        ("locations".into(), call(host, locations_of)),
        ("encode".into(), call(host, encode_text)),
        ("decode".into(), call(host, decode_text)),
        ("encodingAvailable".into(), call(host, encoding_available)),
        ("parse".into(), call(host, parse)),
        ("relative".into(), call(host, relative)),
        ("tempName".into(), call(host, temp_name)),
        ("stat".into(), call(host, stat)),
        ("open".into(), call(host, open)),
        ("close".into(), call(host, close)),
        ("read".into(), call(host, read)),
        ("readln".into(), call(host, readln)),
        ("readch".into(), call(host, readch)),
        ("seek".into(), call(host, seek)),
        ("tell".into(), call(host, tell)),
        ("eof".into(), call(host, eof)),
        ("encoding".into(), call(host, encoding)),
        ("setEncoding".into(), call(host, set_encoding)),
        ("list".into(), call(host, list)),
        ("resolve".into(), call(host, resolve)),
        ("execute".into(), call(host, execute)),
    ];
    realm.define_file_classes(items)
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
        let matches = |mask: &str, name: &str| Mask::new(mask).matches(name, &mut { usize::MAX }) == Some(true);
        assert!(matches("*.JSX", "a.jsx"));
        assert!(matches("a?c*", "abcdef"));
        assert!(!matches("*.txt", "a.jsx"));
        assert!(matches("*", ""));
        assert!(matches("*a*b", "xaxxb"));
        assert!(matches("a***b", "ab"));
        assert!(!matches("?", ""));
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
