//! Swatch library files: the native `.vcswatches` JSON (colour models, global, spot, gradients
//! and colour groups exactly), `.gpl` palettes (8-bit RGB; colour groups as `# Group:` comment
//! headers), swatch exchange `.ase` files (binary: solid colors in their own model, global, spot
//! and process colors, color groups), color books `.acb` (read only: the books a user owns, such
//! as a Pantone book) and CSS custom properties (written only).

use std::collections::{HashMap, HashSet};

pub use craft_cms::palette::css_property;
use craft_cms::palette::{self as cms_palette, Palette, PaletteColor, PaletteError, PaletteGroup, PaletteSwatch};
use serde::{Deserialize, Serialize};

use crate::{Color, GradientKind, Paint, Swatch, SwatchGroup, SwatchLibrary};

/// A swatch library file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteFormat {
    /// `.vcswatches`: JSON, lossless.
    Native,
    /// `.gpl`: a plain-text RGB palette many paint and design tools read.
    Gpl,
    /// `.ase`: a swatch exchange file (binary): solid colors in their own model, global, spot or
    /// process, and color groups.
    Ase,
    /// `.css`: custom properties on `:root` (colours and gradients; written only).
    Css,
}

impl PaletteFormat {
    pub const ALL: [PaletteFormat; 4] = [PaletteFormat::Native, PaletteFormat::Gpl, PaletteFormat::Ase, PaletteFormat::Css];

    /// The format's id, which is also its extension.
    pub fn id(self) -> &'static str {
        match self {
            PaletteFormat::Native => "vcswatches",
            PaletteFormat::Gpl => "gpl",
            PaletteFormat::Ase => "ase",
            PaletteFormat::Css => "css",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            PaletteFormat::Native => "VectorCraft Swatches (.vcswatches)",
            PaletteFormat::Gpl => "GPL Palette (.gpl)",
            PaletteFormat::Ase => "Swatch Exchange (.ase)",
            PaletteFormat::Css => "CSS Custom Properties (.css)",
        }
    }
    /// The format with id or extension `s` (any case, leading dot allowed).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim_start_matches('.').to_ascii_lowercase();
        Self::ALL.into_iter().find(|f| f.id() == s)
    }
    /// Can libraries be read back from this format?
    pub fn readable(self) -> bool {
        self != PaletteFormat::Css
    }
    /// Are files of this format binary rather than text?
    pub fn binary(self) -> bool {
        self == PaletteFormat::Ase
    }
    /// Does [`write`] keep a swatch with `paint` in a file of this format? None and patterns are
    /// never written; `.gpl` and `.ase` hold solid colors only, and CSS holds no freeform gradients.
    pub fn holds(self, paint: &Paint) -> bool {
        match paint {
            Paint::Solid { .. } => true,
            Paint::Gradient(g) => match self {
                PaletteFormat::Native => true,
                PaletteFormat::Css => g.gradient.kind != GradientKind::Freeform,
                PaletteFormat::Gpl | PaletteFormat::Ase => false,
            },
            Paint::None | Paint::Pattern { .. } => false,
        }
    }
}

/// The native file: a header around the library.
#[derive(Serialize, Deserialize)]
struct NativeFile {
    format: String,
    version: u32,
    #[serde(flatten)]
    library: SwatchLibrary,
}

const NATIVE_FORMAT: &str = "vcswatches";
const GPL_HEADER: &str = "GIMP Palette";
const ASE_SIGNATURE: &[u8] = b"ASEF";
const ACB_SIGNATURE: &[u8] = b"8BCB";
#[cfg(test)]
const ASE_GROUP_START: u16 = 0xC001;
#[cfg(test)]
const ASE_MAX_ENTRIES: usize = 100_000;

fn color_to_cms(c: Color) -> PaletteColor {
    match c {
        Color::Rgb { r, g, b } => PaletteColor::Rgb { r, g, b },
        Color::Cmyk { c, m, y, k } => PaletteColor::Cmyk { c, m, y, k },
        Color::Lab { l, a, b } => PaletteColor::Lab { l, a, b },
        Color::Gray { k } => PaletteColor::Gray { k },
    }
}

