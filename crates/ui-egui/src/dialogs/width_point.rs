//! Width Point Edit: double-clicking a stroke with the Width tool (or `ui.widthPointEdit`)
//! creates or edits a point and its side widths. Total Width changes both sides in proportion,
//! the link keeps them equal,
//! and Adjust Adjoining Width Points changes the nearest points either side in proportion too.
//! Shift-double-click on a width point opens the dialog with that option enabled.
//! OK runs `stroke.widthPoint.set`; Delete removes an existing point (`stroke.widthPoint.remove`)
//! or discards a prospective point without changing the document.
//!
//! On either half of a discontinuous point the dialog shows both sets of side widths, Incoming
//! (before the point along the path) and Outgoing (after it), and Single Width Only keeps one of
//! them as a continuous point. OK sets both halves in one `stroke.widthPoint.set` (one undo step),
//! and Delete removes both halves.
//!
//! Fields: `id`, `index` (an existing width point, otherwise null; the incoming half of a
//! discontinuous point), `t` (its place), `side1`, `side2` (the left and right widths, points),
//! `linked`, `adjustAdjoining`; for a discontinuous point also `outIndex`, `outSide1`, `outSide2`,
//! `outLinked` (the outgoing half) and `single` (`""`, `"incoming"` or `"outgoing"`). A side width
//! set through `ui.dialog.set` applies as typing it does, so with `linked` on it sets both sides.
//! Delete is `ui.dialog.confirm {choice: "delete"}`.
//!
//! Opened on several selected width points (`indices`), the dialog edits all of them. Side 1,
//! Side 2 and Total Width show a value only where every point has it, and are mixed otherwise. A
//! side width entered sets that side of every point (both sides with the link on) and leaves the
//! other side of each point as it was; Total Width scales each point's own two sides to that total.
//! Turning the link on changes no width. When the points include a discontinuous one, Incoming
//! edits the incoming halves and Outgoing the outgoing halves, and both edit the continuous points.
//! With Adjust Adjoining Width Points, each changed point changes the nearest unselected point
//! before and after it in proportion, measured from the widths the points had when the dialog
//! opened. OK sets them all in one `stroke.widthProfile.set` (one undo step, none when nothing
//! changed), and Delete removes them all. The fields are `id`, `indices` (every selected profile
//! entry, both halves of a discontinuous point), `side1`, `side2`, `totalWidth` (null when the
//! points differ), `linked`, `adjustAdjoining`, and with a discontinuous point `outSide1`,
//! `outSide2`, `outTotalWidth` and `outLinked`.
//!
//! OK and Delete on existing points, one or several, are refused when those points changed since
//! the dialog opened or another document is active; the dialog stays open and says why.

// PLAN: the app's manifest depends on `craft-ui`, not on the `craft` facade, so the `craft::` paths below need hub-w2's manifest change.
use craft::behavior::Mixed;
use craft::theme::variant::Tone;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::layout_components::Row;
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::{FormDialog, ResetFooter, ResetPick};
use craft::ui::runtime::env::{Label, LengthUnits};
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::text::{NumberField, link_toggle};
use serde_json::{Value, json};
use vectorcraft_doc::{Unit, WidthProfile};
use vectorcraft_engine::doc::NodeId;
use vectorcraft_engine::width_point_pair;

use crate::VectorcraftApp;
use crate::canvas::stroke_lengths;

/// The dialog kind of Width Point Edit.
pub const KIND: &str = "widthPoint";

/// Width points closer than this (in t) are at the same place, as the engine counts them.
const SAME_T: f64 = 1e-6;

/// Side widths (points) closer than this show as one value.
const SAME_WIDTH: f64 = 1e-6;

/// The fields of each set of side widths: Side 1, Side 2, Total Width and the link, for the incoming
/// set and the outgoing set.
const SET_KEYS: [[&str; 4]; 2] = [["side1", "side2", "totalWidth", "linked"], ["outSide1", "outSide2", "outTotalWidth", "outLinked"]];

/// The set (0 incoming, 1 outgoing) and the place in [`SET_KEYS`] that the field `field` names.
fn locate(field: &str) -> Option<(usize, usize)> {
    SET_KEYS.iter().enumerate().find_map(|(set, keys)| keys.iter().position(|key| *key == field).map(|at| (set, at)))
}

/// `text` as a label.
fn word(text: &str) -> Label {
    Label::from(text.to_owned())
}

/// Width point `index` of `id`: its place and side widths (points).
fn point(app: &VectorcraftApp, id: u64, index: usize) -> Option<(f64, f64, f64)> {
    let st = app.session.active()?.doc.node(NodeId(id))?.appearance.stroke()?;
    let half = st.width / 2.0;
    st.profile.as_ref()?.points.get(index).map(|&(t, l, r)| (t, l * half, r * half))
}

/// Width point `index` of `id` as [`point`] reads it, as an array.
fn entry_at(app: &VectorcraftApp, id: u64, index: usize) -> Option<[f64; 3]> {
    point(app, id, index).map(|(t, l, r)| [t, l, r])
}

