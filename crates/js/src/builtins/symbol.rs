//! `Symbol` (ECMA-262 11th edition, section 19.4, <https://262.ecma-international.org/11.0/>):
//! the constructor with `Symbol.for`, `Symbol.keyFor` and the well-known Symbols, the methods of
//! `Symbol.prototype`, `Object.getOwnPropertySymbols` (section 19.1.2.10),
//! `Function.prototype[@@hasInstance]` (section 19.2.3.6) and `Date.prototype[@@toPrimitive]`
//! (section 20.4.4.45). Installed in a realm with [`crate::Options::es2015`] only.

use super::{arg, constructor, create_at, data, method, native, new_array};
use crate::heap::{CONFIGURABLE, Class, NativeFn, NativeKind, ObjId, Object, Prop, SYMBOL_BYTES};
use crate::interp::{Hint, Interp, JsResult};
use crate::symbol::{Es2015Objects, REGISTRY_ENTRY_BYTES, Sym, WellKnown};
use crate::value::{Str, Value};

pub(super) fn install(interp: &mut Interp, object_ctor: ObjId) -> JsResult<()> {
    let well_known = WellKnown::new();
    let names = [
        ("iterator", well_known.iterator.clone()),
        ("asyncIterator", well_known.async_iterator.clone()),
        ("hasInstance", well_known.has_instance.clone()),
        ("toPrimitive", well_known.to_primitive.clone()),
        ("toStringTag", well_known.to_string_tag.clone()),
    ];
    let (to_string_tag, to_primitive, has_instance) =
        (well_known.to_string_tag.clone(), well_known.to_primitive.clone(), well_known.has_instance.clone());
    interp.symbols.well_known = Some(well_known);

    let object_proto = interp.intrinsics.object_proto;
    let proto = interp.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));
    let ctor = native(interp, "Symbol", symbol_ctor, 0, NativeKind::Plain, true)?;
    constructor(interp, "Symbol", ctor, proto)?;
    method(interp, ctor, "for", symbol_for, 1)?;
    method(interp, ctor, "keyFor", key_for, 1)?;
    for (name, sym) in names {
        data(interp, ctor, name, Value::Symbol(sym), 0)?;
    }
    method(interp, proto, "toString", to_string, 0)?;
    method(interp, proto, "valueOf", value_of, 0)?;
    let getter = native(interp, "get description", description, 0, NativeKind::Plain, false)?;
    interp.define(proto, crate::heap::key("description"), Prop::Accessor { get: Some(getter), set: None, flags: CONFIGURABLE })?;
    interp.define_sym(proto, to_string_tag, Prop::Data { value: Value::str("Symbol"), flags: CONFIGURABLE })?;
    sym_method(interp, proto, to_primitive.clone(), "[Symbol.toPrimitive]", symbol_to_primitive, 1, CONFIGURABLE)?;
    let date_proto = interp.intrinsics.date_proto;
    sym_method(interp, date_proto, to_primitive, "[Symbol.toPrimitive]", date_to_primitive, 1, CONFIGURABLE)?;

    method(interp, object_ctor, "getOwnPropertySymbols", get_own_property_symbols, 1)?;
    let function_proto = interp.intrinsics.function_proto;
    let has_instance = sym_method(interp, function_proto, has_instance, "[Symbol.hasInstance]", function_has_instance, 1, 0)?;
    // The iterator and generator prototypes and `%ArrayProto_values%` are set when the iterators
    // and generators are installed.
    interp.symbols.objects = Some(Es2015Objects {
        symbol_proto: proto,
        array_iterator_proto: proto,
        string_iterator_proto: proto,
        array_values: has_instance,
        has_instance,
        generator_proto: proto,
        generator_function_proto: function_proto,
    });
    Ok(())
}

/// Defines a built-in function as the property `sym` of `on`, with the attribute `flags`.
pub(super) fn sym_method(interp: &mut Interp, on: ObjId, sym: Sym, name: &'static str, f: NativeFn, length: u32, flags: u8) -> JsResult<ObjId> {
    let m = native(interp, name, f, length, NativeKind::Plain, false)?;
    interp.define_sym(on, sym, Prop::Data { value: Value::Object(m), flags })?;
    Ok(m)
}

/// A new Symbol, its bytes checked against the heap limit and charged to the heap.
fn new_symbol(interp: &mut Interp, description: Option<Str>, registered: bool, extra: usize) -> JsResult<Sym> {
    let bytes = SYMBOL_BYTES.saturating_add(extra);
    let roots: Vec<Value> = description.iter().cloned().map(Value::String).collect();
    interp.rooted(&roots, |i| i.reserve_heap(bytes))?;
    interp.heap.bytes = interp.heap.bytes.saturating_add(bytes);
    Ok(Sym::new(description, registered))
}

