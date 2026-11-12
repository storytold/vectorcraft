//! Linked images: what a link records, the preview saved in place of the pixels, finding files
//! that moved, missing and modified files (`document.open`, `links.check`, `links.update`,
//! `links.relink`).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use vectorcraft_doc::{Document, ImageObject, LinkInfo, NodeKind};
use vectorcraft_geom::Rect;

use crate::*;

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
        NodeKind::Image(im) => (**im).clone(),
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

/// An uncompressed 24-bit `w`×`h` Targa file of one color.
fn targa(w: u16, h: u16, rgb: [u8; 3]) -> Vec<u8> {
    let mut f = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    f.extend(w.to_le_bytes());
    f.extend(h.to_le_bytes());
    f.extend([24, 0]);
    let [r, g, b] = rgb;
    f.extend(std::iter::repeat_n([b, g, r], usize::from(w) * usize::from(h)).flatten());
    f
}

/// A Targa file placed linked shows its pixels and updates when it changes, and a link can be
/// relinked to one.
#[test]
fn a_linked_targa_file_places_updates_and_relinks() {
    let dir = Folder::new("tga");
    let pic = dir.file("art/photo.tga");
    write(&pic, &targa(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    assert_eq!(image(&s, id).link.unwrap().path, pic);
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
    write(&pic, &targa(400, 400, BLUE));
    assert_eq!(s.execute("links.update", &json!({})).unwrap(), json!({"updated": [id.0], "missing": []}));
    let im = image(&s, id);
    assert_eq!((im.width, im.height), (400, 400));
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE));
    let other = dir.file("other/red.vst");
    write(&other, &targa(300, 100, RED));
    let r = s.execute("links.relink", &json!({"ids": [id.0], "path": other})).unwrap();
    assert_eq!(r, json!({"relinked": [id.0], "notFound": []}));
    let im = image(&s, id);
    assert_eq!((im.width, im.height, im.link.unwrap().path), (300, 100, other));
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
}

/// A BMP file named `.dib` placed linked shows its pixels, is listed as BMP and updates when it
/// changes; relinked to a PNG named `.pns`, it is listed as PNG.
#[test]
fn linked_dib_and_pns_files_place_update_and_relink() {
    let bmp = |w: u32, h: u32, rgb: [u8; 3]| {
        let mut out = vec![];
        let [r, g, b] = rgb;
        image::RgbImage::from_pixel(w, h, image::Rgb([r, g, b])).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Bmp).unwrap();
        out
    };
    let dir = Folder::new("dib");
    let pic = dir.file("art/scan.DIB");
    write(&pic, &bmp(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
    assert_eq!(s.execute("links.info", &json!({"id": id.0})).unwrap()["format"], "BMP");
    write(&pic, &bmp(400, 400, BLUE));
    assert_eq!(s.execute("links.update", &json!({})).unwrap(), json!({"updated": [id.0], "missing": []}));
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE));
    let other = dir.file("other/red.pns");
    write(&other, &png(300, 100, RED));
    assert_eq!(s.execute("links.relink", &json!({"ids": [id.0], "path": other})).unwrap(), json!({"relinked": [id.0], "notFound": []}));
    let im = image(&s, id);
    assert_eq!((im.width, im.height, im.link.unwrap().path), (300, 100, other));
    assert_eq!(s.execute("links.info", &json!({"id": id.0})).unwrap()["format"], "PNG");
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

/// A document moved to another folder while its linked file is missing keeps the file's relative
/// path when it is saved again, there or in a subfolder, so the file is found once it is back
/// beside the document.
#[test]
fn a_missing_link_keeps_its_relative_path_when_the_moved_document_is_saved() {
    let a = Folder::new("missing-save-a");
    let b = Folder::new("missing-save-b");
    let away = Folder::new("missing-save-away");
    let pic = a.file("art/photo.png");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    save(&mut s, &a.file("poster.vectorcraft"));
    // Move the document and its links, then take the links away.
    std::fs::remove_dir_all(&b.0).unwrap();
    std::fs::rename(&a.0, &b.0).unwrap();
    std::fs::rename(b.0.join("art"), away.0.join("art")).unwrap();
    let r = open(&mut s, &b.file("poster.vectorcraft"));
    assert_eq!(r["missingLinks"][0]["name"].as_str(), Some("photo.png"), "{r}");
    save(&mut s, &b.file("poster.vectorcraft"));
    std::fs::create_dir_all(b.0.join("copies")).unwrap();
    save(&mut s, &b.file("copies/poster.vectorcraft"));
    let relative = |path: &str| {
        let mut saved: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        saved_image(&mut saved)["link"]["relative"].as_str().map(str::to_string)
    };
    assert_eq!(relative(&b.file("poster.vectorcraft")).as_deref(), Some("art/photo.png"));
    assert_eq!(relative(&b.file("copies/poster.vectorcraft")).as_deref(), Some("../art/photo.png"));
    // Saved again where the last save put it, the file is still named from the folder it was read from.
    s.execute("document.save", &json!({})).unwrap();
    assert_eq!(relative(&b.file("copies/poster.vectorcraft")).as_deref(), Some("../art/photo.png"));
    // The links come back: both saved documents find the file.
    std::fs::rename(away.0.join("art"), b.0.join("art")).unwrap();
    let moved = b.file("art/photo.png");
    for doc in [b.file("poster.vectorcraft"), b.file("copies/poster.vectorcraft")] {
        let r = open(&mut s, &doc);
        assert_eq!(&r["missingLinks"], &json!([]), "{doc}: {r}");
        assert_eq!(Path::new(&image(&s, id).link.unwrap().path), Path::new(&moved), "{doc}");
    }
}

/// A file found by name in the document's folder, then missing before the next save, keeps the
/// place it was last found: the saved relative path names that place, not the folder its old
/// relative path named.
#[test]
fn a_file_found_elsewhere_then_missing_keeps_the_place_it_was_found() {
    let dir = Folder::new("found-then-missing");
    let pic = dir.file("art/photo.png");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    let doc = dir.file("poster.vectorcraft");
    save(&mut s, &doc);
    let beside = dir.file("photo.png");
    std::fs::rename(&pic, &beside).unwrap();
    let r = open(&mut s, &doc);
    assert_eq!(&r["missingLinks"], &json!([]), "{r}");
    assert_eq!(Path::new(&image(&s, id).link.unwrap().path), Path::new(&beside));
    let away = dir.file("away.png");
    std::fs::rename(&beside, &away).unwrap();
    save(&mut s, &doc);
    let mut saved: Value = serde_json::from_slice(&std::fs::read(&doc).unwrap()).unwrap();
    assert_eq!(saved_image(&mut saved)["link"]["relative"].as_str(), Some("photo.png"));
}

