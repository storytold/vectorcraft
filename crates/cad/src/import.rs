//! DXF import: an ASCII DXF drawing of any version as a document.
//!
//! The drawing's model space (or a paper layout) comes in at a ratio (1 unit of the art = N
//! drawing units; by default the drawing's own unit at 1:1) or fitted to an artboard, centred
//! or with the drawing's origin on the artboard's bottom-left corner. Each DXF layer holding art
//! becomes a layer (hidden, frozen, locked and non-plotting layers keep that), or all art goes on
//! one. Lines, polylines with arc segments, circles, arcs, ellipses, splines (NURBS), solid
//! hatches and solids become paths; text and multiline text point type; named blocks symbols
//! (anonymous ones, such as dimensions, groups of their art). Colours (indexed and true colour),
//! lineweights, linetypes and transparency come through layers and blocks as CAD apps resolve
//! them. What can't come in is listed in the warnings.
//!
//! - `reader`: the file's text, group codes, sections, tables, blocks and layouts.
//! - `entity`: entities to items (geometry and properties).
//! - `curve`: bulges, arcs, ellipses and NURBS as Bézier paths.
//! - `text`: text values to plain text.
//! - `build`: items to the document.

mod build;
mod curve;
mod entity;
mod reader;
mod text;

use vectorcraft_doc::{Document, Unit};

pub use reader::BINARY_SENTINEL;

use crate::{DxfVersion, INSUNITS, SCALE_RANGE};

/// Letter, the box Fit to Artboard fills unless told otherwise (points).
pub const FIT_TO: (f64, f64) = (612.0, 792.0);

/// Why a binary DXF can't be opened.
pub const BINARY: &str = "this is a binary DXF file, which can't be read: save it from the CAD app as an ASCII DXF and open that";

/// How a drawing comes in.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportOptions {
    /// The layout (any case): `Model` (`None`, the default) or a paper layout's name.
    pub layout: Option<String>,
    /// Scale the art to fit `fit_to` (its larger side along the art's), instead of the ratio.
    pub fit: bool,
    /// The artboard Fit fills (points, `width × height`).
    pub fit_to: (f64, f64),
    /// The ratio: 1 `unit` of the art is `scale` drawing units. `None`: the drawing's own unit
    /// at 1:1 (millimetres for unitless metric drawings, inches for imperial ones).
    pub unit: Option<Unit>,
    pub scale: Option<f64>,
    /// Lineweights scale with the art (else they keep their printed widths).
    pub scale_lineweights: bool,
    /// The art's centre on the artboard's (else the drawing's origin, or fitted art's
    /// bottom-left corner, on the artboard's bottom-left corner).
    pub center: bool,
    /// All art on one layer.
    pub merge_layers: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self { layout: None, fit: false, fit_to: FIT_TO, unit: None, scale: None, scale_lineweights: false, center: true, merge_layers: false }
    }
}

/// An imported drawing.
#[derive(Clone, Debug)]
pub struct Imported {
    pub document: Document,
    /// What was approximated or left out.
    pub warnings: Vec<String>,
}

/// What a drawing holds, for the import options.
#[derive(Clone, Debug, PartialEq)]
pub struct DxfInfo {
    /// The DXF version (`2018`, `R12`…; the `$ACADVER` code when it's none of the known ones).
    pub version: String,
    /// The drawing unit's name (`Millimeters`, `Unitless`…).
    pub units: String,
    /// `Model`, then the paper layouts.
    pub layouts: Vec<String>,
    /// The layer table's layers.
    pub layers: Vec<String>,
    /// The default ratio: 1 `unit` = `scale` drawing units.
    pub unit: Unit,
    pub scale: f64,
}

/// Does `bytes` look like a DXF file (ASCII or binary)?
pub fn is_dxf(bytes: &[u8]) -> bool {
    if bytes.starts_with(reader::BINARY_SENTINEL) {
        return true;
    }
    let head = String::from_utf8_lossy(bytes.get(..bytes.len().min(1024)).unwrap_or(bytes));
    let mut lines = head.trim_start_matches('\u{feff}').lines().map(str::trim).skip_while(|l| l.is_empty());
    // Comments (999) may come first; then the first section.
    while let (Some(code), Some(value)) = (lines.next(), lines.next()) {
        match code {
            "999" => continue,
            "0" => return value == "SECTION",
            _ => return false,
        }
    }
    false
}

