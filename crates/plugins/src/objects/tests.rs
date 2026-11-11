use super::*;
use vectorcraft_geom::SubPath;

fn square(x: f64) -> PathData {
    PathData::single(SubPath::polyline(&[Point::new(x, 0.0), Point::new(x + 10.0, 0.0), Point::new(x + 10.0, 10.0), Point::new(x, 10.0)], true))
}

/// A document with paths A, B and C (bottom to top) on its layer, plus a locked path and a guide.
fn doc() -> (Document, Vec<NodeId>, NodeId) {
    let mut d = Document::new(200.0, 200.0);
    let layer = d.layers[0].id;
    let mut ids = vec![];
    for (i, name) in ["A", "B", "C"].into_iter().enumerate() {
        let mut n = Node::path(d.alloc_id(), square(i as f64 * 20.0), Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 1.0));
        n.name = Some(name.into());
        ids.push(d.insert(Some(layer), usize::MAX, n).unwrap());
    }
    let mut locked = Node::path(d.alloc_id(), square(80.0), Appearance::default());
    locked.locked = true;
    d.insert(Some(layer), usize::MAX, locked).unwrap();
    let guide = Node::new(d.alloc_id(), NodeKind::Path { path: square(90.0), rule: FillRule::NonZero, live: None, clipping: false, guide: true });
    d.insert(Some(layer), usize::MAX, guide).unwrap();
    (d, ids, layer)
}

fn names(d: &Document, layer: NodeId) -> Vec<String> {
    d.children(Some(layer)).unwrap().iter().map(|n| n.name.clone().unwrap_or_else(|| "?".into())).collect()
}

/// Admits every effect record as it is (the host's checks are the engine's).
fn keep(fx: &[Effect], _: &Node) -> std::result::Result<Vec<Effect>, String> {
    Ok(fx.to_vec())
}

fn decode_json(v: Value) -> Result<Vec<OutObject>> {
    decode(v.to_string().as_bytes())
}

#[test]
fn targets_are_selected_paths_in_paint_order() {
    let (mut d, ids, layer) = doc();
    assert_eq!(filter_targets(&d, &[ids[2], ids[0]]), [ids[0], ids[2]]);
    // A selected layer gives its paths, without the locked one and the guide.
    assert_eq!(filter_targets(&d, &[layer]), ids);
    d.node_mut(ids[1]).unwrap().visible = false;
    assert_eq!(filter_targets(&d, &[layer]), [ids[0], ids[2]]);
}

/// Objects as plug-ins are given them (type, name, fill rule, fills, strokes, bounds); handed back
/// unchanged, they change nothing, swatch links included.
#[test]
fn objects_round_trip_through_the_json_model() {
    let (mut d, ids, _) = doc();
    let mut g = vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::default());
    g.gradient.stops[0].swatch = Some("White".into());
    d.node_mut(ids[0]).unwrap().appearance.set_stroke(Paint::Gradient(Box::new(g)));
    let input: Value = serde_json::from_slice(&encode_input(&d, &ids)).unwrap();
    let a = &input["objects"][0];
    assert_eq!((a["type"].as_str(), a["name"].as_str(), a["fillRule"].as_str()), (Some("path"), Some("A"), Some("nonzero")));
    assert_eq!(a["fills"][0]["color"], json!({"model": "rgb", "r": 1.0, "g": 0.0, "b": 0.0}));
    assert_eq!(a["strokes"][0]["paint"]["type"], "gradient");
    assert_eq!(a["bounds"], json!([0.0, 0.0, 10.0, 10.0]));
    // Echoed back unchanged, nothing changes (swatch links included).
    let before = d.clone();
    let out = decode(&serde_json::to_vec(&input).unwrap()).unwrap();
    assert_eq!(apply_output(&mut d, &ids, out, None, &keep).unwrap(), ids);
    assert_eq!(d, before);
}

/// A path handed back as it was stays the live shape it was (a rectangle keeps its corner
/// widgets); a path that changed is a plain path.
#[test]
fn an_unchanged_path_stays_a_live_shape() {
    let (mut d, ids, _) = doc();
    let a = ids[0];
    let live = vectorcraft_doc::LiveShape::Rectangle {
        w: 10.0,
        h: 10.0,
        radii: [0.0; 4],
        radii_relative: false,
        kinds: Default::default(),
        xf: vectorcraft_geom::Affine::IDENTITY,
    };
    if let Some(NodeKind::Path { live: l, .. }) = d.node_mut(a).map(|n| &mut n.kind) {
        *l = Some(Box::new(live));
    }
    let input: Value = serde_json::from_slice(&encode_input(&d, &[a])).unwrap();
    let before = d.clone();
    apply_output(&mut d, &[a], decode(&serde_json::to_vec(&input).unwrap()).unwrap(), None, &keep).unwrap();
    assert_eq!(d, before, "still a live rectangle");
    apply_output(&mut d, &[a], decode_json(json!({"objects": [{"id": a.0, "path": square(1.0)}]})).unwrap(), None, &keep).unwrap();
    assert!(matches!(&d.node(a).unwrap().kind, NodeKind::Path { live: None, .. }), "moved: a plain path");
}

/// An output updates the input objects it lists, deletes the ones it leaves out, and adds new
/// objects where it lists them.
#[test]
fn outputs_update_add_and_delete_in_place() {
    let (mut d, ids, layer) = doc();
    let [a, b, c] = [ids[0].0, ids[1].0, ids[2].0];
    let new = |name: &str| json!({"name": name, "path": square(5.0), "fills": [{"type": "solid", "color": {"model": "gray", "k": 0.5}}]});
    // A recoloured, B dropped, C kept; new objects around them.
    let out = json!({"objects": [new("n1"), {"id": a, "fills": [null, {"type": "solid", "color": {"model": "rgb", "r": 5, "g": 0, "b": 0}}]}, new("n2"), {"id": c}, new("n3")]});
    let sel = apply_output(&mut d, &ids, decode_json(out).unwrap(), None, &keep).unwrap();
    assert_eq!(names(&d, layer), ["n1", "A", "n2", "C", "n3", "?", "?"]);
    assert_eq!(sel.len(), 5);
    assert!(d.node(NodeId(b)).is_none());
    let na = d.node(NodeId(a)).unwrap();
    let fills = paints(&na.appearance, true);
    assert_eq!(fills, [&Paint::None, &Paint::solid(Color::rgb(1.0, 0.0, 0.0))], "a second fill was added; colours are clamped");
    let n1 = d.node(sel[0]).unwrap();
    assert_eq!(n1.appearance.fill_paint(), Paint::solid(Color::gray(0.5)));
    assert!(n1.appearance.stroke().is_none());
}

/// New objects take the place of the input objects they replace, or go on top of the given
/// container when there were none.
#[test]
fn new_objects_take_the_place_of_replaced_inputs_or_go_on_top() {
    let (mut d, ids, layer) = doc();
    let out = json!({"objects": [{"name": "X", "path": square(0.0)}]});
    apply_output(&mut d, &ids[1..], decode_json(out.clone()).unwrap(), None, &keep).unwrap();
    assert_eq!(names(&d, layer), ["A", "X", "?", "?"], "B and C replaced by X at B's place");
    // No inputs (a generator): on top of the given container.
    let (mut d, _, layer) = doc();
    apply_output(&mut d, &[], decode_json(out).unwrap(), Some(layer), &keep).unwrap();
    assert_eq!(names(&d, layer).last().map(String::as_str), Some("X"));
}

/// An output turns a path into a compound path and back and sets its fill rule, strokes (weights
/// clamped), opacity and name.
#[test]
fn geometry_type_and_strokes_change() {
    let (mut d, ids, _) = doc();
    let a = ids[0];
    let two = PathData::new(square(0.0).subpaths.into_iter().chain(square(2.0).subpaths).collect());
    let blue = json!({"type": "solid", "color": {"model": "rgb", "r": 0, "g": 0, "b": 1}});
    let out = json!({"objects": [{"id": a.0, "type": "compound", "path": two, "fillRule": "evenodd", "strokes": [{"paint": blue, "width": 4000}], "opacity": 0.5, "name": ""}]});
    apply_output(&mut d, &[a], decode_json(out).unwrap(), None, &keep).unwrap();
    let n = d.node(a).unwrap();
    let NodeKind::Compound { children, rule } = &n.kind else { panic!("not a compound: {:?}", n.kind) };
    assert_eq!((children.len(), *rule), (2, FillRule::EvenOdd));
    assert_eq!(n.appearance.stroke().unwrap().width, MAX_STROKE_WIDTH);
    assert_eq!((n.opacity, n.name.as_deref()), (0.5, None));
    assert_eq!(geometry(n).unwrap().0, two);
    // And back to a single path; strokes removed.
    let out = json!({"objects": [{"id": a.0, "type": "path", "strokes": []}]});
    apply_output(&mut d, &[a], decode_json(out).unwrap(), None, &keep).unwrap();
    let n = d.node(a).unwrap();
    assert!(matches!(&n.kind, NodeKind::Path { path, .. } if *path == two));
    assert!(n.appearance.stroke().is_none());
}

