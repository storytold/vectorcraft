//! Line breaking and glyph placement for point, area and on-path type.

use std::ops::Range;

use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape, Vec2};
use unicode_bidi::{BidiInfo, Level};
use vectorcraft_doc::{
    Burasagari, CharStyle, InlineArt, Justify, Kinsoku, LeadingModel, Mojikumi, ParaDirection, ParaStyle, PathAlign, PathEffect, TextKind, TextObject,
};
use vectorcraft_geom::{ArcPath, PathData};

use crate::composer::{Breakpoint, Params, compose};
use crate::fontdb::FontDb;
use crate::hyphen::hyphen_points;
use crate::shape::{
    Punct, SGlyph, Tcy, cap_x_heights, hyphen_glyph, is_cjk, no_line_end, no_line_start, punct, shape_range, soft_line_start, style_metrics,
};
use crate::{Composer, FirstBaseline, InlineGlyph, LayoutOptions, LineInfo, OtFeatures, PositionedGlyph, TextLayout, VerticalAlign};

const EPS: f64 = 1e-6;

struct Ctx<'a> {
    on_path: bool,
    db: &'a FontDb,
    text: &'a str,
    runs: Vec<(Range<usize>, &'a CharStyle)>,
    /// The inline graphic of each run that is one (parallel to `runs`).
    inlines: Vec<Option<&'a InlineArt>>,
    default: CharStyle,
    opts: &'a LayoutOptions,
    /// Vertical type: lines are laid out as horizontal lines in line space, upright glyphs turned
    /// a quarter turn back (see [`stands_upright`]); [`TextLayout::line_xf`] then stands the lines
    /// up as columns.
    vertical: bool,
    out: TextLayout,
}

impl Ctx<'_> {
    fn style_at(&self, b: usize) -> &CharStyle {
        let find = |b: usize| self.runs.iter().find(|(r, _)| r.start <= b && b < r.end).map(|(_, s)| *s);
        find(b).or_else(|| b.checked_sub(1).and_then(find)).or_else(|| self.runs.first().map(|(_, s)| *s)).unwrap_or(&self.default)
    }

    /// Shape paragraph `r` (its bidi resolution `bidi`).
    fn shape_para(&self, r: Range<usize>, bidi: Option<&BidiInfo<'_>>) -> Vec<SGlyph> {
        let mut v = Vec::with_capacity(r.len());
        let levels = bidi.map_or(&[][..], |b| &b.levels);
        shape_range(self.db, self.text, r, &self.runs, &self.inlines, &self.opts.features, levels, &mut v);
        if self.vertical {
            tate_chu_yoko(&mut v, |g| self.style_at(g.byte).size);
            // An upright glyph advances down the column by its vertical advance (the font's vertical
            // metrics; without them at least one em, as CJK fonts' is), centred across it: a narrow
            // mark like § must not overlap its neighbours.
            for g in v.iter_mut().filter(|g| g.adv > 0.0 && stands_upright(g)) {
                let extra = upright_cell(g) - g.face.advance(g.gid) * g.sx;
                g.dx += extra * 0.5;
                g.adv += extra;
                // Proportional Metrics: `vpal` moves it and changes its length down the column,
                // in fonts with vertical metrics (else the cell stays as it is, #966).
                if let Some(a) = g.vpal.filter(|_| g.tcy.is_none() && g.face.vertical_glyph(g.gid).is_some()) {
                    g.dx += a.dx;
                    g.dy += a.dy;
                    g.adv += a.advance;
                    g.proportional = a.advance != 0.0;
                }
            }
        }
        v
    }

    fn emit(&mut self, g: &SGlyph, pre: Affine, origin: Point, angle: f64, advance: f64, line: usize) {
        let src = if g.inline.is_some() { std::sync::Arc::new(BezPath::new()) } else { self.db.outline(&g.face, g.gid) };
        // A glyph whose leading space was taken off (mojikumi) is drawn that much earlier: an
        // upright one in vertical type by moving it up the column once it stands upright.
        let upright = self.vertical && !self.on_path && g.tcy.is_none() && stands_upright(g);
        let lead = if upright { 0.0 } else { g.lead };
        let local = match &g.inline {
            // Inline graphics: art space to glyph space (no font outline).
            Some(ib) => Affine::translate((-lead, -g.bshift)) * ib.xf,
            None => {
                Affine::rotate(-g.rotation.to_radians()) * Affine::translate((g.dx - lead, g.dy - g.bshift)) * Affine::scale_non_uniform(g.sx, g.sy)
            }
        };
        let mut m = pre * local;
        if self.vertical && self.on_path {
            // Vertical path type keeps the baseline path and turns each glyph across it.
            m = Affine::rotate_about(std::f64::consts::FRAC_PI_2, origin) * m;
        } else if let (true, Some(t)) = (self.vertical, g.tcy) {
            // Tate-chu-yoko: the block set across the column, centred on its em of it.
            let em = self.style_at(g.byte).size;
            let start = origin.x - t.pen;
            let centre = Point::new(start + em * 0.5, origin.y - g.face.ideographic_centre() * em);
            let across = Affine::translate((centre.x - t.width * 0.5 + t.ink - origin.x, 0.0));
            let squeeze = Affine::translate((centre.x, 0.0)) * Affine::scale_non_uniform(t.squeeze, 1.0) * Affine::translate((-centre.x, 0.0));
            m = Affine::rotate_about(-std::f64::consts::FRAC_PI_2, centre) * squeeze * across * m;
        } else if self.vertical && stands_upright(g) {
            // Turned about the centre of its cell, which the column's centre line runs through: the
            // middle of its own cell, not of its advance (tracking and justification add space
            // after the cell, and must not push the glyph off the centre line).
            let em = self.style_at(g.byte).size;
            let cell = if g.adv > 0.0 { upright_cell(g) } else { advance };
            // A leading space taken off moves it up the column after the turn (moving the
            // turning point instead would move it across the column too).
            m = Affine::translate((-g.lead, 0.0))
                * Affine::rotate_about(-std::f64::consts::FRAC_PI_2, Point::new(origin.x + cell * 0.5, origin.y - upright_centre(g, cell, em)))
                * m;
        }
        // Control characters (tabs) and soft hyphens draw nothing (fonts map them to .notdef).
        let outline = if src.elements().is_empty() || g.is_soft_hyphen() || g.ch.is_control() {
            BezPath::new()
        } else {
            let mut p = BezPath::with_capacity(src.elements().len());
            for el in src.elements() {
                p.push(m * *el);
            }
            p
        };
        let font_id = g.face.id();
        if let Some(art) = g.inline.as_ref().and_then(|ib| ib.art) {
            self.out.inlines.push(InlineGlyph { run: g.run, byte: g.byte, glyph: self.out.glyphs.len(), xf: m, bounds: m.transform_rect_bbox(art) });
        }
        self.out.glyphs.push(PositionedGlyph {
            outline,
            run: g.run,
            byte: g.byte,
            origin,
            advance,
            len: g.len,
            angle,
            line,
            font_id,
            gid: g.gid,
            xf: m,
            // Vertical type keeps its logical order down the column.
            rtl: g.level.is_rtl() && !self.vertical,
        });
    }
}

/// Lay out a text object into text-space glyph outlines (default [`LayoutOptions`]).
pub fn layout(db: &FontDb, t: &TextObject) -> TextLayout {
    let a = &t.area;
    let opts = LayoutOptions {
        rows: a.rows,
        columns: a.columns,
        gutter: a.gutter,
        inset: a.inset,
        first_baseline: a.first_baseline,
        first_baseline_min: a.first_baseline_min,
        vertical_align: a.vertical_align,
        fit: a.fit,
        ..LayoutOptions::default()
    };
    layout_with(db, t, &opts)
}

/// The underline and strikethrough bars of `t` laid out as `layout` (#847): per run and line, a
/// bar from the stretch's first glyph to its last one's advance, placed and sized by the font's
/// own underline and strikeout metrics through the glyph's transform (so the size, baseline shift
/// and scaling follow) → (run, bar in text space). Type on a path and vertical type have none yet.
pub fn decorations(layout: &TextLayout, db: &FontDb, t: &TextObject) -> Vec<(usize, BezPath)> {
    let mut out = vec![];
    if layout.on_path || layout.vertical {
        return out;
    }
    let glyphs = &layout.glyphs;
    let mut i = 0;
    while let Some(g) = glyphs.get(i) {
        // The stretch: the glyphs of this run on this line, in a row.
        let end = glyphs.iter().skip(i).position(|h| h.run != g.run || h.line != g.line).map_or(glyphs.len(), |n| i + n);
        let stretch = glyphs.get(i..end).unwrap_or_default();
        i = end.max(i + 1);
        let Some(style) = t.runs.get(g.run).map(|r| &r.style) else { continue };
        if !(style.underline || style.strikethrough) {
            continue;
        }
        let Some(face) = db.face_by_id(g.font_id) else { continue };
        let (x0, x1) = stretch.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), h| {
            let (s, e) = (h.origin.x, h.origin.x + h.advance);
            (a.min(s).min(e), b.max(s).max(e))
        });
        for (on, (offset, thickness)) in [(style.underline, face.underline), (style.strikethrough, face.strikeout)] {
            if !on {
                continue;
            }
            // Font units run y down in the glyph's transform: the bar's top is at -offset.
            let (top, bottom) = ((g.xf * Point::new(0.0, -offset)).y, (g.xf * Point::new(0.0, -offset + thickness)).y);
            let bar = Rect::new(x0, top.min(bottom), x1, top.max(bottom));
            if bar.width() > 0.0 && bar.height() > 0.0 && bar.width().is_finite() && bar.height().is_finite() {
                out.push((g.run, bar.to_path(0.1)));
            }
        }
    }
    out
}

