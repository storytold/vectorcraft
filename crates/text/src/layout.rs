//! Line breaking and glyph placement for point, area and on-path type.

use std::ops::Range;

use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape, Vec2};
use unicode_bidi::{BidiInfo, Level};
use vectorcraft_doc::text::Kashida;
use vectorcraft_doc::{
    Burasagari, CharStyle, HyphenationOptions, InlineArt, Justify, Kinsoku, LeadingModel, Mojikumi, ParaDirection, ParaStyle, PathAlign, PathEffect,
    RubyAlign, RubyPosition, SingleWordJustify, TextKind, TextObject,
};
use vectorcraft_geom::{ArcPath, PathData};

use crate::composer::{Breakpoint, Params, compose};
use crate::fontdb::FontDb;
use crate::hyphen::hyphen_points_with;
use crate::justify::{Flex, LineFit, apply_desired, fit_line, is_word_space};
use crate::kashida;
use crate::shape::{
    Punct, SGlyph, Tcy, cap_x_heights, hyphen_glyph, is_cjk, mark_word_ends, no_line_end, no_line_start, punct, shape_range, soft_line_start,
    style_metrics,
};
use crate::{Composer, FirstBaseline, InlineGlyph, LayoutOptions, LineInfo, ListMarker, OtFeatures, PositionedGlyph, TextLayout, VerticalAlign};

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
    /// Character panel menu › System Layout. On: positive advances are whole points.
    system_layout: bool,
    /// Character panel menu › Fractional Widths. Off, positive advances are whole points.
    fractional_widths: bool,
    out: TextLayout,
}

