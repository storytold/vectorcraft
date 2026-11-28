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

use crate::bytecode::{BlockScope, Code, Handler, Op};
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

    /// Sets the value under `k` like [`Self::insert`], but puts a new `k` right after `after`
    /// when that is there.
    pub(crate) fn insert_after(&mut self, after: &[u16], k: Key, v: T) {
        let Some(at) = self.position(after).filter(|_| !self.contains(&k)) else { return self.insert(k, v) };
        self.entries.insert(at + 1, Some((k, v)));
        self.live += 1;
        self.index = None;
        self.rebuild_index();
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

/// A function the embedding defines that takes and gives structured values.
pub(crate) type DataFn = crate::host::HostFunction;

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
    /// `%GeneratorPrototype%`'s `next`, `return` or `throw`.
    Resume(crate::generator::ResumeMode),
}

/// What a function object runs. `Bound` is a function made by `Function.prototype.bind` (section
/// 15.3.4.5): calling it calls `target` with `this` and `args` before the arguments it is given.
/// `Data` is a host function given [`crate::Data`], its arguments read as `read` says. `Eval` is a
/// host function that gives source to run as global code ([`crate::HostItem::Evaluator`]); `end`
/// is called when that code ends. `Pausable` is a built-in that may ask the host for a value the
/// run to wait for ([`crate::interp::Pause`]). A `Native`'s `prefix` is `get ` or `set ` for a
/// built-in accessor's function and empty otherwise; its ECMAScript 2015 `name` starts with it.
#[derive(Clone)]
pub(crate) enum Func {
    Closure { code: Rc<Code>, env: EnvId },
    Native { f: NativeFn, kind: NativeKind, constructor: bool, name: &'static str, prefix: &'static str },
    Pausable { f: crate::interp::PausableFn, name: &'static str },
    Host { f: Rc<HostFn>, name: Rc<str> },
    Data { f: DataFn, name: Rc<str>, read: crate::host::ArgRead },
    Eval { f: DataFn, end: Rc<dyn Fn()>, strict_writes: Rc<dyn Fn() -> Option<bool>>, name: Rc<str> },
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
    /// An error object. One the interpreter threw itself holds a hash of the description it had
    /// then ([`crate::interp::Interp::describe`]); one a script made holds `None`.
    Error(Option<u64>),
    Arguments(Box<ArgMap>),
    /// A `Boolean` object made by ToObject, with its primitive value.
    Boolean(bool),
    /// A `Number` object and its number.
    Number(f64),
    String(Str),
    RegExp(Rc<crate::regexp_vm::Regex>),
    /// A `Date` object and its time value.
    Date(f64),
    /// A Symbol object made by ToObject, with its Symbol (ECMA-262 11th edition, section 19.4).
    Symbol(crate::symbol::Sym),
    /// An Array Iterator or a String Iterator and how far it has gone (sections 22.1.5 and 21.1.5).
    Iterator(Box<crate::builtins::iterator::IterState>),
    /// The Iterator Record of a `for-of` loop, an array pattern or a spread in progress, on the
    /// operand stack only.
    IterRecord(Box<crate::iteration::IterRecord>),
    /// A generator object and its state (ECMA-262 11th edition, section 25.4).
    Generator(Box<crate::generator::GenState>),
    ForIn(Box<ForIn>),
    /// The `Math` object (section 15.8).
    Math,
    /// The `JSON` object (section 15.12).
    Json,
    /// An XML value (ECMA-357 section 9.1).
    Xml(Box<XmlNode>),
    /// An XMLList value (ECMA-357 section 9.2).
    XmlList(Box<XmlListData>),
    Namespace(Box<XmlNs>),
    /// A QName; `attribute` marks an attribute name (ECMA-357 sections 13.3 and 9.3), and
    /// `reference` a name made by E4X syntax, which only an XML value can be accessed with.
    QName {
        name: Box<XmlName>,
        attribute: bool,
        reference: bool,
    },
    /// A `.( )` filter in progress.
    XmlFilter(Box<XmlFilterState>),
    /// An `XMPMeta` object and its metadata ([`crate::xmp_classes`]).
    Xmp(Box<crate::xmp::Meta>),
    /// An object the `XMPFile` constructor made ([`crate::xmp_classes`]).
    XmpFile(Box<PropMap<Value>>),
    /// An object of a class a host defined ([`crate::host_classes`]).
    Host(crate::host_classes::HostObj),
    /// A `UnitValue` object and its measurement ([`crate::unit_value`]).
    UnitValue(Box<crate::unit_value::Measure>),
    /// A promise (ECMA-262 11th edition, section 25.6) and its state.
    Promise(Box<crate::builtins::promise::PromiseData>),
    /// The state that the functions a promise operation makes share; scripts never see it.
    PromiseRecord(Box<crate::builtins::promise::Record>),
    /// A `Map` (ECMA-262 6.0 section 23.1): its entries in the order they were added.
    Map(Box<Coll>),
    /// A `Set` (section 23.2): its values in the order they were added, each with `undefined`.
    Set(Box<Coll>),
    /// A `WeakMap` (section 23.3): values keyed by object. An entry does not keep its key
    /// reachable, and its value is reachable only while its key is.
    WeakMap(Box<WeakColl>),
    /// A `WeakSet` (section 23.4): objects that it does not keep reachable.
    WeakSet(Box<WeakColl>),
    /// An iterator over a Map, a Set or a typed array (sections 23.1.5, 23.2.5 and 22.1.5).
    Iter(Box<IterState>),
    /// An `ArrayBuffer` (section 24.1) and its bytes, which never change length.
    ArrayBuffer(Box<[u8]>),
    /// A `DataView` (section 24.2): a range of a buffer's bytes.
    DataView(Box<View>),
    /// A typed array (section 22.2): elements of one type in a range of a buffer's bytes.
    TypedArray(Box<View>),
}

/// A Map or Set key compared with SameValueZero (section 7.2.10): `NaN` equals `NaN`, `-0`
/// equals `+0`, strings compare by their code units, and Symbols and objects by identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CollKey {
    Undefined,
    Null,
    Bool(bool),
    /// The bits of the number, with every `NaN` as one value and `-0` as `+0`.
    Number(u64),
    String(Rc<[u16]>),
    Symbol(crate::symbol::Sym),
    Object(u32),
}

