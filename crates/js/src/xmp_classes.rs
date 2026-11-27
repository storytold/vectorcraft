//! `ExternalObject` and the XMP classes it loads, as the help pages
//! <https://extendscript.docsforadobe.dev/integrating-external-libraries/externalobject-object/>,
//! <https://extendscript.docsforadobe.dev/scripting-xmp/accessing-the-xmp-scripting-api/> and
//! <https://extendscript.docsforadobe.dev/scripting-xmp/xmpscript-object-reference/> describe them,
//! as built-ins ([`crate::Realm::define_external_object`]). The metadata itself is
//! [`crate::xmp`].
//!
//! `new ExternalObject("lib:AdobeXMPScript")` defines the globals `XMPMeta`, `XMPFile` and
//! `XMPConst` and gives a handle whose `terminate()` releases it; the globals and the objects made
//! with them stay. Any other library name throws an `Error`, and no native code is loaded.
//! `ExternalObject.search(name)` is true for that name alone.
//!
//! `XMPMeta` holds its metadata in its object. Its namespace prefixes are the realm's: those a
//! packet declares are registered when it is read, and `XMPMeta.registerNamespace` adds others.
//! `getNamespacePrefix` and `registerNamespace` give a prefix with its colon. Property paths are a
//! name, `prefix:name`, and either with `[n]` (from 1) or `[last()]` for an array item. An array
//! property holds simple items; any other value a packet holds is kept as it was written and
//! `getProperty` gives its text, when it has one, and its options.
//! `insertArrayItem` inserts before the last existing item when its index is `ARRAY_LAST_ITEM`.
//! `insertArrayItem` and `setArrayItem` store `null` as an empty item value.
//! Metadata edits charge fuel for size accounting.
//!
//! `XMPFile` reads a file through the host functions `xmpOpen` and `xmpWrite` of `File` and
//! `Folder` ([`crate::Realm::define_file_classes`]), so it reads and writes only where they may. A
//! file named `.xmp` is read whole as a packet, and any other file, or any file with
//! `OPEN_USE_PACKET_SCANNING`, is scanned for its first packet ([`crate::xmp::find_packet`]).
//! `putXMP` keeps the packet and `closeFile` writes it.
//!
//! VectorCraft's own rules: only a `.xmp` file opened with `OPEN_FOR_UPDATE` takes `putXMP`;
//! `closeFile` always replaces the file through a temporary one when the folder may hold one, and
//! with `CLOSE_UPDATE_SAFELY` it writes no other way, throwing and keeping the packet instead; an
//! `XMPFile` is open only in the file run that opened it. Closing it successfully or beginning
//! another file run with `Realm::reset_folder_current` releases its packet buffers. A member the
//! reference lists that is not built yet, and a flag, format or option not handled yet, throws an
//! error that says so.

use crate::builtins::arg;
use crate::heap::{Class, ENUMERABLE, HIDDEN, NativeKind, OBJECT_BYTES, ObjId, Object, PROP_BYTES, Prop, PropMap, key, string_bytes};
use crate::interp::{Abrupt, ErrorType, Interp, JsResult};
use crate::value::Value;
use crate::xmp::{self, Form, Item, Meta, Registry, Step, XmpError};

/// The one library `ExternalObject` loads.
const LIBRARY: &str = "lib:AdobeXMPScript"; // brand-ok: the library name scripts load

const FILE_UNKNOWN: u32 = 0x2020_2020;
const FILE_TEXT: u32 = 0x7465_7874;
const FILE_XML: u32 = 0x584D_4C20;
const OPEN_FOR_READ: u32 = 0x1;
const OPEN_FOR_UPDATE: u32 = 0x2;
const OPEN_ONLY_XMP: u32 = 0x4;
const OPEN_USE_PACKET_SCANNING: u32 = 0x40;
const CLOSE_UPDATE_SAFELY: u32 = 0x1;
const SERIALIZE_OMIT_PACKET_WRAPPER: u32 = 0x10;

const LIBRARY_SLOT: &str = "__vcLibrary";
const PATH: &str = "__vcXmpPath";
const FORMAT: &str = "__vcXmpFormat";
const FLAGS: &str = "__vcXmpFlags";
const RUN: &str = "__vcXmpRun";
const OPEN: &str = "__vcXmpOpen";
const PACKET: &str = "__vcXmpPacket";
const OFFSET: &str = "__vcXmpOffset";
const LENGTH: &str = "__vcXmpLength";
const WRITEABLE: &str = "__vcXmpWriteable";
const SIDECAR: &str = "__vcXmpSidecar";
const CHAR_FORM: &str = "__vcXmpCharForm";
const PENDING: &str = "__vcXmpPending";

/// The objects of `ExternalObject` and the XMP classes, kept by the realm, and its namespaces.
#[derive(Debug)]
pub(crate) struct Classes {
    ids: Ids,
    registry: Registry,
}

#[derive(Clone, Copy, Debug)]
struct Ids {
    external_proto: ObjId,
    meta: ObjId,
    meta_proto: ObjId,
    file: ObjId,
    file_proto: ObjId,
    constants: ObjId,
    property_proto: ObjId,
}

impl Classes {
    pub(crate) fn bytes(&self) -> usize {
        self.registry.bytes()
    }

    /// The objects a collection keeps.
    pub(crate) fn objects(&self) -> [ObjId; 7] {
        let i = self.ids;
        [i.external_proto, i.meta, i.meta_proto, i.file, i.file_proto, i.constants, i.property_proto]
    }
}

fn ids(i: &Interp) -> JsResult<Ids> {
    i.xmp.as_ref().map(|c| c.ids).ok_or_else(|| Abrupt::Internal("ExternalObject is not installed".into()))
}

fn registry(i: &mut Interp) -> JsResult<&mut Registry> {
    i.xmp.as_mut().map(|c| &mut c.registry).ok_or_else(|| Abrupt::Internal("ExternalObject is not installed".into()))
}

// ---- errors ----

fn not_yet(i: &mut Interp, what: &str) -> Abrupt {
    i.throw(ErrorType::Error, &format!("{what} is not available yet"))
}

fn not_handled(i: &mut Interp, what: &str, value: &str) -> Abrupt {
    let value = shown(value);
    i.throw(ErrorType::Range, &format!("{what}: the value {value} is not available yet"))
}

fn wrong_object(i: &mut Interp, what: &str, class: &str) -> Abrupt {
    i.type_error(&format!("{what}: the object is not an {class} object"))
}

/// The first 200 characters of `s`, the most of a script's text an error message holds.
fn shown(s: &str) -> String {
    s.chars().take(200).collect()
}

/// The exception for `e`, which `what` met with the name or path `name`.
fn xmp_error(i: &mut Interp, what: &str, e: XmpError, name: &str) -> Abrupt {
    let name = shown(name);
    match e {
        XmpError::Malformed => i.throw(ErrorType::Error, &format!("{what}: the XMP packet is not well-formed XMP")),
        XmpError::TooLarge | XmpError::Namespaces => i.throw(ErrorType::Range, &format!("{what}: the XMP data is too large")),
        XmpError::BadName => i.throw(ErrorType::Error, &format!("{what}: {name} is not a valid property name")),
        XmpError::BadText => i.throw(ErrorType::Error, &format!("{what}: the text holds a character XML does not allow")),
        XmpError::NotYet => not_handled(i, what, &name),
        XmpError::Heap => Abrupt::Limit(crate::LimitKind::Heap),
    }
}

fn not_array(i: &mut Interp, what: &str, name: &str) -> Abrupt {
    let name = shown(name);
    i.throw(ErrorType::Error, &format!("{what}: {name} is not an array"))
}

fn not_simple(i: &mut Interp, what: &str, name: &str) -> Abrupt {
    let name = shown(name);
    i.throw(ErrorType::Error, &format!("{what}: {name} is not a simple property"))
}

fn not_alt_text(i: &mut Interp, what: &str, name: &str) -> Abrupt {
    let name = shown(name);
    i.throw(ErrorType::Error, &format!("{what}: {name} is not a language alternative array"))
}

fn out_of_range(i: &mut Interp, what: &str, index: i64) -> Abrupt {
    i.throw(ErrorType::Range, &format!("{what}: the array index {index} is out of range"))
}

fn unregistered(i: &mut Interp, what: &str, uri: &str) -> Abrupt {
    let uri = shown(uri);
    i.throw(ErrorType::Error, &format!("{what}: the namespace {uri} is not registered"))
}

// ---- small helpers ----

fn text(i: &mut Interp, v: &Value) -> JsResult<String> {
    Ok(i.to_str(v)?.to_rust_string())
}

/// A string argument's value as plain text, refused when it holds a character XML does not allow.
fn xml_text(i: &mut Interp, v: &Value, what: &str) -> JsResult<String> {
    let s = text(i, v)?;
    if !xmp::is_text(&s) {
        return Err(xmp_error(i, what, XmpError::BadText, ""));
    }
    Ok(s)
}

/// A flags argument: `undefined` and `null` are 0, any other value a whole number from 0 to
/// 2^32 - 1, refused with a `RangeError` otherwise.
fn flags(i: &mut Interp, v: &Value, what: &str) -> JsResult<u32> {
    if matches!(v, Value::Undefined | Value::Null) {
        return Ok(0);
    }
    let n = i.to_number(v)?;
    if n.fract() != 0.0 || !(0.0..=f64::from(u32::MAX)).contains(&n) {
        let shown = crate::number::to_string(n);
        return Err(not_handled(i, what, &shown));
    }
    Ok(n as u32)
}

/// Refuses the bits of `value` outside `allowed`, naming the first.
fn only(i: &mut Interp, value: u32, allowed: u32, what: &str) -> JsResult<()> {
    let extra = value & !allowed;
    if extra != 0 {
        let bit = 1u32 << extra.trailing_zeros();
        return Err(not_handled(i, what, &format!("0x{bit:X}")));
    }
    Ok(())
}

/// Takes fuel for reading or writing `bytes` bytes of XMP.
fn spend(i: &mut Interp, bytes: usize) -> JsResult<()> {
    crate::regexp_vm::use_fuel_n(i, 1 + bytes as u64 / 256)
}

