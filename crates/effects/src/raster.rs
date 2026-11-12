//! Raster (painted) effects: descriptions for the renderer.

use std::borrow::Cow;

use serde_json::{Map, Value, json};
use vectorcraft_doc::color::{BlendMode, Color};
use vectorcraft_doc::rastersettings::MAX_ADD_AROUND;
use vectorcraft_doc::{AppearanceItem, Effect, Node, RasterColorModel, RasterEffectsSettings};

use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use crate::merged_params;
use crate::pixel::{self, PixelFx, PixelSpace};
use crate::util::{flag, num, text};

/// Effect › Rasterize's id.
pub const RASTERIZE: &str = "rasterize";

/// A painted effect, in document units. `blur` is Illustrator's blur distance; the renderer uses
/// a Gaussian with σ = blur / 2 (so the visible spread is about 1.5 × blur).
#[derive(Clone, Debug, PartialEq)]
pub enum RasterFx {
    DropShadow {
        mode: BlendMode,
        opacity: f32,
        dx: f64,
        dy: f64,
        blur: f64,
        color: Color,
        /// Darkness (0..1): the shadow is the object's own colors with this share of black added
        /// instead of `color`; `None` when it is `color`.
        darkness: Option<f32>,
    },
    /// Blurs the art's solid silhouette grown by a quarter of a pixel of the document's raster grid,
    /// as the glows AI files export measure.
    OuterGlow {
        mode: BlendMode,
        opacity: f32,
        blur: f64,
        color: Color,
    },
    InnerGlow {
        mode: BlendMode,
        opacity: f32,
        blur: f64,
        color: Color,
        center: bool,
    },
    /// Soften the object's edges inward over `radius`.
    Feather {
        radius: f64,
    },
    /// Blur the whole object (σ = radius / 2): its colours mixed as they are, or in linear light
    /// (`linear`, as an SVG filter asks with `color-interpolation-filters="linearRGB"`).
    GaussianBlur {
        radius: f64,
        linear: bool,
    },
    /// A Photoshop-style filter over the object's pixels (Radial Blur, Unsharp Mask…).
    Pixel(PixelFx),
    /// Effect › Rasterize: the object drawn as an image of square pixels.
    Rasterize(Rasterize),
}

impl RasterFx {
    /// Painted below the object (shadows, outer glows)?
    pub fn is_below(&self) -> bool {
        matches!(self, RasterFx::DropShadow { .. } | RasterFx::OuterGlow { .. })
    }
    /// The blend mode it paints with (shadows and glows).
    pub fn mode(&self) -> Option<BlendMode> {
        match self {
            RasterFx::DropShadow { mode, .. } | RasterFx::OuterGlow { mode, .. } | RasterFx::InnerGlow { mode, .. } => Some(*mode),
            _ => None,
        }
    }
    /// How far this effect reaches beyond `bounds`, what it paints around (Radial Blur reaches
    /// further around larger objects).
    pub fn outset(&self, bounds: Rect) -> f64 {
        match self {
            RasterFx::DropShadow { dx, dy, blur, .. } => dx.abs().max(dy.abs()) + 1.5 * blur,
            RasterFx::OuterGlow { blur, .. } => 1.5 * blur,
            RasterFx::InnerGlow { .. } | RasterFx::Feather { .. } => 0.0,
            RasterFx::GaussianBlur { radius, .. } => 1.5 * radius,
            RasterFx::Pixel(p) => p.outset(bounds),
            RasterFx::Rasterize(r) => r.outset(),
        }
    }
    /// This effect with a Rasterize that takes the document's resolution at `ppi` ([`Rasterize::at_document_ppi`]).
    pub fn at_document_ppi(&self, ppi: f64) -> RasterFx {
        match self {
            RasterFx::Rasterize(r) => RasterFx::Rasterize(r.at_document_ppi(ppi)),
            fx => fx.clone(),
        }
    }
    /// What this effect paints over when what it paints around covers `bounds`: Rasterize's image
    /// ([`Rasterize::image`], `line` the document's raster grid), `bounds` grown by
    /// [`Self::outset`] for the others.
    pub fn spread(&self, bounds: Rect, line: f64) -> Rect {
        match self {
            RasterFx::Rasterize(r) => r.image(bounds, line),
            RasterFx::OuterGlow { .. } => {
                let o = self.outset(bounds) + 0.25 * line;
                bounds.inflate(o, o)
            }
            fx => {
                let o = fx.outset(bounds);
                bounds.inflate(o, o)
            }
        }
    }
}

