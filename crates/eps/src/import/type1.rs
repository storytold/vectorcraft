//! Embedded fonts whose glyphs are drawn from the font program the file carries: Type 1 fonts,
//! whose encrypted part (`eexec`: Adobe Type 1 Font Format, chapter 7) holds their charstrings,
//! and Type 42 fonts (see `type42`). The interpreter doesn't run the encrypted part; it is read
//! by the Type 1 font parser, and the font dictionary gets the `CharStrings` dictionary the part
//! would have given it, each glyph name standing for its glyph's index, by which the glyph is
//! drawn. Its charstrings are run here, counting the bytes of the charstrings and subroutines
//! they enter, so that the work of drawing glyphs is paid for from the operation budget.

use std::cell::Cell;
use std::rc::Rc;

use skrifa::GlyphId;
use skrifa::outline::OutlinePen;
use skrifa::raw::ps::cs::{self, CharstringContext, CharstringKind, CommandSink, NopFilterSink, TransformSink};
use skrifa::raw::ps::error::Error as CsError;
use skrifa::raw::ps::type1::Type1Font;
use skrifa::raw::types::Fixed;
use vectorcraft_geom::BezPath;

use super::type42;

/// Most bytes of an encrypted font program read.
pub(super) const MAX_PROGRAM: usize = 16 << 20;

/// The font program a plain or descendant font carries.
#[derive(Clone)]
pub(super) enum Embedded {
    TrueType(Rc<type42::Font>),
    Type1(Rc<Font>),
}

impl Embedded {
    /// Glyph `gid`: its outline and advance width, in ems (y up), or `None` when it can't be read.
    pub fn glyph(&self, gid: u32) -> Option<(BezPath, f64)> {
        match self {
            Self::TrueType(f) => type42::glyph(&f.data, gid),
            Self::Type1(f) => {
                let mut pen = type42::Pen(BezPath::new(), 1.0 / UNITS);
                // A glyph that can't be drawn draws nothing, and has no width.
                let w = f.draw(gid, &mut pen).map_or(0.0, |w| w / UNITS);
                Some((pen.0, if w.is_finite() { w } else { 0.0 }))
            }
        }
    }

    /// Glyph `gid`'s advance width, in ems.
    pub fn advance(&self, gid: u32) -> f64 {
        match self {
            Self::TrueType(f) => type42::advance(&f.data, gid),
            Self::Type1(_) => self.glyph(gid).map_or(0.0, |g| g.1),
        }
    }

    /// The character glyph `gid` stands for, when the font says (a TrueType font's character
    /// map or `post` names; a Type 1 glyph's name).
    pub fn char_of(&self, gid: u32) -> Option<char> {
        match self {
            Self::TrueType(f) => f.char_of(gid),
            Self::Type1(f) => f.font.glyph_name(GlyphId::new(gid)).and_then(|n| super::text::named_char(0, n)),
        }
    }

    /// The work done since last asked, for the operation budget: a TrueType font's reading its
    /// character map (see [`type42::Font::take_char_map_work`]), a Type 1 font's running its
    /// charstrings (see [`Font::draw`]).
    pub fn take_work(&self) -> u64 {
        match self {
            Self::TrueType(f) => f.take_char_map_work(),
            Self::Type1(f) => f.work.take(),
        }
    }

    /// The glyph a TrueType font's character map gives `c` (Type 1 fonts have none).
    pub fn gid_of(&self, c: char) -> Option<u32> {
        match self {
            Self::TrueType(f) => f.gid_of(c),
            Self::Type1(_) => None,
        }
    }
}

/// Type 1 glyph space: 1000 units an em, as the font matrix of the program made for parsing says.
const UNITS: f64 = 1000.0;

/// A Type 1 font read from its encrypted part: the font, and its charstrings as the font parser
/// reads them, run here so their work is counted.
pub(super) struct Font {
    pub font: Type1Font,
    /// Each glyph's name and decrypted charstring, by glyph index.
    charstrings: Vec<(Vec<u8>, Vec<u8>)>,
    /// The bytes of charstrings and subroutines run since last asked (see [`Embedded::take_work`]).
    work: Cell<u64>,
}

