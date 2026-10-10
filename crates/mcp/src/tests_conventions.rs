use crate::{Backend, Headless, Server, tool_definitions};
use serde_json::{Value, json};

fn rpc(s: &mut Server, method: &str, params: Value) -> Value {
    s.handle(json!({"jsonrpc":"2.0","id":1,"method":method,"params":params})).unwrap()
}
fn call(s: &mut Server, name: &str, arguments: Value) -> Value {
    let r = rpc(s, "tools/call", json!({"name":name,"arguments":arguments}));
    assert!(r.get("error").is_none(), "{r}");
    r["result"].clone()
}
fn payload(r: &Value) -> Value {
    serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn conventions_catalog_and_strict_keys() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    let tools = tool_definitions();
    for name in [
        "command_list",
        "command_run",
        "command_batch",
        "doc_inspect",
        "render_preview",
        "ui_inspect",
        "ui_screenshot",
        "list_commands",
        "run_command",
    ] {
        assert!(tools.iter().any(|t| t["name"] == name), "missing {name}");
    }
    for t in tools {
        for hint in ["readOnlyHint", "destructiveHint", "idempotentHint", "openWorldHint"] {
            assert!(t["annotations"][hint].is_boolean(), "{t}");
        }
        let r = rpc(&mut s, "tools/call", json!({"name":t["name"],"arguments":{"typo":1}}));
        assert_eq!(r["error"]["code"], -32602, "{r}");
    }
    assert_eq!(rpc(&mut s, "ping", json!({}))["result"], json!({}));
}

#[test]
fn conventions_commands_batch_and_bounded_preview() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    let list = call(&mut s, "command_list", json!({"filter":"shape.rectangle","enabled_only":true}));
    assert_eq!(list["isError"], false, "{list}");
    assert!(payload(&list).as_array().unwrap().iter().any(|c| c["id"] == "shape.rectangle"));
    let steps = json!([{"id":"shape.rectangle","params":{"x":10,"y":20,"width":30,"height":40}}, {"id":"no.such.command"}, {"id":"shape.rectangle","params":{"x":0,"y":0,"width":20,"height":20}}]);
    for (stop, total) in [(true, 2), (false, 3)] {
        let r = call(&mut s, "command_batch", json!({"steps":steps,"stop_on_error":stop}));
        assert_eq!(r["isError"], true, "{r}");
        let p = payload(&r);
        assert_eq!(p["results"].as_array().unwrap().len(), total);
        assert_eq!(p["failed"], 1);
        assert_eq!(p["completed"], total - 1);
    }
    let before = call(&mut s, "doc_inspect", json!({}));
    let r = call(&mut s, "render_preview", json!({"max_side":64}));
    assert_eq!(r["isError"], false, "{r}");
    let png = vectorcraft_format::base64_decode(r["content"][0]["data"].as_str().unwrap()).unwrap();
    let img = image::load_from_memory(&png).unwrap();
    assert!(img.width() <= 64 && img.height() <= 64);
    assert_eq!(before, call(&mut s, "doc_inspect", json!({})));
    for max in [json!(0), json!(4097), json!("64")] {
        assert_eq!(call(&mut s, "render_preview", json!({"max_side":max}))["isError"], true);
    }
    for name in ["ui_inspect", "ui_screenshot"] {
        assert_eq!(call(&mut s, name, json!({}))["isError"], true);
    }
}

struct Panics;
impl Backend for Panics {
    fn call(&mut self, _: &str, _: Value) -> Result<Value, String> {
        panic!("backend bug")
    }
    fn has_ui(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        "test".into()
    }
}
#[test]
fn conventions_tool_panics_are_tool_errors() {
    let mut s = Server::new(Box::new(Panics));
    let r = call(&mut s, "list_commands", json!({}));
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"].as_str().unwrap().contains("backend bug"));
    assert_eq!(rpc(&mut s, "ping", json!({}))["result"], json!({}));
}

#[test]
fn conventions_resources_and_versioned_results() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    let modern = json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28"});
    for (method, extra, ttl) in [
        ("tools/list", json!({}), 600_000),
        ("resources/list", json!({}), 600_000),
        ("resources/templates/list", json!({}), 600_000),
        ("prompts/list", json!({}), 600_000),
        ("resources/read", json!({"uri":"vectorcraft://document"}), 0),
        ("resources/read", json!({"uri":"vectorcraft://commands"}), 0),
    ] {
        let legacy = rpc(&mut s, method, extra.clone());
        assert!(legacy["result"].is_object(), "{legacy}");
        assert!(legacy["result"].get("resultType").is_none());
        let mut params = extra.clone();
        params["_meta"] = modern.clone();
        let r = rpc(&mut s, method, params);
        assert_eq!(r["result"]["resultType"], "complete", "{r}");
        assert_eq!(r["result"]["ttlMs"], ttl);
        assert_eq!(r["result"]["cacheScope"], "private");
        let mut stripped = r["result"].clone();
        for key in ["resultType", "ttlMs", "cacheScope"] {
            stripped.as_object_mut().unwrap().remove(key);
        }
        assert_eq!(stripped, legacy["result"]);
        assert!(rpc(&mut s, method, extra)["result"].get("resultType").is_none());
    }
    let r = rpc(&mut s, "resources/list", json!({}));
    assert!(r["result"]["resources"].as_array().unwrap().iter().any(|r| r["uri"] == "vectorcraft://commands"));
    let commands = rpc(&mut s, "resources/read", json!({"uri":"vectorcraft://commands"}));
    let commands: Value = serde_json::from_str(commands["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(commands, payload(&call(&mut s, "command_list", json!({}))));
    assert_eq!(rpc(&mut s, "initialize", json!({"protocolVersion":"2026-07-28"}))["result"]["protocolVersion"], "2026-07-28");
    assert_eq!(rpc(&mut s, "tools/list", json!({}))["result"]["resultType"], "complete");
    rpc(&mut s, "initialize", json!({"protocolVersion":"2025-06-18"}));
    assert!(rpc(&mut s, "tools/list", json!({}))["result"].get("resultType").is_none());
}

/// #785: a batch step can use an earlier step's result (`"$N.path"`); a reference to nothing
/// fails that step.
#[test]
fn batch_steps_refer_to_earlier_results() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    let steps = json!([
        {"id":"text.create","params":{"x":10,"y":60,"text":"plain and bold words","size":30}},
        {"id":"shape.rectangle","params":{"x":0,"y":0,"width":20,"height":20}},
        {"id":"text.setRangeStyle","params":{"id":"$0.id","start":10,"end":14,"style":"Bold"}},
        {"id":"text.setRangeStyle","params":{"id":"$9.id","start":0,"end":1,"style":"Bold"}}
    ]);
    let r = call(&mut s, "command_batch", json!({"steps":steps,"stop_on_error":false}));
    let p = payload(&r);
    assert_eq!((p["completed"].clone(), p["failed"].clone()), (json!(3), json!(1)), "{p}");
    assert_eq!(p["results"][2]["result"]["id"], p["results"][0]["result"]["id"], "the text, not the selected rectangle");
    assert!(p["results"][3]["error"].as_str().unwrap().contains("$9.id"), "{p}");
}
