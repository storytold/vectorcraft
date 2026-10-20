//! Artboard Options (double-click with the Artboard tool, double-click the tool, or the Artboards
//! panel's options icon or menu): the artboard's fields, sizes parsed with units, and its position
//! from a reference point. A new size also goes to the other selected artboards.

use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_geom::{Point, Rect};

use super::{DialogSpec, form, run_and_close};
use crate::VectorcraftApp;
use crate::panels::transform::{artboard_rect, reference_index, reference_locator};
use crate::state::Dialog;

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Artboard Options").into(), body, confirm, ..DialogSpec::FORM };

/// Where the fields put the artboard: `x` and `y` are its top-left corner in document points, as
/// `artboard.setProps` and `artboard.new` take them.
fn placed(d: &Dialog) -> Rect {
    let (x, y) = (d.f64("x", 0.0), d.f64("y", 0.0));
    Rect::new(x, y, x + d.f64("width", 0.0), y + d.f64("height", 0.0))
}

/// The form's Name, Width and Height, then X and Y in the same grid, the reference point locator
/// before X (#1876). The locator is the one the Transform panel, the Control bar and Properties
/// share, and X and Y are where its point lies on the rulers, counted from the ruler origin (#1212),
/// as the Transform panel and the Control bar show an artboard's. A new Width or Height keeps the
/// reference point where it was, as their sizes do.
fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let (unit, origin, refi) = (app.session.general_unit(), app.ruler_origin(), reference_index(ui));
    let (was, w0, h0) = (placed(d), d.f64("width", 0.0), d.f64("height", 0.0));
    form::grid_then(ui, d, unit, |ui, d| {
        let (w, h) = (d.f64("width", 0.0), d.f64("height", 0.0));
        let n = artboard_rect(was, refi, None, None, Some(w), Some(h), false);
        // Only the side that changed moves, so an untouched coordinate stays exactly as it was. The
        // reference point's column or row is kept for the other selected artboards (`confirm`).
        for (key, changed, at, part) in [("x", w != w0, n.x0, refi % 3), ("y", h != h0, n.y0, refi / 3)] {
            if changed {
                d.fields.insert(key.into(), json!(at));
                d.fields.insert(format!("__ref{key}"), json!(part));
            }
        }
        position_rows(ui, d, refi, unit, origin);
    });
    false
}

/// The X and Y rows: where reference point `refi` lies, counted from the ruler origin `origin`,
/// with the locator before X's label. A typed X or Y moves the artboard so that its reference point
/// lands there.
fn position_rows(ui: &mut egui::Ui, d: &mut Dialog, refi: usize, unit: Unit, origin: Point) {
    let r = placed(d);
    let rp = vectorcraft_geom::reference_point(r, refi);
    let mut indent = 0.0;
    for (key, label, at, zero) in [("x", "X:", rp.x, origin.x), ("y", "Y:", rp.y, origin.y)] {
        ui.horizontal(|ui| {
            // Y's label lines up with X's, after the locator.
            if key == "x" {
                let left = ui.cursor().min.x;
                reference_locator(ui, refi);
                indent = ui.min_rect().right() - left;
            } else {
                ui.add_space(indent);
            }
            form::row_label(ui, label);
        });
        if let Some(v) = crate::widgets::num_field(ui, ("dlg-ab-xy", key), Some(at - zero), unit, form::FIELD_W) {
            let n = if key == "x" {
                artboard_rect(r, refi, Some(v + zero), None, None, None, false).x0
            } else {
                artboard_rect(r, refi, None, Some(v + zero), None, None, false).y0
            };
            d.fields.insert(key.into(), json!(n));
        }
        ui.end_row();
    }
}

/// The dialog kind of Artboard Options.
pub const KIND: &str = "artboardOptions";