fn slot(i: &Interp, o: ObjId, name: &str) -> JsResult<Value> {
    if let Some(Class::XmpFile(fields)) = i.heap.object(o).map(|object| &object.class) {
        return Ok(fields.get(&key(name)).cloned().unwrap_or(Value::Undefined));
    }
    Ok(match i.get_own_property(o, &key(name))? {
        Some(Prop::Data { value, .. }) => value,
        _ => Value::Undefined,
    })
}

fn set_slot(i: &mut Interp, o: ObjId, name: &str, value: Value) -> JsResult<()> {
    set_slot_with_charge(i, o, name, value, true)
}

fn set_slot_with_charge(i: &mut Interp, o: ObjId, name: &str, value: Value, charged: bool) -> JsResult<()> {
    if let Some(Class::XmpFile(fields)) = i.heap.object(o).map(|object| &object.class) {
        let name = key(name);
        let added = if fields.contains(&name) { 0 } else { PROP_BYTES + string_bytes(name.len()) };
        let strings = if charged {
            0
        } else if let Value::String(s) = &value {
            string_bytes(s.len())
        } else {
            0
        };
        return i.rooted(&[Value::Object(o), value.clone()], |i| {
            i.reserve_heap(added + strings)?;
            if let Some(Class::XmpFile(fields)) = i.heap.object_mut(o).map(|object| &mut object.class) {
                fields.insert(name, value);
                i.heap.bytes = i.heap.bytes.saturating_add(added);
            }
            Ok(())
        });
    }
    if let Value::String(s) = &value {
        i.rooted(std::slice::from_ref(&value), |i| i.reserve_heap(PROP_BYTES + string_bytes(s.len())))?;
    }
    i.define(o, key(name), Prop::Data { value, flags: 0 })
}

pub(crate) fn reset_file_state(fields: &mut PropMap<Value>) {
    if let Some(packet) = fields.get_mut(&key(PACKET)) {
        *packet = Value::Null;
    }
    if let Some(pending) = fields.get_mut(&key(PENDING)) {
        *pending = Value::Undefined;
    }
}

/// A string value, charged to the heap.
fn string(i: &mut Interp, s: &str) -> JsResult<Value> {
    let bytes = string_bytes(s.encode_utf16().count());
    i.reserve_heap(bytes)?;
    i.heap.bytes = i.heap.bytes.saturating_add(bytes);
    Ok(Value::str(s))
}

