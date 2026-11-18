//! The script extensions a host adds with [`crate::Realm::add_script_extensions`]: the global
//! `localize`, the global `toSource` and `Object.prototype.toSource`, and the getters behind
//! [`crate::ScriptItem`]. `localize` and `Object.prototype.toString` read the locale of
//! [`Localization`].
//!
//! `toSource` writes primitives, arrays and plain objects as the text of a JavaScript expression
//! that makes the same value: strings in double quotes, property names quoted, properties in the
//! order the object lists them, a hole in an array as an empty element. It never runs a getter or
//! a script's own method: a getter, a function, a date, a regular expression, an error, a boxed
//! primitive, an array with properties besides its elements, an object whose prototype is not
//! `Object.prototype`, a property that is not enumerable or any other object that is not plain
//! data throws a `TypeError`, and so does an object that contains itself. A literal makes an
//! extensible object whose properties are writable and configurable and an array whose `length`
//! is writable, so a property that is not writable or not configurable, an object or array that is
//! not extensible (after `Object.preventExtensions`, `Object.seal` or `Object.freeze`) and an
//! array with a read-only `length` throw a `TypeError` too. A value nested deeper, with more entries or longer text than the
//! realm's [`crate::DataLimits`] allow throws a `RangeError`. Every value written takes fuel.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::{Builder, arg, method};
use crate::heap::{CONFIGURABLE, Class, NativeFn, ObjId, Object, Prop, WRITABLE, key};
use crate::interp::{ErrorType, Interp, JsResult};
use crate::value::{Value, array_index, index_key, number_to_str};
use crate::{ScriptItem, bytecode::Code};

/// The locale `localize` looks a localization object up in, and whether `toString` gives an
/// object's localized text (`$.localize`). A host changes it through [`crate::Realm::localization`].
#[derive(Debug)]
pub struct Localization {
    locale: RefCell<String>,
    platform: Cell<Option<&'static str>>,
    enabled: Cell<bool>,
}

impl Default for Localization {
    fn default() -> Self {
        Self { locale: RefCell::new("en".into()), platform: Cell::new(None), enabled: Cell::new(false) }
    }
}

impl Localization {
    /// The locale, a language and an optional region joined by `_`, such as `de_DE`.
    pub fn locale(&self) -> String {
        self.locale.borrow().clone()
    }

    pub fn set_locale(&self, locale: &str) {
        *self.locale.borrow_mut() = locale.to_string();
    }

    /// The platform suffix the lookup tries first (`Win`, `Mac` or `Unix`), or none.
    pub fn platform(&self) -> Option<&'static str> {
        self.platform.get()
    }

    pub fn set_platform(&self, platform: Option<&'static str>) {
        self.platform.set(platform);
    }

    /// Does `Object.prototype.toString` give an object's localized text?
    pub fn enabled(&self) -> bool {
        self.enabled.get()
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.set(on);
    }

    /// The property names a localization object is looked up by, in order: the locale with the
    /// platform suffix, the locale, its language with the suffix, its language, and `en`.
    pub fn keys(&self) -> Vec<String> {
        let locale = self.locale();
        let language = locale.split('_').next().unwrap_or_default().to_string();
        let mut keys = Vec::with_capacity(5);
        for name in [&locale, &language] {
            if let Some(p) = self.platform() {
                keys.push(format!("{name}_{p}"));
            }
            keys.push(name.clone());
        }
        keys.push("en".into());
        let mut unique = Vec::with_capacity(keys.len());
        for k in keys {
            if !k.is_empty() && !unique.contains(&k) {
                unique.push(k);
            }
        }
        unique
    }
}

pub(crate) fn install(interp: &mut Interp) -> JsResult<()> {
    let global = interp.global;
    method(interp, global, "localize", localize, 1)?;
    method(interp, global, "toSource", to_source, 1)?;
    let object_proto = interp.intrinsics.object_proto;
    method(interp, object_proto, "toSource", to_source_method, 0)
}

/// The text of the localization object `id` for the current locale: the string value of the first
/// of [`Localization::keys`] that the object has as its own data property. No getter runs.
pub(crate) fn localized_text(interp: &mut Interp, id: ObjId) -> JsResult<Option<Value>> {
    if !matches!(interp.object(id)?.class, Class::Ordinary) {
        return Ok(None);
    }
    for k in interp.localization.keys() {
        interp.use_fuel()?;
        if let Some(Prop::Data { value: v @ Value::String(_), .. }) = interp.get_own_property(id, &key(&k))? {
            return Ok(Some(v));
        }
    }
    Ok(None)
}

