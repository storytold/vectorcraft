//! Structured values between a script and its host: [`Data`] (JSON's shapes and `undefined`), the
//! checks a value passes on its way across, and the frozen objects a host adds to the global
//! environment.
//!
//! A value read from a script may hold plain objects, arrays, strings, finite numbers, booleans,
//! `null` and `undefined`. A cycle, a value too deep or too large, a function, a getter, a date, a
//! regular expression, a boxed primitive, a hole in an array and a number that is not finite are
//! refused with a `TypeError` or `RangeError` naming where the value is. The same object may appear
//! more than once when it is not its own ancestor; each appearance counts against the size limit,
//! so sharing cannot multiply the work without limit.

use std::rc::Rc;

use crate::LimitKind;
use crate::heap::{Class, ENUMERABLE, Func, Key, OBJECT_BYTES, ObjId, Object, PROP_BYTES, Prop, key, string_bytes};
use crate::interp::{Abrupt, ErrorType, Interp, JsResult};
use crate::value::{Value, index_key};

/// A value that crosses between a script and the host.
#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Data>),
    /// An object's own enumerable properties, in the order the object lists them.
    Object(Vec<(String, Data)>),
}

/// How much one crossing may carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataLimits {
    /// How deeply arrays and objects may nest.
    pub depth: usize,
    /// Estimated bytes of the strings, names and numbers.
    pub bytes: usize,
    /// Array elements and object properties, all of them counted together.
    pub entries: usize,
}

impl Default for DataLimits {
    fn default() -> Self {
        Self { depth: 64, bytes: 16 << 20, entries: 1 << 20 }
    }
}

/// How a host function fails.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    /// Thrown to the script as an `Error`; the script can catch it.
    Error(String),
    /// Thrown to the script as a `TypeError`.
    Type(String),
    /// Thrown to the script as a `RangeError`.
    Range(String),
    /// Thrown to the script as a `SyntaxError`.
    Syntax(String),
    /// Ends the run with [`LimitKind::Host`] and this message. The script cannot catch it.
    Abort(String),
}

/// A function the host gives a script, with its arguments converted.
pub type HostFunction = Rc<dyn Fn(Vec<Data>) -> Result<Data, HostError>>;

/// A member of an object the host adds to the global environment.
#[derive(Clone)]
pub enum HostItem {
    /// A read-only copy of a value.
    Value(Data),
    Function(HostFunction),
    /// A function whose arguments arrive as strings, each the string `String(value)` gives.
    TextFunction(HostFunction),
    /// A property whose value the function gives each time it is read.
    Getter(HostFunction),
    /// A property whose value `get` gives each time it is read, and that `set` is given each value
    /// assigned to it, as its one argument; an error from `set` is thrown to the script.
    Property {
        get: HostFunction,
        set: HostFunction,
    },
    /// A frozen object with these members.
    Object(Vec<(String, HostItem)>),
    /// A function whose host function gives JavaScript source as a string, which then runs as
    /// global code in the realm, as an indirect `eval` runs it, with the run's fuel and limits:
    /// the call gives that code's completion value, and its declarations become globals. A result
    /// that is not a string is the call's result. `end` is called when the source has run,
    /// normally or not. Each source given has the next tag of [`crate::Realm::error_source`],
    /// from 1.
    Evaluator {
        source: HostFunction,
        end: Rc<dyn Fn()>,
    },
    /// A read-only property that reads the interpreter's own state.
    Script(ScriptItem),
}

/// The interpreter's own state a host object can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptItem {
    /// The global object itself, with every global the script has made.
    Global,
    /// The last `Error` object thrown in the current run, caught or not, or `null` before one.
    LastError,
    /// The line of the script's code that reads it, counted from 1.
    Line,
    /// The running frames of the script's code, outermost first, one a line as `name:line`: the
    /// global code is `[global]` and a function without a name `[anonymous]`. At most 64 frames
    /// are listed, the innermost ones, after a line `…` when some are left out.
    Stack,
}

enum Step {
    Name(Key),
    Index(u32),
}

/// A value being read: what it has used so far, and where the reader is.
struct Reading {
    limits: DataLimits,
    bytes: usize,
    entries: usize,
    ancestors: Vec<ObjId>,
    path: Vec<Step>,
    function: Rc<str>,
}

impl Reading {
    fn place(&self) -> String {
        let mut s = String::new();
        for step in &self.path {
            match step {
                Step::Index(i) => s.push_str(&format!("[{i}]")),
                Step::Name(k) => {
                    s.push('.');
                    s.push_str(&String::from_utf16_lossy(k));
                }
            }
        }
        s
    }
}

enum Shape {
    Array(u32),
    Plain(Vec<(Key, Prop)>),
}

