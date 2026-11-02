//! The lexer: script text into tokens that cover it exactly once, blanks, line breaks and comments
//! included. It reads ECMAScript 3 with E4X XML literals, the preprocessor lines (`#name …` at the
//! start of a line, and `//@name …` comments there that name a directive), and the later syntax a
//! script may hold (template literals, `=>`, `?.`), which the analysis reports. An XML literal's
//! markup gives tokens of its own, and the code in its braces gives the same tokens as code anywhere
//! else. Each step moves forward through the text, and nothing recurses: an XML literal's elements
//! are counted, and the open parentheses, the open braces, the `?` of conditionals at each bracket
//! level and the levels of XML literals and their code are kept on stacks capped at the nesting
//! limit.

use crate::catalog;
use crate::report::{TOO_DEEP, UNTERMINATED_COMMENT, UNTERMINATED_REGEXP, UNTERMINATED_STRING, UNTERMINATED_TEMPLATE};

/// What a token is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum T {
    /// Blanks: spaces, tabs and other white space.
    Space,
    /// One line break: LF, CR LF, CR, U+2028 or U+2029.
    Newline,
    /// A `//` or `/* */` comment.
    Comment,
    /// A preprocessor line: `#name …` at the start of a line, or a `//@name …` comment there that
    /// names a directive.
    Directive,
    Ident,
    Number,
    /// A `'…'` or `"…"` string literal.
    Str,
    /// A `` `…` `` template literal.
    Template,
    RegExp,
    /// The start of an E4X XML literal (`<a b="1">…</a>`, `<>…</>`, `<!-- … -->`): its markup up to
    /// its end, or up to and with the `{` of the code in it.
    Xml,
    /// More of an XML literal's markup: from the `}` that ends the code in it up to the literal's
    /// end or the next `{`.
    XmlMore,
    Punct,
    /// A character no token starts with, or `#` and a name away from the start of a line.
    Other,
}

/// A token: what it is, its text and the line it starts on (from 1), with the problem it has.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Token<'s> {
    pub kind: T,
    pub text: &'s str,
    pub line: usize,
    pub problem: Option<&'static str>,
    /// A statement may start at the token (a `function` there is a declaration).
    pub stmt: bool,
    /// An expression may start at the token (a `/` there starts a regular expression).
    pub expr: bool,
    /// The byte offset in the text where the token starts.
    pub start: usize,
}

/// The kinds of line break the text has used.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Breaks {
    pub lf: bool,
    pub crlf: bool,
    pub cr: bool,
}

/// Punctuators, longest first: ECMAScript 3's, E4X's (`..`, `.@`, `::`, `@`) and later ones (`=>`,
/// `...`, `?.`, `??`, `**` and the assignments built on them), which the analysis reports.
const PUNCTS: &[&str] = &[
    ">>>=", "===", "!==", ">>>", "<<=", ">>=", "**=", "...", "&&=", "||=", "??=", "==", "!=", "<=", ">=", "&&", "||", "++", "--", "+=", "-=", "*=",
    "/=", "%=", "&=", "|=", "^=", "<<", ">>", "=>", "**", "?.", "??", "::", "..", ".@", "{", "}", "(", ")", "[", "]", ";", ",", "<", ">", "+", "-",
    "*", "/", "%", "&", "|", "^", "!", "~", "?", ":", "=", ".", "@",
];

