//! Linked images: what a link records, the preview saved in place of the pixels, finding files
//! that moved, missing and modified files (`document.open`, `links.check`, `links.update`,
//! `links.relink`).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use vectorcraft_doc::{Document, ImageObject, LinkInfo, NodeKind};
use vectorcraft_geom::Rect;

use super::*;

/// A fresh folder for one test (removed when dropped).
pub(crate) struct Folder(pub(crate) PathBuf);

impl Folder {
    pub(crate) fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vectorcraft-links-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    pub(crate) fn file(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A `w`×`h` PNG of one colour.
pub(crate) fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut out = vec![];
    let [r, g, b] = rgb;
    image::RgbaImage::from_pixel(w, h, image::Rgba([r, g, b, 255])).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

pub(crate) const RED: [u8; 3] = [230, 20, 20];
pub(crate) const BLUE: [u8; 3] = [20, 20, 230];

pub(crate) fn write(path: &str, bytes: &[u8]) {
    if let Some(dir) = Path::new(path).parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(path, bytes).unwrap();
}

pub(crate) fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

/// Place the file at `path` (linked) → its id.
pub(crate) fn place(s: &mut Session, path: &str) -> NodeId {
    let r = s.execute("file.place", &json!({"path": path, "at": [200, 150]})).unwrap();
    assert_eq!(r["linked"], true, "{r}");
    NodeId(r["ids"][0].as_u64().unwrap())
}

pub(crate) fn image(s: &Session, id: NodeId) -> ImageObject {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Image(im) => im.clone(),
        k => panic!("not an image: {k:?}"),
    }
}

pub(crate) fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

/// The colour at the centre of the active document's artboard.
pub(crate) fn centre_colour(doc: &Document) -> [u8; 3] {
    let img = vectorcraft_render::Renderer::new().render_region(doc, Rect::new(0.0, 0.0, 400.0, 300.0), 1.0, true);
    let [r, g, b, _] = img.pixel(200, 150);
    [r, g, b]
}

pub(crate) fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 6)
}

/// The first image object's JSON in a saved document.
fn saved_image(v: &mut Value) -> &mut Value {
    &mut v["document"]["layers"][0]["kind"]["children"][0]["kind"]
}

pub(crate) fn save(s: &mut Session, path: &str) {
    s.execute("document.save", &json!({"path": path})).unwrap();
}

pub(crate) fn open(s: &mut Session, path: &str) -> Value {
    s.execute("document.open", &json!({"path": path})).unwrap()
}

#[test]
fn a_placed_link_records_the_file_and_the_document_saves_only_a_preview() {
    let dir = Folder::new("record");
    let pic = dir.file("art/photo.png");
    let bytes = png(600, 300, RED);
    write(&pic, &bytes);
    let mut s = session();
    let id = place(&mut s, &pic);
    let im = image(&s, id);
    let link = im.link.clone().unwrap();
    assert_eq!((link.path.as_str(), link.size), (pic.as_str(), Some(bytes.len() as u64)));
    assert_eq!(link.hash.as_deref(), Some(vectorcraft_doc::links::hash_bytes(&bytes).as_str()));
    assert!(link.modified.is_some_and(|m| m > 1_600_000_000_000), "{link:?}");
    let blob = &s.doc().unwrap().doc.images[&im.key];
    assert!(blob.proxy.is_some() && !blob.is_proxy(), "full pixels, with a preview to save");
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    let mut saved: Value = serde_json::from_slice(&std::fs::read(&doc_path).unwrap()).unwrap();
    let entry = &saved["images"][&im.key];
    assert_eq!(entry["proxy"], true);
    assert!(entry["data"].as_str().unwrap().len() * 3 / 4 < bytes.len(), "the preview is smaller than the file");
    let saved_link = saved_image(&mut saved)["link"].clone();
    assert_eq!((saved_link["path"].as_str(), saved_link["relative"].as_str()), (Some(pic.as_str()), Some("art/photo.png")));
    // Embedding keeps the pixels, and no preview is saved for them.
    s.execute("file.place", &json!({"path": pic, "link": false})).unwrap();
    save(&mut s, &doc_path);
    let saved: Value = serde_json::from_slice(&std::fs::read(&doc_path).unwrap()).unwrap();
    assert_eq!(saved["images"][&im.key]["proxy"], Value::Null, "an embedded image shows it too");
}