/// After a save in another folder, a file Update Links finds by name beside the saved document,
/// then missing before the next save, keeps the place it was found.
#[test]
fn a_file_update_links_finds_after_a_save_elsewhere_keeps_its_place() {
    let a = Folder::new("update-after-save-as-a");
    let b = Folder::new("update-after-save-as-b");
    let pic = a.file("art/photo.png");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    save(&mut s, &a.file("poster.vectorcraft"));
    open(&mut s, &a.file("poster.vectorcraft"));
    let doc = b.file("poster.vectorcraft");
    save(&mut s, &doc);
    let beside = b.file("photo.png");
    std::fs::rename(&pic, &beside).unwrap();
    let r = s.execute("links.update", &json!({"ids": [id.0]})).unwrap();
    assert_eq!(&r["missing"], &json!([]), "{r}");
    assert_eq!(Path::new(&image(&s, id).link.unwrap().path), Path::new(&beside));
    std::fs::remove_file(&beside).unwrap();
    s.execute("document.save", &json!({})).unwrap();
    let mut saved: Value = serde_json::from_slice(&std::fs::read(&doc).unwrap()).unwrap();
    assert_eq!(saved_image(&mut saved)["link"]["relative"].as_str(), Some("photo.png"));
}

/// A file found at its relative path from the folder of the last save, away from its path and from
/// the folder the document was read from, is written relative to where it was found.
#[test]
fn a_file_found_beside_the_last_save_is_written_where_it_was_found() {
    let a = Folder::new("found-beside-save-a");
    let b = Folder::new("found-beside-save-b");
    let c = Folder::new("found-beside-save-c");
    write(&a.file("art/photo.png"), &png(600, 300, RED));
    let mut s = session();
    place(&mut s, &a.file("art/photo.png"));
    save(&mut s, &a.file("poster.vectorcraft"));
    open(&mut s, &a.file("poster.vectorcraft"));
    save(&mut s, &b.file("poster.vectorcraft"));
    std::fs::rename(a.0.join("art"), b.0.join("art")).unwrap();
    save(&mut s, &c.file("poster.vectorcraft"));
    let r = open(&mut s, &c.file("poster.vectorcraft"));
    assert_eq!(&r["missingLinks"], &json!([]), "{r}");
}

/// After a moved document with a missing link is saved in another folder, `links.check` finds the
/// file once it is back at the place its relative path names from the folder the document was
/// read from, which is where the saved file looks for it too.
#[test]
fn a_missing_link_comes_back_where_the_document_saved_elsewhere_looks_for_it() {
    let a = Folder::new("check-after-save-as-a");
    let b = Folder::new("check-after-save-as-b");
    let c = Folder::new("check-after-save-as-c");
    let away = Folder::new("check-after-save-as-away");
    write(&a.file("art/photo.png"), &png(600, 300, RED));
    let mut s = session();
    place(&mut s, &a.file("art/photo.png"));
    save(&mut s, &a.file("poster.vectorcraft"));
    std::fs::remove_dir_all(&b.0).unwrap();
    std::fs::rename(&a.0, &b.0).unwrap();
    std::fs::rename(b.0.join("art"), away.0.join("art")).unwrap();
    open(&mut s, &b.file("poster.vectorcraft"));
    save(&mut s, &c.file("poster.vectorcraft"));
    std::fs::rename(away.0.join("art"), b.0.join("art")).unwrap();
    let r = s.execute("links.check", &json!({})).unwrap();
    let found = r["links"][0]["found"].as_str().map(Path::new);
    assert_eq!((r["missing"].as_u64(), found), (Some(0), Some(Path::new(&b.file("art/photo.png")))), "{r}");
    let r = open(&mut s, &c.file("poster.vectorcraft"));
    assert_eq!(&r["missingLinks"], &json!([]), "{r}");
}

/// Links in symbols, graph designs, patterns and opacity masks are saved with their relative
/// paths, as links in the layers are: after the folder moves, each file is found and its link takes
/// the new path, which the next save writes relative to the document again.
#[test]
fn links_outside_the_layers_find_their_files_after_the_folder_moves() {
    let a = Folder::new("nested-move-a");
    let b = Folder::new("nested-move-b");
    let c = Folder::new("nested-move-c");
    let names = ["layer", "symbol", "design", "pattern", "mask"];
    for (i, name) in (0u8..).zip(names) {
        write(&a.file(&format!("art/{name}.png")), &png(20, 10, [20 + 40 * i, 90, 160]));
    }
    let mut s = session();
    place(&mut s, &a.file("art/layer.png"));
    let id = place(&mut s, &a.file("art/symbol.png"));
    s.execute("symbol.new", &json!({"name": "Mark", "ids": [id.0]})).unwrap();
    let id = place(&mut s, &a.file("art/design.png"));
    s.execute("graph.design", &json!({"new": "Bar"})).unwrap();
    s.execute("edit.clear", &json!({"ids": [id.0]})).unwrap();
    let id = place(&mut s, &a.file("art/pattern.png"));
    s.execute("object.pattern.make", &json!({"ids": [id.0], "edit": false})).unwrap();
    s.execute("edit.clear", &json!({"ids": [id.0]})).unwrap();
    let masked = super::rect(&mut s, 10.0, 10.0, 60.0, 60.0);
    let mask = place(&mut s, &a.file("art/mask.png"));
    s.execute("transparency.makeOpacityMask", &json!({"ids": [masked.0, mask.0]})).unwrap();
    let mut in_layers = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if let NodeKind::Image(im) = &n.kind {
            in_layers.extend(im.link.as_ref().map(|l| l.name().to_string()));
        }
    });
    assert_eq!(in_layers, ["layer.png"], "only one file is shown by an object in the layers");
    // The links' paths, each image checked to show its file's pixels rather than the saved preview.
    let paths = |s: &Session| {
        let d = &s.doc().unwrap().doc;
        let mut out = vec![];
        d.visit_images(|_, im| {
            let l = im.link.as_ref().unwrap();
            assert!(!d.images[&im.key].is_proxy(), "{} shows its preview", l.name());
            out.push(PathBuf::from(&l.path));
        });
        out.sort();
        out
    };
    let files = |dir: &Folder| {
        let mut out: Vec<PathBuf> = names.iter().map(|n| PathBuf::from(dir.file(&format!("art/{n}.png")))).collect();
        out.sort();
        out
    };
    assert_eq!(paths(&s), files(&a));
    save(&mut s, &a.file("poster.vectorcraft"));
    // Move the document with its links: the old folder is gone.
    std::fs::remove_dir_all(&b.0).unwrap();
    std::fs::rename(&a.0, &b.0).unwrap();
    let r = open(&mut s, &b.file("poster.vectorcraft"));
    assert_eq!((r["missingLinks"].clone(), r["modifiedLinks"].clone()), (json!([]), json!([])), "{r}");
    assert_eq!(paths(&s), files(&b), "the links follow the files");
    // Saved again and moved again.
    save(&mut s, &b.file("poster.vectorcraft"));
    std::fs::remove_dir_all(&c.0).unwrap();
    std::fs::rename(&b.0, &c.0).unwrap();
    let r = open(&mut s, &c.file("poster.vectorcraft"));
    assert_eq!((r["missingLinks"].clone(), r["modifiedLinks"].clone()), (json!([]), json!([])), "{r}");
    assert_eq!(paths(&s), files(&c));
}