/// A new ordinary object with the prototype `proto` and the read-only data properties `fields`.
fn record(i: &mut Interp, proto: Option<ObjId>, fields: Vec<(&'static str, Value)>) -> JsResult<Value> {
    i.rooted(&fields.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>(), |i| {
        let keys: usize = fields.iter().map(|(name, _)| string_bytes(name.encode_utf16().count())).sum();
        i.reserve_heap(OBJECT_BYTES + PROP_BYTES * fields.len() + keys)?;
        let o = i.heap.alloc_object(Object::new(Class::Ordinary, proto.or(Some(i.intrinsics.object_proto))));
        for (name, value) in &fields {
            i.define(o, key(name), Prop::Data { value: value.clone(), flags: ENUMERABLE })?;
        }
        Ok(Value::Object(o))
    })
}

// ---- ExternalObject ----

fn external_ctor(i: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let name = text(i, &arg(args, 0))?;
    if name != LIBRARY {
        let name = shown(&name);
        let what = "ExternalObject";
        return Err(i.throw(ErrorType::Error, &format!("{what}: the library {name} is not available")));
    }
    let ids = ids(i)?;
    let global = i.global;
    i.reserve_heap(OBJECT_BYTES + PROP_BYTES)?;
    let o = i.heap.alloc_object(Object::new(Class::Ordinary, Some(ids.external_proto)));
    i.rooted(&[Value::Object(o)], |i| {
        let library = string(i, LIBRARY)?;
        data(i, o, LIBRARY_SLOT, library, 0)?;
        let members = [("XMPMeta", ids.meta), ("XMPFile", ids.file), ("XMPConst", ids.constants)];
        let mut added = 0;
        for (name, _) in members {
            if i.get_own_property(global, &key(name))?.is_none() {
                added += PROP_BYTES + string_bytes(name.len());
            }
        }
        i.reserve_heap(added)?;
        for (name, value) in members {
            crate::builtins::data(i, global, name, Value::Object(value), HIDDEN)?;
        }
        Ok(())
    })?;
    Ok(Value::Object(o))
}

fn external_search(i: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    Ok(Value::Bool(text(i, &arg(args, 0))? == LIBRARY))
}

fn external_terminate(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let handle = this.as_object().filter(|&o| matches!(slot(i, o, LIBRARY_SLOT), Ok(Value::String(_))));
    let Some(o) = handle else { return Err(wrong_object(i, "ExternalObject.terminate", "ExternalObject")) };
    set_slot(i, o, LIBRARY_SLOT, Value::Bool(false))?;
    Ok(Value::Undefined)
}

// ---- XMPMeta ----

fn is_meta(i: &Interp, v: &Value) -> Option<ObjId> {
    v.as_object().filter(|&o| matches!(i.heap.object(o).map(|ob| &ob.class), Some(Class::Xmp(_))))
}

fn this_meta(i: &mut Interp, this: &Value, what: &str) -> JsResult<ObjId> {
    match is_meta(i, this) {
        Some(o) => Ok(o),
        None => Err(wrong_object(i, what, "XMPMeta")),
    }
}

fn meta_of(i: &Interp, o: ObjId) -> JsResult<&Meta> {
    match i.heap.object(o).map(|ob| &ob.class) {
        Some(Class::Xmp(m)) => Ok(m),
        _ => Err(Abrupt::Internal("an XMPMeta lost its metadata".into())),
    }
}

/// Changes the metadata of `o` with `f`, given `grow`, the most bytes the change adds, and charges
/// the heap for what it added or frees what it took out.
fn change<T>(i: &mut Interp, o: ObjId, grow: usize, f: impl FnOnce(&mut Meta, &Registry) -> T) -> JsResult<T> {
    let accounting = meta_of(i, o)?.heap_bytes().saturating_mul(3).saturating_add(grow);
    spend(i, accounting)?;
    change_counted(i, o, grow, None, f)
}

fn change_counted<T>(i: &mut Interp, o: ObjId, grow: usize, delta: Option<(usize, usize)>, f: impl FnOnce(&mut Meta, &Registry) -> T) -> JsResult<T> {
    i.reserve_heap(grow)?;
    let Some(c) = i.xmp.as_ref() else { return Err(Abrupt::Internal("ExternalObject is not installed".into())) };
    let Some(Class::Xmp(m)) = i.heap.object_mut(o).map(|ob| &mut ob.class) else {
        return Err(Abrupt::Internal("an XMPMeta lost its metadata".into()));
    };
    let before = delta.map_or_else(|| m.heap_bytes(), |(removed, _)| removed);
    let r = f(m, &c.registry);
    let after = delta.map_or_else(|| m.heap_bytes(), |(_, added)| added);
    i.heap.bytes = if after >= before { i.heap.bytes.saturating_add(after - before) } else { i.heap.bytes.saturating_sub(before - after) };
    Ok(r)
}

/// A new `XMPMeta` holding `meta`.
fn new_meta(i: &mut Interp, meta: Meta) -> JsResult<Value> {
    let ids = ids(i)?;
    let bytes = meta.heap_bytes();
    i.reserve_heap(OBJECT_BYTES.saturating_add(bytes))?;
    let o = i.heap.alloc_object(Object::new(Class::Xmp(Box::new(meta)), Some(ids.meta_proto)));
    i.heap.bytes = i.heap.bytes.saturating_add(bytes);
    Ok(Value::Object(o))
}

/// The metadata of the packet `text`, read for `what`.
fn parse_packet(i: &mut Interp, what: &str, text: &str) -> JsResult<Meta> {
    let mut staged = registry(i)?.clone();
    let copied = staged.bytes();
    spend(i, text.len().saturating_add(2 * copied))?;
    i.collect_garbage();
    let budget = i.limits.heap_bytes.saturating_sub(i.heap.bytes).saturating_sub(OBJECT_BYTES);
    let meta = xmp::parse_within(text, &mut staged, budget).map_err(|e| xmp_error(i, what, e, ""))?;
    let growth = staged.bytes().saturating_sub(copied);
    i.reserve_heap(OBJECT_BYTES.saturating_add(meta.heap_bytes()).saturating_add(growth))?;
    *registry(i)? = staged;
    i.heap.bytes = i.heap.bytes.saturating_add(growth);
    Ok(meta)
}

fn meta_ctor(i: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let meta = match arg(args, 0) {
        Value::Undefined => Meta::default(),
        Value::Object(o) if matches!(i.heap.object(o).map(|ob| &ob.class), Some(Class::Array { .. })) => {
            return Err(not_yet(i, "new XMPMeta(buffer)"));
        }
        v => {
            let packet = text(i, &v)?;
            parse_packet(i, "XMPMeta", &packet)?
        }
    };
    new_meta(i, meta)
}

/// The namespace argument `v`, refused when it is not registered.
fn namespace(i: &mut Interp, v: &Value, what: &str) -> JsResult<String> {
    let uri = text(i, v)?;
    if registry(i)?.prefix_of(&uri).is_none() {
        return Err(unregistered(i, what, &uri));
    }
    Ok(uri)
}

/// The namespace and path arguments at `at`, each converted once.
struct Target {
    ns: String,
    /// The property name and its step.
    name: String,
    step: Step,
    /// The path as given.
    path: String,
}

/// The namespace and path arguments at `at`.
fn target(i: &mut Interp, args: &[Value], at: usize, what: &str) -> JsResult<Target> {
    let ns = namespace(i, &arg(args, at), what)?;
    let path = text(i, &arg(args, at + 1))?;
    // A packet holds no property in the RDF or XML namespace.
    if ns == xmp::RDF || ns == xmp::XML {
        return Err(xmp_error(i, what, XmpError::BadName, &path));
    }
    let r = xmp::parse_path(&path, &ns, registry(i)?);
    match r {
        Ok((name, step)) => Ok(Target { ns, name, step, path }),
        Err(e) => Err(xmp_error(i, what, e, &path)),
    }
}

/// The index an item position names in an array of `len` items: -1 is the last.
fn position(index: i64, len: usize) -> Option<usize> {
    let i = if index == -1 { len } else { usize::try_from(index).ok()? };
    (1..=len).contains(&i).then(|| i - 1)
}

/// An item position argument: a whole number from 1, or -1 for the last.
fn item_index(i: &mut Interp, v: &Value, what: &str) -> JsResult<i64> {
    let n = i.to_number(v)?;
    if n.fract() != 0.0 || !(n == -1.0 || (1.0..=1e15).contains(&n)) {
        return Err(out_of_range(i, what, if n.is_finite() { n as i64 } else { 0 }));
    }
    Ok(n as i64)
}

/// An `XMPProperty` for `path` in `ns`.
fn property(i: &mut Interp, ns: &str, path: &str, value: Option<&str>, options: u32, locale: &str) -> JsResult<Value> {
    let proto = ids(i)?.property_proto;
    let ns = string(i, ns)?;
    let path = i.rooted(std::slice::from_ref(&ns), |i| string(i, path))?;
    let value = match value {
        Some(v) => i.rooted(&[ns.clone(), path.clone()], |i| string(i, v))?,
        None => Value::Undefined,
    };
    let locale = i.rooted(&[ns.clone(), path.clone(), value.clone()], |i| string(i, locale))?;
    let fields = vec![("namespace", ns), ("path", path), ("value", value), ("options", Value::Number(f64::from(options))), ("locale", locale)];
    record(i, Some(proto), fields)
}

fn property_to_string(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let v = match this.as_object() {
        Some(o) => slot(i, o, "value")?,
        None => Value::Undefined,
    };
    if matches!(v, Value::Undefined) { Ok(Value::str("")) } else { Ok(Value::String(i.to_str(&v)?)) }
}

fn get_property(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPMeta.getProperty";
    let o = this_meta(i, &this, what)?;
    let Target { ns, name, step, path } = target(i, args, 0, what)?;
    if !matches!(arg(args, 2), Value::Undefined) {
        let v = text(i, &arg(args, 2))?;
        return Err(not_handled(i, what, &v));
    }
    let found = {
        let m = meta_of(i, o)?;
        match m.find(&ns, &name).map(|at| &m.props[at].value) {
            None => Ok(None),
            Some(value) => match step {
                Step::Whole => Ok(Some((value.text().map(str::to_string), value.options(), String::new()))),
                Step::Item(index) => match value {
                    xmp::Value::Array { items, .. } => Ok(position(index, items.len())
                        .and_then(|p| items.get(p))
                        .map(|item| (Some(item.text.clone()), item.options(), item.lang.clone().unwrap_or_default()))),
                    _ => Err(()),
                },
            },
        }
    };
    let found = found.map_err(|()| not_array(i, what, &name))?;
    let Some((text, options, locale)) = found else { return Ok(Value::Undefined) };
    property(i, &ns, &path, text.as_deref(), options, &locale)
}

fn set_property(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPMeta.setProperty";
    let o = this_meta(i, &this, what)?;
    let Target { ns, name, step, .. } = target(i, args, 0, what)?;
    let options = flags(i, &arg(args, 3), what)?;
    only(i, options, xmp::PROP_IS_ARRAY | xmp::ARRAY_IS_ORDERED | xmp::ARRAY_IS_ALTERNATIVE, what)?;
    if !matches!(arg(args, 4), Value::Undefined) {
        let v = text(i, &arg(args, 4))?;
        return Err(not_handled(i, what, &v));
    }
    let value = arg(args, 2);
    if let Step::Item(index) = step {
        // An array item is a simple value, which takes no options.
        only(i, options, 0, what)?;
        let v = xml_text(i, &value, what)?;
        return set_item(i, o, &ns, &name, index, v, false, what);
    }
    if options != 0 {
        // An array is made empty; one that exists is left as it is.
        if !matches!(value, Value::Undefined | Value::Null) {
            let v = text(i, &value)?;
            return Err(not_handled(i, what, &v));
        }
        let kind = {
            let m = meta_of(i, o)?;
            m.find(&ns, &name).map(|at| matches!(m.props[at].value, xmp::Value::Array { .. }))
        };
        return match kind {
            Some(true) => Ok(Value::Undefined),
            Some(false) => Err(not_array(i, what, &name)),
            None => {
                let prop = xmp::Prop { ns: ns.into(), name, value: xmp::Value::Array { form: Form::from_options(options), items: Vec::new() } };
                change(i, o, prop.heap_bytes(), |m, _| m.props.push(prop)).map(|()| Value::Undefined)
            }
        };
    }
    let v = xml_text(i, &value, what)?;
    let simple = {
        let m = meta_of(i, o)?;
        m.find(&ns, &name).map(|at| (at, matches!(m.props[at].value, xmp::Value::Simple { .. })))
    };
    let new = xmp::Value::Simple { text: v, uri: false };
    match simple {
        Some((_, false)) => Err(not_simple(i, what, &name)),
        Some((at, true)) => {
            let grow = 96 + new.text().map_or(0, str::len);
            change(i, o, grow, |m, _| {
                if let Some(p) = m.props.get_mut(at) {
                    p.value = new;
                }
            })
        }
        None => {
            let prop = xmp::Prop { ns: ns.into(), name, value: new };
            change(i, o, prop.heap_bytes(), |m, _| m.props.push(prop))
        }
    }
    .map(|()| Value::Undefined)
}

fn delete_property(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPMeta.deleteProperty";
    let o = this_meta(i, &this, what)?;
    let Target { ns, name, step, .. } = target(i, args, 0, what)?;
    match step {
        Step::Whole => {
            let at = meta_of(i, o)?.find(&ns, &name);
            if let Some(at) = at {
                change(i, o, 0, |m, _| m.props.remove(at))?;
            }
            Ok(Value::Undefined)
        }
        Step::Item(index) => delete_item(i, o, &ns, &name, index, what, true),
    }
}

fn does_property_exist(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPMeta.doesPropertyExist";
    let o = this_meta(i, &this, what)?;
    let Target { ns, name, step, .. } = target(i, args, 0, what)?;
    let m = meta_of(i, o)?;
    let exists = match (m.find(&ns, &name).map(|at| &m.props[at].value), step) {
        (None, _) => false,
        (Some(_), Step::Whole) => true,
        (Some(xmp::Value::Array { items, .. }), Step::Item(index)) => position(index, items.len()).is_some(),
        (Some(_), Step::Item(_)) => false,
    };
    Ok(Value::Bool(exists))
}

/// The items of the array `name` in `ns` of `o`, `None` when it has no such property, refused when
/// the property is not an array.
fn array_items(i: &mut Interp, o: ObjId, ns: &str, name: &str, what: &str) -> JsResult<Option<(usize, Form, usize)>> {
    let found = {
        let m = meta_of(i, o)?;
        m.find(ns, name).map(|at| match &m.props[at].value {
            xmp::Value::Array { form, items } => Some((at, *form, items.len())),
            _ => None,
        })
    };
    match found {
        None => Ok(None),
        Some(Some(a)) => Ok(Some(a)),
        Some(None) => Err(not_array(i, what, name)),
    }
}

/// The namespace and array name arguments: a whole property.
fn array_target(i: &mut Interp, args: &[Value], what: &str) -> JsResult<Target> {
    let t = target(i, args, 0, what)?;
    if t.step != Step::Whole {
        return Err(not_handled(i, what, &t.path));
    }
    Ok(t)
}

fn count_array_items(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPMeta.countArrayItems";
    let o = this_meta(i, &this, what)?;
    let Target { ns, name, .. } = array_target(i, args, what)?;
    let n = array_items(i, o, &ns, &name, what)?.map_or(0, |(_, _, len)| len);
    Ok(Value::Number(n as f64))
}

fn get_array_item(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPMeta.getArrayItem";
    let o = this_meta(i, &this, what)?;
    let Target { ns, name, path, .. } = array_target(i, args, what)?;
    let index = item_index(i, &arg(args, 2), what)?;
    let Some((at, _, len)) = array_items(i, o, &ns, &name, what)? else { return Ok(Value::Undefined) };
    let Some(p) = position(index, len) else { return Ok(Value::Undefined) };
    let item = match &meta_of(i, o)?.props[at].value {
        xmp::Value::Array { items, .. } => items.get(p).cloned(),
        _ => None,
    };
    let Some(item) = item else { return Ok(Value::Undefined) };
    let path = format!("{path}[{}]", p + 1);
    property(i, &ns, &path, Some(&item.text), item.options(), item.lang.as_deref().unwrap_or(""))
}

fn does_array_item_exist(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPMeta.doesArrayItemExist";
    let o = this_meta(i, &this, what)?;
    let Target { ns, name, .. } = array_target(i, args, what)?;
    let index = item_index(i, &arg(args, 2), what)?;
    let len = array_items(i, o, &ns, &name, what)?.map_or(0, |(_, _, len)| len);
    Ok(Value::Bool(position(index, len).is_some()))
}

/// The item options argument: 0 alone, for a simple item.
fn item_options(i: &mut Interp, v: &Value, what: &str) -> JsResult<()> {
    let options = flags(i, v, what)?;
    only(i, options, 0, what)
}

/// Adds `value` to the array `name` in `ns` of `o` at the position `index` (from 1; one past the
/// last appends), making the array in `form` when it has none.
#[allow(clippy::too_many_arguments)]
fn insert_item(i: &mut Interp, o: ObjId, ns: &str, name: &str, index: Option<i64>, value: String, form: Option<Form>, what: &str) -> JsResult<Value> {
    let found = array_items(i, o, ns, name, what)?;
    let len = found.map_or(0, |(_, _, len)| len);
    let p = match index {
        None => len,
        Some(-1) => match len.checked_sub(1) {
            Some(p) => p,
            None => return Err(out_of_range(i, what, -1)),
        },
        Some(n) => match usize::try_from(n) {
            Ok(n) if (1..=len + 1).contains(&n) => n - 1,
            _ => return Err(out_of_range(i, what, n)),
        },
    };
    let grow = 48 + value.len();
    spend(i, len.saturating_sub(p).saturating_mul(std::mem::size_of::<Item>()))?;
    match found {
        Some((at, _, _)) => change_counted(i, o, grow, Some((0, grow)), |m, _| {
            if let Some(xmp::Value::Array { items, .. }) = m.props.get_mut(at).map(|pr| &mut pr.value) {
                items.insert(p.min(items.len()), Item { text: value, lang: None });
            }
        })?,
        None => {
            let Some(form) = form else { return Err(not_array(i, what, name)) };
            let prop =
                xmp::Prop { ns: ns.into(), name: name.to_string(), value: xmp::Value::Array { form, items: vec![Item { text: value, lang: None }] } };
            change(i, o, prop.heap_bytes(), |m, _| m.props.push(prop))?;
        }
    }
    Ok(Value::Undefined)
}

/// Replaces the item at `index` of the array `name` in `ns` of `o` with `value`, or appends it at
/// one past the last when `append` allows.
#[allow(clippy::too_many_arguments)]
fn set_item(i: &mut Interp, o: ObjId, ns: &str, name: &str, index: i64, value: String, append: bool, what: &str) -> JsResult<Value> {
    let Some((at, _, len)) = array_items(i, o, ns, name, what)? else { return Err(not_array(i, what, name)) };
    let p = match position(index, len) {
        Some(p) => p,
        None if append && usize::try_from(index).is_ok_and(|n| n == len + 1) => {
            return insert_item(i, o, ns, name, Some(index), value, None, what);
        }
        None => return Err(out_of_range(i, what, index)),
    };
    let grow = 48 + value.len();
    change(i, o, grow, |m, _| {
        if let Some(xmp::Value::Array { items, .. }) = m.props.get_mut(at).map(|pr| &mut pr.value)
            && let Some(item) = items.get_mut(p)
        {
            item.text = value;
        }
    })?;
    Ok(Value::Undefined)
}

/// Deletes the item at `index` of the array `name` in `ns` of `o`; a missing item or array is left
/// as it is when `quiet`.
fn delete_item(i: &mut Interp, o: ObjId, ns: &str, name: &str, index: i64, what: &str, quiet: bool) -> JsResult<Value> {
    let found = array_items(i, o, ns, name, what)?;
    let target = found.and_then(|(at, _, len)| position(index, len).map(|p| (at, p)));
    match target {
        Some((at, p)) => {
            change(i, o, 0, |m, _| {
                if let Some(xmp::Value::Array { items, .. }) = m.props.get_mut(at).map(|pr| &mut pr.value)
                    && p < items.len()
                {
                    items.remove(p);
                }
            })?;
            Ok(Value::Undefined)
        }
        None if quiet => Ok(Value::Undefined),
        None => Err(out_of_range(i, what, index)),
    }
}

fn append_array_item(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPMeta.appendArrayItem";
    let o = this_meta(i, &this, what)?;
    let Target { ns, name, .. } = array_target(i, args, what)?;
    // `itemOptions` may be left out: a string or `null` third argument is the item's value.
    let (value, array_options) = match arg(args, 2) {
        Value::Number(_) => {
            item_options(i, &arg(args, 2), what)?;
            (arg(args, 3), arg(args, 4))
        }
        v => (v, arg(args, 3)),
    };
    let array_options = flags(i, &array_options, what)?;
    only(i, array_options, xmp::PROP_IS_ARRAY | xmp::ARRAY_IS_ORDERED | xmp::ARRAY_IS_ALTERNATIVE, what)?;
    let value = if matches!(value, Value::Null | Value::Undefined) { String::new() } else { xml_text(i, &value, what)? };
    let form = (array_options != 0).then(|| Form::from_options(array_options));
    insert_item(i, o, &ns, &name, None, value, form, what)
}

fn insert_array_item(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPMeta.insertArrayItem";
    let o = this_meta(i, &this, what)?;
    let Target { ns, name, .. } = array_target(i, args, what)?;
    let index = item_index(i, &arg(args, 2), what)?;
    let value = arg(args, 3);
    let value = if matches!(value, Value::Null | Value::Undefined) { String::new() } else { xml_text(i, &value, what)? };
    item_options(i, &arg(args, 4), what)?;
    insert_item(i, o, &ns, &name, Some(index), value, None, what)
}

fn set_array_item(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPMeta.setArrayItem";
    let o = this_meta(i, &this, what)?;
    let Target { ns, name, .. } = array_target(i, args, what)?;
    let index = item_index(i, &arg(args, 2), what)?;
    let value = arg(args, 3);
    let value = if matches!(value, Value::Null | Value::Undefined) { String::new() } else { xml_text(i, &value, what)? };
    item_options(i, &arg(args, 4), what)?;
    set_item(i, o, &ns, &name, index, value, true, what)
}

fn delete_array_item(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPMeta.deleteArrayItem";
    let o = this_meta(i, &this, what)?;
    let Target { ns, name, .. } = array_target(i, args, what)?;
    let index = item_index(i, &arg(args, 2), what)?;
    delete_item(i, o, &ns, &name, index, what, false)
}

/// Does language tag `lang` match `want`, ignoring case?
fn same_lang(lang: Option<&str>, want: &str) -> bool {
    lang.is_some_and(|l| l.eq_ignore_ascii_case(want))
}

/// Is language tag `lang` the language `generic`, or one of its variants (`generic-…`), ignoring
/// case? Never for an empty `generic`.
fn generic_lang(lang: Option<&str>, generic: &str) -> bool {
    !generic.is_empty()
        && lang.is_some_and(|l| {
            l.eq_ignore_ascii_case(generic)
                || l.len() > generic.len()
                    && l.get(..generic.len()).is_some_and(|p| p.eq_ignore_ascii_case(generic))
                    && l.as_bytes().get(generic.len()) == Some(&b'-')
        })
}

/// The item of `items` for `specific`, else one whose language starts with `generic`, else the
/// `x-default` item, else the first.
fn pick_lang<'a>(items: &'a [Item], generic: &str, specific: &str) -> Option<&'a Item> {
    items
        .iter()
        .find(|i| same_lang(i.lang.as_deref(), specific))
        .or_else(|| items.iter().find(|i| generic_lang(i.lang.as_deref(), generic)))
        .or_else(|| items.iter().find(|i| same_lang(i.lang.as_deref(), "x-default")))
        .or_else(|| items.first())
}

