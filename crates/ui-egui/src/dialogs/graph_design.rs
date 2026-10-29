//! Object › Graph › Design…, Column… and Marker…: the document's graph designs in a list.
//!
//! Graph Design: the designs, a preview of the selected one, and New Design (the selected art as a
//! design named "New Design"), Rename (to the name typed), Delete Design (the selected designs),
//! Paste Design (a copy of the selected design into the document, selected, to edit) and Select
//! Unused (the designs no graph draws with), each run at once by `graph.design`; Close closes.
//! Column: the design the selected graph series draw their columns or bars with, or None (plain
//! columns), a preview of it, Column Type, and for a repeating design Each Design Represents and
//! For Fractions; OK runs `graph.column`.
//! Marker: the design the selected graph series draw their data points with, or None (the default
//! square marker); OK runs `graph.marker`.
//!
//! Fields: `selected` (the selected designs' names; Column and Marker: one name, null for None),
//! in Graph Design `name` (the name Rename gives), and in Column `columnType`, `each` and
//! `fractions`.

use serde_json::{Value, json};

use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kinds.
pub const DESIGN: &str = "graphDesign";
pub const COLUMN: &str = "graphColumn";
pub const MARKER: &str = "graphMarker";

/// The preview's size.
const PREVIEW: f32 = 120.0;

pub(super) const DESIGN_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Graph Design").into(),
    body: design_body,
    confirm: |app, _| {
        app.ui.dialog = None;
        Ok(Value::Null)
    },
    ok: None,
    min_width: 420.0,
    ..DialogSpec::FORM
};

pub(super) const COLUMN_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Graph Column").into(),
    body: column_body,
    confirm: |app, d| {
        let p = json!({
            "design": d.fields.get("selected").and_then(Value::as_str),
            "columnType": d.str("columnType"),
            "each": d.f64("each", vectorcraft_doc::ColumnDesign::EACH),
            "fractions": d.str("fractions"),
        });
        run_and_close(app, "graph.column", p)
    },
    min_width: 420.0,
    ..DialogSpec::FORM
};

pub(super) const MARKER_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Graph Marker").into(),
    body: marker_body,
    confirm: |app, d| run_and_close(app, "graph.marker", json!({ "design": d.fields.get("selected").and_then(Value::as_str) })),
    min_width: 300.0,
    ..DialogSpec::FORM
};

/// Open Graph Design.
pub fn open_design(app: &mut VectorcraftApp) -> Result<Value, String> {
    let names = names(app)?;
    let first: Vec<String> = names.first().cloned().into_iter().collect();
    let name = first.first().cloned().unwrap_or_default();
    app.ui.dialog = Some(Dialog::new(DESIGN, json!({ "selected": first, "name": name })));
    Ok(Value::Null)
}

/// Open Column for the selected graph series, on the design and options they draw their columns with (None for plain
/// columns).
pub fn open_column(app: &mut VectorcraftApp) -> Result<Value, String> {
    let v = app.session.execute("graph.column", &json!({})).map_err(|e| e.to_string())?;
    let fields = json!({ "selected": v["design"].as_str(), "columnType": v["columnType"], "each": v["each"], "fractions": v["fractions"] });
    app.ui.dialog = Some(Dialog::new(COLUMN, fields));
    Ok(Value::Null)
}

/// Open Marker for the selected graph series, on the design they draw with (None for the default).
pub fn open_marker(app: &mut VectorcraftApp) -> Result<Value, String> {
    let v = app.session.execute("graph.marker", &json!({})).map_err(|e| e.to_string())?;
    app.ui.dialog = Some(Dialog::new(MARKER, json!({ "selected": v["design"].as_str() })));
    Ok(Value::Null)
}

/// The document's design names, read from the document (the dialogs draw every frame).
fn names(app: &VectorcraftApp) -> Result<Vec<String>, String> {
    let st = app.session.active().ok_or("no document open")?;
    Ok(st.doc.graph_designs.iter().map(|d| d.name.clone()).collect())
}

/// Graph Design's selected designs.
fn selected(d: &Dialog) -> Vec<String> {
    d.fields.get("selected").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn select(d: &mut Dialog, names: Vec<String>) {
    d.fields.insert("name".into(), json!(names.first().cloned().unwrap_or_default()));
    d.fields.insert("selected".into(), json!(names));
}

/// A design's art drawn in `r` (cached by the art's identity).
fn preview(ui: &egui::Ui, app: &VectorcraftApp, name: &str, r: egui::Rect) {
    use std::cell::RefCell;
    use std::collections::HashMap;
    thread_local! {
        static RENDERER: RefCell<vectorcraft_render::Renderer> = RefCell::new(vectorcraft_render::Renderer::new());
        static CACHE: crate::graphics::TexCache<HashMap<usize, egui::TextureHandle>> = crate::graphics::TexCache::default();
    }
    let Some(st) = app.session.active() else { return };
    let Some(art) = st.doc.graph_designs.iter().find(|x| x.name == name).map(|x| x.art.clone()) else { return };
    let key = std::sync::Arc::as_ptr(&art) as usize;
    let px = (r.width() * ui.ctx().pixels_per_point()).round() as u32;
    let tex = CACHE.with(|c| c.borrow().get(&key).cloned()).or_else(|| {
        let img = RENDERER.with(|rr| rr.borrow_mut().render_node_thumbnail(&st.doc, &art, px.max(16), Some([255, 255, 255, 255])))?;
        let color = crate::canvas::color_image(&img);
        let tex = ui.ctx().load_texture(format!("graph-design-{key}"), color, egui::TextureOptions::LINEAR);
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            if c.len() > 200 {
                c.clear();
            }
            c.insert(key, tex.clone());
        });
        Some(tex)
    });
    if let Some(t) = tex {
        ui.painter().image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
}

