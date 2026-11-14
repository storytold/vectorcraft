use super::*;
use vectorcraft_geom::shapes;

mod lookups;
mod slices;
mod units;

/// A guide label's text is a layer color's name or `#rrggbb`, and reads back as the same color;
/// other spellings, an alpha part and a short form are not labels.
#[test]
fn guide_label_text_names_a_layer_color_or_rgb() {
    for (i, (name, _)) in LAYER_COLORS.iter().enumerate() {
        let c = LayerColor::Preset(u8::try_from(i).unwrap());
        assert_eq!(c.label_text(), *name);
        assert_eq!(LayerColor::from_label_text(name), Some(c));
    }
    let custom = LayerColor::Custom([0x0a, 0xbc, 0xff]);
    assert_eq!(custom.label_text(), "#0abcff");
    assert_eq!(LayerColor::from_label_text("#0ABCff"), Some(custom));
    // A preset past the layer colors has no name: its text is the color it draws in.
    let [r, g, b] = LayerColor::Preset(200).rgb();
    assert_eq!(LayerColor::Preset(200).label_text(), format!("#{r:02x}{g:02x}{b:02x}"));
    for bad in ["red", "Red ", "", "#", "#fff", "#0abcff00", "0abcff", "#0abcfg", "#０abcff"] {
        assert_eq!(LayerColor::from_label_text(bad), None, "{bad:?}");
    }
}

/// An artboard's guide label: none unless set, left out of the JSON when none, and a preset past
/// the layer colors reading as none.
#[test]
fn artboard_guide_label_defaults_and_skips_in_json() {
    let mut a = Document::new(100.0, 100.0).artboards[0].clone();
    assert_eq!(a.guide_label, None);
    let plain = serde_json::to_value(&a).unwrap();
    assert!(plain.get("guide_label").is_none(), "{plain}");
    a.guide_label = Some(LayerColor::Custom([1, 2, 3]));
    let v = serde_json::to_value(&a).unwrap();
    assert_eq!(v["guide_label"], serde_json::json!({"Custom": [1, 2, 3]}));
    assert_eq!(serde_json::from_value::<Artboard>(v).unwrap(), a);
    let read = |label: serde_json::Value| {
        let mut v = plain.clone();
        v["guide_label"] = label;
        serde_json::from_value::<Artboard>(v).map(|a| a.guide_label)
    };
    assert_eq!(read(serde_json::json!({"Preset": 26})).unwrap(), Some(LayerColor::Preset(26)));
    assert_eq!(read(serde_json::json!({"Preset": 27})).unwrap(), None);
    assert_eq!(read(serde_json::json!({"Preset": 255})).unwrap(), None);
    assert_eq!(read(serde_json::Value::Null).unwrap(), None);
    assert!(read(serde_json::json!({"Preset": -1})).is_err());
    assert!(read(serde_json::json!({"Tint": 1})).is_err());
    assert_eq!(serde_json::from_value::<Artboard>(plain).unwrap().guide_label, None, "older files have none");
}

/// Zooming in on an inch ruler divided the inch into tenths (#2827). Its steps below an inch
/// are binary fractions now, and the minor ticks between them stay binary too; every other
/// unit is still read in tenths.
#[test]
fn an_inch_ruler_steps_in_binary_fractions() {
    let step = |u: Unit, zoom: f64| u.ruler_step(zoom);
    // Every inch step at or below an inch is 1/2^k, never a tenth or a fifth.
    for zoom in [1.0, 2.0, 5.0, 7.5, 10.0, 20.0, 40.0, 100.0, 400.0] {
        let s = step(Unit::Inches, zoom);
        if s <= 1.0 {
            let k = s.recip();
            assert_eq!(k, k.round(), "{s} at zoom {zoom} is 1/k for a whole k");
            assert!((k.round() as u64).is_power_of_two(), "{s} at zoom {zoom} is 1/2^k");
        }
        // A label step always leaves at least 50 px, which is what the ladder is picked for.
        assert!(s * Unit::Inches.points() * zoom >= 50.0, "zoom {zoom}");
    }
    // The steps the report named: tenths became quarters, eighths and sixteenths.
    assert_eq!(step(Unit::Inches, 5.0), 0.25);
    assert_eq!(step(Unit::Inches, 10.0), 0.125);
    assert_eq!(step(Unit::Inches, 20.0), 0.0625);
    // An inch and wider is unchanged.
    assert_eq!(step(Unit::Inches, 1.0), 1.0);
    // Labelling stops at a sixteenth, as the report says Illustrator's does, so zooming in
    // further only adds ticks. A label therefore never needs more than the six characters of
    // "0.0625", which the left ruler has room for when it spells one out a character per line.
    for zoom in [20.0, 50.0, 100.0, 400.0, 1600.0] {
        assert_eq!(step(Unit::Inches, zoom), 0.0625, "zoom {zoom}");
    }
    // The unlabelled ticks go on down to a hundred and twenty-eighth: the 32nds, 64ths and
    // 128ths the report expects to appear unlabelled when zoomed in close.
    assert_eq!(Unit::Inches.ruler_minor(0.0625), 1.0 / 128.0);
    // The minor ticks: an eighth of the step on an inch ruler, a tenth elsewhere.
    assert_eq!(Unit::Inches.ruler_minor(0.25), 0.03125);
    assert_eq!(Unit::Millimeters.ruler_minor(25.0), 2.5);
    assert_eq!(Unit::Points.ruler_minor(100.0), 10.0);
    // Other units keep the decimal ladder, so a tenth and a fifth still appear.
    assert!([0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 25.0].contains(&step(Unit::Millimeters, 5.0)), "{}", step(Unit::Millimeters, 5.0));
}

