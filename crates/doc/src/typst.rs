//! Typst objects (Type › Typst…): a group whose art was typeset by [Typst](https://typst.app) from
//! the [`TypstSpec`] it keeps, so it can be edited and typeset again.

use serde::{Deserialize, Serialize};
use vectorcraft_geom::{Affine, Rect};

/// The longest source or preamble typeset (bytes). A file's longer one is kept as it is, but
/// editing refuses to typeset it.
pub const MAX_TYPST_SOURCE: usize = 1 << 20;

/// What a Typst object was typeset from, and where its art went.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypstSpec {
    /// The Typst markup typeset (`$ … $` for math).
    pub source: String,
    /// Markup read before the source: `#set` and `#import` rules, `#let` definitions.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preamble: String,
    /// The text size the source starts with (pt).
    pub font_size: f64,
    /// The typeset page: its art's own coordinates, before [`TypstSpec::xf`].
    pub page: Rect,
    /// Where the page is in the document: follows the object as it is moved, scaled or rotated, so
    /// typesetting it again keeps its place.
    pub xf: Affine,
}

impl TypstSpec {
    /// The object moved by `a`.
    pub fn transform(&mut self, a: Affine) {
        self.xf = a * self.xf;
    }
}
