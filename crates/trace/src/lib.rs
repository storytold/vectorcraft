//! VectorCraft Image Trace: raster → vector.
//!
//! A clean-room tracer built from the published ideas behind bitmap tracers (the potrace paper
//! and classic colour quantisation), not from any existing implementation:
//!
//! 1. **Quantise** the image into a small palette: a luminance threshold (Black and White), 1-D
//!    k-means over the luminance histogram (Grayscale), or, in Color, over a 15-bit colour
//!    histogram as the [`Palette`] asks: median cut to a colour count followed by weighted k-means
//!    (Limited), median cut until every colour box is tight (Full Tone), one of those two by
//!    whether the image is flat art (Automatic), or the most used colours of a swatch library
//!    (Document Library). Transparent pixels are left out. The palette is made of the picture's own
//!    colours: the blends that anti-aliasing and blur leave along edges take none of it, colours that
//!    noise made of one colour are merged, and a blend pixel goes to one of the two colours it lies
//!    between (see `quantize`).
//!    A continuous-tone image (a photo) is first smoothed within its tones, edges kept (see
//!    `smooth`): otherwise its noise decides, pixel by pixel, which of two neighbouring tones a
//!    pixel gets, and every such pixel is a speck.
//! 2. **Remove noise**: whole 4-connected same-colour regions are merged, smallest first, into
//!    their most common neighbouring colour until no mergeable region is smaller than `noise`.
//!    Raising Noise continues the same merge sequence without splitting earlier regions.
//! 3. **Follow boundaries**: for each colour layer, the pixel-crack edges between inside and
//!    outside are linked into closed loops (inside on the right, right turns at saddles, so
//!    diagonal pixels stay separate, matching 4-connectivity). Outer loops run clockwise and
//!    holes counter-clockwise (y down), so the result fills correctly with either fill rule.
//! 4. **Sub-pixel edges**: each crack of a loop is moved to where the image says the edge really
//!    is (an anti-aliased or blurred edge pixel is part one tone, part the other), so the outline
//!    is right to a fraction of a pixel before any fitting; hard-edged images are read as
//!    staircases instead (see `subpixel`).
//!    Outlines are cut at their junctions (where three regions meet, and at the image's border)
//!    and each piece between two is fitted on its own, the same way from either side, so that two
//!    abutting regions have one and the same curve along the edge they share.
//! 5. **Curves**: the outline is fitted as Flat Logo mode fits its own (`logo::fit_outline`):
//!    corners the pixels rounded are put back, straight runs become lines, a round outline a
//!    true circle, and the rest as few cubic Béziers as the Paths tolerance allows; Snap Curves To
//!    Lines also straightens slightly curved runs and makes arcs of parts of circles.
//!
//! With **Create: Strokes**, the parts of a colour layer no wider than the stroke width are traced
//! as stroked centre lines instead (see `centerline`): placed in the middle of their ink to a
//! fraction of a pixel, as wide as their ink, and fitted as outlines are. Without Fills, wider
//! parts become stroked outlines.
//!
//! Colour layers are traced either *abutting* (each colour's own area; shapes share edges) or
//! *overlapping* (stacked: each layer also covers every layer above it, so no hairline gaps).
#![forbid(unsafe_code)]

mod centerline;
mod contour;
mod fit;
mod gradient;
mod group;
mod logo;
mod mosaic;
mod quantize;
mod shapes;
mod smooth;
mod subpixel;

use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};
use vectorcraft_geom::PathData;

pub use contour::{Component, Loop, trace_mask};
pub use gradient::TracedGradient;
pub use mosaic::mosaic;
pub use quantize::{FULL_TONE_MAX, Quantized, TRANSPARENT, denoise, quantize};
pub use shapes::TracedShape;
use subpixel::Crack;

/// Errors decoding a raster.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("could not decode image: {0}")]
    Decode(String),
    #[error("image is empty")]
    Empty,
    #[error("the image is {width} × {height} pixels, more than Image Trace takes ({max} megapixels)")]
    TooLarge { width: u32, height: u32, max: u64 },
    #[error("this isn't a flat-colour logo ({:.0}% of its solid areas match a few flat colours), so Flat Logo mode would turn its gradients into patches: use Color mode instead", (.fraction * 100.0).floor())]
    NotFlat { fraction: f64 },
    #[error("\"{0}\" is not a colour: write it as #rrggbb")]
    BadPalette(String),
    #[error(
        "this trace would make more than {max} anchor points, more than the document can show smoothly: raise Noise, lower Paths or Colors, or pick a lower-fidelity preset"
    )]
    TooComplex { max: usize },
}

/// The most pixels an image to trace may have (64 megapixels: 256 MB decoded); larger images are
/// refused instead of running out of memory.
pub const MAX_PIXELS: u64 = 64 << 20;

/// The most pixels an image may have for [`Raster::decode_to_fit`] to decode it and scale it down
/// to [`MAX_PIXELS`] (192 megapixels: 768 MB decoded); larger images are refused.
pub const MAX_SOURCE_PIXELS: u64 = 192 << 20;

/// The largest size with `w : h`'s aspect of at most `max` pixels (each side at least 1).
fn fit_size(w: u32, h: u32, max: u64) -> (u32, u32) {
    let pixels = u64::from(w) * u64::from(h);
    if pixels <= max || pixels == 0 {
        return (w, h);
    }
    let k = (max as f64 / pixels as f64).sqrt();
    let side = |v: u32| ((f64::from(v) * k).floor() as u32).clamp(1, v);
    let (mut nw, mut nh) = (side(w), side(h));
    // A side held at 1 pixel leaves the other too long: cut it to what fits.
    if u64::from(nw) * u64::from(nh) > max {
        if nw >= nh {
            nw = u32::try_from(max / u64::from(nh)).unwrap_or(nw).clamp(1, nw);
        } else {
            nh = u32::try_from(max / u64::from(nw)).unwrap_or(nh).clamp(1, nh);
        }
    }
    (nw, nh)
}