/// A compound path made a guide is left out of a later filter's inputs, as guide paths are; a
/// plain compound path is still one of them.
#[test]
fn compound_guides_are_not_filter_targets() {
    let (mut d, _, layer) = doc();
    let two = PathData::new(square(0.0).subpaths.into_iter().chain(square(2.0).subpaths).collect());
    let out = json!({"objects": [{"type": "compound", "path": two.clone(), "guide": true}, {"type": "compound", "path": two}]});
    let made = apply_output(&mut d, &[], decode_json(out).unwrap(), Some(layer), &keep).unwrap();
    let targets = filter_targets(&d, &[layer]);
    assert!(!targets.contains(&made[0]), "the compound guide");
    assert!(targets.contains(&made[1]), "the plain compound path");
}

/// New objects put on top of a layer go below its ruler guides, which stay on top; a ruler guide
/// is never a filter target.
#[test]
fn new_objects_go_below_a_layers_ruler_guides() {
    let (mut d, _, layer) = doc();
    let guide = Node::new(d.alloc_id(), NodeKind::RulerGuide { vertical: true, pos: 5.0, artboard: None });
    let gid = d.insert(Some(layer), usize::MAX, guide).unwrap();
    assert!(!filter_targets(&d, &[layer]).contains(&gid));
    let out = json!({"objects": [{"name": "N", "type": "path", "path": square(0.0)}]});
    let made = apply_output(&mut d, &[], decode_json(out).unwrap(), Some(layer), &keep).unwrap();
    let ids: Vec<NodeId> = d.children(Some(layer)).unwrap().iter().map(|n| n.id).collect();
    assert_eq!(ids.last(), Some(&gid));
    assert_eq!(ids.get(ids.len() - 2), made.first());
}

/// `guide` and `locked` make objects guides (a compound path's members too) and lock them, or undo
/// both; locked objects and guides are not [`selectable`]. Other values are refused.
#[test]
fn objects_become_guides_and_lock() {
    let (mut d, ids, layer) = doc();
    let two = PathData::new(square(0.0).subpaths.into_iter().chain(square(2.0).subpaths).collect());
    let out = json!({"objects": [
        {"id": ids[0].0, "guide": true},
        {"id": ids[1].0, "locked": true},
        {"name": "G", "type": "compound", "path": two, "guide": true, "locked": true}]});
    let made = apply_output(&mut d, &ids[..2], decode_json(out).unwrap(), Some(layer), &keep).unwrap();
    let a = d.node(ids[0]).unwrap();
    assert!(matches!(a.kind, NodeKind::Path { guide: true, .. }) && !a.locked && !selectable(a));
    let b = d.node(ids[1]).unwrap();
    assert!(b.locked && matches!(b.kind, NodeKind::Path { guide: false, .. }) && !selectable(b));
    let g = d.node(made[2]).unwrap();
    let NodeKind::Compound { children, .. } = &g.kind else { panic!("not a compound: {:?}", g.kind) };
    assert!(g.locked && children.iter().all(|c| matches!(c.kind, NodeKind::Path { guide: true, .. })));
    // And back: a plain, unlocked, selectable path.
    let out = json!({"objects": [{"id": ids[0].0, "guide": false, "locked": false}]});
    apply_output(&mut d, &ids[..1], decode_json(out).unwrap(), None, &keep).unwrap();
    assert!(selectable(d.node(ids[0]).unwrap()));
    for bad in [json!({"guide": 1}), json!({"locked": "yes"})] {
        assert!(decode_json(json!({"objects": [bad]})).is_err(), "{bad}");
    }
}

/// A paint handed back as it was keeps its swatch link and any other is unlinked; a pattern paint
/// must name a pattern the document has.
#[test]
fn changed_paints_lose_swatch_links_and_patterns_must_exist() {
    let (mut d, ids, _) = doc();
    let a = ids[0];
    d.node_mut(a).unwrap().appearance.set_fill(Paint::Solid { color: Color::rgb(1.0, 0.0, 0.0), swatch: Some("Red".into()), tint: 0.5 });
    let linked = json!({"type": "solid", "color": {"model": "rgb", "r": 1.0, "g": 0.0, "b": 0.0}, "swatch": "Red", "tint": 0.5});
    apply_output(&mut d, &[a], decode_json(json!({"objects": [{"id": a.0, "fills": [linked]}]})).unwrap(), None, &keep).unwrap();
    assert!(matches!(d.node(a).unwrap().appearance.fill_paint(), Paint::Solid { swatch: Some(_), .. }), "unchanged: kept");
    let other = json!({"type": "solid", "color": {"model": "rgb", "r": 0.0, "g": 1.0, "b": 0.0}, "swatch": "Red", "tint": 0.5});
    apply_output(&mut d, &[a], decode_json(json!({"objects": [{"id": a.0, "fills": [other]}]})).unwrap(), None, &keep).unwrap();
    assert_eq!(d.node(a).unwrap().appearance.fill_paint(), Paint::solid(Color::rgb(0.0, 1.0, 0.0)));
    let pattern = json!({"objects": [{"id": a.0, "fills": [{"type": "pattern", "pattern": "Dots"}]}]});
    assert!(apply_output(&mut d, &[a], decode_json(pattern.clone()).unwrap(), None, &keep).is_err());
    d.patterns.push(serde_json::from_value(json!({"name": "Dots", "tile": {"x0": 0, "y0": 0, "x1": 10, "y1": 10}, "art": []})).unwrap());
    assert!(apply_output(&mut d, &[a], decode_json(pattern).unwrap(), None, &keep).is_ok());
}

/// Output the host can't read as objects is refused, whatever is wrong with it.
#[test]
fn bad_outputs_are_refused() {
    let (d, ids, _) = doc();
    let a = ids[0].0;
    let far = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(1e9, 0.0)], false));
    for bad in [
        json!([]),
        json!({"objects": 3}),
        json!({"objects": [7]}),
        json!({"objects": [{"id": 999}]}),
        json!({"objects": [{"id": a}, {"id": a}]}),
        json!({"objects": [{"name": "no path"}]}),
        json!({"objects": [{"id": a, "path": far}]}),
        json!({"objects": [{"id": a, "path": {"subpaths": []}}]}),
        json!({"objects": [{"id": a, "type": "text"}]}),
        json!({"objects": [{"id": a, "fillRule": "odd"}]}),
        json!({"objects": [{"id": a, "fills": [{"type": "gradient", "gradient": {"kind": "Linear", "stops": []}}]}]}),
        json!({"objects": [{"id": a, "fills": [{"type": "solid"}]}]}),
        json!({"objects": [{"id": a, "strokes": [3]}]}),
        json!({"objects": [{"id": a, "opacity": "full"}]}),
        json!({"error": "nothing to do"}),
    ] {
        let mut d = d.clone();
        let r = decode_json(bad.clone()).and_then(|out| apply_output(&mut d, &ids, out, None, &keep));
        assert!(r.is_err(), "{bad}");
    }
    assert!(decode(b"\xff\xfe").is_err());
}

