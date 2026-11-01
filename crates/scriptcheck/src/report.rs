//! What a check finds ([`Report`]), and the tallies it is built from ([`Acc`]), each list capped.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::FeatureId;
use crate::{Kind, Limits, Need};

pub(crate) const UNTERMINATED_STRING: &str = "unterminatedString";
pub(crate) const UNTERMINATED_COMMENT: &str = "unterminatedComment";
pub(crate) const UNTERMINATED_REGEXP: &str = "unterminatedRegExp";
pub(crate) const UNTERMINATED_TEMPLATE: &str = "unterminatedTemplate";
pub(crate) const UNTERMINATED_XML: &str = "unterminatedXml";
pub(crate) const UNCLOSED_BRACKET: &str = "unclosedBracket";
pub(crate) const UNEXPECTED_BRACKET: &str = "unexpectedBracket";
pub(crate) const NEWER_SYNTAX: &str = "newerSyntax";
pub(crate) const UNKNOWN_DIRECTIVE: &str = "unknownDirective";
pub(crate) const NOT_UTF8: &str = "notUtf8";
pub(crate) const MIXED_LINE_ENDINGS: &str = "mixedLineEndings";
pub(crate) const TOO_DEEP: &str = "tooDeep";

/// Every problem code a report can list.
pub const PROBLEMS: &[&str] = &[
    UNTERMINATED_STRING,
    UNTERMINATED_COMMENT,
    UNTERMINATED_REGEXP,
    UNTERMINATED_TEMPLATE,
    UNTERMINATED_XML,
    UNCLOSED_BRACKET,
    UNEXPECTED_BRACKET,
    NEWER_SYNTAX,
    UNKNOWN_DIRECTIVE,
    NOT_UTF8,
    MIXED_LINE_ENDINGS,
    TOO_DEEP,
];

/// Declares [`OutsideId`] and [`OUTSIDE`] from one table, in the order the report lists them.
macro_rules! outside {
    ($($variant:ident $id:literal,)*) => {
        /// Something the script does outside the document ([`Outside::id`] is its [`OutsideId::id`]).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        pub(crate) enum OutsideId {
            $($variant),*
        }

        impl OutsideId {
            pub(crate) fn id(self) -> &'static str {
                match self {
                    $(OutsideId::$variant => $id),*
                }
            }
        }

        /// Every id in [`Report::outside`], in the order the report lists them: files written, moved or
        /// deleted; files opened in other apps; network connections; native code; messages to other apps;
        /// environment variables; code the checker can't read; the reference app's preferences changed.
        pub const OUTSIDE: &[&str] = &[$($id),*];
    };
}

outside! {
    FilesWritten "filesWritten",
    FilesOpened "filesOpened",
    Network "network",
    NativeCode "nativeCode",
    OtherApps "otherApps",
    Environment "environment",
    UnseenCode "unseenCode",
    Preferences "preferences",
}

/// What [`crate::check`] found in a script. Lists of located things (targets, directives,
/// includes, problems) give a line and, for an included file, its path (`file`); tallies (features,
/// menu strings, outside effects) count the script and its included files together and list the
/// script's own lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub kind: Kind,
    /// What the script needs from the app that runs it, in [`Need`]'s order.
    pub needs: Vec<Need>,
    /// The script's lines.
    pub lines: usize,
    /// Each `#target` line's application specifier.
    pub targets: Vec<Target>,
    pub directives: Vec<Directive>,
    pub includes: Vec<Include>,
    /// The platform features used, by id.
    pub features: Vec<Feature>,
    /// The document-model member chains used, as written (`app.activeDocument.layers[].name`, with
    /// `[]` for an index and `()` for a call), a variable assigned from a chain replaced by that
    /// chain, and how often each occurs; the most used first.
    pub members: Vec<Named>,
    /// The type names compared with `.typename` or created with `new`, apart from the language's,
    /// the platform's and the script's own functions.
    pub types: Vec<Named>,
    /// The menu-command strings passed as string literals; the most used first.
    pub menu_strings: Vec<MenuString>,
    /// Menu-command calls whose argument isn't a string literal.
    pub menu_dynamic: usize,
    /// What the script does outside the document, in [`OUTSIDE`]'s order.
    pub outside: Vec<Outside>,
    pub problems: Vec<Problem>,
    /// A list of the report, or a table of names the check keeps, reached [`Limits::entries`], and
    /// entries were left out.
    pub truncated: bool,
    /// The steps the check took: the tests check that they grow linearly with the input.
    pub(crate) steps: usize,
}

