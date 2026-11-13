//! `File` and `Folder`, the classes the help pages
//! <https://extendscript.docsforadobe.dev/file-system-access/file-object/> and
//! <https://extendscript.docsforadobe.dev/file-system-access/folder-object/> describe, as built-ins
//! whose file system work a host does ([`crate::Realm::define_file_classes`]).
//!
//! The constructors build their objects with and without `new`: `new File(path)` is always a
//! `File` and `new Folder(path)` always a `Folder`, while `File(path)` gives a `Folder` when the
//! host's `stat` says the path is a folder, and `Folder(path)` a `File` when it is a file. Two
//! objects for one path are different objects. An object keeps its path in the host's platform
//! form, its `error` text, the id of the file it has open (or -1), the encoding set before `open`
//! and its line feed, in properties that are not enumerable; every name the getters give comes from
//! the host's `parse` of that path. `Folder.current` is kept in the realm and starts at the host's
//! `current`.
//!
//! `write` and `writeln` join their values as `String(value)` gives them, and `writeln` adds the
//! line end `lineFeed` names: CRLF for `Windows`, CR for `Macintosh` and LF otherwise. The file
//! dialogs are built-ins that ask the host's `pick` and may pause the run until the person answers
//! ([`crate::interp::Pause`]); they give a `File`, a `Folder`, an array of `File`s or `null`.
//!
//! A method whose host call fails sets `error` and returns `false`, `null` or an empty string. A
//! count or position that is not a finite number, a negative count and a seek mode other than 0, 1
//! or 2 throw a `RangeError`; a method called on an object that is not a `File` or `Folder` throws
//! a `TypeError`. `getFiles` with a function calls it with each entry and keeps those it returns a
//! true value for; every entry takes fuel.

use crate::HostItem;
use crate::builtins::{arg, constructor, data, method, native, pausable_method, put_at};
use crate::heap::{CONFIGURABLE, Class, HIDDEN, NativeFn, NativeKind, OBJECT_BYTES, ObjId, Object, PROP_BYTES, Prop, key, string_bytes};
use crate::interp::{Abrupt, ErrorType, Interp, JsResult, Pause};
use crate::value::{Value, index_key};

/// The objects of `File` and `Folder`, kept by the realm, and `Folder.current`.
#[derive(Clone, Debug)]
pub(crate) struct Classes {
    file_proto: ObjId,
    folder_proto: ObjId,
    host: ObjId,
    current: Value,
    /// The count of [`reset_current`] calls: a file is open only in the run that opened it.
    run: f64,
}

impl Classes {
    /// The objects a collection keeps.
    pub(crate) fn objects(&self) -> [ObjId; 3] {
        [self.file_proto, self.folder_proto, self.host]
    }
}

const PATH: &str = "__vcPath";
const ERROR: &str = "__vcError";
const HANDLE: &str = "__vcHandle";
const RUN: &str = "__vcRun";
const ENCODING: &str = "__vcEncoding";
const LINE_FEED: &str = "__vcLineFeed";

/// The locations `Folder` has as properties, each read from the host's `locations` when it is read.
const PLACES: [&str; 10] = ["appData", "appPackage", "commonFiles", "desktop", "myDocuments", "startup", "system", "temp", "trash", "userData"];

fn classes(i: &Interp) -> JsResult<Classes> {
    i.files.clone().ok_or_else(|| Abrupt::Internal("File and Folder are not installed".into()))
}

/// Calls the host function `name` with `args`.
fn host(i: &mut Interp, name: &str, args: &[Value]) -> JsResult<Value> {
    let c = classes(i)?;
    let f = i.get_value(&Value::Object(c.host), &key(name))?;
    i.call(&f, &Value::Undefined, args)
}

/// The host's message `name`.
fn message(i: &mut Interp, name: &str) -> JsResult<Value> {
    let c = classes(i)?;
    let text = i.get_value(&Value::Object(c.host), &key("text"))?;
    i.get_value(&text, &key(name))
}

/// The field `name` of a record the host gave: its own data property, `undefined` when it has none,
/// so that a property a script put on `Object.prototype` is not read as part of it.
fn field(i: &mut Interp, v: &Value, name: &str) -> JsResult<Value> {
    match v.as_object() {
        Some(o) => slot(i, o, name),
        None => Ok(Value::Undefined),
    }
}

/// The value of the own data property `name` of `o`, `undefined` when it has none.
fn slot(i: &Interp, o: ObjId, name: &str) -> JsResult<Value> {
    Ok(match i.get_own_property(o, &key(name))? {
        Some(Prop::Data { value, .. }) => value,
        _ => Value::Undefined,
    })
}

fn set_slot(i: &mut Interp, o: ObjId, name: &str, value: Value) -> JsResult<()> {
    if let Value::String(s) = &value {
        i.rooted(std::slice::from_ref(&value), |i| i.reserve_heap(PROP_BYTES + string_bytes(s.len())))?;
    }
    i.define(o, key(name), Prop::Data { value, flags: HIDDEN })
}

/// The object `this` and its path, or a `TypeError` naming `what` when it is not a `File` or
/// `Folder`.
fn this_path(i: &mut Interp, this: &Value, what: &str) -> JsResult<(ObjId, Value)> {
    if let Some(o) = this.as_object()
        && let path @ Value::String(_) = slot(i, o, PATH)?
    {
        return Ok((o, path));
    }
    Err(i.type_error(&format!("{what}: the object is not a File or a Folder")))
}

/// The path `x` names, and whether it is already in the platform's form: the path of a `File` or
/// `Folder`, or `String(x)`, which may hold URI escapes.
fn path_text(i: &mut Interp, x: &Value) -> JsResult<(Value, bool)> {
    if let Some(o) = x.as_object()
        && let path @ Value::String(_) = slot(i, o, PATH)?
    {
        return Ok((path, true));
    }
    let s = i.to_str(x)?;
    Ok((Value::String(s), false))
}

/// The host's `parse` of the path `x` names, from `Folder.current`.
fn parse(i: &mut Interp, x: &Value) -> JsResult<Value> {
    let (text, platform) = path_text(i, x)?;
    parse_text(i, text, platform)
}

/// The host's `parse` of `text`, from `Folder.current`; its URI escapes are decoded unless it is
/// already a `platform` path.
fn parse_text(i: &mut Interp, text: Value, platform: bool) -> JsResult<Value> {
    let current = classes(i)?.current;
    host(i, "parse", &[text, current, Value::Bool(platform)])
}

fn parsed(i: &mut Interp, this: &Value, what: &str, name: &str) -> JsResult<Value> {
    let (_, path) = this_path(i, this, what)?;
    let p = parse_text(i, path, true)?;
    i.rooted(std::slice::from_ref(&p), |i| field(i, &p, name))
}

