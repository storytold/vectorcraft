//! WebAssembly plug-ins in the UI (#121): Object › Plug-ins and Effect › Plug-ins, the generated
//! filter dialog with its live preview, plug-in effects in the effect dialog, and installing
//! `.wasm` files through File › Open.

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat as tpl;

use crate::state::Dialog;
use crate::{VectorcraftApp, dialogs, io, menus, theme};

const DESATURATE: &[u8] = include_bytes!("../../../plugins/tests/fixtures/desaturate.wasm");
const DESATURATE_ID: &str = "org.vectorcraft.example.desaturate";

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
}

/// One headless frame of the dialog layer.
fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    let mut out = ctx.run_ui(Default::default(), |ui| dialogs::show(app, ui.ctx()));
    out.textures_delta.clear();
}

fn red_rect(app: &mut VectorcraftApp) -> vectorcraft_doc::NodeId {
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    vectorcraft_doc::NodeId(id)
}

fn fill_rgb(app: &VectorcraftApp, id: vectorcraft_doc::NodeId) -> [f32; 3] {
    app.session.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint().color().unwrap().to_rgb_uncalibrated()
}

fn entry<'a>(entries: &'a [menus::MenuEntry], path: &[&str], label: &str) -> Option<&'a menus::MenuEntry> {
    entries.iter().find(|e| e.path == path && e.label == label)
}

#[test]
fn plugins_are_listed_in_the_object_and_effect_menus() {
    let mut app = app();
    let before = menus::plugin_revision();
    io::open_bytes(&mut app, "desaturate.wasm", DESATURATE, None).unwrap();
    let effect = tpl::module(&tpl::manifest("test.ui.echo", "effect", r#"{"wiggle":{"type":"bool"}}"#), tpl::ECHO, "", 1);
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wat::parse_str(effect).unwrap())})).unwrap();
    assert_ne!(menus::plugin_revision(), before, "the native menu is rebuilt");
    let entries = menus::menu_entries(&app);
    let filter = entry(&entries, &["Object", "Plug-ins"], "Desaturate…").expect("filter listed");
    assert_eq!((filter.command.as_deref(), &filter.params), (Some("plugin.dialog"), &json!({"id": DESATURATE_ID})));
    assert!(entry(&entries, &["Object", "Plug-ins"], "Install Plug-in…").is_some());
    assert!(entry(&entries, &["Object", "Plug-ins"], "Reload Plug-ins").is_some());
    let fx = entry(&entries, &["Effect", "Plug-ins"], "Test test.ui.echo…").expect("effect listed");
    assert_eq!((fx.command.as_deref(), &fx.params), (Some("effect.dialog"), &json!({"effect": "plugin.test.ui.echo"})));
    // Menu clicks on items with params don't turn into the generic parameter dialog.
    assert_eq!(menus::click_target("Desaturate…", "plugin.dialog", &filter.params).0, "plugin.dialog");
    // Their names are the plug-ins' own, not interface text a language catalog has to cover (they
    // are installed for the whole test run, while the catalog tests run).
    let strings = menus::menu_strings();
    for name in ["Desaturate…", "Test test.ui.echo…"] {
        assert!(!strings.contains(name), "{name} is a menu string to translate");
    }
}

