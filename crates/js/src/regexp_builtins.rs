//! `RegExp` (ECMA-262 5.1 section 15.10.3 to 15.10.7): the constructor, regular expression
//! literals' objects, and `exec`, `test` and `toString`. Matching is in [`crate::regexp_vm`].

use std::rc::Rc;

use crate::LimitKind;
use crate::builtins::{arg, constructor, method, native};
use crate::heap::{Class, NativeKind, OBJECT_BYTES, OPEN, ObjId, Object, PROP_BYTES, Prop, WRITABLE, key, string_bytes};
use crate::interp::{Abrupt, ErrorType, Interp, JsResult};
use crate::regexp_vm::{CompileFail, Flags, Matcher, Regex, compile, parse_flags, use_fuel_n};
use crate::string_builtins::{new_array, reserve_units, rooted, set_element, substring_value, to_integer};
use crate::value::{Str, Value};

pub(crate) fn install(interp: &mut Interp) -> JsResult<()> {
    let proto = interp.intrinsics.regexp_proto;
    // The prototype is itself a regular expression object, matching the empty string.
    let empty = compile_regex(interp, &[], &[])?;
    if let Some(p) = interp.heap.object_mut(proto) {
        p.class = Class::RegExp(empty);
    }
    define_flags(interp, proto, Str::from("(?:)"), false, false, false)?;
    let ctor = native(interp, "RegExp", regexp_ctor, 2, NativeKind::Plain, true)?;
    constructor(interp, "RegExp", ctor, proto)?;
    method(interp, proto, "exec", exec, 1)?;
    method(interp, proto, "test", test, 1)?;
    method(interp, proto, "toString", to_string, 0)
}

/// The object and the compiled form of a RegExp object.
pub(crate) fn regexp_of(interp: &Interp, v: &Value) -> Option<(ObjId, Rc<Regex>)> {
    let o = v.as_object()?;
    match &interp.heap.object(o)?.class {
        Class::RegExp(re) => Some((o, re.clone())),
        _ => None,
    }
}

/// Checks and compiles a pattern, throwing a `SyntaxError` for a pattern or flags that are not
/// valid. The compiled form's bytes are reserved on the heap.
fn compile_regex(interp: &mut Interp, pattern: &[u16], flags: &[u16]) -> JsResult<Rc<Regex>> {
    let flags: Flags = match parse_flags(flags) {
        Ok(f) => f,
        Err(m) => return Err(interp.throw(ErrorType::Syntax, &m)),
    };
    let depth = interp.options.max_depth;
    if let Err(m) = crate::regexp::check(pattern, depth) {
        return Err(interp.throw(ErrorType::Syntax, &format!("invalid regular expression: {m}")));
    }
    // A compile that runs out of room collects garbage and tries once more with the room it frees.
    let mut collected = false;
    let re = loop {
        let room = interp.limits.heap_bytes.saturating_sub(interp.heap.bytes);
        match compile(pattern, flags, depth, room, &mut |n| use_fuel_n(interp, n)) {
            Ok(re) => break re,
            Err(CompileFail::Abrupt(Abrupt::Limit(LimitKind::Heap))) if !collected => {
                interp.collect_for_room();
                collected = true;
            }
            Err(CompileFail::Syntax(m)) => return Err(interp.throw(ErrorType::Syntax, &format!("invalid regular expression: {m}"))),
            Err(CompileFail::Abrupt(a)) => return Err(a),
        }
    };
    let bytes = re.heap_bytes();
    interp.reserve_heap(bytes)?;
    interp.heap.bytes = interp.heap.bytes.saturating_add(bytes);
    Ok(Rc::new(re))
}

/// A new RegExp object whose `source` property is `source` (section 15.10.4.1, steps 7 to 12).
fn make(interp: &mut Interp, source: Str, pattern: &[u16], flags: &[u16]) -> JsResult<ObjId> {
    let re = compile_regex(interp, pattern, flags)?;
    interp.reserve_heap(OBJECT_BYTES + 5 * (PROP_BYTES + string_bytes(10)))?;
    let (g, i, m) = (parse_flags(flags).unwrap_or_default().global, re.ignore_case, re.multiline);
    let proto = interp.intrinsics.regexp_proto;
    let r = interp.heap.alloc_object(Object::new(Class::RegExp(re), Some(proto)));
    define_flags(interp, r, source, g, i, m)?;
    Ok(r)
}

