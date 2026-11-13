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
fn the_link_watch_reads_the_link_again_when_only_a_plate_file_changes() {
    let dir = temp_dir("plate-watch");
    write(&dir, "pic.C", plate_eps([0, 255, 255, 255]).as_bytes());
    let main = "%!PS-Adobe-2.0 EPSF-1.2\n%%BoundingBox: 0 0 20 10\n%%CyanPlate: pic.C\n%%EndComments\nshowpage\n";
    let path = write(&dir, "pic.eps", main.as_bytes());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path })).unwrap();
    let id = image_of(&s, &r["ids"][0]).id;
    let files: Vec<String> = crate::link_watch::linked_files(&s.doc().unwrap().doc).into_iter().map(|(p, _)| p).collect();
    assert!(files.iter().any(|p| p.ends_with("pic.C")), "the plate file is watched too: {files:?}");
    // The first look only remembers the files.
    assert_eq!(s.execute("links.updateChanged", &json!({})).unwrap()["updated"], json!([]));
    write(&dir, "pic.C", format!("{}% more\n", plate_eps([255, 255, 255, 0])).as_bytes());
    assert_eq!(s.execute("links.updateChanged", &json!({})).unwrap()["updated"], json!([id.0]));
    let NodeKind::Image(im) = &s.doc().unwrap().doc.node(id).unwrap().kind else { not_an_image() };
    assert_eq!(s.doc().unwrap().doc.images[&im.key].separated().unwrap().plates[0].data, [0, 0, 0, 255]);
}

/// The width of the clipping path of the clip group holding the image in the art of symbol
/// `name`; `None` when no clip group holds it.
fn symbol_clip_width(s: &Session, name: &str) -> Option<f64> {
    fn find(n: &Node) -> Option<Option<f64>> {
        if let NodeKind::Group { children, clip: true } = &n.kind
            && let [c, im] = &children[..]
            && matches!(im.kind, NodeKind::Image(_))
        {
            return Some(c.geometric_bounds().map(|b| b.width()));
        }
        if matches!(n.kind, NodeKind::Image(_)) {
            return Some(None);
        }
        n.children().and_then(|ch| ch.iter().find_map(|c| find(c)))
    }
    let doc = &s.doc().unwrap().doc;
    find(&doc.symbols.iter().find(|w| w.name == name).expect("the symbol").art).expect("an image in the symbol")
}

#[test]
fn update_link_takes_the_files_clipping_path_into_symbols_too() {
    let dir = temp_dir("update-symbol-clip");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path, "at": [100, 100] })).unwrap();
    let id = image_of(&s, &r["ids"][0]).id;
    let other = s.execute("file.place", &json!({ "path": path, "at": [200, 100] })).unwrap();
    s.execute("symbol.new", &json!({ "name": "Pic", "ids": [other["ids"][0]] })).unwrap();
    let before = symbol_clip_width(&s, "Pic").expect("the file's clipping path");
    // The composite now drawn through the left quarter of the page (same length).
    let text = String::from_utf8(two_plates()).unwrap().replacen("10 0 lineto 10 10 lineto", " 5 0 lineto  5 10 lineto", 1);
    std::fs::write(&path, text.as_bytes()).unwrap();
    s.execute("links.update", &json!({ "ids": [id.0] })).unwrap();
    let after = symbol_clip_width(&s, "Pic").expect("still clipped");
    assert!((after - before / 2.0).abs() < 1e-6, "the symbol's clipping path follows the file: {before} → {after}");
    // The file without a clipping path (same length): the symbol's image is no longer clipped.
    let unclipped = text.replacen("closepath clip newpath", "closepath newpath     ", 1);
    std::fs::write(&path, unclipped.as_bytes()).unwrap();
    s.execute("links.update", &json!({ "ids": [id.0] })).unwrap();
    assert_eq!(symbol_clip_width(&s, "Pic"), None);
}

