//! ScriptUI's `Window` with modal dialogs, as the help page
//! <https://extendscript.docsforadobe.dev/user-interface-tools/window-object/> describes it, whose
//! drawing a host does ([`crate::Realm::define_script_ui`]).
//!
//! `new Window("dialog"[, title, bounds, properties])` makes a dialog, with or without `new`, and
//! `new Window("palette", ...)` a palette, once the host's `palette` allows it. A window of type
//! `window`, and a resource specification in place of the type, throw an `Error` that says they are
//! not available yet. `add(type[, bounds, text, properties])` adds a
//! control to a dialog, a palette, a `group` or a `panel` and returns it: `statictext`, `edittext`, `button`,
//! `checkbox`, `radiobutton`, `dropdownlist`, `listbox` (whose third argument is an array of item
//! texts), `slider` (third to fifth: value, minimum, maximum) and `progressbar` (value, maximum).
//! The other documented control types throw an `Error` that says they are not available yet. A
//! list's `add("item", text)` adds an item; its `selection` is an item or `null`, and takes an
//! item, an index or `null`.
//!
//! `show()` gives the host's `open` the dialog's description and runs `onShow`. Then, until the
//! dialog closes, it gives the host's `wait` the description again, and `wait` gives back what the
//! person did, which may pause the run ([`crate::interp::Pause`]): a `click` on a control, a
//! `change` with its new value, or a `close`. The control takes the change, its `onClick`,
//! `onChanging` or `onChange` runs, and the next description shows what the handler changed. A
//! button without `onClick` closes the dialog with 1 when it is the dialog's default element and
//! with 2 when it is its cancel element. `close(result)` closes the dialog after the running handler
//! returns, and `show()` gives `result`, or 0 without one; `onClose` runs first and keeps the dialog
//! open when it returns `false`. A dialog that is showing cannot be shown again, and no other dialog
//! can be shown while one is. When `show()` ends, the host's `end` is given its result and whether
//! it ended normally.
//!
//! A palette's `show()` gives the host's `paletteShow` its description and its id (none the first
//! time), which gives back the id, then runs `onShow` and returns `undefined` without waiting.
//! `hide()` gives `paletteHide` the id, and `close()` runs `onClose`, and unless it returns `false`
//! gives `paletteClose` the id; a `close()` while `onClose` runs does nothing more. The realm keeps
//! each palette the host shows until it closes. `Window.__vcPaletteEvent()` takes an event from the
//! host's `paletteEvent` (`{palette, event, id, value}`, or `null`), gives it to that palette as a
//! dialog takes one, and then gives `paletteUpdate` each palette's description; `add()` on a shown
//! palette does the same for that palette, so the host's list of its controls stays in step.
//! `update()` on a shown palette, or on a `group` or a `panel` in one, takes each event that waits
//! for a palette of the run's engine from the host's `paletteTake` (as `paletteEvent` gives it, or
//! `null`) and gives it to its palette, while the host's `palettePending` says one waits, giving
//! `paletteUpdate` each palette's description before each one. Then, called from the script's own code (not from a
//! handler, a callback of a built-in function, a getter or a conversion), it asks the host's
//! `paletteDraw` whether the palette is drawn now, and when it is, gives `paletteUpdate` the
//! palette's description. On the showing dialog, or a group or panel in it, `update()` does
//! nothing more, and on a window that is not showing it throws an `Error`. A dialog takes the
//! person's events once the running handler returns.
//!
//! An element keeps its kind, its container, its controls and a list's items in properties that a
//! script cannot change. The description lists the dialog first and every control after the
//! container that holds it; it is read with a loop, and a dialog with more than [`MAX_CONTROLS`]
//! controls, nested more than [`MAX_DEPTH`] deep, with more than [`MAX_ITEMS`] list items, or with a
//! text longer than [`MAX_TEXT_BYTES`] throws a `RangeError`. These are VectorCraft's own limits. A
//! getter that changes a list's items after the list was read makes `show()` read the dialog again,
//! and a list that changes again throws an `Error`.

use crate::HostItem;
use crate::builtins::{arg, constructor, method, native, pausable_method};
use crate::heap::{Class, ENUMERABLE, NativeKind, OBJECT_BYTES, OPEN, ObjId, Object, PROP_BYTES, Prop, key};
use crate::host::Data;
use crate::interp::{Abrupt, ErrorType, Interp, JsResult, Pause};
use crate::value::{Value, index_key};

/// The most controls a dialog holds, its groups and panels included.
pub const MAX_CONTROLS: usize = 512;
/// How deeply controls nest in a dialog: a control the dialog holds is at depth 1.
pub const MAX_DEPTH: usize = 16;
/// The most items a dialog's lists hold together.
pub const MAX_ITEMS: usize = 1024;
/// The most bytes (UTF-8) of one text in a dialog: a control's text or name, or a list item's.
pub const MAX_TEXT_BYTES: usize = 16 << 10;

/// The control types `add` makes.
pub const BUILT: [&str; 11] =
    ["group", "panel", "statictext", "edittext", "button", "checkbox", "radiobutton", "dropdownlist", "listbox", "slider", "progressbar"];
/// The documented control types that are not available yet.
const LATER: [&str; 10] =
    ["tabbedpanel", "tab", "scrollbar", "image", "iconbutton", "treeview", "flashplayer", "customview", "custombutton", "customboundedvalue"];

const KIND: &str = "__vcKind";
const PARENT: &str = "__vcParent";
const WINDOW: &str = "__vcWindow";
const KIDS: &str = "__vcKids";
const DEPTH: &str = "__vcDepth";
const COUNT: &str = "__vcCount";
const ITEM_COUNT: &str = "__vcItemCount";
const CLOSING: &str = "__vcClosing";
const ITEMS: &str = "__vcItems";
const SELECTION: &str = "__vcSelection";
const LIST: &str = "__vcList";
const PALETTE_ID: &str = "__vcPaletteId";
const SHOWN: &str = "__vcShown";
const IN_CLOSE: &str = "__vcInClose";
/// How many times a list's items were added or removed: a palette's waiting change of the list is
/// for the items it had then.
const REVISION: &str = "__vcRevision";

/// The objects of `Window`, kept by the realm, the dialog that is showing, and the palettes the
/// host shows, shown or hidden.
#[derive(Clone, Debug)]
pub(crate) struct Classes {
    window_proto: ObjId,
    control_proto: ObjId,
    item_proto: ObjId,
    layout: ObjId,
    host: ObjId,
    selection_get: ObjId,
    selection_set: ObjId,
    showing: Option<ObjId>,
    palettes: Vec<ObjId>,
    /// Whether an `update()` is taking the waiting events ([`take_waiting`]).
    taking: bool,
}

impl Classes {
    /// The objects a collection keeps.
    pub(crate) fn objects(&self) -> Vec<ObjId> {
        let mut all = vec![self.window_proto, self.control_proto, self.item_proto, self.layout, self.host, self.selection_get, self.selection_set];
        all.extend(self.showing);
        all.extend(self.palettes.iter().copied());
        all
    }
}

/// Is `kind` a window's: a dialog's or a palette's?
fn is_window(kind: &str) -> bool {
    kind == "dialog" || kind == "palette"
}

/// Does `kind` hold controls?
fn is_container(kind: &str) -> bool {
    is_window(kind) || kind == "group" || kind == "panel"
}

/// Forgets the dialog that was showing when a run ended where it was.
pub(crate) fn end_run(i: &mut Interp) {
    if let Some(c) = i.script_ui.as_mut() {
        c.showing = None;
    }
}

fn classes(i: &Interp) -> JsResult<Classes> {
    i.script_ui.clone().ok_or_else(|| Abrupt::Internal("Window is not installed".into()))
}

fn error(i: &mut Interp, message: &str) -> Abrupt {
    i.throw(ErrorType::Error, message)
}

fn range(i: &mut Interp, message: &str) -> Abrupt {
    i.throw(ErrorType::Range, message)
}

/// The value of the own data property `name` of `o`, `undefined` when it has none.
fn slot(i: &Interp, o: ObjId, name: &str) -> JsResult<Value> {
    Ok(match i.get_own_property(o, &key(name))? {
        Some(Prop::Data { value, .. }) => value,
        _ => Value::Undefined,
    })
}

/// Sets the property `name` of `o` that a script can read but not change.
fn set_slot(i: &mut Interp, o: ObjId, name: &str, value: Value) -> JsResult<()> {
    i.rooted(std::slice::from_ref(&value), |i| i.reserve_heap(PROP_BYTES))?;
    i.define(o, key(name), Prop::Data { value, flags: 0 })
}

