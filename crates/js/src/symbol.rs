//! Symbols (ECMA-262 11th edition, section 6.1.5, <https://262.ecma-international.org/11.0/>) and
//! the properties they name.
//!
//! A Symbol is a value with its own identity and an optional description; two Symbols are equal
//! only when they are the same Symbol. An object keeps its Symbol-keyed properties apart from its
//! string-keyed ones, in the order they were added, so `for-in`, `Object.keys`,
//! `Object.getOwnPropertyNames` and `JSON` see only names, and `Object.getOwnPropertySymbols` sees
//! only Symbols. `Symbol.for`'s registry belongs to the realm and lives as long as it does; each
//! entry counts against the heap limit. A realm without [`crate::Options::es2015`] makes no Symbol,
//! so its scripts run as before.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use crate::heap::{ObjId, PROP_BYTES, Prop, WRITABLE};
use crate::interp::{Desc, Hint, Interp, JsResult, merge_desc};
use crate::value::{Str, Value};

/// A Symbol value. Clones share the Symbol; equality and hashing compare identity.
#[derive(Clone)]
pub(crate) struct Sym(Rc<SymData>);

struct SymData {
    description: Option<Str>,
    /// `Symbol.for` made it, and the registry holds it under its description.
    registered: bool,
}

impl Sym {
    pub(crate) fn new(description: Option<Str>, registered: bool) -> Self {
        Sym(Rc::new(SymData { description, registered }))
    }

    pub(crate) fn description(&self) -> Option<&Str> {
        self.0.description.as_ref()
    }

    pub(crate) fn registered(&self) -> bool {
        self.0.registered
    }

    /// An address that identifies the Symbol, for counting it once.
    pub(crate) fn addr(&self) -> usize {
        Rc::as_ptr(&self.0) as *const () as usize
    }

    /// `Symbol(description)` for a message, with at most [`NAMED_UNITS`] units of the description
    /// followed by `...` when it has more, and the work reading them took: the rope nodes visited
    /// plus the units read. The description is not flattened or copied whole.
    pub(crate) fn message_name(&self) -> (Vec<u16>, u64) {
        let mut units: Vec<u16> = "Symbol(".encode_utf16().collect();
        let mut work = 0;
        if let Some(d) = self.description() {
            let (part, visited) = d.prefix(NAMED_UNITS, NAMED_NODES);
            work = visited.saturating_add(part.len() as u64);
            let cut = part.len() < d.len();
            units.extend_from_slice(&part);
            if cut {
                units.extend("...".encode_utf16());
            }
        }
        units.push(u16::from(b')'));
        (units, work)
    }
}

/// The most units of a Symbol's description a message names.
const NAMED_UNITS: usize = 256;

/// The most rope nodes read for the units a message names.
const NAMED_NODES: u64 = 4 * NAMED_UNITS as u64;

impl PartialEq for Sym {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Sym {}

impl Hash for Sym {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.addr().hash(state);
    }
}

impl std::fmt::Debug for Sym {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", String::from_utf16_lossy(&self.message_name().0))
    }
}

/// An object's Symbol-keyed properties in the order they were added. A removed entry leaves a
/// gap until half the entries are gaps, so removing keeps the order without moving the rest.
#[derive(Clone, Default)]
pub(crate) struct SymProps {
    entries: Vec<Option<(Sym, Prop)>>,
    index: HashMap<Sym, usize>,
}

impl SymProps {
    pub(crate) fn get(&self, k: &Sym) -> Option<&Prop> {
        let i = *self.index.get(k)?;
        self.entries.get(i)?.as_ref().map(|(_, p)| p)
    }

    pub(crate) fn get_mut(&mut self, k: &Sym) -> Option<&mut Prop> {
        let i = *self.index.get(k)?;
        self.entries.get_mut(i)?.as_mut().map(|(_, p)| p)
    }