fn design_body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let names = names(app).unwrap_or_default();
    let chosen: Vec<String> = selected(d).into_iter().filter(|n| names.contains(n)).collect();
    let one = chosen.first().cloned();
    let has_art = app.session.active().is_some_and(|st| !st.selection.is_empty());
    let mut run = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(220.0);
            // Click picks a design; Cmd/Ctrl-click adds or removes one.
            widgets::list_box(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("graph-designs").min_scrolled_height(160.0).max_height(160.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(160.0);
                    for n in &names {
                        if ui.selectable_label(chosen.contains(n), n.as_str()).clicked() {
                            let mut now = chosen.clone();
                            if ui.input(|i| i.modifiers.command) {
                                if let Some(k) = now.iter().position(|x| x == n) {
                                    now.remove(k);
                                } else {
                                    now.push(n.clone());
                                }
                            } else {
                                now = vec![n.clone()];
                            }
                            select(d, now);
                        }
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                widgets::dim_label(ui, tl!("Name:"));
                form::text(ui, d, "name", 150.0);
            });
        });
        ui.add_space(10.0);
        ui.vertical(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(PREVIEW, PREVIEW), egui::Sense::hover());
            ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, egui::Color32::from_gray(160)), egui::StrokeKind::Inside);
            if let Some(n) = &one {
                preview(ui, app, n, r.shrink(4.0));
            }
            ui.add_space(6.0);
            let button =
                |ui: &mut egui::Ui, on: bool, label: &str| ui.add_enabled_ui(on, |ui| widgets::flat_button(ui, label, PREVIEW)).inner.clicked();
            if button(ui, has_art, tl!("New Design")) {
                run = Some(json!({ "new": true }));
            }
            let to = d.str("name");
            if button(ui, chosen.len() == 1 && !to.trim().is_empty(), tl!("Rename")) {
                run = one.clone().map(|n| json!({ "rename": n, "to": to }));
            }
            if button(ui, !chosen.is_empty(), tl!("Delete Design")) {
                run = Some(json!({ "delete": chosen }));
            }
            if button(ui, chosen.len() == 1, tl!("Paste Design")) {
                run = one.clone().map(|n| json!({ "paste": n }));
            }
            if button(ui, !names.is_empty(), tl!("Select Unused")) {
                run = Some(json!({ "selectUnused": true }));
            }
        });
    });
    if let Some(p) = run {
        match app.run("graph.design", p.clone()) {
            Ok(r) => {
                if let Some(n) = r["name"].as_str() {
                    select(d, vec![n.to_string()]);
                }
                if let Some(unused) = r["unused"].as_array().filter(|_| p.get("selectUnused").is_some()) {
                    select(d, unused.iter().filter_map(|v| v.as_str().map(str::to_string)).collect());
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

/// The list of Column and Marker: None, then the designs `names`; a click picks one into `selected` (null for None).
fn one_design_list(ui: &mut egui::Ui, d: &mut Dialog, names: &[String], salt: &str) {
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::vertical().id_salt(salt).min_scrolled_height(160.0).max_height(160.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(160.0);
            let current = d.fields.get("selected").and_then(Value::as_str).map(str::to_string);
            if ui.selectable_label(current.is_none(), tl!("None")).clicked() {
                d.fields.insert("selected".into(), Value::Null);
            }
            for n in names {
                if ui.selectable_label(current.as_deref() == Some(n.as_str()), n.as_str()).clicked() {
                    d.fields.insert("selected".into(), json!(n));
                }
            }
        });
    });
}

fn column_body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let names = names(app).unwrap_or_default();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(220.0);
            one_design_list(ui, d, &names, "graph-columns");
        });
        ui.add_space(10.0);
        let (r, _) = ui.allocate_exact_size(egui::vec2(PREVIEW, PREVIEW), egui::Sense::hover());
        ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, egui::Color32::from_gray(160)), egui::StrokeKind::Inside);
        if let Some(n) = d.fields.get("selected").and_then(Value::as_str).filter(|n| names.iter().any(|x| x == n)) {
            preview(ui, app, n, r.shrink(4.0));
        }
    });
    ui.add_space(8.0);
    const LABEL_W: f32 = 150.0;
    let types =
        [("vertical", tl!("Vertically Scaled")), ("uniform", tl!("Uniformly Scaled")), ("repeating", tl!("Repeating")), ("sliding", tl!("Sliding"))];
    form::choice(ui, d, "columnType", tl!("Column Type:"), (LABEL_W, 160.0), &types);
    // Each Design Represents and For Fractions are for repeating designs.
    ui.add_enabled_ui(d.str("columnType") == "repeating", |ui| {
        widgets::label_row(ui, tl!("Each Design Represents:"), LABEL_W, |ui| {
            let each = d.f64("each", vectorcraft_doc::ColumnDesign::EACH);
            if let Some(v) = widgets::range_field(ui, ("graph-column", "each"), each, EACH_RANGE, "", 4, 72.0) {
                d.fields.insert("each".into(), json!(v));
            }
        });
        let fractions = [("chop", tl!("Chop Design")), ("scale", tl!("Scale Design"))];
        form::choice(ui, d, "fractions", tl!("For Fractions:"), (LABEL_W, 160.0), &fractions);
    });
    false
}

