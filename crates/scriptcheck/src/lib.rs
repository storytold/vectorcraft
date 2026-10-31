//! Static checker for scripts written for the reference app's scripting: `.jsx`, `.jsxinc` and
//! `.js` files, and compiled `.jsxbin` files. [`check`] reads a script's text without running any
//! of it and reports what kind of file it is ([`Kind`]), its preprocessor directives, the platform
//! features and document-model member chains it uses, what it would do outside the open document,
//! the menu-command strings it passes, the files it includes and the problems that would stop it
//! loading.
//!
//! The lexer reads ECMAScript 3 (ECMA-262 3rd edition,
//! <https://ecma-international.org/wp-content/uploads/ECMA-262_3rd_edition_december_1999.pdf>) with
//! the XML literals and operators of E4X (ECMA-357 2nd edition,
//! <https://ecma-international.org/wp-content/uploads/ECMA-357_2nd_edition_december_2005.pdf>), and
//! it reads later syntax (template literals, `=>`) only to report it. The directive and platform
//! names come from the public pages `catalog.rs` cites by URL; only the names are used. Nothing
//! maps the reference app's names to VectorCraft commands.
//!
//! The checker never recurses: it reads each file once, and each scan of a file goes through its
//! tokens front to back, first for the names its functions and its global code declare and then
//! for the analysis. It scans the script for the includes and what the variables hold, and each
//! file it includes when it follows them, again while that grows (at most three more times), and
//! then for the report; a scan that finds nothing more is the report scan of a script read without
//! its included files. It
//! keeps open brackets and the levels of an XML literal on capped stacks. Its work is linear in the
//! input's size, and no input can overflow the stack.
#![forbid(unsafe_code)]

mod analyze;
mod catalog;
mod directives;
mod includes;
mod lex;
mod report;

pub use includes::{Loaded, Loader};
pub use report::{Directive, Feature, Include, IncludeStatus, MenuString, Named, Outside, Problem, Report, Target};

/// The limits that keep a check small and quick; [`Limits::default`] holds the ones VectorCraft
/// uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most bytes of one file read: a larger script is [`Kind::TooLarge`], and a larger
    /// included file isn't read.
    pub file_bytes: usize,
    /// The most bytes read from the script and its included files together.
    pub total_bytes: usize,
    /// The most included files read.
    pub includes: usize,
    /// How deep includes are followed: a file the script includes is at depth 1.
    pub include_depth: usize,
    /// The most paths looked up for included files, whether a file is found there or not.
    pub lookups: usize,
    /// The most open brackets tracked, and the most levels an XML literal is read to (its markup,
    /// the code in a `{…}` inside it, an XML literal in that code). Deeper nesting is a problem, and
    /// the check goes on.
    pub nesting: usize,
    /// The most entries in each list of the report, and in each table of names the check keeps (the
    /// variables holding a member chain, the variables holding a File or Folder object, the names
    /// one function declares, and the functions the global code declares).
    pub entries: usize,
    /// The most line numbers listed for one entry.
    pub lines: usize,
    /// The most characters of text echoed from a script (names, values, member chains).
    pub echo: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 8 << 20,
            total_bytes: 32 << 20,
            includes: 64,
            include_depth: 16,
            lookups: 1024,
            nesting: 4096,
            entries: 1000,
            lines: 20,
            echo: 120,
        }
    }
}

/// What kind of file a script is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Text that reads as a script.
    Script,
    /// A compiled script (`.jsxbin`, or text that starts with `@JSXBIN@`). It isn't read.
    Compiled,
    /// A JavaScript module (top-level `import` or `export`), such as a React `.jsx` file: not a
    /// script for the reference app.
    Module,
    /// Not text: a NUL byte, or more than one byte in ten a control character, in the first 8 KiB.
    NotText,
    /// Larger than [`Limits::file_bytes`].
    TooLarge,
}

impl Kind {
    pub fn id(self) -> &'static str {
        match self {
            Kind::Script => "script",
            Kind::Compiled => "compiled",
            Kind::Module => "module",
            Kind::NotText => "notText",
            Kind::TooLarge => "tooLarge",
        }
    }
}

