//! The save pipeline: Save writes a document's own format, Save As / a Copy / as Template, Revert,
//! templates, converted legacy files, the saved view and the single Document Color Mode command.

use std::path::PathBuf;

use serde_json::{Value, json};
use vectorcraft_doc::SavedView;
use vectorcraft_geom::Point;

use crate::cmd::fileio::{SaveMode, save_plan};
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s
}

fn rect(s: &mut Session) {
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
}

/// A fresh folder for one test.
fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vc-save-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn path(d: &std::path::Path, name: &str) -> String {
    d.join(name).to_string_lossy().to_string()
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

fn objects(s: &Session) -> usize {
    s.doc().unwrap().doc.node_count()
}

#[test]
fn an_svg_path_writes_svg_and_save_writes_it_back() {
    let d = dir("svg");
    let mut s = session();
    rect(&mut s);
    let svg = path(&d, "art.svg");
    let r = s.execute("document.save", &json!({"path": svg})).unwrap();
    assert_eq!(r["format"], "svg");
    assert!(r["warnings"][0].as_str().unwrap().contains("SVG keeps the artwork"), "{r}");
    assert!(std::fs::read_to_string(&svg).unwrap().starts_with("<?xml"));
    let st = s.doc().unwrap();
    assert_eq!((st.path.as_deref(), st.format, st.is_dirty()), (Some(svg.as_str()), "svg", false));
    // Save writes the same file in the same format.
    rect(&mut s);
    s.execute("document.save", &json!({})).unwrap();
    assert!(std::fs::read_to_string(&svg).unwrap().starts_with("<?xml"));
    assert!(!s.doc().unwrap().is_dirty());
    // A reopened SVG keeps its path and saves as SVG again.
    let mut s = Session::new();
    s.execute("document.open", &json!({"path": svg})).unwrap();
    assert_eq!((s.doc().unwrap().path.as_deref(), s.doc().unwrap().format), (Some(svg.as_str()), "svg"));
    rect(&mut s);
    assert_eq!(s.execute("document.save", &json!({})).unwrap()["format"], "svg");
    assert!(std::fs::read_to_string(&svg).unwrap().starts_with("<?xml"));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn save_as_takes_on_path_format_and_options() {
    let d = dir("saveas");
    let mut s = session();
    let pdf = path(&d, "doc.pdf");
    s.execute("file.saveAs", &json!({"path": pdf, "options": {"range": "1", "bogus": 1}})).unwrap();
    let st = s.doc().unwrap();
    assert_eq!((st.format, st.title()), ("pdf", "doc.pdf".to_string()));
    assert_eq!(Value::Object(st.save_options.clone()), json!({"range": "1"}), "only the format's own options are kept");
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF"));
    // The remembered options are what Save and the options dialog use.
    let o = s.execute("file.formatOptions", &json!({})).unwrap();
    assert_eq!((o["id"].as_str(), o["options"]["range"]["value"].as_str()), (Some("pdf"), Some("1")));
    assert_eq!(o["options"]["artboards"]["value"], Value::Null);
    let svg = s.execute("file.formatOptions", &json!({"format": "svg"})).unwrap();
    assert_eq!(svg["options"]["outlineText"]["value"], false, "another format shows its defaults");
    let ids: Vec<&str> = o["saveFormats"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["vectorcraft", "template", "pdf", "svg", "svgz", "ai", "eps"]);
    // SVGZ is gzip-compressed SVG and reopens as SVGZ.
    let svgz = path(&d, "doc.svgz");
    s.execute("file.saveAs", &json!({"path": svgz})).unwrap();
    assert_eq!(&std::fs::read(&svgz).unwrap()[..2], [0x1f, 0x8b]);
    s.execute("document.open", &json!({"path": svgz})).unwrap();
    assert_eq!(s.doc().unwrap().format, "svgz");
    // Export formats aren't save formats.
    let e = s.execute("file.saveAs", &json!({"path": path(&d, "x.png")})).unwrap_err().to_string();
    assert!(e.contains("document.export"), "{e}");
    assert!(s.execute("document.save", &json!({"format": "webp"})).is_err());
    assert!(s.execute("file.saveAs", &json!({"path": svgz, "options": 3})).is_err());
    let _ = std::fs::remove_dir_all(d);
}

/// #4523: a new document's VectorCraft Options start as the last native save used them (not its
/// artboard range, nor Use Compression, which its preference decides).
#[test]
fn a_new_document_offers_the_native_options_used_last() {
    let d = dir("lastnative");
    let mut s = session();
    let opts = json!({"preview": true, "embedProfiles": false, "pdfCompatible": true, "separateArtboards": true, "range": "1"});
    s.execute("file.saveAs", &json!({"path": path(&d, "a.vectorcraft"), "options": opts})).unwrap();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let o = s.execute("file.formatOptions", &json!({"format": "vectorcraft"})).unwrap();
    let value = |k: &str| o["options"][k]["value"].clone();
    assert_eq!((value("preview"), value("embedProfiles"), value("pdfCompatible")), (json!(true), json!(false), json!(true)));
    assert_eq!(
        (value("separateArtboards"), value("range")),
        (o["options"]["separateArtboards"]["default"].clone(), o["options"]["range"]["default"].clone())
    );
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn another_format_without_a_path_hands_the_bytes_back() {
    let d = dir("otherfmt");
    let mut s = session();
    let native = path(&d, "a.vectorcraft");
    s.execute("document.save", &json!({"path": native})).unwrap();
    let r = s.execute("document.save", &json!({"format": "svg"})).unwrap();
    assert!(String::from_utf8(b64(&r)).unwrap().starts_with("<?xml"));
    assert_eq!(r["name"], "a.svg");
    assert!(vectorcraft_format::sniff(&std::fs::read(&native).unwrap()), "the native file isn't overwritten with SVG");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn save_a_copy_keeps_the_path_title_and_modified_state() {
    let d = dir("copy");
    let mut s = session();
    let native = path(&d, "poster.vectorcraft");
    s.execute("document.save", &json!({"path": native})).unwrap();
    rect(&mut s);
    let r = s.execute("file.saveCopy", &json!({})).unwrap();
    assert_eq!((r["name"].as_str(), r["folder"].as_str()), (Some("poster copy.vectorcraft"), Some(d.to_string_lossy().as_ref())));
    let copy = path(&d, "poster copy.svg");
    s.execute("file.saveCopy", &json!({"path": copy})).unwrap();
    assert!(std::fs::read_to_string(&copy).unwrap().starts_with("<?xml"));
    let st = s.doc().unwrap();
    assert_eq!((st.path.as_deref(), st.format, st.title()), (Some(native.as_str()), "vectorcraft", "poster.vectorcraft".to_string()));
    assert!(st.is_dirty(), "a copy leaves the document modified");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn revert_reloads_the_saved_file_in_the_same_tab() {
    let d = dir("revert");
    let mut s = session();
    rect(&mut s);
    let native = path(&d, "a.vectorcraft");
    s.execute("document.save", &json!({"path": native})).unwrap();
    assert!(s.execute("file.revert", &json!({})).is_err(), "nothing to revert");
    s.execute("file.new", &json!({})).unwrap();
    s.set_active(0);
    let uid = s.doc().unwrap().uid;
    rect(&mut s);
    rect(&mut s);
    assert_eq!(objects(&s), 4);
    s.execute("file.revert", &json!({})).unwrap();
    let st = s.doc().unwrap();
    assert_eq!((s.active_index(), s.documents().len(), objects(&s)), (Some(0), 2, 2));
    assert!(!st.is_dirty() && st.history.undo.is_empty() && st.selection.is_empty());
    assert_eq!((st.uid, st.path.as_deref()), (uid, Some(native.as_str())));
    // With the file gone the document is left as it is, still modified.
    rect(&mut s);
    std::fs::remove_file(&native).unwrap();
    assert!(s.execute("file.revert", &json!({})).is_err());
    let st = s.doc().unwrap();
    assert!(st.is_dirty());
    assert_eq!((objects(&s), st.history.undo.len()), (3, 1));
    // Never saved: nothing to revert to.
    s.set_active(1);
    rect(&mut s);
    assert!(s.execute("file.revert", &json!({})).is_err());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn templates_suggest_a_name_in_the_templates_folder_and_open_untitled() {
    let d = dir("tpl");
    let mut s = session();
    let source = path(&d, "flyer.vectorcraft");
    s.execute("document.save", &json!({"path": source})).unwrap();
    s.prefs.templates_folder = path(&d, "Templates");
    let r = s.execute("file.saveAsTemplate", &json!({})).unwrap();
    assert_eq!(r["name"], "flyer template.vctemplate");
    assert_eq!(r["folder"].as_str(), Some(s.prefs.templates_folder.as_str()));
    assert_ne!(r["name"].as_str(), std::path::Path::new(&source).file_name().and_then(|n| n.to_str()), "never the source file");
    // Writing it leaves the document alone; opening it gives an untitled document.
    rect(&mut s);
    let tpl = path(&d, "flyer template.vctemplate");
    s.execute("file.saveAsTemplate", &json!({"path": tpl})).unwrap();
    assert_eq!(s.doc().unwrap().path.as_deref(), Some(source.as_str()));
    assert!(s.doc().unwrap().is_dirty());
    let r = s.execute("document.open", &json!({"path": tpl})).unwrap();
    assert_eq!(r["format"], "template");
    assert!(r["title"].as_str().unwrap().starts_with("Untitled-"), "{r}");
    assert_eq!(s.doc().unwrap().path, None);
    assert!(!s.doc().unwrap().doc.template);
    // New from Template opens any file untitled.
    let r = s.execute("file.newFromTemplate", &json!({"path": source})).unwrap();
    assert!(r["title"].as_str().unwrap().starts_with("Untitled-"), "{r}");
    assert_eq!(s.doc().unwrap().path, None);
    // With no preference the folder is under the user's home (where there is one).
    s.prefs.templates_folder.clear();
    if let Some(f) = crate::cmd::fileio::templates_folder(&s.prefs) {
        assert!(f.ends_with("VectorCraft Templates"), "{f}");
    }
    let _ = std::fs::remove_dir_all(d);
}

/// A native file as an older version wrote it: under the former name, or format version 1.
fn old_file(legacy: bool) -> Vec<u8> {
    let text = String::from_utf8(vectorcraft_format::save(&vectorcraft_doc::Document::new(50.0, 50.0), false)).unwrap();
    let text = if legacy {
        text.replacen("\"format\":\"vectorcraft\"", "\"format\":\"drawcraft\"", 1)
    } else {
        text.replacen(&format!("\"version\":{}", vectorcraft_format::VERSION), "\"version\":1", 1)
    };
    text.into_bytes()
}

#[test]
fn older_native_files_open_converted_and_save_under_a_new_name() {
    let d = dir("converted");
    let legacy = path(&d, "logo.drawcraft");
    std::fs::write(&legacy, old_file(true)).unwrap();
    let v1 = path(&d, "badge.vectorcraft");
    std::fs::write(&v1, old_file(false)).unwrap();
    let mut s = Session::new();
    s.execute("document.open", &json!({"path": legacy})).unwrap();
    assert_eq!(s.doc().unwrap().title(), "logo [Converted]");
    // Save doesn't overwrite the old file: it asks for a .vectorcraft name.
    let r = s.execute("document.save", &json!({})).unwrap();
    assert_eq!((r["name"].as_str(), r.get("path")), (Some("logo.vectorcraft"), None));
    assert_eq!(std::fs::read(&legacy).unwrap(), old_file(true));
    let r = s.execute("document.open", &json!({"path": v1})).unwrap();
    assert_eq!(r["title"], "badge [Converted]");
    let saved = path(&d, "badge2.vectorcraft");
    s.execute("file.saveAs", &json!({"path": saved})).unwrap();
    assert_eq!(s.doc().unwrap().title(), "badge2.vectorcraft");
    assert!(!s.doc().unwrap().converted);
    // A current file isn't converted; with the preference off old files open as they are.
    s.execute("document.open", &json!({"path": saved})).unwrap();
    assert_eq!(s.doc().unwrap().title(), "badge2.vectorcraft");
    s.prefs.append_converted = false;
    s.execute("document.open", &json!({"path": legacy})).unwrap();
    assert_eq!(s.doc().unwrap().title(), "logo.drawcraft");
    assert_eq!(s.execute("document.save", &json!({})).unwrap()["path"].as_str(), Some(legacy.as_str()));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn the_saved_view_round_trips_without_dirtying() {
    let d = dir("view");
    let mut s = session();
    let view = SavedView { name: String::new(), center: Point::new(40.0, 25.0), zoom: 3.0, rotation: 15.0 };
    s.doc_mut().unwrap().view = Some(view.clone());
    assert!(!s.doc().unwrap().is_dirty(), "the view isn't an edit");
    let native = path(&d, "v.vectorcraft");
    s.execute("document.save", &json!({"path": native})).unwrap();
    assert_eq!(s.doc().unwrap().doc.last_view, None, "only the written file carries it");
    s.execute("document.open", &json!({"path": native})).unwrap();
    assert_eq!(s.doc().unwrap().view.as_ref(), Some(&view));
    // Files without a view (older ones) open with none; SVG doesn't carry it.
    let doc = vectorcraft_format::load(&vectorcraft_format::save(&vectorcraft_doc::Document::new(5.0, 5.0), false)).unwrap();
    assert_eq!(doc.last_view, None);
    let svg = path(&d, "v.svg");
    s.execute("document.save", &json!({"path": svg})).unwrap();
    s.execute("document.open", &json!({"path": svg})).unwrap();
    assert_eq!(s.doc().unwrap().view, None);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn open_layers_rows_round_trip_without_dirtying() {
    let d = dir("open-rows");
    let mut s = session();
    let layer = s.doc().unwrap().doc.layers[0].id;
    let a = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    // A document starts with its top-level layers open, and a file keeps nothing for that.
    assert!(s.doc().unwrap().layers_open.contains(layer) && !s.doc().unwrap().layers_open.contains(g));
    let native = path(&d, "rows.vectorcraft");
    s.execute("document.save", &json!({"path": native})).unwrap();
    let bytes = std::fs::read(&native).unwrap();
    assert_eq!(vectorcraft_format::load(&bytes).unwrap().layers_open, None);
    // Opening the group and closing the layer is view state: not an edit, kept by the file.
    let st = s.doc_mut().unwrap();
    st.layers_open.set(g, true);
    st.layers_open.set(layer, false);
    assert!(!s.doc().unwrap().is_dirty());
    s.execute("document.save", &json!({"path": native})).unwrap();
    assert_eq!(s.doc().unwrap().doc.layers_open, None, "only the written file carries it");
    let mut again = session();
    again.execute("document.open", &json!({"path": native})).unwrap();
    let open = &again.doc().unwrap().layers_open;
    assert!(open.contains(g) && !open.contains(layer));
    // Older files (none saved) open with the top-level layers open.
    let doc = vectorcraft_format::load(&vectorcraft_format::save(&vectorcraft_doc::Document::new(5.0, 5.0), false)).unwrap();
    assert_eq!(doc.layers_open, None);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn save_plans_suggest_names_and_folders() {
    let mut s = session();
    s.execute("file.info", &json!({"title": "Map"})).unwrap();
    let plan = |s: &Session, mode, p: Value| save_plan(s, mode, &p).unwrap();
    let p = plan(&s, SaveMode::Save, json!({}));
    assert_eq!((p.path, p.name.as_str(), p.format.id, p.folder), (None, "Map.vectorcraft", "vectorcraft", None));
    assert_eq!(plan(&s, SaveMode::Copy, json!({"format": "pdf"})).name, "Map copy.pdf");
    assert_eq!(plan(&s, SaveMode::Template, json!({"format": "svg"})).format.id, "template", "a template is native or .ait");
    // A native template by default; .ait when the format, else the path, names it.
    assert_eq!(plan(&s, SaveMode::Template, json!({})).name, "Map template.vctemplate");
    assert_eq!(plan(&s, SaveMode::Template, json!({"format": "ait"})).name, "Map template.ait");
    assert_eq!(plan(&s, SaveMode::Template, json!({"path": "/t/Map.ait"})).format.id, "ait");
    assert_eq!(plan(&s, SaveMode::Template, json!({"path": "/t/Map.ait", "format": "template"})).format.id, "template");
    assert_eq!(SaveMode::of("file.saveCopy"), Some(SaveMode::Copy));
    assert_eq!(crate::cmd::fileio::save_filters("svg")[0], ("SVG", &["svg"][..]));
}

#[test]
fn document_color_mode_is_one_command_with_a_hidden_alias() {
    let mut s = session();
    let ids: Vec<&str> = s.commands().iter().filter(|c| c.menu.first() == Some(&"File") && c.label.contains("Color Mode")).map(|c| c.id).collect();
    assert_eq!(ids, ["file.documentColorMode"], "one File menu entry");
    assert!(crate::cmd::is_alias("object.convertDocumentColorMode") && !crate::cmd::is_alias("file.documentColorMode"));
    // Both ids run the same conversion; `convert: false` only switches the mode.
    rect(&mut s);
    assert!(s.execute("object.convertDocumentColorMode", &json!({"mode": "cmyk"})).unwrap()["changed"].as_u64().unwrap() >= 1);
    let before = s.doc().unwrap().doc.clone();
    let r = s.execute("file.documentColorMode", &json!({"mode": "rgb", "convert": false})).unwrap();
    assert_eq!(r["changed"], 0);
    assert_eq!(s.doc().unwrap().doc.color_mode, vectorcraft_doc::ColorMode::Rgb);
    assert_eq!(s.doc().unwrap().doc.layers, before.layers, "colours untouched");
    let e = s.execute("file.documentColorMode", &json!({"mode": "lab"})).unwrap_err().to_string();
    assert!(e.contains("file.documentColorMode"), "{e}");
}

#[test]
fn choosing_the_current_color_mode_is_not_an_edit() {
    let mut s = session();
    rect(&mut s);
    let st = s.doc_mut().unwrap();
    st.mark_saved();
    let undo = st.history.undo.len();
    for id in ["file.documentColorMode", "object.convertDocumentColorMode"] {
        assert_eq!(s.execute(id, &json!({"mode": "rgb"})).unwrap()["changed"], 0);
    }
    let st = s.doc().unwrap();
    assert!(!st.is_dirty(), "the document stays saved");
    assert_eq!(st.history.undo.len(), undo, "no empty undo step");
}

#[test]
fn a_pdf_opened_in_part_is_not_saved_back_over_the_whole_file() {
    let d = dir("pdfpart");
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "artboards": 2})).unwrap();
    rect(&mut s);
    let pdf = path(&d, "two.pdf");
    s.execute("document.export", &json!({"path": pdf, "format": "pdf"})).unwrap();
    // The whole file: Save writes it back as PDF.
    s.execute("document.open", &json!({"path": pdf})).unwrap();
    let st = s.doc().unwrap();
    assert_eq!((st.path.as_deref(), st.format), (Some(pdf.as_str()), "pdf"));
    // One page of it: Save asks for a name instead of dropping the other page.
    s.execute("document.open", &json!({"path": pdf, "pages": "1"})).unwrap();
    let st = s.doc().unwrap();
    assert_eq!((st.path.as_deref(), st.format), (None, "vectorcraft"));
    assert!(s.execute("document.save", &json!({})).unwrap().get("dataBase64").is_some());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_template_places_like_a_native_file() {
    let mut s = session();
    rect(&mut s);
    let tpl = s.execute("document.serialize", &json!({"format": "template"})).unwrap();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    let empty = s.doc().unwrap().doc.node_count();
    s.execute("file.place", &json!({"name": "card.vctemplate", "dataBase64": tpl["dataBase64"]})).unwrap();
    let st = s.doc().unwrap();
    assert!(st.doc.node_count() > empty && !st.selection.is_empty(), "the template's art is placed and selected");
}

#[test]
fn transparency_grid_round_trips_without_dirtying() {
    let d = dir("transparency-grid");
    let mut s = session();
    let native = path(&d, "grid.vectorcraft");
    for on in [true, false] {
        s.execute("view.transparencyGrid", &json!({"on": on})).unwrap();
        assert!(!s.doc().unwrap().is_dirty(), "the grid is view state, not an edit");
        s.execute("document.save", &json!({"path": native})).unwrap();
        s.execute("document.open", &json!({"path": native})).unwrap();
        assert_eq!(s.doc().unwrap().transparency_grid, on);
        assert!(s.doc().unwrap().history.undo.is_empty());
        assert!(!s.doc().unwrap().is_dirty());
        if on {
            let svg = path(&d, "grid.svg");
            s.execute("file.saveCopy", &json!({"path": svg})).unwrap();
            s.execute("document.open", &json!({"path": svg})).unwrap();
            assert!(!s.doc().unwrap().transparency_grid, "SVG does not store view state");
            s.execute("document.open", &json!({"path": native})).unwrap();
            assert!(s.doc().unwrap().transparency_grid);
        }
    }
    // A native file written before this setting existed still opens with the grid hidden.
    let old = path(&d, "old.vectorcraft");
    let bytes = vectorcraft_format::save(&Document::new(5.0, 5.0), false);
    assert!(!String::from_utf8_lossy(&bytes).contains("transparency_grid"));
    std::fs::write(&old, bytes).unwrap();
    s.execute("document.open", &json!({"path": old})).unwrap();
    assert!(!s.doc().unwrap().transparency_grid);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn opened_eps_saves_to_its_own_file_with_editable_data_and_page_margins() {
    let d = dir("eps-original-format");
    let eps_path = path(&d, "Artwork.eps");
    // Original, contributor-authored PostScript: the artwork occupies only part of the page.
    let source = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 200 100\n0 0 1 setrgbcolor 30 20 40 30 rectfill\nshowpage\n%%EOF\n";
    std::fs::write(&eps_path, source).unwrap();
    let mut s = Session::new();
    s.execute("document.open", &json!({"path": eps_path})).unwrap();
    assert_eq!(s.doc().unwrap().format, "eps");
    assert_eq!(s.doc().unwrap().path.as_deref(), Some(eps_path.as_str()));
    let board = s.doc().unwrap().doc.artboards[0].rect;
    assert_eq!((board.width(), board.height()), (200.0, 100.0));
    rect(&mut s);
    let edited_layers = s.doc().unwrap().doc.layers.clone();
    let r = s.execute("document.save", &json!({"options": {"previewFormat": "none", "thumbnails": false}})).unwrap();
    assert_eq!((r["path"].as_str(), r["format"].as_str()), (Some(eps_path.as_str()), Some("eps")));
    let bytes = std::fs::read(&eps_path).unwrap();
    assert!(bytes.starts_with(b"%!PS-Adobe-3.0 EPSF-3.0"));
    assert!(std::str::from_utf8(vectorcraft_eps::sections(&bytes).unwrap().0).unwrap().contains("%%BoundingBox: 0 0 200 100"));
    let editing = vectorcraft_format::load(&vectorcraft_eps::native(&bytes).unwrap()).unwrap();
    assert_eq!(editing.artboards[0].rect, board);
    assert_eq!(editing.layers, edited_layers);
    assert!(!s.doc().unwrap().is_dirty());
    let opened = s.execute("document.open", &json!({"path": eps_path})).unwrap();
    assert_eq!(opened["restored"], true);
    assert_eq!(s.doc().unwrap().doc.layers, edited_layers);
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect, board);
    rect(&mut s);
    s.execute("document.save", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().path.as_deref(), Some(eps_path.as_str()));
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn eps_save_keeps_all_editing_artboards_while_printing_one_page() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    rect(&mut s);
    let r = s.execute("document.save", &json!({"format": "eps", "artboard": 1, "previewFormat": "none", "thumbnails": false})).unwrap();
    let bytes = b64(&r);
    let native = vectorcraft_format::load(&vectorcraft_eps::native(&bytes).unwrap()).unwrap();
    assert_eq!(native.artboards.len(), 2);
    assert_eq!(native.layers, s.doc().unwrap().doc.layers);
    assert!(s.execute("document.save", &json!({"format": "eps", "range": "all"})).is_err(), "EPS is one printed page");
    for options in [json!({"useArtboards": false}), json!({"selectedOnly": true})] {
        assert!(s.execute("document.save", &json!({"format": "eps", "options": options})).is_err(), "cropping is Export's job");
    }
}

#[test]
fn eps_save_retains_lossy_import_overwrite_protection_and_transparency_warning() {
    let d = dir("eps-import-warning");
    let eps_path = path(&d, "Artwork.eps");
    let mut s = session();
    rect(&mut s);
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    let r = s.execute("document.save", &json!({"path": eps_path, "previewFormat": "none", "thumbnails": false})).unwrap();
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("flatten")), "{r}");
    let original = std::fs::read(&eps_path).unwrap();
    let st = s.doc_mut().unwrap();
    st.imported_from = Some(eps_path.clone());
    st.import_losses.push("test import omitted artwork".into());
    rect(&mut s);
    assert!(s.execute("document.save", &json!({})).unwrap_err().to_string().contains("acknowledgeLoss"));
    assert_eq!(std::fs::read(&eps_path).unwrap(), original);
    s.execute("document.save", &json!({"acknowledgeLoss": true})).unwrap();
    assert_ne!(std::fs::read(&eps_path).unwrap(), original);
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn eps_save_supports_an_empty_artboard_without_cropping_its_page() {
    let mut s = session();
    let r = s.execute("document.save", &json!({"format": "eps", "previewFormat": "none", "thumbnails": false})).unwrap();
    let bytes = b64(&r);
    let native = vectorcraft_format::load(&vectorcraft_eps::native(&bytes).unwrap()).unwrap();
    assert_eq!(native.artboards[0].rect, s.doc().unwrap().doc.artboards[0].rect);
    assert_eq!(native.node_count(), s.doc().unwrap().doc.node_count());
    assert!(std::str::from_utf8(vectorcraft_eps::sections(&bytes).unwrap().0).unwrap().contains("%%BoundingBox: 0 0 200 100"));
}

#[test]
fn partially_read_eps_requires_confirmation_before_save_back() {
    let d = dir("eps-partial-import");
    let eps_path = path(&d, "partial.eps");
    let original = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 200 100\n0 0 1 setrgbcolor 30 20 40 30 rectfill\nunsupportedMockyOperator\n1 0 0 setrgbcolor 80 20 40 30 rectfill\nshowpage\n%%EOF\n";
    std::fs::write(&eps_path, original).unwrap();
    let mut s = Session::new();
    let opened = s.execute("document.open", &json!({"path": eps_path})).unwrap();
    assert!(opened["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().starts_with("the file was read up to an error")), "{opened}");
    rect(&mut s);
    assert!(s.execute("document.save", &json!({})).unwrap_err().to_string().contains("acknowledgeLoss"));
    assert_eq!(std::fs::read(&eps_path).unwrap(), original);
    s.execute("document.save", &json!({"acknowledgeLoss": true})).unwrap();
    assert!(vectorcraft_eps::native(&std::fs::read(&eps_path).unwrap()).is_some());
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn eps_with_damaged_editing_data_requires_confirmation_before_save_back() {
    let d = dir("eps-damaged-editing");
    let eps_path = path(&d, "damaged.eps");
    let mut s = session();
    rect(&mut s);
    let r = s.execute("document.exportEps", &json!({"previewFormat": "none", "thumbnails": false, "cmykPostScript": false})).unwrap();
    let mut bytes = b64(&r);
    let at = bytes.windows(9).position(|w| w == b"native\n% ").unwrap() + 12;
    bytes[at] = b'z';
    std::fs::write(&eps_path, &bytes).unwrap();
    let opened = s.execute("document.open", &json!({"path": eps_path})).unwrap();
    assert_eq!(opened["restored"], false);
    assert!(opened["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("editing data can't be read")), "{opened}");
    rect(&mut s);
    assert!(s.execute("document.save", &json!({})).unwrap_err().to_string().contains("acknowledgeLoss"));
    assert_eq!(std::fs::read(&eps_path).unwrap(), bytes);
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn eps_save_bounds_large_page_preview_without_cropping_vector_or_editing_data() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 4000, "height": 4000})).unwrap();
    rect(&mut s);
    // Explicit colour preview remains a colour preview; Save bounds its resolution, not the art.
    let r = s.execute("document.save", &json!({"format": "eps", "previewFormat": "tiffColor"})).unwrap();
    let bytes = b64(&r);
    let (ps, tiff) = vectorcraft_eps::sections(&bytes).unwrap();
    let tiff = tiff.unwrap();
    assert!(tiff.len() < 4 * 1024 * 1024 + 64 * 1024, "compatibility preview grew to {} bytes", tiff.len());
    let preview = image::load_from_memory_with_format(tiff, image::ImageFormat::Tiff).unwrap();
    assert_eq!((preview.width(), preview.height()), (1024, 1024));
    assert!(std::str::from_utf8(ps).unwrap().contains("%%BoundingBox: 0 0 4000 4000"));
    let native = vectorcraft_format::load(&vectorcraft_eps::native(&bytes).unwrap()).unwrap();
    assert_eq!(native.artboards, s.doc().unwrap().doc.artboards);
    assert_eq!(native.layers, s.doc().unwrap().doc.layers);
}

/// #4646: a new document's units and ruler guides survive Save, Close and Open in every format
/// that carries the whole document (an SVG keeps them only with Preserve Editing).
#[test]
fn units_and_ruler_guides_survive_save_and_reopen() {
    let d = dir("units-guides");
    for ext in ["vectorcraft", "pdf", "ai", "eps"] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 100, "units": "Millimeters"})).unwrap();
        s.execute("guide.add", &json!({"vertical": true, "pos": 50})).unwrap();
        s.execute("guide.add", &json!({"vertical": false, "pos": 30})).unwrap();
        rect(&mut s);
        let p = path(&d, &format!("a.{ext}"));
        s.execute("document.save", &json!({"path": p})).unwrap();
        s.execute("file.close", &json!({})).unwrap();
        s.execute("document.open", &json!({"path": p})).unwrap();
        let doc = &s.doc().unwrap().doc;
        let guides: Vec<_> = doc.guides.iter().map(|g| (g.vertical, g.pos)).collect();
        assert_eq!((doc.units, guides), (vectorcraft_doc::Unit::Millimeters, vec![(true, 50.0), (false, 30.0)]), "{ext}");
    }
    let _ = std::fs::remove_dir_all(d);
}
