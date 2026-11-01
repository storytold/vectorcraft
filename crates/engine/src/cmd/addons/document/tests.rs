//! Document Tools' commands: the timestamped copy's name and place with the document left as it
//! was, symbols made of layers, and bad params refused.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};

use crate::Session;

/// 2026-10-05 14:03:00 UTC.
const TIME: i64 = 1_791_208_980;

/// A new, empty folder of its own for test `name`.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-addon-document-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    s
}

fn stamp(s: &mut Session, p: Value) -> crate::Result<Value> {
    s.execute("addon.document.saveTimestampedCopy", &p)
}

#[test]
fn a_timestamped_copy_goes_next_to_the_document_which_stays_as_it_was() {
    let dir = folder("next-to");
    let mut s = session();
    let own = dir.join("poster.vectorcraft");
    s.execute("document.save", &json!({"path": own})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let (steps, title) = (s.doc().unwrap().history.undo.len(), s.doc().unwrap().doc.title.clone());
    let r = stamp(&mut s, json!({"time": TIME})).unwrap();
    let copy = dir.join("poster_2026-10-05_14-03.vectorcraft");
    assert_eq!(Path::new(r["path"].as_str().unwrap()), copy);
    assert!(copy.is_file());
    let st = s.doc().unwrap();
    assert_eq!(st.path.as_deref().map(Path::new), Some(own.as_path()), "the document keeps its path");
    assert_eq!((st.history.undo.len(), &st.doc.title, st.is_dirty()), (steps, &title, true), "no step; still modified");
    // The copy holds the document as it is now: the ellipse too.
    let mut other = Session::new();
    other.execute("document.open", &json!({"path": copy})).unwrap();
    assert_eq!(other.doc().unwrap().doc.layers[0].children().unwrap().len(), 2);
    // The same minute again: numbered.
    let r = stamp(&mut s, json!({"time": TIME})).unwrap();
    assert!(r["path"].as_str().unwrap().ends_with("poster_2026-10-05_14-03-2.vectorcraft"), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unsaved_document_needs_a_folder_and_other_formats_take_their_extension() {
    let dir = folder("unsaved");
    let mut s = session();
    let before = s.doc().unwrap().doc.clone();
    assert!(stamp(&mut s, json!({"time": TIME})).is_err(), "no folder to put it in");
    let r = stamp(&mut s, json!({"time": TIME, "folder": dir, "format": "svg"})).unwrap();
    let name = Path::new(r["path"].as_str().unwrap()).file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.ends_with("_2026-10-05_14-03.svg") && dir.join(&name).is_file(), "{name}");
    let st = s.doc().unwrap();
    assert!(st.path.is_none() && *st.doc == *before, "the document is unchanged");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_params_are_refused_and_writes_stay_inside_the_automation_roots() {
    let dir = folder("bad");
    let mut s = session();
    for p in [
        json!({"folder": dir, "format": "png"}),
        json!({"folder": dir, "format": "dxf"}),
        json!({"folder": dir, "time": "now"}),
        json!({"folder": dir, "time": 1.5}),
        json!({"folder": dir.join("missing").join("deeper"), "time": TIME}),
    ] {
        assert!(stamp(&mut s, p.clone()).is_err(), "{p}");
    }
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "nothing written");
    let allowed = dir.join("allowed");
    std::fs::create_dir_all(&allowed).unwrap();
    let roots = crate::file_access::AutomationRoots::new(&[&dir], &[&allowed]).unwrap();
    let outside = crate::file_access::confine(roots.as_ref(), || stamp(&mut s, json!({"folder": dir, "time": TIME})));
    assert!(outside.is_err(), "outside the write root");
    let inside = crate::file_access::confine(roots.as_ref(), || stamp(&mut s, json!({"folder": allowed, "time": TIME})));
    assert!(inside.is_ok(), "{inside:?}");
    assert!(!enabled_without_document(), "off with no document");
    let _ = std::fs::remove_dir_all(&dir);
}

fn enabled_without_document() -> bool {
    Session::new().commands().into_iter().find(|c| c.id == "addon.document.saveTimestampedCopy").unwrap().enabled
}

fn layer_ids(s: &Session) -> Vec<u64> {
    s.doc().unwrap().doc.layers.iter().map(|l| l.id.0).collect()
}

fn symbol_names(s: &Session) -> Vec<String> {
    s.doc().unwrap().doc.symbols.iter().map(|x| x.name.clone()).collect()
}

/// A document of three layers: `Layer 1` with two rectangles, an empty one, and `Icons` with one
/// ellipse.
fn layered() -> Session {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 50, "height": 50})).unwrap();
    s.execute("layer.new", &json!({})).unwrap();
    s.execute("layer.new", &json!({"name": "Icons"})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 0, "y": 100, "width": 40, "height": 20})).unwrap();
    s
}

