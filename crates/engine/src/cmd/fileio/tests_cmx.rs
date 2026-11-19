//! `.cmx` files: Open and Place, by extension and by content, mapped to artboards, layers, groups,
//! paths, solid paint and strokes, with their warnings and refusals.

use super::*;
use serde_json::json;
use vectorcraft_cdr::synth_cmx::*;
use vectorcraft_doc::{Node, NodeKind};

fn paths(nodes: &[std::sync::Arc<Node>], out: &mut Vec<Node>) {
    for n in nodes {
        match &n.kind {
            NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => paths(children, out),
            _ => out.push((**n).clone()),
        }
    }
}

fn warnings(r: &serde_json::Value) -> Vec<String> {
    r["warnings"].as_array().map(|w| w.iter().filter_map(|w| w.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

#[test]
fn a_cmx_file_opens_as_editable_art() {
    let mut s = Session::new();
    let r = open_bytes(&mut s, "drawing.cmx", &sample(), None).unwrap();
    assert_eq!(r["format"], "cmx");
    assert!(warnings(&r).is_empty(), "{r}");
    let doc = &s.doc().unwrap().doc;
    // One 210 x 297 mm page.
    assert_eq!(doc.artboards.len(), 1);
    let ab = doc.artboards[0].rect;
    assert_eq!((ab.width().round(), ab.height().round()), (595.0, 842.0));
    // Bottom to top: the layer drawn first is at the bottom.
    let names: Vec<String> = doc.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(names, ["Art", "Top"]);
    let art = doc.layers[0].children().unwrap();
    assert_eq!(art.len(), 2);
    assert!(matches!(art[1].kind, NodeKind::Group { .. }), "the group drawn after the red square is above it");
    let mut all = Vec::new();
    paths(&doc.layers, &mut all);
    assert_eq!(all.len(), 4);
    // The red square: 100 mm on a side, centered on the page, with a fill and a stroke.
    let red = &all[0];
    let b = doc.bounds_of(&[red.id], false).unwrap();
    let mm = 72.0 / 25.4;
    assert!((b.width() - 100.0 * mm).abs() < 0.01 && (b.center().x - ab.center().x).abs() < 0.01, "{b:?}");
    assert_eq!(red.appearance.items.len(), 2);
    // The square whose line type draws no outline has a fill only.
    assert_eq!(all[1].appearance.items.len(), 1);
}

#[test]
fn detection_by_content_and_place() {
    let bytes = sample();
    let mut s = Session::new();
    // A renamed file is found by its content, and a .cmx extension alone reads it as one.
    assert_eq!(open_bytes(&mut s, "renamed.bin", &bytes, None).unwrap()["format"], "cmx");
    assert_eq!(detect("drawing.png", &bytes).map(|f| f.id), Some("cmx"));
    assert_eq!(detect("drawing.cmx", b"").map(|f| f.id), Some("cmx"));
    s.execute("file.new", &json!({"width":100,"height":100})).unwrap();
    s.execute("file.place", &json!({"name":"art.cmx", "dataBase64":vectorcraft_format::base64_encode(&bytes)})).unwrap();
    assert!(s.doc().unwrap().doc.layers.iter().any(|l| !l.children().unwrap().is_empty()));
    let f = format("cmx").unwrap();
    assert!(f.read && !f.write);
    assert!(OPEN_EXTS.contains(&"cmx") && PLACE_EXTS.contains(&"cmx") && UNASSOCIATED_EXTS.contains(&"cmx"));
    assert!(!SAVE_FORMATS.contains(&"cmx"));
}

#[test]
fn what_is_left_out_is_named_in_the_warnings() {
    let mut children = vec![cont(b'4', 1e-7)];
    children.push(page(&[
        begin_page(None),
        begin_layer("L", None),
        instr(66, &fields(&[field(1, &[0; 8])])),
        square(&attrs_fill_type(2), 0, 0, 1000),
        square(&attrs(Some(1), Some(1), true), 0, 0, 1000),
    ]));
    children.extend(tables().into_iter().filter(|t| !t.starts_with(b"rota")));
    children.push(arrowheads(&[(1, 0)]));
    let mut s = Session::new();
    let w = warnings(&open_bytes(&mut s, "drawing.cmx", &riff(&children), None).unwrap());
    assert_eq!(
        w,
        [
            "CMX text, images, rectangles, ellipses, clipping paths and other objects VectorCraft doesn't read yet were left out",
            "CMX fills other than uniform colors were left out",
            "CMX outline arrowheads were left out",
            "CMX lenses were not applied",
        ]
    );
}

#[test]
fn a_file_the_open_left_things_out_of_is_not_written_over_unasked() {
    let d = std::env::temp_dir().join(format!("vc-cmx-loss-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let src = d.join("drawing.cmx");
    let mut children = vec![cont(b'4', 1e-7), page(&[begin_page(None), square(&attrs_fill_type(2), 0, 0, 1000)])];
    children.extend(tables());
    let original = riff(&children);
    std::fs::write(&src, &original).unwrap();
    let mut s = Session::new();
    s.execute("document.open", &json!({ "path": src.to_string_lossy() })).unwrap();
    let e = s.execute("document.export", &json!({ "path": src.to_string_lossy(), "format": "svg" })).unwrap_err().to_string();
    assert!(e.contains("acknowledgeLoss"), "{e}");
    assert_eq!(std::fs::read(&src).unwrap(), original);
    std::fs::remove_dir_all(d).ok();
}

#[test]
fn files_that_are_not_read_say_why() {
    let mut s = Session::new();
    let mut rifx = sample();
    rifx[..4].copy_from_slice(b"RIFX");
    let e = open_bytes(&mut s, "a.cmx", &rifx, None).unwrap_err().to_string();
    assert!(e.contains("this CMX file can't be read (big-endian CMX files are not read)"), "{e}");
    let e = open_bytes(&mut s, "b.cmx", &riff(&[cont(b'2', 1e-3)]), None).unwrap_err().to_string();
    assert!(e.contains("this CMX file can't be read (CMX files with 16-bit coordinates are not read)"), "{e}");
    let e = open_bytes(&mut s, "c.cmx", &riff(&[]), None).unwrap_err().to_string();
    assert!(e.contains("this CMX file can't be read (damaged CMX header)"), "{e}");
    let e = open_bytes(&mut s, "d.cmx", b"not a cmx file", None).unwrap_err().to_string();
    assert!(e.contains("this CMX file can't be read (not a CMX file)"), "{e}");
}

#[test]
fn a_layer_without_a_name_is_named_by_its_place() {
    // An empty layer, which is left out, then a layer named only by spaces: it comes in unnamed,
    // and the importer numbers it by its place among the layers it keeps, as for a `.cdr` drawing.
    let p = page(&[
        begin_page(None),
        begin_layer("Grid", None),
        end_layer(),
        begin_layer("  ", None),
        square(&attrs(Some(2), None, false), 0, 0, 1000),
        end_layer(),
    ]);
    let mut children = vec![cont(b'4', 1e-7), p];
    children.extend(tables());
    let mut s = Session::new();
    let r = open_bytes(&mut s, "drawing.cmx", &riff(&children), None).unwrap();
    assert!(warnings(&r).is_empty(), "{r}");
    let names: Vec<String> = s.doc().unwrap().doc.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(names, ["Layer 1"]);
}