/// The incoming and outgoing halves of the discontinuous point that width point `index` of `id`
/// belongs to, or none when it is a continuous point. The halves are paired by
/// [`width_point_pair`], as `stroke.widthPoint.set` pairs them, so either half finds the same pair.
fn halves(app: &VectorcraftApp, id: u64, index: usize) -> Option<(usize, usize)> {
    let points = &app.session.active()?.doc.node(NodeId(id))?.appearance.stroke()?.profile.as_ref()?.points;
    points.get(index)?;
    let pair = |a: usize| width_point_pair(points, a).map(|b| (a, b));
    pair(index).or_else(|| index.checked_sub(1).and_then(pair))
}

/// One of several width points being edited: its profile index, its place, its side widths
/// (points) and the sets of side widths that edit it, `[incoming, outgoing]` (both for a continuous
/// point).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Entry {
    index: usize,
    t: f64,
    left: f64,
    right: f64,
    sets: [bool; 2],
}

/// What set `set` of `es` shows: Side 1, Side 2 and Total Width, each none where the points differ.
fn shown(es: &[Entry], set: usize) -> [Option<f64>; 3] {
    let common = |f: fn(&Entry) -> f64| {
        let mut values = es.iter().filter(|e| e.sets[set]).map(f);
        let first = values.next()?;
        values.all(|v| (v - first).abs() <= SAME_WIDTH).then_some(first)
    };
    [common(|e| e.left), common(|e| e.right), common(|e| e.left + e.right)]
}

/// A change entered into one set of side widths.
#[derive(Clone, Copy, Debug)]
enum Edit {
    Side1(f64),
    Side2(f64),
    Total(f64),
}

/// The side widths `left` and `right` after `edit`: a side width replaces that side (both sides with
/// `linked`), and Total Width scales the two sides to it (equal halves from nothing).
fn edited_pair(left: f64, right: f64, edit: Edit, linked: bool) -> (f64, f64) {
    let (l, r) = match edit {
        Edit::Side1(v) => (v, if linked { v } else { right }),
        Edit::Side2(v) => (if linked { v } else { left }, v),
        Edit::Total(v) => {
            let sum = left + right;
            if sum > 1e-9 { (left / sum * v, right / sum * v) } else { (v / 2.0, v / 2.0) }
        }
    };
    (l.max(0.0), r.max(0.0))
}

/// Apply `edit` to every point that set `set` edits.
fn apply(es: &mut [Entry], set: usize, edit: Edit, linked: bool) {
    for e in es.iter_mut().filter(|e| e.sets[set]) {
        (e.left, e.right) = edited_pair(e.left, e.right, edit, linked);
    }
}

/// The largest side width or Total Width a value entered can set (points).
const MAX_WIDTH: f64 = 1.0e5;

/// The width a value set through automation names: a number or a length string read in points, from
/// nothing up to [`MAX_WIDTH`].
fn width(value: &Value) -> Option<f64> {
    let v = match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => Unit::Points.parse(s),
        _ => None,
    };
    v.filter(|v| v.is_finite() && (0.0..=MAX_WIDTH).contains(v))
}

/// Width `v` of a point next to one whose width changes from `o` to `n`: in proportion, or by as
/// much from nothing, as `stroke.widthPoint.set` changes adjoining points.
fn scale(v: f64, o: f64, n: f64) -> f64 {
    let width = if o > 1e-9 {
        let product = v * n;
        if product.is_finite() { product / o } else { (v.max(n) / o) * v.min(n) }
    } else {
        v + n - o
    };
    width.max(0.0)
}

/// What the width points `indices` (a JSON array) of `id` name.
enum Resolved {
    /// One point, or the two halves of one discontinuous point: its first profile index.
    One(usize),
    /// Several places: every profile entry, in order.
    Several(Vec<Entry>),
}

/// The outgoing half that pairs with each width point, as [`width_point_pair`] pairs them: point
/// `a` and the next one are the two halves of a discontinuous point when they are the only points
/// within [`SAME_T`] of `a`'s place. Each point counts the points at its place in the sorted
/// places, stopping at three.
fn pairs(points: &[(f64, f64, f64)]) -> Vec<Option<usize>> {
    let mut ts: Vec<f64> = points.iter().map(|q| q.0).filter(|t| !t.is_nan()).collect();
    ts.sort_by(f64::total_cmp);
    let near = |a: usize, b: usize| (points[b].0 - points[a].0).abs() <= SAME_T;
    (0..points.len())
        .map(|a| {
            if a + 1 >= points.len() || !near(a, a + 1) || (a > 0 && near(a, a - 1)) {
                return None;
            }
            let t = points[a].0;
            let from = ts.partition_point(|&s| s < t - 2.0 * SAME_T);
            let at = ts[from..].iter().take_while(|&&s| s <= t + 2.0 * SAME_T).filter(|&&s| (s - t).abs() <= SAME_T).take(3).count();
            (at == 2).then_some(a + 1)
        })
        .collect()
}