/// A new object with prototype `proto` for the path `fs`, already in the platform's form.
fn instance(i: &mut Interp, proto: ObjId, fs: Value) -> JsResult<Value> {
    let line_feed = host_value(i, "lineFeed")?;
    i.rooted(&[fs.clone(), line_feed.clone()], |i| {
        i.reserve_heap(OBJECT_BYTES + 6 * PROP_BYTES)?;
        let o = i.heap.alloc_object(Object::new(Class::Ordinary, Some(proto)));
        i.rooted(&[Value::Object(o)], |i| {
            set_slot(i, o, PATH, fs.clone())?;
            set_slot(i, o, ERROR, Value::str(""))?;
            set_slot(i, o, HANDLE, Value::Number(-1.0))?;
            set_slot(i, o, RUN, Value::Number(-1.0))?;
            set_slot(i, o, ENCODING, Value::str(""))?;
            set_slot(i, o, LINE_FEED, line_feed.clone())?;
            Ok(Value::Object(o))
        })
    })
}

fn host_value(i: &mut Interp, name: &str) -> JsResult<Value> {
    let c = classes(i)?;
    i.get_value(&Value::Object(c.host), &key(name))
}

/// The host's `stat` of the object's path, or `None` after setting `error` when it failed.
fn stat(i: &mut Interp, this: &Value, what: &str) -> JsResult<Option<Value>> {
    let (o, path) = this_path(i, this, what)?;
    let r = host(i, "stat", &[path])?;
    i.rooted(std::slice::from_ref(&r), |i| {
        let error = field(i, &r, "error")?;
        if matches!(error, Value::String(_)) {
            set_slot(i, o, ERROR, error)?;
            return Ok(None);
        }
        Ok(Some(r.clone()))
    })
}

/// The field `name` of the object's `stat`, when it exists, or `missing`.
fn stat_field(i: &mut Interp, this: &Value, what: &str, name: &str, missing: Value) -> JsResult<Value> {
    let Some(r) = stat(i, this, what)? else { return Ok(missing) };
    i.rooted(std::slice::from_ref(&r), |i| {
        if !field(i, &r, "exists")?.truthy() && name != "exists" {
            return Ok(missing);
        }
        field(i, &r, name)
    })
}

/// The result of a host read: its `text`, or `failed` after setting `error`.
fn text_result(i: &mut Interp, o: ObjId, r: Value, failed: Value) -> JsResult<Value> {
    i.rooted(std::slice::from_ref(&r), |i| {
        let error = field(i, &r, "error")?;
        if matches!(error, Value::String(_)) {
            set_slot(i, o, ERROR, error)?;
            return Ok(failed);
        }
        field(i, &r, "text")
    })
}

/// The id of the object's open file, or `None` after setting `error` when it has none.
fn opened(i: &mut Interp, o: ObjId) -> JsResult<Option<Value>> {
    if let Some(h) = is_open(i, o)? {
        return Ok(Some(h));
    }
    let m = message(i, "notOpen")?;
    set_slot(i, o, ERROR, m)?;
    Ok(None)
}

/// The id of the object's open file, when this run opened it.
fn is_open(i: &Interp, o: ObjId) -> JsResult<Option<Value>> {
    let h = slot(i, o, HANDLE)?;
    let this_run = matches!(slot(i, o, RUN)?, Value::Number(n) if n == classes(i)?.run);
    Ok((this_run && matches!(h, Value::Number(n) if n >= 0.0)).then_some(h))
}

/// Closes the object's open file, if it has one, which writes what the script wrote to it. → false
/// after setting `error` when that could not be written; the file then stays open, so a later
/// `close` tries again.
fn close_open(i: &mut Interp, o: ObjId) -> JsResult<bool> {
    let mut written = true;
    if let Some(h) = is_open(i, o)? {
        let r = host(i, "close", &[h])?;
        written = outcome(i, o, &r)?;
    }
    if written {
        set_slot(i, o, HANDLE, Value::Number(-1.0))?;
    }
    Ok(written)
}

fn date(i: &mut Interp, ms: Value) -> JsResult<Value> {
    let Value::Number(t) = ms else { return Ok(Value::Null) };
    i.reserve_heap(OBJECT_BYTES)?;
    let proto = i.intrinsics.date_proto;
    Ok(Value::Object(i.heap.alloc_object(Object::new(Class::Date(t), Some(proto)))))
}

/// A count of characters: `None` for `undefined`, else a whole number 0 or more.
fn count(i: &mut Interp, v: &Value, what: &str) -> JsResult<Option<f64>> {
    if matches!(v, Value::Undefined) {
        return Ok(None);
    }
    let n = i.to_number(v)?;
    if !n.is_finite() || n < 0.0 {
        return Err(i.throw(ErrorType::Range, &format!("{what}: the count must be a finite number, 0 or more")));
    }
    Ok(Some(n.floor()))
}

// ---- constructors ----

fn file_ctor(i: &mut Interp, _: Value, args: &[Value], construct: bool) -> JsResult<Value> {
    make(i, args, construct, true)
}

fn folder_ctor(i: &mut Interp, _: Value, args: &[Value], construct: bool) -> JsResult<Value> {
    make(i, args, construct, false)
}

/// `new File(path)`, `File(path)`, `new Folder(path)` or `Folder(path)`.
fn make(i: &mut Interp, args: &[Value], construct: bool, file: bool) -> JsResult<Value> {
    let c = classes(i)?;
    let (own, other) = if file { (c.file_proto, c.folder_proto) } else { (c.folder_proto, c.file_proto) };
    let (path, platform) = match arg(args, 0) {
        Value::Undefined if file => (host(i, "tempName", &[])?, true),
        Value::Undefined => (c.current.clone(), true),
        v => (v, false),
    };
    let fs = i.rooted(std::slice::from_ref(&path), |i| {
        let p = if platform { parse_text(i, path.clone(), true)? } else { parse(i, &path)? };
        i.rooted(std::slice::from_ref(&p), |i| field(i, &p, "fs"))
    })?;
    let proto = if construct {
        own
    } else {
        let st = i.rooted(std::slice::from_ref(&fs), |i| host(i, "stat", std::slice::from_ref(&fs)))?;
        let flip = i.rooted(&[fs.clone(), st.clone()], |i| {
            let error = field(i, &st, "error")?;
            let exists = field(i, &st, "exists")?.truthy();
            let dir = field(i, &st, "dir")?.truthy();
            Ok(matches!(error, Value::Undefined) && exists && dir == file)
        })?;
        if flip { other } else { own }
    };
    instance(i, proto, fs)
}

// ---- both classes ----

fn absolute_uri(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    parsed(i, &this, "absoluteURI", "uri")
}

fn fs_name(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    Ok(this_path(i, &this, "fsName")?.1)
}

fn relative_uri(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (_, path) = this_path(i, &this, "relativeURI")?;
    let current = classes(i)?.current;
    host(i, "relative", &[path, current])
}

fn name(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    parsed(i, &this, "name", "name")
}

fn display_name(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    parsed(i, &this, "displayName", "display")
}

fn path(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    parsed(i, &this, "path", "path")
}

