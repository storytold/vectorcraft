//! Photoshop-style raster effects (Effect › Blur › Radial Blur and Smart Blur, Brush Strokes ›
//! Accented Edges, Angled Strokes, Crosshatch, Dark Strokes, Ink Outlines, Spatter, Sprayed Strokes
//! and Sumi-e, Distort › Diffuse Glow, Glass and Ocean Ripple, Pixelate › Color Halftone,
//! Crystallize, Mezzotint and Pointillize, Sharpen › Unsharp Mask, Stylize › Glowing Edges, Texture
//! › Craquelure, Grain, Mosaic Tiles, Patchwork, Stained Glass and Texturizer, and Video ›
//! De-Interlace and NTSC Colors): filters over premultiplied RGBA8 pixels.
//!
//! - Every distance is in document units (points) and becomes pixels through the raster's
//!   [`PixelSpace::px`], so an effect looks the same at any zoom and at any Document Raster Effects
//!   Settings resolution (as Gaussian Blur's radius does).
//! - The work per pixel is bounded whatever the parameters: blurs are box approximations whose
//!   cost doesn't grow with the radius, and sample counts come from the effect's quality.
//! - Beyond the raster's edges is transparency.

mod blur;
mod brushstrokes;
mod distort;
mod edges;
mod pixelate;
mod sharpen;
mod texture;
mod video;

use serde_json::Value;
use vectorcraft_geom::{Affine, Point, Rect};

use crate::util::{flag, num, text};

pub use brushstrokes::{STROKE_DIRECTIONS, StrokeDirection};
pub use distort::{GLASS_TEXTURES, GlassTexture};
pub use pixelate::{MEZZOTINT_TYPES, Mezzotint};
pub use texture::{GRAIN_TYPES, Grain, LIGHT_DIRECTIONS, Light, TEXTURES, Texture};

/// The Photoshop-style effect ids (all raster effects, see [`crate::is_raster`]).
pub const PIXEL_EFFECTS: [&str; 27] = [
    "blur.radial",
    "blur.smart",
    "brushStrokes.accentedEdges",
    "brushStrokes.angledStrokes",
    "brushStrokes.crosshatch",
    "brushStrokes.darkStrokes",
    "brushStrokes.inkOutlines",
    "brushStrokes.spatter",
    "brushStrokes.sprayedStrokes",
    "brushStrokes.sumiE",
    "distort.diffuseGlow",
    "distort.glass",
    "distort.oceanRipple",
    "pixelate.colorHalftone",
    "pixelate.crystallize",
    "pixelate.mezzotint",
    "pixelate.pointillize",
    "sharpen.unsharpMask",
    "stylize.glowingEdges",
    "texture.craquelure",
    "texture.grain",
    "texture.mosaicTiles",
    "texture.patchwork",
    "texture.stainedGlass",
    "texture.texturizer",
    "video.deinterlace",
    "video.ntscColors",
];

