//! `Array` (ECMA-262 5.1 section 15.4): the constructor, `Array.isArray` and every method of
//! `Array.prototype`. The methods are generic: they read `length` and the elements of any object.
//!
//! A method that walks a range of indices visits only the indices that can hold an element when
//! the range is much longer than the object has properties (a sparse array such as
//! `new Array(4294967295)`), so a long empty range costs nothing and no index is allocated for
//! it. The list of indices is made again whenever a script adds an index property to any object
//! while the method runs. Every index visited takes a unit of fuel.

use super::{
    Builder, ELEMENT_BYTES, arg, constructor, create_at, delete_at, get_at, has_at, length_of, method, native, new_array, object_to_string, put_at,
    require_callable, to_integer,
};
use crate::heap::{Class, NativeKind, ObjId};
use crate::interp::{Abrupt, Interp, JsResult, strict_eq};
use crate::value::{Str, Value, to_uint32};

pub(super) fn install(interp: &mut Interp) -> JsResult<()> {
    let proto = interp.intrinsics.array_proto;
    let array = native(interp, "Array", array_ctor, 1, NativeKind::Plain, true)?;
    constructor(interp, "Array", array, proto)?;
    method(interp, array, "isArray", is_array, 1)?;
    let methods: [(&'static str, crate::heap::NativeFn, u32); 21] = [
        ("toString", to_string, 0),
        ("toLocaleString", to_locale_string, 0),
        ("concat", concat, 1),
        ("join", join, 1),
        ("pop", pop, 0),
        ("push", push, 1),
        ("reverse", reverse, 0),
        ("shift", shift, 0),
        ("slice", slice, 2),
        ("sort", sort, 1),
        ("splice", splice, 2),
        ("unshift", unshift, 1),
        ("indexOf", index_of, 1),
        ("lastIndexOf", last_index_of, 1),
        ("every", every, 1),
        ("some", some, 1),
        ("forEach", for_each, 1),
        ("map", map, 1),
        ("filter", filter, 1),
        ("reduce", reduce, 1),
        ("reduceRight", reduce_right, 1),
    ];
    for (name, f, length) in methods {
        method(interp, proto, name, f, length)?;
    }
    Ok(())
}

// ---- visiting indices ----

/// Ranges at most this long are visited index by index.
const DENSE_SPAN: u64 = 4096;

/// How an index an object holds names an index to visit.
#[derive(Clone, Copy)]
enum Map {
    /// The index `p` asks for a visit of `p - shift`.
    Shift(i64),
    /// The index `p` asks for a visit of `p` and of `mirror - p`.
    Mirror(i64),
}

/// The indices of a range, in order, that an operation must visit.
struct Indices {
    obj: ObjId,
    /// The range left to visit, `lo` included and `hi` not.
    lo: u64,
    hi: u64,
    ascending: bool,
    maps: Vec<Map>,
    /// The indices left when the range is visited sparsely.
    sparse: Option<Sparse>,
}

struct Sparse {
    list: Vec<u64>,
    pos: usize,
    /// [`crate::heap::Heap::index_epoch`] when the list was made.
    epoch: u64,
}

/// The canonical numeric property name `k` stands for, up to 15 digits.
fn numeric_index(k: &[u16]) -> Option<u64> {
    if k.is_empty() || k.len() > 15 || (k.len() > 1 && k.first() == Some(&u16::from(b'0'))) {
        return None;
    }
    let mut n = 0u64;
    for &u in k {
        let d = u.checked_sub(u16::from(b'0')).filter(|d| *d <= 9)?;
        n = n * 10 + u64::from(d);
    }
    Some(n)
}

/// The objects of the prototype chain from `obj`, with how many properties (and String object
/// indices) each holds.
fn chain(interp: &Interp, obj: ObjId) -> Vec<(ObjId, usize)> {
    let mut out = Vec::new();
    let mut cur = Some(obj);
    while let Some(id) = cur {
        let Some(o) = interp.heap.object(id) else { break };
        let string_len = if let Class::String(s) = &o.class { s.len() } else { 0 };
        out.push((id, o.props.len().saturating_add(string_len)));
        cur = o.proto;
    }
    out
}

impl Indices {
    fn new(interp: &mut Interp, obj: ObjId, lo: u64, hi: u64, ascending: bool, maps: Vec<Map>) -> JsResult<Self> {
        let mut it = Indices { obj, lo, hi, ascending, maps, sparse: None };
        if hi.saturating_sub(lo) > DENSE_SPAN {
            let present: usize = chain(interp, obj).iter().map(|(_, n)| *n).fold(0, usize::saturating_add);
            if hi - lo > (present as u64).saturating_mul(8) {
                it.sparse = Some(it.gather(interp)?);
            }
        }
        Ok(it)
    }

    fn up(interp: &mut Interp, obj: ObjId, lo: u64, hi: u64) -> JsResult<Self> {
        Self::new(interp, obj, lo, hi, true, vec![Map::Shift(0)])
    }

    fn down(interp: &mut Interp, obj: ObjId, lo: u64, hi: u64) -> JsResult<Self> {
        Self::new(interp, obj, lo, hi, false, vec![Map::Shift(0)])
    }

    /// Lists the indices left to visit from the properties of the object and its prototypes.
    /// Each property takes a unit of fuel.
    fn gather(&self, interp: &mut Interp) -> JsResult<Sparse> {
        let epoch = interp.heap.index_epoch;
        let mut list: Vec<u64> = Vec::new();
        for (id, n) in chain(interp, self.obj) {
            for _ in 0..n {
                interp.use_fuel()?;
            }
            interp.reserve_heap(n.saturating_mul(8).saturating_mul(self.maps.len() + 1))?;
            let Some(o) = interp.heap.object(id) else { continue };
            let string_len = if let Class::String(s) = &o.class { s.len() } else { 0 };
            let present = (0..string_len as u64).chain(o.props.iter().filter_map(|(k, _)| numeric_index(k)));
            for p in present {
                for map in &self.maps {
                    let p = p as i64;
                    let ks = match *map {
                        Map::Shift(s) => [Some(p - s), None],
                        Map::Mirror(m) => [Some(p), Some(m - p)],
                    };
                    for k in ks.into_iter().flatten() {
                        if k >= 0 && (k as u64) >= self.lo && (k as u64) < self.hi {
                            list.push(k as u64);
                        }
                    }
                }
            }
        }
        if self.ascending {
            list.sort_unstable();
        } else {
            list.sort_unstable_by(|a, b| b.cmp(a));
        }
        list.dedup();
        Ok(Sparse { list, pos: 0, epoch })
    }

    /// The next index to visit; it takes a unit of fuel.
    fn next(&mut self, interp: &mut Interp) -> JsResult<Option<u64>> {
        if self.lo >= self.hi {
            return Ok(None);
        }
        if self.sparse.as_ref().is_some_and(|s| s.epoch != interp.heap.index_epoch) {
            self.sparse = Some(self.gather(interp)?);
        }
        let k = match &mut self.sparse {
            None if self.ascending => self.lo,
            None => self.hi - 1,
            Some(s) => {
                let Some(&k) = s.list.get(s.pos) else {
                    self.lo = self.hi;
                    return Ok(None);
                };
                s.pos += 1;
                k
            }
        };
        if self.ascending {
            self.lo = k + 1;
        } else {
            self.hi = k;
        }
        interp.use_fuel()?;
        Ok(Some(k))
    }
}

/// The element at `k` when the object or its prototypes have one.
fn element(interp: &mut Interp, o: ObjId, k: u64) -> JsResult<Option<Value>> {
    let k = k as f64;
    if !has_at(interp, o, k)? {
        return Ok(None);
    }
    get_at(interp, o, k).map(Some)
}

/// ToObject of `this`, kept reachable while `f` runs.
fn with_this<T>(interp: &mut Interp, this: &Value, f: impl FnOnce(&mut Interp, ObjId) -> JsResult<T>) -> JsResult<T> {
    let o = interp.to_object(this)?;
    interp.rooted(&[Value::Object(o)], |interp| f(interp, o))
}

/// The start a relative index gives within `0..=len` (`slice` and `splice`).
fn relative(interp: &mut Interp, v: &Value, len: u32) -> JsResult<u64> {
    let n = to_integer(interp, v)?;
    let len = f64::from(len);
    Ok(if n < 0.0 { (len + n).max(0.0) } else { n.min(len) } as u64)
}

fn number(k: u64) -> Value {
    Value::Number(k as f64)
}

// ---- the constructor ----

/// `Array(…)` and `new Array(…)` (sections 15.4.1 and 15.4.2).
fn array_ctor(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    if let [Value::Number(n)] = args {
        let len = to_uint32(*n);
        if f64::from(len) != *n {
            return Err(interp.throw(crate::interp::ErrorType::Range, "invalid array length"));
        }
        return new_array(interp, len).map(Value::Object);
    }
    let a = new_array(interp, 0)?;
    interp.rooted(&[Value::Object(a)], |interp| {
        for (i, v) in args.iter().enumerate() {
            interp.use_fuel()?;
            create_at(interp, a, i as f64, v.clone())?;
        }
        Ok(Value::Object(a))
    })
}

/// `Array.isArray` (section 15.4.3.2).
fn is_array(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let a = arg(args, 0).as_object().and_then(|o| interp.heap.object(o)).is_some_and(|o| matches!(o.class, Class::Array { .. }));
    Ok(Value::Bool(a))
}

// ---- conversions ----

/// `Array.prototype.toString` (section 15.4.4.2).
fn to_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    with_this(interp, &this, |interp, o| {
        let join = interp.get_from(o, &crate::heap::key("join"), &Value::Object(o))?;
        if interp.is_callable(&join) {
            return interp.call(&join, &Value::Object(o), &[]);
        }
        object_to_string(interp, Value::Object(o), &[], false)
    })
}

