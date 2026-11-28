use serde_json::json;
use vectorcraft_engine::cmd::fileio;

use super::png_options;
use super::tests_export::{answer_saves, app, cancel, confirm, frame, kind, set, try_confirm};
use super::tests_raster_formats::fields;

#[test]
fn raster_options_preserve_supplied_values_through_the_png_command() {
    assert_fresh_raster_defaults_without_remembered_resolution();
    let (mut app, written) = app(1);
    app.run("file.export.png", json!({"ppi": 300, "background": "#204080", "antiAlias": "type", "interlaced": true})).unwrap();
    answer_saves(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("pngOptions"));
    let f = fields(&app);
    assert_eq!(f.number("ppi"), Some(300.0));
    assert_eq!(f.text("background"), Some("#204080"));
    assert_eq!(f.text("antiAlias"), Some("type"));
    assert_eq!(f.bool("interlaced"), Some(true));
    frame(&mut app);
    confirm(&mut app);
    {
        let output = written.borrow();
        assert_eq!(output.len(), 1);
        let png = &output[0].1;
        assert_eq!(png[28], 1);
        let physical = png.windows(4).position(|w| w == b"pHYs").unwrap();
        assert_eq!(&png[physical + 4..physical + 8], ((300.0f64 / 0.0254).round() as u32).to_be_bytes());
        let decoded = craft_image_codecs::decode(png).unwrap();
        assert_eq!(decoded.dimensions(), (250, 167));
        assert_eq!(&decoded.to_rgba8()[..4], &[0x20, 0x40, 0x80, 255]);
    }

    png_options::open(&mut app, fileio::format("jpg").unwrap(), json!({"format": "jpg", "path": "/out/supplied.jpg", "ppi": 144, "quality": 40}));
    let f = fields(&app);
    assert_eq!(f.number("quality"), Some(40.0));
    assert_eq!(f.number("ppi"), Some(144.0));
    assert_eq!(f.text("background"), Some("white"));
    frame(&mut app);
    confirm(&mut app);
    assert_eq!(written.borrow().len(), 2);

    // Opening again replaces the open dialog of the same kind.
    for omitted in [json!({}), json!({"ppi": null, "background": null, "antiAlias": null, "interlaced": null})] {
        app.run("file.export.png", omitted).unwrap();
        answer_saves(&mut app);
        let f = fields(&app);
        assert_eq!(f.number("ppi"), Some(144.0), "the last accepted raster export was JPEG at 144 PPI");
        assert_eq!(f.text("background"), Some("transparent"));
        assert_eq!(f.text("antiAlias"), Some("art"));
        assert_eq!(f.bool("interlaced"), Some(false));
    }

    app.run("file.export.png", json!({"ppi": 0})).unwrap();
    answer_saves(&mut app);
    assert_eq!(fields(&app).number("ppi"), Some(0.0));
    assert!(try_confirm(&mut app).unwrap_err().contains("ppi must be a positive number"));
    assert_eq!(written.borrow().len(), 2);
}

fn assert_fresh_raster_defaults_without_remembered_resolution() {
    let (mut app, _) = app(1);
    for omitted in [json!({}), json!({"ppi": null, "background": null, "antiAlias": null, "interlaced": null})] {
        app.run("file.export.png", omitted).unwrap();
        answer_saves(&mut app);
        let f = fields(&app);
        assert_eq!(f.number("ppi"), Some(72.0), "a fresh app keeps the documented default");
        assert_eq!(f.text("background"), Some("transparent"));
        assert_eq!(f.text("antiAlias"), Some("art"));
        assert_eq!(f.bool("interlaced"), Some(false));
        cancel(&mut app);
    }
}

/// Start with Export As, then accept its format/artboard choice and the native file pick.
fn open_png_options(app: &mut crate::VectorcraftApp) {
    app.run("file.exportAs", json!({})).unwrap();
    assert_eq!(kind(app).as_deref(), Some("exportAs"));
    set(app, "useArtboards", json!(true));
    confirm(app);
    answer_saves(app);
    assert_eq!(kind(app).as_deref(), Some("pngOptions"));
}