/// The width points `indices` of `id` names, each discontinuous point's two halves together.
fn target(app: &VectorcraftApp, id: u64, indices: &Value) -> Result<Resolved, String> {
    const NONE: &str = "no such width point";
    let st = app.session.active().and_then(|s| s.doc.node(NodeId(id))).and_then(|n| n.appearance.stroke()).ok_or(NONE)?;
    let points = st.profile.as_ref().map_or(&[][..], |p| &p.points[..]);
    let list = indices.as_array().filter(|a| !a.is_empty()).ok_or(NONE)?;
    let mut picked = list
        .iter()
        .map(|v| v.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|i| *i < points.len()))
        .collect::<Option<Vec<_>>>()
        .ok_or(NONE)?;
    picked.sort_unstable();
    picked.dedup();
    // The other half of each discontinuous point, paired as `stroke.widthPoint.set` pairs them.
    let pairs = pairs(points);
    let outgoing = |i: usize| i.checked_sub(1).is_some_and(|a| pairs[a] == Some(i));
    let halves: Vec<usize> = picked.iter().flat_map(|&i| [pairs[i], i.checked_sub(1).filter(|_| outgoing(i))]).flatten().collect();
    picked.extend(halves);
    picked.sort_unstable();
    picked.dedup();
    // One point, or the two halves of one discontinuous point.
    if picked.len() == 1 || (picked.len() == 2 && pairs[picked[0]] == Some(picked[1])) {
        return Ok(Resolved::One(picked[0]));
    }
    if !(st.width.is_finite() && st.width > 0.0) {
        return Err("object has no weighted stroke".into());
    }
    let half = st.width / 2.0;
    let entry = |i: usize| {
        let (t, l, r) = points[i];
        Entry { index: i, t, left: l * half, right: r * half, sets: [!outgoing(i), pairs[i].is_none()] }
    };
    Ok(Resolved::Several(picked.into_iter().map(entry).collect()))
}

/// The two side widths of one set (points) and whether the link keeps them equal.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sides {
    left: f64,
    right: f64,
    linked: bool,
}

impl Sides {
    /// The widths `left` and `right`, linked when they are equal.
    fn of(left: f64, right: f64) -> Self {
        Sides { left, right, linked: (left - right).abs() < 1e-9 }
    }

    /// Applies `edit` as entering it in the field does.
    fn edit(&mut self, edit: Edit) {
        (self.left, self.right) = edited_pair(self.left, self.right, edit, self.linked);
    }

    /// Turns the link on or off; on, Side 2 takes Side 1's width.
    fn link(&mut self, on: bool) {
        self.linked = on;
        if on {
            self.right = self.left;
        }
    }
}

/// Which half of a discontinuous point Single Width Only keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Single {
    /// Both halves stay.
    #[default]
    None,
    /// The incoming half becomes the point.
    Incoming,
    /// The outgoing half becomes the point.
    Outgoing,
}

impl Single {
    /// The name `stroke.widthPoint.set` and automation use; empty for none.
    fn name(self) -> &'static str {
        match self {
            Single::None => "",
            Single::Incoming => "incoming",
            Single::Outgoing => "outgoing",
        }
    }

    /// The choice a name stands for.
    fn named(name: &str) -> Option<Self> {
        [Single::None, Single::Incoming, Single::Outgoing].into_iter().find(|single| single.name() == name)
    }
}

/// What the dialog edits.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    /// One point, or the two halves of one discontinuous point.
    One {
        /// The profile index of the (incoming) half; none for a prospective point.
        index: Option<usize>,
        /// The point's place.
        t: f64,
        /// The place and side widths of the existing halves when the dialog opened.
        at: Vec<[f64; 3]>,
        /// The incoming set.
        sides: Sides,
        /// The outgoing half's index and set.
        out: Option<(usize, Sides)>,
        /// Single Width Only.
        single: Single,
    },
    /// Several points.
    Several {
        /// The points as they were when the dialog opened.
        open: Vec<Entry>,
        /// The working widths.
        points: Vec<Entry>,
        /// Whether a discontinuous point is among them (two sets shown).
        discontinuous: bool,
        /// The link of each set.
        linked: [bool; 2],
    },
}

impl Target {
    /// How many sets of side widths the dialog shows.
    fn sets(&self) -> usize {
        match self {
            Target::One { out, .. } => 1 + usize::from(out.is_some()),
            Target::Several { discontinuous, .. } => 1 + usize::from(*discontinuous),
        }
    }
}

/// The values the dialog edits, which is what OK and Delete answer on.
#[derive(Clone, Debug, PartialEq)]
struct Form {
    /// The path whose stroke carries the points.
    path: u64,
    /// The uid of the document the dialog opened in.
    doc: Option<u64>,
    /// Adjust Adjoining Width Points.
    adjust: bool,
    target: Target,
}

impl Default for Form {
    fn default() -> Self {
        Form {
            path: 0,
            doc: None,
            adjust: false,
            target: Target::Several { open: Vec::new(), points: Vec::new(), discontinuous: false, linked: [false; 2] },
        }
    }
}