/// An RGBA8 raster (row-major, top row first).
#[derive(Clone, Debug, PartialEq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Raster {
    /// A raster from RGBA bytes (`width * height * 4` of them).
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        assert_eq!(rgba.len(), width as usize * height as usize * 4, "rgba length must be width*height*4");
        Self { width, height, rgba }
    }
    /// A raster whose pixel `(x, y)` is `f(x, y)`.
    pub fn from_fn(width: u32, height: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Self {
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&f(x, y));
            }
        }
        Self { width, height, rgba }
    }
    /// Decode PNG / JPEG / WebP / GIF bytes (at most [`MAX_PIXELS`], checked before decoding).
    pub fn decode(bytes: &[u8]) -> Result<Self, TraceError> {
        let decode_err = |e: image::ImageError| TraceError::Decode(e.to_string());
        let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| TraceError::Decode(e.to_string()))?;
        let (w, h) = reader.into_dimensions().map_err(decode_err)?;
        if w == 0 || h == 0 {
            return Err(TraceError::Empty);
        }
        if u64::from(w) * u64::from(h) > MAX_PIXELS {
            return Err(TraceError::TooLarge { width: w, height: h, max: MAX_PIXELS >> 20 });
        }
        let img = image::load_from_memory(bytes).map_err(decode_err)?.to_rgba8();
        let (w, h) = img.dimensions();
        Ok(Self { width: w, height: h, rgba: img.into_raw() })
    }
    /// Decode as [`Raster::decode`], but an image of more than [`MAX_PIXELS`] (up to
    /// [`MAX_SOURCE_PIXELS`]) is scaled down, aspect kept, to the largest size within
    /// [`MAX_PIXELS`] instead of being refused: a 300 ppi scan of a poster traces at slightly
    /// less than its own resolution. The caller maps the result back with `width` / `height`.
    pub fn decode_to_fit(bytes: &[u8]) -> Result<Self, TraceError> {
        let decode_err = |e: image::ImageError| TraceError::Decode(e.to_string());
        let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| TraceError::Decode(e.to_string()))?;
        let (w, h) = reader.into_dimensions().map_err(decode_err)?;
        if w == 0 || h == 0 {
            return Err(TraceError::Empty);
        }
        let pixels = u64::from(w) * u64::from(h);
        if pixels <= MAX_PIXELS {
            return Self::decode(bytes);
        }
        if pixels > MAX_SOURCE_PIXELS {
            return Err(TraceError::TooLarge { width: w, height: h, max: MAX_SOURCE_PIXELS >> 20 });
        }
        // Decoded within the allocation the source limit needs; the web keeps file import's 512 MB.
        // A deeper image (16 bits a channel) that needs more fails with a decode error.
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| TraceError::Decode(e.to_string()))?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(if cfg!(target_arch = "wasm32") { 512 << 20 } else { MAX_SOURCE_PIXELS * 4 });
        reader.limits(limits);
        let img = reader.decode().map_err(decode_err)?.into_rgba8();
        let (w, h) = img.dimensions();
        let (nw, nh) = fit_size(w, h, MAX_PIXELS);
        // Box-filtered: every source pixel counts, so thin lines and anti-aliased edges survive.
        let small = image::imageops::thumbnail(&img, nw, nh);
        Ok(Self { width: nw, height: nh, rgba: small.into_raw() })
    }
    /// Encode as PNG.
    pub fn encode_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(img) = image::RgbaImage::from_raw(self.width, self.height, self.rgba.clone()) {
            let _ = img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png);
        }
        out
    }
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }
}

/// Tracing mode (Illustrator's Mode popup).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    #[default]
    #[serde(alias = "bw", alias = "blackWhite", alias = "Black and White")]
    BlackAndWhite,
    #[serde(alias = "gray", alias = "Grayscale")]
    Grayscale,
    #[serde(alias = "Color")]
    Color,
    /// Flat-colour logos: sub-pixel edges, sharp corners restored, true lines and circles.
    #[serde(alias = "Logo", alias = "flat", alias = "flatLogo")]
    Logo,
}

/// How colour layers relate (Advanced → Method).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Method {
    /// Each colour traced on its own; neighbouring shapes share their edge.
    #[default]
    Abutting,
    /// Stacked: every layer extends under the layers above it.
    Overlapping,
}

/// Where Color mode's colours come from (the Palette popup).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Palette {
    /// The `colors` colours that best represent the image.
    #[default]
    #[serde(alias = "Limited")]
    Limited,
    /// As many colours as the image's tones need, up to [`FULL_TONE_MAX`] (photos, gradients):
    /// `color_detail` sets how close two colours may be and stay apart.
    #[serde(alias = "Full Tone")]
    FullTone,
    /// Flat art (a few colours cover nearly all of it) traces with exactly those colours, anything
    /// else as Full Tone.
    #[serde(alias = "Automatic")]
    Automatic,
    /// The most used colours of a swatch library ([`TraceParams::swatches`]), at most `colors` of
    /// them, exactly as they are.
    #[serde(alias = "library", alias = "Document Library")]
    DocumentLibrary,
}

