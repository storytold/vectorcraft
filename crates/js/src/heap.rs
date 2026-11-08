//! The heap: objects and environments in one arena, referred to by index, and a mark-sweep
//! collector over it.
//!
//! A cell is either free or holds an [`Object`] or an [`Env`]. The collector marks every cell
//! reachable from the roots the interpreter passes in, with a worklist rather than recursion, and
//! frees the rest; freed indices are reused by later allocations. Cycles between objects are freed
//! like anything else that is unreachable. [`Heap::bytes`] is an estimate of the memory the heap
//! holds, strings reachable from it included; the interpreter compares it with its heap limit.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::bytecode::{Code, Handler, Op};
use crate::value::{Str, Value};

/// A property name: its UTF-16 code units.
pub(crate) type Key = Rc<[u16]>;

pub(crate) fn key(s: &str) -> Key {
    s.encode_utf16().collect::<Vec<u16>>().into()
}

/// An object's index in the heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ObjId(pub(crate) u32);

/// An environment's index in the heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct EnvId(pub(crate) u32);

/// A map from property names to values that keeps insertion order (the order `for-in` lists own
/// properties in). Small maps are searched in order; larger ones keep a hash index.
#[derive(Clone, Debug)]
pub(crate) struct PropMap<T> {
    entries: Vec<Option<(Key, T)>>,
    index: Option<HashMap<Key, usize>>,
    live: usize,
}

/// Maps with more entries than this keep a hash index.
const INDEX_AFTER: usize = 8;

impl<T> Default for PropMap<T> {
    fn default() -> Self {
        Self { entries: Vec::new(), index: None, live: 0 }
    }
}

impl<T> PropMap<T> {
    fn position(&self, k: &[u16]) -> Option<usize> {
        match &self.index {
            Some(index) => index.get(k).copied(),
            None => self.entries.iter().position(|e| e.as_ref().is_some_and(|(ek, _)| &**ek == k)),
        }
    }

    pub(crate) fn get(&self, k: &[u16]) -> Option<&T> {
        let i = self.position(k)?;
        self.entries.get(i)?.as_ref().map(|(_, v)| v)
    }

    pub(crate) fn get_mut(&mut self, k: &[u16]) -> Option<&mut T> {
        let i = self.position(k)?;
        self.entries.get_mut(i)?.as_mut().map(|(_, v)| v)
    }

    pub(crate) fn contains(&self, k: &[u16]) -> bool {
        self.position(k).is_some()
    }

    /// Sets the value under `k`, keeping its place when it is there already.
    pub(crate) fn insert(&mut self, k: Key, v: T) {
        if let Some(slot) = self.get_mut(&k) {
            *slot = v;
            return;
        }
        let i = self.entries.len();
        if let Some(index) = &mut self.index {
            index.insert(k.clone(), i);
        }
        self.entries.push(Some((k, v)));
        self.live += 1;
        if self.index.is_none() && self.entries.len() > INDEX_AFTER {
            self.rebuild_index();
        }
    }

    pub(crate) fn remove(&mut self, k: &[u16]) -> Option<T> {
        let i = self.position(k)?;
        let (_, v) = self.entries.get_mut(i)?.take()?;
        if let Some(index) = &mut self.index {
            index.remove(k);
        }
        self.live -= 1;
        if self.entries.len() > 2 * INDEX_AFTER && self.live * 2 < self.entries.len() {
            self.entries.retain(Option::is_some);
            self.rebuild_index();
        }
        Some(v)
    }

    fn rebuild_index(&mut self) {
        if self.entries.len() <= INDEX_AFTER {
            self.index = None;
            return;
        }
        let index = self.entries.iter().enumerate().filter_map(|(i, e)| e.as_ref().map(|(k, _)| (k.clone(), i))).collect();
        self.index = Some(index);
    }

    pub(crate) fn len(&self) -> usize {
        self.live
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&Key, &T)> {
        self.entries.iter().filter_map(|e| e.as_ref().map(|(k, v)| (k, v)))
    }

    pub(crate) fn keys(&self) -> Vec<Key> {
        self.iter().map(|(k, _)| k.clone()).collect()
    }
}

