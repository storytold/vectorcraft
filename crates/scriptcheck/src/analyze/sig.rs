//! The significant tokens the analysis reads, and the checks on them that both passes share.

use crate::lex::{T, Token};

/// A significant token: what it is, its text, its line, and whether a line break comes before it.
#[derive(Clone, Copy, Debug)]
pub(super) struct Sig<'s> {
    pub(super) kind: T,
    pub(super) text: &'s str,
    pub(super) line: usize,
    pub(super) nl: bool,
    /// The token ends a File or Folder object: the `)` that ends a call of `File` or `Folder` (with
    /// or without `new`) or of `File.openDialog`, `File.saveDialog` or `Folder.selectDialog`, the
    /// name in `Folder.temp` and the other places of [`catalog::FOLDER_PLACES`], or the `)` of
    /// grouping parentheses that hold only such an object or a variable holding one (`(f)`).
    pub(super) file: bool,
    /// A statement may start at the token ([`crate::lex::Token::stmt`]).
    pub(super) stmt: bool,
    /// An expression may start at the token ([`crate::lex::Token::expr`]).
    pub(super) expr: bool,
    /// The byte offset in the text where the token starts.
    pub(super) start: usize,
}

impl<'s> Sig<'s> {
    /// The significant token `t`, after a line break when `nl` is set.
    pub(super) fn of(t: &Token<'s>, nl: bool) -> Self {
        Sig { kind: t.kind, text: t.text, line: t.line, nl, file: false, stmt: t.stmt, expr: t.expr, start: t.start }
    }

    pub(super) fn is(&self, punct: &str) -> bool {
        self.kind == T::Punct && self.text == punct
    }

    pub(super) fn word(&self, word: &str) -> bool {
        self.kind == T::Ident && self.text == word
    }
}

/// What a token means to the passes that follow the significant tokens.
pub(super) enum Read<'s> {
    /// A blank, or a comment or line break (remembered for the next token).
    Skip,
    /// A preprocessor line.
    Directive,
    Token(Sig<'s>),
}

/// Whether a line break came since the last significant token.
#[derive(Default)]
pub(super) struct LineBreaks(bool);

impl LineBreaks {
    pub(super) fn read<'s>(&mut self, t: &Token<'s>) -> Read<'s> {
        match t.kind {
            T::Space => Read::Skip,
            // A comment with a line break in it ends a line, as a line break does.
            T::Comment => {
                self.0 |= t.text.contains(['\n', '\r', '\u{2028}', '\u{2029}']);
                Read::Skip
            }
            T::Newline => {
                self.0 = true;
                Read::Skip
            }
            T::Directive => {
                self.0 = true;
                Read::Directive
            }
            _ => Read::Token(Sig::of(t, std::mem::take(&mut self.0))),
        }
    }
}

pub(super) fn is(s: Option<Sig>, punct: &str) -> bool {
    s.is_some_and(|s| s.is(punct))
}

pub(super) fn word(s: Option<Sig>, w: &str) -> bool {
    s.is_some_and(|s| s.word(w))
}

/// A token after which a name is a member, not a variable: `.`, `.@`, `..` and `::`.
pub(super) fn member_mark(s: Option<Sig>) -> bool {
    s.is_some_and(|s| s.kind == T::Punct && matches!(s.text, "." | ".@" | ".." | "::"))
}

/// A token after which an expression goes on, also past a line break: a punctuator other than `)`,
/// `]`, `}`, `++`, `--` and `;`, or `in`, `instanceof`, `typeof`, `new`, `delete` or `void`.
pub(super) fn goes_on_after(s: Option<Sig>) -> bool {
    s.is_some_and(|s| match s.kind {
        T::Punct => !matches!(s.text, ")" | "]" | "}" | "++" | "--" | ";"),
        T::Ident => matches!(s.text, "in" | "instanceof" | "typeof" | "new" | "delete" | "void"),
        _ => false,
    })
}

/// A token that goes on with the expression on the line before when it starts a line: a
/// punctuator other than `{`, `}`, `;`, `++`, `--`, `!` and `~`, or `in` or `instanceof`.
pub(super) fn goes_on_with(t: &Sig) -> bool {
    match t.kind {
        T::Punct => !matches!(t.text, "{" | "}" | ";" | "++" | "--" | "!" | "~"),
        T::Ident => matches!(t.text, "in" | "instanceof"),
        _ => false,
    }
}