/// Ruler guide nodes on a layer and a sublayer are not art: Select All, a layer's target circle,
/// the object count, the art's bounds, clicks, the marquee and a knockout layer's elements are
/// the same as without them, and transforming the layer leaves them where they are.
#[test]
fn ruler_guide_nodes_are_not_art() {
    use crate::hit::{HitOptions, hit_test, marquee, objects_at};
    let (mut d, a, b) = doc_with_rects();
    let layer = d.layers[0].id;
    let sub = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::layer(sub, "Sub", LayerColor::Preset(2))).unwrap();
    let s1 = d.alloc_id();
    d.insert(Some(sub), usize::MAX, Node::path(s1, shapes::rectangle(Rect::new(40.0, 0.0, 50.0, 10.0)), Appearance::default_art())).unwrap();
    let opt = HitOptions::default();
    let all = Rect::new(-100.0, -100.0, 700.0, 900.0);
    let before = (
        d.selectable_art(),
        d.layers[0].layer_art(false),
        d.node_count(),
        d.art_bounds(),
        marquee(&d, all, None, false),
        hit_test(&d, Point::new(5.0, 5.0), opt).map(|h| h.top_object(None)),
    );
    let guide = |id, vertical, pos| Node::new(id, NodeKind::RulerGuide { vertical, pos, artboard: None });
    let (g1, g2, g3) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    d.insert(Some(layer), usize::MAX, guide(g1, true, 5.0)).unwrap();
    d.insert(Some(layer), usize::MAX, guide(g2, false, 5.0)).unwrap();
    d.insert(Some(sub), usize::MAX, guide(g3, true, 45.0)).unwrap();
    let after = (
        d.selectable_art(),
        d.layers[0].layer_art(false),
        d.node_count(),
        d.art_bounds(),
        marquee(&d, all, None, false),
        hit_test(&d, Point::new(5.0, 5.0), opt).map(|h| h.top_object(None)),
    );
    assert_eq!(after, before);
    assert_eq!(after.0, [a, b, s1]);
    for p in [Point::new(5.0, 300.0), Point::new(300.0, 5.0), Point::new(45.0, 300.0)] {
        assert!(hit_test(&d, p, opt).is_none(), "nothing to click on the guide at {p:?}");
        assert!(objects_at(&d, p, opt, None).is_empty());
    }
    let mut sel = Selection::default();
    sel.set_target(&d, layer);
    assert_eq!(sel.objects, [a, b, s1]);
    let children = d.layers[0].children().unwrap().clone();
    assert_eq!(Node::knockout_elements(&children).iter().map(|c| c.id).collect::<Vec<_>>(), [a, b, s1]);
    assert!(d.node(g1).is_some_and(Node::is_ruler_guide) && d.node(g1).is_some_and(Node::is_guide));
    assert_eq!(d.node(g1).map(Node::geometric_bounds), Some(None));
    let mut moved = (*d.layers[0]).clone();
    moved.transform(Affine::translate((10.0, 10.0)), true);
    let g = moved.children().unwrap().iter().find(|c| c.id == g1).unwrap();
    assert!(matches!(g.kind, NodeKind::RulerGuide { pos, .. } if pos == 5.0));
}

/// Art inserted or moved onto a layer goes below the layer's ruler guides, a ruler guide goes
/// among them, and a clipped layer whose art is gone does not clip by a guide.
#[test]
fn layer_art_stays_below_its_ruler_guides() {
    let (mut d, a, b) = doc_with_rects();
    let layer = d.layers[0].id;
    let guide = |id, pos| Node::new(id, NodeKind::RulerGuide { vertical: true, pos, artboard: None });
    let (g1, g2) = (d.alloc_id(), d.alloc_id());
    d.insert(Some(layer), usize::MAX, guide(g1, 5.0)).unwrap();
    // Asked for the bottom, a guide still goes above the art.
    d.insert(Some(layer), 0, guide(g2, 7.0)).unwrap();
    let ids = |d: &Document| d.layers[0].children().unwrap().iter().map(|c| c.id).collect::<Vec<_>>();
    assert_eq!(ids(&d), [a, b, g2, g1]);
    let c = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::path(c, shapes::rectangle(Rect::new(0.0, 0.0, 5.0, 5.0)), Appearance::default_art())).unwrap();
    assert_eq!(ids(&d), [a, b, c, g2, g1]);
    d.move_node(a, Some(layer), usize::MAX).unwrap();
    assert_eq!(ids(&d), [b, c, a, g2, g1]);
    d.move_node(g1, Some(layer), 0).unwrap();
    assert_eq!(ids(&d), [b, c, a, g1, g2]);
    assert!(d.layer_object(g1).is_none());
    assert_eq!(d.layer_object(a).map(|n| n.id), Some(a));

    // A clipped layer clips by its first art object; with only guides left it clips nothing.
    if let Some(NodeKind::Layer { clip, .. }) = d.node_mut(layer).map(|n| &mut n.kind) {
        *clip = true;
    }
    assert!(d.node(layer).is_some_and(Node::clips));
    for id in [a, b, c] {
        d.remove(id).unwrap();
    }
    assert!(!d.node(layer).is_some_and(Node::clips));

    // Reading a file that clips a layer by a path beyond the canvas empties the layer of its art
    // only: its guides stay.
    let far = d.alloc_id();
    d.insert(Some(layer), 0, Node::path(far, shapes::rectangle(Rect::new(0.0, 0.0, 1e9, 10.0)), Appearance::default_art())).unwrap();
    assert!(d.node(layer).is_some_and(Node::clips));
    let gone = d.drop_beyond_canvas();
    assert_eq!(gone.objects, 1);
    assert_eq!(ids(&d), [g1, g2]);
}

/// `ruler_guides` keeps its list while the layers are unchanged, and lists again after any change
/// inside a layer, made in place or on a shared copy, and after layers are added, removed or
/// reordered.
#[test]
fn ruler_guides_follow_every_edit() {
    let (mut d, _, _) = doc_with_rects();
    let layer = d.layers[0].id;
    let sub = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::layer(sub, "Sub", LayerColor::Preset(1))).unwrap();
    let g1 = d.add_ruler_guide(&Guide { layer: Some(layer), ..Guide::new(true, 5.0) });
    let g2 = d.add_ruler_guide(&Guide { layer: Some(sub), ..Guide::new(false, 6.0) });
    let line = |d: &Document| d.ruler_guides().iter().map(|g| (g.id, g.pos, g.hidden, g.layer)).collect::<Vec<_>>();
    assert_eq!(line(&d), [(Some(g1), 5.0, false, Some(layer)), (Some(g2), 6.0, false, Some(sub))]);
    // Changed in place: the document is the only holder of its layers.
    assert!(d.edit_ruler_guide(g2, |g| g.pos = 9.0));
    assert_eq!(line(&d)[1], (Some(g2), 9.0, false, Some(sub)));
    d.node_mut(g1).unwrap().visible = false;
    assert_eq!(line(&d)[0], (Some(g1), 5.0, true, Some(layer)));
    // A copy shares the list until one of them changes.
    let copy = d.clone();
    assert_eq!(line(&copy), line(&d));
    d.remove(g1).unwrap();
    assert_eq!(line(&d), [(Some(g2), 9.0, false, Some(sub))]);
    assert_eq!(line(&copy).len(), 2);
    // A new layer with a guide, then the layers reordered and one removed.
    let top = d.add_layer(None);
    let g3 = d.add_ruler_guide(&Guide { layer: Some(top), ..Guide::new(true, 1.0) });
    assert_eq!(line(&d).len(), 2);
    d.layers.reverse();
    assert_eq!(line(&d), [(Some(g2), 9.0, false, Some(sub)), (Some(g3), 1.0, false, Some(top))]);
    d.layers.retain(|l| l.id != layer);
    assert_eq!(line(&d), [(Some(g3), 1.0, false, Some(top))]);
}

