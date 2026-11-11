//! A script's preprocessor lines, read before it runs, as the public guide lists them
//! (<https://extendscript.docsforadobe.dev/extendscript-tools-features/preprocessor-directives/>):
//! `#include` and `//@include` put the text of another file in place of the line, `#includepath`
//! sets the folders `#include` looks in, `#script` names the run, `#strict on` turns on the checks
//! of `$.strict` (<https://extendscript.docsforadobe.dev/extendscript-tools-features/dollar-object/>:
//! a write to a read-only property is a runtime error, and objects that take no new properties
//! refuse them), and `#target` and `#strict off` are accepted. The same lines are read in a file
//! `$.evalFile` runs.
//!
//! VectorCraft's own rules:
//!
//! - A line counts only at the start of a line (blanks before it), outside strings and comments, as
//!   the lexer reads it, and each file must be a whole script on its own.
//! - An included file is looked for beside the file that includes it, then in each `#includepath`
//!   folder in turn (a relative one is relative to that same file's folder), then in each folder of
//!   the `JSINCLUDE` environment variable. A later `#includepath` replaces the list.
//! - Files are read through the script's [`ScriptScope`]: a refused path ends the run, except a
//!   `JSINCLUDE` folder outside the scope, which is passed over.
//! - `#target` takes any value and changes nothing. `#engine` is refused as unsupported, and any
//!   other `#name` as unknown; `//@name` with another name stays a comment.
//! - `#targetengine name` names the engine the script runs in ([`Expanded::engine`]). Only the
//!   script itself may have one, at most once, and only before its first code or `#include` line:
//!   blanks, comments and the other preprocessor lines may come before it. The name is an unquoted
//!   name of ASCII letters, digits and `_` that does not start with a digit, or a quoted one of 1 to
//!   [`MAX_ENGINE_NAME_BYTES`] bytes without control characters, compared as written.
//! - The last `#strict on` or `#strict off` of the script and the files it includes sets the checks
//!   for the whole run; without one they are off. A write to a property that has only a getter is
//!   checked as a read-only one, the objects that take no new properties are those that are not
//!   extensible (`Object.preventExtensions`, `Object.seal`, `Object.freeze`), and the rest of ES5
//!   strict mode is left to `'use strict'`. Code that `eval` and `Function` compile takes the
//!   checks of the code that compiles it; a file `$.evalFile` runs takes its own `#strict` line if
//!   it has one, and otherwise the caller's.
//! - A run reads at most [`MAX_DEPTH`] levels of nested files, [`MAX_FILES`] files,
//!   [`MAX_TOTAL_BYTES`] of source in all (the script's own included) and 8 MiB a file, and looks up
//!   at most [`MAX_PROBES`] paths. A file may be included again once its earlier inclusion has
//!   ended; one that includes itself, directly or not, is an error.
//!
//! The expanded source keeps a [`SourceMap`], so an error is reported at the line and column of
//! the file it is in.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use vectorcraft_js::{DirectiveName, Options};

use crate::file_access::{ScriptRead, ScriptScope};

/// The most files nested inside each other, below the script.
pub const MAX_DEPTH: usize = 16;
/// The most files a run includes or evaluates.
pub const MAX_FILES: usize = 64;
/// The most bytes of source a run reads, its own included.
pub const MAX_TOTAL_BYTES: usize = 32 << 20;
/// The most paths a run looks up for its files.
pub const MAX_PROBES: usize = 1024;
/// The longest value a preprocessor line may have, in bytes.
const MAX_VALUE_BYTES: usize = 4096;
/// The longest name `#targetengine` may give, in bytes of UTF-8.
pub const MAX_ENGINE_NAME_BYTES: usize = 64;

/// Where the lines of an expanded source came from: file 0 is the file expanded, the others the
/// files it included.
#[derive(Clone, Debug, Default)]
pub(crate) struct SourceMap {
    files: Vec<Arc<str>>,
    segments: Vec<Segment>,
}

