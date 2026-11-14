//! Raster export keeps the resolution accepted in the desktop options dialog.
// Integration tests may unwrap and panic (AGENTS.md, Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::{Services, VectorcraftApp, dialogs};

type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

fn app() -> (VectorcraftApp, Written) {
    let written = Written::default();
    let output = written.clone();
    let services = Services {
        pick_save: Some(Box::new(|pick| Some(format!("/out/{}", pick.name)))),
        write: Some(Box::new(move |path, bytes| {
            output.borrow_mut().push((path.to_string(), bytes.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 60, "height": 40})).unwrap();
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    app.run("shape.ellipse", json!({"x": 5, "y": 4, "width": 30, "height": 20})).unwrap();
    (app, written)
}

/// Start with the ordinary Export As dialog, then accept its format/artboard choice and file pick.
fn open_png_options(app: &mut VectorcraftApp) {
    app.run("file.exportAs", json!({})).unwrap();
    let dialog = app.ui.dialog.as_mut().unwrap();
    assert_eq!(dialog.kind, "exportAs");
    dialog.fields.insert("useArtboards".into(), json!(true));
    dialogs::confirm(app).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().kind, "pngOptions");
}

#[test]
fn accepted_raster_resolution_is_used_when_export_as_reopens() {
    let (mut app, written) = app();
    open_png_options(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["ppi"].as_f64(), Some(72.0), "a fresh app keeps the existing default");
    app.ui.dialog.as_mut().unwrap().fields.insert("ppi".into(), json!(600));
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(written.borrow().len(), 1, "the accepted export reaches the injected writer");
    open_png_options(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["ppi"].as_f64(), Some(600.0), "the next export remembers the accepted resolution");
}

fn accepted_600_ppi() -> VectorcraftApp {
    let (mut app, _) = app();
    open_png_options(&mut app);
    app.ui.dialog.as_mut().unwrap().fields.insert("ppi".into(), json!(600));
    dialogs::confirm(&mut app).unwrap();
    app
}

#[test]
fn accepted_raster_resolution_survives_ui_preferences_restore() {
    let original = accepted_600_ppi();
    // The desktop host serializes UiState, then reads and sanitizes it on a new launch.
    // Keep this in memory: no user preferences or files are read or written.
    let saved = serde_json::to_vec(&original.ui).unwrap();
    let ui = vectorcraft_ui_egui::UiState::read_preferences(&saved).unwrap().ui.sanitized();
    let (mut restored, _) = app();
    restored.ui = ui;
    vectorcraft_ui_egui::prefs_dialog::restore(&mut restored);
    open_png_options(&mut restored);
    assert_eq!(restored.ui.dialog.as_ref().unwrap().fields["ppi"].as_f64(), Some(600.0));
}

#[test]
fn canceling_raster_options_keeps_the_last_accepted_resolution() {
    let mut app = accepted_600_ppi();
    open_png_options(&mut app);
    app.ui.dialog.as_mut().unwrap().fields.insert("ppi".into(), json!(300));
    dialogs::cancel(&mut app);
    assert!(app.ui.dialog.is_none());
    open_png_options(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["ppi"].as_f64(), Some(600.0), "Cancel does not remember an unaccepted choice");
}

#[test]
fn explicit_png_resolution_takes_precedence_over_remembered_resolution() {
    let mut app = accepted_600_ppi();
    app.run("file.export.png", json!({"ppi": 300})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().kind, "pngOptions");
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["ppi"].as_f64(), Some(300.0), "the caller's explicit resolution wins over the remembered 600");
    dialogs::cancel(&mut app);
    open_png_options(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["ppi"].as_f64(), Some(600.0), "canceling the explicit choice leaves memory alone");
}

#[test]
fn raster_formats_share_the_last_accepted_resolution() {
    let mut app = accepted_600_ppi();
    app.run("file.exportAs", json!({"format": "jpg"})).unwrap();
    let dialog = app.ui.dialog.as_mut().unwrap();
    assert_eq!(dialog.kind, "exportAs");
    dialog.fields.insert("useArtboards".into(), json!(true));
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().kind, "jpgOptions");
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["ppi"].as_f64(), Some(600.0), "JPEG uses the accepted PNG resolution");
    app.ui.dialog.as_mut().unwrap().fields.insert("ppi".into(), json!(300));
    dialogs::confirm(&mut app).unwrap();
    open_png_options(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["ppi"].as_f64(), Some(300.0), "PNG uses the subsequently accepted JPEG resolution");
}

fn painted_texts(output: &egui::FullOutput) -> Vec<String> {
    fn collect(shape: &egui::Shape, texts: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => texts.push(text.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, texts)),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    output.shapes.iter().for_each(|shape| collect(&shape.shape, &mut texts));
    texts
}

fn dialog_texts(app: &mut VectorcraftApp) -> Vec<String> {
    let context = egui::Context::default();
    vectorcraft_ui_egui::theme::install_fonts(&context);
    let mut texts = Vec::new();
    for _ in 0..3 {
        let mut output = context.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() },
            |ui| dialogs::show(app, ui.ctx()),
        );
        texts = painted_texts(&output);
        output.textures_delta.clear();
    }
    texts
}

#[test]
fn remembered_resolution_keeps_live_size_png_metadata_and_transparency() {
    let (mut app, written) = app();
    open_png_options(&mut app);
    app.ui.dialog.as_mut().unwrap().fields.insert("ppi".into(), json!(600));
    dialogs::confirm(&mut app).unwrap();
    open_png_options(&mut app);
    let texts = dialog_texts(&mut app);
    assert!(texts.iter().any(|text| text == "500 × 333 px"), "the actual Size row follows the remembered resolution: {texts:?}");
    dialogs::confirm(&mut app).unwrap();
    let output = written.borrow();
    assert_eq!(output.len(), 2);
    let png = &output[1].1;
    let image = image::load_from_memory(png).unwrap().to_rgba8();
    assert_eq!(image.dimensions(), (500, 333), "60 × 40 pt at 600 ppi");
    assert_eq!(image.get_pixel(0, 0)[3], 0, "the unchanged transparent background is preserved");
    assert!(image.pixels().any(|pixel| pixel[3] != 0), "the original ellipse is present");
    let phys = png.windows(4).position(|chunk| chunk == b"pHYs").expect("PNG resolution metadata");
    let pixels_per_meter = (600.0_f64 / 0.0254).round() as u32;
    assert_eq!(&png[phys + 4..phys + 8], pixels_per_meter.to_be_bytes());
    assert_eq!(&png[phys + 8..phys + 12], pixels_per_meter.to_be_bytes());
    assert_eq!(png[phys + 12], 1, "pHYs stores pixels per meter");
}