pub(crate) const WRITABLE: u8 = 1;
pub(crate) const ENUMERABLE: u8 = 2;
pub(crate) const CONFIGURABLE: u8 = 4;
/// Writable, enumerable and configurable: a property made by assignment.
pub(crate) const OPEN: u8 = WRITABLE | ENUMERABLE | CONFIGURABLE;
/// Writable and configurable but not enumerable: the built-in methods.
pub(crate) const HIDDEN: u8 = WRITABLE | CONFIGURABLE;

/// A property (section 8.6.1).
#[derive(Clone, Debug)]
pub(crate) enum Prop {
    Data { value: Value, flags: u8 },
    Accessor { get: Option<ObjId>, set: Option<ObjId>, flags: u8 },
}

impl Prop {
    pub(crate) fn flags(&self) -> u8 {
        match self {
            Prop::Data { flags, .. } | Prop::Accessor { flags, .. } => *flags,
        }
    }

    pub(crate) fn enumerable(&self) -> bool {
        self.flags() & ENUMERABLE != 0
    }

    pub(crate) fn configurable(&self) -> bool {
        self.flags() & CONFIGURABLE != 0
    }
}

/// A built-in function implemented in Rust: `(interp, this, arguments, called with new)`.
pub(crate) type NativeFn = fn(&mut crate::interp::Interp, Value, &[Value], bool) -> Result<Value, crate::interp::Abrupt>;

/// A function the embedding defines.
pub(crate) type HostFn = dyn Fn(&[crate::JsValue]) -> Result<crate::JsValue, String>;

/// The built-ins the interpreter treats specially when it calls them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeKind {
    Plain,
    /// `Function.prototype.call`.
    Call,
    /// `Function.prototype.apply`.
    Apply,
    /// The global `eval`: a call to it by that name is a direct eval.
    Eval,
}

/// What a function object runs. `Bound` is a function made by `Function.prototype.bind` (section
/// 15.3.4.5): calling it calls `target` with `this` and `args` before the arguments it is given.
#[derive(Clone)]
pub(crate) enum Func {
    Closure { code: Rc<Code>, env: EnvId },
    Native { f: NativeFn, kind: NativeKind, constructor: bool, name: &'static str },
    Host { f: Rc<HostFn>, name: Rc<str> },
    Bound { target: ObjId, this: Value, args: Rc<[Value]> },
}

/// A sloppy mode `arguments` object's link to its function's parameters (section 10.6): index
/// `i` reads and writes the binding `names[i]` while that is `Some`.
#[derive(Clone, Debug)]
pub(crate) struct ArgMap {
    pub(crate) env: EnvId,
    pub(crate) names: Vec<Option<Key>>,
}

/// A `for-in` loop's state: the names listed when it started, and the object they came from.
#[derive(Clone, Debug)]
pub(crate) struct ForIn {
    pub(crate) keys: Vec<Key>,
    pub(crate) pos: usize,
    pub(crate) object: Option<ObjId>,
}

/// What kind of object it is, with the internal state that kind has.
#[derive(Clone)]
pub(crate) enum Class {
    Ordinary,
    Array {
        length: u32,
        length_writable: bool,
    },
    Function(Func),
    Error,
    Arguments(Box<ArgMap>),
    /// A `Boolean` object made by ToObject, with its primitive value.
    Boolean(bool),
    /// A `Number` object made by ToObject.
    Number,
    String(Str),
    RegExp,
    ForIn(Box<ForIn>),
    /// The `Math` object (section 15.8).
    Math,
    /// The `JSON` object (section 15.12).
    Json,
}

impl Class {
    /// The `[[Class]]` name `Object.prototype.toString` reports.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Class::Math => "Math",
            Class::Json => "JSON",
            Class::Ordinary | Class::ForIn(_) => "Object",
            Class::Array { .. } => "Array",
            Class::Function(_) => "Function",
            Class::Error => "Error",
            Class::Arguments(_) => "Arguments",
            Class::Boolean(_) => "Boolean",
            Class::Number => "Number",
            Class::String(_) => "String",
            Class::RegExp => "RegExp",
        }
    }
}

pub(crate) struct Object {
    pub(crate) class: Class,
    pub(crate) proto: Option<ObjId>,
    pub(crate) extensible: bool,
    pub(crate) props: PropMap<Prop>,
}

impl Object {
    pub(crate) fn new(class: Class, proto: Option<ObjId>) -> Self {
        Self { class, proto, extensible: true, props: PropMap::default() }
    }
}

