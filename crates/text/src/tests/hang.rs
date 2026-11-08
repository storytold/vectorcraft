//! Roman Hanging Punctuation and Optical Margin Alignment over whole paragraphs: both composers,
//! justification options, mixed directions, mojikumi and burasagari (helpx "Formatting
//! paragraphs" › Hanging punctuation,
//! <https://helpx.adobe.com/illustrator/using/formatting-paragraphs.html>).

use crate::*;
use kurbo::{Affine, Shape};
use vectorcraft_doc::{CharStyle, Composer, Justify, TextKind};
use vectorcraft_geom::PathData;

/// The bundled fonts only: what these tests measure doesn't change with the fonts installed.
fn db() -> &'static FontDb {
    static DB: std::sync::OnceLock<FontDb> = std::sync::OnceLock::new();
    DB.get_or_init(|| FontDb::with_font_dirs(vec![]))
}

fn area(text: &str, size: f64, frame: Rect, justify: Justify) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, text, CharStyle { size, ..CharStyle::default() });
    t.kind = TextKind::Area { frame: PathData::from_bezpath(&frame.to_path(0.1)) };
    t.xf = Affine::IDENTITY;
    t.para.justify = justify;
    t
}

/// The character a glyph sets (an automatic hyphen has no source characters: `-`).
fn ch(t: &TextObject, g: &PositionedGlyph) -> char {
    if g.len == 0 { '-' } else { t.plain_text().get(g.byte..).and_then(|s| s.chars().next()).unwrap_or('\0') }
}

fn is_space(c: char) -> bool {
    c == ' ' || c == '\n'
}

fn is_quote(c: char) -> bool {
    matches!(c, '\'' | '"' | '‘' | '’' | '‚' | '‛' | '“' | '”' | '„' | '‟')
}

/// The published share of a character's advance that hangs (Roman Hanging Punctuation), plus the
/// slanted letters Optical Margin Alignment hangs by a tenth when `optical`.
fn ratio(c: char, optical: bool) -> f64 {
    match c {
        '-' | '\u{2010}' | '\u{2011}' | '.' | ',' => 1.0,
        c if is_quote(c) => 1.0,
        '*' | '~' | '…' | '–' | '—' | ':' | ';' => 0.5,
        'A' | 'V' | 'W' | 'Y' | 'v' | 'w' | 'y' if optical => 0.1,
        _ => 0.0,
    }
}

/// The glyphs of each line, left to right, without the spaces that end it (a left-to-right
/// paragraph's trailing spaces are set at its right, outside the measure).
fn inked<'a>(t: &TextObject, l: &'a TextLayout) -> Vec<(&'a LineInfo, Vec<&'a PositionedGlyph>)> {
    l.lines
        .iter()
        .map(|li| {
            let mut gs: Vec<&PositionedGlyph> = l.glyphs.get(li.glyph_start..li.glyph_end).unwrap_or_default().iter().collect();
            gs.sort_by(|a, b| a.origin.x.total_cmp(&b.origin.x));
            while gs.last().is_some_and(|g| is_space(ch(t, g))) {
                gs.pop();
            }
            (li, gs)
        })
        .collect()
}

/// How far the glyphs at the ends of `gs` hang: the end glyph's share, and a punctuation mark
/// next to a quotation mark at that end hangs with it.
fn hung(t: &TextObject, gs: &[&PositionedGlyph], optical: bool) -> (f64, f64) {
    let r = |g: &PositionedGlyph| ratio(ch(t, g), optical) * g.advance;
    let punct = |g: &PositionedGlyph| ratio(ch(t, g), false) > 0.0;
    let quote = |g: &PositionedGlyph| is_quote(ch(t, g));
    if gs.len() < 2 {
        return (0.0, 0.0);
    }
    let (mut left, mut right) = (0.0, 0.0);
    if let [a, b, rest @ ..] = gs {
        left = r(a);
        if left > 0.0 && !rest.is_empty() && punct(a) && quote(b) {
            left += r(b);
        }
    }
    if let [.., a, b] = gs {
        right = r(b);
        if right > 0.0 && gs.len() > 2 && punct(a) && quote(b) {
            right += r(a);
        }
    }
    (left, right)
}

