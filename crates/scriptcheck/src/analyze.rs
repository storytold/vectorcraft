//! The analysis: a pass over a file's tokens that finds the names each function body and the global
//! code declare ([`Declared`]), then one pass that finds its directives, platform features,
//! document-model member chains, menu commands and problems. Each token is looked at together with
//! the four significant tokens before it. Open brackets are kept on a capped stack, a member chain
//! whose index or call is still open waits on a second one, and the function bodies being read are
//! kept on a third, each with what its parameters, the names it declares and the names its
//! assignments empty held outside it. A function body's parameters and the names it declares are
//! its own from its `{` on, and the names the global code's function declarations declare are the
//! script's own from the start of the file. The `catch` blocks being read are kept on a fourth,
//! each with what its parameter held outside it. The values in grouping parentheses being read
//! (`name = (…)`) are kept on a fifth, each with the operand read so far in its innermost pair, and
//! the name holds the value when the token after the last `)` ends it. A value without parentheses
//! that starts with `File`, `Folder` or a variable holding a File (`name = new File(…)`) is kept
//! there too, as one operand, and the name holds it when the token after the operand ends the
//! value. The value of a compound assignment (`name += …`) is kept there as well: the name holds what
//! it held while the value is read, and nothing the check follows after it. The conditional
//! expressions whose `:` is still to come are kept on a sixth, so that the `:` of one inside a value
//! doesn't end the value. All grouping parentheses being read are kept on a seventh, each with the
//! operand read so far at its depth, and a call on a File or Folder object in them (`(f).remove()`)
//! is a call on the object. Nothing recurses.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::catalog::{self, FeatureId};
use crate::directives;
use crate::lex::{Lexer, T, Token, expr_after};
use crate::report::{self, Acc, OutsideId};
use sig::{COMPOUND, LineBreaks, NEWER, Read, Sig, ends_value, for_in, goes_on_after, goes_on_with, is, member_mark, newer_number, unquote, word};

mod declared;
mod features;
mod sig;

pub(crate) use declared::{Declared, declarations};
use features::platform_name;

/// An `#include` line: what it names, its line, and the folders `#includepath` named at that point
/// (one list, shared by every line it applies to).
#[derive(Clone, Debug)]
pub(crate) struct IncludeRef {
    pub value: String,
    pub line: usize,
    pub paths: Rc<[String]>,
}

/// What variables hold, as a check learns it: the member chains assigned to them, and the ones
/// holding a File or Folder object.
#[derive(Clone, Debug, Default)]
pub(crate) struct Known {
    pub bound: HashMap<String, String>,
    pub files: HashSet<String>,
}

impl Known {
    /// The names known.
    pub fn len(&self) -> usize {
        self.bound.len() + self.files.len()
    }

    /// What `other` adds, up to [`crate::Limits::entries`] names of each kind: a name already
    /// bound keeps its chain.
    pub fn add(&mut self, other: Known, entries: usize) {
        for (name, chain) in other.bound {
            if self.bound.len() < entries || self.bound.contains_key(&name) {
                self.bound.entry(name).or_insert(chain);
            }
        }
        for name in other.files {
            if self.files.len() < entries {
                self.files.insert(name);
            }
        }
    }
}

/// What [`scan`] found besides the tallies it added to.
pub(crate) struct Scan {
    pub includes: Vec<IncludeRef>,
    /// The file is a JavaScript module (top-level `import` or `export`).
    pub module: bool,
    pub lines: usize,
    /// What the file's variables hold at its end.
    pub known: Known,
}

/// Scan `src` (already through [`unescaped`]): the script itself (`file` none) or a file it includes
/// (`file`: its path inside the script's folder), with the names it declares (`declared`, from
/// [`declarations`] on the same `src`), the `#includepath` folders `paths` in force at its start and
/// its variables holding what `known` says.
pub(crate) fn scan<'s>(src: &'s str, declared: &Declared<'s>, file: Option<&str>, paths: Rc<[String]>, known: &Known, acc: &mut Acc) -> Scan {
    let mut lexer = Lexer::new(src, acc.limits.nesting);
    let mut a = Analyzer::new(acc, file, paths, known, declared);
    let mut breaks = LineBreaks::default();
    for t in lexer.by_ref() {
        if let Some(code) = t.problem {
            a.problem(code, t.line);
        }
        match breaks.read(&t) {
            Read::Skip => {}
            Read::Directive => a.directive(&t),
            Read::Token(sig) => a.token(sig),
        }
    }
    if let Some(line) = lexer.unclosed_xml {
        a.problem(report::UNTERMINATED_XML, line);
    }
    let (includes, module, known) = a.finish();
    acc.steps = acc.steps.saturating_add(lexer.steps);
    let b = lexer.breaks;
    if usize::from(b.lf) + usize::from(b.crlf) + usize::from(b.cr) > 1 {
        acc.problem(report::MIXED_LINE_ENDINGS, None, file, None);
    }
    Scan { includes, module, lines: lexer.lines(), known }
}

/// `src` with the `\uXXXX` escapes in its names written as the characters they stand for (ECMA-262
/// 3rd edition, section 7.6), and `src` itself when no name has one. An escape for a character no
/// name may hold stays as written. Line breaks don't change, so every line keeps its number.
pub(crate) fn unescaped<'a>(src: Cow<'a, str>, acc: &mut Acc) -> Cow<'a, str> {
    match unescape_names(&src, acc) {
        Some(out) => Cow::Owned(out),
        None => src,
    }
}

fn unescape_names(src: &str, acc: &mut Acc) -> Option<String> {
    if !src.contains("\\u") {
        return None;
    }
    let mut out = String::with_capacity(src.len());
    let mut changed = false;
    let mut lexer = Lexer::new(src, acc.limits.nesting);
    for t in lexer.by_ref() {
        if t.kind == T::Ident && t.text.contains("\\u") {
            let name = unescape_name(t.text);
            changed |= name != t.text;
            out.push_str(&name);
        } else {
            out.push_str(t.text);
        }
    }
    acc.steps = acc.steps.saturating_add(lexer.steps);
    changed.then_some(out)
}

