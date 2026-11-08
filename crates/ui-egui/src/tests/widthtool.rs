//! The Width tool in the UI: its cursors and hint.

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_tools::Cursor;

use crate::{VectorcraftApp, chrome, tests_labels::painted_text};

#[test]
fn the_width_tool_has_its_own_cursors() {
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        let painter = ui.painter().clone();
        for c in [Cursor::Width, Cursor::WidthAdd, Cursor::WidthPoint] {
            assert!(crate::cursors::paint(&painter, c, egui::pos2(50.0, 50.0), 1.0), "{c:?} is drawn");
        }
    });
    out.textures_delta.clear();
    assert!(out.shapes.len() > 3, "{} shapes", out.shapes.len());
}

#[test]
fn the_hint_bar_explains_the_width_tool() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("tool.select", json!({"tool": "width"})).unwrap();
    let text = painted_text(&mut app, chrome::hint_bar).replace('\n', "");
    assert!(text.contains("add a width point") && text.contains("one side") && text.contains("remove points"), "{text}");
}
