//! The `JSON` object (ECMA-262 5.1 section 15.12): `parse` with a reviver and `stringify` with a
//! replacer function or property list and indentation.
//!
//! Both work without recursion. `parse` reads nested arrays and objects with an explicit stack and
//! walks them for the reviver with another, and `stringify` writes nested values from a stack of
//! the objects it is inside, so nesting is limited only by the heap. Each code unit of the text
//! parsed, each property visited and each value written takes a unit of fuel; strings, arrays and
//! objects are checked against the heap limit before they are made, and the text `stringify`
//! writes is checked as it grows.

use std::collections::HashSet;

use super::{Builder, ELEMENT_BYTES, arg, data, key_of, length_of, method, new_array, new_string, to_integer};
use crate::heap::{Class, Key, OBJECT_BYTES, OPEN, ObjId, Object, Prop, key, string_bytes};
use crate::interp::{Desc, ErrorType, Interp, JsResult};
use crate::value::{Str, Value, number_to_str};

pub(super) fn install(interp: &mut Interp) -> JsResult<()> {
    interp.reserve_heap(OBJECT_BYTES)?;
    let proto = interp.intrinsics.object_proto;
    let json = interp.heap.alloc_object(Object::new(Class::Json, Some(proto)));
    method(interp, json, "parse", parse, 2)?;
    method(interp, json, "stringify", stringify, 3)?;
    let global = interp.global;
    data(interp, global, "JSON", Value::Object(json), crate::heap::HIDDEN)
}

// ---- parse ----

/// A container being read, with the property name its next value goes under.
enum Open {
    Array { obj: ObjId, next: u32 },
    Object { obj: ObjId, name: Option<Key> },
}

struct Parser<'a> {
    s: &'a [u16],
    pos: usize,
}

const QUOTE: u16 = b'"' as u16;

impl Parser<'_> {
    fn peek(&self) -> Option<u16> {
        self.s.get(self.pos).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(0x09 | 0x0A | 0x0D | 0x20)) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(u16::from(c)) {
            self.pos += 1;
            return true;
        }
        false
    }

    fn word(&mut self, w: &str) -> bool {
        let units: Vec<u16> = w.encode_utf16().collect();
        if self.s.get(self.pos..).is_some_and(|rest| rest.starts_with(&units)) {
            self.pos += units.len();
            return true;
        }
        false
    }

    /// A JSONString's characters (section 15.12.1.1), after its opening quote.
    fn string(&mut self) -> Result<Vec<u16>, &'static str> {
        let mut out = Vec::new();
        loop {
            let Some(c) = self.peek() else { return Err("a string is not closed") };
            self.pos += 1;
            match c {
                QUOTE => return Ok(out),
                0x5C => {
                    let Some(e) = self.peek() else { return Err("a string is not closed") };
                    self.pos += 1;
                    let unit = match u8::try_from(e).unwrap_or(0) {
                        b'"' => QUOTE,
                        b'\\' => 0x5C,
                        b'/' => 0x2F,
                        b'b' => 0x08,
                        b'f' => 0x0C,
                        b'n' => 0x0A,
                        b'r' => 0x0D,
                        b't' => 0x09,
                        b'u' => {
                            let mut v = 0u32;
                            for _ in 0..4 {
                                let Some(d) = self.peek().and_then(|u| crate::chars::hex_value(u32::from(u))) else {
                                    return Err("a \\u escape needs four hexadecimal digits");
                                };
                                self.pos += 1;
                                v = v * 16 + d;
                            }
                            u16::try_from(v).unwrap_or(0)
                        }
                        _ => return Err("a string holds an escape sequence JSON does not have"),
                    };
                    out.push(unit);
                }
                0..=0x1F => return Err("a string holds a control character"),
                _ => out.push(c),
            }
        }
    }

    /// A JSONNumber (section 15.12.1.1).
    fn number(&mut self) -> Result<f64, &'static str> {
        let start = self.pos;
        self.eat(b'-');
        let digit = |p: &Self| p.peek().is_some_and(|u| (u16::from(b'0')..=u16::from(b'9')).contains(&u));
        if self.eat(b'0') {
        } else if digit(self) {
            while digit(self) {
                self.pos += 1;
            }
        } else {
            return Err("a number has no digits");
        }
        if self.eat(b'.') {
            if !digit(self) {
                return Err("a number's fraction has no digits");
            }
            while digit(self) {
                self.pos += 1;
            }
        }
        if self.eat(b'e') || self.eat(b'E') {
            if !self.eat(b'+') {
                self.eat(b'-');
            }
            if !digit(self) {
                return Err("a number's exponent has no digits");
            }
            while digit(self) {
                self.pos += 1;
            }
        }
        let text = String::from_utf16_lossy(self.s.get(start..self.pos).unwrap_or_default());
        text.parse::<f64>().map_err(|_| "a number cannot be read")
    }
}

