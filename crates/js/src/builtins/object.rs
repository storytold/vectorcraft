//! `Object`'s functions (ECMA-262 5.1 section 15.2.3). A primitive given where an object is
//! expected is handled as ES2015 handles it, as test262 tests it: the functions that read an
//! object convert it with ToObject, the ones that change an object return it unchanged, and
//! `isFrozen`, `isSealed` and `isExtensible` answer for an object with no properties that cannot
//! be extended.

use super::{ELEMENT_BYTES, arg, create_at, method, new_array};
use crate::heap::{CONFIGURABLE, ENUMERABLE, Key, OBJECT_BYTES, OPEN, ObjId, Object, Prop, WRITABLE, key};
use crate::interp::{Desc, Interp, JsResult};
use crate::proxy::desc_values;
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

/// ToPropertyDescriptor (section 8.10.5).
pub(crate) fn to_desc(interp: &mut Interp, v: &Value) -> JsResult<Desc> {
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

/// FromPropertyDescriptor (ECMA-262 2020 section 6.2.5.4) of a descriptor that may leave fields
/// out: an object with the fields `desc` has, in the order `value`, `writable`, `get`, `set`,
/// `enumerable`, `configurable`.
pub(crate) fn from_desc(interp: &mut Interp, desc: &Desc) -> JsResult<Value> {
    let roots = desc_values(desc);
    interp.rooted(&roots, |interp| interp.reserve_heap(OBJECT_BYTES + 6 * ELEMENT_BYTES))?;
    let o = interp.new_object();
    let fields = [
        ("value", desc.value.clone()),
        ("writable", desc.writable.map(Value::Bool)),
        ("get", desc.get.map(|g| g.map_or(Value::Undefined, Value::Object))),
        ("set", desc.set.map(|s| s.map_or(Value::Undefined, Value::Object))),
        ("enumerable", desc.enumerable.map(Value::Bool)),
        ("configurable", desc.configurable.map(Value::Bool)),
    ];
    for (name, v) in fields {
        if let Some(v) = v {
            interp.define(o, key(name), Prop::Data { value: v, flags: OPEN })?;
        }
    }
    Ok(Value::Object(o))
}

/// FromPropertyDescriptor (section 8.10.4). The property's values stay rooted while the
/// descriptor object is made, since a proxy's trap can give values no other object holds.
pub(crate) fn from_prop(interp: &mut Interp, prop: &Prop) -> JsResult<Value> {
    let roots: Vec<Value> = match prop {
        Prop::Data { value, .. } => vec![value.clone()],
        Prop::Accessor { get, set, .. } => get.iter().chain(set).map(|f| Value::Object(*f)).collect(),
    };
    interp.rooted(&roots, |interp| descriptor_object(interp, prop))
}

/// The descriptor object of `prop`, whose values the caller keeps rooted.
fn descriptor_object(interp: &mut Interp, prop: &Prop) -> JsResult<Value> {
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
pub(crate) fn names_array(interp: &mut Interp, names: Vec<Key>) -> JsResult<Value> {
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
    Ok(interp.rooted(&[Value::Object(o)], |interp| interp.get_prototype_of(o))?.map_or(Value::Null, Value::Object))
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
    interp.with_key(&arg(args, 1), |interp, k| {
        let desc = to_desc(interp, &arg(args, 2))?;
        let roots = desc_values(&desc);
        interp.rooted(&roots, |interp| interp.define_own_property(o, k, &desc, true))
    })?;
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

/// SetIntegrityLevel (ECMA-262 2020 section 7.3.14): makes the object non-extensible, then every
/// own property non-configurable, and with `freeze` every data property read-only (sections
/// 15.2.3.8 and 15.2.3.9). A proxy that does not become non-extensible is a `TypeError`.
fn lock(interp: &mut Interp, v: Value, freeze: bool) -> JsResult<Value> {
    let Value::Object(o) = v else { return Ok(v) };
    if !interp.prevent_extensions(o)? {
        let what = if freeze { "frozen" } else { "sealed" };
        return Err(interp.type_error(&format!("the object cannot be {what}: it did not stop being extensible")));
    }
    for k in interp.own_keys(o)? {
        interp.use_fuel()?;
        let desc = if freeze {
            let Some(prop) = interp.get_own_property(o, &k)? else { continue };
            match prop {
                Prop::Data { .. } => Desc { configurable: Some(false), writable: Some(false), ..Desc::default() },
                Prop::Accessor { .. } => Desc { configurable: Some(false), ..Desc::default() },
            }
        } else {
            Desc { configurable: Some(false), ..Desc::default() }
        };
        interp.define_own_property(o, k, &desc, true)?;
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
        && !interp.prevent_extensions(o)?
    {
        return Err(interp.type_error("Object.preventExtensions: the proxy did not stop being extensible"));
    }
    Ok(v)
}

/// TestIntegrityLevel (ECMA-262 2020 section 7.3.15): whether the object is not extensible and no
/// own property is configurable (and with `frozen`, no data property is writable) (sections
/// 15.2.3.11 and 15.2.3.12).
fn locked(interp: &mut Interp, v: &Value, frozen: bool) -> JsResult<Value> {
    let Value::Object(o) = v else { return Ok(Value::Bool(true)) };
    let o = *o;
    if interp.is_extensible(o)? {
        return Ok(Value::Bool(false));
    }
    for k in interp.own_keys(o)? {
        interp.use_fuel()?;
        let Some(prop) = interp.get_own_property(o, &k)? else { continue };
        if prop.configurable() || (frozen && matches!(prop, Prop::Data { flags, .. } if flags & WRITABLE != 0)) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
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
    interp.is_extensible(o).map(Value::Bool)
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
