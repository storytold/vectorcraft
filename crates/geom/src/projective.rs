//! Projective (perspective) transforms of the plane: 3×3 homographies.
//!
//! The Perspective Grid maps each grid plane to the page with one, objects in perspective move by
//! conjugating plane-space affine maps with it, and type and symbol instances in perspective keep
//! one that projects their flat art.

use kurbo::{Affine, Point, Rect};

/// A 3×3 projective transform (row-major), acting on `(x, y, 1)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography(pub [[f64; 3]; 3]);

impl Homography {
    pub const IDENTITY: Self = Self([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    /// From the images of (1,0,0), (0,1,0) and (0,0,1).
    pub fn from_cols(c1: [f64; 3], c2: [f64; 3], c3: [f64; 3]) -> Self {
        Self([[c1[0], c2[0], c3[0]], [c1[1], c2[1], c3[1]], [c1[2], c2[2], c3[2]]])
    }

    /// The affine map `a` as a homography.
    pub fn from_affine(a: Affine) -> Self {
        let [m0, m1, m2, m3, m4, m5] = a.as_coeffs();
        Self([[m0, m2, m4], [m1, m3, m5], [0.0, 0.0, 1.0]])
    }

    /// From nine row-major numbers (as stored in documents); `None` unless all are finite and the
    /// map is invertible. When the determinant is not finite, this is the same map divided by the
    /// largest magnitude in its `w` row (so that row's largest entry is 1), and `None` when that
    /// copy has an entry that is not finite or is not invertible.
    pub fn from_array(v: [f64; 9]) -> Option<Self> {
        if v.iter().any(|x| !x.is_finite()) {
            return None;
        }
        let h = Self([[v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]]);
        if h.det().is_finite() {
            return h.is_invertible().then_some(h);
        }
        let k = h.0[2].iter().fold(0.0_f64, |k, v| k.max(v.abs()));
        if k <= 0.0 || !h.is_invertible() {
            return None;
        }
        let c = Self(h.0.map(|row| row.map(|v| v / k)));
        (c.is_finite() && c.is_invertible()).then_some(c)
    }

    /// The nine row-major numbers.
    pub fn to_array(&self) -> [f64; 9] {
        let m = &self.0;
        [m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2]]
    }

    /// Map a point; `None` when it lands on or beyond the horizon (w ≤ 0), or when it does not map
    /// to a finite page point.
    pub fn apply(&self, p: Point) -> Option<Point> {
        let m = &self.0;
        let x = m[0][0] * p.x + m[0][1] * p.y + m[0][2];
        let y = m[1][0] * p.x + m[1][1] * p.y + m[1][2];
        let w = m[2][0] * p.x + m[2][1] * p.y + m[2][2];
        if w <= 1e-9 || !x.is_finite() || !y.is_finite() {
            return None;
        }
        let q = Point::new(x / w, y / w);
        q.is_finite().then_some(q)
    }

    pub fn det(&self) -> f64 {
        let m = &self.0;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// Whether the map is invertible: the magnitude of its determinant is above 1e-15, or, when the
    /// determinant is not finite, [`Self::inverse`] gives one.
    fn is_invertible(&self) -> bool {
        let d = self.det();
        if d.is_finite() {
            return d.abs() > 1e-15;
        }
        self.inverse().is_some()
    }

    /// The same map with every entry divided by one power of two, chosen so that the largest of the
    /// six products its determinant sums is at least 1/8 and below 4. `None` when every one of
    /// those products is 0 or an entry is not finite.
    fn scaled(&self) -> Option<Self> {
        if !self.is_finite() {
            return None;
        }
        let f = self.0.map(|row| row.map(frexp));
        // The columns each of the six products takes from rows 0, 1 and 2.
        let cols = [(0, 1, 2), (0, 2, 1), (1, 0, 2), (1, 2, 0), (2, 0, 1), (2, 1, 0)];
        let e = cols
            .iter()
            .map(|&(a, b, c)| (f[0][a], f[1][b], f[2][c]))
            .filter(|(a, b, c)| a.0 * b.0 * c.0 != 0.0)
            .map(|(a, b, c)| a.1 + b.1 + c.1)
            .max()?;
        let e = e.div_euclid(3);
        Some(Self(self.0.map(|row| row.map(|v| ldexp(v, -e)))))
    }

    /// Inverse; `None` when the map is not invertible or its inverse has an entry that is not
    /// finite. When the determinant is not finite, this is the inverse of [`Self::scaled`] (the
    /// exact inverse multiplied by one power of two, which is the same map), and that copy counts
    /// as not invertible when the magnitude of its determinant is at most 1e-15 times that of the largest
    /// of the six products the determinant sums.
    pub fn inverse(&self) -> Option<Self> {
        let d = self.det();
        let (h, d) = if d.is_finite() {
            if d.abs() < 1e-15 {
                return None;
            }
            (*self, d)
        } else {
            let s = self.scaled()?;
            let m = &s.0;
            let terms = [
                m[0][0] * m[1][1] * m[2][2],
                m[0][0] * m[1][2] * m[2][1],
                m[0][1] * m[1][0] * m[2][2],
                m[0][1] * m[1][2] * m[2][0],
                m[0][2] * m[1][0] * m[2][1],
                m[0][2] * m[1][1] * m[2][0],
            ];
            let largest = terms.iter().fold(0.0_f64, |k, v| k.max(v.abs()));
            let d = s.det();
            if d.abs() <= 1e-15 * largest {
                return None;
            }
            (s, d)
        };
        let m = &h.0;
        let c = |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
        let adj = [
            [c(1, 1, 2, 2), -c(0, 1, 2, 2), c(0, 1, 1, 2)],
            [-c(1, 0, 2, 2), c(0, 0, 2, 2), -c(0, 0, 1, 2)],
            [c(1, 0, 2, 1), -c(0, 0, 2, 1), c(0, 0, 1, 1)],
        ];
        let mut out = [[0.0; 3]; 3];
        for (r, row) in adj.iter().enumerate() {
            for (k, v) in row.iter().enumerate() {
                out[r][k] = v / d;
            }
        }
        if out.iter().flatten().any(|v| !v.is_finite()) {
            return None;
        }
        // H⁻¹·(x, y, 1) = (u, v, 1)/w_H, so points in front of the camera keep w > 0.
        Some(Self(out))
    }

    /// `self ∘ other`: `other` first.
    pub fn then_after(&self, other: &Self) -> Self {
        let (a, b) = (&self.0, &other.0);
        let mut out = [[0.0; 3]; 3];
        for (r, row) in out.iter_mut().enumerate() {
            for (k, v) in row.iter_mut().enumerate() {
                *v = a[r][0] * b[0][k] + a[r][1] * b[1][k] + a[r][2] * b[2][k];
            }
        }
        Self(out)
    }

    /// [`Self::then_after`]; `None` when an entry of the product is not finite (an entry
    /// overflows, or a factor has one that is not finite).
    pub fn checked_then_after(&self, other: &Self) -> Option<Self> {
        let c = self.then_after(other);
        c.is_finite().then_some(c)
    }

    /// `self` seen through `a`: the map `a ∘ self ∘ a⁻¹` (what `self` becomes when everything it
    /// maps from and to is moved by `a`). When the product overflows, every entry is formed with
    /// its powers of two kept apart, and the product is returned divided by the largest magnitude
    /// in its `w` row (the same map, with a `w` row whose largest entry is 1). `None` when `a`
    /// isn't invertible, or when the product overflows and the result at that scale has an entry
    /// that is not finite or is not invertible.
    pub fn conjugated(&self, a: Affine) -> Option<Self> {
        if a.determinant().abs() < 1e-15 {
            return None;
        }
        let (to, from) = (Self::from_affine(a), Self::from_affine(a.inverse()));
        let c = to.then_after(self).then_after(&from);
        if c.is_finite() {
            return Some(c);
        }
        // Each entry of the product is a sum of nine terms to[r][i]·self[i][j]·from[j][k]. Every
        // factor is split into a mantissa and a power of two, so each term is a mantissa product
        // and an exponent sum, and the whole product is scaled by one power of two at the end.
        let split = |h: &Self| h.0.map(|row| row.map(frexp));
        let (t, s, f) = (split(&to), split(self), split(&from));
        let term = |r: usize, i: usize, j: usize, k: usize| {
            let (a, b, c) = (t[r][i], s[i][j], f[j][k]);
            (a.0 * b.0 * c.0, a.1 + b.1 + c.1)
        };
        let ijk = (0..27).map(|n| (n / 9, n / 3 % 3, n % 3));
        let e = ijk.map(|(i, j, k)| term(2, i, j, k)).filter(|(m, _)| *m != 0.0).map(|(_, e)| e).max()?;
        let mut c = [[0.0; 3]; 3];
        for (r, row) in c.iter_mut().enumerate() {
            for (k, v) in row.iter_mut().enumerate() {
                *v = (0..9).map(|n| term(r, n / 3, n % 3, k)).map(|(m, x)| ldexp(m, x - e)).sum();
            }
        }
        let k = c[2].iter().fold(0.0_f64, |k, v| k.max(v.abs()));
        if k <= 0.0 {
            return None;
        }
        let c = Self(c.map(|row| row.map(|v| v / k)));
        (c.is_finite() && c.is_invertible()).then_some(c)
    }

    fn is_finite(&self) -> bool {
        self.0.iter().flatten().all(|v| v.is_finite())
    }

    /// The affine map that matches `self` to first order at `p` (its derivative there), `None`
    /// beyond the horizon or when the derivative is not finite.
    pub fn affine_at(&self, p: Point) -> Option<Affine> {
        let m = &self.0;
        let w = m[2][0] * p.x + m[2][1] * p.y + m[2][2];
        let q = self.apply(p)?;
        // d(x/w) = (dx − (x/w)·dw) / w, and the same for y.
        let j = |r: usize, c: usize| (m[r][c] - [q.x, q.y][r] * m[2][c]) / w;
        let lin = Affine::new([j(0, 0), j(1, 0), j(0, 1), j(1, 1), 0.0, 0.0]);
        let out = Affine::translate(q.to_vec2()) * lin * Affine::translate(-p.to_vec2());
        out.is_finite().then_some(out)
    }

    /// The bounding box of the image of `r` (lines stay lines: its corners bound it), `None` when
    /// part of it crosses the horizon.
    pub fn map_rect_bbox(&self, r: Rect) -> Option<Rect> {
        let pts = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
        let mut out: Option<Rect> = None;
        for p in pts {
            let q = self.apply(p)?;
            out = Some(out.map_or(Rect::from_points(q, q), |b| b.union_pt(q)));
        }
        out
    }
}

/// `v` as `m·2^e` with `0.5 ≤ |m| < 1`; 0, infinities and NaN come back as `(v, 0)`.
fn frexp(v: f64) -> (f64, i32) {
    if v == 0.0 || !v.is_finite() {
        return (v, 0);
    }
    // A subnormal is scaled into the normal range first.
    let (v, bias) = if v.abs() < f64::MIN_POSITIVE { (v * pow2(64), -64) } else { (v, 0) };
    let bits = v.to_bits();
    let e = ((bits >> 52) & 0x7ff) as i32 - 1022;
    (f64::from_bits((bits & !(0x7ff << 52)) | (1022 << 52)), e + bias)
}

/// `m·2^e`, multiplied in steps of powers of two that are each within the normal range.
fn ldexp(mut m: f64, mut e: i32) -> f64 {
    while e > 1023 && m.is_finite() {
        m *= pow2(1023);
        e -= 1023;
    }
    while e < -1022 && m != 0.0 {
        m *= pow2(-1022);
        e += 1022;
    }
    m * pow2(e.clamp(-1022, 1023))
}

/// `2^e` for `-1022 ≤ e ≤ 1023`.
fn pow2(e: i32) -> f64 {
    f64::from_bits(((e + 1023) as u64) << 52)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h() -> Homography {
        Homography([[1.2, 0.1, 5.0], [-0.2, 0.9, 3.0], [0.001, 0.002, 1.0]])
    }

    #[test]
    fn compose_invert_and_round_trip() {
        let a = h();
        let inv = a.inverse().unwrap();
        let id = a.then_after(&inv);
        for (r, row) in id.0.iter().enumerate() {
            for (k, v) in row.iter().enumerate() {
                assert!((v - if r == k { 1.0 } else { 0.0 }).abs() < 1e-9);
            }
        }
        let p = Point::new(30.0, -12.0);
        assert!(inv.apply(a.apply(p).unwrap()).unwrap().distance(p) < 1e-9);
        assert_eq!(Homography::from_array(a.to_array()), Some(a));
        assert!(Homography::from_array([f64::NAN; 9]).is_none());
        assert!(Homography::from_array([0.0; 9]).is_none());
    }

    #[test]
    fn affine_and_conjugation() {
        let t = Affine::translate((10.0, -4.0)) * Affine::scale(2.0);
        let p = Point::new(3.0, 7.0);
        assert!(Homography::from_affine(t).apply(p).unwrap().distance(t * p) < 1e-12);
        // Conjugating: moving the input and the output by `t` keeps the picture moved by `t`.
        let c = h().conjugated(t).unwrap();
        assert!(c.apply(t * p).unwrap().distance(t * h().apply(p).unwrap()) < 1e-9);
        assert!(h().conjugated(Affine::scale(0.0)).is_none());
    }

    #[test]
    fn affine_at_is_the_derivative() {
        let p = Point::new(12.0, -7.0);
        let a = h().affine_at(p).unwrap();
        assert!(a * p == h().apply(p).unwrap() || (a * p).distance(h().apply(p).unwrap()) < 1e-9);
        let e = 1e-4;
        for d in [kurbo::Vec2::new(e, 0.0), kurbo::Vec2::new(0.0, e)] {
            let exact = h().apply(p + d).unwrap();
            assert!((a * (p + d)).distance(exact) < 1e-7, "{d:?}");
        }
    }

    #[test]
    fn a_point_that_overflows_maps_to_nothing() {
        // w = 1e-8 is in front of the horizon, but 1e305 / 1e-8 overflows.
        let near = Homography([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1e-8]]);
        assert_eq!(near.apply(Point::new(1e305, 0.0)), None);
        assert_eq!(near.apply(Point::new(0.0, -1e305)), None);
        assert_eq!(near.map_rect_bbox(Rect::new(0.0, 0.0, 1e305, 1.0)), None);
        assert!(near.apply(Point::new(2.0, 3.0)).unwrap().distance(Point::new(2e8, 3e8)) < 1e-6);
        // A NaN w is not caught by the horizon test.
        let nan = Homography([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, f64::NAN]]);
        assert_eq!(nan.apply(Point::new(1.0, 1.0)), None);
    }

