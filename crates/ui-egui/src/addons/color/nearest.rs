//! Find Nearest Swatch in the interface (Object › Plug-ins › Color Effects › Find Nearest
//! Swatch…): a dialog that picks a library (the document's swatches, or a colour library or book)
//! and Fill and Stroke, and lists each colour of the selection with its closest swatches and their
//! difference, as `addon.color.nearestSwatch` finds them; Apply runs
//! `addon.color.applyNearestSwatch`.

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_engine::cmd::swatchlib::DOCUMENT_SWATCHES;

use crate::VectorcraftApp;
use crate::dialogs::{DialogSpec, form};
use crate::panels::swatches;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets;

/// The command Apply runs, and the dialog's kind.
pub const COMMAND: &str = "addon.color.applyNearestSwatch";
pub const KIND: &str = "nearestSwatch";
/// The query that lists the matches, and how many of each colour's the dialog shows.
const FIND: &str = "addon.color.nearestSwatch";
const MATCHES: u64 = 3;
const LABEL_W: f32 = 60.0;
const CHIP: f32 = 14.0;

/// The dialog's fields to start with: the document's swatches, fills and strokes.
pub(crate) fn fields() -> Value {
    json!({ "library": DOCUMENT_SWATCHES, "fill": true, "stroke": true })
}

pub(crate) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Find Nearest Swatch").to_string(),
    body,
    confirm,
    ok: Some("Apply"),
    min_width: 340.0,
    ok_enabled: Some(|app, _| app.session.active().is_some_and(|st| !st.selection.is_empty())),
    ..DialogSpec::FORM
};

/// A colour chip of `hex`, then `text`.
fn chip_label(ui: &mut Ui, hex: &str, text: &str, color: egui::Color32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(CHIP, CHIP), egui::Sense::hover());
    widgets::swatch_tile(ui, r, &Paint::solid(Color::from_hex(hex).unwrap_or_default()), false, false);
    ui.label(egui::RichText::new(text).color(color));
}

fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let key = d.str("library");
    widgets::label_row(ui, tl!("Library:"), LABEL_W, |ui| {
        if let Some(k) = widgets::combo(ui, "nearest-library", &swatches::library_label(app, &key), 220.0, |ui| swatches::library_list(app, ui, &key))
        {
            d.fields.insert("library".into(), json!(k));
        }
    });
    ui.horizontal(|ui| {
        form::check(ui, d, "fill", tl!("Fill"));
        form::check(ui, d, "stroke", tl!("Stroke"));
    });
    ui.add_space(6.0);
    let t = Tokens::get(ui.ctx());
    if app.session.active().is_none_or(|st| st.selection.is_empty()) {
        ui.label(egui::RichText::new(tl!("Select objects to find the swatches nearest their colors.")).color(t.text_dim));
        return false;
    }
    let mut p = form::params(d);
    p["count"] = json!(MATCHES);
    let found = match app.session.execute(FIND, &p) {
        Ok(v) => v,
        Err(e) => {
            ui.label(egui::RichText::new(crate::i18n::message(crate::i18n::current(), &e.to_string())).color(t.text_dim));
            return false;
        }
    };
    let colors = found["colors"].as_array().cloned().unwrap_or_default();
    // Each colour, then its matches indented, closest first.
    egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
        for c in &colors {
            ui.horizontal(|ui| chip_label(ui, c["hex"].as_str().unwrap_or_default(), c["values"].as_str().unwrap_or_default(), t.text));
            for m in c["matches"].as_array().into_iter().flatten() {
                ui.horizontal(|ui| {
                    ui.add_space(CHIP + 8.0);
                    let de = m["deltaE"].as_f64().unwrap_or_default();
                    chip_label(ui, m["hex"].as_str().unwrap_or_default(), m["name"].as_str().unwrap_or_default(), t.text);
                    ui.label(egui::RichText::new(format!("ΔE {de:.1}")).color(t.text_dim));
                });
            }
        }
    });
    false
}

/// Apply: give each colour its nearest swatch and close.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    app.ui.dialog = None;
    app.run(COMMAND, form::params(d))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_color::Paint;
    use vectorcraft_engine::Session;

    use crate::tests_labels::painted_text;
    use crate::{VectorcraftApp, menus};

    #[test]
    fn find_nearest_swatch_lists_the_matches_and_applies_the_nearest() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("swatch.new", json!({"name": "Brand", "color": "#ff8000", "global": true})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
        app.run("paint.setFill", json!({"color": "#ff8010"})).unwrap();
        app.run("paint.setStroke", json!({"none": true})).unwrap();
        let entries = menus::menu_entries(&app);
        let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Color Effects"] && e.label == "Find Nearest Swatch…").unwrap();
        let (cmd, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
        menus::invoke(&mut app, &cmd, p);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(super::KIND));
        let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        for s in ["Find Nearest Swatch", "Library:", "Document Swatches", "Fill", "Stroke", "R=255 G=128 B=16", "Brand", "ΔE", "Apply"] {
            assert!(shown.contains(s), "{s} in {shown}");
        }
        // Another library lists its own swatches.
        app.ui.dialog.as_mut().unwrap().fields.insert("library".into(), json!("web-safe-216"));
        assert!(painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx())).contains("#FF6600"));
        app.ui.dialog.as_mut().unwrap().fields.insert("library".into(), json!("document"));
        let undo = app.session.doc().unwrap().history.undo.len();
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let st = app.session.active().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1);
        let fill = st.doc.node(st.selection.objects[0]).unwrap().appearance.fill_paint();
        assert!(matches!(fill, Paint::Solid { swatch: Some(n), .. } if n == "Brand"));
        // Nothing selected: the dialog says what to do and Apply is off.
        app.run("select.none", json!({})).unwrap();
        menus::invoke(&mut app, &cmd, serde_json::Value::Null);
        assert!(
            painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()))
                .contains("Select objects to find the swatches nearest their colors.")
        );
        assert!(!(super::SPEC.ok_enabled.unwrap())(&app, app.ui.dialog.as_ref().unwrap()), "Apply is off");
    }
}
