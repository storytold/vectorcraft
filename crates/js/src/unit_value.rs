//! `UnitValue`, the measurement class the help page
//! <https://extendscript.docsforadobe.dev/extendscript-tools-features/specifying-measurement-values/>
//! describes, added to a realm by [`crate::Realm::add_unit_values`].
//!
//! An object holds a magnitude, a unit and the base that pixels and percentages convert through.
//! The length units convert through exact ratios to the inch; a pixel is one base and 100% is one
//! base. The base is an absolute length greater than 0, copied into each new object from
//! `UnitValue.baseUnit` (1 point, which is 1/72 inch, until a script sets it), and an object's own `baseUnit` may
//! replace it. A unit the table does not list, text that is not a number followed by a unit, and a
//! magnitude that is not finite make an invalid object: its `type` is `?`, its `value` 0, and it
//! cannot be converted or calculated with.
//!
//! In a realm with `UnitValue`, the operators take its objects as follows. `+`, `-`, `*`, `/` and
//! `%` with a `UnitValue` and a number calculate with the magnitude and give a new `UnitValue` in
//! its unit; with two, the right one is converted to the left one's unit first, with the left one's
//! base for `px` and `%`, and the result is in the left one's unit. `<`, `<=`, `==` and `!=` compare magnitudes the same way, `>` is the
//! negation of `<=` and `>=` the negation of `<`. Unary `+` and `-` give the magnitude and its
//! negation, `~` inverts the magnitude's 32-bit integer, and `!` is true for a magnitude that is not
//! 0. Any other operand, and every other operator, works as in standard JavaScript, as do all
//! operators in a realm without `UnitValue`. A calculation whose result is not finite (a division
//! by 0 included), a comparison with a number that is not finite, and a conversion of an invalid
//! object or any of these operators applied to one throw a `RangeError`; a unit the table does not list throws an
//! `Error`.

use crate::ast::BinaryOp;
use crate::builtins::{arg, constructor, method, native};
use crate::bytecode::Op;
use crate::heap::{CONFIGURABLE, Class, NativeFn, NativeKind, OBJECT_BYTES, ObjId, Object, Prop, UNIT_VALUE_BYTES, key};
use crate::interp::{Abrupt, ErrorType, Interp, JsResult};
use crate::value::{Value, is_str_space, number_to_str, string_to_number, to_int32};

/// The units of the table, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unit {
    In,
    Ft,
    Yd,
    Mi,
    Mm,
    Cm,
    M,
    Km,
    Pt,
    Pc,
    Tpt,
    Tpc,
    Ci,
    Px,
    Percent,
}

/// Each unit with its abbreviation, singular and plural names, and its length in inches as a ratio
/// (0 for the units that convert through the base).
const UNITS: [(Unit, &str, &str, &str, u64, u64); 15] = [
    (Unit::In, "in", "inch", "inches", 1, 1),
    (Unit::Ft, "ft", "foot", "feet", 12, 1),
    (Unit::Yd, "yd", "yard", "yards", 36, 1),
    (Unit::Mi, "mi", "mile", "miles", 63_360, 1),
    (Unit::Mm, "mm", "millimeter", "millimeters", 5, 127),
    (Unit::Cm, "cm", "centimeter", "centimeters", 50, 127),
    (Unit::M, "m", "meter", "meters", 5_000, 127),
    (Unit::Km, "km", "kilometer", "kilometers", 5_000_000, 127),
    (Unit::Pt, "pt", "point", "points", 1, 72),
    (Unit::Pc, "pc", "pica", "picas", 1, 6),
    (Unit::Tpt, "tpt", "traditional point", "traditional points", 100, 7_227),
    (Unit::Tpc, "tpc", "traditional pica", "traditional picas", 1_200, 7_227),
    (Unit::Ci, "ci", "cicero", "ciceros", 111, 625),
    (Unit::Px, "px", "pixel", "pixels", 0, 0),
    (Unit::Percent, "%", "percent", "percent", 0, 0),
];

/// The longest unit name in the table, in code units.
const LONGEST_NAME: usize = 18;

/// The longest text the constructor reads as a magnitude and a unit, in code units; longer text
/// makes an invalid object without being read.
const LONGEST_TEXT: usize = 64;

