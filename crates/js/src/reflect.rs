//! The script extension `reflect` ([`crate::Realm::add_script_extensions`]): a read-only property
//! of `Object.prototype` that gives, on each read, a new reflection object describing the object it
//! is read on.
//!
//! A reflection object has `name` (the object's class), `description` and `help` (`undefined`),
//! `properties` and `methods` (arrays of ReflectionInfo objects), and `find(name)`, which gives the
//! ReflectionInfo of the property or method with that name, or `null`. A ReflectionInfo has `name`
//! (a string, or a number for an array index), `description`, `help`, `type` (`readonly`,
//! `readwrite`, `method`, or `parameter` for a method's argument), `dataType`, `defaultValue`,
//! `min`, `max`, `isCollection` and `arguments` (an array of ReflectionInfo objects, empty for a
//! property). Each entry of `properties`, `methods` and `arguments` is also under its name when
//! that name is not an array index or `length`.
//!
//! An object of a host class and the class's constructor are described by the class's members
//! ([`crate::HostClass::members`] and [`crate::HostClass::info`]), its parents' included. Any other
//! object is described by its own properties: a property whose value is a function is a method,
//! with the parameter names of a function written in JavaScript as its arguments; a data property
//! is read-write when it is writable and gives the type of its value; an accessor is read-write
//! when it has a setter and its type is `unknown`. A string, number or boolean is described by its
//! prototype's own properties. No getter or other script code runs.
//!
//! Every object made is frozen, so a script cannot change one; `reflect` itself has no setter.
//! Each member described takes fuel, and an object with more than [`MAX_REFLECTED_MEMBERS`]
//! members and arguments throws a `RangeError`.

use crate::builtins::native;
use crate::heap::{Class, ENUMERABLE, Func, Key, NativeKind, OBJECT_BYTES, ObjId, Object, PROP_BYTES, Prop, WRITABLE, key};
use crate::host::Data;
use crate::host_classes::{Access, Reflected};
use crate::interp::{ErrorType, Interp, JsResult};
use crate::value::{Str, Value, array_index, index_key, number_to_str};

/// The most members and arguments `reflect` describes for one object.
pub const MAX_REFLECTED_MEMBERS: usize = 10_000;

/// The properties of a ReflectionInfo object.
const INFO_PROPS: usize = 10;

/// A member or argument as a ReflectionInfo object describes it.
struct Info {
    name: Value,
    kind: &'static str,
    data_type: String,
    description: Option<String>,
    help: Option<String>,
    default: Option<Data>,
    min: Option<f64>,
    max: Option<f64>,
    collection: bool,
    arguments: Vec<Info>,
}

impl Info {
    fn new(name: Value, kind: &'static str, data_type: &str) -> Self {
        Self {
            name,
            kind,
            data_type: data_type.to_string(),
            description: None,
            help: None,
            default: None,
            min: None,
            max: None,
            collection: false,
            arguments: Vec::new(),
        }
    }
}

/// The type a [`crate::MemberInfo`] gives, `unknown` when it gives none.
fn declared(data_type: &str) -> &str {
    if data_type.is_empty() { "unknown" } else { data_type }
}

pub(crate) fn install(interp: &mut Interp) -> JsResult<()> {
    let get = native(interp, "reflect", reflect, 0, NativeKind::Plain, false)?;
    let object_proto = interp.intrinsics.object_proto;
    interp.define(object_proto, key("reflect"), Prop::Accessor { get: Some(get), set: None, flags: 0 })
}

/// Counts one more member or argument against [`MAX_REFLECTED_MEMBERS`], with a unit of fuel.
fn count(interp: &mut Interp, total: &mut usize) -> JsResult<()> {
    interp.use_fuel()?;
    *total += 1;
    if *total > MAX_REFLECTED_MEMBERS {
        return Err(interp.throw(ErrorType::Range, &format!("reflect: the object has more than {MAX_REFLECTED_MEMBERS} members")));
    }
    Ok(())
}

/// The getter of `Object.prototype.reflect`.
fn reflect(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (id, primitive) = match &this {
        Value::Undefined | Value::Null => return Err(interp.type_error("reflect needs an object")),
        Value::String(_) => (interp.intrinsics.string_proto, Some("String")),
        Value::Number(_) => (interp.intrinsics.number_proto, Some("Number")),
        Value::Bool(_) => (interp.intrinsics.boolean_proto, Some("Boolean")),
        Value::Symbol(_) => (interp.symbols.objects.map_or(interp.intrinsics.object_proto, |o| o.symbol_proto), Some("Symbol")),
        Value::Object(o) => (*o, None),
    };
    let (name, properties, methods) = match interp.host_class_members(id)? {
        Some((name, members)) if primitive.is_none() => {
            let (properties, methods) = host_infos(interp, &members)?;
            (name.to_string(), properties, methods)
        }
        _ => {
            let name = primitive.unwrap_or(interp.object(id)?.class.name()).to_string();
            let (properties, methods) = own_infos(interp, id)?;
            (name, properties, methods)
        }
    };
    interp.rooted(&[this], |i| {
        let mark = i.temp_roots.len();
        let r = reflection(i, &name, &properties, &methods);
        i.temp_roots.truncate(mark);
        r
    })
}

