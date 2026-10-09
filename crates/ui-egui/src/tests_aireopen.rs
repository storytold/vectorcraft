//! A document just opened from the .ai it was saved as is not modified, however many frames pass.

use egui::{Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, io};

fn frames(app: &mut VectorcraftApp, ctx: &egui::Context, n: usize) {
    for _ in 0..n {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
    }
}

#[test]
fn a_document_opened_from_its_ai_stays_unmodified() {
    let dir = std::env::temp_dir().join(format!("vc-aireopen-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("art.ai").to_string_lossy().to_string();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 80, "text": "Hello", "size": 18})).unwrap();
    s.execute("artboard.new", &json!({"x": 220, "y": 0, "width": 200, "height": 100, "name": "B"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 230, "y": 10, "width": 40, "height": 30})).unwrap();
    s.execute("file.saveAs", &json!({"path": path})).unwrap();

    let ctx = egui::Context::default();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    frames(&mut app, &ctx, 3);
    let bytes = std::fs::read(&path).unwrap();
    io::open_document(&mut app, "art.ai", &bytes, Some(path.clone()), &serde_json::Value::Null).unwrap();
    assert!(!app.session.active().unwrap().is_dirty(), "just opened");
    frames(&mut app, &ctx, 6);
    assert!(!app.session.active().unwrap().is_dirty(), "after some frames");
    let _ = std::fs::remove_dir_all(dir);
}
