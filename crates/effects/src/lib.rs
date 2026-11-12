//! VectorCraft live effects (Illustrator's Effect menu).
//!
//! Effects live in appearance stacks as [`Effect`] `{id, params, visible}` records. This crate
//! interprets them:
//!
//! - **Geometry effects** (Distort & Transform, Path, Convert to Shape, Round Corners, Scribble,
//!   Warp) rewrite a path: [`apply_geometry`] evaluates them in stack order.
//! - **Raster effects** (Drop Shadow, Inner/Outer Glow, Feather, Gaussian Blur, Rasterize, and the
//!   Photoshop-style filters of [`pixel`]: Radial Blur, Smart Blur, Color Halftone, Crystallize, Mezzotint, Pointillize, Halftone Pattern,
//!   Unsharp Mask and Glowing Edges, and the SVG filters of [`svg_filter`]) are described by
//!   [`raster_effects`] and painted by the renderer; [`outset`] says how far they reach beyond
//!   the geometry.
//! - **Stroke geometry** ([`stroke`]): arrowheads, dash patterns and width profiles, shared by the
//!   renderer, the exporters and Outline Stroke.
//! - [`effect_catalog`] lists every effect with its menu path, parameter documentation and
//!   default values. Missing parameters always fall back to those defaults.
//! - [`reshape`] applies geometry effects to type, images, symbol instances and live objects
//!   through their outlines.
//! - [`evaluate_container`] turns a group's or layer's own fills, strokes and geometry effects into
//!   art painting its members (its raster effects then apply to the composite).
//! - [`clip_outline`] is the region a clip group clips to, shared by the renderer and the SVG and
//!   PDF writers.
//! - [`preview`] measures objects as they are drawn, with their geometry effects (Use Preview
//!   Bounds).
//! - **Plug-in effects** (`plugin.<plug-in id>`, Effect › Plug-ins) are geometry effects run by an
//!   installed WebAssembly plug-in (`vectorcraft-plugins`); [`effect_info`] describes them like
//!   built-in ones, and an effect whose plug-in isn't installed leaves the geometry as it is.
//! - **Colour adjustments** (Brightness/Contrast, Curves, Levels, Hue/Saturation, Shift to Color,
//!   Temperature/Tint) recolour what an object paints with: [`adjust`] evaluates them for the
//!   renderer and the exporters.
//!
//! Everything is deterministic: "random" effects (Roughen, Tweak, Scribble) use a seeded hash
//! noise (`seed` parameter, default 0); the random Pixelate filters hash their pattern's cells,
//! anchored at the object's centre.
#![forbid(unsafe_code)]

pub mod addons;
mod adjust;
mod bake;
mod clip;
mod distort;
mod group;
mod lighting;
mod lighting_presets;
mod live;
mod marks;
pub mod pixel;
pub mod preview;
mod raster;
mod reshape;
mod revolve;
mod solids;
pub mod stroke;
mod stylize;
pub mod svg_filter;
pub mod trap;
mod util;
mod warp;

#[cfg(test)]
mod tests;

use serde_json::{Map, Value, json};
use vectorcraft_doc::{Appearance, AppearanceItem, Effect, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{BezPath, FillRule, PathData, Rect};

pub use adjust::{
    ADJUSTMENTS, CHANNELS, ColorMap, ImageHook, adjust, adjust_in_document, color_map, curve_at, curve_points, has_adjustment, is_adjustment,
    is_channel,
};
pub use bake::{StrokeArt, bake_appearance, bake_document, bake_type_appearance, expand_art, expand_leaf, fresh_ids, needs_bake};
pub use clip::clip_outline;
pub use group::{
    OutlineHook, PATHFINDER_EFFECTS, compound_shape_area, compound_shape_art, evaluate_container, has_container_appearance, has_pathfinder,
    is_pathfinder, member_leaves, member_shapes, paints, pathfinder_children, shape_art,
};
pub use lighting_presets::{LIGHTING_PRESETS, LightingPreset, lighting_preset_params, matching_lighting_preset};
pub use live::{expand_live, expand_live_deep, expanded_live_group, outline_object, text_outliner};
pub use marks::{CROP_MARKS, crop_marks_art, has_crop_marks};
pub use pixel::{PIXEL_EFFECTS, PixelFx, PixelSpace};
pub use raster::{RASTERIZE, RasterFx, Rasterize, color_param, outset, raster_effects};
pub use reshape::{
    SYMBOL_ART_MAX_NODES, expand_outlined, gives_image_shape, has_gradient_copies, has_item_geometry, has_painted_effects, has_transform_art,
    has_transform_copies, has_transform_options, image_paint, needs_outline, outline_art, outline_text, reshape,
};
pub use revolve::{
    CLASSIC_REVOLVE, REVOLVE, has_revolve, is_revolve, revolve_art, revolve_art_in, revolve_options, validate_revolve, validate_revolve_in,
};
pub use solids::{
    CLASSIC_EXTRUDE, CLASSIC_ROTATE, EXTRUDE, INFLATE, ROTATE, ROTATE_SURFACES, ROTATION_PRESETS, SHADING_COLORS, SOLIDS, SolidMesh, THREE_D_TYPES,
    classic_extrude_options, classic_shading, extrude_options, has_solid, inflate_options, is_classic_three_d, is_three_d, solid_art, solid_art_in,
    three_d_meshes, three_d_meshes_in, three_d_meshes_of, three_d_meshes_of_in, three_d_params_as, validate_solid, validate_solid_in,
    validate_three_d, validate_three_d_at, validate_three_d_at_in, validate_three_d_in, validate_three_d_params, view_options,
};
pub use stylize::{ScribbleSetting, scribble_setting_id, scribble_settings};
pub use svg_filter::SVG_FILTER;
pub use trap::TRAP;
pub use vectorcraft_three_d::Mesh;
pub use warp::{WARP_NET, WarpNet, WarpStyle, map_version, warp_point, warp_version};

mod catalog;
mod geometry;
mod params;

pub use catalog::{EffectInfo, Lengths, WARP_STYLES, effect_catalog, effect_info, is_length, length_range, plugin_effects, raster_px_range};
pub(crate) use geometry::{Budget, apply_one};
pub use geometry::{
    Carry, GeomContext, apply_geometry, apply_geometry_bez, apply_geometry_bez_carrying, apply_geometry_carrying, apply_geometry_with,
};
#[cfg(test)]
pub(crate) use geometry::{MAX_EFFECT_LINES, apply_geometry_within};
pub use params::{
    any_plugin_paints_output, default_params, has_geometry, has_painted_plugin_effect, has_plugin_effect, is_geometry, is_raster, merged_params,
    new_effect, plugin_revision, scale_effect, upgrade_params,
};
