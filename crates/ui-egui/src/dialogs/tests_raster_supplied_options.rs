use serde_json::json;
use vectorcraft_engine::cmd::fileio;

use super::png_options;
use super::tests_export::{answer_saves, app, confirm, frame, kind, try_confirm};
use super::tests_raster_formats::fields;

#[test]
fn raster_options_preserve_supplied_values_through_the_png_command() {
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
        assert_eq!(f.number("ppi"), Some(72.0));
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