/// Image Trace parameters (the Image Trace panel).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TraceParams {
    pub mode: Mode,
    /// Black and White: pixels darker than this (luminance 0–255) are black.
    pub threshold: u8,
    /// Color: where the colours come from.
    pub palette: Palette,
    /// Palette size for Color (Limited; at most this many for Document Library), number of grays
    /// for Grayscale (2–256).
    pub colors: u32,
    /// Full Tone and Automatic: how many colours, 0 (few) – 100 (many).
    pub color_detail: f64,
    /// Document Library: the swatch library, `"document"` (the document's swatches) or a library id
    /// or name. The caller looks its colours up into `swatches`.
    pub library: String,
    /// Document Library: the library's colours (sRGB). Not saved: they are looked up from
    /// `library` at every trace.
    #[serde(skip)]
    pub swatches: Vec<[u8; 3]>,
    /// Paths (fidelity) 0–100: higher follows the pixels more tightly.
    pub paths: f64,
    /// Corners 0–100: higher keeps more corners.
    pub corners: f64,
    /// Noise: areas smaller than this many pixels are ignored. (Flat Logo mode: areas smaller than
    /// `noise / 10` pixels of the source image.)
    pub noise: u32,
    pub method: Method,
    /// Ignore Color: drop the areas of one colour (no background shape): white, unless
    /// `ignore_color` names another.
    pub ignore_white: bool,
    /// The colour Ignore Color leaves out (`#rrggbb`): the palette colours within about 30 levels
    /// of it. `None` is white. (Flat Logo mode leaves out white only.)
    pub ignore_color: Option<String>,
    /// Replace nearly-straight curves with straight lines. In Logo mode this also snaps tight arcs
    /// to true circles.
    pub snap_curves_to_lines: bool,
    /// Shapes: a filled outline that is a circle, an ellipse, a rectangle (square or rounded
    /// corners) or a regular polygon comes out as exactly that shape ([`TracedPath::shape`]).
    pub shapes: bool,
    /// Auto Grouping: say which path lies inside which ([`TraceResult::parents`]).
    pub auto_grouping: bool,
    /// Gradients (Grayscale and Color): an area whose colour changes evenly along one direction
    /// is one path filled with a linear gradient ([`TracedPath::gradient`]), not bands of colour.
    /// Grayscale finds them in the image's grays, and their stops are gray.
    pub gradients: bool,
    /// Create: trace areas as filled shapes.
    pub fills: bool,
    /// Create: trace lines no wider than `stroke_width` as stroked centre lines.
    pub strokes: bool,
    /// Stroke: the widest feature (px) traced as a stroke.
    pub stroke_width: f64,
    /// Flat Logo mode: the flat colours (`#rrggbb`) to use; empty finds them automatically. White is
    /// always traced on its own and cannot be named, colours closer than about 12 (RGB distance)
    /// count as one, and at most ten are used. (Not `palette`: that is Color mode's colour source.)
    pub logo_colors: Vec<String>,
}

impl Default for TraceParams {
    /// Illustrator's [Default]: black and white, threshold 128.
    fn default() -> Self {
        Self {
            mode: Mode::BlackAndWhite,
            threshold: 128,
            palette: Palette::Limited,
            colors: 6,
            color_detail: 50.0,
            library: "document".into(),
            swatches: vec![],
            paths: 50.0,
            corners: 75.0,
            noise: 25,
            method: Method::Abutting,
            ignore_white: false,
            ignore_color: None,
            snap_curves_to_lines: false,
            shapes: false,
            auto_grouping: false,
            gradients: false,
            fills: true,
            strokes: false,
            stroke_width: 10.0,
            logo_colors: Vec::new(),
        }
    }
}

impl TraceParams {
    /// How outlines are fitted: Paths sets how closely the curves follow the edge (0.08 px at 100,
    /// 0.33 px at 50, 0.78 px at 0), Corners the turn that makes a corner (35° at 100, 90° at 0).
    /// Straight edges are always lines; Snap Curves To Lines also straightens slightly curved ones,
    /// makes parts of circles true arcs, and makes lines within 2.5° of level or upright exactly so.
    fn outline_style(&self) -> logo::Style {
        // Parameters come from users and agents: a NaN or infinity must not reach the geometry.
        let unit = |v: f64, default: f64| if v.is_finite() { (v / 100.0).clamp(0.0, 1.0) } else { default };
        let snap = self.snap_curves_to_lines;
        logo::Style {
            step: 0.25,
            tol: 0.08 + 0.7 * (1.0 - unit(self.paths, 0.5)).powf(1.5),
            corner_deg: 90.0 - 55.0 * unit(self.corners, 0.75),
            snap: true,
            arcs: snap,
            line_tol: if snap { 0.5 } else { 0.2 },
            circle_tol: if snap { 0.3 } else { 0.15 },
            restore: Some(logo::RestoreParams::default()),
            check_step: 0.15,
            upright: snap,
            ..logo::Style::default()
        }
    }
}

/// Built-in preset names in panel order (our own parameter sets).
pub const PRESET_NAMES: &[&str] = &[
    "Default",
    "Auto Color",
    "High Fidelity Photo",
    "Low Fidelity Photo",
    "3 Colors",
    "6 Colors",
    "16 Colors",
    "Shades of Gray",
    "Black and White Logo",
    "Sketched Art",
    "Silhouettes",
    "Line Art",
    "Technical Drawing",
    "Outline",
    "Flat Logo",
];

/// A built-in preset by name (case-insensitive; `[Default]` is accepted).
pub fn preset(name: &str) -> Option<TraceParams> {
    let key = name.trim().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    let d = TraceParams::default();
    let color = |colors: u32, paths: f64, corners: f64, noise: u32, method: Method| TraceParams {
        mode: Mode::Color,
        colors,
        paths,
        corners,
        noise,
        method,
        ..TraceParams::default()
    };
    let bw = |threshold: u8, paths: f64, corners: f64, noise: u32, snap: bool| TraceParams {
        mode: Mode::BlackAndWhite,
        threshold,
        paths,
        corners,
        noise,
        ignore_white: true,
        snap_curves_to_lines: snap,
        ..TraceParams::default()
    };
    Some(match key.as_str() {
        "default" => d,
        // A posterised image of a photo or of artwork: its own colours where it is flat art, as
        // many as its tones need where it is not.
        // (Color detail 85: a colour on a hundredth of the picture is still one of its colours.)
        "auto color" => TraceParams { palette: Palette::Automatic, color_detail: 85.0, ..color(6, 65.0, 60.0, 16, Method::Abutting) },
        "high fidelity photo" => color(64, 90.0, 25.0, 4, Method::Overlapping),
        "low fidelity photo" => color(20, 60.0, 50.0, 12, Method::Overlapping),
        "3 colors" => color(3, 60.0, 60.0, 20, Method::Abutting),
        "6 colors" => color(6, 65.0, 60.0, 16, Method::Abutting),
        "16 colors" => color(16, 70.0, 55.0, 10, Method::Abutting),
        "shades of gray" => TraceParams { mode: Mode::Grayscale, colors: 8, paths: 60.0, corners: 50.0, noise: 10, method: Method::Overlapping, ..d },
        "black and white logo" => bw(128, 95.0, 80.0, 8, true),
        "sketched art" => bw(150, 50.0, 50.0, 100, false),
        "silhouettes" => bw(200, 40.0, 60.0, 30, false),
        "line art" => bw(128, 80.0, 70.0, 10, false),
        "technical drawing" => bw(128, 95.0, 90.0, 4, true),
        // Black outlines: lines as stroked centre lines, wider areas as stroked outlines.
        "outline" => TraceParams { fills: false, strokes: true, ..bw(128, 80.0, 70.0, 10, false) },
        // Flat-colour logos: colours found automatically, edges placed to a fraction of a pixel.
        "flat logo" => TraceParams { mode: Mode::Logo, colors: 8, paths: 60.0, corners: 50.0, noise: 6, snap_curves_to_lines: true, ..d },
        _ => return None,
    })
}