/// A binding in a declarative environment.
#[derive(Clone, Debug)]
pub(crate) struct Binding {
    pub(crate) value: Value,
    pub(crate) mutable: bool,
    pub(crate) deletable: bool,
}

/// An environment record (section 10.2.1) with its outer environment.
pub(crate) enum Env {
    Decl {
        bindings: PropMap<Binding>,
        outer: Option<EnvId>,
    },
    /// The global object, or a `with` statement's object (`with_this`: calls to its functions get
    /// it as `this`).
    Obj {
        object: ObjId,
        with_this: bool,
        outer: Option<EnvId>,
    },
}

impl Env {
    pub(crate) fn outer(&self) -> Option<EnvId> {
        match self {
            Env::Decl { outer, .. } | Env::Obj { outer, .. } => *outer,
        }
    }
}

enum Cell {
    Free,
    Object(Box<Object>),
    Env(Box<Env>),
}

/// The estimated bytes of a property or a binding entry. A collection adds its name's code units
/// separately, counting a buffer that several names or strings share once.
pub(crate) const PROP_BYTES: usize = 64;
/// The estimated bytes of an object without its properties.
pub(crate) const OBJECT_BYTES: usize = 96;
/// The estimated bytes of an environment without its bindings.
pub(crate) const ENV_BYTES: usize = 64;
/// The estimated bytes of a rope node.
pub(crate) const ROPE_BYTES: usize = 48;
/// The estimated bytes of each argument a bound function holds.
pub(crate) const BOUND_ARG_BYTES: usize = 24;
/// The estimated bytes of a piece of code without its instructions, tables and strings.
const CODE_BYTES: usize = 384;

pub(crate) fn string_bytes(len: usize) -> usize {
    32 + 2 * len
}

#[derive(Default)]
pub(crate) struct Heap {
    cells: Vec<Cell>,
    free: Vec<u32>,
    /// An estimate of the bytes the heap holds: the live total from the last collection plus what
    /// was allocated since.
    pub(crate) bytes: usize,
    /// How many collections have run.
    pub(crate) collections: usize,
    /// Counts the properties whose names start with a digit (the array indices among them) added
    /// to any object, so an operation that lists the indices an object has can tell whether a
    /// script added one since.
    pub(crate) index_epoch: u64,
}

impl Heap {
    fn alloc(&mut self, cell: Cell, bytes: usize) -> u32 {
        self.bytes += bytes;
        if let Some(i) = self.free.pop()
            && let Some(slot) = self.cells.get_mut(i as usize)
        {
            *slot = cell;
            return i;
        }
        self.cells.push(cell);
        u32::try_from(self.cells.len() - 1).unwrap_or(u32::MAX)
    }

    pub(crate) fn alloc_object(&mut self, object: Object) -> ObjId {
        let bytes = OBJECT_BYTES + PROP_BYTES * object.props.len();
        ObjId(self.alloc(Cell::Object(Box::new(object)), bytes))
    }

    pub(crate) fn alloc_env(&mut self, env: Env) -> EnvId {
        let bytes = ENV_BYTES + if let Env::Decl { bindings, .. } = &env { PROP_BYTES * bindings.len() } else { 0 };
        EnvId(self.alloc(Cell::Env(Box::new(env)), bytes))
    }

    pub(crate) fn object(&self, id: ObjId) -> Option<&Object> {
        match self.cells.get(id.0 as usize) {
            Some(Cell::Object(o)) => Some(o),
            _ => None,
        }
    }

    pub(crate) fn object_mut(&mut self, id: ObjId) -> Option<&mut Object> {
        match self.cells.get_mut(id.0 as usize) {
            Some(Cell::Object(o)) => Some(o),
            _ => None,
        }
    }

    pub(crate) fn env(&self, id: EnvId) -> Option<&Env> {
        match self.cells.get(id.0 as usize) {
            Some(Cell::Env(e)) => Some(e),
            _ => None,
        }
    }

    pub(crate) fn env_mut(&mut self, id: EnvId) -> Option<&mut Env> {
        match self.cells.get_mut(id.0 as usize) {
            Some(Cell::Env(e)) => Some(e),
            _ => None,
        }
    }

    /// Live cells, for tests.
    #[cfg(test)]
    pub(crate) fn live_cells(&self) -> usize {
        self.cells.iter().filter(|c| !matches!(c, Cell::Free)).count()
    }