/// Strokes come with their join, cap and miter limit and objects with their effect records; both go
/// back the same way, the records only as the host admits them.
#[test]
fn stroke_styles_and_effect_records_round_trip() {
    let (mut d, ids, _) = doc();
    let a = ids[0];
    let roughen = Effect { id: "distort.roughen".into(), params: json!({"size": 5.0}), visible: true };
    d.node_mut(a).unwrap().appearance.effects.push(roughen.clone());
    let input: Value = serde_json::from_slice(&encode_input(&d, &[a])).unwrap();
    let (stroke, effects) = (&input["objects"][0]["strokes"][0], &input["objects"][0]["effects"]);
    assert_eq!((&stroke["join"], &stroke["cap"], &stroke["miterLimit"]), (&json!("miter"), &json!("butt"), &json!(10.0)));
    assert_eq!(*effects, json!([{"id": "distort.roughen", "params": {"size": 5.0}, "visible": true}]));
    // Handed back unchanged, nothing changes; the host sees the records and what the object had.
    let seen = std::cell::RefCell::new(vec![]);
    let admit = |fx: &[Effect], n: &Node| -> std::result::Result<Vec<Effect>, String> {
        seen.borrow_mut().push((fx.to_vec(), n.appearance.effects.clone()));
        Ok(fx.to_vec())
    };
    let before = d.clone();
    apply_output(&mut d, &[a], decode(&serde_json::to_vec(&input).unwrap()).unwrap(), None, &admit).unwrap();
    assert_eq!(d, before);
    assert_eq!(*seen.borrow(), [(vec![roughen.clone()], vec![roughen.clone()])]);
    // A round join, a square cap, a miter limit past the range; the effects replaced.
    let out = json!({"objects": [{"id": a.0, "strokes": [{"paint": null, "join": "round", "cap": "square", "miterLimit": 900}],
        "effects": [{"id": "x.y", "params": {"k": 1}, "visible": false}, {"id": "z"}]}]});
    apply_output(&mut d, &[a], decode_json(out).unwrap(), None, &admit).unwrap();
    let n = d.node(a).unwrap();
    let st = n.appearance.stroke().unwrap();
    assert_eq!((st.join, st.cap, st.miter_limit, st.width), (LineJoin::Round, LineCap::Square, *MITER_LIMIT.end(), 1.0), "the weight kept");
    let z = Effect { id: "z".into(), params: Value::Null, visible: true };
    assert_eq!(n.appearance.effects, [Effect { id: "x.y".into(), params: json!({"k": 1}), visible: false }, z]);
    // A new object's stroke takes what it is given; the rest is a new stroke's default.
    let out = json!({"objects": [{"path": square(0.0), "strokes": [{"paint": null, "cap": "round"}]}]});
    let layer = d.layers[0].id;
    let new = apply_output(&mut d, &[], decode_json(out).unwrap(), Some(layer), &admit).unwrap();
    let st = d.node(new[0]).unwrap().appearance.stroke().unwrap().clone();
    assert_eq!((st.cap, st.join, st.width, st.miter_limit), (LineCap::Round, LineJoin::Miter, 1.0, 10.0));
    // What the host refuses changes nothing.
    let before = d.clone();
    let refuse = |_: &[Effect], _: &Node| -> std::result::Result<Vec<Effect>, String> { Err("not an effect".into()) };
    let out = json!({"objects": [{"id": a.0, "effects": [{"id": "x.y"}]}]});
    let e = apply_output(&mut d, &[a], decode_json(out).unwrap(), None, &refuse).unwrap_err();
    assert_eq!((e.to_string().contains("objects[0].effects: not an effect"), &d), (true, &before), "{e}");
}

/// Stroke styles and effect records in a form the host doesn't read are refused.
#[test]
fn bad_stroke_styles_and_effect_records_are_refused() {
    let a = doc().1[0].0;
    for bad in [json!({"join": "pointy"}), json!({"cap": 3}), json!({"miterLimit": "x"}), json!({"width": "wide"})] {
        let mut stroke = json!({"paint": null});
        stroke.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
        assert!(decode_json(json!({"objects": [{"id": a, "strokes": [stroke]}]})).is_err(), "{bad}");
    }
    let long = "x".repeat(MAX_EFFECT_ID + 1);
    for bad in [
        json!(3),
        json!([3]),
        json!([{}]),
        json!([{"id": ""}]),
        json!([{"id": 5}]),
        json!([{"id": long}]),
        json!([{"id": "a\nb"}]),
        json!([{"id": "a", "params": [1]}]),
        json!([{"id": "a", "visible": "yes"}]),
    ] {
        assert!(decode_json(json!({"objects": [{"id": a, "effects": bad}]})).is_err(), "{bad}");
    }
}

/// A `w` × `h` PNG whose pixels vary with their column, half transparent.
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x * 100) as u8, (y * 7) as u8, 30, 128]));
    let mut out = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// An image object `w` × `h` pixels showing blob `key`, linked to `link`.
fn image_node(d: &mut Document, key: &str, (w, h): (u32, u32), link: Option<&str>) -> Node {
    let im = vectorcraft_doc::ImageObject {
        key: key.into(),
        width: w,
        height: h,
        xf: Affine::new([2.0, 0.0, 0.0, 2.0, 40.0, 60.0]),
        link: link.map(vectorcraft_doc::LinkInfo::new),
        placement: Default::default(),
        pixels: None,
    };
    Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)))
}

/// [`doc`] with, above its paths: point type "T" in two paragraphs, an embedded 2 × 1 image "E", a
/// linked image "L", an instance "S" of symbol "Star" and an instance "G" of a symbol the document
/// doesn't have. Returns those five too.
fn mixed() -> (Document, Vec<NodeId>, NodeId, [NodeId; 5]) {
    use vectorcraft_doc::{CharStyle, ParaStyle, Symbol, TextObject};
    let (mut d, ids, layer) = doc();
    let mut t = TextObject::point(Point::new(5.0, 50.0), "Hi\nthere", CharStyle::default());
    t.set_paragraph_styles(vec![ParaStyle::default(), ParaStyle { space_before: 6.0, ..Default::default() }]);
    let text = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
    d.images.insert("e".into(), ImageBlob::png(png(2, 1)));
    d.images.insert("l".into(), ImageBlob::png(png(4, 4)));
    let embedded = image_node(&mut d, "e", (2, 1), None);
    let linked = image_node(&mut d, "l", (4, 4), Some("C:/private/photo.png"));
    let art = Arc::new(Node::path(d.alloc_id(), square(0.0), Appearance::default()));
    d.symbols.push(Symbol { name: "Star".into(), art, dynamic: false });
    let star = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Star".into(), xf: Affine::translate((100.0, 0.0)) });
    let ghost = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Gone".into(), xf: Affine::IDENTITY });
    let mut extra = [NodeId(0); 5];
    for (slot, (mut n, name)) in extra.iter_mut().zip([text, embedded, linked, star, ghost].into_iter().zip(["T", "E", "L", "S", "G"])) {
        n.name = Some(name.into());
        *slot = d.insert(Some(layer), usize::MAX, n).unwrap();
    }
    (d, ids, layer, extra)
}

/// A filter that doesn't list `inputs` is given what it was before: the selected paths alone, in
/// the same bytes. One that lists kinds is given those objects too, in paint order, under the same
/// rules (hidden and locked ones left out, selected ones or ones in a selected layer).
#[test]
fn legacy_filter_input_bytes_are_unchanged() {
    let (mut d, ids, layer, [text, embedded, linked, star, ghost]) = mixed();
    assert_eq!(filter_targets(&d, &[layer]), ids);
    assert_eq!(filter_targets_with(&d, &[layer], &[]), ids);
    let every: Vec<NodeId> = ids.iter().copied().chain([text, embedded, linked, star, ghost]).collect();
    let (bytes, read_only) = encode_input_with(&d, &every, &[], usize::MAX).unwrap();
    assert_eq!(bytes, encode_input(&d, &ids), "the other objects are left out");
    assert!(read_only.is_empty());
    let all = filter_targets_with(&d, &[layer], &InputKind::ALL);
    assert_eq!(all, every);
    let images: Vec<NodeId> = ids.iter().copied().chain([embedded, linked]).collect();
    assert_eq!(filter_targets_with(&d, &[layer], &[InputKind::Image]), images);
    assert_eq!(filter_targets_with(&d, &[star, ids[0]], &InputKind::ALL), [ids[0], star], "selected directly");
    d.node_mut(text).unwrap().visible = false;
    d.node_mut(star).unwrap().locked = true;
    assert_eq!(filter_targets_with(&d, &[layer], &InputKind::ALL), [&ids[..], &[embedded, linked, ghost]].concat());
    let (bytes, read_only) = encode_input_with(&d, &all, &[InputKind::SymbolInstance], usize::MAX).unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    let types: Vec<&str> = v["objects"].as_array().unwrap().iter().map(|o| o["type"].as_str().unwrap()).collect();
    assert_eq!(types, ["path", "path", "path", "symbolInstance", "symbolInstance"]);
    assert_eq!(read_only.len(), 2);
}

