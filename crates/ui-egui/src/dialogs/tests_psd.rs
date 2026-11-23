//! PSD Options through Export As, drawn headlessly.

use serde_json::json;

use super::tests_export::{app, frame, kind, set};
use super::*;

#[test]
fn psd_options_pick_flat_or_layers_and_the_colour_model() {
    let (mut app, written) = app(1);
    app.run("file.exportAs", json!({"format": "psd"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("psdOptions"));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((spec(&d.kind).heading)(d), "PSD Options");
    assert_eq!(
        (d.str("colorModel"), d.bool("layers"), d.bool("preserveText"), d.bool("maxEditability"), d.bool("hiddenLayers"), d.bool("embedIcc")),
        ("rgb".into(), true, false, false, false, true)
    );
    assert!(tiff_bmp_tga::keeps_alpha("psd", d), "layers keep transparency");
    frame(&mut app);
    set(&mut app, "maxEditability", json!(true));
    frame(&mut app);
    confirm(&mut app).unwrap();
    let (path, psd) = written.borrow()[0].clone();
    assert_eq!(path, "/out/Untitled-1.psd");
    assert_eq!(&psd[..4], b"8BPS");
    // A group with the ellipse: three records, the merged image's alpha their transparency.
    let at = 26 + 4 + 4 + u32::from_be_bytes(psd[30..34].try_into().unwrap()) as usize;
    assert_eq!(i16::from_be_bytes([psd[at + 8], psd[at + 9]]), -3);
    assert!(app.ui.dialog.is_none());

    // Flat: flattened on white, so the background list starts at White. Preserve Text
    // Editability, still ticked, isn't sent: the export refuses it without layers.
    app.run("file.exportAs", json!({"format": "psd"})).unwrap();
    confirm(&mut app).unwrap();
    set(&mut app, "preserveText", json!(true));
    set(&mut app, "layers", json!(false));
    set(&mut app, "colorModel", json!("gray"));
    assert!(!tiff_bmp_tga::keeps_alpha("psd", app.ui.dialog.as_ref().unwrap()));
    frame(&mut app);
    confirm(&mut app).unwrap();
    let psd = written.borrow()[1].1.clone();
    assert_eq!((u16::from_be_bytes([psd[12], psd[13]]), u16::from_be_bytes([psd[24], psd[25]])), (1, 1), "one grey channel");

    // A CMYK document exports CMYK by default.
    app.run("object.convertDocumentColorMode", json!({"mode": "cmyk"})).unwrap();
    app.run("file.exportAs", json!({"format": "psd"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("colorModel"), "cmyk");
}
