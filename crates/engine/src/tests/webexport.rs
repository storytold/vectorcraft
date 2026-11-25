//! File → Export → Save for Web (Legacy): `document.exportForWeb`, its preview, the remembered
//! settings and the presets.

use serde_json::{Value, json};

use crate::*;

/// The default settings as JSON.
fn defaults() -> Value {
    cmd::webexport::WebSettings::default().to_json()
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A `w`×`h` pt document with unstroked art.
fn session(w: f64, h: f64) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": w, "height": h}));
    s.paint.stroke = vectorcraft_color::Paint::None;
    s
}

fn rect(s: &mut Session, [x, y, w, h]: [f64; 4], color: &str) -> u64 {
    let id = run(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h}))["id"].as_u64().unwrap();
    run(s, "paint.setFill", json!({"color": color}));
    id
}

/// 40×30: red on the left half, blue on the right (two colours, nothing transparent).
fn two_colours() -> Session {
    let mut s = session(40.0, 30.0);
    rect(&mut s, [0.0, 0.0, 20.0, 30.0], "#ff0000");
    rect(&mut s, [20.0, 0.0, 20.0, 30.0], "#0000ff");
    s
}

/// 64×64: an 8×8 grid of 64 different colours.
fn many_colours() -> Session {
    let mut s = session(64.0, 64.0);
    for i in 0..64 {
        let (x, y) = ((i % 8) as f64 * 8.0, (i / 8) as f64 * 8.0);
        rect(&mut s, [x, y, 8.0, 8.0], &format!("#{:02x}{:02x}{:02x}", i * 4, 255 - i * 3, (i * 37) % 256));
    }
    s
}

/// The files of an export without a path: (name, bytes).
fn files(v: &Value) -> Vec<(String, Vec<u8>)> {
    v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["name"].as_str().unwrap().to_string(), vectorcraft_format::base64_decode(f["dataBase64"].as_str().unwrap()).unwrap()))
        .collect()
}

/// The one file of an export.
fn one(s: &mut Session, p: Value) -> Vec<u8> {
    let f = files(&run(s, "document.exportForWeb", p));
    assert_eq!(f.len(), 1);
    f[0].1.clone()
}

fn pixels(file: &[u8]) -> image::RgbaImage {
    craft_image_codecs::decode(file)
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
        .to_rgba8()
}

fn distinct(img: &image::RgbaImage) -> usize {
    img.pixels().map(|p| p.0).collect::<std::collections::HashSet<_>>().len()
}

/// The transparent index of a GIF's graphic control extension, if it has one.
fn gif_transparent_index(gif: &[u8]) -> Option<u8> {
    let i = gif.windows(3).position(|w| w == [0x21, 0xF9, 0x04])?;
    (gif[i + 3] & 1 == 1).then_some(gif[i + 6])
}

#[test]
fn two_colours_give_a_two_colour_palette() {
    let mut s = two_colours();
    for format in ["gif", "png8"] {
        let p = json!({"format": format, "colors": 2, "antiAlias": "none"});
        let file = one(&mut s, p.clone());
        let img = pixels(&file);
        assert_eq!(img.dimensions(), (40, 30));
        assert_eq!(distinct(&img), 2, "{format}");
        assert_eq!(img.get_pixel(5, 5).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(35, 5).0, [0, 0, 255, 255]);
        let pv = run(&mut s, "document.exportForWeb.preview", p);
        let colors: Vec<&str> = pv["colors"].as_array().unwrap().iter().map(|c| c["color"].as_str().unwrap()).collect();
        assert_eq!(colors.len(), 2, "{format}: {colors:?}");
        assert!(colors.contains(&"#ff0000") && colors.contains(&"#0000ff"));
        assert!(pv["colors"].as_array().unwrap().iter().all(|c| c["webSafe"] == true && c["transparent"] == false));
    }
    // PNG-8 of two colours is one bit per pixel.
    let png8 = one(&mut s, json!({"format": "png8", "colors": 2}));
    assert_eq!(png8[24], 1);
}

#[test]
fn colours_cap_the_palette_at_16() {
    let mut s = many_colours();
    for format in ["gif", "png8"] {
        for dither in ["none", "diffusion", "pattern", "noise"] {
            for reduction in ["perceptual", "selective", "adaptive", "web"] {
                let p = json!({"format": format, "colors": 16, "dither": dither, "reduction": reduction});
                let img = pixels(&one(&mut s, p.clone()));
                assert!(distinct(&img) <= 16, "{p}: {}", distinct(&img));
                let pv = run(&mut s, "document.exportForWeb.preview", p.clone());
                assert!(pv["colors"].as_array().unwrap().len() <= 16, "{p}");
            }
        }
    }
    // 128 colours keep all 64.
    assert_eq!(distinct(&pixels(&one(&mut s, json!({"format": "png8", "colors": 128})))), 64);
}

#[test]
fn gif_transparency_index_and_map_to_transparent() {
    let mut s = session(40.0, 30.0);
    rect(&mut s, [10.0, 5.0, 10.0, 20.0], "#ff0000");
    rect(&mut s, [20.0, 5.0, 10.0, 20.0], "#00ff00");
    let gif = one(&mut s, json!({"format": "gif", "antiAlias": "none"}));
    assert_eq!(&gif[..6], b"GIF89a");
    assert_eq!(gif_transparent_index(&gif), Some(0), "the transparent entry is the first");
    let img = pixels(&gif);
    assert_eq!(img.get_pixel(1, 1)[3], 0);
    assert_eq!(img.get_pixel(15, 15).0, [255, 0, 0, 255]);
    // Without transparency: no transparent index; the background is the matte.
    let opaque = one(&mut s, json!({"format": "gif", "transparency": false, "matte": "#000000"}));
    assert_eq!(gif_transparent_index(&opaque), None);
    assert_eq!(pixels(&opaque).get_pixel(1, 1).0, [0, 0, 0, 255]);
    // Map to Transparent: red goes, green stays.
    let p = json!({"format": "gif", "antiAlias": "none", "colorTable": {"transparent": ["#ff0000"]}});
    let mapped = pixels(&one(&mut s, p.clone()));
    assert_eq!(mapped.get_pixel(15, 15)[3], 0);
    assert_eq!(mapped.get_pixel(25, 15).0, [0, 255, 0, 255]);
    let pv = run(&mut s, "document.exportForWeb.preview", p);
    assert_eq!(pv["mappedToTransparent"], json!(["#ff0000"]));
    assert!(pv["colors"].as_array().unwrap().iter().all(|c| c["source"] != "#ff0000"));
    // An opaque image gets a transparent index for a mapped colour.
    let gif = one(&mut s, json!({"format": "gif", "transparency": false, "colorTable": {"transparent": ["#00ff00"]}}));
    let i = gif_transparent_index(&gif).unwrap();
    assert_eq!(pixels(&gif).get_pixel(25, 15)[3], 0, "index {i}");
}

#[test]
fn jpeg_size_grows_with_quality() {
    let mut s = many_colours();
    run(&mut s, "shape.ellipse", json!({"x": 5, "y": 7, "width": 41, "height": 33}));
    let sizes: Vec<usize> = [0, 20, 40, 60, 80, 100].iter().map(|q| one(&mut s, json!({"format": "jpg", "quality": q})).len()).collect();
    assert!(sizes.windows(2).all(|w| w[0] <= w[1]), "{sizes:?}");
    assert!(sizes[0] < sizes[5], "{sizes:?}");
    let jpg = one(&mut s, json!({"format": "jpeg", "quality": 80}));
    assert_eq!(&jpg[..2], &[0xFF, 0xD8]);
    // Transparent areas are on the matte.
    let mut t = session(40.0, 30.0);
    rect(&mut t, [0.0, 0.0, 10.0, 10.0], "#000000");
    let yellow = |p: [u8; 4]| p[0] > 240 && p[1] > 240 && p[2] < 20;
    let img = pixels(&one(&mut t, json!({"format": "jpg", "quality": 100, "matte": "#ffff00", "optimized": false})));
    assert!(yellow(img.get_pixel(30, 20).0), "{:?}", img.get_pixel(30, 20));
    // The preview shows the pixels of an optimised file (decoded from a baseline twin: the
    // `image` crate misreads some optimised Huffman tables).
    let st = cmd::webexport::WebSettings { format: cmd::webexport::WebFormat::Jpg, quality: 90, matte: "#ffff00".into(), ..Default::default() };
    let doc = &t.doc().unwrap().doc;
    let r = cmd::webexport::render(doc, &st).unwrap();
    let o = cmd::webexport::preview_image(doc, &r.rgba, r.width, r.height, &st).unwrap();
    assert_eq!(o.bytes, cmd::webexport::optimize(doc, &r.rgba, r.width, r.height, &st).unwrap().bytes, "the file is the one Save writes");
    let (px, w, h) = cmd::webexport::decode(&o).unwrap();
    assert_eq!((w, h), (40, 30));
    let at = (20 * 40 + 30) * 4;
    assert!(yellow([px[at], px[at + 1], px[at + 2], px[at + 3]]), "{:?}", &px[at..at + 4]);
    // Progressive frames, and the sRGB profile when asked.
    let prog = one(&mut t, json!({"format": "jpg", "progressive": true, "embedProfile": true}));
    assert!(prog.windows(2).any(|w| w == [0xFF, 0xC2]));
    assert!(prog.windows(12).any(|w| w == b"ICC_PROFILE\0"));
}

#[test]
fn preview_size_is_the_written_size() {
    let mut s = many_colours();
    for p in [
        json!({"format": "gif", "colors": 32, "lossy": 40}),
        json!({"format": "png8", "colors": 64, "webSnap": 50, "interlaced": true}),
        json!({"format": "png24", "transparency": false}),
        json!({"format": "jpg", "quality": 35, "metadata": "all"}),
        json!({"format": "gif", "percent": 50, "colorTable": {"sort": "luminance"}}),
    ] {
        let file = one(&mut s, p.clone());
        let pv = run(&mut s, "document.exportForWeb.preview", merge(&p, json!({"image": true})));
        assert_eq!(pv["bytes"].as_u64().unwrap() as usize, file.len(), "{p}");
        assert_eq!(vectorcraft_format::base64_decode(pv["dataBase64"].as_str().unwrap()).unwrap(), file, "{p}");
        let img = pixels(&file);
        assert_eq!((pv["width"].as_u64().unwrap() as u32, pv["height"].as_u64().unwrap() as u32), img.dimensions());
        let seconds = pv["seconds"].as_f64().unwrap();
        assert!((seconds - file.len() as f64 * 8.0 / 56_600.0).abs() < 1e-9, "at 56.6 kbps by default");
    }
    let pv = run(&mut s, "document.exportForWeb.preview", json!({"kbps": 1000}));
    assert!((pv["seconds"].as_f64().unwrap() * 125_000.0 - pv["bytes"].as_f64().unwrap()).abs() < 1e-6);
    assert!(s.execute("document.exportForWeb.preview", &json!({"kbps": 0})).is_err());
}

