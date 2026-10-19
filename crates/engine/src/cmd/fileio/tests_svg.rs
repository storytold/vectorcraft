//! SVG Options through the engine: every option changes the output, artboards write one file
//! each, junk is rejected, and Save writes SVG with remembered options.

use serde_json::{Value, json};

use super::*;

fn session(artboards: usize) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": artboards})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10.123456, "y": 10, "width": 50, "height": 40})).unwrap();
    s
}

pub(super) fn svg(s: &mut Session, p: Value) -> String {
    let mut p = p;
    p["format"] = json!("svg");
    s.execute("document.serialize", &p).unwrap_or_else(|e| panic!("{p}: {e}"))["text"].as_str().unwrap().to_string()
}

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vc-svgopts-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A session whose document holds one embedded 2×2 PNG.
pub(super) fn image_session() -> Session {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 200, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "dot.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
    s
}

#[test]
fn every_option_changes_the_output() {
    let mut s = session(1);
    let plain = svg(&mut s, json!({}));
    assert!(plain.contains("fill=\"#") && plain.contains(" width=\"200\"") && plain.contains("id=\"Layer_1\""), "{plain}");
    let changed = |s: &mut Session, p: Value, check: &dyn Fn(&str) -> bool| {
        let out = svg(s, p.clone());
        assert_ne!(out, plain, "{p} changed nothing");
        assert!(check(&out), "{p}:\n{out}");
    };
    changed(&mut s, json!({"styling": "style"}), &|o| o.contains("style=\"fill:#"));
    changed(&mut s, json!({"styling": "entities"}), &|o| o.contains("<!ENTITY st1 ") && o.contains("style=\"&st1;\""));
    changed(&mut s, json!({"svg": {"styling": "css"}}), &|o| o.contains("<style>") && o.contains("class=\"cls-1\""));
    changed(&mut s, json!({"objectIds": "minimal"}), &|o| !o.contains(" id=\""));
    changed(&mut s, json!({"objectIds": "unique"}), &|o| o.contains(" id=\"u") && o.contains("-Layer_1\""));
    changed(&mut s, json!({"decimals": 1}), &|o| o.contains("M10.1 ") && !o.contains("10.123"));
    changed(&mut s, json!({"decimals": 6}), &|o| o.contains("10.123456"));
    changed(&mut s, json!({"minify": true}), &|o| !o.trim_end().contains('\n') && !o.contains("<?xml"));
    changed(&mut s, json!({"responsive": true}), &|o| !o.contains(" width=\"200\"") && o.contains("viewBox=\"0 0 200 100\""));
    changed(&mut s, json!({"useArtboards": false}), &|o| !o.contains("viewBox=\"0 0 200 100\""));
    changed(&mut s, json!({"metadata": true}), &|o| o.contains("<metadata>") && o.contains("<dc:format>image/svg+xml</dc:format>"));
    changed(&mut s, json!({"preserveEditing": true}), &|o| o.contains(vectorcraft_svg::EDITING_NS));
    s.execute("text.create", &json!({"x": 20, "y": 80, "text": "Hi"})).unwrap();
    let with_text = svg(&mut s, json!({}));
    assert!(with_text.contains("<text"));
    assert!(!svg(&mut s, json!({"outlineText": true})).contains("<text"), "Fonts → outlines");
    // The nested object wins over the top level.
    assert!(svg(&mut s, json!({"styling": "style", "svg": {"styling": "css"}})).contains("<style>"));

    let mut s = image_session();
    assert!(svg(&mut s, json!({})).contains("href=\"data:image/png;base64,"));
    let r = s.execute("document.serialize", &json!({"format": "svg", "images": "link"})).unwrap();
    let name = r["linked"][0]["name"].as_str().unwrap().to_string();
    assert!(name.ends_with(".png") && r["text"].as_str().unwrap().contains(&format!("href=\"{name}\"")), "{r}");
}

#[test]
fn junk_options_are_rejected() {
    let mut s = session(1);
    for p in [
        json!({"styling": "fancy"}),
        json!({"objectIds": true}),
        json!({"decimals": 0}),
        json!({"decimals": 8}),
        json!({"svg": {"bogus": 1}}),
        json!({"svg": "css"}),
        json!({"images": "inline"}),
        json!({"useArtboards": "yes"}),
    ] {
        let mut q = p.clone();
        q["format"] = json!("svg");
        assert!(s.execute("document.export", &q).is_err(), "{p} was accepted");
    }
    // Other formats ignore the SVG options.
    assert!(s.execute("document.export", &json!({"format": "png", "styling": "fancy"})).is_ok());
}

