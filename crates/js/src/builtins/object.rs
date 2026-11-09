//! `Object`'s functions (ECMA-262 5.1 section 15.2.3). A primitive given where an object is
//! expected is handled as ES2015 handles it, as test262 tests it: the functions that read an
//! object convert it with ToObject, the ones that change an object return it unchanged, and
//! `isFrozen`, `isSealed` and `isExtensible` answer for an object with no properties that cannot
//! be extended.

use super::{ELEMENT_BYTES, arg, create_at, method, new_array};
use crate::heap::{CONFIGURABLE, ENUMERABLE, Key, OBJECT_BYTES, OPEN, ObjId, Object, Prop, WRITABLE, key};
use crate::interp::{Desc, Interp, JsResult};
use crate::value::{Str, Value};

pub(super) fn install(interp: &mut Interp, object: ObjId) -> JsResult<()> {
    method(interp, object, "getPrototypeOf", get_prototype_of, 1)?;
    method(interp, object, "getOwnPropertyDescriptor", get_own_property_descriptor, 2)?;
    method(interp, object, "getOwnPropertyNames", get_own_property_names, 1)?;
    method(interp, object, "create", create, 2)?;
    method(interp, object, "defineProperty", define_property, 3)?;
    method(interp, object, "defineProperties", define_properties, 2)?;
    method(interp, object, "seal", seal, 1)?;
    method(interp, object, "freeze", freeze, 1)?;
    method(interp, object, "preventExtensions", prevent_extensions, 1)?;
    method(interp, object, "isSealed", is_sealed, 1)?;
    method(interp, object, "isFrozen", is_frozen, 1)?;
    method(interp, object, "isExtensible", is_extensible, 1)?;
    method(interp, object, "keys", keys, 1)
}

fn require_object(interp: &mut Interp, v: &Value, function: &str) -> JsResult<ObjId> {
    match v {
        Value::Object(o) => Ok(*o),
        _ => Err(interp.type_error(&format!("Object.{function} was given a value that is not an object"))),
    }
}

/// The values a descriptor holds, to keep them reachable while more JavaScript runs.
fn desc_values(desc: &Desc) -> Vec<Value> {
    let mut out: Vec<Value> = desc.value.iter().cloned().collect();
    out.extend(desc.get.flatten().map(Value::Object));
    out.extend(desc.set.flatten().map(Value::Object));
    out
}

/// ToPropertyDescriptor (section 8.10.5).
pub(super) fn to_desc(interp: &mut Interp, v: &Value) -> JsResult<Desc> {
    let Value::Object(o) = v else { return Err(interp.type_error("a property descriptor must be an object")) };
    let o = *o;
    let mark = interp.temp_roots.len();
    let r = read_desc(interp, o);
    interp.temp_roots.truncate(mark);
    r
}

fn read_desc(interp: &mut Interp, o: ObjId) -> JsResult<Desc> {
    let receiver = Value::Object(o);
    let field = |interp: &mut Interp, name: &str| -> JsResult<Option<Value>> {
        let k = key(name);
        if !interp.has_property(o, &k)? {
            return Ok(None);
        }
        let v = interp.get_from(o, &k, &receiver)?;
        interp.temp_roots.push(v.clone());
        Ok(Some(v))
    };
    let mut desc = Desc {
        enumerable: field(interp, "enumerable")?.map(|v| v.truthy()),
        configurable: field(interp, "configurable")?.map(|v| v.truthy()),
        value: field(interp, "value")?,
        writable: field(interp, "writable")?.map(|v| v.truthy()),
        ..Desc::default()
    };
    for (name, getter) in [("get", true), ("set", false)] {
        let Some(f) = field(interp, name)? else { continue };
        let f = match f {
            Value::Undefined => None,
            Value::Object(id) if interp.is_callable(&f) => Some(id),
            _ => return Err(interp.type_error(&format!("a property's {name} must be a function or undefined"))),
        };
        if getter {
            desc.get = Some(f);
        } else {
            desc.set = Some(f);
        }
    }
    if desc.is_accessor() && desc.is_data() {
        return Err(interp.type_error("a property descriptor cannot have both a value or writable and a get or set"));
    }
    Ok(desc)
}