/// A kind of help a script needs from the app that runs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Need {
    /// The script language's own objects and directives (`$`, `alert`, `UnitValue`, `#target`).
    Language,
    /// The reference app's document model (`app.activeDocument…`).
    Dom,
    /// Files and folders.
    Files,
    /// Menu commands run by name.
    MenuCommands,
    /// Dialogs and windows of its own.
    Dialogs,
    /// Features only the reference app has (palettes, messages to other apps, network
    /// connections, native code, E4X, actions).
    HostOnly,
}

impl Need {
    pub const ALL: [Need; 6] = [Need::Language, Need::Dom, Need::Files, Need::MenuCommands, Need::Dialogs, Need::HostOnly];

    pub fn id(self) -> &'static str {
        match self {
            Need::Language => "language",
            Need::Dom => "dom",
            Need::Files => "files",
            Need::MenuCommands => "menuCommands",
            Need::Dialogs => "dialogs",
            Need::HostOnly => "hostOnly",
        }
    }
}

/// Every id in [`Report::outside`], in the order the report lists them.
pub use report::OUTSIDE;
/// Every problem code a report can list.
pub use report::PROBLEMS;

/// Check the script `bytes`, named `name` (a file name; only its last part is used). With a
/// `loader`, the files its `#include` lines name are read through it and checked too; without one
/// they are listed as [`IncludeStatus::NotRead`]. When the script or one of those files assigns a
/// member chain or a File object to a variable, the check treats the variable as holding it in
/// every file, also in code before the assignment. A script that includes nothing gets the same
/// report with a loader and without one. Nothing in the script runs.
pub fn check(name: &str, bytes: &[u8], loader: Option<&mut dyn Loader>, limits: &Limits) -> Report {
    let mut acc = report::Acc::new(limits);
    if bytes.len() > limits.file_bytes {
        return acc.finish(Kind::TooLarge, 0);
    }
    let name = base_name(name);
    if is_compiled(name, bytes) {
        return acc.finish(Kind::Compiled, 0);
    }
    if !is_text(bytes) {
        return acc.finish(Kind::NotText, 0);
    }
    let (text, invalid) = decode(bytes);
    if invalid {
        acc.problem(report::NOT_UTF8, None, None, None);
    }
    let text = analyze::unescaped(text, &mut acc);
    let declared = analyze::declarations(&text, &mut acc);
    let none = analyze::Known::default();
    // With a loader, this first scan finds the includes and what the script's variables hold; its
    // tallies are left out.
    let mut tallies = report::Acc::new(limits);
    let pre = acc.clone();
    let scan = analyze::scan(&text, &declared, None, std::rc::Rc::from([]), &none, if loader.is_some() { &mut tallies } else { &mut acc });
    acc.steps = acc.steps.saturating_add(tallies.steps);
    if scan.module {
        // A module's report holds its kind and its lines, and none of what the scan found.
        let steps = acc.steps;
        let mut acc = report::Acc::new(limits);
        acc.steps = steps;
        return acc.finish(Kind::Module, scan.lines);
    }
    let Some(loader) = loader else {
        let mut acc = settle(acc, pre, &text, &declared, scan.known);
        includes::not_read(&mut acc, scan.includes);
        return acc.finish(Kind::Script, scan.lines);
    };
    let mut known = scan.known;
    let read = includes::follow(&mut acc, scan.includes, loader, name, bytes.len(), &mut known);
    // Each file's text and the names it declares are worked out once for all the scans below.
    let decoded: Vec<_> = read.iter().map(|file| decode(&file.bytes)).collect();
    let texts: Vec<_> = decoded.into_iter().map(|(text, invalid)| (analyze::unescaped(text, &mut acc), invalid)).collect();
    let declarations: Vec<_> = texts.iter().map(|(text, _)| analyze::declarations(text, &mut acc)).collect();
    // A variable may get what it holds from another file's variable: the files are scanned again,
    // each with what is known so far, while that grows, at most three more times.
    for _ in 0..3 {
        let before = known.len();
        let mut tallies = report::Acc::new(limits);
        let more = analyze::scan(&text, &declared, None, std::rc::Rc::from([]), &known, &mut tallies).known;
        known.add(more, limits.entries);
        for ((file, (text, _)), declared) in read.iter().zip(&texts).zip(&declarations) {
            let more = analyze::scan(text, declared, Some(&file.path), std::rc::Rc::clone(&file.paths), &known, &mut tallies).known;
            known.add(more, limits.entries);
        }
        acc.steps = acc.steps.saturating_add(tallies.steps);
        if known.len() == before {
            break;
        }
    }
    // The scans for the report: every file starts with what any file's variables hold.
    let lines = analyze::scan(&text, &declared, None, std::rc::Rc::from([]), &known, &mut acc).lines;
    for ((file, (text, invalid)), declared) in read.iter().zip(&texts).zip(&declarations) {
        if *invalid {
            acc.problem(report::NOT_UTF8, None, Some(&file.path), None);
        }
        analyze::scan(text, declared, Some(&file.path), std::rc::Rc::clone(&file.paths), &known, &mut acc);
    }
    acc.finish(Kind::Script, lines)
}