impl CollKey {
    /// The key of a number.
    pub(crate) fn number(n: f64) -> Self {
        CollKey::Number(if n.is_nan() {
            f64::NAN.to_bits()
        } else if n == 0.0 {
            0
        } else {
            n.to_bits()
        })
    }
}

/// The entries of a Map or a Set in the order they were added. Deleting an entry leaves a hole,
/// so the position an iterator or a `forEach` loop has reached stays valid; a collection removes
/// the holes of a collection with more holes than entries and moves its live iterators with them.
#[derive(Clone, Default)]
pub(crate) struct Coll {
    entries: Vec<Option<(Value, Value)>>,
    index: HashMap<CollKey, usize>,
}

impl Coll {
    /// The number of entries.
    pub(crate) fn len(&self) -> usize {
        self.index.len()
    }

    /// The number of positions, holes included.
    pub(crate) fn slots(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn get(&self, k: &CollKey) -> Option<&Value> {
        let i = *self.index.get(k)?;
        self.entries.get(i)?.as_ref().map(|(_, v)| v)
    }

    pub(crate) fn has(&self, k: &CollKey) -> bool {
        self.index.contains_key(k)
    }

    /// Sets the value under `k`, keeping its position when it is there already. Returns whether
    /// a new entry was added.
    pub(crate) fn set(&mut self, k: CollKey, key: Value, value: Value) -> bool {
        if let Some(&i) = self.index.get(&k)
            && let Some(Some((_, v))) = self.entries.get_mut(i)
        {
            *v = value;
            return false;
        }
        self.index.insert(k, self.entries.len());
        self.entries.push(Some((key, value)));
        true
    }

    /// Deletes the entry under `k`; returns whether there was one.
    pub(crate) fn delete(&mut self, k: &CollKey) -> bool {
        let Some(i) = self.index.remove(k) else { return false };
        if let Some(slot) = self.entries.get_mut(i) {
            *slot = None;
        }
        true
    }

    /// Deletes every entry, leaving the positions in place.
    pub(crate) fn clear(&mut self) {
        self.index.clear();
        self.entries.iter_mut().for_each(|e| *e = None);
    }

    /// The entry at position `i`, or `None` for a hole or a position past the end.
    pub(crate) fn entry(&self, i: usize) -> Option<&(Value, Value)> {
        self.entries.get(i)?.as_ref()
    }

    fn values(&self) -> impl Iterator<Item = &Value> {
        self.entries.iter().flatten().flat_map(|(k, v)| [k, v])
    }

    fn should_compact(&self) -> bool {
        let holes = self.entries.len() - self.index.len();
        holes > 16 && holes > self.index.len()
    }

    /// Removes the holes. Returns, for each old position and the position past the end, the new
    /// position of the first entry at or after it.
    fn compact(&mut self) -> Vec<usize> {
        let mut moved = Vec::with_capacity(self.entries.len() + 1);
        let mut live = 0usize;
        for e in &self.entries {
            moved.push(live);
            live += usize::from(e.is_some());
        }
        moved.push(live);
        self.entries.retain(Option::is_some);
        // The storage the holes held is released along with their charge.
        self.entries.shrink_to_fit();
        self.index.shrink_to_fit();
        for slot in self.index.values_mut() {
            *slot = moved.get(*slot).copied().unwrap_or(*slot);
        }
        moved
    }

    /// The positions the entries and the index have room for, for tests.
    #[cfg(test)]
    pub(crate) fn capacity(&self) -> (usize, usize) {
        (self.entries.capacity(), self.index.capacity())
    }
}

/// The entries of a WeakMap or a WeakSet, by the index of their key; a WeakSet's values are
/// `undefined`.
#[derive(Clone, Default)]
pub(crate) struct WeakColl {
    pub(crate) entries: HashMap<u32, Value>,
}

/// What an iterator object goes through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IterSource {
    Map,
    Set,
    /// A typed array, by index.
    Array,
}

