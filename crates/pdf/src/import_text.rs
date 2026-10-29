//! Text as editable point type: glyphs drawn one after another on the same baseline make one
//! point text object of their Unicode text, a run per font, size and paint (a gap wider than a
//! fifth of the smaller size beside it reads as a space; a glyph the file raised or lowered off the
//! baseline, text rise, is baseline shift). Each glyph stays where the file drew it: the kerning
//! and tracking the type takes put it there (#722). Fonts are named from the file's base font name (its
//! subset prefix dropped, the family matched against the fonts available); a font that isn't
//! available keeps its name, and the text shows in the fallback font until it is.
//!
//! Glyphs each turned a little further along a curve (type set on a path: apps write each glyph
//! with its own placement) make one type-on-a-path object, its path through the glyphs' baseline.

use std::collections::{HashMap, HashSet};

use kurbo::{Affine, BezPath, Point, Rect, Vec2};
use vectorcraft_color::Paint;
use vectorcraft_doc::{CharStyle, Node, ParaDirection, TextKind, TextObject, TextRun};
use vectorcraft_text::TextLayout;

/// One glyph's placement: its baseline origin, advance direction, size and horizontal scale.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placement {
    pub origin: Point,
    /// Unit vector along the baseline.
    pub dir: Vec2,
    /// Font size in points (the em's height).
    pub size: f64,
    /// Width of the em ÷ its height, in percent.
    pub h_scale: f64,
    /// How far the em leans along the baseline per unit of its height (a text matrix with a
    /// shear: faux italic, or type slanted as a whole); 0 when it stands square on the baseline.
    pub slant: f64,
}

impl Placement {
    /// Where a glyph drawn with `m` (glyph space, 1000 units per em, y up → document) sits;
    /// `None` when it is mirrored, degenerate or not finite (those keep their outlines).
    pub fn of(m: Affine) -> Option<Self> {
        let origin = m * Point::ORIGIN;
        let ex = m * Point::new(1000.0, 0.0) - origin;
        let ey = m * Point::new(0.0, 1000.0) - origin;
        let w = ex.hypot();
        // Upright in y-down document space: x to the right of up.
        let upright = ex.cross(ey) < 0.0;
        if !(w.is_finite() && w > 0.01) {
            return None;
        }
        let dir = ex / w;
        // The em's height is across the baseline; what's left of `ey` along it is a slant.
        let up = Vec2::new(dir.y, -dir.x);
        let size = ey.dot(up);
        let slant = ey.dot(dir) / size;
        (origin.is_finite() && size.is_finite() && slant.is_finite() && size > 0.01 && size < 1e5 && upright).then(|| Self {
            origin,
            dir,
            size,
            h_scale: w / size * 100.0,
            slant,
        })
    }

    fn up(&self) -> Vec2 {
        Vec2::new(self.dir.y, -self.dir.x)
    }
}

/// A glyph set vertically (WMode 1: no horizontal advance) and the top centre of its em box: a
/// column of them is vertical type.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Upright {
    pub top: Point,
}

/// What a run of type is drawn with: its font (cache key, family and style), size, horizontal
/// scale, fill and stroke (paint and width).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Look {
    pub font: u128,
    pub family: String,
    pub style: String,
    /// The installed version of the font when it isn't the one family and style resolve to.
    pub version: Option<String>,
    pub size: f64,
    pub h_scale: f64,
    pub fill: Option<Paint>,
    pub stroke: Option<(Paint, f64)>,
    /// The fill, the stroke overprints.
    pub overprint_fill: bool,
    pub overprint_stroke: bool,
}

impl Look {
    /// Can a glyph drawn with `other` join a run drawn with this? A fill joins a run whose
    /// glyphs were also stroked over (fill and stroke rendering draws each glyph twice); invisible
    /// text (neither fill nor stroke) joins only invisible text.
    fn takes(&self, other: &Look) -> bool {
        self.font == other.font
            && (self.size - other.size).abs() <= self.size * 0.01
            && (self.h_scale - other.h_scale).abs() < 0.5
            && self.fill == other.fill
            && self.overprint_fill == other.overprint_fill
            && ((other.stroke.is_none() && other.fill.is_some()) || (self.stroke == other.stroke && self.overprint_stroke == other.overprint_stroke))
    }

    fn style(&self) -> CharStyle {
        let (stroke, stroke_width) = self.stroke.clone().unwrap_or((Paint::None, 0.0));
        CharStyle {
            font_family: self.family.clone(),
            font_style: self.style.clone(),
            font_version: self.version.clone(),
            size: round(self.size),
            h_scale: round(self.h_scale),
            fill: self.fill.clone().unwrap_or(Paint::None),
            stroke,
            stroke_width,
            overprint_fill: self.overprint_fill,
            overprint_stroke: self.overprint_stroke,
            ..CharStyle::default()
        }
    }
}

/// A gap between adjacent glyphs on a line.
#[derive(Clone, Copy, Debug)]
struct Gap {
    /// Byte offset in the line's text of the glyph after the gap.
    at: usize,
    /// From the end of the glyph before to the start of the next (points).
    width: f64,
    /// The smaller of the two glyphs' sizes (points): a word gap beside a much larger glyph is
    /// narrow for the large size (#3623).
    em: f64,
    /// Along the line, not turned along a curve.
    on_line: bool,
}

/// The spacing a line's letters share (letter-spaced type), in ems: the gap a third of the way up
/// from the narrowest, of three or more along the line. Never below zero, and none at all past
/// half an em: glyphs set wider apart than that are words or columns, not tracked letters.
fn common_gap(gaps: &[Gap]) -> f64 {
    let mut widths: Vec<f64> = gaps.iter().filter(|g| g.on_line).map(|g| g.width / g.em).filter(|w| w.is_finite()).collect();
    if widths.len() < 3 {
        return 0.0;
    }
    let pivot = widths.len() / 3;
    let (_, common, _) = widths.select_nth_unstable_by(pivot, f64::total_cmp);
    if *common > 0.5 { 0.0 } else { common.max(0.0) }
}