/// Sets the property `name` of `o` that a script can change.
fn set_open(i: &mut Interp, o: ObjId, name: &str, value: Value) -> JsResult<()> {
    i.rooted(std::slice::from_ref(&value), |i| i.reserve_heap(PROP_BYTES))?;
    i.define(o, key(name), Prop::Data { value, flags: OPEN })
}

fn number_slot(i: &Interp, o: ObjId, name: &str) -> JsResult<f64> {
    Ok(match slot(i, o, name)? {
        Value::Number(n) => n,
        _ => 0.0,
    })
}

fn object_slot(i: &Interp, o: ObjId, name: &str) -> JsResult<Option<ObjId>> {
    Ok(slot(i, o, name)?.as_object())
}

/// An element's kind, or `None` for a value that is not a ScriptUI window or control.
fn kind_of(i: &Interp, v: &Value) -> JsResult<Option<(ObjId, String)>> {
    let Some(o) = v.as_object() else { return Ok(None) };
    Ok(match slot(i, o, KIND)? {
        Value::String(s) => Some((o, s.to_rust_string())),
        _ => None,
    })
}

/// `this` as an element, or a `TypeError` naming `what`.
fn element(i: &mut Interp, this: &Value, what: &str) -> JsResult<(ObjId, String)> {
    match kind_of(i, this)? {
        Some(e) => Ok(e),
        None => Err(i.type_error(&format!("{what}: this is not a ScriptUI window or control"))),
    }
}

/// Are `a` and `b` the same object?
fn same(a: &Value, b: &Value) -> bool {
    matches!((a.as_object(), b.as_object()), (Some(x), Some(y)) if x == y)
}

/// The values of an array made by [`list`].
fn list_values(i: &Interp, a: ObjId) -> JsResult<Vec<Value>> {
    let length = match &i.object(a)?.class {
        Class::Array { length, .. } => *length,
        _ => return Ok(Vec::new()),
    };
    (0..length)
        .map(|n| {
            Ok(match i.get_own_property(a, &index_key(n))? {
                Some(Prop::Data { value, .. }) => value,
                _ => Value::Undefined,
            })
        })
        .collect()
}

/// A new array holding `items`: one a script cannot change unless `open`.
fn list(i: &mut Interp, items: &[Value], open: bool) -> JsResult<ObjId> {
    i.rooted(items, |i| i.reserve_heap(OBJECT_BYTES.saturating_add(PROP_BYTES.saturating_mul(items.len() + 1))))?;
    let proto = i.intrinsics.array_proto;
    let a = i.heap.alloc_object(Object::new(Class::Array { length: 0, length_writable: open }, Some(proto)));
    let flags = if open { OPEN } else { ENUMERABLE };
    for (n, v) in items.iter().enumerate() {
        let n = u32::try_from(n).map_err(|_| Abrupt::Internal("a list is too long".into()))?;
        i.define(a, index_key(n), Prop::Data { value: v.clone(), flags })?;
    }
    i.object_mut(a)?.extensible = open;
    Ok(a)
}

/// Adds `v` to the end of the list in `o`'s slot `name`, and puts a copy a script can change in
/// its property `public`.
fn append(i: &mut Interp, o: ObjId, name: &str, public: &str, v: Value) -> JsResult<()> {
    let mut items = match object_slot(i, o, name)? {
        Some(a) => list_values(i, a)?,
        None => Vec::new(),
    };
    items.push(v);
    i.rooted(&items.clone(), |i| {
        let kept = list(i, &items, false)?;
        set_slot(i, o, name, Value::Object(kept))?;
        let copy = list(i, &items, true)?;
        set_open(i, o, public, Value::Object(copy))
    })
}

fn text_arg(i: &mut Interp, v: &Value) -> JsResult<Value> {
    Ok(match v {
        Value::Undefined => Value::str(""),
        v => Value::String(i.to_str(v)?),
    })
}

/// A new element of `kind`, with the properties every element has.
fn new_element(
    i: &mut Interp,
    proto: ObjId,
    kind: &str,
    parent: Option<ObjId>,
    window: Option<ObjId>,
    depth: usize,
    args: [&Value; 3],
) -> JsResult<ObjId> {
    let [bounds, text, props] = args;
    i.reserve_heap(OBJECT_BYTES.saturating_add(PROP_BYTES.saturating_mul(32)))?;
    let o = i.heap.alloc_object(Object::new(Class::Ordinary, Some(proto)));
    i.temp_roots.push(Value::Object(o));
    let window = window.unwrap_or(o);
    let parent = parent.map_or(Value::Null, Value::Object);
    let layout = classes(i)?.layout;
    set_slot(i, o, KIND, Value::str(kind))?;
    set_slot(i, o, PARENT, parent.clone())?;
    set_slot(i, o, WINDOW, Value::Object(window))?;
    set_slot(i, o, DEPTH, Value::Number(depth as f64))?;
    set_open(i, o, "type", Value::str(kind))?;
    set_open(i, o, "parent", parent)?;
    set_open(i, o, "window", Value::Object(window))?;
    let text = text_arg(i, text)?;
    set_open(i, o, "text", text)?;
    let props = match props {
        Value::Object(_) => props.clone(),
        _ => Value::Object(i.new_object()),
    };
    set_open(i, o, "properties", props.clone())?;
    let name = i.get_value(&props, &key("name"))?;
    let name = text_arg(i, &name)?;
    set_open(i, o, "name", name)?;
    set_open(i, o, "enabled", Value::Bool(true))?;
    set_open(i, o, "visible", Value::Bool(true))?;
    if !matches!(bounds, Value::Undefined | Value::Null) {
        set_open(i, o, "bounds", bounds.clone())?;
    }
    set_open(i, o, "layout", Value::Object(layout))?;
    if is_container(kind) {
        let kids = list(i, &[], false)?;
        set_slot(i, o, KIDS, Value::Object(kids))?;
        let children = list(i, &[], true)?;
        set_open(i, o, "children", Value::Object(children))?;
        set_open(i, o, "orientation", Value::str(if kind == "group" { "row" } else { "column" }))?;
        set_open(i, o, "alignChildren", Value::str("center"))?;
        set_open(i, o, "spacing", Value::Number(10.0))?;
        set_open(i, o, "margins", Value::Number(if kind == "group" { 0.0 } else { 15.0 }))?;
    }
    Ok(o)
}

/// `new Window(type[, title, bounds, properties])`.
fn window_ctor(i: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let kind = i.to_str(&arg(args, 0))?.to_rust_string();
    if kind.contains('{') {
        return Err(error(i, "Window: resource specifications are not available yet; build the dialog with add()"));
    }
    let lower = kind.to_lowercase();
    if lower == "window" {
        return Err(error(i, "Window: a window of type window is not available yet; use dialog or palette"));
    }
    if lower != "dialog" && lower != "palette" {
        return Err(error(i, "Window: the type must be dialog, palette or window"));
    }
    if lower == "palette" {
        let allowed = host_fn(i, "palette")?;
        if !i.is_callable(&allowed) {
            return Err(error(i, "Window: this host shows no palettes"));
        }
        i.call(&allowed, &Value::Undefined, &[])?;
    }
    let ty = if lower == "palette" { "palette" } else { "dialog" };
    let proto = classes(i)?.window_proto;
    let mark = i.temp_roots.len();
    let r = (|| {
        let o = new_element(i, proto, ty, None, None, 0, [&arg(args, 2), &arg(args, 1), &arg(args, 3)])?;
        set_slot(i, o, COUNT, Value::Number(0.0))?;
        set_slot(i, o, ITEM_COUNT, Value::Number(0.0))?;
        Ok(Value::Object(o))
    })();
    i.temp_roots.truncate(mark);
    r
}

fn number_arg(i: &mut Interp, v: &Value, what: &str) -> JsResult<f64> {
    let n = i.to_number(v)?;
    if !n.is_finite() {
        return Err(range(i, &format!("{what} is not a finite number")));
    }
    Ok(n)
}