/// Type comes with its runs and their character attributes, one paragraph style per paragraph, its
/// kind and transform; type past [`MAX_INPUT_TEXT`] or [`MAX_INPUT_RUNS`] comes without text.
#[test]
fn declared_type_input_has_bounded_runs_and_paragraph_styles() {
    use vectorcraft_doc::{CharStyle, ParaStyle, TextRun};
    let (mut d, _, _, [text, ..]) = mixed();
    let o = encode_read_only(&d, d.node(text).unwrap()).unwrap();
    assert_eq!((&o["type"], &o["name"], &o["truncated"]), (&json!("type"), &json!("T"), &json!(false)));
    assert_eq!(o["transform"], json!([1.0, 0.0, 0.0, 1.0, 5.0, 50.0]));
    assert_eq!((&o["text"]["kind"], &o["text"]["vertical"]), (&json!("point"), &json!(false)));
    assert_eq!(o["text"]["runs"], json!([{"text": "Hi\nthere", "style": serde_json::to_value(CharStyle::default()).unwrap()}]));
    let spaced = ParaStyle { space_before: 6.0, ..Default::default() };
    assert_eq!(o["text"]["paragraphs"], json!([serde_json::to_value(ParaStyle::default()).unwrap(), serde_json::to_value(spaced).unwrap()]));
    assert!(o.get("path").is_none() && o.get("fills").is_none(), "not made into outlines");
    let set_runs = |d: &mut Document, runs: Vec<TextRun>| {
        if let NodeKind::Text(t) = &mut d.node_mut(text).unwrap().kind {
            t.runs = runs;
            t.paras.clear();
        }
    };
    let run = |s: String| TextRun::new(s, CharStyle::default());
    for (runs, truncated) in [
        (vec![run("x".repeat(MAX_INPUT_TEXT))], false),
        (vec![run("x".repeat(MAX_INPUT_TEXT - 1)), run("é".into())], true),
        (vec![run("\n".repeat(MAX_INPUT_RUNS - 1))], false),
        (vec![run("\n".repeat(MAX_INPUT_RUNS))], true),
        ((0..MAX_INPUT_RUNS).map(|_| run("a".into())).collect(), false),
        ((0..=MAX_INPUT_RUNS).map(|_| run("a".into())).collect(), true),
    ] {
        let n = runs.len();
        set_runs(&mut d, runs);
        let o = encode_read_only(&d, d.node(text).unwrap()).unwrap();
        assert_eq!(o["truncated"], json!(truncated), "{n} runs");
        let given = o["text"]["runs"].as_array().unwrap().len();
        assert_eq!((given, o["text"]["paragraphs"].as_array().unwrap().is_empty()), (if truncated { 0 } else { n }, truncated), "{n} runs");
    }
}

/// An embedded image comes with its pixels when they are small; a linked image never does, and
/// nothing of its file's path is given. The color mode is read from the image.
#[test]
fn declared_embedded_and_linked_images_have_distinct_metadata_and_pixel_omission() {
    let (mut d, _, layer, [_, embedded, linked, ..]) = mixed();
    let e = encode_read_only(&d, d.node(embedded).unwrap()).unwrap();
    assert_eq!((&e["type"], &e["pixelSize"], &e["colorMode"]), (&json!("image"), &json!([2, 1]), &json!("rgb")));
    assert_eq!((&e["linked"], &e["pixelsOmitted"], &e["transform"]), (&json!(false), &json!(false), &json!([2.0, 0.0, 0.0, 2.0, 40.0, 60.0])));
    let pixels = craft_vectorcraft_format::base64::decode(e["pixelsBase64"].as_str().unwrap()).unwrap();
    assert_eq!(pixels, [0, 0, 30, 128, 100, 0, 30, 128], "straight RGBA, row by row");
    let l = encode_read_only(&d, d.node(linked).unwrap()).unwrap();
    assert_eq!((&l["linked"], &l["pixelsOmitted"], &l["pixelSize"]), (&json!(true), &json!(true), &json!([4, 4])));
    assert!(l.get("pixelsBase64").is_none());
    assert!(!l.to_string().contains("private"), "the link's path isn't given: {l}");
    // Past the limit (443 × 443 fits it; 444 × 443 doesn't), and pixels of another size, are omitted.
    for ((w, h), omitted) in [((443, 443), false), ((444, 443), true)] {
        d.images.insert("big".into(), ImageBlob::png(png(w, h)));
        let n = image_node(&mut d, "big", (w, h), None);
        let id = d.insert(Some(layer), usize::MAX, n).unwrap();
        let o = encode_read_only(&d, d.node(id).unwrap()).unwrap();
        assert_eq!((&o["pixelsOmitted"], o.get("pixelsBase64").is_some()), (&json!(omitted), !omitted), "{w} × {h}");
    }
    let n = image_node(&mut d, "e", (3, 1), None);
    let id = d.insert(Some(layer), usize::MAX, n).unwrap();
    assert_eq!(encode_read_only(&d, d.node(id).unwrap()).unwrap()["pixelsOmitted"], json!(true), "not the size it says");
    // A size whose pixel count overflows is omitted too.
    let n = image_node(&mut d, "e", (u32::MAX, u32::MAX), None);
    let id = d.insert(Some(layer), usize::MAX, n).unwrap();
    assert_eq!(encode_read_only(&d, d.node(id).unwrap()).unwrap()["pixelsOmitted"], json!(true), "too large to count");
    // Gray pixels, and data that isn't an image.
    let mut gray = vec![];
    image::GrayImage::from_pixel(2, 2, image::Luma([90])).write_to(&mut std::io::Cursor::new(&mut gray), image::ImageFormat::Png).unwrap();
    for (blob, mode) in [(ImageBlob::png(gray), "gray"), (ImageBlob::png(b"not an image".to_vec()), "unknown")] {
        d.images.insert("x".into(), blob);
        let n = image_node(&mut d, "x", (2, 2), None);
        let id = d.insert(Some(layer), usize::MAX, n).unwrap();
        assert_eq!(encode_read_only(&d, d.node(id).unwrap()).unwrap()["colorMode"], json!(mode));
    }
}

/// A CMYK image's pixels are its inks as the working CMYK profile shows them on the canvas: black
/// ink alone is the profile's black, not RGB black.
#[test]
fn cmyk_image_pixels_go_through_the_working_cmyk_profile() {
    use vectorcraft_color::cms::{self, lab};
    let (mut d, _, layer, _) = mixed();
    let inks = vectorcraft_doc::cmyk::Inks::new(2, 1, vec![0, 0, 0, 255, 0, 0, 0, 0]).unwrap();
    d.images.insert("k".into(), ImageBlob::cmyk_tiff(&inks).unwrap());
    let n = image_node(&mut d, "k", (2, 1), None);
    let id = d.insert(Some(layer), usize::MAX, n).unwrap();
    let o = encode_read_only(&d, d.node(id).unwrap()).unwrap();
    assert_eq!((&o["colorMode"], &o["pixelsOmitted"]), (&json!("cmyk"), &json!(false)));
    let pixels = craft_vectorcraft_format::base64::decode(o["pixelsBase64"].as_str().unwrap()).unwrap();
    let shown = |c: [f32; 4]| lab::xyz_to_srgb(lab::lab_to_xyz(cms::active().cmyk_display_lab(c))).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    let ([r, g, b], [r2, g2, b2]) = (shown([0.0, 0.0, 0.0, 1.0]), shown([0.0; 4]));
    assert_eq!(pixels, [r, g, b, 255, r2, g2, b2, 255]);
    assert_ne!(&pixels[..3], &[0, 0, 0], "not converted without the profile");
}

/// The image object for `key` in `d`'s layer, `size` pixels.
fn placed(d: &mut Document, layer: NodeId, key: &str, size: (u32, u32)) -> NodeId {
    let n = image_node(d, key, size, None);
    d.insert(Some(layer), usize::MAX, n).unwrap()
}

