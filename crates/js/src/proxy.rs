//! Proxy objects (ECMA-262 2020 section 9.5, <https://262.ecma-international.org/11.0/#sec-proxy-object-internal-methods-and-internal-slots>)
//! and the internal methods every object answers through: [[GetPrototypeOf]], [[SetPrototypeOf]],
//! [[IsExtensible]], [[PreventExtensions]], [[Set]] with a receiver, and the proxy cases of
//! [[GetOwnProperty]], [[DefineOwnProperty]], [[HasProperty]], [[Get]], [[Delete]],
//! [[OwnPropertyKeys]], [[Call]] and [[Construct]], which [`Interp`]'s property methods send here.
//!
//! A trap is a script call: each proxy operation takes a unit of fuel and counts as a nested call
//! against [`crate::Limits::reentry`] while it runs, so a proxy whose target or handler is another
//! proxy, or a trap that uses its own proxy, ends at that limit rather than exhausting the native
//! stack. A trap's result is checked against the target as section 9.5 says, and a result that
//! breaks an invariant is a `TypeError`. The proxy, its target and handler, and every value a trap
//! gives stay rooted while the operation runs, so a trap that revokes the proxy or starts a
//! collection leaves them in place.

use std::collections::HashSet;

use crate::LimitKind;
use crate::builtins::object::{from_desc, to_desc};
use crate::heap::{
    CONFIGURABLE, Class, ENUMERABLE, Key, OBJECT_BYTES, OPEN, ObjId, Object, PROP_BYTES, PROXY_BYTES, Prop, ProxyData, WRITABLE, key, string_bytes,
};
use crate::interp::{Abrupt, Desc, ErrorType, HeldBytes, Interp, JsResult, KEY_ENTRY_BYTES, Lookup, ProxyCall, merge_desc, same_value};
use crate::value::{Str, Value, index_key};

/// The most names an `ownKeys` trap may give.
const MAX_OWN_KEYS: f64 = (1u64 << 24) as f64;

impl Interp {
    pub(crate) fn is_proxy(&self, id: ObjId) -> bool {
        matches!(self.heap.object(id).map(|o| &o.class), Some(Class::Proxy(_)))
    }

    fn proxy_data(&self, id: ObjId) -> Option<ProxyData> {
        match self.heap.object(id).map(|o| &o.class) {
            Some(Class::Proxy(p)) => Some(**p),
            _ => None,
        }
    }

    /// ProxyCreate (section 9.5.14): a proxy for `target` with `handler`.
    pub(crate) fn proxy_create(&mut self, target: &Value, handler: &Value) -> JsResult<ObjId> {
        let Value::Object(t) = target else { return Err(self.type_error("a proxy's target must be an object")) };
        let Value::Object(h) = handler else { return Err(self.type_error("a proxy's handler must be an object")) };
        let data = ProxyData { target: Some(*t), handler: Some(*h), callable: self.is_callable(target), constructor: self.is_constructor(target) };
        self.rooted(&[target.clone(), handler.clone()], |i| i.reserve_heap(OBJECT_BYTES + PROXY_BYTES))?;
        self.heap.bytes = self.heap.bytes.saturating_add(PROXY_BYTES);
        Ok(self.heap.alloc_object(Object::new(Class::Proxy(Box::new(data)), None)))
    }

    /// Revokes the proxy `id`: every later operation on it is a `TypeError`. A value that is not a
    /// proxy is left as it is.
    pub(crate) fn proxy_revoke(&mut self, id: ObjId) {
        if let Some(Object { class: Class::Proxy(p), .. }) = self.heap.object_mut(id) {
            p.target = None;
            p.handler = None;
        }
    }