/// Joins the elements `0..len` of `o`, each turned into a string by `item`, with `sep` between
/// them. A cycle back to an object being joined joins as an empty string.
fn join_with(interp: &mut Interp, o: ObjId, sep: &[u16], item: fn(&mut Interp, Value) -> JsResult<Option<Str>>) -> JsResult<Value> {
    if interp.join_stack.contains(&o) {
        return Ok(Value::str(""));
    }
    let len = length_of(interp, o)?;
    if len == 0 {
        return Ok(Value::str(""));
    }
    // A result that cannot fit is refused before any element is read.
    let least = (sep.len() as u64).saturating_mul(u64::from(len - 1));
    if least > (interp.limits.heap_bytes / 4) as u64 {
        return Err(Abrupt::Limit(crate::LimitKind::Heap));
    }
    interp.join_stack.push(o);
    let r = join_elements(interp, o, len, sep, item);
    interp.join_stack.pop();
    r
}

fn join_elements(interp: &mut Interp, o: ObjId, len: u32, sep: &[u16], item: fn(&mut Interp, Value) -> JsResult<Option<Str>>) -> JsResult<Value> {
    let mut out = Builder::new();
    // Separators written so far: element `k` follows `k` of them.
    let mut seps = 0u64;
    let mut it = Indices::up(interp, o, 0, u64::from(len))?;
    while let Some(k) = it.next(interp)? {
        let v = get_at(interp, o, k as f64)?;
        let Some(s) = item(interp, v)? else { continue };
        for _ in seps..k {
            out.push_units(interp, sep)?;
        }
        seps = k;
        out.push_units(interp, &s.flat())?;
    }
    for _ in seps..u64::from(len - 1) {
        out.push_units(interp, sep)?;
    }
    Ok(out.finish())
}