/// A Photoshop-style raster effect with its parameters read and clamped to their ranges. Lengths
/// are in document units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PixelFx {
    /// Blur › Radial Blur around the object's centre: along arcs (spin, an arc of `amount`°) or
    /// along rays (zoom, the content scaled by up to about ±`amount`/2 %), averaging 2^`passes` samples.
    RadialBlur { amount: f64, zoom: bool, passes: u32 },
    /// Blur › Smart Blur (Normal mode): each pixel averaged with the neighbours within `radius`
    /// whose colour differs from its own by no more than `threshold` levels, over a grid of
    /// `samples` × `samples` neighbours at most.
    SmartBlur { radius: f64, threshold: f64, samples: u32 },
    /// Brush Strokes › Accented Edges: the edges, about `width` (1..14) wide, accented towards
    /// white chalk or black ink as `brightness` (0..50) says; `smoothness` (1..15) softens them.
    AccentedEdges { width: f64, brightness: f64, smoothness: f64 },
    /// Brush Strokes › Angled Strokes: diagonal strokes `length` long, the light areas' and the
    /// dark areas' going opposite ways, split as `balance` (0..100) says, as crisp as `sharpness`
    /// (0..10).
    AngledStrokes { balance: f64, length: f64, sharpness: f64 },
    /// Brush Strokes › Crosshatch: pencil hatching `length` long along both diagonals, as crisp
    /// as `sharpness` (0..20), in `strength` (1..3) passes.
    Crosshatch { length: f64, sharpness: f64, strength: u32 },
    /// Brush Strokes › Dark Strokes: short strokes towards black in the dark areas, long ones
    /// towards white in the light areas, split by `balance` (0..10), as dark as `black` (0..10)
    /// and as light as `white` (0..10).
    DarkStrokes { balance: f64, black: f64, white: f64 },
    /// Brush Strokes › Ink Outlines: pen-and-ink outlines and strokes `length` long, the shadows
    /// darkened by `dark` (0..50) and the highlights lightened by `light` (0..50).
    InkOutlines { length: f64, dark: f64, light: f64 },
    /// Brush Strokes › Spatter: a spatter airbrush spraying within `radius` (0..25), its grains as
    /// smooth as `smoothness` (1..15).
    Spatter { radius: f64, smoothness: f64 },
    /// Brush Strokes › Sprayed Strokes: sprayed strokes `length` long along `direction`, scattered
    /// as `radius` (0..25) says.
    SprayedStrokes { length: f64, radius: f64, direction: StrokeDirection },
    /// Brush Strokes › Sumi-e: wet black brush strokes `width` (3..15) across on rice paper, as
    /// loaded as `pressure` (0..15) says, with `contrast` (0..40).
    SumiE { width: f64, pressure: f64, contrast: f64 },
    /// Distort › Diffuse Glow: the highlights glow white as strongly as `glow` (0..20), from the
    /// brightness `clear` (0..20) leaves clear, under white grain as dense as `graininess` (0..10).
    DiffuseGlow { graininess: f64, glow: f64, clear: f64 },
    /// Distort › Glass: the object seen through a `texture` surface scaled by `scaling` (0.5..2),
    /// bent by `distortion` (0..20), smoothed by `smoothness` (1..15), its heights turned over when
    /// `invert`.
    Glass { distortion: f64, smoothness: f64, texture: GlassTexture, scaling: f64, invert: bool },
    /// Distort › Ocean Ripple: the object under ripples of `size` (1..15) shifting it as much as
    /// `magnitude` (0..20) says.
    OceanRipple { size: f64, magnitude: f64 },
    /// Sharpen › Unsharp Mask: colours pushed away from a Gaussian blur of σ = `radius` by `amount`
    /// (1 = 100 %), where they differ from it by at least `threshold` levels.
    UnsharpMask { amount: f64, radius: f64, threshold: f64 },
    /// Stylize › Glowing Edges: bright coloured Sobel outlines on black.
    GlowingEdges { width: f64, brightness: f64, smoothness: f64 },
    /// Pixelate › Color Halftone: each colour channel screened at its angle (`angles`° of channels
    /// 1 to 4) into dots of up to `max_radius`, whose area follows the channel's strength.
    ColorHalftone { max_radius: f64, angles: [f64; 4] },
    /// Pixelate › Crystallize: polygons of solid colour around random points about `cell` apart.
    Crystallize { cell: f64 },
    /// Pixelate › Mezzotint: every colour channel fully on or off against a random pattern.
    Mezzotint { kind: Mezzotint },
    /// Pixelate › Pointillize: random dots about `cell` across of the object's colours on a white
    /// canvas.
    Pointillize { cell: f64 },
    /// Texture › Craquelure: cracked relief plaster, plates about `spacing` apart, cracks as deep
    /// as `depth` (0..10), lit as brightly as `brightness` (0..10).
    Craquelure { spacing: f64, depth: f64, brightness: f64 },
    /// Texture › Grain: noise of `kind` as strong as `intensity` (0..100), the image's contrast
    /// set by `contrast` (0..100, 50 leaves it).
    Grain { intensity: f64, contrast: f64, kind: Grain },
    /// Texture › Mosaic Tiles: irregular tiles about `tile` across with sunken grout `grout` / 2
    /// wide (1..15), lightened by `lighten` (0..10).
    MosaicTiles { tile: f64, grout: f64, lighten: f64 },
    /// Texture › Patchwork: squares `square` across of the colour around their centres, raised by
    /// up to `relief` (0..25).
    Patchwork { square: f64, relief: f64 },
    /// Texture › Stained Glass: panes about `cell` across leaded `border` / 2 wide (1..20) in
    /// black, lit at the centre by `light` (0..10).
    StainedGlass { cell: f64, border: f64, light: f64 },
    /// Texture › Texturizer: a `texture` surface scaled by `scaling` (0.5..2), in relief as strong
    /// as `relief` (0..50), lit from `light`, its heights turned over when `invert`.
    Texturizer { texture: Texture, scaling: f64, relief: f64, light: Light, invert: bool },
    /// Video › De-Interlace: the odd (or `even`) field lines made again from the others, by
    /// duplication or, with `interpolate`, by averaging.
    DeInterlace { even: bool, interpolate: bool },
    /// Video › NTSC Colors: colours a television signal can't carry made less saturated.
    NtscColors,
}

