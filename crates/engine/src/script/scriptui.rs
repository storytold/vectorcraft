//! ScriptUI dialogs (`new Window("dialog")`, [`vectorcraft_js::scriptui`]): the description a run
//! gives when it shows one, checked and laid out here before the app draws it, and the events the
//! person or `script.run`'s `answers` give back. A palette (`new Window("palette")`) is described
//! and laid out the same way; [`super::palettes`] keeps it and its events, and the host functions
//! `palette`, `paletteShow`, `paletteUpdate`, `paletteHide`, `paletteClose` and `paletteEvent` here
//! act for a run in a named engine, and throw [`PALETTE_NEEDS_ENGINE`] in any other.
//!
//! A description lists the dialog first and each control after the container that holds it. It is
//! refused with an error thrown at `show()` when a control's container comes after it or is not a
//! group or panel, when it holds more than [`MAX_CONTROLS`] controls, nests deeper than
//! [`MAX_DEPTH`], has more than [`MAX_ITEMS`] list items or a text longer than [`MAX_TEXT_BYTES`],
//! names an orientation or an alignment that does not exist, or gives a slider a minimum above its
//! maximum. Sizes, places, spacing and margins are clamped to [`MAX_EXTENT`] pixels, and a slider's
//! or progress bar's value to its range.
//!
//! The layout follows each container's `orientation` (row, column or stack), `alignChildren`,
//! `spacing` and `margins`, a control's `alignment`, `preferredSize` and `bounds` (left, top, right
//! and bottom in its container). A control's size comes from its text, items and kind, by
//! VectorCraft's own estimates of a 7-pixel character and a 16-pixel line. The layout is worked out
//! with loops over the list, from the last control to the first for sizes and from the first to the
//! last for places, so a description's nesting takes no native stack.
//!
//! A run with nobody to ask takes one answer from `answers` for each `show()`: `{kind: "window",
//! value: [step, ...]}`, where a step is `{control, event: "click"}`, `{control, event: "change",
//! value}` or `{event: "close"}`. `control` is a control's `name`, or else its text; `value` is an
//! edit field's text, a slider's number, or a list's item index or `null`. Each time the dialog
//! waits it takes the next step; a dialog that waits with no step left, a step for a control the
//! dialog does not hold or cannot take it, and steps left when the dialog closes end the run.
//!
//! The limits here are VectorCraft's own: [`MAX_EXTENT`], and the js crate's [`MAX_CONTROLS`],
//! [`MAX_DEPTH`], [`MAX_ITEMS`] and [`MAX_TEXT_BYTES`].

use std::rc::Rc;

use serde_json::{Map, Value, json};
use vectorcraft_js::{Data, HostError, HostItem, Realm, RunError};

pub use vectorcraft_js::scriptui::{MAX_CONTROLS, MAX_DEPTH, MAX_ITEMS, MAX_TEXT_BYTES};

use super::Host;

/// The largest size or place, in pixels, of a dialog or a control in it.
pub const MAX_EXTENT: f32 = 8192.0;
/// A character's width and a line's height in VectorCraft's size estimates, in pixels.
const CHAR: f32 = 7.0;
const LINE: f32 = 16.0;

/// A dialog's element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiKind {
    Dialog,
    Group,
    Panel,
    StaticText,
    EditText,
    Button,
    CheckBox,
    RadioButton,
    DropDownList,
    ListBox,
    Slider,
    ProgressBar,
}

impl UiKind {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "dialog" => Self::Dialog,
            "group" => Self::Group,
            "panel" => Self::Panel,
            "statictext" => Self::StaticText,
            "edittext" => Self::EditText,
            "button" => Self::Button,
            "checkbox" => Self::CheckBox,
            "radiobutton" => Self::RadioButton,
            "dropdownlist" => Self::DropDownList,
            "listbox" => Self::ListBox,
            "slider" => Self::Slider,
            "progressbar" => Self::ProgressBar,
            _ => return None,
        })
    }

    /// Does it hold controls?
    pub fn container(self) -> bool {
        matches!(self, Self::Dialog | Self::Group | Self::Panel)
    }

    /// Does a click reach it?
    pub fn clicks(self) -> bool {
        matches!(self, Self::Button | Self::CheckBox | Self::RadioButton)
    }
}

/// How a container places its controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Row,
    Column,
    Stack,
}

/// Where a control goes across its container: left or top, right or bottom, the middle, or the
/// whole width or height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    End,
    Center,
    Fill,
}

