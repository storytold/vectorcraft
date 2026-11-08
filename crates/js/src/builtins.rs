//! The built-in objects (ECMA-262 5.1 section 15): `Object` with `Object.prototype` and the
//! ES5 statics, `Function` with `Function.prototype.call`, `apply`, `bind` and `toString`,
//! `Array`, the error constructors, `Math`, `JSON`, the global functions and constants, Annex
//! B's `escape` and `unescape`, and the prototypes primitive values look properties up on.
//! `String`, `Number`, `Boolean`, `RegExp` and `Date` are not here yet.
//!
//! Every loop in a built-in takes a unit of fuel for each step, and every string, array, object
//! or property it makes is checked against the heap limit before it is made, so no built-in can
//! run past [`crate::Limits`].

mod array;
mod global;
mod json;
mod math;
mod object;

use std::rc::Rc;

use crate::bytecode::{CodeKind, Source};
use crate::compiler::{CompileFail, compile_constructed_function};
use crate::heap::{
    BOUND_ARG_BYTES, Class, Func, HIDDEN, Key, NativeFn, NativeKind, OBJECT_BYTES, OPEN, ObjId, Object, PROP_BYTES, Prop, key, string_bytes,
};
use crate::interp::{Abrupt, Desc, ErrorType, Interp, JsResult};
use crate::value::{Str, Value, index_key, number_to_str, to_uint32};

fn alloc(interp: &mut Interp, class: Class, proto: Option<ObjId>) -> ObjId {
    interp.heap.alloc_object(Object::new(class, proto))
}

/// A built-in function object.
fn native(interp: &mut Interp, name: &'static str, f: NativeFn, length: u32, kind: NativeKind, constructor: bool) -> JsResult<ObjId> {
    let proto = interp.intrinsics.function_proto;
    let o = alloc(interp, Class::Function(Func::Native { f, kind, constructor, name }), Some(proto));
    interp.define(o, key("length"), Prop::Data { value: Value::Number(f64::from(length)), flags: 0 })?;
    Ok(o)
}

fn method(interp: &mut Interp, on: ObjId, name: &'static str, f: NativeFn, length: u32) -> JsResult<()> {
    let m = native(interp, name, f, length, NativeKind::Plain, false)?;
    interp.define(on, key(name), Prop::Data { value: Value::Object(m), flags: HIDDEN })
}

fn data(interp: &mut Interp, on: ObjId, name: &str, value: Value, flags: u8) -> JsResult<()> {
    interp.define(on, key(name), Prop::Data { value, flags })
}

/// Links a constructor and its prototype and makes the constructor a global.
fn constructor(interp: &mut Interp, name: &'static str, ctor: ObjId, proto: ObjId) -> JsResult<()> {
    data(interp, ctor, "prototype", Value::Object(proto), 0)?;
    data(interp, proto, "constructor", Value::Object(ctor), HIDDEN)?;
    let global = interp.global;
    data(interp, global, name, Value::Object(ctor), HIDDEN)
}

/// Creates the built-ins and the global object's properties. Installing is not bounded by
/// [`crate::Limits`]; if it fails, the failure is kept in `interp.install_error` and every run of
/// the realm ends with it.
pub(crate) fn install(interp: &mut Interp) {
    if let Err(e) = try_install(interp) {
        let reason = match e {
            Abrupt::Throw(v) => interp.describe(&v),
            Abrupt::Limit(kind) => format!("limit reached: {kind:?}"),
            Abrupt::Internal(message) => message,
        };
        interp.install_error = Some(format!("the built-ins could not be installed: {reason}"));
    }
}