/// An element as `join` writes it: nothing for `undefined` and `null`.
fn join_item(interp: &mut Interp, v: Value) -> JsResult<Option<Str>> {
    if v.is_nullish() {
        return Ok(None);
    }
    interp.to_str(&v).map(Some)
}

/// An element as `toLocaleString` writes it (section 15.4.4.3).
fn locale_item(interp: &mut Interp, v: Value) -> JsResult<Option<Str>> {
    if v.is_nullish() {
        return Ok(None);
    }
    let e = interp.to_object(&v)?;
    let f = interp.get_from(e, &crate::heap::key("toLocaleString"), &Value::Object(e))?;
    require_callable(interp, &f, "an element's toLocaleString")?;
    let s = interp.call(&f, &Value::Object(e), &[])?;
    interp.to_str(&s).map(Some)
}

/// `Array.prototype.join` (section 15.4.4.5).
fn join(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    with_this(interp, &this, |interp, o| {
        let sep = match arg(args, 0) {
            Value::Undefined => Str::from(","),
            v => interp.to_str(&v)?,
        };
        join_with(interp, o, &sep.flat(), join_item)
    })
}

/// `Array.prototype.toLocaleString` (section 15.4.4.3).
fn to_locale_string(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    with_this(interp, &this, |interp, o| join_with(interp, o, &[u16::from(b',')], locale_item))
}

