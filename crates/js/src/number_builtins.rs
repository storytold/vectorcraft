//! `Number` (ECMA-262 5.1 section 15.7): the constructor, its constants and the methods of
//! `Number.prototype`. The formats are in [`crate::number`].

use crate::builtins::{arg, constructor, data, method, native};
use crate::heap::{Class, NativeKind, Object};
use crate::interp::{ErrorType, Interp, JsResult};
use crate::number;
use crate::string_builtins::{reserve_units, to_integer};
use crate::value::{Str, Value};

pub(crate) fn install(interp: &mut Interp) -> JsResult<()> {
    let proto = interp.intrinsics.number_proto;
    let ctor = native(interp, "Number", number_ctor, 1, NativeKind::Plain, true)?;
    constructor(interp, "Number", ctor, proto)?;
    let constants = [
        ("MAX_VALUE", f64::MAX),
        ("MIN_VALUE", f64::from_bits(1)),
        ("NaN", f64::NAN),
        ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
        ("POSITIVE_INFINITY", f64::INFINITY),
    ];
    for (name, v) in constants {
        data(interp, ctor, name, Value::Number(v), 0)?;
    }
    method(interp, proto, "toString", to_string, 1)?;
    method(interp, proto, "toLocaleString", to_locale_string, 0)?;
    method(interp, proto, "valueOf", value_of, 0)?;
    method(interp, proto, "toFixed", to_fixed, 1)?;
    method(interp, proto, "toExponential", to_exponential, 1)?;
    method(interp, proto, "toPrecision", to_precision, 1)
}

/// `Number(value)` and `new Number(value)` (sections 15.7.1 and 15.7.2).
fn number_ctor(interp: &mut Interp, _: Value, args: &[Value], construct: bool) -> JsResult<Value> {
    let n = match args.first() {
        Some(v) => interp.to_number(v)?,
        None => 0.0,
    };
    if !construct {
        return Ok(Value::Number(n));
    }
    let proto = interp.intrinsics.number_proto;
    Ok(Value::Object(interp.heap.alloc_object(Object::new(Class::Number(n), Some(proto)))))
}

/// The number a `Number.prototype` method works on: a number, or a Number object's number.
fn this_number(interp: &mut Interp, this: &Value, name: &str) -> JsResult<f64> {
    match this {
        Value::Number(n) => Ok(*n),
        Value::Object(o) => match interp.heap.object(*o).map(|o| &o.class) {
            Some(Class::Number(n)) => Ok(*n),
            _ => Err(interp.type_error(&format!("Number.prototype.{name} was called on a value that is not a number"))),
        },
        _ => Err(interp.type_error(&format!("Number.prototype.{name} was called on a value that is not a number"))),
    }
}

fn text(interp: &mut Interp, s: &str) -> JsResult<Value> {
    reserve_units(interp, s.len())?;
    Ok(Value::String(Str::from(s)))
}

/// `toString(radix)` (section 15.7.4.2).
fn to_string(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let x = this_number(interp, &this, "toString")?;
    let radix = match arg(args, 0) {
        Value::Undefined => 10.0,
        r => to_integer(interp.to_number(&r)?),
    };
    if !(2.0..=36.0).contains(&radix) {
        return Err(interp.throw(ErrorType::Range, "the radix must be between 2 and 36"));
    }
    let s = if radix == 10.0 { number::to_string(x) } else { number::to_radix_string(x, radix as u32) };
    text(interp, &s)
}

fn to_locale_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let x = this_number(interp, &this, "toLocaleString")?;
    text(interp, &number::to_string(x))
}

fn value_of(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    this_number(interp, &this, "valueOf").map(Value::Number)
}

/// `toFixed(fractionDigits)` (section 15.7.4.5).
fn to_fixed(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let x = this_number(interp, &this, "toFixed")?;
    let f = to_integer(interp.to_number(&arg(args, 0))?);
    if !(0.0..=20.0).contains(&f) {
        return Err(interp.throw(ErrorType::Range, "toFixed() digits must be between 0 and 20"));
    }
    text(interp, &number::to_fixed(x, f as usize))
}

/// `toExponential(fractionDigits)` (section 15.7.4.6).
fn to_exponential(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let x = this_number(interp, &this, "toExponential")?;
    let digits = arg(args, 0);
    let f = to_integer(interp.to_number(&digits)?);
    if !x.is_finite() {
        return text(interp, &number::to_string(x));
    }
    let f = match digits {
        Value::Undefined => None,
        _ if !(0.0..=20.0).contains(&f) => return Err(interp.throw(ErrorType::Range, "toExponential() digits must be between 0 and 20")),
        _ => Some(f as usize),
    };
    text(interp, &number::to_exponential(x, f))
}

/// `toPrecision(precision)` (section 15.7.4.7).
fn to_precision(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let x = this_number(interp, &this, "toPrecision")?;
    let precision = arg(args, 0);
    if matches!(precision, Value::Undefined) {
        return text(interp, &number::to_string(x));
    }
    let p = to_integer(interp.to_number(&precision)?);
    if !x.is_finite() {
        return text(interp, &number::to_string(x));
    }
    if !(1.0..=21.0).contains(&p) {
        return Err(interp.throw(ErrorType::Range, "toPrecision() argument must be between 1 and 21"));
    }
    text(interp, &number::to_precision(x, p as usize))
}
