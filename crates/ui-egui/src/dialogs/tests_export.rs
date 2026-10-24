//! Export As and the raster options dialogs, drawn headlessly.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::fileio;

use super::*;
use crate::Services;
use crate::theme;

pub(super) type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// An app with `artboards` 60×40 pt artboards whose save dialog answers `/out/<suggested>` and
/// whose writer records what it is given.
pub(super) fn app(artboards: usize) -> (VectorcraftApp, Written) {
    app_saving(artboards, |p| format!("/out/{}", p.name))
}

/// [`app`], with a save dialog that answers `pick`.
pub(super) fn app_saving(artboards: usize, pick: fn(&crate::FilePick) -> String) -> (VectorcraftApp, Written) {
    let written = Written::default();
    let w = written.clone();
    let services = Services {
        pick_save: Some(Box::new(move |p: &crate::FilePick| Some(pick(p)))),
        write: Some(Box::new(move |p: &str, b: &[u8]| {
            w.borrow_mut().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 60, "height": 40, "artboards": artboards})).unwrap();
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    app.run("shape.ellipse", json!({"x": 5, "y": 4, "width": 30, "height": 20})).unwrap();
    (app, written)
}

/// One headless frame of the dialog layer.
pub(super) fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(Default::default(), |ui| show(app, ui.ctx()));
    out.textures_delta.clear();
}

pub(super) fn kind(app: &VectorcraftApp) -> Option<&str> {
    app.ui.dialog.as_ref().map(|d| d.kind.as_str())
}

pub(super) fn set(app: &mut VectorcraftApp, field: &str, value: serde_json::Value) {
    app.ui.dialog.as_mut().expect("a dialog is open").fields.insert(field.into(), value);
}

pub(super) fn names(w: &Written) -> Vec<String> {
    w.borrow().iter().map(|(p, _)| p.clone()).collect()
}

#[test]
fn png_options_draw_and_export_with_the_chosen_options() {
    let (mut app, written) = app(2);
    png_options::open(&mut app, fileio::format("png").unwrap(), json!({"format": "png", "path": "/out/a.png", "artboard": 1}));
    assert_eq!(kind(&app), Some("pngOptions"));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.f64("ppi", 0.0), d.str("background"), d.str("antiAlias"), d.bool("interlaced")), (72.0, "transparent".into(), "art".into(), false));
    assert_eq!(d.fields["__size"], json!([60.0, 40.0]), "the chosen artboard");
    frame(&mut app);
    // Other resolution and an Other background colour draw their own field and colour button.
    set(&mut app, "__otherPpi", json!(true));
    set(&mut app, "ppi", json!(200));
    set(&mut app, "background", json!("#204080"));
    set(&mut app, "antiAlias", json!("type"));
    set(&mut app, "interlaced", json!(true));
    frame(&mut app);
    assert_eq!(confirm(&mut app).unwrap()["path"], "/out/a.png");
    assert!(app.ui.dialog.is_none());
    let png = written.borrow()[0].1.clone();
    assert_eq!(png[28], 1, "interlaced");
    let phys = png.windows(4).position(|w| w == b"pHYs").unwrap();
    assert_eq!(&png[phys + 4..phys + 8], ((200.0f64 / 0.0254).round() as u32).to_be_bytes());
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (167, 111), "60 × 40 pt at 200 ppi");
    assert_eq!(img.get_pixel(0, 0).0, [0x20, 0x40, 0x80, 255]);
    // JPEG Options: quality, and no transparent background.
    png_options::open(&mut app, fileio::format("jpg").unwrap(), json!({"format": "jpg", "path": "/out/a.jpg"}));
    assert_eq!(kind(&app), Some("jpgOptions"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("background"), "white");
    frame(&mut app);
    set(&mut app, "quality", json!(40));
    confirm(&mut app).unwrap();
    assert_eq!(&written.borrow()[1].1[..2], [0xFF, 0xD8]);
}

