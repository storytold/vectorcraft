//! Circle Tool Options (double-click the Circle tool, or `tool.options`): Show Center, Show Radii
//! and Show Diameter switch the center cross, the radius lines and the diameter label the tool
//! shows while a circle is drawn; Color is the color of the cross and the lines; Anchors at
//! Standard Positions off puts the circle's anchors at the points it was drawn through; Third
//! Point by Dragging off places the second point with a click and the third with another. OK sets
//! them with `tool.setOption {tool: "circle", values}`; they last across tool switches and are
//! saved with the preferences.

use serde_json::{Map, Value, json};
use vectorcraft_tools::circle::{COLORS, ID as TOOL};

use super::{DialogSpec, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Circle Tool Options.
pub const KIND: &str = "circleOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: 300.0, ..DialogSpec::FORM };

/// The options the dialog edits, in the order it shows its check boxes, with their labels.
const SWITCHES: [(&str, &str); 5] = [
    ("center", "Show Center"),
    ("radii", "Show Radii"),
    ("diameter", "Show Diameter"),
    ("standardAnchors", "Anchors at Standard Positions"),
    ("dragThird", "Third Point by Dragging"),
];

/// The labels of [`COLORS`], in its order.
const COLOR_LABELS: [&str; 6] = ["Red", "Blue", "Magenta", "Green", "Black", "Gray"];

/// Width of the label column.
const LABEL_W: f32 = 60.0;

fn heading(_: &Dialog) -> String {
    let label = vectorcraft_tools::tool_info(TOOL).map_or("", |t| t.label);
    crate::i18n::fmt(tl!("{tool} Options"), &[("tool", tl!(label))])
}

/// Open the tool's options with their current values.
pub fn open(app: &mut VectorcraftApp) -> Value {
    let o = app.session.tool_options_of(TOOL);
    let keys = SWITCHES.iter().map(|(key, _)| *key).chain(["color"]);
    let fields: Map<String, Value> = keys.filter_map(|key| Some((key.to_string(), o.get(key)?.clone()))).collect();
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    json!({ "dialog": KIND })
}

fn check(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str) {
    let on = d.bool(key);
    if widgets::check(ui, tl!(label), on, true) {
        d.fields.insert(key.into(), json!(!on));
    }
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    for (key, label) in &SWITCHES[..3] {
        check(ui, d, key, label);
    }
    let color = d.str("color");
    let at = COLORS.iter().position(|(name, _)| *name == color).unwrap_or(1);
    widgets::label_row(ui, tl!("Color:"), LABEL_W, |ui| {
        let shown = COLOR_LABELS.get(at).copied().unwrap_or("Blue");
        if let Some((name, _)) =
            widgets::dropdown_named(ui, "circle-options-color", Some(tl!("Color:")), shown, &COLOR_LABELS, 110.0).and_then(|i| COLORS.get(i))
        {
            d.fields.insert("color".into(), json!(name));
        }
    });
    ui.add_space(8.0);
    for (key, label) in &SWITCHES[3..] {
        check(ui, d, key, label);
    }
    false
}

/// OK: set the tool's options.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let keys = SWITCHES.iter().map(|(key, _)| *key).chain(["color"]);
    let values: Map<String, Value> = keys.filter_map(|key| Some((key.to_string(), d.fields.get(key)?.clone()))).collect();
    run_and_close(app, "tool.setOption", json!({ "tool": TOOL, "values": values }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app
    }

    /// The labels the open dialog paints.
    fn painted(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    /// Every label the dialog shows has a row for each of [`COLORS`].
    #[test]
    fn every_color_has_a_label() {
        assert_eq!(COLOR_LABELS.len(), COLORS.len());
        for (label, (name, _)) in COLOR_LABELS.iter().zip(COLORS) {
            assert_eq!(label.to_lowercase(), *name);
        }
    }

    #[test]
    fn double_clicking_the_tool_opens_its_options_and_ok_sets_them() {
        let mut app = app();
        let o = app.session.tool_options_of(TOOL);
        for key in ["center", "radii", "diameter", "standardAnchors", "dragThird"] {
            assert_eq!(o[key], json!(true), "{key}");
        }
        assert_eq!(o["color"], json!("blue"));
        let r = app.run("tool.options", json!({"tool": TOOL})).unwrap();
        assert_eq!(r["dialog"], json!(KIND));
        let text = painted(&mut app);
        for label in [
            "Circle Tool Options",
            "Show Center",
            "Show Radii",
            "Show Diameter",
            "Color:",
            "Blue",
            "Anchors at Standard Positions",
            "Third Point by Dragging",
            "OK",
            "Cancel",
        ] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        for key in ["radii", "standardAnchors", "dragThird"] {
            assert!(d.bool(key), "{key}");
            d.fields.insert(key.into(), json!(false));
        }
        d.fields.insert("color".into(), json!("magenta"));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let o = app.session.tool_options_of(TOOL);
        let got: Vec<&Value> = ["center", "radii", "diameter", "standardAnchors", "dragThird", "color"].iter().map(|k| &o[*k]).collect();
        assert_eq!(got, [&json!(true), &json!(false), &json!(true), &json!(false), &json!(false), &json!("magenta")]);
        // They last while another tool is chosen, and the dialog opens on them again.
        app.select_tool(TOOL);
        app.select_tool("selection");
        app.run("tool.options", json!({"tool": TOOL})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert!(!d.bool("standardAnchors") && d.bool("center"));
        assert_eq!(d.str("color"), "magenta");
    }

    /// `tool.setOption` takes only the values the options take, and an invalid one changes none.
    #[test]
    fn invalid_values_are_errors_and_change_nothing() {
        let mut app = app();
        for (values, error) in [
            (json!({"center": false, "radii": "no"}), "`radii` must be true or false"),
            (json!({"center": false, "color": "orange"}), "`color` must be one of red, blue, magenta, green, black, gray"),
            (json!({"center": false, "size": 3}), "unknown Circle tool option `size`"),
        ] {
            let e = app.run("tool.setOption", json!({"tool": TOOL, "values": values})).unwrap_err();
            assert!(e.contains(error), "{e}");
            assert_eq!(app.session.tool_options_of(TOOL)["center"], json!(true));
        }
    }
}