#[test]
fn several_artboards_write_one_file_each() {
    let mut s = session(3);
    s.execute("artboard.setProps", &json!({"index": 0, "name": "Cover"})).unwrap();
    let dir = tmp_dir("boards");
    let path = dir.join("art.svg").to_string_lossy().to_string();
    let r = s.execute("document.export", &json!({"path": path, "range": "all"})).unwrap();
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    let names: Vec<&str> = files.iter().map(|f| f.rsplit(['/', '\\']).next().unwrap()).collect();
    assert_eq!(names, ["art-Cover.svg", "art-Artboard-2.svg", "art-Artboard-3.svg"]);
    for f in &files {
        assert!(std::fs::read_to_string(f).unwrap().contains("viewBox=\"0 0 200 100\""), "{f}");
    }
    let r = s.execute("document.export", &json!({"format": "svg", "svg": {"range": "2-3"}})).unwrap();
    assert_eq!(r["files"].as_array().unwrap().len(), 2, "{r}");
    let r = s.execute("document.serialize", &json!({"format": "svg", "range": "all"})).unwrap();
    assert_eq!(r["files"].as_array().unwrap().len(), 3);
    assert!(r["files"][2]["text"].as_str().unwrap().starts_with("<?xml"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn linked_images_are_written_next_to_the_svg() {
    let mut s = image_session();
    let dir = tmp_dir("linked");
    let path = dir.join("pic.svg").to_string_lossy().to_string();
    let r = s.execute("document.export", &json!({"path": path, "images": "link"})).unwrap();
    let linked = r["linked"][0].as_str().unwrap();
    assert!(image::load_from_memory(&std::fs::read(linked).unwrap()).is_ok(), "{linked}");
    let text = std::fs::read_to_string(&path).unwrap();
    let name = std::path::Path::new(linked).file_name().unwrap().to_string_lossy().to_string();
    assert!(text.contains(&format!("href=\"{name}\"")), "{text}");
    assert!(s.execute("document.exportSelection", &json!({"format": "svg", "images": "link"})).is_err(), "nowhere to put the images");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn preserve_editing_reopens_the_native_document() {
    let mut s = session(2);
    s.execute("artboard.setProps", &json!({"index": 1, "name": "Back"})).unwrap();
    let text = svg(&mut s, json!({"preserveEditing": true}));
    let r = s.execute("document.open", &json!({"name": "both.svg", "dataBase64": vectorcraft_format::base64_encode(text.as_bytes())})).unwrap();
    assert_eq!(r["format"], "svg");
    let doc = &s.doc().unwrap().doc;
    assert_eq!(doc.artboards.len(), 2, "both artboards came back");
    assert_eq!(doc.artboards[1].name, "Back");
    assert_eq!(doc.title, "both.svg");
    // Without it the SVG is imported (one artboard).
    let text = svg(&mut s, json!({}));
    s.execute("document.open", &json!({"name": "plain.svg", "dataBase64": vectorcraft_format::base64_encode(text.as_bytes())})).unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards.len(), 1);
}

#[test]
fn saving_several_artboards_as_plain_svg_warns_they_are_lost() {
    let artboards = |r: &Value| r["warnings"].as_array().unwrap().iter().filter(|w| w.as_str().unwrap().contains("artboards")).count();
    let mut s = session(3);
    let r = s.execute("document.save", &json!({"format": "svg"})).unwrap();
    assert_eq!(artboards(&r), 1, "{r}");
    // First: the status bar shows only the first warning.
    assert!(r["warnings"][0].as_str().unwrap().contains("1 of the document's 3 artboards"), "{r}");
    assert_eq!(artboards(&s.execute("file.saveAs", &json!({"format": "svgz"})).unwrap()), 1);
    // Preserve Editing keeps them all (they reopen); a single artboard loses none.
    for p in [json!({"format": "svg", "preserveEditing": true}), json!({"format": "svg", "svg": {"preserveEditing": true}})] {
        let r = s.execute("document.save", &p).unwrap();
        assert_eq!(artboards(&r), 0, "{p}: {r}");
    }
    assert_eq!(artboards(&session(1).execute("document.save", &json!({"format": "svg"})).unwrap()), 0);
}

#[test]
fn save_writes_svg_and_remembers_its_options() {
    let mut s = session(1);
    let dir = tmp_dir("save");
    let path = dir.join("doc.svg").to_string_lossy().to_string();
    let r = s.execute("document.save", &json!({"path": path, "svg": {"styling": "css", "decimals": 2}})).unwrap();
    assert_eq!(r["path"], json!(path));
    assert!(std::fs::read_to_string(&path).unwrap().contains("<style>"));
    let st = s.doc().unwrap();
    assert_eq!(st.path.as_deref(), Some(path.as_str()));
    assert!(!st.is_dirty());
    s.execute("shape.ellipse", &json!({"x": 100, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("document.save", &json!({})).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("<style>") && text.matches("<path").count() == 2, "Save reuses the options: {text}");
    assert!(s.execute("document.save", &json!({"path": dir.join("x.png").to_string_lossy()})).is_err(), "PNG is an export");
    // A native save switches back.
    let native = dir.join("doc.vectorcraft").to_string_lossy().to_string();
    s.execute("document.save", &json!({"path": native})).unwrap();
    assert!(vectorcraft_format::sniff(&std::fs::read(&native).unwrap()));
    assert!(s.doc().unwrap().save_options.is_empty(), "a native save remembers no options");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn formats_list_the_svg_options_with_their_defaults() {
    let mut s = Session::new();
    let r = s.execute("document.formats", &json!({})).unwrap();
    let svg = r["formats"].as_array().unwrap().iter().find(|f| f["id"] == "svg").unwrap();
    let defaults = serde_json::to_value(vectorcraft_svg::ExportOptions::default()).unwrap();
    for (k, v) in defaults.as_object().unwrap() {
        assert_eq!(&svg["options"][k]["default"], v, "{k}");
    }
    for k in ["useArtboards", "range", "svg"] {
        assert!(svg["options"].get(k).is_some(), "{k}");
    }
}

#[test]
fn fewer_tspans_reaches_the_writer() {
    let mut s = Session::new();
    let src = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><text x="10" y="40" font-size="12">plain <tspan fill="#ff0000">red</tspan> plain</text></svg>"##;
    s.execute("document.open", &json!({"name": "two-runs.svg", "dataBase64": vectorcraft_format::base64_encode(src.as_bytes())})).unwrap();
    let placed = |svg: &str| svg.matches("<tspan x=").count();
    assert_eq!(placed(&svg(&mut s, json!({}))), 3, "a tspan per style run");
    assert_eq!(placed(&svg(&mut s, json!({"fewerTspans": true}))), 1, "a tspan per line");
}

/// Gunzip SVGZ bytes.
fn gunzip(bytes: &[u8]) -> String {
    assert!(vectorcraft_svg::is_svgz(bytes), "gzip magic");
    vectorcraft_svg::text_of(bytes).unwrap().into_owned()
}

#[test]
fn svgz_is_the_svg_export_gzipped() {
    let mut s = session(2);
    let opts = json!({"styling": "css", "range": "2"});
    let mut p = opts.clone();
    p["format"] = json!("svgz");
    let svgz = vectorcraft_format::base64_decode(s.execute("document.serialize", &p).unwrap()["dataBase64"].as_str().unwrap()).unwrap();
    assert_eq!(gunzip(&svgz), svg(&mut s, opts));
    // The format comes from the extension too, and Save writes it.
    let dir = tmp_dir("svgz");
    let path = dir.join("art.svgz").to_string_lossy().to_string();
    s.execute("document.export", &json!({"path": path, "range": "all"})).unwrap();
    assert!(gunzip(&std::fs::read(dir.join("art-Artboard-1.svgz")).unwrap()).contains("<svg"));
    let saved = dir.join("saved.svgz").to_string_lossy().to_string();
    s.execute("document.save", &json!({"path": saved, "svg": {}})).unwrap();
    assert!(gunzip(&std::fs::read(&saved).unwrap()).contains("<svg"));
    assert_eq!(s.doc().unwrap().path.as_deref(), Some(saved.as_str()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn svgz_opens_and_pastes_like_svg() {
    let mut s = session(1);
    s.execute("shape.ellipse", &json!({"x": 100, "y": 20, "width": 40, "height": 30})).unwrap();
    let text = svg(&mut s, json!({}));
    let open = |s: &mut Session, name: &str, bytes: &[u8]| {
        s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)})).unwrap();
        let mut doc = (*s.doc().unwrap().doc).clone();
        doc.title.clear();
        vectorcraft_format::save(&doc, false)
    };
    let plain = open(&mut s, "a.svg", text.as_bytes());
    // Detected by the magic bytes whatever the name says.
    assert_eq!(open(&mut s, "a.svg", &vectorcraft_svg::compress(&text)), plain);
    assert_eq!(open(&mut s, "a.svgz", &vectorcraft_svg::compress(&text)), plain);
    let r = s.execute("clipboard.importSvg", &json!({"dataBase64": vectorcraft_format::base64_encode(&vectorcraft_svg::compress(&text))})).unwrap();
    assert_eq!(r["count"], 2);
    assert!(s.execute("document.open", &json!({"name": "bad.svgz", "dataBase64": vectorcraft_format::base64_encode(&[0x1f, 0x8b, 1, 2])})).is_err());
}