/// With Update Links: Automatically, a file in a symbol and in the layers that changed and moved
/// with the document is read again for the layers' image and for the symbol's, and both links take
/// its new path, size and hash.
#[test]
fn a_symbols_link_to_a_moved_and_modified_file_takes_its_new_path() {
    let a = Folder::new("nested-modified-a");
    let b = Folder::new("nested-modified-b");
    let pic = a.file("art/photo.png");
    write(&pic, &png(20, 10, RED));
    let mut s = session();
    s.execute("prefs.set", &json!({"key": "updateLinks", "value": "automatically"})).unwrap();
    let id = place(&mut s, &pic);
    s.execute("symbol.new", &json!({"name": "Mark", "ids": [id.0]})).unwrap();
    let id = place(&mut s, &pic);
    save(&mut s, &a.file("poster.vectorcraft"));
    write(&pic, &png(30, 30, BLUE));
    std::fs::remove_dir_all(&b.0).unwrap();
    std::fs::rename(&a.0, &b.0).unwrap();
    let r = open(&mut s, &b.file("poster.vectorcraft"));
    assert_eq!((r["modifiedLinks"].clone(), r["updatedLinks"][0]["ids"].clone()), (json!([]), json!([id.0])), "{r}");
    let moved = PathBuf::from(b.file("art/photo.png"));
    let blue = png(30, 30, BLUE);
    let mut images = vec![];
    s.doc().unwrap().doc.visit_images(|_, im| {
        let l = im.link.as_ref().unwrap();
        images.push((PathBuf::from(&l.path), l.size, l.hash.clone(), im.width, im.height, im.key.clone()));
    });
    let key = image(&s, id).key;
    let new = (moved, Some(blue.len() as u64), Some(vectorcraft_doc::links::hash_bytes(&blue)), 30, 30, key);
    assert_eq!(images, [new.clone(), new], "the layers' image, then the symbol's");
}