/// From line `out_line` of the expanded source on, the lines are those of file `file` from line
/// `line`; on `out_line` itself the columns are `shift` units further right.
#[derive(Clone, Copy, Debug)]
struct Segment {
    out_line: u32,
    file: usize,
    line: u32,
    shift: u32,
}

impl SourceMap {
    /// Line `line`, column `column` of the expanded source → the file it is in (its index and name)
    /// and the line and column there.
    pub(crate) fn locate(&self, line: u32, column: u32) -> (usize, Arc<str>, u32, u32) {
        let at = self.segments.partition_point(|s| s.out_line <= line);
        let Some(seg) = at.checked_sub(1).and_then(|i| self.segments.get(i)) else {
            return (0, self.files.first().cloned().unwrap_or_else(|| Arc::from("")), line, column);
        };
        let column = if line == seg.out_line { column.saturating_sub(seg.shift).max(1) } else { column };
        let name = self.files.get(seg.file).cloned().unwrap_or_else(|| Arc::from(""));
        (seg.file, name, seg.line.saturating_add(line - seg.out_line), column)
    }
}

/// How a preprocessor failure ends the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailKind {
    /// A line that is not a valid preprocessor line, or a file that is not a valid script.
    Syntax,
    /// A file that is missing, refused or unreadable, or a cycle.
    Error,
    /// One of the run's limits.
    Limit,
}

/// A preprocessor failure, at line `line`, column `column` of the file `file` (index `index` of
/// the expansion's [`SourceMap`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    pub kind: FailKind,
    pub message: String,
    pub index: usize,
    pub file: Arc<str>,
    pub line: u32,
    pub column: u32,
}

/// A source with its preprocessor lines read: the text to run, where its lines came from, the
/// name its `#script` line gave, what its last `#strict` line set (`None` without one), and the
/// engine its `#targetengine` line named, with that line's line and column.
pub(crate) struct Expanded {
    pub source: String,
    pub map: SourceMap,
    pub name: Option<String>,
    pub strict: Option<bool>,
    pub engine: Option<(String, u32, u32)>,
}

/// What the preprocessor lines of a file have named so far: its `#script` name and its engine, and
/// whether code or an `#include` line has come before.
#[derive(Default)]
struct Named {
    name: Option<String>,
    engine: Option<(String, u32, u32)>,
    code_seen: bool,
}

/// Are `units` only blanks and comments?
fn only_blanks_and_comments(units: &[u16]) -> bool {
    let mut i = 0;
    while let Some(&u) = units.get(i) {
        match u {
            0x2F if units.get(i + 1) == Some(&0x2F) => {
                while units.get(i).is_some_and(|&c| !matches!(c, 0x0A | 0x0D | 0x2028 | 0x2029)) {
                    i += 1;
                }
            }
            0x2F if units.get(i + 1) == Some(&0x2A) => {
                let Some(end) = units.get(i + 2..).and_then(|rest| rest.windows(2).position(|w| w == [0x2A, 0x2F])) else { return false };
                i += end + 4;
            }
            0xFEFF => i += 1,
            u if char::from_u32(u32::from(u)).is_some_and(char::is_whitespace) => i += 1,
            _ => return false,
        }
    }
    true
}

/// The engine name of a `#targetengine` line's value `value` (as written, quotes included), or what
/// is wrong with it.
fn engine_name(value: &str) -> Result<String, &'static str> {
    let name = match value.chars().next() {
        None => return Err("needs a name"),
        Some(q @ ('"' | '\'')) => {
            let inner = value.get(1..).and_then(|v| v.strip_suffix(q)).ok_or("needs a closing quote after the name")?;
            if inner.contains(q) {
                return Err("takes one name");
            }
            if inner.chars().any(char::is_control) {
                return Err("needs a name without control characters");
            }
            inner
        }
        _ => {
            let mut chars = value.chars();
            let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
            if !first_ok || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err("needs a name of letters, digits and _, or a quoted name");
            }
            value
        }
    };
    if name.is_empty() {
        return Err("needs a name");
    }
    if name.len() > MAX_ENGINE_NAME_BYTES {
        return Err("needs a name of at most 64 bytes");
    }
    Ok(name.to_string())
}