/// A line of type being gathered: runs of glyphs on one baseline.
pub(crate) struct TextLine {
    /// The first glyph's placement.
    at: Placement,
    opacity: f32,
    /// Where the next glyph would start without spacing.
    next: Point,
    /// The last glyph's origin.
    last: Point,
    /// Where the last glyph that isn't a space ends.
    visible: Point,
    /// The last glyph's baseline direction.
    last_dir: Vec2,
    /// The last glyph's size (points).
    last_size: f64,
    /// Each glyph's baseline origin, then where the last one ends.
    baseline: Vec<Point>,
    /// Runs of glyphs drawn alike: their look, how far the file raised them off the line's
    /// baseline (points, up; text rise, `Ts`), their text.
    runs: Vec<(Look, f64, String)>,
    /// The bytes of text the horizontal glyphs added to `runs` so far (counted as they come: a
    /// sum over the runs for each glyph is quadratic when every glyph is a run).
    text_bytes: usize,
    /// Where the file put each character of the runs' text, in order: the first character of each
    /// glyph has its glyph's [`Slot`]; a space read from a gap and the other characters of a
    /// glyph of several have none.
    slots: Vec<Option<Slot>>,
    /// The first glyph was set vertically ([`Upright`]).
    upright: Option<Upright>,
    /// The glyphs make a column (vertical type), each below the last.
    column: bool,
    /// Each glyph's ink box as drawn (document space), in drawing order; glyphs without ink are
    /// left out.
    pub ink: Vec<Rect>,
    /// The gaps between adjacent glyphs that may read as spaces ([`finish`](Self::finish)
    /// decides).
    gaps: Vec<Gap>,
    /// The spaces the file draws between two glyphs that aren't spaces: their byte offset in the
    /// text before the spaces read from gaps go in, and the gaps on both sides of it plus its
    /// advance (points).
    drawn_spaces: Vec<(usize, f64)>,
    /// A space the file drew after a glyph that isn't one, until the next glyph comes: its byte
    /// offset, and the gap before it plus its advance (points).
    drawn_space: Option<(usize, f64)>,
    /// The strokes of its glyphs' outlines drawn between them, one per glyph with ink so far
    /// (written right after the line).
    pub held: Vec<Node>,
}

/// Where the file put a glyph: the glyph's index in the line, and its advance in the file's font
/// (points) when it shows one character (`None` for a ligature's several).
#[derive(Clone, Copy, Debug)]
struct Slot {
    glyph: usize,
    advance: Option<f64>,
}

/// What a finished line of type tells beyond its text object (see [`crate::import_lines`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct LineFacts {
    /// [`TextLine::ink`].
    pub ink: Vec<Rect>,
    /// Horizontal point type: it ended in a space (as a line an editor wrapped in a frame does).
    pub wraps: bool,
    /// Horizontal point type: how long the file set it, from its first glyph's origin to the end
    /// of its last glyph that isn't a space (points, along the baseline).
    pub length: f64,
}

/// The most a glyph on a curve turns from the one before it (about 20°).
const CURVE_TURN_COS: f64 = 0.94;

impl TextLine {
    pub fn new(at: Placement, opacity: f32) -> Self {
        Self {
            at,
            opacity,
            next: at.origin,
            last: at.origin,
            visible: at.origin,
            last_dir: at.dir,
            last_size: at.size,
            baseline: vec![],
            runs: vec![],
            text_bytes: 0,
            slots: vec![],
            upright: None,
            column: false,
            ink: vec![],
            gaps: vec![],
            drawn_spaces: vec![],
            drawn_space: None,
            held: vec![],
        }
    }

    /// The paint the last glyph is filled with.
    pub fn last_fill(&self) -> Option<&Paint> {
        self.runs.last()?.0.fill.as_ref()
    }

    /// Add glyph `text` set vertically (`upright`) if it continues this column (or makes this
    /// line's single glyph a column): below the last one, in the same column. `false`: it starts
    /// another line.
    pub fn push_upright(&mut self, look: &Look, at: Placement, opacity: f32, upright: Upright, text: &str) -> bool {
        let down = -self.at.up();
        let size = self.at.size.max(at.size);
        let Some((last, _, run)) = self.runs.last_mut() else {
            self.runs.push((look.clone(), 0.0, text.to_string()));
            (self.upright, self.next, self.last) = (Some(upright), at.origin + down * at.size, at.origin);
            return true;
        };
        let across = (at.origin - self.at.origin).dot(self.at.dir);
        let step = (at.origin - self.next).dot(down);
        // Only glyphs set vertically (WMode 1) come here, so a second one below the first, one em
        // on or letter-spaced up to another em, makes a column.
        let first_step = (at.origin - self.last).dot(down);
        let starts_column = !self.column && self.upright.is_some() && first_step > size * 0.95 && first_step < size * 2.0;
        let in_column = (self.column || starts_column)
            && opacity == self.opacity
            && at.dir.dot(self.at.dir) > 0.9995
            && across.abs() < size * 0.6
            && step > -size * 0.3
            && step < size * 1.2;
        if !in_column {
            return false;
        }
        self.column = true;
        // Letter-spaced Japanese is common: a gap reads as a space only when a whole em is left
        // out (the tracking set in `finish` keeps narrower gaps).
        if step > size * 0.9 && !run.ends_with(is_space) && !text.starts_with(is_space) {
            run.push(' ');
        }
        if last.takes(look) {
            run.push_str(text);
        } else {
            self.runs.push((look.clone(), 0.0, text.to_string()));
        }
        self.next = at.origin + down * at.size;
        self.last = at.origin;
        true
    }

