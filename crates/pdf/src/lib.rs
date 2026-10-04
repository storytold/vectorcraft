//! VectorCraft PDF export and import.
//!
//! - [`export`] writes one PDF page per artboard with `krilla`: vector paths (fills, strokes with
//!   caps/joins/miter/dashes, inside/outside alignment as clips, non-zero/even-odd; arrowheads,
//!   width profiles, fitted or dotted dashes as the canvas's filled outlines and brushed strokes
//!   as their brush art), opacity and blend modes (transparency groups),
//!   clip groups, linear/radial gradients (shadings), embedded images and text as outlined glyph
//!   paths. Hidden objects, guides and template layers are skipped.
//! - [`import`] reads PDF (and PDF-compatible `.ai`) pages with `hayro-interpret` into a
//!   [`Document`]: one artboard and one layer per page, paths with fill/stroke, clip groups,
//!   transparency groups, axial/radial shadings → gradients, images (JPEG passthrough, others
//!   re-encoded as PNG) and text as glyph outlines.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod export;
mod import;

pub use export::{export, export_with_report};
pub use import::{import, import_with_report};

use vectorcraft_doc::Document;

/// PDF standard / version to target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Compatibility {
    /// PDF 1.4.
    Pdf14,
    /// PDF 1.5.
    Pdf15,
    /// PDF 1.6.
    Pdf16,
    /// PDF 1.7. The default.
    #[default]
    Pdf17,
    /// PDF 2.0.
    Pdf20,
    /// PDF/A-2b (archival; validated by krilla).
    PdfA2b,
    /// PDF/X-4 — not supported by the writer yet; export returns [`PdfError::Unsupported`].
    PdfX4,
}

/// Export options.
#[derive(Clone, Debug)]
pub struct PdfOptions {
    /// 0-based artboard indices to export, in page order; `None` = all artboards.
    pub artboards: Option<Vec<usize>>,
    pub compatibility: Compatibility,
    /// Compress content streams (Flate).
    pub compress: bool,
    /// Document title for the metadata; `None` = the document's title.
    pub title: Option<String>,
    /// Creation date as Unix seconds (UTC); `None` = now (native) / omitted (wasm). PDF/A needs a date.
    pub created: Option<i64>,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self { artboards: None, compatibility: Compatibility::Pdf17, compress: true, title: None, created: None }
    }
}

/// Import options.
#[derive(Clone, Debug)]
pub struct ImportOptions {
    /// Import at most this many pages (from the first); `None` = all pages.
    pub max_pages: Option<usize>,
    /// Horizontal gap in points between the artboards created for consecutive pages.
    pub artboard_gap: f64,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self { max_pages: None, artboard_gap: 36.0 }
    }
}

/// Result of an import with non-fatal warnings (unsupported features, skipped content).
#[derive(Clone, Debug)]
pub struct ImportReport {
    pub document: Document,
    pub warnings: Vec<String>,
}

/// Result of an export with non-fatal warnings (features approximated or dropped).
#[derive(Clone, Debug)]
pub struct ExportReport {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PdfError {
    #[error("the document has no artboards")]
    NoArtboards,
    #[error("artboard {0} does not exist")]
    BadArtboard(usize),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("PDF writer error: {0}")]
    Write(String),
    #[error("cannot read PDF: {0}")]
    Parse(String),
    #[error("the PDF has no pages")]
    NoPages,
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_dashalign;
#[cfg(test)]
mod tests_fx;
#[cfg(test)]
mod tests_stroke;
#[cfg(test)]
mod tests_strokeout;
