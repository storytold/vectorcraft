//! Save PDF settings: the serde model a preset stores and `document.exportPdf` takes (camelCase
//! JSON; every field is optional and falls back to its default), the checks that refuse settings
//! the writer can't honour, and the warnings for options it accepts but doesn't apply yet.

use serde::{Deserialize, Serialize};

use crate::PdfError;

/// A closed list of choices (a settings enum), for UIs that show it as a dropdown.
pub trait Choice: Copy + 'static {
    /// Every choice, in display order.
    const ALL: &'static [Self];
    /// The JSON ids, in display order.
    const IDS: &'static [&'static str];
    /// The labels shown in the PDF dialogs, in display order.
    const LABELS: &'static [&'static str];
}

/// A closed list of choices: a serde enum (its JSON ids) with a UI label per variant.
macro_rules! choice {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident = $id:literal, $label:literal;)+ } default $d:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        pub enum $name {
            $($(#[$vm])* #[serde(rename = $id)] $v,)+
        }

        impl $crate::Choice for $name {
            const ALL: &'static [Self] = &[$(Self::$v),+];
            const IDS: &'static [&'static str] = &[$($id),+];
            const LABELS: &'static [&'static str] = &[$($label),+];
        }

        impl $name {
            /// The label shown in the PDF dialogs.
            pub fn label(self) -> &'static str {
                match self {
                    $(Self::$v => $label,)+
                }
            }

            /// The JSON id.
            pub fn id(self) -> &'static str {
                match self {
                    $(Self::$v => $id,)+
                }
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::$d
            }
        }
    };
}
pub(crate) use choice;

choice! {
    /// The PDF standard the file conforms to.
    Standard {
        None = "none", "None";
        /// PDF/A-2b (archival), validated by the writer.
        PdfA2b = "pdfA2b", "PDF/A-2b";
        PdfX1a = "pdfX1a", "PDF/X-1a:2001";
        PdfX3 = "pdfX3", "PDF/X-3:2002";
        PdfX4 = "pdfX4", "PDF/X-4:2010";
    } default None
}
impl Standard {
    /// A file of this standard can declare PDF version `c`: PDF/A-2 is based on PDF 1.7 (so it
    /// can't be PDF 2.0, nor PDF 1.3, which has no transparency), PDF/X-4 on PDF 1.6, and PDF/X-1a
    /// and PDF/X-3 on PDF 1.3 (PDF 1.4 is accepted too, and written as 1.3).
    pub fn allows(self, c: Compatibility) -> bool {
        use Compatibility::*;
        match self {
            Self::None => true,
            Self::PdfA2b => !matches!(c, Pdf13 | Pdf20),
            Self::PdfX1a | Self::PdfX3 => matches!(c, Pdf13 | Pdf14),
            Self::PdfX4 => matches!(c, Pdf14 | Pdf15 | Pdf16),
        }
    }

    /// The PDF version a file of this standard is (what choosing it sets).
    pub fn version(self) -> Compatibility {
        match self {
            Self::None | Self::PdfA2b => Compatibility::Pdf17,
            Self::PdfX1a | Self::PdfX3 => Compatibility::Pdf13,
            Self::PdfX4 => Compatibility::Pdf16,
        }
    }
}

choice! {
    /// The PDF version written in the file header.
    Compatibility {
        /// No transparency: it is flattened (see [`PdfSettings::pdf13`]).
        Pdf13 = "1.3", "PDF 1.3";
        Pdf14 = "1.4", "PDF 1.4";
        Pdf15 = "1.5", "PDF 1.5";
        Pdf16 = "1.6", "PDF 1.6";
        Pdf17 = "1.7", "PDF 1.7";
        Pdf20 = "2.0", "PDF 2.0";
    } default Pdf17
}

impl Compatibility {
    /// The version has PDF layers (optional content, PDF 1.5).
    pub fn has_layers(self) -> bool {
        matches!(self, Self::Pdf15 | Self::Pdf16 | Self::Pdf17 | Self::Pdf20)
    }

    /// The version has a page's `UserUnit` (PDF 1.6).
    pub fn has_user_unit(self) -> bool {
        matches!(self, Self::Pdf16 | Self::Pdf17 | Self::Pdf20)
    }
}

choice! {
    /// How images above the threshold resolution are resampled.
    Downsample {
        None = "none", "No downsampling";
        Average = "average", "Average";
        Subsample = "subsample", "Subsample";
        Bicubic = "bicubic", "Bicubic";
    } default None
}