/// A language argument: `null` and `undefined` are empty.
fn lang(i: &mut Interp, v: &Value, what: &str) -> JsResult<String> {
    if matches!(v, Value::Null | Value::Undefined) { Ok(String::new()) } else { xml_text(i, v, what) }
}

/// Is the property at `at` of `o` a language alternative: an `rdf:Alt` whose items, if it has any,
/// all have a language? An item with `xml:lang=""` has none.
fn is_alt_text(i: &Interp, o: ObjId, at: usize) -> JsResult<bool> {
    Ok(match meta_of(i, o)?.props.get(at).map(|p| &p.value) {
        Some(xmp::Value::Array { form: Form::Alt, items }) => items.iter().all(Item::has_lang),
        _ => false,
    })
}

fn get_localized_text(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPMeta.getLocalizedText";
    let o = this_meta(i, &this, what)?;
    let Target { ns, name, .. } = array_target(i, args, what)?;
    let generic = lang(i, &arg(args, 2), what)?;
    let specific = lang(i, &arg(args, 3), what)?;
    let Some((at, _, _)) = array_items(i, o, &ns, &name, what)? else { return Ok(Value::Undefined) };
    if !is_alt_text(i, o, at)? {
        return Err(not_alt_text(i, what, &name));
    }
    let found = match &meta_of(i, o)?.props[at].value {
        xmp::Value::Array { items, .. } => pick_lang(items, &generic, &specific).map(|i| i.text.clone()),
        _ => None,
    };
    match found {
        Some(t) => string(i, &t),
        None => Ok(Value::Undefined),
    }
}

fn set_localized_text(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPMeta.setLocalizedText";
    let o = this_meta(i, &this, what)?;
    let Target { ns, name, .. } = array_target(i, args, what)?;
    let generic = lang(i, &arg(args, 2), what)?;
    let specific = lang(i, &arg(args, 3), what)?;
    if specific.is_empty() || specific.contains(char::is_whitespace) {
        return Err(not_handled(i, what, &specific));
    }
    let value = xml_text(i, &arg(args, 4), what)?;
    let found = array_items(i, o, &ns, &name, what)?;
    // A new array is added in the same change that sets its item, so a refused change adds nothing.
    let (at, new) = match found {
        Some((at, _, _)) if !is_alt_text(i, o, at)? => return Err(not_alt_text(i, what, &name)),
        Some((at, _, _)) => (Some(at), None),
        None => (None, Some(xmp::Prop { ns: ns.into(), name, value: xmp::Value::Array { form: Form::Alt, items: Vec::new() } })),
    };
    // An item and the `x-default` item change, or every item with the text of the `x-default` one.
    let changed = match at.and_then(|at| meta_of(i, o).ok()?.props.get(at)).map(|p| &p.value) {
        Some(xmp::Value::Array { items, .. }) if specific.eq_ignore_ascii_case("x-default") => {
            let old = items.iter().find(|i| same_lang(i.lang.as_deref(), "x-default")).map(|i| &i.text);
            items.iter().filter(|i| Some(&i.text) == old).count().max(2)
        }
        _ => 2,
    };
    let grow = changed.saturating_mul(64 + value.len() + specific.len()).saturating_add(new.as_ref().map_or(0, xmp::Prop::heap_bytes));
    change(i, o, grow, |m, _| {
        let at = match new {
            Some(prop) => {
                m.props.push(prop);
                m.props.len() - 1
            }
            None => at.unwrap_or(usize::MAX),
        };
        let Some(xmp::Value::Array { items, .. }) = m.props.get_mut(at).map(|p| &mut p.value) else { return };
        set_lang(items, &generic, &specific, value);
    })?;
    Ok(Value::Undefined)
}

