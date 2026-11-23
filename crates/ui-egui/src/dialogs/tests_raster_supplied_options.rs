use serde_json::json;
use vectorcraft_engine::cmd::fileio;

use super::tests_export::{app, frame, kind};
use super::{confirm, png_options};

#[test]
fn raster_options_preserve_supplied_values_through_the_png_command() {
    let (mut app, written) = app(1);
    app.run("file.export.png", json!({"ppi": 300, "background": "#204080", "antiAlias": "type", "interlaced": true})).unwrap();
    assert_eq!(kind(&app), Some("pngOptions"));
    let fields = &app.ui.dialog.as_ref().unwrap().fields;
    assert_eq!(fields["ppi"], 300);
    assert_eq!(fields["background"], "#204080");
    assert_eq!(fields["antiAlias"], "type");
    assert_eq!(fields["interlaced"], true);
    frame(&mut app);
    confirm(&mut app).unwrap();
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
    let fields = &app.ui.dialog.as_ref().unwrap().fields;
    assert_eq!(fields["quality"], 40);
    assert_eq!(fields["ppi"], 144);
    assert_eq!(fields["background"], "white");
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert_eq!(written.borrow().len(), 2);

    for omitted in [json!({}), json!({"ppi": null, "background": null, "antiAlias": null, "interlaced": null})] {
        app.run("file.export.png", omitted).unwrap();
        let fields = &app.ui.dialog.as_ref().unwrap().fields;
        assert_eq!(fields["ppi"], 72);
        assert_eq!(fields["background"], "transparent");
        assert_eq!(fields["antiAlias"], "art");
        assert_eq!(fields["interlaced"], false);
        app.ui.dialog = None;
    }

    app.run("file.export.png", json!({"ppi": 0})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["ppi"], 0);
    assert!(confirm(&mut app).unwrap_err().contains("ppi must be a positive number"));
    assert_eq!(written.borrow().len(), 2);
}