impl Interp {
    fn data_error(&mut self, r: &Reading, t: ErrorType, what: &str) -> Abrupt {
        let message = format!("{}: argument{} {what}", r.function, r.place());
        self.throw(t, &message)
    }

    fn charge_reading(&mut self, r: &mut Reading, bytes: usize) -> JsResult<()> {
        r.bytes = r.bytes.saturating_add(bytes);
        if r.bytes > r.limits.bytes {
            return Err(self.data_error(r, ErrorType::Range, "is too large"));
        }
        Ok(())
    }

    fn charge_entries(&mut self, r: &mut Reading, n: usize) -> JsResult<()> {
        r.entries = r.entries.saturating_add(n);
        if r.entries > r.limits.entries {
            return Err(self.data_error(r, ErrorType::Range, "has too many entries"));
        }
        Ok(())
    }

    fn read_data(&mut self, v: &Value, r: &mut Reading) -> JsResult<Data> {
        match v {
            Value::Undefined => Ok(Data::Undefined),
            Value::Null => Ok(Data::Null),
            Value::Bool(b) => Ok(Data::Bool(*b)),
            Value::Number(n) if n.is_finite() => Ok(Data::Number(*n)),
            Value::Number(_) => Err(self.data_error(r, ErrorType::Type, "is not a finite number")),
            Value::String(s) => {
                self.charge_reading(r, s.len().saturating_mul(3).saturating_add(2))?;
                Ok(Data::String(s.to_rust_string()))
            }
            Value::Object(id) => self.read_object(*id, r),
        }
    }

    fn read_object(&mut self, id: ObjId, r: &mut Reading) -> JsResult<Data> {
        self.use_fuel()?;
        if r.ancestors.contains(&id) {
            return Err(self.data_error(r, ErrorType::Type, "refers to itself"));
        }
        if r.ancestors.len() >= r.limits.depth {
            return Err(self.data_error(r, ErrorType::Range, "is nested too deep"));
        }
        let shape = match &self.object(id)?.class {
            Class::Array { length, .. } => Shape::Array(*length),
            Class::Ordinary => {
                let o = self.object(id)?;
                if o.props.len() > r.limits.entries {
                    return Err(self.data_error(r, ErrorType::Range, "has too many entries"));
                }
                Shape::Plain(o.props.iter().filter(|(_, p)| p.enumerable()).map(|(k, p)| (k.clone(), p.clone())).collect())
            }
            other => {
                let what = format!("is a {}, not plain data", other.name());
                return Err(self.data_error(r, ErrorType::Type, &what));
            }
        };
        r.ancestors.push(id);
        let out = match shape {
            Shape::Array(length) => self.read_array(id, length, r),
            Shape::Plain(props) => self.read_plain(props, r),
        };
        r.ancestors.pop();
        out
    }

    fn read_array(&mut self, id: ObjId, length: u32, r: &mut Reading) -> JsResult<Data> {
        self.charge_entries(r, length as usize)?;
        let mut items = Vec::with_capacity(length as usize);
        for i in 0..length {
            self.use_fuel()?;
            r.path.push(Step::Index(i));
            let prop = self.object(id)?.props.get(&index_key(i)).cloned();
            let item = match prop {
                Some(Prop::Data { value: Value::Undefined, .. }) | None => Err(self.data_error(r, ErrorType::Type, "is undefined")),
                Some(Prop::Data { value, .. }) => self.read_data(&value, r),
                Some(Prop::Accessor { .. }) => Err(self.data_error(r, ErrorType::Type, "is a getter, not plain data")),
            }?;
            r.path.pop();
            items.push(item);
        }
        Ok(Data::Array(items))
    }

    fn read_plain(&mut self, props: Vec<(Key, Prop)>, r: &mut Reading) -> JsResult<Data> {
        self.charge_entries(r, props.len())?;
        let mut out = Vec::with_capacity(props.len());
        for (k, prop) in props {
            self.use_fuel()?;
            self.charge_reading(r, k.len().saturating_mul(3).saturating_add(8))?;
            let name = String::from_utf16_lossy(&k);
            r.path.push(Step::Name(k));
            let item = match prop {
                // A property holding `undefined` reads as one left out.
                Prop::Data { value: Value::Undefined, .. } => None,
                Prop::Data { value, .. } => Some(self.read_data(&value, r)?),
                Prop::Accessor { .. } => return Err(self.data_error(r, ErrorType::Type, "is a getter, not plain data")),
            };
            r.path.pop();
            if let Some(item) = item {
                out.push((name, item));
            }
        }
        Ok(Data::Object(out))
    }

