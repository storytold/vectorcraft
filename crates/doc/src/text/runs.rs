use super::char_style::CharStyle;
use super::{Affine, Deserialize, PathData, Point, Rect, Serialize, Vec2, yes};

/// A named character or paragraph style: the attributes it sets (a subset of [`CharStyle`] or
/// [`ParaStyle`] fields, by their serialized names). Text using it records the name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextStyleDef {
    pub name: String,
    #[serde(default)]
    pub attrs: serde_json::Map<String, serde_json::Value>,
}

/// A run of text sharing one character style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextRun {
    pub text: String,
    pub style: CharStyle,
    /// An inline graphic: the run is one [`INLINE_CHAR`] drawn as a document symbol that flows
    /// with the text like a glyph (InDesign-style inline anchored object). Its `style` still sets
    /// the size and tracking; fills and strokes don't apply to the art.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<InlineArt>,
}

impl TextRun {
    /// A plain text run.
    pub fn new(text: impl Into<String>, style: CharStyle) -> Self {
        Self { text: text.into(), style, inline: None }
    }
    /// An inline graphic run showing `art`.
    pub fn inline(art: InlineArt, style: CharStyle) -> Self {
        Self { text: INLINE_CHAR.to_string(), style, inline: Some(art) }
    }
}

/// The object replacement character: the plain text of an inline graphic run.
pub const INLINE_CHAR: char = '\u{FFFC}';

/// Largest inline graphic scale (× the run's size) accepted from input.
pub const INLINE_MAX_SCALE: f64 = 100.0;

/// A document symbol placed inline in text ([`TextRun::inline`]).
///
/// Placement: the art is scaled uniformly so its height (visual bounds) is `scale` × the run's
/// font size, its left edge at the pen position, and its vertical centre on the middle of the
/// run's cap height, raised by `baseline_shift` (points, positive = up). Centred on the cap
/// height the art lines up with capitals and figures (like a mana symbol in rules text) and, at
/// the default scale, stays inside the font's ascent and descent so the leading doesn't change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InlineArt {
    /// Name of the document symbol ([`crate::Symbol::name`]).
    pub symbol: String,
    /// Art height as a multiple of the run's font size.
    #[serde(default = "one")]
    pub scale: f64,
    /// Extra raise in points (positive = up).
    #[serde(default)]
    pub baseline_shift: f64,
    /// The symbol art's visual bounds at its natural size, resolved from the document's symbols by
    /// [`crate::Document::resolve_inline_art`] (not saved). `None`: not resolved or missing; the
    /// layout then reserves a one-em square and nothing is drawn.
    #[serde(skip)]
    pub bounds: Option<Rect>,
}

fn one() -> f64 {
    1.0
}

fn fifty() -> f64 {
    50.0
}

