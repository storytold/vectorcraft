//! Classes, `super` and `new.target` (ECMA-262 2015 sections 12.3.5, 12.3.8 and 14.5), and
//! `new` with a `new.target` (sections 7.3.13 and 9.2.2).
//!
//! The running code reaches what these need through bindings whose names start with a NUL,
//! which no identifier holds:
//!
//! - `\0this` holds a derived class constructor's `this`, uninitialized until `super(…)` returns,
//!   and the `this` an arrow function closes over.
//! - `\0func` holds a derived class constructor's function object, whose prototype `super(…)`
//!   constructs.
//! - `\0newtarget` holds `new.target` for the arrow functions inside a function.
//! - `\0home` holds the home object of a class method, in an environment of the method's own.

use std::rc::Rc;

use crate::bytecode::{Code, NamePrefix};
use crate::heap::{
    Binding, CONFIGURABLE, Class, ENUMERABLE, ENV_BYTES, Env, EnvId, Func, HIDDEN, OBJECT_BYTES, ObjId, Object, Prop, PropMap, WRITABLE, key,
    string_bytes,
};
use crate::interp::{ARROW_THIS, Abrupt, Desc, Entry, ErrorType, Interp, JsResult, binding_bytes, closure_bytes, internal};
use crate::symbol::PropKey;
use crate::value::{Str, Value};

const FUNC: &[u16] = &[0, 0x66, 0x75, 0x6E, 0x63];
const NEW_TARGET: &[u16] = &[0, 0x6E, 0x65, 0x77, 0x74, 0x61, 0x72, 0x67, 0x65, 0x74];
const HOME: &[u16] = &[0, 0x68, 0x6F, 0x6D, 0x65];

/// The most bound functions `new` and `IsConstructor` unwrap in one chain.
const MAX_BOUND_CHAIN: usize = 10_000;

/// The bytes of the hidden bindings [`Interp::function_bindings`] adds for `code`.
pub(crate) fn hidden_bytes(code: &Code) -> usize {
    if code.derived {
        binding_bytes(ARROW_THIS.len()) + binding_bytes(FUNC.len()) + binding_bytes(NEW_TARGET.len())
    } else if code.uses_new_target && !code.arrow {
        binding_bytes(NEW_TARGET.len())
    } else {
        0
    }
}

impl Interp {
    /// The nearest declarative environment from `start` outward that holds `name`.
    fn hidden_env(&self, start: EnvId, name: &[u16]) -> Option<EnvId> {
        let mut cur = Some(start);
        while let Some(e) = cur {
            match self.heap.env(e)? {
                Env::Decl { bindings, outer } => {
                    if bindings.contains(name) {
                        return Some(e);
                    }
                    cur = *outer;
                }
                Env::Obj { outer, .. } => cur = *outer,
            }
        }
        None
    }

    /// The value of the hidden binding `name` nearest the running code, or `undefined`.
    fn hidden_value(&self, name: &[u16]) -> JsResult<Value> {
        let env = self.frame()?.env;
        match self.hidden_env(env, name) {
            Some(e) => self.binding_value(e, name),
            None => Ok(Value::Undefined),
        }
    }

    /// Adds the hidden bindings of a call to `code` to its function environment `env`: a derived
    /// class constructor's `this`, function object and `new.target`, and the `new.target` of a
    /// function whose arrow functions read it. Gives the environment of a derived constructor's
    /// `this`. The caller has reserved [`hidden_bytes`] of `code`.
    pub(crate) fn function_bindings(&mut self, code: &Code, env: EnvId, callee: ObjId, new_target: Option<ObjId>) -> JsResult<Option<EnvId>> {
        let nt = new_target.map_or(Value::Undefined, Value::Object);
        if code.derived {
            self.env_insert(env, Rc::from(ARROW_THIS), Binding::lexical(false))?;
            self.env_insert(env, Rc::from(FUNC), Binding::new(Value::Object(callee), false, false))?;
            self.env_insert(env, Rc::from(NEW_TARGET), Binding::new(nt, false, false))?;
            return Ok(Some(env));
        }
        if code.uses_new_target && !code.arrow {
            self.env_insert(env, Rc::from(NEW_TARGET), Binding::new(nt, false, false))?;
        }
        Ok(None)
    }