/// Defines a RegExp object's `source`, `global`, `ignoreCase`, `multiline` and `lastIndex`
/// (section 15.10.7).
fn define_flags(interp: &mut Interp, r: ObjId, source: Str, g: bool, i: bool, m: bool) -> JsResult<()> {
    interp.define(r, key("source"), Prop::Data { value: Value::String(source), flags: 0 })?;
    interp.define(r, key("global"), Prop::Data { value: Value::Bool(g), flags: 0 })?;
    interp.define(r, key("ignoreCase"), Prop::Data { value: Value::Bool(i), flags: 0 })?;
    interp.define(r, key("multiline"), Prop::Data { value: Value::Bool(m), flags: 0 })?;
    interp.define(r, key("lastIndex"), Prop::Data { value: Value::Number(0.0), flags: WRITABLE })?;
    Ok(())
}

/// The object a regular expression literal evaluates to (section 7.8.5): a new RegExp object
/// each time.
pub(crate) fn new_regexp(interp: &mut Interp, source: &Str, flags: &Str) -> JsResult<ObjId> {
    let pattern = source.flat();
    make(interp, source.clone(), &pattern, &flags.flat())
}

/// The `source` text for a pattern given to the constructor: `(?:)` for the empty pattern, and
/// `/` and line terminators escaped, so that `/source/` reads back as the same pattern. Its
/// length is reserved on the heap before it is built.
pub(crate) fn source_text(interp: &mut Interp, pattern: &[u16]) -> JsResult<Str> {
    if pattern.is_empty() {
        return Ok(Str::from("(?:)"));
    }
    let mut len = 0usize;
    source_units(pattern, |u| len = len.saturating_add(u.len()));
    reserve_units(interp, len)?;
    let mut out = Vec::with_capacity(len);
    source_units(pattern, |u| out.extend_from_slice(u));
    Ok(Str::from_units(out))
}

/// Passes the units of [`source_text`] for a non-empty pattern to `emit`, a piece at a time.
fn source_units(pattern: &[u16], mut emit: impl FnMut(&[u16])) {
    const BACKSLASH: u16 = 0x5C;
    let mut in_class = false;
    let mut i = 0;
    while let Some(&u) = pattern.get(i) {
        match u {
            0x5C => {
                emit(pattern.get(i..i + 2).unwrap_or(&[BACKSLASH]));
                i += 2;
                continue;
            }
            0x5B => in_class = true,
            0x5D => in_class = false,
            0x2F if !in_class => emit(&[BACKSLASH]),
            _ => {}
        }
        match u {
            0x0A => emit(&[BACKSLASH, 0x6E]),
            0x0D => emit(&[BACKSLASH, 0x72]),
            0x2028 => emit(&[BACKSLASH, 0x75, 0x32, 0x30, 0x32, 0x38]),
            0x2029 => emit(&[BACKSLASH, 0x75, 0x32, 0x30, 0x32, 0x39]),
            _ => emit(&[u]),
        }
        i += 1;
    }
}

/// `new RegExp(pattern, flags)` (section 15.10.4.1).
pub(crate) fn new_regexp_from(interp: &mut Interp, pattern: &Value, flags: &Value) -> JsResult<ObjId> {
    if let Some((o, _)) = regexp_of(interp, pattern) {
        if !matches!(flags, Value::Undefined) {
            return Err(interp.type_error("flags cannot be given with a RegExp object"));
        }
        let pv = Value::Object(o);
        let source = interp.get_value(&pv, &key("source"))?;
        let source = interp.to_str(&source)?;
        let mut f = String::new();
        for (name, c) in [("global", 'g'), ("ignoreCase", 'i'), ("multiline", 'm')] {
            if interp.get_value(&pv, &key(name))?.truthy() {
                f.push(c);
            }
        }
        let units = source.flat();
        let pattern: &[u16] = if is_empty_group(&units) { &[] } else { &units };
        return make(interp, source.clone(), pattern, &f.encode_utf16().collect::<Vec<u16>>());
    }
    let p = if matches!(pattern, Value::Undefined) { Str::from("") } else { interp.to_str(pattern)? };
    let f = if matches!(flags, Value::Undefined) { Str::from("") } else { interp.to_str(flags)? };
    let units = p.flat();
    let source = source_text(interp, &units)?;
    make(interp, source, &units, &f.flat())
}

fn is_empty_group(units: &[u16]) -> bool {
    units.iter().copied().eq("(?:)".encode_utf16())
}

