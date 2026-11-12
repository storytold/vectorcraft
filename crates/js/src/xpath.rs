//! `xpath` on XML and XMLList values: XPath 1.0 expressions
//! (<https://www.w3.org/TR/1999/REC-xpath-19991116/>) evaluated on E4X trees.
//!
//! An expression is read into tokens, then into a syntax tree whose nodes live in one arena, and is
//! evaluated with an explicit task stack, so neither step uses native recursion. Brackets nest at
//! most [`crate::Options::max_depth`] deep. Every token, task and visited node takes fuel, and the
//! token, tree, node-set and string buffers are held against the heap limit while they live.
//!
//! `/` is a root node above the topmost node of the context node's tree. A node-set result is
//! returned as an XMLList of the selected nodes themselves, in document order with attributes
//! before children. The root node and namespace nodes can be selected and tested inside an
//! expression, but a result that holds one throws a `TypeError`, since E4X has no such value. The
//! tree keeps no DTD, so `id()` selects nothing. Prefixes in the expression are read from the
//! namespaces in scope on the context element, and `xml` is always bound.

use std::collections::{HashMap, HashSet};
use std::mem::replace;
use std::rc::Rc;

use crate::builtins::arg;
use crate::error::TOO_DEEP;
use crate::heap::{Key, ObjId, XmlKind, XmlName, XmlNs, key};
use crate::interp::{Abrupt, ErrorType, HeldBytes, Interp, JsResult};
use crate::regexp_vm::use_fuel_n;
use crate::string_builtins::{self, charge_units, flat_charged, push_held};
use crate::value::Value;
use crate::xml::{Xv, is_name_part, is_name_start, is_xml_space, valid_xml_units};
use crate::xml_builtins::scope_of;

const XML_URI: &str = "http://www.w3.org/XML/1998/namespace";
const COLON: u16 = b':' as u16;
const STAR: u16 = b'*' as u16;
const SLASH: u16 = b'/' as u16;
const DOT: u16 = b'.' as u16;
const EQUALS: u16 = b'=' as u16;
const DASH: u16 = b'-' as u16;
/// The estimated bytes of a syntax tree node's own lists, beyond its slot in the arena.
const EXPR_BYTES: usize = 64;
/// The estimated bytes of one entry of a hash map or set the evaluator keeps.
const ENTRY_BYTES: usize = 64;

fn empty() -> Key {
    Rc::from(Vec::new())
}

fn syntax(interp: &mut Interp, message: &str) -> Abrupt {
    interp.throw(ErrorType::Syntax, message)
}

fn internal() -> Abrupt {
    Abrupt::Internal("XPath evaluation state".into())
}

fn is_digit(c: u16) -> bool {
    (0x30..=0x39).contains(&c)
}

// ---- tokens ----

#[derive(Clone, Copy, PartialEq)]
enum Punct {
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dot,
    DotDot,
    At,
    Comma,
    ColonColon,
    Slash,
    SlashSlash,
    Pipe,
    Plus,
    Minus,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A token as the lexer reads it, before names and `*` are told apart by their position.
enum Raw {
    Punct(Punct),
    Star,
    Name(Option<Key>, Key),
    PrefixStar(Key),
    Literal(Key),
    Number(f64),
    Var(Key),
}

#[derive(Clone, Copy, PartialEq)]
enum Axis {
    Ancestor,
    AncestorOrSelf,
    Attribute,
    Child,
    Descendant,
    DescendantOrSelf,
    Following,
    FollowingSibling,
    Namespace,
    Parent,
    Preceding,
    PrecedingSibling,
    Itself,
}

const AXES: [(&str, Axis); 13] = [
    ("ancestor", Axis::Ancestor),
    ("ancestor-or-self", Axis::AncestorOrSelf),
    ("attribute", Axis::Attribute),
    ("child", Axis::Child),
    ("descendant", Axis::Descendant),
    ("descendant-or-self", Axis::DescendantOrSelf),
    ("following", Axis::Following),
    ("following-sibling", Axis::FollowingSibling),
    ("namespace", Axis::Namespace),
    ("parent", Axis::Parent),
    ("preceding", Axis::Preceding),
    ("preceding-sibling", Axis::PrecedingSibling),
    ("self", Axis::Itself),
];

impl Axis {
    /// Whether the axis lists its nodes in reverse document order.
    fn reverse(self) -> bool {
        matches!(self, Axis::Ancestor | Axis::AncestorOrSelf | Axis::Preceding | Axis::PrecedingSibling)
    }
}

/// A node test. A prefix is resolved when the expression runs.
#[derive(Clone)]
enum Test {
    Any,
    Prefix(Key),
    Name(Option<Key>, Key),
    Node,
    Text,
    Comment,
    Pi(Option<Key>),
}

#[derive(Clone, Copy, PartialEq)]
enum Bin {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Union,
}

impl Bin {
    fn precedence(self) -> u8 {
        match self {
            Bin::Or => 1,
            Bin::And => 2,
            Bin::Eq | Bin::Ne => 3,
            Bin::Lt | Bin::Le | Bin::Gt | Bin::Ge => 4,
            Bin::Add | Bin::Sub => 5,
            Bin::Mul | Bin::Div | Bin::Mod => 6,
            Bin::Union => 8,
        }
    }
}

/// The precedence of unary minus: tighter than `*`, looser than `|`.
const NEG_PRECEDENCE: u8 = 7;

#[derive(Clone, Copy, PartialEq)]
enum Func {
    Last,
    Position,
    Count,
    Id,
    LocalName,
    NamespaceUri,
    Name,
    String,
    Concat,
    StartsWith,
    Contains,
    SubstringBefore,
    SubstringAfter,
    Substring,
    StringLength,
    NormalizeSpace,
    Translate,
    Boolean,
    Not,
    True,
    False,
    Lang,
    Number,
    Sum,
    Floor,
    Ceiling,
    Round,
}

/// The core function library (XPath 1.0 section 4): each name, and its least and most arguments.
const FUNCS: [(&str, Func, usize, usize); 27] = [
    ("last", Func::Last, 0, 0),
    ("position", Func::Position, 0, 0),
    ("count", Func::Count, 1, 1),
    ("id", Func::Id, 1, 1),
    ("local-name", Func::LocalName, 0, 1),
    ("namespace-uri", Func::NamespaceUri, 0, 1),
    ("name", Func::Name, 0, 1),
    ("string", Func::String, 0, 1),
    ("concat", Func::Concat, 2, usize::MAX),
    ("starts-with", Func::StartsWith, 2, 2),
    ("contains", Func::Contains, 2, 2),
    ("substring-before", Func::SubstringBefore, 2, 2),
    ("substring-after", Func::SubstringAfter, 2, 2),
    ("substring", Func::Substring, 2, 3),
    ("string-length", Func::StringLength, 0, 1),
    ("normalize-space", Func::NormalizeSpace, 0, 1),
    ("translate", Func::Translate, 3, 3),
    ("boolean", Func::Boolean, 1, 1),
    ("not", Func::Not, 1, 1),
    ("true", Func::True, 0, 0),
    ("false", Func::False, 0, 0),
    ("lang", Func::Lang, 1, 1),
    ("number", Func::Number, 0, 1),
    ("sum", Func::Sum, 1, 1),
    ("floor", Func::Floor, 1, 1),
    ("ceiling", Func::Ceiling, 1, 1),
    ("round", Func::Round, 1, 1),
];

/// A token the parser reads: names and `*` with their roles decided (XPath 1.0 section 3.7).
enum Tok {
    Punct(Punct),
    Op(Bin),
    Test(Test),
    Axis(Axis),
    Func(Func, &'static str),
    Literal(Key),
    Number(f64),
    Var(Key),
}

fn ncname_end(src: &[u16], at: usize) -> Option<usize> {
    let first = *src.get(at)?;
    if first == COLON || !is_name_start(first) {
        return None;
    }
    let mut end = at + 1;
    while src.get(end).is_some_and(|&c| c != COLON && is_name_part(c)) {
        end += 1;
    }
    valid_xml_units(src.get(at..end)?).then_some(end)
}

fn lex_name(src: &[u16], at: usize) -> Option<(Raw, usize)> {
    let end = ncname_end(src, at)?;
    let first: Key = Rc::from(src.get(at..end)?);
    if src.get(end) != Some(&COLON) || src.get(end + 1) == Some(&COLON) {
        return Some((Raw::Name(None, first), end));
    }
    if src.get(end + 1) == Some(&STAR) {
        return Some((Raw::PrefixStar(first), end + 2));
    }
    let local_end = ncname_end(src, end + 1)?;
    Some((Raw::Name(Some(first), Rc::from(src.get(end + 1..local_end)?)), local_end))
}

fn lex_number(src: &[u16], at: usize) -> Option<(Raw, usize)> {
    let mut end = at;
    while src.get(end).copied().is_some_and(is_digit) {
        end += 1;
    }
    if src.get(end) == Some(&DOT) {
        end += 1;
        while src.get(end).copied().is_some_and(is_digit) {
            end += 1;
        }
    }
    let text = String::from_utf16(src.get(at..end)?).ok()?;
    Some((Raw::Number(text.parse().ok()?), end))
}

fn lex_literal(src: &[u16], at: usize, quote: u16) -> Option<(Raw, usize)> {
    let rest = src.get(at + 1..)?;
    let len = rest.iter().position(|&c| c == quote)?;
    Some((Raw::Literal(Rc::from(rest.get(..len)?)), at + len + 2))
}

fn lex_var(src: &[u16], at: usize) -> Option<(Raw, usize)> {
    let (Raw::Name(..), end) = lex_name(src, at + 1)? else { return None };
    Some((Raw::Var(Rc::from(src.get(at + 1..end)?)), end))
}

/// The token that starts at `src[at]`, and the offset after it.
fn lex_one(src: &[u16], at: usize, c: u16) -> Option<(Raw, usize)> {
    if c != COLON && is_name_start(c) {
        return lex_name(src, at);
    }
    let next = src.get(at + 1).copied();
    let p = |p: Punct, n: usize| Some((Raw::Punct(p), at + n));
    match char::from_u32(u32::from(c))? {
        '(' => p(Punct::LParen, 1),
        ')' => p(Punct::RParen, 1),
        '[' => p(Punct::LBracket, 1),
        ']' => p(Punct::RBracket, 1),
        '@' => p(Punct::At, 1),
        ',' => p(Punct::Comma, 1),
        '|' => p(Punct::Pipe, 1),
        '+' => p(Punct::Plus, 1),
        '-' => p(Punct::Minus, 1),
        '=' => p(Punct::Eq, 1),
        '*' => Some((Raw::Star, at + 1)),
        '/' if next == Some(SLASH) => p(Punct::SlashSlash, 2),
        '/' => p(Punct::Slash, 1),
        ':' if next == Some(COLON) => p(Punct::ColonColon, 2),
        '!' if next == Some(EQUALS) => p(Punct::Ne, 2),
        '<' if next == Some(EQUALS) => p(Punct::Le, 2),
        '<' => p(Punct::Lt, 1),
        '>' if next == Some(EQUALS) => p(Punct::Ge, 2),
        '>' => p(Punct::Gt, 1),
        '.' if next == Some(DOT) => p(Punct::DotDot, 2),
        '.' if next.is_some_and(is_digit) => lex_number(src, at),
        '.' => p(Punct::Dot, 1),
        '"' | '\'' => lex_literal(src, at, c),
        '$' => lex_var(src, at),
        _ if is_digit(c) => lex_number(src, at),
        _ => None,
    }
}

fn lex(interp: &mut Interp, held: &mut HeldBytes, src: &[u16]) -> JsResult<Vec<Raw>> {
    let mut out = Vec::new();
    let mut at = 0;
    loop {
        while src.get(at).copied().is_some_and(is_xml_space) {
            at += 1;
        }
        let Some(&c) = src.get(at) else { return Ok(out) };
        let Some((raw, end)) = lex_one(src, at, c) else {
            return Err(syntax(interp, &format!("the XPath expression cannot be read at offset {at}")));
        };
        charge_units(interp, end - at)?;
        push_held(interp, held, &mut out, raw)?;
        at = end;
    }
}

fn units_are(units: &[u16], text: &str) -> bool {
    units.iter().copied().eq(text.encode_utf16())
}

fn operator_name(name: &[u16]) -> Option<Bin> {
    [("and", Bin::And), ("or", Bin::Or), ("mod", Bin::Mod), ("div", Bin::Div)].into_iter().find(|(n, _)| units_are(name, n)).map(|(_, b)| b)
}

fn punct_token(p: Punct) -> Tok {
    match p {
        Punct::Pipe => Tok::Op(Bin::Union),
        Punct::Plus => Tok::Op(Bin::Add),
        Punct::Minus => Tok::Op(Bin::Sub),
        Punct::Eq => Tok::Op(Bin::Eq),
        Punct::Ne => Tok::Op(Bin::Ne),
        Punct::Lt => Tok::Op(Bin::Lt),
        Punct::Le => Tok::Op(Bin::Le),
        Punct::Gt => Tok::Op(Bin::Gt),
        Punct::Ge => Tok::Op(Bin::Ge),
        other => Tok::Punct(other),
    }
}

/// Whether a token at this point is an operator: there is a token before it, and it is not `@`,
/// `::`, `(`, `[`, `,` or an operator (XPath 1.0 section 3.7).
fn operator_position(last: Option<&Tok>) -> bool {
    last.is_some_and(|t| {
        !matches!(
            t,
            Tok::Punct(Punct::LParen | Punct::LBracket | Punct::At | Punct::Comma | Punct::Slash | Punct::SlashSlash)
                | Tok::Op(_)
                | Tok::Axis(_)
                | Tok::Func(..)
        )
    })
}

/// A name followed by `(`: a node type test, which also takes its `)`, or a function call.
/// Returns the token and how many raw tokens it takes.
fn call_or_node_type(interp: &mut Interp, prefix: Option<&Key>, name: &Key, after: &[Raw]) -> JsResult<(Tok, usize)> {
    if prefix.is_none() {
        let plain = [("node", Test::Node), ("text", Test::Text), ("comment", Test::Comment)].into_iter().find(|(n, _)| units_are(name, n));
        if let Some((_, test)) = plain {
            return match after.first() {
                Some(Raw::Punct(Punct::RParen)) => Ok((Tok::Test(test), 3)),
                _ => Err(syntax(interp, "an XPath node type test takes no argument")),
            };
        }
        if units_are(name, "processing-instruction") {
            return match after {
                [Raw::Punct(Punct::RParen), ..] => Ok((Tok::Test(Test::Pi(None)), 3)),
                [Raw::Literal(target), Raw::Punct(Punct::RParen), ..] => Ok((Tok::Test(Test::Pi(Some(target.clone()))), 4)),
                _ => Err(syntax(interp, "processing-instruction() takes one string literal or none")),
            };
        }
    }
    let found = FUNCS.iter().find(|(n, ..)| prefix.is_none() && units_are(name, n));
    match found {
        Some(&(n, f, ..)) => Ok((Tok::Func(f, n), 2)),
        None => {
            let shown = String::from_utf16_lossy(name);
            Err(interp.type_error(&format!("{shown} is not an XPath 1.0 function")))
        }
    }
}

fn classify(interp: &mut Interp, held: &mut HeldBytes, raw: &[Raw]) -> JsResult<Vec<Tok>> {
    let mut out: Vec<Tok> = Vec::new();
    let mut at = 0;
    while let Some(r) = raw.get(at) {
        interp.use_fuel()?;
        let operator = operator_position(out.last());
        let next = raw.get(at + 1);
        let (tok, used) = match r {
            Raw::Star if operator => (Tok::Op(Bin::Mul), 1),
            Raw::Star => (Tok::Test(Test::Any), 1),
            Raw::Name(None, n) if operator => match operator_name(n) {
                Some(op) => (Tok::Op(op), 1),
                None => return Err(syntax(interp, "an XPath operator is missing")),
            },
            Raw::Name(p, n) if matches!(next, Some(Raw::Punct(Punct::LParen))) => {
                call_or_node_type(interp, p.as_ref(), n, raw.get(at + 2..).unwrap_or_default())?
            }
            Raw::Name(p, n) if matches!(next, Some(Raw::Punct(Punct::ColonColon))) => {
                let axis = AXES.iter().find(|(a, _)| p.is_none() && units_are(n, a)).map(|&(_, a)| a);
                match axis {
                    Some(a) => (Tok::Axis(a), 2),
                    None => return Err(syntax(interp, &format!("{} is not an XPath axis", String::from_utf16_lossy(n)))),
                }
            }
            Raw::Name(p, n) => (Tok::Test(Test::Name(p.clone(), n.clone())), 1),
            Raw::PrefixStar(p) => (Tok::Test(Test::Prefix(p.clone())), 1),
            Raw::Punct(Punct::ColonColon) => return Err(syntax(interp, "an XPath axis name is missing before ::")),
            Raw::Punct(p) => (punct_token(*p), 1),
            Raw::Literal(s) => (Tok::Literal(s.clone()), 1),
            Raw::Number(n) => (Tok::Number(*n), 1),
            Raw::Var(v) => (Tok::Var(v.clone()), 1),
        };
        push_held(interp, held, &mut out, tok)?;
        at += used;
    }
    Ok(out)
}

// ---- the syntax tree ----

struct Step {
    axis: Axis,
    test: Test,
    preds: Vec<usize>,
}

enum Expr {
    Bin(Bin, usize, usize),
    Neg(usize),
    Num(f64),
    Lit(Key),
    Var(usize),
    Call(Func, Vec<usize>),
    /// A location path, or a filter expression with its predicates, followed by steps.
    Path {
        filter: Option<(usize, Vec<usize>)>,
        absolute: bool,
        steps: Vec<Step>,
    },
}

/// A parsed expression: the tree in its arena, its root, and the variables and prefixes it names.
struct Parsed {
    exprs: Vec<Expr>,
    root: usize,
    vars: Vec<Key>,
    prefixes: Vec<Key>,
    _held: HeldBytes,
}

#[derive(Clone, Copy)]
enum Op {
    Bin(Bin),
    Neg,
}

impl Op {
    fn precedence(self) -> u8 {
        match self {
            Op::Bin(b) => b.precedence(),
            Op::Neg => NEG_PRECEDENCE,
        }
    }
}

/// A path expression being read.
#[derive(Default)]
struct Operand {
    filter: Option<(usize, Vec<usize>)>,
    absolute: bool,
    steps: Vec<Step>,
    /// The last step is `.` or `..`, which take no predicate.
    abbreviated: bool,
}

impl Operand {
    fn bare_root(&self) -> bool {
        self.absolute && self.steps.is_empty() && self.filter.is_none()
    }
}

enum FrameKind {
    Top,
    Group,
    Pred,
    Args(Func, &'static str, Vec<usize>),
}

/// One bracket level: its operators and operands, read by operator precedence.
struct Frame {
    kind: FrameKind,
    ops: Vec<Op>,
    vals: Vec<usize>,
    operand: Option<Operand>,
}

impl Frame {
    fn new(kind: FrameKind) -> Self {
        Frame { kind, ops: Vec::new(), vals: Vec::new(), operand: None }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Want {
    Operand,
    Step,
    Test(Axis),
    After,
}

struct Parser<'i> {
    interp: &'i mut Interp,
    held: HeldBytes,
    exprs: Vec<Expr>,
    vars: Vec<Key>,
    var_index: HashMap<Key, usize>,
    prefixes: HashSet<Key>,
    frames: Vec<Frame>,
    want: Want,
}

impl Parser<'_> {
    fn error(&mut self, message: &str) -> Abrupt {
        syntax(self.interp, message)
    }