    /// Runs `f` with the proxy's target and handler for the operation `what`: a revoked proxy is a
    /// `TypeError`. The operation takes a unit of fuel and a nested call, and `p`, its target and
    /// handler and `roots` stay rooted while it runs, with anything `f` adds to the roots.
    fn proxy_op<T>(&mut self, p: ObjId, what: &str, roots: &[Value], f: impl FnOnce(&mut Self, ObjId, ObjId) -> JsResult<T>) -> JsResult<T> {
        if self.reentry >= self.limits.reentry {
            return Err(Abrupt::Limit(LimitKind::Reentry));
        }
        self.use_fuel()?;
        let Some(ProxyData { target: Some(target), handler: Some(handler), .. }) = self.proxy_data(p) else {
            return Err(self.type_error(&format!("cannot {what} a revoked proxy")));
        };
        let mark = self.temp_roots.len();
        self.temp_roots.extend([Value::Object(p), Value::Object(target), Value::Object(handler)]);
        self.temp_roots.extend(roots.iter().cloned());
        self.reentry += 1;
        let r = f(self, target, handler);
        self.reentry -= 1;
        self.temp_roots.truncate(mark);
        r
    }

    /// GetMethod (section 7.3.10) of the handler: the trap `name`, or `None` when it is
    /// `undefined` or `null`.
    fn proxy_trap(&mut self, handler: ObjId, name: &str) -> JsResult<Option<Value>> {
        let trap = self.get_from(handler, &key(name), &Value::Object(handler))?;
        if trap.is_nullish() {
            return Ok(None);
        }
        if !self.is_callable(&trap) {
            return Err(self.type_error(&format!("the proxy handler's {name} is not a function")));
        }
        self.temp_roots.push(trap.clone());
        Ok(Some(trap))
    }

    /// Calls the trap with the handler as `this`, keeping its result rooted.
    fn call_trap(&mut self, trap: &Value, handler: ObjId, args: &[Value]) -> JsResult<Value> {
        let v = self.call(trap, &Value::Object(handler), args)?;
        self.temp_roots.push(v.clone());
        Ok(v)
    }

    fn key_value(k: &[u16]) -> Value {
        Value::String(Str::from_units(k.to_vec()))
    }

    fn invariant(&mut self, trap: &str, why: &str) -> Abrupt {
        self.type_error(&format!("the proxy's {trap} trap {why}"))
    }

    // ---- [[GetPrototypeOf]], [[SetPrototypeOf]], [[IsExtensible]], [[PreventExtensions]] ----

    /// [[GetPrototypeOf]] (sections 9.1.1 and 9.5.1).
    pub(crate) fn get_prototype_of(&mut self, id: ObjId) -> JsResult<Option<ObjId>> {
        if !self.is_proxy(id) {
            return Ok(self.object(id)?.proto);
        }
        self.proxy_op(id, "read the prototype of", &[], |i, target, handler| {
            let Some(trap) = i.proxy_trap(handler, "getPrototypeOf")? else { return i.get_prototype_of(target) };
            let proto = match i.call_trap(&trap, handler, &[Value::Object(target)])? {
                Value::Object(p) => Some(p),
                Value::Null => None,
                _ => return Err(i.invariant("getPrototypeOf", "returned a value that is neither an object nor null")),
            };
            if i.is_extensible(target)? {
                return Ok(proto);
            }
            if i.get_prototype_of(target)? != proto {
                return Err(i.invariant("getPrototypeOf", "returned a prototype other than its non-extensible target's"));
            }
            Ok(proto)
        })
    }

    /// [[SetPrototypeOf]] (sections 9.1.2 and 9.5.2): whether the prototype was set. The
    /// prototype of `Object.prototype` cannot change (section 9.4.7).
    pub(crate) fn set_prototype_of(&mut self, id: ObjId, proto: Option<ObjId>) -> JsResult<bool> {
        if self.is_proxy(id) {
            let arg = proto.map_or(Value::Null, Value::Object);
            return self.proxy_op(id, "set the prototype of", std::slice::from_ref(&arg), |i, target, handler| {
                let Some(trap) = i.proxy_trap(handler, "setPrototypeOf")? else { return i.set_prototype_of(target, proto) };
                if !i.call_trap(&trap, handler, &[Value::Object(target), arg.clone()])?.truthy() {
                    return Ok(false);
                }
                if i.is_extensible(target)? {
                    return Ok(true);
                }
                if i.get_prototype_of(target)? != proto {
                    return Err(i.invariant("setPrototypeOf", "reported setting a prototype its non-extensible target does not have"));
                }
                Ok(true)
            });
        }
        let o = self.object(id)?;
        if o.proto == proto {
            return Ok(true);
        }
        if !o.extensible || id == self.intrinsics.object_proto {
            return Ok(false);
        }
        // The new prototype's chain must not lead back to the object; a proxy on it ends the check.
        let mut cur = proto;
        while let Some(p) = cur {
            if p == id {
                return Ok(false);
            }
            if self.is_proxy(p) {
                break;
            }
            self.use_fuel()?;
            cur = self.object(p)?.proto;
        }
        self.object_mut(id)?.proto = proto;
        // A new prototype can bring indices with it.
        self.heap.index_epoch = self.heap.index_epoch.wrapping_add(1);
        Ok(true)
    }

