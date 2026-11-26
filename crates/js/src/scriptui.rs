//! ScriptUI's `Window` with modal dialogs, as the help page
//! <https://extendscript.docsforadobe.dev/user-interface-tools/window-object/> describes it, whose
//! drawing a host does ([`crate::Realm::define_script_ui`]).
//!
//! `new Window("dialog"[, title, bounds, properties])` makes a dialog, with or without `new`. A
//! window of type `palette` or `window` throws an `Error` that says it is not available yet.
//! `new Window` and `add` also take a resource specification in place of the type, as the help
//! page <https://extendscript.docsforadobe.dev/user-interface-tools/resource-specifications/>
//! describes it (read by [`crate::scriptui_resource`]), and then read no other argument: the
//! window or control is made with the creation properties given in `properties`. Then, in order,
//! each control listed in its braces is added to it through `add`, with `add`'s limits, and
//! assigned to the container's property of that name, a list's `items` gives its item texts as
//! `add`'s third argument does, and each other value is assigned to the property of its name. A
//! malformed specification throws a `SyntaxError`. `add(type[, bounds, text, properties])` adds a
//! control to a dialog, a `group` or a `panel` and returns it: `statictext`, `edittext`, `button`,
//! `checkbox`, `radiobutton`, `dropdownlist`, `listbox` (whose third argument is an array of item
//! texts), `slider` (third to fifth: value, minimum, maximum) and `progressbar` (value, maximum).
//! The other documented control types throw an `Error` that says they are not available yet, and
//! `show()` throws one for an `edittext` whose `properties` have `noecho`. A
//! group takes its creation properties as the third argument (or the fourth when the third is not
//! an object), and a list takes them there in place of its items, with the items in
//! `properties.items`. A list's `add("item", text)` adds an item, `find(text)` finds one and
//! `remove(what)` removes one; its `selection` is an item or `null`, and takes an item, an index or
//! `null`. A container's `remove(what)` removes a control, which then has no `parent` and no longer
//! belongs to the dialog: its `window`, and that of each control it holds, is `null`, and `add`,
//! `remove` and `removeAll` on them throw an `Error`. Each element has an id, which the description
//! gives, that no other element of the run has.
//! Each element starts with `preferredSize`, `size`, `minimumSize` and `maximumSize` objects whose
//! `width` and `height` are -1, which leaves the size to the layout. Setting a radio button's
//! `value` to true sets the other radio buttons next to it in its container to false, as a click
//! does.
//!
//! `addEventListener(type, handler[, capture])` and `removeEventListener` register handlers for
//! the events `show()` runs: `click`, `change`, `changing`, and `show` on a dialog only; a window
//! or control takes at most [`MAX_LISTENERS`] of them. A handler gets an event object, in the
//! capture phase on the containers above the control, then on the control after its `onClick`,
//! `onChange` or `onChanging`, then in the bubbling phase on the containers again, until one calls
//! `stopPropagation()`. A handler removed before its turn does not run, and a container that no
//! longer holds the control when its turn comes does not get the event. Events are not cancelable,
//! and `preventDefault()` does nothing.
//! `notify(event)` runs the same handlers, and `notify("onClick")` turns a check box over and turns
//! a radio button on first; it does not close the dialog. `Window.alert`, `Window.confirm` and
//! `Window.prompt` are the global functions of those names, when the host defines them.
//!
//! `show()` gives the host's `open` the dialog's description and runs `onShow`. Then, until the
//! dialog closes, it gives the host's `wait` the description again, and `wait` gives back what the
//! person did, which may pause the run ([`crate::interp::Pause`]): a `click` on a control, a
//! `change` with its new value, or a `close`. The control takes the change, its `onClick`,
//! `onChanging` or `onChange` runs, and the next description shows what the handler changed. A
//! button without `onClick` and without a `click` listener of its own closes the dialog with 1 when
//! it is the dialog's default element and with 2 when it is its cancel element, unless a capturing
//! listener on a container stopped the click. `close(result)` closes the dialog after the running
//! handler returns, and `show()` gives `result`, or 0 without one; `onClose` runs first and keeps
//! the dialog open when it returns `false`. A dialog that is showing cannot be shown again, and no
//! other dialog can be shown while one is. When `show()` ends, the host's `end` is given its result
//! and whether it ended normally.
//!
//! An element keeps its kind, its container, its controls and a list's items in properties that a
//! script cannot change. The description lists the dialog first and every control after the
//! container that holds it; it is read with a loop, and a dialog with more than [`MAX_CONTROLS`]
//! controls, nested more than [`MAX_DEPTH`] deep, with more than [`MAX_ITEMS`] list items, or with a
//! text longer than [`MAX_TEXT_BYTES`] throws a `RangeError`. These and [`MAX_LISTENERS`] are
//! VectorCraft's own limits. A getter that changes a list's items after the list was read, or adds
//! or removes a control after the dialog's controls were read, makes `show()` read the dialog
//! again, and a dialog that changes again throws an `Error`. A `text` getter
//! that changes a list or a container while `remove(text)` looks for an item or a control makes
//! `remove` throw an `Error`.

use crate::HostItem;
use crate::builtins::{arg, constructor, data, method, native, pausable_method};
use crate::heap::{Class, ENUMERABLE, HIDDEN, NativeKind, OBJECT_BYTES, OPEN, ObjId, Object, PROP_BYTES, Prop, key, string_bytes};
use crate::host::Data;
use crate::interp::{Abrupt, ErrorType, Interp, JsResult, Pause};
use crate::scriptui_resource::{self, Fault, Res, Spec};
use crate::value::{Value, index_key};

/// The most controls a dialog holds, its groups and panels included.
pub const MAX_CONTROLS: usize = 512;
/// How deeply controls nest in a dialog: a control the dialog holds is at depth 1.
pub const MAX_DEPTH: usize = 16;
/// The most items a dialog's lists hold together.
pub const MAX_ITEMS: usize = 1024;
/// The most bytes (UTF-8) of one text in a dialog: a control's text or name, or a list item's.
pub const MAX_TEXT_BYTES: usize = 16 << 10;
/// The most event listeners one window or control holds.
pub const MAX_LISTENERS: usize = 256;

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
/// How many times a list's items have been replaced, which tells a list from one whose items were
/// removed and added again with the same texts.
const ITEMS_REV: &str = "__vcItemsRev";
const SELECTION: &str = "__vcSelection";
const LIST: &str = "__vcList";
const VALUE: &str = "__vcValue";
const LISTENERS: &str = "__vcListeners";
const STOPPED: &str = "__vcStopped";
const ID: &str = "__vcId";
/// The number of listeners an element has had registered, which numbers each registration.
const REGISTERED: &str = "__vcRegistered";

/// The longest array an element keeps in a slot: a list's items, a container's controls, its
/// listeners (four values each), and the elements of a description.
const LIST_CAP: u32 = {
    let most = if MAX_ITEMS > MAX_CONTROLS + 1 { MAX_ITEMS } else { MAX_CONTROLS + 1 };
    let most = if most > 4 * MAX_LISTENERS { most } else { 4 * MAX_LISTENERS };
    most as u32
};

/// The size properties every element starts with, as objects with `width` and `height`.
const SIZES: [&str; 4] = ["preferredSize", "size", "minimumSize", "maximumSize"];
/// The events `addEventListener` takes, which `show()` and `notify` run.
const EVENTS: [&str; 4] = ["click", "change", "changing", "show"];
/// The other documented events, which nothing runs yet.
const EVENTS_LATER: [&str; 14] = [
    "move",
    "moving",
    "resize",
    "resizing",
    "enterKey",
    "focus",
    "blur",
    "mousedown",
    "mouseup",
    "mousemove",
    "mouseover",
    "mouseout",
    "keyup",
    "keydown",
];

