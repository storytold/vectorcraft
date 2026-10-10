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

mod composer;
pub mod craft_fonts;
pub mod edit;
pub mod embed;
mod features;
mod fontdb;
pub mod hyphen;
mod layout;
mod shape;
#[cfg(not(target_arch = "wasm32"))]
mod suitcase;
#[cfg(any(test, feature = "test-fonts"))]
pub mod test_fonts;
pub mod thread;

pub use craft_fonts::{CRAFT_FONTS, CraftFont, WEB_FONTS, WebFont};
pub use features::{LIGATURE_TRACKING_LIMITS, OtFeatures, explicit_ligatures, ligatures_suppressed_by};
pub use fontdb::{
    FALLBACK_FAMILY, FontClass, FontDb, FontFace, FontMatch, FontTraits, IcfMargins, PlatformFontFiles, WantedFont, WantedFonts, app_font_dir,
    is_font_file, is_suitcase, set_app_font_dir, set_platform_font_files, set_user_font_dirs, style_weight, system_font_dirs, user_font_dirs,
};
pub use hyphen::{hyphenation_exceptions, set_hyphenation_exceptions};
use kurbo::{Affine, BezPath, Point, Rect, Vec2};
pub use layout::{decorations, layout, layout_with};
pub use vectorcraft_doc::TextObject;

pub use vectorcraft_doc::{AreaFit, FirstBaseline, VerticalAlign};

/// Paragraph composer; stored per text object in [`vectorcraft_doc::ParaStyle::composer`].
pub use vectorcraft_doc::Composer;

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
    /// Vertical alignment of the lines in each row/column (Area Type Options "Align").
    pub vertical_align: VerticalAlign,
    /// Area type only: Shrink Text to Fit scales overflowing text down at layout time (see
    /// [`TextLayout::fit_scale`]); the other fits are the engine's business and lay out as `None`.
    pub fit: AreaFit,
    /// Overrides the object's paragraph composer (`None` = use `ParaStyle::composer`).
    pub composer: Option<Composer>,
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
            vertical_align: VerticalAlign::Top,
            fit: AreaFit::None,
            composer: None,
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
    /// The glyph's id in that face.
    pub gid: u32,
    /// Font units (y down, as [`FontDb::outline`] gives them) → text space: where the glyph is
    /// drawn, also for glyphs without an outline (spaces).
    pub xf: Affine,
    /// True when the logical leading edge is the glyph’s right edge.
    pub rtl: bool,
}

/// One line of laid-out text.
///
/// For type on a path there is a single line; `x0, baseline` is the start point of the text on the
/// path and `x1` the x of its end point.
#[derive(Clone, Debug)]
pub struct LineInfo {
    /// Automatically detected paragraph base direction.
    pub rtl: bool,
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
    /// Area type: index of the frame cell ([`TextLayout::frames`]) the line sits in (0 otherwise).
    pub region: usize,
}

/// An inline graphic ([`vectorcraft_doc::TextRun::inline`]) placed by the layout: draw the art of
/// the run's symbol through `xf`. Missing symbols reserve their room but get no entry.
#[derive(Clone, Debug, PartialEq)]
pub struct InlineGlyph {
    /// Index of the run (`TextObject::runs`).
    pub run: usize,
    /// Byte offset of its character in the plain text.
    pub byte: usize,
    /// Index of its (outline-less) glyph in [`TextLayout::glyphs`].
    pub glyph: usize,
    /// The symbol's art at its natural size (`Document::symbol_natural_xf`) → text space.
    pub xf: Affine,
    /// The art's bounds in text space.
    pub bounds: Rect,
}

#[derive(Clone, Debug)]
pub struct TextLayout {
    /// Lines retain inline/block coordinates; glyph geometry is in physical text space.
    pub vertical: bool,
    /// Inline/block (line) space → physical text space: identity for horizontal type, a quarter
    /// turn clockwise for vertical type (point type also centres its first column on the anchor).
    pub line_xf: Affine,
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
    /// Shrink Text to Fit: the factor the text's sizes, leading and baseline shifts were scaled
    /// by to fit its frame (1.0 when the text is not shrunk).
    pub fit_scale: f64,
    /// Inline graphics, in text order.
    pub inlines: Vec<InlineGlyph>,
}

impl Default for TextLayout {
    fn default() -> Self {
        Self {
            vertical: false,
            line_xf: Affine::IDENTITY,
            glyphs: Vec::new(),
            lines: Vec::new(),
            bounds: Rect::ZERO,
            overflow: false,
            on_path: false,
            frames: Vec::new(),
            fit_scale: 1.0,
            inlines: Vec::new(),
        }
    }
}