#[test]
fn strip_ruler_guides_leaves_a_layers_art() {
    let (mut d, a, b) = doc_with_rects();
    let layer = d.layers[0].id;
    let guide = |id| Node::new(id, NodeKind::RulerGuide { vertical: false, pos: 3.0, artboard: None });
    let sub = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::layer(sub, "Sub", LayerColor::Preset(1))).unwrap();
    let (g1, g2) = (d.alloc_id(), d.alloc_id());
    d.insert(Some(layer), usize::MAX, guide(g1)).unwrap();
    d.insert(Some(sub), usize::MAX, guide(g2)).unwrap();
    let mut n = (*d.layers[0]).clone();
    assert!(n.holds_ruler_guides());
    n.strip_ruler_guides();
    assert!(!n.holds_ruler_guides());
    let mut left = vec![];
    n.walk(&mut |c| left.push(c.id));
    assert_eq!(left, [layer, a, b, sub]);
    // Other nodes hold none and are left as they are.
    let mut p = d.node(a).unwrap().clone();
    p.strip_ruler_guides();
    assert_eq!(&p, d.node(a).unwrap());
}

/// The saved `guides` list becomes ruler guide nodes and is written back from them unchanged:
/// guides on a layer, a sublayer, no layer and a deleted layer, with an artboard guide, keep their
/// order, ids, names and their own lock and visibility; a saved id that another node has is
/// replaced; and the saved form of an older entry still reads.
#[test]
fn saved_guides_become_nodes_and_are_saved_back_the_same() {
    let (mut d, a, _) = doc_with_rects();
    let layer = d.layers[0].id;
    let top = d.add_layer(Some("Top"));
    let sub = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::layer(sub, "Sub", LayerColor::Preset(1))).unwrap();
    let json = serde_json::json!([
        {"vertical": true, "pos": 10.0, "layer": layer},
        {"vertical": false, "pos": 20.0, "layer": sub, "artboard": 1},
        {"vertical": true, "pos": 30.0},
        {"vertical": false, "pos": 40.0, "layer": 9999},
        {"vertical": true, "pos": 50.0, "layer": layer, "id": 500, "name": "Edge", "locked": true, "hidden": true},
        {"vertical": true, "pos": 60.0, "layer": layer, "id": a},
    ]);
    d.saved_guides = serde_json::from_value(json).unwrap();
    let before = d.peek_next_id();
    assert_eq!(d.guides_into_nodes(), 6);
    assert!(d.saved_guides.is_empty());
    let guides = d.ruler_guides();
    let lines: Vec<(f64, Option<NodeId>)> = guides.iter().map(|g| (g.pos, g.layer)).collect();
    // A guide on no layer, or on a deleted one, goes on the top visible, unlocked layer.
    assert_eq!(lines[..4], [(50.0, Some(layer)), (10.0, Some(layer)), (20.0, Some(sub)), (30.0, Some(top))], "by id: the kept id first");
    assert_eq!(lines[4..], [(40.0, Some(top)), (60.0, Some(layer))]);
    assert_eq!(guides[0].id, Some(NodeId(500)));
    assert!(guides[1..].iter().all(|g| g.id.unwrap().0 > 500), "new ids start above a kept one");
    assert_ne!(guides[5].id, Some(a), "an id another object has is not reused");
    assert!(d.peek_next_id() > 500 && before <= 500);
    let edge = d.node(NodeId(500)).unwrap();
    assert!(edge.locked && !edge.visible && edge.name.as_deref() == Some("Edge"));
    assert_eq!(guides[2].artboard, Some(1));
    // Each layer's guides sit at its top, above its art and its sublayer.
    let kids: Vec<bool> = d.node(layer).unwrap().children().unwrap().iter().map(|c| c.is_ruler_guide()).collect();
    assert_eq!(kids, [false, false, false, true, true, true]);
    assert_eq!(d.guides_into_nodes(), 0, "a second run does nothing");

    // Saved back: the guides leave the layers and the list names each one's layer and id.
    let mut saved = d.clone();
    saved.guides_to_saved();
    assert!(saved.ruler_guides().is_empty());
    assert_eq!(saved.node_count(), d.node_count());
    assert_eq!(saved.saved_guides, guides);
    let json = serde_json::to_value(&saved).unwrap();
    assert_eq!(json["guides"].as_array().map(Vec::len), Some(6));
    assert_eq!(
        json["guides"][0],
        serde_json::json!({"vertical": true, "pos": 50.0, "layer": layer, "id": 500, "name": "Edge", "locked": true, "hidden": true})
    );
    // Read again, the same guides come back with the same ids.
    let mut back: Document = serde_json::from_value(json).unwrap();
    back.fix_next_id();
    back.guides_into_nodes();
    assert_eq!(back.ruler_guides(), guides);
    // An older reader of the list ignores the new fields.
    #[derive(serde::Deserialize, PartialEq, Debug)]
    struct Old {
        vertical: bool,
        pos: f64,
        #[serde(default)]
        artboard: Option<u32>,
        #[serde(default)]
        layer: Option<NodeId>,
    }
    let old: Vec<Old> = serde_json::from_value(serde_json::to_value(&saved.saved_guides).unwrap()).unwrap();
    assert_eq!(old[0], Old { vertical: true, pos: 50.0, artboard: None, layer: Some(layer) });

    // A document with no layer gets a "Layer 1" for its guides.
    let mut empty = Document::new(10.0, 10.0);
    empty.layers.clear();
    empty.saved_guides = vec![Guide::new(true, 1.0)];
    assert_eq!(empty.guides_into_nodes(), 1);
    assert_eq!(empty.layers.len(), 1);
    assert_eq!(empty.layers[0].name.as_deref(), Some("Layer 1"));
    assert_eq!(empty.ruler_guides().len(), 1);
}

/// Editing, deleting and the artboard moves of ruler guide nodes, and the selection keeping step.
#[test]
fn ruler_guide_nodes_are_edited_by_id() {
    let (mut d, a, _) = doc_with_rects();
    let layer = d.layers[0].id;
    let g1 = d.add_ruler_guide(&Guide { layer: Some(layer), ..Guide::new(true, 10.0) });
    let g2 = d.add_ruler_guide(&Guide { artboard: Some(1), ..Guide::new(false, 20.0) });
    assert!(d.edit_ruler_guide(g1, |g| g.pos = 15.0));
    assert!(!d.edit_ruler_guide(a, |g| g.pos = 15.0), "not a guide");
    assert_eq!(d.ruler_guide(g1).map(|g| g.pos), Some(15.0));
    d.move_artboard_guides(1, Vec2::new(5.0, 7.0));
    assert_eq!(d.ruler_guide(g2).map(|g| g.pos), Some(27.0));
    d.map_artboard_guides(1, Affine::scale(2.0));
    assert_eq!(d.ruler_guide(g2).map(|g| g.pos), Some(54.0));
    assert_eq!(d.ruler_guide(g1).map(|g| g.pos), Some(15.0), "a canvas guide stays");
    d.copy_artboard_guides(1, 2, Vec2::new(0.0, 100.0));
    let copies: Vec<(f64, Option<u32>)> = d.ruler_guides().iter().filter(|g| g.artboard == Some(2)).map(|g| (g.pos, g.artboard)).collect();
    assert_eq!(copies, [(154.0, Some(2))]);
    let mut sel = Selection::default();
    sel.set_guides([g1, g2]);
    sel.key_guide = Some(g2);
    assert_eq!(d.retain_guides(&mut sel, |g| g.artboard.is_none()), 2);
    assert_eq!(sel.guides, [g1]);
    assert_eq!(sel.key_guide, None);
    assert_eq!(d.ruler_guides().iter().map(|g| g.id).collect::<Vec<_>>(), [Some(g1)]);
    assert_eq!(d.retain_guides(&mut sel, |_| true), 0);
}