/// A file being run: its name for people and the folder its relative paths start from (none for a
/// script given as text).
struct Frame {
    shown: Arc<str>,
    dir: Option<PathBuf>,
}

/// The files of one run: its scope, the `#includepath` list, what it has read and the files open
/// now, innermost last.
pub(crate) struct Files {
    scope: ScriptScope,
    jsinclude: Vec<PathBuf>,
    include_path: Vec<String>,
    files: usize,
    bytes: usize,
    probes: usize,
    /// The real paths of the files open now, for cycles.
    chain: Vec<PathBuf>,
    /// How many of them are below the script.
    levels: usize,
    /// The files `$.evalFile` is running, the script first.
    frames: Vec<Frame>,
}

/// The output being built: its UTF-16 text, the line it is on, the units since that line began,
/// whether its last unit is a carriage return, and what the last `#strict` line so far set.
struct Out {
    units: Vec<u16>,
    line: u32,
    column: u32,
    after_cr: bool,
    map: SourceMap,
    strict: Option<bool>,
}

impl Out {
    fn emit(&mut self, text: &[u16]) {
        for &u in text {
            match u {
                0x0A if self.after_cr => self.column = 0,
                0x0A | 0x0D | 0x2028 | 0x2029 => {
                    self.line = self.line.saturating_add(1);
                    self.column = 0;
                }
                _ => self.column = self.column.saturating_add(1),
            }
            self.after_cr = u == 0x0D;
        }
        self.units.extend_from_slice(text);
    }

    /// The lines from here on come from file `file`, from its line `line`.
    fn begin(&mut self, file: usize, line: u32) {
        self.map.segments.push(Segment { out_line: self.line, file, line, shift: self.column });
    }

    /// Ends the line the output is on, unless it has just ended.
    fn end_line(&mut self) {
        if self.after_cr || !matches!(self.units.last(), Some(0x0A | 0x2028 | 0x2029)) {
            self.emit(&[0x0A]);
        }
    }
}

