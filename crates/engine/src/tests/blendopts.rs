//! Blend Options (defaults with nothing selected, `object.blend.info`) and the Blend tool:
//! start points from clicked anchors, adding keys to a blend instead of nesting it (M8.18), and
//! Escape dropping the first pick.

use serde_json::{Value, json};
use vectorcraft_doc::live::{BlendOrientation, BlendSpacing, BlendSpec};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": w})).unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn spec(s: &Session, id: NodeId) -> (Vec<NodeId>, BlendSpec) {
    match node(s, id).kind {
        NodeKind::Blend { children, spec } => (children.iter().map(|c| c.id).collect(), *spec),
        _ => panic!("not a blend"),
    }
}

fn sel(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

#[test]
fn options_with_nothing_selected_set_what_new_blends_start_with() {
    let mut s = session();
    let info = s.execute("object.blend.info", &json!({})).unwrap();
    assert_eq!((info["target"].as_str(), info["spacing"].as_str()), (Some("defaults"), Some("smooth")));
    let depth = s.doc().unwrap().history.undo.len();
    let r = s.execute("object.blend.options", &json!({"spacing": "distance", "value": 20, "orientation": "path"})).unwrap();
    assert_eq!(r["defaults"], json!(true));
    assert_eq!(s.doc().unwrap().history.undo.len(), depth, "a tool setting, not an undo step");
    let info = s.execute("object.blend.info", &json!({})).unwrap();
    assert_eq!((info["spacing"].as_str(), info["distance"].as_f64(), info["orientation"].as_str()), (Some("distance"), Some(20.0), Some("path")));
    // New blends start with them; explicit params still win.
    let (a, b) = (rect(&mut s, 0.0, 0.0, 10.0), rect(&mut s, 100.0, 0.0, 10.0));
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.blend.make", &json!({})).unwrap());
    let (_, sp) = spec(&s, g);
    assert_eq!((sp.spacing, sp.orientation), (BlendSpacing::Distance(20.0), BlendOrientation::AlignToPath));
    s.execute("edit.undo", &json!({})).unwrap();
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 2})).unwrap());
    assert_eq!(spec(&s, g).1.spacing, BlendSpacing::Steps(2));
    // The preference survives a round trip through the preferences file.
    let back: Prefs = serde_json::from_value(s.prefs.to_json()).unwrap();
    assert_eq!(back.blend_options, s.prefs.blend_options);
    assert!(Prefs::default().to_json().get("blendOptions").is_none());
}

#[test]
fn info_reads_the_selected_blend() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0, 0.0, 10.0), rect(&mut s, 100.0, 0.0, 10.0));
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 4})).unwrap());
    let info = s.execute("object.blend.info", &json!({})).unwrap();
    assert_eq!(info["target"], "blend");
    assert_eq!(info["id"], json!(g.0));
    assert_eq!((info["spacing"].as_str(), info["steps"].as_u64()), (Some("steps"), Some(4)));
    assert_eq!(info["keys"], json!([a.0, b.0]));
}

#[test]
fn start_points_pair_the_clicked_anchors() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0, 0.0, 10.0), rect(&mut s, 100.0, 0.0, 10.0));
    // a starts at its top left, b at its bottom right: corners meet their opposites.
    let g = id_of(&s.execute("object.blend.make", &json!({"ids": [b.0, a.0], "starts": [2, 0], "steps": 1})).unwrap());
    let (keys, sp) = spec(&s, g);
    assert_eq!(keys, vec![a, b], "keys in paint order");
    assert_eq!(sp.starts, vec![Some(0), Some(2)], "each start stays with its object");
    let steps = vectorcraft_doc::live::expand_live(&node(&s, g));
    let mid = steps[1].geometric_bounds().unwrap();
    assert!(mid.height() < 1e-6, "the middle step collapses where opposite corners meet: {mid:?}");
    assert!(s.execute("object.blend.make", &json!({"ids": [a.0, b.0], "starts": "x"})).is_err());
    assert!(s.execute("object.blend.make", &json!({"ids": [a.0, b.0], "starts": [-1, 0]})).is_err());
    // Reverse Front to Back keeps each key's start.
    sel(&mut s, &[g]);
    s.execute("object.blend.reverseFrontToBack", &json!({})).unwrap();
    assert_eq!(spec(&s, g).1.starts, vec![Some(2), Some(0)]);
}