/// The button that was pressed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Pressed {
    #[default]
    Ok,
    Delete,
}

/// A held press with the values it was pressed on.
#[derive(Clone, Debug)]
struct Filed {
    pressed: Pressed,
    form: Form,
}

/// The Width Point Edit dialog on the window's dialog host.
pub(crate) struct WidthPointDialog {
    id: DialogId,
    form: Form,
    /// Stored in points, shown in the stroke unit.
    lengths: LengthUnits,
    /// The press OK or Delete filed, until the app has answered it.
    held: HeldAnswer<Filed>,
    /// The answer automation's `choice` names for the next confirm.
    choice: Pressed,
}

impl WidthPointDialog {
    fn new(form: Form, lengths: LengthUnits) -> Self {
        WidthPointDialog { id: dialog_id(KIND), form, lengths, held: HeldAnswer::new(), choice: Pressed::Ok }
    }
}

/// One set of side widths: Side 1 with its link, Side 2 and Total Width, showing `values` (none is
/// mixed). Returns the edit entered and the link's new state.
fn set_rows(ui: &mut egui::Ui, title: &Label, lengths: LengthUnits, values: [Option<f64>; 3], linked: bool) -> (Option<Edit>, Option<bool>) {
    let (mut edit, mut link) = (None, None);
    let field = |ui: &mut egui::Ui, caption: &Label, value: Option<f64>| {
        let bind = match value {
            Some(v) => Bind::<Option<f64>>::given(Some(v)),
            None => Bind::<Option<f64>>::given(Mixed::Mixed),
        };
        NumberField::length(caption.clone(), bind, lengths).range(0.0..=MAX_WIDTH).show(ui).value().copied().flatten()
    };
    FieldGroup::new(title.clone()).show(ui, |ui, _| {
        let (side1, side2, total) = (word(tl!("Side 1")), word(tl!("Side 2")), word(tl!("Total Width")));
        Field::new(side1.clone()).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                if let Some(v) = field(ui, &side1, values[0]) {
                    edit = Some(Edit::Side1(v));
                }
                let toggle = link_toggle(word(tl!("Keep both sides the same width")), Bind::given(linked)).show(ui);
                link = toggle.value().copied();
            });
        });
        Field::new(side2.clone()).show(ui, |ui, _| {
            if let Some(v) = field(ui, &side2, values[1]) {
                edit = Some(Edit::Side2(v));
            }
        });
        Field::new(total.clone()).show(ui, |ui, _| {
            if let Some(v) = field(ui, &total, values[2]) {
                edit = Some(Edit::Total(v));
            }
        });
    });
    (edit, link)
}

/// The rows of the dialog; whether the values can be accepted.
fn body(ui: &mut egui::Ui, form: &mut Form, lengths: LengthUnits, refusal: Option<&Label>) -> bool {
    let titles = if form.target.sets() > 1 { [word(tl!("Incoming")), word(tl!("Outgoing"))] } else { [word(tl!("Width Point Edit")), Label::Empty] };
    match &mut form.target {
        Target::One { sides, out, single, .. } => {
            let rows = |ui: &mut egui::Ui, title: &Label, sides: &mut Sides| {
                let values = [Some(sides.left), Some(sides.right), Some(sides.left + sides.right)];
                let (edit, link) = set_rows(ui, title, lengths, values, sides.linked);
                if let Some(edit) = edit {
                    sides.edit(edit);
                }
                if let Some(on) = link {
                    sides.link(on);
                }
            };
            match out {
                Some((_, out)) => {
                    crate::panels::section(titles[0].clone()).show(ui, |ui, _| rows(ui, &titles[0], sides));
                    crate::panels::section(titles[1].clone()).show(ui, |ui, _| rows(ui, &titles[1], out));
                    crate::panels::section(word(tl!("Single Width Only"))).show(ui, |ui, _| {
                        RadioGroup::new(word(tl!("Single Width Only")), Bind::bound(single)).show(ui, |ui, _| {
                            Row::new().show(ui, |ui, _| {
                                for (choice, text) in
                                    [(Single::None, tl!("None")), (Single::Incoming, tl!("Incoming")), (Single::Outgoing, tl!("Outgoing"))]
                                {
                                    Radio::new(choice).label(word(text)).show(ui);
                                }
                            });
                        });
                    });
                }
                None => rows(ui, &titles[0], sides),
            }
        }
        Target::Several { points, discontinuous, linked, .. } => {
            let mut rows = |ui: &mut egui::Ui, set: usize| {
                let (edit, link) = set_rows(ui, &titles[set], lengths, shown(points, set), linked[set]);
                if let Some(edit) = edit {
                    apply(points, set, edit, linked[set]);
                }
                if let Some(on) = link {
                    linked[set] = on;
                }
            };
            if *discontinuous {
                crate::panels::section(titles[0].clone()).show(ui, |ui, _| rows(ui, 0));
                crate::panels::section(titles[1].clone()).show(ui, |ui, _| rows(ui, 1));
            } else {
                rows(ui, 0);
            }
        }
    }
    let adjust = Checkbox::new(Bind::given(form.adjust)).label(word(tl!("Adjust Adjoining Width Points"))).show(ui);
    if let Some(on) = adjust.value().copied() {
        form.adjust = on;
    }
    if let Some(reason) = refusal {
        Notice::new(Tone::Bad, reason.clone()).layout(NoticeLayout::Inline).show(ui);
    }
    true
}