impl Unit {
    fn row(self) -> Option<&'static (Unit, &'static str, &'static str, &'static str, u64, u64)> {
        UNITS.iter().find(|row| row.0 == self)
    }

    fn abbreviation(self) -> &'static str {
        self.row().map_or("?", |row| row.1)
    }

    /// The unit's length in inches as a ratio, `None` for pixels and percentages.
    fn inches(self) -> Option<(u64, u64)> {
        let &(_, _, _, _, n, d) = self.row()?;
        (n != 0).then_some((n, d))
    }

    /// The unit with this abbreviation, singular or plural name.
    fn named(units: &[u16]) -> Option<Unit> {
        if units.is_empty() || units.len() > LONGEST_NAME {
            return None;
        }
        let name = String::from_utf16(units).ok()?;
        UNITS.iter().find(|(_, a, s, p, _, _)| name == *a || name == *s || name == *p).map(|row| row.0)
    }
}

/// The length that pixels and percentages convert through: a finite magnitude greater than 0 in a
/// unit with a length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Base {
    value: f64,
    unit: Unit,
}

/// 1/72 inch, kept as 1 point so that conversions through it are exact.
const DEFAULT_BASE: Base = Base { value: 1.0, unit: Unit::Pt };

/// What a `UnitValue` object holds. `unit` is `None` for an invalid object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Measure {
    value: f64,
    unit: Option<Unit>,
    base: Base,
}

/// The prototype of `UnitValue` objects and the current `UnitValue.baseUnit`, kept by the realm.
#[derive(Clone, Debug)]
pub(crate) struct Classes {
    proto: ObjId,
    base: Base,
}

impl Classes {
    /// The object a collection keeps.
    pub(crate) fn object(&self) -> ObjId {
        self.proto
    }
}

