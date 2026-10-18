//! The options of the export formats beyond PNG, drawn headlessly: JPEG, PNG-8, GIF and WebP, Text
//! Export Options and Save for Office Documents.

use serde_json::json;

use super::tests_export::{app, frame, kind, set};
use super::*;

/// A JPEG's SOF marker and component count.
fn jpeg_frame(file: &[u8]) -> (u8, u8) {
    let mut i = 2;
    while !(0xC0..=0xC2).contains(&file[i + 1]) {
        i += 2 + u16::from_be_bytes([file[i + 2], file[i + 3]]) as usize;
    }
    (file[i + 1], file[i + 9])
}

#[test]
fn jpeg_options_pick_the_colour_model_method_and_profile() {
    let (mut app, written) = app(1);
    app.run("file.exportAs", json!({"format": "jpg"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("jpgOptions"));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(
        (d.str("colorModel"), d.str("method"), d.f64("scans", 0.0), d.bool("embedIcc"), d.str("imageMap")),
        ("rgb".into(), "baseline".into(), 3.0, true, "none".into())
    );
    frame(&mut app);
    set(&mut app, "method", json!("progressive"));
    set(&mut app, "scans", json!(4));
    set(&mut app, "colorModel", json!("cmyk"));
    set(&mut app, "quality", json!(30));
    frame(&mut app);
    confirm(&mut app).unwrap();
    let file = written.borrow()[0].1.clone();
    assert_eq!(jpeg_frame(&file), (0xC2, 4), "progressive CMYK");
    assert!(file.windows(12).any(|w| w == b"ICC_PROFILE\0"));

    // A CMYK document exports CMYK by default.
    app.run("object.convertDocumentColorMode", json!({"mode": "cmyk"})).unwrap();
    app.run("file.exportAs", json!({"format": "jpg"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("colorModel"), "cmyk");
}

#[test]
fn palette_options_reduce_the_colours() {
    let (mut app, written) = app(1);
    app.run("file.exportAs", json!({"format": "gif"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("gifOptions"));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((spec(&d.kind).heading)(d), "GIF Options");
    assert_eq!(
        (d.str("reduction"), d.f64("colors", 0.0), d.str("dither"), d.bool("transparency"), d.str("matte")),
        ("selective".into(), 256.0, "diffusion".into(), true, "white".into())
    );
    frame(&mut app);
    set(&mut app, "colors", json!(4));
    set(&mut app, "dither", json!("pattern"));
    set(&mut app, "matte", json!("#336699"));
    set(&mut app, "interlaced", json!(true));
    frame(&mut app);
    confirm(&mut app).unwrap();
    let (path, gif) = written.borrow()[0].clone();
    assert_eq!(path, "/out/Untitled-1.gif");
    assert_eq!(&gif[..6], b"GIF89a");
    assert_eq!(gif[10] & 0x07, 1, "a global colour table of 4 entries");

    app.run("file.exportAs", json!({"format": "png8"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("png8Options"));
    set(&mut app, "reduction", json!("gray"));
    frame(&mut app);
    confirm(&mut app).unwrap();
    let (path, png) = written.borrow()[1].clone();
    assert_eq!(path, "/out/Untitled-1.png");
    assert_eq!(png[25], 3, "indexed");
}

#[test]
fn text_export_options_write_the_stories() {
    let (mut app, written) = app(1);
    app.run("text.create", json!({"x": 5, "y": 30, "text": "Hello\nthere"})).unwrap();
    app.run("file.exportAs", json!({"format": "txt"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("txtOptions"));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("path"), d.str("encoding")), ("/out/Untitled-1.txt".into(), "utf8".into()));
    frame(&mut app);
    set(&mut app, "lineEndings", json!("crlf"));
    set(&mut app, "encoding", json!("bogus"));
    assert!(confirm(&mut app).is_err());
    assert_eq!(kind(&app), Some("txtOptions"), "a bad option keeps the dialog open");
    set(&mut app, "encoding", json!("utf8"));
    frame(&mut app);
    assert_eq!(confirm(&mut app).unwrap()["path"], "/out/Untitled-1.txt");
    assert!(app.ui.dialog.is_none());
    let (path, bytes) = written.borrow().last().cloned().unwrap();
    assert_eq!((path.as_str(), bytes.as_slice()), ("/out/Untitled-1.txt", &b"Hello\r\nthere\r\n"[..]));
}

#[test]
fn webp_options_say_the_file_is_lossless() {
    let (mut app, _) = app(1);
    app.run("file.exportAs", json!({"format": "webp"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("webpOptions"));
    frame(&mut app);
}

#[test]
fn save_for_office_documents_picks_the_resolution() {
    let (mut app, written) = app(2);
    let item = crate::menus::menu_entries(&app).into_iter().find(|e| e.command.as_deref() == Some("document.exportForOffice")).unwrap();
    assert_eq!((item.path[0].as_str(), item.label.as_str(), item.enabled), ("File", "Save for Office Documents…", true));
    app.run("document.exportForOffice", json!({})).unwrap();
    assert_eq!(kind(&app), Some("saveForOffice"));
    frame(&mut app);
    set(&mut app, "ppi", json!(300));
    set(&mut app, "artboard", json!(1));
    set(&mut app, "transparent", json!(true));
    frame(&mut app);
    assert_eq!(confirm(&mut app).unwrap()["path"], "/out/Untitled-1.png");
    let img = image::load_from_memory(&written.borrow()[0].1).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (250, 167), "60 × 40 pt at 300 ppi");
    assert_eq!(img.get_pixel(0, 0)[3], 0, "transparent");
    // With a path (agents), no dialog.
    let r = app.run("document.exportForOffice", json!({"path": "/x/o.png", "ppi": 72})).unwrap();
    assert_eq!(r["path"], "/x/o.png");
    assert!(app.ui.dialog.is_none());
    let img = image::load_from_memory(&written.borrow()[1].1).unwrap().to_rgba8();
    assert_eq!((img.dimensions(), img.get_pixel(0, 0).0), ((60, 40), [255, 255, 255, 255]));
}

#[test]
fn save_for_office_and_export_selection_say_when_an_image_could_not_be_decoded() {
    const NOTE: &str = "with 1 note(s): an image couldn't be decoded and is missing from the picture";
    let (mut app, written) = app(1);
    app.run("document.exportForOffice", json!({"path": "/x/o.png", "ppi": 72})).unwrap();
    assert_eq!(app.ui.status, "Saved /x/o.png");
    // A PNG whose header reads (so it places at its size) but whose pixel data is cut short.
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 30, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let idat = png.windows(4).position(|w| w == b"IDAT").unwrap();
    png.truncate(idat + 8);
    app.run("file.place", json!({"name": "cut.png", "dataBase64": vectorcraft_format::base64_encode(&png), "at": [10, 10]})).unwrap();
    app.run("document.exportForOffice", json!({"path": "/x/o.png", "ppi": 72})).unwrap();
    assert_eq!(app.ui.status, format!("Saved /x/o.png {NOTE}"));
    // Export Selection of the placed image, which stays selected.
    app.run("document.exportSelection", json!({"path": "/x/cut.png"})).unwrap();
    assert_eq!(app.ui.status, format!("Saved /x/cut.png {NOTE}"));
    assert_eq!(written.borrow().len(), 3, "the files are still written");
}
