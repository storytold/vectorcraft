//! Barcode and QR Code encoders, written from the public symbology standards.
//!
//! A caller hands [`encode`] a [`BarcodeSpec`] — a symbology, the data and the sizing options —
//! and gets back a [`Code`]: either a [`Symbol`] (the dark runs of a linear bar code, in
//! millimetres) or a [`Matrix`] (the module grid of a QR Code). Neither carries any paint or
//! geometry type of its own, so the same encoders serve the engine's commands, the desktop panel
//! and the tests.
//!
//! Where the numbers come from (all public):
//!
//! - **EAN-13, EAN-8, UPC-A, UPC-E** — GS1 *General Specifications* §5.2.2.4 (the nominal
//!   X-dimension of 0.33 mm, the guard bars extended downward by 5X, the quiet zones, the
//!   ±1/13-module bar adjustment for the characters 1, 2, 7 and 8) and §5.2.4 (the human
//!   readable line: 2.75 mm digits, at least 0.5X of clear space below the bars, the UPC first
//!   and last digits reduced to four modules). Also ISO/IEC 15420.
//! - **ITF-14** — GS1 *General Specifications* §5.3.1 (10X quiet zones, a wide-to-narrow ratio of
//   2.25:1 to 3.0:1, a 32 mm symbol, the 4.8 mm bearer bar, 1.02 mm between it and the human
//!   readable line). Also ISO/IEC 16390.
//! - **GS1-128** — GS1 *General Specifications* §5.4.7 (bar height of at least 32 mm in general
//!   distribution) on top of ISO/IEC 15417.
//! - **Code 39** — ISO/IEC 16388; **Code 93** — AIM/ISO 93; **Code 128** — ISO/IEC 15417.
//! - **QR Code** — ISO/IEC 18004 (Model 2, versions 1–40, the four error-correction levels, the
//!   four encoding modes and the 4-module quiet zone).
//!
//! The standards fix a *nominal* X-dimension for the EAN/UPC family and for ITF-14; for the other
//! linear symbologies they leave the module width to the application, so those fall back to the
//! same 0.33 mm and to a bar height that scans reliably. Everything the standards do not fix is a
//! default a caller can override through [`BarcodeSpec`].

mod check;
mod code128;
mod code39;
mod code93;
mod ean;
mod itf;
mod linear;
mod qr;
#[cfg(test)]
mod testutil;

use serde::{Deserialize, Serialize};

pub use check::CheckDigit;
pub use code128::CodeSet;
pub use qr::Ecc;

use linear::Linear;

/// Millimetres per inch, for callers that work in points.
pub const MM_PER_INCH: f64 = 25.4;

/// The most data any one symbol may carry, so that a runaway value cannot ask for an unbounded
/// allocation. QR Code version 40 at level L holds 2,953 bytes; the linear symbologies hold far
/// less, and their own encoders refuse first.
pub const MAX_DATA: usize = 4096;

/// What went wrong encoding a [`BarcodeSpec`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The symbology name is not one of [`Symbology::all`].
    #[error("unknown barcode type `{0}`")]
    UnknownSymbology(String),
    /// The data does not suit the symbology (wrong length, a character outside the set, …).
    #[error("{0}")]
    Data(String),
}

/// A result from this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// The symbologies the encoder knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Symbology {
    /// EAN-13, the retail article number (ISO/IEC 15420). GS1 calls the 13-digit form GTIN-13.
    Ean13,
    /// EAN-8, the short form for small packages.
    Ean8,
    /// UPC-A, the North American retail number.
    UpcA,
    /// UPC-E, UPC-A zero-suppressed for small packages.
    UpcE,
    /// Code 128 with the GS1 application identifier conventions (GS1-128, formerly EAN-128).
    Ean128,
    /// Code 128, with the code set chosen automatically from the data.
    Code128,
    /// Code 128, code set A (upper case, digits and control characters).
    Code128A,
    /// Code 128, code set B (the full ASCII printable set).
    Code128B,
    /// Code 128, code set C (digit pairs).
    Code128C,
    /// Code 39 (ISO/IEC 16388).
    Code39,
    /// Code 93.
    Code93,
    /// ITF-14, the interleaved 2 of 5 shipping-container code with its bearer bars.
    Itf14,
    /// Interleaved 2 of 5, without the ITF-14 bearer bars.
    Itf25,
    /// QR Code, Model 2 (ISO/IEC 18004).
    QrCode,
}

