//! Integration tests: one test binary for the crate, one module per area.
// Unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod blend_export;
mod colour_glyphs;
mod container_export;
mod css_rules;
mod dash_align;
mod data_attributes;
mod effects_export;
mod encoding_profile_fonts;
mod focal;
mod font_weights;
mod foreign_attributes;
mod gradients;
mod ids;
mod import_details;
mod import_live;
mod import_text;
mod import_units;
mod interactivity;
mod mask_options;
mod non_scaling_stroke;
mod options;
mod prop_svg;
mod raster_paints;
mod roundtrip;
mod stroke_export;
mod svg_glyphs;
mod symbols_layers;
mod text_layout;
mod text_paint_order;
mod text_stroke;
mod title_control_chars;
