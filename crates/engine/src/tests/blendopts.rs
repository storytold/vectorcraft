//! Blend Options (defaults with nothing selected, `object.blend.info`) and the Blend tool:
//! start points from clicked anchors, adding keys to a blend instead of nesting it (M8.18), and
//! Escape, or an edit made outside the tool, dropping the first pick.

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
fn an_edit_undo_or_redo_made_outside_the_blend_tool_drops_its_first_pick() {
    for case in ["edit", "undo", "redo", "group"] {
        let undo = case == "undo";
        let mut s = session();
        let (a, b, c) = (rect(&mut s, 0.0, 0.0, 40.0), rect(&mut s, 200.0, 0.0, 40.0), rect(&mut s, 400.0, 0.0, 40.0));
        if case == "redo" {
            s.execute("paint.setFill", &json!({"ids": [c.0], "color": "#ff0000"})).unwrap();
            s.execute("edit.undo", &json!({})).unwrap();
        }
        // In an open undo group, the edit after the pick is merged into the group's step.
        if case == "group" {
            s.begin_undo_group();
            s.execute("paint.setFill", &json!({"ids": [c.0], "color": "#00ff00"})).unwrap();
        }
        s.select_tool("blend", ViewInfo::default()).unwrap();
        let click = |s: &mut Session, x: f64, y: f64| s.pointer(&PointerEvent::new(PointerKind::Down, x, y), ViewInfo::default()).unwrap();
        // Undo takes away c, the object picked first; an edit or a redo recolors c, which is not picked.
        click(&mut s, if undo { 420.0 } else { 20.0 }, 20.0);
        assert!(!s.overlays(ViewInfo::default()).is_empty());
        match case {
            "undo" => s.execute("edit.undo", &json!({})).unwrap(),
            "redo" => s.execute("edit.redo", &json!({})).unwrap(),
            _ => s.execute("paint.setFill", &json!({"ids": [c.0], "color": "#ff0000"})).unwrap(),
        };
        assert!(s.overlays(ViewInfo::default()).is_empty(), "{case}: the pick is not marked");
        let (layers, depth) = (s.doc().unwrap().doc.layers.clone(), s.doc().unwrap().history.undo.len());
        click(&mut s, 220.0, 20.0);
        assert_eq!(s.doc().unwrap().selection.objects, vec![b], "{case}: b is picked first");
        assert_eq!((&s.doc().unwrap().doc.layers, s.doc().unwrap().history.undo.len()), (&layers, depth), "{case}: the click changes no art");
        click(&mut s, if undo { 20.0 } else { 420.0 }, 20.0);
        let g = s.doc().unwrap().selection.objects[0];
        assert_eq!(spec(&s, g).0, if undo { vec![a, b] } else { vec![b, c] }, "{case}");
    }
}

#[test]
fn two_picks_with_no_edit_between_them_blend() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0, 0.0, 40.0), rect(&mut s, 200.0, 0.0, 40.0));
    s.select_tool("blend", ViewInfo::default()).unwrap();
    let click = |s: &mut Session, x: f64, y: f64| s.pointer(&PointerEvent::new(PointerKind::Down, x, y), ViewInfo::default()).unwrap();
    click(&mut s, 20.0, 20.0);
    // Drawing the overlays between the picks does not drop the first one.
    assert!(!s.overlays(ViewInfo::default()).is_empty());
    click(&mut s, 220.0, 20.0);
    let g = s.doc().unwrap().selection.objects[0];
    assert_eq!(spec(&s, g).0, vec![a, b]);
}

#[test]
fn removing_the_blend_the_blend_tool_adds_to_ends_adding() {
    let mut s = session();
    let (a, b, c) = (rect(&mut s, 0.0, 0.0, 40.0), rect(&mut s, 200.0, 0.0, 40.0), rect(&mut s, 400.0, 0.0, 40.0));
    let (d, e) = (rect(&mut s, 0.0, 300.0, 40.0), rect(&mut s, 200.0, 300.0, 40.0));
    let other = id_of(&s.execute("object.blend.make", &json!({"ids": [d.0, e.0]})).unwrap());
    s.select_tool("blend", ViewInfo::default()).unwrap();
    let click = |s: &mut Session, x: f64, y: f64| s.pointer(&PointerEvent::new(PointerKind::Down, x, y), ViewInfo::default()).unwrap();
    click(&mut s, 20.0, 20.0);
    click(&mut s, 220.0, 20.0);
    let g = s.doc().unwrap().selection.objects[0];
    s.execute("edit.clear", &json!({"ids": [g.0]})).unwrap();
    sel(&mut s, &[other]);
    click(&mut s, 420.0, 20.0);
    assert_eq!(s.doc().unwrap().selection.objects, vec![c], "c is picked first");
    assert_eq!(spec(&s, other).0, vec![d, e], "c is not added to the selected blend");
    assert!([a, b, g].iter().all(|id| s.doc().unwrap().doc.node(*id).is_none()), "the blend went with its keys");
}