/// A flat `w`×`h` Photoshop document of one colour (raw RGB, no layers).
fn psd(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut f = b"8BPS\0\x01\0\0\0\0\0\0\0\x03".to_vec();
    f.extend(h.to_be_bytes());
    f.extend(w.to_be_bytes());
    f.extend([0, 8, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    for v in rgb {
        f.extend(std::iter::repeat_n(v, (w * h) as usize));
    }
    f
}

/// #918: a Photoshop document placed linked shows its merged image, and updates when it changes.
#[test]
fn a_linked_photoshop_document_places_and_updates() {
    let dir = Folder::new("psd");
    let pic = dir.file("art/photo.psd");
    write(&pic, &psd(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    let link = image(&s, id).link.unwrap();
    assert_eq!(link.path, pic);
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
    write(&pic, &psd(400, 400, BLUE));
    assert_eq!(s.execute("links.update", &json!({})).unwrap(), json!({"updated": [id.0], "missing": []}));
    let im = image(&s, id);
    assert_eq!((im.width, im.height), (400, 400));
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE));
}

#[test]
fn the_relative_path_finds_the_file_after_the_folder_moves() {
    let a = Folder::new("move-a");
    let b = Folder::new("move-b");
    let pic = a.file("art/photo.png");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    save(&mut s, &a.file("poster.vectorcraft"));
    // Move the document with its links: the old folder is gone.
    std::fs::remove_dir_all(&b.0).unwrap();
    std::fs::rename(&a.0, &b.0).unwrap();
    let r = open(&mut s, &b.file("poster.vectorcraft"));
    assert_eq!((r["missingLinks"].clone(), r["modifiedLinks"].clone()), (json!([]), json!([])), "{r}");
    let moved = b.file("art/photo.png");
    let im = image(&s, id);
    assert_eq!(Path::new(&im.link.unwrap().path), Path::new(&moved), "the link follows the file");
    assert!(!s.doc().unwrap().doc.images[&im.key].is_proxy(), "the file's pixels were read");
    let c = s.execute("links.check", &json!({})).unwrap();
    assert_eq!((c["links"][0]["status"].as_str(), c["missing"].as_u64()), (Some("ok"), Some(0)), "{c}");
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
}

#[test]
fn a_missing_link_renders_its_preview_and_relinks() {
    let dir = Folder::new("missing");
    let pic = dir.file("photo.png");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    let placed = bounds(&s, id);
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    std::fs::remove_file(&pic).unwrap();
    let r = open(&mut s, &doc_path);
    assert_eq!(r["missingLinks"], json!([{"name": "photo.png", "path": pic, "ids": [id.0]}]));
    let doc = &s.doc().unwrap().doc;
    assert!(doc.images[&image(&s, id).key].is_proxy(), "the preview stands in");
    assert!(near(centre_colour(doc), RED), "missing still renders");
    let c = s.execute("links.check", &json!({})).unwrap();
    assert_eq!((c["links"][0]["status"].as_str(), c["links"][0]["preview"].as_bool(), c["missing"].as_u64()), (Some("missing"), Some(true), Some(1)));
    assert_eq!(s.execute("links.update", &json!({})).unwrap(), json!({"updated": [], "missing": [id.0]}));
    // Replace: another file, smaller; the image keeps its bounds.
    let other = dir.file("other/blue.png");
    write(&other, &png(300, 100, BLUE));
    let r = s.execute("links.relink", &json!({"ids": [id.0], "path": other})).unwrap();
    assert_eq!(r, json!({"relinked": [id.0], "notFound": []}));
    let im = image(&s, id);
    assert_eq!((im.width, im.height, im.link.unwrap().path), (300, 100, other.clone()));
    let b = bounds(&s, id);
    assert!((b.x0 - placed.x0).abs() < 1e-6 && (b.y1 - placed.y1).abs() < 1e-6, "{b:?} vs {placed:?}");
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED), "one undo step");
    // Relink to Folder: the file of the link's name there.
    write(&dir.file("found/photo.png"), &png(600, 300, BLUE));
    let r = s.execute("links.relink", &json!({"folder": dir.file("found")})).unwrap();
    assert_eq!(r, json!({"relinked": [id.0], "notFound": []}), "nothing selected: every missing link");
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE));
    assert!(s.execute("links.relink", &json!({"ids": [id.0]})).is_err(), "path or folder");
}

