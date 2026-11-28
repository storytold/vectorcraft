//! The names declared, read before the analysis.

use std::collections::{HashMap, HashSet};

use super::{
    Acc, Brackets, COMPOUND, Lexer, LineBreaks, Read, Sig, T, Token, catalog, directives, for_in, goes_on_after, goes_on_with, is, member_mark,
    unquote, word,
};

/// The names that give variables values the analysis doesn't read: a `with` statement makes an
/// object's properties stand for names in its body (ECMA-262 3rd edition, section 12.10), and `eval`,
/// `Function` and `$.evalFile` run code given as text. A string literal in brackets that names one
/// reaches it too (`this['eval']`), and so does a function's `constructor` property, which is
/// `Function` (section 15.3.4.1).
const UNREAD_BINDINGS: &[&str] = &["with", "eval", "Function", catalog::DOLLAR_EVAL_FILE];

/// The property a function reaches `Function` through.
const CONSTRUCTOR: &str = "constructor";

/// The longest property name, in characters, that [`Declared::rebound`] keeps. A string literal in
/// brackets that spells a longer one sets [`Declared::unsure`].
const PROPERTY_CHARS: usize = 256;

/// The names a file's function bodies and its global code declare, read before the analysis. A
/// function's `var` statements and the function declarations directly in its body make the names
/// they declare the function's own before any statement in it runs, and the function declarations
/// directly in the global code do the same for the whole file (ECMA-262 3rd edition, section
/// 10.1.3). A `var` statement in the global code does not change a property the global object
/// already has, such as the host's `alert` (sections 10.1.3, 10.1.5 and 10.2.1). The global code's
/// `var` statements are not read here. A function declaration inside a block, which ECMAScript 3
/// does not allow (sections 12.1 and 14), makes its name the script's own from where it stands.
#[derive(Default)]
pub(crate) struct Declared<'s> {
    /// For each function body, by the offset of its `{` in the text, the names it declares, in the
    /// order the bodies start.
    pub(super) bodies: Vec<(usize, Vec<&'s str>)>,
    /// The names the function declarations directly in the global code declare.
    pub(super) global: HashSet<&'s str>,
    /// The property names a member assignment writes (`o.d = …`, `o['d'] = …`). The object may be
    /// the global object, so a variable of that name may get a value the analysis doesn't read.
    pub(super) rebound: HashSet<String>,
    /// The names a function body assigns by name (`d = …`, `d += …`, `d++`, `for (d in …)`) other than
    /// in a `var` statement's declarations, unless a body around the assignment declared the name with
    /// `var` before it. The function may run at any time after it is defined,
    /// also before its body in the text when it is a declaration, so a variable of that name may lose
    /// the value an assignment the analysis read gave it.
    pub(super) body_assigned: HashSet<&'s str>,
    /// The file has an `#include` line: the included file's code may give its variables values.
    pub(super) includes: bool,
    /// The file gives names values the analysis doesn't read in other ways ([`UNREAD_BINDINGS`] or
    /// a `constructor` property used, a property read, called or assigned whose name in brackets is
    /// not one string or number literal, which may be a variable's name on the global object
    /// (ECMA-262 3rd edition, section 10.1.3) or one of [`UNREAD_BINDINGS`] whatever object the
    /// brackets follow, a property assigned whose name is a string literal longer than
    /// [`PROPERTY_CHARS`], or `Folder` assigned or declared), or `rebound` ran out of room: no
    /// object counts as a Folder object whatever the path names.
    pub(super) unsure: bool,
}

/// A function body [`Declarations`] is reading: the bracket depth inside it, its entry in
/// [`Declared::bodies`], the names it declares so far, and the bracket depth of the `var` statement
/// the body is in, which goes on when the body closes.
struct Body<'s> {
    depth: usize,
    index: usize,
    names: HashSet<&'s str>,
    var_depth: Option<usize>,
}

/// A `[` [`Declarations`] is reading: the bracket depth inside it, whether a property's name in
/// brackets starts at it (it comes after a value, where no array literal starts), and the tokens read
/// inside it so far: how many, and the kind of the first.
struct Key {
    depth: usize,
    member: bool,
    tokens: usize,
    first: Option<T>,
}

