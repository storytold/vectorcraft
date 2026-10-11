//! Text as editable point type: glyphs drawn one after another on the same baseline make one
//! point text object of their Unicode text, a run per font, size and paint (a gap wider than a
//! fifth of the size reads as a space). Fonts are named from the file's base font name (its
//! subset prefix dropped, the family matched against the fonts available); a font that isn't
//! available keeps its name, and the text shows in the fallback font until it is.
//!
//! Glyphs each turned a little further along a curve (type set on a path: apps write each glyph
//! with its own placement) make one type-on-a-path object, its path through the glyphs' baseline.

use std::collections::HashMap;

use kurbo::{Affine, BezPath, Point, Rect, Vec2};
use vectorcraft_color::Paint;
use vectorcraft_doc::{CharStyle, ParaDirection, TextKind, TextObject, TextRun};
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
}

impl Look {
    /// Can a glyph drawn with `other` join a run drawn with this? A fill joins a run whose
    /// glyphs were also stroked over (fill and stroke rendering draws each glyph twice).
    fn takes(&self, other: &Look) -> bool {
        self.font == other.font
            && (self.size - other.size).abs() <= self.size * 0.01
            && (self.h_scale - other.h_scale).abs() < 0.5
            && self.fill == other.fill
            && (other.stroke.is_none() || self.stroke == other.stroke)
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
            ..CharStyle::default()
        }
    }
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
    /// Each glyph's baseline origin, then where the last one ends.
    baseline: Vec<Point>,
    runs: Vec<(Look, String)>,
    /// The first glyph was set vertically ([`Upright`]).
    upright: Option<Upright>,
    /// The glyphs make a column (vertical type), each below the last.
    column: bool,
    /// Each glyph's ink box as drawn (document space), in drawing order; glyphs without ink are
    /// left out.
    pub ink: Vec<Rect>,
    /// The spaces read from gaps between glyphs: their byte offset in the text, and the gap from
    /// the end of the glyph before to the start of the next (points).
    gaps: Vec<(usize, f64)>,
    /// Each glyph's advance in the file's font (points), with [`Self::baseline`].
    advances: Vec<f64>,
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
            baseline: vec![],
            runs: vec![],
            upright: None,
            column: false,
            ink: vec![],
            gaps: vec![],
            advances: vec![],
        }
    }

    /// Add glyph `text` set vertically (`upright`) if it continues this column (or makes this
    /// line's single glyph a column): below the last one, in the same column. `false`: it starts
    /// another line.
    pub fn push_upright(&mut self, look: &Look, at: Placement, opacity: f32, upright: Upright, text: &str) -> bool {
        let down = -self.at.up();
        let size = self.at.size.max(at.size);
        let Some((last, run)) = self.runs.last_mut() else {
            self.runs.push((look.clone(), text.to_string()));
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
        if step > size * 0.9 && !run.ends_with(' ') && !text.starts_with(' ') {
            run.push(' ');
        }
        if last.takes(look) {
            run.push_str(text);
        } else {
            self.runs.push((look.clone(), text.to_string()));
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
        let bytes: usize = self.runs.iter().map(|(_, t)| t.len()).sum();
        if let Some((last, run)) = self.runs.last_mut() {
            let size = self.at.size.max(at.size);
            let gap = (at.origin - self.next).dot(self.last_dir);
            let on_line = opacity == self.opacity
                && at.dir.dot(self.at.dir) > 0.9995
                && (at.slant - self.at.slant).abs() < 1e-3
                && (at.origin - self.at.origin).dot(self.at.up()).abs() < size * 0.15
                // Proportional widths close CJK marks up by up to half an em.
                && gap > -size * 0.6
                && gap < size * 3.0;
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
            // A gap wider than a fifth of an em reads as a space.
            if gap > size * 0.2 && !run.ends_with(' ') && !text.starts_with(' ') {
                run.push(' ');
                if on_line {
                    self.gaps.push((bytes, gap));
                }
            }
            if last.takes(look) {
                run.push_str(text);
            } else {
                self.runs.push((look.clone(), text.to_string()));
            }
        } else {
            self.runs.push((look.clone(), text.to_string()));
        }
        self.next = at.origin + at.dir * advance;
        if !text.trim().is_empty() {
            self.visible = self.next;
        }
        self.last = at.origin;
        self.last_dir = at.dir;
        self.baseline.push(at.origin);
        self.advances.push(advance);
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
    /// and stroke rendering): the run is stroked. `false`: it isn't that glyph.
    pub fn stroke_last(&mut self, font: u128, at: Placement, stroke: (Paint, f64)) -> bool {
        let Some((look, _)) = self.runs.last_mut().filter(|(l, _)| l.font == font && (at.origin - self.last).hypot() < self.at.size * 1e-3) else {
            return false;
        };
        look.stroke = Some(stroke);
        true
    }

    /// The point type object, its opacity and what else the line tells.
    pub fn finish(mut self) -> Option<(TextObject, f32, LineFacts)> {
        let mut facts = LineFacts { ink: std::mem::take(&mut self.ink), ..LineFacts::default() };
        // Turned along a curve: type on a path through the glyphs.
        let path = if self.curved() { self.baseline_path() } else { None };
        facts.wraps = path.is_none() && self.upright.is_none() && self.runs.last().is_some_and(|(_, t)| t.ends_with(' '));
        if let Some((_, last)) = self.runs.last_mut() {
            last.truncate(last.trim_end().len());
        }
        self.runs.retain(|(_, t)| !t.is_empty());
        if self.runs.iter().all(|(_, t)| t.trim().is_empty()) {
            return None;
        }
        let mut runs = self.runs.into_iter().map(|(look, text)| TextRun { text, style: look.style(), inline: None });
        let first = runs.next()?;
        let mut t = TextObject::point(Point::ORIGIN, &first.text, first.style);
        t.runs.extend(runs);
        if self.upright.is_none() {
            to_logical(&mut t);
        }
        let angle = self.at.dir.atan2();
        // The glyphs' tops lean along the baseline (+x), up being -y in the type's own space.
        t.xf = Affine::translate(self.at.origin.to_vec2()) * Affine::rotate(angle) * Affine::skew(-self.at.slant, 0.0);
        let db = vectorcraft_text::FontDb::global();
        // Glyphs spaced each their own way (character tightening, optical kerning) keep their
        // place: each character its own tracking.
        let own = (self.upright.is_none() && t.para.direction.is_none())
            .then(|| own_spacing(&t, &self.baseline, &self.advances, path.as_ref(), self.at.dir, self.at.size))
            .flatten();
        if let Some(own) = &own {
            t.runs = tracked_each(std::mem::take(&mut t.runs), own, self.at.size);
        }
        if let Some(path) = path {
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
        let laid = vectorcraft_text::layout(db, &t);
        let natural: f64 = laid.glyphs.iter().map(|g| g.advance).sum();
        let chars: usize = t.runs.iter().map(|r| r.text.chars().count()).sum();
        let size = self.at.size;
        let mut bounds = laid.bounds;
        if own.is_none() && chars > 1 && length.is_finite() && (length - natural).abs() > size * 0.01 {
            // The spaces read from gaps keep their own place only in text left in the file's order.
            let total: usize = t.runs.iter().map(|r| r.text.len()).sum();
            let gaps: Vec<(usize, f64)> =
                if t.para.direction.is_none() { self.gaps.into_iter().filter(|(b, _)| *b < total).collect() } else { vec![] };
            let (tracking, spaces) = spacing(&laid, &gaps, length - natural, chars, size);
            t.runs = tracked(std::mem::take(&mut t.runs), tracking, &spaces);
            // Tracked: laid out again for its bounds.
            bounds = vectorcraft_text::layout(db, &t).bounds;
        }
        t.cached_bounds = Some(bounds);
        Some((t, self.opacity, facts))
    }
}

/// The space each glyph of `t` (laid out untracked) leaves after its advance to put the next one
/// where the file did (points; the last glyph's is the one before it), when that varies by more
/// than a fiftieth of an em: one tracking value can't place them. Glyph origins are `baseline`,
/// measured along `path` (type on a path: one curve per glyph) or along `dir`. None when the
/// glyphs can't be matched to the characters one to one (spaces, ligatures), or when the font
/// laid out isn't the file's (its advances differ from `advances`): spacing measured against a
/// stand-in's widths would keep the stand-in's differences once the font is installed.
fn own_spacing(t: &TextObject, baseline: &[Point], advances: &[f64], path: Option<&BezPath>, dir: Vec2, size: f64) -> Option<Vec<f64>> {
    let text: String = t.runs.iter().map(|r| r.text.as_str()).collect();
    let chars = text.chars().count();
    if chars < 3 || text.contains(' ') || baseline.len() != chars || advances.len() != chars {
        return None;
    }
    let laid = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    let advs: Vec<f64> = laid.glyphs.iter().map(|g| g.advance).collect();
    if advs.len() != chars || advs.iter().zip(advances).any(|(a, b)| (a - b).abs() > size * 0.01) {
        return None;
    }
    let dist: Vec<f64> = match path {
        Some(bp) => bp.segments().take(chars - 1).map(|seg| kurbo::ParamCurveArclen::arclen(&seg, 1e-3)).collect(),
        None => baseline.windows(2).map(|w| (w[1] - w[0]).dot(dir)).collect(),
    };
    if dist.len() != chars - 1 {
        return None;
    }
    let mut extra: Vec<f64> = dist.iter().zip(&advs).map(|(d, a)| d - a).collect();
    let mean = extra.iter().sum::<f64>() / extra.len() as f64;
    if extra.iter().all(|e| (e - mean).abs() <= size * 0.02) {
        return None;
    }
    extra.push(*extra.last()?);
    Some(extra)
}

/// `runs` with each character's tracking from `own` (points, one per character), characters with
/// the same tracking kept in one run.
fn tracked_each(runs: Vec<TextRun>, own: &[f64], size: f64) -> Vec<TextRun> {
    let em = |pts: f64| (pts / size * 1000.0).clamp(-1000.0, 1000.0).round();
    let mut out: Vec<TextRun> = vec![];
    let mut k = 0;
    for r in runs {
        for c in r.text.chars() {
            let style = CharStyle { tracking: own.get(k).map_or(0.0, |e| em(*e)), ..r.style.clone() };
            k += 1;
            match out.last_mut() {
                Some(last) if last.style == style && last.inline == r.inline => last.text.push(c),
                _ => out.push(TextRun { text: c.to_string(), style, inline: r.inline.clone() }),
            }
        }
    }
    out
}

/// The tracking (thousandths of an em) that sets a line of `chars` characters, laid out untracked
/// as `laid`, `extra` points longer, as the file set it: the same after every character, except
/// the spaces read from gaps (`gaps`: byte offset, gap in points) where that would put the glyphs
/// after them more than a tenth of an em off (a barcode's digits, set in groups apart). Those take
/// their own tracking, given with their byte offsets.
fn spacing(laid: &TextLayout, gaps: &[(usize, f64)], extra: f64, chars: usize, size: f64) -> (f64, Vec<(usize, f64)>) {
    let pairs = chars.saturating_sub(1).max(1) as f64;
    // The space and the character before it take `need` (the gap less the space) between them;
    // tracking gives them twice its share.
    let space = |b: usize| laid.glyphs.iter().find(|g| g.byte == b).map(|g| g.advance);
    let needs: Vec<(usize, f64)> = gaps.iter().filter_map(|&(b, gap)| Some((b, gap - space(b)?))).collect();
    let mut own: Vec<(usize, f64)> = vec![];
    let mut tracking = extra / pairs;
    // Each space taken apart changes the others' share: until no more are.
    loop {
        let more: Vec<(usize, f64)> =
            needs.iter().filter(|(b, need)| !own.iter().any(|(o, _)| o == b) && (need - 2.0 * tracking).abs() > size * 0.1).copied().collect();
        if more.is_empty() {
            break;
        }
        own.extend(more);
        let rest = pairs - 2.0 * own.len() as f64;
        tracking = if rest >= 1.0 { (extra - own.iter().map(|(_, need)| need).sum::<f64>()) / rest } else { 0.0 };
    }
    own.sort_by_key(|(b, _)| *b);
    let em = |pts: f64| (pts / size * 1000.0).clamp(-1000.0, 1000.0).round();
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
            let (Some(before), Some(space)) = (r.text.get(from..at), r.text.get(at..at + 1)) else { continue };
            if !before.is_empty() {
                out.push(TextRun { text: before.to_string(), style: style.clone(), inline: None });
            }
            out.push(TextRun { text: space.to_string(), style: CharStyle { tracking: own, ..style.clone() }, inline: None });
            from = at + 1;
        }
        if let Some(rest) = r.text.get(from..).filter(|t| !t.is_empty()) {
            out.push(TextRun { text: rest.to_string(), style, inline: None });
        }
        off = end;
    }
    out
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