impl Symbology {
    /// Every symbology, in the order a picker lists them: the retail codes first, then the
    /// general-purpose ones, then the matrix code.
    pub const fn all() -> [Symbology; 14] {
        [
            Symbology::Ean13,
            Symbology::Ean8,
            Symbology::UpcA,
            Symbology::UpcE,
            Symbology::Ean128,
            Symbology::Code128,
            Symbology::Code128A,
            Symbology::Code128B,
            Symbology::Code128C,
            Symbology::Code39,
            Symbology::Code93,
            Symbology::Itf14,
            Symbology::Itf25,
            Symbology::QrCode,
        ]
    }

    /// The stable id, as a [`BarcodeSpec`] and the commands carry it.
    pub const fn id(self) -> &'static str {
        match self {
            Symbology::Ean13 => "ean13",
            Symbology::Ean8 => "ean8",
            Symbology::UpcA => "upcA",
            Symbology::UpcE => "upcE",
            Symbology::Ean128 => "ean128",
            Symbology::Code128 => "code128",
            Symbology::Code128A => "code128A",
            Symbology::Code128B => "code128B",
            Symbology::Code128C => "code128C",
            Symbology::Code39 => "code39",
            Symbology::Code93 => "code93",
            Symbology::Itf14 => "itf14",
            Symbology::Itf25 => "itf25",
            Symbology::QrCode => "qrCode",
        }
    }

    /// Where the symbology comes from: the standard a reader can check this encoder against.
    ///
    /// The EAN/UPC family is ISO/IEC 15420, which GS1 publishes as its own General Specifications
    /// §5.2 — and China's GB 12904, the United States' UPC, Japan's JAN and others are the same
    /// symbol under a national number. ITF-14 is ISO/IEC 16390 with GS1's bearer bars on top.
    pub const fn standard(self) -> &'static str {
        match self {
            Symbology::Ean13 => "GS1 §5.2 · ISO/IEC 15420 · GB 12904 · JAN",
            Symbology::Ean8 => "GS1 §5.2 · ISO/IEC 15420",
            Symbology::UpcA | Symbology::UpcE => "GS1 §5.2 · ISO/IEC 15420 · UPC",
            Symbology::Ean128 => "GS1 §5.4.7 · ISO/IEC 15417",
            Symbology::Code128 | Symbology::Code128A | Symbology::Code128B | Symbology::Code128C => "ISO/IEC 15417",
            Symbology::Code39 => "ISO/IEC 16388",
            Symbology::Code93 => "AIM/ISO 93",
            Symbology::Itf14 => "GS1 General Specifications §5.3 · ISO/IEC 16390",
            Symbology::Itf25 => "ISO/IEC 16390",
            Symbology::QrCode => "ISO/IEC 18004",
        }
    }

    /// Where the symbology is used, in two or three words: what the picker's rows say beside the
    /// name, as the barcode tools a packaging shop already uses do.
    pub const fn origin(self) -> &'static str {
        match self {
            Symbology::Ean13 => "GS1 · GB 12904",
            Symbology::Ean8 => "GS1",
            Symbology::UpcA | Symbology::UpcE => "GS1 · UPC",
            Symbology::Ean128 => "GS1-128",
            Symbology::Code128 | Symbology::Code128A | Symbology::Code128B | Symbology::Code128C => "ISO/IEC 15417",
            Symbology::Code39 => "ISO/IEC 16388",
            Symbology::Code93 => "AIM/ISO 93",
            Symbology::Itf14 => "GS1 · ISO/IEC 16390",
            Symbology::Itf25 => "ISO/IEC 16390",
            Symbology::QrCode => "ISO/IEC 18004",
        }
    }

    /// The name a menu or a label shows.
    pub const fn label(self) -> &'static str {
        match self {
            Symbology::Ean13 => "EAN-13",
            Symbology::Ean8 => "EAN-8",
            Symbology::UpcA => "UPC-A",
            Symbology::UpcE => "UPC-E",
            Symbology::Ean128 => "EAN-128 (GS1-128)",
            Symbology::Code128 => "Code 128",
            Symbology::Code128A => "Code 128 A",
            Symbology::Code128B => "Code 128 B",
            Symbology::Code128C => "Code 128 C",
            Symbology::Code39 => "Code 39",
            Symbology::Code93 => "Code 93",
            Symbology::Itf14 => "ITF-14",
            Symbology::Itf25 => "ITF-25",
            Symbology::QrCode => "QR Code",
        }
    }

    /// The symbology an id names.
    pub fn from_id(id: &str) -> Option<Symbology> {
        Symbology::all().into_iter().find(|s| s.id() == id)
    }

    /// What a data field has to hold for this symbology, for the panel's hint and the error.
    pub const fn hint(self) -> &'static str {
        match self {
            Symbology::Ean13 => "12 or 13 digits",
            Symbology::Ean8 => "7 or 8 digits",
            Symbology::UpcA => "11 or 12 digits",
            Symbology::UpcE => "6 or 8 digits",
            Symbology::Ean128 => "digits, with or without the (01) application identifier",
            Symbology::Code128 | Symbology::Code128B => "printable ASCII",
            Symbology::Code128A => "printable ASCII, upper case and control characters",
            Symbology::Code128C => "an even number of digits",
            Symbology::Code39 => "digits, A–Z and - . $ / + % and space",
            Symbology::Code93 => "ASCII",
            Symbology::Itf14 | Symbology::Itf25 => "an even number of digits",
            Symbology::QrCode => "any text",
        }
    }

    /// Whether the symbol is a matrix rather than bars, so a caller knows which [`Code`] to expect.
    pub const fn is_matrix(self) -> bool {
        matches!(self, Symbology::QrCode)
    }

    /// Data a new symbol of this type is drawn with, so that a caller who has not typed anything
    /// yet sees a whole symbol rather than an error message. Every one of these is a real code of
    /// its type: the GTIN-13 is GS1's own worked example.
    pub const fn sample_data(self) -> &'static str {
        match self {
            Symbology::Ean13 => "6907045517163",
            Symbology::Ean8 => "96385074",
            Symbology::UpcA => "036000291452",
            Symbology::UpcE => "01234565",
            Symbology::Ean128 => "(01)12345678901231",
            Symbology::Code128 | Symbology::Code128B => "ABC-123",
            Symbology::Code128A => "ABC123",
            Symbology::Code128C => "1234567890",
            Symbology::Code39 | Symbology::Code93 => "ABC-123",
            Symbology::Itf14 => "15400141288763",
            Symbology::Itf25 => "1234567890",
            Symbology::QrCode => "https://example.com",
        }
    }

    /// The nominal module width in millimetres, before [`BarcodeSpec::magnification`].
    ///
    /// GS1 fixes 0.33 mm for the EAN/UPC family (§5.2.2.4.1) and leaves ITF-14's X to the
    /// application; 1.016 mm is the target GS1 prints its own worked example with (§5.3.1.3). The
    /// remaining linear symbologies have no nominal X either, and take the EAN/UPC one.
    pub const fn nominal_module_mm(self) -> f64 {
        match self {
            Symbology::Itf14 => 1.016,
            Symbology::QrCode => 0.5,
            _ => 0.33,
        }
    }

    /// The bar height in millimetres at the nominal module width, before
    /// [`BarcodeSpec::magnification`].
    ///
    /// The EAN/UPC family heights are GS1 §5.2.2.4.2 (an EAN-13/UPC-A/UPC-E symbol is about
    /// 25.9 mm high and an EAN-8 about 21.3 mm, both *including* the human readable line, which
    /// the encoder takes back out again: 2.75 mm digits with one module of clear space above
    /// them). ITF-14 prints its 32 mm symbol (§5.3.1.5) and GS1-128 wants at least 32 mm in
    /// general distribution (§5.4.7.1). The rest are not fixed by their standards.
    pub const fn nominal_height_mm(self) -> f64 {
        match self {
            Symbology::Ean13 | Symbology::UpcA | Symbology::UpcE => 22.85,
            Symbology::Ean8 => 18.22,
            Symbology::Ean128 | Symbology::Itf14 => 32.0,
            _ => 15.0,
        }
    }

    /// The quiet zones in modules, left and right (GS1 §5.2.2.4.4 for the EAN/UPC family and
    /// §5.3.1.3 for ITF-14; the others take the 10X the linear symbologies ask for).
    pub const fn quiet_modules(self) -> (f64, f64) {
        match self {
            Symbology::Ean13 => (11.0, 7.0),
            Symbology::Ean8 => (7.0, 7.0),
            Symbology::UpcA => (9.0, 9.0),
            Symbology::UpcE => (9.0, 7.0),
            _ => (10.0, 10.0),
        }
    }

    /// The height of the human readable digits in millimetres at the nominal module width.
    ///
    /// GS1 §5.2.4 fixes 2.75 mm for the EAN/UPC family; the other symbologies leave it open and
    /// take the same size, which is what a scanner-agnostic artwork uses.
    pub const fn hri_mm(self) -> f64 {
        2.75
    }

    /// The gap between the bottom of the bars and the top of the digits, in millimetres at the
    /// nominal module width.
    ///
    /// GS1 §5.2.4 asks for at least 0.5X below the bars and notes that one module is the usual
    /// value; ITF-14 §5.3.1.3 wants at least 1.02 mm below its **bearer bar** instead.
    pub const fn hri_gap_mm(self) -> f64 {
        match self {
            Symbology::Itf14 => 1.02,
            _ => 0.33,
        }
    }

    /// The bearer bar thickness in millimetres, or 0 for a symbol that has none.
    ///
    /// GS1 §5.3.1.5: a nominal 4.8 mm, surrounding the symbol including its quiet zones and
    /// butting directly against the top and bottom of the bars.
    pub const fn bearer_mm(self) -> f64 {
        match self {
            Symbology::Itf14 => 4.8,
            _ => 0.0,
        }
    }
}

