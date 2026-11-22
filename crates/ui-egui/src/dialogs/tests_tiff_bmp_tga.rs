//! BMP options and the Targa depth's rule for transparency, drawn headlessly. TIFF and Targa's
//! rows reaching the file are rows of `raster_option_rows_reach_the_export_params`.

use craft::ui::patterns::form_dialogs::DialogFields;
use serde_json::json;

use super::tests_export::{app, confirm, frame, kind, set};
use super::tests_raster_formats::fields;
use super::*;

#[test]
fn bmp_options_keep_their_choices_writable() {
    let (mut app, written) = app(1);
    app.run("file.exportAs", json!({"format": "bmp"})).unwrap();
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("bmpOptions"));
    let f = fields(&app);
    assert_eq!(
        (f.text("colorModel"), f.text("fileFormat"), f.text("depth"), f.bool("rle"), f.bool("flipRows")),
        (Some("rgb"), Some("windows"), Some("24"), Some(false), Some(false))
    );
    frame(&mut app);
    // OS/2 has no 32 bits: the depth falls back to 24.
    set(&mut app, "depth", json!("32"));
    frame(&mut app);
    set(&mut app, "fileFormat", json!("os2"));
    frame(&mut app);
    assert_eq!(fields(&app).text("depth"), Some("24"));
    // Compressed rows are bottom-up: RLE turns flipped rows off.
    set(&mut app, "fileFormat", json!("windows"));
    set(&mut app, "depth", json!("8"));
    set(&mut app, "flipRows", json!(true));
    set(&mut app, "rle", json!(true));
    frame(&mut app);
    assert_eq!(fields(&app).bool("flipRows"), Some(false));
    confirm(&mut app);
    let (path, bmp) = written.borrow()[0].clone();
    assert_eq!(path, "/out/Untitled-1.bmp");
    assert_eq!((&bmp[..2], bmp[28], bmp[30]), (&b"BM"[..], 8, 1), "8-bit RLE8");
}

/// Targa at 24 bits is flattened on white; its other depths keep transparency.
#[test]
fn targa_keeps_transparency_above_24_bits() {
    assert!(!tiff_bmp_tga::keeps_alpha("tga", &DialogFields::from_pairs([("depth", "24")])));
    assert!(tiff_bmp_tga::keeps_alpha("tga", &DialogFields::from_pairs([("depth", "32")])));
}
