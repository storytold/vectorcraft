use super::flow::flow;
use super::region::Region;
use super::*;

/// System Layout on or Fractional Widths off: each character that takes space advances by a whole
/// number of points, at least one, so small type does not collapse onto itself. A zero advance (a
/// tate-chu-yoko continuation, a zero-width mark) stays zero.
pub(super) fn whole_character_widths(glyphs: &mut [SGlyph]) {
    for g in glyphs {
        if g.adv.is_finite() && g.adv > 0.0 {
            g.adv = g.adv.round().max(1.0);
        }
    }
}

/// Tate-chu-yoko: a horizontal block one em of the column long, squeezed to the em when wider.
///
/// Automatic: a run of two or three half-width digits (`10`月, `100`年). A single digit and a
/// longer number (`2026`) stay on their side.
///
/// Manual (Character panel menu › Tate-chu-yoko): any characters marked `tateChuYoko`, including
/// one character, letters and numbers longer than three digits. Marked characters are taken
/// before the automatic digit rule.
/// <https://helpx.adobe.com/illustrator/using/formatting-asian-characters.html>
pub(super) fn tate_chu_yoko(g: &mut [SGlyph], style: impl Fn(&SGlyph) -> (f64, bool)) {
    let manual: Vec<bool> = g.iter().map(|gl| style(gl).1).collect();
    let digits: Vec<bool> = g.iter().map(|gl| gl.inline.is_none() && gl.ch.is_ascii_digit()).collect();
    let mut i = 0;
    while i < g.len() {
        if manual.get(i).copied().unwrap_or(false) {
            let mut end = i + 1;
            while manual.get(end).copied().unwrap_or(false) {
                end += 1;
            }
            let em = style(&g[i]).0;
            place_tcy_block(&mut g[i..end], em);
            i = end;
            continue;
        }
        if !digits.get(i).copied().unwrap_or(false) {
            i += 1;
            continue;
        }
        let mut end = i + 1;
        while digits.get(end).copied().unwrap_or(false) && !manual.get(end).copied().unwrap_or(false) {
            end += 1;
        }
        if (2..=3).contains(&(end - i)) {
            let em = style(&g[i]).0;
            place_tcy_block(&mut g[i..end], em);
        }
        i = end;
    }
}

/// Lay `g` across one em of the column. The first glyph takes the em; the rest share its cell.
fn place_tcy_block(g: &mut [SGlyph], em: f64) {
    let width: f64 = g.iter().map(|gl| gl.adv).sum();
    let squeeze = if width > em && width > 0.0 { em / width } else { 1.0 };
    let mut ink = 0.0;
    for (k, gl) in g.iter_mut().enumerate() {
        let adv = gl.adv;
        gl.tcy = Some(Tcy { pen: if k == 0 { 0.0 } else { em }, ink, width, squeeze });
        gl.adv = if k == 0 { em } else { 0.0 };
        ink += adv;
    }
}

/// Half an em of `g`'s size when it is full-width Japanese punctuation of kind `kind` (its
/// advance an em, give or take a tenth), the space that mojikumi can take off.
pub(super) fn punct_half(g: &SGlyph, kind: Punct) -> Option<f64> {
    // A glyph Proportional Metrics re-spaced has no empty half left to take off (#966).
    if punct(g.ch)? != kind || g.tcy.is_some() || g.proportional {
        return None;
    }
    let em = g.face.units_per_em();
    ((g.face.advance(g.gid) - em).abs() <= em * 0.1).then_some(em * 0.5 * g.sx)
}

/// Mojikumi (JLREQ 3.1.4): consecutive punctuation shares one half-em space. A closing bracket,
/// comma or full stop followed by punctuation loses the space after it; an opening bracket after
/// another loses the space before it. Returns which glyphs lost the space after them (a line
/// ending in one takes nothing more off).
pub(super) fn compress_punctuation(sg: &mut [SGlyph]) -> Vec<bool> {
    let mut lost_after = vec![false; sg.len()];
    for j in 1..sg.len() {
        let (before, after) = sg.split_at_mut(j);
        let (Some(a), Some(b)) = (before.last_mut(), after.first_mut()) else { continue };
        if punct(b.ch).is_none() {
            continue;
        }
        if let Some(h) = punct_half(a, Punct::Closing) {
            a.adv -= h;
            if let Some(l) = lost_after.get_mut(j - 1) {
                *l = true;
            }
        } else if punct(a.ch) == Some(Punct::Opening)
            && let Some(h) = punct_half(b, Punct::Opening)
        {
            b.adv -= h;
            b.lead += h;
        }
    }
    lost_after
}