impl TextLayout {
    /// Convert physical text coordinates to inline/block coordinates.
    pub fn logical_point(&self, p: Point) -> Point {
        if self.vertical { self.line_xf.inverse() * p } else { p }
    }
    pub fn physical_point(&self, p: Point) -> Point {
        if self.vertical { self.line_xf * p } else { p }
    }
    /// Each line's baseline in text space, start to end (a vertical column's centre line), for
    /// the lines that hold characters; none for type on a path, which its path stands for.
    pub fn baselines(&self) -> Vec<(Point, Point)> {
        if self.on_path {
            return vec![];
        }
        let line = |l: &LineInfo| {
            let y = if self.vertical { l.baseline + (l.descent - l.ascent) / 2.0 } else { l.baseline };
            (self.physical_point(Point::new(l.x0, y)), self.physical_point(Point::new(l.x1, y)))
        };
        self.lines.iter().filter(|l| l.x1 > l.x0).map(line).collect()
    }
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
    /// Snap type to the pixel grid (type-optimized anti-aliasing): move each glyph so its pen
    /// position on the baseline lands on a whole device pixel, given `to_device` (text space →
    /// device pixels). Whole baselines and glyph starts keep small type crisp and evenly spaced.
    /// Only text that is neither rotated nor skewed nor on a path is snapped; returns whether
    /// the layout changed.
    pub fn snap_to_pixels(&mut self, to_device: Affine) -> bool {
        let [a, b, c, d, _, _] = to_device.as_coeffs();
        if self.on_path || self.vertical || b.abs() > 1e-9 || c.abs() > 1e-9 || a.abs() < 1e-12 || d.abs() < 1e-12 {
            return false;
        }
        let mut moved = false;
        let mut shifts = vec![];
        for (gi, g) in self.glyphs.iter_mut().enumerate().filter(|(_, g)| g.angle == 0.0) {
            let p = to_device * g.origin;
            let shift = Vec2::new((p.x.round() - p.x) / a, (p.y.round() - p.y) / d);
            if shift == Vec2::ZERO {
                continue;
            }
            g.origin += shift;
            g.outline.apply_affine(Affine::translate(shift));
            g.xf = Affine::translate(shift) * g.xf;
            if !self.inlines.is_empty() {
                shifts.push((gi, shift));
            }
            moved = true;
        }
        // Inline graphics move with their glyphs.
        for i in &mut self.inlines {
            if let Some(&(_, s)) = shifts.iter().find(|(gi, _)| *gi == i.glyph) {
                i.xf = Affine::translate(s) * i.xf;
                i.bounds = i.bounds + s;
            }
        }
        moved
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
    let (pos, angle) = if let Some(g) = glyphs.iter().find(|g| g.byte <= byte && g.byte + g.len > byte) {
        let frac = if byte <= g.byte { 0.0 } else { (byte - g.byte) as f64 / g.len.max(1) as f64 };
        (g.origin + dir(g.angle) * (g.advance * if g.rtl { 1.0 - frac } else { frac }), g.angle)
    } else if let Some(g) = glyphs.iter().max_by_key(|g| g.byte + g.len) {
        (g.origin + dir(g.angle) * if g.rtl { 0.0 } else { g.advance }, g.angle)
    } else {
        (layout.physical_point(Point::new(line.x0, line.baseline)), if layout.vertical { std::f64::consts::FRAC_PI_2 } else { 0.0 })
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
            Some(g) if ((p - g.origin).dot(dir(g.angle)) < g.advance * 0.5) != g.rtl => g.byte,
            Some(g) => g.byte + g.len,
            None => layout.lines.first().map_or(0, |l| l.start),
        };
    }
    let p = layout.logical_point(p);
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
    let nearest = glyphs
        .iter()
        .flat_map(|g| {
            let left = layout.logical_point(g.origin).x;
            let right = left + g.advance;
            if g.rtl { [(right, g.byte), (left, g.byte + g.len)] } else { [(left, g.byte), (right, g.byte + g.len)] }
        })
        .min_by(|a, b| (a.0 - x).abs().total_cmp(&(b.0 - x).abs()));
    nearest.map_or(line.start, |(_, byte)| byte)
}

