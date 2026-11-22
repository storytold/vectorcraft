//! Classes a host defines ([`crate::Realm::define_host_classes`]): objects that stand for something
//! the host keeps, such as an open document or an object in one, with members the host's functions
//! answer.
//!
//! Each object holds the name of its class and a handle, a list of numbers the host chose, in an
//! internal slot that a script cannot read, write or copy onto another object. A host function sees
//! such an object as the [`Data`] that [`host_ref`] makes, and gives one back the same way; a plain
//! object a script made that carries [`HOST_REF_KEY`] is refused, so a script cannot make an object
//! of a host class from its own data. A class's prototype and constructor are frozen. While a
//! script holds the object for a class and handle, each read of that class and handle gives the
//! same object. Classes that descend from the same class without a parent share their objects: a
//! read of another of them with the same handle gives the object the script holds, now of the
//! class that read names.
//!
//! Every member function gets the object it is called on (`this`) as its first argument, or
//! `undefined` when `this` is not an object of a host class, followed by the call's arguments; a
//! setter gets `this` and the value assigned. A class with an indexer answers `object[i]` for each
//! array index `i` with it, called with `this` and the index, and `i in object` is true when it
//! gives a value other than `undefined`; other names are looked up as usual.
//!
//! A script's `reflect` ([`crate::reflect`]) describes an object of a host class, and the class's
//! constructor, from the members installed here and their [`HostClass::info`], so what a script is
//! told always matches what the class has. A class that names a member twice, or describes a member
//! it does not have, is refused.

use std::rc::Rc;

use crate::heap::{Class, Func, ObjId, Object, Prop, key};
use crate::host::ArgRead;
use crate::interp::{Abrupt, ErrorType, Interp, JsResult};
use crate::value::Value;
use crate::{Data, HostError, HostFunction};

/// The one key of the [`Data`] object that stands for an object of a host class ([`host_ref`]).
pub const HOST_REF_KEY: &str = "\u{0}hostRef";

/// A member of a host class's prototype.
#[derive(Clone)]
pub enum HostMember {
    /// A read-only property: the function gives its value each time it is read.
    Get(HostFunction),
    /// A property the function `get` reads and `set` assigns.
    Property { get: HostFunction, set: HostFunction },
    /// A method.
    Method(HostFunction),
}

/// What a script's `reflect` tells of a member of a host class ([`HostClass::info`]). The member's
/// name and whether it is read-only, read-write or a method come from the [`HostMember`] itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemberInfo {
    /// The type of the member's value, or of a method's result: `boolean`, `number`, `string`, a
    /// class name, `*` for any type, `null` or `undefined`. Empty reads as `unknown`.
    pub data_type: String,
    pub description: Option<String>,
    pub help: Option<String>,
    /// The value an omitted value means.
    pub default: Option<Data>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// The value, or a method's result, is a collection.
    pub collection: bool,
    /// A method's arguments, in order.
    pub arguments: Vec<ArgInfo>,
    /// The member is installed but throws when used: `reflect` leaves it out.
    pub unavailable: bool,
}

/// What a script's `reflect` tells of an argument of a host class's method ([`MemberInfo::arguments`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArgInfo {
    pub name: String,
    /// As [`MemberInfo::data_type`].
    pub data_type: String,
    pub description: Option<String>,
    pub default: Option<Data>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// A class [`crate::Realm::define_host_classes`] adds.
#[derive(Clone)]
pub struct HostClass {
    pub name: String,
    /// A class defined before this one, or in the same call before it, whose prototype this
    /// class's prototype inherits from.
    pub parent: Option<String>,
    pub members: Vec<(String, HostMember)>,
    /// What `reflect` tells of the members, each under the name of one of `members`. A member
    /// without an entry has the type `unknown` and nothing else.
    pub info: Vec<(String, MemberInfo)>,
    /// What `object[i]` gives for an array index `i`.
    pub index: Option<HostFunction>,
    /// What `new Name(...)` and `Name(...)` give; without it both throw a `TypeError`.
    pub construct: Option<HostFunction>,
}

/// The internal slot of an object of a host class: its class, as an index into the realm's
/// classes, and its handle.
#[derive(Clone, Debug)]
pub(crate) struct HostObj {
    pub(crate) class: u32,
    pub(crate) handle: Rc<[f64]>,
}

/// A class the realm has.
#[derive(Clone)]
pub(crate) struct Entry {
    name: Rc<str>,
    proto: ObjId,
    ctor: ObjId,
    index: Option<HostFunction>,
    /// The class without a parent this class descends from (itself for such a class), as an index
    /// into the realm's classes.
    family: u32,
    /// The parent class, as an index into the realm's classes.
    parent: Option<u32>,
    /// The class's own members as `reflect` lists them.
    members: Rc<[Reflected]>,
}