/// A link inside a symbol that the document records without a relative path, beside a link in the
/// layers to the same file that has one, takes the file's new path after the folder moves.
#[test]
fn a_symbols_link_without_a_relative_path_follows_the_moved_file() {
    let a = Folder::new("nested-unrelative-a");
    let b = Folder::new("nested-unrelative-b");
    let pic = a.file("art/photo.png");
    write(&pic, &png(20, 10, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    s.execute("symbol.new", &json!({"name": "Mark", "ids": [id.0]})).unwrap();
    place(&mut s, &pic);
    let doc_path = a.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    // Removes the relative path of every link under `v` → how many it removed.
    fn unrelative(v: &mut Value) -> usize {
        match v {
            Value::Object(o) => {
                let own = match o.get_mut("link") {
                    Some(Value::Object(l)) => usize::from(l.remove("relative").is_some()),
                    _ => 0,
                };
                own + o.values_mut().map(unrelative).sum::<usize>()
            }
            Value::Array(a) => a.iter_mut().map(unrelative).sum(),
            _ => 0,
        }
    }
    let mut saved: Value = serde_json::from_slice(&std::fs::read(&doc_path).unwrap()).unwrap();
    assert_eq!(unrelative(&mut saved["document"]["symbols"]), 1, "the symbol's link");
    assert_eq!(unrelative(&mut saved["document"]["layers"].clone()), 1, "the layers' link keeps its relative path");
    std::fs::write(&doc_path, serde_json::to_vec(&saved).unwrap()).unwrap();
    std::fs::remove_dir_all(&b.0).unwrap();
    std::fs::rename(&a.0, &b.0).unwrap();
    let r = open(&mut s, &b.file("poster.vectorcraft"));
    assert_eq!((r["missingLinks"].clone(), r["modifiedLinks"].clone()), (json!([]), json!([])), "{r}");
    let mut paths = vec![];
    s.doc().unwrap().doc.visit_images(|_, im| paths.push(PathBuf::from(&im.link.as_ref().unwrap().path)));
    let moved = PathBuf::from(b.file("art/photo.png"));
    assert_eq!(paths, [moved.clone(), moved], "the layers' link, then the symbol's");
}

/// Update Links reads a changed file again for the images inside symbols, patterns and opacity
/// masks too: those showing the same content as an image in the layers, and those no object in the
/// layers shows. Each takes the file's place as its own Placement Options say, in one undo step.
#[test]
fn update_links_reads_files_again_inside_symbols_patterns_and_masks() {
    let dir = Folder::new("nested-update");
    let (photo, logo) = (dir.file("photo.png"), dir.file("logo.png"));
    write(&photo, &png(20, 10, RED));
    write(&logo, &png(20, 10, [200, 200, 20]));
    let mut s = session();
    // photo.png in the layers, and in a symbol filling its bounds, clipped to them.
    let id = place(&mut s, &photo);
    s.execute("links.placementOptions", &json!({"ids": [id.0], "preserve": "fill", "clip": true})).unwrap();
    s.execute("symbol.new", &json!({"name": "Mark", "ids": [id.0]})).unwrap();
    let id = place(&mut s, &photo);
    // logo.png only in a pattern and in an opacity mask.
    let p = place(&mut s, &logo);
    s.execute("object.pattern.make", &json!({"ids": [p.0], "edit": false})).unwrap();
    s.execute("edit.clear", &json!({"ids": [p.0]})).unwrap();
    let masked = super::rect(&mut s, 10.0, 10.0, 60.0, 60.0);
    let mask = place(&mut s, &logo);
    s.execute("transparency.makeOpacityMask", &json!({"ids": [masked.0, mask.0]})).unwrap();
    // photo.png wider than the symbol's image is: filling its bounds overflows them.
    let (blue, green) = (png(60, 20, BLUE), png(40, 20, [20, 200, 20]));
    write(&photo, &blue);
    write(&logo, &green);
    let u = s.execute("links.update", &json!({})).unwrap();
    assert_eq!(u, json!({"updated": [id.0], "missing": []}));
    // (name, size, hash, pixel size) of each image, which shows its file's pixels.
    let images = |s: &Session| {
        let d = &s.doc().unwrap().doc;
        let mut out = vec![];
        d.visit_images(|_, im| {
            let l = im.link.as_ref().unwrap();
            assert!(!d.images[&im.key].is_proxy(), "{} shows its preview", l.name());
            out.push((l.name().to_string(), l.size, l.hash.clone(), (im.width, im.height)));
        });
        out.sort();
        out
    };
    let shown = |name: &str, bytes: &[u8], size| (name.to_string(), Some(bytes.len() as u64), Some(vectorcraft_doc::links::hash_bytes(bytes)), size);
    let logo_now = shown("logo.png", &green, (40, 20));
    let photo_now = shown("photo.png", &blue, (60, 20));
    assert_eq!(images(&s), [logo_now.clone(), logo_now, photo_now.clone(), photo_now]);
    // The symbol's image is clipped to its old bounds.
    let mut clipped = false;
    s.doc().unwrap().doc.symbols[0].art.walk(&mut |n| {
        if let NodeKind::Group { children, clip: true } = &n.kind {
            clipped |=
                matches!(&children[..], [c, im] if matches!(c.kind, NodeKind::Path { clipping: true, .. }) && matches!(im.kind, NodeKind::Image(_)));
        }
    });
    assert!(clipped, "{:?}", s.doc().unwrap().doc.symbols[0].art);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(images(&s).iter().all(|(_, _, _, size)| *size == (20, 10)), "{:?}", images(&s));
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

/// The path of the file image `id` links to.
fn link_path(s: &Session, id: NodeId) -> String {
    image(s, id).link.map(|l| l.path).unwrap_or_default()
}

/// Replace: All Instances relinks every image linked to the same file as the one given, in one
/// undo step; Selected Instances (the default) only the one given.
#[test]
fn relink_all_instances_takes_every_image_of_the_same_file() {
    let dir = Folder::new("relink-all");
    let (pic, other) = (dir.file("photo.png"), dir.file("other.png"));
    write(&pic, &png(60, 30, RED));
    write(&other, &png(60, 30, RED));
    let mut s = session();
    let (a, b, c) = (place(&mut s, &pic), place(&mut s, &pic), place(&mut s, &other));
    let blue = dir.file("new/blue.png");
    write(&blue, &png(60, 30, BLUE));
    let r = s.execute("links.relink", &json!({"ids": [a.0], "path": blue, "scope": "selected"})).unwrap();
    assert_eq!(r, json!({"relinked": [a.0], "notFound": []}));
    assert_eq!(link_path(&s, b), pic, "selected instances: the other copy keeps its file");
    s.execute("edit.undo", &json!({})).unwrap();
    let r = s.execute("links.relink", &json!({"ids": [a.0], "path": blue, "scope": "all"})).unwrap();
    assert_eq!(r, json!({"relinked": [a.0, b.0], "notFound": []}));
    assert_eq!((link_path(&s, a), link_path(&s, b), link_path(&s, c)), (blue.clone(), blue.clone(), other.clone()));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((link_path(&s, a), link_path(&s, b)), (pic.clone(), pic.clone()), "one undo step");
    // The selection stands in for ids.
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    let r = s.execute("links.relink", &json!({"path": blue, "scope": "all"})).unwrap();
    assert_eq!(r["relinked"], json!([b.0, a.0]));
}

/// Replace: Missing Links from This Folder relinks the given links to the chosen file, then every
/// other missing link to the file of its name in that file's folder, in one undo step; names not
/// there are listed in notFound and stay missing.
#[test]
fn relink_missing_links_from_this_folder_finds_the_others_by_name() {
    let dir = Folder::new("relink-missing-folder");
    let names = ["a.png", "b.png", "c.png", "d.png"];
    let mut s = session();
    let ids: Vec<NodeId> = names
        .iter()
        .map(|n| {
            write(&dir.file(n), &png(60, 30, RED));
            place(&mut s, &dir.file(n))
        })
        .collect();
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    // d.png stays where it was: it isn't missing.
    for n in &names[..3] {
        std::fs::remove_file(dir.file(n)).unwrap();
    }
    open(&mut s, &doc_path);
    let found = dir.file("found/a new name.png");
    write(&found, &png(60, 30, BLUE));
    write(&dir.file("found/b.png"), &png(60, 30, BLUE));
    write(&dir.file("found/d.png"), &png(60, 30, BLUE));
    let r = s.execute("links.relink", &json!({"ids": [ids[0].0], "path": found, "scope": "missingInFolder"})).unwrap();
    assert_eq!(r, json!({"relinked": [ids[0].0, ids[1].0], "notFound": ["c.png"]}));
    let b_found = dir.0.join("found").join("b.png").to_string_lossy().into_owned();
    assert_eq!((link_path(&s, ids[0]), link_path(&s, ids[1])), (found.clone(), b_found));
    assert_eq!((link_path(&s, ids[2]), link_path(&s, ids[3])), (dir.file("c.png"), dir.file("d.png")));
    let c = s.execute("links.check", &json!({})).unwrap();
    assert_eq!(c["missing"], 1, "{c}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.execute("links.check", &json!({})).unwrap()["missing"], 3, "one undo step");
}

/// Replace: Each Link Separately takes one file for each link, in one undo step.
#[test]
fn relink_each_link_separately_takes_a_file_for_each() {
    let dir = Folder::new("relink-each");
    let (one, two) = (dir.file("one.png"), dir.file("two.png"));
    write(&one, &png(60, 30, RED));
    write(&two, &png(60, 30, RED));
    let mut s = session();
    let (a, b, c) = (place(&mut s, &one), place(&mut s, &one), place(&mut s, &two));
    let (x, y) = (dir.file("new/x.png"), dir.file("new/y.png"));
    write(&x, &png(60, 30, BLUE));
    write(&y, &png(30, 30, BLUE));
    let paths = json!([{"ids": [a.0, b.0], "path": x}, {"ids": [c.0], "path": y}]);
    let r = s.execute("links.relink", &json!({"scope": "each", "paths": paths})).unwrap();
    assert_eq!(r, json!({"relinked": [a.0, b.0, c.0], "notFound": []}));
    assert_eq!((link_path(&s, a), link_path(&s, b), link_path(&s, c)), (x.clone(), x.clone(), y.clone()));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((link_path(&s, a), link_path(&s, c)), (one.clone(), two.clone()), "one undo step");
    // Two entries naming the same file read it once.
    let paths = json!([{"ids": [a.0], "path": y}, {"ids": [c.0], "path": y}]);
    let r = s.execute("links.relink", &json!({"scope": "each", "paths": paths})).unwrap();
    assert_eq!(r["relinked"], json!([a.0, c.0]));
}

/// With copy, a file that can't be shown stops the call before any file is copied into the
/// document's folder.
#[test]
fn relink_each_with_copy_copies_nothing_when_a_file_cant_be_read() {
    let dir = Folder::new("relink-each-copy");
    let (one, two) = (dir.file("doc/one.png"), dir.file("doc/two.png"));
    write(&one, &png(60, 30, RED));
    write(&two, &png(60, 30, RED));
    let mut s = session();
    let (a, c) = (place(&mut s, &one), place(&mut s, &two));
    save(&mut s, &dir.file("doc/poster.vectorcraft"));
    let (x, bad) = (dir.file("new/x.png"), dir.file("new/broken.png"));
    write(&x, &png(60, 30, BLUE));
    write(&bad, b"not an image");
    let paths = json!([{"ids": [a.0], "path": x}, {"ids": [c.0], "path": bad}]);
    assert!(s.execute("links.relink", &json!({"scope": "each", "paths": paths, "copy": true})).is_err());
    assert!(!Path::new(&dir.file("doc/x.png")).exists(), "no copy is left behind");
    assert_eq!(link_path(&s, a), one);
}

/// With copy, for a client whose read roots don't hold the document's folder: the reply names the
/// copy by its file name, and a copy refused there keeps the write root and names the folder and
/// the file by their names.
#[test]
fn relink_with_copy_names_the_documents_folder_by_its_name_outside_the_read_roots() {
    use crate::file_access::{AutomationRoots, confine};

    let dir = Folder::new("relink-copy-roots");
    let one = dir.file("doc/one.png");
    write(&one, &png(60, 30, RED));
    let mut s = session();
    let a = place(&mut s, &one);
    save(&mut s, &dir.file("doc/poster.vectorcraft"));
    let x = dir.file("new/x.png");
    write(&x, &png(60, 30, BLUE));
    let (new, doc) = (Path::new(&x).parent().unwrap().to_path_buf(), Path::new(&one).parent().unwrap().to_path_buf());
    let hidden = doc.to_string_lossy().into_owned();
    let p = json!({"ids": [a.0], "path": x, "copy": true});

    let writes_there = AutomationRoots::new(&[new.as_path()], &[doc.as_path()]).unwrap();
    let r = confine(writes_there.as_ref(), || s.execute("links.relink", &p)).unwrap();
    assert_eq!(r["copied"], json!([{"from": x, "to": "x.png"}]), "{r}");
    assert!(!r.to_string().contains(&hidden), "{r}");
    std::fs::remove_file(dir.file("doc/x.png")).unwrap();

    let elsewhere = AutomationRoots::new(&[new.as_path()], &[new.as_path()]).unwrap();
    let root = crate::cmd::findfiles::plain(std::fs::canonicalize(&new).unwrap()).display().to_string();
    let e = confine(elsewhere.as_ref(), || s.execute("links.relink", &p)).unwrap_err().to_string();
    assert_eq!(e, format!("doc: automation path rejected: outside the write root {root}: x.png"));
    assert!(!Path::new(&dir.file("doc/x.png")).exists());
}

/// With copy, for a document whose path has no folder: outside the read roots the error does not
/// name the path, and without roots it names it whole.
#[test]
fn relink_with_copy_names_a_document_path_without_a_folder_as_the_request_may_see_it() {
    use crate::file_access::{AutomationRoots, confine};

    let dir = Folder::new("relink-copy-no-folder");
    let mut s = session();
    s.doc_mut().unwrap().path = Some("/".into());
    let p = json!({"path": dir.file("x.png"), "copy": true});
    let roots = AutomationRoots::new(&[dir.0.as_path()], &[dir.0.as_path()]).unwrap();
    let e = confine(roots.as_ref(), || s.execute("links.relink", &p)).unwrap_err().to_string();
    assert_eq!(e, "invalid parameters for `links.relink`: a file or folder outside the read roots has no folder");
    let whole = s.execute("links.relink", &p).unwrap_err().to_string();
    let doc = crate::cmd::fileio::absolute_path("/");
    assert_eq!(whole, format!("invalid parameters for `links.relink`: {doc} has no folder"));
}

/// A scope or paths that doesn't fit is refused, and nothing is relinked.
#[test]
fn relink_refuses_scopes_and_paths_that_dont_fit() {
    let dir = Folder::new("relink-bad-scope");
    let pic = dir.file("photo.png");
    write(&pic, &png(60, 30, RED));
    let mut s = session();
    let a = place(&mut s, &pic);
    let blue = dir.file("blue.png");
    write(&blue, &png(60, 30, BLUE));
    let bad = [
        json!({"ids": [a.0], "path": blue, "scope": "everything"}),
        json!({"ids": [a.0], "path": blue, "scope": 3}),
        json!({"scope": "each"}),
        json!({"scope": "each", "paths": []}),
        json!({"scope": "each", "paths": [{"ids": [a.0]}]}),
        json!({"scope": "each", "paths": [{"path": blue}]}),
        json!({"scope": "each", "paths": [{"ids": ["one"], "path": blue}]}),
        json!({"scope": "each", "paths": [{"ids": [a.0], "path": blue}, {"ids": [a.0], "path": pic}]}),
        json!({"scope": "each", "paths": [{"ids": [a.0], "path": blue}], "path": blue}),
        json!({"scope": "each", "paths": [{"ids": [a.0], "path": blue}], "ids": [a.0]}),
        json!({"scope": "all", "paths": [{"ids": [a.0], "path": blue}]}),
        json!({"ids": [a.0], "paths": [{"ids": [a.0], "path": blue}]}),
        json!({"ids": [a.0], "folder": dir.0.to_string_lossy(), "scope": "missingInFolder"}),
        json!({"ids": [a.0], "scope": "missingInFolder"}),
    ];
    for p in bad {
        assert!(s.execute("links.relink", &p).is_err(), "{p}");
        assert_eq!(link_path(&s, a), pic, "{p} relinks nothing");
    }
}

/// A link stored as a classic Mac OS path (`Disk:Folder:file`, as older `.ai` files keep them) is
/// named by its last component: the Links panel shows that name and its folder, Relink to Folder
/// looks for that name, and the document's own folder is searched for it. The stored path is kept.
#[test]
fn a_classic_mac_path_is_found_by_its_file_name() {
    const CLASSIC: &str = "Macintosh HD:Images:photo.png";
    let dir = Folder::new("classic-mac");
    let pic = dir.file("art/photo.png");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    place(&mut s, &pic);
    let mut v: Value = serde_json::from_slice(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    saved_image(&mut v)["link"] = json!({ "path": CLASSIC });
    let doc_path = dir.file("poster.vectorcraft");
    write(&doc_path, &serde_json::to_vec(&v).unwrap());
    let r = open(&mut s, &doc_path);
    let id = s.doc().unwrap().doc.layers[0].children().unwrap()[0].id;
    assert_eq!(r["missingLinks"], json!([{"name": "photo.png", "path": CLASSIC, "ids": [id.0]}]));
    let i = s.execute("links.info", &json!({"id": id.0})).unwrap();
    assert_eq!(
        (i["fileName"].as_str(), i["location"].as_str(), i["path"].as_str()),
        (Some("photo.png"), Some("Macintosh HD:Images"), Some(CLASSIC)),
        "{i}"
    );
    // Relink to Folder: the file of the link's name there.
    let r = s.execute("links.relink", &json!({"ids": [id.0], "folder": dir.file("found")})).unwrap();
    assert_eq!(r, json!({"relinked": [], "notFound": ["photo.png"]}));
    write(&dir.file("found/photo.png"), &png(600, 300, BLUE));
    let r = s.execute("links.relink", &json!({"ids": [id.0], "folder": dir.file("found")})).unwrap();
    assert_eq!(r, json!({"relinked": [id.0], "notFound": []}));
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE));
    // The document's folder: the file of the link's name beside the document.
    write(&dir.file("photo.png"), &png(600, 300, RED));
    let r = open(&mut s, &doc_path);
    assert_eq!((r["missingLinks"].clone(), r["modifiedLinks"].clone()), (json!([]), json!([])), "{r}");
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
}

/// A `w`×`h` PNG of noise cut to half its length: its header reads, its pixels don't.
pub(crate) fn cut_png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        let v = x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503).rotate_left(13);
        image::Rgb([v as u8, (v >> 8) as u8, (v >> 16) as u8])
    });
    let mut out = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out.truncate(out.len() / 2);
    out
}