/// What each step of an iterator gives: the key, the value or a `[key, value]` array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IterKind {
    Keys,
    Values,
    Entries,
}

/// An iterator's state. `target` is `None` once the iterator has finished.
#[derive(Clone, Debug)]
pub(crate) struct IterState {
    pub(crate) source: IterSource,
    pub(crate) kind: IterKind,
    pub(crate) target: Option<ObjId>,
    pub(crate) pos: usize,
}

/// The element type of a typed array (section 22.2, table 50).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Elem {
    Int8,
    Uint8,
    Uint8Clamped,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Float32,
    Float64,
}

impl Elem {
    pub(crate) const ALL: [Elem; 9] =
        [Elem::Int8, Elem::Uint8, Elem::Uint8Clamped, Elem::Int16, Elem::Uint16, Elem::Int32, Elem::Uint32, Elem::Float32, Elem::Float64];

    /// The bytes of one element.
    pub(crate) fn size(self) -> usize {
        match self {
            Elem::Int8 | Elem::Uint8 | Elem::Uint8Clamped => 1,
            Elem::Int16 | Elem::Uint16 => 2,
            Elem::Int32 | Elem::Uint32 | Elem::Float32 => 4,
            Elem::Float64 => 8,
        }
    }

    /// The name of the constructor.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Elem::Int8 => "Int8Array",
            Elem::Uint8 => "Uint8Array",
            Elem::Uint8Clamped => "Uint8ClampedArray",
            Elem::Int16 => "Int16Array",
            Elem::Uint16 => "Uint16Array",
            Elem::Int32 => "Int32Array",
            Elem::Uint32 => "Uint32Array",
            Elem::Float32 => "Float32Array",
            Elem::Float64 => "Float64Array",
        }
    }

    /// The position of the type in [`Elem::ALL`].
    pub(crate) fn index(self) -> usize {
        Elem::ALL.iter().position(|e| *e == self).unwrap_or(0)
    }
}

/// A typed array or a `DataView`: `length` elements of `elem` (bytes, for a `DataView`) starting
/// `offset` bytes into `buffer`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct View {
    pub(crate) buffer: ObjId,
    pub(crate) offset: usize,
    pub(crate) length: usize,
    pub(crate) elem: Elem,
}

/// What an XML value is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum XmlKind {
    Element,
    Attribute,
    Text,
    Comment,
    Pi,
}

/// A qualified name. `uri` is `None` for a name that matches any namespace.
#[derive(Clone, Debug)]
pub(crate) struct XmlName {
    pub(crate) uri: Option<Key>,
    pub(crate) local: Key,
    pub(crate) prefix: Option<Key>,
}