/// What the parser read at a value position.
enum Token {
    Value(Value),
    OpenArray,
    OpenObject,
}

fn syntax(interp: &mut Interp, why: &str, at: usize) -> crate::interp::Abrupt {
    interp.throw(ErrorType::Syntax, &format!("JSON.parse: {why} (at offset {at})"))
}

/// `JSON.parse` (section 15.12.2).
fn parse(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let text = interp.to_str(&arg(args, 0))?.flat();
    for _ in 0..text.len() {
        interp.use_fuel()?;
    }
    let mark = interp.temp_roots.len();
    let r = parse_text(interp, &text).and_then(|v| {
        let reviver = arg(args, 1);
        if !interp.is_callable(&reviver) {
            return Ok(v);
        }
        interp.temp_roots.push(v.clone());
        revive(interp, v, &reviver)
    });
    interp.temp_roots.truncate(mark);
    r
}

/// Reads a value at the parser's position: a primitive, or the start of an array or object.
fn token(interp: &mut Interp, p: &mut Parser<'_>) -> JsResult<Token> {
    p.skip_space();
    let at = p.pos;
    let t = match p.peek() {
        Some(QUOTE) => {
            p.pos += 1;
            let units = p.string().map_err(|why| syntax(interp, why, at))?;
            Token::Value(new_string(interp, units)?)
        }
        Some(0x5B) => {
            p.pos += 1;
            Token::OpenArray
        }
        Some(0x7B) => {
            p.pos += 1;
            Token::OpenObject
        }
        _ if p.word("true") => Token::Value(Value::Bool(true)),
        _ if p.word("false") => Token::Value(Value::Bool(false)),
        _ if p.word("null") => Token::Value(Value::Null),
        Some(u) if u == u16::from(b'-') || (u16::from(b'0')..=u16::from(b'9')).contains(&u) => {
            Token::Value(Value::Number(p.number().map_err(|why| syntax(interp, why, at))?))
        }
        Some(_) => return Err(syntax(interp, "unexpected character", at)),
        None => return Err(syntax(interp, "unexpected end of text", at)),
    };
    Ok(t)
}

/// Reads the property name of an object member and its colon, or `None` at the closing brace
/// of an empty object.
fn member_name(interp: &mut Interp, p: &mut Parser<'_>, first: bool) -> JsResult<Option<Key>> {
    p.skip_space();
    if first && p.eat(b'}') {
        return Ok(None);
    }
    let at = p.pos;
    if !p.eat(b'"') {
        return Err(syntax(interp, "a property name must be a string", at));
    }
    let units = p.string().map_err(|why| syntax(interp, why, at))?;
    interp.reserve_heap(string_bytes(units.len()))?;
    p.skip_space();
    if !p.eat(b':') {
        return Err(syntax(interp, "a property name must be followed by ':'", p.pos));
    }
    Ok(Some(units.into()))
}