    /// Sets the property under `k`, keeping its place when it is there already. Returns whether
    /// the key is new.
    pub(crate) fn insert(&mut self, k: Sym, prop: Prop) -> bool {
        if let Some(slot) = self.get_mut(&k) {
            *slot = prop;
            return false;
        }
        self.index.insert(k.clone(), self.entries.len());
        self.entries.push(Some((k, prop)));
        true
    }

    pub(crate) fn remove(&mut self, k: &Sym) -> Option<Prop> {
        let i = self.index.remove(k)?;
        let (_, prop) = self.entries.get_mut(i)?.take()?;
        if self.entries.len() > 16 && self.index.len() * 2 < self.entries.len() {
            self.entries.retain(Option::is_some);
            self.entries.shrink_to_fit();
            self.index = self.entries.iter().enumerate().filter_map(|(i, e)| e.as_ref().map(|(k, _)| (k.clone(), i))).collect();
        }
        Some(prop)
    }

    pub(crate) fn len(&self) -> usize {
        self.index.len()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&Sym, &Prop)> {
        self.entries.iter().filter_map(|e| e.as_ref().map(|(k, p)| (k, p)))
    }
}

/// A property key (section 6.1.7): a name or a Symbol.
pub(crate) enum PropKey {
    Str(crate::heap::Key),
    Sym(Sym),
}

/// The well-known Symbols a realm has (section 6.1.5.1): `Symbol.iterator` and the others the
/// language reads, with `Symbol.asyncIterator`, which nothing reads yet.
pub(crate) struct WellKnown {
    pub(crate) iterator: Sym,
    pub(crate) async_iterator: Sym,
    pub(crate) has_instance: Sym,
    pub(crate) to_primitive: Sym,
    pub(crate) to_string_tag: Sym,
    /// The five as values, for the collector.
    values: Vec<Value>,
}

impl WellKnown {
    pub(crate) fn new() -> Self {
        let make = |name: &str| Sym::new(Some(Str::from(name)), false);
        let (iterator, async_iterator, has_instance, to_primitive, to_string_tag) = (
            make("Symbol.iterator"),
            make("Symbol.asyncIterator"),
            make("Symbol.hasInstance"),
            make("Symbol.toPrimitive"),
            make("Symbol.toStringTag"),
        );
        let values = [&iterator, &async_iterator, &has_instance, &to_primitive, &to_string_tag].into_iter().cloned().map(Value::Symbol).collect();
        WellKnown { iterator, async_iterator, has_instance, to_primitive, to_string_tag, values }
    }
}

/// The objects of a realm with [`crate::Options::es2015`] that the interpreter itself uses.
#[derive(Clone, Copy)]
pub(crate) struct Es2015Objects {
    pub(crate) symbol_proto: ObjId,
    pub(crate) array_iterator_proto: ObjId,
    pub(crate) string_iterator_proto: ObjId,
    /// `%ArrayProto_values%`, the `@@iterator` of every `arguments` object.
    pub(crate) array_values: ObjId,
    /// `Function.prototype[@@hasInstance]`.
    pub(crate) has_instance: ObjId,
    /// `%GeneratorPrototype%`, which each generator function's `prototype` inherits from.
    pub(crate) generator_proto: ObjId,
    /// `%GeneratorFunction.prototype%`, the `[[Prototype]]` of every generator function.
    pub(crate) generator_function_proto: ObjId,
}

/// The estimated bytes of a registry entry without its Symbol.
pub(crate) const REGISTRY_ENTRY_BYTES: usize = 48;

/// A realm's Symbols: the well-known ones, `Symbol.for`'s registry and the built-in objects that
/// go with them. All are `None` or empty in a realm without [`crate::Options::es2015`].
#[derive(Default)]
pub(crate) struct Symbols {
    pub(crate) well_known: Option<WellKnown>,
    /// Each key `Symbol.for` was given, with its Symbol as a value.
    pub(crate) registry: HashMap<Rc<[u16]>, Value>,
    pub(crate) objects: Option<Es2015Objects>,
}

