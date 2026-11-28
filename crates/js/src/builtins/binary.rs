//! `ArrayBuffer`, `DataView` and the typed arrays (ECMA-262 6.0 sections 24.1, 24.2 and 22.2):
//! `Int8Array`, `Uint8Array`, `Uint8ClampedArray`, `Int16Array`, `Uint16Array`, `Int32Array`,
//! `Uint32Array`, `Float32Array` and `Float64Array`, with `%TypedArray%` and its prototype's
//! methods. A realm has them with [`crate::Options::es2015`].
//!
//! A buffer's bytes are zero when it is made and never change length; no operation detaches
//! one. Views share their buffer's bytes. Typed arrays store their elements little-endian on
//! every platform, and a `DataView` reads and writes big-endian unless `littleEndian` is true.
//! A typed array's numeric property names are its elements or nothing (section 9.4.5, with the
//! element descriptors of later editions: writable, enumerable and configurable, and an element
//! that cannot be deleted); [`crate::interp::Interp`] calls [`element`], [`set_element`] and
//! [`define_element`] for them.
//!
//! A buffer longer than the realm's heap limit, or than 2^32 - 1 bytes, is a `RangeError`, and
//! every buffer's bytes are checked against the heap before they are allocated. Every element
//! a method visits takes a unit of fuel, and a sort's comparisons take one each.
//!
//! The language has no iterators yet, so constructing from an object reads an array or an object
//! with a `length`, by index, and `%TypedArray%.from` reads those or a string, by code point. A
//! constructor given a primitive string reads it as a length.

use super::collections::{for_each_item, getter, length_of, new_iter, new_proto};
use super::{arg, array, data, method, native, require_callable, to_integer};
use crate::heap::{Class, Elem, HIDDEN, IterKind, IterSource, NativeFn, NativeKind, OBJECT_BYTES, ObjId, Object, VIEW_BYTES, View, key};
use crate::interp::{Abrupt, Desc, ErrorType, Interp, JsResult};
use crate::value::{Value, array_index, number_to_str, string_to_number, to_uint32};

/// The longest buffer, in bytes.
const MAX_BUFFER: u64 = u32::MAX as u64;

/// The bytes each item a method gathers before it writes them is charged.
const ITEM_BYTES: usize = 16;

pub(super) fn install(interp: &mut Interp) -> JsResult<()> {
    install_array_buffer(interp)?;
    install_data_view(interp)?;
    install_typed_arrays(interp)
}

fn install_array_buffer(interp: &mut Interp) -> JsResult<()> {
    let object_proto = interp.intrinsics.object_proto;
    let proto = new_proto(interp, object_proto);
    interp.intrinsics.es2015.array_buffer_proto = proto;
    let ctor = native(interp, "ArrayBuffer", array_buffer_ctor, 1, NativeKind::Plain, true)?;
    super::constructor(interp, "ArrayBuffer", ctor, proto)?;
    method(interp, ctor, "isView", is_view, 1)?;
    getter(interp, proto, "get byteLength", buffer_byte_length)?;
    method(interp, proto, "slice", buffer_slice, 2)
}

