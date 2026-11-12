//! Ruler guides held as nodes of a layer: commands that work on a layer's art leave them out and
//! keep them above the art.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_doc::{Node, NodeId, NodeKind};
use vectorcraft_engine::Session;

fn id(v: &serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    id(&s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap())
}

/// A ruler guide added on top of `layer`'s contents.
fn guide(s: &mut Session, layer: NodeId, pos: f64) -> NodeId {
    let d = Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let g = Node::new(d.alloc_id(), NodeKind::RulerGuide { vertical: true, pos, artboard: None });
    d.insert(Some(layer), usize::MAX, g).unwrap()
}

fn children(s: &Session, layer: NodeId) -> Vec<NodeId> {
    s.doc().unwrap().doc.children(Some(layer)).unwrap().iter().map(|c| c.id).collect()
}

/// A document with a layer holding two rectangles and two ruler guides: (layer, [art], [guides]).
fn session() -> (Session, NodeId, Vec<NodeId>, Vec<NodeId>) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let layer = s.doc().unwrap().doc.layers[0].id;
    let art = vec![rect(&mut s, 10.0), rect(&mut s, 50.0)];
    let guides = vec![guide(&mut s, layer, 5.0), guide(&mut s, layer, 7.0)];
    (s, layer, art, guides)
}

#[test]
fn layer_commands_keep_ruler_guides_above_the_art() {
    // Send to Current Layer puts the object below the current layer's guides.
    let (mut s, a, art, guides) = session();
    let b = id(&s.execute("layer.new", &json!({})).unwrap());
    let moved = rect(&mut s, 90.0);
    s.execute("layer.setCurrent", &json!({"id": a.0})).unwrap();
    s.execute("select.set", &json!({"ids": [moved.0]})).unwrap();
    s.execute("object.arrange.sendToCurrentLayer", &json!({})).unwrap();
    assert_eq!(children(&s, a), [art[0], art[1], moved, guides[0], guides[1]]);
    // Already on top of the art: nothing changes and no undo step is added.
    let steps = s.doc().unwrap().history.undo.len();
    s.execute("object.arrange.sendToCurrentLayer", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), steps);

    // Moving a row on top of a layer's contents puts it below the guides.
    let other = rect(&mut s, 130.0);
    s.execute("layer.move", &json!({"id": other.0, "target": a.0})).unwrap();
    assert_eq!(children(&s, a), [art[0], art[1], moved, other, guides[0], guides[1]]);

    // Merging layers keeps every guide above the merged art.
    let gb = guide(&mut s, b, 9.0);
    let late = {
        s.execute("layer.setCurrent", &json!({"id": b.0})).unwrap();
        rect(&mut s, 170.0)
    };
    s.execute("layer.merge", &json!({"ids": [a.0, b.0]})).unwrap();
    let merged = children(&s, b);
    let first_guide = merged.iter().position(|c| [guides[0], guides[1], gb].contains(c)).unwrap();
    assert_eq!(merged.len() - first_guide, 3, "{merged:?}");
    assert!(merged.contains(&late));
}

#[test]
fn release_and_clipping_leave_ruler_guides_alone() {
    let (mut s, layer, art, guides) = session();
    // Make Clipping Mask clips by the top object, not by a guide above it.
    assert_eq!(s.execute("layer.clippingMask.toggle", &json!({"id": layer.0})).unwrap()["clip"], json!(true));
    assert_eq!(children(&s, layer), [art[1], art[0], guides[0], guides[1]]);
    s.execute("edit.undo", &json!({})).unwrap();

    // Release to Layers makes layers of the art only, below the guides.
    s.execute("layer.releaseToLayers", &json!({"id": layer.0})).unwrap();
    let ch = children(&s, layer);
    assert_eq!(ch.len(), 4);
    assert_eq!(ch[2..], [guides[0], guides[1]]);
    let d = &s.doc().unwrap().doc;
    assert!(ch[..2].iter().all(|c| d.node(*c).is_some_and(Node::is_layer)));
}