/// A namespace. `prefix` is `None` when it is undefined.
#[derive(Clone, Debug)]
pub(crate) struct XmlNs {
    pub(crate) prefix: Option<Key>,
    pub(crate) uri: Key,
}

/// An XML value: a node of a tree, linked by object index.
#[derive(Clone)]
pub(crate) struct XmlNode {
    pub(crate) kind: XmlKind,
    pub(crate) name: Option<XmlName>,
    pub(crate) parent: Option<ObjId>,
    pub(crate) value: Str,
    pub(crate) attrs: Vec<ObjId>,
    pub(crate) children: Vec<ObjId>,
    pub(crate) ns: Vec<XmlNs>,
}

/// An XMLList value: its items, and the object and name a write to an empty list targets.
#[derive(Clone)]
pub(crate) struct XmlListData {
    pub(crate) items: Vec<ObjId>,
    pub(crate) target: Option<ObjId>,
    pub(crate) target_name: Option<(XmlName, bool)>,
}

/// The state of a `.( )` filter: the items to test, the next one's position and those kept.
#[derive(Clone)]
pub(crate) struct XmlFilterState {
    pub(crate) input: Vec<ObjId>,
    pub(crate) pos: usize,
    pub(crate) kept: Vec<ObjId>,
}

impl Class {
    /// The `[[Class]]` name `Object.prototype.toString` reports.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Class::Xml(_) => "XML",
            Class::XmlList(_) => "XMLList",
            Class::Namespace(_) => "Namespace",
            Class::QName { .. } => "QName",
            Class::XmlFilter(_) | Class::Xmp(_) | Class::XmpFile(_) | Class::PromiseRecord(_) => "Object",
            Class::Promise(_) => "Promise",
            Class::Math => "Math",
            Class::Json => "JSON",
            Class::Ordinary
            | Class::ForIn(_)
            | Class::Host(_)
            | Class::Symbol(_)
            | Class::Iterator(_)
            | Class::IterRecord(_)
            | Class::Generator(_) => "Object",
            Class::Array { .. } => "Array",
            Class::Function(_) => "Function",
            Class::Error(_) => "Error",
            Class::Arguments(_) => "Arguments",
            Class::Boolean(_) => "Boolean",
            Class::Number(_) => "Number",
            Class::String(_) => "String",
            Class::RegExp(_) => "RegExp",
            Class::Date(_) => "Date",
            Class::UnitValue(_) => "UnitValue",
            Class::Map(_) => "Map",
            Class::Set(_) => "Set",
            Class::WeakMap(_) => "WeakMap",
            Class::WeakSet(_) => "WeakSet",
            Class::Iter(state) => match state.source {
                IterSource::Map => "Map Iterator",
                IterSource::Set => "Set Iterator",
                IterSource::Array => "Array Iterator",
            },
            Class::ArrayBuffer(_) => "ArrayBuffer",
            Class::DataView(_) => "DataView",
            Class::TypedArray(view) => view.elem.name(),
        }
    }
}

pub(crate) struct Object {
    pub(crate) class: Class,
    pub(crate) proto: Option<ObjId>,
    pub(crate) extensible: bool,
    pub(crate) props: PropMap<Prop>,
    /// The properties whose keys are Symbols, once the object has one.
    pub(crate) syms: Option<Box<crate::symbol::SymProps>>,
}

impl Object {
    pub(crate) fn new(class: Class, proto: Option<ObjId>) -> Self {
        Self { class, proto, extensible: true, props: PropMap::default(), syms: None }
    }
}

/// A binding in a declarative environment.
#[derive(Clone, Debug)]
pub(crate) struct Binding {
    pub(crate) value: Value,
    pub(crate) mutable: bool,
    pub(crate) deletable: bool,
    /// A `let`, `const` or parameter binding not yet initialized: reading or writing it is a
    /// `ReferenceError` (its temporal dead zone).
    pub(crate) uninitialized: bool,
    /// A `const` binding: writing it is a `TypeError`, in strict mode code or not.
    pub(crate) constant: bool,
    /// A `let` or `const` binding, or a function declared in a block: a `var` that eval code
    /// declares cannot take its name.
    pub(crate) lexical: bool,
}