/// Most layout passes Shrink Text to Fit runs past the first (bisection plus the final pass).
const SHRINK_PASSES: usize = 11;

/// Lay out a text object with explicit options (area type rows/columns, inset, first baseline,
/// fit, composer, OpenType features).
pub fn layout_with(db: &FontDb, t: &TextObject, opts: &LayoutOptions) -> TextLayout {
    let first = layout_once(db, t, opts);
    match (opts.fit.min_scale(), &t.kind) {
        (Some(min), TextKind::Area { .. }) if first.overflow => shrink_to_fit(db, t, opts, min).unwrap_or(first),
        _ => first,
    }
}

/// Shrink Text to Fit for text that overflows at full size: the largest scale in `min..1` at which
/// it fits, searched by bisection over the first run's document size in steps of 0.1 pt (so its scaled size
/// is a whole number of tenths of a point, the same every time). Down at `min` it lays out there,
/// still overflowing. None (keep the full-size layout) when the text has no usable size.
fn shrink_to_fit(db: &FontDb, t: &TextObject, opts: &LayoutOptions, min: f64) -> Option<TextLayout> {
    // Exact size edits after object scaling may store a local size below 0.1. Fit in the
    // same document points as the Character controls, independent of that representation.
    let points = t.style_scale()?.points;
    let size = t.runs.first().map(|r| r.style.size * points).filter(|s| s.is_finite() && *s >= 0.1)?;
    if !(min.is_finite() && min < 1.0) {
        return None;
    }
    let tenths = size * 10.0;
    if !tenths.is_finite() {
        return None;
    }
    // Candidate sizes k/10 pt for k in lo..hi: `hi` (full size, rounded down) is known to overflow
    // unless it is below the full size; `lo` is the smallest allowed.
    let mut lo = (tenths * min).ceil().max(1.0);
    let mut hi = tenths.floor();
    if hi < lo {
        hi = lo;
    }
    let at = |k: f64| -> TextLayout {
        let f = (k / tenths).clamp(min, 1.0);
        let mut l = layout_once(db, &scaled(t, f), opts);
        l.fit_scale = f;
        l
    };
    let mut passes = 0;
    let mut best: Option<TextLayout> = None;
    // `hi` itself may fit when the size isn't a whole number of tenths.
    if hi < tenths && hi > lo {
        let l = at(hi);
        passes += 1;
        if !l.overflow {
            return Some(l);
        }
    }
    // Invariant: everything at `hi` or above overflows; `lo` fits or is the floor.
    while hi - lo > 1.0 && passes + 1 < SHRINK_PASSES {
        let mid = (lo + (hi - lo) * 0.5).floor();
        let l = at(mid);
        passes += 1;
        if l.overflow {
            hi = mid;
        } else {
            lo = mid;
            best = Some(l);
        }
    }
    // `best` is the layout at `lo` once `lo` has moved; at the floor it still needs laying out.
    Some(best.unwrap_or_else(|| at(lo)))
}

/// `t` with every run's size, explicit leading and baseline shift scaled by `f`, inline art's own
/// shift included (auto leading and the art's size follow the size; paragraph spacing stays).
fn scaled(t: &TextObject, f: f64) -> TextObject {
    let mut s = t.clone();
    for r in &mut s.runs {
        r.style.size *= f;
        r.style.leading = r.style.leading.map(|l| l * f);
        r.style.baseline_shift *= f;
        if let Some(a) = &mut r.inline {
            a.baseline_shift *= f;
        }
    }
    s
}

/// One layout pass, without fitting.
fn layout_once(db: &FontDb, t: &TextObject, opts: &LayoutOptions) -> TextLayout {
    let text = t.plain_text();
    let mut runs = Vec::with_capacity(t.runs.len());
    let mut inlines = Vec::with_capacity(t.runs.len());
    let mut off = 0;
    for r in &t.runs {
        runs.push((off..off + r.text.len(), &r.style));
        // Only a well-formed inline run (one object replacement character) is a graphic.
        inlines.push(r.inline.as_ref().filter(|_| crate::edit::is_inline_text(&r.text)));
        off += r.text.len();
    }
    let mut paras = Vec::new();
    let mut s = 0;
    for (i, c) in text.char_indices() {
        if c == '\n' {
            paras.push(s..i);
            s = i + 1;
        }
    }
    paras.push(s..text.len());
    let vertical = t.vertical;
    let is_on_path = matches!(&t.kind, TextKind::OnPath { .. });
    let vopts;
    let opts = if vertical {
        vopts = LayoutOptions { features: OtFeatures { vertical: true, ..opts.features }, ..opts.clone() };
        &vopts
    } else {
        opts
    };
    let mut cx =
        Ctx { on_path: is_on_path, db, text: &text, runs, inlines, default: CharStyle::default(), opts, vertical, out: TextLayout::default() };
    // Vertical type: line space turned a quarter turn clockwise (lines become columns, each next
    // one to the left). Point type's anchor is on the first column's centre line. Vertical path
    // type stays on its path (each glyph turned across it in `emit`).
    let line_xf = match &t.kind {
        _ if !vertical => Affine::IDENTITY,
        TextKind::OnPath { .. } => Affine::IDENTITY,
        TextKind::Point => {
            let first = cx.style_at(0);
            let centre =
                db.face_version(&first.font_family, &first.font_style, first.font_version.as_deref()).map_or(EM_CENTER, |f| f.ideographic_centre());
            Affine::translate((-centre * first.size, 0.0)) * QUARTER_TURN
        }
        _ => QUARTER_TURN,
    };
    match &t.kind {
        TextKind::Point => flow(&mut cx, &paras, t, None),
        TextKind::Area { frame } => {
            let to_lines = line_xf.inverse();
            let wrap: Vec<vectorcraft_doc::WrapShape> = if vertical {
                t.wrap.iter().map(|w| vectorcraft_doc::WrapShape { path: w.path.transformed(to_lines), ..w.clone() }).collect()
            } else {
                t.wrap.clone()
            };
            let mut regions = Region::cells(&(to_lines * frame.to_bezpath()), opts, &wrap);
            cx.out.frames = regions.iter().map(|r| line_xf.transform_rect_bbox(r.cell)).collect();
            flow(&mut cx, &paras, t, Some(&regions));
            if opts.vertical_align != VerticalAlign::Top {
                align_vertically(&mut cx, &paras, t, &mut regions);
            }
        }
        TextKind::OnPath { path, .. } => on_path(&mut cx, &paras, t, path),
    }
    if vertical && !is_on_path {
        // Glyph origins, outlines and transforms to text space (lines stay in line space).
        for g in &mut cx.out.glyphs {
            g.outline.apply_affine(line_xf);
            g.xf = line_xf * g.xf;
            g.origin = line_xf * g.origin;
            g.angle += std::f64::consts::FRAC_PI_2;
        }
        for i in &mut cx.out.inlines {
            i.xf = line_xf * i.xf;
            i.bounds = line_xf.transform_rect_bbox(i.bounds);
        }
        cx.out.vertical = true;
        cx.out.line_xf = line_xf;
    }
    finish_bounds(&mut cx.out);
    cx.out
}

/// Tate-chu-yoko: a run of two or three half-width digits (`10`月, `100`年) stands upright across
/// the column as one block one em long, squeezed to the em when wider; longer numbers (`2026`)
/// stay on their side.
fn tate_chu_yoko(g: &mut [SGlyph], size: impl Fn(&SGlyph) -> f64) {
    let digits: Vec<bool> = g.iter().map(|g| g.ch.is_ascii_digit()).collect();
    let digit = |i: usize| digits.get(i).copied().unwrap_or(false);
    let mut i = 0;
    while i < g.len() {
        if !digit(i) {
            i += 1;
            continue;
        }
        let mut end = i;
        while digit(end) {
            end += 1;
        }
        if (2..=3).contains(&(end - i)) {
            let em = size(&g[i]);
            let width: f64 = g[i..end].iter().map(|g| g.adv).sum();
            let squeeze = if width > em && width > 0.0 { em / width } else { 1.0 };
            let mut ink = 0.0;
            for (k, gl) in g[i..end].iter_mut().enumerate() {
                let adv = gl.adv;
                gl.tcy = Some(Tcy { pen: if k == 0 { 0.0 } else { em }, ink, width, squeeze });
                gl.adv = if k == 0 { em } else { 0.0 };
                ink += adv;
            }
        }
        i = end;
    }
}

