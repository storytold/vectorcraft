//! The protocol surface beyond tools: prompts, completions, resource templates, subscriptions,
//! logging, progress and cancellation, driven the way a client drives them.

use serde_json::{Value, json};

use crate::{Headless, PROMPTS, Server};

fn server() -> Server {
    Server::new(Box::new(Headless::with_document()))
}

/// One request, and whatever the server queued while answering it.
fn rpc(s: &mut Server, id: u64, method: &str, params: Value) -> (Value, Vec<Value>) {
    let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
    let reply = s.handle_line(&line).and_then(|r| serde_json::from_str::<Value>(&r).ok());
    let notes: Vec<Value> = s.take_notifications().iter().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let reply = reply.unwrap_or(Value::Null);
    assert_eq!(reply["jsonrpc"], "2.0", "{reply}");
    assert_eq!(reply["id"], id, "{reply}");
    (reply, notes)
}

fn result(s: &mut Server, id: u64, method: &str, params: Value) -> Value {
    let (v, _) = rpc(s, id, method, params);
    assert!(v.get("error").is_none(), "{method}: {v}");
    v["result"].clone()
}

/// Run a tool and parse the JSON it answers with.
fn tool(s: &mut Server, id: u64, name: &str, args: Value) -> Value {
    let r = result(s, id, "tools/call", json!({"name": name, "arguments": args}));
    assert_eq!(r["isError"], false, "{name}: {r}");
    let text = r["content"][0]["text"].as_str().unwrap_or("null");
    serde_json::from_str(text).unwrap_or(Value::Null)
}

/// Draw a rectangle and return its id.
fn rectangle(s: &mut Server, id: u64) -> i64 {
    let made = tool(s, id, "draw_shape", json!({"shape": "rectangle", "x": 10, "y": 10, "width": 60, "height": 40}));
    made["id"].as_i64().unwrap_or_default()
}

fn err_code(s: &mut Server, id: u64, method: &str, params: Value) -> i64 {
    let (v, _) = rpc(s, id, method, params);
    v.get("error").and_then(|e| e["code"].as_i64()).unwrap_or_else(|| panic!("{method} should have failed: {v}"))
}

// ---------- capabilities ----------

#[test]
fn initialize_advertises_what_the_server_actually_answers() {
    let mut s = server();
    let caps = result(&mut s, 1, "initialize", json!({}))["capabilities"].clone();
    assert!(caps["tools"].is_object(), "{caps}");
    assert!(caps["resources"].is_object(), "{caps}");
    // No `subscribe`: this server cannot observe a change while a call is running, so it does not
    // claim to. Advertising a capability that silently misses edits is worse than not having it.
    assert!(caps["resources"].get("subscribe").is_none(), "{caps}");
    assert!(caps["prompts"].is_object(), "prompts are implemented");
    assert!(caps["completions"].is_object(), "completion/complete is implemented");
    assert!(caps["logging"].is_object(), "logging/setLevel is implemented");
    // And the instructions point at them, since that is where a client looks first.
    let instructions = result(&mut s, 2, "initialize", json!({}))["instructions"].as_str().unwrap_or_default().to_string();
    assert!(instructions.contains("prompts/list"), "{instructions}");
    assert!(instructions.contains("points in document space"), "{instructions}");
}

// ---------- prompts ----------

#[test]
fn prompts_are_listed_and_rendered_over_the_protocol() {
    let mut s = server();
    let listed = result(&mut s, 1, "prompts/list", json!({}))["prompts"].clone();
    let entries = listed.as_array().expect("prompts");
    assert_eq!(entries.len(), PROMPTS.len());
    for e in entries {
        assert!(e["name"].as_str().is_some_and(|n| !n.is_empty()), "{e}");
        assert!(e["description"].as_str().is_some_and(|d| d.len() > 20), "{e}");
        // Arguments are advertised with the flag prompts/get honours.
        for a in e["arguments"].as_array().map_or(&[][..], Vec::as_slice) {
            assert!(a["required"].is_boolean(), "{a}");
        }
    }

    let got = result(&mut s, 2, "prompts/get", json!({"name": "poster", "arguments": {"brief": "a jazz festival"}}));
    assert_eq!(got["messages"].as_array().map(Vec::len), Some(1));
    assert_eq!(got["messages"][0]["role"], "user");
    let text = got["messages"][0]["content"]["text"].as_str().unwrap_or_default();
    assert!(text.contains("a jazz festival"), "{text}");
}