/// What a raster's colour channels hold, for the filters that treat them apart (Color Halftone
/// screens each channel, Mezzotint thresholds each).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Channels {
    /// Red, green and blue light.
    #[default]
    Rgb,
    /// Screen colours of a CMYK document: treated as its cyan, magenta, yellow and black inks.
    Cmyk,
    /// The complemented C, M and Y inks (a CMYK document drawn ink plane by ink plane).
    CmyPlane,
    /// The complemented K ink, as a grey.
    KPlane,
}

/// Where a raster's pixels lie in the document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelSpace {
    /// Pixel coordinates (x right, y down, (0, 0) the top-left corner of the first pixel) →
    /// document coordinates.
    pub to_doc: Affine,
    /// The size of a pixel in document units.
    pub px: f64,
    /// The object's centre (document coordinates): where Radial Blur blurs around, and where the
    /// Pixelate filters' patterns are anchored (so they move with the object).
    pub center: Point,
    /// What the colour channels hold.
    pub channels: Channels,
    /// The height of a line of the document's raster grid (document units: 72 / Document Raster
    /// Effects Settings › Resolution), the fields De-Interlace works on.
    pub line: f64,
    /// How far the corners of the object's bounds lie from its centre (document units): Stained
    /// Glass's light fades out over it.
    pub extent: f64,
}