pub(crate) struct Lexer<'s> {
    src: &'s str,
    pos: usize,
    line: usize,
    /// Only blanks since the last line break (or the start): a `#` here starts a preprocessor line.
    line_start: bool,
    /// An expression may start here: a `/` starts a regular expression and a `<` an XML literal.
    expr: bool,
    /// A statement may start here: a `{` here opens a block. A `{` where only an expression may
    /// start opens an object literal.
    stmt: bool,
    /// The last significant token was `return`, `throw`, `break` or `continue`: a line break here
    /// ends the statement (ECMA-262 3rd edition, section 7.9.1).
    restricted: bool,
    /// The last significant token was `.`, `..`, `.@` or `::`: a name here is a property's, also when
    /// it is a keyword such as `return`.
    member: bool,
    /// The last significant token was `if`, `while`, `for`, `with` or the `each` of `for each`: a
    /// `(` here opens a condition.
    cond: bool,
    /// For each `(` still open, whether it opened a condition, up to `nesting` of them; `deeper`
    /// counts the ones past that.
    parens: Vec<bool>,
    deeper: usize,
    /// For each `{` still open, what it opened, up to `nesting` of them; `deeper_braces` counts the
    /// ones past that.
    braces: Vec<Brace>,
    deeper_braces: usize,
    /// `function` was read and its body's `{` is still to come: whether the function is an
    /// expression (it starts where no statement may).
    function: Option<bool>,
    /// For the text outside any bracket and each bracket still open, how many `?` of a conditional
    /// (`? :`) wait for their `:` there, up to `nesting` levels; `deeper_questions` counts the
    /// levels past that.
    questions: Vec<usize>,
    deeper_questions: usize,
    /// The XML literal being read, its innermost level last: its markup, the code in a `{…}` in
    /// it, an XML literal in that code, and so on. Empty outside an XML literal.
    xml: Vec<Level>,
    /// The line the outermost XML literal being read starts on.
    xml_line: usize,
    /// The most open parentheses kept, and the most levels in `xml`.
    nesting: usize,
    /// The line of the XML literal the text ended in, when it ended in one.
    pub unclosed_xml: Option<usize>,
    pub breaks: Breaks,
    /// One per character read (and per token): the work done.
    pub steps: usize,
}

fn is_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn is_blank(c: char) -> bool {
    !is_break(c) && (c.is_whitespace() || c == '\u{feff}')
}

fn ident_start(c: char) -> bool {
    c == '$' || c == '_' || c.is_ascii_alphabetic() || (!c.is_ascii() && c.is_alphabetic())
}

/// A character a name may hold after its first.
pub(crate) fn ident_part(c: char) -> bool {
    ident_start(c) || c.is_ascii_digit() || (!c.is_ascii() && c.is_alphanumeric())
}

/// After `<`, the start of an XML literal: a name, `>` (a list), `!` (a comment or CDATA), `?` (a
/// processing instruction) or `{` (a computed name).
fn xml_start(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c, '_' | '>' | '!' | '?' | '{')
}

/// Whether an expression may start after a significant token. After a `}`, the brace's kind decides
/// it ([`Lexer::brace`]), after an XML literal's markup [`Lexer::markup_token`] sets it, and after a
/// property's name, a keyword included ([`Lexer::member`]), none starts.
pub(crate) fn expr_after(kind: T, text: &str) -> bool {
    match kind {
        T::Ident => matches!(text, "return" | "typeof" | "instanceof" | "in" | "new" | "delete" | "void" | "throw" | "case" | "do" | "else"),
        T::Number | T::Str | T::Template | T::RegExp => false,
        T::Punct => !matches!(text, ")" | "]" | "++" | "--"),
        _ => true,
    }
}

/// `rest`, the text after `//@`, starts with a directive's name.
fn names_directive(rest: &str) -> bool {
    let end = rest.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(rest.len());
    let name = rest.get(..end).unwrap_or_default();
    catalog::DIRECTIVES.iter().any(|d| d.eq_ignore_ascii_case(name))
}

/// Where in its markup an XML literal is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum At {
    /// The `<` of a tag, comment, CDATA section or processing instruction.
    Markup,
    /// A start tag (`end` false) or an end tag, after its `<`.
    Tag { end: bool },
    /// An element's text.
    Text,
}

/// A level of the XML literal being read.
#[derive(Clone, Copy, Debug)]
enum Level {
    /// The markup of an XML literal, `open` of its elements open.
    Xml { open: usize, at: At },
    /// The code in braces, `braces` of them open.
    Code { braces: usize },
}

/// What a `{` opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Brace {
    /// A block, or a function declaration's body: a statement may start after its `}`.
    Block,
    /// An object literal: a value.
    Object,
    /// A function expression's body: statements inside, and a value after its `}`.
    Function,
}

