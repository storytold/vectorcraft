//! Embedded fonts whose glyphs are drawn from the font program the file carries: Type 1 fonts,
//! whose encrypted part (`eexec`: Adobe Type 1 Font Format, chapter 7) holds their charstrings,
//! and Type 42 fonts (see `type42`). The interpreter doesn't run the encrypted part; it is read
//! by the Type 1 font parser, and the font dictionary gets the `CharStrings` dictionary the part
//! would have given it, each glyph name standing for its glyph's index, by which the glyph is
//! drawn.

use std::rc::Rc;

use skrifa::GlyphId;
use skrifa::raw::ps::type1::Type1Font;
use vectorcraft_geom::BezPath;

use super::type42;

/// Most bytes of an encrypted font program read.
pub(super) const MAX_PROGRAM: usize = 16 << 20;

/// The font program a plain or descendant font carries.
#[derive(Clone)]
pub(super) enum Embedded {
    TrueType(Rc<type42::Font>),
    Type1(Rc<Type1Font>),
}

impl Embedded {
    /// Glyph `gid`: its outline and advance width, in ems (y up), or `None` when it can't be read.
    pub fn glyph(&self, gid: u32) -> Option<(BezPath, f64)> {
        match self {
            Self::TrueType(f) => type42::glyph(&f.data, gid),
            Self::Type1(f) => {
                let mut pen = type42::Pen(BezPath::new(), 1.0 / UNITS);
                // A glyph that can't be drawn draws nothing, and has no width.
                let w = f.draw(GlyphId::new(gid), None, &mut pen).ok().flatten().map_or(0.0, |w| f64::from(w) / UNITS);
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
            Self::Type1(f) => f.glyph_name(GlyphId::new(gid)).and_then(|n| super::text::named_char(0, n)),
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

/// The Type 1 font whose encrypted part is `encrypted` (from just after `eexec`, binary or
/// hexadecimal), or `None` when it can't be read. The part is read on its own, after a clear
/// part that gives only the font matrix of 1000-unit glyph space: the font's own clear part, run
/// by the interpreter, gives its matrix and encoding.
pub(super) fn parse(encrypted: &[u8]) -> Option<Type1Font> {
    if encrypted.len() > MAX_PROGRAM {
        return None;
    }
    let mut program = b"%!FontType1-1.0: Embedded\n/FontMatrix [0.001 0 0 0.001 0 0] readonly def\ncurrentfile eexec\n".to_vec();
    program.extend_from_slice(encrypted);
    Type1Font::new(&program).ok()
}