/// Sets the item of `items` in `specific` to `value`, or, when there is none, the one item in the
/// language `generic` or a variant of it. VectorCraft's own rule: the `x-default` item is first; it
/// is added with the value when there is none, and follows a change to an item that had the same
/// text; a change to the `x-default` item also changes the items that had its text.
fn set_lang(items: &mut Vec<Item>, generic: &str, specific: &str, value: String) {
    // An `x-default` item a packet gave later moves to the front; the others keep their order.
    if let Some(d) = items.iter().position(|i| same_lang(i.lang.as_deref(), "x-default"))
        && d > 0
    {
        let item = items.remove(d);
        items.insert(0, item);
    }
    let default = items.iter().position(|i| same_lang(i.lang.as_deref(), "x-default"));
    if specific.eq_ignore_ascii_case("x-default") {
        match default.and_then(|d| items.get(d)).map(|i| i.text.clone()) {
            Some(old) => {
                for item in items.iter_mut().filter(|i| i.text == old) {
                    item.text.clone_from(&value);
                }
            }
            None => items.insert(0, Item { text: value, lang: Some("x-default".into()) }),
        }
        return;
    }
    let old = items.iter().position(|i| same_lang(i.lang.as_deref(), specific)).or_else(|| {
        let mut generic_items =
            items.iter().enumerate().filter(|(_, i)| !same_lang(i.lang.as_deref(), "x-default") && generic_lang(i.lang.as_deref(), generic));
        match (generic_items.next(), generic_items.next()) {
            (Some((at, _)), None) => Some(at),
            _ => None,
        }
    });
    let follows = match (old, default) {
        (Some(o), Some(d)) => items.get(o).map(|i| &i.text) == items.get(d).map(|i| &i.text),
        (None, Some(_)) => false,
        (_, None) => true,
    };
    match old {
        Some(o) => {
            if let Some(item) = items.get_mut(o) {
                item.text.clone_from(&value);
            }
        }
        None => items.push(Item { text: value.clone(), lang: Some(specific.to_string()) }),
    }
    match default {
        Some(d) if follows => {
            if let Some(item) = items.get_mut(d) {
                item.text = value;
            }
        }
        None => items.insert(0, Item { text: value, lang: Some("x-default".into()) }),
        Some(_) => {}
    }
}

/// The packet `o` serializes to with the serialize options argument `v`.
fn serialized(i: &mut Interp, o: ObjId, v: &Value, what: &str) -> JsResult<String> {
    let options = flags(i, v, what)?;
    only(i, options, SERIALIZE_OMIT_PACKET_WRAPPER, what)?;
    let work = {
        let Some(classes) = i.xmp.as_ref() else { return Err(Abrupt::Internal("ExternalObject is not installed".into())) };
        let meta = meta_of(i, o)?;
        let prefixes = meta
            .props
            .iter()
            .filter(|property| !matches!(property.value, xmp::Value::Other { .. }))
            .fold(0usize, |bytes, property| bytes.saturating_add(classes.registry.prefix_of(&property.ns).map_or(0, str::len)));
        meta.heap_bytes().saturating_mul(6).saturating_add(prefixes.saturating_mul(3))
    };
    spend(i, work)?;
    let r = {
        let Some(c) = i.xmp.as_ref() else { return Err(Abrupt::Internal("ExternalObject is not installed".into())) };
        xmp::serialize(meta_of(i, o)?, &c.registry, options & SERIALIZE_OMIT_PACKET_WRAPPER == 0)
    };
    let s = r.map_err(|e| xmp_error(i, what, e, ""))?;
    Ok(s)
}

fn serialize(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPMeta.serialize";
    let o = this_meta(i, &this, what)?;
    // Padding, indent, newline and base indent are not handled yet.
    for (n, a) in args.iter().enumerate().skip(1) {
        if !matches!(a, Value::Undefined) {
            let name = ["", "padding", "indent", "newline", "baseIndent"].get(n).copied().unwrap_or("argument");
            return Err(not_yet(i, &format!("XMPMeta.serialize {name}")));
        }
    }
    let s = serialized(i, o, &arg(args, 0), what)?;
    string(i, &s)
}

fn register_namespace(i: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPMeta.registerNamespace";
    let uri = text(i, &arg(args, 0))?;
    let prefix = text(i, &arg(args, 1))?;
    let (proposed, growth) = registry(i)?.registration(&uri, &prefix).map_err(|e| xmp_error(i, what, e, &prefix))?;
    i.reserve_heap(growth.saturating_add(string_bytes(proposed.len() + 1)))?;
    let r = registry(i)?.register(&uri, &prefix);
    match r {
        Ok(p) => {
            i.heap.bytes = i.heap.bytes.saturating_add(growth);
            string(i, &format!("{p}:"))
        }
        Err(e) => Err(xmp_error(i, what, e, &prefix)),
    }
}

fn get_namespace_prefix(i: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let uri = text(i, &arg(args, 0))?;
    let found = registry(i)?.prefix_of(&uri).map(|p| format!("{p}:"));
    match found {
        Some(p) => string(i, &p),
        None => Ok(Value::Undefined),
    }
}

fn get_namespace_uri(i: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let prefix = text(i, &arg(args, 0))?;
    let prefix = prefix.strip_suffix(':').unwrap_or(&prefix).to_string();
    let found = registry(i)?.uri_of(&prefix).map(str::to_string);
    match found {
        Some(u) => string(i, &u),
        None => Ok(Value::Undefined),
    }
}

/// Defines functions that throw that they are not available yet.
macro_rules! not_yet_fns {
    ($($f:ident => $name:literal,)*) => {
        $(fn $f(i: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
            Err(not_yet(i, $name))
        })*
    };
}

not_yet_fns! {
    meta_iterator => "XMPMeta.iterator",
    meta_serialize_to_array => "XMPMeta.serializeToArray",
    meta_dump_object => "XMPMeta.dumpObject",
    meta_sort => "XMPMeta.sort",
    meta_get_struct_field => "XMPMeta.getStructField",
    meta_set_struct_field => "XMPMeta.setStructField",
    meta_delete_struct_field => "XMPMeta.deleteStructField",
    meta_does_struct_field_exist => "XMPMeta.doesStructFieldExist",
    meta_get_qualifier => "XMPMeta.getQualifier",
    meta_set_qualifier => "XMPMeta.setQualifier",
    meta_delete_qualifier => "XMPMeta.deleteQualifier",
    meta_does_qualifier_exist => "XMPMeta.doesQualifierExist",
    meta_dump_namespaces => "XMPMeta.dumpNamespaces",
    meta_register_alias => "XMPMeta.registerAlias",
    meta_resolve_alias => "XMPMeta.resolveAlias",
    meta_dump_aliases => "XMPMeta.dumpAliases",
    meta_delete_alias => "XMPMeta.deleteAlias",
    meta_delete_namespace => "XMPMeta.deleteNamespace",
    file_get_format_info => "XMPFile.getFormatInfo",
}

// ---- XMPFile ----

/// The `XMPFile` `this`, refused when the `XMPFile` constructor did not make it.
fn this_file(i: &mut Interp, this: &Value, what: &str) -> JsResult<ObjId> {
    if let Some(o) = this.as_object()
        && matches!(i.heap.object(o).map(|ob| &ob.class), Some(Class::XmpFile(_)))
        && matches!(slot(i, o, PATH)?, Value::String(_))
    {
        return Ok(o);
    }
    Err(wrong_object(i, what, "XMPFile"))
}

/// The `XMPFile` `this` while it is open in this run, refused when it is closed.
fn open_file(i: &mut Interp, this: &Value, what: &str) -> JsResult<ObjId> {
    let o = this_file(i, this, what)?;
    let this_run = matches!(slot(i, o, RUN)?, Value::Number(n) if Some(n) == crate::file_classes::run_count(i));
    if !this_run || !slot(i, o, OPEN)?.truthy() {
        return Err(i.throw(ErrorType::Error, &format!("{what}: the file is closed")));
    }
    Ok(o)
}

fn file_ctor(i: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPFile";
    if i.files.is_none() {
        return Err(not_yet(i, "XMPFile without File and Folder"));
    }
    let format = match arg(args, 1) {
        Value::Undefined | Value::Null => FILE_UNKNOWN,
        v => flags(i, &v, what)?,
    };
    if ![FILE_UNKNOWN, FILE_TEXT, FILE_XML].contains(&format) {
        return Err(not_handled(i, what, &format!("0x{format:08X}")));
    }
    let open_flags = flags(i, &arg(args, 2), what)?;
    // `OPEN_ONLY_XMP` asks for the XMP alone, which is all an `XMPFile` reads.
    only(i, open_flags, OPEN_FOR_READ | OPEN_FOR_UPDATE | OPEN_ONLY_XMP | OPEN_USE_PACKET_SCANNING, what)?;
    let update = open_flags & OPEN_FOR_UPDATE != 0;
    let scan = open_flags & OPEN_USE_PACKET_SCANNING != 0;
    let path = crate::file_classes::platform_path(i, &arg(args, 0))?;
    let r = i.rooted(std::slice::from_ref(&path), |i| {
        crate::file_classes::call_host(i, "xmpOpen", &[path.clone(), Value::Bool(update), Value::Bool(scan)])
    })?;
    let keep = [path.clone(), r.clone()];
    i.rooted(&keep, |i| {
        let error = crate::file_classes::record_field(i, &r, "error")?;
        if let Value::String(_) = error {
            let m = text(i, &error)?;
            return Err(i.throw(ErrorType::Error, &m));
        }
        let field = |i: &mut Interp, name: &str| crate::file_classes::record_field(i, &r, name);
        let sidecar = field(i, "sidecar")?.truthy();
        let reported = if format == FILE_UNKNOWN && sidecar { FILE_XML } else { format };
        let file_proto = ids(i)?.file_proto;
        let state_bytes = std::mem::size_of::<PropMap<Value>>();
        i.reserve_heap(OBJECT_BYTES + state_bytes + 12 * PROP_BYTES)?;
        let o = i.heap.alloc_object(Object::new(Class::XmpFile(Box::default()), Some(file_proto)));
        i.heap.bytes = i.heap.bytes.saturating_add(state_bytes);
        i.rooted(&[Value::Object(o)], |i| {
            let packet = field(i, "packet")?;
            let fields = [
                (PATH, path.clone()),
                (FORMAT, Value::Number(f64::from(reported))),
                (FLAGS, Value::Number(f64::from(open_flags))),
                (RUN, crate::file_classes::run_count(i).map_or(Value::Undefined, Value::Number)),
                (OPEN, Value::Bool(true)),
                (PACKET, if matches!(packet, Value::String(_)) { packet } else { Value::Null }),
                (OFFSET, field(i, "offset")?),
                (LENGTH, field(i, "length")?),
                (WRITEABLE, Value::Bool(field(i, "writeable")?.truthy())),
                (SIDECAR, Value::Bool(sidecar)),
                (CHAR_FORM, field(i, "charForm")?),
                (PENDING, Value::Undefined),
            ];
            for (name, value) in fields {
                set_slot(i, o, name, value)?;
            }
            Ok(Value::Object(o))
        })
    })
}