fn try_install(interp: &mut Interp) -> JsResult<()> {
    let object_proto = alloc(interp, Class::Ordinary, None);
    interp.intrinsics.object_proto = object_proto;
    let function_proto = alloc(
        interp,
        Class::Function(Func::Native { f: function_proto_call, kind: NativeKind::Plain, constructor: false, name: "" }),
        Some(object_proto),
    );
    interp.intrinsics.function_proto = function_proto;
    data(interp, function_proto, "length", Value::Number(0.0), 0)?;
    let global = interp.global;
    if let Some(g) = interp.heap.object_mut(global) {
        g.proto = Some(object_proto);
    }
    interp.intrinsics.array_proto = alloc(interp, Class::Array { length: 0, length_writable: true }, Some(object_proto));
    interp.intrinsics.string_proto = alloc(interp, Class::String(Str::from("")), Some(object_proto));
    interp.intrinsics.number_proto = alloc(interp, Class::Number(0.0), Some(object_proto));
    interp.intrinsics.boolean_proto = alloc(interp, Class::Boolean(false), Some(object_proto));
    interp.intrinsics.regexp_proto = alloc(interp, Class::Ordinary, Some(object_proto));
    let thrower = native(interp, "", throw_type_error, 0, NativeKind::Plain, false)?;
    if let Some(t) = interp.heap.object_mut(thrower) {
        t.extensible = false;
    }
    interp.intrinsics.throw_type_error = thrower;

    data(interp, global, "NaN", Value::Number(f64::NAN), 0)?;
    data(interp, global, "Infinity", Value::Number(f64::INFINITY), 0)?;
    data(interp, global, "undefined", Value::Undefined, 0)?;
    let eval = native(interp, "eval", eval, 1, NativeKind::Eval, false)?;
    interp.intrinsics.eval = eval;
    data(interp, global, "eval", Value::Object(eval), HIDDEN)?;

    let object = native(interp, "Object", object_ctor, 1, NativeKind::Plain, true)?;
    constructor(interp, "Object", object, object_proto)?;
    method(interp, object_proto, "toString", object_to_string, 0)?;
    method(interp, object_proto, "toLocaleString", object_to_locale_string, 0)?;
    method(interp, object_proto, "valueOf", object_value_of, 0)?;
    method(interp, object_proto, "hasOwnProperty", has_own_property, 1)?;
    method(interp, object_proto, "isPrototypeOf", is_prototype_of, 1)?;
    method(interp, object_proto, "propertyIsEnumerable", property_is_enumerable, 1)?;

    let function = native(interp, "Function", function_ctor, 1, NativeKind::Plain, true)?;
    constructor(interp, "Function", function, function_proto)?;
    let call = native(interp, "call", unreachable_native, 1, NativeKind::Call, false)?;
    data(interp, function_proto, "call", Value::Object(call), HIDDEN)?;
    let apply = native(interp, "apply", unreachable_native, 2, NativeKind::Apply, false)?;
    data(interp, function_proto, "apply", Value::Object(apply), HIDDEN)?;
    method(interp, function_proto, "bind", function_bind, 1)?;
    method(interp, function_proto, "toString", function_to_string, 0)?;

    install_errors(interp, object_proto)?;
    object::install(interp, object)?;
    array::install(interp)?;
    math::install(interp)?;
    json::install(interp)?;
    global::install(interp)
}

// ---- helpers for the built-ins ----

/// ToInteger (section 9.4).
fn to_integer(interp: &mut Interp, v: &Value) -> JsResult<f64> {
    let n = interp.to_number(v)?;
    Ok(if n.is_nan() { 0.0 } else { n.trunc() })
}

/// The property name of the index `n`, which may be past the largest array index.
fn key_of(n: f64) -> Key {
    if (0.0..4_294_967_295.0).contains(&n) && n.fract() == 0.0 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        return index_key(n as u32);
    }
    number_to_str(n).flat()
}

/// ToUint32 of an object's `length`.
fn length_of(interp: &mut Interp, o: ObjId) -> JsResult<u32> {
    let len = interp.get_from(o, &key("length"), &Value::Object(o))?;
    Ok(to_uint32(interp.to_number(&len)?))
}

fn get_at(interp: &mut Interp, o: ObjId, k: f64) -> JsResult<Value> {
    interp.get_from(o, &key_of(k), &Value::Object(o))
}

fn has_at(interp: &mut Interp, o: ObjId, k: f64) -> JsResult<bool> {
    interp.has_property(o, &key_of(k))
}

/// The estimated bytes a new element adds.
const ELEMENT_BYTES: usize = PROP_BYTES + 32 + 2 * 10;

/// `o[k] = v` as the array methods assign: a failure throws. When `o` has no own property `k`, `v`
/// stays rooted while heap is reserved for the new element, since the caller may hold it nowhere
/// else; overwriting an own property reserves nothing.
pub(crate) fn put_at(interp: &mut Interp, o: ObjId, k: f64, v: Value) -> JsResult<()> {
    let k = key_of(k);
    if interp.heap.object(o).is_some_and(|obj| obj.props.contains(&k)) {
        return interp.put(o, k, v, true);
    }
    interp.rooted(std::slice::from_ref(&v), |i| {
        i.reserve_heap(ELEMENT_BYTES)?;
        i.put(o, k.clone(), v.clone(), true)
    })
}

