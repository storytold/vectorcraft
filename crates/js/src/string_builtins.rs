//! `String` (ECMA-262 5.1 section 15.5): the constructor, `String.fromCharCode` and the methods of
//! `String.prototype`, with `substr` from Annex B.
//!
//! The methods read a string's code units once (flattening a rope, whose flat copy counts as a new
//! string), take a unit of fuel for every [`UNITS_PER_FUEL`] code units they read or write, and
//! reserve each new string's bytes on the heap before building it, so a method that would make a
//! string past the heap limit ends the run instead of allocating it.

use std::rc::Rc;

use crate::LimitKind;
use crate::builtins::{arg, constructor, method, native};
use crate::heap::{Class, NativeKind, OPEN, ObjId, Object, PROP_BYTES, Prop, string_bytes};
use crate::interp::{Abrupt, HeldBytes, Interp, JsResult};
use crate::regexp_builtins::{exec_raw, new_regexp_from, regexp_of};
use crate::regexp_vm::{Matcher, NONE, use_fuel_n};
use crate::value::{Str, Value, index_key, is_str_space, to_uint32};

/// Code units a unit of fuel pays for when a built-in reads or writes text.
pub(crate) const UNITS_PER_FUEL: usize = 16;

pub(crate) fn install(interp: &mut Interp) -> JsResult<()> {
    let proto = interp.intrinsics.string_proto;
    let ctor = native(interp, "String", string_ctor, 1, NativeKind::Plain, true)?;
    constructor(interp, "String", ctor, proto)?;
    method(interp, ctor, "fromCharCode", from_char_code, 1)?;
    let methods: [(&'static str, crate::heap::NativeFn, u32); 20] = [
        ("toString", to_string, 0),
        ("valueOf", to_string, 0),
        ("charAt", char_at, 1),
        ("charCodeAt", char_code_at, 1),
        ("concat", concat, 1),
        ("indexOf", index_of, 1),
        ("lastIndexOf", last_index_of, 1),
        ("localeCompare", locale_compare, 1),
        ("match", match_, 1),
        ("replace", replace, 2),
        ("search", search, 1),
        ("slice", slice, 2),
        ("split", split, 2),
        ("substring", substring, 2),
        ("substr", substr, 2),
        ("toLowerCase", to_lower_case, 0),
        ("toLocaleLowerCase", to_lower_case, 0),
        ("toUpperCase", to_upper_case, 0),
        ("toLocaleUpperCase", to_upper_case, 0),
        ("trim", trim, 0),
    ];
    for (name, f, len) in methods {
        method(interp, proto, name, f, len)?;
    }
    Ok(())
}

// ---- helpers the other text built-ins share ----

/// ToInteger (section 9.4) applied to a number.
pub(crate) fn to_integer(n: f64) -> f64 {
    if n.is_nan() {
        0.0
    } else if n.is_infinite() {
        n
    } else {
        n.trunc() + 0.0
    }
}

/// Takes the fuel for reading or writing `n` code units.
pub(crate) fn charge_units(interp: &mut Interp, n: usize) -> JsResult<()> {
    use_fuel_n(interp, (n / UNITS_PER_FUEL) as u64 + 1)
}

/// Reserves the heap bytes of a new string of `len` code units before it is built, ending the
/// run with the heap limit when it would not fit. Everything the caller still needs must be
/// reachable from the roots.
pub(crate) fn reserve_units(interp: &mut Interp, len: usize) -> JsResult<()> {
    reserve_string(interp, len)?;
    charge_units(interp, len)
}

/// [`reserve_units`] without the fuel, for a caller that took it as it read the units.
fn reserve_string(interp: &mut Interp, len: usize) -> JsResult<()> {
    check_string_room(interp, len)?;
    let bytes = string_bytes(len);
    interp.heap.bytes = interp.heap.bytes.saturating_add(bytes);
    Ok(())
}

/// Ends the run with the heap limit when a new string of `len` code units would not fit, after
/// a collection if one is needed. Everything the caller still needs must be reachable from the
/// roots.
fn check_string_room(interp: &mut Interp, len: usize) -> JsResult<()> {
    if len > interp.limits.heap_bytes / 4 {
        return Err(Abrupt::Limit(LimitKind::Heap));
    }
    interp.reserve_heap(string_bytes(len))
}

/// Pushes `item` onto `list`, first holding the bytes of the list's new capacity when it must
/// grow.
fn push_held<T>(interp: &mut Interp, held: &mut HeldBytes, list: &mut Vec<T>, item: T) -> JsResult<()> {
    if list.len() == list.capacity() {
        let more = list.capacity().max(4);
        held.add(interp, more.saturating_mul(size_of::<T>()))?;
        list.reserve_exact(more);
    }
    list.push(item);
    Ok(())
}

/// The code units of `s`. Flattening a rope makes a copy of its units, so for a rope that copy's
/// bytes are reserved and added to the heap, and its units charged as fuel, first.
pub(crate) fn flat_charged(interp: &mut Interp, s: &Str) -> JsResult<Rc<[u16]>> {
    if s.is_rope() {
        reserve_units(interp, s.len())?;
    }
    Ok(s.flat())
}

/// A new string holding `units[start..end]`.
pub(crate) fn substring_value(interp: &mut Interp, units: &[u16], start: usize, end: usize) -> JsResult<Value> {
    let part = units.get(start..end.max(start)).unwrap_or_default();
    reserve_units(interp, part.len())?;
    Ok(Value::String(Str::from_units(part.to_vec())))
}

/// Runs `f`, then drops the temporary roots it added.
pub(crate) fn rooted<T>(interp: &mut Interp, f: impl FnOnce(&mut Interp) -> JsResult<T>) -> JsResult<T> {
    let mark = interp.temp_roots.len();
    let r = f(interp);
    interp.temp_roots.truncate(mark);
    r
}

/// A new, empty array, kept in the temporary roots: call it inside [`rooted`].
pub(crate) fn new_array(interp: &mut Interp) -> ObjId {
    let proto = interp.intrinsics.array_proto;
    let a = interp.heap.alloc_object(Object::new(Class::Array { length: 0, length_writable: true }, Some(proto)));
    interp.temp_roots.push(Value::Object(a));
    a
}

/// Sets element `i` of an array made by [`new_array`], reserving its bytes first.
pub(crate) fn set_element(interp: &mut Interp, a: ObjId, i: usize, v: Value) -> JsResult<()> {
    let i = u32::try_from(i).map_err(|_| Abrupt::Limit(LimitKind::Heap))?;
    interp.temp_roots.push(v.clone());
    let reserved = interp.reserve_heap(PROP_BYTES + string_bytes(10));
    interp.temp_roots.pop();
    reserved?;
    interp.use_fuel()?;
    interp.define(a, index_key(i), Prop::Data { value: v, flags: OPEN })
}

/// CheckObjectCoercible on `this`, then ToString.
pub(crate) fn this_str(interp: &mut Interp, this: &Value, method: &str) -> JsResult<Str> {
    if this.is_nullish() {
        let message = format!("String.prototype.{method} was called on null or undefined");
        return Err(interp.type_error(&message));
    }
    interp.to_str(this)
}

/// ToInteger of an argument.
fn int_arg(interp: &mut Interp, args: &[Value], i: usize) -> JsResult<f64> {
    let n = interp.to_number(&arg(args, i))?;
    Ok(to_integer(n))
}

/// Clamps an integer to `0..=len`.
fn clamp(n: f64, len: usize) -> usize {
    if n <= 0.0 {
        0
    } else if n >= len as f64 {
        len
    } else {
        n as usize
    }
}

/// The first index at `from` or after where `needle` occurs in `hay`. Every position tried takes
/// fuel.
fn find(interp: &mut Interp, hay: &[u16], needle: &[u16], from: usize) -> JsResult<Option<usize>> {
    if needle.len() > hay.len() {
        return Ok(None);
    }
    let last = hay.len() - needle.len();
    let per = (needle.len() / UNITS_PER_FUEL) as u64 + 1;
    let mut steps = 0u64;
    let mut i = from;
    let mut found = None;
    while i <= last {
        steps += per;
        if steps >= 1024 {
            use_fuel_n(interp, std::mem::take(&mut steps))?;
        }
        if hay.get(i..i + needle.len()) == Some(needle) {
            found = Some(i);
            break;
        }
        i += 1;
    }
    use_fuel_n(interp, steps)?;
    Ok(found)
}

// ---- the constructor ----

/// `String(value)` and `new String(value)` (sections 15.5.1 and 15.5.2).
fn string_ctor(interp: &mut Interp, _: Value, args: &[Value], construct: bool) -> JsResult<Value> {
    let s = match args.first() {
        Some(v) => interp.to_str(v)?,
        None => Str::from(""),
    };
    if !construct {
        return Ok(Value::String(s));
    }
    // Flattened now, with its copy counted, so reading a character copies nothing. The string
    // stays rooted, so what it holds is still counted if reserving the copy collects.
    interp.rooted(&[Value::String(s.clone())], |interp| flat_charged(interp, &s))?;
    let proto = interp.intrinsics.string_proto;
    Ok(Value::Object(interp.heap.alloc_object(Object::new(Class::String(s), Some(proto)))))
}

/// `String.fromCharCode(…)` (section 15.5.3.2).
fn from_char_code(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let mut units = Vec::with_capacity(args.len());
    for a in args {
        let n = interp.to_number(a)?;
        units.push(to_uint32(n) as u16);
    }
    reserve_units(interp, units.len())?;
    Ok(Value::String(Str::from_units(units)))
}

// ---- String.prototype ----

/// `toString` and `valueOf` (sections 15.5.4.2 and 15.5.4.3).
fn to_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    match &this {
        Value::String(_) => Ok(this),
        Value::Object(o) => match interp.heap.object(*o).map(|o| &o.class) {
            Some(Class::String(s)) => Ok(Value::String(s.clone())),
            _ => Err(interp.type_error("String.prototype.toString was called on a value that is not a string")),
        },
        _ => Err(interp.type_error("String.prototype.toString was called on a value that is not a string")),
    }
}

