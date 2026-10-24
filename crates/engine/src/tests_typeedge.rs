//! #3784: exports that crop to artboards say when type runs over an artboard's edge, where the
//! part past it isn't drawn.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use super::*;

const LONG: &str = "This line is much too long for a 300 pt wide artboard";

/// A 300 × 200 pt document.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// Point type at (x, y) in 24 pt.
fn point(s: &mut Session, x: f64, y: f64, text: &str) -> NodeId {
    id_of(&s.execute("text.create", &json!({"x": x, "y": y, "text": text, "size": 24})).unwrap())
}

/// The warnings of `document.export {format, …options}` about type over an artboard's edge.
fn edge_warnings(s: &mut Session, format: &str, options: Value) -> Vec<String> {
    let mut p = json!({ "format": format });
    if let (Some(p), Some(o)) = (p.as_object_mut(), options.as_object()) {
        p.extend(o.clone());
    }
    let v = s.execute("document.export", &p).unwrap();
    v["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).filter(|w| w.contains("edge")).map(str::to_string).collect()
}

/// The exports that crop to the artboards, with the options that pick the first one.
fn cropping() -> [(&'static str, Value); 9] {
    [
        ("pdf", json!({})),
        ("png", json!({})),
        ("jpg", json!({})),
        ("tiff", json!({})),
        ("emf", json!({})),
        ("wmf", json!({})),
        ("svg", json!({"range": "1"})),
        ("eps", json!({"useArtboards": true, "range": "1"})),
        ("pdf", json!({"range": "1"})),
    ]
}

fn one(id: NodeId) -> String {
    format!("1 text object runs over the edge of its artboard: the part past the edge isn't drawn in the export (id {})", id.0)
}

#[test]
fn type_over_the_artboard_edge_is_named_by_every_export_that_crops() {
    let mut s = session();
    let id = point(&mut s, 20.0, 100.0, LONG);
    for (format, options) in cropping() {
        assert_eq!(edge_warnings(&mut s, format, options.clone()), [one(id)], "{format} {options}");
    }
}

#[test]
fn exports_that_hold_all_the_art_say_nothing() {
    let mut s = session();
    point(&mut s, 20.0, 100.0, LONG);
    // An SVG of the whole document, an EPS of the art's bounds and a .ai file (its editing data)
    // hold the whole line.
    for format in ["svg", "eps", "ai"] {
        assert_eq!(edge_warnings(&mut s, format, json!({})), Vec::<String>::new(), "{format}");
    }
}

#[test]
fn type_inside_or_wholly_outside_the_artboard_is_left_alone() {
    let mut s = session();
    point(&mut s, 20.0, 100.0, "Fits");
    // A note on the pasteboard, past the right edge.
    point(&mut s, 400.0, 100.0, LONG);
    for (format, options) in cropping() {
        assert_eq!(edge_warnings(&mut s, format, options.clone()), Vec::<String>::new(), "{format} {options}");
    }
}

#[test]
fn hidden_type_and_type_on_a_template_layer_are_left_alone() {
    let mut s = session();
    let id = point(&mut s, 20.0, 100.0, LONG);
    s.execute("object.setProps", &json!({"ids": [id.0], "visible": false})).unwrap();
    assert_eq!(edge_warnings(&mut s, "pdf", json!({})), Vec::<String>::new());
    s.execute("object.setProps", &json!({"ids": [id.0], "visible": true})).unwrap();
    let layer = s.doc().unwrap().doc.layers[0].id;
    s.execute("layer.setProps", &json!({"id": layer.0, "template": true})).unwrap();
    assert_eq!(edge_warnings(&mut s, "png", json!({})), Vec::<String>::new());
}

#[test]
fn type_on_a_layer_that_does_not_print_is_named_only_where_it_is_drawn() {
    let mut s = session();
    point(&mut s, 20.0, 100.0, LONG);
    let layer = s.doc().unwrap().doc.layers[0].id;
    s.execute("layer.setProps", &json!({"id": layer.0, "printable": false})).unwrap();
    assert_eq!(edge_warnings(&mut s, "pdf", json!({})), Vec::<String>::new());
    assert_eq!(edge_warnings(&mut s, "pdf", json!({"includeNonPrinting": true})).len(), 1);
    assert_eq!(edge_warnings(&mut s, "png", json!({})).len(), 1);
}

#[test]
fn type_cut_by_its_own_clipping_path_inside_the_artboard_is_left_alone() {
    let mut s = session();
    let text = point(&mut s, 20.0, 100.0, LONG);
    let mask = id_of(&s.execute("shape.rectangle", &json!({"x": 10, "y": 50, "width": 200, "height": 100})).unwrap());
    s.execute("select.set", &json!({"ids": [text.0, mask.0]})).unwrap();
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    for (format, options) in cropping() {
        assert_eq!(edge_warnings(&mut s, format, options.clone()), Vec::<String>::new(), "{format} {options}");
    }
    // A clipping path that itself reaches past the edge lets the type run over it.
    s.execute("object.move", &json!({"ids": [mask.0], "dx": 150, "dy": 0})).unwrap();
    assert_eq!(edge_warnings(&mut s, "pdf", json!({})), [one(text)]);
}

#[test]
fn area_type_is_judged_by_its_glyphs_not_its_frame() {
    let mut s = session();
    // The frame reaches past the right edge, the words stay well inside it.
    let v = s.execute("text.create", &json!({"x": 20, "y": 40, "text": "Short", "size": 24, "area": {"width": 400, "height": 60}})).unwrap();
    assert!(v.get("overflow").is_none(), "{v}");
    assert_eq!(edge_warnings(&mut s, "pdf", json!({})), Vec::<String>::new());
    // A word longer than the artboard is wide does cross it.
    let id = id_of(&v);
    s.execute("text.setText", &json!({"id": v["id"], "text": "Overlongwordthatrunsfarpasttheedge"})).unwrap();
    assert_eq!(edge_warnings(&mut s, "pdf", json!({})), [one(id)]);
}

#[test]
fn type_flush_with_the_edges_is_inside() {
    let mut s = session();
    // The glyphs' box, not the line's: a cap height under the top edge, a baseline on the bottom
    // one (nothing below it in "HI").
    let top = point(&mut s, 0.0, 18.0, "HI");
    let bottom = point(&mut s, 0.0, 200.0, "HI");
    let ink = |s: &Session, id: NodeId| {
        crate::cmd::pathops::text_outline(match &s.doc().unwrap().doc.node(id).unwrap().kind {
            vectorcraft_doc::NodeKind::Text(t) => t,
            k => panic!("not text: {k:?}"),
        })
    };
    let (t, b) = (ink(&s, top).bounds().unwrap(), ink(&s, bottom).bounds().unwrap());
    assert!(t.y0 >= 0.0 && b.y1 <= 200.0, "{t:?} {b:?}");
    assert_eq!(edge_warnings(&mut s, "pdf", json!({})), Vec::<String>::new());
}

#[test]
fn type_across_two_artboards_is_named_on_both_and_once() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200, "artboards": 2})).unwrap();
    let (a, b) = {
        let d = &s.doc().unwrap().doc;
        (d.artboards[0].rect, d.artboards[1].rect)
    };
    assert!(b.x0 > a.x1, "side by side: {a:?} {b:?}");
    let v = s.execute("text.create", &json!({"x": a.x1 - 40.0, "y": 100, "text": LONG, "size": 24})).unwrap();
    let id = id_of(&v);
    let right = v["bounds"]["x"].as_f64().unwrap() + v["bounds"]["width"].as_f64().unwrap();
    assert!(right > b.x0 + 10.0, "reaches into the second artboard: {v}");
    // Both pages cut it: one warning naming it once.
    assert_eq!(edge_warnings(&mut s, "pdf", json!({})), [one(id)]);
    assert_eq!(edge_warnings(&mut s, "pdf", json!({"range": "1"})), [one(id)]);
    assert_eq!(edge_warnings(&mut s, "pdf", json!({"range": "2"})), [one(id)]);
}

