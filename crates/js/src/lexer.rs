//! The lexer (ECMA-262 5.1 section 7): source text in UTF-16 code units into tokens, with each
//! token's line and column and whether a line terminator comes before it.
//!
//! A `/` starts a division punctuator or a regular expression literal depending on where the
//! grammar is (section 7, the InputElementDiv and InputElementRegExp goals). The lexer reads a
//! token for the goal it is given; the parser reads tokens for the division goal and asks for a
//! token to be read again as a regular expression where an expression starts
//! ([`Lexer::rescan_regexp`]).
//!
//! With [`crate::Options::preprocessor_lines`], a `#name value` line, or a `//@name value` comment
//! that names a directive, at the start of a line (only blanks before it) gives a
//! [`TokenKind::Directive`] token.
//!
//! The lexer moves forward through the source one character at a time and never recurses.

use crate::chars::{hex_value, is_decimal_digit, is_id_part, is_id_start, is_line_terminator, is_octal_digit, is_whitespace, radix_value};
use crate::error::SyntaxError;
use crate::regexp;
use crate::token::{Directive, DirectiveName, Keyword, Punct, Span, Token, TokenKind};

/// Which goal symbol the next token is read for: whether a `/` there divides or starts a regular
/// expression literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goal {
    Div,
    RegExp,
}

const BACKSLASH: u32 = b'\\' as u32;
const SLASH: u32 = b'/' as u32;
const STAR: u32 = b'*' as u32;
const DOT: u32 = b'.' as u32;
const CR: u32 = 0x0D;
const LF: u32 = 0x0A;

/// The lexer's state. It is small and cheap to clone, which is how the parser looks one token
/// ahead.
#[derive(Clone, Debug)]
pub struct Lexer<'s> {
    src: &'s [u16],
    pos: usize,
    /// The current line (from 1) and the offset where it starts.
    line: u32,
    line_start: usize,
    /// Only blanks since the start of the line: a preprocessor line may start here.
    blank_line: bool,
    preprocessor_lines: bool,
    /// The deepest nesting of groups a regular expression literal may have.
    max_depth: usize,
    /// Read the E4X punctuators `..`, `::` and `@`.
    e4x: bool,
}

impl<'s> Lexer<'s> {
    /// A lexer of `src`. `preprocessor_lines` turns on the reading of preprocessor lines;
    /// `max_depth` limits how deeply a regular expression literal's groups may nest.
    pub fn new(src: &'s [u16], preprocessor_lines: bool, max_depth: usize) -> Self {
        Self { src, pos: 0, line: 1, line_start: 0, blank_line: true, preprocessor_lines, max_depth, e4x: false }
    }

    /// The same lexer, reading the E4X punctuators when `e4x` is true.
    pub fn with_e4x(mut self, e4x: bool) -> Self {
        self.e4x = e4x;
        self
    }

    /// Moves to source offset `pos`, at the given line and column (both from 1), after the parser
    /// has read an XML literal up to there.
    pub(crate) fn seek(&mut self, pos: usize, line: u32, column: u32) {
        self.pos = pos;
        self.line = line;
        self.line_start = pos.saturating_sub(column.saturating_sub(1) as usize);
        self.blank_line = false;
    }

    /// The code unit at `pos`, as a code point value.
    fn unit(&self, pos: usize) -> Option<u32> {
        self.src.get(pos).map(|&u| u32::from(u))
    }

    fn peek(&self) -> Option<u32> {
        self.unit(self.pos)
    }

    fn peek_at(&self, n: usize) -> Option<u32> {
        self.unit(self.pos + n)
    }

