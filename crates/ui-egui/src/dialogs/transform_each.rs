//! Object → Transform → Transform Each: scale, move and rotate every selected object about its own
//! reference point, with reflection, randomness, Transform Objects, Transform Patterns, Scale
//! Corners and Scale Strokes & Effects (`object.transformEach`), previewed live on the canvas. Its Scale and Move sliders, Rotate dial
//! and reflection, reference point and Random options are the Transform effect's too.
//!
//! Fields: `scaleH`, `scaleV` (%), `moveH`, `moveV` (pt), `rotate` (°), `reflectX`, `reflectY`,
//! `random`, `reference` (0..8, the 9-point grid), `copy` (set by the Copy button, which confirms),
//! `objects`, `patterns`, `corners`, `strokes` (the last two default to the preferences, which OK
//! updates) and `preview`. It opens on the values it was last confirmed with, Copy aside (#1732).

use serde_json::{Value, json};

use super::transform::{add_object_options, add_scale_options, object_options, scale_options, with_scale_options};
use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets;

/// The dialog kind of Transform Each.
pub const KIND: &str = "transformEach";

const CMD: &str = "object.transformEach";

/// OK transforms each object; Copy, left of Cancel as in the documented dialog, transforms a copy
/// of each ([Scale multiple objects](https://helpx.adobe.com/illustrator/desktop/manage-objects/reshape-transform-objects/scale-multiple-objects.html)).
pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Transform Each").into(),
    body,
    confirm,
    alternate: Some(("Copy", "copy")),
    preview: true,
    min_width: 360.0,
    inputs: |_| &["objects", "patterns", "corners", "strokes"],
    ..DialogSpec::FORM
};

/// The fields a fresh dialog starts with.
pub fn fields() -> Value {
    json!({"scaleH": 100, "scaleV": 100, "moveH": 0, "moveV": 0, "rotate": 0, "reflectX": false, "reflectY": false, "random": false, "reference": 4, "copy": false})
}

/// The fields kept from the last OK or Copy (`UiState::transform_each`): all but Copy, Transform
/// Patterns (the preference it starts from) and the scale options (preferences too).
const REMEMBERED: [&str; 11] = ["scaleH", "scaleV", "moveH", "moveV", "rotate", "reflectX", "reflectY", "random", "reference", "objects", "preview"];

/// The fields the menu item opens with: [`fields`], with the values it was last confirmed with.
/// A kept value of the wrong kind (a hand-edited file) is left out.
pub fn open_fields(app: &VectorcraftApp) -> Value {
    let mut out = fields();
    for k in REMEMBERED {
        match app.ui.transform_each.get(k) {
            Some(v @ Value::Number(_)) if out.get(k).is_some_and(Value::is_number) => out[k] = v.clone(),
            Some(v @ Value::Bool(_)) if out.get(k).is_none_or(Value::is_boolean) => out[k] = v.clone(),
            _ => {}
        }
    }
    out
}

/// The command's parameters without the scale options.
fn params(d: &Dialog) -> Value {
    let f = |k: &str, v: f64| d.f64(k, v);
    json!({
        "scaleH": f("scaleH", 100.0), "scaleV": f("scaleV", 100.0), "moveH": f("moveH", 0.0), "moveV": f("moveV", 0.0),
        "rotate": f("rotate", 0.0), "reflectX": d.bool("reflectX"), "reflectY": d.bool("reflectY"), "random": d.bool("random"),
        "reference": f("reference", 4.0).clamp(0.0, 8.0) as u64, "copy": d.bool("copy"),
    })
}

/// Width of the label column and the value fields of [`sections`].
const LABEL_W: f32 = 72.0;
const FIELD_W: f32 = 70.0;
/// Largest scale (%) the fields take (as `object.transformEach` and the Transform effect).
const MAX_SCALE: f64 = 100_000.0;