/// A control's place and size in the dialog, from its top left corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UiRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// One element of a [`ScriptUiDialog`].
#[derive(Clone, Debug, PartialEq)]
pub struct UiNode {
    pub kind: UiKind,
    /// The index of its container; the dialog's own is 0.
    pub parent: usize,
    pub name: String,
    pub text: String,
    /// It and every container around it are enabled.
    pub enabled: bool,
    /// It and every container around it are visible.
    pub visible: bool,
    /// A check box's or radio button's value.
    pub checked: bool,
    /// A slider's or progress bar's value, within `min` to `max`.
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub items: Vec<String>,
    pub selection: Option<usize>,
    /// How many times a list's items were added or removed, which a palette's waiting change of
    /// the list is checked against.
    pub revision: f64,
    pub multiline: bool,
    pub readonly: bool,
    pub rect: UiRect,
    orientation: Orientation,
    align_children: (Option<Align>, Option<Align>),
    alignment: (Option<Align>, Option<Align>),
    spacing: f32,
    margins: [f32; 4],
    size: [Option<f32>; 2],
    bounds: Option<UiRect>,
}

impl UiNode {
    /// How much of a progress bar is done, from 0 to 1.
    pub fn fraction(&self) -> f32 {
        if self.max <= self.min || self.value <= self.min {
            return 0.0;
        }
        if self.value >= self.max {
            return 1.0;
        }
        let span = self.max - self.min;
        // A range wider than the largest number is measured at half scale.
        let part =
            if span.is_finite() { (self.value - self.min) / span } else { (self.value / 2.0 - self.min / 2.0) / (self.max / 2.0 - self.min / 2.0) };
        part.clamp(0.0, 1.0) as f32
    }
}

/// A dialog a run shows, checked and laid out.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptUiDialog {
    /// The dialog first, then each control after the container that holds it.
    pub nodes: Vec<UiNode>,
    /// The button Enter clicks and the one Escape clicks.
    pub default_element: Option<usize>,
    pub cancel_element: Option<usize>,
    /// Which `show()` of the run this is, counted from 1; 0 before it shows.
    pub shown: u64,
}

impl ScriptUiDialog {
    /// The dialog's title.
    pub fn title(&self) -> &str {
        self.nodes.first().map_or("", |n| n.text.as_str())
    }

    /// The size of the dialog's content.
    pub fn size(&self) -> (f32, f32) {
        self.nodes.first().map_or((0.0, 0.0), |n| (n.rect.w, n.rect.h))
    }

    /// The first control whose name, or else whose text, is `control`.
    pub fn find(&self, control: &str) -> Option<usize> {
        let by = |name: bool| {
            self.nodes.iter().enumerate().skip(1).find(|(_, n)| if name { n.name == control } else { n.text == control }).map(|(i, _)| i)
        };
        by(true).or_else(|| by(false))
    }

    /// Where control `i` sits: its position among its container's controls, after its container's
    /// own position, and so on up to the dialog. `add` puts a new control after the others in its
    /// container, so adding controls leaves the place of every other one as it was, while its index
    /// in [`Self::nodes`] can move. Empty for the dialog and for an index it does not have.
    pub fn place_of(&self, mut i: usize) -> Vec<usize> {
        let mut place = Vec::new();
        while i > 0 {
            let Some(n) = self.nodes.get(i) else { return Vec::new() };
            place.push(self.nodes.iter().take(i).skip(1).filter(|m| m.parent == n.parent).count());
            if n.parent >= i {
                return Vec::new();
            }
            i = n.parent;
        }
        place.reverse();
        place
    }

    /// The index of the control at `place` ([`Self::place_of`]), if the dialog has one there.
    pub fn control_at(&self, place: &[usize]) -> Option<usize> {
        let mut at = 0;
        for &k in place {
            at = self.nodes.iter().enumerate().skip(1).filter(|(_, n)| n.parent == at).nth(k)?.0;
        }
        (at > 0).then_some(at)
    }
}

/// A value a control takes in a change.
#[derive(Clone, Debug, PartialEq)]
pub enum UiValue {
    Text(String),
    Number(f64),
    Index(Option<usize>),
}

/// What the person did with a dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum UiEvent {
    /// A click on a button, check box or radio button.
    Click(usize),
    /// A new value for an edit field, a slider or a list.
    Change(usize, UiValue),
    /// The dialog's close box.
    Close,
}