/// A linked file whose pixels can't be read (cut short) isn't shown: Relink is an error, and
/// Update Links and Open (Update Links: Automatically) count it as missing. The image keeps what
/// it showed.
#[test]
fn a_linked_file_whose_pixels_cant_be_read_is_not_shown() {
    let dir = Folder::new("cut");
    let pic = dir.file("photo.png");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    let shown = image(&s, id);
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    let cut = dir.file("cut.png");
    write(&cut, &cut_png(300, 100));
    let e = s.execute("links.relink", &json!({"ids": [id.0], "path": cut})).unwrap_err();
    assert!(e.to_string().ends_with("the image's pixels can't be read"), "{e}");
    write(&pic, &cut_png(300, 100));
    assert_eq!(s.execute("links.update", &json!({})).unwrap(), json!({"updated": [], "missing": [id.0]}));
    assert_eq!(image(&s, id), shown);
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED));
    s.execute("prefs.set", &json!({"key": "updateLinks", "value": "automatically"})).unwrap();
    let r = open(&mut s, &doc_path);
    assert_eq!((r["missingLinks"][0]["ids"].clone(), r["updatedLinks"].clone()), (json!([id.0]), json!([])), "{r}");
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED), "the image still shows its preview");
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

/// Preferences › File Handling › Use Low Resolution Proxy for Linked EPS (#394): an unchanged linked
/// `.eps` keeps its saved preview on open; Update Links still reads a modified file when set to
/// Automatically.
#[test]
fn low_res_proxy_eps_keeps_unchanged_preview_and_automatic_update_reads_modified_eps() {
    let dir = Folder::new("eps-proxy");
    let pic = dir.file("photo.eps");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    s.execute("prefs.set", &json!({"key": "lowResProxyEps", "value": true})).unwrap();
    s.execute("prefs.set", &json!({"key": "updateLinks", "value": "askWhenModified"})).unwrap();
    let r = open(&mut s, &doc_path);
    assert_eq!((r["modifiedLinks"].clone(), r["updatedLinks"].clone()), (json!([]), json!([])), "{r}");
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED), "unchanged EPS stays on its preview");
    assert!(s.doc().unwrap().doc.images[&image(&s, id).key].is_proxy());
    write(&pic, &png(400, 400, BLUE));
    s.execute("prefs.set", &json!({"key": "updateLinks", "value": "automatically"})).unwrap();
    let r = open(&mut s, &doc_path);
    assert_eq!(r["updatedLinks"][0]["ids"].clone(), json!([id.0]), "modified EPS updates on open: {r}");
    assert!(!s.doc().unwrap().doc.images[&image(&s, id).key].is_proxy());
    assert!(near(centre_colour(&s.doc().unwrap().doc), BLUE));
}