/// The pass that finds [`Declared`]. It follows brackets, function bodies and `var` statements by
/// the rules [`Analyzer`] follows them by, so that both passes find the same function bodies.
struct Declarations<'s, 'a, 'l> {
    acc: &'a mut Acc<'l>,
    brackets: Brackets,
    /// The three significant tokens before this one, the latest first.
    h: [Option<Sig<'s>>; 3],
    /// `function` was read at this bracket depth, and its body's `{` is still to come.
    function: Option<usize>,
    /// The bracket depth of the `var` statement being read.
    var_depth: Option<usize>,
    /// `name =` was read, and the first token of the value is next (`true`: after `new`). As in
    /// [`Analyzer::assignment`], that token doesn't start or end a `var` statement.
    assign: Option<bool>,
    /// The function bodies being read, the innermost last, up to [`crate::Limits::nesting`].
    bodies: Vec<Body<'s>>,
    /// The names the bodies being read declare so far, each with the number of those bodies that
    /// declare it.
    declared_around: HashMap<&'s str, usize>,
    /// The bracket depth inside the outermost function body past that limit being read. The names
    /// declared inside it are not kept.
    untracked: Option<usize>,
    /// The `[` brackets being read, the innermost last, up to [`crate::Limits::nesting`].
    keys: Vec<Key>,
    out: Declared<'s>,
}

/// What `src` declares ([`Declared`]), read in one pass over its tokens.
pub(crate) fn declarations<'s>(src: &'s str, acc: &mut Acc) -> Declared<'s> {
    let mut lexer = Lexer::new(src, acc.limits.nesting);
    let mut d = Declarations {
        acc,
        brackets: Brackets::default(),
        h: [None; 3],
        function: None,
        var_depth: None,
        assign: None,
        bodies: vec![],
        declared_around: HashMap::new(),
        untracked: None,
        keys: vec![],
        out: Declared::default(),
    };
    let mut breaks = LineBreaks::default();
    for t in lexer.by_ref() {
        match breaks.read(&t) {
            Read::Skip => {}
            Read::Directive => d.directive(&t),
            Read::Token(sig) => d.token(sig),
        }
    }
    let mut out = d.out;
    out.unsure |= out.rebound.contains(catalog::FOLDER);
    acc.steps = acc.steps.saturating_add(lexer.steps);
    out
}

impl<'s> Declarations<'s, '_, '_> {
    fn depth(&self) -> usize {
        self.brackets.depth()
    }

    fn token(&mut self, t: Sig<'s>) {
        let [h0, h1, _] = self.h;
        self.unread_binding(&t);
        if !t.is("]")
            && let Some(key) = self.keys.last_mut()
        {
            key.tokens = key.tokens.saturating_add(1);
            key.first = key.first.or(Some(t.kind));
        }
        // As in [`Analyzer::token`], a line break ends a `var` statement when it is outside the
        // statement's brackets, and the tokens on either side of it don't go on with the expression.
        if t.nl && self.var_depth == Some(self.depth()) && !goes_on_after(h0) && !goes_on_with(&t) {
            self.var_depth = None;
        }
        if t.kind == T::Punct {
            self.bracket(&t);
        } else if t.kind == T::Ident {
            self.declaration(&t);
        }
        let depth = self.depth();
        if let Some(after_new) = self.assign.take() {
            if t.word("new") && !after_new {
                self.assign = Some(true);
            }
        } else {
            if t.word("var") {
                self.var_depth = Some(depth);
            }
            if (t.is(";") || for_in(&t, h0, h1)) && self.var_depth == Some(depth) {
                self.var_depth = None;
            }
            if t.is("=") && h0.is_some_and(|s| s.kind == T::Ident) && !member_mark(h1) && !is(h1, "@") {
                self.assign = Some(false);
            }
        }
        // `function` that isn't a property's name: its body's `{` comes after its name and
        // parameters.
        if t.word("function") && !member_mark(h0) && !word(h0, "function") {
            self.function = Some(depth);
        }
        if !self.bodies.is_empty() || self.untracked.is_some() {
            self.body_assignment(&t, depth);
        }
        self.h = [Some(t), h0, h1];
    }