fn greatest_common_divisor(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// The ratio of a length in unit `from` to the same length in unit `to`, as a reduced fraction.
fn ratio(from: Unit, to: Unit) -> (f64, f64) {
    let (from, to) = (from.inches().unwrap_or((1, 1)), to.inches().unwrap_or((1, 1)));
    // The products stay below 2^53, so the ratio's terms are exact.
    let (n, d) = (from.0 * to.1, from.1 * to.0);
    let g = greatest_common_divisor(n, d).max(1);
    #[allow(clippy::cast_precision_loss)]
    (((n / g) as f64), ((d / g) as f64))
}

/// `x` as a significand in [0.5, 1) and a power of 2; zero, infinities and NaN as themselves.
fn split(x: f64) -> (f64, i32) {
    if x == 0.0 || !x.is_finite() {
        return (x, 0);
    }
    let (x, shift) = if x.is_normal() { (x, 0) } else { (x * 2f64.powi(64), -64) };
    let bits = x.to_bits();
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1022;
    (f64::from_bits((bits & !(0x7ff << 52)) | (1022 << 52)), exponent + shift)
}

/// The product of `factors` divided by each of `divisors`, in that order. Each step rounds as
/// plain multiplication and division do, but no step overflows or reaches 0 on the way, so only
/// a result outside the range of numbers is infinite or 0.
fn product(factors: &[f64], divisors: &[f64]) -> f64 {
    let (mut m, mut e) = (1.0, 0);
    for (&x, divide) in factors.iter().map(|x| (x, false)).chain(divisors.iter().map(|x| (x, true))) {
        let (xm, xe) = split(x);
        let (rm, re) = split(if divide { m / xm } else { m * xm });
        (m, e) = (rm, if divide { e - xe } else { e + xe } + re);
    }
    while e > 1000 {
        (m, e) = (m * 2f64.powi(1000), e - 1000);
    }
    while e < -1000 {
        (m, e) = (m * 2f64.powi(-1000), e + 1000);
    }
    m * 2f64.powi(e)
}

fn invalid(i: &mut Interp, what: &str) -> Abrupt {
    i.throw(ErrorType::Range, &format!("{what}: an invalid UnitValue cannot be converted or calculated with"))
}

fn finite_value(i: &mut Interp, what: &str) -> Abrupt {
    i.throw(ErrorType::Range, &format!("{what}: the value must be a finite number"))
}

fn not_finite(i: &mut Interp, what: &str) -> Abrupt {
    i.throw(ErrorType::Range, &format!("{what}: the result is not a finite number"))
}

/// The magnitude of `m` in unit `to`, through `m`'s own base.
fn convert(i: &mut Interp, m: &Measure, to: Unit, what: &str) -> JsResult<f64> {
    let Some(from) = m.unit else {
        return Err(invalid(i, what));
    };
    let base = m.base;
    let r = match (from, to) {
        (a, b) if a == b => m.value,
        (Unit::Px, Unit::Percent) => m.value * 100.0,
        (Unit::Percent, Unit::Px) => m.value / 100.0,
        (Unit::Px | Unit::Percent, _) => {
            let (n, d) = ratio(base.unit, to);
            if from == Unit::Px { product(&[m.value, base.value, n], &[d]) } else { product(&[m.value, base.value, n], &[100.0, d]) }
        }
        (_, Unit::Px | Unit::Percent) => {
            let (n, d) = ratio(from, base.unit);
            if to == Unit::Px { product(&[m.value, n], &[d, base.value]) } else { product(&[m.value, n, 100.0], &[d, base.value]) }
        }
        (_, _) => {
            let (n, d) = ratio(from, to);
            product(&[m.value, n], &[d])
        }
    };
    if r.is_finite() { Ok(r) } else { Err(not_finite(i, what)) }
}

/// The measure of `v` when it is a `UnitValue` object.
pub(crate) fn measure(i: &Interp, v: &Value) -> Option<Measure> {
    let o = v.as_object()?;
    match &i.heap.object(o)?.class {
        Class::UnitValue(m) => Some(**m),
        _ => None,
    }
}

fn this_measure(i: &mut Interp, this: &Value, what: &str) -> JsResult<(ObjId, Measure)> {
    if let (Some(o), Some(m)) = (this.as_object(), measure(i, this)) {
        return Ok((o, m));
    }
    Err(i.type_error(&format!("{what}: the object is not a UnitValue")))
}

fn store(i: &mut Interp, o: ObjId, m: Measure) {
    if let Some(obj) = i.heap.object_mut(o)
        && let Class::UnitValue(slot) = &mut obj.class
    {
        **slot = m;
    }
}

fn classes(i: &Interp) -> JsResult<Classes> {
    i.unit_values.clone().ok_or_else(|| Abrupt::Internal("UnitValue is not installed".into()))
}

/// A new `UnitValue` object holding `m`.
fn make(i: &mut Interp, m: Measure) -> JsResult<Value> {
    let proto = classes(i)?.proto;
    i.reserve_heap(OBJECT_BYTES + UNIT_VALUE_BYTES)?;
    i.heap.bytes = i.heap.bytes.saturating_add(UNIT_VALUE_BYTES);
    Ok(Value::Object(i.heap.alloc_object(Object::new(Class::UnitValue(Box::new(m)), Some(proto)))))
}

/// The unit `v` names, or an `Error` naming `what`.
fn unit_arg(i: &mut Interp, v: &Value, what: &str) -> JsResult<Unit> {
    let s = i.to_str(v)?;
    let unit = if s.len() > LONGEST_NAME { None } else { Unit::named(&s.flat()) };
    unit.ok_or_else(|| i.throw(ErrorType::Error, &format!("{what}: the unit is not known")))
}

/// A magnitude followed by a unit, with spaces around and between them allowed: `12 pt`,
/// `-1.5e2 traditional points`, `10%`.
fn parse_text(units: &[u16]) -> Option<(f64, Unit)> {
    let start = units.iter().position(|&u| !is_str_space(u)).unwrap_or(units.len());
    let end = units.iter().rposition(|&u| !is_str_space(u)).map_or(start, |e| e + 1);
    let text = units.get(start..end)?;
    let digit = |u: u16| (u16::from(b'0')..=u16::from(b'9')).contains(&u);
    let at = |k: usize| text.get(k).copied().unwrap_or(0);
    let mut k = 0;
    if at(k) == u16::from(b'+') || at(k) == u16::from(b'-') {
        k += 1;
    }
    let mut digits = 0;
    while digit(at(k)) {
        k += 1;
        digits += 1;
    }
    if at(k) == u16::from(b'.') {
        k += 1;
        while digit(at(k)) {
            k += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if at(k) == u16::from(b'e') || at(k) == u16::from(b'E') {
        let mut e = k + 1;
        if at(e) == u16::from(b'+') || at(e) == u16::from(b'-') {
            e += 1;
        }
        if digit(at(e)) {
            while digit(at(e)) {
                e += 1;
            }
            k = e;
        }
    }
    let (number, rest) = text.split_at_checked(k)?;
    let value = string_to_number(number);
    let u = rest.iter().position(|&u| !is_str_space(u)).unwrap_or(rest.len());
    let unit = Unit::named(rest.get(u..)?)?;
    value.is_finite().then_some((value, unit))
}

/// `new UnitValue(value, unit)`, `new UnitValue("value unit")`, `new UnitValue(unitValue)` and the
/// same without `new`.
fn unit_value_ctor(i: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let base = classes(i)?.base;
    let bad = Measure { value: 0.0, unit: None, base };
    let first = arg(args, 0);
    let m = if args.len() >= 2 {
        let value = i.to_number(&first)?;
        let s = i.to_str(&arg(args, 1))?;
        let unit = if s.len() > LONGEST_NAME { None } else { Unit::named(&s.flat()) };
        match unit {
            Some(unit) if value.is_finite() => Measure { value, unit: Some(unit), base },
            _ => bad,
        }
    } else if let Some(m) = measure(i, &first) {
        m
    } else if matches!(first, Value::Undefined | Value::Number(_)) {
        bad
    } else {
        let s = i.to_str(&first)?;
        match if s.len() > LONGEST_TEXT { None } else { parse_text(&s.flat()) } {
            Some((value, unit)) => Measure { value, unit: Some(unit), base },
            None => bad,
        }
    };
    make(i, m)
}

fn get_value(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (_, m) = this_measure(i, &this, "UnitValue.value")?;
    Ok(Value::Number(m.value))
}

fn set_value(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_measure(i, &this, "UnitValue.value")?;
    let value = i.to_number(&arg(args, 0))?;
    if !value.is_finite() {
        return Err(finite_value(i, "UnitValue.value"));
    }
    // The conversion may have run script code that changed the object, so it is read again.
    let (_, mut m) = this_measure(i, &this, "UnitValue.value")?;
    m.value = value;
    store(i, o, m);
    Ok(Value::Undefined)
}

fn get_type(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (_, m) = this_measure(i, &this, "UnitValue.type")?;
    Ok(Value::str(m.unit.map_or("?", Unit::abbreviation)))
}

fn set_type(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_measure(i, &this, "UnitValue.type")?;
    let unit = unit_arg(i, &arg(args, 0), "UnitValue.type")?;
    let (_, mut m) = this_measure(i, &this, "UnitValue.type")?;
    m.unit = Some(unit);
    store(i, o, m);
    Ok(Value::Undefined)
}

/// The base `v` gives: `null` gives the default, and any other value must be a valid `UnitValue`
/// with a length greater than 0.
fn base_arg(i: &mut Interp, v: &Value, null: Base, what: &str) -> JsResult<Base> {
    if matches!(v, Value::Null) {
        return Ok(null);
    }
    match measure(i, v) {
        Some(Measure { value, unit: Some(unit), .. }) if unit.inches().is_some() && value.is_finite() && value > 0.0 => Ok(Base { value, unit }),
        _ => Err(i.throw(ErrorType::Range, &format!("{what}: the base must be null or a UnitValue of a length greater than 0"))),
    }
}

fn base_object(i: &mut Interp, base: Base) -> JsResult<Value> {
    let class = classes(i)?.base;
    make(i, Measure { value: base.value, unit: Some(base.unit), base: class })
}

fn get_base(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (_, m) = this_measure(i, &this, "UnitValue.baseUnit")?;
    base_object(i, m.base)
}

fn set_base(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, mut m) = this_measure(i, &this, "UnitValue.baseUnit")?;
    let class = classes(i)?.base;
    m.base = base_arg(i, &arg(args, 0), class, "UnitValue.baseUnit")?;
    store(i, o, m);
    Ok(Value::Undefined)
}

fn get_class_base(i: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let base = classes(i)?.base;
    base_object(i, base)
}

fn set_class_base(i: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let base = base_arg(i, &arg(args, 0), DEFAULT_BASE, "UnitValue.baseUnit")?;
    if let Some(c) = i.unit_values.as_mut() {
        c.base = base;
    }
    Ok(Value::Undefined)
}

/// `unitValue.as(unit)`: the magnitude in `unit`, leaving the object as it is.
fn unit_as(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    this_measure(i, &this, "UnitValue.as")?;
    let unit = unit_arg(i, &arg(args, 0), "UnitValue.as")?;
    let (_, m) = this_measure(i, &this, "UnitValue.as")?;
    convert(i, &m, unit, "UnitValue.as").map(Value::Number)
}

/// `unitValue.convert(unit)`: changes the object to `unit` and gives `true`. A failure throws and
/// leaves the object as it was.
fn unit_convert(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    this_measure(i, &this, "UnitValue.convert")?;
    let unit = unit_arg(i, &arg(args, 0), "UnitValue.convert")?;
    let (o, mut m) = this_measure(i, &this, "UnitValue.convert")?;
    m.value = convert(i, &m, unit, "UnitValue.convert")?;
    m.unit = Some(unit);
    store(i, o, m);
    Ok(Value::Bool(true))
}

/// `unitValue.toString()`: the magnitude and the abbreviation, `12 pt`; an invalid object gives
/// `UnitValue 0.00000`.
fn unit_to_string(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (_, m) = this_measure(i, &this, "UnitValue.toString")?;
    Ok(Value::str(&match m.unit {
        Some(unit) => format!("{} {}", number_to_str(m.value).to_rust_string(), unit.abbreviation()),
        None => format!("UnitValue {:.5}", m.value),
    }))
}

/// Defines the accessor `name` on `on`, not enumerable.
fn accessor(i: &mut Interp, on: ObjId, name: &'static str, get: NativeFn, set: NativeFn) -> JsResult<()> {
    let g = native(i, name, get, 0, NativeKind::Plain, false)?;
    i.rooted(&[Value::Object(g)], |i| {
        let s = native(i, name, set, 1, NativeKind::Plain, false)?;
        i.define(on, key(name), Prop::Accessor { get: Some(g), set: Some(s), flags: CONFIGURABLE })
    })
}

/// Adds the global constructor `UnitValue` ([`crate::Realm::add_unit_values`]). A second call
/// after one that succeeded changes nothing; after one that failed, it installs everything again.
pub(crate) fn install(i: &mut Interp) -> JsResult<()> {
    install_with(i, define_members)
}

/// [`install`] with `define` defining the members on the new prototype. When `define` fails, the
/// realm is left without the operators, and the global `UnitValue` as it was before the call.
pub(crate) fn install_with(i: &mut Interp, define: impl FnOnce(&mut Interp, ObjId) -> JsResult<()>) -> JsResult<()> {
    if i.unit_values.is_some() {
        return Ok(());
    }
    let global = i.global;
    let name = key("UnitValue");
    let previous = i.get_own_property(global, &name)?;
    let held: Vec<Value> = match &previous {
        Some(Prop::Data { value, .. }) => vec![value.clone()],
        Some(Prop::Accessor { get, set, .. }) => get.iter().chain(set.iter()).map(|&o| Value::Object(o)).collect(),
        None => Vec::new(),
    };
    i.rooted(&held, |i| {
        let object_proto = i.intrinsics.object_proto;
        i.reserve_heap(OBJECT_BYTES)?;
        let proto = i.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));
        // `unit_values` roots the prototype while the members are defined, and turns the operators on.
        i.unit_values = Some(Classes { proto, base: DEFAULT_BASE });
        let defined = define(i, proto);
        if defined.is_err() {
            i.unit_values = None;
            match previous {
                Some(p) => i.define(global, name, p)?,
                None => {
                    i.object_mut(global)?.props.remove(&name);
                }
            }
        }
        defined
    })
}

