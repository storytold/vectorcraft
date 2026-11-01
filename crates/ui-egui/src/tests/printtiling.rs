//! View → Show Print Tiling and the Print Tiling tool on the canvas and in the menus.

use egui::{Pos2, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

/// One headless canvas frame on an 800 × 600 window; the texts it painted.
fn frame_texts(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<String> {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| crate::canvas::show(app, ui));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_canvas_numbers_the_print_tiles_of_print_preview() {
    let mut app = app();
    let ctx = egui::Context::default();
    assert!(!frame_texts(&mut app, &ctx).iter().any(|t| t == "1"), "hidden by default");
    assert_eq!(crate::menus::dynamic_label(&app, "view.printTiling", "Show Print Tiling"), "Show Print Tiling");
    app.run("view.printTiling", json!({})).unwrap();
    assert_eq!(crate::menus::dynamic_label(&app, "view.printTiling", "Show Print Tiling"), "Hide Print Tiling");
    assert!(frame_texts(&mut app, &ctx).iter().any(|t| t == "1"), "the page is numbered");
    // Tiled at 400%: every tile print.preview lays out is drawn and numbered.
    app.run("print.setup", json!({"settings": {"scaling": "tileImageable", "scale": {"width": 400, "height": 400}}})).unwrap();
    let texts = frame_texts(&mut app, &ctx);
    let pv = app.session.execute("print.preview", &json!({})).unwrap();
    let count = pv["tiles"][0]["tiles"].as_array().unwrap().len();
    assert!(count > 1);
    assert_eq!(app.canvas.print_tiling.as_ref().map(|(_, p)| p.len()), Some(count));
    for n in 1..=count {
        assert!(texts.contains(&n.to_string()), "tile {n} numbered: {texts:?}");
    }
    // The Print Tiling tool shows the tiling too; double-clicking its button resets it.
    app.run("view.printTiling", json!({})).unwrap();
    app.run("print.tiling.set", json!({"origin": [10, 10]})).unwrap();
    app.select_tool("printTiling");
    assert!(frame_texts(&mut app, &ctx).iter().any(|t| t == "1"));
    app.run("tool.options", json!({"tool": "printTiling"})).unwrap();
    assert_eq!(app.session.execute("print.setup", &json!({})).unwrap()["settings"]["tileOrigin"]["placed"], false);
}
