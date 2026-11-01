//! Transform dialogs: Move, Rotate, Scale, Reflect and Shear (with Transform Objects, Transform
//! Patterns, Copy and Preview; Scale also has Uniform, Scale Corners and Scale Strokes & Effects;
//! Reflect's axis is Horizontal, Vertical or an angle). With Preview on (`preview`), the transform
//! shows on the canvas as the values change.
//!
//! Move opens on the offset moved last, by a drag or by itself, and with the checkboxes (Preview
//! too) it was last confirmed with, so Enter, Enter repeats a move or a copy at the same spacing.
//! The other dialogs open with Preview off.

use serde_json::{Value, json};

use super::{DialogSpec, form, run_and_close};
use crate::VectorcraftApp;
use crate::state::Dialog;

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |d| title(&d.kind).into(), body, confirm, preview: true, inputs, ..DialogSpec::FORM };

/// The fields a transform dialog reads that it may not hold yet: Copy and the object options,
/// Move's Distance and Angle, Scale's scale options, and the reference point of the others.
fn inputs(kind: &str) -> &'static [&'static str] {
    match kind {
        "move" => &["copy", "objects", "patterns", "distance", "angle"],
        "scale" => &["copy", "objects", "patterns", "corners", "strokes", "origin"],
        _ => &["copy", "objects", "patterns", "origin"],
    }
}

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
    if d.kind == "move" {
        link_distance_and_angle(d);
    }
    form::grid(ui, d, app.session.general_unit());
    if d.kind == "move" {
        link_distance_and_angle(d);
    }
    ui.add_space(6.0);
    if d.kind == "scale" {
        form::check(ui, d, "uniform", tl!("Uniform"));
        scale_options(app, ui, d);
    }
    object_options(app, ui, d);
    form::check(ui, d, "copy", tl!("Copy (make a transformed copy)"));
    let (cmd, mut params) = command(app, d);
    if d.kind == "scale" {
        add_scale_options(app, d, &mut params);
    }
    form::preview(app, ui, d, title(&d.kind), cmd, params);
    false
}

/// Move's Distance and Angle (fields `distance`, `angle`) and Horizontal and Vertical (`dx`, `dy`)
/// are the same offset: the pair edited since the last call (`__polar` holds the values then)
/// sets the other. The angle is counter-clockwise on the page and y points down, so 90° moves up.
fn link_distance_and_angle(d: &mut Dialog) {
    let [dx, dy, distance, angle] = ["dx", "dy", "distance", "angle"].map(|k| d.f64(k, 0.0));
    let last: Option<Vec<f64>> = d.fields.get("__polar").and_then(|v| serde_json::from_value(v.clone()).ok());
    let polar_edited = last.as_deref().is_some_and(|l| l.get(2..) != Some(&[distance, angle][..]));
    let [dx, dy, distance, angle] = if polar_edited {
        let a = angle.to_radians();
        // An angle past 180° reads as its negative one (270° is -90°), as the documented dialog has it.
        let turn = angle.rem_euclid(360.0);
        let angle = if turn > 180.0 { turn - 360.0 } else { turn };
        [distance * a.cos(), -distance * a.sin(), distance, angle].map(clean)
    } else {
        [dx, dy, dx.hypot(dy), (-dy).atan2(dx).to_degrees()].map(clean)
    };
    for (k, v) in [("dx", dx), ("dy", dy), ("distance", distance), ("angle", angle)] {
        d.fields.insert(k.into(), json!(v));
    }
    d.fields.insert("__polar".into(), json!([dx, dy, distance, angle]));
}

/// `v` without the float noise of the trigonometry (cos 90° is not quite 0). Adding 0 turns -0
/// into 0.
fn clean(v: f64) -> f64 {
    (v * 1e6).round() / 1e6 + 0.0
}

/// The Move dialog's fields as it opens: Horizontal and Vertical are the offset moved last, Distance
/// and Angle are the same offset, and Copy, Transform Objects and Preview are as it was last
/// confirmed with (Transform Patterns is General › Transform Pattern Tiles, which its OK sets).
pub(crate) fn move_fields(app: &VectorcraftApp) -> Value {
    let (dx, dy) = last_move(app).unwrap_or_default();
    let [dx, dy, distance, angle] = [dx, dy, dx.hypot(dy), (-dy).atan2(dx).to_degrees()].map(clean);
    let mut fields = json!({
        "dx": format!("{dx} pt"),
        "dy": format!("{dy} pt"),
        "distance": distance,
        "angle": angle,
        "__polar": [dx, dy, distance, angle],
    });
    let kept = &app.ui.move_options;
    let kept_bool = |k: &str| kept.get(k).and_then(Value::as_bool);
    for k in ["copy", "objects", "preview"] {
        if let Some(on) = kept_bool(k) {
            fields[k] = json!(on);
        }
    }
    fields
}

