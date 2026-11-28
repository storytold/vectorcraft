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
//! array index `i` with it, called with `this` and the index as a number. Other names are looked up
//! as usual, and a name that the object and its prototypes lack goes to the indexer as a string (as
//! a number when it is the text of one, such as `-1`), so the indexer may throw for a name nothing
//! has. Names the engine looks up itself, such as `toJSON` in `JSON.stringify`, do not go to the
//! indexer. Assigning to such a name reads it with the indexer first, so the assignment throws what
//! the read throws; otherwise the value is not kept. `k in object` calls the indexer with `true` as
//! a third argument, and is true when it gives a value other than `undefined`.
//!
//! `hasOwnProperty` and `propertyIsEnumerable` are true on an object of a host class for each
//! member of its class and of the classes it descends from, apart from the members that are not
//! available yet ([`HostMember::Unavailable`]). A member a class declares, available or not, hides
//! the member of that name in the classes it descends from. `for`-`in` lists the properties among those members,
//! and not the methods. `Object.keys`, `Object.getOwnPropertyNames` and
//! `Object.getOwnPropertyDescriptor` do not report the members.

use std::collections::HashMap;
use std::rc::Rc;

use crate::heap::{CONFIGURABLE, Class, ENUMERABLE, Func, Key, ObjId, Object, Prop, key};
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
    /// A property the class lists that is not available yet: the function answers both a read and
    /// an assignment, with an error. It is not reported as the object's own, and `for`-`in` skips it.
    Unavailable(HostFunction),
}

/// A class [`crate::Realm::define_host_classes`] adds.
#[derive(Clone)]
pub struct HostClass {
    pub name: String,
    /// A class defined before this one, or in the same call before it, whose prototype this
    /// class's prototype inherits from.
    pub parent: Option<String>,
    pub members: Vec<(String, HostMember)>,
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
    /// The class this class's prototype inherits from, as an index into the realm's classes.
    parent: Option<u32>,
    /// The names of the members the class declares, each with whether `hasOwnProperty` reports it:
    /// every member but the unavailable ones.
    own: Rc<HashMap<Key, bool>>,
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
    /// Whether `k` is a member that `hasOwnProperty` reports on `id`: `id` is an object of a host
    /// class, and the closest of that class and the classes it descends from that declares `k`
    /// declares it as available.
    pub(crate) fn is_host_member(&self, id: ObjId, k: &[u16]) -> JsResult<bool> {
        let Class::Host(h) = &self.object(id)?.class else { return Ok(false) };
        let mut class = Some(h.class);
        while let Some(c) = class {
            let Some(e) = self.host_classes.get(c as usize) else { return Err(Abrupt::Internal("host class out of range".into())) };
            if let Some(available) = e.own.get(k) {
                return Ok(*available);
            }
            class = e.parent;
        }
        Ok(false)
    }

    /// The [`Data`] for the object of a host class `id`, or `None` when it is not one.
    pub(crate) fn host_ref_of(&self, id: ObjId) -> JsResult<Option<Data>> {
        let Class::Host(h) = &self.object(id)?.class else { return Ok(None) };
        let name =
            self.host_classes.get(h.class as usize).map(|e| e.name.clone()).ok_or_else(|| Abrupt::Internal("host class out of range".into()))?;
        Ok(Some(host_ref(&name, &h.handle)))
    }