    /// The `this` an arrow function closing over `env` starts with: `undefined` while a derived
    /// constructor's `this` is not yet bound.
    pub(crate) fn arrow_this(&self, env: EnvId) -> Value {
        match self.hidden_env(env, ARROW_THIS) {
            Some(e) if !self.uninitialized(e, ARROW_THIS) => self.binding_value(e, ARROW_THIS).unwrap_or(Value::Undefined),
            _ => Value::Undefined,
        }
    }

    fn this_not_bound(&mut self) -> Abrupt {
        self.throw(ErrorType::Reference, "'this' is used before super() is called")
    }

    /// The value of `this` in the running code (ResolveThisBinding, section 8.3.2). In a derived
    /// class constructor and the arrow functions and direct eval code inside it, reading it before
    /// `super(…)` returns is a `ReferenceError`.
    pub(crate) fn this_value(&mut self) -> JsResult<Value> {
        let frame = self.frame()?;
        if frame.this_in_env
            && let Some(e) = self.hidden_env(frame.env, ARROW_THIS)
        {
            if self.uninitialized(e, ARROW_THIS) {
                return Err(self.this_not_bound());
            }
            return self.binding_value(e, ARROW_THIS);
        }
        Ok(self.frame()?.this.clone())
    }

    /// What a derived class constructor whose `this` is in `env` gives for the return value `v`
    /// (section 9.2.2, steps 13 to 15).
    pub(crate) fn derived_result(&mut self, env: EnvId, v: Value) -> JsResult<Value> {
        match v {
            Value::Object(_) => Ok(v),
            Value::Undefined if self.uninitialized(env, ARROW_THIS) => Err(self.this_not_bound()),
            Value::Undefined => self.binding_value(env, ARROW_THIS),
            _ => Err(self.type_error("a derived class constructor must return an object or undefined")),
        }
    }

    /// IsConstructor (section 7.2.4).
    pub(crate) fn is_constructor(&self, v: &Value) -> bool {
        let mut v = v.clone();
        for _ in 0..MAX_BOUND_CHAIN {
            match self.func_of(&v) {
                Some(Func::Closure { code, .. }) => return !(code.arrow || code.method || code.generator),
                Some(Func::Native { constructor, .. }) => return constructor,
                Some(Func::Data { read: crate::host::ArgRead::Hosted, .. }) => return v.as_object().is_some_and(|o| self.is_host_constructor(o)),
                Some(Func::Bound { target, .. }) => v = Value::Object(target),
                _ => return false,
            }
        }
        false
    }

    /// Does the built-in constructor `f` make objects that a subclass may give its own
    /// prototype? It does when its `prototype` is one of the realm's built-in prototypes.
    fn subclassable(&self, f: ObjId) -> JsResult<bool> {
        let Some(Prop::Data { value: Value::Object(p), .. }) = self.get_own_property(f, &key("prototype"))? else { return Ok(false) };
        let i = &self.intrinsics;
        Ok([i.object_proto, i.function_proto, i.array_proto, i.string_proto, i.number_proto, i.boolean_proto, i.regexp_proto, i.date_proto]
            .contains(&p)
            || i.error_protos.contains(&p))
    }

    /// `new.target`'s `prototype` when it is an object, otherwise `fallback` (section 9.1.14).
    fn prototype_from(&mut self, new_target: ObjId, fallback: ObjId) -> JsResult<ObjId> {
        match self.get_from(new_target, &key("prototype"), &Value::Object(new_target))? {
            Value::Object(p) => Ok(p),
            _ => Ok(fallback),
        }
    }

    /// Does the prototype chain from `start` reach `target`?
    fn chain_reaches(&self, start: ObjId, target: ObjId) -> JsResult<bool> {
        let mut cur = Some(start);
        let mut steps = 0usize;
        while let Some(o) = cur {
            if o == target {
                return Ok(true);
            }
            steps += 1;
            if steps > 1_000_000 {
                return Ok(true);
            }
            cur = self.object(o)?.proto;
        }
        Ok(false)
    }