/// FromPropertyDescriptor (section 8.10.4).
fn from_prop(interp: &mut Interp, prop: &Prop) -> JsResult<Value> {
    interp.reserve_heap(OBJECT_BYTES + 4 * ELEMENT_BYTES)?;
    let o = interp.new_object();
    let flags = prop.flags();
    match prop {
        Prop::Data { value, .. } => {
            interp.define(o, key("value"), Prop::Data { value: value.clone(), flags: OPEN })?;
            interp.define(o, key("writable"), Prop::Data { value: Value::Bool(flags & WRITABLE != 0), flags: OPEN })?;
        }
        Prop::Accessor { get, set, .. } => {
            interp.define(o, key("get"), Prop::Data { value: get.map_or(Value::Undefined, Value::Object), flags: OPEN })?;
            interp.define(o, key("set"), Prop::Data { value: set.map_or(Value::Undefined, Value::Object), flags: OPEN })?;
        }
    }
    interp.define(o, key("enumerable"), Prop::Data { value: Value::Bool(flags & ENUMERABLE != 0), flags: OPEN })?;
    interp.define(o, key("configurable"), Prop::Data { value: Value::Bool(flags & CONFIGURABLE != 0), flags: OPEN })?;
    Ok(Value::Object(o))
}

/// An array of the names, kept reachable while it is filled.
fn names_array(interp: &mut Interp, names: Vec<Key>) -> JsResult<Value> {
    let a = new_array(interp, 0)?;
    interp.rooted(&[Value::Object(a)], |interp| {
        for (i, k) in names.into_iter().enumerate() {
            interp.use_fuel()?;
            create_at(interp, a, i as f64, Value::String(Str::from_rc(k)))?;
        }
        Ok(Value::Object(a))
    })
}

/// `Object.getPrototypeOf` (section 15.2.3.2).
fn get_prototype_of(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = interp.to_object(&arg(args, 0))?;
    Ok(interp.heap.object(o).and_then(|o| o.proto).map_or(Value::Null, Value::Object))
}

/// `Object.getOwnPropertyDescriptor` (section 15.2.3.3).
fn get_own_property_descriptor(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = interp.to_object(&arg(args, 0))?;
    interp.rooted(&[Value::Object(o)], |interp| {
        let k = interp.to_key(&arg(args, 1))?;
        match interp.get_own_property(o, &k)? {
            Some(p) => from_prop(interp, &p),
            None => Ok(Value::Undefined),
        }
    })
}

/// `Object.getOwnPropertyNames` (section 15.2.3.4).
fn get_own_property_names(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = interp.to_object(&arg(args, 0))?;
    interp.rooted(&[Value::Object(o)], |interp| {
        let names = interp.own_keys(o)?;
        names_array(interp, names)
    })
}

/// `Object.create` (section 15.2.3.5).
fn create(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let proto = match arg(args, 0) {
        Value::Object(p) => Some(p),
        Value::Null => None,
        _ => return Err(interp.type_error("Object.create's prototype must be an object or null")),
    };
    interp.reserve_heap(OBJECT_BYTES)?;
    let o = interp.heap.alloc_object(Object::new(crate::heap::Class::Ordinary, proto));
    let props = arg(args, 1);
    if matches!(props, Value::Undefined) {
        return Ok(Value::Object(o));
    }
    interp.rooted(&[Value::Object(o)], |interp| define_all(interp, o, &props))?;
    Ok(Value::Object(o))
}

/// `Object.defineProperty` (section 15.2.3.6).
fn define_property(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = require_object(interp, &arg(args, 0), "defineProperty")?;
    let k = interp.to_key(&arg(args, 1))?;
    let desc = to_desc(interp, &arg(args, 2))?;
    let roots = desc_values(&desc);
    interp.rooted(&roots, |interp| interp.define_own_property(o, k, &desc, true))?;
    Ok(Value::Object(o))
}

/// `Object.defineProperties` (section 15.2.3.7).
fn define_properties(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = require_object(interp, &arg(args, 0), "defineProperties")?;
    define_all(interp, o, &arg(args, 1))?;
    Ok(Value::Object(o))
}

