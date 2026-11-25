//! WebAssembly plug-ins in the UI (#121): Object › Plug-ins and Effect › Plug-ins, the generated
//! filter dialog with its live preview, plug-in effects in the effect dialog, and installing
//! `.wasm` files through File › Open.

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat as tpl;

use crate::i18n::LangExt as _;
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
    let app = h.state_mut();
    app.adopt_context(&ctx);
    craft::ui::shell::embed::before_input(&ctx, |out| app.frame_input(out));
    h.step();
    crate::testing::run_fired(h.state_mut(), &ctx);
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

/// A filter at API level 2 named `name` whose commands are `commands` (a JSON list).
fn placing_plugin(id: &str, name: &str, commands: &str) -> String {
    placing_kind(id, name, "filter", commands)
}

/// A plug-in of `kind` at API level 2 named `name` whose commands are `commands` (a JSON list).
fn placing_kind(id: &str, name: &str, kind: &str, commands: &str) -> String {
    let manifest = format!(r#"{{"id":"{id}","name":"{name}","api":2,"version":"1","kind":"{kind}","commands":{commands}}}"#);
    vectorcraft_format::base64_encode(&wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap())
}

/// A command whose manifest's `menu` names another menu is listed there, after the built-in item
/// `after` names or at the end of the menu, with its plug-in's name after its label, and not in its
/// plug-in's submenu; one with no `menu` stays there. Commands of two plug-ins at the same spot are
/// listed in plug-in id order whichever was installed first. Keyboard Shortcuts and the command
/// palette list it under its menu, and removing its plug-in takes it out of every menu.
#[test]
fn a_plugin_command_is_placed_in_the_menu_its_manifest_names() {
    let mut app = app();
    let (ant, bee) = ("test.ui.place.ant", "test.ui.place.bee");
    let low = |id: &str| vectorcraft_engine::cmd::plugin::command_id(id, "low");
    // Bee is installed first; Ant's id sorts first.
    let bee_commands = r#"[{"id":"low","label":"Low Area","menu":{"path":["Select"],"after":"Inverse"}},
        {"id":"tail","label":"Tail","menu":{"path":["Select"],"after":"No Such Item"}}]"#;
    app.run("plugin.install", json!({"dataBase64": placing_plugin(bee, "Bee", bee_commands)})).unwrap();
    let ant_commands = r#"[{"id":"low","label":"Low Area","menu":{"path":["Select"],"after":"Inverse…"},"params":{"area":{"type":"number","min":0,"max":100,"default":4}}},
        {"id":"same","label":"Same Area","menu":{"path":["Select","Same"]}},
        {"id":"lost","label":"Lost Anchor","menu":{"path":["Select"],"after":"No Such Item"}},
        {"id":"end","label":"End","menu":{"path":["Select"]}},
        {"id":"edges","label":"Edges","menu":{"path":["View"],"after":"Show Edges"}},
        {"id":"stay","label":"Stay"}]"#;
    app.run("plugin.install", json!({"dataBase64": placing_plugin(ant, "Ant", ant_commands)})).unwrap();

    let entries = menus::menu_entries(&app);
    let select: Vec<&menus::MenuEntry> = entries.iter().filter(|e| e.path == ["Select"]).collect();
    let at = |command: &str| select.iter().position(|e| e.command.as_deref() == Some(command));
    let inverse = at("select.inverse").expect("Inverse is listed");
    assert_eq!((at(&low(ant)), at(&low(bee))), (Some(inverse + 1), Some(inverse + 2)), "after Inverse, Ant's before Bee's");
    assert_eq!(select[inverse + 1].label, "Low Area (Ant)…", "the item names its plug-in");
    assert_eq!(select[inverse + 2].label, "Low Area (Bee)");
    // An `after` that names no item means the end of the menu, as no `after` does, and the end keeps
    // plug-in id order and then manifest order too.
    let command = vectorcraft_engine::cmd::plugin::command_id;
    let lost = command(ant, "lost");
    let tail = [at(&lost), at(&command(ant, "end")), at(&command(bee, "tail"))];
    let n = select.len();
    assert_eq!(tail, [Some(n - 3), Some(n - 2), Some(n - 1)], "Ant's two, then Bee's, at the end");
    let same = vectorcraft_engine::cmd::plugin::command_id(ant, "same");
    assert!(entry(&entries, &["Select", "Same"], "Same Area (Ant)").is_some_and(|e| e.command.as_deref() == Some(same.as_str())));
    assert!(menus::menu_strings().contains("Same"), "Select › Same is still a built-in submenu");
    // An item that switches between two labels is matched by either of them.
    let view: Vec<&menus::MenuEntry> = entries.iter().filter(|e| e.path == ["View"]).collect();
    let edges = view.iter().position(|e| e.command.as_deref() == Some("view.edges")).expect("Hide Edges is listed");
    assert_eq!(view.get(edges + 1).and_then(|e| e.command.clone()), Some(command(ant, "edges")), "after the edges toggle");

    // Each placed command is in one menu only; Stay is still in Ant's submenu.
    for command in [low(ant), low(bee), same.clone(), lost.clone()] {
        assert_eq!(entries.iter().filter(|e| e.command.as_deref() == Some(command.as_str())).count(), 1, "{command}");
    }
    assert!(entry(&entries, &["Object", "Plug-ins", "Ant"], "Stay").is_some());
    let bee_item = entry(&entries, &["Object", "Plug-ins"], "Bee").expect("Bee is a plain item again: all its commands are placed");
    assert_eq!(bee_item.command.as_deref(), Some("plugin.dialog"));

    let listed = crate::shortcut_editor::entry(&low(ant)).expect("in Keyboard Shortcuts");
    assert_eq!((listed.label.as_str(), listed.group.as_str()), ("Low Area (Ant)", "Select"));
    assert!(crate::palette::items().iter().any(|(name, id)| *id == low(bee) && name.ends_with("Low Area (Bee)")), "in the command palette");

    app.run("plugin.remove", json!({"id": ant})).unwrap();
    let entries = menus::menu_entries(&app);
    assert!(entries.iter().all(|e| e.command.as_deref().is_none_or(|c| !c.starts_with("plugin.test.ui.place.ant."))), "Ant's commands are gone");
    assert!(entries.iter().any(|e| e.path == ["Select"] && e.command.as_deref() == Some(low(bee).as_str())));
    app.run("plugin.remove", json!({"id": bee})).unwrap();
    assert!(menus::menu_entries(&app).iter().all(|e| e.command.as_deref() != Some(low(bee).as_str())));
    assert!(crate::shortcut_editor::entries().iter().all(|e| e.key != low(bee)));
}