#[test]
fn export_as_png_picks_the_file_then_shows_png_options() {
    let (mut app, written) = app(1);
    app.run("file.export.png", json!({})).unwrap();
    assert_eq!(kind(&app), Some("pngOptions"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("path"), "/out/Untitled-1.png");
    assert!(written.borrow().is_empty());
    app.ui.dialog = None;
    let r = app.run("file.export.png", json!({"path": "/x/a.png", "ppi": 144})).unwrap();
    assert_eq!(r["path"], "/x/a.png");
    assert_eq!(&written.borrow()[0].1[16..20], 120u32.to_be_bytes(), "60 pt at 144 ppi");
}

#[test]
fn export_as_with_a_range_then_png_options_writes_one_file_per_artboard() {
    let (mut app, written) = app(3);
    app.run("file.exportAs", json!({})).unwrap();
    assert_eq!(kind(&app), Some("exportAs"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("range"), "1-3");
    frame(&mut app);
    set(&mut app, "useArtboards", json!(true));
    set(&mut app, "all", json!(false));
    set(&mut app, "range", json!("1, 3"));
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("pngOptions"), "PNG Options follow");
    assert!(written.borrow().is_empty(), "nothing is written before the options are confirmed");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("path"), d.str("range"), d.str("background")), ("/out/Untitled-1.png".into(), "1, 3".into(), "transparent".into()));
    assert!(!d.fields.contains_key("__size"), "two artboards: no single size to show");
    frame(&mut app);
    set(&mut app, "ppi", json!(144));
    set(&mut app, "interlaced", json!(true));
    set(&mut app, "antiAlias", json!("none"));
    frame(&mut app);
    let r = confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(r["files"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!(names(&written), ["/out/Untitled-1-Artboard-1.png", "/out/Untitled-1-Artboard-3.png"]);
    let png = &written.borrow()[0].1;
    assert_eq!(png[28], 1, "interlaced");
    let img = image::load_from_memory(png).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (120, 80), "144 ppi");
    assert!(img.pixels().all(|p| p[3] == 0 || p[3] == 255), "no anti-aliasing");
}

#[test]
fn a_bad_range_keeps_export_as_open() {
    let (mut app, written) = app(2);
    app.run("file.exportAs", json!({"useArtboards": true, "range": "5"})).unwrap();
    assert!(!app.ui.dialog.as_ref().unwrap().bool("all"));
    assert!(confirm(&mut app).is_err());
    assert_eq!(kind(&app), Some("exportAs"));
    assert!(written.borrow().is_empty());
}

#[test]
fn svg_continues_in_svg_options_and_pdf_art_is_written_directly() {
    let (mut app, written) = app(2);
    app.run("file.exportAs", json!({"format": "svg", "useArtboards": true})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("svgOptions"), "the SVG options follow");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("path"), d.bool("useArtboards"), d.str("range")), ("/out/Untitled-1.svg".into(), true, "all".into()));
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(names(&written), ["/out/Untitled-1-Artboard-1.svg", "/out/Untitled-1-Artboard-2.svg"]);
    // Without artboards: one file of the art's bounds, with no options dialog for a PDF.
    app.run("file.exportAs", json!({"format": "pdf"})).unwrap();
    let r = confirm(&mut app).unwrap();
    assert_eq!(r["path"], "/out/Untitled-1.pdf");
    let pdf = vectorcraft_pdf::import(&written.borrow()[2].1).unwrap();
    assert_eq!((pdf.artboards[0].rect.width().round(), pdf.artboards[0].rect.height().round()), (30.0, 20.0), "the ellipse's bounds");
}

#[test]
fn jpeg_and_webp_options_draw_and_export() {
    let (mut app, written) = app(1);
    app.run("file.exportAs", json!({"format": "jpg"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("jpgOptions"));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("background"), d.f64("quality", 0.0)), ("white".into(), 90.0));
    assert_eq!(d.fields["__size"], json!([30.0, 20.0]), "the art's bounds");
    frame(&mut app);
    set(&mut app, "background", json!("#204080"));
    set(&mut app, "quality", json!(70));
    frame(&mut app);
    assert_eq!(confirm(&mut app).unwrap()["path"], "/out/Untitled-1.jpg");
    let img = image::load_from_memory(&written.borrow()[0].1).unwrap().to_rgb8();
    assert_eq!(img.dimensions(), (30, 20));
    let px = img.get_pixel(0, 0);
    assert!(px[2] > 100 && px[0] < 60, "the background colour: {px:?}");

    app.run("file.exportAs", json!({"format": "webp", "useArtboards": true})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("webpOptions"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["__size"], json!([60.0, 40.0]), "the one artboard");
    set(&mut app, "__otherPpi", json!(true));
    set(&mut app, "ppi", json!(36));
    frame(&mut app);
    confirm(&mut app).unwrap();
    let (path, bytes) = written.borrow().last().cloned().unwrap();
    assert_eq!(path, "/out/Untitled-1.webp", "one artboard is one file, named as picked");
    assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 30);
}

#[test]
fn old_ids_and_agents_export_directly_with_a_path() {
    let (mut app, written) = app(2);
    let r = app.run("file.export.png", json!({"path": "/x/a.png", "artboard": 1, "ppi": 144})).unwrap();
    assert_eq!(r["path"], "/x/a.png");
    let r = app.run("file.exportAs", json!({"path": "/x/b.webp", "useArtboards": true})).unwrap();
    assert_eq!(r["files"], json!(["/x/b-Artboard-1.webp", "/x/b-Artboard-2.webp"]));
    assert!(app.ui.dialog.is_none());
    assert_eq!(written.borrow().len(), 3);
    assert_eq!(&written.borrow()[0].1[16..20], 120u32.to_be_bytes(), "60 pt at 144 ppi");
    assert!(crate::menus::enabled(&app, "file.exportAs"));
}