#[test]
fn a_symbol_is_made_of_each_layer_and_empty_layers_are_reported() {
    let mut s = layered();
    let before = symbol_names(&s).len();
    let n = s.doc().unwrap().history.undo.len();
    let r = s.execute("addon.document.symbolsFromLayers", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), n + 1, "one undo step");
    let names: Vec<String> = serde_json::from_value(r["symbols"].clone()).unwrap();
    assert_eq!(names.len(), 2, "{r}");
    assert!(names.contains(&"Icons".to_string()));
    assert_eq!(symbol_names(&s).len(), before + 2);
    assert_eq!(r["skipped"].as_array().unwrap().len(), 1, "the empty layer: {r}");
    assert_eq!(r["skipped"][0]["reason"], json!("empty"));
    assert_eq!(r["ids"], json!([]), "without replace the art stays");
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 2);
    // Again: the names are taken, so they get numbers.
    let r = s.execute("addon.document.symbolsFromLayers", &json!({})).unwrap();
    assert!(r["symbols"].as_array().unwrap().contains(&json!("Icons 2")), "{r}");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(symbol_names(&s).len(), before);
}

#[test]
fn replace_puts_an_instance_where_the_layers_art_was() {
    let mut s = layered();
    let first = layer_ids(&s)[0];
    let bounds = |s: &Session, id: NodeId| s.doc().unwrap().doc.bounds_of(&[id], false).unwrap();
    let layer = NodeId(first);
    let art = bounds(&s, layer);
    let r = s.execute("addon.document.symbolsFromLayers", &json!({"layers": [first], "replace": true})).unwrap();
    let ids = r["ids"].as_array().unwrap();
    assert_eq!(ids.len(), 1);
    let inst = NodeId(ids[0].as_u64().unwrap());
    let st = s.doc().unwrap();
    let kids = st.doc.layers[0].children().unwrap();
    assert_eq!(kids.len(), 1);
    assert!(matches!(&kids[0].kind, NodeKind::SymbolInstance { .. }));
    assert_eq!(st.selection.objects, [inst]);
    let now = bounds(&s, layer);
    assert!((now.x0 - art.x0).abs() < 1e-6 && (now.y1 - art.y1).abs() < 1e-6 && (now.width() - art.width()).abs() < 1e-6, "{art:?} {now:?}");
    assert_eq!(s.doc().unwrap().doc.layers.len(), 3, "the other layers untouched");
}

#[test]
fn locked_layers_keep_their_art_and_bad_layers_are_refused() {
    let mut s = layered();
    let ids = layer_ids(&s);
    s.execute("layer.setProps", &json!({"id": ids[2], "locked": true})).unwrap();
    let r = s.execute("addon.document.symbolsFromLayers", &json!({"replace": true})).unwrap();
    let skipped: Vec<&str> = r["skipped"].as_array().unwrap().iter().map(|k| k["reason"].as_str().unwrap()).collect();
    assert_eq!(skipped, ["empty", "locked"]);
    let before = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let rect = s.doc().unwrap().doc.layers[0].children().unwrap()[0].id.0;
    for p in [json!({"layers": [rect]}), json!({"layers": [u64::MAX]}), json!({"layers": "all"}), json!({"layers": [-1]}), json!({"replace": "yes"})]
    {
        assert!(s.execute("addon.document.symbolsFromLayers", &p).is_err(), "{p}");
    }
    assert!(*s.doc().unwrap().doc == *before.0 && s.doc().unwrap().history.undo.len() == before.1);
    // Only an empty layer: nothing made, no step.
    let r = s.execute("addon.document.symbolsFromLayers", &json!({"layers": [ids[1]]})).unwrap();
    assert_eq!((r["symbols"].clone(), s.doc().unwrap().history.undo.len()), (json!([]), before.1));
}

#[test]
fn the_layers_highlighted_in_the_layers_panel_are_the_default() {
    let mut s = layered();
    let ids = layer_ids(&s);
    // One highlighted row is just the current layer: every layer.
    s.execute("layer.setCurrent", &json!({"id": ids[0]})).unwrap();
    let r = s.execute("addon.document.symbolsFromLayers", &json!({})).unwrap();
    assert_eq!(r["symbols"].as_array().unwrap().len(), 2);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("layer.highlight", &json!({"ids": [ids[1], ids[2]]})).unwrap();
    let r = s.execute("addon.document.symbolsFromLayers", &json!({})).unwrap();
    assert_eq!((r["symbols"].clone(), r["skipped"].as_array().unwrap().len()), (json!(["Icons"]), 1));
}

