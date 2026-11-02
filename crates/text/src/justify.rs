//! Paragraph panel menu › Justification…: word spacing, letter spacing and glyph scaling.
//!
//! Each has a Minimum, a Desired and a Maximum value in percent
//! ([`vectorcraft_doc::JustificationOptions`]). [`apply_desired`] sets every glyph of a paragraph
//! at the Desired values, so they apply to ragged lines too, and returns how far each glyph may
//! stretch or shrink towards Maximum and Minimum. The every-line composer scores lines by that
//! flexibility, and [`fit_line`] spreads a justified line's leftover space over it: the word spaces
//! first, then the gaps between letters, then the glyphs' widths. A line that still has room left
//! (or too little) takes the rest on its word spaces, as before these options existed.
//!
//! Behaviour reference (public documentation):
//! <https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/adjust-word-and-letterspacing-in-justified-text.html>.

use vectorcraft_doc::JustificationOptions;

use crate::shape::SGlyph;

/// How far one glyph may stretch (`.0`) or shrink (`.1`), in points, in each kind of spacing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Flex {
    /// A word space's width, between Minimum and Maximum Word Spacing.
    pub word: (f64, f64),
    /// The gap after a letter, between Minimum and Maximum Letter Spacing.
    pub letter: (f64, f64),
    /// The glyph's own width, between Minimum and Maximum Glyph Scaling.
    pub glyph: (f64, f64),
    /// The glyph's width at Desired Glyph Scaling, without its letter spacing: what a glyph
    /// scaling adjustment is measured against.
    pub width: f64,
    /// The Desired Glyph Scaling the glyph is drawn at (1: 100%).
    pub scale: f64,
}

impl Default for Flex {
    fn default() -> Self {
        Self { word: (0.0, 0.0), letter: (0.0, 0.0), glyph: (0.0, 0.0), width: 0.0, scale: 1.0 }
    }
}

impl Flex {
    /// All the glyph may stretch.
    pub fn stretch(&self) -> f64 {
        self.word.0 + self.letter.0 + self.glyph.0
    }
    /// All the glyph may shrink.
    pub fn shrink(&self) -> f64 {
        self.word.1 + self.letter.1 + self.glyph.1
    }
}

/// A glyph justification spreads space over as a word space (tabs keep their stops).
pub(crate) fn is_word_space(g: &SGlyph) -> bool {
    g.is_space() && g.ch != '\t'
}

/// A glyph letter spacing follows: a visible glyph followed by another one in the same word.
fn letter_gap(g: &SGlyph, next: Option<&SGlyph>) -> bool {
    !g.is_space() && g.ch != '\u{2028}' && g.inline.is_none() && next.is_some_and(|n| !n.is_space() && n.ch != '\u{2028}')
}

/// The width of a word space in `g`'s font and size: what letter spacing is a share of.
fn space_width(g: &SGlyph) -> f64 {
    let gid = g.face.glyph_for(' ');
    let w = if gid != 0 { g.face.advance(gid) * g.sx } else { g.face.upem * 0.25 * g.sx };
    if w.is_finite() { w.abs() } else { 0.0 }
}

/// Set the glyphs `sg` of a paragraph at `j`'s Desired values and return each glyph's [`Flex`].
/// `scale_glyphs` is false in vertical type, whose glyphs are not scaled across the column.
pub(crate) fn apply_desired(sg: &mut [SGlyph], j: &JustificationOptions, scale_glyphs: bool) -> Vec<Flex> {
    let pct = |v: f64| if v.is_finite() { v / 100.0 } else { 0.0 };
    let [w_min, w_des, w_max] = j.word_spacing.map(pct);
    let [l_min, l_des, l_max] = j.letter_spacing.map(pct);
    let [g_min, g_des, g_max] = if scale_glyphs { j.glyph_scaling.map(pct) } else { [1.0; 3] };
    let mut flex = vec![Flex::default(); sg.len()];
    for k in 0..sg.len() {
        let gap = letter_gap(&sg[k], sg.get(k + 1));
        let (Some(g), Some(f)) = (sg.get_mut(k), flex.get_mut(k)) else { continue };
        if is_word_space(g) {
            // The font's word space (with its tracking), set at Desired.
            let base = g.adv;
            g.adv = base * w_des;
            f.word = (base * (w_max - w_des).max(0.0), base * (w_des - w_min).max(0.0));
            continue;
        }
        if g.is_space() || g.inline.is_some() || g.ch == '\u{2028}' {
            continue;
        }
        let base = g.adv;
        if g_des != 1.0 {
            g.adv *= g_des;
            g.sx *= g_des;
            g.dx *= g_des;
        }
        f.width = g.adv;
        f.scale = g_des;
        f.glyph = (base * (g_max - g_des).max(0.0), base * (g_des - g_min).max(0.0));
        if gap {
            let space = space_width(g);
            g.adv += space * l_des;
            f.letter = (space * (l_max - l_des).max(0.0), space * (l_des - l_min).max(0.0));
        }
    }
    flex
}