fn doc_with_rects() -> (Document, NodeId, NodeId) {
    let mut d = Document::new(612.0, 792.0);
    let layer = d.layers[0].id;
    let a = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art())).unwrap();
    let b = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::path(b, shapes::rectangle(Rect::new(20.0, 0.0, 30.0, 10.0)), Appearance::default_art())).unwrap();
    (d, a, b)
}

/// Output draws the background colors of the artboards asked for, and only theirs, as rectangles
/// of those artboards on a layer below the art; without a color there is nothing to draw.
#[test]
fn artboard_fills_go_below_the_art() {
    let (mut d, a, _) = doc_with_rects();
    assert!(d.with_artboard_fills([0]).is_none(), "no color, no fill");
    let second = Rect::new(700.0, 0.0, 800.0, 100.0);
    d.artboards.push(Artboard { id: 2, rect: second, ..d.artboards[0].clone() });
    let blue = vectorcraft_color::Color::cmyk(1.0, 0.5, 0.0, 0.0);
    d.artboards[1].color = Some(blue);
    assert!(d.with_artboard_fills([0]).is_none(), "only the artboards asked for");
    let out = d.with_artboard_fills([0, 1]).unwrap();
    assert_eq!(out.layers.len(), d.layers.len() + 1);
    assert_eq!(out.layers[1..], d.layers[..], "the art stays as it was, above the fills");
    let fills = out.layers[0].children().unwrap();
    assert_eq!(out.layers[0].display_name(), "Artboard Fill");
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].geometric_bounds(), Some(second));
    assert_eq!(fills[0].appearance.fill_paint(), vectorcraft_color::Paint::solid(blue), "in the color's own model");
    assert!(out.node(a).is_some() && out.node(fills[0].id).is_some() && fills[0].id != a);
    assert_eq!(d.artboards, out.artboards);
}

#[test]
fn new_document_has_layer_and_artboard() {
    let d = Document::new(612.0, 792.0);
    assert_eq!(d.layers.len(), 1);
    assert_eq!(d.layers[0].display_name(), "Layer 1");
    assert_eq!(d.artboards[0].rect, Rect::new(0.0, 0.0, 612.0, 792.0));
}

#[test]
fn art_on_artboard_takes_unlocked_objects_wholly_inside() {
    let (mut d, a, b) = doc_with_rects();
    let wide = Rect::new(-1.0, -1.0, 40.0, 15.0);
    assert_eq!(d.art_on_artboard(Rect::new(-1.0, -1.0, 15.0, 15.0), false), vec![a]);
    assert_eq!(d.art_on_artboard(Rect::new(-1.0, -1.0, 25.0, 15.0), false), vec![a], "b only half inside");
    // Flush with the artboard's edges counts as inside: the right and bottom ones too (#890).
    for board in [Rect::new(0.0, 0.0, 10.0, 10.0), Rect::new(-5.0, 0.0, 10.0, 15.0), Rect::new(0.0, -5.0, 15.0, 10.0)] {
        assert_eq!(d.art_on_artboard(board, false), vec![a], "{board:?}");
    }
    d.node_mut(a).unwrap().locked = true;
    assert_eq!(d.art_on_artboard(wide, false), vec![b]);
    // Move Locked and Hidden Artwork with Artboard (#394): hidden art stays too, unless on.
    d.node_mut(b).unwrap().visible = false;
    assert!(d.art_on_artboard(wide, false).is_empty());
    assert_eq!(d.art_on_artboard(wide, true), vec![a, b]);
    Arc::make_mut(&mut d.layers[0]).locked = true;
    assert!(d.art_on_artboard(wide, false).is_empty());
    assert_eq!(d.art_on_artboard(wide, true), vec![a, b], "a locked layer's art too");
}

#[test]
fn art_on_artboard_tolerates_roundoff_at_each_edge() {
    let (d, a, _) = doc_with_rects();
    let roundoff = 1e-12;
    for board in [
        Rect::new(roundoff, 0.0, 10.0, 10.0),
        Rect::new(0.0, roundoff, 10.0, 10.0),
        Rect::new(0.0, 0.0, 10.0 - roundoff, 10.0),
        Rect::new(0.0, 0.0, 10.0, 10.0 - roundoff),
    ] {
        assert_eq!(d.art_on_artboard(board, false), vec![a], "roundoff at {board:?}");
    }
    // A real overhang remains outside.
    for board in
        [Rect::new(0.001, 0.0, 10.0, 10.0), Rect::new(0.0, 0.001, 10.0, 10.0), Rect::new(0.0, 0.0, 9.999, 10.0), Rect::new(0.0, 0.0, 10.0, 9.999)]
    {
        assert!(d.art_on_artboard(board, false).is_empty(), "overhang at {board:?}");
    }
}

