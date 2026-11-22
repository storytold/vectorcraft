//! Set Corners in the interface: its dialog (radius, corner kind, which anchors: all, the
//! odd-numbered or the even-numbered ones, and the range of corner angles to round), which
//! previews on the canvas and runs `addon.pathtools.cornerSet` on OK, one undo step. It opens on
//! the Corner Tool's radius and kind, every corner angle (0° to 180°).

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_tools::addons::corner_tool::{ID, MAX_RADIUS, RADIUS};

use crate::VectorcraftApp;
use crate::dialogs::{DialogSpec, form};
use crate::state::Dialog;
use crate::widgets;

/// The command, and its dialog's kind.
pub const COMMAND: &str = "addon.pathtools.cornerSet";
pub const KIND: &str = "setCorners";
/// The corner kinds, as the command names them, and their labels.
const KINDS: [(&str, &str); 3] = [("round", "Round"), ("invertedRound", "Inverted Round"), ("chamfer", "Chamfer")];
/// Which anchors, as the command's `every` names them, and their labels.
const EVERY: [(&str, &str); 3] = [("all", "All Anchors"), ("odd", "Odd-Numbered Anchors"), ("even", "Even-Numbered Anchors")];
const LABEL_W: f32 = 64.0;
const FIELD_W: f32 = 170.0;
const ANGLE_W: f32 = 64.0;
/// The corner angles the command takes (degrees).
const ANGLES: std::ops::RangeInclusive<f64> = 0.0..=180.0;

/// The dialog's fields to start with: the Corner Tool's radius and kind, every anchor, Preview on.
pub(crate) fn fields(app: &VectorcraftApp) -> Value {
    let options = app.session.tool_options_of(ID);
    let radius = options["radius"].as_f64().unwrap_or(RADIUS);
    let kind = options["kind"].as_str().unwrap_or("round").to_string();
    json!({ "radius": radius, "kind": kind, "every": "all", "minAngle": 0.0, "maxAngle": 180.0, "preview": true })
}

pub(crate) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Set Corners").to_string(), body, confirm, preview: true, min_width: 280.0, ..DialogSpec::FORM };

fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    widgets::label_row(ui, tl!("Radius:"), LABEL_W, |ui| {
        form::length_within(ui, d, "radius", unit, FIELD_W, Some(0.0..=MAX_RADIUS));
    });
    form::choice(ui, d, "kind", tl!("Corner:"), (LABEL_W, FIELD_W), &KINDS);
    form::choice(ui, d, "every", tl!("Anchors:"), (LABEL_W, FIELD_W), &EVERY);
    // Only the corners whose sides meet at an angle in this range: small for a sharp spike, near
    // 180° for a shallow bend. Typing one end past the other moves the other along.
    let (min, max) = (d.f64("minAngle", 0.0), d.f64("maxAngle", 180.0));
    widgets::label_row(ui, tl!("Angle:"), LABEL_W, |ui| {
        if let Some(v) = widgets::range_field_named(ui, "set-corners-min-angle", Some(tl!("Minimum angle")), min, ANGLES, "°", 1, ANGLE_W) {
            d.fields.insert("minAngle".into(), json!(v));
            if v > max {
                d.fields.insert("maxAngle".into(), json!(v));
            }
        }
        ui.label(tl!("to"));
        if let Some(v) = widgets::range_field_named(ui, "set-corners-max-angle", Some(tl!("Maximum angle")), max, ANGLES, "°", 1, ANGLE_W) {
            d.fields.insert("maxAngle".into(), json!(v));
            if v < min {
                d.fields.insert("minAngle".into(), json!(v));
            }
        }
    });
    form::preview(app, ui, d, "Set Corners", COMMAND, form::params(d));
    false
}

/// OK: keep the preview (or run the command) and close.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    form::commit_preview(app, COMMAND, form::params(d))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_doc::{LiveCorners, NodeId};
    use vectorcraft_engine::Session;

    use crate::tests_labels::painted_text;
    use crate::{VectorcraftApp, menus};

    /// The anchor indices of `id`'s uncut outline with a corner drawn.
    fn cut(app: &VectorcraftApp, id: u64) -> Vec<usize> {
        let doc = &app.session.doc().unwrap().doc;
        let lc = LiveCorners::of(doc.node(NodeId(id)).unwrap()).unwrap();
        (0..lc.base.anchor_count()).filter(|k| lc.drawn_radius(*k) > 0.0).collect()
    }

    #[test]
    fn set_corners_opens_from_the_menu_previews_odd_anchors_and_commits_one_step() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let anchors = json!([{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}, {"x": 0, "y": 100}, {"x": -50, "y": 50}]);
        let id = app.run("path.create", json!({"anchors": anchors, "closed": true})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        let entries = menus::menu_entries(&app);
        let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Path Precision Tools"] && e.label == "Set Corners…").unwrap();
        let (cmd, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
        menus::invoke(&mut app, &cmd, p);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(super::KIND));
        let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        for s in ["Set Corners", "Radius:", "Corner:", "Anchors:", "All Anchors", "Preview"] {
            assert!(shown.contains(s), "{s} in {shown}");
        }
        assert_eq!(cut(&app, id), vec![0, 1, 2, 3, 4], "all previewed");
        let n = app.session.doc().unwrap().history.undo.len();
        app.ui.dialog.as_mut().unwrap().fields.insert("every".into(), json!("odd"));
        let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        assert!(shown.contains("Odd-Numbered Anchors"), "{shown}");
        assert_eq!(cut(&app, id), vec![0, 2, 4], "odd previewed");
        crate::dialogs::cancel(&mut app);
        assert_eq!(cut(&app, id), Vec::<usize>::new(), "rolled back");
        menus::invoke(&mut app, &cmd, serde_json::Value::Null);
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("every".into(), json!("even"));
        d.fields.insert("radius".into(), json!(6));
        painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.doc().unwrap().history.undo.len(), n + 1);
        assert_eq!(cut(&app, id), vec![1, 3]);
    }

    #[test]
    fn set_corners_previews_only_the_corners_in_its_angle_range() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        // A house: square corners at 0, 1 and 3, the roof's 135° corners at 2 and 4.
        let anchors = json!([{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}, {"x": 50, "y": 150}, {"x": 0, "y": 100}]);
        let id = app.run("path.create", json!({"anchors": anchors, "closed": true})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        menus::invoke(&mut app, super::COMMAND, json!({}));
        let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        for s in ["Angle:", "to", "0°", "180°"] {
            assert!(shown.contains(s), "{s} in {shown}");
        }
        let n = app.session.doc().unwrap().history.undo.len();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("minAngle".into(), json!(120));
        painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        assert_eq!(cut(&app, id), vec![2, 4], "the roof previewed");
        app.ui.dialog.as_mut().unwrap().fields.insert("maxAngle".into(), json!(100));
        app.ui.dialog.as_mut().unwrap().fields.insert("minAngle".into(), json!(80));
        painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        assert_eq!(cut(&app, id), vec![0, 1, 3], "the square corners previewed");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(app.session.doc().unwrap().history.undo.len(), n + 1);
        assert_eq!(cut(&app, id), vec![0, 1, 3]);
    }
}