#[test]
fn a_selection_change_keeps_the_blend_tools_first_pick() {
    for drag in [false, true] {
        let mut s = session();
        let (a, b, c) = (rect(&mut s, 0.0, 0.0, 40.0), rect(&mut s, 200.0, 0.0, 40.0), rect(&mut s, 400.0, 0.0, 40.0));
        let view = ViewInfo::default();
        s.select_tool("blend", view).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Down, 20.0, 20.0), view).unwrap();
        sel(&mut s, &[c]);
        // Cmd lends the selection tool: a click on c selects it, a drag moves it (an edit).
        let cmd = Mods { cmd: true, ..Mods::default() };
        let to = if drag { 440.0 } else { 420.0 };
        s.pointer(&PointerEvent::new(PointerKind::Down, 420.0, 20.0).with_mods(cmd), view).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Drag, to, 20.0).with_mods(cmd), view).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, to, 20.0).with_mods(cmd), view).unwrap();
        assert_eq!(s.tool_id(), "blend");
        assert_eq!(s.overlays(view).is_empty(), drag, "drag {drag}");
        s.pointer(&PointerEvent::new(PointerKind::Down, 220.0, 20.0), view).unwrap();
        if drag {
            assert_eq!(s.doc().unwrap().selection.objects, vec![b], "the drag dropped the pick");
        } else {
            let g = s.doc().unwrap().selection.objects[0];
            assert_eq!(spec(&s, g).0, vec![a, b]);
        }
    }
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

