//! The canvas draws what live-effect plug-ins show besides their geometry, over the art, where it
//! draws their effects (`canvas::plugin_effect_overlay`, `plugin_overlays`): not in Outline view,
//! for hidden objects, in layers whose Preview is off or in hidden templates. The overlays come
//! with the frame that ran the plug-ins: the canvas never runs one to show them.

use egui::{Color32, Pos2, Shape, vec2};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_plugins::overlay::{FORMAT, Line, Overlay};
use vectorcraft_plugins::wat as tpl;

use crate::VectorcraftApp;

/// The colour of the line [`install_showing`]'s plug-ins show.
const BLUE: [u8; 4] = [10, 20, 250, 255];

/// An app with a new 400 × 300 pt document, whose canvas renders on this thread.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    // A worker's frame would land later than the tests look.
    app.canvas.worker_started = true;
    app
}

/// Installs live-effect plug-in `id`, which keeps the geometry it is given and shows one
/// [`BLUE`] line a pixel wide through (50, 50), (150, 50) and (150, 120).
fn install_showing(app: &mut VectorcraftApp, id: &str) {
    let overlay = Overlay { lines: vec![Line { color: BLUE, width: 0.0, points: vec![[50.0, 50.0], [150.0, 50.0], [150.0, 120.0]] }] };
    let output = json!({"overlay": {"format": FORMAT, "data": vectorcraft_format::base64_encode(&overlay.encode())}});
    let module = wat::parse_str(tpl::returning(&tpl::manifest(id, "effect", "{}"), &output.to_string())).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&module)})).unwrap();
}

/// The points of each [`BLUE`] line one canvas frame draws.
fn drawn(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<Vec<Pos2>> {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| crate::canvas::show(app, ui));
    out.textures_delta.clear();
    let [r, g, b, a] = BLUE;
    let solid = egui::epaint::ColorMode::Solid(Color32::from_rgba_unmultiplied(r, g, b, a));
    out.shapes
        .into_iter()
        .filter_map(|c| match c.shape {
            Shape::Path(p) if p.stroke.color == solid => Some(p.points),
            _ => None,
        })
        .collect()
}

/// The canvas draws a live effect's overlay as the plug-in drew it, but not in Outline view nor
/// for a hidden object.
#[test]
fn the_canvas_draws_what_a_live_effect_shows() {
    let mut app = app();
    install_showing(&mut app, "test.ui.overlay");
    let id = app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 60, "height": 60})).unwrap()["id"].clone();
    let ctx = egui::Context::default();
    assert!(drawn(&mut app, &ctx).is_empty(), "no effect yet");
    app.run("effect.apply", json!({"effect": "plugin.test.ui.overlay"})).unwrap();
    let lines = drawn(&mut app, &ctx);
    assert_eq!(lines.iter().map(Vec::len).collect::<Vec<_>>(), [3], "one line of three points");
    let [a, b, c] = [lines[0][0], lines[0][1], lines[0][2]];
    let as_drawn = (a.y - b.y).abs() < 0.01 && b.x > a.x && (c.x - b.x).abs() < 0.01 && (c.y - b.y).abs() > 1.0;
    assert!(as_drawn, "as the plug-in drew it: {:?}", lines[0]);
    // Outline view draws no effects, and a hidden object nothing.
    app.run("view.outline", json!({})).unwrap();
    assert!(drawn(&mut app, &ctx).is_empty(), "Outline view");
    app.run("view.outline", json!({})).unwrap();
    assert_eq!(drawn(&mut app, &ctx).len(), 1);
    app.run("object.hide", json!({"ids": [id]})).unwrap();
    assert!(drawn(&mut app, &ctx).is_empty(), "hidden");
    vectorcraft_plugins::registry::remove("test.ui.overlay");
}