/// `container.add(type[, bounds, text, properties])` and `list.add("item", text)`.
fn add(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, kind) = element(i, &this, "add")?;
    let ty = i.to_str(&arg(args, 0))?.to_rust_string();
    if kind == "dropdownlist" || kind == "listbox" {
        if ty.to_lowercase() != "item" {
            return Err(error(i, "add: a list holds only items"));
        }
        let item = add_item(i, o, &arg(args, 1))?;
        if let Some(win) = object_slot(i, o, WINDOW)? {
            i.rooted(std::slice::from_ref(&item), |i| update_palette(i, win))?;
        }
        return Ok(item);
    }
    if !is_container(&kind) {
        return Err(error(i, &format!("add: a {kind} holds no controls")));
    }
    if ty.contains('{') {
        return Err(error(i, "add: resource specifications are not available yet; add each control by its type"));
    }
    let ty = ty.to_lowercase();
    if LATER.contains(&ty.as_str()) {
        return Err(error(i, &format!("add: the control type {ty} is not available yet")));
    }
    let Some(&ty) = BUILT.iter().find(|b| **b == ty) else { return Err(error(i, &format!("add: there is no control type {ty}"))) };
    let depth = number_slot(i, o, DEPTH)? as usize + 1;
    if depth > MAX_DEPTH {
        return Err(range(i, &format!("add: controls can be nested at most {MAX_DEPTH} deep")));
    }
    let Some(win) = object_slot(i, o, WINDOW)? else { return Err(Abrupt::Internal("a control without its window".into())) };
    let count = number_slot(i, win, COUNT)?;
    if count as usize >= MAX_CONTROLS {
        return Err(range(i, &format!("add: a dialog can hold at most {MAX_CONTROLS} controls")));
    }
    let props = arg(args, 3);
    let list_kind = ty == "dropdownlist" || ty == "listbox";
    if ty == "listbox" && props.as_object().is_some() && i.get_value(&props, &key("multiselect"))?.truthy() {
        return Err(error(i, "add: a list box that selects several items is not available yet"));
    }
    let proto = classes(i)?.control_proto;
    let mark = i.temp_roots.len();
    let r = (|| {
        let creation = if matches!(ty, "slider" | "progressbar") { Value::Undefined } else { props.clone() };
        let text = if list_kind || matches!(ty, "slider" | "progressbar") { Value::Undefined } else { arg(args, 2) };
        let c = new_element(i, proto, ty, Some(o), Some(win), depth, [&arg(args, 1), &text, &creation])?;
        match ty {
            "checkbox" | "radiobutton" => set_open(i, c, "value", Value::Bool(false))?,
            "slider" | "progressbar" => {
                let (value, min, max) =
                    if ty == "slider" { (arg(args, 2), arg(args, 3), arg(args, 4)) } else { (arg(args, 2), Value::Undefined, arg(args, 3)) };
                let given = |v: &Value| !matches!(v, Value::Undefined);
                let value = if given(&value) { number_arg(i, &value, "add: the value")? } else { 0.0 };
                let min = if given(&min) { number_arg(i, &min, "add: the minimum value")? } else { 0.0 };
                let max = if given(&max) { number_arg(i, &max, "add: the maximum value")? } else { 100.0 };
                set_open(i, c, "minvalue", Value::Number(min))?;
                set_open(i, c, "maxvalue", Value::Number(max))?;
                set_open(i, c, "value", Value::Number(value))?;
            }
            "dropdownlist" | "listbox" => {
                let kept = list(i, &[], false)?;
                set_slot(i, c, ITEMS, Value::Object(kept))?;
                set_slot(i, c, SELECTION, Value::Number(-1.0))?;
                let items = list(i, &[], true)?;
                set_open(i, c, "items", Value::Object(items))?;
                let cl = classes(i)?;
                i.define(c, key("selection"), Prop::Accessor { get: Some(cl.selection_get), set: Some(cl.selection_set), flags: ENUMERABLE })?;
            }
            _ => {}
        }
        // A new list's items take their places in the dialog only once the list is added.
        let mut staged = 0usize;
        if list_kind {
            match arg(args, 2) {
                Value::Undefined | Value::Null => {}
                Value::Object(a) if matches!(i.object(a)?.class, Class::Array { .. }) => {
                    let length = match &i.object(a)?.class {
                        Class::Array { length, .. } => *length as usize,
                        _ => 0,
                    };
                    if length > MAX_ITEMS.saturating_sub(number_slot(i, win, ITEM_COUNT)? as usize) {
                        return Err(range(i, &format!("add: a dialog's lists hold at most {MAX_ITEMS} items in all")));
                    }
                    let values = list_values(i, a)?;
                    i.temp_roots.extend(values.iter().cloned());
                    for item in &values {
                        let text = text_arg(i, item)?;
                        new_item(i, c, text)?;
                        staged += 1;
                    }
                }
                _ => return Err(i.type_error("add: give a list's items as an array")),
            }
        }
        // A getter or a conversion above may have added controls or items itself.
        let count = number_slot(i, win, COUNT)?;
        if count as usize >= MAX_CONTROLS {
            return Err(range(i, &format!("add: a dialog can hold at most {MAX_CONTROLS} controls")));
        }
        let items = number_slot(i, win, ITEM_COUNT)?;
        if items as usize + staged > MAX_ITEMS {
            return Err(range(i, &format!("add: a dialog's lists hold at most {MAX_ITEMS} items in all")));
        }
        append(i, o, KIDS, "children", Value::Object(c))?;
        set_slot(i, win, COUNT, Value::Number(count + 1.0))?;
        set_slot(i, win, ITEM_COUNT, Value::Number(items + staged as f64))?;
        // The host's copy of a shown palette keeps each control's place in the list, which events
        // name it by.
        update_palette(i, win)?;
        Ok(Value::Object(c))
    })();
    i.temp_roots.truncate(mark);
    r
}

/// Adds an item with `text` to the list `o`, which the dialog holds.
fn add_item(i: &mut Interp, o: ObjId, text: &Value) -> JsResult<Value> {
    let Some(win) = object_slot(i, o, WINDOW)? else { return Err(Abrupt::Internal("a list without its window".into())) };
    // The conversion runs first: it may add or remove items itself.
    let text = text_arg(i, text)?;
    i.rooted(std::slice::from_ref(&text), |i| {
        let count = number_slot(i, win, ITEM_COUNT)?;
        if count as usize >= MAX_ITEMS {
            return Err(range(i, &format!("add: a dialog's lists hold at most {MAX_ITEMS} items in all")));
        }
        let item = new_item(i, o, text.clone())?;
        i.rooted(std::slice::from_ref(&item), |i| set_slot(i, win, ITEM_COUNT, Value::Number(count + 1.0)))?;
        Ok(item)
    })
}

/// Adds an item with `text` to the end of the list `o`, without counting it in the dialog's items.
fn new_item(i: &mut Interp, o: ObjId, text: Value) -> JsResult<Value> {
    let proto = classes(i)?.item_proto;
    i.rooted(std::slice::from_ref(&text), |i| {
        i.reserve_heap(OBJECT_BYTES.saturating_add(PROP_BYTES.saturating_mul(4)))?;
        let item = i.heap.alloc_object(Object::new(Class::Ordinary, Some(proto)));
        i.rooted(&[Value::Object(item)], |i| {
            set_slot(i, item, LIST, Value::Object(o))?;
            set_open(i, item, "text", text.clone())?;
            append(i, o, ITEMS, "items", Value::Object(item))?;
            bump_revision(i, o)?;
            Ok(Value::Object(item))
        })
    })
}

fn list_of(i: &mut Interp, this: &Value, what: &str) -> JsResult<ObjId> {
    match kind_of(i, this)? {
        Some((o, kind)) if kind == "dropdownlist" || kind == "listbox" => Ok(o),
        _ => Err(i.type_error(&format!("{what}: this is not a list"))),
    }
}

fn items_of(i: &Interp, o: ObjId) -> JsResult<Vec<Value>> {
    match object_slot(i, o, ITEMS)? {
        Some(a) => list_values(i, a),
        None => Ok(Vec::new()),
    }
}

fn selection_index(i: &Interp, o: ObjId) -> JsResult<Option<usize>> {
    let n = number_slot(i, o, SELECTION)?;
    Ok((n >= 0.0).then_some(n as usize))
}

fn selection_get(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let o = list_of(i, &this, "selection")?;
    let items = items_of(i, o)?;
    Ok(selection_index(i, o)?.and_then(|n| items.get(n).cloned()).unwrap_or(Value::Null))
}

fn selection_set(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = list_of(i, &this, "selection")?;
    let items = items_of(i, o)?;
    let v = arg(args, 0);
    let n = match &v {
        Value::Undefined | Value::Null => -1.0,
        Value::Number(n) if n.fract() == 0.0 && *n >= 0.0 && (*n as usize) < items.len() => *n,
        Value::Number(n) => return Err(range(i, &format!("selection: the list has no item {n}"))),
        v => match items.iter().position(|item| same(item, v)) {
            Some(n) => n as f64,
            None => return Err(i.type_error("selection: give an item of this list, its index or null")),
        },
    };
    set_slot(i, o, SELECTION, Value::Number(n))?;
    Ok(Value::Undefined)
}

/// The list an item belongs to and its index there.
fn item_place(i: &mut Interp, this: &Value, what: &str) -> JsResult<(ObjId, Option<usize>)> {
    let list = match this.as_object() {
        Some(o) => object_slot(i, o, LIST)?,
        None => None,
    };
    let Some(list) = list else { return Err(i.type_error(&format!("{what}: this is not a list item"))) };
    let index = items_of(i, list)?.iter().position(|item| same(item, this));
    Ok((list, index))
}