choice! {
    /// Compression of colour and greyscale images.
    ImageCodec {
        None = "none", "None";
        Zip = "zip", "ZIP";
        Jpeg = "jpeg", "JPEG";
        Jpeg2000 = "jpeg2000", "JPEG 2000";
        /// JPEG images stay JPEG, the others are compressed losslessly.
        Auto = "auto", "Automatic";
    } default Auto
}

choice! {
    /// JPEG image quality.
    JpegQuality {
        Minimum = "minimum", "Minimum";
        Low = "low", "Low";
        Medium = "medium", "Medium";
        High = "high", "High";
        Maximum = "maximum", "Maximum";
    } default Maximum
}

choice! {
    /// Compression of 1-bit (monochrome) images.
    MonoCodec {
        None = "none", "None";
        CcittG3 = "ccittG3", "CCITT Group 3";
        CcittG4 = "ccittG4", "CCITT Group 4";
        Zip = "zip", "ZIP";
        RunLength = "runLength", "Run Length";
    } default Zip
}

choice! {
    /// The style of printer's marks.
    MarkKind {
        Roman = "roman", "Roman";
        Japanese = "japanese", "Japanese";
    } default Roman
}

choice! {
    /// Colour conversion on output.
    ColorConversion {
        None = "none", "No conversion";
        Destination = "destination", "Convert to destination";
        /// Convert only colours whose profile differs from the destination's.
        PreserveNumbers = "preserveNumbers", "Convert to destination (preserve numbers)";
    } default None
}

choice! {
    /// Which ICC profiles the file embeds.
    ProfileInclusion {
        None = "none", "Don't include profiles";
        All = "all", "Include all profiles";
        Destination = "destination", "Include destination profile";
        TaggedSource = "taggedSource", "Include tagged source profiles";
    } default None
}

choice! {
    /// What happens to overprinting fills and strokes.
    Overprint {
        Preserve = "preserve", "Preserve";
        Discard = "discard", "Discard";
    } default Preserve
}

choice! {
    /// Printing allowed by the permissions.
    Printing {
        None = "none", "None";
        Low = "low", "Low resolution (150 ppi)";
        High = "high", "High resolution";
    } default High
}

choice! {
    /// Changes allowed by the permissions.
    Changes {
        None = "none", "None";
        Pages = "pages", "Inserting, deleting and rotating pages";
        Forms = "forms", "Filling in form fields and signing";
        Comments = "comments", "Commenting, filling in form fields and signing";
        Any = "any", "Any except extracting pages";
    } default Any
}

/// Save PDF settings (the dialog's sections; what a preset stores).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PdfSettings {
    pub standard: Standard,
    pub compatibility: Compatibility,
    // General.
    /// Embed the native document so the PDF reopens editable.
    pub preserve_editing: bool,
    /// Embed a thumbnail image per page.
    pub thumbnails: bool,
    /// Write a linearised file that displays page by page while it downloads.
    pub fast_web_view: bool,
    /// The app opens the written file (frontends only; the writer ignores it).
    pub view_after_saving: bool,
    /// Write layers and sublayers as PDF layers (optional content).
    pub create_layers: bool,
    /// Keep the layers whose Print option is off (they are left out otherwise, unless
    /// `create_layers` is on).
    pub include_non_printing: bool,
    pub compression: CompressionSettings,
    pub marks: MarkSettings,
    pub bleed: BleedSettings,
    pub output: OutputSettings,
    pub advanced: AdvancedSettings,
    pub security: SecuritySettings,
    /// The transparency flattener preset PDF 1.3 files (PDF/X-1a and PDF/X-3 ones too) are
    /// flattened with: a built-in or saved preset's name; empty, High Resolution. The app flattens
    /// the document before writing; the writer doesn't read it.
    pub flattener_preset: String,
}

/// Resampling and compression of one kind of image.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImageCompression {
    pub downsample: Downsample,
    /// Target resolution (ppi).
    pub ppi: f64,
    /// Only images above this resolution (ppi) are downsampled.
    pub above_ppi: f64,
    pub compression: ImageCodec,
    pub quality: JpegQuality,
}

impl Default for ImageCompression {
    fn default() -> Self {
        Self { downsample: Downsample::None, ppi: 300.0, above_ppi: 450.0, compression: ImageCodec::Auto, quality: JpegQuality::Maximum }
    }
}