fn parse_text(interp: &mut Interp, text: &[u16]) -> JsResult<Value> {
    let mut p = Parser { s: text, pos: 0 };
    let mut stack: Vec<Open> = Vec::new();
    // The next value to read: the first of an array or object, or one after a comma.
    let mut value = token(interp, &mut p)?;
    loop {
        // Open a container, or place a finished value into the container it belongs to.
        let mut done = match value {
            Token::OpenArray => {
                let a = new_array(interp, 0)?;
                interp.temp_roots.push(Value::Object(a));
                p.skip_space();
                if p.eat(b']') {
                    Some(Value::Object(a))
                } else {
                    stack.push(Open::Array { obj: a, next: 0 });
                    value = token(interp, &mut p)?;
                    continue;
                }
            }
            Token::OpenObject => {
                interp.reserve_heap(OBJECT_BYTES)?;
                let o = interp.new_object();
                interp.temp_roots.push(Value::Object(o));
                match member_name(interp, &mut p, true)? {
                    None => Some(Value::Object(o)),
                    Some(name) => {
                        stack.push(Open::Object { obj: o, name: Some(name) });
                        value = token(interp, &mut p)?;
                        continue;
                    }
                }
            }
            Token::Value(v) => Some(v),
        };
        // Close every container that the finished value completes.
        while let Some(v) = done.take() {
            let Some(top) = stack.last_mut() else {
                p.skip_space();
                if p.pos < text.len() {
                    return Err(syntax(interp, "unexpected text after the value", p.pos));
                }
                return Ok(v);
            };
            match top {
                Open::Array { obj, next } => {
                    let (obj, i) = (*obj, *next);
                    *next = next.saturating_add(1);
                    interp.reserve_heap(ELEMENT_BYTES)?;
                    interp.define(obj, key_of(f64::from(i)), Prop::Data { value: v, flags: OPEN })?;
                    p.skip_space();
                    if p.eat(b',') {
                        break;
                    }
                    if !p.eat(b']') {
                        return Err(syntax(interp, "expected ',' or ']'", p.pos));
                    }
                    stack.pop();
                    done = Some(Value::Object(obj));
                }
                Open::Object { obj, name } => {
                    let obj = *obj;
                    let Some(k) = name.take() else { return Err(syntax(interp, "a member has no name", p.pos)) };
                    interp.reserve_heap(ELEMENT_BYTES)?;
                    interp.define(obj, k, Prop::Data { value: v, flags: OPEN })?;
                    p.skip_space();
                    if p.eat(b',') {
                        let k = member_name(interp, &mut p, false)?;
                        if let Some(Open::Object { name, .. }) = stack.last_mut() {
                            *name = k;
                        }
                        break;
                    }
                    if !p.eat(b'}') {
                        return Err(syntax(interp, "expected ',' or '}'", p.pos));
                    }
                    stack.pop();
                    done = Some(Value::Object(obj));
                }
            }
        }
        value = token(interp, &mut p)?;
    }
}

/// A holder being walked for the reviver (section 15.12.2, Walk).
struct Walk {
    holder: ObjId,
    name: Key,
    val: Value,
    keys: Vec<Key>,
    next: usize,
}

/// Starts walking `holder[name]`: reads the value and, for an array or object, the names of its
/// elements or own enumerable properties. The value is pushed onto the temporary roots.
fn enter(interp: &mut Interp, holder: ObjId, name: Key) -> JsResult<Walk> {
    let val = interp.get_from(holder, &name, &Value::Object(holder))?;
    interp.temp_roots.push(val.clone());
    let mut keys = Vec::new();
    if let Value::Object(o) = val {
        if interp.heap.object(o).is_some_and(|x| matches!(x.class, Class::Array { .. })) {
            let len = length_of(interp, o)?;
            interp.reserve_heap((len as usize).saturating_mul(crate::heap::PROP_BYTES))?;
            for i in 0..len {
                interp.use_fuel()?;
                keys.push(key_of(f64::from(i)));
            }
        } else {
            for k in interp.own_keys(o)? {
                if interp.get_own_property(o, &k)?.is_some_and(|p| p.enumerable()) {
                    keys.push(k);
                }
            }
        }
    }
    Ok(Walk { holder, name, val, keys, next: 0 })
}