#[test]
fn many_objects_are_counted_past_twenty() {
    let mut s = session();
    let ids: Vec<NodeId> = (0..23).map(|i| point(&mut s, 20.0, 20.0 + 8.0 * f64::from(i), LONG)).collect();
    let w = edge_warnings(&mut s, "pdf", json!({}));
    let named = ids[..20].iter().map(|i| i.0.to_string()).collect::<Vec<_>>().join(", ");
    assert_eq!(
        w,
        [format!(
            "23 text objects run over the edges of their artboards: the parts past the edges aren't drawn in the export (ids {named} and 3 more)"
        )]
    );
}

#[test]
fn two_objects_read_in_the_plural() {
    let mut s = session();
    let a = point(&mut s, 20.0, 60.0, LONG);
    let b = point(&mut s, -100.0, 140.0, LONG);
    assert_eq!(
        edge_warnings(&mut s, "png", json!({})),
        [format!("2 text objects run over the edges of their artboards: the parts past the edges aren't drawn in the export (ids {}, {})", a.0, b.0)]
    );
}

#[test]
fn type_in_a_group_with_an_effect_of_its_own_is_named() {
    let mut s = session();
    let id = point(&mut s, 20.0, 100.0, LONG);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    // The group prints where its effect paints, so its members are walked without the areas.
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow"})).unwrap();
    for (format, options) in cropping() {
        assert_eq!(edge_warnings(&mut s, format, options.clone()), [one(id)], "{format} {options}");
    }
}

