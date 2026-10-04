//! VectorCraft Image Trace: raster → vector.
//!
//! A clean-room tracer built from the published ideas behind bitmap tracers (the potrace paper
//! and classic colour quantisation), not from any existing implementation:
//!
//! 1. **Quantise** the image into a small palette: a luminance threshold (Black and White), 1-D
//!    k-means over the luminance histogram (Grayscale), or median cut followed by weighted
//!    k-means over a 15-bit colour histogram (Color). Transparent pixels are left out.
//! 2. **Remove noise**: 4-connected same-colour components smaller than `noise` pixels are merged
//!    into their most common neighbouring colour.
//! 3. **Follow boundaries**: for each colour layer, the pixel-crack edges between inside and
//!    outside are linked into closed loops (inside on the right, right turns at saddles, so
//!    diagonal pixels stay separate, matching 4-connectivity). Outer loops run clockwise and
//!    holes counter-clockwise (y down), so the result fills correctly with either fill rule.
//! 4. **Polygon**: each loop is reduced to a polygon whose vertices stay within a fidelity
//!    dependent distance of the pixel boundary (Douglas–Peucker on the crack vertices, which
//!    removes the staircase).
//! 5. **Curves**: the polygon is fitted with cubic Béziers (`vectorcraft_pathops::simplify_with`,
//!    least squares with corner detection); optionally nearly-straight curves snap to lines.
//!
//! Colour layers are traced either *abutting* (each colour's own area; shapes share edges) or
//! *overlapping* (stacked: each layer also covers every layer above it, so no hairline gaps).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod contour;
mod fit;
mod mosaic;
mod quantize;

use serde::{Deserialize, Serialize};
use vectorcraft_geom::PathData;

pub use contour::{Component, Loop, trace_mask};
pub use mosaic::mosaic;
pub use quantize::{Quantized, TRANSPARENT, denoise, quantize};

/// Errors decoding a raster.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("could not decode image: {0}")]
    Decode(String),
    #[error("image is empty")]
    Empty,
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
    /// Decode PNG / JPEG / WebP / GIF bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, TraceError> {
        let img = image::load_from_memory(bytes).map_err(|e| TraceError::Decode(e.to_string()))?.to_rgba8();
        let (w, h) = img.dimensions();
        if w == 0 || h == 0 {
            return Err(TraceError::Empty);
        }
        Ok(Self { width: w, height: h, rgba: img.into_raw() })
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

/// Image Trace parameters (the Image Trace panel).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TraceParams {
    pub mode: Mode,
    /// Black and White: pixels darker than this (luminance 0–255) are black.
    pub threshold: u8,
    /// Palette size for Color, number of grays for Grayscale (2–256).
    pub colors: u32,
    /// Paths (fidelity) 0–100: higher follows the pixels more tightly.
    pub paths: f64,
    /// Corners 0–100: higher keeps more corners.
    pub corners: f64,
    /// Noise: areas smaller than this many pixels are ignored.
    pub noise: u32,
    pub method: Method,
    /// Drop white areas (no white background shape).
    pub ignore_white: bool,
    /// Replace nearly-straight curves with straight lines.
    pub snap_curves_to_lines: bool,
}

impl Default for TraceParams {
    /// Illustrator's [Default]: black and white, threshold 128.
    fn default() -> Self {
        Self {
            mode: Mode::BlackAndWhite,
            threshold: 128,
            colors: 6,
            paths: 50.0,
            corners: 75.0,
            noise: 25,
            method: Method::Abutting,
            ignore_white: false,
            snap_curves_to_lines: false,
        }
    }
}

impl TraceParams {
    /// Maximum distance (px) of polygon vertices from the pixel boundary.
    fn polygon_tolerance(&self) -> f64 {
        let f = (self.paths / 100.0).clamp(0.0, 1.0);
        0.55 + 0.75 * (1.0 - f)
    }
    /// Curve fitting tolerance (px).
    fn fit_tolerance(&self) -> f64 {
        let f = (self.paths / 100.0).clamp(0.0, 1.0);
        0.2 + 1.6 * (1.0 - f)
    }
    /// Turn angle (degrees) above which a polygon vertex stays a corner.
    fn corner_angle(&self) -> f64 {
        let c = (self.corners / 100.0).clamp(0.0, 1.0);
        150.0 - 115.0 * c
    }
}