/// The properties and methods of a host class's `members`.
fn host_infos(interp: &mut Interp, members: &[Reflected]) -> JsResult<(Vec<Info>, Vec<Info>)> {
    let (mut properties, mut methods) = (Vec::new(), Vec::new());
    let mut total = 0;
    for m in members {
        count(interp, &mut total)?;
        let kind = match m.access {
            Access::ReadOnly => "readonly",
            Access::ReadWrite => "readwrite",
            Access::Method => "method",
        };
        let mut info = Info::new(Value::str(&m.name), kind, declared(&m.info.data_type));
        info.description = m.info.description.clone();
        info.help = m.info.help.clone();
        info.default = m.info.default.clone();
        info.min = m.info.min;
        info.max = m.info.max;
        info.collection = m.info.collection;
        for a in &m.info.arguments {
            count(interp, &mut total)?;
            let mut arg = Info::new(Value::str(&a.name), "parameter", declared(&a.data_type));
            arg.description = a.description.clone();
            arg.default = a.default.clone();
            arg.min = a.min;
            arg.max = a.max;
            info.arguments.push(arg);
        }
        if m.access == Access::Method { methods.push(info) } else { properties.push(info) }
    }
    Ok((properties, methods))
}

/// The properties and methods of the own properties of `id`. No getter runs.
fn own_infos(interp: &mut Interp, id: ObjId) -> JsResult<(Vec<Info>, Vec<Info>)> {
    let o = interp.object(id)?;
    let extra = match &o.class {
        Class::String(s) => s.len().saturating_add(1),
        Class::Array { .. } => 1,
        _ => 0,
    };
    // Checked before the names are listed, so a huge object costs no more than a small one.
    if o.props.len().saturating_add(extra) > MAX_REFLECTED_MEMBERS {
        return Err(interp.throw(ErrorType::Range, &format!("reflect: the object has more than {MAX_REFLECTED_MEMBERS} members")));
    }
    let keys = interp.own_keys(id)?;
    let (mut properties, mut methods) = (Vec::new(), Vec::new());
    let mut total = 0;
    for k in keys {
        count(interp, &mut total)?;
        let Some(prop) = interp.get_own_property(id, &k)? else { continue };
        let name = member_name(&k);
        match prop {
            Prop::Data { value, .. } if interp.is_callable(&value) => {
                let mut info = Info::new(name, "method", "unknown");
                if let Some(params) = parameters(interp, &value)? {
                    for p in params {
                        count(interp, &mut total)?;
                        info.arguments.push(Info::new(Value::String(Str::from_rc(p)), "parameter", "*"));
                    }
                }
                methods.push(info);
            }
            Prop::Data { value, flags } => {
                let kind = if flags & WRITABLE != 0 { "readwrite" } else { "readonly" };
                let data_type = value_type(interp, &value)?;
                properties.push(Info::new(name, kind, &data_type));
            }
            Prop::Accessor { set, .. } => properties.push(Info::new(name, if set.is_some() { "readwrite" } else { "readonly" }, "unknown")),
        }
    }
    Ok((properties, methods))
}

/// A property name as a ReflectionInfo's `name`: a number for an array index.
fn member_name(k: &Key) -> Value {
    match array_index(k) {
        Some(i) => Value::Number(f64::from(i)),
        None => Value::String(Str::from_rc(k.clone())),
    }
}

/// The parameter names of the function `f` when it is written in JavaScript.
fn parameters(interp: &Interp, f: &Value) -> JsResult<Option<Vec<Key>>> {
    let Some(id) = f.as_object() else { return Ok(None) };
    Ok(match &interp.object(id)?.class {
        Class::Function(Func::Closure { code, .. }) => Some(code.params.clone()),
        _ => None,
    })
}

/// The `dataType` of a data property holding `v`.
fn value_type(interp: &Interp, v: &Value) -> JsResult<String> {
    Ok(match v {
        Value::Undefined => "undefined".into(),
        Value::Null => "null".into(),
        Value::Bool(_) => "boolean".into(),
        Value::Number(_) => "number".into(),
        Value::String(_) => "string".into(),
        Value::Symbol(_) => "symbol".into(),
        Value::Object(o) => match interp.host_class_name(*o)? {
            Some(name) => name.to_string(),
            None => interp.object(*o)?.class.name().into(),
        },
    })
}

