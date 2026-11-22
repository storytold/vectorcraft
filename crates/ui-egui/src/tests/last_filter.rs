//! Object › Plug-ins › Apply Last Filter and Last Filter…: the object filter that last ran in the
//! session runs again with the same parameters, or opens its dialog with them, and each filter's
//! dialog opens with the parameters it last ran with. Both items are off before a filter ran and
//! once that filter was removed or installed again.

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat as tpl;

use crate::dialogs::plugin::{APPLY_LAST, LAST, NO_LAST_FILTER};
use crate::{VectorcraftApp, dialogs, menus, theme};

/// The whole-number parameter `n` of [`filter`]: 0 to 10, 3 by default.
const N: &str = r#"{"n":{"type":"int","min":0,"max":10,"default":3}}"#;

/// An app with a new 200 × 200 pt document.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.allow_plugin_grants_from_requests();
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
}

/// A filter `id` named `name` with `params` that adds a 30 pt wide triangle when it is given `n` 7,
/// and a 50 pt square otherwise.
fn filter(id: &str, name: &str, params: &str) -> Vec<u8> {
    let manifest = format!(r#"{{"id":"{id}","name":"{name}","version":"1","kind":"filter","params":{params}}}"#);
    let triangle = r#"{"objects":[{"path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[30,0]},{"p":[15,20]}]}]}}]}"#;
    let square = r#"{"objects":[{"path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[50,0]},{"p":[50,50]},{"p":[0,50]}]}]}}]}"#;
    wat::parse_str(tpl::when_given_else(&manifest, tpl::Given::Params, r#""n":7"#, triangle, square)).unwrap()
}

/// Installs `wasm`, in place of a plug-in with its id if there is one.
fn install(app: &mut VectorcraftApp, wasm: &[u8]) {
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(wasm), "replace": true})).unwrap();
}

/// The width of what a filter run returned as `ids`, in points.
fn width(app: &VectorcraftApp, ran: &Value) -> f64 {
    let ids: Vec<vectorcraft_doc::NodeId> = ran["ids"].as_array().unwrap().iter().map(|i| vectorcraft_doc::NodeId(i.as_u64().unwrap())).collect();
    let r = app.session.doc().unwrap().doc.bounds_of(&ids, false).unwrap();
    r.x1 - r.x0
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// One headless frame of the dialog layer, which previews the dialog's fields.
fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(Default::default(), |ui| dialogs::show(app, ui.ctx()));
    out.textures_delta.clear();
}

/// Both items are off, and say so when run, until a filter ran; then they are named after it, and
/// Apply runs it again with its parameters on what is selected now (nothing, here), without a
/// dialog, as one undo step.
#[test]
fn apply_last_filter_runs_the_last_filter_again_with_its_parameters() {
    let mut app = app();
    let id = "test.ui.last.apply";
    install(&mut app, &filter(id, "Grow Shape", N));
    let entries = menus::menu_entries(&app);
    let under: Vec<&str> = entries.iter().filter(|e| e.path == ["Object", "Plug-ins"]).map(|e| e.label.as_str()).take(2).collect();
    assert_eq!(under, ["Apply Last Filter", "Last Filter…"], "the first items of Object › Plug-ins");
    for cmd in [APPLY_LAST, LAST] {
        assert!(!menus::enabled(&app, cmd), "{cmd} before any filter ran");
        assert_eq!(app.run(cmd, json!({})), Err(NO_LAST_FILTER.to_string()));
    }
    assert_eq!(menus::dynamic_label(&app, APPLY_LAST, "Apply Last Filter"), "Apply Last Filter");

    let first = app.run("plugin.dialog", json!({"id": id, "params": {"n": 7}})).unwrap();
    assert_eq!(width(&app, &first), 30.0);
    assert_eq!(app.last_filters.last_id(), Some(id));
    assert!(menus::enabled(&app, APPLY_LAST) && menus::enabled(&app, LAST));
    assert_eq!(menus::dynamic_label(&app, APPLY_LAST, "Apply Last Filter"), "Apply Grow Shape");
    assert_eq!(menus::dynamic_label(&app, LAST, "Last Filter…"), "Grow Shape…");
    assert_eq!(menus::display_label(&app, APPLY_LAST, "Apply Last Filter"), "Apply Grow Shape");

    app.run("select.none", json!({})).unwrap();
    let undo = undo_len(&app);
    let again = app.run(APPLY_LAST, json!({})).unwrap();
    assert!(app.ui.dialog.is_none(), "no dialog");
    assert_eq!(width(&app, &again), 30.0, "the same parameters: n 7");
    assert_ne!(again["ids"], first["ids"], "a new run");
    let st = app.session.doc().unwrap();
    assert_eq!((st.history.undo.len(), st.history.undo.last().unwrap().label.as_str()), (undo + 1, "Grow Shape"));
}

