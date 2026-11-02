//! The scanner of the editing data: PostScript-like tokens, read line by line.
//!
//! - A line starting with `%_` holds tokens like any other line (they are marked [`Token::hidden`]:
//!   readers of the printed format skip them as comments).
//! - Any other line starting with `%` is a [`Tok::Comment`] (the section markers).
//! - `%_` later in a line starts tokens too, hidden (a pseudo-comment operator after its
//!   operands, as a gradient stop's `… 50 100 %_Bs`: AI7 spec §1.4.3); any other `%` later in a
//!   line starts a comment that is skipped.
//! - The binary image data after `%%BeginData:` is one [`Tok::Data`]; the binary preview of a placed
//!   file is one [`Tok::Preview`].
//! - A document included whole (`%%BeginDocument:` … `%%EndDocument` lines, DSC 3.0 p. 68, nested
//!   ones counted, the counted payloads of `%%BeginData:` and `%%BeginBinary:` inside skipped) is
//!   one [`Tok::Document`].

use super::super::lex::{find, hex_decode};

/// A token.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Tok<'a> {
    Num(f64),
    /// A literal `(…)` string, escapes undone.
    Str(Vec<u8>),
    /// A hexadecimal `<…>` string.
    Hex(Vec<u8>),
    /// A literal `/name`.
    Name(&'a [u8]),
    /// An operator (any other word, `[` and `]` included).
    Word(&'a [u8]),
    /// A comment line, without its first `%`.
    Comment(&'a [u8]),
    /// An image's samples: what follows `XI` in a `%%BeginData:` section.
    Data(&'a [u8]),
    /// A placed file's preview: what is between `%AI26_BeginPlacedObjectPreview` and its end.
    Preview(&'a [u8]),
    /// A document included whole: what is between `%%BeginDocument:`'s line and the line of the
    /// `%%EndDocument` that ends it, or `None` when none does (the rest of the data is skipped).
    Document(Option<&'a [u8]>),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Token<'a> {
    pub tok: Tok<'a>,
    /// Read from a `%_` line.
    pub hidden: bool,
}

/// A placed file's preview (binary): (begin, end) markers.
const PREVIEW: (&[u8], &[u8]) = (b"%AI26_BeginPlacedObjectPreview", b"%AI26_EndPlacedObjectPreview");
/// A placed file's markers, which a hidden one has on `%_` lines.
const PLACE: [&[u8]; 2] = [b"%AI5_BeginPlace", b"%AI5_EndPlace"];
/// A document included whole: (begin, end) markers.
const DOCUMENT: (&[u8], &[u8]) = (b"%%BeginDocument", b"%%EndDocument");

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

fn is_eol(b: u8) -> bool {
    matches!(b, b'\r' | b'\n')
}

pub(super) struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    /// The current line started with `%_`.
    hidden: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a [u8]) -> Self {
        Self { src, pos: 0, hidden: false }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn rest(&self) -> &'a [u8] {
        self.src.get(self.pos..).unwrap_or_default()
    }

    fn at_line_start(&self) -> bool {
        self.pos == 0 || self.src.get(self.pos - 1).is_some_and(|b| is_eol(*b))
    }

    /// The rest of the current line (without its end), the position moved past it.
    fn line(&mut self) -> &'a [u8] {
        let rest = self.rest();
        let n = rest.iter().position(|b| is_eol(*b)).unwrap_or(rest.len());
        self.pos += n;
        rest.get(..n).unwrap_or_default()
    }

    /// [`Self::line`], the position moved past its end too (`\r`, `\n` or `\r\n`).
    fn whole_line(&mut self) -> &'a [u8] {
        let line = self.line();
        if self.peek() == Some(b'\r') {
            self.pos += 1;
        }
        if self.peek() == Some(b'\n') {
            self.pos += 1;
        }
        line
    }

    /// Where the next token is looked for.
    pub fn offset(&self) -> usize {
        self.pos
    }

    /// Look for the next token at `pos` again (the end of the data at most).
    pub fn seek(&mut self, pos: usize) {
        self.pos = pos.min(self.src.len());
    }

    /// The data from `from` to `to` (empty where that isn't in it).
    pub fn slice(&self, from: usize, to: usize) -> &'a [u8] {
        self.src.get(from..to).unwrap_or_default()
    }

    /// Skip past the next `needle` (the end of data that isn't tokens) → what was skipped.
    pub fn skip_past(&mut self, needle: &[u8]) -> &'a [u8] {
        let rest = self.rest();
        let n = find(rest, needle).map_or(rest.len(), |i| i + needle.len());
        self.pos += n;
        rest.get(..n).unwrap_or_default()
    }

    /// The next token; `None` at the end.
    pub fn next_token(&mut self) -> Option<Token<'a>> {
        loop {
            let b = self.peek()?;
            if is_eol(b) {
                self.pos += 1;
                self.hidden = false;
                continue;
            }
            if is_space(b) {
                self.pos += 1;
                continue;
            }
            if b == b'%' {
                let start = self.at_line_start();
                // A pseudo-comment: `%_` at the line's start, or after operands.
                if self.rest().starts_with(b"%_") && (start || self.rest().get(2).is_some_and(|b| b.is_ascii_alphabetic())) {
                    self.pos += 2;
                    self.hidden = true;
                    continue;
                }
                // Right after `%_` a section still starts (a hidden image's data, #1264).
                let prefixed = self.hidden && self.pos >= 2 && self.src.get(self.pos - 2..self.pos) == Some(b"%_");
                let line = self.line();
                if (start || prefixed)
                    && let Some(t) = self.special(line)
                {
                    return Some(Token { hidden: prefixed, ..t });
                }
                if start {
                    return Some(Token { tok: Tok::Comment(line.get(1..).unwrap_or_default()), hidden: false });
                }
                // So does a hidden placed file: its `~` ends what `%_%AI5_BeginPlace` begins.
                if prefixed && PLACE.iter().any(|m| line.starts_with(m)) {
                    return Some(Token { tok: Tok::Comment(line.get(1..).unwrap_or_default()), hidden: true });
                }
                // A comment later in a line.
                continue;
            }
            let hidden = self.hidden;
            self.pos += 1;
            let tok = match b {
                b'(' => Tok::Str(self.string()),
                b'<' => {
                    let rest = self.rest();
                    let end = rest.iter().position(|b| *b == b'>').unwrap_or(rest.len());
                    self.pos += (end + 1).min(rest.len());
                    // Malformed digits read as an empty string.
                    Tok::Hex(rest.get(..end).and_then(hex_decode).unwrap_or_default())
                }
                b'[' | b']' | b'{' | b'}' | b')' | b'>' => Tok::Word(self.src.get(self.pos - 1..self.pos).unwrap_or_default()),
                b'/' => Tok::Name(self.word()),
                _ => {
                    self.pos -= 1;
                    let w = self.word();
                    match number(w) {
                        Some(v) => Tok::Num(v),
                        None => Tok::Word(w),
                    }
                }
            };
            return Some(Token { tok, hidden });
        }
    }

    /// The token a comment line stands for when it starts a section read differently: image data
    /// (`%%BeginData:` followed by `XI`), a placed file's preview and skipped binary sections.
    fn special(&mut self, line: &'a [u8]) -> Option<Token<'a>> {
        if line.starts_with(PREVIEW.0) {
            let from = self.pos;
            let after = find(self.rest(), PREVIEW.1).map_or(self.src.len(), |i| self.pos + i);
            self.pos = after;
            self.line();
            return Some(Token { tok: Tok::Preview(self.slice(from, after)), hidden: false });
        }
        if line.starts_with(DOCUMENT.0) {
            return Some(Token { tok: Tok::Document(self.document()), hidden: false });
        }
        let n = line.strip_prefix(b"%%BeginData:")?;
        let n: usize = std::str::from_utf8(n).ok()?.split_whitespace().next()?.parse().ok()?;
        // The count starts after the comment's line end character (a following `\n` counts).
        let from = self.pos + 1;
        let region = self.src.get(from..)?;
        let mut skip = region.iter().take_while(|b| is_space(**b)).count();
        // A hidden object's section is on `%_` lines up to its samples: `%_XI`, then `%_%%EndData`.
        let hidden = region.get(skip..)?.starts_with(b"%_XI");
        if hidden {
            skip += 2;
        } else if !region.get(skip..)?.starts_with(b"XI") {
            return None;
        }
        let mut data = from + skip + 2;
        match (self.src.get(data), self.src.get(data + 1)) {
            (Some(b'\r'), Some(b'\n')) => data += 2,
            (Some(b'\r' | b'\n'), _) => data += 1,
            _ => {}
        }
        // The count leaves out an alpha channel: the data ends where `%%EndData` starts.
        let declared = from.saturating_add(n).min(self.src.len()).max(data);
        let window =
            self.src.get(declared.saturating_sub(4).max(data)..declared.saturating_add(n.saturating_mul(2)).saturating_add(64).min(self.src.len()));
        let end = match window.and_then(|w| find(w, b"%%EndData")) {
            Some(i) => declared.saturating_sub(4).max(data) + i,
            None => declared,
        };
        self.pos = end;
        let samples = self.src.get(data..end).unwrap_or_default();
        let samples = if hidden { samples.strip_suffix(b"%_").unwrap_or(samples) } else { samples };
        Some(Token { tok: Tok::Data(samples), hidden: false })
    }

    /// After a `%%BeginDocument:` line: the document up to the start of the `%%EndDocument` line
    /// that ends it, the position moved past that line. A `%%BeginDocument:` line inside starts a
    /// nested document, and the bytes a `%%BeginData:` or `%%BeginBinary:` line inside counts are
    /// skipped (DSC 3.0 pp. 68 and 72): a marker in them ends nothing. `None` when no line ends it:
    /// the position moves to the end of the data.
    fn document(&mut self) -> Option<&'a [u8]> {
        // The rest of the `%%BeginDocument:` line is its end; the line after the document starts
        // tokens afresh (not hidden unless it starts with `%_`).
        self.whole_line();
        self.hidden = false;
        let from = self.pos;
        let mut depth = 0usize;
        while self.pos < self.src.len() {
            let start = self.pos;
            // A hidden object's document is on `%_` lines.
            let line = self.whole_line();
            let line = line.strip_prefix(b"%_").unwrap_or(line);
            if line.starts_with(DOCUMENT.1) {
                if depth == 0 {
                    return Some(self.slice(from, start));
                }
                depth -= 1;
            } else if line.starts_with(DOCUMENT.0) {
                depth = depth.saturating_add(1);
            } else if let Some(n) = line.strip_prefix(b"%%BeginData:").or_else(|| line.strip_prefix(b"%%BeginBinary:")) {
                self.skip_counted(n);
            }
        }
        None
    }

    /// Skip what a `%%BeginData:` or `%%BeginBinary:` comment counts (`n`, its text after the
    /// colon): `count [type [Bytes|Lines]]`, bytes unless it says `Lines`.
    fn skip_counted(&mut self, n: &[u8]) {
        let mut words = std::str::from_utf8(n).unwrap_or_default().split_whitespace();
        let Some(count) = words.next().and_then(|c| c.parse::<usize>().ok()) else { return };
        if words.nth(1).is_some_and(|unit| unit.eq_ignore_ascii_case("lines")) {
            for _ in 0..count {
                if self.pos >= self.src.len() {
                    break;
                }
                self.whole_line();
            }
        } else {
            // The count starts after the comment's line end, which `document` has passed.
            self.pos = self.pos.saturating_add(count).min(self.src.len());
        }
    }

    /// A regular token's characters.
    fn word(&mut self) -> &'a [u8] {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if is_space(b) || is_delim(b) {
                break;
            }
            self.pos += 1;
        }
        self.src.get(start..self.pos).unwrap_or_default()
    }

    /// A literal string after its `(` (an unterminated one ends with the data).
    fn string(&mut self) -> Vec<u8> {
        let mut out = vec![];
        let mut depth = 0usize;
        while let Some(b) = self.peek() {
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' if depth == 0 => break,
                b')' => {
                    depth -= 1;
                    out.push(b);
                }
                b'\\' => {
                    let Some(e) = self.peek() else { break };
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
        out
    }
}

/// A finite number token (`12`, `-3.5`, `.5`, `1e-3`), or `None`.
pub(super) fn number(w: &[u8]) -> Option<f64> {
    let first = *w.first()?;
    if !(first.is_ascii_digit() || matches!(first, b'+' | b'-' | b'.')) {
        return None;
    }
    if w.iter().any(|b| b.is_ascii_alphabetic() && !matches!(b, b'e' | b'E')) {
        return None;
    }
    let v: f64 = std::str::from_utf8(w).ok()?.parse().ok()?;
    v.is_finite().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(src: &[u8]) -> Vec<Token<'_>> {
        let mut l = Lexer::new(src);
        std::iter::from_fn(|| l.next_token()).collect()
    }

    fn t(tok: Tok<'_>, hidden: bool) -> Token<'_> {
        Token { tok, hidden }
    }

    #[test]
    fn hidden_lines_and_comments() {
        let toks = all(b"%AI5_BeginLayer\r\n1 (a\\)b) Ln\r%_/X : ;\n0 %_BS\n<4142>\n");
        assert_eq!(
            toks,
            vec![
                t(Tok::Comment(b"AI5_BeginLayer"), false),
                t(Tok::Num(1.0), false),
                t(Tok::Str(b"a)b".to_vec()), false),
                t(Tok::Word(b"Ln"), false),
                t(Tok::Name(b"X"), true),
                t(Tok::Word(b":"), true),
                t(Tok::Word(b";"), true),
                t(Tok::Num(0.0), false),
                // A pseudo-comment operator after its operands.
                t(Tok::Word(b"BS"), true),
                t(Tok::Hex(b"AB".to_vec()), false),
            ]
        );
    }

    #[test]
    fn a_percent_later_in_a_line_is_a_comment_unless_an_operator_follows_it() {
        let toks = all(b"1 % a note (\n2 %_Bs\n3 %_ 4\n");
        let words: Vec<_> = toks.iter().map(|t| (t.tok.clone(), t.hidden)).collect();
        assert_eq!(words, [(Tok::Num(1.0), false), (Tok::Num(2.0), false), (Tok::Word(b"Bs"), true), (Tok::Num(3.0), false)]);
    }

    #[test]
    fn image_data_is_one_token() {
        // The count starts after the comment's `\r`: `\nXI\n` and 3 samples; an alpha channel
        // after them isn't counted.
        let src = b"[1 0 0 1 0 0] XI\r\n%%BeginData: 7\r\nXI\n(%)A%%EndData\r\nXH\r\n";
        let toks = all(src);
        assert!(toks.contains(&t(Tok::Data(b"(%)A"), false)), "{toks:?}");
        assert_eq!(toks.last(), Some(&t(Tok::Word(b"XH"), false)));
        // #1264: on `%_` lines too, where the samples would otherwise be read as tokens.
        let toks = all(b"%_[1 0 0 1 0 0] XI\r%_%%BeginData: 7\rXI\n(U)A%%EndData\r%_XH\ru\r");
        assert!(toks.contains(&t(Tok::Data(b"(U)A"), true)), "{toks:?}");
        assert_eq!(toks.last(), Some(&t(Tok::Word(b"u"), false)), "{toks:?}");
        // A section of hex lines is comments.
        let toks = all(b"%%BeginData: 9 Hex Bytes\r\n%00FF\r\n%%EndData\r\n1\r\n");
        assert_eq!(toks.last(), Some(&t(Tok::Num(1.0), false)));
        // On `%_` lines (an opacity mask's image, say) too: a `(` in the samples starts no string.
        let toks = all(b"%_XI\r%_%%BeginData: 4\r%_XI\r(\x00\x00\x00%_%%EndData\r%_XH\r1\r");
        assert!(toks.contains(&t(Tok::Data(b"(\x00\x00\x00"), true)), "{toks:?}");
        assert_eq!(toks.last(), Some(&t(Tok::Num(1.0), false)));
    }

    #[test]
    fn binary_sections_are_one_token() {
        let toks = all(b"%AI26_BeginPlacedObjectPreview\r\n2 1 (\x00\xff\r\n%AI26_EndPlacedObjectPreview\r\nN\r\n");
        assert_eq!(toks, vec![t(Tok::Preview(b"\r\n2 1 (\x00\xff\r\n"), false), t(Tok::Word(b"N"), false)]);
        let toks = all(b"%%BeginDocument: a.eps\n(\x00 frobnicate\n%%EndDocument\nN\n");
        assert_eq!(toks, vec![t(Tok::Document(Some(b"(\x00 frobnicate\n")), false), t(Tok::Word(b"N"), false)]);
        // One that doesn't end takes the rest.
        assert_eq!(all(b"%AI26_BeginPlacedObjectPreview\r1 (").len(), 1);
    }

    #[test]
    fn an_included_document_ends_at_its_own_end_line() {
        // A nested document, `%%EndDocument` later in a line and in counted data end nothing; `\r\n`,
        // `\r` and `\n` lines alike.
        let body = b"1 0 0 setrgbcolor\r\n%%BeginDocument: inner.eps\r(%%EndDocument) pop\r%%EndDocument\r\n\
            %%BeginData: 14 Binary Bytes\n%%EndDocument\n%%BeginBinary: 14\n%%EndDocument\n\
            %%BeginData: 1 Hex Lines\n%%EndDocument\n";
        let mut src = b"%%BeginDocument: outer.eps\r\n".to_vec();
        src.extend_from_slice(body);
        src.extend_from_slice(b"%%EndDocument\r\n~\r\n");
        assert_eq!(all(&src), vec![t(Tok::Document(Some(body)), false), t(Tok::Word(b"~"), false)]);
        // On a hidden object's `%_` lines.
        let toks = all(b"%_%%BeginDocument: a.eps\r%_1 0 0 setrgbcolor\r%_%%EndDocument\r%_~\r");
        assert_eq!(toks, vec![t(Tok::Document(Some(b"%_1 0 0 setrgbcolor\r")), true), t(Tok::Word(b"~"), true)]);
        // The line after it is hidden only when it starts with `%_`.
        let toks = all(b"%_%%BeginDocument: a.eps\r%_%%EndDocument\rN\r");
        assert_eq!(toks, vec![t(Tok::Document(Some(b"")), true), t(Tok::Word(b"N"), false)]);
    }

    #[test]
    fn an_included_document_that_doesnt_end_takes_the_rest() {
        for src in [&b"%%BeginDocument: a.eps\n%%BeginDocument: b.eps\n%%EndDocument\n~\n"[..], b"%%BeginDocument: a.eps\n%%BeginData: 99999999999\n"]
        {
            assert_eq!(all(src), vec![t(Tok::Document(None), false)]);
        }
    }

    #[test]
    fn skip_past_data() {
        let mut l = Lexer::new(b"/Binary : /ASCII85Decode ,\r\n%_(abc[%\r\n%x~>\r\n%_; 1");
        for _ in 0..4 {
            l.next_token();
        }
        assert_eq!(l.skip_past(b"~>"), b"\r\n%_(abc[%\r\n%x~>");
        assert_eq!(l.next_token().map(|t| t.tok), Some(Tok::Word(b";")));
        assert_eq!(l.next_token().map(|t| t.tok), Some(Tok::Num(1.0)));
    }

    #[test]
    fn hostile_input_ends() {
        for src in [&b"(unterminated \\"[..], b"<12", b"%%BeginData: 99999999999\r\nXI", b"%%BeginData: 5\rXI\r", b"/", b"1e999 nan"] {
            let toks = all(src);
            assert!(toks.len() < 10);
        }
    }
}