fn char_at(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let s = this_str(interp, &this, "charAt")?;
    let pos = int_arg(interp, args, 0)?;
    if pos < 0.0 || pos >= s.len() as f64 {
        return Ok(Value::str(""));
    }
    let unit = flat_charged(interp, &s)?.get(pos as usize).copied().unwrap_or(0);
    Ok(Value::String(Str::from_units(vec![unit])))
}

fn char_code_at(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let s = this_str(interp, &this, "charCodeAt")?;
    let pos = int_arg(interp, args, 0)?;
    if pos < 0.0 || pos >= s.len() as f64 {
        return Ok(Value::Number(f64::NAN));
    }
    Ok(Value::Number(f64::from(flat_charged(interp, &s)?.get(pos as usize).copied().unwrap_or(0))))
}

fn concat(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let mut s = this_str(interp, &this, "concat")?;
    for a in args {
        let r = interp.to_str(a)?;
        s = interp.concat(&s, &r)?;
    }
    Ok(Value::String(s))
}

fn index_of(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let s = this_str(interp, &this, "indexOf")?;
    let search = interp.to_str(&arg(args, 0))?;
    let pos = int_arg(interp, args, 1)?;
    let (hay, needle) = (flat_charged(interp, &s)?, flat_charged(interp, &search)?);
    let start = clamp(pos, hay.len());
    let found = find(interp, &hay, &needle, start)?;
    Ok(Value::Number(found.map_or(-1.0, |i| i as f64)))
}