/// `Symbol([description])` (section 19.4.1.1). `new Symbol()` throws a `TypeError`.
fn symbol_ctor(interp: &mut Interp, _: Value, args: &[Value], construct: bool) -> JsResult<Value> {
    if construct {
        return Err(interp.type_error("the value is not a constructor"));
    }
    let description = match arg(args, 0) {
        Value::Undefined => None,
        d => Some(interp.to_str(&d)?),
    };
    new_symbol(interp, description, false, 0).map(Value::Symbol)
}

/// `Symbol.for(key)` (section 19.4.2.2): the registry's Symbol for the key, made the first time.
fn symbol_for(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let key = interp.to_str(&arg(args, 0))?;
    let units = crate::string_builtins::flat_charged(interp, &key)?;
    crate::string_builtins::charge_units(interp, units.len())?;
    if let Some(v) = interp.symbols.registry.get(&*units) {
        return Ok(v.clone());
    }
    crate::string_builtins::charge_units(interp, units.len())?;
    let sym = new_symbol(interp, Some(Str::from_rc(units.clone())), true, REGISTRY_ENTRY_BYTES)?;
    interp.symbols.registry.insert(units, Value::Symbol(sym.clone()));
    Ok(Value::Symbol(sym))
}

/// `Symbol.keyFor(sym)` (section 19.4.2.6).
fn key_for(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let Value::Symbol(sym) = arg(args, 0) else { return Err(interp.type_error("Symbol.keyFor was given a value that is not a Symbol")) };
    Ok(match sym.description() {
        Some(d) if sym.registered() => Value::String(d.clone()),
        _ => Value::Undefined,
    })
}

/// thisSymbolValue (section 19.4.3): a Symbol, or the Symbol of a Symbol object.
fn this_symbol(interp: &mut Interp, this: &Value, name: &str) -> JsResult<Sym> {
    let sym = match this {
        Value::Symbol(s) => Some(s.clone()),
        Value::Object(o) => match interp.heap.object(*o).map(|x| &x.class) {
            Some(Class::Symbol(s)) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    };
    sym.ok_or_else(|| interp.type_error(&format!("Symbol.prototype.{name} was called on a value that is not a Symbol")))
}

/// `Symbol.prototype.toString` (section 19.4.3.3): `Symbol(description)`.
fn to_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let sym = this_symbol(interp, &this, "toString")?;
    interp.descriptive_string(&sym)
}

/// `Symbol.prototype.valueOf` (section 19.4.3.4).
fn value_of(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    this_symbol(interp, &this, "valueOf").map(Value::Symbol)
}

/// The getter of `Symbol.prototype.description` (section 19.4.3.2).
fn description(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let sym = this_symbol(interp, &this, "description")?;
    Ok(sym.description().map_or(Value::Undefined, |d| Value::String(d.clone())))
}

/// `Symbol.prototype[@@toPrimitive](hint)` (section 19.4.3.5).
fn symbol_to_primitive(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    this_symbol(interp, &this, "[Symbol.toPrimitive]").map(Value::Symbol)
}

/// `Date.prototype[@@toPrimitive](hint)` (section 20.4.4.45): OrdinaryToPrimitive of any object,
/// `toString` first for the hints `"string"` and `"default"` and `valueOf` first for `"number"`.
/// Any other hint, or a `this` that is not an object, throws a `TypeError`.
fn date_to_primitive(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let Value::Object(o) = this else {
        return Err(interp.type_error("Date.prototype[Symbol.toPrimitive] was called on a value that is not an object"));
    };
    // The longest hint has 7 units, so a longer string is refused before it is flattened.
    let name = match arg(args, 0) {
        Value::String(s) if s.len() <= 7 => String::from_utf16_lossy(&s.flat()),
        _ => String::new(),
    };
    let hint = match name.as_str() {
        "string" | "default" => Hint::String,
        "number" => Hint::Number,
        _ => return Err(interp.type_error("Date.prototype[Symbol.toPrimitive] was given a hint that is not \"string\", \"number\" or \"default\"")),
    };
    interp.ordinary_to_primitive(o, &this, hint)
}

/// `Object.getOwnPropertySymbols(o)` (section 19.1.2.10): the object's own Symbol keys in the
/// order they were added.
fn get_own_property_symbols(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = interp.to_object(&arg(args, 0))?;
    interp.rooted(&[Value::Object(o)], |interp| {
        let syms = interp.own_symbols(o)?;
        let a = new_array(interp, 0)?;
        interp.rooted(&[Value::Object(a)], |interp| {
            for (i, s) in syms.into_iter().enumerate() {
                interp.use_fuel()?;
                create_at(interp, a, i as f64, Value::Symbol(s))?;
            }
            Ok(Value::Object(a))
        })
    })
}

/// `Function.prototype[@@hasInstance](v)` (section 19.2.3.6): OrdinaryHasInstance.
fn function_has_instance(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    interp.ordinary_has_instance(&arg(args, 0), &this).map(Value::Bool)
}
