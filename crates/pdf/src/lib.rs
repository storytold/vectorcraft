//! VectorCraft PDF export and import.
//!
//! - [`export`] writes one PDF page per artboard with `krilla`: vector paths (fills, strokes with
//!   caps/joins/miter/dashes, inside/outside alignment as clips, non-zero/even-odd; arrowheads,
//!   width profiles, fitted or dotted dashes as the canvas's filled outlines and brushed strokes
//!   as their brush art), opacity and blend modes (transparency groups),
//!   clip groups, linear/radial gradients (shadings), pattern fills and strokes as their tiles and
//!   freeform and Perceptual gradients and dithered gradient fills as images at the raster
//!   effects resolution (clipped to what they paint), embedded images (resampled and
//!   compressed as the Compression settings say) and text as outlined glyph paths, or as real
//!   text in embedded subset fonts (Advanced). Colours are converted, tagged with ICC profiles and
//!   given an output intent as the Output settings say. Hidden objects,
//!   guides and template layers are skipped, and so are non-printing layers unless asked for.
//!   Layers and sublayers can be PDF layers (optional content groups with their visibility, print
//!   state and lock), and overprinting fills and strokes overprint (Advanced).
//!   Each page is its artboard (the trim box) grown by the bleed (the bleed box) and by the
//!   printer's marks around it (the media box), drawn in Registration. [`PdfSettings`] is the Save PDF
//!   dialog's model (standard, compatibility, General, Compression, Marks and Bleeds, Output,
//!   Advanced, Security); options the writer doesn't apply yet come back as warnings. With Preserve
//!   Editing the native document rides along as an embedded file ([`editing()`]). Each page can
//!   carry a thumbnail ([`Thumbnail`], drawn by the caller), and Fast Web View writes a linearised
//!   file (still linearised when encrypted). A PDF 1.3 file ([`PdfSettings::pdf13`]) must be flat:
//!   the app flattens the document first, and the writer refuses transparency that is left.
//! - [`import`] reads PDF (and PDF-compatible `.ai`) pages with `hayro-interpret` into a
//!   [`Document`]: one artboard per page, and a layer per page or per optional content group
//!   (with its visibility, print state and lock; art that is off comes in as a hidden layer),
//!   paths with fill/stroke, clip groups, transparency groups (with isolation and knockout), soft
//!   masks → opacity masks, axial/radial shadings → gradients (stop opacity and unextended ends
//!   kept), mesh shadings → gradient meshes, tiling patterns → pattern swatches, images (JPEG
//!   passthrough, others re-encoded as PNG) and text as point type (or glyph outlines, see
//!   [`TextAs`]). Colours keep their model: CMYK, Gray, and spot
//!   inks (Separation, DeviceN) as spot swatches at a tint; a file painted mostly in CMYK opens as a
//!   CMYK document. [`ImportOptions`] pick the pages, the box each
//!   artboard gets ([`CropTo`]) and the password; [`info`] lists the pages and their boxes.
//!   The native document a PDF carries comes back too ([`ImportReport::native`], [`editing()`]),
//!   and [`file_info()`] reads its File Info (the information dictionary and the XMP packet).
//! - Presets: named settings, the built-in ones generated in code ([`builtin_presets`]).
//! - [`print()`] lays a document out on paper as File → Print does ([`PrintSettings`]) and writes
//!   the job as a print-ready PDF: composite or one page per ink.
//! - Security: the open and permissions passwords encrypt the written file with the standard
//!   security handler, the algorithm following the compatibility ([`Encryption`]); either password
//!   opens an encrypted file.
#![forbid(unsafe_code)]

mod editing;
mod encrypt;
mod export;
mod file_info;
mod forms;
mod images;
mod import;
mod import_color;
mod import_image;
mod import_lines;
mod import_mask;
mod import_scan;
mod import_shading;
mod import_text;
mod lab_spot;
mod linearize;
mod marks;
mod output;
mod pages;
mod patch;
mod pdfx;
mod post;
mod presets;
mod print;
mod settings;
mod streams;
mod syntax;

pub use editing::{EDITING_FILE, Editing, LEGACY_EDITING_FILE, editing, editing_with};
pub use encrypt::Encryption;
pub use export::{export, export_with_report, page_areas};
pub use file_info::{FileInfo, file_info};
pub use import::{
    INVISIBLE_TEXT_NOTE, OFF_ARTBOARD_NOTE, OUTLINED_TEXT_NOTE, PAST_PAGE_NOTE, UNMATCHED_GLYPHS_NOTE, UNNAMED_CHARACTERS_NOTE, holds_outlines,
    import, import_with_report, is_glyph_outlines, is_outlines_note,
};
pub use pages::{PageInfo, PdfInfo, has_illustrator_data, illustrator_data, info, is_postscript};
pub use post::{THUMBNAIL_SIZE, Thumbnail};
pub use presets::*;
pub use print::*;
pub use settings::*;

use vectorcraft_doc::Document;

