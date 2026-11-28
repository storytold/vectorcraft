//! The matrix functions of `app`, as the help pages
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Matrix/> and
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Application/> describe them. A matrix is a plain
//! record with `mValueA`, `mValueB`, `mValueC`, `mValueD`, `mValueTX` and `mValueTY` and the
//! `typename` `"Matrix"`; each function other than `isEqualMatrix` and `isSingularMatrix` gives a
//! new record, those two give a boolean, and none changes the records passed to it.
//!
//! The record maps a point `(x, y)` to `(a·x + c·y + tx, b·x + d·y + ty)`, as a PDF `cm` matrix
//! does (ISO 32000-1, 8.3.4), in script coordinates with Y up.
//!
//! VectorCraft's own rules, which the help pages do not state:
//! - `concatenateMatrix(m, n)` gives the matrix that applies `m` and then `n`; the other
//!   `concatenate…` functions apply `m` and then the rotation, scale or translation they make.
//! - Left-out arguments are 0 for a translation and an angle, and 100 for a horizontal scale; a
//!   left-out vertical scale is the horizontal one.
//! - Angles are degrees, positive counterclockwise, and quarter turns are exact.
//! - A matrix whose determinant is 0 is singular, and `invertMatrix` of one throws.
//! - `isEqualMatrix` compares the six values exactly.
//! - An argument that is not a finite number, or a matrix with a value that is not one, throws a
//!   `TypeError`; a result with a value too large for a number throws a `RangeError`.

use vectorcraft_js::{Data, HostError, HostMember};

use super::{Answer, Host, Link, arg, method, named, num, text};

/// A matrix's six values: `[a, b, c, d, tx, ty]`.
pub(super) type Matrix = [f64; 6];

const KEYS: [&str; 6] = ["mValueA", "mValueB", "mValueC", "mValueD", "mValueTX", "mValueTY"];

const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// The matrix functions of `Application`.
pub(super) fn app_members(link: &Link) -> Vec<(String, HostMember)> {
    let l = link;
    named(vec![
        ("getIdentityMatrix", method(l, identity)),
        ("getTranslationMatrix", method(l, translation)),
        ("getScaleMatrix", method(l, scale)),
        ("getRotationMatrix", method(l, rotation)),
        ("concatenateMatrix", method(l, concatenate)),
        ("concatenateTranslationMatrix", method(l, concatenate_translation)),
        ("concatenateScaleMatrix", method(l, concatenate_scale)),
        ("concatenateRotationMatrix", method(l, concatenate_rotation)),
        ("invertMatrix", method(l, invert)),
        ("isEqualMatrix", method(l, is_equal)),
        ("isSingularMatrix", method(l, is_singular)),
    ])
}

/// The record for `m`, or a `RangeError` when a value is not finite.
fn record(m: Matrix) -> Answer {
    if m.iter().any(|v| !v.is_finite()) {
        return Err(HostError::Range("the matrix has a value too large for a number".into()));
    }
    let mut items: Vec<(String, Data)> = KEYS.iter().zip(m).map(|(k, v)| ((*k).to_string(), num(v))).collect();
    items.push(("typename".into(), text("Matrix")));
    Ok(Data::Object(items))
}

/// The matrix argument `d`.
pub(super) fn matrix(d: &Data, what: &str) -> std::result::Result<Matrix, HostError> {
    let bad = || HostError::Type(format!("{what} must be a Matrix"));
    let Data::Object(items) = d else { return Err(bad()) };
    let mut m = [0.0; 6];
    for (slot, k) in m.iter_mut().zip(KEYS) {
        match items.iter().find(|(n, _)| n == k) {
            Some((_, Data::Number(v))) if v.is_finite() => *slot = *v,
            _ => return Err(bad()),
        }
    }
    Ok(m)
}

/// A number argument, `default` when it is left out.
fn number(d: &Data, what: &str, default: f64) -> std::result::Result<f64, HostError> {
    match d {
        Data::Undefined => Ok(default),
        Data::Number(n) if n.is_finite() => Ok(*n),
        _ => Err(HostError::Type(format!("{what} must be a finite number"))),
    }
}

/// The matrix that applies `m` and then `n`. Adding 0 turns -0 into 0.
fn then(m: Matrix, n: Matrix) -> Matrix {
    let [a, b, c, d, e, f] = m;
    let [a2, b2, c2, d2, e2, f2] = n;
    [
        products_sum(a, a2, b, c2, 0.0) + 0.0,
        products_sum(a, b2, b, d2, 0.0) + 0.0,
        products_sum(c, a2, d, c2, 0.0) + 0.0,
        products_sum(c, b2, d, d2, 0.0) + 0.0,
        products_sum(e, a2, f, c2, e2) + 0.0,
        products_sum(e, b2, f, d2, f2) + 0.0,
    ]
}

