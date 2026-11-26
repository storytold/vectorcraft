//! The platform features, menu commands and later syntax the analysis recognises.

use super::{Analyzer, COMPOUND, FeatureId, NEWER, OutsideId, Pending, Sig, T, catalog, is, member_mark, newer_number, report, unquote, word};

/// Words that start or continue a statement or an expression, never a variable's name.
const RESERVED: [&str; 24] = [
    "var",
    "function",
    "if",
    "else",
    "for",
    "while",
    "do",
    "return",
    "new",
    "typeof",
    "delete",
    "in",
    "instanceof",
    "this",
    "with",
    "switch",
    "case",
    "try",
    "catch",
    "finally",
    "throw",
    "break",
    "continue",
    "void",
];

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
            Pending::MenuString(l, s) if t.is(",") => self.pending = Pending::MenuStringComma(l, s),
            Pending::MenuString(l, s) | Pending::MenuStringComma(l, s) => self.menu(t.is(")").then_some(s), l),
            Pending::Open(l) => {
                if t.kind == T::Str {
                    self.open_mode(&unquote(t.text, 2), l);
                }
            }
            Pending::Setter(l, prefix) => {
                let sets = if prefix {
                    !t.is(".") && !t.is("[") && !t.is("(")
                } else {
                    // An update operator after a line break belongs to the next line's operand.
                    t.kind == T::Punct && (t.text == "=" || COMPOUND.contains(&t.text) || (matches!(t.text, "++" | "--") && !t.nl))
                };
                if sets {
                    self.file_setter(l);
                }
            }
        }
        self.writer_use(t);
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
            Pending::Menu(l) | Pending::MenuString(l, _) | Pending::MenuStringComma(l, _) => self.menu(None, l),
            Pending::Setter(l, true) => self.file_setter(l),
            Pending::Open(_) | Pending::Setter(_, false) | Pending::Name(..) | Pending::None => {}
        }
    }

    /// `SaveOptions.SAVECHANGES` read at `line`: a `Document.close` given it saves the document to
    /// its own file, and the value may reach `close` through a variable or an argument.
    fn save_changes(&mut self, line: Option<usize>) {
        self.acc.feature(FeatureId::FileWrite, line);
        self.acc.outside(OutsideId::FilesWritten, line, None);
    }

    /// A File object's `length`, `hidden` or `readonly` set at `line`: the file changes.
    fn file_setter(&mut self, line: Option<usize>) {
        self.acc.feature(FeatureId::FileWrite, line);
        self.acc.outside(OutsideId::FilesWritten, line, None);
    }

    /// `name`, a property of `receiver` read with `.` or as a string literal in brackets, `before`
    /// coming before the receiver: when it is the setter of an object that may be a File object (not
    /// of one that is a Folder object whatever the path names), the next token says whether it is set
    /// ([`Pending::Setter`]). An update operator in `before` is a prefix
    /// (`++f.length`) when the receiver is on its line, or when it starts a line or comes where an
    /// expression may start; otherwise it ends the operand before it (`i++` and a line break, then
    /// `f.length`).
    fn setter(&mut self, name: &str, receiver: Option<Sig<'s>>, before: Option<Sig<'s>>, line: Option<usize>) {
        if catalog::FILE_SETTERS.contains(&name) && self.held_file(receiver, before) == Some(false) {
            // Before a receiver that ends in `)` (`new File(i++).length`), the operator is inside the
            // parentheses.
            let update = before.filter(|b| (b.is("++") || b.is("--")) && receiver.is_some_and(|r| r.kind == T::Ident));
            let prefix = update.is_some_and(|b| b.nl || b.expr || receiver.is_some_and(|r| !r.nl));
            self.pending = Pending::Setter(line, prefix);
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
                } else {
                    self.acc.feature(FeatureId::Dollar, line);
                }
            } else if (of(catalog::FILE) && catalog::FILE_DIALOGS.contains(&n)) || (of(catalog::FOLDER) && catalog::FOLDER_DIALOGS.contains(&n)) {
                self.acc.feature(FeatureId::FileDialog, line);
            } else if of(catalog::FOLDER) && catalog::FOLDER_PLACES.contains(&n) {
                self.acc.feature(FeatureId::FilePlaces, line);
            }
            if n == catalog::SAVE_CHANGES {
                self.save_changes(line);
            }
            if n == catalog::ON_DRAW {
                self.acc.feature(FeatureId::ScriptuiDrawing, line);
            }
            if n == catalog::REFLECT {
                self.acc.feature(FeatureId::Reflection, line);
            }
            self.setter(n, h1, h2, line);
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
            // `f['length']`: a File object's setter may follow.
            "]" if is(h1, "[") && h0.is_some_and(|s| s.kind == T::Str) => {
                let name = h0.map(|s| unquote(s.text, 16)).unwrap_or_default();
                self.setter(&name, h2, h3, line);
            }
            // `local['saveAs']`: what the object before the bracket defines, for a call that may follow.
            "[" => {
                self.index_receiver = self.writer_bits(h0, h1);
                self.index_stable = h0.is_some_and(|r| self.stable(r.text));
            }
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

    /// What the call whose `(` is the current token reaches.
    fn classify_call(&mut self, line: Option<usize>) {
        let [h0, h1, h2, h3] = self.h;
        // `doc['saveAs'](f)`: a document method named by a string literal in brackets.
        if is(h0, "]")
            && is(h2, "[")
            && let Some(method) = h1.filter(|s| s.kind == T::Str).map(|s| unquote(s.text, 32))
            && catalog::DOM_FILE_WRITES.contains(&method.as_str())
            && !(self.index_stable && Self::defines(self.index_receiver, &method))
        {
            self.acc.feature(FeatureId::FileWrite, line);
            self.acc.outside(OutsideId::FilesWritten, line, None);
            return;
        }
        let Some(callee) = h0.filter(|s| s.kind == T::Ident) else { return };
        let n = callee.text;
        if is(h1, ".") {
            let held = self.held_file(h2, h3);
            // A Folder object whatever the path names has no File-only function: the call does not
            // reach the file system.
            if held == Some(true) && catalog::FILE_ONLY.contains(&n) {
                return;
            }
            let file = held.is_some();
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
            } else if (file && catalog::FILE_WRITES.contains(&n))
                || (catalog::DOM_FILE_WRITES.contains(&n) && !(h2.is_some_and(|r| self.stable(r.text)) && Self::defines(self.writer_bits(h2, h3), n)))
            {
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

    /// The methods that `receiver`, which `before` comes before, defines when it is a variable holding
    /// an object literal of the script's own ([`Analyzer::writers`]), not a property of the same name;
    /// 0 for anything else. A receiver the check does not follow may be a document.
    /// Whether a call of `name`'s own method may be left out: the call is in the global code, or in a
    /// function body that declares `name` itself. A body that reads `name` from outside may run
    /// after the variable was given another value, so its calls are not known.
    fn stable(&self, name: &str) -> bool {
        self.scopes.last().is_none_or(|s| s.saved.contains_key(name))
    }

    /// A function body uses `name`, which no literal has given methods and the body does not declare,
    /// in a way that may change or share it ([`Analyzer::shared_early`]). With no room left for the
    /// name, no literal's methods are the script's own from then on.
    pub(super) fn share_early(&mut self, name: &str) {
        let Some(body) = self.scopes.last() else { return };
        if body.saved.contains_key(name) || self.writers.contains_key(name) {
            return;
        }
        if self.shared_early.contains_key(name) || self.acc.room(self.shared_early.len()) {
            let open = self.scopes.len();
            let seen = self.shared_early.entry(name.to_string()).or_insert(open);
            *seen = (*seen).max(open);
        } else {
            self.shared_full = true;
        }
    }

    fn writer_bits(&self, receiver: Option<Sig<'s>>, before: Option<Sig<'s>>) -> u8 {
        match receiver {
            Some(r) if r.kind == T::Ident && !member_mark(before) => self.writers.get(r.text).copied().unwrap_or(0),
            _ => 0,
        }
    }

    /// A variable of [`Analyzer::writers`] used before `t` in any way but a member (`local.name`,
    /// `local['name']`) or an assignment to it ([`Analyzer::assignment`]), or whose `saveAs` or
    /// `exportFile` is assigned: another name may hold its object, or its methods may change, so its
    /// own methods are no longer known (`var b = local`, `f(local)`, `local.saveAs = w`,
    /// `local[k] = w`, `delete local.saveAs`). A property assignment whose name is such a variable's
    /// (`this.local = d`, `$.global['local'] = d`) may give the variable a new value through the global
    /// object, so its methods are no longer known either. An assignment to a property of the global
    /// object under any other key (`$.global[key] = d`, `this['lo' + 'cal'] = d`) may give any such
    /// variable a new value: no variable's methods are known after it.
    fn writer_use(&mut self, t: &Sig<'s>) {
        let [h0, h1, h2, h3] = self.h;
        let global_index_closed = std::mem::take(&mut self.global_index_closed);
        if t.is("[") {
            let global = (word(h0, "global") && is(h1, ".") && word(h2, "$") && !member_mark(h3))
                || ((word(h0, "this") || word(h0, "globalThis")) && !member_mark(h1));
            if global && self.global_indexes.len() < self.acc.limits.nesting {
                self.global_indexes.push(self.depth());
            }
        } else if t.is("]") {
            let depth = self.depth();
            self.global_index_closed = self.global_indexes.last() == Some(&(depth + 1));
            self.global_indexes.retain(|d| *d <= depth);
        }
        let writer = |s: Option<Sig<'s>>, before: Option<Sig<'s>>| s.filter(|s| self.writer_bits(Some(*s), before) != 0).map(|s| s.text);
        let changes = t.kind == T::Punct && (t.text == "=" || COMPOUND.contains(&t.text) || matches!(t.text, "++" | "--"));
        let key = t.is(":") && (is(h1, "{") || is(h1, ","));
        let escape = |writer: &dyn Fn(Option<Sig<'s>>, Option<Sig<'s>>) -> Option<&'s str>| -> Option<&'s str> {
            if word(h0, "delete") {
                // `delete local…`
                writer(Some(*t), h0)
            } else if let Some(name) = writer(h0, h1) {
                // `local` followed by anything but a member, its own assignment or an object key's `:`.
                (!t.is(".") && !t.is("[") && !t.is("=") && !key).then_some(name)
            } else if is(h1, ".") && changes && h0.is_some_and(|m| catalog::DOM_FILE_WRITES.contains(&m.text)) {
                // `local.saveAs = …`
                writer(h2, h3)
            } else if is(h0, "[") {
                // `local[k]` with a key that is not a string literal.
                writer(h1, h2).filter(|_| t.kind != T::Str)
            } else if is(h1, "[") && h0.is_some_and(|s| s.kind == T::Str) {
                // `local['name' + k]`.
                writer(h2, h3).filter(|_| !t.is("]"))
            } else if is(h0, "]") && is(h2, "[") && changes && h1.is_some_and(|m| catalog::DOM_FILE_WRITES.contains(&unquote(m.text, 32).as_str())) {
                // `local['saveAs'] = …`
                writer(h3, None)
            } else {
                None
            }
        };
        let escaped = escape(&writer);
        // A function body reads a variable of the enclosing code that no literal has given methods yet
        // in a way that changes or shares it: the body may run after the literal, as a hoisted
        // declaration does.
        let early = if escaped.is_none() && !self.scopes.is_empty() {
            let captured = |s: Option<Sig<'s>>, before: Option<Sig<'s>>| {
                s.filter(|s| {
                    s.kind == T::Ident
                        && !member_mark(before)
                        && !self.writers.contains_key(s.text)
                        && self.scopes.last().is_some_and(|b| !b.saved.contains_key(s.text))
                })
                .map(|s| s.text)
            };
            escape(&captured)
        } else {
            None
        };
        if let Some(name) = escaped {
            self.writers.remove(name);
        }
        if let Some(name) = early {
            self.share_early(name);
        }
        // A name read in a function body that the body does not declare, as a receiver or otherwise.
        if let Some(r) = h0.filter(|r| r.kind == T::Ident && !member_mark(h1) && !RESERVED.contains(&r.text)) {
            self.share_early(r.text);
        }
        if changes {
            let rebound = if is(h1, ".") {
                h0.filter(|s| s.kind == T::Ident).map(|s| s.text.to_string())
            } else if is(h0, "]") && is(h2, "[") {
                h1.filter(|s| s.kind == T::Str).map(|s| unquote(s.text, 256))
            } else {
                None
            };
            if let Some(name) = rebound {
                self.writers.remove(&name);
            } else if global_index_closed {
                // `$.global[key] = …`: the key may name any of them.
                self.writers.clear();
            }
        }
    }

    /// `(` after a name: a call ([`Self::classify_call`]). A call of a variable of
    /// [`Analyzer::writers`]'s method other than the `saveAs` or `exportFile` its literal defines
    /// (`local.install()`, `local['install']()`) may replace the object's methods through `this`, so
    /// after the call they are no longer known.
    fn call(&mut self, line: Option<usize>) {
        let [h0, h1, h2, h3] = self.h;
        let bits = self.writer_bits(h2, h3);
        let receiver = if is(h1, ".") && bits != 0 && h0.is_some_and(|s| s.kind == T::Ident && !Self::defines(bits, s.text)) {
            h2
        } else if is(h0, "]")
            && is(h2, "[")
            && self.index_receiver != 0
            && h1.is_some_and(|s| s.kind == T::Str && !Self::defines(self.index_receiver, &unquote(s.text, 32)))
        {
            h3
        } else {
            None
        };
        self.classify_call(line);
        if let Some(r) = receiver {
            self.writers.remove(r.text);
        }
    }

    /// Whether `bits` ([`Self::writer_bits`]) has `method`: the receiver's own `method` is called.
    fn defines(bits: u8, method: &str) -> bool {
        catalog::DOM_FILE_WRITES.iter().position(|m| *m == method).is_some_and(|i| bits & (1 << i) != 0)
    }

    /// Whether `receiver`, which `before` comes before, is a variable holding a File or Folder object
    /// (not a property of the same name), or a File or Folder object made in place or held in
    /// grouping parentheses ([`Sig::file`]): `Some(true)` for a Folder object whatever the path names
    /// ([`Sig::folder`], [`Analyzer::folder`]), unless the file gives names values the analysis
    /// doesn't read ([`Analyzer::unsure`]), `Some(false)` for an object that may be a File object, and
    /// `None` for anything else.
    fn held_file(&self, receiver: Option<Sig<'s>>, before: Option<Sig<'s>>) -> Option<bool> {
        let r = receiver?;
        if r.file {
            Some(r.folder && !self.unsure)
        } else if r.kind == T::Ident && !member_mark(before) && self.alias(&r) {
            Some(self.folder(r.text))
        } else {
            None
        }
    }

    fn string(&mut self, t: &Sig<'s>, line: Option<usize>) {
        let [h0, h1, h2, _] = self.h;
        if t.text.contains(catalog::ACTION_TEXT) {
            self.acc.feature(FeatureId::Actions, line);
        }
        // `SaveOptions['SAVECHANGES']`, or the value written as the string `"SaveOptions.SAVECHANGES"`.
        let value = unquote(t.text, 32);
        if value == catalog::SAVE_CHANGES
            || value.strip_prefix(catalog::SAVE_OPTIONS).and_then(|r| r.strip_prefix('.')) == Some(catalog::SAVE_CHANGES)
        {
            self.save_changes(line);
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