/// Export options: the Save PDF settings plus what one export run decides (pages, title, date).
#[derive(Clone, Debug, Default)]
pub struct PdfOptions {
    pub settings: PdfSettings,
    /// 0-based artboard indices to export, in page order; `None` = all artboards.
    pub artboards: Option<Vec<usize>>,
    /// Document title for the metadata; `None` = the document's title.
    pub title: Option<String>,
    /// Ask PDF viewers to display the document title instead of the file name.
    pub display_doc_title: bool,
    /// Creation date as Unix seconds (UTC); `None` = now (native) / omitted (wasm). PDF/A needs a date.
    pub created: Option<i64>,
    /// Modification date as Unix seconds (UTC); `None` = the creation date.
    pub modified: Option<i64>,
    /// The native document (`.vectorcraft` bytes) Preserve Editing embeds.
    pub native: Option<Vec<u8>>,
    /// The pages drawn small, in page order: what Embed Page Thumbnails embeds (the writer
    /// doesn't draw; [`page_areas`] says what each page shows).
    pub thumbnails: Vec<Thumbnail>,
}

impl PdfOptions {
    /// Default options with uncompressed content streams (readable operators, for tests and
    /// debugging).
    pub fn uncompressed() -> Self {
        let compression = CompressionSettings { compress_text: false, ..Default::default() };
        Self { settings: PdfSettings { compression, ..Default::default() }, ..Default::default() }
    }
}

/// Import options.
#[derive(Clone, Debug)]
pub struct ImportOptions {
    /// Import at most this many pages (of those picked); `None` = all of them.
    pub max_pages: Option<usize>,
    /// Horizontal gap in points between the artboards created for consecutive pages.
    pub artboard_gap: f64,
    /// 0-based pages to import, in this order; `None` = every page.
    pub pages: Option<Vec<usize>>,
    /// The page box each artboard gets.
    pub crop: CropTo,
    /// The (user) password of an encrypted PDF.
    pub password: Option<String>,
    /// What text becomes.
    pub text_as: TextAs,
    /// Optional content groups become layers (else each page is one layer, without the art
    /// that is off).
    pub layers: bool,
    /// The file is an editor's own document (`.ai`): RGB in its `ICCBased` spaces keeps its numbers
    /// (see `Colors::uncalibrated_rgb`). Off for PDFs, whose profiles stand.
    pub uncalibrated_rgb: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            max_pages: None,
            artboard_gap: 36.0,
            pages: None,
            crop: CropTo::default(),
            password: None,
            text_as: TextAs::default(),
            layers: true,
            uncalibrated_rgb: false,
        }
    }
}

settings::choice! {
    /// What imported text becomes.
    TextAs {
        /// Point type, one object per line run, in the file's fonts (by name).
        Text = "text", "Text";
        /// Paths of the glyphs' outlines (looks the same without the fonts).
        Outlines = "outlines", "Outlines";
    } default Text
}

settings::choice! {
    /// The page box an imported or placed page is cropped to: its artboard (or placed frame).
    CropTo {
        /// The bounds of the page's art.
        Bounding = "bounding", "Bounding Box";
        Art = "art", "Art";
        /// The visible page area (what viewers show).
        Crop = "crop", "Crop";
        /// The finished page: an import takes the bleed box round it as the document's bleed.
        Trim = "trim", "Trim";
        Bleed = "bleed", "Bleed";
        /// The whole sheet.
        Media = "media", "Media";
    } default Crop
}

/// Result of an import with non-fatal warnings (unsupported features, skipped content).
#[derive(Clone, Debug)]
pub struct ImportReport {
    pub document: Document,
    pub warnings: Vec<String>,
    /// The native document the PDF carries (written with Preserve Editing), if any.
    pub native: Option<Editing>,
    /// Every page shows only text, over an editor's private data: that data, where it reads,
    /// is the file's art (a file saved without its PDF part shows a notice there instead).
    pub only_text: bool,
    /// The size in points of a unit of the first page read's user space: the `UserUnit` the PDF
    /// reader applies to that page (1 for a page without one). The page came in at its size in
    /// points, its numbers times this.
    pub user_unit: f64,
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
    /// The caller asked for the export to stop ([`vectorcraft_doc::cancel`]).
    #[error("cancelled")]
    Cancelled,
    #[error("cannot read PDF: {0}")]
    Parse(String),
    #[error("the PDF has no pages")]
    NoPages,
    #[error("invalid PDF setting: {0}")]
    BadSetting(String),
    #[error("the PDF is password-protected: give its password")]
    NeedsPassword,
    #[error("the PDF password is wrong")]
    WrongPassword,
    #[error("page {0} does not exist (the PDF has {1})")]
    BadPage(usize, usize),
    #[error(
        "this is a PostScript file (an .ai saved in an older format or without PDF compatibility, or an EPS file), not a PDF: File › Open and File › Place read it as EPS"
    )]
    PostScript,
    #[error(
        "the file holds only a placeholder page, not its art (it was saved without PDF compatibility): save it again with PDF compatibility on, or as PDF or SVG"
    )]
    PlaceholderOnly,
}

impl From<craft_pdf::write::WriteError> for PdfError {
    fn from(error: craft_pdf::write::WriteError) -> Self {
        Self::Write(error.to_string())
    }
}

#[cfg(test)]
use tests::{import_fidelity as tests_import_fidelity, live_text as tests_live_text, stroke as tests_stroke};
#[cfg(test)]
mod tests;