/// One dark bar of a linear symbol.
///
/// Horizontal positions are in millimetres from the left edge of the left quiet zone, so a caller
/// draws the bars where they are and leaves the quiet zone empty. Vertical positions are in
/// millimetres down from the top of the symbol's tallest mark, so the bars of an ordinary symbol
/// start at 0 and the guard bars of an EAN/UPC symbol run past the others.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    /// Left edge.
    pub x: f64,
    /// Width. Always positive.
    pub width: f64,
    /// Top edge.
    pub y: f64,
    /// Height. Always positive.
    pub height: f64,
}

/// The human readable line under (or, for an add-on, over) a linear symbol.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HumanReadable {
    /// One run of text, placed by the centre of the character cells it sits under.
    pub parts: Vec<HriPart>,
    /// The digit height the parts are set at, in millimetres: the cap height of the type, not its
    /// em size.
    pub height_mm: f64,
    /// The clear space between the bars and the top of the digits, in millimetres.
    pub gap_mm: f64,
    /// The distance from the top of the symbol down to the top of the digits, in millimetres.
    pub top_mm: f64,
}

/// One run of the human readable line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HriPart {
    /// The characters, as the standard prints them.
    pub text: String,
    /// Where the run is anchored, in millimetres from the left edge of the left quiet zone.
    pub at_mm: f64,
    /// Which edge of the run [`HriPart::at_mm`] is.
    pub align: HriAlign,
    /// How wide the run may be drawn before it stops matching the bars, in millimetres. A caller
    /// shrinks the type to fit when the font it was given is wider than this.
    pub width_mm: f64,
    /// False for the reduced first and last digits of a UPC-A or UPC-E symbol.
    pub full_size: bool,
}