/// Per line: (first character, how far the line starts before its start edge less what hangs
/// there, last character, how far it ends after its end edge less what hangs there): 0 and 0 for a
/// line that hangs exactly as published.
fn edges(t: &TextObject, l: &TextLayout, optical: bool) -> Vec<(char, f64, char, f64)> {
    inked(t, l)
        .into_iter()
        .filter_map(|(li, gs)| {
            let (first, last) = (gs.first()?, gs.last()?);
            let (left, right) = hung(t, &gs, optical);
            Some((ch(t, first), li.avail.0 - first.origin.x - left, ch(t, last), last.origin.x + last.advance - li.avail.1 - right))
        })
        .collect()
}

const TEXT: &str = "\u{201C}Hanging\u{201D} quotes, commas. And periods. \"Straight\" quotes and well-known \
hyphens, dashes \u{2014} colons: semicolons; and \u{2018}single\u{2019} ones. Extraordinarily comprehensive \
typographical considerations notwithstanding. \u{201C}Again\u{201D}, then more words follow here.";

/// A whole paragraph, ragged and justified, with both composers: every line starts exactly as
/// far outside its start edge as its first mark hangs; right-aligned and justified lines end
/// exactly as far outside the end edge as their last mark hangs, and ragged lines never pass it.
#[test]
fn every_line_of_a_paragraph_hangs_as_published_with_both_composers() {
    let frame = Rect::new(0.0, 0.0, 150.0, 4000.0);
    let (mut quote, mut hyphen, mut period) = (false, false, false);
    for justify in [Justify::Left, Justify::Right, Justify::JustifyLeft] {
        for composer in [Composer::SingleLine, Composer::EveryLine] {
            let mut t = area(TEXT, 12.0, frame, justify);
            t.para.hyphenate = true;
            t.para.composer = composer;
            t.para.roman_hanging = true;
            let on = layout(db(), &t);
            assert!(!on.overflow);
            let n = on.lines.len();
            for (k, (c0, before, c1, after)) in edges(&t, &on, false).into_iter().enumerate() {
                if justify != Justify::Right {
                    assert!(before.abs() < 1e-6, "{justify:?} {composer:?} line {k} starts {c0:?}: {before}");
                    quote |= matches!(c0, '\u{201C}' | '"' | '\u{2018}');
                }
                let full = justify == Justify::Right || (justify == Justify::JustifyLeft && k + 1 < n);
                if full {
                    assert!(after.abs() < 1e-6, "{justify:?} {composer:?} line {k} ends {c1:?}: {after}");
                    hyphen |= c1 == '-';
                    period |= matches!(c1, '.' | ',');
                } else {
                    assert!(after <= 1e-6, "{justify:?} {composer:?} line {k} passes the hung edge: {after}");
                }
            }
        }
    }
    assert!(quote && hyphen && period, "quote {quote}, hyphen {hyphen}, period {period}");
}

/// Justify All Lines with Optical Margin Alignment: every line fills to the hung edges, letters
/// included, with both composers.
#[test]
fn justified_lines_fill_to_the_hung_edges_with_both_composers() {
    for composer in [Composer::SingleLine, Composer::EveryLine] {
        let mut t = area(TEXT, 11.0, Rect::new(0.0, 0.0, 170.0, 4000.0), Justify::JustifyAll);
        t.para.composer = composer;
        t.para.hyphenate = true;
        t.optical_margin = true;
        let l = layout(db(), &t);
        let mut count = 0;
        for (c0, before, c1, after) in edges(&t, &l, true) {
            assert!(before.abs() < 1e-6 && after.abs() < 1e-6, "{composer:?}: {c0:?} {before} / {c1:?} {after}");
            count += usize::from(ratio(c0, true) > 0.0) + usize::from(ratio(c1, true) > 0.0);
        }
        assert!(count >= 3, "{composer:?}: {count}");
    }
}