/// Effect `id`'s parameters `p` (defaults merged in) as a [`PixelFx`]; `None` for other effects.
pub(crate) fn parse(id: &str, p: &Value) -> Option<PixelFx> {
    let quality = |levels: [(&str, u32); 3], default: u32| {
        let q = text(p, "quality", "");
        levels.iter().find(|(name, _)| name.eq_ignore_ascii_case(q)).map_or(default, |(_, v)| *v)
    };
    Some(match id {
        "blur.radial" => PixelFx::RadialBlur {
            amount: num(p, "amount", 10.0).clamp(1.0, 100.0),
            zoom: text(p, "method", "spin").eq_ignore_ascii_case("zoom"),
            passes: quality([("draft", 4), ("good", 6), ("best", 8)], 6),
        },
        "blur.smart" => PixelFx::SmartBlur {
            radius: num(p, "radius", 3.0).clamp(0.1, 100.0),
            threshold: num(p, "threshold", 25.0).clamp(0.1, 100.0),
            samples: quality([("low", 5), ("medium", 7), ("high", 9)], 7),
        },
        "brushStrokes.accentedEdges" => PixelFx::AccentedEdges {
            width: num(p, "edgeWidth", 2.0).clamp(1.0, 14.0),
            brightness: num(p, "edgeBrightness", 38.0).clamp(0.0, 50.0),
            smoothness: num(p, "smoothness", 5.0).clamp(1.0, 15.0),
        },
        "brushStrokes.angledStrokes" => PixelFx::AngledStrokes {
            balance: num(p, "directionBalance", 50.0).clamp(0.0, 100.0),
            length: num(p, "strokeLength", 15.0).clamp(3.0, 50.0),
            sharpness: num(p, "sharpness", 3.0).clamp(0.0, 10.0),
        },
        "brushStrokes.crosshatch" => PixelFx::Crosshatch {
            length: num(p, "strokeLength", 9.0).clamp(3.0, 50.0),
            sharpness: num(p, "sharpness", 6.0).clamp(0.0, 20.0),
            strength: num(p, "strength", 1.0).clamp(1.0, 3.0).round() as u32,
        },
        "brushStrokes.darkStrokes" => PixelFx::DarkStrokes {
            balance: num(p, "balance", 5.0).clamp(0.0, 10.0),
            black: num(p, "blackIntensity", 6.0).clamp(0.0, 10.0),
            white: num(p, "whiteIntensity", 2.0).clamp(0.0, 10.0),
        },
        "brushStrokes.inkOutlines" => PixelFx::InkOutlines {
            length: num(p, "strokeLength", 4.0).clamp(1.0, 50.0),
            dark: num(p, "darkIntensity", 20.0).clamp(0.0, 50.0),
            light: num(p, "lightIntensity", 10.0).clamp(0.0, 50.0),
        },
        "brushStrokes.spatter" => {
            PixelFx::Spatter { radius: num(p, "sprayRadius", 10.0).clamp(0.0, 25.0), smoothness: num(p, "smoothness", 5.0).clamp(1.0, 15.0) }
        }
        "brushStrokes.sprayedStrokes" => PixelFx::SprayedStrokes {
            length: num(p, "strokeLength", 12.0).clamp(0.0, 20.0),
            radius: num(p, "sprayRadius", 7.0).clamp(0.0, 25.0),
            direction: StrokeDirection::parse(text(p, "strokeDirection", "rightDiagonal")),
        },
        "brushStrokes.sumiE" => PixelFx::SumiE {
            width: num(p, "strokeWidth", 10.0).clamp(3.0, 15.0),
            pressure: num(p, "strokePressure", 2.0).clamp(0.0, 15.0),
            contrast: num(p, "contrast", 16.0).clamp(0.0, 40.0),
        },
        "distort.diffuseGlow" => PixelFx::DiffuseGlow {
            graininess: num(p, "graininess", 6.0).clamp(0.0, 10.0),
            glow: num(p, "glowAmount", 10.0).clamp(0.0, 20.0),
            clear: num(p, "clearAmount", 15.0).clamp(0.0, 20.0),
        },
        "distort.glass" => PixelFx::Glass {
            distortion: num(p, "distortion", 5.0).clamp(0.0, 20.0),
            smoothness: num(p, "smoothness", 3.0).clamp(1.0, 15.0),
            texture: GlassTexture::parse(text(p, "texture", "frosted")),
            scaling: num(p, "scaling", 100.0).clamp(50.0, 200.0) / 100.0,
            invert: flag(p, "invert", false),
        },
        "distort.oceanRipple" => {
            PixelFx::OceanRipple { size: num(p, "rippleSize", 9.0).clamp(1.0, 15.0), magnitude: num(p, "rippleMagnitude", 9.0).clamp(0.0, 20.0) }
        }
        "sharpen.unsharpMask" => PixelFx::UnsharpMask {
            amount: num(p, "amount", 50.0).clamp(1.0, 500.0) / 100.0,
            radius: num(p, "radius", 1.0).clamp(0.1, 250.0),
            threshold: num(p, "threshold", 0.0).clamp(0.0, 255.0),
        },
        "stylize.glowingEdges" => PixelFx::GlowingEdges {
            width: num(p, "edgeWidth", 2.0).clamp(1.0, 14.0),
            brightness: num(p, "edgeBrightness", 6.0).clamp(0.0, 20.0),
            smoothness: num(p, "smoothness", 5.0).clamp(1.0, 15.0),
        },
        "pixelate.colorHalftone" => PixelFx::ColorHalftone {
            max_radius: num(p, "maxRadius", 8.0).clamp(4.0, 127.0),
            angles: [("channel1", 108.0), ("channel2", 162.0), ("channel3", 90.0), ("channel4", 45.0)]
                .map(|(k, d)| num(p, k, d).clamp(-360.0, 360.0)),
        },
        "pixelate.crystallize" => PixelFx::Crystallize { cell: num(p, "cellSize", 10.0).clamp(3.0, 300.0) },
        "pixelate.mezzotint" => PixelFx::Mezzotint { kind: Mezzotint::parse(text(p, "type", "fineDots")) },
        "pixelate.pointillize" => PixelFx::Pointillize { cell: num(p, "cellSize", 5.0).clamp(3.0, 300.0) },
        "texture.craquelure" => PixelFx::Craquelure {
            spacing: num(p, "crackSpacing", 15.0).clamp(2.0, 100.0),
            depth: num(p, "crackDepth", 6.0).clamp(0.0, 10.0),
            brightness: num(p, "crackBrightness", 9.0).clamp(0.0, 10.0),
        },
        "texture.grain" => PixelFx::Grain {
            intensity: num(p, "intensity", 40.0).clamp(0.0, 100.0),
            contrast: num(p, "contrast", 50.0).clamp(0.0, 100.0),
            kind: Grain::parse(text(p, "grainType", "regular")),
        },
        "texture.mosaicTiles" => PixelFx::MosaicTiles {
            tile: num(p, "tileSize", 12.0).clamp(2.0, 100.0),
            grout: num(p, "groutWidth", 3.0).clamp(1.0, 15.0),
            lighten: num(p, "lightenGrout", 9.0).clamp(0.0, 10.0),
        },
        "texture.patchwork" => {
            PixelFx::Patchwork { square: num(p, "squareSize", 4.0).clamp(0.0, 10.0), relief: num(p, "relief", 8.0).clamp(0.0, 25.0) }
        }
        "texture.stainedGlass" => PixelFx::StainedGlass {
            cell: num(p, "cellSize", 10.0).clamp(2.0, 50.0),
            border: num(p, "borderThickness", 4.0).clamp(1.0, 20.0),
            light: num(p, "lightIntensity", 3.0).clamp(0.0, 10.0),
        },
        "texture.texturizer" => PixelFx::Texturizer {
            texture: Texture::parse(text(p, "texture", "canvas")),
            scaling: num(p, "scaling", 100.0).clamp(50.0, 200.0) / 100.0,
            relief: num(p, "relief", 4.0).clamp(0.0, 50.0),
            light: Light::parse(text(p, "lightDirection", "top")),
            invert: flag(p, "invert", false),
        },
        "video.deinterlace" => PixelFx::DeInterlace {
            even: text(p, "eliminate", "odd").eq_ignore_ascii_case("even"),
            interpolate: text(p, "create", "duplication").eq_ignore_ascii_case("interpolation"),
        },
        "video.ntscColors" => PixelFx::NtscColors,
        _ => return None,
    })
}

