//! WebAssembly plug-ins in the UI (#121): Object › Plug-ins and Effect › Plug-ins, the generated
//! filter dialog with its live preview, plug-in effects in the effect dialog, and installing
//! `.wasm` files through File › Open.

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat as tpl;

use crate::{VectorcraftApp, dialogs, io, menus};

const DESATURATE: &[u8] = include_bytes!("../../../plugins/tests/fixtures/desaturate.wasm");
const DESATURATE_ID: &str = "org.vectorcraft.example.desaturate";

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
}

pub(super) type Window = egui_kittest::Harness<'static, VectorcraftApp>;

/// `app` in a window that runs whole app frames, each drawn at the time [`at`] gives it, so the
/// dialog host (which lives in the window's context) shows, previews and answers as in the app.
/// Building it draws frames at times of its own, so a test opens the dialog after.
pub(super) fn window(app: VectorcraftApp) -> Window {
    let mut fonts = false;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 800.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !std::mem::replace(&mut fonts, true) {
                crate::testing::install(ui.ctx());
                return;
            }
            app.tick(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    // The window reports its place and the focus, as a host does: the approval counts its time in the focused window.
    let root = h.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default();
    root.inner_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 800.0)));
    root.focused = Some(true);
    // The first frame installs the fonts; the second gives the app its context and host.
    app_step(&mut h);
    h
}

/// One frame of `h` as the hosts run the window's (`testing::part_frame`): the app gives the frame's input, and the
/// commands the pass fired run as the next frame begins.
pub(super) fn app_step(h: &mut Window) {
    let ctx = h.ctx.clone();
    let input = |h: &mut Window| {
        let app = h.state_mut();
        app.adopt_context(&ctx);
        craft::ui::shell::embed::before_input(&ctx, |out| app.frame_input(out));
    };
    input(h);
    h.step();
    input(h);
}

/// Draws a frame of `h` at `time` seconds.
pub(super) fn at(h: &mut Window, time: f64) {
    h.input_mut().time = Some(time);
    app_step(h);
}

/// Draws a frame a tenth of a second after the one before.
fn step(h: &mut Window) {
    let time = h.input_mut().time.unwrap_or(0.0) + 0.1;
    at(h, time);
}

/// Whether the approval's OK is enabled, as a screen reader reads it.
pub(super) fn ok_enabled(h: &Window) -> bool {
    use egui_kittest::kittest::{NodeT, Queryable};
    !h.get_by_label("OK").accesskit_node().is_disabled()
}

/// Clicks the approval's OK as a person does, on a frame at `time` seconds.
pub(super) fn click_ok(h: &mut Window, time: f64) {
    use egui_kittest::kittest::Queryable;
    h.get_by_label("OK").click();
    at(h, time);
}

/// Shows the open install approval frame after frame a quarter of a second apart until its OK is
/// armed, presses it and draws the frame that takes the answer, as a person does.
pub(super) fn approve(h: &mut Window) {
    let mut time = h.input_mut().time.unwrap_or(0.0);
    for _ in 0..12 {
        time += 0.25;
        at(h, time);
        if ok_enabled(h) {
            click_ok(h, time + 0.05);
            at(h, time + 0.3);
            return;
        }
    }
    panic!("the approval's OK never took input");
}

/// The install approval is open.
fn prompt_open(app: &VectorcraftApp) -> bool {
    app.dialogs().is_some_and(|d| d.with(|host| host.get::<dialogs::plugin::InstallPrompt>().is_some()))
}

/// The approval is open and plug-in `id` neither installed nor granted.
pub(super) fn waiting(app: &VectorcraftApp, id: &str) -> bool {
    prompt_open(app) && vectorcraft_plugins::registry::get(id).is_none() && vectorcraft_plugins::registry::granted(id).is_none()
}

/// The id of the dialog on top of the host.
fn top(app: &VectorcraftApp) -> Option<egui::Id> {
    app.dialogs()?.with(|host| host.open_dialogs().last().map(|d| d.id))
}

