//! `draw_path` / `draw_shape` style the object they draw in the same call: `name`, `opacity`,
//! `dash`, `cap`, `join` and `effects` (#2112).

use serde_json::{Value, json};
use vectorcraft_doc::{LineCap, LineJoin, Node, NodeId};

use crate::{Backend, Headless, Server, call_tool};

fn headless() -> Headless {
    let mut h = Headless::new();
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 400, "height": 400}})).unwrap();
    h
}

fn draw(h: &mut Headless, tool: &str, args: Value) -> (NodeId, Value) {
    let r = call_tool(h, tool, &args);
    assert!(!r.is_error, "{tool} {args}: {r:?}");
    let v: Value = serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap();
    (NodeId(v["id"].as_u64().unwrap()), v)
}

fn node(h: &Headless, id: NodeId) -> Node {
    h.session.doc().unwrap().doc.node(id).unwrap().clone()
}

fn objects(h: &Headless) -> usize {
    h.session.doc().unwrap().doc.node_count()
}

#[test]
fn draw_path_takes_name_opacity_dash_cap_join_and_effects() {
    let mut h = headless();
    let args = json!({
        "d": "M230 30 L230 300", "fill": "none", "stroke": "#c19a4b", "strokeWidth": 1.2,
        "dash": [4, 3], "cap": "round", "join": "bevel",
        "opacity": 60, "name": "Zip seam",
        "effects": [{"effect": "blur.gaussian", "params": {"radius": 6}}],
    });
    let (id, reply) = draw(&mut h, "draw_path", args);
    let n = node(&h, id);
    assert_eq!(n.name.as_deref(), Some("Zip seam"));
    assert!((n.opacity - 0.6).abs() < 1e-6, "{}", n.opacity);
    let st = n.appearance.stroke().unwrap();
    assert!((st.width - 1.2).abs() < 1e-9);
    assert_eq!(st.dash.as_ref().unwrap().pattern, vec![4.0, 3.0]);
    assert_eq!(st.cap, LineCap::Round);
    assert_eq!(st.join, LineJoin::Bevel);
    assert_eq!(n.appearance.effects.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["blur.gaussian"]);
    assert_eq!(n.appearance.effects[0].params["radius"], 6.0);
    // The reply says what was applied.
    assert_eq!(reply["name"], "Zip seam");
    assert_eq!(reply["opacity"].as_f64(), Some(60.0));
    assert_eq!(reply["paint"]["dash"], json!([4, 3]));
    assert_eq!(reply["effects"].as_array().unwrap().len(), 1);
}

#[test]
fn draw_shape_takes_the_same_options() {
    let mut h = headless();
    let args = json!({
        "shape": "ellipse", "x": 10, "y": 10, "width": 80, "height": 40, "fill": "#000000", "stroke": "none",
        "opacity": 35, "name": "Shadow", "effects": [{"effect": "blur.gaussian"}],
    });
    let (id, _) = draw(&mut h, "draw_shape", args);
    let n = node(&h, id);
    assert_eq!(n.name.as_deref(), Some("Shadow"));
    assert!((n.opacity - 0.35).abs() < 1e-6);
    assert_eq!(n.appearance.effects.len(), 1);
    // A dash alone gives a dashed stroke at the inherited weight.
    let (id, _) = draw(&mut h, "draw_shape", json!({"shape": "line", "x1": 0, "y1": 0, "x2": 100, "y2": 0, "dash": [2, 2], "cap": "butt"}));
    let n = node(&h, id);
    assert_eq!(n.appearance.stroke().unwrap().dash.as_ref().unwrap().pattern, vec![2.0, 2.0]);
    assert_eq!(n.appearance.stroke().unwrap().cap, LineCap::Butt);
}

#[test]
fn without_the_options_nothing_changes() {
    let mut h = headless();
    let (id, reply) = draw(&mut h, "draw_path", json!({"d": "M0 0 L100 0 L50 80 Z", "fill": "#550505", "stroke": "none"}));
    let n = node(&h, id);
    assert_eq!(n.name, None);
    assert_eq!(n.opacity, 1.0);
    assert!(n.appearance.effects.is_empty());
    for k in ["name", "opacity", "effects"] {
        assert!(reply.get(k).is_none(), "{k}: {reply}");
    }
}

/// A bad option is refused before anything is drawn: no half-styled object is left behind.
#[test]
fn bad_options_draw_nothing() {
    let mut h = headless();
    let before = objects(&h);
    for bad in [
        json!({"opacity": "half"}),
        json!({"opacity": 150}),
        json!({"opacity": -1}),
        json!({"name": 3}),
        json!({"dash": "4 3"}),
        json!({"dash": [4, "x"]}),
        json!({"cap": "pointy"}),
        json!({"join": 1}),
        json!({"effects": {"effect": "blur.gaussian"}}),
        json!({"effects": [{"params": {}}]}),
        json!({"effects": [{"effect": "blur.gaussian", "extra": 1}]}),
        json!({"effects": [{"effect": "no.suchEffect"}]}),
        json!({"effects": [{"effect": "blur.gaussian", "params": 6}]}),
    ] {
        for (tool, base) in
            [("draw_path", json!({"d": "M0 0 L10 10"})), ("draw_shape", json!({"shape": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10}))]
        {
            let mut args = base.clone();
            for (k, v) in bad.as_object().unwrap() {
                args[k] = v.clone();
            }
            let r = call_tool(&mut h, tool, &args);
            assert!(r.is_error, "{tool} {args} must fail");
            assert_eq!(objects(&h), before, "{tool} {args} drew something");
        }
    }
}