fn item_index(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (_, index) = item_place(i, &this, "index")?;
    Ok(Value::Number(index.map_or(-1.0, |n| n as f64)))
}

fn item_selected_get(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (list, index) = item_place(i, &this, "selected")?;
    Ok(Value::Bool(index.is_some() && selection_index(i, list)? == index))
}

fn item_selected_set(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (list, index) = item_place(i, &this, "selected")?;
    let Some(index) = index else { return Ok(Value::Undefined) };
    if arg(args, 0).truthy() {
        set_slot(i, list, SELECTION, Value::Number(index as f64))?;
    } else if selection_index(i, list)? == Some(index) {
        set_slot(i, list, SELECTION, Value::Number(-1.0))?;
    }
    Ok(Value::Undefined)
}

fn item_to_string(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    item_place(i, &this, "toString")?;
    let text = i.get_value(&this, &key("text"))?;
    Ok(Value::String(i.to_str(&text)?))
}

/// `list.removeAll()`.
fn remove_all(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let o = list_of(i, &this, "removeAll")?;
    let n = items_of(i, o)?.len() as f64;
    let Some(win) = object_slot(i, o, WINDOW)? else { return Err(Abrupt::Internal("a list without its window".into())) };
    let count = number_slot(i, win, ITEM_COUNT)?;
    set_slot(i, win, ITEM_COUNT, Value::Number((count - n).max(0.0)))?;
    let kept = list(i, &[], false)?;
    set_slot(i, o, ITEMS, Value::Object(kept))?;
    let items = list(i, &[], true)?;
    set_open(i, o, "items", Value::Object(items))?;
    set_slot(i, o, SELECTION, Value::Number(-1.0))?;
    bump_revision(i, o)?;
    Ok(Value::Undefined)
}

/// Counts a change of the list `o`'s items ([`REVISION`]).
fn bump_revision(i: &mut Interp, o: ObjId) -> JsResult<()> {
    let r = number_slot(i, o, REVISION)?;
    set_slot(i, o, REVISION, Value::Number(r + 1.0))
}

fn kids_of(i: &Interp, o: ObjId) -> JsResult<Vec<Value>> {
    match object_slot(i, o, KIDS)? {
        Some(a) => list_values(i, a),
        None => Ok(Vec::new()),
    }
}

/// `findElement(name)`: the first control in the element, in order, whose `name` is `name`.
fn find_element(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = element(i, &this, "findElement")?;
    let want = i.to_str(&arg(args, 0))?;
    let mut stack: Vec<Value> = kids_of(i, o)?.into_iter().rev().collect();
    while let Some(v) = stack.pop() {
        i.use_fuel()?;
        let Some(c) = v.as_object() else { continue };
        let name = i.get_value(&v, &key("name"))?;
        if matches!(&name, Value::String(s) if *s == want) {
            return Ok(v);
        }
        stack.extend(kids_of(i, c)?.into_iter().rev());
    }
    Ok(Value::Null)
}

/// Calls the element's handler `name` when it has one. → its result, or `None` without one.
fn handler(i: &mut Interp, o: ObjId, name: &str) -> JsResult<Option<Value>> {
    let f = i.get_value(&Value::Object(o), &key(name))?;
    if !i.is_callable(&f) {
        return Ok(None);
    }
    i.call(&f, &Value::Object(o), &[]).map(Some)
}

/// `notify([event])`: runs the control's `onClick`, `onChange` or `onChanging`.
fn notify(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, kind) = element(i, &this, "notify")?;
    let event = match arg(args, 0) {
        Value::Undefined if matches!(kind.as_str(), "button" | "checkbox" | "radiobutton") => "onClick".to_string(),
        Value::Undefined => "onChange".to_string(),
        v => i.to_str(&v)?.to_rust_string(),
    };
    if !matches!(event.as_str(), "onClick" | "onChange" | "onChanging") {
        return Err(error(i, "notify: give onClick, onChange or onChanging"));
    }
    handler(i, o, &event)?;
    Ok(Value::Undefined)
}

fn control_show(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = element(i, &this, "show")?;
    i.put(o, key("visible"), Value::Bool(true), true)?;
    Ok(Value::Undefined)
}

fn control_hide(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = element(i, &this, "hide")?;
    i.put(o, key("visible"), Value::Bool(false), true)?;
    Ok(Value::Undefined)
}

fn nothing(_: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
    Ok(Value::Undefined)
}

/// `this` as a window, and whether it is a palette.
fn window_of(i: &mut Interp, this: &Value, what: &str) -> JsResult<(ObjId, bool)> {
    match kind_of(i, this)? {
        Some((o, kind)) if is_window(&kind) => Ok((o, kind == "palette")),
        _ => Err(i.type_error(&format!("{what}: this is not a ScriptUI window"))),
    }
}

/// `close([result])`: closes the dialog that is showing once the running handler returns, or the
/// palette at once.
fn close(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, palette) = window_of(i, &this, "close")?;
    let result = match arg(args, 0) {
        Value::Undefined => 0.0,
        v => number_arg(i, &v, "close: the result")?,
    };
    if palette {
        close_palette(i, o)?;
    } else if classes(i)?.showing == Some(o) && !slot(i, o, IN_CLOSE)?.truthy() {
        set_slot(i, o, CLOSING, Value::Number(result))?;
    }
    Ok(Value::Undefined)
}

/// `hide()`: closes the dialog that is showing with the result 0, or hides the palette, which a
/// later `show()` shows again as it was. A palette's `visible` is false once the host hid it, and
/// true again once the host shows it.
fn hide(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, palette) = window_of(i, &this, "hide")?;
    if !palette {
        return close(i, this, &[], false);
    }
    if let Value::Number(id) = slot(i, o, PALETTE_ID)? {
        let f = host_fn(i, "paletteHide")?;
        i.call(&f, &Value::Undefined, &[Value::Number(id)])?;
        set_slot(i, o, SHOWN, Value::Bool(false))?;
        i.put(o, key("visible"), Value::Bool(false), true)?;
    }
    Ok(Value::Undefined)
}

/// `update()` on a window: [`update_window`].
fn update(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let own = own_code(i);
    let (o, palette) = window_of(i, &this, "update")?;
    update_window(i, o, palette, own)?;
    Ok(Value::Undefined)
}

/// Whether the native running now was called from the script's own code in a run that can pause,
/// as the host's functions find whether a run can wait.
fn own_code(i: &Interp) -> bool {
    i.in_slice && i.reentry == 0
}

/// `update()` on a `group` or a `panel`: [`update_window`] for the window that holds it.
fn control_update(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let own = own_code(i);
    let (o, kind) = element(i, &this, "update")?;
    if !is_container(&kind) {
        return Err(i.type_error("update: this is not a ScriptUI window or container"));
    }
    let Some(win) = object_slot(i, o, WINDOW)? else { return Err(Abrupt::Internal("a container without its window".into())) };
    let palette = matches!(slot(i, win, KIND)?, Value::String(s) if s.to_rust_string() == "palette");
    update_window(i, win, palette, own)?;
    Ok(Value::Undefined)
}

/// `update()` on the window `win`. On a shown palette: hands each event that waits for a palette of
/// the run's engine to its palette ([`take_waiting`]), and then, from the script's own code (`own`),
/// gives the host the palette's controls when the host draws it now (`paletteDraw`). The showing
/// dialog's controls go to the host when its handler returns, since a handler runs to its end in
/// one slice. Whether a palette shows is the host's, not its `visible` property. A window that is
/// not showing throws.
fn update_window(i: &mut Interp, win: ObjId, palette: bool, own: bool) -> JsResult<()> {
    let showing = if palette { slot(i, win, SHOWN)?.truthy() } else { classes(i)?.showing == Some(win) };
    if !showing {
        return Err(error(i, "update: the window is not showing"));
    }
    if !palette {
        return Ok(());
    }
    take_waiting(i)?;
    if !own || !slot(i, win, SHOWN)?.truthy() {
        return Ok(());
    }
    let f = host_fn(i, "paletteDraw")?;
    if i.call(&f, &Value::Undefined, &[])?.truthy() {
        update_palette(i, win)?;
    }
    Ok(())
}

/// Hands each event the host's `paletteTake` gives, oldest first, to its palette, until none waits
/// (`palettePending`). Before each one the host gets every palette's controls as the script and the
/// handlers before it left them, as at the end of a run, and drops an event whose control has since
/// been disabled, hidden or removed. An `update()` that a getter or a handler calls meanwhile takes
/// no events: the `update()` already taking them goes on with the next one.
fn take_waiting(i: &mut Interp) -> JsResult<()> {
    if i.script_ui.as_ref().is_some_and(|c| c.taking) {
        return Ok(());
    }
    set_taking(i, true);
    let r = take_each(i);
    set_taking(i, false);
    r
}

