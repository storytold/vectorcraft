//! Tokens: what the lexer gives the parser.

/// A range of the source in UTF-16 code units, `start` included and `end` not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// The span from the start of `self` to the end of `other`.
    pub fn to(self, other: Span) -> Span {
        Span { start: self.start, end: other.end.max(self.start) }
    }
}

/// The reserved words of ECMA-262 5.1 section 7.6.1 that are reserved in all code, with the
/// literals `null`, `true` and `false`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Keyword {
    Break,
    Case,
    Catch,
    Class,
    Const,
    Continue,
    Debugger,
    Default,
    Delete,
    Do,
    Else,
    Enum,
    Export,
    Extends,
    False,
    Finally,
    For,
    Function,
    If,
    Import,
    In,
    Instanceof,
    New,
    Null,
    Return,
    Super,
    Switch,
    This,
    Throw,
    True,
    Try,
    Typeof,
    Var,
    Void,
    While,
    With,
}

impl Keyword {
    /// The keyword spelled `s`, if `s` is one.
    pub fn from_word(s: &str) -> Option<Keyword> {
        use Keyword as K;
        Some(match s {
            "break" => K::Break,
            "case" => K::Case,
            "catch" => K::Catch,
            "class" => K::Class,
            "const" => K::Const,
            "continue" => K::Continue,
            "debugger" => K::Debugger,
            "default" => K::Default,
            "delete" => K::Delete,
            "do" => K::Do,
            "else" => K::Else,
            "enum" => K::Enum,
            "export" => K::Export,
            "extends" => K::Extends,
            "false" => K::False,
            "finally" => K::Finally,
            "for" => K::For,
            "function" => K::Function,
            "if" => K::If,
            "import" => K::Import,
            "in" => K::In,
            "instanceof" => K::Instanceof,
            "new" => K::New,
            "null" => K::Null,
            "return" => K::Return,
            "super" => K::Super,
            "switch" => K::Switch,
            "this" => K::This,
            "throw" => K::Throw,
            "true" => K::True,
            "try" => K::Try,
            "typeof" => K::Typeof,
            "var" => K::Var,
            "void" => K::Void,
            "while" => K::While,
            "with" => K::With,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        use Keyword as K;
        match self {
            K::Break => "break",
            K::Case => "case",
            K::Catch => "catch",
            K::Class => "class",
            K::Const => "const",
            K::Continue => "continue",
            K::Debugger => "debugger",
            K::Default => "default",
            K::Delete => "delete",
            K::Do => "do",
            K::Else => "else",
            K::Enum => "enum",
            K::Export => "export",
            K::Extends => "extends",
            K::False => "false",
            K::Finally => "finally",
            K::For => "for",
            K::Function => "function",
            K::If => "if",
            K::Import => "import",
            K::In => "in",
            K::Instanceof => "instanceof",
            K::New => "new",
            K::Null => "null",
            K::Return => "return",
            K::Super => "super",
            K::Switch => "switch",
            K::This => "this",
            K::Throw => "throw",
            K::True => "true",
            K::Try => "try",
            K::Typeof => "typeof",
            K::Var => "var",
            K::Void => "void",
            K::While => "while",
            K::With => "with",
        }
    }
}

/// The words reserved only in strict mode code (ECMA-262 5.1 section 7.6.1.2).
pub fn strict_reserved(name: &str) -> bool {
    matches!(name, "implements" | "interface" | "let" | "package" | "private" | "protected" | "public" | "static" | "yield")
}

/// The punctuators of ECMA-262 5.1 sections 7.7 and 7.8.5 (division).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Punct {
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dot,
    Semicolon,
    Comma,
    Lt,
    Gt,
    LtEq,
    GtEq,
    EqEq,
    NotEq,
    EqEqEq,
    NotEqEq,
    Plus,
    Minus,
    Star,
    Percent,
    PlusPlus,
    MinusMinus,
    Shl,
    Shr,
    UShr,
    Amp,
    Pipe,
    Caret,
    Bang,
    Tilde,
    AmpAmp,
    PipePipe,
    Question,
    Colon,
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    PercentEq,
    ShlEq,
    ShrEq,
    UShrEq,
    AmpEq,
    PipeEq,
    CaretEq,
    Slash,
    SlashEq,
    /// `..`, read only with [`crate::Options::e4x`].
    DotDot,
    /// `::`, read only with [`crate::Options::e4x`].
    ColonColon,
    /// `@`, read only with [`crate::Options::e4x`].
    At,
}

impl Punct {
    /// Punctuators by spelling, longest first, so the first that matches is the longest.
    pub(crate) const ALL: &'static [(&'static str, Punct)] = &[
        (">>>=", Punct::UShrEq),
        ("===", Punct::EqEqEq),
        ("!==", Punct::NotEqEq),
        (">>>", Punct::UShr),
        ("<<=", Punct::ShlEq),
        (">>=", Punct::ShrEq),
        ("<=", Punct::LtEq),
        (">=", Punct::GtEq),
        ("==", Punct::EqEq),
        ("!=", Punct::NotEq),
        ("++", Punct::PlusPlus),
        ("--", Punct::MinusMinus),
        ("<<", Punct::Shl),
        (">>", Punct::Shr),
        ("&&", Punct::AmpAmp),
        ("||", Punct::PipePipe),
        ("+=", Punct::PlusEq),
        ("-=", Punct::MinusEq),
        ("*=", Punct::StarEq),
        ("%=", Punct::PercentEq),
        ("&=", Punct::AmpEq),
        ("|=", Punct::PipeEq),
        ("^=", Punct::CaretEq),
        ("/=", Punct::SlashEq),
        ("..", Punct::DotDot),
        ("::", Punct::ColonColon),
        ("@", Punct::At),
        ("{", Punct::LBrace),
        ("}", Punct::RBrace),
        ("(", Punct::LParen),
        (")", Punct::RParen),
        ("[", Punct::LBracket),
        ("]", Punct::RBracket),
        (".", Punct::Dot),
        (";", Punct::Semicolon),
        (",", Punct::Comma),
        ("<", Punct::Lt),
        (">", Punct::Gt),
        ("+", Punct::Plus),
        ("-", Punct::Minus),
        ("*", Punct::Star),
        ("%", Punct::Percent),
        ("&", Punct::Amp),
        ("|", Punct::Pipe),
        ("^", Punct::Caret),
        ("!", Punct::Bang),
        ("~", Punct::Tilde),
        ("?", Punct::Question),
        (":", Punct::Colon),
        ("=", Punct::Eq),
        ("/", Punct::Slash),
    ];

    pub fn as_str(self) -> &'static str {
        Punct::ALL.iter().find(|(_, p)| *p == self).map_or("", |(s, _)| s)
    }
}

