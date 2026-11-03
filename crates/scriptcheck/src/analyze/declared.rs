//! The names declared, read before the analysis.

use std::collections::HashSet;

use super::{Acc, Brackets, Lexer, LineBreaks, Read, Sig, T, for_in, goes_on_after, goes_on_with, is, member_mark, word};

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

/// The pass that finds [`Declared`]. It follows brackets, function bodies and `var` statements by
/// the rules [`Analyzer`] follows them by, so that both passes find the same function bodies.
struct Declarations<'s, 'a, 'l> {
    acc: &'a mut Acc<'l>,
    brackets: Brackets,
    /// The two significant tokens before this one, the latest first.
    h: [Option<Sig<'s>>; 2],
    /// `function` was read at this bracket depth, and its body's `{` is still to come.
    function: Option<usize>,
    /// The bracket depth of the `var` statement being read.
    var_depth: Option<usize>,
    /// `name =` was read, and the first token of the value is next (`true`: after `new`). As in
    /// [`Analyzer::assignment`], that token doesn't start or end a `var` statement.
    assign: Option<bool>,
    /// The function bodies being read, the innermost last, up to [`crate::Limits::nesting`].
    bodies: Vec<Body<'s>>,
    /// The bracket depth inside the outermost function body past that limit being read. The names
    /// declared inside it are not kept.
    untracked: Option<usize>,
    out: Declared<'s>,
}

/// What `src` declares ([`Declared`]), read in one pass over its tokens.
pub(crate) fn declarations<'s>(src: &'s str, acc: &mut Acc) -> Declared<'s> {
    let mut lexer = Lexer::new(src, acc.limits.nesting);
    let mut d = Declarations {
        acc,
        brackets: Brackets::default(),
        h: [None; 2],
        function: None,
        var_depth: None,
        assign: None,
        bodies: vec![],
        untracked: None,
        out: Declared::default(),
    };
    let mut breaks = LineBreaks::default();
    for t in lexer.by_ref() {
        match breaks.read(&t) {
            Read::Skip => {}
            Read::Directive => d.directive(),
            Read::Token(sig) => d.token(sig),
        }
    }
    let out = d.out;
    acc.steps = acc.steps.saturating_add(lexer.steps);
    out
}

impl<'s> Declarations<'s, '_, '_> {
    fn depth(&self) -> usize {
        self.brackets.depth()
    }

    fn token(&mut self, t: Sig<'s>) {
        let [h0, h1] = self.h;
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
        self.h = [Some(t), h0];
    }

    /// An open or close bracket, by the rules of [`Analyzer::bracket`]: the first `{` after
    /// `function` at the depth of `function` opens the function's body.
    fn bracket(&mut self, t: &Sig<'s>) {
        match t.text {
            "(" | "[" | "{" => {
                let body = t.text == "{" && self.function.take() == Some(self.depth());
                self.brackets.open(t, self.acc.limits.nesting);
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
                if !self.brackets.close(t) {
                    return;
                }
                let depth = self.depth();
                if self.var_depth.is_some_and(|d| depth < d) {
                    self.var_depth = None;
                }
                while let Some(body) = self.bodies.pop_if(|b| depth < b.depth) {
                    self.var_depth = body.var_depth;
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
        let [h0, _] = self.h;
        let depth = self.depth();
        let var = word(h0, "var") || (is(h0, ",") && self.var_depth == Some(depth));
        let function = word(h0, "function") && h0.is_some_and(|f| f.stmt);
        if !(var || function) || self.untracked.is_some() {
            return;
        }
        if let Some(body) = self.bodies.last_mut() {
            if (var || body.depth == depth) && !body.names.contains(t.text) && self.acc.room(body.names.len()) {
                body.names.insert(t.text);
                if let Some((_, names)) = self.out.bodies.get_mut(body.index) {
                    names.push(t.text);
                }
            }
        } else if function && depth == 0 && !self.out.global.contains(t.text) && self.acc.room(self.out.global.len()) {
            self.out.global.insert(t.text);
        }
    }

    /// A preprocessor line ends the statement before it, as in [`Analyzer::directive`].
    fn directive(&mut self) {
        self.h = [None; 2];
        self.var_depth = None;
        self.assign = None;
    }
}