#[test]
fn filter_dialog_previews_and_commits_one_undo_step() {
    let mut app = app();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(DESATURATE)})).unwrap();
    let id = red_rect(&mut app);
    let undo = app.session.doc().unwrap().history.undo.len();
    menus::invoke(&mut app, "plugin.dialog", json!({"id": DESATURATE_ID}));
    let d = app.ui.dialog.as_ref().expect("dialog open");
    assert_eq!((d.kind.as_str(), d.str("__label"), d.fields.get("amount")), ("plugin", "Desaturate".to_string(), Some(&json!(100.0))));
    // Agents set fields like `ui.dialog.set` does; the frame previews them.
    app.ui.dialog.as_mut().unwrap().fields.insert("amount".into(), json!(50));
    frame(&mut app);
    assert!(app.session.in_interaction(), "previewing");
    let [r, g, _] = fill_rgb(&app, id);
    assert!(r < 1.0 && g > 0.0, "half desaturated on the canvas: {r} {g}");
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let st = app.session.doc().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(st.history.undo.last().unwrap().label, "Desaturate");
    // Cancel rolls the preview back.
    menus::invoke(&mut app, "plugin.dialog", json!({"id": DESATURATE_ID}));
    frame(&mut app);
    let previewed = fill_rgb(&app, id);
    dialogs::cancel(&mut app);
    assert_ne!(fill_rgb(&app, id), previewed);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    // With params, the menu command runs at once.
    app.run("plugin.dialog", json!({"id": DESATURATE_ID, "params": {"amount": 100}})).unwrap();
    let [r, g, b] = fill_rgb(&app, id);
    assert!((r - g).abs() < 1e-6 && (g - b).abs() < 1e-6);
    assert!(app.run("plugin.dialog", json!({"id": "test.ui.missing"})).is_err());
}

#[test]
fn plugin_effects_open_the_effect_dialog_with_their_schema() {
    let mut app = app();
    let src =
        tpl::module(&tpl::manifest("test.ui.fx", "effect", r#"{"mode":{"type":"choice","options":["a","b"],"default":"b"}}"#), tpl::ECHO, "", 1);
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wat::parse_str(src).unwrap())})).unwrap();
    let id = red_rect(&mut app);
    app.run("effect.dialog", json!({"effect": "plugin.test.ui.fx"})).unwrap();
    let d: &Dialog = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.str("__label"), d.str("mode")), ("effect", "Test test.ui.fx".to_string(), "b".to_string()));
    frame(&mut app);
    dialogs::confirm(&mut app).unwrap();
    let fx = &app.session.doc().unwrap().doc.node(id).unwrap().appearance.effects;
    assert_eq!((fx[0].id.as_str(), &fx[0].params), ("plugin.test.ui.fx", &json!({"mode": "b"})));
    assert_eq!(crate::panels::appearance::effect_label("plugin.test.ui.fx"), "Test test.ui.fx");
    // Editing it again starts from its values.
    app.run("effect.dialog", json!({"effect": "plugin.test.ui.fx", "index": 0})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields.get("mode"), Some(&Value::from("b")));
}

