//! Object → Expand: what to expand (Object: live shapes, type and effects; Fill: gradient and pattern fills;
//! Stroke: strokes into filled outlines) and what gradient fills become (a gradient mesh, or a
//! number of objects). Options the selection has nothing for are disabled. OK runs
//! `object.expand` as one undo step.
//!
//! Fields: `object`, `fill`, `stroke`, `gradient` (`objects` | `mesh`), `steps` (1..1000), and
//! `__object`, `__fill`, `__stroke` (that option has something to expand: `object.expand.info`) and
//! `__gradient` (Fill has a gradient fill to expand).

use serde_json::{Value, json};
use vectorcraft_engine::cmd::expand::{DEFAULT_STEPS, MAX_STEPS};

use super::{DialogSpec, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Expand.
pub const KIND: &str = "expand";

const CMD: &str = "object.expand";

/// The options and their checkbox labels.
const OPTIONS: [(&str, &str); 3] = [("object", "Object"), ("fill", "Fill"), ("stroke", "Stroke")];

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Expand").into(), body, confirm, min_width: 260.0, ..DialogSpec::FORM };

/// Why Object › Expand… is dimmed: the selection has appearance attributes, which Expand Appearance
/// turns into plain art first (#3089).
pub(crate) fn appearance_first() -> &'static str {
    tl!("Expand Appearance first (Object › Expand Appearance)")
}

/// Open Expand for the selection, every option on (as `object.expand {}` does). A selection with
/// appearance attributes (effects, several fills or strokes…) refuses with [`appearance_first`]:
/// the menu item is dimmed for it.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let info = app.session.execute("object.expand.info", &json!({})).map_err(|e| e.to_string())?;
    if info["appearance"] == json!(true) {
        return Err(appearance_first().into());
    }
    let mut fields = json!({"object": true, "fill": true, "stroke": true, "gradient": "objects", "steps": DEFAULT_STEPS});
    for (key, _) in OPTIONS {
        fields[format!("__{key}")] = info[key].clone();
    }
    fields["__gradient"] = info["gradient"].clone();
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let set = |d: &mut Dialog, key: &str, v: Value| {
        d.fields.insert(key.into(), v);
    };
    widgets::subheader(ui, tl!("Expand"));
    for (key, label) in OPTIONS {
        let on = d.bool(key);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            if widgets::check(ui, label, on, d.bool(&format!("__{key}"))) {
                set(d, key, json!(!on));
            }
        });
    }
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Expand Gradient To"));
    let enabled = gradient_to_enabled(d);
    let objects = d.str("gradient") != "mesh";
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        if widgets::radio(ui, tl!("Gradient Mesh"), !objects, enabled) {
            set(d, "gradient", json!("mesh"));
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        if widgets::radio(ui, tl!("Specify:"), objects, enabled) {
            set(d, "gradient", json!("objects"));
        }
        ui.add_enabled_ui(enabled && objects, |ui| {
            if let Some(n) = widgets::plain_field(ui, "expand-steps", d.f64("steps", DEFAULT_STEPS as f64), "", 0, 52.0) {
                set(d, "steps", json!(n.round().clamp(1.0, MAX_STEPS as f64)));
            }
            widgets::dim_label(ui, tl!("Objects"));
        });
    });
    false
}

/// Does Expand Gradient To apply? Only gradient fills that Fill expands care.
fn gradient_to_enabled(d: &Dialog) -> bool {
    d.bool("__gradient") && d.bool("fill")
}