    /// [[IsExtensible]] (sections 9.1.3 and 9.5.3).
    pub(crate) fn is_extensible(&mut self, id: ObjId) -> JsResult<bool> {
        if !self.is_proxy(id) {
            return Ok(self.object(id)?.extensible);
        }
        self.proxy_op(id, "test the extensibility of", &[], |i, target, handler| {
            let Some(trap) = i.proxy_trap(handler, "isExtensible")? else { return i.is_extensible(target) };
            let answer = i.call_trap(&trap, handler, &[Value::Object(target)])?.truthy();
            if answer != i.is_extensible(target)? {
                return Err(i.invariant("isExtensible", "gave an answer other than its target's"));
            }
            Ok(answer)
        })
    }

    /// [[PreventExtensions]] (sections 9.1.4 and 9.5.4): whether the object is now not
    /// extensible.
    pub(crate) fn prevent_extensions(&mut self, id: ObjId) -> JsResult<bool> {
        if !self.is_proxy(id) {
            self.object_mut(id)?.extensible = false;
            return Ok(true);
        }
        self.proxy_op(id, "prevent extensions of", &[], |i, target, handler| {
            let Some(trap) = i.proxy_trap(handler, "preventExtensions")? else { return i.prevent_extensions(target) };
            let done = i.call_trap(&trap, handler, &[Value::Object(target)])?.truthy();
            if done && i.is_extensible(target)? {
                return Err(i.invariant("preventExtensions", "returned true while its target is extensible"));
            }
            Ok(done)
        })
    }

    // ---- properties ----

    /// [[GetOwnProperty]] of a proxy (section 9.5.5).
    pub(crate) fn proxy_get_own_property(&mut self, p: ObjId, k: &[u16]) -> JsResult<Option<Prop>> {
        self.proxy_op(p, "read a property of", &[], |i, target, handler| {
            let Some(trap) = i.proxy_trap(handler, "getOwnPropertyDescriptor")? else { return i.get_own_property(target, k) };
            let result = i.call_trap(&trap, handler, &[Value::Object(target), Self::key_value(k)])?;
            if !matches!(result, Value::Object(_) | Value::Undefined) {
                return Err(i.invariant("getOwnPropertyDescriptor", "returned a value that is neither an object nor undefined"));
            }
            let target_prop = i.get_own_property(target, k)?;
            if matches!(result, Value::Undefined) {
                let Some(current) = target_prop else { return Ok(None) };
                if !current.configurable() {
                    return Err(i.invariant("getOwnPropertyDescriptor", "hid a non-configurable property of its target"));
                }
                if !i.is_extensible(target)? {
                    return Err(i.invariant("getOwnPropertyDescriptor", "hid a property of its non-extensible target"));
                }
                return Ok(None);
            }
            let extensible = i.is_extensible(target)?;
            let desc = to_desc(i, &result)?;
            i.temp_roots.extend(desc_values(&desc));
            let prop = complete(&desc);
            let desc = prop_desc(&prop);
            if !compatible(extensible, &desc, target_prop.as_ref()) {
                return Err(i.invariant("getOwnPropertyDescriptor", "gave a descriptor its target's property does not allow"));
            }
            if !prop.configurable() {
                match &target_prop {
                    Some(t) if !t.configurable() => {}
                    _ => {
                        return Err(
                            i.invariant("getOwnPropertyDescriptor", "reported a non-configurable property its target has as configurable or lacks")
                        );
                    }
                }
                if desc.writable == Some(false) && matches!(target_prop, Some(Prop::Data { flags, .. }) if flags & WRITABLE != 0) {
                    return Err(
                        i.invariant("getOwnPropertyDescriptor", "reported a writable property of its target as non-configurable and read-only")
                    );
                }
            }
            Ok(Some(prop))
        })
    }

