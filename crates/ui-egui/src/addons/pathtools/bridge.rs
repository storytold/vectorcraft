//! Smooth Bridge in the interface: its dialog (the arcs' radius as a percentage of the smaller
//! circle's radius, Keep Originals), which previews on the canvas and runs `addon.pathtools.bridge`
//! on OK.

use egui::Ui;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::dialogs::{DialogSpec, form};
use crate::state::Dialog;
use crate::widgets;

/// The command, and its dialog's kind.
pub const COMMAND: &str = "addon.pathtools.bridge";
pub const KIND: &str = "smoothBridge";
/// The radius the command takes (%), and its default.
const RADII: std::ops::RangeInclusive<f64> = 1.0..=1000.0;
const RADIUS: f64 = 100.0;
const LABEL_W: f32 = 60.0;
const FIELD_W: f32 = 90.0;

/// The dialog's fields to start with: the command's defaults.
pub(crate) fn fields() -> Value {
    json!({ "radius": RADIUS, "keepOriginals": false })
}

pub(crate) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Smooth Bridge").to_string(), body, confirm, preview: true, min_width: 280.0, ..DialogSpec::FORM };

/// The command's parameters from the dialog's fields.
fn params(d: &Dialog) -> Value {
    json!({ "radius": d.f64("radius", RADIUS), "keepOriginals": d.bool("keepOriginals") })
}

fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    widgets::label_row(ui, tl!("Radius:"), LABEL_W, |ui| {
        if let Some(v) = widgets::range_field(ui, ("dlg-num", "radius"), d.f64("radius", RADIUS), RADII, "%", 0, FIELD_W) {
            d.fields.insert("radius".into(), json!(v));
        }
    });
    form::check(ui, d, "keepOriginals", tl!("Keep Originals"));
    form::preview(app, ui, d, "Smooth Bridge", COMMAND, params(d));
    false
}

/// OK: run the command (keep its preview) and close.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    form::commit_preview(app, COMMAND, params(d))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use crate::tests_labels::painted_text;
    use crate::{VectorcraftApp, menus};

    fn circle(app: &mut VectorcraftApp, x: f64, r: f64) -> u64 {
        app.run("shape.ellipse", json!({"x": x - r, "y": 100.0 - r, "width": 2.0 * r, "height": 2.0 * r})).unwrap()["id"].as_u64().unwrap()
    }

    #[test]
    fn smooth_bridge_opens_its_dialog_previews_and_runs_on_ok() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let (a, b) = (circle(&mut app, 100.0, 20.0), circle(&mut app, 160.0, 20.0));
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        let entries = menus::menu_entries(&app);
        let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Path Precision Tools"] && e.label == "Smooth Bridge…").unwrap();
        let (cmd, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
        menus::invoke(&mut app, &cmd, p);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(super::KIND));
        let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        for s in ["Smooth Bridge", "Radius:", "Keep Originals", "Preview"] {
            assert!(shown.contains(s), "{s} in {shown}");
        }
        let count = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].children().unwrap().len();
        app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(true));
        painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        assert_eq!(count(&app), 1, "previewed: one outline in place of the circles");
        crate::dialogs::cancel(&mut app);
        assert_eq!(count(&app), 2, "rolled back");
        menus::invoke(&mut app, &cmd, serde_json::Value::Null);
        let n = app.session.doc().unwrap().history.undo.len();
        app.ui.dialog.as_mut().unwrap().fields.insert("keepOriginals".into(), json!(true));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.doc().unwrap().history.undo.len(), n + 1);
        assert_eq!(count(&app), 3, "the circles and the outline above them");
    }
}
