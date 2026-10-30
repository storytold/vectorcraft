//! Native, editable CorelDRAW import. See the crate README for the supported subset.
//! No embedded preview is used as drawing content.
#![forbid(unsafe_code)]

mod bitmap;
mod bspline;
mod container;
mod corners;
mod drawing;
mod polygon;
mod reader;

#[cfg(any(test, feature = "synth"))]
pub mod synth;

use vectorcraft_doc::Document;

/// A drawing decoded from its native CDR records.
#[derive(Clone, Debug)]
pub struct Imported {
    pub document: Document,
    /// CDR format version multiplied by 100 (for example, X6 is 1600).
    pub version: u32,
    /// Features omitted or approximated while importing.
    pub warnings: Vec<String>,
}

/// Identify CDR RIFF signatures or ZIP archives containing a native CDR root.
/// ZIP recognition expands only a 12-byte header; import validates the full payload.
/// Recognition does not promise that the version or contents can be imported.
pub fn is_cdr(bytes: &[u8]) -> bool {
    container::recognize(bytes)
}

/// Read supported native CDR geometry, page structure, fills and outlines.
/// All input, expansion, nesting and record counts are bounded.
pub fn import(bytes: &[u8]) -> Result<Imported, String> {
    let (chunks, version) = container::load(bytes)?;
    drawing::import(&chunks, version)
}

#[cfg(test)]
mod tests;