    fn push_expr(&mut self, e: Expr) -> JsResult<usize> {
        self.held.add(self.interp, EXPR_BYTES)?;
        push_held(self.interp, &mut self.held, &mut self.exprs, e)?;
        Ok(self.exprs.len() - 1)
    }

    fn top(&mut self) -> JsResult<&mut Frame> {
        self.frames.last_mut().ok_or_else(internal)
    }

    fn operand(&mut self) -> JsResult<&mut Operand> {
        self.top()?.operand.as_mut().ok_or_else(internal)
    }

    fn open(&mut self, kind: FrameKind) -> JsResult<()> {
        if self.frames.len() >= self.interp.options.max_depth {
            return Err(self.error(TOO_DEEP));
        }
        self.held.add(self.interp, size_of::<Frame>())?;
        self.frames.push(Frame::new(kind));
        self.want = Want::Operand;
        Ok(())
    }

    fn token(&mut self, tok: Tok) -> JsResult<()> {
        match self.want {
            Want::Operand => self.operand_token(tok),
            Want::Step => self.step_token(tok),
            Want::Test(axis) => match tok {
                Tok::Test(test) => self.push_step(axis, test, false),
                _ => Err(self.error("an XPath node test is missing")),
            },
            Want::After => self.after_token(tok),
        }
    }

    fn primary(&mut self, e: Expr) -> JsResult<()> {
        let id = self.push_expr(e)?;
        self.top()?.operand = Some(Operand { filter: Some((id, Vec::new())), ..Operand::default() });
        self.want = Want::After;
        Ok(())
    }

    fn operand_token(&mut self, tok: Tok) -> JsResult<()> {
        match tok {
            Tok::Op(Bin::Sub) => {
                self.top()?.ops.push(Op::Neg);
                Ok(())
            }
            Tok::Punct(Punct::Slash) => {
                self.top()?.operand = Some(Operand { absolute: true, ..Operand::default() });
                self.want = Want::Step;
                Ok(())
            }
            Tok::Punct(Punct::SlashSlash) => {
                self.top()?.operand = Some(Operand { absolute: true, ..Operand::default() });
                self.descendants_step()
            }
            Tok::Literal(s) => self.primary(Expr::Lit(s)),
            Tok::Number(n) => self.primary(Expr::Num(n)),
            Tok::Var(name) => {
                let next = self.vars.len();
                let at = *self.var_index.entry(name.clone()).or_insert(next);
                if at == next {
                    push_held(self.interp, &mut self.held, &mut self.vars, name)?;
                }
                self.primary(Expr::Var(at))
            }
            Tok::Func(f, n) => self.open(FrameKind::Args(f, n, Vec::new())),
            Tok::Punct(Punct::LParen) => self.open(FrameKind::Group),
            Tok::Punct(Punct::RParen) if self.empty_call()? => self.close_paren(),
            tok @ (Tok::Test(_) | Tok::Axis(_) | Tok::Punct(Punct::At | Punct::Dot | Punct::DotDot)) => {
                self.top()?.operand = Some(Operand::default());
                self.step_token(tok)
            }
            _ => Err(self.error("an XPath operand is missing")),
        }
    }