fn last_index_of(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let s = this_str(interp, &this, "lastIndexOf")?;
    let search = interp.to_str(&arg(args, 0))?;
    let n = interp.to_number(&arg(args, 1))?;
    let pos = if n.is_nan() { f64::INFINITY } else { to_integer(n) };
    let (hay, needle) = (flat_charged(interp, &s)?, flat_charged(interp, &search)?);
    let start = clamp(pos, hay.len());
    if needle.len() > hay.len() {
        return Ok(Value::Number(-1.0));
    }
    let mut k = start.min(hay.len() - needle.len());
    let per = (needle.len() / UNITS_PER_FUEL) as u64 + 1;
    loop {
        use_fuel_n(interp, per)?;
        if hay.get(k..k + needle.len()) == Some(&needle[..]) {
            return Ok(Value::Number(k as f64));
        }
        if k == 0 {
            return Ok(Value::Number(-1.0));
        }
        k -= 1;
    }
}

/// `localeCompare` (section 15.5.4.9): code unit order.
fn locale_compare(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let s = this_str(interp, &this, "localeCompare")?;
    let that = interp.to_str(&arg(args, 0))?;
    let (a, b) = (flat_charged(interp, &s)?, flat_charged(interp, &that)?);
    charge_units(interp, a.len().min(b.len()))?;
    Ok(Value::Number(match a.cmp(&b) {
        std::cmp::Ordering::Less => -1.0,
        std::cmp::Ordering::Equal => 0.0,
        std::cmp::Ordering::Greater => 1.0,
    }))
}