/// A filter `id` named `name` whose manifest lists `permissions` (a JSON list), at a version with
/// a bidirectional override, a tab and a bell in it.
fn asking(id: &str, name: &str, permissions: &str) -> Vec<u8> {
    let manifest = json!({"id": id, "name": name, "version": "3.0\u{202E}\t\u{0007}", "kind": "filter"}).to_string();
    let manifest = format!(r#"{},"permissions":{permissions}}}"#, manifest.trim_end_matches('}'));
    wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap()
}

/// Installing a plug-in that lists permissions asks first, showing its name, id, version, file hash
/// and each permission in plain words, with hidden characters from the manifest escaped. Cancel
/// installs nothing, OK installs it; installing it again with the same permissions doesn't ask, and
/// one that asks for another does.
#[test]
fn installing_a_plugin_that_lists_permissions_asks_first() {
    let mut app = app();
    let id = "test.ui.permissions";
    // U+202E would show the text after it reversed, and U+2066 isolates it; a manifest's name may
    // hold them, though not control characters, which its version may.
    let name = "Tidy\u{202E}fdp.exe\u{2066}";
    let bytes = asking(id, name, r#"["doc.read","doc.write"]"#);
    let data = vectorcraft_format::base64_encode(&bytes);
    let r = app.run("ui.installPlugin", json!({"dataBase64": data, "name": "tidy.wasm"})).unwrap();
    assert_eq!(r["pending"], dialogs::confirm::KIND);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.str("message"), "Install “Tidy\\u{202E}fdp.exe\\u{2066}”, which asks for these permissions?");
    let hash = vectorcraft_plugins::permissions::sha256_hex(&bytes);
    assert_eq!(d.fields.get("__params").map(|p| p["hash"].clone()), Some(json!(hash)), "OK installs the module shown");
    let detail = d.str("detail");
    assert_eq!(
        detail,
        format!(
            "ID: {id}\nVersion: 3.0\\u{{202E}}\\u{{0009}}\\u{{0007}}\nFile SHA-256: {hash}\n\n• Read the objects it is given\n• Change, add and delete objects in the document"
        )
    );
    assert!(!detail.chars().chain(d.str("message").chars()).any(|c| matches!(c, '\u{202E}' | '\u{2066}' | '\t' | '\u{0007}')));
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "not before the user agrees");
    dialogs::cancel(&mut app);
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "Cancel installs nothing");
    // File › Open asks too, and OK installs it with what it lists granted.
    io::open_bytes(&mut app, "tidy.wasm", &bytes, None).unwrap();
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(dialogs::confirm::KIND));
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert!(vectorcraft_plugins::registry::get(id).is_some());
    assert_eq!(vectorcraft_plugins::registry::granted(id).map(|g| g.hash), Some(hash));
    // Again with the same permissions: installed without asking.
    let again = asking(id, "Tidy 2", r#"["doc.write","doc.read"]"#);
    app.run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&again)})).unwrap();
    assert!(app.ui.dialog.is_none(), "the same permissions don't ask again");
    assert_eq!(vectorcraft_plugins::registry::get(id).map(|p| p.manifest().name.clone()), Some("Tidy 2".into()));
    // One that asks for another permission asks again.
    let more = asking(id, "Tidy 3", r#"["doc.read","doc.write","state"]"#);
    let r = app.run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&more)})).unwrap();
    assert_eq!(r["pending"], dialogs::confirm::KIND);
    assert!(app.ui.dialog.as_ref().unwrap().str("detail").ends_with("• Keep data of its own with the document"));
    dialogs::cancel(&mut app);
    assert_eq!(vectorcraft_plugins::registry::get(id).map(|p| p.manifest().name.clone()), Some("Tidy 2".into()), "Cancel keeps the one installed");
    // OK keeps the `replace` it was asked with: `false` leaves the installed one in place.
    app.run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&more), "replace": false})).unwrap();
    let _ = dialogs::confirm(&mut app);
    assert_eq!(vectorcraft_plugins::registry::get(id).map(|p| p.manifest().name.clone()), Some("Tidy 2".into()), "replace: false is kept");
    app.run("plugin.remove", json!({"id": id})).unwrap();
}

/// A plug-in with two commands: Grow, which adds a triangle (`_context.command` is `grow`), and
/// Size, with a parameter, whose `menu` names another menu.
fn commands_plugin(id: &str) -> Vec<u8> {
    let manifest = format!(
        r#"{{"id":"{id}","name":"Cmd Test","version":"1","kind":"filter","commands":[{{"id":"grow","label":"Grow"}},
        {{"id":"size","label":"Size","menu":"File","params":{{"by":{{"type":"number","min":0,"max":10,"default":2}}}}}}]}}"#
    );
    let triangle = r#"{"objects":[{"path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[5,0]},{"p":[5,5]}]}]}}]}"#;
    wat::parse_str(tpl::when_given(&manifest, tpl::Given::Params, r#""command":"grow""#, triangle)).unwrap()
}

fn objects(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, |c| c.len())).sum()
}