impl Font {
    /// The bytes the font keeps.
    pub fn size(&self) -> usize {
        self.charstrings.iter().fold(0usize, |n, (a, b)| n.saturating_add(a.len()).saturating_add(b.len()))
    }

    /// Draw glyph `gid` with `pen` (in 1000-unit glyph space, as the font parser draws it): its
    /// advance width, or `None` when it can't be run. The bytes of each charstring it enters
    /// (its own, its subroutines', `seac` accents') are added to the work done.
    fn draw(&self, gid: u32, pen: &mut impl OutlinePen) -> Option<f64> {
        let (name, charstring) = self.charstrings.get(gid as usize)?;
        if self.font.glyph_name(GlyphId::new(gid)).map(str::as_bytes) != Some(name.as_slice()) {
            return None;
        }
        let transform = self.font.transform(None);
        let mut filter = NopFilterSink::new(pen);
        let mut sink = TransformSink::new(&mut filter, transform);
        let (w, work) = evaluate(&self.font, charstring, &mut sink);
        self.work.set(self.work.get().saturating_add(work));
        let w = w.ok()??;
        Some(f64::from(transform.transform_h_metric(w).to_f32().max(0.0)))
    }
}

/// Type 1 charstring `data` run in context `ctx` into `sink`: its width (when it gives one) and
/// the bytes of the charstrings it entered, at least one for each, which bound the operations it
/// ran. The evaluator stops a charstring after a million operations but starts again for every
/// glyph, so drawing many glyphs is paid for with these.
pub(super) fn evaluate(ctx: &impl CharstringContext, data: &[u8], sink: &mut impl CommandSink) -> (Result<Option<Fixed>, CsError>, u64) {
    let counted = Counted { ctx, work: Cell::new(data.len() as u64 + 1) };
    let w = cs::evaluate(&counted, None, data, sink);
    (w, counted.work.get())
}

/// A charstring context counting the bytes of the charstrings it gives the evaluator.
struct Counted<'a, C> {
    ctx: &'a C,
    work: Cell<u64>,
}

impl<C> Counted<'_, C> {
    fn add<'b>(&self, s: &'b [u8]) -> &'b [u8] {
        self.work.set(self.work.get().saturating_add(s.len() as u64 + 1));
        s
    }
}

impl<C: CharstringContext> CharstringContext for Counted<'_, C> {
    fn kind(&self) -> CharstringKind {
        self.ctx.kind()
    }
    fn seac_components(&self, base: i32, accent: i32) -> Result<[&[u8]; 2], CsError> {
        let [b, a] = self.ctx.seac_components(base, accent)?;
        Ok([self.add(b), self.add(a)])
    }
    fn global_subr(&self, index: i32) -> Result<&[u8], CsError> {
        self.ctx.global_subr(index).map(|s| self.add(s))
    }
    fn subr(&self, index: i32) -> Result<&[u8], CsError> {
        self.ctx.subr(index).map(|s| self.add(s))
    }
    fn weight_vector(&self) -> &[Fixed] {
        self.ctx.weight_vector()
    }
}

/// The Type 1 font whose encrypted part is `encrypted` (from just after `eexec`, binary or
/// hexadecimal), or `None` when it can't be read. The part is read on its own, after a clear
/// part that gives only the font matrix of 1000-unit glyph space: the font's own clear part, run
/// by the interpreter, gives its matrix and encoding.
pub(super) fn parse(encrypted: &[u8]) -> Option<Font> {
    if encrypted.len() > MAX_PROGRAM {
        return None;
    }
    let mut program = b"%!FontType1-1.0: Embedded\n/FontMatrix [0.001 0 0 0.001 0 0] readonly def\ncurrentfile eexec\n".to_vec();
    program.extend_from_slice(encrypted);
    let font = Type1Font::new(&program).ok()?;
    Some(Font { font, charstrings: charstrings(encrypted), work: Cell::new(0) })
}