/// `localize(object, ...values)`: the text of a localization object for the current locale, or of
/// a string, with `%1` replaced by the first value, `%2` by the second and so on. A number with no
/// value stays as it is. An object with none of the locale's names is returned as it is.
fn localize(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let mark = interp.temp_roots.len();
    interp.temp_roots.extend(args.iter().cloned());
    let r = localize_rooted(interp, args);
    interp.temp_roots.truncate(mark);
    r
}

fn localize_rooted(interp: &mut Interp, args: &[Value]) -> JsResult<Value> {
    let text = match args.first() {
        Some(Value::String(s)) => s.flat(),
        Some(first @ Value::Object(id)) => match localized_text(interp, *id)? {
            Some(Value::String(s)) => s.flat(),
            _ => return Ok(first.clone()),
        },
        _ => return Err(interp.type_error("localize: the first argument must be a localization object or a string")),
    };
    let values = args.get(1..).unwrap_or_default();
    let mut converted: Vec<Option<Rc<[u16]>>> = vec![None; values.len()];
    let mut out = Builder::new();
    let (mut start, mut i) = (0, 0);
    while let Some(&u) = text.get(i) {
        if u != u16::from(b'%') {
            i += 1;
            continue;
        }
        let digits = text.get(i + 1..).unwrap_or_default().iter().take(4).take_while(|d| (0x30..=0x39).contains(*d)).count();
        let n = text.get(i + 1..i + 1 + digits).unwrap_or_default().iter().fold(0usize, |n, d| n * 10 + usize::from(*d - 0x30));
        let Some(slot) = n.checked_sub(1).filter(|&k| digits > 0 && k < values.len()) else {
            i += 1 + digits;
            continue;
        };
        out.push_units(interp, text.get(start..i).unwrap_or_default())?;
        let units = match converted.get(slot).cloned().flatten() {
            Some(u) => u,
            None => {
                let s = interp.to_str(&arg(values, slot))?.flat();
                if let Some(c) = converted.get_mut(slot) {
                    *c = Some(s.clone());
                }
                s
            }
        };
        out.push_units(interp, &units)?;
        i += 1 + digits;
        start = i;
    }
    out.push_units(interp, text.get(start..).unwrap_or_default())?;
    Ok(out.finish(interp))
}

/// The global `toSource(value)`.
fn to_source(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    write_source(interp, arg(args, 0))
}

/// `Object.prototype.toSource()`: the source of the value it is called on.
fn to_source_method(interp: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    write_source(interp, this)
}

fn write_source(interp: &mut Interp, v: Value) -> JsResult<Value> {
    let mark = interp.temp_roots.len();
    interp.temp_roots.push(v.clone());
    let mut w = Source { out: Builder::new(), ancestors: Vec::new(), entries: 0 };
    let r = w.value(interp, &v);
    interp.temp_roots.truncate(mark);
    r?;
    Ok(w.out.finish(interp))
}

/// A value being written as source: the text so far, the objects it is inside and the entries it
/// has written.
struct Source {
    out: Builder,
    ancestors: Vec<ObjId>,
    entries: usize,
}

impl Source {
    fn push(&mut self, interp: &mut Interp, s: &str) -> JsResult<()> {
        self.out.push_str(interp, s)?;
        self.check_length(interp)
    }

    fn check_length(&mut self, interp: &mut Interp) -> JsResult<()> {
        self.fit(interp, 0)
    }

    /// Checks that `more` code units can be added without passing the length limit.
    fn fit(&mut self, interp: &mut Interp, more: usize) -> JsResult<()> {
        if self.out.units.len().saturating_add(more) > interp.data_limits.bytes {
            return Err(interp.throw(ErrorType::Range, "toSource: the source is too long"));
        }
        Ok(())
    }

    fn count(&mut self, interp: &mut Interp, n: usize) -> JsResult<()> {
        self.entries = self.entries.saturating_add(n);
        if self.entries > interp.data_limits.entries {
            return Err(interp.throw(ErrorType::Range, "toSource: the value has too many entries"));
        }
        Ok(())
    }

    fn value(&mut self, interp: &mut Interp, v: &Value) -> JsResult<()> {
        interp.use_fuel()?;
        match v {
            Value::Undefined => self.push(interp, "(void 0)"),
            Value::Null => self.push(interp, "null"),
            Value::Bool(b) => self.push(interp, if *b { "true" } else { "false" }),
            Value::Number(n) if *n == 0.0 && n.is_sign_negative() => self.push(interp, "-0"),
            Value::Number(n) => {
                self.out.push_units(interp, &number_to_str(*n).flat())?;
                self.check_length(interp)
            }
            Value::String(s) => self.quoted(interp, &s.flat()),
            Value::Object(id) => self.object(interp, *id),
        }
    }

