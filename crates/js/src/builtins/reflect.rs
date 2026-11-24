//! `Proxy` and `Reflect` (ECMA-262 2020 sections 26.2 and 26.1,
//! <https://262.ecma-international.org/11.0/#sec-reflection>), installed in a realm with
//! [`crate::Options::es2015`]. The proxy objects themselves are in `crate::proxy`.

use std::rc::Rc;

use super::object::{from_prop, names_array, to_desc};
use super::{arg, data, method, native};
use crate::heap::{Class, Func, HIDDEN, NativeKind, OBJECT_BYTES, OPEN, ObjId, Object, Prop, key};
use crate::interp::{Desc, Interp, JsResult};
use crate::proxy::desc_values;
use crate::value::Value;

/// Defines `Proxy` and `Reflect` on the global object. With `keep`, for a realm that gets them
/// after earlier runs, a global of the same name that a script defined stays as it is. Each
/// global is defined after its functions, so an install that stops part way leaves no global
/// that a later install would keep without them.
pub(super) fn install(interp: &mut Interp, keep: bool) -> JsResult<()> {
    let global = interp.global;
    if !(keep && interp.get_own_property(global, &key("Proxy"))?.is_some()) {
        let proxy = native(interp, "Proxy", proxy_ctor, 2, NativeKind::Plain, true)?;
        method(interp, proxy, "revocable", revocable, 2)?;
        data(interp, global, "Proxy", Value::Object(proxy), HIDDEN)?;
    }
    if !(keep && interp.get_own_property(global, &key("Reflect"))?.is_some()) {
        install_reflect(interp)?;
    }
    interp.es2015_installed = true;
    Ok(())
}

/// The `Reflect` object and its functions (section 26.1).
fn install_reflect(interp: &mut Interp) -> JsResult<()> {
    let global = interp.global;
    let object_proto = interp.intrinsics.object_proto;
    let reflect = interp.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));
    method(interp, reflect, "apply", apply, 3)?;
    method(interp, reflect, "construct", construct, 2)?;
    method(interp, reflect, "defineProperty", define_property, 3)?;
    method(interp, reflect, "deleteProperty", delete_property, 2)?;
    method(interp, reflect, "get", get, 2)?;
    method(interp, reflect, "getOwnPropertyDescriptor", get_own_property_descriptor, 2)?;
    method(interp, reflect, "getPrototypeOf", get_prototype_of, 1)?;
    method(interp, reflect, "has", has, 2)?;
    method(interp, reflect, "isExtensible", is_extensible, 1)?;
    method(interp, reflect, "ownKeys", own_keys, 1)?;
    method(interp, reflect, "preventExtensions", prevent_extensions, 1)?;
    method(interp, reflect, "set", set, 3)?;
    method(interp, reflect, "setPrototypeOf", set_prototype_of, 2)?;
    data(interp, global, "Reflect", Value::Object(reflect), HIDDEN)
}

// ---- Proxy ----

/// `new Proxy(target, handler)` (section 26.2.1.1). Calling it without `new` is a `TypeError`.
fn proxy_ctor(interp: &mut Interp, _: Value, args: &[Value], constructing: bool) -> JsResult<Value> {
    if !constructing {
        return Err(interp.type_error("Proxy must be called with new"));
    }
    interp.proxy_create(&arg(args, 0), &arg(args, 1)).map(Value::Object)
}

/// `Proxy.revocable(target, handler)` (section 26.2.2.1): an object whose `proxy` is a new proxy
/// and whose `revoke` function revokes it. The function holds the proxy as the `this` of a bound
/// call to [`revoke`], and calling it again does nothing.
fn revocable(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let p = interp.proxy_create(&arg(args, 0), &arg(args, 1))?;
    interp.rooted(&[Value::Object(p)], |interp| {
        interp.reserve_heap(3 * OBJECT_BYTES)?;
        let inner = native(interp, "", revoke, 0, NativeKind::Plain, false)?;
        let proto = interp.intrinsics.function_proto;
        let class = Class::Function(Func::Bound { target: inner, this: Value::Object(p), args: Rc::from(Vec::new()) });
        let f = interp.heap.alloc_object(Object::new(class, Some(proto)));
        interp.define(f, key("length"), Prop::Data { value: Value::Number(0.0), flags: 0 })?;
        let result = interp.new_object();
        interp.define(result, key("proxy"), Prop::Data { value: Value::Object(p), flags: OPEN })?;
        interp.define(result, key("revoke"), Prop::Data { value: Value::Object(f), flags: OPEN })?;
        Ok(Value::Object(result))
    })
}

/// A revocation function's call (section 26.2.2.1.1): `this` is the proxy it revokes.
fn revoke(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    if let Value::Object(p) = this {
        interp.proxy_revoke(p);
    }
    Ok(Value::Undefined)
}

// ---- Reflect ----

/// The `target` argument of a `Reflect` function, which must be an object.
fn target(interp: &mut Interp, args: &[Value], function: &str) -> JsResult<ObjId> {
    match arg(args, 0) {
        Value::Object(o) => Ok(o),
        _ => Err(interp.type_error(&format!("Reflect.{function}: the target is not an object"))),
    }
}

/// CreateListFromArrayLike (section 7.3.17) of an argument list, which must be an object.
fn argument_list(interp: &mut Interp, list: &Value, function: &str) -> JsResult<Vec<Value>> {
    if !matches!(list, Value::Object(_)) {
        return Err(interp.type_error(&format!("Reflect.{function}: the argument list is not an object")));
    }
    interp.list_from_array_like(list, true)
}