    /// Whether the open bracket is a function call's, with nothing in it yet.
    fn empty_call(&mut self) -> JsResult<bool> {
        let f = self.top()?;
        Ok(matches!(&f.kind, FrameKind::Args(_, _, args) if args.is_empty()) && f.ops.is_empty() && f.vals.is_empty() && f.operand.is_none())
    }

    fn descendants_step(&mut self) -> JsResult<()> {
        self.operand()?.steps.push(Step { axis: Axis::DescendantOrSelf, test: Test::Node, preds: Vec::new() });
        self.want = Want::Step;
        Ok(())
    }

    fn push_step(&mut self, axis: Axis, test: Test, abbreviated: bool) -> JsResult<()> {
        if let Test::Prefix(p) | Test::Name(Some(p), _) = &test
            && !self.prefixes.contains(p)
        {
            self.held.add(self.interp, ENTRY_BYTES)?;
            self.prefixes.insert(p.clone());
        }
        self.held.add(self.interp, size_of::<Step>())?;
        let operand = self.operand()?;
        operand.steps.push(Step { axis, test, preds: Vec::new() });
        operand.abbreviated = abbreviated;
        self.want = Want::After;
        Ok(())
    }

    fn step_token(&mut self, tok: Tok) -> JsResult<()> {
        match tok {
            Tok::Punct(Punct::Dot) => self.push_step(Axis::Itself, Test::Node, true),
            Tok::Punct(Punct::DotDot) => self.push_step(Axis::Parent, Test::Node, true),
            Tok::Punct(Punct::At) => {
                self.want = Want::Test(Axis::Attribute);
                Ok(())
            }
            Tok::Axis(a) => {
                self.want = Want::Test(a);
                Ok(())
            }
            Tok::Test(t) => self.push_step(Axis::Child, t, false),
            tok if self.operand()?.bare_root() => {
                self.want = Want::After;
                self.after_token(tok)
            }
            _ => Err(self.error("an XPath step is missing")),
        }
    }

    fn after_token(&mut self, tok: Tok) -> JsResult<()> {
        match tok {
            Tok::Punct(p @ (Punct::LBracket | Punct::Slash | Punct::SlashSlash)) => {
                let operand = self.operand()?;
                let (bare, abbreviated) = (operand.bare_root(), operand.abbreviated);
                if bare || (p == Punct::LBracket && abbreviated) {
                    return Err(self.error("an XPath predicate or step cannot follow here"));
                }
                match p {
                    Punct::LBracket => self.open(FrameKind::Pred),
                    Punct::Slash => {
                        self.want = Want::Step;
                        Ok(())
                    }
                    _ => self.descendants_step(),
                }
            }
            Tok::Op(b) => self.binary(b),
            Tok::Punct(Punct::RParen) => self.close_paren(),
            Tok::Punct(Punct::RBracket) => self.close_bracket(),
            Tok::Punct(Punct::Comma) => self.next_argument(),
            _ => Err(self.error("an XPath operator is missing")),
        }
    }

    fn finish_operand(&mut self, operand: Operand) -> JsResult<usize> {
        match operand {
            Operand { filter: Some((e, preds)), steps, .. } if preds.is_empty() && steps.is_empty() => Ok(e),
            Operand { filter, absolute, steps, .. } => self.push_expr(Expr::Path { filter, absolute, steps }),
        }
    }

    fn reduce_one(&mut self, frame: &mut Frame) -> JsResult<()> {
        let op = frame.ops.pop().ok_or_else(internal)?;
        let rhs = frame.vals.pop().ok_or_else(internal)?;
        let e = match op {
            Op::Neg => Expr::Neg(rhs),
            Op::Bin(b) => Expr::Bin(b, frame.vals.pop().ok_or_else(internal)?, rhs),
        };
        let id = self.push_expr(e)?;
        frame.vals.push(id);
        Ok(())
    }

    fn binary(&mut self, b: Bin) -> JsResult<()> {
        let mut frame = self.frames.pop().ok_or_else(internal)?;
        let result = self.binary_in(&mut frame, b);
        self.frames.push(frame);
        result?;
        self.want = Want::Operand;
        Ok(())
    }

    fn binary_in(&mut self, frame: &mut Frame, b: Bin) -> JsResult<()> {
        let operand = frame.operand.take().ok_or_else(internal)?;
        let e = self.finish_operand(operand)?;
        frame.vals.push(e);
        while frame.ops.last().is_some_and(|op| op.precedence() >= b.precedence()) {
            self.interp.use_fuel()?;
            self.reduce_one(frame)?;
        }
        self.held.add(self.interp, 2 * size_of::<usize>())?;
        frame.ops.push(Op::Bin(b));
        Ok(())
    }

    /// Ends the innermost bracket and returns its kind and expression.
    fn close(&mut self) -> JsResult<(FrameKind, usize)> {
        if self.frames.len() < 2 {
            return Err(self.error("the XPath expression has an unmatched closing bracket"));
        }
        let mut frame = self.frames.pop().ok_or_else(internal)?;
        let e = self.reduce(&mut frame)?;
        Ok((frame.kind, e))
    }

    fn reduce(&mut self, frame: &mut Frame) -> JsResult<usize> {
        let operand = frame.operand.take().ok_or_else(|| syntax(self.interp, "an XPath operand is missing"))?;
        let e = self.finish_operand(operand)?;
        frame.vals.push(e);
        while !frame.ops.is_empty() {
            self.interp.use_fuel()?;
            self.reduce_one(frame)?;
        }
        match frame.vals.as_slice() {
            [e] => Ok(*e),
            _ => Err(internal()),
        }
    }

    fn close_paren(&mut self) -> JsResult<()> {
        let (kind, e) = if self.top()?.operand.is_none() {
            (self.frames.pop().ok_or_else(internal)?.kind, None)
        } else {
            let (kind, e) = self.close()?;
            (kind, Some(e))
        };
        match kind {
            FrameKind::Group => self.primary_id(e.ok_or_else(internal)?),
            FrameKind::Args(f, name, mut args) => {
                args.extend(e);
                let (_, _, least, most) = FUNCS.iter().find(|(_, g, ..)| *g == f).copied().ok_or_else(internal)?;
                if args.len() < least || args.len() > most {
                    return Err(self.interp.type_error(&format!("the XPath function {name} takes the wrong number of arguments")));
                }
                let call = self.push_expr(Expr::Call(f, args))?;
                self.primary_id(call)
            }
            _ => Err(self.error("the XPath expression has an unmatched )")),
        }
    }

    fn primary_id(&mut self, id: usize) -> JsResult<()> {
        if self.frames.is_empty() {
            return Err(self.error("the XPath expression has an unmatched )"));
        }
        self.top()?.operand = Some(Operand { filter: Some((id, Vec::new())), ..Operand::default() });
        self.want = Want::After;
        Ok(())
    }

    fn close_bracket(&mut self) -> JsResult<()> {
        let (kind, e) = self.close()?;
        if !matches!(kind, FrameKind::Pred) {
            return Err(self.error("the XPath expression has an unmatched ]"));
        }
        let operand = self.operand()?;
        match (operand.steps.last_mut(), operand.filter.as_mut()) {
            (Some(step), _) => step.preds.push(e),
            (None, Some((_, preds))) => preds.push(e),
            (None, None) => return Err(internal()),
        }
        self.want = Want::After;
        Ok(())
    }

    fn next_argument(&mut self) -> JsResult<()> {
        if !matches!(self.top()?.kind, FrameKind::Args(..)) || self.frames.len() < 2 {
            return Err(self.error("a comma in an XPath expression must separate function arguments"));
        }
        let mut frame = self.frames.pop().ok_or_else(internal)?;
        let e = self.reduce(&mut frame);
        if let (Ok(e), FrameKind::Args(_, _, args)) = (&e, &mut frame.kind) {
            args.push(*e);
        }
        frame.ops.clear();
        frame.vals.clear();
        self.frames.push(frame);
        e?;
        self.want = Want::Operand;
        Ok(())
    }

    fn finish(mut self) -> JsResult<Parsed> {
        let complete = self.want == Want::After || (self.want == Want::Step && self.operand()?.bare_root());
        if !complete || self.frames.len() != 1 {
            return Err(self.error("the XPath expression ends early"));
        }
        let mut frame = self.frames.pop().ok_or_else(internal)?;
        let root = self.reduce(&mut frame)?;
        Ok(Parsed { exprs: self.exprs, root, vars: self.vars, prefixes: self.prefixes.into_iter().collect(), _held: self.held })
    }
}

fn parse(interp: &mut Interp, src: &[u16]) -> JsResult<Parsed> {
    let mut held = HeldBytes::new(interp);
    let raw = lex(interp, &mut held, src)?;
    let toks = classify(interp, &mut held, &raw)?;
    drop(raw);
    let mut p = Parser {
        interp,
        held,
        exprs: Vec::new(),
        vars: Vec::new(),
        var_index: HashMap::new(),
        prefixes: HashSet::new(),
        frames: vec![Frame::new(FrameKind::Top)],
        want: Want::Operand,
    };
    for tok in toks {
        p.interp.use_fuel()?;
        p.token(tok)?;
    }
    p.finish()
}

// ---- values ----

/// A node of the XPath data model. `Root` is the root above the topmost node it names, and `Ns`
/// is a namespace node of `owner`, at position `at` among its namespaces.
#[derive(Clone, PartialEq, Eq, Hash)]
enum XNode {
    Root(ObjId),
    Node(ObjId),
    Ns { owner: ObjId, at: u32, prefix: Key, uri: Key },
}

#[derive(Clone, Copy, PartialEq)]
enum NodeKind {
    Root,
    Ns,
    Xml(XmlKind),
}

/// A list of nodes whose buffer is held against the heap limit.
struct Nodes {
    list: Vec<XNode>,
    held: HeldBytes,
}

impl Nodes {
    fn new(interp: &Interp) -> Self {
        Nodes { list: Vec::new(), held: HeldBytes::new(interp) }
    }

    fn push(&mut self, interp: &mut Interp, n: XNode) -> JsResult<()> {
        push_held(interp, &mut self.held, &mut self.list, n)
    }
}

/// A string value whose units are held against the heap limit when the evaluator built them.
struct Text {
    units: Key,
    _held: HeldBytes,
}

impl Text {
    fn plain(interp: &Interp, units: Key) -> Self {
        Text { units, _held: HeldBytes::new(interp) }
    }
}

struct TextBuf {
    units: Vec<u16>,
    held: HeldBytes,
}

impl TextBuf {
    fn new(interp: &Interp) -> Self {
        TextBuf { units: Vec::new(), held: HeldBytes::new(interp) }
    }

    fn push(&mut self, interp: &mut Interp, more: &[u16]) -> JsResult<()> {
        charge_units(interp, more.len())?;
        let want = self.units.len().saturating_add(more.len());
        let cap = self.units.capacity();
        if want > cap {
            let grown = want.max(cap.saturating_mul(2)).max(16);
            self.held.add(interp, (grown - cap).saturating_mul(2))?;
            self.units.reserve_exact(grown - self.units.len());
        }
        self.units.extend_from_slice(more);
        Ok(())
    }