/// With Low Resolution Proxy on, export reads the linked EPS file's full pixels, not the preview.
#[test]
fn low_res_proxy_eps_export_uses_full_resolution_from_the_linked_file() {
    let dir = Folder::new("eps-proxy-export");
    let pic = dir.file("photo.eps");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    s.execute("prefs.set", &json!({"key": "lowResProxyEps", "value": true})).unwrap();
    let r = open(&mut s, &doc_path);
    assert_eq!((r["modifiedLinks"].clone(), r["updatedLinks"].clone()), (json!([]), json!([])), "{r}");
    let key = image(&s, id).key;
    assert!(s.doc().unwrap().doc.images[&key].is_proxy());
    let (full, warnings) = cmd::links::full_linked_images(&s.doc().unwrap().doc, Default::default());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(!full.images[&key].is_proxy());
    let dim =
        image::ImageReader::new(std::io::Cursor::new(full.images[&key].bytes.as_ref())).with_guessed_format().unwrap().into_dimensions().unwrap();
    assert_eq!(dim, (600, 300));
    let png_out = s.execute("document.export", &json!({"format": "png"})).unwrap();
    let bytes = vectorcraft_format::base64_decode(png_out["dataBase64"].as_str().unwrap()).unwrap();
    let rendered = image::load_from_memory(&bytes).unwrap();
    assert!(rendered.width() >= 400 && rendered.height() >= 300);
    assert!(near(centre_colour(&s.doc().unwrap().doc), RED), "canvas still shows the preview");
}

/// An SVG linking two missing files: once one of them is found, export reads its pixels into its
/// own images only, and the other file's image keeps its placeholder.
#[test]
fn export_reads_a_found_svg_link_into_its_own_images_only() {
    let dir = Folder::new("svg-missing-keys");
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><image href="a.png" width="20" height="20"/><image href="b.png" x="20" width="20" height="20"/></svg>"#;
    let svg_path = dir.file("art.svg");
    write(&svg_path, svg.as_bytes());
    let mut s = session();
    open(&mut s, &svg_path);
    let mut found = vec![];
    s.doc().unwrap().doc.visit_images(|_, im| found.push(im.clone()));
    let [a, b] = &found[..] else { panic!("two images: {found:?}") };
    assert!(a.link.as_ref().unwrap().path.ends_with("a.png") && b.link.as_ref().unwrap().path.ends_with("b.png"));
    let placeholder = s.doc().unwrap().doc.images[&b.key].bytes.clone();

    let bytes = png(20, 20, RED);
    write(&dir.file("a.png"), &bytes);
    let (full, warnings) = cmd::links::full_linked_images(&s.doc().unwrap().doc, Default::default());
    assert_eq!(warnings.len(), 1, "b.png is still missing: {warnings:?}");
    assert_eq!(*full.images[&a.key].bytes, bytes, "a.png's pixels");
    assert_eq!(full.images[&b.key].bytes, placeholder, "b.png keeps its placeholder");
}

/// An SVG linking two files with the same pixels, saved with previews only: once one file is gone
/// and the other has changed, export reads the changed file into its own images only.
#[test]
fn export_reads_a_changed_svg_link_into_its_own_images_only() {
    let dir = Folder::new("svg-same-pixels");
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><image href="a.png" width="20" height="20"/><image href="b.png" x="20" width="20" height="20"/></svg>"#;
    let svg_path = dir.file("art.svg");
    write(&svg_path, svg.as_bytes());
    write(&dir.file("a.png"), &png(600, 300, RED));
    write(&dir.file("b.png"), &png(600, 300, RED));
    let mut s = session();
    open(&mut s, &svg_path);
    let doc_path = dir.file("art.vectorcraft");
    save(&mut s, &doc_path);
    std::fs::remove_file(dir.file("a.png")).unwrap();
    let blue = png(600, 300, BLUE);
    write(&dir.file("b.png"), &blue);

    open(&mut s, &doc_path);
    let mut found = vec![];
    s.doc().unwrap().doc.visit_images(|_, im| found.push(im.clone()));
    let [a, b] = &found[..] else { panic!("two images: {found:?}") };
    assert!(a.link.as_ref().unwrap().path.ends_with("a.png") && b.link.as_ref().unwrap().path.ends_with("b.png"));
    assert_ne!(a.key, b.key, "two files, two keys");
    let doc = &s.doc().unwrap().doc;
    assert!(doc.images[&a.key].is_proxy() && doc.images[&b.key].is_proxy(), "previews");
    let preview = doc.images[&a.key].bytes.clone();
    let (full, warnings) = cmd::links::full_linked_images(doc, Default::default());
    assert_eq!(warnings.len(), 1, "a.png is missing: {warnings:?}");
    assert_eq!(*full.images[&b.key].bytes, blue, "b.png's new pixels");
    assert_eq!(full.images[&a.key].bytes, preview, "a.png keeps its preview");
}

