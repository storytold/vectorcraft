//! The syntax tree: expressions, statements and functions kept in arenas ([`Ast`]) and referred to
//! by index, each with its source span. Nodes hold indices rather than boxes, so dropping a deeply
//! nested tree does not recurse.

use crate::token::Span;

/// An expression's index in [`Ast::exprs`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExprId(pub u32);

/// A statement's index in [`Ast::stmts`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StmtId(pub u32);

/// A function's index in [`Ast::functions`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FunctionId(pub u32);

/// An identifier as written, with its span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Delete,
    Void,
    Typeof,
    Plus,
    Minus,
    BitNot,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateOp {
    Increment,
    Decrement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Mul,
    Div,
    Mod,
    Add,
    Sub,
    Shl,
    Shr,
    UShr,
    Lt,
    Gt,
    LtEq,
    GtEq,
    Instanceof,
    In,
    Eq,
    NotEq,
    StrictEq,
    StrictNotEq,
    BitAnd,
    BitXor,
    BitOr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicalOp {
    And,
    Or,
    /// `??`: the right side only when the left is `undefined` or `null`.
    Coalesce,
}

/// `=` (`None`) or a compound assignment's operator (`+=` is `Some(BinaryOp::Add)`).
pub type AssignOp = Option<BinaryOp>;

/// A property name in an object literal.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyKey {
    Ident(String),
    String(Vec<u16>),
    Number(f64),
    /// `[expression]` with [`crate::Options::es2015`]: the key is the expression's value.
    Computed(ExprId),
}