    fn finish(self) -> Text {
        Text { units: Rc::from(self.units), _held: self.held }
    }
}

enum Val {
    Set(Nodes),
    Bool(bool),
    Num(f64),
    Str(Text),
}

/// A variable's value, read from the variables object before evaluation starts.
enum Bound {
    Set(Vec<XNode>),
    Bool(bool),
    Num(f64),
    Str(Key),
}

#[derive(Clone)]
struct Ctx {
    node: XNode,
    pos: usize,
    size: usize,
}

/// A location path being evaluated. Stage 0 applies the filter expression's predicates to the
/// filter's nodes; stage `s` applies step `s - 1` to each node of `inputs` in turn.
struct PathRun {
    path: usize,
    stage: usize,
    inputs: Nodes,
    ci: usize,
    cur: Nodes,
    pred: usize,
    at: usize,
    kept: Nodes,
    waiting: bool,
    out: Nodes,
    seen: HashSet<XNode>,
    seen_held: HeldBytes,
}

enum Task {
    Eval(usize, Ctx),
    Bin(Bin),
    Neg,
    Logic { and: bool, rhs: usize, ctx: Ctx },
    Bool,
    Call(Func, usize, Ctx),
    PathStart(usize),
    Path(Box<PathRun>),
}

fn to_bool(v: &Val) -> bool {
    match v {
        Val::Set(s) => !s.list.is_empty(),
        Val::Bool(b) => *b,
        Val::Num(n) => *n != 0.0 && !n.is_nan(),
        Val::Str(t) => !t.units.is_empty(),
    }
}

/// The number a string stands for (XPath 1.0 section 4.4): optional white space, an optional
/// minus, digits with an optional fraction, optional white space; anything else is NaN.
fn str_num(interp: &mut Interp, units: &[u16]) -> JsResult<f64> {
    let start = units.iter().position(|&c| !is_xml_space(c)).unwrap_or(units.len());
    let end = units.iter().rposition(|&c| !is_xml_space(c)).map_or(start, |e| e + 1);
    let body = units.get(start..end).unwrap_or_default();
    let (negative, digits) = match body.split_first() {
        Some((&DASH, rest)) => (true, rest),
        _ => (false, body),
    };
    let dots = digits.iter().filter(|&&c| c == DOT).count();
    if dots > 1 || !digits.iter().all(|&c| c == DOT || is_digit(c)) || !digits.iter().any(|&c| is_digit(c)) {
        return Ok(f64::NAN);
    }
    let mut held = HeldBytes::new(interp);
    held.add(interp, digits.len())?;
    let text: String = digits.iter().map(|&c| char::from(c as u8)).collect();
    let n: f64 = text.parse().unwrap_or(f64::NAN);
    Ok(if negative { -n } else { n })
}

/// A number as a string (XPath 1.0 section 4.2): integers without a decimal point, and no
/// exponent.
fn num_text(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity".into() } else { "-Infinity".into() }
    } else if n == 0.0 {
        "0".into()
    } else {
        format!("{n}")
    }
}

/// `round` (XPath 1.0 section 4.4): the nearest integer, halves toward positive infinity, with
/// negative zero for values from -0.5 up to zero.
fn round(n: f64) -> f64 {
    if !n.is_finite() || n == 0.0 {
        return n;
    }
    let floor = n.floor();
    let r = if n - floor >= 0.5 { floor + 1.0 } else { floor };
    if r == 0.0 && n < 0.0 { -0.0 } else { r }
}

fn compare_numbers(op: Bin, a: f64, b: f64) -> bool {
    match op {
        Bin::Eq => a == b,
        Bin::Ne => a != b,
        Bin::Lt => a < b,
        Bin::Le => a <= b,
        Bin::Gt => a > b,
        _ => a >= b,
    }
}

fn flip(op: Bin) -> Bin {
    match op {
        Bin::Lt => Bin::Gt,
        Bin::Le => Bin::Ge,
        Bin::Gt => Bin::Lt,
        Bin::Ge => Bin::Le,
        other => other,
    }
}

fn code_points(units: &[u16]) -> Vec<u32> {
    char::decode_utf16(units.iter().copied()).map(|r| r.map_or_else(|e| u32::from(e.unpaired_surrogate()), u32::from)).collect()
}

fn push_point(out: &mut Vec<u16>, p: u32) {
    match char::from_u32(p) {
        Some(c) => {
            let mut buf = [0u16; 2];
            out.extend_from_slice(c.encode_utf16(&mut buf));
        }
        None => out.extend(u16::try_from(p).ok()),
    }
}

fn lower(c: u16) -> u16 {
    if (0x41..=0x5A).contains(&c) { c + 0x20 } else { c }
}

fn same_ignoring_case(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| lower(x) == lower(y))
}

fn name_uri(name: &XmlName) -> &[u16] {
    name.uri.as_deref().unwrap_or_default()
}

fn kind_of(interp: &Interp, n: &XNode) -> JsResult<NodeKind> {
    Ok(match n {
        XNode::Root(_) => NodeKind::Root,
        XNode::Ns { .. } => NodeKind::Ns,
        XNode::Node(x) => NodeKind::Xml(interp.xml_kind(*x)?),
    })
}

fn parent_of(interp: &Interp, n: &XNode) -> JsResult<Option<XNode>> {
    Ok(match n {
        XNode::Root(_) => None,
        XNode::Node(x) => Some(interp.xml_node(*x)?.parent.map_or(XNode::Root(*x), XNode::Node)),
        XNode::Ns { owner, .. } => Some(XNode::Node(*owner)),
    })
}

/// The parent of `x` and the position of `x` among its children, when `x` is a child.
fn child_position(interp: &mut Interp, x: ObjId) -> JsResult<Option<(ObjId, usize)>> {
    let node = interp.xml_node(x)?;
    let Some(p) = node.parent.filter(|_| node.kind != XmlKind::Attribute) else { return Ok(None) };
    let len = interp.xml_node(p)?.children.len();
    for i in 0..len {
        interp.use_fuel()?;
        if interp.xml_node(p)?.children.get(i) == Some(&x) {
            return Ok(Some((p, i)));
        }
    }
    Ok(None)
}

fn top_of(interp: &mut Interp, mut x: ObjId) -> JsResult<ObjId> {
    while let Some(p) = interp.xml_node(x)?.parent {
        interp.use_fuel()?;
        x = p;
    }
    Ok(x)
}

/// The XPath expression evaluator for one call: the parsed expression, the variables, the
/// prefixes resolved for the current context element, and the document order of the trees met.
struct Ev<'p> {
    parsed: &'p Parsed,
    bound: Vec<Bound>,
    uris: HashMap<Key, Key>,
    order: HashMap<ObjId, u64>,
    root_order: HashMap<ObjId, u64>,
    next: u64,
    held: HeldBytes,
    tasks: Vec<Task>,
    vals: Vec<Val>,
}

impl<'p> Ev<'p> {
    fn new(interp: &mut Interp, parsed: &'p Parsed, bound: Vec<Bound>) -> JsResult<Self> {
        let mut ev = Ev {
            parsed,
            bound: Vec::new(),
            uris: HashMap::new(),
            order: HashMap::new(),
            root_order: HashMap::new(),
            next: 0,
            held: HeldBytes::new(interp),
            tasks: Vec::new(),
            vals: Vec::new(),
        };
        for b in bound {
            let b = match b {
                Bound::Set(list) => {
                    let mut nodes = Nodes::new(interp);
                    for n in list {
                        nodes.push(interp, n)?;
                    }
                    let sorted = ev.sort(interp, nodes)?;
                    ev.held.add(interp, sorted.list.len().saturating_mul(size_of::<XNode>()))?;
                    Bound::Set(sorted.list)
                }
                other => other,
            };
            ev.bound.push(b);
        }
        Ok(ev)
    }

    // ---- document order ----

    fn put_order(&mut self, interp: &mut Interp, x: ObjId) -> JsResult<()> {
        self.held.add(interp, ENTRY_BYTES)?;
        self.order.insert(x, self.next);
        self.next += 1;
        Ok(())
    }

    /// Numbers the tree under `top` in document order: the root, then each node followed by its
    /// attributes and then its children.
    fn number_tree(&mut self, interp: &mut Interp, top: ObjId) -> JsResult<()> {
        if self.root_order.contains_key(&top) {
            return Ok(());
        }
        self.held.add(interp, ENTRY_BYTES)?;
        self.root_order.insert(top, self.next);
        self.next += 1;
        self.number_node(interp, top)?;
        let mut held = HeldBytes::new(interp);
        let mut stack = Vec::new();
        push_held(interp, &mut held, &mut stack, (top, 0usize))?;
        while let Some(&(x, at)) = stack.last() {
            interp.use_fuel()?;
            let Some(c) = interp.xml_node(x)?.children.get(at).copied() else {
                stack.pop();
                continue;
            };
            if let Some(frame) = stack.last_mut() {
                frame.1 += 1;
            }
            self.number_node(interp, c)?;
            push_held(interp, &mut held, &mut stack, (c, 0))?;
        }
        Ok(())
    }

    fn number_node(&mut self, interp: &mut Interp, x: ObjId) -> JsResult<()> {
        self.put_order(interp, x)?;
        let count = interp.xml_node(x)?.attrs.len();
        for i in 0..count {
            interp.use_fuel()?;
            if let Some(a) = interp.xml_node(x)?.attrs.get(i).copied() {
                self.put_order(interp, a)?;
            }
        }
        Ok(())
    }

    fn order_of(&mut self, interp: &mut Interp, n: &XNode) -> JsResult<(u64, u32)> {
        match n {
            XNode::Root(t) => {
                self.number_tree(interp, *t)?;
                Ok((self.root_order.get(t).copied().ok_or_else(internal)?, 0))
            }
            XNode::Node(x) => {
                if !self.order.contains_key(x) {
                    let top = top_of(interp, *x)?;
                    self.number_tree(interp, top)?;
                }
                Ok((self.order.get(x).copied().ok_or_else(internal)?, 0))
            }
            XNode::Ns { owner, at, .. } => {
                let (o, _) = self.order_of(interp, &XNode::Node(*owner))?;
                Ok((o, at.saturating_add(1)))
            }
        }
    }

    /// The nodes in document order, without repeats.
    fn sort(&mut self, interp: &mut Interp, set: Nodes) -> JsResult<Nodes> {
        if set.list.len() < 2 {
            return Ok(set);
        }
        let n = set.list.len() as u64;
        let mut held = HeldBytes::new(interp);
        let mut keyed = Vec::new();
        for x in set.list {
            let k = self.order_of(interp, &x)?;
            push_held(interp, &mut held, &mut keyed, (k, x))?;
        }
        use_fuel_n(interp, n.saturating_mul(u64::from(n.ilog2()) + 1) / 8 + 1)?;
        keyed.sort_by_key(|(k, _)| *k);
        keyed.dedup_by_key(|(k, _)| *k);
        let mut out = Nodes::new(interp);
        for (_, x) in keyed {
            out.push(interp, x)?;
        }
        Ok(out)
    }

    // ---- axes and node tests ----

    fn prefix_uri(&self, p: &Key) -> JsResult<&Key> {
        self.uris.get(p).ok_or_else(internal)
    }