    /// Add glyph `text` drawn with `look` at `at` (advancing `advance` points) at `opacity` if
    /// it continues this line; `false`: it starts another.
    pub fn push(&mut self, look: &Look, at: Placement, opacity: f32, advance: f64, text: &str) -> bool {
        // A vertical line (even a single upright glyph) takes no horizontal glyphs.
        if self.column || self.upright.is_some() {
            return false;
        }
        let bytes = self.text_bytes;
        let glyph = self.baseline.len();
        let curved = self.curved();
        let mut shift = 0.0;
        if let Some((last, last_shift, run)) = self.runs.last_mut() {
            let size = self.at.size.max(at.size);
            let gap = (at.origin - self.next).dot(self.last_dir);
            // How far the glyph sits above the line's baseline. The file raises a superscript or
            // lowers a subscript (text rise, `Ts`) with the glyphs turned exactly as the rest of
            // the line (`level`): it stays on the line, shifted off its baseline. Up to half an em
            // when it follows on from the glyph before (no wider gap than a space reads from):
            // further apart, it is type of its own (the next cell of a table's row, its baseline a
            // little higher).
            let above = (at.origin - self.at.origin).dot(self.at.up());
            let level = at.dir.dot(self.at.dir) > 1.0 - 1e-9 && !curved;
            let on_line = opacity == self.opacity
                && at.dir.dot(self.at.dir) > 0.9995
                && (at.slant - self.at.slant).abs() < 1e-3
                && (above.abs() < size * 0.15 || (level && above.abs() < size * 0.5 && gap <= size * 0.2))
                // Proportional widths close CJK marks up by up to half an em.
                && gap > -size * 0.6
                && gap < size * 3.0;
            // Within a hair of the baseline (rounding in the file) is on it; a glyph turned a
            // little (type on a path) sits on the curve, not shifted off it.
            if on_line && level && above.abs() > size * 0.01 {
                shift = above;
            }
            // Or on a curve: turned a little from the glyph before it, and starting about where
            // that one ends.
            let on_curve = !on_line
                && opacity == self.opacity
                && (at.slant - self.at.slant).abs() < 1e-3
                && at.dir.dot(self.last_dir) > CURVE_TURN_COS
                && at.dir.dot(self.last_dir) < 0.99999
                && (at.origin - self.next).hypot() < size * 0.6;
            if !on_line && !on_curve {
                return false;
            }
            if !run.ends_with(is_space) && !text.starts_with(is_space) {
                self.gaps.push(Gap { at: bytes, width: gap, em: self.last_size.min(at.size), on_line });
            }
            // A space the file draws between two glyphs spans the gaps on both sides of it.
            let drawn = self.drawn_space.take();
            if on_line {
                let mut chars = text.chars();
                if chars.next().is_some_and(is_space) && chars.next().is_none() && !run.ends_with(is_space) {
                    self.drawn_space = Some((bytes, gap + advance));
                } else if let Some((b, before)) = drawn.filter(|_| !text.starts_with(is_space)) {
                    self.drawn_spaces.push((b, before + gap));
                }
            }
            if last.takes(look) && (*last_shift - shift).abs() < 1e-3 {
                run.push_str(text);
            } else {
                self.runs.push((look.clone(), shift, text.to_string()));
            }
        } else {
            self.runs.push((look.clone(), 0.0, text.to_string()));
        }
        self.text_bytes = self.text_bytes.saturating_add(text.len());
        let one = text.chars().count() == 1;
        self.slots.push(Some(Slot { glyph, advance: one.then_some(advance) }));
        self.slots.extend(text.chars().skip(1).map(|_| None));
        self.next = at.origin + at.dir * advance;
        if !text.trim().is_empty() {
            self.visible = self.next;
        }
        self.last = at.origin;
        self.last_dir = at.dir;
        self.last_size = at.size;
        self.baseline.push(at.origin);
        true
    }

    /// Has the baseline turned (more than about 3° from the first glyph to the last)?
    fn curved(&self) -> bool {
        self.last_dir.dot(self.at.dir) < 0.9986
    }

    /// A smooth path through the glyphs' baseline origins and the end of the last glyph, an em
    /// further (Catmull-Rom through the points, as cubic Béziers).
    fn baseline_path(&self) -> Option<BezPath> {
        let mut pts = self.baseline.clone();
        // On past the end of the last glyph by an em, so rounding in the spacing doesn't push it
        // off the end of the path (where type on a path stops).
        pts.push(self.next);
        pts.push(self.next + self.last_dir * self.at.size);
        let first = *pts.first()?;
        // Finite placements far out can still overflow once an em is added: no path then.
        if pts.len() < 3 || !pts.iter().all(|p| p.is_finite()) {
            return None;
        }
        let mut bp = BezPath::new();
        bp.move_to(first);
        for (i, seg) in pts.windows(2).enumerate() {
            let &[p1, p2] = seg else { continue };
            let p0 = *pts.get(i.wrapping_sub(1)).unwrap_or(&p1);
            let p3 = *pts.get(i + 2).unwrap_or(&p2);
            bp.curve_to(p1 + (p2 - p0) / 6.0, p2 - (p3 - p1) / 6.0, p2);
        }
        Some(bp)
    }

    /// A stroke (`stroke`: paint and width) over the glyph just drawn at `at` in `font` (fill
    /// and stroke rendering), which `overprint`s or not: the run is stroked. `false`: it isn't
    /// that glyph.
    pub fn stroke_last(&mut self, font: u128, at: Placement, stroke: (Paint, f64), overprint: bool) -> bool {
        let Some((look, ..)) = self.runs.last_mut().filter(|(l, ..)| l.font == font && (at.origin - self.last).hypot() < self.at.size * 1e-3) else {
            return false;
        };
        (look.stroke, look.overprint_stroke) = (Some(stroke), overprint);
        true
    }