/// Built-in preset names in panel order (our own parameter sets).
pub const PRESET_NAMES: &[&str] = &[
    "Default",
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
        _ => return None,
    })
}

/// All built-in presets in panel order.
pub fn presets() -> Vec<(&'static str, TraceParams)> {
    PRESET_NAMES.iter().filter_map(|n| Some((*n, preset(n)?))).collect()
}

/// One traced shape: an outer contour plus its holes, filled with `color`.
#[derive(Clone, Debug, PartialEq)]
pub struct TracedPath {
    /// In pixel coordinates (x right, y down; the image spans `0..width × 0..height`).
    pub path: PathData,
    pub color: [u8; 3],
    /// Filled pixel count of the component the path came from.
    pub pixels: usize,
}

/// The traced result, bottom-most path first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraceResult {
    pub paths: Vec<TracedPath>,
    /// Colours actually used.
    pub palette: Vec<[u8; 3]>,
}

impl TraceResult {
    pub fn anchor_count(&self) -> usize {
        self.paths.iter().map(|p| p.path.anchor_count()).sum()
    }
}

fn is_white(c: [u8; 3]) -> bool {
    c.iter().all(|&v| v >= 245)
}

/// Trace `img` with `params`.
pub fn trace(img: &Raster, params: &TraceParams) -> TraceResult {
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 {
        return TraceResult::default();
    }
    let mut q = quantize(img, params);
    denoise(&mut q.labels, w, h, params.noise as usize);
    // Pixel count per palette entry.
    let mut counts = vec![0usize; q.palette.len()];
    for &l in &q.labels {
        if l != TRANSPARENT {
            counts[l as usize] += 1;
        }
    }
    // Layers bottom → top: largest area first.
    let mut order: Vec<usize> = (0..q.palette.len()).filter(|&i| counts[i] > 0).collect();
    order.sort_by(|a, b| counts[*b].cmp(&counts[*a]).then(a.cmp(b)));
    let mut rank = vec![usize::MAX; q.palette.len()];
    for (r, &i) in order.iter().enumerate() {
        rank[i] = r;
    }
    let opts = fit::FitOptions {
        polygon_tol: params.polygon_tolerance(),
        fit_tol: params.fit_tolerance(),
        corner_angle: params.corner_angle(),
        snap_lines: params.snap_curves_to_lines,
    };
    let min_hole = params.noise.max(1) as i64;
    let mut out = TraceResult::default();
    let mut mask = vec![false; w * h];
    for (r, &ci) in order.iter().enumerate() {
        let color = q.palette[ci];
        if params.ignore_white && is_white(color) {
            continue;
        }
        let overlapping = params.method == Method::Overlapping;
        for (m, &l) in mask.iter_mut().zip(&q.labels) {
            *m = l != TRANSPARENT && if overlapping { rank[l as usize] >= r } else { l as usize == ci };
        }
        for comp in trace_mask(&mask, w, h) {
            let mut subs = Vec::with_capacity(1 + comp.holes.len());
            if let Some(sp) = fit::fit_loop(&comp.outer, &opts) {
                subs.push(sp);
            } else {
                continue;
            }
            for hole in &comp.holes {
                if hole.area2.abs() / 2 < min_hole && params.noise > 0 {
                    continue;
                }
                if let Some(sp) = fit::fit_loop(hole, &opts) {
                    subs.push(sp);
                }
            }
            out.paths.push(TracedPath { path: PathData::new(subs), color, pixels: comp.pixels });
        }
        if !out.palette.contains(&color) {
            out.palette.push(color);
        }
    }
    out
}

#[cfg(test)]
mod tests;
