//! The fonts a document uses that aren't available (`text.missingFonts`), the files that provide
//! them in a folder the user picks (`text.findFontFiles`, searched on separate threads:
//! [`super::findfiles`]), and copying the files the user chooses into VectorCraft's own Fonts
//! folder, which the font scan reads (`text.addFontFiles`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use vectorcraft_doc::Document;
use vectorcraft_text::{FontDb, FontMatch, WantedFont, WantedFonts};

use super::findfiles::{self, Limits, Visitor};
use super::*;
use crate::DocState;

/// The most fonts listed, or looked for at once.
const MAX_FONTS: usize = 1_000;
/// The longest family or style name listed or given (bytes).
const MAX_NAME: usize = 256;
/// The CSS generic font families (CSS Fonts Module Level 4), which SVG and HTML files name in place
/// of a font and no font file provides; compared in any ASCII case. The list leaves out
/// `fangsong`, which is also the family name of a Chinese font (FangSong).
const GENERIC_FAMILIES: &[&str] = &[
    "serif",
    "sans-serif",
    "monospace",
    "cursive",
    "fantasy",
    "system-ui",
    "ui-serif",
    "ui-sans-serif",
    "ui-monospace",
    "ui-rounded",
    "math",
    "emoji",
];
/// The longest folder path given (bytes).
const MAX_PATH: usize = 4096;
/// The most files one `text.addFontFiles` copies: as many as a search keeps.
const MAX_ADD_FILES: usize = 1_000;
/// The largest font file a search reads and `text.addFontFiles` copies, and the most bytes one
/// `text.addFontFiles` copies.
pub(crate) const MAX_FONT_FILE: u64 = 256 << 20;
#[cfg(not(target_arch = "wasm32"))]
const MAX_ADD_BYTES: u64 = 1 << 30;
/// The highest number a copy's name gets (`Name 99.otf`).
#[cfg(not(target_arch = "wasm32"))]
const MAX_NUMBER: u32 = 99;
/// The extensions of the font files `text.addFontFiles` copies (in any case), besides suitcase
/// fonts.
#[cfg(not(target_arch = "wasm32"))]
const FONT_EXTENSIONS: &[&str] = &["ttf", "otf", "ttc", "otc", "woff", "woff2"];
/// Why a file wasn't copied.
const NOT_A_FILE: &str = "not a file";
#[cfg(not(target_arch = "wasm32"))]
const NOT_A_FONT: &str = "not a font file";
#[cfg(not(target_arch = "wasm32"))]
const TOO_LARGE: &str = "larger than 256 MB";
#[cfg(not(target_arch = "wasm32"))]
const TOO_MUCH: &str = "the files come to more than 1 GB: add fewer at a time";
#[cfg(not(target_arch = "wasm32"))]
const NO_NAME: &str = "every numbered name for it is taken in the Fonts folder";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "text.missingFonts",
            "Missing Fonts",
            [],
            None,
            "{} → {fonts: [{family, style, status: missing|substitute, resolved: {family, style}}] (sorted by name; at most 1000), count, fontsNextToDocument?} the fonts the active document's type uses, in its layers and its symbols, that aren't available as named: missing (the family is unknown and the fallback family stands in) or substitute (the family is there but not the style; resolved names the stand-in), as text.fonts says. CSS generic families such as sans-serif and names longer than 256 bytes are left out. fontsNextToDocument: the Fonts folder next to the saved document (File › Package writes it), when there is one and a search may start there (not in another app's folders, such as a mail app's attachments)",
            has_doc,
            missing
        ),
        cmd!(
            query "text.findFontFiles",
            "Find Font Files",
            [],
            None,
            "{folder?: an absolute path, fonts?: [{family, style?}] (1 to 1000; default: the active document's missing fonts, as text.missingFonts lists them), maxSeconds?: 60 (1 to 600), stop?: false (true: stop the last search)} with folder: look in it and its subfolders for font files that make the fonts available, on separate threads, and answer at once (state: searching); without folder: the last search's state, with the files found so far. A search reads the table directories and the name, fvar and OS/2 tables of .ttf, .otf, .ttc and .otc files of at most 256 MB and, on macOS, of suitcase fonts (files without an extension or with .suit, whose fonts are in a resource fork of at most 8 MB), and lists a file for a font when the file, added with text.addFontFiles, makes the font resolve exactly as text.fonts resolves it; fonts that resolve exactly already are left out. It doesn't enter app packages, folders named Program Files (also x86 and Arm), ProgramData, $Recycle.Bin or System Volume Information, the system's folders at the root of a volume, the Shared and Public folders in Users, a home folder's Library, AppData, Applications, snap and dot folders, a Library folder's app data folders or the folders the environment names for apps' data, and a search of a folder inside one of them fails; the walk also skips other hidden folders, folders whose contents are in the cloud and folders more than 64 levels deep, and follows no links (a picked path is resolved through its links); none on the web → {id (absent while idle), state: idle|searching|done|stopped|failed, folder?, fonts: [{family, style, status, files: [paths]}], searched?: {folders, files, fontFiles}, skipped?, unreadable?, stopped?: stop|time|limit, error?, seconds?}",
            always,
            find
        ),
        cmd!(
            query "text.addFontFiles",
            "Add Font Files",
            [],
            None,
            "{files: [paths the last text.findFontFiles search found] (1 to 1000)} copy the font files into VectorCraft's own Fonts folder (next to the preferences; the desktop app and vectorcraft-cli read it), never replacing a file there: a file with the same contents is kept, and when another file has that name, the copy gets a number (Name 2.otf); then scan the fonts again, as text.rescanFonts does, and type set in them redraws. Only regular .ttf, .otf, .ttc, .otc, .woff and .woff2 files and suitcase fonts (with their resource fork) are copied, at most 256 MB a file and 1 GB in all; any other file is skipped. Add only fonts you own or are licensed to install; none on the web → {folder, copied: [{from, to}], kept: [{from, to}], skipped: [{file, reason}], families?, faces?}",
            always,
            add
        ),
    ]
}