/// What reading a little more of an XML literal's markup led to.
enum Step {
    /// The markup goes on.
    On,
    /// The literal ends: none of its elements is open.
    Up,
    /// Code in braces starts at the next character, a `{`.
    Code,
    /// The text ended first.
    End,
}

impl<'s> Lexer<'s> {
    /// A lexer of `src` that keeps at most `nesting` open parentheses, open braces, bracket levels
    /// of conditionals and levels of an XML literal ([`crate::Limits::nesting`]).
    pub fn new(src: &'s str, nesting: usize) -> Self {
        Self {
            src,
            pos: 0,
            line: 1,
            line_start: true,
            expr: true,
            stmt: true,
            restricted: false,
            member: false,
            cond: false,
            parens: vec![],
            deeper: 0,
            braces: vec![],
            deeper_braces: 0,
            function: None,
            questions: vec![0],
            deeper_questions: 0,
            xml: vec![],
            xml_line: 0,
            nesting,
            unclosed_xml: None,
            breaks: Breaks::default(),
            steps: 0,
        }
    }

    /// The lines read so far: the line breaks, and one more for a last line without one.
    pub fn lines(&self) -> usize {
        let last = self.src.chars().next_back();
        self.line - usize::from(last.is_none_or(is_break))
    }

    fn rest(&self) -> &'s str {
        self.src.get(self.pos..).unwrap_or_default()
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.rest().chars().nth(n)
    }

    fn starts(&self, s: &str) -> bool {
        self.rest().starts_with(s)
    }

    fn bump(&mut self, c: char) {
        self.pos += c.len_utf8();
        self.steps += 1;
    }

    /// Read `n` ASCII characters.
    fn skip(&mut self, n: usize) {
        self.pos += n;
        self.steps += 1;
    }

    /// A line break here, read: false when there is none.
    fn line_break(&mut self) -> bool {
        match self.peek() {
            Some('\n') => {
                self.pos += 1;
                self.breaks.lf = true;
            }
            Some('\r') if self.starts("\r\n") => {
                self.pos += 2;
                self.breaks.crlf = true;
            }
            Some('\r') => {
                self.pos += 1;
                self.breaks.cr = true;
            }
            Some(c @ ('\u{2028}' | '\u{2029}')) => self.pos += c.len_utf8(),
            _ => return false,
        }
        self.line += 1;
        self.steps += 1;
        true
    }

    /// Up to the next line break.
    fn until_line_break(&mut self) {
        while let Some(c) = self.peek().filter(|&c| !is_break(c)) {
            self.bump(c);
        }
    }

    /// The rest of a name (its first character included): letters, digits, `$`, `_` and `\u`
    /// escapes.
    fn name(&mut self) {
        loop {
            match self.peek() {
                Some('\\') if self.peek_at(1) == Some('u') => {
                    self.skip(2);
                    for _ in 0..4 {
                        match self.peek() {
                            Some(h) if h.is_ascii_hexdigit() => self.bump(h),
                            _ => break,
                        }
                    }
                }
                Some(c) if ident_part(c) => self.bump(c),
                _ => break,
            }
        }
    }

    fn digits(&mut self, ok: fn(char) -> bool) {
        while let Some(c) = self.peek().filter(|&c| ok(c) || c == '_') {
            self.bump(c);
        }
    }

    /// A number: decimal (with a fraction and an exponent), hexadecimal, or the later binary,
    /// octal and `n` (big integer) forms.
    fn number(&mut self) {
        let b = self.rest().as_bytes();
        match (b.first(), b.get(1)) {
            (Some(b'0'), Some(b'x' | b'X')) => {
                self.skip(2);
                self.digits(|c| c.is_ascii_hexdigit());
            }
            (Some(b'0'), Some(b'b' | b'B' | b'o' | b'O')) => {
                self.skip(2);
                self.digits(|c| c.is_ascii_digit());
            }
            _ => {
                self.digits(|c| c.is_ascii_digit());
                if self.peek() == Some('.') {
                    self.bump('.');
                    self.digits(|c| c.is_ascii_digit());
                }
                let signed = matches!(self.peek_at(1), Some('+' | '-')) && self.peek_at(2).is_some_and(|c| c.is_ascii_digit());
                if matches!(self.peek(), Some('e' | 'E')) && (self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) || signed) {
                    self.skip(if signed { 2 } else { 1 });
                    self.digits(|c| c.is_ascii_digit());
                }
            }
        }
        if self.peek() == Some('n') {
            self.bump('n');
        }
    }

    /// A `'…'` or `"…"` string, which ends at its line's end when it isn't closed.
    fn string(&mut self, quote: char) -> Option<&'static str> {
        self.bump(quote);
        loop {
            match self.peek() {
                None => return Some(UNTERMINATED_STRING),
                Some(c) if c == quote => {
                    self.bump(c);
                    return None;
                }
                Some('\\') => {
                    self.bump('\\');
                    // A backslash before a line break carries the string on to the next line.
                    if !self.line_break()
                        && let Some(c) = self.peek()
                    {
                        self.bump(c);
                    }
                }
                Some(c) if is_break(c) => return Some(UNTERMINATED_STRING),
                Some(c) => self.bump(c),
            }
        }
    }

    fn template(&mut self) -> Option<&'static str> {
        self.bump('`');
        loop {
            if self.line_break() {
                continue;
            }
            match self.peek() {
                None => return Some(UNTERMINATED_TEMPLATE),
                Some('`') => {
                    self.bump('`');
                    return None;
                }
                Some('\\') => {
                    self.bump('\\');
                    if !self.line_break()
                        && let Some(c) = self.peek()
                    {
                        self.bump(c);
                    }
                }
                Some(c) => self.bump(c),
            }
        }
    }

    /// A regular expression and its flags, which ends at its line's end when it isn't closed.
    fn regexp(&mut self) -> Option<&'static str> {
        self.bump('/');
        let mut class = false;
        loop {
            match self.peek() {
                None => return Some(UNTERMINATED_REGEXP),
                Some(c) if is_break(c) => return Some(UNTERMINATED_REGEXP),
                Some('\\') => {
                    self.bump('\\');
                    if let Some(c) = self.peek().filter(|&c| !is_break(c)) {
                        self.bump(c);
                    }
                }
                Some('/') if !class => {
                    self.bump('/');
                    break;
                }
                Some(c) => {
                    match c {
                        '[' => class = true,
                        ']' => class = false,
                        _ => {}
                    }
                    self.bump(c);
                }
            }
        }
        while let Some(c) = self.peek().filter(|&c| ident_part(c)) {
            self.bump(c);
        }
        None
    }

    fn block_comment(&mut self) -> Option<&'static str> {
        self.skip(2);
        loop {
            if self.starts("*/") {
                self.skip(2);
                return None;
            }
            if self.line_break() {
                continue;
            }
            match self.peek() {
                Some(c) => self.bump(c),
                None => return Some(UNTERMINATED_COMMENT),
            }
        }
    }

    /// Up to and past `end`: false when the text ends first.
    fn past(&mut self, end: &str) -> bool {
        loop {
            if self.starts(end) {
                self.skip(end.len());
                return true;
            }
            if self.line_break() {
                continue;
            }
            match self.peek() {
                Some(c) => self.bump(c),
                None => return false,
            }
        }
    }

    /// A token of `kind` that starts at `start` on `line`: the markup of the XML literal being
    /// read, up to the literal's end or up to and with the `{` of the code in it. A `{` whose code
    /// would take a level past `nesting` is read as markup, and the token has the problem
    /// [`TOO_DEEP`]. When the text ends first, the literal is left unclosed
    /// ([`Lexer::unclosed_xml`]). `stmt`: a statement may start at the token; `expr`: an expression
    /// may start at it.
    fn markup_token(&mut self, start: usize, line: usize, kind: T, stmt: bool, expr: bool) -> Token<'s> {
        let mut problem = None;
        while let Some(&Level::Xml { mut open, mut at }) = self.xml.last() {
            let step = self.markup(&mut open, &mut at);
            if let Some(level) = self.xml.last_mut() {
                *level = Level::Xml { open, at };
            }
            match step {
                Step::On => {}
                Step::Code if self.xml.len() < self.nesting => {
                    self.bump('{');
                    self.xml.push(Level::Code { braces: 1 });
                    self.open_level();
                    self.expr = true;
                    break;
                }
                Step::Code => {
                    self.bump('{');
                    problem = Some(TOO_DEEP);
                }
                Step::Up => {
                    // An XML literal is a value: no expression starts after it.
                    self.xml.pop();
                    self.expr = false;
                    break;
                }
                Step::End => {
                    self.xml.clear();
                    self.unclosed_xml = Some(self.xml_line);
                    self.expr = false;
                    break;
                }
            }
        }
        // The code in an XML literal's braces is an expression, and so is the literal: no statement
        // starts after either.
        self.line_start = false;
        self.stmt = false;
        self.cond = false;
        self.restricted = false;
        self.member = false;
        Token { kind, text: self.src.get(start..self.pos).unwrap_or_default(), line, problem, stmt, expr, start }
    }

    /// A little more of an XML literal's markup, `open` of its elements open and `at` where in it:
    /// a comment, CDATA section or processing instruction whole, or one character of a tag or of
    /// an element's text.
    fn markup(&mut self, open: &mut usize, at: &mut At) -> Step {
        match *at {
            At::Markup => {
                let read = if self.starts("<!--") {
                    self.past("-->")
                } else if self.starts("<![CDATA[") {
                    self.past("]]>")
                } else if self.starts("<?") {
                    self.past("?>")
                } else {
                    let end = self.starts("</");
                    self.skip(if end { 2 } else { 1 });
                    *at = At::Tag { end };
                    return Step::On;
                };
                *at = At::Text;
                match (read, *open) {
                    (false, _) => Step::End,
                    (true, 0) => Step::Up,
                    (true, _) => Step::On,
                }
            }
            At::Tag { end } => {
                if self.line_break() {
                    return Step::On;
                }
                // `>` ends a tag, and `/>` a tag whose element closes itself.
                let closes_itself = match self.peek() {
                    None => return Step::End,
                    Some('>') => {
                        self.bump('>');
                        false
                    }
                    Some('/') if self.peek_at(1) == Some('>') => {
                        self.skip(2);
                        true
                    }
                    Some(q @ ('"' | '\'')) => {
                        self.bump(q);
                        return if self.past(if q == '"' { "\"" } else { "'" }) { Step::On } else { Step::End };
                    }
                    Some('{') => return Step::Code,
                    Some(c) => {
                        self.bump(c);
                        return Step::On;
                    }
                };
                *open = match (end, closes_itself) {
                    (true, _) => open.saturating_sub(1),
                    (false, false) => open.saturating_add(1),
                    (false, true) => *open,
                };
                *at = At::Text;
                if *open == 0 { Step::Up } else { Step::On }
            }
            At::Text => {
                if self.line_break() {
                    return Step::On;
                }
                match self.peek() {
                    None => Step::End,
                    Some('<') => {
                        *at = At::Markup;
                        Step::On
                    }
                    Some('{') => Step::Code,
                    Some(c) => {
                        self.bump(c);
                        Step::On
                    }
                }
            }
        }
    }

    fn punct(&self) -> Option<&'static str> {
        let rest = self.rest();
        // `?.` before a digit is `?` and a number (`a ?.5 : 1`).
        PUNCTS.iter().copied().find(|p| rest.starts_with(p) && !(*p == "?." && rest.as_bytes().get(2).is_some_and(u8::is_ascii_digit)))
    }

    /// The parentheses after the punctuator `punct`: `(` opens one, a condition when `cond` is set,
    /// and `)` closes the last one open. True when `punct` closes a condition.
    fn paren(&mut self, punct: &str) -> bool {
        match punct {
            "(" if self.parens.len() < self.nesting => self.parens.push(self.cond),
            "(" => self.deeper = self.deeper.saturating_add(1),
            ")" if self.deeper > 0 => self.deeper -= 1,
            ")" => return self.parens.pop().unwrap_or(false),
            _ => {}
        }
        false
    }

    /// The braces after the punctuator `punct`: `{` opens one and `}` closes the last one open. For
    /// `{` and `}`, what the brace opened: the body of the function just read, a block where a
    /// statement may start or no expression may, else an object literal. A brace past `nesting` is
    /// taken as a block's when it closes.
    fn brace(&mut self, punct: &str) -> Option<Brace> {
        match punct {
            "{" => {
                let brace = match self.function.take() {
                    Some(true) => Brace::Function,
                    Some(false) => Brace::Block,
                    None if self.stmt || !self.expr => Brace::Block,
                    None => Brace::Object,
                };
                if self.braces.len() < self.nesting {
                    self.braces.push(brace);
                } else {
                    self.deeper_braces = self.deeper_braces.saturating_add(1);
                }
                Some(brace)
            }
            "}" if self.deeper_braces > 0 => {
                self.deeper_braces -= 1;
                Some(Brace::Block)
            }
            "}" => Some(self.braces.pop().unwrap_or(Brace::Block)),
            _ => None,
        }
    }

    /// A bracket opens: the `?` inside it are counted apart from the ones outside.
    fn open_level(&mut self) {
        if self.questions.len() <= self.nesting {
            self.questions.push(0);
        } else {
            self.deeper_questions = self.deeper_questions.saturating_add(1);
        }
    }

    /// The innermost bracket closes.
    fn close_level(&mut self) {
        if self.deeper_questions > 0 {
            self.deeper_questions -= 1;
        } else if self.questions.len() > 1 {
            self.questions.pop();
        }
    }

    /// The punctuator `punct` at the innermost level: `?` opens a conditional there, and a `:`
    /// there while one is open is that conditional's. True for that `:`.
    fn conditional(&mut self, punct: &str) -> bool {
        if self.deeper_questions > 0 {
            return false;
        }
        let Some(open) = self.questions.last_mut() else { return false };
        match punct {
            "?" => *open = open.saturating_add(1),
            ":" if *open > 0 => {
                *open -= 1;
                return true;
            }
            _ => {}
        }
        false
    }

    /// A line break after the last significant token. After the end of an expression a statement
    /// may start (JavaScript ends the statement at the line break when the next token can't go on
    /// with it), and after `return`, `throw`, `break` and `continue` a new statement starts.
    fn line_ended(&mut self) {
        if std::mem::take(&mut self.restricted) {
            self.expr = true;
            self.stmt = true;
        }
        self.stmt |= !self.expr;
    }
}

