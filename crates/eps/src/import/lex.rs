//! The scanner: PostScript source into objects (numbers, names, strings, procedures), with the
//! position the program's own data (`currentfile`) is read from.

use std::rc::Rc;

use super::obj::{Obj, PsError, Res};
use crate::ps;

/// Deepest procedure nesting the scanner reads.
const MAX_NESTING: usize = 256;

pub(crate) struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    /// The whitespace character that ended the last token is still unread (data the program
    /// reads from itself starts after it).
    fresh: bool,
    /// The last object read has immediately evaluated names (`//name`) in it, read as executable
    /// names starting with [`IMMEDIATE`] for the interpreter to replace by their values.
    pub immediate: bool,
}

/// How an immediately evaluated name (`//name`) is read, until the interpreter replaces it.
pub(crate) const IMMEDIATE: &str = "//";

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

fn syntax<T>(what: &str) -> Res<T> {
    Err(PsError::Ps("syntaxerror", what.to_string()))
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a [u8]) -> Self {
        Self { src, pos: 0, fresh: false, immediate: false }
    }

    /// What is left to read.
    pub fn rest(&self) -> &'a [u8] {
        self.src.get(self.pos..).unwrap_or_default()
    }

    /// What is left to read as data: after the whitespace that ended the last token (a CR LF
    /// pair counts as one).
    pub fn data(&mut self) -> &'a [u8] {
        if std::mem::take(&mut self.fresh) {
            match self.peek() {
                Some(b'\r') => {
                    self.pos += 1;
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                }
                Some(b) if is_space(b) => self.pos += 1,
                _ => {}
            }
        }
        self.rest()
    }

    /// Skip `n` bytes (the data a filter or `readstring` took).
    pub fn advance(&mut self, n: usize) {
        self.fresh = false;
        self.pos = self.pos.saturating_add(n).min(self.src.len());
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn skip_space(&mut self) {
        while let Some(b) = self.peek() {
            if is_space(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(c) = self.peek() {
                    if c == b'\n' || c == b'\r' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// The next object at the top level (`None` at the end of the file).
    pub fn next(&mut self) -> Res<Option<Obj>> {
        self.immediate = false;
        let t = self.token(0);
        self.fresh = true;
        t
    }

    fn token(&mut self, depth: usize) -> Res<Option<Obj>> {
        self.skip_space();
        let Some(b) = self.peek() else { return Ok(None) };
        self.pos += 1;
        Ok(Some(match b {
            b'(' => Obj::string(self.string()?),
            b'<' => match self.peek() {
                Some(b'<') => {
                    self.pos += 1;
                    Obj::Exec(Rc::from("<<"))
                }
                Some(b'~') => {
                    self.pos += 1;
                    Obj::string(self.ascii85()?)
                }
                _ => Obj::string(self.hex()?),
            },
            b'>' => {
                if self.peek() != Some(b'>') {
                    return syntax(">");
                }
                self.pos += 1;
                Obj::Exec(Rc::from(">>"))
            }
            b'[' => Obj::Exec(Rc::from("[")),
            b']' => Obj::Exec(Rc::from("]")),
            b'{' => {
                if depth >= MAX_NESTING {
                    return Err(PsError::Limit("procedures nested too deeply"));
                }
                let mut items = vec![];
                loop {
                    self.skip_space();
                    match self.peek() {
                        None => return syntax("{"),
                        Some(b'}') => {
                            self.pos += 1;
                            break;
                        }
                        _ => match self.token(depth + 1)? {
                            Some(o) => items.push(o),
                            None => return syntax("{"),
                        },
                    }
                }
                Obj::proc(items)
            }
            b'}' | b')' => return syntax(&char::from(b).to_string()),
            b'/' => {
                // `//name` stands for the name's value when it is read.
                let immediate = self.peek() == Some(b'/');
                if immediate {
                    self.pos += 1;
                    self.immediate = true;
                }
                let name = self.word();
                if immediate { Obj::Exec(Rc::from(format!("{IMMEDIATE}{name}").as_str())) } else { Obj::Name(Rc::from(name.as_str())) }
            }
            _ => {
                self.pos -= 1;
                let w = self.word();
                number(&w).unwrap_or_else(|| Obj::Exec(Rc::from(w.as_str())))
            }
        }))
    }

    /// A regular token's characters.
    fn word(&mut self) -> String {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if is_space(b) || is_delim(b) {
                break;
            }
            self.pos += 1;
        }
        String::from_utf8_lossy(self.src.get(start..self.pos).unwrap_or_default()).into_owned()
    }

    /// A literal string after its `(`.
    fn string(&mut self) -> Res<Vec<u8>> {
        let mut out = vec![];
        let mut depth = 0usize;
        loop {
            let Some(b) = self.peek() else { return syntax("(") };
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' if depth == 0 => return Ok(out),
                b')' => {
                    depth -= 1;
                    out.push(b);
                }
                b'\\' => {
                    let Some(e) = self.peek() else { return syntax("(") };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'\r' => {
                            if self.peek() == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d @ b'0'..=b'7') => {
                                        v = v * 8 + u32::from(d - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push((v & 0xff) as u8);
                        }
                        other => out.push(other),
                    }
                }
                _ => out.push(b),
            }
        }
    }

    /// A hexadecimal string after its `<`.
    fn hex(&mut self) -> Res<Vec<u8>> {
        let end = self.rest().iter().position(|b| *b == b'>').ok_or(PsError::Ps("syntaxerror", "<".into()))?;
        let body = self.rest().get(..end).unwrap_or_default();
        let out = hex_decode(body).ok_or(PsError::Ps("syntaxerror", "<".into()))?;
        self.advance(end + 1);
        Ok(out)
    }

    /// An ASCII85 string after its `<~`.
    fn ascii85(&mut self) -> Res<Vec<u8>> {
        let end = find(self.rest(), b"~>").ok_or(PsError::Ps("syntaxerror", "<~".into()))?;
        let text = String::from_utf8_lossy(self.rest().get(..end + 2).unwrap_or_default()).into_owned();
        self.advance(end + 2);
        ps::ascii85_decode(&text).ok_or(PsError::Ps("syntaxerror", "<~".into()))
    }
}

/// Where `needle` first starts in `hay`.
pub(crate) fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    memchr::memmem::find(hay, needle)
}

/// Hexadecimal digits (whitespace ignored, an odd last digit padded with 0); `None` on others.
pub(crate) fn hex_decode(body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(body.len() / 2);
    let mut high: Option<u8> = None;
    for &b in body {
        if is_space(b) {
            continue;
        }
        let v = char::from(b).to_digit(16)? as u8;
        match high.take() {
            Some(h) => out.push(h << 4 | v),
            None => high = Some(v),
        }
    }
    if let Some(h) = high {
        out.push(h << 4);
    }
    Some(out)
}

/// A number token (`12`, `-3.5`, `.5`, `1e-3`, `16#ff`), or `None`.
fn number(w: &str) -> Option<Obj> {
    let first = w.bytes().next()?;
    if !(first.is_ascii_digit() || matches!(first, b'+' | b'-' | b'.')) {
        return None;
    }
    if let Ok(i) = w.parse::<i64>() {
        return Some(Obj::Int(i));
    }
    if let Some((base, digits)) = w.split_once('#') {
        let base: u32 = base.parse().ok().filter(|b| (2..=36).contains(b))?;
        return i64::from_str_radix(digits, base).ok().map(Obj::Int);
    }
    // Rust reads `inf` and `nan`; PostScript doesn't.
    if w.bytes().any(|b| b.is_ascii_alphabetic() && !matches!(b, b'e' | b'E')) {
        return None;
    }
    let v: f64 = w.parse().ok()?;
    v.is_finite().then_some(Obj::Real(v))
}