/// Commands placed after an item that switches between two labels are one spot whichever label
/// their `after` names: they keep manifest order within a plug-in and plug-in id order across
/// plug-ins.
#[test]
fn both_labels_of_a_switching_item_are_one_spot() {
    let mut app = app();
    let (ape, bat) = ("test.ui.place.ape", "test.ui.place.bat");
    let command = vectorcraft_engine::cmd::plugin::command_id;
    let ape_commands = r#"[{"id":"first","label":"First","menu":{"path":["View"],"after":"Hide Bounding Box"}},
        {"id":"second","label":"Second","menu":{"path":["View"],"after":"Show Bounding Box"}}]"#;
    let bat_commands = r#"[{"id":"third","label":"Third","menu":{"path":["View"],"after":"Hide Bounding Box"}}]"#;
    // Bat is installed first; Ape's id sorts first.
    app.run("plugin.install", json!({"dataBase64": placing_plugin(bat, "Bat", bat_commands)})).unwrap();
    app.run("plugin.install", json!({"dataBase64": placing_plugin(ape, "Ape", ape_commands)})).unwrap();

    let entries = menus::menu_entries(&app);
    let view: Vec<&menus::MenuEntry> = entries.iter().filter(|e| e.path == ["View"]).collect();
    let toggle = view.iter().position(|e| e.command.as_deref() == Some("view.boundingBox")).expect("Hide Bounding Box is listed");
    let after: Vec<Option<String>> = (1..=3).map(|i| view.get(toggle + i).and_then(|e| e.command.clone())).collect();
    assert_eq!(after, [Some(command(ape, "first")), Some(command(ape, "second")), Some(command(bat, "third"))]);
    for id in [ape, bat] {
        app.run("plugin.remove", json!({"id": id})).unwrap();
    }
}

