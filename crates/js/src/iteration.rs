//! The iterator protocol as `for-of`, array destructuring and spread use it (ECMA-262 11th
//! edition, <https://262.ecma-international.org/11.0/>): GetIterator (section 7.4.1),
//! IteratorStep (section 7.4.5) and IteratorClose (section 7.4.6), and the instructions that run
//! them.
//!
//! An Iterator Record lives on the operand stack as an object of its own class, which no script
//! can reach. Its `done` flag is set before each call that can throw, so a handler that closes
//! the iterator on an exception does not call `return` for an iterator that threw itself, has
//! finished, or is being closed already. Each step takes a unit of fuel.
//!
//! A realm made without [`crate::Options::es2015`] has no `Symbol.iterator`. When such a realm
//! later reads ECMAScript 2015 code ([`crate::Realm::set_es2015`]), arrays, `arguments` objects
//! and strings are walked as their iterators would walk them, and other values are not iterable.

use crate::heap::{Class, OPEN, ObjId, Object, PROP_BYTES, Prop, key, string_bytes};
use crate::interp::{Abrupt, ErrorType, Interp, JsResult, internal};
use crate::value::{Str, Value, index_key, number_to_str};

/// An Iterator Record (section 7.4.1): the iterator, its `next` method, and whether it is done.
#[derive(Clone)]
pub(crate) struct IterRecord {
    pub(crate) iterator: Value,
    pub(crate) next: Value,
    pub(crate) done: bool,
    /// In a realm without the ECMAScript 2015 built-ins (one made without
    /// [`crate::Options::es2015`] that later reads ECMAScript 2015 code), the walk over an array,
    /// an `arguments` object or a string that stands in for their iterators.
    pub(crate) walk: Option<Walk>,
}

/// A walk that steps as `%ArrayIteratorPrototype%.next` and `%StringIteratorPrototype%.next` do.
#[derive(Clone)]
pub(crate) enum Walk {
    /// The object, read with its `length` at each step, and the next index.
    Array(ObjId, f64),
    /// The string, flat, and the next position in code units.
    String(Str, usize),
}

impl Interp {
    fn record(&mut self, rec: ObjId) -> JsResult<&mut IterRecord> {
        match &mut self.object_mut(rec)?.class {
            Class::IterRecord(r) => Ok(r),
            _ => internal("not an iterator record"),
        }
    }

    fn record_at(&self, n: usize) -> JsResult<ObjId> {
        match self.peek(n)? {
            Value::Object(o) => Ok(o),
            _ => internal("iterator record is not an object"),
        }
    }

    /// GetIterator (section 7.4.1): the record of the iterator `v`'s `@@iterator` method gives.
    pub(crate) fn get_iterator(&mut self, v: &Value) -> JsResult<ObjId> {
        let method = match self.well_known(|w| &w.iterator) {
            Some(sym) if !v.is_nullish() => self.get_value_sym(v, &sym)?,
            Some(_) => Value::Undefined,
            None => return self.walk_record(v),
        };
        if !self.is_callable(&method) {
            let what = match v {
                Value::Undefined => "undefined",
                Value::Null => "null",
                _ => "the value",
            };
            return Err(self.type_error(&format!("{what} is not iterable")));
        }
        let iterator = self.call(&method, v, &[])?;
        if !matches!(iterator, Value::Object(_)) {
            return Err(self.type_error("the iterator is not an object"));
        }
        let next = self.rooted(std::slice::from_ref(&iterator), |i| i.get_value(&iterator, &key("next")))?;
        let record = IterRecord { iterator, next, done: false, walk: None };
        Ok(self.heap.alloc_object(Object::new(Class::IterRecord(Box::new(record)), None)))
    }

    /// The record of a [`Walk`] over an array, an `arguments` object, a string or a `String`
    /// object, for a realm without `Symbol.iterator`; any other value is not iterable there. A
    /// `String` object is converted to a string first, as the string iterator does. A
    /// concatenated string's flattened copy is counted against the heap before it is made.
    fn walk_record(&mut self, v: &Value) -> JsResult<ObjId> {
        let walk = match v {
            Value::String(s) => Walk::String(Str::from_rc(crate::string_builtins::flat_charged(self, s)?), 0),
            Value::Object(o) if matches!(self.object(*o)?.class, Class::Array { .. } | Class::Arguments(_)) => Walk::Array(*o, 0.0),
            Value::Object(o) if matches!(self.object(*o)?.class, Class::String(_)) => {
                let s = self.to_str(v)?;
                Walk::String(Str::from_rc(crate::string_builtins::flat_charged(self, &s)?), 0)
            }
            _ => return Err(self.type_error("the value is not iterable")),
        };
        let record = IterRecord { iterator: Value::Undefined, next: Value::Undefined, done: false, walk: Some(walk) };
        Ok(self.heap.alloc_object(Object::new(Class::IterRecord(Box::new(record)), None)))
    }