/// Last Filter… opens the dialog with the parameters the filter last ran with; Cancel changes
/// nothing, and OK runs it, with a kept preview or without one, the values it ran with becoming
/// the last ones.
#[test]
fn last_filter_opens_its_dialog_with_the_last_parameters() {
    let mut app = app();
    let id = "test.ui.last.dialog";
    install(&mut app, &filter(id, "Grow Dialog…", N));
    app.run("plugin.dialog", json!({"id": id, "params": {"n": 7}})).unwrap();
    assert_eq!(menus::dynamic_label(&app, LAST, "Last Filter…"), "Grow Dialog…", "one ellipsis");
    app.run("select.none", json!({})).unwrap();
    let undo = undo_len(&app);

    app.run(LAST, json!({})).unwrap();
    let d = app.ui.dialog.as_ref().expect("its dialog");
    assert_eq!((d.kind.as_str(), d.str("__plugin"), d.fields.get("n")), (dialogs::plugin::KIND, id.to_string(), Some(&json!(7))));
    frame(&mut app);
    dialogs::cancel(&mut app);
    assert_eq!(undo_len(&app), undo, "Cancel changes nothing");

    // OK with the preview kept.
    app.run(LAST, json!({})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("n".into(), json!(5));
    frame(&mut app);
    assert!(app.session.in_interaction(), "previewing");
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(undo_len(&app), undo + 1);
    app.run(LAST, json!({})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields.get("n"), Some(&json!(5)), "OK made 5 the last value");

    // OK without a preview.
    app.ui.dialog.as_mut().unwrap().fields.insert("n".into(), json!(7));
    app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(false));
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(undo_len(&app), undo + 2);
    app.run("select.none", json!({})).unwrap();
    let again = app.run(APPLY_LAST, json!({})).unwrap();
    assert_eq!(width(&app, &again), 30.0);
}

/// Each filter's own dialog opens with the parameters that filter last ran with, never another
/// filter's; one that hasn't run opens with its defaults.
#[test]
fn each_filter_dialog_opens_with_its_own_last_parameters() {
    let mut app = app();
    let (a, b, c) = ("test.ui.last.own.a", "test.ui.last.own.b", "test.ui.last.own.c");
    for (id, name) in [(a, "Own A"), (b, "Own B"), (c, "Own C")] {
        install(&mut app, &filter(id, name, N));
    }
    app.run("plugin.dialog", json!({"id": a, "params": {"n": 7}})).unwrap();
    app.run("plugin.dialog", json!({"id": b, "params": {"n": 2}})).unwrap();
    app.run("plugin.dialog", json!({"id": a, "params": {"n": 8}})).unwrap();
    app.run("plugin.dialog", json!({"id": b, "params": {"n": 1}})).unwrap();
    assert_eq!(app.last_filters.last_id(), Some(b));
    for (id, n) in [(a, 8), (b, 1), (c, 3)] {
        app.run("plugin.dialog", json!({"id": id})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields.get("n"), Some(&json!(n)), "{id}");
        dialogs::cancel(&mut app);
    }
}