/// A preprocessor line's directive: `#include`, `#includepath`, `#script`, `#strict`, `#target`
/// and `#targetengine`, as the public JavaScript Tools Guide lists them
/// (<https://extendscript.docsforadobe.dev/extendscript-tools-features/preprocessor-directives/>).
/// Only the names are taken from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DirectiveName {
    Include,
    IncludePath,
    Script,
    Strict,
    Target,
    TargetEngine,
}

impl DirectiveName {
    /// The directive named `name`, compared without case.
    pub fn from_name(name: &str) -> Option<DirectiveName> {
        const NAMES: [(&str, DirectiveName); 6] = [
            ("include", DirectiveName::Include),
            ("includepath", DirectiveName::IncludePath),
            ("script", DirectiveName::Script),
            ("strict", DirectiveName::Strict),
            ("target", DirectiveName::Target),
            ("targetengine", DirectiveName::TargetEngine),
        ];
        NAMES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, d)| *d)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DirectiveName::Include => "include",
            DirectiveName::IncludePath => "includepath",
            DirectiveName::Script => "script",
            DirectiveName::Strict => "strict",
            DirectiveName::Target => "target",
            DirectiveName::TargetEngine => "targetengine",
        }
    }
}

/// A preprocessor line: `#name value` at the start of a line, or a `//@name value` comment there
/// that names a directive. It is read and kept, never run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directive {
    /// The directive, when the name is one of the six (`#name` lines with another name are kept
    /// with `None`).
    pub known: Option<DirectiveName>,
    /// The name as written.
    pub name: String,
    /// The text after the name, with blanks around it removed.
    pub value: String,
    /// `#` or `//@`.
    pub form: &'static str,
    pub span: Span,
    pub line: u32,
}

/// What a token is.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    /// The end of the source.
    Eof,
    /// An identifier, the strict-mode reserved words included. `escaped` tokens held a `\u` escape.
    Ident(String),
    Keyword(Keyword),
    Punct(Punct),
    /// A numeric literal's value.
    Number(f64),
    /// A string literal's value in UTF-16 code units.
    String(Vec<u16>),
    /// A regular expression literal: its pattern and flags as written.
    RegExp {
        pattern: Vec<u16>,
        flags: String,
    },
    /// A preprocessor line.
    Directive(Directive),
    /// An XML or XMLList literal; the tokens of its embedded expressions follow it.
    XmlLiteral,
}

/// A token, with where it is and the facts the grammar's rules need about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    /// The line (from 1) and column (from 1, in UTF-16 code units) the token starts at.
    pub line: u32,
    pub column: u32,
    /// A line terminator comes between the previous token and this one (automatic semicolon
    /// insertion and the restricted productions use it).
    pub newline_before: bool,
    /// An identifier written with a `\u` escape.
    pub escaped: bool,
    /// A legacy octal numeric literal (`017`, `08`) or a string with an octal escape (`"\1"`), which
    /// strict mode code does not allow.
    pub legacy_octal: bool,
}

impl Token {
    pub fn is_punct(&self, p: Punct) -> bool {
        self.kind == TokenKind::Punct(p)
    }

    pub fn is_keyword(&self, k: Keyword) -> bool {
        self.kind == TokenKind::Keyword(k)
    }

    /// The identifier's name, for an identifier token.
    pub fn ident(&self) -> Option<&str> {
        match &self.kind {
            TokenKind::Ident(name) => Some(name),
            _ => None,
        }
    }

    /// The name this token gives when it stands where any identifier name may, as a property's
    /// name after `.` or in an object literal: identifiers, keywords and the literals `null`,
    /// `true` and `false`.
    pub fn identifier_name(&self) -> Option<&str> {
        match &self.kind {
            TokenKind::Ident(name) => Some(name),
            TokenKind::Keyword(k) => Some(k.as_str()),
            _ => None,
        }
    }
}