fn parent(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let up = parsed(i, &this, "parent", "parent")?;
    if matches!(up, Value::Null | Value::Undefined) {
        return Ok(Value::Null);
    }
    let proto = classes(i)?.folder_proto;
    instance(i, proto, up)
}

/// `exists`: whether the path names something of the object's own kind, a folder for a `Folder`
/// and anything else for a `File`.
fn exists(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let folder = classes(i)?.folder_proto;
    let want_dir = match this.as_object() {
        Some(o) => i.object(o)?.proto == Some(folder),
        None => false,
    };
    let Some(r) = stat(i, &this, "exists")? else { return Ok(Value::Bool(false)) };
    i.rooted(std::slice::from_ref(&r), |i| {
        let dir = field(i, &r, "dir")?.truthy();
        Ok(Value::Bool(field(i, &r, "exists")?.truthy() && dir == want_dir))
    })
}

fn created(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let ms = stat_field(i, &this, "created", "created", Value::Null)?;
    date(i, ms)
}

fn modified(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let ms = stat_field(i, &this, "modified", "modified", Value::Null)?;
    date(i, ms)
}

fn alias(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let Some(r) = stat(i, &this, "alias")? else { return Ok(Value::Bool(false)) };
    let v = i.rooted(std::slice::from_ref(&r), |i| field(i, &r, "alias"))?;
    Ok(Value::Bool(v.truthy()))
}

fn get_error(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "error")?;
    slot(i, o, ERROR)
}

fn set_error(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "error")?;
    let v = match arg(args, 0) {
        Value::Undefined | Value::Null => Value::str(""),
        v => Value::String(i.to_str(&v)?),
    };
    set_slot(i, o, ERROR, v)?;
    Ok(Value::Undefined)
}

fn get_relative_uri(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (_, path) = this_path(i, &this, "getRelativeURI")?;
    let base = match arg(args, 0) {
        Value::Undefined => classes(i)?.current,
        b => {
            let p = i.rooted(std::slice::from_ref(&path), |i| parse(i, &b))?;
            i.rooted(std::slice::from_ref(&p), |i| field(i, &p, "fs"))?
        }
    };
    host(i, "relative", &[path, base])
}

fn change_path(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "changePath")?;
    // An object whose changes could not be written keeps its path; `error` says why.
    if !close_open(i, o)? {
        return Ok(Value::Bool(false));
    }
    let p = parse(i, &arg(args, 0))?;
    let fs = i.rooted(std::slice::from_ref(&p), |i| field(i, &p, "fs"))?;
    set_slot(i, o, PATH, fs)?;
    Ok(Value::Bool(true))
}

fn resolve(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, path) = this_path(i, &this, "resolve")?;
    let r = host(i, "resolve", &[path])?;
    let (error, fs, dir) = i.rooted(std::slice::from_ref(&r), |i| Ok((field(i, &r, "error")?, field(i, &r, "fs")?, field(i, &r, "dir")?)))?;
    if matches!(error, Value::String(_)) {
        set_slot(i, o, ERROR, error)?;
        return Ok(Value::Null);
    }
    if !matches!(fs, Value::String(_)) {
        return Ok(Value::Null);
    }
    let c = classes(i)?;
    instance(i, if dir.truthy() { c.folder_proto } else { c.file_proto }, fs)
}

fn execute(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, path) = this_path(i, &this, "execute")?;
    let r = host(i, "execute", &[path])?;
    let error = i.rooted(std::slice::from_ref(&r), |i| field(i, &r, "error"))?;
    set_slot(i, o, ERROR, error)?;
    Ok(Value::Bool(false))
}

fn to_string(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    parsed(i, &this, "toString", "uri")
}

/// Whether the host's answer `r` is not `{error}`; when it is, the object's `error` is set to it.
fn outcome(i: &mut Interp, o: ObjId, r: &Value) -> JsResult<bool> {
    let error = i.rooted(std::slice::from_ref(r), |i| field(i, r, "error"))?;
    if matches!(error, Value::String(_)) {
        set_slot(i, o, ERROR, error)?;
        return Ok(false);
    }
    Ok(true)
}

/// Calls the host function `name` with the object's path and `args`. → whether it did the work.
fn path_call(i: &mut Interp, this: &Value, what: &str, name: &str, args: &[Value]) -> JsResult<Value> {
    let (o, path) = this_path(i, this, what)?;
    let mut all = vec![path];
    all.extend_from_slice(args);
    let r = i.rooted(&all, |i| host(i, name, &all))?;
    Ok(Value::Bool(outcome(i, o, &r)?))
}

/// `createAlias`: not available, so it sets `error` and returns `false`.
fn create_alias(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "createAlias")?;
    let m = message(i, "noAlias")?;
    set_slot(i, o, ERROR, m)?;
    Ok(Value::Bool(false))
}

/// Setting `hidden`: not available, so it sets `error` and changes nothing.
fn set_hidden(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "hidden")?;
    let m = message(i, "noHidden")?;
    set_slot(i, o, ERROR, m)?;
    Ok(Value::Undefined)
}

/// `remove()`: the file, or the folder when it is empty, deleted at once.
fn remove(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "remove")?;
    // A file whose changes could not be written is not removed; `error` says why.
    if !close_open(i, o)? {
        return Ok(Value::Bool(false));
    }
    path_call(i, &this, "remove", "remove", &[])
}

/// `rename(name)`: the object's file or folder named `name`, in the same folder; the object takes
/// the new path.
fn rename(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, path) = this_path(i, &this, "rename")?;
    let name = Value::String(i.to_str(&arg(args, 0))?);
    if !close_open(i, o)? {
        return Ok(Value::Bool(false));
    }
    let r = i.rooted(&[path.clone(), name.clone()], |i| host(i, "rename", &[path.clone(), name.clone()]))?;
    if !outcome(i, o, &r)? {
        return Ok(Value::Bool(false));
    }
    let fs = i.rooted(std::slice::from_ref(&r), |i| field(i, &r, "fs"))?;
    set_slot(i, o, PATH, fs)?;
    Ok(Value::Bool(true))
}

/// `copy(target)`: the file copied to `target`, a path or a `File`, replacing a file there.
fn copy(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    this_path(i, &this, "copy")?;
    let p = parse(i, &arg(args, 0))?;
    let fs = i.rooted(std::slice::from_ref(&p), |i| field(i, &p, "fs"))?;
    i.rooted(std::slice::from_ref(&fs), |i| path_call(i, &this, "copy", "copy", std::slice::from_ref(&fs)))
}

/// `Folder.prototype.create()`: the folder, with the folders above it that are missing.
fn create(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    path_call(i, &this, "create", "create", &[])
}

/// Setting `length`: the file cut, or filled with zero bytes, to that many bytes.
fn set_length(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    this_path(i, &this, "length")?;
    let n = i.to_number(&arg(args, 0))?;
    if !n.is_finite() || n < 0.0 {
        return Err(i.throw(ErrorType::Range, "File.length: the length must be a finite number, 0 or more"));
    }
    path_call(i, &this, "length", "setLength", &[Value::Number(n.floor())])?;
    Ok(Value::Undefined)
}

