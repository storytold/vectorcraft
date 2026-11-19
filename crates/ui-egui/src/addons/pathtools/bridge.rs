//! Smooth Bridge in the interface: its dialog (the arcs' radius as a percentage of the smaller
//! circle's radius, or with Constant Gap that radius plus a gap; the reach, unavailable with two
//! circles selected; Keep Originals), which previews on the canvas and runs
//! `addon.pathtools.bridge` on OK.

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
/// The gap the command takes (pt), and the dialog's first one.
const GAPS: std::ops::RangeInclusive<f64> = 0.0..=1000.0;
const GAP: f64 = 4.0;
/// The reach the command takes (pt), and its default.
const REACHES: std::ops::RangeInclusive<f64> = 0.0..=10000.0;
const REACH: f64 = 10.0;
const LABEL_W: f32 = 60.0;
const FIELD_W: f32 = 90.0;

/// The dialog's fields to start with: the command's defaults.
pub(crate) fn fields() -> Value {
    json!({ "radius": RADIUS, "constantGap": false, "gap": GAP, "reach": REACH, "keepOriginals": false })
}

pub(crate) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Smooth Bridge").to_string(), body, confirm, preview: true, min_width: 280.0, ..DialogSpec::FORM };

/// The command's parameters from the dialog's fields: the radius, or with Constant Gap the gap.
fn params(d: &Dialog) -> Value {
    let (reach, keep) = (d.f64("reach", REACH), d.bool("keepOriginals"));
    if d.bool("constantGap") {
        json!({ "gap": d.f64("gap", GAP), "reach": reach, "keepOriginals": keep })
    } else {
        json!({ "radius": d.f64("radius", RADIUS), "reach": reach, "keepOriginals": keep })
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    form::check(ui, d, "constantGap", tl!("Constant Gap"));
    if d.bool("constantGap") {
        widgets::label_row(ui, tl!("Gap:"), LABEL_W, |ui| {
            form::length_within(ui, d, "gap", unit, FIELD_W, Some(GAPS));
        });
    } else {
        widgets::label_row(ui, tl!("Radius:"), LABEL_W, |ui| {
            if let Some(v) = widgets::range_field(ui, ("dlg-num", "radius"), d.f64("radius", RADIUS), RADII, "%", 0, FIELD_W) {
                d.fields.insert("radius".into(), json!(v));
            }
        });
    }
    // The reach applies to three or more circles; two are bridged at any distance. Counted once,
    // before a preview replaces the selection with its result.
    let many = match d.fields.get("many").and_then(Value::as_bool) {
        Some(many) => many,
        None => {
            let many = app.session.doc().map(|doc| doc.selection.objects.len() > 2).unwrap_or(true);
            d.fields.insert("many".into(), json!(many));
            many
        }
    };
    ui.add_enabled_ui(many, |ui| {
        widgets::label_row(ui, tl!("Reach:"), LABEL_W, |ui| {
            form::length_within(ui, d, "reach", unit, FIELD_W, Some(REACHES));
        });
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
        for s in ["Smooth Bridge", "Constant Gap", "Radius:", "Reach:", "Keep Originals", "Preview"] {
            assert!(shown.contains(s), "{s} in {shown}");
        }
        assert!(!shown.contains("Gap:"), "{shown}");
        assert!(!app.ui.dialog.as_ref().unwrap().bool("many"), "Reach unavailable for two circles");
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

    #[test]
    fn constant_gap_shows_the_gap_and_bridges_many_circles_within_reach() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        // Three circles 4 apart in a row and one far off.
        let ids: Vec<u64> = [100.0, 124.0, 148.0, 300.0].iter().map(|&x| circle(&mut app, x, 10.0)).collect();
        app.run("select.set", json!({ "ids": ids })).unwrap();
        menus::invoke(&mut app, super::COMMAND, serde_json::Value::Null);
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("constantGap".into(), json!(true));
        let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        assert!(shown.contains("Gap:") && !shown.contains("Radius:"), "{shown}");
        // A preview leaves two objects selected (one shape and the far circle); Reach stays
        // available for the four circles the command still bridges.
        app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(true));
        painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        assert_eq!(app.session.doc().unwrap().selection.objects.len(), 2, "previewed");
        assert!(app.ui.dialog.as_ref().unwrap().bool("many"), "Reach available for four circles");
        let p = super::params(app.ui.dialog.as_ref().unwrap());
        assert_eq!(p, json!({"gap": 4.0, "reach": 10.0, "keepOriginals": false}), "the gap, not the radius");
        crate::dialogs::confirm(&mut app).unwrap();
        let top: Vec<u64> = app.session.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id.0).collect();
        assert_eq!(top, [ids[2], ids[3]], "one shape in the topmost joined circle's place, and the far circle");
    }
}