impl StackDialog for WidthPointDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    /// Shows the dialog; OK and Delete file their press for [`pump`], which answers it. A press the
    /// app accepted closes the dialog in the pass that follows.
    fn show(&mut self, ui: &mut egui::Ui, _stack: &DialogStack) -> Option<DialogResult<()>> {
        let finish = self.held.is_accepted();
        let waiting = self.held.is_waiting();
        // The footer `FormDialog` builds for this pass, made the same way, so its `pick` reads `FormOutput::picked`.
        let footer = ResetFooter::new(ui, None, false);
        let refusal = self.held.refusal().filter(|(_, filed)| filed.form == self.form).map(|(reason, _)| reason.clone());
        let lengths = self.lengths;
        let shown = FormDialog::new(word(tl!("Width Point Edit")), Bind::bound(&mut self.form))
            .alt_resets(false)
            .accept_stays_open(true)
            .ok_enabled(!waiting)
            .extra_buttons([FooterButton::new(word(tl!("Delete")), DialogRole::Other).enabled(!waiting)])
            .show(ui, |ui, cx, form| {
                // A measuring run closes nothing: the real run of the pass makes the close.
                if finish && !craft::ui::runtime::measure::active(ui.ctx()) {
                    cx.request_close_as(CloseReason::Activated);
                }
                body(ui, form, lengths, refusal.as_ref())
            });
        // `picked` is this pass's press: a second pass of the frame has none, so a press is filed once.
        let pressed = match footer.pick(shown.picked) {
            Some(ResetPick::Accept) => Some(Pressed::Ok),
            Some(ResetPick::Extra(0)) => Some(Pressed::Delete),
            _ => None,
        };
        if let Some(pressed) = pressed.filter(|_| !waiting) {
            self.held.file(Filed { pressed, form: self.form.clone() });
        }
        if self.held.refusal().is_some_and(|(_, filed)| filed.form != self.form) {
            self.held.clear_refusal();
        }
        match shown.closed {
            Some(CloseReason::Activated) if finish => Some(self.held.finish().map_or(DialogResult::Cancel, |_| DialogResult::Ok(()))),
            Some(_) => Some(DialogResult::Cancel),
            None => None,
        }
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    /// Files OK, or Delete when `choice` named it, for [`pump`] to answer; once the app accepted the
    /// press, closes the dialog with it.
    fn confirm(&mut self, _ctx: &egui::Context) -> Option<()> {
        if self.held.is_accepted() {
            return self.held.finish().map(|_| ());
        }
        if !self.held.is_waiting() {
            let pressed = std::mem::take(&mut self.choice);
            self.held.file(Filed { pressed, form: self.form.clone() });
        }
        None
    }

    fn json_fields(&self) -> Option<Value> {
        let Form { path, adjust, target, .. } = &self.form;
        Some(match target {
            Target::One { index, t, sides, out, single, .. } => {
                let mut fields = json!({
                    "id": path, "index": index, "t": t, "side1": sides.left, "side2": sides.right, "linked": sides.linked,
                    "adjustAdjoining": adjust,
                });
                if let Some((out_index, out)) = out {
                    for (key, value) in [
                        ("outIndex", json!(out_index)),
                        ("outSide1", json!(out.left)),
                        ("outSide2", json!(out.right)),
                        ("outLinked", json!(out.linked)),
                        ("single", json!(single.name())),
                    ] {
                        fields[key] = value;
                    }
                }
                fields
            }
            Target::Several { points, linked, .. } => {
                let indices: Vec<usize> = points.iter().map(|e| e.index).collect();
                let mut fields = json!({"id": path, "indices": indices, "adjustAdjoining": adjust});
                for (set, keys) in SET_KEYS.iter().enumerate().take(self.form.target.sets()) {
                    let [side1, side2, total] = shown(points, set);
                    for (key, value) in [(keys[0], json!(side1)), (keys[1], json!(side2)), (keys[2], json!(total)), (keys[3], json!(linked[set]))] {
                        fields[key] = value;
                    }
                }
                fields
            }
        })
    }

    /// Sets a field as the user entering it would: a side width or Total Width applies by the edit
    /// rules, so a set equal to the value shown still sets that side, and with the link on a side width
    /// sets both sides. `choice` is `"ok"` or `"delete"`, the press the next confirm files.
    fn set_json_field(&mut self, _ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let rejects = || FieldRefusal::Rejects(&value).message(field);
        let boolean = || value.as_bool().ok_or_else(rejects);
        match field {
            "choice" => {
                self.choice = match value.as_str() {
                    Some("ok") => Pressed::Ok,
                    Some("delete") => Pressed::Delete,
                    _ => return Err(rejects()),
                };
                return Ok(());
            }
            "adjustAdjoining" => {
                self.form.adjust = boolean()?;
                return Ok(());
            }
            "id" => return Err(FieldRefusal::ReadOnly.message(field)),
            _ => {}
        }
        let sets = self.form.target.sets();
        match &mut self.form.target {
            Target::One { sides, out, single, .. } => match field {
                "index" | "t" | "outIndex" if field != "outIndex" || out.is_some() => Err(FieldRefusal::ReadOnly.message(field)),
                "single" => match out {
                    Some(_) => {
                        *single = value.as_str().and_then(Single::named).ok_or_else(rejects)?;
                        Ok(())
                    }
                    None => Err(FieldRefusal::Inactive.message(field)),
                },
                _ => {
                    // A single point has no Total Width field.
                    let Some((set, at)) = locate(field).filter(|(_, at)| *at != 2) else { return Err(FieldRefusal::Unknown.message(field)) };
                    let half = match (set, out) {
                        (0, _) => sides,
                        (_, Some((_, out))) => out,
                        _ => return Err(FieldRefusal::Inactive.message(field)),
                    };
                    match at {
                        3 => half.link(boolean()?),
                        0 => half.edit(Edit::Side1(width(&value).ok_or_else(rejects)?)),
                        _ => half.edit(Edit::Side2(width(&value).ok_or_else(rejects)?)),
                    }
                    Ok(())
                }
            },
            Target::Several { points, linked, .. } => {
                if field == "indices" {
                    return Err(FieldRefusal::ReadOnly.message(field));
                }
                let Some((set, at)) = locate(field) else {
                    let refusal = if field == "single" { FieldRefusal::Inactive } else { FieldRefusal::Unknown };
                    return Err(refusal.message(field));
                };
                if set >= sets {
                    return Err(FieldRefusal::Inactive.message(field));
                }
                match at {
                    3 => linked[set] = boolean()?,
                    0 => apply(points, set, Edit::Side1(width(&value).ok_or_else(rejects)?), linked[set]),
                    1 => apply(points, set, Edit::Side2(width(&value).ok_or_else(rejects)?), linked[set]),
                    _ => apply(points, set, Edit::Total(width(&value).ok_or_else(rejects)?), linked[set]),
                }
                Ok(())
            }
        }
    }
}