/// Setting `readonly`.
fn set_readonly(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    this_path(i, &this, "readonly")?;
    let on = arg(args, 0).truthy();
    path_call(i, &this, "readonly", "setReadonly", &[Value::Bool(on)])?;
    Ok(Value::Undefined)
}

/// The line end the object's `lineFeed` names: CRLF for `Windows`, CR for `Macintosh` and LF for
/// anything else, without regard to case.
fn line_end(i: &Interp, o: ObjId) -> JsResult<&'static str> {
    Ok(match slot(i, o, LINE_FEED)? {
        Value::String(s) => match s.to_rust_string().to_ascii_lowercase().as_str() {
            "windows" => "\r\n",
            "macintosh" => "\r",
            _ => "\n",
        },
        _ => "\n",
    })
}

/// `write(...values)` and, with `line`, `writeln(...values)`: the values as `String(value)` gives
/// them, written at the position of the object's open file. The values are joined only for an open
/// file, and only while the text stays within twice the host's `maxWrite` UTF-16 code units: each
/// character takes at least one byte and at most two code units, so a longer text cannot fit in a
/// file, and the write sets `error` to the host's `tooLarge` and returns `false`.
fn write_values(i: &mut Interp, this: &Value, args: &[Value], what: &str, line: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, this, what)?;
    let Some(h) = opened(i, o)? else { return Ok(Value::Bool(false)) };
    let c = classes(i)?;
    let max = match i.get_value(&Value::Object(c.host), &key("maxWrite"))? {
        Value::Number(n) if n.is_finite() && n >= 0.0 => (n as usize).saturating_mul(2),
        _ => usize::MAX,
    };
    // The values are joined as UTF-16 code units and made text once, so a surrogate pair split
    // between two values is one character.
    let mut joined: Vec<u16> = Vec::new();
    for a in args {
        i.use_fuel()?;
        let s = i.to_str(a)?;
        if joined.len().saturating_add(s.len()) > max {
            let m = message(i, "tooLarge")?;
            set_slot(i, o, ERROR, m)?;
            return Ok(Value::Bool(false));
        }
        joined.extend_from_slice(&s.flat());
    }
    let mut text = String::from_utf16_lossy(&joined);
    drop(joined);
    if line {
        text.push_str(line_end(i, o)?);
    }
    i.reserve_heap(string_bytes(text.len()))?;
    let text = Value::str(&text);
    let r = i.rooted(&[h.clone(), text.clone()], |i| host(i, "write", &[h.clone(), text.clone()]))?;
    Ok(Value::Bool(outcome(i, o, &r)?))
}

fn write(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    write_values(i, &this, args, "write", false)
}

fn writeln(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    write_values(i, &this, args, "writeln", true)
}

// ---- file dialogs ----

/// Asks the host's `pick` for the dialog `kind` (`open`, `save` or `folder`), with the arguments
/// `[kind, prompt, multi, start, types]`. `me` is the `File` or `Folder` whose `openDlg`, `saveDlg`
/// or `selectDlg` was called, or `None` for the class's own dialog: the dialog starts at its path,
/// and an `{error}` answer sets its `error`. The prompt is `String(prompt)`, or `""` for
/// `undefined` and `null`. A `filter` that is a string gives the file types; one that is a function
/// is called with each file picked ([`picked`]). The run may pause at `pick` until the person
/// answers.
fn ask_pick(i: &mut Interp, kind: &'static str, me: Option<&Value>, prompt: &Value, multi: bool, filter: &Value) -> JsResult<Pause> {
    let start = match me {
        None => Value::str(""),
        Some(me) => this_path(i, me, "dialog")?.1,
    };
    let prompt = match prompt {
        Value::Undefined | Value::Null => Value::str(""),
        p => Value::String(i.to_str(p)?),
    };
    let types = match filter {
        f @ Value::String(_) => f.clone(),
        _ => Value::str(""),
    };
    let pick = host_value(i, "pick")?;
    Ok(Pause {
        host: pick,
        args: vec![Value::str(kind), prompt, Value::Bool(multi), start, types],
        keep: vec![Value::Bool(kind == "folder"), me.cloned().unwrap_or(Value::Null), filter.clone()],
        resume: picked,
    })
}

/// What a file dialog gives for `pick`'s `answer`, with `[folder, me, filter]` kept from
/// [`ask_pick`]: the objects [`wrap`] makes, of which a `filter` that is a function keeps the
/// files it gives a true value for, or `null` when it keeps none.
fn picked(i: &mut Interp, keep: &[Value], answer: Value) -> JsResult<Value> {
    let files = wrap(i, arg(keep, 0).truthy(), &arg(keep, 1), answer)?;
    let filter = arg(keep, 2);
    if !i.is_callable(&filter) {
        return Ok(files);
    }
    let Some(o) = files.as_object() else { return Ok(files) };
    let length = match &i.object(o)?.class {
        Class::Array { length, .. } => Some(*length),
        _ => None,
    };
    let Some(length) = length else {
        let kept = i.rooted(std::slice::from_ref(&files), |i| i.call(&filter, &Value::Undefined, std::slice::from_ref(&files)))?;
        return Ok(if kept.truthy() { files } else { Value::Null });
    };
    i.rooted(std::slice::from_ref(&files), |i| {
        i.reserve_heap(OBJECT_BYTES)?;
        let array_proto = i.intrinsics.array_proto;
        let out = i.heap.alloc_object(Object::new(Class::Array { length: 0, length_writable: true }, Some(array_proto)));
        i.rooted(&[Value::Object(out)], |i| {
            let mut n = 0.0;
            for at in 0..length {
                i.use_fuel()?;
                let file = i.get_value(&files, &index_key(at))?;
                let kept = i.rooted(std::slice::from_ref(&file), |i| i.call(&filter, &Value::Undefined, std::slice::from_ref(&file)))?;
                if kept.truthy() {
                    put_at(i, out, n, file)?;
                    n += 1.0;
                }
            }
            Ok(if n > 0.0 { Value::Object(out) } else { Value::Null })
        })
    })
}