/// A `#target` line's application specifier, split into its parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub value: String,
    pub app: String,
    pub instance: Option<String>,
    pub version: Option<String>,
    pub locale: Option<String>,
    pub line: usize,
    pub file: Option<String>,
}

/// A preprocessor line of a documented directive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directive {
    /// The directive (`target`, `include`…).
    pub name: &'static str,
    pub value: String,
    pub line: usize,
    /// `#` or `//@`.
    pub form: &'static str,
    pub file: Option<String>,
}

/// What became of an `#include` line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IncludeStatus {
    /// Read and checked (or checked already, for a file included twice).
    Checked,
    /// No such file in the script's folder, or in the folders `#includepath` names inside it.
    Missing,
    /// Not followed: an absolute name, a name longer than 4,096 bytes, a name that leads out of the
    /// script's folder, or a file the caller may not read.
    NotFollowed,
    /// The file it names includes the file the line is in, directly or through other files.
    Cycle,
    /// Includes weren't read (no loader).
    NotRead,
    /// Not read: a limit was reached (files, depth, size, paths looked up).
    Limit,
    /// A compiled file, or not text.
    Unreadable,
}

impl IncludeStatus {
    pub const ALL: [IncludeStatus; 7] = [
        IncludeStatus::Checked,
        IncludeStatus::Missing,
        IncludeStatus::NotFollowed,
        IncludeStatus::Cycle,
        IncludeStatus::NotRead,
        IncludeStatus::Limit,
        IncludeStatus::Unreadable,
    ];

    pub fn id(self) -> &'static str {
        match self {
            IncludeStatus::Checked => "checked",
            IncludeStatus::Missing => "missing",
            IncludeStatus::NotFollowed => "notFollowed",
            IncludeStatus::Cycle => "cycle",
            IncludeStatus::NotRead => "notRead",
            IncludeStatus::Limit => "limit",
            IncludeStatus::Unreadable => "unreadable",
        }
    }
}

/// An `#include` line and what became of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Include {
    /// The name as the line writes it.
    pub value: String,
    pub line: usize,
    /// The included file the line is in.
    pub file: Option<String>,
    pub status: IncludeStatus,
    /// The file it names, as a path inside the script's folder, when one was found.
    pub path: Option<String>,
}

/// A platform feature: how often it occurs, and the script's first lines with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feature {
    pub id: &'static str,
    pub count: usize,
    pub lines: Vec<usize>,
}

/// A name and how often it occurs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    pub name: String,
    pub count: usize,
}

/// A menu-command string: how often it is passed, and the script's first lines with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuString {
    pub value: String,
    pub count: usize,
    pub lines: Vec<usize>,
}

/// Something the script does outside the document ([`OUTSIDE`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outside {
    pub id: &'static str,
    pub count: usize,
    pub lines: Vec<usize>,
    /// For `filesWritten`: the open modes used (`w`, `a`, `e`).
    pub modes: Vec<&'static str>,
}

/// A problem that would stop the script loading or running as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub code: &'static str,
    pub line: Option<usize>,
    pub file: Option<String>,
    /// The text concerned, as written (an unknown directive, a construct of later syntax, a
    /// bracket).
    pub detail: Option<String>,
}

/// A count and the first lines it was seen on.
#[derive(Clone, Debug, Default)]
struct Tally {
    count: usize,
    lines: Vec<usize>,
}