impl Symbols {
    /// The values the collector starts from.
    pub(crate) fn root_values(&self) -> impl Iterator<Item = &Value> {
        self.well_known.iter().flat_map(|w| w.values.iter()).chain(self.registry.values())
    }

    /// The objects the collector starts from.
    pub(crate) fn root_objects(&self) -> Vec<ObjId> {
        self.objects.map_or_else(Vec::new, |o| {
            vec![
                o.symbol_proto,
                o.array_iterator_proto,
                o.string_iterator_proto,
                o.array_values,
                o.has_instance,
                o.generator_proto,
                o.generator_function_proto,
            ]
        })
    }

    /// The bytes the registry's entries hold besides their Symbols, which a collection adds.
    pub(crate) fn registry_bytes(&self) -> usize {
        self.registry.len().saturating_mul(REGISTRY_ENTRY_BYTES)
    }
}

impl Interp {
    /// ToPropertyKey (section 7.1.14): a Symbol, or an object whose primitive value is one, stays a
    /// Symbol; any other value becomes a name.
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn to_prop_key(&mut self, v: &Value) -> JsResult<PropKey> {
        match v {
            Value::Symbol(s) => Ok(PropKey::Sym(s.clone())),
            Value::Object(_) => match self.to_primitive(v, Hint::String)? {
                Value::Symbol(s) => Ok(PropKey::Sym(s)),
                p => self.to_key(&p).map(PropKey::Str),
            },
            _ => self.to_key(v).map(PropKey::Str),
        }
    }

    /// Runs `f` with the key [`Self::to_prop_key`] makes from `v`, kept reachable while `f` runs
    /// when it was converted from another value.
    pub(crate) fn with_prop_key<T>(&mut self, v: &Value, f: impl FnOnce(&mut Self, PropKey) -> JsResult<T>) -> JsResult<T> {
        match v {
            Value::Symbol(s) => f(self, PropKey::Sym(s.clone())),
            Value::Object(_) => {
                let p = self.to_primitive(v, Hint::String)?;
                self.rooted(std::slice::from_ref(&p), |i| match &p {
                    Value::Symbol(s) => f(i, PropKey::Sym(s.clone())),
                    _ => i.with_key(&p, |i, k| f(i, PropKey::Str(k))),
                })
            }
            _ => self.with_key(v, |i, k| f(i, PropKey::Str(k))),
        }
    }

    /// [`Sym::message_name`] as code units, its reading charged as fuel.
    pub(crate) fn sym_name_units(&mut self, k: &Sym) -> JsResult<Vec<u16>> {
        let (units, work) = k.message_name();
        crate::regexp_vm::use_fuel_n(self, work.max(1))?;
        Ok(units)
    }

    /// [`Sym::message_name`] as text, its reading charged as fuel.
    pub(crate) fn sym_name(&mut self, k: &Sym) -> JsResult<String> {
        self.sym_name_units(k).map(|u| String::from_utf16_lossy(&u))
    }

    /// `prefix`, the units of `body` and `suffix` as a new string. Flattening `body` and building
    /// the string are charged as fuel and checked against the heap limit first.
    pub(crate) fn wrap_str(&mut self, prefix: &str, body: Option<&Str>, suffix: &str) -> JsResult<Value> {
        let body = match body {
            Some(b) => crate::string_builtins::flat_charged(self, b)?,
            None => Rc::from([]),
        };
        let (prefix, suffix): (Vec<u16>, Vec<u16>) = (prefix.encode_utf16().collect(), suffix.encode_utf16().collect());
        let len = prefix.len().saturating_add(body.len()).saturating_add(suffix.len());
        crate::string_builtins::reserve_units(self, len)?;
        let mut units = Vec::with_capacity(len);
        units.extend_from_slice(&prefix);
        units.extend_from_slice(&body);
        units.extend_from_slice(&suffix);
        Ok(Value::String(Str::from_units(units)))
    }

