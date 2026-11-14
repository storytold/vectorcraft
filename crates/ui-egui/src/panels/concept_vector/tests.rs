//! The Concept to Vector panel drawn headless: its routes (Window, Object, the context menu,
//! Properties and the Control bar), why it can't run, the style limit, the disabled prompt,
//! Generate at once and off the UI thread, Cancel, and Insert Selected as one undo step.

use std::sync::Arc;

use egui::{Color32, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::Session;
use vectorcraft_engine::concept_vector::{Capabilities, Provider, ProviderError, Request, Steps, Style, Variation, VectorPath, offline};
use vectorcraft_geom::{PathData, Point, SubPath};

use super::{
    CANCEL, ID, OPEN, PREVIEW_H, PREVIEW_POINTS, Work, art_rect, cancel, elements, finish, generate, insert, poll, preview, preview_width, show,
    show_job, within_budget,
};
use crate::tests_labels::painted_text;
use crate::tests_removeanchors::{at, click_control, control_frame, has, properties_frame};
use crate::{VectorcraftApp, menus};

/// Waits for the run on its thread to end and keeps its results, as `poll` does once they are there.
fn wait(app: &mut VectorcraftApp) {
    let Some(r) = app.concept_runs.running.take() else { return };
    let Work::Thread(rx) = &r.work else { panic!("the run is not on a thread") };
    let done = rx.recv().ok();
    finish(app, &r.token, done);
}

/// The styles made so far of a run made one part a frame.
fn made_so_far(app: &VectorcraftApp) -> Option<usize> {
    match &app.concept_runs.running.as_ref()?.work {
        Work::Frames(_, made) => Some(made.styles_done()),
        Work::Thread(_) => None,
    }
}

/// A PNG of `w` × `h` pixels: black on the left half, a red square on the right, white elsewhere.
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba(if x < w / 2 {
            [0, 0, 0, 255]
        } else if (w * 5 / 8..w * 7 / 8).contains(&x) && (h / 4..h * 3 / 4).contains(&y) {
            [220, 30, 30, 255]
        } else {
            [255; 4]
        })
    });
    let mut out = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// An app with a 400 × 300 document and one embedded image of 40 × 30 pixels, selected.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let data = vectorcraft_format::base64_encode(&png(40, 30));
    app.run("file.place", json!({"name": "art.png", "dataBase64": data, "link": false, "at": [60, 60]})).unwrap();
    app
}

fn undo_steps(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

fn root_count(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, |c| c.len())).sum()
}

#[test]
fn the_panel_is_registered_with_a_bundled_icon_and_opens_from_window() {
    let row = crate::state::ICON_PANELS.iter().find(|(id, ..)| *id == ID).expect("registered");
    assert_eq!(row.1, "Concept to Vector");
    assert!(crate::icons::exists(row.2), "{}", row.2);
    let mut app = app();
    let item = menus::menu_entries(&app).into_iter().find(|e| e.path == ["Window"] && e.label == "Concept to Vector").expect("Window item");
    let (id, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
    menus::invoke(&mut app, &id, p);
    assert!(crate::panel_docking::showing(&app.ui, ID));
}

/// Object › Concept to Vector… and the context menu need one image (or Image Trace object) that is
/// shown and unlocked; without one the Object item is disabled, the context menu leaves it out and
/// the command says why.
#[test]
fn object_and_context_menus_open_it_for_one_image_only() {
    let mut app = app();
    let object = |app: &VectorcraftApp| {
        menus::menu_entries(app).into_iter().find(|e| e.path == ["Object"] && e.label == "Concept to Vector…").expect("Object item")
    };
    let in_context = |app: &VectorcraftApp| menus::context_entries(app).iter().any(|e| e.label == "Concept to Vector…");
    assert!(object(&app).enabled && in_context(&app));
    let item = object(&app);
    let (id, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
    menus::invoke(&mut app, &id, p);
    assert!(crate::panel_docking::showing(&app.ui, ID), "{}", app.ui.status);

    app.run("object.lock", json!({})).unwrap();
    app.run("select.none", json!({})).unwrap();
    assert!(!object(&app).enabled && !in_context(&app));
    let e = app.run(OPEN, json!({})).unwrap_err();
    assert!(e.contains("select one image or an unexpanded Image Trace object"), "{e}");

    app.run("object.unlockAll", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 250, "y": 200, "width": 20, "height": 20})).unwrap();
    assert!(!object(&app).enabled && !in_context(&app), "a rectangle is not a source");
    app.run("select.all", json!({})).unwrap();
    assert!(!object(&app).enabled, "two objects are not one source");
}

