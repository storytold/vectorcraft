//! Export for Icon Composer: a file per visible layer, the artboard's size, numbered from the back,
//! in a folder per document and a sub-folder per artboard.

use serde_json::{Value, json};

use super::*;

const C: &str = "document.exportForIconComposer";

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// `(name, bytes)` of each returned file.
fn files(r: &Value) -> Vec<(String, Vec<u8>)> {
    r["files"].as_array().unwrap().iter().map(|f| (f["name"].as_str().unwrap().to_string(), b64(f))).collect()
}

fn names(r: &Value) -> Vec<String> {
    files(r).into_iter().map(|(n, _)| n).collect()
}

fn new_layer(s: &mut Session, name: &str) {
    s.execute("layer.new", &json!({"name": name})).unwrap();
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) {
    s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
}

/// A 100 × 100 pt artboard; layer `Back` (the first) holds a square at the top left, layer `Front`
/// one at the bottom right.
fn two_layers() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let back = s.doc().unwrap().doc.layers[0].id.0;
    s.execute("layer.setProps", &json!({"id": back, "name": "Back"})).unwrap();
    rect(&mut s, 5.0, 5.0, 40.0, 40.0);
    new_layer(&mut s, "Front");
    rect(&mut s, 55.0, 55.0, 40.0, 40.0);
    s
}

fn alpha(png: &[u8], x: u32, y: u32) -> u8 {
    image::load_from_memory(png).unwrap().to_rgba8().get_pixel(x, y)[3]
}

#[test]
fn each_layer_is_a_file_the_artboards_size_numbered_from_the_back() {
    let mut s = two_layers();
    let r = s.execute(C, &json!({"format": "png"})).unwrap();
    assert_eq!(names(&r), ["1.back.png", "2.front.png"]);
    let f = files(&r);
    for (_, png) in &f {
        assert_eq!(image::load_from_memory(png).unwrap().to_rgba8().dimensions(), (100, 100));
    }
    // Each file holds its own layer's art alone, on a transparent background.
    assert_eq!((alpha(&f[0].1, 25, 25), alpha(&f[0].1, 75, 75)), (255, 0));
    assert_eq!((alpha(&f[1].1, 25, 25), alpha(&f[1].1, 75, 75)), (0, 255));
    assert_eq!(r["folder"], json!(super::file_stem(&s.doc().unwrap().doc.title)));
}

#[test]
fn svg_is_the_default_and_keeps_the_artboards_bounds() {
    let mut s = two_layers();
    let r = s.execute(C, &json!({})).unwrap();
    assert_eq!(names(&r), ["1.back.svg", "2.front.svg"]);
    for (_, svg) in files(&r) {
        let svg = String::from_utf8(svg).unwrap();
        assert!(svg.contains("viewBox=\"0 0 100 100\""), "{svg}");
    }
}

#[test]
fn hidden_template_and_empty_layers_and_art_off_the_artboard_are_left_out() {
    let mut s = two_layers();
    new_layer(&mut s, "Empty");
    new_layer(&mut s, "Off the artboard");
    rect(&mut s, 300.0, 300.0, 10.0, 10.0);
    new_layer(&mut s, "Hidden");
    rect(&mut s, 10.0, 10.0, 10.0, 10.0);
    let hidden = s.doc().unwrap().doc.layers.last().unwrap().id.0;
    s.execute("layer.setProps", &json!({"id": hidden, "visible": false})).unwrap();
    new_layer(&mut s, "Template");
    rect(&mut s, 10.0, 10.0, 10.0, 10.0);
    let template = s.doc().unwrap().doc.layers.last().unwrap().id.0;
    s.execute("layer.setProps", &json!({"id": template, "template": true})).unwrap();
    new_layer(&mut s, "Top");
    rect(&mut s, 10.0, 10.0, 10.0, 10.0);
    // An object of the top layer off the artboard stays out of its file.
    rect(&mut s, 500.0, 500.0, 10.0, 10.0);
    let r = s.execute(C, &json!({})).unwrap();
    assert_eq!(names(&r), ["1.back.svg", "2.front.svg", "3.top.svg"]);
    let top = String::from_utf8(files(&r).remove(2).1).unwrap();
    assert!(!top.contains("500"), "{top}");
}

#[test]
fn several_artboards_get_a_sub_folder_each_and_a_range_picks_them() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 50, "height": 50, "artboards": 3})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 0, "name": "Light: Mode"})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "name": "light: mode"})).unwrap();
    let boards: Vec<_> = s.doc().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    for b in &boards {
        rect(&mut s, b.x0 + 5.0, b.y0 + 5.0, 10.0, 10.0);
    }
    let r = s.execute(C, &json!({})).unwrap();
    // Names that differ only in case get a number; `:` can't be in a file name.
    assert_eq!(names(&r), ["Light- Mode/1.layer 1.svg", "light- mode-2/1.layer 1.svg", "Artboard 3/1.layer 1.svg"]);
    let r = s.execute(C, &json!({"range": "2-3"})).unwrap();
    assert_eq!(names(&r), ["light- mode/1.layer 1.svg", "Artboard 3/1.layer 1.svg"]);
    // An artboard with no art of a layer writes nothing for it.
    s.execute("layer.new", &json!({"name": "Second"})).unwrap();
    rect(&mut s, boards[1].x0 + 1.0, boards[1].y0 + 1.0, 5.0, 5.0);
    let r = s.execute(C, &json!({"artboards": [0, 1]})).unwrap();
    assert_eq!(names(&r), ["Light- Mode/1.layer 1.svg", "light- mode-2/1.layer 1.svg", "light- mode-2/2.second.svg"]);
}