/// The half of opening that the desktop app runs on a thread of its own (`read_for_open`) reads
/// the linked files and lays out the type. The half left for the UI thread (`open_read`) reads no
/// file: with a linked file moved away in between, it opens the document that `document.open` opens.
#[test]
fn a_file_read_for_opening_has_its_links_read_and_its_type_laid_out() {
    let dir = Folder::new("read-for-open");
    let tif = dir.file("art/photo.tif");
    let mut tiff = vec![];
    image::RgbImage::from_pixel(600, 300, image::Rgb(RED)).write_to(&mut std::io::Cursor::new(&mut tiff), image::ImageFormat::Tiff).unwrap();
    write(&tif, &tiff);
    let (gone, changed) = (dir.file("gone.png"), dir.file("changed.png"));
    write(&gone, &png(300, 100, BLUE));
    write(&changed, &png(300, 100, BLUE));
    let mut s = session();
    let photo = place(&mut s, &tif);
    place(&mut s, &gone);
    place(&mut s, &changed);
    let area = json!({"width": 150, "height": 20, "fit": "autoHeight"});
    let r = s.execute("text.create", &json!({"x": 20, "y": 40, "text": "Linked files and type. ".repeat(12), "area": area})).unwrap();
    let text = NodeId(r["id"].as_u64().unwrap());
    s.execute("text.areaOptions", &json!({"ids": [text.0], "columns": 2})).unwrap();
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    std::fs::remove_file(&gone).unwrap();
    write(&changed, &png(200, 200, RED));
    let bytes = std::fs::read(&doc_path).unwrap();
    let away = dir.file("away.tif");
    for update in ["askWhenModified", "automatically"] {
        let mut at_once = session();
        at_once.execute("prefs.set", &json!({"key": "updateLinks", "value": update})).unwrap();
        let expected = open(&mut at_once, &doc_path);
        let rows = if update == "automatically" { &expected["updatedLinks"] } else { &expected["modifiedLinks"] };
        assert!(expected["missingLinks"][0]["name"] == "gone.png" && rows[0]["name"] == "changed.png", "{update}: {expected}");
        let file = cmd::fileio::read_for_open(&doc_path, &bytes, Some(doc_path.clone()), &Value::Null, update == "automatically", false).unwrap();
        let doc = file.document();
        let NodeKind::Image(im) = &doc.node(photo).unwrap().kind else { panic!("not an image") };
        assert!(!doc.images[&im.key].is_proxy(), "{update}: the TIFF is read, not the preview saved for it");
        let NodeKind::Text(t) = &doc.node(text).unwrap().kind else { panic!("not type") };
        assert!(t.cached_bounds.is_some(), "{update}: the type is laid out");
        std::fs::rename(&tif, &away).unwrap();
        let mut opened = session();
        let r = cmd::fileio::open_read(&mut opened, file).unwrap();
        std::fs::rename(&away, &tif).unwrap();
        assert_eq!(r, expected, "{update}");
        let (a, b) = (at_once.doc().unwrap(), opened.doc().unwrap());
        assert!(*b.doc == *a.doc, "{update}: the same document");
        assert!(b.history.undo.is_empty() && !b.is_dirty(), "{update}");
    }
}

/// Opening lays out each type object with the plans the objects before it left on the font's
/// face: its bounds and baselines are those of a layout in a font database of its own, which holds
/// no plans. The objects share a script and direction and differ in features and language. Only
/// bundled Latin glyphs are set, which every font database holds.
#[test]
fn type_read_for_opening_has_the_layout_of_a_font_database_with_no_plans() {
    let dir = Folder::new("read-for-open-plans");
    let style = vectorcraft_doc::CharStyle { size: 100.0, ..Default::default() };
    let manual = vectorcraft_doc::CharStyle { kerning: Some(0.0), ..style.clone() };
    let turkish = vectorcraft_doc::CharStyle { language: Some("tr".into()), ..style.clone() };
    let mut doc = Document::new(800.0, 600.0);
    let layer = doc.layers[0].id;
    for (text, style) in [("AVAV", &style), ("AVAV", &manual), ("office", &turkish), ("To Wa", &style), ("To Wa", &manual)] {
        let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(20.0, 120.0), text, style.clone());
        let id = doc.alloc_id();
        doc.insert(Some(layer), usize::MAX, vectorcraft_doc::Node::new(id, NodeKind::Text(Box::new(t)))).unwrap();
    }
    let doc_path = dir.file("type.vectorcraft");
    let bytes = vectorcraft_format::save_file(&doc);
    let file = cmd::fileio::read_for_open(&doc_path, &bytes, Some(doc_path.clone()), &Value::Null, false, false).unwrap();
    let mut seen = 0;
    file.document().walk(|node| {
        if let NodeKind::Text(t) = &node.kind {
            let fresh = vectorcraft_text::layout(&vectorcraft_text::FontDb::with_font_dirs(vec![]), t);
            assert_eq!(t.cached_bounds, Some(fresh.bounds), "{:?}", t.plain_text());
            assert_eq!(t.cached_baselines, fresh.baselines(), "{:?}", t.plain_text());
            seen += 1;
        }
    });
    assert_eq!(seen, 5);
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
        pixels: None,
    };
    d.insert(Some(lid), 0, vectorcraft_doc::Node::new(nid, NodeKind::Image(Box::new(im)))).unwrap();
    let rel = |dest: String| {
        let d = cmd::links::with_relative_paths(&d, Default::default(), &dest).unwrap_or_else(|| d.clone());
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

/// The placed (linked) files of a `.ai` file come in with the preview the file keeps of them: one
/// not found shows it, one found is read at once into the same bounds, which isn't a change. A copy
/// saved without PDF compatibility, placed without Link, reads the found file too.
#[test]
fn links_of_an_imported_file_are_read_where_they_are_found() {
    use vectorcraft_testkit::ai;
    let dir = Folder::new("imported");
    let pic = dir.file("photo.png").replace('\\', "/");
    // A 2×1 preview (premultiplied ARGB: a blue at half opacity) of a file of 600 × 300 px placed at
    // 60 × 30 pt, which the PDF part draws there too.
    let place = format!(
        "%AI5_BeginPlace\n[0.1 0 0 -0.1 20 80] 0 0 600 300 4 4 0\n({pic})`\n%AI26_BeginPlacedObjectPreview\n2 1 8 32 17 \x7f\x10\x10\x60\x7f\x10\x10\x60\n%AI26_EndPlacedObjectPreview\n~\n%AI5_EndPlace\n"
    );
    let doc_path = dir.file("poster.ai");
    let data = ai::editing_data(200.0, 100.0, &ai::layer("Art", &place));
    write(&doc_path, &ai::ai(data.as_bytes(), "0.56 0.56 0.87 rg 20 50 60 30 re f"));
    let placed = |s: &Session| {
        let mut found = vec![];
        s.doc().unwrap().doc.visit_images(|n, im| found.push((n.geometric_bounds().unwrap(), im.clone())));
        let [one] = &found[..] else { panic!("one image: {found:?}") };
        one.clone()
    };
    let in_place = |b: Rect| [b.x0 - 20.0, b.y0 - 20.0, b.x1 - 80.0, b.y1 - 50.0].iter().all(|d| d.abs() < 1e-6);
    let mut s = session();

    let r = open(&mut s, &doc_path);
    assert_eq!((r["missingLinks"][0]["name"].as_str(), r["missingLinks"][0]["path"].as_str()), (Some("photo.png"), Some(pic.as_str())), "{r}");
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("low-resolution preview")), "{r}");
    let (b, im) = placed(&s);
    assert!(in_place(b) && im.width == 2 && s.doc().unwrap().doc.images[&im.key].is_proxy(), "{b:?} {im:?}");

    write(&pic, &png(600, 300, RED));
    let r = open(&mut s, &doc_path);
    assert_eq!((&r["missingLinks"], &r["modifiedLinks"], &r["updatedLinks"]), (&json!([]), &json!([]), &json!([])), "{r}");
    let (b, im) = placed(&s);
    assert!(in_place(b) && (im.width, im.height) == (600, 300) && im.link.as_ref().is_some_and(|l| l.hash.is_some()), "{b:?} {im:?}");
    assert!(!s.doc().unwrap().doc.images[&im.key].is_proxy());

    let alone = dir.file("alone.ai");
    write(&alone, &ai::ai(data.as_bytes(), "BT /F1 12 Tf 10 50 Td (Saved without its PDF part) Tj ET"));
    let mut s = session();
    s.execute("file.place", &json!({"path": alone, "link": false})).unwrap();
    let (_, im) = placed(&s);
    assert!((im.width, im.height) == (600, 300) && !s.doc().unwrap().doc.images[&im.key].is_proxy(), "{im:?}");
}

