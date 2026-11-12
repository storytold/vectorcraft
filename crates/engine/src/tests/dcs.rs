//! DCS files placed with Link (`file.place`): one linked image of their plates, their spot plates
//! as spot swatches, clipped by their composite's clipping path, never opened or embedded; and
//! the Links panel's Show DCS Transparency Interactions (`links.list` with `dcsTransparency`).
//! The files are made from scratch as http://www.telegraphics.net/doc/DCS.txt describes them.

use std::path::PathBuf;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Node, NodeKind};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

/// A fresh scratch folder for one test.
fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("vc-dcs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A plate program: one 2 × 2 grayscale image (`gray`, row by row) on a 20 × 10 pt page.
fn plate_eps(gray: [u8; 4]) -> String {
    let hex: String = gray.iter().map(|g| format!("{g:02x}")).collect();
    format!(
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 20 10\n%%EndComments\n20 10 scale\n2 2 8 [2 0 0 -2 0 2] <{hex}> image\nshowpage\n%%Trailer\n%%EOF\n"
    )
}

/// A single-file DCS 2.0 file of `plates` (name, plate program), with the header lines `extra`; its
/// composite is drawn through a clipping path, the left half of the page.
fn single_file(plates: &[(&str, String)], extra: &str) -> Vec<u8> {
    let header = |offsets: &[usize]| {
        let mut s = String::from("%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 20 10\n");
        s.push_str(extra);
        for ((name, eps), at) in plates.iter().zip(offsets) {
            s.push_str(&format!("%%PlateFile: ({name}) EPS #{at:010} {:010}\n", eps.len()));
        }
        s.push_str("%%EndComments\ngsave newpath 0 0 moveto 10 0 lineto 10 10 lineto 0 10 lineto closepath clip newpath\n");
        s.push_str("20 10 scale 2 2 8 [2 0 0 -2 0 2] <808080808080808080808080> false 3 colorimage grestore showpage\n%%EOF\n");
        s
    };
    let main = header(&vec![0; plates.len()]).len();
    let mut offsets = vec![];
    let mut at = main;
    for (_, eps) in plates {
        offsets.push(at);
        at += eps.len();
    }
    let mut out = header(&offsets).into_bytes();
    for (_, eps) in plates {
        out.extend(eps.as_bytes());
    }
    out
}

/// Cyan solid on the top row, PANTONE 185 C at 50% on the left column.
fn two_plates() -> Vec<u8> {
    single_file(
        &[("Cyan", plate_eps([0, 0, 255, 255])), ("PANTONE 185 C", plate_eps([127, 255, 127, 255]))],
        "%%DocumentCustomColors: (PANTONE 185 C)\n%%CMYKCustomColor: 0 .91 .76 0 (PANTONE 185 C)\n",
    )
}

fn write(dir: &std::path::Path, name: &str, bytes: &[u8]) -> String {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p.to_string_lossy().into_owned()
}

/// The image inside placed object `id` (itself, or the image of its clipping group).
fn image_of(s: &Session, id: &Value) -> Node {
    let n = s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().clone();
    let mut found = None;
    n.walk(&mut |c: &Node| {
        if found.is_none() && matches!(c.kind, NodeKind::Image(_)) {
            found = Some(c.clone());
        }
    });
    found.expect("an image")
}

fn error(s: &mut Session, cmd: &str, p: Value) -> String {
    s.execute(cmd, &p).unwrap_err().to_string()
}

#[test]
fn a_dcs_file_places_as_one_linked_image_of_its_plates() {
    let dir = temp_dir("place");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path, "at": [100, 100] })).unwrap();
    assert_eq!(r["linked"], true);
    assert_eq!(r["warnings"], json!([]));
    let id = &r["ids"][0];
    let placed = s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().clone();
    let NodeKind::Group { children, clip: true } = &placed.kind else { panic!("a clipping group: {:?}", placed.kind) };
    assert!(matches!(children[0].kind, NodeKind::Path { clipping: true, .. }), "the composite's clipping path");
    assert_eq!(children[0].geometric_bounds().map(|b| b.width().round()), Some(10.0));
    assert_eq!(r["width"].as_f64().map(f64::round), Some(10.0), "clipped to the left half of its 20 pt page");
    let image = image_of(&s, id);
    let NodeKind::Image(im) = &image.kind else { not_an_image() };
    assert!(im.link.as_ref().is_some_and(|l| l.path.ends_with("pic.eps")));
    let doc = &s.doc().unwrap().doc;
    let plates = doc.images[&im.key].separated().expect("its plates are kept");
    assert_eq!(plates.plates.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Cyan", "PANTONE 185 C"]);
    assert_eq!(plates.plates[1].data, [128, 0, 128, 0]);
    let sw = doc.swatch("PANTONE 185 C").expect("a spot swatch for the spot plate");
    assert!(sw.spot);
    assert_eq!(sw.paint, Paint::solid(Color::cmyk(0.0, 0.91, 0.76, 0.0)));
    assert!(!doc.swatches_iter().any(|w| w.spot && w.name == "Cyan"), "process plates add no spot swatch");
    let list = s.execute("links.list", &json!({})).unwrap();
    assert_eq!(list["links"][0]["format"], "DCS");
    assert_eq!(list["links"][0]["status"], "ok");
    let info = s.execute("file.place.info", &json!({ "path": path })).unwrap();
    assert_eq!(info["plates"], json!(["Cyan", "PANTONE 185 C"]));
    assert_eq!(info["colorMode"], "CMYK");
    assert_eq!(info["pixelWidth"], 2);
    // One undo step takes the swatch away too.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.swatch("PANTONE 185 C").is_none());
}