/// `object.expand` parameters from the fields.
fn params(d: &Dialog) -> Value {
    json!({
        "object": d.bool("object"),
        "fill": d.bool("fill"),
        "stroke": d.bool("stroke"),
        "gradient": d.str("gradient"),
        "steps": d.f64("steps", DEFAULT_STEPS as f64),
    })
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    run_and_close(app, CMD, params(d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::NodeKind;

    fn frame(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    #[test]
    fn opens_with_what_applies_and_ok_expands_the_gradient_to_a_mesh() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 60})).unwrap();
        app.run("paint.setFill", json!({"gradient": {"kind": "linear"}})).unwrap();
        app.run("paint.setStroke", json!({"none": true})).unwrap();
        // The menu item (and `object.expand` without params) opens the dialog.
        crate::menus::invoke(&mut app, "object.expand", json!({}));
        let d = app.ui.dialog.as_ref().expect("the Expand dialog");
        assert_eq!(d.kind, KIND);
        assert_eq!((d.bool("__object"), d.bool("__fill"), d.bool("__stroke")), (true, true, false), "no stroke to expand");
        assert_eq!(params(d), json!({"object": true, "fill": true, "stroke": true, "gradient": "objects", "steps": 255.0}));
        let text = frame(&mut app);
        for label in ["Expand", "Object", "Fill", "Stroke", "Expand Gradient To", "Gradient Mesh", "Specify:", "Objects", "OK", "Cancel"] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let undo = app.session.doc().unwrap().history.undo.len();
        app.ui.dialog.as_mut().unwrap().fields.insert("gradient".into(), json!("mesh"));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let st = app.session.doc().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1);
        let g = st.doc.node(st.selection.objects[0]).unwrap();
        let NodeKind::Group { children, clip: true } = &g.kind else { panic!("a clip group: {:?}", g.kind) };
        assert!(matches!(children[1].kind, NodeKind::Mesh(_)));
    }

    #[test]
    fn expand_gradient_to_is_disabled_for_a_pattern_fill_alone() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let dot = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
        app.run("object.pattern.make", json!({"ids": [dot], "name": "Dots", "width": 20, "height": 20, "edit": false})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 60})).unwrap();
        app.run("paint.setFill", json!({"swatch": "Dots"})).unwrap();
        app.run("ui.expandDialog", json!({})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert!(d.bool("__fill"), "Fill expands the pattern fill");
        assert!(!gradient_to_enabled(d), "no gradient fill for Expand Gradient To");
        frame(&mut app);
        // A gradient fill enables it.
        app.ui.dialog = None;
        app.run("paint.setFill", json!({"gradient": {"kind": "linear"}})).unwrap();
        app.run("ui.expandDialog", json!({})).unwrap();
        assert!(gradient_to_enabled(app.ui.dialog.as_ref().unwrap()));
    }

    #[test]
    fn the_command_palette_opens_it_and_ui_expand_dialog_too() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 60})).unwrap();
        app.run("ui.expandDialog", json!({})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.bool("__object"), d.bool("__fill"), d.bool("__stroke")), (true, false, true));
        frame(&mut app);
        // Explicit params still run the command directly.
        app.ui.dialog = None;
        crate::menus::invoke(&mut app, "object.expand", json!({"stroke": false}));
        assert!(app.ui.dialog.is_none());
        let st = app.session.doc().unwrap();
        assert!(matches!(st.doc.node(st.selection.objects[0]).unwrap().kind, NodeKind::Path { live: None, .. }));
    }

    /// A line with a stroke and a Zig Zag effect is selected (#3089).
    fn zig_zag_line(app: &mut VectorcraftApp) {
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("path.create", json!({"d": "M 20 100 L 180 100"})).unwrap();
        app.run("paint.setFill", json!({"none": true})).unwrap();
        app.run("paint.setStroke", json!({"color": "#000000"})).unwrap();
        app.run("effect.apply", json!({"effect": "distort.zigZag", "params": {"size": 10, "ridges": 6, "relative": false}})).unwrap();
    }

    #[test]
    fn expand_waits_for_expand_appearance_on_a_line_with_an_effect() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        zig_zag_line(&mut app);
        assert!(!crate::menus::enabled(&app, "ui.expandDialog"), "Expand… is dimmed for the Zig Zag effect");
        assert!(crate::menus::enabled(&app, "effect.expandAppearance"));
        // A shortcut or the command palette doesn't open it either; the status bar says why.
        let before = app.session.doc().unwrap().doc.clone();
        crate::menus::invoke(&mut app, "object.expand", json!({}));
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.ui.status, appearance_first());
        assert_eq!(app.run("ui.expandDialog", json!({})).unwrap_err(), appearance_first());
        assert_eq!(app.session.doc().unwrap().doc, before, "the line is left as it was");
        // Expand Appearance first: the line stays a stroked line, and Expand… is available for it.
        crate::menus::invoke(&mut app, "effect.expandAppearance", json!({}));
        let st = app.session.doc().unwrap();
        let line = st.doc.node(st.selection.objects[0]).unwrap();
        assert!(line.appearance.effects.is_empty() && !line.appearance.stroke_paint().is_none());
        assert!(crate::menus::enabled(&app, "ui.expandDialog"));
        crate::menus::invoke(&mut app, "object.expand", json!({}));
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
    }

    #[test]
    fn expand_is_dimmed_for_two_fills_but_not_for_a_plain_rectangle() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 60})).unwrap();
        assert!(crate::menus::enabled(&app, "ui.expandDialog"), "a basic appearance");
        app.run("appearance.addFill", json!({})).unwrap();
        assert!(!crate::menus::enabled(&app, "ui.expandDialog"), "two fills");
        // Nothing selected: dimmed as before.
        app.run("select.none", json!({})).unwrap();
        assert!(!crate::menus::enabled(&app, "ui.expandDialog"));
    }
}