// ---- methods that change the object ----

/// `Array.prototype.concat` (section 15.4.4.4).
fn concat(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    with_this(interp, &this, |interp, o| {
        let a = new_array(interp, 0)?;
        interp.rooted(&[Value::Object(a)], |interp| {
            let mut n = 0u64;
            for e in std::iter::once(Value::Object(o)).chain(args.iter().cloned()) {
                interp.use_fuel()?;
                let spread = e.as_object().filter(|&id| interp.heap.object(id).is_some_and(|x| matches!(x.class, Class::Array { .. })));
                let Some(e) = spread else {
                    create_at(interp, a, n as f64, e)?;
                    n += 1;
                    continue;
                };
                let len = u64::from(length_of(interp, e)?);
                let mut it = Indices::up(interp, e, 0, len)?;
                while let Some(k) = it.next(interp)? {
                    if let Some(v) = element(interp, e, k)? {
                        create_at(interp, a, (n + k) as f64, v)?;
                    }
                }
                n += len;
            }
            set_length(interp, a, n as f64)?;
            Ok(Value::Object(a))
        })
    })
}

fn set_length(interp: &mut Interp, o: ObjId, n: f64) -> JsResult<()> {
    interp.put(o, crate::heap::key("length"), Value::Number(n), true)
}

/// `Array.prototype.pop` (section 15.4.4.6).
fn pop(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    with_this(interp, &this, |interp, o| {
        let len = length_of(interp, o)?;
        if len == 0 {
            set_length(interp, o, 0.0)?;
            return Ok(Value::Undefined);
        }
        let last = f64::from(len - 1);
        let v = get_at(interp, o, last)?;
        interp.rooted(std::slice::from_ref(&v), |interp| {
            delete_at(interp, o, last)?;
            set_length(interp, o, last)
        })?;
        Ok(v)
    })
}

/// `Array.prototype.push` (section 15.4.4.7).
fn push(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    with_this(interp, &this, |interp, o| {
        let mut n = f64::from(length_of(interp, o)?);
        for v in args {
            interp.use_fuel()?;
            put_at(interp, o, n, v.clone())?;
            n += 1.0;
        }
        set_length(interp, o, n)?;
        Ok(Value::Number(n))
    })
}