fn merge(a: &Value, b: Value) -> Value {
    let mut a = a.clone();
    if let (Some(o), Value::Object(b)) = (a.as_object_mut(), b) {
        o.extend(b);
    }
    a
}

#[test]
fn slices_give_one_image_each_and_a_page() {
    let mut s = session(300.0, 200.0);
    rect(&mut s, [0.0, 0.0, 300.0, 200.0], "#336699");
    let a = rect(&mut s, [20.0, 20.0, 100.0, 60.0], "#ff8800");
    run(&mut s, "object.slice.make", json!({"ids": [a]}));
    run(&mut s, "object.slice.options", json!({"name": "logo", "url": "https://example.com/", "alt": "Logo <1>", "target": "_blank"}));
    let b = run(&mut s, "object.slice.create", json!({"x": 150, "y": 100, "width": 100, "height": 50}))["id"].as_u64().unwrap();
    let layout = run(&mut s, "slice.list", json!({}))["slices"].as_array().unwrap().clone();
    let n = layout.len();
    assert!(n > 2, "auto slices fill the rest: {n}");

    let r = run(&mut s, "document.exportForWeb", json!({"format": "gif", "output": "html"}));
    let f = files(&r);
    assert_eq!(f.len(), n + 1, "N images plus the page");
    assert_eq!(f[0].0, "Untitled-1.html");
    assert!(f[1..].iter().all(|(name, bytes)| name.starts_with("images/") && name.ends_with(".gif") && bytes.starts_with(b"GIF")));
    assert!(f.iter().any(|(name, _)| name == "images/logo.gif"));
    let html = String::from_utf8(f[0].1.clone()).unwrap();
    assert_eq!(html.matches("<img ").count(), n);
    assert!(html.contains("<a href=\"https://example.com/\" target=\"_blank\""));
    assert!(html.contains("alt=\"Logo &lt;1&gt;\""));
    assert!(html.contains("width: 300px; height: 200px;"));
    // Each slice image has its slice's pixel size.
    let logo = f.iter().find(|(name, _)| name == "images/logo.gif").unwrap();
    assert_eq!(pixels(&logo.1).dimensions(), (100, 60));
    // The images add up to the whole: every pixel is in one slice.
    let area: u64 = f[1..].iter().map(|(_, b)| pixels(b).dimensions()).map(|(w, h)| u64::from(w) * u64::from(h)).sum();
    assert_eq!(area, 300 * 200);

    // Images only: no page.
    assert_eq!(files(&run(&mut s, "document.exportForWeb", json!({}))).len(), n);
    // A No Image slice is text on the page, not a file.
    run(&mut s, "object.slice.select", json!({"slices": [b]}));
    run(&mut s, "object.slice.options", json!({"kind": "noImage", "text": "<b>Hi</b>", "background": "#ffffff", "hAlign": "center"}));
    let f = files(&run(&mut s, "document.exportForWeb", json!({"output": "html"})));
    assert_eq!(f.len(), n);
    let html = String::from_utf8(f[0].1.clone()).unwrap();
    assert!(html.contains("<b>Hi</b>") && html.contains("background: #ffffff;") && html.contains("justify-content: center"));
    // Selected slices only.
    run(&mut s, "object.slice.select", json!({"slices": [a]}));
    let f = files(&run(&mut s, "document.exportForWeb", json!({"slices": "selected"})));
    assert_eq!(f.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["images/logo.gif"]);
    run(&mut s, "object.slice.select", json!({}));
    assert!(s.execute("document.exportForWeb", &json!({"slices": "selected"})).is_err());
    // No slices: one image.
    assert_eq!(files(&run(&mut s, "document.exportForWeb", json!({"slices": "none"}))).len(), 1);
    assert_eq!(run(&mut s, "document.exportForWeb.preview", json!({}))["slices"], n - 1);
    let one_slice = run(&mut s, "document.exportForWeb.preview", json!({"slice": layout[0]["number"]}));
    assert_eq!((one_slice["width"].as_f64(), one_slice["height"].as_f64()), (layout[0]["width"].as_f64(), layout[0]["height"].as_f64()));
}

/// Save for Web writes the image of a slice named after a Windows device (`aux`, `NUL`, `COM1`…)
/// with a `-` after the device name, on every platform.
#[test]
fn a_slice_named_after_a_windows_device_gets_a_dash() {
    let mut s = session(300.0, 200.0);
    let a = rect(&mut s, [20.0, 20.0, 100.0, 60.0], "#ff8800");
    run(&mut s, "object.slice.make", json!({"ids": [a]}));
    run(&mut s, "object.slice.options", json!({"name": "aux"}));
    let b = run(&mut s, "object.slice.create", json!({"x": 150, "y": 100, "width": 100, "height": 50}))["id"].as_u64().unwrap();
    run(&mut s, "object.slice.select", json!({"slices": [b]}));
    run(&mut s, "object.slice.options", json!({"name": "aux-"}));
    run(&mut s, "object.slice.select", json!({"slices": [a, b]}));
    let mut names: Vec<String> = files(&run(&mut s, "document.exportForWeb", json!({"slices": "selected"}))).into_iter().map(|(n, _)| n).collect();
    names.sort();
    assert_eq!(names, ["images/aux- 2.gif", "images/aux-.gif"]);
}

/// Without a path the files are named after the document's title, with the characters Windows does
/// not allow in a file name as `-` and a `-` after a Windows device name, on every platform.
#[test]
fn files_named_after_the_title_follow_windows_naming_rules() {
    for (title, page, image) in [("a|b?c*\"<d>", "a-b-c---d-.html", "images/a-b-c---d-.gif"), ("nul", "nul-.html", "images/nul-.gif")] {
        let mut s = Session::new();
        run(&mut s, "file.new", json!({"width": 40, "height": 30, "name": title}));
        rect(&mut s, [0.0, 0.0, 20.0, 30.0], "#ff0000");
        let names: Vec<String> = files(&run(&mut s, "document.exportForWeb", json!({"output": "html"}))).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, [page, image], "{title}");
    }
}

#[test]
fn files_are_written_beside_the_path() {
    let dir = vectorcraft_testkit::TempDir::new("webexport");
    let mut s = two_colours();
    let path = dir.join("art.gif").to_string_lossy().to_string();
    let r = run(&mut s, "document.exportForWeb", json!({"path": path}));
    assert_eq!(r["files"], json!([path]));
    assert!(std::fs::read(&path).unwrap().starts_with(b"GIF89a"));
    let page = dir.join("page.html").to_string_lossy().to_string();
    let r = run(&mut s, "document.exportForWeb", json!({"path": page, "output": "html", "format": "png24"}));
    assert_eq!(r["path"], json!(page));
    assert!(std::fs::read_to_string(&page).unwrap().contains("src=\"images/page.png\""));
    assert!(dir.join("images").join("page.png").exists());
    assert_eq!(r["bytes"].as_u64().unwrap(), std::fs::metadata(&page).unwrap().len() + std::fs::metadata(dir.join("images/page.png")).unwrap().len());
}

#[test]
fn an_export_says_when_an_image_could_not_be_decoded() {
    let undecoded =
        |r: &Value| r["warnings"].as_array().is_some_and(|w| w.iter().any(|w| w.as_str().is_some_and(|w| w.contains("couldn't be decoded"))));
    let mut s = two_colours();
    let r = run(&mut s, "document.exportForWeb", json!({}));
    assert_eq!(r["warnings"], json!([]), "{r}");
    assert_eq!(run(&mut s, "document.exportForWeb.preview", json!({}))["warnings"], json!([]));
    // The renderer can't decode the placed image and draws nothing in its place.
    crate::tests_menucmds::place_undecodable_png(&mut s);
    for format in ["gif", "jpg", "png8", "png24"] {
        let r = run(&mut s, "document.exportForWeb", json!({"format": format}));
        assert!(undecoded(&r), "{format}: {}", r["warnings"]);
        let r = run(&mut s, "document.exportForWeb.preview", json!({"format": format}));
        assert!(undecoded(&r), "{format} preview: {}", r["warnings"]);
    }
    let dir = vectorcraft_testkit::TempDir::new("webexport-undecoded");
    let page = dir.join("page.html").to_string_lossy().to_string();
    let r = run(&mut s, "document.exportForWeb", json!({"path": page, "output": "html"}));
    assert!(undecoded(&r), "{r}");
    assert!(dir.join("images").join("page.gif").exists(), "the files are still written");
}

#[test]
fn explicit_artboards_and_unclipped_art_keep_their_pixels_and_dimensions() {
    let mut s = session(60.0, 40.0);
    rect(&mut s, [0.0, 0.0, 60.0, 40.0], "#ff0000");
    run(&mut s, "artboard.new", json!({"x": 100, "y": 0, "width": 32, "height": 24}));
    rect(&mut s, [100.0, 0.0, 32.0, 24.0], "#0000ff");
    for (params, size, color) in [
        (json!({"format": "png24", "artboard": 0}), (60, 40), [255, 0, 0, 255]),
        (json!({"format": "png24", "artboard": 1}), (32, 24), [0, 0, 255, 255]),
        (json!({"format": "png24"}), (60, 40), [255, 0, 0, 255]),
    ] {
        let image = pixels(&one(&mut s, params));
        assert_eq!(image.dimensions(), size);
        assert_eq!(image.get_pixel(8, 8).0, color);
    }
    let full = pixels(&one(&mut s, json!({"format": "png24", "artboard": 1, "clipToArtboard": false})));
    assert_eq!(full.dimensions(), (132, 40));
    assert_eq!(full.get_pixel(8, 8).0, [255, 0, 0, 255]);
    assert_eq!(full.get_pixel(108, 8).0, [0, 0, 255, 255]);
    assert_eq!(full.get_pixel(80, 8).0, [0, 0, 0, 0]);
}

