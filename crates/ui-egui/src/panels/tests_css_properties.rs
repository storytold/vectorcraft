//! The CSS Properties panel drawn headless: the selection's CSS in the code view, Generate CSS
//! for the whole document, the export options in the panel menu, and Copy Selected Style and
//! Export through their UI commands.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_svg::{CssOptions, CssUnits};

use super::css_properties::{ALL, ID, OPTIONS, menu, show};
use super::set_pstate;
use crate::tests_labels::painted_text;
use crate::{Services, VectorcraftApp};

type Log = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// An app with a selected red 30 × 20 rectangle; files written go to the log.
fn app() -> (VectorcraftApp, Log) {
    let log = Log::default();
    let l = log.clone();
    let services = Services {
        write: Some(Box::new(move |path: &str, bytes: &[u8]| {
            l.borrow_mut().push((path.to_string(), bytes.to_vec()));
            Ok(())
        })),
        ..Services::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    (app, log)
}

#[test]
fn the_code_view_shows_the_selections_css_with_the_options() {
    let (mut app, _) = app();
    assert!(crate::state::ICON_PANELS.iter().any(|(id, ..)| *id == ID));
    let text = painted_text(&mut app, show);
    assert!(text.contains(".rectangle {") && text.contains("background-color: #ff0000;") && text.contains("width: 30px;"), "{text}");
    let text = painted_text(&mut app, |app, ui| {
        set_pstate(ui.ctx(), OPTIONS, CssOptions { units: CssUnits::Pt, position: true, ..CssOptions::default() });
        show(app, ui);
    });
    assert!(text.contains("width: 30pt;") && text.contains("left: 10pt;"), "{text}");
}

#[test]
fn generate_css_shows_the_document_while_nothing_is_selected() {
    let (mut app, _) = app();
    app.run("shape.ellipse", json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let text = painted_text(&mut app, show);
    assert!(text.contains("No selection") && !text.contains(".rectangle"), "{text}");
    let text = painted_text(&mut app, |app, ui| {
        set_pstate(ui.ctx(), ALL, true);
        show(app, ui);
    });
    assert!(text.contains(".rectangle {") && text.contains(".ellipse {") && text.contains("border-radius: 50%;"), "{text}");
}

#[test]
fn the_panel_menu_lists_the_actions_and_options() {
    let (mut app, _) = app();
    let text = painted_text(&mut app, menu);
    for item in [
        "Copy Selected Style",
        "Export Selected CSS…",
        "Export All…",
        "Generate CSS",
        "Units: px",
        "Absolute Position",
        "Width and Height",
        "Unnamed Objects",
        "Rasterize Unsupported Art",
    ] {
        assert!(text.contains(item), "{item} in {text}");
    }
}

#[test]
fn copy_and_export_go_through_the_ui_commands() {
    let (mut app, log) = app();
    let r = app.run("css.copy", json!({})).unwrap();
    assert_eq!(r["rules"], json!(1));
    assert!(app.clipboard_out.as_deref().is_some_and(|c| c.starts_with(".rectangle {") && c.contains("#ff0000")));
    app.run("select.none", json!({})).unwrap();
    assert!(app.run("css.copy", json!({})).is_err(), "nothing selected: nothing to copy");
    // Export All with a star rasterized: the sheet and the star's picture beside it.
    app.run("shape.star", json!({"cx": 100, "cy": 50, "radius1": 30, "radius2": 15})).unwrap();
    let r = app.run("css.exportFile", json!({"path": "/tmp/site/site.css", "scope": "all", "rasterize": true})).unwrap();
    assert_eq!(r["rules"], json!(2));
    let png = std::path::Path::new("/tmp/site").join("path.png").to_string_lossy().into_owned();
    assert_eq!(r["images"], json!([png]));
    let log = log.borrow();
    let names: Vec<&str> = log.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(names, ["/tmp/site/site.css", png.as_str()]);
    let css = String::from_utf8(log[0].1.clone()).unwrap();
    assert!(css.contains(".rectangle {") && css.contains("background-image: url(path.png);"), "{css}");
    assert!(log[1].1.starts_with(b"\x89PNG"));
}

#[test]
fn export_says_in_the_status_bar_when_an_image_could_not_be_decoded() {
    const WARNING: &str = "an image couldn't be decoded and is missing from the picture";
    let (mut app, log) = app();
    app.run("css.exportFile", json!({"path": "/tmp/site/site.css", "scope": "all", "rasterize": true})).unwrap();
    assert_eq!(app.ui.status, "Exported /tmp/site/site.css");
    // A PNG whose header reads (so it places at its size) but whose pixel data is cut short.
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 30, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let idat = png.windows(4).position(|w| w == b"IDAT").unwrap();
    png.truncate(idat + 8);
    app.run("file.place", json!({"name": "cut.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
    let r = app.run("css.exportFile", json!({"path": "/tmp/site/site.css", "scope": "all", "rasterize": true})).unwrap();
    assert_eq!(r["warnings"], json!([WARNING]));
    assert_eq!(app.ui.status, format!("Exported /tmp/site/site.css with 1 note(s): {WARNING}"));
    assert_eq!(log.borrow().len(), 3, "the sheet and the image's picture are still written");
}