impl Tally {
    fn add(&mut self, line: Option<usize>, max: usize) {
        self.count = self.count.saturating_add(1);
        if let Some(line) = line
            && self.lines.len() < max
            && self.lines.last() != Some(&line)
        {
            self.lines.push(line);
        }
    }
}

/// What a check has found so far, across the script and its included files.
#[derive(Clone)]
pub(crate) struct Acc<'l> {
    pub limits: &'l Limits,
    pub steps: usize,
    features: BTreeMap<FeatureId, Tally>,
    outside: BTreeMap<OutsideId, (Tally, Vec<&'static str>)>,
    menu: BTreeMap<String, Tally>,
    menu_dynamic: usize,
    members: BTreeMap<String, usize>,
    types: BTreeMap<String, usize>,
    /// Functions the script declares: `new` of one is no document-model type.
    declared: BTreeSet<String>,
    targets: Vec<Target>,
    directives: Vec<Directive>,
    includes: Vec<Include>,
    problems: Vec<Problem>,
    truncated: bool,
}

impl<'l> Acc<'l> {
    pub fn new(limits: &'l Limits) -> Self {
        Self {
            limits,
            steps: 0,
            features: BTreeMap::new(),
            outside: BTreeMap::new(),
            menu: BTreeMap::new(),
            menu_dynamic: 0,
            members: BTreeMap::new(),
            types: BTreeMap::new(),
            declared: BTreeSet::new(),
            targets: vec![],
            directives: vec![],
            includes: vec![],
            problems: vec![],
            truncated: false,
        }
    }

    /// `s` as the report echoes it: at most [`Limits::echo`] characters, the last one `…` when
    /// it was cut.
    pub fn echo(&self, s: &str) -> String {
        echo(s, self.limits.echo)
    }

