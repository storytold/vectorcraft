//! Colours and gradients of the editing data.
//!
//! - Colours: `g`/`G` grey (1 is white), `k`/`K` CMYK, `x`/`X` a named CMYK colour at a tint
//!   (`c m y k (name) tint`; tint 0 is the full colour), `Xa`/`XA` RGB (`c m y k r g b`, the CMYK
//!   values its equivalent), `Xx`/`XX` a named colour of any model
//!   (`c m y k [r g b | L a b] (name) tint type`, type 0 CMYK, 1 RGB, 2 Lab), `Xk`/`XK` a global
//!   process colour (its operands as `Xx`'s: a global swatch that isn't an ink of its own),
//!   `Xs`/`XS` (`c m y k (name) tint`) and `Xz`/`XZ` (`c m y k r g b (name) tint type`) the
//!   Registration colour at a tint, as written in files users sent. Lower case paints fills, upper
//!   case strokes.
//! - Gradient definitions: `(name) type stops Bd`, its stops as `colour style [opacity 6]
//!   midpoint ramp Bs` (style 0 grey, 1 CMYK, 2 RGB with its CMYK, 3 and 4 named colours), `BD`.
//! - A gradient on an object: `[flag] Bb` (1: the stroke), `flag (name) x y angle length a b c d
//!   tx ty Bg`, the focal point `x y angle length Bh`, `flag BB`. Newer files write the gradient's
//!   place as matrices after `Bg` (whose own numbers are then not in art space): `a b c d tx ty Xm`
//!   takes the unit vector (a radial's unit circle) onto the gradient's whole ramp (0 to 100, its
//!   angle and length in it), and `a b c d tx ty Bm` takes it onto the part between two stops next to each other, one
//!   `Bm` for each pair in the order of the ramp (`Bc`, around them, are the caps past the first and
//!   last stops). Without `Xm`, a radial gradient's one `Bm` takes the unit circle onto it.
//!
//! "AI7 spec": Adobe Illustrator File Format Specification, version 7.0 (1998), listed by PRONOM at
//! https://www.nationalarchives.gov.uk/PRONOM/fmt/423; §3.5 documents the gradient operators.

use std::collections::HashMap;

use kurbo::{Affine, Point, Vec2};
use vectorcraft_color::gradient::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
use vectorcraft_color::{Color, Paint};

use super::obj::V;

/// Largest number of stops read in one gradient.
const MAX_STOPS: usize = 256;

/// A colour read with the swatch it names (spot and global colours).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Named {
    pub name: String,
    /// The full colour.
    pub color: Color,
    pub tint: f32,
    /// An ink of its own (a spot colour), not a global process colour.
    pub spot: bool,
}

/// The paint of a colour operator's operands (`op`: its name, lower case or not), with the named
/// colour it uses. `None` when the operands don't fit.
pub(super) fn color_op(op: &str, vals: &[V]) -> Option<(Paint, Option<Named>)> {
    let nums: Vec<f32> = vals.iter().filter_map(V::num).map(|v| v as f32).collect();
    let unit = |v: f32| v.clamp(0.0, 1.0);
    let last = |n: usize| nums.get(nums.len().checked_sub(n)?..);
    match op.to_ascii_lowercase().as_str() {
        "g" => Some((Paint::solid(Color::gray(1.0 - unit(*nums.last()?))), None)),
        "k" => {
            let [c, m, y, k] = last(4)? else { return None };
            Some((Paint::solid(Color::cmyk(unit(*c), unit(*m), unit(*y), unit(*k))), None))
        }
        "xa" => {
            let [r, g, b] = last(3)? else { return None };
            Some((Paint::solid(Color::rgb(unit(*r), unit(*g), unit(*b))), None))
        }
        "x" => named(vals, Some(false)),
        "xx" => named(vals, None),
        "xk" => named(vals, None).map(|(p, n)| (p, n.map(|n| Named { spot: false, ..n }))),
        "xs" | "xz" => registration(vals),
        _ => None,
    }
}