/// `x·y + z·w + t`. When both products and their sum are finite, and a product below the range of
/// normal numbers is the only term that is not 0, the sum is taken as numbers are, unless adding
/// `t` to the rounded sum of the products can come within a step of 2^1024. Otherwise the products
/// are summed past the range of a number and rounded at the end, so a finite value is not lost to
/// a product too large or too small for a number, and a sum is not taken past the largest number,
/// or kept below it, by a rounding on the way.
fn products_sum(x: f64, y: f64, z: f64, w: f64, t: f64) -> f64 {
    let (p, q) = (x * y, z * w);
    let lost = |v: f64, a: f64, b: f64| a != 0.0 && b != 0.0 && v.abs() < f64::MIN_POSITIVE;
    let alone = |a: f64, b: f64| (a == 0.0 || b == 0.0) && t == 0.0;
    let plain = p + q;
    let near_overflow = |v: f64| v.abs() >= pow2(1023);
    let overflow_safe = t == 0.0 || !(near_overflow(plain) || near_overflow(plain + t));
    if plain.is_finite() && overflow_safe && (!lost(p, x, y) || alone(z, w)) && (!lost(q, z, w) || alone(x, y)) {
        return plain + t;
    }
    // Each product is split into its rounded value and the part that rounding drops, both exact.
    // The five parts are added one at a time into a list of values that do not overlap, smallest
    // first, whose sum is exact (Shewchuk's expansion).
    let product = |a: f64, b: f64| {
        let (a, b) = (Wide::of(a), Wide::of(b));
        let (hi, e) = (a.m * b.m, a.e.saturating_add(b.e));
        [Wide::new(hi, e), Wide::new(a.m.mul_add(b.m, 0.0 - hi), e)]
    };
    let ([p, p_lost], [q, q_lost]) = (product(x, y), product(z, w));
    let exact = [p, p_lost, q, q_lost, Wide::of(t)].into_iter().fold(Vec::with_capacity(10), grow);
    nearest(&exact)
}

/// `sums` with `part` added: values that do not overlap, smallest first, whose sum is exactly the
/// sum of `sums` and `part`. The last value is the largest and has the sign of the sum.
fn grow(sums: Vec<Wide>, part: Wide) -> Vec<Wide> {
    let mut carry = part;
    let mut kept = Vec::with_capacity(sums.len() + 1);
    for h in sums {
        let (sum, rest) = carry.two_sum(h);
        if rest.m != 0.0 {
            kept.push(rest);
        }
        carry = sum;
    }
    if carry.m != 0.0 {
        kept.push(carry);
    }
    kept
}

/// The sign of the exact sum of `sums`, `0.0 - v` and `offset`: 1, -1 or 0.
fn sign_past(sums: &[Wide], v: f64, offset: Wide) -> i32 {
    let rest = [Wide::of(0.0 - v), offset].into_iter().fold(sums.to_vec(), grow);
    match rest.last() {
        Some(w) if w.m > 0.0 => 1,
        Some(w) if w.m < 0.0 => -1,
        _ => 0,
    }
}

/// The number nearest the exact sum of `sums`, ties to the even one, and infinite when the sum
/// rounds to 2^1024 or past it. The sum of the values from the smallest up comes within a step of
/// it; the halfway points to the neighbors of that number are then compared with the exact sum, so
/// the result is rounded once.
fn nearest(sums: &[Wide]) -> f64 {
    let even = |v: f64| v.to_bits() & 1 == 0;
    let mut v = sums.iter().fold(Wide::of(0.0), |total, &h| total.two_sum(h).0).value().clamp(f64::MIN, f64::MAX);
    for _ in 0..4 {
        // Half the step to the next number up, and to the next number down. Past the largest
        // number the step is to 2^1024, which is 2^971 away.
        let up = if v == f64::MAX { f64::INFINITY } else { v.next_up() };
        let down = if v == f64::MIN { f64::NEG_INFINITY } else { v.next_down() };
        let half_up = if up.is_finite() { Wide::new(up - v, -1) } else { Wide::new(1.0, 970) };
        let half_down = if down.is_finite() { Wide::new(v - down, -1) } else { Wide::new(1.0, 970) };
        match sign_past(sums, v, Wide { m: 0.0 - half_up.m, e: half_up.e }) {
            1 => {
                v = up;
                if !v.is_finite() {
                    return v;
                }
                continue;
            }
            0 => return if even(v) { v } else { up },
            _ => {}
        }
        match sign_past(sums, v, half_down) {
            -1 => {
                v = down;
                if !v.is_finite() {
                    return v;
                }
            }
            0 => return if even(v) { v } else { down },
            _ => return v,
        }
    }
    v
}