/// A plug-in's commands are items of Object › Plug-ins › its name (never the menu a manifest
/// names), entries of Edit › Keyboard Shortcuts with no shortcut of their own, and gone from both
/// once it is removed. A command with parameters opens its dialog.
#[test]
fn plugin_commands_are_menu_items_and_shortcut_entries() {
    let mut app = app();
    let id = "test.ui.cmds";
    let (grow, size) = ("plugin.test.ui.cmds.grow", "plugin.test.ui.cmds.size");
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&commands_plugin(id))})).unwrap();
    let entries = menus::menu_entries(&app);
    let under = ["Object", "Plug-ins", "Cmd Test"];
    let item = entry(&entries, &under, "Grow").expect("Grow is listed");
    assert_eq!((item.command.as_deref(), item.enabled), (Some(grow), true));
    assert!(
        entry(&entries, &under, "Size…").is_some(),
        "Size is listed: {:?}",
        entries.iter().filter(|e| e.path == under).map(|e| &e.label).collect::<Vec<_>>()
    );
    assert!(!entries.iter().any(|e| e.command.as_deref() == Some(size) && e.path.first().map(String::as_str) != Some("Object")), "not in File");
    assert!(!menus::menu_strings().contains("Cmd Test"), "the plug-in's name is not text to translate");

    let listed = crate::shortcut_editor::entry(grow).expect("in Keyboard Shortcuts");
    assert_eq!((listed.label.as_str(), listed.group.as_str()), ("Grow", "Object › Plug-ins › Cmd Test"));
    assert_eq!(crate::shortcut_editor::default_of(grow), None, "a manifest gives no shortcut");
    assert!(crate::keymap::template().bindable(&crate::keymap::command_of(grow)).is_some(), "a shortcut can be given to it");

    // Size asks for its parameter; OK runs it as one undo step named after it.
    let undo = app.session.doc().unwrap().history.undo.len();
    menus::invoke(&mut app, size, json!({}));
    let d = app.ui.dialog.as_ref().expect("Size opens its dialog");
    assert_eq!((d.str("__command"), d.str("__label"), d.fields.get("by")), (size.to_string(), "Size".to_string(), Some(&json!(2.0))));
    frame(&mut app);
    dialogs::confirm(&mut app).unwrap();
    let st = app.session.doc().unwrap();
    assert_eq!((st.history.undo.len(), st.history.undo.last().unwrap().label.as_str()), (undo + 1, "Size"));

    app.run("plugin.remove", json!({"id": id})).unwrap();
    assert!(menus::menu_entries(&app).iter().all(|e| e.command.as_deref() != Some(grow)));
    assert!(crate::shortcut_editor::entries().iter().all(|e| e.key != grow));
    assert!(crate::keymap::template().bindable(&crate::keymap::command_of(grow)).is_none());
}

/// A shortcut the user gives a plug-in command runs it.
#[test]
fn a_shortcut_runs_a_plugin_command() {
    let mut app = app();
    let id = "test.ui.cmds.keys";
    let grow = vectorcraft_engine::cmd::plugin::command_id(id, "grow");
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&commands_plugin(id))})).unwrap();
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, &grow, "Cmd+Alt+Shift+F9");
    crate::shortcut_editor::sync(&app.ui);
    assert_eq!(crate::shortcut_editor::command_for_key("Cmd+Alt+Shift+F9"), Some(grow.as_str()));
    let before = objects(&app);
    let m = if crate::keymap::platform().is_mac() { egui::Modifiers::MAC_CMD } else { egui::Modifiers::CTRL } | egui::Modifiers::COMMAND;
    let m = m | egui::Modifiers::ALT | egui::Modifiers::SHIFT;
    let events = vec![
        egui::Event::ModifiersChanged(m),
        egui::Event::Key { key: egui::Key::F9, physical_key: Some(egui::Key::F9), pressed: true, repeat: false, modifiers: m },
    ];
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
        app.logic(ui.ctx());
        crate::shortcuts::handle(&mut app, ui.ctx());
    });
    out.textures_delta.clear();
    crate::shortcut_editor::sync(&crate::state::UiState::default());
    assert_eq!(objects(&app), before + 1, "the command added its triangle: {}", app.ui.status);
    app.run("plugin.remove", json!({"id": id})).unwrap();
}