/// Centred lines centre what doesn't hang: the hung characters stand outside it at either end.
#[test]
fn centred_lines_centre_the_text_between_the_hung_characters() {
    let mut t = area(TEXT, 12.0, Rect::new(0.0, 0.0, 150.0, 4000.0), Justify::Center);
    t.para.roman_hanging = true;
    let l = layout(db(), &t);
    let mut count = 0;
    for (k, (li, gs)) in inked(&t, &l).into_iter().enumerate() {
        let (Some(first), Some(last)) = (gs.first(), gs.last()) else { continue };
        let (left, right) = hung(&t, &gs, false);
        count += usize::from(left > 0.0) + usize::from(right > 0.0);
        let (body_start, body_end) = (first.origin.x + left, last.origin.x + last.advance - right);
        assert!(((body_start - li.avail.0) - (li.avail.1 - body_end)).abs() < 1e-6, "line {k}: {body_start} … {body_end} in {:?}", li.avail);
    }
    assert!(count >= 2, "{count}");
}

/// A right-to-left run inside a left-to-right paragraph: the line hangs by its visual ends, so
/// nothing passes the frame but the mark at the line's visual start or end, by what it hangs.
#[test]
fn right_to_left_runs_in_a_left_to_right_paragraph_stay_inside() {
    let shalom = "\u{05E9}\u{05DC}\u{05D5}\u{05DD}";
    let text = format!("Go {shalom} {shalom} {shalom}. {shalom} {shalom}. abc def");
    for composer in [Composer::SingleLine, Composer::EveryLine] {
        let mut t = area(&text, 12.0, Rect::new(0.0, 0.0, 56.0, 400.0), Justify::JustifyLeft);
        t.para.composer = composer;
        t.para.roman_hanging = true;
        let l = layout(db(), &t);
        assert!(!l.overflow);
        assert!(l.glyphs.iter().any(|g| g.rtl));
        for (li, gs) in inked(&t, &l) {
            let n = gs.len();
            for (k, g) in gs.iter().enumerate() {
                let c = ch(&t, g);
                let lo = if k == 0 && n > 1 { ratio(c, false) * g.advance } else { 0.0 };
                let hi = if k + 1 == n && n > 1 { ratio(c, false) * g.advance } else { 0.0 };
                assert!(g.origin.x >= li.avail.0 - lo - 1e-6, "{composer:?}: {c:?} starts at {} before {}", g.origin.x, li.avail.0);
                assert!(
                    g.origin.x + g.advance <= li.avail.1 + hi + 1e-6,
                    "{composer:?}: {c:?} ends at {} past {}",
                    g.origin.x + g.advance,
                    li.avail.1
                );
            }
        }
    }
}

/// Justification's Glyph Scaling and Letter Spacing leave the hung first and last marks as they
/// are: they hang exactly as published, and the character after a mark hung wholly outside the
/// start stands on the edge.
#[test]
fn glyph_scaling_and_letter_spacing_keep_the_hung_edges() {
    use vectorcraft_doc::JustificationOptions;
    for composer in [Composer::SingleLine, Composer::EveryLine] {
        let mut t = area(TEXT, 12.0, Rect::new(0.0, 0.0, 160.0, 4000.0), Justify::JustifyAll);
        t.para.composer = composer;
        t.para.roman_hanging = true;
        t.para.justification =
            JustificationOptions { letter_spacing: [-5.0, 0.0, 20.0], glyph_scaling: [95.0, 100.0, 105.0], ..JustificationOptions::default() };
        let l = layout(db(), &t);
        let mut count = 0;
        for (c0, before, c1, after) in edges(&t, &l, false) {
            assert!(before.abs() < 1e-6 && after.abs() < 1e-6, "{composer:?}: {c0:?} {before} / {c1:?} {after}");
            count += usize::from(ratio(c0, false) > 0.0) + usize::from(ratio(c1, false) > 0.0);
        }
        assert!(count >= 3, "{composer:?}: {count}");
        for (li, gs) in inked(&t, &l) {
            if let [first, second, ..] = gs[..]
                && ratio(ch(&t, first), false) == 1.0
                && !is_quote(ch(&t, second))
            {
                assert!((second.origin.x - li.avail.0).abs() < 1e-6, "{composer:?}: {:?} at {}", ch(&t, second), second.origin.x);
            }
        }
    }
}