/// All built-in presets in panel order.
pub fn presets() -> Vec<(&'static str, TraceParams)> {
    PRESET_NAMES.iter().filter_map(|n| Some((*n, preset(n)?))).collect()
}

/// The enhanced version of built-in preset `name`: the preset with the options the enhanced
/// presets have built in: Auto Grouping in all of them, Shapes in all but High Fidelity Photo.
/// Gradients stay an option to turn on. Flat Logo, a preset of our own, has none.
pub fn enhanced_preset(name: &str) -> Option<TraceParams> {
    let p = preset(name).filter(|p| p.mode != Mode::Logo)?;
    Some(TraceParams { auto_grouping: true, shapes: !name.eq_ignore_ascii_case("High Fidelity Photo"), ..p })
}

/// One traced shape: an outer contour plus its holes, filled with `color`, or (with `stroke`)
/// lines stroked with it.
#[derive(Clone, Debug, PartialEq)]
pub struct TracedPath {
    /// In pixel coordinates (x right, y down; the image spans `0..width × 0..height`).
    pub path: PathData,
    pub color: [u8; 3],
    /// Filled pixel count of the component the path came from.
    pub pixels: usize,
    /// Create Strokes: the path is stroked this wide (px), not filled (centre lines, or the
    /// outlines of wider areas without Fills).
    pub stroke: Option<f64>,
    /// Shapes: the shape the path is, exactly (`path` is that shape's outline).
    pub shape: Option<TracedShape>,
    /// Gradients: the linear gradient the path is filled with (`color` is its middle colour).
    pub gradient: Option<TracedGradient>,
    /// Create Strokes: the stroke's ends are square (a cap that goes half the width past the
    /// path's end, cut straight), as the line's ink is; otherwise round.
    pub square_caps: bool,
}

/// The traced result, bottom-most path first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraceResult {
    pub paths: Vec<TracedPath>,
    /// Colours actually used.
    pub palette: Vec<[u8; 3]>,
    /// Auto Grouping: for each of `paths`, the path it lies inside, if any (empty without the
    /// option). Each path drawn together with those inside it, in the order they have here, paints
    /// what `paths` paints in its order.
    pub parents: Vec<Option<usize>>,
}

impl TraceResult {
    pub fn anchor_count(&self) -> usize {
        self.paths.iter().map(|p| p.path.anchor_count()).sum()
    }
}

fn is_white(c: [u8; 3]) -> bool {
    c.iter().all(|&v| v >= 245)
}

/// Whether Ignore Color leaves palette colour `c` out: `target` is the colour picked (`None`: white).
fn is_ignored(c: [u8; 3], target: Option<[u8; 3]>) -> bool {
    match target {
        None => is_white(c),
        Some(t) => (0..3).map(|k| (f64::from(c[k]) - f64::from(t[k])).powi(2)).sum::<f64>() <= 30.0 * 30.0 || (is_white(t) && is_white(c)),
    }
}

/// Trace `img` with `params`. Failures (an image Logo mode refuses, say) give an empty result: use
/// [`trace_within`] to see why.
pub fn trace(img: &Raster, params: &TraceParams) -> TraceResult {
    // No trace has more than `usize::MAX` anchors.
    trace_within(img, params, usize::MAX).unwrap_or_default()
}

/// Trace `img` with `params`, giving up with [`TraceError::TooComplex`] once the paths have more
/// than `max_anchors` anchor points (a photo traced at high fidelity can make millions). The
/// colour layers are traced in parallel where threads are available.
pub fn trace_within(img: &Raster, params: &TraceParams, max_anchors: usize) -> Result<TraceResult, TraceError> {
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 || img.rgba.len() != w * h * 4 {
        return Ok(TraceResult::default());
    }
    let mut out = trace_paths(img, params, max_anchors)?;
    if params.auto_grouping {
        // Abutting areas cannot overlap; stacked layers, Logo's shapes and stroked lines can.
        let stacked = params.method == Method::Overlapping || params.mode == Mode::Logo || params.strokes;
        out.parents = group::parents(&out.paths, img.width as usize, img.height as usize, stacked);
    }
    Ok(out)
}