/// The space between Japanese and Latin letters or digits (JLREQ 3.2.2: a quarter em of the
/// Japanese characters' size), with Line-end Punctuation Half Width.
const WAKAN_AKI: f64 = 0.25;

/// A Japanese character for the space next to Latin text: kana and kanji (and full-width
/// letters, and the iteration and abbreviation marks 々 〆 〇), not punctuation or symbols, nor
/// Hangul (Korean sets a word space instead).
fn is_japanese_letter(c: char) -> bool {
    if matches!(c, '々' | '〆' | '〇') {
        return true;
    }
    is_cjk(c)
        && !matches!(
            c as u32,
            0x3000..=0x303F | 0x3130..=0x318F | 0xAC00..=0xD7AF | 0xFF01..=0xFF0F | 0xFF1A..=0xFF20 | 0xFF3B..=0xFF40 | 0xFF5B..=0xFF65 | 0xFFA0..=0xFFDC
        )
        && c != '・'
        && punct(c).is_none()
}

/// A Latin letter or digit (or another script's letter), set proportionally.
fn is_latin_letter(c: char) -> bool {
    c.is_alphanumeric() && !is_cjk(c)
}

/// Mojikumi (JLREQ 3.2.2): a quarter em between a Japanese character and a Latin letter or digit,
/// either way round, added after the first of the two. Returns what each glyph got after it (taken
/// off again when the line ends there).
pub(super) fn space_japanese_and_latin(sg: &mut [SGlyph]) -> Vec<f64> {
    let mut added = vec![0.0; sg.len()];
    for j in 1..sg.len() {
        let (before, after) = sg.split_at_mut(j);
        let (Some(a), Some(b)) = (before.last_mut(), after.first()) else { continue };
        if a.tcy.is_some() || b.tcy.is_some() {
            continue;
        }
        let japanese = if is_japanese_letter(a.ch) && is_latin_letter(b.ch) {
            Some(&*a)
        } else if is_latin_letter(a.ch) && is_japanese_letter(b.ch) {
            Some(b)
        } else {
            None
        };
        if let Some(g) = japanese {
            let aki = WAKAN_AKI * g.face.units_per_em() * g.sx;
            a.adv += aki;
            if let Some(x) = added.get_mut(j - 1) {
                *x = aki;
            }
        }
    }
    added
}

/// The size of `g`'s em, in points.
pub(super) fn glyph_em(g: &SGlyph) -> f64 {
    g.face.units_per_em() * g.sy
}

/// How far up (line space) Character Alignment `a` moves glyph `g` on a line whose largest em is
/// `line_em`: the glyph's em box top, centre or bottom onto the line's (the em box running from
/// its centre less half an em to its centre plus half an em above the baseline), or its ICF's top
/// or bottom (the em box's less the face's [`crate::IcfMargins`]; across a vertical line, its
/// right and left). Nothing on the Roman baseline, or for the line's largest characters.
///
/// An inline graphic has its run's em (whatever its scale): it counts as one of the line's
/// largest characters only when its run's size is, and moves with the text of its run (its art
/// stays centred on that text's cap height).
pub(super) fn align_shift(g: &SGlyph, a: vectorcraft_doc::CharAlign, line_em: f64, vertical: bool) -> f64 {
    use vectorcraft_doc::CharAlign;
    let icf = || g.face.icf_margins();
    let k = match a {
        CharAlign::RomanBaseline => return 0.0,
        CharAlign::EmBoxTop => 0.5,
        CharAlign::EmBoxCenter => 0.0,
        CharAlign::EmBoxBottom => -0.5,
        CharAlign::IcfTop => 0.5 - if vertical { icf().right } else { icf().top },
        CharAlign::IcfBottom => -0.5 + if vertical { icf().left } else { icf().bottom },
    };
    (g.face.ideographic_centre() + k) * (line_em - glyph_em(g)).max(0.0)
}