fn install_data_view(interp: &mut Interp) -> JsResult<()> {
    let object_proto = interp.intrinsics.object_proto;
    let proto = new_proto(interp, object_proto);
    interp.intrinsics.es2015.data_view_proto = proto;
    let ctor = native(interp, "DataView", data_view_ctor, 1, NativeKind::Plain, true)?;
    super::constructor(interp, "DataView", ctor, proto)?;
    getter(interp, proto, "get buffer", view_buffer)?;
    getter(interp, proto, "get byteLength", view_byte_length)?;
    getter(interp, proto, "get byteOffset", view_byte_offset)?;
    let methods: [(&'static str, NativeFn, u32); 16] = [
        ("getInt8", get_int8, 1),
        ("getUint8", get_uint8, 1),
        ("getInt16", get_int16, 1),
        ("getUint16", get_uint16, 1),
        ("getInt32", get_int32, 1),
        ("getUint32", get_uint32, 1),
        ("getFloat32", get_float32, 1),
        ("getFloat64", get_float64, 1),
        ("setInt8", set_int8, 2),
        ("setUint8", set_uint8, 2),
        ("setInt16", set_int16, 2),
        ("setUint16", set_uint16, 2),
        ("setInt32", set_int32, 2),
        ("setUint32", set_uint32, 2),
        ("setFloat32", set_float32, 2),
        ("setFloat64", set_float64, 2),
    ];
    for (name, f, length) in methods {
        method(interp, proto, name, f, length)?;
    }
    Ok(())
}

fn install_typed_arrays(interp: &mut Interp) -> JsResult<()> {
    let object_proto = interp.intrinsics.object_proto;
    let proto = new_proto(interp, object_proto);
    interp.intrinsics.es2015.typed_array_proto = proto;
    let ctor = native(interp, "TypedArray", abstract_ctor, 0, NativeKind::Plain, true)?;
    interp.intrinsics.es2015.typed_array_ctor = ctor;
    data(interp, ctor, "prototype", Value::Object(proto), 0)?;
    data(interp, proto, "constructor", Value::Object(ctor), HIDDEN)?;
    method(interp, ctor, "from", typed_from, 1)?;
    method(interp, ctor, "of", typed_of, 0)?;
    for (name, f) in
        [("get buffer", ta_buffer as NativeFn), ("get byteLength", ta_byte_length), ("get byteOffset", ta_byte_offset), ("get length", ta_length)]
    {
        getter(interp, proto, name, f)?;
    }
    let methods: [(&'static str, NativeFn, u32); 23] = [
        ("copyWithin", copy_within, 2),
        ("entries", ta_entries, 0),
        ("every", ta_every, 1),
        ("fill", fill, 1),
        ("filter", filter, 1),
        ("find", find, 1),
        ("findIndex", find_index, 1),
        ("forEach", ta_for_each, 1),
        ("indexOf", ta_index_of, 1),
        ("join", ta_join, 1),
        ("keys", ta_keys, 0),
        ("lastIndexOf", ta_last_index_of, 1),
        ("map", map, 1),
        ("reduce", ta_reduce, 1),
        ("reduceRight", ta_reduce_right, 1),
        ("reverse", ta_reverse, 0),
        ("set", set, 1),
        ("slice", slice, 2),
        ("some", ta_some, 1),
        ("sort", sort, 1),
        ("subarray", subarray, 2),
        ("toLocaleString", ta_to_locale_string, 0),
        ("values", ta_values, 0),
    ];
    for (name, f, length) in methods {
        method(interp, proto, name, f, length)?;
    }
    // `%TypedArray%.prototype.toString` is `Array.prototype.toString` (section 22.2.3.28).
    let array_proto = interp.intrinsics.array_proto;
    let to_string = interp.get_from(array_proto, &key("toString"), &Value::Object(array_proto))?;
    interp.define(proto, key("toString"), crate::heap::Prop::Data { value: to_string, flags: HIDDEN })?;

    let ctors: [NativeFn; 9] =
        [int8_ctor, uint8_ctor, uint8_clamped_ctor, int16_ctor, uint16_ctor, int32_ctor, uint32_ctor, float32_ctor, float64_ctor];
    for (elem, f) in Elem::ALL.into_iter().zip(ctors) {
        let sub_proto = new_proto(interp, proto);
        let sub = native(interp, elem.name(), f, 3, NativeKind::Plain, true)?;
        if let Some(o) = interp.heap.object_mut(sub) {
            o.proto = Some(ctor);
        }
        super::constructor(interp, elem.name(), sub, sub_proto)?;
        #[allow(clippy::cast_precision_loss)]
        let size = Value::Number(elem.size() as f64);
        data(interp, sub, "BYTES_PER_ELEMENT", size.clone(), 0)?;
        data(interp, sub_proto, "BYTES_PER_ELEMENT", size, 0)?;
        if let Some(slot) = interp.intrinsics.es2015.typed_ctors.get_mut(elem.index()) {
            *slot = sub;
        }
        if let Some(slot) = interp.intrinsics.es2015.typed_protos.get_mut(elem.index()) {
            *slot = sub_proto;
        }
    }
    Ok(())
}

// ---- elements ----

/// CanonicalNumericIndexString (section 7.1.16): the number a property name stands for when it
/// is the canonical text of that number, `-0` included.
pub(crate) fn canonical_index(k: &[u16]) -> Option<f64> {
    if let Some(i) = array_index(k) {
        return Some(f64::from(i));
    }
    let first = *k.first()?;
    if !(u16::from(b'0')..=u16::from(b'9')).contains(&first) && !b"-IN".iter().any(|c| u16::from(*c) == first) {
        return None;
    }
    if k == [u16::from(b'-'), u16::from(b'0')] {
        return Some(-0.0);
    }
    let n = string_to_number(k, true);
    (*number_to_str(n).flat() == *k).then_some(n)
}

/// The position of the element `n` names in a typed array of `length` elements, if it names one.
fn valid_index(n: f64, length: usize) -> Option<usize> {
    if n.fract() != 0.0 || (n == 0.0 && n.is_sign_negative()) || n < 0.0 || !n.is_finite() {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = n as usize;
    (i < length).then_some(i)
}

/// The buffer's bytes.
fn bytes(interp: &Interp, buffer: ObjId) -> Option<&[u8]> {
    match &interp.heap.object(buffer)?.class {
        Class::ArrayBuffer(b) => Some(b),
        _ => None,
    }
}

fn bytes_mut(interp: &mut Interp, buffer: ObjId) -> Option<&mut [u8]> {
    match &mut interp.heap.object_mut(buffer)?.class {
        Class::ArrayBuffer(b) => Some(b),
        _ => None,
    }
}

/// Reads a value of `elem` from the start of `b` (GetValueFromBuffer, section 24.1.1.5).
fn decode(b: &[u8], elem: Elem, little: bool) -> Option<f64> {
    let size = elem.size();
    let raw = b.get(..size)?;
    let mut bits = 0u64;
    for j in 0..size {
        let byte = if little { raw.get(j) } else { raw.get(size - 1 - j) };
        bits |= u64::from(*byte?) << (8 * j);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    Some(match elem {
        Elem::Int8 => f64::from(bits as u8 as i8),
        Elem::Uint8 | Elem::Uint8Clamped => f64::from(bits as u8),
        Elem::Int16 => f64::from(bits as u16 as i16),
        Elem::Uint16 => f64::from(bits as u16),
        Elem::Int32 => f64::from(bits as u32 as i32),
        Elem::Uint32 => f64::from(bits as u32),
        Elem::Float32 => f64::from(f32::from_bits(bits as u32)),
        Elem::Float64 => f64::from_bits(bits),
    })
}

/// ToUint8Clamp (section 7.1.11): rounds to the nearest integer in 0 to 255, ties to even.
pub(crate) fn to_uint8_clamp(n: f64) -> u8 {
    if n.is_nan() || n <= 0.0 {
        return 0;
    }
    if n >= 255.0 {
        return 255;
    }
    let f = n.floor();
    let r = if f + 0.5 < n {
        f + 1.0
    } else if n < f + 0.5 || f % 2.0 == 0.0 {
        f
    } else {
        f + 1.0
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = r as u8;
    byte
}

/// Writes `n` as a value of `elem` at the start of `b` (SetValueInBuffer, section 24.1.1.6).
/// The integer types keep the low bits of ToUint32, which equal those of ToInt8, ToInt16 and
/// ToInt32.
fn encode(b: &mut [u8], elem: Elem, n: f64, little: bool) {
    let size = elem.size();
    let bits = match elem {
        Elem::Uint8Clamped => u64::from(to_uint8_clamp(n)),
        #[allow(clippy::cast_possible_truncation)]
        Elem::Float32 => u64::from((n as f32).to_bits()),
        Elem::Float64 => n.to_bits(),
        _ => u64::from(to_uint32(n)),
    };
    for j in 0..size {
        let slot = if little { b.get_mut(j) } else { b.get_mut(size - 1 - j) };
        if let Some(slot) = slot {
            #[allow(clippy::cast_possible_truncation)]
            let byte = (bits >> (8 * j)) as u8;
            *slot = byte;
        }
    }
}

/// The element of `view` at the index `n`, or `None` when `n` names no element.
pub(crate) fn element(interp: &Interp, view: &View, n: f64) -> Option<f64> {
    let i = valid_index(n, view.length)?;
    let at = view.offset.checked_add(i.checked_mul(view.elem.size())?)?;
    decode(bytes(interp, view.buffer)?.get(at..)?, view.elem, true)
}

/// The view of the typed array `id`.
fn view_of(interp: &Interp, id: ObjId) -> Option<View> {
    match &interp.heap.object(id)?.class {
        Class::TypedArray(view) => Some(**view),
        _ => None,
    }
}

/// Writes `n` to the element at position `i` of `view`, when there is one.
fn write(interp: &mut Interp, view: &View, i: usize, n: f64) {
    let Some(at) = i.checked_mul(view.elem.size()).and_then(|o| o.checked_add(view.offset)) else { return };
    if i < view.length
        && let Some(b) = bytes_mut(interp, view.buffer).and_then(|b| b.get_mut(at..))
    {
        encode(b, view.elem, n, true);
    }
}

/// Copies `count` elements of `src` from position `from` to `dst` at position `to` byte for byte, as
/// a typed array is copied to one of the same element type (sections 22.2.3.22.2, 22.2.3.23 and
/// 22.2.4.3), so every bit pattern, a NaN's included, is kept. Overlapping ranges of one buffer copy
/// correctly. Each element takes a unit of fuel.
fn copy_elements(interp: &mut Interp, src: &View, from: usize, dst: &View, to: usize, count: usize) -> JsResult<()> {
    for _ in 0..count {
        interp.use_fuel()?;
    }
    let size = src.elem.size();
    let (s, d, n) = (src.offset + from * size, dst.offset + to * size, count * size);
    if src.buffer == dst.buffer {
        if let Some(b) = bytes_mut(interp, src.buffer)
            && s + n <= b.len()
            && d + n <= b.len()
        {
            b.copy_within(s..s + n, d);
        }
        return Ok(());
    }
    // The destination's bytes are moved out while the source's are read, so no third copy is made.
    let mut out = match &mut interp.object_mut(dst.buffer)?.class {
        Class::ArrayBuffer(b) => std::mem::take(b),
        _ => return Err(Abrupt::Internal("not an array buffer".into())),
    };
    if let (Some(from_bytes), Some(to_bytes)) = (bytes(interp, src.buffer).and_then(|b| b.get(s..s + n)), out.get_mut(d..d + n)) {
        to_bytes.copy_from_slice(from_bytes);
    }
    if let Class::ArrayBuffer(b) = &mut interp.object_mut(dst.buffer)?.class {
        *b = out;
    }
    Ok(())
}

/// The number a typed array `id`'s numeric property name `k` stands for.
pub(crate) fn typed_key(interp: &Interp, id: ObjId, k: &[u16]) -> Option<f64> {
    match &interp.heap.object(id)?.class {
        Class::TypedArray(_) => canonical_index(k),
        _ => None,
    }
}

/// `[[Set]]` of a numeric name of a typed array: the value is converted with ToNumber, then
/// written when the index names an element and ignored otherwise.
pub(crate) fn set_element(interp: &mut Interp, id: ObjId, n: f64, v: &Value) -> JsResult<()> {
    let num = interp.to_number(v)?;
    if let Some(view) = view_of(interp, id)
        && let Some(i) = valid_index(n, view.length)
    {
        write(interp, &view, i, num);
    }
    Ok(())
}

/// `[[DefineOwnProperty]]` of a numeric name of a typed array (section 9.4.5.3, with the
/// configurable elements of later editions).
pub(crate) fn define_element(interp: &mut Interp, id: ObjId, k: &[u16], desc: &Desc, throw: bool) -> JsResult<bool> {
    let view = view_of(interp, id);
    let index = canonical_index(k).zip(view).and_then(|(n, view)| valid_index(n, view.length));
    let refused =
        index.is_none() || desc.configurable == Some(false) || desc.enumerable == Some(false) || desc.is_accessor() || desc.writable == Some(false);
    if refused {
        if throw {
            let name = String::from_utf16_lossy(k);
            return Err(interp.type_error(&format!("cannot define property '{name}' of a typed array")));
        }
        return Ok(false);
    }
    if let (Some(v), Some(view), Some(i)) = (&desc.value, view, index) {
        let n = interp.to_number(v)?;
        write(interp, &view, i, n);
    }
    Ok(true)
}

// ---- making buffers and views ----

/// ToIndex (section 7.1.17 of later editions): `undefined` is 0, and a negative or too large
/// integer is a `RangeError`.
fn to_index(interp: &mut Interp, v: &Value, what: &str) -> JsResult<u64> {
    if matches!(v, Value::Undefined) {
        return Ok(0);
    }
    let n = to_integer(interp, v)?;
    if !(0.0..=9_007_199_254_740_991.0).contains(&n) {
        return Err(interp.throw(ErrorType::Range, &format!("{what} must be an integer from 0 to 2^53 - 1")));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(n as u64)
}

/// A new zeroed buffer of `len` bytes. A length past [`MAX_BUFFER`] or the heap limit is a
/// `RangeError`; otherwise the bytes and `extra` more are reserved before the buffer is
/// allocated.
fn new_buffer(interp: &mut Interp, len: u64, extra: usize) -> JsResult<ObjId> {
    if len > MAX_BUFFER || len > interp.limits.heap_bytes as u64 {
        return Err(interp.throw(ErrorType::Range, &format!("an array buffer of {len} bytes is larger than the heap limit allows")));
    }
    let len = usize::try_from(len).map_err(|_| interp.throw(ErrorType::Range, "the array buffer is too large"))?;
    interp.reserve_heap(OBJECT_BYTES.saturating_add(len).saturating_add(extra))?;
    let mut b = Vec::new();
    if b.try_reserve_exact(len).is_err() {
        return Err(interp.throw(ErrorType::Range, "the memory for the array buffer could not be allocated"));
    }
    b.resize(len, 0);
    let proto = interp.intrinsics.es2015.array_buffer_proto;
    let o = interp.heap.alloc_object(Object::new(Class::ArrayBuffer(b.into_boxed_slice()), Some(proto)));
    interp.heap.bytes = interp.heap.bytes.saturating_add(len);
    Ok(o)
}

/// A new view object of `class` on a buffer that is reachable. Its heap was reserved before.
fn new_view(interp: &mut Interp, class: Class, proto: ObjId) -> ObjId {
    let o = interp.heap.alloc_object(Object::new(class, Some(proto)));
    interp.heap.bytes = interp.heap.bytes.saturating_add(VIEW_BYTES);
    o
}

fn typed_proto(interp: &Interp, elem: Elem) -> ObjId {
    interp.intrinsics.es2015.typed_protos.get(elem.index()).copied().unwrap_or(interp.intrinsics.es2015.typed_array_proto)
}

/// A new typed array of `len` zero elements of `elem`, with a new buffer.
fn new_typed(interp: &mut Interp, elem: Elem, len: u64) -> JsResult<ObjId> {
    let byte_len = len.saturating_mul(elem.size() as u64);
    let buffer = new_buffer(interp, byte_len, OBJECT_BYTES + VIEW_BYTES)?;
    let length = usize::try_from(len).unwrap_or(0);
    let proto = typed_proto(interp, elem);
    Ok(new_view(interp, Class::TypedArray(Box::new(View { buffer, offset: 0, length, elem })), proto))
}

/// `new ArrayBuffer(length)` (section 24.1.2.1).
fn array_buffer_ctor(interp: &mut Interp, _: Value, args: &[Value], constructing: bool) -> JsResult<Value> {
    if !constructing {
        return Err(interp.type_error("ArrayBuffer must be called with new"));
    }
    let len = to_index(interp, &arg(args, 0), "the length")?;
    new_buffer(interp, len, 0).map(Value::Object)
}

/// `ArrayBuffer.isView` (section 24.1.3.1).
fn is_view(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let view =
        arg(args, 0).as_object().and_then(|o| interp.heap.object(o)).is_some_and(|o| matches!(o.class, Class::DataView(_) | Class::TypedArray(_)));
    Ok(Value::Bool(view))
}

fn this_buffer(interp: &mut Interp, this: &Value, name: &str) -> JsResult<ObjId> {
    match this.as_object() {
        Some(o) if bytes(interp, o).is_some() => Ok(o),
        _ => Err(interp.type_error(&format!("ArrayBuffer.prototype.{name} was called on a value that is not an ArrayBuffer"))),
    }
}

fn buffer_len(interp: &Interp, o: ObjId) -> usize {
    bytes(interp, o).map_or(0, <[u8]>::len)
}

fn buffer_byte_length(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let o = this_buffer(interp, &this, "byteLength")?;
    #[allow(clippy::cast_precision_loss)]
    Ok(Value::Number(buffer_len(interp, o) as f64))
}

/// A relative index (as `slice` reads its arguments): negative counts from `len`, and the result
/// is clamped to 0 to `len`. `undefined` gives `default`.
fn relative(interp: &mut Interp, v: &Value, len: usize, default: usize) -> JsResult<usize> {
    if matches!(v, Value::Undefined) {
        return Ok(default);
    }
    let n = to_integer(interp, v)?;
    #[allow(clippy::cast_precision_loss)]
    let len_f = len as f64;
    let r = if n < 0.0 { (len_f + n).max(0.0) } else { n.min(len_f) };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(r as usize)
}

/// The `constructor` steps of SpeciesConstructor (section 7.3.20): reads `o.constructor` and
/// throws a `TypeError` when it is neither `undefined` nor an object. The language has no
/// `@@species`, so the result is always made with the intrinsic constructor.
fn species_check(interp: &mut Interp, o: ObjId, name: &str) -> JsResult<()> {
    let c = interp.get_from(o, &key("constructor"), &Value::Object(o))?;
    if matches!(c, Value::Undefined | Value::Object(_)) {
        return Ok(());
    }
    Err(interp.type_error(&format!("{name}: the constructor property is not an object")))
}

/// `ArrayBuffer.prototype.slice` (section 24.1.4.3).
fn buffer_slice(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = this_buffer(interp, &this, "slice")?;
    let len = buffer_len(interp, o);
    let first = relative(interp, &arg(args, 0), len, 0)?;
    let last = relative(interp, &arg(args, 1), len, len)?;
    species_check(interp, o, "ArrayBuffer.prototype.slice")?;
    let count = last.saturating_sub(first);
    // Each copied byte takes a unit of fuel, charged before the copy is made.
    for _ in 0..count {
        interp.use_fuel()?;
    }
    let new = interp.rooted(std::slice::from_ref(&this), |i| new_buffer(i, count as u64, 0))?;
    // The new buffer's bytes are moved out while the source's are read, so no third copy is made.
    let mut dst = match &mut interp.object_mut(new)?.class {
        Class::ArrayBuffer(b) => std::mem::take(b),
        _ => return Err(Abrupt::Internal("not an array buffer".into())),
    };
    if let (Some(src), Some(to)) = (bytes(interp, o).and_then(|b| b.get(first..first + count)), dst.get_mut(..count)) {
        to.copy_from_slice(src);
    }
    if let Class::ArrayBuffer(b) = &mut interp.object_mut(new)?.class {
        *b = dst;
    }
    Ok(Value::Object(new))
}

// ---- DataView ----

/// `new DataView(buffer, byteOffset, byteLength)` (section 24.2.2.1).
fn data_view_ctor(interp: &mut Interp, _: Value, args: &[Value], constructing: bool) -> JsResult<Value> {
    if !constructing {
        return Err(interp.type_error("DataView must be called with new"));
    }
    let buffer = match arg(args, 0).as_object() {
        Some(o) if bytes(interp, o).is_some() => o,
        _ => return Err(interp.type_error("the first argument of DataView must be an ArrayBuffer")),
    };
    let offset = to_index(interp, &arg(args, 1), "the byte offset")?;
    let len = buffer_len(interp, buffer) as u64;
    if offset > len {
        return Err(interp.throw(ErrorType::Range, "the byte offset is past the end of the buffer"));
    }
    let view_len = match arg(args, 2) {
        Value::Undefined => len - offset,
        v => to_index(interp, &v, "the byte length")?,
    };
    if offset.saturating_add(view_len) > len {
        return Err(interp.throw(ErrorType::Range, "the view does not fit in the buffer"));
    }
    interp.rooted(&[Value::Object(buffer)], |i| i.reserve_heap(OBJECT_BYTES + VIEW_BYTES))?;
    let proto = interp.intrinsics.es2015.data_view_proto;
    let view = View { buffer, offset: usize::try_from(offset).unwrap_or(0), length: usize::try_from(view_len).unwrap_or(0), elem: Elem::Uint8 };
    Ok(Value::Object(new_view(interp, Class::DataView(Box::new(view)), proto)))
}

fn this_data_view(interp: &mut Interp, this: &Value, name: &str) -> JsResult<View> {
    if let Some(Class::DataView(view)) = this.as_object().and_then(|o| interp.heap.object(o)).map(|o| &o.class) {
        return Ok(**view);
    }
    Err(interp.type_error(&format!("DataView.prototype.{name} was called on a value that is not a DataView")))
}

fn view_buffer(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    Ok(Value::Object(this_data_view(interp, &this, "buffer")?.buffer))
}

fn view_byte_length(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    #[allow(clippy::cast_precision_loss)]
    Ok(Value::Number(this_data_view(interp, &this, "byteLength")?.length as f64))
}

fn view_byte_offset(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    #[allow(clippy::cast_precision_loss)]
    Ok(Value::Number(this_data_view(interp, &this, "byteOffset")?.offset as f64))
}

/// The byte position in the buffer of a `DataView` access of `elem` at `index`, or a
/// `RangeError` when it does not fit in the view (sections 24.2.1.1 and 24.2.1.2).
fn view_position(interp: &mut Interp, view: &View, index: u64, elem: Elem) -> JsResult<usize> {
    let end = index.saturating_add(elem.size() as u64);
    if end > view.length as u64 {
        return Err(interp.throw(ErrorType::Range, "the offset is outside the bounds of the DataView"));
    }
    Ok(view.offset + usize::try_from(index).unwrap_or(0))
}

/// GetViewValue (section 24.2.1.1).
fn view_get(interp: &mut Interp, this: &Value, args: &[Value], elem: Elem, name: &str) -> JsResult<Value> {
    let view = this_data_view(interp, this, name)?;
    let index = to_index(interp, &arg(args, 0), "the offset")?;
    let little = arg(args, 1).truthy();
    let at = view_position(interp, &view, index, elem)?;
    let n = bytes(interp, view.buffer).and_then(|b| b.get(at..)).and_then(|b| decode(b, elem, little)).unwrap_or(0.0);
    Ok(Value::Number(n))
}

/// SetViewValue (section 24.2.1.2): the offset, then the value, then `littleEndian` are read
/// before the bounds are checked.
fn view_set(interp: &mut Interp, this: &Value, args: &[Value], elem: Elem, name: &str) -> JsResult<Value> {
    let view = this_data_view(interp, this, name)?;
    let index = to_index(interp, &arg(args, 0), "the offset")?;
    let n = interp.to_number(&arg(args, 1))?;
    let little = arg(args, 2).truthy();
    let at = view_position(interp, &view, index, elem)?;
    if let Some(b) = bytes_mut(interp, view.buffer).and_then(|b| b.get_mut(at..)) {
        encode(b, elem, n, little);
    }
    Ok(Value::Undefined)
}

macro_rules! view_methods {
    ($($get:ident, $set:ident, $elem:ident, $getname:literal, $setname:literal;)*) => {
        $(
            fn $get(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
                view_get(interp, &this, args, Elem::$elem, $getname)
            }

            fn $set(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
                view_set(interp, &this, args, Elem::$elem, $setname)
            }
        )*
    };
}

view_methods! {
    get_int8, set_int8, Int8, "getInt8", "setInt8";
    get_uint8, set_uint8, Uint8, "getUint8", "setUint8";
    get_int16, set_int16, Int16, "getInt16", "setInt16";
    get_uint16, set_uint16, Uint16, "getUint16", "setUint16";
    get_int32, set_int32, Int32, "getInt32", "setInt32";
    get_uint32, set_uint32, Uint32, "getUint32", "setUint32";
    get_float32, set_float32, Float32, "getFloat32", "setFloat32";
    get_float64, set_float64, Float64, "getFloat64", "setFloat64";
}

// ---- typed array constructors ----

/// `%TypedArray%` itself (section 22.2.1.1) cannot be called or constructed.
fn abstract_ctor(interp: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
    Err(interp.type_error("TypedArray cannot be called or constructed; use a typed array constructor such as Uint8Array"))
}

/// `new Int8Array(…)` and the other typed array constructors (section 22.2.4).
fn typed_ctor(interp: &mut Interp, args: &[Value], constructing: bool, elem: Elem) -> JsResult<Value> {
    if !constructing {
        return Err(interp.type_error(&format!("{} must be called with new", elem.name())));
    }
    let first = arg(args, 0);
    let Value::Object(o) = first else {
        let len = to_index(interp, &first, "the length")?;
        return new_typed(interp, elem, len).map(Value::Object);
    };
    if bytes(interp, o).is_some() {
        return typed_on_buffer(interp, o, &arg(args, 1), &arg(args, 2), elem).map(Value::Object);
    }
    if let Some(src) = view_of(interp, o) {
        let ta = new_typed(interp, elem, src.length as u64)?;
        let dst = view_of(interp, ta).ok_or_else(|| Abrupt::Internal("not a typed array".into()))?;
        if src.elem == elem {
            copy_elements(interp, &src, 0, &dst, 0, src.length)?;
            return Ok(Value::Object(ta));
        }
        for i in 0..src.length {
            interp.use_fuel()?;
            #[allow(clippy::cast_precision_loss)]
            let n = element(interp, &src, i as f64).unwrap_or(0.0);
            write(interp, &dst, i, n);
        }
        return Ok(Value::Object(ta));
    }
    let len = length_of(interp, o)?;
    let ta = new_typed(interp, elem, len)?;
    interp.rooted(&[Value::Object(ta)], |interp| fill_from(interp, ta, &first, len))?;
    Ok(Value::Object(ta))
}

/// Writes the elements `0` to `len - 1` of the array-like object `src` to the typed array `ta`,
/// each converted with ToNumber.
fn fill_from(interp: &mut Interp, ta: ObjId, src: &Value, len: u64) -> JsResult<()> {
    let Value::Object(o) = src else { return Ok(()) };
    let view = view_of(interp, ta).ok_or_else(|| Abrupt::Internal("not a typed array".into()))?;
    for i in 0..len {
        interp.use_fuel()?;
        #[allow(clippy::cast_precision_loss)]
        let v = interp.get_from(*o, &super::key_of(i as f64), src)?;
        let n = interp.to_number(&v)?;
        write(interp, &view, usize::try_from(i).unwrap_or(usize::MAX), n);
    }
    Ok(())
}

/// A typed array on part of `buffer` (section 22.2.4.5).
fn typed_on_buffer(interp: &mut Interp, buffer: ObjId, offset: &Value, length: &Value, elem: Elem) -> JsResult<ObjId> {
    let size = elem.size() as u64;
    let offset = to_index(interp, offset, "the byte offset")?;
    if !offset.is_multiple_of(size) {
        return Err(interp.throw(ErrorType::Range, &format!("the byte offset of a {} must be a multiple of {size}", elem.name())));
    }
    let new_length = match length {
        Value::Undefined => None,
        v => Some(to_index(interp, v, "the length")?),
    };
    let buf_len = buffer_len(interp, buffer) as u64;
    let byte_len = match new_length {
        None => {
            if !buf_len.is_multiple_of(size) {
                return Err(interp.throw(ErrorType::Range, &format!("the buffer's length must be a multiple of {size}")));
            }
            buf_len.checked_sub(offset).ok_or_else(|| interp.throw(ErrorType::Range, "the byte offset is past the end of the buffer"))?
        }
        Some(n) => n.saturating_mul(size),
    };
    if offset.saturating_add(byte_len) > buf_len {
        return Err(interp.throw(ErrorType::Range, "the typed array does not fit in the buffer"));
    }
    interp.rooted(&[Value::Object(buffer)], |i| i.reserve_heap(OBJECT_BYTES + VIEW_BYTES))?;
    let proto = typed_proto(interp, elem);
    let view = View { buffer, offset: usize::try_from(offset).unwrap_or(0), length: usize::try_from(byte_len / size).unwrap_or(0), elem };
    Ok(new_view(interp, Class::TypedArray(Box::new(view)), proto))
}

macro_rules! typed_ctors {
    ($($f:ident, $elem:ident;)*) => {
        $(
            fn $f(interp: &mut Interp, _: Value, args: &[Value], constructing: bool) -> JsResult<Value> {
                typed_ctor(interp, args, constructing, Elem::$elem)
            }
        )*
    };
}

typed_ctors! {
    int8_ctor, Int8;
    uint8_ctor, Uint8;
    uint8_clamped_ctor, Uint8Clamped;
    int16_ctor, Int16;
    uint16_ctor, Uint16;
    int32_ctor, Int32;
    uint32_ctor, Uint32;
    float32_ctor, Float32;
    float64_ctor, Float64;
}

/// The element type of the typed array constructor `this`, as `from` and `of` need.
fn ctor_elem(interp: &mut Interp, this: &Value, name: &str) -> JsResult<Elem> {
    let ctors = interp.intrinsics.es2015.typed_ctors;
    if let Some(o) = this.as_object()
        && let Some(i) = ctors.iter().position(|c| *c == o)
        && let Some(elem) = Elem::ALL.get(i)
    {
        return Ok(*elem);
    }
    Err(interp.type_error(&format!("TypedArray.{name} must be called on a typed array constructor")))
}

/// `%TypedArray%.from(source, mapFn, thisArg)` (section 22.2.2.1).
fn typed_from(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    if !interp.is_constructor(&this) {
        return Err(interp.type_error("TypedArray.from must be called on a typed array constructor"));
    }
    let map_fn = arg(args, 1);
    let mapping = !matches!(map_fn, Value::Undefined);
    if mapping {
        require_callable(interp, &map_fn, "the map function")?;
    }
    let this_arg = arg(args, 2);
    let source = arg(args, 0);
    if !matches!(source, Value::String(_)) {
        // An array-like source: each element is read, mapped and written before the next is read.
        let src = interp.to_object(&source)?;
        let src_value = Value::Object(src);
        let len = length_of(interp, src)?;
        let elem = ctor_elem(interp, &this, "from")?;
        let ta = interp.rooted(std::slice::from_ref(&src_value), |interp| new_typed(interp, elem, len))?;
        let view = view_of(interp, ta).ok_or_else(|| Abrupt::Internal("not a typed array".into()))?;
        return interp.rooted(&[Value::Object(ta), src_value.clone()], |interp| {
            for i in 0..len {
                interp.use_fuel()?;
                #[allow(clippy::cast_precision_loss)]
                let item = interp.get_from(src, &super::key_of(i as f64), &src_value)?;
                #[allow(clippy::cast_precision_loss)]
                let v = if mapping { interp.call(&map_fn, &this_arg, &[item, Value::Number(i as f64)])? } else { item };
                let n = interp.to_number(&v)?;
                write(interp, &view, usize::try_from(i).unwrap_or(usize::MAX), n);
            }
            Ok(Value::Object(ta))
        });
    }
    // A string is read by code point first, as an iterable source is, and then mapped.
    let mark = interp.temp_roots.len();
    let mut held = crate::interp::HeldBytes::new(interp);
    let gathered = for_each_item(interp, &source, false, |interp, item| {
        held.add(interp, ITEM_BYTES)?;
        interp.temp_roots.push(item);
        Ok(())
    });
    let result = gathered.and_then(|()| {
        let items: Vec<Value> = interp.temp_roots.get(mark..).map(<[Value]>::to_vec).unwrap_or_default();
        let elem = ctor_elem(interp, &this, "from")?;
        let ta = new_typed(interp, elem, items.len() as u64)?;
        interp.temp_roots.push(Value::Object(ta));
        let view = view_of(interp, ta).ok_or_else(|| Abrupt::Internal("not a typed array".into()))?;
        for (i, item) in items.into_iter().enumerate() {
            interp.use_fuel()?;
            #[allow(clippy::cast_precision_loss)]
            let v = if mapping { interp.call(&map_fn, &this_arg, &[item, Value::Number(i as f64)])? } else { item };
            let n = interp.to_number(&v)?;
            write(interp, &view, i, n);
        }
        Ok(Value::Object(ta))
    });
    interp.temp_roots.truncate(mark);
    drop(held);
    result
}

/// `%TypedArray%.of(...items)` (section 22.2.2.2).
fn typed_of(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let elem = ctor_elem(interp, &this, "of")?;
    let ta = new_typed(interp, elem, args.len() as u64)?;
    let view = view_of(interp, ta).ok_or_else(|| Abrupt::Internal("not a typed array".into()))?;
    interp.rooted(&[Value::Object(ta)], |interp| {
        for (i, v) in args.iter().enumerate() {
            interp.use_fuel()?;
            let n = interp.to_number(v)?;
            write(interp, &view, i, n);
        }
        Ok(Value::Object(ta))
    })
}

// ---- %TypedArray%.prototype ----

/// The typed array a method was called on (ValidateTypedArray, section 22.2.3.5.1).
fn this_typed(interp: &mut Interp, this: &Value, name: &str) -> JsResult<(ObjId, View)> {
    if let Some(o) = this.as_object()
        && let Some(view) = view_of(interp, o)
    {
        return Ok((o, view));
    }
    Err(interp.type_error(&format!("TypedArray.prototype.{name} was called on a value that is not a typed array")))
}

fn ta_buffer(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    Ok(Value::Object(this_typed(interp, &this, "buffer")?.1.buffer))
}

fn ta_byte_length(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let view = this_typed(interp, &this, "byteLength")?.1;
    #[allow(clippy::cast_precision_loss)]
    Ok(Value::Number((view.length * view.elem.size()) as f64))
}

fn ta_byte_offset(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    #[allow(clippy::cast_precision_loss)]
    Ok(Value::Number(this_typed(interp, &this, "byteOffset")?.1.offset as f64))
}

fn ta_length(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    #[allow(clippy::cast_precision_loss)]
    Ok(Value::Number(this_typed(interp, &this, "length")?.1.length as f64))
}

/// The element at position `i`, 0 past the end.
fn at(interp: &Interp, view: &View, i: usize) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    element(interp, view, i as f64).unwrap_or(0.0)
}

/// Runs the array prototype's generic method `f` on a typed array, after checking `this`.
/// The array method reads the typed array's element count as its length, not its `length`
/// property.
fn generic(interp: &mut Interp, this: Value, args: &[Value], name: &str, f: NativeFn) -> JsResult<Value> {
    let (o, _) = this_typed(interp, &this, name)?;
    let saved = interp.typed_length.replace(o);
    let result = f(interp, this, args, false);
    interp.typed_length = saved;
    result
}

macro_rules! generic_methods {
    ($($f:ident, $name:literal, $array:path;)*) => {
        $(
            fn $f(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
                generic(interp, this, args, $name, $array)
            }
        )*
    };
}

generic_methods! {
    ta_every, "every", array::every;
    ta_some, "some", array::some;
    ta_for_each, "forEach", array::for_each;
    ta_index_of, "indexOf", array::index_of;
    ta_last_index_of, "lastIndexOf", array::last_index_of;
    ta_join, "join", array::join;
    ta_reduce, "reduce", array::reduce;
    ta_reduce_right, "reduceRight", array::reduce_right;
    ta_reverse, "reverse", array::reverse;
    ta_to_locale_string, "toLocaleString", array::typed_to_locale_string;
}

fn iter_of(interp: &mut Interp, this: &Value, kind: IterKind, name: &str) -> JsResult<Value> {
    let (o, _) = this_typed(interp, this, name)?;
    new_iter(interp, o, IterSource::Array, kind, None).map(Value::Object)
}

fn ta_keys(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    iter_of(interp, &this, IterKind::Keys, "keys")
}

fn ta_values(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    iter_of(interp, &this, IterKind::Values, "values")
}

fn ta_entries(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    iter_of(interp, &this, IterKind::Entries, "entries")
}

/// `fill(value, start, end)` (section 22.2.3.8): the value is converted once, before the range.
fn fill(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (_, view) = this_typed(interp, &this, "fill")?;
    let n = interp.to_number(&arg(args, 0))?;
    let start = relative(interp, &arg(args, 1), view.length, 0)?;
    let end = relative(interp, &arg(args, 2), view.length, view.length)?;
    for i in start..end {
        interp.use_fuel()?;
        write(interp, &view, i, n);
    }
    Ok(this)
}

/// Calls `f(element, index, array)` for each element in order until `stop` says the result is
/// found; returns the index and the element.
fn search(interp: &mut Interp, this: &Value, args: &[Value], name: &str) -> JsResult<Option<(usize, f64)>> {
    let (_, view) = this_typed(interp, this, name)?;
    let f = arg(args, 0);
    require_callable(interp, &f, "the predicate")?;
    let this_arg = arg(args, 1);
    for i in 0..view.length {
        interp.use_fuel()?;
        let v = at(interp, &view, i);
        #[allow(clippy::cast_precision_loss)]
        let r = interp.call(&f, &this_arg, &[Value::Number(v), Value::Number(i as f64), this.clone()])?;
        if r.truthy() {
            return Ok(Some((i, v)));
        }
    }
    Ok(None)
}

/// `find` (section 22.2.3.10).
fn find(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    Ok(search(interp, &this, args, "find")?.map_or(Value::Undefined, |(_, v)| Value::Number(v)))
}

/// `findIndex` (section 22.2.3.11).
fn find_index(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    #[allow(clippy::cast_precision_loss)]
    Ok(Value::Number(search(interp, &this, args, "findIndex")?.map_or(-1.0, |(i, _)| i as f64)))
}

/// `map` (section 22.2.3.18): a new typed array of the same type holding each callback result.
fn map(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, view) = this_typed(interp, &this, "map")?;
    let f = arg(args, 0);
    require_callable(interp, &f, "the callback")?;
    let this_arg = arg(args, 1);
    species_check(interp, o, "TypedArray.prototype.map")?;
    let out = new_typed(interp, view.elem, view.length as u64)?;
    let dst = view_of(interp, out).ok_or_else(|| Abrupt::Internal("not a typed array".into()))?;
    interp.rooted(&[Value::Object(out)], |interp| {
        for i in 0..view.length {
            interp.use_fuel()?;
            let v = at(interp, &view, i);
            #[allow(clippy::cast_precision_loss)]
            let r = interp.call(&f, &this_arg, &[Value::Number(v), Value::Number(i as f64), this.clone()])?;
            let n = interp.to_number(&r)?;
            write(interp, &dst, i, n);
        }
        Ok(Value::Object(out))
    })
}

/// A new typed array of `elem` holding `values`.
fn typed_from_values(interp: &mut Interp, elem: Elem, values: &[f64]) -> JsResult<ObjId> {
    let out = new_typed(interp, elem, values.len() as u64)?;
    let dst = view_of(interp, out).ok_or_else(|| Abrupt::Internal("not a typed array".into()))?;
    for (i, n) in values.iter().enumerate() {
        write(interp, &dst, i, *n);
    }
    Ok(out)
}

/// `filter` (section 22.2.3.9).
fn filter(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, view) = this_typed(interp, &this, "filter")?;
    let f = arg(args, 0);
    require_callable(interp, &f, "the callback")?;
    let this_arg = arg(args, 1);
    species_check(interp, o, "TypedArray.prototype.filter")?;
    let mut held = crate::interp::HeldBytes::new(interp);
    let mut kept = Vec::new();
    for i in 0..view.length {
        interp.use_fuel()?;
        let v = at(interp, &view, i);
        #[allow(clippy::cast_precision_loss)]
        let r = interp.call(&f, &this_arg, &[Value::Number(v), Value::Number(i as f64), this.clone()])?;
        if r.truthy() {
            held.add(interp, 8)?;
            kept.push(v);
        }
    }
    let out = typed_from_values(interp, view.elem, &kept)?;
    drop(held);
    Ok(Value::Object(out))
}

/// `slice(start, end)` (section 22.2.3.23): a copy of the range in a new buffer.
fn slice(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, view) = this_typed(interp, &this, "slice")?;
    let start = relative(interp, &arg(args, 0), view.length, 0)?;
    let end = relative(interp, &arg(args, 1), view.length, view.length)?;
    let count = end.saturating_sub(start);
    species_check(interp, o, "TypedArray.prototype.slice")?;
    let out = new_typed(interp, view.elem, count as u64)?;
    let dst = view_of(interp, out).ok_or_else(|| Abrupt::Internal("not a typed array".into()))?;
    copy_elements(interp, &view, start, &dst, 0, count)?;
    Ok(Value::Object(out))
}

/// `subarray(begin, end)` (section 22.2.3.26): a new view on the same buffer.
fn subarray(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, view) = this_typed(interp, &this, "subarray")?;
    let begin = relative(interp, &arg(args, 0), view.length, 0)?;
    let end = relative(interp, &arg(args, 1), view.length, view.length)?;
    species_check(interp, o, "TypedArray.prototype.subarray")?;
    interp.rooted(std::slice::from_ref(&this), |i| i.reserve_heap(OBJECT_BYTES + VIEW_BYTES))?;
    let sub = View { buffer: view.buffer, offset: view.offset + begin * view.elem.size(), length: end.saturating_sub(begin), elem: view.elem };
    let proto = typed_proto(interp, view.elem);
    Ok(Value::Object(new_view(interp, Class::TypedArray(Box::new(sub)), proto)))
}

/// `set(source, offset)` (section 22.2.3.22): writes a typed array's elements, or an array-like
/// object's elements converted with ToNumber, starting at `offset`. A source that does not fit
/// is a `RangeError`. A typed array source on the same buffer is read in full before anything is
/// written, so it copies correctly.
fn set(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (_, view) = this_typed(interp, &this, "set")?;
    let offset = to_integer(interp, &arg(args, 1))?;
    if offset < 0.0 {
        return Err(interp.throw(ErrorType::Range, "the offset must not be negative"));
    }
    let source = arg(args, 0);
    let src_view = source.as_object().and_then(|o| view_of(interp, o));
    let src_len = match (&source, src_view) {
        (_, Some(v)) => v.length as u64,
        (Value::Object(o), None) => length_of(interp, *o)?,
        _ => {
            let o = interp.to_object(&source)?;
            length_of(interp, o)?
        }
    };
    #[allow(clippy::cast_precision_loss)]
    if offset + src_len as f64 > view.length as f64 {
        return Err(interp.throw(ErrorType::Range, "the source does not fit in the typed array at that offset"));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let offset = offset as usize;
    let count = usize::try_from(src_len).unwrap_or(0);
    if let Some(src) = src_view.filter(|src| src.elem == view.elem) {
        copy_elements(interp, &src, 0, &view, offset, count)?;
        return Ok(Value::Undefined);
    }
    if let Some(src) = src_view.filter(|src| src.buffer != view.buffer) {
        // On another buffer, the writes do not change the source, so each element is written as
        // it is read.
        for i in 0..count {
            interp.use_fuel()?;
            let n = at(interp, &src, i);
            write(interp, &view, offset + i, n);
        }
        return Ok(Value::Undefined);
    }
    if let Some(src) = src_view {
        let mut held = crate::interp::HeldBytes::new(interp);
        held.add(interp, count.saturating_mul(8))?;
        let mut values = Vec::with_capacity(count);
        for i in 0..count {
            interp.use_fuel()?;
            values.push(at(interp, &src, i));
        }
        for (i, n) in values.into_iter().enumerate() {
            write(interp, &view, offset + i, n);
        }
        return Ok(Value::Undefined);
    }
    let src = interp.to_object(&source)?;
    interp.rooted(&[Value::Object(src)], |interp| {
        for i in 0..count {
            interp.use_fuel()?;
            #[allow(clippy::cast_precision_loss)]
            let v = interp.get_from(src, &super::key_of(i as f64), &Value::Object(src))?;
            let n = interp.to_number(&v)?;
            write(interp, &view, offset + i, n);
        }
        Ok(Value::Undefined)
    })
}

/// `copyWithin(target, start, end)` (section 22.2.3.5): moves the bytes of the range within the
/// buffer, overlapping ranges included.
fn copy_within(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (_, view) = this_typed(interp, &this, "copyWithin")?;
    let len = view.length;
    let to = relative(interp, &arg(args, 0), len, 0)?;
    let from = relative(interp, &arg(args, 1), len, 0)?;
    let end = relative(interp, &arg(args, 2), len, len)?;
    let count = end.saturating_sub(from).min(len - to);
    copy_elements(interp, &view, from, &view, to, count)?;
    Ok(this)
}

/// The default order of `sort` (section 22.2.3.25): numeric, with `-0` before `+0` and `NaN`
/// last.
fn default_order(a: f64, b: f64) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ if a == 0.0 && b == 0.0 => b.is_sign_negative().cmp(&a.is_sign_negative()),
        _ => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}

/// Whether `b` goes before `a`: by `compare` when it is a function, the default order otherwise.
/// Each comparison takes a unit of fuel.
fn goes_before(interp: &mut Interp, compare: &Value, a: f64, b: f64) -> JsResult<bool> {
    interp.use_fuel()?;
    if matches!(compare, Value::Undefined) {
        return Ok(default_order(b, a) == std::cmp::Ordering::Less);
    }
    let r = interp.call(compare, &Value::Undefined, &[Value::Number(a), Value::Number(b)])?;
    let n = interp.to_number(&r)?;
    Ok(n > 0.0)
}

/// Sorts `values` stably with a merge sort whose comparisons may fail.
fn merge_sort(interp: &mut Interp, values: &mut Vec<f64>, compare: &Value) -> JsResult<()> {
    let n = values.len();
    let mut buf = values.clone();
    let mut width = 1;
    while width < n {
        let mut lo = 0;
        while lo < n {
            let mid = (lo + width).min(n);
            let hi = (lo + 2 * width).min(n);
            let (mut i, mut j) = (lo, mid);
            for slot in buf.iter_mut().take(hi).skip(lo) {
                let take_right = match (values.get(i), values.get(j)) {
                    (Some(&a), Some(&b)) if i < mid && j < hi => goes_before(interp, compare, a, b)?,
                    _ => i >= mid,
                };
                let pick = if take_right { &mut j } else { &mut i };
                *slot = values.get(*pick).copied().unwrap_or(0.0);
                *pick += 1;
            }
            lo = hi;
        }
        std::mem::swap(values, &mut buf);
        width *= 2;
    }
    Ok(())
}

/// `sort(comparefn)` (section 22.2.3.25).
fn sort(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let compare = arg(args, 0);
    if !matches!(compare, Value::Undefined) {
        require_callable(interp, &compare, "the comparison function")?;
    }
    let (_, view) = this_typed(interp, &this, "sort")?;
    let mut held = crate::interp::HeldBytes::new(interp);
    held.add(interp, view.length.saturating_mul(16))?;
    // Each element is read and written back: two units of fuel, charged before the sort starts.
    for _ in 0..view.length.saturating_mul(2) {
        interp.use_fuel()?;
    }
    let mut values: Vec<f64> = (0..view.length).map(|i| at(interp, &view, i)).collect();
    merge_sort(interp, &mut values, &compare)?;
    for (i, n) in values.into_iter().enumerate() {
        write(interp, &view, i, n);
    }
    Ok(this)
}