/// Effect › Rasterize's options, read from its parameters. The image covers the object's bounds
/// (as far as the effects before it spread the object) and `add_around` points around them, in
/// pixels `72 / ppi` points wide whose first one starts at that area's top-left corner, as Object
/// › Rasterize makes its image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rasterize {
    /// Pixels per inch (1 to 2400); `None` takes Document Raster Effects Settings › Resolution.
    pub ppi: Option<f64>,
    /// The image's color model.
    pub color_model: RasterColorModel,
    /// A white background: the image is opaque. Otherwise it keeps its transparency.
    pub white: bool,
    /// Smooth edges (Art Optimized or Type Optimized). Off, each pixel takes the art at its center.
    pub anti_alias: bool,
    /// Type Optimized: smooth edges with type fitted to the pixels.
    pub hinted: bool,
    /// The image is clipped to the art's outline.
    pub clipping_mask: bool,
    /// Room around the art, in points (0 to [`MAX_ADD_AROUND`]).
    pub add_around: f64,
}

impl Rasterize {
    /// The options in the effect's parameters `p` (defaults merged in).
    fn parse(p: &Value) -> Self {
        let ppi = (!flag(p, "useDocumentResolution", true)).then(|| num(p, "resolution", 72.0).clamp(1.0, 2400.0));
        let color_model = match text(p, "colorModel", "document").to_ascii_lowercase().as_str() {
            "grayscale" | "gray" => RasterColorModel::Grayscale,
            "bitmap" => RasterColorModel::Bitmap,
            _ => RasterColorModel::Document,
        };
        Self {
            ppi,
            color_model,
            white: text(p, "background", "transparent").eq_ignore_ascii_case("white"),
            anti_alias: !text(p, "antiAlias", "art").eq_ignore_ascii_case("none"),
            hinted: text(p, "antiAlias", "art").eq_ignore_ascii_case("type"),
            clipping_mask: flag(p, "clippingMask", false),
            add_around: num(p, "addAround", 0.0).clamp(0.0, MAX_ADD_AROUND),
        }
    }

    /// The width of a pixel of the image in document units: 72 / `ppi`, or `line` (72 / the
    /// document's raster effects resolution) when it takes the document's.
    pub fn cell(&self, line: f64) -> f64 {
        self.ppi.map_or(line, |ppi| 72.0 / ppi)
    }

    /// These options at `ppi` when they take the document's resolution.
    pub fn at_document_ppi(self, ppi: f64) -> Self {
        Self { ppi: self.ppi.or(Some(ppi)), ..self }
    }

    /// How far beyond the object's bounds the image reaches: the room around it and, at a known
    /// resolution, one pixel more for the last column and row, which are whole pixels.
    pub fn outset(&self) -> f64 {
        self.add_around + self.ppi.map_or(0.0, |ppi| 72.0 / ppi)
    }

    /// How far (document units) from a pixel the content that decides it lies: across its pixel
    /// of the image (72 points when the resolution is the document's, not known here).
    pub fn reach(&self) -> f64 {
        self.cell(72.0) * std::f64::consts::SQRT_2
    }

    /// The image of art over `bounds`: `bounds` and the room around them, in whole pixels
    /// ([`Self::cell`] of `line`) from that area's top-left corner.
    pub fn image(&self, bounds: Rect, line: f64) -> Rect {
        let cell = self.cell(line);
        let area = bounds.inflate(self.add_around, self.add_around);
        let (cols, rows) = ((area.width() / cell).ceil().max(1.0), (area.height() / cell).ceil().max(1.0));
        Rect::new(area.x0, area.y0, area.x0 + cols * cell, area.y0 + rows * cell)
    }