/// The length an upright glyph takes down the column before tracking and justification: its
/// vertical advance (the font's vertical metrics), else its advance, at least one em.
pub(super) fn upright_cell(g: &SGlyph) -> f64 {
    match g.face.vertical_glyph(g.gid) {
        Some((advance, _)) => advance * g.sy,
        None => (g.face.advance(g.gid) * g.sx).max(g.face.units_per_em() * g.sx),
    }
}

/// Height above the baseline (line space) of the centre of an upright glyph's `cell`, the point it
/// turns about: from the font's vertical metrics, the cell hanging from the glyph's vertical origin;
/// else the centre of the ideographic em box ([`EM_CENTER`] of the size `em`).
pub(super) fn upright_centre(g: &SGlyph, cell: f64, em: f64) -> f64 {
    match g.face.vertical_glyph(g.gid) {
        Some((_, origin)) => origin * g.sy - cell * 0.5,
        None => EM_CENTER * em,
    }
}

/// Height of the centre of the ideographic em box above the baseline, in ems (the em box runs
/// from 0.12 em below the baseline to 0.88 em above it), for fonts without vertical metrics.
pub(super) const EM_CENTER: f64 = 0.38;

/// A quarter turn clockwise (y down), exact: line space → text space for vertical type.
pub(super) const QUARTER_TURN: Affine = Affine::new([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]);

/// Does the glyph stand upright in vertical type? CJK characters and symbols do (their vertical
/// forms come from the font's `vert` feature); Latin letters, digits and other characters lie on
/// their side. Brackets, the long vowel mark and similar marks that need a vertical form lie on
/// their side when the font has none (Unicode's Vertical_Orientation Tr).
/// Fullwidth !/? instead keep the nominal glyph upright (Tu fallback, UAX #50 §3.1):
/// <https://www.unicode.org/reports/tr50/#vo>.
pub(super) fn stands_upright(g: &SGlyph) -> bool {
    let c = g.ch as u32;
    let upright = matches!(c,
        0x00A7 | 0x00A9 | 0x00AE | 0x00B1 | 0x00BC..=0x00BE | 0x00D7 | 0x00F7
        | 0x2016 | 0x2020 | 0x2021 | 0x2030 | 0x2031 | 0x203B | 0x203C | 0x2042 | 0x2047..=0x2049 | 0x2051
        | 0x20DD..=0x20E0 | 0x20E2..=0x20E4 | 0x2100..=0x2101 | 0x2103..=0x2109 | 0x210F | 0x2113 | 0x2116 | 0x2117
        | 0x211E..=0x2123 | 0x2125 | 0x2127 | 0x2129 | 0x212E | 0x2135..=0x213F | 0x2145..=0x214A | 0x214C | 0x214D | 0x214F..=0x2189
        | 0x2460..=0x24FF | 0x25A0..=0x27BF | 0x2B12..=0x2B2F | 0x2B50..=0x2B59
        | 0x1100..=0x11FF | 0x2E80..=0x303F | 0x3040..=0xA4CF | 0xA960..=0xA97F | 0xAC00..=0xD7FF
        | 0xE000..=0xFAFF | 0xFE10..=0xFE1F | 0xFE30..=0xFE4F | 0xFE50..=0xFE6F | 0xFF00..=0xFFE7
        | 0x1F000..=0x1FAFF | 0x20000..=0x3FFFF);
    if !upright {
        return false;
    }
    let needs_vertical_form = matches!(c,
        0x3001 | 0x3002 | 0x3008..=0x3011 | 0x3013..=0x301F | 0x3030 | 0x30A0 | 0x30FC
        | 0xFE50..=0xFE52 | 0xFE59..=0xFE5E | 0xFF08 | 0xFF09 | 0xFF0C | 0xFF0E | 0xFF1A | 0xFF1B
        | 0xFF3B | 0xFF3D | 0xFF3F | 0xFF5B..=0xFF60 | 0xFFE3);
    // Without a vertical alternate the shaped glyph is the nominal one.
    !needs_vertical_form || g.gid != g.face.glyph_for(g.ch)
}