/// The offset of the last move the session's journal keeps: a drag that moved the selection (a
/// Transform that only translates) or Move.
fn last_move(app: &VectorcraftApp) -> Option<(f64, f64)> {
    let length = |v: Option<&Value>| match v {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => vectorcraft_doc::Unit::Points.parse(s),
        _ => Some(0.0),
    };
    let finite = |(dx, dy): (f64, f64)| (dx.is_finite() && dy.is_finite()).then_some((dx, dy));
    app.session.journal.iter().rev().find_map(|(id, p)| match id.as_str() {
        "object.move" => finite((length(p.get("dx"))?, length(p.get("dy"))?)),
        "object.transform" => {
            let [a, b, c, d, e, f] = vectorcraft_engine::cmd::matrix_param(p, "matrix")?.as_coeffs();
            let translates = (a, b, c, d) == (1.0, 0.0, 0.0, 1.0) && (e, f) != (0.0, 0.0);
            translates.then_some((e, f)).and_then(finite)
        }
        _ => None,
    })
}

/// Transform Patterns: the dialog's field, else General › Transform Pattern Tiles.
fn transform_patterns(app: &VectorcraftApp, d: &Dialog) -> bool {
    d.fields.get("patterns").and_then(Value::as_bool).unwrap_or(app.session.prefs.transform_pattern_tiles)
}

/// Transform Objects: the dialog's field, on until it is switched off (the pattern tiles alone
/// transform then, with Transform Patterns).
fn transform_objects(d: &Dialog) -> bool {
    d.fields.get("objects").and_then(Value::as_bool).unwrap_or(true)
}

/// The Transform Objects and Transform Patterns checkboxes (fields `objects` and `patterns`).
pub(super) fn object_options(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    d.fields.insert("objects".into(), Value::Bool(transform_objects(d)));
    form::check(ui, d, "objects", tl!("Transform Objects"));
    d.fields.insert("patterns".into(), Value::Bool(transform_patterns(app, d)));
    form::check(ui, d, "patterns", tl!("Transform Patterns"));
}