    /// SymbolDescriptiveString (section 19.4.3.3.1): `Symbol(description)` as a new string.
    pub(crate) fn descriptive_string(&mut self, k: &Sym) -> JsResult<Value> {
        let roots = [Value::Symbol(k.clone())];
        self.rooted(&roots, |i| i.wrap_str("Symbol(", k.description(), ")"))
    }

    /// A `TypeError` for a property `k` that cannot be defined, when `throw`; otherwise `false`.
    fn reject_sym_bool(&mut self, throw: bool, k: &Sym, why: &str) -> JsResult<bool> {
        if !throw {
            return Ok(false);
        }
        let name = self.sym_name_units(k)?;
        self.reject_bool(true, &name, why)
    }

    /// A `TypeError` for a property `k` that cannot be set, when `throw`.
    fn reject_sym(&mut self, throw: bool, k: &Sym, why: &str) -> JsResult<()> {
        if !throw {
            return Ok(());
        }
        let name = self.sym_name_units(k)?;
        self.reject(true, &name, why)
    }

    pub(crate) fn get_own_sym(&self, id: ObjId, k: &Sym) -> JsResult<Option<Prop>> {
        Ok(self.object(id)?.syms.as_ref().and_then(|m| m.get(k)).cloned())
    }

    /// The property `k` found on the object or its prototype chain. Each step to a prototype takes
    /// a unit of fuel.
    pub(crate) fn find_sym(&mut self, id: ObjId, k: &Sym) -> JsResult<Option<Prop>> {
        let mut cur = Some(id);
        let mut steps = 0usize;
        while let Some(o) = cur {
            if let Some(p) = self.get_own_sym(o, k)? {
                return Ok(Some(p));
            }
            cur = self.object(o)?.proto;
            if cur.is_some() {
                self.use_fuel()?;
            }
            steps += 1;
            if steps > 1_000_000 {
                return Err(crate::interp::Abrupt::Internal("prototype chain too long".into()));
            }
        }
        Ok(None)
    }

    /// [[Get]] of `k` starting at `start`, with `receiver` as `this` for a getter.
    pub(crate) fn get_sym_from(&mut self, start: ObjId, k: &Sym, receiver: &Value) -> JsResult<Value> {
        match self.find_sym(start, k)? {
            None | Some(Prop::Accessor { get: None, .. }) => Ok(Value::Undefined),
            Some(Prop::Data { value, .. }) => Ok(value),
            Some(Prop::Accessor { get: Some(g), .. }) => self.call(&Value::Object(g), receiver, &[]),
        }
    }

    /// `base[k]` for any value.
    pub(crate) fn get_value_sym(&mut self, base: &Value, k: &Sym) -> JsResult<Value> {
        if base.is_nullish() {
            let (key, base) = (self.sym_name(k)?, if matches!(base, Value::Null) { "null" } else { "undefined" });
            return Err(self.type_error(&format!("cannot read property '{key}' of {base}")));
        }
        let start = self.primitive_proto(base, &[])?;
        self.get_sym_from(start, k, base)
    }

    /// Adds or replaces an own property directly, keeping the heap estimate.
    pub(crate) fn define_sym(&mut self, id: ObjId, k: Sym, prop: Prop) -> JsResult<()> {
        let o = self.object_mut(id)?;
        if o.syms.get_or_insert_with(Box::default).insert(k, prop) {
            self.heap.bytes = self.heap.bytes.saturating_add(PROP_BYTES);
        }
        Ok(())
    }

