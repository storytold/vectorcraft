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

/// #2173: the command listings take `query` (as `filter`), a `menu` filter, and `brief`, which
/// leaves out the long params text, so the catalogue fits a client's output limit.
#[test]
fn command_listings_take_query_menu_and_brief() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    let ids = |v: &Value| v.as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let by_filter = payload(&call(&mut s, "command_list", json!({"filter":"rectangle"})));
    let by_query = payload(&call(&mut s, "command_list", json!({"query":"rectangle"})));
    assert!(ids(&by_filter).contains(&"shape.rectangle".to_string()));
    assert_eq!(by_filter, by_query);
    // `menu` keeps the commands under that menu path.
    let path_menu = payload(&call(&mut s, "command_list", json!({"menu":"object > path"})));
    assert!(ids(&path_menu).contains(&"object.path.addAnchorPoints".to_string()), "{path_menu}");
    for c in path_menu.as_array().unwrap() {
        let menu: Vec<&str> = c["menu"].as_array().unwrap().iter().map(|m| m.as_str().unwrap()).collect();
        assert_eq!(menu.get(..2), Some(&["Object", "Path"][..]), "{c}");
    }
    // `brief` drops the params text and nothing else.
    let full = payload(&call(&mut s, "command_list", json!({})));
    let brief = payload(&call(&mut s, "command_list", json!({"brief":true})));
    assert_eq!(ids(&full), ids(&brief));
    assert!(full.as_array().unwrap().iter().any(|c| c.get("params").is_some()));
    assert!(brief.as_array().unwrap().iter().all(|c| c.get("params").is_none() && c.get("label").is_some()));
    assert!(brief.to_string().len() * 2 < full.to_string().len(), "brief is much shorter");
    // The other listing takes them too.
    let r = payload(&call(&mut s, "list_commands", json!({"query":"rectangle","brief":true,"menu":""})));
    assert_eq!(ids(&r["commands"]), ids(&by_filter));
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
    // atomic: the whole batch is one undo step, named by `label` (#1163).
    let undo_steps = |s: &mut Server| payload(&call(s, "doc_inspect", json!({"depth":0})))["history"].as_array().map_or(0, Vec::len);
    let before = undo_steps(&mut s);
    let two = json!([{"id":"shape.rectangle","params":{"x":0,"y":0,"width":5,"height":5}}, {"id":"shape.ellipse","params":{"x":9,"y":0,"width":5,"height":5}}]);
    let r = call(&mut s, "command_batch", json!({"steps":two,"atomic":true,"label":"Agent build"}));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(undo_steps(&mut s), before + 1);
    assert_eq!(payload(&call(&mut s, "run_command", json!({"command":"edit.undo"})))["undone"], "Agent build");
    assert_eq!(undo_steps(&mut s), before);
    for bad in [json!({"steps":two,"label":"x"}), json!({"steps":two,"atomic":"yes"}), json!({"steps":two,"atomic":true,"label":""})] {
        assert_eq!(call(&mut s, "command_batch", bad.clone())["isError"], true, "{bad}");
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
    // Another artboard, three times wider than tall, previews by its index (#1163).
    let r = call(&mut s, "run_command", json!({"command":"artboard.new","params":{"x":2000,"y":0,"width":300,"height":100}}));
    assert_eq!(r["isError"], false, "{r}");
    let r = call(&mut s, "render_preview", json!({"max_side":60,"artboard":1}));
    assert_eq!(r["isError"], false, "{r}");
    let png = vectorcraft_format::base64_decode(r["content"][0]["data"].as_str().unwrap()).unwrap();
    let img = image::load_from_memory(&png).unwrap();
    assert_eq!((img.width(), img.height()), (60, 20));
    for board in [json!(2), json!(-1), json!("1")] {
        assert_eq!(call(&mut s, "render_preview", json!({"artboard":board}))["isError"], true, "{board}");
    }
    for name in ["ui_inspect", "ui_screenshot"] {
        assert_eq!(call(&mut s, name, json!({}))["isError"], true);
    }
}