/// A batch step, a direct `plugin.run` and an Action Batch step count; a failed run, a plug-in's own
/// command and a live effect don't.
#[test]
fn only_successful_filter_runs_become_the_last_filter() {
    let mut app = app();
    let id = "test.ui.last.kinds";
    install(&mut app, &filter(id, "Kinds", N));
    assert!(app.run("plugin.run", json!({"id": id, "params": {"n": "seven"}})).is_err());
    assert_eq!(app.last_filters.last_id(), None, "a run that failed");

    let steps = json!({"commands": [{"command": "plugin.run", "params": {"id": id, "params": {"n": 7}}}]});
    app.run("command.batch", steps).unwrap();
    assert_eq!(app.last_filters.last_id(), Some(id), "a batch step");
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields.get("n"), Some(&json!(7)));
    dialogs::cancel(&mut app);

    let other = "test.ui.last.kinds.other";
    install(&mut app, &filter(other, "Other", N));
    app.run("plugin.run", json!({"id": other, "n": 2})).unwrap();
    assert_eq!(app.last_filters.last_id(), Some(other), "a direct run, its parameters as top-level keys");
    app.run("plugin.dialog", json!({"id": other})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields.get("n"), Some(&json!(2)));
    dialogs::cancel(&mut app);

    let cmds = "test.ui.last.cmds";
    let manifest = format!(r#"{{"id":"{cmds}","name":"Cmds","version":"1","kind":"filter","commands":[{{"id":"grow","label":"Grow"}}]}}"#);
    let triangle = r#"{"objects":[{"path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[5,0]},{"p":[5,5]}]}]}}]}"#;
    install(&mut app, &wat::parse_str(tpl::returning(&manifest, triangle)).unwrap());
    app.run("plugin.test.ui.last.cmds.grow", json!({})).unwrap();
    assert_eq!(app.last_filters.last_id(), Some(other), "a plug-in's own command");

    let fx = tpl::module(&tpl::manifest("test.ui.last.fx", "effect", "{}"), tpl::ECHO, "", 1);
    install(&mut app, &wat::parse_str(fx).unwrap());
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    app.run("effect.apply", json!({"effect": "plugin.test.ui.last.fx"})).unwrap();
    assert_eq!(app.last_filters.last_id(), Some(other), "a live effect");

    // Action Batch over the document's data sets counts once a data set ran the step.
    app.run("dataset.new", json!({"name": "a"})).unwrap();
    let steps = json!([{"command": "plugin.run", "params": {"id": id, "params": {"n": 5}}}]);
    let r = app.run("action.batch", json!({"source": "datasets", "commands": steps})).unwrap();
    assert_eq!((r["count"].clone(), r["failed"].clone()), (json!(1), json!(0)), "{r}");
    assert_eq!(app.last_filters.last_id(), Some(id), "an Action Batch step");
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields.get("n"), Some(&json!(5)));
    dialogs::cancel(&mut app);
}