    /// [[DefineOwnProperty]] of a proxy (section 9.5.6): whether the trap defined the property.
    pub(crate) fn proxy_define_own_property(&mut self, p: ObjId, k: &Key, desc: &Desc) -> JsResult<bool> {
        let roots = desc_values(desc);
        self.proxy_op(p, "define a property of", &roots, |i, target, handler| {
            let Some(trap) = i.proxy_trap(handler, "defineProperty")? else { return i.define_own_property(target, k.clone(), desc, false) };
            let desc_obj = from_desc(i, desc)?;
            i.temp_roots.push(desc_obj.clone());
            if !i.call_trap(&trap, handler, &[Value::Object(target), Self::key_value(k), desc_obj])?.truthy() {
                return Ok(false);
            }
            let target_prop = i.get_own_property(target, k)?;
            let extensible = i.is_extensible(target)?;
            let setting_config_false = desc.configurable == Some(false);
            let Some(current) = target_prop else {
                if !extensible {
                    return Err(i.invariant("defineProperty", "added a property to its non-extensible target"));
                }
                if setting_config_false {
                    return Err(i.invariant("defineProperty", "defined a non-configurable property its target lacks"));
                }
                return Ok(true);
            };
            if !compatible(extensible, desc, Some(&current)) {
                return Err(i.invariant("defineProperty", "defined a property its target's property does not allow"));
            }
            if setting_config_false && current.configurable() {
                return Err(i.invariant("defineProperty", "defined a property as non-configurable that is configurable on its target"));
            }
            if let Prop::Data { flags, .. } = current
                && flags & CONFIGURABLE == 0
                && flags & WRITABLE != 0
                && desc.writable == Some(false)
            {
                return Err(i.invariant("defineProperty", "made a property read-only that is writable on its target"));
            }
            Ok(true)
        })
    }

    /// [[HasProperty]] of a proxy (section 9.5.7).
    pub(crate) fn proxy_has(&mut self, p: ObjId, k: &[u16]) -> JsResult<bool> {
        self.proxy_op(p, "test a property of", &[], |i, target, handler| {
            let Some(trap) = i.proxy_trap(handler, "has")? else { return i.has_property(target, k) };
            let found = i.call_trap(&trap, handler, &[Value::Object(target), Self::key_value(k)])?.truthy();
            if !found && let Some(current) = i.get_own_property(target, k)? {
                if !current.configurable() {
                    return Err(i.invariant("has", "hid a non-configurable property of its target"));
                }
                if !i.is_extensible(target)? {
                    return Err(i.invariant("has", "hid a property of its non-extensible target"));
                }
            }
            Ok(found)
        })
    }

    /// [[Get]] of a proxy (section 9.5.8).
    pub(crate) fn proxy_get(&mut self, p: ObjId, k: &[u16], receiver: &Value) -> JsResult<Value> {
        self.proxy_op(p, "read a property of", std::slice::from_ref(receiver), |i, target, handler| {
            let Some(trap) = i.proxy_trap(handler, "get")? else { return i.get_from(target, k, receiver) };
            let v = i.call_trap(&trap, handler, &[Value::Object(target), Self::key_value(k), receiver.clone()])?;
            match i.get_own_property(target, k)? {
                Some(Prop::Data { value, flags }) if flags & (CONFIGURABLE | WRITABLE) == 0 && !same_value(&v, &value) => {
                    Err(i.invariant("get", "gave a value other than its target's non-configurable, read-only property"))
                }
                Some(Prop::Accessor { get: None, flags, .. }) if flags & CONFIGURABLE == 0 && !matches!(v, Value::Undefined) => {
                    Err(i.invariant("get", "gave a value for a non-configurable property of its target that has no getter"))
                }
                _ => Ok(v),
            }
        })
    }