fn slice(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let s = this_str(interp, &this, "slice")?;
    let len = s.len() as f64;
    let start = int_arg(interp, args, 0)?;
    let end = if matches!(arg(args, 1), Value::Undefined) { len } else { int_arg(interp, args, 1)? };
    let from = if start < 0.0 { (len + start).max(0.0) } else { start.min(len) };
    let to = if end < 0.0 { (len + end).max(0.0) } else { end.min(len) };
    if to <= from {
        return Ok(Value::str(""));
    }
    let units = flat_charged(interp, &s)?;
    substring_value(interp, &units, from as usize, to as usize)
}

fn substring(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let s = this_str(interp, &this, "substring")?;
    let len = s.len();
    let start = int_arg(interp, args, 0)?;
    let end = if matches!(arg(args, 1), Value::Undefined) { len as f64 } else { int_arg(interp, args, 1)? };
    let (a, b) = (clamp(start, len), clamp(end, len));
    let units = flat_charged(interp, &s)?;
    substring_value(interp, &units, a.min(b), a.max(b))
}

/// `substr` (Annex B.2.3). Its first step applies ToString to `this` with no object-coercible
/// check, so `null` and `undefined` give substrings of `"null"` and `"undefined"`.
fn substr(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let s = interp.to_str(&this)?;
    let len = s.len() as f64;
    let start = int_arg(interp, args, 0)?;
    let length = if matches!(arg(args, 1), Value::Undefined) { f64::INFINITY } else { int_arg(interp, args, 1)? };
    let start = if start >= 0.0 { start } else { (len + start).max(0.0) };
    let count = length.max(0.0).min(len - start);
    if count <= 0.0 {
        return Ok(Value::str(""));
    }
    let units = flat_charged(interp, &s)?;
    substring_value(interp, &units, start as usize, (start + count) as usize)
}

/// The UTF-16 length of `c` mapped to lower or upper case.
fn mapped_len(c: char, upper: bool) -> usize {
    if upper { c.to_uppercase().map(char::len_utf16).sum() } else { c.to_lowercase().map(char::len_utf16).sum() }
}

/// Maps the characters of `units` to lower or upper case (Unicode's full mappings, final sigma
/// included), passing lone surrogates through. The result's length is reserved before it is built.
fn map_case(interp: &mut Interp, units: &[u16], upper: bool) -> JsResult<Vec<u16>> {
    let len: usize = char::decode_utf16(units.iter().copied()).map(|r| r.map_or(1, |c| mapped_len(c, upper))).sum();
    reserve_units(interp, len)?;
    let mut out = Vec::with_capacity(len);
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut Vec<u16>| {
        let mapped = if upper { run.to_uppercase() } else { run.to_lowercase() };
        out.extend(mapped.encode_utf16());
        run.clear();
    };
    for r in char::decode_utf16(units.iter().copied()) {
        match r {
            Ok(c) => run.push(c),
            Err(e) => {
                flush(&mut run, &mut out);
                out.push(e.unpaired_surrogate());
            }
        }
    }
    flush(&mut run, &mut out);
    Ok(out)
}