    /// Give `pixels` (premultiplied RGBA8), the art drawn on the image's own grid, the background
    /// and then the color model, as Object › Rasterize does.
    pub fn finish(&self, pixels: &mut [u8]) {
        for p in pixels.as_chunks_mut::<4>().0 {
            if self.white {
                let under = 255 - p[3];
                *p = p.map(|c| c.saturating_add(under));
            }
        }
        RasterEffectsSettings { color_model: self.color_model, ..Default::default() }.finish_pixels(pixels);
    }

    /// Draw `image` into `data`, `w` × `h` premultiplied RGBA8 pixels lying in the document as
    /// `space` says: `image` is `cols` × `rows` premultiplied RGBA8 pixels covering `rect` in the
    /// document. In a view that isn't turned, each pixel of `data` takes the mean of the image over
    /// its area (transparent outside the image); otherwise the mean of the image at up to 8 × 8
    /// points across it. Does nothing when the sizes don't match.
    #[allow(clippy::too_many_arguments)]
    pub fn compose(&self, data: &mut [u8], w: usize, h: usize, space: &PixelSpace, image: &[u8], cols: usize, rows: usize, rect: Rect) {
        let (px, rest) = data.as_chunks_mut::<4>();
        let (src, src_rest) = image.as_chunks::<4>();
        let fits = |n: usize, w: usize, h: usize, rest: usize| w > 0 && h > 0 && rest == 0 && w.checked_mul(h) == Some(n);
        if !(fits(px.len(), w, h, rest.len()) && fits(src.len(), cols, rows, src_rest.len()) && rect.width() > 0.0 && rect.height() > 0.0) {
            return;
        }
        let (sx, sy) = (cols as f64 / rect.width(), rows as f64 / rect.height());
        // Document → image pixels.
        let to_image = |p: Point| ((p.x - rect.x0) * sx, (p.y - rect.y0) * sy);
        let at = |i: f64, j: f64| {
            (i >= 0.0 && j >= 0.0 && i < cols as f64 && j < rows as f64).then(|| src.get(j as usize * cols + i as usize).copied()).flatten()
        };
        let straight = matches!(space.to_doc.as_coeffs(), [_, b, c, _, _, _] if b == 0.0 && c == 0.0);
        // Points across a raster pixel in a turned view: about one an image pixel.
        let n = (space.px * sx.max(sy)).ceil().clamp(1.0, 8.0) as usize;
        for (k, p) in px.iter_mut().enumerate() {
            let (x, y) = ((k % w) as f64, (k / w) as f64);
            let mut acc = [0.0f64; 4];
            let weight = if straight {
                let (a, b) = (to_image(space.to_doc * Point::new(x, y)), to_image(space.to_doc * Point::new(x + 1.0, y + 1.0)));
                let (u0, u1, v0, v1) = (a.0.min(b.0), a.0.max(b.0), a.1.min(b.1), a.1.max(b.1));
                let (i0, i1) = (u0.floor().max(0.0), u1.ceil().min(cols as f64));
                let (j0, j1) = (v0.floor().max(0.0), v1.ceil().min(rows as f64));
                let mut j = j0;
                while j < j1 {
                    let mut i = i0;
                    while i < i1 {
                        let share = (u1.min(i + 1.0) - u0.max(i)).max(0.0) * (v1.min(j + 1.0) - v0.max(j)).max(0.0);
                        if let Some(q) = at(i, j) {
                            for (s, v) in acc.iter_mut().zip(q) {
                                *s += share * f64::from(v);
                            }
                        }
                        i += 1.0;
                    }
                    j += 1.0;
                }
                (u1 - u0) * (v1 - v0)
            } else {
                for t in 0..n * n {
                    let (fx, fy) = (((t % n) as f64 + 0.5) / n as f64, ((t / n) as f64 + 0.5) / n as f64);
                    let (i, j) = to_image(space.to_doc * Point::new(x + fx, y + fy));
                    if let Some(q) = at(i.floor(), j.floor()) {
                        for (s, v) in acc.iter_mut().zip(q) {
                            *s += f64::from(v);
                        }
                    }
                }
                (n * n) as f64
            };
            *p = if weight > 0.0 { acc.map(|s| (s / weight).round().clamp(0.0, 255.0) as u8) } else { [0; 4] };
        }
    }