/// A live effect whose commands are all placed elsewhere is a single item in Effect › Plug-ins. A
/// filter with the same name placing a command with the same label at the same spot keeps it in its
/// own submenu, so the menu shows that label once. Replaced by a module whose command has no
/// `menu`, the effect's command leaves the menu it was placed in and is listed once, in its
/// submenu.
#[test]
fn placed_commands_of_a_live_effect_and_a_clashing_label() {
    let mut app = app();
    let (eff, twin) = ("test.ui.place.eff", "test.ui.place.twin");
    let command = vectorcraft_engine::cmd::plugin::command_id;
    let placed = r#"[{"id":"low","label":"Low Area","menu":{"path":["Select"],"after":"Inverse"}}]"#;
    app.run("plugin.install", json!({"dataBase64": placing_kind(eff, "Glow", "effect", placed)})).unwrap();
    app.run("plugin.install", json!({"dataBase64": placing_plugin(twin, "Glow", placed)})).unwrap();

    let entries = menus::menu_entries(&app);
    let (low, twin_low) = (command(eff, "low"), command(twin, "low"));
    let in_select: Vec<&menus::MenuEntry> = entries.iter().filter(|e| e.path == ["Select"] && e.label == "Low Area (Glow)").collect();
    assert_eq!(in_select.len(), 1, "the label is listed once");
    assert_eq!(in_select[0].command.as_deref(), Some(low.as_str()), "the plug-in with the lower id takes the spot");
    let own = entry(&entries, &["Effect", "Plug-ins"], "Glow").expect("the effect is a single item");
    assert!(own.command.as_deref().is_some_and(|c| !vectorcraft_engine::cmd::plugin::is_command_id(c)), "{:?}", own.command);
    assert!(entries.iter().all(|e| e.path != ["Effect", "Plug-ins", "Glow"]), "no submenu is left");
    assert!(entry(&entries, &["Object", "Plug-ins", "Glow"], "Low Area").is_some_and(|e| e.command.as_deref() == Some(twin_low.as_str())));
    let listed = crate::shortcut_editor::entry(&twin_low).expect("in Keyboard Shortcuts");
    assert_eq!((listed.label.as_str(), listed.group.as_str()), ("Low Area", "Object › Plug-ins › Glow"));

    // An item whose two labels share no first word is matched by either label too.
    let global = "test.ui.place.global";
    let after_stop = r#"[{"id":"next","label":"Next","menu":{"path":["Select"],"after":"Stop Global Edit"}}]"#;
    app.run("plugin.install", json!({"dataBase64": placing_plugin(global, "Global", after_stop)})).unwrap();
    let entries = menus::menu_entries(&app);
    let select: Vec<&menus::MenuEntry> = entries.iter().filter(|e| e.path == ["Select"]).collect();
    let toggle = select.iter().position(|e| e.command.as_deref() == Some("select.globalEdit")).expect("Start Global Edit is listed");
    assert_eq!(select.get(toggle + 1).and_then(|e| e.command.clone()), Some(command(global, "next")));
    app.run("plugin.remove", json!({"id": global})).unwrap();

    let unplaced = placing_kind(eff, "Glow", "effect", r#"[{"id":"low","label":"Low Area"}]"#);
    app.run("plugin.install", json!({"dataBase64": unplaced, "replace": true})).unwrap();
    let entries = menus::menu_entries(&app);
    assert_eq!(entries.iter().filter(|e| e.command.as_deref() == Some(low.as_str())).count(), 1, "listed once");
    assert!(entry(&entries, &["Effect", "Plug-ins", "Glow"], "Low Area").is_some(), "back in its submenu");
    assert!(
        entry(&entries, &["Select"], "Low Area (Glow)").is_some_and(|e| e.command.as_deref() == Some(twin_low.as_str())),
        "the filter's takes the spot"
    );
    for id in [eff, twin] {
        app.run("plugin.remove", json!({"id": id})).unwrap();
    }
}

/// Every menu a manifest's `menu` may name is a built-in menu of the menu bar, so a command placed
/// there is never left in its plug-in's submenu while Keyboard Shortcuts lists it under that menu.
#[test]
fn every_menu_a_plugin_may_name_is_in_the_menu_bar() {
    fn find<'a>(items: &'a [menus::Node], label: &str) -> Option<&'a [menus::Node]> {
        items.iter().find_map(|it| match it {
            menus::Node::Submenu { label: l, children, .. } if l == label => Some(children.as_slice()),
            _ => None,
        })
    }
    let tree = menus::menu_tree();
    for place in vectorcraft_plugins::manifest::MENU_PLACES {
        let (first, rest) = place.split_first().unwrap();
        let mut menu = tree.iter().find(|(title, _)| title == first).map(|(_, items)| items.as_slice());
        for segment in rest {
            menu = menu.and_then(|m| find(m, segment));
        }
        assert!(menu.is_some(), "{place:?} is in the menu bar");
    }
}