fn to_lower_case(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let s = this_str(interp, &this, "toLowerCase")?;
    let units = flat_charged(interp, &s)?;
    let out = map_case(interp, &units, false)?;
    Ok(Value::String(Str::from_units(out)))
}

fn to_upper_case(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let s = this_str(interp, &this, "toUpperCase")?;
    let units = flat_charged(interp, &s)?;
    let out = map_case(interp, &units, true)?;
    Ok(Value::String(Str::from_units(out)))
}

fn trim(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let s = this_str(interp, &this, "trim")?;
    let units = flat_charged(interp, &s)?;
    // The scan reads every unit once at most.
    charge_units(interp, units.len())?;
    let start = units.iter().position(|&u| !is_str_space(u)).unwrap_or(units.len());
    let end = units.iter().rposition(|&u| !is_str_space(u)).map_or(start, |i| i + 1);
    if start == 0 && end == units.len() {
        return Ok(Value::String(s));
    }
    substring_value(interp, &units, start, end)
}

// ---- the methods that take a regular expression ----

/// The regular expression argument of `match` and `search`: a RegExp object, or a new one made
/// from the value as `new RegExp(value)` would.
fn regexp_arg(interp: &mut Interp, v: &Value) -> JsResult<ObjId> {
    if let Some((o, _)) = regexp_of(interp, v) {
        return Ok(o);
    }
    let o = new_regexp_from(interp, v, &Value::Undefined)?;
    interp.temp_roots.push(Value::Object(o));
    Ok(o)
}

/// `match` (section 15.5.4.10).
fn match_(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    rooted(interp, |interp| {
        let s = this_str(interp, &this, "match")?;
        let rx = regexp_arg(interp, &arg(args, 0))?;
        let global = interp.get_value(&Value::Object(rx), &crate::heap::key("global"))?.truthy();
        if !global {
            return crate::regexp_builtins::exec_value(interp, rx, &s);
        }
        let last_index = crate::heap::key("lastIndex");
        interp.put(rx, last_index.clone(), Value::Number(0.0), true)?;
        let a = new_array(interp);
        let mut n = 0usize;
        let mut previous = 0.0;
        let units = flat_charged(interp, &s)?;
        while let Some(caps) = exec_raw(interp, rx, &s)? {
            let this_index = interp.get_value(&Value::Object(rx), &last_index)?;
            let this_index = interp.to_number(&this_index)?;
            if this_index == previous {
                previous = this_index + 1.0;
                interp.put(rx, last_index.clone(), Value::Number(previous), true)?;
            } else {
                previous = this_index;
            }
            let m = substring_value(interp, &units, caps.first().copied().unwrap_or(0), caps.get(1).copied().unwrap_or(0))?;
            set_element(interp, a, n, m)?;
            n += 1;
        }
        Ok(if n == 0 { Value::Null } else { Value::Object(a) })
    })
}

/// `search` (section 15.5.4.12): the index of the first match from the start, ignoring
/// `lastIndex` and `global`.
fn search(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    rooted(interp, |interp| {
        let s = this_str(interp, &this, "search")?;
        let rx = regexp_arg(interp, &arg(args, 0))?;
        let Some((_, re)) = regexp_of(interp, &Value::Object(rx)) else { return Err(interp.type_error("not a regular expression")) };
        let units = flat_charged(interp, &s)?;
        let mut m = Matcher::new(&re, interp)?;
        let found = m.search(interp, &units, 0)?;
        Ok(Value::Number(if found { m.caps.first().copied().unwrap_or(0) as f64 } else { -1.0 }))
    })
}

/// A piece of a result string: a range of the input, a range of the replacement pattern, or the
/// units of a string a replacement function returned.
enum Piece {
    Range(usize, usize),
    Template(usize, usize),
    Units(Rc<[u16]>),
}

