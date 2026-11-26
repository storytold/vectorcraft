//! Flare objects: the settings a flare group was drawn with (the Flare tool, `shape.flare`). Its
//! children are ordinary paths for every renderer and exporter; the settings let it be drawn again.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use vectorcraft_color::{BlendMode, Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_geom::{Affine, BezPath, PathData, Point, Rect, Vec2, shapes};

use crate::Appearance;
use crate::node::{Node, NodeId, NodeKind};

/// The seed of the sizes a flare's rays and rings take when its settings give none.
pub const DEFAULT_SEED: u64 = 0x2545_f491_4f6c_dd1d;

/// One part of a flare: its name, outline, fill and opacity.
pub type Part = (&'static str, PathData, Paint, f32);

/// The number `key` of the settings `v`, or `default`.
fn number(v: &Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(default)
}

/// The parts of a flare with the settings `v` (`shape.flare`'s params, each one given): a bright
/// center, a halo ring, rays of varying length and rings along the path to the end point.
pub fn parts(v: &Value) -> Vec<Part> {
    let c = Point::new(number(v, "cx", 0.0), number(v, "cy", 0.0));
    let end = Point::new(number(v, "x2", c.x), number(v, "y2", c.y));
    let pct = |k: &str| (number(v, k, 0.0) / 100.0).clamp(0.0, 10.0);
    let r = (number(v, "diameter", 100.0) / 2.0).max(0.5);
    let opacity = pct("opacity").min(1.0) as f32;
    let brightness = pct("brightness").min(1.0) as f32;
    let growth = pct("growth");
    let fuzz = pct("fuzziness").min(1.0);
    let rays = v.get("rays").and_then(Value::as_u64).unwrap_or(0).min(50) as usize;
    let longest = pct("longest");
    let ray_fuzz = pct("rayFuzziness").min(1.0);
    let ray_turn = number(v, "rotation", 0.0).to_radians();
    let rings = v.get("rings").and_then(Value::as_u64).unwrap_or(0).min(50) as usize;
    let largest = pct("largest");
    // Deterministic pseudo-random sizes (same parameters → same flare). `ringSeed` places the rings
    // along the path from a sequence of its own; their count and sizes and the other parts stay.
    // Xorshift64. Both streams start from a nonzero state.
    let random = |mut state: u64| {
        move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64
        }
    };
    let mut rnd = random(v.get("seed").and_then(Value::as_u64).unwrap_or(DEFAULT_SEED) | 1);
    // Each `ringSeed` below 2^53 (what a JSON number holds exactly) gets a state of its own.
    let mut ring_at = v.get("ringSeed").and_then(Value::as_u64).filter(|r| *r < 1 << 53).map(|r| {
        // SplitMix64 initializes the ring-position stream.
        let mut z = r.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        random(z ^ (z >> 31))
    });
    let stop = |offset: f32, opacity: f32| GradientStop { opacity, ..GradientStop::new(offset, Color::WHITE) };
    let radial = |centre: Point, radius: f64, stops: Vec<GradientStop>| {
        let mut g = GradientPaint::new(Gradient::new(GradientKind::Radial, stops));
        g.geom = Some(GradientGeom { start: centre, end: centre + Vec2::new(radius, 0.0), aspect: 1.0, focal: None });
        Paint::Gradient(Box::new(g))
    };
    let circle = |centre: Point, radius: f64| shapes::ellipse(Rect::from_center_size(centre, (radius * 2.0, radius * 2.0)));
    let mut parts: Vec<Part> = vec![];
    // Halo: a soft ring just outside the centre.
    let halo_r = r * (1.0 + growth) * 1.6;
    let inner = (1.0 - fuzz as f32 * 0.6).clamp(0.05, 0.95);
    parts.push(("Halo", circle(c, halo_r), radial(c, halo_r, vec![stop(0.0, 0.0), stop(inner * 0.75, 0.0), stop(inner, 0.5), stop(1.0, 0.0)]), 1.0));
    // Rays: thin spikes of random length (Longest = % of the centre radius), fading outwards.
    // `rotation` turns them clockwise about the center.
    if rays > 0 {
        let mut bp = BezPath::new();
        let mut reach: f64 = 0.0;
        for i in 0..rays {
            let a = std::f64::consts::TAU * (i as f64 + rnd() * 0.6) / rays as f64 + ray_turn;
            let len = r * longest * (1.0 - ray_fuzz * 0.75 * rnd()).max(0.1);
            reach = reach.max(len);
            let w = r * 0.035;
            let dir = Vec2::new(a.cos(), a.sin());
            let n = Vec2::new(-dir.y, dir.x) * w;
            bp.move_to(c + n);
            bp.line_to(c + dir * len);
            bp.line_to(c - n);
            bp.line_to(c - dir * (w * 4.0));
            bp.close_path();
        }
        parts.push(("Rays", PathData::from_bezpath(&bp), radial(c, reach, vec![stop(0.0, 0.9), stop(0.25, 0.35), stop(1.0, 0.0)]), 1.0));
    }
    // Centre: bright core fading to transparent.
    parts.push(("Center", circle(c, r), radial(c, r, vec![stop(0.0, 1.0), stop(0.25 + brightness * 0.5, 0.6), stop(1.0, 0.0)]), opacity));
    // Rings along the path to the end point (Largest = % of the average ring size).
    for _ in 0..rings {
        let t = rnd();
        let t = ring_at.as_mut().map_or(t, |next| next());
        let at = c + (end - c) * t;
        let rr = (r * largest * (0.25 + rnd() * 0.75)).max(0.5);
        let solid = rnd() < 0.5;
        let stops = if solid {
            vec![stop(0.0, 0.25), stop(0.8, 0.2), stop(1.0, 0.0)]
        } else {
            vec![stop(0.0, 0.0), stop(0.7, 0.05), stop(0.92, 0.35), stop(1.0, 0.0)]
        };
        parts.push(("Ring", circle(at, rr), radial(at, rr, stops), 1.0));
    }
    parts
}

