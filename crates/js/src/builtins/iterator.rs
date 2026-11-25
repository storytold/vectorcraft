//! The iterators of arrays, strings and `arguments` objects (ECMA-262 11th edition,
//! <https://262.ecma-international.org/11.0/>): `%IteratorPrototype%` (section 25.1.2),
//! `%ArrayIteratorPrototype%` with `Array.prototype.values`, `keys`, `entries` and `@@iterator`
//! (sections 22.1.3 and 22.1.5), `%StringIteratorPrototype%` with `String.prototype[@@iterator]`
//! (sections 21.1.3.29 and 21.1.5), which steps by code point, and the `@@iterator` of every
//! `arguments` object (section 9.4.4). Installed in a realm with [`crate::Options::es2015`] only.
//!
//! An iterator's `next` takes a unit of fuel for each step. An Array Iterator reads its object's
//! `length` at every step, so elements added while it runs are visited, and it lets go of the
//! object once it is done.

use super::{create_at, data, native, new_array, new_string};
use crate::heap::{CONFIGURABLE, Class, HIDDEN, OBJECT_BYTES, OPEN, ObjId, Object, PROP_BYTES, Prop, key, string_bytes};
use crate::interp::{Interp, JsResult};
use crate::value::{Str, Value, number_to_str};

/// What an Array Iterator gives at each step (section 22.1.5.1).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArrayKind {
    Keys,
    Values,
    Entries,
}

/// An iterator's state. The object or string is `None` once the iterator is done.
#[derive(Clone)]
pub(crate) enum IterState {
    /// [[IteratedObject]], [[ArrayIteratorNextIndex]] and [[ArrayIterationKind]].
    Array { target: Option<ObjId>, next: f64, kind: ArrayKind },
    /// [[IteratedString]], flat, and [[StringIteratorNextIndex]] in code units.
    String { text: Option<Str>, pos: usize },
}

pub(super) fn install(interp: &mut Interp) -> JsResult<()> {
    let Some(wk) = interp.symbols.well_known.as_ref() else { return Ok(()) };
    let (iterator, to_string_tag) = (wk.iterator.clone(), wk.to_string_tag.clone());
    let Some(mut objects) = interp.symbols.objects else { return Ok(()) };

    let object_proto = interp.intrinsics.object_proto;
    let iterator_proto = interp.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));
    super::symbol::sym_method(interp, iterator_proto, iterator.clone(), "[Symbol.iterator]", return_this, 0, HIDDEN)?;

    let array_iterator_proto = interp.heap.alloc_object(Object::new(Class::Ordinary, Some(iterator_proto)));
    super::method(interp, array_iterator_proto, "next", array_iterator_next, 0)?;
    interp.define_sym(array_iterator_proto, to_string_tag.clone(), Prop::Data { value: Value::str("Array Iterator"), flags: CONFIGURABLE })?;

    let string_iterator_proto = interp.heap.alloc_object(Object::new(Class::Ordinary, Some(iterator_proto)));
    super::method(interp, string_iterator_proto, "next", string_iterator_next, 0)?;
    interp.define_sym(string_iterator_proto, to_string_tag, Prop::Data { value: Value::str("String Iterator"), flags: CONFIGURABLE })?;

    let array_proto = interp.intrinsics.array_proto;
    let values = native(interp, "values", array_values, 0, crate::heap::NativeKind::Plain, false)?;
    data(interp, array_proto, "values", Value::Object(values), HIDDEN)?;
    interp.define_sym(array_proto, iterator.clone(), Prop::Data { value: Value::Object(values), flags: HIDDEN })?;
    super::method(interp, array_proto, "keys", array_keys, 0)?;
    super::method(interp, array_proto, "entries", array_entries, 0)?;

    let string_proto = interp.intrinsics.string_proto;
    super::symbol::sym_method(interp, string_proto, iterator, "[Symbol.iterator]", string_iterator, 0, HIDDEN)?;

    objects.array_iterator_proto = array_iterator_proto;
    objects.string_iterator_proto = string_iterator_proto;
    objects.array_values = values;
    interp.symbols.objects = Some(objects);
    Ok(())
}