/// How a script may use a member of a host class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    ReadOnly,
    ReadWrite,
    Method,
}

/// A member of a host class as `reflect` lists it.
#[derive(Clone, Debug)]
pub(crate) struct Reflected {
    pub(crate) name: Rc<str>,
    pub(crate) access: Access,
    pub(crate) info: MemberInfo,
}

impl Entry {
    pub(crate) fn objects(&self) -> [ObjId; 2] {
        [self.proto, self.ctor]
    }
}

/// A family of classes ([`Entry::family`]) and the bits of a handle.
type MadeKey = (u32, Box<[u64]>);

/// The object made for each family of classes and handle ([`Interp::host_objects`]).
pub(crate) type Made = std::collections::HashMap<MadeKey, ObjId>;

/// Whether `id` is still the object made for `key`, and not a freed cell or another object that
/// was given its cell.
fn is_made(heap: &crate::heap::Heap, classes: &[Entry], id: ObjId, key: &MadeKey) -> bool {
    heap.object(id).is_some_and(|o| match &o.class {
        Class::Host(h) => {
            classes.get(h.class as usize).is_some_and(|e| e.family == key.0) && h.handle.iter().map(|n| n.to_bits()).eq(key.1.iter().copied())
        }
        _ => false,
    })
}

/// Drops the entries of `made` whose objects the last collection freed.
pub(crate) fn forget_collected(made: &mut Made, heap: &crate::heap::Heap, classes: &[Entry]) {
    made.retain(|key, id| is_made(heap, classes, *id, key));
}

/// The [`Data`] that stands for the object of host class `class` with `handle`.
pub fn host_ref(class: &str, handle: &[f64]) -> Data {
    let mut list = Vec::with_capacity(handle.len() + 1);
    list.push(Data::String(class.to_string()));
    list.extend(handle.iter().map(|n| Data::Number(*n)));
    Data::Object(vec![(HOST_REF_KEY.to_string(), Data::Array(list))])
}

/// The class and handle of the object `d` stands for, when it is one [`host_ref`] made.
pub fn host_ref_parts(d: &Data) -> Option<(&str, Vec<f64>)> {
    let Data::Object(items) = d else { return None };
    let [(k, Data::Array(list))] = items.as_slice() else { return None };
    if k != HOST_REF_KEY {
        return None;
    }
    let (Data::String(class), rest) = list.split_first()? else { return None };
    let handle = rest.iter().map(|d| if let Data::Number(n) = d { Some(*n) } else { None }).collect::<Option<Vec<f64>>>()?;
    Some((class.as_str(), handle))
}

impl Interp {
    /// The [`Data`] for the object of a host class `id`, or `None` when it is not one.
    pub(crate) fn host_ref_of(&self, id: ObjId) -> JsResult<Option<Data>> {
        let Class::Host(h) = &self.object(id)?.class else { return Ok(None) };
        let name =
            self.host_classes.get(h.class as usize).map(|e| e.name.clone()).ok_or_else(|| Abrupt::Internal("host class out of range".into()))?;
        Ok(Some(host_ref(&name, &h.handle)))
    }

    /// The object of a host class that `d`, which a host function gave, stands for.
    pub(crate) fn write_host_ref(&mut self, d: &Data) -> JsResult<Value> {
        let Some((name, handle)) = host_ref_parts(d) else {
            return Err(self.throw(ErrorType::Type, "a host value names an object of a host class wrongly"));
        };
        let Some((class, entry)) = self.host_classes.iter().enumerate().find(|(_, e)| &*e.name == name) else {
            return Err(self.throw(ErrorType::Type, &format!("a host value names the unknown class {name}")));
        };
        let (proto, family) = (entry.proto, entry.family);
        let class = u32::try_from(class).unwrap_or(u32::MAX);
        let made = (family, handle.iter().map(|n| n.to_bits()).collect::<Box<[u64]>>());
        if let Some(&id) = self.host_objects.get(&made)
            && is_made(&self.heap, &self.host_classes, id, &made)
        {
            // An object of another class of the family becomes one of `class`.
            let o = self.object_mut(id)?;
            if let Class::Host(h) = &mut o.class
                && h.class != class
            {
                h.class = class;
                o.proto = Some(proto);
            }
            return Ok(Value::Object(id));
        }
        self.reserve_heap(crate::heap::OBJECT_BYTES)?;
        let mut o = Object::new(Class::Host(HostObj { class, handle: handle.into() }), Some(proto));
        o.extensible = false;
        let id = self.heap.alloc_object(o);
        self.host_objects.insert(made, id);
        Ok(Value::Object(id))
    }