    /// The default [[DefineOwnProperty]] (section 9.1.6) for a Symbol key.
    pub(crate) fn define_own_sym(&mut self, id: ObjId, k: Sym, desc: &Desc, throw: bool) -> JsResult<bool> {
        let Some(current) = self.get_own_sym(id, &k)? else {
            if !self.object(id)?.extensible {
                return self.reject_sym_bool(throw, &k, "the object is not extensible");
            }
            let mut roots = vec![Value::Object(id), Value::Symbol(k.clone())];
            roots.extend(desc.value.iter().cloned());
            roots.extend(desc.get.flatten().into_iter().chain(desc.set.flatten()).map(Value::Object));
            self.rooted(&roots, |i| i.reserve_heap(PROP_BYTES))?;
            let flags = desc.flags_or(0);
            let prop = if desc.is_accessor() {
                Prop::Accessor { get: desc.get.flatten(), set: desc.set.flatten(), flags: flags & !WRITABLE }
            } else {
                Prop::Data { value: desc.value.clone().unwrap_or(Value::Undefined), flags }
            };
            self.define_sym(id, k, prop)?;
            return Ok(true);
        };
        let Some(merged) = merge_desc(&current, desc) else { return self.reject_sym_bool(throw, &k, "the property cannot be redefined") };
        self.define_sym(id, k, merged)?;
        Ok(true)
    }

    /// [[Set]] (section 9.1.9) of an ordinary object for a Symbol key.
    pub(crate) fn put_sym(&mut self, id: ObjId, k: Sym, v: Value, throw: bool) -> JsResult<()> {
        match self.get_own_sym(id, &k)? {
            Some(Prop::Data { flags, .. }) => {
                if flags & WRITABLE == 0 {
                    return self.reject_sym(throw, &k, "it is read-only");
                }
                if let Some(Some(Prop::Data { value, .. })) = self.object_mut(id)?.syms.as_mut().map(|m| m.get_mut(&k)) {
                    *value = v;
                }
                return Ok(());
            }
            Some(Prop::Accessor { set: Some(s), .. }) => return self.call(&Value::Object(s), &Value::Object(id), &[v]).map(drop),
            Some(Prop::Accessor { set: None, .. }) => return self.reject_sym(throw, &k, "it has only a getter"),
            None => {}
        }
        let proto = self.object(id)?.proto;
        match proto.map(|p| self.find_sym(p, &k)).transpose()?.flatten() {
            Some(Prop::Accessor { set: Some(s), .. }) => return self.call(&Value::Object(s), &Value::Object(id), &[v]).map(drop),
            Some(Prop::Accessor { set: None, .. }) => return self.reject_sym(throw, &k, "it has only a getter"),
            Some(Prop::Data { flags, .. }) if flags & WRITABLE == 0 => return self.reject_sym(throw, &k, "it is read-only"),
            _ => {}
        }
        if !self.object(id)?.extensible {
            return self.reject_sym(throw, &k, "the object is not extensible");
        }
        let roots = [Value::Object(id), Value::Symbol(k.clone()), v.clone()];
        self.rooted(&roots, |i| i.reserve_heap(PROP_BYTES))?;
        self.define_sym(id, k, Prop::Data { value: v, flags: crate::heap::OPEN })
    }

    /// PutValue (section 6.2.4.9) on a Symbol-keyed reference, whose base may be a primitive.
    pub(crate) fn put_value_sym(&mut self, base: &Value, k: Sym, v: Value) -> JsResult<()> {
        match base {
            Value::Object(o) => {
                let throw = self.strict_writes();
                self.put_sym(*o, k, v, throw)
            }
            Value::Undefined | Value::Null => {
                let (key, base) = (self.sym_name(&k)?, if matches!(base, Value::Null) { "null" } else { "undefined" });
                Err(self.type_error(&format!("cannot set property '{key}' of {base}")))
            }
            _ => {
                let proto = self.primitive_proto(base, &[])?;
                if let Some(Prop::Accessor { set: Some(s), .. }) = self.find_sym(proto, &k)? {
                    return self.call(&Value::Object(s), base, &[v]).map(drop);
                }
                let strict = self.strict();
                self.reject_sym(strict, &k, "the value is a primitive")
            }
        }
    }

    /// [[Delete]] (section 9.1.10) for a Symbol key.
    pub(crate) fn delete_sym(&mut self, id: ObjId, k: &Sym, throw: bool) -> JsResult<bool> {
        match self.get_own_sym(id, k)? {
            None => Ok(true),
            Some(p) if p.configurable() => {
                if let Some(m) = self.object_mut(id)?.syms.as_mut() {
                    m.remove(k);
                }
                Ok(true)
            }
            Some(_) if throw => {
                let key = self.sym_name(k)?;
                Err(self.type_error(&format!("cannot delete property '{key}'")))
            }
            Some(_) => Ok(false),
        }
    }

