//! Events plug-ins react to (`events`, `vc_event`): each event reaches the plug-ins that name it,
//! once, after the change; a burst of selection changes in one frame is one event; a handler only
//! reads; and a handler that keeps failing stops being called.

use serde_json::{Value, json};
use vectorcraft_plugins::LogLevel;
use vectorcraft_plugins::wat as tpl;

use crate::*;

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

/// A session with a 400 × 200 document that takes `grant` from requests.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
    s.allow_plugin_grants_from_requests();
    s
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 10, "width": 10, "height": 10}))["id"].as_u64().unwrap())
}

/// Installs plug-in `id` that reacts to `events` (a JSON list) with the vc_event body `handler`,
/// writes log lines and also has the permissions `more`; `imports` are module fields its handler
/// uses besides the host imports, and `allowlist` the commands it may call.
fn install(s: &mut Session, id: &str, events: &str, more: &[&str], imports: &str, handler: &str, allowlist: &[&str]) {
    let mut grant = vec!["progress", "events"];
    grant.extend_from_slice(more);
    let allow = if allowlist.is_empty() { String::new() } else { format!(r#","commandAllowlist":{}"#, json!(allowlist)) };
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":{},"events":{events}{allow}}}"#, json!(grant));
    let wasm = wat::parse_str(tpl::reacting(&manifest, &format!("{} {imports}", tpl::HOST_IMPORTS), handler)).unwrap();
    run(s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": grant, "replace": true}));
}

/// The events plug-in `id`'s handler was given since the last call, as it logged them.
fn given(s: &mut Session, id: &str) -> Vec<Value> {
    let notices = s.take_plugin_event_notices();
    notices.iter().filter(|n| n.plugin == id && n.level == LogLevel::Info).map(|n| serde_json::from_str(&n.text).unwrap()).collect()
}

fn types(events: &[Value]) -> Vec<&str> {
    events.iter().map(|e| e["type"].as_str().unwrap()).collect()
}

/// A selection change, a new layer, a save and an open each reach the plug-in that names them once,
/// after the change, with the document they happened in; a plug-in that names only `document.saved`
/// is given only the save.
#[test]
fn each_event_reaches_the_plugins_that_name_it_once() {
    let (all, saves) = ("test.events.all", "test.events.saves");
    let mut s = session();
    let r = rect(&mut s, 10.0);
    run(&mut s, "select.none", json!({}));
    install(&mut s, all, r#"["selection.changed","document.opened","document.saved","layer.changed"]"#, &["doc.read"], "", tpl::LOG_EVENT, &[]);
    install(&mut s, saves, r#"["document.saved"]"#, &[], "", tpl::LOG_EVENT, &[]);
    assert!(given(&mut s, all).is_empty(), "installing replays nothing");
    let doc = s.doc().unwrap().uid;

    run(&mut s, "select.set", json!({"ids": [r.0]}));
    let notices = s.take_plugin_event_notices();
    let got: Vec<Value> = notices.iter().filter(|n| n.plugin == all).map(|n| serde_json::from_str(&n.text).unwrap()).collect();
    assert_eq!(got, [json!({"type": "selection.changed", "document": doc, "count": 1, "ids": [r.0], "truncated": false})]);
    assert!(notices.iter().all(|n| n.plugin != saves), "not named, not given");

    run(&mut s, "select.set", json!({"ids": [r.0]}));
    assert!(given(&mut s, all).is_empty(), "the same selection again is no change");

    let layer = run(&mut s, "layer.new", json!({"name": "Notes"}))["id"].as_u64().unwrap();
    let got = given(&mut s, all);
    assert_eq!(types(&got), ["layer.changed"]);
    assert_eq!(got[0]["layer"], json!(layer));
    assert!(got[0]["reasons"].as_array().unwrap().contains(&json!("added")), "{}", got[0]);

    let dir = std::env::temp_dir().join(format!("vectorcraft-engine-plugin-events-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("events.vectorcraft").to_string_lossy().into_owned();
    run(&mut s, "document.save", json!({"path": path}));
    let notices = s.take_plugin_event_notices();
    for id in [all, saves] {
        let got: Vec<Value> = notices.iter().filter(|n| n.plugin == id).map(|n| serde_json::from_str(&n.text).unwrap()).collect();
        assert_eq!(got, [json!({"type": "document.saved", "document": doc})], "{id}");
    }

    run(&mut s, "document.open", json!({"path": path}));
    let opened = s.doc().unwrap().uid;
    assert_ne!(opened, doc);
    assert_eq!(given(&mut s, all), [json!({"type": "document.opened", "document": opened})], "a new document's own selection is no change");

    std::fs::remove_dir_all(&dir).ok();
    run(&mut s, "plugin.remove", json!({"id": all}));
    run(&mut s, "plugin.remove", json!({"id": saves}));
}

/// A save of a document that closes before the frame's delivery still reaches the plug-in, with
/// the document's number; its selection change does not. So does a save written on another thread
/// that finishes after its document closed.
#[test]
fn a_save_reaches_the_plugin_when_its_document_closes_first() {
    let id = "test.events.closed";
    let mut s = session();
    let doc = s.doc().unwrap().uid;
    let r = rect(&mut s, 10.0);
    run(&mut s, "select.none", json!({}));
    install(&mut s, id, r#"["selection.changed","document.saved"]"#, &[], "", tpl::LOG_EVENT, &[]);
    s.plugin_event_frames = true;
    run(&mut s, "select.set", json!({"ids": [r.0]}));
    let dir = std::env::temp_dir().join(format!("vectorcraft-engine-plugin-events-closed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    run(&mut s, "document.save", json!({"path": dir.join("closed.vectorcraft").to_string_lossy()}));
    run(&mut s, "file.close", json!({}));
    assert!(s.docs.iter().all(|d| d.uid != doc), "the document closed");
    s.deliver_plugin_events();
    assert_eq!(given(&mut s, id), [json!({"type": "document.saved", "document": doc})]);

    run(&mut s, "file.new", json!({"width": 100, "height": 100}));
    let later = s.doc().unwrap().uid;
    let p = json!({"path": dir.join("later.vectorcraft").to_string_lossy()});
    let plan = crate::cmd::fileio::save_plan(&s, crate::cmd::fileio::SaveMode::SaveAs, &p).unwrap();
    let job = crate::cmd::fileio::save_job(&mut s, plan).unwrap();
    job.write(|path, bytes| std::fs::write(path, bytes).map_err(|e| EngineError::Other(e.to_string()))).unwrap();
    run(&mut s, "file.close", json!({}));
    assert!(s.docs.iter().all(|d| d.uid != later), "the document closed");
    job.complete(&mut s, later);
    s.deliver_plugin_events();
    assert_eq!(given(&mut s, id), [json!({"type": "document.saved", "document": later})]);
    std::fs::remove_dir_all(&dir).ok();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A plug-in installed after a save or a selection change, before the frame's delivery, is given
/// neither; the plug-in installed before is given both, and the next change reaches both.
#[test]
fn a_plugin_installed_before_the_delivery_gets_no_earlier_events() {
    let (early, late) = ("test.events.early", "test.events.late");
    let events = r#"["selection.changed","document.saved"]"#;
    let mut s = session();
    let doc = s.doc().unwrap().uid;
    let (a, b) = (rect(&mut s, 10.0), rect(&mut s, 40.0));
    run(&mut s, "select.none", json!({}));
    install(&mut s, early, events, &[], "", tpl::LOG_EVENT, &[]);
    s.plugin_event_frames = true;
    let dir = std::env::temp_dir().join(format!("vectorcraft-engine-plugin-events-late-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    run(&mut s, "document.save", json!({"path": dir.join("late.vectorcraft").to_string_lossy()}));
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    install(&mut s, late, events, &[], "", tpl::LOG_EVENT, &[]);
    s.deliver_plugin_events();
    let notices = s.take_plugin_event_notices();
    let of = |id: &str| -> Vec<Value> { notices.iter().filter(|n| n.plugin == id).map(|n| serde_json::from_str(&n.text).unwrap()).collect() };
    assert_eq!(types(&of(early)), ["document.saved", "selection.changed"]);
    assert!(of(late).is_empty(), "{:?}", of(late));
    run(&mut s, "select.set", json!({"ids": [b.0]}));
    s.deliver_plugin_events();
    let notices = s.take_plugin_event_notices();
    for id in [early, late] {
        let got: Vec<Value> = notices.iter().filter(|n| n.plugin == id).map(|n| serde_json::from_str(&n.text).unwrap()).collect();
        assert_eq!(got, [json!({"type": "selection.changed", "document": doc, "count": 1, "ids": null, "truncated": false})], "{id}");
    }
    std::fs::remove_dir_all(&dir).ok();
    run(&mut s, "plugin.remove", json!({"id": early}));
    run(&mut s, "plugin.remove", json!({"id": late}));
}

/// With delivery once a frame, a selection change made after a plug-in was installed, in the same
/// frame, reaches it.
#[test]
fn a_change_after_the_install_in_the_same_frame_reaches_the_plugin() {
    let id = "test.events.same_frame";
    let mut s = session();
    let doc = s.doc().unwrap().uid;
    let r = rect(&mut s, 10.0);
    run(&mut s, "select.none", json!({}));
    s.plugin_event_frames = true;
    install(&mut s, id, r#"["selection.changed"]"#, &["doc.read"], "", tpl::LOG_EVENT, &[]);
    run(&mut s, "select.set", json!({"ids": [r.0]}));
    s.deliver_plugin_events();
    assert_eq!(given(&mut s, id), [json!({"type": "selection.changed", "document": doc, "count": 1, "ids": [r.0], "truncated": false})]);
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// With delivery once a frame, a change made to a document created or opened since the last
/// delivery gives its event at the next one; an opened document's `document.opened` comes first,
/// once.
#[test]
fn a_change_to_a_document_opened_since_the_last_delivery_gives_its_event() {
    let id = "test.events.new_doc";
    let mut s = session();
    let dir = std::env::temp_dir().join(format!("vectorcraft-engine-plugin-events-new-doc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("new-doc.vectorcraft").to_string_lossy().into_owned();
    run(&mut s, "document.save", json!({"path": path}));
    install(&mut s, id, r#"["selection.changed","document.opened","layer.changed"]"#, &["doc.read"], "", tpl::LOG_EVENT, &[]);
    s.plugin_event_frames = true;
    s.deliver_plugin_events();
    assert!(given(&mut s, id).is_empty());

    run(&mut s, "file.new", json!({"width": 100, "height": 100}));
    let created = s.doc().unwrap().uid;
    let r = rect(&mut s, 10.0);
    run(&mut s, "select.set", json!({"ids": [r.0]}));
    s.deliver_plugin_events();
    assert_eq!(given(&mut s, id), [json!({"type": "selection.changed", "document": created, "count": 1, "ids": [r.0], "truncated": false})]);

    run(&mut s, "document.open", json!({"path": path}));
    let opened = s.doc().unwrap().uid;
    let layer = run(&mut s, "layer.new", json!({"name": "Notes"}))["id"].as_u64().unwrap();
    s.deliver_plugin_events();
    let got = given(&mut s, id);
    assert_eq!(types(&got), ["document.opened", "layer.changed"], "{got:?}");
    assert_eq!((got[0]["document"].clone(), got[1]["document"].clone(), got[1]["layer"].clone()), (json!(opened), json!(opened), json!(layer)));
    assert!(got[1]["reasons"].as_array().unwrap().contains(&json!("added")), "{}", got[1]);
    s.deliver_plugin_events();
    assert!(given(&mut s, id).is_empty(), "once");
    std::fs::remove_dir_all(&dir).ok();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A save waiting for delivery is not given to the same module removed and installed again before
/// the delivery.
#[test]
fn a_module_installed_again_gets_no_event_from_before_its_removal() {
    let id = "test.events.again";
    let mut s = session();
    s.plugin_event_frames = true;
    install(&mut s, id, r#"["document.saved"]"#, &[], "", tpl::LOG_EVENT, &[]);
    let dir = std::env::temp_dir().join(format!("vectorcraft-engine-plugin-events-again-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    run(&mut s, "document.save", json!({"path": dir.join("again.vectorcraft").to_string_lossy()}));
    run(&mut s, "plugin.remove", json!({"id": id}));
    install(&mut s, id, r#"["document.saved"]"#, &[], "", tpl::LOG_EVENT, &[]);
    s.deliver_plugin_events();
    assert!(given(&mut s, id).is_empty(), "the save came before this install");
    std::fs::remove_dir_all(&dir).ok();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A save waiting in one session is not given to the same module that another session removed and
/// installed again before this session's delivery.
#[test]
fn a_module_installed_again_by_another_session_gets_no_earlier_event() {
    let id = "test.events.again_elsewhere";
    let mut s = session();
    s.plugin_event_frames = true;
    install(&mut s, id, r#"["document.saved"]"#, &[], "", tpl::LOG_EVENT, &[]);
    let dir = std::env::temp_dir().join(format!("vectorcraft-engine-plugin-events-elsewhere-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    run(&mut s, "document.save", json!({"path": dir.join("elsewhere.vectorcraft").to_string_lossy()}));
    let mut other = session();
    run(&mut other, "plugin.remove", json!({"id": id}));
    install(&mut other, id, r#"["document.saved"]"#, &[], "", tpl::LOG_EVENT, &[]);
    s.deliver_plugin_events();
    assert!(given(&mut s, id).is_empty(), "the save came before this install");
    std::fs::remove_dir_all(&dir).ok();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A plug-in installed between two layer changes waiting for the same delivery is given only the
/// reason of the change after its install; the plug-in installed before is given both.
#[test]
fn a_plugin_installed_between_layer_changes_gets_only_the_later_reason() {
    let (early, late) = ("test.events.reasons_early", "test.events.reasons_late");
    let mut s = session();
    let first = s.doc().unwrap().active_layer.unwrap();
    let second = NodeId(run(&mut s, "layer.new", json!({"name": "Two"}))["id"].as_u64().unwrap());
    install(&mut s, early, r#"["layer.changed"]"#, &[], "", tpl::LOG_EVENT, &[]);
    s.plugin_event_frames = true;
    run(&mut s, "layer.setProps", json!({"id": second.0, "name": "Notes"}));
    install(&mut s, late, r#"["layer.changed"]"#, &[], "", tpl::LOG_EVENT, &[]);
    run(&mut s, "layer.setCurrent", json!({"id": first.0}));
    s.deliver_plugin_events();
    let notices = s.take_plugin_event_notices();
    let reasons = |id: &str| -> Vec<Value> {
        notices.iter().filter(|n| n.plugin == id).map(|n| serde_json::from_str::<Value>(&n.text).unwrap()["reasons"].clone()).collect()
    };
    assert_eq!(reasons(early), [json!(["current", "options"])]);
    assert_eq!(reasons(late), [json!(["current"])], "the rename came before this install");
    run(&mut s, "plugin.remove", json!({"id": early}));
    run(&mut s, "plugin.remove", json!({"id": late}));
}

/// A save waiting for delivery is held while an interaction is open, and given at the first
/// delivery after it ends.
#[test]
fn waiting_events_are_held_while_an_interaction_is_open() {
    let id = "test.events.interaction";
    let mut s = session();
    let doc = s.doc().unwrap().uid;
    install(&mut s, id, r#"["document.saved"]"#, &[], "", tpl::LOG_EVENT, &[]);
    s.plugin_event_frames = true;
    let dir = std::env::temp_dir().join(format!("vectorcraft-engine-plugin-events-interaction-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    run(&mut s, "document.save", json!({"path": dir.join("interaction.vectorcraft").to_string_lossy()}));
    s.begin_interaction("Move").unwrap();
    s.deliver_plugin_events();
    assert!(given(&mut s, id).is_empty(), "not while the interaction is open");
    s.cancel_interaction().unwrap();
    s.deliver_plugin_events();
    assert_eq!(given(&mut s, id), [json!({"type": "document.saved", "document": doc})]);
    std::fs::remove_dir_all(&dir).ok();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A save in a batch whose later step fails is still given to the plug-in when the request ends;
/// an open in that batch, which the rollback closes again, is not.
#[test]
fn a_save_before_a_failed_batch_step_is_delivered() {
    let id = "test.events.failed_batch";
    let mut s = session();
    let doc = s.doc().unwrap().uid;
    install(&mut s, id, r#"["document.saved","document.opened"]"#, &[], "", tpl::LOG_EVENT, &[]);
    let dir = std::env::temp_dir().join(format!("vectorcraft-engine-plugin-events-failed-batch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("failed-batch.vectorcraft").to_string_lossy().into_owned();
    let steps = json!([
        {"command": "document.save", "params": {"path": path}},
        {"command": "document.open", "params": {"path": path}},
        {"command": "no.such.command", "params": {}}
    ]);
    assert!(s.execute("command.batch", &json!({"commands": steps})).is_err());
    assert!(std::path::Path::new(&path).exists(), "the save was written");
    assert_eq!(s.documents().len(), 1, "the opened document was closed again");
    assert_eq!(given(&mut s, id), [json!({"type": "document.saved", "document": doc})]);
    std::fs::remove_dir_all(&dir).ok();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A document a script opened before it failed is discarded with the run's working copy, so the
/// plug-in is not told of it. The save the script made before failing is still given, and so is a
/// document the frontend opened while the run was paused.
#[test]
fn a_failed_script_gives_its_save_but_not_its_open() {
    let id = "test.events.failed_script";
    let mut s = session();
    let doc = s.doc().unwrap().uid;
    install(&mut s, id, r#"["document.saved","document.opened"]"#, &[], "", tpl::LOG_EVENT, &[]);
    s.plugin_event_frames = true;
    let dir = std::env::temp_dir().join(format!("vectorcraft-engine-plugin-events-failed-script-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    run(&mut s, "document.save", json!({"path": dir.join("failed-script.vectorcraft").to_string_lossy()}));
    s.deliver_plugin_events();
    assert_eq!(given(&mut s, id), [json!({"type": "document.saved", "document": doc})]);
    let data = run(&mut s, "document.serialize", json!({}))["dataBase64"].as_str().unwrap().to_string();

    s.script_frames = true;
    let source = format!(
        "vc.run('document.save', {{}}); vc.run('document.open', {{name: 'b.vectorcraft', dataBase64: '{data}'}}); \
         for (var i = 0; i < 3000000; i++) {{}} throw new Error('stop');"
    );
    assert_eq!(run(&mut s, "script.run", json!({"source": source, "name": "t.js"}))["running"], true);
    assert!(s.script_pump().is_none(), "paused after its first slice");
    let bytes = vectorcraft_format::base64_decode(&data).unwrap();
    let index = crate::cmd::fileio::open_bytes(&mut s, "front.vectorcraft", &bytes, None).unwrap()["index"].as_u64().unwrap();
    let front = s.documents()[index as usize].uid;
    let mut slices = 0;
    let r = loop {
        slices += 1;
        assert!(slices < 100_000, "the script ends");
        if let Some(r) = s.script_pump() {
            break r;
        }
    };
    assert!(r.is_err(), "the script threw");
    assert!(s.docs.iter().all(|d| d.uid == doc || d.uid == front), "the script's document was discarded");
    s.deliver_plugin_events();
    assert_eq!(given(&mut s, id), [json!({"type": "document.saved", "document": doc}), json!({"type": "document.opened", "document": front})]);
    std::fs::remove_dir_all(&dir).ok();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A `selection.changed` event of a plug-in without `doc.read` gives the count and no ids; one
/// with it lists the selected ids.
#[test]
fn selected_ids_need_doc_read() {
    let id = "test.events.noread";
    let mut s = session();
    let (a, b) = (rect(&mut s, 10.0), rect(&mut s, 40.0));
    install(&mut s, id, r#"["selection.changed"]"#, &[], "", tpl::LOG_EVENT, &[]);
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    let got = given(&mut s, id);
    assert_eq!(got.len(), 1);
    assert_eq!((got[0]["count"].clone(), got[0]["ids"].clone()), (json!(2), Value::Null));
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// With delivery once a frame, the selection changes of one frame are one event with the selection
/// the frame ended with, and changes that end where they started are none.
#[test]
fn selection_changes_in_one_frame_are_one_event() {
    let id = "test.events.burst";
    let mut s = session();
    let (a, b) = (rect(&mut s, 10.0), rect(&mut s, 40.0));
    run(&mut s, "select.none", json!({}));
    install(&mut s, id, r#"["selection.changed"]"#, &["doc.read"], "", tpl::LOG_EVENT, &[]);
    s.plugin_event_frames = true;
    for ids in [vec![a.0], vec![b.0], vec![], vec![a.0, b.0], vec![b.0]] {
        run(&mut s, "select.set", json!({"ids": ids}));
    }
    assert!(given(&mut s, id).is_empty(), "nothing before the frame");
    s.deliver_plugin_events();
    let got = given(&mut s, id);
    assert_eq!(types(&got), ["selection.changed"]);
    assert_eq!(got[0]["ids"], json!([b.0]));
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    run(&mut s, "select.set", json!({"ids": [b.0]}));
    s.deliver_plugin_events();
    assert!(given(&mut s, id).is_empty(), "back where the frame started");
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// `layer.changed` names what changed: a new name is `options`, the same name again is no change,
/// another current layer is `current`, a moved layer `reordered` and a deleted one `deleted`.
#[test]
fn layer_changes_name_their_reasons() {
    let id = "test.events.layers";
    let mut s = session();
    let first = s.doc().unwrap().active_layer.unwrap();
    let second = NodeId(run(&mut s, "layer.new", json!({"name": "Two"}))["id"].as_u64().unwrap());
    install(&mut s, id, r#"["layer.changed"]"#, &[], "", tpl::LOG_EVENT, &[]);
    let reasons = |s: &mut Session, cmd: &str, p: Value| {
        run(s, cmd, p);
        given(s, id).iter().map(|e| e["reasons"].clone()).collect::<Vec<_>>()
    };
    assert_eq!(reasons(&mut s, "layer.setProps", json!({"id": second.0, "name": "Notes"})), [json!(["options"])]);
    assert!(reasons(&mut s, "layer.setProps", json!({"id": second.0, "name": "Notes"})).is_empty(), "same name, no change");
    assert_eq!(reasons(&mut s, "layer.setCurrent", json!({"id": first.0})), [json!(["current"])]);
    assert_eq!(reasons(&mut s, "layer.move", json!({"ids": [first.0], "target": second.0, "place": "above"})), [json!(["reordered"])]);
    let deleted = reasons(&mut s, "layer.delete", json!({"ids": [second.0]}));
    assert_eq!(deleted.len(), 1);
    assert!(deleted[0].as_array().unwrap().contains(&json!("deleted")), "{deleted:?}");

    // A sublayer moved past an object beside it is reordered; an object drawn beside it is not.
    run(&mut s, "layer.setCurrent", json!({"id": first.0}));
    given(&mut s, id);
    let sub = NodeId(run(&mut s, "layer.newSublayer", json!({"parent": first.0, "name": "Sub"}))["id"].as_u64().unwrap());
    run(&mut s, "layer.setCurrent", json!({"id": first.0}));
    given(&mut s, id);
    let r = rect(&mut s, 10.0);
    assert!(given(&mut s, id).is_empty(), "drawing beside a sublayer moves no layer");
    assert_eq!(reasons(&mut s, "layer.move", json!({"ids": [sub.0], "target": r.0, "place": "above"})), [json!(["reordered"])]);
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A handler may call a command listed for handlers, and is given its result; one that calls a
/// command that changes the document or the current layer, or a read not listed for handlers, is
/// refused with an error that names the event, and the selection, the current layer and the undo
/// history stay as the user left them.
#[test]
fn a_handler_only_reads() {
    let (reads, edits) = ("test.events.reads", "test.events.edits");
    let (unlisted, layers) = ("test.events.unlisted", "test.events.layer");
    let mut s = session();
    let first = s.doc().unwrap().active_layer.unwrap();
    let other = run(&mut s, "layer.new", json!({"name": "Other"}))["id"].as_u64().unwrap();
    run(&mut s, "layer.setCurrent", json!({"id": first.0}));
    let (a, _) = (rect(&mut s, 10.0), rect(&mut s, 40.0));
    run(&mut s, "select.none", json!({}));
    let expected = run(&mut s, "variable.list", json!({}));
    // The read's result, copied with `vc_call_result`, is written as a log line.
    let (imports, call) = calling("variable.list", "{}");
    let imports = format!(r#"{imports} (import "env" "vc_call_result" (func $vc_call_result (param i32 i32) (result i32)))"#);
    let handler = format!(
        "(drop {call}) (drop (call $vc_log (i32.const 2) (i32.const 0x4000) (call $vc_call_result (i32.const 0x4000) (i32.const 0x4000)))) (i32.const 0)"
    );
    install(&mut s, reads, r#"["selection.changed"]"#, &["doc.read", "commands"], &imports, &handler, &["variable.list"]);
    assert!(!find_command("addon.cleanup.check").unwrap().journal, "a query the journal does not record");
    let (imports, handler) = calling("addon.cleanup.check", "{}");
    install(&mut s, unlisted, r#"["selection.changed"]"#, &["doc.read", "commands"], &imports, &handler, &["addon.cleanup.check"]);
    let (imports, handler) = calling("select.all", "{}");
    install(&mut s, edits, r#"["selection.changed"]"#, &["doc.read", "commands"], &imports, &handler, &["select.all"]);
    let (imports, handler) = calling("layer.setCurrent", &format!(r#"{{"id":{other}}}"#));
    install(&mut s, layers, r#"["selection.changed"]"#, &["doc.read", "commands"], &imports, &handler, &["layer.setCurrent"]);
    let (undo, rows) = (s.doc().unwrap().history.undo.len(), s.doc().unwrap().layer_rows.clone());
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    let notices = s.take_plugin_event_notices();
    let read: Vec<Value> = notices.iter().filter(|n| n.plugin == reads).map(|n| serde_json::from_str(&n.text).unwrap()).collect();
    assert_eq!(read, [expected], "the read call succeeds and gives its result: {notices:?}");
    for (id, cmd) in [(unlisted, "addon.cleanup.check"), (edits, "select.all"), (layers, "layer.setCurrent")] {
        let failed: Vec<&str> = notices.iter().filter(|n| n.plugin == id && n.level == LogLevel::Error).map(|n| n.text.as_str()).collect();
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(failed[0].contains(&format!("a selection.changed handler only reads: `{cmd}` is not a command it may call")), "{}", failed[0]);
    }
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, [a], "the selection the user made");
    assert_eq!((st.active_layer, &st.layer_rows), (Some(first), &rows), "the current layer the user chose");
    assert_eq!(st.history.undo.len(), undo, "no undo step");
    let info = run(&mut s, "plugin.info", json!({"id": edits}));
    assert!(info["lastEventError"].as_str().unwrap().contains("only reads"), "{info}");
    assert_eq!(info["eventsOff"], json!(false));
    assert_eq!(info["events"], json!(["selection.changed"]));
    run(&mut s, "plugin.remove", json!({"id": reads}));
    run(&mut s, "plugin.remove", json!({"id": unlisted}));
    run(&mut s, "plugin.remove", json!({"id": edits}));
    run(&mut s, "plugin.remove", json!({"id": layers}));
}

/// A handler that calls a query the journal does not record but that changes the selection (Select
/// All Instances) is refused before the query runs: the document, its revision, selection and undo
/// history, and the session's paint and Appearance state, are as the user left them.
#[test]
fn a_handler_query_that_selects_is_refused_before_it_runs() {
    let id = "test.events.selects";
    let (cmd, args) = ("symbol.selectInstances", r#"{"name":"Dot"}"#);
    assert!(!find_command(cmd).unwrap().journal, "a query the journal does not record");
    let mut s = session();
    let dot = rect(&mut s, 10.0);
    run(&mut s, "symbol.new", json!({"name": "Dot", "ids": [dot.0]}));
    let b = rect(&mut s, 40.0);
    run(&mut s, "select.none", json!({}));
    let (imports, handler) = calling(cmd, args);
    install(&mut s, id, r#"["selection.changed"]"#, &["doc.read", "commands"], &imports, &handler, &[cmd]);
    s.plugin_event_frames = true;
    run(&mut s, "select.set", json!({"ids": [b.0]}));
    let st = s.doc().unwrap();
    let (doc, revision, undo) = (st.doc.clone(), st.revision, st.history.undo.len());
    let (paint, appearance, fill_active) = (s.paint.clone(), format!("{:?}", s.active_appearance_item), s.fill_active);
    s.deliver_plugin_events();
    let notices = s.take_plugin_event_notices();
    let failed: Vec<&str> = notices.iter().filter(|n| n.plugin == id && n.level == LogLevel::Error).map(|n| n.text.as_str()).collect();
    assert_eq!(failed.len(), 1, "{notices:?}");
    assert!(failed[0].contains(&format!("a selection.changed handler only reads: `{cmd}` is not a command it may call")), "{}", failed[0]);
    let st = s.doc().unwrap();
    assert!(std::sync::Arc::ptr_eq(&st.doc, &doc), "the same document");
    assert_eq!((st.revision, st.history.undo.len()), (revision, undo), "the same revision and undo history");
    assert_eq!(st.selection.objects, [b], "the selection the user made");
    assert_eq!((&s.paint, format!("{:?}", s.active_appearance_item), s.fill_active), (&paint, appearance, fill_active));
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A refused handler call leaves the Appearance panel's active row and the Fill or Stroke in
/// front as the user chose them, also when the query it called selected art.
#[test]
fn a_refused_handler_call_keeps_the_active_appearance_row() {
    let id = "test.events.appearance";
    let (cmd, args) = ("addon.cleanup.show", r#"{"kind":"duplicateObjects","scope":"document"}"#);
    assert!(!find_command(cmd).unwrap().journal, "a query the journal does not record");
    let mut s = session();
    let (a, _) = (rect(&mut s, 10.0), rect(&mut s, 10.0));
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    let (imports, handler) = calling(cmd, args);
    install(&mut s, id, r#"["layer.changed"]"#, &["doc.read", "commands"], &imports, &handler, &[cmd]);
    run(&mut s, "appearance.setActiveItem", json!({"index": 0}));
    let fill_active = s.fill_active;
    run(&mut s, "layer.new", json!({"name": "Other"}));
    let notices = s.take_plugin_event_notices();
    let failed: Vec<&str> = notices.iter().filter(|n| n.plugin == id && n.level == LogLevel::Error).map(|n| n.text.as_str()).collect();
    assert_eq!(failed.len(), 1, "{notices:?}");
    assert!(failed[0].contains(&format!("a layer.changed handler only reads: `{cmd}` is not a command it may call")), "{}", failed[0]);
    assert_eq!(s.doc().unwrap().selection.objects, [a], "the selection the user made");
    assert!(s.active_appearance_item.is_some(), "the active Appearance row");
    assert_eq!(s.fill_active, fill_active);
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// Module fields that import `vc_call` and hold `cmd` and `args`, and a `vc_event` body that calls
/// `cmd` with `args` and returns what the call returned.
fn calling(cmd: &str, args: &str) -> (String, String) {
    let imports = format!(
        r#"(import "env" "vc_call" (func $vc_call (param i32 i32 i32 i32) (result i32)))
  (data (i32.const 0x2000) "{}") (data (i32.const 0x2100) "{}")"#,
        tpl::string(cmd),
        tpl::string(args)
    );
    let handler = format!("(call $vc_call (i32.const 0x2000) (i32.const {}) (i32.const 0x2100) (i32.const {}))", cmd.len(), args.len());
    (imports, handler)
}

/// A handler that fails is reported once; after three failures in a row its events stop for the
/// session, while each selection the user makes stays.
#[test]
fn a_handler_that_keeps_failing_is_turned_off() {
    let id = "test.events.failing";
    let mut s = session();
    let (a, b) = (rect(&mut s, 10.0), rect(&mut s, 40.0));
    run(&mut s, "select.none", json!({}));
    install(&mut s, id, r#"["selection.changed"]"#, &[], "", "unreachable", &[]);
    let mut notices = Vec::new();
    for ids in [vec![a.0], vec![b.0], vec![a.0], vec![b.0], vec![a.0]] {
        run(&mut s, "select.set", json!({"ids": ids.clone()}));
        assert_eq!(s.doc().unwrap().selection.objects.iter().map(|i| i.0).collect::<Vec<_>>(), ids, "the user's selection stays");
        notices.extend(s.take_plugin_event_notices().into_iter().filter(|n| n.plugin == id).map(|n| n.text));
    }
    assert_eq!(notices.len(), 2, "{notices:?}");
    assert!(notices[0].starts_with("its selection.changed handler failed") && notices[0].contains("trapped"), "{}", notices[0]);
    assert!(notices[1].contains("after 3 failed handlers in a row"), "{}", notices[1]);
    let info = run(&mut s, "plugin.info", json!({"id": id}));
    assert_eq!(info["eventsOff"], json!(true));
    run(&mut s, "plugin.remove", json!({"id": id}));
}
