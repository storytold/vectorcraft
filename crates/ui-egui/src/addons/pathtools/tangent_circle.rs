//! Tangent Circle in the interface: its dialog (the radius, with two shapes selected, and which
//! circle to draw, or all), which previews the circles on the canvas and runs
//! `addon.pathtools.tangentCircle` on OK.

use egui::Ui;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::dialogs::{DialogSpec, form};
use crate::state::Dialog;
use crate::widgets;

/// The command, and its dialog's kind.
pub const COMMAND: &str = "addon.pathtools.tangentCircle";
pub const KIND: &str = "tangentCircle";
/// The radius the command takes (pt).
const RADII: std::ops::RangeInclusive<f64> = 0.01..=1e6;
/// The most circles the command lists.
const MAX_CIRCLES: u64 = 64;
const LABEL_W: f32 = 60.0;
const FIELD_W: f32 = 90.0;
/// How many circles the last preview found (a private field).
const FOUND: &str = "__found";
/// The parameters the last preview ran with (a private field).
const TRIED: &str = "__tried";

/// How many objects were selected when the dialog opened (a private field): two take a radius,
/// three set it themselves. The preview selects the circles it draws, so the count is kept.
const SHAPES: &str = "__shapes";

/// The dialog's fields to start with: a radius of 36 pt, every circle, Preview on (the Circle menu
/// lists the circles the last preview found), and the selection's size.
pub(crate) fn fields(app: &VectorcraftApp) -> Value {
    let shapes = app.session.active().map_or(0, |st| st.selection.objects.len());
    json!({ "radius": 36, "circle": 0, "preview": true, SHAPES: shapes })
}

pub(crate) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Tangent Circle").to_string(), body, confirm, preview: true, min_width: 280.0, ..DialogSpec::FORM };

fn shapes(d: &Dialog) -> u64 {
    d.fields.get(SHAPES).and_then(Value::as_u64).unwrap_or(0)
}

/// The command's parameters: the radius only with two shapes.
fn params(d: &Dialog) -> Value {
    let mut p = json!({ "circle": d.fields.get("circle").cloned().unwrap_or(json!(0)) });
    if shapes(d) == 2
        && let (Some(o), Some(r)) = (p.as_object_mut(), d.fields.get("radius"))
    {
        o.insert("radius".into(), r.clone());
    }
    p
}

fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    widgets::label_row(ui, tl!("Radius:"), LABEL_W, |ui| {
        ui.add_enabled_ui(shapes(d) == 2, |ui| form::length_within(ui, d, "radius", unit, FIELD_W, Some(RADII)));
    });
    // The command lists at most 64 circles; the menu never holds more.
    let found = d.fields.get(FOUND).and_then(Value::as_u64).unwrap_or(0).min(MAX_CIRCLES);
    let pick = d.fields.get("circle").and_then(Value::as_u64).unwrap_or(0).min(MAX_CIRCLES);
    let mut options = vec![tl!("All").to_string()];
    options.extend((1..=found.max(pick)).map(|n| n.to_string()));
    let names: Vec<&str> = options.iter().map(String::as_str).collect();
    let current = options.get(usize::try_from(pick).unwrap_or(0)).cloned().unwrap_or_default();
    widgets::label_row(ui, tl!("Circle:"), LABEL_W, |ui| {
        if let Some(i) = widgets::dropdown_names(ui, "tangent-circle-pick", &current, &names, FIELD_W) {
            d.fields.insert("circle".into(), json!(i));
        }
    });
    let p = params(d);
    let fresh = d.fields.get(TRIED) != Some(&p);
    match form::preview_result(app, ui, d, "Tangent Circle", COMMAND, p.clone()) {
        Some(r) => {
            d.fields.insert(FOUND.into(), r["found"].clone());
        }
        // A new radius or pick found nothing: the menu no longer lists the earlier circles, and a
        // pick past the circles left shows them all again.
        None if fresh && d.bool("preview") => {
            d.fields.insert(FOUND.into(), json!(0));
            if pick > 0 {
                d.fields.insert("circle".into(), json!(0));
            }
        }
        None => {}
    }
    d.fields.insert(TRIED.into(), p);
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

    fn circle(app: &mut VectorcraftApp, cx: f64, cy: f64, r: f64) -> u64 {
        app.run("shape.ellipse", json!({"x": cx - r, "y": cy - r, "width": 2.0 * r, "height": 2.0 * r})).unwrap()["id"].as_u64().unwrap()
    }

    #[test]
    fn tangent_circle_previews_picks_one_cancels_and_runs_on_ok() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let a = circle(&mut app, 100.0, 100.0, 10.0);
        let b = circle(&mut app, 130.0, 100.0, 10.0);
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        let entries = menus::menu_entries(&app);
        let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Path Precision Tools"] && e.label == "Tangent Circle…").unwrap();
        let (cmd, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
        menus::invoke(&mut app, &cmd, p);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(super::KIND));
        let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        for s in ["Tangent Circle", "Radius:", "Circle:", "All", "Preview"] {
            assert!(shown.contains(s), "{s} in {shown}");
        }
        let count = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].children().unwrap().len();
        // Preview is on from the start: radius 36 gives eight circles, and the menu lists them.
        assert_eq!(count(&app), 10, "previewed on opening: {}", app.ui.status);
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["__found"], json!(8));
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("radius".into(), json!(20));
        painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        // Six circles of radius 20 touch both: all previewed.
        assert_eq!(count(&app), 8, "previewed: {}", app.ui.status);
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["__found"], json!(6));
        app.ui.dialog.as_mut().unwrap().fields.insert("circle".into(), json!(2));
        painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        assert_eq!(count(&app), 3, "one previewed");
        // A pick past the circles found (a control request can set any number) goes back to All.
        app.ui.dialog.as_mut().unwrap().fields.insert("circle".into(), json!(u64::MAX));
        painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["circle"], json!(0));
        painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["__found"], json!(6));
        // Radius 2 cannot reach across the 10 pt gap: the menu lists no circles.
        app.ui.dialog.as_mut().unwrap().fields.insert("radius".into(), json!(2));
        painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["__found"], json!(0), "{}", app.ui.status);
        crate::dialogs::cancel(&mut app);
        assert_eq!(count(&app), 2, "rolled back");
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        menus::invoke(&mut app, &cmd, serde_json::Value::Null);
        let n = app.session.doc().unwrap().history.undo.len();
        app.ui.dialog.as_mut().unwrap().fields.insert("radius".into(), json!(5));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.doc().unwrap().history.undo.len(), n + 1);
        // Radius 5: the one circle in the gap.
        assert_eq!(count(&app), 3);
    }
}