    /// Constructs `f` with `args` and `new_target` (`f` itself when `None`) and pushes the
    /// result, or the frame that makes it, after taking the stack back to `start`. The operands
    /// at `start` and above keep `f` and `args` reachable.
    pub(crate) fn construct(&mut self, start: usize, f: Value, mut args: Vec<Value>, new_target: Option<ObjId>) -> JsResult<()> {
        let mut f = f;
        let mut new_target = new_target;
        // A bound function constructs its target with its arguments first, and a `new.target`
        // that is the bound function becomes the target (section 9.4.1.2).
        let mut steps = 0usize;
        while let Some(Func::Bound { target, args: bound_args, .. }) = self.func_of(&f) {
            self.use_fuel()?;
            steps += 1;
            if steps > MAX_BOUND_CHAIN {
                return Err(self.type_error("the value is not a constructor"));
            }
            let mut all = bound_args.to_vec();
            all.append(&mut args);
            args = all;
            if new_target.is_some() && new_target == f.as_object() {
                new_target = Some(target);
            }
            f = Value::Object(target);
        }
        let Some(callee) = f.as_object() else { return Err(self.type_error("the value is not a constructor")) };
        let nt = new_target.unwrap_or(callee);
        match self.func_of(&f) {
            Some(Func::Closure { code, .. }) if code.arrow => Err(self.type_error("an arrow function is not a constructor")),
            Some(Func::Closure { code, .. }) if code.generator => Err(self.type_error("a generator function is not a constructor")),
            Some(Func::Closure { code, .. }) if code.method => Err(self.type_error("a method is not a constructor")),
            Some(Func::Closure { code, env }) if code.derived => {
                self.stack.truncate(start);
                self.enter_function(Entry { callee, code, env, this: Value::Undefined, args, new_target: Some(nt) }, None, false)
            }
            Some(Func::Closure { code, env }) => {
                let fallback = self.intrinsics.object_proto;
                let proto = self.prototype_from(nt, fallback)?;
                let obj = self.heap.alloc_object(Object::new(Class::Ordinary, Some(proto)));
                self.stack.truncate(start);
                self.enter_function(Entry { callee, code, env, this: Value::Object(obj), args, new_target: Some(nt) }, Some(obj), false)
            }
            Some(Func::Native { f: native, constructor: true, .. }) => {
                let subclass = nt != callee;
                if subclass && !self.subclassable(callee)? {
                    return Err(self.type_error("this built-in cannot be subclassed"));
                }
                let intrinsic = match self.get_own_property(callee, &key("prototype"))? {
                    Some(Prop::Data { value: Value::Object(p), .. }) => Some(p),
                    _ => None,
                };
                let v = if subclass && intrinsic == Some(self.intrinsics.object_proto) {
                    // `Object` constructed for a subclass makes an ordinary object, whatever its
                    // argument (section 19.1.1.1, step 1).
                    let fallback = self.intrinsics.object_proto;
                    let proto = self.prototype_from(nt, fallback)?;
                    Value::Object(self.heap.alloc_object(Object::new(Class::Ordinary, Some(proto))))
                } else {
                    let held = self.held_bytes;
                    let r = native(self, Value::Undefined, &args, true);
                    self.held_bytes = held;
                    let v = r?;
                    if subclass && let Value::Object(o) = v {
                        let proto = self.rooted(std::slice::from_ref(&v), |i| {
                            let fallback = intrinsic.unwrap_or(i.intrinsics.object_proto);
                            i.prototype_from(nt, fallback)
                        })?;
                        if self.object(o)?.proto == intrinsic && !self.chain_reaches(proto, o)? {
                            self.object_mut(o)?.proto = Some(proto);
                        }
                    }
                    v
                };
                self.stack.truncate(start);
                self.push(v)
            }
            // A host class's constructor ([`crate::host_classes`]) gives the object it makes. A plain
            // object it gives inherits from the class's prototype. The class's other functions are
            // not constructors. A constructor called with `new` cannot wait for its result.
            Some(Func::Data { f: host, name, read: read @ crate::host::ArgRead::Hosted }) if self.is_host_constructor(callee) => {
                if nt != callee {
                    return Err(self.type_error("this built-in cannot be subclassed"));
                }
                let was = std::mem::replace(&mut self.constructing, true);
                let v = self.call_data(&host, &name, &Value::Undefined, &args, &read);
                self.constructing = was;
                let v = v?;
                if let Value::Object(o) = &v
                    && matches!(self.object(*o)?.class, Class::Ordinary)
                    && let Some(Prop::Data { value: Value::Object(proto), .. }) = self.find_property(callee, &key("prototype"))?
                {
                    self.object_mut(*o)?.proto = Some(proto);
                }
                self.stack.truncate(start);
                self.push(v)
            }
            _ => Err(self.type_error("the value is not a constructor")),
        }
    }

