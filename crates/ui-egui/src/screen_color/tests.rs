use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{Sender, channel};

use vectorcraft_color::cms::{ColorSettings, DISPLAY_P3};
use vectorcraft_engine::Session;

use super::*;

/// The senders of the samplers a stand-in host showed, newest last.
type Shown = Rc<RefCell<Vec<Sender<Answer>>>>;

/// An app with a document and a selected rectangle, whose host has a stand-in sampler.
fn app() -> (VectorcraftApp, Shown) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    let shown = Shown::default();
    let host = shown.clone();
    app.services.screen_color = Some(Box::new(move || {
        let (tx, rx) = channel();
        host.borrow_mut().push(tx);
        Ok(rx)
    }));
    (app, shown)
}

/// The person picks `hex` (or cancels, `None`) in the last sampler shown; the next frame takes it.
fn pick(app: &mut VectorcraftApp, shown: &Shown, hex: Option<&str>) {
    let srgb = hex.map(|h| Color::from_hex(h).unwrap().to_rgb());
    shown.borrow().last().unwrap().send(Ok(srgb)).unwrap();
    poll(app, &egui::Context::default());
}

fn paints(app: &VectorcraftApp) -> (String, String) {
    let (f, s) = crate::panels::current_paints(app);
    (f.color().map_or("none".into(), |c| c.to_hex()), s.color().map_or("none".into(), |c| c.to_hex()))
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// The color picked on the screen becomes the fill, as one undo step.
#[test]
fn a_sampled_color_fills_the_selection() {
    let (mut app, shown) = app();
    let (_, stroke) = paints(&app);
    let before = undo_len(&app);
    assert_eq!(app.run("ui.sampleScreenColor", json!({})).unwrap(), Value::Null, "answers at once");
    assert_eq!(undo_len(&app), before, "nothing changes until the person picks");
    pick(&mut app, &shown, Some("#123456"));
    assert_eq!(paints(&app), ("#123456".into(), stroke));
    assert_eq!(undo_len(&app), before + 1, "one step");
    app.run("edit.undo", json!({})).unwrap();
    assert_ne!(paints(&app).0, "#123456");
}

/// `stroke: true`, or the stroke proxy in front, sets the stroke instead.
#[test]
fn stroke_takes_the_stroke() {
    let (mut app, shown) = app();
    let (fill, _) = paints(&app);
    app.run("ui.sampleScreenColor", json!({"stroke": true})).unwrap();
    pick(&mut app, &shown, Some("#336699"));
    assert_eq!(paints(&app), (fill.clone(), "#336699".into()));
    app.session.fill_active = false;
    app.run("ui.sampleScreenColor", json!({})).unwrap();
    pick(&mut app, &shown, Some("#996633"));
    assert_eq!(paints(&app), (fill, "#996633".into()));
    app.run("ui.sampleScreenColor", json!({"stroke": false})).unwrap();
    pick(&mut app, &shown, Some("#669933"));
    assert_eq!(paints(&app).0, "#669933");
}

/// With `picker`, the color goes to the open Color Picker, which shows it on the selection as any
/// pick; Cancel puts the art back.
#[test]
fn the_color_picker_takes_the_sampled_color() {
    let (mut app, shown) = app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |app: &mut VectorcraftApp| {
        let mut out = ctx.run_ui(Default::default(), |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
    };
    let before = paints(&app);
    assert!(app.run("ui.sampleScreenColor", json!({"picker": true})).is_err(), "no Color Picker open");
    assert!(app.screen_sampler.is_none());
    app.run("ui.colorPicker", json!({})).unwrap();
    frame(&mut app);
    app.run("ui.sampleScreenColor", json!({"picker": true})).unwrap();
    pick(&mut app, &shown, Some("#123456"));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.fields.get("color").and_then(vectorcraft_engine::cmd::color_value).map(|c| c.to_hex()).as_deref(), Some("#123456"));
    frame(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("hex"), "123456");
    assert_eq!(paints(&app).0, "#123456", "previewed on the selection");
    crate::dialogs::cancel(&mut app);
    assert_eq!(paints(&app), before);
    // A Color Picker closed before the answer arrives takes nothing, and the art stays as it is.
    app.run("ui.colorPicker", json!({})).unwrap();
    app.run("ui.sampleScreenColor", json!({"picker": true})).unwrap();
    crate::dialogs::cancel(&mut app);
    pick(&mut app, &shown, Some("#654321"));
    assert_eq!(paints(&app), before);
    assert!(app.ui.dialog.is_none());
    // Nor does a new Color Picker for the same proxy and color that replaced it.
    app.run("ui.colorPicker", json!({})).unwrap();
    app.run("ui.sampleScreenColor", json!({"picker": true})).unwrap();
    crate::dialogs::cancel(&mut app);
    app.run("ui.colorPicker", json!({})).unwrap();
    pick(&mut app, &shown, Some("#654321"));
    frame(&mut app);
    assert_eq!(paints(&app), before);
    assert_ne!(app.ui.dialog.as_ref().unwrap().str("hex"), "654321");
    crate::dialogs::cancel(&mut app);
    // Nor does a Color Picker for the other proxy that replaced it.
    app.run("ui.colorPicker", json!({})).unwrap();
    app.run("ui.sampleScreenColor", json!({"picker": true})).unwrap();
    crate::dialogs::cancel(&mut app);
    app.run("ui.colorPicker", json!({"stroke": true})).unwrap();
    pick(&mut app, &shown, Some("#654321"));
    frame(&mut app);
    assert_eq!(paints(&app), before);
    assert_ne!(app.ui.dialog.as_ref().unwrap().str("hex"), "654321");
    crate::dialogs::cancel(&mut app);
    // Nor one in another document: the status bar says why.
    app.run("ui.colorPicker", json!({})).unwrap();
    app.run("ui.sampleScreenColor", json!({"picker": true})).unwrap();
    crate::dialogs::cancel(&mut app);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    app.run("ui.colorPicker", json!({})).unwrap();
    pick(&mut app, &shown, Some("#654321"));
    assert_ne!(app.ui.dialog.as_ref().unwrap().fields.get("color"), Some(&crate::panels::color_json(&Color::from_hex("#654321").unwrap())));
    assert_eq!(app.ui.status, "The active document changed while the screen color sampler was open");
}

/// The Color Picker's eyedropper button shows the sampler for the Color Picker, and the color
/// the frame takes goes there.
#[test]
fn the_color_pickers_eyedropper_samples_into_it() {
    let (mut app, shown) = app();
    app.run("ui.colorPicker", json!({})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| {
            app.logic(ui.ctx());
            crate::dialogs::show(app, ui.ctx());
        });
        out.textures_delta.clear();
    };
    // The dialog's one square clickable widget of the button's size, once the window has settled.
    let buttons = || -> Vec<egui::Rect> {
        ctx.viewport(|v| {
            let widgets = v.this_pass.widgets.layers().flat_map(|(_, w)| w.iter());
            widgets.filter(|w| w.sense.senses_click() && w.rect.size() == egui::Vec2::splat(24.0)).map(|w| w.rect).collect()
        })
    };
    let mut last = vec![];
    for _ in 0..10 {
        frame(&mut app, vec![]);
        if !last.is_empty() && buttons() == last {
            break;
        }
        last = buttons();
    }
    let [button] = last[..] else { panic!("{last:?}") };
    let pos = button.center();
    let press = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, vec![egui::Event::PointerMoved(pos), press(true), press(false)]);
    assert_eq!(shown.borrow().len(), 1, "the sampler is shown");
    assert!(matches!(app.screen_sampler.as_ref().map(|w| &w.target), Some(Target::Picker(_))));
    assert_eq!(app.screen_sampler.as_ref().map(|w| &w.target), open_picker(&app).as_ref());
    shown.borrow().last().unwrap().send(Ok(Some([0x12, 0x34, 0x56].map(|v| v as f32 / 255.0)))).unwrap();
    frame(&mut app, vec![]);
    assert!(app.screen_sampler.is_none());
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("hex"), "123456");
}