fn not_an_image() -> ! {
    panic!("an image")
}

#[test]
fn a_dcs_file_is_never_opened_or_embedded() {
    let dir = temp_dir("refused");
    let bytes = two_plates();
    let path = write(&dir, "pic.eps", &bytes);
    let mut s = session();
    let only = "DCS files can only be placed linked";
    assert!(error(&mut s, "file.place", json!({ "path": path, "link": false })).contains(only));
    let data = json!({ "name": "pic.eps", "dataBase64": vectorcraft_format::base64_encode(&bytes) });
    assert!(error(&mut s, "file.place", data).contains(only), "bytes alone can't be linked");
    assert!(error(&mut s, "document.open", json!({ "path": path })).contains(only));
    let r = s.execute("file.place", &json!({ "path": path })).unwrap();
    let image = image_of(&s, &r["ids"][0]);
    let e = error(&mut s, "links.embed", json!({ "ids": [image.id.0] }));
    assert!(e.contains(only), "{e}");
    let NodeKind::Image(im) = &image_of(&s, &r["ids"][0]).kind else { not_an_image() };
    assert!(im.link.is_some(), "still linked");
}

#[test]
fn a_swatch_of_the_same_name_is_kept_with_a_warning() {
    let dir = temp_dir("swatch");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "PANTONE 185 C", "color": {"c": 0.0, "m": 1.0, "y": 1.0, "k": 0.0}, "spot": true})).unwrap();
    let r = s.execute("file.place", &json!({ "path": path })).unwrap();
    assert_eq!(r["warnings"], json!(["the document's swatch `PANTONE 185 C` is kept: it is a different color"]));
    let sw = s.doc().unwrap().doc.swatch("PANTONE 185 C").unwrap().clone();
    assert_eq!(sw.paint.color(), Some(Color::cmyk(0.0, 1.0, 1.0, 0.0)));
}

#[test]
fn plate_files_of_their_own_are_read_from_the_dcs_files_folder() {
    let dir = temp_dir("files");
    write(&dir, "pic.C", plate_eps([0, 255, 255, 255]).as_bytes());
    write(&dir, "pic.K", plate_eps([255, 255, 255, 0]).as_bytes());
    let main = "%!PS-Adobe-2.0 EPSF-1.2\n%%BoundingBox: 0 0 20 10\n%%CyanPlate: OldDisk:Scans:pic.C\n%%BlackPlate: pic.K\n%%EndComments\nshowpage\n";
    let path = write(&dir, "pic.eps", main.as_bytes());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path })).unwrap();
    let image = image_of(&s, &r["ids"][0]);
    assert!(matches!(image.kind, NodeKind::Image(_)), "no clipping path without a composite");
    let NodeKind::Image(im) = &image.kind else { not_an_image() };
    let plates = s.doc().unwrap().doc.images[&im.key].separated().unwrap();
    assert_eq!(plates.plates[0].data, [255, 0, 0, 0], "the plate file found by its name in the folder");
    assert_eq!(plates.plates[1].data, [0, 0, 0, 255]);
    std::fs::remove_file(dir.join("pic.K")).unwrap();
    let e = error(&mut s, "file.place", json!({ "path": path }));
    assert!(e.contains("the Black plate can't be read") && e.contains("`pic.K` is not in the DCS file's folder"), "{e}");
}

