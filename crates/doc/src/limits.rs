use super::NodeId;

/// Coordinates beyond this many points exceed the supported document range.
pub const MAX_COORD: f64 = 4.0e6;

/// Adobe Illustrator's standard drawing area (canvas) side length in points ($2^{14} - 1$ = 16,383 pt = 5,779.558 mm).
pub const DRAWING_AREA_SIDE: f64 = 16383.0;
/// Half of the standard drawing area side (8,191.5 pt).
pub const DRAWING_AREA_HALF: f64 = DRAWING_AREA_SIDE / 2.0;
/// Large Canvas side length (10x standard canvas = 163,830 pt).
pub const LARGE_CANVAS_SIDE: f64 = DRAWING_AREA_SIDE * 10.0;
/// Half of the large canvas side (81,915 pt).
pub const LARGE_CANVAS_HALF: f64 = LARGE_CANVAS_SIDE / 2.0;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DocError {
    #[error("no such object {0}")]
    NoNode(NodeId),
    #[error("object {0} cannot have children")]
    NotContainer(NodeId),
    #[error("{0}")]
    Invalid(String),
}

/// The document version new documents get. Version 2: a positive Twist angle turns clockwise
/// ([`crate::twist`]). Version 3: Color Halftone's Max Radius is in pixels of the raster effects
/// resolution (older documents are converted on opening,
/// [`crate::Document::migrate_halftone_radii`]).
pub const FORMAT_VERSION: u32 = 3;

/// Node, slice and asset ids are handed out below this: no document gets near it by counting, so
/// larger ids in a file are damage (or made by hand) and the counter never follows them (#1718).
pub(crate) const MAX_NEXT_ID: u64 = 1 << 62;
