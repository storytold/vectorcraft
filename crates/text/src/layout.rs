//! Line breaking and glyph placement for point, area and on-path type.

use std::ops::Range;

use kurbo::{Affine, BezPath, ParamCurve, PathEl, Point, Rect, Shape, Vec2};
use unicode_bidi::{BidiInfo, Level};
use vectorcraft_doc::text::Kashida;
use vectorcraft_doc::{
    Burasagari, CharStyle, HyphenationOptions, InlineArt, Justify, Kinsoku, LeadingModel, Mojikumi, ParaDirection, ParaStyle, PathAlign, PathEffect,
    RubyAlign, RubyPosition, SingleWordJustify, TextKind, TextObject,
};
use vectorcraft_geom::{ArcPath, PathData};

use crate::GlyphBend;
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

mod bidi;
mod breaking;
mod driver;
mod flow;
mod on_path;
mod region;
mod ruby;
mod vertical;

pub(crate) use bidi::{is_rtl, para_bidi};
use breaking::tab_advance;
#[cfg(test)]
use breaking::{break_line, candidates};
pub use driver::{decorations, fit_first_line, layout, layout_with, scaled};
use vertical::{stands_upright, tate_chu_yoko, upright_cell, upright_centre, whole_character_widths};

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
            bend: None,
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

#[cfg(test)]
mod wrapping_tests;
