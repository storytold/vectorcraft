//! The built-in provider: Concept to Vector styles made with Image Trace on this computer.
//!
//! Each style is a fixed set of Image Trace settings of VectorCraft's own, and Detail (0 to 100)
//! moves its Paths, Noise and color count within that style's range. The image is decoded once and
//! traced once per style; an image over [`MAX_PIXELS`] is scaled down first, aspect kept, and
//! each variation says so. The paths are mapped back to the image's own pixel size.

use std::sync::Arc;

use vectorcraft_geom::Affine;
use vectorcraft_trace as tr;

use super::{Capabilities, MAX_ANCHORS, Output, Parts, ProviderError, Request, Steps, Style, Variation, VectorPath};

/// The built-in provider's id.
pub const ID: &str = "offline.traceStyles";

/// The most pixels the built-in provider traces an image at (16 megapixels).
pub const MAX_PIXELS: u64 = 16 << 20;

/// The built-in provider's styles, in the order it lists them.
pub const STYLES: [Style; 5] = [
    Style { id: "outline", label: "Outline" },
    Style { id: "inkLines", label: "Ink Lines" },
    Style { id: "flatColor", label: "Flat Color" },
    Style { id: "detailedColor", label: "Detailed Color" },
    Style { id: "grayTones", label: "Gray Tones" },
];

/// Concept to Vector styles made with Image Trace ([`ID`]).
pub struct TraceStyles;

/// `lo` at Detail 0 to `hi` at Detail 100.
fn between(lo: f64, hi: f64, t: f64) -> f64 {
    lo + (hi - lo) * t
}

/// The trace settings of style `id` at Detail `detail` (0 to 100); `None` for an id that isn't one
/// of [`STYLES`].
pub fn style_params(id: &str, detail: u8) -> Option<tr::TraceParams> {
    let t = f64::from(detail.min(100)) / 100.0;
    let noise = |lo: f64, hi: f64| between(hi, lo, t).round().max(1.0) as u32;
    let count = |lo: f64, hi: f64| between(lo, hi, t).round() as u32;
    let base = tr::TraceParams { shapes: false, gradients: false, auto_grouping: false, ..tr::TraceParams::default() };
    Some(match id {
        // Black stroked lines only: thin lines as center lines, wider areas as outlines.
        "outline" => {
            tr::TraceParams { fills: false, strokes: true, ignore_white: true, paths: between(50.0, 90.0, t), noise: noise(4.0, 40.0), ..base }
        }
        // Black filled ink on no background, the straight runs straightened.
        "inkLines" => tr::TraceParams {
            ignore_white: true,
            snap_curves_to_lines: true,
            paths: between(50.0, 95.0, t),
            corners: 80.0,
            noise: noise(4.0, 30.0),
            ..base
        },
        // A few flat colors, abutting.
        "flatColor" => tr::TraceParams {
            mode: tr::Mode::Color,
            colors: count(3.0, 8.0),
            paths: between(40.0, 80.0, t),
            corners: 60.0,
            noise: noise(8.0, 40.0),
            snap_curves_to_lines: true,
            ..base
        },
        // Many colors, stacked so no hairline gaps show.
        "detailedColor" => tr::TraceParams {
            mode: tr::Mode::Color,
            colors: count(12.0, 32.0),
            paths: between(70.0, 95.0, t),
            corners: 40.0,
            noise: noise(2.0, 12.0),
            method: tr::Method::Overlapping,
            ..base
        },
        // Grays, stacked.
        "grayTones" => tr::TraceParams {
            mode: tr::Mode::Grayscale,
            colors: count(4.0, 12.0),
            paths: between(55.0, 90.0, t),
            corners: 50.0,
            noise: noise(4.0, 20.0),
            method: tr::Method::Overlapping,
            ..base
        },
        _ => return None,
    })
}