    #[test]
    fn an_inverse_that_overflows_is_none() {
        let big = Homography([[1e300, 0.0, 0.0], [0.0, 1e300, 0.0], [0.0, 0.0, 1e-310]]);
        assert_eq!(big.inverse(), None);
        assert!(h().inverse().is_some());
    }

    #[test]
    fn a_determinant_that_overflows_is_taken_at_a_smaller_scale() {
        // (x, y) ↦ (x + 1, y) / (x + 2) with every entry multiplied by 1e200: the determinant is
        // inf − inf, though the map is invertible.
        let big = Homography([[1e200, 0.0, 1e200], [0.0, 1e200, 0.0], [1e200, 0.0, 2e200]]);
        assert!(big.det().is_nan());
        // Read back as the same map with a w row whose largest entry is 1.
        let read = Homography::from_array(big.to_array()).unwrap();
        assert_eq!(read, Homography([[0.5, 0.0, 0.5], [0.0, 0.5, 0.0], [0.5, 0.0, 1.0]]));
        assert_eq!(Homography::from_array(read.to_array()), Some(read));
        let inv = big.inverse().unwrap();
        for p in [Point::new(3.0, -2.0), Point::new(-1.5, 4.0)] {
            assert!(read.apply(p).unwrap().distance(big.apply(p).unwrap()) < 1e-12);
            assert!(inv.apply(big.apply(p).unwrap()).unwrap().distance(p) < 1e-9);
        }
        // The identity with every entry 1e308: stored as it is, (2, 2) overflows to 2e308, but the
        // map read back draws it at (2, 2).
        let wide = Homography([[1e308, 0.0, 0.0], [0.0, 1e308, 0.0], [0.0, 0.0, 1e308]]);
        assert_eq!(wide.apply(Point::new(2.0, 2.0)), None);
        let read = Homography::from_array(wide.to_array()).unwrap();
        assert_eq!(read.apply(Point::new(2.0, 2.0)), Some(Point::new(2.0, 2.0)));
        // A large map whose w row is 1e-205 times its largest entry: read back at a w row of 1,
        // its points are in front of the horizon and draw.
        let low = Homography([[1e300, 0.0, 1e305], [0.0, 1e300, 0.0], [1e90, 0.0, 1e100]]);
        assert!(low.det().is_nan());
        let read = Homography::from_array(low.to_array()).unwrap();
        assert_eq!(Homography::from_array(read.to_array()), Some(read));
        let p = Point::new(1.0, 1.0);
        let want = Point::new((1e200 + 1e205) / (1.0 + 1e-10), 1e200 / (1.0 + 1e-10));
        assert!(read.apply(p).unwrap().distance(want) <= 1e-9 * want.to_vec2().hypot());
        // A scale by 1e305 and a move by -3.9e307 with a w row of 1: the determinant is inf, and
        // the inverse is a scale by 1e-305 and a move by 390.
        let moved = Homography([[1e305, 0.0, -3.9e307], [0.0, 1e305, -3.2e307], [0.0, 0.0, 1.0]]);
        assert!(moved.det().is_infinite());
        assert_eq!(Homography::from_array(moved.to_array()), Some(moved));
        let inv = moved.inverse().unwrap();
        for p in [Point::new(400.0, 330.0), Point::new(500.0, 400.0)] {
            assert!(inv.apply(moved.apply(p).unwrap()).unwrap().distance(p) < 1e-9);
        }
        // A scale by 1e500: the determinant is inf, but scaled so that its products are near 1
        // the w row is 0, and no point maps to a finite page point except the origin.
        let flat = Homography([[1e300, 0.0, 0.0], [0.0, 1e300, 0.0], [0.0, 0.0, 1e-200]]);
        assert!(flat.det().is_infinite());
        assert_eq!(Homography::from_array(flat.to_array()), None);
        assert_eq!(flat.inverse(), None);
        // Two equal rows with large entries.
        let twice = Homography([[1e200, 1e200, 0.0], [1e200, 1e200, 0.0], [0.0, 0.0, 1e200]]);
        assert_eq!(Homography::from_array(twice.to_array()), None);
        assert_eq!(twice.inverse(), None);
        // (x, y) ↦ 1e200·(x + 1, y) / (x + 2) seen through a scale by 1e10: the product
        // overflows, and the determinant of the result at a w row of 1 is inf − inf.
        let far = Homography([[1e300, 0.0, 1e300], [0.0, 1e300, 0.0], [1e100, 0.0, 2e100]]);
        let t = Affine::scale(1e10);
        let c = far.conjugated(t).unwrap();
        assert!(c.det().is_nan());
        assert_eq!(Homography::from_array(c.to_array()), Some(c));
        for p in [Point::new(3.0, -2.0), Point::new(0.5, 7.0)] {
            let want = t * far.apply(p).unwrap();
            assert!(c.apply(t * p).unwrap().distance(want) <= 1e-9 * want.to_vec2().hypot(), "{c:?}");
        }
        // A determinant that does not overflow is judged as it is.
        assert!(Homography::from_array([1e-6, 0.0, 0.0, 0.0, 1e-6, 0.0, 0.0, 0.0, 1e-6]).is_none());
        assert!(Homography::from_array([f64::NAN; 9]).is_none());
        assert!(Homography::from_array([0.0; 9]).is_none());
    }

