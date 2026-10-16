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
