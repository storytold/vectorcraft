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

/// One headless canvas frame on an 800 × 600 window, then the numbers of the tiles the marks control
/// shows for the view that frame set.
// PLAN: needs `pub(crate) mod marks` in canvas.rs (canvas-w10); the sheet's `mod marks;` is private to `canvas`.
fn tile_numbers(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<String> {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    ctx.run_ui(raw, |ui| crate::canvas::show(app, ui)).textures_delta.clear();
    let Some(view) = app.canvas_view() else { return vec![] };
    crate::canvas::marks::marks_of(app, &view).tiles.into_iter().map(|t| t.label).collect()
}

#[test]
fn the_canvas_numbers_the_print_tiles_of_print_preview() {
    let mut app = app();
    let ctx = crate::testing::ui_context();
    assert!(tile_numbers(&mut app, &ctx).is_empty(), "hidden by default");
    app.run("view.printTiling", json!({})).unwrap();
    assert_eq!(tile_numbers(&mut app, &ctx), ["1"], "the page is numbered");
    // Tiled at 400%: every tile print.preview lays out is drawn and numbered.
    app.run("print.setup", json!({"settings": {"scaling": "tileImageable", "scale": {"width": 400, "height": 400}}})).unwrap();
    let numbers = tile_numbers(&mut app, &ctx);
    let pv = app.session.execute("print.preview", &json!({})).unwrap();
    let count = pv["tiles"][0]["tiles"].as_array().unwrap().len();
    assert!(count > 1);
    assert_eq!(app.canvas.print_tiling.as_ref().map(|(_, p)| p.len()), Some(count));
    for n in 1..=count {
        assert!(numbers.contains(&n.to_string()), "tile {n} numbered: {numbers:?}");
    }
    // The Print Tiling tool shows the tiling too; double-clicking its button resets it.
    app.run("view.printTiling", json!({})).unwrap();
    app.run("print.tiling.set", json!({"origin": [10, 10]})).unwrap();
    app.select_tool("printTiling");
    assert!(tile_numbers(&mut app, &ctx).contains(&"1".to_string()));
    app.run("tool.options", json!({"tool": "printTiling"})).unwrap();
    assert_eq!(app.session.execute("print.setup", &json!({})).unwrap()["settings"]["tileOrigin"]["placed"], false);
}