    /// The point type object, its opacity and what else the line tells.
    pub fn finish(mut self) -> Option<(TextObject, f32, LineFacts)> {
        let mut facts = LineFacts { ink: std::mem::take(&mut self.ink), ..LineFacts::default() };
        // Turned along a curve: type on a path through the glyphs.
        let path = if self.curved() { self.baseline_path() } else { None };
        facts.wraps = path.is_none() && self.upright.is_none() && self.runs.last().is_some_and(|(.., t)| t.ends_with(' '));
        if let Some((.., last)) = self.runs.last_mut() {
            last.truncate(last.trim_end().len());
        }
        self.runs.retain(|(.., t)| !t.is_empty());
        if self.runs.iter().all(|(.., t)| t.trim().is_empty()) {
            return None;
        }
        // The spaces trimmed off the end have no place either.
        let chars: usize = self.runs.iter().map(|(.., t)| t.chars().count()).sum();
        self.slots.truncate(chars);
        // The baseline most of the characters sit on is the line's: the others are shifted off it
        // (a line starting with a raised footnote mark keeps the baseline of its text).
        let mut on: Vec<(f64, usize)> = vec![];
        for (_, shift, text) in &self.runs {
            let n = text.chars().count();
            match on.iter_mut().find(|(s, _)| (s - shift).abs() < 1e-3) {
                Some((_, count)) => *count += n,
                None => on.push((*shift, n)),
            }
        }
        // A tie keeps the first glyph's ("m²" sits on the m's baseline).
        let base = on.iter().fold(None::<(f64, usize)>, |best, &(s, n)| if best.is_none_or(|(_, b)| n > b) { Some((s, n)) } else { best });
        let base = base.map_or(0.0, |(s, _)| s);
        if path.is_some() {
            // Type on a path sits on its curve: no glyph is shifted off it.
            for (_, shift, _) in &mut self.runs {
                *shift = 0.0;
            }
        } else if base != 0.0 {
            // Up the slant the type leans by (a sheared text matrix), as the file raises it.
            self.at.origin += (self.at.up() + self.at.dir * self.at.slant) * base;
            for (_, shift, _) in &mut self.runs {
                *shift -= base;
            }
        }
        let mut runs = self.runs.into_iter().map(|(look, shift, text)| {
            let style = CharStyle { baseline_shift: round(shift), ..look.style() };
            TextRun { text, style, inline: None }
        });
        let first = runs.next()?;
        let mut t = TextObject::point(Point::ORIGIN, &first.text, first.style);
        t.runs.extend(runs);
        // A gap wider than a fifth of an em, past the spacing the line's letters share, reads as a
        // space (one set at exactly a fifth still counts, whatever the rounding). Letter spacing is
        // read only in text left in the file's order.
        let space = |gap: &Gap, common: f64| gap.width / gap.em - common > 0.199;
        let directed = vectorcraft_text::logical_order(&t.plain_text()).is_some();
        let mut common = if directed { 0.0 } else { common_gap(&self.gaps) };
        // Evenly spaced glyphs alone could be letters set apart or words of one letter each: the
        // shared spacing is letter spacing only on a line that also shows where its words break,
        // with spaces it draws or with gaps wider than the rest.
        if self.drawn_spaces.is_empty() && !self.gaps.iter().any(|gap| space(gap, common)) {
            common = 0.0;
        }
        let mut spaces = vec![];
        for gap in &self.gaps {
            if !space(gap, common) {
                continue;
            }
            // After the earlier spaces, at the end of the run before it (the run it follows).
            let at = gap.at + spaces.len();
            let mut offset = 0;
            let Some(i) = t.runs.iter().position(|run| {
                offset += run.text.len();
                at <= offset
            }) else {
                break;
            };
            // The characters before it, in the runs before and in its own.
            let earlier: usize = t.runs.iter().take(i).map(|r| r.text.chars().count()).sum();
            let Some(run) = t.runs.get_mut(i) else { break };
            let at = at - (offset - run.text.len());
            let Some(head) = run.text.get(..at) else { continue };
            // A space read from a gap has no slot of its own.
            let char_at = earlier + head.chars().count();
            if char_at <= self.slots.len() {
                self.slots.insert(char_at, None);
            }
            run.text.insert(at, ' ');
            spaces.push((gap.at, gap.width, gap.on_line));
        }
        // Each by its offset in the text with the spaces in.
        let mut gaps: Vec<(usize, f64)> = spaces.iter().enumerate().filter(|(_, s)| s.2).map(|(i, s)| (s.0 + i, s.1)).collect();
        gaps.extend(self.drawn_spaces.iter().map(|&(b, width)| (b + spaces.iter().filter(|s| s.0 < b).count(), width)));
        gaps.sort_by_key(|&(b, _)| b);
        if self.upright.is_none() {
            to_logical(&mut t);
        }
        let angle = self.at.dir.atan2();
        // The glyphs' tops lean along the baseline (+x), up being -y in the type's own space.
        t.xf = Affine::translate(self.at.origin.to_vec2()) * Affine::rotate(angle) * Affine::skew(-self.at.slant, 0.0);
        let db = vectorcraft_text::FontDb::global();
        // Each glyph where the file put it (#722): the kerning (the font's, or none) and tracking
        // (one value for the line, or each character its own) that place them there.
        let placed = (self.upright.is_none() && t.para.direction.is_none())
            .then(|| place_each(&t, &self.slots, &self.baseline, path.as_ref(), self.at, self.at.size))
            .flatten()
            .map(|(runs, laid)| {
                t.runs = runs;
                laid.bounds
            });
        if let Some(path) = path {
            // Evenly spaced type keeps its length along the path: the path runs through the
            // glyphs' origins, one curve each, to the end of the last glyph, and tracking makes up
            // the difference from the advances.
            if placed.is_none() {
                let length: f64 = path.segments().take(self.baseline.len()).map(|seg| kurbo::ParamCurveArclen::arclen(&seg, 1e-3)).sum();
                let laid = vectorcraft_text::layout(db, &t);
                let natural: f64 = laid.glyphs.iter().map(|g| g.advance).sum();
                let chars: usize = t.runs.iter().map(|r| r.text.chars().count()).sum();
                let size = self.at.size;
                if chars > 1 && length.is_finite() && (length - natural).abs() > size * 0.01 {
                    let (tracking, spaces) = spacing(&laid, &[], length - natural, chars, size);
                    t.runs = tracked(std::mem::take(&mut t.runs), tracking, &spaces);
                }
            }
            t.kind = TextKind::OnPath { path: vectorcraft_geom::PathData::from_bezpath(&path), start: 0.0, end: None };
            t.xf = Affine::IDENTITY;
            t.cached_bounds = Some(vectorcraft_text::layout(db, &t).bounds);
            return Some((t, self.opacity, facts));
        }
        let (start, along) = match self.upright {
            // Vertical point type (a column, or a single upright glyph) is anchored at the top
            // centre of its first em box.
            Some(u) => {
                t.vertical = true;
                t.xf = Affine::translate(u.top.to_vec2()) * Affine::rotate(angle);
                (self.at.origin, -self.at.up())
            }
            _ => (self.at.origin, self.at.dir),
        };
        // Tracking that makes the line as long as in the file: the PDF placed each glyph, the
        // layout sets them by their advances. Spread over the gaps between characters (tracking
        // follows each character, the last one's past the end of the line). A horizontal line is
        // measured to its last glyph that isn't a space: the spaces after it were dropped.
        let length = (if t.vertical { self.next } else { self.visible } - start).dot(along);
        if !t.vertical {
            facts.length = length;
        }
        // Placed glyph by glyph: laid out so already.
        if let Some(bounds) = placed {
            t.cached_bounds = Some(bounds);
            return Some((t, self.opacity, facts));
        }
        let laid = vectorcraft_text::layout(db, &t);
        let natural: f64 = laid.glyphs.iter().map(|g| g.advance).sum();
        let chars: usize = t.runs.iter().map(|r| r.text.chars().count()).sum();
        let size = self.at.size;
        let mut bounds = laid.bounds;
        if chars > 1 && length.is_finite() && (length - natural).abs() > size * 0.01 {
            // The spaces read from gaps or drawn between glyphs keep their own place only in text
            // left in the file's order.
            let total: usize = t.runs.iter().map(|r| r.text.len()).sum();
            let gaps: Vec<(usize, f64)> = if t.para.direction.is_none() { gaps.into_iter().filter(|(b, _)| *b < total).collect() } else { vec![] };
            let (tracking, spaces) = spacing(&laid, &gaps, length - natural, chars, size);
            t.runs = tracked(std::mem::take(&mut t.runs), tracking, &spaces);
            // Tracked: laid out again for its bounds.
            bounds = vectorcraft_text::layout(db, &t).bounds;
        }
        t.cached_bounds = Some(bounds);
        Some((t, self.opacity, facts))
    }
}