    /// Marks everything reachable from `roots` and frees the rest. Afterwards [`Heap::bytes`] is
    /// the estimated size of what is left, strings reachable from it included.
    pub(crate) fn collect(&mut self, roots: &Roots<'_>) {
        let mut marks = vec![false; self.cells.len()];
        let mut work: Vec<u32> = Vec::new();
        let mut strings = StringSizes::default();
        let mut live = 0usize;
        for v in &roots.values {
            mark_value(v, &mut work, &mut strings);
        }
        work.extend(roots.objects.iter().map(|o| o.0));
        work.extend(roots.envs.iter().map(|e| e.0));
        for code in &roots.codes {
            mark_code(code, &mut strings);
        }
        while let Some(i) = work.pop() {
            let Some(mark) = marks.get_mut(i as usize) else { continue };
            if *mark {
                continue;
            }
            *mark = true;
            match self.cells.get(i as usize) {
                Some(Cell::Object(o)) => live += trace_object(o, &mut work, &mut strings),
                Some(Cell::Env(e)) => live += trace_env(e, &mut work, &mut strings),
                _ => {}
            }
        }
        for (i, (cell, marked)) in self.cells.iter_mut().zip(&marks).enumerate() {
            if !marked && !matches!(cell, Cell::Free) {
                *cell = Cell::Free;
                self.free.push(u32::try_from(i).unwrap_or(u32::MAX));
            }
        }
        self.bytes = live + strings.bytes;
        self.collections += 1;
    }
}

/// What the collector starts from.
#[derive(Default)]
pub(crate) struct Roots<'a> {
    pub(crate) values: Vec<&'a Value>,
    pub(crate) objects: Vec<ObjId>,
    pub(crate) envs: Vec<EnvId>,
    pub(crate) codes: Vec<&'a Code>,
}

/// Sums the sizes of the strings and property names reached, counting each node and each
/// code-unit buffer once.
#[derive(Default)]
struct StringSizes {
    seen: HashSet<usize>,
    bytes: usize,
}

impl StringSizes {
    fn add(&mut self, s: &Str) {
        let mut stack = vec![s.clone()];
        while let Some(s) = stack.pop() {
            if !self.seen.insert(s.node_addr()) {
                continue;
            }
            match s.halves() {
                Some((l, r)) => {
                    self.bytes += ROPE_BYTES;
                    stack.push(l);
                    stack.push(r);
                }
                None => {
                    if s.buffer_addr().is_none_or(|a| self.seen.insert(a)) {
                        self.bytes += string_bytes(s.len());
                    }
                }
            }
        }
    }

    /// Adds a property or binding name, unless a string or another name shares its buffer.
    fn add_key(&mut self, k: &Key) {
        if self.seen.insert(Rc::as_ptr(k) as *const () as usize) {
            self.bytes += string_bytes(k.len());
        }
    }
}

fn mark_value(v: &Value, work: &mut Vec<u32>, strings: &mut StringSizes) {
    match v {
        Value::Object(o) => work.push(o.0),
        Value::String(s) => strings.add(s),
        _ => {}
    }
}

/// The estimated bytes of `code` with the functions nested in it and its source text, for
/// charging code made at run time to the heap before anything refers to it.
pub(crate) fn code_bytes(code: &Code) -> usize {
    let mut sizes = StringSizes::default();
    mark_code(code, &mut sizes);
    sizes.bytes
}

/// Adds the estimated size of `code` and the functions nested in it, counting each piece of code,
/// each source text and each string once however many closures share them.
fn mark_code(code: &Code, strings: &mut StringSizes) {
    let mut stack = vec![code];
    while let Some(code) = stack.pop() {
        if !strings.seen.insert(code as *const Code as usize) {
            continue;
        }
        if strings.seen.insert(Rc::as_ptr(&code.source) as usize) {
            strings.bytes += code.source.lines.heap_bytes();
            // The text can share its buffer with the string eval was given.
            if strings.seen.insert(Rc::as_ptr(&code.source.text) as *const () as usize) {
                strings.bytes += string_bytes(code.source.text.len());
            }
        }
        let handles = code.strings.len()
            + 2 * code.regexps.len()
            + code.names.len()
            + code.functions.len()
            + code.params.len()
            + code.vars.len()
            + 2 * code.fun_decls.len();
        let tables = code.finally_tables.iter().map(|t| 24 + 4 * t.len()).sum::<usize>();
        strings.bytes += CODE_BYTES
            + size_of::<Op>() * code.ops.len()
            + 4 * code.positions.len()
            + 8 * code.numbers.len()
            + size_of::<Handler>() * code.handlers.len()
            + 16 * handles
            + tables;
        for s in &code.strings {
            strings.add(s);
        }
        for (pattern, flags) in &code.regexps {
            strings.add(pattern);
            strings.add(flags);
        }
        let keys = code.names.iter().chain(&code.params).chain(&code.vars).chain(code.fun_decls.iter().map(|(k, _)| k));
        for k in keys.chain(&code.self_name) {
            strings.add_key(k);
        }
        stack.extend(code.functions.iter().map(|f| &**f));
    }
}

