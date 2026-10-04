//! VectorCraft text: font database, shaping, layout, glyph outlines.
//!
//! API contract used by `vectorcraft-render`, `vectorcraft-tools` and the UI:
//! - [`FontDb::global`]: process-wide database preloaded with the bundled OFL fonts.
//! - [`layout`]: lays out a [`TextObject`] into glyph outlines in *text space* (apply `t.xf` to
//!   get document coordinates), plus line and caret information.
//! - [`caret_position`] / [`hit_byte`]: caret geometry and hit testing for the Type tool.
//!
//! Frames (`TextKind::Area`) and paths (`TextKind::OnPath`) are interpreted in text space.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod composer;
pub mod edit;
mod features;
mod fontdb;
pub mod hyphen;
mod layout;
mod shape;
pub mod thread;

pub use features::OtFeatures;
pub use fontdb::{FALLBACK_FAMILY, FontDb, FontFace};
use kurbo::{BezPath, Point, Rect, Vec2};
pub use layout::{layout, layout_with};
pub use vectorcraft_doc::TextObject;

pub use vectorcraft_doc::FirstBaseline;

/// Paragraph composer (Paragraph panel menu).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Composer {
    /// Break each line as soon as it is full.
    SingleLine,
    /// Knuth–Plass total fit over the paragraph (justified area text only).
    #[default]
    EveryLine,
}

/// Layout parameters that the document model doesn't store per object (Area Type Options,
/// composer, OpenType features). [`layout`] takes rows/columns/inset/first baseline from the object.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutOptions {
    /// Area type rows and columns (text flows down each column, then across).
    pub rows: usize,
    pub columns: usize,
    /// Gutter between rows/columns in points.
    pub gutter: f64,
    /// Inset from the frame edges in points.
    pub inset: f64,
    pub first_baseline: FirstBaseline,
    /// Minimum first-baseline offset in points.
    pub first_baseline_min: f64,
    pub composer: Composer,
    pub features: OtFeatures,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            rows: 1,
            columns: 1,
            gutter: 18.0,
            inset: 0.0,
            first_baseline: FirstBaseline::Ascent,
            first_baseline_min: 0.0,
            composer: Composer::EveryLine,
            features: OtFeatures::default(),
        }
    }
}

/// One laid-out glyph.
#[derive(Clone, Debug)]
pub struct PositionedGlyph {
    /// Outline in text space.
    pub outline: BezPath,
    /// Index of the run (`TextObject::runs`) the glyph came from.
    pub run: usize,
    /// Byte offset of the source character (cluster start) in the plain text.
    pub byte: usize,
    /// Pen position on the baseline (leading edge of the glyph).
    pub origin: Point,
    /// Advance along the baseline (includes tracking and justification space).
    pub advance: f64,
    /// Number of source bytes covered by the glyph's cluster.
    pub len: usize,
    /// Baseline direction in radians (0 = horizontal; non-zero for type on a path).
    pub angle: f64,
    /// Index into [`TextLayout::lines`].
    pub line: usize,
    /// [`FontFace::id`] of the face that supplied the glyph.
    pub font_id: u32,
}

/// One line of laid-out text.
///
/// For type on a path there is a single line; `x0, baseline` is the start point of the text on the
/// path and `x1` the x of its end point.
#[derive(Clone, Debug)]
pub struct LineInfo {
    pub baseline: f64,
    /// Left edge of the line content (after alignment).
    pub x0: f64,
    /// Right edge of the line content, trailing spaces excluded.
    pub x1: f64,
    pub ascent: f64,
    pub descent: f64,
    /// Byte range of the line in the plain text (excludes the paragraph's `\n`).
    pub start: usize,
    pub end: usize,
    /// Range of the line's glyphs in [`TextLayout::glyphs`].
    pub glyph_start: usize,
    pub glyph_end: usize,
    /// Horizontal span available to the line (frame span minus indents; the content extent for
    /// point type). Used for hit testing across columns.
    pub avail: (f64, f64),
}

#[derive(Clone, Debug, Default)]
pub struct TextLayout {
    pub glyphs: Vec<PositionedGlyph>,
    pub lines: Vec<LineInfo>,
    /// Ink/advance bounds in text space.
    pub bounds: Rect,
    /// True if area text did not fit its frame (the red overflow "+" marker), or on-path text ran
    /// past the end of the path.
    pub overflow: bool,
    /// True for type on a path (glyphs have individual `angle`s).
    pub on_path: bool,
    /// Area type: the frame cells text flowed into (one per row/column).
    pub frames: Vec<Rect>,
}

