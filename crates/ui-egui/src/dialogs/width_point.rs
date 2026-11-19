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
//! `outLinked` (the outgoing half) and `single` (`""`, `"incoming"` or `"outgoing"`); Delete sets
//! `discard: true` and confirms.
//!
//! Opened on several selected width points (`indices`), the dialog edits all of them. Side 1,
//! Side 2 and Total Width show a value only where every point has it, and are blank otherwise. A
//! side width typed in sets that side of every point (both sides with the link on) and leaves the
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
//! the dialog opened or another document is active.

use egui::vec2;
use serde_json::{Value, json};
use vectorcraft_doc::{Unit, WidthProfile};
use vectorcraft_engine::doc::NodeId;
use vectorcraft_engine::width_point_pair;

use super::swatch_options::{grid, label};
use super::{DialogSpec, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Width Point Edit.
pub const KIND: &str = "widthPoint";

/// Width points closer than this (in t) are at the same place, as the engine counts them.
const SAME_T: f64 = 1e-6;

/// Side widths (points) closer than this show as one value.
const SAME_WIDTH: f64 = 1e-6;

/// The working side widths of several points being edited.
const POINTS: &str = "__points";
/// Their side widths when the dialog opened.
const OPEN: &str = "__open";
/// Whether they include a discontinuous point (two sets of side widths shown).
const DISCONTINUOUS: &str = "__discontinuous";
/// The uid of the document the dialog opened in.
const DOC: &str = "__doc";
/// The place and side widths `[t, left, right]` (points) of the existing point the one-point dialog
/// opened on, and of the outgoing half of a discontinuous point.
const AT: &str = "__at";

/// The fields of each set of side widths of several points: Side 1, Side 2, Total Width and the
/// link, for the incoming set and the outgoing set.
const MANY: [[&str; 4]; 2] = [["side1", "side2", "totalWidth", "linked"], ["outSide1", "outSide2", "outTotalWidth", "outLinked"]];
/// The number fields that show them.
const MANY_IDS: [[&str; 3]; 2] = [["wp-side1", "wp-side2", "wp-total"], ["wp-out-side1", "wp-out-side2", "wp-out-total"]];

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Width Point Edit").into(), body, confirm, discard: Some("Delete"), min_width: 300.0, ..DialogSpec::FORM };

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

/// The entries held in field `key`.
fn entries(d: &Dialog, key: &str) -> Vec<Entry> {
    let entry = |e: &Value| {
        let e = e.as_array()?;
        let index = usize::try_from(e.first()?.as_u64()?).ok()?;
        let (t, left, right) = (e.get(1)?.as_f64()?, e.get(2)?.as_f64()?, e.get(3)?.as_f64()?);
        Some(Entry { index, t, left, right, sets: [e.get(4)?.as_bool()?, e.get(5)?.as_bool()?] })
    };
    d.fields.get(key).and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().filter_map(entry).collect())
}