/// Every engine command whose menu item switches between two labels is in the engine's
/// `SWITCHING_LABELS`, so a placed plug-in command that reads like its other label stays in its
/// plug-in's submenu without the desktop app too.
#[test]
fn engine_commands_with_two_labels_are_known_to_the_engine() {
    use vectorcraft_engine::cmd::plugin::{SWITCHING_LABELS, other_state};
    for spec in vectorcraft_engine::cmd::command_specs() {
        let switches = menus::DYNAMIC_LABELS.contains(&spec.label)
            && other_state(spec.label).is_some_and(|other| menus::DYNAMIC_LABELS.contains(&other.as_str()));
        assert_eq!(SWITCHING_LABELS.contains(&spec.label), switches, "{} ({})", spec.id, spec.label);
    }
}

/// Every item of a menu a plug-in command may be placed in, other than an engine command of that
/// menu, is in the engine's `DESKTOP_MENU_ITEMS` with both of its labels, so a placed plug-in
/// command that reads like it stays in its plug-in's submenu without the desktop app too.
#[test]
fn desktop_items_of_placeable_menus_are_known_to_the_engine() {
    use vectorcraft_engine::cmd::plugin::{DESKTOP_MENU_ITEMS, SWITCHING_LABELS, other_state};
    let key = |l: &str| l.chars().filter(|ch| ch.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let tree = menus::menu_tree();
    let mut missing = Vec::new();
    for place in vectorcraft_plugins::manifest::MENU_PLACES {
        let (first, rest) = place.split_first().unwrap();
        let mut items = tree.iter().find(|(title, _)| title == first).map(|(_, items)| items.as_slice()).unwrap();
        for segment in rest {
            items = items
                .iter()
                .find_map(|it| match it {
                    menus::Node::Submenu { label, children, .. } if label.as_str() == *segment => Some(children.as_slice()),
                    _ => None,
                })
                .unwrap();
        }
        let engine_reads = |label: &str| {
            vectorcraft_engine::cmd::command_specs().iter().any(|s| {
                s.menu == *place
                    && (key(s.label) == key(label)
                        || SWITCHING_LABELS.contains(&s.label) && other_state(s.label).is_some_and(|o| key(&o) == key(label)))
            })
        };
        for it in items {
            let label = match it {
                menus::Node::Item(i) if !menus::lists_plugin(&i.command.id, &i.command.params) => i.label.as_str(),
                menus::Node::Submenu { label, children, .. } if !menus::holds_plugin_commands(children) => label.as_str(),
                _ => continue,
            };
            let mut labels = vec![label.to_string()];
            if menus::DYNAMIC_LABELS.contains(&label) {
                labels.extend(other_state(label));
            }
            for l in labels {
                let known = DESKTOP_MENU_ITEMS.iter().any(|(path, d)| path == place && key(d) == key(&l));
                if !engine_reads(&l) && !known && !missing.contains(&(place, l.clone())) {
                    missing.push((place, l));
                }
            }
        }
    }
    let rows: Vec<String> = missing.iter().map(|(place, l)| format!("    (&{place:?}, {l:?}),")).collect();
    assert!(missing.is_empty(), "missing from DESKTOP_MENU_ITEMS:\n{}", rows.join("\n"));
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

/// The approval names a plug-in whose manifest translates its name in the UI language and also
/// gives the manifest's own name, both escaped; a language without a translation shows the
/// manifest's name alone. What OK installs stays bound to the id and hash shown.
#[test]
fn the_install_approval_shows_the_translated_and_the_manifests_own_name() {
    let mut h = window(app());
    let id = "test.ui.translated_approval";
    let manifest = json!({"id": id, "name": "Tidy", "version": "1", "kind": "filter", "permissions": ["doc.read"],
        "translations": {"de": {"name": "Aufräumen"}}})
    .to_string();
    let bytes = wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    let hash = vectorcraft_plugins::permissions::sha256_hex(&bytes);
    let data = vectorcraft_format::base64_encode(&bytes);
    let ask = |h: &mut Window, lang: &str| {
        crate::i18n::set_current(crate::i18n::Lang::from_code(lang).unwrap());
        let r = h.state_mut().run("ui.installPlugin", json!({ "dataBase64": data }));
        let pending = h.state().pending_plugin_install.as_ref().map(|p| (p.message.clone(), p.detail.clone(), p.params["hash"].clone()));
        crate::i18n::set_current(crate::i18n::Lang::EN);
        step(h);
        answer(h, false);
        assert_eq!(r.unwrap()["pending"], dialogs::plugin::INSTALL_KIND);
        pending.unwrap()
    };
    let (message, detail, shown_hash) = ask(&mut h, "de");
    assert!(message.contains("Aufräumen") && !message.contains("Tidy"), "{message}");
    assert!(detail.contains("ID: test.ui.translated_approval"), "{detail}");
    assert!(detail.contains("Tidy"), "the manifest's own name: {detail}");
    assert!(!detail.contains("Aufräumen"), "{detail}");
    assert_eq!(shown_hash, json!(hash));
    let (message, detail, _) = ask(&mut h, "en");
    assert_eq!(message, "Install “Tidy”, which asks for these permissions?");
    assert_eq!(detail.lines().next(), Some("ID: test.ui.translated_approval"));
    assert_eq!(detail.lines().nth(1), Some("Version: 1"), "no second name line: {detail}");
    assert!(vectorcraft_plugins::registry::get(id).is_none());
}

/// The label of each item of `items`, a submenu's as `name ›` followed by its items'.
fn menu_labels(items: &[menus::Node]) -> Vec<String> {
    items
        .iter()
        .flat_map(|it| match it {
            menus::Node::Item(i) => vec![i.label.clone()],
            menus::Node::Heading(label) => vec![label.clone()],
            menus::Node::Submenu { label, children, .. } => std::iter::once(format!("{label} ›")).chain(menu_labels(children)).collect(),
            _ => Vec::new(),
        })
        .collect()
}

/// Object › Plug-ins, Effect › Plug-ins and File › Open With name a plug-in and its commands in the
/// UI language when its manifest's `translations` give them: German text in German, `pt` text for
/// Brazilian Portuguese without its own, and the manifest's own text in any other language.
#[test]
fn plugin_menus_show_the_plugins_names_in_the_ui_language() {
    let mut app = app();
    let (filter, effect, importer) = ("test.ui.translated.filter", "test.ui.translated.effect", "test.ui.translated.importer");
    let translations = r#""translations":{"de":{"name":"Aufräumen","commands":{"grow":{"label":"Wachsen"}}},"pt":{"name":"Arrumar"}}"#;
    let filter_manifest = format!(
        r#"{{"id":"{filter}","name":"Tidy","kind":"filter","commands":[{{"id":"grow","label":"Grow","params":{{"by":{{"type":"bool"}}}}}}],{translations}}}"#
    );
    let effect_manifest = format!(
        r#"{{"id":"{effect}","name":"Wobble","kind":"effect","params":{{"on":{{"type":"bool"}}}},"translations":{{"de":{{"name":"Wackeln"}}}}}}"#
    );
    let importer_manifest =
        format!(r#"{{"id":"{importer}","name":"Plotter","kind":"importer","extensions":["tpx"],"translations":{{"de":{{"name":"Zeichner"}}}}}}"#);
    for manifest in [filter_manifest, effect_manifest, importer_manifest] {
        let module = wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
        app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&module)})).unwrap();
    }
    let shown = |lang: &str| {
        crate::i18n::set_current(crate::i18n::Lang::from_code(lang).expect("language registered"));
        let effects = dialogs::plugin::effect_menu().map(|m| menu_labels(&[m])).unwrap_or_default();
        let all = [menu_labels(&dialogs::plugin::object_menu()), effects, menu_labels(&dialogs::plugin::open_with_menu())].concat();
        crate::i18n::set_current(crate::i18n::Lang::EN);
        all
    };
    let de = shown("de");
    for label in ["Aufräumen ›", "Aufräumen", "Wachsen…", "Wackeln…", "Zeichner…"] {
        assert!(de.contains(&label.to_string()), "{label} in German: {de:?}");
    }
    // Other tests' plug-ins may be installed meanwhile: only this test's names are checked.
    for label in ["Tidy ›", "Tidy", "Grow…", "Wobble…", "Plotter…"] {
        assert!(!de.contains(&label.to_string()), "{label} in German: {de:?}");
    }
    let pt_br = shown("pt-br");
    assert!(pt_br.contains(&"Arrumar ›".to_string()) && pt_br.contains(&"Grow…".to_string()), "pt, then the manifest: {pt_br:?}");
    for lang in ["en", "fr"] {
        let other = shown(lang);
        for label in ["Tidy ›", "Grow…", "Wobble…", "Plotter…"] {
            assert!(other.contains(&label.to_string()), "{label} in {lang}: {other:?}");
        }
    }
    // A German item is still the plug-in's own name, not text a catalog translates, and Keyboard
    // Shortcuts lists the filter and its command as the menu does, in the language it is shown in.
    let grow = format!("plugin.{filter}.grow");
    let rows = |lang: &str| {
        crate::i18n::set_current(crate::i18n::Lang::from_code(lang).unwrap());
        let names = menus::shows_a_name("effect.dialog", "Wackeln…");
        let row = |key: &str| crate::shortcut_editor::entry(key).map(|e| (e.label.clone(), e.group.clone()));
        let rows = (names, row(&grow), row(&format!("plugin:{filter}")));
        crate::i18n::set_current(crate::i18n::Lang::EN);
        rows
    };
    let (names, command, own) = rows("de");
    assert!(names);
    assert_eq!(command, Some(("Wachsen".to_string(), "Object › Plug-ins › Aufräumen".to_string())));
    assert_eq!(own.map(|r| r.0), Some("Aufräumen".to_string()));
    let (_, command, _) = rows("en");
    assert_eq!(command, Some(("Grow".to_string(), "Object › Plug-ins › Tidy".to_string())));
    // Last Effect names the effect as Effect › Plug-ins does.
    app.last_effect = Some((vectorcraft_plugins::effect::effect_id(effect), json!({})));
    let last = |lang: &str| {
        crate::i18n::set_current(crate::i18n::Lang::from_code(lang).unwrap());
        let shown = (menus::display_label(&app, "effect.last", "Last Effect…"), menus::dynamic_label(&app, "effect.last", "Last Effect…"));
        crate::i18n::set_current(crate::i18n::Lang::EN);
        shown
    };
    let (de, de_dynamic) = last("de");
    assert!(de.contains("Wackeln…") && de_dynamic == "Last Effect: Wackeln…", "{de} / {de_dynamic}");
    assert_eq!(last("en"), ("Last Effect: Wobble…".to_string(), "Last Effect: Wobble…".to_string()));
    app.last_effect = None;
    for id in [filter, effect, importer] {
        app.run("plugin.remove", json!({"id": id})).unwrap();
    }
}