/// Defines the constructor, its `baseUnit` and the prototype's members.
fn define_members(i: &mut Interp, proto: ObjId) -> JsResult<()> {
    let ctor = native(i, "UnitValue", unit_value_ctor, 2, NativeKind::Plain, true)?;
    constructor(i, "UnitValue", ctor, proto)?;
    accessor(i, ctor, "baseUnit", get_class_base, set_class_base)?;
    accessor(i, proto, "value", get_value, set_value)?;
    accessor(i, proto, "type", get_type, set_type)?;
    accessor(i, proto, "baseUnit", get_base, set_base)?;
    method(i, proto, "as", unit_as, 1)?;
    method(i, proto, "convert", unit_convert, 1)?;
    method(i, proto, "toString", unit_to_string, 0)
}

/// Unary `op` on `v` when `v` is a `UnitValue`; `None` otherwise.
pub(crate) fn unary(i: &mut Interp, op: Op, v: &Value) -> JsResult<Option<Value>> {
    let Some(m) = measure(i, v) else { return Ok(None) };
    if !matches!(op, Op::Neg | Op::Plus | Op::BitNot | Op::Not) {
        return Ok(None);
    }
    if m.unit.is_none() {
        return Err(invalid(i, "UnitValue"));
    }
    Ok(Some(match op {
        Op::Neg => Value::Number(-m.value),
        Op::Plus => Value::Number(m.value),
        Op::BitNot => Value::Number(f64::from(!to_int32(m.value))),
        _ => Value::Bool(m.value != 0.0),
    }))
}