/// `Array.prototype.reverse` (section 15.4.4.8).
fn reverse(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    with_this(interp, &this, |interp, o| {
        let len = u64::from(length_of(interp, o)?);
        let middle = len / 2;
        let mirror = len as i64 - 1;
        let mut it = Indices::new(interp, o, 0, middle, true, vec![Map::Mirror(mirror)])?;
        while let Some(lower) = it.next(interp)? {
            let upper = len - lower - 1;
            let lower_value = element(interp, o, lower)?;
            let upper_value = interp.rooted(lower_value.as_slice(), |interp| element(interp, o, upper))?;
            let mut roots: Vec<Value> = lower_value.iter().cloned().collect();
            roots.extend(upper_value.iter().cloned());
            interp.rooted(&roots, |interp| match (lower_value, upper_value) {
                (Some(l), Some(u)) => {
                    put_at(interp, o, lower as f64, u)?;
                    put_at(interp, o, upper as f64, l)
                }
                (None, Some(u)) => {
                    put_at(interp, o, lower as f64, u)?;
                    delete_at(interp, o, upper as f64)
                }
                (Some(l), None) => {
                    delete_at(interp, o, lower as f64)?;
                    put_at(interp, o, upper as f64, l)
                }
                (None, None) => Ok(()),
            })?;
        }
        Ok(Value::Object(o))
    })
}

/// Moves the element at `from` to `to`, or deletes `to` when there is none at `from`.
fn move_element(interp: &mut Interp, o: ObjId, from: u64, to: u64) -> JsResult<()> {
    match element(interp, o, from)? {
        Some(v) => put_at(interp, o, to as f64, v),
        None => delete_at(interp, o, to as f64),
    }
}

/// `Array.prototype.shift` (section 15.4.4.9).
fn shift(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    with_this(interp, &this, |interp, o| {
        let len = u64::from(length_of(interp, o)?);
        if len == 0 {
            set_length(interp, o, 0.0)?;
            return Ok(Value::Undefined);
        }
        let first = get_at(interp, o, 0.0)?;
        interp.rooted(std::slice::from_ref(&first), |interp| {
            let mut it = Indices::new(interp, o, 1, len, true, vec![Map::Shift(0), Map::Shift(-1)])?;
            while let Some(k) = it.next(interp)? {
                move_element(interp, o, k, k - 1)?;
            }
            delete_at(interp, o, (len - 1) as f64)?;
            set_length(interp, o, (len - 1) as f64)
        })?;
        Ok(first)
    })
}

/// `Array.prototype.unshift` (section 15.4.4.13).
fn unshift(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    with_this(interp, &this, |interp, o| {
        let len = u64::from(length_of(interp, o)?);
        let count = args.len() as u64;
        if count > 0 {
            let maps = vec![Map::Shift(-1), Map::Shift(count as i64 - 1)];
            let mut it = Indices::new(interp, o, 1, len + 1, false, maps)?;
            while let Some(k) = it.next(interp)? {
                move_element(interp, o, k - 1, k + count - 1)?;
            }
        }
        for (j, v) in args.iter().enumerate() {
            interp.use_fuel()?;
            put_at(interp, o, j as f64, v.clone())?;
        }
        let n = (len + count) as f64;
        set_length(interp, o, n)?;
        Ok(Value::Number(n))
    })
}

/// `Array.prototype.slice` (section 15.4.4.10).
fn slice(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    with_this(interp, &this, |interp, o| {
        let len = length_of(interp, o)?;
        let start = relative(interp, &arg(args, 0), len)?;
        let end = match arg(args, 1) {
            Value::Undefined => u64::from(len),
            v => relative(interp, &v, len)?,
        };
        let a = new_array(interp, 0)?;
        interp.rooted(&[Value::Object(a)], |interp| {
            let mut it = Indices::up(interp, o, start, end)?;
            while let Some(k) = it.next(interp)? {
                if let Some(v) = element(interp, o, k)? {
                    create_at(interp, a, (k - start) as f64, v)?;
                }
            }
            set_length(interp, a, end.saturating_sub(start) as f64)?;
            Ok(Value::Object(a))
        })
    })
}