/// The objects of `Window`, kept by the realm, and the dialog that is showing.
#[derive(Clone, Debug)]
pub(crate) struct Classes {
    window_proto: ObjId,
    control_proto: ObjId,
    item_proto: ObjId,
    event_proto: ObjId,
    layout: ObjId,
    host: ObjId,
    selection_get: ObjId,
    selection_set: ObjId,
    radio_get: ObjId,
    radio_set: ObjId,
    showing: Option<ObjId>,
    /// The event objects whose listeners are running, innermost last.
    dispatching: Vec<ObjId>,
    /// The containers above the targets of the running events.
    above: Vec<ObjId>,
    /// The id the next element gets: a number no other element of the run has.
    next_id: f64,
}

impl Classes {
    /// The objects a collection keeps.
    pub(crate) fn objects(&self) -> Vec<ObjId> {
        let mut all = vec![
            self.window_proto,
            self.control_proto,
            self.item_proto,
            self.event_proto,
            self.layout,
            self.host,
            self.selection_get,
            self.selection_set,
            self.radio_get,
            self.radio_set,
        ];
        all.extend(self.showing);
        all.extend(self.dispatching.iter().copied());
        all.extend(self.above.iter().copied());
        all
    }
}

/// Forgets the dialog that was showing, and the events that were running, when a run ended where
/// it was.
pub(crate) fn end_run(i: &mut Interp) {
    if let Some(c) = i.script_ui.as_mut() {
        c.showing = None;
        c.dispatching.clear();
        c.above.clear();
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

/// Gives the list `o` the array `kept` as its items and counts the replacement ([`ITEMS_REV`]).
fn set_items(i: &mut Interp, o: ObjId, kept: ObjId) -> JsResult<()> {
    let rev = number_slot(i, o, ITEMS_REV)? + 1.0;
    // The array stays kept from collection while the revision's slot reserves room.
    i.rooted(&[Value::Object(kept)], |i| set_slot(i, o, ITEMS_REV, Value::Number(rev)))?;
    set_slot(i, o, ITEMS, Value::Object(kept))
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

/// The dialog that holds the element `o`, or an `Error` naming `what` for a control that was
/// removed from its dialog.
fn window_of(i: &mut Interp, o: ObjId, what: &str) -> JsResult<ObjId> {
    match object_slot(i, o, WINDOW)? {
        Some(w) => Ok(w),
        None => Err(error(i, &format!("{what}: the control was removed from its dialog"))),
    }
}

/// [`window_of`] for an element whose controls or items are to be removed, checked to be in that
/// dialog: each container from it up to the dialog holds the one below. An object a script made to
/// look like an element is a `TypeError` naming `what`.
fn window_in(i: &mut Interp, o: ObjId, what: &str) -> JsResult<ObjId> {
    let win = window_of(i, o, what)?;
    let mut at = o;
    for _ in 0..=MAX_DEPTH {
        if at == win {
            break;
        }
        let Some(p) = object_slot(i, at, PARENT)? else { break };
        if !kids_of(i, p)?.iter().any(|k| same(k, &Value::Object(at))) {
            break;
        }
        at = p;
    }
    if at == win && object_slot(i, win, WINDOW)? == Some(win) {
        return Ok(win);
    }
    Err(i.type_error(&format!("{what}: this is not a ScriptUI window or control")))
}

/// Are `a` and `b` the same object?
fn same(a: &Value, b: &Value) -> bool {
    matches!((a.as_object(), b.as_object()), (Some(x), Some(y)) if x == y)
}

/// The values of an array made by [`list`]. An array longer than any such array ([`LIST_CAP`]),
/// which only an object a script made to look like an element holds, reads as empty.
fn list_values(i: &Interp, a: ObjId) -> JsResult<Vec<Value>> {
    let length = match &i.object(a)?.class {
        Class::Array { length, .. } if *length <= LIST_CAP => *length,
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
    let id = match i.script_ui.as_mut() {
        Some(c) => {
            c.next_id += 1.0;
            c.next_id
        }
        None => 0.0,
    };
    set_slot(i, o, ID, Value::Number(id))?;
    let listeners = list(i, &[], false)?;
    set_slot(i, o, LISTENERS, Value::Object(listeners))?;
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
    for name in SIZES {
        let d = dimension(i, -1.0, -1.0)?;
        set_open(i, o, name, d)?;
    }
    let container = matches!(kind, "dialog" | "group" | "panel");
    if container {
        let kids = list(i, &[], false)?;
        set_slot(i, o, KIDS, Value::Object(kids))?;
        let children = list(i, &[], true)?;
        set_open(i, o, "children", Value::Object(children))?;
        set_open(i, o, "orientation", Value::str(if kind == "group" { "row" } else { "column" }))?;
        set_open(i, o, "alignChildren", Value::str("center"))?;
        set_open(i, o, "spacing", Value::Number(10.0))?;
        set_open(i, o, "margins", Value::Number(if kind == "group" { 0.0 } else { 15.0 }))?;
    }
    // Layout properties given with the creation properties start the element with those values.
    let layout_names: &[&str] = if container { &["alignment", "orientation", "alignChildren", "spacing", "margins"] } else { &["alignment"] };
    for name in layout_names {
        let v = i.get_value(&props, &key(name))?;
        if !matches!(v, Value::Undefined) {
            set_open(i, o, name, v)?;
        }
    }
    Ok(o)
}

/// A new size object with `width` and `height`.
fn dimension(i: &mut Interp, width: f64, height: f64) -> JsResult<Value> {
    i.reserve_heap(OBJECT_BYTES.saturating_add(PROP_BYTES.saturating_mul(2)))?;
    let o = i.new_object();
    i.rooted(&[Value::Object(o)], |i| {
        set_open(i, o, "width", Value::Number(width))?;
        set_open(i, o, "height", Value::Number(height))?;
        Ok(Value::Object(o))
    })
}

/// `new Window(type[, title, bounds, properties])`.
fn window_ctor(i: &mut Interp, _: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let kind = i.to_str(&arg(args, 0))?.to_rust_string();
    let spec = if kind.contains('{') { Some(resource(i, &kind, "Window")?) } else { None };
    let lower = spec.as_ref().map_or(kind.as_str(), |s| s.class.as_str()).to_lowercase();
    if lower == "palette" || lower == "window" {
        return Err(error(i, &format!("Window: a window of type {lower} is not available yet; use dialog")));
    }
    if lower != "dialog" {
        return Err(error(i, "Window: the type must be dialog, palette or window"));
    }
    let proto = classes(i)?.window_proto;
    let mark = i.temp_roots.len();
    let r = (|| {
        // A resource specification gives everything; the other arguments are not read.
        let o = match &spec {
            Some(spec) => {
                let creation = creation_properties(i, spec)?;
                new_element(i, proto, "dialog", None, None, 0, [&Value::Undefined, &Value::Undefined, &creation])?
            }
            None => new_element(i, proto, "dialog", None, None, 0, [&arg(args, 2), &arg(args, 1), &arg(args, 3)])?,
        };
        set_slot(i, o, COUNT, Value::Number(0.0))?;
        set_slot(i, o, ITEM_COUNT, Value::Number(0.0))?;
        if let Some(spec) = &spec {
            apply_resource(i, o, spec, false)?;
        }
        Ok(Value::Object(o))
    })();
    i.temp_roots.truncate(mark);
    r
}

/// The resource specification `text` given to `what`, or a `SyntaxError` or `RangeError` naming
/// `what`.
fn resource(i: &mut Interp, text: &str, what: &str) -> JsResult<Spec> {
    match scriptui_resource::parse(text) {
        Ok(spec) => Ok(spec),
        Err(Fault::Syntax(m)) => Err(i.throw(ErrorType::Syntax, &format!("{what}: {m}"))),
        Err(Fault::Limit(m)) => Err(range(i, &format!("{what}: {m}"))),
    }
}

/// The value of a resource specification's `properties`, its creation properties, kept in the
/// temporary roots; `undefined` when it has none.
fn creation_properties(i: &mut Interp, spec: &Spec) -> JsResult<Value> {
    let Some((_, v)) = spec.props.iter().rev().find(|(name, _)| name == "properties") else { return Ok(Value::Undefined) };
    if matches!(v, Res::Control(_)) {
        return Err(i.type_error("properties: give the creation properties as an object"));
    }
    let v = resource_value(i, v)?;
    i.temp_roots.push(v.clone());
    Ok(v)
}

/// A new value for the resource value `v`, built in the temporary roots.
fn resource_value(i: &mut Interp, v: &Res) -> JsResult<Value> {
    Ok(match v {
        Res::Null => Value::Null,
        Res::Bool(b) => Value::Bool(*b),
        Res::Number(n) => Value::Number(*n),
        Res::String(s) => Value::str(s),
        Res::Array(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                let v = resource_value(i, item)?;
                i.temp_roots.push(v.clone());
                values.push(v);
            }
            let a = list(i, &values, true)?;
            i.temp_roots.push(Value::Object(a));
            Value::Object(a)
        }
        Res::Object(props) => {
            i.reserve_heap(OBJECT_BYTES)?;
            let o = i.new_object();
            i.temp_roots.push(Value::Object(o));
            for (name, v) in props {
                let v = resource_value(i, v)?;
                set_open(i, o, name, v)?;
            }
            Value::Object(o)
        }
        Res::Control(_) => return Err(i.type_error("a control is not a value")),
    })
}

/// Adds the control `spec` describes to the container `parent`, as `add` does, with the controls
/// and properties it lists.
fn add_resource(i: &mut Interp, parent: ObjId, spec: &Spec) -> JsResult<Value> {
    let creation = creation_properties(i, spec)?;
    // `add` reads a slider's and a progress bar's later arguments as numbers; their creation
    // properties are set after `add` makes them.
    let class = spec.class.to_lowercase();
    let numeric = matches!(class.as_str(), "slider" | "progressbar");
    // A list's `items` gives its item texts, as `add`'s third argument does.
    let list_kind = matches!(class.as_str(), "dropdownlist" | "listbox");
    let items = match spec.props.iter().rev().find(|(name, _)| name == "items") {
        // Only an array is item texts; `add` would read an object third as creation properties.
        Some((_, Res::Null)) if list_kind => Value::Undefined,
        Some((_, v)) if list_kind && !matches!(v, Res::Array(_)) => {
            return Err(i.type_error("items: give a list's items as an array"));
        }
        Some((_, v)) if list_kind => {
            let v = resource_value(i, v)?;
            i.temp_roots.push(v.clone());
            v
        }
        _ => Value::Undefined,
    };
    let args = if numeric { vec![Value::str(&spec.class)] } else { vec![Value::str(&spec.class), Value::Undefined, items, creation.clone()] };
    let c = add(i, Value::Object(parent), &args, false)?;
    i.temp_roots.push(c.clone());
    let Some(o) = c.as_object() else { return Err(Abrupt::Internal("add gave no control".into())) };
    if numeric && creation.as_object().is_some() {
        set_open(i, o, "properties", creation.clone())?;
        let name = i.get_value(&creation, &key("name"))?;
        let name = text_arg(i, &name)?;
        set_open(i, o, "name", name)?;
        let alignment = i.get_value(&creation, &key("alignment"))?;
        if !matches!(alignment, Value::Undefined) {
            set_open(i, o, "alignment", alignment)?;
        }
    }
    apply_resource(i, o, spec, list_kind)?;
    Ok(c)
}

/// Sets the properties `spec` lists on the element `o`, in order, leaving out `properties`, and
/// `items` for a list (`list`): a control is added to `o` and assigned to `o`'s property of that
/// name, and any other value is assigned to the property of its name.
fn apply_resource(i: &mut Interp, o: ObjId, spec: &Spec, list: bool) -> JsResult<()> {
    for (name, v) in &spec.props {
        if name == "properties" || (list && name == "items") {
            continue;
        }
        let value = match v {
            Res::Control(child) => add_resource(i, o, child)?,
            v => resource_value(i, v)?,
        };
        i.temp_roots.push(value.clone());
        i.put(o, key(name), value, true)?;
    }
    Ok(())
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
    window_of(i, o, "add")?;
    let ty = i.to_str(&arg(args, 0))?.to_rust_string();
    if kind == "dropdownlist" || kind == "listbox" {
        if ty.to_lowercase() != "item" {
            return Err(error(i, "add: a list holds only items"));
        }
        return add_item(i, o, &arg(args, 1));
    }
    if !matches!(kind.as_str(), "dialog" | "group" | "panel") {
        return Err(error(i, &format!("add: a {kind} holds no controls")));
    }
    if ty.contains('{') {
        let spec = resource(i, &ty, "add")?;
        let mark = i.temp_roots.len();
        let r = add_resource(i, o, &spec);
        i.temp_roots.truncate(mark);
        return r;
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
    let win = window_of(i, o, "add")?;
    let count = number_slot(i, win, COUNT)?;
    if count as usize >= MAX_CONTROLS {
        return Err(range(i, &format!("add: a dialog can hold at most {MAX_CONTROLS} controls")));
    }
    let list_kind = ty == "dropdownlist" || ty == "listbox";
    let third = arg(args, 2);
    let is_array = |i: &Interp, v: &Value| -> JsResult<bool> {
        Ok(match v.as_object() {
            Some(a) => matches!(i.object(a)?.class, Class::Array { .. }),
            None => false,
        })
    };
    // A group takes its creation properties third, and so may a list in place of its items; when
    // the third argument is not an object, they are the fourth.
    let props_third = third.as_object().is_some() && (ty == "group" || (list_kind && !is_array(i, &third)?));
    let props = if props_third { third.clone() } else { arg(args, 3) };
    let proto = classes(i)?.control_proto;
    let mark = i.temp_roots.len();
    let r = (|| {
        let mut given_items = if list_kind && !props_third { third.clone() } else { Value::Undefined };
        if list_kind && props.as_object().is_some() {
            let listed = i.get_value(&props, &key("items"))?;
            i.temp_roots.push(listed.clone());
            if !matches!(listed, Value::Undefined | Value::Null) {
                if !matches!(given_items, Value::Undefined | Value::Null) {
                    return Err(i.type_error("add: give a list's items as the third argument or in properties.items, not both"));
                }
                given_items = listed;
            }
        }
        if ty == "listbox" && props.as_object().is_some() && i.get_value(&props, &key("multiselect"))?.truthy() {
            return Err(error(i, "add: a list box that selects several items is not available yet"));
        }
        let creation = if matches!(ty, "slider" | "progressbar") { Value::Undefined } else { props.clone() };
        let text = if list_kind || matches!(ty, "group" | "slider" | "progressbar") { Value::Undefined } else { third.clone() };
        let c = new_element(i, proto, ty, Some(o), Some(win), depth, [&arg(args, 1), &text, &creation])?;
        match ty {
            "checkbox" => set_open(i, c, "value", Value::Bool(false))?,
            "radiobutton" => {
                set_slot(i, c, VALUE, Value::Bool(false))?;
                let cl = classes(i)?;
                i.define(c, key("value"), Prop::Accessor { get: Some(cl.radio_get), set: Some(cl.radio_set), flags: ENUMERABLE })?;
            }
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
                set_items(i, c, kept)?;
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
            match given_items.clone() {
                Value::Undefined | Value::Null => {}
                Value::Object(a) if matches!(i.object(a)?.class, Class::Array { .. }) => {
                    let length = match &i.object(a)?.class {
                        Class::Array { length, .. } => *length as usize,
                        _ => 0,
                    };
                    if length > MAX_ITEMS.saturating_sub(number_slot(i, win, ITEM_COUNT)? as usize) {
                        return Err(range(i, &format!("add: a dialog's lists hold at most {MAX_ITEMS} items in all")));
                    }
                    // Each item is read as a script reads it, so an index getter gives its value.
                    let mut texts = Vec::with_capacity(length);
                    for n in 0..length {
                        let n = u32::try_from(n).map_err(|_| Abrupt::Internal("a list is too long".into()))?;
                        let item = i.get_value(&Value::Object(a), &index_key(n))?;
                        i.temp_roots.push(item.clone());
                        let text = text_arg(i, &item)?;
                        i.temp_roots.push(text.clone());
                        texts.push(text);
                    }
                    staged = new_items(i, c, &texts)?;
                }
                _ => return Err(i.type_error("add: give a list's items as an array")),
            }
        }
        // A getter or a conversion above may have added controls or items itself, or removed the
        // container from its dialog. An object made to look like a container is not in the dialog,
        // so it charges nothing to the dialog's controls.
        window_in(i, o, "add")?;
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
        Ok(Value::Object(c))
    })();
    i.temp_roots.truncate(mark);
    r
}

/// Adds an item with `text` to the list `o`, which the dialog holds.
fn add_item(i: &mut Interp, o: ObjId, text: &Value) -> JsResult<Value> {
    // The conversion runs first: it may add or remove items itself.
    let text = text_arg(i, text)?;
    i.rooted(std::slice::from_ref(&text), |i| {
        let win = window_in(i, o, "add")?;
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
            Ok(Value::Object(item))
        })
    })
}

/// Gives the new list `o`, which has no items yet, an item for each of `texts`, building its
/// arrays of items once, without counting them in the dialog's items. → how many were added.
fn new_items(i: &mut Interp, o: ObjId, texts: &[Value]) -> JsResult<usize> {
    let proto = classes(i)?.item_proto;
    let mark = i.temp_roots.len();
    let r = (|| {
        let mut items = Vec::with_capacity(texts.len());
        for text in texts {
            i.reserve_heap(OBJECT_BYTES.saturating_add(PROP_BYTES.saturating_mul(4)))?;
            let item = i.heap.alloc_object(Object::new(Class::Ordinary, Some(proto)));
            i.temp_roots.push(Value::Object(item));
            set_slot(i, item, LIST, Value::Object(o))?;
            set_open(i, item, "text", text.clone())?;
            items.push(Value::Object(item));
        }
        let kept = list(i, &items, false)?;
        i.temp_roots.push(Value::Object(kept));
        set_items(i, o, kept)?;
        let copy = list(i, &items, true)?;
        set_open(i, o, "items", Value::Object(copy))?;
        Ok(items.len())
    })();
    i.temp_roots.truncate(mark);
    r
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
    let win = window_in(i, o, "removeAll")?;
    let n = items_of(i, o)?.len() as f64;
    let count = number_slot(i, win, ITEM_COUNT)?;
    set_slot(i, win, ITEM_COUNT, Value::Number((count - n).max(0.0)))?;
    let kept = list(i, &[], false)?;
    set_items(i, o, kept)?;
    let items = list(i, &[], true)?;
    set_open(i, o, "items", Value::Object(items))?;
    set_slot(i, o, SELECTION, Value::Number(-1.0))?;
    Ok(Value::Undefined)
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
    let mark = i.temp_roots.len();
    let r = (|| {
        // The controls still to look at stay rooted while a `name` getter runs, and each one is
        // looked at only while the element still holds it.
        let mut stack: Vec<Value> = kids_of(i, o)?.into_iter().rev().collect();
        i.temp_roots.extend(stack.iter().cloned());
        while let Some(v) = stack.pop() {
            i.use_fuel()?;
            let Some(c) = v.as_object() else { continue };
            if !holds(i, o, c)? {
                continue;
            }
            let name = i.get_value(&v, &key("name"))?;
            if !holds(i, o, c)? {
                continue;
            }
            if matches!(&name, Value::String(s) if *s == want) {
                return Ok(v);
            }
            let kids = kids_of(i, c)?;
            i.temp_roots.extend(kids.iter().cloned());
            stack.extend(kids.into_iter().rev());
        }
        Ok(Value::Null)
    })();
    i.temp_roots.truncate(mark);
    r
}

/// Does the element `top` hold the control `c`: does each container from `c`'s up to `top` hold
/// the one below?
fn holds(i: &Interp, top: ObjId, c: ObjId) -> JsResult<bool> {
    let mut at = c;
    for _ in 0..=MAX_DEPTH {
        let Some(p) = object_slot(i, at, PARENT)? else { return Ok(false) };
        if !kids_of(i, p)?.iter().any(|k| same(k, &Value::Object(at))) {
            return Ok(false);
        }
        if p == top {
            return Ok(true);
        }
        at = p;
    }
    Ok(false)
}

/// Calls the element's handler `name` when it has one. → its result, or `None` without one.
fn handler(i: &mut Interp, o: ObjId, name: &str) -> JsResult<Option<Value>> {
    let f = i.get_value(&Value::Object(o), &key(name))?;
    if !i.is_callable(&f) {
        return Ok(None);
    }
    i.call(&f, &Value::Object(o), &[]).map(Some)
}

/// The radio buttons that form a set with `c`: it and the radio buttons next to it in its
/// container, up to the first control of another kind on each side.
fn radio_set_of(i: &Interp, c: ObjId) -> JsResult<Vec<ObjId>> {
    let Some(parent) = object_slot(i, c, PARENT)? else { return Ok(vec![c]) };
    let kids = kids_of(i, parent)?;
    let Some(at) = kids.iter().position(|k| same(k, &Value::Object(c))) else { return Ok(vec![c]) };
    let radio = |v: &Value| -> JsResult<Option<ObjId>> {
        Ok(match kind_of(i, v)? {
            Some((o, k)) if k == "radiobutton" => Some(o),
            _ => None,
        })
    };
    let mut set = vec![c];
    for k in kids[..at].iter().rev() {
        let Some(o) = radio(k)? else { break };
        set.push(o);
    }
    for k in &kids[at + 1..] {
        let Some(o) = radio(k)? else { break };
        set.push(o);
    }
    Ok(set)
}

fn radio_get(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = element(i, &this, "value")?;
    Ok(Value::Bool(slot(i, o, VALUE)?.truthy()))
}

/// A radio button's `value`: true turns the other radio buttons of its set off.
fn radio_set(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = element(i, &this, "value")?;
    let on = arg(args, 0).truthy();
    if on {
        for other in radio_set_of(i, o)? {
            if other != o {
                set_slot(i, other, VALUE, Value::Bool(false))?;
            }
        }
    }
    set_slot(i, o, VALUE, Value::Bool(on))?;
    Ok(Value::Undefined)
}

/// A listener registered on an element: its event type, handler, capture flag and registration
/// number.
type Listener = (Value, Value, bool, f64);

/// The listeners registered on `o`, in the order they were added.
fn listeners_of(i: &Interp, o: ObjId) -> JsResult<Vec<Listener>> {
    let flat = match object_slot(i, o, LISTENERS)? {
        Some(a) => list_values(i, a)?,
        None => Vec::new(),
    };
    Ok(flat
        .as_chunks::<4>()
        .0
        .iter()
        .map(|[t, f, c, n]| (t.clone(), f.clone(), c.truthy(), if let Value::Number(n) = n { *n } else { 0.0 }))
        .collect())
}

fn set_listeners(i: &mut Interp, o: ObjId, all: &[Listener]) -> JsResult<()> {
    let flat: Vec<Value> = all.iter().flat_map(|(t, f, c, n)| [t.clone(), f.clone(), Value::Bool(*c), Value::Number(*n)]).collect();
    i.rooted(&flat.clone(), |i| {
        let kept = list(i, &flat, false)?;
        set_slot(i, o, LISTENERS, Value::Object(kept))
    })
}

/// Is the stored event type `t` the type `ty`?
fn is_type(t: &Value, ty: &str) -> bool {
    matches!(t, Value::String(s) if s.to_rust_string() == ty)
}

/// The event type `addEventListener` and `removeEventListener` were given, checked.
fn event_type(i: &mut Interp, v: &Value, what: &str) -> JsResult<&'static str> {
    let text = i.to_str(v)?.to_rust_string();
    if let Some(name) = EVENTS.iter().find(|e| **e == text) {
        return Ok(name);
    }
    if EVENTS_LATER.contains(&text.as_str()) {
        return Err(error(i, &format!("{what}: the event {text} is not available yet")));
    }
    Err(error(i, &format!("{what}: there is no event {}", text.chars().take(64).collect::<String>())))
}

/// `addEventListener(type, handler[, capture])`.
fn add_event_listener(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, kind) = element(i, &this, "addEventListener")?;
    let ty = event_type(i, &arg(args, 0), "addEventListener")?;
    if ty == "show" && kind != "dialog" {
        return Err(error(i, "addEventListener: the event show runs only on a dialog"));
    }
    let f = arg(args, 1);
    if !i.is_callable(&f) {
        return Err(i.type_error("addEventListener: give the handler as a function"));
    }
    let capture = arg(args, 2).truthy();
    let mut all = listeners_of(i, o)?;
    if !all.iter().any(|(t, g, c, _)| is_type(t, ty) && same(g, &f) && *c == capture) {
        if all.len() >= MAX_LISTENERS {
            return Err(range(i, &format!("addEventListener: a window or control takes at most {MAX_LISTENERS} listeners")));
        }
        // A listener removed and added again is a new registration, which an event already
        // running does not reach.
        let n = number_slot(i, o, REGISTERED)? + 1.0;
        set_slot(i, o, REGISTERED, Value::Number(n))?;
        all.push((Value::str(ty), f, capture, n));
        set_listeners(i, o, &all)?;
    }
    Ok(Value::Undefined)
}

