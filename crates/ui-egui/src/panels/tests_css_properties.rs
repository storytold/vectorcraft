//! The CSS Properties panel drawn headless: the selection's CSS in the code view, Generate CSS
//! for the whole document, the export options, and Copy Selected Style and Export through their UI
//! commands.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_svg::{CssOptions, CssUnits};

use super::css_properties::show;
use crate::tests::panel_harness::Panel;
use crate::{Services, VectorcraftApp};

/// The accessible name of the code view.
const CODE: &str = "CSS code";

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
    let (app, _) = app();
    let mut panel = Panel::new(app, show);
    panel.frame();
    let text = panel.value(CODE).unwrap_or_default();
    assert!(text.contains(".rectangle {") && text.contains("background-color: #ff0000;") && text.contains("width: 30px;"), "{text}");
    panel.app().ui.css_properties_panel.options = CssOptions { units: CssUnits::Pt, position: true, ..CssOptions::default() };
    panel.frame();
    let text = panel.value(CODE).unwrap_or_default();
    assert!(text.contains("width: 30pt;") && text.contains("left: 10pt;"), "{text}");
}

#[test]
fn generate_css_shows_the_document_while_nothing_is_selected() {
    let (mut app, _) = app();
    app.run("shape.ellipse", json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let mut panel = Panel::new(app, show);
    panel.frame();
    assert!(panel.shows("No selection") && !panel.shows(CODE), "the empty state");
    panel.app().run("ui.cssProperties.generate", json!({})).unwrap();
    panel.frame();
    let text = panel.value(CODE).unwrap_or_default();
    assert!(text.contains(".rectangle {") && text.contains(".ellipse {") && text.contains("border-radius: 50%;"), "{text}");
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
    // A PNG whose header reads but whose pixel data is cut short.
    let mut png = Vec::new();
    {
        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 30, 30, 255]))).into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
    vectorcraft_testkit::fixtures::place_cut_png(&mut app.session, &png, json!({}));
    let r = app.run("css.exportFile", json!({"path": "/tmp/site/site.css", "scope": "all", "rasterize": true})).unwrap();
    assert_eq!(r["warnings"], json!([WARNING]));
    assert_eq!(app.ui.status, format!("Exported /tmp/site/site.css with 1 note: {WARNING}"));
    assert_eq!(log.borrow().len(), 3, "the sheet and the image's picture are still written");
}

/// A CSS export with several notes: the status bar names the first, counts them, and keeps every
/// one of them for its hover.
#[test]
fn export_keeps_every_note_for_the_hover() {
    let (mut app, _) = app();
    let mut png = Vec::new();
    {
        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 30, 30, 255]))).into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
    vectorcraft_testkit::fixtures::place_cut_png(&mut app.session, &png, json!({}));
    // Type painted with a gradient is rasterized, and its picture is drawn in the fallback font.
    app.run("text.create", json!({"x": 5, "y": 20, "text": "Ab", "font": "Nonexistent Sans Probe"})).unwrap();
    app.run("paint.setFill", json!({"gradient": {"stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]}})).unwrap();
    let r = app.run("css.exportFile", json!({"path": "/tmp/site/site.css", "scope": "all", "rasterize": true})).unwrap();
    let notes = crate::io::notes_of(&r);
    assert_eq!(notes.len(), 2, "{r}");
    assert_eq!(app.ui.status, format!("Exported /tmp/site/site.css with 2 notes: {}", notes[0]));
    assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), notes)));
}