#[test]
fn nothing_to_export_keeps_export_as_open_before_the_save_dialog() {
    let (mut app, written) = app(1);
    app.services.pick_save = Some(Box::new(|_: &crate::FilePick| panic!("no save dialog without art")));
    app.run("select.all", json!({})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    app.run("file.exportAs", json!({})).unwrap();
    assert!(confirm(&mut app).unwrap_err().contains("no visible art"));
    assert_eq!(kind(&app), Some("exportAs"));
    // With Use Artboards the empty artboard is exported.
    set(&mut app, "useArtboards", json!(true));
    app.services.pick_save = Some(Box::new(|p: &crate::FilePick| Some(format!("/out/{}", p.name))));
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("pngOptions"));
    assert!(written.borrow().is_empty());
}

#[test]
fn pdf_artboards_continue_in_the_save_pdf_dialog() {
    let (mut app, written) = app(3);
    app.run("file.exportAs", json!({"format": "pdf", "useArtboards": true, "range": "2-3"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("savePdf"), "the PDF options follow");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("path"), d.str("range")), ("/out/Untitled-1.pdf".into(), "2-3".into()));
    assert!(written.borrow().is_empty());
    frame(&mut app);
    confirm(&mut app).unwrap();
    let pdf = vectorcraft_pdf::import(&written.borrow()[0].1).unwrap();
    assert_eq!(pdf.artboards.len(), 2, "one page per artboard in the range");
}

/// #546, #550: Export As with Use Artboards and a range writes the chosen artboard alone in every
/// format, each file holding that artboard's art (the ellipse is on artboard 1).
#[test]
fn export_as_writes_only_the_chosen_artboard_in_every_format() {
    for (format, ellipse) in [("svg", "<path"), ("dxf", "HATCH"), ("pdf", ""), ("png", "")] {
        for (range, on) in [("1", true), ("3", false)] {
            let (mut app, written) = app(3);
            app.run("file.exportAs", json!({"format": format})).unwrap();
            set(&mut app, "useArtboards", json!(true));
            set(&mut app, "all", json!(false));
            set(&mut app, "range", json!(range));
            confirm(&mut app).unwrap();
            // The format's own options, then the file.
            while app.ui.dialog.is_some() {
                frame(&mut app);
                confirm(&mut app).unwrap();
            }
            assert_eq!(names(&written), [format!("/out/Untitled-1.{format}")], "{format} {range}");
            if !ellipse.is_empty() {
                let text = String::from_utf8_lossy(&written.borrow()[0].1).into_owned();
                assert_eq!(text.contains(ellipse), on, "{format} {range}: {text}");
            }
        }
    }
}

/// The Enter that closed the save panel, still held as the format's options open, doesn't write
/// the file; Enter pressed again after it is let go does. So for every options dialog Export As
/// opens after its panel, an exporter plug-in's included (as Save As's options).
#[test]
fn the_enter_that_closed_the_save_panel_does_not_confirm_the_options() {
    let params = r#"{"speed":{"type":"number","min":1,"max":100,"default":40}}"#;
    let module = wat::parse_str(vectorcraft_plugins::wat::returning(
        &vectorcraft_plugins::wat::exporter("test.ui.held", r#"["tuh"]"#, params),
        r#"{"dataBase64":"SGk="}"#,
    ))
    .unwrap();
    let enter = |pressed| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui.ctx()));
        out.textures_delta.clear();
    };
    // Each format whose options follow the panel (PDF's with Use Artboards: Save PDF).
    let formats = [("png", false), ("svg", false), ("pdf", true), ("txt", false), ("dxf", false), ("eps", false), ("plugin.test.ui.held", false)];
    for (format, artboards) in formats {
        let (mut app, written) = app(1);
        app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&module)})).unwrap();
        app.run("file.exportAs", json!({"format": format})).unwrap();
        set(&mut app, "useArtboards", json!(artboards));
        confirm(&mut app).unwrap();
        let options = kind(&app).map(str::to_string);
        assert!(options.as_deref().is_some_and(|k| k != "exportAs"), "{format}: its options follow the panel, not {options:?}");
        frame(&mut app, vec![enter(true)]);
        assert_eq!(kind(&app).map(str::to_string), options, "{format}: the Enter still held isn't the options'");
        assert!(written.borrow().is_empty(), "{format}: nothing written");
        frame(&mut app, vec![enter(false)]);
        frame(&mut app, vec![enter(true)]);
        assert!(app.ui.dialog.is_none(), "{format}: Enter pressed again confirms");
        assert!(!written.borrow().is_empty(), "{format}: and writes");
    }
    vectorcraft_plugins::registry::remove("test.ui.held");
}
