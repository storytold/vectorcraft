//! CAD interchange: a hand-written ASCII DXF writer (versions R12 to 2018) and reader.
//!
//! [`export`] writes the visible art of a document into one DXF drawing: each layer becomes a DXF
//! layer (hidden ones switched off, locked ones locked, non-printing ones not plotted), paths
//! become polylines (straight segments) or cubic splines (curves), fills their outlines and solid
//! hatches, type text entities or glyph outlines, and placed images image entities linked to PNG
//! or JPEG files the caller writes next to the drawing. Coordinates are y-up, in drawing units: `scale` units per
//! `unit` of the art, with the origin at the bottom-left corner of [`DxfOptions::region`].
//!
//! What DXF can't hold (blending modes, opacity masks, raster effects, gradients, patterns,
//! clipping) is approximated or left out, and each such loss comes back as a warning.
//!
//! [`import()`] reads an ASCII DXF drawing (any version) into a document: see [`import`].
//!
//! - `aci`: the 256-colour index palette and the nearest index of a colour.
//! - `writer`: group codes, handles and the file's sections, tables and objects.
//! - `scene`: the document walk that turns objects into entities.
//! - `import`: the reader, entities to art, blocks to symbols.

mod aci;
pub mod import;
mod scene;
mod writer;

use vectorcraft_doc::{Document, Unit};
use vectorcraft_geom::Rect;

pub use aci::{aci_rgb, nearest_aci};
pub use import::{DxfInfo, ImportOptions, Imported, import, info, is_dxf};

/// How deep symbols, blocks and brush art may nest in one another (deeper art is left out).
const MAX_NEST: u32 = 8;
/// Cap height as a fraction of the type size: the height of CAD text is that of its capitals.
const CAP_HEIGHT: f64 = 0.7;

/// The `$INSUNITS` codes of the units documents use (feet and inches write feet).
const INSUNITS: [(u8, Unit); 7] =
    [(1, Unit::Inches), (2, Unit::Feet), (2, Unit::FeetInches), (4, Unit::Millimeters), (5, Unit::Centimeters), (6, Unit::Meters), (10, Unit::Yards)];

/// The `$INSUNITS` code of `unit` (none for points, picas and pixels).
fn insunits_code(unit: Unit) -> Option<u8> {
    INSUNITS.iter().find(|(_, u)| *u == unit).map(|(c, _)| *c)
}

/// The DXF version a file is written for (the `$ACADVER` it declares).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum DxfVersion {
    R12,
    R13,
    R14,
    R2000,
    R2004,
    R2007,
    R2010,
    R2013,
    #[default]
    R2018,
}

impl DxfVersion {
    /// Newest first, as version menus list them.
    pub const ALL: [Self; 9] = [Self::R2018, Self::R2013, Self::R2010, Self::R2007, Self::R2004, Self::R2000, Self::R14, Self::R13, Self::R12];

    /// The `version` param value.
    pub fn id(self) -> &'static str {
        match self {
            Self::R12 => "R12",
            Self::R13 => "R13",
            Self::R14 => "R14",
            Self::R2000 => "2000",
            Self::R2004 => "2004",
            Self::R2007 => "2007",
            Self::R2010 => "2010",
            Self::R2013 => "2013",
            Self::R2018 => "2018",
        }
    }

    /// The releases that write this version (menus).
    pub fn label(self) -> &'static str {
        match self {
            Self::R12 => "R12",
            Self::R13 => "R13",
            Self::R14 => "R14",
            Self::R2000 => "2000–2002",
            Self::R2004 => "2004–2006",
            Self::R2007 => "2007–2009",
            Self::R2010 => "2010–2012",
            Self::R2013 => "2013–2017",
            Self::R2018 => "2018",
        }
    }

    /// The `$ACADVER` header value.
    pub fn acadver(self) -> &'static str {
        match self {
            Self::R12 => "AC1009",
            Self::R13 => "AC1012",
            Self::R14 => "AC1014",
            Self::R2000 => "AC1015",
            Self::R2004 => "AC1018",
            Self::R2007 => "AC1021",
            Self::R2010 => "AC1024",
            Self::R2013 => "AC1027",
            Self::R2018 => "AC1032",
        }
    }

    /// A version by id (`2018`, `R2018`, `r12`) or `$ACADVER` (`AC1032`), any case.
    pub fn from_id(s: &str) -> Option<Self> {
        let s = s.trim();
        Self::ALL.into_iter().find(|v| {
            let id = v.id();
            s.eq_ignore_ascii_case(id) || s.eq_ignore_ascii_case(v.acadver()) || s.strip_prefix(['R', 'r']).is_some_and(|y| y == id)
        })
    }

    /// Objects carry handles and subclass markers; splines, hatches and images exist.
    fn handles(self) -> bool {
        self >= Self::R13
    }
    /// Lightweight polylines.
    fn lwpolyline(self) -> bool {
        self >= Self::R14
    }
    /// Lineweights, layouts, plot styles and long symbol names.
    fn r2000(self) -> bool {
        self >= Self::R2000
    }
    /// True colour and transparency.
    fn true_color(self) -> bool {
        self >= Self::R2004
    }
    /// Text is UTF-8 (earlier versions escape what isn't ASCII as `\U+XXXX`).
    fn utf8(self) -> bool {
        self >= Self::R2007
    }
}