/// A font a document names that isn't available as named.
#[derive(Clone, Debug, PartialEq)]
pub struct MissingFont {
    pub family: String,
    pub style: String,
    /// [`FontMatch::Missing`] or [`FontMatch::Style`].
    pub status: FontMatch,
    /// The family and style of the face shown instead.
    pub resolved: Option<(String, String)>,
}

/// The fonts the type of `d` uses, in its layers and its symbols, that aren't available as named
/// (sorted by family and style, each once whatever version its type names; at most 1000). CSS generic families such as `sans-serif`
/// ([`GENERIC_FAMILIES`]) and names longer than 256 bytes are left out.
pub fn missing_fonts(d: &Document) -> Vec<MissingFont> {
    let db = FontDb::global();
    let generic = |family: &str| GENERIC_FAMILIES.iter().any(|g| g.eq_ignore_ascii_case(family.trim()));
    let mut used: Vec<(String, String)> = super::fonts::used_fonts(d).into_iter().map(|(family, style, _)| (family, style)).collect();
    // Sorted by family and style: the versions type names of one face are neighbors.
    used.dedup();
    used.into_iter()
        .filter(|(family, style)| !family.trim().is_empty() && !generic(family) && family.len() <= MAX_NAME && style.len() <= MAX_NAME)
        .filter_map(|(family, style)| {
            let resolved = db.resolve(&family, &style);
            let status = resolved.as_ref().map_or(FontMatch::Missing, |(_, m)| *m);
            (status != FontMatch::Exact).then(|| MissingFont {
                family,
                style,
                status,
                resolved: resolved.map(|(f, _)| (f.family.clone(), f.style.clone())),
            })
        })
        .take(MAX_FONTS)
        .collect()
}