impl Piece {
    fn len(&self) -> usize {
        match self {
            Piece::Range(a, b) | Piece::Template(a, b) => b.saturating_sub(*a),
            Piece::Units(u) => u.len(),
        }
    }
}

/// The pieces of a result string and the length they add up to.
#[derive(Default)]
struct Pieces {
    list: Vec<Piece>,
    len: usize,
}

impl Pieces {
    /// Adds `piece`, holding its bytes and taking fuel for its units, and ends the run with the
    /// heap limit once the result would not fit.
    fn add(&mut self, interp: &mut Interp, held: &mut HeldBytes, piece: Piece) -> JsResult<()> {
        let n = piece.len();
        if n == 0 {
            return Ok(());
        }
        push_held(interp, held, &mut self.list, piece)?;
        self.len = self.len.saturating_add(n);
        charge_units(interp, n)?;
        check_string_room(interp, self.len)
    }

    /// Joins the pieces into a new string, reserving its length first. `units` is the input and
    /// `template` the replacement pattern.
    fn build(&self, interp: &mut Interp, units: &[u16], template: &[u16]) -> JsResult<Value> {
        reserve_string(interp, self.len)?;
        let mut out = Vec::with_capacity(self.len);
        for p in &self.list {
            match p {
                Piece::Range(a, b) => out.extend_from_slice(units.get(*a..*b).unwrap_or_default()),
                Piece::Template(a, b) => out.extend_from_slice(template.get(*a..*b).unwrap_or_default()),
                Piece::Units(u) => out.extend_from_slice(u),
            }
        }
        Ok(Value::String(Str::from_units(out)))
    }
}

/// A part of a replacement pattern (section 15.5.4.11, Table 22).
#[derive(Clone, Copy)]
enum Part {
    /// The pattern's own text in a range.
    Text(usize, usize),
    /// `$&`
    Matched,
    /// `` $` ``
    Before,
    /// `$'`
    After,
    /// `$n` or `$nn`
    Group(usize),
}

impl Part {
    /// The piece this part gives for one match: `caps` holds the match and its groups as pairs of
    /// positions in an input of `len` units.
    fn piece(self, caps: &[usize], len: usize) -> Option<Piece> {
        let (start, end) = (caps.first().copied().unwrap_or(0), caps.get(1).copied().unwrap_or(0));
        match self {
            Part::Text(a, b) => Some(Piece::Template(a, b)),
            Part::Matched => Some(Piece::Range(start, end)),
            Part::Before => Some(Piece::Range(0, start)),
            Part::After => Some(Piece::Range(end, len)),
            Part::Group(n) => {
                let (s, e) = (caps.get(2 * n).copied().unwrap_or(NONE), caps.get(2 * n + 1).copied().unwrap_or(NONE));
                (s != NONE && e != NONE).then_some(Piece::Range(s, e))
            }
        }
    }
}

/// Reads a replacement pattern once into its parts, for a regular expression with `groups`
/// groups. The parts are held against the heap limit, and the units read take fuel.
fn parse_template(interp: &mut Interp, held: &mut HeldBytes, template: &[u16], groups: usize) -> JsResult<Vec<Part>> {
    charge_units(interp, template.len())?;
    let digit = |i: usize| template.get(i).filter(|&&u| (0x30..=0x39).contains(&u)).map(|&u| usize::from(u - 0x30));
    let mut parts = Vec::new();
    let mut lit = 0;
    let mut i = 0;
    while let Some(&u) = template.get(i) {
        if u != 0x24 {
            i += 1;
            continue;
        }
        let (part, taken) = match template.get(i + 1) {
            // `$$` gives one `$`: the text up to and including the first.
            Some(0x24) => {
                push_held(interp, held, &mut parts, Part::Text(lit, i + 1))?;
                lit = i + 2;
                i += 2;
                continue;
            }
            Some(0x26) => (Part::Matched, 2),
            Some(0x60) => (Part::Before, 2),
            Some(0x27) => (Part::After, 2),
            _ => match (digit(i + 1), digit(i + 2)) {
                (Some(a), Some(b)) if (1..=groups).contains(&(a * 10 + b)) => (Part::Group(a * 10 + b), 3),
                (Some(a), _) if (1..=groups).contains(&a) => (Part::Group(a), 2),
                _ => {
                    i += 1;
                    continue;
                }
            },
        };
        if i > lit {
            push_held(interp, held, &mut parts, Part::Text(lit, i))?;
        }
        push_held(interp, held, &mut parts, part)?;
        i += taken;
        lit = i;
    }
    if template.len() > lit {
        push_held(interp, held, &mut parts, Part::Text(lit, template.len()))?;
    }
    Ok(parts)
}