    /// An object's own Symbol keys in the order they were added. Each takes a unit of fuel, and
    /// the heap for the list is checked first.
    pub(crate) fn own_symbols(&mut self, id: ObjId) -> JsResult<Vec<Sym>> {
        let count = self.object(id)?.syms.as_ref().map_or(0, |m| m.len());
        self.reserve_heap(count.saturating_mul(16))?;
        for _ in 0..count {
            self.use_fuel()?;
        }
        Ok(self.object(id)?.syms.iter().flat_map(|m| m.iter()).map(|(k, _)| k.clone()).collect())
    }

    // ---- either kind of key ----

    pub(crate) fn get_own_pk(&self, id: ObjId, k: &PropKey) -> JsResult<Option<Prop>> {
        match k {
            PropKey::Str(name) => self.get_own_property(id, name),
            PropKey::Sym(s) => self.get_own_sym(id, s),
        }
    }

    pub(crate) fn has_pk(&mut self, id: ObjId, k: &PropKey) -> JsResult<bool> {
        match k {
            PropKey::Str(name) => self.has_property(id, name),
            PropKey::Sym(s) => Ok(self.find_sym(id, s)?.is_some()),
        }
    }

    pub(crate) fn define_own_pk(&mut self, id: ObjId, k: PropKey, desc: &Desc, throw: bool) -> JsResult<bool> {
        match k {
            PropKey::Str(name) => self.define_own_property(id, name, desc, throw),
            PropKey::Sym(s) => self.define_own_sym(id, s, desc, throw),
        }
    }

    pub(crate) fn get_value_pk(&mut self, base: &Value, k: &PropKey) -> JsResult<Value> {
        match k {
            PropKey::Str(name) => self.get_value(base, name),
            PropKey::Sym(s) => self.get_value_sym(base, s),
        }
    }

    pub(crate) fn put_value_pk(&mut self, base: &Value, k: PropKey, v: Value) -> JsResult<()> {
        match k {
            PropKey::Str(name) => self.put_value(base, name, v),
            PropKey::Sym(s) => self.put_value_sym(base, s, v),
        }
    }

    pub(crate) fn delete_pk(&mut self, id: ObjId, k: &PropKey, throw: bool) -> JsResult<bool> {
        match k {
            PropKey::Str(name) => self.delete(id, name, throw),
            PropKey::Sym(s) => self.delete_sym(id, s, throw),
        }
    }

    // ---- the hooks the well-known Symbols name ----

    /// The well-known Symbol `pick` chooses, in a realm that has them.
    pub(crate) fn well_known(&self, pick: fn(&WellKnown) -> &Sym) -> Option<Sym> {
        self.symbols.well_known.as_ref().map(|w| pick(w).clone())
    }

    /// Steps 2 and 3 of ToPrimitive (section 7.1.1): the result of the object's
    /// `@@toPrimitive` method called with the hint, or `None` when it has none. Code run without
    /// [`crate::Options::es2015`] reads no such method, even in a realm that has the well-known
    /// Symbols from an earlier run.
    pub(crate) fn exotic_to_primitive(&mut self, o: ObjId, v: &Value, hint: Hint) -> JsResult<Option<Value>> {
        if !self.options.es2015 {
            return Ok(None);
        }
        let Some(sym) = self.well_known(|w| &w.to_primitive) else { return Ok(None) };
        let f = self.get_sym_from(o, &sym, v)?;
        if f.is_nullish() {
            return Ok(None);
        }
        if !self.is_callable(&f) {
            return Err(self.type_error("the value is not a function"));
        }
        let hint = match hint {
            Hint::Default => "default",
            Hint::Number => "number",
            Hint::String => "string",
        };
        let r = self.call(&f, v, &[Value::str(hint)])?;
        if matches!(r, Value::Object(_)) {
            return Err(self.type_error("cannot convert the object to a primitive value"));
        }
        Ok(Some(r))
    }