#[test]
fn art_for_artboard_copy_takes_artwork_larger_than_artboard_or_with_bleed() {
    let mut d = Document::new(200.0, 200.0);
    d.setup.bleed = [9.0; 4];
    let layer = d.layers[0].id;
    // An oversized background (250x250 centered on the 200x200 artboard, #4479).
    let bg = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::path(bg, shapes::rectangle(Rect::new(-25.0, -25.0, 225.0, 225.0)), Appearance::default_art())).unwrap();
    // A shape with 5pt bleed over the top-right corner.
    let bleed = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::path(bleed, shapes::rectangle(Rect::new(150.0, -5.0, 205.0, 50.0)), Appearance::default_art())).unwrap();
    // A shape lying entirely in the bleed margin without touching the artboard itself (-8..-2 vs 0..200).
    let bleed_margin_only = d.alloc_id();
    d.insert(
        Some(layer),
        usize::MAX,
        Node::path(bleed_margin_only, shapes::rectangle(Rect::new(-8.0, 50.0, -2.0, 100.0)), Appearance::default_art()),
    )
    .unwrap();
    // A small shape hanging 20pt over the left edge (center outside, more than bleed).
    let hanging = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::path(hanging, shapes::rectangle(Rect::new(-20.0, 90.0, 10.0, 120.0)), Appearance::default_art()))
        .unwrap();
    // A straight line hanging 60pt over the left edge (center outside, no area): it lies on the
    // artboard for 40pt, so it is copied as a thin rectangle there would be.
    let line = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::path(line, shapes::line(Point::new(-60.0, 150.0), Point::new(40.0, 150.0)), Appearance::default_art()))
        .unwrap();
    // A shape completely outside the artboard.
    let outside = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::path(outside, shapes::rectangle(Rect::new(300.0, 300.0, 400.0, 400.0)), Appearance::default_art()))
        .unwrap();

    // Standard art_on_artboard keeps strict containment: none of the overhanging shapes are on it.
    let strict_art = d.art_on_artboard(Rect::new(0.0, 0.0, 200.0, 200.0), false);
    assert!(!strict_art.contains(&bg), "strict art_on_artboard excludes oversized background");
    assert!(!strict_art.contains(&bleed), "strict art_on_artboard excludes bleed shape");
    assert!(!strict_art.contains(&bleed_margin_only), "strict art_on_artboard excludes bleed-margin-only shape");
    assert!(!strict_art.contains(&hanging), "strict art_on_artboard excludes hanging shape");
    assert!(!strict_art.contains(&outside), "strict art_on_artboard excludes outside shape");

    // Copying the artboard takes the artwork touching this artboard that belongs to it, or within bleed.
    let copy_art = d.art_for_artboard_copy(0, false);
    assert!(copy_art.contains(&bg), "oversized background is copied with artboard: {copy_art:?}");
    assert!(copy_art.contains(&bleed), "bleed shape is copied with artboard: {copy_art:?}");
    assert!(copy_art.contains(&bleed_margin_only), "bleed-margin-only shape is copied with artboard: {copy_art:?}");
    assert!(copy_art.contains(&hanging), "touching shape owned by artboard 0 is copied: {copy_art:?}");
    assert!(copy_art.contains(&line), "a line hanging over the edge is copied: {copy_art:?}");
    assert!(!copy_art.contains(&outside), "outside shape is not copied: {copy_art:?}");

    // If Artboard 1 is added next to Artboard 0, art primarily on Artboard 1 is not copied with Artboard 0:
    d.artboards.push(Artboard {
        id: 2,
        name: "Artboard 2".into(),
        rect: Rect::new(200.0, 0.0, 400.0, 200.0),
        locked: false,
        show_center_mark: false,
        show_cross_hairs: false,
        show_video_safe_areas: false,
        video_ruler_pixel_aspect_ratio: 1.0,
        color: None,
    });
    // Shape spanning (190..300) x (50..150): 10pt on Artboard 0, 100pt on Artboard 1.
    let neighbor_art = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::path(neighbor_art, shapes::rectangle(Rect::new(190.0, 50.0, 300.0, 150.0)), Appearance::default_art()))
        .unwrap();

    let copy_art0 = d.art_for_artboard_copy(0, false);
    assert!(!copy_art0.contains(&neighbor_art), "art owned by neighboring Artboard 1 is not copied with Artboard 0");
    let copy_art1 = d.art_for_artboard_copy(1, false);
    assert!(copy_art1.contains(&neighbor_art), "art owned by Artboard 1 is copied with Artboard 1");
}

#[test]
fn artboard_for_bounds_edge_cases_and_tie_breaks() {
    let mut d = Document::new(100.0, 100.0);
    // Artboard 0: (0, 0, 100, 100)
    // Artboard 1: (200, 0, 300, 100)
    d.artboards.push(Artboard {
        id: 2,
        name: "Artboard 2".into(),
        rect: Rect::new(200.0, 0.0, 300.0, 100.0),
        locked: false,
        show_center_mark: false,
        show_cross_hairs: false,
        show_video_safe_areas: false,
        video_ruler_pixel_aspect_ratio: 1.0,
        color: None,
    });

    // 1. Artwork whose center is outside all boards, but overlaps Artboard 0 most:
    // Bounds: (50, -100, 150, 50) -> Center: (100, -25) which is outside all artboards.
    // Overlap with Board 0: 50x50 = 2500; Board 1: 0.
    let b_center_outside = Rect::new(50.0, -100.0, 150.0, 50.0);
    assert_eq!(d.artboard_for_bounds(b_center_outside), Some(0));

    // 2. Equal overlap on two boards (defined tie-break: the lower index, board 0):
    // Bounds: (50, -50, 250, 50) -> Center: (150, 0) outside all boards.
    // Overlap with Board 0: (50..100) x (0..50) = 2500; Board 1: (200..250) x (0..50) = 2500.
    let b_equal_overlap = Rect::new(50.0, -50.0, 250.0, 50.0);
    assert_eq!(d.artboard_for_bounds(b_equal_overlap), Some(0), "equal overlap picks lower index");

    // 3. No overlap at all: returns None.
    let b_no_overlap = Rect::new(500.0, 500.0, 600.0, 600.0);
    assert_eq!(d.artboard_for_bounds(b_no_overlap), None);

    // 4. Rotated object bounds (bounds, not the inner geometry):
    // A 40x40 square rotated 45 deg centered at (50, 50) has bounding box ~56.57x56.57.
    let b_rotated_bounds = Rect::new(50.0 - 28.28, 50.0 - 28.28, 50.0 + 28.28, 50.0 + 28.28);
    assert_eq!(d.artboard_for_bounds(b_rotated_bounds), Some(0));

    // 5. Zero-size bounds (a point):
    // On board 0:
    let b_zero_on_board = Rect::new(50.0, 50.0, 50.0, 50.0);
    assert_eq!(d.artboard_for_bounds(b_zero_on_board), Some(0));
    // Outside all boards:
    let b_zero_outside = Rect::new(150.0, 50.0, 150.0, 50.0);
    assert_eq!(d.artboard_for_bounds(b_zero_outside), None);

    // 6. Straight lines whose center is outside every board: owned by the board they lie on most,
    // as a rectangle of any thickness would be (they have no area to compare).
    // Horizontal, from x = -100 to 50 on board 0: center (-25, 50).
    assert_eq!(d.artboard_for_bounds(Rect::new(-100.0, 50.0, 50.0, 50.0)), Some(0));
    assert_eq!(d.artboard_for_bounds(Rect::new(-100.0, 50.0, 50.0, 50.1)), Some(0), "the same as a thin rectangle");
    // Vertical, from y = -100 to 50 on board 1: center (250, -25).
    assert_eq!(d.artboard_for_bounds(Rect::new(250.0, -100.0, 250.0, 50.0)), Some(1));
    // Below both boards.
    assert_eq!(d.artboard_for_bounds(Rect::new(80.0, 120.0, 260.0, 120.0)), None);
    // Across the gap between the boards, 20 pt on board 0 and 60 on board 1.
    assert_eq!(d.artboard_for_bounds(Rect::new(80.0, 50.0, 260.0, 50.0)), Some(1), "center (170, 50) in the gap: board 1 holds more of it");
    // Only touching a board's corner from outside: no overlap.
    assert_eq!(d.artboard_for_bounds(Rect::new(-100.0, 0.0, 0.0, 0.0)), None);
}

