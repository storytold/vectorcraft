//! `draw_path` and `draw_shape` as `command_batch` steps, with the new objects' ids in the reply
//! (#2113).

use crate::{Headless, Server};
use serde_json::{Value, json};

fn call(s: &mut Server, name: &str, arguments: Value) -> Value {
    let r = s.handle(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}})).unwrap();
    assert!(r.get("error").is_none(), "{r}");
    r["result"].clone()
}

fn payload(r: &Value) -> Value {
    serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn node(s: &mut Server, id: &Value) -> Value {
    payload(&call(s, "run_command", json!({"command":"document.node","params":{"id":id}})))
}

#[test]
fn drawing_tools_run_as_batch_steps() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    let steps = json!([
        {"id":"draw_path","params":{"d":"M0 0 L100 0 L50 80 Z","fill":"#550505","stroke":"none"}},
        {"id":"draw_shape","params":{"shape":"rectangle","x":0,"y":0,"width":100,"height":80,"fill":"#000000"}},
        {"id":"object.setProps","params":{"id":"$0.id","name":"Facet 1"}},
        {"id":"draw_path","params":{"points":[[0,0],[10,10]],"stroke":"#c19a4b","strokeWidth":2}}
    ]);
    let r = call(&mut s, "command_batch", json!({"steps":steps,"atomic":true,"label":"Facets"}));
    assert_eq!(r["isError"], false, "{r}");
    let p = payload(&r);
    assert_eq!(p["completed"], 4, "{p}");
    let (path, rect, line) =
        (p["results"][0]["result"]["id"].clone(), p["results"][1]["result"]["id"].clone(), p["results"][3]["result"]["id"].clone());
    assert!(path.is_u64() && rect.is_u64() && line.is_u64(), "{p}");
    // The step results are the tools' own: the paint applied, the shape drawn.
    assert_eq!(p["results"][0]["result"]["paint"]["fill"], "#550505");
    assert_eq!(p["results"][1]["result"]["shape"], "rectangle");
    // A later step used the path's id.
    assert_eq!(node(&mut s, &path)["name"], "Facet 1");
    // The drawing steps' new objects, in order.
    assert_eq!(p["created"], json!([path, rect, line]));
    // One undo step takes the whole batch back.
    let undone = payload(&call(&mut s, "run_command", json!({"command":"edit.undo"})));
    assert_eq!(undone["undone"], "Facets");
    assert_eq!(call(&mut s, "run_command", json!({"command":"document.node","params":{"id":path}}))["isError"], true);
}

#[test]
fn a_batch_without_drawing_steps_lists_nothing_created() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    let r = call(&mut s, "command_batch", json!({"steps":[{"id":"shape.rectangle","params":{"x":0,"y":0,"width":5,"height":5}}]}));
    let p = payload(&r);
    assert_eq!(p["completed"], 1);
    assert_eq!(p["created"], json!([]));
}

/// Drawing steps check their arguments as `tools/call` does (#713): an unknown or misspelled one
/// fails the step and draws nothing.
#[test]
fn drawing_steps_refuse_unknown_arguments() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    let count = |s: &mut Server| payload(&call(s, "doc_inspect", json!({})))["objects"].as_u64().unwrap();
    let before = count(&mut s);
    for step in [
        json!({"id":"draw_shape","params":{"shape":"rectangle","x":0,"y":0,"w":5,"h":5}}),
        json!({"id":"draw_path","params":{"d":"M0 0 L5 5","colour":"#ff0000"}}),
        json!({"id":"draw_path","params":{}}),
        json!({"id":"draw_shape","params":{"shape":"hexagon"}}),
    ] {
        let r = call(&mut s, "command_batch", json!({"steps":[step]}));
        assert_eq!(r["isError"], true, "{step}: {r}");
        let p = payload(&r);
        assert_eq!(p["failed"], 1, "{step}: {p}");
        assert_eq!(p["created"], json!([]), "{step}: {p}");
    }
    let p = payload(&call(&mut s, "command_batch", json!({"steps":[{"id":"draw_shape","params":{"shape":"rectangle","x":0,"y":0,"w":5}}]})));
    assert!(p["results"][0]["error"].as_str().unwrap().contains("unknown argument `w` for `draw_shape`"), "{p}");
    assert_eq!(count(&mut s), before);
}

/// A drawing step's arguments can come from an earlier step's result.
#[test]
fn drawing_steps_take_earlier_results() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    let steps = json!([
        {"id":"shape.rectangle","params":{"x":40,"y":40,"width":20,"height":20}},
        {"id":"document.node","params":{"id":"$0.id","summary":true}},
        {"id":"draw_shape","params":{"shape":"rectangle","x":0,"y":0,"width":10,"height":10,"fill":"#ff0000"}},
        {"id":"draw_path","params":{"d":"M0 0 L5 5","stroke":"$2.paint.fill"}}
    ]);
    let p = payload(&call(&mut s, "command_batch", json!({"steps":steps})));
    assert_eq!(p["completed"], 4, "{p}");
    assert_eq!(p["results"][3]["result"]["paint"]["stroke"], "#ff0000", "{p}");
    assert_eq!(p["created"].as_array().unwrap().len(), 2);
}