    /// Turn `data`, `w` × `h` premultiplied RGBA8 pixels of the object lying in the document as
    /// `space` says, into its image over `bounds` (what the object covers after the effects before
    /// this one), when the image is too large to draw on its own grid. Each pixel of the image more
    /// than a raster pixel wide takes the mean of the raster pixels whose centers lie in it
    /// (anti-aliased) or the raster pixel at its center (not anti-aliased). Smaller pixels are left
    /// as drawn. Then the background and the color model apply as Object › Rasterize applies them.
    /// What lies outside the image becomes transparent, and in a view that isn't turned the raster
    /// pixels along its edges keep the share of them it covers. Does nothing when the sizes don't
    /// match. The renderer draws the content with hard edges when it goes into Rasterize with
    /// anti-aliasing off and image pixels at least a raster pixel wide.
    pub fn apply(&self, data: &mut [u8], w: usize, h: usize, space: &PixelSpace, bounds: Rect) {
        let cell = self.cell(space.line);
        let (px, rest) = data.as_chunks_mut::<4>();
        if w == 0 || h == 0 || !rest.is_empty() || w.checked_mul(h) != Some(px.len()) || !(cell.is_finite() && cell > 0.0 && space.px > 0.0) {
            return;
        }
        let area = self.image(bounds, space.line);
        let (cols, rows) = ((area.width() / cell).round().max(1.0), (area.height() / cell).round().max(1.0));
        // The pixel of the image (column, row) that a raster pixel's center lies in.
        let pixel_at = |x: usize, y: usize| {
            let p = space.to_doc * Point::new(x as f64 + 0.5, y as f64 + 0.5);
            let (c, r) = (((p.x - area.x0) / cell).floor(), ((p.y - area.y0) / cell).floor());
            (c >= 0.0 && r >= 0.0 && c < cols && r < rows).then_some((c, r))
        };
        let wide = cell / space.px;
        // The image's pixels the raster covers: columns c0..c1, rows r0..r1.
        let seen = space.to_doc.transform_rect_bbox(Rect::new(0.0, 0.0, w as f64, h as f64));
        let (c0, c1) = (((seen.x0 - area.x0) / cell).floor().clamp(0.0, cols), ((seen.x1 - area.x0) / cell).ceil().clamp(0.0, cols));
        let (r0, r1) = (((seen.y0 - area.y0) / cell).floor().clamp(0.0, rows), ((seen.y1 - area.y0) / cell).ceil().clamp(0.0, rows));
        let span = (c1 - c0) as usize;
        let slot = |(c, r): (f64, f64)| ((r - r0) as usize).saturating_mul(span).saturating_add((c - c0) as usize);
        // Anti-aliased: the sums of each pixel's raster pixels (premultiplied) and their count,
        // which saturate past 16 million raster pixels to an image pixel. They are kept for the
        // whole grid when it has no more pixels than the raster, else for the pixels the raster
        // reaches (a turned view of a narrow raster).
        let grid = span.saturating_mul((r1 - r0) as usize);
        let whole = grid <= px.len();
        let mut dense: Vec<[u32; 5]> = vec![];
        let mut sparse: std::collections::HashMap<usize, [u32; 5]> = std::collections::HashMap::new();
        if self.anti_alias && wide > 1.0 {
            if whole {
                dense = vec![[0; 5]; grid];
            }
            for (i, p) in px.iter().enumerate() {
                let Some(k) = pixel_at(i % w, i / w).map(slot) else { continue };
                let s = if whole { dense.get_mut(k) } else { Some(sparse.entry(k).or_default()) };
                if let Some(s) = s {
                    for (acc, v) in s.iter_mut().zip(p) {
                        *acc = acc.saturating_add(u32::from(*v));
                    }
                    s[4] = s[4].saturating_add(1);
                }
            }
        }
        let sum = |k: usize| if whole { dense.get(k) } else { sparse.get(&k) };
        // Not anti-aliased: the raster as drawn, to read each pixel's center from.
        let drawn = (!self.anti_alias && wide > 1.0).then(|| px.to_vec());
        let to_raster = space.to_doc.inverse();
        let model = RasterEffectsSettings { color_model: self.color_model, ..Default::default() };
        // In a view that isn't turned, the raster pixels along the image's edges take the share of
        // them it covers; otherwise those whose center lies in it are in it.
        let straight = matches!(space.to_doc.as_coeffs(), [_, b, c, _, _, _] if b == 0.0 && c == 0.0);
        let image = to_raster.transform_rect_bbox(area);
        let cover =
            |x: f64, y: f64| (image.x1.min(x + 1.0) - image.x0.max(x)).clamp(0.0, 1.0) * (image.y1.min(y + 1.0) - image.y0.max(y)).clamp(0.0, 1.0);
        for (i, p) in px.iter_mut().enumerate() {
            let (x, y) = (i % w, i / w);
            let (at, share) = if straight {
                // The image pixel nearest the raster pixel's center.
                let q = space.to_doc * Point::new(x as f64 + 0.5, y as f64 + 0.5);
                let at = (((q.x - area.x0) / cell).floor().clamp(0.0, cols - 1.0), ((q.y - area.y0) / cell).floor().clamp(0.0, rows - 1.0));
                (Some(at), cover(x as f64, y as f64))
            } else {
                (pixel_at(x, y), 1.0)
            };
            let Some(at) = at.filter(|_| share > 0.0) else {
                *p = [0; 4];
                continue;
            };
            // An image pixel's color, over the share of the raster pixel the image covers. (A raster
            // pixel left as drawn holds art the image covers whole.)
            let made = if let Some(s) = sum(slot(at)).filter(|s| s[4] > 0) {
                Some([0, 1, 2, 3].map(|k| ((u64::from(s[k]) + u64::from(s[4] / 2)) / u64::from(s[4])).min(255) as u8))
            } else {
                drawn.as_ref().map(|drawn| {
                    let center = to_raster * Point::new(area.x0 + (at.0 + 0.5) * cell, area.y0 + (at.1 + 0.5) * cell);
                    let (x, y) = (center.x.floor(), center.y.floor());
                    let inside = x >= 0.0 && y >= 0.0 && x < w as f64 && y < h as f64;
                    if inside { drawn.get(y as usize * w + x as usize).copied().unwrap_or_default() } else { [0; 4] }
                })
            };
            if let Some(v) = made {
                *p = if share < 1.0 { v.map(|c| (f64::from(c) * share).round() as u8) } else { v };
            }
            // White goes under the art first, over the image's share of the pixel, then the color
            // model applies, as in Object › Rasterize.
            if self.white {
                let under = ((255.0 * share).round() as u8).saturating_sub(p[3]);
                *p = p.map(|c| c.saturating_add(under));
            }
            model.finish_pixels(p);
        }
    }
}

