//! SVG filters → live effects. A filter is read by evaluating its primitives on symbols (the
//! source, its silhouette blurred, moved or inverted, a colour flood, a coloured silhouette…),
//! and the result recognised as one of the effects the canvas draws: Gaussian Blur, Drop Shadow,
//! Outer Glow, Inner Glow or Feather. That covers `feGaussianBlur`, `feDropShadow`, the usual
//! shadow chains (offset, blur, flood or color matrix, composite, merge), the object moved and
//! blurred with its colors scaled by `feComponentTransfer` (a drop shadow with Darkness) and what
//! our export writes, so export → import keeps the effects.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;
use usvg::filter::{ColorInterpolation, ColorMatrixKind, CompositeOperator, Filter, Input, Kind, MorphologyOperator, TransferFunction};
use usvg::roxmltree;
use vectorcraft_color::Color;
use vectorcraft_doc::Effect;
use vectorcraft_geom::{Affine, Vec2};

/// Attribute our export writes on the filter of a shadow or glow whose blend mode isn't normal
/// (filters composite normally; the mode comes back on import).
pub(crate) const BLEND: &str = "data-vc-blend";

/// The blend modes our export recorded ([`BLEND`]), by filter id.
pub(super) fn blends(xml: &roxmltree::Document) -> HashMap<String, String> {
    if !xml.input_text().contains(BLEND) {
        return HashMap::new();
    }
    xml.descendants()
        .filter(|n| n.tag_name().name() == "filter")
        .filter_map(|n| Some((n.attribute("id")?.to_string(), n.attribute(BLEND)?.to_string())))
        .collect()
}

/// The object's alpha, blurred (σ), moved and maybe inverted.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Shape {
    sd: f64,
    dx: f64,
    dy: f64,
    moved: bool,
    inverted: bool,
}

impl Shape {
    /// The silhouette as it is.
    fn plain(self) -> bool {
        self == Shape::default()
    }
}

/// What a filter primitive's result is, as far as the effects go.
#[derive(Clone, Debug)]
enum V {
    /// `SourceGraphic`.
    Graphic,
    /// The source blurred (σ), on linear RGB values when `true`.
    Blurred(f64, bool),
    /// A black silhouette.
    Alpha(Shape),
    Flood(Color, f32),
    /// A silhouette painted a colour and opacity; `clipped` to the object's own shape.
    Paint {
        shape: Shape,
        color: Color,
        opacity: f32,
        clipped: bool,
    },
    /// The source inside its own blurred silhouette (σ).
    Feathered(f64),
    /// The source blurred (σ `sd`, on linear RGB values when `linear`), moved, its colors times
    /// `keep` and its alpha times `opacity`: a drop shadow with Darkness 1 − `keep`.
    Shade {
        sd: f64,
        linear: bool,
        dx: f64,
        dy: f64,
        keep: f32,
        opacity: f32,
    },
    /// Layers, bottom first.
    Stack(Vec<V>),
}

impl V {
    fn layers(self) -> Vec<V> {
        match self {
            V::Stack(l) => l,
            v => vec![v],
        }
    }

    /// Blurred by σ `sd` (blurs add up as variances), on linear RGB values when `linear`.
    fn blur(self, sd: f64, linear: bool) -> Option<V> {
        let add = |a: f64| (a * a + sd * sd).sqrt();
        Some(match self {
            v if sd <= 0.0 => v,
            V::Graphic => V::Blurred(sd, linear),
            V::Blurred(a, l) => V::Blurred(add(a), l || linear),
            V::Alpha(s) => V::Alpha(Shape { sd: add(s.sd), ..s }),
            V::Paint { shape, color, opacity, clipped: false } => {
                V::Paint { shape: Shape { sd: add(shape.sd), ..shape }, color, opacity, clipped: false }
            }
            V::Shade { sd: s, linear: l, dx, dy, keep, opacity } => V::Shade { sd: add(s), linear: l || linear, dx, dy, keep, opacity },
            _ => return None,
        })
    }

