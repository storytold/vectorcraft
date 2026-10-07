//! Transform dialogs: Move, Rotate, Scale, Reflect and Shear (with Copy; Scale also has Relative
//! (percentages) or Absolute (a size), Uniform, Scale Corners and Scale Strokes & Effects).

use serde_json::{Value, json};

use super::{DialogSpec, form, run_and_close};
use crate::VectorcraftApp;
use crate::state::Dialog;

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |d| title(&d.kind).into(), body, confirm, ..DialogSpec::FORM };

fn title(kind: &str) -> &'static str {
    match kind {
        "move" => tl!("Move"),
        "rotate" => tl!("Rotate"),
        "scale" => tl!("Scale"),
        "reflect" => tl!("Reflect"),
        _ => tl!("Shear"),
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    if d.kind == "scale" {
        // Relative scales by percentages, Absolute to a size (fields absWidth / absHeight, from the
        // selection's size when first chosen).
        let abs = d.bool("absolute");
        ui.horizontal(|ui| {
            if crate::widgets::radio(ui, tl!("Relative"), !abs, true) {
                d.fields.insert("absolute".into(), json!(false));
            }
            if crate::widgets::radio(ui, tl!("Absolute"), abs, true) {
                d.fields.insert("absolute".into(), json!(true));
                if !d.fields.contains_key("absWidth")
                    && let Some(st) = app.session.active()
                    && let Some(b) = st.doc.bounds_of(&st.selection.objects, true)
                {
                    d.fields.insert("absWidth".into(), json!(b.width()));
                    d.fields.insert("absHeight".into(), json!(b.height()));
                }
            }
        });
        ui.add_space(4.0);
    }
    if d.kind == "scale" && d.bool("absolute") {
        egui::Grid::new("dlg-abs").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
            form::length_field(ui, d, "absWidth", "Width", unit);
            form::length_field(ui, d, "absHeight", "Height", unit);
        });
    } else {
        form::grid(ui, d, unit);
    }
    ui.add_space(6.0);
    if d.kind == "scale" {
        form::check(ui, d, "uniform", tl!("Uniform"));
        scale_options(app, ui, d);
    }
    form::check(ui, d, "copy", tl!("Copy (make a transformed copy)"));
    false
}

/// The scale options: (dialog field, preference, label).
const SCALE_OPTIONS: [(&str, &str, &str); 2] = [("corners", "scaleCorners", "Scale Corners"), ("strokes", "scaleStrokes", "Scale Strokes & Effects")];

/// A scale option's value: the dialog's field, else the preference.
fn scale_option(app: &VectorcraftApp, d: &Dialog, field: &str) -> bool {
    let prefs = &app.session.prefs;
    d.fields.get(field).and_then(Value::as_bool).unwrap_or(if field == "corners" { prefs.scale_corners } else { prefs.scale_strokes })
}

/// The Scale Corners and Scale Strokes & Effects checkboxes (fields `corners` and `strokes`),
/// starting from the preferences.
pub(super) fn scale_options(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    for (field, _, label) in SCALE_OPTIONS {
        let on = scale_option(app, d, field);
        d.fields.insert(field.into(), Value::Bool(on));
        form::check(ui, d, field, tl!(label));
    }
}

/// Add the dialog's scale options to command `params`.
pub(super) fn add_scale_options(app: &VectorcraftApp, d: &Dialog, params: &mut Value) {
    for (field, _, _) in SCALE_OPTIONS {
        params[field] = json!(scale_option(app, d, field));
    }
}

/// `params` with the dialog's scale options, which also become the preferences (they are shared
/// with the Transform panel and Preferences, as in the reference app).
pub(super) fn with_scale_options(app: &mut VectorcraftApp, d: &Dialog, mut params: Value) -> Result<Value, String> {
    add_scale_options(app, d, &mut params);
    let prefs: serde_json::Map<String, Value> = SCALE_OPTIONS.iter().map(|(field, pref, _)| (pref.to_string(), params[*field].clone())).collect();
    app.run("prefs.set", json!({ "values": prefs }))?;
    Ok(params)
}

/// Pass a tool-chosen reference point (origin) through to transform commands.
fn origin_params(d: &Dialog, mut p: Value) -> Value {
    if let Some(o) = d.fields.get("origin") {
        p["origin"] = o.clone();
    }
    p
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let copy = d.bool("copy");
    let (id, params) = match d.kind.as_str() {
        "move" => return run_and_close(app, "object.move", json!({"dx": d.f64("dx", 0.0), "dy": d.f64("dy", 0.0), "copy": copy})),
        "rotate" => ("object.rotate", json!({"angle": d.f64("angle", 0.0), "copy": copy})),
        "scale" if d.bool("absolute") => {
            let w = d.f64("absWidth", 0.0);
            // Uniform keeps the proportions: the width decides.
            let size = if d.bool("uniform") { json!({"width": w}) } else { json!({"width": w, "height": d.f64("absHeight", 0.0)}) };
            let mut p = size;
            p["copy"] = json!(copy);
            ("object.scale", with_scale_options(app, d, p)?)
        }
        "scale" => {
            let sx = d.f64("sx", 100.0);
            let sy = if d.bool("uniform") { sx } else { d.f64("sy", 100.0) };
            ("object.scale", with_scale_options(app, d, json!({"sx": sx, "sy": sy, "copy": copy}))?)
        }
        "reflect" => ("object.reflect", json!({"axis": d.fields.get("axis").cloned().unwrap_or(json!("vertical")), "copy": copy})),
        _ => ("object.shear", json!({"angle": d.f64("angle", 0.0), "axis": d.str("axis"), "copy": copy})),
    };
    run_and_close(app, id, origin_params(d, params))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use crate::VectorcraftApp;

    #[test]
    fn scale_options_start_from_and_update_the_preferences() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50, "radius": 5})).unwrap();
        app.run("select.all", json!({})).unwrap();
        app.run("prefs.set", json!({"values": {"scaleStrokes": true, "scaleCorners": false}})).unwrap();
        crate::menus::invoke(&mut app, "object.scale", json!({}));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((&d.fields["strokes"], &d.fields["corners"]), (&json!(true), &json!(false)));
        d.fields.insert("sx".into(), json!(200));
        d.fields.insert("strokes".into(), json!(false));
        d.fields.insert("corners".into(), json!(true));
        crate::dialogs::confirm(&mut app).unwrap();
        let prefs = &app.session.prefs;
        assert_eq!((prefs.scale_strokes, prefs.scale_corners), (false, true));
        let n = &app.session.doc().unwrap().doc.layers[0].children().unwrap()[0];
        assert_eq!(n.appearance.stroke_width(), 1.0);
        let (cmd, p) = app.session.journal.last().unwrap();
        assert_eq!((cmd.as_str(), &p["strokes"], &p["corners"]), ("object.scale", &json!(false), &json!(true)));
    }
}
