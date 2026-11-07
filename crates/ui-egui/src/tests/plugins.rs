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