impl<'s> Lexer<'s> {
    /// Update expression, statement and nesting state after reading a significant token.
    fn update_context(&mut self, kind: T, text: &str) {
        self.line_start = false;
        // A name after `.`, `..`, `.@` or `::` is a property's, also when it is a keyword: no
        // expression or statement starts after it.
        let property = kind == T::Ident && self.member;
        // After the `)` of a condition comes a statement, which may start with a regular
        // expression or an XML literal.
        let condition = kind == T::Punct && self.paren(text);
        // A statement may also start after the `}` of a block. After the `}` of an object
        // literal or of a function expression's body, `/` divides.
        let brace = if kind == T::Punct { self.brace(text) } else { None };
        self.expr = condition
            || match (text, brace) {
                ("}", Some(brace)) => brace == Brace::Block,
                _ => !property && expr_after(kind, text),
            };
        if kind == T::Ident && !property && text == "function" {
            self.function = Some(!self.stmt);
        }
        if kind == T::Punct {
            match text {
                "(" | "[" | "{" => self.open_level(),
                ")" | "]" | "}" => self.close_level(),
                _ => {}
            }
        }
        let ternary = kind == T::Punct && self.conditional(text);
        // A statement may start inside a block or a function's body, after `;`, `=>`, a
        // preprocessor line, `else`, `do`, `try` and `finally`, and after the `:` of a label or
        // a `case` (not a conditional's, and outside an object literal).
        self.stmt = condition
            || match (text, brace) {
                ("{", Some(brace)) => brace != Brace::Object,
                ("}", Some(brace)) => brace == Brace::Block,
                _ => false,
            }
            || kind == T::Directive
            || (kind == T::Punct && matches!(text, ";" | "=>"))
            || (kind == T::Punct && text == ":" && !ternary && self.braces.last() != Some(&Brace::Object))
            || (kind == T::Ident && !property && matches!(text, "else" | "do" | "try" | "finally"));
        self.cond = kind == T::Ident && !property && (matches!(text, "if" | "while" | "for" | "with") || (text == "each" && self.cond));
        self.restricted = kind == T::Ident && !property && matches!(text, "return" | "throw" | "break" | "continue");
        self.member = kind == T::Punct && matches!(text, "." | ".." | ".@" | "::");
        // The braces inside the code of an XML literal.
        if kind == T::Punct
            && let Some(Level::Code { braces }) = self.xml.last_mut()
        {
            match text {
                "{" => *braces = braces.saturating_add(1),
                "}" => *braces = braces.saturating_sub(1),
                _ => {}
            }
        }
    }
}