/// Walks the parsed value bottom-up, calling the reviver for each property and replacing or
/// deleting the property with what it returns.
fn revive(interp: &mut Interp, v: Value, reviver: &Value) -> JsResult<Value> {
    interp.reserve_heap(OBJECT_BYTES + ELEMENT_BYTES)?;
    let root = interp.new_object();
    interp.temp_roots.push(Value::Object(root));
    interp.define(root, key(""), Prop::Data { value: v, flags: OPEN })?;
    let mut stack = vec![enter(interp, root, key(""))?];
    let mut ret: Option<Value> = None;
    loop {
        let Some(top) = stack.last_mut() else { return Ok(Value::Undefined) };
        if let (Some(r), Value::Object(o)) = (ret.take(), &top.val) {
            let o = *o;
            let Some(k) = top.next.checked_sub(1).and_then(|i| top.keys.get(i)).cloned() else { return Ok(Value::Undefined) };
            if matches!(r, Value::Undefined) {
                interp.delete(o, &k, false)?;
            } else {
                interp.rooted(std::slice::from_ref(&r), |i| {
                    i.reserve_heap(ELEMENT_BYTES)?;
                    i.define_own_property(o, k, &Desc::data(r.clone(), OPEN), false)
                })?;
            }
        }
        if let (Some(k), Value::Object(o)) = (top.keys.get(top.next).cloned(), &top.val) {
            let o = *o;
            top.next += 1;
            interp.use_fuel()?;
            stack.push(enter(interp, o, k)?);
            continue;
        }
        let Some(done) = stack.pop() else { return Ok(Value::Undefined) };
        let name = Value::String(Str::from_rc(done.name));
        let r = interp.call(reviver, &Value::Object(done.holder), &[name, done.val])?;
        interp.temp_roots.pop();
        if stack.is_empty() {
            return Ok(r);
        }
        ret = Some(r);
    }
}

// ---- stringify ----

/// The settings of one `stringify` call.
struct Settings {
    replacer: Option<Value>,
    properties: Option<Vec<Key>>,
    gap: Vec<u16>,
}

/// A value as `stringify` writes it, after `toJSON`, the replacer and unwrapping.
enum Item {
    Skip,
    Primitive(Value),
    Container(ObjId),
}

/// An object or array being written.
struct Writing {
    obj: ObjId,
    keys: Vec<Key>,
    array: bool,
    next: usize,
    count: usize,
    /// The indentation outside it.
    stepback: usize,
}

struct Writer {
    out: Builder,
    indent: Vec<u16>,
    stack: Vec<Writing>,
    open: HashSet<ObjId>,
}

/// `JSON.stringify` (section 15.12.3).
fn stringify(interp: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let mark = interp.temp_roots.len();
    let r = stringify_rooted(interp, args);
    interp.temp_roots.truncate(mark);
    r
}

fn stringify_rooted(interp: &mut Interp, args: &[Value]) -> JsResult<Value> {
    let settings = settings(interp, &arg(args, 1), &arg(args, 2))?;
    interp.reserve_heap(OBJECT_BYTES + ELEMENT_BYTES)?;
    let wrapper = interp.new_object();
    interp.temp_roots.push(Value::Object(wrapper));
    interp.define(wrapper, key(""), Prop::Data { value: arg(args, 0), flags: OPEN })?;
    let mut w = Writer { out: Builder::new(), indent: Vec::new(), stack: Vec::new(), open: HashSet::new() };
    match resolve(interp, &settings, wrapper, &key(""))? {
        Item::Skip => return Ok(Value::Undefined),
        Item::Primitive(v) => write_primitive(interp, &mut w.out, &v)?,
        Item::Container(o) => {
            interp.temp_roots.push(Value::Object(o));
            open(interp, &settings, &mut w, o)?;
        }
    }
    while !w.stack.is_empty() {
        step(interp, &settings, &mut w)?;
    }
    Ok(w.out.finish())
}

/// Reads the replacer and the space argument (section 15.12.3, steps 4 to 8).
fn settings(interp: &mut Interp, replacer: &Value, space: &Value) -> JsResult<Settings> {
    let mut s = Settings { replacer: None, properties: None, gap: Vec::new() };
    if let Value::Object(r) = replacer {
        if interp.is_callable(replacer) {
            s.replacer = Some(replacer.clone());
        } else if interp.heap.object(*r).is_some_and(|o| matches!(o.class, Class::Array { .. })) {
            s.properties = Some(property_list(interp, *r)?);
        }
    }
    let space = match space {
        Value::Object(o) => match interp.heap.object(*o).map(|x| &x.class) {
            Some(Class::Number) => Value::Number(interp.to_number(space)?),
            Some(Class::String(_)) => Value::String(interp.to_str(space)?),
            _ => space.clone(),
        },
        _ => space.clone(),
    };
    match space {
        Value::Number(_) => {
            let n = to_integer(interp, &space)?.clamp(0.0, 10.0) as usize;
            s.gap = vec![u16::from(b' '); n];
        }
        Value::String(t) => s.gap = t.flat().iter().take(10).copied().collect(),
        _ => {}
    }
    Ok(s)
}