/// `t` is the `in` of `for (var name in …)`, which ends the `var` statement's declarations: it comes
/// right after a name the statement declares (ECMA-262 3rd edition, section 12.6.4). Outside a `for`
/// statement's parentheses, `in` can't stand there.
pub(super) fn for_in(t: &Sig, h0: Option<Sig>, h1: Option<Sig>) -> bool {
    t.word("in") && h0.is_some_and(|s| s.kind == T::Ident) && (word(h1, "var") || is(h1, ","))
}

/// A token after which the value before it is complete: `;`, `,`, `:` (a conditional expression's
/// colon, ECMA-262 3rd edition, section 11.12), `)`, `]` or `}`, or a token on a new line that
/// doesn't go on with the expression.
pub(super) fn ends_value(t: &Sig) -> bool {
    (t.nl && !goes_on_with(t)) || (t.kind == T::Punct && matches!(t.text, ";" | "," | ":" | ")" | "]" | "}"))
}

/// Punctuators of later syntax.
pub(super) const NEWER: &[&str] = &["=>", "...", "?.", "??", "**", "**=", "??=", "&&=", "||="];

/// The compound assignment operators (ECMA-262 3rd edition, section 11.13.2): each gives the name a
/// number or a string worked out from its old value.
pub(super) const COMPOUND: &[&str] = &["*=", "/=", "%=", "+=", "-=", "<<=", ">>=", ">>>=", "&=", "^=", "|="];

/// A number in a later form: binary, octal (`0b1`, `0o7`), with separators (`1_000`) or big (`1n`).
pub(super) fn newer_number(text: &str) -> bool {
    let b = text.as_bytes();
    text.contains('_') || (b.len() > 1 && b.last() == Some(&b'n')) || matches!((b.first(), b.get(1)), (Some(b'0'), Some(b'b' | b'B' | b'o' | b'O')))
}

/// A string literal's value, its quotes removed and its escapes undone (`\n`, `\xHH`, `\uHHHH`,
/// octal escapes, a backslash before a line break), up to `max` characters.
pub(super) fn unquote(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let body = match chars.next() {
        Some(q @ ('"' | '\'')) => chars.as_str().strip_suffix(q).unwrap_or(chars.as_str()),
        _ => text,
    };
    let mut out = String::new();
    let mut n = 0;
    let mut it = body.chars().peekable();
    while n < max
        && let Some(c) = it.next()
    {
        let c = if c == '\\' {
            match it.next() {
                None => break,
                Some('n') => '\n',
                Some('t') => '\t',
                Some('r') => '\r',
                Some('b') => '\u{8}',
                Some('f') => '\u{c}',
                Some('v') => '\u{b}',
                // A legacy octal escape (ECMA-262 3rd edition, annex B.1.2): up to three digits
                // after `\0` to `\3`, two after `\4` to `\7`.
                Some(d @ '0'..='7') => {
                    let mut v = d.to_digit(8).unwrap_or_default();
                    for _ in 0..if v <= 3 { 2 } else { 1 } {
                        match it.peek().and_then(|o| o.to_digit(8)) {
                            Some(o) => {
                                v = v * 8 + o;
                                it.next();
                            }
                            None => break,
                        }
                    }
                    char::from_u32(v).unwrap_or('\u{fffd}')
                }
                Some(x @ ('x' | 'u')) => {
                    let digits = if x == 'x' { 2 } else { 4 };
                    let mut v = 0u32;
                    for _ in 0..digits {
                        match it.peek().and_then(|d| d.to_digit(16)) {
                            Some(d) => {
                                v = v * 16 + d;
                                it.next();
                            }
                            None => break,
                        }
                    }
                    char::from_u32(v).unwrap_or('\u{fffd}')
                }
                Some('\r') => {
                    if it.peek() == Some(&'\n') {
                        it.next();
                    }
                    continue;
                }
                Some('\n' | '\u{2028}' | '\u{2029}') => continue,
                Some(c) => c,
            }
        } else {
            c
        };
        out.push(c);
        n += 1;
    }
    out
}