/// `render_preview` and `screenshot` draw the artboard pixel for pixel as a PNG export on white
/// does: two rectangles that meet at x 40.3 and a diagonal edge. `render_preview` with the default
/// `max_side`, which is larger than the artboard, returns the export's size.
#[test]
fn render_preview_and_screenshot_match_a_png_export_on_white() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    assert_eq!(call(&mut s, "run_command", json!({"command":"file.new","params":{"width":120,"height":80}}))["isError"], false);
    for (x, w, fill) in [(0.0, 40.3, "#204080"), (40.3, 49.7, "#e0a020")] {
        let r = call(&mut s, "draw_shape", json!({"shape":"rectangle","x":x,"y":10,"width":w,"height":50,"fill":fill}));
        assert_eq!(r["isError"], false, "{r}");
    }
    let r = call(&mut s, "draw_path", json!({"points":[[10,75],[110,5],[115,30]],"closed":true,"fill":"#30a050"}));
    assert_eq!(r["isError"], false, "{r}");
    let pixels = |b64: &str| image::load_from_memory(&vectorcraft_format::base64_decode(b64).unwrap()).unwrap().to_rgba8();
    let export = payload(&call(&mut s, "run_command", json!({"command":"document.export","params":{"format":"png","background":"white"}})));
    let export = pixels(export["dataBase64"].as_str().unwrap());
    assert_eq!(export.dimensions(), (120, 80));
    for (name, args) in [("render_preview", json!({"max_side":120})), ("render_preview", json!({})), ("screenshot", json!({}))] {
        let r = call(&mut s, name, args);
        assert_eq!(r["isError"], false, "{name}: {r}");
        let img = pixels(r["content"][0]["data"].as_str().unwrap());
        assert_eq!(img.dimensions(), export.dimensions(), "{name}");
        let differ = img.pixels().zip(export.pixels()).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "{name}: {differ} pixels differ from the export");
    }
}

/// An atomic `command_batch` that starts with `file.new` makes the edits after it one undo step in
/// the new document, with or without a document open before it, and also when a later step fails.
#[test]
fn an_atomic_batch_is_one_undo_step_in_a_document_it_creates() {
    let mut steps = vec![
        json!({"id":"file.new","params":{"width":200,"height":100}}),
        json!({"id":"shape.rectangle","params":{"x":0,"y":0,"width":5,"height":5}}),
        json!({"id":"shape.ellipse","params":{"x":9,"y":0,"width":5,"height":5}}),
    ];
    for fail in [false, true] {
        if fail {
            steps.push(json!({"id":"no.such.command"}));
        }
        for backend in [Headless::new(), Headless::with_document()] {
            let mut s = Server::new(Box::new(backend));
            let r = call(&mut s, "command_batch", json!({"steps":steps,"atomic":true,"label":"Job"}));
            assert_eq!(r["isError"], fail, "{r}");
            let p = payload(&r);
            assert_eq!((p["completed"].clone(), p["failed"].clone()), (json!(3), json!(u8::from(fail))), "{r}");
            assert_eq!(payload(&call(&mut s, "doc_inspect", json!({"depth":0})))["history"], json!(["Job"]));
        }
    }
}

/// In an atomic `command_batch`, the edits made in a document that was open before the batch and
/// that a step switches to are undo steps of their own, and the edits in the document active when
/// the batch began are one undo step, also when they come before and after the switch.
#[test]
fn an_atomic_batch_groups_no_edits_in_a_document_it_switches_to() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    let r = call(&mut s, "command_run", json!({"id":"file.new","params":{"width":200,"height":100}}));
    assert_eq!(r["isError"], false, "{r}");
    let rect = json!({"x":0,"y":0,"width":5,"height":5});
    let steps = json!([
        {"id":"shape.rectangle","params":rect},
        {"id":"document.activate","params":{"index":0}},
        {"id":"shape.rectangle","params":rect},
        {"id":"shape.ellipse","params":rect},
        {"id":"document.activate","params":{"index":1}},
        {"id":"shape.ellipse","params":rect},
    ]);
    let r = call(&mut s, "command_batch", json!({"steps":steps,"atomic":true,"label":"Job"}));
    assert_eq!((r["isError"].clone(), payload(&r)["completed"].clone()), (json!(false), json!(6)), "{r}");
    let history = |s: &mut Server| payload(&call(s, "doc_inspect", json!({"depth":0})))["history"].clone();
    assert_eq!(history(&mut s), json!(["Job"]));
    call(&mut s, "command_run", json!({"id":"document.activate","params":{"index":0}}));
    assert_eq!(history(&mut s).as_array().map(Vec::len), Some(2), "{}", history(&mut s));
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
/// An app whose window capture is a 1200 × 800 PNG.
struct Window(Vec<u8>);
impl Backend for Window {
    fn call(&mut self, method: &str, _: Value) -> Result<Value, String> {
        match method {
            "ui.screenshot" => Ok(json!({"pngBase64": vectorcraft_format::base64_encode(&self.0), "width": 1200, "height": 800})),
            _ => Err(format!("unexpected {method}")),
        }
    }
    fn has_ui(&self) -> bool {
        true
    }
    fn describe(&self) -> String {
        "window".into()
    }
}

