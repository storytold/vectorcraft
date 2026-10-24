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