/// Resampling and compression of monochrome images.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MonoCompression {
    pub downsample: Downsample,
    pub ppi: f64,
    pub above_ppi: f64,
    pub compression: MonoCodec,
}

impl Default for MonoCompression {
    fn default() -> Self {
        Self { downsample: Downsample::None, ppi: 1200.0, above_ppi: 1800.0, compression: MonoCodec::Zip }
    }
}

/// The Compression section.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompressionSettings {
    pub color: ImageCompression,
    pub gray: ImageCompression,
    pub mono: MonoCompression,
    /// Compress text and line art (Flate content streams).
    pub compress_text: bool,
}

impl Default for CompressionSettings {
    fn default() -> Self {
        Self { color: ImageCompression::default(), gray: ImageCompression::default(), mono: MonoCompression::default(), compress_text: true }
    }
}

/// Printer's marks (Marks and Bleeds section).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MarkSettings {
    pub trim: bool,
    pub registration: bool,
    pub color_bars: bool,
    pub page_info: bool,
    pub kind: MarkKind,
    /// Trim mark weight (pt).
    pub weight: f64,
    /// Distance of the marks from the artboard (pt).
    pub offset: f64,
}

impl Default for MarkSettings {
    fn default() -> Self {
        Self { trim: false, registration: false, color_bars: false, page_info: false, kind: MarkKind::Roman, weight: 0.25, offset: 6.0 }
    }
}

impl MarkSettings {
    /// Any mark is on.
    pub fn any(&self) -> bool {
        self.trim || self.registration || self.color_bars || self.page_info
    }
}

/// Bleed (Marks and Bleeds section), in points.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BleedSettings {
    /// Use the document's bleed instead of the values below.
    pub use_document: bool,
    pub top: f64,
    pub bottom: f64,
    pub left: f64,
    pub right: f64,
}

impl BleedSettings {
    /// `[top, bottom, left, right]`.
    pub(crate) fn values(&self) -> [f64; 4] {
        [self.top, self.bottom, self.left, self.right]
    }
}

/// The Output destination that names the CMYK profile of the colour settings in effect, in an RGB
/// document as in a CMYK one (Illustrator's "Document CMYK"; #2950).
pub const DOCUMENT_CMYK: &str = "Document CMYK";

/// The Output section.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OutputSettings {
    pub conversion: ColorConversion,
    /// Destination profile name (empty: the document's profile; [`DOCUMENT_CMYK`]: the CMYK
    /// profile in effect, whatever the document's colour mode).
    pub destination: String,
    pub profiles: ProfileInclusion,
    /// PDF/X output intent profile name.
    pub output_intent: String,
    pub output_condition: String,
    pub output_condition_id: String,
    pub registry: String,
    /// Mark the file as trapped.
    pub trapped: bool,
}

/// The Advanced section.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AdvancedSettings {
    /// Embed whole fonts when more than this share (%) of their characters is used.
    pub font_subset_percent: f64,
    /// Text as glyph outlines; off, text is real (selectable, searchable) text in embedded subset
    /// fonts.
    pub outline_text: bool,
    pub overprint: Overprint,
}

impl Default for AdvancedSettings {
    fn default() -> Self {
        Self { font_subset_percent: 100.0, outline_text: false, overprint: Overprint::Preserve }
    }
}

/// The Security section. Passwords are never serialized (presets and summaries leave them out).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SecuritySettings {
    #[serde(skip_serializing)]
    pub open_password: String,
    #[serde(skip_serializing)]
    pub permissions_password: String,
    pub printing: Printing,
    pub changes: Changes,
    /// Allow copying text, images and other content.
    pub copy: bool,
    /// Allow screen readers to read the text.
    pub screen_reader: bool,
    /// Leave the metadata unencrypted.
    pub plaintext_metadata: bool,
}

impl Default for SecuritySettings {
    fn default() -> Self {
        Self {
            open_password: String::new(),
            permissions_password: String::new(),
            printing: Printing::High,
            changes: Changes::Any,
            copy: true,
            screen_reader: true,
            plaintext_metadata: true,
        }
    }
}

/// `v` lies in `lo..=hi` (and is a number).
pub(crate) fn within(name: &str, v: f64, lo: f64, hi: f64, unit: &str) -> Result<(), PdfError> {
    if (lo..=hi).contains(&v) { Ok(()) } else { Err(PdfError::BadSetting(format!("{name} must be {lo}–{hi}{unit} (got {v})"))) }
}