fn get_xmp(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPFile.getXMP";
    let o = open_file(i, &this, what)?;
    let pending = slot(i, o, PENDING)?;
    let packet = if matches!(pending, Value::String(_)) { pending } else { slot(i, o, PACKET)? };
    if !matches!(packet, Value::String(_)) {
        return Ok(Value::Null);
    }
    let text = text(i, &packet)?;
    let meta = if text.trim_matches([' ', '\t', '\r', '\n']).is_empty() { Meta::default() } else { parse_packet(i, what, &text)? };
    new_meta(i, meta)
}

fn get_packet_info(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPFile.getPacketInfo";
    let o = open_file(i, &this, what)?;
    let packet = slot(i, o, PACKET)?;
    if !matches!(packet, Value::String(_)) {
        return Ok(Value::Null);
    }
    let char_form = slot(i, o, CHAR_FORM)?;
    // The padding is counted in bytes: two for each character of a UTF-16 packet.
    let unit = if matches!(char_form, Value::Number(n) if n == 2.0 || n == 3.0) { 2 } else { 1 };
    let packet_text = text(i, &packet)?;
    spend(i, packet_text.len())?;
    let pad = crate::xmp::pad_size(&packet_text) * unit;
    let fields = vec![
        ("charForm", char_form),
        ("length", slot(i, o, LENGTH)?),
        ("offset", slot(i, o, OFFSET)?),
        ("packet", packet),
        ("padSize", Value::Number(pad as f64)),
        ("writeable", slot(i, o, WRITEABLE)?),
    ];
    record(i, None, fields)
}

fn get_file_info(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPFile.getFileInfo";
    let o = open_file(i, &this, what)?;
    let fields = vec![
        ("filePath", slot(i, o, PATH)?),
        ("format", slot(i, o, FORMAT)?),
        ("handlerFlags", Value::Number(0.0)),
        ("openFlags", slot(i, o, FLAGS)?),
    ];
    record(i, None, fields)
}

/// Can `o` take a packet: opened for update, and a `.xmp` file?
fn can_update(i: &Interp, o: ObjId) -> JsResult<bool> {
    let open_flags = match slot(i, o, FLAGS)? {
        Value::Number(n) => n as u32,
        _ => 0,
    };
    Ok(open_flags & OPEN_FOR_UPDATE != 0 && slot(i, o, SIDECAR)?.truthy())
}

/// The packet the argument `v` of `putXMP` or `canPutXMP` gives: an `XMPMeta` serialized with its
/// wrapper, or a string, which must be a packet that can be read.
fn packet_arg(i: &mut Interp, v: &Value, what: &str) -> JsResult<String> {
    if let Some(m) = is_meta(i, v) {
        return serialized(i, m, &Value::Undefined, what);
    }
    if let Value::Object(o) = v
        && matches!(i.heap.object(*o).map(|ob| &ob.class), Some(Class::Array { .. }))
    {
        return Err(not_yet(i, &format!("{what}(buffer)")));
    }
    let s = text(i, v)?;
    if s.len() > xmp::MAX_BYTES {
        return Err(xmp_error(i, what, XmpError::TooLarge, ""));
    }
    // A packet is checked by reading it into a copy of the realm's namespaces, so checking it
    // registers nothing.
    let mut scratch = registry(i)?.clone();
    spend(i, s.len().saturating_add(2 * scratch.bytes()))?;
    xmp::validate_within(&s, &mut scratch, i.limits.heap_bytes).map_err(|e| xmp_error(i, what, e, ""))?;
    Ok(s)
}

fn can_put_xmp(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPFile.canPutXMP";
    let o = open_file(i, &this, what)?;
    if !can_update(i, o)? {
        return Ok(Value::Bool(false));
    }
    let packet = packet_arg(i, &arg(args, 0), what);
    match packet {
        // Converting the argument runs script code, which may have closed the file.
        Ok(_) => {
            let o = open_file(i, &this, what)?;
            Ok(Value::Bool(can_update(i, o)?))
        }
        Err(Abrupt::Throw(_)) => Ok(Value::Bool(false)),
        Err(e) => Err(e),
    }
}

fn put_xmp(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPFile.putXMP";
    let o = open_file(i, &this, what)?;
    if !can_update(i, o)? {
        return Err(i.throw(ErrorType::Error, &format!("{what}: the file cannot be updated")));
    }
    let packet = packet_arg(i, &arg(args, 0), what)?;
    // Converting the argument runs script code, which may have closed the file.
    let o = open_file(i, &this, what)?;
    if !can_update(i, o)? {
        return Err(i.throw(ErrorType::Error, &format!("{what}: the file cannot be updated")));
    }
    let v = string(i, &packet)?;
    i.rooted(std::slice::from_ref(&v), |i| set_slot_with_charge(i, o, PENDING, v.clone(), true))?;
    Ok(Value::Undefined)
}

fn close_file(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let what = "XMPFile.closeFile";
    let o = this_file(i, &this, what)?;
    let close_flags = flags(i, &arg(args, 0), what)?;
    only(i, close_flags, CLOSE_UPDATE_SAFELY, what)?;
    let this_run = matches!(slot(i, o, RUN)?, Value::Number(n) if Some(n) == crate::file_classes::run_count(i));
    let pending = slot(i, o, PENDING)?;
    if this_run && slot(i, o, OPEN)?.truthy() && matches!(pending, Value::String(_)) {
        let path = slot(i, o, PATH)?;
        let safely = Value::Bool(close_flags & CLOSE_UPDATE_SAFELY != 0);
        let r =
            i.rooted(&[path.clone(), pending.clone()], |i| crate::file_classes::call_host(i, "xmpWrite", &[path.clone(), pending.clone(), safely]))?;
        let error = i.rooted(std::slice::from_ref(&r), |i| crate::file_classes::record_field(i, &r, "error"))?;
        if let Value::String(_) = error {
            // The file stays open with its packet, so a later `closeFile` tries again.
            let m = text(i, &error)?;
            return Err(i.throw(ErrorType::Error, &m));
        }
    }
    set_slot(i, o, PENDING, Value::Undefined)?;
    set_slot(i, o, PACKET, Value::Null)?;
    set_slot(i, o, OPEN, Value::Bool(false))?;
    Ok(Value::Undefined)
}

// ---- install ----

fn native(i: &mut Interp, name: &'static str, f: crate::heap::NativeFn, length: u32, kind: NativeKind, constructor: bool) -> JsResult<ObjId> {
    i.reserve_heap(OBJECT_BYTES + PROP_BYTES + string_bytes("length".len()))?;
    crate::builtins::native(i, name, f, length, kind, constructor)
}

fn data(i: &mut Interp, on: ObjId, name: &str, value: Value, flags: u8) -> JsResult<()> {
    let added = if i.get_own_property(on, &key(name))?.is_some() { 0 } else { PROP_BYTES + string_bytes(name.encode_utf16().count()) };
    i.rooted(&[Value::Object(on), value.clone()], |i| {
        i.reserve_heap(added)?;
        crate::builtins::data(i, on, name, value, flags)
    })
}

fn method(i: &mut Interp, on: ObjId, name: &'static str, f: crate::heap::NativeFn, length: u32) -> JsResult<()> {
    i.rooted(&[Value::Object(on)], |i| {
        let method = native(i, name, f, length, NativeKind::Plain, false)?;
        data(i, on, name, Value::Object(method), HIDDEN)
    })
}

/// A data property of `XMPConst`: read-only, enumerable.
fn constant(i: &mut Interp, on: ObjId, name: &str, value: Value) -> JsResult<()> {
    data(i, on, name, value, ENUMERABLE)
}

fn constants(i: &mut Interp) -> JsResult<ObjId> {
    i.reserve_heap(OBJECT_BYTES)?;
    let o = i.new_object();
    i.rooted(&[Value::Object(o)], |i| {
        let namespaces = [
            ("NS_DC", xmp::DC),
            ("NS_RDF", xmp::RDF),
            ("NS_XML", xmp::XML),
            ("NS_XMP", xmp::XMP),
            ("NS_XMP_RIGHTS", xmp::XMP_RIGHTS),
            ("NS_PDF", xmp::PDF),
            ("NS_PHOTOSHOP", xmp::PHOTOSHOP), // brand-ok: the constant's name in the reference
        ];
        for (name, uri) in namespaces {
            let value = string(i, uri)?;
            constant(i, o, name, value)?;
        }
        let numbers: [(&str, f64); 14] = [
            ("FILE_UNKNOWN", f64::from(FILE_UNKNOWN)),
            ("FILE_TEXT", f64::from(FILE_TEXT)),
            ("FILE_XML", f64::from(FILE_XML)),
            ("OPEN_FOR_READ", f64::from(OPEN_FOR_READ)),
            ("OPEN_FOR_UPDATE", f64::from(OPEN_FOR_UPDATE)),
            ("OPEN_ONLY_XMP", f64::from(OPEN_ONLY_XMP)),
            ("OPEN_USE_PACKET_SCANNING", f64::from(OPEN_USE_PACKET_SCANNING)),
            ("CLOSE_UPDATE_SAFELY", f64::from(CLOSE_UPDATE_SAFELY)),
            ("SERIALIZE_OMIT_PACKET_WRAPPER", f64::from(SERIALIZE_OMIT_PACKET_WRAPPER)),
            ("PROP_IS_ARRAY", f64::from(xmp::PROP_IS_ARRAY)),
            ("PROP_IS_STRUCT", f64::from(xmp::PROP_IS_STRUCT)),
            ("ARRAY_IS_ORDERED", f64::from(xmp::ARRAY_IS_ORDERED)),
            ("ARRAY_IS_ALTERNATIVE", f64::from(xmp::ARRAY_IS_ALTERNATIVE)),
            ("ARRAY_LAST_ITEM", -1.0),
        ];
        for (name, n) in numbers {
            constant(i, o, name, Value::Number(n))?;
        }
        i.object_mut(o)?.extensible = false;
        Ok(o)
    })
}