/// The pairs of a DXF file and where it is damaged (binary and DWG files are refused).
fn text(bytes: &[u8]) -> Result<std::borrow::Cow<'_, str>, String> {
    if bytes.starts_with(reader::BINARY_SENTINEL) {
        return Err(BINARY.into());
    }
    if bytes.starts_with(b"AC10") || bytes.starts_with(b"AC1.") {
        return Err("this is a DWG drawing, which can't be read: save it from the CAD app as an ASCII DXF and open that".into());
    }
    Ok(reader::decode(bytes))
}

/// The unit of `$INSUNITS` code `code` and the scale that shows the drawing 1:1 in it.
fn default_ratio(h: &reader::Header) -> (Unit, f64) {
    if let Some((_, u)) = INSUNITS.iter().find(|(c, _)| i64::from(*c) == h.insunits) {
        return (*u, 1.0);
    }
    match build::insunits_points(h.insunits) {
        Some(pt) => (Unit::Millimeters, Unit::Millimeters.points() / pt),
        None if h.measurement == Some(0) => (Unit::Inches, 1.0),
        None => (Unit::Millimeters, 1.0),
    }
}

/// The name of `$INSUNITS` code `code`.
fn units_name(code: i64) -> &'static str {
    const NAMES: [&str; 25] = [
        "Unitless",
        "Inches",
        "Feet",
        "Miles",
        "Millimeters",
        "Centimeters",
        "Meters",
        "Kilometers",
        "Microinches",
        "Mils",
        "Yards",
        "Angstroms",
        "Nanometers",
        "Microns",
        "Decimeters",
        "Decameters",
        "Hectometers",
        "Gigameters",
        "Astronomical units",
        "Light years",
        "Parsecs",
        "US survey feet",
        "US survey inches",
        "US survey yards",
        "US survey miles",
    ];
    usize::try_from(code).ok().and_then(|i| NAMES.get(i)).copied().unwrap_or("Unitless")
}

/// Read what `bytes` holds: its version, units, layouts and layers.
pub fn info(bytes: &[u8]) -> Result<DxfInfo, String> {
    let text = text(bytes)?;
    let (pairs, _) = reader::pairs(&text)?;
    let d = reader::Drawing::read(&pairs);
    if d.sections == 0 {
        return Err("not a DXF drawing".into());
    }
    let h = &d.header;
    let (unit, scale) = default_ratio(h);
    let version = DxfVersion::ALL.into_iter().find(|v| v.acadver() == h.acadver).map_or_else(|| h.acadver.clone(), |v| v.id().to_string());
    Ok(DxfInfo {
        version,
        units: units_name(h.insunits).into(),
        layouts: d.layouts.iter().map(|l| l.name.clone()).collect(),
        layers: d.layers.iter().map(|l| l.name.clone()).collect(),
        unit,
        scale,
    })
}

/// Import an ASCII DXF drawing with `o`.
pub fn import(bytes: &[u8], o: &ImportOptions) -> Result<Imported, String> {
    let (w, h) = o.fit_to;
    if !(w > 0.0 && h > 0.0 && w.max(h) <= build::MAX_EXTENT) {
        return Err(format!("the artboard to fit must be a positive size up to {} pt", build::MAX_EXTENT));
    }
    if let Some(s) = o.scale
        && !(s.is_finite() && SCALE_RANGE.contains(&s))
    {
        return Err(format!("scale must be a number from {} to {}, not {s}", SCALE_RANGE.start(), SCALE_RANGE.end()));
    }
    let text = text(bytes)?;
    let (pairs, damaged) = reader::pairs(&text)?;
    let d = reader::Drawing::read(&pairs);
    if d.sections == 0 {
        return Err("not a DXF drawing".into());
    }
    let layout = match &o.layout {
        None => d.layouts.first(),
        Some(name) => d.layout(name),
    };
    let Some(layout) = layout else {
        let names: Vec<&str> = d.layouts.iter().map(|l| l.name.as_str()).collect();
        return Err(format!("the drawing has no layout `{}` (it has {})", o.layout.as_deref().unwrap_or_default(), names.join(", ")));
    };
    let (unit, scale) = default_ratio(&d.header);
    let natural = unit.points() / scale;
    let ratio = match (o.unit, o.scale) {
        (None, None) => natural,
        (u, s) => u.unwrap_or(unit).points() / s.unwrap_or(1.0),
    };
    let (mut document, mut warnings) = build::Build::new(&d, o).run(layout, natural, ratio)?;
    document.units = if o.fit { unit } else { o.unit.unwrap_or(unit) };
    warnings.extend(damaged);
    document.limit_depth_noting(&mut warnings);
    Ok(Imported { document, warnings })
}

#[cfg(test)]
mod tests;