/// Which edge of a human readable run its anchor is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HriAlign {
    Left,
    Center,
    Right,
}

/// A linear bar code, laid out in millimetres.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    pub symbology: Symbology,
    /// The dark bars, left to right, already merged where they touch.
    pub bars: Vec<Bar>,
    /// The module width the symbol was laid out with, after `magnification`.
    pub module_mm: f64,
    /// The width of the bar pattern alone, without the quiet zones.
    pub modules: f64,
    /// The overall width, quiet zones included.
    pub width_mm: f64,
    /// The overall height: bars, bearer bars and the human readable line.
    pub height_mm: f64,
    /// The bearer bars' thickness in millimetres, 0 when the symbol has none.
    pub bearer_mm: f64,
    /// The human readable line, when one is drawn.
    pub human_readable: Option<HumanReadable>,
}

/// A matrix (two-dimensional) code.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Matrix {
    pub symbology: Symbology,
    /// Modules a side, quiet zone excluded.
    pub size: usize,
    /// Quiet zone width in modules (4 for a QR Code, per ISO/IEC 18004).
    pub quiet: usize,
    /// The module size in millimetres, after `magnification`.
    pub module_mm: f64,
    /// `size × size` modules, row by row, row 0 at the top: true is a dark module.
    pub dark: Vec<bool>,
    /// The QR Code version, 1–40 (the symbol size is `4 × version + 17`).
    pub version: u8,
    /// The error-correction level it was encoded at.
    pub ecc: Ecc,
}