/// The art on a locked artboard (each object of a layer whose bounds overlap it, a group as one
/// object) can't be edited, and Select All, a click and a marquee pass over it; art flush with
/// the artboard from outside isn't on it, a layer is on no artboard, and new art isn't on one.
/// Unlocking the artboard gives the art back.
#[test]
fn art_on_a_locked_artboard_cannot_be_selected_or_edited() {
    use crate::hit::{HitOptions, hit_test, marquee, objects_at};
    let (mut d, a, b) = doc_with_rects();
    let layer = d.layers[0].id;
    d.artboards.push(Artboard {
        id: 2,
        name: "Artboard 2".into(),
        rect: Rect::new(700.0, 0.0, 900.0, 200.0),
        locked: false,
        show_center_mark: false,
        show_cross_hairs: false,
        show_video_safe_areas: false,
        video_ruler_pixel_aspect_ratio: 1.0,
        color: None,
        guide_label: None,
    });
    let rect = |d: &mut Document, r: Rect, appearance: Appearance| {
        let id = d.alloc_id();
        d.insert(Some(layer), usize::MAX, Node::path(id, shapes::rectangle(r), appearance)).unwrap();
        id
    };
    let c = rect(&mut d, Rect::new(710.0, 10.0, 730.0, 30.0), Appearance::default_art());
    // A group with one member on the artboard and one right of it.
    let (g, m1, m2) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    let members = [(m1, 750.0), (m2, 950.0)]
        .map(|(id, x)| Arc::new(Node::path(id, shapes::rectangle(Rect::new(x, 50.0, x + 10.0, 60.0)), Appearance::default_art())));
    d.insert(Some(layer), usize::MAX, Node::group(g, members.to_vec())).unwrap();
    // Unstroked and flush with the artboard's left edge, from outside.
    let e = rect(&mut d, Rect::new(680.0, 10.0, 700.0, 30.0), Appearance::default());
    let opt = HitOptions::default();
    let top = |d: &Document, x, y| hit_test(d, Point::new(x, y), opt).map(|h| h.top_object(None));
    let all = Rect::new(-10.0, -10.0, 1000.0, 800.0);
    assert_eq!(top(&d, 720.0, 20.0), Some(c));
    d.artboards[1].locked = true;
    for id in [c, g, m1, m2] {
        assert!(d.on_locked_artboard(id) && !d.is_editable(id), "{id:?} is on the locked artboard");
    }
    for id in [a, b, e, layer] {
        assert!(!d.on_locked_artboard(id) && d.is_editable(id), "{id:?} isn't");
    }
    assert_eq!(d.selectable_art(), [a, b, e]);
    assert_eq!(top(&d, 720.0, 20.0), None, "a click passes over c");
    assert_eq!(top(&d, 955.0, 55.0), None, "and over the group's member off the artboard");
    let isolated = HitOptions { scope: Some(g), ..opt };
    assert!(hit_test(&d, Point::new(755.0, 55.0), isolated).is_none(), "and inside the group isolated before the lock");
    assert!(objects_at(&d, Point::new(720.0, 20.0), opt, None).is_empty());
    assert_eq!(marquee(&d, all, None, false), [a, b, e]);
    assert_eq!(marquee(&d, all, None, true), [a, b, e], "the Direct Selection tool's marquee too");
    // Deselected in one pass, with the key object and the anchors.
    let mut sel = Selection::default();
    sel.set([a, c]);
    sel.key = Some(c);
    sel.anchors.insert(c, [(0, 0)].into());
    sel.deselect_on_locked_artboards(&d);
    assert_eq!((sel.objects.clone(), sel.key, sel.anchors.is_empty()), (vec![a], None, true));
    // A targeted layer stays targeted, with its art off the locked artboard selected.
    sel.set_target(&d, layer);
    sel.deselect_on_locked_artboards(&d);
    assert_eq!((sel.objects.clone(), sel.target), (vec![a, b, e], Some(layer)));
    // Shift-clicking the layer's target circle removes its art off the locked artboard, and
    // a second Shift-click adds it again.
    sel.toggle_target(&d, layer);
    sel.deselect_on_locked_artboards(&d);
    assert_eq!((sel.objects.clone(), sel.target), (vec![], None));
    sel.toggle_target(&d, layer);
    sel.deselect_on_locked_artboards(&d);
    assert_eq!(sel.objects, [a, b, e]);
    // A sublayer with art on both artboards: its art off the locked artboard can still be clicked.
    let (sub, s1, s2) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
    d.insert(Some(layer), usize::MAX, Node::layer(sub, "Sub", LayerColor::Preset(2))).unwrap();
    for (id, x, y) in [(s1, 300.0, 300.0), (s2, 800.0, 100.0)] {
        d.insert(Some(sub), usize::MAX, Node::path(id, shapes::rectangle(Rect::new(x, y, x + 10.0, y + 10.0)), Appearance::default_art())).unwrap();
    }
    assert_eq!((top(&d, 305.0, 305.0), top(&d, 805.0, 105.0)), (Some(s1), None));
    assert!(!d.on_locked_artboard(sub));
    let mut both = Selection::default();
    both.set([s1, s2]);
    both.deselect_on_locked_artboards(&d);
    assert_eq!(both.objects, [s1]);
    // New art on it is editable and can be clicked while it is new art.
    d.new_art.set(Arc::new([c].into()));
    assert!(!d.on_locked_artboard(c) && d.is_editable(c), "new art");
    assert_eq!(top(&d, 720.0, 20.0), Some(c));
    d.new_art.set(Default::default());
    d.artboards[1].locked = false;
    assert_eq!(d.selectable_art(), [a, b, c, g, e, s1, s2]);
    assert_eq!(top(&d, 955.0, 55.0), Some(g));
    assert_eq!(marquee(&d, all, None, false), [a, b, c, g, e, s1, s2]);
}