/// The largest pixels one image may have fill exactly [`MAX_INPUT_PIXELS`] characters of base64
/// (512 × 384); one pixel more doesn't fit.
#[test]
fn an_image_at_the_limit_is_given_and_one_pixel_more_is_omitted() {
    let (mut d, _, layer, _) = mixed();
    for ((w, h), omitted) in [((512, 384), false), ((1, 196_609), true)] {
        d.images.insert("limit".into(), ImageBlob::png(png(w, h)));
        let id = placed(&mut d, layer, "limit", (w, h));
        let o = encode_read_only(&d, d.node(id).unwrap()).unwrap();
        assert_eq!(o["pixelsOmitted"], json!(omitted), "{w} × {h}");
        assert_eq!(o.get("pixelsBase64").and_then(Value::as_str).map(str::len), (!omitted).then_some(MAX_INPUT_PIXELS), "{w} × {h}");
    }
}

/// Sixteen images at the per-image limit fill [`MAX_INPUT_PIXELS_TOTAL`] exactly and are all given;
/// the next one in paint order, even a small one, comes with `pixelsOmitted`.
#[test]
fn images_fill_the_pixel_total_and_the_next_is_omitted() {
    let (mut d, _, layer, [_, embedded, ..]) = mixed();
    d.images.insert("limit".into(), ImageBlob::png(png(512, 384)));
    let mut ids: Vec<NodeId> = (0..16).map(|_| placed(&mut d, layer, "limit", (512, 384))).collect();
    ids.push(embedded);
    let (bytes, _) = encode_input_with(&d, &ids, &[InputKind::Image], usize::MAX).unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    let objects = v["objects"].as_array().unwrap();
    assert_eq!(objects.len(), 17);
    let given: usize = objects.iter().filter_map(|o| o.get("pixelsBase64")?.as_str()).map(str::len).sum();
    assert_eq!(given, MAX_INPUT_PIXELS_TOTAL);
    let omitted: Vec<bool> = objects.iter().map(|o| o["pixelsOmitted"] == json!(true)).collect();
    assert_eq!(omitted, [vec![false; 16], vec![true]].concat());
}

/// Type whose runs and paragraphs together fill [`MAX_INPUT_TEXT_TOTAL`] exactly is all given, and
/// type after it in paint order comes with `truncated`. With one byte more, the last type that
/// doesn't fit comes with `truncated` and what it leaves still goes to the type after it.
#[test]
fn type_fills_the_text_total_and_what_follows_is_truncated() {
    use vectorcraft_doc::{CharStyle, TextObject};
    let (mut d, _, layer, _) = mixed();
    let place = |d: &mut Document, len: usize| {
        let t = TextObject::point(Point::new(0.0, 0.0), &"x".repeat(len), CharStyle::default());
        let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
        d.insert(Some(layer), usize::MAX, n).unwrap()
    };
    let one = place(&mut d, 1);
    let o = encode_read_only(&d, d.node(one).unwrap()).unwrap();
    // Everything but the text itself, with the commas between runs and paragraphs and without
    // their brackets.
    let overhead: usize = ["runs", "paragraphs"].iter().map(|k| o["text"][k].to_string().len() - 2).sum::<usize>() - 1;
    let full = overhead + MAX_INPUT_TEXT;
    let count = MAX_INPUT_TEXT_TOTAL / full;
    let rest = MAX_INPUT_TEXT_TOTAL - count * full - overhead;
    assert!((1..=MAX_INPUT_TEXT).contains(&rest));
    for (filler, truncated) in [(rest, [false, true]), (rest + 1, [true, false])] {
        let mut ids: Vec<NodeId> = (0..count).map(|_| place(&mut d, MAX_INPUT_TEXT)).collect();
        ids.push(place(&mut d, filler));
        ids.push(one);
        let (bytes, _) = encode_input_with(&d, &ids, &[InputKind::Type], usize::MAX).unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        let objects = v["objects"].as_array().unwrap();
        assert_eq!(objects.len(), count + 2);
        assert!(objects[..count].iter().all(|o| o["truncated"] == json!(false)), "filler {filler}");
        assert_eq!([&objects[count]["truncated"], &objects[count + 1]["truncated"]], truncated.map(|t| json!(t)).each_ref(), "filler {filler}");
    }
}

/// A CMYK TIFF `w` × `h` pixels of no ink.
fn cmyk_blob(w: u32, h: u32) -> ImageBlob {
    let inks = vectorcraft_doc::cmyk::Inks::new(w, h, vec![0; (w * h * 4) as usize]).unwrap();
    ImageBlob::cmyk_tiff(&inks).unwrap()
}

/// `blob`, a little-endian TIFF, with its strip offset past the end of the file: its header still
/// reads and its pixels don't.
fn without_strips(blob: &ImageBlob) -> ImageBlob {
    let mut bytes = blob.bytes.to_vec();
    assert_eq!(&bytes[..2], b"II");
    let word = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize;
    let ifd = word(&bytes, 4);
    let count = usize::from(u16::from_le_bytes([bytes[ifd], bytes[ifd + 1]]));
    let entry = (0..count).map(|i| ifd + 2 + i * 12).find(|&e| bytes[e..e + 2] == 273u16.to_le_bytes()).unwrap();
    let past = (bytes.len() as u32 + 4096).to_le_bytes();
    bytes[entry + 8..entry + 12].copy_from_slice(&past);
    ImageBlob::png(bytes)
}

/// An image that can't be given isn't decoded: past the per-image limit, past what is left of the
/// total, once the total is used up, when its header disagrees with its object, and when it is
/// linked, a CMYK image as much as any other. One that fits is decoded once.
#[test]
fn an_image_that_cannot_be_given_is_not_decoded() {
    let (mut d, _, layer, [_, embedded, linked, ..]) = mixed();
    d.images.insert("big".into(), ImageBlob::png(png(1, 196_609)));
    d.images.insert("k".into(), cmyk_blob(2, 1));
    let big = placed(&mut d, layer, "big", (1, 196_609));
    let wrong = placed(&mut d, layer, "e", (3, 1));
    let cmyk = placed(&mut d, layer, "k", (2, 1));
    let cmyk_wrong = placed(&mut d, layer, "k", (3, 1));
    let within = |pixels: usize| Allowance { pixels, ..Allowance::FULL };
    for (id, left, omitted, decodes, what) in [
        (big, Allowance::FULL, true, 0, "past the per-image limit"),
        (embedded, within(11), true, 0, "past what is left of the total"),
        (embedded, within(0), true, 0, "the total used up"),
        (wrong, Allowance::FULL, true, 0, "another size than its object"),
        (linked, Allowance::FULL, true, 0, "linked"),
        (embedded, within(12), false, 1, "fits"),
        (cmyk, within(11), true, 0, "CMYK past what is left of the total"),
        (cmyk_wrong, Allowance::FULL, true, 0, "CMYK of another size than its object"),
        (cmyk, within(12), false, 1, "CMYK that fits"),
    ] {
        DECODES.with(|n| n.set(0));
        let (bytes, _) = encode_input_within(&d, &[id], &[InputKind::Image], left).unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["objects"][0]["pixelsOmitted"], json!(omitted), "{what}");
        assert_eq!(DECODES.with(|n| n.get()), decodes, "{what}");
    }
}

/// An image that reaches the decoder and fails takes its size from the total, a broken CMYK image
/// as much as a broken PNG: the next image doesn't fit what the broken one left.
#[test]
fn an_image_that_fails_to_decode_takes_from_the_total() {
    let (mut d, _, layer, [_, embedded, ..]) = mixed();
    let mut png_data = png(2, 1);
    let idat = png_data.windows(4).position(|w| w == b"IDAT").unwrap();
    png_data[idat + 6] ^= 0xFF;
    for (what, blob, expect_cmyk) in [("PNG", ImageBlob::png(png_data), false), ("CMYK", without_strips(&cmyk_blob(2, 1)), true)] {
        assert_eq!(blob.is_cmyk(), expect_cmyk, "{what}");
        d.images.insert("broken".into(), blob);
        let bad = placed(&mut d, layer, "broken", (2, 1));
        let omitted = |pixels: usize| {
            DECODES.with(|n| n.set(0));
            let left = Allowance { pixels, ..Allowance::FULL };
            let (bytes, _) = encode_input_within(&d, &[bad, embedded], &[InputKind::Image], left).unwrap();
            let v: Value = serde_json::from_slice(&bytes).unwrap();
            (v["objects"][0]["pixelsOmitted"].clone(), v["objects"][1]["pixelsOmitted"].clone(), DECODES.with(|n| n.get()))
        };
        assert_eq!(omitted(12), (json!(true), json!(true), 1), "{what}: one decode fits 12 characters");
        assert_eq!(omitted(24), (json!(true), json!(false), 2), "{what}: the next image fits what is left");
    }
}