/// `text.missingFonts`'s answer for the open document `st`, where searches go by `rules` (this
/// computer's when `None`).
pub fn missing_fonts_of(st: &DocState, rules: Option<&findfiles::Rules>) -> Value {
    let fonts: Vec<Value> = missing_fonts(&st.doc)
        .into_iter()
        .map(|m| {
            json!({
                "family": m.family, "style": m.style, "status": m.status.as_str(),
                "resolved": m.resolved.map(|(family, style)| json!({ "family": family, "style": style })),
            })
        })
        .collect();
    let mut out = json!({ "count": fonts.len(), "fonts": fonts });
    // File › Package puts the fonts in a `Fonts` folder next to the packaged document. The folder
    // is made absolute against the working folder when the document's path is relative. It is
    // offered when a search may start there, as far as the rules tell without listing a folder.
    if let Some(dir) = st.path.as_deref().and_then(|p| Path::new(p).parent()).map(|d| d.join("Fonts")).filter(|d| d.is_dir()) {
        let dir = super::fileio::absolute_path(&dir.to_string_lossy());
        let system;
        let rules = match rules {
            Some(rules) => rules,
            None => {
                system = findfiles::Rules::system();
                &system
            }
        };
        if std::fs::canonicalize(&dir).is_ok_and(|c| rules.refusal_unlisted(&c).is_none()) {
            out["fontsNextToDocument"] = json!(dir);
        }
    }
    out
}

fn missing(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(missing_fonts_of(s.doc()?, s.search_rules.as_ref()))
}

/// A font a search looks for.
#[derive(Clone, Debug)]
pub(crate) struct Sought {
    pub(crate) family: String,
    pub(crate) style: String,
    pub(crate) status: FontMatch,
}

/// The session's last search for font files (`text.findFontFiles`).
pub(crate) struct FontSearch {
    /// The search's number in the session; [`stop`] takes it.
    pub(crate) id: u64,
    /// The folder as given.
    pub(crate) folder: String,
    pub(crate) fonts: Vec<Sought>,
    pub(crate) search: findfiles::Search,
}

/// The files a search for fonts reads: font files, by their names ([`WantedFonts`]).
struct FontFiles(WantedFonts);

impl Visitor for FontFiles {
    fn wants(&self, name: &str) -> bool {
        let path = Path::new(name);
        // A suitcase font (macOS) has no extension, or `.suit`.
        vectorcraft_text::is_font_file(path) || cfg!(target_os = "macos") && path.extension().is_none_or(|e| e.eq_ignore_ascii_case("suit"))
    }
    fn worth_reading(&self, path: &Path) -> bool {
        // A search reads font files and suitcase fonts of at most [`MAX_FONT_FILE`] bytes, the files
        // `text.addFontFiles` copies.
        (vectorcraft_text::is_font_file(path) || vectorcraft_text::is_suitcase(path))
            && std::fs::metadata(path).is_ok_and(|m| m.len() <= MAX_FONT_FILE)
    }
    fn items(&self, path: &Path) -> Vec<usize> {
        self.0.provided_by(path)
    }
}

/// `[{family, style?}]`: 1 to 1000 fonts with a family of 1 to 256 bytes and a style of at most 256
/// (Regular when left out).
fn font_list(v: &Value) -> Option<Vec<(String, String)>> {
    let list = v.as_array().filter(|a| (1..=MAX_FONTS).contains(&a.len()))?;
    list.iter()
        .map(|f| {
            let family = f.get("family")?.as_str().filter(|s| !s.trim().is_empty() && s.len() <= MAX_NAME)?;
            let style = match f.get("style") {
                None | Some(Value::Null) => "Regular",
                Some(s) => s.as_str().filter(|s| s.len() <= MAX_NAME)?,
            };
            Some((family.to_string(), style.to_string()))
        })
        .collect()
}

