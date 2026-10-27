//! Text objects (model only; layout and glyph outlines live in `vectorcraft-text`).

use serde::{Deserialize, Serialize};
use vectorcraft_color::{Color, Paint};
use vectorcraft_geom::{Affine, PathData, Point, Rect, Vec2};

use crate::appearance::{Appearance, AppearanceItem, Dash, FillLayer, LineCap, LineJoin, StrokeAlign, StrokeLayer};

mod area;
mod char_style;
mod object;
mod para_style;
mod runs;
mod tabs_lists;

pub use area::{AreaFit, AreaOptions, PathAlign, PathEffect, TextWrap, VerticalAlign, WrapShape};
use char_style::yes;
pub use char_style::{CharAlign, CharPosition, CharStyle, Digits, ScriptMetrics};
pub use para_style::{
    Burasagari, Composer, FirstBaseline, HyphenationOptions, JustificationOptions, Justify, Kashida, Kinsoku, LeadingModel, Mojikumi, ParaDirection,
    ParaStyle, SingleWordJustify,
};
pub use runs::{INLINE_CHAR, INLINE_MAX_SCALE, InlineArt, RubyAlign, RubyPosition, RubySpan, TextKind, TextRun, TextStyleDef};
pub use tabs_lists::{Bullet, DEFAULT_TAB_INTERVAL, LIST_HANGING_INDENT, NumberStyle, ParaList, TabAlign, TabStop};

/// A text object. `runs` split into paragraphs at `\n`.
///
/// Paragraph attributes: `para` is paragraph 0's (and, while `paras` is empty, every
/// paragraph's). Invariant (kept by [`TextObject::normalize_paras`]): `paras` is either empty —
/// every paragraph uses `para` — or holds one style per paragraph with `paras[0] == para` and at
/// least two different styles. Readers that only know `para` see the first paragraph's style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextObject {
    /// Top-to-bottom, right-to-left writing. Defaults to horizontal for old documents.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub vertical: bool,
    pub kind: TextKind,
    /// Maps text space (origin = first baseline start for point type) to the document.
    pub xf: Affine,
    pub runs: Vec<TextRun>,
    /// Ruby (furigana / phonetic guide) over byte ranges of the plain text; empty = none. Kept on
    /// the object, so the runs (and their styles) stay the text itself.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ruby: Vec<RubySpan>,
    #[serde(default)]
    pub para: ParaStyle,
    /// Per-paragraph attributes (see the type's docs); empty = every paragraph uses `para`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paras: Vec<ParaStyle>,
    /// Threaded text: the first paragraph continues from the frame before this one in its thread,
    /// so its first line here has no first-line indent and no list marker. The engine sets it for
    /// every frame of a thread when the story flows through them and when a file opens, and clears
    /// it on a frame that leaves its thread and on a copy. Not saved.
    #[serde(skip)]
    pub first_para_continues: bool,
    /// Area Type Options (area type only).
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub area: AreaOptions,
    /// Type on a Path effect (type on a path only).
    #[serde(default, rename = "pathEffect", skip_serializing_if = "crate::skip::is_default")]
    pub path_effect: PathEffect,
    /// Type on a Path Options › Align to Path (type on a path only).
    #[serde(default, rename = "pathAlign", skip_serializing_if = "crate::skip::is_default")]
    pub path_align: PathAlign,
    /// Type on a Path Options › Spacing in points (type on a path only): glyphs are spaced as if
    /// set this far above the path, which closes them up round the outside of a curve and opens
    /// them up round the inside.
    #[serde(default, rename = "pathSpacing", skip_serializing_if = "crate::skip::is_default")]
    pub path_spacing: f64,
    /// Wrap objects above this area type, resolved by the engine after each edit (text space).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wrap: Vec<WrapShape>,
    /// Type › Optical Margin Alignment: in every paragraph, Latin punctuation and the slanted
    /// sides of letters such as A and W at a line's start or end hang outside the margin there
    /// (point and area type, and type on a path at its brackets; see the layout's `optical_hangs`).
    /// Off for new type and for documents from before it, and saved only when on.
    #[serde(default, rename = "opticalMarginAlignment", skip_serializing_if = "crate::skip::is_default")]
    pub optical_margin: bool,
    /// Character panel menu › System Layout, copied from the document: when on, each character
    /// that takes space advances by a whole number of points. Off by default, and saved only when
    /// on. The document setting is [`crate::DocSetup::system_layout`].
    #[serde(default, rename = "systemLayout", skip_serializing_if = "crate::skip::is_default")]
    pub system_layout: bool,
    /// Character panel menu › Fractional Widths: spacing between characters uses fractions of a
    /// point. On for new type and for documents from before it, and saved only when off. Off, each
    /// character that takes any space advances by a whole number of points, at least one, so small
    /// type does not run together. The setting is the whole type object, not a range of characters.
    /// <https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/turn-fractional-character-widths-off-or-on.html>
    #[serde(default = "yes", rename = "fractionalWidths", skip_serializing_if = "crate::skip::is_true")]
    pub fractional_widths: bool,
    /// Threaded text: the numbered list paragraphs the story's earlier frames count before this
    /// frame's first paragraph ([`ParaList::next_marker`]), so one count runs through the story.
    /// The engine sets it whenever it re-flows a thread. Type that is not threaded has 0, and the
    /// file saves the count only when it is not 0.
    #[serde(default, rename = "listCount", skip_serializing_if = "crate::skip::is_default")]
    pub list_count: u32,
    /// Cached layout bounds in text space, filled in by the layout engine (not serialized).
    #[serde(skip)]
    pub cached_bounds: Option<Rect>,
    /// Cached baselines in text space, start to end, one per line that holds characters (a
    /// vertical column's centre line), filled in with `cached_bounds` (not serialized).
    #[serde(skip)]
    pub cached_baselines: Vec<(Point, Point)>,
}

/// Conversion from stored character attributes to document-space type controls.
/// Point size follows the transformed em-height axis; horizontal scale is the ratio of
/// the two transformed em axes. Rotation, reflection and shear remain in the affine.
#[derive(Clone, Copy, Debug)]
pub struct TextStyleScale {
    pub points: f64,
    pub horizontal: f64,
}

#[cfg(test)]
mod tests;