pub(super) fn translate(dx: f64, dy: f64) -> Matrix {
    [1.0, 0.0, 0.0, 1.0, dx, dy]
}

/// A scale by `sx` and `sy` percent.
pub(super) fn scaled(sx: f64, sy: f64) -> Matrix {
    [sx / 100.0, 0.0, 0.0, sy / 100.0, 0.0, 0.0]
}

/// A turn of `degrees` counterclockwise, exact at quarter turns. The remainder keeps the sign of
/// `degrees` and is exact, so an angle next to a quarter turn is not rounded onto it.
pub(super) fn rotate(degrees: f64) -> Matrix {
    let turn = degrees % 360.0;
    let (sin, cos) = match turn {
        0.0 => (0.0, 1.0),
        90.0 | -270.0 => (1.0, 0.0),
        180.0 | -180.0 => (0.0, -1.0),
        270.0 | -90.0 => (-1.0, 0.0),
        _ => turn.to_radians().sin_cos(),
    };
    // `0.0 - sin` rather than `-sin`, so a turn of 0 gives 0 and not -0.
    [cos, sin, 0.0 - sin, cos, 0.0, 0.0]
}

fn translation_of(a: &[Data], at: usize) -> std::result::Result<Matrix, HostError> {
    Ok(translate(number(arg(a, at), "deltaX", 0.0)?, number(arg(a, at + 1), "deltaY", 0.0)?))
}

fn scale_of(a: &[Data], at: usize) -> std::result::Result<Matrix, HostError> {
    let sx = number(arg(a, at), "scaleX", 100.0)?;
    Ok(scaled(sx, number(arg(a, at + 1), "scaleY", sx)?))
}

fn rotation_of(a: &[Data], at: usize) -> std::result::Result<Matrix, HostError> {
    Ok(rotate(number(arg(a, at), "angle", 0.0)?))
}

fn identity(_: &Host, _: &[Data]) -> Answer {
    record(IDENTITY)
}

fn translation(_: &Host, a: &[Data]) -> Answer {
    record(translation_of(a, 1)?)
}

fn scale(_: &Host, a: &[Data]) -> Answer {
    record(scale_of(a, 1)?)
}

fn rotation(_: &Host, a: &[Data]) -> Answer {
    record(rotation_of(a, 1)?)
}

fn concatenate(_: &Host, a: &[Data]) -> Answer {
    record(then(matrix(arg(a, 1), "matrix")?, matrix(arg(a, 2), "secondMatrix")?))
}

fn concatenate_translation(_: &Host, a: &[Data]) -> Answer {
    record(then(matrix(arg(a, 1), "matrix")?, translation_of(a, 2)?))
}

fn concatenate_scale(_: &Host, a: &[Data]) -> Answer {
    record(then(matrix(arg(a, 1), "matrix")?, scale_of(a, 2)?))
}

fn concatenate_rotation(_: &Host, a: &[Data]) -> Answer {
    record(then(matrix(arg(a, 1), "matrix")?, rotation_of(a, 2)?))
}

/// A number as `m · 2^e` with `m` 0 or in `[0.5, 1)`, so products and quotients of such numbers
/// keep their value however far they go past the range of an `f64`.
#[derive(Clone, Copy)]
struct Wide {
    m: f64,
    e: i32,
}

/// `2^e`, for `e` from -1022 to 1023.
fn pow2(e: i32) -> f64 {
    f64::from_bits(u64::from((e + 1023).clamp(1, 2046).unsigned_abs()) << 52)
}

impl Wide {
    /// `x`, a finite number.
    fn of(x: f64) -> Self {
        if x == 0.0 {
            return Self { m: 0.0, e: 0 };
        }
        if x.abs() < f64::MIN_POSITIVE {
            let w = Self::of(x * pow2(54));
            return Self { m: w.m, e: w.e - 54 };
        }
        let bits = x.to_bits();
        let e = i32::try_from((bits >> 52) & 0x7ff).unwrap_or(0) - 1022;
        Self { m: f64::from_bits((bits & !(0x7ff << 52)) | (1022 << 52)), e }
    }

    /// `m · 2^e`, with `m` any finite number.
    fn new(m: f64, e: i32) -> Self {
        let w = Self::of(m);
        Self { m: w.m, e: w.e.saturating_add(e) }
    }

