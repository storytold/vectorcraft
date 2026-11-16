//! The Affinity reader is `craft-affinity` (storytold/craft-libs). This crate keeps its name so
//! the corpus tests and fuzz targets under it, and the CI job that runs them, still build.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub use craft_affinity::*;