fn trace_object(o: &Object, work: &mut Vec<u32>, strings: &mut StringSizes) -> usize {
    if let Some(p) = o.proto {
        work.push(p.0);
    }
    for (k, prop) in o.props.iter() {
        strings.add_key(k);
        match prop {
            Prop::Data { value, .. } => mark_value(value, work, strings),
            Prop::Accessor { get, set, .. } => work.extend(get.iter().chain(set.iter()).map(|f| f.0)),
        }
    }
    let mut extra = 0;
    match &o.class {
        Class::Function(Func::Closure { env, code }) => {
            work.push(env.0);
            mark_code(code, strings);
        }
        Class::Function(Func::Bound { target, this, args }) => {
            work.push(target.0);
            mark_value(this, work, strings);
            for a in args.iter() {
                mark_value(a, work, strings);
            }
            extra = BOUND_ARG_BYTES * args.len();
        }
        Class::Arguments(map) => work.push(map.env.0),
        Class::String(s) => strings.add(s),
        Class::ForIn(state) => {
            work.extend(state.object.iter().map(|o| o.0));
            strings.bytes += state.keys.iter().map(|k| string_bytes(k.len())).sum::<usize>();
        }
        _ => {}
    }
    OBJECT_BYTES + PROP_BYTES * o.props.len() + extra
}

fn trace_env(e: &Env, work: &mut Vec<u32>, strings: &mut StringSizes) -> usize {
    match e {
        Env::Decl { bindings, outer } => {
            work.extend(outer.iter().map(|o| o.0));
            for (k, b) in bindings.iter() {
                strings.add_key(k);
                mark_value(&b.value, work, strings);
            }
            ENV_BYTES + PROP_BYTES * bindings.len()
        }
        Env::Obj { object, outer, .. } => {
            work.push(object.0);
            work.extend(outer.iter().map(|o| o.0));
            ENV_BYTES
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Class, Heap, OBJECT_BYTES, Object, PROP_BYTES, Prop, PropMap, Roots, key, string_bytes};
    use crate::value::{Str, Value};

    #[test]
    fn a_collection_counts_a_name_and_a_value_that_share_a_buffer_once() {
        let mut heap = Heap::default();
        let name = key(&"n".repeat(1000));
        let mut object = Object::new(Class::Ordinary, None);
        object.props.insert(name.clone(), Prop::Data { value: Value::String(Str::from_rc(name.clone())), flags: 0 });
        object.props.insert(key("other"), Prop::Data { value: Value::Undefined, flags: 0 });
        let id = heap.alloc_object(object);
        heap.collect(&Roots { objects: vec![id], ..Roots::default() });
        assert_eq!(heap.bytes, OBJECT_BYTES + 2 * PROP_BYTES + string_bytes(1000) + string_bytes(5));
    }

    #[test]
    fn prop_maps_keep_insertion_order_through_removals_and_growth() {
        let mut m = PropMap::default();
        for i in 0..40 {
            m.insert(key(&format!("k{i}")), i);
        }
        for i in (0..40).step_by(2) {
            assert_eq!(m.remove(&key(&format!("k{i}"))), Some(i));
        }
        m.insert(key("k1"), 100);
        m.insert(key("new"), 7);
        let keys: Vec<String> = m.iter().map(|(k, _)| String::from_utf16_lossy(k)).collect();
        let mut want: Vec<String> = (1..40).step_by(2).map(|i| format!("k{i}")).collect();
        want.push("new".into());
        assert_eq!(keys, want);
        assert_eq!(m.get(&key("k1")), Some(&100));
        assert_eq!(m.len(), 21);
        assert!(!m.contains(&key("k0")));
    }
}