    /// `heritage?` → `F proto`: the constructor and prototype of a class whose constructor is
    /// `functions[ctor]` (ClassDefinitionEvaluation, section 14.5.14, steps 5 to 18).
    pub(crate) fn op_class_start(&mut self, ctor: u32, heritage: bool) -> JsResult<()> {
        self.use_fuel()?;
        let (proto_parent, ctor_parent) = if heritage {
            let h = self.peek(0)?;
            match h {
                Value::Null => (None, self.intrinsics.function_proto),
                Value::Object(c) if self.is_constructor(&h) => match self.get_from(c, &key("prototype"), &h)? {
                    Value::Object(p) => (Some(p), c),
                    Value::Null => (None, c),
                    _ => return Err(self.type_error("the prototype of the class's superclass is not an object or null")),
                },
                _ => return Err(self.type_error("a class can extend only a constructor or null")),
            }
        } else {
            (Some(self.intrinsics.object_proto), self.intrinsics.function_proto)
        };
        let frame = self.frame()?;
        let env = frame.env;
        let Some(code) = frame.code.functions.get(ctor as usize).cloned() else { return internal("function out of range") };
        let need = (2 * OBJECT_BYTES)
            .saturating_add(ENV_BYTES)
            .saturating_add(binding_bytes(HOME.len()))
            .saturating_add(closure_bytes(&code))
            .saturating_add(binding_bytes("prototype".len()))
            .saturating_add(binding_bytes("constructor".len()));
        let roots: Vec<Value> = proto_parent.map(Value::Object).into_iter().collect();
        self.rooted(&roots, |i| i.reserve_heap(need))?;
        let proto = self.heap.alloc_object(Object::new(Class::Ordinary, proto_parent));
        // The constructor's home object is the prototype, as for the other prototype methods
        // (section 14.5.14, step 12).
        let home = self.home_env(proto, env);
        let f = self.make_closure(code, home)?;
        self.object_mut(f)?.proto = Some(ctor_parent);
        self.define(f, key("prototype"), Prop::Data { value: Value::Object(proto), flags: 0 })?;
        self.define(proto, key("constructor"), Prop::Data { value: Value::Object(f), flags: HIDDEN })?;
        if heritage {
            self.pop()?;
        }
        self.push(Value::Object(f))?;
        self.push(Value::Object(proto))
    }

    /// An environment inside `outer` whose home object is `home`.
    fn home_env(&mut self, home: ObjId, outer: EnvId) -> EnvId {
        let mut bindings = PropMap::default();
        bindings.insert(Rc::from(HOME), Binding::new(Value::Object(home), false, false));
        self.heap.bytes += string_bytes(HOME.len());
        self.heap.alloc_env(Env::Decl { bindings, outer: Some(outer) })
    }

    /// `F proto key` → `F proto`: defines the method `functions[func]` (`kind` 0), getter (1) or
    /// setter (2) named `key` on the prototype, or with `is_static` on the constructor (section
    /// 14.3.8 and 14.5.14, step 21).
    pub(crate) fn op_class_method(&mut self, func: u32, kind: u8, is_static: bool) -> JsResult<()> {
        let kv = self.peek(0)?;
        let k = self.to_prop_key(&kv)?;
        let Value::Object(target) = self.peek(if is_static { 2 } else { 1 })? else { return internal("class operand is not an object") };
        self.define_method(target, k, func, kind, false)?;
        self.pop().map(drop)
    }