pub(super) fn finish_bounds(out: &mut TextLayout) {
    let xf = out.line_xf;
    let mut b: Option<Rect> = None;
    let mut add = |r: Rect| b = Some(b.map_or(r, |b| b.union(r)));
    for g in &out.glyphs {
        if !g.outline.elements().is_empty() {
            add(g.outline.bounding_box());
        }
        if let Some((image, xf)) = g.placed_image() {
            add(xf.transform_rect_bbox(image.rect()));
        }
        if let Some(b) = g.colour.as_ref().and_then(|c| c.geometric_bounds()) {
            add(g.xf.transform_rect_bbox(b));
        }
    }
    // Ruby: covered by its line's ascent, but its ink can overhang sideways past the line.
    for g in &out.ruby {
        if !g.outline.elements().is_empty() {
            add(g.outline.bounding_box());
        }
    }
    for i in &out.inlines {
        add(i.bounds);
    }
    if !out.on_path {
        for l in &out.lines {
            add(xf.transform_rect_bbox(Rect::new(l.x0.min(l.x1), l.baseline - l.ascent, l.x0.max(l.x1), l.baseline + l.descent)));
        }
    }
    out.bounds = b.unwrap_or_default();
}

/// Space left in each cell after flowing top-aligned: (space above the first line's ascent, space
/// below the last line's descent, number of distinct baselines). Cells without lines are `None`.
fn cell_space(out: &TextLayout, regions: &[Region]) -> Vec<Option<(f64, f64, usize)>> {
    regions
        .iter()
        .enumerate()
        .map(|(ri, r)| {
            let mut top = f64::INFINITY;
            let mut bottom = f64::NEG_INFINITY;
            let mut baselines: Vec<f64> = vec![];
            for l in out.lines.iter().filter(|l| l.region == ri) {
                top = top.min(l.baseline - l.ascent);
                bottom = bottom.max(l.baseline + l.descent);
                if !baselines.iter().any(|b| (b - l.baseline).abs() < 1e-6) {
                    baselines.push(l.baseline);
                }
            }
            if baselines.is_empty() || !top.is_finite() || !bottom.is_finite() {
                return None;
            }
            // Never negative: a full (or overflowing) cell stays where top alignment put it.
            Some(((top - r.top()).max(0.0), (r.bottom() - bottom).max(0.0), baselines.len()))
        })
        .collect()
}