/// `poster.ai` in `dir`, a `.ai` file another app wrote, placing `src/photo.png` (relative to it)
/// under a path that no longer exists → its path.
fn imported_with_placed_file(dir: &Folder) -> String {
    use vectorcraft_testkit::ai;
    std::fs::create_dir_all(dir.0.join("src")).unwrap();
    let place = "%AI5_BeginPlace\n%%FileType: 1\n[0.1 0 0 -0.1 20 80] 0 0 600 300 4 4 0\n(/no/such/folder/src/photo.png)`\n\
         %%IncludeFile:/no/such/folder/src/photo.png\n%AI17_Begin_Content_if_version_gt:24 12\n%AI28_BeginPlacedRelativePath\n(src/)\n\
         %AI28_EndPlacedRelativePath\n%AI17_Alternate_Content\n%AI17_End_Versioned_Content\n%AI26_BeginPlacedObjectPreview\n\
         2 1 8 32 17 \x7f\x10\x10\x60\x7f\x10\x10\x60\n%AI26_EndPlacedObjectPreview\n~\n%AI5_EndPlace\n";
    let doc_path = dir.file("poster.ai");
    let data = ai::editing_data(200.0, 100.0, &ai::layer("Art", place));
    write(&doc_path, &ai::ai(data.as_bytes(), "0.56 0.56 0.87 rg 20 50 60 30 re f"));
    doc_path
}

/// A placed file of a `.ai` file whose folder moved with the document is found by the folder the
/// file keeps relative to the document's (`src/`), and its link takes the size, modification time
/// and hash of the file found there. Not found, the link keeps no such details.
#[test]
fn links_of_an_imported_file_are_found_by_their_relative_folder() {
    let dir = Folder::new("imported-relative");
    let pic = dir.file("src/photo.png").replace('\\', "/");
    let doc_path = imported_with_placed_file(&dir);
    let link = |s: &Session| {
        let mut found = vec![];
        s.doc().unwrap().doc.visit_images(|_, im| found.push(im.clone()));
        let [im] = &found[..] else { panic!("one image: {found:?}") };
        (im.link.clone().unwrap(), s.doc().unwrap().doc.images[&im.key].is_proxy())
    };
    let mut s = session();

    let r = open(&mut s, &doc_path);
    assert_eq!(r["missingLinks"][0]["name"].as_str(), Some("photo.png"), "{r}");
    let (l, proxy) = link(&s);
    assert_eq!((l.path.as_str(), l.relative.as_deref()), ("/no/such/folder/src/photo.png", Some("src/photo.png")));
    assert!(proxy && l.size.is_none() && l.modified.is_none() && l.hash.is_none(), "{l:?}");
    // Saved beside it while the file is missing, the document keeps the folder the file is in.
    let native = dir.file("poster.vectorcraft");
    save(&mut s, &native);
    let mut saved: Value = serde_json::from_slice(&std::fs::read(&native).unwrap()).unwrap();
    assert_eq!(saved_image(&mut saved)["link"]["relative"].as_str(), Some("src/photo.png"));

    let bytes = png(600, 300, RED);
    write(&pic, &bytes);
    let r = open(&mut s, &native);
    assert_eq!(&r["missingLinks"], &json!([]), "the native copy finds the file: {r}");
    let r = open(&mut s, &doc_path);
    assert_eq!(&r["missingLinks"], &json!([]), "{r}");
    let (l, proxy) = link(&s);
    assert!(!proxy, "the file's pixels, not the preview");
    assert_eq!(l.path.replace('\\', "/"), pic);
    assert_eq!((l.size, l.hash.clone()), (Some(bytes.len() as u64), Some(vectorcraft_doc::links::hash_bytes(&bytes))));
    assert!(l.modified.is_some(), "{l:?}");
}

/// A Data Recovery copy of such a document keeps the file its links' relative paths name places
/// from: restored and saved beside the `.ai` file, it keeps the folder the placed file is in.
#[test]
fn a_recovered_copy_of_an_imported_file_keeps_its_links_folder() {
    let dir = Folder::new("imported-recovered");
    let doc_path = imported_with_placed_file(&dir);
    let store = std::sync::Arc::new(crate::cmd::recovery::MemoryStore::default());
    let mut s = session();
    s.recovery.set_store(store.clone());
    open(&mut s, &doc_path);
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
    let r = s.execute("file.recovery.save", &json!({})).unwrap();
    assert_eq!(r["saved"].as_array().map(Vec::len), Some(1), "{r}");
    drop(s);
    let mut s = Session::new();
    s.recovery.set_store(store);
    let r = s.execute("file.recovery.restore", &json!({})).unwrap();
    assert_eq!(r["restored"].as_array().map(Vec::len), Some(1), "{r}");
    let native = dir.file("poster.vectorcraft");
    save(&mut s, &native);
    let mut saved: Value = serde_json::from_slice(&std::fs::read(&native).unwrap()).unwrap();
    assert_eq!(saved_image(&mut saved)["link"]["relative"].as_str(), Some("src/photo.png"));
}