#[test]
fn ten_layers_or_more_get_numbers_of_one_width_so_names_sort_in_stacking_order() {
    let mut s = two_layers();
    for i in 3..=12 {
        new_layer(&mut s, &format!("L{i}"));
        rect(&mut s, 10.0, 10.0, 10.0, 10.0);
    }
    let names = names(&s.execute(C, &json!({})).unwrap());
    assert_eq!(names.len(), 12);
    assert_eq!([&names[0], &names[1], &names[9], &names[11]], ["01.back.svg", "02.front.svg", "10.l10.svg", "12.l12.svg"]);
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(sorted, names, "a plain sort by name keeps the stacking order");
}

#[test]
fn an_artboard_picked_twice_is_exported_once() {
    let mut s = two_layers();
    let r = s.execute(C, &json!({"artboards": [0, 0]})).unwrap();
    assert_eq!(names(&r), ["1.back.svg", "2.front.svg"]);
}

#[test]
fn an_object_off_the_artboard_whose_shadow_reaches_it_is_kept() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let r = s.doc().unwrap().doc.artboards[0].rect;
    let id = s.execute("shape.rectangle", &json!({"x": r.x0 - 30.0, "y": r.y0 + 10.0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    assert!(s.execute(C, &json!({})).unwrap_err().to_string().contains("nothing to export"), "the plain rectangle is off the artboard");
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [id], "params": {"x": 20, "y": 0, "blur": 0}})).unwrap();
    let r = s.execute(C, &json!({"format": "png"})).unwrap();
    let f = files(&r);
    assert_eq!(f.len(), 1, "its shadow reaches 10 pt onto the artboard");
    assert!(alpha(&f[0].1, 5, 20) > 0, "the shadow is drawn");
    assert_eq!(alpha(&f[0].1, 50, 50), 0);
}

#[test]
fn layer_names_are_made_safe_for_file_names() {
    let mut s = two_layers();
    let front = s.doc().unwrap().doc.layers[1].id.0;
    s.execute("layer.setProps", &json!({"id": front, "name": "..Glass/Shine: TOP"})).unwrap();
    let r = s.execute(C, &json!({})).unwrap();
    assert_eq!(names(&r), ["1.back.svg", "2.glass-shine- top.svg"]);
}

#[test]
fn bad_params_and_nothing_to_export_are_errors() {
    let mut s = two_layers();
    assert!(s.execute(C, &json!({"format": "pdf"})).unwrap_err().to_string().contains("svg or png"));
    assert!(s.execute(C, &json!({"range": "4"})).is_err());
    assert!(s.execute(C, &json!({"fromat": "png"})).is_err(), "unknown keys are refused");
    let mut empty = Session::new();
    empty.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    assert!(empty.execute(C, &json!({})).unwrap_err().to_string().contains("nothing to export"));
}

#[test]
fn zip_holds_the_folder() {
    let mut s = two_layers();
    let r = s.execute(C, &json!({"zip": true})).unwrap();
    let title = super::file_stem(&s.doc().unwrap().doc.title);
    assert_eq!(r["name"], json!(format!("{title}.zip")));
    assert_eq!(r["files"], json!(["1.back.svg", "2.front.svg"]));
    // Read back, every entry's CRC and size checked: the folder's files, the same bytes as
    // exported without the ZIP.
    let entries = super::zip::entries(&b64(&r)).unwrap();
    let plain = files(&s.execute(C, &json!({})).unwrap());
    let expected: Vec<(String, Vec<u8>)> = plain.into_iter().map(|(n, b)| (format!("{title}/{n}"), b)).collect();
    assert_eq!(entries, expected, "entries are inside the folder");
}

/// A temporary folder this test made itself (never one already there), removed when the test
/// ends, failing or not.
#[cfg(not(target_arch = "wasm32"))]
pub(super) struct TempFolder(pub(super) std::path::PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl TempFolder {
    pub(super) fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        for i in 0..1000 {
            let dir = std::env::temp_dir().join(format!("vc-iconcomposer-{name}-{}-{nanos}-{i}", std::process::id()));
            if std::fs::create_dir(&dir).is_ok() {
                return Self(dir);
            }
        }
        panic!("no new temporary folder for {name}");
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_folder_gets_a_new_folder_named_after_the_document_each_time() {
    let dir = TempFolder::new("each-time");
    let parent = dir.0.to_string_lossy().to_string();
    let mut s = two_layers();
    let title = super::file_stem(&s.doc().unwrap().doc.title);
    let r = s.execute(C, &json!({"folder": parent})).unwrap();
    let first = format!("{parent}/{title}");
    assert_eq!(r["folder"], json!(first));
    assert_eq!(r["files"], json!([format!("{first}/1.back.svg"), format!("{first}/2.front.svg")]));
    assert!(std::fs::read_to_string(format!("{first}/2.front.svg")).unwrap().contains("<svg"));
    // Exporting again never writes into the first folder: the new one has the date and time.
    let r = s.execute(C, &json!({"folder": parent})).unwrap();
    let second = r["folder"].as_str().unwrap().to_string();
    assert_ne!(second, first);
    assert!(second.starts_with(&format!("{first} ")), "{second}");
    assert!(std::path::Path::new(&format!("{second}/1.back.svg")).is_file());
    let r = s.execute(C, &json!({"folder": parent})).unwrap();
    assert!(!matches!(r["folder"].as_str(), Some(f) if f == first || f == second));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_failed_export_to_a_folder_makes_no_folder() {
    let dir = TempFolder::new("failed");
    let mut empty = Session::new();
    empty.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    assert!(empty.execute(C, &json!({"folder": dir.0.to_string_lossy()})).is_err());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0, "no folder is made");
}