fn set_taking(i: &mut Interp, taking: bool) {
    if let Some(c) = i.script_ui.as_mut() {
        c.taking = taking;
    }
}

/// [`take_waiting`]'s loop.
fn take_each(i: &mut Interp) -> JsResult<()> {
    let pending = host_fn(i, "palettePending")?;
    let f = host_fn(i, "paletteTake")?;
    loop {
        if !i.call(&pending, &Value::Undefined, &[])?.truthy() {
            return Ok(());
        }
        update_palettes(i)?;
        let answer = i.call(&f, &Value::Undefined, &[])?;
        if answer.as_object().is_none() {
            return Ok(());
        }
        deliver(i, answer)?;
    }
}

fn set_palettes(i: &mut Interp, f: impl FnOnce(&mut Vec<ObjId>)) {
    if let Some(c) = i.script_ui.as_mut() {
        f(&mut c.palettes);
    }
}

/// Closes the palette `win` once its `onClose` allows it: the host no longer shows it and its
/// handlers no longer run. A `close()` while its `onClose` runs does nothing more.
fn close_palette(i: &mut Interp, win: ObjId) -> JsResult<()> {
    if slot(i, win, IN_CLOSE)?.truthy() {
        return Ok(());
    }
    if !may_close(i, win)? {
        return Ok(());
    }
    if let Value::Number(id) = slot(i, win, PALETTE_ID)? {
        set_slot(i, win, PALETTE_ID, Value::Undefined)?;
        set_slot(i, win, SHOWN, Value::Bool(false))?;
        set_palettes(i, |p| p.retain(|o| *o != win));
        let f = host_fn(i, "paletteClose")?;
        i.call(&f, &Value::Undefined, &[Value::Number(id)])?;
    }
    Ok(())
}

/// Gives the host the controls of the palette `win` as they are now, when it shows the palette.
fn update_palette(i: &mut Interp, win: ObjId) -> JsResult<()> {
    let Value::Number(id) = slot(i, win, PALETTE_ID)? else { return Ok(()) };
    let (desc, ..) = describe(i, win)?;
    let f = host_fn(i, "paletteUpdate")?;
    i.rooted(std::slice::from_ref(&desc), |i| i.call(&f, &Value::Undefined, &[desc.clone(), Value::Number(id)]))?;
    Ok(())
}

/// Gives the host the controls of every palette it shows, after a handler may have changed them.
fn update_palettes(i: &mut Interp) -> JsResult<()> {
    // A getter may close a later palette, which removes it from the rooted list, so the copy is
    // rooted until the loop ends.
    let palettes = classes(i)?.palettes;
    let roots: Vec<Value> = palettes.iter().map(|&win| Value::Object(win)).collect();
    i.rooted(&roots, |i| {
        for win in palettes {
            update_palette(i, win)?;
        }
        Ok(())
    })
}

/// Gives the host the controls of every palette the realm keeps, as a run left them
/// ([`crate::Realm::refresh_script_ui`]); nothing without palettes.
pub(crate) fn refresh(i: &mut Interp) -> JsResult<()> {
    if i.script_ui.as_ref().is_none_or(|c| c.palettes.is_empty()) {
        return Ok(());
    }
    update_palettes(i)
}

/// A palette's `show()`: the host shows its description at once, then `onShow` runs, and `show()`
/// returns `undefined` without waiting.
fn show_palette(i: &mut Interp, win: ObjId) -> JsResult<Pause> {
    let (desc, ..) = describe(i, win)?;
    let id = slot(i, win, PALETTE_ID)?;
    let f = host_fn(i, "paletteShow")?;
    Ok(Pause { host: f, args: vec![desc, id], keep: vec![Value::Object(win)], resume: palette_shown, failed: None })
}

fn palette_shown(i: &mut Interp, keep: &[Value], id: Value) -> JsResult<Value> {
    let Some(win) = arg(keep, 0).as_object() else { return Err(Abrupt::Internal("show lost its palette".into())) };
    let Value::Number(_) = id else { return Err(Abrupt::Internal("the host gave the palette no id".into())) };
    set_slot(i, win, PALETTE_ID, id)?;
    if !classes(i)?.palettes.contains(&win) {
        set_palettes(i, |p| p.push(win));
    }
    set_slot(i, win, SHOWN, Value::Bool(true))?;
    i.put(win, key("visible"), Value::Bool(true), true)?;
    handler(i, win, "onShow")?;
    update_palette(i, win)?;
    Ok(Value::Undefined)
}

/// `Window.__vcPaletteEvent()`, which the run of a palette's handler calls: takes the event from the
/// host's `paletteEvent` and gives it to the palette the way a dialog takes the person's event. The
/// host gets every palette's controls once the run ends ([`crate::Realm::refresh_script_ui`]). An
/// event for a palette that has closed does nothing.
fn palette_event(i: &mut Interp, _: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let f = host_fn(i, "paletteEvent")?;
    let answer = i.call(&f, &Value::Undefined, &[])?;
    deliver(i, answer)?;
    Ok(Value::Undefined)
}

/// Gives the host's event `answer` (`{palette, event, id, value, place}`, or anything else for
/// none) to its palette the way a dialog takes the person's event. An event for a palette that has
/// closed does nothing.
fn deliver(i: &mut Interp, answer: Value) -> JsResult<Value> {
    let Some(ev) = answer.as_object() else { return Ok(Value::Undefined) };
    let mark = i.temp_roots.len();
    i.temp_roots.push(answer.clone());
    let r = (|| {
        let id = slot(i, ev, "palette")?;
        let palettes = classes(i)?.palettes;
        let mut win = None;
        for p in palettes {
            if matches!((slot(i, p, PALETTE_ID)?, &id), (Value::Number(a), Value::Number(b)) if a == *b) {
                win = Some(p);
            }
        }
        let Some(win) = win else { return Ok(Value::Undefined) };
        let event = match slot(i, ev, "event")? {
            Value::String(s) => s.to_rust_string(),
            _ => String::new(),
        };
        if event == "close" {
            close_palette(i, win)?;
        } else {
            let (_, nodes, ..) = describe(i, win)?;
            // A getter read in the description may have closed the palette.
            if !still_takes(i, win, ev, None)? {
                return Ok(Value::Undefined);
            }
            i.temp_roots.push(Value::Object(nodes));
            let held = list_values(i, nodes)?;
            // The host gives the control's place, which is found again in the palette as it is now:
            // a getter read in the description may have added a control before it.
            let at = match slot(i, ev, "place")? {
                Value::Object(place) => {
                    let control = control_at_place(i, win, place)?;
                    control.and_then(|c| held.iter().position(|v| v.as_object() == Some(c)))
                }
                _ => match slot(i, ev, "id")? {
                    Value::Number(n) if n >= 1.0 && n.fract() == 0.0 => Some(n as usize),
                    _ => None,
                },
            };
            let Some(at) = at.filter(|&n| n >= 1 && n < held.len()) else {
                return Err(error(i, "a palette event names a control the palette no longer holds"));
            };
            set_slot(i, ev, "id", Value::Number(at as f64))?;
            take_event(i, win, nodes, ev, &event, None)?;
        }
        Ok(Value::Undefined)
    })();
    i.temp_roots.truncate(mark);
    r
}

/// Whether the window `win` still takes the event `ev`: a dialog (`keep` is `Some`) always does,
/// and a palette only while the host shows it under the id the event names. A getter or a handler
/// run for the event may have closed the palette.
fn still_takes(i: &mut Interp, win: ObjId, ev: ObjId, keep: Option<&[Value]>) -> JsResult<bool> {
    if keep.is_some() {
        return Ok(true);
    }
    let id = slot(i, ev, "palette")?;
    Ok(matches!((slot(i, win, PALETTE_ID)?, id), (Value::Number(a), Value::Number(b)) if a == b))
}

/// Calls control `c`'s handler `name` for the event `ev` when it has one and `win` still takes the
/// event once the handler has been read: a getter for `name` may have closed the palette.
/// → `None` when `win` no longer takes the event, otherwise `handler`'s answer.
fn event_handler(i: &mut Interp, win: ObjId, c: ObjId, ev: ObjId, keep: Option<&[Value]>, name: &str) -> JsResult<Option<Option<Value>>> {
    if !still_takes(i, win, ev, keep)? {
        return Ok(None);
    }
    let f = i.get_value(&Value::Object(c), &key(name))?;
    if !still_takes(i, win, ev, keep)? {
        return Ok(None);
    }
    if !i.is_callable(&f) {
        return Ok(Some(None));
    }
    i.call(&f, &Value::Object(c), &[]).map(|v| Some(Some(v)))
}