/// `object.blend.setEasing` sets one blend's easing and color shift, records no undo step when
/// nothing changes, and refuses bad values, a blend whose steps follow no spine (easing only), a
/// hidden or locked blend and a target that is not a blend, leaving the blend and the history alone.
#[test]
fn set_easing_edits_one_blend_and_refuses_what_it_cannot_set() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0, 0.0, 40.0), rect(&mut s, 200.0, 0.0, 40.0));
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 3})).unwrap());
    let depth = |s: &Session| s.doc().unwrap().history.undo.len();
    let d0 = depth(&s);
    // Align to Page along the straight lines between the keys: the steps follow no spine.
    for bad in [json!({"ramp": 30}), json!({"easingType": "easeOut"})] {
        let e = s.execute("object.blend.setEasing", &bad).unwrap_err().to_string();
        assert!(e.contains("the blend has no spine"), "{e}");
    }
    assert_eq!(depth(&s), d0);
    let r = s.execute("object.blend.setEasing", &json!({"colorShift": 25})).unwrap();
    assert_eq!(r, json!({"id": g.0, "easingType": "easeIn", "ramp": 0.0, "colorShift": 25.0, "changed": true}));
    assert_eq!((spec(&s, g).1.color_shift, depth(&s)), (25.0, d0 + 1));
    let r = s.execute("object.blend.setEasing", &json!({"colorShift": 25})).unwrap();
    assert_eq!((&r["changed"], depth(&s)), (&json!(false), d0 + 1), "an unchanged value records no undo step");
    // Along the spine each of the four types keeps its kind through a ramp change and its undo.
    s.execute("object.blend.options", &json!({"orientation": "path"})).unwrap();
    for kind in ["easeIn", "easeOut", "easeInOut", "easeOutIn"] {
        s.execute("object.blend.setEasing", &json!({"easingType": kind, "ramp": 60})).unwrap();
        let r = s.execute("object.blend.setEasing", &json!({"ramp": 20})).unwrap();
        assert_eq!((&r["easingType"], &r["ramp"], &r["colorShift"]), (&json!(kind), &json!(20.0), &json!(25.0)));
        s.execute("edit.undo", &json!({})).unwrap();
        let sp = spec(&s, g).1;
        assert_eq!((sp.ease.kind.id(), sp.ease.ramp, sp.color_shift), (kind, 0.6, 25.0));
    }
    let before = spec(&s, g).1;
    let d1 = depth(&s);
    for bad in [
        json!({"ramp": 101}),
        json!({"ramp": -1}),
        json!({"ramp": "x"}),
        json!({"colorShift": 1e308}),
        json!({"colorShift": -101}),
        json!({"easingType": "sideways"}),
        json!({"id": "abc", "ramp": 10}),
        json!({"id": a.0, "ramp": 10}),
    ] {
        assert!(s.execute("object.blend.setEasing", &bad).is_err(), "{bad}");
        assert_eq!((spec(&s, g).1, depth(&s)), (before.clone(), d1), "{bad}");
    }
    // An explicit id works with nothing selected; nothing selected and no id is an error.
    s.execute("select.none", &json!({})).unwrap();
    assert!(s.execute("object.blend.setEasing", &json!({"ramp": 10})).is_err());
    s.execute("object.blend.setEasing", &json!({"id": g.0, "ramp": 10})).unwrap();
    assert_eq!(spec(&s, g).1.ease.ramp, 0.1);
    // A hidden or locked blend is refused.
    for hide in [true, false] {
        s.edit("state", |d, _| {
            let n = d.node_mut(g).unwrap();
            (n.visible, n.locked) = (!hide, !hide);
            Ok(())
        })
        .unwrap();
        let (before, d2) = (spec(&s, g).1, depth(&s));
        let e = s.execute("object.blend.setEasing", &json!({"id": g.0, "colorShift": 5})).unwrap_err().to_string();
        assert!(e.contains("the object is hidden or locked"), "{e}");
        assert_eq!((spec(&s, g).1, depth(&s)), (before, d2));
    }
    // A blend made without a spine takes no easing, Align to Path included; its color shift still
    // changes.
    let (c, d) = (rect(&mut s, 0.0, 300.0, 40.0), rect(&mut s, 200.0, 300.0, 40.0));
    sel(&mut s, &[c, d]);
    let n = id_of(&s.execute("object.blend.make", &json!({"createSpine": "never", "orientation": "path"})).unwrap());
    let d3 = depth(&s);
    let e = s.execute("object.blend.setEasing", &json!({"id": n.0, "ramp": 30})).unwrap_err().to_string();
    assert!(e.contains("the blend has no spine"), "{e}");
    assert_eq!(depth(&s), d3);
    s.execute("object.blend.setEasing", &json!({"id": n.0, "colorShift": 10})).unwrap();
    assert_eq!(spec(&s, n).1.color_shift, 10.0);
}