#[test]
fn a_file_that_lost_its_clipping_path_unclips_the_image() {
    let dir = temp_dir("update-lost-clip");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path })).unwrap();
    let id = image_of(&s, &r["ids"][0]).id;
    let unclipped = String::from_utf8(two_plates()).unwrap().replacen("closepath clip newpath", "closepath newpath     ", 1);
    std::fs::write(&path, unclipped.as_bytes()).unwrap();
    s.execute("links.update", &json!({ "ids": [id.0] })).unwrap();
    let doc = &s.doc().unwrap().doc;
    let placed = NodeId(r["ids"][0].as_u64().unwrap());
    assert_eq!(doc.node(placed).and_then(plain_group_of_image), Some((false, true)), "the placed group stays, without the clip");
    let parent = doc.parent_of(id).expect("a parent");
    assert!(!matches!(doc.node(parent).unwrap().kind, NodeKind::Group { clip: true, .. }), "the image is not clipped");
    // Relinked to a plain image: no clip group either.
    let png = dir.join("plain.png");
    image::RgbImage::from_pixel(2, 2, image::Rgb([9, 9, 9])).save(&png).unwrap();
    std::fs::write(&path, two_plates()).unwrap();
    s.execute("links.update", &json!({ "ids": [id.0] })).unwrap();
    assert!(
        s.doc().unwrap().doc.parent_of(id).is_some_and(|p| matches!(s.doc().unwrap().doc.node(p).unwrap().kind, NodeKind::Group { clip: true, .. }))
    );
    s.execute("links.relink", &json!({ "ids": [id.0], "path": png.to_string_lossy() })).unwrap();
    let doc = &s.doc().unwrap().doc;
    assert!(
        !matches!(doc.node(doc.parent_of(id).unwrap()).unwrap().kind, NodeKind::Group { clip: true, .. }),
        "a plain image isn't clipped by the DCS file's path"
    );
}

/// The kind of group `g` is, as (clip, the kinds of its children are just one image).
fn plain_group_of_image(g: &Node) -> Option<(bool, bool)> {
    match &g.kind {
        NodeKind::Group { children, clip } => Some((*clip, matches!(&children[..], [c] if matches!(c.kind, NodeKind::Image(_))))),
        _ => None,
    }
}

#[test]
fn a_hidden_or_transparent_clip_group_loses_the_files_old_clipping_path() {
    let dir = temp_dir("lost-clip-kept-group");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    let place = |s: &mut Session, x: f64| s.execute("file.place", &json!({ "path": path, "at": [x, 100] })).unwrap();
    let (faded, hidden, in_symbol) = (place(&mut s, 100.0), place(&mut s, 200.0), place(&mut s, 300.0));
    let (faded, hidden) = (NodeId(faded["ids"][0].as_u64().unwrap()), NodeId(hidden["ids"][0].as_u64().unwrap()));
    let image = image_of(&s, &json!(faded.0)).id;
    s.execute("transparency.set", &json!({"ids": [faded.0, in_symbol["ids"][0]], "opacity": 50})).unwrap();
    s.edit("hide", |d, _| {
        if let Some(n) = d.node_mut(hidden) {
            n.visible = false;
        }
        Ok(())
    })
    .unwrap();
    s.execute("symbol.new", &json!({ "name": "Pic", "ids": [in_symbol["ids"][0]] })).unwrap();
    let unclipped = String::from_utf8(two_plates()).unwrap().replacen("closepath clip newpath", "closepath newpath     ", 1);
    std::fs::write(&path, unclipped.as_bytes()).unwrap();
    let hidden_image = image_of(&s, &json!(hidden.0)).id;
    s.execute("links.update", &json!({ "ids": [image.0, hidden_image.0] })).unwrap();
    let doc = &s.doc().unwrap().doc;
    for (g, why) in [(faded, "the transparent group"), (hidden, "the hidden group")] {
        let n = doc.node(g).unwrap_or_else(|| panic!("{why} stays"));
        assert_eq!(plain_group_of_image(n), Some((false, true)), "{why} holds the image alone, unclipped");
    }
    assert!((doc.node(faded).unwrap().opacity - 0.5).abs() < 1e-6, "its transparency kept");
    assert!(!doc.node(hidden).unwrap().visible, "still hidden");
    // In a symbol's art too.
    let art = &doc.symbols.iter().find(|w| w.name == "Pic").expect("the symbol").art;
    let mut found = None;
    art.walk(&mut |n: &Node| {
        if found.is_none() && n.children().is_some_and(|ch| ch.iter().any(|c| matches!(c.kind, NodeKind::Image(_)))) {
            found = plain_group_of_image(n);
        }
    });
    assert_eq!(found, Some((false, true)), "the symbol's transparent group holds the image alone, unclipped");
}