/// Runs in one paragraph keep their own leading: each run comes with its character attributes, and
/// the paragraph is one paragraph style.
#[test]
fn runs_in_one_paragraph_keep_their_own_leading() {
    use vectorcraft_doc::{CharStyle, TextObject, TextRun};
    let (mut d, _, layer, _) = mixed();
    let mut t = TextObject::point(Point::new(0.0, 0.0), "", CharStyle::default());
    let leading = CharStyle { leading: Some(14.0), ..CharStyle::default() };
    t.runs = vec![TextRun::new("a".to_string(), leading), TextRun::new("b".to_string(), CharStyle::default())];
    let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
    let id = d.insert(Some(layer), usize::MAX, n).unwrap();
    let o = encode_read_only(&d, d.node(id).unwrap()).unwrap();
    assert_eq!(o["text"]["runs"][0]["style"]["leading"], json!(14.0));
    assert_eq!((&o["text"]["runs"][1]["text"], &o["text"]["runs"][1]["style"]["leading"]), (&json!("b"), &Value::Null));
    assert_eq!(o["text"]["paragraphs"].as_array().unwrap().len(), 1);
}

/// Each read-only object's place in the input holds that object, pixels included, and the input's
/// paths have no place. An output that echoes an object is compared with what that place holds; an
/// input that doesn't hold it refuses the output, as the host's own error rather than the
/// plug-in's.
#[test]
fn read_only_objects_are_read_from_their_place_in_the_input_and_fail_closed() {
    let (d, ids, _, [text, embedded, ..]) = mixed();
    let order = [ids[0], text, embedded];
    let (bytes, read_only) = encode_input_with(&d, &order, &InputKind::ALL, usize::MAX).unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(read_only.len(), 2);
    let at = |id: NodeId| serde_json::from_slice::<Value>(&bytes[read_only[&id].clone()]).unwrap();
    assert_eq!(at(text), v["objects"][1]);
    assert_eq!(at(embedded), v["objects"][2]);
    assert!(at(embedded)["pixelsBase64"].is_string());
    let echo = json!({"objects": [v["objects"][1], v["objects"][2]]}).to_string();
    assert!(decode_output_with(echo.as_bytes(), &bytes, &read_only).is_ok());
    let garbage = vec![b'x'; bytes.len()];
    for input in [garbage.as_slice(), b"not json", br#"{"objects":[]}"#, b"{}", b""] {
        let e = decode_output_with(echo.as_bytes(), input, &read_only).unwrap_err().to_string();
        assert!(!e.starts_with("plug-in"), "{e}");
    }
    // `{"id": …}` alone needs nothing from the input.
    let kept = json!({"objects": [{"id": text.0}, {"id": embedded.0}]}).to_string();
    assert!(decode_output_with(kept.as_bytes(), b"", &read_only).is_ok());
}

/// A path's effect parameters can nest deeper than the JSON reader goes. Checking the output reads
/// only the read-only objects from the input, so such a path beside one doesn't fail the run.
#[test]
fn a_path_nested_past_the_json_reader_beside_a_read_only_object_passes_the_check() {
    let (mut d, ids, _, [text, ..]) = mixed();
    let params = (0..200).fold(json!(1), |inner, _| json!([inner]));
    d.node_mut(ids[0]).unwrap().appearance.effects.push(vectorcraft_doc::Effect { id: "x".into(), params, visible: true });
    let (bytes, read_only) = encode_input_with(&d, &[ids[0], text], &[InputKind::Type], usize::MAX).unwrap();
    assert!(serde_json::from_slice::<Value>(&bytes).is_err(), "the input is deeper than the reader goes");
    let echo = serde_json::from_slice::<Value>(&bytes[read_only[&text].clone()]).unwrap();
    let out = decode_output_with(json!({"objects": [echo]}).to_string().as_bytes(), &bytes, &read_only).unwrap();
    assert_eq!(out.objects.iter().filter(|o| o.kept).count(), 1);
}

/// A symbol instance comes with the symbol it shows and its transform, never its art; one whose
/// symbol the document lacks says so.
#[test]
fn declared_symbol_instance_keeps_identity_and_transform() {
    let (d, _, _, [.., star, ghost]) = mixed();
    let s = encode_read_only(&d, d.node(star).unwrap()).unwrap();
    assert_eq!((&s["type"], &s["symbolId"], &s["overrides"]), (&json!("symbolInstance"), &json!("Star"), &json!({})));
    assert_eq!(s["transform"], json!([1.0, 0.0, 0.0, 1.0, 100.0, 0.0]));
    assert!(s.get("missing").is_none() && s.get("path").is_none());
    let g = encode_read_only(&d, d.node(ghost).unwrap()).unwrap();
    assert_eq!((&g["symbolId"], &g["missing"]), (&json!("Gone"), &json!(true)));
}

/// Pixels and type count against totals over the whole input, in order: an image whose pixels
/// don't fit what the images before it left comes with `pixelsOmitted` (its size checked before it
/// is decoded), and type whose runs and paragraphs don't fit comes with `truncated`. The read-only
/// objects returned are the ones in the input.
#[test]
fn input_totals_cover_images_and_type_in_order() {
    let (mut d, _, layer, [text, embedded, ..]) = mixed();
    let copy = |d: &mut Document, id: NodeId| {
        let mut n = d.node(id).unwrap().clone();
        n.id = d.alloc_id();
        d.insert(Some(layer), usize::MAX, n).unwrap()
    };
    let (text2, image2, image3) = (copy(&mut d, text), copy(&mut d, embedded), copy(&mut d, embedded));
    let one = encode_read_only(&d, d.node(text).unwrap()).unwrap();
    // The runs and paragraphs with the commas between them, without their brackets.
    let size: usize = ["runs", "paragraphs"].iter().map(|k| one["text"][k].to_string().len() - 2).sum();
    // A 2 × 1 image is 12 characters of base64: two fit 30, the third doesn't.
    let ids = [text, text2, embedded, image2, image3];
    let (bytes, read_only) = encode_input_within(&d, &ids, &InputKind::ALL, Allowance { pixels: 30, text: size, input: usize::MAX }).unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    let objects = v["objects"].as_array().unwrap();
    assert_eq!(objects.len(), 5);
    let flags: Vec<&Value> = objects.iter().map(|o| if o["type"] == "type" { &o["truncated"] } else { &o["pixelsOmitted"] }).collect();
    assert_eq!(flags, [&json!(false), &json!(true), &json!(false), &json!(false), &json!(true)]);
    assert!(objects[1]["text"]["runs"].as_array().unwrap().is_empty() && objects[4].get("pixelsBase64").is_none());
    assert_eq!(objects[3]["pixelsBase64"], encode_read_only(&d, d.node(embedded).unwrap()).unwrap()["pixelsBase64"]);
    for o in objects {
        assert!(read_only.contains_key(&NodeId(o["id"].as_u64().unwrap())));
    }
    // Within the full totals every one of them is given.
    let (bytes, _) = encode_input_with(&d, &ids, &InputKind::ALL, usize::MAX).unwrap();
    let all = String::from_utf8(bytes).unwrap();
    assert!(!all.contains("\"pixelsOmitted\":true") && !all.contains("\"truncated\":true"), "{all}");
}

/// Every object counts toward the input's capacity in order, its id, bounds, transform and name
/// included: a symbol instance before type leaves the type less room, so text that fits the exact
/// size of the whole input comes with `truncated` one byte under it, and an image's pixels are
/// omitted the same way.
#[test]
fn object_fields_count_toward_the_input_capacity() {
    let (d, ids, _, [text, embedded, _, star, _]) = mixed();
    for (order, flag) in [([ids[0], star, text], "truncated"), ([ids[0], star, embedded], "pixelsOmitted")] {
        let (bytes, _) = encode_input_with(&d, &order, &InputKind::ALL, usize::MAX).unwrap();
        let (exact, _) = encode_input_with(&d, &order, &InputKind::ALL, bytes.len()).unwrap();
        assert_eq!(exact, bytes, "{flag}: the exact size fits");
        let (under, _) = encode_input_with(&d, &order, &InputKind::ALL, bytes.len() - 1).unwrap();
        let v: Value = serde_json::from_slice(&under).unwrap();
        assert_eq!(v["objects"][2][flag], json!(true), "{flag}: one byte under");
        assert!(under.len() < bytes.len() - 1, "{flag}");
    }
}