#[test]
fn art_on_artboard_measures_a_symbol_instance_by_its_art() {
    let mut d = Document::new(612.0, 792.0);
    let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
    d.symbols.push(Symbol { dynamic: false, name: "Box".into(), art: Arc::new(art) });
    let (layer, id) = (d.layers[0].id, d.alloc_id());
    d.insert(Some(layer), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Box".into(), xf: Affine::translate((2.0, 2.0)) })).unwrap();
    // The art, (2, 2) to (12, 12), lies inside; the 20 × 20 square, (-8, -8) to (12, 12), doesn't.
    assert_eq!(d.art_on_artboard(Rect::new(0.0, 0.0, 15.0, 15.0), false), vec![id]);
}

#[test]
fn artboard_at_counts_the_right_and_bottom_edges() {
    // Artboard 0 is (0, 0) to (100, 100), artboard 1 is to its right and artboard 2 below it.
    let mut d = Document::new(100.0, 100.0);
    for rect in [Rect::new(100.0, 0.0, 200.0, 100.0), Rect::new(0.0, 100.0, 100.0, 200.0)] {
        let mut a = d.artboards[0].clone();
        a.rect = rect;
        d.artboards.push(a);
    }
    let held = |d: &Document, x: f64, y: f64| (d.artboard_at(Point::new(x, y)), d.artboard_holding(Point::new(x, y), false));
    // Each artboard's own right and bottom edges, edges with roundoff, and points beyond them.
    assert_eq!(held(&d, 200.0, 50.0), (Some(1), Some(1)), "right edge");
    assert_eq!(held(&d, 150.0, 100.0), (Some(1), Some(1)), "bottom edge");
    assert_eq!(held(&d, 200.0, 100.0), (Some(1), Some(1)), "bottom right corner");
    assert_eq!(held(&d, 100.0, 150.0), (Some(2), Some(2)), "right edge");
    assert_eq!(held(&d, 50.0, 200.0 + 1e-9), (Some(2), Some(2)), "roundoff past the bottom edge");
    assert_eq!(held(&d, -1e-9, 50.0), (Some(0), Some(0)), "roundoff past the left edge");
    assert_eq!(held(&d, 200.001, 50.0), (None, None));
    assert_eq!(held(&d, 50.0, 200.001), (None, None));
    // On an edge two artboards share, the artboard to the right of it or below it holds the
    // point, in either list order, except at the end of the edge that lies on that artboard's
    // bottom or right edge.
    assert_eq!(held(&d, 100.0, 50.0), (Some(1), Some(1)));
    assert_eq!(held(&d, 50.0, 100.0), (Some(2), Some(2)));
    // The bottom edges of artboards 0 and 1 meet at (100, 100), which is also artboard 2's top
    // right corner: the first holds the point, or the last when `topmost`.
    assert_eq!(held(&d, 100.0, 100.0), (Some(2), Some(0)), "shared corner");
    d.artboards.swap(0, 1);
    assert_eq!(held(&d, 100.0, 50.0), (Some(0), Some(0)));
    // Where two artboards overlap, the topmost holds the point, or the first one when not
    // `topmost`.
    let mut inner = d.artboards[1].clone();
    inner.rect = Rect::new(10.0, 10.0, 90.0, 90.0);
    d.artboards.push(inner);
    assert_eq!(held(&d, 50.0, 50.0), (Some(3), Some(1)));
    assert_eq!(held(&d, 90.0, 50.0), (Some(1), Some(1)), "inside artboard 1, on the right edge of artboard 3");
}

#[test]
fn find_and_paths() {
    let (d, a, b) = doc_with_rects();
    assert_eq!(d.index_path(b), Some(vec![0, 1]));
    assert_eq!(d.parent_of(a), Some(d.layers[0].id));
    assert_eq!(d.layer_of(b), Some(d.layers[0].id));
    assert!(d.node(NodeId(999)).is_none());
    assert_eq!(d.node_count(), 3);
}

#[test]
fn copy_on_write_shares_untouched() {
    let (d, a, b) = doc_with_rects();
    let mut e = d.clone();
    e.node_mut(a).unwrap().name = Some("A".into());
    // b's Arc is shared between both versions; a's is not.
    let lb = &d.layers[0].children().unwrap()[1];
    let eb = &e.layers[0].children().unwrap()[1];
    assert!(Arc::ptr_eq(lb, eb));
    assert_eq!(d.node(a).unwrap().name, None);
    assert_eq!(e.node(a).unwrap().name.as_deref(), Some("A"));
    let _ = b;
}

#[test]
fn move_and_remove() {
    let (mut d, a, b) = doc_with_rects();
    let layer = d.layers[0].id;
    d.move_node(a, Some(layer), usize::MAX).unwrap();
    assert_eq!(d.index_path(a), Some(vec![0, 1]));
    assert_eq!(d.index_path(b), Some(vec![0, 0]));
    let r = d.remove(a).unwrap();
    assert_eq!(r.id, a);
    assert!(d.node(a).is_none());
    assert!(d.move_node(layer, Some(layer), 0).is_err());
}

#[test]
fn layers_and_names() {
    let mut d = Document::new(100.0, 100.0);
    let l2 = d.add_layer(None);
    assert_eq!(d.node(l2).unwrap().display_name(), "Layer 2");
    assert_eq!(d.default_layer(), Some(l2));
    assert_eq!(d.layer_color(l2), LAYER_COLORS[1].1);
}

/// #1718: a file whose ids or counters are at their type's largest value still hands out ids
/// nothing in it uses (and never 0), instead of overflowing.
#[test]
fn exhausted_ids_in_a_file_dont_run_out() {
    let (d, a, b) = doc_with_rects();
    let mut v = serde_json::to_value(&d).unwrap();
    v["next_id"] = serde_json::json!(u64::MAX);
    // Object `a` gets the largest id there is.
    fn renumber(v: &mut serde_json::Value, from: u64) {
        match v {
            serde_json::Value::Object(o) => {
                if o.get("id").and_then(serde_json::Value::as_u64) == Some(from) {
                    o.insert("id".into(), serde_json::json!(u64::MAX));
                }
                o.values_mut().for_each(|c| renumber(c, from));
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(|c| renumber(c, from)),
            _ => {}
        }
    }
    renumber(&mut v["layers"], a.0);
    let mut d: Document = serde_json::from_value(v).unwrap();
    assert!(d.node(NodeId(u64::MAX)).is_some(), "the file's largest id is kept");
    d.fix_next_id();
    let fresh: Vec<NodeId> = (0..3).map(|_| d.alloc_id()).collect();
    assert!(fresh.iter().all(|id| d.node(*id).is_none() && id.0 > b.0 && id.0 < MAX_NEXT_ID), "{fresh:?}");
    assert!(fresh[0] != fresh[1] && fresh[1] != fresh[2]);
    // A counter left at the end by other means stays there rather than wrapping to 0.
    d.next_id = u64::MAX;
    assert_eq!((d.alloc_id(), d.peek_next_id()), (NodeId(u64::MAX), u64::MAX));
    d.reserve_ids(u64::MAX);
    // Artboards and graphic styles: one past the largest, else the smallest id free.
    assert_eq!(free_id([1, 2, 7].into_iter()), 8);
    assert_eq!(free_id([u32::MAX, 0, 1, 2, 4].into_iter()), 3);
    assert_eq!(free_id([u32::MAX].into_iter()), 1);
    d.artboards[0].id = u32::MAX;
    assert_eq!(d.next_artboard_id(), 1);
}

#[test]
fn serde_roundtrip() {
    let (d, _, _) = doc_with_rects();
    let s = serde_json::to_string(&d).unwrap();
    let back: Document = serde_json::from_str(&s).unwrap();
    assert_eq!(back.node_count(), d.node_count());
    assert_eq!(back.peek_next_id(), d.peek_next_id());
}

#[test]
fn reid_gives_fresh_ids() {
    let (mut d, a, b) = doc_with_rects();
    let g = d.alloc_id();
    let na = (*d.remove(a).unwrap()).clone();
    let nb = (*d.remove(b).unwrap()).clone();
    let group = Node::group(g, vec![Arc::new(na), Arc::new(nb)]);
    let copy = d.reid(&group);
    assert_ne!(copy.id, g);
    let ids: Vec<NodeId> = copy.children().unwrap().iter().map(|c| c.id).collect();
    assert!(!ids.contains(&a) && !ids.contains(&b));
}

#[test]
fn setup_round_trips_and_old_files_load() {
    let (mut d, _, _) = doc_with_rects();
    // A document without a setup writes no `setup` key (old readers and files stay unchanged).
    assert!(!serde_json::to_string(&d).unwrap().contains("\"setup\""));
    d.setup.bleed = [9.0, 9.0, 0.0, 18.0];
    d.setup.grid_size = GridSize::Large;
    d.setup.typographers_quotes = false;
    d.setup.quotes = setup::language_quotes("German").unwrap();
    d.setup.flattener_preset = Some("High Resolution".into());
    d.setup.background = Background::White;
    d.setup.export_text = ExportText::Appearance;
    let back: Document = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
    assert_eq!(back.setup, d.setup);
    // Files written before Document Setup existed load with the defaults.
    let mut v = serde_json::to_value(&d).unwrap();
    v.as_object_mut().unwrap().remove("setup");
    let old: Document = serde_json::from_value(v).unwrap();
    assert_eq!(old.setup, DocSetup::default());
}

#[test]
fn char_position_round_trips() {
    let st =
        CharStyle { position: CharPosition::Subscript(ScriptMetrics { size: 50.0, position: 20.0 }), small_caps: Some(75.0), ..CharStyle::default() };
    let v = serde_json::to_value(&st).unwrap();
    assert_eq!(v["position"], serde_json::json!({"kind": "subscript", "size": 50.0, "position": 20.0}));
    assert_eq!(serde_json::from_value::<CharStyle>(v).unwrap(), st);
    let plain = serde_json::to_value(CharStyle::default()).unwrap();
    assert!(plain.get("position").is_none() && plain.get("small_caps").is_none());
    let (scale, shift) = CharPosition::Subscript(ScriptMetrics::DEFAULT).scale_shift(10.0);
    assert!((scale - 0.583).abs() < 1e-9 && (shift + 3.33).abs() < 1e-9);
}

#[test]
fn optical_kerning_round_trips_and_old_style_defaults_to_auto() {
    let st = CharStyle { optical_kerning: true, kerning: Some(120.0), ..CharStyle::default() };
    let value = serde_json::to_value(&st).unwrap();
    assert_eq!(value["opticalKerning"], true);
    assert_eq!(serde_json::from_value::<CharStyle>(value.clone()).unwrap(), st);
    let mut old = value;
    old.as_object_mut().unwrap().remove("opticalKerning");
    assert!(!serde_json::from_value::<CharStyle>(old).unwrap().optical_kerning);
    assert!(serde_json::to_value(CharStyle::default()).unwrap().get("opticalKerning").is_none());
}

#[test]
fn units() {
    assert_eq!(Unit::Inches.parse("1"), Some(72.0));
    assert_eq!(Unit::Points.parse("1in"), Some(72.0));
    assert_eq!(Unit::Points.parse("10 mm").map(|v| (v * 1000.0).round()), Some(28346.0));
    assert_eq!(Unit::Points.parse("2p6"), Some(30.0));
    assert_eq!(Unit::Points.parse("10+5"), Some(15.0));
    assert_eq!(Unit::Points.parse("100/4"), Some(25.0));
    assert_eq!(Unit::Points.parse("-5"), Some(-5.0));
    assert_eq!(Unit::Points.parse("abc"), None);
    assert_eq!(Unit::Points.format(12.5), "12.5 pt");
    assert_eq!(Unit::Inches.format(144.0), "2 in");
}

#[test]
fn negative_picas_and_points_sign_applies_to_whole_value() {
    assert_eq!(Unit::Points.parse("-2p6"), Some(-30.0));
    assert_eq!(Unit::Points.parse("-0p6"), Some(-6.0));
    assert_eq!(Unit::Points.parse("-2p"), Some(-24.0));
    assert_eq!(Unit::Points.parse("2p6"), Some(30.0));
}

#[test]
fn pica_form_rejects_exponent_notation() {
    for s in ["1e5p", "infp", "nanp", "infp3", "1p1e2"] {
        assert_eq!(Unit::Points.parse(s), None, "{s}");
    }
}

/// Symbol editing of symbol "S" on `layer`.
fn symbol_edit_on(layer: NodeId) -> SymbolEdit {
    SymbolEdit {
        symbol: "S".into(),
        layer,
        instance: None,
        xf: Affine::IDENTITY,
        group: false,
        changed: false,
        back_isolation: None,
        back_layer: None,
    }
}

/// The opacity-mask and symbol editing layers are left out of the saved document wherever they
/// are, also when `layer.move` made them sublayers of another layer.
#[test]
fn edit_mode_layers_are_dropped_as_sublayers_too() {
    let mut d = Document::new(612.0, 792.0);
    let host = d.layers[0].id;
    let mask = d.add_layer(Some("Mask"));
    let symbol = d.add_layer(Some("Symbol"));
    d.move_node(mask, Some(host), 0).unwrap();
    d.move_node(symbol, Some(host), usize::MAX).unwrap();
    let object = d.alloc_id();
    d.mask_edit = Some(MaskEdit { object, layer: mask });
    d.symbol_edit = Some(symbol_edit_on(symbol));
    let saved = d.without_edit_modes();
    assert!(saved.node(mask).is_none(), "the mask editing layer is left out");
    assert!(saved.node(symbol).is_none(), "the symbol editing layer is left out");
    assert!(saved.node(host).is_some_and(|l| l.children().is_some_and(|c| c.is_empty())));
    assert!(saved.mask_edit.is_none() && saved.symbol_edit.is_none());
}

/// Symbol editing is never read from a file, so a layer that file data names as the editing layer
/// loads and stays in the document, and saving does not write the value again.
#[test]
fn symbol_edit_is_not_read_from_a_file() {
    let mut d = Document::new(612.0, 792.0);
    let layer = d.layers[0].id;
    let mut v = serde_json::to_value(&d).unwrap();
    v["symbol_edit"] = serde_json::to_value(symbol_edit_on(layer)).unwrap();
    d = serde_json::from_value(v).unwrap();
    assert_eq!(d.symbol_edit, None);
    assert!(d.extra.is_empty(), "not kept as an unknown key");
    d.drop_edit_modes();
    assert!(d.node(layer).is_some(), "the layer is kept");
    assert!(serde_json::to_value(&d).unwrap().get("symbol_edit").is_none(), "not saved again");
}

/// An opacity-mask editing id in file data that names art inside a layer, not a layer, leaves the
/// art in place.
#[test]
fn mask_edit_naming_art_leaves_it() {
    let (mut d, a, b) = doc_with_rects();
    d.mask_edit = Some(MaskEdit { object: b, layer: a });
    d.drop_edit_modes();
    assert!(d.node(a).is_some() && d.node(b).is_some());
    assert_eq!(d.mask_edit, None);
}