impl UiEvent {
    /// The event as `show()` takes it.
    pub(crate) fn data(&self) -> Data {
        let event = |e: &str| ("event".to_string(), Data::String(e.into()));
        let id = |i: usize| ("id".to_string(), Data::Number(i as f64));
        match self {
            Self::Click(i) => Data::Object(vec![event("click"), id(*i)]),
            Self::Change(i, v) => {
                let value = match v {
                    UiValue::Text(t) => Data::String(t.clone()),
                    UiValue::Number(n) => Data::Number(*n),
                    UiValue::Index(Some(k)) => Data::Number(*k as f64),
                    UiValue::Index(None) => Data::Null,
                };
                Data::Object(vec![event("change"), id(*i), ("value".into(), value)])
            }
            Self::Close => Data::Object(vec![event("close")]),
        }
    }
}

/// Checks `event` against `dialog`. → the event as the dialog takes it: a slider's value clamped
/// to its range.
pub fn check_event(dialog: &ScriptUiDialog, event: UiEvent) -> Result<UiEvent, String> {
    let id = match &event {
        UiEvent::Close => return Ok(event),
        UiEvent::Click(i) | UiEvent::Change(i, _) => *i,
    };
    let Some(n) = dialog.nodes.get(id).filter(|_| id > 0) else { return Err(format!("the dialog has no control {id}")) };
    if !n.enabled || !n.visible {
        return Err(format!("the control {} is disabled or hidden", label(n)));
    }
    match event {
        UiEvent::Click(_) if n.kind.clicks() => Ok(event),
        UiEvent::Click(_) => Err(format!("the control {} cannot be clicked", label(n))),
        UiEvent::Change(_, UiValue::Text(t)) if n.kind == UiKind::EditText && !n.readonly => {
            if t.len() > MAX_TEXT_BYTES {
                return Err("the text is longer than 16 KiB".into());
            }
            Ok(UiEvent::Change(id, UiValue::Text(t)))
        }
        UiEvent::Change(_, UiValue::Number(v)) if n.kind == UiKind::Slider && v.is_finite() => {
            Ok(UiEvent::Change(id, UiValue::Number(v.clamp(n.min, n.max))))
        }
        UiEvent::Change(_, UiValue::Index(k)) if matches!(n.kind, UiKind::DropDownList | UiKind::ListBox) => match k {
            Some(k) if k >= n.items.len() => Err(format!("the list {} has no item {k}", label(n))),
            k => Ok(UiEvent::Change(id, UiValue::Index(k))),
        },
        UiEvent::Change(..) => Err(format!("the control {} cannot take that change", label(n))),
        UiEvent::Close => Ok(UiEvent::Close),
    }
}

fn label(n: &UiNode) -> String {
    let shown = if n.name.is_empty() { &n.text } else { &n.name };
    super::dialogs::cut_to(shown, 64).to_string()
}

/// A step of a `window` answer in `script.run`'s `answers`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiStep {
    /// The control's name or text; none for a close.
    pub control: Option<String>,
    pub event: String,
    pub value: Option<Value>,
}

/// The steps of a `window` answer: `[{control, event: "click"} | {control, event: "change", value}
/// | {event: "close"}]`, where `control` is a string of at most 16 KiB and `value` a string, a
/// number or null.
pub(crate) fn parse_steps(v: &Value) -> Result<Vec<UiStep>, String> {
    let Some(list) = v.as_array() else { return Err("is a window's answer, whose `value` must be a list of steps".into()) };
    list.iter().enumerate().map(|(i, s)| step(s).ok_or_else(|| format!("has a step {i} that is not a valid click, change or close"))).collect()
}

fn step(s: &Value) -> Option<UiStep> {
    let o = s.as_object()?;
    if o.keys().any(|k| !matches!(k.as_str(), "control" | "event" | "value")) {
        return None;
    }
    let control = match o.get("control") {
        None => None,
        Some(Value::String(c)) if c.len() <= MAX_TEXT_BYTES => Some(c.clone()),
        Some(_) => return None,
    };
    let value = o.get("value").cloned();
    let event = o.get("event").and_then(Value::as_str)?;
    let fits = match event {
        "close" => control.is_none() && value.is_none(),
        "click" => control.is_some() && value.is_none(),
        "change" => control.is_some() && matches!(value, Some(Value::String(_) | Value::Number(_) | Value::Null)),
        _ => false,
    };
    fits.then(|| UiStep { control, event: event.to_string(), value })
}