    fn matches(&self, interp: &Interp, axis: Axis, test: &Test, n: &XNode) -> JsResult<bool> {
        let kind = kind_of(interp, n)?;
        let principal = match axis {
            Axis::Attribute => NodeKind::Xml(XmlKind::Attribute),
            Axis::Namespace => NodeKind::Ns,
            _ => NodeKind::Xml(XmlKind::Element),
        };
        let name = match n {
            XNode::Node(x) => interp.xml_node(*x)?.name.as_ref(),
            _ => None,
        };
        Ok(match test {
            Test::Node => true,
            Test::Text => kind == NodeKind::Xml(XmlKind::Text),
            Test::Comment => kind == NodeKind::Xml(XmlKind::Comment),
            Test::Pi(target) => kind == NodeKind::Xml(XmlKind::Pi) && target.as_ref().is_none_or(|t| name.is_some_and(|q| q.local == *t)),
            Test::Any => kind == principal,
            Test::Prefix(p) => {
                let uri = self.prefix_uri(p)?;
                kind == principal && name.is_some_and(|q| name_uri(q) == &**uri)
            }
            Test::Name(p, local) => kind == principal && self.name_matches(n, name, p.as_ref(), local)?,
        })
    }

    fn name_matches(&self, n: &XNode, name: Option<&XmlName>, prefix: Option<&Key>, local: &Key) -> JsResult<bool> {
        if let XNode::Ns { prefix: ns_prefix, .. } = n {
            return Ok(prefix.is_none() && ns_prefix == local);
        }
        let Some(q) = name else { return Ok(false) };
        let uri: &[u16] = match prefix {
            Some(p) => self.prefix_uri(p)?,
            None => &[],
        };
        Ok(q.local == *local && name_uri(q) == uri)
    }

    fn add(&self, interp: &mut Interp, out: &mut Nodes, axis: Axis, test: &Test, n: XNode) -> JsResult<()> {
        interp.use_fuel()?;
        if self.matches(interp, axis, test, &n)? {
            out.push(interp, n)?;
        }
        Ok(())
    }

    /// Adds `x` and the nodes under it, in document order.
    fn subtree(&self, interp: &mut Interp, out: &mut Nodes, axis: Axis, test: &Test, x: ObjId) -> JsResult<()> {
        self.add(interp, out, axis, test, XNode::Node(x))?;
        self.under(interp, out, axis, test, x)
    }

    /// Adds the nodes under `x`, in document order.
    fn under(&self, interp: &mut Interp, out: &mut Nodes, axis: Axis, test: &Test, x: ObjId) -> JsResult<()> {
        let mut held = HeldBytes::new(interp);
        let mut stack = Vec::new();
        push_held(interp, &mut held, &mut stack, (x, 0usize))?;
        while let Some(&(p, at)) = stack.last() {
            let Some(c) = interp.xml_node(p)?.children.get(at).copied() else {
                stack.pop();
                continue;
            };
            if let Some(frame) = stack.last_mut() {
                frame.1 += 1;
            }
            self.add(interp, out, axis, test, XNode::Node(c))?;
            push_held(interp, &mut held, &mut stack, (c, 0))?;
        }
        Ok(())
    }

    fn descendants(&self, interp: &mut Interp, out: &mut Nodes, axis: Axis, test: &Test, n: &XNode) -> JsResult<()> {
        match n {
            XNode::Root(t) => self.subtree(interp, out, axis, test, *t),
            XNode::Node(x) => self.under(interp, out, axis, test, *x),
            XNode::Ns { .. } => Ok(()),
        }
    }

    fn children(&self, interp: &mut Interp, out: &mut Nodes, axis: Axis, test: &Test, n: &XNode) -> JsResult<()> {
        let x = match n {
            XNode::Root(t) => return self.add(interp, out, axis, test, XNode::Node(*t)),
            XNode::Node(x) => *x,
            XNode::Ns { .. } => return Ok(()),
        };
        let count = interp.xml_node(x)?.children.len();
        for i in 0..count {
            if let Some(c) = interp.xml_node(x)?.children.get(i).copied() {
                self.add(interp, out, axis, test, XNode::Node(c))?;
            }
        }
        Ok(())
    }

    fn ancestors(&self, interp: &mut Interp, out: &mut Nodes, axis: Axis, test: &Test, n: &XNode) -> JsResult<()> {
        let mut cur = parent_of(interp, n)?;
        while let Some(p) = cur {
            cur = parent_of(interp, &p)?;
            self.add(interp, out, axis, test, p)?;
        }
        Ok(())
    }

    fn siblings(&self, interp: &mut Interp, out: &mut Nodes, axis: Axis, test: &Test, n: &XNode) -> JsResult<()> {
        let XNode::Node(x) = n else { return Ok(()) };
        let Some((p, at)) = child_position(interp, *x)? else { return Ok(()) };
        let count = interp.xml_node(p)?.children.len();
        let range: Box<dyn Iterator<Item = usize>> = if axis == Axis::FollowingSibling { Box::new(at + 1..count) } else { Box::new((0..at).rev()) };
        for i in range {
            if let Some(c) = interp.xml_node(p)?.children.get(i).copied() {
                self.add(interp, out, axis, test, XNode::Node(c))?;
            }
        }
        Ok(())
    }

    /// The node the following and preceding axes start from: an attribute's or a namespace
    /// node's element, or the node itself. The flag is true for the first two.
    fn start_of(interp: &Interp, n: &XNode) -> JsResult<Option<(ObjId, bool)>> {
        Ok(match n {
            XNode::Root(_) => None,
            XNode::Ns { owner, .. } => Some((*owner, true)),
            XNode::Node(x) => match interp.xml_node(*x)?.parent {
                Some(p) if interp.xml_kind(*x)? == XmlKind::Attribute => Some((p, true)),
                _ => Some((*x, false)),
            },
        })
    }

    fn following(&self, interp: &mut Interp, out: &mut Nodes, axis: Axis, test: &Test, n: &XNode) -> JsResult<()> {
        let Some((mut cur, owned)) = Self::start_of(interp, n)? else { return Ok(()) };
        if owned {
            self.under(interp, out, axis, test, cur)?;
        }
        while let Some((p, at)) = child_position(interp, cur)? {
            let count = interp.xml_node(p)?.children.len();
            for i in at + 1..count {
                if let Some(s) = interp.xml_node(p)?.children.get(i).copied() {
                    self.subtree(interp, out, axis, test, s)?;
                }
            }
            cur = p;
        }
        Ok(())
    }

    fn preceding(&self, interp: &mut Interp, out: &mut Nodes, axis: Axis, test: &Test, n: &XNode) -> JsResult<()> {
        let Some((mut cur, _)) = Self::start_of(interp, n)? else { return Ok(()) };
        while let Some((p, at)) = child_position(interp, cur)? {
            for i in (0..at).rev() {
                let Some(s) = interp.xml_node(p)?.children.get(i).copied() else { continue };
                let mut part = Nodes::new(interp);
                self.subtree(interp, &mut part, axis, test, s)?;
                for x in part.list.into_iter().rev() {
                    out.push(interp, x)?;
                }
            }
            cur = p;
        }
        Ok(())
    }

    fn attributes(&self, interp: &mut Interp, out: &mut Nodes, axis: Axis, test: &Test, n: &XNode) -> JsResult<()> {
        let XNode::Node(x) = n else { return Ok(()) };
        if interp.xml_kind(*x)? != XmlKind::Element {
            return Ok(());
        }
        let count = interp.xml_node(*x)?.attrs.len();
        for i in 0..count {
            if let Some(a) = interp.xml_node(*x)?.attrs.get(i).copied() {
                self.add(interp, out, axis, test, XNode::Node(a))?;
            }
        }
        Ok(())
    }

    fn namespaces(&self, interp: &mut Interp, out: &mut Nodes, axis: Axis, test: &Test, n: &XNode) -> JsResult<()> {
        let XNode::Node(x) = n else { return Ok(()) };
        if interp.xml_kind(*x)? != XmlKind::Element {
            return Ok(());
        }
        let xml = key("xml");
        let mut at = 0u32;
        let mut has_xml = false;
        for ns in scope_of(interp, Some(*x))? {
            let Some(prefix) = ns.prefix else { continue };
            if prefix.is_empty() && ns.uri.is_empty() {
                continue;
            }
            has_xml |= prefix == xml;
            self.add(interp, out, axis, test, XNode::Ns { owner: *x, at, prefix, uri: ns.uri })?;
            at += 1;
        }
        if !has_xml {
            self.add(interp, out, axis, test, XNode::Ns { owner: *x, at, prefix: xml, uri: key(XML_URI) })?;
        }
        Ok(())
    }

    /// The nodes `axis` reaches from `n` that pass `test`, nearest first.
    fn axis(&self, interp: &mut Interp, axis: Axis, test: &Test, n: &XNode) -> JsResult<Nodes> {
        let mut out = Nodes::new(interp);
        match axis {
            Axis::Itself => self.add(interp, &mut out, axis, test, n.clone())?,
            Axis::Child => self.children(interp, &mut out, axis, test, n)?,
            Axis::Descendant => self.descendants(interp, &mut out, axis, test, n)?,
            Axis::DescendantOrSelf => {
                self.add(interp, &mut out, axis, test, n.clone())?;
                self.descendants(interp, &mut out, axis, test, n)?;
            }
            Axis::Parent => {
                if let Some(p) = parent_of(interp, n)? {
                    self.add(interp, &mut out, axis, test, p)?;
                }
            }
            Axis::Ancestor => self.ancestors(interp, &mut out, axis, test, n)?,
            Axis::AncestorOrSelf => {
                self.add(interp, &mut out, axis, test, n.clone())?;
                self.ancestors(interp, &mut out, axis, test, n)?;
            }
            Axis::FollowingSibling | Axis::PrecedingSibling => self.siblings(interp, &mut out, axis, test, n)?,
            Axis::Following => self.following(interp, &mut out, axis, test, n)?,
            Axis::Preceding => self.preceding(interp, &mut out, axis, test, n)?,
            Axis::Attribute => self.attributes(interp, &mut out, axis, test, n)?,
            Axis::Namespace => self.namespaces(interp, &mut out, axis, test, n)?,
        }
        Ok(out)
    }

    // ---- string values and conversions ----

    /// The string-value of a node (XPath 1.0 section 5): the text under an element or the root,
    /// the value of any other node, and a namespace node's URI.
    fn string_value(&mut self, interp: &mut Interp, n: &XNode) -> JsResult<Text> {
        let x = match n {
            XNode::Ns { uri, .. } => return Ok(Text::plain(interp, uri.clone())),
            XNode::Root(t) => *t,
            XNode::Node(x) => *x,
        };
        let kind = interp.xml_kind(x)?;
        if kind == XmlKind::Element {
            return self.text_under(interp, x);
        }
        if matches!(n, XNode::Root(_)) && kind != XmlKind::Text {
            return Ok(Text::plain(interp, empty()));
        }
        let value = interp.xml_node(x)?.value.clone();
        let units = flat_charged(interp, &value)?;
        charge_units(interp, units.len())?;
        Ok(Text::plain(interp, units))
    }