/// Where along the line (points) layout `laid` puts each character at a byte offset of `chars`
/// (in order), and its advance: `None` for one inside another's cluster. Text in the file's order
/// (left to right).
fn pens(laid: &TextLayout, chars: &[usize]) -> Vec<Option<(f64, f64)>> {
    let mut at = vec![None; chars.len()];
    let mut pen = 0.0;
    for g in &laid.glyphs {
        if let Some(slot) = chars.binary_search(&g.byte).ok().and_then(|i| at.get_mut(i)) {
            slot.get_or_insert((pen, g.advance));
        }
        pen += g.advance;
    }
    at
}

/// `t`'s runs with kerning `kern` and each character's tracking from `own` (points, one per
/// character, in thousandths of each run's em), characters alike kept in one run.
fn restyled(t: &TextObject, kern: Option<f64>, own: &[f64]) -> Vec<TextRun> {
    let mut out: Vec<TextRun> = Vec::with_capacity(t.runs.len());
    // The run of `t` the last character came from.
    let mut from = None;
    let mut k = 0;
    for (i, r) in t.runs.iter().enumerate() {
        let size = r.style.size.max(1e-3);
        for c in r.text.chars() {
            let tracking = own.get(k).map_or(0.0, |e| (e / size * 1000.0).clamp(-1000.0, 10000.0));
            k += 1;
            // Hundredths of a thousandth of an em: no drift along a line, no noise in the panels.
            let tracking = (tracking * 100.0).round() / 100.0;
            match out.last_mut() {
                Some(last) if from == Some(i) && last.style.tracking == tracking => last.text.push(c),
                last => {
                    let style = CharStyle { tracking, kerning: kern, ..r.style.clone() };
                    match last {
                        Some(last) if last.style == style && last.inline == r.inline => last.text.push(c),
                        _ => out.push(TextRun { text: c.to_string(), style, inline: r.inline.clone() }),
                    }
                }
            }
            from = Some(i);
        }
    }
    out
}