/// How a justified line takes up its leftover space: the share of each kind of flexibility used
/// (negative when the line shrinks), and what each word space takes beyond its own range.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct LineFit {
    pub word: f64,
    pub letter: f64,
    pub glyph: f64,
    /// Added to every word space once the ranges are used up (negative: taken off).
    pub per_space: f64,
}

impl LineFit {
    /// The extra advance of glyph `f` that is a word space.
    pub fn space(&self, f: &Flex) -> f64 {
        pick(f.word, self.word) + self.per_space
    }
    /// The extra advance after a letter: its letter spacing.
    pub fn letter(&self, f: &Flex) -> f64 {
        pick(f.letter, self.letter)
    }
    /// How much wider glyph `f` is drawn (1: as it is).
    pub fn glyph_scale(&self, f: &Flex) -> f64 {
        let extra = pick(f.glyph, self.glyph);
        if f.width > 0.0 && extra != 0.0 { (f.width + extra) / f.width } else { 1.0 }
    }
    /// The extra advance of glyph `f` from its scaling.
    pub fn glyph(&self, f: &Flex) -> f64 {
        pick(f.glyph, self.glyph)
    }
}

/// The stretch (share > 0) or shrink (share < 0) of a `(stretch, shrink)` pair.
fn pick(range: (f64, f64), share: f64) -> f64 {
    if share >= 0.0 { range.0 * share } else { range.1 * share }
}

/// Spread `room` (the line's width less its natural width; negative when it is too long) over
/// the glyphs `flex` of a justified line: word spaces first, then letter gaps, then glyph widths,
/// each up to its range. `spaces` counts the line's word spaces, and `shrink_floor` is how much a
/// word space may lose beyond its range (its whole width: never below zero). The rest goes on the
/// word spaces when the line has any.
pub(crate) fn fit_line(flex: &[Flex], room: f64, spaces: usize, shrink_floor: f64) -> LineFit {
    let mut fit = LineFit::default();
    if !room.is_finite() || room == 0.0 {
        return fit;
    }
    let grow = room > 0.0;
    let total = |part: fn(&Flex) -> (f64, f64)| flex.iter().map(|f| if grow { part(f).0 } else { part(f).1 }).sum::<f64>();
    let mut left = room.abs();
    for (part, share) in [
        ((|f: &Flex| f.word) as fn(&Flex) -> (f64, f64), &mut fit.word),
        (|f: &Flex| f.letter, &mut fit.letter),
        (|f: &Flex| f.glyph, &mut fit.glyph),
    ] {
        let cap = total(part);
        if left <= 0.0 || cap <= 0.0 {
            continue;
        }
        let used = left.min(cap);
        *share = if grow { used / cap } else { -used / cap };
        left -= used;
    }
    if left > 1e-9 && spaces > 0 {
        let per = left / spaces as f64;
        fit.per_space = if grow { per } else { -per.min(shrink_floor.max(0.0)) };
    }
    fit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flex(word: (f64, f64), letter: (f64, f64), glyph: (f64, f64), width: f64) -> Flex {
        Flex { word, letter, glyph, width, scale: 1.0 }
    }

    #[test]
    fn word_spaces_take_the_room_first_then_letters_then_glyphs() {
        let line = [flex((2.0, 1.0), (0.0, 0.0), (0.0, 0.0), 0.0), flex((0.0, 0.0), (1.0, 1.0), (4.0, 4.0), 8.0)];
        // Within the word spaces' range.
        assert_eq!(fit_line(&line, 1.0, 1, 3.0), LineFit { word: 0.5, ..LineFit::default() });
        // Spaces full, half of the letter gaps.
        let fit = fit_line(&line, 2.5, 1, 3.0);
        assert_eq!((fit.word, fit.letter, fit.glyph), (1.0, 0.5, 0.0));
        // Everything used, the rest on the word space.
        let fit = fit_line(&line, 10.0, 1, 3.0);
        assert_eq!((fit.word, fit.letter, fit.glyph, fit.per_space), (1.0, 1.0, 1.0, 3.0));
        assert_eq!(fit.glyph_scale(&line[1]), 1.5);
        // Shrinking: the space can't lose more than its width.
        let fit = fit_line(&line, -10.0, 1, 3.0);
        assert_eq!((fit.word, fit.letter, fit.glyph, fit.per_space), (-1.0, -1.0, -1.0, -3.0));
        assert_eq!(fit.space(&line[0]), -4.0);
        // No room to share, or nowhere to put it.
        assert_eq!(fit_line(&line, 0.0, 1, 3.0), LineFit::default());
        assert_eq!(fit_line(&[], 5.0, 0, 0.0), LineFit::default());
    }
}