/// The event `step` is for `dialog`.
pub(crate) fn step_event(dialog: &ScriptUiDialog, step: &UiStep) -> Result<UiEvent, String> {
    let Some(control) = &step.control else { return Ok(UiEvent::Close) };
    let Some(id) = dialog.find(control) else { return Err(format!("the dialog has no control named {}", super::dialogs::cut_to(control, 64))) };
    let event = match (step.event.as_str(), &step.value) {
        ("click", _) => UiEvent::Click(id),
        (_, Some(Value::String(t))) => UiEvent::Change(id, UiValue::Text(t.clone())),
        (_, Some(Value::Number(n))) if matches!(dialog.nodes[id].kind, UiKind::DropDownList | UiKind::ListBox) => match list_index(n) {
            Some(k) => UiEvent::Change(id, UiValue::Index(Some(k))),
            None => return Err("a list's `value` must be an item index or null".into()),
        },
        (_, Some(Value::Number(n))) => UiEvent::Change(id, UiValue::Number(n.as_f64().unwrap_or(f64::NAN))),
        (_, Some(Value::Null)) => UiEvent::Change(id, UiValue::Index(None)),
        _ => return Err("a change's `value` must be a string, a number or null".into()),
    };
    check_event(dialog, event)
}

/// A list's item index written as a JSON number: a whole number of 0 or more, as an integer or as
/// a float such as `1.0`; an index past the end stays past it. `None` for a fraction or a negative
/// number.
pub fn list_index(n: &serde_json::Number) -> Option<usize> {
    if let Some(k) = n.as_u64() {
        return Some(usize::try_from(k).unwrap_or(usize::MAX));
    }
    let f = n.as_f64()?;
    // `as` saturates a whole number too large for usize at usize::MAX.
    (f >= 0.0 && f.fract() == 0.0).then_some(f as usize)
}

fn field<'a>(fields: &'a [(String, Data)], name: &str) -> &'a Data {
    fields.iter().find(|(k, _)| k == name).map_or(&Data::Undefined, |(_, v)| v)
}

fn bad(what: &str) -> HostError {
    HostError::Type(format!("show: the dialog's description {what}"))
}

/// A finite number.
fn number(d: &Data) -> Option<f64> {
    match d {
        Data::Number(n) if n.is_finite() => Some(*n),
        _ => None,
    }
}

fn text(d: &Data, what: &str) -> Result<String, HostError> {
    match d {
        Data::String(s) if s.len() <= MAX_TEXT_BYTES => Ok(s.clone()),
        Data::String(_) => Err(HostError::Range(format!("show: {what} is longer than 16 KiB"))),
        _ => Err(bad(&format!("has no {what}"))),
    }
}

fn extent(n: f64) -> f32 {
    (n as f32).clamp(-MAX_EXTENT, MAX_EXTENT)
}

fn align_name(s: &str, horizontal: Option<bool>) -> Result<(Option<Align>, Option<Align>), HostError> {
    let a = match (s, horizontal) {
        ("left", Some(true) | None) => (Some(Align::Start), None),
        ("right", Some(true) | None) => (Some(Align::End), None),
        ("top", Some(false) | None) => (None, Some(Align::Start)),
        ("bottom", Some(false) | None) => (None, Some(Align::End)),
        ("center", _) => (Some(Align::Center), Some(Align::Center)),
        ("fill", _) => (Some(Align::Fill), Some(Align::Fill)),
        _ => return Err(HostError::Error(format!("show: there is no alignment {}", super::dialogs::cut_to(s, 64)))),
    };
    Ok(match horizontal {
        Some(true) => (a.0, None),
        Some(false) => (None, a.1),
        None => a,
    })
}

fn alignment(d: &Data) -> Result<(Option<Align>, Option<Align>), HostError> {
    match d {
        Data::Null | Data::Undefined => Ok((None, None)),
        Data::String(s) => align_name(s, None),
        Data::Array(a) if a.len() == 2 => {
            let part = |d: &Data, h: bool| match d {
                Data::String(s) => align_name(s, Some(h)),
                _ => Err(bad("has an alignment that is not a name")),
            };
            Ok((part(&a[0], true)?.0, part(&a[1], false)?.1))
        }
        _ => Err(bad("has an alignment that is not a name or a pair of names")),
    }
}

fn numbers<const N: usize>(d: &Data) -> Result<Option<[f64; N]>, HostError> {
    match d {
        Data::Null | Data::Undefined => Ok(None),
        Data::Array(a) if a.len() == N => {
            let mut out = [0.0; N];
            for (o, v) in out.iter_mut().zip(a) {
                *o = number(v).ok_or_else(|| bad("has a size that is not a number"))?;
            }
            Ok(Some(out))
        }
        _ => Err(bad("has a size with the wrong count of numbers")),
    }
}