/// `removeEventListener(type, handler[, capture])`.
fn remove_event_listener(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, _) = element(i, &this, "removeEventListener")?;
    let ty = event_type(i, &arg(args, 0), "removeEventListener")?;
    let f = arg(args, 1);
    let capture = arg(args, 2).truthy();
    let mut all = listeners_of(i, o)?;
    let before = all.len();
    all.retain(|(t, g, c, _)| !(is_type(t, ty) && same(g, &f) && *c == capture));
    if all.len() != before {
        set_listeners(i, o, &all)?;
    }
    Ok(Value::Undefined)
}

/// `event.stopPropagation()`: stops an event whose listeners are running; it does nothing to any
/// other object.
fn event_stop(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    if let Some(o) = this.as_object()
        && classes(i)?.dispatching.contains(&o)
    {
        set_slot(i, o, STOPPED, Value::Bool(true))?;
    }
    Ok(Value::Undefined)
}

/// Runs the listeners for `ty` on `at` (only those for `capture` unless it is `None`), with `event`,
/// in the order they were added. A listener that an earlier one removed does not run, even when
/// it was added again.
fn run_listeners(i: &mut Interp, at: ObjId, ty: &str, capture: Option<bool>, event: ObjId, phase: f64) -> JsResult<bool> {
    let wanted = |i: &Interp| -> JsResult<Vec<(Value, f64)>> {
        Ok(listeners_of(i, at)?
            .into_iter()
            .filter(|(t, _, c, _)| is_type(t, ty) && capture.is_none_or(|want| *c == want))
            .map(|(_, f, _, n)| (f, n))
            .collect())
    };
    let first = wanted(i)?;
    if first.is_empty() {
        return Ok(false);
    }
    let functions: Vec<Value> = first.iter().map(|(f, _)| f.clone()).collect();
    i.rooted(&functions, |i| {
        set_open(i, event, "currentTarget", Value::Object(at))?;
        set_open(i, event, "eventPhase", Value::Number(phase))?;
        for (f, n) in &first {
            if !wanted(i)?.iter().any(|(_, m)| m == n) {
                continue;
            }
            i.call(f, &Value::Object(at), &[Value::Object(event)])?;
        }
        Ok(true)
    })
}