/// The property names a replacer array lists, without repeats (section 15.12.3, step 4.b).
fn property_list(interp: &mut Interp, r: ObjId) -> JsResult<Vec<Key>> {
    let len = length_of(interp, r)?;
    let mut list: Vec<Key> = Vec::new();
    let mut seen = HashSet::new();
    for i in 0..len {
        interp.use_fuel()?;
        let v = interp.get_from(r, &key_of(f64::from(i)), &Value::Object(r))?;
        let item = match &v {
            Value::String(s) => Some(s.flat()),
            Value::Number(n) => Some(number_to_str(*n).flat()),
            Value::Object(o) if interp.heap.object(*o).is_some_and(|x| matches!(x.class, Class::String(_) | Class::Number)) => {
                Some(interp.to_str(&v)?.flat())
            }
            _ => None,
        };
        if let Some(k) = item
            && seen.insert(k.clone())
        {
            interp.reserve_heap(string_bytes(k.len()))?;
            list.push(k);
        }
    }
    Ok(list)
}

/// Steps 1 to 4 and 10 of Str (section 15.12.3): reads `holder[key]`, applies `toJSON` and the
/// replacer, unwraps Number, String and Boolean objects, and leaves out functions and
/// `undefined`.
fn resolve(interp: &mut Interp, settings: &Settings, holder: ObjId, k: &Key) -> JsResult<Item> {
    let slot = interp.temp_roots.len();
    interp.temp_roots.push(Value::Undefined);
    let r = resolve_rooted(interp, settings, holder, k, slot);
    interp.temp_roots.truncate(slot);
    r
}

fn resolve_rooted(interp: &mut Interp, settings: &Settings, holder: ObjId, k: &Key, slot: usize) -> JsResult<Item> {
    let keep = |interp: &mut Interp, v: &Value| {
        if let Some(s) = interp.temp_roots.get_mut(slot) {
            *s = v.clone();
        }
    };
    let mut v = interp.get_from(holder, k, &Value::Object(holder))?;
    keep(interp, &v);
    let name = Value::String(Str::from_rc(k.clone()));
    if let Value::Object(o) = v {
        let to_json = interp.get_from(o, &key("toJSON"), &v)?;
        if interp.is_callable(&to_json) {
            v = interp.call(&to_json, &v, std::slice::from_ref(&name))?;
            keep(interp, &v);
        }
    }
    if let Some(replacer) = &settings.replacer {
        v = interp.call(replacer, &Value::Object(holder), &[name, v])?;
        keep(interp, &v);
    }
    if let Value::Object(o) = &v {
        v = match interp.heap.object(*o).map(|x| &x.class) {
            Some(Class::Number) => Value::Number(interp.to_number(&v)?),
            Some(Class::String(_)) => Value::String(interp.to_str(&v)?),
            Some(Class::Boolean(b)) => Value::Bool(*b),
            _ => v,
        };
    }
    Ok(match v {
        Value::Undefined => Item::Skip,
        Value::Object(_) if interp.is_callable(&v) => Item::Skip,
        Value::Object(o) => Item::Container(o),
        other => Item::Primitive(other),
    })
}

/// Quote (section 15.12.3): the string in double quotes with the characters JSON escapes.
fn write_quoted(interp: &mut Interp, out: &mut Builder, s: &[u16]) -> JsResult<()> {
    out.push_units(interp, &[QUOTE])?;
    let mut start = 0;
    for (i, &c) in s.iter().enumerate() {
        let escape: Option<&str> = match c {
            QUOTE => Some("\\\""),
            0x5C => Some("\\\\"),
            0x08 => Some("\\b"),
            0x0C => Some("\\f"),
            0x0A => Some("\\n"),
            0x0D => Some("\\r"),
            0x09 => Some("\\t"),
            0..=0x1F => None,
            _ => continue,
        };
        out.push_units(interp, s.get(start..i).unwrap_or_default())?;
        match escape {
            Some(e) => out.push_str(interp, e)?,
            None => out.push_str(interp, &format!("\\u{c:04x}"))?,
        }
        start = i + 1;
    }
    out.push_units(interp, s.get(start..).unwrap_or_default())?;
    out.push_units(interp, &[QUOTE])
}