#[test]
fn a_clipping_mask_of_ones_own_stays_when_the_link_is_updated() {
    let dir = temp_dir("own-mask");
    let unclipped = String::from_utf8(two_plates()).unwrap().replacen("closepath clip newpath", "closepath newpath     ", 1);
    let path = write(&dir, "pic.eps", unclipped.as_bytes());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path, "at": [100, 100] })).unwrap();
    let id = NodeId(r["ids"][0].as_u64().unwrap());
    let rect = s.execute("shape.rectangle", &json!({"x": 95, "y": 95, "width": 4, "height": 4})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({ "ids": [id.0, rect] })).unwrap();
    let g = NodeId(s.execute("object.clippingMask.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("links.update", &json!({ "ids": [id.0] })).unwrap();
    assert_eq!(s.doc().unwrap().doc.parent_of(id), Some(g), "the clipping mask made in the document stays");
    let mask_width = |s: &Session| {
        let NodeKind::Group { children, .. } = &s.doc().unwrap().doc.node(g).unwrap().kind else { panic!("the mask's group") };
        children[0].geometric_bounds().map(|b| b.width().round())
    };
    assert_eq!(mask_width(&s), Some(4.0));
    // The file gains a clipping path: its clip group goes inside the mask, which keeps its path.
    std::fs::write(&path, two_plates()).unwrap();
    s.execute("links.update", &json!({ "ids": [id.0] })).unwrap();
    let doc = &s.doc().unwrap().doc;
    let inner = doc.parent_of(id).expect("a parent");
    assert_ne!(inner, g);
    assert!(matches!(doc.node(inner).unwrap().kind, NodeKind::Group { clip: true, .. }), "the file's clip group");
    assert_eq!(doc.parent_of(inner), Some(g));
    assert_eq!(mask_width(&s), Some(4.0), "the mask's path is kept");
}

#[test]
fn a_dcs_file_takes_the_place_parameters_an_eps_file_takes() {
    let dir = temp_dir("params");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    assert!(error(&mut s, "file.place", json!({ "path": path, "page": 0 })).contains("page"));
    assert!(error(&mut s, "file.place", json!({ "path": path, "crop": "trim" })).contains("crop"));
}

#[test]
fn a_clipped_placement_answers_its_image_for_the_links_commands() {
    let dir = temp_dir("image-id");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path })).unwrap();
    assert_eq!(r["image"], json!(image_of(&s, &r["ids"][0]).id.0));
    let info = s.execute("links.info", &json!({ "id": r["image"] })).unwrap();
    assert_eq!(info["format"], "DCS");
    let unclipped = String::from_utf8(two_plates()).unwrap().replacen("closepath clip newpath", "closepath newpath     ", 1);
    let plain = write(&dir, "plain.eps", unclipped.as_bytes());
    assert!(s.execute("file.place", &json!({ "path": plain })).unwrap().get("image").is_none(), "an image placed alone is its own id");
}