/// The report of a script read without its included files: what the scans above find for a script
/// that includes nothing, so a variable a function uses before the global code assigns it holds
/// what it is assigned. `first` holds the scan made with nothing known, which found `known`, and
/// `pre` what came before it. A scan made with what the scans before it found, and that finds
/// nothing more, is the report scan; no other scan is repeated.
fn settle<'l>(
    first: report::Acc<'l>,
    pre: report::Acc<'l>,
    text: &str,
    declared: &analyze::Declared<'_>,
    mut known: analyze::Known,
) -> report::Acc<'l> {
    if known.len() == 0 {
        return first;
    }
    let entries = pre.limits.entries;
    let mut spent = first.steps.saturating_sub(pre.steps);
    for _ in 0..3 {
        let before = known.len();
        let mut acc = pre.clone();
        let more = analyze::scan(text, declared, None, std::rc::Rc::from([]), &known, &mut acc).known;
        known.add(more, entries);
        if known.len() == before {
            acc.steps = acc.steps.saturating_add(spent);
            return acc;
        }
        spent = spent.saturating_add(acc.steps.saturating_sub(pre.steps));
    }
    let mut acc = pre;
    analyze::scan(text, declared, None, std::rc::Rc::from([]), &known, &mut acc);
    acc.steps = acc.steps.saturating_add(spent);
    acc
}

/// The last part of a path (`lib/Tidy.jsx` → `Tidy.jsx`), whichever separator it uses.
pub(crate) fn base_name(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// A compiled script: named `.jsxbin`, or text that starts with `@JSXBIN@` (after a byte order mark
/// and blanks).
pub(crate) fn is_compiled(name: &str, bytes: &[u8]) -> bool {
    if base_name(name).rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("jsxbin")) {
        return true;
    }
    let b = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let start = b.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(b.len());
    b.get(start..).is_some_and(|b| b.starts_with(b"@JSXBIN@"))
}

/// Text, judged by its first 8 KiB: no NUL byte, and at most one byte in ten a control character
/// other than tab, line feed, vertical tab, form feed, carriage return and escape.
pub(crate) fn is_text(bytes: &[u8]) -> bool {
    let head = bytes.get(..bytes.len().min(8 << 10)).unwrap_or(bytes);
    if head.contains(&0) {
        return false;
    }
    let controls = head.iter().filter(|&&b| b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0b | 0x0c | 0x1b)).count();
    controls * 10 <= head.len()
}

/// The text of `bytes`: UTF-8 after an optional byte order mark, with invalid bytes replaced by
/// U+FFFD (`true` when there were any).
pub(crate) fn decode(bytes: &[u8]) -> (std::borrow::Cow<'_, str>, bool) {
    let b = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = String::from_utf8_lossy(b);
    let invalid = matches!(text, std::borrow::Cow::Owned(_));
    (text, invalid)
}

#[cfg(test)]
mod tests;