/// Installing the plug-in again, even the same module, another module with its id, one whose
/// manifest no longer takes the parameters, and removing it turn both items off, and the new
/// module's dialog opens with its own defaults. Installing another plug-in keeps them.
#[test]
fn the_last_filter_is_off_once_its_plugin_is_removed_or_replaced() {
    let mut app = app();
    let id = "test.ui.last.stale";
    let wasm = filter(id, "Stale", N);
    install(&mut app, &wasm);
    app.run("plugin.dialog", json!({"id": id, "params": {"n": 7}})).unwrap();
    install(&mut app, &filter("test.ui.last.unrelated", "Unrelated", N));
    assert!(menus::enabled(&app, APPLY_LAST), "another plug-in installed");
    install(&mut app, &wasm);
    assert!(!menus::enabled(&app, APPLY_LAST), "the same module again");
    app.run("plugin.dialog", json!({"id": id, "params": {"n": 7}})).unwrap();

    // Another module and then the first one again: the run with the first one doesn't come back.
    install(&mut app, &filter(id, "Stale", r#"{"n":{"type":"int","min":0,"max":20,"default":3}}"#));
    install(&mut app, &wasm);
    assert!(!menus::enabled(&app, APPLY_LAST), "replaced, then the module that ran installed again");

    install(&mut app, &filter(id, "Stale", r#"{"n":{"type":"choice","options":["a","b"],"default":"b"}}"#));
    for cmd in [APPLY_LAST, LAST] {
        assert!(!menus::enabled(&app, cmd), "{cmd} once the parameters no longer fit");
        assert_eq!(app.run(cmd, json!({})), Err(NO_LAST_FILTER.to_string()));
    }
    assert_eq!(menus::dynamic_label(&app, APPLY_LAST, "Apply Last Filter"), "Apply Last Filter");
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields.get("n"), Some(&json!("b")), "the new module's default");
    dialogs::cancel(&mut app);

    install(&mut app, &filter(id, "Stale", r#"{"n":{"type":"int","min":0,"max":20,"default":3}}"#));
    assert!(!menus::enabled(&app, APPLY_LAST), "another module, though it takes n 7");
    app.run("plugin.dialog", json!({"id": id, "params": {"n": 7}})).unwrap();
    assert!(menus::enabled(&app, APPLY_LAST));
    app.run("plugin.remove", json!({"id": id})).unwrap();
    assert!(!menus::enabled(&app, APPLY_LAST) && !menus::enabled(&app, LAST), "removed");
    assert_eq!(app.run(APPLY_LAST, json!({})), Err(NO_LAST_FILTER.to_string()));

    // A batch that runs the filter and then replaces its module.
    install(&mut app, &wasm);
    let other = filter(id, "Stale", r#"{"n":{"type":"int","min":0,"max":20,"default":3}}"#);
    let steps = json!([
        {"command": "plugin.run", "params": {"id": id, "params": {"n": 7}}},
        {"command": "plugin.install", "params": {"dataBase64": vectorcraft_format::base64_encode(&other), "replace": true}},
    ]);
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert_eq!(app.last_filters.last_id(), Some(id));
    assert!(!menus::enabled(&app, APPLY_LAST) && !menus::enabled(&app, LAST), "replaced after it ran in the batch");

    // A batch that runs the filter and then installs another plug-in keeps it.
    install(&mut app, &wasm);
    let unrelated = filter("test.ui.last.unrelated", "Unrelated", N);
    let steps = json!([
        {"command": "plugin.run", "params": {"id": id, "params": {"n": 7}}},
        {"command": "plugin.install", "params": {"dataBase64": vectorcraft_format::base64_encode(&unrelated), "replace": true}},
    ]);
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert!(menus::enabled(&app, APPLY_LAST), "another plug-in installed after it ran in the batch");

    // A batch that runs the filter, removes it and installs the same module again.
    let steps = json!([
        {"command": "plugin.run", "params": {"id": id, "params": {"n": 7}}},
        {"command": "plugin.remove", "params": {"id": id}},
        {"command": "plugin.install", "params": {"dataBase64": vectorcraft_format::base64_encode(&wasm), "replace": true}},
    ]);
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert!(!menus::enabled(&app, APPLY_LAST), "installed again in the batch after it ran");
}

/// A filter that runs in the background becomes the last filter only when its result is applied:
/// starting it or canceling it doesn't. Apply Last Filter starts it in the background as its menu
/// item does. With no document open both items are off.
#[test]
fn a_background_run_becomes_the_last_filter_only_when_applied() {
    let mut app = app();
    let id = "test.ui.last.heavy";
    let manifest = format!(r#"{{"id":"{id}","name":"Heavy Last","version":"1","kind":"filter","budget":"heavy"}}"#);
    let wasm = wat::parse_str(tpl::spinning(&manifest, 1_000_000_000, r#"{"objects":[]}"#)).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "allowHeavy": true})).unwrap();
    assert_eq!(app.run("plugin.dialog", json!({"id": id})).unwrap()["background"], true);
    assert_eq!(app.last_filters.last_id(), None, "started");
    crate::plugin_runs::cancel_all(&mut app);
    crate::plugin_runs::poll(&mut app);
    assert_eq!(app.last_filters.last_id(), None, "canceled");

    // As its result would be applied: `plugin.run` with its parameters.
    dialogs::plugin::record(&mut app, &json!({"id": id, "params": {}}));
    assert_eq!(app.run(APPLY_LAST, json!({})).unwrap()["background"], true, "in the background, as its menu item runs it");
    crate::plugin_runs::cancel_all(&mut app);

    // A filter with parameters, so Last Filter… would open its dialog.
    let with_params = "test.ui.last.nodoc";
    install(&mut app, &filter(with_params, "No Document", N));
    let mut empty = VectorcraftApp::new(Session::new(), Default::default());
    dialogs::plugin::record(&mut empty, &json!({"id": with_params, "params": {"n": 7}}));
    assert_eq!(empty.last_filters.last_id(), Some(with_params));
    assert!(!menus::enabled(&empty, APPLY_LAST) && !menus::enabled(&empty, LAST), "no document");
    assert_eq!(dialogs::plugin::apply_last(&mut empty), Err("no document open".into()));
    assert_eq!(dialogs::plugin::open_last(&mut empty), Err("no document open".into()));
    assert!(empty.ui.dialog.is_none(), "no dialog opens without a document");
}

/// The native menu reads its rows again when the filter that ran last changes, also when nothing
/// else changed.
#[test]
fn the_native_menu_follows_the_last_filter() {
    let mut app = app();
    let (a, b) = ("test.ui.last.native.a", "test.ui.last.native.b");
    install(&mut app, &filter(a, "Native A", N));
    install(&mut app, &filter(b, "Native B", N));
    dialogs::plugin::record(&mut app, &json!({"id": a, "params": {}}));
    let before = crate::native_menu::state_hash(&app, None);
    dialogs::plugin::record(&mut app, &json!({"id": b, "params": {}}));
    assert_ne!(crate::native_menu::state_hash(&app, None), before);
}