fn raster_document_state(app: &crate::VectorcraftApp) -> serde_json::Value {
    let st = app.session.active().unwrap();
    json!({"document": st.doc, "selection": st.selection, "undo": st.history.undo.len(), "redo": st.history.redo.len()})
}

#[test]
fn accepted_raster_resolution_is_used_when_export_as_reopens() {
    let (mut app, written) = app(1);
    let before = raster_document_state(&app);
    open_png_options(&mut app);
    assert_eq!(fields(&app).number("ppi"), Some(72.0), "a fresh app keeps the existing default");
    set(&mut app, "ppi", json!(600));
    confirm(&mut app);
    assert!(kind(&app).is_none());
    assert_eq!(written.borrow().len(), 1, "the accepted export reaches the injected writer");
    open_png_options(&mut app);
    assert_eq!(fields(&app).number("ppi"), Some(600.0), "the next export remembers the accepted resolution");
    assert_eq!(raster_document_state(&app), before, "export preferences leave the document, selection and Undo/Redo intact");
}

fn accepted_600_ppi() -> crate::VectorcraftApp {
    let (mut app, _) = app(1);
    open_png_options(&mut app);
    set(&mut app, "ppi", json!(600));
    confirm(&mut app);
    app
}

#[test]
fn accepted_raster_resolution_survives_ui_preferences_restore() {
    let original = accepted_600_ppi();
    // The native host restores serialized UiState; no user preferences or files are accessed.
    let saved = serde_json::to_vec(&original.ui).unwrap();
    let ui = serde_json::from_slice::<crate::state::UiState>(&saved).unwrap().sanitized();
    let (mut restored, _) = app(1);
    restored.ui = ui;
    open_png_options(&mut restored);
    assert_eq!(fields(&restored).number("ppi"), Some(600.0));
}

#[test]
fn canceling_raster_options_keeps_the_last_accepted_resolution() {
    let mut app = accepted_600_ppi();
    open_png_options(&mut app);
    set(&mut app, "ppi", json!(300));
    cancel(&mut app);
    assert!(kind(&app).is_none());
    open_png_options(&mut app);
    assert_eq!(fields(&app).number("ppi"), Some(600.0), "Cancel does not remember an unaccepted choice");
}

#[test]
fn explicit_png_resolution_takes_precedence_over_remembered_resolution() {
    let mut app = accepted_600_ppi();
    app.run("file.export.png", json!({"ppi": 300})).unwrap();
    answer_saves(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("pngOptions"));
    assert_eq!(fields(&app).number("ppi"), Some(300.0), "the caller's explicit resolution wins over the remembered 600");
    cancel(&mut app);
    open_png_options(&mut app);
    assert_eq!(fields(&app).number("ppi"), Some(600.0), "canceling the explicit choice leaves memory alone");
}

#[test]
fn raster_formats_share_the_last_accepted_resolution() {
    let mut app = accepted_600_ppi();
    app.run("file.exportAs", json!({"format": "jpg"})).unwrap();
    assert_eq!(kind(&app).as_deref(), Some("exportAs"));
    set(&mut app, "useArtboards", json!(true));
    confirm(&mut app);
    answer_saves(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("jpgOptions"));
    assert_eq!(fields(&app).number("ppi"), Some(600.0), "JPEG uses the accepted PNG resolution");
    set(&mut app, "ppi", json!(300));
    confirm(&mut app);
    open_png_options(&mut app);
    assert_eq!(fields(&app).number("ppi"), Some(300.0), "PNG uses the subsequently accepted JPEG resolution");
}

fn dialog_texts(app: &mut crate::VectorcraftApp) -> Vec<String> {
    fn collect(shape: &egui::Shape, texts: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => texts.push(text.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, texts)),
            _ => {}
        }
    }
    super::tests::settle(app);
    let output = super::tests::frame(app, vec![]);
    let mut texts = Vec::new();
    output.shapes.iter().for_each(|shape| collect(&shape.shape, &mut texts));
    texts
}

