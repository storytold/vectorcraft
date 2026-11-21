//! Preferences › User Interface › UI Scaling zooms the interface and not the art: at a given zoom
//! a document point covers the same device pixels at every UI Scaling. The Navigator's view box and
//! tiled panes have their own tests (`panels::navigator`, `canvas::panes`).

use super::*;
use vectorcraft_engine::Session;

/// The window, in points of the screen before UI Scaling.
const WINDOW: egui::Vec2 = vec2(1200.0, 900.0);

/// One canvas frame in a `WINDOW` window at `native` device pixels a point, with the interface
/// zoomed by `ui_zoom`, on a display whose textures are up to 8,192 pixels a side.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, (native, ui_zoom): (f32, f32), events: Vec<egui::Event>) {
    ctx.set_zoom_factor(ui_zoom);
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, WINDOW / ui_zoom)),
        max_texture_side: Some(8192),
        events,
        ..Default::default()
    };
    input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native);
    let mut out = ctx.run_ui(input, |ui| show(app, ui));
    out.textures_delta.clear();
}

/// A 512 × 512 pt document shown at `native` device pixels a point and UI Scaling `ui_zoom`, with a
/// controlled render worker, and the receiver of the frames it is asked to render.
fn shown(native: f32, ui_zoom: f32) -> (VectorcraftApp, egui::Context, std::sync::mpsc::Receiver<crate::render_worker::Job>) {
    // PLAN: assumed `crate::testing::ui_context() -> egui::Context` (the context with the theme's fonts installed).
    let ctx = crate::testing::ui_context();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 512, "height": 512})).unwrap();
    app.canvas.worker_started = true;
    let (worker, jobs, _done) = crate::render_worker::Worker::test_channel();
    app.canvas.worker = Some(worker);
    // The first frames set the UI zoom and fit the artboard.
    frame(&mut app, &ctx, (native, ui_zoom), vec![]);
    frame(&mut app, &ctx, (native, ui_zoom), vec![]);
    assert!((ctx.pixels_per_point() - native * ui_zoom).abs() < 1e-4, "{} device pixels a point", ctx.pixels_per_point());
    (app, ctx, jobs)
}

/// The view matrix of the last frame the canvas asked the worker to render.
fn rendered(jobs: &std::sync::mpsc::Receiver<crate::render_worker::Job>) -> Affine {
    jobs.try_iter().last().expect("a frame was rendered").view
}

/// The device pixels between two document points in the frame `view` renders.
fn device_length(view: Affine, a: Point, b: Point) -> f64 {
    (view * b - view * a).hypot()
}

/// At 100% a 128 pt edge covers 128 device pixels a point of the screen, whatever the UI Scaling,
/// in the rendered frame and in the canvas's pointer mapping; a fitted artboard draws at the zoom
/// the view reports and fits the canvas.
#[test]
fn a_document_point_covers_the_same_device_pixels_at_every_ui_scaling() {
    let (a, b) = (Point::new(100.0, 100.0), Point::new(228.0, 100.0));
    for native in [1.0_f32, 2.0] {
        for ui_zoom in [0.75_f32, 1.0, 1.5, 2.0] {
            let at = format!("{native} device pixels a point, UI Scaling {ui_zoom}×");
            let (mut app, ctx, jobs) = shown(native, ui_zoom);
            let device = |zoom: f64| 128.0 * zoom * f64::from(native);

            app.run("view.actualSize", json!({})).unwrap();
            frame(&mut app, &ctx, (native, ui_zoom), vec![]);
            assert_eq!(app.view().unwrap().zoom, 1.0, "{at}: Actual Size is 100%");
            let drawn = device_length(rendered(&jobs), a, b);
            assert!((drawn - device(1.0)).abs() < 1e-6, "{at}: the frame draws 128 pt over {drawn} device pixels");
            let shown_view = app.canvas_view().unwrap();
            let pointed = f64::from((shown_view.to_screen(b) - shown_view.to_screen(a)).length() * ctx.pixels_per_point());
            assert!((pointed - device(1.0)).abs() < 1e-3, "{at}: the pointer maps 128 pt to {pointed} device pixels");

            app.run("view.fitArtboard", json!({})).unwrap();
            frame(&mut app, &ctx, (native, ui_zoom), vec![]);
            let zoom = app.view().unwrap().zoom;
            let view = rendered(&jobs);
            let drawn = device_length(view, a, b);
            assert!((drawn - device(zoom)).abs() < 1e-6, "{at}: at {zoom} the frame draws 128 pt over {drawn} device pixels");
            // The artboard fits the canvas with 30 interface points to spare on its closer sides.
            let rect = app.canvas_view().unwrap().rect;
            let ppp = f64::from(ctx.pixels_per_point());
            let (lo, hi) = (view * Point::new(0.0, 0.0), view * Point::new(512.0, 512.0));
            let (w, h) = (f64::from(rect.width()) * ppp, f64::from(rect.height()) * ppp);
            let spare = [lo.x, lo.y, w - hi.x, h - hi.y].into_iter().fold(f64::INFINITY, f64::min);
            assert!((spare - 30.0 * ppp).abs() < 1.0, "{at}: {spare} device pixels to spare in {w} × {h}");
        }
    }
}

/// View › Pixel Preview shows document pixels while one is bigger than a device pixel: at 80% on
/// a screen of one device pixel a point it does not, whatever the UI Scaling.
#[test]
fn pixel_preview_compares_a_document_pixel_with_a_device_pixel() {
    for (native, zoom, pixels) in [(1.0_f32, 0.8, false), (1.0, 1.25, true), (2.0, 0.8, true)] {
        for ui_zoom in [0.75_f32, 1.0, 1.5, 2.0] {
            let (mut app, ctx, jobs) = shown(native, ui_zoom);
            app.ui.view.pixel_preview = true;
            app.run("view.setZoom", json!({"zoom": zoom * 100.0})).unwrap();
            frame(&mut app, &ctx, (native, ui_zoom), vec![]);
            let job = jobs.try_iter().last().expect("a frame was rendered");
            assert_eq!(job.key.pixel.is_some(), pixels, "{native} device pixels a point at {zoom}, UI Scaling {ui_zoom}×");
        }
    }
}