/// Parse a blend mode name (`"multiply"`, `"Screen"`, `"color-burn"`…).
pub fn blend_mode(s: &str) -> BlendMode {
    let key: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
    BlendMode::ALL.iter().copied().find(|m| format!("{m:?}").to_ascii_lowercase() == key).unwrap_or(BlendMode::Normal)
}

/// An effect's `color` parameter (`"#rrggbb"`, `[r, g, b]` 0..1 or a colour object).
pub fn color_param(p: &Value, default: Color) -> Color {
    match p.get("color") {
        Some(Value::String(s)) => Color::from_hex(s).unwrap_or(default),
        Some(Value::Array(a)) if a.len() >= 3 => {
            let f = |i: usize| a[i].as_f64().unwrap_or(0.0) as f32;
            Color::rgb(f(0), f(1), f(2))
        }
        Some(v @ Value::Object(_)) => serde_json::from_value(v.clone()).unwrap_or(default),
        _ => default,
    }
}

fn opacity(p: &Value) -> f32 {
    (num(p, "opacity", 75.0) / 100.0).clamp(0.0, 1.0) as f32
}

/// The visible raster effects of `effects`, in stack order, each following the Transform effects
/// listed after it ([`followed`]).
pub fn raster_effects(effects: &[Effect]) -> Vec<RasterFx> {
    followed(effects)
        .iter()
        .filter(|e| e.visible)
        .filter_map(|e| {
            let p = merged_params(&e.id, &e.params);
            let blur = num(&p, "blur", 5.0).clamp(0.0, 1000.0);
            Some(match e.id.as_str() {
                "stylize.dropShadow" => RasterFx::DropShadow {
                    mode: blend_mode(text(&p, "mode", "multiply")),
                    opacity: opacity(&p),
                    dx: num(&p, "x", 7.0).clamp(-1e4, 1e4),
                    dy: num(&p, "y", 7.0).clamp(-1e4, 1e4),
                    blur,
                    color: color_param(&p, Color::BLACK),
                    darkness: text(&p, "shade", "color")
                        .eq_ignore_ascii_case("darkness")
                        .then(|| (num(&p, "darkness", 100.0) / 100.0).clamp(0.0, 1.0) as f32),
                },
                "stylize.outerGlow" => RasterFx::OuterGlow {
                    mode: blend_mode(text(&p, "mode", "screen")),
                    opacity: opacity(&p),
                    blur,
                    color: color_param(&p, Color::rgb(1.0, 1.0, 0.0)),
                },
                "stylize.innerGlow" => RasterFx::InnerGlow {
                    mode: blend_mode(text(&p, "mode", "screen")),
                    opacity: opacity(&p),
                    blur,
                    color: color_param(&p, Color::WHITE),
                    center: text(&p, "source", "edge").eq_ignore_ascii_case("center"),
                },
                "stylize.feather" => RasterFx::Feather { radius: num(&p, "radius", 5.0).clamp(0.0, 1000.0) },
                "blur.gaussian" => RasterFx::GaussianBlur { radius: num(&p, "radius", 5.0).clamp(0.0, 1000.0), linear: flag(&p, "linear", false) },
                RASTERIZE => RasterFx::Rasterize(Rasterize::parse(&p)),
                id => RasterFx::Pixel(pixel::parse(id, &p)?),
            })
        })
        .collect()
}