#[test]
fn type_inside_one_artboard_and_over_the_edge_of_an_overlapping_one_is_named() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 150, "y": 0, "width": 300, "height": 200})).unwrap();
    let v = s.execute("text.create", &json!({"x": 20, "y": 100, "text": "Twenty characters ok", "size": 24})).unwrap();
    let id = id_of(&v);
    let right = v["bounds"]["x"].as_f64().unwrap() + v["bounds"]["width"].as_f64().unwrap();
    assert!(right > 160.0 && right < 290.0, "inside the first artboard, into the second: {v}");
    // The second page cuts it, though the first holds all of it.
    assert_eq!(edge_warnings(&mut s, "pdf", json!({})), [one(id)]);
    assert_eq!(edge_warnings(&mut s, "pdf", json!({"range": "2"})), [one(id)]);
    assert_eq!(edge_warnings(&mut s, "pdf", json!({"range": "1"})), Vec::<String>::new());
}

#[test]
fn type_cut_by_the_clipping_path_of_a_clip_group_with_an_effect_is_left_alone() {
    let mut s = session();
    let text = point(&mut s, 20.0, 100.0, LONG);
    let mask = id_of(&s.execute("shape.rectangle", &json!({"x": 10, "y": 50, "width": 200, "height": 100})).unwrap());
    s.execute("select.set", &json!({"ids": [text.0, mask.0]})).unwrap();
    let group = s.execute("object.clippingMask.make", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [group["id"]]})).unwrap();
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow"})).unwrap();
    for (format, options) in cropping() {
        assert_eq!(edge_warnings(&mut s, format, options.clone()), Vec::<String>::new(), "{format} {options}");
    }
}

#[test]
fn a_drop_shadow_past_the_edge_does_not_widen_the_clipping_path() {
    let mut s = session();
    let text = point(&mut s, 20.0, 100.0, LONG);
    // The clipping path ends 4 pt short of the right edge; the group's shadow paints past it.
    let mask = id_of(&s.execute("shape.rectangle", &json!({"x": 10, "y": 50, "width": 286, "height": 100})).unwrap());
    s.execute("select.set", &json!({"ids": [text.0, mask.0]})).unwrap();
    let group = s.execute("object.clippingMask.make", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [group["id"]]})).unwrap();
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow"})).unwrap();
    let painted = {
        let d = &s.doc().unwrap().doc;
        vectorcraft_render::painted_bounds(d, d.node(id_of(&group)).unwrap()).unwrap()
    };
    assert!(painted.x1 > 300.0, "the shadow paints past the edge: {painted:?}");
    for (format, options) in cropping() {
        assert_eq!(edge_warnings(&mut s, format, options.clone()), Vec::<String>::new(), "{format} {options}");
    }
}