    /// `object` → `object`: defines an object literal's method or accessor `functions[func]`
    /// under `names[name]` (section 12.2.6.8).
    pub(crate) fn op_init_method(&mut self, func: u32, kind: u8, name: u32) -> JsResult<()> {
        let k = self.name(name)?;
        let Value::Object(target) = self.peek(0)? else { return internal("object literal is not an object") };
        self.define_method(target, PropKey::Str(k), func, kind, true)
    }

    /// `object key` → `object`: defines an object literal's method or accessor `functions[func]`
    /// under a computed key, already a name or a Symbol (section 12.2.6.8).
    pub(crate) fn op_init_method_key(&mut self, func: u32, kind: u8) -> JsResult<()> {
        let kv = self.peek(0)?;
        let k = self.to_prop_key(&kv)?;
        let Value::Object(target) = self.peek(1)? else { return internal("object literal is not an object") };
        self.define_method(target, k, func, kind, true)?;
        self.pop().map(drop)
    }

    /// Defines the method (`kind` 0), getter (1) or setter (2) made from `functions[func]` of the
    /// running code under `k` on `target`, its home object (MakeMethod, section 14.3.8), and
    /// names the function after `k`, with `get ` or `set ` before an accessor's name (section
    /// 9.2.11). The caller keeps `target` and a Symbol key reachable.
    fn define_method(&mut self, target: ObjId, k: PropKey, func: u32, kind: u8, enumerable: bool) -> JsResult<()> {
        self.use_fuel()?;
        let frame = self.frame()?;
        let env = frame.env;
        let Some(code) = frame.code.functions.get(func as usize).cloned() else { return internal("function out of range") };
        let named = code.own_name.is_some();
        let key_bytes = match &k {
            PropKey::Str(name) => binding_bytes(name.len()),
            PropKey::Sym(_) => binding_bytes(0),
        };
        let need = OBJECT_BYTES
            .saturating_add(ENV_BYTES)
            .saturating_add(binding_bytes(HOME.len()))
            .saturating_add(closure_bytes(&code))
            .saturating_add(key_bytes);
        let root = match &k {
            PropKey::Str(name) => Value::String(Str::from_rc(name.clone())),
            PropKey::Sym(s) => Value::Symbol(s.clone()),
        };
        self.rooted(std::slice::from_ref(&root), |i| i.reserve_heap(need))?;
        let home = self.home_env(target, env);
        let f = self.make_closure(code, home)?;
        if named {
            let prefix = match kind {
                0 => NamePrefix::None,
                1 => NamePrefix::Get,
                _ => NamePrefix::Set,
            };
            self.rooted(&[Value::Object(f), root], |i| {
                let name = i.key_name(prefix, &k)?;
                i.set_function_name(f, name)
            })?;
        }
        let (enumerable, configurable) = (Some(enumerable), Some(true));
        let desc = match kind {
            0 => Desc { value: Some(Value::Object(f)), writable: Some(true), enumerable, configurable, ..Desc::default() },
            1 => Desc { get: Some(Some(f)), enumerable, configurable, ..Desc::default() },
            _ => Desc { set: Some(Some(f)), enumerable, configurable, ..Desc::default() },
        };
        self.rooted(&[Value::Object(f)], |i| i.define_own_pk(target, k, &desc, true)).map(drop)
    }

    /// The `name` of a function defined under `k`: the name with `prefix` before it, or a
    /// Symbol's description in brackets, or `prefix` alone for a Symbol without one (SetFunctionName,
    /// section 9.2.11).
    fn key_name(&mut self, prefix: NamePrefix, k: &PropKey) -> JsResult<Value> {
        match k {
            PropKey::Str(name) if prefix == NamePrefix::None => Ok(Value::String(Str::from_rc(name.clone()))),
            PropKey::Str(name) => self.wrap_str(prefix.text(), Some(&Str::from_rc(name.clone())), ""),
            PropKey::Sym(s) => match s.description() {
                Some(d) => {
                    let d = d.clone();
                    self.wrap_str(&format!("{}[", prefix.text()), Some(&d), "]")
                }
                None => self.wrap_str(prefix.text(), None, ""),
            },
        }
    }

