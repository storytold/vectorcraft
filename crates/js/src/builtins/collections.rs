//! `Map`, `Set`, `WeakMap` and `WeakSet` (ECMA-262 6.0 sections 23.1 to 23.4), and the iterator
//! objects that `keys`, `values` and `entries` give for Maps, Sets and typed arrays (sections
//! 23.1.5, 23.2.5 and 22.1.5). A realm has them with [`crate::Options::es2015`].
//!
//! Keys compare with SameValueZero, and a key of `-0` is stored as `+0`. Entries keep the order
//! they were added in; deleting one leaves a hole, so `forEach` and an iterator visit the entries
//! added while they run and skip the ones deleted before they reach them. WeakMap and WeakSet
//! entries hold their key weakly: the collector removes an entry once its key is reachable only
//! through it ([`crate::heap::Heap::collect`]), and they have no size and cannot be listed.
//!
//! The language has no iterators yet, so the constructors read their argument by index: an
//! array or an object with a `length`, or a string by code point. Every entry added is checked
//! against the heap limit before it is added, and every entry visited takes a unit of fuel.

use super::{arg, constructor, create_at, key_of, method, native, new_array, require_callable};
use crate::heap::{
    COLL_ENTRY_BYTES, CONFIGURABLE, Class, Coll, CollKey, IterKind, IterSource, IterState, NativeFn, NativeKind, OBJECT_BYTES, OPEN, ObjId, Object,
    PROP_BYTES, Prop, VIEW_BYTES, WEAK_ENTRY_BYTES, WeakColl, key, string_bytes,
};
use crate::interp::{Abrupt, Interp, JsResult};
use crate::string_builtins::flat_charged;
use crate::value::{Str, Value};

/// The ECMAScript 2015 intrinsics the built-ins make their objects with, whatever a script does
/// to the globals.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Es2015 {
    pub(crate) map_proto: ObjId,
    pub(crate) set_proto: ObjId,
    pub(crate) weak_map_proto: ObjId,
    pub(crate) weak_set_proto: ObjId,
    pub(crate) map_iter_proto: ObjId,
    pub(crate) set_iter_proto: ObjId,
    pub(crate) array_iter_proto: ObjId,
    pub(crate) array_buffer_proto: ObjId,
    pub(crate) data_view_proto: ObjId,
    /// `%TypedArray%` and its prototype (section 22.2.1).
    pub(crate) typed_array_ctor: ObjId,
    pub(crate) typed_array_proto: ObjId,
    /// The typed array constructors and prototypes, in the order of [`crate::heap::Elem::ALL`].
    pub(crate) typed_ctors: [ObjId; 9],
    pub(crate) typed_protos: [ObjId; 9],
}

impl Es2015 {
    pub(crate) fn placeholder() -> Self {
        let o = ObjId(0);
        Self {
            map_proto: o,
            set_proto: o,
            weak_map_proto: o,
            weak_set_proto: o,
            map_iter_proto: o,
            set_iter_proto: o,
            array_iter_proto: o,
            array_buffer_proto: o,
            data_view_proto: o,
            typed_array_ctor: o,
            typed_array_proto: o,
            typed_ctors: [o; 9],
            typed_protos: [o; 9],
        }
    }

    pub(crate) fn all(&self) -> Vec<ObjId> {
        let mut v = vec![
            self.map_proto,
            self.set_proto,
            self.weak_map_proto,
            self.weak_set_proto,
            self.map_iter_proto,
            self.set_iter_proto,
            self.array_iter_proto,
            self.array_buffer_proto,
            self.data_view_proto,
            self.typed_array_ctor,
            self.typed_array_proto,
        ];
        v.extend(self.typed_ctors);
        v.extend(self.typed_protos);
        v
    }
}

/// A new ordinary object with `proto`, for a prototype.
pub(crate) fn new_proto(interp: &mut Interp, proto: ObjId) -> ObjId {
    interp.heap.alloc_object(Object::new(Class::Ordinary, Some(proto)))
}

/// Defines a getter `name` on `on`, configurable and not enumerable, as the prototype accessors
/// of ECMAScript 2015 are.
pub(crate) fn getter(interp: &mut Interp, on: ObjId, name: &'static str, f: NativeFn) -> JsResult<()> {
    let g = native(interp, name, f, 0, NativeKind::Plain, false)?;
    let prop_name = name.strip_prefix("get ").unwrap_or(name);
    interp.define(on, key(prop_name), Prop::Accessor { get: Some(g), set: None, flags: CONFIGURABLE })
}