    /// `s` in double quotes, with the characters a string literal cannot hold escaped. The length
    /// limit is checked before each addition, so a string that grows as it is escaped throws a
    /// `RangeError` before the text takes more heap than the limit allows.
    fn quoted(&mut self, interp: &mut Interp, s: &[u16]) -> JsResult<()> {
        self.fit(interp, 1)?;
        self.out.push_str(interp, "\"")?;
        let mut start = 0;
        for (i, &c) in s.iter().enumerate() {
            let escape = match c {
                0x22 => "\\\"".to_string(),
                0x5C => "\\\\".to_string(),
                0x0A => "\\n".to_string(),
                0x0D => "\\r".to_string(),
                0x09 => "\\t".to_string(),
                0x08 => "\\b".to_string(),
                0x0C => "\\f".to_string(),
                0..=0x1F | 0x7F | 0x2028 | 0x2029 | 0xD800..=0xDFFF => format!("\\u{c:04X}"),
                _ => continue,
            };
            self.fit(interp, (i - start).saturating_add(escape.len()))?;
            self.out.push_units(interp, s.get(start..i).unwrap_or_default())?;
            self.out.push_str(interp, &escape)?;
            start = i + 1;
        }
        self.fit(interp, s.len().saturating_sub(start).saturating_add(1))?;
        self.out.push_units(interp, s.get(start..).unwrap_or_default())?;
        self.push(interp, "\"")
    }

    fn object(&mut self, interp: &mut Interp, id: ObjId) -> JsResult<()> {
        if self.ancestors.contains(&id) {
            return Err(interp.type_error("toSource: the value contains itself"));
        }
        if self.ancestors.len() >= interp.data_limits.depth {
            return Err(interp.throw(ErrorType::Range, "toSource: the value is nested too deep"));
        }
        let (object_proto, array_proto) = (interp.intrinsics.object_proto, interp.intrinsics.array_proto);
        let o = interp.object(id)?;
        // The text makes an array or an object from `Object.prototype` with enumerable properties
        // only, so an array with other properties, an object with another prototype and a property
        // that is not enumerable are refused.
        let enumerable = o.props.iter().all(|(_, p)| p.enumerable());
        let shape = match &o.class {
            Class::Array { length, .. }
                if o.proto == Some(array_proto) && enumerable && o.props.iter().all(|(k, _)| array_index(k).is_some_and(|i| i < *length)) =>
            {
                Some(Err(*length))
            }
            Class::Array { .. } => return Err(interp.type_error(NOT_PLAIN_ARRAY)),
            Class::Ordinary if o.proto == Some(object_proto) && enumerable => {
                Some(Ok(o.props.iter().map(|(k, p)| (k.clone(), p.clone())).collect::<Vec<_>>()))
            }
            Class::Ordinary => return Err(interp.type_error(NOT_PLAIN_OBJECT)),
            _ => None,
        };
        let fixed = shape.as_ref().and(fixed_state(o));
        let Some(shape) = shape else {
            let what = format!("toSource: a {} cannot be written as source", interp.object(id)?.class.name());
            return Err(interp.type_error(&what));
        };
        if let Some(what) = fixed {
            return Err(interp.type_error(what));
        }
        self.ancestors.push(id);
        let r = match shape {
            Err(length) => self.array(interp, id, length),
            Ok(props) => self.plain(interp, props),
        };
        self.ancestors.pop();
        r
    }

    fn array(&mut self, interp: &mut Interp, id: ObjId, length: u32) -> JsResult<()> {
        self.count(interp, length as usize)?;
        self.push(interp, "[")?;
        let mut hole = false;
        for i in 0..length {
            interp.use_fuel()?;
            if i > 0 {
                self.push(interp, ", ")?;
            }
            let prop = interp.object(id)?.props.get(&index_key(i)).cloned();
            hole = prop.is_none();
            match prop {
                None => {}
                Some(Prop::Data { value, .. }) => self.value(interp, &value)?,
                Some(Prop::Accessor { .. }) => return Err(interp.type_error("toSource: a getter cannot be written as source")),
            }
        }
        // A hole at the end needs a comma of its own, or the array would be one shorter.
        if hole {
            self.push(interp, ",")?;
        }
        self.push(interp, "]")
    }