#[test]
fn prompt_argument_errors_are_invalid_params() {
    let mut s = server();
    assert_eq!(err_code(&mut s, 1, "prompts/get", json!({"name": "poster"})), -32602);
    assert_eq!(err_code(&mut s, 2, "prompts/get", json!({"name": "nope"})), -32602);
    assert_eq!(err_code(&mut s, 3, "prompts/get", json!({})), -32602);
}

// ---------- envelope ----------

/// Requests and notifications outside a `"jsonrpc": "2.0"` envelope are never dispatched (#1728).
#[test]
fn envelopes_without_version_20_are_not_dispatched() {
    let mut s = server();
    // A request in no envelope: invalid, and nothing ran.
    let line = json!({"id": 1, "method": "ping", "params": {}}).to_string();
    let v: Value = serde_json::from_str(&s.handle_line(&line).unwrap()).unwrap();
    assert_eq!(v["error"]["code"], -32600, "{v}");
    assert!(v["error"]["message"].as_str().unwrap_or_default().contains("2.0"), "{v}");
    // A wrong version is the same.
    let line = json!({"jsonrpc": "1.0", "id": 2, "method": "ping", "params": {}}).to_string();
    let v: Value = serde_json::from_str(&s.handle_line(&line).unwrap()).unwrap();
    assert_eq!(v["error"]["code"], -32600, "{v}");
    // In a batch the unversioned message is an error entry, not a dispatch; the 2.0 one answers.
    let batch = json!([
        {"jsonrpc": "2.0", "id": 3, "method": "ping", "params": {}},
        {"id": 4, "method": "ping", "params": {}},
    ])
    .to_string();
    let v: Value = serde_json::from_str(&s.handle_line(&batch).unwrap()).unwrap();
    assert_eq!(v.as_array().map(Vec::len), Some(2), "{v}");
    assert!(v[0].get("result").is_some(), "{v}");
    assert_eq!(v[1]["error"]["code"], -32600, "{v}");
    // The 2.0 request still works.
    assert_eq!(result(&mut s, 5, "ping", json!({})), json!({}));
}

#[test]
fn notification_without_version_20_changes_nothing() {
    let mut s = server();
    assert!(!s.is_initialized());
    // No envelope, no id: there is no address for a reply, so it is ignored — and it must not
    // flip the server into its initialised state.
    let line = json!({"method": "notifications/initialized", "params": {}}).to_string();
    assert!(s.handle_line(&line).is_none());
    assert!(!s.is_initialized(), "an unversioned notification changed state");
    // The 2.0 notification still works.
    let line = json!({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}).to_string();
    assert!(s.handle_line(&line).is_none());
    assert!(s.is_initialized());
}

// ---------- completions ----------

#[test]
fn completions_come_from_the_live_catalogues() {
    let mut s = server();
    // A prompt argument fed by the export formats.
    let v = result(
        &mut s,
        1,
        "completion/complete",
        json!({"ref": {"type": "ref/prompt", "name": "export-set"}, "argument": {"name": "formats", "value": "sv"}}),
    );
    let values: Vec<&str> = v["completion"]["values"].as_array().map_or(Vec::new(), |a| a.iter().filter_map(Value::as_str).collect());
    assert!(values.contains(&"svg"), "{v}");
    assert_eq!(v["completion"]["hasMore"], false);

    // A resource template variable, after something exists to point at.
    rectangle(&mut s, 2);
    let v = result(
        &mut s,
        3,
        "completion/complete",
        json!({"ref": {"type": "ref/resource", "uri": "vectorcraft://object/{id}"}, "argument": {"name": "id", "value": ""}}),
    );
    let values = v["completion"]["values"].as_array().map_or(0, Vec::len);
    assert!(values > 0, "{v}");
    assert!(v["completion"]["total"].as_u64().unwrap_or(0) >= values as u64);
}

// ---------- resource templates ----------