    /// `this key` → `this home key`: the prototype of the running method's home object, `null`
    /// when it has none, and the key as a property name or a Symbol (MakeSuperPropertyReference,
    /// section 12.3.5.3).
    pub(crate) fn op_super_ref(&mut self) -> JsResult<()> {
        let kv = self.peek(0)?;
        let k = match self.to_prop_key(&kv)? {
            PropKey::Str(name) => Value::String(Str::from_rc(name)),
            PropKey::Sym(s) => Value::Symbol(s),
        };
        let Value::Object(home) = self.hidden_value(HOME)? else { return Err(self.throw(ErrorType::Syntax, "'super' is allowed only in methods")) };
        let proto = self.object(home)?.proto.map_or(Value::Null, Value::Object);
        self.replace(1, proto)?;
        self.push(k)
    }

    /// The three operands of a `super` property: `this`, the prototype and the key.
    fn super_operands(&mut self, depth: usize) -> JsResult<(Value, ObjId, PropKey)> {
        let k = self.peek(depth)?;
        let proto = self.peek(depth + 1)?;
        let this = self.peek(depth + 2)?;
        let k = self.to_prop_key(&k)?;
        let Value::Object(p) = proto else {
            let name = self.pk_name(&k)?;
            return Err(self.type_error(&format!("cannot use super property '{name}': the home object has no prototype")));
        };
        Ok((this, p, k))
    }

    /// The key as text for a message.
    fn pk_name(&mut self, k: &PropKey) -> JsResult<String> {
        match k {
            PropKey::Str(name) => Ok(String::from_utf16_lossy(name)),
            PropKey::Sym(s) => self.sym_name(s),
        }
    }

    /// The property `k` of `start` or an object it inherits from.
    fn find_pk(&mut self, start: ObjId, k: &PropKey) -> JsResult<Option<Prop>> {
        match k {
            PropKey::Str(name) => self.find_property(start, name),
            PropKey::Sym(s) => self.find_sym(start, s),
        }
    }

    /// [[Get]] of `k` from `start` with `receiver` as `this`.
    fn get_pk_from(&mut self, start: ObjId, k: &PropKey, receiver: &Value) -> JsResult<Value> {
        match k {
            PropKey::Str(name) => self.get_from(start, name, receiver),
            PropKey::Sym(s) => self.get_sym_from(start, s, receiver),
        }
    }

    /// `this home key` → `value`, or with `with_this` → `value this`.
    pub(crate) fn op_super_get(&mut self, with_this: bool) -> JsResult<()> {
        let (this, p, k) = self.super_operands(0)?;
        let v = self.get_pk_from(p, &k, &this)?;
        self.replace(3, v)?;
        if with_this { self.push(this) } else { Ok(()) }
    }

    /// `this home key value` → `value`.
    pub(crate) fn op_super_set(&mut self) -> JsResult<()> {
        let v = self.peek(0)?;
        let (this, p, k) = self.super_operands(1)?;
        self.super_put(p, k, v.clone(), &this)?;
        self.replace(4, v)
    }

    /// `this home key` → the old or new number of a `++` or `--` on a `super` property.
    pub(crate) fn op_super_inc(&mut self, prefix: bool, dec: bool) -> JsResult<()> {
        let (this, p, k) = self.super_operands(0)?;
        let v = self.get_pk_from(p, &k, &this)?;
        let old = self.to_number(&v)?;
        let new = if dec { old - 1.0 } else { old + 1.0 };
        self.super_put(p, k, Value::Number(new), &this)?;
        self.replace(3, Value::Number(if prefix { new } else { old }))
    }