pub(super) fn install(interp: &mut Interp) -> JsResult<()> {
    let object_proto = interp.intrinsics.object_proto;

    let map_proto = new_proto(interp, object_proto);
    interp.intrinsics.es2015.map_proto = map_proto;
    let map = native(interp, "Map", map_ctor, 0, NativeKind::Plain, true)?;
    constructor(interp, "Map", map, map_proto)?;
    let map_methods: [(&'static str, NativeFn, u32); 9] = [
        ("get", map_get, 1),
        ("set", map_set, 2),
        ("has", map_has, 1),
        ("delete", map_delete, 1),
        ("clear", map_clear, 0),
        ("forEach", map_for_each, 1),
        ("keys", map_keys, 0),
        ("values", map_values, 0),
        ("entries", map_entries, 0),
    ];
    for (name, f, length) in map_methods {
        method(interp, map_proto, name, f, length)?;
    }
    getter(interp, map_proto, "get size", map_size)?;

    let set_proto = new_proto(interp, object_proto);
    interp.intrinsics.es2015.set_proto = set_proto;
    let set = native(interp, "Set", set_ctor, 0, NativeKind::Plain, true)?;
    constructor(interp, "Set", set, set_proto)?;
    let set_methods: [(&'static str, NativeFn, u32); 7] = [
        ("has", set_has, 1),
        ("add", set_add, 1),
        ("delete", set_delete, 1),
        ("clear", set_clear, 0),
        ("entries", set_entries, 0),
        ("forEach", set_for_each, 1),
        ("values", set_values, 0),
    ];
    for (name, f, length) in set_methods {
        method(interp, set_proto, name, f, length)?;
    }
    getter(interp, set_proto, "get size", set_size)?;
    // `Set.prototype.keys` is the same function object as `values` (section 23.2.3.8).
    let values = interp.get_from(set_proto, &key("values"), &Value::Object(set_proto))?;
    interp.define(set_proto, key("keys"), Prop::Data { value: values, flags: crate::heap::HIDDEN })?;

    let weak_map_proto = new_proto(interp, object_proto);
    interp.intrinsics.es2015.weak_map_proto = weak_map_proto;
    let weak_map = native(interp, "WeakMap", weak_map_ctor, 0, NativeKind::Plain, true)?;
    constructor(interp, "WeakMap", weak_map, weak_map_proto)?;
    let weak_map_methods: [(&'static str, NativeFn, u32); 4] =
        [("delete", weak_map_delete, 1), ("get", weak_map_get, 1), ("has", weak_map_has, 1), ("set", weak_map_set, 2)];
    for (name, f, length) in weak_map_methods {
        method(interp, weak_map_proto, name, f, length)?;
    }

    let weak_set_proto = new_proto(interp, object_proto);
    interp.intrinsics.es2015.weak_set_proto = weak_set_proto;
    let weak_set = native(interp, "WeakSet", weak_set_ctor, 0, NativeKind::Plain, true)?;
    constructor(interp, "WeakSet", weak_set, weak_set_proto)?;
    let weak_set_methods: [(&'static str, NativeFn, u32); 3] = [("add", weak_set_add, 1), ("delete", weak_set_delete, 1), ("has", weak_set_has, 1)];
    for (name, f, length) in weak_set_methods {
        method(interp, weak_set_proto, name, f, length)?;
    }

    for source in [IterSource::Map, IterSource::Set, IterSource::Array] {
        let proto = new_proto(interp, object_proto);
        let next: NativeFn = match source {
            IterSource::Map => map_iterator_next,
            IterSource::Set => set_iterator_next,
            IterSource::Array => array_iterator_next,
        };
        method(interp, proto, "next", next, 0)?;
        match source {
            IterSource::Map => interp.intrinsics.es2015.map_iter_proto = proto,
            IterSource::Set => interp.intrinsics.es2015.set_iter_proto = proto,
            IterSource::Array => interp.intrinsics.es2015.array_iter_proto = proto,
        }
    }
    Ok(())
}

// ---- keys ----

/// The SameValueZero key of `v`. A string is flattened, its copy charged to the heap.
fn coll_key(interp: &mut Interp, v: &Value) -> JsResult<CollKey> {
    Ok(match v {
        Value::Undefined => CollKey::Undefined,
        Value::Null => CollKey::Null,
        Value::Bool(b) => CollKey::Bool(*b),
        Value::Number(n) => CollKey::number(*n),
        Value::String(s) => CollKey::String(flat_charged(interp, s)?),
        Value::Symbol(s) => CollKey::Symbol(s.clone()),
        Value::Object(o) => CollKey::Object(o.0),
    })
}

/// `v`, with `-0` made `+0`, as Map and Set store a key (sections 23.1.3.9 and 23.2.3.1).
fn stored_key(v: Value) -> Value {
    match v {
        // `-0 + 0` is `+0`, and every other number is unchanged by adding `+0`.
        Value::Number(n) => Value::Number(n + 0.0),
        v => v,
    }
}

// ---- reading a list ----

/// ToLength (section 7.1.15) of an object's `length`.
pub(crate) fn length_of(interp: &mut Interp, o: ObjId) -> JsResult<u64> {
    let len = interp.get_from(o, &key("length"), &Value::Object(o))?;
    let n = super::to_integer(interp, &len)?;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(if n <= 0.0 { 0 } else { n.min(9_007_199_254_740_991.0) as u64 })
}

/// Calls `f` with each item of `list`, read by index: an object's elements up to its `length`,
/// or a string's code points. Any other value is a `TypeError`, and so is an object without a
/// `length` when `need_length` is true. Each item takes a unit of fuel.
pub(crate) fn for_each_item(
    interp: &mut Interp,
    list: &Value,
    need_length: bool,
    mut f: impl FnMut(&mut Interp, Value) -> JsResult<()>,
) -> JsResult<()> {
    match list {
        Value::Object(o) => {
            if need_length && !interp.has_property(*o, &key("length"))? {
                return Err(interp.type_error("the value is not an array, a string or an object with a length"));
            }
            let len = length_of(interp, *o)?;
            let mut i = 0u64;
            while i < len {
                interp.use_fuel()?;
                #[allow(clippy::cast_precision_loss)]
                let item = interp.get_from(*o, &key_of(i as f64), list)?;
                f(interp, item)?;
                i += 1;
            }
            Ok(())
        }
        Value::String(s) => {
            let units = flat_charged(interp, s)?;
            let mut i = 0;
            while i < units.len() {
                interp.use_fuel()?;
                let width = match (units.get(i), units.get(i + 1)) {
                    (Some(hi), Some(lo)) if (0xD800..0xDC00).contains(hi) && (0xDC00..0xE000).contains(lo) => 2,
                    _ => 1,
                };
                let piece = units.get(i..i + width).unwrap_or_default().to_vec();
                let bytes = string_bytes(piece.len());
                interp.reserve_heap(bytes)?;
                interp.heap.bytes = interp.heap.bytes.saturating_add(bytes);
                f(interp, Value::String(Str::from_units(piece)))?;
                i += width;
            }
            Ok(())
        }
        _ => Err(interp.type_error("the value is not an array, a string or an object with a length")),
    }
}

/// Adds the items of `list` to the new collection `target` with its `adder` method, as the
/// constructors do (sections 23.1.1.1, 23.2.1.1, 23.3.1.1 and 23.4.1.1); `pairs` reads each item
/// as a `[key, value]` entry.
fn fill(interp: &mut Interp, target: ObjId, list: &Value, adder: &str, pairs: bool) -> JsResult<()> {
    if list.is_nullish() {
        return Ok(());
    }
    let this = Value::Object(target);
    interp.rooted(&[this.clone(), list.clone()], |interp| {
        let add = interp.get_from(target, &key(adder), &this)?;
        require_callable(interp, &add, adder)?;
        interp.rooted(std::slice::from_ref(&add), |interp| {
            for_each_item(interp, list, true, |interp, item| {
                if !pairs {
                    return interp.call(&add, &this, &[item]).map(drop);
                }
                let Value::Object(entry) = item else { return Err(interp.type_error("an entry is not an object")) };
                let k = interp.get_from(entry, &key("0"), &item)?;
                let v = interp.rooted(std::slice::from_ref(&k), |interp| interp.get_from(entry, &key("1"), &item))?;
                interp.call(&add, &this, &[k, v]).map(drop)
            })
        })
    })
}

/// A new collection object of `class` with `proto`, filled from `args[0]`.
fn construct(interp: &mut Interp, args: &[Value], constructing: bool, name: &str, class: Class, proto: ObjId) -> JsResult<Value> {
    if !constructing {
        return Err(interp.type_error(&format!("{name} must be called with new")));
    }
    interp.reserve_heap(OBJECT_BYTES)?;
    let o = interp.heap.alloc_object(Object::new(class, Some(proto)));
    let (adder, pairs) = match name {
        "Map" | "WeakMap" => ("set", true),
        _ => ("add", false),
    };
    fill(interp, o, &arg(args, 0), adder, pairs)?;
    Ok(Value::Object(o))
}

fn map_ctor(interp: &mut Interp, _: Value, args: &[Value], constructing: bool) -> JsResult<Value> {
    let proto = interp.intrinsics.es2015.map_proto;
    construct(interp, args, constructing, "Map", Class::Map(Box::default()), proto)
}

fn set_ctor(interp: &mut Interp, _: Value, args: &[Value], constructing: bool) -> JsResult<Value> {
    let proto = interp.intrinsics.es2015.set_proto;
    construct(interp, args, constructing, "Set", Class::Set(Box::default()), proto)
}

fn weak_map_ctor(interp: &mut Interp, _: Value, args: &[Value], constructing: bool) -> JsResult<Value> {
    let proto = interp.intrinsics.es2015.weak_map_proto;
    construct(interp, args, constructing, "WeakMap", Class::WeakMap(Box::default()), proto)
}

fn weak_set_ctor(interp: &mut Interp, _: Value, args: &[Value], constructing: bool) -> JsResult<Value> {
    let proto = interp.intrinsics.es2015.weak_set_proto;
    construct(interp, args, constructing, "WeakSet", Class::WeakSet(Box::default()), proto)
}

// ---- Map and Set ----

/// The Map (or with `set`, the Set) a method was called on.
fn this_coll(interp: &mut Interp, this: &Value, set: bool, method: &str) -> JsResult<ObjId> {
    if let Value::Object(o) = this
        && let Some(obj) = interp.heap.object(*o)
        && matches!((&obj.class, set), (Class::Map(_), false) | (Class::Set(_), true))
    {
        return Ok(*o);
    }
    let name = if set { "Set" } else { "Map" };
    Err(interp.type_error(&format!("{name}.prototype.{method} was called on a value that is not a {name}")))
}

fn coll(interp: &Interp, o: ObjId) -> JsResult<&Coll> {
    match &interp.object(o)?.class {
        Class::Map(c) | Class::Set(c) => Ok(c),
        _ => Err(Abrupt::Internal("not a Map or a Set".into())),
    }
}

fn coll_mut(interp: &mut Interp, o: ObjId) -> JsResult<&mut Coll> {
    match &mut interp.object_mut(o)?.class {
        Class::Map(c) | Class::Set(c) => Ok(c),
        _ => Err(Abrupt::Internal("not a Map or a Set".into())),
    }
}

/// Adds or replaces the entry of `k`, reserving the heap for a new one first.
fn coll_put(interp: &mut Interp, o: ObjId, k: Value, v: Value) -> JsResult<()> {
    let ck = coll_key(interp, &k)?;
    if !coll(interp, o)?.has(&ck) {
        interp.rooted(&[Value::Object(o), k.clone(), v.clone()], |i| i.reserve_heap(COLL_ENTRY_BYTES))?;
        interp.heap.bytes = interp.heap.bytes.saturating_add(COLL_ENTRY_BYTES);
    }
    coll_mut(interp, o)?.set(ck, stored_key(k), v);
    Ok(())
}

fn map_get(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = this_coll(interp, &this, false, "get")?;
    let k = coll_key(interp, &arg(args, 0))?;
    Ok(coll(interp, o)?.get(&k).cloned().unwrap_or(Value::Undefined))
}

fn map_set(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = this_coll(interp, &this, false, "set")?;
    coll_put(interp, o, arg(args, 0), arg(args, 1))?;
    Ok(this)
}

fn has(interp: &mut Interp, this: &Value, args: &[Value], set: bool) -> JsResult<Value> {
    let o = this_coll(interp, this, set, "has")?;
    let k = coll_key(interp, &arg(args, 0))?;
    Ok(Value::Bool(coll(interp, o)?.has(&k)))
}

fn delete(interp: &mut Interp, this: &Value, args: &[Value], set: bool) -> JsResult<Value> {
    let o = this_coll(interp, this, set, "delete")?;
    let k = coll_key(interp, &arg(args, 0))?;
    Ok(Value::Bool(coll_mut(interp, o)?.delete(&k)))
}

fn clear(interp: &mut Interp, this: &Value, set: bool) -> JsResult<Value> {
    let o = this_coll(interp, this, set, "clear")?;
    let slots = coll(interp, o)?.slots();
    // Clearing visits every position, and each takes a unit of fuel.
    for _ in 0..slots {
        interp.use_fuel()?;
    }
    coll_mut(interp, o)?.clear();
    Ok(Value::Undefined)
}

fn size(interp: &mut Interp, this: &Value, set: bool) -> JsResult<Value> {
    let o = this_coll(interp, this, set, "size")?;
    #[allow(clippy::cast_precision_loss)]
    Ok(Value::Number(coll(interp, o)?.len() as f64))
}

fn map_has(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    has(interp, &this, args, false)
}

fn map_delete(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    delete(interp, &this, args, false)
}

fn map_clear(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    clear(interp, &this, false)
}

fn map_size(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    size(interp, &this, false)
}

fn set_add(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = this_coll(interp, &this, true, "add")?;
    coll_put(interp, o, arg(args, 0), Value::Undefined)?;
    Ok(this)
}

fn set_has(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    has(interp, &this, args, true)
}

fn set_delete(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    delete(interp, &this, args, true)
}

fn set_clear(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    clear(interp, &this, true)
}

fn set_size(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    size(interp, &this, true)
}

/// `forEach` (sections 23.1.3.5 and 23.2.3.6): calls `f` with each entry's value, its key (for a
/// Set, the value again) and the collection, in order, visiting entries added while it runs. The
/// position is kept in an iterator object, so a collection that removes holes moves it.
fn for_each(interp: &mut Interp, this: &Value, args: &[Value], set: bool) -> JsResult<Value> {
    let o = this_coll(interp, this, set, "forEach")?;
    let f = arg(args, 0);
    require_callable(interp, &f, "the callback")?;
    let this_arg = arg(args, 1);
    let source = if set { IterSource::Set } else { IterSource::Map };
    let it = new_iter(interp, o, source, IterKind::Entries, None)?;
    interp.rooted(&[Value::Object(it), f.clone(), this_arg.clone()], |interp| {
        while let Some((k, v)) = advance(interp, it)? {
            let v = if set { k.clone() } else { v };
            interp.call(&f, &this_arg, &[v, k, Value::Object(o)])?;
        }
        Ok(Value::Undefined)
    })
}

fn map_for_each(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    for_each(interp, &this, args, false)
}

fn set_for_each(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    for_each(interp, &this, args, true)
}

// ---- iterators ----

/// A new iterator over `target`, with `proto` (by default the prototype for `source`).
pub(crate) fn new_iter(interp: &mut Interp, target: ObjId, source: IterSource, kind: IterKind, proto: Option<ObjId>) -> JsResult<ObjId> {
    interp.rooted(&[Value::Object(target)], |i| i.reserve_heap(OBJECT_BYTES + VIEW_BYTES))?;
    let proto = proto.unwrap_or(match source {
        IterSource::Map => interp.intrinsics.es2015.map_iter_proto,
        IterSource::Set => interp.intrinsics.es2015.set_iter_proto,
        IterSource::Array => interp.intrinsics.es2015.array_iter_proto,
    });
    let state = IterState { source, kind, target: Some(target), pos: 0 };
    let it = interp.heap.alloc_object(Object::new(Class::Iter(Box::new(state)), Some(proto)));
    interp.heap.bytes = interp.heap.bytes.saturating_add(VIEW_BYTES);
    Ok(it)
}

/// The next entry of a Map or Set iterator, or `None` when it has finished. Each position
/// passed takes a unit of fuel.
fn advance(interp: &mut Interp, it: ObjId) -> JsResult<Option<(Value, Value)>> {
    loop {
        interp.use_fuel()?;
        let Class::Iter(state) = &interp.object(it)?.class else { return Err(Abrupt::Internal("not an iterator".into())) };
        let (Some(target), pos) = (state.target, state.pos) else { return Ok(None) };
        let c = coll(interp, target)?;
        let found = (pos < c.slots()).then(|| c.entry(pos).cloned());
        let Class::Iter(state) = &mut interp.object_mut(it)?.class else { return Err(Abrupt::Internal("not an iterator".into())) };
        match found {
            None => {
                state.target = None;
                return Ok(None);
            }
            Some(entry) => {
                state.pos = pos + 1;
                if let Some(entry) = entry {
                    return Ok(Some(entry));
                }
            }
        }
    }
}

/// An iterator result object `{ value, done }` (section 7.4.7).
pub(crate) fn iter_result(interp: &mut Interp, value: Value, done: bool) -> JsResult<Value> {
    interp.rooted(std::slice::from_ref(&value), |i| i.reserve_heap(OBJECT_BYTES + 2 * (PROP_BYTES + string_bytes(5))))?;
    let o = interp.new_object();
    interp.define(o, key("value"), Prop::Data { value, flags: OPEN })?;
    interp.define(o, key("done"), Prop::Data { value: Value::Bool(done), flags: OPEN })?;
    Ok(Value::Object(o))
}

/// A new array `[a, b]`.
pub(crate) fn pair(interp: &mut Interp, a: Value, b: Value) -> JsResult<Value> {
    interp.rooted(&[a.clone(), b.clone()], |interp| {
        let arr = new_array(interp, 0)?;
        interp.rooted(&[Value::Object(arr)], |interp| {
            create_at(interp, arr, 0.0, a.clone())?;
            create_at(interp, arr, 1.0, b.clone())?;
            Ok(Value::Object(arr))
        })
    })
}

/// The iterator a `next` method was called on, when it goes through `source`.
fn this_iter(interp: &mut Interp, this: &Value, source: IterSource, name: &str) -> JsResult<(ObjId, IterKind)> {
    if let Value::Object(o) = this
        && let Some(Class::Iter(state)) = interp.heap.object(*o).map(|o| &o.class)
        && state.source == source
    {
        return Ok((*o, state.kind));
    }
    Err(interp.type_error(&format!("{name} Iterator.prototype.next was called on a value that is not a {name} Iterator")))
}

fn coll_next(interp: &mut Interp, this: &Value, source: IterSource, name: &str) -> JsResult<Value> {
    let (it, kind) = this_iter(interp, this, source, name)?;
    let Some((k, v)) = advance(interp, it)? else { return iter_result(interp, Value::Undefined, true) };
    let value = match (kind, source) {
        (IterKind::Keys, _) | (IterKind::Values, IterSource::Set) => k,
        (IterKind::Values, _) => v,
        (IterKind::Entries, IterSource::Set) => pair(interp, k.clone(), k)?,
        (IterKind::Entries, _) => pair(interp, k, v)?,
    };
    iter_result(interp, value, false)
}

fn map_iterator_next(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    coll_next(interp, &this, IterSource::Map, "Map")
}

fn set_iterator_next(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    coll_next(interp, &this, IterSource::Set, "Set")
}

/// `%ArrayIteratorPrototype%.next` (section 22.1.5.2.1) for the iterators of typed arrays.
fn array_iterator_next(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (it, kind) = this_iter(interp, &this, IterSource::Array, "Array")?;
    interp.use_fuel()?;
    let Class::Iter(state) = &interp.object(it)?.class else { return Err(Abrupt::Internal("not an iterator".into())) };
    let (target, pos) = (state.target, state.pos);
    let element = match target {
        Some(t) => match &interp.object(t)?.class {
            Class::TypedArray(view) => super::binary::element(interp, view, pos as f64),
            _ => None,
        },
        None => None,
    };
    let Class::Iter(state) = &mut interp.object_mut(it)?.class else { return Err(Abrupt::Internal("not an iterator".into())) };
    let Some(element) = element else {
        state.target = None;
        return iter_result(interp, Value::Undefined, true);
    };
    state.pos = pos + 1;
    #[allow(clippy::cast_precision_loss)]
    let index = Value::Number(pos as f64);
    let value = match kind {
        IterKind::Keys => index,
        IterKind::Values => Value::Number(element),
        IterKind::Entries => pair(interp, index, Value::Number(element))?,
    };
    iter_result(interp, value, false)
}

fn iter_method(interp: &mut Interp, this: &Value, set: bool, kind: IterKind, name: &str) -> JsResult<Value> {
    let o = this_coll(interp, this, set, name)?;
    let source = if set { IterSource::Set } else { IterSource::Map };
    new_iter(interp, o, source, kind, None).map(Value::Object)
}

fn map_keys(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    iter_method(interp, &this, false, IterKind::Keys, "keys")
}

fn map_values(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    iter_method(interp, &this, false, IterKind::Values, "values")
}

fn map_entries(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    iter_method(interp, &this, false, IterKind::Entries, "entries")
}

fn set_values(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    iter_method(interp, &this, true, IterKind::Values, "values")
}

fn set_entries(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    iter_method(interp, &this, true, IterKind::Entries, "entries")
}

// ---- WeakMap and WeakSet ----

fn this_weak(interp: &mut Interp, this: &Value, set: bool, method: &str) -> JsResult<ObjId> {
    if let Value::Object(o) = this
        && let Some(obj) = interp.heap.object(*o)
        && matches!((&obj.class, set), (Class::WeakMap(_), false) | (Class::WeakSet(_), true))
    {
        return Ok(*o);
    }
    let name = if set { "WeakSet" } else { "WeakMap" };
    Err(interp.type_error(&format!("{name}.prototype.{method} was called on a value that is not a {name}")))
}

fn weak(interp: &Interp, o: ObjId) -> JsResult<&WeakColl> {
    match &interp.object(o)?.class {
        Class::WeakMap(w) | Class::WeakSet(w) => Ok(w),
        _ => Err(Abrupt::Internal("not a WeakMap or a WeakSet".into())),
    }
}

fn weak_mut(interp: &mut Interp, o: ObjId) -> JsResult<&mut WeakColl> {
    match &mut interp.object_mut(o)?.class {
        Class::WeakMap(w) | Class::WeakSet(w) => Ok(w),
        _ => Err(Abrupt::Internal("not a WeakMap or a WeakSet".into())),
    }
}

/// Adds or replaces the entry of the object `k`; a key that is not an object is a `TypeError`.
fn weak_put(interp: &mut Interp, o: ObjId, k: &Value, v: Value, set: bool) -> JsResult<()> {
    let Value::Object(key_obj) = k else {
        let what = if set { "a WeakSet value" } else { "a WeakMap key" };
        return Err(interp.type_error(&format!("{what} must be an object")));
    };
    if !weak(interp, o)?.entries.contains_key(&key_obj.0) {
        interp.rooted(&[Value::Object(o), k.clone(), v.clone()], |i| i.reserve_heap(WEAK_ENTRY_BYTES))?;
        interp.heap.bytes = interp.heap.bytes.saturating_add(WEAK_ENTRY_BYTES);
    }
    weak_mut(interp, o)?.entries.insert(key_obj.0, v);
    Ok(())
}

fn weak_map_get(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = this_weak(interp, &this, false, "get")?;
    let Value::Object(k) = arg(args, 0) else { return Ok(Value::Undefined) };
    Ok(weak(interp, o)?.entries.get(&k.0).cloned().unwrap_or(Value::Undefined))
}

fn weak_map_set(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = this_weak(interp, &this, false, "set")?;
    weak_put(interp, o, &arg(args, 0), arg(args, 1), false)?;
    Ok(this)
}

fn weak_has(interp: &mut Interp, this: &Value, args: &[Value], set: bool) -> JsResult<Value> {
    let o = this_weak(interp, this, set, "has")?;
    let Value::Object(k) = arg(args, 0) else { return Ok(Value::Bool(false)) };
    Ok(Value::Bool(weak(interp, o)?.entries.contains_key(&k.0)))
}

fn weak_delete(interp: &mut Interp, this: &Value, args: &[Value], set: bool) -> JsResult<Value> {
    let o = this_weak(interp, this, set, "delete")?;
    let Value::Object(k) = arg(args, 0) else { return Ok(Value::Bool(false)) };
    Ok(Value::Bool(weak_mut(interp, o)?.entries.remove(&k.0).is_some()))
}

fn weak_map_has(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    weak_has(interp, &this, args, false)
}

fn weak_map_delete(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    weak_delete(interp, &this, args, false)
}

fn weak_set_add(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = this_weak(interp, &this, true, "add")?;
    weak_put(interp, o, &arg(args, 0), Value::Undefined, true)?;
    Ok(this)
}

fn weak_set_has(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    weak_has(interp, &this, args, true)
}

fn weak_set_delete(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    weak_delete(interp, &this, args, true)
}