impl PdfSettings {
    /// Refuse settings the writer can't honour: what [`Self::check_values`] refuses, a destination
    /// profile that isn't there, or a PDF/X output intent it can't write ([`crate::pdfx`]).
    pub fn check(&self) -> Result<(), PdfError> {
        crate::output::check(self)?;
        crate::pdfx::check(self)?;
        self.check_values()
    }

    /// The part of [`Self::check`] a preset must pass (without the colour settings' profiles): a
    /// standard with a PDF version it doesn't allow, a standard with editing data, PDF/X-1a or
    /// PDF/X-3 with PDF layers, or an out-of-range value.
    pub fn check_values(&self) -> Result<(), PdfError> {
        self.security.check(self.standard, self.compatibility)?;
        if !self.standard.allows(self.compatibility) {
            return Err(PdfError::BadSetting(format!("{} files can't be {}", self.standard.label(), self.compatibility.label())));
        }
        if self.preserve_editing && self.standard != Standard::None {
            return Err(PdfError::BadSetting(format!("{} files can't carry editing data: turn off preserveEditing", self.standard.label())));
        }
        if self.create_layers && !self.standard.allows_layers() {
            return Err(PdfError::BadSetting(format!("{} files can't have PDF layers: turn off createLayers", self.standard.label())));
        }
        let c = &self.compression;
        for (name, img) in
            [("color", (c.color.ppi, c.color.above_ppi)), ("gray", (c.gray.ppi, c.gray.above_ppi)), ("mono", (c.mono.ppi, c.mono.above_ppi))]
        {
            within(&format!("compression.{name}.ppi"), img.0, 9.0, 2400.0, " ppi")?;
            within(&format!("compression.{name}.abovePpi"), img.1, 9.0, 2400.0, " ppi")?;
        }
        self.marks.check()?;
        self.bleed.check()?;
        within("advanced.fontSubsetPercent", self.advanced.font_subset_percent, 0.0, 100.0, "%")
    }

    /// Layers and sublayers are written as PDF layers: asked for, at a version that has them.
    pub fn writes_layers(&self) -> bool {
        self.create_layers && self.compatibility.has_layers()
    }

    /// The file is PDF 1.3 (asked for, or a PDF/X-1a or PDF/X-3 file): it has no transparency (the
    /// app flattens it, see [`Self::flattener_preset`]), is written with the PDF 1.4 settings, and
    /// its header and metadata say 1.3.
    pub fn pdf13(&self) -> bool {
        self.compatibility == Compatibility::Pdf13 || self.standard.flattens()
    }

    /// Forget the passwords (presets never store them).
    pub fn clear_passwords(&mut self) {
        self.security.open_password.clear();
        self.security.permissions_password.clear();
    }

    /// Options that are accepted but not applied by the writer yet, one warning each.
    pub fn warnings(&self) -> Vec<String> {
        let d = Self::default();
        let s = &self.security;
        [
            (self.create_layers && !self.compatibility.has_layers(), "PDF layers need PDF 1.5 or later: every layer is plain page content"),
            (
                !self.advanced.outline_text && self.advanced.font_subset_percent < 100.0,
                "fonts are embedded as subsets of the characters used: a subset threshold below 100% is not applied",
            ),
            (
                !s.protected()
                    && (s.printing != d.security.printing
                        || s.changes != d.security.changes
                        || s.copy != d.security.copy
                        || s.screen_reader != d.security.screen_reader
                        || s.plaintext_metadata != d.security.plaintext_metadata),
                "permissions need a password: without one they are not applied",
            ),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, w)| w.to_string())
        .chain(crate::encrypt::warnings(self))
        .chain(crate::output::warnings(self))
        .collect()
    }
}

impl MarkSettings {
    /// Refuse a weight or offset out of range (PDF export and print).
    pub(crate) fn check(&self) -> Result<(), PdfError> {
        within("marks.weight", self.weight, 0.05, 2.0, " pt")?;
        within("marks.offset", self.offset, 0.0, 72.0, " pt")
    }
}

impl BleedSettings {
    /// Refuse a side out of range (PDF export and print).
    pub(crate) fn check(&self) -> Result<(), PdfError> {
        for (side, v) in ["top", "bottom", "left", "right"].iter().zip(self.values()) {
            within(&format!("bleed.{side}"), v, 0.0, 72.0, " pt")?;
        }
        Ok(())
    }
}