#[test]
fn image_size_region_and_metadata() {
    let mut s = two_colours();
    let size = |s: &mut Session, p: Value| pixels(&one(s, p)).dimensions();
    assert_eq!(size(&mut s, json!({"width": 80})), (80, 60));
    assert_eq!(size(&mut s, json!({"height": 15})), (20, 15));
    assert_eq!(size(&mut s, json!({"percent": 150})), (60, 45));
    // The dialog's fields send decimal numbers: rounded to whole pixels.
    assert_eq!(size(&mut s, json!({"width": 80.0})), (80, 60));
    assert_eq!(size(&mut s, json!({"height": 14.6})), (20, 15));
    for bad in [
        json!({"percent": 0}),
        json!({"width": 0}),
        json!({"width": 0.2}),
        json!({"width": -5}),
        json!({"height": 1e300}),
        json!({"height": "tall"}),
        json!({"format": "bmp"}),
        json!({"colors": "many"}),
        json!({"matte": "plaid"}),
        json!({"matte": "#ffff0080"}),
        json!({"colorTable": {"locked": ["nope"]}}),
        json!({"colorTable": {"transparent": ["#ff000080"]}}),
        json!({"artboard": 3}),
        json!({"preset": "Nope"}),
    ] {
        assert!(s.execute("document.exportForWeb", &bad).is_err(), "{bad}");
    }
    // Clip to Artboard off: the art's bounds.
    let mut t = session(100.0, 100.0);
    rect(&mut t, [10.0, 20.0, 30.0, 15.0], "#000000");
    assert_eq!(size(&mut t, json!({"clipToArtboard": false})), (30, 15));
    assert_eq!(size(&mut t, json!({})), (100, 100));
    // File Info: the copyright by default, everything with all, nothing with none.
    run(&mut s, "file.info", json!({"copyrightNotice": "(c) Someone", "author": "Ann", "description": "Flags"}));
    let png = one(&mut s, json!({"format": "png24"}));
    let has = |f: &[u8], t: &str| f.windows(t.len()).any(|w| w == t.as_bytes());
    assert!(has(&png, "(c) Someone") && !has(&png, "Ann"));
    assert!(has(&png, "sRGB"), "converted to sRGB");
    assert!(!has(&one(&mut s, json!({"format": "png24", "convertToSrgb": false})), "sRGB"));
    let all = one(&mut s, json!({"format": "png8", "metadata": "all"}));
    assert!(has(&all, "Ann") && has(&all, "Flags"));
    assert!(has(&one(&mut s, json!({"format": "png24", "metadata": "contact"})), "Ann"));
    assert!(!has(&one(&mut s, json!({"format": "gif", "metadata": "none"})), "Someone"));
    assert!(has(&one(&mut s, json!({"format": "gif"})), "Copyright: (c) Someone"));
    assert!(has(&one(&mut s, json!({"format": "jpg"})), "Copyright: (c) Someone"));
}

#[test]
fn colour_table_lock_web_shift_snap_and_sort() {
    let mut s = session(30.0, 10.0);
    rect(&mut s, [0.0, 0.0, 10.0, 10.0], "#123456");
    rect(&mut s, [10.0, 0.0, 10.0, 10.0], "#fe0101");
    rect(&mut s, [20.0, 0.0, 10.0, 10.0], "#808080");
    let colors = |s: &mut Session, p: Value| -> Vec<Value> { run(s, "document.exportForWeb.preview", p)["colors"].as_array().unwrap().clone() };
    let base = colors(&mut s, json!({}));
    assert_eq!(base.len(), 3);
    assert!(base.iter().all(|c| c["color"] == c["source"]));
    // Web Shift one colour by its source.
    let shifted = colors(&mut s, json!({"colorTable": {"webShift": ["#123456"]}}));
    let c = shifted.iter().find(|c| c["source"] == "#123456").unwrap();
    assert_eq!((c["color"].as_str(), c["webShifted"].as_bool(), c["webSafe"].as_bool()), (Some("#003366"), Some(true), Some(true)));
    // Web Snap: near colours snap, far ones don't; locked ones never do.
    let snapped = colors(&mut s, json!({"webSnap": 5}));
    assert!(snapped.iter().any(|c| c["source"] == "#fe0101" && c["color"] == "#ff0000"));
    assert!(snapped.iter().any(|c| c["source"] == "#123456" && c["color"] == "#123456"));
    let locked = colors(&mut s, json!({"webSnap": 100, "colorTable": {"locked": ["#fe0101"]}}));
    let c = locked.iter().find(|c| c["source"] == "#fe0101").unwrap();
    assert_eq!((c["color"].as_str(), c["locked"].as_bool()), (Some("#fe0101"), Some(true)));
    // Locked colours survive fewer colours.
    let two = colors(&mut s, json!({"colors": 2, "dither": "none", "colorTable": {"locked": ["#808080"]}}));
    assert!(two.len() <= 2 && two.iter().any(|c| c["color"] == "#808080"), "{two:?}");
    // Sorting by luminance.
    let sorted = colors(&mut s, json!({"colorTable": {"sort": "luminance"}}));
    let names: Vec<&str> = sorted.iter().map(|c| c["color"].as_str().unwrap()).collect();
    assert_eq!(names, ["#123456", "#fe0101", "#808080"]);
    // Lossy makes smaller GIFs of busy art.
    let mut m = many_colours();
    let plain = one(&mut m, json!({"colors": 32})).len();
    let lossy = one(&mut m, json!({"colors": 32, "lossy": 100})).len();
    assert!(lossy < plain, "{lossy} vs {plain}");
}

/// Delete Color: the deleted colors leave the table and the file, their pixels take the nearest
/// color left, and the edit stays with the settings until another reduction is chosen.
#[test]
fn colour_table_delete_maps_pixels_to_the_nearest_color_left() {
    // 40×10: red, a near red and blue, and 10 px with nothing (transparent).
    let mut s = session(40.0, 10.0);
    rect(&mut s, [0.0, 0.0, 10.0, 10.0], "#ff0000");
    rect(&mut s, [10.0, 0.0, 10.0, 10.0], "#ee1100");
    rect(&mut s, [20.0, 0.0, 10.0, 10.0], "#0000ff");
    let preview = |s: &mut Session, p: Value| run(s, "document.exportForWeb.preview", p);
    let sources = |v: &Value| -> Vec<String> {
        let mut out: Vec<String> =
            v["colors"].as_array().unwrap().iter().filter(|c| c["transparent"] == false).map(|c| c["source"].as_str().unwrap().to_string()).collect();
        out.sort();
        out
    };
    let base = preview(&mut s, json!({"dither": "none"}));
    assert_eq!(base["colors"].as_array().unwrap().len(), 4, "{base}");
    for format in ["gif", "png8"] {
        let p = json!({"format": format, "dither": "none", "colorTable": {"deleted": ["#EE1100"]}});
        let v = preview(&mut s, p.clone());
        assert_eq!(v["colors"].as_array().unwrap().len(), 3, "{format}: {v}");
        assert!(v["colors"].as_array().unwrap().iter().any(|c| c["transparent"] == true), "{format}: the transparent entry stays");
        assert_eq!(sources(&v), ["#0000ff", "#ff0000"], "{format}");
        let file = one(&mut s, p);
        assert_eq!(v["bytes"].as_u64(), Some(file.len() as u64), "{format}: the preview is the file");
        let img = pixels(&file);
        assert_eq!(img.get_pixel(15, 5).0, [255, 0, 0, 255], "{format}: the nearest color left");
        assert_eq!((img.get_pixel(5, 5).0, img.get_pixel(25, 5).0, img.get_pixel(35, 5).0[3]), ([255, 0, 0, 255], [0, 0, 255, 255], 0));
        assert_eq!(distinct(&img), 3, "{format}");
    }
    // A color the table does not hold is no edit; sorting does not bring a deleted one back.
    assert_eq!(sources(&preview(&mut s, json!({"dither": "none", "colorTable": {"deleted": ["#123456"]}}))).len(), 3);
    let sorted = preview(&mut s, json!({"dither": "none", "colorTable": {"deleted": ["#ee1100"], "sort": "luminance"}}));
    assert_eq!(sources(&sorted), ["#0000ff", "#ff0000"]);
    // A fixed palette: colors it lacks are not made again, and deletions come out of it.
    let fixed = preview(&mut s, json!({"dither": "none", "colorTable": {"customPalette": ["#0000ff", "#ff0000"]}}));
    assert_eq!(sources(&fixed), ["#0000ff", "#ff0000"]);
    let fixed = preview(&mut s, json!({"dither": "none", "colorTable": {"customPalette": ["#0000ff", "#ff0000"], "deleted": ["#ff0000"]}}));
    assert_eq!(sources(&fixed), ["#0000ff"]);
    // At most `colors` entries of a fixed palette, the transparent one included.
    let two = preview(&mut s, json!({"dither": "none", "colors": 2, "colorTable": {"customPalette": ["#0000ff", "#ff0000"]}}));
    assert_eq!(sources(&two), ["#0000ff"]);
    // Deleting every opaque color is refused: the pixels need a color.
    // Colors mapped to transparent do not count as left.
    for table in [
        json!({"deleted": ["#ff0000", "#ee1100", "#0000ff"]}),
        json!({"customPalette": ["#ff0000"], "deleted": ["#ff0000"]}),
        json!({"deleted": ["#0000ff", "#ee1100"], "transparent": ["#ff0000"]}),
        json!({"customPalette": ["#ff0000", "#0000ff"], "deleted": ["#0000ff"], "transparent": ["#ff0000"]}),
    ] {
        let p = json!({"colorTable": table});
        let e = s.execute("document.exportForWeb.preview", &p).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(e.contains(cmd::webexport::KEEP_ONE), "{p}: {e}");
        assert!(s.execute("document.exportForWeb", &p).is_err(), "{p}");
    }
    for bad in [
        json!({"colorTable": {"deleted": ["nope"]}}),
        json!({"colorTable": {"customPalette": ["#ff000080"]}}),
        json!({"colorTable": {"deleted": vec!["#ff0000"; 257]}}),
    ] {
        assert!(s.execute("document.exportForWeb", &bad).is_err(), "{bad}");
        assert!(s.execute("webExport.settings", &bad).is_err(), "{bad}");
    }
    // A deleted color leaves the other lists; JPEG and PNG-24 have no table to edit.
    let table = json!({
        "deleted": ["#ee1100"], "customPalette": ["#ff0000", "#ee1100", "#0000ff"], "locked": ["#EE1100", "#0000ff"],
        "transparent": ["#ee1100"], "webShift": ["#ee1100"],
    });
    let r = run(&mut s, "webExport.settings", json!({ "colorTable": table }));
    let t = r["settings"]["colorTable"].clone();
    assert_eq!((&t["locked"], &t["transparent"], &t["webShift"]), (&json!(["#0000ff"]), &json!([]), &json!([])));
    assert_eq!(t["deleted"], json!(["#ee1100"]));
    for format in ["jpg", "png24"] {
        assert!(preview(&mut s, json!({"format": format, "colorTable": t.clone()})).get("colors").is_none(), "{format}");
    }
    // The remembered edit survives other keys and a format change, and goes with another reduction.
    let r = run(&mut s, "webExport.settings", json!({"colors": 64, "format": "png8"}));
    assert_eq!(r["settings"]["colorTable"]["deleted"], json!(["#ee1100"]));
    let r = run(&mut s, "webExport.settings", json!({"reduction": "adaptive"}));
    assert_eq!((&r["settings"]["colorTable"]["deleted"], &r["settings"]["colorTable"]["customPalette"]), (&json!([]), &json!([])));
    // A saved preset keeps the edit.
    run(&mut s, "webExport.presets.save", json!({"name": "No near red", "dither": "none", "colorTable": {"deleted": ["#ee1100"]}}));
    assert_eq!(sources(&preview(&mut s, json!({"preset": "No near red"}))), ["#0000ff", "#ff0000"]);
}