impl TextLayout {
    /// All glyph outlines combined (e.g. for Create Outlines).
    pub fn to_bezpath(&self) -> BezPath {
        let mut p = BezPath::new();
        for g in &self.glyphs {
            p.extend(g.outline.iter());
        }
        p
    }
    /// Index of the line containing byte offset `byte`.
    pub fn line_of(&self, byte: usize) -> usize {
        self.lines.iter().rposition(|l| l.start <= byte).unwrap_or(0)
    }
}

fn dir(angle: f64) -> Vec2 {
    Vec2::new(angle.cos(), angle.sin())
}

/// Caret for byte offset `byte`, as a (top, bottom) segment in text space.
pub fn caret_position(layout: &TextLayout, byte: usize) -> (Point, Point) {
    let Some(line) = layout.lines.get(layout.line_of(byte)) else {
        return (Point::ZERO, Point::ZERO);
    };
    let glyphs = &layout.glyphs[line.glyph_start..line.glyph_end];
    let (pos, angle) = if let Some(g) = glyphs.iter().find(|g| g.byte + g.len > byte) {
        let frac = if byte <= g.byte { 0.0 } else { (byte - g.byte) as f64 / g.len.max(1) as f64 };
        (g.origin + dir(g.angle) * (g.advance * frac), g.angle)
    } else if let Some(g) = glyphs.last() {
        (g.origin + dir(g.angle) * g.advance, g.angle)
    } else {
        (Point::new(line.x0, line.baseline), 0.0)
    };
    // Up vector in y-down space, rotated with the baseline.
    let d = dir(angle);
    let up = Vec2::new(d.y, -d.x);
    (pos + up * line.ascent, pos - up * line.descent)
}

/// Byte offset nearest to the text-space point `p` (for clicking/dragging with the Type tool).
pub fn hit_byte(layout: &TextLayout, p: Point) -> usize {
    if layout.on_path {
        let best = layout.glyphs.iter().min_by(|a, b| {
            let ca = a.origin + dir(a.angle) * (a.advance * 0.5);
            let cb = b.origin + dir(b.angle) * (b.advance * 0.5);
            (p - ca).hypot2().total_cmp(&(p - cb).hypot2())
        });
        return match best {
            Some(g) if (p - g.origin).dot(dir(g.angle)) < g.advance * 0.5 => g.byte,
            Some(g) => g.byte + g.len,
            None => layout.lines.first().map_or(0, |l| l.start),
        };
    }
    // Nearest line band (vertical distance first, then distance to the line's column).
    let dist = |l: &LineInfo| {
        let dy = if p.y < l.baseline - l.ascent {
            l.baseline - l.ascent - p.y
        } else if p.y > l.baseline + l.descent {
            p.y - l.baseline - l.descent
        } else {
            0.0
        };
        let (a, b) = (l.avail.0.min(l.x0), l.avail.1.max(l.x1));
        let dx = if p.x < a {
            a - p.x
        } else if p.x > b {
            p.x - b
        } else {
            0.0
        };
        // Vertical distance first (to 1/100 pt), then the nearest column.
        ((dy * 100.0).round(), dx)
    };
    let Some(li) = (0..layout.lines.len()).min_by(|&a, &b| {
        let (da, db) = (dist(&layout.lines[a]), dist(&layout.lines[b]));
        da.0.total_cmp(&db.0).then(da.1.total_cmp(&db.1))
    }) else {
        return 0;
    };
    byte_in_line(layout, li, p.x)
}

/// Byte offset nearest to `x` on line `li`.
pub fn byte_in_line(layout: &TextLayout, li: usize, x: f64) -> usize {
    let Some(line) = layout.lines.get(li) else { return 0 };
    let glyphs = &layout.glyphs[line.glyph_start..line.glyph_end];
    if let Some(g) = glyphs.iter().find(|g| x < g.origin.x + g.advance * 0.5) {
        return g.byte;
    }
    line_end(layout, li)
}

/// Caret byte at the end of line `li`: a soft-wrapped line ends before its trailing space so the
/// caret stays on the line.
pub fn line_end(layout: &TextLayout, li: usize) -> usize {
    let Some(line) = layout.lines.get(li) else { return 0 };
    let glyphs = &layout.glyphs[line.glyph_start..line.glyph_end];
    let soft_wrap = layout.lines.get(li + 1).is_some_and(|n| n.start == line.end);
    match glyphs.iter().rev().find(|g| g.len > 0) {
        Some(g) if soft_wrap && g.byte + g.len == line.end => g.byte,
        _ => line.end,
    }
}