/// Open Artboard Options for the active artboard: the Artboard tool's while it is in use, else the
/// window's.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let index = if app.session.tool_id() == "artboard" {
        app.session.tool_options()["active"].as_u64().and_then(|i| usize::try_from(i).ok()).unwrap_or(0)
    } else {
        app.view().map_or(0, |v| v.artboard)
    };
    let st = app.session.active().ok_or("no document")?;
    let a = st.doc.artboards.get(index).ok_or("no artboard")?;
    let r = a.rect;
    let fields = json!({"index": index, "name": a.name, "x": r.x0, "y": r.y0, "width": r.width(), "height": r.height()});
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(json!({ "dialog": KIND }))
}

/// OK sets the artboard's fields. A Width or Height that differs from the artboard's also goes to
/// the other selected artboards (when this one is among them), each resized about the reference
/// point the form's last Width or Height change used (fields `__refx` and `__refy`, its column and
/// row; the top-left corner when the form set none), all in one undo step.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if d.bool("new") {
        return confirm_new(app, d);
    }
    let mut p = Value::Object(d.fields.clone());
    for k in ["x", "y", "width", "height"] {
        if d.fields.contains_key(k) {
            p[k] = json!(d.f64(k, 0.0));
        }
    }
    if let Some(o) = p.as_object_mut() {
        o.retain(|k, _| !k.starts_with("__"));
    }
    let index = p["index"].as_u64().and_then(|i| usize::try_from(i).ok()).unwrap_or(0);
    let was = app.session.active().and_then(|st| st.doc.artboards.get(index)).map(|a| a.rect);
    let changed = |k: &str, old: Option<f64>| p[k].as_f64().filter(|v| old.is_some_and(|old| (v - old).abs() > 1e-6));
    let (w, h) = (changed("width", was.map(|r| r.width())), changed("height", was.map(|r| r.height())));
    let part = |k: &str| d.fields.get(k).and_then(Value::as_u64).map_or(0, |v| v.min(2) as usize);
    let refi = part("__refy") * 3 + part("__refx");
    let mut entries = vec![];
    if w.is_some() || h.is_some() {
        for i in crate::panels::artboards::resized_with(app, index).into_iter().filter(|i| *i != index) {
            if let Some(r) = app.session.active().and_then(|st| st.doc.artboards.get(i)).map(|a| a.rect) {
                let n = artboard_rect(r, refi, None, None, w, h, false);
                entries.push(json!({"index": i, "x": n.x0, "y": n.y0, "width": n.width(), "height": n.height()}));
            }
        }
    }
    entries.insert(0, p);
    let common = json!({"moveArt": crate::panels::artboards::move_art(app)});
    let p = crate::panels::artboards::set_props_params(common, entries).ok_or("no artboard")?;
    run_and_close(app, "artboard.setProps", p)
}

/// `ui.newArtboard` (Alt-click on the Artboards panel's New Artboard): Artboard Options for a new
/// artboard (field `new`), filled in as `artboard.new {}` would make it: named Artboard N (N one
/// more than the number of artboards), as large as the last artboard and 20 pt to the right of it.
pub fn open_new(app: &mut VectorcraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let abs = &st.doc.artboards;
    let last = abs.last().map_or(vectorcraft_geom::Rect::new(0.0, 0.0, 612.0, 792.0), |a| a.rect);
    let fields = json!({
        "new": true, "name": format!("Artboard {}", abs.len() + 1),
        "x": last.x1 + 20.0, "y": last.y0, "width": last.width(), "height": last.height(),
    });
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(json!({ "dialog": KIND }))
}

/// OK for a new artboard: `artboard.new` with the fields (an empty name gives the default one),
/// and the new artboard becomes the active one.
fn confirm_new(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = json!({});
    for k in ["x", "y", "width", "height"] {
        p[k] = json!(d.f64(k, 0.0));
    }
    let name = d.str("name");
    if !name.trim().is_empty() {
        p["name"] = json!(name);
    }
    let r = run_and_close(app, "artboard.new", p)?;
    if let Some(i) = r["index"].as_u64().and_then(|i| usize::try_from(i).ok()) {
        crate::panels::artboards::select(app, i);
    }
    Ok(r)
}