#[test]
fn a_blend_and_an_object_make_one_blend_with_more_keys() {
    let mut s = session();
    let (a, b, c) = (rect(&mut s, 0.0, 0.0, 10.0), rect(&mut s, 100.0, 0.0, 10.0), rect(&mut s, 200.0, 0.0, 10.0));
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 3})).unwrap());
    s.edit("name", |d, _| {
        d.node_mut(g).unwrap().name = Some("Ribbon".into());
        Ok(())
    })
    .unwrap();
    let g2 = id_of(&s.execute("object.blend.make", &json!({"ids": [g.0, c.0]})).unwrap());
    let n = node(&s, g2);
    let (keys, sp) = spec(&s, g2);
    assert_eq!(keys, vec![a, b, c], "no blend inside the blend");
    assert_eq!(sp.spacing, BlendSpacing::Steps(3), "the blend's options are kept");
    assert_eq!(n.name.as_deref(), Some("Ribbon"));
    assert!(s.doc().unwrap().doc.node(g).is_none());
    assert_eq!(vectorcraft_doc::live::expand_live(&n).len(), 3 + 2 * 3);
}

/// Choosing the Blend tool again (its button, W, `tool.select`) finishes the blend: the first
/// pick and the chain are dropped, and the document and the selection stay. Choosing another
/// tool again keeps its work: the Pen's open path stays open.
#[test]
fn choosing_the_blend_tool_again_finishes_the_blend() {
    let mut s = session();
    let (a, b, c) = (rect(&mut s, 0.0, 0.0, 40.0), rect(&mut s, 200.0, 0.0, 40.0), rect(&mut s, 400.0, 0.0, 40.0));
    let v = ViewInfo::default();
    s.select_tool("blend", v).unwrap();
    let click = |s: &mut Session, x: f64, y: f64| s.pointer(&PointerEvent::new(PointerKind::Down, x, y), ViewInfo::default()).unwrap();
    click(&mut s, 20.0, 20.0);
    assert!(s.tool_busy());
    let (layers, depth) = (s.doc().unwrap().doc.layers.clone(), s.doc().unwrap().history.undo.len());
    s.select_tool("blend", v).unwrap();
    assert!(!s.tool_busy(), "choosing the tool again drops the pick");
    assert_eq!((&s.doc().unwrap().doc.layers, s.doc().unwrap().history.undo.len()), (&layers, depth), "and changes nothing");
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    // The next two clicks blend b and c; a is not part of it.
    click(&mut s, 220.0, 20.0);
    assert_eq!(s.doc().unwrap().doc.layers, layers, "b is picked first");
    click(&mut s, 420.0, 20.0);
    let g = s.doc().unwrap().selection.objects[0];
    assert_eq!(spec(&s, g).0, vec![b, c]);
    // Chosen again after the blend is made: the next click starts a new blend instead of adding.
    s.select_tool("blend", v).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![g], "the blend stays selected");
    click(&mut s, 20.0, 20.0);
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    assert_eq!(spec(&s, g).0, vec![b, c], "a was not added");
    // The Pen chosen again goes on drawing its open path.
    s.select_tool("pen", v).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let pen = |s: &mut Session, x: f64, y: f64| {
        for kind in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(kind, x, y), ViewInfo::default()).unwrap();
        }
    };
    pen(&mut s, 100.0, 300.0);
    pen(&mut s, 200.0, 300.0);
    let p = s.doc().unwrap().selection.objects[0];
    s.select_tool("pen", v).unwrap();
    pen(&mut s, 300.0, 300.0);
    let anchors = match node(&s, p).kind {
        NodeKind::Path { path, .. } => path.subpaths[0].anchors.len(),
        _ => 0,
    };
    assert_eq!(anchors, 3, "the Pen keeps drawing the same path");
}

#[test]
fn the_blend_tool_picks_again_when_its_first_pick_is_deleted() {
    let mut s = session();
    let (a, b, c) = (rect(&mut s, 0.0, 0.0, 40.0), rect(&mut s, 200.0, 0.0, 40.0), rect(&mut s, 400.0, 0.0, 40.0));
    s.select_tool("blend", ViewInfo::default()).unwrap();
    let click = |s: &mut Session, x: f64, y: f64| s.pointer(&PointerEvent::new(PointerKind::Down, x, y), ViewInfo::default()).unwrap();
    click(&mut s, 20.0, 20.0);
    s.execute("edit.clear", &json!({"ids": [a.0]})).unwrap();
    let (layers, depth) = (s.doc().unwrap().doc.layers.clone(), s.doc().unwrap().history.undo.len());
    click(&mut s, 220.0, 20.0);
    assert_eq!(s.doc().unwrap().selection.objects, vec![b], "b is picked first");
    assert_eq!((&s.doc().unwrap().doc.layers, s.doc().unwrap().history.undo.len()), (&layers, depth), "the click changes no art");
    click(&mut s, 420.0, 20.0);
    let g = s.doc().unwrap().selection.objects[0];
    assert_eq!(spec(&s, g).0, vec![b, c]);
}