fn trace_paths(img: &Raster, params: &TraceParams, max_anchors: usize) -> Result<TraceResult, TraceError> {
    let (w, h) = (img.width as usize, img.height as usize);
    if params.mode == Mode::Logo {
        return logo::trace_logo(img, params, max_anchors);
    }
    // The colour Ignore Color leaves out, if not white.
    let ignore = match &params.ignore_color {
        Some(name) if params.ignore_white => Some(logo::parse_hex(name).ok_or_else(|| TraceError::BadPalette(name.clone()))?),
        _ => None,
    };
    // A photo is traced from a copy with its noise smoothed away (see `smooth`).
    let smoothed = (params.mode != Mode::BlackAndWhite && quantize::continuous_tone(img)).then(|| smooth::smoothed(img));
    let img = smoothed.as_ref().unwrap_or(img);
    // Gradients are found first and kept out of the palette: no colour is spent on bands of them.
    // Grayscale finds them in the image's grays (as its flat colours are), so their stops are gray.
    let grays = (params.gradients && params.mode == Mode::Grayscale).then(|| {
        let mut grays = img.clone();
        for p in grays.rgba.as_chunks_mut::<4>().0 {
            let g = quantize::luma(*p);
            p[..3].fill(g);
        }
        grays
    });
    let traced_as = grays.as_ref().unwrap_or(img);
    let found = (params.gradients && params.mode != Mode::BlackAndWhite).then(|| gradient::detect(traced_as)).filter(|f| !f.1.is_empty());
    let without = found.as_ref().map(|(of, _)| {
        let mut rest = img.clone();
        for (p, &g) in rest.rgba.as_chunks_mut::<4>().0.iter_mut().zip(of) {
            if g != u32::MAX {
                p[3] = 0;
            }
        }
        rest
    });
    let mut q = quantize(without.as_ref().unwrap_or(img), params);
    drop(without);
    // Each gradient is a palette entry of its own (its colour: the gradient's middle).
    let mut gradients: Vec<Option<TracedGradient>> = vec![None; q.palette.len()];
    if let Some((mut of, found)) = found {
        gradient::settle_edges(traced_as, &mut of, &found, &mut q.labels);
        let first = q.palette.len();
        for (l, &g) in q.labels.iter_mut().zip(&of) {
            if g != u32::MAX {
                *l = u16::try_from(first + g as usize).unwrap_or(TRANSPARENT);
            }
        }
        q.palette.extend(found.iter().map(|g| g.color_at(g.start.midpoint(g.end)).map(|v| v.round().clamp(0.0, 255.0) as u8)));
        gradients.extend(found.into_iter().map(Some));
    }
    drop(grays);
    denoise(&mut q.labels, w, h, params.noise as usize);
    // Pixel count per palette entry, and the box its pixels lie in.
    let mut counts = vec![0usize; q.palette.len()];
    let mut boxes: Vec<Option<(usize, usize, usize, usize)>> = vec![None; q.palette.len()];
    for (i, &l) in q.labels.iter().enumerate() {
        if let (Some(c), Some(b)) = (counts.get_mut(l as usize), boxes.get_mut(l as usize)) {
            *c += 1;
            let (x, y) = (i % w, i / w);
            *b = Some(b.map_or((x, y, x + 1, y + 1), |b| (b.0.min(x), b.1.min(y), b.2.max(x + 1), b.3.max(y + 1))));
        }
    }
    // Layers bottom → top: largest area first.
    let mut order: Vec<usize> = (0..q.palette.len()).filter(|&i| counts[i] > 0).collect();
    order.sort_by(|a, b| counts[*b].cmp(&counts[*a]).then(a.cmp(b)));
    if params.method == Method::Overlapping {
        // Stacked layers go by lightness instead, from the largest colour's end of the scale: each
        // layer reaches under all those above it, so where two colours meet, every layer between
        // them in the stack ends on that same edge and shows through its anti-aliasing. Neighbours
        // in a picture are mostly neighbours in tone, and in this order they are neighbours in the
        // stack too (by area, an unrelated colour's edge would shine through as a hairline).
        let light = |i: usize| q.palette.get(i).map_or(0, |c| u32::from(c[0]) * 299 + u32::from(c[1]) * 587 + u32::from(c[2]) * 114);
        let first = order.first().copied();
        order.sort_by(|a, b| light(*a).cmp(&light(*b)).then(a.cmp(b)));
        if first.is_some_and(|f| order.iter().position(|&i| i == f).is_some_and(|at| at * 2 >= order.len())) {
            order.reverse();
        }
    }
    let mut rank = vec![usize::MAX; q.palette.len()];
    for (r, &i) in order.iter().enumerate() {
        rank[i] = r;
    }
    let layers: Vec<(usize, usize)> =
        order.iter().copied().enumerate().filter(|&(_, ci)| !(params.ignore_white && is_ignored(q.palette[ci], ignore))).collect();
    let job = Layers {
        q: &q,
        rank: &rank,
        params,
        img,
        ignore,
        boxes,
        gradients: &gradients,
        bw_tones: bw_tones(img, &q),
        style: params.outline_style(),
        w,
        h,
        anchors: AtomicUsize::new(0),
        max_anchors,
    };
    let mut out = TraceResult::default();
    // A layer takes about ten bytes a pixel of its box while it is traced (its mask, the labels
    // of its components, the stack that fills them, its edges): as many layers at a time as fit
    // in half a gigabyte, so that a large image is not traced at the cost of gigabytes.
    let largest = job.boxes.iter().flatten().map(|b| (b.2 - b.0) * (b.3 - b.1)).max().unwrap_or(1).max(1);
    let at_a_time = ((512 << 20) / (largest * 10)).max(1);
    for (paths, &(_, ci)) in parallel_map_at_most(at_a_time, &layers, |&(r, ci)| job.layer(r, ci)).into_iter().zip(&layers) {
        out.paths.extend(paths.ok_or(TraceError::TooComplex { max: max_anchors })?);
        let color = q.palette[ci];
        if !out.palette.contains(&color) {
            out.palette.push(color);
        }
    }
    Ok(out)
}

/// The luminance of white and of black in a Black and White quantisation: what its lightest
/// white pixels and its darkest black ones have (the 98th and 2nd percentiles; pure white and
/// black where a side has no pixel). Not the means: where the black is thin lines, blurred or
/// scanned, most black pixels are part paper, their mean is a gray, and read against it the middle
/// of every line would count as more than all ink, now more and now less as the line crosses the
/// pixel rows, which put a ripple on its edges.
fn bw_tones(img: &Raster, q: &Quantized) -> [f64; 2] {
    let mut hist = [[0u64; 256]; 2];
    for (p, &l) in img.rgba.as_chunks::<4>().0.iter().zip(&q.labels) {
        if let Some(h) = hist.get_mut(l as usize) {
            h[((u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114 + 500) / 1000).min(255) as usize] += 1;
        }
    }
    // The luminance a share `at` of the side's pixels are darker than.
    let percentile = |h: &[u64; 256], at: f64, none: f64| {
        let total: u64 = h.iter().sum();
        let mut seen = 0;
        for (v, n) in h.iter().enumerate() {
            seen += n;
            if total > 0 && seen as f64 >= at * total as f64 {
                return v as f64;
            }
        }
        none
    };
    [percentile(&hist[0], 0.98, 255.0), percentile(&hist[1], 0.02, 0.0)]
}