/// `replace` (section 15.5.4.11). The matches, the parsed replacement pattern and the pieces of
/// the result are held against the heap limit while the result is put together, and the
/// result's length is checked against the heap as each piece is added.
fn replace(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    rooted(interp, |interp| {
        let s = this_str(interp, &this, "replace")?;
        let search_value = arg(args, 0);
        let replace_value = arg(args, 1);
        let units = flat_charged(interp, &s)?;
        let mut held = HeldBytes::new(interp);
        // Each match as the positions of the match and its groups.
        let mut matches: Vec<Vec<usize>> = Vec::new();
        if let Some((rx, _)) = regexp_of(interp, &search_value) {
            let global = interp.get_value(&search_value, &crate::heap::key("global"))?.truthy();
            if global {
                let last_index = crate::heap::key("lastIndex");
                interp.put(rx, last_index.clone(), Value::Number(0.0), true)?;
                let mut previous = 0.0;
                while let Some(caps) = exec_raw(interp, rx, &s)? {
                    held.add(interp, size_of::<usize>().saturating_mul(caps.capacity()))?;
                    push_held(interp, &mut held, &mut matches, caps)?;
                    let this_index = interp.get_value(&search_value, &last_index)?;
                    let this_index = interp.to_number(&this_index)?;
                    if this_index == previous {
                        previous = this_index + 1.0;
                        interp.put(rx, last_index.clone(), Value::Number(previous), true)?;
                    } else {
                        previous = this_index;
                    }
                }
            } else if let Some(caps) = exec_raw(interp, rx, &s)? {
                matches.push(caps);
            }
        } else {
            let search = interp.to_str(&search_value)?;
            let needle = flat_charged(interp, &search)?;
            if let Some(i) = find(interp, &units, &needle, 0)? {
                matches.push(vec![i, i + needle.len()]);
            }
        }
        let replacer = interp.is_callable(&replace_value).then(|| replace_value.clone());
        let template = match &replacer {
            Some(_) => None,
            None => {
                let t = interp.to_str(&replace_value)?;
                Some(flat_charged(interp, &t)?)
            }
        };
        if matches.is_empty() {
            return Ok(Value::String(s));
        }
        let template: &[u16] = template.as_deref().unwrap_or_default();
        // Every match has the same number of groups.
        let groups = matches.first().map_or(0, |caps| (caps.len() / 2).saturating_sub(1));
        let parts = if replacer.is_none() { parse_template(interp, &mut held, template, groups)? } else { Vec::new() };
        let mut pieces = Pieces::default();
        let mut last = 0usize;
        for caps in &matches {
            let (start, end) = (caps.first().copied().unwrap_or(0), caps.get(1).copied().unwrap_or(0));
            pieces.add(interp, &mut held, Piece::Range(last, start))?;
            match &replacer {
                Some(f) => {
                    let mut call_args = Vec::with_capacity(caps.len() / 2 + 2);
                    for pair in caps.chunks(2) {
                        let v = match pair {
                            [a, b] if *a != NONE && *b != NONE => substring_value(interp, &units, *a, *b)?,
                            _ => Value::Undefined,
                        };
                        call_args.push(v);
                    }
                    call_args.push(Value::Number(start as f64));
                    call_args.push(Value::String(s.clone()));
                    let r = interp.call(f, &Value::Undefined, &call_args)?;
                    let text = interp.to_str(&r)?;
                    let text = flat_charged(interp, &text)?;
                    pieces.add(interp, &mut held, Piece::Units(text))?;
                }
                None => {
                    // Reading the parts takes fuel even when they give empty pieces.
                    use_fuel_n(interp, (parts.len() / UNITS_PER_FUEL) as u64)?;
                    for part in &parts {
                        if let Some(p) = part.piece(caps, units.len()) {
                            pieces.add(interp, &mut held, p)?;
                        }
                    }
                }
            }
            use_fuel_n(interp, 1)?;
            last = end;
        }
        pieces.add(interp, &mut held, Piece::Range(last, units.len()))?;
        pieces.build(interp, &units, template)
    })
}