/// Gives a new `arguments` object its `@@iterator`, `%ArrayProto_values%` (section 9.4.4.6,
/// step 20, and section 9.4.4.7, step 7), in a realm that has it.
pub(crate) fn define_arguments_iterator(interp: &mut Interp, a: ObjId) -> JsResult<()> {
    let (Some(objects), Some(sym)) = (interp.symbols.objects, interp.well_known(|w| &w.iterator)) else { return Ok(()) };
    interp.define_sym(a, sym, Prop::Data { value: Value::Object(objects.array_values), flags: HIDDEN })
}

/// `%IteratorPrototype%[@@iterator]()` (section 25.1.2.1): the iterator itself.
fn return_this(_: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    Ok(this)
}

/// The bytes of an iterator result: the object and its `value` and `done` properties with their
/// keys, as [`Interp::define`] counts them.
fn iter_result_bytes() -> usize {
    OBJECT_BYTES + 2 * PROP_BYTES + string_bytes("value".len()) + string_bytes("done".len())
}

/// CreateIterResultObject (section 7.4.7): `{ value, done }`.
pub(crate) fn iter_result(interp: &mut Interp, value: Value, done: bool) -> JsResult<Value> {
    interp.rooted(std::slice::from_ref(&value), |i| i.reserve_heap(iter_result_bytes()))?;
    let proto = interp.intrinsics.object_proto;
    let o = interp.heap.alloc_object(Object::new(Class::Ordinary, Some(proto)));
    interp.define(o, key("value"), Prop::Data { value, flags: OPEN })?;
    interp.define(o, key("done"), Prop::Data { value: Value::Bool(done), flags: OPEN })?;
    Ok(Value::Object(o))
}

/// A new iterator object with `state`, its target kept reachable while it is made.
fn new_iterator(interp: &mut Interp, proto: ObjId, state: IterState, root: Value) -> JsResult<Value> {
    interp.rooted(&[root], |i| i.reserve_heap(OBJECT_BYTES))?;
    Ok(Value::Object(interp.heap.alloc_object(Object::new(Class::Iterator(Box::new(state)), Some(proto)))))
}

/// CreateArrayIterator (section 22.1.5.1) for `ToObject(this)`.
fn create_array_iterator(interp: &mut Interp, this: &Value, kind: ArrayKind) -> JsResult<Value> {
    let o = interp.to_object(this)?;
    let Some(proto) = interp.symbols.objects.map(|o| o.array_iterator_proto) else { return Ok(Value::Undefined) };
    new_iterator(interp, proto, IterState::Array { target: Some(o), next: 0.0, kind }, Value::Object(o))
}

/// `Array.prototype.values()` (section 22.1.3.32), which is also `Array.prototype[@@iterator]`.
fn array_values(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    create_array_iterator(interp, &this, ArrayKind::Values)
}

/// `Array.prototype.keys()` (section 22.1.3.16).
fn array_keys(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    create_array_iterator(interp, &this, ArrayKind::Keys)
}

/// `Array.prototype.entries()` (section 22.1.3.4).
fn array_entries(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    create_array_iterator(interp, &this, ArrayKind::Entries)
}

/// `String.prototype[@@iterator]()` (section 21.1.3.29).
fn string_iterator(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    if this.is_nullish() {
        return Err(interp.type_error("String.prototype[Symbol.iterator] was called on null or undefined"));
    }
    let s = interp.to_str(&this)?;
    let flat = Str::from_rc(crate::string_builtins::flat_charged(interp, &s)?);
    let Some(proto) = interp.symbols.objects.map(|o| o.string_iterator_proto) else { return Ok(Value::Undefined) };
    let root = Value::String(flat.clone());
    new_iterator(interp, proto, IterState::String { text: Some(flat), pos: 0 }, root)
}