/// Properties' Quick Actions and the Control bar offer it for a selected image, and the Control
/// bar's button opens the panel.
#[test]
fn properties_and_the_control_bar_offer_it() {
    let mut app = app();
    assert!(has(&properties_frame(&mut app, 280.0), "Concept to Vector"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let bar = control_frame(&mut app, &ctx, vec![]);
    click_control(&mut app, &ctx, at(&bar, "Concept to Vector"));
    assert!(crate::panel_docking::showing(&app.ui, ID), "{}", app.ui.status);
}

/// The panel says its styles are offline Image Trace styles, disables the prompt, allows 1 to 8
/// styles, and shows why it can't run with nothing to use.
#[test]
fn the_panel_shows_offline_styles_a_disabled_prompt_and_why_it_cannot_run() {
    let mut app = app();
    let text = painted_text(&mut app, show);
    for s in ["Offline Image Trace styles, made on this computer", "Offline styles do not use prompts", "Detail", "5 / 8", "Generate"] {
        assert!(text.contains(s), "{s}: {text}");
    }
    for s in &offline::STYLES {
        assert!(text.contains(s.label), "{}: {text}", s.label);
    }
    app.run("select.none", json!({})).unwrap();
    let text = painted_text(&mut app, show);
    assert!(text.contains("select one image or an unexpanded Image Trace object"), "{text}");
}

/// Generate at once (no thread): a preview for each ticked style, every one ticked, the document
/// unchanged; Insert Selected puts the ticked ones beside the image as one undo step.
#[test]
fn generate_previews_every_style_and_insert_adds_the_ticked_ones_as_one_step() {
    let mut app = app();
    app.concept_runs.styles = vec!["outline".into(), "flatColor".into(), "grayTones".into()];
    let (steps, roots) = (undo_steps(&app), root_count(&app));
    generate(&mut app).unwrap();
    assert_eq!((undo_steps(&app), root_count(&app)), (steps, roots), "Generate changes nothing");
    let shown = app.concept_runs.shown.as_ref().expect("a job is shown");
    let styles: Vec<&str> = shown.previews.iter().map(|p| p.style.as_str()).collect();
    assert_eq!(styles, ["outline", "flatColor", "grayTones"]);
    let painted = |p: &super::Preview| p.image.pixels.iter().any(|c| *c != Color32::WHITE);
    assert!(shown.previews.iter().all(painted), "every style paints its preview");
    assert_eq!(shown.picked.len(), 3);
    let text = painted_text(&mut app, show);
    assert!(text.contains("Insert Selected") && text.contains("Flat Color"), "{text}");

    app.concept_runs.shown.as_mut().unwrap().picked = vec!["grayTones".into(), "outline".into()];
    let v = insert(&mut app).unwrap();
    assert_eq!(v["ids"].as_array().unwrap().len(), 2);
    assert_eq!((undo_steps(&app), root_count(&app)), (steps + 1, roots + 2), "one step, two groups");
    let st = app.session.doc().unwrap();
    let names: Vec<Option<&str>> = st.selection.objects.iter().map(|id| st.doc.node(*id).and_then(|n| n.name.as_deref())).collect();
    assert_eq!(names, [Some("Outline"), Some("Gray Tones")], "in the provider's order");
    assert!(st.selection.objects.iter().all(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Group { .. }))));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(root_count(&app), roots);
}