fn color_from_cms(c: PaletteColor) -> Color {
    match c {
        PaletteColor::Rgb { r, g, b } => Color::rgb(r, g, b),
        PaletteColor::Cmyk { c, m, y, k } => Color::cmyk(c, m, y, k),
        PaletteColor::Lab { l, a, b } => Color::lab(l, a, b),
        PaletteColor::Gray { k } => Color::gray(k),
    }
}

// GPL must retain the app-managed display conversion; ASE preserves authoring models.
fn swatches_to_cms(list: &[Swatch], rgb_only: bool) -> Vec<PaletteSwatch> {
    list.iter()
        .filter_map(|w| {
            w.paint.color().map(|c| PaletteSwatch {
                name: w.name.clone(),
                color: if rgb_only {
                    let [r, g, b, _] = c.to_rgba8(1.0);
                    PaletteColor::rgb8(r, g, b)
                } else {
                    color_to_cms(c)
                },
                global: w.global,
                spot: w.spot,
            })
        })
        .collect()
}

fn swatches_from_cms(list: Vec<PaletteSwatch>) -> Vec<Swatch> {
    list.into_iter().map(|w| Swatch { name: w.name, paint: Paint::solid(color_from_cms(w.color)), global: w.global, spot: w.spot }).collect()
}

fn to_cms_palette(lib: &SwatchLibrary, rgb_only: bool) -> Palette {
    Palette {
        name: lib.name.clone(),
        swatches: swatches_to_cms(&lib.swatches, rgb_only),
        groups: lib.groups.iter().map(|g| PaletteGroup { name: g.name.clone(), swatches: swatches_to_cms(&g.swatches, rgb_only) }).collect(),
    }
}

fn from_cms_palette(p: Palette) -> SwatchLibrary {
    SwatchLibrary {
        name: p.name,
        swatches: swatches_from_cms(p.swatches),
        groups: p.groups.into_iter().map(|g| SwatchGroup { name: g.name, swatches: swatches_from_cms(g.swatches) }).collect(),
    }
}

/// Maps structured shared palette errors to VectorCraft's translatable messages.
fn palette_error(err: PaletteError) -> String {
    match err {
        PaletteError::UnsupportedFormat => "not a supported palette (.gpl, .ase, or .acb)".into(),
        PaletteError::TruncatedAse => "the swatch exchange file is cut short".into(),
        PaletteError::TruncatedAcb => "the color book is cut short".into(),
        PaletteError::AseWriteEntryLimit { limit, entries } => format!(
            "VectorCraft reads at most {ASE_MAX_ENTRIES} colors and groups from a swatch exchange file, and this library has {entries}",
            ASE_MAX_ENTRIES = limit
        ),
        PaletteError::AseReadEntryLimit { limit } => {
            format!("the swatch exchange file holds more than {ASE_MAX_ENTRIES} colors and groups", ASE_MAX_ENTRIES = limit)
        }
        PaletteError::AseBlockCount => "too many swatch exchange blocks".into(),
        PaletteError::AseBlockLength => "a swatch exchange block is too long".into(),
        PaletteError::AseNameLength => "a swatch exchange name is too long".into(),
        PaletteError::AseVersion { major, minor } => format!("swatch exchange version {major}.{minor} isn't supported (only 1.x)"),
        PaletteError::AcbVersion { version } => format!("color book version {version} isn't supported (only 1)"),
        PaletteError::AcbEntryLimit { limit } => format!("the color book holds more than {ASE_MAX_ENTRIES} colors", ASE_MAX_ENTRIES = limit),
        PaletteError::AcbColorSpace { space: other } => format!("color books in color space {other} aren't supported (only RGB, CMYK and Lab)"),
        PaletteError::AcbNameBytes { limit } => format!("the color book's names take more than {} MB", limit >> 20),
    }
}

