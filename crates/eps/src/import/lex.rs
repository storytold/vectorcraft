//! The scanner: PostScript source into objects (numbers, names, strings, procedures), with the
//! position the program's own data (`currentfile`) is read from.
//!
//! Backed by `craft_ps::read::lex`.

use std::rc::Rc;

use craft_ps::read::lex::{LexError, Number, Token};

use super::obj::{Obj, PsError, Res};

pub(crate) use craft_ps::read::lex::{IMMEDIATE, hex_decode};

/// Where `needle` first starts in `hay` (uses `memmem` so large inputs stay linear-time).
pub(crate) fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    memchr::memmem::find(hay, needle)
}

pub(crate) struct Lexer<'a> {
    inner: craft_ps::read::lex::Lexer<'a>,
    /// The last object read has immediately evaluated names (`//name`) in it, read as executable
    /// names starting with [`IMMEDIATE`] for the interpreter to replace by their values.
    pub immediate: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a [u8]) -> Self {
        Self { inner: craft_ps::read::lex::Lexer::new(src), immediate: false }
    }

    /// What is left to read.
    pub fn rest(&self) -> &'a [u8] {
        self.inner.rest()
    }

    /// What is left to read as data: after the whitespace that ended the last token (a CR LF
    /// pair counts as one).
    pub fn data(&mut self) -> &'a [u8] {
        self.inner.data()
    }

    /// Skip `n` bytes (the data a filter or `readstring` took).
    pub fn advance(&mut self, n: usize) {
        self.inner.advance(n);
    }

    /// The next object at the top level (`None` at the end of the file).
    pub fn next(&mut self) -> Res<Option<Obj>> {
        let res = self.inner.next().map(|opt| opt.map(into_obj)).map_err(into_err);
        self.immediate = self.inner.immediate;
        res
    }
}

fn into_err(err: LexError) -> PsError {
    match err {
        LexError::Syntax(what) => PsError::Ps("syntaxerror", what),
        LexError::Limit(msg) => PsError::Limit(msg),
    }
}

fn into_obj(tok: Token) -> Obj {
    match tok {
        Token::Int(i) => Obj::Int(i),
        Token::Real(r) => Obj::Real(r),
        Token::Name(n) => Obj::Name(Rc::from(n.as_str())),
        Token::Exec(n) => Obj::Exec(Rc::from(n.as_str())),
        Token::Str(s) => Obj::string(s),
        Token::Proc(items) => Obj::proc(items.into_iter().map(into_obj).collect()),
    }
}

/// A number token (`12`, `-3.5`, `.5`, `1e-3`, `16#ff`), or `None`.
pub(crate) fn number(w: &str) -> Option<Obj> {
    match craft_ps::read::lex::number(w)? {
        Number::Int(i) => Some(Obj::Int(i)),
        Number::Real(r) => Some(Obj::Real(r)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radix_number_with_sign_after_hash_is_not_a_number() {
        assert!(number("16#+ff").is_none());
        assert!(number("16#-ff").is_none());
        assert!(matches!(number("16#ff"), Some(Obj::Int(255))));
    }
}