/// Runs the event `ty` on the element `target`: the listeners of the containers above it that
/// capture, its `callback`, its own listeners, then the listeners of the containers that do not
/// capture, until one stops it. → whether the event was handled: the element had a callback or a
/// listener for it, or a capturing listener above it stopped it. Listeners of the containers that
/// do not capture do not count. A container that a listener took the element out of before its
/// turn does not hear the event.
fn dispatch(i: &mut Interp, target: ObjId, ty: &str, callback: &str) -> JsResult<bool> {
    let above = containers_above(i, target)?;
    let proto = classes(i)?.event_proto;
    i.reserve_heap(OBJECT_BYTES.saturating_add(PROP_BYTES.saturating_mul(10)))?;
    let event = i.heap.alloc_object(Object::new(Class::Ordinary, Some(proto)));
    // The event and the containers stay kept from collection while the listeners run.
    let kept = match i.script_ui.as_mut() {
        Some(c) => {
            c.dispatching.push(event);
            let mark = c.above.len();
            c.above.extend(above.iter().copied());
            mark
        }
        None => 0,
    };
    let r = run_event(i, event, target, &above, ty, callback);
    if let Some(c) = i.script_ui.as_mut() {
        if let Some(n) = c.dispatching.iter().rposition(|e| *e == event) {
            c.dispatching.remove(n);
        }
        c.above.truncate(kept);
    }
    r
}