/// The runs of `t` that put its glyphs where the file did (#722), and their layout: with the
/// font's kerning or none, whichever the file's spacing follows, and tracking that is the same
/// after every character when the file spaced them evenly, else each character's own (word
/// spacing, `TJ` adjustments, character tightening, spaces read from gaps), measured from the
/// file's widths (not the font's advances, which may be a hair off them) and kept on course where
/// the differences add up ([`on_course`]).
///
/// The file put the glyph of each character with a [`Slot`] at `baseline` (one origin per glyph),
/// measured along `path` (type on a path: one curve per glyph) or along `line`'s baseline from its
/// origin, where a character shifted off it by its baseline shift draws that shift times `line`'s
/// slant further along (the type's skew leans it so, as the file's text rise did). `None` when the
/// font laid out isn't the file's (its advances differ from the file's): spacing measured against
/// a stand-in's widths would keep the stand-in's differences once the font is installed; or when
/// the glyphs can't be placed so (the layout joins characters the file drew apart).
fn place_each(
    t: &TextObject,
    slots: &[Option<Slot>],
    baseline: &[Point],
    path: Option<&BezPath>,
    line: Placement,
    size: f64,
) -> Option<(Vec<TextRun>, TextLayout)> {
    let chars: Vec<usize> = t
        .runs
        .iter()
        .scan(0, |off, r| {
            let base = *off;
            *off += r.text.len();
            Some(r.text.char_indices().map(move |(i, _)| base + i).collect::<Vec<_>>())
        })
        .flatten()
        .collect();
    if chars.len() < 2 || chars.len() != slots.len() || t.runs.iter().any(|r| r.inline.is_some()) {
        return None;
    }
    // How far along the line the file put each glyph.
    let along: Vec<f64> = match path {
        Some(bp) => std::iter::once(0.0)
            .chain(bp.segments().scan(0.0, |d, seg| {
                *d += kurbo::ParamCurveArclen::arclen(&seg, 1e-3);
                Some(*d)
            }))
            .take(baseline.len())
            .collect(),
        None => baseline.iter().map(|p| (*p - line.origin).dot(line.dir)).collect(),
    };
    // Each character's baseline shift, which the type's slant draws `slant` times as far along.
    let shifts: Vec<f64> = t.runs.iter().flat_map(|r| r.text.chars().map(move |_| r.style.baseline_shift)).collect();
    let slant = if path.is_some() { 0.0 } else { line.slant };
    // Each character's target: where the file put it, if it put it anywhere.
    let target: Vec<Option<(f64, Option<f64>)>> =
        slots.iter().zip(&shifts).map(|(s, shift)| s.and_then(|s| Some((*along.get(s.glyph)? - slant * shift, s.advance)))).collect();
    if target.first().is_none_or(Option::is_none) || target.iter().flatten().any(|(d, _)| !d.is_finite()) {
        return None;
    }
    let db = vectorcraft_text::FontDb::global();
    // The spacing each character takes after it, laid out with `pens`: whatever puts the next one
    // the file placed where it did. Characters between two the file placed (a space read from a
    // gap, the rest of a ligature) take the line's spacing, the last of them the rest.
    let spacing = |pens: &[Option<(f64, f64)>]| -> Option<Vec<f64>> {
        // How far each character the file placed is from where the layout puts it.
        let placed: Vec<(usize, f64)> =
            target.iter().zip(pens).enumerate().filter_map(|(i, (t, p))| Some((i, t.as_ref()?.0 - p.as_ref()?.0))).collect();
        let steps: Vec<f64> = placed.windows(2).filter(|w| w[1].0 == w[0].0 + 1).map(|w| w[1].1 - w[0].1).collect();
        let line = median(steps).unwrap_or(0.0);
        let mut own = vec![line; chars.len()];
        for w in placed.windows(2) {
            let ((a, da), (b, db)) = (w[0], w[1]);
            *own.get_mut(b.checked_sub(1)?)? = db - da - line * (b - a - 1) as f64;
        }
        // After the last character the file placed, spacing changes nothing: the line's.
        let last = placed.last()?.0;
        let tail = if last > 0 { own.get(last - 1).copied().unwrap_or(line) } else { line };
        for e in own.get_mut(last..).into_iter().flatten() {
            *e = tail;
        }
        Some(own)
    };
    // Evenly spaced: one tracking value for the line. (A file rounds its widths to thousandths of
    // an em: a hair either way is even.)
    let even = |own: &[f64]| {
        let line = median(own.to_vec())?;
        own.iter().all(|e| (e - line).abs() <= size * 0.002).then(|| vec![line; own.len()])
    };
    let changes = |own: &[f64]| own.windows(2).filter(|w| (w[1] - w[0]).abs() > size * 1e-4).count();
    // The type as it is: untracked, with the font's kerning.
    let auto = vectorcraft_text::layout(db, t);
    // Each glyph's own advance (the font's, unkerned) tells whether the layout's font is the
    // file's, whether the font kerned any of the line's pairs, and where the glyphs go unkerned
    // (the same glyphs, each on its own advance: no layout needed to tell).
    let mut faces: Vec<std::sync::Arc<vectorcraft_text::FontFace>> = vec![];
    let mut unkerned_pens = vec![None; chars.len()];
    let mut pen = 0.0;
    let mut kerned_any = false;
    for g in &auto.glyphs {
        let face = match faces.iter().find(|f| f.id() == g.font_id) {
            Some(f) => f.clone(),
            None => {
                let f = db.face_by_id(g.font_id)?;
                faces.push(f.clone());
                f
            }
        };
        let style = &t.runs.get(g.run)?.style;
        let own = face.advance(g.gid) * style.size / face.units_per_em().max(1e-9) * style.h_scale / 100.0;
        kerned_any |= (own - g.advance).abs() > size * 1e-6;
        if let Some(slot) = chars.binary_search(&g.byte).ok().and_then(|i| unkerned_pens.get_mut(i)) {
            slot.get_or_insert((pen, own));
        }
        pen += own;
    }
    let same_font = unkerned_pens.iter().zip(&target).all(|(n, t)| match t {
        Some((_, Some(adv))) => n.is_some_and(|(_, a): (f64, f64)| (a - adv).abs() <= size * 0.01),
        _ => true,
    });
    if !same_font {
        return None;
    }
    // Where the glyphs go on the file's own widths: each character's pen moved by how much wider
    // the file says the characters before it are than the font's advances (a file gives widths to
    // the thousandth of an em, so a font of 2048 units an em is a hair off), where the file gives
    // a character's width and the layout sets it as a glyph of its own. Spacing measured from them
    // is the file's (its `Tc`, `Tw`, `TJ`): measured from the installed font's advances, each
    // character would also take its own hair of tracking (a run each), and keep it once a font
    // rounded otherwise is installed.
    let end = pen;
    let mut wider = 0.0;
    let mut drift = Vec::with_capacity(chars.len());
    for (i, t) in target.iter().enumerate() {
        drift.push(wider);
        let (Some(Some((start, _))), Some((_, Some(width)))) = (unkerned_pens.get(i), t) else { continue };
        let next = match unkerned_pens.get(i + 1) {
            Some(n) => n.map(|(p, _)| p),
            None => Some(end),
        };
        if let Some(next) = next {
            wider += width - (next - start);
        }
    }
    let by_file =
        |pens: &[Option<(f64, f64)>]| -> Vec<Option<(f64, f64)>> { pens.iter().zip(&drift).map(|(p, d)| p.map(|(x, a)| (x + d, a))).collect() };
    // The font's kerning when the file followed it (or there is none to follow), else none.
    let kerned_pens = pens(&auto, &chars);
    let kerned = spacing(&by_file(&kerned_pens))?;
    let (kern, own) = match even(&kerned) {
        Some(own) => (None, own),
        // Without kerning the line lays out the same.
        None if !kerned_any => (None, kerned),
        None => {
            let unkerned = spacing(&by_file(&unkerned_pens))?;
            match even(&unkerned) {
                Some(own) => (Some(0.0), own),
                None if changes(&unkerned) < changes(&kerned) => (Some(0.0), unkerned),
                None => (None, kerned),
            }
        }
    };
    let own = on_course(own, if kern.is_some() { &unkerned_pens } else { &kerned_pens }, &target, size);
    let runs = restyled(t, kern, &own);
    // Laid out so, are they where the file put them? (A ligature the layout forms where the file
    // drew two glyphs moves those after it.)
    let mut probe = t.clone();
    probe.runs = runs;
    let check = vectorcraft_text::layout(db, &probe);
    let got = pens(&check, &chars);
    let ok = got.iter().zip(&target).all(|(g, t)| match t {
        Some((d, _)) => g.is_some_and(|(p, _)| (p - d).abs() <= size * 0.02),
        None => true,
    });
    ok.then_some((probe.runs, check))
}