impl InlineArt {
    pub fn new(symbol: impl Into<String>) -> Self {
        Self { symbol: symbol.into(), scale: 1.0, baseline_shift: 0.0, bounds: None }
    }
    /// `scale`, made finite and positive (untrusted input).
    pub fn safe_scale(&self) -> f64 {
        if self.scale.is_finite() && self.scale > 0.0 { self.scale.min(INLINE_MAX_SCALE) } else { 1.0 }
    }
    /// `baseline_shift`, made finite (untrusted input).
    pub fn safe_shift(&self) -> f64 {
        if self.baseline_shift.is_finite() { self.baseline_shift.clamp(-1e5, 1e5) } else { 0.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TextKind {
    /// Point type: anchored at the baseline origin of the first line.
    Point,
    /// Area type flowed inside `frame` (document coordinates, untransformed by `xf`).
    Area { frame: PathData },
    /// Type on a path, flowing from its start bracket `start` to its end bracket `end` (0..1 of the
    /// path length; no `end`: the end of the path, or once round a closed path). See
    /// [`TextKind::path_span`].
    OnPath {
        path: PathData,
        start: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<f64>,
    },
}

impl TextKind {
    /// Type on a path's span: from its start to its end bracket, as fractions of its path's length.
    /// Round a closed path the end may be past 1 (the span runs on past the path's start), a full
    /// turn without an end or with the end at the start. None for other type.
    pub fn path_span(&self) -> Option<(f64, f64)> {
        let TextKind::OnPath { path, start, end } = self else { return None };
        let s = fraction(*start);
        Some(if path.is_closed() {
            let run = end.map_or(0.0, |e| (fraction(e) - s).rem_euclid(1.0));
            (s, s + if run < 1e-9 { 1.0 } else { run })
        } else {
            (s, end.map_or(1.0, |e| fraction(e).max(s)))
        })
    }
}

/// Move anchors `refs` of `path`, in text space, by `d` in document space (`to_text`: the
/// document → text map). False when an anchor doesn't exist.
pub(super) fn move_anchors(path: &mut PathData, to_text: Affine, refs: &[(usize, usize)], d: Vec2) -> bool {
    // A distance: only the linear part of the document → text map applies.
    let local = to_text * d.to_point() - to_text * Point::ZERO;
    refs.iter().all(|&(si, ai)| path.anchor_mut(si, ai).map(|a| a.translate(local)).is_some())
}

/// `x` as a fraction of a path's length (0..1; 0 when not finite).
fn fraction(x: f64) -> f64 {
    if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 }
}

/// `path` run the other way, each point as far from its new start as it was from its old end
/// (the subpaths in reverse order, a closed one keeping its first anchor first).
pub(super) fn reverse_from_end(path: &mut PathData) {
    path.subpaths.reverse();
    for sp in &mut path.subpaths {
        sp.reverse();
        if sp.closed && !sp.anchors.is_empty() {
            sp.anchors.rotate_right(1);
        }
    }
}

/// Where ruby (furigana) text sits: above its base text in horizontal type, right of it in
/// vertical type (the Japanese convention), or below (left in vertical type).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RubyPosition {
    #[default]
    Above,
    Below,
}

/// How ruby text lines up over its base text when the two are different widths.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RubyAlign {
    /// Centred over the base text.
    #[default]
    Center,
    /// Starting at the base text's start (left in horizontal type, top in vertical).
    Start,
    /// Ending at the base text's end (right in horizontal type, bottom in vertical).
    End,
}

/// Ruby (furigana, phonetic guide): a reading set over a range of the text.
///
/// `start`/`end` are byte offsets into [`TextObject::plain_text`] (the same space as
/// [`crate::TextObject`] commands' `start`/`end`). The base text itself stays the text;
/// emptying the reading removes the ruby. New ruby is half the base text's size, centred, and
/// allowed to overhang neighbouring characters when the reading is longer than its base.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RubySpan {
    /// Byte offset of the first base character in the plain text.
    pub start: usize,
    /// Byte offset just past the last base character.
    pub end: usize,
    /// The reading, set in the base characters' font (its size comes from `size_percent`).
    pub text: String,
    #[serde(default)]
    pub position: RubyPosition,
    #[serde(default)]
    pub align: RubyAlign,
    /// Ruby size as a percentage of the base text's size, 1–100 (50 = half, the usual).
    #[serde(default = "fifty")]
    pub size_percent: f64,
    /// Let a reading wider than its base hang over the neighbouring characters instead of being
    /// squeezed to the base's width.
    #[serde(default = "yes")]
    pub overhang: bool,
}

impl RubySpan {
    /// Ruby over `start..end` with reading `text`, at the defaults (half size, centred, overhanging).
    pub fn new(start: usize, end: usize, text: impl Into<String>) -> Self {
        Self { start, end, text: text.into(), position: RubyPosition::Above, align: RubyAlign::Center, size_percent: 50.0, overhang: true }
    }
    /// `size_percent` made finite and usable: 1–100% (untrusted input).
    pub fn safe_size_percent(&self) -> f64 {
        if self.size_percent.is_finite() { self.size_percent.clamp(1.0, 100.0) } else { 50.0 }
    }
}