/// The `File` (a `Folder` for a `folder` dialog) for each path `pick` gave in `answer`, an array of
/// them for a list, `null` for `null`, and `null` after setting the `error` of `me`, when it is a
/// `File` or `Folder`, for `{error}`.
fn wrap(i: &mut Interp, folder: bool, me: &Value, answer: Value) -> JsResult<Value> {
    let c = classes(i)?;
    let proto = if folder { c.folder_proto } else { c.file_proto };
    match &answer {
        Value::String(_) => return instance(i, proto, answer.clone()),
        Value::Object(_) => {}
        _ => return Ok(Value::Null),
    }
    let error = i.rooted(std::slice::from_ref(&answer), |i| field(i, &answer, "error"))?;
    if matches!(error, Value::String(_)) {
        if let Some(o) = me.as_object()
            && matches!(slot(i, o, PATH)?, Value::String(_))
        {
            set_slot(i, o, ERROR, error)?;
        }
        return Ok(Value::Null);
    }
    i.rooted(std::slice::from_ref(&answer), |i| {
        let length = i.get_value(&answer, &key("length"))?;
        let n = i.to_number(&length)?;
        i.reserve_heap(OBJECT_BYTES)?;
        let array_proto = i.intrinsics.array_proto;
        let out = i.heap.alloc_object(Object::new(Class::Array { length: 0, length_writable: true }, Some(array_proto)));
        i.rooted(&[Value::Object(out)], |i| {
            let mut at = 0.0;
            while at < n {
                i.use_fuel()?;
                let fs = i.get_value(&answer, &crate::value::number_to_str(at).flat())?;
                let item = i.rooted(std::slice::from_ref(&fs), |i| instance(i, proto, fs.clone()))?;
                put_at(i, out, at, item)?;
                at += 1.0;
            }
            Ok(Value::Object(out))
        })
    })
}

/// `File.openDialog(prompt, filter, multiSelect)`.
fn open_dialog(i: &mut Interp, _: Value, args: &[Value]) -> JsResult<Pause> {
    ask_pick(i, "open", None, &arg(args, 0), arg(args, 2).truthy(), &arg(args, 1))
}

/// `File.saveDialog(prompt, filter)`.
fn save_dialog(i: &mut Interp, _: Value, args: &[Value]) -> JsResult<Pause> {
    ask_pick(i, "save", None, &arg(args, 0), false, &arg(args, 1))
}

/// `File.prototype.openDlg(prompt, filter, multiSelect)`.
fn open_dlg(i: &mut Interp, this: Value, args: &[Value]) -> JsResult<Pause> {
    ask_pick(i, "open", Some(&this), &arg(args, 0), arg(args, 2).truthy(), &arg(args, 1))
}

/// `File.prototype.saveDlg(prompt, filter)`.
fn save_dlg(i: &mut Interp, this: Value, args: &[Value]) -> JsResult<Pause> {
    ask_pick(i, "save", Some(&this), &arg(args, 0), false, &arg(args, 1))
}

/// `Folder.selectDialog(prompt)`.
fn select_dialog(i: &mut Interp, _: Value, args: &[Value]) -> JsResult<Pause> {
    ask_pick(i, "folder", None, &arg(args, 0), false, &Value::Null)
}

/// `Folder.prototype.selectDlg(prompt)`.
fn select_dlg(i: &mut Interp, this: Value, args: &[Value]) -> JsResult<Pause> {
    ask_pick(i, "folder", Some(&this), &arg(args, 0), false, &Value::Null)
}

/// Defines the file dialogs on `File`, `Folder` and their prototypes.
fn dialogs(i: &mut Interp, file: ObjId, file_proto: ObjId, folder: ObjId, folder_proto: ObjId) -> JsResult<()> {
    pausable_method(i, file, "openDialog", open_dialog, 3)?;
    pausable_method(i, file, "saveDialog", save_dialog, 2)?;
    pausable_method(i, file_proto, "openDlg", open_dlg, 3)?;
    pausable_method(i, file_proto, "saveDlg", save_dlg, 2)?;
    pausable_method(i, folder, "selectDialog", select_dialog, 1)?;
    pausable_method(i, folder_proto, "selectDlg", select_dlg, 1)
}

fn text_static(i: &mut Interp, name: &str, args: &[Value]) -> JsResult<Value> {
    let s = i.to_str(&arg(args, 0))?;
    host(i, name, &[Value::String(s)])
}

fn encode(i: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    text_static(i, "encode", args)
}

fn decode(i: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    text_static(i, "decode", args)
}

fn encoding_available(i: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    text_static(i, "encodingAvailable", args)
}

// ---- File ----

fn length(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let Some(r) = stat(i, &this, "length")? else { return Ok(Value::Number(0.0)) };
    i.rooted(std::slice::from_ref(&r), |i| {
        let regular = field(i, &r, "exists")?.truthy() && field(i, &r, "file")?.truthy();
        if regular { field(i, &r, "length") } else { Ok(Value::Number(0.0)) }
    })
}

fn hidden(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    Ok(Value::Bool(stat_field(i, &this, "hidden", "hidden", Value::Bool(false))?.truthy()))
}

fn readonly(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    Ok(Value::Bool(stat_field(i, &this, "readonly", "readonly", Value::Bool(false))?.truthy()))
}

fn four_question_marks(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    this_path(i, &this, "creator")?;
    Ok(Value::str("????"))
}

fn eof(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "eof")?;
    match is_open(i, o)? {
        Some(h) => host(i, "eof", &[h]),
        None => Ok(Value::Bool(true)),
    }
}

fn get_line_feed(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "lineFeed")?;
    slot(i, o, LINE_FEED)
}

fn set_line_feed(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "lineFeed")?;
    let v = i.to_str(&arg(args, 0))?;
    set_slot(i, o, LINE_FEED, Value::String(v))?;
    Ok(Value::Undefined)
}

fn get_encoding(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "encoding")?;
    if let Some(h) = is_open(i, o)? {
        return host(i, "encoding", &[h]);
    }
    Ok(match slot(i, o, ENCODING)? {
        Value::String(s) if s.len() != 0 => Value::String(s),
        _ => Value::str("UTF-8"),
    })
}

fn set_encoding(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "encoding")?;
    let v = Value::String(i.to_str(&arg(args, 0))?);
    match is_open(i, o)? {
        Some(h) => {
            let r = i.rooted(std::slice::from_ref(&v), |i| host(i, "setEncoding", &[h, v.clone()]))?;
            let error = i.rooted(std::slice::from_ref(&r), |i| field(i, &r, "error"))?;
            if matches!(error, Value::String(_)) {
                set_slot(i, o, ERROR, error)?;
            } else {
                set_slot(i, o, ENCODING, v)?;
            }
        }
        None => set_slot(i, o, ENCODING, v)?,
    }
    Ok(Value::Undefined)
}

fn open(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, path) = this_path(i, &this, "open")?;
    let mode = match arg(args, 0) {
        Value::Undefined => "r".to_string(),
        m => i.to_str(&m)?.to_rust_string().to_lowercase(),
    };
    if !matches!(mode.as_str(), "r" | "w" | "e" | "a") {
        let m = message(i, "badMode")?;
        set_slot(i, o, ERROR, m)?;
        return Ok(Value::Bool(false));
    }
    // The object's earlier changes are written first; when they cannot be, it does not open again.
    if !close_open(i, o)? {
        return Ok(Value::Bool(false));
    }
    let encoding = slot(i, o, ENCODING)?;
    let mode = Value::str(&mode);
    let r = host(i, "open", &[path, encoding, mode])?;
    let (error, id) = i.rooted(std::slice::from_ref(&r), |i| Ok((field(i, &r, "error")?, field(i, &r, "id")?)))?;
    if matches!(error, Value::String(_)) {
        set_slot(i, o, ERROR, error)?;
        return Ok(Value::Bool(false));
    }
    set_slot(i, o, HANDLE, id)?;
    let run = classes(i)?.run;
    set_slot(i, o, RUN, Value::Number(run))?;
    Ok(Value::Bool(true))
}