#[test]
fn the_blend_tool_blends_from_clicked_anchors_and_keeps_adding() {
    let mut s = session();
    let (a, b, c) = (rect(&mut s, 0.0, 0.0, 40.0), rect(&mut s, 200.0, 0.0, 40.0), rect(&mut s, 400.0, 0.0, 40.0));
    s.select_tool("blend", ViewInfo::default()).unwrap();
    let click = |s: &mut Session, x: f64, y: f64| s.pointer(&PointerEvent::new(PointerKind::Down, x, y), ViewInfo::default()).unwrap();
    // a's top right anchor, then b's interior.
    click(&mut s, 40.0, 0.0);
    click(&mut s, 220.0, 20.0);
    let g = s.doc().unwrap().selection.objects[0];
    let (keys, sp) = spec(&s, g);
    assert_eq!(keys, vec![a, b]);
    assert_eq!(sp.starts, vec![Some(1), None]);
    // The next object joins the same blend.
    click(&mut s, 420.0, 20.0);
    let g2 = s.doc().unwrap().selection.objects[0];
    assert_eq!(spec(&s, g2).0, vec![a, b, c]);
    // Alt-click asks for Blend Options.
    let r = s.pointer(&PointerEvent::new(PointerKind::Down, 600.0, 500.0).with_mods(Mods { alt: true, ..Default::default() }), ViewInfo::default());
    assert!(matches!(r.unwrap().as_slice(), [UiRequest::Dialog(k, _)] if k == "blendOptions"));
}

#[test]
fn escape_drops_the_blend_tools_first_pick() {
    let mut s = session();
    let (a, b, c) = (rect(&mut s, 0.0, 0.0, 40.0), rect(&mut s, 200.0, 0.0, 40.0), rect(&mut s, 400.0, 0.0, 40.0));
    s.select_tool("blend", ViewInfo::default()).unwrap();
    let click = |s: &mut Session, x: f64, y: f64| s.pointer(&PointerEvent::new(PointerKind::Down, x, y), ViewInfo::default()).unwrap();
    let esc = |s: &mut Session| s.tool_key(ToolKey::Escape, Mods::default(), ViewInfo::default()).unwrap();
    click(&mut s, 20.0, 20.0);
    assert!(s.tool_busy());
    let (layers, depth) = (s.doc().unwrap().doc.layers.clone(), s.doc().unwrap().history.undo.len());
    assert!(esc(&mut s).is_empty());
    assert!(!s.tool_busy(), "Escape drops the pick");
    assert_eq!((&s.doc().unwrap().doc.layers, s.doc().unwrap().history.undo.len()), (&layers, depth), "and changes nothing");
    // The next two clicks blend b and c; a is not part of it.
    click(&mut s, 220.0, 20.0);
    assert_eq!(s.doc().unwrap().doc.layers, layers, "b is picked first");
    click(&mut s, 420.0, 20.0);
    let g = s.doc().unwrap().selection.objects[0];
    assert_eq!(spec(&s, g).0, vec![b, c]);
    // Escape after the blend is made: the next click starts a new blend instead of adding to it.
    esc(&mut s);
    click(&mut s, 20.0, 20.0);
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    assert_eq!(spec(&s, g).0, vec![b, c], "a was not added");
}

#[test]
fn start_points_round_trip_and_old_blends_load() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0, 0.0, 10.0), rect(&mut s, 100.0, 0.0, 10.0));
    s.execute("object.blend.make", &json!({"ids": [a.0, b.0], "starts": [null, 3]})).unwrap();
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(d, false)).unwrap();
    assert_eq!(&back.layers, &d.layers);
    // A blend saved before start points has none.
    let old: BlendSpec = serde_json::from_value(json!({"spacing": {"steps": 4}, "orientation": "alignToPage"})).unwrap();
    assert!(old.starts.is_empty() && old.start(0).is_none());
    assert!(serde_json::to_value(&old).unwrap().get("starts").is_none());
}