impl PixelFx {
    /// How far beyond `bounds` (the content's, document units) the effect can paint.
    pub fn outset(&self, bounds: Rect) -> f64 {
        // From the centre to the farthest corner, and to the nearest side.
        let far = 0.5 * bounds.width().hypot(bounds.height());
        let near = 0.5 * bounds.width().min(bounds.height());
        match *self {
            // A rotated corner reaches the circle through the corners.
            PixelFx::RadialBlur { zoom: false, .. } => (far - near).max(0.0),
            PixelFx::RadialBlur { amount, zoom: true, .. } => far * (blur::zoom_extent(amount).exp() - 1.0),
            PixelFx::SmartBlur { radius, .. } => radius,
            PixelFx::AccentedEdges { .. } => 0.0,
            // Strokes reach out of the object as far as they reach into it.
            PixelFx::AngledStrokes { length, .. } | PixelFx::InkOutlines { length, .. } => brushstrokes::stroke_reach(length),
            PixelFx::Crosshatch { length, .. } => brushstrokes::stroke_reach(length) + brushstrokes::rough(length),
            PixelFx::DarkStrokes { .. } => brushstrokes::stroke_reach(brushstrokes::LIGHT_LENGTH),
            PixelFx::Spatter { radius, .. } => brushstrokes::spatter_shift(radius),
            PixelFx::SprayedStrokes { length, radius, .. } => brushstrokes::stroke_reach(length) + brushstrokes::spray_shift(radius),
            // Sumi-e keeps the object's shape.
            PixelFx::SumiE { .. } => 0.0,
            PixelFx::DiffuseGlow { .. } => 0.0,
            // Content shifted outwards reaches past the bounds by as much as the shift.
            PixelFx::Glass { distortion, .. } => distort::glass_shift(distortion),
            PixelFx::OceanRipple { size, magnitude } => distort::ripple_shift(size, magnitude),
            PixelFx::UnsharpMask { .. } => 0.0,
            PixelFx::GlowingEdges { width, smoothness, .. } => width.max(smoothness * 3.0),
            PixelFx::ColorHalftone { .. }
            | PixelFx::Mezzotint { .. }
            | PixelFx::Craquelure { .. }
            | PixelFx::Grain { .. }
            | PixelFx::MosaicTiles { .. }
            | PixelFx::Texturizer { .. }
            | PixelFx::DeInterlace { .. }
            | PixelFx::NtscColors => 0.0,
            // A crystal's, dot's or pane's point inside the object reaches out by up to its cell.
            PixelFx::Crystallize { cell } | PixelFx::Pointillize { cell } | PixelFx::StainedGlass { cell, .. } => 1.5 * cell,
            // A square whose centre is on the object is drawn whole.
            PixelFx::Patchwork { square, .. } => square.max(1.0),
        }
    }