/// The values of the dialog on top of the host, as automation lists them.
fn fields(app: &VectorcraftApp) -> Value {
    app.dialogs().and_then(|d| d.with(|host| host.open_dialogs().last().and_then(|d| d.fields.clone()))).unwrap_or(Value::Null)
}

/// Sets `field` of the top dialog as `ui.dialog.set` does.
fn set_field(h: &mut Window, field: &str, value: Value) -> Result<(), String> {
    let (ctx, id) = (h.ctx.clone(), top(h.state()).ok_or("no dialog is open")?);
    h.state().dialogs().ok_or("no dialog host")?.with(|host| host.set_json_field(&ctx, id, field, value))
}

/// Answers the top dialog with OK (`ok`) or Cancel and draws the frame that takes the answer.
fn answer(h: &mut Window, ok: bool) {
    let (ctx, id) = (h.ctx.clone(), top(h.state()).expect("a dialog is open"));
    let host = h.state().dialogs().expect("the dialog host");
    // A held OK (the effect dialog's) is filed for the app, which applies it and then closes the dialog.
    let done = host.with(|host| if ok { host.confirm_by_id(&ctx, id) || host.held_state(id).is_some() } else { host.cancel_by_id(&ctx, id) });
    assert!(done, "the dialog took the answer");
    step(h);
    for _ in 0..2 {
        if top(h.state()) != Some(id) {
            return;
        }
        step(h);
    }
    assert_ne!(top(h.state()), Some(id), "the answered dialog closed");
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
    let mut h = window(app());
    h.state_mut().run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(DESATURATE)})).unwrap();
    let id = red_rect(h.state_mut());
    let undo = h.state().session.doc().unwrap().history.undo.len();
    menus::invoke(h.state_mut(), "plugin.dialog", json!({"id": DESATURATE_ID}));
    step(&mut h);
    assert!(top(h.state()).is_some(), "dialog open");
    assert_eq!(fields(h.state())["amount"], json!(100.0));
    // Agents set fields like `ui.dialog.set` does; the frame previews them.
    set_field(&mut h, "amount", json!(50)).unwrap();
    step(&mut h);
    assert!(h.state().session.in_interaction(), "previewing");
    let [r, g, _] = fill_rgb(h.state(), id);
    assert!(r < 1.0 && g > 0.0, "half desaturated on the canvas: {r} {g}");
    answer(&mut h, true);
    assert!(top(h.state()).is_none());
    let st = h.state().session.doc().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(st.history.undo.last().unwrap().label, "Desaturate");
    // Cancel rolls the preview back.
    menus::invoke(h.state_mut(), "plugin.dialog", json!({"id": DESATURATE_ID}));
    step(&mut h);
    let previewed = fill_rgb(h.state(), id);
    answer(&mut h, false);
    assert_ne!(fill_rgb(h.state(), id), previewed);
    assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo + 1);
    // With params, the menu command runs at once.
    h.state_mut().run("plugin.dialog", json!({"id": DESATURATE_ID, "params": {"amount": 100}})).unwrap();
    let [r, g, b] = fill_rgb(h.state(), id);
    assert!((r - g).abs() < 1e-6 && (g - b).abs() < 1e-6);
    assert!(h.state_mut().run("plugin.dialog", json!({"id": "test.ui.missing"})).is_err());
}

#[test]
fn plugin_effects_open_the_effect_dialog_with_their_schema() {
    let mut h = window(app());
    let src =
        tpl::module(&tpl::manifest("test.ui.fx", "effect", r#"{"mode":{"type":"choice","options":["a","b"],"default":"b"}}"#), tpl::ECHO, "", 1);
    h.state_mut().run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wat::parse_str(src).unwrap())})).unwrap();
    let id = red_rect(h.state_mut());
    h.state_mut().run("effect.dialog", json!({"effect": "plugin.test.ui.fx"})).unwrap();
    step(&mut h);
    assert_eq!(fields(h.state())["mode"], json!("b"));
    answer(&mut h, true);
    let fx = &h.state().session.doc().unwrap().doc.node(id).unwrap().appearance.effects;
    assert_eq!((fx[0].id.as_str(), &fx[0].params), ("plugin.test.ui.fx", &json!({"mode": "b"})));
    assert_eq!(crate::panels::appearance::effect_label("plugin.test.ui.fx"), "Test test.ui.fx");
    // Editing it again starts from its values.
    h.state_mut().run("effect.dialog", json!({"effect": "plugin.test.ui.fx", "index": 0})).unwrap();
    step(&mut h);
    assert_eq!(fields(h.state())["mode"], json!("b"));
    answer(&mut h, false);
}