    fn offset(self, dx: f64, dy: f64) -> Option<V> {
        let moved = |s: Shape| Shape { dx: s.dx + dx, dy: s.dy + dy, moved: true, ..s };
        Some(match self {
            V::Alpha(s) => V::Alpha(moved(s)),
            V::Paint { shape, color, opacity, clipped: false } => V::Paint { shape: moved(shape), color, opacity, clipped: false },
            v @ (V::Graphic | V::Blurred(..)) if dx == 0.0 && dy == 0.0 => v,
            V::Graphic => V::Shade { sd: 0.0, linear: false, dx, dy, keep: 1.0, opacity: 1.0 },
            V::Blurred(sd, linear) => V::Shade { sd, linear, dx, dy, keep: 1.0, opacity: 1.0 },
            V::Shade { sd, linear, dx: x, dy: y, keep, opacity } => V::Shade { sd, linear, dx: x + dx, dy: y + dy, keep, opacity },
            _ => return None,
        })
    }

    /// The source, blurred or moved, with each color channel times `keep` and its alpha times
    /// `opacity` (`feComponentTransfer` with linear functions through 0).
    fn scaled(self, keep: f32, opacity: f32) -> Option<V> {
        let (sd, linear, dx, dy, k, o) = match self {
            V::Graphic => (0.0, false, 0.0, 0.0, 1.0, 1.0),
            V::Blurred(sd, linear) => (sd, linear, 0.0, 0.0, 1.0, 1.0),
            V::Shade { sd, linear, dx, dy, keep, opacity } => (sd, linear, dx, dy, keep, opacity),
            _ => return None,
        };
        Some(V::Shade { sd, linear, dx, dy, keep: k * keep, opacity: o * opacity })
    }

    /// `self` in the alpha of `mask` (`feComposite operator="in"`).
    fn inside(self, mask: V) -> Option<V> {
        // The source's alpha is its silhouette.
        let mask = match mask {
            V::Graphic => V::Alpha(Shape::default()),
            m => m,
        };
        Some(match (self, mask) {
            (V::Flood(color, opacity), V::Alpha(shape)) => V::Paint { shape, color, opacity, clipped: false },
            (V::Paint { shape, color, opacity, .. }, V::Alpha(s)) if s.plain() => V::Paint { shape, color, opacity, clipped: true },
            (V::Graphic, V::Alpha(s)) if !s.moved && !s.inverted => V::Feathered(s.sd),
            (f @ V::Feathered(_), V::Alpha(s)) if s.plain() => f,
            _ => return None,
        })
    }

    /// A silhouette (or the blurred source, by its alpha) painted by a colour matrix that ignores
    /// the colour channels: the matrix's constant colour, at its alpha scale.
    fn color_matrix(self, m: &[f32]) -> Option<V> {
        let shape = match self {
            V::Alpha(s) => s,
            V::Blurred(sd, _) => Shape { sd, ..Shape::default() },
            _ => return None,
        };
        let row = |r: usize| m.get(r * 5..r * 5 + 5);
        let (Some(r), Some(g), Some(b), Some(a)) = (row(0), row(1), row(2), row(3)) else { return None };
        let colour_only = [r, g, b].iter().all(|row| row.iter().take(4).all(|v| *v == 0.0));
        let alpha_only = a.iter().take(3).all(|v| *v == 0.0) && a.get(4) == Some(&0.0);
        let (Some(cr), Some(cg), Some(cb), Some(k)) = (r.get(4), g.get(4), b.get(4), a.get(3)) else { return None };
        (colour_only && alpha_only).then(|| V::Paint {
            shape,
            color: Color::rgb(cr.clamp(0.0, 1.0), cg.clamp(0.0, 1.0), cb.clamp(0.0, 1.0)),
            opacity: k.clamp(0.0, 1.0),
            clipped: false,
        })
    }
}

/// σ of a blur with standard deviations `x`, `y` (their mean: the effects blur evenly).
fn sd(x: f32, y: f32) -> f64 {
    (x as f64 + y as f64) / 2.0
}