/// How many colours entities may use: indexed colours of the 256-colour palette, or true colour
/// (with the nearest index for apps that read only the index).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorDepth {
    /// Indices 1–8: the seven standard colours and dark grey.
    Aci8,
    /// Indices 1–9, orange (30) and the six greys (250–255).
    Aci16,
    /// Every index 1–255.
    Aci256,
    /// 24-bit colour (DXF 2004 and later).
    #[default]
    True,
}

impl ColorDepth {
    pub const ALL: [Self; 4] = [Self::Aci8, Self::Aci16, Self::Aci256, Self::True];

    /// The `colors` param value.
    pub fn id(self) -> &'static str {
        match self {
            Self::Aci8 => "8",
            Self::Aci16 => "16",
            Self::Aci256 => "256",
            Self::True => "true",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Aci8 => "8 Colors",
            Self::Aci16 => "16 Colors",
            Self::Aci256 => "256 Colors",
            Self::True => "True Colors",
        }
    }

    /// A depth by id (`8`, `16`, `256`, `true`; `truecolor` and `24` also read).
    pub fn from_id(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        match s.as_str() {
            "truecolor" | "truecolors" | "true colors" | "24" => Some(Self::True),
            _ => Self::ALL.into_iter().find(|d| d.id() == s),
        }
    }
}

/// The file format placed images are written in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RasterFormat {
    #[default]
    Png,
    /// No transparency: transparent pixels show white.
    Jpeg,
}

impl RasterFormat {
    pub const ALL: [Self; 2] = [Self::Png, Self::Jpeg];

    /// The `rasterFormat` param value.
    pub fn id(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
        }
    }

    /// The extension the image files get.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }

    /// A format by id (`png`, `jpeg`, `jpg`), any case.
    pub fn from_id(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("jpg") {
            return Some(Self::Jpeg);
        }
        Self::ALL.into_iter().find(|f| s.eq_ignore_ascii_case(f.id()))
    }
}

/// What the export favours when DXF can't hold an object's look as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preserve {
    /// Type becomes glyph outlines; strokes a plain CAD line can't draw (aligned inside or
    /// outside, width profiles, arrowheads, fitted dashes, dashes offset into their pattern)
    /// become their filled outlines; brush
    /// strokes become their brush art.
    #[default]
    Appearance,
    /// Type stays text; every stroke is a line with its weight and dashes.
    Editability,
}

impl Preserve {
    pub const ALL: [Self; 2] = [Self::Appearance, Self::Editability];

    /// The `preserve` param value.
    pub fn id(self) -> &'static str {
        match self {
            Self::Appearance => "appearance",
            Self::Editability => "editability",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Preserve Appearance",
            Self::Editability => "Maximize Editability",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| s.trim().eq_ignore_ascii_case(p.id()))
    }
}

/// The DXF export settings.
#[derive(Clone, Debug)]
pub struct DxfOptions {
    pub version: DxfVersion,
    /// The unit of the art that `scale` drawing units stand for.
    pub unit: Unit,
    /// Drawing units per `unit` (1 mm = `scale` units).
    pub scale: f64,
    /// Lineweights grow with `scale` (else they keep their printed width).
    pub scale_lineweights: bool,
    pub colors: ColorDepth,
    pub raster: RasterFormat,
    pub preserve: Preserve,
    /// Alter Paths for Appearance: a stroke a CAD line can't draw (one [`Preserve::Appearance`]
    /// outlines, one wider than the widest lineweight, or any stroke in R13 and R14, which have
    /// no lineweights) is written as the filled area it paints.
    pub alter_paths: bool,
    /// Type is written as glyph outlines (always with [`Preserve::Appearance`]).
    pub outline_text: bool,
    /// The drawing's origin is this region's bottom-left corner (document space).
    pub region: Rect,
    /// Leave out objects entirely outside `region` (one drawing per artboard).
    pub crop: bool,
}

impl Default for DxfOptions {
    fn default() -> Self {
        Self {
            version: DxfVersion::default(),
            unit: Unit::Millimeters,
            scale: 1.0,
            scale_lineweights: false,
            colors: ColorDepth::default(),
            raster: RasterFormat::default(),
            preserve: Preserve::default(),
            alter_paths: false,
            outline_text: false,
            region: Rect::ZERO,
            crop: false,
        }
    }
}

/// An image file a drawing links to, to be written next to it under `name`.
#[derive(Clone, Debug)]
pub struct DxfImage {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// A written drawing.
#[derive(Clone, Debug, Default)]
pub struct DxfOutput {
    pub bytes: Vec<u8>,
    /// The image files the drawing links to (by name, next to it).
    pub images: Vec<DxfImage>,
    /// What was approximated or left out.
    pub warnings: Vec<String>,
}

/// The smallest and largest `scale` (drawing units per unit).
pub const SCALE_RANGE: std::ops::RangeInclusive<f64> = 1e-6..=1e6;

/// Write `doc` as a DXF drawing with `opts`. Live geometry effects are applied first; template
/// layers, guides and hidden objects are left out.
pub fn export(doc: &Document, opts: &DxfOptions) -> Result<DxfOutput, String> {
    if !(opts.scale.is_finite() && SCALE_RANGE.contains(&opts.scale)) {
        return Err(format!("scale must be a number from {} to {}, not {}", SCALE_RANGE.start(), SCALE_RANGE.end(), opts.scale));
    }
    let r = opts.region;
    if ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) {
        return Err("the export region is not finite".into());
    }
    let baked = vectorcraft_effects::bake_document(doc);
    let doc = baked.as_ref().unwrap_or(doc);
    Ok(scene::Scene::new(doc, opts).run())
}

#[cfg(test)]
mod tests;