#[test]
fn a_miter_spike_past_the_edge_is_named() {
    let mut s = session();
    let id = point(&mut s, 100.0, 100.0, "k");
    s.execute(
        "text.setRangeStyle",
        &json!({"id": id.0, "stroke": "#000000", "strokeWidth": 6, "strokeOptions": {"join": "miter", "miterLimit": 10}}),
    )
    .unwrap();
    let t = |s: &Session| match &s.doc().unwrap().doc.node(id).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => (**t).clone(),
        k => panic!("not text: {k:?}"),
    };
    // Put the glyph's right side 1 pt inside the right edge with the stroke's even reach.
    let ink = crate::cmd::pathops::text_outline(&t(&s)).bounds().unwrap();
    let dx = 300.0 - t(&s).char_stroke_outset() - 1.0 - ink.x1;
    s.execute("object.move", &json!({"ids": [id.0], "dx": dx, "dy": 0})).unwrap();
    let t = t(&s);
    let outline = crate::cmd::pathops::text_outline(&t);
    let ink = outline.bounds().unwrap();
    assert!(ink.x1 + t.char_stroke_outset() < 300.0, "{ink:?}");
    let stroked = t.runs[0].style.stroke_layer().bounds_on(&outline.to_bezpath(), ink);
    assert!(stroked.x1 > 300.5, "the spike at the arm of the k crosses the edge: {stroked:?}");
    assert_eq!(edge_warnings(&mut s, "pdf", json!({})), [one(id)]);
}

#[test]
fn a_round_joined_stroke_inside_the_edge_is_left_alone() {
    let mut s = session();
    let id = point(&mut s, 100.0, 100.0, "k");
    s.execute(
        "text.setRangeStyle",
        &json!({"id": id.0, "stroke": "#000000", "strokeWidth": 6, "strokeOptions": {"join": "round", "miterLimit": 10}}),
    )
    .unwrap();
    let t = |s: &Session| match &s.doc().unwrap().doc.node(id).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => (**t).clone(),
        k => panic!("not text: {k:?}"),
    };
    // Within the miter limit's reach of the right edge, but no spike to cross it.
    let ink = crate::cmd::pathops::text_outline(&t(&s)).bounds().unwrap();
    let dx = 300.0 - t(&s).char_stroke_outset() - 1.0 - ink.x1;
    s.execute("object.move", &json!({"ids": [id.0], "dx": dx, "dy": 0})).unwrap();
    assert_eq!(edge_warnings(&mut s, "pdf", json!({})), Vec::<String>::new());
}

/// A clipping path whose bounds cross the edge but whose shape doesn't reach past it over the type
/// (a compound path of two rectangles, the second off the artboard beside the line, not over it)
/// shows none of the type past the edge.
#[test]
fn a_compound_clipping_path_is_judged_by_its_shape_not_its_bounds() {
    let mut s = session();
    let text = point(&mut s, 20.0, 100.0, LONG);
    let inside = id_of(&s.execute("shape.rectangle", &json!({"x": 10, "y": 50, "width": 200, "height": 100})).unwrap());
    let away = id_of(&s.execute("shape.rectangle", &json!({"x": 350, "y": 0, "width": 50, "height": 20})).unwrap());
    s.execute("select.set", &json!({"ids": [inside.0, away.0]})).unwrap();
    let compound = id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap());
    let b = s.doc().unwrap().doc.node(compound).unwrap().geometric_bounds().unwrap();
    assert!(b.x1 > 300.0 && b.y1 > 100.0, "its bounds reach past the edge, over the line: {b:?}");
    s.execute("select.set", &json!({"ids": [text.0, compound.0]})).unwrap();
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    for (format, options) in cropping() {
        assert_eq!(edge_warnings(&mut s, format, options.clone()), Vec::<String>::new(), "{format} {options}");
    }
    // Moved so the second rectangle lies over the line across the edge, it shows the type there.
    s.execute("object.move", &json!({"ids": [compound.0], "dx": -60, "dy": 80})).unwrap();
    assert_eq!(edge_warnings(&mut s, "pdf", json!({})), [one(text)]);
}