/// Reads, checks and lays out a dialog's description.
pub fn parse(d: &Data) -> Result<ScriptUiDialog, HostError> {
    let Data::Object(top) = d else { return Err(bad("is not an object")) };
    let Data::Array(list) = field(top, "nodes") else { return Err(bad("has no list of controls")) };
    if list.is_empty() {
        return Err(bad("has no dialog"));
    }
    if list.len() > MAX_CONTROLS + 1 {
        return Err(HostError::Range(format!("show: a dialog can hold at most {MAX_CONTROLS} controls")));
    }
    let mut nodes: Vec<UiNode> = Vec::with_capacity(list.len());
    let mut depth: Vec<usize> = Vec::with_capacity(list.len());
    let mut items_total = 0usize;
    for (i, n) in list.iter().enumerate() {
        let Data::Object(f) = n else { return Err(bad("has a control that is not an object")) };
        let kind = match field(f, "kind") {
            Data::String(k) => UiKind::parse(k).ok_or_else(|| bad("has a control of a kind that does not exist"))?,
            _ => return Err(bad("has a control without a kind")),
        };
        let parent = number(field(f, "parent")).ok_or_else(|| bad("has a control without its container"))?;
        let parent = if i == 0 {
            if kind != UiKind::Dialog || parent != -1.0 {
                return Err(bad("does not start with the dialog"));
            }
            0
        } else {
            let p = parent as usize;
            if parent < 0.0 || parent.fract() != 0.0 || p >= i || kind == UiKind::Dialog {
                return Err(bad("has a control before its container"));
            }
            if !nodes[p].kind.container() {
                return Err(bad("has a control inside a control that holds none"));
            }
            p
        };
        let level = if i == 0 { 0 } else { depth[parent] + 1 };
        if level > MAX_DEPTH {
            return Err(HostError::Range(format!("show: controls can be nested at most {MAX_DEPTH} deep")));
        }
        depth.push(level);
        let flag = |name: &str| matches!(field(f, name), Data::Bool(true));
        let (parent_enabled, parent_visible) = if i == 0 { (true, true) } else { (nodes[parent].enabled, nodes[parent].visible) };
        let items = match field(f, "items") {
            Data::Array(a) => a.iter().map(|t| text(t, "a list item's text")).collect::<Result<Vec<_>, _>>()?,
            _ => Vec::new(),
        };
        items_total = items_total.saturating_add(items.len());
        if items_total > MAX_ITEMS {
            return Err(HostError::Range(format!("show: a dialog's lists hold at most {MAX_ITEMS} items in all")));
        }
        let selection = number(field(f, "selection")).map(|k| k as usize).filter(|k| *k < items.len());
        let (min, max) = (number(field(f, "minvalue")).unwrap_or(0.0), number(field(f, "maxvalue")).unwrap_or(100.0));
        if min > max {
            return Err(HostError::Range("show: a slider's or progress bar's minvalue is larger than its maxvalue".into()));
        }
        let orientation = match field(f, "orientation") {
            Data::String(s) if s == "row" => Orientation::Row,
            Data::String(s) if s == "column" => Orientation::Column,
            Data::String(s) if s == "stack" => Orientation::Stack,
            Data::Undefined => Orientation::Column,
            _ => return Err(HostError::Error("show: a container's orientation must be row, column or stack".into())),
        };
        let bounds = numbers::<4>(field(f, "bounds"))?.map(|[l, t, r, b]| {
            let (l, t) = (extent(l), extent(t));
            UiRect { x: l, y: t, w: (extent(r) - l).clamp(0.0, MAX_EXTENT), h: (extent(b) - t).clamp(0.0, MAX_EXTENT) }
        });
        let size = numbers::<2>(field(f, "size"))?.map_or([None, None], |[w, h]| [(w > 0.0).then(|| extent(w)), (h > 0.0).then(|| extent(h))]);
        let margins = numbers::<4>(field(f, "margins"))?.map_or([0.0; 4], |m| m.map(|v| extent(v).max(0.0)));
        nodes.push(UiNode {
            kind,
            parent,
            name: text(field(f, "name"), "a control's name")?,
            text: text(field(f, "text"), "a control's text")?,
            enabled: flag("enabled") && parent_enabled,
            visible: flag("visible") && parent_visible,
            checked: flag("value"),
            value: number(field(f, "value")).unwrap_or(0.0).clamp(min, max),
            min,
            max,
            items,
            selection,
            revision: number(field(f, "revision")).unwrap_or(0.0),
            multiline: flag("multiline"),
            readonly: flag("readonly"),
            rect: UiRect::default(),
            orientation,
            align_children: alignment(field(f, "alignChildren"))?,
            alignment: alignment(field(f, "alignment"))?,
            spacing: number(field(f, "spacing")).map_or(0.0, |s| extent(s).max(0.0)),
            margins,
            size,
            bounds,
        });
    }
    let target = |name: &str| number(field(top, name)).map(|k| k as usize).filter(|k| nodes.get(*k).is_some_and(|n| n.kind == UiKind::Button));
    let (default_element, cancel_element) = (target("defaultElement"), target("cancelElement"));
    let mut dialog = ScriptUiDialog { nodes, default_element, cancel_element, shown: 0 };
    layout(&mut dialog);
    Ok(dialog)
}