    /// Whether `id` is the constructor of a host class (its other functions are not constructors).
    pub(crate) fn is_host_constructor(&self, id: ObjId) -> bool {
        self.host_classes.iter().any(|e| e.ctor == id)
    }

    /// The name of the host class of `id`, when it is an object of a host class.
    pub(crate) fn host_class_name(&self, id: ObjId) -> JsResult<Option<Rc<str>>> {
        let Class::Host(h) = &self.object(id)?.class else { return Ok(None) };
        Ok(self.host_classes.get(h.class as usize).map(|e| e.name.clone()))
    }

    /// The name and members of the host class of `id`, an object of the class or its constructor:
    /// the members of its parents first, then its own, a member a class declares again in the place
    /// of its parent's. Members marked [`MemberInfo::unavailable`] are left out. `None` when `id` is
    /// neither.
    pub(crate) fn host_class_members(&self, id: ObjId) -> JsResult<Option<(Rc<str>, Vec<Reflected>)>> {
        let class = match &self.object(id)?.class {
            Class::Host(h) => Some(h.class as usize),
            _ => self.host_classes.iter().position(|e| e.ctor == id),
        };
        let Some(class) = class else { return Ok(None) };
        let Some(entry) = self.host_classes.get(class) else { return Err(Abrupt::Internal("host class out of range".into())) };
        let mut chain = vec![entry];
        while let Some(p) = chain.last().and_then(|e| e.parent) {
            let Some(parent) = self.host_classes.get(p as usize) else { return Err(Abrupt::Internal("host class out of range".into())) };
            chain.push(parent);
        }
        let mut out: Vec<Reflected> = Vec::new();
        for e in chain.iter().rev() {
            for m in e.members.iter() {
                match out.iter_mut().find(|r| r.name == m.name) {
                    Some(slot) => *slot = m.clone(),
                    None => out.push(m.clone()),
                }
            }
        }
        out.retain(|m| !m.info.unavailable);
        Ok(Some((entry.name.clone(), out)))
    }

    /// The indexer of the host class of `id`, when it is an object of a host class that has one.
    pub(crate) fn host_indexer(&self, id: ObjId) -> JsResult<Option<HostFunction>> {
        let Class::Host(h) = &self.object(id)?.class else { return Ok(None) };
        Ok(self.host_classes.get(h.class as usize).and_then(|e| e.index.clone()))
    }

    /// `object[index]` of an object of a host class, which its class's indexer `f` answers.
    pub(crate) fn host_index(&mut self, f: &HostFunction, id: ObjId, index: u32) -> JsResult<Value> {
        self.use_fuel()?;
        let Some(this) = self.host_ref_of(id)? else { return Ok(Value::Undefined) };
        match f(vec![this, Data::Number(f64::from(index))]) {
            Ok(d) => self.write_hosted(d),
            Err(HostError::Wait) => Err(Abrupt::Internal("a host class's indexer waited for its result".into())),
            Err(e) => Err(self.host_error(e)?),
        }
    }

    /// [[HasProperty]] of `k` on `id`: for an array index of an object of a host class with an
    /// indexer, whether the indexer gives a value other than `undefined` for it.
    pub(crate) fn has_property_indexed(&mut self, id: ObjId, k: &[u16]) -> JsResult<bool> {
        if let Some(i) = crate::value::array_index(k)
            && let Some(f) = self.host_indexer(id)?
        {
            return Ok(!matches!(self.host_index(&f, id, i)?, Value::Undefined));
        }
        self.has_property(id, k)
    }

    /// `d` as a JavaScript value, the objects of host classes in it made from their [`host_ref`].
    pub(crate) fn write_hosted(&mut self, d: Data) -> JsResult<Value> {
        let was = std::mem::replace(&mut self.write_hosted, true);
        let r = self.write_data(d, 0, false);
        self.write_hosted = was;
        r
    }
}

/// Adds `classes` and the global objects `instances`, each made from the [`host_ref`] given.
pub(crate) fn install(i: &mut Interp, classes: Vec<HostClass>, instances: Vec<(String, Data)>) -> JsResult<()> {
    for class in classes {
        install_class(i, class)?;
    }
    for (name, d) in instances {
        let v = i.write_hosted(d)?;
        let global = i.global;
        i.define(global, key(&name), Prop::Data { value: v, flags: 0 })?;
    }
    Ok(())
}