    /// [[Set]] of a proxy (section 9.5.9): whether the trap set the property.
    fn proxy_set(&mut self, p: ObjId, k: &Key, v: &Value, receiver: &Value) -> JsResult<bool> {
        self.proxy_op(p, "set a property of", &[v.clone(), receiver.clone()], |i, target, handler| {
            let Some(trap) = i.proxy_trap(handler, "set")? else { return i.set_with_receiver(target, k.clone(), v.clone(), receiver) };
            if !i.call_trap(&trap, handler, &[Value::Object(target), Self::key_value(k), v.clone(), receiver.clone()])?.truthy() {
                return Ok(false);
            }
            match i.get_own_property(target, k)? {
                Some(Prop::Data { value, flags }) if flags & (CONFIGURABLE | WRITABLE) == 0 && !same_value(v, &value) => {
                    Err(i.invariant("set", "set a value other than its target's non-configurable, read-only property"))
                }
                Some(Prop::Accessor { set: None, flags, .. }) if flags & CONFIGURABLE == 0 => {
                    Err(i.invariant("set", "set a non-configurable property of its target that has no setter"))
                }
                _ => Ok(true),
            }
        })
    }

    /// [[Delete]] of a proxy (section 9.5.10): whether the trap deleted the property.
    pub(crate) fn proxy_delete(&mut self, p: ObjId, k: &[u16]) -> JsResult<bool> {
        self.proxy_op(p, "delete a property of", &[], |i, target, handler| {
            let Some(trap) = i.proxy_trap(handler, "deleteProperty")? else { return i.delete(target, k, false) };
            if !i.call_trap(&trap, handler, &[Value::Object(target), Self::key_value(k)])?.truthy() {
                return Ok(false);
            }
            let Some(current) = i.get_own_property(target, k)? else { return Ok(true) };
            if !current.configurable() {
                return Err(i.invariant("deleteProperty", "deleted a non-configurable property of its target"));
            }
            if !i.is_extensible(target)? {
                return Err(i.invariant("deleteProperty", "deleted a property of its non-extensible target"));
            }
            Ok(true)
        })
    }

    /// [[OwnPropertyKeys]] of a proxy (section 9.5.11). The trap must give an array-like object
    /// of distinct strings, at most [`MAX_OWN_KEYS`] of them, that holds every non-configurable
    /// name of the target, and of a non-extensible target exactly its names. Each name takes a
    /// unit of fuel and counts against the heap limit while the list is built.
    pub(crate) fn proxy_own_keys(&mut self, p: ObjId) -> JsResult<Vec<Key>> {
        self.proxy_op(p, "list the properties of", &[], |i, target, handler| {
            let Some(trap) = i.proxy_trap(handler, "ownKeys")? else { return i.own_keys(target) };
            let result = i.call_trap(&trap, handler, &[Value::Object(target)])?;
            let mut held = HeldBytes::new(i);
            let keys = i.trap_key_list(&result, &mut held)?;
            i.check_own_keys(target, &keys)?;
            Ok(keys)
        })
    }

    /// CreateListFromArrayLike (section 7.3.17) of an `ownKeys` trap's result, refusing a value
    /// that is not a string and a name given twice.
    fn trap_key_list(&mut self, result: &Value, held: &mut HeldBytes) -> JsResult<Vec<Key>> {
        let Value::Object(list) = result else { return Err(self.invariant("ownKeys", "returned a value that is not an object")) };
        let length = self.get_from(*list, &key("length"), result)?;
        let n = self.to_number(&length)?;
        // ToLength (section 7.1.15).
        let n = if n.is_nan() || n <= 0.0 { 0.0 } else { n.trunc() };
        if n > MAX_OWN_KEYS {
            return Err(self.throw(ErrorType::Range, "the proxy's ownKeys trap returned too many names"));
        }
        let mut keys = Vec::new();
        let mut seen: HashSet<Key> = HashSet::new();
        for index in 0..n as u32 {
            self.use_fuel()?;
            let v = self.get_from(*list, &index_key(index), result)?;
            let Value::String(s) = v else { return Err(self.invariant("ownKeys", "returned a name that is not a string")) };
            held.add(self, string_bytes(s.len()).saturating_add(2 * KEY_ENTRY_BYTES))?;
            let k = s.flat();
            if !seen.insert(k.clone()) {
                let name = String::from_utf16_lossy(&k);
                return Err(self.invariant("ownKeys", &format!("returned the name '{name}' twice")));
            }
            keys.push(k);
        }
        Ok(keys)
    }