/// #4920: Export All Open Documents writes an SVG of each open document; a folder that can't be
/// written lists every document in `failed`, and either way the active document stays active.
#[test]
fn export_all_open_documents_writes_each_and_reports_failures() {
    let dir = std::env::temp_dir().join(format!("vc-export-all-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "name": "one"})).unwrap();
    s.execute("file.new", &json!({"width": 100, "height": 100, "name": "two"})).unwrap();
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    let r = s.execute("addon.document.exportAllOpen", &json!({"folder": dir, "format": "svg"})).unwrap();
    assert_eq!(r["files"].as_array().unwrap().len(), 2, "{r}");
    assert!(dir.join("one.svg").exists() && dir.join("two.svg").exists());
    assert_eq!(s.active_index(), Some(0));
    assert!(s.documents().iter().all(|d| d.path.is_none()), "the documents stay unsaved");
    let missing = dir.join("missing").join("deeper");
    let r = s.execute("addon.document.exportAllOpen", &json!({"folder": missing, "format": "svg"})).unwrap();
    assert_eq!((r["files"].as_array().unwrap().len(), r["failed"].as_array().unwrap().len()), (0, 2), "{r}");
    assert_eq!(s.active_index(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// An SVG document last saved with linked images exports as one SVG with its images embedded: no
/// image file is written beside the copies (#4920 review).
#[test]
fn export_all_open_documents_writes_one_file_each() {
    let dir = std::env::temp_dir().join(format!("vc-export-all-one-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("out")).unwrap();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "name": "pic"})).unwrap();
    let mut png = vec![];
    image::RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.execute("file.place", &json!({"name": "a.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
    s.execute("document.save", &json!({"path": dir.join("pic.svg"), "format": "svg", "options": {"images": "link"}})).unwrap();
    let saved: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.path().is_file()).collect();
    assert!(saved.len() > 1, "the linked save wrote its image beside it: {saved:?}");
    let out = dir.join("out");
    let r = s.execute("addon.document.exportAllOpen", &json!({"folder": out, "format": "svg"})).unwrap();
    let names: Vec<String> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["pic.svg"], "{r}");
    assert!(std::fs::read_to_string(out.join("pic.svg")).unwrap().contains("data:image/png;base64,"), "the image is embedded");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A PDF document last saved with a page range exports with every page, and an SVG document with
/// two artboards exports as one SVG holding the art of both (#4920 review).
#[test]
fn export_all_open_documents_keeps_every_page_and_artboard() {
    let dir = std::env::temp_dir().join(format!("vc-export-all-pages-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("out")).unwrap();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "artboards": 2, "name": "pages"})).unwrap();
    s.execute("document.save", &json!({"path": dir.join("pages.pdf"), "format": "pdf", "options": {"range": "1"}})).unwrap();
    let pages = |b: &[u8]| {
        String::from_utf8_lossy(b).matches("/Type /Page\n").count()
            + String::from_utf8_lossy(b).matches("/Type/Page/").count()
            + String::from_utf8_lossy(b).matches("/Type /Page/").count()
            + String::from_utf8_lossy(b).matches("/Type /Page ").count()
    };
    let first = pages(&std::fs::read(dir.join("pages.pdf")).unwrap());
    let out = dir.join("out");
    s.execute("addon.document.exportAllOpen", &json!({"folder": out, "format": "pdf"})).unwrap();
    let copy = pages(&std::fs::read(out.join("pages.pdf")).unwrap());
    assert!(copy > first, "the copy has every page: {first} saved, {copy} exported");
    // SVG: one file with the art on both artboards.
    let a = s.doc().unwrap().doc.artboards[1].rect;
    s.execute("shape.rectangle", &json!({"x": a.x0 + 10.0, "y": a.y0 + 10.0, "width": 20, "height": 20})).unwrap();
    s.execute("addon.document.exportAllOpen", &json!({"folder": out, "format": "svg"})).unwrap();
    let svgs: Vec<String> =
        std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".svg")).collect();
    assert_eq!(svgs, ["pages.svg"]);
    let svg = std::fs::read_to_string(out.join("pages.svg")).unwrap();
    // Not the first artboard alone (its 100 × 100 box at the origin): the art's own bounds.
    let view = svg.split("viewBox=\"").nth(1).and_then(|v| v.split('"').next()).unwrap();
    let w: f64 = view.split_whitespace().nth(2).unwrap().parse().unwrap();
    assert!(w > 19.0 && w < 30.0, "the SVG shows the art (the square on the second artboard), not the first artboard: viewBox {view}");
    let _ = std::fs::remove_dir_all(&dir);
}