/// `c m y k (name) tint x` or `c m y k [r g b | L a b] (name) tint type Xx` (type 1: RGB, 2: Lab);
/// `rgb` forces RGB or CMYK.
fn named(vals: &[V], rgb: Option<bool>) -> Option<(Paint, Option<Named>)> {
    let at = vals.iter().rposition(|v| matches!(v, V::Str(_)))?;
    let name = vals.get(at)?.text()?;
    let before: Vec<f32> = vals.get(..at)?.iter().filter_map(V::num).map(|v| v as f32).collect();
    let after: Vec<f32> = vals.get(at + 1..)?.iter().filter_map(V::num).map(|v| v as f32).collect();
    let unit = |v: f32| v.clamp(0.0, 1.0);
    let tint = 1.0 - unit(*after.first()?);
    let n = before.len();
    let kind = after.get(1).copied();
    let color = if rgb.unwrap_or(kind == Some(1.0)) {
        let [r, g, b] = before.get(n.checked_sub(3)?..)? else { return None };
        Color::rgb(unit(*r), unit(*g), unit(*b))
    } else if rgb.is_none() && kind == Some(2.0) {
        // Type 2: a colour defined in Lab, its L a b values after the CMYK equivalent (#1032).
        let [l, a, b] = before.get(n.checked_sub(3)?..)? else { return None };
        Color::lab(l.clamp(0.0, 100.0), a.clamp(-128.0, 127.0), b.clamp(-128.0, 127.0))
    } else {
        // The CMYK values come first (an RGB equivalent may follow them).
        let from = if n >= 7 { n - 7 } else { n.checked_sub(4)? };
        let [c, m, y, k] = before.get(from..from + 4)? else { return None };
        Color::cmyk(unit(*c), unit(*m), unit(*y), unit(*k))
    };
    let paint = Paint::Solid { color: color.tinted(tint), swatch: Some(name.clone()), tint };
    Some((paint, Some(Named { name, color, tint, spot: true })))
}

/// `… (name) tint Xs` / `… (name) tint type Xz`: the built-in Registration swatch at the tint (its
/// own colour, whatever values the operands give).
fn registration(vals: &[V]) -> Option<(Paint, Option<Named>)> {
    let at = vals.iter().rposition(|v| matches!(v, V::Str(_)))?;
    let tint = 1.0 - vals.get(at + 1).and_then(V::num).unwrap_or(0.0).clamp(0.0, 1.0) as f32;
    let full = Color::cmyk(1.0, 1.0, 1.0, 1.0);
    Some((Paint::Solid { color: full.tinted(tint), swatch: Some(vectorcraft_color::swatch::REGISTRATION.into()), tint }, None))
}

/// The colour of a gradient stop's colour values and style.
fn stop_color(style: f64, comps: &[V]) -> Option<(Color, Option<Named>)> {
    let nums: Vec<f32> = comps.iter().filter_map(V::num).map(|v| v as f32).collect();
    let unit = |v: f32| v.clamp(0.0, 1.0);
    let tail = |n: usize| nums.get(nums.len().checked_sub(n)?..);
    Some(match style as i64 {
        0 => (Color::gray(1.0 - unit(*nums.last()?)), None),
        1 => {
            let [c, m, y, k] = tail(4)? else { return None };
            (Color::cmyk(unit(*c), unit(*m), unit(*y), unit(*k)), None)
        }
        2 => {
            let [r, g, b] = tail(3)? else { return None };
            (Color::rgb(unit(*r), unit(*g), unit(*b)), None)
        }
        _ => {
            let (paint, named) = named(comps, Some(style as i64 == 4))?;
            (paint.color()?, named)
        }
    })
}

/// A gradient definition being read.
#[derive(Debug)]
pub(super) struct GradientDef {
    pub name: String,
    pub gradient: Gradient,
}

impl GradientDef {
    /// `(name) type stops Bd`.
    pub fn begin(vals: &[V]) -> Option<Self> {
        let name = vals.iter().find_map(|v| if let V::Str(_) = v { v.text() } else { None })?;
        let nums: Vec<f64> = vals.iter().filter_map(V::num).collect();
        let kind = if nums.first().copied() == Some(1.0) { GradientKind::Radial } else { GradientKind::Linear };
        Some(Self { name, gradient: Gradient::new(kind, vec![]) })
    }