/// A command placed in another menu shows its label and its plug-in's name in the UI language there
/// and in Keyboard Shortcuts, and the manifest's own text in a language the plug-in does not
/// translate.
#[test]
fn a_placed_plugin_command_shows_the_plugins_translation() {
    let mut app = app();
    let id = "test.ui.translated.placed";
    let manifest = format!(
        r#"{{"id":"{id}","name":"Ridge","api":2,"version":"1","kind":"filter","commands":[{{"id":"low","label":"Low Area","menu":{{"path":["Object","Clipping Mask"]}}}}],"translations":{{"de":{{"name":"Grat","commands":{{"low":{{"label":"Tiefe Fläche"}}}}}}}}}}"#
    );
    let module = wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&module)})).unwrap();
    let low = vectorcraft_engine::cmd::plugin::command_id(id, "low");
    let shown = |app: &VectorcraftApp, lang: &str| {
        crate::i18n::set_current(crate::i18n::Lang::from_code(lang).unwrap());
        let item = menus::menu_entries(app).into_iter().find(|e| e.command.as_deref() == Some(low.as_str())).map(|e| (e.path, e.label));
        let row = crate::shortcut_editor::entry(&low).map(|e| (e.group, e.label));
        crate::i18n::set_current(crate::i18n::Lang::EN);
        (item, row)
    };
    let (de, en) = (shown(&app, "de"), shown(&app, "en"));
    app.run("plugin.remove", json!({"id": id})).unwrap();
    // Other tests count the items of Select and View: this one places its command in a menu no other test uses.
    let menu = || vec!["Object".to_string(), "Clipping Mask".to_string()];
    assert_eq!(de.0, Some((menu(), "Tiefe Fläche (Grat)".to_string())));
    assert_eq!(de.1, Some(("Object › Clipping Mask".to_string(), "Tiefe Fläche (Grat)".to_string())));
    assert_eq!(en.0, Some((menu(), "Low Area (Ridge)".to_string())));
    assert_eq!(en.1, Some(("Object › Clipping Mask".to_string(), "Low Area (Ridge)".to_string())));
}