    /// `t` inside a function body at bracket depth `depth`: the name it assigns, when it assigns one
    /// by name outside a `var` statement's declarations, joins [`Declared::body_assigned`].
    fn body_assignment(&mut self, t: &Sig<'s>, depth: usize) {
        let [h0, h1, _] = self.h;
        let name = |s: Option<Sig<'s>>| s.filter(|s| s.kind == T::Ident).map(|s| s.text);
        let declares = word(h1, "var") || (is(h1, ",") && self.var_depth == Some(depth));
        let assigned = if t.kind == T::Punct && (t.text == "=" || COMPOUND.contains(&t.text) || matches!(t.text, "++" | "--")) {
            // `d = …`, `d += …` and `d++`, not `o.d = …` (a member: [`Declared::rebound`]) or `var d = …`.
            name(h0).filter(|_| !member_mark(h1) && !(t.text == "=" && declares))
        } else if t.kind == T::Ident && (is(h0, "++") || is(h0, "--")) {
            // `++d`.
            Some(t.text)
        } else if t.word("in") && (is(h1, "(") || word(h1, "var")) {
            // `for (d in …)`.
            name(h0)
        } else {
            None
        };
        // A name a body around `t` has declared with `var` before it is that body's own.
        let own = |name: &str| self.untracked.is_none() && self.declared_around.contains_key(name);
        if let Some(name) = assigned.filter(|n| !own(n)) {
            if self.out.body_assigned.contains(name) || self.acc.room(self.out.body_assigned.len()) {
                self.out.body_assigned.insert(name);
            } else {
                self.out.unsure = true;
            }
        }
    }

    /// `t`, a token before the brackets are followed past it: notes in [`Declared::rebound`] and
    /// [`Declared::unsure`] the ways the file gives names values the analysis doesn't read.
    fn unread_binding(&mut self, t: &Sig<'s>) {
        let [h0, h1, h2] = self.h;
        if t.kind == T::Ident && (UNREAD_BINDINGS.contains(&t.text) || t.text == CONSTRUCTOR) {
            self.out.unsure = true;
        }
        // `this['eval']`, `$['evalFile']` and `o['constructor']`.
        if t.kind == T::Str && is(h0, "[") {
            let name = unquote(t.text, PROPERTY_CHARS.saturating_add(1));
            if UNREAD_BINDINGS.contains(&name.as_str()) || name == CONSTRUCTOR {
                self.out.unsure = true;
            }
        }
        // `var Folder`, `function Folder` and `Folder = …`: `Folder` may stop being the platform's.
        if (t.word(catalog::FOLDER) && (word(h0, "var") || word(h0, "function") || (is(h0, ",") && self.var_depth.is_some())))
            || (t.is("=") && word(h0, catalog::FOLDER))
        {
            self.out.unsure = true;
        }
        if t.is("=") {
            let literal = |kind: T| h1.is_some_and(|s| s.kind == kind) && is(h2, "[");
            let property = if h0.is_some_and(|s| s.kind == T::Ident) && member_mark(h1) {
                h0.map(|s| s.text.to_string())
            } else if is(h0, "]") && literal(T::Str) {
                // The value spells a longer name than is kept: it may be any variable's.
                h1.map(|s| unquote(s.text, PROPERTY_CHARS.saturating_add(1))).filter(|name| name.chars().count() <= PROPERTY_CHARS)
            } else {
                None
            };
            if let Some(name) = property {
                if self.out.rebound.contains(&name) || self.acc.room(self.out.rebound.len()) {
                    self.out.rebound.insert(name);
                } else {
                    self.out.unsure = true;
                }
            } else if is(h0, "]") && !literal(T::Number) {
                // `o[k] = …` with a computed name, or a string literal too long to keep. A number
                // names no variable.
                self.out.unsure = true;
            }
        }
    }