/// Part `part` of a flare as the object `id`, transformed by `xf` (the transform its flare has had
/// since it was drawn), with the pixel-aligned property when `aligned`.
pub fn part_node(id: NodeId, part: Part, xf: Affine, aligned: bool) -> Node {
    let (name, pd, paint, op) = part;
    let mut n = Node::path(id, pd, Appearance::basic(paint, Paint::None, 0.0));
    n.name = Some(name.into());
    n.blend = BlendMode::Screen;
    n.opacity = op;
    if aligned {
        n.extras_mut().pixel_aligned = true;
    }
    if xf != Affine::IDENTITY {
        n.transform(xf, false);
    }
    n
}

/// The ellipse of a radial gradient's placement `geom` as the symmetric matrix `[xx, xy, yy]` that
/// maps it to the unit circle's: the same for every end point and aspect that give that ellipse
/// (transforms applied one by one and all at once can put the end on different axes of it).
fn ellipse(geom: &GradientGeom) -> [f64; 3] {
    let u = geom.end - geom.start;
    let w = Vec2::new(-u.y, u.x) * geom.aspect;
    [u.x * u.x + w.x * w.x, u.x * u.y + w.x * w.y, u.y * u.y + w.y * w.y]
}

/// The ellipse `[xx, xy, yy]` ([`ellipse`]) as its size, the length of its longer half-axis, and
/// its shape: the ratio of its shorter axis to its longer one, then the cosine and sine of twice
/// the longer axis's angle, each times one minus that ratio (0 for a circle). Each number is a
/// length or a ratio of lengths, never a squared length that a tiny flare or a narrow gradient
/// makes smaller than the tolerance.
fn ellipse_size_and_shape(geom: &GradientGeom) -> (f64, [f64; 3]) {
    let [xx, xy, yy] = ellipse(geom);
    let half_diff = (xx - yy) / 2.0;
    let d = half_diff.hypot(xy);
    let longer = (xx + yy) / 2.0 + d;
    if longer.is_nan() || longer <= 0.0 {
        return (0.0, [0.0; 3]);
    }
    // The product of the half-axes is the area factor of the map from the unit circle, `u` across
    // `w` as [`ellipse`] builds them: a sum of squares times the aspect, without the cancellation
    // of `xx * yy - xy * xy` for a narrow ellipse.
    let u = geom.end - geom.start;
    let w = Vec2::new(-u.y, u.x) * geom.aspect;
    let ratio = ((u.x * w.y - u.y * w.x).abs() / longer).min(1.0);
    let (cos, sin) = if d > 0.0 { (half_diff / d, xy / d) } else { (0.0, 0.0) };
    (longer.sqrt(), [ratio, (1.0 - ratio) * cos, (1.0 - ratio) * sin])
}