fn color(c: usvg::Color) -> Color {
    Color::rgb8(c.red, c.green, c.blue)
}

/// Evaluate one filter's primitives, `k` the scale from the filter's user space to the document;
/// `None` when one does something the effects can't.
fn eval(f: &Filter, k: f64) -> Option<V> {
    let mut results: HashMap<&str, V> = HashMap::new();
    let mut last = V::Graphic;
    for p in f.primitives() {
        let input = |i: &Input| -> Option<V> {
            match i {
                Input::SourceGraphic => Some(V::Graphic),
                Input::SourceAlpha => Some(V::Alpha(Shape::default())),
                Input::Reference(r) => results.get(r.as_str()).cloned(),
            }
        };
        let v = match p.kind() {
            Kind::GaussianBlur(b) => {
                let linear = p.color_interpolation() == ColorInterpolation::LinearRGB;
                input(b.input())?.blur(sd(b.std_dev_x().get(), b.std_dev_y().get()), linear)?
            }
            Kind::Offset(o) => input(o.input())?.offset(o.dx() as f64, o.dy() as f64)?,
            Kind::Flood(fl) => V::Flood(color(fl.color()), fl.opacity().get()),
            Kind::Composite(c) => {
                let (a, b) = (input(c.input1())?, input(c.input2())?);
                match c.operator() {
                    CompositeOperator::In => a.inside(b)?,
                    CompositeOperator::Over => V::Stack(b.layers().into_iter().chain(a.layers()).collect()),
                    _ => return None,
                }
            }
            Kind::Merge(m) => V::Stack(m.inputs().iter().map(input).collect::<Option<Vec<_>>>()?.into_iter().flat_map(V::layers).collect()),
            Kind::ColorMatrix(c) => match c.kind() {
                ColorMatrixKind::Matrix(m) => input(c.input())?.color_matrix(m)?,
                _ => return None,
            },
            Kind::ComponentTransfer(c) => {
                let identity = [c.func_r(), c.func_g(), c.func_b()].iter().all(|f| matches!(f, TransferFunction::Identity));
                let invert = match c.func_a() {
                    TransferFunction::Table(t) => t.as_slice() == [1.0, 0.0],
                    TransferFunction::Linear { slope, intercept } => *slope == -1.0 && *intercept == 1.0,
                    _ => false,
                };
                // A scale (a linear function through 0, or none) of each channel.
                let scale = |f: &TransferFunction| match f {
                    TransferFunction::Identity => Some(1.0),
                    TransferFunction::Linear { slope, intercept } if *intercept == 0.0 && (0.0..=1.0).contains(slope) => Some(*slope),
                    _ => None,
                };
                // The same scale of R, G and B, on sRGB values (a scale of 0 or 1 is the same on
                // linear ones).
                let srgb = p.color_interpolation() == ColorInterpolation::SRGB;
                let keep = match [c.func_r(), c.func_g(), c.func_b()].map(scale) {
                    [Some(r), Some(g), Some(b)] if r == g && g == b && (srgb || r == 0.0 || r == 1.0) => Some(r),
                    _ => None,
                };
                // Our Outer Glow's solid silhouette (alpha × 255): an object's silhouette as far as the effects go.
                let solid = matches!(c.func_a(), TransferFunction::Linear { slope, intercept } if *slope >= 255.0 && *intercept == 0.0);
                match (input(c.input())?, keep, scale(c.func_a())) {
                    (V::Alpha(s), _, _) if identity && invert && s.sd == 0.0 => V::Alpha(Shape { inverted: !s.inverted, ..s }),
                    (V::Alpha(s), _, _) if identity && solid && s.plain() => V::Alpha(s),
                    (v @ (V::Graphic | V::Blurred(..) | V::Shade { .. }), Some(keep), Some(opacity)) => v.scaled(keep, opacity)?,
                    _ => return None,
                }
            }
            // Our Outer Glow's silhouette grown by a quarter raster pixel (`export`): a dilation of a
            // plain silhouette by up to that much at 72 ppi is that step.
            Kind::Morphology(m)
                if m.operator() == MorphologyOperator::Dilate
                    && m.radius_x().get() == m.radius_y().get()
                    && f64::from(m.radius_x().get()) * k <= 0.25 + 1e-6 =>
            {
                match input(m.input())? {
                    V::Alpha(s) if s.plain() => V::Alpha(s),
                    _ => return None,
                }
            }
            Kind::DropShadow(d) if matches!(input(d.input())?, V::Graphic) => {
                let shape =
                    Shape { sd: sd(d.std_dev_x().get(), d.std_dev_y().get()), dx: d.dx() as f64, dy: d.dy() as f64, moved: true, inverted: false };
                V::Stack(vec![V::Paint { shape, color: color(d.color()), opacity: d.opacity().get(), clipped: false }, V::Graphic])
            }
            _ => return None,
        };
        results.insert(p.result(), v.clone());
        last = v;
    }
    Some(last)
}