/// The control at `place` in `win` as it is now: the index of each control among its container's
/// controls, from the window down, as the host counts places. `None` when the palette has no
/// control there.
fn control_at_place(i: &Interp, win: ObjId, place: ObjId) -> JsResult<Option<ObjId>> {
    let mut at = win;
    for k in list_values(i, place)? {
        let Value::Number(k) = k else { return Ok(None) };
        if k < 0.0 || k.fract() != 0.0 {
            return Ok(None);
        }
        let Some(kid) = kids_of(i, at)?.get(k as usize).and_then(Value::as_object) else { return Ok(None) };
        at = kid;
    }
    Ok((at != win).then_some(at))
}

fn host_fn(i: &mut Interp, name: &str) -> JsResult<Value> {
    let host = classes(i)?.host;
    i.get_value(&Value::Object(host), &key(name))
}

fn set_showing(i: &mut Interp, showing: Option<ObjId>) {
    if let Some(c) = i.script_ui.as_mut() {
        c.showing = showing;
    }
}

/// `show()` on a dialog: gives its description to the host's `open`, then runs it ([`opened`]). On a
/// palette: [`show_palette`].
fn show(i: &mut Interp, this: Value, _: &[Value]) -> JsResult<Pause> {
    let (win, palette) = window_of(i, &this, "show")?;
    if palette {
        return show_palette(i, win);
    }
    match classes(i)?.showing {
        Some(o) if o == win => return Err(error(i, "show: this dialog is already showing")),
        Some(_) => return Err(error(i, "show: a dialog cannot open while another dialog is showing")),
        None => {}
    }
    set_showing(i, Some(win));
    set_slot(i, win, CLOSING, Value::Undefined)?;
    let described = describe(i, win);
    let (desc, nodes, def, cancel, lists) = match described {
        Ok(d) => d,
        Err(e) => {
            set_showing(i, None);
            return Err(e);
        }
    };
    let open = host_fn(i, "open")?;
    Ok(Pause {
        host: open,
        args: vec![desc],
        keep: vec![Value::Object(win), Value::Object(nodes), Value::Number(def), Value::Number(cancel), Value::Object(lists)],
        resume: opened,
        failed: Some(failed),
    })
}

/// A host call of `show()` failed, or a handler threw: the dialog is no longer showing.
fn failed(i: &mut Interp, keep: &[Value]) {
    let Some(win) = arg(keep, 0).as_object() else { return };
    if classes(i).ok().and_then(|c| c.showing) != Some(win) {
        return;
    }
    set_showing(i, None);
    if let Ok(end) = host_fn(i, "end") {
        let _ = i.call(&end, &Value::Undefined, &[Value::Number(0.0), Value::Bool(false)]);
    }
}

/// `show()` ends with `result`.
fn finish(i: &mut Interp, win: ObjId, result: f64) -> JsResult<Value> {
    set_showing(i, None);
    set_slot(i, win, CLOSING, Value::Undefined)?;
    let end = host_fn(i, "end")?;
    i.call(&end, &Value::Undefined, &[Value::Number(result), Value::Bool(true)])?;
    Ok(Value::Number(result))
}

/// Runs `onClose`, during which a `close()` of the same window does nothing. → false when it
/// returned `false`.
fn may_close(i: &mut Interp, win: ObjId) -> JsResult<bool> {
    set_slot(i, win, IN_CLOSE, Value::Bool(true))?;
    let ran = handler(i, win, "onClose");
    set_slot(i, win, IN_CLOSE, Value::Undefined)?;
    Ok(!matches!(ran?, Some(Value::Bool(false))))
}

/// Closes the dialog when a handler asked to, or asks the host's `wait` what happens next.
fn next(i: &mut Interp, win: ObjId) -> JsResult<Value> {
    loop {
        i.use_fuel()?;
        if let Value::Number(result) = slot(i, win, CLOSING)? {
            set_slot(i, win, CLOSING, Value::Undefined)?;
            if may_close(i, win)? {
                return finish(i, win, result);
            }
            continue;
        }
        let (desc, nodes, def, cancel, lists) = describe(i, win)?;
        let wait = host_fn(i, "wait")?;
        i.again = Some(Pause {
            host: wait,
            args: vec![desc],
            keep: vec![Value::Object(win), Value::Object(nodes), Value::Number(def), Value::Number(cancel), Value::Object(lists)],
            resume: answered,
            failed: Some(failed),
        });
        return Ok(Value::Undefined);
    }
}

/// Runs `r`; when it fails, the dialog is no longer showing.
fn guarded(i: &mut Interp, keep: &[Value], r: impl FnOnce(&mut Interp) -> JsResult<Value>) -> JsResult<Value> {
    let v = r(i);
    if v.is_err() {
        failed(i, keep);
    }
    v
}

fn opened(i: &mut Interp, keep: &[Value], _: Value) -> JsResult<Value> {
    guarded(i, keep, |i| {
        let Some(win) = arg(keep, 0).as_object() else { return Err(Abrupt::Internal("show lost its dialog".into())) };
        handler(i, win, "onShow")?;
        next(i, win)
    })
}

/// What the host's `wait` gave: the event, which the dialog takes, and what happens next.
fn answered(i: &mut Interp, keep: &[Value], answer: Value) -> JsResult<Value> {
    guarded(i, keep, |i| {
        let (Some(win), Some(nodes)) = (arg(keep, 0).as_object(), arg(keep, 1).as_object()) else {
            return Err(Abrupt::Internal("show lost its dialog".into()));
        };
        let Some(ev) = answer.as_object() else { return Err(Abrupt::Internal("the host gave show no event".into())) };
        let event = match slot(i, ev, "event")? {
            Value::String(s) => s.to_rust_string(),
            _ => String::new(),
        };
        if event == "close" {
            if may_close(i, win)? {
                return finish(i, win, 0.0);
            }
            return next(i, win);
        }
        take_event(i, win, nodes, ev, &event, Some(keep))?;
        next(i, win)
    })
}

/// A click or a change the host gave in `ev` for control `id` of `nodes` (a description's list of
/// elements): the control takes it and its handler runs. A dialog's button without `onClick` asks
/// to close it with 1 when it is the default element and 2 when it is the cancel element, which
/// `keep` (`show()`'s kept values) names; a palette has no `keep`. No handler of a palette runs
/// once a getter, a setter or an earlier handler of the event closed the palette.
fn take_event(i: &mut Interp, win: ObjId, nodes: ObjId, ev: ObjId, event: &str, keep: Option<&[Value]>) -> JsResult<()> {
    let id = match slot(i, ev, "id")? {
        Value::Number(n) if n >= 0.0 && n.fract() == 0.0 => n as usize,
        _ => return Err(Abrupt::Internal("the host gave show an event without a control".into())),
    };
    let nodes = list_values(i, nodes)?;
    let Some((c, kind)) = nodes.get(id).map(|v| kind_of(i, v)).transpose()?.flatten() else {
        return Err(Abrupt::Internal("the host gave show an event for a control the dialog does not hold".into()));
    };
    let value = slot(i, ev, "value")?;
    match event {
        "click" => {
            if kind == "checkbox" {
                let on = i.get_value(&Value::Object(c), &key("value"))?.truthy();
                i.put(c, key("value"), Value::Bool(!on), true)?;
            } else if kind == "radiobutton" {
                let parent = object_slot(i, c, PARENT)?;
                for other in parent.map(|p| kids_of(i, p)).transpose()?.unwrap_or_default() {
                    if !same(&other, &Value::Object(c))
                        && let Some((o, k)) = kind_of(i, &other)?
                        && k == "radiobutton"
                    {
                        i.put(o, key("value"), Value::Bool(false), true)?;
                    }
                }
                i.put(c, key("value"), Value::Bool(true), true)?;
            }
            let Some(ran) = event_handler(i, win, c, ev, keep, "onClick")? else {
                return Ok(());
            };
            if ran.is_none()
                && kind == "button"
                && let Some(keep) = keep
            {
                let is = |k: usize| matches!(arg(keep, k), Value::Number(n) if n == id as f64);
                if is(2) {
                    set_slot(i, win, CLOSING, Value::Number(1.0))?;
                } else if is(3) {
                    set_slot(i, win, CLOSING, Value::Number(2.0))?;
                }
            }
        }
        "change" => {
            match kind.as_str() {
                "edittext" => {
                    i.put(c, key("text"), value, true)?;
                    if event_handler(i, win, c, ev, keep, "onChanging")?.is_none() {
                        return Ok(());
                    }
                }
                "slider" => {
                    i.put(c, key("value"), value, true)?;
                    if event_handler(i, win, c, ev, keep, "onChanging")?.is_none() {
                        return Ok(());
                    }
                }
                _ => {
                    let n = match value {
                        Value::Number(n) => n,
                        _ => -1.0,
                    };
                    match keep {
                        // The index is into the items the person saw.
                        Some(keep) => {
                            let shown = match arg(keep, 4).as_object() {
                                Some(lists) => list_values(i, lists)?.get(id).cloned().unwrap_or(Value::Undefined),
                                None => Value::Undefined,
                            };
                            if !same(&slot(i, c, ITEMS)?, &shown) {
                                return Err(error(i, "show: a list's items changed before the person's choice was taken"));
                            }
                        }
                        // A palette's list may have changed while the event waited: the host gives
                        // the items the person saw and the list's revision then.
                        // The revision is read after the texts, whose getters may change the list.
                        None => {
                            let same = same_items(i, c, &slot(i, ev, "items")?)?;
                            let revised = match slot(i, ev, "revision")? {
                                Value::Number(r) => r != number_slot(i, c, REVISION)?,
                                _ => false,
                            };
                            // An item's text getter may have closed the palette, and its `onClose`
                            // may have changed the list: the event is then dropped, not refused.
                            if !still_takes(i, win, ev, keep)? {
                                return Ok(());
                            }
                            if revised || !same || (n >= 0.0 && n as usize >= items_of(i, c)?.len()) {
                                return Err(error(i, "a palette's list changed before the person's choice was taken"));
                            }
                        }
                    }
                    set_slot(i, c, SELECTION, Value::Number(n))?;
                }
            }
            // `onChanging` may have closed the palette.
            event_handler(i, win, c, ev, keep, "onChange")?;
        }
        _ => return Err(Abrupt::Internal(format!("the host gave show the event {event}"))),
    }
    Ok(())
}