/// The state of the iterator `this`, or a `TypeError` naming `what` when `this` is not one.
fn state_of<'a>(interp: &'a mut Interp, this: &Value, string: bool, what: &str) -> JsResult<&'a mut IterState> {
    let ok = match this {
        Value::Object(o) => match interp.heap.object(*o).map(|x| &x.class) {
            Some(Class::Iterator(s)) => matches!(**s, IterState::String { .. }) == string,
            _ => false,
        },
        _ => false,
    };
    if !ok {
        return Err(interp.type_error(&format!("{what}.next was called on a value that is not an {what}")));
    }
    match this.as_object().and_then(|o| interp.heap.object_mut(o)).map(|x| &mut x.class) {
        Some(Class::Iterator(s)) => Ok(s),
        _ => Err(crate::interp::Abrupt::Internal("iterator state went missing".into())),
    }
}

/// ToLength (section 7.1.20).
fn to_length(interp: &mut Interp, v: &Value) -> JsResult<f64> {
    let n = interp.to_number(v)?;
    if n.is_nan() || n <= 0.0 {
        return Ok(0.0);
    }
    Ok(n.trunc().min(9_007_199_254_740_991.0))
}

/// `%ArrayIteratorPrototype%.next()` (section 22.1.5.2.1).
fn array_iterator_next(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let IterState::Array { target, next, kind } = *state_of(interp, &this, false, "Array Iterator")? else {
        return Err(crate::interp::Abrupt::Internal("not an Array Iterator".into()));
    };
    interp.use_fuel()?;
    let Some(a) = target else { return iter_result(interp, Value::Undefined, true) };
    // Reading `length` and the element can run script that collects, and that script can finish
    // the iterator, which then no longer holds the object.
    let roots = [this.clone(), Value::Object(a)];
    interp.rooted(&roots, |interp| array_step(interp, &this, a, next, kind))
}

/// One step of an Array Iterator over `a` at index `next`, with `this` and `a` kept reachable.
fn array_step(interp: &mut Interp, this: &Value, a: ObjId, next: f64, kind: ArrayKind) -> JsResult<Value> {
    let len = interp.get_from(a, &key("length"), &Value::Object(a))?;
    let len = to_length(interp, &len)?;
    if next >= len {
        if let IterState::Array { target, .. } = state_of(interp, this, false, "Array Iterator")? {
            *target = None;
        }
        return iter_result(interp, Value::Undefined, true);
    }
    if let IterState::Array { next: n, .. } = state_of(interp, this, false, "Array Iterator")? {
        *n = next + 1.0;
    }
    let index = Value::Number(next);
    let value = match kind {
        ArrayKind::Keys => index,
        ArrayKind::Values | ArrayKind::Entries => {
            let k = number_to_str(next).flat();
            let v = interp.get_from(a, &k, &Value::Object(a))?;
            if kind == ArrayKind::Values {
                v
            } else {
                let pair = interp.rooted(std::slice::from_ref(&v), |i| new_array(i, 0))?;
                interp.rooted(&[Value::Object(pair), v.clone()], |i| {
                    create_at(i, pair, 0.0, index)?;
                    create_at(i, pair, 1.0, v.clone())
                })?;
                Value::Object(pair)
            }
        }
    };
    iter_result(interp, value, false)
}

/// `%StringIteratorPrototype%.next()` (section 21.1.5.2.1): the next code point, a surrogate
/// pair kept together and a lone surrogate given alone.
fn string_iterator_next(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    interp.use_fuel()?;
    let state = state_of(interp, &this, true, "String Iterator")?;
    let IterState::String { text, pos } = state else { return Err(crate::interp::Abrupt::Internal("not a String Iterator".into())) };
    let Some(s) = text.as_ref() else { return iter_result(interp, Value::Undefined, true) };
    let units = s.flat();
    let Some(&first) = units.get(*pos) else {
        *text = None;
        return iter_result(interp, Value::Undefined, true);
    };
    let pair = (0xD800..0xDC00).contains(&first) && units.get(*pos + 1).is_some_and(|u| (0xDC00..0xE000).contains(u));
    let width = if pair { 2 } else { 1 };
    let piece = units.get(*pos..*pos + width).map(<[u16]>::to_vec).unwrap_or_default();
    *pos += width;
    interp.rooted(std::slice::from_ref(&this), |interp| {
        let v = new_string(interp, piece)?;
        iter_result(interp, v, false)
    })
}