#[test]
fn ruler_guides_are_not_art_to_select_paint_or_inspect() {
    let (mut s, layer, art, guides) = session();
    // Target Contents selects the layer's objects only.
    let out = s.execute("appearance.targetContents", &json!({"ids": [layer.0]})).unwrap();
    assert_eq!(out["ids"], json!([art[0].0, art[1].0]));
    // Painting the layer paints its objects and leaves the guides with no appearance.
    s.execute("paint.setFill", &json!({"ids": [layer.0], "color": "#ff0000"})).unwrap();
    let d = &s.doc().unwrap().doc;
    for g in &guides {
        assert_eq!(d.node(*g).unwrap().appearance, vectorcraft_doc::Appearance::default());
    }
    // Document inspection lists the objects, not the guides.
    assert_eq!(s.execute("document.find", &json!({"kind": "Guide"})).unwrap()["total"], json!(0));
    let layer_row = s.execute("document.inspect", &json!({"ids": [layer.0]})).unwrap();
    let listed = layer_row.to_string();
    assert!(guides.iter().all(|g| !listed.contains(&format!("\"id\":{}", g.0))), "{listed}");
    // Unlock All does not reach a locked guide.
    Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(guides[0]).unwrap().locked = true;
    s.execute("object.unlockAll", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(guides[0]).unwrap().locked);
}

#[test]
fn flattening_a_sublayers_art_keeps_its_ruler_guides() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let layer = s.doc().unwrap().doc.layers[0].id;
    let sub = id(&s.execute("layer.newSublayer", &json!({"parent": layer.0})).unwrap());
    s.execute("layer.setCurrent", &json!({"id": sub.0})).unwrap();
    let r = rect(&mut s, 10.0);
    s.execute("transparency.set", &json!({"ids": [r.0], "opacity": 50})).unwrap();
    let g = guide(&mut s, sub, 5.0);
    s.execute("object.flattenTransparency", &json!({"ids": [sub.0]})).unwrap();
    // The sublayer stands for its art: the art is flattened and the guide stays on top of it.
    let d = &s.doc().unwrap().doc;
    assert!(d.node(r).is_none(), "the art was flattened");
    assert!(d.node(g).is_some_and(Node::is_ruler_guide), "the guide is kept");
    assert_eq!(d.parent_of(g), Some(sub));
    assert_eq!(children(&s, sub).len(), 2);
    assert_eq!(children(&s, sub).last(), Some(&g));
    assert_eq!(d.parent_of(sub), Some(layer));
}

#[test]
fn object_commands_refuse_a_ruler_guide_id() {
    let (mut s, _, art, guides) = session();
    s.execute("select.set", &json!({"ids": [art[0].0]})).unwrap();
    let before = (s.doc().unwrap().doc.clone(), s.doc().unwrap().selection.clone(), s.doc().unwrap().history.undo.len());
    for (command, params) in [
        ("select.set", json!({"ids": [guides[0].0]})),
        ("select.set", json!({"ids": [art[1].0, guides[1].0]})),
        ("edit.clear", json!({"ids": [guides[0].0]})),
        ("object.lock", json!({"ids": [guides[0].0]})),
        ("object.move", json!({"ids": [guides[0].0], "dx": 5, "dy": 0})),
        ("paint.setFill", json!({"ids": [guides[1].0], "color": "#ff0000"})),
    ] {
        let e = s.execute(command, &params).unwrap_err().to_string();
        assert!(e.contains("is a ruler guide, not an object"), "{command}: {e}");
        let d = s.doc().unwrap();
        assert_eq!((&d.doc, &d.selection, d.history.undo.len()), (&before.0, &before.1, before.2), "{command} changed something");
    }
    // A selection holding a guide id as an object loses it when pruned; the guide stays.
    let d = &s.doc().unwrap().doc;
    let mut sel = vectorcraft_doc::Selection { objects: vec![art[0], guides[0]], key: Some(guides[1]), ..Default::default() };
    sel.prune(d);
    assert_eq!((sel.objects, sel.key), (vec![art[0]], None));
    assert!(d.node(guides[0]).is_some_and(Node::is_ruler_guide));
}