    fn div(self, o: Self) -> Self {
        Self::new(self.m / o.m, self.e.saturating_sub(o.e))
    }

    /// The sum and the part of it that rounding drops, so that `self + o` is exactly their sum. A
    /// number more than 2^110 times smaller than the other is returned whole as the dropped part.
    fn two_sum(self, o: Self) -> (Self, Self) {
        let (big, small) = if self.m == 0.0 || (o.m != 0.0 && o.e > self.e) { (o, self) } else { (self, o) };
        let gap = small.e.saturating_sub(big.e);
        if small.m == 0.0 || gap < -110 {
            return (big, small);
        }
        // Knuth's two-sum: `small` scaled to `big`'s power of 2 is exact, and so is the error.
        let s = small.m * pow2(gap);
        let sum = big.m + s;
        let back = sum - big.m;
        let rest = (big.m - (sum - back)) + (s - back);
        (Self::new(sum, big.e), Self::new(rest, big.e))
    }

    /// The nearest `f64`: infinite past its range, and 0 or a subnormal number below it.
    fn value(self) -> f64 {
        let (mut m, mut e) = (self.m, self.e);
        while e > 1000 && m.is_finite() {
            m *= pow2(1000);
            e -= 1000;
        }
        while e < -1000 && m != 0.0 {
            m *= pow2(-1000);
            e += 1000;
        }
        m * pow2(e)
    }
}

/// `a·d − b·c` with no product passing the range of a number; its `m` is 0 only when the two
/// products are equal. The products are taken whole with fused multiply-adds (Kahan's method), so
/// a difference that is small next to them keeps its value.
fn products_difference(a: f64, d: f64, b: f64, c: f64) -> Wide {
    let [a, b, c, d] = [a, b, c, d].map(Wide::of);
    let (ad, bc) = (a.e.saturating_add(d.e), b.e.saturating_add(c.e));
    if b.m == 0.0 || c.m == 0.0 {
        return Wide::new(a.m * d.m, ad);
    }
    if a.m == 0.0 || d.m == 0.0 {
        return Wide::new(0.0 - b.m * c.m, bc);
    }
    // `a·d` and `b·c` both lie in [0.25, 1) times a power of 2; one more than 2^120 times the other
    // decides the sign and size alone.
    let gap = bc.saturating_sub(ad);
    if gap < -120 {
        return Wide::new(a.m * d.m, ad);
    }
    if gap > 120 {
        return Wide::new(0.0 - b.m * c.m, bc);
    }
    let b = b.m * pow2(gap);
    let w = b * c.m;
    let err = (0.0 - b).mul_add(c.m, w);
    Wide::new(a.m.mul_add(d.m, 0.0 - w) + err, ad)
}

/// The determinant of `m`'s linear part.
fn determinant(m: Matrix) -> Wide {
    products_difference(m[0], m[3], m[1], m[2])
}

/// The square root of the size of `m`'s determinant: the factor by which `m` scales lengths on
/// average. It is infinite only when that factor is too large for a number.
pub(super) fn area_scale(m: Matrix) -> f64 {
    let d = determinant(m);
    let (mut v, mut e) = (d.m.abs(), d.e);
    if e % 2 != 0 {
        v *= 2.0;
        e -= 1;
    }
    Wide::new(v.sqrt(), e / 2).value()
}

pub(super) fn singular(m: Matrix) -> bool {
    determinant(m).m == 0.0
}

fn invert(_: &Host, a: &[Data]) -> Answer {
    let m = matrix(arg(a, 1), "matrix")?;
    let det = determinant(m);
    if det.m == 0.0 {
        return Err(HostError::Error("the matrix is singular and cannot be inverted".into()));
    }
    let [a, b, c, d, e, f] = m;
    // Each value is divided by the determinant before it is brought back into the range of a
    // number, and the translations are taken whole, so a finite inverse is not lost to an
    // intermediate value. Adding 0 turns -0 into 0.
    let over = |w: Wide| w.div(det).value() + 0.0;
    let plain = |v: f64| over(Wide::of(v));
    record([plain(d), plain(0.0 - b), plain(0.0 - c), plain(a), over(products_difference(c, f, d, e)), over(products_difference(b, e, a, f))])
}

fn is_equal(_: &Host, a: &[Data]) -> Answer {
    Ok(Data::Bool(matrix(arg(a, 1), "matrix")? == matrix(arg(a, 2), "secondMatrix")?))
}

fn is_singular(_: &Host, a: &[Data]) -> Answer {
    Ok(Data::Bool(singular(matrix(arg(a, 1), "matrix")?)))
}
