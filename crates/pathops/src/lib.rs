//! VectorCraft path operations: booleans (Pathfinder), shape builder regions, offset, outline stroke,
//! simplify and the other Object → Path commands.
//!
//! Booleans are curve-preserving: `linesweeper`'s robust sweep-line works directly on cubic
//! Béziers, and the pieces it splits curves into are refitted afterwards so results carry few
//! anchors. See the crate README for the API overview.
#![forbid(unsafe_code)]

mod boolean;
mod budget;
mod edit;
mod fit;
mod offset;
mod pathfinder;
mod planar;
mod trap;

pub use boolean::{
    BoolOp, DEFAULT_PRECISION, area, boolean, boolean_n, normalize, try_area, try_boolean, try_boolean_n, try_normalize, try_unite_all,
    try_unite_all_with, unite_all, unite_all_with,
};
pub use edit::{
    AverageAxis, SIMPLIFY_LEVEL, SMOOTH_AUTO_LEVEL, SimplifiedPart, SimplifyOptions, add_anchor_points, auto_simplify_level, average, join,
    remove_anchor, remove_redundant_points, simplify, simplify_level_tolerance, simplify_part, simplify_with, smooth, smooth_level, split_into_grid,
};
pub use offset::{Cap, Join, offset_path, outline_stroke, stroke_region};
pub use pathfinder::{FaceMerger, PathfinderOp, Region, Shape, merge_regions, pathfinder, pathfinder_with, region_at, regions};
pub use planar::{
    BuilderArrangement, SHAPE_BUILDER_MAX_GAP, SHAPE_BUILDER_MAX_SEGMENTS, cut_out, encloses_area, find_gaps, has_open_path, interior_point,
    live_paint, live_paint_with_gaps, run_around, shape_builder, shape_builder_with_gap,
};
pub use trap::{TRAP_HEIGHT_WIDTH_RANGE, TRAP_MAX_WIDTH, TRAP_MIN_WIDTH, trap_band};

/// Errors from fallible operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PathOpsError {
    #[error("input contains NaN or infinite coordinates")]
    NonFinite,
    #[error("input path could not be closed")]
    OpenPath,
    #[error("the shapes are too degenerate to combine")]
    Degenerate,
    #[error("the shapes cross themselves and each other too often to combine")]
    TooComplex,
}