/// A filter `id` named `name` whose manifest lists `permissions` (a JSON list), at a version with
/// a bidirectional override, a tab and a bell in it.
pub(super) fn asking(id: &str, name: &str, permissions: &str) -> Vec<u8> {
    let manifest = json!({"id": id, "name": name, "version": "3.0\u{202E}\t\u{0007}", "kind": "filter"}).to_string();
    let manifest = format!(r#"{},"permissions":{permissions}}}"#, manifest.trim_end_matches('}'));
    wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap()
}

/// Installing a plug-in that lists permissions asks first, showing its name, id, version, file hash
/// and each permission in plain words, with hidden characters from the manifest escaped. Cancel
/// installs nothing, OK installs it; installing the same module again doesn't ask, one with other
/// bytes asks again, even when it lists the same permissions, and in the same dialog whether to
/// take the installed one's place, as does one that asks for another permission.
#[test]
fn installing_a_plugin_that_lists_permissions_asks_first() {
    let mut h = window(app());
    let id = "test.ui.permissions";
    // U+202E would show the text after it reversed, and U+2066 isolates it; a manifest's name may
    // hold them, though not control characters, which its version may.
    let name = "Tidy\u{202E}fdp.exe\u{2066}";
    let bytes = asking(id, name, r#"["doc.read","doc.write"]"#);
    let data = vectorcraft_format::base64_encode(&bytes);
    let r = h.state_mut().run("ui.installPlugin", json!({"dataBase64": data, "name": "tidy.wasm"})).unwrap();
    assert_eq!(r["pending"], dialogs::plugin::INSTALL_KIND);
    let pending = h.state().pending_plugin_install.as_ref().unwrap();
    assert_eq!(pending.message, "Install “Tidy\\u{202E}fdp.exe\\u{2066}”, which asks for these permissions?");
    let hash = vectorcraft_plugins::permissions::sha256_hex(&bytes);
    assert_eq!(pending.params["hash"], json!(hash), "OK installs the module shown");
    let detail = pending.detail.clone();
    assert_eq!(
        detail,
        format!(
            "ID: {id}\nVersion: 3.0\\u{{202E}}\\u{{0009}}\\u{{0007}}\nFile SHA-256: {hash}\n\n• Read the objects it is given\n• Change, add and delete objects in the document"
        )
    );
    assert!(!detail.chars().chain(pending.message.chars()).any(|c| matches!(c, '\u{202E}' | '\u{2066}' | '\t' | '\u{0007}')));
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "not before the user agrees");
    step(&mut h);
    answer(&mut h, false);
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "Cancel installs nothing");
    assert!(h.state().pending_plugin_install.is_none(), "Cancel drops what it asked about");
    // File › Open asks too, and OK installs it with what it lists granted.
    io::open_bytes(h.state_mut(), "tidy.wasm", &bytes, None).unwrap();
    assert!(waiting(h.state(), id));
    approve(&mut h);
    assert!(top(h.state()).is_none());
    assert!(vectorcraft_plugins::registry::get(id).is_some());
    assert_eq!(vectorcraft_plugins::registry::granted(id).map(|g| g.hash), Some(hash));
    // The same module again: installed without asking.
    h.state_mut().run("ui.installPlugin", json!({"dataBase64": data})).unwrap();
    assert!(!prompt_open(h.state()), "the same bytes don't ask again");
    // Other bytes with the same permissions ask for them, from both ways in; Cancel keeps the one
    // installed.
    let again = asking(id, "Tidy 2", r#"["doc.write","doc.read"]"#);
    let r = h.state_mut().run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&again)})).unwrap();
    assert_eq!(r["pending"], dialogs::plugin::INSTALL_KIND, "other bytes ask again");
    assert_eq!(h.state().pending_plugin_install.as_ref().unwrap().message, "Install “Tidy 2”, which asks for these permissions?");
    step(&mut h);
    answer(&mut h, false);
    let installed = || vectorcraft_plugins::registry::get(id).map(|p| p.manifest().name.clone());
    assert_eq!(installed(), Some(name.into()), "Cancel keeps the one installed");
    io::open_bytes(h.state_mut(), "tidy2.wasm", &again, None).unwrap();
    assert!(prompt_open(h.state()), "File › Open asks too");
    // The same dialog says it takes the installed one's place, and OK replaces it; Cancel keeps the
    // one installed and records no grant.
    let pending = h.state().pending_plugin_install.as_ref().unwrap();
    let again_hash = vectorcraft_plugins::permissions::sha256_hex(&again);
    assert!(pending.detail.contains(&format!("File SHA-256: {again_hash}")), "{}", pending.detail);
    assert!(
        pending.detail.ends_with("\n\nThis file is not the plug-in installed before with this ID. It gets the settings and cache kept for that ID."),
        "{}",
        pending.detail
    );
    assert_eq!(pending.params["replace"], json!(true), "OK replaces the one installed");
    step(&mut h);
    answer(&mut h, false);
    assert_eq!(installed(), Some(name.into()), "Cancel keeps the one installed");
    assert_ne!(vectorcraft_plugins::registry::granted(id).map(|g| g.hash), Some(again_hash), "Cancel records no grant");
    h.state_mut().run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&again)})).unwrap();
    approve(&mut h);
    assert!(top(h.state()).is_none(), "one question");
    assert_eq!(installed(), Some("Tidy 2".into()));
    let new_hash = vectorcraft_plugins::permissions::sha256_hex(&again);
    assert_eq!(vectorcraft_plugins::registry::granted(id).map(|g| g.hash), Some(new_hash), "OK records the new module's hash");
    // One that asks for another permission asks again.
    let more = asking(id, "Tidy 3", r#"["doc.read","doc.write","state"]"#);
    let r = h.state_mut().run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&more)})).unwrap();
    assert_eq!(r["pending"], dialogs::plugin::INSTALL_KIND);
    assert!(h.state().pending_plugin_install.as_ref().unwrap().detail.contains("• Keep data of its own with the document"));
    step(&mut h);
    answer(&mut h, false);
    assert_eq!(installed(), Some("Tidy 2".into()), "Cancel keeps the one installed");
    // OK keeps the `replace` it was asked with: `false` leaves the installed one in place.
    h.state_mut().run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&more), "replace": false})).unwrap();
    approve(&mut h);
    assert_eq!(installed(), Some("Tidy 2".into()), "replace: false is kept");
    h.state_mut().run("plugin.remove", json!({"id": id})).unwrap();
}