/// The containers that hold `target`, innermost first. A chain longer than a dialog's nesting
/// allows comes from objects a script made to look like elements, and is a `TypeError`.
fn containers_above(i: &mut Interp, target: ObjId) -> JsResult<Vec<ObjId>> {
    let mut above = Vec::new();
    let mut at = object_slot(i, target, PARENT)?;
    while let Some(p) = at {
        i.use_fuel()?;
        if above.len() > MAX_DEPTH {
            return Err(i.type_error("the event's target is not a ScriptUI window or control"));
        }
        above.push(p);
        at = object_slot(i, p, PARENT)?;
    }
    Ok(above)
}

/// The phases of [`dispatch`] for the event object `event`.
fn run_event(i: &mut Interp, event: ObjId, target: ObjId, above: &[ObjId], ty: &str, callback: &str) -> JsResult<bool> {
    i.rooted(&[Value::Object(event)], |i| {
        let bubbles = ty != "show";
        set_open(i, event, "type", Value::str(ty))?;
        set_open(i, event, "target", Value::Object(target))?;
        set_open(i, event, "bubbles", Value::Bool(bubbles))?;
        set_open(i, event, "cancelable", Value::Bool(false))?;
        set_open(i, event, "detail", Value::Number(if ty == "click" { 1.0 } else { 0.0 }))?;
        let stopped = |i: &Interp| -> JsResult<bool> { Ok(slot(i, event, STOPPED)?.truthy()) };
        for p in above.iter().rev() {
            if !containers_above(i, target)?.contains(p) {
                continue;
            }
            run_listeners(i, *p, ty, Some(true), event, 1.0)?;
            if stopped(i)? {
                return Ok(true);
            }
        }
        let had = handler(i, target, callback)?.is_some();
        let heard = run_listeners(i, target, ty, None, event, 2.0)?;
        if bubbles && !stopped(i)? {
            for p in above {
                if !containers_above(i, target)?.contains(p) {
                    continue;
                }
                run_listeners(i, *p, ty, Some(false), event, 3.0)?;
                if stopped(i)? {
                    break;
                }
            }
        }
        Ok(had || heard)
    })
}