impl Binding {
    /// An initialized binding.
    pub(crate) fn new(value: Value, mutable: bool, deletable: bool) -> Self {
        Self { value, mutable, deletable, uninitialized: false, constant: false, lexical: false }
    }

    /// A `let` (or with `constant`, a `const`) binding not yet initialized.
    pub(crate) fn lexical(constant: bool) -> Self {
        Self { value: Value::Undefined, mutable: !constant, deletable: false, uninitialized: true, constant, lexical: true }
    }
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
/// The estimated bytes of an XML node without its names and text.
pub(crate) const XML_NODE_BYTES: usize = 160;
/// The estimated bytes of an XML list without its items.
pub(crate) const XML_LIST_BYTES: usize = 96;
/// The estimated bytes of a reference from an XML node or list to another.
pub(crate) const XML_REF_BYTES: usize = 8;
/// The estimated bytes of a namespace an XML node holds.
pub(crate) const XML_NS_BYTES: usize = 48;
/// The estimated bytes of a Symbol without its description.
pub(crate) const SYMBOL_BYTES: usize = 48;
/// The estimated bytes of the measurement a `UnitValue` object holds.
pub(crate) const UNIT_VALUE_BYTES: usize = 32;
/// The estimated bytes of a piece of code without its instructions, tables and strings.
const CODE_BYTES: usize = 384;
/// The estimated bytes of a Map or Set position, with its entry in the hash index.
pub(crate) const COLL_ENTRY_BYTES: usize = 96;
/// The estimated bytes of a WeakMap or WeakSet entry.
pub(crate) const WEAK_ENTRY_BYTES: usize = 64;
/// The estimated bytes of the state of an iterator, a typed array or a `DataView`.
pub(crate) const VIEW_BYTES: usize = 48;

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

    /// The objects in the heap, in the order of their cells.
    pub(crate) fn object_ids(&self) -> Vec<ObjId> {
        let ids = self.cells.iter().enumerate().filter(|(_, c)| matches!(c, Cell::Object(_)));
        ids.filter_map(|(i, _)| u32::try_from(i).ok().map(ObjId)).collect()
    }

    pub(crate) fn reset_xmp_files(&mut self) {
        for cell in &mut self.cells {
            if let Cell::Object(object) = cell
                && let Class::XmpFile(fields) = &mut object.class
            {
                crate::xmp_classes::reset_file_state(fields);
            }
        }
    }

    /// Live cells, for tests.
    #[cfg(test)]
    pub(crate) fn live_cells(&self) -> usize {
        self.cells.iter().filter(|c| !matches!(c, Cell::Free)).count()
    }