/// Line-end Punctuation Half Width with Roman Hanging Punctuation: both composers set every
/// line on its hung edges (a Japanese bracket isn't Roman punctuation and doesn't hang).
#[test]
fn mojikumi_and_roman_hanging_agree() {
    use vectorcraft_doc::Mojikumi;
    let text = format!("{TEXT} \u{300C}\u{4E00}\u{4E8C}\u{300D} \u{201C}words\u{201D} {TEXT}");
    for composer in [Composer::SingleLine, Composer::EveryLine] {
        let mut t = area(&text, 12.0, Rect::new(0.0, 0.0, 150.0, 4000.0), Justify::JustifyLeft);
        t.para.composer = composer;
        t.para.mojikumi = Mojikumi::LineEndHalf;
        t.para.hyphenate = true;
        t.para.roman_hanging = true;
        let l = layout(db(), &t);
        assert!(!l.overflow);
        let n = l.lines.len();
        for (k, (c0, before, c1, after)) in edges(&t, &l, false).into_iter().enumerate() {
            if c0 != '\u{300C}' {
                assert!(before.abs() < 1e-6, "{composer:?} line {k} starts {c0:?}: {before}");
            }
            if k + 1 < n && !matches!(c1, '\u{300D}' | '\u{3001}' | '\u{3002}') {
                assert!(after.abs() < 1e-6, "{composer:?} line {k} ends {c1:?}: {after}");
            }
        }
    }
}

/// Standard burasagari with a hung opening quote: the quote's hang is room on the line, so a
/// comma that fits once the quote hangs stays inside (ends on the edge) instead of hanging.
/// Needs a font with full-width Japanese punctuation (skipped without one).
#[test]
fn burasagari_counts_the_hung_start() {
    use vectorcraft_doc::Burasagari;
    let lay = |text: &str, roman: bool| {
        let mut t = area(text, 20.0, Rect::new(0.0, 0.0, 100.0, 200.0), Justify::JustifyLeft);
        t.para.burasagari = Burasagari::Standard;
        t.para.roman_hanging = roman;
        // Installed fonts too: the bundled ones have no Japanese.
        layout(FontDb::global(), &t)
    };
    if lay("\u{4E00}\u{3001}\u{4E8C}", false).glyphs.get(1).is_none_or(|g| (g.advance - 20.0).abs() > 2.0) {
        eprintln!("skipping: no font with full-width punctuation");
        return;
    }
    const EPS: f64 = 1e-6;
    let l = lay("\u{201C}\u{4E00}\u{4E8C}\u{4E09}\u{56DB}\u{3001}\u{4E94}\u{516D}\u{4E03}", true);
    // The installed font may not set the nine characters as nine glyphs (a fallback that lacks
    // some, or ligates them): the positions below would then name other characters.
    let (9, Some(quote), Some(comma)) = (l.glyphs.len(), l.glyphs.first(), l.glyphs.get(5)) else {
        eprintln!("skipping: the installed fonts don't set the nine characters as nine glyphs ({})", l.glyphs.len());
        return;
    };
    assert_eq!(comma.line, quote.line, "、 ends the first line");
    assert!((quote.origin.x + quote.advance).abs() < EPS, "the quote hangs: {:?}", quote.origin);
    assert!((comma.origin.x + comma.advance - 100.0).abs() < EPS, "、 inside, on the edge: {:?} {}", comma.origin, comma.advance);
}