/// Half an em of `g`'s size when it is full-width Japanese punctuation of kind `kind` (its
/// advance an em, give or take a tenth), the space that mojikumi can take off.
fn punct_half(g: &SGlyph, kind: Punct) -> Option<f64> {
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
fn compress_punctuation(sg: &mut [SGlyph]) -> Vec<bool> {
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
fn space_japanese_and_latin(sg: &mut [SGlyph]) -> Vec<f64> {
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
fn glyph_em(g: &SGlyph) -> f64 {
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
fn align_shift(g: &SGlyph, a: vectorcraft_doc::CharAlign, line_em: f64, vertical: bool) -> f64 {
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
fn upright_cell(g: &SGlyph) -> f64 {
    match g.face.vertical_glyph(g.gid) {
        Some((advance, _)) => advance * g.sy,
        None => (g.face.advance(g.gid) * g.sx).max(g.face.units_per_em() * g.sx),
    }
}

/// Height above the baseline (line space) of the centre of an upright glyph's `cell`, the point it
/// turns about: from the font's vertical metrics, the cell hanging from the glyph's vertical origin;
/// else the centre of the ideographic em box ([`EM_CENTER`] of the size `em`).
fn upright_centre(g: &SGlyph, cell: f64, em: f64) -> f64 {
    match g.face.vertical_glyph(g.gid) {
        Some((_, origin)) => origin * g.sy - cell * 0.5,
        None => EM_CENTER * em,
    }
}

/// Height of the centre of the ideographic em box above the baseline, in ems (the em box runs
/// from 0.12 em below the baseline to 0.88 em above it), for fonts without vertical metrics.
const EM_CENTER: f64 = 0.38;

/// A quarter turn clockwise (y down), exact: line space → text space for vertical type.
const QUARTER_TURN: Affine = Affine::new([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]);

/// Does the glyph stand upright in vertical type? CJK characters and symbols do (their vertical
/// forms come from the font's `vert` feature); Latin letters, digits and other characters lie on
/// their side. Brackets, the long vowel mark and similar marks that need a vertical form lie on
/// their side when the font has none (Unicode's Vertical_Orientation Tr).
fn stands_upright(g: &SGlyph) -> bool {
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
        | 0xFE50..=0xFE52 | 0xFE59..=0xFE5E | 0xFF01 | 0xFF08 | 0xFF09 | 0xFF0C | 0xFF0E | 0xFF1A | 0xFF1B | 0xFF1F
        | 0xFF3B | 0xFF3D | 0xFF3F | 0xFF5B..=0xFF60 | 0xFFE3);
    // Without a vertical alternate the shaped glyph is the nominal one.
    !needs_vertical_form || g.gid != g.face.glyph_for(g.ch)
}

fn finish_bounds(out: &mut TextLayout) {
    let xf = out.line_xf;
    let mut b: Option<Rect> = None;
    let mut add = |r: Rect| b = Some(b.map_or(r, |b| b.union(r)));
    for g in &out.glyphs {
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
fn align_vertically(cx: &mut Ctx<'_>, paras: &[Range<usize>], t: &TextObject, regions: &mut [Region]) {
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
            std::mem::replace(&mut cx.out.overflow, false),
        );
        flow(cx, paras, t, Some(regions));
        if laid_out(&cx.out) < before {
            // Text no longer fits (lines got narrower, or flow around a wrap object): undo the
            // pass and try a smaller step.
            (cx.out.glyphs, cx.out.lines, cx.out.inlines, cx.out.overflow) = prev;
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

/// One cell of a flattened area-type frame (the whole frame, or one row/column of it).
struct Region {
    /// The cell (frame bounds, or a grid cell of them).
    cell: Rect,
    /// Inset applied inside the frame edges.
    inset: f64,
    polys: Vec<Vec<Point>>,
    /// The frame is its own bounding rectangle (spans need no polygon intersection).
    rect: bool,
    /// Text Wrap shapes: (polygons, offset, invert).
    wraps: Vec<(Vec<Vec<Point>>, f64, bool)>,
    /// Vertical alignment: how far below the top-aligned position the first line starts.
    shift: f64,
    /// Vertical justification: extra space added between consecutive lines.
    gap: f64,
}

/// Flattened closed polygons of a path.
fn polygons(path: &BezPath) -> Vec<Vec<Point>> {
    let mut polys: Vec<Vec<Point>> = Vec::new();
    kurbo::flatten(path, 0.1, |el| match el {
        PathEl::MoveTo(p) => polys.push(vec![p]),
        PathEl::LineTo(p) => {
            if let Some(v) = polys.last_mut() {
                v.push(p);
            }
        }
        _ => {}
    });
    polys.retain(|p| p.len() >= 3);
    polys
}

/// Inside intervals (even-odd) of the horizontal line at `y` through `polys`.
fn poly_intervals(polys: &[Vec<Point>], y: f64) -> Vec<(f64, f64)> {
    let mut xs = Vec::new();
    for poly in polys {
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            if (a.y <= y) != (b.y <= y) {
                xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
            }
        }
    }
    xs.sort_by(f64::total_cmp);
    xs.as_chunks::<2>().0.iter().map(|c| (c[0], c[1])).collect()
}

/// Union of sorted-or-not intervals.
fn union_intervals(mut v: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(v.len());
    for (a, b) in v {
        match out.last_mut() {
            Some(l) if a <= l.1 => l.1 = l.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// `a` minus `b` (both unions of disjoint intervals).
fn subtract_intervals(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = vec![];
    for &(mut s, e) in a {
        for &(c, d) in b {
            if d <= s || c >= e {
                continue;
            }
            if c > s {
                out.push((s, c));
            }
            s = s.max(d);
        }
        if e > s {
            out.push((s, e));
        }
    }
    out
}

/// `a` ∩ `b`.
fn intersect_intervals(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = vec![];
    for &(s, e) in a {
        for &(c, d) in b {
            let (x, y) = (s.max(c), e.min(d));
            if y > x {
                out.push((x, y));
            }
        }
    }
    out
}

impl Region {
    fn cells(path: &BezPath, opts: &LayoutOptions, wrap: &[vectorcraft_doc::WrapShape]) -> Vec<Region> {
        let polys = polygons(path);
        let wraps: Vec<(Vec<Vec<Point>>, f64, bool)> =
            wrap.iter().map(|w| (polygons(&w.path.to_bezpath()), w.wrap.offset.max(0.0), w.wrap.invert)).filter(|w| !w.0.is_empty()).collect();
        let bbox = path.bounding_box();
        let rect = polys.len() == 1 && {
            let area: f64 = polys[0].iter().zip(polys[0].iter().cycle().skip(1)).map(|(a, b)| a.x * b.y - b.x * a.y).sum::<f64>().abs() * 0.5;
            (area - bbox.area()).abs() <= bbox.area() * 1e-6 + 1e-9
        };
        let (rows, cols) = (opts.rows.max(1), opts.columns.max(1));
        let gutter = opts.gutter.max(0.0);
        let cw = ((bbox.width() - gutter * (cols - 1) as f64) / cols as f64).max(0.0);
        let rh = ((bbox.height() - gutter * (rows - 1) as f64) / rows as f64).max(0.0);
        let mut out = Vec::with_capacity(rows * cols);
        // Text flows down each column, then across (Illustrator's default "by columns").
        for c in 0..cols {
            for r in 0..rows {
                let x0 = bbox.x0 + c as f64 * (cw + gutter);
                let y0 = bbox.y0 + r as f64 * (rh + gutter);
                let cell = if rows * cols == 1 { bbox } else { Rect::new(x0, y0, x0 + cw, y0 + rh) };
                out.push(Region { cell, inset: opts.inset.max(0.0), polys: polys.clone(), rect, wraps: wraps.clone(), shift: 0.0, gap: 0.0 });
            }
        }
        out
    }

    fn top(&self) -> f64 {
        self.cell.y0 + self.inset
    }
    fn bottom(&self) -> f64 {
        self.cell.y1 - self.inset
    }

    /// Inside intervals (even-odd) of the horizontal line at `y`, minus the wrap objects (or
    /// inside them, for Invert Wrap). Offsets grow each wrap shape vertically and horizontally.
    fn intervals(&self, y: f64) -> Vec<(f64, f64)> {
        let mut iv = if self.polys.is_empty() || self.rect { vec![(self.cell.x0, self.cell.x1)] } else { poly_intervals(&self.polys, y) };
        for (polys, off, invert) in &self.wraps {
            let ys = if *off > 0.0 { vec![y - off, y - off * 0.5, y, y + off * 0.5, y + off] } else { vec![y] };
            let w = union_intervals(ys.into_iter().flat_map(|y| poly_intervals(polys, y)).map(|(a, b)| (a - off, b + off)).collect());
            iv = if *invert { intersect_intervals(&iv, &w) } else { subtract_intervals(&iv, &w) };
        }
        iv
    }

    /// Widest horizontal span inside the frame (and the cell) over the band `top..bottom`.
    fn span(&self, top: f64, bottom: f64) -> Option<(f64, f64)> {
        self.spans(top, bottom).into_iter().max_by(|x, y| (x.1 - x.0).total_cmp(&(y.1 - y.0)))
    }

    /// Every horizontal span inside the frame (and the cell) over the band `top..bottom`, left to
    /// right (text wraps on both sides of an object).
    fn spans(&self, top: f64, bottom: f64) -> Vec<(f64, f64)> {
        let clip = |(a, b): (f64, f64)| {
            let (a, b) = (a.max(self.cell.x0) + self.inset, b.min(self.cell.x1) - self.inset);
            (b > a).then_some((a, b))
        };
        let plain = self.polys.is_empty() || self.rect;
        if plain && self.wraps.is_empty() {
            return (self.cell.width() > 0.0).then_some((self.cell.x0, self.cell.x1)).and_then(clip).into_iter().collect();
        }
        let fb = if plain {
            self.cell
        } else {
            self.polys.iter().flatten().fold(Rect::new(f64::MAX, f64::MAX, f64::MIN, f64::MIN), |r, p| r.union_pt(*p))
        };
        let clamp = |y: f64| if plain { y } else { y.clamp(fb.y0 + 1e-4, fb.y1 - 1e-4) };
        // Sample the band densely enough for curved frames (circles, blobs).
        let samples = 5;
        let mut rows: Vec<Vec<(f64, f64)>> =
            (0..samples).map(|k| self.intervals(clamp(top + (bottom - top) * k as f64 / (samples - 1) as f64))).collect();
        let mid = rows.swap_remove(samples / 2);
        if !self.wraps.is_empty() {
            // Wrap objects split lines: keep exactly what is free on every sampled row.
            let free = rows.iter().fold(mid, |acc, r| intersect_intervals(&acc, r));
            return free.into_iter().filter_map(clip).collect();
        }
        mid.into_iter()
            .filter_map(|(mut a, mut b)| {
                for o in &rows {
                    let best =
                        o.iter().map(|&(c, d)| (a.max(c), b.min(d))).filter(|(c, d)| d > c).max_by(|x, y| (x.1 - x.0).total_cmp(&(y.1 - y.0)))?;
                    a = best.0;
                    b = best.1;
                }
                clip((a, b))
            })
            .collect()
    }
}

/// Vertical metrics of a line: (ascent, descent, leading, cap height, x height).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Metrics {
    asc: f64,
    desc: f64,
    lead: f64,
    cap: f64,
    xh: f64,
    /// Height of the top of the ideographic em box above the baseline.
    top: f64,
}

impl Metrics {
    fn of(g: &SGlyph) -> Self {
        let em = g.face.units_per_em() * g.sy;
        Self { asc: g.ascent, desc: g.descent, lead: g.leading, cap: g.cap, xh: g.xh, top: (g.face.ideographic_centre() + 0.5) * em }
    }
    fn max(g: &[SGlyph]) -> Option<Self> {
        let mut it = g.iter();
        let first = Self::of(it.next()?);
        Some(it.fold(first, |m, g| Self {
            asc: m.asc.max(g.ascent),
            desc: m.desc.max(g.descent),
            lead: m.lead.max(g.leading),
            cap: m.cap.max(g.cap),
            xh: m.xh.max(g.xh),
            top: m.top.max(Self::of(g).top),
        }))
    }
    /// Distance from the frame top to the first baseline.
    fn first_baseline(&self, fb: FirstBaseline, min: f64) -> f64 {
        let v = match fb {
            FirstBaseline::Ascent => self.asc,
            FirstBaseline::CapHeight => self.cap,
            FirstBaseline::XHeight => self.xh,
            FirstBaseline::Leading => self.lead,
            FirstBaseline::Fixed => 0.0,
        };
        v.max(min)
    }
}

/// Where the next line goes: the region being filled and the previous baseline in it.
#[derive(Clone)]
struct Pen<'r> {
    regions: Option<&'r [Region]>,
    ri: usize,
    prev: Option<f64>,
    pending: f64,
    fb: FirstBaseline,
    fb_min: f64,
    /// Further spans at the current baseline (text wrapping on both sides of an object).
    queued: Vec<(f64, f64, f64)>,
    /// Leading measured from em box top to em box top: where the next line's em box top goes.
    model: LeadingModel,
    next_top: Option<f64>,
}

impl Pen<'_> {
    /// Baseline and horizontal span for a line with estimated metrics `est` and indents; `None` =
    /// the frame is full (overflow).
    /// The `bool` is true for a further span at the previous line's baseline.
    fn place(&mut self, est: Metrics, ind_l: f64, ind_r: f64) -> Option<(f64, f64, f64, bool)> {
        let Some(regions) = self.regions else {
            let b = self.prev.map_or(0.0, |b| self.next_baseline(b, est));
            return Some((b, f64::NEG_INFINITY, f64::INFINITY, false));
        };
        if !self.queued.is_empty() {
            let (b, x0, x1) = self.queued.remove(0);
            return Some((b, x0, x1, true));
        }
        loop {
            let r = regions.get(self.ri)?;
            let mut baseline = match self.prev {
                None if self.model == LeadingModel::EmBoxTop => r.top() + r.shift + est.top.max(self.fb_min),
                None => r.top() + r.shift + est.first_baseline(self.fb, self.fb_min),
                Some(b) => self.next_baseline(b, est) + r.gap,
            };
            loop {
                if baseline + est.desc > r.bottom() + 0.01 {
                    break;
                }
                let fits = |&(a, b): &(f64, f64)| b - a - ind_l - ind_r > est.asc.max(1.0);
                if r.wraps.is_empty() {
                    match r.span(baseline - est.asc, baseline + est.desc) {
                        Some(s) if fits(&s) => return Some((baseline, s.0, s.1, false)),
                        _ => baseline += est.lead.max(1.0),
                    }
                } else {
                    // Wrap objects can split a line: fill every span, left to right.
                    let mut spans: Vec<(f64, f64)> = r.spans(baseline - est.asc, baseline + est.desc).into_iter().filter(fits).collect();
                    if spans.is_empty() {
                        baseline += est.lead.max(1.0);
                        continue;
                    }
                    let (a, b) = spans.remove(0);
                    self.queued = spans.into_iter().map(|(a, b)| (baseline, a, b)).collect();
                    return Some((baseline, a, b, false));
                }
            }
            // Next row/column.
            self.ri += 1;
            self.prev = None;
            self.pending = 0.0;
            self.queued.clear();
        }
    }
    fn bottom(&self) -> f64 {
        self.regions.and_then(|r| r.get(self.ri)).map_or(f64::INFINITY, |r| r.bottom())
    }
    /// The baseline of the line after the one at `prev`, for a line of metrics `est`.
    fn next_baseline(&self, prev: f64, est: Metrics) -> f64 {
        match (self.model, self.next_top) {
            (LeadingModel::EmBoxTop, Some(top)) => top + est.top + self.pending,
            _ => prev + est.lead + self.pending,
        }
    }
    /// A line was set at `baseline` with metrics `m`: where the next one goes from.
    fn settled(&mut self, baseline: f64, m: Metrics) {
        self.prev = Some(baseline);
        self.next_top = Some(baseline - m.top + m.lead);
    }
}

/// Advance of a tab at pen position `x`: up to the next stop after it (explicit stops first, then
/// every ½ inch). Right, centre and decimal stops align the text that follows (`rest`, up to the
/// next tab) on the stop.
fn tab_advance(tabs: &[vectorcraft_doc::TabStop], origin: f64, x: f64, rest: &[SGlyph]) -> f64 {
    use vectorcraft_doc::TabAlign;
    let rel = x - origin;
    let seg: Vec<&SGlyph> = rest.iter().take_while(|g| g.ch != '\t').collect();
    let width: f64 = seg.iter().map(|g| g.adv).sum();
    let target = |pos: f64, align: TabAlign, on: char| match align {
        TabAlign::Left => pos,
        TabAlign::Right => pos - width,
        TabAlign::Center => pos - width / 2.0,
        TabAlign::Decimal => pos - seg.iter().take_while(|g| g.ch != on).map(|g| g.adv).sum::<f64>(),
    };
    let explicit = tabs.iter().find_map(|t| {
        let at = target(t.position, t.align, t.align_on);
        (at > rel + 1e-6).then_some(at)
    });
    let at = explicit.unwrap_or_else(|| {
        let last = tabs.iter().map(|t| t.position).fold(0.0, f64::max).max(0.0);
        let base = rel.max(last);
        ((base / vectorcraft_doc::text::DEFAULT_TAB_INTERVAL).floor() + 1.0) * vectorcraft_doc::text::DEFAULT_TAB_INTERVAL
    });
    (at - rel).max(0.0)
}

/// Greedy break: returns (end glyph index, hyphenated) for a line starting at `i` of `width`.
/// With burasagari, a comma or full stop that doesn't fit ends the line, hanging outside it.
fn break_line(text: &str, g: &[SGlyph], i: usize, width: f64, hyphenate: bool, burasagari: Burasagari, kinsoku: Kinsoku) -> (usize, bool) {
    if !width.is_finite() {
        return (g.len(), false);
    }
    let mut x = 0.0;
    let mut last_break: Option<(usize, bool)> = None;
    let mut j = i;
    while j < g.len() {
        let gl = &g[j];
        let mut cluster_end = j + 1;
        while g.get(cluster_end).is_some_and(|next| next.byte == gl.byte) {
            cluster_end += 1;
        }
        let cluster_width: f64 = g[j..cluster_end].iter().map(|glyph| glyph.adv).sum();
        if j > i && !gl.is_space() && x + cluster_width > width + EPS {
            // It ends the line with the spaces after it.
            if burasagari != Burasagari::None && hangs(gl) && kinsoku_allows(g, cluster_end - 1, kinsoku) {
                let mut end = cluster_end;
                while g.get(end).is_some_and(SGlyph::is_space) {
                    end += 1;
                }
                return (end, false);
            }
            break;
        }
        x += cluster_width;
        let last = &g[cluster_end - 1];
        if last.break_after() && kinsoku_allows(g, cluster_end - 1, kinsoku) {
            if last.is_soft_hyphen() {
                if x + hyphen_glyph(last).adv <= width + EPS {
                    last_break = Some((cluster_end, true));
                }
            } else {
                last_break = Some((cluster_end, false));
            }
        }
        j = cluster_end;
    }
    if j >= g.len() {
        return (g.len(), false);
    }
    // Hyphenate the word that overflows.
    let word_start = last_break.map_or(i, |b| b.0);
    if hyphenate && g[j].is_letter() {
        let mut we = j;
        while we < g.len() && !g[we].is_space() && !g[we].break_after() {
            we += 1;
        }
        let x_ws: f64 = g[i..word_start].iter().map(|g| g.adv).sum();
        let pts: Vec<usize> = hyphen_breaks(text, g, word_start, we).into_iter().filter(|&k| k > 0 && kinsoku_allows(g, k - 1, kinsoku)).collect();
        for &k in pts.iter().rev() {
            if k <= j && k > i {
                let w = x_ws + g[word_start..k].iter().map(|g| g.adv).sum::<f64>() + hyphen_glyph(&g[k - 1]).adv;
                if w <= width + EPS {
                    return (k, true);
                }
            }
        }
    }
    // Break candidates and greedy overflow are cluster boundaries; hyphenation also returns only
    // cluster starts, so the resulting end always keeps each source cluster together.
    let (end, hy) = match last_break {
        Some(b) if b.0 > i => b,
        _ => (j, false),
    };
    (end, hy)
}

/// Can glyph `g` hang outside the line (burasagari)? An East Asian comma or full stop, full width
/// (、。，．) or half width (､｡); not a closing bracket, nor Latin punctuation (Latin text keeps
/// its line breaks and composer). Never an inline graphic (U+FFFC).
fn hangs(g: &SGlyph) -> bool {
    g.inline.is_none() && matches!(g.ch, '、' | '。' | '，' | '．' | '､' | '｡') && g.tcy.is_none()
}

/// Does kinsoku allow a line break after glyph `j`? Not after an opening bracket, nor before a
/// closing one, a comma, a full stop, a small kana… (the character is pushed to the next line
/// with the one before it).
fn kinsoku_allows(g: &[SGlyph], j: usize, kinsoku: Kinsoku) -> bool {
    g.get(j).is_some_and(|gl| kinsoku_between(gl.ch, g.get(j + 1).map(|n| n.ch), kinsoku))
}

/// Kinsoku for a break between `before` and `after` (none: the end of the paragraph): Hard keeps
/// every character of the set off the line start and end, Soft lets 々, ー and small kana start a
/// line, None allows the break.
fn kinsoku_between(before: char, after: Option<char>, kinsoku: Kinsoku) -> bool {
    match kinsoku {
        Kinsoku::None => true,
        Kinsoku::Hard => !no_line_end(before) && after.is_none_or(|c| !no_line_start(c)),
        Kinsoku::Soft => !no_line_end(before) && after.is_none_or(|c| !no_line_start(c) || soft_line_start(c)),
    }
}

#[cfg(test)]
mod wrapping_tests {
    use super::*;
    use std::sync::Arc;

    fn synthetic_glyphs(specs: &[(usize, usize, f64, char)]) -> Vec<SGlyph> {
        // The face is only a required SGlyph field here; advances and cluster boundaries below are
        // entirely synthetic, so the regressions don't depend on a particular font's shaping.
        static DB: std::sync::OnceLock<FontDb> = std::sync::OnceLock::new();
        let db = DB.get_or_init(|| FontDb::with_font_dirs(vec![]));
        let face = db.face("Source Sans 3", "Regular").expect("bundled test face");
        specs
            .iter()
            .map(|&(byte, len, adv, ch)| SGlyph {
                face: Arc::clone(&face),
                gid: 0,
                byte,
                len,
                run: 0,
                adv,
                dx: 0.0,
                dy: 0.0,
                sx: 1.0,
                sy: 1.0,
                bshift: 0.0,
                rotation: 0.0,
                ascent: 8.0,
                descent: 2.0,
                leading: 0.0,
                cap: 6.0,
                xh: 4.0,
                ch,
                tcy: None,
                inline: None,
                lead: 0.0,
                proportional: false,
                vpal: None,
                level: unicode_bidi::Level::ltr(),
            })
            .collect()
    }

    fn source_range(glyphs: &[SGlyph], start: usize, end: usize) -> Range<usize> {
        let first = glyphs.get(start).expect("nonempty source range");
        let last = glyphs.get(end - 1).expect("nonempty source range");
        first.byte..last.byte + last.len
    }

    #[test]
    fn greedy_wrap_keeps_an_oversized_first_cluster_and_its_source_range() {
        let text = "بَت";
        let glyphs = synthetic_glyphs(&[(0, 4, 2.0, 'ب'), (0, 4, 2.0, 'ب'), (4, 2, 1.0, 'ت')]);

        let (end, hyphenated) = break_line(text, &glyphs, 0, 3.0, false, Burasagari::None, Kinsoku::Hard);

        assert!(!hyphenated);
        assert_eq!(end, 2, "oversized base-plus-mark cluster must stay together");
        assert_eq!(source_range(&glyphs, 0, end), 0..4);
        assert_eq!(glyphs.get(end).map(|g| g.byte), Some(4));
    }

    #[test]
    fn greedy_wrap_keeps_an_oversized_cluster_after_a_line_start_atomic() {
        let text = "بَتَث";
        let glyphs = synthetic_glyphs(&[(0, 4, 1.0, 'ب'), (4, 4, 2.0, 'ت'), (4, 4, 2.0, 'ت'), (8, 2, 1.0, 'ث')]);

        let (first_end, _) = break_line(text, &glyphs, 0, 3.0, false, Burasagari::None, Kinsoku::Hard);
        assert_eq!(first_end, 1);
        assert_eq!(source_range(&glyphs, 0, first_end), 0..4);

        let (second_end, hyphenated) = break_line(text, &glyphs, first_end, 3.0, false, Burasagari::None, Kinsoku::Hard);

        assert!(!hyphenated);
        assert_eq!(second_end, 3, "oversized cluster at the next line start must stay together");
        assert_eq!(source_range(&glyphs, first_end, second_end), 4..8);
        assert_eq!(glyphs.get(second_end).map(|g| g.byte), Some(8));
    }

    #[test]
    fn narrow_arabic_area_wrap_keeps_base_and_mark_together_when_supported() {
        let db = FontDb::global();
        let Some(face) = db.face_covering('ب').filter(|face| face.covers('َ')) else {
            // Arabic integration coverage depends on an installed font; synthetic tests above are
            // the font-independent regression oracle.
            return;
        };
        let text = "بَت";
        let style = CharStyle { font_family: face.family.clone(), font_style: face.style.clone(), size: 20.0, ..CharStyle::default() };
        let bidi = para_bidi(text, None);
        let levels = bidi.as_ref().map_or(&[][..], |info| &info.levels);
        let mut shaped = Vec::new();
        shape_range(db, text, 0..text.len(), &[(0..text.len(), &style)], &[], &OtFeatures::default(), levels, &mut shaped);
        let Some(first) = shaped.first() else { return };
        let first_end = shaped.iter().take_while(|g| g.byte == first.byte).count();
        if first.byte != 0 || first_end < 2 || first_end >= shaped.len() {
            return;
        }
        // Pick a narrow width that fits glyphs before one positive-advance glyph but not that next
        // glyph. This ensures the old per-glyph loop would have returned inside the first cluster.
        let mut prefix = shaped.first().map_or(0.0, |g| g.adv);
        let width = (1..first_end).find_map(|k| {
            let advance = shaped.get(k)?.adv;
            if prefix > EPS && advance > EPS {
                Some(prefix)
            } else {
                prefix += advance;
                None
            }
        });
        let Some(width) = width else { return };

        let mut object = TextObject::point(Point::ZERO, text, style);
        object.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, width, 100.0).to_path(0.1)) };
        let result = layout_with(db, &object, &LayoutOptions::default());

        let next_cluster_byte = shaped.get(first_end).map(|g| g.byte);
        assert_eq!(result.lines.first().map(|line| line.end), next_cluster_byte);
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

#[cfg(test)]
#[test]
fn kinsoku_keeps_closing_marks_off_line_starts_and_opening_ones_off_line_ends() {
    for (a, b) in [('字', '。'), ('弧', '」'), ('」', '、'), ('ャ', 'ー'), ('カ', 'ッ'), ('「', 'か'), ('（', '雅')] {
        assert!(!kinsoku_between(a, Some(b), Kinsoku::Hard), "{a}{b}");
    }
    for (a, b) in [('。', '雅'), ('」', 'は'), ('字', '「'), ('の', '演')] {
        assert!(kinsoku_between(a, Some(b), Kinsoku::Hard), "{a}{b}");
    }
    assert!(kinsoku_between('。', None, Kinsoku::Hard));
}

/// Soft kinsoku lets 々, ー and small kana (JLREQ cl-09's 々, cl-10, cl-11) start a line and keeps
/// the rest of the set (half-width forms aren't in those classes); None allows any break.
#[cfg(test)]
#[test]
fn soft_kinsoku_lets_small_kana_and_the_prolonged_sound_mark_start_a_line() {
    for (a, b) in [('ャ', 'ー'), ('カ', 'ッ'), ('時', '々'), ('ト', 'ㇰ'), ('か', 'ゃ')] {
        assert!(!kinsoku_between(a, Some(b), Kinsoku::Hard), "{a}{b}");
        assert!(kinsoku_between(a, Some(b), Kinsoku::Soft), "{a}{b}");
    }
    for (a, b) in [('字', '。'), ('弧', '」'), ('「', 'か'), ('ｶ', 'ｯ'), ('ｶ', 'ｰ'), ('字', 'ゝ')] {
        assert!(!kinsoku_between(a, Some(b), Kinsoku::Soft), "{a}{b}");
        assert!(kinsoku_between(a, Some(b), Kinsoku::None), "{a}{b}");
    }
}

/// Glyph indices inside `g[ws..we]` (a word) where a hyphenated break may go.
fn hyphen_breaks(text: &str, g: &[SGlyph], ws: usize, we: usize) -> Vec<usize> {
    // Strip leading/trailing punctuation (quotes, commas).
    let (mut a, mut b) = (ws, we);
    while a < b && !g[a].is_letter() {
        a += 1;
    }
    while b > a && !g[b - 1].is_letter() {
        b -= 1;
    }
    if b <= a || g[a..b].iter().any(|g| !g.is_letter()) {
        return vec![];
    }
    let (s, e) = (g[a].byte, g[b - 1].byte + g[b - 1].len);
    let Some(word) = text.get(s..e) else { return vec![] };
    let offs: Vec<usize> = word.char_indices().map(|(o, _)| s + o).collect();
    hyphen_points(word)
        .into_iter()
        .filter_map(|ci| {
            let byte = *offs.get(ci)?;
            // Only at cluster starts (not inside a ligature).
            (a + 1..b).find(|&k| g[k].byte == byte && g[k - 1].byte != byte)
        })
        .collect()
}

/// Break candidates for the every-line composer.
fn candidates(text: &str, g: &[SGlyph], hyphenate: bool, kinsoku: Kinsoku) -> Vec<Breakpoint> {
    let mut v = vec![];
    let mut ws = 0;
    for (j, gl) in g.iter().enumerate() {
        let end_of_word = gl.is_space() || gl.break_after();
        if end_of_word {
            if hyphenate && j > ws {
                for k in hyphen_breaks(text, g, ws, j).into_iter().filter(|&k| k > 0 && kinsoku_allows(g, k - 1, kinsoku)) {
                    v.push(Breakpoint { end: k, hyphen: hyphen_glyph(&g[k - 1]).adv });
                }
            }
            let hy = if gl.is_soft_hyphen() { hyphen_glyph(gl).adv } else { 0.0 };
            if j + 1 < g.len() && g[j + 1].byte != gl.byte && kinsoku_allows(g, j, kinsoku) {
                v.push(Breakpoint { end: j + 1, hyphen: hy });
            }
            ws = j + 1;
        }
    }
    if hyphenate && g.len() > ws {
        for k in hyphen_breaks(text, g, ws, g.len()).into_iter().filter(|&k| k > 0 && kinsoku_allows(g, k - 1, kinsoku)) {
            v.push(Breakpoint { end: k, hyphen: hyphen_glyph(&g[k - 1]).adv });
        }
    }
    v.sort_by_key(|b| b.end);
    v.dedup_by_key(|b| b.end);
    v
}

/// Paragraphs longer than this (glyphs) are always broken line by line.
const MAX_COMPOSE_GLYPHS: usize = 200_000;

/// Every-line composition of paragraph glyphs `sg` if applicable (area text with uniform line
/// metrics, justified or ragged); `None` falls back to the greedy single-line composer.
fn compose_para(cx: &Ctx<'_>, sg: &[SGlyph], para: &ParaStyle, pen: &Pen<'_>, rtl: bool) -> Option<Vec<(usize, bool)>> {
    let ragged = matches!(para.justify, Justify::Auto | Justify::Left | Justify::Center | Justify::Right);
    let composer = cx.opts.composer.unwrap_or(para.composer);
    // A paragraph whose commas or full stops may hang is composed line by line (as the Japanese
    // single-line composer does).
    let may_hang = para.burasagari != Burasagari::None && sg.iter().any(hangs);
    if composer != Composer::EveryLine || pen.regions.is_none() || sg.len() < 2 || sg.len() > MAX_COMPOSE_GLYPHS || may_hang {
        return None;
    }
    let m = Metrics::of(&sg[0]);
    if sg.iter().any(|g| Metrics::of(g) != m) {
        return None;
    }
    // Line widths are independent of the breaks when every line has the same metrics.
    let total: f64 = sg.iter().map(|g| g.adv).sum();
    let mut sim = pen.clone();
    let mut widths = vec![];
    let mut acc = 0.0;
    while widths.len() < sg.len() {
        let first = widths.is_empty();
        let ind_l = para.left_indent + if first { para.first_line_indent } else { 0.0 };
        let Some((b, x0, x1, _)) = sim.place(m, ind_l, para.right_indent) else { break };
        let w = x1 - x0 - ind_l - para.right_indent;
        widths.push(w);
        acc += w.max(1.0);
        sim.settled(b, m);
        sim.pending = 0.0;
        if acc > total * 1.6 + 4.0 * w.max(1.0) {
            break;
        }
    }
    let last = *widths.last()?;
    let width = |k: usize| widths.get(k).copied().unwrap_or(last);
    // Lines from `uniform_from` on are interchangeable (same width).
    let uniform_from = widths.iter().rposition(|&w| (w - last).abs() > EPS).map_or(0, |k| k + 1);
    let cands = candidates(cx.text, sg, para.hyphenate, para.kinsoku);
    let justify_last = para.justify == Justify::JustifyAll;
    let params = |tolerance| Params { justify_last, ragged, tolerance, uniform_from };
    // Ragged: first look for breaks that leave at most one rag zone (a sixth of the width) on
    // each line, then accept any lines that fit.
    let (strict, loose) = if ragged { (1.0, f64::INFINITY) } else { (1.0, 4.0) };
    // The same rule as the line loop below: an opening bracket starting a wrapped line gives up
    // the space before it.
    let start_credit = |i: usize| {
        let flush = !rtl && para.mojikumi == Mojikumi::LineEndHalf;
        sg.get(i).filter(|g| flush && g.lead <= 0.0).and_then(|g| punct_half(g, Punct::Opening)).unwrap_or(0.0)
    };
    compose(sg, &width, &start_credit, &cands, &params(strict)).or_else(|| compose(sg, &width, &start_credit, &cands, &params(loose)))
}

/// Flow paragraphs `paras` (byte ranges) of `t`, each with its own paragraph attributes
/// (alignment, indents, spacing, direction, mojikumi, leading model…).
fn flow(cx: &mut Ctx<'_>, paras: &[Range<usize>], t: &TextObject, regions: Option<&[Region]>) {
    let mut pen = Pen {
        regions,
        ri: 0,
        prev: None,
        pending: 0.0,
        fb: cx.opts.first_baseline,
        fb_min: cx.opts.first_baseline_min,
        queued: vec![],
        model: t.para_at(0).leading_model,
        next_top: None,
    };
    'paras: for (pi, pr) in paras.iter().enumerate() {
        let para = t.para_at(pi);
        // The leading model is a paragraph attribute: this paragraph's lines (and the space to
        // its first line) follow its own.
        pen.model = para.leading_model;
        let text = cx.text;
        let bidi = para_bidi(text.get(pr.clone()).unwrap_or_default(), para.direction);
        let rtl = is_rtl(bidi.as_ref());
        let mut sg = cx.shape_para(pr.clone(), bidi.as_ref());
        // Japanese composition: consecutive punctuation shares one half-em space.
        let compressed = if para.mojikumi == Mojikumi::LineEndHalf { compress_punctuation(&mut sg) } else { vec![] };
        // …and a quarter em between Japanese and Latin text.
        let wakan = if para.mojikumi == Mojikumi::LineEndHalf { space_japanese_and_latin(&mut sg) } else { vec![] };
        let pm = {
            let (asc, desc, lead) = style_metrics(cx.db, cx.style_at(pr.start));
            let st = cx.style_at(pr.start);
            let (cap, xh) = cap_x_heights(cx.db, st);
            let centre =
                cx.db.face_version(&st.font_family, &st.font_style, st.font_version.as_deref()).map_or(EM_CENTER, |f| f.ideographic_centre());
            Metrics { asc, desc, lead, cap, xh, top: (centre + 0.5) * st.size * st.v_scale / 100.0 }
        };
        if pi > 0 {
            pen.pending += para.space_before;
        }
        let n = sg.len();
        let composed = compose_para(cx, &sg, para, &pen, rtl);
        let mut li_para = 0;
        let mut i = 0;
        loop {
            let est = if i < n { Metrics::of(&sg[i]) } else { pm };
            let first_line = li_para == 0;
            let ind_l = para.left_indent + if first_line { para.first_line_indent } else { 0.0 };
            // Mojikumi: an opening bracket starting a line is set flush with the line's start (the
            // space before it goes), and the line has that much more room. At a paragraph's start
            // that is the first-line indent: JIS X 4051's principle (JLREQ 3.1.5, Figure 71 ①).
            if !rtl
                && para.mojikumi == Mojikumi::LineEndHalf
                && let Some(g) = sg.get_mut(i)
                && g.lead <= 0.0
                && let Some(h) = punct_half(g, Punct::Opening)
            {
                g.adv -= h;
                g.lead += h;
            }
            // Place, break, then settle the baseline on the line's real metrics (moving on to the
            // next row/column if it no longer fits).
            let (baseline, x0, x1, end, hyph, m) = loop {
                let first_in_region = pen.prev.is_none();
                let Some((mut baseline, x0, x1, same_baseline)) = pen.place(est, ind_l, para.right_indent) else {
                    cx.out.overflow = cx.text.len() > if i < n { sg[i].byte } else { pr.start };
                    break 'paras;
                };
                let width = x1 - x0 - ind_l - para.right_indent;
                let (end, hyph) = match composed.as_ref().and_then(|c| c.get(li_para)) {
                    Some(&(e, h)) if e > i => (e, h),
                    _ if i < n => break_line(cx.text, &sg, i, width, para.hyphenate, para.burasagari, para.kinsoku),
                    _ => (n, false),
                };
                let m = Metrics::max(&sg[i..end]).unwrap_or(pm);
                baseline += if same_baseline || (pen.regions.is_none() && first_in_region) {
                    0.0
                } else if pen.model == LeadingModel::EmBoxTop {
                    // The em box top is fixed (the frame's top, or the line above's): the baseline
                    // hangs from it by this line's tallest em box.
                    if first_in_region { m.top.max(pen.fb_min) - est.top.max(pen.fb_min) } else { m.top - est.top }
                } else if first_in_region {
                    m.first_baseline(pen.fb, pen.fb_min) - est.first_baseline(pen.fb, pen.fb_min)
                } else {
                    m.lead - est.lead
                };
                if pen.regions.is_some() && baseline + m.desc > pen.bottom() + 0.01 {
                    // Try the next row/column; overflow if there is none.
                    pen.ri += 1;
                    pen.prev = None;
                    pen.pending = 0.0;
                    pen.queued.clear();
                    continue;
                }
                break (baseline, x0, x1, end, hyph, m);
            };
            let (ax0, ax1) = if regions.is_some() { (x0 + ind_l, x1 - para.right_indent) } else { (x0, x1) };
            let width = ax1 - ax0;
            pen.pending = 0.0;
            let last_of_para = end >= n;
            let mut trimmed = end;
            while trimmed > i && sg[trimmed - 1].is_space() {
                trimmed -= 1;
            }
            let hyphen = (hyph && end > i).then(|| hyphen_glyph(&sg[end - 1]));
            // Mojikumi: a closing bracket, comma or full stop ending the line is set half width, and
            // no Japanese–Latin space is left at the end of a line.
            let end_trim = match trimmed.checked_sub(1).filter(|&k| k >= i && para.mojikumi == Mojikumi::LineEndHalf) {
                Some(k) => {
                    let half = if compressed.get(k).copied().unwrap_or(false) { None } else { sg.get(k).and_then(|g| punct_half(g, Punct::Closing)) };
                    half.unwrap_or(0.0) + wakan.get(k).copied().unwrap_or(0.0)
                }
                None => 0.0,
            };
            let w: f64 = sg[i..trimmed].iter().map(|g| g.adv).sum::<f64>() + hyphen.as_ref().map_or(0.0, |h| h.adv) - end_trim;
            // Burasagari: a comma or full stop ending an area type line hangs outside it (Standard:
            // when it doesn't fit; Forced: always). The rest of the line is aligned and justified
            // without it, and it follows the line's last character.
            let hang = (regions.is_some() && !rtl && hyphen.is_none() && trimmed > i + 1)
                .then(|| sg.get(trimmed - 1))
                .flatten()
                .filter(|g| hangs(g))
                .map(|g| g.adv - end_trim)
                .filter(|_| match para.burasagari {
                    Burasagari::None => false,
                    Burasagari::Standard => w > width + EPS,
                    Burasagari::Forced => true,
                });
            let body = if hang.is_some() { trimmed - 1 } else { trimmed };
            let w = w - hang.unwrap_or(0.0);
            let (align, justify) = match para.justify {
                Justify::Auto => (if rtl { 2 } else { 0 }, false),
                Justify::Left => (0, false),
                Justify::Center => (1, false),
                Justify::Right => (2, false),
                Justify::JustifyLeft => (0, !last_of_para),
                Justify::JustifyCenter => (1, !last_of_para),
                Justify::JustifyRight => (2, !last_of_para),
                Justify::JustifyAll => (0, true),
            };
            let justify = justify && regions.is_some();
            let (mut per_space, mut per_gap, mut per_cjk) = (0.0, 0.0, 0.0);
            let spaces = sg[i..body].iter().filter(|g| g.is_space()).count();
            // Japanese (and Chinese) lines are justified between their characters (JLREQ 3.8): the
            // gaps next to a CJK character, not inside a Latin word or a tate-chu-yoko block.
            let cjk_gap = |j: usize| {
                j + 1 < body
                    && sg
                        .get(j)
                        .zip(sg.get(j + 1))
                        .is_some_and(|(a, b)| !a.is_space() && !b.is_space() && !b.continues_tcy() && a.ch != '\t' && (is_cjk(a.ch) || is_cjk(b.ch)))
            };
            let cjk_gaps = (i..body).filter(|&j| cjk_gap(j)).count();
            if justify && (width - w).abs() > EPS {
                if cjk_gaps > 0 && width > w {
                    // Spread over the CJK gaps and the word spaces alike.
                    let per = (width - w) / (cjk_gaps + spaces) as f64;
                    (per_space, per_cjk) = (per, per);
                } else if spaces > 0 {
                    // Composed lines may shrink word spaces (never below zero).
                    per_space =
                        ((width - w) / spaces as f64).max(-sg[i..body].iter().filter(|g| g.is_space()).map(|g| g.adv).fold(f64::MAX, f64::min));
                } else if para.justify == Justify::JustifyAll && width > w {
                    let gaps = sg.get(i + 1..body).map_or(0, |s| s.iter().filter(|g| !g.continues_tcy()).count());
                    if gaps > 0 {
                        per_gap = (width - w) / gaps as f64;
                    }
                }
            }
            let start_x = if justify {
                ax0
            } else if regions.is_none() {
                match align {
                    0 => ind_l,
                    1 => (ind_l - para.right_indent - w) * 0.5,
                    _ => -para.right_indent - w,
                }
            } else {
                match align {
                    0 => ax0,
                    1 => ax0 + (width - w) * 0.5,
                    _ => ax1 - w,
                }
            };
            let li = cx.out.lines.len();
            let glyph_start = cx.out.glyphs.len();
            // Character Alignment lines smaller characters up with the line's largest em box.
            let line_em = sg.get(i..end).map_or(0.0, |line| line.iter().map(glyph_em).fold(0.0, f64::max));
            let mut x_end = start_x;
            // Tab stops are measured from the frame's left edge (point type: the origin).
            let tab_origin = if regions.is_some() { x0 } else { 0.0 };
            let line_start = sg.get(i).map_or(pr.start, |g| g.byte);
            let line_end = sg.get(end).map_or(pr.end, |g| g.byte);
            let order = visual_order(bidi.as_ref().filter(|_| !cx.vertical), pr.start, line_start..line_end, &sg[i..end]);
            // RTL's logical trailing spaces precede the visible content. Keep them outside
            // the aligned content extent, just as LTR trailing spaces extend to its right.
            let leading_space_width: f64 =
                order.iter().take_while(|&&offset| i + offset >= trimmed).filter_map(|&offset| sg.get(i + offset)).map(|g| g.adv).sum();
            let mut x = start_x - leading_space_width;
            for offset in order {
                let j = i + offset;
                let Some(g) = sg.get(j) else { continue };
                let mut adv = g.adv;
                if j + 1 == trimmed {
                    adv -= end_trim;
                }
                if g.ch == '\t' {
                    adv = tab_advance(&para.tabs, tab_origin, x, &sg[j + 1..trimmed.max(j + 1)]);
                } else if j < body {
                    if g.is_space() {
                        adv += per_space;
                    } else if per_cjk != 0.0 {
                        if cjk_gap(j) {
                            adv += per_cjk;
                        }
                    } else if j + 1 < body && sg.get(j + 1).is_some_and(|next| !next.continues_tcy()) {
                        // Between glyphs, never inside a tate-chu-yoko block (one cell).
                        adv += per_gap;
                    }
                }
                let y = baseline - align_shift(g, cx.style_at(g.byte).char_align, line_em, cx.vertical);
                cx.emit(g, Affine::translate((x, y)), Point::new(x, y), 0.0, adv, li);
                x += adv;
                if j < trimmed {
                    x_end = x;
                }
            }
            if let Some(h) = &hyphen {
                // The hyphen follows the last non-space glyph.
                let hx = x_end;
                cx.emit(h, Affine::translate((hx, baseline)), Point::new(hx, baseline), 0.0, h.adv, li);
                x_end = hx + h.adv;
            }
            cx.out.lines.push(LineInfo {
                rtl: rtl && !cx.vertical,
                baseline,
                x0: start_x,
                x1: x_end,
                ascent: m.asc,
                descent: m.desc,
                start: if i < n { sg[i].byte } else { pr.start },
                end: if last_of_para { pr.end } else { sg[end].byte },
                glyph_start,
                glyph_end: cx.out.glyphs.len(),
                avail: if regions.is_some() { (ax0, ax1) } else { (start_x, x_end) },
                region: if regions.is_some() { pen.ri } else { 0 },
            });
            pen.settled(baseline, m);
            // Further spans of this line band share the settled baseline.
            for q in &mut pen.queued {
                q.0 = baseline;
            }
            li_para += 1;
            i = end;
            if i >= n {
                break;
            }
        }
        pen.pending += para.space_after;
    }
}

/// The distance along type on a path each glyph takes: its advance measured `spacing` points above
/// the path (Type on a Path Options › Spacing), so glyphs close up round the outside of a curve and
/// open up round the inside; just the advances without spacing. Glyphs start `from` along the path.
fn path_steps(ap: &ArcPath, glyphs: &[SGlyph], from: f64, spacing: f64) -> Vec<f64> {
    let mut s = from;
    glyphs
        .iter()
        .map(|g| {
            let mut step = g.adv;
            if spacing != 0.0 && g.adv > 1e-9 {
                // How far the path turns across the glyph: positive bending away from its top.
                let ((_, a), (_, b)) = (ap.at(s), ap.at(s + g.adv));
                let curvature = a.cross(b).atan2(a.dot(b)) / g.adv;
                step = g.adv / (1.0 + curvature * spacing).clamp(0.25, 4.0);
            }
            s += step;
            step
        })
        .collect()
}

/// Lay type on a path out along `path` (text space), between its brackets.
fn on_path(cx: &mut Ctx<'_>, paras: &[Range<usize>], t: &TextObject, path: &PathData) {
    cx.out.on_path = true;
    // Type on a path is one line: the first paragraph's attributes align it.
    let para = t.para_at(0);
    let mut sg = Vec::new();
    // The first paragraph's direction aligns the line (Auto) and sets the caret's.
    let mut rtl = None;
    for pr in paras {
        let text = cx.text;
        let bidi = para_bidi(text.get(pr.clone()).unwrap_or_default(), para.direction);
        rtl.get_or_insert(is_rtl(bidi.as_ref()) && !cx.vertical);
        let shaped = cx.shape_para(pr.clone(), bidi.as_ref());
        for i in visual_order(bidi.as_ref().filter(|_| !cx.vertical), pr.start, pr.clone(), &shaped) {
            if let Some(g) = shaped.get(i) {
                sg.push(g.clone());
            }
        }
    }
    let rtl = rtl.unwrap_or(false);
    let ap = ArcPath::new(path);
    let m = Metrics::max(&sg).map(|m| (m.asc, m.desc)).unwrap_or_else(|| {
        let s = style_metrics(cx.db, cx.style_at(0));
        (s.0, s.1)
    });
    let text_len = cx.text.len();
    if ap.is_empty() {
        cx.out.overflow = !sg.is_empty();
        cx.out.lines.push(LineInfo {
            rtl,
            baseline: 0.0,
            x0: 0.0,
            x1: 0.0,
            ascent: m.0,
            descent: m.1,
            start: 0,
            end: text_len,
            glyph_start: 0,
            glyph_end: 0,
            avail: (0.0, 0.0),
            region: 0,
        });
        return;
    }
    let centre = path.bounds().unwrap_or_default().center();
    let (from, to) = t.kind.path_span().unwrap_or((0.0, 1.0));
    let s_start = from * ap.len();
    let avail = (to - from) * ap.len();
    let spacing = if t.path_spacing.is_finite() { t.path_spacing } else { 0.0 };
    let mut steps = path_steps(&ap, &sg, s_start, spacing);
    let w: f64 = steps.iter().sum();
    let s0 = match para.justify {
        Justify::Auto if rtl => s_start + (avail - w).max(0.0),
        Justify::Center | Justify::JustifyCenter => s_start + ((avail - w) * 0.5).max(0.0),
        Justify::Right | Justify::JustifyRight => s_start + (avail - w).max(0.0),
        _ => s_start,
    };
    if spacing != 0.0 && s0 != s_start {
        // Spaced from where the alignment puts them, the curve under them is another.
        steps = path_steps(&ap, &sg, s0, spacing);
    }
    // Align to Path: how far down (glyph space) the type moves to run its ascender, centre or
    // descender along the path instead of its baseline.
    let rise = match t.path_align {
        PathAlign::Baseline => 0.0,
        PathAlign::Ascender => m.0,
        PathAlign::Descender => -m.1,
        PathAlign::Center => (m.0 - m.1) * 0.5,
    };
    let mut x = 0.0;
    for (g, &step) in sg.iter().zip(&steps) {
        let s = s0 + x;
        if s + step - s_start > avail + 1e-6 {
            cx.out.overflow = true;
            break;
        }
        let (p, dir) = ap.at(s + step * 0.5);
        let angle = dir.y.atan2(dir.x);
        let half = Affine::translate((-g.adv * 0.5, rise));
        // Glyph space: x along the advance, y down from the baseline; `pre` maps it onto the path.
        let pre = match t.path_effect {
            PathEffect::Rainbow => Affine::translate(p.to_vec2()) * Affine::rotate(angle) * half,
            // x axis along the tangent, y axis stays vertical.
            PathEffect::Skew => Affine::translate(p.to_vec2()) * Affine::new([dir.x, dir.y, 0.0, 1.0, 0.0, 0.0]) * half,
            // x axis stays horizontal (facing the path's direction), y axis perpendicular to the path.
            PathEffect::Ribbon3d => {
                let sx = if dir.x < 0.0 { -1.0 } else { 1.0 };
                Affine::translate(p.to_vec2()) * Affine::new([sx, 0.0, -dir.y * sx, dir.x * sx, 0.0, 0.0]) * half
            }
            PathEffect::StairStep => Affine::translate(ap.at(s).0.to_vec2()) * Affine::translate((0.0, rise)),
            // x axis along the tangent; vertical edges point at the path's centre (kept on the glyph's
            // up side, and never closer than ~17° to the baseline so glyphs stay legible).
            PathEffect::Gravity => {
                let n = Vec2::new(dir.y, -dir.x);
                let mut up = p - centre;
                up = if up.hypot() < 1e-9 { n } else { up / up.hypot() };
                if up.dot(n) < 0.0 {
                    up = -up;
                }
                if up.dot(n) < 0.3 {
                    let along = up - n * up.dot(n);
                    up = n * 0.3 + along / along.hypot().max(1e-9) * (1.0 - 0.09f64).sqrt();
                }
                Affine::translate(p.to_vec2()) * Affine::new([dir.x, dir.y, -up.x, -up.y, 0.0, 0.0]) * half
            }
        };
        // The caret's baseline moves with the type.
        let origin = p - dir * (g.adv * 0.5) + Vec2::new(-dir.y, dir.x) * rise;
        cx.emit(g, pre, origin, angle, g.adv, 0);
        x += step;
    }
    let (ps, _) = ap.at(s0);
    let (pe, _) = ap.at(s0 + x);
    cx.out.lines.push(LineInfo {
        rtl,
        baseline: ps.y,
        x0: ps.x,
        x1: pe.x,
        ascent: m.0,
        descent: m.1,
        start: 0,
        end: text_len,
        glyph_start: 0,
        glyph_end: cx.out.glyphs.len(),
        avail: (0.0, ap.len()),
        region: 0,
    });
}

/// The bidirectional resolution (UAX #9) of paragraph `text`, its base direction `direction` or,
/// without one, from its first strong character. `None` for the usual paragraph with nothing right
/// to left in it (no right-to-left character or direction), which skips the algorithm.
pub(crate) fn para_bidi(text: &str, direction: Option<ParaDirection>) -> Option<BidiInfo<'_>> {
    use unicode_bidi::BidiClass::{AL, AN, FSI, R, RLE, RLI, RLO};
    let rtl_set = direction == Some(ParaDirection::RightToLeft);
    if !rtl_set && !text.chars().any(|c| matches!(unicode_bidi::bidi_class(c), R | AL | AN | RLE | RLO | RLI | FSI)) {
        return None;
    }
    let level = direction.map(|d| if d == ParaDirection::RightToLeft { Level::rtl() } else { Level::ltr() });
    Some(BidiInfo::new(text, level))
}

/// Does the paragraph run right to left ([`para_bidi`])?
pub(crate) fn is_rtl(bidi: Option<&BidiInfo<'_>>) -> bool {
    bidi.and_then(|b| b.paragraphs.first()).is_some_and(|p| p.level.is_rtl())
}

/// The visual order of a line's `glyphs` (indices into them), whole shaping clusters reordered
/// (UAX #9 L1–L2) for the line `line` (bytes of the text) of the paragraph starting at byte
/// `paragraph_start`; their own order when nothing is right to left (`bidi` None).
fn visual_order(bidi: Option<&BidiInfo<'_>>, paragraph_start: usize, line: Range<usize>, glyphs: &[SGlyph]) -> Vec<usize> {
    let Some((bidi, para)) = bidi.filter(|b| b.has_rtl()).and_then(|b| Some((b, b.paragraphs.first()?))) else {
        return (0..glyphs.len()).collect();
    };
    if glyphs.is_empty() {
        return vec![];
    }
    let levels = bidi.reordered_levels(para, line.start.saturating_sub(paragraph_start)..line.end.saturating_sub(paragraph_start));
    let mut clusters: Vec<Range<usize>> = Vec::new();
    for (i, g) in glyphs.iter().enumerate() {
        if i > 0 && glyphs.get(i - 1).is_some_and(|p| p.byte == g.byte) {
            if let Some(c) = clusters.last_mut() {
                c.end = i + 1;
            }
        } else {
            clusters.push(i..i + 1);
        }
    }
    let cluster_levels: Vec<_> = clusters
        .iter()
        .filter_map(|c| glyphs.get(c.start))
        .map(|g| levels.get(g.byte.saturating_sub(paragraph_start)).copied().unwrap_or(g.level))
        .collect();
    unicode_bidi::BidiInfo::reorder_visual(&cluster_levels).into_iter().filter_map(|i| clusters.get(i)).flat_map(|c| c.clone()).collect()
}