/// The reflection object. Every object it makes is kept in `interp.temp_roots`.
fn reflection(interp: &mut Interp, name: &str, properties: &[Info], methods: &[Info]) -> JsResult<Value> {
    let properties = info_array(interp, properties)?;
    let methods = info_array(interp, methods)?;
    interp.reserve_heap(2 * OBJECT_BYTES + 8 * PROP_BYTES)?;
    let find = native(interp, "find", find, 1, NativeKind::Plain, false)?;
    interp.object_mut(find)?.extensible = false;
    interp.temp_roots.push(Value::Object(find));
    let r = interp.new_object();
    interp.temp_roots.push(Value::Object(r));
    for (k, v) in [
        ("name", Value::str(name)),
        ("description", Value::Undefined),
        ("help", Value::Undefined),
        ("properties", Value::Object(properties)),
        ("methods", Value::Object(methods)),
    ] {
        interp.define(r, key(k), Prop::Data { value: v, flags: ENUMERABLE })?;
    }
    interp.define(r, key("find"), Prop::Data { value: Value::Object(find), flags: 0 })?;
    interp.object_mut(r)?.extensible = false;
    Ok(Value::Object(r))
}

/// A frozen array of ReflectionInfo objects for `infos`, each also under its name when that name
/// is not an array index or `length`.
fn info_array(interp: &mut Interp, infos: &[Info]) -> JsResult<ObjId> {
    interp.reserve_heap(OBJECT_BYTES.saturating_add(infos.len().saturating_mul(2 * PROP_BYTES)))?;
    let proto = interp.intrinsics.array_proto;
    let a = interp.heap.alloc_object(Object::new(Class::Array { length: 0, length_writable: false }, Some(proto)));
    interp.temp_roots.push(Value::Object(a));
    for (n, info) in infos.iter().enumerate() {
        let o = info_object(interp, info)?;
        interp.define(a, index_key(u32::try_from(n).unwrap_or(u32::MAX)), Prop::Data { value: Value::Object(o), flags: ENUMERABLE })?;
    }
    for (n, info) in infos.iter().enumerate() {
        let Value::String(s) = &info.name else { continue };
        let k = s.flat();
        if array_index(&k).is_some() || *k == *key("length") || interp.object(a)?.props.contains(&k) {
            continue;
        }
        let Some(Prop::Data { value, .. }) = interp.get_own_property(a, &index_key(u32::try_from(n).unwrap_or(u32::MAX)))? else { continue };
        interp.define(a, k, Prop::Data { value, flags: 0 })?;
    }
    interp.object_mut(a)?.extensible = false;
    Ok(a)
}

/// A frozen ReflectionInfo object for `info`.
fn info_object(interp: &mut Interp, info: &Info) -> JsResult<ObjId> {
    let arguments = info_array(interp, &info.arguments)?;
    interp.reserve_heap(OBJECT_BYTES + INFO_PROPS * PROP_BYTES)?;
    let o = interp.new_object();
    interp.temp_roots.push(Value::Object(o));
    let text = |s: &Option<String>| s.as_deref().map_or(Value::Undefined, Value::str);
    let number = |n: Option<f64>| n.map_or(Value::Undefined, Value::Number);
    let default = match &info.default {
        Some(d) => interp.write_data(d.clone(), 0, true)?,
        None => Value::Undefined,
    };
    let fields = [
        ("name", info.name.clone()),
        ("description", text(&info.description)),
        ("help", text(&info.help)),
        ("type", Value::str(info.kind)),
        ("dataType", Value::str(&info.data_type)),
        ("defaultValue", default),
        ("min", number(info.min)),
        ("max", number(info.max)),
        ("isCollection", Value::Bool(info.collection)),
        ("arguments", Value::Object(arguments)),
    ];
    for (k, v) in fields {
        interp.define(o, key(k), Prop::Data { value: v, flags: ENUMERABLE })?;
    }
    interp.object_mut(o)?.extensible = false;
    Ok(o)
}

/// `reflect.find(name)`: the ReflectionInfo of the property, or else the method, named `name` (a
/// string, or a number for an array index), or `null`.
fn find(interp: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let wanted = match args.first() {
        Some(Value::String(s)) => s.flat(),
        Some(Value::Number(n)) => number_to_str(*n).flat(),
        _ => return Err(interp.type_error("reflect.find: give the member's name, as a string or a number")),
    };
    let not_reflection = "reflect.find must be called on a reflection object";
    let Value::Object(r) = this else { return Err(interp.type_error(not_reflection)) };
    for list in ["properties", "methods"] {
        let Some(Prop::Data { value: Value::Object(a), .. }) = interp.get_own_property(r, &key(list))? else {
            return Err(interp.type_error(not_reflection));
        };
        let Class::Array { length, .. } = interp.object(a)?.class else { return Err(interp.type_error(not_reflection)) };
        for i in 0..length {
            interp.use_fuel()?;
            let Some(Prop::Data { value: Value::Object(e), .. }) = interp.get_own_property(a, &index_key(i))? else { continue };
            let name = match interp.get_own_property(e, &key("name"))? {
                Some(Prop::Data { value: Value::String(s), .. }) => s.flat(),
                Some(Prop::Data { value: Value::Number(n), .. }) => number_to_str(n).flat(),
                _ => continue,
            };
            if *name == *wanted {
                return Ok(Value::Object(e));
            }
        }
    }
    Ok(Value::Null)
}