    /// How far (document units) from a pixel the content that decides it lies; `None` when that
    /// is anywhere in the object (Radial Blur).
    pub fn reach(&self) -> Option<f64> {
        match *self {
            PixelFx::RadialBlur { .. } => None,
            PixelFx::SmartBlur { radius, .. } => Some(radius),
            PixelFx::AccentedEdges { width, smoothness, .. } => Some(brushstrokes::accent_reach(width, smoothness)),
            // The strokes, and the softened tones that choose them.
            PixelFx::AngledStrokes { length, .. } => Some(brushstrokes::stroke_reach(length).max(brushstrokes::TONE_REACH)),
            PixelFx::Crosshatch { length, .. } => {
                Some((brushstrokes::stroke_reach(length) + brushstrokes::rough(length)).max(brushstrokes::TONE_REACH))
            }
            PixelFx::DarkStrokes { .. } => Some(brushstrokes::stroke_reach(brushstrokes::LIGHT_LENGTH).max(brushstrokes::TONE_REACH)),
            PixelFx::InkOutlines { length, .. } => Some(brushstrokes::ink_reach(length)),
            PixelFx::Spatter { radius, .. } => Some(brushstrokes::spatter_shift(radius)),
            PixelFx::SprayedStrokes { length, radius, .. } => Some(brushstrokes::stroke_reach(length) + brushstrokes::spray_shift(radius)),
            PixelFx::SumiE { width, .. } => Some(brushstrokes::sumi_reach(width)),
            PixelFx::DiffuseGlow { .. } => Some(3.0 * distort::GLOW_SPREAD),
            PixelFx::Glass { distortion, .. } => Some(distort::glass_shift(distortion)),
            PixelFx::OceanRipple { size, magnitude } => Some(distort::ripple_shift(size, magnitude)),
            PixelFx::UnsharpMask { radius, .. } => Some(3.0 * radius),
            PixelFx::GlowingEdges { width, smoothness, .. } => Some(width.max(smoothness * 3.0)),
            // The cells around the pixel's, and the cells around theirs.
            PixelFx::ColorHalftone { max_radius, .. } => Some(3.0 * std::f64::consts::SQRT_2 * max_radius),
            PixelFx::Crystallize { cell } => Some(1.5 * cell),
            PixelFx::Mezzotint { .. } => Some(0.0),
            // The dots' points, and the softened colour around them.
            PixelFx::Pointillize { cell } => Some(2.5 * cell),
            // The softened tones around the pixel.
            PixelFx::Craquelure { spacing, .. } => Some(3.0 * texture::CONTOUR_SOFTNESS * spacing),
            PixelFx::Grain { .. } | PixelFx::MosaicTiles { .. } | PixelFx::Texturizer { .. } => Some(0.0),
            // The square's or pane's point, and the softened colour around it.
            PixelFx::Patchwork { square, .. } => Some(2.0 * square.max(1.0)),
            PixelFx::StainedGlass { cell, .. } => Some(2.5 * cell),
            // The lines above and below.
            PixelFx::DeInterlace { .. } => Some(video::MAX_LINE),
            PixelFx::NtscColors => Some(0.0),
        }
    }

    /// Run the effect on `data`, `w` × `h` premultiplied RGBA8 pixels lying in the document as
    /// `space` says. Does nothing when the sizes don't match.
    pub fn apply(&self, data: &mut [u8], w: usize, h: usize, space: &PixelSpace) {
        let Some(px) = pixels(data, w, h) else { return };
        let to_px = |len: f64| len / space.px.max(1e-9);
        match *self {
            PixelFx::RadialBlur { amount, zoom, passes } => blur::radial(px, w, h, space, amount, zoom, passes),
            PixelFx::SmartBlur { radius, threshold, samples } => blur::smart(px, w, h, to_px(radius), threshold, samples),
            PixelFx::AccentedEdges { width, brightness, smoothness } => brushstrokes::accented_edges(px, w, h, space, width, brightness, smoothness),
            PixelFx::AngledStrokes { balance, length, sharpness } => brushstrokes::angled_strokes(px, w, h, space, balance, length, sharpness),
            PixelFx::Crosshatch { length, sharpness, strength } => brushstrokes::crosshatch(px, w, h, space, length, sharpness, strength),
            PixelFx::DarkStrokes { balance, black, white } => brushstrokes::dark_strokes(px, w, h, space, balance, black, white),
            PixelFx::InkOutlines { length, dark, light } => brushstrokes::ink_outlines(px, w, h, space, length, dark, light),
            PixelFx::Spatter { radius, smoothness } => brushstrokes::spatter(px, w, h, space, radius, smoothness),
            PixelFx::SprayedStrokes { length, radius, direction } => brushstrokes::sprayed_strokes(px, w, h, space, length, radius, direction),
            PixelFx::SumiE { width, pressure, contrast } => brushstrokes::sumi_e(px, w, h, space, width, pressure, contrast),
            PixelFx::DiffuseGlow { graininess, glow, clear } => distort::diffuse_glow(px, w, h, space, graininess, glow, clear),
            PixelFx::Glass { distortion, smoothness, texture, scaling, invert } => {
                distort::glass(px, w, h, space, distortion, smoothness, texture, scaling, invert)
            }
            PixelFx::OceanRipple { size, magnitude } => distort::ocean_ripple(px, w, h, space, size, magnitude),
            PixelFx::UnsharpMask { amount, radius, threshold } => sharpen::unsharp(px, w, h, amount, to_px(radius), threshold),
            PixelFx::GlowingEdges { width, brightness, smoothness } => edges::glow(px, w, h, to_px(width), brightness, to_px(smoothness)),
            PixelFx::ColorHalftone { max_radius, angles } => pixelate::color_halftone(px, w, h, space, max_radius, angles),
            PixelFx::Crystallize { cell } => pixelate::crystallize(px, w, h, space, cell),
            PixelFx::Mezzotint { kind } => pixelate::mezzotint(px, w, h, space, kind),
            PixelFx::Pointillize { cell } => pixelate::pointillize(px, w, h, space, cell),
            PixelFx::Craquelure { spacing, depth, brightness } => texture::craquelure(px, w, h, space, spacing, depth, brightness),
            PixelFx::Grain { intensity, contrast, kind } => texture::grain(px, w, h, space, intensity, contrast, kind),
            PixelFx::MosaicTiles { tile, grout, lighten } => texture::mosaic_tiles(px, w, h, space, tile, grout, lighten),
            PixelFx::Patchwork { square, relief } => texture::patchwork(px, w, h, space, square, relief),
            PixelFx::StainedGlass { cell, border, light } => texture::stained_glass(px, w, h, space, cell, border, light),
            PixelFx::Texturizer { texture, scaling, relief, light, invert } => {
                texture::texturizer(px, w, h, space, texture, scaling, relief, light, invert)
            }
            PixelFx::DeInterlace { even, interpolate } => video::deinterlace(px, w, h, space, even, interpolate),
            PixelFx::NtscColors => video::ntsc(px),
        }
    }
}