impl Ctx<'_> {
    fn style_at(&self, b: usize) -> &CharStyle {
        // The runs follow one another: the first that ends after `b` is the only one that can hold it.
        let find = |b: usize| {
            let i = self.runs.partition_point(|(r, _)| r.end <= b);
            self.runs.get(i).filter(|(r, _)| r.start <= b).map(|(_, s)| *s)
        };
        find(b).or_else(|| b.checked_sub(1).and_then(find)).or_else(|| self.runs.first().map(|(_, s)| *s)).unwrap_or(&self.default)
    }

    /// The stretches of paragraph `r` whose characters have a Character Direction, as byte ranges
    /// from `r.start` with their direction, in order; neighboring runs of one direction make one
    /// stretch.
    fn char_directions(&self, r: &Range<usize>) -> Vec<(Range<usize>, ParaDirection)> {
        let mut out: Vec<(Range<usize>, ParaDirection)> = Vec::new();
        for (rr, st) in &self.runs {
            let (a, b) = (rr.start.max(r.start), rr.end.min(r.end));
            let Some(d) = st.char_direction.filter(|_| a < b) else { continue };
            let (a, b) = (a - r.start, b - r.start);
            match out.last_mut() {
                Some((prev, pd)) if *pd == d && prev.end == a => prev.end = b,
                _ => out.push((a..b, d)),
            }
        }
        out
    }

    /// Shape paragraph `r` (its bidi resolution `bidi`).
    fn shape_para(&self, r: Range<usize>, bidi: Option<&BidiInfo<'_>>) -> Vec<SGlyph> {
        let mut v = Vec::with_capacity(r.len());
        let levels = bidi.map_or(&[][..], |b| &b.levels);
        shape_range(self.db, self.text, r, &self.runs, &self.inlines, &self.opts.features, levels, &mut v);
        self.finish_shaping(&mut v, |g| self.style_at(g.byte));
        v
    }

    /// Set shaped glyphs `v` as the type sets them: in vertical type, tate-chu-yoko (by the size
    /// and Tate-chu-yoko of each glyph's `style`) and the cells of upright glyphs; with System
    /// Layout on or Fractional Widths off, whole-point advances.
    fn finish_shaping<'s>(&self, v: &mut [SGlyph], style: impl Fn(&SGlyph) -> &'s CharStyle) {
        if self.vertical {
            tate_chu_yoko(v, |g| {
                let st = style(g);
                (st.size, st.tate_chu_yoko)
            });
            // An upright glyph advances down the column by its vertical advance (the font's vertical
            // metrics; without them at least one em, as CJK fonts' is), centred across it: a narrow
            // mark like § must not overlap its neighbours.
            for g in v.iter_mut().filter(|g| g.adv > 0.0 && stands_upright(g)) {
                let extra = upright_cell(g) - g.face.advance(g.gid) * g.sx;
                g.dx += extra * 0.5;
                g.adv += extra;
                // Proportional Metrics: `vpal` (and the vertical kerning with it) moves it and
                // changes its length down the column, in fonts with vertical metrics (else the cell
                // stays as it is, #966).
                if let Some(a) = g.vpal.filter(|_| g.tcy.is_none() && g.face.vertical_glyph(g.gid).is_some()) {
                    g.dx += a.dx;
                    g.dy += a.dy;
                    g.adv += a.advance;
                    g.proportional = a.proportional;
                }
            }
        }
        // System Layout or Fractional Widths off: whole-point advances, after tate-chu-yoko and the
        // upright cell have set the widths the line breaker will use. A zero advance stays zero.
        if self.system_layout || !self.fractional_widths {
            whole_character_widths(v);
        }
    }

    /// Bullets and numbered lists: put marker `text` (the bullet, or the number and its period;
    /// [`list_markers`]) of list paragraph `pr` (attributes `para`, right to left when `rtl`)
    /// before its glyphs `sg` and return how many glyphs went in (none for a paragraph without a
    /// marker). The marker's characters are set in
    /// [`vectorcraft_doc::text::ParaList::marker_style`], and a tab after them takes the text to
    /// the next tab stop past them on the first line, the left indent counting as a stop when no
    /// explicit stop comes before it. In a horizontal right-to-left paragraph, whose indents are
    /// mirrored ([`line_indents`]), these are measured from the right, and a number reads left to
    /// right with its period at its left. The tab keeps that width however the line is aligned or
    /// justified. Each of these glyphs stands for no character of the text: length 0, at the
    /// paragraph's first byte, in its first character's run, at the paragraph's embedding level.
    fn prepend_marker(&self, pr: &Range<usize>, para: &ParaStyle, text: Option<&str>, rtl: bool, sg: &mut Vec<SGlyph>) -> usize {
        let Some(text) = text.filter(|_| para.list.is_some() && !pr.is_empty()) else { return 0 };
        let st = vectorcraft_doc::text::ParaList::marker_style(self.style_at(pr.start));
        let Some(run) = self.runs.iter().position(|(r, _)| r.start <= pr.start && pr.start < r.end) else { return 0 };
        // The paragraph's direction: the marker starts a right-to-left paragraph at its right.
        let level = if rtl { Level::rtl() } else { Level::ltr() };
        let mut marker = Vec::with_capacity(text.len() + 1);
        shape_range(self.db, text, 0..text.len(), &[(0..text.len(), &st)], &[None], &self.opts.features, &vec![level; text.len()], &mut marker);
        if rtl {
            // [`visual_order`] draws a right-to-left marker's glyphs in reverse: the number's
            // glyphs go in reversed, so that it reads left to right, and the period after them.
            let number = marker.iter().take_while(|g| text.get(g.byte..).is_some_and(|s| !s.starts_with('.'))).count();
            marker[..number].reverse();
        }
        for g in &mut marker {
            g.byte = pr.start;
            g.len = 0;
            g.run = run;
        }
        self.finish_shaping(&mut marker, |_| &st);
        let Some(last) = marker.last() else { return 0 };
        let mut tab = last.clone();
        tab.ch = '\t';
        tab.gid = 0;
        tab.dx = 0.0;
        tab.dy = 0.0;
        tab.tcy = None;
        tab.vpal = None;
        let pen = para.left_indent + para.first_line_indent + marker.iter().map(|g| g.adv).sum::<f64>();
        let to_indent = para.left_indent - pen;
        let stop_before_indent = para.tabs.iter().any(|t| t.position > pen + EPS && t.position < para.left_indent - EPS);
        tab.adv = if to_indent > EPS && !stop_before_indent { to_indent } else { tab_advance(&para.tabs, 0.0, pen, sg).0 };
        marker.push(tab);
        let n = marker.len();
        sg.splice(0..0, marker);
        n
    }

    fn emit(&mut self, g: &SGlyph, pre: Affine, origin: Point, angle: f64, advance: f64, line: usize) {
        // Control characters (tabs) and soft hyphens draw nothing (fonts map them to .notdef).
        let blank = g.is_soft_hyphen() || g.ch.is_control();
        // A colour glyph draws its art (COLR layers, an SVG document) or its image (a bitmap, or
        // COLR paints documents have no form for), not its outline; COLR, then SVG, then bitmaps.
        // Art in the text's colours takes the run's fill (black when it isn't a colour) and
        // stroke.
        let plain = g.inline.is_some() || blank;
        let colour_of = |p: &vectorcraft_doc::color::Paint| match p {
            vectorcraft_doc::color::Paint::Solid { color, .. } => Some(*color),
            _ => None,
        };
        let st = self.style_at(g.byte);
        let foreground = colour_of(&st.fill).unwrap_or(vectorcraft_doc::color::Color::BLACK);
        let stroke = colour_of(&st.stroke).filter(|_| st.has_stroke());
        let (colour, image) = match (!plain).then(|| self.db.glyph_colour(&g.face, g.gid, foreground, stroke)).flatten() {
            Some(crate::colr::ColourContent::Art(art)) => (Some(art), None),
            Some(crate::colr::ColourContent::Image(image)) => (None, Some(image)),
            None => (None, (!plain).then(|| self.db.glyph_image(&g.face, g.gid)).flatten()),
        };
        let src = if g.inline.is_some() || image.is_some() || colour.is_some() {
            std::sync::Arc::new(BezPath::new())
        } else {
            self.db.outline(&g.face, g.gid)
        };
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
        let outline = if src.elements().is_empty() || blank {
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
            scale: 1.0,
            image,
            colour,
        });
    }

    /// Draw a kashida `len` points long from `at` (the pen on the baseline) on line `line`, as
    /// tatweels in the face, size and style of glyph `g`. Short, Medium and Long kashidas are one
    /// tatweel stretched along the baseline; a Stylistic one is tatweels side by side. Each tatweel
    /// is a glyph at `g`'s byte offset that stands for no character (length 0) and does not
    /// advance: `g`'s advance takes the kashida in.
    fn emit_kashida(&mut self, g: &SGlyph, kind: Kashida, len: f64, at: Point, line: usize) {
        let Some((gid, t)) = kashida::tatweel(g) else { return };
        let mut k = g.clone();
        k.gid = gid;
        k.adv = t;
        k.dx = 0.0;
        k.dy = 0.0;
        k.len = 0;
        k.lead = 0.0;
        k.ch = kashida::TATWEEL;
        k.inline = None;
        k.tcy = None;
        if kind == Kashida::Stylistic {
            let n = (len / t).round().clamp(1.0, kashida::MAX_TATWEELS) as usize;
            for m in 0..n {
                let x = at.x + m as f64 * t;
                self.emit(&k, Affine::translate((x, at.y)), Point::new(x, at.y), 0.0, 0.0, line);
            }
            return;
        }
        self.emit(&k, Affine::translate((at.x, at.y)) * Affine::scale_non_uniform(len / t, 1.0), at, 0.0, 0.0, line);
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

/// Area type in a frame too short for its first line draws nothing (type that doesn't fit its frame
/// isn't drawn). For importers whose source shows that line: the frame is made taller, from its
/// top, by what the first line needs. Whether it was.
pub fn fit_first_line(db: &FontDb, t: &mut TextObject) -> bool {
    let TextKind::Area { frame } = &t.kind else { return false };
    let Some(b) = frame.bounds().filter(|b| b.height() > 1e-6) else { return false };
    if !layout(db, t).lines.is_empty() {
        return false;
    }
    // The height of its first line: the line's as point type.
    let point = TextObject { kind: TextKind::Point, ..t.clone() };
    let Some(need) = layout(db, &point).lines.first().map(|l| l.ascent + l.descent) else { return false };
    // (A little more than it, for the rounding of the layout.)
    let by = (need + 0.05) / b.height();
    if !(by.is_finite() && by > 1.0) {
        return false;
    }
    let taller = frame.transformed(Affine::translate((0.0, b.y0)) * Affine::scale_non_uniform(1.0, by) * Affine::translate((0.0, -b.y0)));
    let short = std::mem::replace(&mut t.kind, TextKind::Area { frame: taller });
    // Something else keeps the line out (a frame narrower than a letter): as the source has it.
    if layout(db, t).lines.is_empty() {
        t.kind = short;
        return false;
    }
    true
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
        // The stretch: the glyphs of this run on this line, in a row. A list marker has no
        // underline or strikethrough.
        let marker = layout.marker_of(i).is_some();
        let end = glyphs
            .iter()
            .enumerate()
            .skip(i)
            .position(|(k, h)| h.run != g.run || h.line != g.line || layout.marker_of(k).is_some() != marker)
            .map_or(glyphs.len(), |n| i + n);
        let stretch = glyphs.get(i..end).unwrap_or_default();
        i = end.max(i + 1);
        if marker {
            continue;
        }
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
/// Shrink Text to Fit lays `t` out as `scaled(t, f)`, where `f` is the layout's
/// [`TextLayout::fit_scale`], so the result holds the styles the text is drawn in.
pub fn scaled(t: &TextObject, f: f64) -> TextObject {
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
    let mut cx = Ctx {
        on_path: is_on_path,
        db,
        text: &text,
        runs,
        inlines,
        default: CharStyle::default(),
        opts,
        vertical,
        system_layout: t.system_layout,
        fractional_widths: t.fractional_widths,
        out: TextLayout::default(),
    };
    // Vertical type: line space turned a quarter turn clockwise (lines become columns, each next
    // one to the left). Point type's anchor is on the first column's centre line. Vertical path
    // type stays on its path (each glyph turned across it in `emit`).
    let line_xf = match &t.kind {
        _ if !vertical => Affine::IDENTITY,
        TextKind::OnPath { .. } => Affine::IDENTITY,
        TextKind::Point => {
            let first = cx.style_at(0);
            let centre = db.style_face(first).map_or(EM_CENTER, |f| f.ideographic_centre());
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
    // Ruby (furigana): placed over its base text in line space, before vertical type turns the
    // lines into columns, so the same pass puts it above (right in vertical type) and widens the
    // lines it sits on. Type on a path has no above/below; its ruby is left out for now.
    if !is_on_path && !t.ruby.is_empty() {
        ruby_layout(&mut cx, t);
    }
    if vertical && !is_on_path {
        // Glyph origins, outlines and transforms to text space (lines stay in line space).
        for g in &mut cx.out.glyphs {
            g.outline.apply_affine(line_xf);
            g.xf = line_xf * g.xf;
            g.origin = line_xf * g.origin;
            g.angle += std::f64::consts::FRAC_PI_2;
        }
        // Ruby glyphs are placed in line space too (upright, along the column, in the vertical
        // pass); the same turn takes them to text space.
        for g in &mut cx.out.ruby {
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

/// System Layout on or Fractional Widths off: each character that takes space advances by a whole
/// number of points, at least one, so small type does not collapse onto itself. A zero advance (a
/// tate-chu-yoko continuation, a zero-width mark) stays zero.
fn whole_character_widths(glyphs: &mut [SGlyph]) {
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
fn tate_chu_yoko(g: &mut [SGlyph], style: impl Fn(&SGlyph) -> (f64, bool)) {
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
/// Fullwidth !/? instead keep the nominal glyph upright (Tu fallback, UAX #50 §3.1):
/// <https://www.unicode.org/reports/tr50/#vo>.
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
        | 0xFE50..=0xFE52 | 0xFE59..=0xFE5E | 0xFF08 | 0xFF09 | 0xFF0C | 0xFF0E | 0xFF1A | 0xFF1B
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

/// A flattened text wrap shape: (polygons, offset, invert).
type FlatWrap = (Vec<Vec<Point>>, f64, bool);

/// One cell of a flattened area-type frame (the whole frame, or one row/column of it).
struct Region {
    /// The cell (frame bounds, or a grid cell of them).
    cell: Rect,
    /// Inset applied inside the frame edges.
    inset: f64,
    polys: std::sync::Arc<[Vec<Point>]>,
    /// The frame is its own bounding rectangle (spans need no polygon intersection).
    rect: bool,
    /// Text Wrap shapes.
    wraps: std::sync::Arc<[FlatWrap]>,
    /// Vertical alignment: how far below the top-aligned position the first line starts.
    shift: f64,
    /// Vertical justification: extra space added between consecutive lines.
    gap: f64,
}

/// The largest magnitude a coordinate of a frame, a text wrap shape or the path of type on a path
/// may have (points), control points included. A frame or path with a larger coordinate, or a
/// coordinate that isn't finite, holds no text, and a text wrap shape with one is ignored.
const MAX_COORD: f64 = 1e9;

/// Are the coordinates of `path`, control points included, finite and within [`MAX_COORD`]?
fn in_range(path: &BezPath) -> bool {
    let near = |p: &Point| p.x.abs() <= MAX_COORD && p.y.abs() <= MAX_COORD;
    path.elements().iter().all(|el| match el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => near(p),
        PathEl::QuadTo(a, p) => near(a) && near(p),
        PathEl::CurveTo(a, b, p) => near(a) && near(b) && near(p),
        PathEl::ClosePath => true,
    })
}

/// Flattened closed polygons of a path; `None` when it isn't [`in_range`].
fn polygons(path: &BezPath) -> Option<Vec<Vec<Point>>> {
    if !in_range(path) {
        return None;
    }
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
    Some(polys)
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
        // A frame out of range has no cells, and a wrap shape out of range is left out.
        let Some(polys) = polygons(path) else { return vec![] };
        // Every cell shares one copy of the frame's polygons and one copy of the text wrap shapes.
        let polys: std::sync::Arc<[Vec<Point>]> = polys.into();
        let wraps: std::sync::Arc<[FlatWrap]> = wrap
            .iter()
            .filter_map(|w| Some((polygons(&w.path.to_bezpath())?, w.wrap.offset.max(0.0), w.wrap.invert)))
            .filter(|w| !w.0.is_empty())
            .collect();
        let bbox = path.bounding_box();
        let rect = polys.len() == 1 && {
            let area: f64 = polys[0].iter().zip(polys[0].iter().cycle().skip(1)).map(|(a, b)| a.x * b.y - b.x * a.y).sum::<f64>().abs() * 0.5;
            (area - bbox.area()).abs() <= bbox.area() * 1e-6 + 1e-9
        };
        let max = vectorcraft_doc::AreaOptions::MAX_ROWS_COLUMNS;
        let (rows, cols) = (opts.rows.clamp(1, max), opts.columns.clamp(1, max));
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
        for (polys, off, invert) in self.wraps.iter() {
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

/// The 10,000 cells of a frame in 100 rows and 100 columns share one copy of the frame's polygons
/// and one copy of the text wrap shapes.
#[cfg(test)]
#[test]
fn the_cells_of_a_frame_share_its_polygons_and_the_wrap_shapes() {
    let frame = kurbo::Circle::new((0.0, 0.0), 1000.0).to_path(0.1);
    let square = PathData::from_bezpath(&Rect::new(-10.0, -10.0, 10.0, 10.0).to_path(0.1));
    let wrap = vectorcraft_doc::WrapShape { path: square, wrap: Default::default() };
    let cells = Region::cells(&frame, &LayoutOptions { rows: 100, columns: 100, ..LayoutOptions::default() }, &[wrap]);
    assert_eq!(cells.len(), 10_000);
    let first = &cells[0];
    assert!(!first.polys.is_empty() && !first.wraps.is_empty());
    assert!(cells.iter().all(|c| c.polys.as_ptr() == first.polys.as_ptr() && c.wraps.as_ptr() == first.wraps.as_ptr()));
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
    /// The narrowest word no line from here down is wide enough for (see [`Pen::place`]).
    no_room_for: f64,
}

/// The largest number of steps layout takes to move a line down a frame cell to find room for it:
/// in a cell taller than this many leads, a step is the cell's height divided by this.
const MAX_STEPS: usize = 100_000;

impl Pen<'_> {
    /// Baseline and horizontal span for a line with estimated metrics `est` and indents that starts
    /// with a word `word` wide; `None` = the frame is full (overflow).
    /// In a frame that isn't a rectangle, lines too short for the word are left empty and it moves
    /// down to the first one wide enough. Only when no line is does it go on the first line with
    /// room for any text, and get broken there.
    /// The `bool` is true for a further span at the previous line's baseline.
    fn place(&mut self, est: Metrics, ind_l: f64, ind_r: f64, word: f64) -> Option<(f64, f64, f64, bool)> {
        if word > 0.0 && word < self.no_room_for {
            let mut pen = self.clone();
            if let Some(placed) = pen.place_at_least(est, ind_l, ind_r, word) {
                *self = pen;
                return Some(placed);
            }
            // Lines further down are no wider for this word: don't search for it, or a wider one, again.
            self.no_room_for = word;
        }
        self.place_at_least(est, ind_l, ind_r, 0.0)
    }

    /// [`Pen::place`], skipping lines of a non-rectangular frame narrower than `min`.
    fn place_at_least(&mut self, est: Metrics, ind_l: f64, ind_r: f64, min: f64) -> Option<(f64, f64, f64, bool)> {
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
            // The baseline moves down the cell a lead at a time, or a `MAX_STEPS`th of the cell's
            // height when that is more, in at most `MAX_STEPS` steps.
            let step = est.lead.max(1.0).max(r.cell.height() / MAX_STEPS as f64);
            for _ in 0..=MAX_STEPS {
                if baseline + est.desc > r.bottom() + 0.01 {
                    break;
                }
                // A rectangle's lines are all as wide: none below has more room for the word
                // (until wrap shapes can narrow the first baseline: they stay bounded).
                let constant = (r.rect || r.polys.is_empty()) && r.wraps.is_empty();
                let min = if constant { 0.0 } else { min };
                let fits = |&(a, b): &(f64, f64)| {
                    let room = b - a - ind_l - ind_r;
                    room > est.asc.max(1.0) && room + EPS >= min
                };
                if r.wraps.is_empty() {
                    match r.span(baseline - est.asc, baseline + est.desc) {
                        Some(s) if fits(&s) => return Some((baseline, s.0, s.1, false)),
                        // Too narrow here is too narrow all the way down the cell: next cell.
                        Some(_) if constant => break,
                        _ => baseline += step,
                    }
                } else {
                    // Wrap objects can split a line: fill every span, left to right.
                    let mut spans: Vec<(f64, f64)> = r.spans(baseline - est.asc, baseline + est.desc).into_iter().filter(fits).collect();
                    if spans.is_empty() {
                        baseline += step;
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
/// every ½ inch), and the leader of that stop (default stops have none). Right, centre and decimal
/// stops align the text that follows (`rest`, up to the next tab) on the stop.
fn tab_advance<'t>(tabs: &'t [vectorcraft_doc::TabStop], origin: f64, x: f64, rest: &[SGlyph]) -> (f64, &'t str) {
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
        (at > rel + 1e-6).then_some((at, t.leader.as_str()))
    });
    let (at, leader) = explicit.unwrap_or_else(|| {
        let last = tabs.iter().map(|t| t.position).fold(0.0, f64::max).max(0.0);
        let base = rel.max(last);
        (((base / vectorcraft_doc::text::DEFAULT_TAB_INTERVAL).floor() + 1.0) * vectorcraft_doc::text::DEFAULT_TAB_INTERVAL, "")
    });
    ((at - rel).max(0.0), leader)
}

/// The width of glyphs `from..to` of `sg` set on a line whose content starts `x_start` from the
/// tab origin: tabs of the text take what [`tab_advance`] gives them, not the width of their
/// glyph. The tab after a list marker (length 0) keeps its own width.
fn set_width(sg: &[SGlyph], from: usize, to: usize, tabs: &[vectorcraft_doc::TabStop], x_start: f64) -> f64 {
    let mut x = x_start;
    for (j, g) in sg.iter().enumerate().take(to).skip(from) {
        x += if g.ch == '\t' && g.len > 0 { tab_advance(tabs, 0.0, x, sg.get(j + 1..to).unwrap_or_default()).0 } else { g.adv };
    }
    x - x_start
}

/// Most copies of a leader drawn in one tab: a tiny type size and a far stop must not make
/// millions of outlines.
const MAX_LEADER_COPIES: f64 = 4096.0;

/// The outline of a tab leader: `leader`'s characters, in the font and size of tab `g`, repeated
/// across the tab's gap `from..to` on baseline `y`. Copies sit on a grid of the leader's width
/// from `origin` (where tab stops are measured from), so the leaders of lines above one another
/// line up; only whole copies are drawn. Characters the font lacks draw nothing.
fn leader_outline(db: &FontDb, g: &SGlyph, leader: &str, origin: f64, from: f64, to: f64, y: f64) -> BezPath {
    let mut out = BezPath::new();
    let glyphs: Vec<(u32, f64)> = leader
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| {
            let gid = g.face.glyph_for(c);
            (gid, g.face.advance(gid) * g.sx)
        })
        .collect();
    let width: f64 = glyphs.iter().map(|(_, a)| a).sum();
    if !(width > EPS && width.is_finite() && from.is_finite() && to.is_finite()) {
        return out;
    }
    let first = ((from - origin) / width - EPS).ceil();
    let copies = (((to - origin) / width + EPS).floor() - first).clamp(0.0, MAX_LEADER_COPIES) as usize;
    let local = Affine::rotate(-g.rotation.to_radians()) * Affine::translate((0.0, -g.bshift)) * Affine::scale_non_uniform(g.sx, g.sy);
    for k in 0..copies {
        let mut pen = origin + (first + k as f64) * width;
        for &(gid, adv) in &glyphs {
            if gid != 0 {
                let m = Affine::translate((pen, y)) * local;
                out.extend(db.outline(&g.face, gid).elements().iter().map(|el| m * *el));
            }
            pen += adv;
        }
    }
    out
}

/// Width of the word starting at glyph `i`: its advances up to the first break opportunity (spaces
/// left out, the hyphen of a soft hyphen in), the least room a line needs not to break inside it.
/// A list marker at `i` (glyphs of length 0) counts whole, with the word after it.
fn word_width(g: &[SGlyph], i: usize) -> f64 {
    /// Glyphs measured at most: a word wrapped over many lines is measured again from each line's
    /// start, and one this long is broken inside whatever cell it meets.
    const MAX_GLYPHS: usize = 1024;
    let rest = g.get(i..).unwrap_or_default();
    let marker = rest.iter().take_while(|gl| gl.len == 0).count();
    let mut w: f64 = rest.iter().take(marker).map(|gl| gl.adv).sum();
    for gl in rest.iter().skip(marker).take(MAX_GLYPHS) {
        if gl.is_space() {
            break;
        }
        if gl.is_soft_hyphen() {
            return w + hyphen_glyph(gl).adv;
        }
        w += gl.adv;
        if gl.break_after() {
            break;
        }
    }
    w
}

/// Greedy break: returns (end glyph index, hyphenated) for a line starting at `i` of `width`.
/// With burasagari, a comma or full stop that doesn't fit ends the line, hanging outside it.
/// `hyphenate` holds the paragraph's Hyphenation options and the Hyphenation Zone in force when
/// the line may hyphenate a word; a line whose text before the overflowing word already ends
/// within the zone breaks before that word. A tab is as wide as it is set: up to its next stop
/// (`tabs`), with the line's content starting `x_start` from the tab origin.
#[allow(clippy::too_many_arguments)]
fn break_line(
    text: &str,
    g: &[SGlyph],
    i: usize,
    width: f64,
    tabs: &[vectorcraft_doc::TabStop],
    x_start: f64,
    hyphenate: Option<(&HyphenationOptions, f64)>,
    burasagari: Burasagari,
    kinsoku: Kinsoku,
) -> (usize, bool) {
    if !width.is_finite() {
        return (g.len(), false);
    }
    let mut x = 0.0;
    let mut last_break: Option<(usize, bool)> = None;
    // A list marker (length 0) is a cluster of its own; the line holds it and at least the first
    // cluster after it.
    let first = i + g.get(i..).unwrap_or_default().iter().take_while(|h| h.len == 0).count();
    let mut j = i;
    while j < g.len() {
        let gl = &g[j];
        let mut cluster_end = j + 1;
        while g.get(cluster_end).is_some_and(|next| next.byte == gl.byte && (next.len == 0) == (gl.len == 0)) {
            cluster_end += 1;
        }
        // No Break: the marked characters stay on one line, including across a space inside the run.
        if gl.no_break {
            while let Some(n) = g.get(cluster_end) {
                if !n.no_break {
                    break;
                }
                let b = n.byte;
                cluster_end += 1;
                while g.get(cluster_end).is_some_and(|next| next.byte == b) {
                    cluster_end += 1;
                }
            }
        }
        let cluster_width: f64 = if gl.ch == '\t' {
            tab_advance(tabs, 0.0, x_start + x, g.get(cluster_end..).unwrap_or_default()).0
        } else {
            g[j..cluster_end].iter().map(|glyph| glyph.adv).sum()
        };
        if j > first && !gl.is_space() && x + cluster_width > width + EPS {
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
        if last.break_after() && last.len > 0 && kinsoku_allows(g, cluster_end - 1, kinsoku) && !splits_no_break(g, cluster_end) {
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
    let in_zone = |zone: f64| {
        let mut te = word_start;
        while te > i && g[te - 1].is_space() {
            te -= 1;
        }
        te > i && zone > 0.0 && width - g[i..te].iter().map(|g| g.adv).sum::<f64>() <= zone + EPS
    };
    if let Some((opts, zone)) = hyphenate
        && g[j].is_letter()
        && !in_zone(zone)
    {
        let mut we = j;
        while we < g.len() && !g[we].is_space() && !g[we].break_after() {
            we += 1;
        }
        let x_ws: f64 = g[i..word_start].iter().map(|g| g.adv).sum();
        let pts: Vec<usize> = hyphen_breaks(text, g, word_start, we, opts)
            .into_iter()
            .filter(|&k| k > 0 && kinsoku_allows(g, k - 1, kinsoku) && !splits_no_break(g, k))
            .collect();
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

/// Optical Margin Alignment: the share of glyph `g`'s advance that hangs outside the margin when
/// `g` starts or ends a line. Quotation marks, hyphens, full stops and commas hang whole, and
/// asterisks, tildes, ellipses, en and em dashes, colons and semicolons hang by half (the amounts
/// published for Roman Hanging Punctuation). A, V, W, Y, v, w and y hang by a tenth of their
/// advance, VectorCraft's own amount for their slanted sides. Other characters, East Asian
/// punctuation (which burasagari hangs) and inline graphics don't hang.
fn optical_share(g: &SGlyph) -> f64 {
    if g.inline.is_some() || g.tcy.is_some() {
        return 0.0;
    }
    match g.ch {
        '-' | '\u{2010}' | '\u{2011}' | '.' | ',' => 1.0,
        c if is_quote(c) => 1.0,
        '*' | '~' | '…' | '–' | '—' | ':' | ';' => 0.5,
        'A' | 'V' | 'W' | 'Y' | 'v' | 'w' | 'y' => 0.1,
        _ => 0.0,
    }
}

/// Roman Hanging Punctuation: the share of glyph `g`'s advance that hangs outside the margin when
/// `g` starts or ends a line: [`optical_share`]'s punctuation, and no letters.
fn roman_share(g: &SGlyph) -> f64 {
    if g.ch.is_alphabetic() { 0.0 } else { optical_share(g) }
}

/// A punctuation mark that hangs ([`optical_share`]), not a letter.
fn optical_punct(g: &SGlyph) -> bool {
    !g.ch.is_alphabetic() && optical_share(g) > 0.0
}

/// A single or double quotation mark: straight, curly, low-9 or reversed-9.
fn is_quote(c: char) -> bool {
    matches!(c, '\'' | '"' | '‘' | '’' | '‚' | '‛' | '“' | '”' | '„' | '‟')
}

/// Optical Margin Alignment and Roman Hanging Punctuation: how far a line hangs outside its left
/// and right margins, from the glyph at each end of `line` (its glyphs in visual order, each with
/// the advance it is set with) and its `share` ([`optical_share`] or [`roman_share`]), and how many glyphs hang at each end → (left, right, left glyphs,
/// right glyphs). A punctuation mark followed by a quotation mark hangs together with it: `."`
/// ending a line and `'"` starting one hang both their glyphs. A line of one glyph hangs at
/// neither end.
fn optical_hangs(line: &[(&SGlyph, f64)], share: fn(&SGlyph) -> f64) -> (f64, f64, usize, usize) {
    let n = line.len();
    if n < 2 {
        return (0.0, 0.0, 0, 0);
    }
    let hang = |k: usize| line.get(k).map_or(0.0, |&(g, adv)| share(g) * adv.max(0.0));
    let punct = |k: usize| line.get(k).is_some_and(|&(g, _)| optical_punct(g));
    let quote = |k: usize| line.get(k).is_some_and(|&(g, _)| is_quote(g.ch) && share(g) > 0.0);
    let mut left = hang(0);
    // The glyphs that hang at the left; the right end takes none of them.
    let mut taken = usize::from(left > 0.0);
    if left > 0.0 && n > 2 && punct(0) && quote(1) {
        left += hang(1);
        taken = 2;
    }
    let mut right = hang(n - 1);
    let mut right_glyphs = usize::from(right > 0.0);
    if right > 0.0 && n > 2 && n - 2 >= taken && punct(n - 2) && quote(n - 1) {
        right += hang(n - 2);
        right_glyphs = 2;
    }
    (left, right, taken, right_glyphs)
}

/// Works as [`optical_hangs`] does for a right-to-left line, whose `line` is also in visual order.
/// The line is read from right to left: `."` ending it hangs both its glyphs at the left, `'"` starting it hangs both at
/// the right, and the right end takes its glyphs first.
fn optical_hangs_rtl(line: &[(&SGlyph, f64)], share: fn(&SGlyph) -> f64) -> (f64, f64, usize, usize) {
    let reversed: Vec<(&SGlyph, f64)> = line.iter().rev().copied().collect();
    let (right, left, right_glyphs, left_glyphs) = optical_hangs(&reversed, share);
    (left, right, left_glyphs, right_glyphs)
}

/// Would a line ending at glyph index `end` split a No Break run?
fn splits_no_break(g: &[SGlyph], end: usize) -> bool {
    end > 0 && g.get(end - 1).is_some_and(|a| a.no_break) && g.get(end).is_some_and(|b| b.no_break)
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
                word_end: false,
                lead: 0.0,
                proportional: false,
                vpal: None,
                level: unicode_bidi::Level::ltr(),
                no_break: false,
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

        let (end, hyphenated) = break_line(text, &glyphs, 0, 3.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard);

        assert!(!hyphenated);
        assert_eq!(end, 2, "oversized base-plus-mark cluster must stay together");
        assert_eq!(source_range(&glyphs, 0, end), 0..4);
        assert_eq!(glyphs.get(end).map(|g| g.byte), Some(4));
    }

    #[test]
    fn greedy_wrap_keeps_an_oversized_cluster_after_a_line_start_atomic() {
        let text = "بَتَث";
        let glyphs = synthetic_glyphs(&[(0, 4, 1.0, 'ب'), (4, 4, 2.0, 'ت'), (4, 4, 2.0, 'ت'), (8, 2, 1.0, 'ث')]);

        let (first_end, _) = break_line(text, &glyphs, 0, 3.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard);
        assert_eq!(first_end, 1);
        assert_eq!(source_range(&glyphs, 0, first_end), 0..4);

        let (second_end, hyphenated) = break_line(text, &glyphs, first_end, 3.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard);

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

    /// No Break keeps the marked characters on one line, including across a space, and moves a
    /// marked run that does not fit to the next line whole.
    #[test]
    fn no_break_keeps_marked_characters_on_one_line() {
        let text = "A B";
        let plain = synthetic_glyphs(&[(0, 1, 10.0, 'A'), (1, 1, 4.0, ' '), (2, 1, 10.0, 'B')]);
        let (end, hyphen) = break_line(text, &plain, 0, 16.0, &[], 0.0, None, Burasagari::None, Kinsoku::None);
        assert!(!hyphen);
        assert_eq!(end, 2, "without the mark the line ends after the space");

        let mut marked = plain;
        for g in &mut marked {
            g.no_break = true;
        }
        let (end, hyphen) = break_line(text, &marked, 0, 16.0, &[], 0.0, None, Burasagari::None, Kinsoku::None);
        assert!(!hyphen);
        assert_eq!(end, 3, "the marked words stay together even though they are wider than the line");

        // 日 and 本 may break between them. Marked, they move together when they do not fit after "A ".
        let text = "A 日本";
        let mut glyphs = synthetic_glyphs(&[(0, 1, 8.0, 'A'), (1, 1, 2.0, ' '), (2, 3, 10.0, '日'), (5, 3, 10.0, '本')]);
        glyphs[2].no_break = true;
        glyphs[3].no_break = true;
        let (end, _) = break_line(text, &glyphs, 0, 22.0, &[], 0.0, None, Burasagari::None, Kinsoku::None);
        assert_eq!(end, 2, "the marked pair moves to the next line together");
        let (end, _) = break_line(text, &glyphs, end, 15.0, &[], 0.0, None, Burasagari::None, Kinsoku::None);
        assert_eq!(end, 4, "on its own line the pair stays together even when it is wider than the line");

        let breaks = candidates(text, &glyphs, None, Kinsoku::None);
        assert!(breaks.iter().all(|b| b.end != 3), "no break between the marked characters: {breaks:?}");
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

/// Glyph indices inside `g[ws..we]` (a word) where a hyphenated break may go under `opts`.
fn hyphen_breaks(text: &str, g: &[SGlyph], ws: usize, we: usize, opts: &HyphenationOptions) -> Vec<usize> {
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
    // The first glyph of each cluster start (not inside a ligature), by its byte: one pass, not a
    // search for each point.
    let mut starts = std::collections::HashMap::new();
    for k in a + 1..b {
        if g[k].byte != g[k - 1].byte {
            starts.entry(g[k].byte).or_insert(k);
        }
    }
    hyphen_points_with(word, opts).into_iter().filter_map(|ci| starts.get(offs.get(ci)?).copied()).collect()
}

/// Break candidates for the every-line composer: hyphenation points under `hyphenate` (the
/// paragraph's Hyphenation options) when it is given. No break comes before a space: a line that
/// may end after a dash, a Thai word or a space ends after the spaces that follow it, as in
/// [`break_line`].
fn candidates(text: &str, g: &[SGlyph], hyphenate: Option<&HyphenationOptions>, kinsoku: Kinsoku) -> Vec<Breakpoint> {
    let mut v = vec![];
    let mut ws = 0;
    for (j, gl) in g.iter().enumerate() {
        let end_of_word = gl.is_space() || gl.break_after();
        if end_of_word {
            if let Some(opts) = hyphenate
                && j > ws
            {
                for k in
                    hyphen_breaks(text, g, ws, j, opts).into_iter().filter(|&k| k > 0 && kinsoku_allows(g, k - 1, kinsoku) && !splits_no_break(g, k))
                {
                    v.push(Breakpoint { end: k, hyphen: hyphen_glyph(&g[k - 1]).adv });
                }
            }
            let hy = if gl.is_soft_hyphen() { hyphen_glyph(gl).adv } else { 0.0 };
            if j + 1 < g.len() && g[j + 1].byte != gl.byte && !g[j + 1].is_space() && kinsoku_allows(g, j, kinsoku) && !splits_no_break(g, j + 1) {
                v.push(Breakpoint { end: j + 1, hyphen: hy });
            }
            ws = j + 1;
        }
    }
    if let Some(opts) = hyphenate
        && g.len() > ws
    {
        for k in
            hyphen_breaks(text, g, ws, g.len(), opts).into_iter().filter(|&k| k > 0 && kinsoku_allows(g, k - 1, kinsoku) && !splits_no_break(g, k))
        {
            v.push(Breakpoint { end: k, hyphen: hyphen_glyph(&g[k - 1]).adv });
        }
    }
    v.sort_by_key(|b| b.end);
    v.dedup_by_key(|b| b.end);
    v
}

/// Paragraphs longer than this (glyphs) are always broken line by line.
const MAX_COMPOSE_GLYPHS: usize = 200_000;

/// Every-line composition of paragraph glyphs `sg` if applicable (area text, justified or ragged,
/// with uniform line metrics or in rectangular cells without text wrap); `None` falls back to the
/// greedy single-line composer. With `continued`, the paragraph continues from the frame before,
/// and its first composed line is not its first line.
fn compose_para(
    cx: &Ctx<'_>,
    sg: &[SGlyph],
    flex: &[Flex],
    para: &ParaStyle,
    pen: &Pen<'_>,
    rtl: bool,
    continued: bool,
) -> Option<Vec<(usize, bool)>> {
    let ragged = matches!(para.justify, Justify::Auto | Justify::Left | Justify::Center | Justify::Right);
    let composer = cx.opts.composer.unwrap_or(para.composer);
    // A paragraph whose commas or full stops may hang is composed line by line (as the Japanese
    // single-line composer does).
    let may_hang = para.burasagari != Burasagari::None && sg.iter().any(hangs);
    // A tab's width depends on where it falls in its line, which the composer's fixed glyph widths
    // can't follow: such a paragraph is broken line by line ([`break_line`] measures its tabs).
    // The tab after a list marker always starts the first line and has a fixed width.
    let has_tab = sg.iter().any(|g| g.ch == '\t' && g.len > 0);
    if composer != Composer::EveryLine || pen.regions.is_none() || sg.len() < 2 || sg.len() > MAX_COMPOSE_GLYPHS || may_hang || has_tab {
        return None;
    }
    let m = Metrics::of(&sg[0]);
    // Line widths are independent of the breaks when every line has the same metrics, or when
    // a line's width does not depend on how far down the frame it is set: rectangular cells
    // without text wrap. Then a paragraph whose characters differ in line metrics (type size,
    // font, leading, vertical scale, an inline graphic taller than the text) is composed too. In
    // other shapes, or with text wrap, a taller line can move the lines below it to where the
    // frame is narrower, so such a paragraph is broken line by line.
    let plain = pen.regions.is_some_and(|r| r.iter().all(|r| (r.rect || r.polys.is_empty()) && r.wraps.is_empty()));
    if !plain && sg.iter().any(|g| Metrics::of(g) != m) {
        return None;
    }
    let total: f64 = sg.iter().map(|g| g.adv).sum();
    let mut sim = pen.clone();
    let mut widths = vec![];
    let mut acc = 0.0;
    // A line's width and placement depend on the sum of its indents alone, which the mirrored
    // indents of a right-to-left list ([`line_indents`]) keep.
    while widths.len() < sg.len() {
        let first = widths.is_empty();
        let ind_l = para.left_indent + if first && !continued { para.first_line_indent } else { 0.0 };
        // Only the first line's first word is known here. Later lines are placed as the composed
        // lines need: each fits its width, so its first word does too.
        let word = if first { word_width(sg, 0) } else { 0.0 };
        let Some((b, x0, x1, _)) = sim.place(m, ind_l, para.right_indent, word) else { break };
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
    let cands = candidates(cx.text, sg, para.hyphenate.then_some(&para.hyphenation), para.kinsoku);
    let justify_last = para.justify == Justify::JustifyAll;
    let hyphen_limit = usize::try_from(para.hyphenation.hyphen_limit).unwrap_or(usize::MAX);
    let params = |tolerance| Params { justify_last, ragged, tolerance, uniform_from, hyphen_limit };
    // First look for breaks that keep every line within its range (ragged: at most one rag zone,
    // a sixth of the width, left on each line; justified: the Justification options' Minimum and
    // Maximum), then accept any lines that fit, the loosest scored worst. Justified text used to
    // stop at four times its stretch and fall back to the single-line composer; the documented
    // default range (word spacing up to 133%) is narrow enough that most paragraphs would.
    let (strict, loose) = (1.0, f64::INFINITY);
    // The same rule as the line loop below: an opening bracket starting a wrapped line gives up
    // the space before it.
    let start_credit = |i: usize| {
        let flush = !rtl && para.mojikumi == Mojikumi::LineEndHalf;
        sg.get(i).filter(|g| flush && g.lead <= 0.0).and_then(|g| punct_half(g, Punct::Opening)).unwrap_or(0.0)
    };
    compose(sg, &width, &start_credit, &cands, flex, &params(strict)).or_else(|| compose(sg, &width, &start_credit, &cands, flex, &params(loose)))
}

/// The list marker of each of the paragraphs `paras` of `t` (None for none): bullets, and numbers
/// counted through consecutive list paragraphs from [`TextObject::list_count`], as
/// [`vectorcraft_doc::text::ParaList::next_marker`] counts them.
fn list_markers(t: &TextObject, paras: &[Range<usize>]) -> Vec<Option<String>> {
    let mut count = t.list_count;
    paras.iter().enumerate().map(|(pi, pr)| vectorcraft_doc::text::ParaList::next_marker(t.para_at(pi).list, pr.is_empty(), &mut count)).collect()
}

/// Whether paragraph `para` (right to left when `rtl`) has its indents mirrored: a horizontal
/// right-to-left list paragraph does.
fn mirrors_indents(para: &ParaStyle, rtl: bool, vertical: bool) -> bool {
    rtl && !vertical && para.list.is_some()
}

/// The indents at the left and at the right of a line of paragraph `para` (its first line when
/// `first`). With `mirror` ([`mirrors_indents`]) the left and first-line indents are measured from
/// the right and the right indent from the left: the bullet of a right-to-left list hangs at the
/// right, and the lines it wraps to line up with the text after the bullet.
fn line_indents(para: &ParaStyle, first: bool, mirror: bool) -> (f64, f64) {
    let start = para.left_indent + if first { para.first_line_indent } else { 0.0 };
    if mirror { (para.right_indent, start) } else { (start, para.right_indent) }
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
        no_room_for: f64::INFINITY,
    };
    let markers = list_markers(t, paras);
    'paras: for (pi, pr) in paras.iter().enumerate() {
        let para = t.para_at(pi);
        let marker_text = markers.get(pi).and_then(Option::as_deref);
        // The leading model is a paragraph attribute: this paragraph's lines (and the space to
        // its first line) follow its own.
        pen.model = para.leading_model;
        let text = cx.text;
        let bidi = para_bidi_with(text.get(pr.clone()).unwrap_or_default(), para.direction, &cx.char_directions(pr));
        let rtl = is_rtl(bidi.as_ref());
        let mut sg = cx.shape_para(pr.clone(), bidi.as_ref());
        // Thai, Lao, Khmer and Myanmar break lines between words, not at spaces.
        mark_word_ends(text.get(pr.clone()).unwrap_or_default(), pr.start, &mut sg);
        // Japanese composition: consecutive punctuation shares one half-em space.
        let mut compressed = if para.mojikumi == Mojikumi::LineEndHalf { compress_punctuation(&mut sg) } else { vec![] };
        // …and a quarter em between Japanese and Latin text.
        let mut wakan = if para.mojikumi == Mojikumi::LineEndHalf { space_japanese_and_latin(&mut sg) } else { vec![] };
        // Justification…: the glyphs at Desired word and letter spacing and glyph scaling, and how
        // far a justified line may move each one from it.
        let mut flex = apply_desired(&mut sg, &para.justification, !cx.vertical);
        // Threaded text: a first paragraph that continues from the frame before has its first
        // line, first-line indent and list marker there.
        let continued = pi == 0 && t.first_para_continues;
        // Bullets and numbers: the list marker's glyphs `0..marker` start the paragraph.
        // Justification and Optical Margin Alignment leave them as they are.
        let marker = if continued { 0 } else { cx.prepend_marker(pr, para, marker_text, rtl, &mut sg) };
        if marker > 0 {
            flex.splice(0..0, vec![Flex::default(); marker]);
            if !compressed.is_empty() {
                compressed.splice(0..0, vec![false; marker]);
            }
            if !wakan.is_empty() {
                wakan.splice(0..0, vec![0.0; marker]);
            }
        }
        // …and Auto Leading: characters whose leading is Auto take this paragraph's share of their size.
        let auto_leading = |st: &CharStyle| st.leading.unwrap_or_else(|| para.justification.auto_leading_for(st.size));
        if para.justification.auto_leading != vectorcraft_doc::JustificationOptions::default().auto_leading {
            for g in sg.iter_mut().filter(|g| g.inline.is_none()) {
                g.leading = auto_leading(cx.style_at(g.byte));
            }
        }
        let pm = {
            let (asc, desc, _) = style_metrics(cx.db, cx.style_at(pr.start));
            let lead = auto_leading(cx.style_at(pr.start));
            let st = cx.style_at(pr.start);
            let (cap, xh) = cap_x_heights(cx.db, st);
            let centre = cx.db.style_face(st).map_or(EM_CENTER, |f| f.ideographic_centre());
            Metrics { asc, desc, lead, cap, xh, top: (centre + 0.5) * st.size * st.v_scale / 100.0 }
        };
        if pi > 0 {
            pen.pending += para.space_before;
        }
        let n = sg.len();
        let composed = compose_para(cx, &sg, &flex, para, &pen, rtl, continued);
        // Hyphenation options: the Hyphenation Zone applies with the Single-line Composer, and the
        // Hyphen Limit counts the lines in a row that end with a hyphen.
        let zone = if cx.opts.composer.unwrap_or(para.composer) == Composer::SingleLine { para.hyphenation.zone } else { 0.0 };
        let hyphen_limit = usize::try_from(para.hyphenation.hyphen_limit).unwrap_or(usize::MAX);
        let mut hyphen_run = 0;
        let mut li_para = 0;
        let mut i = 0;
        let mirror = mirrors_indents(para, rtl, cx.vertical);
        loop {
            let est = if i < n { Metrics::of(&sg[i]) } else { pm };
            let first_line = li_para == 0 && !continued;
            let (ind_l, ind_r) = line_indents(para, first_line, mirror);
            // Measured before Mojikumi narrows an opening bracket, as the composer measures it.
            let word = word_width(&sg, i);
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
                let Some((mut baseline, x0, x1, same_baseline)) = pen.place(est, ind_l, ind_r, word) else {
                    cx.out.overflow = cx.text.len() > if i < n { sg[i].byte } else { pr.start };
                    break 'paras;
                };
                let width = x1 - x0 - ind_l - ind_r;
                let hyphenate = (para.hyphenate && (hyphen_limit == 0 || hyphen_run < hyphen_limit)).then_some((&para.hyphenation, zone));
                let (end, hyph) = match composed.as_ref().and_then(|c| c.get(li_para)) {
                    Some(&(e, h)) if e > i => (e, h),
                    _ if i < n => break_line(cx.text, &sg, i, width, &para.tabs, ind_l, hyphenate, para.burasagari, para.kinsoku),
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
            hyphen_run = if hyph && end > i { hyphen_run + 1 } else { 0 };
            let (ax0, ax1) = if regions.is_some() { (x0 + ind_l, x1 - ind_r) } else { (x0, x1) };
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
            let w: f64 = set_width(&sg, i, trimmed, &para.tabs, ind_l) + hyphen.as_ref().map_or(0.0, |h| h.adv) - end_trim;
            let line_start = sg.get(i).map_or(pr.start, |g| g.byte);
            let line_end = sg.get(end).map_or(pr.end, |g| g.byte);
            let order = visual_order(bidi.as_ref().filter(|_| !cx.vertical), pr.start, line_start..line_end, &sg[i..end]);
            // Optical Margin Alignment or Roman Hanging Punctuation: the share of a glyph that hangs.
            let share: Option<fn(&SGlyph) -> f64> =
                (t.optical_margin || para.roman_hanging).then_some(if t.optical_margin { optical_share } else { roman_share });
            // The glyphs before `stop` in visual order, each with the advance it is set with. A list
            // marker does not hang: it is counted with no width.
            let set = |j: usize, g: &SGlyph| {
                if j < marker {
                    0.0
                } else if j + 1 == trimmed {
                    g.adv - end_trim
                } else {
                    g.adv
                }
            };
            let visible = |stop: usize| -> (Vec<usize>, Vec<(&SGlyph, f64)>) {
                let at: Vec<usize> = order.iter().map(|&offset| i + offset).filter(|&j| j < stop).collect();
                let glyphs = at.iter().filter_map(|&j| sg.get(j).map(|g| (g, set(j, g)))).collect();
                (at, glyphs)
            };
            // Burasagari: a comma or full stop ending an area type line hangs outside it (Standard:
            // when it doesn't fit, counting what hangs at the line's start as room; Forced:
            // always). The rest of the line is aligned and justified without it, and it follows
            // the line's last character.
            let room_at_start = || share.map_or(0.0, |share| optical_hangs(&visible(trimmed).1, share).0);
            let hang = (regions.is_some() && !rtl && hyphen.is_none() && trimmed > i + 1)
                .then(|| sg.get(trimmed - 1))
                .flatten()
                .filter(|g| hangs(g))
                .map(|g| g.adv - end_trim)
                .filter(|_| match para.burasagari {
                    Burasagari::None => false,
                    Burasagari::Standard => w > width + room_at_start() + EPS,
                    Burasagari::Forced => true,
                });
            let body = if hang.is_some() { trimmed - 1 } else { trimmed };
            let w = w - hang.unwrap_or(0.0);
            // The glyphs before the line's trailing spaces (and its burasagari mark), in the order
            // they are drawn.
            let (drawn, mut visible) = visible(body);
            // Optical Margin Alignment: the glyphs at the ends of a line hang outside its margins
            // by `opt_l` and `opt_r`. The line is aligned and justified in a measure that much
            // wider at each end. A left-aligned line then hangs only at its left, a right-aligned
            // line hangs only at its right, and centered and justified lines hang at both; Auto
            // aligns a right-to-left line at its right. A right-to-left line is read from right to
            // left ([`optical_hangs_rtl`]). A line that ends in a burasagari mark hangs only at its
            // left. Justification adds no space after a punctuation mark hanging at the left, nor
            // before one hanging at the right (`held`). A letter that hangs by part of its width
            // keeps its gaps. Glyph Scaling leaves the glyphs that hang (`hung`) at their width, so
            // they hang exactly by their published share. Roman Hanging Punctuation (a paragraph
            // attribute) hangs the same punctuation by the same amounts, and no letters; with
            // Optical Margin Alignment on as well, the letters hang too.
            let (opt_l, opt_r, held, hung) = if let Some(share) = share {
                if let Some(h) = &hyphen {
                    visible.push((h, h.adv));
                }
                let (l, r, left_glyphs, right_glyphs) =
                    if rtl && !cx.vertical { optical_hangs_rtl(&visible, share) } else { optical_hangs(&visible, share) };
                let (r, right_glyphs) = if hang.is_some() { (0.0, 0) } else { (r, right_glyphs) };
                let n_visible = visible.len();
                let punct = |k: usize| visible.get(k).is_some_and(|&(g, _)| optical_punct(g));
                let held: Vec<usize> = drawn
                    .iter()
                    .enumerate()
                    .filter(|&(k, _)| (k < left_glyphs && punct(k)) || (right_glyphs > 0 && k + 1 + right_glyphs >= n_visible && punct(k + 1)))
                    .map(|(_, &j)| j)
                    .collect();
                let hung: Vec<usize> = drawn
                    .iter()
                    .enumerate()
                    .filter(|&(k, _)| k < left_glyphs || (right_glyphs > 0 && k + right_glyphs >= n_visible))
                    .map(|(_, &j)| j)
                    .collect();
                (l, r, held, hung)
            } else {
                (0.0, 0.0, vec![], vec![])
            };
            // Justification adds nothing to the list marker's glyphs.
            let mut held = held;
            held.extend(i..marker.min(body));
            // What justification may change of glyph `j`: nothing of a `held` glyph, and no glyph
            // scaling of a `hung` one.
            let line_flex = |j: usize| {
                if held.contains(&j) {
                    return Flex::default();
                }
                let mut f = flex.get(j).copied().unwrap_or_default();
                if hung.contains(&j) {
                    f.glyph = (0.0, 0.0);
                }
                f
            };
            let room = width + opt_l + opt_r;
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
            let mut fit = LineFit::default();
            let spaces = (i..body).filter(|&j| j >= marker && sg[j].is_space()).count();
            // A gap that justification adds between two glyphs goes into the advance of the one
            // drawn first: `drawn_after(j)` is the glyph drawn right after glyph `j`, which in a
            // right-to-left line comes before it in the text.
            let mut after: Vec<Option<usize>> = vec![None; body.saturating_sub(i)];
            for (&a, &b) in drawn.iter().zip(drawn.iter().skip(1)) {
                if let Some(slot) = after.get_mut(a - i) {
                    *slot = Some(b);
                }
            }
            let drawn_after = |j: usize| j.checked_sub(i).and_then(|k| after.get(k).copied().flatten());
            // Japanese (and Chinese) lines are justified between their characters (JLREQ 3.8): the
            // gaps next to a CJK character, not inside a Latin word or a tate-chu-yoko block.
            let cjk_gap = |j: usize| {
                !held.contains(&j)
                    && drawn_after(j)
                        .and_then(|k| sg.get(j).zip(sg.get(k)))
                        .is_some_and(|(a, b)| !a.is_space() && !b.is_space() && !b.continues_tcy() && a.ch != '\t' && (is_cjk(a.ch) || is_cjk(b.ch)))
            };
            let cjk_gaps = (i..body).filter(|&j| cjk_gap(j)).count();
            // Single Word Justification: a justified line holding one word (no word space) is
            // spread between its letters (Full Justify) or aligned.
            let single_word = justify && spaces == 0 && cjk_gaps == 0;
            let (align, justify) = match (single_word, para.justification.single_word) {
                (false, _) | (true, SingleWordJustify::FullJustify) => (align, justify),
                (true, SingleWordJustify::AlignLeft) => (0, false),
                (true, SingleWordJustify::AlignCenter) => (1, false),
                (true, SingleWordJustify::AlignRight) => (2, false),
            };
            // Insert Kashidas: the Arabic words of a horizontal line take its room on kashidas,
            // and the word spaces and gaps take what is left.
            let kashidas =
                if justify && !cx.vertical && room > w + EPS { kashida::plan(cx.text, &sg, i..body, para.kashida, room - w) } else { vec![] };
            let w = w + kashidas.iter().map(|k| k.1).sum::<f64>();
            if justify && (room - w).abs() > EPS {
                if cjk_gaps > 0 && room > w {
                    // Spread over the CJK gaps and the word spaces alike.
                    let per = (room - w) / (cjk_gaps + spaces) as f64;
                    (per_space, per_cjk) = (per, per);
                } else if spaces > 0 {
                    // Word spacing, letter spacing and glyph scaling within their ranges, then the
                    // word spaces alone; composed lines may shrink word spaces (never below zero).
                    let line: Vec<Flex> = (i..body).map(line_flex).collect();
                    let floor = (i..body).filter(|&j| j >= marker && sg[j].is_space()).map(|j| sg[j].adv).fold(f64::MAX, f64::min);
                    fit = fit_line(&line, room - w, spaces, floor);
                } else if room > w {
                    // One word: its letter spacing and glyph scaling, then the gaps between its glyphs.
                    let line: Vec<Flex> = (i..body).map(line_flex).collect();
                    fit = fit_line(&line, room - w, 0, 0.0);
                    let used: f64 = line.iter().map(|f| fit.letter(f) + fit.glyph(f)).sum();
                    let rest = room - w - used;
                    let gaps = (i..body)
                        .filter(|&j| !held.contains(&j) && drawn_after(j).and_then(|k| sg.get(k)).is_some_and(|next| !next.continues_tcy()))
                        .count();
                    if gaps > 0 && rest > EPS {
                        per_gap = rest / gaps as f64;
                    }
                }
            }
            let start_x = if justify {
                ax0 - opt_l
            } else if regions.is_none() {
                match align {
                    0 => ind_l - opt_l,
                    1 => (ind_l - opt_l - ind_r + opt_r - w) * 0.5,
                    _ => -ind_r + opt_r - w,
                }
            } else {
                match align {
                    0 => ax0 - opt_l,
                    1 => ax0 - opt_l + (room - w) * 0.5,
                    _ => ax1 + opt_r - w,
                }
            };
            let li = cx.out.lines.len();
            let glyph_start = cx.out.glyphs.len();
            // Character Alignment lines smaller characters up with the line's largest em box.
            let line_em = sg.get(i..end).map_or(0.0, |line| line.iter().map(glyph_em).fold(0.0, f64::max));
            let mut x_end = start_x;
            // Tab stops are measured from the frame's left edge (point type: the origin).
            let tab_origin = if regions.is_some() { x0 } else { 0.0 };
            // A line with tabs is measured as if it started at the left (see `set_width`), then
            // aligned as a whole: its tabs stop where they were measured, however it is aligned.
            let tab_shift = start_x - if regions.is_some() { ax0 } else { ind_l };
            // RTL's logical trailing spaces precede the visible content. Keep them outside
            // the aligned content extent, just as LTR trailing spaces extend to its right.
            let leading_space_width: f64 =
                order.iter().take_while(|&&offset| i + offset >= trimmed).filter_map(|&offset| sg.get(i + offset)).map(|g| g.adv).sum();
            let mut x = start_x - leading_space_width;
            // The glyphs drawn for the list marker, and the index of its tab's glyph, when the line starts the paragraph.
            let mut marker_glyphs: Option<Range<usize>> = None;
            let mut marker_tab = 0;
            for (oi, &offset) in order.iter().enumerate() {
                let j = i + offset;
                let Some(g) = sg.get(j) else { continue };
                let mut adv = g.adv;
                if j + 1 == trimmed {
                    adv -= end_trim;
                }
                // Glyph scaling a justified line adds to the glyph (1: none).
                let mut scale = 1.0;
                let mut leader = "";
                if g.ch == '\t' {
                    // The tab after a list marker keeps the width `prepend_marker` gave it.
                    if g.len > 0 {
                        (adv, leader) = tab_advance(&para.tabs, tab_origin, x - tab_shift, &sg[j + 1..trimmed.max(j + 1)]);
                    }
                } else if j < body {
                    let f = line_flex(j);
                    if is_word_space(g) {
                        adv += fit.space(&f);
                    } else {
                        adv += fit.letter(&f) + fit.glyph(&f);
                        scale = fit.glyph_scale(&f);
                    }
                    if g.is_space() {
                        adv += per_space;
                    } else if per_cjk != 0.0 {
                        if cjk_gap(j) {
                            adv += per_cjk;
                        }
                    } else if !held.contains(&j) && drawn_after(j).and_then(|k| sg.get(k)).is_some_and(|next| !next.continues_tcy()) {
                        // Between glyphs, never inside a tate-chu-yoko block (one cell).
                        adv += per_gap;
                    }
                }
                // The kashida before a cluster in the text is drawn at the cluster's right, between
                // it and the letter before it, and the cluster's last glyph advances over it: the
                // caret, selection and hit testing take it as part of the cluster.
                let kashida = kashidas
                    .binary_search_by_key(&g.byte, |k| k.0)
                    .ok()
                    .and_then(|at| kashidas.get(at))
                    .filter(|_| order.get(oi + 1).and_then(|&o| sg.get(i + o)).is_none_or(|next| next.byte != g.byte))
                    .map(|k| k.1);
                adv += kashida.unwrap_or(0.0);
                let y = baseline - align_shift(g, cx.style_at(g.byte).char_align, line_em, cx.vertical);
                let place = if scale != 1.0 { Affine::translate((x, y)) * Affine::scale_non_uniform(scale, 1.0) } else { Affine::translate((x, y)) };
                if j < marker {
                    let at = cx.out.glyphs.len();
                    marker_glyphs = Some(marker_glyphs.map_or(at..at + 1, |r: Range<usize>| r.start.min(at)..r.end.max(at + 1)));
                    if j + 1 == marker {
                        marker_tab = at;
                    }
                }
                cx.emit(g, place, Point::new(x, y), 0.0, adv, li);
                if let Some(out) = cx.out.glyphs.last_mut() {
                    out.scale = flex.get(j).map_or(1.0, |f| f.scale) * scale;
                }
                if !leader.is_empty() {
                    // The leader belongs to the tab's glyph: the gap stays one character for the
                    // caret, selection and hit testing.
                    let dots = leader_outline(cx.db, g, leader, tab_origin, x, x + adv, y);
                    if let Some(tab) = cx.out.glyphs.last_mut() {
                        tab.outline.extend(dots);
                    }
                }
                if let Some(len) = kashida {
                    cx.emit_kashida(g, para.kashida, len, Point::new(x + adv - len, y), li);
                }
                x += adv;
                if j < trimmed {
                    x_end = x;
                }
            }
            if let (Some(glyphs), Some(text)) = (marker_glyphs, marker_text) {
                cx.out.markers.push(ListMarker { glyphs, tab: marker_tab, text: text.to_string() });
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
                first_baseline: if pen.model == LeadingModel::EmBoxTop { m.top.max(pen.fb_min) } else { m.first_baseline(pen.fb, pen.fb_min) },
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
    // The first paragraph's direction aligns the line (Auto) and sets the caret's.
    let mut rtl = None;
    let mut shaped = Vec::new();
    let markers = list_markers(t, paras);
    for (pi, pr) in paras.iter().enumerate() {
        let text = cx.text;
        let bidi = para_bidi_with(text.get(pr.clone()).unwrap_or_default(), para.direction, &cx.char_directions(pr));
        rtl.get_or_insert(is_rtl(bidi.as_ref()) && !cx.vertical);
        let mut glyphs = cx.shape_para(pr.clone(), bidi.as_ref());
        // Bullets and numbers: a list paragraph's text starts after its marker here too, unless
        // the paragraph continues from the frame before in its thread.
        if pi > 0 || !t.first_para_continues {
            let marker_text = markers.get(pi).and_then(Option::as_deref);
            cx.prepend_marker(pr, t.para_at(pi), marker_text, is_rtl(bidi.as_ref()) && !cx.vertical, &mut glyphs);
        }
        shaped.push((pr.clone(), bidi.filter(|b| b.has_rtl() && !cx.vertical), glyphs));
    }
    let rtl = rtl.unwrap_or(false);
    // The text before byte `cut` in visual order, as area type reorders each line it breaks.
    let visual = |cut: usize| {
        let mut sg = Vec::new();
        for (pr, bidi, glyphs) in &shaped {
            let n = glyphs.iter().take_while(|g| g.byte < cut).count();
            let line = glyphs.get(..n).unwrap_or_default();
            let end = glyphs.get(n).map_or(pr.end, |g| g.byte);
            sg.extend(visual_order(bidi.as_ref(), pr.start, pr.start..end, line).into_iter().filter_map(|i| line.get(i).cloned()));
        }
        sg
    };
    let text_len = cx.text.len();
    let mut sg = visual(text_len);
    // A path out of range holds no text, as a path with no length holds none.
    let ap = if in_range(&path.to_bezpath()) { ArcPath::new(path) } else { ArcPath::default() };
    let m = Metrics::max(&sg).map(|m| (m.asc, m.desc)).unwrap_or_else(|| {
        let s = style_metrics(cx.db, cx.style_at(0));
        (s.0, s.1)
    });
    if ap.is_empty() {
        cx.out.overflow = !sg.is_empty();
        cx.out.lines.push(LineInfo {
            rtl,
            baseline: 0.0,
            x0: 0.0,
            x1: 0.0,
            ascent: m.0,
            descent: m.1,
            first_baseline: 0.0,
            start: 0,
            end: 0,
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
    // Optical Margin Alignment and Roman Hanging Punctuation: the brackets are the margins, and the
    // glyphs at the line's ends hang outside them by `hang_l` and `hang_r`, measured along the path.
    // The line is aligned in a span that much longer at each end, and the glyphs that hang at the
    // end (`hung_r` of them) do not make it overflow past the end bracket. On an open path a hang
    // stops at the path's end. On a closed path the two hangs together take at most the stretch
    // between the end and start brackets: the end where the line is placed takes its share first.
    // A centered line's ends each get up to half of it, and either end takes what the other leaves.
    // The hangs are measured on the steps the glyphs take where the line is placed. A right-to-left
    // line is read from right to left ([`optical_hangs_rtl`]); in vertical type `sg` is in text order.
    let hanging = t.optical_margin || para.roman_hanging;
    let hangs = |sg: &[SGlyph], steps: &[f64]| {
        if !hanging {
            return (0.0, 0.0, 0);
        }
        let share: fn(&SGlyph) -> f64 = if t.optical_margin { optical_share } else { roman_share };
        // A list marker (the glyphs of length 0) does not hang: it is counted with no width.
        let line: Vec<(&SGlyph, f64)> = sg.iter().zip(steps).map(|(g, &step)| (g, if g.len == 0 { 0.0 } else { step })).collect();
        let (l, r, _, hung_r) = if rtl { optical_hangs_rtl(&line, share) } else { optical_hangs(&line, share) };
        let (l, r) = if ap.is_closed() {
            let gap = (ap.len() - avail).max(0.0);
            match para.justify {
                j if matches!(j, Justify::Right | Justify::JustifyRight) || (rtl && j == Justify::Auto) => {
                    let r = r.min(gap);
                    (l.min(gap - r), r)
                }
                Justify::Center | Justify::JustifyCenter => {
                    let l = l.min(gap - r.min(gap * 0.5));
                    (l, r.min(gap - l))
                }
                _ => {
                    let l = l.min(gap);
                    (l, r.min(gap - l))
                }
            }
        } else {
            (l.min(s_start), r.min((ap.len() - s_start - avail).max(0.0)))
        };
        (l, r, hung_r)
    };
    // The glyphs `sg` arranged on the path: their steps, where the first starts, and the hangs.
    let arrange = |sg: &[SGlyph]| {
        let mut steps = path_steps(&ap, sg, s_start, spacing);
        let w: f64 = steps.iter().sum();
        let place = |(l, r, _): (f64, f64, usize)| {
            let (lo, room) = (s_start - l, avail + l + r);
            match para.justify {
                Justify::Auto if rtl => lo + (room - w).max(0.0),
                Justify::Center | Justify::JustifyCenter => lo + ((room - w) * 0.5).max(0.0),
                Justify::Right | Justify::JustifyRight => lo + (room - w).max(0.0),
                _ => lo,
            }
        };
        let mut hung = hangs(sg, &steps);
        let mut s0 = place(hung);
        if spacing != 0.0 && s0 != s_start {
            // Spaced from where the alignment puts them, the curve under them is another. That
            // changes the hanging glyphs' steps, so the line is placed again from the new steps
            // until it stays where it is (at most 8 times).
            steps = path_steps(&ap, sg, s0, spacing);
            for _ in 0..8 {
                hung = hangs(sg, &steps);
                let moved = place(hung);
                if (moved - s0).abs() < 1e-9 {
                    break;
                }
                s0 = moved;
                steps = path_steps(&ap, sg, s0, spacing);
            }
        }
        (steps, s0, hung)
    };
    // How many of the arranged glyphs fit before one runs past the end bracket (with the hang).
    let fitting = |steps: &[f64], s0: f64, (hang_l, hang_r, hung_r): (f64, f64, usize)| {
        let (lo, room) = (s_start - hang_l, avail + hang_l + hang_r);
        let mut x = 0.0;
        steps
            .iter()
            .enumerate()
            .take_while(|&(k, &step)| {
                let end = if k + hung_r >= steps.len() { room } else { room - hang_r };
                let fit = s0 + x + step - lo <= end + 1e-6;
                x += step;
                fit
            })
            .count()
    };
    let (mut steps, mut s0, mut hung) = arrange(&sg);
    // Where the text that fits ends: before the first glyph that doesn't, which threaded text
    // flows on from, as from the last line of area type.
    let mut fits = text_len;
    // Right-to-left text that overflows keeps the start of its text, not what comes first in visual
    // order: the longest start, in reading order, whose glyphs fit in visual order is laid out.
    if shaped.iter().any(|(_, bidi, _)| bidi.is_some()) && fitting(&steps, s0, hung) < sg.len() {
        let mut starts: Vec<usize> = shaped.iter().flat_map(|(_, _, glyphs)| glyphs.iter().map(|g| g.byte)).collect();
        starts.dedup();
        // The text before `starts[fit]` fits; before `starts[over]` (the whole text at the end) it
        // doesn't.
        let (mut fit, mut over) = (0, starts.len());
        while over - fit > 1 {
            let mid = (fit + over) / 2;
            let sg = visual(starts.get(mid).copied().unwrap_or(text_len));
            let (steps, s0, hung) = arrange(&sg);
            if fitting(&steps, s0, hung) == sg.len() {
                fit = mid;
            } else {
                over = mid;
            }
        }
        cx.out.overflow = true;
        fits = starts.get(fit).copied().unwrap_or(text_len);
        sg = visual(fits);
        (steps, s0, hung) = arrange(&sg);
    }
    let fit = fitting(&steps, s0, hung);
    if fit < sg.len() {
        cx.out.overflow = true;
        fits = sg.get(fit..).and_then(|rest| rest.iter().map(|g| g.byte).min()).unwrap_or(text_len);
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
    for (g, &step) in sg.iter().zip(&steps).take(fit) {
        let s = s0 + x;
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
        // A list marker's glyphs (the only glyphs here with no length) are in a row at the byte of
        // their paragraph's start.
        if g.len == 0 {
            let at = cx.out.glyphs.len();
            let starts_paragraph = |b: usize| paras.iter().position(|p| p.start == b);
            match cx.out.markers.last_mut() {
                Some(m) if m.glyphs.end == at && cx.out.glyphs.get(m.glyphs.start).is_some_and(|h| h.byte == g.byte) => m.glyphs.end = at + 1,
                _ => {
                    if let Some(text) = starts_paragraph(g.byte).and_then(|pi| markers.get(pi)).and_then(Option::as_ref) {
                        cx.out.markers.push(ListMarker { glyphs: at..at + 1, tab: at, text: text.clone() });
                    }
                }
            }
            if let Some(m) = cx.out.markers.last_mut().filter(|m| m.glyphs.end == at + 1 && g.ch == '\t') {
                m.tab = at;
            }
        }
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
        first_baseline: 0.0,
        start: 0,
        end: fits,
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

/// [`para_bidi`] for a paragraph whose characters in `set` (byte ranges of `text`, in order and not
/// overlapping, from [`Ctx::char_directions`]) have a Character Direction: each range is resolved
/// as a Unicode bidi embedding, as if an LRE or RLE stood before it and a PDF after it (UAX #9 X2,
/// X3, X7). The levels, classes and paragraphs come back for the bytes of `text`. The paragraph's own
/// direction is unchanged: the first strong character is found inside embeddings too (P2).
pub(crate) fn para_bidi_with<'a>(text: &'a str, direction: Option<ParaDirection>, set: &[(Range<usize>, ParaDirection)]) -> Option<BidiInfo<'a>> {
    let plain = para_bidi(text, direction);
    if set.is_empty() || (plain.is_none() && set.iter().all(|(_, d)| *d == ParaDirection::LeftToRight)) {
        // A left-to-right embedding in left-to-right text takes an even level: its order is the same.
        return plain;
    }
    // The text with the embedding controls, and where each byte of `text` is in it.
    let mut marked = String::with_capacity(text.len() + set.len() * 6);
    let mut at_byte: Vec<usize> = Vec::with_capacity(text.len());
    let mut copy = |marked: &mut String, from: usize, to: usize| {
        if let Some(s) = text.get(from..to) {
            at_byte.extend(marked.len()..marked.len() + s.len());
            marked.push_str(s);
        }
    };
    let mut done = 0;
    for (r, d) in set {
        let (a, b) = (r.start.max(done).min(text.len()), r.end.min(text.len()));
        if a >= b {
            continue;
        }
        copy(&mut marked, done, a);
        marked.push(if *d == ParaDirection::RightToLeft { '\u{202B}' } else { '\u{202A}' });
        copy(&mut marked, a, b);
        marked.push('\u{202C}');
        done = b;
    }
    copy(&mut marked, done, text.len());
    let level = direction.map(|d| if d == ParaDirection::RightToLeft { Level::rtl() } else { Level::ltr() });
    let info = BidiInfo::new(&marked, level);
    let back = |i: usize| at_byte.partition_point(|&m| m < i);
    let paragraphs =
        info.paragraphs.iter().map(|p| unicode_bidi::ParagraphInfo { range: back(p.range.start)..back(p.range.end), level: p.level }).collect();
    Some(BidiInfo {
        text,
        original_classes: at_byte.iter().filter_map(|&i| info.original_classes.get(i).copied()).collect(),
        levels: at_byte.iter().filter_map(|&i| info.levels.get(i).copied()).collect(),
        paragraphs,
    })
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
    // A list marker (length 0) is a cluster of its own.
    let mut clusters: Vec<Range<usize>> = Vec::new();
    for (i, g) in glyphs.iter().enumerate() {
        if i > 0 && glyphs.get(i - 1).is_some_and(|p| p.byte == g.byte && (p.len == 0) == (g.len == 0)) {
            if let Some(c) = clusters.last_mut() {
                c.end = i + 1;
            }
        } else {
            clusters.push(i..i + 1);
        }
    }
    // A list marker is at the paragraph's level, and at a right-to-left one its tab is drawn
    // before (left of) its characters.
    let cluster_levels: Vec<_> = clusters
        .iter()
        .filter_map(|c| glyphs.get(c.start))
        .map(|g| if g.len == 0 { g.level } else { levels.get(g.byte.saturating_sub(paragraph_start)).copied().unwrap_or(g.level) })
        .collect();
    let reversed = |i: usize| glyphs.get(clusters.get(i).map_or(0, |c| c.start)).is_some_and(|g| g.len == 0 && g.level.is_rtl());
    unicode_bidi::BidiInfo::reorder_visual(&cluster_levels)
        .into_iter()
        .filter_map(|i| clusters.get(i).map(|c| (c.clone(), reversed(i))))
        .flat_map(|(c, rev)| {
            let mut v: Vec<usize> = c.collect();
            if rev {
                v.reverse();
            }
            v
        })
        .collect()
}

/// Ruby (furigana): lays each reading out over its base text in line space, grows the line's
/// ascent/descent so the ruby fits, and moves the lines after it on in the block direction.
///
/// Positions are measured from the line's baseline. Vertical type uses the same geometry — ruby
/// "above" the baseline is right of the column once the lines are turned — with each reading
/// character set upright along the column (a quarter turn that the final vertical turn undoes).
/// A reading over a line break is split between the lines, by the base characters' share of it.
/// A reading wider than its base hangs over the neighbours (`overhang`), or is squeezed to the
/// base's width. Type on a path is left out (it has no above/below).
fn ruby_layout(cx: &mut Ctx<'_>, t: &TextObject) {
    use std::collections::BTreeMap;

    let db = cx.db;
    let text_len = cx.text.len();
    let n_lines = cx.out.lines.len();
    if n_lines == 0 || cx.out.glyphs.is_empty() {
        return;
    }
    let base_baselines: Vec<f64> = cx.out.lines.iter().map(|l| l.baseline).collect();
    let old_ascent: Vec<f64> = cx.out.lines.iter().map(|l| l.ascent).collect();
    let old_descent: Vec<f64> = cx.out.lines.iter().map(|l| l.descent).collect();
    // One piece of base text on one line, with the part of the reading that belongs to it.
    struct Piece {
        line: usize,
        x0: f64,
        x1: f64,
        run: usize,
        byte: usize,
        reading: String,
        style: CharStyle,
        position: RubyPosition,
        align: RubyAlign,
        base_size: f64,
        overhang: bool,
    }
    let mut pieces: Vec<Piece> = Vec::new();
    // The glyphs by their byte, to find a span's without a scan of them all for each span.
    let byte_of = |i: usize| cx.out.glyphs.get(i).map_or(0, |g| g.byte);
    let mut by_byte: Vec<usize> = (0..cx.out.glyphs.len()).collect();
    by_byte.sort_by_key(|&i| byte_of(i));
    let longest = cx.out.glyphs.iter().map(|g| g.len.max(1)).max().unwrap_or(1);
    for span in &t.ruby {
        let (start, end) = (span.start.min(text_len), span.end.min(text_len));
        if start >= end || span.text.trim().is_empty() {
            continue;
        }
        // The base glyphs of the span, by line; a glyph's cluster is its byte range (`len`, or
        // one byte for generated glyphs). A list marker is not part of the base.
        let mut by_line: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        let hi = by_byte.partition_point(|&i| byte_of(i) < end);
        let lo = by_byte.partition_point(|&i| byte_of(i).saturating_add(longest) <= start).min(hi);
        let mut hits: Vec<usize> = by_byte.get(lo..hi).unwrap_or_default().to_vec();
        // In glyph order, as the scan found them.
        hits.sort_unstable();
        for i in hits {
            let Some(g) = cx.out.glyphs.get(i) else { continue };
            let b = g.byte.saturating_add(g.len.max(1));
            if g.byte < end && b > start && cx.out.marker_of(i).is_none() {
                by_line.entry(g.line).or_default().push(i);
            }
        }
        let total: usize = by_line.values().map(Vec::len).sum();
        if total == 0 {
            continue;
        }
        let reading: Vec<char> = span.text.trim().chars().collect();
        let ratio = span.safe_size_percent() / 100.0;
        // Split the reading by each line's share of the base glyphs: cumulative fractions keep
        // every character once and in order.
        let mut taken = 0;
        let mut done = 0;
        let lines: Vec<usize> = by_line.keys().copied().collect();
        for line in &lines {
            let count = by_line.get(line).map_or(0, Vec::len);
            done += count;
            let upto = if reading.is_empty() { 0 } else { (reading.len() * done / total).min(reading.len()) };
            let part: String = reading.get(taken.min(reading.len())..upto.min(reading.len())).unwrap_or_default().iter().collect();
            taken = upto;
            if part.is_empty() {
                continue;
            }
            let glyphs = by_line.get(line).map(Vec::as_slice).unwrap_or_default();
            let mut x0 = f64::INFINITY;
            let mut x1 = f64::NEG_INFINITY;
            let mut base_size: f64 = 0.0;
            let mut run = 0;
            let mut byte = start;
            for (j, &gi) in glyphs.iter().enumerate() {
                let Some(g) = cx.out.glyphs.get(gi) else { continue };
                x0 = x0.min(g.origin.x);
                x1 = x1.max(g.origin.x + g.advance);
                base_size = base_size.max(cx.style_at(g.byte).size);
                if j == 0 {
                    run = g.run;
                    byte = g.byte;
                }
            }
            if !(x0.is_finite() && x1.is_finite()) || base_size <= 0.0 {
                continue;
            }
            let mut style = cx.style_at(byte).clone();
            style.size = base_size * ratio;
            style.leading = None;
            style.baseline_shift = 0.0;
            style.tracking = 0.0;
            style.kerning = None;
            pieces.push(Piece {
                line: *line,
                x0,
                x1,
                run,
                byte,
                reading: part,
                style,
                position: span.position,
                align: span.align,
                base_size,
                overhang: span.overhang,
            });
        }
    }
    // Phase A: shape each reading, place it relative to the (unshifted) baseline, and note how
    // far above/below the baseline its ink reaches.
    let mut placed: Vec<(usize, Vec<PositionedGlyph>, f64, f64)> = Vec::new(); // (line, glyphs, ascent, descent)
    for piece in &pieces {
        let Some(baseline) = base_baselines.get(piece.line).copied() else { continue };
        let mini = TextObject::point(Point::ZERO, &piece.reading, piece.style.clone());
        let lay = layout(db, &mini);
        let Some(first_line) = lay.lines.first() else { continue };
        let mini_base = first_line.baseline;
        let mini_asc = first_line.ascent;
        let mini_desc = first_line.descent;
        let base_w = (piece.x1 - piece.x0).max(0.0);
        // Above: the reading's bottom sits on the base's em box top. Below: its top sits on the
        // base's em box bottom (the line's descent below the baseline).
        let ruby_y = if piece.position == RubyPosition::Above {
            baseline - piece.base_size - mini_desc
        } else {
            baseline + old_descent.get(piece.line).copied().unwrap_or(0.0) + mini_asc
        };
        let dest = |width: f64| match piece.align {
            RubyAlign::Center => piece.x0 + (base_w - width) * 0.5,
            RubyAlign::Start => piece.x0,
            RubyAlign::End => piece.x1 - width,
        };
        let mut glyphs: Vec<PositionedGlyph> = Vec::new();
        if cx.vertical {
            // Each reading character in its own cell along the column, upright once the lines turn.
            let cell = (mini_asc + mini_desc).max(piece.style.size * 0.5);
            let mut clusters: Vec<Vec<&PositionedGlyph>> = Vec::new();
            for g in &lay.glyphs {
                match clusters.last_mut() {
                    Some(c) if c.last().is_some_and(|p| p.byte == g.byte && p.len == g.len) => c.push(g),
                    _ => clusters.push(vec![g]),
                }
            }
            let mut ruby_w = clusters.len() as f64 * cell;
            let mut squeeze = 1.0;
            if !piece.overhang && ruby_w > base_w + EPS && ruby_w > EPS {
                squeeze = base_w / ruby_w;
                ruby_w = base_w;
            }
            let x = dest(ruby_w);
            for (ci, cluster) in clusters.iter().enumerate() {
                let anchor = Point::new(x + (ci as f64 + 0.5) * cell, ruby_y);
                let mut box_: Option<Rect> = None;
                for g in cluster {
                    if g.outline.elements().is_empty() {
                        continue;
                    }
                    let b = g.outline.bounding_box();
                    box_ = Some(box_.map_or(b, |a| a.union(b)));
                }
                let centre = box_.map_or(anchor, |b| Point::new((b.x0 + b.x1) * 0.5, (b.y0 + b.y1) * 0.5));
                let mut m =
                    Affine::rotate_about(-std::f64::consts::FRAC_PI_2, anchor) * Affine::translate((anchor.x - centre.x, anchor.y - centre.y));
                if squeeze != 1.0 {
                    m = Affine::translate((x, 0.0)) * Affine::scale_non_uniform(squeeze, 1.0) * Affine::translate((-x, 0.0)) * m;
                }
                for g in cluster {
                    glyphs.push(ruby_glyph(g, m, piece.run, piece.byte, piece.line));
                }
            }
        } else {
            // The reading set along the line: the line's extent, centred or aligned to the base.
            let mut ruby_w = (first_line.x1 - first_line.x0).max(0.0);
            let mut squeeze = 1.0;
            if !piece.overhang && ruby_w > base_w + EPS && ruby_w > EPS {
                squeeze = base_w / ruby_w;
                ruby_w = base_w;
            }
            let x = dest(ruby_w);
            let mut m = Affine::translate((x - first_line.x0, ruby_y - mini_base));
            if squeeze != 1.0 {
                m = Affine::translate((x, 0.0)) * Affine::scale_non_uniform(squeeze, 1.0) * Affine::translate((-x, 0.0)) * m;
            }
            for g in &lay.glyphs {
                glyphs.push(ruby_glyph(g, m, piece.run, piece.byte, piece.line));
            }
        }
        // How far the reading reaches above and below the base line's baseline (its ink).
        let mut ascent: f64 = 0.0;
        let mut descent: f64 = 0.0;
        for g in &glyphs {
            if g.outline.elements().is_empty() {
                continue;
            }
            let b = g.outline.bounding_box();
            ascent = ascent.max(baseline - b.y0);
            descent = descent.max(b.y1 - baseline);
        }
        if !glyphs.is_empty() {
            placed.push((piece.line, glyphs, ascent, descent));
        }
    }
    if placed.is_empty() {
        return;
    }
    // Phase B: grow each line's box to fit its ruby, then move the lines on after it by what they
    // need in the block direction (the first line too for area type, so the ruby stays inside the
    // frame; point type's first baseline is its anchor and stays).
    let mut growth = vec![0.0_f64; n_lines];
    for (line, _, ascent, descent) in &placed {
        if let (Some(a), Some(d)) = (old_ascent.get(*line), old_descent.get(*line)) {
            let grown = (a.max(*ascent) + d.max(*descent)) - (a + d);
            if grown > 0.0 {
                let g = growth.get_mut(*line);
                if let Some(slot) = g {
                    *slot = slot.max(grown);
                }
            }
        }
    }
    // A line's ruby grows above it: the line moves on in the block direction by its own growth,
    // so the line before it keeps its room (for area type the first line moves too, keeping the
    // ruby inside the frame; point type's first baseline is its anchor and stays).
    let area = matches!(t.kind, TextKind::Area { .. });
    let mut shift = vec![0.0_f64; n_lines];
    let mut cum = 0.0;
    for i in 0..n_lines {
        if area || i > 0 {
            cum += growth.get(i).copied().unwrap_or(0.0);
            if let Some(s) = shift.get_mut(i) {
                *s = cum;
            }
        }
    }
    // Grow the lines' boxes to fit their ruby, then move them on in the block direction.
    for (line, _, ascent, descent) in &placed {
        if let Some(l) = cx.out.lines.get_mut(*line) {
            l.ascent = l.ascent.max(*ascent);
            l.descent = l.descent.max(*descent);
        }
    }
    for (i, line) in cx.out.lines.iter_mut().enumerate() {
        line.baseline += shift.get(i).copied().unwrap_or(0.0);
    }
    for g in &mut cx.out.glyphs {
        let s = shift.get(g.line).copied().unwrap_or(0.0);
        if s != 0.0 {
            g.origin.y += s;
            g.outline.apply_affine(Affine::translate((0.0, s)));
            g.xf = Affine::translate((0.0, s)) * g.xf;
        }
    }
    for (line, glyphs, _, _) in placed {
        let s = shift.get(line).copied().unwrap_or(0.0);
        for mut g in glyphs {
            if s != 0.0 {
                g.origin.y += s;
                g.outline.apply_affine(Affine::translate((0.0, s)));
            }
            g.line = line;
            cx.out.ruby.push(g);
        }
    }
}

/// A ruby glyph from one of a reading's laid-out glyphs, moved by `m` into the base text's line
/// space. It is not a character of the text: `len` 0, the base text's `run` for fill and stroke.
fn ruby_glyph(g: &PositionedGlyph, m: Affine, run: usize, byte: usize, line: usize) -> PositionedGlyph {
    PositionedGlyph {
        outline: m * g.outline.clone(),
        run,
        byte,
        origin: m * g.origin,
        advance: 0.0,
        len: 0,
        angle: 0.0,
        line,
        font_id: g.font_id,
        gid: g.gid,
        xf: Affine::IDENTITY,
        rtl: false,
        scale: 1.0,
        // Ruby draws outlines only.
        image: None,
        colour: None,
    }
}
