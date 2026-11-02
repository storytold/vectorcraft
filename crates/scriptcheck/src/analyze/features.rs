//! The platform features, menu commands and later syntax the analysis recognises.

use super::{Analyzer, FeatureId, NEWER, OutsideId, Pending, Sig, T, catalog, is, member_mark, newer_number, report, unquote, word};

/// A global name of the platform the report looks for (`$`, `File`, `Socket`, `alert`, `eval`…).
pub(super) fn platform_name(n: &str) -> bool {
    matches!(
        n,
        catalog::DOLLAR
            | catalog::LOCALIZE
            | catalog::UNIT_VALUE
            | catalog::FILE
            | catalog::FOLDER
            | catalog::WINDOW
            | catalog::BRIDGE_TALK
            | catalog::SOCKET
            | catalog::EXTERNAL_OBJECT
            | "eval"
            | "Function"
    ) || catalog::NOTIFY.contains(&n)
        || catalog::XMP.contains(&n)
        || catalog::XML.contains(&n)
}

/// The feature of a window of type `value` (`"dialog"`, or a resource string that starts with it).
pub(super) fn window_type(value: &str) -> FeatureId {
    let word = value.trim_start().split(|c: char| c.is_whitespace() || c == '{').next().unwrap_or_default();
    if word.eq_ignore_ascii_case("dialog") {
        FeatureId::ScriptuiDialog
    } else if word.eq_ignore_ascii_case("palette") {
        FeatureId::ScriptuiPalette
    } else {
        FeatureId::ScriptuiWindow
    }
}

impl<'s, 'a, 'l> Analyzer<'s, 'a, 'l> {
    pub(super) fn features(&mut self, t: &Sig<'s>) {
        let line = self.agg(t.line);
        let mut pending = std::mem::take(&mut self.pending);
        // A platform name that no `:` follows stands for the platform's object.
        if let Pending::Name(n, l) = pending {
            pending = Pending::None;
            if !t.is(":") {
                self.platform(n, l);
                pending = std::mem::take(&mut self.pending);
            }
        }
        match pending {
            Pending::None | Pending::Name(..) => {}
            Pending::Window(l) => {
                if t.is("(") {
                    self.pending = Pending::WindowType(l);
                } else {
                    self.acc.feature(FeatureId::ScriptuiWindow, l);
                }
            }
            Pending::WindowType(l) => {
                let feature = if t.kind == T::Str { window_type(&unquote(t.text, 64)) } else { FeatureId::ScriptuiWindow };
                self.acc.feature(feature, l);
            }
            Pending::Menu(l) => {
                if t.kind == T::Str {
                    self.pending = Pending::MenuString(l, t.text);
                } else {
                    self.menu(None, l);
                }
            }
            Pending::MenuString(l, s) => self.menu(t.is(")").then_some(s), l),
            Pending::Open(l) => {
                if t.kind == T::Str {
                    self.open_mode(&unquote(t.text, 2), l);
                }
            }
        }
        match t.kind {
            T::Ident => self.name(t, line),
            T::Punct => self.punct(t, line),
            T::Str => self.string(t, line),
            T::Xml => self.acc.feature(FeatureId::E4x, line),
            _ => {}
        }
    }

    /// A menu command run with the string literal `literal`, or with another value.
    fn menu(&mut self, literal: Option<&str>, line: Option<usize>) {
        match literal {
            Some(s) => {
                let value = unquote(s, self.acc.limits.echo.saturating_add(1));
                self.acc.menu_string(&value, line);
            }
            None => self.acc.menu_dynamic(),
        }
        self.acc.feature(FeatureId::MenuCommand, line);
    }

    fn open_mode(&mut self, mode: &str, line: Option<usize>) {
        if mode.eq_ignore_ascii_case("r") {
            self.acc.feature(FeatureId::FileRead, line);
        } else if let Some(m) = catalog::WRITE_MODES.iter().copied().find(|m| m.eq_ignore_ascii_case(mode)) {
            self.acc.feature(FeatureId::FileWrite, line);
            self.acc.outside(OutsideId::FilesWritten, line, Some(m));
        }
    }

    /// What a pending feature waits for that never came.
    pub(super) fn settle_pending(&mut self) {
        let mut pending = std::mem::take(&mut self.pending);
        if let Pending::Name(n, l) = pending {
            self.platform(n, l);
            pending = std::mem::take(&mut self.pending);
        }
        match pending {
            Pending::Window(l) | Pending::WindowType(l) => self.acc.feature(FeatureId::ScriptuiWindow, l),
            Pending::Menu(l) | Pending::MenuString(l, _) => self.menu(None, l),
            Pending::Open(_) | Pending::Name(..) | Pending::None => {}
        }
    }

