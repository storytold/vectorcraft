#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Production preflight must find actual delivery problems and clear them after repairs.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

fn new() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({ "width": 500, "height": 400 })).unwrap();
    s
}

fn check(s: &mut Session) -> Value {
    s.execute("document.preflight", &json!({})).unwrap()
}

fn codes(report: &Value) -> Vec<&str> {
    report["findings"].as_array().unwrap().iter().map(|f| f["code"].as_str().unwrap()).collect()
}

#[test]
fn preflight_finds_four_problems_and_repairs_clear_each_one() {
    let dir = Folder::new("preflight-four");
    let path = dir.file("picture.png");
    let bytes = png(40, 40, [200, 50, 10]);
    std::fs::write(&path, &bytes).unwrap();
    let mut s = new();
    let text = s.execute("text.create", &json!({ "x": 10, "y": 10, "text": "Ab\nCd\nEf\nGh", "font": "Preflight Missing Family 917", "size": 12, "area": { "width": 100, "height": 20 } })).unwrap()["id"].clone();
    let image = s.execute("file.place", &json!({ "path": path, "at": [200, 200], "link": true })).unwrap()["ids"][0].clone();
    std::fs::remove_file(&path).unwrap();
    let before_doc = s.doc().unwrap().doc.clone();
    let before_sel = s.doc().unwrap().selection.clone();
    let revision = s.doc().unwrap().revision;
    let report = check(&mut s);
    for code in ["missingFont", "overflow", "missingLink", "lowResolution"] {
        assert!(codes(&report).contains(&code), "{code}: {report}");
    }
    assert_eq!(*before_doc, *s.doc().unwrap().doc);
    assert_eq!(before_sel, s.doc().unwrap().selection);
    assert_eq!(revision, s.doc().unwrap().revision, "checking doesn't edit the document");
    for f in report["findings"].as_array().unwrap() {
        assert!(f["ids"].as_array().unwrap().iter().all(|id| id == &text || id == &image), "{f}");
    }
    s.execute("text.areaOptions", &json!({ "ids": [text], "height": 150 })).unwrap();
    assert!(!codes(&check(&mut s)).contains(&"overflow"));
    std::fs::write(&path, &bytes).unwrap();
    s.execute("links.update", &json!({ "ids": [image] })).unwrap();
    assert!(!codes(&check(&mut s)).contains(&"missingLink"));
    s.execute("select.set", &json!({ "ids": [image] })).unwrap();
    s.execute("object.setBounds", &json!({ "width": 7.2, "height": 7.2 })).unwrap();
    assert!(!codes(&check(&mut s)).contains(&"lowResolution"));
    // Deleting the missing-font object fixes the final delivery issue on any machine, including
    // builds with no optional craft-fonts checkout or installed fonts.
    s.execute("select.set", &json!({ "ids": [text] })).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    let clean = check(&mut s);
    assert!(codes(&clean).is_empty(), "{clean}");
    assert!(clean["checks"].as_array().unwrap().iter().all(|c| c["status"] == "passed"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(codes(&check(&mut s)).contains(&"missingFont"), "undo is checked too");
}

#[test]
fn preflight_reuses_link_probes_for_external_changes_without_an_edit() {
    let dir = Folder::new("preflight-modified");
    let path = dir.file("picture.png");
    std::fs::write(&path, png(40, 40, [200, 50, 10])).unwrap();
    let mut s = new();
    let image = s.execute("file.place", &json!({ "path": path, "at": [200, 200] })).unwrap()["ids"][0].clone();
    let rev = s.doc().unwrap().revision;
    std::fs::write(&path, png(41, 45, [10, 50, 200])).unwrap();
    assert!(codes(&check(&mut s)).contains(&"modifiedLink"));
    assert_eq!(rev, s.doc().unwrap().revision);
    s.execute("links.update", &json!({ "ids": [image] })).unwrap();
    assert!(!codes(&check(&mut s)).contains(&"modifiedLink"));
}

#[test]
fn preflight_scopes_artboards_and_respects_hidden_and_nonprinting_layers() {
    let mut s = new();
    let id = s.execute("text.create", &json!({ "x": 10, "y": 30, "text": "Label", "font": "Preflight Missing Scope 917" })).unwrap()["id"].clone();
    s.execute("artboard.new", &json!({ "x": 600, "y": 0, "width": 500, "height": 400 })).unwrap();
    let other = s.execute("document.preflight", &json!({ "artboards": [1] })).unwrap();
    assert!(codes(&other).is_empty(), "{other}");
    assert!(codes(&check(&mut s)).contains(&"missingFont"));
    s.execute("object.setProps", &json!({ "ids": [id], "visible": false })).unwrap();
    assert!(codes(&check(&mut s)).is_empty());
    s.execute("object.setProps", &json!({ "ids": [id], "visible": true, "locked": true })).unwrap();
    assert!(codes(&check(&mut s)).contains(&"missingFont"), "locked artwork still exports");
    let layer = s.doc().unwrap().doc.layers[0].id;
    s.execute("layer.setProps", &json!({ "id": layer.0, "printable": false })).unwrap();
    let print = s.execute("document.preflight", &json!({ "output": "print" })).unwrap();
    assert!(codes(&print).is_empty(), "{print}");
}

#[test]
fn preflight_resolution_follows_placed_size_and_both_axes() {
    let dir = Folder::new("preflight-resolution");
    let path = dir.file("picture.png");
    std::fs::write(&path, png(100, 100, [200, 50, 10])).unwrap();
    let mut s = new();
    let id = s.execute("file.place", &json!({ "path": path, "link": false, "at": [200, 200] })).unwrap()["ids"][0].clone();
    s.execute("select.set", &json!({ "ids": [id] })).unwrap();
    s.execute("object.setBounds", &json!({ "width": 24, "height": 72 })).unwrap();
    let r = check(&mut s);
    let f = r["findings"].as_array().unwrap().iter().find(|f| f["code"] == "lowResolution").unwrap();
    assert_eq!(f["effectivePpi"], json!([300.0, 100.0]));
    let screen = s.execute("document.preflight", &json!({ "output": "screen" })).unwrap();
    assert_eq!(screen["minPpi"], 96.0);
    assert!(codes(&screen).is_empty(), "{screen}");
}

#[test]
fn preflight_resolution_accepts_the_exact_threshold_but_rejects_a_real_shortfall() {
    let dir = Folder::new("preflight-resolution-boundary");
    let path = dir.file("picture.png");
    std::fs::write(&path, png(40, 40, [200, 50, 10])).unwrap();
    let mut s = new();
    s.execute("file.place", &json!({ "path": path, "link": false, "at": [200, 200] })).unwrap();
    for threshold in [31.0, 59.0, 300.0, 600.0, 9600.0] {
        let size = 40.0 * 72.0 / threshold;
        s.execute("object.setBounds", &json!({ "width": size, "height": size })).unwrap();
        let exact = s.execute("document.preflight", &json!({ "minPpi": threshold })).unwrap();
        assert!(!codes(&exact).contains(&"lowResolution"), "exactly {threshold} ppi: {exact}");
        s.execute("object.setBounds", &json!({ "width": size * 1.000001, "height": size })).unwrap();
        let below = s.execute("document.preflight", &json!({ "minPpi": threshold })).unwrap();
        let f = below["findings"].as_array().unwrap().iter().find(|f| f["code"] == "lowResolution").unwrap();
        assert_eq!(f["threshold"], threshold);
    }
}

#[test]
fn preflight_resolution_includes_the_print_jobs_scale() {
    let dir = Folder::new("preflight-print-scale");
    let path = dir.file("picture.png");
    std::fs::write(&path, png(100, 100, [200, 50, 10])).unwrap();
    let mut s = new();
    s.execute("file.place", &json!({ "path": path, "link": false, "at": [250, 200] })).unwrap();
    s.execute("object.setBounds", &json!({ "width": 24, "height": 24 })).unwrap();
    let doc = s.execute("document.preflight", &json!({ "minPpi": 200 })).unwrap();
    assert!(codes(&doc).is_empty(), "{doc}");
    s.execute("print.setup", &json!({ "settings": { "scaling": "custom", "scale": { "width": 200, "height": 200 } } })).unwrap();
    let print = s.execute("document.preflight", &json!({ "output": "print", "minPpi": 200 })).unwrap();
    let f = print["findings"].as_array().unwrap().iter().find(|f| f["code"] == "lowResolution").unwrap();
    assert_eq!(f["effectivePpi"], json!([150.0, 150.0]));
}

#[test]
fn preflight_reports_a_frame_that_cannot_show_even_its_first_line() {
    let mut s = new();
    let id = s.execute("text.create", &json!({ "x": 10, "y": 10, "text": "Abc", "size": 30, "area": { "width": 100, "height": 1 } })).unwrap()["id"]
        .clone();
    let r = check(&mut s);
    assert!(r["findings"].as_array().unwrap().iter().any(|f| f["code"] == "overflow" && f["ids"] == json!([id])), "{r}");
}

#[test]
fn preflight_reports_type_that_cannot_place_a_glyph_on_a_horizontal_path() {
    let mut s = new();
    let path = s.execute("path.create", &json!({ "d": "M10 20 L11 20" })).unwrap()["id"].clone();
    let id = s.execute("text.createInPath", &json!({ "path": path, "mode": "onPath", "text": "Longer than the path" })).unwrap()["id"].clone();
    let r = check(&mut s);
    assert!(r["findings"].as_array().unwrap().iter().any(|f| f["code"] == "overflow" && f["ids"] == json!([id])), "{r}");
}

#[test]
fn preflight_bad_options_leave_document_and_selection_unchanged() {
    let mut s = new();
    let doc = s.doc().unwrap().doc.clone();
    for p in [
        json!({ "output": "unknown" }),
        json!({ "output": 1 }),
        json!({ "minPpi": 0 }),
        json!({ "minPpi": "300" }),
        json!({ "artboards": [] }),
        json!({ "artboards": [0.5] }),
        json!({ "artboards": [999] }),
    ] {
        assert!(s.execute("document.preflight", &p).is_err(), "{p}");
        assert_eq!(*doc, *s.doc().unwrap().doc);
    }
}

#[test]
fn preflight_resource_font_finding_locates_its_instance() {
    let mut s = new();
    let text = s.execute("text.create", &json!({ "x": 10, "y": 30, "text": "Label", "font": "Preflight Missing Symbol 917" })).unwrap()["id"].clone();
    s.execute("select.set", &json!({ "ids": [text] })).unwrap();
    s.execute("symbol.new", &json!({ "name": "Label" })).unwrap();
    let r = check(&mut s);
    let f = r["findings"].as_array().unwrap().iter().find(|f| f["code"] == "missingFont").unwrap();
    assert!(f["ids"].as_array().unwrap().iter().all(|id| s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).is_some()), "{r}");
}

struct Folder(std::path::PathBuf);
impl Folder {
    fn new(name: &str) -> Self {
        Self(vectorcraft_testkit::temp_dir(name))
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
fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let pixels = (0..w * h).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect();
    let image =
        craft_image_codecs::Image::from_raw(w, h, craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, pixels).unwrap();
    craft_image_codecs::encode(&image, craft_image_codecs::Format::Png, &Default::default()).unwrap()
}
