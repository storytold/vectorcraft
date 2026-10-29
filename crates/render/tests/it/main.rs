//! Integration tests: one test binary for the crate, one module per area.
// Unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod dash_align;
mod golden;
mod gradient_alpha;
mod gradient_text;
mod radial_focal;
mod stroke_geometry;