fn close(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "close")?;
    if opened(i, o)?.is_none() {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(close_open(i, o)?))
}

fn read(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "read")?;
    let n = count(i, &arg(args, 0), "File.read")?;
    let Some(h) = opened(i, o)? else { return Ok(Value::str("")) };
    let r = host(i, "read", &[h, n.map_or(Value::Number(-1.0), Value::Number)])?;
    text_result(i, o, r, Value::str(""))
}

fn read_line(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "readln")?;
    let Some(h) = opened(i, o)? else { return Ok(Value::str("")) };
    let r = host(i, "readln", &[h])?;
    text_result(i, o, r, Value::str(""))
}

fn read_char(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "readch")?;
    let Some(h) = opened(i, o)? else { return Ok(Value::str("")) };
    let r = host(i, "readch", &[h])?;
    text_result(i, o, r, Value::str(""))
}

fn seek(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "seek")?;
    let pos = i.to_number(&arg(args, 0))?;
    if !pos.is_finite() {
        return Err(i.throw(ErrorType::Range, "File.seek: the position must be a finite number"));
    }
    let mode = match arg(args, 1) {
        Value::Undefined => 0.0,
        m => i.to_number(&m)?,
    };
    if mode != 0.0 && mode != 1.0 && mode != 2.0 {
        return Err(i.throw(ErrorType::Range, "File.seek: the mode must be 0, 1 or 2"));
    }
    let Some(h) = opened(i, o)? else { return Ok(Value::Bool(false)) };
    host(i, "seek", &[h, Value::Number(pos.floor()), Value::Number(mode)])
}

fn tell(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = this_path(i, &this, "tell")?;
    let Some(h) = opened(i, o)? else { return Ok(Value::Number(-1.0)) };
    host(i, "tell", &[h])
}

// ---- Folder ----

fn get_files(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, path) = this_path(i, &this, "getFiles")?;
    let mask = arg(args, 0);
    let call = i.is_callable(&mask);
    let pattern = if call || matches!(mask, Value::Undefined) { Value::str("*") } else { Value::String(i.to_str(&mask)?) };
    let r = host(i, "list", &[path, pattern])?;
    i.rooted(&[r.clone(), mask.clone()], |i| {
        let error = field(i, &r, "error")?;
        if matches!(error, Value::String(_)) {
            set_slot(i, o, ERROR, error)?;
            return Ok(Value::Null);
        }
        let entries = field(i, &r, "entries")?;
        let length = i.get_value(&entries, &key("length"))?;
        let n = i.to_number(&length)?;
        i.reserve_heap(OBJECT_BYTES)?;
        let proto = i.intrinsics.array_proto;
        let out = i.heap.alloc_object(Object::new(Class::Array { length: 0, length_writable: true }, Some(proto)));
        i.rooted(&[Value::Object(out)], |i| {
            let c = classes(i)?;
            let mut kept = 0.0;
            let mut at = 0.0;
            while at < n {
                i.use_fuel()?;
                let e = i.get_value(&entries, &crate::value::number_to_str(at).flat())?;
                let (fs, dir) = i.rooted(std::slice::from_ref(&e), |i| Ok((field(i, &e, "fs")?, field(i, &e, "dir")?.truthy())))?;
                let item = instance(i, if dir { c.folder_proto } else { c.file_proto }, fs)?;
                let keep = if call {
                    i.rooted(std::slice::from_ref(&item), |i| i.call(&mask, &Value::Undefined, std::slice::from_ref(&item)))?.truthy()
                } else {
                    true
                };
                if keep {
                    put_at(i, out, kept, item)?;
                    kept += 1.0;
                }
                at += 1.0;
            }
            Ok(Value::Object(out))
        })
    })
}

fn get_current(i: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let c = classes(i)?;
    instance(i, c.folder_proto, c.current)
}

fn set_current(i: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let p = parse(i, &arg(args, 0))?;
    let fs = i.rooted(std::slice::from_ref(&p), |i| field(i, &p, "fs"))?;
    if let Some(c) = i.files.as_mut() {
        c.current = fs;
    }
    Ok(Value::Undefined)
}

/// `Folder.<place>`: a new `Folder` for the host's location `place`, or `null`.
fn place(i: &mut Interp, place: &str) -> JsResult<Value> {
    let all = host(i, "locations", &[Value::str(place)])?;
    let fs = i.rooted(std::slice::from_ref(&all), |i| field(i, &all, place))?;
    if !matches!(fs, Value::String(_)) {
        return Ok(Value::Null);
    }
    let proto = classes(i)?.folder_proto;
    instance(i, proto, fs)
}

macro_rules! places {
    ($($f:ident => $name:literal),* $(,)?) => {
        $(fn $f(i: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
            place(i, $name)
        })*
        /// The getter of each of [`PLACES`], in its order.
        const PLACE_GETTERS: [NativeFn; 10] = [$($f),*];
    };
}

places!(
    app_data => "appData",
    app_package => "appPackage",
    common_files => "commonFiles",
    desktop => "desktop",
    my_documents => "myDocuments",
    startup => "startup",
    system => "system",
    temp => "temp",
    trash => "trash",
    user_data => "userData",
);

// ---- install ----

/// Defines the accessor `name` on `on`, not enumerable, as the help pages' objects have them.
fn accessor(i: &mut Interp, on: ObjId, name: &'static str, get: NativeFn, set: Option<NativeFn>) -> JsResult<()> {
    let g = native(i, name, get, 0, NativeKind::Plain, false)?;
    i.rooted(&[Value::Object(g)], |i| {
        let s = match set {
            Some(f) => Some(native(i, name, f, 1, NativeKind::Plain, false)?),
            None => None,
        };
        i.define(on, key(name), Prop::Accessor { get: Some(g), set: s, flags: CONFIGURABLE })
    })
}

/// What `File` and `Folder` both have.
fn common(i: &mut Interp, ctor: ObjId, proto: ObjId) -> JsResult<()> {
    accessor(i, proto, "absoluteURI", absolute_uri, None)?;
    accessor(i, proto, "fullName", absolute_uri, None)?;
    accessor(i, proto, "fsName", fs_name, None)?;
    accessor(i, proto, "relativeURI", relative_uri, None)?;
    accessor(i, proto, "name", name, None)?;
    accessor(i, proto, "displayName", display_name, None)?;
    accessor(i, proto, "path", path, None)?;
    accessor(i, proto, "parent", parent, None)?;
    accessor(i, proto, "exists", exists, None)?;
    accessor(i, proto, "created", created, None)?;
    accessor(i, proto, "modified", modified, None)?;
    accessor(i, proto, "alias", alias, None)?;
    accessor(i, proto, "error", get_error, Some(set_error))?;
    method(i, proto, "toString", to_string, 0)?;
    method(i, proto, "getRelativeURI", get_relative_uri, 1)?;
    method(i, proto, "changePath", change_path, 1)?;
    method(i, proto, "resolve", resolve, 0)?;
    method(i, proto, "execute", execute, 0)?;
    method(i, proto, "remove", remove, 0)?;
    method(i, proto, "rename", rename, 1)?;
    method(i, ctor, "encode", encode, 1)?;
    method(i, ctor, "decode", decode, 1)?;
    method(i, ctor, "isEncodingAvailable", encoding_available, 1)?;
    let system = host_value(i, "system")?;
    data(i, ctor, "fs", system, HIDDEN)
}