/// The live effects `filters` (in the order they apply) stand for, in user space mapped to the
/// document by `ts`; `blends`: the modes our export recorded ([`blends`]). `None` when one of
/// them isn't an effect the canvas draws.
pub(super) fn effects(filters: &[Arc<Filter>], ts: Affine, blends: &HashMap<String, String>) -> Option<Vec<Effect>> {
    let k = ts.determinant().abs().sqrt();
    let [a, b, c, d, _, _] = ts.as_coeffs();
    let along = |dx: f64, dy: f64| Vec2::new(a * dx + c * dy, b * dx + d * dy);
    let num = |v: f64| (v * 1000.0).round() / 1000.0;
    let mut out = vec![];
    for f in filters {
        let blur = |sd: f64| num(2.0 * sd * k);
        let mode = blends.get(f.id()).map_or("normal", String::as_str);
        let paint = |c: Color, o: f32| (c.to_hex(), num(o as f64 * 100.0));
        let (id, params) = match eval(f, k)? {
            V::Graphic => continue,
            V::Blurred(sd, _) => ("blur.gaussian", json!({ "radius": blur(sd) })),
            V::Feathered(sd) => ("stylize.feather", json!({ "radius": blur(sd) })),
            V::Stack(layers) => match layers.as_slice() {
                [V::Paint { shape, color, opacity, clipped: false }, V::Graphic] if !shape.inverted => {
                    let (color, opacity) = paint(*color, *opacity);
                    if shape.moved {
                        let v = along(shape.dx, shape.dy);
                        let p = json!({ "x": num(v.x), "y": num(v.y), "blur": blur(shape.sd), "color": color, "opacity": opacity, "mode": mode });
                        ("stylize.dropShadow", p)
                    } else {
                        ("stylize.outerGlow", json!({ "blur": blur(shape.sd), "color": color, "opacity": opacity, "mode": mode }))
                    }
                }
                // A drop shadow with Darkness below 100% blurs the object's sRGB values.
                [V::Shade { sd, linear, dx, dy, keep, opacity }, V::Graphic] if !*linear || *keep == 0.0 => {
                    let v = along(*dx, *dy);
                    let (darkness, opacity) = (num((1.0 - *keep as f64) * 100.0), num(*opacity as f64 * 100.0));
                    let p = json!({ "x": num(v.x), "y": num(v.y), "blur": blur(*sd), "shade": "darkness", "darkness": darkness, "opacity": opacity, "mode": mode });
                    ("stylize.dropShadow", p)
                }
                [V::Graphic, V::Paint { shape, color, opacity, clipped: true }] if !shape.moved => {
                    let (color, opacity) = paint(*color, *opacity);
                    let source = if shape.inverted { "edge" } else { "center" };
                    ("stylize.innerGlow", json!({ "blur": blur(shape.sd), "color": color, "opacity": opacity, "mode": mode, "source": source }))
                }
                _ => return None,
            },
            _ => return None,
        };
        out.push(Effect { id: id.into(), params, visible: true });
    }
    Some(out)
}