/// `RegExp(pattern, flags)` and `new RegExp(pattern, flags)` (sections 15.10.3 and 15.10.4).
fn regexp_ctor(interp: &mut Interp, _: Value, args: &[Value], construct: bool) -> JsResult<Value> {
    let (pattern, flags) = (arg(args, 0), arg(args, 1));
    if !construct && regexp_of(interp, &pattern).is_some() && matches!(flags, Value::Undefined) {
        return Ok(pattern);
    }
    new_regexp_from(interp, &pattern, &flags).map(Value::Object)
}

fn this_regexp(interp: &mut Interp, this: &Value, name: &str) -> JsResult<ObjId> {
    match regexp_of(interp, this) {
        Some((o, _)) => Ok(o),
        None => Err(interp.type_error(&format!("RegExp.prototype.{name} was called on a value that is not a RegExp object"))),
    }
}

/// The matching steps of `exec` (section 15.10.6.2, steps 3 to 11): the positions of the match
/// and of its groups ([`crate::regexp_vm::NONE`] for a group that did not take part), or `None`.
/// Reads `lastIndex` and `global`, and sets `lastIndex` as `exec` does.
pub(crate) fn exec_raw(interp: &mut Interp, rx: ObjId, s: &Str) -> JsResult<Option<Vec<usize>>> {
    let Some((_, re)) = regexp_of(interp, &Value::Object(rx)) else { return Err(interp.type_error("not a RegExp object")) };
    let rv = Value::Object(rx);
    let last_index = key("lastIndex");
    let li = interp.get_value(&rv, &last_index)?;
    let li = to_integer(interp.to_number(&li)?);
    let global = interp.get_value(&rv, &key("global"))?.truthy();
    let units = s.flat();
    let i = if global { li } else { 0.0 };
    let found = if i < 0.0 || i > units.len() as f64 {
        None
    } else {
        let mut m = Matcher::new(&re, interp)?;
        if m.search(interp, &units, i as usize)? { Some(m.caps) } else { None }
    };
    match found {
        None => {
            interp.put(rx, last_index, Value::Number(0.0), true)?;
            Ok(None)
        }
        Some(caps) => {
            if global {
                let end = caps.get(1).copied().unwrap_or(0);
                interp.put(rx, last_index, Value::Number(end as f64), true)?;
            }
            Ok(Some(caps))
        }
    }
}

/// `exec`'s result for `rx` on `s`: the match array, or `null`.
pub(crate) fn exec_value(interp: &mut Interp, rx: ObjId, s: &Str) -> JsResult<Value> {
    rooted(interp, |interp| {
        let Some(caps) = exec_raw(interp, rx, s)? else { return Ok(Value::Null) };
        let units = s.flat();
        let a = new_array(interp);
        interp.reserve_heap(2 * (PROP_BYTES + string_bytes(5)))?;
        let index = caps.first().copied().unwrap_or(0);
        interp.define(a, key("index"), Prop::Data { value: Value::Number(index as f64), flags: OPEN })?;
        interp.define(a, key("input"), Prop::Data { value: Value::String(s.clone()), flags: OPEN })?;
        for (n, pair) in caps.chunks(2).enumerate() {
            let v = match pair {
                [x, y] if *x != crate::regexp_vm::NONE && *y != crate::regexp_vm::NONE => substring_value(interp, &units, *x, *y)?,
                _ => Value::Undefined,
            };
            set_element(interp, a, n, v)?;
        }
        Ok(Value::Object(a))
    })
}

/// `RegExp.prototype.exec` (section 15.10.6.2).
fn exec(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let rx = this_regexp(interp, &this, "exec")?;
    let s = interp.to_str(&arg(args, 0))?;
    exec_value(interp, rx, &s)
}

/// `RegExp.prototype.test` (section 15.10.6.3).
fn test(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let rx = this_regexp(interp, &this, "test")?;
    let s = interp.to_str(&arg(args, 0))?;
    Ok(Value::Bool(exec_raw(interp, rx, &s)?.is_some()))
}

/// `RegExp.prototype.toString` (section 15.10.6.4).
fn to_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let rx = this_regexp(interp, &this, "toString")?;
    let rv = Value::Object(rx);
    let source = interp.get_value(&rv, &key("source"))?;
    let source = interp.to_str(&source)?;
    let mut flags = String::from("/");
    for (name, c) in [("global", 'g'), ("ignoreCase", 'i'), ("multiline", 'm')] {
        if interp.get_value(&rv, &key(name))?.truthy() {
            flags.push(c);
        }
    }
    let head = interp.concat(&Str::from("/"), &source)?;
    interp.concat(&head, &Str::from(flags.as_str())).map(Value::String)
}