/// Does the list `c` hold items with the texts in `seen` (an array of strings), in that order? True
/// when `seen` is not an array.
fn same_items(i: &mut Interp, c: ObjId, seen: &Value) -> JsResult<bool> {
    let Some(seen) = seen.as_object() else { return Ok(true) };
    let seen = list_values(i, seen)?;
    let items = items_of(i, c)?;
    if seen.len() != items.len() {
        return Ok(false);
    }
    i.rooted(&items, |i| {
        for (item, want) in items.iter().zip(&seen) {
            let Some(item) = item.as_object() else { return Ok(false) };
            let text = get(i, item, "text")?;
            if i.to_str(&text)?.to_rust_string() != i.to_str(want)?.to_rust_string() {
                return Ok(false);
            }
        }
        Ok(true)
    })
}

fn text_of(i: &mut Interp, v: &Value, what: &str) -> JsResult<String> {
    let s = i.to_str(v)?;
    let text = if s.len() > MAX_TEXT_BYTES { None } else { Some(s.to_rust_string()) };
    match text {
        Some(t) if t.len() <= MAX_TEXT_BYTES => Ok(t),
        _ => Err(range(i, &format!("show: {what} is longer than 16 KiB"))),
    }
}

fn get(i: &mut Interp, o: ObjId, name: &str) -> JsResult<Value> {
    i.get_value(&Value::Object(o), &key(name))
}

/// [`get`], keeping the value in the roots [`describe`] lets go of when it returns.
fn get_kept(i: &mut Interp, o: ObjId, name: &str) -> JsResult<Value> {
    let v = get(i, o, name)?;
    i.temp_roots.push(v.clone());
    Ok(v)
}

fn number_of(i: &mut Interp, o: ObjId, name: &str) -> JsResult<Data> {
    let v = get(i, o, name)?;
    Ok(Data::Number(number_arg(i, &v, &format!("show: a control's {name}"))?))
}

/// An alignment: a name, or an array of a horizontal and a vertical name; `null` when not set.
fn alignment(i: &mut Interp, v: &Value) -> JsResult<Data> {
    match v {
        Value::Undefined | Value::Null => Ok(Data::Null),
        Value::Object(a) if matches!(i.object(*a)?.class, Class::Array { .. }) => {
            let mut out = Vec::new();
            for n in 0..2 {
                let part = i.get_value(v, &index_key(n))?;
                out.push(Data::String(text_of(i, &part, "an alignment")?.to_lowercase()));
            }
            Ok(Data::Array(out))
        }
        v => Ok(Data::String(text_of(i, v, "an alignment")?.to_lowercase())),
    }
}

/// Numbers read from `v`'s indices, or else from its properties `names`.
fn numbers(i: &mut Interp, v: &Value, names: &[&str], what: &str) -> JsResult<Data> {
    let Some(o) = v.as_object() else {
        if matches!(v, Value::Undefined | Value::Null) {
            return Ok(Data::Null);
        }
        return Err(i.type_error(&format!("show: give {what} as an array or an object")));
    };
    let array = matches!(i.object(o)?.class, Class::Array { .. });
    let mut out = Vec::new();
    for (n, name) in names.iter().enumerate() {
        let part = if array { i.get_value(v, &index_key(n as u32))? } else { i.get_value(v, &key(name))? };
        out.push(Data::Number(number_arg(i, &part, &format!("show: {what}"))?));
    }
    Ok(Data::Array(out))
}

/// Bounds as `[left, top, right, bottom]`: from an array, from `x`, `y`, `width` and `height`, or
/// from `left`, `top`, `right` and `bottom`.
fn bounds(i: &mut Interp, v: &Value) -> JsResult<Data> {
    if let Some(o) = v.as_object()
        && !matches!(i.object(o)?.class, Class::Array { .. })
        && !matches!(i.get_value(v, &key("width"))?, Value::Undefined)
    {
        let Data::Array(xywh) = numbers(i, v, &["x", "y", "width", "height"], "bounds")? else { return Ok(Data::Null) };
        let n = |k: usize| match xywh.get(k) {
            Some(Data::Number(n)) => *n,
            _ => 0.0,
        };
        // Two finite numbers can add up to infinity; the sum is kept finite.
        let end = |a: f64, b: f64| (a + b).clamp(-f64::MAX, f64::MAX);
        return Ok(Data::Array(vec![Data::Number(n(0)), Data::Number(n(1)), Data::Number(end(n(0), n(2))), Data::Number(end(n(1), n(3)))]));
    }
    numbers(i, v, &["left", "top", "right", "bottom"], "bounds")
}

fn margins(i: &mut Interp, v: &Value) -> JsResult<Data> {
    if v.as_object().is_some() {
        return numbers(i, v, &["left", "top", "right", "bottom"], "margins");
    }
    let m = number_arg(i, v, "show: margins")?;
    Ok(Data::Array(vec![Data::Number(m); 4]))
}

/// The dialog's description: its elements in order, each after the container that holds it, as
/// [`Data`] for the host's `open` and `wait`, with the elements in that order, and the indices of
/// its default and cancel elements (-1 for none), and each element's array of list items as it
/// was described (`undefined` for an element that is not a list). When a getter changes a list
/// after it was read, the dialog is read again once; a list that changes again throws an `Error`.
fn describe(i: &mut Interp, win: ObjId) -> JsResult<Described> {
    for _ in 0..2 {
        let mark = i.temp_roots.len();
        let r = describe_rooted(i, win);
        i.temp_roots.truncate(mark);
        if let Some(d) = r? {
            return Ok(d);
        }
    }
    Err(error(i, "show: a list's items changed while the dialog was read"))
}

/// A description, the elements in its order, the default and cancel elements, and the lists' items.
type Described = (Value, ObjId, f64, f64, ObjId);