    /// A host value as a JavaScript value, `frozen` when a script may not change it. A value
    /// deeper or longer than the limits throws a `RangeError`.
    pub(crate) fn write_data(&mut self, d: Data, depth: usize, frozen: bool) -> JsResult<Value> {
        if depth > self.data_limits.depth {
            return Err(self.throw(ErrorType::Range, "a host value is nested too deep"));
        }
        Ok(match d {
            Data::Undefined => Value::Undefined,
            Data::Null => Value::Null,
            Data::Bool(b) => Value::Bool(b),
            Data::Number(n) => Value::Number(n),
            Data::String(s) => {
                let bytes = string_bytes(s.encode_utf16().count());
                self.reserve_heap(bytes)?;
                self.heap.bytes = self.heap.bytes.saturating_add(bytes);
                Value::str(&s)
            }
            Data::Array(items) => {
                self.write_container(items.into_iter().enumerate().map(|(i, d)| (Slot::Index(i), d)).collect(), true, depth, frozen)?
            }
            Data::Object(items) => self.write_container(items.into_iter().map(|(k, d)| (Slot::Name(k), d)).collect(), false, depth, frozen)?,
        })
    }

    fn write_container(&mut self, items: Vec<(Slot, Data)>, array: bool, depth: usize, frozen: bool) -> JsResult<Value> {
        if items.len() > self.data_limits.entries {
            return Err(self.throw(ErrorType::Range, "a host value has too many entries"));
        }
        let flags = if frozen { ENUMERABLE } else { crate::heap::OPEN };
        self.reserve_heap(OBJECT_BYTES.saturating_add(PROP_BYTES.saturating_mul(items.len())))?;
        let id = if array {
            let proto = self.intrinsics.array_proto;
            self.heap.alloc_object(Object::new(Class::Array { length: 0, length_writable: !frozen }, Some(proto)))
        } else {
            self.new_object()
        };
        let mark = self.temp_roots.len();
        self.temp_roots.push(Value::Object(id));
        let mut result = Ok(());
        for (slot, d) in items {
            let name = match slot {
                Slot::Index(i) => index_key(u32::try_from(i).unwrap_or(u32::MAX)),
                Slot::Name(n) => key(&n),
            };
            result = self.write_data(d, depth + 1, frozen).and_then(|value| self.define(id, name, Prop::Data { value, flags }));
            if result.is_err() {
                break;
            }
        }
        self.temp_roots.truncate(mark);
        result?;
        if frozen {
            self.object_mut(id)?.extensible = false;
        }
        Ok(Value::Object(id))
    }

    /// Calls host function `f`: reads its arguments (with `text`, each as the string `String(value)`
    /// gives), runs it, and gives its result to the script.
    pub(crate) fn call_data(&mut self, f: &crate::heap::DataFn, name: &Rc<str>, args: &[Value], text: bool) -> JsResult<Value> {
        let data = self.read_args(name, args, text)?;
        match f(data) {
            Ok(d) => self.write_data(d, 0, false),
            Err(e) => Err(self.host_error(e)?),
        }
    }

    /// Calls evaluator `f` ([`HostItem::Evaluator`]): reads its arguments, runs it, and runs the
    /// source it gives as global code, as an indirect `eval` does, giving that code's completion
    /// value. A result that is not a string is given to the script as it is. `end` is called once
    /// the source has run, normally or not.
    pub(crate) fn call_evaluator(&mut self, f: &crate::heap::DataFn, end: &Rc<dyn Fn()>, name: &Rc<str>, args: &[Value]) -> JsResult<Value> {
        let data = self.read_args(name, args, false)?;
        let source = match f(data) {
            Ok(Data::String(source)) => source,
            Ok(d) => return self.write_data(d, 0, false),
            Err(e) => return Err(self.host_error(e)?),
        };
        self.evaluated = self.evaluated.saturating_add(1);
        let tag = std::mem::replace(&mut self.source_tag, self.evaluated);
        let code = self.encode_source(&source).and_then(|units| self.compile_source(&units, false, crate::bytecode::CodeKind::Eval));
        self.source_tag = tag;
        let r = match code {
            Ok(code) => self.run_global(code, true),
            Err(e) => Err(self.compile_error_to_abrupt(e)),
        };
        end();
        r
    }

    /// The arguments of host function `name` as [`Data`] (with `text`, each as the string
    /// `String(value)` gives).
    fn read_args(&mut self, name: &Rc<str>, args: &[Value], text: bool) -> JsResult<Vec<Data>> {
        let mut reading = Reading { limits: self.data_limits, bytes: 0, entries: 0, ancestors: Vec::new(), path: Vec::new(), function: name.clone() };
        let mut data = Vec::with_capacity(args.len());
        // A conversion can run the script's own `toString`, which may collect garbage.
        let mark = self.temp_roots.len();
        if text {
            self.temp_roots.extend(args.iter().cloned());
        }
        let mut read = Ok(());
        for (i, a) in args.iter().enumerate() {
            reading.path.push(Step::Index(u32::try_from(i).unwrap_or(u32::MAX)));
            let d = if text { self.read_text(a, &mut reading) } else { self.read_data(a, &mut reading) };
            match d {
                Ok(d) => data.push(d),
                Err(e) => {
                    read = Err(e);
                    break;
                }
            }
            reading.path.pop();
        }
        self.temp_roots.truncate(mark);
        read?;
        Ok(data)
    }