/// `Array.prototype.splice` (section 15.4.4.12).
fn splice(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    with_this(interp, &this, |interp, o| {
        let len = length_of(interp, o)?;
        let start = relative(interp, &arg(args, 0), len)?;
        let len = u64::from(len);
        let delete_count = match args.get(1) {
            // Without a count, everything from the start is deleted, as later editions and
            // every implementation do.
            None if !args.is_empty() => len - start,
            None => 0,
            Some(v) => to_integer(interp, v)?.max(0.0).min((len - start) as f64) as u64,
        };
        let items = args.get(2..).unwrap_or_default();
        let a = new_array(interp, 0)?;
        interp.rooted(&[Value::Object(a)], |interp| {
            let mut it = Indices::up(interp, o, start, start + delete_count)?;
            while let Some(k) = it.next(interp)? {
                if let Some(v) = element(interp, o, k)? {
                    create_at(interp, a, (k - start) as f64, v)?;
                }
            }
            set_length(interp, a, delete_count as f64)?;
            move_tail(interp, o, len, start, delete_count, items.len() as u64)?;
            for (j, v) in items.iter().enumerate() {
                interp.use_fuel()?;
                put_at(interp, o, (start + j as u64) as f64, v.clone())?;
            }
            set_length(interp, o, (len - delete_count + items.len() as u64) as f64)?;
            Ok(Value::Object(a))
        })
    })
}

/// Moves the elements after a spliced range so that `item_count` items replace `delete_count`
/// elements at `start` (section 15.4.4.12, steps 12 and 13).
fn move_tail(interp: &mut Interp, o: ObjId, len: u64, start: u64, delete_count: u64, item_count: u64) -> JsResult<()> {
    let (dc, ic) = (delete_count as i64, item_count as i64);
    if item_count < delete_count {
        let mut it = Indices::new(interp, o, start, len - delete_count, true, vec![Map::Shift(dc), Map::Shift(ic)])?;
        while let Some(k) = it.next(interp)? {
            move_element(interp, o, k + delete_count, k + item_count)?;
        }
        let mut it = Indices::down(interp, o, len - delete_count + item_count, len)?;
        while let Some(k) = it.next(interp)? {
            delete_at(interp, o, k as f64)?;
        }
    } else if item_count > delete_count {
        let mut it = Indices::new(interp, o, start + 1, len - delete_count + 1, false, vec![Map::Shift(dc - 1), Map::Shift(ic - 1)])?;
        while let Some(k) = it.next(interp)? {
            move_element(interp, o, k + delete_count - 1, k + item_count - 1)?;
        }
    }
    Ok(())
}

// ---- sort ----

/// A stable merge sort whose comparison may fail. `greater(a, b)` says whether `a` sorts after
/// `b`; each comparison takes a unit of fuel.
fn merge_sort<T: Clone>(interp: &mut Interp, items: &mut Vec<T>, mut greater: impl FnMut(&mut Interp, &T, &T) -> JsResult<bool>) -> JsResult<()> {
    let n = items.len();
    let mut width = 1;
    let mut buf: Vec<T> = Vec::with_capacity(n);
    while width < n {
        buf.clear();
        let mut lo = 0;
        while lo < n {
            let mid = (lo + width).min(n);
            let hi = (lo + 2 * width).min(n);
            let (mut i, mut j) = (lo, mid);
            while i < mid && j < hi {
                let (Some(a), Some(b)) = (items.get(i), items.get(j)) else { break };
                interp.use_fuel()?;
                if greater(interp, a, b)? {
                    buf.push(b.clone());
                    j += 1;
                } else {
                    buf.push(a.clone());
                    i += 1;
                }
            }
            buf.extend(items.get(i..mid).unwrap_or_default().iter().cloned());
            buf.extend(items.get(j..hi).unwrap_or_default().iter().cloned());
            lo = hi;
        }
        std::mem::swap(items, &mut buf);
        width *= 2;
    }
    Ok(())
}