/// Open Width Point Edit for `index`, or a prospective point at fraction `t` without an index
/// (`{id, index?, t?, adjustAdjoining?}`), or for several existing points (`{id, indices,
/// adjustAdjoining?}`). Opening only samples the stroke: confirmation creates the point.
/// Adjoining adjustment defaults to false. Either half of a discontinuous point opens both.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let id = p.get("id").and_then(Value::as_u64).ok_or("give the path `id`")?;
    let doc = &app.session.active().ok_or("no document")?.doc;
    let id = vectorcraft_tools::distort::stroke_owner(doc, NodeId(id)).0;
    let given = |key: &str| p.get(key).is_some_and(|v| !v.is_null());
    let mut index = match p.get("index").filter(|v| !v.is_null()) {
        Some(v) => Some(v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("no such width point")?),
        None => None,
    };
    if ["index", "indices", "t"].iter().filter(|key| given(key)).count() > 1 {
        return Err("give only one of `index`, `indices` and `t`".into());
    }
    let adjust = p.get("adjustAdjoining").and_then(Value::as_bool).unwrap_or(false);
    let uid = app.session.active().map(|st| st.uid);
    let mut several = None;
    if let Some(indices) = p.get("indices").filter(|v| !v.is_null()) {
        match target(app, id, indices)? {
            Resolved::One(i) => index = Some(i),
            Resolved::Several(es) => several = Some(es),
        }
    }
    let target = match several {
        Some(es) => {
            let discontinuous = es.iter().any(|e| e.sets != [true, true]);
            let linked = [0, 1].map(|set| es.iter().filter(|e| e.sets[set]).all(|e| (e.left - e.right).abs() < 1e-9));
            Target::Several { open: es.clone(), points: es, discontinuous, linked }
        }
        None => one_point(app, id, index, p)?,
    };
    let dialog = WidthPointDialog::new(Form { path: id, doc: uid, adjust, target }, stroke_lengths(app));
    app.dialogs().ok_or("no window")?.with(|host| host.open(dialog));
    Ok(Value::Null)
}