#[test]
fn templates_are_listed_and_readable() {
    let mut s = server();
    let listed = result(&mut s, 1, "resources/templates/list", json!({}));
    let templates = listed["resourceTemplates"].as_array().expect("templates");
    assert!(templates.len() >= 4, "{listed}");

    let id = rectangle(&mut s, 2);

    let read = result(&mut s, 3, "resources/read", json!({"uri": format!("vectorcraft://object/{id}")}));
    let body: Value = serde_json::from_str(read["contents"][0]["text"].as_str().unwrap_or("{}")).unwrap_or_default();
    assert_eq!(body["id"].as_i64(), Some(id), "{body}");
    assert!(body["bounds"].is_object(), "{body}");

    // A command definition is one small read instead of the whole 400-command catalogue.
    let read = result(&mut s, 4, "resources/read", json!({"uri": "vectorcraft://command/paint.setFill"}));
    let body: Value = serde_json::from_str(read["contents"][0]["text"].as_str().unwrap_or("{}")).unwrap_or_default();
    assert_eq!(body["id"], "paint.setFill", "{body}");
    assert!(body["params"].as_str().is_some_and(|p| !p.is_empty()), "{body}");

    // An effect's parameters and defaults.
    let read = result(&mut s, 5, "resources/read", json!({"uri": "vectorcraft://effect/stylize.dropShadow"}));
    let body: Value = serde_json::from_str(read["contents"][0]["text"].as_str().unwrap_or("{}")).unwrap_or_default();
    assert_eq!(body["id"], "stylize.dropShadow", "{body}");

    // Missing values and unknown resources are distinguishable.
    assert_eq!(err_code(&mut s, 6, "resources/read", json!({"uri": "vectorcraft://object/"})), -32602);
    assert_eq!(err_code(&mut s, 7, "resources/read", json!({"uri": "vectorcraft://command/nope"})), -32602);
    assert_eq!(err_code(&mut s, 8, "resources/read", json!({"uri": "vectorcraft://nope"})), -32002);
}

// ---------- logging ----------

#[test]
fn logging_levels_are_validated_and_off_by_default() {
    let _turn = crate::logging::test_turn();
    let mut s = server();
    assert_eq!(crate::logging::level(), None, "silent until asked for");
    result(&mut s, 1, "logging/setLevel", json!({"level": "debug"}));
    assert_eq!(crate::logging::level(), Some("debug"));
    // A null level turns it back off; a missing one is a bad request.
    result(&mut s, 2, "logging/setLevel", json!({"level": null}));
    assert_eq!(crate::logging::level(), None);
    assert_eq!(err_code(&mut s, 3, "logging/setLevel", json!({"level": "shouty"})), -32602);
    assert_eq!(err_code(&mut s, 4, "logging/setLevel", json!({})), -32602);
    // A rejected level leaves the accepted one alone.
    result(&mut s, 5, "logging/setLevel", json!({"level": "warning"}));
    assert_eq!(err_code(&mut s, 6, "logging/setLevel", json!({"level": "shouty"})), -32602);
    assert_eq!(crate::logging::level(), Some("warning"));
    result(&mut s, 7, "logging/setLevel", json!({"level": null}));
    assert_eq!(crate::logging::level(), None);
}

/// Log records reach the client as `notifications/message`, and only after it asked.
#[test]
fn log_records_go_out_as_notifications_once_asked_for() {
    let _turn = crate::logging::test_turn();
    // The binary installs the logger, not the library, so a test does it for itself.
    crate::logging::install();
    let mut s = server();

    let input = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "logging/setLevel", "params": {"level": "debug"}}).to_string(),
        json!({"jsonrpc": "2.0", "id": 2, "method": "prompts/list"}).to_string(),
        json!({"jsonrpc": "2.0", "id": 3, "method": "logging/setLevel", "params": {"level": null}}).to_string(),
    ]
    .join("\n");
    let mut out = Vec::new();
    s.serve(input.as_bytes(), &mut out).unwrap();
    let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();

    let notes: Vec<&Value> = lines.iter().filter(|l| l["method"] == "notifications/message").collect();
    assert!(!notes.is_empty(), "the level change itself is logged: {lines:?}");
    for n in &notes {
        assert_eq!(n["jsonrpc"], "2.0", "{n}");
        assert!(
            ["debug", "info", "notice", "warning", "error", "critical", "alert", "emergency"]
                .contains(&n["params"]["level"].as_str().unwrap_or_default()),
            "{n}"
        );
        assert!(n["params"]["data"].is_string(), "{n}");
    }
    // Every reply still came before the notifications that followed it.
    let answered = lines.iter().filter(|l| l.get("id").is_some()).count();
    assert_eq!(answered, 3, "{lines:?}");
    let last_reply = lines.iter().rposition(|l| l.get("id").is_some()).unwrap_or(0);
    assert!(lines[last_reply + 1..].iter().all(|l| l.get("id").is_none()), "{lines:?}");
    result(&mut s, 4, "logging/setLevel", json!({"level": null}));
}