    /// Marks everything reachable from `roots` and frees the rest. Afterwards [`Heap::bytes`] is
    /// the estimated size of what is left, strings reachable from it included.
    ///
    /// A WeakMap's entry marks its value only once its key is marked by another path, and the
    /// entries of WeakMaps and WeakSets whose keys stay unmarked are removed before the sweep, so
    /// a value that refers back to its own key does not keep it alive. Values waiting for their
    /// key are kept by key, so the marking stays linear in the number of entries.
    pub(crate) fn collect(&mut self, roots: &Roots<'_>) -> HashSet<usize> {
        let mut marks = vec![false; self.cells.len()];
        let mut work: Vec<u32> = Vec::new();
        let mut strings = StringSizes::default();
        let mut live = 0usize;
        let mut pending: HashMap<u32, Vec<Value>> = HashMap::new();
        let mut weak: Vec<u32> = Vec::new();
        for v in &roots.values {
            mark_value(v, &mut work, &mut strings);
        }
        work.extend(roots.objects.iter().map(|o| o.0));
        work.extend(roots.envs.iter().map(|e| e.0));
        for code in &roots.codes {
            mark_code(code, &mut strings);
        }
        let mut reached_codes = 0;
        loop {
            while let Some(&code) = strings.codes.get(reached_codes) {
                reached_codes += 1;
                work.extend(roots.template_objects.get(&code).into_iter().flatten().map(|id| id.0));
            }
            let Some(i) = work.pop() else { break };
            let Some(mark) = marks.get_mut(i as usize) else { continue };
            if *mark {
                continue;
            }
            *mark = true;
            if let Some(values) = pending.remove(&i) {
                for v in &values {
                    mark_value(v, &mut work, &mut strings);
                }
            }
            match self.cells.get(i as usize) {
                Some(Cell::Object(o)) => {
                    live += trace_object(o, &mut work, &mut strings);
                    match &o.class {
                        Class::WeakMap(w) => {
                            weak.push(i);
                            for (k, v) in &w.entries {
                                if marks.get(*k as usize).copied().unwrap_or(false) {
                                    mark_value(v, &mut work, &mut strings);
                                } else if matches!(v, Value::Object(_) | Value::String(_)) {
                                    pending.entry(*k).or_default().push(v.clone());
                                }
                            }
                        }
                        Class::WeakSet(_) => weak.push(i),
                        _ => {}
                    }
                }
                Some(Cell::Env(e)) => live += trace_env(e, &mut work, &mut strings),
                _ => {}
            }
        }
        for i in weak {
            if let Some(Cell::Object(o)) = self.cells.get_mut(i as usize)
                && let Class::WeakMap(w) | Class::WeakSet(w) = &mut o.class
            {
                w.entries.retain(|k, _| marks.get(*k as usize).copied().unwrap_or(false));
                // Only the entries left are charged, and a table with far more room than entries
                // releases the extra room.
                if w.entries.capacity() > 2 * w.entries.len() + 16 {
                    w.entries.shrink_to_fit();
                }
                live += WEAK_ENTRY_BYTES * w.entries.len();
            }
        }
        for (i, (cell, marked)) in self.cells.iter_mut().zip(&marks).enumerate() {
            if !marked && !matches!(cell, Cell::Free) {
                *cell = Cell::Free;
                self.free.push(u32::try_from(i).unwrap_or(u32::MAX));
            }
        }
        live = live.saturating_sub(self.compact_collections());
        self.bytes = live + strings.bytes;
        self.collections += 1;
        strings.codes.into_iter().collect()
    }

    /// Removes the holes of the Maps and Sets that have more holes than entries, and moves the
    /// positions of their iterators with them. Returns the bytes the holes held.
    fn compact_collections(&mut self) -> usize {
        let mut moved: HashMap<u32, Vec<usize>> = HashMap::new();
        let mut freed = 0usize;
        for (i, cell) in self.cells.iter_mut().enumerate() {
            if let Cell::Object(o) = cell
                && let Class::Map(c) | Class::Set(c) = &mut o.class
                && c.should_compact()
            {
                freed += COLL_ENTRY_BYTES * (c.slots() - c.len());
                moved.insert(u32::try_from(i).unwrap_or(u32::MAX), c.compact());
            }
        }
        if moved.is_empty() {
            return 0;
        }
        for cell in &mut self.cells {
            if let Cell::Object(o) = cell
                && let Class::Iter(state) = &mut o.class
                && state.source != IterSource::Array
                && let Some(map) = state.target.and_then(|t| moved.get(&t.0))
            {
                state.pos = map.get(state.pos).or(map.last()).copied().unwrap_or(0);
            }
        }
        freed
    }
}

/// What the collector starts from.
#[derive(Default)]
pub(crate) struct Roots<'a> {
    pub(crate) values: Vec<&'a Value>,
    pub(crate) objects: Vec<ObjId>,
    pub(crate) envs: Vec<EnvId>,
    pub(crate) codes: Vec<&'a Code>,
    pub(crate) template_objects: HashMap<usize, Vec<ObjId>>,
}

/// Sums the sizes of the strings and property names reached, counting each node and each
/// code-unit buffer once.
#[derive(Default)]
struct StringSizes {
    seen: HashSet<usize>,
    codes: Vec<usize>,
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

    /// Adds a Symbol and its description, once however many values and keys share it.
    fn add_symbol(&mut self, s: &crate::symbol::Sym) {
        if self.seen.insert(s.addr()) {
            self.bytes += SYMBOL_BYTES;
            if let Some(d) = s.description() {
                self.add(d);
            }
        }
    }
}

fn mark_value(v: &Value, work: &mut Vec<u32>, strings: &mut StringSizes) {
    match v {
        Value::Object(o) => work.push(o.0),
        Value::String(s) => strings.add(s),
        Value::Symbol(s) => strings.add_symbol(s),
        _ => {}
    }
}