impl Matrix {
    /// The overall width in millimetres, quiet zone included.
    pub fn width_mm(&self) -> f64 {
        (self.size + 2 * self.quiet) as f64 * self.module_mm
    }

    /// Whether the module at `x`, `y` is dark. Outside the grid: false.
    pub fn dark_at(&self, x: usize, y: usize) -> bool {
        if x >= self.size || y >= self.size {
            return false;
        }
        self.dark.get(y * self.size + x).copied().unwrap_or(false)
    }

    /// The dark modules as horizontal runs, one per (row, run): the row, the first and last
    /// module column, and how many module rows the run spans. Merging the rows is what keeps a
    /// version-40 symbol from becoming 31,000 separate objects.
    pub fn runs(&self) -> Vec<MatrixRun> {
        let mut out: Vec<MatrixRun> = Vec::new();
        for y in 0..self.size {
            let mut x = 0;
            while x < self.size {
                if !self.dark_at(x, y) {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < self.size && self.dark_at(x, y) {
                    x += 1;
                }
                let run = MatrixRun { x: start, y, width: x - start, height: 1 };
                // Extend the run one row down while the same span is dark there too.
                match out.last_mut() {
                    Some(last) if last.x == run.x && last.width == run.width && last.y + last.height == run.y => last.height += 1,
                    _ => out.push(run),
                }
            }
        }
        out
    }
}

/// A run of dark modules of a [`Matrix`], in module coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatrixRun {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

/// What [`encode`] produced.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Code {
    Linear(Symbol),
    Matrix(Matrix),
}

impl Code {
    /// The overall size in millimetres, quiet zone included.
    pub fn size_mm(&self) -> (f64, f64) {
        match self {
            Code::Linear(s) => (s.width_mm, s.height_mm),
            Code::Matrix(m) => (m.width_mm(), m.width_mm()),
        }
    }

    /// The symbology it was encoded with.
    pub fn symbology(&self) -> Symbology {
        match self {
            Code::Linear(s) => s.symbology,
            Code::Matrix(m) => m.symbology,
        }
    }
}

