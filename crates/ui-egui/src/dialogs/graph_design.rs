//! Object › Graph › Design… and Marker…: the document's graph designs in a list.
//!
//! Graph Design: Save (Save Design) makes the selected art a design under the name typed, Paste (Paste Design) puts
//! a copy of the selected design into the document, Delete Design removes it (`graph.design`); each
//! runs at once and Close closes. Marker: the design (or the default marker) the selected graph
//! series draw their data points with; OK runs `graph.marker`.
//!
//! Fields: `selected` (a design's name; empty for the default marker) and, in Graph Design, `name`.

use serde_json::{Value, json};

use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kinds.
pub const DESIGN: &str = "graphDesign";
pub const MARKER: &str = "graphMarker";

pub(super) const DESIGN_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Graph Design").into(),
    body: design_body,
    confirm: |app, _| {
        app.ui.dialog = None;
        Ok(Value::Null)
    },
    ok: None,
    min_width: 380.0,
    ..DialogSpec::FORM
};

pub(super) const MARKER_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Graph Marker").into(),
    body: marker_body,
    confirm: |app, d| {
        let design = Some(d.str("selected")).filter(|s| !s.is_empty());
        run_and_close(app, "graph.marker", json!({ "design": design }))
    },
    min_width: 300.0,
    ..DialogSpec::FORM
};

/// Open Graph Design.
pub fn open_design(app: &mut VectorcraftApp) -> Result<Value, String> {
    let names = names(app)?;
    let first = names.first().cloned().unwrap_or_default();
    app.ui.dialog = Some(Dialog::new(DESIGN, json!({ "selected": first, "name": "" })));
    Ok(Value::Null)
}

/// Open Marker for the selected graph series, on the design they draw with.
pub fn open_marker(app: &mut VectorcraftApp) -> Result<Value, String> {
    let v = app.session.execute("graph.marker", &json!({})).map_err(|e| e.to_string())?;
    let current = v["design"].as_str().unwrap_or_default();
    app.ui.dialog = Some(Dialog::new(MARKER, json!({ "selected": current })));
    Ok(Value::Null)
}

/// The document's design names, read from the document (the dialogs draw every frame).
fn names(app: &VectorcraftApp) -> Result<Vec<String>, String> {
    let st = app.session.active().ok_or("no document open")?;
    Ok(st.doc.graph_designs.iter().map(|d| d.name.clone()).collect())
}

/// The designs as a list to pick from (with the default marker first when `default` is given).
fn list(ui: &mut egui::Ui, d: &mut Dialog, names: &[String], default: Option<&str>) {
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("graph-designs").min_scrolled_height(160.0).max_height(160.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(160.0);
            let entries = default.map(|label| ("", label)).into_iter().chain(names.iter().map(|n| (n.as_str(), n.as_str())));
            for (value, label) in entries {
                if ui.selectable_label(d.str("selected") == value, label).clicked() {
                    d.fields.insert("selected".into(), json!(value));
                }
            }
        });
    });
}

fn design_body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let names = names(app).unwrap_or_default();
    list(ui, d, &names, None);
    let selected = d.str("selected").to_string();
    let chosen = names.contains(&selected);
    let mut run = None;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if ui.add_enabled_ui(chosen, |ui| widgets::flat_button(ui, tl!("Paste"), 64.0)).inner.clicked() {
            run = Some(json!({ "paste": selected }));
        }
        if ui.add_enabled_ui(chosen, |ui| widgets::flat_button(ui, tl!("Delete"), 64.0)).inner.clicked() {
            run = Some(json!({ "delete": selected }));
        }
    });
    ui.add_space(10.0);
    let has_art = app.session.active().is_some_and(|st| !st.selection.is_empty());
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Name:"));
        form::text(ui, d, "name", 170.0);
        let named = !d.str("name").trim().is_empty();
        let save = ui.add_enabled_ui(has_art && named, |ui| widgets::flat_button(ui, tl!("Save"), 64.0)).inner;
        if save.clicked() {
            run = Some(json!({ "save": d.str("name") }));
        }
    });
    if let Some(p) = run {
        match app.run("graph.design", p.clone()) {
            Ok(r) => {
                if let Some(n) = r["name"].as_str() {
                    d.fields.insert("selected".into(), json!(n));
                    d.fields.insert("name".into(), json!(""));
                }
                // Paste Design leaves the copy selected to edit, as the dialog closes.
                if p.get("paste").is_some() {
                    return true;
                }
            }
            Err(e) => app.status(e),
        }
    }
    false
}

fn marker_body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let names = names(app).unwrap_or_default();
    list(ui, d, &names, Some(tl!("Default")));
    false
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::VectorcraftApp;
    use crate::state::Dialog;

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    #[test]
    fn the_graph_menu_opens_both_dialogs_and_they_draw() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let r = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [r]})).unwrap();
        app.run("graph.design", json!({"save": "Box"})).unwrap();
        crate::menus::invoke(&mut app, "graph.design", json!({}));
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(super::DESIGN));
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("selected"), "Box");
        frame(&mut app);
        frame(&mut app);
        assert!(app.ui.dialog.is_some());
        app.ui.dialog = None;
        // Marker needs a graph.
        crate::menus::invoke(&mut app, "graph.marker", json!({}));
        assert!(app.ui.dialog.is_none());
        let g = app.run("graph.create", json!({"type": "line", "x": 100, "y": 100, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [g]})).unwrap();
        crate::menus::invoke(&mut app, "graph.marker", json!({}));
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(super::MARKER));
        frame(&mut app);
        assert!(app.ui.dialog.is_some());
    }

    #[test]
    fn graph_design_saves_and_marker_applies_a_design() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let r = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [r]})).unwrap();
        super::open_design(&mut app).unwrap();
        let mut d = app.ui.dialog.take().unwrap();
        d.fields.insert("name".into(), json!("Box"));
        // What Save Design runs.
        app.run("graph.design", json!({"save": d.str("name")})).unwrap();
        let g = app.run("graph.create", json!({"type": "line", "x": 100, "y": 100, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [g]})).unwrap();
        super::open_marker(&mut app).unwrap();
        let mut d: Dialog = app.ui.dialog.take().unwrap();
        assert_eq!(d.str("selected"), "", "the default marker");
        d.fields.insert("selected".into(), json!("Box"));
        (super::MARKER_SPEC.confirm)(&mut app, &d).unwrap();
        let v = app.session.execute("graph.marker", &json!({})).unwrap();
        assert_eq!(v["design"], "Box");
        // Back to the default.
        super::open_marker(&mut app).unwrap();
        let mut d: Dialog = app.ui.dialog.take().unwrap();
        assert_eq!(d.str("selected"), "Box");
        d.fields.insert("selected".into(), json!(""));
        (super::MARKER_SPEC.confirm)(&mut app, &d).unwrap();
        assert!(app.session.execute("graph.marker", &json!({})).unwrap()["design"].is_null());
    }
}