#[test]
fn update_and_relink_warn_of_a_swatch_kept_for_a_new_spot_plate() {
    let dir = temp_dir("update-swatch");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path })).unwrap();
    let id = image_of(&s, &r["ids"][0]).id;
    s.execute("swatch.new", &json!({"name": "Varnish", "color": {"c": 1.0, "m": 0.0, "y": 0.0, "k": 0.0}})).unwrap();
    let changed = single_file(&[("Cyan", plate_eps([0, 0, 0, 0])), ("Varnish", plate_eps([255; 4]))], "%%CMYKCustomColor: 0 0 .2 0 (Varnish)\n");
    std::fs::write(&path, &changed).unwrap();
    let u = s.execute("links.update", &json!({ "ids": [id.0] })).unwrap();
    let kept = "the document's swatch `Varnish` is kept: it is a process color, so the plate prints as process inks";
    assert_eq!(u["warnings"], json!([kept]));
    let other = write(&dir, "other.eps", &changed);
    let r = s.execute("links.relink", &json!({ "ids": [id.0], "path": other })).unwrap();
    assert_eq!(r["warnings"], json!([kept]));
    assert!(s.execute("links.update", &json!({ "ids": [id.0] })).unwrap().get("warnings").is_some(), "every read warns");
}

#[test]
fn a_deleted_dcs_image_leaves_its_spot_swatch_unused() {
    let dir = temp_dir("unused");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path })).unwrap();
    let unused = |s: &mut Session| s.execute("swatch.unused", &json!({})).unwrap()["names"].as_array().unwrap().iter().any(|n| n == "PANTONE 185 C");
    assert!(!unused(&mut s), "in use while the image shows it");
    s.execute("select.set", &json!({ "ids": [r["ids"][0]] })).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(unused(&mut s), "unused once the image is deleted");
}

#[test]
fn a_process_swatch_giving_a_dcs_plate_its_color_is_used() {
    let dir = temp_dir("unused-process");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "PANTONE 185 C", "color": {"c": 1.0, "m": 0.0, "y": 0.0, "k": 0.0}})).unwrap();
    let unused = |s: &mut Session| s.execute("swatch.unused", &json!({})).unwrap()["names"].as_array().unwrap().iter().any(|n| n == "PANTONE 185 C");
    assert!(unused(&mut s), "nothing uses it yet");
    s.execute("file.place", &json!({ "path": path })).unwrap();
    assert!(!unused(&mut s), "the plate of its name prints as its process inks");
}

#[test]
fn an_opened_clipping_path_is_not_the_files_and_stays() {
    let dir = temp_dir("opened-clip");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path })).unwrap();
    let group = NodeId(r["ids"][0].as_u64().unwrap());
    let id = image_of(&s, &r["ids"][0]).id;
    // The clipping path opened, its anchors where they were.
    s.edit("open", |d, _| {
        let clip = d.node(group).and_then(|g| g.children()).map(|ch| ch[0].id).unwrap();
        if let Some(Node { kind: NodeKind::Path { path, .. }, .. }) = d.node_mut(clip) {
            path.subpaths[0].closed = false;
        }
        Ok(())
    })
    .unwrap();
    let unclipped = String::from_utf8(two_plates()).unwrap().replacen("closepath clip newpath", "closepath newpath     ", 1);
    std::fs::write(&path, unclipped.as_bytes()).unwrap();
    s.execute("links.update", &json!({ "ids": [id.0] })).unwrap();
    assert_eq!(s.doc().unwrap().doc.parent_of(id), Some(group), "a clipping path edited in the document stays");
}

#[test]
fn a_clipping_path_given_another_fill_rule_is_not_the_files_and_stays() {
    let dir = temp_dir("rule-clip");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path })).unwrap();
    let group = NodeId(r["ids"][0].as_u64().unwrap());
    let id = image_of(&s, &r["ids"][0]).id;
    // The clipping path's fill rule changed, its anchors where they were.
    s.edit("rule", |d, _| {
        let clip = d.node(group).and_then(|g| g.children()).map(|ch| ch[0].id).unwrap();
        if let Some(Node { kind: NodeKind::Path { rule, .. }, .. }) = d.node_mut(clip) {
            *rule = vectorcraft_geom::FillRule::EvenOdd;
        }
        Ok(())
    })
    .unwrap();
    let unclipped = String::from_utf8(two_plates()).unwrap().replacen("closepath clip newpath", "closepath newpath     ", 1);
    std::fs::write(&path, unclipped.as_bytes()).unwrap();
    s.execute("links.update", &json!({ "ids": [id.0] })).unwrap();
    assert_eq!(s.doc().unwrap().doc.parent_of(id), Some(group), "a clipping path whose fill rule was changed stays");
}

