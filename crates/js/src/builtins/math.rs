//! The `Math` object (ECMA-262 5.1 section 15.8): its constants and functions, with the special
//! cases the section lists for NaN, signed zeros and infinities.

use super::{arg, data, method};
use crate::heap::{Class, NativeFn, OBJECT_BYTES, Object};
use crate::interp::{Interp, JsResult};
use crate::value::Value;

pub(super) fn install(interp: &mut Interp) -> JsResult<()> {
    interp.reserve_heap(OBJECT_BYTES)?;
    let proto = interp.intrinsics.object_proto;
    let math = interp.heap.alloc_object(Object::new(Class::Math, Some(proto)));
    let constants = [
        ("E", std::f64::consts::E),
        ("LN10", std::f64::consts::LN_10),
        ("LN2", std::f64::consts::LN_2),
        ("LOG2E", std::f64::consts::LOG2_E),
        ("LOG10E", std::f64::consts::LOG10_E),
        ("PI", std::f64::consts::PI),
        ("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2),
        ("SQRT2", std::f64::consts::SQRT_2),
    ];
    for (name, v) in constants {
        data(interp, math, name, Value::Number(v), 0)?;
    }
    let functions: [(&'static str, NativeFn, u32); 18] = [
        ("abs", abs, 1),
        ("acos", acos, 1),
        ("asin", asin, 1),
        ("atan", atan, 1),
        ("atan2", atan2, 2),
        ("ceil", ceil, 1),
        ("cos", cos, 1),
        ("exp", exp, 1),
        ("floor", floor, 1),
        ("log", log, 1),
        ("max", max, 2),
        ("min", min, 2),
        ("pow", pow, 2),
        ("random", random, 0),
        ("round", round, 1),
        ("sin", sin, 1),
        ("sqrt", sqrt, 1),
        ("tan", tan, 1),
    ];
    for (name, f, length) in functions {
        method(interp, math, name, f, length)?;
    }
    let global = interp.global;
    data(interp, global, "Math", Value::Object(math), crate::heap::HIDDEN)
}

/// Applies `f` to ToNumber of the first argument.
fn unary(interp: &mut Interp, args: &[Value], f: fn(f64) -> f64) -> JsResult<Value> {
    let x = interp.to_number(&arg(args, 0))?;
    Ok(Value::Number(f(x)))
}

fn abs(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    unary(interp, args, f64::abs)
}

fn acos(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    unary(interp, args, f64::acos)
}

fn asin(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    unary(interp, args, f64::asin)
}

fn atan(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    unary(interp, args, f64::atan)
}

fn ceil(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    unary(interp, args, f64::ceil)
}

fn cos(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    unary(interp, args, f64::cos)
}

fn exp(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    unary(interp, args, f64::exp)
}

fn floor(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    unary(interp, args, f64::floor)
}

fn log(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    unary(interp, args, f64::ln)
}

fn sin(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    unary(interp, args, f64::sin)
}

fn sqrt(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    unary(interp, args, f64::sqrt)
}

fn tan(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    unary(interp, args, f64::tan)
}

/// `Math.round` (section 15.8.2.15): the nearest integer, halves rounding up, with -0 for
/// arguments from -0.5 to -0.
fn round_number(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    if (-0.5..0.0).contains(&x) {
        return -0.0;
    }
    let floor = x.floor();
    // `x - floor` is exact, so a value just below a half (0.49999999999999994) does not round
    // up the way `floor(x + 0.5)` would.
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

fn round(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    unary(interp, args, round_number)
}

/// `Math.atan2` (section 15.8.2.5).
fn atan2(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let y = interp.to_number(&arg(args, 0))?;
    let x = interp.to_number(&arg(args, 1))?;
    Ok(Value::Number(y.atan2(x)))
}

/// `Math.pow` (section 15.8.2.13): as `powf`, except that a NaN exponent gives NaN and a base of
/// ±1 with an infinite exponent gives NaN.
fn pow_number(x: f64, y: f64) -> f64 {
    if y.is_nan() || (x.abs() == 1.0 && y.is_infinite()) {
        return f64::NAN;
    }
    x.powf(y)
}

fn pow(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let x = interp.to_number(&arg(args, 0))?;
    let y = interp.to_number(&arg(args, 1))?;
    Ok(Value::Number(pow_number(x, y)))
}

/// `Math.max` and `Math.min` (sections 15.8.2.11 and 15.8.2.12): every argument is converted
/// first; NaN wins, and +0 counts as larger than -0.
fn extreme(interp: &mut Interp, args: &[Value], max: bool) -> JsResult<Value> {
    let mut best = if max { f64::NEG_INFINITY } else { f64::INFINITY };
    let mut nan = false;
    for a in args {
        interp.use_fuel()?;
        let x = interp.to_number(a)?;
        if x.is_nan() {
            nan = true;
            continue;
        }
        let better = if x == best {
            if max { best.is_sign_negative() && x.is_sign_positive() } else { best.is_sign_positive() && x.is_sign_negative() }
        } else {
            (x > best) == max
        };
        if better {
            best = x;
        }
    }
    Ok(Value::Number(if nan { f64::NAN } else { best }))
}

fn max(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    extreme(interp, args, true)
}

fn min(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    extreme(interp, args, false)
}

/// `Math.random` (section 15.8.2.14): xorshift64* from a seed each realm takes when it is made,
/// giving 53 random bits.
fn random(interp: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let mut s = interp.random;
    s ^= s >> 12;
    s ^= s << 25;
    s ^= s >> 27;
    interp.random = s;
    let bits = s.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11;
    Ok(Value::Number(bits as f64 / (1u64 << 53) as f64))
}

#[cfg(test)]
mod tests {
    use super::{pow_number, round_number};

    #[test]
    fn round_and_pow_follow_the_special_cases() {
        assert_eq!(round_number(0.499_999_999_999_999_94), 0.0);
        assert_eq!(round_number(2.5), 3.0);
        assert_eq!(round_number(-2.5), -2.0);
        assert!(round_number(-0.5).is_sign_negative());
        assert!(round_number(-0.0).is_sign_negative());
        assert_eq!(round_number(4_503_599_627_370_495.5), 4_503_599_627_370_496.0);
        assert!(pow_number(1.0, f64::NAN).is_nan());
        assert!(pow_number(-1.0, f64::INFINITY).is_nan());
        assert_eq!(pow_number(f64::NAN, 0.0), 1.0);
        assert_eq!(pow_number(2.0, 10.0), 1024.0);
    }
}
