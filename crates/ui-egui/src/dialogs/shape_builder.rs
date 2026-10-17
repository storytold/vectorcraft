//! Shape Builder Tool Options (double-click the Shape Builder tool): Gap Detection, which treats
//! open paths whose ends come within the Gap Length of another path as closed for the regions they
//! bound. The Gap Length is Small (3 pt), Medium (6 pt), Large (12 pt) or a Custom length. OK sets
//! the tool's options with `tool.setOption`; the tool keeps them across tool switches and in the
//! preferences.
//!
//! Fields: `gapDetection` (bool), `gapLength` (points), and `__custom` (the Custom choice is
//! picked, whatever the length).

use serde_json::{Value, json};

use super::document_setup::check;
use super::{DialogSpec, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the Shape Builder's Tool Options.
pub const KIND: &str = "shapeBuilderOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: 340.0, ..DialogSpec::FORM };

fn heading(_: &Dialog) -> String {
    crate::i18n::fmt(tl!("{tool} Options"), &[("tool", tl!("Shape Builder Tool"))])
}

/// Width of the label column.
const LABEL_W: f32 = 90.0;

/// The longest Custom Gap Length (points), as the engine takes.
const MAX_GAP: f64 = 1000.0;

/// The choices of the Gap Length list; the last is Custom.
const CHOICES: [&str; 4] = ["Small", "Medium", "Large", "Custom"];

/// Open the dialog on the tool's options.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let o = app.session.tool_options_of("shapeBuilder");
    let len = o["gapLength"].as_f64().unwrap_or(vectorcraft_tools::builder::GAP_LENGTHS[0]);
    let custom = !vectorcraft_tools::builder::GAP_LENGTHS.contains(&len);
    let fields = json!({ "gapDetection": o["gapDetection"].as_bool().unwrap_or(false), "gapLength": len, "__custom": custom });
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(json!({ "dialog": KIND }))
}

/// The index of the choice the fields show.
fn choice(d: &Dialog) -> usize {
    if d.bool("__custom") {
        return CHOICES.len() - 1;
    }
    let len = d.f64("gapLength", 0.0);
    vectorcraft_tools::builder::GAP_LENGTHS.iter().position(|l| *l == len).unwrap_or(CHOICES.len() - 1)
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    check(ui, d, "gapDetection", tl!("Gap Detection"));
    ui.add_space(6.0);
    let on = d.bool("gapDetection");
    ui.add_enabled_ui(on, |ui| {
        let current = choice(d);
        widgets::label_row(ui, tl!("Gap Length:"), LABEL_W, |ui| {
            let labels = CHOICES.map(|c| tl!(c));
            if let Some(i) = widgets::dropdown(ui, "shape-builder-gap", labels[current], &labels, 120.0) {
                match vectorcraft_tools::builder::GAP_LENGTHS.get(i) {
                    Some(len) => {
                        d.fields.insert("gapLength".into(), json!(len));
                        d.fields.insert("__custom".into(), json!(false));
                    }
                    None => {
                        d.fields.insert("__custom".into(), json!(true));
                    }
                }
            }
            ui.add_space(6.0);
            let len = d.f64("gapLength", 0.0);
            ui.add_enabled_ui(choice(d) == CHOICES.len() - 1, |ui| {
                if let Some(x) = widgets::plain_field(ui, "shape-builder-gap-length", len, "pt", 1, 72.0) {
                    d.fields.insert("gapLength".into(), json!(x.clamp(0.0, MAX_GAP)));
                }
            });
        });
    });
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let values = json!({ "gapDetection": d.bool("gapDetection"), "gapLength": d.f64("gapLength", 0.0) });
    run_and_close(app, "tool.setOption", json!({ "tool": "shapeBuilder", "values": values }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    #[test]
    fn the_dialog_sets_gap_detection() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("tool.options", json!({"tool": "shapeBuilder"})).unwrap();
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| super::super::show(app, ui.ctx()));
        for label in ["Shape Builder Tool Options", "Gap Detection", "Gap Length:", "Small"] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.bool("gapDetection"), d.f64("gapLength", 0.0), choice(d)), (false, 3.0, 0));
        d.fields.insert("gapDetection".into(), json!(true));
        d.fields.insert("gapLength".into(), json!(9.5));
        d.fields.insert("__custom".into(), json!(true));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.tool_options_of("shapeBuilder"), json!({"gapDetection": true, "gapLength": 9.5}));
        // It opens on a length that is none of the three as Custom.
        app.run("tool.options", json!({"tool": "shapeBuilder"})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.bool("gapDetection"), choice(d)), (true, 3));
    }
}