    /// The code point at `pos` (a surrogate pair read as one) and how many units it takes.
    fn code_point(&self, pos: usize) -> Option<(u32, usize)> {
        let hi = self.unit(pos)?;
        if (0xD800..0xDC00).contains(&hi)
            && let Some(lo) = self.unit(pos + 1).filter(|lo| (0xDC00..0xE000).contains(lo))
        {
            return Some((0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00), 2));
        }
        Some((hi, 1))
    }

    fn column_of(&self, pos: usize) -> u32 {
        u32::try_from(pos.saturating_sub(self.line_start) + 1).unwrap_or(u32::MAX)
    }

    fn offset(pos: usize) -> u32 {
        u32::try_from(pos).unwrap_or(u32::MAX)
    }

    /// An error at `pos` on the current line.
    fn error_at(&self, pos: usize, message: impl Into<String>) -> SyntaxError {
        SyntaxError { message: message.into(), line: self.line, column: self.column_of(pos), offset: Self::offset(pos) }
    }

    /// An error at a position recorded earlier.
    fn error_from(start: (usize, u32, u32), message: impl Into<String>) -> SyntaxError {
        SyntaxError { message: message.into(), line: start.1, column: start.2, offset: Self::offset(start.0) }
    }

    /// Reads a line terminator at `pos` (CR LF as one), counting the line. False when there is
    /// none.
    fn line_terminator(&mut self) -> bool {
        match self.peek() {
            Some(CR) if self.peek_at(1) == Some(LF) => self.pos += 2,
            Some(c) if is_line_terminator(c) => self.pos += 1,
            _ => return false,
        }
        self.line = self.line.saturating_add(1);
        self.line_start = self.pos;
        self.blank_line = true;
        true
    }

    /// The next token, read for `goal`.
    pub fn next_token(&mut self, goal: Goal) -> Result<Token, SyntaxError> {
        let newline_before = match self.skip_trivia()? {
            Trivia::Directive(token) => return Ok(*token),
            Trivia::Skipped { newline } => newline,
        };
        let start = self.pos;
        let (line, column) = (self.line, self.column_of(start));
        let Some(c) = self.peek() else {
            return Ok(self.token(TokenKind::Eof, start, line, column, newline_before));
        };
        self.blank_line = false;
        let mut token = if c == BACKSLASH || is_id_start(c) || self.code_point(start).is_some_and(|(cp, n)| n == 2 && is_id_start(cp)) {
            self.identifier(start, line, column)?
        } else if is_decimal_digit(c) || (c == DOT && self.peek_at(1).is_some_and(is_decimal_digit)) {
            self.number(start, line, column)?
        } else if c == u32::from(b'"') || c == u32::from(b'\'') {
            self.string(start, line, column)?
        } else if c == SLASH && goal == Goal::RegExp {
            self.regexp(start, line, column)?
        } else {
            let punct = self.punct().ok_or_else(|| self.error_at(start, unexpected_character(c)))?;
            self.token(TokenKind::Punct(punct), start, line, column, false)
        };
        // A string continued over a line ends on a new line, but not at its start.
        self.blank_line = false;
        token.newline_before = newline_before;
        Ok(token)
    }

    /// Reads the token `token` again as a regular expression literal: the parser read a `/` or
    /// `/=` where an expression starts.
    pub fn rescan_regexp(&mut self, token: &Token) -> Result<Token, SyntaxError> {
        let start = usize::try_from(token.span.start).unwrap_or(usize::MAX);
        self.pos = start;
        self.line = token.line;
        self.line_start = start.saturating_sub(usize::try_from(token.column.saturating_sub(1)).unwrap_or(0));
        self.blank_line = false;
        let mut regexp = self.regexp(start, token.line, token.column)?;
        regexp.newline_before = token.newline_before;
        Ok(regexp)
    }

    fn token(&self, kind: TokenKind, start: usize, line: u32, column: u32, newline_before: bool) -> Token {
        Token {
            kind,
            span: Span::new(Self::offset(start), Self::offset(self.pos)),
            line,
            column,
            newline_before,
            escaped: false,
            legacy_octal: false,
        }
    }

    /// Skips white space, line terminators and comments, and reads a preprocessor line when one
    /// starts here.
    fn skip_trivia(&mut self) -> Result<Trivia, SyntaxError> {
        let mut newline = false;
        while let Some(c) = self.peek() {
            if is_whitespace(c) {
                self.pos += 1;
            } else if self.line_terminator() {
                newline = true;
            } else if c == SLASH && self.peek_at(1) == Some(SLASH) {
                if self.blank_line && self.preprocessor_lines && self.names_directive(self.pos + 2) {
                    return Ok(Trivia::Directive(Box::new(self.directive("//@", newline))));
                }
                while self.peek().is_some_and(|c| !is_line_terminator(c)) {
                    self.pos += 1;
                }
            } else if c == SLASH && self.peek_at(1) == Some(STAR) {
                newline |= self.block_comment()?;
                // The comment's end is on this line, so a `#` after it is not at the line's start.
                self.blank_line = false;
            } else if c == u32::from(b'#') && self.blank_line && self.preprocessor_lines && self.peek_at(1).is_some_and(is_name_char) {
                return Ok(Trivia::Directive(Box::new(self.directive("#", newline))));
            } else {
                break;
            }
        }
        Ok(Trivia::Skipped { newline })
    }

    /// Skips a `/* */` comment; true when it holds a line terminator.
    fn block_comment(&mut self) -> Result<bool, SyntaxError> {
        let start = (self.pos, self.line, self.column_of(self.pos));
        self.pos += 2;
        let mut newline = false;
        loop {
            match self.peek() {
                None => return Err(Self::error_from(start, "unterminated comment")),
                Some(STAR) if self.peek_at(1) == Some(SLASH) => {
                    self.pos += 2;
                    return Ok(newline);
                }
                Some(_) if self.line_terminator() => newline = true,
                Some(_) => self.pos += 1,
            }
        }
    }

    /// `//@` at `pos - 2` is followed by a directive's name.
    fn names_directive(&self, pos: usize) -> bool {
        if self.unit(pos) != Some(u32::from(b'@')) {
            return false;
        }
        let name: String = self.src[pos + 1..].iter().take_while(|&&u| is_name_char(u32::from(u))).map(|&u| char::from(u as u8)).collect();
        DirectiveName::from_name(&name).is_some()
    }

    /// Reads a preprocessor line written with `form` (`#` or `//@`), up to the end of the line.
    fn directive(&mut self, form: &'static str, newline_before: bool) -> Token {
        let start = self.pos;
        let (line, column) = (self.line, self.column_of(start));
        self.pos += form.encode_utf16().count();
        let name_start = self.pos;
        while self.peek().is_some_and(is_name_char) {
            self.pos += 1;
        }
        let name = String::from_utf16_lossy(&self.src[name_start..self.pos]);
        let value_start = self.pos;
        while self.peek().is_some_and(|c| !is_line_terminator(c)) {
            self.pos += 1;
        }
        let value = String::from_utf16_lossy(&self.src[value_start..self.pos]).trim().to_string();
        let directive = Directive {
            known: DirectiveName::from_name(&name),
            name,
            value,
            form,
            span: Span::new(Self::offset(start), Self::offset(self.pos)),
            line,
        };
        self.blank_line = false;
        self.token(TokenKind::Directive(directive), start, line, column, newline_before)
    }

    /// Reads an identifier or a keyword (section 7.6), `\u` escapes included.
    fn identifier(&mut self, start: usize, line: u32, column: u32) -> Result<Token, SyntaxError> {
        let mut name = String::new();
        let mut escaped = false;
        let mut first = true;
        loop {
            let at = self.pos;
            let (c, len, is_escape) = match self.code_point(at) {
                Some((BACKSLASH, _)) => {
                    let c = self.identifier_escape()?;
                    (c, 0, true)
                }
                Some((c, len)) => (c, len, false),
                None => break,
            };
            let ok = if first { is_id_start(c) } else { is_id_part(c) };
            if !ok {
                if is_escape {
                    return Err(self.error_at(at, "invalid character in identifier escape"));
                }
                break;
            }
            self.pos += len;
            escaped |= is_escape;
            first = false;
            match char::from_u32(c) {
                Some(ch) => name.push(ch),
                None => return Err(self.error_at(at, "invalid character in identifier")),
            }
        }
        let kind = match Keyword::from_word(&name) {
            Some(k) if !escaped => TokenKind::Keyword(k),
            _ => TokenKind::Ident(name),
        };
        let mut token = self.token(kind, start, line, column, false);
        token.escaped = escaped;
        Ok(token)
    }

    /// Reads a `\uXXXX` escape in an identifier and returns its code point.
    fn identifier_escape(&mut self) -> Result<u32, SyntaxError> {
        let at = self.pos;
        if self.peek_at(1) != Some(u32::from(b'u')) {
            return Err(self.error_at(at, "invalid escape in identifier"));
        }
        self.pos += 2;
        self.unicode_escape().ok_or_else(|| self.error_at(at, "invalid Unicode escape sequence"))
    }

    /// After `\u`: four hexadecimal digits, or a code point in braces (`\u{1F600}`, ECMA-262 2015
    /// section 11.8.4) up to U+10FFFF.
    fn unicode_escape(&mut self) -> Option<u32> {
        if self.peek() != Some(u32::from(b'{')) {
            return self.hex_digits(4);
        }
        let mut n = 1;
        let mut value: u32 = 0;
        while let Some(d) = self.peek_at(n).and_then(hex_value) {
            value = value.checked_mul(16)?.checked_add(d).filter(|&v| v <= 0x10FFFF)?;
            n += 1;
        }
        if n == 1 || self.peek_at(n) != Some(u32::from(b'}')) {
            return None;
        }
        self.pos += n + 1;
        Some(value)
    }

    /// Reads `n` hexadecimal digits and returns their value, or `None` (leaving the position) when
    /// fewer follow.
    fn hex_digits(&mut self, n: usize) -> Option<u32> {
        let mut value = 0;
        for i in 0..n {
            value = value * 16 + hex_value(self.peek_at(i)?)?;
        }
        self.pos += n;
        Some(value)
    }

    /// Reads a numeric literal (section 7.8.3, with the legacy octal literals of Annex B).
    fn number(&mut self, start: usize, line: u32, column: u32) -> Result<Token, SyntaxError> {
        let mut legacy_octal = false;
        let value = if self.peek() == Some(u32::from(b'0')) && self.peek_at(1).is_some_and(|c| c == u32::from(b'x') || c == u32::from(b'X')) {
            self.pos += 2;
            self.radix_digits(16).ok_or_else(|| self.error_at(start, "hexadecimal literal without digits"))?
        } else if self.peek() == Some(u32::from(b'0')) && self.peek_at(1).is_some_and(is_decimal_digit) {
            legacy_octal = true;
            self.legacy_octal_or_decimal()
        } else {
            self.decimal()
        };
        if self.code_point(self.pos).is_some_and(|(c, _)| c == BACKSLASH || is_id_start(c) || is_decimal_digit(c)) {
            return Err(self.error_at(self.pos, "identifier starts immediately after numeric literal"));
        }
        let mut token = self.token(TokenKind::Number(value), start, line, column, false);
        token.legacy_octal = legacy_octal;
        Ok(token)
    }

    /// Reads digits in `radix` (8 or 16) and returns their value, or `None` when there are none.
    fn radix_digits(&mut self, radix: u32) -> Option<f64> {
        let begin = self.pos;
        while self.peek().and_then(hex_value).is_some_and(|d| d < radix) {
            self.pos += 1;
        }
        radix_value(self.src[begin..self.pos].iter().filter_map(|&u| hex_value(u32::from(u))), radix)
    }

    /// After `0` and a digit: an octal literal when every digit is octal, else a decimal literal
    /// whose integer part starts with `0` (`08`, `09.5`).
    fn legacy_octal_or_decimal(&mut self) -> f64 {
        let begin = self.pos;
        let mut end = begin;
        while self.unit(end).is_some_and(is_decimal_digit) {
            end += 1;
        }
        if self.src[begin..end].iter().all(|&u| is_octal_digit(u32::from(u))) {
            self.pos += 1;
            return self.radix_digits(8).unwrap_or(0.0);
        }
        self.decimal()
    }

    /// Reads a decimal literal: digits, an optional fraction and an optional exponent.
    fn decimal(&mut self) -> f64 {
        let mut text = String::new();
        self.push_digits(&mut text);
        if text.is_empty() {
            text.push('0');
        }
        if self.peek() == Some(DOT) {
            self.pos += 1;
            text.push('.');
            let before = text.len();
            self.push_digits(&mut text);
            if text.len() == before {
                text.push('0');
            }
        }
        if self.peek().is_some_and(|c| c == u32::from(b'e') || c == u32::from(b'E')) {
            let sign = self.peek_at(1).filter(|&c| c == u32::from(b'+') || c == u32::from(b'-'));
            let digit_at = if sign.is_some() { 2 } else { 1 };
            if self.peek_at(digit_at).is_some_and(is_decimal_digit) {
                self.pos += digit_at;
                text.push('e');
                if sign == Some(u32::from(b'-')) {
                    text.push('-');
                }
                self.push_digits(&mut text);
            }
        }
        text.parse().unwrap_or(f64::NAN)
    }

    fn push_digits(&mut self, text: &mut String) {
        while let Some(c) = self.peek().filter(|&c| is_decimal_digit(c)) {
            text.push(char::from(c as u8));
            self.pos += 1;
        }
    }

    /// Reads a string literal (section 7.8.4, with the octal escapes of Annex B) and returns its
    /// value.
    fn string(&mut self, start: usize, line: u32, column: u32) -> Result<Token, SyntaxError> {
        let quote = self.peek().unwrap_or_default();
        self.pos += 1;
        let mut value = Vec::new();
        let mut legacy_octal = false;
        loop {
            match self.peek() {
                None => return Err(Self::error_from((start, line, column), "unterminated string literal")),
                // U+2028 and U+2029 may stand in a string, as ECMA-262 2019 allows; they still
                // start a new line for positions.
                Some(c @ (0x2028 | 0x2029)) => {
                    value.push(c as u16);
                    self.line_terminator();
                }
                Some(c) if is_line_terminator(c) => {
                    return Err(Self::error_from((start, line, column), "unterminated string literal"));
                }
                Some(c) if c == quote => {
                    self.pos += 1;
                    break;
                }
                Some(BACKSLASH) => legacy_octal |= self.string_escape(&mut value)?,
                Some(c) => {
                    value.push(c as u16);
                    self.pos += 1;
                }
            }
        }
        let mut token = self.token(TokenKind::String(value), start, line, column, false);
        token.legacy_octal = legacy_octal;
        Ok(token)
    }

    /// Reads an escape in a string literal into `value`; true for an octal escape (`\1`, `\08`)
    /// or `\8` and `\9`.
    fn string_escape(&mut self, value: &mut Vec<u16>) -> Result<bool, SyntaxError> {
        let at = self.pos;
        self.pos += 1;
        let Some(c) = self.peek() else {
            return Err(self.error_at(at, "unterminated string literal"));
        };
        if self.line_terminator() {
            return Ok(false);
        }
        let single = match char::from_u32(c) {
            Some('b') => Some(0x08),
            Some('t') => Some(0x09),
            Some('n') => Some(0x0A),
            Some('v') => Some(0x0B),
            Some('f') => Some(0x0C),
            Some('r') => Some(0x0D),
            _ => None,
        };
        if let Some(unit) = single {
            value.push(unit);
            self.pos += 1;
            return Ok(false);
        }
        match char::from_u32(c) {
            Some('x') => {
                self.pos += 1;
                let unit = self.hex_digits(2).ok_or_else(|| self.error_at(at, "invalid hexadecimal escape sequence"))?;
                value.push(unit as u16);
                Ok(false)
            }
            Some('u') => {
                self.pos += 1;
                let code_point = self.unicode_escape().ok_or_else(|| self.error_at(at, "invalid Unicode escape sequence"))?;
                match char::from_u32(code_point) {
                    Some(ch) if code_point > 0xFFFF => value.extend(ch.encode_utf16(&mut [0; 2]).iter()),
                    _ => value.push(code_point as u16),
                }
                Ok(false)
            }
            Some('0') if !self.peek_at(1).is_some_and(is_decimal_digit) => {
                self.pos += 1;
                value.push(0);
                Ok(false)
            }
            Some('0'..='7') => {
                value.push(self.octal_escape(c));
                Ok(true)
            }
            Some('8' | '9') => {
                self.pos += 1;
                value.push(c as u16);
                Ok(true)
            }
            _ => {
                self.pos += 1;
                value.push(c as u16);
                Ok(false)
            }
        }
    }

    /// Reads an octal escape whose first digit is `first`: up to three digits when it is 0 to 3,
    /// up to two when it is 4 to 7.
    fn octal_escape(&mut self, first: u32) -> u16 {
        let max = if first <= u32::from(b'3') { 3 } else { 2 };
        let mut value = 0;
        let mut n = 0;
        while n < max
            && let Some(d) = self.peek().filter(|&d| is_octal_digit(d))
        {
            value = value * 8 + (d - u32::from(b'0'));
            self.pos += 1;
            n += 1;
        }
        value as u16
    }

    /// Reads a regular expression literal (section 7.8.5) and checks its flags and pattern.
    fn regexp(&mut self, start: usize, line: u32, column: u32) -> Result<Token, SyntaxError> {
        let at = (start, line, column);
        self.pos = start + 1;
        let mut in_class = false;
        loop {
            match self.peek() {
                None => return Err(Self::error_from(at, "unterminated regular expression literal")),
                Some(c) if is_line_terminator(c) => return Err(Self::error_from(at, "unterminated regular expression literal")),
                Some(BACKSLASH) => {
                    if self.peek_at(1).is_none_or(is_line_terminator) {
                        return Err(Self::error_from(at, "unterminated regular expression literal"));
                    }
                    self.pos += 2;
                }
                Some(c) if c == u32::from(b'[') => {
                    in_class = true;
                    self.pos += 1;
                }
                Some(c) if c == u32::from(b']') => {
                    in_class = false;
                    self.pos += 1;
                }
                Some(SLASH) if !in_class => break,
                Some(_) => self.pos += 1,
            }
        }
        let pattern = self.src[start + 1..self.pos].to_vec();
        self.pos += 1;
        let mut flags = String::new();
        while let Some((c, len)) = self.code_point(self.pos) {
            if c == BACKSLASH {
                return Err(Self::error_from(at, "invalid regular expression flags"));
            }
            if !is_id_part(c) {
                break;
            }
            flags.push(char::from_u32(c).unwrap_or('\u{fffd}'));
            self.pos += len;
        }
        if !valid_flags(&flags) {
            return Err(Self::error_from(at, "invalid regular expression flags"));
        }
        if let Err(message) = regexp::check(&pattern, self.max_depth) {
            return Err(Self::error_from(at, format!("invalid regular expression: {message}")));
        }
        Ok(self.token(TokenKind::RegExp { pattern, flags }, start, line, column, false))
    }

    /// Reads the longest punctuator that starts here.
    fn punct(&mut self) -> Option<Punct> {
        let rest = &self.src[self.pos..];
        for (spelling, punct) in Punct::ALL {
            if !self.e4x && matches!(punct, Punct::DotDot | Punct::ColonColon | Punct::At) {
                continue;
            }
            let bytes = spelling.as_bytes();
            if rest.len() >= bytes.len() && rest.iter().zip(bytes).all(|(&u, &b)| u == u16::from(b)) {
                self.pos += bytes.len();
                return Some(*punct);
            }
        }
        None
    }
}

/// What skipping the blanks and comments before a token found.
enum Trivia {
    Skipped { newline: bool },
    Directive(Box<Token>),
}

/// A character of a directive's name.
fn is_name_char(c: u32) -> bool {
    c < 0x80 && (char::from(c as u8).is_ascii_alphanumeric() || c == u32::from(b'_'))
}

/// The flags `g`, `i` and `m`, each at most once (section 15.10.4.1).
fn valid_flags(flags: &str) -> bool {
    let mut seen = [false; 3];
    flags.chars().all(|c| {
        let i = match c {
            'g' => 0,
            'i' => 1,
            'm' => 2,
            _ => return false,
        };
        !std::mem::replace(&mut seen[i], true)
    })
}

fn unexpected_character(c: u32) -> String {
    match char::from_u32(c) {
        Some(ch) if !ch.is_control() => format!("unexpected character '{ch}'"),
        _ => format!("unexpected character U+{c:04X}"),
    }
}