fn delete_at(interp: &mut Interp, o: ObjId, k: f64) -> JsResult<()> {
    interp.delete(o, &key_of(k), true).map(drop)
}

/// Defines `a[k]` as a writable, enumerable and configurable data property, as the methods that
/// build a new array do (DefineOwnProperty with `throw` false). `v` stays rooted while heap is
/// reserved for it.
fn create_at(interp: &mut Interp, a: ObjId, k: f64, v: Value) -> JsResult<()> {
    interp.rooted(std::slice::from_ref(&v), |i| {
        i.reserve_heap(ELEMENT_BYTES)?;
        i.define_own_property(a, key_of(k), &Desc::data(v.clone(), OPEN), false).map(drop)
    })
}

/// A new array with the given length and no elements.
fn new_array(interp: &mut Interp, length: u32) -> JsResult<ObjId> {
    interp.reserve_heap(OBJECT_BYTES)?;
    let proto = interp.intrinsics.array_proto;
    Ok(interp.heap.alloc_object(Object::new(Class::Array { length, length_writable: true }, Some(proto))))
}

/// A new string from code units, checked against the heap limit and charged to the heap.
fn new_string(interp: &mut Interp, units: Vec<u16>) -> JsResult<Value> {
    let bytes = string_bytes(units.len());
    interp.reserve_heap(bytes)?;
    interp.heap.bytes = interp.heap.bytes.saturating_add(bytes);
    Ok(Value::String(Str::from_units(units)))
}

/// Code units a built-in may write into a string for one unit of fuel.
const UNITS_PER_FUEL: usize = 16;

/// A string the built-ins build piece by piece: each piece takes fuel for its length, is checked
/// against the longest string the heap limit allows, and has its bytes reserved before it is
/// added. The buffer's bytes count toward the heap until `finish` or until the native call that
/// made the builder returns. Everything the caller still needs must be reachable from the roots.
struct Builder {
    units: Vec<u16>,
    /// Code units already reserved and charged to the heap.
    charged: usize,
    /// Code units written since fuel was last taken, fewer than `UNITS_PER_FUEL`.
    unpaid: usize,
}

impl Builder {
    fn new() -> Self {
        Builder { units: Vec::new(), charged: 0, unpaid: 0 }
    }

    /// Checks that `more` code units fit, reserving heap for them, and takes fuel for writing them.
    fn make_room(&mut self, interp: &mut Interp, more: usize) -> JsResult<()> {
        let want = self.units.len().saturating_add(more);
        if want > interp.limits.heap_bytes / 4 {
            return Err(Abrupt::Limit(crate::LimitKind::Heap));
        }
        self.unpaid += more;
        let fuel = (self.unpaid / UNITS_PER_FUEL) as u64;
        self.unpaid %= UNITS_PER_FUEL;
        if fuel > 0 {
            if interp.fuel < fuel {
                interp.fuel = 0;
                return Err(Abrupt::Limit(crate::LimitKind::Fuel));
            }
            // The last unit goes through `use_fuel`, which also checks for an interrupt.
            interp.fuel -= fuel - 1;
            interp.use_fuel()?;
        }
        if want > self.charged {
            let grow = (want - self.charged).max(self.charged / 2).max(256).min((interp.limits.heap_bytes / 4).saturating_sub(self.charged));
            interp.reserve_heap(2 * grow)?;
            interp.heap.bytes = interp.heap.bytes.saturating_add(2 * grow);
            interp.held_bytes = interp.held_bytes.saturating_add(2 * grow);
            self.charged += grow;
        }
        Ok(())
    }

    fn push_units(&mut self, interp: &mut Interp, units: &[u16]) -> JsResult<()> {
        self.make_room(interp, units.len())?;
        self.units.extend_from_slice(units);
        Ok(())
    }

    /// Appends `units` `count` times. Nothing is written for an empty `units`, however large
    /// `count` is.
    fn push_repeated(&mut self, interp: &mut Interp, units: &[u16], count: u64) -> JsResult<()> {
        if units.is_empty() || count == 0 {
            return Ok(());
        }
        let more = usize::try_from(count).unwrap_or(usize::MAX).saturating_mul(units.len());
        self.make_room(interp, more)?;
        for _ in 0..count {
            self.units.extend_from_slice(units);
        }
        Ok(())
    }