/// Everything needed to draw one code: the symbology, the data and the sizing. This is what a
/// document stores on a generated object, so the art can be rebuilt after an edit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BarcodeSpec {
    pub symbology: Symbology,
    pub data: String,
    /// The X-dimension multiplier, 1.0 at nominal (GS1's "magnification factor"). Clamped to
    /// 0.25–10 when encoding.
    pub magnification: f64,
    /// The module width in millimetres, overriding the symbology's nominal one. The QR Code's
    /// module size is the same field.
    pub module_mm: Option<f64>,
    /// The bar height in millimetres, overriding the symbology's nominal one.
    pub height_mm: Option<f64>,
    /// Draw the human readable line under the bars.
    pub text: bool,
    /// Extra quiet zone in modules, added to both sides.
    pub quiet_extra: f64,
    /// Draw the ITF-14 bearer bars (ignored by a symbology that has none).
    pub bearer: bool,
    /// The wide-to-narrow element ratio of Code 39 and Interleaved 2 of 5, 2:1 to 3:1. Neither
    /// standard fixes a value; each falls back to its own default when this is unset.
    pub ratio: Option<f64>,
    /// The QR Code error-correction level.
    pub ecc: Ecc,
    /// The QR Code version to pin the symbol to, 1–40, or None for the smallest the data fits.
    pub version: Option<u8>,
    /// The QR Code mask to pin the symbol to, 0–7, or None to have the standard's four penalty
    /// rules choose the least penalised of the eight.
    pub mask: Option<u8>,
    /// The font family for the human readable line, or the document's default when unset.
    pub font: Option<String>,
    /// The font style for the human readable line.
    pub font_style: Option<String>,
    /// Draw the human readable digits as outlines rather than as type, so that the art carries no
    /// font dependency and prints the same on a machine that has neither the font nor the document.
    pub outline_text: bool,
    /// Draw the white box the standard puts round the symbol, which is what keeps the quiet zones
    /// clear when the art is laid over a photograph or a coloured carton.
    pub background: bool,
}

impl Default for BarcodeSpec {
    fn default() -> Self {
        Self {
            symbology: Symbology::Ean13,
            data: String::new(),
            magnification: 1.0,
            module_mm: None,
            height_mm: None,
            text: true,
            quiet_extra: 0.0,
            bearer: true,
            ratio: None,
            ecc: Ecc::Medium,
            version: None,
            mask: None,
            font: None,
            font_style: None,
            outline_text: true,
            background: true,
        }
    }
}

impl BarcodeSpec {
    /// A spec for `symbology` carrying `data`.
    pub fn new(symbology: Symbology, data: impl Into<String>) -> Self {
        Self { symbology, data: data.into(), ..Self::default() }
    }

    /// The module width in millimetres this spec asks for: the override, else the symbology's
    /// nominal one, both scaled by the magnification and kept finite and positive.
    pub fn resolved_module_mm(&self) -> f64 {
        let nominal = self.module_mm.unwrap_or_else(|| self.symbology.nominal_module_mm());
        let nominal = if nominal.is_finite() && nominal > 0.0 { nominal } else { self.symbology.nominal_module_mm() };
        (nominal * self.magnification()).clamp(0.05, 100.0)
    }

    /// The bar height in millimetres this spec asks for.
    pub fn resolved_height_mm(&self) -> f64 {
        let nominal = self.height_mm.unwrap_or_else(|| self.symbology.nominal_height_mm());
        let nominal = if nominal.is_finite() && nominal > 0.0 { nominal } else { self.symbology.nominal_height_mm() };
        (nominal * self.magnification()).clamp(0.5, 1000.0)
    }

    /// The magnification, kept finite and within what a printer can hold.
    pub fn magnification(&self) -> f64 {
        if self.magnification.is_finite() { self.magnification.clamp(0.25, 10.0) } else { 1.0 }
    }
}

