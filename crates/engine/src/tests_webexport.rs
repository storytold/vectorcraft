//! File → Export → Save for Web (Legacy): `document.exportForWeb`, its preview, the remembered
//! settings and the presets.

use serde_json::{Value, json};

use super::*;

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
    image::load_from_memory(file).unwrap().to_rgba8()
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

#[test]
fn files_are_written_beside_the_path() {
    let dir = vectorcraft_testkit::temp_dir("webexport");
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
        json!({"colorTable": {"locked": ["nope"]}}),
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