/// Binary `op` on `a` and `b` when one is a `UnitValue` and the other a number or a `UnitValue`,
/// for the arithmetic and comparison operators; `None` otherwise.
pub(crate) fn binary(i: &mut Interp, op: BinaryOp, a: &Value, b: &Value) -> JsResult<Option<Value>> {
    let arithmetic = matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod);
    let comparison = matches!(op, BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq | BinaryOp::Eq | BinaryOp::NotEq);
    if !arithmetic && !comparison {
        return Ok(None);
    }
    let what = "UnitValue";
    let (x, y, owner) = match (measure(i, a), measure(i, b), a, b) {
        (Some(l), Some(r), _, _) => {
            let Some(unit) = l.unit else {
                return Err(invalid(i, what));
            };
            // Pixels and percentages of the left object are measured with its own base.
            let y = match unit {
                Unit::Px | Unit::Percent if r.base != l.base || !matches!(r.unit, Some(Unit::Px | Unit::Percent)) => match r.unit {
                    Some(from @ (Unit::Px | Unit::Percent)) => {
                        let (n, d) = ratio(r.base.unit, l.base.unit);
                        let mut factors = vec![r.value, r.base.value, n];
                        let mut divisors = vec![d, l.base.value];
                        if from == Unit::Percent {
                            divisors.push(100.0);
                        }
                        if unit == Unit::Percent {
                            factors.push(100.0);
                        }
                        let y = product(&factors, &divisors);
                        if !y.is_finite() {
                            return Err(not_finite(i, what));
                        }
                        y
                    }
                    _ => convert(i, &Measure { base: l.base, ..r }, unit, what)?,
                },
                _ => convert(i, &r, unit, what)?,
            };
            (l.value, y, l)
        }
        (Some(l), None, _, Value::Number(n)) => (l.value, *n, l),
        (None, Some(r), Value::Number(n), _) => (*n, r.value, r),
        (l, r, _, _) => {
            if l.or(r).is_some_and(|m| m.unit.is_none()) {
                return Err(invalid(i, what));
            }
            return Ok(None);
        }
    };
    if owner.unit.is_none() {
        return Err(invalid(i, what));
    }
    if comparison {
        if !x.is_finite() || !y.is_finite() {
            return Err(finite_value(i, what));
        }
        // Without NaN, `x > y` is the negation of `x <= y` and `x >= y` the negation of `x < y`.
        return Ok(Some(Value::Bool(match op {
            BinaryOp::Lt => x < y,
            BinaryOp::LtEq => x <= y,
            BinaryOp::Gt => x > y,
            BinaryOp::GtEq => x >= y,
            BinaryOp::Eq => x == y,
            _ => x != y,
        })));
    }
    // A division by 0 gives an infinite result or NaN, which is refused below.
    let r = match op {
        BinaryOp::Add => x + y,
        BinaryOp::Sub => x - y,
        BinaryOp::Mul => x * y,
        BinaryOp::Div => x / y,
        _ => x % y,
    };
    if !r.is_finite() {
        return Err(not_finite(i, what));
    }
    make(i, Measure { value: r, ..owner }).map(Some)
}