/// Overlays show where the canvas draws effects, which Layer Options decide as drawing does: not
/// in a layer whose Preview is off (it draws in outline) nor in a template layer Hide Template
/// hides, and in Outline view in a template layer, which stays in Preview.
#[test]
fn overlays_show_where_the_canvas_draws_effects() {
    let mut app = app();
    install_showing(&mut app, "test.ui.overlay.layers");
    app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 60, "height": 60})).unwrap();
    app.run("effect.apply", json!({"effect": "plugin.test.ui.overlay.layers"})).unwrap();
    let layer = app.session.active().unwrap().doc.layers[0].id.0;
    let ctx = egui::Context::default();
    assert_eq!(drawn(&mut app, &ctx).len(), 1);
    app.run("layer.setProps", json!({"id": layer, "preview": false})).unwrap();
    assert_eq!(drawn(&mut app, &ctx).len(), 0, "Preview off");
    app.run("layer.setProps", json!({"id": layer, "preview": true, "template": true})).unwrap();
    assert_eq!(drawn(&mut app, &ctx).len(), 1, "a template layer");
    assert_eq!(app.run("view.template", json!({})).unwrap(), json!(false));
    assert_eq!(drawn(&mut app, &ctx).len(), 0, "Hide Template");
    app.run("view.template", json!({})).unwrap();
    app.run("view.outline", json!({})).unwrap();
    assert_eq!(drawn(&mut app, &ctx).len(), 1, "a template layer in Outline view");
    app.run("layer.setProps", json!({"id": layer, "template": false})).unwrap();
    assert_eq!(drawn(&mut app, &ctx).len(), 0, "an ordinary layer in Outline view");
    vectorcraft_plugins::registry::remove("test.ui.overlay.layers");
}

/// A heavy document renders on the worker, and the overlays come with the worker's frame, from
/// the runs that rendering made: until it lands, the canvas shows those of the frame it shows.
#[test]
fn a_heavy_documents_overlays_come_with_the_workers_frame() {
    let mut app = app();
    install_showing(&mut app, "test.ui.overlay.worker");
    let fx = json!({"effect": "plugin.test.ui.overlay.worker"});
    app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 60, "height": 60})).unwrap();
    app.run("effect.apply", fx.clone()).unwrap();
    let ctx = egui::Context::default();
    assert_eq!(drawn(&mut app, &ctx).len(), 1);
    // From here on, frames render on a worker this test drives.
    let (worker, jobs, done) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);
    (app.canvas.last_ms, app.canvas.from_worker) = (50.0, true);
    app.run("shape.rectangle", json!({"x": 140, "y": 40, "width": 60, "height": 60})).unwrap();
    app.run("effect.apply", fx).unwrap();
    assert_eq!(drawn(&mut app, &ctx).len(), 1, "the frame shown is from before the second rectangle");
    let job = jobs.try_recv().expect("the new frame went to the worker");
    let img = vectorcraft_render::Renderer::new().render(&job.doc, job.w, job.h, job.view, &job.opts);
    done.send(crate::render_worker::Done { key: job.key, doc: job.doc, opts: job.opts, img, ms: 50.0, moved: 0 }).unwrap();
    assert_eq!(drawn(&mut app, &ctx).len(), 2, "the worker's frame, with both");
    vectorcraft_plugins::registry::remove("test.ui.overlay.worker");
}

/// `CRAFT_UI_OVERLAY_EFFECT_WASM=<folder>` writes a live effect to try in the app
/// (`pen-preview-test-effect.wasm`): it keeps the geometry and shows a pen hatching a 200 × 150 pt
/// rectangle at (100, 80), the pen's strokes in blue and its moves between them in orange.
#[test]
fn overlay_test_effect_for_pictures() {
    let Some(dir) = std::env::var_os("CRAFT_UI_OVERLAY_EFFECT_WASM") else { return };
    let (x0, y0, x1, y1) = (104.0_f32, 86.0, 296.0, 226.0);
    let mut lines = Vec::new();
    let mut from: Option<[f32; 2]> = None;
    for (row, y) in (0..).map(|i| (i, y0 + 10.0 * i as f32)).take_while(|&(_, y)| y <= y1) {
        let (a, b) = if row % 2 == 0 { ([x0, y], [x1, y]) } else { ([x1, y], [x0, y]) };
        if let Some(f) = from {
            lines.push(Line { color: [255, 140, 0, 255], width: 0.0, points: vec![f, a] });
        }
        lines.push(Line { color: [30, 110, 255, 255], width: 0.75, points: vec![a, b] });
        from = Some(b);
    }
    let overlay = Overlay { lines };
    let output = json!({"overlay": {"format": FORMAT, "data": vectorcraft_format::base64_encode(&overlay.encode())}});
    let manifest = r#"{"id":"org.example.penpreview","name":"Pen Path Preview (test plug-in)","version":"1","kind":"effect",
        "description":"Shows how a pen would hatch the object, without changing it."}"#;
    let module = wat::parse_str(tpl::returning(manifest, &output.to_string())).unwrap();
    std::fs::write(std::path::PathBuf::from(dir).join("pen-preview-test-effect.wasm"), module).unwrap();
}
