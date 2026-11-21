//! Curvature Circle Tool Options (double-click the Curvature Circle tool, or `tool.options`): Snap
//! Tolerance is how near, in screen points, a path pulls the pointer (Smart Guides' Snapping
//! Tolerance applies when it is larger); with Snap with Smart Guides Off unchecked, the pointer
//! reaches no further than the selection tolerance while Smart Guides are off; Annotate Radius
//! off hides the radius label; Anchors at Standard Positions off gives the circle an anchor where
//! it touches the path and one every 90° from there; Use the Radius of Circular Arcs off gives the
//! curve's own curvature on the curve of a circular arc too. OK sets them with
//! `tool.setOption {tool: "curvatureCircle", values}`; they last across tool switches and are saved
//! with the preferences.

use serde_json::{Map, Value, json};
use vectorcraft_tools::curvature_circle::{DEFAULT_SNAP, ID as TOOL, SNAP_RANGE};

use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Curvature Circle Tool Options.
pub const KIND: &str = "curvatureCircleOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: 320.0, ..DialogSpec::FORM };

/// The switches the dialog edits, in the order it shows them, with their labels.
const SWITCHES: [(&str, &str); 4] = [
    ("snapWithoutGuides", "Snap with Smart Guides Off"),
    ("annotateRadius", "Annotate Radius"),
    ("standardAnchors", "Anchors at Standard Positions"),
    ("constantRadius", "Use the Radius of Circular Arcs"),
];

/// Width of the label column.
const LABEL_W: f32 = 100.0;

fn heading(_: &Dialog) -> String {
    let label = vectorcraft_tools::tool_info(TOOL).map_or("", |t| t.label);
    crate::i18n::fmt(tl!("{tool} Options"), &[("tool", tl!(label))])
}

/// The option keys the dialog edits.
fn keys() -> impl Iterator<Item = &'static str> {
    ["snapTolerance"].into_iter().chain(SWITCHES.iter().map(|(key, _)| *key))
}

/// Open the tool's options with their current values.
pub fn open(app: &mut VectorcraftApp) -> Value {
    let o = app.session.tool_options_of(TOOL);
    let fields: Map<String, Value> = keys().filter_map(|key| Some((key.to_string(), o.get(key)?.clone()))).collect();
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    json!({ "dialog": KIND })
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let (min, max) = SNAP_RANGE;
    let v = d.f64("snapTolerance", DEFAULT_SNAP);
    widgets::label_row(ui, tl!("Snap Tolerance:"), LABEL_W, |ui| {
        if let Some(x) = widgets::range_field(ui, "curvature-circle-snap", v, min..=max, "px", 1, 72.0) {
            d.fields.insert("snapTolerance".into(), json!(x));
        }
    });
    ui.add_space(6.0);
    for (key, label) in SWITCHES {
        form::check(ui, d, key, tl!(label));
    }
    false
}

/// OK: set the tool's options.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let values: Map<String, Value> = keys().filter_map(|key| Some((key.to_string(), d.fields.get(key)?.clone()))).collect();
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

    #[test]
    fn double_clicking_the_tool_opens_its_options_and_ok_sets_them() {
        let mut app = app();
        let r = app.run("tool.options", json!({"tool": TOOL})).unwrap();
        assert_eq!(r["dialog"], json!(KIND));
        let text = painted(&mut app);
        for label in [
            "Curvature Circle Tool Options",
            "Snap Tolerance:",
            "Snap with Smart Guides Off",
            "Annotate Radius",
            "Anchors at Standard Positions",
            "Use the Radius of Circular Arcs",
            "OK",
            "Cancel",
        ] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        for (key, _) in SWITCHES {
            assert!(d.bool(key), "{key}");
        }
        let before = d.f64("snapTolerance", 0.0);
        d.fields.insert("snapTolerance".into(), json!(before + 3.0));
        for key in ["annotateRadius", "standardAnchors"] {
            d.fields.insert(key.into(), json!(false));
        }
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let o = app.session.tool_options_of(TOOL);
        assert_eq!(o["snapTolerance"], json!(before + 3.0));
        assert_eq!((&o["annotateRadius"], &o["standardAnchors"], &o["constantRadius"]), (&json!(false), &json!(false), &json!(true)));
        // They last while another tool is chosen, and the dialog opens on them again.
        app.select_tool(TOOL);
        app.select_tool("selection");
        app.run("tool.options", json!({"tool": TOOL})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert!(!d.bool("standardAnchors") && d.bool("snapWithoutGuides"));
        assert_eq!(d.f64("snapTolerance", 0.0), before + 3.0);
    }

    /// `tool.setOption` takes only the values the options take, and an invalid one changes none.
    #[test]
    fn invalid_values_are_errors_and_change_nothing() {
        let mut app = app();
        for values in [
            json!({"annotateRadius": false, "constantRadius": "no"}),
            json!({"annotateRadius": false, "snapTolerance": 0}),
            json!({"annotateRadius": false, "size": 3}),
        ] {
            assert!(app.run("tool.setOption", json!({"tool": TOOL, "values": values})).is_err(), "{values}");
            assert_eq!(app.session.tool_options_of(TOOL)["annotateRadius"], json!(true));
        }
    }
}