/// Defines on `o` each own enumerable property of `props`, reading every descriptor before
/// defining any.
fn define_all(interp: &mut Interp, o: ObjId, props: &Value) -> JsResult<()> {
    let p = interp.to_object(props)?;
    let mark = interp.temp_roots.len();
    interp.temp_roots.push(Value::Object(p));
    let r = (|| {
        let mut descs = Vec::new();
        for k in interp.own_keys(p)? {
            interp.use_fuel()?;
            if !interp.get_own_property(p, &k)?.is_some_and(|prop| prop.enumerable()) {
                continue;
            }
            let v = interp.get_from(p, &k, &Value::Object(p))?;
            interp.temp_roots.push(v.clone());
            let desc = to_desc(interp, &v)?;
            interp.temp_roots.extend(desc_values(&desc));
            descs.push((k, desc));
        }
        for (k, desc) in descs {
            interp.use_fuel()?;
            interp.define_own_property(o, k, &desc, true)?;
        }
        Ok(())
    })();
    interp.temp_roots.truncate(mark);
    r
}

/// Makes every own property non-configurable, and with `freeze` every data property read-only,
/// then makes the object non-extensible (sections 15.2.3.8 and 15.2.3.9).
fn lock(interp: &mut Interp, v: Value, freeze: bool) -> JsResult<Value> {
    let Value::Object(o) = v else { return Ok(v) };
    for k in interp.own_keys(o)? {
        interp.use_fuel()?;
        let Some(prop) = interp.get_own_property(o, &k)? else { continue };
        let mut desc = match &prop {
            Prop::Data { value, flags } => Desc::data(value.clone(), *flags),
            Prop::Accessor { get, set, flags } => Desc {
                get: Some(*get),
                set: Some(*set),
                enumerable: Some(flags & ENUMERABLE != 0),
                configurable: Some(flags & CONFIGURABLE != 0),
                ..Desc::default()
            },
        };
        desc.configurable = Some(false);
        if freeze && desc.is_data() {
            desc.writable = Some(false);
        }
        interp.define_own_property(o, k, &desc, true)?;
    }
    if let Some(obj) = interp.heap.object_mut(o) {
        obj.extensible = false;
    }
    Ok(Value::Object(o))
}

fn seal(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    lock(interp, arg(args, 0), false)
}

fn freeze(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    lock(interp, arg(args, 0), true)
}

/// `Object.preventExtensions` (section 15.2.3.10).
fn prevent_extensions(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let v = arg(args, 0);
    if let Value::Object(o) = v
        && let Some(obj) = interp.heap.object_mut(o)
    {
        obj.extensible = false;
    }
    Ok(v)
}

/// Whether no own property is configurable (and with `frozen`, no data property is writable)
/// and the object is not extensible (sections 15.2.3.11 and 15.2.3.12).
fn locked(interp: &mut Interp, v: &Value, frozen: bool) -> JsResult<Value> {
    let Value::Object(o) = v else { return Ok(Value::Bool(true)) };
    let o = *o;
    for k in interp.own_keys(o)? {
        interp.use_fuel()?;
        let Some(prop) = interp.get_own_property(o, &k)? else { continue };
        if prop.configurable() || (frozen && matches!(prop, Prop::Data { flags, .. } if flags & WRITABLE != 0)) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(!interp.heap.object(o).is_some_and(|o| o.extensible)))
}

fn is_sealed(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    locked(interp, &arg(args, 0), false)
}

fn is_frozen(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    locked(interp, &arg(args, 0), true)
}

/// `Object.isExtensible` (section 15.2.3.13).
fn is_extensible(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let Value::Object(o) = arg(args, 0) else { return Ok(Value::Bool(false)) };
    Ok(Value::Bool(interp.heap.object(o).is_some_and(|o| o.extensible)))
}

/// `Object.keys` (section 15.2.3.14).
fn keys(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = interp.to_object(&arg(args, 0))?;
    interp.rooted(&[Value::Object(o)], |interp| {
        let mut names = Vec::new();
        for k in interp.own_keys(o)? {
            interp.use_fuel()?;
            if interp.get_own_property(o, &k)?.is_some_and(|p| p.enumerable()) {
                names.push(k);
            }
        }
        names_array(interp, names)
    })
}