/// The colors of a fixed palette mapped to transparent take one slot of Colors together with the
/// transparent pixels, so the opaque colors after them still fit: their pixels stay opaque, and the
/// pixels of the colors mapped to transparent become transparent.
#[test]
fn colour_table_fixed_palette_keeps_an_opaque_color() {
    // 40×10: red, a near red and blue, and 10 px with nothing (transparent).
    let mut s = session(40.0, 10.0);
    rect(&mut s, [0.0, 0.0, 10.0, 10.0], "#ff0000");
    rect(&mut s, [10.0, 0.0, 10.0, 10.0], "#ee1100");
    rect(&mut s, [20.0, 0.0, 10.0, 10.0], "#0000ff");
    for table in [
        json!({"customPalette": ["#ff0000", "#ee1100", "#0000ff"], "deleted": ["#ee1100"], "transparent": ["#ff0000"]}),
        json!({"customPalette": ["#ff0000", "#0000ff"], "transparent": ["#ff0000"]}),
    ] {
        let p = json!({"format": "png8", "dither": "none", "colors": 2, "colorTable": table});
        let v = run(&mut s, "document.exportForWeb.preview", p.clone());
        let opaque: Vec<&Value> = v["colors"].as_array().unwrap().iter().filter(|c| c["transparent"] == false).map(|c| &c["source"]).collect();
        assert_eq!((opaque, v["colors"].as_array().unwrap().len()), (vec![&json!("#0000ff")], 2), "{p}");
        let img = pixels(&one(&mut s, p.clone()));
        let alpha = |x: u32| img.get_pixel(x, 5).0[3];
        assert_eq!((alpha(5), alpha(15), img.get_pixel(25, 5).0, alpha(35)), (0, 0, [0, 0, 255, 255], 0), "{p}");
    }
    // A color mapped to transparent that no pixel takes leaves its slot to an opaque color.
    let mut u = session(20.0, 10.0);
    rect(&mut u, [0.0, 0.0, 10.0, 10.0], "#0000ff");
    rect(&mut u, [10.0, 0.0, 10.0, 10.0], "#00ff00");
    let p = json!({"format": "png8", "dither": "none", "colors": 2, "colorTable": {"customPalette": ["#0000ff", "#00ff00", "#ff0000"], "transparent": ["#ff0000"]}});
    let img = pixels(&one(&mut u, p));
    assert_eq!((img.get_pixel(5, 5).0, img.get_pixel(15, 5).0), ([0, 0, 255, 255], [0, 255, 0, 255]));
    // At 256 colors a color mapped to transparent after 255 opaque ones still keeps an entry.
    let full: Vec<String> =
        std::iter::once("#0000ff".to_string()).chain((1..255).map(|i| format!("#00{i:02x}00"))).chain(["#ff0000".to_string()]).collect();
    let p = json!({"format": "png8", "dither": "none", "colors": 256, "colorTable": {"customPalette": full, "transparent": ["#ff0000"]}});
    let img = pixels(&one(&mut s, p));
    assert_eq!((img.get_pixel(5, 5).0[3], img.get_pixel(25, 5).0, img.get_pixel(35, 5).0[3]), (0, [0, 0, 255, 255], 0));
    // With one opaque color and 255 mapped to transparent, the last of those does not fit, and its
    // pixels still become transparent rather than taking the opaque color nearest them.
    let mut d = session(30.0, 10.0);
    rect(&mut d, [0.0, 0.0, 10.0, 10.0], "#0000ff");
    rect(&mut d, [10.0, 0.0, 10.0, 10.0], "#0000ee");
    let clear: Vec<String> = (1..255).map(|i| format!("#00{i:02x}00")).chain(["#0000ee".to_string()]).collect();
    let palette: Vec<String> = std::iter::once("#0000ff".to_string()).chain(clear.iter().cloned()).collect();
    let p = json!({"format": "png8", "dither": "none", "colors": 256, "colorTable": {"customPalette": palette, "transparent": clear}});
    let v = run(&mut d, "document.exportForWeb.preview", p.clone());
    assert_eq!(v["mappedToTransparent"], json!(["#0000ee"]), "the color that did not fit is listed: {v}");
    let img = pixels(&one(&mut d, p));
    assert_eq!((img.get_pixel(5, 5).0, img.get_pixel(15, 5).0[3], img.get_pixel(25, 5).0[3]), ([0, 0, 255, 255], 0, 0));
    // Without transparent pixels the colors mapped to transparent still take one slot together.
    let mut t = session(30.0, 10.0);
    rect(&mut t, [0.0, 0.0, 10.0, 10.0], "#ff0000");
    rect(&mut t, [10.0, 0.0, 10.0, 10.0], "#ee1100");
    rect(&mut t, [20.0, 0.0, 10.0, 10.0], "#0000ff");
    let p = json!({"format": "png8", "dither": "none", "colors": 2, "colorTable": {
        "customPalette": ["#ff0000", "#ee1100", "#0000ff", "#00ff00"], "transparent": ["#ff0000", "#ee1100"],
    }});
    let v = run(&mut t, "document.exportForWeb.preview", p.clone());
    assert_eq!(v["colors"].as_array().unwrap().len(), 2, "{v}");
    let img = pixels(&one(&mut t, p));
    assert_eq!((img.get_pixel(5, 5).0[3], img.get_pixel(15, 5).0[3], img.get_pixel(25, 5).0), (0, 0, [0, 0, 255, 255]));
}

/// With transparent pixels, the colors of a fixed palette mapped to transparent share the
/// transparent pixels' slot, so at 256 colors all 255 opaque colors keep their entries.
#[test]
fn colour_table_fixed_palette_with_transparent_pixels_keeps_255_opaque_colors() {
    // A 16×16 grid of 2×2 squares in 255 colors, the last cell left empty (transparent).
    let mut s = session(32.0, 32.0);
    let color = |i: usize| format!("#{:02x}{:02x}80", i, 255 - i);
    for i in 0..255 {
        rect(&mut s, [(i % 16) as f64 * 2.0, (i / 16) as f64 * 2.0, 2.0, 2.0], &color(i));
    }
    let palette: Vec<String> = (0..255).map(color).chain(["#000000".to_string()]).collect();
    for table in [json!({"customPalette": palette, "transparent": ["#000000"]}), json!({"customPalette": palette[..255], "transparent": []})] {
        let p = json!({"format": "png8", "dither": "none", "colors": 256, "colorTable": table});
        let img = pixels(&one(&mut s, p.clone()));
        let opaque: std::collections::HashSet<[u8; 4]> = img.pixels().map(|p| p.0).filter(|p| p[3] == 255).collect();
        assert_eq!(opaque.len(), 255, "{p}");
        assert_eq!((img.get_pixel(29, 31).0, img.get_pixel(31, 31).0[3]), ([254, 1, 128, 255], 0), "{p}");
    }
}

/// When Colors leaves a fixed palette fewer slots than it has colors, its locked and shifted colors
/// keep their slots first (shifted ones before locked ones), so a shifted color still shows its new
/// color.
#[test]
fn colour_table_fixed_palette_keeps_locked_and_shifted_colors_first() {
    let mut s = session(30.0, 10.0);
    rect(&mut s, [0.0, 0.0, 10.0, 10.0], "#ff0000");
    rect(&mut s, [10.0, 0.0, 10.0, 10.0], "#00ff00");
    rect(&mut s, [20.0, 0.0, 10.0, 10.0], "#0000ff");
    let palette = json!(["#ff0000", "#00ff00", "#0000ff"]);
    let p = json!({"format": "png8", "dither": "none", "colors": 2, "colorTable": {"customPalette": palette, "shifted": {"#0000ff": "#ff8800"}}});
    let img = pixels(&one(&mut s, p));
    assert_eq!((img.get_pixel(5, 5).0, img.get_pixel(25, 5).0), ([255, 0, 0, 255], [255, 136, 0, 255]));
    // A shifted color keeps its slot before the locked ones.
    let table = json!({"customPalette": palette, "locked": ["#ff0000", "#00ff00"], "shifted": {"#0000ff": "#ff8800"}});
    let img = pixels(&one(&mut s, json!({"format": "png8", "dither": "none", "colors": 2, "colorTable": table})));
    assert_eq!((img.get_pixel(5, 5).0, img.get_pixel(25, 5).0), ([255, 0, 0, 255], [255, 136, 0, 255]));
    let p = json!({"format": "png8", "dither": "none", "colors": 2, "colorTable": {"customPalette": palette, "locked": ["#0000ff"]}});
    let v = run(&mut s, "document.exportForWeb.preview", p);
    let sources: Vec<&Value> = v["colors"].as_array().unwrap().iter().map(|c| &c["source"]).collect();
    assert_eq!(sources, vec![&json!("#ff0000"), &json!("#0000ff")], "the palette keeps its order: {v}");
}