fn mark_prop(prop: &Prop, work: &mut Vec<u32>, strings: &mut StringSizes) {
    match prop {
        Prop::Data { value, .. } => mark_value(value, work, strings),
        Prop::Accessor { get, set, .. } => work.extend(get.iter().chain(set.iter()).map(|f| f.0)),
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
        strings.codes.push(code as *const Code as usize);
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
            + code.pattern_names.len()
            + code.vars.len()
            + 2 * code.fun_decls.len()
            + code.templates.iter().map(|t| 2 + t.cooked.len() + t.raw.len()).sum::<usize>();
        let tables = code.finally_tables.iter().map(|t| 24 + 4 * t.len()).sum::<usize>();
        let scopes = code
            .scopes
            .iter()
            .map(|s| size_of::<BlockScope>() + size_of::<(Key, bool)>() * s.lets.len() + size_of::<(Key, u32)>() * s.funcs.len())
            .sum::<usize>();
        strings.bytes += CODE_BYTES
            + scopes
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
        let keys = code.names.iter().chain(&code.params).chain(&code.pattern_names).chain(&code.vars).chain(code.fun_decls.iter().map(|(k, _)| k));
        for k in keys.chain(&code.self_name).chain(&code.decl_name).chain(&code.rest) {
            strings.add_key(k);
        }
        for scope in &code.scopes {
            scope.lets.iter().map(|(k, _)| k).chain(scope.funcs.iter().map(|(k, _)| k)).for_each(|k| strings.add_key(k));
        }
        for t in &code.templates {
            t.cooked.iter().chain(&t.raw).for_each(|s| strings.add(s));
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
        mark_prop(prop, work, strings);
    }
    let syms = o.syms.as_ref().map_or(0, |m| m.len());
    for (k, prop) in o.syms.iter().flat_map(|m| m.iter()) {
        strings.add_symbol(k);
        mark_prop(prop, work, strings);
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
        Class::RegExp(re) => {
            if strings.seen.insert(Rc::as_ptr(re) as *const () as usize) {
                strings.bytes += re.heap_bytes();
            }
        }
        Class::ForIn(state) => {
            work.extend(state.object.iter().map(|o| o.0));
            strings.bytes += state.keys.iter().map(|k| string_bytes(k.len())).sum::<usize>();
        }
        Class::Symbol(s) => strings.add_symbol(s),
        Class::IterRecord(r) => {
            mark_value(&r.iterator, work, strings);
            mark_value(&r.next, work, strings);
            match &r.walk {
                Some(crate::iteration::Walk::Array(a, _)) => work.push(a.0),
                Some(crate::iteration::Walk::String(s, _)) => strings.add(s),
                None => {}
            }
        }
        Class::Iterator(state) => match &**state {
            crate::builtins::iterator::IterState::Array { target, .. } => work.extend(target.iter().map(|o| o.0)),
            crate::builtins::iterator::IterState::String { text, .. } => text.iter().for_each(|s| strings.add(s)),
        },
        Class::Generator(state) => {
            if let Some(saved) = state.saved() {
                work.push(saved.env.0);
                work.push(saved.var_env.0);
                work.extend(saved.callee.iter().map(|o| o.0));
                work.extend(saved.marks.iter().map(|&(id, _, _)| id.0));
                mark_value(&saved.this, work, strings);
                mark_value(&saved.completion, work, strings);
                for v in &saved.stack {
                    mark_value(v, work, strings);
                }
                mark_code(&saved.code, strings);
                for ((code, _), _) in &saved.sites {
                    mark_code(code, strings);
                }
                extra = crate::generator::saved_bytes(saved);
            }
        }
        Class::Xml(node) => {
            work.extend(node.parent.iter().chain(&node.attrs).chain(&node.children).map(|o| o.0));
            strings.add(&node.value);
            if let Some(name) = &node.name {
                add_name(name, strings);
            }
            for ns in &node.ns {
                add_ns(ns, strings);
            }
            extra = XML_NODE_BYTES + XML_REF_BYTES * (node.attrs.len() + node.children.len()) + XML_NS_BYTES * node.ns.len();
        }
        Class::XmlList(list) => {
            work.extend(list.target.iter().chain(&list.items).map(|o| o.0));
            if let Some((name, _)) = &list.target_name {
                add_name(name, strings);
            }
            extra = XML_LIST_BYTES + XML_REF_BYTES * list.items.len();
        }
        Class::Namespace(ns) => add_ns(ns, strings),
        Class::QName { name, .. } => add_name(name, strings),
        Class::XmlFilter(state) => {
            work.extend(state.input.iter().chain(&state.kept).map(|o| o.0));
            extra = XML_LIST_BYTES + XML_REF_BYTES * (state.input.len() + state.kept.len());
        }
        Class::Map(c) | Class::Set(c) => {
            for v in c.values() {
                mark_value(v, work, strings);
            }
            extra = COLL_ENTRY_BYTES * c.slots();
        }
        // Their entries are marked and counted by the collector's weak pass.
        Class::WeakMap(_) | Class::WeakSet(_) => {}
        Class::Iter(state) => {
            work.extend(state.target.iter().map(|o| o.0));
            extra = VIEW_BYTES;
        }
        Class::ArrayBuffer(bytes) => extra = bytes.len(),
        Class::DataView(view) | Class::TypedArray(view) => {
            work.push(view.buffer.0);
            extra = VIEW_BYTES;
        }
        Class::Xmp(meta) => extra = meta.heap_bytes(),
        Class::XmpFile(fields) => {
            for (name, value) in fields.iter() {
                strings.add_key(name);
                mark_value(value, work, strings);
            }
            extra = std::mem::size_of::<PropMap<Value>>() + PROP_BYTES * fields.len();
        }
        Class::UnitValue(_) => extra = UNIT_VALUE_BYTES,
        Class::Host(h) => extra = crate::host_classes::handle_bytes(h.handle.len()),
        Class::Promise(p) => {
            for v in p.values() {
                mark_value(&v, work, strings);
            }
            if let Some((code, _)) = &p.site {
                mark_code(code, strings);
            }
            extra = p.bytes();
        }
        Class::PromiseRecord(r) => {
            for v in r.values() {
                mark_value(&v, work, strings);
            }
            extra = crate::builtins::promise::RECORD_BYTES;
        }
        _ => {}
    }
    OBJECT_BYTES + PROP_BYTES * (o.props.len() + syms) + extra
}

fn add_name(name: &XmlName, strings: &mut StringSizes) {
    strings.add_key(&name.local);
    name.uri.iter().chain(&name.prefix).for_each(|k| strings.add_key(k));
}

fn add_ns(ns: &XmlNs, strings: &mut StringSizes) {
    strings.add_key(&ns.uri);
    ns.prefix.iter().for_each(|k| strings.add_key(k));
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
        let _ = heap.collect(&Roots { objects: vec![id], ..Roots::default() });
        assert_eq!(heap.bytes, OBJECT_BYTES + 2 * PROP_BYTES + string_bytes(1000) + string_bytes(5));
    }

    #[test]
    fn xmp_file_buffers_are_traced_and_released_at_the_file_run_boundary() {
        let mut heap = Heap::default();
        let pending = heap.alloc_object(Object::new(Class::Ordinary, None));
        let mut fields = PropMap::default();
        fields.insert(key("__vcXmpPacket"), Value::str(&"p".repeat(8192)));
        fields.insert(key("__vcXmpPending"), Value::Object(pending));
        fields.insert(key("__vcXmpPath"), Value::str("kept.xmp"));
        let file = heap.alloc_object(Object::new(Class::XmpFile(Box::new(fields)), None));
        let roots = Roots { objects: vec![file], ..Roots::default() };
        heap.collect(&roots);
        assert!(heap.object(pending).is_some(), "native values keep their objects reachable");
        let before = heap.bytes;
        assert!(before >= string_bytes(8192), "native packet strings count toward the heap limit");
        heap.reset_xmp_files();
        heap.collect(&roots);
        assert!(heap.object(pending).is_none(), "the prior run's pending value is released");
        assert!(before - heap.bytes >= string_bytes(8192), "the prior run's packet is released");
        let Class::XmpFile(fields) = &heap.object(file).unwrap().class else { panic!("native file") };
        assert!(matches!(fields.get(&key("__vcXmpPath")), Some(Value::String(path)) if path.to_rust_string() == "kept.xmp"));
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