fn file_members(i: &mut Interp, proto: ObjId) -> JsResult<()> {
    accessor(i, proto, "length", length, Some(set_length))?;
    accessor(i, proto, "hidden", hidden, Some(set_hidden))?;
    accessor(i, proto, "readonly", readonly, Some(set_readonly))?;
    accessor(i, proto, "creator", four_question_marks, None)?;
    accessor(i, proto, "type", four_question_marks, None)?;
    accessor(i, proto, "eof", eof, None)?;
    accessor(i, proto, "lineFeed", get_line_feed, Some(set_line_feed))?;
    accessor(i, proto, "encoding", get_encoding, Some(set_encoding))?;
    method(i, proto, "open", open, 1)?;
    method(i, proto, "close", close, 0)?;
    method(i, proto, "read", read, 1)?;
    method(i, proto, "readln", read_line, 0)?;
    method(i, proto, "readch", read_char, 0)?;
    method(i, proto, "seek", seek, 2)?;
    method(i, proto, "tell", tell, 0)?;
    method(i, proto, "write", write, 1)?;
    method(i, proto, "writeln", writeln, 1)?;
    method(i, proto, "copy", copy, 1)?;
    method(i, proto, "createAlias", create_alias, 1)
}

fn folder_members(i: &mut Interp, ctor: ObjId, proto: ObjId) -> JsResult<()> {
    method(i, proto, "getFiles", get_files, 1)?;
    method(i, proto, "create", create, 0)?;
    accessor(i, ctor, "current", get_current, Some(set_current))?;
    for (place, get) in PLACES.into_iter().zip(PLACE_GETTERS) {
        accessor(i, ctor, place, get, None)?;
    }
    Ok(())
}

/// Sets `Folder.current` back to the host's `current`, when the realm has `File` and `Folder`, and
/// begins a new run, in which the files earlier runs opened are not open.
pub(crate) fn reset_current(i: &mut Interp) -> JsResult<()> {
    let Some(c) = i.files.as_mut() else { return Ok(()) };
    c.run += 1.0;
    let start = host(i, "current", &[])?;
    if let Some(c) = i.files.as_mut() {
        c.current = start;
    }
    Ok(())
}