/// A color shifted by hand (`colorTable.shifted`) is written as the color picked wherever it is in
/// the image, the other colors stay, the table shows it as shifted and locked, and the preview is
/// the file.
#[test]
fn colour_table_shift_writes_the_picked_color_wherever_the_source_is() {
    let mut s = session(30.0, 10.0);
    rect(&mut s, [0.0, 0.0, 10.0, 10.0], "#ff0000");
    rect(&mut s, [10.0, 0.0, 10.0, 10.0], "#0000ff");
    rect(&mut s, [20.0, 0.0, 10.0, 10.0], "#00ff00");
    let preview = |s: &mut Session, p: Value| run(s, "document.exportForWeb.preview", p);
    let entry = |v: &Value, source: &str| v["colors"].as_array().unwrap().iter().find(|c| c["source"] == source).cloned().unwrap_or(Value::Null);
    for format in ["gif", "png8"] {
        let p = json!({"format": format, "dither": "none", "colorTable": {"shifted": {"#0000FF": "#FF8800"}}});
        let v = preview(&mut s, p.clone());
        let c = entry(&v, "#0000ff");
        assert_eq!(
            (c["color"].as_str(), c["shifted"].as_bool(), c["locked"].as_bool(), c["webShifted"].as_bool()),
            (Some("#ff8800"), Some(true), Some(true), Some(false)),
            "{format}: {v}"
        );
        assert_eq!((entry(&v, "#ff0000")["shifted"].as_bool(), entry(&v, "#ff0000")["color"].as_str()), (Some(false), Some("#ff0000")));
        let file = one(&mut s, p);
        assert_eq!(v["bytes"].as_u64(), Some(file.len() as u64), "{format}: the preview is the file");
        let img = pixels(&file);
        assert_eq!(
            (img.get_pixel(5, 5).0, img.get_pixel(12, 2).0, img.get_pixel(18, 8).0, img.get_pixel(25, 5).0),
            ([255, 0, 0, 255], [255, 136, 0, 255], [255, 136, 0, 255], [0, 255, 0, 255]),
            "{format}"
        );
        assert_eq!(distinct(&img), 3, "{format}");
    }
    // A shifted color is kept when the colors are reduced, as a locked one is.
    let two = preview(&mut s, json!({"colors": 2, "dither": "none", "colorTable": {"shifted": {"#00ff00": "#123456"}}}));
    assert_eq!(entry(&two, "#00ff00")["color"], "#123456", "{two}");
    // With one slot to keep, the shifted color takes it before a locked one.
    let p = json!({"colors": 2, "dither": "none", "colorTable": {"locked": ["#ff0000"], "shifted": {"#0000ff": "#123456"}}});
    let first = preview(&mut s, p);
    assert_eq!(entry(&first, "#0000ff")["color"], "#123456", "{first}");
    // A source the table does not hold shifts nothing, and the shift stays with the settings.
    let base = preview(&mut s, json!({"dither": "none"}));
    let absent = preview(&mut s, json!({"dither": "none", "colorTable": {"shifted": {"#123456": "#000000"}}}));
    assert_eq!(absent["colors"], base["colors"]);
    // Two colors may be shifted to one color and keep their own sources.
    let same = preview(&mut s, json!({"dither": "none", "colorTable": {"shifted": {"#ff0000": "#123456", "#0000ff": "#123456"}}}));
    assert_eq!((entry(&same, "#ff0000")["color"].as_str(), entry(&same, "#0000ff")["color"].as_str()), (Some("#123456"), Some("#123456")));
    assert_eq!(entry(&same, "#00ff00")["shifted"], false);
    // A shift by hand wins over web shift, web snap and a lock; transparency wins over it.
    let p =
        json!({"dither": "none", "webSnap": 100, "colorTable": {"shifted": {"#0000ff": "#fe0101"}, "webShift": ["#0000ff"], "locked": ["#0000ff"]}});
    let c = entry(&preview(&mut s, p), "#0000ff");
    assert_eq!((c["color"].as_str(), c["webShifted"].as_bool(), c["locked"].as_bool()), (Some("#fe0101"), Some(false), Some(true)));
    let clear = json!({"dither": "none", "colorTable": {"shifted": {"#0000ff": "#fe0101"}, "transparent": ["#0000ff"]}});
    let v = preview(&mut s, clear.clone());
    assert!(v["colors"].as_array().unwrap().iter().all(|c| c["shifted"] == false), "{v}");
    assert_eq!(pixels(&one(&mut s, clear)).get_pixel(15, 5).0[3], 0);
    // The settings keep the shifts in lowercase, drop a color shifted to itself, a deleted one and
    // the web shift of a color shifted by hand.
    let table = json!({"shifted": {"#0000FF": "#FE0101", "#FF0000": "#ff0000", "#00ff00": "#000000"}, "webShift": ["#0000ff", "#00ff00"], "deleted": ["#00ff00"]});
    let r = run(&mut s, "webExport.settings", json!({ "colorTable": table }));
    let t = &r["settings"]["colorTable"];
    assert_eq!((&t["shifted"], &t["webShift"]), (&json!({"#0000ff": "#fe0101"}), &json!([])));
    // Another reduction keeps the shifts; a saved preset keeps them.
    let r = run(&mut s, "webExport.settings", json!({"reduction": "adaptive"}));
    assert_eq!(r["settings"]["colorTable"]["shifted"], json!({"#0000ff": "#fe0101"}));
    run(&mut s, "webExport.presets.save", json!({"name": "Blue to red", "dither": "none", "colorTable": {"shifted": {"#0000ff": "#fe0101"}}}));
    assert_eq!(entry(&preview(&mut s, json!({"preset": "Blue to red"})), "#0000ff")["color"], "#fe0101");
    // Bad shifts are refused and change nothing.
    let many: serde_json::Map<String, Value> = (0..257).map(|i| (format!("#{i:06x}"), json!("#000000"))).collect();
    for bad in [
        json!({"nope": "#000000"}),
        json!({"#ff0000": "nope"}),
        json!({"#ff0000": "#ff000080"}),
        json!({"#ff000080": "#000000"}),
        json!({"#ff0000": "#000000", "#FF0000": "#ffffff"}),
        json!({"#f00": "#000000", "#ff0000": "#ffffff"}),
        Value::Object(many),
    ] {
        let p = json!({"colorTable": {"shifted": bad}});
        assert!(s.execute("document.exportForWeb", &p).is_err(), "{p}");
        assert!(s.execute("document.exportForWeb.preview", &p).is_err(), "{p}");
        assert!(s.execute("webExport.settings", &p).is_err(), "{p}");
    }
    assert_eq!(run(&mut s, "webExport.settings", json!({}))["settings"]["colorTable"]["shifted"], json!({"#0000ff": "#fe0101"}));
    // Settings saved before shifts existed still read.
    let old: cmd::webexport::WebSettings = serde_json::from_value(json!({"colorTable": {"locked": ["#ff0000"]}})).unwrap();
    assert!(old.color_table.shifted.is_empty());
}

/// Lossy GIF compression leaves the pixels of a color shifted by hand alone, so a color shifted near
/// another one does not take in that color's pixels.
#[test]
fn lossy_does_not_spread_a_shifted_color_into_another() {
    let mut s = session(30.0, 10.0);
    rect(&mut s, [0.0, 0.0, 10.0, 10.0], "#0000ff");
    rect(&mut s, [10.0, 0.0, 20.0, 10.0], "#c80000");
    let p = json!({"format": "gif", "dither": "none", "lossy": 100, "colorTable": {"shifted": {"#0000ff": "#be0000"}}});
    let img = pixels(&one(&mut s, p));
    assert_eq!((img.get_pixel(5, 5).0, img.get_pixel(20, 5).0, img.get_pixel(29, 5).0), ([190, 0, 0, 255], [200, 0, 0, 255], [200, 0, 0, 255]));
}

/// Lossy GIF compression moves no pixel into or out of a color shifted by hand: the colors next to
/// a shifted one keep their own pixels, and the shifted color keeps all of its pixels.
#[test]
fn lossy_moves_no_pixel_into_or_out_of_a_shifted_color() {
    let mut s = session(30.0, 10.0);
    rect(&mut s, [0.0, 0.0, 10.0, 10.0], "#1414e6");
    rect(&mut s, [10.0, 0.0, 20.0, 10.0], "#0000ff");
    rect(&mut s, [20.0, 0.0, 30.0, 10.0], "#1414e6");
    let p = json!({"format": "gif", "dither": "none", "lossy": 100, "colorTable": {"shifted": {"#0000ff": "#ff0000"}}});
    let img = pixels(&one(&mut s, p));
    let near_blue = [20, 20, 230, 255];
    for x in 0..30 {
        let want = if (10..20).contains(&x) { [255, 0, 0, 255] } else { near_blue };
        assert_eq!(img.get_pixel(x, 5).0, want, "x = {x}");
    }
}

/// Lossy GIF compression compares the pixels with the colors web snap writes, so a pixel does not
/// take a color that snaps far from it.
#[test]
fn lossy_compares_pixels_with_the_snapped_colors() {
    let mut s = session(20.0, 10.0);
    rect(&mut s, [0.0, 0.0, 10.0, 10.0], "#1a1a1a");
    rect(&mut s, [10.0, 0.0, 20.0, 10.0], "#000000");
    let img = pixels(&one(&mut s, json!({"format": "gif", "dither": "none", "webSnap": 100, "lossy": 50})));
    for x in 0..20 {
        let want = if x < 10 { [51, 51, 51, 255] } else { [0, 0, 0, 255] };
        assert_eq!(img.get_pixel(x, 5).0, want, "x = {x}");
    }
}