fn meta_members(i: &mut Interp, ctor: ObjId, proto: ObjId) -> JsResult<()> {
    method(i, proto, "getProperty", get_property, 2)?;
    method(i, proto, "setProperty", set_property, 3)?;
    method(i, proto, "deleteProperty", delete_property, 2)?;
    method(i, proto, "doesPropertyExist", does_property_exist, 2)?;
    method(i, proto, "countArrayItems", count_array_items, 2)?;
    method(i, proto, "getArrayItem", get_array_item, 3)?;
    method(i, proto, "appendArrayItem", append_array_item, 3)?;
    method(i, proto, "insertArrayItem", insert_array_item, 4)?;
    method(i, proto, "setArrayItem", set_array_item, 4)?;
    method(i, proto, "deleteArrayItem", delete_array_item, 3)?;
    method(i, proto, "doesArrayItemExist", does_array_item_exist, 3)?;
    method(i, proto, "getLocalizedText", get_localized_text, 4)?;
    method(i, proto, "setLocalizedText", set_localized_text, 5)?;
    method(i, proto, "serialize", serialize, 0)?;
    method(i, proto, "iterator", meta_iterator, 0)?;
    method(i, proto, "serializeToArray", meta_serialize_to_array, 0)?;
    method(i, proto, "dumpObject", meta_dump_object, 0)?;
    method(i, proto, "sort", meta_sort, 0)?;
    method(i, proto, "getStructField", meta_get_struct_field, 4)?;
    method(i, proto, "setStructField", meta_set_struct_field, 5)?;
    method(i, proto, "deleteStructField", meta_delete_struct_field, 4)?;
    method(i, proto, "doesStructFieldExist", meta_does_struct_field_exist, 4)?;
    method(i, proto, "getQualifier", meta_get_qualifier, 4)?;
    method(i, proto, "setQualifier", meta_set_qualifier, 5)?;
    method(i, proto, "deleteQualifier", meta_delete_qualifier, 4)?;
    method(i, proto, "doesQualifierExist", meta_does_qualifier_exist, 4)?;
    method(i, ctor, "registerNamespace", register_namespace, 2)?;
    method(i, ctor, "getNamespacePrefix", get_namespace_prefix, 1)?;
    method(i, ctor, "getNamespaceURI", get_namespace_uri, 1)?;
    method(i, ctor, "dumpNamespaces", meta_dump_namespaces, 0)?;
    method(i, ctor, "registerAlias", meta_register_alias, 5)?;
    method(i, ctor, "resolveAlias", meta_resolve_alias, 2)?;
    method(i, ctor, "dumpAliases", meta_dump_aliases, 0)?;
    method(i, ctor, "deleteAlias", meta_delete_alias, 2)?;
    method(i, ctor, "deleteNamespace", meta_delete_namespace, 1)
}

fn file_members(i: &mut Interp, ctor: ObjId, proto: ObjId) -> JsResult<()> {
    method(i, proto, "getXMP", get_xmp, 0)?;
    method(i, proto, "putXMP", put_xmp, 1)?;
    method(i, proto, "canPutXMP", can_put_xmp, 1)?;
    method(i, proto, "getPacketInfo", get_packet_info, 0)?;
    method(i, proto, "getFileInfo", get_file_info, 0)?;
    method(i, proto, "closeFile", close_file, 1)?;
    method(i, ctor, "getFormatInfo", file_get_format_info, 1)
}

/// Links the constructor `ctor` and its prototype `proto`, without making it a global.
fn link(i: &mut Interp, ctor: ObjId, proto: ObjId) -> JsResult<()> {
    data(i, ctor, "prototype", Value::Object(proto), 0)?;
    data(i, proto, "constructor", Value::Object(ctor), HIDDEN)
}

/// Adds the global `ExternalObject`, which defines `XMPMeta`, `XMPFile` and `XMPConst` when a
/// script loads the XMP library ([`crate::Realm::define_external_object`]). A call after a
/// successful installation is refused and preserves registered namespaces. A failed installation can be retried.
pub(crate) fn install(i: &mut Interp) -> JsResult<()> {
    if i.xmp.is_some() {
        return Err(Abrupt::Internal("ExternalObject is already added".into()));
    }
    let object_proto = i.intrinsics.object_proto;
    i.reserve_heap(4 * OBJECT_BYTES)?;
    let external_proto = i.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));
    let meta_proto = i.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));
    let file_proto = i.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));
    let property_proto = i.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));
    let keep = [external_proto, meta_proto, file_proto, property_proto].map(Value::Object);
    let installed = i.rooted(&keep, |i| {
        let meta = native(i, "XMPMeta", meta_ctor, 1, NativeKind::Plain, true)?;
        i.rooted(&[Value::Object(meta)], |i| {
            let file = native(i, "XMPFile", file_ctor, 3, NativeKind::Plain, true)?;
            i.rooted(&[Value::Object(file)], |i| {
                let constants = constants(i)?;
                let registry = Registry::default();
                i.rooted(&[Value::Object(constants)], |i| i.reserve_heap(registry.bytes()))?;
                i.heap.bytes = i.heap.bytes.saturating_add(registry.bytes());
                i.xmp = Some(Classes { ids: Ids { external_proto, meta, meta_proto, file, file_proto, constants, property_proto }, registry });
                link(i, meta, meta_proto)?;
                link(i, file, file_proto)?;
                meta_members(i, meta, meta_proto)?;
                file_members(i, file, file_proto)?;
                method(i, property_proto, "toString", property_to_string, 0)?;
                let external = native(i, "ExternalObject", external_ctor, 1, NativeKind::Plain, true)?;
                i.rooted(&[Value::Object(external)], |i| {
                    method(i, external, "search", external_search, 1)?;
                    method(i, external_proto, "terminate", external_terminate, 0)?;
                    link(i, external, external_proto)?;
                    data(i, i.global, "ExternalObject", Value::Object(external), HIDDEN)
                })
            })
        })
    });
    if installed.is_err() {
        i.xmp = None;
    }
    installed
}

#[cfg(test)]
mod namespace_heap_tests {
    use super::*;

    #[test]
    fn non_xml_whitespace_sidecars_are_not_empty_metadata() {
        for text in ["\u{a0}", "\u{2003}", "\u{85}", " \t\r\n"] {
            let mut interp = Interp::new(crate::Options::default(), crate::Limits::default());
            crate::file_classes::install(
                &mut interp,
                vec![("current".into(), crate::HostItem::Function(std::rc::Rc::new(|_| Ok(crate::Data::String(String::new())))))],
            )
            .unwrap();
            install(&mut interp).unwrap();
            let mut fields = PropMap::default();
            fields.insert(key(PATH), Value::str("packet.xmp"));
            fields.insert(key(OPEN), Value::Bool(true));
            fields.insert(key(RUN), Value::Number(crate::file_classes::run_count(&interp).unwrap()));
            fields.insert(key(PACKET), Value::str(text));
            let object = interp.heap.alloc_object(Object::new(Class::XmpFile(Box::new(fields)), Some(ids(&interp).unwrap().file_proto)));
            let value = Value::Object(object);
            interp
                .rooted(std::slice::from_ref(&value), |interp| {
                    let result = get_xmp(interp, value.clone(), &[], false);
                    if text == " \t\r\n" {
                        assert!(result.is_ok());
                    } else {
                        assert!(matches!(result, Err(Abrupt::Throw(_))), "{text:?}: {result:?}");
                    }
                    Ok(())
                })
                .unwrap();
        }
    }

