//! The options of the export formats beyond PNG, drawn headlessly: JPEG, PNG-8, GIF and WebP, Text
//! Export Options and Save for Office Documents.

use serde_json::json;

use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog};

use super::tests_export::{app, confirm, frame, kind, set, try_confirm, try_set};
use crate::VectorcraftApp;

/// The values of the top dialog, a parameter dialog; empty when none is open.
pub(super) fn fields(app: &VectorcraftApp) -> DialogFields {
    app.dialogs()
        .and_then(|dialogs| {
            dialogs.with(|host| {
                let id = host.open_dialogs().last()?.id;
                host.get_by_id::<HostedParamDialog>(id).map(|d| d.fields().clone())
            })
        })
        .unwrap_or_default()
}

/// A JPEG's SOF marker and component count.
fn jpeg_frame(file: &[u8]) -> (u8, u8) {
    let mut i = 2;
    while !(0xC0..=0xC2).contains(&file[i + 1]) {
        i += 2 + u16::from_be_bytes([file[i + 2], file[i + 3]]) as usize;
    }
    (file[i + 1], file[i + 9])
}

/// Each raster format's option rows, set as the user sets them, reach the written file:
/// `(format, the rows set, what the file shows)`.
#[test]
fn raster_option_rows_reach_the_export_params() {
    type Check = fn(&[u8]);
    type Set<'a> = &'a [(&'a str, serde_json::Value)];
    let rows: [(&str, Set<'_>, Check); 8] = [
        ("jpg", &[("method", json!("progressive")), ("scans", json!(4)), ("colorModel", json!("cmyk")), ("quality", json!(30))], |file| {
            assert_eq!(jpeg_frame(file), (0xC2, 4), "progressive CMYK");
            assert!(file.windows(12).any(|w| w == b"ICC_PROFILE\0"));
        }),
        ("gif", &[("colors", json!(4)), ("dither", json!("pattern")), ("matte", json!("#336699")), ("interlaced", json!(true))], |gif| {
            assert_eq!(&gif[..6], b"GIF89a");
            assert_eq!(gif[10] & 0x07, 1, "a global colour table of 4 entries");
        }),
        ("png8", &[("reduction", json!("gray"))], |png| assert_eq!(png[25], 3, "indexed")),
        ("webp", &[("lossless", json!("false")), ("quality", json!(40))], |file| {
            assert!(file.windows(4).any(|w| w == b"VP8 ") && !file.windows(4).any(|w| w == b"VP8L"), "lossy");
        }),
        ("tiff", &[("byteOrder", json!("big")), ("colorModel", json!("cmyk")), ("lzw", json!(false))], |tiff| assert_eq!(&tiff[..4], b"MM\0*")),
        ("tga", &[("depth", json!("32"))], |tga| assert_eq!((tga[16], tga[17]), (32, 8))),
        ("psd", &[("maxEditability", json!(true))], |psd| {
            assert_eq!(&psd[..4], b"8BPS");
            // A group with the ellipse: three records, the merged image's alpha their transparency.
            let at = 26 + 4 + 4 + u32::from_be_bytes(psd[30..34].try_into().unwrap()) as usize;
            assert_eq!(i16::from_be_bytes([psd[at + 8], psd[at + 9]]), -3);
        }),
        // Flat, flattened on white; Preserve Text Editability, still ticked, isn't sent: the export refuses it without layers.
        ("psd", &[("preserveText", json!(true)), ("layers", json!("false")), ("colorModel", json!("gray"))], |psd| {
            assert_eq!((u16::from_be_bytes([psd[12], psd[13]]), u16::from_be_bytes([psd[24], psd[25]])), (1, 1), "one grey channel");
        }),
    ];
    for (format, sets, check) in rows {
        let (mut app, written) = app(1);
        app.run("file.exportAs", json!({"format": format})).unwrap();
        confirm(&mut app);
        assert_eq!(kind(&app).map(|k| k.strip_suffix("Options").map(str::to_string)), Some(Some(format.to_string())));
        frame(&mut app);
        for (key, value) in sets {
            set(&mut app, key, value.clone());
        }
        frame(&mut app);
        try_confirm(&mut app).unwrap_or_else(|e| panic!("{format}: {e}"));
        let file = written.borrow()[0].1.clone();
        check(&file);
    }
}

/// A CMYK document starts the colour-model formats in CMYK.
#[test]
fn a_cmyk_document_exports_cmyk_by_default() {
    let (mut app, _) = app(1);
    app.run("object.convertDocumentColorMode", json!({"mode": "cmyk"})).unwrap();
    for format in ["jpg", "tiff", "psd"] {
        app.run("file.exportAs", json!({"format": format})).unwrap();
        confirm(&mut app);
        assert_eq!(fields(&app).text("colorModel"), Some("cmyk"), "{format}");
    }
}

#[test]
fn text_export_options_write_the_stories() {
    let (mut app, written) = app(1);
    app.run("text.create", json!({"x": 5, "y": 30, "text": "Hello\nthere"})).unwrap();
    app.run("file.exportAs", json!({"format": "txt"})).unwrap();
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("txtOptions"));
    assert_eq!((fields(&app).text("path"), fields(&app).text("encoding")), (Some("/out/Untitled-1.txt"), Some("utf8")));
    frame(&mut app);
    set(&mut app, "lineEndings", json!("crlf"));
    assert!(try_set(&mut app, "encoding", json!("bogus")).is_err(), "an encoding the list lacks is refused");
    assert_eq!(kind(&app).as_deref(), Some("txtOptions"), "a bad option keeps the dialog open");
    set(&mut app, "encoding", json!("utf8"));
    frame(&mut app);
    confirm(&mut app);
    assert_eq!(kind(&app), None);
    let (path, bytes) = written.borrow().last().cloned().unwrap();
    assert_eq!((path.as_str(), bytes.as_slice()), ("/out/Untitled-1.txt", &b"Hello\r\nthere\r\n"[..]));
}

/// WebP is lossless by default, and OK writes a lossless file without a note.
#[test]
fn webp_is_lossless_by_default() {
    let (mut app, written) = app(1);
    app.run("file.exportAs", json!({"format": "webp"})).unwrap();
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("webpOptions"));
    frame(&mut app);
    confirm(&mut app);
    assert!(!app.ui.status.contains("note"), "{}", app.ui.status);
    assert_eq!(&written.borrow()[0].1[12..16], b"VP8L", "lossless by default");
}

#[test]
fn save_for_office_documents_picks_the_resolution() {
    let (mut app, written) = app(2);
    let item = crate::menus::menu_entries(&app).into_iter().find(|e| e.command.as_deref() == Some("document.exportForOffice")).unwrap();
    assert_eq!((item.path[0].as_str(), item.label.as_str(), item.enabled), ("File", "Save for Office Documents…", true));
    app.run("document.exportForOffice", json!({})).unwrap();
    assert_eq!(kind(&app).as_deref(), Some("saveForOffice"));
    frame(&mut app);
    set(&mut app, "ppi", json!("300"));
    // The Artboard row is a choice keyed by the artboard's index.
    set(&mut app, "artboard", json!("1"));
    set(&mut app, "transparent", json!(true));
    frame(&mut app);
    confirm(&mut app);
    assert_eq!(written.borrow()[0].0, "/out/Untitled-1.png");
    let img = craft_image_codecs::decode(&written.borrow()[0].1)
        .map(|__cimg| {
            let (__w, __h) = __cimg.dimensions();
            match (__cimg.layout(), __cimg.sample_type()) {
                (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
            }
        })
        .unwrap()
        .to_rgba8();
    assert_eq!(img.dimensions(), (250, 167), "60 × 40 pt at 300 ppi");
    assert_eq!(img.get_pixel(0, 0)[3], 0, "transparent");
    // With a path (agents), no dialog.
    let r = app.run("document.exportForOffice", json!({"path": "/x/o.png", "ppi": 72})).unwrap();
    assert_eq!(r["path"], "/x/o.png");
    assert!(kind(&app).is_none());
    let img = craft_image_codecs::decode(&written.borrow()[1].1)
        .map(|__cimg| {
            let (__w, __h) = __cimg.dimensions();
            match (__cimg.layout(), __cimg.sample_type()) {
                (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
            }
        })
        .unwrap()
        .to_rgba8();
    assert_eq!((img.dimensions(), img.get_pixel(0, 0).0), ((60, 40), [255, 255, 255, 255]));
}

#[test]
fn save_for_office_and_export_selection_say_when_an_image_could_not_be_decoded() {
    const NOTE: &str = "with 1 note: an image couldn't be decoded and is missing from the picture";
    let (mut app, written) = app(1);
    app.run("document.exportForOffice", json!({"path": "/x/o.png", "ppi": 72})).unwrap();
    assert_eq!(app.ui.status, "Saved /x/o.png");
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
    vectorcraft_testkit::fixtures::place_cut_png(&mut app.session, &png, json!({"at": [10, 10]}));
    app.run("document.exportForOffice", json!({"path": "/x/o.png", "ppi": 72})).unwrap();
    assert_eq!(app.ui.status, format!("Saved /x/o.png {NOTE}"));
    // Export Selection of the placed image, which stays selected.
    app.run("document.exportSelection", json!({"path": "/x/cut.png"})).unwrap();
    assert_eq!(app.ui.status, format!("Saved /x/cut.png {NOTE}"));
    assert_eq!(written.borrow().len(), 3, "the files are still written");
}