    /// An open or close bracket, by the rules of [`Analyzer::bracket`]: the first `{` after
    /// `function` at the depth of `function` opens the function's body.
    fn bracket(&mut self, t: &Sig<'s>) {
        match t.text {
            "(" | "[" | "{" => {
                let body = t.text == "{" && self.function.take() == Some(self.depth());
                self.brackets.open(t, self.acc.limits.nesting);
                if t.text == "[" {
                    // After a value, `[` starts a property's name in brackets (ECMA-262 3rd
                    // edition, section 11.2.1); elsewhere it starts an array literal.
                    let member = !t.expr;
                    if self.keys.len() < self.acc.limits.nesting {
                        self.keys.push(Key { depth: self.depth(), member, tokens: 0, first: None });
                    } else if member {
                        self.out.unsure = true;
                    }
                }
                if !body {
                    return;
                }
                if self.bodies.len() < self.acc.limits.nesting {
                    let index = self.out.bodies.len();
                    self.out.bodies.push((t.start, vec![]));
                    self.bodies.push(Body { depth: self.depth(), index, names: HashSet::new(), var_depth: self.var_depth.take() });
                } else if self.untracked.is_none() {
                    self.untracked = Some(self.depth());
                }
            }
            ")" | "]" | "}" => {
                let inside = self.depth();
                if !self.brackets.close(t) {
                    return;
                }
                let depth = self.depth();
                // `o[k]`, `this['ev' + 'al']` and `o[('eval')]`, read, called or assigned: a name
                // in brackets that isn't one string or number literal may be any variable's on the
                // global object, or `eval`, `Function`, `evalFile` or `constructor`.
                if t.text == "]"
                    && let Some(key) = self.keys.pop_if(|k| k.depth == inside)
                    && key.member
                    && !(key.tokens == 1 && matches!(key.first, Some(T::Str | T::Number)))
                {
                    self.out.unsure = true;
                }
                while self.keys.pop_if(|k| depth < k.depth).is_some() {}
                if self.var_depth.is_some_and(|d| depth < d) {
                    self.var_depth = None;
                }
                while let Some(body) = self.bodies.pop_if(|b| depth < b.depth) {
                    self.var_depth = body.var_depth;
                    for name in body.names {
                        if let Some(n) = self.declared_around.get_mut(name) {
                            *n = n.saturating_sub(1);
                            if *n == 0 {
                                self.declared_around.remove(name);
                            }
                        }
                    }
                }
                if self.untracked.is_some_and(|d| depth < d) {
                    self.untracked = None;
                }
            }
            _ => {}
        }
    }

    /// The name `t`, when a `var` statement or a function declaration declares it: kept for the
    /// innermost function body when the `var` statement is in that body or the declaration is
    /// directly in it, and for the global code when the declaration is directly in the global code.
    /// Each body keeps up to [`crate::Limits::entries`] names, and so does the global code.
    fn declaration(&mut self, t: &Sig<'s>) {
        let [h0, ..] = self.h;
        let depth = self.depth();
        let var = word(h0, "var") || (is(h0, ",") && self.var_depth == Some(depth));
        let function = word(h0, "function") && h0.is_some_and(|f| f.stmt);
        if !(var || function) || self.untracked.is_some() {
            return;
        }
        if let Some(body) = self.bodies.last_mut() {
            if (var || body.depth == depth) && !body.names.contains(t.text) && self.acc.room(body.names.len()) {
                body.names.insert(t.text);
                *self.declared_around.entry(t.text).or_insert(0) += 1;
                if let Some((_, names)) = self.out.bodies.get_mut(body.index) {
                    names.push(t.text);
                }
            }
        } else if function && depth == 0 && !self.out.global.contains(t.text) && self.acc.room(self.out.global.len()) {
            self.out.global.insert(t.text);
        }
    }

    /// A preprocessor line ends the statement before it, as in [`Analyzer::directive`]. Inside a
    /// property's name in brackets it hides the tokens before the name (`this[`, a line, `'eval']`),
    /// so the name may be any.
    fn directive(&mut self, t: &Token<'s>) {
        if directives::parse(t.text).known == Some("include") {
            self.out.includes = true;
        }
        if self.keys.iter().any(|k| k.member) {
            self.out.unsure = true;
        }
        self.h = [None; 3];
        self.var_depth = None;
        self.assign = None;
    }
}