/// The new arguments are in the tools' schemas, so `tools/call` accepts them (#713 refuses
/// arguments a schema doesn't list).
#[test]
fn schemas_list_the_options() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    for (tool, args) in [
        ("draw_path", json!({"d": "M0 0 L10 10", "dash": [1, 1], "cap": "round", "join": "round", "opacity": 50, "name": "a", "effects": []})),
        (
            "draw_shape",
            json!({"shape": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10, "dash": null, "cap": "square", "join": "miter", "opacity": 50, "name": "b", "effects": []}),
        ),
    ] {
        let line = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": args}}).to_string();
        let v: Value = serde_json::from_str(&s.handle_line(&line).unwrap()).unwrap();
        assert!(v.get("error").is_none(), "{tool}: {v}");
        assert_eq!(v["result"]["isError"], false, "{tool}: {v}");
    }
}

/// A paint the engine refuses (a fill or stroke it can't read, a swatch the document lacks) fails
/// the whole call: no object, no undo step, nothing in the SVG (#3545).
#[test]
fn a_bad_paint_draws_nothing() {
    let mut h = headless();
    let (before, undo, svg) =
        (objects(&h), h.session.doc().unwrap().history.undo.len(), vectorcraft_svg::export(&h.session.doc().unwrap().doc, &Default::default()));
    for bad in [
        json!({"stroke": "currentColor"}),
        json!({"fill": "currentColor"}),
        json!({"fill": "#550505", "stroke": "not-a-colour"}),
        json!({"fill": {"swatch": "No Such Swatch"}}),
        json!({"stroke": "#000000", "strokeWidth": "thick"}),
    ] {
        for (tool, base) in
            [("draw_path", json!({"d": "M0 0 L10 10"})), ("draw_shape", json!({"shape": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10}))]
        {
            let mut args = base.clone();
            for (k, v) in bad.as_object().unwrap() {
                args[k] = v.clone();
            }
            let r = call_tool(&mut h, tool, &args);
            assert!(r.is_error, "{tool} {args} must fail");
            let st = h.session.doc().unwrap();
            assert_eq!(objects(&h), before, "{tool} {args} drew something");
            assert_eq!(st.history.undo.len(), undo, "{tool} {args} left an undo step");
            assert_eq!(vectorcraft_svg::export(&st.doc, &Default::default()), svg, "{tool} {args} changed the SVG");
        }
    }
    // A good call is one undo step, named as the drawing command names its own.
    for (tool, args, label) in [
        ("draw_path", json!({"d": "M0 0 L10 10", "stroke": "#c19a4b", "strokeWidth": 2, "opacity": 50}), "Pen"),
        ("draw_shape", json!({"shape": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10, "fill": "#000000", "stroke": "none"}), "Rectangle"),
    ] {
        let undo = h.session.doc().unwrap().history.undo.len();
        draw(&mut h, tool, args);
        let history = &h.session.doc().unwrap().history.undo;
        assert_eq!(history.len(), undo + 1, "{tool}");
        assert_eq!(history.last().unwrap().label, label);
    }
}

/// The opacity `draw_path` takes in percent reads back in the same scale: `doc_inspect` (and
/// the `vectorcraft://object/{id}` resource, the same summary) report `opacityPct` beside the
/// stored 0–1 `opacity`, so an agent that reuses what it read does not turn 55 % into 0.55 %.
#[test]
fn inspect_reports_opacity_in_the_percent_scale_the_drawing_tools_take() {
    let mut h = headless();
    let (id, reply) = draw(&mut h, "draw_path", json!({"d": "M10 10 L90 90", "opacity": 55}));
    assert_eq!(reply["opacity"], json!(55));
    let summary = |h: &mut Headless, id: NodeId| -> Value {
        let r = call_tool(h, "doc_inspect", &json!({"ids": [id.0]}));
        assert!(!r.is_error, "{r:?}");
        let v: Value = serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap();
        v["nodes"].as_array().unwrap().iter().find(|n| n["id"] == id.0).cloned().expect("the node")
    };
    let n = summary(&mut h, id);
    assert_eq!(n["opacityPct"], json!(55.0), "{n}");
    assert!((n["opacity"].as_f64().unwrap() - 0.55).abs() < 1e-6, "{n}");
    // The whole tree and the object resource say the same.
    let r = call_tool(&mut h, "inspect_document", &json!({}));
    let tree: Value = serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap();
    let in_tree = tree["layers"][0]["children"].as_array().unwrap().iter().find(|c| c["id"] == id.0).expect("the path in its layer");
    assert_eq!(in_tree["opacityPct"], json!(55.0), "{in_tree}");
    let res = crate::resources::read(&mut h, &format!("vectorcraft://object/{}", id.0)).unwrap();
    assert_eq!((res["opacityPct"].clone(), res["opacity"].as_f64().map(|o| (o * 100.0).round())), (json!(55.0), Some(55.0)), "{res}");
    // Writing `opacityPct` back, as the descriptions say, keeps the object as it was.
    let r = call_tool(&mut h, "run_command", &json!({"command": "object.setProps", "params": {"id": id.0, "opacity": n["opacityPct"]}}));
    assert!(!r.is_error, "{r:?}");
    assert!((node(&h, id).opacity - 0.55).abs() < 1e-6);
    // A fully opaque object reports neither field, so the two never disagree.
    let (full, _) = draw(&mut h, "draw_path", json!({"d": "M0 0 L5 5"}));
    let n = summary(&mut h, full);
    assert!(n.get("opacity").is_none() && n.get("opacityPct").is_none(), "{n}");
}