/// A click on the control `c` of `kind`: a check box turns over and a radio button turns on, then
/// the click runs. → whether the click was handled, as [`dispatch`] says.
fn click_control(i: &mut Interp, c: ObjId, kind: &str) -> JsResult<bool> {
    if kind == "checkbox" {
        let on = i.get_value(&Value::Object(c), &key("value"))?.truthy();
        i.put(c, key("value"), Value::Bool(!on), true)?;
    } else if kind == "radiobutton" {
        i.put(c, key("value"), Value::Bool(true), true)?;
    }
    dispatch(i, c, "click", "onClick")
}

/// `notify([event])`: simulates the control's `onClick`, `onChange` or `onChanging`.
fn notify(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, kind) = element(i, &this, "notify")?;
    let event = match arg(args, 0) {
        Value::Undefined if matches!(kind.as_str(), "button" | "checkbox" | "radiobutton") => "onClick".to_string(),
        Value::Undefined => "onChange".to_string(),
        v => i.to_str(&v)?.to_rust_string(),
    };
    match event.as_str() {
        "onClick" => click_control(i, o, &kind)?,
        "onChange" => dispatch(i, o, "change", "onChange")?,
        "onChanging" => dispatch(i, o, "changing", "onChanging")?,
        _ => return Err(error(i, "notify: give onClick, onChange or onChanging")),
    };
    Ok(Value::Undefined)
}

/// The index in `values` that `what` names: an index, one of the values, or a text that an
/// element's `text` equals. A text is converted before the values are read.
fn index_named(i: &mut Interp, what: &Value, values: impl Fn(&Interp) -> JsResult<Vec<Value>>) -> JsResult<(Vec<Value>, Option<usize>)> {
    match what {
        Value::Number(n) => {
            let all = values(i)?;
            let at = (*n >= 0.0 && n.fract() == 0.0 && (*n as usize) < all.len()).then_some(*n as usize);
            Ok((all, at))
        }
        Value::Object(_) => {
            let all = values(i)?;
            let at = all.iter().position(|v| same(v, what));
            Ok((all, at))
        }
        v => {
            let want = i.to_str(v)?;
            let all = values(i)?;
            i.rooted(&all.clone(), |i| {
                for (n, item) in all.iter().enumerate() {
                    i.use_fuel()?;
                    let t = i.get_value(item, &key("text"))?;
                    if matches!(&t, Value::String(s) if *s == want) {
                        return Ok((all.clone(), Some(n)));
                    }
                }
                Ok((all.clone(), None))
            })
        }
    }
}

/// `list.find(text)`: the first item whose text is `text`, or `null`.
fn find(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = list_of(i, &this, "find")?;
    let text = Value::String(i.to_str(&arg(args, 0))?);
    let (items, at) = index_named(i, &text, |i| items_of(i, o))?;
    Ok(at.and_then(|n| items.get(n).cloned()).unwrap_or(Value::Null))
}

/// The controls `o` holds whose container is `o`.
fn own_kids(i: &Interp, o: ObjId) -> JsResult<Vec<Value>> {
    let mut own = Vec::new();
    for k in kids_of(i, o)? {
        if let Some(c) = k.as_object()
            && object_slot(i, c, PARENT)? == Some(o)
        {
            own.push(k);
        }
    }
    Ok(own)
}

/// The controls in `c` and in the containers it holds, `c` included, and the number of their list
/// items.
fn subtree(i: &mut Interp, c: &Value) -> JsResult<(Vec<ObjId>, f64)> {
    let (mut controls, mut items) = (Vec::new(), 0.0);
    let mut stack = vec![c.clone()];
    while let Some(v) = stack.pop() {
        i.use_fuel()?;
        let Some(o) = v.as_object() else { continue };
        controls.push(o);
        items += items_of(i, o)?.len() as f64;
        stack.extend(own_kids(i, o)?);
    }
    Ok((controls, items))
}

/// `remove(what)`: removes a list's item, or a container's control, given by its index, the
/// object itself or its text. Nothing happens when there is no such item or control.
fn remove(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let (o, kind) = element(i, &this, "remove")?;
    let what = arg(args, 0);
    window_in(i, o, "remove")?;
    if kind == "dropdownlist" || kind == "listbox" {
        let before = slot(i, o, ITEMS)?;
        let (mut items, at) = i.rooted(std::slice::from_ref(&before), |i| index_named(i, &what, |i| items_of(i, o)))?;
        // A `text` getter read during the lookup may have changed the list or removed it.
        if !same(&slot(i, o, ITEMS)?, &before) {
            return Err(error(i, "remove: the list changed while the item was looked up"));
        }
        let win = window_in(i, o, "remove")?;
        let Some(at) = at else { return Ok(Value::Undefined) };
        items.remove(at);
        i.rooted(&items.clone(), |i| {
            let kept = list(i, &items, false)?;
            set_items(i, o, kept)?;
            let copy = list(i, &items, true)?;
            set_open(i, o, "items", Value::Object(copy))
        })?;
        let count = number_slot(i, win, ITEM_COUNT)?;
        set_slot(i, win, ITEM_COUNT, Value::Number((count - 1.0).max(0.0)))?;
        match selection_index(i, o)? {
            Some(n) if n == at => set_slot(i, o, SELECTION, Value::Number(-1.0))?,
            Some(n) if n > at => set_slot(i, o, SELECTION, Value::Number((n - 1) as f64))?,
            _ => {}
        }
        return Ok(Value::Undefined);
    }
    if !matches!(kind.as_str(), "dialog" | "group" | "panel") {
        return Err(error(i, &format!("remove: a {kind} holds no controls or items")));
    }
    let before = slot(i, o, KIDS)?;
    let (mut kids, at) = i.rooted(std::slice::from_ref(&before), |i| index_named(i, &what, |i| kids_of(i, o)))?;
    // A `text` getter read during the lookup may have changed the container or removed it.
    if !same(&slot(i, o, KIDS)?, &before) {
        return Err(error(i, "remove: the container's controls changed while the control was looked up"));
    }
    let win = window_in(i, o, "remove")?;
    let Some(at) = at else { return Ok(Value::Undefined) };
    if !matches!(kids.get(at).and_then(Value::as_object), Some(c) if object_slot(i, c, PARENT)? == Some(o)) {
        return Err(i.type_error("remove: this is not a ScriptUI window or control"));
    }
    let gone = kids.remove(at);
    let Some(root) = gone.as_object() else { return Ok(Value::Undefined) };
    let mut keep = kids.clone();
    keep.push(gone);
    i.rooted(&keep, |i| {
        // Everything that can fail runs before the first change, so a run that ends here (out of
        // fuel or memory) leaves the container, the dialog's counts and the control as they were.
        let (controls, items) = subtree(i, &keep[keep.len() - 1])?;
        let kept = Value::Object(list(i, &kids, false)?);
        i.rooted(std::slice::from_ref(&kept), |i| {
            let copy = Value::Object(list(i, &kids, true)?);
            i.rooted(std::slice::from_ref(&copy), |i| {
                let count = number_slot(i, win, COUNT)?;
                let total = number_slot(i, win, ITEM_COUNT)?;
                let mut writes = vec![
                    (o, KIDS, kept.clone(), 0),
                    (o, "children", copy.clone(), OPEN),
                    (win, COUNT, Value::Number((count - controls.len() as f64).max(0.0)), 0),
                    (win, ITEM_COUNT, Value::Number((total - items).max(0.0)), 0),
                ];
                writes.extend(detached(root, &controls));
                // The heap grows only by the properties a write adds, so the room for them is
                // checked once, before the first write.
                let mut growth = 0;
                for (at, name, _, _) in &writes {
                    if !i.object(*at)?.props.contains(&key(name)) {
                        growth += PROP_BYTES + string_bytes(name.len());
                    }
                }
                i.reserve_heap(growth)?;
                for (at, name, value, flags) in writes {
                    i.define(at, key(name), Prop::Data { value, flags })?;
                }
                Ok(())
            })
        })
    })?;
    Ok(Value::Undefined)
}