fn entries_json(es: &[Entry]) -> Value {
    json!(es.iter().map(|e| json!([e.index, e.t, e.left, e.right, e.sets[0], e.sets[1]])).collect::<Vec<_>>())
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

/// Keep `es` as the working widths of `d`, and show them in its fields. A value set straight into a
/// field other than `typed` that OK refuses, and that is not the value the field showed, stays for
/// OK to refuse.
fn store(d: &mut Dialog, es: &[Entry], typed: Option<&str>) {
    let old = entries(d, POINTS);
    d.fields.insert(POINTS.into(), entries_json(es));
    let sets = if d.bool(DISCONTINUOUS) { 2 } else { 1 };
    for (set, keys) in MANY.iter().enumerate().take(sets) {
        for ((key, value), was) in keys.iter().zip(shown(es, set)).zip(shown(&old, set)) {
            let showed = reading(d, key).ok().flatten().zip(was).is_some_and(|(v, s)| (v - s).abs() <= SAME_WIDTH);
            if Some(*key) != typed && number(d, key).is_err() && !showed {
                continue;
            }
            d.fields.insert((*key).into(), json!(value));
        }
    }
}

/// A change typed into one set of side widths of several points.
#[derive(Clone, Copy, Debug)]
enum Edit {
    Side1(f64),
    Side2(f64),
    Total(f64),
}

/// Apply `edit` to every point that set `set` edits: a side width replaces that side (both sides
/// with `linked`), and Total Width scales each point's two sides to it (equal halves from nothing).
fn apply(es: &mut [Entry], set: usize, edit: Edit, linked: bool) {
    for e in es.iter_mut().filter(|e| e.sets[set]) {
        let (l, r) = match edit {
            Edit::Side1(v) => (v, if linked { v } else { e.right }),
            Edit::Side2(v) => (if linked { v } else { e.left }, v),
            Edit::Total(v) => {
                let sum = e.left + e.right;
                if sum > 1e-9 { (e.left / sum * v, e.right / sum * v) } else { (v / 2.0, v / 2.0) }
            }
        };
        (e.left, e.right) = (l.max(0.0), r.max(0.0));
    }
}

/// The width in field `key`, any finite width from nothing up (none when it is null or absent).
fn reading(d: &Dialog, key: &str) -> Result<Option<f64>, String> {
    let v = match d.fields.get(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => Unit::Points.parse(s),
        Some(_) => None,
    };
    match v {
        Some(v) if v.is_finite() && v >= 0.0 => Ok(Some(v)),
        _ => Err("invalid stroke widths".into()),
    }
}

/// The largest side width or Total Width a value typed in can set (points).
const MAX_WIDTH: f64 = 1.0e5;

/// The number in field `key` that a value typed in can set (none when it is null or absent).
fn number(d: &Dialog, key: &str) -> Result<Option<f64>, String> {
    match reading(d, key)? {
        Some(v) if v > MAX_WIDTH => Err("invalid stroke widths".into()),
        v => Ok(v),
    }
}

/// The widths OK sets on several points: the working widths, with any value set straight into a
/// field (`ui.dialog.set`) that they do not show yet applied as typing it would, Total Width before
/// the sides. A value the field shows changes nothing, even a Total Width above [`MAX_WIDTH`].
fn edited(d: &Dialog) -> Result<Vec<Entry>, String> {
    let mut es = entries(d, POINTS);
    let sets = if d.bool(DISCONTINUOUS) { 2 } else { 1 };
    let before: Vec<_> = (0..sets).map(|set| shown(&es, set)).collect();
    for (set, shown) in before.iter().enumerate() {
        let [k1, k2, kt, kl] = MANY[set];
        for (key, at) in [(kt, 2), (k1, 0), (k2, 1)] {
            let Some(v) = reading(d, key)? else { continue };
            let same = shown[at].filter(|s| (s - v).abs() <= SAME_WIDTH);
            // Only a value above the limit that the points already have passes it.
            if v > MAX_WIDTH && !same.is_some_and(|s| s > MAX_WIDTH) {
                return Err("invalid stroke widths".into());
            }
            if same.is_some() {
                continue;
            }
            let edit = match at {
                0 => Edit::Side1(v),
                1 => Edit::Side2(v),
                _ => Edit::Total(v),
            };
            apply(&mut es, set, edit, d.bool(kl));
        }
    }
    Ok(es)
}

/// After `ui.dialog.set` sets `field` of several points' dialog: a side width or Total Width applies
/// as typing it does, so a value equal to the one shown still sets that side of every point (both
/// sides with the link on). A null or unreadable value is left for OK to skip or refuse, and a value
/// set later into another field keeps it.
pub(super) fn note_set(d: &mut Dialog, field: &str) {
    if !d.fields.contains_key(OPEN) {
        return;
    }
    let sets = if d.bool(DISCONTINUOUS) { 2 } else { 1 };
    for (set, keys) in MANY.iter().enumerate().take(sets) {
        let Some(at) = keys[..3].iter().position(|k| *k == field) else { continue };
        let Ok(Some(v)) = number(d, field) else { return };
        let edit = match at {
            0 => Edit::Side1(v),
            1 => Edit::Side2(v),
            _ => Edit::Total(v),
        };
        let mut es = entries(d, POINTS);
        apply(&mut es, set, edit, d.bool(keys[3]));
        store(d, &es, Some(field));
        return;
    }
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
enum Target {
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
fn target(app: &VectorcraftApp, id: u64, indices: &Value) -> Result<Target, String> {
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
        return Ok(Target::One(picked[0]));
    }
    if !(st.width.is_finite() && st.width > 0.0) {
        return Err("object has no weighted stroke".into());
    }
    let half = st.width / 2.0;
    let entry = |i: usize| {
        let (t, l, r) = points[i];
        Entry { index: i, t, left: l * half, right: r * half, sets: [!outgoing(i), pairs[i].is_none()] }
    };
    Ok(Target::Several(picked.into_iter().map(entry).collect()))
}

/// Open Width Point Edit on the several width points `es` of `id`.
fn open_several(app: &mut VectorcraftApp, id: u64, es: &[Entry], adjust: bool) {
    let discontinuous = es.iter().any(|e| e.sets != [true, true]);
    let indices: Vec<usize> = es.iter().map(|e| e.index).collect();
    let doc = app.session.active().map(|st| st.uid);
    let mut d = Dialog::new(
        KIND,
        json!({"id": id, "indices": indices, "adjustAdjoining": adjust, DISCONTINUOUS: discontinuous, OPEN: entries_json(es), DOC: doc}),
    );
    for (set, keys) in MANY.iter().enumerate().take(if discontinuous { 2 } else { 1 }) {
        let linked = es.iter().filter(|e| e.sets[set]).all(|e| (e.left - e.right).abs() < 1e-9);
        d.fields.insert(keys[3].into(), json!(linked));
    }
    store(&mut d, es, None);
    app.ui.dialog = Some(d);
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
    if let Some(indices) = p.get("indices").filter(|v| !v.is_null()) {
        match target(app, id, indices)? {
            Target::One(i) => index = Some(i),
            Target::Several(es) => {
                let adjust = p.get("adjustAdjoining").and_then(Value::as_bool).unwrap_or(false);
                open_several(app, id, &es, adjust);
                return Ok(Value::Null);
            }
        }
    }
    let pair = index.and_then(|i| halves(app, id, i));
    let index = pair.map_or(index, |(a, _)| Some(a));
    let (t, l, r) = if let Some(index) = index {
        point(app, id, index).ok_or("no such width point")?
    } else {
        let t = p.get("t").and_then(Value::as_f64).filter(|t| t.is_finite() && (0.0..=1.0).contains(t)).ok_or("give a fraction `t` from 0 to 1")?;
        let st = doc
            .node(NodeId(id))
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
    let adjust = p.get("adjustAdjoining").and_then(Value::as_bool).unwrap_or(false);
    let doc = app.session.active().map(|st| st.uid);
    let at: Vec<_> = index.into_iter().chain(pair.map(|(_, out)| out)).filter_map(|i| entry_at(app, id, i)).collect();
    let linked = (l - r).abs() < 1e-9;
    let mut fields = json!({"id": id, "index": index, "t": t, "side1": l, "side2": r, "linked": linked, "adjustAdjoining": adjust, DOC: doc, AT: at});
    if let Some((_, out)) = pair {
        let (_, l, r) = point(app, id, out).ok_or("no such width point")?;
        for (key, value) in [
            ("outIndex", json!(out)),
            ("outSide1", json!(l)),
            ("outSide2", json!(r)),
            ("outLinked", json!((l - r).abs() < 1e-9)),
            ("single", json!("")),
        ] {
            fields[key] = value;
        }
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

/// Whether the dialog edits a discontinuous point.
fn paired(d: &Dialog) -> bool {
    d.fields.get("outIndex").is_some_and(|v| !v.is_null())
}

/// One set of side widths: Side 1 with its link, Side 2 and Total Width, kept in the fields `keys`
/// (side 1, side 2, linked) and shown in the number fields `ids`.
fn sides(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit, keys: [&str; 3], ids: [&str; 3]) {
    let [k1, k2, kl] = keys;
    let (s1, s2, linked) = (d.f64(k1, 0.0), d.f64(k2, 0.0), d.bool(kl));
    let mut new = None;
    grid(ui, |ui| {
        label(ui, tl!("Side 1:"));
        ui.horizontal(|ui| {
            if let Some(v) = widgets::num_field(ui, ids[0], Some(s1), unit, 90.0) {
                new = Some((v, if linked { v } else { s2 }));
            }
            if widgets::icon_button(ui, if linked { "link" } else { "link-2-off" }, tl!("Keep both sides the same width"), linked, 22.0).clicked() {
                d.fields.insert(kl.into(), json!(!linked));
                if !linked {
                    new = Some((s1, s1));
                }
            }
        });
        ui.end_row();
        label(ui, tl!("Side 2:"));
        if let Some(v) = widgets::num_field(ui, ids[1], Some(s2), unit, 90.0) {
            new = Some((if linked { v } else { s1 }, v));
        }
        ui.end_row();
        label(ui, tl!("Total Width:"));
        if let Some(v) = widgets::num_field(ui, ids[2], Some(s1 + s2), unit, 90.0) {
            // Both sides in proportion (equal halves from nothing).
            let k = if s1 + s2 > 1e-9 { v / (s1 + s2) } else { 0.0 };
            new = Some(if k > 0.0 { (s1 * k, s2 * k) } else { (v / 2.0, v / 2.0) });
        }
        ui.end_row();
    });
    if let Some((a, b)) = new {
        d.fields.insert(k1.into(), json!(a.max(0.0)));
        d.fields.insert(k2.into(), json!(b.max(0.0)));
    }
}

/// Set `set` of the side widths of several points: Side 1 with its link, Side 2 and Total Width,
/// blank where the points differ. A value typed in changes every point the set edits; the link only
/// changes how later values apply.
fn sides_of_several(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit, set: usize) {
    let [k1, k2, kt, kl] = MANY[set];
    let ids = MANY_IDS[set];
    let value = |key: &str| reading(d, key).ok().flatten();
    let (s1, s2, total, linked) = (value(k1), value(k2), value(kt), d.bool(kl));
    let mut edit = None;
    grid(ui, |ui| {
        label(ui, tl!("Side 1:"));
        ui.horizontal(|ui| {
            if let Some(v) = widgets::num_field(ui, ids[0], s1, unit, 90.0) {
                edit = Some((Edit::Side1(v), k1));
            }
            if widgets::icon_button(ui, if linked { "link" } else { "link-2-off" }, tl!("Keep both sides the same width"), linked, 22.0).clicked() {
                d.fields.insert(kl.into(), json!(!linked));
            }
        });
        ui.end_row();
        label(ui, tl!("Side 2:"));
        if let Some(v) = widgets::num_field(ui, ids[1], s2, unit, 90.0) {
            edit = Some((Edit::Side2(v), k2));
        }
        ui.end_row();
        label(ui, tl!("Total Width:"));
        if let Some(v) = widgets::num_field(ui, ids[2], total, unit, 90.0) {
            edit = Some((Edit::Total(v), kt));
        }
        ui.end_row();
    });
    if let Some((Edit::Side1(v) | Edit::Side2(v) | Edit::Total(v), key)) = edit
        && !(0.0..=MAX_WIDTH).contains(&v)
    {
        // Not a width it can set: the field keeps it for OK to refuse.
        d.fields.insert(key.into(), if v.is_finite() { json!(v) } else { json!(v.to_string()) });
    } else if let Some((edit, key)) = edit {
        // Values set straight into the fields count first, so typing does not lose them.
        let mut es = edited(d).unwrap_or_else(|_| entries(d, POINTS));
        apply(&mut es, set, edit, linked);
        store(d, &es, Some(key));
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.stroke_unit();
    let incoming = (["side1", "side2", "linked"], ["wp-side1", "wp-side2", "wp-total"]);
    if d.fields.contains_key(OPEN) {
        if d.bool(DISCONTINUOUS) {
            widgets::section_header(ui, tl!("Incoming"));
            ui.push_id("wp-incoming", |ui| sides_of_several(ui, d, unit, 0));
            widgets::section_header(ui, tl!("Outgoing"));
            ui.push_id("wp-outgoing", |ui| sides_of_several(ui, d, unit, 1));
        } else {
            sides_of_several(ui, d, unit, 0);
        }
    } else if paired(d) {
        widgets::section_header(ui, tl!("Incoming"));
        ui.push_id("wp-incoming", |ui| sides(ui, d, unit, incoming.0, incoming.1));
        widgets::section_header(ui, tl!("Outgoing"));
        ui.push_id("wp-outgoing", |ui| sides(ui, d, unit, ["outSide1", "outSide2", "outLinked"], ["wp-out-side1", "wp-out-side2", "wp-out-total"]));
        widgets::section_header(ui, tl!("Single Width Only"));
        ui.horizontal(|ui| {
            let single = d.str("single");
            for (value, text) in [("", tl!("None")), ("incoming", tl!("Incoming")), ("outgoing", tl!("Outgoing"))] {
                if widgets::radio(ui, text, single == value, true) {
                    d.fields.insert("single".into(), json!(value));
                }
            }
        });
    } else {
        sides(ui, d, unit, incoming.0, incoming.1);
    }
    ui.add_space(4.0);
    ui.allocate_ui(vec2(ui.available_width(), 22.0), |ui| {
        if widgets::check(ui, tl!("Adjust Adjoining Width Points"), d.bool("adjustAdjoining"), true) {
            d.fields.insert("adjustAdjoining".into(), json!(!d.bool("adjustAdjoining")));
        }
    });
    false
}

/// Whether the document the dialog opened in is still the active one.
fn same_document(app: &VectorcraftApp, d: &Dialog) -> Result<(), String> {
    match (d.fields.get(DOC).and_then(Value::as_u64), app.session.active()) {
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

/// OK or Delete on several width points.
fn confirm_several(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let id = d.fields.get("id").and_then(Value::as_u64).ok_or("no such width point")?;
    let open = entries(d, OPEN);
    same_document(app, d)?;
    opened_profile(app, id, &open)?;
    if d.bool("discard") {
        let indices: Vec<usize> = open.iter().map(|e| e.index).collect();
        return run_and_close(app, "stroke.widthPoint.remove", json!({"id": id, "indices": indices}));
    }
    let new = edited(d)?;
    if open.len() == new.len() && open.iter().zip(&new).all(|(o, e)| o.index == e.index && o.left == e.left && o.right == e.right) {
        // Nothing changed: no command and no undo step.
        app.ui.dialog = None;
        return Ok(Value::Null);
    }
    let points = edited_profile(app, id, &open, &new, d.bool("adjustAdjoining"))?;
    run_and_close(app, "stroke.widthProfile.set", json!({"ids": [id], "points": points}))
}

/// Whether width point `index` of the one-point dialog's stroke, and the outgoing half `out` of a
/// discontinuous point, are still as they were when the dialog opened, and still pair.
fn unchanged_point(app: &VectorcraftApp, d: &Dialog, index: usize, out: Option<usize>) -> Result<(), String> {
    const NONE: &str = "no such width point";
    let id = d.fields.get("id").and_then(Value::as_u64).ok_or(NONE)?;
    let at: Vec<[f64; 3]> = d.fields.get(AT).cloned().and_then(|v| serde_json::from_value(v).ok()).ok_or(NONE)?;
    let now: Vec<_> = std::iter::once(index).chain(out).map(|i| entry_at(app, id, i)).collect::<Option<_>>().ok_or(NONE)?;
    // The widths are read at the stroke's current weight, so a changed weight changes them too, and
    // a continuous point must still be one.
    if now != at || halves(app, id, index) != out.map(|o| (index, o)) {
        return Err(NONE.into());
    }
    Ok(())
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if d.fields.contains_key(OPEN) {
        return confirm_several(app, d);
    }
    let (id, index) = (d.fields.get("id").cloned().unwrap_or_default(), d.fields.get("index").cloned().unwrap_or_default());
    let out = d.fields.get("outIndex").filter(|_| paired(d)).cloned();
    if d.bool("discard") && index.is_null() {
        // Deleting a prospective point discards the dialog; no document point exists yet.
        app.ui.dialog = None;
        return Ok(Value::Null);
    }
    // OK and Delete act only on the document and the points the dialog opened on.
    same_document(app, d)?;
    if let Some(index) = index.as_u64().and_then(|i| usize::try_from(i).ok()) {
        unchanged_point(app, d, index, out.as_ref().and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()))?;
    }
    if d.bool("discard") {
        let p = match out {
            Some(out) => json!({"id": id, "indices": [index, out]}),
            None => json!({"id": id, "index": index}),
        };
        return run_and_close(app, "stroke.widthPoint.remove", p);
    }
    let (left, right) = (d.f64("side1", 0.0), d.f64("side2", 0.0));
    let mut p = json!({"id": id, "index": index, "t": d.f64("t", 0.0), "left": left, "right": right, "adjustAdjoining": d.bool("adjustAdjoining")});
    if let Some(out) = out {
        p["outIndex"] = out;
        p["outLeft"] = json!(d.f64("outSide1", 0.0));
        p["outRight"] = json!(d.f64("outSide2", 0.0));
        let single = d.str("single");
        if !single.is_empty() {
            p["single"] = json!(single);
        }
    }
    run_and_close(app, "stroke.widthPoint.set", p)
}

#[cfg(test)]
mod tests;
