//! Copying and pasting ruler guides (#3226): selected guides go on the clipboard with the objects
//! or alone, and paste into another document or onto another artboard.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use crate::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// The active document's guides: (vertical, pos, layer).
fn guides(s: &Session) -> Vec<(bool, f64, Option<NodeId>)> {
    s.doc().unwrap().doc.guides.iter().map(|g| (g.vertical, g.pos, g.layer)).collect()
}

#[test]
fn ruler_guides_copy_and_paste_into_another_document() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    run(&mut s, "guide.add", json!({"vertical": true, "pos": 120}));
    run(&mut s, "guide.add", json!({"vertical": false, "pos": 80}));
    run(&mut s, "guide.select", json!({"indexes": [0, 1]}));
    let r = run(&mut s, "edit.copy", json!({}));
    assert_eq!(r["guides"], 2);
    // Into another document: the same places, on its current layer, selected, one undo step.
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    let layer = s.doc().unwrap().doc.layers[0].id;
    let undo = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "edit.paste", json!({}));
    assert_eq!(r["guides"], 2);
    assert_eq!(guides(&s), [(true, 120.0, Some(layer)), (false, 80.0, Some(layer))], "where they were, not offset");
    assert_eq!(s.doc().unwrap().selection.guides, [0, 1]);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    run(&mut s, "edit.undo", json!({}));
    assert!(guides(&s).is_empty());
    // With an object, they move as it does (Paste offsets it by the Paste Offset preference).
    run(&mut s, "document.activate", json!({"index": 0}));
    let rect = run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [rect]}));
    run(&mut s, "guide.select", json!({"indexes": [0], "toggle": true}));
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "document.activate", json!({"index": 1}));
    run(&mut s, "edit.paste", json!({"dx": 5, "dy": 7}));
    assert_eq!(guides(&s).iter().map(|g| (g.0, g.1)).collect::<Vec<_>>(), [(true, 125.0)]);
}

#[test]
fn guides_paste_in_place_onto_another_artboard_and_cut_removes_them() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 100, "height": 100, "artboards": 2}));
    let [a, b] = [0, 1].map(|i| s.doc().unwrap().doc.artboards[i].rect);
    run(&mut s, "guide.add", json!({"vertical": true, "pos": a.x0 + 30.0}));
    run(&mut s, "guide.select", json!({"indexes": [0]}));
    // Cut takes the guide away and keeps it on the clipboard.
    run(&mut s, "edit.cut", json!({}));
    assert!(guides(&s).is_empty());
    // Paste in Place onto the second artboard: 30 pt into it.
    run(&mut s, "edit.pasteInPlace", json!({"artboard": 1}));
    assert_eq!(guides(&s).iter().map(|g| g.1).collect::<Vec<_>>(), [b.x0 + 30.0]);
    // Paste on All Artboards: one on each.
    run(&mut s, "edit.pasteOnAllArtboards", json!({}));
    let mut at: Vec<f64> = guides(&s).iter().map(|g| g.1).collect();
    at.sort_by(f64::total_cmp);
    assert_eq!(at, [a.x0 + 30.0, b.x0 + 30.0, b.x0 + 30.0]);
    // Cut with an object and a guide selected takes both away, in one undo step.
    let rect = run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}))["id"].as_u64().unwrap();
    let before = guides(&s).len();
    run(&mut s, "select.set", json!({"ids": [rect]}));
    run(&mut s, "guide.select", json!({"indexes": [0], "toggle": true}));
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, "edit.cut", json!({}));
    assert!(s.doc().unwrap().doc.node(NodeId(rect)).is_none());
    assert_eq!(guides(&s).len(), before - 1, "the selected guide goes too");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    // Cut with direct-selected anchors and a guide: the anchors and the guide go, one undo step.
    let path = run(&mut s, "shape.rectangle", json!({"x": 40, "y": 40, "width": 20, "height": 20}))["id"].as_u64().unwrap();
    run(&mut s, "guide.add", json!({"vertical": false, "pos": 90}));
    let n = guides(&s).len();
    run(&mut s, "select.anchors", json!({"id": path, "anchors": [[0, 0]], "mode": "set"}));
    run(&mut s, "guide.select", json!({"indexes": [n - 1], "toggle": true}));
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, "edit.cut", json!({}));
    assert_eq!(guides(&s).len(), n - 1, "the guide goes with the anchors");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    // Nothing selected: Copy is off.
    run(&mut s, "select.none", json!({}));
    assert!(s.execute("edit.copy", &json!({})).is_err());
}

/// #3226 review: each paste adds guides of its own (a guide is its place and layer; there is no
/// id to share), on the layer new art goes to, so they take that layer's colour, visibility and
/// lock; and the SVG put on the system clipboard carries no guides.
#[test]
fn two_pastes_add_their_own_guides_on_the_current_layer_and_svg_has_none() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    run(&mut s, "guide.add", json!({"vertical": true, "pos": 120}));
    run(&mut s, "guide.select", json!({"indexes": [0]}));
    run(&mut s, "edit.copy", json!({}));
    // Guides alone: no art for the system clipboard's SVG.
    assert!(s.clipboard_svg().is_none(), "guides alone put no SVG on the system clipboard");
    assert!(s.clipboard.flavour_types(true, true).is_empty(), "nor any other format (it keeps what other apps put there)");
    let layer = NodeId(run(&mut s, "layer.new", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(s.doc().unwrap().selection.guides, [1], "the first paste selects its own guide");
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(s.doc().unwrap().selection.guides, [2], "the second paste selects its own guide");
    let g = guides(&s);
    assert_eq!(g.len(), 3, "two pastes, two more guides: {g:?}");
    assert_eq!((g[1].2, g[2].2), (Some(layer), Some(layer)), "on the layer new art goes to");
    // Moving the second pasted guide leaves the first where it is.
    run(&mut s, "guide.move", json!({"index": 2, "pos": 200}));
    assert_eq!(guides(&s).iter().map(|g| g.1).collect::<Vec<_>>(), [120.0, 120.0, 200.0]);
    // Hiding that layer hides the guides pasted on it: they follow its visibility (and lock).
    run(&mut s, "layer.setProps", json!({"id": layer.0, "visible": false}));
    let st = s.doc().unwrap();
    let shown: Vec<bool> = st.doc.guides.iter().map(|g| st.doc.guide_shown(g)).collect();
    assert_eq!(shown, [true, false, false]);
    // With an object, the SVG holds the object and no guide.
    run(&mut s, "layer.setProps", json!({"id": layer.0, "visible": true}));
    let rect = run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids": [rect]}));
    run(&mut s, "guide.select", json!({"indexes": [0], "toggle": true}));
    let r = run(&mut s, "edit.copy", json!({}));
    assert_eq!(r["guides"], 1);
    let svg = s.clipboard_svg().unwrap();
    assert!(svg.contains("<rect") || svg.contains("<path"), "the object is there");
    assert!(!svg.to_lowercase().contains("guide"), "no guide in the SVG: {svg}");
}