/// `split` (section 15.5.4.14).
fn split(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    rooted(interp, |interp| {
        let s = this_str(interp, &this, "split")?;
        let separator = arg(args, 0);
        let limit = arg(args, 1);
        let a = new_array(interp);
        let lim = if matches!(limit, Value::Undefined) { u32::MAX } else { to_uint32(interp.to_number(&limit)?) };
        let rx = regexp_of(interp, &separator);
        let sep = match &rx {
            Some(_) => None,
            None => {
                let sep = interp.to_str(&separator)?;
                Some(flat_charged(interp, &sep)?)
            }
        };
        if lim == 0 {
            return Ok(Value::Object(a));
        }
        if matches!(separator, Value::Undefined) {
            set_element(interp, a, 0, Value::String(s))?;
            return Ok(Value::Object(a));
        }
        let units = flat_charged(interp, &s)?;
        let size = units.len();
        let re: Option<Rc<crate::regexp_vm::Regex>> = rx.map(|(_, re)| re);
        let mut matcher = re.as_deref().map(|re| Matcher::new(re, interp)).transpose()?;
        // SplitMatch (section 15.5.4.14): the end of a separator match at `q` and the groups.
        let mut split_match = |interp: &mut Interp, q: usize| -> JsResult<Option<(usize, Vec<usize>)>> {
            match (&mut matcher, &sep) {
                (Some(m), _) => {
                    if m.match_at(interp, &units, q)? {
                        Ok(Some((m.caps.get(1).copied().unwrap_or(q), m.caps.get(2..).map(<[usize]>::to_vec).unwrap_or_default())))
                    } else {
                        Ok(None)
                    }
                }
                (None, Some(r)) => {
                    use_fuel_n(interp, (r.len() / UNITS_PER_FUEL) as u64 + 1)?;
                    if units.get(q..q + r.len()) == Some(&r[..]) { Ok(Some((q + r.len(), Vec::new()))) } else { Ok(None) }
                }
                (None, None) => Ok(None),
            }
        };
        if size == 0 {
            if split_match(interp, 0)?.is_none() {
                set_element(interp, a, 0, Value::String(s.clone()))?;
            }
            return Ok(Value::Object(a));
        }
        let mut n = 0usize;
        let (mut p, mut q) = (0usize, 0usize);
        while q != size {
            match split_match(interp, q)? {
                None => q += 1,
                Some((e, _)) if e == p => q += 1,
                Some((e, caps)) => {
                    let t = substring_value(interp, &units, p, q)?;
                    set_element(interp, a, n, t)?;
                    n += 1;
                    if n as u64 == u64::from(lim) {
                        return Ok(Value::Object(a));
                    }
                    p = e;
                    for pair in caps.chunks(2) {
                        let v = match pair {
                            [x, y] if *x != NONE && *y != NONE => substring_value(interp, &units, *x, *y)?,
                            _ => Value::Undefined,
                        };
                        set_element(interp, a, n, v)?;
                        n += 1;
                        if n as u64 == u64::from(lim) {
                            return Ok(Value::Object(a));
                        }
                    }
                    q = p;
                }
            }
        }
        let t = substring_value(interp, &units, p, size)?;
        set_element(interp, a, n, t)?;
        Ok(Value::Object(a))
    })
}