    /// One step of the [`Walk`] of `rec`, which is not done: the next value, or `None` at the end.
    fn walk_step(&mut self, rec: ObjId, walk: Walk) -> JsResult<Option<Value>> {
        self.use_fuel()?;
        let (value, walk) = match walk {
            Walk::Array(a, i) => {
                let length = self.get_from(a, &key("length"), &Value::Object(a))?;
                let n = self.to_number(&length)?;
                if n.is_nan() || i >= n.trunc() {
                    return Ok(None);
                }
                let v = self.get_from(a, &number_to_str(i).flat(), &Value::Object(a))?;
                (v, Walk::Array(a, i + 1.0))
            }
            Walk::String(s, pos) => {
                let units = s.flat();
                let Some(&first) = units.get(pos) else { return Ok(None) };
                let pair = (0xD800..0xDC00).contains(&first) && units.get(pos + 1).is_some_and(|u| (0xDC00..0xE000).contains(u));
                let width = if pair { 2 } else { 1 };
                let piece = units.get(pos..pos + width).map(<[u16]>::to_vec).unwrap_or_default();
                (Value::String(Str::from_units(piece)), Walk::String(s, pos + width))
            }
        };
        let r = self.record(rec)?;
        r.walk = Some(walk);
        r.done = false;
        Ok(Some(value))
    }

    /// IteratorStep and IteratorValue (sections 7.4.5 and 7.4.4): the next value, or `None` when
    /// the iterator is done. The record stays done when the step throws.
    pub(crate) fn iter_next(&mut self, rec: ObjId) -> JsResult<Option<Value>> {
        self.iter_step(rec, true)
    }

    /// IteratorStep (section 7.4.5), and IteratorValue when `read_value` is set: the next value
    /// (`undefined` when it is not read), or `None` when the iterator is done.
    fn iter_step(&mut self, rec: ObjId, read_value: bool) -> JsResult<Option<Value>> {
        let r = self.record(rec)?;
        if r.done {
            return Ok(None);
        }
        r.done = true;
        if let Some(walk) = r.walk.clone() {
            return self.walk_step(rec, walk);
        }
        let (iterator, next) = (r.iterator.clone(), r.next.clone());
        self.use_fuel()?;
        let result = self.call(&next, &iterator, &[])?;
        if !matches!(result, Value::Object(_)) {
            return Err(self.type_error("the iterator result is not an object"));
        }
        let value = self.rooted(std::slice::from_ref(&result), |i| {
            if i.get_value(&result, &key("done"))?.truthy() {
                return Ok(None);
            }
            if !read_value {
                return Ok(Some(Value::Undefined));
            }
            i.get_value(&result, &key("value")).map(Some)
        })?;
        if value.is_some() {
            self.record(rec)?.done = false;
        }
        Ok(value)
    }

    /// IteratorClose (section 7.4.6) with a normal completion: calls the iterator's `return`
    /// method, unless it is done, and checks that it gives an object.
    pub(crate) fn iter_close(&mut self, rec: ObjId) -> JsResult<()> {
        let r = self.record(rec)?;
        if r.done {
            return Ok(());
        }
        r.done = true;
        if r.walk.is_some() {
            return Ok(());
        }
        let iterator = r.iterator.clone();
        self.rooted(std::slice::from_ref(&iterator), |i| {
            let method = i.get_value(&iterator, &key("return"))?;
            if method.is_nullish() {
                return Ok(());
            }
            if !i.is_callable(&method) {
                return Err(i.type_error("the iterator's return is not a function"));
            }
            match i.call(&method, &iterator, &[])? {
                Value::Object(_) => Ok(()),
                _ => Err(i.type_error("the iterator's return gave something that is not an object")),
            }
        })
    }

    /// `value` → `record`.
    pub(crate) fn op_get_iterator(&mut self) -> JsResult<()> {
        let v = self.peek(0)?;
        let rec = self.get_iterator(&v)?;
        self.replace(1, Value::Object(rec))
    }

    /// `record` → `record value`, or jumps with `record` when the iterator is done.
    pub(crate) fn op_iter_step(&mut self, target: u32) -> JsResult<()> {
        let rec = self.record_at(0)?;
        match self.iter_next(rec)? {
            Some(v) => self.push(v),
            None => self.jump_to(target),
        }
    }

