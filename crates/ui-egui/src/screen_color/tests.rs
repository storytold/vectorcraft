use craft::color_ui::ui::StillCapture;
use craft::platform::screen::ScreenError;
use vectorcraft_color::cms::{ColorSettings, DISPLAY_P3};
use vectorcraft_engine::Session;

use super::*;

/// An app with a document and a selected rectangle, and the window the pick runs in.
fn app() -> (VectorcraftApp, egui::Context) {
    let ctx = crate::testing::ui_context();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.ui_ctx = Some(ctx.clone());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    (app, ctx)
}

/// The capture ends in `result`, and the pass takes it.
fn finish(app: &mut VectorcraftApp, ctx: &egui::Context, result: Result<Srgba, ScreenError>) {
    for _ in 0..2 {
        let result = result.clone();
        app.screen_pick.tick_with(ctx, target(ctx), &move || {
            Box::new(StillCapture::new(result.clone().map(|c| craft::platform::screen::Capture::Color([c.r, c.g, c.b]))))
        });
    }
    settle(app, ctx);
}

fn srgba(hex: &str) -> Srgba {
    Srgba::from_hex(hex).unwrap()
}

fn paints(app: &VectorcraftApp) -> (String, String) {
    let (f, s) = crate::panels::current_paints(app);
    (f.color().map_or("none".into(), |c| c.to_hex()), s.color().map_or("none".into(), |c| c.to_hex()))
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// The color picked on the screen becomes the fill, or with `stroke` the stroke, as one undo step.
#[test]
fn a_sampled_color_fills_the_selection() {
    let (mut app, ctx) = app();
    let (_, stroke) = paints(&app);
    let before = undo_len(&app);
    assert_eq!(app.run("ui.sampleScreenColor", json!({})).unwrap(), Value::Null, "answers at once");
    assert_eq!(undo_len(&app), before, "nothing changes until the person picks");
    finish(&mut app, &ctx, Ok(srgba("#123456")));
    assert_eq!(paints(&app), ("#123456".into(), stroke));
    assert_eq!(undo_len(&app), before + 1, "one step");
    app.run("edit.undo", json!({})).unwrap();
    assert_ne!(paints(&app).0, "#123456");
    let (fill, _) = paints(&app);
    app.run("ui.sampleScreenColor", json!({"stroke": true})).unwrap();
    finish(&mut app, &ctx, Ok(srgba("#336699")));
    assert_eq!(paints(&app), (fill, "#336699".into()));
}

/// With `picker`, the color goes to the open Color Picker only, which shows it on the selection.
#[test]
fn the_color_picker_takes_the_sampled_color() {
    let (mut app, ctx) = app();
    let before = paints(&app);
    assert!(app.run("ui.sampleScreenColor", json!({"picker": true})).is_err(), "no Color Picker open");
    assert!(!app.screen_pick.busy());
    app.run("ui.colorPicker", json!({})).unwrap();
    app.run("ui.sampleScreenColor", json!({"picker": true})).unwrap();
    finish(&mut app, &ctx, Ok(srgba("#123456")));
    assert_eq!(paints(&app), before, "the fill takes nothing");
    let mut out = ctx.run_ui(Default::default(), |ui| crate::dialogs::color_picker::pump(&mut app, ui.ctx()));
    out.textures_delta.clear();
    assert_eq!(paints(&app).0, "#123456", "previewed on the selection");
}

/// The pipette's press runs the pick for the picker, and the color it answers previews in it.
#[test]
fn the_color_pickers_eyedropper_samples_into_it() {
    let (mut app, ctx) = app();
    app.run("ui.colorPicker", json!({})).unwrap();
    let id = app.color_dialogs.pickers.last().unwrap().id;
    let owner = crate::dialogs::color_picker::owner(id);
    assert_eq!(hold(&mut app, &ctx, owner, ScreenPickGesture::Clicked), None);
    assert!(app.screen_pick.busy());
    finish(&mut app, &ctx, Ok(srgba("#123456")));
    let mut out = ctx.run_ui(Default::default(), |ui| crate::dialogs::color_picker::pump(&mut app, ui.ctx()));
    out.textures_delta.clear();
    assert_eq!(paints(&app).0, "#123456");
}

/// A canceled capture changes nothing; a failed one reaches the status line.
#[test]
fn cancel_and_errors_leave_the_art_alone() {
    let (mut app, ctx) = app();
    let before = (paints(&app), undo_len(&app));
    app.run("ui.sampleScreenColor", json!({})).unwrap();
    finish(&mut app, &ctx, Err(ScreenError::Cancelled));
    assert_eq!((paints(&app), undo_len(&app)), before);
    assert!(!app.screen_pick.busy());
    app.run("ui.sampleScreenColor", json!({})).unwrap();
    finish(&mut app, &ctx, Err(ScreenError::Failed("the sampler failed".into())));
    assert!(app.ui.status.contains("the sampler failed"), "{}", app.ui.status);
    assert_eq!((paints(&app), undo_len(&app)), before);
}

/// A color that arrives after another document became active paints nothing.
#[test]
fn a_new_active_document_takes_no_color() {
    let (mut app, ctx) = app();
    app.run("ui.sampleScreenColor", json!({})).unwrap();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let before = paints(&app);
    finish(&mut app, &ctx, Ok(srgba("#123456")));
    assert_eq!(paints(&app), before);
    assert_eq!(app.ui.status, "The active document changed while the screen color sampler was open");
}

/// The sampler answers display sRGB: with Display P3 as the working space, the color it becomes
/// still shows as the sampled #123456.
#[test]
fn a_wide_working_space_shows_the_sampled_color() {
    let p3 = Cms::new(&ColorSettings { rgb: DISPLAY_P3.into(), ..Default::default() }).unwrap();
    let srgb = [0x12, 0x34, 0x56].map(|v| v as f32 / 255.0);
    let Color::Rgb { r, g, b } = sampled(&p3, srgb) else { panic!("not RGB") };
    let shown = p3.rgb_to_srgb([r, g, b]);
    for (s, v) in shown.iter().zip(srgb) {
        assert!((s - v).abs() < 0.5 / 255.0, "{shown:?} shows as {srgb:?}");
    }
    assert!([r, g, b].iter().zip(srgb).any(|(w, v)| (w - v).abs() > 1.0 / 255.0), "Display P3 numbers differ from sRGB's");
    // sRGB keeps the numbers; values out of range or not numbers are clamped.
    let srgb_cms = Cms::new(&ColorSettings::default()).unwrap();
    assert_eq!(sampled(&srgb_cms, srgb), Color::rgb(srgb[0], srgb[1], srgb[2]));
    assert_eq!(sampled(&srgb_cms, [f32::NAN, 2.0, -1.0]), Color::rgb(0.0, 1.0, 0.0));
}

/// The command answers every junk parameter without a panic.
#[test]
fn junk_params_never_panic() {
    let (mut app, ctx) = app();
    let doc = crate::menus::UI_COMMANDS.iter().find(|c| c.0 == "ui.sampleScreenColor").unwrap().3;
    let mut params = vectorcraft_testkit::strategies::junk_params(doc, "/nonexistent");
    params.extend(vectorcraft_testkit::strategies::junk_values());
    assert!(params.len() > 40);
    for p in params {
        let _ = app.run("ui.sampleScreenColor", p);
        if app.screen_pick.busy() {
            finish(&mut app, &ctx, Ok(srgba("#808080")));
        }
    }
    assert!(!app.screen_pick.busy());
}
