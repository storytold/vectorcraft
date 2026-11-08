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
        assert_eq!(Some(o), read_only.get(&NodeId(o["id"].as_u64().unwrap())));
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
    let e = decode_output_with(v.to_string().as_bytes(), &read_only).unwrap_err().to_string();
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
    let out = decode_output_with(&bytes, &read_only).unwrap();
    assert_eq!(out.objects.iter().filter(|o| o.kept).count(), 5);
    apply_output(&mut d, &targets, out.objects, None, &keep).unwrap();
    assert_eq!(d, before, "an exact echo changes nothing");
    // A filter that doesn't list `inputs` could never hand them back.
    assert!(decode_output(&bytes).is_err());
    // Numbers written another way are the same numbers.
    let mut echo = input.clone();
    echo["objects"][3]["transform"] = json!([1, 0, 0, 1, 5, 50]);
    assert!(decode_output_with(echo.to_string().as_bytes(), &read_only).is_ok());
    // `{"id"}` alone keeps it; the paths left out are deleted and a new path goes above the type.
    let ro = |id: NodeId| json!({"id": id.0});
    let out = json!({"objects": [ro(text), {"name": "new", "path": square(0.0)}, ro(embedded), ro(linked), ro(star), ro(ghost)]});
    let made = apply_output(&mut d, &targets, decode_output_with(out.to_string().as_bytes(), &read_only).unwrap().objects, None, &keep).unwrap();
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
        let e = decode_output_with(out.to_string().as_bytes(), &read_only).unwrap_err().to_string();
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