/// Type with fractional colors counts the bytes the input gives it: one byte under its exact size
/// it comes with `truncated`, not a limit.
#[test]
fn fractional_colors_count_as_the_input_writes_them() {
    use vectorcraft_doc::{CharStyle, TextObject};
    let (mut d, ids, layer, _) = mixed();
    let style = CharStyle { fill: Paint::solid(Color::rgb(0.1, 0.2, 0.3)), ..CharStyle::default() };
    let t = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(0.0, 0.0), "Hi", style))));
    let text = d.insert(Some(layer), usize::MAX, t).unwrap();
    let order = [ids[0], text];
    let (bytes, _) = encode_input_with(&d, &order, &[InputKind::Type], usize::MAX).unwrap();
    let (under, _) = encode_input_with(&d, &order, &[InputKind::Type], bytes.len() - 1).unwrap();
    let v: Value = serde_json::from_slice(&under).unwrap();
    assert_eq!(v["objects"][1]["truncated"], json!(true));
}

/// A read-only object whose fields other than its pixels and text alone don't fit what is left of
/// the input fails the run with a limit naming it and its size, rather than being left out.
#[test]
fn an_object_too_large_to_give_is_a_limit_naming_it() {
    let (mut d, ids, _, [_, _, linked, star, _]) = mixed();
    d.node_mut(linked).unwrap().name = Some("n".repeat(4096));
    let (fits, _) = encode_input_with(&d, &[ids[0], star], &InputKind::ALL, usize::MAX).unwrap();
    let order = [ids[0], star, linked];
    let e = encode_input_with(&d, &order, &InputKind::ALL, fits.len() + 4096).unwrap_err();
    assert!(matches!(e, Error::Limit(_)), "{e}");
    // Measured with `pixelsOmitted` false and a comma before it.
    let mut o = encode_read_only(&d, d.node(linked).unwrap()).unwrap();
    o["pixelsOmitted"] = json!(false);
    let size = o.to_string().len() + 1;
    assert_eq!(e.to_string(), format!("plug-in exceeded its memory budget (object {} needs {size} bytes of input, 4096 left)", linked.0));
    // A filter that doesn't list its kind is given the rest.
    let (bytes, read_only) = encode_input_with(&d, &order, &[InputKind::SymbolInstance], fits.len() + 4096).unwrap();
    assert_eq!((bytes, read_only.len()), (fits, 1));
}

/// A path or compound path that doesn't fit what is left of the input fails the run with the limit
/// the runtime gives an input past its share, before anything after it is written; the exact size
/// still fits.
#[test]
fn a_path_past_the_input_capacity_is_the_runtime_limit() {
    let (d, ids, _, [text, ..]) = mixed();
    let (bytes, _) = encode_input_with(&d, &ids, &[], usize::MAX).unwrap();
    assert_eq!(encode_input_with(&d, &ids, &[], bytes.len()).unwrap().0, bytes, "the exact size fits");
    let e = encode_input_with(&d, &[ids[0], ids[1], ids[2], text], &[InputKind::Type], bytes.len() - 1).unwrap_err();
    assert_eq!(e.to_string(), "plug-in exceeded its memory budget (the selection is too large to hand over)");
}

/// An output lists at most [`MAX_OBJECTS`] objects and must return every read-only one, so an input
/// with more read-only objects than that is a limit; exactly that many are given.
#[test]
fn more_read_only_objects_than_an_output_lists_is_a_limit() {
    let (mut d, _, layer, [.., ghost]) = mixed();
    let mut ids = vec![ghost];
    for _ in 1..MAX_OBJECTS {
        let n = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Gone".into(), xf: Affine::IDENTITY });
        ids.push(d.insert(Some(layer), usize::MAX, n).unwrap());
    }
    let (_, read_only) = encode_input_with(&d, &ids, &[InputKind::SymbolInstance], usize::MAX).unwrap();
    assert_eq!(read_only.len(), MAX_OBJECTS);
    let n = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Gone".into(), xf: Affine::IDENTITY });
    ids.push(d.insert(Some(layer), usize::MAX, n).unwrap());
    let e = encode_input_with(&d, &ids, &[InputKind::SymbolInstance], usize::MAX).unwrap_err();
    assert_eq!(e.to_string(), format!("plug-in exceeded its object budget (more than {MAX_OBJECTS} read-only objects)"));
    // A filter that doesn't list the kind is given none of them.
    assert!(encode_input_with(&d, &ids, &[], usize::MAX).unwrap().1.is_empty());
}

/// Objects that clip a clip group are left out, and so are those inside a group that clips one;
/// those it clips are given. This covers a filter that lists no `inputs` too.
#[test]
fn objects_that_clip_a_clip_group_are_not_inputs() {
    let (mut d, _, layer, [text, embedded, ..]) = mixed();
    let (t, e) = (Arc::new(d.node(text).unwrap().clone()), Arc::new(d.node(embedded).unwrap().clone()));
    let mut copy = |n: &Arc<Node>| {
        let mut c = (**n).clone();
        c.id = d.alloc_id();
        Arc::new(c)
    };
    let (t_source, e_source, t_inner, e_inner, t_clipped) = (copy(&t), copy(&e), copy(&t), copy(&e), copy(&t));
    let inner = Node::group(d.alloc_id(), vec![t_inner.clone(), e_inner.clone()]);
    let mut by_group = Node::group(d.alloc_id(), vec![Arc::new(inner), t_clipped.clone()]);
    by_group.set_clips(true);
    let mut by_type = Node::group(d.alloc_id(), vec![t_source.clone(), e_source.clone()]);
    by_type.set_clips(true);
    let by_group = d.insert(Some(layer), usize::MAX, by_group).unwrap();
    let by_type = d.insert(Some(layer), usize::MAX, by_type).unwrap();
    let path = |d: &mut Document, x: f64| Arc::new(Node::path(d.alloc_id(), square(x), Appearance::default()));
    let (part, clipped_by_compound, in_source, clipped_by_paths) = (path(&mut d, 0.0), path(&mut d, 10.0), path(&mut d, 20.0), path(&mut d, 30.0));
    let compound = Node::new(d.alloc_id(), NodeKind::Compound { children: vec![part], rule: FillRule::NonZero });
    let mut by_compound = Node::group(d.alloc_id(), vec![Arc::new(compound), clipped_by_compound.clone()]);
    by_compound.set_clips(true);
    let source = Node::group(d.alloc_id(), vec![in_source]);
    let mut by_paths = Node::group(d.alloc_id(), vec![Arc::new(source), clipped_by_paths.clone()]);
    by_paths.set_clips(true);
    let by_compound = d.insert(Some(layer), usize::MAX, by_compound).unwrap();
    let by_paths = d.insert(Some(layer), usize::MAX, by_paths).unwrap();
    let ids = |list: &[NodeId]| filter_targets_with(&d, list, &InputKind::ALL);
    assert_eq!(ids(&[by_type]), [e_source.id], "the type clips; the image is clipped");
    assert_eq!(ids(&[by_group]), [t_clipped.id], "the group's type and image clip");
    assert_eq!(ids(&[by_compound]), [clipped_by_compound.id], "the compound path clips");
    assert_eq!(ids(&[by_paths]), [clipped_by_paths.id], "the group's path clips");
    assert_eq!(filter_targets(&d, &[by_compound]), [clipped_by_compound.id], "a filter without inputs");
    assert_eq!(filter_targets(&d, &[by_paths]), [clipped_by_paths.id], "a filter without inputs");
}

/// A TIFF with more strips and planes than the decoder can count comes as `"unknown"`, without its
/// pixels, and never reaches the decoder.
#[test]
fn a_tiff_too_large_to_count_is_unknown_and_has_no_pixels() {
    let entries: [(u16, u16, u32); 9] =
        [(256, 4, 1), (257, 4, 1 << 31 | 1), (258, 3, 8), (262, 3, 1), (273, 4, 0), (277, 3, 2), (278, 4, 1), (279, 4, 0), (284, 3, 2)];
    let mut tiff = b"II*\0\x08\0\0\0".to_vec();
    tiff.extend(u16::try_from(entries.len()).unwrap().to_le_bytes());
    for (tag, ty, v) in entries {
        tiff.extend(tag.to_le_bytes());
        tiff.extend(ty.to_le_bytes());
        tiff.extend(1u32.to_le_bytes());
        tiff.extend(v.to_le_bytes());
    }
    tiff.extend([0; 4]);
    let (mut d, _, layer, _) = mixed();
    d.images.insert("big".into(), ImageBlob::new("image/tiff", tiff));
    let big = image_node(&mut d, "big", (1, 1), None);
    let big = d.insert(Some(layer), usize::MAX, big).unwrap();
    let (bytes, _) = encode_input_with(&d, &[big], &[InputKind::Image], usize::MAX).unwrap();
    let input: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(input["objects"][0]["colorMode"], "unknown");
    assert_eq!(input["objects"][0]["pixelsOmitted"], true);
}