    /// `record x₁…xₙ` → `record x₁…xₙ value`: the next value, or `undefined` when the iterator
    /// is done.
    pub(crate) fn op_iter_value(&mut self, under: u32) -> JsResult<()> {
        let rec = self.record_at(under as usize)?;
        let v = self.iter_next(rec)?.unwrap_or(Value::Undefined);
        self.push(v)
    }

    /// `record` → `record`: one step of the iterator, for a hole in an array pattern
    /// (section 13.3.3.6, Elision), which does not read the result's `value`.
    pub(crate) fn op_iter_skip(&mut self) -> JsResult<()> {
        let rec = self.record_at(0)?;
        self.iter_step(rec, false).map(drop)
    }

    /// `record x₁…xₙ` → `record x₁…xₙ array`: an array of the values left.
    pub(crate) fn op_iter_rest(&mut self, under: u32) -> JsResult<()> {
        let rec = self.record_at(under as usize)?;
        let proto = self.intrinsics.array_proto;
        let a = self.heap.alloc_object(Object::new(Class::Array { length: 0, length_writable: true }, Some(proto)));
        self.push(Value::Object(a))?;
        self.append_values(rec, a)
    }

    /// Appends the values the iterator of `rec` gives to the array `a`; both are on the stack.
    fn append_values(&mut self, rec: ObjId, a: ObjId) -> JsResult<()> {
        while let Some(v) = self.iter_next(rec)? {
            let Class::Array { length, .. } = &self.object(a)?.class else { return internal("not an array") };
            let i = *length;
            if i == u32::MAX - 1 {
                return Err(self.throw(ErrorType::Range, "array too long"));
            }
            let k = index_key(i);
            self.rooted(std::slice::from_ref(&v), |interp| interp.reserve_heap(PROP_BYTES + string_bytes(k.len())))?;
            let Class::Array { length, .. } = &mut self.object_mut(a)?.class else { return internal("not an array") };
            *length = i + 1;
            self.define(a, k, Prop::Data { value: v, flags: OPEN })?;
        }
        Ok(())
    }

    /// `array value` → `array`, with the values the value's iterator gives appended (spread,
    /// ECMA-262 11th edition section 12.2.5.2).
    pub(crate) fn op_array_spread(&mut self) -> JsResult<()> {
        let v = self.peek(0)?;
        let Value::Object(a) = self.peek(1)? else { return internal("spread into something that is not an array") };
        let rec = self.get_iterator(&v)?;
        self.replace(1, Value::Object(rec))?;
        self.append_values(rec, a)?;
        self.pop().map(drop)
    }

    /// `record` → nothing: closes the iterator unless it is done.
    pub(crate) fn op_iter_close(&mut self) -> JsResult<()> {
        let rec = self.record_at(0)?;
        self.iter_close(rec)?;
        self.pop().map(drop)
    }

    /// `record exception` → throws the exception, after closing the iterator unless it is done.
    /// What closing it throws is dropped, the lookup of `return` included (ECMA-262 2023 section
    /// 7.4.8, steps 4 to 6); running out of fuel, heap or time is not. The exception stays in its
    /// stack slot while the iterator is closed, so [`Interp::rethrow`] finds the source position
    /// and host mark kept for that slot.
    pub(crate) fn op_iter_close_throw(&mut self) -> JsResult<()> {
        let rec = self.record_at(1)?;
        match self.iter_close(rec) {
            Ok(()) | Err(Abrupt::Throw(_)) => {
                let exception = self.pop()?;
                Err(self.rethrow(exception))
            }
            Err(other) => Err(other),
        }
    }

    /// `value` → `value`: a `TypeError` when it is `undefined` or `null` (RequireObjectCoercible,
    /// section 7.2.1), before an object pattern reads it.
    pub(crate) fn op_require_object_coercible(&mut self) -> JsResult<()> {
        let v = self.peek(0)?;
        if v.is_nullish() {
            let what = if matches!(v, Value::Null) { "null" } else { "undefined" };
            return Err(self.type_error(&format!("cannot destructure {what}")));
        }
        Ok(())
    }

    /// `object key x₁…xₙ` → `object key x₁…xₙ object[key]`.
    pub(crate) fn op_pattern_get(&mut self, under: u32) -> JsResult<()> {
        let k = self.peek(under as usize)?;
        let object = self.peek(under as usize + 1)?;
        let v = self.with_prop_key(&k, |i, pk| i.get_value_pk(&object, &pk))?;
        self.push(v)
    }
}