/// The entry of `all` whose value in `table` (the same order) is `value`, any case; `default`
/// otherwise.
fn pick<T: Copy>(table: &[(&str, &str)], all: &[T], value: &str, default: T) -> T {
    table.iter().zip(all).find(|((_, v), _)| v.eq_ignore_ascii_case(value)).map_or(default, |(_, t)| *t)
}

/// `data` as `w` × `h` RGBA pixels, `None` when it isn't that size (or is empty).
fn pixels(data: &mut [u8], w: usize, h: usize) -> Option<&mut [[u8; 4]]> {
    let (px, rest) = data.as_chunks_mut::<4>();
    (w > 0 && h > 0 && rest.is_empty() && w.checked_mul(h) == Some(px.len())).then_some(px)
}

/// The largest box radius the blurs use: wider boxes look the same on any raster we make.
const MAX_BOX: usize = 1 << 17;

/// Radii of three box blurs approximating a Gaussian of `sigma` pixels.
pub fn gauss_boxes(sigma: f64) -> [usize; 3] {
    let sigma = if sigma.is_finite() { sigma.clamp(0.0, MAX_BOX as f64 / 2.0) } else { 0.0 };
    let n = 3.0;
    let w_ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let m_ideal = (12.0 * sigma * sigma - n * (wl * wl) as f64 - 4.0 * n * wl as f64 - 3.0 * n) / (-4.0 * wl as f64 - 4.0);
    let m = m_ideal.round() as i64;
    let r = |i: i64| ((((if i < m { wl } else { wu }) - 1) / 2).max(0) as usize).min(MAX_BOX);
    [r(0), r(1), r(2)]
}

/// One horizontal box blur of radius `r` over the rows of `w` values of a plane (running sum,
/// zero past the ends).
fn box_plane(src: &[f32], dst: &mut [f32], w: usize, r: usize) {
    if r == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let norm = 1.0 / (2 * r + 1) as f32;
    for (row, out) in src.chunks_exact(w).zip(dst.chunks_exact_mut(w)) {
        let mut acc: f32 = row.iter().take(r + 1).sum();
        for (x, o) in out.iter_mut().enumerate() {
            *o = acc * norm;
            if let Some(v) = row.get(x + r + 1) {
                acc += v;
            }
            if let Some(v) = x.checked_sub(r).and_then(|i| row.get(i)) {
                acc -= v;
            }
        }
    }
}

fn transpose<T: Copy>(src: &[T], dst: &mut [T], w: usize, h: usize) {
    for (y, row) in src.chunks_exact(w).enumerate().take(h) {
        for (x, v) in row.iter().enumerate() {
            if let Some(d) = dst.get_mut(x * h + y) {
                *d = *v;
            }
        }
    }
}

/// Gaussian-blur a `w` × `h` plane of values in place (three box passes per axis, zero past the
/// edges). Does nothing when the sizes don't match.
pub fn blur_plane(a: &mut Vec<f32>, w: usize, h: usize, sigma: f64) {
    if sigma < 0.2 || w == 0 || h == 0 || w.checked_mul(h) != Some(a.len()) {
        return;
    }
    let boxes = gauss_boxes(sigma);
    let mut tmp = vec![0.0; a.len()];
    for (len, other) in [(w, h), (h, w)] {
        for r in boxes {
            box_plane(a, &mut tmp, len, r);
            std::mem::swap(a, &mut tmp);
        }
        transpose(a, &mut tmp, len, other);
        std::mem::swap(a, &mut tmp);
    }
}