#[test]
fn easing_is_set_validated_saved_and_old_blends_are_even() {
    let mut s = session();
    assert!(s.execute("object.blend.options", &json!({"ramp": 50})).is_err());
    let (a, b) = (rect(&mut s, 0.0, 0.0, 10.0), rect(&mut s, 100.0, 0.0, 10.0));
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 3, "easingType": "easeIn", "ramp": 40})).unwrap());
    assert_eq!(spec(&s, g).1.ease.ramp, 0.4);
    sel(&mut s, &[g]);
    for kind in ["easeIn", "easeOut", "easeInOut", "easeOutIn"] {
        s.execute("object.blend.options", &json!({"easingType": kind, "colorShift": -50})).unwrap();
        let info = s.execute("object.blend.info", &json!({})).unwrap();
        assert_eq!((&info["easingType"], &info["ramp"], &info["colorShift"]), (&json!(kind), &json!(40.0), &json!(-50.0)));
    }
    let before = spec(&s, g).1;
    let depth = s.doc().unwrap().history.undo.len();
    for bad in [json!({"ramp": 101}), json!({"ramp": -1}), json!({"colorShift": "x"}), json!({"colorShift": 1e308}), json!({"easingType": "unknown"})]
    {
        assert!(s.execute("object.blend.options", &bad).is_err(), "{bad}");
        assert_eq!(spec(&s, g).1, before);
        assert_eq!(s.doc().unwrap().history.undo.len(), depth);
    }
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(d, false)).unwrap();
    assert_eq!(&back.layers, &d.layers);
    s.execute("object.blend.options", &json!({"ramp": 0})).unwrap();
    let zero = spec(&s, g).1;
    let back: BlendSpec = serde_json::from_value(serde_json::to_value(&zero).unwrap()).unwrap();
    assert_eq!(back.ease, zero.ease, "a chosen Type survives at Ramp 0");
    let old: BlendSpec = serde_json::from_value(json!({"spacing": {"steps": 4}})).unwrap();
    assert!(old.ease.is_linear() && old.color_shift == 0.0);
    let v = serde_json::to_value(&old).unwrap();
    assert!(v.get("ease").is_none() && v.get("colorShift").is_none());
}

#[test]
fn a_document_with_bad_blend_easing_numbers_opens_clamped_while_commands_refuse() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0, 0.0, 10.0), rect(&mut s, 100.0, 0.0, 10.0));
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 3, "easingType": "easeOut", "ramp": 40, "colorShift": -50})).unwrap());
    let text = String::from_utf8(vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let blend_of = |doc: &vectorcraft_doc::Document| match doc.node(g).unwrap().kind.clone() {
        NodeKind::Blend { spec, .. } => *spec,
        _ => panic!("not a blend"),
    };
    for (ramp, shift, want) in [("12.5", "-9000.0", (1.0, -100.0)), ("-3.0", "250.0", (0.0, 100.0)), ("null", "\"NaN\"", (0.0, 0.0))] {
        let t = text.replacen("\"ramp\":0.4", &format!("\"ramp\":{ramp}"), 1).replacen("\"colorShift\":-50.0", &format!("\"colorShift\":{shift}"), 1);
        assert!(t != text && t.contains("\"ramp\":") && t.contains(shift), "the saved blend carries ramp and colorShift");
        let doc = vectorcraft_format::load(t.as_bytes()).expect("one bad blend number must not fail the document");
        let spec = blend_of(&doc);
        assert_eq!((spec.ease.ramp, spec.color_shift), want, "{ramp} / {shift}");
    }
    // The commands still refuse the same values and leave the blend alone.
    sel(&mut s, &[g]);
    let before = spec(&s, g).1;
    for bad in [json!({"ramp": 1250}), json!({"ramp": -3}), json!({"colorShift": 250}), json!({"colorShift": "NaN"}), json!({"colorShift": 1e308})] {
        assert!(s.execute("object.blend.options", &bad).is_err(), "{bad}");
        assert_eq!(spec(&s, g).1, before);
    }
}

#[test]
fn native_blend_easing_values_clamp_and_non_finite_become_default() {
    for (bad, ramp, shift) in [
        (json!({"ease": {"ramp": 1e308}}), 1.0, 0.0),
        (json!({"ease": {"ramp": -0.1}}), 0.0, 0.0),
        (json!({"colorShift": 101}), 0.0, 100.0),
        (json!({"colorShift": -101}), 0.0, -100.0),
        (json!({"ease": {"ramp": null}, "colorShift": "NaN"}), 0.0, 0.0),
    ] {
        let spec: BlendSpec = serde_json::from_value(bad.clone()).unwrap_or_else(|e| panic!("{bad}: {e}"));
        assert_eq!((spec.ease.ramp, spec.color_shift), (ramp, shift), "{bad}");
    }
}