/// `own` (each character's spacing, points, measured from the file's widths) kept on course: laid
/// out from `pens` (where each character goes untracked), the glyphs land within a hundredth of an
/// em of `target` (where the file put them). The font's advances may each be a little off the
/// file's widths (by up to the 1% that still counts as the same font), and a line spaced evenly to
/// a hair takes the same value after every character: small differences that add up along a line.
/// Off course, each stretch of the line takes a share of the difference after every character
/// (one run), as long a stretch as one share keeps on course.
fn on_course(mut own: Vec<f64>, pens: &[Option<(f64, f64)>], target: &[Option<(f64, Option<f64>)>], size: f64) -> Vec<f64> {
    let tolerance = size * 0.01;
    // How much further along than `own` lays it out each glyph the file placed must go.
    let mut at = 0.0;
    let mut behind = vec![];
    for (i, ((p, t), e)) in pens.iter().zip(target).zip(&own).enumerate() {
        if let (Some((p, _)), Some((t, _))) = (p, t) {
            behind.push((i, t - p - at));
        }
        at += e;
    }
    if behind.iter().all(|(_, d)| d.abs() <= tolerance) {
        return own;
    }
    // The character the stretch starts at, how far the shares before it moved it, the last glyph
    // the stretch keeps on course, and the shares (lowest, highest) that keep each since on it.
    let (mut from, mut moved, mut last) = (0, 0.0, 0);
    let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
    for &(i, d) in &behind {
        if i <= from {
            continue;
        }
        let course = |from: usize, moved: f64| {
            let n = (i - from) as f64;
            ((d - moved - tolerance) / n, (d - moved + tolerance) / n)
        };
        let (l, h) = course(from, moved);
        if l > hi || h < lo {
            // No share keeps this glyph on course too: the stretch ends at the last one it does.
            let share = (lo + hi) / 2.0;
            for e in own.get_mut(from..last).into_iter().flatten() {
                *e += share;
            }
            moved += share * (last - from) as f64;
            from = last;
            (lo, hi) = course(from, moved);
        } else {
            (lo, hi) = (lo.max(l), hi.min(h));
        }
        last = i;
    }
    // The last stretch takes its share after the characters past the last glyph the file placed
    // too (it changes nothing there).
    if lo.is_finite() && hi.is_finite() {
        let share = (lo + hi) / 2.0;
        for e in own.get_mut(from..).into_iter().flatten() {
            *e += share;
        }
    }
    own
}

/// The middle value of `v` (the upper one of an even count).
pub(crate) fn median(mut v: Vec<f64>) -> Option<f64> {
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied()
}

/// The tracking (thousandths of an em) that sets a line of `chars` characters, laid out untracked
/// as `laid`, `extra` points longer, as the file set it: the same after every character, except
/// the spaces read from gaps or drawn between glyphs (`gaps`: byte offset, gap in points) where
/// that would put the glyphs after them more than a tenth of an em off (a barcode's digits, set in
/// groups apart). Those take their own tracking, given with their byte offsets.
fn spacing(laid: &TextLayout, gaps: &[(usize, f64)], extra: f64, chars: usize, size: f64) -> (f64, Vec<(usize, f64)>) {
    let pairs = chars.saturating_sub(1).max(1) as f64;
    // The advance of the first glyph laid out at each byte offset.
    let space: HashMap<usize, f64> = laid.glyphs.iter().rev().map(|g| (g.byte, g.advance)).collect();
    // The space and the character before it take `need` (the gap less the space) between them;
    // tracking gives them twice its share.
    let needs: Vec<(usize, f64)> = gaps.iter().filter_map(|&(b, gap)| Some((b, gap - space.get(&b)?))).collect();
    let mut own: Vec<(usize, f64)> = vec![];
    let mut taken: HashSet<usize> = HashSet::new();
    let mut tracking = extra / pairs;
    // Each space taken apart changes the others' share: until no more are.
    loop {
        let more: Vec<(usize, f64)> =
            needs.iter().filter(|(b, need)| !taken.contains(b) && (need - 2.0 * tracking).abs() > size * 0.1).copied().collect();
        if more.is_empty() {
            break;
        }
        taken.extend(more.iter().map(|(b, _)| *b));
        own.extend(more);
        let rest = pairs - 2.0 * own.len() as f64;
        tracking = if rest >= 1.0 { (extra - own.iter().map(|(_, need)| need).sum::<f64>()) / rest } else { 0.0 };
    }
    own.sort_by_key(|(b, _)| *b);
    let em = |pts: f64| (pts / size * 1000.0).clamp(-1000.0, 10000.0).round();
    (em(tracking), own.into_iter().map(|(b, need)| (b, em(need - tracking))).collect())
}