/// A file's text read as a script: a byte order mark is left out, and compiled scripts and bytes
/// that are not UTF-8 are refused.
fn decode(bytes: &[u8], value: &str) -> Result<String, String> {
    if bytes.starts_with(b"@JSXBIN@") {
        return Err(format!("script file is compiled: {value}"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| format!("script file is not UTF-8 text: {value}"))?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(text).to_string())
}

/// `value` without the single or double quotes around it, or `None` when a quote is not closed.
fn unquote(value: &str) -> Option<&str> {
    match value.chars().next() {
        Some(q @ ('"' | '\'')) => value.get(1..).and_then(|v| v.strip_suffix(q)),
        _ => Some(value),
    }
}

/// The folder the file at `path` is in, without Windows' `\\?\` prefix: under that prefix `..` and
/// `/` in the names joined to it would be read as plain characters.
fn folder_of(path: &Path) -> Option<PathBuf> {
    path.parent().map(|p| crate::cmd::findfiles::plain(p.to_path_buf()))
}

/// Does the path `p` have a `..` name, with `/` or `\` between names?
fn has_parent(p: &str) -> bool {
    p.split(['/', '\\']).any(|name| name == "..")
}

/// A real path as people read it.
fn display(path: &Path) -> Arc<str> {
    Arc::from(crate::cmd::findfiles::plain(path.to_path_buf()).to_string_lossy().as_ref())
}

impl Files {
    /// The files of a run of the script at `path` (`None` for a script given as text), with the
    /// `JSINCLUDE` folders `jsinclude`.
    pub(crate) fn new(path: Option<&str>, jsinclude: Vec<PathBuf>) -> Self {
        let real = path.and_then(|p| std::fs::canonicalize(p).ok());
        let dir = real.as_deref().and_then(folder_of);
        Self {
            scope: dir.as_deref().map(ScriptScope::of_folder).unwrap_or_default(),
            jsinclude,
            include_path: Vec::new(),
            files: 0,
            bytes: 0,
            probes: 0,
            chain: real.into_iter().collect(),
            levels: 0,
            frames: vec![Frame { shown: Arc::from(path.unwrap_or_default()), dir }],
        }
    }

    /// The `JSINCLUDE` folders of this process: absolute folders separated by `;`.
    pub(crate) fn jsinclude_env() -> Vec<PathBuf> {
        #[cfg(test)]
        if let Some(list) = super::TEST_JSINCLUDE.with(|t| t.borrow().clone()) {
            return list;
        }
        std::env::var("JSINCLUDE").map(|v| v.split(';').map(str::trim).filter(|p| !p.is_empty()).map(PathBuf::from).collect()).unwrap_or_default()
    }

    /// The file being run now, as `$.fileName` gives it (empty for a script given as text).
    pub(crate) fn file_name(&self) -> String {
        self.frames.last().map(|f| f.shown.to_string()).unwrap_or_default()
    }

    /// The `#includepath` list, as `$.includePath` gives it.
    pub(crate) fn include_path(&self) -> String {
        self.include_path.join(";")
    }

    /// The script's own text, `text`, with its preprocessor lines read.
    pub(crate) fn expand_script(&mut self, text: &str) -> Result<Expanded, Failure> {
        self.bytes = self.bytes.saturating_add(text.len());
        let shown = self.frames.first().map(|f| f.shown.clone()).unwrap_or_else(|| Arc::from(""));
        let dir = self.frames.first().and_then(|f| f.dir.clone());
        self.expand(shown, dir, text, true)
    }

    /// The file `path` names for `$.evalFile`, relative to the file being run now, read and with its
    /// preprocessor lines read. It becomes the file being run until [`Files::end_eval`].
    pub(crate) fn eval_file(&mut self, path: &str) -> Result<Expanded, Failure> {
        let current = self.frames.last().map(|f| f.shown.clone()).unwrap_or_else(|| Arc::from(""));
        let fail = |kind, message: String| Failure { kind, message, index: 0, file: current.clone(), line: 0, column: 0 };
        let dir = self.frames.last().and_then(|f| f.dir.clone());
        let (real, bytes) = self.open(path, dir.as_deref(), false).map_err(|(kind, message)| fail(kind, message))?;
        let text = self.admit(&real, bytes, path).map_err(|(kind, message)| fail(kind, message))?;
        let shown = display(&real);
        self.chain.push(real.clone());
        self.levels += 1;
        self.frames.push(Frame { shown: shown.clone(), dir: folder_of(&real) });
        let r = self.expand(shown, folder_of(&real), &text, false);
        if r.is_err() {
            self.end_eval();
        }
        r
    }

    /// The file `$.evalFile` ran has ended.
    pub(crate) fn end_eval(&mut self) {
        if self.frames.len() > 1 {
            self.frames.pop();
            self.chain.pop();
            self.levels = self.levels.saturating_sub(1);
        }
    }

    /// Checks a file read at `real` against the run's limits and the files open now, and decodes
    /// it. `value` is the path as the script wrote it.
    fn admit(&mut self, real: &Path, bytes: Vec<u8>, value: &str) -> Result<String, (FailKind, String)> {
        if self.chain.iter().any(|p| p == real) {
            return Err((FailKind::Error, format!("include cycle: {value}")));
        }
        if self.levels >= MAX_DEPTH {
            return Err((FailKind::Limit, format!("include depth limit ({MAX_DEPTH}) reached")));
        }
        if self.files >= MAX_FILES {
            return Err((FailKind::Limit, format!("include file limit ({MAX_FILES}) reached")));
        }
        if self.bytes.saturating_add(bytes.len()) > MAX_TOTAL_BYTES {
            return Err((FailKind::Limit, "script source limit reached (32 MiB)".to_string()));
        }
        self.files += 1;
        self.bytes += bytes.len();
        decode(&bytes, value).map_err(|m| (FailKind::Error, m))
    }

    /// Finds and reads the file `value` names, from the file in folder `dir`; with `search`, also in
    /// the `#includepath` and `JSINCLUDE` folders.
    fn open(&mut self, value: &str, dir: Option<&Path>, search: bool) -> Result<(PathBuf, Vec<u8>), (FailKind, String)> {
        let path = Path::new(value);
        let refused = || (FailKind::Error, format!("script file read refused: {value}"));
        // A script given as text has no folder, and so no file it may read.
        let Some(dir) = dir.filter(|_| !has_parent(value)) else { return Err(refused()) };
        // Each candidate, and whether the scope's refusal passes it over.
        let mut candidates: Vec<(Option<PathBuf>, bool)> = vec![(Some(dir.join(path)), false)];
        if search && !path.is_absolute() {
            candidates.extend(self.include_path.iter().map(|p| ((!has_parent(p)).then(|| dir.join(p).join(path)), false)));
            candidates.extend(self.jsinclude.iter().filter(|p| p.is_absolute()).map(|p| (Some(p.join(path)), true)));
        }
        for (candidate, passed_over_when_refused) in candidates {
            let Some(candidate) = candidate else { return Err(refused()) };
            self.probes += 1;
            if self.probes > MAX_PROBES {
                return Err((FailKind::Limit, format!("include search limit ({MAX_PROBES} paths) reached")));
            }
            match self.scope.read(&candidate, crate::script::MAX_SOURCE_BYTES) {
                Ok(found) => return Ok(found),
                Err(ScriptRead::Missing) => {}
                Err(ScriptRead::Refused) if passed_over_when_refused => {}
                Err(ScriptRead::Refused) => return Err(refused()),
                Err(ScriptRead::TooLarge) => return Err((FailKind::Limit, format!("script file is too large: {value}"))),
                Err(ScriptRead::Failed(e)) => return Err((FailKind::Error, format!("cannot read script file {value}: {e}"))),
            }
        }
        Err((FailKind::Error, format!("include file not found: {value}")))
    }

    /// `text`, the file `shown` in folder `dir`, with its preprocessor lines read. With `script`,
    /// its `#script` line names the run.
    fn expand(&mut self, shown: Arc<str>, dir: Option<PathBuf>, text: &str, script: bool) -> Result<Expanded, Failure> {
        let mut out = Out { units: Vec::with_capacity(text.len()), line: 1, column: 0, after_cr: false, map: SourceMap::default(), strict: None };
        let mut named = Named::default();
        self.file(&mut out, shown, dir.as_deref(), text, script, &mut named)?;
        let source = String::from_utf16(&out.units).map_err(|_| Failure {
            kind: FailKind::Syntax,
            message: "the script is not valid text".into(),
            index: 0,
            file: out.map.files.first().cloned().unwrap_or_else(|| Arc::from("")),
            line: 0,
            column: 0,
        })?;
        Ok(Expanded { source, map: out.map, name: named.name, strict: out.strict, engine: named.engine })
    }

    /// Adds file `shown` (in folder `dir`, with text `text`) to `out`, its included files in place
    /// of their lines.
    fn file(&mut self, out: &mut Out, shown: Arc<str>, dir: Option<&Path>, text: &str, script: bool, named: &mut Named) -> Result<(), Failure> {
        let index = out.map.files.len();
        out.map.files.push(shown.clone());
        let units: Vec<u16> = text.encode_utf16().collect();
        out.begin(index, 1);
        // A source with neither `#` nor `@` has no preprocessor line.
        if !units.iter().any(|&u| u == u16::from(b'#') || u == u16::from(b'@')) {
            out.emit(&units);
            return Ok(());
        }
        let fail = |kind, message: String, line, column| Failure { kind, message, index, file: shown.clone(), line, column };
        let options = Options { preprocessor_lines: true, max_source_units: MAX_TOTAL_BYTES, ..Options::default() };
        let program = vectorcraft_js::parse_utf16(&units, &options).map_err(|e| fail(FailKind::Syntax, e.message, e.line, e.column))?;
        let mut cursor = 0usize;
        for d in &program.directives {
            let (start, end) = (d.span.start as usize, d.span.end as usize);
            let line_start =
                units.get(..start).and_then(|u| u.iter().rposition(|&c| matches!(c, 0x0A | 0x0D | 0x2028 | 0x2029))).map_or(0, |i| i + 1);
            let column = u32::try_from(start - line_start + 1).unwrap_or(u32::MAX);
            let at = |kind, message: String| fail(kind, message, d.line, column);
            let before = units.get(cursor..start).unwrap_or_default();
            if !named.code_seen && !only_blanks_and_comments(before) {
                named.code_seen = true;
            }
            out.emit(before);
            cursor = end;
            let label = format!("{}{}", d.form, d.name);
            if d.value.len() > MAX_VALUE_BYTES {
                return Err(at(FailKind::Syntax, format!("the value of {label} is too long")));
            }
            let value = unquote(&d.value).ok_or_else(|| at(FailKind::Syntax, format!("unterminated quote in {label}")))?;
            match d.known {
                Some(DirectiveName::Include) => {
                    if value.is_empty() {
                        return Err(at(FailKind::Syntax, format!("{label} needs a file name")));
                    }
                    named.code_seen = true;
                    let (real, bytes) = self.open(value, dir, true).map_err(|(kind, message)| at(kind, message))?;
                    let included = self.admit(&real, bytes, value).map_err(|(kind, message)| at(kind, message))?;
                    self.chain.push(real.clone());
                    self.levels += 1;
                    let r = self.file(out, display(&real), folder_of(&real).as_deref(), &included, false, named);
                    self.chain.pop();
                    self.levels -= 1;
                    r?;
                    out.end_line();
                    out.begin(index, d.line);
                }
                Some(DirectiveName::IncludePath) => {
                    self.include_path = value.split(';').map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect();
                }
                Some(DirectiveName::Script) => {
                    if value.is_empty() {
                        return Err(at(FailKind::Syntax, format!("{label} needs a name")));
                    }
                    if script && index == 0 {
                        named.name = Some(value.to_string());
                    }
                }
                Some(DirectiveName::Target) => {
                    if value.is_empty() {
                        return Err(at(FailKind::Syntax, format!("{label} needs a value")));
                    }
                }
                Some(DirectiveName::Strict) => match value.to_ascii_lowercase().as_str() {
                    "off" => out.strict = Some(false),
                    "on" => out.strict = Some(true),
                    _ => return Err(at(FailKind::Syntax, format!("{label} takes on or off"))),
                },
                Some(DirectiveName::TargetEngine) => {
                    if !script || index > 0 {
                        return Err(at(FailKind::Syntax, format!("{label} may only be in the script itself, not in a file it runs")));
                    }
                    if named.engine.is_some() {
                        return Err(at(FailKind::Syntax, format!("the script has more than one {label} line")));
                    }
                    if named.code_seen {
                        return Err(at(FailKind::Syntax, format!("{label} must come before the script's code and #include lines")));
                    }
                    let engine = engine_name(&d.value).map_err(|why| at(FailKind::Syntax, format!("{label} {why}")))?;
                    named.engine = Some((engine, d.line, column));
                }
                None if d.name.eq_ignore_ascii_case("engine") => {
                    return Err(at(FailKind::Syntax, format!("unsupported script directive: {label}")));
                }
                None => return Err(at(FailKind::Syntax, format!("unknown script directive: {label}"))),
            }
        }
        out.emit(units.get(cursor..).unwrap_or_default());
        Ok(())
    }
}