fn find(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.findFontFiles";
    static NEXT: AtomicU64 = AtomicU64::new(1);
    if bool_or(p, "stop", false) {
        if let Some(f) = &s.font_search {
            f.search.stop();
        }
        return Ok(state(s));
    }
    let Some(folder) = p.get("folder").filter(|f| !f.is_null()) else { return Ok(state(s)) };
    let given = match p.get("fonts") {
        None | Some(Value::Null) => None,
        Some(v) => Some(font_list(v).ok_or_else(|| bad(C, "each of `fonts` is {family, style?}; 1 to 1000 of them"))?),
    };
    let seconds = p.get("maxSeconds").and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(60.0).clamp(1.0, 600.0);
    // Only checked here: the folder is read on a walker thread.
    let folder = folder
        .as_str()
        .filter(|f| !f.is_empty() && f.len() <= MAX_PATH && !f.contains(['\0', '\u{FFFD}']) && Path::new(f).is_absolute())
        .ok_or_else(|| bad(C, "`folder` must be an absolute folder path"))?
        .to_string();
    let fonts = match given {
        Some(fonts) => fonts,
        None => missing_fonts(&s.active().ok_or_else(|| bad(C, "give `fonts`, or open a document"))?.doc)
            .into_iter()
            .map(|m| (m.family, m.style))
            .collect(),
    };
    let db = FontDb::global();
    let (mut sought, mut wanted) = (vec![], vec![]);
    for (family, style) in fonts {
        let (status, installed) = match db.resolve(&family, &style) {
            Some((_, FontMatch::Exact)) => continue,
            Some((f, FontMatch::Style)) => (FontMatch::Style, Some(f.family.clone())),
            _ => (FontMatch::Missing, None),
        };
        wanted.push(WantedFont { family: family.clone(), style: style.clone(), installed });
        sought.push(Sought { family, style, status });
    }
    if sought.is_empty() {
        return Err(bad(C, "no fonts to look for: they are all available"));
    }
    // A new search stops the last one.
    s.font_search = None;
    let threads = s.search_threads.unwrap_or_else(findfiles::threads);
    let limits = Limits { seconds, ..Limits::default() };
    let visitor = Arc::new(FontFiles(WantedFonts::new(&wanted)));
    let search =
        findfiles::start(PathBuf::from(&folder), s.search_rules.clone(), limits, threads, sought.len(), visitor).map_err(EngineError::Other)?;
    s.font_search = Some(FontSearch { id: NEXT.fetch_add(1, Ordering::Relaxed), folder, fonts: sought, search });
    Ok(state(s))
}

/// Where the session's last search stands (`text.findFontFiles` without `folder`).
fn state(s: &Session) -> Value {
    let Some(f) = &s.font_search else { return json!({ "state": "idle", "fonts": [] }) };
    let p = f.search.progress();
    let mut files: Vec<Vec<String>> = vec![vec![]; f.fonts.len()];
    for (path, items) in &p.hits {
        for i in items {
            if let Some(v) = files.get_mut(*i) {
                v.push(path.to_string_lossy().into_owned());
            }
        }
    }
    let fonts: Vec<Value> = f
        .fonts
        .iter()
        .zip(files)
        .map(|(font, mut files)| {
            files.sort();
            json!({ "family": font.family, "style": font.style, "status": font.status.as_str(), "files": files })
        })
        .collect();
    let folder = if p.folder.as_os_str().is_empty() { f.folder.clone() } else { p.folder.to_string_lossy().into_owned() };
    let mut out = json!({
        "id": f.id, "state": p.state(), "folder": folder, "fonts": fonts,
        "searched": { "folders": p.folders, "files": p.files, "fontFiles": p.read },
        "skipped": p.skipped, "unreadable": p.unreadable, "seconds": (p.seconds * 10.0).round() / 10.0,
    });
    if let Some(why) = p.stopped() {
        out["stopped"] = json!(why);
    }
    if let Some(e) = p.error() {
        out["error"] = json!(e);
    }
    out
}

/// Stop the session's search when it is the one numbered `id` (`text.findFontFiles`'s `id`): a
/// newer search, started by someone else, goes on.
pub fn stop(s: &mut Session, id: u64) {
    if let Some(f) = s.font_search.as_ref().filter(|f| f.id == id) {
        f.search.stop();
    }
}