/// Last Effect shows a live-effect plug-in's translated name as the plug-in gives it, even where it
/// matches text the app translates: "Open" in German stays "Open".
#[test]
fn last_effect_leaves_a_plugins_translated_name_as_it_gives_it() {
    let mut app = app();
    let effect = "test.ui.translated.last_effect";
    let manifest = format!(r#"{{"id":"{effect}","name":"Unseal","kind":"effect","translations":{{"de":{{"name":"Open"}}}}}}"#);
    let module = wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&module)})).unwrap();
    app.last_effect = Some((vectorcraft_plugins::effect::effect_id(effect), json!({})));
    let de = crate::i18n::Lang::from_code("de").unwrap();
    crate::i18n::set_current(de);
    let shown = menus::display_label(&app, "effect.last", "Last Effect…");
    crate::i18n::set_current(crate::i18n::Lang::EN);
    app.last_effect = None;
    app.run("plugin.remove", json!({"id": effect})).unwrap();
    assert_ne!(crate::i18n::tr(de, "Open"), "Open", "German translates the app's Open");
    assert_eq!(shown, crate::i18n::fmt(crate::i18n::tr(de, "Last Effect: {name}"), &[("name", "Open")]));
}

/// Keyboard Shortcuts shows a plug-in command's label and its plug-in's name as the plug-in gives
/// them, even where they match text the app translates: in German, a command labeled "Open" of a
/// plug-in named "Close" keeps both, as in its menu.
#[test]
fn keyboard_shortcuts_leave_a_plugin_commands_text_as_the_plugin_gives_it() {
    let mut app = app();
    let id = "test.ui.untranslated.command";
    let manifest = format!(r#"{{"id":"{id}","name":"Close","kind":"filter","commands":[{{"id":"go","label":"Open"}}]}}"#);
    let module = wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&module)})).unwrap();
    crate::i18n::set_current(crate::i18n::Lang::from_code("de").unwrap());
    let shown = crate::shortcut_editor::entry(&format!("plugin.{id}.go")).map(|e| crate::shortcut_editor::shown_name(&e));
    let expected = format!("{} › {} › Close › Open", crate::i18n::t("Object"), crate::i18n::t("Plug-ins"));
    assert_ne!((crate::i18n::t("Open"), crate::i18n::t("Close")), ("Open", "Close"), "German translates both as app text");
    crate::i18n::set_current(crate::i18n::Lang::EN);
    app.run("plugin.remove", json!({"id": id})).unwrap();
    assert_eq!(shown, Some(expected));
}