    /// What host error `e` throws, or the limit it ends the run with.
    fn host_error(&mut self, e: HostError) -> JsResult<Abrupt> {
        match e {
            HostError::Abort(message) => {
                self.host_abort = Some(message);
                Ok(Abrupt::Limit(LimitKind::Host))
            }
            HostError::Error(message) => self.throw_host(ErrorType::Error, &message),
            HostError::Type(message) => self.throw_host(ErrorType::Type, &message),
            HostError::Range(message) => self.throw_host(ErrorType::Range, &message),
            HostError::Syntax(message) => self.throw_host(ErrorType::Syntax, &message),
        }
    }

    /// `v` as the string `String(v)` gives, within the limits of a crossing.
    fn read_text(&mut self, v: &Value, r: &mut Reading) -> JsResult<Data> {
        let s = self.to_str(v)?;
        self.charge_reading(r, s.len().saturating_mul(3).saturating_add(2))?;
        Ok(Data::String(s.to_rust_string()))
    }

    /// The exception for a host error, charged to the heap.
    fn throw_host(&mut self, t: ErrorType, message: &str) -> JsResult<Abrupt> {
        let bytes = string_bytes(message.encode_utf16().count());
        self.reserve_heap(bytes)?;
        self.heap.bytes = self.heap.bytes.saturating_add(bytes);
        Ok(self.throw(t, message))
    }

    /// Adds the frozen object `name` with `items` to the global environment.
    pub(crate) fn install_host_object(&mut self, name: &str, items: Vec<(String, HostItem)>) -> JsResult<()> {
        let o = self.host_object(name, items)?;
        let global = self.global;
        self.define(global, key(name), Prop::Data { value: Value::Object(o), flags: 0 })
    }

    fn host_object(&mut self, path: &str, items: Vec<(String, HostItem)>) -> JsResult<ObjId> {
        let o = self.new_object();
        let mark = self.temp_roots.len();
        self.temp_roots.push(Value::Object(o));
        let mut result = Ok(());
        for (name, item) in items {
            result = self.install_item(o, path, name, item);
            if result.is_err() {
                break;
            }
        }
        self.temp_roots.truncate(mark);
        result?;
        self.object_mut(o)?.extensible = false;
        Ok(o)
    }

    fn install_item(&mut self, o: ObjId, path: &str, name: String, item: HostItem) -> JsResult<()> {
        let full = format!("{path}.{name}");
        let prop = match item {
            HostItem::Value(d) => Prop::Data { value: self.write_data(d, 0, true)?, flags: ENUMERABLE },
            HostItem::Function(f) => Prop::Data { value: Value::Object(self.host_function(f, &full, false)), flags: ENUMERABLE },
            HostItem::TextFunction(f) => Prop::Data { value: Value::Object(self.host_function(f, &full, true)), flags: ENUMERABLE },
            HostItem::Getter(f) => Prop::Accessor { get: Some(self.host_function(f, &full, false)), set: None, flags: ENUMERABLE },
            HostItem::Property { get, set } => Prop::Accessor {
                get: Some(self.host_function(get, &full, false)),
                set: Some(self.host_function(set, &full, false)),
                flags: ENUMERABLE,
            },
            HostItem::Object(items) => Prop::Data { value: Value::Object(self.host_object(&full, items)?), flags: ENUMERABLE },
            HostItem::Evaluator { source, end } => {
                let proto = self.intrinsics.function_proto;
                let f = self.heap.alloc_object(Object::new(Class::Function(Func::Eval { f: source, end, name: full.into() }), Some(proto)));
                Prop::Data { value: Value::Object(f), flags: ENUMERABLE }
            }
            HostItem::Script(item) => match crate::builtins::extensions::getter(item) {
                None => Prop::Data { value: Value::Object(self.global), flags: ENUMERABLE },
                Some((name, f)) => {
                    let get = crate::builtins::native(self, name, f, 0, crate::heap::NativeKind::Plain, false)?;
                    Prop::Accessor { get: Some(get), set: None, flags: ENUMERABLE }
                }
            },
        };
        self.define(o, key(&name), prop)
    }

    fn host_function(&mut self, f: HostFunction, name: &str, text: bool) -> ObjId {
        let proto = self.intrinsics.function_proto;
        self.heap.alloc_object(Object::new(Class::Function(Func::Data { f, name: name.into(), text }), Some(proto)))
    }
}

enum Slot {
    Index(usize),
    Name(String),
}