/// Caret byte at the end of line `li`: a soft-wrapped line ends before its trailing space so the
/// caret stays on the line.
pub fn line_end(layout: &TextLayout, li: usize) -> usize {
    let Some(line) = layout.lines.get(li) else { return 0 };
    let glyphs = &layout.glyphs[line.glyph_start..line.glyph_end];
    let soft_wrap = layout.lines.get(li + 1).is_some_and(|n| n.start == line.end);
    match glyphs.iter().filter(|g| g.len > 0).max_by_key(|g| g.byte) {
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
    if let Some(g) = glyphs.iter().find(|g| g.byte <= byte && g.byte + g.len > byte) {
        let frac = if byte <= g.byte { 0.0 } else { (byte - g.byte) as f64 / g.len.max(1) as f64 };
        return layout.logical_point(g.origin).x + g.advance * if g.rtl { 1.0 - frac } else { frac };
    }
    glyphs.iter().max_by_key(|g| g.byte + g.len).map_or(line.x0, |g| layout.logical_point(g.origin).x + if g.rtl { 0.0 } else { g.advance })
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
    if layout.glyphs.iter().any(|g| g.rtl) {
        for g in layout.glyphs.iter().filter(|g| g.byte < b && g.byte + g.len > a) {
            let Some(l) = layout.lines.get(g.line) else { continue };
            let p = layout.logical_point(g.origin);
            let (x0, x1) = (p.x, p.x + g.advance);
            out.push(
                [
                    Point::new(x0, l.baseline - l.ascent),
                    Point::new(x1, l.baseline - l.ascent),
                    Point::new(x1, l.baseline + l.descent),
                    Point::new(x0, l.baseline + l.descent),
                ]
                .map(|p| layout.physical_point(p)),
            );
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
            let last = layout.glyphs[l.glyph_start..l.glyph_end].last().map_or(l.x1, |g| layout.logical_point(g.origin).x + g.advance);
            xb = last.max(l.x1) + (l.ascent + l.descent) * 0.25;
        }
        if xb - xa <= 1e-9 {
            continue;
        }
        let (t, bt) = (l.baseline - l.ascent, l.baseline + l.descent);
        out.push([Point::new(xa, t), Point::new(xb, t), Point::new(xb, bt), Point::new(xa, bt)].map(|p| layout.physical_point(p)));
    }
    out
}

/// Move horizontally through visual caret stops; source offsets stay in logical order.
pub fn caret_horizontal(layout: &TextLayout, byte: usize, right: bool) -> usize {
    let li = layout.line_of(byte);
    let Some(line) = layout.lines.get(li) else { return byte };
    let x = layout.logical_point(caret_position(layout, byte).0).x;
    let mut stops: Vec<_> =
        layout.glyphs.get(line.glyph_start..line.glyph_end).unwrap_or_default().iter().flat_map(|g| [g.byte, g.byte + g.len]).collect();
    stops.sort_unstable();
    stops.dedup();
    stops
        .into_iter()
        .filter_map(|b| {
            let bx = layout.logical_point(caret_position(layout, b).0).x;
            let d = if right { bx - x } else { x - bx };
            (d > 1e-6).then_some((d, b))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or_else(
            || {
                if right != line.rtl {
                    layout.lines.get(li + 1).map_or(byte, |next| next.start)
                } else {
                    li.checked_sub(1).and_then(|previous| layout.lines.get(previous)).map_or(byte, |previous| previous.end)
                }
            },
            |(_, b)| b,
        )
}

/// Does paragraph `text` run right to left: its `direction`, else (None) from its first strong
/// character (numbers and punctuation don't count)?
pub fn paragraph_is_rtl(text: &str, direction: Option<vectorcraft_doc::ParaDirection>) -> bool {
    layout::is_rtl(layout::para_bidi(text, direction).as_ref())
}

/// Text stored in visual order (as PDF content draws it, each glyph where it is drawn), back in
/// logical order: the order of its characters (indices into `visual.chars()`) and whether its
/// paragraph runs right to left; laid out that way, it reads as drawn. `None` when nothing in it is
/// right to left (the order stands).
pub fn logical_order(visual: &str) -> Option<(Vec<usize>, bool)> {
    let bidi = layout::para_bidi(visual, None)?;
    let para = bidi.paragraphs.first()?;
    // Reordering is its own inverse for runs of one direction (the usual case): the visual order
    // of the visual text is its logical order.
    let levels = bidi.reordered_levels_per_char(para, para.range.clone());
    Some((unicode_bidi::BidiInfo::reorder_visual(&levels), para.level.is_rtl()))
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_bidi;
#[cfg(test)]
mod tests_combos;
#[cfg(test)]
mod tests_embed;
#[cfg(test)]
mod tests_fit;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests_font_coverage;
#[cfg(test)]
mod tests_inline;
#[cfg(test)]
mod tests_scripts;
#[cfg(test)]
mod tests_snap;
#[cfg(test)]
mod tests_sysfonts;
#[cfg(test)]
mod tests_typo;
#[cfg(test)]
mod tests_variable;
#[cfg(test)]
mod tests_versions;
#[cfg(test)]
mod tests_vertical;
#[cfg(test)]
#[path = "../build/web_fonts.rs"]
mod web_fonts_build;