    fn text_under(&mut self, interp: &mut Interp, x: ObjId) -> JsResult<Text> {
        let mut buf = TextBuf::new(interp);
        let mut held = HeldBytes::new(interp);
        let mut stack = Vec::new();
        push_held(interp, &mut held, &mut stack, (x, 0usize))?;
        while let Some(&(p, at)) = stack.last() {
            interp.use_fuel()?;
            let Some(c) = interp.xml_node(p)?.children.get(at).copied() else {
                stack.pop();
                continue;
            };
            if let Some(frame) = stack.last_mut() {
                frame.1 += 1;
            }
            match interp.xml_kind(c)? {
                XmlKind::Text => {
                    let value = interp.xml_node(c)?.value.clone();
                    let units = flat_charged(interp, &value)?;
                    buf.push(interp, &units)?;
                }
                XmlKind::Element => push_held(interp, &mut held, &mut stack, (c, 0))?,
                _ => {}
            }
        }
        Ok(buf.finish())
    }

    fn text_of(&mut self, interp: &mut Interp, v: Val) -> JsResult<Text> {
        match v {
            Val::Set(s) => match s.list.first() {
                Some(n) => self.string_value(interp, n),
                None => Ok(Text::plain(interp, empty())),
            },
            Val::Bool(b) => Ok(Text::plain(interp, key(if b { "true" } else { "false" }))),
            Val::Num(n) => Ok(Text::plain(interp, key(&num_text(n)))),
            Val::Str(t) => {
                // The caller reads the units, so they take fuel each time a value is used.
                charge_units(interp, t.units.len())?;
                Ok(t)
            }
        }
    }

    /// The number a node's string-value stands for.
    fn node_number(&mut self, interp: &mut Interp, n: &XNode) -> JsResult<f64> {
        let t = self.string_value(interp, n)?;
        str_num(interp, &t.units)
    }

    fn number_of(&mut self, interp: &mut Interp, v: Val) -> JsResult<f64> {
        match v {
            Val::Num(n) => Ok(n),
            Val::Bool(b) => Ok(f64::from(u8::from(b))),
            other => {
                let t = self.text_of(interp, other)?;
                str_num(interp, &t.units)
            }
        }
    }

    // ---- comparisons and operators ----

    fn compare(&mut self, interp: &mut Interp, op: Bin, a: Val, b: Val) -> JsResult<bool> {
        match (a, b) {
            (Val::Set(x), Val::Set(y)) => self.compare_sets(interp, op, &x, &y),
            (Val::Set(x), other) => self.compare_set(interp, op, &x, other),
            (other, Val::Set(y)) => self.compare_set(interp, flip(op), &y, other),
            (x, y) => self.compare_values(interp, op, x, y),
        }
    }

    fn compare_values(&mut self, interp: &mut Interp, op: Bin, a: Val, b: Val) -> JsResult<bool> {
        if !matches!(op, Bin::Eq | Bin::Ne) {
            let (x, y) = (self.number_of(interp, a)?, self.number_of(interp, b)?);
            return Ok(compare_numbers(op, x, y));
        }
        let equal = if matches!(a, Val::Bool(_)) || matches!(b, Val::Bool(_)) {
            to_bool(&a) == to_bool(&b)
        } else if matches!(a, Val::Num(_)) || matches!(b, Val::Num(_)) {
            let (x, y) = (self.number_of(interp, a)?, self.number_of(interp, b)?);
            return Ok(compare_numbers(op, x, y));
        } else {
            self.text_of(interp, a)?.units == self.text_of(interp, b)?.units
        };
        Ok(equal == (op == Bin::Eq))
    }