/// At most eight styles: a ninth tick is refused by the command, and none is refused too.
#[test]
fn no_style_or_too_many_are_refused() {
    let mut app = app();
    app.concept_runs.styles.clear();
    let e = generate(&mut app).unwrap_err();
    assert!(e.contains("must list 1 to 8 different style ids"), "{e}");
    app.concept_runs.styles = (0..9).map(|_| "outline".to_string()).collect();
    assert!(generate(&mut app).is_err());
    assert!(app.concept_runs.shown.is_none());
}

/// Off the UI thread: Generate returns at once with the run going; its results are kept when it
/// ends. Cancel drops a run, and nothing is shown or changed.
#[test]
fn generate_runs_off_the_ui_thread_and_cancel_drops_it() {
    let mut app = app();
    app.concept_runs.off_the_ui_thread = true;
    let steps = undo_steps(&app);
    generate(&mut app).unwrap();
    assert!(app.concept_runs.busy());
    let text = painted_text(&mut app, show);
    assert!(text.contains("Cancel"), "{text}");
    wait(&mut app);
    assert!(!app.concept_runs.busy());
    let job = app.concept_runs.job().expect("results shown");
    assert!(app.session.concept_variations(job).is_some());

    app.concept_runs.shown = None;
    generate(&mut app).unwrap();
    cancel(&mut app);
    assert!(!app.concept_runs.busy() && app.concept_runs.shown.is_none());
    assert_eq!(undo_steps(&app), steps);
}

/// `ui.conceptVector.cancel` is the panel's Cancel: on while styles are being made, it stops them
/// and says so, and nothing is kept or shown.
#[test]
fn the_cancel_command_stops_a_run_as_the_button_does() {
    let mut app = app();
    app.concept_runs.off_the_ui_thread = true;
    assert!(!menus::enabled(&app, CANCEL));
    generate(&mut app).unwrap();
    assert!(menus::enabled(&app, CANCEL));
    assert_eq!(app.run(CANCEL, json!({})).unwrap(), json!({"canceled": 1}));
    assert!(!app.concept_runs.busy() && !menus::enabled(&app, CANCEL));
    assert_eq!(app.ui.status, vectorcraft_engine::EngineError::Cancelled.to_string());
    poll(&mut app);
    assert!(app.concept_runs.shown.is_none());
}

fn one_path(points: &[Point], fill: Option<[u8; 3]>, stroke: Option<([u8; 3], f64)>) -> Variation {
    Variation {
        style: "s".into(),
        paths: vec![VectorPath { path: PathData::single(SubPath::polyline(points, true)), fill, stroke, square_caps: false }],
        warnings: vec![],
    }
}

/// A filled path paints its color inside its outline and leaves the rest white; a stroked one
/// paints its outline and leaves the inside white.
#[test]
fn previews_paint_fills_and_strokes() {
    let square = [Point::new(25.0, 25.0), Point::new(75.0, 25.0), Point::new(75.0, 75.0), Point::new(25.0, 75.0)];
    let red = [200, 20, 20];
    let p = preview(&one_path(&square, Some(red), None), "s".into(), (100, 100));
    let [w, h] = p.image.size;
    let px = |p: &super::Preview, x: usize, y: usize| p.image.pixels[y * w + x];
    assert_eq!(px(&p, w / 2, h / 2), Color32::from_rgb(200, 20, 20));
    assert_eq!((px(&p, 2, 2), px(&p, w - 3, h - 3)), (Color32::WHITE, Color32::WHITE));
    let p = preview(&one_path(&square, None, Some((red, 4.0))), "s".into(), (100, 100));
    assert_eq!((px(&p, w / 2, h / 2), px(&p, 2, 2)), (Color32::WHITE, Color32::WHITE));
    assert_eq!(px(&p, w / 2, h / 4), Color32::from_rgb(200, 20, 20), "the top edge");
}

/// The pixels a preview paints across the top edge of a square stroked `width` wide, in an image
/// 200 by 100 pixels.
fn stroke_cover(width: f64) -> usize {
    let square = [Point::new(25.0, 25.0), Point::new(75.0, 25.0), Point::new(75.0, 75.0), Point::new(25.0, 75.0)];
    let p = preview(&one_path(&square, None, Some(([0, 0, 0], width))), "s".into(), (200, 100));
    let [w, h] = p.image.size;
    let x = w / 4;
    (0..h / 2).filter(|y| p.image.pixels[y * w + x] != Color32::WHITE).count()
}