    fn plain(&mut self, interp: &mut Interp, props: Vec<(crate::heap::Key, Prop)>) -> JsResult<()> {
        self.count(interp, props.len())?;
        self.push(interp, "({")?;
        for (n, (k, prop)) in props.into_iter().enumerate() {
            interp.use_fuel()?;
            if n > 0 {
                self.push(interp, ", ")?;
            }
            let Prop::Data { value, .. } = prop else { return Err(interp.type_error("toSource: a getter cannot be written as source")) };
            self.quoted(interp, &k)?;
            self.push(interp, ":")?;
            self.value(interp, &value)?;
        }
        self.push(interp, "})")
    }
}

const NOT_PLAIN_ARRAY: &str =
    "toSource: an array with properties besides its elements, a property that is not enumerable or another prototype cannot be written as source";
const NOT_PLAIN_OBJECT: &str =
    "toSource: an object with a property that is not enumerable, or whose prototype is not Object.prototype, cannot be written as source";
const NOT_EXTENSIBLE: &str = "toSource: a non-extensible object cannot be written as source";
const READ_ONLY_LENGTH: &str = "toSource: an array with a read-only length cannot be written as source";
const FIXED_PROPERTY: &str = "toSource: a property that is not writable or configurable cannot be written as source";

/// Why the literal for the array or plain object `o` would not make it again: `o` is not
/// extensible, its `length` is read-only, or one of its data properties is not writable or not
/// configurable. A literal makes each of these open.
fn fixed_state(o: &Object) -> Option<&'static str> {
    if !o.extensible {
        return Some(NOT_EXTENSIBLE);
    }
    if matches!(o.class, Class::Array { length_writable: false, .. }) {
        return Some(READ_ONLY_LENGTH);
    }
    let open = WRITABLE | CONFIGURABLE;
    o.props.iter().any(|(_, p)| matches!(p, Prop::Data { flags, .. } if flags & open != open)).then_some(FIXED_PROPERTY)
}

/// The getter behind `item`; [`ScriptItem::Global`] is a value and has none.
pub(crate) fn getter(item: ScriptItem) -> Option<(&'static str, NativeFn)> {
    match item {
        ScriptItem::Global => None,
        ScriptItem::LastError => Some(("error", last_error)),
        ScriptItem::Line => Some(("line", line)),
        ScriptItem::Stack => Some(("stack", stack)),
    }
}

fn last_error(interp: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
    Ok(interp.last_error.clone())
}

fn line(interp: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let line = interp.script_frames().last().map_or(0, |(code, pc, _)| place(interp.locator.as_ref(), code, pc).1);
    Ok(Value::Number(f64::from(line)))
}

/// Where the instruction before `pc` in `code` came from: the file the realm's locator names
/// (`None` without one) and the line there, or the line in the compiled source without a locator.
fn place(locator: Option<&crate::realm::SourceLocator>, code: &Code, pc: usize) -> (Option<String>, u32) {
    let line = code.line_column(pc.saturating_sub(1)).0;
    match locator {
        Some(locate) => {
            let (file, line) = locate(code.source.tag, line);
            (Some(file), line)
        }
        None => (None, line),
    }
}

/// The most frames `$.stack` lists, and the most characters of a function's name it shows.
const STACK_FRAMES: usize = 64;
const STACK_NAME_UNITS: usize = 64;

/// The script's frames, outermost first, one a line: the function's name and the line it is at,
/// as `name:line`, or as `name@file:line` with the file the realm's locator names.
fn stack(interp: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let frames: Vec<(Rc<Code>, usize, bool)> = interp.script_frames().map(|(c, pc, f)| (c.clone(), pc, f)).collect();
    let skip = frames.len().saturating_sub(STACK_FRAMES);
    let locator = interp.locator.clone();
    let mut text = String::new();
    if skip > 0 {
        text.push_str("…\n");
    }
    for (code, pc, function) in frames.iter().skip(skip) {
        interp.use_fuel()?;
        let name = if *function { function_name(code) } else { "[global]".to_string() };
        match place(locator.as_ref(), code, *pc) {
            (Some(file), line) => text.push_str(&format!("{name}@{file}:{line}\n")),
            (None, line) => text.push_str(&format!("{name}:{line}\n")),
        }
    }
    let mut out = Builder::new();
    out.push_str(interp, &text)?;
    Ok(out.finish(interp))
}

/// The name `code` has: its own as a function expression's, or its declaration's, cut to
/// [`STACK_NAME_UNITS`]; `[anonymous]` without one.
fn function_name(code: &Code) -> String {
    match code.self_name.as_ref().or(code.decl_name.as_ref()) {
        Some(name) => String::from_utf16_lossy(name.get(..STACK_NAME_UNITS).unwrap_or(&**name)),
        None => "[anonymous]".to_string(),
    }
}