    /// A stop (`Bs`).
    pub fn stop(&mut self, vals: &[V], spots: &mut Vec<Named>) {
        if self.gradient.stops.len() >= MAX_STOPS {
            return;
        }
        let n = vals.len();
        let num = |i: usize| vals.get(i).and_then(V::num);
        let (Some(ramp), Some(mid)) = (n.checked_sub(1).and_then(num), n.checked_sub(2).and_then(num)) else { return };
        // Newer stops have an opacity and the marker 6 before the midpoint.
        let newer = n >= 5 && n.checked_sub(3).and_then(num) == Some(6.0);
        let (style_at, opacity) = if newer { (n.checked_sub(5), n.checked_sub(4).and_then(num).unwrap_or(1.0)) } else { (n.checked_sub(3), 1.0) };
        let (Some(style_at), Some(style)) = (style_at, style_at.and_then(num)) else { return };
        let Some((color, named)) = stop_color(style, vals.get(..style_at).unwrap_or_default()) else { return };
        let mut stop = GradientStop::new((ramp / 100.0).clamp(0.0, 1.0) as f32, color);
        stop.opacity = opacity.clamp(0.0, 1.0) as f32;
        stop.midpoint = (mid / 100.0).clamp(0.13, 0.87) as f32;
        if let Some(n) = named {
            stop.set_color(n.color.tinted(n.tint), Some((n.name.clone(), n.tint)));
            spots.push(n);
        }
        self.gradient.stops.push(stop);
    }

    /// The finished gradient (`BD`): stops in order of their ramp points.
    pub fn finish(mut self) -> Option<(String, Gradient)> {
        if self.gradient.stops.is_empty() {
            return None;
        }
        // Stops are listed along the ramp from either end: listed downwards they're reversed, so
        // stops sharing a ramp point keep their order (and each its midpoint, to the next stop).
        if self.gradient.stops.first().zip(self.gradient.stops.last()).is_some_and(|(first, last)| first.offset > last.offset) {
            self.gradient.stops.reverse();
        }
        self.gradient.stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        Some((self.name, self.gradient))
    }
}

/// A gradient instance on an object, between `Bb` and `BB`.
#[derive(Clone, Debug, Default)]
pub(super) struct Instance {
    pub stroke: bool,
    pub name: Option<String>,
    origin: Point,
    angle: f64,
    length: f64,
    /// `Bg`'s own matrix.
    bg: Affine,
    /// `Xm`: "the overall matrix applied to the gradient" (AI7 spec §3.5.4), when there is one.
    xm: Option<Affine>,
    /// The first and the last gradient matrix (`Bm`), when there are any. After `Xm` they are
    /// imaging bands (AI7 spec §3.5.6), not the gradient's matrix.
    bm: Option<(Affine, Affine)>,
    /// The focal point's offset from the origin (`Bh`).
    hilight: Vec2,
}

pub(super) fn affine(n: &[f64]) -> Option<Affine> {
    let [a, b, c, d, e, f] = n else { return None };
    let m = Affine::new([*a, *b, *c, *d, *e, *f]);
    (m.as_coeffs().iter().all(|v| v.is_finite())).then_some(m)
}

impl Instance {
    pub fn begin(vals: &[V]) -> Self {
        let stroke = vals.iter().filter_map(V::num).next_back() == Some(1.0);
        Self { stroke, length: 1.0, ..Self::default() }
    }

    /// `flag (name) x y angle length a b c d tx ty Bg`.
    pub fn bg(&mut self, vals: &[V]) {
        let Some(at) = vals.iter().rposition(|v| matches!(v, V::Str(_))) else { return };
        self.name = vals.get(at).and_then(V::text);
        let n: Vec<f64> = vals.get(at + 1..).unwrap_or_default().iter().filter_map(V::num).collect();
        if let [x, y, angle, length, m @ ..] = n.as_slice() {
            self.origin = Point::new(*x, *y);
            self.angle = *angle;
            self.length = *length;
            self.bg = affine(m).unwrap_or(Affine::IDENTITY);
        }
    }

    /// The matrix of `a b c d tx ty Bm` or `Xm`, when it can be inverted.
    fn matrix(vals: &[V]) -> Option<Affine> {
        let n: Vec<f64> = vals.iter().filter_map(V::num).collect();
        n.get(n.len().saturating_sub(6)..).and_then(affine).filter(|m| m.determinant().abs() > 1e-12)
    }

    /// `a b c d tx ty Bm`: one for each pair of stops next to each other.
    pub fn bm(&mut self, vals: &[V]) {
        if let Some(m) = Self::matrix(vals) {
            self.bm = Some((self.bm.map_or(m, |(first, _)| first), m));
        }
    }

    /// `a b c d tx ty Xm`: "the overall matrix applied to the gradient" (AI7 spec §3.5.4).
    pub fn xm(&mut self, vals: &[V]) {
        self.xm = Self::matrix(vals);
    }

    /// `x y angle length Bh`. The point is `--` (undefined) when the gradient has no highlight.
    pub fn bh(&mut self, vals: &[V]) {
        let n: Vec<f64> = vals.iter().filter_map(V::num).collect();
        if let [x, y, ..] = n.as_slice()
            && x.is_finite()
            && y.is_finite()
        {
            self.hilight = Vec2::new(*x, *y);
        }
    }