/// Premultiplied RGBA at 16 bits (×257): blurs and averages that keep their tones.
pub(super) type Px16 = [u16; 4];

pub(super) fn to16(px: &[[u8; 4]]) -> Vec<Px16> {
    px.iter().map(|p| p.map(|v| v as u16 * 257)).collect()
}

pub(super) fn to8(v: u16) -> u8 {
    ((v as u32 + 128) / 257).min(255) as u8
}

fn add(acc: &mut [u64; 4], p: &Px16) {
    for (a, v) in acc.iter_mut().zip(p) {
        *a += *v as u64;
    }
}

fn sub(acc: &mut [u64; 4], p: &Px16) {
    for (a, v) in acc.iter_mut().zip(p) {
        *a = a.saturating_sub(*v as u64);
    }
}

fn mean(acc: &[u64; 4], n: u64) -> Px16 {
    acc.map(|a| ((a + n / 2) / n.max(1)).min(65535) as u16)
}

/// One box blur of radius `r` along the rows of `w` pixels (transparent past the ends).
fn box_rows(src: &[Px16], dst: &mut [Px16], w: usize, r: usize) {
    let n = 2 * r as u64 + 1;
    for (row, out) in src.chunks_exact(w).zip(dst.chunks_exact_mut(w)) {
        let mut acc = [0u64; 4];
        row.iter().take(r + 1).for_each(|p| add(&mut acc, p));
        for (x, o) in out.iter_mut().enumerate() {
            *o = mean(&acc, n);
            if let Some(p) = row.get(x + r + 1) {
                add(&mut acc, p);
            }
            if let Some(p) = x.checked_sub(r).and_then(|i| row.get(i)) {
                sub(&mut acc, p);
            }
        }
    }
}

/// One box blur of radius `r` down the columns of `w`-wide rows (transparent past the ends), row
/// by row with a running sum per column.
fn box_cols(src: &[Px16], dst: &mut [Px16], w: usize, r: usize) {
    let n = 2 * r as u64 + 1;
    let rows: Vec<&[Px16]> = src.chunks_exact(w).collect();
    let mut acc = vec![[0u64; 4]; w];
    let add_row = |acc: &mut [[u64; 4]], row: &[Px16]| acc.iter_mut().zip(row).for_each(|(a, p)| add(a, p));
    rows.iter().take(r + 1).for_each(|row| add_row(&mut acc, row));
    for (y, out) in dst.chunks_exact_mut(w).enumerate() {
        for (o, a) in out.iter_mut().zip(&acc) {
            *o = mean(a, n);
        }
        if let Some(row) = rows.get(y + r + 1) {
            add_row(&mut acc, row);
        }
        if let Some(row) = y.checked_sub(r).and_then(|i| rows.get(i)) {
            acc.iter_mut().zip(*row).for_each(|(a, p)| sub(a, p));
        }
    }
}

/// Box-blur `w`-wide rows of pixels in place with radius `r` along both axes.
pub(super) fn box_blur(px: &mut [Px16], w: usize, r: usize) {
    if r == 0 || w == 0 {
        return;
    }
    let mut tmp = vec![[0u16; 4]; px.len()];
    box_rows(px, &mut tmp, w, r);
    box_cols(&tmp, px, w, r);
}

/// Gaussian-blur `w`-wide rows of premultiplied pixels in place (σ = `sigma` pixels, three box
/// passes per axis, transparent past the edges).
pub(super) fn gaussian(px: &mut [Px16], w: usize, sigma: f64) {
    if sigma < 0.2 || w == 0 {
        return;
    }
    let mut tmp = vec![[0u16; 4]; px.len()];
    for r in gauss_boxes(sigma) {
        box_rows(px, &mut tmp, w, r);
        px.copy_from_slice(&tmp);
    }
    for r in gauss_boxes(sigma) {
        box_cols(px, &mut tmp, w, r);
        px.copy_from_slice(&tmp);
    }
}

/// Gaussian-blur `data`, `w` × `h` premultiplied RGBA8 pixels, in place (σ = `sigma` pixels). Does
/// nothing when the sizes don't match.
pub fn gaussian_rgba(data: &mut [u8], w: usize, h: usize, sigma: f64) {
    if let Some(px) = pixels(data, w, h)
        && sigma >= 0.2
    {
        let mut wide = to16(px);
        gaussian(&mut wide, w, sigma);
        for (p, q) in px.iter_mut().zip(&wide) {
            *p = q.map(to8);
        }
    }
}

#[cfg(test)]
mod tests;