/// Write `lib` in `format` → the file's bytes. Pattern swatches (their tiles live in a document)
/// and None are left out; `.gpl` keeps solid colours only, as 8-bit RGB, and `.ase` keeps solid
/// colors in their own model. Writing `.ase` fails for a library with more colors and groups than
/// [`read_bytes`] reads from one file.
pub fn write(lib: &SwatchLibrary, format: PaletteFormat) -> Result<Vec<u8>, String> {
    let keep = |w: &&Swatch| format.holds(&w.paint);
    let text = match format {
        PaletteFormat::Native => {
            let strip = |list: &[Swatch]| list.iter().filter(keep).cloned().collect::<Vec<_>>();
            let library = SwatchLibrary {
                name: lib.name.clone(),
                swatches: strip(&lib.swatches),
                groups: lib.groups.iter().map(|g| SwatchGroup { name: g.name.clone(), swatches: strip(&g.swatches) }).collect(),
            };
            let file = NativeFile { format: NATIVE_FORMAT.into(), version: 1, library };
            serde_json::to_string_pretty(&file).unwrap_or_default()
        }
        PaletteFormat::Gpl => {
            return cms_palette::write_palette_typed(&to_cms_palette(lib, true), cms_palette::PaletteFormat::Gpl).map_err(palette_error);
        }
        PaletteFormat::Ase => {
            return cms_palette::write_palette_typed(&to_cms_palette(lib, false), cms_palette::PaletteFormat::Ase).map_err(palette_error);
        }
        PaletteFormat::Css => write_css(lib, keep),
    };
    Ok(text.into_bytes())
}

/// One line of text: no line breaks or tabs.
fn one_line(s: &str) -> String {
    s.split(['\n', '\r', '\t']).collect::<Vec<_>>().join(" ").trim().to_string()
}

/// A colour as CSS (`#rrggbb`, or `#rrggbbaa` below full opacity).
fn css_color(c: &Color, opacity: f32) -> String {
    let [r, g, b, a] = c.to_rgba8(opacity);
    if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
}

/// A comment's text: `*/` can't end it early.
fn css_comment(s: &str) -> String {
    one_line(s).replace("*/", "* /")
}

fn write_css(lib: &SwatchLibrary, keep: impl Fn(&&Swatch) -> bool) -> String {
    let mut out = format!("/* {} */\n:root {{\n", css_comment(&lib.name));
    let mut taken: HashSet<String> = HashSet::new();
    // The next number to try for each base, so repeated names don't search the ones used before.
    let mut next: HashMap<String, usize> = HashMap::new();
    let mut props = |out: &mut String, list: &[Swatch]| {
        for w in list.iter().filter(&keep) {
            let value = match &w.paint {
                Paint::Solid { color, .. } => css_color(color, 1.0),
                Paint::Gradient(g) if g.gradient.kind != GradientKind::Freeform => {
                    // A mirrored conic turns counterclockwise, and CSS only turns clockwise: its stops
                    // are written mirrored about the seam instead.
                    let mirrored = g.gradient.kind == GradientKind::Conic && g.geom.is_some_and(|geom| geom.is_mirrored());
                    let stop = |&(t, c, o): &(f32, Color, f32)| {
                        let at = if mirrored { 1.0 - t } else { t };
                        format!("{} {}%", css_color(&c, o), at * 100.0)
                    };
                    // Stops that repeat or reflect run that many times across the box. CSS mixes
                    // hex colors evenly in sRGB between two stops
                    // (https://www.w3.org/TR/css-color-4/#interpolation-space), so an off-center
                    // midpoint and the curve of a Perceptual or hue gradient are written as stops
                    // of their own, each with its color and opacity. A gradient that would need
                    // more than `MAX_SPREAD_STOPS` stops that way is written with its own stops.
                    let unrolled = g.gradient.unrolled();
                    let expanded: Vec<(f32, Color, f32)> = if unrolled.expanded().nth(crate::gradient::MAX_SPREAD_STOPS).is_none() {
                        unrolled.expanded_stops()
                    } else {
                        unrolled.stops.iter().map(|s| (s.offset, s.color, s.opacity)).collect()
                    };
                    let stops: Vec<String> = if mirrored { expanded.iter().rev().map(stop).collect() } else { expanded.iter().map(stop).collect() };
                    match g.gradient.kind {
                        GradientKind::Radial => format!("radial-gradient(circle, {})", stops.join(", ")),
                        // CSS sweeps clockwise from straight up; the angle is counted counterclockwise
                        // from the right.
                        GradientKind::Conic => {
                            format!("conic-gradient(from {}deg, {})", (90.0 - g.angle).rem_euclid(360.0).round(), stops.join(", "))
                        }
                        // CSS has no diamond gradient: four layers, each a quarter of the box with
                        // the stops running from the center out to its corner at a diagonal angle,
                        // make the diamond its fit gives a box of any shape: the level at a point
                        // is its distance from the center along both axes over the box's
                        // half-width plus half-height
                        // (https://www.w3.org/TR/css-images-3/#linear-gradients). Turned by an angle
                        // that is not a multiple of 90 degrees, its corners leave the box's axes,
                        // and a circle stands in for it, as for a radial swatch.
                        GradientKind::Diamond if (1e-9..90.0 - 1e-9).contains(&g.angle.rem_euclid(90.0)) => {
                            format!("radial-gradient(circle, {})", stops.join(", "))
                        }
                        GradientKind::Diamond => {
                            let stops = stops.join(", ");
                            [("top left", 315), ("top right", 45), ("bottom right", 135), ("bottom left", 225)]
                                .map(|(corner, deg)| format!("linear-gradient({deg}deg, {stops}) {corner} / 50% 50% no-repeat"))
                                .join(", ")
                        }
                        _ => format!("linear-gradient({}deg, {})", (90.0 - g.angle).rem_euclid(360.0).round(), stops.join(", ")),
                    }
                }
                _ => continue,
            };
            // Names differing only in case or punctuation map to one property: number the others.
            let base = css_property(&w.name);
            let name = if taken.insert(base.clone()) {
                base
            } else {
                let n = next.entry(base.clone()).or_insert(1);
                loop {
                    *n += 1;
                    let name = format!("{base}-{n}");
                    if taken.insert(name.clone()) {
                        break name;
                    }
                }
            };
            let note = if w.spot { " /* spot */" } else { "" };
            out.push_str(&format!("  {name}: {value};{note}\n"));
        }
    };
    props(&mut out, &lib.swatches);
    for g in &lib.groups {
        out.push_str(&format!("  /* {} */\n", css_comment(&g.name)));
        props(&mut out, &g.swatches);
    }
    out.push_str("}\n");
    out
}

