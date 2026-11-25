//! Preferences › User Interface › UI Scaling zooms the interface and not the art: at a given zoom
//! a document point covers the same device pixels at every UI Scaling. The Navigator's view box and
//! tiled panes have their own tests (`panels::navigator`, `canvas::panes`).

use egui::{Pos2, vec2};
use vectorcraft_engine::Session;
use vectorcraft_geom::{Affine, Point};

use super::*;

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
    // As the hosts run it: the zoom stops come from the preferences through the app's frame input.
    crate::testing::part_frame(app, ctx, input, show);
}

/// A 512 × 512 pt document shown at `native` device pixels a point and UI Scaling `ui_zoom`, with a
/// controlled render worker, and the receiver of the frames it is asked to render.
fn shown(native: f32, ui_zoom: f32) -> (VectorcraftApp, egui::Context, std::sync::mpsc::Receiver<crate::render_worker::Job>) {
    let ctx = crate::testing::ui_context();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 512, "height": 512})).unwrap();
    // The app's frames zoom the interface by the preference.
    app.session.prefs.ui_scaling = f64::from(ui_zoom);
    // A view command eases under Animated Zoom; these tests read the view it lands on.
    app.run("prefs.set", json!({"key": "animatedZoom", "value": false})).unwrap();
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

/// At Actual Size a 128 pt edge covers the same device pixels in the rendered frame whatever the
/// UI Scaling: the interface zooms, the art does not.
#[test]
fn a_document_point_covers_the_same_device_pixels_at_every_ui_scaling() {
    let (a, b) = (Point::new(100.0, 100.0), Point::new(228.0, 100.0));
    for native in [1.0_f32, 2.0] {
        let drawn = [1.0_f32, 2.0].map(|ui_zoom| {
            let (mut app, ctx, jobs) = shown(native, ui_zoom);
            app.run("view.actualSize", json!({})).unwrap();
            frame(&mut app, &ctx, (native, ui_zoom), vec![]);
            device_length(rendered(&jobs), a, b)
        });
        assert!(
            drawn[0] > 0.0 && (drawn[0] - drawn[1]).abs() < 1e-6,
            "{native} device pixels a point: 128 pt covers {drawn:?} device pixels at UI Scaling 1× and 2×"
        );
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