/// `ui_screenshot` and `screenshot {window: true}` take `max_side`, which scales the returned
/// image down to it and reports the capture's own size; without it the capture comes back as sent.
#[test]
fn window_captures_take_max_side() {
    let mut capture = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_fn(1200, 800, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, 255]))
        .write_to(&mut capture, image::ImageFormat::Png)
        .unwrap();
    let capture = capture.into_inner();
    let mut s = Server::new(Box::new(Window(capture.clone())));
    let image_of = |r: &Value| vectorcraft_format::base64_decode(r["content"][0]["data"].as_str().unwrap()).unwrap();
    let info = |r: &Value| -> Value { serde_json::from_str(r["content"][1]["text"].as_str().unwrap()).unwrap() };
    for (name, mut args) in [("ui_screenshot", json!({})), ("screenshot", json!({"window": true}))] {
        let r = call(&mut s, name, args.clone());
        assert_eq!(r["isError"], false, "{name}: {r}");
        assert_eq!(image_of(&r), capture, "{name}: the capture as sent");
        assert!(info(&r).get("capturedWidth").is_none(), "{name}: {r}");
        args["max_side"] = json!(300);
        let r = call(&mut s, name, args.clone());
        assert_eq!(r["isError"], false, "{name}: {r}");
        let img = image::load_from_memory(&image_of(&r)).unwrap();
        assert_eq!((img.width(), img.height()), (300, 200), "{name}");
        let i = info(&r);
        assert_eq!((i["width"].clone(), i["height"].clone()), (json!(300), json!(200)), "{name}: {i}");
        assert_eq!((i["capturedWidth"].clone(), i["capturedHeight"].clone()), (json!(1200), json!(800)), "{name}: {i}");
        // A capture that already fits comes back as sent.
        args["max_side"] = json!(4096);
        let r = call(&mut s, name, args.clone());
        assert_eq!(image_of(&r), capture, "{name}");
        assert_eq!(info(&r)["capturedWidth"], 1200, "{name}");
        for bad in [json!(0), json!(4097), json!("300"), json!(1.5)] {
            args["max_side"] = bad.clone();
            assert_eq!(call(&mut s, name, args.clone())["isError"], true, "{name}: {bad}");
        }
    }
    // An artboard render is sized with `scale`.
    let r = call(&mut s, "screenshot", json!({"max_side": 300}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(r["content"][0]["text"].as_str().unwrap().contains("`scale`"), "{r}");
}

/// Each tool with another name for the same call names it in its description.
#[test]
fn alias_tools_name_their_pair() {
    let tools = tool_definitions();
    let description = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap()["description"].as_str().unwrap().to_string();
    let names = |text: &str, name: &str| text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).any(|word| word == name);
    for (a, b) in [
        ("command_list", "list_commands"),
        ("command_run", "run_command"),
        ("doc_inspect", "inspect_document"),
        ("ui_inspect", "inspect_ui"),
        ("ui_screenshot", "screenshot"),
        ("undo", "redo"),
    ] {
        assert!(names(&description(a), b), "{a} names {b}: {}", description(a));
        assert!(names(&description(b), a), "{b} names {a}: {}", description(b));
    }
    assert!(description("ui_screenshot").contains("window:true"));
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