#[test]
fn remembered_resolution_keeps_live_size_png_metadata_and_transparency() {
    let (mut app, written) = app(1);
    open_png_options(&mut app);
    set(&mut app, "ppi", json!(600));
    confirm(&mut app);
    open_png_options(&mut app);
    let texts = dialog_texts(&mut app);
    assert!(texts.iter().any(|text| text == "500 × 333 px"), "the actual Size row follows the remembered resolution: {texts:?}");
    confirm(&mut app);
    let output = written.borrow();
    assert_eq!(output.len(), 2);
    let png = &output[1].1;
    let image = super::tests_export::rgba(png);
    assert_eq!(image.dimensions(), (500, 333), "60 × 40 pt at 600 ppi");
    assert_eq!(image.get_pixel(0, 0)[3], 0, "the unchanged transparent background is preserved");
    assert!(image.pixels().any(|pixel| pixel[3] != 0), "the original ellipse is present");
    let phys = png.windows(4).position(|chunk| chunk == b"pHYs").unwrap();
    let pixels_per_meter = (600.0_f64 / 0.0254).round() as u32;
    assert_eq!(&png[phys + 4..phys + 8], pixels_per_meter.to_be_bytes());
    assert_eq!(&png[phys + 8..phys + 12], pixels_per_meter.to_be_bytes());
    assert_eq!(png[phys + 12], 1, "pHYs stores pixels per meter");
}

#[test]
fn supplied_raster_options_keep_values_without_changing_remembered_resolution() {
    let mut app = accepted_600_ppi();
    assert_eq!(app.ui.raster_export_ppi, Some(600.0));
    let png = json!({"ppi": 300, "background": "#204080", "antiAlias": "type", "interlaced": true});
    app.run("file.export.png", png.clone()).unwrap();
    answer_saves(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("pngOptions"));
    let f = fields(&app);
    assert_eq!(f.number("ppi"), png["ppi"].as_f64());
    for key in ["background", "antiAlias"] {
        assert_eq!(f.text(key), png[key].as_str(), "the supplied PNG {key} is retained");
    }
    assert_eq!(f.bool("interlaced"), png["interlaced"].as_bool());
    cancel(&mut app);
    assert_eq!(app.ui.raster_export_ppi, Some(600.0), "Cancel does not accept the explicit PNG resolution");

    let jpeg = json!({
        "format": "jpg", "path": "/out/supplied.jpg", "ppi": 144, "quality": 40,
        "colorModel": "gray", "method": "progressive", "scans": 5, "embedIcc": false, "imageMap": "client"
    });
    png_options::open(&mut app, fileio::format("jpg").unwrap(), jpeg.clone());
    assert_eq!(kind(&app).as_deref(), Some("jpgOptions"));
    let f = fields(&app);
    for key in ["ppi", "quality", "scans"] {
        assert_eq!(f.number(key), jpeg[key].as_f64(), "the supplied JPEG {key} is retained");
    }
    for key in ["colorModel", "method", "imageMap"] {
        assert_eq!(f.text(key), jpeg[key].as_str(), "the supplied JPEG {key} is retained");
    }
    assert_eq!(f.bool("embedIcc"), jpeg["embedIcc"].as_bool());
    cancel(&mut app);
    assert_eq!(app.ui.raster_export_ppi, Some(600.0), "Cancel does not accept the explicit JPEG resolution");
    open_png_options(&mut app);
    assert_eq!(fields(&app).number("ppi"), Some(600.0));
}

#[test]
fn invalid_explicit_resolution_is_rejected_without_writing_or_changing_memory() {
    let (mut app, written) = app(1);
    open_png_options(&mut app);
    set(&mut app, "ppi", json!(600));
    confirm(&mut app);
    assert_eq!(written.borrow().len(), 1, "the legitimate accepted export reaches the writer first");
    assert_eq!(app.ui.raster_export_ppi, Some(600.0));

    app.run("file.export.png", json!({"ppi": 0})).unwrap();
    answer_saves(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("pngOptions"));
    assert_eq!(fields(&app).number("ppi"), Some(0.0), "an explicit invalid resolution is not replaced by memory");
    let error = try_confirm(&mut app).unwrap_err();
    assert!(error.contains("ppi must be a positive number"), "{error}");
    assert_eq!(written.borrow().len(), 1, "the invalid resolution writes no file");
    assert_eq!(app.ui.raster_export_ppi, Some(600.0), "an error does not accept the invalid resolution");
    open_png_options(&mut app);
    assert_eq!(fields(&app).number("ppi"), Some(600.0));
}