/// The wait starts over for each question: one that takes the place of the question shown just
/// before a click lands, which the click doesn't approve, and the same question asked again.
#[test]
fn the_install_approval_waits_again_for_a_new_question() {
    let (a, b) = ("test.ui.approval.swap.a", "test.ui.approval.swap.b");
    let module = |id: &str| json!({"dataBase64": vectorcraft_format::base64_encode(&asking(id, "Same", r#"["doc.read"]"#))});
    let mut h = window(app());
    h.state_mut().run("ui.installPlugin", module(a)).unwrap();
    at(&mut h, 10.0);
    at(&mut h, 10.0);
    at(&mut h, 10.6);
    assert!(ok_enabled(&h));
    // Between two frames, a request closes the question and asks about another module.
    let (ctx, id) = (h.ctx.clone(), top(h.state()).unwrap());
    h.state().dialogs().unwrap().with(|host| host.cancel_by_id(&ctx, id));
    h.state_mut().run("ui.installPlugin", module(b)).unwrap();
    click_ok(&mut h, 10.7);
    assert!(waiting(h.state(), b) && vectorcraft_plugins::registry::get(a).is_none(), "the click approves neither");
    assert!(!ok_enabled(&h));
    at(&mut h, 10.9);
    assert!(!ok_enabled(&h), "the new question waits its own delay");
    at(&mut h, 11.8);
    assert!(ok_enabled(&h));
    // The same question again, from the same bytes.
    let (ctx, id) = (h.ctx.clone(), top(h.state()).unwrap());
    h.state().dialogs().unwrap().with(|host| host.cancel_by_id(&ctx, id));
    h.state_mut().run("ui.installPlugin", module(b)).unwrap();
    at(&mut h, 11.9);
    assert!(!ok_enabled(&h), "asked again, it waits again");
    at(&mut h, 12.8);
    assert!(ok_enabled(&h));
    click_ok(&mut h, 12.9);
    assert!(top(h.state()).is_none(), "{}", h.state().ui.status);
    assert!(vectorcraft_plugins::registry::get(b).is_some() && vectorcraft_plugins::registry::get(a).is_none());
    h.state_mut().run("plugin.remove", json!({"id": b})).unwrap();
}

/// A module with other bytes than the one installed with its id asks before taking its place, in
/// the approval dialog, with its id and file hash, also when it asks for a heavy budget.
#[test]
fn installing_a_plugin_over_another_module_asks_first() {
    let mut h = window(app());
    let id = "test.ui.replaces";
    let plain = |output: &str| wat::parse_str(tpl::returning(&format!(r#"{{"id":"{id}","name":"Plain","kind":"filter"}}"#), output)).unwrap();
    let module = |wasm: &[u8]| json!({"dataBase64": vectorcraft_format::base64_encode(wasm)});
    let takes_place = "This file is not the plug-in installed before with this ID. It gets the settings and cache kept for that ID.";
    h.state_mut().run("ui.installPlugin", module(&plain(r#"{"objects":[]}"#))).unwrap();
    assert!(!prompt_open(h.state()), "nothing to take the place of");

    let other = plain(r#"{"objects":[],"error":null}"#);
    let other_hash = vectorcraft_plugins::permissions::sha256_hex(&other);
    let r = h.state_mut().run("ui.installPlugin", module(&other)).unwrap();
    assert_eq!(r, json!({"pending": dialogs::plugin::INSTALL_KIND}));
    let pending = h.state().pending_plugin_install.as_ref().unwrap();
    assert_eq!(pending.message, "Install “Plain” in place of the plug-in with the same ID?");
    assert_eq!(pending.detail, format!("ID: {id}\nFile SHA-256: {other_hash}\n\n{takes_place}"));
    assert_eq!(pending.params["replace"], json!(true), "OK replaces the one installed");
    approve(&mut h);
    assert!(top(h.state()).is_none());
    assert_eq!(vectorcraft_plugins::registry::get(id).map(|p| p.hash().to_string()), Some(other_hash));

    let manifest = format!(r#"{{"id":"{id}","name":"Heavy","kind":"filter","budget":"heavy"}}"#);
    let heavy = wat::parse_str(tpl::spinning(&manifest, 1, r#"{"objects":[]}"#)).unwrap();
    let heavy_hash = vectorcraft_plugins::permissions::sha256_hex(&heavy);
    let r = h.state_mut().run("ui.installPlugin", module(&heavy)).unwrap();
    assert_eq!(r["pending"], dialogs::plugin::INSTALL_KIND);
    let pending = h.state().pending_plugin_install.as_ref().unwrap();
    assert!(pending.detail.ends_with(&format!("\n\nID: {id}\nFile SHA-256: {heavy_hash}\n\n{takes_place}")), "{}", pending.detail);
    assert_eq!(pending.params["replace"], json!(true), "OK replaces the one installed");
    approve(&mut h);
    assert_eq!(vectorcraft_plugins::registry::get(id).map(|p| p.hash().to_string()), Some(heavy_hash));
    h.state_mut().run("plugin.remove", json!({"id": id})).unwrap();
}

/// A plug-in with two commands, each of which adds a triangle when run as a command: Grow, and
/// Size, with a parameter, whose `menu` names another menu.
fn commands_plugin(id: &str) -> Vec<u8> {
    let manifest = format!(
        r#"{{"id":"{id}","name":"Cmd Test","version":"1","kind":"filter","commands":[{{"id":"grow","label":"Grow"}},
        {{"id":"size","label":"Size","menu":"File","params":{{"by":{{"type":"number","min":0,"max":10,"default":2}}}}}}]}}"#
    );
    let triangle = r#"{"objects":[{"path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[5,0]},{"p":[5,5]}]}]}}]}"#;
    wat::parse_str(tpl::when_given(&manifest, tpl::Given::Params, r#""mode":"command""#, triangle)).unwrap()
}

/// A plug-in's commands are items of Object › Plug-ins › its name (never the menu a manifest
/// names), rows of the shortcut catalog with no shortcut of their own, and gone from both once it
/// is removed. A command with parameters opens its dialog.
#[test]
fn plugin_commands_are_menu_items_and_shortcut_entries() {
    let mut h = window(app());
    let id = "test.ui.cmds";
    let (grow, size) = ("plugin.test.ui.cmds.grow", "plugin.test.ui.cmds.size");
    h.state_mut().run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&commands_plugin(id))})).unwrap();
    let entries = menus::menu_entries(h.state());
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
    let row = |command: &str| {
        let defs = crate::keymap::catalog_defs(crate::keymap::platform(), &crate::shortcut_editor::snapshot());
        defs.into_iter().find(|d| d.command == crate::keymap::command_of(command))
    };
    let def = row(grow).expect("a shortcut can be given to it");
    assert!(def.defaults.is_empty(), "a manifest gives no shortcut");

    // Size asks for its parameter; OK runs it as one undo step named after it.
    let undo = h.state().session.doc().unwrap().history.undo.len();
    menus::invoke(h.state_mut(), size, json!({}));
    step(&mut h);
    assert_eq!(fields(h.state())["by"], json!(2.0));
    answer(&mut h, true);
    let st = h.state().session.doc().unwrap();
    assert_eq!((st.history.undo.len(), st.history.undo.last().unwrap().label.as_str()), (undo + 1, "Size"));

    h.state_mut().run("plugin.remove", json!({"id": id})).unwrap();
    assert!(menus::menu_entries(h.state()).iter().all(|e| e.command.as_deref() != Some(grow)));
    assert!(crate::shortcut_editor::entries().iter().all(|e| e.key != grow));
    assert!(row(grow).is_none());
}

/// The `grant` and `allowHeavy` that `ui.installPlugin` is given approve nothing: it asks all the
/// same, and what OK would approve comes from the manifest. A module with nothing to approve
/// installs without either.
#[test]
fn ui_install_plugin_takes_no_grant_or_heavy_budget_from_its_params() {
    let mut h = window(app());
    let id = "test.ui.approval.params";
    let manifest = format!(r#"{{"id":"{id}","name":"Asks","kind":"filter","permissions":["doc.read"]}}"#);
    let asks = wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    let given = |wasm: &[u8]| json!({"dataBase64": vectorcraft_format::base64_encode(wasm), "grant": ["doc.read", "doc.write"], "allowHeavy": true});
    let r = h.state_mut().run("ui.installPlugin", given(&asks)).unwrap();
    assert_eq!(r["pending"], dialogs::plugin::INSTALL_KIND);
    assert!(vectorcraft_plugins::registry::get(id).is_none() && vectorcraft_plugins::registry::granted(id).is_none());
    let pending = h.state().pending_plugin_install.as_ref().unwrap();
    assert_eq!(pending.approval.permissions, ["doc.read"], "the permissions the manifest lists");
    assert_eq!(pending.params["grant"], json!(["doc.read"]));
    assert!(!pending.approval.heavy && pending.params.get("allowHeavy").is_none(), "no heavy budget was asked about");
    step(&mut h);
    answer(&mut h, false);
    assert!(vectorcraft_plugins::registry::get(id).is_none());

    let heavy_id = "test.ui.approval.params.heavy";
    let manifest = format!(r#"{{"id":"{heavy_id}","name":"Heavy","kind":"filter","budget":"heavy"}}"#);
    let heavy = wat::parse_str(tpl::spinning(&manifest, 1, r#"{"objects":[]}"#)).unwrap();
    let r = h.state_mut().run("ui.installPlugin", given(&heavy)).unwrap();
    assert_eq!(r["pending"], dialogs::plugin::INSTALL_KIND, "allowHeavy skips no question");
    assert!(vectorcraft_plugins::registry::get(heavy_id).is_none());
    step(&mut h);
    answer(&mut h, false);

    let plain_id = "test.ui.approval.params.plain";
    let manifest = format!(r#"{{"id":"{plain_id}","name":"Plain","kind":"filter"}}"#);
    let plain = wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    let r = h.state_mut().run("ui.installPlugin", given(&plain)).unwrap();
    step(&mut h);
    assert!(r.get("pending").is_none() && !h.state().any_dialog_open(), "{r}");
    assert!(vectorcraft_plugins::registry::get(plain_id).is_some());
    assert!(vectorcraft_plugins::registry::granted(plain_id).is_none(), "the grant it was given isn't recorded");
    h.state_mut().run("plugin.remove", json!({"id": plain_id})).unwrap();
}

/// An action's step can't grant what only a person approves: a played `plugin.install` with `grant`
/// is refused at Accelerated and at Step by Step, and nothing is installed.
#[test]
fn an_action_step_cannot_grant_a_plugin_its_permissions() {
    use crate::panels::actions::tests_support::{action, push_set};
    use crate::panels::actions::{PlaybackSpeed, play};
    let mut app = app();
    let id = "test.ui.approval.action";
    let manifest = format!(r#"{{"id":"{id}","name":"Asks","kind":"filter","permissions":["doc.read"]}}"#);
    let wasm = wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    let step = json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": ["doc.read"]});
    push_set(&mut app, "Test", vec![action("Install", &[("plugin.install", step)])]);
    // The set added after the Default Actions.
    let set = (0..).find(|&i| crate::panels::actions::model::set_name(&app.ui.actions, i).is_none_or(|name| name == "Test")).unwrap_or(0);
    for speed in [PlaybackSpeed::Accelerated, PlaybackSpeed::Step] {
        app.ui.action_playback.speed = speed;
        let e = play(&mut app, set, 0).unwrap_err();
        assert!(e.contains("take effect only in a session started with --allow-plugin-grants"), "{e}");
        assert!(vectorcraft_plugins::registry::get(id).is_none() && vectorcraft_plugins::registry::granted(id).is_none());
    }
}

/// A plug-in of API level 2 that lists `progress` names it in plain words when it asks.
#[test]
fn installing_a_plugin_that_reports_progress_says_so() {
    let mut h = window(app());
    let id = "test.ui.permissions.progress";
    let manifest = format!(r#"{{"id":"{id}","name":"Count","kind":"filter","api":2,"permissions":["doc.read","progress"]}}"#);
    let bytes = wat::parse_str(tpl::reporting(&manifest, "(call $vc_progress (i64.const 1) (i64.const 2))", r#"{"objects":[]}"#)).unwrap();
    let r = h.state_mut().run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap();
    assert_eq!(r["pending"], dialogs::plugin::INSTALL_KIND);
    let detail = h.state().pending_plugin_install.as_ref().unwrap().detail.clone();
    assert!(detail.ends_with("• Read the objects it is given\n• Show its progress, see Cancel and write log lines"), "{detail}");
    approve(&mut h);
    assert_eq!(vectorcraft_plugins::registry::granted(id).map(|g| g.permissions.len()), Some(2));
    h.state_mut().run("plugin.remove", json!({"id": id})).unwrap();
}