/// A control's own size, by VectorCraft's estimates.
fn own_size(n: &UiNode) -> [f32; 2] {
    let chars = |s: &str| s.chars().count() as f32;
    let widest = |lines: &mut dyn Iterator<Item = &str>| lines.map(chars).fold(0.0, f32::max);
    let lines = |s: &str| s.lines().count().max(1) as f32;
    let longest_item = n.items.iter().map(|t| chars(t)).fold(0.0, f32::max);
    match n.kind {
        UiKind::StaticText => [widest(&mut n.text.lines()) * CHAR + 4.0, lines(&n.text) * LINE + 4.0],
        UiKind::Button => [(chars(&n.text) * CHAR + 24.0).max(80.0), 24.0],
        UiKind::CheckBox | UiKind::RadioButton => [chars(&n.text) * CHAR + 24.0, 20.0],
        UiKind::EditText if n.multiline => [(widest(&mut n.text.lines()) * CHAR + 12.0).max(160.0), lines(&n.text).max(3.0) * LINE + 10.0],
        UiKind::EditText => [(chars(&n.text) * CHAR + 12.0).max(80.0), 24.0],
        UiKind::DropDownList => [(longest_item * CHAR + 40.0).max(100.0), 24.0],
        UiKind::ListBox => [(longest_item * CHAR + 24.0).max(120.0), (n.items.len() as f32).clamp(3.0, 10.0) * 18.0 + 6.0],
        UiKind::Slider => [160.0, 20.0],
        UiKind::ProgressBar => [160.0, 12.0],
        UiKind::Dialog | UiKind::Group | UiKind::Panel => [0.0, 0.0],
    }
}