/// What copying font files into a folder did: each file copied (from, to), kept because the same
/// file is there already (from, the file there), or skipped (file, why).
#[derive(Debug, Default)]
pub(crate) struct CopyReport {
    pub(crate) copied: Vec<(PathBuf, PathBuf)>,
    pub(crate) kept: Vec<(PathBuf, PathBuf)>,
    pub(crate) skipped: Vec<(PathBuf, String)>,
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.addFontFiles";
    let mut files: Vec<PathBuf> = p
        .get("files")
        .and_then(Value::as_array)
        .filter(|a| (1..=MAX_ADD_FILES).contains(&a.len()))
        .and_then(|a| a.iter().map(|f| f.as_str().map(PathBuf::from)).collect::<Option<Vec<_>>>())
        .ok_or_else(|| bad(C, "`files` must be 1 to 1000 paths of font files the last text.findFontFiles search found"))?;
    files.sort();
    files.dedup();
    let dir = vectorcraft_text::app_font_dir()
        .ok_or_else(|| EngineError::Other("VectorCraft's Fonts folder isn't set here: the desktop app and vectorcraft-cli set it".into()))?;
    let found: Vec<PathBuf> =
        s.font_search.as_ref().map(|f| f.search.progress().hits.into_iter().map(|(path, _)| path).collect()).unwrap_or_default();
    if let Some(f) = files.iter().find(|f| !found.contains(f)) {
        return Err(bad(C, format!("`{}` is not a font file the last text.findFontFiles search found", f.display())));
    }
    for f in &files {
        crate::file_access::check_read(&f.to_string_lossy()).map_err(EngineError::Other)?;
    }
    // VectorCraft's own Fonts folder: no automation root applies to it.
    crate::file_access::unconfined(|| super::fileio::create_dir(&dir.to_string_lossy()))?;
    let report = copy_into(&files, dir);
    let pairs = |v: &[(PathBuf, PathBuf)]| {
        v.iter().map(|(from, to)| json!({ "from": from.to_string_lossy(), "to": to.to_string_lossy() })).collect::<Vec<_>>()
    };
    let mut out = json!({
        "folder": dir.to_string_lossy(), "copied": pairs(&report.copied), "kept": pairs(&report.kept),
        "skipped": report.skipped.iter().map(|(file, why)| json!({ "file": file.to_string_lossy(), "reason": why })).collect::<Vec<_>>(),
    });
    if !report.copied.is_empty() || !report.kept.is_empty() {
        // The fonts are cataloged and type set in them redraws, as Refresh Font List does it.
        let r = super::fonts::rescan(s, &Value::Null)?;
        out["families"] = r["families"].clone();
        out["faces"] = r["faces"].clone();
    }
    Ok(out)
}

/// Where [`copy_one`] put a file: copied to, or kept as, the file in the folder.
#[cfg(not(target_arch = "wasm32"))]
enum Placed {
    Copied(PathBuf),
    Kept(PathBuf),
}

/// Copy `files` into `dir`, never replacing a file there: a file with the same contents is kept,
/// and when another file has the name, the copy gets the first free name from `Name 2.otf` to
/// `Name 99.otf`. At most [`MAX_FONT_FILE`] bytes a file and 1 GB in all.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn copy_into(files: &[PathBuf], dir: &Path) -> CopyReport {
    let mut report = CopyReport::default();
    let mut total = 0;
    for from in files {
        match copy_one(from, dir, &mut total) {
            Ok(Placed::Copied(to)) => report.copied.push((from.clone(), to)),
            Ok(Placed::Kept(to)) => report.kept.push((from.clone(), to)),
            Err(why) => report.skipped.push((from.clone(), why)),
        }
    }
    report
}

/// The web has no file system: nothing is copied.
#[cfg(target_arch = "wasm32")]
pub(crate) fn copy_into(files: &[PathBuf], _: &Path) -> CopyReport {
    CopyReport { skipped: files.iter().map(|f| (f.clone(), NOT_A_FILE.to_string())).collect(), ..CopyReport::default() }
}

#[cfg(not(target_arch = "wasm32"))]
fn copy_one(from: &Path, dir: &Path, total: &mut u64) -> std::result::Result<Placed, String> {
    use std::io::ErrorKind;
    let meta = std::fs::symlink_metadata(from).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(NOT_A_FILE.into());
    }
    let font = from.extension().and_then(|e| e.to_str()).is_some_and(|e| FONT_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)));
    // A suitcase font's fonts are in its resource fork, which is copied with it, whatever its name.
    let fork = match (vectorcraft_text::is_suitcase(from), font) {
        (true, _) => Some(read_fork(from)?),
        (false, true) => None,
        (false, false) => return Err(NOT_A_FONT.into()),
    };
    let len = fork.as_ref().map_or(meta.len(), |f| f.len() as u64);
    if len > MAX_FONT_FILE {
        return Err(TOO_LARGE.into());
    }
    if total.saturating_add(len) > MAX_ADD_BYTES {
        return Err(TOO_MUCH.into());
    }
    let (Some(name), Some(stem)) = (from.file_name(), from.file_stem()) else { return Err(NOT_A_FILE.into()) };
    let ext = from.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for n in 1..=MAX_NUMBER {
        let to = if n == 1 { dir.join(name) } else { dir.join(format!("{} {n}{ext}", stem.to_string_lossy())) };
        match std::fs::symlink_metadata(&to) {
            Err(e) if e.kind() == ErrorKind::NotFound => match create_copy(from, &to, fork.as_deref()) {
                Ok(()) => {
                    *total += len;
                    return Ok(Placed::Copied(to));
                }
                // Taken meanwhile: the next name.
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.to_string()),
            },
            Ok(m) if m.is_file() && same_file(from, &to, m.len(), len, fork.as_deref()) => return Ok(Placed::Kept(to)),
            _ => {}
        }
    }
    Err(NO_NAME.into())
}

