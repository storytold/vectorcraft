//! Integration tests: one test binary for the crate, one module per area.
// Unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../support/affinity_docs.rs"]
mod affinity_docs;
#[path = "../../../affinity/tests/support/text_fixture.rs"]
mod text_fixture;

mod affinity_features;
mod affinity_local;
mod artboard_click;
mod beyond_canvas;
mod clear_commands;
mod compound_attributes;
mod dimensions;
mod eyedropper_sources;
mod find_replace_paras;
mod inline_text;
mod known_bugs;
mod lossy_overwrite;
mod model_based;
mod param_keys;
mod pen_click_after_smooth;
mod pen_endpoint_picking;
mod ruler_origin;
mod selection_commands;
mod stroke_visibility;
mod text_stroke_scale;
mod three_d_light_color;