fn write_primitive(interp: &mut Interp, out: &mut Builder, v: &Value) -> JsResult<()> {
    match v {
        Value::Null => out.push_str(interp, "null"),
        Value::Bool(b) => out.push_str(interp, if *b { "true" } else { "false" }),
        Value::Number(n) if n.is_finite() => out.push_units(interp, &number_to_str(*n).flat()),
        Value::Number(_) => out.push_str(interp, "null"),
        Value::String(s) => write_quoted(interp, out, &s.flat()),
        Value::Undefined | Value::Object(_) => Ok(()),
    }
}

/// Starts writing an object or array: a cycle back to one already being written throws a
/// `TypeError`. The caller pushes `o` onto the temporary roots first, and `step` pops it when the
/// object or array is closed.
fn open(interp: &mut Interp, settings: &Settings, w: &mut Writer, o: ObjId) -> JsResult<()> {
    if w.open.contains(&o) {
        return Err(interp.type_error("JSON.stringify cannot write a structure that contains itself"));
    }
    let array = interp.heap.object(o).is_some_and(|x| matches!(x.class, Class::Array { .. }));
    let keys = if array {
        let len = length_of(interp, o)?;
        // Every element is written as at least `null` and a comma, so a length that cannot fit
        // is refused before anything is listed.
        if u64::from(len).saturating_mul(5) > (interp.limits.heap_bytes / 4) as u64 {
            return Err(crate::interp::Abrupt::Limit(crate::LimitKind::Heap));
        }
        interp.reserve_heap((len as usize).saturating_mul(crate::heap::PROP_BYTES))?;
        let mut keys = Vec::with_capacity(len as usize);
        for i in 0..len {
            interp.use_fuel()?;
            keys.push(key_of(f64::from(i)));
        }
        keys
    } else if let Some(list) = &settings.properties {
        list.clone()
    } else {
        let mut keys = Vec::new();
        for k in interp.own_keys(o)? {
            if interp.get_own_property(o, &k)?.is_some_and(|p| p.enumerable()) {
                keys.push(k);
            }
        }
        keys
    };
    w.open.insert(o);
    let stepback = w.indent.len();
    w.indent.extend_from_slice(&settings.gap);
    w.out.push_str(interp, if array { "[" } else { "{" })?;
    w.stack.push(Writing { obj: o, keys, array, next: 0, count: 0, stepback });
    Ok(())
}

/// The separator before a member or element: a comma after the first, and with a gap a line
/// break and the indentation.
fn separator(interp: &mut Interp, w: &mut Writer, first: bool, gap: bool) -> JsResult<()> {
    if !first {
        w.out.push_str(interp, ",")?;
    }
    if gap {
        w.out.push_str(interp, "\n")?;
        w.out.push_units(interp, &w.indent.clone())?;
    }
    Ok(())
}

/// Writes the next member or element of the innermost object or array, or closes it.
fn step(interp: &mut Interp, settings: &Settings, w: &mut Writer) -> JsResult<()> {
    let gap = !settings.gap.is_empty();
    let Some(top) = w.stack.last_mut() else { return Ok(()) };
    let (obj, array, count) = (top.obj, top.array, top.count);
    let Some(k) = top.keys.get(top.next).cloned() else {
        let stepback = top.stepback;
        w.stack.pop();
        w.open.remove(&obj);
        interp.temp_roots.pop();
        w.indent.truncate(stepback);
        if count > 0 && gap {
            w.out.push_str(interp, "\n")?;
            w.out.push_units(interp, &w.indent.clone())?;
        }
        return w.out.push_str(interp, if array { "]" } else { "}" });
    };
    top.next += 1;
    interp.use_fuel()?;
    let item = resolve(interp, settings, obj, &k)?;
    if !array && matches!(item, Item::Skip) {
        return Ok(());
    }
    if let Some(top) = w.stack.last_mut() {
        top.count += 1;
    }
    // The separator and the key reserve heap, so a container is rooted before they are written.
    if let Item::Container(o) = item {
        interp.temp_roots.push(Value::Object(o));
    }
    separator(interp, w, count == 0, gap)?;
    if !array {
        write_quoted(interp, &mut w.out, &k)?;
        w.out.push_str(interp, if gap { ": " } else { ":" })?;
    }
    match item {
        Item::Skip => w.out.push_str(interp, "null"),
        Item::Primitive(v) => write_primitive(interp, &mut w.out, &v),
        Item::Container(o) => open(interp, settings, w, o),
    }
}