    fn push_str(&mut self, interp: &mut Interp, s: &str) -> JsResult<()> {
        let units: Vec<u16> = s.encode_utf16().collect();
        self.push_units(interp, &units)
    }

    /// The string built, whose bytes were charged as it grew. From here on the heap counts them
    /// as the string's.
    fn finish(self, interp: &mut Interp) -> Value {
        interp.held_bytes = interp.held_bytes.saturating_sub(2 * self.charged);
        Value::String(Str::from_units(self.units))
    }
}

fn require_callable(interp: &mut Interp, v: &Value, what: &str) -> JsResult<()> {
    if interp.is_callable(v) {
        return Ok(());
    }
    Err(interp.type_error(&format!("{what} is not a function")))
}

fn install_errors(interp: &mut Interp, object_proto: ObjId) -> JsResult<()> {
    let kinds: [(&'static str, NativeFn); 7] = [
        ("Error", error_ctor),
        ("EvalError", eval_error_ctor),
        ("RangeError", range_error_ctor),
        ("ReferenceError", reference_error_ctor),
        ("SyntaxError", syntax_error_ctor),
        ("TypeError", type_error_ctor),
        ("URIError", uri_error_ctor),
    ];
    let mut error_proto = object_proto;
    for (i, (name, f)) in kinds.into_iter().enumerate() {
        // Error.prototype and each NativeError prototype are Error objects (sections 15.11.4 and
        // 15.11.7.7).
        let proto = alloc(interp, Class::Error, Some(if i == 0 { object_proto } else { error_proto }));
        if i == 0 {
            error_proto = proto;
            method(interp, proto, "toString", error_to_string, 0)?;
        }
        if let Some(slot) = interp.intrinsics.error_protos.get_mut(i) {
            *slot = proto;
        }
        data(interp, proto, "name", Value::str(name), HIDDEN)?;
        data(interp, proto, "message", Value::str(""), HIDDEN)?;
        let ctor = native(interp, name, f, 1, NativeKind::Plain, true)?;
        constructor(interp, name, ctor, proto)?;
    }
    Ok(())
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Undefined)
}

fn unreachable_native(_: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
    Err(Abrupt::Internal("call or apply reached its native body".into()))
}

fn throw_type_error(interp: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
    Err(interp.type_error("'caller', 'callee' and 'arguments' cannot be used on strict mode functions"))
}

fn function_proto_call(_: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
    Ok(Value::Undefined)
}

// ---- Object ----

/// `Object(value)` and `new Object(value)` (sections 15.2.1 and 15.2.2).
fn object_ctor(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let v = arg(args, 0);
    if v.is_nullish() {
        return Ok(Value::Object(interp.new_object()));
    }
    interp.to_object(&v).map(Value::Object)
}

/// `Object.prototype.toString` (section 15.2.4.2).
fn object_to_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let class = match &this {
        Value::Undefined => "Undefined",
        Value::Null => "Null",
        _ => {
            let o = interp.to_object(&this)?;
            interp.heap.object(o).map_or("Object", |o| o.class.name())
        }
    };
    Ok(Value::str(&format!("[object {class}]")))
}

/// The string form of a Number, String or Boolean object's [[PrimitiveValue]] (sections 8.6.2 and
/// 9.9), when `toString` is not found on the object or on its prototype chain up to and including
/// its standard prototype. Returns `None` otherwise, and for any other object.
fn boxed_primitive_string(interp: &mut Interp, o: ObjId) -> JsResult<Option<Value>> {
    let (primitive, proto) = match interp.heap.object(o).map(|x| &x.class) {
        Some(Class::Number(n)) => (Value::Number(*n), interp.intrinsics.number_proto),
        Some(Class::String(s)) => (Value::String(s.clone()), interp.intrinsics.string_proto),
        Some(Class::Boolean(b)) => (Value::Bool(*b), interp.intrinsics.boolean_proto),
        _ => return Ok(None),
    };
    if !json::reaches_prototype(interp, o, proto, "toString")? {
        return Ok(None);
    }
    Ok(Some(Value::String(interp.primitive_to_str(&primitive))))
}

/// `Object.prototype.toLocaleString` (section 15.2.4.3).
fn object_to_locale_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let o = interp.to_object(&this)?;
    if let Some(s) = boxed_primitive_string(interp, o)? {
        return Ok(s);
    }
    let receiver = Value::Object(o);
    let f = interp.get_from(o, &key("toString"), &receiver)?;
    if !interp.is_callable(&f) {
        return Err(interp.type_error("toString is not a function"));
    }
    interp.call(&f, &receiver, &[])
}

