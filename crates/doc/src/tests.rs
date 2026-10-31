use super::*;
use vectorcraft_geom::shapes;

mod lookups;
mod slices;
mod units;

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

fn doc_with_rects() -> (Document, NodeId, NodeId) {
    let mut d = Document::new(612.0, 792.0);
    let layer = d.layers[0].id;
    let a = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art())).unwrap();
    let b = d.alloc_id();
    d.insert(Some(layer), usize::MAX, Node::path(b, shapes::rectangle(Rect::new(20.0, 0.0, 30.0, 10.0)), Appearance::default_art())).unwrap();
    (d, a, b)
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
    d.symbols.push(Symbol { name: "Box".into(), art: Arc::new(art) });
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