/// The values Each Design Represents takes in the dialog (`graph.column` takes any value more than 0).
const EACH_RANGE: std::ops::RangeInclusive<f64> = 0.0001..=1_000_000_000.0;

fn marker_body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let names = names(app).unwrap_or_default();
    // A list as wide as Graph Design's, not the whole window.
    ui.set_width(220.0);
    one_design_list(ui, d, &names, "graph-markers");
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

    fn app_with_design() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let r = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [r]})).unwrap();
        app.run("graph.design", json!({"new": true})).unwrap();
        app
    }

    #[test]
    fn the_graph_menu_opens_both_dialogs_and_they_draw_with_a_preview() {
        let mut app = app_with_design();
        crate::menus::invoke(&mut app, "graph.design", json!({}));
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(d.kind, super::DESIGN);
        assert_eq!((super::selected(d), d.str("name")), (vec!["New Design".to_string()], "New Design".to_string()));
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
    fn marker_lists_the_designs_and_applies_the_one_picked() {
        let mut app = app_with_design();
        let g = app.run("graph.create", json!({"type": "line", "x": 100, "y": 100, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [g]})).unwrap();
        super::open_marker(&mut app).unwrap();
        let mut d: Dialog = app.ui.dialog.take().unwrap();
        assert_eq!(d.fields["selected"], json!(null), "None: the default marker");
        d.fields.insert("selected".into(), json!("New Design"));
        (super::MARKER_SPEC.confirm)(&mut app, &d).unwrap();
        assert_eq!(app.session.execute("graph.marker", &json!({})).unwrap()["design"], "New Design");
        super::open_marker(&mut app).unwrap();
        let mut d: Dialog = app.ui.dialog.take().unwrap();
        assert_eq!(d.str("selected"), "New Design");
        // None goes back to the default marker.
        d.fields.insert("selected".into(), json!(null));
        (super::MARKER_SPEC.confirm)(&mut app, &d).unwrap();
        assert_eq!(app.session.execute("graph.marker", &json!({})).unwrap()["design"], json!(null));
    }

    #[test]
    fn column_opens_from_the_graph_menu_and_applies_the_design_and_its_options() {
        let mut app = app_with_design();
        let g = app.run("graph.create", json!({"type": "column", "x": 100, "y": 100, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [g]})).unwrap();
        crate::menus::invoke(&mut app, "graph.column", json!({}));
        let mut d: Dialog = app.ui.dialog.clone().unwrap();
        assert_eq!(d.kind, super::COLUMN);
        assert_eq!(
            (d.fields["selected"].clone(), d.str("columnType"), d.f64("each", 0.0), d.str("fractions")),
            (json!(null), "vertical".into(), 10.0, "chop".into())
        );
        frame(&mut app);
        assert!(app.ui.dialog.is_some());
        d.fields.insert("selected".into(), json!("New Design"));
        d.fields.insert("columnType".into(), json!("repeating"));
        d.fields.insert("each".into(), json!(0.5));
        d.fields.insert("fractions".into(), json!("scale"));
        (super::COLUMN_SPEC.confirm)(&mut app, &d).unwrap();
        let v = app.session.execute("graph.column", &json!({})).unwrap();
        assert_eq!(v, json!({"designs": ["New Design"], "design": "New Design", "columnType": "repeating", "each": 0.5, "fractions": "scale"}));
        // It opens again on what the series draw with, and None goes back to plain columns.
        super::open_column(&mut app).unwrap();
        let mut d: Dialog = app.ui.dialog.take().unwrap();
        assert_eq!((d.str("selected"), d.str("columnType"), d.f64("each", 0.0)), ("New Design".into(), "repeating".into(), 0.5));
        d.fields.insert("selected".into(), json!(null));
        (super::COLUMN_SPEC.confirm)(&mut app, &d).unwrap();
        assert_eq!(app.session.execute("graph.column", &json!({})).unwrap()["design"], json!(null));
    }
}