/// The name `text` with each `\uXXXX` escape for a character a name may hold written as that
/// character.
fn unescape_name(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("\\u") {
        out.push_str(rest.get(..i).unwrap_or_default());
        let after = rest.get(i + 2..).unwrap_or_default();
        let c = after
            .get(..4)
            .filter(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .and_then(char::from_u32)
            .filter(|&c| crate::lex::ident_part(c));
        match c {
            Some(c) => {
                out.push(c);
                rest = after.get(4..).unwrap_or_default();
            }
            None => {
                out.push_str("\\u");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// What the token after a few others decides.
#[derive(Clone, Copy, Debug, Default)]
enum Pending<'s> {
    #[default]
    None,
    /// `Window`: `(` and a string literal with the window's type may follow.
    Window(Option<usize>),
    WindowType(Option<usize>),
    /// `.executeMenuCommand(`: a string literal and `)` may follow.
    Menu(Option<usize>),
    MenuString(Option<usize>, &'s str),
    /// `.open(`: a string literal with the open mode may follow.
    Open(Option<usize>),
    /// A platform name where a property key or a label may stand (first, or after `{`, `,`, `;` or
    /// `}`): it is a key or a label when `:` follows.
    Name(&'s str, Option<usize>),
}

/// `name =` read: the next token says what the variable holds.
struct Assign {
    name: String,
    /// `new` came first.
    after_new: bool,
}

/// What a value the check follows holds: a member chain, or a File or Folder object.
enum Held {
    Chain(String),
    File,
}

/// `name = (`: a value in grouping parentheses. Its value is the last operand in each pair (ECMA-262
/// 3rd edition, sections 11.1.6 and 11.14), and the name holds it when the token after the last `)`
/// ends the value. A value without parentheses that starts with `File`, `Folder` or a variable
/// holding a File is read as one operand at the depth it starts at, and the name holds it when the
/// token after the operand ends the value. The value of a compound assignment is read the same way,
/// and the name holds nothing the check follows once it ends.
struct Group {
    /// The number the member chain an operand starts with carries in [`Chain::group`].
    id: usize,
    name: String,
    /// The bracket depth outside the first `(`; for a value without parentheses, the depth it starts
    /// at.
    base: usize,
    /// The bracket depth inside the innermost pair being read; for a value without parentheses, the
    /// depth it starts at.
    level: usize,
    /// The operand being read in that pair, or in a value without parentheses; after the last `)`, the
    /// value.
    operand: Operand,
    /// The last `)` was read.
    closed: bool,
    /// A value without parentheses: the first token at [`Group::level`] that ends a value ends it.
    bare: bool,
    /// The conditionals whose `:` was still to come when the value started ([`Analyzer::conds`]):
    /// the `:` of one read inside the value doesn't end it.
    conds: usize,
}

/// An operand in a grouped value's parentheses, a value without parentheses, or the operand in
/// grouping parentheses ([`Paren`]), as far as it has been read.
enum Operand {
    /// Its first token is next.
    Start,
    /// `new` came first.
    New,
    /// `File` or `Folder` came first, after `new` or not.
    FileName { new: bool, folder: bool },
    /// `File.` or `Folder.` came first: a member's name is next.
    FileMember { folder: bool },
    /// `File.openDialog`, `File.saveDialog` or `Folder.selectDialog` came first: its `(` is next.
    FileDialog,
    /// A call of `File`, `Folder` or one of their dialogs came first, and its `)` is still to come.
    FileCall,
    /// Grouping parentheses came first, and their `)` is still to come ([`Paren`] only).
    Pair,
    /// A member chain came first and is still being read.
    Chain,
    /// A complete value the check follows: the pair's value when its `)` comes next.
    Done(Held),
    /// Anything else.
    Other,
}

impl Operand {
    /// The operand after `t`, a token at the operand's depth, as far as `t` makes it a File or Folder
    /// object: `[new] File|Folder(…)`, `new File|Folder`, `File.openDialog(…)`, `File.saveDialog(…)`,
    /// `Folder.selectDialog(…)`, `Folder.<place>` ([`catalog::FOLDER_PLACES`]) or a variable holding
    /// one. `file`: `t` is the platform's `File` or `Folder`; `alias`: `t` names a variable holding a
    /// File or Folder object.
    fn file_step(self, t: &Sig, file: bool, alias: bool) -> Operand {
        match self {
            Operand::Start if t.word("new") => Operand::New,
            Operand::Start if file => Operand::FileName { new: false, folder: t.text == catalog::FOLDER },
            Operand::New if file => Operand::FileName { new: true, folder: t.text == catalog::FOLDER },
            Operand::Start if alias => Operand::Done(Held::File),
            Operand::FileName { .. } | Operand::FileDialog if t.is("(") => Operand::FileCall,
            Operand::FileName { new: false, folder } if t.is(".") => Operand::FileMember { folder },
            Operand::FileMember { folder: true } if catalog::FOLDER_PLACES.contains(&t.text) => Operand::Done(Held::File),
            Operand::FileMember { folder } if (if folder { catalog::FOLDER_DIALOGS } else { catalog::FILE_DIALOGS }).contains(&t.text) => {
                Operand::FileDialog
            }
            _ => Operand::Other,
        }
    }

    /// The operand is a File or Folder object when the `)` of its parentheses comes next: a complete
    /// one, or `new File` or `new Folder` without arguments.
    fn holds_file(&self) -> bool {
        matches!(self, Operand::Done(Held::File) | Operand::FileName { new: true, .. })
    }
}

/// Grouping parentheses (`(f)`, `(new File(p))`) being read: the bracket depth inside them, and the
/// operand read so far at that depth. Their value is the last operand (ECMA-262 3rd edition, sections
/// 11.1.6 and 11.14), and when it is a File or Folder object, their `)` ends one ([`Sig::file`]): a
/// call on it is a call on the object.
struct Paren {
    level: usize,
    operand: Operand,
}

/// A document-model member chain being read (`app.activeDocument.layers[0].name`).
struct Chain {
    /// The chain as written: the root (a variable's chain in place of the variable), `.name` for a
    /// member, `[]` for an index and `()` for a call.
    path: String,
    chars: usize,
    /// The path reached [`crate::Limits::echo`] and ends with `…`.
    cut: bool,
    /// Members, indexes and calls after the root.
    segments: usize,
    /// The root is `activeDocument`, `documents` or `selection`, which count as a use alone.
    bare: bool,
    line: usize,
    /// A `.` came last: a member's name comes next.
    after_dot: bool,
    /// The last member's name starts with `set` or `remove`.
    last_change: bool,
    /// A member whose name starts with `set` or `remove` was called.
    change_call: bool,
    /// While its index or call is open: the depth of brackets outside it.
    wait: usize,
    /// The variable it is assigned to: bound to it when the statement ends with the chain.
    assign: Option<String>,
    /// The grouped value ([`Group::id`]) whose operand starts with the chain.
    group: Option<usize>,
}

impl Chain {
    fn new(root: String, bare: bool, line: usize) -> Self {
        let (chars, cut) = (root.chars().count(), root.ends_with('…'));
        Chain {
            path: root,
            chars,
            cut,
            segments: 0,
            bare,
            line,
            after_dot: false,
            last_change: false,
            change_call: false,
            wait: 0,
            assign: None,
            group: None,
        }
    }

    fn append(&mut self, s: &str, cap: usize) {
        for c in s.chars() {
            if self.cut {
                return;
            }
            if self.chars + 1 >= cap {
                self.path.push('…');
                self.chars += 1;
                self.cut = true;
                return;
            }
            self.path.push(c);
            self.chars += 1;
        }
    }

    fn member(&mut self, name: &str, cap: usize) {
        self.segments += 1;
        self.append(".", cap);
        self.append(name, cap);
        self.last_change = name.starts_with("set") || name.starts_with("remove");
    }

    fn index_or_call(&mut self, call: bool, cap: usize) {
        self.segments += 1;
        self.change_call |= call && self.last_change;
        self.last_change = false;
        self.append(if call { "()" } else { "[]" }, cap);
    }
}

/// A function body being read: the bracket depth inside it, and what each name declared in it
/// (its parameters, `var`, `function name`) or emptied by an assignment in it held outside it, put
/// back when the body closes. The names it declares are its own from its `{` ([`Declared`]). A
/// member chain or a File object assigned in it to an outer name stays.
struct Scope {
    depth: usize,
    saved: HashMap<String, Saved>,
    /// The bracket depth of the `var` statement the body is in, if any: it goes on when the body
    /// closes.
    var_depth: Option<usize>,
}

/// A `catch` block being read: the bracket depth inside it, its parameter, and what that name held
/// outside the block, put back when the block closes. The parameter is the block's own (ECMA-262
/// 3rd edition, section 12.14); the other names in the block are those of the code around it.
struct Catch {
    depth: usize,
    name: String,
    saved: Saved,
    /// The depth of the `catch` block around this one with the same parameter, if any.
    outer: Option<usize>,
}

/// What a name held outside the function body or `catch` block that declares it.
struct Saved {
    /// The member chain assigned to it.
    chain: Option<String>,
    /// It held a File or Folder object.
    file: bool,
    /// For a root of the document model: whether the script's own variable hid the root.
    shadowed: Option<bool>,
    /// It was a platform name the script had a meaning of its own for.
    own: bool,
}

/// The open brackets and their lines, up to [`crate::Limits::nesting`]; the brackets past that are
/// counted, not kept. Both passes over a file follow brackets with it.
#[derive(Default)]
struct Brackets {
    open: Vec<(char, usize)>,
    overflow: usize,
}

impl Brackets {
    fn depth(&self) -> usize {
        self.open.len() + self.overflow
    }

    /// Open the bracket `t`: false when `cap` brackets are kept already, and `t` is only counted.
    fn open(&mut self, t: &Sig, cap: usize) -> bool {
        if self.open.len() < cap {
            self.open.push((t.text.chars().next().unwrap_or('('), t.line));
            return true;
        }
        self.overflow += 1;
        false
    }

    /// Close the innermost bracket when `t` matches it: false when it doesn't.
    fn close(&mut self, t: &Sig) -> bool {
        if self.overflow > 0 {
            self.overflow -= 1;
            return true;
        }
        let opening = match t.text {
            ")" => '(',
            "]" => '[',
            _ => '{',
        };
        if self.open.last().is_some_and(|(c, _)| *c == opening) {
            self.open.pop();
            return true;
        }
        false
    }
}

struct Analyzer<'s, 'a, 'l> {
    acc: &'a mut Acc<'l>,
    file: Option<&'a str>,
    /// The four significant tokens before this one, the latest first.
    h: [Option<Sig<'s>>; 4],
    brackets: Brackets,
    /// Nesting too deep to follow was reported: it is reported once.
    deep: bool,
    active: Option<Chain>,
    /// Chains whose index or call is open, the innermost last.
    suspended: Vec<Chain>,
    /// The active chain waits for the bracket this token opens.
    suspend: bool,
    /// The document-model roots the script declares a variable of its own for.
    shadowed: [bool; 4],
    /// Variables assigned from a member chain, and the chain.
    bound: HashMap<String, String>,
    /// Variables holding a File or Folder object.
    files: HashSet<String>,
    /// The platform names ([`platform_name`]) the script declares or assigns a meaning of its own
    /// for: they no longer stand for the platform's objects.
    own_names: HashSet<String>,
    /// The function bodies being read, the innermost last, up to [`crate::Limits::nesting`].
    scopes: Vec<Scope>,
    /// The names each function body and the global code declare, read before the analysis.
    declared: &'a Declared<'s>,
    /// The first entry of `declared.bodies` whose body's `{` hasn't been read yet.
    next_body: usize,
    /// `function` was read at this bracket depth, and its body's `{` is still to come.
    function: Option<usize>,
    /// The parameters of the function whose body's `{` is still to come, and the bracket depth
    /// inside the parentheses they are read in.
    params: Option<(usize, Vec<String>)>,
    /// The name of the function expression whose parameters are still to come: a variable inside
    /// that function only.
    expression_name: Option<String>,
    /// `catch` was read at this bracket depth and its block's `{` is still to come, with the name
    /// in its parentheses once read.
    catch: Option<(usize, Option<String>)>,
    /// The `catch` blocks being read, the innermost last, up to [`crate::Limits::nesting`].
    catches: Vec<Catch>,
    /// The parameters of the `catch` blocks being read, each with the depth inside the innermost
    /// block it names.
    catch_names: HashMap<String, usize>,
    /// The calls of `File`, `Folder` and their dialogs still open, by the bracket depth outside their
    /// `(`, up to [`crate::Limits::nesting`] of them.
    file_calls: Vec<usize>,
    /// The values in grouping parentheses, the values without them that may hold a File, and the
    /// values of compound assignments, being read, the innermost last, up to
    /// [`crate::Limits::nesting`] of them.
    groups: Vec<Group>,
    /// The id of the last value put on `groups`.
    group_id: usize,
    /// The grouping parentheses being read, the innermost last, up to [`crate::Limits::nesting`] of
    /// them.
    parens: Vec<Paren>,
    /// The bracket depth of each `?` whose `:` is still to come, the latest last, up to
    /// [`crate::Limits::nesting`] of them.
    conds: Vec<usize>,
    assign: Option<Assign>,
    /// The bracket depth of the `var` statement being read.
    var_depth: Option<usize>,
    pending: Pending<'s>,
    module_candidate: bool,
    module: bool,
    includes: Vec<IncludeRef>,
    /// The folders the last `#includepath` named.
    paths: Rc<[String]>,
    /// The last construct of later syntax reported, and its line: reported once per line.
    last_newer: Option<(usize, &'s str)>,
}

impl<'s, 'a, 'l> Analyzer<'s, 'a, 'l> {
    fn new(acc: &'a mut Acc<'l>, file: Option<&'a str>, paths: Rc<[String]>, known: &Known, declared: &'a Declared<'s>) -> Self {
        let global: Vec<&'s str> = declared.global.iter().copied().collect();
        let mut a = Analyzer {
            acc,
            file,
            h: [None; 4],
            brackets: Brackets::default(),
            deep: false,
            active: None,
            suspended: vec![],
            suspend: false,
            shadowed: [false; 4],
            bound: known.bound.clone(),
            files: known.files.clone(),
            own_names: HashSet::new(),
            scopes: vec![],
            declared,
            next_body: 0,
            function: None,
            params: None,
            expression_name: None,
            catch: None,
            catches: vec![],
            catch_names: HashMap::new(),
            file_calls: vec![],
            groups: vec![],
            group_id: 0,
            parens: vec![],
            conds: vec![],
            assign: None,
            var_depth: None,
            pending: Pending::None,
            module_candidate: false,
            module: false,
            includes: vec![],
            paths,
            last_newer: None,
        };
        // The names the global code's function declarations declare are the script's own from the
        // start of the file, before any assignment runs: a member chain or a File object that another
        // file's variable of that name holds stays.
        for name in global {
            a.acc.steps = a.acc.steps.saturating_add(1);
            a.make_own(name);
            a.hide_root(name);
        }
        a
    }

    /// `line` for a tally's lines: only the script's own lines are listed.
    fn agg(&self, line: usize) -> Option<usize> {
        self.file.is_none().then_some(line)
    }

    fn depth(&self) -> usize {
        self.brackets.depth()
    }

    /// A problem at `line` of the file.
    fn problem(&mut self, code: &'static str, line: usize) {
        if code == report::TOO_DEEP {
            if self.deep {
                return;
            }
            self.deep = true;
        }
        self.acc.problem(code, Some(line), self.file, None);
    }

    /// This token names a variable being declared: it follows `var` or `function`, a `,` in a `var`
    /// statement, the `(` or a `,` of a function's parameters, or the `(` after `catch`.
    fn declaring(&self) -> bool {
        word(self.h[0], "var")
            || word(self.h[0], "function")
            || (is(self.h[0], ",") && self.var_depth == Some(self.depth()))
            || self.parameter()
            || self.catch_parameter()
    }

    /// This token names a `catch` block's parameter: it follows the `(` after `catch` (not after a
    /// property named `catch`).
    fn catch_parameter(&self) -> bool {
        is(self.h[0], "(") && word(self.h[1], "catch") && !member_mark(self.h[2])
    }

    /// This token names one of a function's parameters: it follows the `(` or a `,` of the
    /// parentheses after `function`.
    fn parameter(&self) -> bool {
        (is(self.h[0], "(") || is(self.h[0], ",")) && self.params.as_ref().is_some_and(|(depth, _)| *depth == self.depth())
    }

    fn token(&mut self, mut t: Sig<'s>) {
        self.acc.steps = self.acc.steps.saturating_add(1);
        // The name of a grouped value whose last `)` was read, or of a value without parentheses that
        // `t` ends, holds the value, or nothing the check follows, before `t` starts a member chain or
        // closes a function body.
        self.settle_groups(&t);
        // A line break ends a `var` statement when it is outside the statement's brackets, and the
        // tokens on either side of it don't go on with the expression.
        if t.nl && self.var_depth == Some(self.depth()) && !goes_on_after(self.h[0]) && !goes_on_with(&t) {
            self.var_depth = None;
        }
        self.declaration_without_value(&t);
        let consumed = self.continue_chain(&t);
        let before = self.depth();
        if t.kind == T::Punct {
            t.file = self.bracket(&t);
        } else {
            t.file = self.folder_place(&t);
        }
        self.conditional(&t);
        self.add_parameter(&t);
        let started = !consumed && t.kind == T::Ident && self.start_chain(&t);
        self.group_step(&t, started, before);
        self.paren_step(&t, before);
        self.assignment(&t, started);
        self.features(&t);
        self.newer(&t);
        self.module_check(&t);
        self.h = [Some(t), self.h[0], self.h[1], self.h[2]];
    }

    // ---------- member chains ----------

    /// Whether the active chain takes `t` (a `.`, a member's name, `[` or `(`); any other token ends
    /// it.
    fn continue_chain(&mut self, t: &Sig<'s>) -> bool {
        let cap = self.acc.limits.echo;
        let Some(c) = self.active.as_mut() else { return false };
        if c.after_dot {
            c.after_dot = false;
            if t.kind == T::Ident {
                c.member(t.text, cap);
                return true;
            }
        } else if t.is(".") {
            c.after_dot = true;
            return true;
        } else if t.is("[") || t.is("(") {
            c.index_or_call(t.is("("), cap);
            self.suspend = true;
            return true;
        }
        self.end_active(Some(t));
        false
    }

    /// Start a chain at `t` when it names a root of the document model or a variable assigned
    /// from a chain.
    fn start_chain(&mut self, t: &Sig<'s>) -> bool {
        if member_mark(self.h[0]) || self.declaring() {
            return false;
        }
        let (root, bare) = if let Some(path) = self.bound.get(t.text) {
            (path.clone(), false)
        } else if let Some(i) = catalog::DOM_ROOTS.iter().position(|r| *r == t.text)
            && !self.shadowed.get(i).copied().unwrap_or(true)
        {
            (t.text.to_string(), i > 0)
        } else {
            return false;
        };
        self.active = Some(Chain::new(root, bare, t.line));
        true
    }

    fn end_active(&mut self, ending: Option<&Sig<'s>>) {
        if let Some(c) = self.active.take() {
            self.record(c, ending);
        }
    }

    /// A chain read to its end (`ending`: the token after it, none at the end of a statement).
    fn record(&mut self, c: Chain, ending: Option<&Sig<'s>>) {
        let line = self.agg(c.line);
        // `{selection: …}` names a property.
        let key = c.segments == 0 && ending.is_some_and(|t| t.is(":"));
        let then = |rest: &str| rest.is_empty() || rest.starts_with(['.', '(', '[']);
        if !key {
            let method = c.path.strip_prefix("app.").is_some_and(|rest| catalog::APP_METHODS.iter().any(|m| rest.strip_prefix(m).is_some_and(then)));
            if (c.segments > 0 || c.bare) && !method {
                self.acc.member(&c.path);
            }
            if c.path.strip_prefix("app.preferences").is_some_and(then) {
                self.acc.feature(FeatureId::Preferences, line);
                if c.change_call || ending.is_some_and(|t| t.is("=")) {
                    self.acc.outside(OutsideId::Preferences, line, None);
                }
            }
            if ["app.name", "app.version"].iter().any(|p| c.path.strip_prefix(p).is_some_and(then)) {
                self.acc.feature(FeatureId::HostCheck, line);
            }
        }
        // A chain that an operand in a grouped value's parentheses starts with is complete: the
        // operand is the chain when the pair's `)` comes next.
        if let Some(id) = c.group
            && let Some(g) = self.groups.last_mut()
            && g.id == id
            && matches!(g.operand, Operand::Chain)
        {
            g.operand = Operand::Done(Held::Chain(c.path.clone()));
        }
        if let Some(name) = c.assign {
            let ends = ending.is_none_or(ends_value);
            if ends && !key {
                self.bind(name, c.path);
            } else {
                self.unbind(&name);
            }
        }
    }

    fn bind(&mut self, name: String, path: String) {
        self.files.remove(&name);
        if self.bound.contains_key(&name) || self.acc.room(self.bound.len()) {
            self.bound.insert(name, path);
        }
    }

    /// `name` holds a File or Folder object.
    fn hold_file(&mut self, name: String) {
        self.bound.remove(&name);
        if self.files.contains(&name) || self.acc.room(self.files.len()) {
            self.files.insert(name);
        }
    }

    /// `t`, when it names a parameter of the function whose body's `{` is still to come: kept, up to
    /// [`crate::Limits::entries`] of them, for the body to declare. A `catch` block's parameter is
    /// kept for the block.
    fn add_parameter(&mut self, t: &Sig<'s>) {
        if t.kind != T::Ident {
            return;
        }
        if self.catch_parameter() {
            if let Some((_, name)) = self.catch.as_mut() {
                *name = Some(t.text.to_string());
            }
            return;
        }
        if !self.parameter() {
            return;
        }
        if let Some((_, names)) = self.params.as_mut()
            && self.acc.room(names.len())
        {
            names.push(t.text.to_string());
        }
    }

    /// `name` is declared (`var name`, `function name`, a parameter), or an assignment empties it.
    /// Inside a function body, what it held outside the body is kept, to put back when the body
    /// closes.
    fn keep_outer(&mut self, name: &str) {
        // The parameter of a `catch` block inside the body is the block's own: the block puts back
        // what it held.
        if self.in_catch(name) {
            return;
        }
        let Some(scope) = self.scopes.last() else { return };
        if scope.saved.contains_key(name) || !self.acc.room(scope.saved.len()) {
            return;
        }
        let saved = self.saved(name);
        if let Some(scope) = self.scopes.last_mut() {
            scope.saved.insert(name.to_string(), saved);
        }
    }

    /// The names [`Declared`] has for the function body whose `{` starts at `start`.
    fn declared_in(&mut self, start: usize) -> &'a [&'s str] {
        let declared = self.declared;
        let bodies = &declared.bodies;
        while bodies.get(self.next_body).is_some_and(|(at, _)| *at < start) {
            self.next_body += 1;
        }
        match bodies.get(self.next_body) {
            Some((at, names)) if *at == start => names,
            _ => &[],
        }
    }

    /// `name`, declared in the innermost function body, is the body's own from its `{`: what it held
    /// outside is kept to put back when the body closes. A name the body's table has no room for
    /// stays as it is.
    fn hoist(&mut self, name: &str) {
        self.keep_outer(name);
        if self.scopes.last().is_some_and(|s| s.saved.contains_key(name)) {
            self.empty(name);
        }
    }

    /// `name` was made its own by what [`Declared`] has for the innermost function body this token
    /// is directly in, or for the global code this token is directly in.
    fn hoisted(&self, name: &str) -> bool {
        match self.scopes.last() {
            Some(s) => s.depth == self.depth() && s.saved.contains_key(name),
            None => self.depth() == 0 && self.declared.global.contains(name),
        }
    }

    /// `name` is the parameter of a `catch` block being read inside the innermost function body, or
    /// outside any function.
    fn in_catch(&self, name: &str) -> bool {
        self.catch_names.get(name).is_some_and(|&depth| self.scopes.last().is_none_or(|s| depth > s.depth))
    }

    /// What `name` holds now.
    fn saved(&self, name: &str) -> Saved {
        let shadowed = catalog::DOM_ROOTS.iter().position(|r| *r == name).and_then(|i| self.shadowed.get(i).copied());
        Saved { chain: self.bound.get(name).cloned(), file: self.files.contains(name), shadowed, own: self.own_names.contains(name) }
    }

    /// `name` holds `saved` again.
    fn restore(&mut self, name: String, saved: Saved) {
        self.acc.steps = self.acc.steps.saturating_add(1);
        match saved.chain {
            Some(chain) => {
                self.bound.insert(name.clone(), chain);
            }
            None => {
                self.bound.remove(&name);
            }
        }
        if saved.file {
            self.files.insert(name.clone());
        } else {
            self.files.remove(&name);
        }
        if saved.own {
            self.own_names.insert(name.clone());
        } else {
            self.own_names.remove(&name);
        }
        if let Some(hidden) = saved.shadowed
            && let Some(i) = catalog::DOM_ROOTS.iter().position(|r| *r == name)
            && let Some(s) = self.shadowed.get_mut(i)
        {
            *s = hidden;
        }
    }

    /// The function bodies and `catch` blocks that the bracket just read closed: each name declared
    /// in them gets back what it held outside them, and the `var` statement a body was in goes on.
    fn close_scopes(&mut self) {
        while self.scopes.last().is_some_and(|s| self.depth() < s.depth) {
            let Some(scope) = self.scopes.pop() else { return };
            self.var_depth = scope.var_depth;
            for (name, saved) in scope.saved {
                self.restore(name, saved);
            }
        }
        while self.catches.last().is_some_and(|c| self.depth() < c.depth) {
            let Some(c) = self.catches.pop() else { return };
            match c.outer {
                Some(depth) => self.catch_names.insert(c.name.clone(), depth),
                None => self.catch_names.remove(&c.name),
            };
            self.restore(c.name, c.saved);
        }
    }

    /// `name` has a meaning of the script's own from here: a platform name no longer stands for the
    /// platform's object.
    fn make_own(&mut self, name: &str) {
        if platform_name(name) {
            self.own_names.insert(name.to_string());
        }
    }

    /// `name` now holds something else: no chain, no file, no longer a root of the document model,
    /// and no longer a platform's object. Inside a function body this lasts until the body closes.
    fn unbind(&mut self, name: &str) {
        self.keep_outer(name);
        self.empty(name);
    }

    /// `name` holds nothing the check follows, with a meaning of the script's own.
    fn empty(&mut self, name: &str) {
        self.make_own(name);
        self.bound.remove(name);
        self.files.remove(name);
        self.hide_root(name);
    }

    /// A root of the document model named `name` no longer stands for the root.
    fn hide_root(&mut self, name: &str) {
        if let Some(i) = catalog::DOM_ROOTS.iter().position(|r| *r == name)
            && let Some(s) = self.shadowed.get_mut(i)
        {
            *s = true;
        }
    }

    // ---------- brackets ----------

    /// An open or close bracket: true for the `)` that ends a call of `File` or `Folder`, or grouping
    /// parentheses that hold a File or Folder object ([`Analyzer::close_parens`]).
    fn bracket(&mut self, t: &Sig<'s>) -> bool {
        match t.text {
            "(" | "[" | "{" => {
                if std::mem::take(&mut self.suspend)
                    && let Some(mut c) = self.active.take()
                {
                    if self.suspended.len() < self.acc.limits.nesting {
                        c.wait = self.depth();
                        self.suspended.push(c);
                    } else {
                        self.record(c, Some(t));
                    }
                }
                // The first `{` after `function` at this depth opens the function's body, and a `(`
                // right after `function` or its name opens its parameters.
                let body = t.text == "{" && self.function.take() == Some(self.depth());
                let params = t.text == "(" && self.function == Some(self.depth()) && (word(self.h[0], "function") || word(self.h[1], "function"));
                // The first `{` after `catch` and its parentheses at this depth opens its block.
                let depth = self.depth();
                let catch = if t.text == "{" { self.catch.take().filter(|(d, _)| *d == depth).and_then(|(_, name)| name) } else { None };
                let file_call = t.text == "(" && self.calls_for_file();
                if file_call && self.file_calls.len() < self.acc.limits.nesting {
                    self.file_calls.push(self.depth());
                }
                self.open(t);
                if let Some(name) = catch
                    && self.catches.len() < self.acc.limits.nesting
                {
                    let saved = self.saved(&name);
                    let outer = self.catch_names.insert(name.clone(), self.depth());
                    self.empty(&name);
                    self.catches.push(Catch { depth: self.depth(), name, saved, outer });
                }
                if params {
                    // A function expression's name is the body's own, as its parameters are.
                    self.params = Some((self.depth(), self.expression_name.take().into_iter().collect()));
                }
                if body {
                    let params = self.params.take().map(|(_, names)| names).unwrap_or_default();
                    if self.scopes.len() < self.acc.limits.nesting {
                        self.scopes.push(Scope { depth: self.depth(), saved: HashMap::new(), var_depth: self.var_depth.take() });
                        // The parameters are the body's own names, and so are the names it declares.
                        for name in params {
                            self.acc.steps = self.acc.steps.saturating_add(1);
                            self.keep_outer(&name);
                            self.unbind(&name);
                        }
                        for &name in self.declared_in(t.start) {
                            self.acc.steps = self.acc.steps.saturating_add(1);
                            self.hoist(name);
                        }
                    }
                }
            }
            ")" | "]" | "}" => {
                self.end_active(Some(t));
                if self.close(t) {
                    if self.var_depth.is_some_and(|d| self.depth() < d) {
                        self.var_depth = None;
                    }
                    if self.suspended.last().is_some_and(|c| c.wait == self.depth()) {
                        self.active = self.suspended.pop();
                    }
                    self.close_scopes();
                    let grouped = self.close_parens(t);
                    if t.text == ")" && self.file_calls.last() == Some(&self.depth()) {
                        self.file_calls.pop();
                        return true;
                    }
                    return grouped;
                }
            }
            _ => {}
        }
        false
    }

    fn open(&mut self, t: &Sig<'s>) {
        if !self.brackets.open(t, self.acc.limits.nesting) {
            self.problem(report::TOO_DEEP, t.line);
        }
    }

    /// Close the innermost bracket when `t` matches it: false (and a problem) when it doesn't.
    fn close(&mut self, t: &Sig<'s>) -> bool {
        if self.brackets.close(t) {
            return true;
        }
        self.acc.problem(report::UNEXPECTED_BRACKET, Some(t.line), self.file, Some(t.text));
        false
    }

    /// A `(` here calls something that gives a File or Folder object: `File` or `Folder` (with or
    /// without `new`), or `File.openDialog`, `File.saveDialog` or `Folder.selectDialog`, each the
    /// platform's.
    fn calls_for_file(&self) -> bool {
        let [h0, h1, h2, h3] = self.h;
        let platform = |s: Option<Sig>, name: &str| word(s, name) && !self.own_names.contains(name);
        let constructor = (platform(h0, catalog::FILE) || platform(h0, catalog::FOLDER)) && !member_mark(h1);
        let dialog = h0.filter(|s| s.kind == T::Ident).is_some_and(|m| {
            is(h1, ".")
                && !member_mark(h3)
                && ((platform(h2, catalog::FILE) && catalog::FILE_DIALOGS.contains(&m.text))
                    || (platform(h2, catalog::FOLDER) && catalog::FOLDER_DIALOGS.contains(&m.text)))
        });
        constructor || dialog
    }

    /// `t` names one of the platform's `Folder` places after `Folder.` (`Folder.temp`): a Folder
    /// object.
    fn folder_place(&self, t: &Sig<'s>) -> bool {
        let [h0, h1, h2, _] = self.h;
        t.kind == T::Ident
            && catalog::FOLDER_PLACES.contains(&t.text)
            && is(h0, ".")
            && word(h1, catalog::FOLDER)
            && !self.own_names.contains(catalog::FOLDER)
            && !member_mark(h2)
    }

    // ---------- variables ----------

    /// `var name` without `= value`. In a function body that doesn't have the name yet, it is a new
    /// variable of the body's own, which may hide a root of the document model. A `var` without a
    /// value doesn't change a variable that exists (ECMA-262 3rd edition, section 10.1.3), so a name
    /// the body already has, the parameter of a `catch` block it is in, and any name outside a
    /// function keep what they hold.
    fn declaration_without_value(&mut self, t: &Sig<'s>) {
        let Some(name) = self.h[0].filter(|s| s.kind == T::Ident) else { return };
        let declared = word(self.h[1], "var") || (is(self.h[1], ",") && self.var_depth == Some(self.depth()));
        if !declared || t.is("=") {
            return;
        }
        if self.in_catch(name.text) {
            self.declare_in_catch(name.text);
        } else if self.scopes.last().is_some_and(|s| !s.saved.contains_key(name.text)) {
            self.keep_outer(name.text);
            self.unbind(name.text);
        }
    }

    /// `var name` in a `catch` block whose parameter is `name`. In a function body that doesn't have
    /// the name yet, the declaration makes it the function's own variable (ECMA-262 3rd edition,
    /// section 10.1.3), with no value: the parameter keeps what it holds, and after the outermost
    /// `catch` block in the body with that parameter the name holds nothing.
    fn declare_in_catch(&mut self, name: &str) {
        let Some(&innermost) = self.catch_names.get(name) else { return };
        let Some(scope) = self.scopes.last() else { return };
        let body = scope.depth;
        if scope.saved.contains_key(name) || !self.acc.room(scope.saved.len()) {
            return;
        }
        // From the innermost `catch` block with this parameter out to the outermost one in the body.
        let mut depth = innermost;
        let i = loop {
            self.acc.steps = self.acc.steps.saturating_add(1);
            let Ok(i) = self.catches.binary_search_by_key(&depth, |c| c.depth) else { return };
            match self.catches.get(i).and_then(|c| c.outer) {
                Some(outer) if outer > body => depth = outer,
                _ => break i,
            }
        };
        let Some(c) = self.catches.get_mut(i) else { return };
        let empty = Saved { chain: None, file: false, shadowed: catalog::DOM_ROOTS.contains(&name).then_some(true), own: platform_name(name) };
        let outside = std::mem::replace(&mut c.saved, empty);
        if let Some(scope) = self.scopes.last_mut() {
            scope.saved.insert(name.to_string(), outside);
        }
    }

    fn assignment(&mut self, t: &Sig<'s>, started: bool) {
        // The first token of what `name =` assigns.
        if let Some(mut a) = self.assign.take() {
            if started {
                if let Some(c) = self.active.as_mut() {
                    c.assign = Some(a.name);
                }
            } else if t.word("new") && !a.after_new {
                a.after_new = true;
                self.assign = Some(a);
            } else if (t.word(catalog::FILE) || t.word(catalog::FOLDER)) && !self.own_names.contains(t.text) {
                self.start_value(a.name, Operand::FileName { new: a.after_new, folder: t.text == catalog::FOLDER });
            } else if !a.after_new && t.kind == T::Ident && self.files.contains(t.text) {
                self.start_value(a.name, Operand::Done(Held::File));
            } else if !a.after_new && t.is("(") {
                self.start_group(a.name);
            } else {
                self.unbind(&a.name);
            }
            return;
        }
        if t.word("var") {
            self.var_depth = Some(self.depth());
        }
        if (t.is(";") || for_in(t, self.h[0], self.h[1])) && self.var_depth == Some(self.depth()) {
            self.var_depth = None;
        }
        if t.is("=")
            && let Some(name) = self.h[0].filter(|s| s.kind == T::Ident)
        {
            let h1 = self.h[1];
            let declared = word(h1, "var") || (is(h1, ",") && self.var_depth.is_some());
            if declared {
                if self.in_catch(name.text) {
                    self.declare_in_catch(name.text);
                }
                self.keep_outer(name.text);
                self.make_own(name.text);
            }
            // A name after `.`, `..`, `.@`, `::` or `@` is a property's or an attribute's; any other
            // name before `=` is a variable, at the start of a statement or inside an expression.
            if !member_mark(h1) && !is(h1, "@") {
                self.assign = Some(Assign { name: name.text.to_string(), after_new: false });
            }
        }
        match self.updated(t) {
            // The value on the right is read at this depth, and the name holds nothing the check
            // follows once that value ends ([`Analyzer::settle_groups`]).
            Some((name, true)) => self.start_value(name.to_string(), Operand::Other),
            Some((name, false)) => self.unbind(name),
            None => {}
        }
    }

    /// The variable that `t` changes to a number or a string, nothing the check follows, and whether
    /// the change waits for the value on the right. `name += …` and the other compound assignments
    /// (ECMA-262 3rd edition, section 11.13.2) store their result after that value is read, so the
    /// name holds what it held until the value ends. `name++` and `name--` (section 11.3), read at
    /// their operator, and `++name` and `--name` (section 11.4), read at the token after the name when
    /// no member, index or call follows it, change the name at once. A `++` or `--` is the name's
    /// after it when it is on the name's line, and before it when no operand ends just before the
    /// `++` on that line. A property changed so (`o.name += 1`, `++o.name`) leaves the variable as it
    /// is.
    fn updated(&self, t: &Sig<'s>) -> Option<(&'s str, bool)> {
        let [h0, h1, h2, _] = self.h;
        let name = h0.filter(|s| s.kind == T::Ident)?;
        let compound = t.kind == T::Punct && COMPOUND.contains(&t.text);
        let postfix = (t.is("++") || t.is("--")) && !t.nl;
        if (compound || postfix) && !expr_after(T::Ident, name.text) && !member_mark(h1) && !is(h1, "@") {
            return Some((name.text, compound));
        }
        let prefix = h1.is_some_and(|op| (op.is("++") || op.is("--")) && (op.nl || h2.is_none_or(|s| expr_after(s.kind, s.text))));
        (prefix && !member_mark(Some(*t)) && !t.is("[") && !t.is("(")).then_some((name.text, false))
    }

    // ---------- values in grouping parentheses, and values that may hold a File ----------

    /// `name = (`, its `(` just read: the value is read inside the parentheses.
    fn start_group(&mut self, name: String) {
        let level = self.depth();
        self.push_group(Group { id: 0, name, base: level.saturating_sub(1), level, operand: Operand::Start, closed: false, bare: false, conds: 0 });
    }

    /// `name =` and the first token of a value without parentheses that may hold a File (`File`,
    /// `Folder`, or a variable holding a File), read as far as `operand`, or a compound assignment's
    /// operator (`name += …`), with [`Operand::Other`]: the value is read at this depth.
    fn start_value(&mut self, name: String, operand: Operand) {
        let level = self.depth();
        self.push_group(Group { id: 0, name, base: level, level, operand, closed: false, bare: true, conds: 0 });
    }

    /// Read the value `g` from the next token on. Past [`crate::Limits::nesting`] values open at
    /// once, its name holds nothing the check follows.
    fn push_group(&mut self, mut g: Group) {
        if self.groups.len() >= self.acc.limits.nesting {
            self.unbind(&g.name);
            return;
        }
        self.group_id = self.group_id.wrapping_add(1);
        g.id = self.group_id;
        g.conds = self.conds.len();
        self.groups.push(g);
    }

    /// `t` in the innermost value still open: in its innermost pair of parentheses, or, for a value
    /// without parentheses, at the depth it started at. `before` is the bracket depth before `t`, and
    /// `started` is true when `t` started a member chain.
    fn group_step(&mut self, t: &Sig<'s>, started: bool, before: usize) {
        let after = self.depth();
        let file = (t.word(catalog::FILE) || t.word(catalog::FOLDER)) && !self.own_names.contains(t.text);
        let alias = t.kind == T::Ident && self.files.contains(t.text);
        let Some(g) = self.groups.last_mut().filter(|g| !g.closed) else { return };
        if after < g.level {
            // The `)` of the innermost pair: the operand, when complete, is the pair's value and the
            // operand of the pair around it.
            let held = match std::mem::replace(&mut g.operand, Operand::Other) {
                Operand::Done(held) => Some(held),
                Operand::FileName { new: true, .. } => Some(Held::File),
                _ => None,
            };
            g.operand = held.map_or(Operand::Other, Operand::Done);
            g.level = after;
            g.closed = after <= g.base;
            return;
        }
        if before > g.level {
            // Inside a bracket of the operand: the `)` back at the pair's depth ends a call of `File`,
            // `Folder` or one of their dialogs.
            if after == g.level && matches!(g.operand, Operand::FileCall) {
                g.operand = Operand::Done(Held::File);
            }
            return;
        }
        if t.is(";") {
            // A `;` inside the parentheses ends the value: the name holds nothing the check follows.
            if let Some(g) = self.groups.pop() {
                self.unbind(&g.name);
            }
            return;
        }
        let next = match std::mem::replace(&mut g.operand, Operand::Other) {
            _ if t.is(",") => Operand::Start,
            Operand::Start if t.is("(") => {
                g.level = after;
                Operand::Start
            }
            Operand::Start if started => {
                if let Some(c) = self.active.as_mut() {
                    c.group = Some(g.id);
                }
                Operand::Chain
            }
            Operand::Chain => Operand::Chain,
            operand => operand.file_step(t, file, alias),
        };
        g.operand = next;
    }

    /// `t` in the innermost grouping parentheses being read ([`Paren`]): at their depth, or the `)` that
    /// brings a call or a pair of brackets in them back to it. A `(` where an expression may start
    /// opens grouping parentheses, up to [`crate::Limits::nesting`] of them, except after a `.`, where
    /// it opens an E4X filter (ECMA-357 2nd edition, section 11.2.4); `before` is the bracket depth
    /// before `t`.
    fn paren_step(&mut self, t: &Sig<'s>, before: usize) {
        let after = self.depth();
        let file = (t.word(catalog::FILE) || t.word(catalog::FOLDER)) && !self.own_names.contains(t.text);
        let alias = t.kind == T::Ident && self.files.contains(t.text);
        if let Some(p) = self.parens.last_mut() {
            if before == p.level {
                p.operand = match std::mem::replace(&mut p.operand, Operand::Other) {
                    _ if t.is(",") => Operand::Start,
                    Operand::Start if t.is("(") => Operand::Pair,
                    operand => operand.file_step(t, file, alias),
                };
            } else if after == p.level && before > after {
                // The `)` of a call of `File`, `Folder` or a dialog, or of grouping parentheses, ends a
                // File or Folder object when [`Analyzer::bracket`] set its [`Sig::file`].
                p.operand = match p.operand {
                    Operand::FileCall | Operand::Pair if t.file => Operand::Done(Held::File),
                    _ => Operand::Other,
                };
            }
        }
        if t.is("(") && t.expr && !member_mark(self.h[0]) && self.parens.len() < self.acc.limits.nesting {
            self.parens.push(Paren { level: after, operand: Operand::Start });
        }
    }

    /// The grouping parentheses that the bracket `t` closed, if any (a bracket closes one level): true
    /// when `t` is their `)` and their operand is a File or Folder object.
    fn close_parens(&mut self, t: &Sig<'s>) -> bool {
        let depth = self.depth();
        self.parens.pop_if(|p| depth < p.level).is_some_and(|p| t.is(")") && p.operand.holds_file())
    }

    /// `t` follows the last `)` of a grouped value, or ends a value without parentheses at the depth
    /// it started at: the name holds the value when `t` ends it.
    fn settle_groups(&mut self, t: &Sig<'s>) {
        // A line break ends a value only when the token before it doesn't go on with the expression
        // either (`File.` before `openDialog(…)` on the next line).
        let ends = ends_value(t) && !(t.nl && goes_on_after(self.h[0]));
        let depth = self.depth();
        // A `:` that answers a `?` read inside a value without parentheses goes on with that value
        // (`f += ok ? '' : f.remove()`), and any other `:` ends it (`ok ? f = new File(p) : 0`).
        let open = if t.is(":") && self.conds.last() == Some(&depth) { self.conds.len() } else { 0 };
        let done = |g: &mut Group| g.closed || (g.bare && ends && g.level == depth && open <= g.conds);
        while let Some(g) = self.groups.pop_if(done) {
            self.settle(g, ends);
        }
    }

    /// The name of the value `g` holds it when the value is complete (a grouped value's last `)` was
    /// read, or a value without parentheses ended) and `ends` (the value ends after it), and
    /// otherwise nothing the check follows.
    fn settle(&mut self, g: Group, ends: bool) {
        let complete = (g.closed || g.bare) && ends;
        match g.operand {
            Operand::Done(Held::Chain(path)) if complete => self.bind(g.name, path),
            Operand::Done(Held::File) | Operand::FileName { new: true, .. } if complete => self.hold_file(g.name),
            _ => self.unbind(&g.name),
        }
    }

    /// The statement ends (a preprocessor line, or the end of the file), and with it every value still
    /// being read: the name of a grouped value whose last `)` was read, or of a value without
    /// parentheses whose operand is complete, holds the value, and any other name holds nothing the
    /// check follows.
    fn end_groups(&mut self) {
        while let Some(g) = self.groups.pop() {
            self.acc.steps = self.acc.steps.saturating_add(1);
            self.settle(g, true);
        }
    }

    /// The conditional expressions being read after `t`: a `?` opens one at this depth, and the `:`
    /// at its depth, a closing bracket around it or a `;` at its depth ends it.
    fn conditional(&mut self, t: &Sig<'s>) {
        let depth = self.depth();
        let end = if t.is(";") { depth } else { depth.saturating_add(1) };
        while self.conds.pop_if(|d| *d >= end).is_some() {}
        if t.is("?") && self.conds.len() < self.acc.limits.nesting {
            self.conds.push(depth);
        } else if t.is(":") && self.conds.last() == Some(&depth) {
            self.conds.pop();
        }
    }

    // ---------- preprocessor lines ----------

    fn directive(&mut self, t: &Token<'s>) {
        // A preprocessor line ends the statement before it.
        self.end_active(None);
        self.end_groups();
        self.settle_pending();
        self.assign = None;
        self.var_depth = None;
        self.conds.clear();
        self.h = [None; 4];
        self.acc.steps = self.acc.steps.saturating_add(1);
        let d = directives::parse(t.text);
        let Some(name) = d.known else {
            let detail = if d.name.is_empty() { t.text.trim().to_string() } else { format!("{}{}", d.form, d.name) };
            self.acc.problem(report::UNKNOWN_DIRECTIVE, Some(t.line), self.file, Some(&detail));
            return;
        };
        let feature = match name {
            "target" => FeatureId::DirectiveTarget,
            "include" | "includepath" => FeatureId::DirectiveInclude,
            "targetengine" => FeatureId::DirectiveTargetengine,
            _ => FeatureId::DirectiveOther,
        };
        let line = self.agg(t.line);
        self.acc.feature(feature, line);
        self.acc.directive(name, d.value, t.line, d.form, self.file);
        match name {
            "target" => self.acc.target(d.value, t.line, self.file),
            "include" if self.includes.len() < self.acc.limits.entries => {
                self.includes.push(IncludeRef { value: d.value.to_string(), line: t.line, paths: Rc::clone(&self.paths) });
            }
            "includepath" => self.paths = directives::include_paths(d.value).into(),
            _ => {}
        }
    }

    /// At the end of the file: the chains still open, what was pending, and the brackets never
    /// closed → the `#include` lines read, whether the file is a module, and what its variables
    /// hold.
    fn finish(mut self) -> (Vec<IncludeRef>, bool, Known) {
        self.end_active(None);
        while let Some(c) = self.suspended.pop() {
            self.acc.steps = self.acc.steps.saturating_add(1);
            self.record(c, None);
        }
        self.end_groups();
        self.settle_pending();
        let module = self.module || self.module_candidate;
        for (bracket, line) in std::mem::take(&mut self.brackets.open) {
            self.acc.steps = self.acc.steps.saturating_add(1);
            let mut buf = [0; 4];
            let bracket: &str = bracket.encode_utf8(&mut buf);
            self.acc.problem(report::UNCLOSED_BRACKET, Some(line), self.file, Some(bracket));
        }
        let known = Known { bound: std::mem::take(&mut self.bound), files: std::mem::take(&mut self.files) };
        (std::mem::take(&mut self.includes), module, known)
    }
}