#[test]
fn an_even_odd_clipping_path_is_placed_and_matched_under_the_nonzero_rule() {
    let dir = temp_dir("eo-clip");
    // The same length, so the plates' offsets hold.
    let eo = String::from_utf8(two_plates()).unwrap().replacen("gsave newpath 0 0 moveto", "gsave 0 0 moveto      ", 1);
    let eo = eo.replacen("closepath clip newpath", "closepath eoclip newpath", 1);
    assert_eq!(eo.len(), two_plates().len());
    let path = write(&dir, "pic.eps", eo.as_bytes());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path })).unwrap();
    let group = NodeId(r["ids"][0].as_u64().unwrap());
    let id = image_of(&s, &r["ids"][0]).id;
    let doc = &s.doc().unwrap().doc;
    let clip = &doc.node(group).and_then(|g| g.children()).expect("the file's clip group")[0];
    assert!(matches!(clip.kind, NodeKind::Path { rule: vectorcraft_geom::FillRule::NonZero, clipping: true, .. }), "{:?}", clip.kind);
    // The file without its clipping path: the group's clipping path is the file's, and goes.
    let unclipped = String::from_utf8(two_plates()).unwrap().replacen("closepath clip newpath", "closepath newpath     ", 1);
    std::fs::write(&path, unclipped.as_bytes()).unwrap();
    s.execute("links.update", &json!({ "ids": [id.0] })).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(group).and_then(plain_group_of_image), Some((false, true)), "the file's clipping path is gone");
}

#[test]
fn files_updated_together_warn_when_they_add_one_ink_in_two_colors() {
    let dir = temp_dir("update-two-inks");
    let (a, b) = (write(&dir, "a.eps", &two_plates()), write(&dir, "b.eps", &two_plates()));
    let mut s = session();
    s.execute("file.place", &json!({ "path": a })).unwrap();
    s.execute("file.place", &json!({ "path": b })).unwrap();
    let varnish =
        |c: &str| single_file(&[("Cyan", plate_eps([0; 4])), ("Varnish", plate_eps([255; 4]))], &format!("%%CMYKCustomColor: {c} (Varnish)\n"));
    std::fs::write(&a, varnish("0 0 .2 0")).unwrap();
    std::fs::write(&b, varnish("1 0 0 0")).unwrap();
    let u = s.execute("links.update", &json!({})).unwrap();
    assert_eq!(u["updated"].as_array().map(Vec::len), Some(2), "{u}");
    assert_eq!(u["warnings"], json!(["the document's swatch `Varnish` is kept: it is a different color"]));
}