/// `ui.inspect` reports `screenSampler` while the sampler waits; one at a time.
#[test]
fn inspect_reports_the_open_sampler() {
    let (mut app, shown) = app();
    let ctx = egui::Context::default();
    let open = |app: &VectorcraftApp| crate::control::inspect(app, &ctx)["screenSampler"].clone();
    assert_eq!(open(&app), json!(false));
    app.run("ui.sampleScreenColor", json!({})).unwrap();
    assert_eq!(open(&app), json!(true));
    poll(&mut app, &ctx);
    assert_eq!(open(&app), json!(true), "still waiting");
    let e = app.run("ui.sampleScreenColor", json!({})).unwrap_err();
    assert!(e.contains("already open"), "{e}");
    assert_eq!(shown.borrow().len(), 1);
    pick(&mut app, &shown, None);
    assert_eq!(open(&app), json!(false));
}

/// Escape (no color) changes nothing; an error from the host reaches the status bar; a sampler
/// that is gone without answering counts as canceled.
#[test]
fn cancel_and_errors_leave_the_art_alone() {
    let (mut app, shown) = app();
    let before = (paints(&app), undo_len(&app));
    app.run("ui.sampleScreenColor", json!({})).unwrap();
    pick(&mut app, &shown, None);
    assert_eq!((paints(&app), undo_len(&app)), before);
    app.run("ui.sampleScreenColor", json!({})).unwrap();
    shown.borrow().last().unwrap().send(Err("the sampler failed".into())).unwrap();
    poll(&mut app, &egui::Context::default());
    assert_eq!(app.ui.status, "the sampler failed");
    app.run("ui.sampleScreenColor", json!({})).unwrap();
    shown.borrow_mut().clear();
    poll(&mut app, &egui::Context::default());
    assert!(app.screen_sampler.is_none());
    assert_eq!((paints(&app), undo_len(&app)), before);
    // A sampler that can't be shown answers with its error and waits for nothing.
    app.services.screen_color = Some(Box::new(|| Err("no screen".into())));
    assert_eq!(app.run("ui.sampleScreenColor", json!({})).unwrap_err(), "no screen");
    assert!(app.screen_sampler.is_none());
}