/// `n` as JSON without what depends on how its transforms were applied: its id, the angle of its
/// bounding box, and which axis of a gradient's ellipse its end point is on (the ellipse stays: its
/// size in the placement, and its shape beside the placement, out of reach of a pixel-aligned
/// part's snap allowance, which is a length).
fn comparable(n: &Node) -> Value {
    fn strip(v: &mut Value) {
        match v {
            Value::Object(m) => {
                for key in ["id", "bbox_angle"] {
                    m.remove(key);
                }
                if let Some(geom) = m.get_mut("geom")
                    && let Ok(g) = serde_json::from_value::<GradientGeom>(geom.clone())
                    && let Value::Object(geom) = geom
                {
                    let (size, shape) = ellipse_size_and_shape(&g);
                    geom.remove("end");
                    geom.remove("aspect");
                    geom.insert("ellipse".into(), serde_json::json!(size));
                    m.insert("ellipseShape".into(), serde_json::json!(shape));
                }
                m.values_mut().for_each(strip);
            }
            Value::Array(a) => a.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut v = serde_json::to_value(n).unwrap_or(Value::Null);
    strip(&mut v);
    v
}

/// How close, relative to its size, a number in an outline or a gradient's placement must be to the
/// one its transforms give: a few thousandths of a point near the edge of the canvas.
const GEOMETRY_TOL: f64 = 1e-9;

/// Whether `a` and `b` are the same JSON, with each number within `tol` relative to its size; each
/// number in an outline (`path`) or a gradient's placement (`geom`) is within [`GEOMETRY_TOL`]
/// relative to its size (or `tol`, when smaller), and `snap` points as well.
fn close(a: &Value, b: &Value, tol: f64, snap: f64, geometry: bool) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => {
                let (snap, tol) = if geometry { (snap, tol.min(GEOMETRY_TOL)) } else { (0.0, tol) };
                (x - y).abs() <= snap + tol * (1.0 + x.abs().max(y.abs()))
            }
            _ => x == y,
        },
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(x, y)| close(x, y, tol, snap, geometry)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter().all(|(k, v)| {
                    // A gradient's shape is ratios: compared as closely as geometry, without the snap.
                    let snap = if k == "ellipseShape" { 0.0 } else { snap };
                    y.get(k).is_some_and(|w| close(v, w, tol, snap, geometry || k == "path" || k == "geom" || k == "ellipseShape"))
                })
        }
        _ => a == b,
    }
}

/// Whether `a` and `b` are the same JSON, with each number within `tol` relative to its size.
pub fn json_close(a: &Value, b: &Value, tol: f64) -> bool {
    close(a, b, tol, 0.0, false)
}

/// How far a pixel-aligned object's points may be from where its transforms put them: snapping to
/// the pixel grid moves each by up to a pixel.
const PIXEL_SNAP: f64 = 1.0;

/// The settings of a flare group ([`crate::NodeExtras::flare`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlareSpec {
    /// `shape.flare`'s params the flare was drawn with, each one given: the center (`cx`, `cy`) and
    /// the rings' end point (`x2`, `y2`) are in the coordinates it was drawn in.
    pub params: serde_json::Value,
    /// The transform its art has had since it was drawn (moves, scales, rotations, reflections and
    /// shears).
    #[serde(default = "identity", skip_serializing_if = "is_identity")]
    pub transform: Affine,
}