/// Read a `.vcswatches` or `.gpl` library (detected from its content). `name` names it when the
/// file doesn't.
pub fn read(text: &str, name: &str) -> Result<SwatchLibrary, String> {
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if text.starts_with(GPL_HEADER) {
        return cms_palette::read_palette_typed(text.as_bytes(), name).map(from_cms_palette).map_err(palette_error);
    }
    if text.starts_with('{') {
        let f: NativeFile = serde_json::from_str(text).map_err(|e| format!("not a swatch library: {e}"))?;
        if f.format != NATIVE_FORMAT {
            return Err(format!("not a swatch library (format `{}`)", f.format));
        }
        let mut lib = f.library;
        if lib.name.trim().is_empty() {
            lib.name = name.into();
        }
        return Ok(lib);
    }
    Err("not a swatch library (.vcswatches or .gpl)".into())
}

/// Is `text` a library [`read`] understands?
pub fn sniff(text: &str) -> bool {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    t.starts_with(GPL_HEADER) || (t.starts_with('{') && t.contains(NATIVE_FORMAT))
}

/// Read the bytes of a library file: a swatch exchange `.ase` file, else a `.vcswatches` or
/// `.gpl` library ([`read`]). `name` names it when the file doesn't.
pub fn read_bytes(bytes: &[u8], name: &str) -> Result<SwatchLibrary, String> {
    if bytes.starts_with(ASE_SIGNATURE) || bytes.starts_with(ACB_SIGNATURE) {
        return cms_palette::read_palette_typed(bytes, name).map(from_cms_palette).map_err(palette_error);
    }
    read(&String::from_utf8_lossy(bytes), name)
}

/// Are `bytes` a library [`read_bytes`] understands? Text that isn't valid UTF-8 is checked as
/// [`read_bytes`] reads it, with the invalid bytes replaced (its first KiB is enough).
pub fn sniff_bytes(bytes: &[u8]) -> bool {
    bytes.starts_with(ASE_SIGNATURE)
        || bytes.starts_with(ACB_SIGNATURE)
        || std::str::from_utf8(bytes).is_ok_and(sniff)
        || sniff(&String::from_utf8_lossy(bytes.get(..1024).unwrap_or(bytes)))
}

#[cfg(test)]
mod tests;