    #[test]
    fn inserting_array_items_charges_for_shifted_items() {
        let mut interp = Interp::new(crate::Options::default(), crate::Limits::default());
        install(&mut interp).unwrap();
        let meta = Meta {
            props: vec![xmp::Prop {
                ns: xmp::DC.into(),
                name: "items".into(),
                value: xmp::Value::Array { form: Form::Bag, items: vec![Item { text: "text".into(), lang: None }; 1000] },
            }],
            ..Meta::default()
        };
        let value = new_meta(&mut interp, meta).unwrap();
        let object = value.as_object().unwrap();
        interp
            .rooted(&[value], |interp| {
                interp.fuel = 100;
                let result = insert_item(interp, object, xmp::DC, "items", Some(1), "insert".into(), None, "insert");
                assert!(matches!(result, Err(Abrupt::Limit(crate::LimitKind::Fuel))), "{result:?}");
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn result_records_reserve_their_retained_property_keys() {
        let mut interp = Interp::new(crate::Options::default(), crate::Limits::default());
        interp.collect_garbage();
        let name = "a_property_name_with_retained_key_storage";
        interp.limits.heap_bytes = interp.heap.bytes + OBJECT_BYTES + PROP_BYTES + string_bytes(name.len()) - 1;
        let result = record(&mut interp, None, vec![(name, Value::Number(1.0))]);
        assert!(matches!(result, Err(Abrupt::Limit(crate::LimitKind::Heap))), "{result:?}");
    }

    #[test]
    fn replacing_array_items_charges_metadata_accounting_work() {
        let mut interp = Interp::new(crate::Options::default(), crate::Limits::default());
        install(&mut interp).unwrap();
        let meta = Meta {
            props: vec![xmp::Prop {
                ns: xmp::DC.into(),
                name: "items".into(),
                value: xmp::Value::Array { form: Form::Bag, items: vec![Item { text: "text".into(), lang: None }; 1000] },
            }],
            ..Meta::default()
        };
        let value = new_meta(&mut interp, meta).unwrap();
        interp
            .rooted(std::slice::from_ref(&value), |interp| {
                interp.fuel = 100;
                let result =
                    set_array_item(interp, value.clone(), &[Value::str(xmp::DC), Value::str("items"), Value::Number(1.0), Value::str("edit")], false);
                assert!(matches!(result, Err(Abrupt::Limit(crate::LimitKind::Fuel))), "{result:?}");
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn unrelated_namespaces_do_not_spend_serialization_fuel() {
        let mut interp = Interp::new(crate::Options::default(), crate::Limits::default());
        install(&mut interp).unwrap();
        for index in 0..400 {
            registry(&mut interp).unwrap().register(&format!("urn:unused:{index}:{}", "x".repeat(990)), &format!("unused_{index}")).unwrap();
        }
        let value = new_meta(&mut interp, Meta::default()).unwrap();
        let object = value.as_object().unwrap();
        interp
            .rooted(&[value], |interp| {
                interp.fuel = 10_000;
                assert!(serialized(interp, object, &Value::Undefined, "serialize").is_ok());
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn repeated_library_handles_fit_their_retained_size() {
        let mut interp = Interp::new(crate::Options::default(), crate::Limits::default());
        install(&mut interp).unwrap();
        external_ctor(&mut interp, Value::Undefined, &[Value::str(LIBRARY)], true).unwrap();
        for room in 0..2048 {
            interp.collect_garbage();
            let baseline = interp.heap.bytes;
            interp.limits.heap_bytes = baseline + room;
            if let Ok(value) = external_ctor(&mut interp, Value::Undefined, &[Value::str(LIBRARY)], true) {
                interp
                    .rooted(&[value], |interp| {
                        interp.collect_garbage();
                        assert_eq!(room, interp.heap.bytes - baseline, "extra heap reservation for a retained handle");
                        Ok(())
                    })
                    .unwrap();
                return;
            }
        }
        panic!("no library handle fit");
    }

    #[test]
    fn a_failed_external_constructor_publishes_no_partial_globals() {
        let mut failures = 0;
        let mut successes = 0;
        for room in (0..4096).step_by(16) {
            let mut interp = Interp::new(crate::Options::default(), crate::Limits::default());
            install(&mut interp).unwrap();
            interp.collect_garbage();
            interp.limits.heap_bytes = interp.heap.bytes + room;
            let result = external_ctor(&mut interp, Value::Undefined, &[Value::str(LIBRARY)], true);
            if result.is_err() {
                failures += 1;
                for name in ["XMPMeta", "XMPFile", "XMPConst"] {
                    assert!(interp.get_own_property(interp.global, &key(name)).unwrap().is_none(), "partial global {name}, room={room}");
                }
            } else if let Ok(value) = result {
                successes += 1;
                interp
                    .rooted(&[value], |interp| {
                        interp.collect_garbage();
                        assert!(
                            interp.heap.bytes <= interp.limits.heap_bytes,
                            "constructor retained {} bytes with limit {}",
                            interp.heap.bytes,
                            interp.limits.heap_bytes
                        );
                        Ok(())
                    })
                    .unwrap();
            }
        }
        assert!(failures > 0 && successes > 0);
    }

    #[test]
    fn packet_info_charges_fuel_for_scanning_the_packet() {
        let mut interp = Interp::new(crate::Options::default(), crate::Limits::default());
        crate::file_classes::install(
            &mut interp,
            vec![("current".into(), crate::HostItem::Function(std::rc::Rc::new(|_| Ok(crate::Data::String(String::new())))))],
        )
        .unwrap();
        install(&mut interp).unwrap();
        let mut fields = PropMap::default();
        fields.insert(key(PATH), Value::str("packet.xmp"));
        fields.insert(key(OPEN), Value::Bool(true));
        fields.insert(key(RUN), Value::Number(crate::file_classes::run_count(&interp).unwrap()));
        fields.insert(key(PACKET), Value::str(&" ".repeat(4096)));
        let object = interp.heap.alloc_object(Object::new(Class::XmpFile(Box::new(fields)), Some(ids(&interp).unwrap().file_proto)));
        let this = Value::Object(object);
        interp
            .rooted(std::slice::from_ref(&this), |interp| {
                interp.fuel = 1000;
                get_packet_info(interp, this.clone(), &[], false)?;
                assert!(interp.fuel <= 983, "packet scan used only {} fuel", 1000 - interp.fuel);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn staging_a_packet_charges_its_string_only_once() {
        let mut interp = Interp::new(crate::Options::default(), crate::Limits::default());
        crate::file_classes::install(
            &mut interp,
            vec![("current".into(), crate::HostItem::Function(std::rc::Rc::new(|_| Ok(crate::Data::String(String::new())))))],
        )
        .unwrap();
        install(&mut interp).unwrap();
        let mut fields = PropMap::default();
        for (name, value) in [
            (PATH, Value::str("packet.xmp")),
            (RUN, Value::Number(crate::file_classes::run_count(&interp).unwrap())),
            (OPEN, Value::Bool(true)),
            (FLAGS, Value::Number(f64::from(OPEN_FOR_UPDATE))),
            (SIDECAR, Value::Bool(true)),
            (PENDING, Value::Undefined),
        ] {
            fields.insert(key(name), value);
        }
        let object = interp.heap.alloc_object(Object::new(Class::XmpFile(Box::new(fields)), Some(ids(&interp).unwrap().file_proto)));
        let this = Value::Object(object);
        interp
            .rooted(std::slice::from_ref(&this), |interp| {
                let packet = format!(
                    "<rdf:RDF xmlns:rdf=\"{}\"><rdf:Description xmlns:a=\"urn:test\"><a:value>{}</a:value></rdf:Description></rdf:RDF>",
                    xmp::RDF,
                    "x".repeat(4096)
                );
                let argument = string(interp, &packet)?;
                interp.rooted(std::slice::from_ref(&argument), |interp| {
                    interp.collect_garbage();
                    interp.limits.heap_bytes = interp.heap.bytes + string_bytes(packet.encode_utf16().count()) + 512;
                    let result = put_xmp(interp, this.clone(), std::slice::from_ref(&argument), false);
                    assert!(result.is_ok(), "a single pending packet charge fits: {result:?}");
                    Ok(())
                })
            })
            .unwrap();
    }

    #[test]
    fn storing_an_open_packet_reuses_its_charged_string() {
        let mut interp = Interp::new(crate::Options::default(), crate::Limits::default());
        install(&mut interp).unwrap();
        let packet = string(&mut interp, &"x".repeat(4096)).unwrap();
        interp
            .rooted(std::slice::from_ref(&packet), |interp| {
                let object = interp.heap.alloc_object(Object::new(Class::XmpFile(Box::default()), Some(ids(interp)?.file_proto)));
                interp.rooted(&[Value::Object(object)], |interp| {
                    interp.collect_garbage();
                    interp.limits.heap_bytes = interp.heap.bytes + 512;
                    set_slot(interp, object, PACKET, packet.clone())
                })
            })
            .unwrap();
    }

    #[test]
    fn a_failed_serialization_charges_its_escaping_work() {
        let mut interp = Interp::new(crate::Options::default(), crate::Limits::default());
        install(&mut interp).unwrap();
        let meta = Meta {
            props: vec![xmp::Prop {
                ns: xmp::DC.into(),
                name: "value".into(),
                value: xmp::Value::Simple { text: "&".repeat(xmp::MAX_BYTES / 4), uri: false },
            }],
            ..Meta::default()
        };
        let value = new_meta(&mut interp, meta).unwrap();
        let object = value.as_object().unwrap();
        interp
            .rooted(&[value], |interp| {
                interp.fuel = 100_000;
                let result = serialized(interp, object, &Value::Undefined, "serialize");
                assert!(matches!(result, Err(Abrupt::Throw(_))), "{result:?}");
                assert!(interp.fuel < 100_000, "failed escaping left its work unpaid");
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn indexed_property_reads_copy_only_the_selected_item() {
        let mut interp = Interp::new(crate::Options::default(), crate::Limits::default());
        install(&mut interp).unwrap();
        let meta = Meta {
            props: vec![xmp::Prop {
                ns: xmp::DC.into(),
                name: "items".into(),
                value: xmp::Value::Array { form: Form::Bag, items: vec![Item { text: "text".into(), lang: None }; 1000] },
            }],
            ..Meta::default()
        };
        let value = new_meta(&mut interp, meta).unwrap();
        interp
            .rooted(std::slice::from_ref(&value), |interp| {
                xmp::ITEM_COPIES.with(|copies| copies.set(0));
                for index in 1..=1000 {
                    get_property(interp, value.clone(), &[Value::str(xmp::DC), Value::str(&format!("items[{index}]"))], false).unwrap();
                }
                let copies = xmp::ITEM_COPIES.with(|copies| copies.get());
                assert!(copies <= 1000, "copied {copies} items for 1000 indexed reads");
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn appending_array_items_has_linear_accounting_work() {
        let mut interp = Interp::new(crate::Options::default(), crate::Limits::default());
        install(&mut interp).unwrap();
        let value = new_meta(&mut interp, Meta::default()).unwrap();
        let object = value.as_object().unwrap();
        interp
            .rooted(&[value], |interp| {
                xmp::ACCOUNTING_ITEM_VISITS.with(|visits| visits.set(0));
                for _ in 0..1000 {
                    insert_item(interp, object, xmp::DC, "items", None, "text".into(), Some(Form::Bag), "append").unwrap();
                }
                let visits = xmp::ACCOUNTING_ITEM_VISITS.with(|visits| visits.get());
                assert!(visits <= 2000, "accounting visited {visits} items for 1000 appends");
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn parsed_namespaces_fit_with_the_metadata_before_registration() {
        let mut interp = Interp::new(crate::Options::default(), crate::Limits::default());
        install(&mut interp).unwrap();
        interp.collect_garbage();
        interp.limits.heap_bytes = interp.heap.bytes + (24 << 10);
        let mut declarations = String::new();
        let mut properties = String::new();
        let mut first_uri = String::new();
        for index in 0..16 {
            let uri = format!("urn:{index}:{}", "x".repeat(950));
            if index == 0 {
                first_uri.clone_from(&uri);
            }
            declarations.push_str(&format!(" xmlns:n{index}=\"{uri}\""));
            properties.push_str(&format!("<n{index}:value>text</n{index}:value>"));
        }
        let packet = format!(
            "<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"{declarations}><rdf:Description>{properties}</rdf:Description></rdf:RDF>"
        );
        let result = parse_packet(&mut interp, "XMPMeta", &packet).and_then(|meta| new_meta(&mut interp, meta));
        assert!(matches!(result, Err(Abrupt::Limit(crate::LimitKind::Heap))), "{result:?}");
        assert!(registry(&mut interp).unwrap().prefix_of(&first_uri).is_none(), "a failed allocation leaves the namespaces unchanged");
    }
}