#[test]
fn presets_and_remembered_settings() {
    let mut s = two_colours();
    let list = run(&mut s, "webExport.presets.list", json!({}));
    let names: Vec<&str> = list["presets"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert!(names.len() >= 6 && list["presets"].as_array().unwrap().iter().all(|p| p["builtIn"] == true));
    // A built-in preset as a starting point.
    let jpg = one(&mut s, json!({"preset": "JPEG, quality 80"}));
    assert_eq!(&jpg[..2], &[0xFF, 0xD8]);
    assert!(one(&mut s, json!({"preset": "jpeg, QUALITY 80", "quality": 10})).len() < jpg.len(), "keys win over the preset");

    let r = run(&mut s, "webExport.presets.save", json!({"name": "Mine", "format": "png8", "colors": 4}));
    assert_eq!((r["created"].as_bool(), r["settings"]["colors"].as_u64()), (Some(true), Some(4)));
    let r = run(&mut s, "webExport.presets.save", json!({"name": "mine", "settings": {"interlaced": true}}));
    assert_eq!((r["created"].as_bool(), r["settings"]["colors"].as_u64(), r["settings"]["interlaced"].as_bool()), (Some(false), Some(4), Some(true)));
    let png = one(&mut s, json!({"preset": "Mine"}));
    assert_eq!((png[25], png[28]), (3, 1), "indexed and interlaced");
    run(&mut s, "webExport.presets.save", json!({"name": "Mine", "newName": "Ours"}));
    assert!(s.execute("webExport.presets.save", &json!({"name": "PNG-24"})).is_err(), "built in");
    assert!(s.execute("webExport.presets.save", &json!({"name": "Ours", "newName": "PNG-24"})).is_err(), "taken");
    assert!(s.execute("webExport.presets.save", &json!({})).is_err());
    assert!(s.execute("webExport.presets.delete", &json!({"name": "PNG-24"})).is_err());
    // Saved presets are preferences (not Preferences dialog keys) and survive a reset.
    let saved: Prefs = serde_json::from_value(s.prefs.to_json()).unwrap();
    assert_eq!(saved, s.prefs, "round trip");
    assert_eq!(s.prefs.to_json()["webExportPresets"][0]["name"], "Ours");
    assert!(Prefs::default().to_json().get("webExportPresets").is_none() && Prefs::default().to_json().get("webExportSettings").is_none());
    run(&mut s, "prefs.reset", json!({}));
    assert_eq!(s.prefs.web_export_presets.len(), 1);
    assert_eq!(run(&mut s, "webExport.presets.delete", json!({"name": "ours"}))["deleted"], "Ours");
    assert!(s.execute("webExport.presets.delete", &json!({"name": "Ours"})).is_err());

    // The dialog's remembered settings.
    assert_eq!(run(&mut s, "webExport.settings", json!({}))["settings"], defaults());
    assert!(s.prefs.web_export_settings.is_none(), "reading stores nothing");
    run(&mut s, "webExport.settings", json!({"format": "jpg", "quality": 30}));
    let r = run(&mut s, "webExport.settings", json!({"progressive": true}));
    assert_eq!(
        (r["settings"]["format"].as_str(), r["settings"]["quality"].as_u64(), r["settings"]["progressive"].as_bool()),
        (Some("jpg"), Some(30), Some(true))
    );
    assert_eq!(run(&mut s, "webExport.settings", json!({"preset": "PNG-24"}))["settings"]["format"], "png24");
    assert_eq!(run(&mut s, "webExport.settings", json!({"reset": true}))["settings"], defaults());
    assert!(s.execute("webExport.settings", &json!({"quality": 500})).is_err());
}

#[test]
fn setting_keys_are_their_json_values() {
    use cmd::webexport::{SliceScope, WebFormat, WebMetadata, WebOutput};
    for f in WebFormat::ALL {
        assert_eq!(serde_json::to_value(f).unwrap(), json!(f.id()));
    }
    for m in WebMetadata::ALL {
        assert_eq!(serde_json::to_value(m).unwrap(), json!(m.key()));
    }
    for s in SliceScope::ALL {
        assert_eq!(serde_json::to_value(s).unwrap(), json!(s.key()));
    }
    for o in WebOutput::ALL {
        assert_eq!(serde_json::to_value(o).unwrap(), json!(o.key()));
    }
    // Every setting round-trips through JSON (presets, the remembered settings).
    let s = Session::new();
    let custom = s
        .web_settings(
            &json!({"format": "png8", "reduction": "web", "dither": "pattern", "antiAlias": "type", "colorTable": {"sort": "hue"}, "percent": 50}),
        )
        .unwrap();
    assert_eq!(serde_json::from_value::<cmd::webexport::WebSettings>(custom.to_json()).unwrap(), custom);
}

fn optimize_to(s: &mut Session, p: Value) -> Value {
    run(s, "webExport.optimizeToFileSize", p)
}

/// The size of the file the settings `p` write (the whole image, or slice `slice`).
fn written(s: &mut Session, p: &Value) -> usize {
    run(s, "document.exportForWeb.preview", p.clone())["bytes"].as_u64().unwrap() as usize
}

/// 20×20: 400 one-point squares of different colors (more than a palette holds).
fn photo_like() -> Session {
    let mut s = session(20.0, 20.0);
    for i in 0..400 {
        let (x, y) = ((i % 20) as f64, (i / 20) as f64);
        rect(&mut s, [x, y, 1.0, 1.0], &format!("#{:02x}{:02x}{:02x}", (i % 20) * 12, (i / 20) * 12, (i * 7) % 256));
    }
    s
}

#[test]
fn optimize_to_file_size_takes_the_highest_jpeg_quality_that_fits() {
    let mut s = many_colours();
    let start = json!({"format": "jpg", "quality": 90});
    let full = written(&mut s, &start);
    let k = (full * 3 / 4) as f64 / 1024.0;
    let r = optimize_to(&mut s, merge(&start, json!({"desiredK": k})));
    let target = r["targetBytes"].as_u64().unwrap() as usize;
    assert_eq!(target, (k * 1024.0).floor() as usize);
    assert_eq!((r["achieved"].as_bool(), r["format"].as_str(), r["images"].as_u64()), (Some(true), Some("jpg"), Some(1)), "{r}");
    let q = r["settings"]["quality"].as_u64().unwrap();
    assert!(q < 90, "{r}");
    let bytes = r["bytes"].as_u64().unwrap() as usize;
    assert!(bytes <= target, "{r}");
    // The size is the size of the file Save writes with the settings.
    assert_eq!(written(&mut s, &r["settings"]), bytes);
    assert_eq!(one(&mut s, r["settings"].clone()).len(), bytes);
    // One quality step up is over the size.
    assert!(written(&mut s, &merge(&r["settings"], json!({"quality": q + 1}))) > target);
    assert!(r["trials"].as_u64().unwrap() as usize <= cmd::webexport::OPTIMIZE_TRIALS);
    // The other settings stay.
    assert_eq!(r["settings"]["matte"], json!("white"));
    assert_eq!(r["settings"]["metadata"], json!("copyright"));
}

#[test]
fn optimize_to_file_size_lowers_palette_colors() {
    let mut s = many_colours();
    for format in ["gif", "png8"] {
        let start = json!({"format": format, "dither": "none", "colors": 128});
        let full = written(&mut s, &start);
        let r = optimize_to(&mut s, merge(&start, json!({"desiredK": (full * 3 / 4) as f64 / 1024.0})));
        let target = r["targetBytes"].as_u64().unwrap() as usize;
        assert_eq!((r["achieved"].as_bool(), r["format"].as_str()), (Some(true), Some(format)), "{r}");
        let colors = r["settings"]["colors"].as_u64().unwrap();
        assert!(colors < 64, "{format}: {r}");
        assert_eq!((r["settings"]["lossy"].as_u64(), r["settings"]["dither"].as_str()), (Some(0), Some("none")), "{format}: {r}");
        let bytes = r["bytes"].as_u64().unwrap() as usize;
        assert!(bytes <= target && written(&mut s, &r["settings"]) == bytes, "{format}: {r}");
        assert!(written(&mut s, &merge(&r["settings"], json!({"colors": colors + 1}))) > target, "{format}: {r}");
    }
}

#[test]
fn optimize_to_file_size_auto_selects_gif_or_jpeg_by_content() {
    // Flat, opaque art: GIF, from a JPEG start.
    let mut flat = two_colours();
    let r = optimize_to(&mut flat, json!({"format": "jpg", "desiredK": 100, "startWith": "auto"}));
    assert_eq!((r["format"].as_str(), r["achieved"].as_bool()), (Some("gif"), Some(true)), "{r}");
    // More colors than a palette holds: JPEG, from a GIF start.
    let mut photo = photo_like();
    let r = optimize_to(&mut photo, json!({"format": "gif", "desiredK": 100, "startWith": "auto"}));
    assert_eq!((r["format"].as_str(), r["achieved"].as_bool()), (Some("jpg"), Some(true)), "{r}");
    // Transparent pixels: GIF, which keeps them.
    let mut clear = session(40.0, 30.0);
    rect(&mut clear, [0.0, 0.0, 10.0, 10.0], "#000000");
    for (k, transparency, format) in [(100.0, true, "gif"), (100.0, false, "gif"), (0.01, true, "gif")] {
        let r = optimize_to(&mut clear, json!({"format": "jpg", "desiredK": k, "startWith": "auto", "transparency": transparency}));
        assert_eq!(r["format"].as_str(), Some(format), "{k} {transparency}: {r}");
    }
    // The current format stays without auto.
    let r = optimize_to(&mut photo, json!({"format": "png8", "desiredK": 100}));
    assert_eq!(r["format"].as_str(), Some("png8"));
}

#[test]
fn optimize_to_file_size_auto_counts_the_colors_blended_over_the_matte() {
    // Three colors at 100 opacities: without transparency, 300 colors once blended over the matte.
    let mut s = session(20.0, 15.0);
    for i in 0..300 {
        let (x, y) = ((i % 20) as f64, (i / 20) as f64);
        let id = rect(&mut s, [x, y, 1.0, 1.0], ["#000000", "#ff0000", "#0000ff"][i % 3]);
        run(&mut s, "transparency.set", json!({"ids": [id], "opacity": i / 3 + 1}));
    }
    let r = optimize_to(&mut s, json!({"format": "gif", "desiredK": 100, "startWith": "auto", "transparency": false, "antiAlias": "none"}));
    assert_eq!((r["format"].as_str(), r["achieved"].as_bool()), (Some("jpg"), Some(true)), "{r}");
    // With transparency the pixels stay transparent: GIF.
    let r = optimize_to(&mut s, json!({"format": "jpg", "desiredK": 100, "startWith": "auto", "antiAlias": "none"}));
    assert_eq!(r["format"].as_str(), Some("gif"), "{r}");
}

#[test]
fn optimize_to_file_size_says_when_no_setting_fits() {
    let mut s = many_colours();
    for format in ["jpg", "gif", "png8"] {
        let start = json!({"format": format});
        let r = optimize_to(&mut s, merge(&start, json!({"desiredK": 0.01})));
        assert_eq!((r["targetBytes"].as_u64(), r["achieved"].as_bool(), r["format"].as_str()), (Some(10), Some(false), Some(format)), "{r}");
        let bytes = r["bytes"].as_u64().unwrap() as usize;
        assert!(bytes > 10 && bytes <= written(&mut s, &start), "{format}: the smallest setting tried: {r}");
        assert_eq!(written(&mut s, &r["settings"]), bytes);
        assert!(r["trials"].as_u64().unwrap() as usize <= cmd::webexport::OPTIMIZE_TRIALS);
    }
    // GIF tries lossy when 2 colors are over.
    let r = optimize_to(&mut s, json!({"format": "gif", "desiredK": 0.01}));
    assert_eq!(r["settings"]["lossy"], json!(100), "{r}");
    // PNG-24 has nothing to lower.
    let r = optimize_to(&mut s, json!({"format": "png24", "desiredK": 0.01}));
    assert_eq!((r["achieved"].as_bool(), r["trials"].as_u64()), (Some(false), Some(1)), "{r}");
    assert_eq!(r["settings"], s.web_settings(&json!({"format": "png24"})).unwrap().to_json());
    // Settings that fit stay as they are.
    let p = json!({"format": "jpg", "quality": 80, "desiredK": 1000});
    let r = optimize_to(&mut s, p.clone());
    assert_eq!((r["achieved"].as_bool(), r["trials"].as_u64()), (Some(true), Some(1)), "{r}");
    assert_eq!(r["settings"], s.web_settings(&p).unwrap().to_json());
}

#[test]
fn optimize_to_file_size_refuses_bad_params_and_changes_nothing() {
    let mut s = many_colours();
    run(&mut s, "webExport.settings", json!({"format": "jpg", "quality": 77}));
    let remembered = s.last_web_settings();
    for p in [
        json!({}),
        json!({"desiredK": 0}),
        json!({"desiredK": -5}),
        json!({"desiredK": "10"}),
        json!({"desiredK": null}),
        json!({"desiredK": 2_000_000}),
        json!({"desiredK": 10, "startWith": "gif"}),
        json!({"desiredK": 10, "startWith": 1}),
        json!({"desiredK": 10, "use": "all"}),
        json!({"desiredK": 10, "slice": 0}),
        json!({"desiredK": 10, "slice": "1"}),
        json!({"desiredK": 10, "slice": 1, "use": "eachSlice"}),
        json!({"desiredK": 10, "slice": 99}),
        json!({"desiredK": 10, "percent": 0}),
    ] {
        assert!(s.execute("webExport.optimizeToFileSize", &p).is_err(), "{p}");
    }
    optimize_to(&mut s, json!({"desiredK": 0.5, "format": "jpg"}));
    assert_eq!(s.last_web_settings(), remembered, "the search changes no remembered setting");
}

#[test]
fn optimize_to_file_size_measures_the_slices_as_use_says() {
    let mut s = session(300.0, 200.0);
    rect(&mut s, [0.0, 0.0, 300.0, 200.0], "#336699");
    for i in 0..40 {
        rect(&mut s, [f64::from(i) * 7.0, f64::from(i) * 4.0, 30.0, 20.0], &format!("#{:02x}{:02x}40", i * 6, 255 - i * 6));
    }
    let a = rect(&mut s, [20.0, 20.0, 100.0, 60.0], "#ff8800");
    run(&mut s, "object.slice.make", json!({"ids": [a]}));
    let layout = run(&mut s, "slice.list", json!({}))["slices"].as_array().unwrap().clone();
    let start = json!({"format": "jpg", "quality": 95});
    let sizes = |s: &mut Session, p: &Value| files(&run(s, "document.exportForWeb", p.clone())).iter().map(|(_, b)| b.len()).collect::<Vec<_>>();
    let all = sizes(&mut s, &start);
    assert_eq!(all.len(), layout.len());
    let total: usize = all.iter().sum();
    let r = optimize_to(&mut s, merge(&start, json!({"desiredK": (total * 3 / 4) as f64 / 1024.0})));
    assert_eq!((r["achieved"].as_bool(), r["images"].as_u64()), (Some(true), Some(layout.len() as u64)), "{r}");
    let after = sizes(&mut s, &r["settings"]);
    assert_eq!(after.iter().sum::<usize>(), r["bytes"].as_u64().unwrap() as usize, "the slice files together");
    let largest = *all.iter().max().unwrap();
    let r = optimize_to(&mut s, merge(&start, json!({"desiredK": (largest * 3 / 4) as f64 / 1024.0, "use": "eachSlice"})));
    let after = sizes(&mut s, &r["settings"]);
    assert_eq!(*after.iter().max().unwrap(), r["bytes"].as_u64().unwrap() as usize, "the largest slice file");
    assert!(r["achieved"].as_bool().unwrap() && after.iter().all(|n| *n <= r["targetBytes"].as_u64().unwrap() as usize), "{r}");
    let n = layout[0]["number"].as_u64().unwrap();
    let one_slice = written(&mut s, &merge(&start, json!({"slice": n})));
    let r = optimize_to(&mut s, merge(&start, json!({"desiredK": (one_slice / 2) as f64 / 1024.0, "slice": n})));
    assert_eq!(r["images"].as_u64(), Some(1));
    assert_eq!(written(&mut s, &merge(&r["settings"], json!({"slice": n}))), r["bytes"].as_u64().unwrap() as usize, "{r}");
}

#[test]
fn optimize_to_file_size_finds_a_fit_when_a_lower_setting_gives_a_larger_file() {
    use cmd::webexport::{OPTIMIZE_SAMPLES, WebSettings, highest_within, set_quality};
    // Sizes that do not fall with the quality: 1 to 20 give 900 bytes, 21 to 70 give 400 to 449,
    // and 71 and up give over 1000.
    let size_of = |q: u8| match q {
        ..=20 => 900,
        21..=70 => 400 + usize::from(q - 21),
        _ => 1000 + usize::from(q),
    };
    let start = WebSettings { quality: 90, ..WebSettings::default() };
    let mut measured = Vec::new();
    let mut smallest = None;
    let mut size = |s: &WebSettings| {
        measured.push(s.quality);
        Ok(Some(size_of(s.quality)))
    };
    let found = highest_within(&mut size, &start, (1, 90), set_quality, 500, &mut smallest).unwrap();
    let (settings, bytes) = found.expect("a quality within 500 bytes");
    assert_eq!((settings.quality, bytes), (70, 449), "measured {measured:?}");
    assert!(measured.len() <= usize::from(OPTIMIZE_SAMPLES) + 7, "measured {measured:?}");
    // Nothing fits: the smallest size measured is kept.
    let mut smallest = None;
    let mut size = |s: &WebSettings| Ok(Some(size_of(s.quality)));
    assert!(highest_within(&mut size, &start, (1, 90), set_quality, 100, &mut smallest).unwrap().is_none());
    assert_eq!(smallest.map(|(s, n)| (s.quality, n)), Some((36, 415)));
}

/// The bytes of a `document.exportForWeb.saveColorTable` reply without a path.
fn color_table_file(s: &mut Session, p: Value) -> (Value, Vec<u8>) {
    let v = run(s, "document.exportForWeb.saveColorTable", p);
    let bytes = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    (v, bytes)
}

/// The color table file of `colors` (`#rrggbb`) and transparent entry `transparent`, as the
/// published layout gives it: 256 RGB triples (zero past the colors used), then the number of
/// colors and the transparent index (0xFFFF for none), two bytes each, most significant first.
fn act_layout(colors: &[&str], transparent: Option<u16>) -> Vec<u8> {
    let mut out = vec![];
    for c in colors {
        let rgb = u32::from_str_radix(c.trim_start_matches('#'), 16).unwrap();
        out.extend_from_slice(&[(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]);
    }
    out.resize(768, 0);
    out.extend_from_slice(&(colors.len() as u16).to_be_bytes());
    out.extend_from_slice(&transparent.unwrap_or(0xFFFF).to_be_bytes());
    out
}

/// Save Color Table writes the table the preview shows as a 772-byte color table file: the colors
/// as written, in file order, zero bytes after them, the count and the transparent entry.
#[test]
fn save_color_table_writes_the_act_layout() {
    let mut s = two_colours();
    for format in ["gif", "png8"] {
        let p = json!({"format": format, "dither": "none", "transparency": false});
        let pv = run(&mut s, "document.exportForWeb.preview", p.clone());
        let colors: Vec<&str> = pv["colors"].as_array().unwrap().iter().map(|c| c["color"].as_str().unwrap()).collect();
        assert_eq!(colors.len(), 2, "{pv}");
        let (v, bytes) = color_table_file(&mut s, p);
        assert_eq!(bytes, act_layout(&colors, None), "{format}");
        assert_eq!((v["colors"].as_u64(), v["transparentIndex"].is_null(), v["bytes"].as_u64()), (Some(2), true, Some(772)), "{v}");
    }
    // Transparent pixels: the trailer names the transparent entry.
    let mut s = session(40.0, 30.0);
    rect(&mut s, [10.0, 5.0, 10.0, 20.0], "#ff0000");
    let p = json!({"format": "gif", "antiAlias": "none", "dither": "none"});
    let pv = run(&mut s, "document.exportForWeb.preview", p.clone());
    let table = pv["colors"].as_array().unwrap();
    let at = table.iter().position(|c| c["transparent"] == true).expect("a transparent entry") as u16;
    let colors: Vec<&str> = table.iter().map(|c| c["color"].as_str().unwrap()).collect();
    let (v, bytes) = color_table_file(&mut s, p);
    assert_eq!(bytes, act_layout(&colors, Some(at)));
    assert_eq!(v["transparentIndex"].as_u64(), Some(u64::from(at)));
}

/// A color shifted by hand goes into the file as the color it is written as.
#[test]
fn save_color_table_writes_shifted_colors_as_written() {
    let mut s = two_colours();
    let p = json!({"format": "gif", "dither": "none", "transparency": false, "colorTable": {"shifted": {"#ff0000": "#123456"}}});
    let (_, bytes) = color_table_file(&mut s, p);
    let colors: Vec<[u8; 3]> = bytes[..6].chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
    assert!(colors.contains(&[0x12, 0x34, 0x56]) && !colors.contains(&[255, 0, 0]), "{colors:?}");
}

/// Save Color Table writes the file at `path`, adding `.act` to a name without an extension and
/// keeping another extension, and refuses formats without a color table and bad params.
#[test]
fn save_color_table_writes_the_path_and_refuses_bad_params() {
    let dir = std::env::temp_dir().join(format!("vc-color-table-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = two_colours();
    let p = json!({"format": "gif", "dither": "none"});
    let (_, bytes) = color_table_file(&mut s, p.clone());
    let bare = dir.join("table").to_string_lossy().into_owned();
    let v = run(&mut s, "document.exportForWeb.saveColorTable", merge(&p, json!({"path": bare})));
    let written = format!("{bare}.act");
    assert_eq!(v["path"].as_str(), Some(written.as_str()));
    assert_eq!(std::fs::read(&written).unwrap(), bytes);
    let other = dir.join("table.tbl").to_string_lossy().into_owned();
    run(&mut s, "document.exportForWeb.saveColorTable", merge(&p, json!({"path": other})));
    assert_eq!(std::fs::read(&other).unwrap(), bytes);
    for (bad, says) in [
        (json!({"format": "jpg"}), "`jpg` has no color table"),
        (json!({"format": "png24"}), "`png24` has no color table"),
        (json!({"path": 5}), "`path` must be a string"),
        (json!({"path": ""}), "`path` is empty"),
        (json!({"path": "  "}), "`path` is empty"),
        (json!({"slice": "1"}), "`slice` is a slice number"),
        (json!({"slice": 0}), "`slice` is a slice number"),
    ] {
        let e = s.execute("document.exportForWeb.saveColorTable", &bad).unwrap_err().to_string();
        assert!(e.contains(says), "{bad}: {e}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// With Save for Web set to the selected slices and none selected, Save Color Table still writes
/// the whole image's table, the one the dialog's preview shows. A `slice` is still looked up among
/// the selected slices.
#[test]
fn save_color_table_of_the_whole_image_needs_no_selected_slice() {
    let mut s = two_colours();
    let slice = run(&mut s, "object.slice.create", json!({"x": 0, "y": 0, "width": 20, "height": 30}))["id"].as_u64().unwrap();
    run(&mut s, "object.slice.select", json!({}));
    let p = json!({"format": "gif", "dither": "none", "transparency": false});
    let (_, whole) = color_table_file(&mut s, merge(&p, json!({"slices": "none"})));
    let (v, bytes) = color_table_file(&mut s, merge(&p, json!({"slices": "selected"})));
    assert_eq!(bytes, whole, "{v}");
    let number = run(&mut s, "slice.list", json!({}))["slices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"].as_u64() == Some(slice))
        .map(|l| l["number"].clone())
        .unwrap();
    let e = s.execute("document.exportForWeb.saveColorTable", &merge(&p, json!({"slices": "selected", "slice": number}))).unwrap_err().to_string();
    assert!(e.contains("no selected slice"), "{e}");
}

/// The preview refuses a `slice` that is not a slice number instead of previewing the whole image.
#[test]
fn preview_refuses_a_slice_that_is_not_a_number() {
    let mut s = two_colours();
    for slice in [json!("1"), json!(0), json!(-1), json!(1.5)] {
        let e = s.execute("document.exportForWeb.preview", &json!({"slice": slice})).unwrap_err().to_string();
        assert!(e.contains("`slice` is a slice number"), "{slice}: {e}");
    }
    assert!(s.execute("document.exportForWeb.preview", &json!({"slice": null})).is_ok());
}

/// Color table files are read as the published layout gives them: 768 bytes are 256 colors without
/// a transparent entry; 772 bytes add the count and the transparent index.
#[test]
fn color_table_files_are_read_by_the_published_layout() {
    use cmd::webexport::read_act;
    let long: Vec<u8> = (0..768).map(|i| (i % 251) as u8).collect();
    let t = read_act(&long).unwrap();
    assert_eq!((t.colors.len(), t.colors[0], t.colors[255], t.transparent), (256, [0, 1, 2], [12, 13, 14], None));
    let t = read_act(&act_layout(&["#ff0000", "#00ff00", "#0000ff"], Some(1))).unwrap();
    assert_eq!((t.colors, t.transparent), (vec![[255, 0, 0], [0, 255, 0], [0, 0, 255]], Some(1)));
    assert_eq!(read_act(&act_layout(&["#ff0000", "#00ff00"], None)).unwrap().transparent, None);
    // An index past the colors used names no entry.
    assert_eq!(read_act(&act_layout(&["#ff0000", "#00ff00"], Some(2))).unwrap().transparent, None);
    let mut zero = act_layout(&["#ff0000"], None);
    zero[768..770].copy_from_slice(&0u16.to_be_bytes());
    let mut over = act_layout(&["#ff0000"], None);
    over[768..770].copy_from_slice(&257u16.to_be_bytes());
    for bad in [vec![], vec![0; 767], vec![0; 769], vec![0; 771], vec![0; 773], zero, over] {
        assert!(read_act(&bad).unwrap_err().contains("not a color table"), "{} bytes", bad.len());
    }
}

/// Load Color Table makes the file's colors the fixed palette, other than its transparent entry,
/// with the file's color count; the preview of the loaded settings writes those colors.
#[test]
fn load_color_table_makes_the_file_the_fixed_palette() {
    let mut s = two_colours();
    let file = act_layout(&["#00ff00", "#ffffff", "#ff0000"], Some(1));
    let table = json!({
        "deleted": ["#0000ff"], "shifted": {"#ff0000": "#111111"}, "webShift": ["#ff0000"], "locked": ["#ff0000"],
        "transparent": ["#00ff00"], "sort": "hue",
    });
    let p = json!({"format": "gif", "dither": "none", "transparency": false, "webSnap": 100, "colorTable": table});
    let v = run(&mut s, "webExport.loadColorTable", merge(&p, json!({"dataBase64": vectorcraft_format::base64_encode(&file)})));
    assert_eq!((&v["colors"], &v["transparentIndex"]), (&json!(["#00ff00", "#ffffff", "#ff0000"]), &json!(1)));
    let settings = &v["settings"];
    assert_eq!((&settings["colors"], &settings["webSnap"]), (&json!(3), &json!(0)));
    let table = &settings["colorTable"];
    assert_eq!(
        (&table["customPalette"], &table["deleted"], &table["shifted"], &table["webShift"]),
        (&json!(["#00ff00", "#ff0000"]), &json!([]), &json!({}), &json!([]))
    );
    assert_eq!((&table["locked"], &table["transparent"], &table["sort"]), (&json!([]), &json!([]), &json!("none")));
    // The loaded palette is what the image is written with: blue takes the nearest loaded color.
    let pv = run(&mut s, "document.exportForWeb.preview", settings.clone());
    let written: Vec<&str> = pv["colors"].as_array().unwrap().iter().map(|c| c["color"].as_str().unwrap()).collect();
    assert!(!written.is_empty() && written.iter().all(|c| ["#00ff00", "#ff0000"].contains(c)), "{pv}");
    // A table saved and loaded again keeps its colors.
    let opaque = json!({"format": "png8", "dither": "none", "transparency": false});
    let saved = color_table_file(&mut s, opaque.clone()).1;
    let again = run(&mut s, "webExport.loadColorTable", json!({"format": "png8", "dataBase64": vectorcraft_format::base64_encode(&saved)}));
    let shown = run(&mut s, "document.exportForWeb.preview", opaque);
    let before: Vec<&Value> = shown["colors"].as_array().unwrap().iter().map(|c| &c["color"]).collect();
    assert_eq!(again["colors"].as_array().unwrap().iter().collect::<Vec<_>>(), before);
    // A table whose only color is its transparent entry loads that color.
    let only = act_layout(&["#336699"], Some(0));
    let v = run(&mut s, "webExport.loadColorTable", json!({"dataBase64": vectorcraft_format::base64_encode(&only)}));
    assert_eq!((&v["settings"]["colorTable"]["customPalette"], &v["settings"]["colors"]), (&json!(["#336699"]), &json!(2)));
}

/// Load Color Table reads a path, reads no more of a file than a color table takes, and refuses
/// bad params without changing the remembered settings.
#[test]
fn load_color_table_reads_a_path_and_refuses_bad_params() {
    let dir = std::env::temp_dir().join(format!("vc-color-table-load-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = two_colours();
    let path = dir.join("t.act").to_string_lossy().into_owned();
    std::fs::write(&path, act_layout(&["#010203", "#040506"], None)).unwrap();
    let v = run(&mut s, "webExport.loadColorTable", json!({"path": path}));
    assert_eq!(v["settings"]["colorTable"]["customPalette"], json!(["#010203", "#040506"]));
    let large = dir.join("large.act").to_string_lossy().into_owned();
    std::fs::write(&large, vec![0u8; 1 << 20]).unwrap();
    let b64 = vectorcraft_format::base64_encode(&act_layout(&["#010203"], None));
    for (bad, says) in [
        (json!({"path": large}), "not a color table"),
        (json!({"dataBase64": vectorcraft_format::base64_encode(&[0u8; 100])}), "not a color table"),
        // A whole table with more data after it is too long.
        (json!({"dataBase64": format!("{b64}AAAA")}), "not a color table"),
        (json!({}), "give exactly one of `path` and `dataBase64`"),
        (json!({"path": path, "dataBase64": b64}), "give exactly one of `path` and `dataBase64`"),
        (json!({"dataBase64": "%%%"}), "bad dataBase64"),
        (json!({"dataBase64": b64, "format": "jpg"}), "`jpg` has no color table"),
    ] {
        let e = s.execute("webExport.loadColorTable", &bad).unwrap_err().to_string();
        assert!(e.contains(says), "{bad}: {e}");
    }
    assert!(s.last_web_settings().color_table.custom_palette.is_empty(), "nothing is remembered");
    let _ = std::fs::remove_dir_all(dir);
}