impl<'s> Iterator for Lexer<'s> {
    type Item = Token<'s>;

    fn next(&mut self) -> Option<Token<'s>> {
        let (start, line, stmt, expr) = (self.pos, self.line, self.stmt, self.expr);
        let Some(c) = self.peek() else {
            // The text ends in the code of an XML literal.
            if !self.xml.is_empty() {
                self.xml.clear();
                self.unclosed_xml = Some(self.xml_line);
            }
            return None;
        };
        let mut problem = None;
        let kind = if self.line_break() {
            T::Newline
        } else if is_blank(c) {
            while let Some(c) = self.peek().filter(|&c| is_blank(c)) {
                self.bump(c);
            }
            T::Space
        } else if self.starts("//") {
            let directive = self.line_start && self.rest().strip_prefix("//@").is_some_and(names_directive);
            self.until_line_break();
            if directive { T::Directive } else { T::Comment }
        } else if self.starts("/*") {
            problem = self.block_comment();
            T::Comment
        } else if c == '#' && self.line_start {
            self.until_line_break();
            T::Directive
        } else if c == '#' && self.peek_at(1).is_some_and(ident_start) {
            self.bump('#');
            self.name();
            T::Other
        } else if c == '\'' || c == '"' {
            problem = self.string(c);
            T::Str
        } else if c == '`' {
            problem = self.template();
            T::Template
        } else if c == '/' && self.expr {
            problem = self.regexp();
            T::RegExp
        } else if c == '<' && self.expr && self.peek_at(1).is_some_and(xml_start) {
            if self.xml.len() < self.nesting {
                // An XML literal starts, in the script or in the code of another one.
                if self.xml.is_empty() {
                    self.xml_line = line;
                }
                self.xml.push(Level::Xml { open: 0, at: At::Markup });
                return Some(self.markup_token(start, line, T::Xml, stmt, expr));
            }
            // An XML literal more levels deep than are kept: its `<` is read as an operator.
            self.skip(1);
            problem = Some(TOO_DEEP);
            T::Punct
        } else if c == '}' && matches!(self.xml.last(), Some(Level::Code { braces: 1 })) {
            // The `}` that ends the code in an XML literal's braces: the literal's markup goes on.
            self.xml.pop();
            self.close_level();
            self.bump('}');
            return Some(self.markup_token(start, line, T::XmlMore, stmt, expr));
        } else if ident_start(c) || (c == '\\' && self.peek_at(1) == Some('u')) {
            self.name();
            T::Ident
        } else if c.is_ascii_digit() || (c == '.' && self.peek_at(1).is_some_and(|d| d.is_ascii_digit())) {
            self.number();
            T::Number
        } else if let Some(p) = self.punct() {
            self.skip(p.len());
            T::Punct
        } else {
            self.bump(c);
            T::Other
        };
        let text = self.src.get(start..self.pos).unwrap_or_default();
        match kind {
            T::Space => {}
            T::Newline => {
                self.line_start = true;
                self.line_ended();
            }
            // A comment that holds a line break counts as one.
            T::Comment => {
                self.line_start = false;
                if text.contains(is_break) {
                    self.line_ended();
                }
            }
            _ => self.update_context(kind, text),
        }
        Some(Token { kind, text, line, problem, stmt, expr, start })
    }
}