#[test]
fn a_ruler_guide_stays_directly_in_a_layer() {
    let (mut s, layer, art, guides) = session();
    s.execute("select.set", &json!({"ids": [art[0].0, art[1].0]})).unwrap();
    let group = id(&s.execute("object.group", &json!({})).unwrap());
    let before = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    // Reorder refuses a group as a guide's parent, and the top level.
    for parent in [json!(group.0), json!(null)] {
        assert!(s.execute("node.move", &json!({"id": guides[0].0, "parent": parent, "index": 0})).is_err(), "{parent}");
        assert_eq!((&s.doc().unwrap().doc, s.doc().unwrap().history.undo.len()), (&before.0, before.1));
    }
    // So do the document's own insert and move.
    let d = Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    assert!(d.move_node(guides[1], Some(group), 0).is_err());
    let loose = Node::new(d.alloc_id(), NodeKind::RulerGuide { vertical: false, pos: 3.0, artboard: None });
    assert!(d.insert(Some(group), 0, loose.clone()).is_err());
    assert!(d.insert(None, 0, loose.clone()).is_err());
    assert_eq!(d.ruler_guides().iter().filter_map(|g| g.id).collect::<Vec<_>>(), guides);
    assert_eq!(children(&s, layer), [group, guides[0], guides[1]]);
    // Within its layer it still moves among the guides.
    s.execute("node.move", &json!({"id": guides[1].0, "parent": layer.0, "index": 0})).unwrap();
    assert_eq!(children(&s, layer), [group, guides[1], guides[0]]);
}

/// Arrange counts only a layer's art: the guides above it are no place to move to.
#[test]
fn arranging_counts_only_a_layers_art() {
    let (mut s, layer, art, guides) = session();
    let top = rect(&mut s, 90.0);
    assert_eq!(children(&s, layer), [art[0], art[1], top, guides[0], guides[1]]);
    s.execute("select.set", &json!({"ids": [art[0].0, top.0]})).unwrap();
    s.execute("object.arrange.bringToFront", &json!({})).unwrap();
    assert_eq!(children(&s, layer), [art[1], art[0], top, guides[0], guides[1]], "the selection keeps its order at the top of the art");
    // The topmost art brought to the front or forward: nothing changes and no undo step is added.
    s.execute("select.set", &json!({"ids": [top.0]})).unwrap();
    let steps = s.doc().unwrap().history.undo.len();
    for command in ["object.arrange.bringToFront", "object.arrange.bringForward"] {
        s.execute(command, &json!({})).unwrap();
        assert_eq!(children(&s, layer), [art[1], art[0], top, guides[0], guides[1]], "{command}");
        assert_eq!(s.doc().unwrap().history.undo.len(), steps, "{command}");
    }
    s.execute("select.set", &json!({"ids": [art[1].0]})).unwrap();
    s.execute("object.arrange.bringForward", &json!({})).unwrap();
    assert_eq!(children(&s, layer), [art[0], art[1], top, guides[0], guides[1]]);
}