    /// Does it carry a matrix of its own (`Xm` or `Bm`) rather than only `Bg`'s numbers?
    pub fn has_matrix(&self) -> bool {
        self.xm.is_some() || self.bm.is_some()
    }

    /// The paint, through `to_doc` (art space → the document), when the gradient is defined.
    pub fn paint(&self, defs: &HashMap<String, Gradient>, to_doc: Affine) -> Option<Paint> {
        let name = self.name.as_ref()?;
        let gradient = defs.get(name)?.clone();
        let radial = gradient.kind == GradientKind::Radial;
        // `Xm` maps the unit circle (or vector) to the art, `Bg`'s origin, angle and length already
        // in it, and so does a radial's `Bm`: files whose PDF part draws the radial draw it through
        // that `Bm` alone, whatever `Bg` holds. The highlight is an offset from the centre in art space.
        let unit = self.xm.or(self.bm.filter(|_| radial).map(|(first, _)| first));
        // A skewed matrix leaves `along` and `across` oblique: a linear ramp runs along the normal to
        // its isolines (`across`), and a radial's ellipse is set by the matrix's principal axes.
        let through = |m: Affine, origin: Point, dir: Vec2| {
            let start = m * origin;
            let along = m * (origin + dir) - start;
            let across = m * (origin + Vec2::new(-dir.y, dir.x)) - start;
            if radial {
                let (major, minor) = principal_axes(along, across);
                (start, start + major, minor.hypot() / major.hypot())
            } else {
                let normal = Vec2::new(-across.y, across.x).normalize();
                (start, start + normal * along.dot(normal), 1.0)
            }
        };
        let (start, end, aspect, focal) = match (unit, self.bm) {
            (Some(unit), _) => {
                let (start, end, aspect) = through(to_doc * unit, Point::ORIGIN, Vec2::new(1.0, 0.0));
                (start, end, aspect, to_doc * (unit * Point::ORIGIN + self.hilight))
            }
            // A linear gradient's `Bm`s alone: they run from its first stop to its last.
            (None, Some((first, last))) => {
                let (a, b) = (to_doc * first * Point::ORIGIN, to_doc * last * Point::new(1.0, 0.0));
                let from = gradient.stops.first().map_or(0.0, |s| f64::from(s.offset));
                let to = gradient.stops.last().map_or(1.0, |s| f64::from(s.offset));
                let whole = (b - a) / (to - from).max(1e-6);
                let start = a - whole * from;
                (start, start + whole, 1.0, start)
            }
            // `Bg`'s own numbers, in art space through its matrix.
            (None, None) => {
                let m = to_doc * self.bg;
                let a = self.angle.to_radians();
                let (start, end, aspect) = through(m, self.origin, Vec2::new(a.cos(), a.sin()) * self.length);
                (start, end, aspect, m * (self.origin + self.hilight))
            }
        };
        if !(start.is_finite() && end.is_finite()) || start.distance(end) < 1e-9 {
            return None;
        }
        let aspect = aspect.clamp(0.01, 100.0);
        let focal = (self.hilight.hypot() > 1e-9).then_some(focal).filter(|p| p.is_finite());
        let geom = GradientGeom { start, end, aspect, focal: if radial { focal } else { None } };
        let angle = -(end - start).atan2().to_degrees();
        Some(Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), angle, swatch: Some(name.clone()), ..GradientPaint::new(gradient) })))
    }
}