    pub(crate) fn host_ref_size(&self, id: ObjId) -> JsResult<Option<(usize, usize)>> {
        let Class::Host(h) = &self.object(id)?.class else { return Ok(None) };
        let name = self.host_classes.get(h.class as usize).map(|e| e.name.len()).ok_or_else(|| Abrupt::Internal("host class out of range".into()))?;
        Ok(Some((name, h.handle.len())))
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

    /// The indexer of the host class of `id`, when it is an object of a host class that has one.
    pub(crate) fn host_indexer(&self, id: ObjId) -> JsResult<Option<HostFunction>> {
        let Class::Host(h) = &self.object(id)?.class else { return Ok(None) };
        Ok(self.host_classes.get(h.class as usize).and_then(|e| e.index.clone()))
    }

    /// `object[k]` of an object of a host class, which its class's indexer `f` answers: `k` is an
    /// array index as a number, or a name as a string. With `probe`, for `k in object`, the indexer
    /// gets `true` as a third argument.
    pub(crate) fn host_index(&mut self, f: &HostFunction, id: ObjId, k: Data, probe: bool) -> JsResult<Value> {
        self.use_fuel()?;
        if self.host_ref_size(id)?.is_none() {
            return Ok(Value::Undefined);
        }
        let args = self.read_index_args(id, k, probe)?;
        match f(args) {
            Ok(d) => self.write_hosted(d),
            Err(HostError::Wait) => Err(Abrupt::Internal("a host class's indexer waited for its result".into())),
            Err(e) => Err(self.host_error(e)?),
        }
    }

    /// The key an indexer gets for `k`: a number for an array index and for any other key that is
    /// the text of a number (`-1`, `1.5`, `NaN`), otherwise the name.
    pub(crate) fn indexer_key(k: &[u16]) -> Data {
        if let Some(i) = crate::value::array_index(k) {
            return Data::Number(f64::from(i));
        }
        let name = String::from_utf16_lossy(k);
        match name.parse::<f64>() {
            Ok(n) if crate::number_to_string(n) == name => Data::Number(n),
            _ => Data::String(name),
        }
    }

    /// [[HasProperty]] of `k` on `id`: for an object of a host class with an indexer, an array index,
    /// or a name that the object and its prototypes lack, is there when the indexer gives a value other
    /// than `undefined` for it.
    pub(crate) fn has_property_indexed(&mut self, id: ObjId, k: &[u16]) -> JsResult<bool> {
        let index = crate::value::array_index(k).is_some();
        if (index || !self.has_property(id, k)?)
            && let Some(f) = self.host_indexer(id)?
        {
            return Ok(!matches!(self.host_index(&f, id, Self::indexer_key(k), true)?, Value::Undefined));
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

/// Adds the global properties `globals`, each read with its getter and assigned with its setter as
/// a member of a host class is. Assigning one without a setter keeps its value, and throws in strict
/// mode code.
pub(crate) fn install_globals(i: &mut Interp, globals: Vec<(String, HostFunction, Option<HostFunction>)>) -> JsResult<()> {
    let global = i.global;
    for (name, get, set) in globals {
        let mark = i.temp_roots.len();
        let get = hosted_function(i, get, &name);
        i.temp_roots.push(Value::Object(get));
        let set = set.map(|f| hosted_function(i, f, &name));
        i.temp_roots.truncate(mark);
        i.define(global, key(&name), Prop::Accessor { get: Some(get), set, flags: CONFIGURABLE })?;
    }
    Ok(())
}

fn install_class(i: &mut Interp, class: HostClass) -> JsResult<()> {
    if i.host_classes.iter().any(|e| *e.name == *class.name) {
        return Err(i.throw(ErrorType::Type, &format!("the host class {} is defined twice", class.name)));
    }
    let own = u32::try_from(i.host_classes.len()).unwrap_or(u32::MAX);
    let (parent, family, parent_class) = match &class.parent {
        None => (i.intrinsics.object_proto, own, None),
        Some(p) => match i.host_classes.iter().enumerate().find(|(_, e)| *e.name == **p) {
            Some((n, e)) => (e.proto, e.family, u32::try_from(n).ok()),
            None => return Err(i.throw(ErrorType::Type, &format!("the host class {p} is not defined"))),
        },
    };
    let proto = i.heap.alloc_object(Object::new(Class::Ordinary, Some(parent)));
    let mark = i.temp_roots.len();
    i.temp_roots.push(Value::Object(proto));
    let r = fill_class(i, proto, &class);
    i.temp_roots.truncate(mark);
    let ctor = r?;
    let members = class.members.iter().map(|(name, m)| (key(name), !matches!(m, HostMember::Unavailable(_)))).collect();
    i.host_classes.push(Entry {
        name: class.name.as_str().into(),
        proto,
        ctor,
        index: class.index,
        family,
        parent: parent_class,
        own: Rc::new(members),
    });
    let global = i.global;
    i.define(global, key(&class.name), Prop::Data { value: Value::Object(ctor), flags: crate::heap::HIDDEN })
}

/// Puts the members of `class` on `proto` and makes its constructor. → the constructor.
fn fill_class(i: &mut Interp, proto: ObjId, class: &HostClass) -> JsResult<ObjId> {
    let path = &class.name;
    for (name, member) in &class.members {
        let full = format!("{path}.{name}");
        let prop = match member {
            HostMember::Get(f) => Prop::Accessor { get: Some(hosted_function(i, f.clone(), &full)), set: None, flags: ENUMERABLE },
            HostMember::Property { get, set } => {
                let get = hosted_function(i, get.clone(), &full);
                i.temp_roots.push(Value::Object(get));
                Prop::Accessor { get: Some(get), set: Some(hosted_function(i, set.clone(), &full)), flags: ENUMERABLE }
            }
            HostMember::Method(f) => Prop::Data { value: Value::Object(hosted_function(i, f.clone(), &full)), flags: 0 },
            HostMember::Unavailable(f) => {
                let f = hosted_function(i, f.clone(), &full);
                Prop::Accessor { get: Some(f), set: Some(f), flags: 0 }
            }
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