fn object_value_of(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    interp.to_object(&this).map(Value::Object)
}

fn has_own_property(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let k = interp.to_key(&arg(args, 0))?;
    let o = interp.to_object(&this)?;
    Ok(Value::Bool(interp.get_own_property(o, &k)?.is_some()))
}

fn is_prototype_of(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let Value::Object(mut v) = arg(args, 0) else { return Ok(Value::Bool(false)) };
    let o = interp.to_object(&this)?;
    while let Some(p) = interp.heap.object(v).and_then(|v| v.proto) {
        if p == o {
            return Ok(Value::Bool(true));
        }
        v = p;
        interp.use_fuel()?;
    }
    Ok(Value::Bool(false))
}

fn property_is_enumerable(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let k = interp.to_key(&arg(args, 0))?;
    let o = interp.to_object(&this)?;
    Ok(Value::Bool(interp.get_own_property(o, &k)?.is_some_and(|p| p.enumerable())))
}

// ---- Function ----

/// `Function(p1, …, body)` and `new Function(…)` (section 15.3.2.1): the source is read as
/// `(function anonymous(p1,…\n) {\nbody\n})` and must be exactly that one function.
fn function_ctor(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    const HEAD: &str = "(function anonymous(";
    const MID: &str = "\n) {\n";
    const TAIL: &str = "\n})";
    let mut parts = Vec::with_capacity(args.len());
    for a in args {
        parts.push(interp.to_str(a)?);
    }
    let body = parts.pop().unwrap_or_else(|| Str::from(""));
    // The length is checked against the source limit before any argument is flattened or copied.
    let total = parts
        .iter()
        .map(Str::len)
        .chain([parts.len().saturating_sub(1), HEAD.len() + MID.len() + TAIL.len(), body.len()])
        .fold(0usize, usize::saturating_add);
    interp.check_source_units(total).map_err(|e| interp.compile_error_to_abrupt(e))?;
    let mut units = Vec::with_capacity(total);
    units.extend(HEAD.encode_utf16());
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            units.push(u16::from(b','));
        }
        units.extend_from_slice(&p.flat());
    }
    units.extend(MID.encode_utf16());
    units.extend_from_slice(&body.flat());
    units.extend(TAIL.encode_utf16());
    interp.source_units = interp.source_units.saturating_add(units.len());
    let remaining = interp.options.max_source_units.saturating_sub(interp.source_units.saturating_sub(units.len()));
    let options = crate::Options { strict: false, max_source_units: remaining, ..interp.options.clone() };
    let program = match crate::parse_utf16(&units, &options) {
        Ok(p) => p,
        Err(e) => return Err(interp.throw(ErrorType::Syntax, &e.message)),
    };
    let source = Rc::new(Source { lines: program.lines.clone(), text: units.into() });
    let code = match compile_constructed_function(&program, &source) {
        Ok(Some(code)) => code,
        Ok(None) => return Err(interp.throw(ErrorType::Syntax, "the parameters or body do not form one function")),
        Err(CompileFail::Syntax(e)) => return Err(interp.throw(ErrorType::Syntax, &e.message)),
        Err(CompileFail::Internal(m)) => return Err(Abrupt::Internal(m)),
    };
    interp.charge_code(&code, 0)?;
    let env = interp.global_env;
    interp.make_closure(code, env).map(Value::Object)
}

/// `Function.prototype.toString` (section 15.3.4.2): a function's source text, or a stand-in
/// for a built-in.
fn function_to_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let func = this.as_object().and_then(|o| interp.heap.object(o)).and_then(|o| match &o.class {
        Class::Function(f) => Some(f.clone()),
        _ => None,
    });
    let text = match func {
        Some(Func::Closure { code, .. }) => {
            let (start, end) = (code.span.0 as usize, code.span.1 as usize);
            let units = code.source.text.get(start..end).unwrap_or_default();
            let bytes = string_bytes(units.len());
            interp.reserve_heap(bytes)?;
            interp.heap.bytes = interp.heap.bytes.saturating_add(bytes);
            return Ok(Value::String(Str::from_units(units.to_vec())));
        }
        Some(Func::Native { name, .. }) => format!("function {name}() {{ [native code] }}"),
        Some(Func::Host { name, .. }) => format!("function {name}() {{ [native code] }}"),
        Some(Func::Bound { .. }) => "function () { [native code] }".to_string(),
        None => return Err(interp.type_error("Function.prototype.toString was called on a value that is not a function")),
    };
    Ok(Value::str(&text))
}