/// The one-point target for `index` of `id`, or the prospective point at the fraction `t` of `p`.
fn one_point(app: &VectorcraftApp, id: u64, index: Option<usize>, p: &Value) -> Result<Target, String> {
    let pair = index.and_then(|i| halves(app, id, i));
    let index = pair.map_or(index, |(a, _)| Some(a));
    let (t, l, r) = if let Some(index) = index {
        point(app, id, index).ok_or("no such width point")?
    } else {
        let t = p.get("t").and_then(Value::as_f64).filter(|t| t.is_finite() && (0.0..=1.0).contains(t)).ok_or("give a fraction `t` from 0 to 1")?;
        let st = app
            .session
            .active()
            .and_then(|s| s.doc.node(NodeId(id)))
            .and_then(|n| n.appearance.stroke())
            .filter(|s| s.width.is_finite() && s.width > 0.0)
            .ok_or("object has no weighted stroke")?;
        let (l, r) = st.profile.as_ref().map_or((1.0, 1.0), |profile| profile.at(t));
        let (l, r) = (l * st.width / 2.0, r * st.width / 2.0);
        if !l.is_finite() || !r.is_finite() || l < 0.0 || r < 0.0 {
            return Err("invalid stroke widths".into());
        }
        (t, l, r)
    };
    let at: Vec<_> = index.into_iter().chain(pair.map(|(_, out)| out)).filter_map(|i| entry_at(app, id, i)).collect();
    let out = match pair {
        Some((_, out)) => {
            let (_, l, r) = point(app, id, out).ok_or("no such width point")?;
            Some((out, Sides::of(l, r)))
        }
        None => None,
    };
    Ok(Target::One { index, t, at, sides: Sides::of(l, r), out, single: Single::None })
}

/// Whether the document the dialog opened in (`doc`) is still the active one.
fn same_document(app: &VectorcraftApp, doc: Option<u64>) -> Result<(), String> {
    match (doc, app.session.active()) {
        (Some(uid), Some(st)) if st.uid == uid => Ok(()),
        _ => Err("no such width point".into()),
    }
}

/// `id`'s width profile and the stroke's half weight, when the points `open` are still where and as
/// wide as when the dialog opened, exactly.
fn opened_profile(app: &VectorcraftApp, id: u64, open: &[Entry]) -> Result<(WidthProfile, f64), String> {
    const NONE: &str = "no such width point";
    let st = app.session.active().and_then(|s| s.doc.node(NodeId(id))).and_then(|n| n.appearance.stroke()).ok_or(NONE)?;
    let half = st.width / 2.0;
    if !(half.is_finite() && half > 0.0) {
        return Err("object has no weighted stroke".into());
    }
    let profile = st.profile.clone().ok_or(NONE)?;
    if open.is_empty() || !open.windows(2).all(|w| w[0].index < w[1].index) {
        return Err(NONE.into());
    }
    for o in open {
        let &(t, l, r) = profile.points.get(o.index).ok_or(NONE)?;
        // The entries were read from this profile at this weight, so an unchanged point compares equal.
        if t != o.t || l * half != o.left || r * half != o.right {
            return Err(NONE.into());
        }
    }
    Ok((profile, half))
}

/// `id`'s width profile, as width factors `[t, left, right]`, with the selected points `open` set to
/// `new` (side widths in points). With `adjust`, each changed point also changes the nearest
/// unselected point at another place before it (unless it is the outgoing half of a discontinuous
/// point) and after it (unless it is the incoming half) in proportion. Every change is measured
/// from the widths when the dialog opened, and a point next to two changed points takes both
/// changes, in path order.
fn edited_profile(app: &VectorcraftApp, id: u64, open: &[Entry], new: &[Entry], adjust: bool) -> Result<Vec<[f64; 3]>, String> {
    let (WidthProfile { points: old }, half) = opened_profile(app, id, open)?;
    if open.len() != new.len() || open.iter().zip(new).any(|(o, e)| o.index != e.index) {
        return Err("no such width point".into());
    }
    let mut points = old.clone();
    let selected = |j: usize| open.binary_search_by_key(&j, |o| o.index).is_ok();
    // The nearest unselected point before and after each point.
    let (mut prev, mut next) = (vec![None; old.len()], vec![None; old.len()]);
    let mut last = None;
    for (j, p) in prev.iter_mut().enumerate() {
        *p = last;
        if !selected(j) {
            last = Some(j);
        }
    }
    last = None;
    for (j, n) in next.iter_mut().enumerate().rev() {
        *n = last;
        if !selected(j) {
            last = Some(j);
        }
    }
    for (o, e) in open.iter().zip(new).filter(|_| adjust) {
        if o.left == e.left && o.right == e.right {
            continue;
        }
        // The point's place; both halves of a discontinuous point measure from the incoming half's.
        let t = old[if o.sets[0] { o.index } else { o.index.saturating_sub(1) }].0;
        // The nearest unselected point farther than SAME_T from that place on one side.
        let nearest = |side: &[Option<usize>]| {
            let mut j = side[o.index];
            while let Some(k) = j
                && (old[k].0 - t).abs() <= SAME_T
            {
                j = side[k];
            }
            j
        };
        let before = nearest(&prev).filter(|_| o.sets[0]);
        let after = nearest(&next).filter(|_| o.sets[1]);
        for j in before.into_iter().chain(after) {
            let q = &mut points[j];
            (q.1, q.2) = (scale(q.1, o.left / half, e.left / half), scale(q.2, o.right / half, e.right / half));
        }
    }
    for e in new {
        (points[e.index].1, points[e.index].2) = (e.left / half, e.right / half);
    }
    if points.iter().any(|q| !q.1.is_finite() || !q.2.is_finite()) {
        return Err("invalid stroke widths".into());
    }
    Ok(points.into_iter().map(|(t, l, r)| [t, l, r]).collect())
}