/// The `.notdef` glyph of a font that has none: `0 333 hsbw endchar` (Adobe Type 1 Font Format,
/// §6), as the font parser gives it.
const NOTDEF: &[u8] = &[139, 247, 225, 13, 14];

/// The charstrings of encrypted part `encrypted`, with their names, in the order the font parser
/// numbers them: the first `CharStrings` dictionary's entries (`/name len RD bytes ND`) in the
/// order written, decrypted with the charstring key unless `lenIV` is negative (Adobe Type 1 Font
/// Format, §2.4 and §7), `.notdef` (made when missing) swapped to the front. Empty when the part
/// can't be read.
fn charstrings(encrypted: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let skip = encrypted.iter().take_while(|c| matches!(c, b' ' | b'\t' | b'\r' | b'\n')).count();
    let encrypted = encrypted.get(skip..).unwrap_or_default();
    let hex = encrypted.len() > 3 && encrypted.iter().take(4).all(u8::is_ascii_hexdigit);
    let bytes = if hex { unhex(encrypted) } else { encrypted.to_vec() };
    let plain: Vec<u8> = decrypt(&bytes, 55665).into_iter().skip(4).collect();
    let mut lex = Lexer { d: &plain, pos: 0 };
    let mut len_iv = 4;
    let mut out = Vec::new();
    while let Some(t) = lex.next() {
        match t {
            Tok::Name(b"lenIV") => match lex.next() {
                Some(Tok::Int(n)) => len_iv = n,
                _ => return vec![],
            },
            Tok::Name(b"CharStrings") => {
                let Some(Tok::Int(_)) = lex.next() else { return vec![] };
                while let Some(t) = lex.next() {
                    let ends = lex.peek().is_some_and(special_or_space);
                    match t {
                        Tok::Other(b"end") if ends => break,
                        Tok::Other(b"def") if ends && !out.is_empty() => break,
                        Tok::Name(name) => {
                            let Some(Tok::Blob(data)) = lex.next() else { return vec![] };
                            let data = if len_iv >= 0 { decrypt(data, 4330).into_iter().skip(len_iv as usize).collect() } else { data.to_vec() };
                            out.push((name.to_vec(), data));
                        }
                        _ => {}
                    }
                }
                break;
            }
            _ => {}
        }
    }
    if out.is_empty() {
        return out;
    }
    let notdef = match out.iter().position(|(n, _)| n == b".notdef") {
        Some(i) => i,
        None => {
            out.push((b".notdef".to_vec(), NOTDEF.to_vec()));
            out.len() - 1
        }
    };
    out.swap(0, notdef);
    out
}

/// `data` decrypted with key `r` (Adobe Type 1 Font Format, §7.1).
fn decrypt(data: &[u8], mut r: u16) -> Vec<u8> {
    data.iter()
        .map(|&c| {
            let p = c ^ (r >> 8) as u8;
            r = (u16::from(c).wrapping_add(r)).wrapping_mul(52845).wrapping_add(22719);
            p
        })
        .collect()
}

/// Hexadecimal `data` as bytes, white space skipped, up to the first other character (an odd
/// last digit is its byte's high half).
fn unhex(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let mut high: Option<u8> = None;
    for &c in data {
        if space(c) {
            continue;
        }
        let Some(v) = char::from(c).to_digit(16) else { break };
        let v = v as u8;
        match high.take() {
            Some(h) => out.push(h << 4 | v),
            None => high = Some(v),
        }
    }
    out.extend(high.map(|h| h << 4));
    out
}

fn space(c: u8) -> bool {
    matches!(c, b' ' | b'\n' | b'\r' | b'\t' | b'\0' | 0x0C)
}