/// Type whose glyphs and even stroke lie past the left edge, with only a miter spike reaching onto
/// the artboard, is cut by that edge too: what type paints (the cull before the edge check) takes
/// its strokes' full miter reach, so the spike is never culled with it.
#[test]
fn a_miter_spike_alone_reaching_onto_the_artboard_is_named() {
    let mut s = session();
    let id = point(&mut s, 100.0, 100.0, "k");
    s.execute(
        "text.setRangeStyle",
        &json!({"id": id.0, "stroke": "#000000", "strokeWidth": 6, "strokeOptions": {"join": "miter", "miterLimit": 10}}),
    )
    .unwrap();
    let t = |s: &Session| match &s.doc().unwrap().doc.node(id).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => (**t).clone(),
        k => panic!("not text: {k:?}"),
    };
    // The glyph's right side, with the stroke's even reach, 1 pt left of the left edge.
    let ink = crate::cmd::pathops::text_outline(&t(&s)).bounds().unwrap();
    let dx = -t(&s).char_stroke_outset() - 1.0 - ink.x1;
    s.execute("object.move", &json!({"ids": [id.0], "dx": dx, "dy": 0})).unwrap();
    let painted = vectorcraft_render::painted_bounds(&s.doc().unwrap().doc, s.doc().unwrap().doc.node(id).unwrap()).unwrap();
    let t = t(&s);
    let outline = crate::cmd::pathops::text_outline(&t);
    let ink = outline.bounds().unwrap();
    assert!(ink.x1 + t.char_stroke_outset() < 0.0, "the glyph and its even stroke are off the artboard: {ink:?}");
    let stroked = t.runs[0].style.stroke_layer().bounds_on(&outline.to_bezpath(), ink);
    assert!(stroked.x1 > 0.5, "the spike at the arm of the k reaches onto the artboard: {stroked:?}");
    assert!(painted.x1 >= stroked.x1, "the painted bounds hold the spike: {painted:?} {stroked:?}");
    assert_eq!(edge_warnings(&mut s, "pdf", json!({})), [one(id)]);
}

/// A clip group whose clipping path has nothing to clip by (here one turned into a guide, as a
/// file can hold it; a symbol instance is another) hides the type it clips, as the renderer and the
/// PDF and SVG writers do: nothing of it is drawn, so nothing is cut off, even where the clipping
/// path's box crosses the edge.
#[test]
fn type_hidden_by_a_clipping_path_with_nothing_to_clip_by_is_left_alone() {
    let mut s = session();
    let text = point(&mut s, 20.0, 100.0, LONG);
    let mask = id_of(&s.execute("shape.rectangle", &json!({"x": 10, "y": 50, "width": 400, "height": 100})).unwrap());
    s.execute("select.set", &json!({"ids": [text.0, mask.0]})).unwrap();
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    assert_eq!(edge_warnings(&mut s, "pdf", json!({})), [one(text)], "a clipping path past the edge shows the type there");
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    if let vectorcraft_doc::NodeKind::Path { guide, .. } = &mut d.node_mut(mask).unwrap().kind {
        *guide = true;
    }
    let clip = s.doc().unwrap().doc.node(mask).unwrap();
    assert!(clip.geometric_bounds().is_some_and(|b| b.x1 > 300.0));
    assert!(vectorcraft_render::effects::clip_outline(clip).is_none(), "nothing to clip by");
    for (format, options) in cropping() {
        assert_eq!(edge_warnings(&mut s, format, options.clone()), Vec::<String>::new(), "{format} {options}");
    }
}