/// `img` box-filtered down to at most [`MAX_PIXELS`], aspect kept; `img` itself when it fits.
fn fit(img: tr::Raster) -> tr::Raster {
    let (w, h) = (u64::from(img.width), u64::from(img.height));
    if w * h <= MAX_PIXELS || w == 0 || h == 0 {
        return img;
    }
    let k = (MAX_PIXELS as f64 / (w * h) as f64).sqrt();
    let nw = ((w as f64 * k).floor() as u32).clamp(1, img.width);
    let nh = ((h as f64 * k).floor() as u32).clamp(1, img.height);
    let mut rgba = Vec::with_capacity(nw as usize * nh as usize * 4);
    for y in 0..nh {
        let (y0, y1) = (u64::from(y) * h / u64::from(nh), (u64::from(y + 1) * h / u64::from(nh)).max(u64::from(y) * h / u64::from(nh) + 1));
        for x in 0..nw {
            let (x0, x1) = (u64::from(x) * w / u64::from(nw), (u64::from(x + 1) * w / u64::from(nw)).max(u64::from(x) * w / u64::from(nw) + 1));
            let mut sum = [0u64; 4];
            for sy in y0..y1.min(h) {
                for sx in x0..x1.min(w) {
                    for (s, v) in sum.iter_mut().zip(img.pixel(sx as u32, sy as u32)) {
                        *s += u64::from(v);
                    }
                }
            }
            let n = ((y1.min(h) - y0) * (x1.min(w) - x0)).max(1);
            rgba.extend(sum.map(|s| (s / n) as u8));
        }
    }
    tr::Raster { width: nw, height: nh, rgba }
}

impl super::Provider for TraceStyles {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "Offline Image Trace Styles"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { image_input: true, vector_output: true, prompt: false, raster_output: false, offline: true, styles: STYLES.to_vec() }
    }

    fn generate(&self, request: &Request, steps: Steps) -> Result<Vec<Variation>, ProviderError> {
        let params = checked(request)?;
        let total = params.len() as u64;
        if !steps(0, total) {
            return Err(ProviderError::Canceled);
        }
        let image = Decoded::new(request)?;
        let mut left = MAX_ANCHORS;
        let mut out = Vec::with_capacity(params.len());
        for (i, (style, p)) in request.styles.iter().zip(&params).enumerate() {
            let go_on = |_: u64, _: u64| steps(i as u64, total);
            let res = tr::trace_watched(&image.raster, p, left, &go_on).map_err(trace_error)?;
            left = left.saturating_sub(res.anchor_count());
            out.push(image.variation(style, res));
        }
        if !steps(total, total) {
            return Err(ProviderError::Canceled);
        }
        Ok(out)
    }

    /// The image decoded in the first part; then for each style, its colour layers found in one
    /// part and each layer traced in a part of its own ([`tr::TraceSteps`]), or all of them in
    /// one part where the style stacks them (Overlapping with Fills).
    fn parts(&self, request: &Request) -> Option<Box<dyn Parts>> {
        Some(Box::new(TraceParts { request: request.clone(), image: None, trace: None, left: MAX_ANCHORS, out: vec![], ended: false }))
    }
}

/// The trace settings of each of `request`'s styles; an error for a prompt, an output other than
/// vector art, or a style that isn't one of [`STYLES`]. The host refuses these first; a direct
/// caller gets the same answer.
fn checked(request: &Request) -> Result<Vec<tr::TraceParams>, ProviderError> {
    let pid = ID;
    if request.prompt.is_some() || request.output != Output::Vector {
        return Err(ProviderError::Unsupported(format!("`{pid}` takes no prompt and makes vector art only")));
    }
    request
        .styles
        .iter()
        .map(|s| style_params(s, request.detail))
        .collect::<Option<_>>()
        .ok_or_else(|| ProviderError::InvalidInput(format!("`styles` must list 1 to 8 different style ids of `{pid}` (see conceptVector.providers)")))
}