/// The writes that take the control `root`, removed from its container, out of the dialog: it has
/// no container, and neither it nor `controls`, the controls it holds, belong to the dialog any
/// longer, so their `window` is `null`.
fn detached(root: ObjId, controls: &[ObjId]) -> Vec<(ObjId, &'static str, Value, u8)> {
    let mut writes = Vec::with_capacity(2 * controls.len() + 2);
    for &o in controls {
        writes.push((o, WINDOW, Value::Null, 0));
        writes.push((o, "window", Value::Null, OPEN));
    }
    writes.push((root, PARENT, Value::Null, 0));
    writes.push((root, "parent", Value::Null, OPEN));
    writes
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

fn dialog_of(i: &mut Interp, this: &Value, what: &str) -> JsResult<ObjId> {
    match kind_of(i, this)? {
        Some((o, kind)) if kind == "dialog" => Ok(o),
        _ => Err(i.type_error(&format!("{what}: this is not a ScriptUI window"))),
    }
}

/// `close([result])`: closes the dialog that is showing once the running handler returns.
fn close(i: &mut Interp, this: Value, args: &[Value], _: bool) -> JsResult<Value> {
    let o = dialog_of(i, &this, "close")?;
    let result = match arg(args, 0) {
        Value::Undefined => 0.0,
        v => number_arg(i, &v, "close: the result")?,
    };
    if classes(i)?.showing == Some(o) {
        set_slot(i, o, CLOSING, Value::Number(result))?;
    }
    Ok(Value::Undefined)
}

/// `hide()`: closes the dialog that is showing with the result 0.
fn hide(i: &mut Interp, this: Value, _: &[Value], _: bool) -> JsResult<Value> {
    close(i, this, &[], false)
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

/// `show()` on a dialog: gives its description to the host's `open`, then runs it ([`opened`]).
fn show(i: &mut Interp, this: Value, _: &[Value]) -> JsResult<Pause> {
    let win = dialog_of(i, &this, "show")?;
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
    let open = match host_fn(i, "open") {
        Ok(open) => open,
        Err(e) => {
            set_showing(i, None);
            return Err(e);
        }
    };
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

/// Runs `onClose`. → false when it returned `false`. A `close()` that `onClose` calls belongs to
/// the close it decides on, so after `false` the dialog waits for the next event.
fn may_close(i: &mut Interp, win: ObjId) -> JsResult<bool> {
    let closes = !matches!(handler(i, win, "onClose")?, Some(Value::Bool(false)));
    if !closes {
        set_slot(i, win, CLOSING, Value::Undefined)?;
    }
    Ok(closes)
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
        // A getter that `describe` ran may have closed the dialog.
        if matches!(slot(i, win, CLOSING)?, Value::Number(_)) {
            continue;
        }
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
    if let Err(e) = &v {
        // The thrown value stays kept from collection while `failed` runs.
        let thrown = match e {
            Abrupt::Throw(t) => vec![t.clone()],
            _ => Vec::new(),
        };
        let mark = i.temp_roots.len();
        i.temp_roots.extend(thrown);
        failed(i, keep);
        i.temp_roots.truncate(mark);
    }
    v
}

fn opened(i: &mut Interp, keep: &[Value], _: Value) -> JsResult<Value> {
    guarded(i, keep, |i| {
        let Some(win) = arg(keep, 0).as_object() else { return Err(Abrupt::Internal("show lost its dialog".into())) };
        dispatch(i, win, "show", "onShow")?;
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
        let id = match slot(i, ev, "id")? {
            Value::Number(n) if n >= 0.0 && n.fract() == 0.0 => n as usize,
            _ => return Err(Abrupt::Internal("the host gave show an event without a control".into())),
        };
        let nodes = list_values(i, nodes)?;
        let Some((c, kind)) = nodes.get(id).map(|v| kind_of(i, v)).transpose()?.flatten() else {
            return Err(Abrupt::Internal("the host gave show an event for a control the dialog does not hold".into()));
        };
        let value = slot(i, ev, "value")?;
        match event.as_str() {
            "click" => {
                // A listener above the button that closed the dialog gives the result.
                if !click_control(i, c, &kind)? && kind == "button" && matches!(slot(i, win, CLOSING)?, Value::Undefined) {
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
                        dispatch(i, c, "changing", "onChanging")?;
                    }
                    "slider" => {
                        i.put(c, key("value"), value, true)?;
                        dispatch(i, c, "changing", "onChanging")?;
                    }
                    _ => {
                        let n = match value {
                            Value::Number(n) => n,
                            _ => -1.0,
                        };
                        // The index is into the items the person saw.
                        let shown = match arg(keep, 4).as_object() {
                            Some(lists) => list_values(i, lists)?.get(id).cloned().unwrap_or(Value::Undefined),
                            None => Value::Undefined,
                        };
                        if !same(&slot(i, c, ITEMS)?, &shown) {
                            return Err(error(i, "show: a list's items changed before the person's choice was taken"));
                        }
                        set_slot(i, c, SELECTION, Value::Number(n))?;
                    }
                }
                dispatch(i, c, "change", "onChange")?;
            }
            _ => return Err(Abrupt::Internal(format!("the host gave show the event {event}"))),
        }
        next(i, win)
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
        return Ok(Data::Array(vec![Data::Number(n(0)), Data::Number(n(1)), Data::Number(n(0) + n(2)), Data::Number(n(1) + n(3))]));
    }
    numbers(i, v, &["left", "top", "right", "bottom"], "bounds")
}

/// `base`'s width and height, each replaced by `over`'s when that is above 0.
fn overridden(base: Data, over: Data) -> Data {
    let Data::Array(over) = over else { return base };
    let mut out = match base {
        Data::Array(b) => b,
        _ => vec![Data::Number(-1.0), Data::Number(-1.0)],
    };
    for (o, v) in out.iter_mut().zip(over) {
        if matches!(v, Data::Number(n) if n > 0.0) {
            *o = v;
        }
    }
    Data::Array(out)
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
/// was described (`undefined` for an element that is not a list). When a getter changes a list,
/// or adds or removes a control, after it was read, the dialog is read again once; a dialog that
/// changes again throws an `Error`.
fn describe(i: &mut Interp, win: ObjId) -> JsResult<Described> {
    for _ in 0..2 {
        let mark = i.temp_roots.len();
        let r = describe_rooted(i, win);
        i.temp_roots.truncate(mark);
        if let Some(d) = r? {
            return Ok(d);
        }
    }
    Err(error(i, "show: a list's items or a container's controls changed while the dialog was read"))
}

/// A description, the elements in its order, the default and cancel elements, and the lists' items.
type Described = (Value, ObjId, f64, f64, ObjId);

/// Has each list in `order` the array of items and the selection it had when it was described?
fn lists_unchanged(i: &Interp, order: &[(ObjId, String, f64)], snapshot: &[Value], selections: &[f64]) -> JsResult<bool> {
    for (((o, ..), kept), sel) in order.iter().zip(snapshot).zip(selections) {
        if !matches!(kept, Value::Undefined) && (!same(&slot(i, *o, ITEMS)?, kept) || number_slot(i, *o, SELECTION)? != *sel) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Has each element in `order` the array of controls it had when the dialog was read?
fn tree_unchanged(i: &Interp, order: &[(ObjId, String, f64)], tree: &[Value]) -> JsResult<bool> {
    for ((o, ..), kept) in order.iter().zip(tree) {
        let now = slot(i, *o, KIDS)?;
        if !(same(&now, kept) || matches!((&now, kept), (Value::Undefined, Value::Undefined))) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn describe_rooted(i: &mut Interp, win: ObjId) -> JsResult<Option<Described>> {
    let mut order: Vec<(ObjId, String, f64)> = Vec::new();
    let mut tree = Vec::new();
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
        // Each container's array of controls as it is read; a change to it means a later getter
        // added or removed a control.
        let held = slot(i, o, KIDS)?;
        i.temp_roots.push(held.clone());
        tree.push(held);
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
    // Each list's selection as it is read; a later getter may change it.
    let mut selections = Vec::with_capacity(order.len());
    for (o, kind, parent) in &order {
        let o = *o;
        let mut listed = Value::Undefined;
        let mut selected = -1.0;
        let name_v = get(i, o, "name")?;
        let name = text_of(i, &name_v, "a control's name")?;
        let text_v = get(i, o, "text")?;
        let text = text_of(i, &text_v, "a control's text")?;
        let mut d: Vec<(String, Data)> = vec![
            ("kind".into(), Data::String(kind.clone())),
            ("id".into(), Data::Number(number_slot(i, o, ID)?)),
            ("parent".into(), Data::Number(*parent)),
            ("name".into(), Data::String(name.clone())),
            ("text".into(), Data::String(text.clone())),
        ];
        let enabled = get(i, o, "enabled")?.truthy();
        let visible = get(i, o, "visible")?.truthy();
        // Enter and Escape click a button found by its name or text only when it can be clicked.
        let clickable = kind == "button" && enabled && visible;
        names.push((clickable, name, text));
        d.push(("enabled".into(), Data::Bool(enabled)));
        d.push(("visible".into(), Data::Bool(visible)));
        // A value a getter gives is kept from collection while its own properties are read.
        let b = get_kept(i, o, "bounds")?;
        d.push(("bounds".into(), bounds(i, &b)?));
        // `size` set to a positive width or height takes the place of `preferredSize`'s.
        let preferred = get_kept(i, o, "preferredSize")?;
        let preferred = numbers(i, &preferred, &["width", "height"], "preferredSize")?;
        let set = get_kept(i, o, "size")?;
        let set = numbers(i, &set, &["width", "height"], "size")?;
        d.push(("size".into(), overridden(preferred, set)));
        for (name, field) in [("minimumSize", "min"), ("maximumSize", "max")] {
            let v = get_kept(i, o, name)?;
            d.push((field.into(), numbers(i, &v, &["width", "height"], name)?));
        }
        let a = get_kept(i, o, "alignment")?;
        d.push(("alignment".into(), alignment(i, &a)?));
        // The characters a text field or static text reserves room for in its width.
        if matches!(kind.as_str(), "edittext" | "statictext") {
            let n = get(i, o, "characters")?;
            if !matches!(n, Value::Undefined | Value::Null) {
                d.push(("characters".into(), Data::Number(number_arg(i, &n, "show: a control's characters")?)));
            }
        }
        if matches!(kind.as_str(), "dialog" | "group" | "panel") {
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
                // The dialog cannot hide what is typed, so a field meant to hide it is refused before it shows.
                if props.as_object().is_some() && i.get_value(&props, &key("noecho"))?.truthy() {
                    return Err(error(i, "show: an edit field that hides its text (noecho) is not available yet"));
                }
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
                d.push(("itemsRevision".into(), Data::Number(number_slot(i, o, ITEMS_REV)?)));
                // The item texts' getters ran above, so the selection is read after them.
                selected = number_slot(i, o, SELECTION)?;
                let sel = selection_index(i, o)?.filter(|n| *n < items.len());
                d.push(("selection".into(), sel.map_or(Data::Null, |n| Data::Number(n as f64))));
            }
            _ => {}
        }
        nodes.push(Data::Object(d));
        snapshot.push(listed);
        selections.push(selected);
    }
    let target = |i: &mut Interp, name: &str, word: &str| -> JsResult<f64> {
        let given = get(i, win, name)?;
        if let Some(n) = objects.iter().position(|o| same(o, &given)) {
            return Ok(n as f64);
        }
        if !matches!(given, Value::Undefined | Value::Null) {
            return Ok(-1.0);
        }
        let button = |by_name: bool| names.iter().position(|(c, n, t)| *c && (if by_name { n } else { t }).to_lowercase() == word);
        Ok(button(true).or_else(|| button(false)).map_or(-1.0, |n| n as f64))
    };
    let def = target(i, "defaultElement", "ok")?;
    let cancel = target(i, "cancelElement", "cancel")?;
    if !lists_unchanged(i, &order, &snapshot, &selections)? || !tree_unchanged(i, &order, &tree)? {
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
        i.reserve_heap(OBJECT_BYTES.saturating_mul(5))?;
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
        let event_proto = i.heap.alloc_object(Object::new(Class::Ordinary, Some(object_proto)));
        i.temp_roots.push(Value::Object(event_proto));
        let radio_get = native(i, "value", radio_get, 0, NativeKind::Plain, false)?;
        i.temp_roots.push(Value::Object(radio_get));
        let radio_set = native(i, "value", radio_set, 1, NativeKind::Plain, false)?;
        i.temp_roots.push(Value::Object(radio_set));
        i.script_ui = Some(Classes {
            window_proto,
            control_proto,
            item_proto,
            event_proto,
            layout,
            host,
            selection_get,
            selection_set,
            radio_get,
            radio_set,
            showing: None,
            dispatching: Vec::new(),
            above: Vec::new(),
            next_id: 0.0,
        });
        let window = native(i, "Window", window_ctor, 4, NativeKind::Plain, true)?;
        constructor(i, "Window", window, window_proto)?;
        // `Window.alert`, `Window.confirm` and `Window.prompt` are the host's global functions.
        let global = i.global;
        for name in ["alert", "confirm", "prompt"] {
            if let Some(Prop::Data { value, .. }) = i.get_own_property(global, &key(name))?
                && i.is_callable(&value)
            {
                data(i, window, name, value, HIDDEN)?;
            }
        }
        for proto in [window_proto, control_proto] {
            method(i, proto, "add", add, 4)?;
            method(i, proto, "findElement", find_element, 1)?;
            method(i, proto, "remove", remove, 1)?;
            method(i, proto, "addEventListener", add_event_listener, 3)?;
            method(i, proto, "removeEventListener", remove_event_listener, 3)?;
        }
        method(i, window_proto, "close", close, 1)?;
        method(i, window_proto, "hide", hide, 0)?;
        method(i, window_proto, "center", nothing, 1)?;
        pausable_method(i, window_proto, "show", show, 0)?;
        method(i, control_proto, "removeAll", remove_all, 0)?;
        method(i, control_proto, "find", find, 1)?;
        method(i, control_proto, "notify", notify, 1)?;
        method(i, control_proto, "show", control_show, 0)?;
        method(i, control_proto, "hide", control_hide, 0)?;
        method(i, event_proto, "stopPropagation", event_stop, 0)?;
        method(i, event_proto, "preventDefault", nothing, 0)?;
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