    /// The platform name `n`, used at `line` (a tally's line): the feature it is, and what it does
    /// outside the document.
    fn platform(&mut self, n: &'s str, line: Option<usize>) {
        match n {
            catalog::UNIT_VALUE => self.acc.feature(FeatureId::UnitValue, line),
            catalog::FILE | catalog::FOLDER => self.acc.feature(FeatureId::FileObjects, line),
            catalog::BRIDGE_TALK => {
                self.acc.feature(FeatureId::BridgeTalk, line);
                self.acc.outside(OutsideId::OtherApps, line, None);
            }
            catalog::SOCKET => {
                self.acc.feature(FeatureId::Socket, line);
                self.acc.outside(OutsideId::Network, line, None);
            }
            catalog::EXTERNAL_OBJECT => {
                self.acc.feature(FeatureId::ExternalObject, line);
                self.acc.outside(OutsideId::NativeCode, line, None);
            }
            catalog::WINDOW => self.pending = Pending::Window(line),
            _ if catalog::XMP.contains(&n) => self.acc.feature(FeatureId::Xmp, line),
            _ if catalog::XML.contains(&n) => self.acc.feature(FeatureId::Xml, line),
            _ => {}
        }
    }

    fn name(&mut self, t: &Sig<'s>, line: Option<usize>) {
        let [h0, h1, h2, _] = self.h;
        let n = t.text;
        if is(h0, ".") {
            // A member: of `$`, `File` or `Folder` when they come first and are the platform's.
            let own = &self.own_names;
            let of = |name: &str| word(h1, name) && !member_mark(h2) && !own.contains(name);
            if of(catalog::DOLLAR) {
                if catalog::DOLLAR_ENV.contains(&n) {
                    self.acc.feature(FeatureId::DollarEnv, line);
                    self.acc.outside(OutsideId::Environment, line, None);
                } else if n == catalog::DOLLAR_EVAL_FILE {
                    self.acc.feature(FeatureId::DollarEvalFile, line);
                    self.acc.outside(OutsideId::UnseenCode, line, None);
                } else if n == catalog::LOCALIZE {
                    self.acc.feature(FeatureId::Localize, line);
                } else {
                    self.acc.feature(FeatureId::Dollar, line);
                }
            } else if (of(catalog::FILE) && catalog::FILE_DIALOGS.contains(&n)) || (of(catalog::FOLDER) && catalog::FOLDER_DIALOGS.contains(&n)) {
                self.acc.feature(FeatureId::FileDialog, line);
            } else if of(catalog::FOLDER) && catalog::FOLDER_PLACES.contains(&n) {
                self.acc.feature(FeatureId::FilePlaces, line);
            }
            if n == catalog::ON_DRAW {
                self.acc.feature(FeatureId::ScriptuiDrawing, line);
            }
            if n == catalog::REFLECT {
                self.acc.feature(FeatureId::Reflection, line);
            }
            return;
        }
        if member_mark(h0) {
            return;
        }
        if word(h0, "function") {
            self.acc.declared(n);
            if h0.is_some_and(|f| f.stmt) {
                // A function declaration's name is a variable where the function is declared. A
                // declaration directly in a function body or in the global code makes it one from the
                // body's `{` or from the start of the file ([`Declared`]), and any other declaration
                // makes it one from here.
                if !self.hoisted(n) {
                    self.keep_outer(n);
                    self.unbind(n);
                }
            } else {
                // A function expression's name is a variable inside the function only.
                self.expression_name = Some(n.to_string());
            }
            return;
        }
        if n == "catch" {
            // Its block's `{` comes after its parameter's parentheses.
            self.catch = Some((self.depth(), None));
        }
        if n == "function" {
            // Its body's `{` comes after its name and parameters.
            self.function = Some(self.depth());
            self.expression_name = None;
            // `name = function`: the script's own function too.
            if is(h0, "=")
                && let Some(f) = h1.filter(|s| s.kind == T::Ident)
            {
                self.acc.declared(f.text);
            }
        }
        if word(h0, "new") && n.starts_with(|c: char| c.is_ascii_uppercase()) && !catalog::CONSTRUCTORS.contains(&n) {
            self.acc.type_name(n);
        }
        // A platform name stands for the platform's object, except where the script declares a
        // variable, function or parameter of that name, and where it has a meaning of its own.
        // Where a property key or a label may stand, the token after it decides.
        if platform_name(n) && !self.declaring() && !self.own_names.contains(n) {
            if h0.is_none() || [",", "{", ";", "}"].iter().any(|p| is(h0, p)) {
                self.pending = Pending::Name(n, line);
            } else {
                self.platform(n, line);
            }
        }
        match n {
            // E4X: `default xml namespace = …` and `for each (…)`.
            "namespace" if word(h0, "xml") && word(h1, "default") => self.acc.feature(FeatureId::E4x, line),
            "each" if word(h0, "for") => self.acc.feature(FeatureId::E4x, line),
            _ => {}
        }
    }

    fn punct(&mut self, t: &Sig<'s>, line: Option<usize>) {
        let [h0, h1, h2, h3] = self.h;
        match t.text {
            "(" => self.call(line),
            // `Name.prototype["+"] = …` defines an operator.
            "=" => {
                let operator = || h1.filter(|s| s.kind == T::Str).is_some_and(|s| catalog::OPERATORS.contains(&unquote(s.text, 4).as_str()));
                if is(h0, "]") && is(h2, "[") && word(h3, "prototype") && operator() {
                    self.acc.feature(FeatureId::OperatorOverloading, line);
                }
            }
            // E4X: attributes (`x.@id`, `@*`), descendants (`x..item`) and qualified names (`ns::x`).
            ".@" | ".." | "::" | "@" => self.acc.feature(FeatureId::E4x, line),
            _ => {}
        }
    }