/// `effects` with each raster effect following the visible Transform effects listed after it
/// ([`follow_transform`]): effects apply in the order they are listed, so a Transform effect
/// transforms what the raster effects before it paint, whatever its Scale Strokes & Effects says.
/// A hidden raster effect follows too, so it draws the same when it is shown after the Transform
/// effect has been expanded. A raster effect that a Transform effect making copies or moving at
/// random follows stays as it is, as does one after the last Transform effect. Borrowed when
/// nothing changes.
pub(crate) fn followed(effects: &[Effect]) -> Cow<'_, [Effect]> {
    if !effects.iter().any(|e| e.visible && e.id == "distort.transform") {
        return Cow::Borrowed(effects);
    }
    let mut out: Option<Vec<Effect>> = None;
    // The linear part of the Transform effects after the effect at hand, the last one outermost;
    // `None` once one of them makes copies or moves at random.
    let mut after = Some(Affine::IDENTITY);
    for (i, e) in effects.iter().enumerate().rev() {
        if e.id == "distort.transform" {
            after = after.and_then(|a| transform_step(e).map(|s| a * s));
            continue;
        }
        let Some(a) = after.filter(|a| *a != Affine::IDENTITY && follows(&e.id)) else { continue };
        let list = out.get_or_insert_with(|| effects.to_vec());
        if let Some(e) = list.get_mut(i) {
            follow_transform(e, a);
        }
    }
    out.map_or(Cow::Borrowed(effects), Cow::Owned)
}

/// `effects` without their geometry effects, each raster effect following the Transform effects
/// it loses ([`followed`]): what an object, or a fill or stroke, keeps once its geometry effects
/// are applied to its art.
pub fn without_geometry(effects: &[Effect]) -> Vec<Effect> {
    followed(effects).iter().filter(|e| !crate::is_geometry(&e.id)).cloned().collect()
}

/// Make the raster effects of `effects`, all listed before Transform effect `t`, follow `t` as
/// [`followed`] makes them follow it, for art that applies its effects one at a time. `false`
/// when `t` leaves them as they are.
pub(crate) fn follow_one(effects: &mut [Effect], t: &Effect) -> bool {
    let Some(a) = transform_step(t).filter(|a| *a != Affine::IDENTITY) else { return false };
    let mut changed = false;
    for e in effects.iter_mut().filter(|e| follows(&e.id)) {
        follow_transform(e, a);
        changed = true;
    }
    changed
}