/// Lays the dialog out: each control's size from the last to the first, then each place from the
/// first to the last.
fn layout(d: &mut ScriptUiDialog) {
    let n = d.nodes.len();
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, node) in d.nodes.iter().enumerate().skip(1) {
        kids[node.parent].push(i);
    }
    let mut size = vec![[0.0f32; 2]; n];
    for i in (0..n).rev() {
        let node = &d.nodes[i];
        let mut s = if node.kind.container() {
            let [l, t, r, b] = node.margins;
            let (mut main, mut cross, mut count, mut reach) = (0.0f32, 0.0f32, 0usize, [0.0f32; 2]);
            for &k in &kids[i] {
                if let Some(bounds) = d.nodes[k].bounds {
                    reach = [reach[0].max(bounds.x + bounds.w), reach[1].max(bounds.y + bounds.h)];
                    continue;
                }
                let [w, h] = size[k];
                count += 1;
                match node.orientation {
                    Orientation::Row => (main, cross) = (main + w, cross.max(h)),
                    Orientation::Column => (main, cross) = (main + h, cross.max(w)),
                    Orientation::Stack => (main, cross) = (main.max(w), cross.max(h)),
                }
            }
            if node.orientation != Orientation::Stack {
                main += node.spacing * count.saturating_sub(1) as f32;
            }
            let (w, h) = match node.orientation {
                Orientation::Row | Orientation::Stack => (main, cross),
                Orientation::Column => (cross, main),
            };
            [(w + l + r).max(reach[0]), (h + t + b).max(reach[1])]
        } else {
            own_size(node)
        };
        for axis in 0..2 {
            if let Some(v) = node.size[axis] {
                s[axis] = v;
            }
        }
        if let Some(b) = node.bounds {
            s = [b.w, b.h];
        }
        size[i] = [s[0].clamp(0.0, MAX_EXTENT), s[1].clamp(0.0, MAX_EXTENT)];
    }
    d.nodes[0].rect = UiRect { x: 0.0, y: 0.0, w: size[0][0], h: size[0][1] };
    for i in 0..n {
        if !d.nodes[i].kind.container() {
            continue;
        }
        let node = d.nodes[i].clone();
        let [l, t, r, b] = node.margins;
        let (x, y) = (node.rect.x + l, node.rect.y + t);
        let (cw, ch) = ((node.rect.w - l - r).max(0.0), (node.rect.h - t - b).max(0.0));
        let mut cursor = 0.0f32;
        for &k in &kids[i] {
            let kid = &d.nodes[k];
            let rect = if let Some(bounds) = kid.bounds {
                UiRect { x: node.rect.x + bounds.x, y: node.rect.y + bounds.y, w: bounds.w, h: bounds.h }
            } else {
                let [w, h] = size[k];
                let h_align = kid.alignment.0.or(node.align_children.0).unwrap_or(Align::Center);
                let v_align = kid.alignment.1.or(node.align_children.1).unwrap_or(Align::Center);
                let place = |align: Align, own: f32, room: f32| match align {
                    Align::Start => (0.0, own),
                    Align::End => ((room - own).max(0.0), own),
                    Align::Center => (((room - own) / 2.0).max(0.0), own),
                    Align::Fill => (0.0, room.max(own)),
                };
                match node.orientation {
                    Orientation::Row => {
                        let (dy, kh) = place(v_align, h, ch);
                        let r = UiRect { x: x + cursor, y: y + dy, w, h: kh };
                        cursor += w + node.spacing;
                        r
                    }
                    Orientation::Column => {
                        let (dx, kw) = place(h_align, w, cw);
                        let r = UiRect { x: x + dx, y: y + cursor, w: kw, h };
                        cursor += h + node.spacing;
                        r
                    }
                    Orientation::Stack => {
                        let (dx, kw) = place(h_align, w, cw);
                        let (dy, kh) = place(v_align, h, ch);
                        UiRect { x: x + dx, y: y + dy, w: kw, h: kh }
                    }
                }
            };
            d.nodes[k].rect = UiRect {
                x: rect.x.clamp(-MAX_EXTENT, MAX_EXTENT),
                y: rect.y.clamp(-MAX_EXTENT, MAX_EXTENT),
                w: rect.w.clamp(0.0, MAX_EXTENT),
                h: rect.h.clamp(0.0, MAX_EXTENT),
            };
        }
    }
}

/// The dialog of one `show()`: the steps left for a run with nobody to ask, and how many events it
/// took.
pub(crate) struct Showing {
    pub(crate) dialog: ScriptUiDialog,
    pub(crate) steps: Option<std::collections::VecDeque<UiStep>>,
    pub(crate) events: u64,
}

impl Showing {
    /// The report's record of the dialog, with its `answer`: the result of `show()`, `"stop"`,
    /// `"missing"` or `"error"`.
    pub(crate) fn record(&self, answer: Value) -> Value {
        let mut r = Map::new();
        r.insert("kind".into(), json!("window"));
        r.insert("title".into(), json!(super::dialogs::cut_to(self.dialog.title(), super::dialogs::MAX_TITLE_BYTES)));
        r.insert("controls".into(), json!(self.dialog.nodes.len().saturating_sub(1)));
        r.insert("events".into(), json!(self.events));
        r.insert("answer".into(), answer);
        Value::Object(r)
    }
}

/// The host function `name` of `Window`, for the run `link` holds.
fn function(link: &super::Link, f: fn(&Host, &[Data]) -> Result<Data, HostError>) -> HostItem {
    let l = link.clone();
    HostItem::Function(Rc::new(move |a| f(&*super::current(&l)?, &a)))
}

/// Adds `Window` to the realm, for the run `link` holds.
pub(super) fn install(realm: &mut Realm, link: &super::Link) -> Result<(), RunError> {
    realm.define_script_ui(vec![
        ("open".into(), function(link, open)),
        ("wait".into(), function(link, wait)),
        ("end".into(), function(link, end)),
        ("palette".into(), function(link, palette)),
        ("paletteShow".into(), function(link, palette_show)),
        ("paletteUpdate".into(), function(link, palette_update)),
        ("paletteHide".into(), function(link, palette_hide)),
        ("paletteClose".into(), function(link, palette_close)),
        ("paletteEvent".into(), function(link, palette_event)),
    ])
}

/// The error of `new Window("palette")` in a run without a named engine.
pub const PALETTE_NEEDS_ENGINE: &str = "Window: a palette needs a named engine, which keeps its controls and handlers after the script ends; put a #targetengine line with a name other than main at the top of the script";