/// What tracing one colour layer needs.
struct Layers<'a> {
    q: &'a Quantized,
    /// Each palette entry's layer, bottom (0) up.
    rank: &'a [usize],
    params: &'a TraceParams,
    img: &'a Raster,
    /// The colour Ignore Color leaves out, if not white.
    ignore: Option<[u8; 3]>,
    /// The box each palette entry's pixels lie in: `(x0, y0, x1, y1)`, the far sides excluded.
    boxes: Vec<Option<(usize, usize, usize, usize)>>,
    /// Gradients: the gradient each palette entry is, if it is one.
    gradients: &'a [Option<TracedGradient>],
    /// Black and White: the luminance of white and of black (see `bw_tones`).
    bw_tones: [f64; 2],
    /// How outlines and centre lines are fitted.
    style: logo::Style,
    w: usize,
    h: usize,
    /// Anchor points made so far, by every layer.
    anchors: AtomicUsize,
    max_anchors: usize,
}

impl Layers<'_> {
    /// Ignore White: white is no part of any layer (stacked layers reach under the layers above
    /// them, and must not reach under white that is left out).
    fn ignored(&self, label: u16) -> bool {
        self.params.ignore_white && self.q.palette.get(label as usize).is_some_and(|c| is_ignored(*c, self.ignore))
    }
    /// The region across a crack, for cutting outlines at their junctions: the outside pixel's
    /// colour. Stacked layers end on many colours at once and share no edge: one neighbour.
    fn neighbour(&self, c: Crack) -> u32 {
        if self.params.method == Method::Overlapping {
            return 0;
        }
        c.at(1).and_then(|i| self.q.labels.get(i)).map_or(u32::from(TRANSPARENT), |&l| u32::from(l))
    }
    /// The junction at pixel corner `(x, y)`, if there is one: where three or four regions
    /// meet, or two parts of a region touch by the corner only, or an edge reaches the image's
    /// border. It lies at the corner, moved across by what the edges that run up and down from it
    /// say, and up or down by what those to its left and right say (the mean, where there are
    /// two). It depends on the corner alone, so every region around it gets the same point; on the
    /// image's border it stays on the border.
    fn junction(&self, x: i32, y: i32) -> Option<vectorcraft_geom::Point> {
        let label = |px: i32, py: i32| Crack::new((px, py), (1, 0), self.w, self.h).at(0).and_then(|i| self.q.labels.get(i).copied());
        // The four pixels around the corner (`None` off the image), and the edges between them.
        let around = [label(x - 1, y - 1), label(x, y - 1), label(x, y), label(x - 1, y)];
        let edges = (0..4).filter(|&i| around[i] != around[(i + 1) % 4]).count();
        let apart = |a: usize, b: usize, c: usize, d: usize| around[a] == around[c] && around[b] != around[a] && around[d] != around[a];
        let touching = apart(0, 1, 2, 3) || apart(1, 2, 3, 0);
        // (Stacked layers share no edges: they are cut at the image's border only.)
        if edges < 3 && !touching || self.params.method == Method::Overlapping {
            return None;
        }
        // The shift of the crack between `a` and the pixel `step` from it, if both are in the
        // image and of different regions.
        let shift = |a: (i32, i32), step: (i32, i32)| {
            let (la, lb) = (label(a.0, a.1)?, label(a.0 + step.0, a.1 + step.1)?);
            (la != lb).then(|| self.shift(Crack::new(a, step, self.w, self.h)).clamp(-0.5, 0.5))
        };
        let mean = |a: Option<f64>, b: Option<f64>| match (a, b) {
            (Some(a), Some(b)) => (a + b) / 2.0,
            (Some(v), None) | (None, Some(v)) => v,
            (None, None) => 0.0,
        };
        let dx = mean(shift((x - 1, y - 1), (1, 0)), shift((x - 1, y), (1, 0)));
        let dy = mean(shift((x - 1, y - 1), (0, 1)), shift((x, y - 1), (0, 1)));
        Some(vectorcraft_geom::Point::new(f64::from(x) + dx, f64::from(y) + dy))
    }
    /// How far the edge lies from a crack between two pixels, towards the outside one (−0.5 to 0.5).
    ///
    /// Each pixel of the column across the crack is read as a blend of the tones on its two sides
    /// (the two pixels' palette colours; in Black and White the luminance of the image's black and of
    /// the white pixels). The edge is where the blends, summed, put it: as far out as there is
    /// inside tone in the outside pixels, less the outside tone in the inside ones (two pixels each
    /// way, while they are on the same side and no more blended than the pixel before: more would be
    /// another edge's doing).
    ///
    /// Where the inside is at most three pixels deep with the outside's colour behind it (a thin
    /// line), its two edges share those pixels and cannot be read apart; the inside tone of the
    /// whole column gives the line's middle (its centroid) and width (its sum) instead. A thin gap
    /// between two parts of the inside is read the same way, with the tones swapped.
    ///
    /// Black and White then moves the edge to where the luminance crosses the threshold: by the
    /// threshold's distance from the middle tone, over how steep the edge is (nothing for a
    /// threshold in the middle, more the softer the edge).
    fn shift(&self, c: Crack) -> f64 {
        let label = |k: i32| c.at(k).and_then(|i| self.q.labels.get(i).copied()).filter(|&l| l != TRANSPARENT);
        let (Some(la), Some(lb)) = (label(0), label(1)) else { return 0.0 };
        if la == lb {
            return 0.0;
        }
        let bw = self.params.mode == Mode::BlackAndWhite;
        // Black and White: black is the image's black; white is the lightest pixel within a few
        // pixels of the crack, the paper as it is here (a scan's paper is darker in one corner than
        // in another, and read against the image's lightest white it would count as part ink).
        let black = self.bw_tones[1];
        let white = if bw {
            let light = (-3..=4)
                .filter_map(|k| c.at(k).and_then(|i| self.img.rgba.get(i * 4..i * 4 + 3)))
                .map(|p| (f64::from(p[0]) * 299.0 + f64::from(p[1]) * 587.0 + f64::from(p[2]) * 114.0) / 1000.0);
            light.fold(f64::NEG_INFINITY, f64::max).clamp((black + 32.0).min(self.bw_tones[0]), self.bw_tones[0])
        } else {
            self.bw_tones[0]
        };
        let (ca, cb) = if bw {
            let tone = |l: u16| [if l == 1 { black } else { white }; 3];
            (tone(la), tone(lb))
        } else {
            // A side's own colour: its palette entry's, or its gradient's colour here.
            let own = |l: u16, k: i32| {
                let flat = self.q.palette.get(l as usize)?.map(f64::from);
                let Some(g) = self.gradients.get(l as usize).and_then(Option::as_ref) else { return Some(flat) };
                let i = c.at(k)?;
                Some(g.color_at(vectorcraft_geom::Point::new((i % self.w) as f64 + 0.5, (i / self.w) as f64 + 0.5)))
            };
            let (Some(ca), Some(cb)) = (own(la, 0), own(lb, 1)) else { return 0.0 };
            (ca, cb)
        };
        let d = [0, 1, 2].map(|k| cb[k] - ca[k]);
        let len2: f64 = d.iter().map(|v| v * v).sum();
        if len2 <= 0.0 {
            return 0.0;
        }
        // How much of the inside tone (0 to 1) the pixel `k` steps out has; none off the image.
        let inside = |k: i32| {
            let Some(p) = c.at(k).and_then(|i| self.img.rgba.get(i * 4..i * 4 + 3)) else { return 0.0 };
            let p = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
            let p = if bw { [(p[0] * 299.0 + p[1] * 587.0 + p[2] * 114.0) / 1000.0; 3] } else { p };
            1.0 - ((0..3).map(|k| (p[k] - ca[k]) * d[k]).sum::<f64>() / len2).clamp(0.0, 1.0)
        };
        // The edge from a thin run of `n` pixels starting at pixel `from` and going `dir`, by the
        // amounts `tone` of the run's own tone in it and in the two pixels at either end of it.
        // Returns the run's middle and width, measured from the crack towards the outside.
        let thin = |from: i32, dir: i32, n: i32, tone: &dyn Fn(i32) -> f64| {
            let (near, far) = (from - dir, from + n * dir);
            let mut cells: Vec<(i32, f64)> = (0..n).map(|j| from + j * dir).chain([near, far]).map(|k| (k, tone(k))).collect();
            for (edge, out) in [(near, -dir), (far, dir)] {
                if label(edge + out) == label(edge) {
                    cells.push((edge + out, tone(edge + out).min(tone(edge))));
                }
            }
            let width: f64 = cells.iter().map(|c| c.1).sum();
            // Pixel `k`'s centre is `k - 0.5` from the crack.
            (width > 1e-6).then(|| (cells.iter().map(|c| c.1 * (f64::from(c.0) - 0.5)).sum::<f64>() / width, width))
        };
        // How deep each side is here: up to three pixels of its own label, then the other's.
        let depth = |dir: i32, own: u16, other: u16| {
            let start = if dir < 0 { 0 } else { 1 };
            let n = (1..=3).take_while(|j| label(start + j * dir) == Some(own)).count() as i32 + 1;
            (n <= 3 && label(start + n * dir) == Some(other)).then_some(n)
        };
        let mut steep = 1.0;
        let mut shift = if let Some((middle, width)) = depth(-1, la, lb).and_then(|n| thin(0, -1, n, &inside)) {
            middle + width / 2.0
        } else if let Some((middle, width)) = depth(1, lb, la).and_then(|n| thin(1, 1, n, &|k| 1.0 - inside(k))) {
            middle - width / 2.0
        } else {
            let (a1, b1) = (1.0 - inside(0), inside(1));
            let a2 = (label(-1) == Some(la)).then(|| (1.0 - inside(-1)).min(a1));
            let b2 = (label(2) == Some(lb)).then(|| inside(2).min(b1));
            // The edge's steepness per pixel: the square of the change over two pixels (1 for an
            // anti-aliased edge, less for a blurred one).
            let change = [a2.map(|v| 1.0 - v - b1), b2.map(|v| 1.0 - a1 - v)].into_iter().flatten().fold(f64::NAN, f64::max);
            if !change.is_nan() {
                steep = (change * change).clamp(0.35, 1.0);
            }
            b1 + b2.unwrap_or(0.0) - a1 - a2.unwrap_or(0.0)
        };
        if bw {
            // The share of black at which a pixel turns black.
            let turn = ((white - f64::from(self.params.threshold)) / (white - black).max(1.0)).clamp(0.02, 0.98);
            shift += if la == 1 { 1.0 } else { -1.0 } * (0.5 - turn) / steep;
        }
        shift
    }

    /// Whether the pixels inside `l` (a loop along pixel edges) all have one label.
    fn one_colour_within(&self, l: &Loop) -> bool {
        let mut first = None;
        let (y0, y1) = l.pts.iter().fold((i32::MAX, i32::MIN), |m, p| (m.0.min(p.1), m.1.max(p.1)));
        for y in y0..y1 {
            // The loop's upright edges that pass this row of pixels, left to right: the pixels
            // between the first and the second are inside, and so on.
            let mut xs: Vec<i32> = l
                .pts
                .iter()
                .zip(l.pts.iter().cycle().skip(1))
                .filter(|(a, b)| a.0 == b.0 && a.1.min(b.1) <= y && y < a.1.max(b.1))
                .map(|(a, _)| a.0)
                .collect();
            xs.sort_unstable();
            for pair in xs.as_chunks::<2>().0 {
                for x in pair[0]..pair[1] {
                    let label = usize::try_from(y)
                        .ok()
                        .zip(usize::try_from(x).ok())
                        .and_then(|(y, x)| self.q.labels.get(y * self.w + x).copied().filter(|_| x < self.w));
                    if label.is_none() || *first.get_or_insert(label) != label {
                        return false;
                    }
                }
            }
        }
        first.is_some()
    }

    /// The paths of layer `r` (palette entry `ci`), or `None` once the anchors made pass the
    /// maximum.
    fn layer(&self, r: usize, ci: usize) -> Option<Vec<TracedPath>> {
        let params = self.params;
        let color = *self.q.palette.get(ci)?;
        let overlapping = params.method == Method::Overlapping;
        let (w, h) = (self.w, self.h);
        // Create Strokes: this colour's lines, left out of its areas.
        let (thin, lines) = if params.strokes {
            // The ink the lines are read against: the layer's colour; Black and White's own black.
            let ink = if params.mode == Mode::BlackAndWhite { [self.bw_tones[usize::from(ci == 1)]; 3] } else { color.map(f64::from) };
            centerline::lines(&self.q.labels, u16::try_from(ci).unwrap_or(TRANSPARENT), w, h, params.stroke_width, self.img, ink, &self.style)
        } else {
            (vec![], vec![])
        };
        // The layer is traced within the box its pixels lie in, not the whole image.
        let part = |l: u16| if overlapping { self.rank.get(l as usize).is_some_and(|&lr| lr >= r) && !self.ignored(l) } else { l as usize == ci };
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
        for (l, b) in self.boxes.iter().enumerate() {
            if let Some(b) = b.filter(|_| part(l as u16)) {
                (x0, y0, x1, y1) = (x0.min(b.0), y0.min(b.1), x1.max(b.2), y1.max(b.3));
            }
        }
        if x0 >= x1 || y0 >= y1 {
            return Some(vec![]);
        }
        let (bw, bh) = (x1 - x0, y1 - y0);
        let mut mask = Vec::with_capacity(bw * bh);
        for y in y0..y1 {
            let row = self.q.labels.get(y * w + x0..y * w + x1)?;
            mask.extend(row.iter().enumerate().map(|(k, &l)| l != TRANSPARENT && !thin.get(y * w + x0 + k).copied().unwrap_or(false) && part(l)));
        }
        // Without Fills, areas are outlined with a 1 px stroke.
        let area_stroke = (params.strokes && !params.fills).then_some(1.0);
        let min_hole = params.noise.max(1) as i64;
        let mut paths = vec![];
        let mut add = |path: PathData, pixels: usize, stroke: Option<f64>, shape: Option<TracedShape>, square_caps: bool| {
            let n = path.anchor_count();
            if self.anchors.fetch_add(n, Ordering::Relaxed).saturating_add(n) > self.max_anchors {
                return false;
            }
            // (A gradient fills; a stroked line of it is its middle colour.)
            let gradient = self.gradients.get(ci).cloned().flatten().filter(|_| stroke.is_none());
            paths.push(TracedPath { path, color, pixels, stroke, shape, gradient, square_caps });
            true
        };
        let mut comps = trace_mask(&mask, bw, bh);
        drop(mask);
        for comp in &mut comps {
            for l in std::iter::once(&mut comp.outer).chain(&mut comp.holes) {
                for p in &mut l.pts {
                    *p = (p.0 + x0 as i32, p.1 + y0 as i32);
                }
            }
        }
        for comp in comps {
            // (`hard`: some of the outline has no anti-aliasing, and is known less exactly.)
            let hard = std::cell::Cell::new(false);
            let fit = |l: &Loop| {
                let (sp, is_hard) = fit::fit_loop(l, w, h, &|c| (self.shift(c), self.neighbour(c)), &|x, y| self.junction(x, y), &self.style)?;
                hard.set(hard.get() || is_hard);
                Some(sp)
            };
            let Some(outer) = fit(&comp.outer) else { continue };
            let mut subs = vec![outer];
            for hole in &comp.holes {
                if hole.area2.abs() / 2 < min_hole && params.noise > 0 {
                    continue;
                }
                let Some(mut sp) = fit(hole) else { continue };
                // Shapes: a hole that is a shape is that shape, as what fills it is (the two are
                // one outline: they stay one). Only where one colour fills it: the parts of
                // several are not that shape, and would no longer meet it.
                if params.shapes
                    && area_stroke.is_none()
                    && self.one_colour_within(hole)
                    && let Some(exact) = shapes::recognise(&sp, hard.get()).and_then(|shape| shape.path().subpaths.into_iter().next())
                {
                    // (Which way round each goes: by the area of the polygon through points along it.)
                    let turn = |sp: &vectorcraft_geom::SubPath| {
                        use vectorcraft_geom::ParamCurve;
                        let pts: Vec<_> =
                            (0..sp.segment_count()).flat_map(|i| (0..6).map(move |k| sp.segment(i).eval(f64::from(k) / 6.0).to_vec2())).collect();
                        pts.iter().zip(pts.iter().cycle().skip(1)).map(|(a, b)| a.cross(*b)).sum::<f64>()
                    };
                    let mut exact = exact;
                    if (turn(&exact) < 0.0) != (turn(&sp) < 0.0) {
                        exact.reverse();
                    }
                    sp = exact;
                }
                subs.push(sp);
            }
            // Shapes: a filled outline without holes that is a shape becomes that shape.
            let shape = match subs.as_slice() {
                [only] if params.shapes && area_stroke.is_none() => shapes::recognise(only, hard.get()),
                _ => None,
            };
            let path = shape.map_or_else(|| PathData::new(subs), |s| s.path());
            if !add(path, comp.pixels, area_stroke, shape, false) {
                return None;
            }
        }
        for line in lines {
            if !add(line.path, line.pixels, Some(line.width), None, line.square) {
                return None;
            }
        }
        Some(paths)
    }
}

/// `f` of each of `items`, in order, spread over the available cores.
fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    parallel_map_at_most(usize::MAX, items, f)
}

/// `f` of each of `items`, in order, spread over the available cores, at most `at_a_time` at once.
#[cfg(not(target_arch = "wasm32"))]
fn parallel_map_at_most<T: Sync, R: Send>(at_a_time: usize, items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(items.len()).min(at_a_time);
    if threads <= 1 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let mut done: Vec<(usize, R)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut mine = vec![];
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        mine.push((i, f(item)));
                    }
                    mine
                })
            })
            .collect();
        // A worker's panic goes on in this thread, as if it had run here (the engine's guard
        // reports it).
        workers.into_iter().flat_map(|w| w.join().unwrap_or_else(|e| std::panic::resume_unwind(e))).collect()
    });
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

/// `f` of each of `items`, in order (no threads on the web).
#[cfg(target_arch = "wasm32")]
fn parallel_map_at_most<T: Sync, R: Send>(_: usize, items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    items.iter().map(f).collect()
}

#[cfg(test)]
mod tests;
