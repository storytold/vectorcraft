//! `document.open` of an SVG whose images link to files: relative links are found in the SVG's
//! folder and stay linked (Links reports them), missing ones warn and are listed in
//! `missingLinks` with a placeholder; File → Place reads them the same way.

use std::path::PathBuf;

use serde_json::json;
use vectorcraft_doc::{ImageObject, NodeKind};

use super::*;

/// A fresh folder for one test (removed when dropped).
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vectorcraft-svgimport-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("img")).unwrap();
        Self(dir)
    }
    fn file(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![];
    {
        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(w, h, image::Rgba([230, 20, 20, 255]))).into();
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
            .map(|__b| out.extend_from_slice(&__b))
    }
    .unwrap();
    out
}

/// An SVG linking to `img/photo.png` (here) and `img/gone.png` (missing).
fn write_svg(dir: &Folder) -> String {
    std::fs::write(dir.file("img/photo.png"), png(300, 200)).unwrap();
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="400" height="300">
        <image xlink:href="img/photo.png" x="10" y="10" width="150" height="100"/>
        <image href="img/gone.png" x="200" y="10" width="100" height="100"/></svg>"#;
    let path = dir.file("art.svg");
    std::fs::write(&path, svg).unwrap();
    path
}

fn images(s: &Session) -> Vec<ImageObject> {
    let mut out = vec![];
    s.documents()[s.active_index().unwrap()].doc.walk(|n| {
        if let NodeKind::Image(im) = &n.kind {
            out.push((**im).clone());
        }
    });
    out
}

#[test]
fn opening_an_svg_links_its_images_from_its_folder() {
    let dir = Folder::new("open");
    let path = write_svg(&dir);
    let mut s = Session::new();
    let r = s.execute("document.open", &json!({"path": path})).unwrap();
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("gone.png")), "{r}");
    let missing = r["missingLinks"].as_array().unwrap();
    assert_eq!(missing.len(), 1, "{r}");
    assert_eq!(missing[0]["name"], "gone.png");
    assert_eq!(r["modifiedLinks"], json!([]));
    let ims = images(&s);
    assert_eq!(ims.len(), 2);
    let photo = ims[0].link.as_ref().unwrap();
    assert_eq!(PathBuf::from(&photo.path), PathBuf::from(dir.file("img/photo.png")));
    assert!(photo.modified.is_some() && photo.hash.is_some(), "{photo:?}");
    assert_eq!((ims[0].width, ims[0].height), (300, 200));
    // Links reports the found file as it is, the missing one as missing.
    let c = s.execute("links.check", &json!({})).unwrap();
    assert_eq!((c["missing"].as_u64(), c["modified"].as_u64()), (Some(1), Some(0)), "{c}");
    // Once the file is there, Update Links reads it into the placeholder's box.
    std::fs::write(dir.file("img/gone.png"), png(50, 50)).unwrap();
    let r = s.execute("links.update", &json!({})).unwrap();
    assert_eq!(r["updated"].as_array().unwrap().len(), 1, "{r}");
    let gone = &images(&s)[1];
    assert_eq!((gone.width, gone.height), (50, 50));
    let b = gone.xf.transform_rect_bbox(vectorcraft_geom::Rect::new(0.0, 0.0, 50.0, 50.0));
    assert!((b.x0 - 200.0).abs() < 1e-6 && (b.width() - 100.0).abs() < 1e-6, "{b:?}");
}

#[test]
fn placing_an_svg_reads_its_linked_images() {
    let dir = Folder::new("place");
    let path = write_svg(&dir);
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s.execute("file.place", &json!({"path": path})).unwrap();
    let ims = images(&s);
    assert_eq!(ims.len(), 2);
    assert!(ims.iter().all(|im| im.link.is_some()));
    let doc = &s.documents()[s.active_index().unwrap()].doc;
    assert!(!doc.images[&ims[0].key].is_proxy(), "the file's pixels");
}