/// A guide made while an opacity mask, a pattern or a symbol is edited goes on the temporary
/// editing layer, and stays in the document when editing ends; a save made during mask editing
/// keeps it.
#[test]
fn guides_made_while_editing_a_mask_a_pattern_or_a_symbol_stay() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    rect(&mut s, 10.0);
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    let edit_layer = s.doc().unwrap().doc.mask_edit.expect("editing the mask").layer;
    s.execute("guide.add", &json!({"vertical": true, "pos": 40})).unwrap();
    let made = s.doc().unwrap().doc.ruler_guides();
    assert_eq!(made.iter().map(|g| (g.layer, g.pos)).collect::<Vec<_>>(), [(Some(edit_layer), 40.0)]);
    let saved = s.doc().unwrap().doc.without_edit_modes().ruler_guides();
    assert_eq!(saved.iter().map(|g| (g.id, g.pos)).collect::<Vec<_>>(), [(made[0].id, 40.0)], "a save keeps the guide");
    s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    let kept = d.ruler_guides();
    assert_eq!(kept.iter().map(|g| (g.id, g.pos)).collect::<Vec<_>>(), [(made[0].id, 40.0)]);
    assert!(kept[0].layer.is_some_and(|l| l != edit_layer && d.node(l).is_some_and(Node::is_layer)));

    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    rect(&mut s, 10.0);
    s.execute("object.pattern.make", &json!({})).unwrap();
    let tile_layer = s.doc().unwrap().doc.pattern_edit.as_ref().expect("editing the pattern").layer;
    s.execute("guide.add", &json!({"vertical": false, "pos": 25})).unwrap();
    assert_eq!(s.doc().unwrap().doc.ruler_guides()[0].layer, Some(tile_layer));
    s.execute("object.pattern.done", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    let kept = d.ruler_guides();
    assert_eq!(kept.iter().map(|g| g.pos).collect::<Vec<_>>(), [25.0]);
    assert!(kept[0].layer.is_some_and(|l| l != tile_layer && d.node(l).is_some_and(Node::is_layer)));
    let tile = &d.patterns.last().expect("the new pattern").art;
    assert!(tile.iter().all(|n| !n.is_ruler_guide()), "the guide is not part of the tile");

    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    rect(&mut s, 10.0);
    let instance = id(&s.execute("symbol.new", &json!({"name": "Mark"})).unwrap());
    s.execute("symbol.edit", &json!({"ids": [instance.0]})).unwrap();
    let edit_layer = s.doc().unwrap().doc.symbol_edit.as_ref().expect("editing the symbol").layer;
    s.execute("guide.add", &json!({"vertical": true, "pos": 15})).unwrap();
    assert_eq!(s.doc().unwrap().doc.ruler_guides()[0].layer, Some(edit_layer));
    s.execute("symbol.exitEditing", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    let kept = d.ruler_guides();
    assert_eq!(kept.iter().map(|g| g.pos).collect::<Vec<_>>(), [15.0]);
    assert!(kept[0].layer.is_some_and(|l| l != edit_layer && d.node(l).is_some_and(Node::is_layer)));
    let art = &d.symbols.iter().find(|x| x.name == "Mark").expect("the symbol").art;
    let mut in_symbol = false;
    art.walk(&mut |n| in_symbol |= n.is_ruler_guide());
    assert!(!in_symbol, "the guide is not part of the symbol");
}

/// `bytes` without the lines that depend on when the file was written: the PDF dates, and the XMP
/// packet (one line, with its dates and IDs) and trailer `/ID`.
fn undated(bytes: &[u8]) -> Vec<u8> {
    let stamps: [&[u8]; 5] = [b"CreationDate", b"ModDate", b"xmp:CreateDate", b"xmp:ModifyDate", b"/ID["];
    let dated = |line: &&[u8]| stamps.iter().any(|s| line.windows(s.len()).any(|w| w == *s));
    bytes.split(|b| *b == b'\n').filter(|l| !dated(l)).flat_map(|l| l.iter().copied().chain(*b"\n")).collect()
}

/// The bytes of each export format (PDF without the embedded native document, which keeps the
/// guides, and without its dates), and the box Fit All in Window fits (the art and every artboard).
fn exports(s: &mut Session) -> (Vec<(String, Vec<u8>)>, Option<vectorcraft_geom::Rect>) {
    let files = [json!({"format": "svg"}), json!({"format": "pdf", "preserveEditing": false}), json!({"format": "png"}), json!({"format": "psd"})]
        .iter()
        .map(|p| {
            let out = s.execute("document.export", p).unwrap();
            let bytes = vectorcraft_format::base64_decode(out["dataBase64"].as_str().unwrap()).unwrap();
            let format = p["format"].as_str().unwrap().to_string();
            let bytes = if format == "pdf" { undated(&bytes) } else { bytes };
            (format, bytes)
        })
        .collect();
    let d = &s.doc().unwrap().doc;
    let fit = d.art_bounds().map(|a| d.artboards.iter().fold(a, |r, ab| r.union(ab.rect)));
    (files, fit)
}

/// Ruler guides on a layer, on a sublayer and on an artboard change no export and not the box
/// Fit All in Window fits.
#[test]
fn exports_and_fit_all_are_the_same_with_ruler_guides() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 150, "artboards": 2})).unwrap();
    let layer = s.doc().unwrap().doc.layers[0].id;
    rect(&mut s, 10.0);
    let sub = id(&s.execute("layer.newSublayer", &json!({"parent": layer.0})).unwrap());
    s.execute("layer.setCurrent", &json!({"id": sub.0})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 60, "y": 30, "width": 40, "height": 30})).unwrap();
    let without = exports(&mut s);
    assert_eq!(exports(&mut s), without, "the exports are repeatable");
    // Guides far outside the art too: they must not widen any box.
    s.execute("guide.add", &json!({"vertical": true, "pos": -5000, "layer": layer.0})).unwrap();
    s.execute("guide.add", &json!({"vertical": false, "pos": 9000, "layer": sub.0})).unwrap();
    s.execute("guide.add", &json!({"vertical": true, "pos": 120, "artboard": 1})).unwrap();
    assert_eq!(s.doc().unwrap().doc.ruler_guides().len(), 3);
    let with = exports(&mut s);
    for ((format, a), (_, b)) in without.0.iter().zip(&with.0) {
        assert!(a == b, "{format} export changed with ruler guides present");
    }
    assert_eq!(with.1, without.1, "Fit All");
    // A PDF that preserves editing carries them in its native document, and opens with them.
    let guides = s.doc().unwrap().doc.ruler_guides();
    let pdf = s.execute("document.export", &json!({"format": "pdf"})).unwrap()["dataBase64"].clone();
    s.execute("document.open", &json!({"name": "guides.pdf", "dataBase64": pdf})).unwrap();
    assert_eq!(s.doc().unwrap().doc.ruler_guides(), guides);
}