/// `Array.prototype.sort` (section 15.4.4.11). The elements are read, sorted with `undefined`
/// after every other value, written back from index 0, and the indices after them are deleted.
fn sort(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let compare = arg(args, 0);
    if !matches!(compare, Value::Undefined) {
        require_callable(interp, &compare, "the comparison")?;
    }
    with_this(interp, &this, |interp, o| {
        let len = u64::from(length_of(interp, o)?);
        let mark = interp.temp_roots.len();
        let r = sort_elements(interp, o, len, &compare);
        interp.temp_roots.truncate(mark);
        r?;
        Ok(Value::Object(o))
    })
}

fn sort_elements(interp: &mut Interp, o: ObjId, len: u64, compare: &Value) -> JsResult<()> {
    let mut values = Vec::new();
    let mut undefined = 0u64;
    let mut it = Indices::up(interp, o, 0, len)?;
    while let Some(k) = it.next(interp)? {
        match element(interp, o, k)? {
            Some(Value::Undefined) => undefined += 1,
            Some(v) => {
                interp.reserve_heap(ELEMENT_BYTES)?;
                interp.temp_roots.push(v.clone());
                values.push(v);
            }
            None => {}
        }
    }
    if matches!(compare, Value::Undefined) {
        let mut keyed = Vec::with_capacity(values.len());
        for v in values {
            interp.use_fuel()?;
            let s = interp.to_str(&v)?.flat();
            interp.reserve_heap(crate::heap::string_bytes(s.len()))?;
            keyed.push((s, v));
        }
        merge_sort(interp, &mut keyed, |_, a, b| Ok(a.0 > b.0))?;
        values = keyed.into_iter().map(|(_, v)| v).collect();
    } else {
        merge_sort(interp, &mut values, |interp, a, b| {
            let r = interp.call(compare, &Value::Undefined, &[a.clone(), b.clone()])?;
            Ok(interp.to_number(&r)? > 0.0)
        })?;
    }
    let mut i = 0u64;
    for v in values {
        interp.use_fuel()?;
        put_at(interp, o, i as f64, v)?;
        i += 1;
    }
    for _ in 0..undefined {
        interp.use_fuel()?;
        put_at(interp, o, i as f64, Value::Undefined)?;
        i += 1;
    }
    let mut it = Indices::down(interp, o, i, len)?;
    while let Some(k) = it.next(interp)? {
        delete_at(interp, o, k as f64)?;
    }
    Ok(())
}

// ---- searching ----

/// `Array.prototype.indexOf` (section 15.4.4.14).
fn index_of(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    with_this(interp, &this, |interp, o| {
        let len = f64::from(length_of(interp, o)?);
        if len == 0.0 {
            return Ok(Value::Number(-1.0));
        }
        let n = if args.len() > 1 { to_integer(interp, &arg(args, 1))? } else { 0.0 };
        if n >= len {
            return Ok(Value::Number(-1.0));
        }
        let start = if n >= 0.0 { n } else { (len + n).max(0.0) };
        let target = arg(args, 0);
        let mut it = Indices::up(interp, o, start as u64, len as u64)?;
        while let Some(k) = it.next(interp)? {
            if let Some(v) = element(interp, o, k)?
                && strict_eq(&v, &target)
            {
                return Ok(number(k));
            }
        }
        Ok(Value::Number(-1.0))
    })
}

/// `Array.prototype.lastIndexOf` (section 15.4.4.15).
fn last_index_of(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    with_this(interp, &this, |interp, o| {
        let len = f64::from(length_of(interp, o)?);
        if len == 0.0 {
            return Ok(Value::Number(-1.0));
        }
        let n = if args.len() > 1 { to_integer(interp, &arg(args, 1))? } else { len - 1.0 };
        let from = if n >= 0.0 { n.min(len - 1.0) } else { len + n };
        if from < 0.0 {
            return Ok(Value::Number(-1.0));
        }
        let target = arg(args, 0);
        let mut it = Indices::down(interp, o, 0, from as u64 + 1)?;
        while let Some(k) = it.next(interp)? {
            if let Some(v) = element(interp, o, k)?
                && strict_eq(&v, &target)
            {
                return Ok(number(k));
            }
        }
        Ok(Value::Number(-1.0))
    })
}

// ---- iteration ----