    /// [[Set]] of `k` on `proto` with `receiver` as `this` (OrdinarySet, section 9.1.9). A write
    /// that cannot be made throws a `TypeError` in strict mode code and in code from a source with
    /// [`crate::bytecode::Source::strict_writes`], and does nothing otherwise (PutValue, section
    /// 6.2.3.2).
    fn super_put(&mut self, proto: ObjId, k: PropKey, v: Value, receiver: &Value) -> JsResult<()> {
        let throw = self.strict_writes();
        let why = match self.find_pk(proto, &k)? {
            Some(Prop::Accessor { set: Some(s), .. }) => return self.call(&Value::Object(s), receiver, &[v]).map(drop),
            Some(Prop::Accessor { set: None, .. }) => Some("it has only a getter"),
            Some(Prop::Data { flags, .. }) if flags & WRITABLE == 0 => Some("it is read-only"),
            _ => None,
        };
        let (r, desc) = match (why, receiver) {
            (Some(why), _) => return self.super_reject(throw, &k, why),
            (None, Value::Object(r)) => match self.get_own_pk(*r, &k)? {
                Some(Prop::Accessor { .. }) => return self.super_reject(throw, &k, "it is an accessor"),
                Some(Prop::Data { flags, .. }) if flags & WRITABLE == 0 => return self.super_reject(throw, &k, "it is read-only"),
                Some(Prop::Data { .. }) => (*r, Desc { value: Some(v), ..Desc::default() }),
                None => (*r, Desc::data(v, WRITABLE | ENUMERABLE | CONFIGURABLE)),
            },
            (None, _) => return self.super_reject(throw, &k, "`this` is a primitive value"),
        };
        self.define_own_pk(r, k, &desc, throw).map(drop)
    }

    fn super_reject(&mut self, throw: bool, k: &PropKey, why: &str) -> JsResult<()> {
        if !throw {
            return Ok(());
        }
        let name = self.pk_name(k)?;
        Err(self.type_error(&format!("cannot assign to property '{name}': {why}")))
    }

    /// Pushes the constructor `super(…)` calls: the prototype of the running derived class
    /// constructor (GetSuperConstructor, section 12.3.5.2).
    pub(crate) fn op_super_ctor(&mut self) -> JsResult<()> {
        let Value::Object(f) = self.hidden_value(FUNC)? else { return Err(self.throw(ErrorType::Syntax, "'super' is allowed only in constructors")) };
        let proto = self.object(f)?.proto.map_or(Value::Null, Value::Object);
        self.push(proto)
    }

    /// `ctor args…` → the object `ctor` constructs with the running code's `new.target`
    /// (section 12.3.5.1, steps 6 to 9).
    pub(crate) fn op_super_call(&mut self, argc: u32) -> JsResult<()> {
        let n = argc as usize;
        let start =
            self.stack.len().checked_sub(n + 1).filter(|&s| s >= self.base()).ok_or_else(|| Abrupt::Internal("super call below the stack".into()))?;
        let ctor = self.stack.get(start).cloned().unwrap_or(Value::Undefined);
        if !self.is_constructor(&ctor) {
            return Err(self.type_error("the superclass is not a constructor"));
        }
        let Value::Object(nt) = self.hidden_value(NEW_TARGET)? else { return Err(self.type_error("super() needs new.target")) };
        let args: Vec<Value> = self.stack.get(start + 1..).map(<[Value]>::to_vec).unwrap_or_default();
        self.construct(start, ctor, args, Some(nt))
    }

    /// `value` → `value`: binds the running derived class constructor's `this` to the object
    /// `super(…)` made; a second binding is a `ReferenceError` (section 8.1.1.3.1).
    pub(crate) fn op_bind_this(&mut self) -> JsResult<()> {
        let v = self.peek(0)?;
        let frame = self.frame()?;
        let (start, own) = (frame.env, frame.this_env);
        let Some(env) = self.hidden_env(start, ARROW_THIS) else { return internal("super() outside a derived constructor") };
        if !self.uninitialized(env, ARROW_THIS) {
            return Err(self.throw(ErrorType::Reference, "super() was already called"));
        }
        if let Some(Env::Decl { bindings, .. }) = self.heap.env_mut(env)
            && let Some(b) = bindings.get_mut(ARROW_THIS)
        {
            b.value = v.clone();
            b.uninitialized = false;
        }
        if own == Some(env) {
            self.frame_mut()?.this = v;
        }
        Ok(())
    }

    /// Pushes `new.target` (section 12.3.8.1).
    pub(crate) fn op_new_target(&mut self) -> JsResult<()> {
        let frame = self.frame()?;
        let v = if frame.code.arrow { self.hidden_value(NEW_TARGET)? } else { frame.new_target.map_or(Value::Undefined, Value::Object) };
        self.push(v)
    }
}