/// Adds `File` and `Folder` to the global object, their work done by the host functions in
/// `items` ([`crate::Realm::define_file_classes`]).
pub(crate) fn install(i: &mut Interp, items: Vec<(String, HostItem)>) -> JsResult<()> {
    let host_obj = i.host_object("File", items)?;
    let object_proto = i.intrinsics.object_proto;
    i.rooted(&[Value::Object(host_obj)], |i| {
        i.reserve_heap(2 * OBJECT_BYTES)?;
        let file_proto = i.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));
        let folder_proto = i.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));
        i.files = Some(Classes { file_proto, folder_proto, host: host_obj, current: Value::str(""), run: 0.0 });
        let start = host(i, "current", &[])?;
        if let Some(c) = i.files.as_mut() {
            c.current = start;
        }
        let file = native(i, "File", file_ctor, 1, NativeKind::Plain, true)?;
        constructor(i, "File", file, file_proto)?;
        let folder = native(i, "Folder", folder_ctor, 1, NativeKind::Plain, true)?;
        constructor(i, "Folder", folder, folder_proto)?;
        common(i, file, file_proto)?;
        common(i, folder, folder_proto)?;
        file_members(i, file_proto)?;
        folder_members(i, folder, folder_proto)?;
        dialogs(i, file, file_proto, folder, folder_proto)
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::{Data, HostError, HostItem, JsValue, Progress, Realm};

    /// A host whose paths are `/` and a name, where `/d` is a folder holding `a` and `b` and every
    /// other path is missing.
    fn realm() -> Realm {
        realm_with(|_| Vec::new())
    }

    /// [`realm`] with a `pick` that waits where it can and throws where it cannot. → the arguments
    /// of each call, joined with `|`.
    fn dialog_realm() -> (Realm, Rc<RefCell<Vec<String>>>) {
        let asked: Rc<RefCell<Vec<String>>> = Rc::default();
        let calls = asked.clone();
        let r = realm_with(move |r| {
            let can_wait = r.wait_flag();
            let pick = move |a: Vec<Data>| -> Result<Data, HostError> {
                let text = |d: &Data| match d {
                    Data::String(s) => s.clone(),
                    Data::Bool(b) => b.to_string(),
                    other => format!("{other:?}"),
                };
                calls.borrow_mut().push(a.iter().map(text).collect::<Vec<_>>().join("|"));
                if can_wait.get() { Err(HostError::Wait) } else { Err(HostError::Error("cannot wait here".into())) }
            };
            vec![("pick".into(), HostItem::Function(Rc::new(pick)))]
        });
        (r, asked)
    }

    /// Advances the run until it waits or ends. → its value when it ended.
    fn until_wait(r: &mut Realm) -> Option<JsValue> {
        for _ in 0..10_000 {
            match r.advance(1_000).unwrap() {
                Progress::Done(v) => return Some(v),
                Progress::Paused if r.waits() => return None,
                Progress::Paused => {}
            }
        }
        panic!("the run neither waited nor ended")
    }

    fn realm_with(more: impl FnOnce(&Realm) -> Vec<(String, HostItem)>) -> Realm {
        let f = |g: fn(&[Data]) -> Data| HostItem::Function(Rc::new(move |a: Vec<Data>| Ok(g(&a))));
        let text = |a: &[Data], i: usize| match a.get(i) {
            Some(Data::String(s)) => s.clone(),
            _ => String::new(),
        };
        let obj = |fields: Vec<(&str, Data)>| Data::Object(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect());
        let parse = move |a: &[Data]| {
            let t = text(a, 0);
            let fs = if t.starts_with('/') { t } else { format!("/{t}") };
            let name = fs.trim_start_matches('/').to_string();
            obj(vec![
                ("fs", Data::String(fs.clone())),
                ("uri", Data::String(fs)),
                ("name", Data::String(name.clone())),
                ("display", Data::String(name)),
                ("path", Data::String("/".into())),
                ("parent", Data::String("/".into())),
            ])
        };
        let messages = ["notOpen", "badMode", "noAlias", "noHidden"].map(|k| (k.to_string(), HostItem::Value(Data::String(k.into()))));
        let stat = move |a: Vec<Data>| -> Result<Data, crate::HostError> {
            let dir = text(&a, 0) == "/d";
            Ok(obj(vec![("exists", Data::Bool(dir)), ("dir", Data::Bool(dir))]))
        };
        let list = move |_: Vec<Data>| -> Result<Data, crate::HostError> {
            let e = |n: &str| obj(vec![("fs", Data::String(format!("/d/{n}"))), ("dir", Data::Bool(false))]);
            Ok(obj(vec![("entries", Data::Array(vec![e("a"), e("b")]))]))
        };
        let mut r = Realm::new();
        let mut items: Vec<(String, HostItem)> = vec![
            ("text".into(), HostItem::Object(messages.to_vec())),
            ("system".into(), HostItem::Value(Data::String("Unix".into()))),
            ("lineFeed".into(), HostItem::Value(Data::String("Unix".into()))),
            ("current".into(), f(|_| Data::String("/".into()))),
            ("parse".into(), HostItem::Function(Rc::new(move |a: Vec<Data>| Ok(parse(&a))))),
            ("stat".into(), HostItem::Function(Rc::new(stat))),
            ("list".into(), HostItem::Function(Rc::new(list))),
            ("tempName".into(), f(|_| Data::String("/tmp1".into()))),
            ("locations".into(), f(|_| Data::Object(vec![]))),
        ];
        items.extend(more(&r));
        r.define_file_classes(items).unwrap();
        r
    }

    #[test]
    fn a_dialog_pauses_the_run_and_its_answer_becomes_files() {
        let (mut r, asked) = dialog_realm();
        let v = r.run("[File.openDialog.length, File.prototype.saveDlg.length, String(Folder.selectDialog).indexOf('[native code]') > 0].join()");
        assert_eq!(v.unwrap(), JsValue::String("3,2,true".into()), "the dialogs are built-ins");
        r.start("var f = File.openDialog('Pick', 'Text:*.txt', 1); [f.length, f[0].name, f[1].fsName, f[1] instanceof File].join()").unwrap();
        assert_eq!(until_wait(&mut r), None);
        assert_eq!(*asked.borrow(), ["open|Pick|true||Text:*.txt"]);
        r.answer(Data::Array(vec![Data::String("/a".into()), Data::String("/b".into())])).unwrap();
        assert_eq!(until_wait(&mut r), Some(JsValue::String("2,a,/b,true".into())));
        r.start("var g = new Folder('d').selectDlg(5); [g.name, g instanceof Folder].join()").unwrap();
        assert_eq!(until_wait(&mut r), None);
        assert_eq!(asked.borrow()[1], "folder|5|false|/d|", "the folder's own dialog starts at its path");
        r.answer(Data::String("/e".into())).unwrap();
        assert_eq!(until_wait(&mut r), Some(JsValue::String("e,true".into())));
    }

    #[test]
    fn a_canceled_dialog_gives_null_and_an_error_answer_sets_error() {
        let (mut r, asked) = dialog_realm();
        r.start(
            "var d = new File('x'); var a = File.saveDialog(); var b = d.saveDlg('Save', function () { return true; }); [a, b, d.error].join('|')",
        )
        .unwrap();
        assert_eq!(until_wait(&mut r), None);
        r.answer(Data::Null).unwrap();
        assert_eq!(until_wait(&mut r), None, "a canceled dialog lets the run go on to the next one");
        assert_eq!(asked.borrow()[1], "save|Save|false|/x|", "a filter that is not a string gives no file types");
        r.answer(Data::Object(vec![("error".into(), Data::String("denied".into()))])).unwrap();
        assert_eq!(until_wait(&mut r), Some(JsValue::String("||denied".into())));
        r.start("File.openDialog(); 'went on'").unwrap();
        assert_eq!(until_wait(&mut r), None);
        r.end();
        assert!(!r.waits() && !r.is_running(), "a run ended at a dialog drops it");
        assert_eq!(r.run("'next'").unwrap(), JsValue::String("next".into()));
        let e = r.run("File.openDialog()").unwrap_err();
        assert!(e.message.contains("cannot wait here"), "a run that cannot pause does not wait: {e:?}");
        let e = r.run("File.prototype.openDlg.call({})").unwrap_err();
        assert!(e.message.contains("is not a File or a Folder"), "{e:?}");
    }

    #[test]
    fn a_dialog_in_a_function_or_a_loop_resumes_where_it_was_called() {
        let (mut r, asked) = dialog_realm();
        let script = "var seen = [];
            function pickOne(n) {
                try { var f = File.openDialog('n' + n, function (f) { return f.name !== 'skip'; }); return f === null ? 'none' : f.name; }
                finally { seen.push('f' + n); }
            }
            for (var k = 0; k < 3; k++) seen.push(pickOne(k));
            var many = File.openDialog('m', function (f) { return f.name !== 'b'; }, true);
            var thrown;
            try { File.openDialog('t', function () { throw new Error('no'); }); } catch (e) { thrown = e.message; }
            seen.concat([many.length, many[1].name, thrown]).join()";
        r.start(script).unwrap();
        for answer in ["/a", "/skip", "/c"] {
            assert_eq!(until_wait(&mut r), None);
            r.answer(Data::String(answer.into())).unwrap();
        }
        assert_eq!(until_wait(&mut r), None);
        r.answer(Data::Array(["/a", "/b", "/c"].map(|p| Data::String(p.into())).to_vec())).unwrap();
        assert_eq!(until_wait(&mut r), None);
        r.collect_garbage();
        r.answer(Data::String("/a".into())).unwrap();
        assert_eq!(until_wait(&mut r), Some(JsValue::String("f0,a,f1,none,f2,c,2,c,no".into())));
        assert_eq!(asked.borrow().len(), 5, "each dialog asked once");
    }

    #[test]
    fn objects_keep_their_classes_through_collections() {
        let mut r = realm();
        r.run("var kept = new File('x'); var listed = new Folder('d').getFiles(); File = null; Folder = null;").unwrap();
        r.collect_garbage();
        r.run("var many = []; for (var i = 0; i < 2000; i++) many.push(kept.parent);").unwrap();
        r.collect_garbage();
        let v = r.run(
            "[kept.name, kept.fsName, listed.length, listed[1].name, kept.parent.getFiles(function (f) { return f.name === 'd/a'; }).length].join()",
        );
        assert_eq!(v.unwrap(), JsValue::String("x,/x,2,d/b,1".into()));
    }

    #[test]
    fn the_constructor_picks_the_class_only_without_new() {
        let mut r = realm();
        let v = r.run(
            "var F = File, D = Folder; [F('d') instanceof D, new F('d') instanceof F, D('x') instanceof D, \
             typeof Object.keys(new F('x')).length, Object.keys(new F('x')).length].join()",
        );
        assert_eq!(v.unwrap(), JsValue::String("true,true,true,number,0".into()));
        let e = r.run("File.prototype.read.call({})").unwrap_err();
        assert!(e.message.contains("is not a File or a Folder"), "{e:?}");
    }
}