#[test]
fn relink_with_copy_refuses_a_dcs_file_with_plate_files() {
    let dir = temp_dir("copy");
    write(&dir, "pic.C", plate_eps([0, 255, 255, 255]).as_bytes());
    let main = "%!PS-Adobe-2.0 EPSF-1.2\n%%BoundingBox: 0 0 20 10\n%%CyanPlate: pic.C\n%%EndComments\nshowpage\n";
    let path = write(&dir, "pic.eps", main.as_bytes());
    let docs = dir.join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    let mut s = session();
    s.execute("document.save", &json!({ "path": docs.join("doc.vectorcraft").to_string_lossy() })).unwrap();
    let png = dir.join("plain.png");
    image::RgbImage::from_pixel(2, 2, image::Rgb([9, 9, 9])).save(&png).unwrap();
    let r = s.execute("file.place", &json!({ "path": png.to_string_lossy() })).unwrap();
    let id = r["ids"][0].clone();
    let e = error(&mut s, "links.relink", json!({ "ids": [id], "path": path, "copy": true }));
    assert!(e.contains("plate files"), "{e}");
    assert!(!docs.join("pic.eps").exists(), "nothing is copied");
    // A single-file DCS file has its plates inside it: it is copied.
    let single = write(&dir, "single.eps", &two_plates());
    s.execute("links.relink", &json!({ "ids": [id], "path": single, "copy": true })).unwrap();
    assert!(docs.join("single.eps").exists());
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

#[test]
fn dcs_transparency_looks_only_at_the_part_of_an_image_its_clipping_path_shows() {
    let dir = temp_dir("transparency-clipped");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    let r = s.execute("file.place", &json!({ "path": path, "at": [100, 100] })).unwrap();
    let group = NodeId(r["ids"][0].as_u64().unwrap());
    let image = image_of(&s, &r["ids"][0]).id;
    let (clip, all) = {
        let doc = &s.doc().unwrap().doc;
        let NodeKind::Group { children, clip: true } = &doc.node(group).unwrap().kind else { panic!("a clipping group") };
        (children[0].geometric_bounds().unwrap(), doc.node(image).unwrap().geometric_bounds().unwrap())
    };
    assert!(all.x1 - clip.x1 > 2.0, "the file's clipping path shows the left half: {clip:?} of {all:?}");
    // A half-transparent square over the part of the image the clipping path hides.
    let x = (clip.x1 + all.x1) / 2.0 - 1.0;
    let over = s.execute("shape.rectangle", &json!({"x": x, "y": all.y0, "width": 2, "height": 2})).unwrap()["id"].as_u64().unwrap();
    s.execute("transparency.set", &json!({"ids": [over], "opacity": 50})).unwrap();
    let marked = |s: &mut Session| s.execute("links.list", &json!({ "dcsTransparency": true })).unwrap()["links"][0]["dcsTransparency"] == true;
    assert!(!marked(&mut s), "the square is over the hidden part only");
    s.execute("transparency.set", &json!({"ids": [over], "opacity": 100})).unwrap();
    let shown = s.execute("shape.rectangle", &json!({"x": clip.x0, "y": all.y0, "width": 2, "height": 2})).unwrap()["id"].as_u64().unwrap();
    s.execute("transparency.set", &json!({"ids": [shown], "opacity": 50})).unwrap();
    assert!(marked(&mut s), "a square over the part that shows");
}

#[test]
fn dcs_transparency_counts_a_transparent_dcs_image_over_another() {
    let dir = temp_dir("transparency-two");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    let a = s.execute("file.place", &json!({ "path": path, "at": [100, 100] })).unwrap();
    let b = s.execute("file.place", &json!({ "path": path, "at": [103, 100] })).unwrap();
    let (a, b) = (image_of(&s, &a["ids"][0]).id, image_of(&s, &b["ids"][0]).id);
    s.execute("transparency.set", &json!({"ids": [a.0], "opacity": 80})).unwrap();
    let list = s.execute("links.list", &json!({ "dcsTransparency": true })).unwrap();
    let marked = |id: NodeId| list["links"].as_array().unwrap().iter().any(|r| r["id"] == id.0 && r["dcsTransparency"] == true);
    assert!(marked(a), "the transparent image");
    assert!(marked(b), "the opaque image it overlaps");
}

#[test]
fn dcs_transparency_counts_characters_with_a_transparent_gradient() {
    use vectorcraft_color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let dir = temp_dir("transparency-text");
    let path = write(&dir, "pic.eps", &two_plates());
    let mut s = session();
    s.execute("file.place", &json!({ "path": path, "at": [100, 100] })).unwrap();
    let t = NodeId(s.execute("text.create", &json!({"x": 96, "y": 104, "text": "Hi", "size": 12})).unwrap()["id"].as_u64().unwrap());
    let marked = |s: &mut Session| s.execute("links.list", &json!({ "dcsTransparency": true })).unwrap()["links"][0]["dcsTransparency"] == true;
    assert!(!marked(&mut s), "opaque characters over it");
    let mut stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    stops[1].opacity = 0.5;
    let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    s.edit("t", |d, _| {
        if let Some(Node { kind: NodeKind::Text(text), .. }) = d.node_mut(t) {
            text.runs[0].style.fill = gradient.clone();
        }
        Ok(())
    })
    .unwrap();
    assert!(marked(&mut s), "characters filled with a gradient that has a transparent stop");
    assert!(s.doc().unwrap().doc.node(t).unwrap().shows_transparency());
}