/// Dragging a blend widget with the Selection tool or the Blend tool is one undo step; a click, a
/// drag that ends at its starting value and a drag canceled with Escape record none, and Escape
/// puts the value back.
#[test]
fn dragging_a_blend_widget_is_one_undo_step_and_a_click_or_escape_none() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0, 0.0, 40.0), rect(&mut s, 200.0, 0.0, 40.0));
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 3, "orientation": "path", "easingType": "easeOut"})).unwrap());
    let v = ViewInfo::default();
    let depth = |s: &Session| s.doc().unwrap().history.undo.len();
    let ptr = |s: &mut Session, kind: PointerKind, x: f64, y: f64| {
        s.pointer(&PointerEvent::new(kind, x, y), ViewInfo::default()).unwrap();
    };
    s.select_tool("selection", v).unwrap();
    sel(&mut s, &[g]);
    let d0 = depth(&s);
    // The Easing widget sits 16 px below the spine's halfway point (120, 20).
    ptr(&mut s, PointerKind::Down, 120.0, 36.0);
    ptr(&mut s, PointerKind::Drag, 135.0, 36.0);
    ptr(&mut s, PointerKind::Drag, 150.0, 36.0);
    ptr(&mut s, PointerKind::Up, 150.0, 36.0);
    let sp = spec(&s, g).1;
    assert_eq!((sp.ease.kind.id(), sp.ease.ramp, sp.color_shift, depth(&s)), ("easeOut", 0.3, 0.0, d0 + 1));
    assert_eq!(s.doc().unwrap().selection.objects, vec![g], "the blend stays selected");
    // A click, and a drag back to where it started.
    ptr(&mut s, PointerKind::Down, 120.0, 36.0);
    ptr(&mut s, PointerKind::Up, 120.0, 36.0);
    ptr(&mut s, PointerKind::Down, 120.0, 36.0);
    ptr(&mut s, PointerKind::Drag, 160.0, 36.0);
    ptr(&mut s, PointerKind::Drag, 120.0, 36.0);
    ptr(&mut s, PointerKind::Up, 120.0, 36.0);
    assert_eq!((spec(&s, g).1.ease.ramp, depth(&s)), (0.3, d0 + 1));
    // Escape during a drag puts the value back.
    ptr(&mut s, PointerKind::Down, 120.0, 36.0);
    ptr(&mut s, PointerKind::Drag, 170.0, 36.0);
    assert_eq!(spec(&s, g).1.ease.ramp, 0.8, "the drag previews");
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    ptr(&mut s, PointerKind::Up, 170.0, 36.0);
    assert_eq!((spec(&s, g).1.ease.ramp, depth(&s)), (0.3, d0 + 1));
    // The Blend tool drags the Appearance widget, 16 px above: the color shift alone.
    s.select_tool("blend", v).unwrap();
    sel(&mut s, &[g]);
    ptr(&mut s, PointerKind::Down, 120.0, 4.0);
    ptr(&mut s, PointerKind::Drag, 100.0, 4.0);
    ptr(&mut s, PointerKind::Up, 100.0, 4.0);
    let sp = spec(&s, g).1;
    assert_eq!((sp.ease.kind.id(), sp.ease.ramp, sp.color_shift, depth(&s)), ("easeOut", 0.3, -20.0, d0 + 2));
    assert_eq!(s.doc().unwrap().selection.objects, vec![g]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(spec(&s, g).1.color_shift, 0.0);
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

fn create_spine(s: &mut Session, p: Value) -> Value {
    s.execute("object.blend.createSpine", &p).unwrap()["mode"].clone()
}

/// Two squares 100 pt apart blended with `p`; the blend.
fn make(s: &mut Session, y: f64, p: Value) -> NodeId {
    let (a, b) = (rect(s, 0.0, y, 10.0), rect(s, 100.0, y, 10.0));
    let mut p = p;
    p["ids"] = json!([a.0, b.0]);
    p["steps"] = json!(3);
    id_of(&s.execute("object.blend.make", &p).unwrap())
}

#[test]
fn create_spine_sets_whether_new_blends_get_a_spine() {
    let mut s = session();
    assert_eq!(create_spine(&mut s, json!({})), "auto", "Auto until one is chosen");
    let spined = make(&mut s, 0.0, json!({}));
    assert!(!spec(&s, spined).1.no_spine);
    let depth = s.doc().unwrap().history.undo.len();
    assert_eq!(create_spine(&mut s, json!({"mode": "never"})), "never");
    assert_eq!(s.doc().unwrap().history.undo.len(), depth, "a tool setting, not an undo step");
    assert_eq!(create_spine(&mut s, json!({})), "never");
    // The Blend Options set with nothing selected keep it, and it keeps them.
    s.execute("select.none", &json!({})).unwrap();
    s.execute("object.blend.options", &json!({"steps": 7})).unwrap();
    assert_eq!(create_spine(&mut s, json!({})), "never");
    assert_eq!(s.prefs.blend_options.unwrap().spacing, BlendSpacing::Steps(7));
    // A new blend has no spine; the blend made before keeps its own.
    let bare = make(&mut s, 50.0, json!({}));
    assert!(spec(&s, bare).1.no_spine);
    assert!(!spec(&s, spined).1.no_spine, "blends already made keep their spine");
    sel(&mut s, &[bare]);
    let info = s.execute("object.blend.info", &json!({})).unwrap();
    assert_eq!(info["hasSpine"], json!(false));
    assert!(info.get("spine").is_none(), "{info}");
    sel(&mut s, &[spined]);
    assert_eq!(s.execute("object.blend.info", &json!({})).unwrap()["hasSpine"], json!(true));
    // `createSpine` on Make chooses for that blend only.
    let forced = make(&mut s, 100.0, json!({"createSpine": "always"}));
    assert!(!spec(&s, forced).1.no_spine);
    assert_eq!(create_spine(&mut s, json!({})), "never");
    assert_eq!(create_spine(&mut s, json!({"mode": "always"})), "always");
    let always = make(&mut s, 150.0, json!({}));
    assert!(!spec(&s, always).1.no_spine);
    assert!(spec(&s, bare).1.no_spine, "blends already made keep having no spine");
    let none = make(&mut s, 200.0, json!({"createSpine": "never"}));
    assert!(spec(&s, none).1.no_spine);
    // The setting survives a round trip through the preferences file; Auto is not written.
    let back: Prefs = serde_json::from_value(s.prefs.to_json()).unwrap();
    assert_eq!(back.blend_options.unwrap().create_spine, vectorcraft_doc::live::BlendSpineMode::Always);
    assert_eq!(s.prefs.to_json()["blendOptions"]["createSpine"], json!("always"));
    create_spine(&mut s, json!({"mode": "auto"}));
    assert!(s.prefs.to_json()["blendOptions"].get("createSpine").is_none());
    let mut odd = s.prefs.to_json();
    odd["blendOptions"]["createSpine"] = json!("sometimes");
    let unknown: Prefs = serde_json::from_value(odd).unwrap();
    assert_eq!(unknown.blend_options.unwrap().create_spine, vectorcraft_doc::live::BlendSpineMode::Auto, "an unknown value reads as Auto");
}

#[test]
fn create_spine_refuses_bad_values_and_changes_nothing() {
    let mut s = session();
    create_spine(&mut s, json!({"mode": "never"}));
    let prefs = s.prefs.blend_options;
    for bad in [json!({"mode": "sometimes"}), json!({"mode": 1}), json!({"mode": true}), json!({"mode": ["never"]}), json!({"mode": "Never"})] {
        let e = s.execute("object.blend.createSpine", &bad).unwrap_err().to_string();
        assert!(e.contains("`mode` must be one of auto, always, never"), "{bad}: {e}");
        assert_eq!(s.prefs.blend_options, prefs, "{bad}");
    }
    let (a, b) = (rect(&mut s, 0.0, 0.0, 10.0), rect(&mut s, 100.0, 0.0, 10.0));
    let depth = s.doc().unwrap().history.undo.len();
    for bad in [json!({"createSpine": "x"}), json!({"createSpine": 0}), json!({"createSpine": "never", "easingType": "easeIn"}), json!({"ramp": 30})]
    {
        let mut p = bad.clone();
        p["ids"] = json!([a.0, b.0]);
        assert!(s.execute("object.blend.make", &p).is_err(), "{bad}");
        assert_eq!(s.doc().unwrap().history.undo.len(), depth, "{bad}: nothing made");
    }
    assert!(s.execute("object.blend.createSpine", &json!({"spine": "never"})).is_err(), "an unknown key");
}

#[test]
fn a_blend_without_a_spine_has_none_to_edit_reverse_ease_or_release() {
    let mut s = session();
    let g = make(&mut s, 0.0, json!({"createSpine": "never"}));
    sel(&mut s, &[g]);
    let before = node(&s, g);
    let depth = s.doc().unwrap().history.undo.len();
    // Reverse Spine, easing and the spine point commands refuse it and change nothing.
    let refused = [
        ("object.blend.reverseSpine", json!({})),
        ("object.blend.options", json!({"easingType": "easeOut"})),
        ("object.blend.options", json!({"ramp": 40})),
        ("object.blend.spine.addAnchor", json!({"x": 50, "y": 5})),
        ("object.blend.spine.moveAnchor", json!({"anchor": 0, "x": 0, "y": 50})),
    ];
    for (cmd, p) in refused {
        let e = s.execute(cmd, &p).unwrap_err().to_string();
        assert!(e.contains("the blend has no spine"), "{cmd}: {e}");
        assert_eq!((node(&s, g), s.doc().unwrap().history.undo.len()), (before.clone(), depth), "{cmd}");
    }
    // Spacing, Orientation and Shift still apply; Align to Path makes no spine.
    s.execute("object.blend.options", &json!({"steps": 2, "orientation": "path", "colorShift": 20})).unwrap();
    let sp = spec(&s, g).1;
    assert_eq!((sp.spacing, sp.orientation, sp.color_shift, sp.no_spine), (BlendSpacing::Steps(2), BlendOrientation::AlignToPath, 20.0, true));
    assert!(s.execute("object.blend.info", &json!({})).unwrap().get("spine").is_none());
    // Taking in another key keeps it without a spine, whatever Create Spine says.
    let c = rect(&mut s, 200.0, 0.0, 10.0);
    sel(&mut s, &[g, c]);
    create_spine(&mut s, json!({"mode": "always"}));
    let g = id_of(&s.execute("object.blend.make", &json!({})).unwrap());
    assert_eq!(spec(&s, g).0.len(), 3);
    assert!(spec(&s, g).1.no_spine && spec(&s, g).1.spine.is_none());
    // Release gives back the keys and no spine path.
    sel(&mut s, &[g]);
    let r = s.execute("object.blend.release", &json!({})).unwrap();
    assert_eq!((r["ids"].as_array().map(Vec::len), &r["spines"]), (Some(3), &json!([])));
    s.execute("edit.undo", &json!({})).unwrap();
    // Replace Spine gives it one: then it reverses and eases like any other.
    let line = id_of(&s.execute("path.create", &json!({"d": "M0 100 L200 150"})).unwrap());
    sel(&mut s, &[g, line]);
    s.execute("object.blend.replaceSpine", &json!({})).unwrap();
    let sp = spec(&s, g).1;
    assert!(!sp.no_spine && sp.spine.is_some());
    let info = s.execute("object.blend.info", &json!({})).unwrap();
    assert_eq!((&info["hasSpine"], &info["spine"]["explicit"]), (&json!(true), &json!(true)));
    s.execute("object.blend.reverseSpine", &json!({})).unwrap();
    s.execute("object.blend.options", &json!({"easingType": "easeOut", "ramp": 40})).unwrap();
    assert_eq!(spec(&s, g).1.ease.ramp, 0.4);
}

#[test]
fn a_blend_without_a_spine_draws_its_steps_where_one_with_a_spine_does() {
    let mut s = session();
    for orientation in ["page", "path"] {
        let spined = make(&mut s, 0.0, json!({"orientation": orientation}));
        let bare = make(&mut s, 0.0, json!({"orientation": orientation, "createSpine": "never"}));
        let boxes = |g: NodeId| -> Vec<_> { vectorcraft_doc::live::expand_live(&node(&s, g)).iter().map(|n| n.geometric_bounds()).collect() };
        assert_eq!(boxes(bare), boxes(spined), "{orientation}");
        assert!(boxes(bare).len() >= 3, "the three steps at least");
    }
    // Easing stored on a blend without a spine (a hand-edited file) leaves its steps evenly spaced.
    let bare = make(&mut s, 300.0, json!({"orientation": "path", "createSpine": "never"}));
    let even = vectorcraft_doc::live::expand_live(&node(&s, bare));
    let mut eased = node(&s, bare);
    if let NodeKind::Blend { spec, .. } = &mut eased.kind {
        spec.ease = vectorcraft_doc::live::BlendEase { kind: vectorcraft_doc::live::BlendEaseType::In, ramp: 1.0 };
    }
    let boxes = |v: &[Node]| -> Vec<_> { v.iter().map(|n| n.geometric_bounds()).collect() };
    assert_eq!(boxes(&vectorcraft_doc::live::expand_live(&eased)), boxes(&even));
}

#[test]
fn whether_a_blend_has_a_spine_is_saved_and_old_blends_have_one() {
    let mut s = session();
    let bare = make(&mut s, 0.0, json!({"createSpine": "never"}));
    let spined = make(&mut s, 50.0, json!({}));
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(d, false)).unwrap();
    assert_eq!(&back.layers, &d.layers);
    let saved = |g: NodeId| match &back.node(g).unwrap().kind {
        NodeKind::Blend { spec, .. } => spec.no_spine,
        _ => panic!("not a blend"),
    };
    assert!(saved(bare) && !saved(spined));
    assert_eq!(serde_json::to_value(spec(&s, bare).1).unwrap()["noSpine"], json!(true));
    assert!(serde_json::to_value(spec(&s, spined).1).unwrap().get("noSpine").is_none(), "a spine is not written");
    let old: BlendSpec = serde_json::from_value(json!({"spacing": {"steps": 4}})).unwrap();
    assert!(!old.no_spine, "a blend saved before Create Spine has a spine");
    for odd in [json!("yes"), json!(1), json!(null)] {
        let v: BlendSpec = serde_json::from_value(json!({"noSpine": odd.clone()})).unwrap_or_else(|e| panic!("{odd}: {e}"));
        assert!(!v.no_spine, "{odd}");
    }
}