/// Preview strokes are painted to scale with the image, and at least half a point wide.
#[test]
fn preview_strokes_keep_their_widths() {
    // The image's 200 pixels are about 408 preview pixels wide: a 10-pixel stroke is about 20.
    let (thin, thick) = (stroke_cover(1.0), stroke_cover(10.0));
    assert!((19..=23).contains(&thick), "{thick}");
    assert!((1..=4).contains(&thin), "{thin}");
    assert!(stroke_cover(0.01) >= 1, "thin lines still show");
}

/// A line with square caps is painted to the corners of its ends, and one with round caps leaves
/// them white.
#[test]
fn preview_strokes_keep_their_caps() {
    let line = PathData::single(SubPath::polyline(&[Point::new(25.0, 50.0), Point::new(75.0, 50.0)], false));
    let corner = |square_caps: bool| {
        let paths = vec![VectorPath { path: line.clone(), fill: None, stroke: Some(([0, 0, 0], 20.0)), square_caps }];
        let p = preview(&Variation { style: "s".into(), paths, warnings: vec![] }, "s".into(), (100, 100));
        let [w, h] = p.image.size;
        // Past the line's right end, near the corner of a square cap: 8 and 8 pixels of the image.
        p.image.pixels[(h * 58 / 100) * w + w * 83 / 100]
    };
    assert_eq!(corner(true), Color32::BLACK, "square caps");
    assert_eq!(corner(false), Color32::WHITE, "round caps");
}

/// Paths are painted in their order: a fill above a stroke covers it, and a stroke above a fill
/// shows.
#[test]
fn previews_paint_paths_in_their_order() {
    let square = [Point::new(25.0, 25.0), Point::new(75.0, 25.0), Point::new(75.0, 75.0), Point::new(25.0, 75.0)];
    let cover = [Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)];
    let path = |pts: &[Point]| PathData::single(SubPath::polyline(pts, true));
    let stroke = VectorPath { path: path(&square), fill: None, stroke: Some(([0, 0, 0], 4.0)), square_caps: false };
    let fill = VectorPath { path: path(&cover), fill: Some([200, 20, 20]), stroke: None, square_caps: false };
    let on_edge = |paths: Vec<VectorPath>| {
        let p = preview(&Variation { style: "s".into(), paths, warnings: vec![] }, "s".into(), (100, 100));
        let [w, h] = p.image.size;
        p.image.pixels[(h / 4) * w + w / 2]
    };
    assert_eq!(on_edge(vec![stroke.clone(), fill.clone()]), Color32::from_rgb(200, 20, 20), "the fill covers the stroke below it");
    assert_eq!(on_edge(vec![fill, stroke]), Color32::BLACK, "the stroke shows above the fill");
}

/// A path's elements, made as they are taken, are the elements of its converted path.
#[test]
fn preview_elements_match_the_converted_path() {
    let mut bp = vectorcraft_geom::BezPath::new();
    bp.move_to((0.0, 0.0));
    bp.line_to((10.0, 0.0));
    bp.curve_to((15.0, 0.0), (20.0, 5.0), (20.0, 10.0));
    bp.close_path();
    bp.move_to((30.0, 30.0));
    bp.line_to((40.0, 35.0));
    let path = PathData::from_bezpath(&bp);
    let made: Vec<_> = elements(&path).collect();
    assert_eq!(made, path.to_bezpath().elements().to_vec());
    assert_eq!(elements(&path).take(2).count(), 2);
}