/// Add the dialog's Transform Objects and Transform Patterns options to command `params`.
pub(super) fn add_object_options(app: &VectorcraftApp, d: &Dialog, params: &mut Value) {
    params["objects"] = json!(transform_objects(d));
    params["patterns"] = json!(transform_patterns(app, d));
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

/// The dialog's command and its params (Scale's without its scale options).
fn command(app: &VectorcraftApp, d: &Dialog) -> (&'static str, Value) {
    let (id, mut params) = match d.kind.as_str() {
        "move" => ("object.move", json!({"dx": d.f64("dx", 0.0), "dy": d.f64("dy", 0.0)})),
        "rotate" => ("object.rotate", json!({"angle": d.f64("angle", 0.0)})),
        "scale" => {
            let sx = d.f64("sx", 100.0);
            let sy = if d.bool("uniform") { sx } else { d.f64("sy", 100.0) };
            ("object.scale", json!({"sx": sx, "sy": sy}))
        }
        // The axis by name, or the angle it is at.
        "reflect" if d.str("axis") == "angle" => ("object.reflect", json!({"axis": d.f64("angle", 90.0)})),
        "reflect" => ("object.reflect", json!({"axis": d.fields.get("axis").cloned().unwrap_or(json!("vertical"))})),
        _ => ("object.shear", json!({"angle": d.f64("angle", 0.0), "axis": d.str("axis")})),
    };
    params["copy"] = json!(d.bool("copy"));
    add_object_options(app, d, &mut params);
    // Move has no reference point.
    if id == "object.move" { (id, params) } else { (id, origin_params(d, params)) }
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    // The preview is rolled back and the command run as without one, so its undo step, its journal
    // entry and Transform Again are the same either way.
    if app.session.in_interaction() {
        app.session.cancel_interaction().map_err(|e| e.to_string())?;
    }
    // A Distance and Angle set since the last frame set the offset Move runs with.
    let mut d = d.clone();
    if d.kind == "move" {
        link_distance_and_angle(&mut d);
    }
    let d = &d;
    let (id, mut params) = command(app, d);
    if d.kind == "scale" {
        params = with_scale_options(app, d, params)?;
    }
    let r = run_and_close(app, id, params)?;
    if d.kind == "move" {
        app.ui.move_options = json!({"copy": d.bool("copy"), "objects": transform_objects(d), "preview": d.bool("preview")});
        // Move's Transform Patterns stays as confirmed: it is the preference it starts from.
        let patterns = transform_patterns(app, d);
        if patterns != app.session.prefs.transform_pattern_tiles {
            app.run("prefs.set", json!({"key": "transformPatternTiles", "value": patterns}))?;
        }
    }
    Ok(r)
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

    /// #1412: Preview shows the transform on the canvas as the values change, ↑/↓ in the angle
    /// field step it by 1° (Shift: 10°), Cancel rolls the preview back and OK keeps one undo step
    /// that Transform Again repeats.
    #[test]
    fn preview_follows_the_stepped_angle() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 60, "height": 20})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let frame = |app: &mut VectorcraftApp, key: Option<(egui::Key, egui::Modifiers)>| {
            let events = key.map(|(key, modifiers)| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
            let mut out =
                ctx.run_ui(egui::RawInput { events: events.into_iter().collect(), ..Default::default() }, |ui| crate::dialogs::show(app, ui.ctx()));
            out.textures_delta.clear();
        };
        let size = |app: &VectorcraftApp| {
            let b = app.session.doc().unwrap().doc.layers[0].children().unwrap()[0].geometric_bounds().unwrap();
            (b.width().round(), b.height().round())
        };
        let undo_steps = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
        let steps = undo_steps(&app);
        crate::menus::invoke(&mut app, "object.rotate", json!({}));
        frame(&mut app, None);
        assert!(!app.session.in_interaction(), "Preview is off when the dialog opens");
        // ↑ nine times with Shift: 90°, in the dialog only.
        for _ in 0..9 {
            frame(&mut app, Some((egui::Key::ArrowUp, egui::Modifiers::SHIFT)));
        }
        frame(&mut app, Some((egui::Key::ArrowUp, egui::Modifiers::NONE)));
        frame(&mut app, Some((egui::Key::ArrowDown, egui::Modifiers::NONE)));
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["angle"], json!(90.0));
        assert_eq!(size(&app), (60.0, 20.0));
        // Preview on: the canvas shows it, and follows the next steps.
        app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(true));
        frame(&mut app, None);
        assert!(app.session.in_interaction());
        assert_eq!(size(&app), (20.0, 60.0));
        for _ in 0..9 {
            frame(&mut app, Some((egui::Key::ArrowUp, egui::Modifiers::SHIFT)));
        }
        assert_eq!(size(&app), (60.0, 20.0), "180°");
        // Cancel rolls it back.
        for _ in 0..9 {
            frame(&mut app, Some((egui::Key::ArrowDown, egui::Modifiers::SHIFT)));
        }
        crate::dialogs::cancel(&mut app);
        assert!(!app.session.in_interaction());
        assert_eq!((size(&app), undo_steps(&app)), ((60.0, 20.0), steps));
        // OK keeps the previewed transform as one undo step, and Transform Again repeats it.
        crate::menus::invoke(&mut app, "object.rotate", json!({}));
        app.ui.dialog.as_mut().unwrap().fields.extend([("preview".to_string(), json!(true)), ("angle".to_string(), json!(90))]);
        frame(&mut app, None);
        assert_eq!(size(&app), (20.0, 60.0));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(!app.session.in_interaction() && app.ui.dialog.is_none());
        assert_eq!((size(&app), undo_steps(&app)), ((20.0, 60.0), steps + 1));
        assert_eq!(app.session.journal.last().unwrap().0, "object.rotate");
        app.run("object.transformAgain", json!({})).unwrap();
        assert_eq!(size(&app), (60.0, 20.0));
    }

    /// #1460: Reflect's axis is Horizontal, Vertical or an angle, and Transform Objects goes with
    /// the command (on until switched off).
    #[test]
    fn reflect_takes_an_angle_and_transform_objects() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 60, "height": 20})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        for (axis, objects, size) in [("angle", true, (20.0, 60.0)), ("vertical", false, (20.0, 60.0))] {
            crate::menus::invoke(&mut app, "object.reflect", json!({}));
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(&mut app, ui.ctx()));
            out.textures_delta.clear();
            let d = app.ui.dialog.as_mut().unwrap();
            assert_eq!((&d.fields["objects"], &d.fields["angle"]), (&json!(true), &json!(90)));
            d.fields.extend([("axis".to_string(), json!(axis)), ("angle".to_string(), json!(45)), ("objects".to_string(), json!(objects))]);
            crate::dialogs::confirm(&mut app).unwrap();
            let (cmd, p) = app.session.journal.last().unwrap();
            let want = if axis == "angle" { json!(45.0) } else { json!(axis) };
            assert_eq!((cmd.as_str(), &p["axis"], &p["objects"]), ("object.reflect", &want, &json!(objects)));
            // Across the 45° axis the rectangle's sides swap; without Transform Objects it stays.
            let b = app.session.doc().unwrap().doc.layers[0].children().unwrap()[0].geometric_bounds().unwrap();
            assert_eq!((b.width().round(), b.height().round()), size);
        }
    }

    /// #1915: Move's Distance and Angle follow Horizontal and Vertical, and they follow them back.
    /// The angle is counter-clockwise on the page and Vertical is down = +, so 90° moves up.
    #[test]
    fn move_takes_a_distance_and_an_angle() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 60, "height": 20})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let frame = |app: &mut VectorcraftApp| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(app, ui.ctx()));
            out.textures_delta.clear();
        };
        let set = |app: &mut VectorcraftApp, values: [(&str, f64); 2]| {
            app.ui.dialog.as_mut().unwrap().fields.extend(values.map(|(k, v)| (k.to_string(), json!(v))));
        };
        let fields = |app: &VectorcraftApp| {
            let d = app.ui.dialog.as_ref().unwrap();
            ["dx", "dy", "distance", "angle"].map(|k| d.f64(k, f64::NAN))
        };
        crate::menus::invoke(&mut app, "object.move", json!({}));
        frame(&mut app);
        assert_eq!(fields(&app), [0.0, 0.0, 0.0, 0.0]);
        set(&mut app, [("distance", 10.0), ("angle", 90.0)]);
        frame(&mut app);
        assert_eq!(fields(&app), [0.0, -10.0, 10.0, 90.0]);
        set(&mut app, [("dx", 3.0), ("dy", 4.0)]);
        frame(&mut app);
        assert_eq!(fields(&app), [3.0, 4.0, 5.0, -53.130102]);
        set(&mut app, [("distance", 10.0), ("angle", 270.0)]);
        frame(&mut app);
        assert_eq!(fields(&app), [0.0, 10.0, 10.0, -90.0], "270° is -90°");
        set(&mut app, [("distance", 10.0), ("angle", 90.0)]);
        frame(&mut app);
        crate::dialogs::confirm(&mut app).unwrap();
        let (cmd, p) = app.session.journal.last().unwrap();
        assert_eq!((cmd.as_str(), &p["dx"], &p["dy"]), ("object.move", &json!(0.0), &json!(-10.0)));
        let b = app.session.doc().unwrap().doc.layers[0].children().unwrap()[0].geometric_bounds().unwrap();
        assert_eq!((b.x0, b.y0), (100.0, 90.0));
    }

    /// A Distance and Angle set before the dialog's first frame, or just before OK, move by that
    /// offset (as `ui.dialog.set` then `ui.dialog.confirm` from a control client).
    #[test]
    fn move_takes_a_distance_and_an_angle_set_before_a_frame() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 60, "height": 20})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let corner = |app: &VectorcraftApp| {
            let b = app.session.doc().unwrap().doc.layers[0].children().unwrap()[0].geometric_bounds().unwrap();
            (b.x0, b.y0)
        };
        // Up 10 pt with no frame, then right 20 pt (Move opens on the last offset) with one.
        for (with_frame, distance, angle, want, corner_after) in
            [(false, 10.0, 90.0, (0.0, -10.0), (100.0, 90.0)), (true, 20.0, 0.0, (20.0, 0.0), (120.0, 90.0))]
        {
            crate::menus::invoke(&mut app, "object.move", json!({}));
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.extend([("distance".to_string(), json!(distance)), ("angle".to_string(), json!(angle))]);
            if with_frame {
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(&mut app, ui.ctx()));
                out.textures_delta.clear();
                let d = app.ui.dialog.as_ref().unwrap();
                assert_eq!(["distance", "angle"].map(|k| d.f64(k, f64::NAN)), [distance, angle], "the first frame keeps them");
            }
            crate::dialogs::confirm(&mut app).unwrap();
            let (cmd, p) = app.session.journal.last().unwrap();
            assert_eq!((cmd.as_str(), &p["dx"], &p["dy"]), ("object.move", &json!(want.0), &json!(want.1)), "with a frame: {with_frame}");
            assert_eq!(corner(&app), corner_after);
        }
    }

    /// Move opens on the offset of the last drag or Move, and with the checkboxes it was last
    /// confirmed with: Enter, Enter makes copies at the same spacing.
    #[test]
    fn move_opens_on_the_last_move_and_checkboxes() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let open = |app: &mut VectorcraftApp| {
            crate::toolbar::open_options(app, "selection").unwrap();
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(app, ui.ctx()));
            out.textures_delta.clear();
            let d = app.ui.dialog.as_ref().unwrap();
            ([d.f64("dx", f64::NAN), d.f64("dy", f64::NAN)], ["copy", "objects", "preview"].map(|k| d.bool(k)))
        };
        let lefts = |app: &VectorcraftApp| {
            let kids = app.session.doc().unwrap().doc.layers[0].children().unwrap();
            kids.iter().map(|n| n.geometric_bounds().unwrap().x0).collect::<Vec<_>>()
        };
        assert_eq!(open(&mut app), ([0.0, 0.0], [false, true, false]), "nothing moved yet");
        crate::dialogs::cancel(&mut app);
        // A drag: its offset, and a rotation since doesn't count.
        app.session.begin_interaction("Move").unwrap();
        app.session.preview("object.transform", &json!({"matrix": [1, 0, 0, 1, 20, 5]})).unwrap();
        app.session.commit_interaction().unwrap();
        app.run("object.rotate", json!({"angle": 90})).unwrap();
        app.run("object.rotate", json!({"angle": -90})).unwrap();
        assert_eq!(open(&mut app).0, [20.0, 5.0]);
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.extend([("dx".to_string(), json!(30)), ("dy".to_string(), json!(0)), ("copy".to_string(), json!(true))]);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(open(&mut app), ([30.0, 0.0], [true, true, false]));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(lefts(&app), [20.0, 50.0, 80.0], "a copy at 30 pt each time");
        // A drag moves the next one by its offset instead; the checkboxes stay.
        app.session.begin_interaction("Move").unwrap();
        app.session.preview("object.transform", &json!({"matrix": [1, 0, 0, 1, 0, 12]})).unwrap();
        app.session.commit_interaction().unwrap();
        assert_eq!(open(&mut app), ([0.0, 12.0], [true, true, false]));
        crate::dialogs::cancel(&mut app);
    }

    /// The Move dialog's Transform Patterns is as it was last confirmed with: its OK sets General ›
    /// Transform Pattern Tiles, which it starts from.
    #[test]
    fn move_keeps_transform_patterns_in_the_preference() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let open = |app: &mut VectorcraftApp| {
            crate::menus::invoke(app, "object.move", json!({}));
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(app, ui.ctx()));
            out.textures_delta.clear();
            app.ui.dialog.as_ref().unwrap().bool("patterns")
        };
        app.run("prefs.set", json!({"key": "transformPatternTiles", "value": false})).unwrap();
        assert!(!open(&mut app));
        app.ui.dialog.as_mut().unwrap().fields.insert("patterns".into(), json!(true));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(open(&mut app) && app.session.prefs.transform_pattern_tiles, "as confirmed");
        crate::dialogs::cancel(&mut app);
        app.run("prefs.set", json!({"key": "transformPatternTiles", "value": false})).unwrap();
        assert!(!open(&mut app), "the preference");
        crate::dialogs::cancel(&mut app);
    }

    /// Transform Patterns starts from General › Transform Pattern Tiles (#394) and goes with the
    /// command.
    #[test]
    fn transform_patterns_starts_from_the_preference() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        for on in [false, true] {
            app.run("prefs.set", json!({"key": "transformPatternTiles", "value": on})).unwrap();
            crate::menus::invoke(&mut app, "object.move", json!({}));
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::dialogs::show(&mut app, ui.ctx()));
            out.textures_delta.clear();
            let d = app.ui.dialog.as_mut().unwrap();
            assert_eq!(d.fields["patterns"], json!(on));
            d.fields.insert("dx".into(), json!(10));
            crate::dialogs::confirm(&mut app).unwrap();
            let (cmd, p) = app.session.journal.last().unwrap();
            assert_eq!((cmd.as_str(), &p["patterns"]), ("object.move", &json!(on)));
        }
    }
}