/// One step of [`guide_commands_follow_a_list_model`].
#[derive(Clone, Debug)]
enum GuideOp {
    Add { vertical: bool, pos: u16, layer: usize },
    Remove(usize),
    Copy(usize),
    Paste,
    SetCurrent(usize),
    Undo,
    Redo,
}

fn guide_op() -> impl proptest::strategy::Strategy<Value = GuideOp> {
    use proptest::prelude::*;
    prop_oneof![
        3 => (any::<bool>(), 0u16..400, 0usize..3).prop_map(|(vertical, pos, layer)| GuideOp::Add { vertical, pos, layer }),
        2 => (0usize..16).prop_map(GuideOp::Remove),
        1 => (0usize..16).prop_map(GuideOp::Copy),
        1 => Just(GuideOp::Paste),
        1 => (0usize..3).prop_map(GuideOp::SetCurrent),
        2 => Just(GuideOp::Undo),
        1 => Just(GuideOp::Redo),
    ]
}

/// A guide as `guide.list` reports it: (vertical, pos, layer).
type Line = (bool, f64, u64);

fn listed(s: &mut Session) -> Vec<Line> {
    let rows = s.execute("guide.list", &json!({})).unwrap();
    rows.as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, r)| {
            assert_eq!(r["index"], json!(i));
            (r["vertical"].as_bool().unwrap(), r["pos"].as_f64().unwrap(), r["layer"].as_u64().unwrap())
        })
        .collect()
}