impl PropertyKey {
    /// The key as the string a property is stored under (a number through ToString); empty for
    /// a computed key, whose string is known only at run time.
    pub fn to_utf16(&self) -> Vec<u16> {
        match self {
            PropertyKey::Ident(name) => name.encode_utf16().collect(),
            PropertyKey::String(s) => s.clone(),
            PropertyKey::Number(n) => crate::number::to_string(*n).encode_utf16().collect(),
            PropertyKey::Computed(_) => Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    Init(ExprId),
    Get(FunctionId),
    Set(FunctionId),
    /// A method (`m() {…}`, ECMA-262 2015 section 14.3).
    Method(FunctionId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub key: PropertyKey,
    pub value: PropertyValue,
    pub span: Span,
}

impl Property {
    /// The property is `__proto__: value`, which sets the new object's prototype (ECMA-262 2015
    /// section B.3.1): a data property with a name written out, not computed and not shorthand
    /// (`{__proto__}`, whose value starts where the property does).
    pub fn is_proto_setter(&self, ast: &Ast) -> bool {
        let PropertyValue::Init(value) = self.value else { return false };
        ast.expr_span(value).start != self.span.start
            && matches!(self.key, PropertyKey::Ident(_) | PropertyKey::String(_))
            && self.key.to_utf16().into_iter().eq("__proto__".encode_utf16())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    This,
    Ident(String),
    Null,
    Bool(bool),
    Number(f64),
    String(Vec<u16>),
    RegExp {
        pattern: Vec<u16>,
        flags: String,
    },
    /// An array literal; `None` is a hole (`[1, , 3]`).
    Array(Vec<Option<ExprId>>),
    Object(Vec<Property>),
    Function(FunctionId),
    /// `object.property`.
    Member {
        object: ExprId,
        property: Name,
    },
    /// `object[index]`.
    Index {
        object: ExprId,
        index: ExprId,
    },
    Call {
        callee: ExprId,
        args: Vec<ExprId>,
    },
    New {
        callee: ExprId,
        args: Vec<ExprId>,
    },
    Update {
        op: UpdateOp,
        prefix: bool,
        target: ExprId,
    },
    Unary {
        op: UnaryOp,
        arg: ExprId,
    },
    Binary {
        op: BinaryOp,
        left: ExprId,
        right: ExprId,
    },
    Logical {
        op: LogicalOp,
        left: ExprId,
        right: ExprId,
    },
    Conditional {
        test: ExprId,
        consequent: ExprId,
        alternate: ExprId,
    },
    Assign {
        op: AssignOp,
        target: ExprId,
        value: ExprId,
    },
    /// Comma-separated expressions.
    Sequence(Vec<ExprId>),
    /// A parenthesized expression, kept so a rule about the bare form (a label, a directive) can
    /// tell them apart.
    Paren(ExprId),
    /// An XML literal (`<a b="c">d</a>`), or with `list` an XMLList literal (`<>…</>`).
    XmlLiteral {
        parts: Vec<XmlPart>,
        list: bool,
    },
    /// An E4X name (`@a`, `*`, `ns::a`), the key of an [`Expr::Index`] or the operand of
    /// [`Expr::XmlScopeName`] and [`Expr::XmlDescendants`].
    XmlName {
        attribute: bool,
        qualifier: Option<XmlQualifier>,
        name: XmlNamePart,
    },
    /// An E4X name standing alone (`@a` in a filter): looked up on the nearest XML value in scope
    /// that holds it, and also an assignment, update or `delete` target.
    XmlScopeName(ExprId),
    /// `object..name`.
    XmlDescendants {
        object: ExprId,
        name: ExprId,
    },
    /// `object.(predicate)`.
    XmlFilter {
        object: ExprId,
        predicate: ExprId,
    },
    /// An arrow function.
    Arrow(FunctionId),
    /// A template literal: its pieces (one more than its substitutions) and its substitutions,
    /// in source order.
    Template {
        quasis: Vec<TemplatePiece>,
        exprs: Vec<ExprId>,
    },
    /// A tagged template (`` tag`a${b}c` ``): `tag` called with the template object and the
    /// substitutions.
    TaggedTemplate {
        tag: ExprId,
        quasis: Vec<TemplatePiece>,
        exprs: Vec<ExprId>,
    },
    /// `...expression` in an array literal or an argument list, and as the last of an arrow
    /// function's parenthesized parameters while they are read.
    Spread(ExprId),
    /// An optional chain (`a?.b.c`, `a?.[k]`, `f?.()`): its accesses and calls, the
    /// [`Expr::Optional`] steps among them included. It is `undefined` when one of those steps
    /// finds `undefined` or `null`, and the rest of the chain is skipped.
    OptionalChain(ExprId),
    /// The object or function before `?.` in an [`Expr::OptionalChain`].
    Optional(ExprId),
}

/// A piece of a template literal's text between its substitutions.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplatePiece {
    /// The text with its escapes read.
    pub cooked: Vec<u16>,
    /// The text as written.
    pub raw: Vec<u16>,
}

/// Which lexical declaration a name has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LexKind {
    Let,
    Const,
}

/// A piece of an XML literal, in source order.
#[derive(Clone, Debug, PartialEq)]
pub enum XmlPart {
    /// Markup kept as written.
    Text(Vec<u16>),
    /// `{expression}` in element content: XML values keep their markup, other values are escaped.
    Content(ExprId),
    /// `{expression}` as an attribute's value.
    Attribute(ExprId),
    /// `{expression}` as an element or attribute name, its string checked as an XML name.
    Name(ExprId),
    /// `{expression}` as a run of attributes, its string checked as `name="value"` attributes.
    Attributes(ExprId),
}

/// What stands before `::` in a qualified name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XmlQualifier {
    /// `*::name`: any namespace.
    Any,
    Expr(ExprId),
}

/// The name in an E4X name.
#[derive(Clone, Debug, PartialEq)]
pub enum XmlNamePart {
    Ident(String),
    /// `*`.
    Star,
    /// `[expression]`.
    Computed(ExprId),
}

/// What a declaration, a parameter or a catch clause binds: a name, or with
/// [`crate::Options::es2015`] a destructuring pattern (ECMA-262 11th edition, section 13.3.3).
///
/// A pattern is an [`Expr::Array`] or an [`Expr::Object`] whose targets are [`Expr::Ident`]
/// names or nested patterns; an [`Expr::Assign`] with no operator gives a target its default
/// value, and an [`Expr::Spread`] as the last element of an array pattern is its rest element.
#[derive(Clone, Debug, PartialEq)]
pub enum Binding {
    Name(Name),
    Pattern(ExprId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct VarDecl {
    pub target: Binding,
    pub init: Option<ExprId>,
}

/// What a `for (… in …)` or `for (… of …)` statement assigns each property name or value to.
#[derive(Clone, Debug, PartialEq)]
pub enum ForInTarget {
    /// `for (var x in o)`, with the declaration's initializer if it has one.
    Var(VarDecl),
    /// An assignment target, or with [`crate::Options::es2015`] an assignment pattern.
    Expr(ExprId),
    /// `for (let x in o)` or `for (const x in o)`: a new binding for each name.
    Lexical(LexKind, Binding),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ForInit {
    Var(Vec<VarDecl>),
    Expr(ExprId),
    /// `let` or `const` declarations, in a scope of the loop's own with a new copy of each `let`
    /// binding for each iteration.
    Lexical(LexKind, Vec<VarDecl>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SwitchCase {
    /// `None` for `default`.
    pub test: Option<ExprId>,
    pub body: Vec<StmtId>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CatchClause {
    pub param: Binding,
    pub body: Vec<StmtId>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Var(Vec<VarDecl>),
    /// `let` or `const` declarations (a `const` always has an initializer).
    Lexical(LexKind, Vec<VarDecl>),
    FunctionDecl(FunctionId),
    Expr(ExprId),
    Block(Vec<StmtId>),
    Empty,
    If {
        test: ExprId,
        consequent: StmtId,
        alternate: Option<StmtId>,
    },
    DoWhile {
        body: StmtId,
        test: ExprId,
    },
    While {
        test: ExprId,
        body: StmtId,
    },
    For {
        init: Option<ForInit>,
        test: Option<ExprId>,
        update: Option<ExprId>,
        body: StmtId,
    },
    /// `for (… in …)`, or with `each` the E4X `for each (… in …)`, which assigns the values.
    ForIn {
        target: ForInTarget,
        object: ExprId,
        body: StmtId,
        each: bool,
    },
    /// `for (… of …)` with [`crate::Options::es2015`] (ECMA-262 11th edition, section 13.7.5).
    ForOf {
        target: ForInTarget,
        iterable: ExprId,
        body: StmtId,
    },
    /// `default xml namespace = expression;`.
    DefaultXmlNamespace(ExprId),
    Continue(Option<Name>),
    Break(Option<Name>),
    Return(Option<ExprId>),
    With {
        object: ExprId,
        body: StmtId,
    },
    Switch {
        discriminant: ExprId,
        cases: Vec<SwitchCase>,
    },
    Labeled {
        label: Name,
        body: StmtId,
    },
    Throw(ExprId),
    Try {
        block: Vec<StmtId>,
        handler: Option<CatchClause>,
        finalizer: Option<Vec<StmtId>>,
    },
    Debugger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FunctionKind {
    Declaration,
    Expression,
    Getter,
    Setter,
    /// An arrow function; one with an expression body has a body of one `return` statement.
    Arrow,
    /// A method in an object literal.
    Method,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub kind: FunctionKind,
    pub name: Option<Name>,
    /// The parameters before a rest parameter.
    pub params: Vec<Binding>,
    /// Each parameter's default value, in the order of `params` (empty when none has one).
    pub defaults: Vec<Option<ExprId>>,
    /// The rest parameter (`...name` or `...[pattern]`).
    pub rest: Option<Binding>,
    pub body: Vec<StmtId>,
    /// The function's code is strict mode code: its own directive prologue holds `"use strict"`,
    /// or the code around it is strict.
    pub strict: bool,
    pub span: Span,
}

impl Function {
    /// The parameters are plain names: no pattern, no default value and no rest parameter.
    pub fn simple_params(&self) -> bool {
        self.rest.is_none() && self.defaults.iter().all(Option::is_none) && self.params.iter().all(|p| matches!(p, Binding::Name(_)))
    }
}

/// The arenas of a parsed script.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ast {
    pub exprs: Vec<(Expr, Span)>,
    pub stmts: Vec<(Stmt, Span)>,
    pub functions: Vec<Function>,
}

impl Ast {
    pub fn expr(&self, id: ExprId) -> Option<&Expr> {
        self.exprs.get(id.0 as usize).map(|(e, _)| e)
    }

    pub fn expr_span(&self, id: ExprId) -> Span {
        self.exprs.get(id.0 as usize).map(|(_, s)| *s).unwrap_or_default()
    }

    pub fn stmt(&self, id: StmtId) -> Option<&Stmt> {
        self.stmts.get(id.0 as usize).map(|(s, _)| s)
    }

    pub fn stmt_span(&self, id: StmtId) -> Span {
        self.stmts.get(id.0 as usize).map(|(_, s)| *s).unwrap_or_default()
    }

    pub fn function(&self, id: FunctionId) -> Option<&Function> {
        self.functions.get(id.0 as usize)
    }

    /// The names a binding binds, in source order.
    pub fn bound_names(&self, binding: &Binding) -> Vec<Name> {
        match binding {
            Binding::Name(n) => vec![n.clone()],
            Binding::Pattern(p) => self.pattern_names(*p),
        }
    }

    /// The names a destructuring pattern binds, in source order, walked without recursion.
    pub fn pattern_names(&self, pattern: ExprId) -> Vec<Name> {
        let mut out = Vec::new();
        let mut work = vec![pattern];
        while let Some(id) = work.pop() {
            match self.expr(id) {
                Some(Expr::Ident(n)) => out.push(Name { name: n.clone(), span: self.expr_span(id) }),
                Some(Expr::Array(elements)) => work.extend(elements.iter().rev().flatten().copied()),
                Some(Expr::Object(props)) => work.extend(props.iter().rev().filter_map(|p| match p.value {
                    PropertyValue::Init(e) => Some(e),
                    _ => None,
                })),
                Some(Expr::Assign { target, .. }) => work.push(*target),
                Some(Expr::Spread(arg)) => work.push(*arg),
                _ => {}
            }
        }
        out
    }

    /// The expression with any parentheses around it removed.
    pub fn unparenthesized(&self, mut id: ExprId) -> ExprId {
        while let Some(Expr::Paren(inner)) = self.expr(id) {
            id = *inner;
        }
        id
    }

    pub(crate) fn push_expr(&mut self, expr: Expr, span: Span) -> ExprId {
        self.exprs.push((expr, span));
        ExprId(u32::try_from(self.exprs.len() - 1).unwrap_or(u32::MAX))
    }

    pub(crate) fn push_stmt(&mut self, stmt: Stmt, span: Span) -> StmtId {
        self.stmts.push((stmt, span));
        StmtId(u32::try_from(self.stmts.len() - 1).unwrap_or(u32::MAX))
    }

    pub(crate) fn push_function(&mut self, function: Function) -> FunctionId {
        self.functions.push(function);
        FunctionId(u32::try_from(self.functions.len() - 1).unwrap_or(u32::MAX))
    }
}

/// A parsed script.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub ast: Ast,
    pub body: Vec<StmtId>,
    /// The script is strict mode code (its directive prologue holds `"use strict"`, or it was
    /// parsed with [`crate::Options::strict`]).
    pub strict: bool,
    /// Its preprocessor lines, in source order.
    pub directives: Vec<crate::token::Directive>,
    pub(crate) lines: crate::LineTable,
    /// It was read with [`crate::Options::es2015`].
    pub(crate) es2015: bool,
}

impl Program {
    /// The line and column (both from 1, the column in UTF-16 code units) of a source offset.
    pub fn line_column(&self, offset: u32) -> (u32, u32) {
        self.lines.line_column(offset)
    }

    /// The number of lines in the source: its line terminators plus one for a last line without
    /// one.
    pub fn line_count(&self) -> u32 {
        self.lines.line_count()
    }
}