    /// [`Acc::echo`] without a copy when `s` isn't cut.
    fn echoed<'a>(&self, s: &'a str) -> Cow<'a, str> {
        echo_cow(s, self.limits.echo)
    }

    /// Room for one more entry in a list of the report, or in a table of names the check keeps, of
    /// `len` entries: false (and the report marked truncated) when it is full.
    pub fn room(&mut self, len: usize) -> bool {
        let room = len < self.limits.entries;
        self.truncated |= !room;
        room
    }

    pub fn feature(&mut self, id: FeatureId, line: Option<usize>) {
        let max = self.limits.lines;
        self.features.entry(id).or_default().add(line, max);
    }

    pub fn outside(&mut self, id: OutsideId, line: Option<usize>, mode: Option<&'static str>) {
        let max = self.limits.lines;
        let (tally, modes) = self.outside.entry(id).or_default();
        tally.add(line, max);
        if let Some(m) = mode
            && !modes.contains(&m)
        {
            modes.push(m);
        }
    }

    pub fn menu_string(&mut self, value: &str, line: Option<usize>) {
        let key = self.echoed(value);
        let max = self.limits.lines;
        if let Some(tally) = self.menu.get_mut(&*key) {
            tally.add(line, max);
        } else if self.room(self.menu.len()) {
            self.menu.entry(key.into_owned()).or_default().add(line, max);
        }
    }

    pub fn menu_dynamic(&mut self) {
        self.menu_dynamic = self.menu_dynamic.saturating_add(1);
    }

    pub fn member(&mut self, path: &str) {
        if let Some(n) = self.members.get_mut(path) {
            *n = n.saturating_add(1);
        } else if self.room(self.members.len()) {
            self.members.insert(path.to_string(), 1);
        }
    }

    pub fn type_name(&mut self, name: &str) {
        let key = self.echoed(name);
        if let Some(n) = self.types.get_mut(&*key) {
            *n = n.saturating_add(1);
        } else if self.room(self.types.len()) {
            self.types.insert(key.into_owned(), 1);
        }
    }

    pub fn declared(&mut self, name: &str) {
        if self.declared.len() < self.limits.entries {
            let name = self.echoed(name);
            if !self.declared.contains(&*name) {
                self.declared.insert(name.into_owned());
            }
        }
    }

    pub fn problem(&mut self, code: &'static str, line: Option<usize>, file: Option<&str>, detail: Option<&str>) {
        if self.room(self.problems.len()) {
            let detail = detail.map(|d| self.echo(d));
            self.problems.push(Problem { code, line, file: file.map(str::to_string), detail });
        }
    }

    pub fn directive(&mut self, name: &'static str, value: &str, line: usize, form: &'static str, file: Option<&str>) {
        if self.room(self.directives.len()) {
            let value = self.echo(value);
            self.directives.push(Directive { name, value, line, form, file: file.map(str::to_string) });
        }
    }

    pub fn target(&mut self, value: &str, line: usize, file: Option<&str>) {
        if !self.room(self.targets.len()) {
            return;
        }
        let t = crate::directives::target(value);
        let part = |s: Option<&str>| s.map(|s| echo(s, self.limits.echo));
        let target = Target {
            value: self.echo(value),
            app: self.echo(t.app),
            instance: part(t.instance),
            version: part(t.version),
            locale: part(t.locale),
            line,
            file: file.map(str::to_string),
        };
        self.targets.push(target);
    }

    pub fn include(&mut self, value: &str, line: usize, file: Option<&str>, status: IncludeStatus, path: Option<&str>) {
        if self.room(self.includes.len()) {
            let include = Include { value: self.echo(value), line, file: file.map(str::to_string), status, path: path.map(|p| self.echo(p)) };
            self.includes.push(include);
        }
    }

    /// The report for a file of `kind` with `lines` lines.
    pub fn finish(self, kind: Kind, lines: usize) -> Report {
        let declared = self.declared;
        let mut members: Vec<Named> = self.members.into_iter().map(|(name, count)| Named { name, count }).collect();
        members.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        let mut types: Vec<Named> =
            self.types.into_iter().filter(|(name, _)| !declared.contains(name)).map(|(name, count)| Named { name, count }).collect();
        types.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        let mut menu_strings: Vec<MenuString> =
            self.menu.into_iter().map(|(value, t)| MenuString { value, count: t.count, lines: t.lines }).collect();
        menu_strings.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
        let mut needs: BTreeSet<Need> = self.features.keys().map(|id| id.need()).collect();
        let features: Vec<Feature> = self.features.into_iter().map(|(id, t)| Feature { id: id.id(), count: t.count, lines: t.lines }).collect();
        let outside: Vec<Outside> =
            self.outside.into_iter().map(|(id, (t, modes))| Outside { id: id.id(), count: t.count, lines: t.lines, modes }).collect();
        if !members.is_empty() || !types.is_empty() {
            needs.insert(Need::Dom);
        }
        Report {
            kind,
            needs: needs.into_iter().collect(),
            lines,
            targets: self.targets,
            directives: self.directives,
            includes: self.includes,
            features,
            members,
            types,
            menu_strings,
            menu_dynamic: self.menu_dynamic,
            outside,
            problems: self.problems,
            truncated: self.truncated,
            steps: self.steps,
        }
    }
}

/// `s` cut to at most `max` characters, the last one `…` when it was cut.
pub(crate) fn echo(s: &str, max: usize) -> String {
    echo_cow(s, max).into_owned()
}

/// [`echo`], borrowing `s` when it isn't cut.
fn echo_cow(s: &str, max: usize) -> Cow<'_, str> {
    match s.char_indices().nth(max) {
        None => Cow::Borrowed(s),
        Some(_) => {
            let keep = s.char_indices().nth(max.saturating_sub(1)).map_or(0, |(i, _)| i);
            let mut out = s.get(..keep).unwrap_or_default().to_string();
            out.push('…');
            Cow::Owned(out)
        }
    }
}