#[test]
fn a_modified_link_follows_the_update_links_preference() {
    let dir = Folder::new("modified");
    let pic = dir.file("photo.png");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    let placed = bounds(&s, id);
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    write(&pic, &png(400, 400, BLUE));
    let c = s.execute("links.check", &json!({})).unwrap();
    assert_eq!((c["links"][0]["status"].as_str(), c["modified"].as_u64()), (Some("modified"), Some(1)), "{c}");

    // Ask When Modified (the default): reported, left as it was.
    let r = open(&mut s, &doc_path);
    assert_eq!((r["modifiedLinks"][0]["ids"].clone(), r["updatedLinks"].clone()), (json!([id.0]), json!([])), "{r}");
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED), "the preview until updated");
    let r = s.execute("links.update", &json!({})).unwrap();
    assert_eq!(r, json!({"updated": [id.0], "missing": []}));
    let im = image(&s, id);
    assert_eq!((im.width, im.height), (400, 400));
    let b = bounds(&s, id);
    assert!((b.width() - placed.width()).abs() < 1e-6 && (b.height() - placed.height()).abs() < 1e-6, "keeps its bounds: {b:?}");
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE));
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Update Links");
    let c = s.execute("links.check", &json!({})).unwrap();
    assert_eq!(c["links"][0]["status"], "ok");
    assert_eq!(s.execute("links.update", &json!({})).unwrap()["updated"], json!([]), "nothing left to update");

    // Automatically: read again as the document opens.
    s.execute("prefs.set", &json!({"key": "updateLinks", "value": "automatically"})).unwrap();
    let r = open(&mut s, &doc_path);
    assert_eq!((r["modifiedLinks"].clone(), r["updatedLinks"][0]["ids"].clone()), (json!([]), json!([id.0])), "{r}");
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE));
    assert!(s.doc().unwrap().history.undo.is_empty(), "opening is not an undo step");
}