/// Area Type Options "Align" other than Top: move each cell's lines down. Rectangular cells
/// without text wrap shift their lines (centre: half the space left below the last line;
/// bottom: all of it; justify: the first line stays, each further line gets an equal share of it).
/// Other frames give lines different widths at different heights, so the text flows again with
/// each cell's lines started lower (or spaced wider) until it settles, at most eight passes; a
/// pass that would push text out of the frame is undone and retried with half the step.
pub(super) fn align_vertically(cx: &mut Ctx<'_>, paras: &[Range<usize>], t: &TextObject, regions: &mut [Region]) {
    let align = cx.opts.vertical_align;
    let plain = regions.iter().all(|r| (r.rect || r.polys.is_empty()) && r.wraps.is_empty());
    if plain {
        let space = cell_space(&cx.out, regions);
        // Each distinct baseline's index within its cell, for justify.
        let mut seen: Vec<Vec<f64>> = vec![vec![]; regions.len()];
        let offs: Vec<f64> = cx
            .out
            .lines
            .iter()
            .map(|l| {
                let Some(Some((_, below, n))) = space.get(l.region).copied() else { return 0.0 };
                match align {
                    VerticalAlign::Top => 0.0,
                    VerticalAlign::Center => below * 0.5,
                    VerticalAlign::Bottom => below,
                    VerticalAlign::Justify => {
                        let Some(s) = seen.get_mut(l.region) else { return 0.0 };
                        let k = s.iter().position(|b| (b - l.baseline).abs() < 1e-6).unwrap_or_else(|| {
                            s.push(l.baseline);
                            s.len() - 1
                        });
                        if n > 1 { below * k as f64 / (n - 1) as f64 } else { 0.0 }
                    }
                }
            })
            .collect();
        for (l, d) in cx.out.lines.iter_mut().zip(&offs) {
            l.baseline += d;
        }
        for g in &mut cx.out.glyphs {
            let d = offs.get(g.line).copied().unwrap_or(0.0);
            if d != 0.0 {
                let m = Affine::translate((0.0, d));
                g.outline.apply_affine(m);
                g.xf = m * g.xf;
                g.origin.y += d;
            }
        }
        // Inline art moves with its glyph's line.
        for i in &mut cx.out.inlines {
            let d = cx.out.glyphs.get(i.glyph).and_then(|g| offs.get(g.line)).copied().unwrap_or(0.0);
            if d != 0.0 {
                let m = Affine::translate((0.0, d));
                i.xf = m * i.xf;
                i.bounds = m.transform_rect_bbox(i.bounds);
            }
        }
        return;
    }
    let laid_out = |out: &TextLayout| out.lines.last().map_or(0, |l| l.end);
    // How much of the measured space a pass takes up: halved after a pass that lost text.
    let mut step = 1.0;
    for _ in 0..8 {
        let space = cell_space(&cx.out, regions);
        let mut moved = false;
        let saved: Vec<(f64, f64)> = regions.iter().map(|r| (r.shift, r.gap)).collect();
        for (r, s) in regions.iter_mut().zip(&space) {
            let Some((above, below, n)) = *s else { continue };
            let (shift, gap) = match align {
                VerticalAlign::Top => (r.shift, r.gap),
                VerticalAlign::Center => (r.shift + (below - above) * 0.5 * step, r.gap),
                VerticalAlign::Bottom => (r.shift + below * step, r.gap),
                VerticalAlign::Justify if n > 1 => (r.shift, r.gap + below * step / (n - 1) as f64),
                VerticalAlign::Justify => (r.shift, r.gap),
            };
            let (shift, gap) = (shift.clamp(0.0, r.cell.height().max(0.0)), gap.clamp(0.0, r.cell.height().max(0.0)));
            if (shift - r.shift).abs() > 0.25 || (gap - r.gap).abs() > 0.01 {
                moved = true;
            }
            r.shift = shift;
            r.gap = gap;
        }
        if !moved {
            return;
        }
        let before = laid_out(&cx.out);
        let prev = (
            std::mem::take(&mut cx.out.glyphs),
            std::mem::take(&mut cx.out.lines),
            std::mem::take(&mut cx.out.inlines),
            std::mem::take(&mut cx.out.markers),
            std::mem::replace(&mut cx.out.overflow, false),
        );
        flow(cx, paras, t, Some(regions));
        if laid_out(&cx.out) < before {
            // Text no longer fits (lines got narrower, or flow around a wrap object): undo the
            // pass and try a smaller step.
            (cx.out.glyphs, cx.out.lines, cx.out.inlines, cx.out.markers, cx.out.overflow) = prev;
            for (r, (shift, gap)) in regions.iter_mut().zip(saved) {
                r.shift = shift;
                r.gap = gap;
            }
            step *= 0.5;
            if step < 0.1 {
                return;
            }
        }
    }
}

#[cfg(test)]
#[test]
fn japanese_letters_for_the_latin_space_are_kana_kanji_and_marks_not_hangul_or_punctuation() {
    for c in ['あ', 'カ', '漢', '々', '〆', '〇', 'Ａ'] {
        assert!(is_japanese_letter(c), "{c}");
    }
    // Hangul in every form: syllables, compatibility jamo (ㄱ ㅏ), half-width jamo (ﾡ).
    for c in ['한', 'ㄱ', 'ㅏ', '\u{FFA1}', '。', '「', '・', '、', 'a'] {
        assert!(!is_japanese_letter(c), "{c}");
    }
}