    /// `(` after a name: a call.
    fn call(&mut self, line: Option<usize>) {
        let [h0, h1, h2, h3] = self.h;
        let Some(callee) = h0.filter(|s| s.kind == T::Ident) else { return };
        let n = callee.text;
        if is(h1, ".") {
            // The receiver is a variable holding a File or Folder object (not a property of the same
            // name), or a File or Folder object made in place or held in grouping parentheses
            // ([`Sig::file`]).
            let file = h2.is_some_and(|r| r.file || (r.kind == T::Ident && !member_mark(h3) && self.files.contains(r.text)));
            if n == catalog::MENU_COMMAND {
                self.pending = Pending::Menu(line);
            } else if n == catalog::OPEN && file {
                self.pending = Pending::Open(line);
            } else if catalog::DRAWING.contains(&n) {
                self.acc.feature(FeatureId::ScriptuiDrawing, line);
            } else if catalog::INSTANCE_DIALOGS.contains(&n) {
                self.acc.feature(FeatureId::FileDialog, line);
            } else if catalog::FOLDER_READS.contains(&n) || (file && catalog::FILE_READS.contains(&n)) {
                self.acc.feature(FeatureId::FileRead, line);
            } else if file && catalog::FILE_WRITES.contains(&n) {
                self.acc.feature(FeatureId::FileWrite, line);
                self.acc.outside(OutsideId::FilesWritten, line, None);
            } else if n == catalog::EXECUTE {
                self.acc.feature(FeatureId::FileExecute, line);
                self.acc.outside(OutsideId::FilesOpened, line, None);
            } else if catalog::ACTIONS.contains(&n) {
                self.acc.feature(FeatureId::Actions, line);
            } else if n == catalog::SELECT_TOOL {
                self.acc.feature(FeatureId::SelectTool, line);
            }
        } else if !member_mark(h1) && !word(h1, "function") && !self.own_names.contains(n) {
            if catalog::NOTIFY.contains(&n) {
                self.acc.feature(FeatureId::Notify, line);
            } else if n == catalog::LOCALIZE {
                self.acc.feature(FeatureId::Localize, line);
            } else if matches!(n, "eval" | "Function") {
                self.acc.feature(FeatureId::DynamicCode, line);
                self.acc.outside(OutsideId::UnseenCode, line, None);
            }
        }
    }

    fn string(&mut self, t: &Sig<'s>, line: Option<usize>) {
        let [h0, h1, h2, _] = self.h;
        if t.text.contains(catalog::ACTION_TEXT) {
            self.acc.feature(FeatureId::Actions, line);
        }
        let compares = h0.is_some_and(|s| s.kind == T::Punct && matches!(s.text, "==" | "===" | "!=" | "!=="));
        if compares && word(h1, "typename") && is(h2, ".") {
            let name = unquote(t.text, self.acc.limits.echo.saturating_add(1));
            self.acc.type_name(&name);
        }
    }

    /// Syntax later than ECMAScript 3: reported once per construct and line.
    pub(super) fn newer(&mut self, t: &Sig<'s>) {
        let [h0, h1, ..] = self.h;
        let construct: Option<&'s str> = match t.kind {
            T::Template => Some("`"),
            T::Punct if NEWER.contains(&t.text) => Some(t.text),
            T::Number if newer_number(t.text) => Some(t.text),
            T::Other if t.text.len() > 1 && t.text.starts_with('#') => Some(t.text),
            T::Ident if word(h0, "class") && !member_mark(h1) => Some("class"),
            // `let name` declares a variable. ES3 reads `let` as a variable's name in `let = 1`,
            // `let.x` and `let in o`.
            T::Ident if word(h0, "let") && !t.nl && !member_mark(h1) && !matches!(t.text, "in" | "instanceof") => Some("let"),
            T::Ident if t.text == "function" && word(h0, "async") && !t.nl => Some("async"),
            _ => None,
        };
        if let Some(c) = construct
            && self.last_newer != Some((t.line, c))
        {
            self.last_newer = Some((t.line, c));
            self.acc.problem(report::NEWER_SYNTAX, Some(t.line), self.file, Some(c));
        }
    }

    /// `import` or `export` starting a statement outside any bracket, unless a call, member or
    /// property (`import(…)`, `export.x`, `import: 1`) follows: a module.
    pub(super) fn module_check(&mut self, t: &Sig<'s>) {
        if std::mem::take(&mut self.module_candidate) && !(t.kind == T::Punct && matches!(t.text, "(" | "." | "=" | ":" | "," | ")")) {
            self.module = true;
        }
        let starts = self.h[0].is_none() || t.nl || is(self.h[0], ";") || is(self.h[0], "}");
        if t.kind == T::Ident && matches!(t.text, "import" | "export") && self.depth() == 0 && !member_mark(self.h[0]) && starts {
            self.module_candidate = true;
        }
    }
}