/// A curve flattened into more points than are left ends the elements taken: those after it are
/// not flattened, while a closing element still closes the ring.
#[test]
fn preview_elements_stop_once_no_point_is_left() {
    let mut bp = vectorcraft_geom::BezPath::new();
    bp.move_to((0.0, 0.0));
    for i in 0..100 {
        let x = f64::from(i) * 100.0;
        bp.curve_to((x + 30.0, 400.0), (x + 70.0, -400.0), (x + 100.0, 0.0));
    }
    let path = PathData::from_bezpath(&bp);
    let left = std::cell::Cell::new(10);
    let taken = std::cell::Cell::new(0);
    vectorcraft_geom::flatten(within_budget(elements(&path).inspect(|_| taken.set(taken.get() + 1)), &left), 0.25, |el| {
        if matches!(el, vectorcraft_geom::PathEl::LineTo(_)) && left.get() > 0 {
            left.set(left.get() - 1);
        }
    });
    assert_eq!(left.get(), 0);
    assert!(taken.get() <= 3, "{} elements taken", taken.get());
    let closing = [vectorcraft_geom::PathEl::LineTo((1.0, 1.0).into()), vectorcraft_geom::PathEl::ClosePath];
    assert_eq!(within_budget(closing.into_iter(), &left).count(), 0, "nothing is taken once none is left");
    assert_eq!(within_budget(closing[1..].iter().copied(), &left).count(), 1, "but a ring still closes");
}

/// A preview of an image wider or taller than its box allows keeps the image's proportions, with
/// margins, instead of stretching the image to the box.
#[test]
fn a_preview_of_a_wide_or_tall_image_keeps_its_proportions() {
    let square = [Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)];
    for (size, aspect) in [((1000, 100), 10.0f32), ((100, 1000), 0.1)] {
        let p = preview(&one_path(&square, Some([9, 9, 9]), None), "s".into(), size);
        let [w, h] = p.image.size;
        assert!(((w as f32 / h as f32) / aspect - 1.0).abs() < 0.06, "{size:?}: {w} x {h}");
        let inner = Rect::from_min_size(Pos2::ZERO, vec2(preview_width(aspect, PREVIEW_H), PREVIEW_H)).shrink(4.0);
        let art = art_rect(inner, aspect);
        assert!(inner.expand(0.01).contains_rect(art), "{art:?} in {inner:?}");
        assert!((art.aspect_ratio() / aspect - 1.0).abs() < 1e-3, "{art:?}");
        assert!((art.center() - inner.center()).length() < 1e-3, "{art:?}");
    }
}

/// One path with more points than a preview draws is cut off within the path: its points in the
/// top quarter are painted, and those past the limit, in the bottom quarter, are not.
#[test]
fn a_preview_stops_at_its_point_limit_within_one_path() {
    let zigzag: Vec<Point> =
        (0..PREVIEW_POINTS * 3).map(|i| Point::new((i % 100) as f64, if i < PREVIEW_POINTS { 10.0 } else { 90.0 } + (i % 2) as f64 * 0.5)).collect();
    let mut path = one_path(&zigzag, None, Some(([0, 0, 0], 1.0)));
    if let Some(sp) = path.paths[0].path.subpaths.first_mut() {
        sp.closed = false;
    }
    let p = preview(&path, "s".into(), (100, 100));
    let [w, h] = p.image.size;
    let painted = |rows: std::ops::Range<usize>| rows.flat_map(|y| (0..w).map(move |x| y * w + x)).any(|i| p.image.pixels[i] != Color32::WHITE);
    assert!(painted(0..h / 4), "the points before the limit");
    assert!(!painted(h * 3 / 4..h), "the points past it");
}