    /// Steps 2 to 4 of InstanceofOperator (section 12.10.4): the answer of `f`'s
    /// `@@hasInstance` method, or `None` when `f` is not an object or has none. Code run without
    /// [`crate::Options::es2015`] reads no such method, so `instanceof` keeps the ECMAScript 5.1
    /// check that the right side is a function.
    pub(crate) fn has_instance_hook(&mut self, v: &Value, f: &Value) -> JsResult<Option<bool>> {
        if !self.options.es2015 {
            return Ok(None);
        }
        let Some(sym) = self.well_known(|w| &w.has_instance) else { return Ok(None) };
        let Value::Object(fid) = f else { return Ok(None) };
        let handler = self.get_sym_from(*fid, &sym, f)?;
        if handler.is_nullish() {
            return Ok(None);
        }
        if !self.is_callable(&handler) {
            return Err(self.type_error("the value is not a function"));
        }
        Ok(Some(self.call(&handler, f, std::slice::from_ref(v))?.truthy()))
    }

    /// For the target of a bound function in OrdinaryHasInstance (section 7.3.21, step 2): the
    /// answer of its own `@@hasInstance` method when that is not `Function.prototype`'s, whose
    /// answer is the one the caller goes on to give.
    pub(crate) fn bound_target_hook(&mut self, v: &Value, target: ObjId) -> JsResult<Option<bool>> {
        let default = self.symbols.objects.map(|o| o.has_instance);
        match self.has_instance_handler(target)? {
            Some(h) if Some(h) != default => {
                let f = Value::Object(target);
                Ok(Some(self.call(&Value::Object(h), &f, std::slice::from_ref(v))?.truthy()))
            }
            _ => Ok(None),
        }
    }

    /// `target[@@hasInstance]` when it is callable, `None` when it is absent or the code runs
    /// without [`crate::Options::es2015`]; anything else throws a `TypeError`.
    fn has_instance_handler(&mut self, target: ObjId) -> JsResult<Option<ObjId>> {
        if !self.options.es2015 {
            return Ok(None);
        }
        let Some(sym) = self.well_known(|w| &w.has_instance) else { return Ok(None) };
        let handler = self.get_sym_from(target, &sym, &Value::Object(target))?;
        match handler {
            Value::Undefined | Value::Null => Ok(None),
            Value::Object(h) if self.is_callable(&handler) => Ok(Some(h)),
            _ => Err(self.type_error("the value is not a function")),
        }
    }

    /// The `@@toStringTag` of `o` when it is a string (section 19.1.3.6, steps 15 and 16). Code
    /// run without [`crate::Options::es2015`] reads no tag, so it gets the ECMAScript 5.1 result
    /// even in a realm that has the well-known Symbols from an earlier run.
    pub(crate) fn string_tag_of(&mut self, o: ObjId) -> JsResult<Option<Str>> {
        if !self.options.es2015 {
            return Ok(None);
        }
        let Some(sym) = self.well_known(|w| &w.to_string_tag) else { return Ok(None) };
        match self.get_sym_from(o, &sym, &Value::Object(o))? {
            Value::String(s) => Ok(Some(s)),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Sym, SymProps};
    use crate::heap::{OPEN, Prop};
    use crate::value::Value;

    #[test]
    fn removing_most_symbol_keys_gives_back_their_room_and_keeps_the_order() {
        let mut m = SymProps::default();
        let keys: Vec<Sym> = (0..100).map(|_| Sym::new(None, false)).collect();
        for k in &keys {
            m.insert(k.clone(), Prop::Data { value: Value::Undefined, flags: OPEN });
        }
        for k in &keys[..90] {
            m.remove(k);
        }
        assert!(m.entries.capacity() < 64, "{}", m.entries.capacity());
        assert_eq!(m.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(), keys[90..]);
    }
}
