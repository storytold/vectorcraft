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

use egui::vec2;
use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_engine::doc::NodeId;
use vectorcraft_engine::width_point_pair;

use super::swatch_options::{grid, label};
use super::{DialogSpec, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Width Point Edit.
pub const KIND: &str = "widthPoint";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Width Point Edit").into(), body, confirm, discard: Some("Delete"), min_width: 300.0, ..DialogSpec::FORM };

/// Width point `index` of `id`: its place and side widths (points).
fn point(app: &VectorcraftApp, id: u64, index: usize) -> Option<(f64, f64, f64)> {
    let st = app.session.active()?.doc.node(NodeId(id))?.appearance.stroke()?;
    let half = st.width / 2.0;
    st.profile.as_ref()?.points.get(index).map(|&(t, l, r)| (t, l * half, r * half))
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

/// Open Width Point Edit for `index`, or a prospective point at fraction `t` without an index
/// (`{id, index?, t?, adjustAdjoining?}`). Opening only samples the stroke: confirmation creates
/// the point. Adjoining adjustment defaults to false. Either half of a discontinuous point opens
/// both.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let id = p.get("id").and_then(Value::as_u64).ok_or("give the path `id`")?;
    let doc = &app.session.active().ok_or("no document")?.doc;
    let id = vectorcraft_tools::distort::stroke_owner(doc, NodeId(id)).0;
    let index = match p.get("index").filter(|v| !v.is_null()) {
        Some(v) => Some(v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("no such width point")?),
        None => None,
    };
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
    let mut fields = json!({"id": id, "index": index, "t": t, "side1": l, "side2": r, "linked": (l - r).abs() < 1e-9, "adjustAdjoining": adjust});
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

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.stroke_unit();
    let incoming = (["side1", "side2", "linked"], ["wp-side1", "wp-side2", "wp-total"]);
    if paired(d) {
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

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (id, index) = (d.fields.get("id").cloned().unwrap_or_default(), d.fields.get("index").cloned().unwrap_or_default());
    let out = d.fields.get("outIndex").filter(|_| paired(d)).cloned();
    if d.bool("discard") {
        if index.is_null() {
            // Deleting a prospective point discards the dialog; no document point exists yet.
            app.ui.dialog = None;
            return Ok(Value::Null);
        }
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