fn trace_error(e: tr::TraceError) -> ProviderError {
    match e {
        tr::TraceError::Cancelled => ProviderError::Canceled,
        tr::TraceError::TooComplex { .. } => ProviderError::ResourceLimit(MAX_ANCHORS),
        e => ProviderError::InvalidInput(e.to_string()),
    }
}

/// A request's image decoded for tracing, with what maps its traced paths back.
struct Decoded {
    raster: Arc<tr::Raster>,
    /// The decoded raster's pixels → the image's own pixel space.
    back: Affine,
    unit: f64,
    warnings: Vec<String>,
}

impl Decoded {
    /// `request`'s image decoded, scaled down to at most [`MAX_PIXELS`].
    fn new(request: &Request) -> Result<Self, ProviderError> {
        let raster = fit(tr::Raster::decode_to_fit(&request.image).map_err(|e| ProviderError::InvalidInput(e.to_string()))?);
        let mut warnings = vec![];
        let (iw, ih) = (request.width.max(1), request.height.max(1));
        if u64::from(raster.width) * u64::from(raster.height) < u64::from(iw) * u64::from(ih) {
            warnings.push(format!("The image was traced at {} × {} pixels, scaled down from {iw} × {ih}.", raster.width, raster.height));
        }
        let back = Affine::scale_non_uniform(f64::from(iw) / f64::from(raster.width.max(1)), f64::from(ih) / f64::from(raster.height.max(1)));
        let unit = back.determinant().abs().sqrt();
        Ok(Self { raster: Arc::new(raster), back, unit, warnings })
    }

    /// The variation of `style` that the trace `res` of this image makes.
    fn variation(&self, style: &str, res: tr::TraceResult) -> Variation {
        let paths = res
            .paths
            .into_iter()
            .map(|tp| {
                let path = tp.path.transformed(self.back);
                match tp.stroke {
                    Some(w) => VectorPath { path, fill: None, stroke: Some((tp.color, w * self.unit)) },
                    None => VectorPath { path, fill: Some(tp.color), stroke: None },
                }
            })
            .collect();
        Variation { style: style.to_string(), paths, warnings: self.warnings.clone() }
    }
}

/// [`TraceStyles`]' variations made a part at a time.
struct TraceParts {
    request: Request,
    image: Option<Decoded>,
    /// The trace of the style after those in `out`, once started.
    trace: Option<tr::TraceSteps>,
    /// The anchor points the styles not made yet may have.
    left: usize,
    out: Vec<Variation>,
    /// The variations or an error have been given.
    ended: bool,
}

impl Parts for TraceParts {
    /// After the variations or an error, each call gives `None`.
    fn step(&mut self) -> Result<Option<Vec<Variation>>, ProviderError> {
        if self.ended {
            return Ok(None);
        }
        let part = self.next_part();
        self.ended = !matches!(part, Ok(None));
        part
    }

    fn styles_done(&self) -> usize {
        self.out.len()
    }
}

impl TraceParts {
    fn next_part(&mut self) -> Result<Option<Vec<Variation>>, ProviderError> {
        let Some(image) = &self.image else {
            checked(&self.request)?;
            self.image = Some(Decoded::new(&self.request)?);
            return Ok(None);
        };
        let Some(style) = self.request.styles.get(self.out.len()) else { return Ok(Some(std::mem::take(&mut self.out))) };
        let trace = match &mut self.trace {
            Some(t) => t,
            None => {
                let params = style_params(style, self.request.detail).ok_or(ProviderError::InvalidResult)?;
                self.trace.insert(tr::TraceSteps::new(Arc::clone(&image.raster), params, self.left))
            }
        };
        let Some(res) = trace.step().map_err(trace_error)? else { return Ok(None) };
        self.trace = None;
        self.left = self.left.saturating_sub(res.anchor_count());
        self.out.push(image.variation(style, res));
        if self.out.len() < self.request.styles.len() {
            return Ok(None);
        }
        Ok(Some(std::mem::take(&mut self.out)))
    }
}