#[test]
fn update_link_reads_the_plates_again() {
    let dir = temp_dir("update");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path })).unwrap();
    let id = image_of(&s, &r["ids"][0]).id;
    let changed = single_file(&[("Cyan", plate_eps([0, 0, 0, 0])), ("Varnish", plate_eps([255; 4]))], "%%CMYKCustomColor: 0 0 .2 0 (Varnish)\n");
    std::fs::write(&path, &changed).unwrap();
    let u = s.execute("links.update", &json!({ "ids": [id.0] })).unwrap();
    assert_eq!(u["updated"], json!([id.0]));
    let NodeKind::Image(im) = &s.doc().unwrap().doc.node(id).unwrap().kind else { not_an_image() };
    let plates = s.doc().unwrap().doc.images[&im.key].separated().unwrap();
    assert_eq!(plates.plates.len(), 2);
    assert_eq!(plates.plates[0].data, [255; 4]);
    let sw = s.doc().unwrap().doc.swatch("Varnish").expect("a spot swatch for the plate the file gained");
    assert!(sw.spot);
    assert_eq!(sw.paint, Paint::solid(Color::cmyk(0.0, 0.0, 0.2, 0.0)));
}

#[test]
fn update_link_takes_the_files_new_clipping_path() {
    let dir = temp_dir("update-clip");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path, "at": [100, 100] })).unwrap();
    let group = NodeId(r["ids"][0].as_u64().unwrap());
    let id = image_of(&s, &r["ids"][0]).id;
    let clip_width = |s: &Session| {
        let NodeKind::Group { children, .. } = &s.doc().unwrap().doc.node(group).unwrap().kind else { panic!("a clipping group") };
        children[0].geometric_bounds().map(|b| b.width())
    };
    let before = clip_width(&s).unwrap();
    // The same file, its composite now drawn through the left quarter of the page (same length,
    // so the plates stay where the header says).
    let text = String::from_utf8(two_plates()).unwrap().replacen("10 0 lineto 10 10 lineto", " 5 0 lineto  5 10 lineto", 1);
    std::fs::write(&path, text.as_bytes()).unwrap();
    let u = s.execute("links.update", &json!({ "ids": [id.0] })).unwrap();
    assert_eq!(u["updated"], json!([id.0]));
    let after = clip_width(&s).unwrap();
    assert!((after - before / 2.0).abs() < 1e-6, "the clipping path follows the file: {before} → {after}");
}

#[test]
fn update_link_clips_an_image_whose_file_gained_a_clipping_path() {
    let dir = temp_dir("update-gained-clip");
    // The composite drawn without its clipping path (same length).
    let unclipped = String::from_utf8(two_plates()).unwrap().replacen("closepath clip newpath", "closepath newpath     ", 1);
    let path = write(&dir, "pic.eps", unclipped.as_bytes());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path })).unwrap();
    let id = NodeId(r["ids"][0].as_u64().unwrap());
    assert!(matches!(s.doc().unwrap().doc.node(id).unwrap().kind, NodeKind::Image(_)), "no clipping path, no group");
    std::fs::write(&path, two_plates()).unwrap();
    s.execute("links.update", &json!({ "ids": [id.0] })).unwrap();
    let doc = &s.doc().unwrap().doc;
    let group = doc.node(doc.parent_of(id).expect("a parent")).unwrap();
    let NodeKind::Group { children, clip: true } = &group.kind else { panic!("a clipping group: {:?}", group.kind) };
    assert_eq!(children[0].geometric_bounds().map(|b| b.width().round()), Some(10.0), "the left half of the page");
}