/// Caret byte at the start of the line containing `byte` (Home).
pub fn line_home(layout: &TextLayout, byte: usize) -> usize {
    layout.lines.get(layout.line_of(byte)).map_or(0, |l| l.start)
}

/// Caret byte at the end of the line containing `byte` (End).
pub fn line_end_of(layout: &TextLayout, byte: usize) -> usize {
    line_end(layout, layout.line_of(byte))
}

/// Caret x position (text space) of `byte` on line `li`.
fn x_in_line(layout: &TextLayout, li: usize, byte: usize) -> f64 {
    let line = &layout.lines[li];
    let glyphs = &layout.glyphs[line.glyph_start..line.glyph_end];
    if let Some(g) = glyphs.iter().find(|g| g.byte + g.len > byte) {
        let frac = if byte <= g.byte { 0.0 } else { (byte - g.byte) as f64 / g.len.max(1) as f64 };
        return g.origin.x + g.advance * frac;
    }
    glyphs.last().map_or(line.x0, |g| g.origin.x + g.advance)
}

/// Move the caret `delta` lines up (negative) or down, keeping horizontal position `goal_x`
/// (text space). Moving past the first/last line goes to the start/end of the text.
pub fn caret_vertical(layout: &TextLayout, byte: usize, delta: i32, goal_x: f64) -> usize {
    if layout.lines.is_empty() {
        return byte;
    }
    let li = layout.line_of(byte) as i64 + delta as i64;
    if li < 0 {
        return layout.lines[0].start;
    }
    if li as usize >= layout.lines.len() {
        return layout.lines.last().map_or(byte, |l| l.end);
    }
    byte_in_line(layout, li as usize, goal_x)
}

/// Nearest point on `path` to `p`: (fraction of the path's arc length 0..1, distance). Used to
/// start type on a path where the user clicked.
pub fn path_fraction_at(path: &BezPath, p: Point) -> (f64, f64) {
    use kurbo::{ParamCurve, ParamCurveArclen, ParamCurveNearest};
    let mut total = 0.0;
    let mut best = (0.0, f64::INFINITY);
    for seg in path.segments() {
        let len = seg.arclen(1e-3);
        let n = seg.nearest(p, 1e-4);
        let d = n.distance_sq.sqrt();
        if d < best.1 {
            best = (total + seg.subsegment(0.0..n.t).arclen(1e-3), d);
        }
        total += len;
    }
    if total <= 0.0 { (0.0, best.1) } else { ((best.0 / total).clamp(0.0, 1.0), best.1) }
}

/// Highlight quads (text space, clockwise from top-left) covering the selected bytes `a..b`.
pub fn selection_quads(layout: &TextLayout, a: usize, b: usize) -> Vec<[Point; 4]> {
    let (a, b) = (a.min(b), a.max(b));
    let mut out = vec![];
    if a == b {
        return out;
    }
    if layout.on_path {
        for g in layout.glyphs.iter().filter(|g| g.byte >= a && g.byte < b) {
            let li = &layout.lines[0];
            let d = dir(g.angle);
            let up = Vec2::new(d.y, -d.x);
            let (p0, p1) = (g.origin, g.origin + d * g.advance);
            out.push([p0 + up * li.ascent, p1 + up * li.ascent, p1 - up * li.descent, p0 - up * li.descent]);
        }
        return out;
    }
    for (li, l) in layout.lines.iter().enumerate() {
        if l.start > b || l.end < a || (l.end == a && l.start < a && layout.lines.get(li + 1).is_some_and(|n| n.start == l.end)) {
            continue;
        }
        let xa = if a <= l.start { l.x0.min(x_in_line(layout, li, l.start)) } else { x_in_line(layout, li, a) };
        let mut xb = x_in_line(layout, li, b.min(l.end));
        if b > l.end {
            // The selection continues past the line: include the line break / trailing space.
            let last = layout.glyphs[l.glyph_start..l.glyph_end].last().map_or(l.x1, |g| g.origin.x + g.advance);
            xb = last.max(l.x1) + (l.ascent + l.descent) * 0.25;
        }
        if xb - xa <= 1e-9 {
            continue;
        }
        let (t, bt) = (l.baseline - l.ascent, l.baseline + l.descent);
        out.push([Point::new(xa, t), Point::new(xb, t), Point::new(xb, bt), Point::new(xa, bt)]);
    }
    out
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_typo;