/// `Function.prototype.bind` (section 15.3.4.5).
fn function_bind(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    require_callable(interp, &this, "the value bind was called on")?;
    let Some(target) = this.as_object() else { return Err(interp.type_error("the value bind was called on is not a function")) };
    let bound_this = arg(args, 0);
    let bound_args: Rc<[Value]> = args.get(1..).unwrap_or_default().into();
    // A host function's `length` is an ordinary property a script can set, so it is converted
    // to an integer first, as later editions specify.
    let target_length = match interp.get_from(target, &key("length"), &this)? {
        Value::Number(n) if n.is_nan() => 0.0,
        Value::Number(n) => n.trunc(),
        _ => 0.0,
    };
    // Adding 0 turns a negative zero into 0.
    let length = (target_length - bound_args.len() as f64).max(0.0) + 0.0;
    interp.reserve_heap(OBJECT_BYTES + 4 * PROP_BYTES + BOUND_ARG_BYTES * bound_args.len())?;
    let proto = interp.intrinsics.function_proto;
    let class = Class::Function(Func::Bound { target, this: bound_this, args: bound_args.clone() });
    let f = interp.heap.alloc_object(Object::new(class, Some(proto)));
    interp.heap.bytes = interp.heap.bytes.saturating_add(BOUND_ARG_BYTES * bound_args.len());
    interp.define(f, key("length"), Prop::Data { value: Value::Number(length), flags: 0 })?;
    let thrower = Some(interp.intrinsics.throw_type_error);
    for name in ["caller", "arguments"] {
        interp.define(f, key(name), Prop::Accessor { get: thrower, set: thrower, flags: 0 })?;
    }
    Ok(Value::Object(f))
}

/// The global `eval` called indirectly (section 15.1.2.1.1): global code in the global
/// environment.
fn eval(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let x = arg(args, 0);
    let Value::String(s) = &x else { return Ok(x) };
    interp.check_source_units(s.len()).map_err(|e| interp.compile_error_to_abrupt(e))?;
    let units = s.flat();
    let code = interp.compile_source(&units, false, CodeKind::Eval).map_err(|e| interp.compile_error_to_abrupt(e))?;
    interp.run_global(code, true)
}

// ---- errors ----

fn make_error(interp: &mut Interp, t: ErrorType, args: &[Value]) -> JsResult<Value> {
    let message = arg(args, 0);
    let text = if matches!(message, Value::Undefined) { None } else { Some(interp.to_str(&message)?) };
    let o = interp.error_object(t, "");
    if let Some(text) = text {
        interp.define(o, key("message"), Prop::Data { value: Value::String(text), flags: HIDDEN })?;
    }
    Ok(Value::Object(o))
}

fn error_ctor(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    make_error(interp, ErrorType::Error, args)
}

fn eval_error_ctor(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    make_error(interp, ErrorType::Eval, args)
}

fn range_error_ctor(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    make_error(interp, ErrorType::Range, args)
}

fn reference_error_ctor(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    make_error(interp, ErrorType::Reference, args)
}

fn syntax_error_ctor(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    make_error(interp, ErrorType::Syntax, args)
}

fn type_error_ctor(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    make_error(interp, ErrorType::Type, args)
}

fn uri_error_ctor(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    make_error(interp, ErrorType::Uri, args)
}

/// `Error.prototype.toString` (section 15.11.4.4).
fn error_to_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let Value::Object(o) = this else { return Err(interp.type_error("Error.prototype.toString was called on a value that is not an object")) };
    let name = interp.get_from(o, &key("name"), &this)?;
    let name = if matches!(name, Value::Undefined) { Str::from("Error") } else { interp.to_str(&name)? };
    let message = interp.get_from(o, &key("message"), &this)?;
    let message = if matches!(message, Value::Undefined) { Str::from("") } else { interp.to_str(&message)? };
    if name.len() == 0 {
        return Ok(Value::String(message));
    }
    if message.len() == 0 {
        return Ok(Value::String(name));
    }
    let sep = Str::from(": ");
    let head = interp.concat(&name, &sep)?;
    interp.concat(&head, &message).map(Value::String)
}