/// Create `to` as a copy of the font file `from`, or for a suitcase font, as an empty file whose
/// resource fork is `fork`; never in place of a file.
#[cfg(not(target_arch = "wasm32"))]
fn create_copy(from: &Path, to: &Path, fork: Option<&[u8]>) -> std::io::Result<()> {
    let Some(fork) = fork else { return vectorcraft_format::write_new_with(to, |f| copy_capped(from, f)) };
    vectorcraft_format::write_new_with(to, |_| Ok(()))?;
    std::fs::write(fork_path(to), fork).inspect_err(|_| {
        // Best effort: the empty file just made is all there is to clean up.
        let _ = std::fs::remove_file(to);
    })
}

/// Whether the file `to` (`to_len` bytes) holds what the copy of `from` (`len` bytes, or the
/// resource fork `fork` of a suitcase font) would.
#[cfg(not(target_arch = "wasm32"))]
fn same_file(from: &Path, to: &Path, to_len: u64, len: u64, fork: Option<&[u8]>) -> bool {
    match fork {
        None => to_len == len && same_contents(from, to),
        Some(fork) => to_len == 0 && read_fork(to).is_ok_and(|f| f == fork),
    }
}

/// The resource fork of the file at `path` (where macOS keeps a suitcase font's fonts), at most
/// [`MAX_FONT_FILE`] bytes.
#[cfg(not(target_arch = "wasm32"))]
fn read_fork(path: &Path) -> std::result::Result<Vec<u8>, String> {
    use std::io::Read;
    let mut fork = vec![];
    std::fs::File::open(fork_path(path)).and_then(|f| f.take(MAX_FONT_FILE + 1).read_to_end(&mut fork)).map_err(|e| e.to_string())?;
    if fork.len() as u64 > MAX_FONT_FILE {
        return Err(TOO_LARGE.into());
    }
    Ok(fork)
}

/// Where macOS shows the resource fork of the file at `path`.
#[cfg(not(target_arch = "wasm32"))]
fn fork_path(path: &Path) -> PathBuf {
    path.join("..namedfork/rsrc")
}

/// Copy the file `from` into `to`, failing past [`MAX_FONT_FILE`] bytes (a file that grew since
/// it was measured).
#[cfg(not(target_arch = "wasm32"))]
fn copy_capped(from: &Path, to: &mut std::fs::File) -> std::io::Result<()> {
    use std::io::Read;
    let copied = std::io::copy(&mut std::fs::File::open(from)?.take(MAX_FONT_FILE + 1), to)?;
    if copied > MAX_FONT_FILE {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, TOO_LARGE));
    }
    Ok(())
}

/// Whether the files `a` and `b` hold the same bytes (read 64 KB at a time).
#[cfg(not(target_arch = "wasm32"))]
fn same_contents(a: &Path, b: &Path) -> bool {
    let (Ok(mut a), Ok(mut b)) = (std::fs::File::open(a), std::fs::File::open(b)) else { return false };
    let (mut x, mut y) = (vec![0; 1 << 16], vec![0; 1 << 16]);
    loop {
        match (fill(&mut a, &mut x), fill(&mut b, &mut y)) {
            (Ok(0), Ok(0)) => return true,
            (Ok(n), Ok(m)) if n == m && x.get(..n) == y.get(..m) => {}
            _ => return false,
        }
    }
}

/// Read from `f` until `buf` is full or the file ends: the bytes read.
#[cfg(not(target_arch = "wasm32"))]
fn fill(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    use std::io::Read;
    let mut n = 0;
    while let Some(rest) = buf.get_mut(n..).filter(|r| !r.is_empty()) {
        match f.read(rest)? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}