/// The linear part of the visible Transform effects of `owner`, an object's or a container's own
/// effects, in order: what they do to the raster effects of everything the object paints (its
/// fills and strokes, and a container's members and theirs), which those effects transform with
/// the rest of the art. `None` when they leave them as they are: no Transform effect moves the
/// art, or one of them makes copies or moves at random ([`followed`]).
pub(crate) fn contents_step(owner: &[Effect]) -> Option<Affine> {
    let a = owner.iter().filter(|e| e.visible && e.id == "distort.transform").try_fold(Affine::IDENTITY, |a, e| transform_step(e).map(|s| s * a))?;
    (a != Affine::IDENTITY).then_some(a)
}

/// Make the raster effects of `effects`, the own effects of something inside an object (a fill's
/// or stroke's, or a member's), follow `a`, the linear part of the object's Transform effects
/// ([`contents_step`]), which apply after all of them. `outer` is what the Transform effects
/// between them and `a` do to them (those of the fill's or stroke's object and of the groups that
/// hold it inside the object). A raster effect that the Transform effects after it in `effects`
/// move ([`followed`]) is set so that it draws as those effects, then `outer` and then `a`
/// transform it, whichever of them is applied first. One that a Transform effect after it in
/// `effects` holds back keeps its value through those effects, so it is set to draw as `outer` and
/// then `a` transform it. `false` when nothing changed.
pub(crate) fn follow_contents(effects: &mut [Effect], a: Affine, outer: Affine) -> bool {
    if a == Affine::IDENTITY || !effects.iter().any(|e| follows(&e.id)) {
        return false;
    }
    let mut changed = false;
    // The linear part of the Transform effects after the effect at hand, as `followed` reads it;
    // `None` once one of them holds the effect back, which they then leave as it is.
    let mut after = Some(Affine::IDENTITY);
    for e in effects.iter_mut().rev() {
        if e.id == "distort.transform" {
            after = after.and_then(|b| transform_step(e).map(|s| b * s));
            continue;
        }
        if !follows(&e.id) {
            continue;
        }
        let b = outer * after.unwrap_or(Affine::IDENTITY);
        // The effect is still to be mapped by `b` when it is drawn, so it takes b⁻¹·a·b now:
        // drawn, it is a·b of what it was. A `b` that flattens the art takes `a` alone.
        let det = b.determinant();
        let c = if b == Affine::IDENTITY || !(det.is_finite() && det.abs() > 1e-12) { a } else { b.inverse() * a * b };
        if c.is_finite() {
            follow_transform(e, c);
            changed = true;
        }
    }
    changed
}

/// Make the raster effects of the fills and strokes of `n` follow the Transform effects of its own
/// effect list ([`contents_step`]), as they do while the object is drawn with them: for art that
/// applies those effects and keeps the fills and strokes. `false` when nothing changed.
pub fn follow_items(n: &mut Node) -> bool {
    let Some(a) = contents_step(&n.appearance.effects) else { return false };
    follow_items_by(n, a)
}

/// [`follow_items`] for `n` holding the fills and strokes of an object whose own effects are
/// `owner` (the area of a compound shape, which carries none of the shape's effects).
pub(crate) fn follow_items_of(n: &mut Node, owner: &[Effect]) -> bool {
    let Some(a) = contents_step(owner) else { return false };
    follow_items_by(n, a)
}

/// Make the raster effects of the fills and strokes of `n` follow `a` ([`follow_contents`]).
fn follow_items_by(n: &mut Node, a: Affine) -> bool {
    if !n.appearance.items.iter().any(|i| i.effects().iter().any(|e| follows(&e.id))) {
        return false;
    }
    let mut changed = false;
    for item in &mut n.appearance.items {
        changed |= follow_contents(item.effects_mut(), a, Affine::IDENTITY);
    }
    changed
}

/// Make the raster effects of `items`, the fills and strokes of an object whose own effects are
/// `owner`, follow the Transform effects of `owner` ([`follow_contents`]): for art that applies
/// those effects to the object's path and draws each fill and stroke on it as a node of its own.
pub(crate) fn follow_paints(items: &mut [AppearanceItem], owner: &[Effect]) {
    let Some(a) = contents_step(owner) else { return };
    for item in items {
        follow_contents(item.effects_mut(), a, Affine::IDENTITY);
    }
}