/// The Scale, Move and Rotate sections of Transform Each and the Transform effect: sliders for
/// `scaleH` and `scaleV` (rails 0–200 %) and for `moveH` and `moveV` (rails ±1000 pt, fields in
/// `units`), and an angle dial for `rotate` (counter-clockwise). The fields take values past the
/// rails.
pub(super) fn sections(ui: &mut egui::Ui, d: &mut Dialog, units: vectorcraft_doc::Unit) {
    let rail = Tokens::get(ui.ctx()).input_border;
    let track = move |_| rail;
    let axes = |h, v| [(h, tl!("Horizontal:")), (v, tl!("Vertical:"))];
    widgets::subheader(ui, tl!("Scale"));
    for (key, label) in axes("scaleH", "scaleV") {
        widgets::label_row(ui, label, LABEL_W, |ui| {
            let shown = widgets::SliderValue::field(d.f64(key, 100.0).clamp(-MAX_SCALE, MAX_SCALE), 0.0..=200.0, "%", 2);
            form::slider_rail_valued(ui, d, key, label, 0.0..=200.0, Some(shown), &track);
            if let Some(n) = widgets::range_field(ui, ("te", key), d.f64(key, 100.0), -MAX_SCALE..=MAX_SCALE, "%", 2, FIELD_W) {
                d.fields.insert(key.into(), json!(n));
            }
        });
    }
    widgets::subheader(ui, tl!("Move"));
    for (key, label) in axes("moveH", "moveV") {
        widgets::label_row(ui, label, LABEL_W, |ui| {
            let shown = form::length_value(d, key, units, -1000.0..=1000.0);
            form::slider_rail_valued(ui, d, key, label, -1000.0..=1000.0, shown, &track);
            form::length(ui, d, key, units, FIELD_W);
        });
    }
    widgets::subheader(ui, tl!("Rotate"));
    widgets::label_row(ui, tl!("Angle:"), LABEL_W, |ui| {
        let angle = d.f64("rotate", 0.0);
        let dialed = widgets::angle_dial(ui, "te-dial", angle, 28.0);
        if let Some(a) = widgets::plain_field(ui, ("te", "rotate"), angle, "°", 2, FIELD_W).or(dialed) {
            d.fields.insert("rotate".into(), json!(a));
        }
    });
}