/// A filter's and an importer's menu item show their names as prompts do, a right-to-left override
/// escaped, as a filter with commands already showed its submenu.
#[test]
fn plugin_menu_items_escape_bidirectional_characters() {
    let mut app = app();
    let (filter, importer) = ("test.ui.bidi.filter", "test.ui.bidi.importer");
    // The override as a JSON escape in the manifest.
    let rlo = format!("\\u{:04X}", 0x202E);
    for manifest in [
        format!(r#"{{"id":"{filter}","name":"Flip{rlo}txt.exe","kind":"filter"}}"#),
        format!(r#"{{"id":"{importer}","name":"Read{rlo}fdp.exe","kind":"importer","extensions":["rdx"]}}"#),
    ] {
        let module = wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
        app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&module)})).unwrap();
    }
    let labels = [menu_labels(&dialogs::plugin::object_menu()), menu_labels(&dialogs::plugin::open_with_menu())].concat();
    assert!(labels.contains(&"Flip\\u{202E}txt.exe".to_string()), "{labels:?}");
    assert!(labels.contains(&"Read\\u{202E}fdp.exe…".to_string()), "{labels:?}");
    assert!(!labels.iter().any(|l| l.contains('\u{202E}')), "{labels:?}");
    for id in [filter, importer] {
        app.run("plugin.remove", json!({"id": id})).unwrap();
    }
}