#[test]
fn links_from_older_files_load_and_keep_their_pixels() {
    let dir = Folder::new("legacy");
    let pic = dir.file("photo.png");
    let bytes = png(600, 300, RED);
    write(&pic, &bytes);
    let mut s = session();
    s.execute("file.place", &json!({"path": pic, "at": [200, 150], "link": false})).unwrap();
    // As older versions saved a link: the path alone, the pixels embedded.
    let mut v: Value = serde_json::from_slice(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    saved_image(&mut v)["link"] = json!(pic);
    v["format"] = json!("drawcraft");
    let legacy = serde_json::to_vec(&v).unwrap();
    let doc_path = dir.file("old.drawcraft");
    write(&doc_path, &legacy);
    let r = open(&mut s, &doc_path);
    assert_eq!((r["missingLinks"].clone(), r["modifiedLinks"].clone()), (json!([]), json!([])), "{r}");
    let st = s.doc().unwrap();
    let id = st.doc.layers[0].children().unwrap()[0].id;
    let im = image(&s, id);
    let link = im.link.unwrap();
    assert_eq!((link.path.as_str(), link.size), (pic.as_str(), Some(bytes.len() as u64)), "the details are filled in");
    assert!(s.doc().unwrap().doc.images[&im.key].proxy.is_some(), "a preview to save from now on");
    // Gone, the embedded pixels still show.
    std::fs::remove_file(&pic).unwrap();
    let r = open(&mut s, &doc_path);
    assert_eq!(r["missingLinks"][0]["name"], "photo.png");
    assert!(!s.doc().unwrap().doc.images[&im.key].is_proxy());
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
}

#[test]
fn without_a_file_system_the_saved_preview_shows() {
    // What the web does: the document arrives as bytes and no linked file can be read.
    let dir = Folder::new("web");
    let pic = dir.file("photo.png");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    let saved = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    std::fs::remove_file(&pic).unwrap();
    let doc = vectorcraft_format::load(&saved).unwrap();
    assert!(doc.images.values().all(|b| b.is_proxy()));
    assert!(near(centre_colour(&doc), RED), "the preview renders");
    let r = s.execute("document.open", &json!({"name": "poster.vectorcraft", "dataBase64": vectorcraft_format::base64_encode(&saved)})).unwrap();
    assert_eq!(r["missingLinks"][0]["ids"], json!([id.0]));
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
    // Exports carry the preview.
    let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap();
    assert!(svg["text"].as_str().unwrap().contains("data:image/png;base64,"));
}

#[test]
fn relative_paths_are_written_against_the_saved_file() {
    let mut d = Document::new(10.0, 10.0);
    let (lid, nid) = (d.layers[0].id, d.alloc_id());
    let root = if cfg!(windows) { "C:\\work" } else { "/work" };
    let sep = std::path::MAIN_SEPARATOR;
    let im = ImageObject {
        key: "k".into(),
        width: 1,
        height: 1,
        xf: Default::default(),
        link: Some(LinkInfo::new(format!("{root}{sep}art{sep}a.png"))),
        placement: Default::default(),
    };
    d.insert(Some(lid), 0, vectorcraft_doc::Node::new(nid, NodeKind::Image(im))).unwrap();
    let rel = |dest: String| {
        let d = cmd::links::with_relative_paths(&d, &dest).unwrap_or_else(|| d.clone());
        match &d.node(nid).unwrap().kind {
            NodeKind::Image(im) => im.link.as_ref().unwrap().relative.clone(),
            _ => None,
        }
    };
    assert_eq!(rel(format!("{root}{sep}doc.vectorcraft")).as_deref(), Some("art/a.png"));
    assert_eq!(rel(format!("{root}{sep}docs{sep}deep{sep}doc.vectorcraft")).as_deref(), Some("../../art/a.png"));
    assert_eq!(rel(format!("{root}{sep}art{sep}doc.vectorcraft")).as_deref(), Some("a.png"));
    if cfg!(windows) {
        assert_eq!(rel("D:\\other\\doc.vectorcraft".into()), None, "another drive");
        assert_eq!(rel("c:\\WORK\\doc.vectorcraft".into()).as_deref(), Some("art/a.png"), "without case");
    }
}

#[test]
fn a_placed_document_brings_its_links_with_the_files_pixels() {
    let dir = Folder::new("place-doc");
    let pic = dir.file("photo.png");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    place(&mut s, &pic);
    let doc_path = dir.file("part.vectorcraft");
    save(&mut s, &doc_path);
    let mut s = session();
    // As an editable copy (placed linked, it stays one locked object).
    s.execute("file.place", &json!({"path": doc_path, "link": false})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let mut placed = vec![];
    doc.visit_images(|_, im| placed.push(im.clone()));
    let [im] = &placed[..] else { panic!("one image: {placed:?}") };
    assert_eq!(im.link.as_ref().map(|l| l.path.as_str()), Some(pic.as_str()), "still linked");
    assert!(!doc.images[&im.key].is_proxy(), "the file's pixels, not the preview");
}