/// A field a plug-in adds to a read-only object is named in the refusal with its control and
/// direction characters escaped, and cut short.
#[test]
fn a_changed_field_is_named_escaped_and_short() {
    let (d, _, layer, [text, ..]) = mixed();
    let targets = filter_targets_with(&d, &[layer], &[InputKind::Type]);
    let (bytes, read_only) = encode_input_with(&d, &targets, &[InputKind::Type], usize::MAX).unwrap();
    let mut v: Value = serde_json::from_slice(&bytes).unwrap();
    let at = v["objects"].as_array().unwrap().iter().position(|o| o["id"] == json!(text.0)).unwrap();
    let key = format!("\u{202E}{}", "k".repeat(500));
    v["objects"][at][key.as_str()] = json!(1);
    let e = decode_output_with(v.to_string().as_bytes(), &bytes, &read_only).unwrap_err().to_string();
    let shown = e.split('\'').nth(1).unwrap_or_default();
    assert!(!e.contains('\u{202E}') && shown.starts_with("\\u{202E}kkk") && shown.ends_with('…'), "{e}");
    assert_eq!(shown.chars().count(), crate::permissions::MAX_SHOWN_FIELD, "{e}");
}

/// A filter given read-only objects returns each one as `{"id": …}` or exactly as given (numbers by
/// value), and they stay as they are, new objects listed after one going just above it. Any change,
/// an object left out or listed twice, and a new object of such a kind are refused before anything
/// changes.
#[test]
fn read_only_kind_field_and_omission_refuse() {
    let (mut d, ids, layer, [text, embedded, linked, star, ghost]) = mixed();
    let targets = filter_targets_with(&d, &[layer], &InputKind::ALL);
    let (bytes, read_only) = encode_input_with(&d, &targets, &InputKind::ALL, usize::MAX).unwrap();
    assert_eq!(read_only.len(), 5);
    let input: Value = serde_json::from_slice(&bytes).unwrap();
    let before = d.clone();
    let out = decode_output_with(&bytes, &bytes, &read_only).unwrap();
    assert_eq!(out.objects.iter().filter(|o| o.kept).count(), 5);
    apply_output(&mut d, &targets, out.objects, None, &keep).unwrap();
    assert_eq!(d, before, "an exact echo changes nothing");
    // A filter that doesn't list `inputs` could never hand them back.
    assert!(decode_output(&bytes).is_err());
    // Numbers written another way are the same numbers.
    let mut echo = input.clone();
    echo["objects"][3]["transform"] = json!([1, 0, 0, 1, 5, 50]);
    assert!(decode_output_with(echo.to_string().as_bytes(), &bytes, &read_only).is_ok());
    // So is an id written as a float; a fractional one names no object.
    let id = input["objects"][3]["id"].as_u64().unwrap();
    let mut echo = input.clone();
    echo["objects"][3]["id"] = json!(id as f64);
    let out = decode_output_with(echo.to_string().as_bytes(), &bytes, &read_only).unwrap();
    assert_eq!(out.objects.iter().filter(|o| o.kept).count(), 5);
    echo["objects"][3]["id"] = json!(id as f64 + 0.5);
    assert!(decode_output_with(echo.to_string().as_bytes(), &bytes, &read_only).is_err());
    // `{"id"}` alone keeps it; the paths left out are deleted and a new path goes above the type.
    let ro = |id: NodeId| json!({"id": id.0});
    let out = json!({"objects": [ro(text), {"name": "new", "path": square(0.0)}, ro(embedded), ro(linked), ro(star), ro(ghost)]});
    let made =
        apply_output(&mut d, &targets, decode_output_with(out.to_string().as_bytes(), &bytes, &read_only).unwrap().objects, None, &keep).unwrap();
    assert_eq!(names(&d, layer), ["?", "?", "T", "new", "E", "L", "S", "G"]);
    assert_eq!(made.len(), 6);
    assert!(ids.iter().all(|id| d.node(*id).is_none()));
    assert_eq!(d.node(text), before.node(text), "the type is as it was");
    // Refused, naming the object and the field.
    let t = text.0;
    let changed = |f: &dyn Fn(&mut Value)| {
        let mut v = input.clone();
        f(&mut v);
        v
    };
    for (out, says) in [
        (
            changed(&|v| v["objects"][3]["text"]["runs"][0]["text"] = json!("Bye")),
            format!("objects[3] (id {t}): read-only field 'text.runs[0].text' changed"),
        ),
        (changed(&|v| v["objects"][3]["type"] = json!("path")), format!("(id {t}): read-only field 'type' changed")),
        (changed(&|v| v["objects"][3]["opacity"] = json!(0.5)), format!("(id {t}): read-only field 'opacity' changed")),
        (changed(&|v| v["objects"][3]["transform"][4] = json!(6)), format!("(id {t}): read-only field 'transform[4]' changed")),
        (
            changed(&|v| {
                v["objects"][4].as_object_mut().unwrap().remove("pixelsBase64");
            }),
            format!("(id {}): read-only field 'pixelsBase64' changed", embedded.0),
        ),
        (changed(&|v| v["objects"][6]["symbolId"] = json!("Moon")), format!("(id {}): read-only field 'symbolId' changed", star.0)),
        (
            changed(&|v| {
                v["objects"].as_array_mut().unwrap().remove(3);
            }),
            format!("objects (id {t}): a read-only object was left out"),
        ),
        (changed(&|v| v["objects"].as_array_mut().unwrap().push(json!({"id": ghost.0}))), format!("objects[8] (id {}): is listed twice", ghost.0)),
        (
            changed(&|v| v["objects"].as_array_mut().unwrap().push(json!({"type": "type", "path": square(0.0)}))),
            "`type` must be \"path\" or \"compound\"".to_string(),
        ),
    ] {
        let e = decode_output_with(out.to_string().as_bytes(), &bytes, &read_only).unwrap_err().to_string();
        assert!(e.contains(&says), "{says}: {e}");
    }
    // Writing one or leaving one out is refused when applying too, whatever decoded the output.
    let mut d = before.clone();
    let write = vec![OutObject { id: Some(text), name: Some("Renamed".into()), ..OutObject::default() }];
    let e = apply_output(&mut d, &[text], write, None, &keep).unwrap_err().to_string();
    assert!(e.contains("is read-only"), "{e}");
    let e = apply_output(&mut d, &[text], vec![], None, &keep).unwrap_err().to_string();
    assert!(e.contains("a read-only object was left out"), "{e}");
    assert_eq!(d, before);
}

/// A filter's `documentData`: absent keeps what the plug-in keeps, `null` forgets it, any other JSON
/// of at most [`MAX_DOCUMENT_DATA`] bytes is kept, and more refuses the whole output.
#[test]
fn document_data_is_kept_forgotten_or_refused() {
    let out = |data: Option<Value>| {
        let mut v = json!({"objects": []});
        if let Some(data) = data {
            v["documentData"] = data;
        }
        decode_output(v.to_string().as_bytes()).map(|o| o.document_data)
    };
    assert_eq!(out(None).unwrap(), None);
    assert_eq!(out(Some(Value::Null)).unwrap(), Some(None));
    assert_eq!(out(Some(json!({"pens": 2}))).unwrap(), Some(Some(json!({"pens": 2}))));
    // A string's quotes count: this one is the limit exactly, and one more byte refuses it.
    let at = json!("x".repeat(MAX_DOCUMENT_DATA - 2));
    assert_eq!(out(Some(at.clone())).unwrap(), Some(Some(at)));
    assert!(matches!(out(Some(json!("x".repeat(MAX_DOCUMENT_DATA - 1)))), Err(Error::Output(_))));
    // A decline is still a decline.
    assert!(matches!(decode_output(br#"{"error": "No pens.", "documentData": 1}"#), Err(Error::Failed(_))));
}