/// Has each list in `order` the array of items it had when it was described?
fn lists_unchanged(i: &Interp, order: &[(ObjId, String, f64)], snapshot: &[Value]) -> JsResult<bool> {
    for ((o, ..), kept) in order.iter().zip(snapshot) {
        if !matches!(kept, Value::Undefined) && !same(&slot(i, *o, ITEMS)?, kept) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn describe_rooted(i: &mut Interp, win: ObjId) -> JsResult<Option<Described>> {
    let mut order: Vec<(ObjId, String, f64)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack: Vec<(Value, f64, usize)> = vec![(Value::Object(win), -1.0, 0)];
    while let Some((v, parent, depth)) = stack.pop() {
        i.use_fuel()?;
        let Some((o, kind)) = kind_of(i, &v)? else { return Err(i.type_error("show: the dialog holds something that is not a control")) };
        if !seen.insert(o) {
            return Err(i.type_error("show: a control appears twice in the dialog"));
        }
        if order.len() > MAX_CONTROLS {
            return Err(range(i, &format!("show: a dialog can hold at most {MAX_CONTROLS} controls")));
        }
        if depth > MAX_DEPTH {
            return Err(range(i, &format!("show: controls can be nested at most {MAX_DEPTH} deep")));
        }
        let index = order.len() as f64;
        order.push((o, kind, parent));
        for kid in kids_of(i, o)?.into_iter().rev() {
            stack.push((kid, index, depth + 1));
        }
    }
    let objects: Vec<Value> = order.iter().map(|(o, ..)| Value::Object(*o)).collect();
    i.temp_roots.extend(objects.iter().cloned());
    let mut nodes = Vec::with_capacity(order.len());
    let mut names = Vec::with_capacity(order.len());
    let mut items_total = 0usize;
    // Each list's array of items as it is read; a change to it means a later getter changed the list.
    let mut snapshot = Vec::with_capacity(order.len());
    for (o, kind, parent) in &order {
        let o = *o;
        let mut listed = Value::Undefined;
        let name_v = get(i, o, "name")?;
        let name = text_of(i, &name_v, "a control's name")?;
        let text_v = get(i, o, "text")?;
        let text = text_of(i, &text_v, "a control's text")?;
        // A palette is described as a dialog is; the host knows which it shows.
        let shown_kind = if kind == "palette" { "dialog" } else { kind.as_str() };
        let mut d: Vec<(String, Data)> = vec![
            ("kind".into(), Data::String(shown_kind.to_string())),
            ("parent".into(), Data::Number(*parent)),
            ("name".into(), Data::String(name.clone())),
            ("text".into(), Data::String(text.clone())),
        ];
        names.push((kind.clone(), name, text));
        let enabled = get(i, o, "enabled")?.truthy();
        let visible = get(i, o, "visible")?.truthy();
        d.push(("enabled".into(), Data::Bool(enabled)));
        d.push(("visible".into(), Data::Bool(visible)));
        // A value a getter gives is kept from collection while its own properties are read.
        let b = get_kept(i, o, "bounds")?;
        d.push(("bounds".into(), bounds(i, &b)?));
        let size = get_kept(i, o, "preferredSize")?;
        d.push(("size".into(), numbers(i, &size, &["width", "height"], "preferredSize")?));
        let a = get_kept(i, o, "alignment")?;
        d.push(("alignment".into(), alignment(i, &a)?));
        if is_container(kind) {
            let orientation = get(i, o, "orientation")?;
            d.push(("orientation".into(), Data::String(text_of(i, &orientation, "an orientation")?.to_lowercase())));
            let ac = get_kept(i, o, "alignChildren")?;
            d.push(("alignChildren".into(), alignment(i, &ac)?));
            d.push(("spacing".into(), number_of(i, o, "spacing")?));
            let m = get_kept(i, o, "margins")?;
            d.push(("margins".into(), margins(i, &m)?));
        }
        match kind.as_str() {
            "checkbox" | "radiobutton" => {
                let on = get(i, o, "value")?.truthy();
                d.push(("value".into(), Data::Bool(on)));
            }
            "slider" | "progressbar" => {
                d.push(("value".into(), number_of(i, o, "value")?));
                d.push(("minvalue".into(), number_of(i, o, "minvalue")?));
                d.push(("maxvalue".into(), number_of(i, o, "maxvalue")?));
            }
            "edittext" => {
                let props = get_kept(i, o, "properties")?;
                let (multiline, readonly) = match props {
                    Value::Object(_) => (i.get_value(&props, &key("multiline"))?.truthy(), i.get_value(&props, &key("readonly"))?.truthy()),
                    _ => (false, false),
                };
                d.push(("multiline".into(), Data::Bool(multiline)));
                d.push(("readonly".into(), Data::Bool(readonly)));
            }
            "dropdownlist" | "listbox" => {
                let kept = slot(i, o, ITEMS)?;
                i.temp_roots.push(kept.clone());
                listed = kept;
                let items = items_of(i, o)?;
                items_total = items_total.saturating_add(items.len());
                if items_total > MAX_ITEMS {
                    return Err(range(i, &format!("show: a dialog's lists hold at most {MAX_ITEMS} items in all")));
                }
                let mut texts = Vec::with_capacity(items.len());
                for item in &items {
                    let t = i.get_value(item, &key("text"))?;
                    texts.push(Data::String(text_of(i, &t, "a list item's text")?));
                }
                d.push(("items".into(), Data::Array(texts)));
                let sel = selection_index(i, o)?.filter(|n| *n < items.len());
                d.push(("selection".into(), sel.map_or(Data::Null, |n| Data::Number(n as f64))));
                d.push(("revision".into(), Data::Number(number_slot(i, o, REVISION)?)));
            }
            _ => {}
        }
        nodes.push(Data::Object(d));
        snapshot.push(listed);
    }
    let target = |i: &mut Interp, name: &str, word: &str| -> JsResult<f64> {
        let given = get(i, win, name)?;
        if let Some(n) = objects.iter().position(|o| same(o, &given)) {
            return Ok(n as f64);
        }
        if !matches!(given, Value::Undefined | Value::Null) {
            return Ok(-1.0);
        }
        let button = |by_name: bool| names.iter().position(|(k, n, t)| k == "button" && (if by_name { n } else { t }).to_lowercase() == word);
        Ok(button(true).or_else(|| button(false)).map_or(-1.0, |n| n as f64))
    };
    let def = target(i, "defaultElement", "ok")?;
    let cancel = target(i, "cancelElement", "cancel")?;
    if !lists_unchanged(i, &order, &snapshot)? {
        return Ok(None);
    }
    let desc = Data::Object(vec![
        ("nodes".into(), Data::Array(nodes)),
        ("defaultElement".into(), Data::Number(def)),
        ("cancelElement".into(), Data::Number(cancel)),
    ]);
    let desc = i.write_data(desc, 0, true)?;
    i.temp_roots.push(desc.clone());
    let kept = list(i, &objects, false)?;
    i.temp_roots.push(Value::Object(kept));
    let lists = list(i, &snapshot, false)?;
    Ok(Some((desc, kept, def, cancel, lists)))
}

/// Adds `Window`, its prototype's methods, and the host functions in `items`.
pub(crate) fn install(i: &mut Interp, items: Vec<(String, HostItem)>) -> JsResult<()> {
    let host = i.host_object("Window", items)?;
    let object_proto = i.intrinsics.object_proto;
    i.rooted(&[Value::Object(host)], |i| {
        i.reserve_heap(OBJECT_BYTES.saturating_mul(4))?;
        let window_proto = i.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));
        i.temp_roots.push(Value::Object(window_proto));
        let control_proto = i.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));
        i.temp_roots.push(Value::Object(control_proto));
        let item_proto = i.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));
        i.temp_roots.push(Value::Object(item_proto));
        let layout = i.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));
        i.temp_roots.push(Value::Object(layout));
        let selection_get = native(i, "selection", selection_get, 0, NativeKind::Plain, false)?;
        i.temp_roots.push(Value::Object(selection_get));
        let selection_set = native(i, "selection", selection_set, 1, NativeKind::Plain, false)?;
        i.temp_roots.push(Value::Object(selection_set));
        i.script_ui = Some(Classes {
            window_proto,
            control_proto,
            item_proto,
            layout,
            host,
            selection_get,
            selection_set,
            showing: None,
            palettes: Vec::new(),
            taking: false,
        });
        let window = native(i, "Window", window_ctor, 4, NativeKind::Plain, true)?;
        constructor(i, "Window", window, window_proto)?;
        let event = native(i, "__vcPaletteEvent", palette_event, 0, NativeKind::Plain, false)?;
        i.define(window, key("__vcPaletteEvent"), Prop::Data { value: Value::Object(event), flags: 0 })?;
        method(i, window_proto, "add", add, 4)?;
        method(i, window_proto, "findElement", find_element, 1)?;
        method(i, window_proto, "close", close, 1)?;
        method(i, window_proto, "hide", hide, 0)?;
        method(i, window_proto, "center", nothing, 1)?;
        method(i, window_proto, "update", update, 0)?;
        pausable_method(i, window_proto, "show", show, 0)?;
        method(i, control_proto, "add", add, 4)?;
        method(i, control_proto, "findElement", find_element, 1)?;
        method(i, control_proto, "removeAll", remove_all, 0)?;
        method(i, control_proto, "notify", notify, 1)?;
        method(i, control_proto, "show", control_show, 0)?;
        method(i, control_proto, "hide", control_hide, 0)?;
        method(i, control_proto, "update", control_update, 0)?;
        method(i, item_proto, "toString", item_to_string, 0)?;
        let index = native(i, "index", item_index, 0, NativeKind::Plain, false)?;
        i.define(item_proto, key("index"), Prop::Accessor { get: Some(index), set: None, flags: 0 })?;
        let get = native(i, "selected", item_selected_get, 0, NativeKind::Plain, false)?;
        let set = native(i, "selected", item_selected_set, 1, NativeKind::Plain, false)?;
        i.define(item_proto, key("selected"), Prop::Accessor { get: Some(get), set: Some(set), flags: 0 })?;
        method(i, layout, "layout", nothing, 1)?;
        method(i, layout, "resize", nothing, 0)?;
        i.object_mut(layout)?.extensible = false;
        Ok(())
    })
}