fn special_or_space(c: u8) -> bool {
    space(c) || matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

/// A token of a font's decrypted encrypted part, as the font parser splits them.
enum Tok<'a> {
    Int(i64),
    Name(&'a [u8]),
    /// The bytes after `len RD ` (or `-|`).
    Blob(&'a [u8]),
    Other(&'a [u8]),
}

struct Lexer<'a> {
    d: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<u8> {
        self.d.get(self.pos).copied()
    }

    fn byte(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    fn skip_while(&mut self, f: impl Fn(u8) -> bool) {
        while self.peek().is_some_and(&f) {
            self.pos += 1;
        }
    }

    fn skip_line(&mut self) {
        while let Some(c) = self.byte() {
            if c == b'\n' || c == b'\r' {
                break;
            }
        }
    }

    /// The rest of a string after its `(`, nested parentheses and escapes included.
    fn string(&mut self) -> Option<()> {
        let mut depth = 1u32;
        loop {
            match self.byte()? {
                b'(' => depth = depth.saturating_add(1),
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(());
                    }
                }
                b'\\' => {
                    self.byte()?;
                }
                _ => {}
            }
        }
    }

    /// The rest of a procedure after its `{`, nested procedures, comments and strings included.
    fn procedure(&mut self) -> Option<()> {
        let mut depth = 1u32;
        loop {
            match self.byte()? {
                b'{' => depth = depth.saturating_add(1),
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(());
                    }
                }
                b'%' => self.skip_line(),
                b'(' => self.string()?,
                _ => {}
            }
        }
    }

    fn next(&mut self) -> Option<Tok<'a>> {
        loop {
            self.skip_while(space);
            let start = self.pos;
            match self.byte()? {
                b'%' => self.skip_line(),
                b'{' => return self.procedure().map(|()| Tok::Other(b"{")),
                b'(' => return self.string().map(|()| Tok::Other(b"(")),
                b'<' if self.peek() == Some(b'<') => self.pos += 1,
                b'<' => {
                    // Hexadecimal digits and white space, then `>`.
                    let end = loop {
                        let c = self.byte()?;
                        if !space(c) && !c.is_ascii_hexdigit() {
                            break c;
                        }
                    };
                    return (end == b'>').then_some(Tok::Other(b"<"));
                }
                b'>' => {
                    if self.byte()? != b'>' {
                        return None;
                    }
                }
                b'/' => {
                    let c = self.peek()?;
                    if special_or_space(c) {
                        if space(c) {
                            self.pos += 1;
                        }
                        return Some(Tok::Name(&[]));
                    }
                    self.skip_while(|c| !special_or_space(c));
                    return self.d.get(start + 1..self.pos).map(Tok::Name);
                }
                b'[' | b']' => return self.d.get(start..self.pos).map(Tok::Other),
                c => {
                    self.skip_while(|c| !special_or_space(c));
                    let text = self.d.get(start..self.pos)?;
                    if (c.is_ascii_digit() || c == b'-')
                        && !text.contains(&b'.')
                        && let Some(n) = int(text)
                    {
                        return Some(self.blob(n).unwrap_or(Tok::Int(n)));
                    }
                    return Some(Tok::Other(text));
                }
            }
        }
    }

    /// The `n` bytes after `RD ` or `-| `, when those come next.
    fn blob(&mut self, n: i64) -> Option<Tok<'a>> {
        let mut at = self.pos;
        while self.d.get(at).copied().is_some_and(space) {
            at += 1;
        }
        let start = at;
        while self.d.get(at).copied().is_some_and(|c| !special_or_space(c)) {
            at += 1;
        }
        if !matches!(self.d.get(start..at), Some(b"RD" | b"-|")) {
            return None;
        }
        // One space, then the bytes; a count past the end ends the part.
        let from = at + 1;
        let to = usize::try_from(n).ok().and_then(|n| from.checked_add(n));
        self.pos = to.filter(|to| *to <= self.d.len()).unwrap_or(self.d.len());
        Some(Tok::Blob(to.and_then(|to| self.d.get(from..to)).unwrap_or_default()))
    }
}

/// A PostScript integer (`12`, `-3`, `8#17`), or `None`.
fn int(text: &[u8]) -> Option<i64> {
    let s = std::str::from_utf8(text).ok()?;
    match s.find('#') {
        Some(i @ (1 | 2)) => {
            let radix = s.get(..i)?.parse::<u32>().ok().filter(|r| (2..=36).contains(r))?;
            i64::from_str_radix(s.get(i + 1..)?, radix).ok()
        }
        _ => s.parse().ok(),
    }
}