    #[test]
    fn a_product_that_overflows_is_none() {
        // Each factor is finite and invertible, but the product's first entry is 1e400.
        let big = Homography([[1e200, 0.0, 0.0], [0.0, 1e200, 0.0], [0.0, 0.0, 1e200]]);
        assert!(Homography::from_array(big.to_array()).is_some());
        assert_eq!(big.checked_then_after(&big), None);
        let nan = Homography([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, f64::NAN]]);
        assert_eq!(h().checked_then_after(&nan), None);
        assert_eq!(h().checked_then_after(&Homography::IDENTITY), Some(h()));
        let inv = h().inverse().unwrap();
        assert_eq!(h().checked_then_after(&inv), Some(h().then_after(&inv)));
    }

    #[test]
    fn a_derivative_that_overflows_is_none() {
        // The point maps to 1e300, but the derivative there is 1e302 / 1e-8.
        let steep = Homography([[1e302, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1e-8]]);
        let p = Point::new(1e-10, 0.0);
        assert!(steep.apply(p).is_some());
        assert_eq!(steep.affine_at(p), None);
    }

    #[test]
    fn a_conjugate_that_overflows_is_scaled_down() {
        // A move by 10, scaled by 1e307: seen through a scale by 10 it moves by 100, but the
        // product's corner entry is 1e309.
        let big = Homography([[1e307, 0.0, 1e308], [0.0, 1e307, 0.0], [0.0, 0.0, 1e307]]);
        let t = Affine::scale(10.0);
        let c = big.conjugated(t).unwrap();
        assert!(c.is_finite());
        assert_eq!(c.0[2], [0.0, 0.0, 1.0]);
        assert_eq!(Homography::from_array(c.to_array()), Some(c));
        for p in [Point::new(3.0, 7.0), Point::new(-4.0, 1.5)] {
            assert!(c.apply(t * p).unwrap().distance(t * big.apply(p).unwrap()) < 1e-9);
            assert!(c.apply(p).unwrap().distance(p + kurbo::Vec2::new(100.0, 0.0)) < 1e-9);
        }
        // A move by 1 scaled by 1e307, seen through a scale by 1e7: a move by 1e7, whose
        // determinant is 1 at a w row of 1 but 1e-21 at a largest entry of 1.
        let one = Homography([[1e307, 0.0, 1e307], [0.0, 1e307, 0.0], [0.0, 0.0, 1e307]]);
        assert!(Homography::from_array(one.to_array()).is_some());
        let c = one.conjugated(Affine::scale(1e7)).unwrap();
        assert_eq!(Homography::from_array(c.to_array()), Some(c));
        let p = Point::new(3.0, -2.0);
        assert!(c.apply(p).unwrap().distance(p + kurbo::Vec2::new(1e7, 0.0)) < 1e-6);
        // A scale by 1e10 seen through a scale by 1e10: still a scale by 1e10, which every point
        // reaches only when w is not left at 1e-10.
        let wide = Homography([[1e300, 0.0, 0.0], [0.0, 1e300, 0.0], [0.0, 0.0, 1e290]]);
        let c = wide.conjugated(Affine::scale(1e10)).unwrap();
        assert_eq!(Homography::from_array(c.to_array()), Some(c));
        for p in [Point::new(1.0, 2.0), Point::new(-5.0, 0.5)] {
            assert!(c.apply(p).unwrap().distance(Point::new(p.x * 1e10, p.y * 1e10)) < 1e-3);
        }
        // A scale by 2 seen through a move by 1e308: p goes to 2p - 1e308, though the product's
        // corner entry is 2 * -1e308 + 1e308 and the moves scaled to a largest entry of 1 leave
        // their w rows at 1e-308.
        let two = Homography([[2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 1.0]]);
        let c = two.conjugated(Affine::translate((1e308, 0.0))).unwrap();
        let want = [2.0, 0.0, -1e308, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0];
        for (v, w) in c.to_array().iter().zip(want) {
            assert!((v - w).abs() <= 1e-9 * w.abs().max(1.0), "{c:?}");
        }
        assert_eq!(Homography::from_array(c.to_array()), Some(c));
        // A scale by (1e160, 1e-160) seen through the same scale is itself, though the product's
        // first entry is 1e320 and a scale to a largest entry of 1 takes its second to 1e-480.
        let thin = Homography([[1e160, 0.0, 0.0], [0.0, 1e-160, 0.0], [0.0, 0.0, 1.0]]);
        assert!(Homography::from_array(thin.to_array()).is_some());
        let c = thin.conjugated(Affine::scale_non_uniform(1e160, 1e-160)).unwrap();
        for (v, w) in c.to_array().iter().zip(thin.to_array()) {
            assert!((v - w).abs() <= 1e-9 * w.abs(), "{c:?}");
        }
        assert_eq!(Homography::from_array(c.to_array()), Some(c));
        // A move by 1e200 seen through a scale by 1e150 sends every point to about (1e350, 0),
        // which no invertible map of finite numbers draws.
        let far = Homography([[1.0, 0.0, 1e200], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        assert_eq!(far.conjugated(Affine::scale(1e150)), None);
        assert!(far.conjugated(Affine::scale(2.0)).is_some());
        // A product that does not overflow is kept as it is.
        assert_eq!(far.conjugated(Affine::scale(2.0)).unwrap().0[0][2], 2e200);
    }

    #[test]
    fn rect_bbox_is_the_corners_box() {
        let b = h().map_rect_bbox(Rect::new(0.0, 0.0, 10.0, 20.0)).unwrap();
        for p in [Point::new(0.0, 0.0), Point::new(10.0, 20.0), Point::new(10.0, 0.0), Point::new(5.0, 10.0)] {
            let q = h().apply(p).unwrap();
            assert!(b.inflate(1e-9, 1e-9).contains(q));
        }
    }
}