fn identity() -> Affine {
    Affine::IDENTITY
}

fn is_identity(a: &Affine) -> bool {
    *a == Affine::IDENTITY
}

impl FlareSpec {
    /// The settings of a flare drawn with `params`, not transformed since.
    pub fn new(params: serde_json::Value) -> Self {
        Self { params, transform: Affine::IDENTITY }
    }

    /// The point `key_x`, `key_y` of the params, where it is in the document now.
    fn point(&self, key_x: &str, key_y: &str) -> Option<Point> {
        let x = self.params.get(key_x)?.as_f64()?;
        let y = self.params.get(key_y)?.as_f64()?;
        Some(self.transform * Point::new(x, y)).filter(|p| p.x.is_finite() && p.y.is_finite())
    }

    /// The flare's center, where it is in the document now.
    pub fn center(&self) -> Option<Point> {
        self.point("cx", "cy")
    }

    /// The rings' end point, where it is in the document now.
    pub fn end(&self) -> Option<Point> {
        self.point("x2", "y2")
    }

    /// Its art transformed by `a`.
    pub fn transform(&mut self, a: Affine) {
        self.transform = a * self.transform;
    }

    /// Whether `group`'s parts are still the ones these settings draw, in place: none was edited,
    /// distorted, removed or added, and the transform can be undone. The parts are compared where
    /// they are in the document, at any scale.
    pub fn draws(&self, group: &Node) -> bool {
        let NodeKind::Group { children, .. } = &group.kind else { return false };
        if !self.transform.is_finite() || self.transform.determinant() == 0.0 || !self.transform.inverse().is_finite() {
            return false;
        }
        let expected = parts(&self.params);
        expected.len() == children.len()
            && expected.into_iter().zip(children).all(|(part, child)| {
                let aligned = child.extras().pixel_aligned;
                let snap = if aligned { PIXEL_SNAP } else { 0.0 };
                close(&comparable(&part_node(child.id, part, self.transform, aligned)), &comparable(child), 1e-6, snap, false)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A flare's center and end point follow its art through a transform, and a save and reload
    /// keeps its settings.
    #[test]
    fn a_flare_keeps_its_settings_through_a_transform_and_a_save() {
        let mut g = Node::group(NodeId(1), vec![std::sync::Arc::new(Node::path(NodeId(2), PathData::default(), Default::default()))]);
        g.extras_mut().flare = Some(Box::new(FlareSpec::new(json!({ "cx": 10.0, "cy": 20.0, "x2": 110.0, "y2": 20.0, "seed": 7 }))));
        g.transform(Affine::translate((5.0, 0.0)) * Affine::rotate(std::f64::consts::FRAC_PI_2), false);
        let spec = g.extras().flare.as_deref().cloned().unwrap_or_else(|| panic!("no flare"));
        let c = spec.center().unwrap_or_else(|| panic!("no center"));
        let e = spec.end().unwrap_or_else(|| panic!("no end"));
        assert!((c.x - -15.0).abs() < 1e-9 && (c.y - 10.0).abs() < 1e-9, "{c:?}");
        assert!((e.x - -15.0).abs() < 1e-9 && (e.y - 110.0).abs() < 1e-9, "{e:?}");
        let text = serde_json::to_string(&g).unwrap_or_else(|e| panic!("{e}"));
        let back: Node = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back.extras().flare.as_deref(), Some(&spec));
        // A flare that was not transformed writes no transform.
        let plain = FlareSpec::new(json!({ "cx": 1.0 }));
        assert_eq!(serde_json::to_value(&plain).unwrap_or_default(), json!({ "params": { "cx": 1.0 } }));
    }

    /// A flare group draws its settings until a part's gradient, opacity or outline is edited; a
    /// pixel-aligned part may sit up to a pixel from where its transforms put it, but only in its
    /// outline and its gradient's placement.
    #[test]
    fn a_flare_with_an_edited_part_no_longer_draws_its_settings() {
        let v = json!({ "cx": 50.0, "cy": 50.0, "x2": 250.0, "y2": 50.0, "rays": 5, "rings": 2, "seed": 9, "diameter": 40.0,
            "opacity": 50.0, "brightness": 30.0, "growth": 20.0, "fuzziness": 50.0, "longest": 300.0, "rayFuzziness": 100.0,
            "rotation": 0.0, "largest": 50.0 });
        let flare = |aligned: bool| {
            let xf = Affine::rotate(0.3) * Affine::scale_non_uniform(1.5, 0.75);
            let children = parts(&v).into_iter().enumerate().map(|(i, p)| std::sync::Arc::new(part_node(NodeId(10 + i as u64), p, xf, aligned)));
            let mut g = Node::group(NodeId(1), children.collect());
            g.extras_mut().flare = Some(Box::new(FlareSpec { params: v.clone(), transform: xf }));
            g
        };
        let spec = |g: &Node| g.extras().flare.as_deref().cloned().unwrap_or_else(|| panic!("no flare"));
        let edit = |g: &mut Node, f: &dyn Fn(&mut Node)| {
            let NodeKind::Group { children, .. } = &mut g.kind else { panic!("not a group") };
            // The center, drawn at half opacity.
            f(std::sync::Arc::make_mut(&mut children[2]));
        };
        for aligned in [false, true] {
            let g = flare(aligned);
            assert!(spec(&g).draws(&g), "aligned {aligned}");
            let mut moved = g.clone();
            edit(&mut moved, &|n| n.transform(Affine::translate((0.4, 0.0)), false));
            assert_eq!(spec(&moved).draws(&moved), aligned, "a fraction of a pixel, aligned {aligned}");
            let mut faded = g.clone();
            edit(&mut faded, &|n| n.opacity *= 0.8);
            assert!(!spec(&faded).draws(&faded), "opacity, aligned {aligned}");
            let mut shifted = g.clone();
            edit(&mut shifted, &|n| {
                if let Some(crate::AppearanceItem::Fill(fill)) = n.appearance.items.first_mut()
                    && let Paint::Gradient(gp) = &mut fill.paint
                    && let Some(geom) = &mut gp.geom
                {
                    geom.start.x += 20.0;
                }
            });
            assert!(!spec(&shifted).draws(&shifted), "gradient placement, aligned {aligned}");
            let mut squashed = g.clone();
            edit(&mut squashed, &|n| {
                if let Some(crate::AppearanceItem::Fill(fill)) = n.appearance.items.first_mut()
                    && let Paint::Gradient(gp) = &mut fill.paint
                    && let Some(geom) = &mut gp.geom
                {
                    geom.aspect *= 0.5;
                }
            });
            assert!(!spec(&squashed).draws(&squashed), "gradient shape, aligned {aligned}");
        }
    }

    /// An ellipse's size and shape are the same whichever axis its end point is on, and a narrow
    /// gradient made narrower, or turned, changes its shape by more than [`GEOMETRY_TOL`].
    #[test]
    fn an_ellipse_shape_tells_narrow_gradients_apart() {
        let geom = |end: (f64, f64), aspect: f64| GradientGeom { start: Point::new(1.0, 2.0), end: Point::new(end.0, end.1), aspect, focal: None };
        let of = ellipse_size_and_shape;
        let (size, shape) = of(&geom((11.0, 2.0), 0.5));
        // The same ellipse with its end point on the other axis.
        let (size2, shape2) = of(&geom((1.0, 7.0), 2.0));
        assert!((size - size2).abs() < 1e-12 && shape.iter().zip(shape2).all(|(a, b)| (a - b).abs() < 1e-12), "{shape:?} {shape2:?}");
        // A very narrow ellipse, given both ways, keeps its shape to within the tolerance.
        let (thin, thin2) = (of(&geom((11.0, 2.0), 1e-8)).1, of(&geom((1.0, 2.0 + 1e-7), 1e8)).1);
        assert!(thin.iter().zip(thin2).all(|(a, b)| (a - b).abs() < GEOMETRY_TOL), "{thin:?} {thin2:?}");
        let narrow = of(&geom((11.0, 2.0), 1e-4)).1;
        for other in [of(&geom((11.0, 2.0), 5e-5)).1, of(&geom((1.0, 12.0), 1e-4)).1] {
            assert!(narrow.iter().zip(other).any(|(a, b)| (a - b).abs() > 1e3 * GEOMETRY_TOL), "{narrow:?} {other:?}");
        }
        assert_eq!(of(&geom((1.0, 2.0), 1.0)), (0.0, [0.0; 3]));
    }

    /// A flare scaled far down, in one direction or in all, anywhere on the canvas, still draws its
    /// settings, and a part's gradient whose shape was edited is still told apart; a flattened flare
    /// does not draw its settings.
    #[test]
    fn a_flare_scaled_far_down_still_draws_its_settings() {
        let v = json!({ "cx": 50.0, "cy": 50.0, "x2": 250.0, "y2": 50.0, "rays": 5, "rings": 2, "seed": 9, "diameter": 40.0 });
        let flare_aligned = |xf: Affine, aligned: bool| {
            let children = parts(&v).into_iter().enumerate().map(|(i, p)| std::sync::Arc::new(part_node(NodeId(10 + i as u64), p, xf, aligned)));
            let mut g = Node::group(NodeId(1), children.collect());
            g.extras_mut().flare = Some(Box::new(FlareSpec { params: v.clone(), transform: xf }));
            g
        };
        let flare = |xf: Affine| flare_aligned(xf, false);
        // The center's gradient placement, to edit.
        let center_geom = |g: &mut Node, f: &dyn Fn(&mut GradientGeom)| {
            let NodeKind::Group { children, .. } = &mut g.kind else { panic!("not a group") };
            if let Some(crate::AppearanceItem::Fill(fill)) = std::sync::Arc::make_mut(&mut children[2]).appearance.items.first_mut()
                && let Paint::Gradient(gp) = &mut fill.paint
                && let Some(geom) = &mut gp.geom
            {
                f(geom);
            } else {
                panic!("the center has no gradient fill");
            }
        };
        let draws = |g: &Node| g.extras().flare.as_deref().is_some_and(|spec| spec.draws(g));
        assert!(draws(&flare(Affine::scale(1e-7))), "scaled to 0.00001%");
        assert!(draws(&flare(Affine::rotate(0.3) * Affine::scale_non_uniform(1.0, 1e-10))), "shrunk far on one axis");
        assert!(draws(&flare(Affine::translate((crate::MAX_COORD - 1000.0, 0.0)) * Affine::scale(0.5))), "near the edge of the canvas");
        assert!(draws(&flare(Affine::translate((1000.0, 0.0)) * Affine::scale(1e-5))), "scaled to 0.001% and moved away");
        assert!(!draws(&flare(Affine::scale_non_uniform(1.0, 0.0))), "flattened");
        // At that size a part whose gradient's shape was edited no longer draws the settings.
        let mut squashed = flare(Affine::scale(1e-6));
        assert!(draws(&squashed), "scaled to 0.0001%");
        center_geom(&mut squashed, &|geom| geom.aspect *= 0.5);
        assert!(!draws(&squashed), "gradient shape at 0.0001%");
        // A pixel-aligned part may sit up to a pixel off, but its gradient's shape may not change,
        // even when the ellipse keeps its size.
        let mut reshaped = flare_aligned(Affine::IDENTITY, true);
        assert!(draws(&reshaped), "aligned");
        center_geom(&mut reshaped, &|geom| {
            let a = geom.aspect;
            geom.aspect = a * 0.5;
            geom.end = geom.start + (geom.end - geom.start) * ((1.0 + a * a) / (1.0 + a * a / 4.0)).sqrt();
        });
        assert!(!draws(&reshaped), "gradient shape of the same size, aligned");
    }
}