#[test]
fn a_changed_plate_file_marks_the_link_modified() {
    let dir = temp_dir("plate-changed");
    write(&dir, "pic.C", plate_eps([0, 255, 255, 255]).as_bytes());
    let main = "%!PS-Adobe-2.0 EPSF-1.2\n%%BoundingBox: 0 0 20 10\n%%CyanPlate: pic.C\n%%EndComments\nshowpage\n";
    let path = write(&dir, "pic.eps", main.as_bytes());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path })).unwrap();
    let id = image_of(&s, &r["ids"][0]).id;
    assert_eq!(s.execute("links.list", &json!({})).unwrap()["links"][0]["status"], "ok");
    // Only the plate file changes (a size of its own, so the change shows whatever the clock).
    write(&dir, "pic.C", format!("{}% more\n", plate_eps([255, 255, 255, 0])).as_bytes());
    assert_eq!(s.execute("links.list", &json!({})).unwrap()["links"][0]["status"], "modified");
    let u = s.execute("links.update", &json!({})).unwrap();
    assert_eq!(u["updated"], json!([id.0]));
    let NodeKind::Image(im) = &s.doc().unwrap().doc.node(id).unwrap().kind else { not_an_image() };
    assert_eq!(s.doc().unwrap().doc.images[&im.key].separated().unwrap().plates[0].data, [0, 0, 0, 255]);
    assert_eq!(s.execute("links.list", &json!({})).unwrap()["links"][0]["status"], "ok");
    std::fs::remove_file(dir.join("pic.C")).unwrap();
    assert_eq!(s.execute("links.list", &json!({})).unwrap()["links"][0]["status"], "modified", "a plate file that is gone");
}

#[test]
fn a_saved_document_reads_the_plates_again_and_prints_the_spot_ink() {
    let dir = temp_dir("saved");
    let path = write(&dir, "pic.eps", &two_plates());
    let saved = dir.join("doc.vectorcraft").to_string_lossy().into_owned();
    let mut s = session();
    s.execute("file.place", &json!({ "path": path })).unwrap();
    let unused = s.execute("swatch.unused", &json!({})).unwrap();
    assert!(!unused["names"].as_array().unwrap().iter().any(|n| n == "PANTONE 185 C"), "the plate's swatch is in use: {unused}");
    s.execute("document.save", &json!({ "path": saved })).unwrap();
    let mut s = Session::new();
    s.execute("document.open", &json!({ "path": saved })).unwrap();
    let list = s.execute("links.list", &json!({})).unwrap();
    assert_eq!(list["links"][0]["format"], "DCS");
    assert_eq!(list["links"][0]["status"], "ok");
    let id = NodeId(list["links"][0]["id"].as_u64().unwrap());
    let NodeKind::Image(im) = &s.doc().unwrap().doc.node(id).unwrap().kind else { not_an_image() };
    let plates = s.doc().unwrap().doc.images[&im.key].separated().expect("the plates, read from the file again");
    assert_eq!(plates.plates[1].data, [128, 0, 128, 0]);
    let v = s.execute("print.preview", &json!({"settings": {"output": {"mode": "separations"}}})).unwrap();
    let inks: Vec<&str> = v["sheets"].as_array().unwrap().iter().filter_map(|p| p["ink"].as_str()).collect();
    assert_eq!(inks, ["Cyan", "Magenta", "Yellow", "Black", "PANTONE 185 C"]);
}

#[test]
fn dcs_transparency_marks_transparent_and_overlapped_dcs_images() {
    let dir = temp_dir("transparency");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path, "at": [100, 100] })).unwrap();
    let image = image_of(&s, &r["ids"][0]).id;
    let marked = |s: &mut Session| s.execute("links.list", &json!({ "dcsTransparency": true })).unwrap()["links"][0]["dcsTransparency"] == true;
    assert!(!marked(&mut s), "nothing transparent");
    assert!(s.execute("links.list", &json!({})).unwrap()["links"][0].get("dcsTransparency").is_none());
    // A half-transparent square away from it, then over it.
    let far = s.execute("shape.rectangle", &json!({"x": 300, "y": 200, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("transparency.set", &json!({"ids": [far], "opacity": 50})).unwrap();
    assert!(!marked(&mut s), "a transparent object elsewhere");
    s.execute("shape.rectangle", &json!({"x": 90, "y": 90, "width": 20, "height": 20})).unwrap();
    let over = s.execute("shape.rectangle", &json!({"x": 92, "y": 92, "width": 4, "height": 4})).unwrap()["id"].as_u64().unwrap();
    assert!(!marked(&mut s), "opaque objects over it");
    s.execute("transparency.set", &json!({"ids": [over], "blend": "multiply"})).unwrap();
    assert!(marked(&mut s), "a blend mode over it");
    s.execute("transparency.set", &json!({"ids": [over], "blend": "normal"})).unwrap();
    s.execute("transparency.set", &json!({"ids": [image.0], "opacity": 80})).unwrap();
    assert!(marked(&mut s), "the image itself");
    assert!(error(&mut s, "links.list", json!({ "dcsTransparency": "yes" })).contains("dcsTransparency"), "a flag that isn't true or false");
}
