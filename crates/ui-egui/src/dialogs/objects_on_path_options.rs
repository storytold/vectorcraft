//! Objects on Path Tool Options (double-click the Objects on Path tool): the Pivot and Rotate of
//! the objects the tool attaches next. Groups attached before keep theirs. Save sets them with
//! `tool.setOption {tool: "objectsOnPath", values}`; they last across tool switches and are saved
//! with the preferences. Reset puts the center and 0 back.
//!
//! Fields: `pivot` (0-8, row by row from the top left; 4, the center) and `rotate` (degrees
//! counter-clockwise from upright on the path).
//!
//! Behavior: <https://helpx.adobe.com/illustrator/using/objects-on-path.html>.

use serde_json::{Map, Value, json};
use vectorcraft_doc::objects_on_path::CENTER;

use super::{DialogSpec, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Objects on Path Tool Options.
pub const KIND: &str = "objectsOnPathOptions";

/// The tool whose options the dialog edits.
const TOOL: &str = "objectsOnPath";

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, ok: Some("Save"), reset: Some(reset), min_width: 300.0, ..DialogSpec::FORM };

/// Width of the label column.
const LABEL_W: f32 = 70.0;

fn heading(_: &Dialog) -> String {
    let label = vectorcraft_tools::tool_info(TOOL).map_or("", |t| t.label);
    crate::i18n::fmt(tl!("{tool} Options"), &[("tool", tl!(label))])
}

/// Open the tool's options with their current values.
pub fn open(app: &mut VectorcraftApp) -> Value {
    let o = app.session.tool_options_of(TOOL);
    let fields: Map<String, Value> = ["pivot", "rotate"].iter().filter_map(|key| Some((key.to_string(), o.get(*key)?.clone()))).collect();
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    json!({ "dialog": KIND })
}

/// Reset: the center and 0.
fn reset(_: &VectorcraftApp, d: &mut Dialog) {
    d.fields.insert("pivot".into(), json!(CENTER));
    d.fields.insert("rotate".into(), json!(0.0));
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    widgets::label_row(ui, tl!("Pivot:"), LABEL_W, |ui| {
        let current = d.f64("pivot", f64::from(CENTER)).clamp(0.0, 8.0) as usize;
        if let Some(i) = widgets::reference_point(ui, current) {
            d.fields.insert("pivot".into(), json!(i));
        }
    });
    ui.add_space(8.0);
    widgets::label_row(ui, tl!("Rotate:"), LABEL_W, |ui| {
        if let Some(r) = widgets::range_field(ui, "objects-on-path-rotate", d.f64("rotate", 0.0), -360.0..=360.0, "°", 1, 64.0) {
            d.fields.insert("rotate".into(), json!(r));
        }
    });
    ui.add_space(8.0);
    let r = widgets::flat_button(ui, tl!("Reset"), 64.0);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Reset")));
    if r.clicked() {
        reset(app, d);
    }
    false
}

/// Save: set the tool's Pivot and Rotate.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let values: Map<String, Value> = ["pivot", "rotate"].iter().filter_map(|k| Some((k.to_string(), d.fields.get(*k)?.clone()))).collect();
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

    /// The Pivot and Rotate of the Objects on Path group `g`.
    fn record(app: &VectorcraftApp, g: &Value) -> (u8, f64) {
        let id = vectorcraft_doc::NodeId(g["id"].as_u64().unwrap());
        let n = app.session.doc().unwrap().doc.node(id).unwrap();
        n.extras().objects_on_path.as_deref().map(|r| (r.pivot, r.rotate)).unwrap()
    }

    #[test]
    fn double_clicking_the_tool_opens_its_options_and_save_sets_the_next_attach() {
        let mut app = app();
        let first = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        let line = app.run("shape.line", json!({"x1": 50, "y1": 300, "x2": 350, "y2": 300})).unwrap();
        app.run("select.set", json!({"ids": [first["id"]]})).unwrap();
        let before = app.run("object.objectsOnPath.attach", json!({"path": line["id"]})).unwrap();
        let r = app.run("tool.options", json!({"tool": "objectsOnPath"})).unwrap();
        assert_eq!(r["dialog"], json!(KIND));
        let text = painted(&mut app);
        for label in ["Objects on Path Tool Options", "Pivot:", "Rotate:", "Reset", "Save", "Cancel"] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.f64("pivot", -1.0), d.f64("rotate", -1.0)), (4.0, 0.0), "the center and 0");
        d.fields.insert("pivot".into(), json!(7));
        d.fields.insert("rotate".into(), json!(30));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        // They last while another tool is chosen, and the dialog opens on them again.
        app.select_tool("objectsOnPath");
        app.select_tool("selection");
        assert_eq!(app.session.tool_options_of(TOOL), json!({"pivot": 7, "rotate": 30.0}));
        // The objects attached next take them; the group attached before keeps its own.
        let second = app.run("shape.rectangle", json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap();
        let other = app.run("shape.line", json!({"x1": 50, "y1": 380, "x2": 350, "y2": 380})).unwrap();
        app.run("select.set", json!({"ids": [second["id"]]})).unwrap();
        let next = app.run("object.objectsOnPath.attach", json!({"path": other["id"]})).unwrap();
        assert_eq!(record(&app, &next), (7, 30.0));
        assert_eq!(record(&app, &before), (4, 0.0));
        // Reset puts the center and 0 back, and Save keeps them.
        app.run("tool.options", json!({"tool": "objectsOnPath"})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.f64("pivot", -1.0), d.f64("rotate", -1.0)), (7.0, 30.0));
        super::super::reset(&mut app).unwrap();
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.tool_options_of(TOOL), json!({"pivot": 4, "rotate": 0.0}));
    }
}
