//! Width Point Edit: double-clicking a stroke with the Width tool (or `ui.widthPointEdit`)
//! creates or edits a point and its side widths. Total Width changes both sides in proportion,
//! the link keeps them equal,
//! and Adjust Adjoining Width Points changes the nearest points either side in proportion too.
//! Shift-double-click on a width point opens the dialog with that option enabled.
//! OK runs `stroke.widthPoint.set`; Delete removes an existing point (`stroke.widthPoint.remove`)
//! or discards a prospective point without changing the document.
//!
//! Fields: `id`, `index` (an existing width point, otherwise null), `t` (its place), `side1`,
//! `side2` (the left and right widths, points), `linked`, `adjustAdjoining`; Delete sets `discard: true` and confirms.

use egui::vec2;
use serde_json::{Value, json};
use vectorcraft_engine::doc::NodeId;

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

/// Open Width Point Edit for `index`, or a prospective point at fraction `t` without an index
/// (`{id, index?, t?, adjustAdjoining?}`). Opening only samples the stroke: confirmation creates
/// the point. Adjoining adjustment defaults to false.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let id = p.get("id").and_then(Value::as_u64).ok_or("give the path `id`")?;
    let doc = &app.session.active().ok_or("no document")?.doc;
    let id = vectorcraft_tools::distort::stroke_owner(doc, NodeId(id)).0;
    let index = match p.get("index").filter(|v| !v.is_null()) {
        Some(v) => Some(v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("no such width point")?),
        None => None,
    };
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
    let fields = json!({"id": id, "index": index, "t": t, "side1": l, "side2": r, "linked": (l - r).abs() < 1e-9, "adjustAdjoining": adjust});
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.stroke_unit();
    let (s1, s2, linked) = (d.f64("side1", 0.0), d.f64("side2", 0.0), d.bool("linked"));
    let mut sides = None;
    grid(ui, |ui| {
        label(ui, tl!("Side 1:"));
        ui.horizontal(|ui| {
            if let Some(v) = widgets::num_field(ui, "wp-side1", Some(s1), unit, 90.0) {
                sides = Some((v, if linked { v } else { s2 }));
            }
            if widgets::icon_button(ui, if linked { "link" } else { "link-2-off" }, tl!("Keep both sides the same width"), linked, 22.0).clicked() {
                d.fields.insert("linked".into(), json!(!linked));
                if !linked {
                    sides = Some((s1, s1));
                }
            }
        });
        ui.end_row();
        label(ui, tl!("Side 2:"));
        if let Some(v) = widgets::num_field(ui, "wp-side2", Some(s2), unit, 90.0) {
            sides = Some((if linked { v } else { s1 }, v));
        }
        ui.end_row();
        label(ui, tl!("Total Width:"));
        if let Some(v) = widgets::num_field(ui, "wp-total", Some(s1 + s2), unit, 90.0) {
            // Both sides in proportion (equal halves from nothing).
            let k = if s1 + s2 > 1e-9 { v / (s1 + s2) } else { 0.0 };
            sides = Some(if k > 0.0 { (s1 * k, s2 * k) } else { (v / 2.0, v / 2.0) });
        }
        ui.end_row();
    });
    if let Some((a, b)) = sides {
        d.fields.insert("side1".into(), json!(a.max(0.0)));
        d.fields.insert("side2".into(), json!(b.max(0.0)));
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
    if d.bool("discard") {
        if index.is_null() {
            // Deleting a prospective point discards the dialog; no document point exists yet.
            app.ui.dialog = None;
            return Ok(Value::Null);
        }
        return run_and_close(app, "stroke.widthPoint.remove", json!({"id": id, "index": index}));
    }
    let (left, right) = (d.f64("side1", 0.0), d.f64("side2", 0.0));
    let p = json!({"id": id, "index": index, "t": d.f64("t", 0.0), "left": left, "right": right, "adjustAdjoining": d.bool("adjustAdjoining")});
    run_and_close(app, "stroke.widthPoint.set", p)
}

#[cfg(test)]
mod tests;