/// The named engine of the run, for a palette.
fn palette_engine(host: &Host) -> Result<&str, HostError> {
    host.engine.as_deref().ok_or_else(|| HostError::Error(PALETTE_NEEDS_ENGINE.into()))
}

/// A palette's id as a script holds it: a whole number from 1, or none.
fn palette_id(d: Option<&Data>) -> Option<u64> {
    match d {
        Some(Data::Number(n)) if *n >= 1.0 && n.fract() == 0.0 && *n < 9.0e15 => Some(*n as u64),
        _ => None,
    }
}

/// `new Window("palette")`: refused in a run without a named engine.
fn palette(host: &Host, _: &[Data]) -> Result<Data, HostError> {
    palette_engine(host)?;
    Ok(Data::Undefined)
}

/// A palette's `show()`, with its description and its id (none the first time). → its id.
fn palette_show(host: &Host, args: &[Data]) -> Result<Data, HostError> {
    let engine = palette_engine(host)?.to_string();
    let dialog = described(host, args)?;
    let id = palette_id(args.get(1));
    let grants = host.disk.borrow().own().to_string();
    let shown = host.session.borrow_mut().script_palettes.show(id, &engine, (host.file.clone(), grants), dialog);
    shown.map(|id| Data::Number(id as f64)).map_err(HostError::Range)
}

/// The palette `id`'s controls as a handler or `add()` left them.
fn palette_update(host: &Host, args: &[Data]) -> Result<Data, HostError> {
    let engine = palette_engine(host)?.to_string();
    let dialog = described(host, args)?;
    if let Some(id) = palette_id(args.get(1)) {
        host.session.borrow_mut().script_palettes.update(id, &engine, dialog);
    }
    Ok(Data::Undefined)
}

fn palette_hide(host: &Host, args: &[Data]) -> Result<Data, HostError> {
    let engine = palette_engine(host)?.to_string();
    if let Some(id) = palette_id(args.first()) {
        host.session.borrow_mut().script_palettes.hide(id, &engine);
    }
    Ok(Data::Undefined)
}

fn palette_close(host: &Host, args: &[Data]) -> Result<Data, HostError> {
    let engine = palette_engine(host)?.to_string();
    if let Some(id) = palette_id(args.first()) {
        host.session.borrow_mut().script_palettes.close(id, &engine);
    }
    Ok(Data::Undefined)
}

/// The event a palette handler's run is for, once: `{palette, event, id, value, place, items}`, as
/// `show()` takes a dialog's event with the palette's id added, the place of a click's or a
/// change's control ([`ScriptUiDialog::place_of`]), which the run finds the control by, and for a
/// list's change the items the palette showed; `null` for any other run, and after the first call.
fn palette_event(host: &Host, _: &[Data]) -> Result<Data, HostError> {
    let Some(queued) = host.palette_event.borrow_mut().take() else { return Ok(Data::Null) };
    let Data::Object(mut fields) = queued.event.data() else { return Ok(Data::Null) };
    fields.push(("palette".into(), Data::Number(queued.palette as f64)));
    if !queued.place.is_empty() {
        fields.push(("place".into(), Data::Array(queued.place.iter().map(|&k| Data::Number(k as f64)).collect())));
    }
    if let Some((items, revision)) = queued.list {
        fields.push(("items".into(), Data::Array(items.into_iter().map(Data::String).collect())));
        fields.push(("revision".into(), Data::Number(revision)));
    }
    Ok(Data::Object(fields))
}

fn described(host: &Host, args: &[Data]) -> Result<ScriptUiDialog, HostError> {
    host.charge_call(args.iter().map(super::data_size).sum())?;
    parse(args.first().unwrap_or(&Data::Undefined))
}

fn open(host: &Host, args: &[Data]) -> Result<Data, HostError> {
    let dialog = described(host, args)?;
    host.dialogs.window_open(dialog)?;
    Ok(Data::Undefined)
}

fn wait(host: &Host, args: &[Data]) -> Result<Data, HostError> {
    let dialog = described(host, args)?;
    host.dialogs.window_wait(dialog)
}

fn end(host: &Host, args: &[Data]) -> Result<Data, HostError> {
    let result = number(args.first().unwrap_or(&Data::Undefined)).unwrap_or(0.0);
    let normal = matches!(args.get(1), Some(Data::Bool(true)));
    host.dialogs.window_end(result, normal)?;
    Ok(Data::Undefined)
}
