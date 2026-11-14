//! Preferences › User Interface › UI Scaling zooms the interface and not the art: at a given zoom
//! a document point covers the same device pixels at every UI Scaling, and the pointer lands on the
//! art drawn under it. The Navigator's view box and tiled panes have their own tests
//! (`panels::navigator`, `canvas::panes`).

use super::*;
use vectorcraft_engine::Session;

/// The window, in points of the screen before UI Scaling.
const WINDOW: egui::Vec2 = vec2(1200.0, 900.0);

/// One canvas frame in a `WINDOW` window at `native` device pixels a point, with the interface
/// zoomed by `ui_zoom`.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, (native, ui_zoom): (f32, f32), events: Vec<egui::Event>) {
    ctx.set_zoom_factor(ui_zoom);
    let mut input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, WINDOW / ui_zoom)), events, ..Default::default() };
    input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native);
    let mut out = ctx.run_ui(input, |ui| show(app, ui));
    out.textures_delta.clear();
}

/// A 512 × 512 pt document shown at `native` device pixels a point and UI Scaling `ui_zoom`, with a
/// controlled render worker, and the receiver of the frames it is asked to render.
fn shown(native: f32, ui_zoom: f32) -> (VectorcraftApp, egui::Context, std::sync::mpsc::Receiver<crate::render_worker::Job>) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
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
            let xf = app.canvas_xf().unwrap();
            let pointed = f64::from((xf.to_screen(b) - xf.to_screen(a)).length() * ctx.pixels_per_point());
            assert!((pointed - device(1.0)).abs() < 1e-3, "{at}: the pointer maps 128 pt to {pointed} device pixels");

            app.run("view.fitArtboard", json!({})).unwrap();
            frame(&mut app, &ctx, (native, ui_zoom), vec![]);
            let zoom = app.view().unwrap().zoom;
            let view = rendered(&jobs);
            let drawn = device_length(view, a, b);
            assert!((drawn - device(zoom)).abs() < 1e-6, "{at}: at {zoom} the frame draws 128 pt over {drawn} device pixels");
            // The artboard fits the canvas with 30 interface points to spare on its closer sides.
            let rect = app.canvas_rect.unwrap();
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

/// At 100% a 20 pt square 100 pt to the right of the view's center is 100 device pixels a point
/// to the right of the canvas's center: a click there selects it, the frame draws it there, and the
/// tools measure their tolerances in the interface's points, whatever the UI Scaling.
#[test]
fn a_click_at_the_true_size_position_selects_the_object_drawn_there_at_every_ui_scaling() {
    for (native, ui_zoom) in [(1.0_f32, 0.75_f32), (1.0, 1.5), (2.0, 1.5), (1.0, 2.0)] {
        let at = format!("{native} device pixels a point, UI Scaling {ui_zoom}×");
        let (mut app, ctx, jobs) = shown(native, ui_zoom);
        app.run("view.actualSize", json!({})).unwrap();
        let c = app.view().unwrap().center;
        let square = Point::new(c.x + 100.0, c.y);
        let id =
            app.session.execute("shape.rectangle", &json!({"x": square.x - 10.0, "y": square.y - 10.0, "width": 20, "height": 20})).unwrap()["id"]
                .as_u64()
                .unwrap();
        app.session.execute("select.none", &json!({})).unwrap();
        frame(&mut app, &ctx, (native, ui_zoom), vec![]);
        let rect = app.canvas_rect.unwrap();
        let ppp = ctx.pixels_per_point();
        let p = rect.center() + vec2(100.0 * native / ppp, 0.0);
        let button = |pressed| egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, (native, ui_zoom), vec![egui::Event::PointerMoved(p)]);
        frame(&mut app, &ctx, (native, ui_zoom), vec![button(true)]);
        frame(&mut app, &ctx, (native, ui_zoom), vec![button(false)]);
        let selected = app.session.active().unwrap().selection.objects.clone();
        assert_eq!(selected, vec![vectorcraft_doc::NodeId(id)], "{at}: a click at {p:?}");
        let device = rendered(&jobs) * square;
        let drawn = rect.min + vec2(device.x as f32, device.y as f32) / ppp;
        assert!((drawn - p).length() * ppp < 1.0, "{at}: drawn at {drawn:?}, not {p:?}");
        assert!((app.view_info().zoom - 1.0 / f64::from(ui_zoom)).abs() < 1e-9, "{at}: the tools' zoom is {}", app.view_info().zoom);
    }
}

