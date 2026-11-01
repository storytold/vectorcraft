use super::{Affine, CornerKind, Deserialize, PathData, Point, Rect, Serialize, cut_corners, shapes};

/// Live shape parameters (Illustrator's "Live Shapes"). The node's path is regenerated from these
/// while the shape stays live; editing anchors directly converts it to a plain path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "lowercase")]
pub enum LiveShape {
    Rectangle {
        /// Untransformed width/height. Transforms fold their scale into these (see
        /// [`LiveShape::transform`]), so they are document lengths unless `xf` shears.
        w: f64,
        h: f64,
        /// Corner radii: top-left, top-right, bottom-right, bottom-left. In the same units as
        /// `w`/`h`: the corners are circular arcs in the document whenever `xf` doesn't scale.
        radii: [f64; 4],
        /// Corner kinds, in the same order (Live Corners' corner types).
        #[serde(default, skip_serializing_if = "all_round")]
        kinds: [CornerKind; 4],
        /// When true, `radii` are fractions in 0..1 of each corner's largest possible radius.
        #[serde(default, skip_serializing_if = "is_false")]
        radii_relative: bool,
        /// Maps the untransformed shape (origin at its top-left) into the document. A rotation
        /// or reflection plus a move, unless a shear (or a file saved before transforms folded
        /// their scale into `w`/`h`) put more in it.
        xf: Affine,
    },
    Ellipse {
        w: f64,
        h: f64,
        /// Pie start/end angles in degrees (0/360 = full ellipse).
        pie: (f64, f64),
        xf: Affine,
    },
    Polygon {
        radius: f64,
        /// 3–1000, as [`LiveShape::set_sides`] gives it; a file's count is clamped when read.
        #[serde(deserialize_with = "de_sides")]
        sides: u32,
        xf: Affine,
        /// Corner radii in document units, by vertex from the first one clockwise (none: sharp).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        radii: Vec<f64>,
        /// Corner kinds, in the same order (none: round).
        #[serde(default, skip_serializing_if = "all_round")]
        kinds: Vec<CornerKind>,
        #[serde(default, skip_serializing_if = "is_false")]
        radii_relative: bool,
    },
    Line {
        a: Point,
        b: Point,
    },
    /// Any other path whose corners Live Corners cut (a star, a pen path): it keeps its uncut
    /// outline so the corners stay editable. Without a cut corner it is a plain path again.
    Path {
        /// The path with its corners uncut, in the document.
        base: PathData,
        /// Corner radii in document units, by anchor index of `base` (counting every subpath's
        /// anchors in order; missing: sharp).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        radii: Vec<f64>,
        /// Corner kinds, in the same order (missing: round).
        #[serde(default, skip_serializing_if = "all_round")]
        kinds: Vec<CornerKind>,
        #[serde(default, skip_serializing_if = "is_false")]
        radii_relative: bool,
    },
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl LiveShape {
    pub fn radii_relative(&self) -> bool {
        match self {
            LiveShape::Rectangle { radii_relative, .. } | LiveShape::Polygon { radii_relative, .. } | LiveShape::Path { radii_relative, .. } => {
                *radii_relative
            }
            LiveShape::Ellipse { .. } | LiveShape::Line { .. } => false,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            LiveShape::Rectangle { radii, .. } if radii.iter().any(|r| *r > 0.0) => "Rounded Rectangle",
            LiveShape::Rectangle { .. } => "Rectangle",
            LiveShape::Ellipse { .. } => "Ellipse",
            LiveShape::Polygon { .. } => "Polygon",
            LiveShape::Line { .. } => "Line",
            LiveShape::Path { .. } => "Path",
        }
    }
    /// Regenerate the path.
    pub fn to_path(&self) -> PathData {
        match self {
            LiveShape::Rectangle { w, h, radii, kinds, radii_relative, xf } => {
                let base = shapes::rectangle(Rect::new(0.0, 0.0, *w, *h));
                let resolved = crate::corners::resolve_stored_radii(&base, radii, *radii_relative);
                let radii = std::array::from_fn(|i| resolved.get(i).copied().unwrap_or(0.0));
                shapes::rectangle_with_corners(Rect::new(0.0, 0.0, *w, *h), radii, *kinds).transformed(*xf)
            }
            LiveShape::Ellipse { w, h, pie, xf } => shapes::ellipse_pie(Rect::new(0.0, 0.0, *w, *h), pie.0, pie.1).transformed(*xf),
            LiveShape::Polygon { radii, kinds, radii_relative, .. } => {
                let outline = self.polygon_outline();
                let radii = crate::corners::resolve_stored_radii(&outline, radii, *radii_relative);
                cut_corners(&outline, &radii, kinds).0
            }
            LiveShape::Line { a, b } => shapes::line(*a, *b),
            LiveShape::Path { base, radii, kinds, radii_relative } => {
                let radii = crate::corners::resolve_stored_radii(base, radii, *radii_relative);
                cut_corners(base, &radii, kinds).0
            }
        }
    }
    /// A polygon's outline with its corners uncut, in the document (empty for other shapes).
    pub fn polygon_outline(&self) -> PathData {
        match self {
            LiveShape::Polygon { radius, sides, xf, .. } => shapes::polygon(Point::ZERO, *radius, *sides, 0.0).transformed(*xf),
            _ => PathData::default(),
        }
    }
    /// Give a polygon `n` sides (3–1000). Its corners keep the radius and kind they shared; else
    /// the corners past the old last one are sharp.
    pub fn set_sides(&mut self, n: u64) {
        let LiveShape::Polygon { sides, radii, kinds, .. } = self else { return };
        *sides = n.clamp(3, 1000) as u32;
        fn spread<T: Copy + PartialEq + Default>(v: &mut Vec<T>, n: usize) {
            match v.first().copied() {
                Some(x) if v.iter().all(|y| *y == x) => *v = vec![x; n],
                Some(_) => v.resize(n, T::default()),
                None => {}
            }
        }
        spread(radii, *sides as usize);
        spread(kinds, *sides as usize);
    }
    /// A polygon's radius in the document, centre to vertex: its own radius by the mean scale of
    /// `xf` (exact while its sides are equal). None for other shapes.
    pub fn polygon_radius(&self) -> Option<f64> {
        let LiveShape::Polygon { radius, xf, .. } = self else { return None };
        Some(radius * xf.determinant().abs().sqrt())
    }
    /// A polygon's angle: the counterclockwise degrees in [0, 360) its first vertex is turned by
    /// from straight up (the Rotate field's sense). None for other shapes.
    pub fn polygon_angle(&self) -> Option<f64> {
        let LiveShape::Polygon { .. } = self else { return None };
        // No −0° or 360° from rounding: a polygon drawn upright reads 0°.
        let a = (-self.angle_deg()).rem_euclid(360.0);
        Some(if a > 360.0 - 1e-9 { 0.0 } else { a + 0.0 })
    }
    /// Whether a polygon's sides are all as long: `xf` scales it evenly and doesn't shear it. True
    /// for other shapes.
    pub fn polygon_sides_equal(&self) -> bool {
        let LiveShape::Polygon { xf, .. } = self else { return true };
        let [a, b, c, d, _, _] = xf.as_coeffs();
        let (sx, sy) = (a.hypot(b), c.hypot(d));
        (sx - sy).abs() <= 1e-9 * sx.max(sy) && (a * c + b * d).abs() <= 1e-9 * sx * sy
    }
    /// Give a polygon the document radius `r` (centre to vertex, see [`LiveShape::polygon_radius`]).
    pub fn set_polygon_radius(&mut self, r: f64) {
        let LiveShape::Polygon { radius, xf, .. } = self else { return };
        let k = xf.determinant().abs().sqrt();
        if k > 1e-12 && r.is_finite() {
            *radius = r.max(0.0) / k;
        }
    }
    /// The document radius that makes a polygon's sides `side` long (with its sides equal).
    pub fn polygon_radius_for_side(&self, side: f64) -> Option<f64> {
        let LiveShape::Polygon { sides, .. } = self else { return None };
        Some(side / (2.0 * (std::f64::consts::PI / f64::from((*sides).max(3))).sin()))
    }
    /// Turn a polygon about its centre to `angle` (degrees, as [`LiveShape::polygon_angle`]).
    pub fn set_polygon_angle(&mut self, angle: f64) {
        let Some(now) = self.polygon_angle() else { return };
        if let LiveShape::Polygon { xf, .. } = self
            && angle.is_finite()
        {
            let centre = *xf * Point::ZERO;
            *xf = Affine::rotate_about((now - angle).to_radians(), centre) * *xf;
        }
    }
    /// Make a polygon's sides equal (Make Sides Equal): drop the uneven scale and the shear from
    /// `xf`, keeping its centre, angle, reflection and [`LiveShape::polygon_radius`].
    pub fn make_sides_equal(&mut self) {
        let Some(r) = self.polygon_radius() else { return };
        let LiveShape::Polygon { radius, xf, .. } = self else { return };
        let [a, b, _, _, e, f] = xf.as_coeffs();
        let flip = if xf.determinant() < 0.0 { Affine::FLIP_Y } else { Affine::IDENTITY };
        *xf = Affine::translate((e, f)) * Affine::rotate(b.atan2(a)) * flip;
        *radius = r;
    }
    /// Apply `a` to the shape. True when the shape is no longer `a` applied to the old path, so
    /// the caller must regenerate the path with [`LiveShape::to_path`].
    ///
    /// A rectangle keeps round corners whatever its proportions: when `a` scales it along its own
    /// sides (any rotation or reflection, no shear), the scale goes into `w`/`h` and `xf` keeps
    /// only the rotation, reflection and position. The radii scale by the mean scale (the square
    /// root of the determinant), which [`LiveShape::keep_corners`] undoes for Scale Corners off.
    /// A shear can't keep circular corners: `xf` takes it, as do moves (so files saved with a
    /// scale in `xf` keep their exact geometry until they're next transformed). A polygon's or a
    /// path's corner radii are document lengths: they scale by the mean scale and the corners are
    /// cut again, circular whatever `a` does.
    pub fn transform(&mut self, a: Affine) -> bool {
        match self {
            LiveShape::Rectangle { xf, .. } => {
                *xf = a * *xf;
                !moves_only(a) && self.fold_scale()
            }
            LiveShape::Ellipse { xf, .. } => {
                *xf = a * *xf;
                false
            }
            LiveShape::Polygon { xf, radii, radii_relative, .. } => {
                *xf = a * *xf;
                scale_radii(radii, a, *radii_relative)
            }
            LiveShape::Path { base, radii, radii_relative, .. } => {
                base.transform(a);
                scale_radii(radii, a, *radii_relative)
            }
            LiveShape::Line { a: p, b } => {
                *p = a * *p;
                *b = a * *b;
                false
            }
        }
    }
    /// Move the scale of a rectangle's `xf` into `w`/`h` (and its radii, by the mean scale), so
    /// `xf` keeps only a rotation or reflection plus a move and the corners are circular in the
    /// document. False (and nothing changes) when there is no scale to move, when `xf` shears or
    /// isn't finite, and for other shapes. Files saved before transforms did this (#291) carry an
    /// uneven scale in `xf`: their corners are elliptical until this runs.
    pub fn fold_scale(&mut self) -> bool {
        let LiveShape::Rectangle { w, h, radii, radii_relative, xf, .. } = self else { return false };
        let [m0, m1, m2, m3, m4, m5] = xf.as_coeffs();
        let (sx, sy) = (m0.hypot(m1), m2.hypot(m3));
        let orthogonal = sx > 1e-12 && sy > 1e-12 && (m0 * m2 + m1 * m3).abs() <= 1e-9 * sx * sy;
        let finite = xf.as_coeffs().iter().all(|c| c.is_finite()) && w.is_finite() && h.is_finite();
        let unscaled = (sx - 1.0).abs() < 1e-12 && (sy - 1.0).abs() < 1e-12;
        if !orthogonal || !finite || unscaled {
            return false;
        }
        let k = (sx * sy).sqrt();
        if !*radii_relative {
            radii.iter_mut().for_each(|r| *r *= k);
        }
        *w *= sx;
        *h *= sy;
        *xf = Affine::new([m0 / sx, m1 / sx, m2 / sy, m3 / sy, m4, m5]);
        true
    }
    /// This shape with [`LiveShape::fold_scale`] applied: a rectangle's size and radii in document
    /// units (what Live Corners show and edit).
    pub fn folded(&self) -> Self {
        let mut s = self.clone();
        s.fold_scale();
        s
    }
    /// Divide live corner radii by `k`, the mean scale of a transform just applied, so the corners
    /// keep their size (Scale Corners off). False when nothing changed.
    pub fn keep_corners(&mut self, k: f64) -> bool {
        let relative = self.radii_relative();
        let radii: &mut [f64] = match self {
            LiveShape::Rectangle { radii, .. } => radii,
            LiveShape::Polygon { radii, .. } | LiveShape::Path { radii, .. } => radii,
            LiveShape::Ellipse { .. } | LiveShape::Line { .. } => return false,
        };
        if relative || !radii.iter().any(|r| *r > 0.0) || k <= 1e-12 {
            return false;
        }
        radii.iter_mut().for_each(|r| *r /= k);
        true
    }
    /// Rotation angle of the live shape in degrees, clockwise on the page (y points down).
    pub fn angle_deg(&self) -> f64 {
        match self {
            LiveShape::Rectangle { xf, .. } | LiveShape::Ellipse { xf, .. } | LiveShape::Polygon { xf, .. } => {
                let c = xf.as_coeffs();
                c[1].atan2(c[0]).to_degrees()
            }
            LiveShape::Line { a, b } => (b.y - a.y).atan2(b.x - a.x).to_degrees(),
            LiveShape::Path { .. } => 0.0,
        }
    }
}

fn all_round(kinds: &[CornerKind]) -> bool {
    kinds.iter().all(|k| *k == CornerKind::Round)
}

/// A polygon's side count read from a file, clamped as the commands clamp it: the outline and its
/// corner lists hold one entry per side, so a huge count would allocate gigabytes.
fn de_sides<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    u32::deserialize(d).map(|n| n.clamp(3, 1000))
}