/// Whether width point `index` of `id` and the outgoing half `out` of a discontinuous point are still
/// as they were when the dialog opened (`at`), and still pair.
fn unchanged_point(app: &VectorcraftApp, id: u64, at: &[[f64; 3]], index: usize, out: Option<usize>) -> Result<(), String> {
    const NONE: &str = "no such width point";
    let now: Vec<_> = std::iter::once(index).chain(out).map(|i| entry_at(app, id, i)).collect::<Option<_>>().ok_or(NONE)?;
    // The widths are read at the stroke's current weight, so a changed weight changes them too, and
    // a continuous point must still be one.
    if now != at || halves(app, id, index) != out.map(|o| (index, o)) {
        return Err(NONE.into());
    }
    Ok(())
}

/// Runs what `pressed` asks of the points `form` names: OK sets them, Delete removes them. A refusal
/// is the reason the dialog shows; nothing changed then.
// PLAN: the sheet's `answer(app, &WidthPointDialog, Pressed)` cannot work: the held press is cloned out of the
// dialog before the host is released, and `HeldAnswer` needs the answer to be `Clone`, so the press carries the `Form`.
fn answer(app: &mut VectorcraftApp, form: &Form, pressed: Pressed) -> Result<(), String> {
    let id = form.path;
    let (command, params) = match &form.target {
        Target::Several { open, points, .. } => {
            same_document(app, form.doc)?;
            opened_profile(app, id, open)?;
            if pressed == Pressed::Delete {
                let indices: Vec<usize> = open.iter().map(|e| e.index).collect();
                ("stroke.widthPoint.remove", json!({"id": id, "indices": indices}))
            } else if open.len() == points.len() && open.iter().zip(points).all(|(o, e)| o.index == e.index && o.left == e.left && o.right == e.right)
            {
                // Nothing changed: no command and no undo step.
                return Ok(());
            } else {
                let profile = edited_profile(app, id, open, points, form.adjust)?;
                ("stroke.widthProfile.set", json!({"ids": [id], "points": profile}))
            }
        }
        Target::One { index, t, at, sides, out, single } => {
            if pressed == Pressed::Delete && index.is_none() {
                // Deleting a prospective point discards the dialog; no document point exists yet.
                return Ok(());
            }
            // OK and Delete act only on the document and the points the dialog opened on.
            same_document(app, form.doc)?;
            let out_index = out.as_ref().map(|(i, _)| *i);
            if let Some(index) = index {
                unchanged_point(app, id, at, *index, out_index)?;
            }
            if pressed == Pressed::Delete {
                match out_index {
                    Some(out) => ("stroke.widthPoint.remove", json!({"id": id, "indices": [index, out]})),
                    None => ("stroke.widthPoint.remove", json!({"id": id, "index": index})),
                }
            } else {
                let mut p = json!({"id": id, "index": index, "t": t, "left": sides.left, "right": sides.right, "adjustAdjoining": form.adjust});
                if let Some((out_index, out)) = out {
                    p["outIndex"] = json!(out_index);
                    p["outLeft"] = json!(out.left);
                    p["outRight"] = json!(out.right);
                    if *single != Single::None {
                        p["single"] = json!(single.name());
                    }
                }
                ("stroke.widthPoint.set", p)
            }
        }
    };
    app.run(command, params).map(|_| ())
}

/// Gives the open dialog the stroke unit it shows the side widths in, which the preferences can change
/// while it is open.
pub(crate) fn sync(app: &mut VectorcraftApp) {
    let lengths = stroke_lengths(app);
    let Some(dialogs) = app.dialogs() else { return };
    dialogs.with(|host| {
        if let Some(dialog) = host.get_mut::<WidthPointDialog>() {
            dialog.lengths = lengths;
        }
    });
}

/// Answers the press OK or Delete filed: runs it in the document, then tells the dialog to close or
/// to show why the app refused. The host stays unlocked while the press runs, as a command may open
/// dialogs.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let filed = dialogs.with(|host| {
        // The result of a closed dialog is not read; drop it.
        let _ = host.take::<WidthPointDialog>();
        host.get_mut::<WidthPointDialog>().and_then(|dialog| dialog.held.take())
    });
    let Some(Filed { pressed, form }) = filed else { return };
    let result = answer(app, &form, pressed).map_err(Label::from);
    dialogs.with(|host| {
        if let Some(dialog) = host.get_mut::<WidthPointDialog>() {
            dialog.held.answer(result);
        }
    });
    // The accepted press closes the dialog in its next pass.
    ctx.request_repaint();
}

#[cfg(test)]
mod tests;