/// A pinch zooms about the pointer: the document point under the pointer stays under it, whatever
/// the UI Scaling.
#[test]
fn a_pinch_zooms_about_the_pointer_at_every_ui_scaling() {
    for (native, ui_zoom) in [(1.0_f32, 0.75_f32), (1.0, 1.5), (2.0, 2.0)] {
        let at = format!("{native} device pixels a point, UI Scaling {ui_zoom}×");
        let (mut app, ctx, _jobs) = shown(native, ui_zoom);
        let rect = app.canvas_rect.unwrap();
        let p = rect.center() + vec2(150.0, -90.0) / ui_zoom;
        frame(&mut app, &ctx, (native, ui_zoom), vec![egui::Event::PointerMoved(p)]);
        let (before, z0) = (app.canvas_xf().unwrap().to_doc(p), app.view().unwrap().zoom);
        frame(&mut app, &ctx, (native, ui_zoom), vec![egui::Event::PointerMoved(p), egui::Event::Zoom(1.25)]);
        for _ in 0..4 {
            frame(&mut app, &ctx, (native, ui_zoom), vec![]);
        }
        let z1 = app.view().unwrap().zoom;
        assert!(z1 > z0 * 1.01, "{at}: the pinch zooms in, {z0} → {z1}");
        let after = app.canvas_xf().unwrap().to_doc(p);
        let moved = (after - before).hypot() * app.view().unwrap().screen_zoom(ui_zoom) * f64::from(ctx.pixels_per_point());
        assert!(moved < 1e-3, "{at}: {before:?} under the pointer moved {moved} device pixels to {after:?}");
    }
}

/// With Animated Zoom off, the Zoom tool zooms to the area dragged across: the zoom grows by the
/// times the marquee fits in the canvas, and the marquee's area then fills the canvas, whatever
/// the UI Scaling.
#[test]
fn the_zoom_tools_marquee_fills_the_canvas_at_every_ui_scaling() {
    for (native, ui_zoom) in [(1.0_f32, 0.75_f32), (1.0, 1.5), (2.0, 2.0)] {
        let at = format!("{native} device pixels a point, UI Scaling {ui_zoom}×");
        let (mut app, ctx, _jobs) = shown(native, ui_zoom);
        app.run("prefs.set", json!({"key": "animatedZoom", "value": false})).unwrap();
        app.select_tool("zoom");
        frame(&mut app, &ctx, (native, ui_zoom), vec![]);
        let rect = app.canvas_rect.unwrap();
        let (from, to) = (rect.center() - vec2(60.0, 20.0), rect.center() + vec2(40.0, 30.0));
        let xf = app.canvas_xf().unwrap();
        let (a, b, z0) = (xf.to_doc(from), xf.to_doc(to), app.view().unwrap().zoom);
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, (native, ui_zoom), vec![egui::Event::PointerMoved(from)]);
        frame(&mut app, &ctx, (native, ui_zoom), vec![button(from, true)]);
        frame(&mut app, &ctx, (native, ui_zoom), vec![egui::Event::PointerMoved(to)]);
        frame(&mut app, &ctx, (native, ui_zoom), vec![button(to, false)]);
        frame(&mut app, &ctx, (native, ui_zoom), vec![]);
        let fill = f64::from((rect.width() / 100.0).min(rect.height() / 50.0));
        let z1 = app.view().unwrap().zoom;
        assert!((z1 - z0 * fill).abs() < 1e-6, "{at}: {z0} → {z1}, not {}", z0 * fill);
        let xf = app.canvas_xf().unwrap();
        let shown = xf.to_screen(b) - xf.to_screen(a);
        let filled = (shown.x - rect.width()).abs().min((shown.y - rect.height()).abs());
        assert!(filled < 1e-2, "{at}: the marquee's area is {shown:?} in a {:?} canvas", rect.size());
    }
}

/// A middle-button drag moves the art with the pointer: the document point under the press is
/// under the release, whatever the UI Scaling.
#[test]
fn a_pan_moves_the_art_with_the_pointer_at_every_ui_scaling() {
    for (native, ui_zoom) in [(1.0_f32, 0.75_f32), (1.0, 1.5), (2.0, 2.0)] {
        let (mut app, ctx, _jobs) = shown(native, ui_zoom);
        let rect = app.canvas_rect.unwrap();
        let (from, to) = (rect.center(), rect.center() + vec2(60.0, -40.0));
        let pressed = app.canvas_xf().unwrap().to_doc(from);
        let middle = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Middle, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, (native, ui_zoom), vec![egui::Event::PointerMoved(from)]);
        frame(&mut app, &ctx, (native, ui_zoom), vec![middle(from, true)]);
        for k in 1..=4 {
            frame(&mut app, &ctx, (native, ui_zoom), vec![egui::Event::PointerMoved(from + (to - from) * (k as f32 / 4.0))]);
        }
        frame(&mut app, &ctx, (native, ui_zoom), vec![middle(to, false)]);
        let released = app.canvas_xf().unwrap().to_doc(to);
        let ppp = f64::from(ctx.pixels_per_point());
        let zoom = app.view().unwrap().screen_zoom(ui_zoom);
        assert!(
            (released - pressed).hypot() * zoom * ppp < 1.0,
            "{native} device pixels a point, UI Scaling {ui_zoom}×: {pressed:?} is now under the pointer as {released:?}"
        );
    }
}
