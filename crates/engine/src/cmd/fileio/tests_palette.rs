//! PNG-8 and GIF: palette exports through document.export and Export for Screens.

use serde_json::{Value, json};

use super::*;

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// A 40×30 pt document with a red rectangle and a blue ellipse, no strokes.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 40, "height": 30})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 30})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 22, "y": 5, "width": 15, "height": 20})).unwrap();
    // Fills the new ellipse (selected).
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    s
}

/// PNG `PLTE` entries.
fn plte_len(file: &[u8]) -> usize {
    let i = file.windows(4).position(|w| w == b"PLTE").expect("PLTE");
    u32::from_be_bytes(file[i - 4..i].try_into().unwrap()) as usize / 3
}

#[test]
fn png8_and_gif_export_palettes() {
    let mut s = session();
    let png8 = b64(&s.execute("document.export", &json!({"format": "png8", "colors": 16})).unwrap());
    assert_eq!(png8[25], 3, "colour type 3");
    assert!(plte_len(&png8) <= 16);
    let img = craft_image_codecs::decode(&png8)
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
    assert_eq!(img.dimensions(), (40, 30));
    assert_eq!(img.get_pixel(5, 15).0, [255, 0, 0, 255]);
    assert_eq!(img.get_pixel(39, 0)[3], 0, "transparent where there is no art");

    let gif = b64(&s.execute("document.export", &json!({"format": "gif", "background": "white", "dither": "none"})).unwrap());
    assert_eq!(&gif[..6], b"GIF89a");
    let img = craft_image_codecs::decode_as(craft_image_codecs::Format::Gif, &gif)
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
    assert_eq!(img.get_pixel(5, 15).0, [255, 0, 0, 255]);
    assert_eq!(img.get_pixel(39, 0).0, [255, 255, 255, 255], "the white background");

    // Black & White: two entries whatever the art.
    let bw = b64(&s.execute("document.export", &json!({"format": "png8", "reduction": "blackWhite", "transparency": false})).unwrap());
    assert!(plte_len(&bw) <= 2);
    for bad in [json!({"reduction": "octree"}), json!({"dither": "lots"}), json!({"matte": "plaid"})] {
        assert!(s.execute("document.export", &merge(json!({"format": "gif"}), bad.clone())).is_err(), "{bad}");
    }
}

#[test]
fn palette_formats_reach_export_as_and_screens() {
    let mut s = session();
    // A .png path stays PNG; png8 is asked for by format.
    let r = s.execute("document.formats", &json!({})).unwrap();
    let f = |id: &str| r["formats"].as_array().unwrap().iter().find(|f| f["id"] == id).unwrap().clone();
    assert_eq!((f("png8")["extensions"][0].as_str(), f("png8")["read"].as_bool()), (Some("png"), Some(false)));
    assert_eq!(f("gif")["write"], true);
    for k in ["colors", "reduction", "dither", "ditherAmount", "transparency", "matte", "interlaced"] {
        assert!(f("gif")["options"].get(k).is_some() && f("png8")["options"].get(k).is_some(), "{k}");
    }
    assert_eq!(format_for_name("a.png").unwrap().id, "png");
    let r = s.execute("document.exportForScreens", &json!({"formats": [{"format": "gif", "scale": 2}, {"format": "png8"}]})).unwrap();
    let names: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Artboard-1@2x.gif", "Artboard-1.png"]);
    let gif = vectorcraft_format::base64_decode(r["files"][0]["dataBase64"].as_str().unwrap()).unwrap();
    assert_eq!(
        craft_image_codecs::decode(&gif)
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
            .width(),
        80
    );
}