    /// Steps 16 to 23 of section 9.5.11: the names `keys` the trap gave against the target's.
    fn check_own_keys(&mut self, target: ObjId, keys: &[Key]) -> JsResult<()> {
        let extensible = self.is_extensible(target)?;
        let target_keys = self.own_keys(target)?;
        let mut fixed = Vec::new();
        let mut configurable = Vec::new();
        for k in target_keys {
            self.use_fuel()?;
            if self.get_own_property(target, &k)?.is_some_and(|p| !p.configurable()) {
                fixed.push(k);
            } else {
                configurable.push(k);
            }
        }
        if extensible && fixed.is_empty() {
            return Ok(());
        }
        let mut unchecked: HashSet<&[u16]> = keys.iter().map(|k| &**k).collect();
        for k in &fixed {
            if !unchecked.remove(&**k) {
                let name = String::from_utf16_lossy(k);
                return Err(self.invariant("ownKeys", &format!("left out the non-configurable property '{name}' of its target")));
            }
        }
        if extensible {
            return Ok(());
        }
        for k in &configurable {
            if !unchecked.remove(&**k) {
                let name = String::from_utf16_lossy(k);
                return Err(self.invariant("ownKeys", &format!("left out the property '{name}' of its non-extensible target")));
            }
        }
        if !unchecked.is_empty() {
            return Err(self.invariant("ownKeys", "gave a name its non-extensible target does not have"));
        }
        Ok(())
    }

    /// [[Set]] (section 9.1.9) of `k` on `o` with `receiver`: a proxy's `set` trap, a setter found
    /// on the chain, or a data property defined on the receiver. Whether the value was set.
    pub(crate) fn set_with_receiver(&mut self, o: ObjId, k: Key, v: Value, receiver: &Value) -> JsResult<bool> {
        let mark = self.temp_roots.len();
        self.temp_roots.extend([Value::Object(o), v.clone(), receiver.clone()]);
        let r = self.ordinary_set(o, k, v, receiver);
        self.temp_roots.truncate(mark);
        r
    }

    fn ordinary_set(&mut self, o: ObjId, k: Key, v: Value, receiver: &Value) -> JsResult<bool> {
        let own = match self.lookup(o, &k)? {
            Lookup::Proxy(p) => return self.proxy_set(p, &k, &v, receiver),
            Lookup::Found(prop) => prop,
            Lookup::Missing => Prop::Data { value: Value::Undefined, flags: OPEN },
        };
        match own {
            Prop::Accessor { set: Some(s), .. } => {
                self.call(&Value::Object(s), receiver, &[v])?;
                Ok(true)
            }
            Prop::Accessor { set: None, .. } => Ok(false),
            Prop::Data { flags, .. } if flags & WRITABLE == 0 => Ok(false),
            Prop::Data { .. } => {
                let Value::Object(r) = receiver else { return Ok(false) };
                let r = *r;
                match self.get_own_property(r, &k)? {
                    Some(Prop::Accessor { .. }) => Ok(false),
                    Some(Prop::Data { flags, .. }) if flags & WRITABLE == 0 => Ok(false),
                    Some(Prop::Data { .. }) => self.define_own_property(r, k, &Desc { value: Some(v), ..Desc::default() }, false),
                    None => self.define_own_property(r, k, &Desc::data(v, OPEN), false),
                }
            }
        }
    }

    // ---- calls ----

    /// [[Call]] of a proxy (section 9.5.12): the trap's result, or the target to call when the
    /// handler has no `apply` trap.
    pub(crate) fn proxy_call(&mut self, p: ObjId, this: &Value, args: &[Value]) -> JsResult<ProxyCall> {
        if !self.proxy_data(p).is_some_and(|d| d.callable) {
            return Err(self.type_error("the value is not a function"));
        }
        let mut roots = vec![this.clone()];
        roots.extend(args.iter().cloned());
        self.proxy_op(p, "call", &roots, |i, target, handler| {
            let Some(trap) = i.proxy_trap(handler, "apply")? else { return Ok(ProxyCall::Target(target)) };
            let list = i.trap_array(args)?;
            let v = i.call_trap(&trap, handler, &[Value::Object(target), this.clone(), list])?;
            Ok(ProxyCall::Done(v))
        })
    }