    /// Compares each node of `set` with `other`, which is not a node-set; `set` is on the left.
    fn compare_set(&mut self, interp: &mut Interp, op: Bin, set: &Nodes, other: Val) -> JsResult<bool> {
        if let Val::Str(t) = &other {
            charge_units(interp, t.units.len())?;
        }
        let n = match other {
            Val::Bool(b) => return self.compare_values(interp, op, Val::Bool(!set.list.is_empty()), Val::Bool(b)),
            Val::Str(t) if matches!(op, Bin::Eq | Bin::Ne) => {
                for node in &set.list {
                    if (self.string_value(interp, node)?.units == t.units) == (op == Bin::Eq) {
                        return Ok(true);
                    }
                }
                return Ok(false);
            }
            Val::Str(t) => str_num(interp, &t.units)?,
            Val::Num(n) => n,
            Val::Set(_) => return Err(internal()),
        };
        for node in &set.list {
            let v = self.node_number(interp, node)?;
            if compare_numbers(op, v, n) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn strings_of(&mut self, interp: &mut Interp, held: &mut HeldBytes, set: &Nodes) -> JsResult<HashSet<Key>> {
        let mut out = HashSet::new();
        for node in &set.list {
            let t = self.string_value(interp, node)?;
            held.add(interp, ENTRY_BYTES.saturating_add(t.units.len().saturating_mul(2)))?;
            out.insert(t.units);
        }
        Ok(out)
    }

    /// The smallest and largest numbers the nodes' string-values stand for, leaving out NaN.
    fn number_range(&mut self, interp: &mut Interp, set: &Nodes) -> JsResult<Option<(f64, f64)>> {
        let mut range: Option<(f64, f64)> = None;
        for node in &set.list {
            let v = self.node_number(interp, node)?;
            if !v.is_nan() {
                range = Some(range.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v))));
            }
        }
        Ok(range)
    }

    fn compare_sets(&mut self, interp: &mut Interp, op: Bin, a: &Nodes, b: &Nodes) -> JsResult<bool> {
        if a.list.is_empty() || b.list.is_empty() {
            return Ok(false);
        }
        if matches!(op, Bin::Eq | Bin::Ne) {
            let mut held = HeldBytes::new(interp);
            let right = self.strings_of(interp, &mut held, b)?;
            for node in &a.list {
                let s = self.string_value(interp, node)?;
                let found = if op == Bin::Eq { right.contains(&s.units) } else { right.len() > 1 || !right.contains(&s.units) };
                if found {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        let (Some((a_lo, a_hi)), Some((b_lo, b_hi))) = (self.number_range(interp, a)?, self.number_range(interp, b)?) else { return Ok(false) };
        Ok(match op {
            Bin::Lt | Bin::Le => compare_numbers(op, a_lo, b_hi),
            _ => compare_numbers(op, a_hi, b_lo),
        })
    }

    fn binary(&mut self, interp: &mut Interp, op: Bin, a: Val, b: Val) -> JsResult<Val> {
        match op {
            Bin::Union => {
                let (Val::Set(mut x), Val::Set(y)) = (a, b) else {
                    return Err(interp.type_error("both sides of an XPath | must select nodes"));
                };
                if x.list.is_empty() {
                    return Ok(Val::Set(y));
                }
                for n in y.list {
                    x.push(interp, n)?;
                }
                Ok(Val::Set(self.sort(interp, x)?))
            }
            Bin::Add | Bin::Sub | Bin::Mul | Bin::Div | Bin::Mod => {
                let (x, y) = (self.number_of(interp, a)?, self.number_of(interp, b)?);
                Ok(Val::Num(match op {
                    Bin::Add => x + y,
                    Bin::Sub => x - y,
                    Bin::Mul => x * y,
                    Bin::Div => x / y,
                    _ => x % y,
                }))
            }
            Bin::Or | Bin::And => Err(internal()),
            _ => Ok(Val::Bool(self.compare(interp, op, a, b)?)),
        }
    }

    // ---- functions ----

    fn set_arg(interp: &mut Interp, v: Option<Val>, name: &str) -> JsResult<Nodes> {
        match v {
            Some(Val::Set(s)) => Ok(s),
            _ => Err(interp.type_error(&format!("the XPath function {name} needs a node-set"))),
        }
    }

    fn text_arg(&mut self, interp: &mut Interp, v: Option<Val>, ctx: &Ctx) -> JsResult<Text> {
        match v {
            Some(v) => self.text_of(interp, v),
            None => self.string_value(interp, &ctx.node),
        }
    }

    fn number_arg(&mut self, interp: &mut Interp, v: Option<Val>) -> JsResult<f64> {
        let v = v.ok_or_else(internal)?;
        self.number_of(interp, v)
    }

    fn call(&mut self, interp: &mut Interp, f: Func, args: Vec<Val>, ctx: &Ctx) -> JsResult<Val> {
        let mut args = args.into_iter();
        Ok(match f {
            Func::Last => Val::Num(ctx.size as f64),
            Func::Position => Val::Num(ctx.pos as f64),
            Func::Count => Val::Num(Self::set_arg(interp, args.next(), "count")?.list.len() as f64),
            Func::Id => Val::Set(Nodes::new(interp)),
            Func::LocalName | Func::NamespaceUri | Func::Name => Val::Str(self.name_function(interp, f, args.next(), ctx)?),
            Func::Sum => {
                let set = Self::set_arg(interp, args.next(), "sum")?;
                let mut total = 0.0;
                for n in &set.list {
                    total += self.node_number(interp, n)?;
                }
                Val::Num(total)
            }
            Func::Boolean => Val::Bool(to_bool(&args.next().ok_or_else(internal)?)),
            Func::Not => Val::Bool(!to_bool(&args.next().ok_or_else(internal)?)),
            Func::True => Val::Bool(true),
            Func::False => Val::Bool(false),
            Func::Lang => {
                let wanted = self.text_arg(interp, args.next(), ctx)?;
                Val::Bool(self.lang(interp, &ctx.node, &wanted.units)?)
            }
            Func::Number => match args.next() {
                Some(v) => Val::Num(self.number_of(interp, v)?),
                None => Val::Num(self.node_number(interp, &ctx.node)?),
            },
            Func::Floor => Val::Num(self.number_arg(interp, args.next())?.floor()),
            Func::Ceiling => Val::Num(self.number_arg(interp, args.next())?.ceil()),
            Func::Round => Val::Num(round(self.number_arg(interp, args.next())?)),
            _ => return self.string_function(interp, f, args.collect(), ctx),
        })
    }

    fn string_function(&mut self, interp: &mut Interp, f: Func, args: Vec<Val>, ctx: &Ctx) -> JsResult<Val> {
        let mut texts = Vec::new();
        let mut held = HeldBytes::new(interp);
        for v in args {
            let t = self.text_of(interp, v)?;
            push_held(interp, &mut held, &mut texts, t)?;
        }
        if texts.is_empty() && matches!(f, Func::String | Func::StringLength | Func::NormalizeSpace) {
            texts.push(self.string_value(interp, &ctx.node)?);
        }
        if f == Func::String {
            return Ok(Val::Str(texts.into_iter().next().ok_or_else(internal)?));
        }
        let units = |at: usize| texts.get(at).map(|t| &*t.units).unwrap_or_default();
        Ok(match f {
            Func::StringLength => Val::Num(char::decode_utf16(units(0).iter().copied()).count() as f64),
            Func::StartsWith => Val::Bool(units(0).starts_with(units(1))),
            Func::Contains => Val::Bool(find(interp, units(0), units(1))?.is_some()),
            Func::SubstringBefore => {
                let at = find(interp, units(0), units(1))?;
                Val::Str(self.slice(interp, at.and_then(|i| units(0).get(..i)).unwrap_or_default())?)
            }
            Func::SubstringAfter => {
                let at = find(interp, units(0), units(1))?;
                Val::Str(self.slice(interp, at.and_then(|i| units(0).get(i + units(1).len()..)).unwrap_or_default())?)
            }
            Func::Concat => {
                let mut buf = TextBuf::new(interp);
                for t in &texts {
                    buf.push(interp, &t.units)?;
                }
                Val::Str(buf.finish())
            }
            Func::NormalizeSpace => Val::Str(normalize_space(interp, units(0))?),
            Func::Translate => Val::Str(translate(interp, units(0), units(1), units(2))?),
            _ => return Err(internal()),
        })
    }

    fn slice(&mut self, interp: &mut Interp, units: &[u16]) -> JsResult<Text> {
        let mut buf = TextBuf::new(interp);
        buf.push(interp, units)?;
        Ok(buf.finish())
    }

    fn substring(&mut self, interp: &mut Interp, args: Vec<Val>) -> JsResult<Val> {
        let mut args = args.into_iter();
        let text = self.text_of(interp, args.next().ok_or_else(internal)?)?;
        let start = round(self.number_arg(interp, args.next())?);
        let end = match args.next() {
            Some(len) => start + round(self.number_of(interp, len)?),
            None => f64::INFINITY,
        };
        charge_units(interp, text.units.len())?;
        let mut held = HeldBytes::new(interp);
        // The code points take 4 bytes each, and the result up to twice its 2-byte units.
        held.add(interp, text.units.len().saturating_mul(8))?;
        let mut out = Vec::new();
        for (i, p) in code_points(&text.units).into_iter().enumerate() {
            let position = (i + 1) as f64;
            if position >= start && position < end {
                push_point(&mut out, p);
            }
        }
        Ok(Val::Str(Text { units: Rc::from(out), _held: held }))
    }

    fn name_function(&mut self, interp: &mut Interp, f: Func, arg: Option<Val>, ctx: &Ctx) -> JsResult<Text> {
        let node = match arg {
            None => Some(ctx.node.clone()),
            Some(Val::Set(s)) => s.list.into_iter().next(),
            Some(_) => return Err(interp.type_error("the XPath name functions need a node-set")),
        };
        let units = match node {
            None | Some(XNode::Root(_)) => empty(),
            Some(XNode::Ns { prefix, .. }) => {
                if f == Func::NamespaceUri {
                    empty()
                } else {
                    prefix
                }
            }
            Some(XNode::Node(x)) => node_name(interp, f, x)?,
        };
        Ok(Text::plain(interp, units))
    }

    /// Whether the context node's language, from the nearest `xml:lang` attribute on it or an
    /// ancestor, is `wanted` or a sublanguage of it, ignoring case.
    fn lang(&mut self, interp: &mut Interp, n: &XNode, wanted: &[u16]) -> JsResult<bool> {
        let mut cur = match n {
            XNode::Root(_) => None,
            XNode::Node(x) => Some(*x),
            XNode::Ns { owner, .. } => Some(*owner),
        };
        let xml = key(XML_URI);
        while let Some(x) = cur {
            interp.use_fuel()?;
            let node = interp.xml_node(x)?;
            let parent = node.parent;
            let count = node.attrs.len();
            for i in 0..count {
                interp.use_fuel()?;
                let Some(a) = interp.xml_node(x)?.attrs.get(i).copied() else { break };
                let attr = interp.xml_node(a)?;
                if attr.name.as_ref().is_some_and(|q| name_uri(q) == &*xml && units_are(&q.local, "lang")) {
                    let value = attr.value.clone();
                    let value = flat_charged(interp, &value)?;
                    let head = value.get(..wanted.len()).unwrap_or_default();
                    return Ok(same_ignoring_case(head, wanted) && (value.len() == wanted.len() || value.get(wanted.len()) == Some(&DASH)));
                }
            }
            cur = parent;
        }
        Ok(false)
    }

    // ---- running ----

    fn task(&mut self, interp: &mut Interp, t: Task) -> JsResult<()> {
        push_held(interp, &mut self.held, &mut self.tasks, t)
    }

    fn val(&mut self, interp: &mut Interp, v: Val) -> JsResult<()> {
        push_held(interp, &mut self.held, &mut self.vals, v)
    }

    fn pop(&mut self) -> JsResult<Val> {
        self.vals.pop().ok_or_else(internal)
    }

    fn root_of(interp: &mut Interp, n: &XNode) -> JsResult<XNode> {
        Ok(match n {
            XNode::Root(t) => XNode::Root(*t),
            XNode::Node(x) => XNode::Root(top_of(interp, *x)?),
            XNode::Ns { owner, .. } => XNode::Root(top_of(interp, *owner)?),
        })
    }

    fn bound_value(&mut self, interp: &mut Interp, at: usize) -> JsResult<Val> {
        Ok(match self.bound.get(at).ok_or_else(internal)? {
            Bound::Bool(b) => Val::Bool(*b),
            Bound::Num(n) => Val::Num(*n),
            Bound::Str(s) => Val::Str(Text::plain(interp, s.clone())),
            Bound::Set(list) => {
                let list = list.clone();
                let mut nodes = Nodes::new(interp);
                nodes.held.add(interp, list.len().saturating_mul(size_of::<XNode>()))?;
                nodes.list = list;
                Val::Set(nodes)
            }
        })
    }

    fn eval(&mut self, interp: &mut Interp, e: usize, ctx: Ctx) -> JsResult<()> {
        let parsed = self.parsed;
        match parsed.exprs.get(e).ok_or_else(internal)? {
            Expr::Num(n) => self.val(interp, Val::Num(*n)),
            Expr::Lit(s) => {
                let t = Text::plain(interp, s.clone());
                self.val(interp, Val::Str(t))
            }
            Expr::Var(at) => {
                let v = self.bound_value(interp, *at)?;
                self.val(interp, v)
            }
            Expr::Neg(a) => {
                self.task(interp, Task::Neg)?;
                self.task(interp, Task::Eval(*a, ctx))
            }
            Expr::Bin(op @ (Bin::And | Bin::Or), a, b) => {
                self.task(interp, Task::Logic { and: *op == Bin::And, rhs: *b, ctx: ctx.clone() })?;
                self.task(interp, Task::Eval(*a, ctx))
            }
            Expr::Bin(op, a, b) => {
                self.task(interp, Task::Bin(*op))?;
                self.task(interp, Task::Eval(*b, ctx.clone()))?;
                self.task(interp, Task::Eval(*a, ctx))
            }
            Expr::Call(f, args) => {
                self.task(interp, Task::Call(*f, args.len(), ctx.clone()))?;
                for a in args.iter().rev() {
                    self.task(interp, Task::Eval(*a, ctx.clone()))?;
                }
                Ok(())
            }
            Expr::Path { filter: Some((primary, _)), .. } => {
                self.task(interp, Task::PathStart(e))?;
                self.task(interp, Task::Eval(*primary, ctx))
            }
            Expr::Path { filter: None, absolute, steps } => {
                let start = if *absolute { Self::root_of(interp, &ctx.node)? } else { ctx.node };
                let mut inputs = Nodes::new(interp);
                inputs.push(interp, start)?;
                if steps.is_empty() {
                    return self.val(interp, Val::Set(inputs));
                }
                let first = self.step_axis(interp, e, 1, &inputs.list.first().cloned().ok_or_else(internal)?)?;
                let run = self.new_run(interp, e, 1, inputs, first);
                self.run_path(interp, Box::new(run))
            }
        }
    }

    fn new_run(&self, interp: &Interp, path: usize, stage: usize, inputs: Nodes, cur: Nodes) -> PathRun {
        PathRun {
            path,
            stage,
            inputs,
            ci: 0,
            cur,
            pred: 0,
            at: 0,
            kept: Nodes::new(interp),
            waiting: false,
            out: Nodes::new(interp),
            seen: HashSet::new(),
            seen_held: HeldBytes::new(interp),
        }
    }

    fn step(&mut self, interp: &mut Interp, t: Task) -> JsResult<()> {
        match t {
            Task::Eval(e, ctx) => self.eval(interp, e, ctx),
            Task::Bin(op) => {
                let b = self.pop()?;
                let a = self.pop()?;
                let v = self.binary(interp, op, a, b)?;
                self.val(interp, v)
            }
            Task::Neg => {
                let a = self.pop()?;
                let n = self.number_of(interp, a)?;
                self.val(interp, Val::Num(-n))
            }
            Task::Logic { and, rhs, ctx } => {
                let left = to_bool(&self.pop()?);
                if left != and {
                    return self.val(interp, Val::Bool(left));
                }
                self.task(interp, Task::Bool)?;
                self.task(interp, Task::Eval(rhs, ctx))
            }
            Task::Bool => {
                let v = to_bool(&self.pop()?);
                self.val(interp, Val::Bool(v))
            }
            Task::Call(f, n, ctx) => {
                let args = self.vals.split_off(self.vals.len().checked_sub(n).ok_or_else(internal)?);
                let v = if f == Func::Substring { self.substring(interp, args)? } else { self.call(interp, f, args, &ctx)? };
                self.val(interp, v)
            }
            Task::PathStart(e) => {
                let Val::Set(set) = self.pop()? else {
                    return Err(interp.type_error("an XPath predicate or step follows a value that is not a node-set"));
                };
                let run = self.new_run(interp, e, 0, Nodes::new(interp), set);
                self.run_path(interp, Box::new(run))
            }
            Task::Path(run) => self.run_path(interp, run),
        }
    }

    /// The predicates of a path's stage, and its step for a stage after the filter's.
    fn stage_of(&self, path: usize, stage: usize) -> JsResult<(&'p [usize], Option<&'p Step>)> {
        let parsed = self.parsed;
        let Some(Expr::Path { filter, steps, .. }) = parsed.exprs.get(path) else { return Err(internal()) };
        match stage.checked_sub(1) {
            None => Ok((filter.as_ref().map(|(_, p)| p.as_slice()).unwrap_or_default(), None)),
            Some(s) => {
                let step = steps.get(s).ok_or_else(internal)?;
                Ok((&step.preds, Some(step)))
            }
        }
    }

    fn step_axis(&self, interp: &mut Interp, path: usize, stage: usize, n: &XNode) -> JsResult<Nodes> {
        let (_, step) = self.stage_of(path, stage)?;
        let step = step.ok_or_else(internal)?;
        self.axis(interp, step.axis, &step.test, n)
    }

    /// Runs a path until a predicate needs evaluating, which it schedules before itself, or until
    /// the path's node-set is complete, which it pushes.
    fn run_path(&mut self, interp: &mut Interp, mut r: Box<PathRun>) -> JsResult<()> {
        loop {
            interp.use_fuel()?;
            if r.waiting {
                r.waiting = false;
                let v = self.pop()?;
                let keep = match v {
                    Val::Num(n) => n == (r.at + 1) as f64,
                    other => to_bool(&other),
                };
                if keep && let Some(n) = r.cur.list.get(r.at).cloned() {
                    r.kept.push(interp, n)?;
                }
                r.at += 1;
            }
            let (preds, _) = self.stage_of(r.path, r.stage)?;
            if let Some(&pred) = preds.get(r.pred) {
                if let Some(node) = r.cur.list.get(r.at).cloned() {
                    let ctx = Ctx { node, pos: r.at + 1, size: r.cur.list.len() };
                    r.waiting = true;
                    self.task(interp, Task::Path(r))?;
                    return self.task(interp, Task::Eval(pred, ctx));
                }
                r.cur = replace(&mut r.kept, Nodes::new(interp));
                r.pred += 1;
                r.at = 0;
                continue;
            }
            if !self.next_context(interp, &mut r)? {
                return Ok(());
            }
        }
    }

    /// Adds the current context's nodes to the stage's result and moves to the next context or
    /// stage. Returns false when the path is complete and its node-set has been pushed.
    fn next_context(&mut self, interp: &mut Interp, r: &mut PathRun) -> JsResult<bool> {
        let cur = replace(&mut r.cur, Nodes::new(interp));
        if r.inputs.list.len() > 1 {
            for n in cur.list {
                if !r.seen.contains(&n) {
                    r.seen_held.add(interp, ENTRY_BYTES)?;
                    r.seen.insert(n.clone());
                    r.out.push(interp, n)?;
                }
            }
        } else {
            r.out = cur;
        }
        r.ci += 1;
        if let Some(n) = r.inputs.list.get(r.ci).cloned() {
            r.cur = self.step_axis(interp, r.path, r.stage, &n)?;
            r.pred = 0;
            r.at = 0;
            return Ok(true);
        }
        let out = replace(&mut r.out, Nodes::new(interp));
        let (_, step) = self.stage_of(r.path, r.stage)?;
        let done = if r.inputs.list.len() > 1 {
            self.sort(interp, out)?
        } else if step.is_some_and(|s| s.axis.reverse()) {
            let mut out = out;
            out.list.reverse();
            out
        } else {
            out
        };
        r.seen.clear();
        r.seen_held = HeldBytes::new(interp);
        r.stage += 1;
        let more = self.stage_of(r.path, r.stage).is_ok();
        let Some(first) = done.list.first().cloned().filter(|_| more) else {
            self.val(interp, Val::Set(done))?;
            return Ok(false);
        };
        r.inputs = done;
        r.ci = 0;
        r.cur = self.step_axis(interp, r.path, r.stage, &first)?;
        r.pred = 0;
        r.at = 0;
        Ok(true)
    }

    fn run(&mut self, interp: &mut Interp, e: usize, ctx: Ctx) -> JsResult<Val> {
        self.tasks.clear();
        self.vals.clear();
        self.task(interp, Task::Eval(e, ctx))?;
        while let Some(t) = self.tasks.pop() {
            interp.use_fuel()?;
            self.step(interp, t)?;
        }
        let v = self.pop()?;
        if !self.vals.is_empty() {
            return Err(internal());
        }
        Ok(v)
    }

    /// Binds the expression's prefixes to the namespaces in scope on `x`.
    fn resolve_prefixes(&mut self, interp: &mut Interp, x: ObjId) -> JsResult<()> {
        self.uris.clear();
        if self.parsed.prefixes.is_empty() {
            return Ok(());
        }
        let scope = scope_of(interp, Some(x))?;
        let parsed = self.parsed;
        for p in &parsed.prefixes {
            let uri = if units_are(p, "xml") {
                key(XML_URI)
            } else {
                match scope.iter().find(|ns| ns.prefix.as_ref() == Some(p)) {
                    Some(ns) if !ns.uri.is_empty() => ns.uri.clone(),
                    _ => {
                        let shown = String::from_utf16_lossy(p);
                        return Err(interp.type_error(&format!("the XPath prefix {shown} is not bound to a namespace")));
                    }
                }
            };
            self.held.add(interp, ENTRY_BYTES)?;
            self.uris.insert(p.clone(), uri);
        }
        Ok(())
    }

    /// The nodes the expression selects with `x` as the context node.
    fn select(&mut self, interp: &mut Interp, x: ObjId) -> JsResult<Vec<ObjId>> {
        self.resolve_prefixes(interp, x)?;
        let root = self.parsed.root;
        let Val::Set(set) = self.run(interp, root, Ctx { node: XNode::Node(x), pos: 1, size: 1 })? else {
            return Err(interp.type_error("the XPath expression does not evaluate to a node-set"));
        };
        let mut out = Vec::new();
        let mut held = HeldBytes::new(interp);
        for n in set.list {
            match n {
                XNode::Node(id) => push_held(interp, &mut held, &mut out, id)?,
                XNode::Root(_) => return Err(interp.type_error("the XPath expression selects the root node, which is not an XML value")),
                XNode::Ns { .. } => return Err(interp.type_error("the XPath expression selects a namespace node, which is not an XML value")),
            }
        }
        Ok(out)
    }
}

fn node_name(interp: &mut Interp, f: Func, x: ObjId) -> JsResult<Key> {
    let node = interp.xml_node(x)?;
    let (kind, parent) = (node.kind, node.parent);
    let Some(name) = node.name.clone() else { return Ok(empty()) };
    match (f, kind) {
        (_, XmlKind::Text | XmlKind::Comment) => Ok(empty()),
        (Func::NamespaceUri, XmlKind::Pi) => Ok(empty()),
        (Func::NamespaceUri, _) => Ok(Rc::from(name_uri(&name))),
        (Func::LocalName, _) | (_, XmlKind::Pi) => Ok(name.local),
        _ => qualified_name(interp, x, parent, &name, kind == XmlKind::Attribute),
    }
}

/// An element's or attribute's name as a QName, with a prefix declared for its namespace.
fn qualified_name(interp: &mut Interp, x: ObjId, parent: Option<ObjId>, name: &XmlName, attribute: bool) -> JsResult<Key> {
    let uri = name_uri(name);
    if uri.is_empty() {
        return Ok(name.local.clone());
    }
    let prefix = if units_are(uri, XML_URI) {
        key("xml")
    } else {
        match &name.prefix {
            Some(p) if !(attribute && p.is_empty()) => p.clone(),
            _ => {
                let scope = scope_of(interp, if attribute { parent } else { Some(x) })?;
                let usable = |ns: &XmlNs| *ns.uri == *uri && ns.prefix.as_ref().is_some_and(|p| !(attribute && p.is_empty()));
                scope.into_iter().find(usable).and_then(|ns| ns.prefix).unwrap_or_else(empty)
            }
        }
    };
    if prefix.is_empty() {
        return Ok(name.local.clone());
    }
    let mut units = prefix.to_vec();
    units.push(COLON);
    units.extend_from_slice(&name.local);
    Ok(Rc::from(units))
}

/// The first index where `needle` occurs in `hay`. Every position tried takes fuel.
fn find(interp: &mut Interp, hay: &[u16], needle: &[u16]) -> JsResult<Option<usize>> {
    string_builtins::find(interp, hay, needle, 0)
}

fn normalize_space(interp: &mut Interp, units: &[u16]) -> JsResult<Text> {
    let mut buf = TextBuf::new(interp);
    for word in units.split(|&c| is_xml_space(c)).filter(|w| !w.is_empty()) {
        if !buf.units.is_empty() {
            buf.push(interp, &[0x20])?;
        }
        buf.push(interp, word)?;
    }
    Ok(buf.finish())
}

fn translate(interp: &mut Interp, units: &[u16], from: &[u16], to: &[u16]) -> JsResult<Text> {
    charge_units(interp, units.len() + from.len() + to.len())?;
    let mut held = HeldBytes::new(interp);
    held.add(interp, from.len().saturating_add(units.len()).saturating_add(to.len()).saturating_mul(ENTRY_BYTES))?;
    let to = code_points(to);
    let mut map: HashMap<u32, Option<u32>> = HashMap::new();
    for (i, p) in code_points(from).into_iter().enumerate() {
        map.entry(p).or_insert_with(|| to.get(i).copied());
    }
    let mut out = Vec::new();
    for p in code_points(units) {
        match map.get(&p) {
            Some(Some(t)) => push_point(&mut out, *t),
            Some(None) => {}
            None => push_point(&mut out, p),
        }
    }
    Ok(Text { units: Rc::from(out), _held: held })
}

/// Reads the variables the expression names from `variables`.
fn bind(interp: &mut Interp, held: &mut HeldBytes, parsed: &Parsed, variables: &Value) -> JsResult<Vec<Bound>> {
    let mut out = Vec::new();
    for name in &parsed.vars {
        let v = match variables {
            Value::Object(o) if interp.has_property(*o, name)? => interp.get_from(*o, name, variables)?,
            _ => Value::Undefined,
        };
        if matches!(v, Value::Undefined) {
            let shown = String::from_utf16_lossy(name);
            return Err(interp.type_error(&format!("the XPath variable ${shown} is not bound")));
        }
        interp.temp_roots.push(v.clone());
        let b = match (interp.classify(&v), &v) {
            (Xv::Node(n), _) => Bound::Set(vec![XNode::Node(n)]),
            (Xv::List(l), _) => {
                // A later getter may change the list, so its items are held as roots themselves.
                let count = interp.xml_list(l)?.items.len();
                held.add(interp, count.saturating_mul(size_of::<ObjId>() + size_of::<XNode>() + size_of::<Value>()))?;
                let items = interp.xml_list(l)?.items.clone();
                interp.temp_roots.extend(items.iter().map(|&n| Value::Object(n)));
                Bound::Set(items.into_iter().map(XNode::Node).collect())
            }
            (Xv::Other, Value::Bool(b)) => Bound::Bool(*b),
            (Xv::Other, Value::Number(n)) => Bound::Num(*n),
            (Xv::Other, _) => {
                let s = interp.to_str(&v)?;
                Bound::Str(flat_charged(interp, &s)?)
            }
        };
        push_held(interp, held, &mut out, b)?;
    }
    Ok(out)
}

/// The nodes an expression runs on: the XML value itself, or each element of an XMLList.
fn contexts(interp: &mut Interp, held: &mut HeldBytes, this: &Value) -> JsResult<Vec<ObjId>> {
    match interp.classify(this) {
        Xv::Node(n) => Ok(vec![n]),
        Xv::List(l) => {
            let count = interp.xml_list(l)?.items.len();
            let mut out = Vec::new();
            for i in 0..count {
                interp.use_fuel()?;
                let Some(item) = interp.xml_list(l)?.items.get(i).copied() else { break };
                if interp.xml_kind(item)? == XmlKind::Element {
                    push_held(interp, held, &mut out, item)?;
                }
            }
            Ok(out)
        }
        Xv::Other => Err(interp.type_error("the method needs an XML value")),
    }
}

/// `xpath(expression[, variables])`: the nodes the XPath 1.0 `expression` selects with this XML
/// value as the context node, or, for an XMLList, with each of its elements in turn, as one
/// XMLList. Each property of `variables` binds the variable of its name.
pub(crate) fn xpath(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    if matches!(interp.classify(&this), Xv::Other) {
        return Err(interp.type_error("the method needs an XML value"));
    }
    let Value::String(source) = arg(args, 0) else {
        return Err(interp.type_error("xpath needs the expression as a string"));
    };
    let variables = arg(args, 1);
    if !matches!(variables, Value::Undefined | Value::Object(_)) {
        return Err(interp.type_error("xpath needs its variables as an object"));
    }
    interp.rooted(&[this.clone(), variables.clone()], |i| {
        let units = flat_charged(i, &source)?;
        let parsed = parse(i, &units)?;
        let mut held = HeldBytes::new(i);
        let bound = bind(i, &mut held, &parsed, &variables)?;
        let items = contexts(i, &mut held, &this)?;
        let out = i.new_xml_list(None, None)?;
        i.xml_keep(out);
        let mut ev = Ev::new(i, &parsed, bound)?;
        for item in items {
            for id in ev.select(i, item)? {
                i.list_push(out, id)?;
            }
        }
        Ok(Value::Object(out))
    })
}