/// As on the web (no thread): Generate returns with the run going and nothing made; each frame
/// makes one part (the image decoded, a style's color layers found, one layer traced), the styles
/// are made in order, and the last part's frame shows them all.
#[test]
fn on_the_web_generate_makes_one_part_a_frame() {
    let mut app = app();
    app.concept_runs.by_frames = true;
    app.concept_runs.styles = vec!["outline".into(), "flatColor".into(), "grayTones".into()];
    let (steps, roots) = (undo_steps(&app), root_count(&app));
    generate(&mut app).unwrap();
    assert!(app.concept_runs.busy() && app.concept_runs.shown.is_none());
    assert_eq!(made_so_far(&app), Some(0), "made one part a frame, not on a thread");
    let text = painted_text(&mut app, show);
    assert!(text.contains("Cancel"), "{text}");
    let mut seen = vec![];
    while let Some(k) = made_so_far(&app) {
        assert!(seen.len() < 1000, "the parts never end");
        seen.push(k);
        assert!(app.concept_runs.shown.is_none());
        poll(&mut app);
    }
    assert!(seen.windows(2).all(|w| w[0] <= w[1]) && seen.contains(&1) && seen.contains(&2), "{seen:?}");
    assert!(seen.len() > 3, "more than one part a style: {seen:?}");
    assert!(!app.concept_runs.busy());
    let shown = app.concept_runs.shown.as_ref().expect("results shown");
    let styles: Vec<&str> = shown.previews.iter().map(|p| p.style.as_str()).collect();
    assert_eq!(styles, ["outline", "flatColor", "grayTones"]);
    assert!(app.session.concept_variations(shown.job).is_some());
    assert_eq!((undo_steps(&app), root_count(&app)), (steps, roots), "Generate changes nothing");
}

/// On the web, Cancel takes effect inside a style: after the image's color layers are found and
/// one is traced, the run is dropped, and nothing is shown or changed.
#[test]
fn on_the_web_cancel_stops_a_style_part_way() {
    let mut app = app();
    app.concept_runs.by_frames = true;
    app.concept_runs.styles = vec!["flatColor".into()];
    let (steps, roots) = (undo_steps(&app), root_count(&app));
    generate(&mut app).unwrap();
    // The image decoded, its color layers found, and the first layer traced.
    for _ in 0..3 {
        poll(&mut app);
        assert_eq!(made_so_far(&app), Some(0), "the one style is still being made");
    }
    cancel(&mut app);
    poll(&mut app);
    assert!(!app.concept_runs.busy() && app.concept_runs.shown.is_none());
    assert_eq!((undo_steps(&app), root_count(&app)), (steps, roots));
}

/// A provider with one style, `sketch`, named "Pencil Sketch": a square for each style asked.
struct Sketches;

impl Provider for Sketches {
    fn id(&self) -> &'static str {
        "test.sketches"
    }
    fn name(&self) -> &'static str {
        "Sketches"
    }
    fn capabilities(&self) -> Capabilities {
        let styles = vec![Style { id: "sketch", label: "Pencil Sketch" }];
        Capabilities { image_input: true, vector_output: true, prompt: false, raster_output: false, offline: true, styles }
    }
    fn generate(&self, r: &Request, _: Steps) -> Result<Vec<Variation>, ProviderError> {
        let sq = [Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)];
        Ok(r.styles.iter().map(|s| Variation { style: s.clone(), ..one_path(&sq, Some([0; 3]), None) }).collect())
    }
}

/// The results of another provider are named as that provider names its styles.
#[test]
fn results_are_named_as_their_provider_names_its_styles() {
    let mut app = app();
    app.session.concept_providers_mut().register(Arc::new(Sketches)).unwrap();
    let v = app.run("conceptVector.generate", json!({"provider": "test.sketches", "styles": ["sketch"]})).unwrap();
    show_job(&mut app, &v);
    let shown = app.concept_runs.shown.as_ref().expect("a job is shown");
    let labels: Vec<&str> = shown.previews.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, ["Pencil Sketch"]);
    let text = painted_text(&mut app, show);
    assert!(text.contains("Pencil Sketch"), "{text}");
}

/// Insert Selected after the image moved: the job no longer applies, and nothing changes.
#[test]
fn insert_after_the_image_moved_changes_nothing() {
    let mut app = app();
    generate(&mut app).unwrap();
    app.run("object.transform", json!({"matrix": [1, 0, 0, 1, 5, 0]})).unwrap();
    let (steps, roots) = (undo_steps(&app), root_count(&app));
    let e = insert(&mut app).unwrap_err();
    assert!(e.contains("no longer applies"), "{e}");
    assert_eq!((undo_steps(&app), root_count(&app)), (steps, roots));
}