/// `Reflect.apply(target, thisArgument, argumentsList)` (section 26.1.1).
fn apply(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let f = arg(args, 0);
    if !interp.is_callable(&f) {
        return Err(interp.type_error("Reflect.apply: the target is not a function"));
    }
    let list = argument_list(interp, &arg(args, 2), "apply")?;
    let mark = interp.temp_roots.len();
    interp.temp_roots.extend(list.iter().cloned());
    let r = interp.call(&f, &arg(args, 1), &list);
    interp.temp_roots.truncate(mark);
    r
}

/// `Reflect.construct(target, argumentsList[, newTarget])` (section 26.1.2). Without
/// `newTarget` the target is constructed with its own prototype. A constructor written in
/// JavaScript gives its new object the `prototype` of `newTarget`; a built-in constructor makes
/// its object as it does for `new`.
fn construct(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let f = arg(args, 0);
    if !interp.is_constructor(&f) {
        return Err(interp.type_error("Reflect.construct: the target is not a constructor"));
    }
    let new_target = if args.len() > 2 { arg(args, 2) } else { f.clone() };
    if !interp.is_constructor(&new_target) {
        return Err(interp.type_error("Reflect.construct: newTarget is not a constructor"));
    }
    let list = argument_list(interp, &arg(args, 1), "construct")?;
    let mark = interp.temp_roots.len();
    interp.temp_roots.extend(list.iter().cloned());
    let r = interp.construct(&f, &list, &new_target);
    interp.temp_roots.truncate(mark);
    r
}

/// `Reflect.defineProperty(target, propertyKey, attributes)` (section 26.1.3): whether the
/// property was defined.
fn define_property(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = target(interp, args, "defineProperty")?;
    interp.with_key(&arg(args, 1), |interp, k| {
        let desc: Desc = to_desc(interp, &arg(args, 2))?;
        let roots = desc_values(&desc);
        interp.rooted(&roots, |interp| interp.define_own_property(o, k, &desc, false)).map(Value::Bool)
    })
}

/// `Reflect.deleteProperty(target, propertyKey)` (section 26.1.4): whether the property is gone.
fn delete_property(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = target(interp, args, "deleteProperty")?;
    interp.with_key(&arg(args, 1), |interp, k| interp.delete(o, &k, false)).map(Value::Bool)
}

/// `Reflect.get(target, propertyKey[, receiver])` (section 26.1.5).
fn get(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = target(interp, args, "get")?;
    let receiver = if args.len() > 2 { arg(args, 2) } else { Value::Object(o) };
    interp.with_key(&arg(args, 1), |interp, k| interp.get_from(o, &k, &receiver))
}

/// `Reflect.getOwnPropertyDescriptor(target, propertyKey)` (section 26.1.6).
fn get_own_property_descriptor(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = target(interp, args, "getOwnPropertyDescriptor")?;
    interp.with_key(&arg(args, 1), |interp, k| match interp.get_own_property(o, &k)? {
        Some(p) => from_prop(interp, &p),
        None => Ok(Value::Undefined),
    })
}

/// `Reflect.getPrototypeOf(target)` (section 26.1.7).
fn get_prototype_of(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = target(interp, args, "getPrototypeOf")?;
    Ok(interp.get_prototype_of(o)?.map_or(Value::Null, Value::Object))
}

/// `Reflect.has(target, propertyKey)` (section 26.1.8).
fn has(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = target(interp, args, "has")?;
    interp.with_key(&arg(args, 1), |interp, k| interp.has_property(o, &k)).map(Value::Bool)
}

/// `Reflect.isExtensible(target)` (section 26.1.9).
fn is_extensible(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = target(interp, args, "isExtensible")?;
    interp.is_extensible(o).map(Value::Bool)
}

/// `Reflect.ownKeys(target)` (section 26.1.10): every own property name, enumerable or not.
fn own_keys(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = target(interp, args, "ownKeys")?;
    let names = interp.own_keys(o)?;
    names_array(interp, names)
}

/// `Reflect.preventExtensions(target)` (section 26.1.11): whether the object is now not
/// extensible.
fn prevent_extensions(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = target(interp, args, "preventExtensions")?;
    interp.prevent_extensions(o).map(Value::Bool)
}

/// `Reflect.set(target, propertyKey, V[, receiver])` (section 26.1.12): whether the value was
/// set.
fn set(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = target(interp, args, "set")?;
    let receiver = if args.len() > 3 { arg(args, 3) } else { Value::Object(o) };
    let v = arg(args, 2);
    interp.with_key(&arg(args, 1), |interp, k| interp.set_with_receiver(o, k, v, &receiver)).map(Value::Bool)
}

/// `Reflect.setPrototypeOf(target, proto)` (section 26.1.13): whether the prototype was set. A
/// `proto` that is neither an object nor `null` is a `TypeError`.
fn set_prototype_of(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = target(interp, args, "setPrototypeOf")?;
    let proto = match arg(args, 1) {
        Value::Object(p) => Some(p),
        Value::Null => None,
        _ => return Err(interp.type_error("Reflect.setPrototypeOf: the prototype is neither an object nor null")),
    };
    interp.set_prototype_of(o, proto).map(Value::Bool)
}