/// The semi-axes of the ellipse the unit circle maps to under the linear map with columns `u` and `v`,
/// the major one first.
fn principal_axes(u: Vec2, v: Vec2) -> (Vec2, Vec2) {
    let (xx, yy, xy) = (u.x * u.x + v.x * v.x, u.y * u.y + v.y * v.y, u.x * u.y + v.x * v.y);
    let angle = 0.5 * (2.0 * xy).atan2(xx - yy);
    let spread = (0.25 * (xx - yy).powi(2) + xy * xy).sqrt();
    let (major, minor) = ((0.5 * (xx + yy) + spread).sqrt(), (0.5 * (xx + yy) - spread).max(0.0).sqrt());
    let axis = Vec2::new(angle.cos(), angle.sin());
    (axis * major, Vec2::new(-axis.y, axis.x) * minor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nums(v: &[f64]) -> Vec<V> {
        v.iter().map(|n| V::Num(*n)).collect()
    }

    #[test]
    fn registration_paints_with_the_registration_swatch_at_its_tint() {
        let mut v = nums(&[0.75, 0.68, 0.67, 0.9, 0.0, 0.0, 0.0]);
        v.extend([V::Str(b"[Registration]".to_vec()), V::Num(0.1), V::Num(1.0)]);
        let (p, named) = color_op("XZ", &v).unwrap();
        assert!(named.is_none());
        let Paint::Solid { swatch, tint, color } = p else { panic!() };
        assert_eq!((swatch.as_deref(), color), (Some("[Registration]"), Color::cmyk(1.0, 1.0, 1.0, 1.0).tinted(0.9)));
        assert!((tint - 0.9).abs() < 1e-6);
        let mut v = nums(&[1.0, 1.0, 1.0, 1.0]);
        v.extend([V::Str(b"[Registration]".to_vec()), V::Num(0.0)]);
        assert!(matches!(color_op("Xs", &v), Some((Paint::Solid { tint, .. }, None)) if tint == 1.0));
    }

    #[test]
    fn colours() {
        let (p, _) = color_op("g", &nums(&[0.7])).unwrap();
        assert!(matches!(p.color(), Some(Color::Gray { k }) if (k - 0.3).abs() < 1e-6));
        let (p, _) = color_op("XA", &nums(&[0.0, 0.99, 1.0, 0.0, 1.0, 0.0, 0.0])).unwrap();
        assert_eq!(p.color(), Some(Color::rgb(1.0, 0.0, 0.0)));
        let mut v = nums(&[0.0, 0.5, 1.0, 0.0]);
        v.push(V::Str(b"MySpot".to_vec()));
        v.push(V::Num(0.4));
        let (p, named) = color_op("x", &v).unwrap();
        let named = named.unwrap();
        assert_eq!(named.name, "MySpot");
        assert!((named.tint - 0.6).abs() < 1e-6);
        assert!(matches!(p, Paint::Solid { swatch: Some(_), .. }));
        assert!(color_op("k", &nums(&[1.0])).is_none());
        // A global process colour: CMYK (type 0) or RGB (type 1), not an ink.
        let mut v = nums(&[0.0, 0.0, 0.0, 1.0]);
        v.extend([V::Str(b"K100".to_vec()), V::Num(0.4), V::Num(0.0)]);
        let (p, named) = color_op("XK", &v).unwrap();
        let named = named.unwrap();
        assert_eq!((named.name.as_str(), named.color, named.spot), ("K100", Color::cmyk(0.0, 0.0, 0.0, 1.0), false));
        assert!((named.tint - 0.6).abs() < 1e-6);
        assert!(
            matches!(p, Paint::Solid { swatch: Some(s), tint, color: Color::Cmyk { k, .. } } if s == "K100" && (tint - 0.6).abs() < 1e-6 && (k - 0.6).abs() < 1e-6)
        );
        let mut v = nums(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.5, 0.0]);
        v.extend([V::Str(b"Orange".to_vec()), V::Num(0.0), V::Num(1.0)]);
        let (p, _) = color_op("Xk", &v).unwrap();
        assert_eq!(p.color(), Some(Color::rgb(1.0, 0.5, 0.0)));
        // A spot colour defined in Lab (type 2): its L a b values, not its CMYK equivalent (#1032).
        let mut v = nums(&[0.52, 0.98, 0.28, 0.1, 32.9412, 45.0, -18.0]);
        v.extend([V::Str(b"Plum Ink".to_vec()), V::Num(0.0), V::Num(2.0)]);
        let (p, named) = color_op("Xx", &v).unwrap();
        let named = named.unwrap();
        assert_eq!((named.color, named.spot, named.tint), (Color::lab(32.9412, 45.0, -18.0), true, 1.0));
        assert_eq!(p.color(), Some(Color::lab(32.9412, 45.0, -18.0)));
        v[8] = V::Num(0.25);
        let (_, named) = color_op("XX", &v).unwrap();
        assert_eq!(named.map(|n| (n.color, n.tint)), Some((Color::lab(32.9412, 45.0, -18.0), 0.75)));
    }

    #[test]
    fn gradient_definition_and_instance() {
        let mut d = GradientDef::begin(&[V::Str(b"G".to_vec()), V::Num(1.0), V::Num(2.0)]).unwrap();
        let mut spots = vec![];
        d.stop(&nums(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.5, 6.0, 30.0, 100.0]), &mut spots);
        d.stop(&nums(&[0.81, 0.0, 1.0, 6.0, 50.0, 0.0]), &mut spots);
        let (name, g) = d.finish().unwrap();
        assert_eq!(name, "G");
        assert_eq!(g.kind, GradientKind::Radial);
        assert_eq!(g.stops.len(), 2);
        assert_eq!(g.stops[1].opacity, 0.5);
        assert!(matches!(g.stops[0].color, Color::Gray { k } if (k - 0.19).abs() < 1e-6));
        let defs = HashMap::from([(name, g)]);
        let mut i = Instance::begin(&[]);
        let mut bg = vec![V::Num(1.0), V::Str(b"G".to_vec())];
        bg.extend(nums(&[100.0, 100.0, 0.0, 50.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
        i.bg(&bg);
        i.bm(&nums(&[50.0, 0.0, 0.0, -50.0, 100.0, 100.0]));
        let Some(Paint::Gradient(gp)) = i.paint(&defs, Affine::IDENTITY) else { panic!() };
        let geom = gp.geom.unwrap();
        assert_eq!(geom.start, Point::new(100.0, 100.0));
        assert_eq!(geom.end, Point::new(150.0, 100.0));
        assert!((geom.aspect - 1.0).abs() < 1e-9);
        assert!(i.paint(&HashMap::new(), Affine::IDENTITY).is_none());
        // Newer files give the matrix with `Xm`; the `Bm`s after it are bands of a cached ramp.
        let mut i = Instance::begin(&[]);
        i.bg(&bg);
        i.xm(&nums(&[40.0, 0.0, 0.0, -40.0, 10.0, 10.0]));
        i.bm(&nums(&[5.0, 0.0, 0.0, -40.0, 30.0, 10.0]));
        let Some(Paint::Gradient(gp)) = i.paint(&defs, Affine::IDENTITY) else { panic!() };
        let geom = gp.geom.unwrap();
        assert_eq!((geom.start, geom.end), (Point::new(10.0, 10.0), Point::new(50.0, 10.0)));
    }

    #[test]
    fn gradient_stops_beyond_the_cap_are_ignored() {
        let mut d = GradientDef::begin(&[V::Str(b"H".to_vec()), V::Num(0.0)]).unwrap();
        let mut spots = vec![];
        for i in 0..257 {
            let ramp = i as f64 / 256.0 * 100.0;
            d.stop(&nums(&[0.2, 0.4, 0.6, 0.0, 1.0, 1.0, 6.0, 50.0, ramp]), &mut spots);
        }
        let (_, g) = d.finish().unwrap();
        assert_eq!(g.stops.len(), MAX_STOPS);
    }

    /// A linear gradient of three stops as newer files place it: `Bg` with numbers that aren't in
    /// art space, `Xm` for the whole ramp and a `Bm` for each pair of stops.
    #[test]
    fn a_linear_gradient_runs_its_whole_ramp_not_its_last_part() {
        let mut d = GradientDef::begin(&[V::Str(b"L".to_vec()), V::Num(0.0), V::Num(3.0)]).unwrap();
        for ramp in [100.0, 60.0, 20.0] {
            d.stop(&nums(&[0.2, 0.4, 0.6, 0.0, 1.0, 1.0, 6.0, 50.0, ramp]), &mut vec![]);
        }
        let defs = HashMap::from([d.finish().unwrap()]);
        let instance = |xm: bool| {
            let mut i = Instance::begin(&[]);
            let mut bg = vec![V::Num(1.0), V::Str(b"L".to_vec())];
            bg.extend(nums(&[0.3, -0.2, -90.0, 0.5, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]));
            i.bg(&bg);
            if xm {
                i.xm(&nums(&[0.0, -200.0, -30.0, 0.0, 10.0, 300.0]));
            }
            // From the stop at 20 to the one at 60, then on to the one at 100.
            i.bm(&nums(&[0.0, -80.0, -30.0, 0.0, 10.0, 260.0]));
            i.bm(&nums(&[0.0, -80.0, -30.0, 0.0, 10.0, 180.0]));
            let Some(Paint::Gradient(gp)) = i.paint(&defs, Affine::IDENTITY) else { panic!() };
            gp.geom.unwrap()
        };
        for xm in [true, false] {
            let geom = instance(xm);
            // (The stops' offsets are `f32`.)
            assert!(geom.start.distance(Point::new(10.0, 300.0)) < 1e-5 && geom.end.distance(Point::new(10.0, 100.0)) < 1e-5, "{xm}: {geom:?}");
            assert_eq!((geom.aspect, geom.focal), (1.0, None));
        }
    }
}