/// The Reflect X, Reflect Y, reference point and Random options of Transform Each and the
/// Transform effect.
pub(super) fn reflect_options(ui: &mut egui::Ui, d: &mut Dialog) {
    form::check(ui, d, "reflectX", tl!("Reflect X"));
    form::check(ui, d, "reflectY", tl!("Reflect Y"));
    ui.horizontal(|ui| {
        let cur = d.f64("reference", 4.0).clamp(0.0, 8.0) as usize;
        if let Some(i) = widgets::reference_point(ui, cur) {
            d.fields.insert("reference".into(), json!(i));
        }
        widgets::dim_label(ui, tl!("Reference Point"));
    });
    form::check(ui, d, "random", tl!("Random"));
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let units = app.session.general_unit();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| sections(ui, d, units));
        ui.add_space(16.0);
        ui.vertical(|ui| {
            widgets::subheader(ui, tl!("Options"));
            ui.add_space(4.0);
            object_options(app, ui, d);
            scale_options(app, ui, d);
            reflect_options(ui, d);
        });
    });
    let mut p = params(d);
    add_object_options(app, d, &mut p);
    add_scale_options(app, d, &mut p);
    form::preview(app, ui, d, "Transform Each", CMD, p);
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = params(d);
    add_object_options(app, d, &mut p);
    let p = with_scale_options(app, d, p)?;
    let mut kept = serde_json::Map::new();
    for k in REMEMBERED {
        let v = if k == "preview" { json!(d.bool("preview")) } else { p[k].clone() };
        kept.insert(k.into(), v);
    }
    let r = form::commit_preview(app, CMD, p)?;
    app.ui.transform_each = Value::Object(kept);
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::NodeId;
    use vectorcraft_engine::Session;

    #[test]
    fn ok_transforms_each_and_keeps_the_scale_options() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("select.all", json!({})).unwrap();
        app.run("prefs.set", json!({"key": "scaleStrokes", "value": true})).unwrap();
        app.run("ui.menuDialog", json!({ "command": CMD })).unwrap();
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
        // One drawn frame: the checkboxes start from the preferences.
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!(d.fields["strokes"], json!(true));
        d.fields.insert("scaleH".into(), json!(200));
        d.fields.insert("scaleV".into(), json!(200));
        d.fields.insert("strokes".into(), json!(false));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert!(!app.session.prefs.scale_strokes, "OK keeps the option as the preference");
        let n = app.session.doc().unwrap().doc.node(NodeId(id)).unwrap().clone();
        assert!((n.geometric_bounds().unwrap().width() - 100.0).abs() < 1e-6);
        assert_eq!(n.appearance.stroke_width(), 1.0, "strokes kept their weight");
        let (cmd, p) = app.session.journal.last().unwrap();
        assert_eq!((cmd.as_str(), &p["strokes"], &p["objects"]), (CMD, &json!(false), &json!(true)), "#1448: Transform Objects goes along");
    }

    /// An app with a selected 50 × 50 square at (20, 20).
    fn square_app() -> (VectorcraftApp, NodeId) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        (app, NodeId(id))
    }

    fn open(app: &mut VectorcraftApp) -> &mut Dialog {
        crate::menus::invoke(app, CMD, json!({}));
        let d = app.ui.dialog.as_mut().expect("Transform Each is open");
        assert_eq!(d.kind, KIND);
        d
    }

    #[test]
    fn a_fresh_app_opens_on_the_defaults() {
        let (mut app, _) = square_app();
        let d = open(&mut app);
        assert_eq!(Value::Object(d.fields.clone()), fields());
    }

    /// #1732: the dialog opens on the values it was last confirmed with (as Move and Offset Path
    /// do), but never on Copy.
    #[test]
    fn it_reopens_on_the_values_confirmed_last() {
        let (mut app, _) = square_app();
        let d = open(&mut app);
        for (k, v) in [
            ("scaleH", json!(150)),
            ("moveH", json!(20)),
            ("rotate", json!(15)),
            ("reflectX", json!(true)),
            ("random", json!(true)),
            ("reference", json!(0)),
            ("objects", json!(false)),
            ("patterns", json!(true)),
            ("copy", json!(true)),
        ] {
            d.fields.insert(k.into(), v);
        }
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let d = open(&mut app);
        for (k, v) in [
            ("scaleH", json!(150.0)),
            ("scaleV", json!(100.0)),
            ("moveH", json!(20.0)),
            ("rotate", json!(15.0)),
            ("reflectX", json!(true)),
            ("reflectY", json!(false)),
            ("random", json!(true)),
            ("reference", json!(0)),
            ("objects", json!(false)),
            ("preview", json!(false)),
            ("copy", json!(false)),
        ] {
            assert_eq!(d.fields.get(k), Some(&v), "{k}");
        }
        // Cancel keeps nothing new.
        d.fields.insert("scaleH".into(), json!(300));
        super::super::cancel(&mut app);
        assert_eq!(open(&mut app).fields["scaleH"], json!(150.0));
        // The remembered values go with the UI state; one saved before they existed loads, and
        // opens on the defaults.
        app.ui.dialog = None;
        app.ui = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert_eq!(open(&mut app).fields["scaleH"], json!(150.0));
        app.ui = serde_json::from_value(json!({"move_options": {"copy": true}})).unwrap();
        assert_eq!(Value::Object(open(&mut app).fields.clone()), fields());
    }

    /// #1732: the Copy button transforms a copy of each object, in one undo step, and Transform
    /// Again makes a further copy; there is no Copy checkbox.
    #[test]
    fn the_copy_button_transforms_a_copy_of_each_object() {
        use egui_kittest::kittest::Queryable;
        let (mut app, id) = square_app();
        let d = open(&mut app);
        d.fields.insert("moveH".into(), json!(100));
        let undo = app.session.doc().unwrap().history.undo.len();
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::theme::install_fonts(ui.ctx());
                    return;
                }
                super::super::show(app, ui.ctx());
            },
            app,
        );
        h.run_steps(4);
        assert!(h.query_by_role_and_label(egui::accesskit::Role::CheckBox, "Copy").is_none(), "no Copy checkbox");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Copy").click();
        h.run_steps(3);
        let app = h.state_mut();
        assert!(app.ui.dialog.is_none(), "Copy closes the dialog");
        let xs = |app: &VectorcraftApp| {
            let st = app.session.doc().unwrap();
            let mut xs: Vec<f64> = st.doc.layers[0].children().unwrap().iter().map(|n| n.geometric_bounds().unwrap().x0.round()).collect();
            xs.sort_by(f64::total_cmp);
            xs
        };
        assert_eq!(xs(app), vec![20.0, 120.0], "the square stays, its copy moved 100 pt");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
        assert!(app.session.doc().unwrap().doc.node(id).is_some());
        assert_eq!(open(app).fields["copy"], json!(false), "Copy isn't remembered");
        app.ui.dialog = None;
        app.run("object.transformAgain", json!({})).unwrap();
        assert_eq!(xs(app), vec![20.0, 120.0, 220.0], "Transform Again copies once more");
        h.ctx.forget_all_images();
    }
}
