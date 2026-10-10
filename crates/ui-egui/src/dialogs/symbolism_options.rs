//! Symbolism Tools Options (double-click a Symbolism tool): the brush the eight tools share, which
//! lasts across tool switches and is saved with the preferences. OK sets it with
//! `tool.setOption {tool, values}`.
//!
//! Fields: `tool`; `diameter` (points), `intensity` and `density` (Symbol Set Density), 1–10.

use serde_json::{Map, Value, json};

use super::{DialogResult, DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the Symbolism Tools Options.
pub const KIND: &str = "symbolismOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Symbolism Tools Options").into(), body, confirm, min_width: 320.0, ..DialogSpec::FORM };

/// The options the dialog edits.
const KEYS: [&str; 3] = ["diameter", "intensity", "density"];

/// Width of the label column.
const LABEL_W: f32 = 130.0;

/// Open the options of Symbolism tool `tool` (one of `vectorcraft_tools::settings::SYMBOLISM`)
/// with their current values.
pub fn open(app: &mut VectorcraftApp, tool: &str) -> Value {
    let o = app.session.tool_options_of(tool);
    let mut fields: Map<String, Value> = KEYS.iter().filter_map(|key| Some((key.to_string(), o.get(*key)?.clone()))).collect();
    fields.insert("tool".into(), json!(tool));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    json!({ "dialog": KIND })
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    widgets::label_row(ui, tl!("Diameter:"), LABEL_W, |ui| {
        form::length(ui, d, "diameter", unit, 100.0);
    });
    for (key, label) in [("intensity", tl!("Intensity:")), ("density", tl!("Symbol Set Density:"))] {
        widgets::label_row(ui, label, LABEL_W, |ui| {
            if let Some(v) = widgets::range_field(ui, ("symbolism", key), d.f64(key, 5.0), 1.0..=10.0, "", 0, 100.0) {
                d.fields.insert(key.into(), json!(v));
            }
        });
    }
    false
}

/// OK: set the brush the Symbolism tools share (the tool keeps it within its ranges).
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let values: Map<String, Value> = KEYS.iter().filter(|k| d.fields.contains_key(**k)).map(|k| (k.to_string(), json!(d.f64(k, 0.0)))).collect();
    run_and_close(app, "tool.setOption", json!({ "tool": d.str("tool"), "values": values }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    #[test]
    fn the_symbolism_tools_share_the_options_ok_sets() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("tool.options", json!({"tool": "symbolSizer"})).unwrap();
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| super::super::show(app, ui.ctx()));
        for label in ["Symbolism Tools Options", "Diameter:", "Intensity:", "Symbol Set Density:"] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.str("tool"), d.f64("diameter", 0.0), d.f64("intensity", 0.0)), ("symbolSizer".to_string(), 80.0, 5.0));
        for (k, v) in [("diameter", json!(120)), ("intensity", json!(9)), ("density", json!(2))] {
            d.fields.insert(k.into(), v);
        }
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        // The Sprayer has them too: one brush for the eight tools.
        let o = app.session.tool_options_of("symbolSprayer");
        assert_eq!((o["diameter"].as_f64(), o["intensity"].as_f64(), o["density"].as_f64()), (Some(120.0), Some(9.0), Some(2.0)));
    }
}