/// Runs `ops` on a document with two layers and a sublayer, checking `guide.list` against a plain
/// list after every step: Add appends (a new guide takes the highest id), Remove takes the entry
/// out, Paste appends the copied guide on the current layer at the same place, and Undo and
/// Redo bring back the list as it was, the same indexes included.
fn run_guide_ops(ops: &[GuideOp]) -> Result<(), proptest::test_runner::TestCaseError> {
    use proptest::prop_assert_eq;
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let bottom = s.doc().unwrap().doc.layers[0].id;
    let top = id(&s.execute("layer.new", &json!({})).unwrap());
    let sub = id(&s.execute("layer.newSublayer", &json!({"parent": bottom.0})).unwrap());
    let layers = [bottom, top, sub];
    s.execute("layer.setCurrent", &json!({"id": bottom.0})).unwrap();
    let mut current = bottom;
    let mut model: Vec<Line> = vec![];
    let mut clip: Option<(bool, f64)> = None;
    let (mut undo, mut redo): (Vec<Vec<Line>>, Vec<Vec<Line>>) = (vec![], vec![]);
    for (step, op) in ops.iter().enumerate() {
        let steps = s.doc().unwrap().history.undo.len();
        let before = model.clone();
        // Whether the step is an edit (one undo step).
        let edit = match *op {
            GuideOp::Add { vertical, pos, layer } => {
                let l = layers[layer];
                let r = s.execute("guide.add", &json!({"vertical": vertical, "pos": pos, "layer": l.0})).unwrap();
                model.push((vertical, f64::from(pos), l.0));
                prop_assert_eq!(&r["index"], &json!(model.len() - 1), "step {}", step);
                true
            }
            GuideOp::Remove(i) if !model.is_empty() => {
                let i = i % model.len();
                s.execute("guide.remove", &json!({"index": i})).unwrap();
                model.remove(i);
                true
            }
            GuideOp::Copy(i) if !model.is_empty() => {
                let i = i % model.len();
                s.execute("guide.select", &json!({"indexes": [i]})).unwrap();
                s.execute("edit.copy", &json!({})).unwrap();
                clip = Some((model[i].0, model[i].1));
                false
            }
            GuideOp::Paste => match clip {
                Some((vertical, pos)) => {
                    s.execute("edit.paste", &json!({})).unwrap();
                    model.push((vertical, pos, current.0));
                    true
                }
                None => false,
            },
            GuideOp::SetCurrent(l) => {
                current = layers[l];
                s.execute("layer.setCurrent", &json!({"id": current.0})).unwrap();
                false
            }
            GuideOp::Undo => {
                if let Some(m) = undo.pop() {
                    s.execute("edit.undo", &json!({})).unwrap();
                    redo.push(std::mem::replace(&mut model, m));
                }
                false
            }
            GuideOp::Redo => {
                if let Some(m) = redo.pop() {
                    s.execute("edit.redo", &json!({})).unwrap();
                    undo.push(std::mem::replace(&mut model, m));
                }
                false
            }
            GuideOp::Remove(_) | GuideOp::Copy(_) => false,
        };
        let after = s.doc().unwrap().history.undo.len();
        if edit {
            prop_assert_eq!(after, steps + 1, "step {} {:?}: one undo step", step, op);
            undo.push(before);
            redo.clear();
        } else if !matches!(op, GuideOp::Undo | GuideOp::Redo) {
            prop_assert_eq!(after, steps, "step {} {:?}: no undo step", step, op);
        }
        prop_assert_eq!(listed(&mut s), model.clone(), "step {} {:?}", step, op);
    }
    Ok(())
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config { cases: 64, failure_persistence: None, ..proptest::test_runner::Config::default() })]

    /// Add, remove, copy, paste, undo and redo keep `guide.list` in step with a plain list, on
    /// three layers.
    #[test]
    fn guide_commands_follow_a_list_model(ops in proptest::collection::vec(guide_op(), 1..40)) {
        run_guide_ops(&ops)?;
    }
}