/// Whether `a` only moves (no rotation, scale, shear or reflection).
pub(super) fn moves_only(a: Affine) -> bool {
    let [a0, a1, a2, a3, _, _] = a.as_coeffs();
    (a0 - 1.0).abs() < 1e-12 && a1.abs() < 1e-12 && a2.abs() < 1e-12 && (a3 - 1.0).abs() < 1e-12
}

/// Scale corner radii in document units by the mean scale of `a`, applied to their path. True
/// when the path must be cut again: some corner is cut and `a` does more than move it (an
/// uneven scale would otherwise make the cuts elliptical).
fn scale_radii(radii: &mut [f64], a: Affine, relative: bool) -> bool {
    if relative || moves_only(a) || !radii.iter().any(|r| *r > 0.0) {
        return false;
    }
    let k = a.determinant().abs().sqrt();
    radii.iter_mut().for_each(|r| *r *= k);
    true
}

/// What a transform scales besides geometry, by its mean scale (the square root of its
/// determinant). `Scaling::default()` (and `false`) scales nothing else; `true` scales stroke
/// weights and dashes only. Transforms the user asks for take theirs from Scale Strokes & Effects
/// and Scale Corners.
#[derive(Clone, Copy, Debug, Default)]
pub struct Scaling {
    /// Stroke weights, dash lengths and dash offsets scale.
    pub strokes: bool,
    /// With `strokes`: scales an effect's distance parameters (the effects catalogue's).
    pub effects: Option<fn(&mut crate::Effect, f64)>,
    /// Type keeps its character strokes' weight (they otherwise scale with the type).
    pub keep_type_strokes: bool,
    /// Live corner radii keep their size.
    pub keep_corners: bool,
    /// Pattern fills and strokes move with the art (Transform Patterns; General › Transform
    /// Pattern Tiles is its default): their tiles transform too, instead of staying put.
    pub patterns: bool,
}

impl Scaling {
    /// The mean scale of `a` when it scales (isn't 1): what strokes, effects and corners scale by.
    pub fn factor(a: Affine) -> Option<f64> {
        let k = a.determinant().abs().sqrt();
        ((k - 1.0).abs() > 1e-9).then_some(k)
    }
}

impl From<bool> for Scaling {
    fn from(strokes: bool) -> Self {
        Self { strokes, ..Self::default() }
    }
}