/// Encode `spec`.
///
/// A symbology that carries a check digit in its data (EAN, UPC, ITF-14) takes the data with or
/// without it and adds it when it is missing; a wrong check digit is refused rather than silently
/// corrected, so a typo in a product number does not print as a different, valid-looking code.
pub fn encode(spec: &BarcodeSpec) -> Result<Code> {
    if spec.data.chars().count() > MAX_DATA {
        return Err(Error::Data(format!("the data is longer than the {MAX_DATA} characters a symbol may hold")));
    }
    let data = spec.data.trim();
    if data.is_empty() {
        return Err(Error::Data("the code is empty".into()));
    }
    let symbol = match spec.symbology {
        Symbology::QrCode => return qr::encode(spec, data).map(Code::Matrix),
        Symbology::Ean13 | Symbology::Ean8 | Symbology::UpcA | Symbology::UpcE => ean::encode(spec, data)?,
        Symbology::Ean128 | Symbology::Code128 | Symbology::Code128A | Symbology::Code128B | Symbology::Code128C => code128::encode(spec, data)?,
        Symbology::Code39 => code39::encode(spec, data)?,
        Symbology::Code93 => code93::encode(spec, data)?,
        Symbology::Itf14 | Symbology::Itf25 => itf::encode(spec, data)?,
    };
    Ok(Code::Linear(symbol))
}

/// A linear symbol around `linear`, with the quiet zones, the bearer bars, the height and the
/// human readable line filled in from `spec`.
///
/// `parts` are the digits as the symbology lays them out, placed in module coordinates from the
/// left edge of the bar pattern; the size, the gap and the vertical position are the spec's. An
/// empty `parts` with the text turned on is a symbol with a human readable line the caller did
/// not name the characters of, which no symbology does.
pub(crate) fn finish_linear(spec: &BarcodeSpec, mut linear: Linear, parts: Vec<HriPart>) -> Symbol {
    let symbology = spec.symbology;
    let module_mm = spec.resolved_module_mm();
    let (ql, qr) = symbology.quiet_modules();
    let extra = if spec.quiet_extra.is_finite() { spec.quiet_extra.clamp(0.0, 100.0) } else { 0.0 };
    let quiet_left = (ql + extra) * module_mm;
    let quiet_right = (qr + extra) * module_mm;
    let bearer = if spec.bearer { symbology.bearer_mm() * spec.magnification() } else { 0.0 };
    let height = spec.resolved_height_mm();
    let pattern_bottom = bearer + height;
    // The human readable line starts below the bottom bearer bar, when there is one: GS1 measures
    // its clearance from the bearer bar rather than from the bars.
    let bars_bottom = pattern_bottom + bearer;
    let modules = linear.width_modules();
    let width_mm = quiet_left + modules * module_mm + quiet_right;
    // Move the bar pattern right of the left quiet zone and down under the top bearer bar.
    linear.shift(quiet_left, bearer, module_mm);
    let mut bars = linear.into_bars();
    if bearer > 0.0 {
        bars.push(Bar { x: 0.0, y: 0.0, width: width_mm, height: bearer });
        bars.push(Bar { x: 0.0, y: pattern_bottom, width: width_mm, height: bearer });
    }
    let human_readable = hri(spec, parts).map(|mut h| {
        for part in &mut h.parts {
            // Module coordinates into millimetres, right of the left quiet zone.
            part.at_mm = quiet_left + part.at_mm * module_mm;
            part.width_mm *= module_mm;
        }
        h.height_mm = (symbology.hri_mm() * spec.magnification()).clamp(0.2, 200.0);
        h.gap_mm = (symbology.hri_gap_mm() * spec.magnification()).clamp(0.0, 100.0);
        h.top_mm = bars_bottom + h.gap_mm;
        h
    });
    let total_height = bars_bottom + human_readable.as_ref().map_or(0.0, |h| h.gap_mm + h.height_mm);
    Symbol { symbology, bars, module_mm, modules, width_mm, height_mm: total_height, bearer_mm: bearer, human_readable }
}

/// The human readable line of a symbol, when the spec asks for one. The size, the gap and the
/// vertical position are filled in by [`finish_linear`]; an encoder only says where the digits go.
pub(crate) fn hri(spec: &BarcodeSpec, parts: Vec<HriPart>) -> Option<HumanReadable> {
    spec.text.then_some(HumanReadable { parts, height_mm: 0.0, gap_mm: 0.0, top_mm: 0.0 })
}