/// `runs` with tracking `tracking`, and each space at a byte offset of `spaces` (in order) a run
/// of its own with the tracking given with it.
fn tracked(runs: Vec<TextRun>, tracking: f64, spaces: &[(usize, f64)]) -> Vec<TextRun> {
    let mut out = Vec::with_capacity(runs.len() + 2 * spaces.len());
    let mut off = 0;
    for r in runs {
        let style = CharStyle { tracking, ..r.style };
        let end = off + r.text.len();
        let mut from = 0;
        for &(b, own) in spaces.iter().filter(|(b, _)| (off..end).contains(b)) {
            let at = b - off;
            let len = r.text.get(at..).and_then(|t| t.chars().next()).map_or(1, char::len_utf8);
            let (Some(before), Some(space)) = (r.text.get(from..at), r.text.get(at..at + len)) else { continue };
            if !before.is_empty() {
                out.push(TextRun { text: before.to_string(), style: style.clone(), inline: None });
            }
            out.push(TextRun { text: space.to_string(), style: CharStyle { tracking: own, ..style.clone() }, inline: None });
            from = at + len;
        }
        if let Some(rest) = r.text.get(from..).filter(|t| !t.is_empty()) {
            out.push(TextRun { text: rest.to_string(), style, inline: None });
        }
        off = end;
    }
    out
}

/// A space a file draws as a glyph: U+0020, the no-break spaces (U+00A0, U+202F) or one of the
/// fixed-width spaces U+2000 to U+200A (en, em, thin, hair and the others).
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\u{a0}' | '\u{2000}'..='\u{200a}' | '\u{202f}')
}

fn round(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Lower-case letters and digits only (`"Source Sans 3"` → `"sourcesans3"`).
fn norm(s: &str) -> String {
    s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

/// `"TimesNewRoman"` → `"Times New Roman"`, `"SourceSans3"` → `"Source Sans 3"`.
fn spaced(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let mut prev: Option<char> = None;
    for c in s.chars() {
        if let Some(p) = prev
            && ((p.is_ascii_lowercase() && c.is_ascii_uppercase()) || (p.is_ascii_alphabetic() && c.is_ascii_digit()))
        {
            out.push(' ');
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

/// `name` without a PostScript name's trailing `PSMT`, `MT` or `PS`.
fn strip_ps(name: &str) -> &str {
    ["PSMT", "MT", "PS"].iter().find_map(|s| name.strip_suffix(s).filter(|r| !r.is_empty())).unwrap_or(name)
}

/// The families available, by [`norm`]ed name.
pub(crate) struct Families(HashMap<String, String>);

impl Families {
    pub fn available() -> Self {
        Self(vectorcraft_text::FontDb::global().families().into_iter().map(|f| (norm(&f), f)).collect())
    }

    /// The family and style of a font with base (PostScript) name `name`, or of weight `weight`
    /// and slant `italic` when the name has no style; whether the family is available.
    pub fn resolve(&self, name: &str, weight: Option<u32>, italic: bool) -> (String, String, bool) {
        // A subset's six-letter tag.
        let name = match name.split_once('+') {
            Some((tag, rest)) if tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()) => rest,
            _ => name,
        };
        // An installed face of that exact PostScript name: its own family and style.
        if let Some((family, style)) = vectorcraft_text::FontDb::global().by_postscript_name(name) {
            return (family, style, true);
        }
        let (fam, style) = name.split_once(['-', ',']).unwrap_or((name, ""));
        let found = [fam, strip_ps(fam)].iter().find_map(|f| self.0.get(&norm(f)).cloned());
        let style = match spaced(strip_ps(style)).as_str() {
            "" | "Roman" | "Book" | "Normal" | "Plain" => match (weight.is_some_and(|w| w >= 600) || style.contains("Bold"), italic) {
                (true, true) => "Bold Italic".to_string(),
                (true, false) => "Bold".to_string(),
                (false, true) => "Italic".to_string(),
                (false, false) => "Regular".to_string(),
            },
            s => s.to_string(),
        };
        match found {
            Some(f) => (f, style, true),
            None => (spaced(strip_ps(fam)).trim().to_string(), style, false),
        }
    }
}

/// Hebrew or Arabic comes from a PDF in visual order (each glyph where it is drawn): put `t`'s text
/// back in logical order, with the paragraph direction that shows it as drawn.
fn to_logical(t: &mut TextObject) {
    let Some((order, rtl)) = vectorcraft_text::logical_order(&t.plain_text()) else { return };
    let chars: Vec<(char, usize)> = t.runs.iter().enumerate().flat_map(|(i, r)| r.text.chars().map(move |c| (c, i))).collect();
    let mut runs: Vec<(usize, String)> = Vec::with_capacity(t.runs.len());
    for &(c, i) in order.iter().filter_map(|&k| chars.get(k)) {
        match runs.last_mut() {
            Some((run, text)) if *run == i => text.push(c),
            _ => runs.push((i, c.to_string())),
        }
    }
    let runs = runs.into_iter().filter_map(|(i, text)| Some(TextRun { text, style: t.runs.get(i)?.style.clone(), inline: None })).collect();
    t.runs = runs;
    t.para.direction = Some(if rtl { ParaDirection::RightToLeft } else { ParaDirection::LeftToRight });
}
