//! Printing beyond the system spooler: network printers over WS-Print and IPP.
//!
//! `network` is the generic half (one printer behind a private craft-print registry, two phases:
//! the printer accepts the document, then the job is followed; nothing is ever sent twice) and
//! `raster` the VectorCraft half (the converter that turns a print-ready PDF into PWG raster for a
//! printer that lists no PDF). The spooler itself stays in craft-print and the app's
//! `printing.rs`. Not built for wasm32: the crate is empty there.
#![cfg(not(target_arch = "wasm32"))]

pub mod network;
pub mod raster;