/// A color that arrives after another document became active paints nothing.
#[test]
fn a_new_active_document_takes_no_color() {
    let (mut app, shown) = app();
    app.run("ui.sampleScreenColor", json!({})).unwrap();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let before = paints(&app);
    pick(&mut app, &shown, Some("#123456"));
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

/// The browser's EyeDropper answers `sRGBHex`; anything else isn't a color.
#[test]
fn hex_answers_read_as_srgb() {
    assert_eq!(srgb_from_hex("#123456"), Some([0x12, 0x34, 0x56].map(|v| v as f32 / 255.0)));
    assert_eq!(srgb_from_hex("#FFFFFF"), Some([1.0; 3]));
    for junk in ["", "#", "#12345", "#1234567", "#12345g", "+12345", "rgb(1, 2, 3)", "red"] {
        assert_eq!(srgb_from_hex(junk), None, "{junk}");
    }
}

/// Without a sampler (Windows, Linux, a browser without an EyeDropper) the command answers an
/// error and is disabled, and the Color Picker draws its button gray.
#[test]
fn without_a_sampler_the_command_is_unavailable() {
    let (mut app, _) = app();
    assert!(crate::menus::enabled(&app, "ui.sampleScreenColor"));
    app.services.screen_color = None;
    assert!(!crate::menus::enabled(&app, "ui.sampleScreenColor"));
    assert_eq!(app.run("ui.sampleScreenColor", json!({})).unwrap_err(), "this system has no screen color sampler");
    assert!(app.screen_sampler.is_none());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut clicked = true;
    let mut out = ctx.run_ui(Default::default(), |ui| clicked = button(&app, ui));
    out.textures_delta.clear();
    assert!(!clicked);
    // Keyboard Shortcuts lists the command, so it can be given a key.
    assert!(crate::shortcut_editor::entries().iter().any(|e| e.key == "ui.sampleScreenColor"));
}

/// The command answers every junk parameter without a panic.
#[test]
fn junk_params_never_panic() {
    let (mut app, shown) = app();
    let doc = crate::menus::UI_COMMANDS.iter().find(|c| c.0 == "ui.sampleScreenColor").unwrap().3;
    let mut params = vectorcraft_testkit::strategies::junk_params(doc, "/nonexistent");
    params.extend(vectorcraft_testkit::strategies::junk_values());
    assert!(params.len() > 40);
    for p in params {
        let _ = app.run("ui.sampleScreenColor", p);
        if !shown.borrow().is_empty() {
            pick(&mut app, &shown, Some("#808080"));
            shown.borrow_mut().clear();
        }
    }
    assert!(app.screen_sampler.is_none());
}