/// `effects`, those of a fill or stroke of an object whose own effects are `owner`, with each
/// raster effect following the Transform effects of `owner` as well ([`follow_contents`]): the
/// list to draw that fill's or stroke's raster effects from ([`raster_effects`]). Borrowed when
/// nothing changes.
pub fn item_effects_in<'a>(owner: &[Effect], effects: &'a [Effect]) -> Cow<'a, [Effect]> {
    let Some(a) = contents_step(owner) else { return Cow::Borrowed(effects) };
    if !effects.iter().any(|e| follows(&e.id)) {
        return Cow::Borrowed(effects);
    }
    let mut out = effects.to_vec();
    if follow_contents(&mut out, a, Affine::IDENTITY) { Cow::Owned(out) } else { Cow::Borrowed(effects) }
}

/// How many of the effects of `effects` that [`without_geometry`] keeps no longer follow the
/// Transform effects listed after `effects` ([`followed`]): those before the last visible Transform
/// effect among them that makes copies or moves at random.
pub(crate) fn held_back(effects: &[Effect]) -> usize {
    let Some(last) = effects.iter().rposition(|e| transform_step(e).is_none()) else { return 0 };
    effects.get(..last).unwrap_or_default().iter().filter(|e| !crate::is_geometry(&e.id)).count()
}

/// Does `e` stop the raster effects listed before it from following the Transform effects after
/// it ([`followed`]): is it a visible Transform effect that makes copies or moves at random?
pub(crate) fn holds_back(e: &Effect) -> bool {
    transform_step(e).is_none()
}

/// What `e` does to the raster effects listed before it ([`followed`]): the linear part of its map
/// when it is a visible Transform effect that moves the art once, the identity when it is no
/// Transform effect, is hidden or has Transform Objects off (the art stays where it is), and `None`
/// when it makes copies or moves at random.
fn transform_step(e: &Effect) -> Option<Affine> {
    if e.id != "distort.transform" || !e.visible {
        return Some(Affine::IDENTITY);
    }
    let p = merged_params(&e.id, &e.params);
    if !flag(&p, "objects", true) {
        return Some(Affine::IDENTITY);
    }
    let (map, copies) = crate::distort::transform_map(Rect::ZERO, &p, 0);
    (copies == 0 && !flag(&p, "random", false)).then(|| map.with_translation(Vec2::ZERO))
}

/// Does raster effect `id` follow the Transform effects listed after it ([`followed`])? An SVG
/// filter does not, and neither does Rasterize, whose image keeps its own pixel grid and room
/// around the art.
pub(crate) fn follows(id: &str) -> bool {
    crate::is_raster(id) && id != crate::SVG_FILTER && id != RASTERIZE
}

/// Make raster effect `e` follow `a`, the linear part of a Transform effect listed after it, as
/// that effect transforms what `e` paints: its distances and raster sizes scale by `a`'s mean
/// scale ([`crate::scale_effect`]), and a Drop Shadow's offset turns, reflects and scales with
/// `a`. Effects that [`follows`] leaves out stay as they are.
fn follow_transform(e: &mut Effect, a: Affine) {
    if !follows(&e.id) {
        return;
    }
    let offset = (e.id == "stylize.dropShadow").then(|| {
        let p = merged_params(&e.id, &e.params);
        a * Point::new(num(&p, "x", 7.0), num(&p, "y", 7.0))
    });
    if let Some(k) = vectorcraft_doc::Scaling::factor(a) {
        crate::scale_effect(e, k);
    }
    if let Some(o) = offset.filter(|o| o.x.is_finite() && o.y.is_finite()) {
        if !e.params.is_object() {
            e.params = Value::Object(Map::new());
        }
        e.params["x"] = json!(o.x);
        e.params["y"] = json!(o.y);
    }
}

/// How far the visible raster effects of `effects` paint beyond `bounds`, the (effected)
/// geometry's, in document units. Geometry effects are accounted for by evaluating them.
pub fn outset(effects: &[Effect], bounds: Rect) -> f64 {
    raster_effects(effects).iter().map(|fx| fx.outset(bounds)).fold(0.0, f64::max)
}