/// What an iteration method does with each callback result.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Each {
    Every,
    Some,
    /// `forEach`.
    Visit,
    Map,
    Filter,
}

/// `every`, `some`, `forEach`, `map` and `filter` (sections 15.4.4.16 to 15.4.4.20).
fn iterate(interp: &mut Interp, this: Value, args: &[Value], kind: Each) -> JsResult<Value> {
    with_this(interp, &this, |interp, o| {
        let len = length_of(interp, o)?;
        let callback = arg(args, 0);
        require_callable(interp, &callback, "the callback")?;
        let this_arg = arg(args, 1);
        let a = match kind {
            Each::Map => Some(new_array(interp, len)?),
            Each::Filter => Some(new_array(interp, 0)?),
            _ => None,
        };
        let roots: Vec<Value> = a.map(Value::Object).into_iter().collect();
        interp.rooted(&roots, |interp| {
            let mut to = 0u64;
            let mut it = Indices::up(interp, o, 0, u64::from(len))?;
            while let Some(k) = it.next(interp)? {
                let Some(v) = element(interp, o, k)? else { continue };
                let r = interp.call(&callback, &this_arg, &[v.clone(), number(k), Value::Object(o)])?;
                match (kind, a) {
                    (Each::Every, _) if !r.truthy() => return Ok(Value::Bool(false)),
                    (Each::Some, _) if r.truthy() => return Ok(Value::Bool(true)),
                    (Each::Map, Some(a)) => create_at(interp, a, k as f64, r)?,
                    (Each::Filter, Some(a)) if r.truthy() => {
                        create_at(interp, a, to as f64, v)?;
                        to += 1;
                    }
                    _ => {}
                }
            }
            Ok(match (kind, a) {
                (Each::Every, _) => Value::Bool(true),
                (Each::Some, _) => Value::Bool(false),
                (_, Some(a)) => Value::Object(a),
                _ => Value::Undefined,
            })
        })
    })
}

fn every(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    iterate(interp, this, args, Each::Every)
}

fn some(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    iterate(interp, this, args, Each::Some)
}

fn for_each(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    iterate(interp, this, args, Each::Visit)
}

fn map(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    iterate(interp, this, args, Each::Map)
}

fn filter(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    iterate(interp, this, args, Each::Filter)
}

/// `reduce` and `reduceRight` (sections 15.4.4.21 and 15.4.4.22).
fn fold(interp: &mut Interp, this: Value, args: &[Value], right: bool) -> JsResult<Value> {
    with_this(interp, &this, |interp, o| {
        let len = u64::from(length_of(interp, o)?);
        let callback = arg(args, 0);
        require_callable(interp, &callback, "the callback")?;
        let mut it = Indices::new(interp, o, 0, len, !right, vec![Map::Shift(0)])?;
        let slot = interp.temp_roots.len();
        interp.temp_roots.push(Value::Undefined);
        // The accumulator is kept in a root slot while the elements are read.
        let keep = |interp: &mut Interp, acc: &Value| {
            if let Some(s) = interp.temp_roots.get_mut(slot) {
                *s = acc.clone();
            }
        };
        let r = (|| {
            let mut acc = match args.get(1) {
                Some(v) => v.clone(),
                None => loop {
                    let Some(k) = it.next(interp)? else { return Err(interp.type_error("reduce of an empty array with no initial value")) };
                    if let Some(v) = element(interp, o, k)? {
                        break v;
                    }
                },
            };
            keep(interp, &acc);
            while let Some(k) = it.next(interp)? {
                let Some(v) = element(interp, o, k)? else { continue };
                acc = interp.call(&callback, &Value::Undefined, &[acc, v, number(k), Value::Object(o)])?;
                keep(interp, &acc);
            }
            Ok(acc)
        })();
        interp.temp_roots.truncate(slot);
        r
    })
}

fn reduce(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    fold(interp, this, args, false)
}

fn reduce_right(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    fold(interp, this, args, true)
}