    /// [[Construct]] of a proxy (section 9.5.13): the object the trap made, or the target to
    /// construct when the handler has no `construct` trap.
    pub(crate) fn proxy_construct(&mut self, p: ObjId, args: &[Value], new_target: &Value) -> JsResult<ProxyCall> {
        if !self.proxy_data(p).is_some_and(|d| d.constructor) {
            return Err(self.type_error("the value is not a constructor"));
        }
        let mut roots = vec![new_target.clone()];
        roots.extend(args.iter().cloned());
        self.proxy_op(p, "construct", &roots, |i, target, handler| {
            let Some(trap) = i.proxy_trap(handler, "construct")? else { return Ok(ProxyCall::Target(target)) };
            let list = i.trap_array(args)?;
            let v = i.call_trap(&trap, handler, &[Value::Object(target), list, new_target.clone()])?;
            if !matches!(v, Value::Object(_)) {
                return Err(i.invariant("construct", "returned a value that is not an object"));
            }
            Ok(ProxyCall::Done(v))
        })
    }

    /// CreateArrayFromList (section 7.3.16) of the arguments a trap is given, checked against the
    /// heap limit and kept rooted.
    fn trap_array(&mut self, args: &[Value]) -> JsResult<Value> {
        self.reserve_heap(OBJECT_BYTES.saturating_add(args.len().saturating_mul(PROP_BYTES + string_bytes(10))))?;
        let a = Value::Object(self.array_of(args)?);
        self.temp_roots.push(a.clone());
        Ok(a)
    }

    /// IsArray (section 7.2.2): an array, or a proxy whose target is one. A revoked proxy is a
    /// `TypeError`.
    pub(crate) fn is_array(&mut self, v: &Value) -> JsResult<bool> {
        let mut cur = v.as_object();
        while let Some(o) = cur {
            match self.heap.object(o).map(|x| &x.class) {
                Some(Class::Array { .. }) => return Ok(true),
                Some(Class::Proxy(p)) => {
                    let ProxyData { target, handler, .. } = **p;
                    if handler.is_none() {
                        return Err(self.type_error("cannot test whether a revoked proxy is an array"));
                    }
                    self.use_fuel()?;
                    cur = target;
                }
                _ => return Ok(false),
            }
        }
        Ok(false)
    }
}

/// The values a descriptor holds, to keep them reachable while more script runs.
pub(crate) fn desc_values(desc: &Desc) -> Vec<Value> {
    let mut out: Vec<Value> = desc.value.iter().cloned().collect();
    out.extend(desc.get.flatten().map(Value::Object));
    out.extend(desc.set.flatten().map(Value::Object));
    out
}

/// CompletePropertyDescriptor (section 6.2.5.6): the property `desc` describes, each absent field
/// at its default.
fn complete(desc: &Desc) -> Prop {
    let flags = desc.flags_or(0);
    if desc.is_accessor() {
        Prop::Accessor { get: desc.get.flatten(), set: desc.set.flatten(), flags: flags & (ENUMERABLE | CONFIGURABLE) }
    } else {
        Prop::Data { value: desc.value.clone().unwrap_or(Value::Undefined), flags }
    }
}

/// The descriptor with every field of `prop`.
fn prop_desc(prop: &Prop) -> Desc {
    match prop {
        Prop::Data { value, flags } => Desc::data(value.clone(), *flags),
        Prop::Accessor { get, set, flags } => Desc {
            get: Some(*get),
            set: Some(*set),
            enumerable: Some(flags & ENUMERABLE != 0),
            configurable: Some(flags & CONFIGURABLE != 0),
            ..Desc::default()
        },
    }
}

/// IsCompatiblePropertyDescriptor (section 9.1.6.2): whether a target with `current` (and, when
/// it has none, `extensible`) allows `desc`.
fn compatible(extensible: bool, desc: &Desc, current: Option<&Prop>) -> bool {
    match current {
        None => extensible,
        Some(current) => merge_desc(current, desc).is_some(),
    }
}