fn install_class(i: &mut Interp, class: HostClass) -> JsResult<()> {
    if i.host_classes.iter().any(|e| *e.name == *class.name) {
        return Err(i.throw(ErrorType::Type, &format!("the host class {} is defined twice", class.name)));
    }
    let own = u32::try_from(i.host_classes.len()).unwrap_or(u32::MAX);
    let (parent, family, parent_index) = match &class.parent {
        None => (i.intrinsics.object_proto, own, None),
        Some(p) => match i.host_classes.iter().enumerate().find(|(_, e)| *e.name == **p) {
            Some((n, e)) => (e.proto, e.family, Some(u32::try_from(n).unwrap_or(u32::MAX))),
            None => return Err(i.throw(ErrorType::Type, &format!("the host class {p} is not defined"))),
        },
    };
    let members = match reflected_members(&class) {
        Ok(m) => m,
        Err(message) => return Err(i.throw(ErrorType::Type, &message)),
    };
    let proto = i.heap.alloc_object(Object::new(Class::Ordinary, Some(parent)));
    let mark = i.temp_roots.len();
    i.temp_roots.push(Value::Object(proto));
    let r = fill_class(i, proto, &class);
    i.temp_roots.truncate(mark);
    let ctor = r?;
    i.host_classes.push(Entry { name: class.name.as_str().into(), proto, ctor, index: class.index, family, parent: parent_index, members });
    let global = i.global;
    i.define(global, key(&class.name), Prop::Data { value: Value::Object(ctor), flags: crate::heap::HIDDEN })
}

/// The members of `class` as `reflect` lists them, each with its [`HostClass::info`] entry. → the
/// error's message when a member is named twice, or an entry names no member or is given twice.
fn reflected_members(class: &HostClass) -> Result<Rc<[Reflected]>, String> {
    let mut out: Vec<Reflected> = Vec::with_capacity(class.members.len());
    for (name, member) in &class.members {
        if out.iter().any(|r| *r.name == **name) {
            return Err(format!("the host class {} names the member {name} twice", class.name));
        }
        // The prototype's `constructor` is the class's own constructor.
        if name == "constructor" {
            return Err(format!("the host class {} names a member constructor, which is the name of its constructor", class.name));
        }
        let access = match member {
            HostMember::Get(_) => Access::ReadOnly,
            HostMember::Property { .. } => Access::ReadWrite,
            HostMember::Method(_) => Access::Method,
        };
        out.push(Reflected { name: name.as_str().into(), access, info: MemberInfo::default() });
    }
    let mut seen: Vec<&str> = Vec::with_capacity(class.info.len());
    for (name, info) in &class.info {
        if seen.contains(&name.as_str()) {
            return Err(format!("the host class {} describes the member {name} twice", class.name));
        }
        seen.push(name);
        let Some(slot) = out.iter_mut().find(|r| *r.name == **name) else {
            return Err(format!("the host class {} describes {name}, which is not one of its members", class.name));
        };
        slot.info = info.clone();
    }
    Ok(out.into())
}

/// Puts the members of `class` on `proto` and makes its constructor. → the constructor.
fn fill_class(i: &mut Interp, proto: ObjId, class: &HostClass) -> JsResult<ObjId> {
    let path = &class.name;
    for (name, member) in &class.members {
        let full = format!("{path}.{name}");
        let prop = match member {
            HostMember::Get(f) => Prop::Accessor { get: Some(hosted_function(i, f.clone(), &full)), set: None, flags: 0 },
            HostMember::Property { get, set } => {
                let get = hosted_function(i, get.clone(), &full);
                i.temp_roots.push(Value::Object(get));
                Prop::Accessor { get: Some(get), set: Some(hosted_function(i, set.clone(), &full)), flags: 0 }
            }
            HostMember::Method(f) => Prop::Data { value: Value::Object(hosted_function(i, f.clone(), &full)), flags: 0 },
        };
        i.define(proto, key(name), prop)?;
    }
    let construct = class.construct.clone().unwrap_or_else(|| {
        let message = format!("{path} cannot be constructed by a script");
        Rc::new(move |_| Err(HostError::Type(message.clone())))
    });
    let ctor = hosted_function(i, construct, path);
    i.define(ctor, key("prototype"), Prop::Data { value: Value::Object(proto), flags: 0 })?;
    i.define(proto, key("constructor"), Prop::Data { value: Value::Object(ctor), flags: 0 })?;
    i.object_mut(proto)?.extensible = false;
    i.object_mut(ctor)?.extensible = false;
    Ok(ctor)
}

fn hosted_function(i: &mut Interp, f: HostFunction, name: &str) -> ObjId {
    let proto = i.intrinsics.function_proto;
    i.heap.alloc_object(Object::new(Class::Function(Func::Data { f, name: name.into(), read: ArgRead::Hosted }), Some(proto)))
}
