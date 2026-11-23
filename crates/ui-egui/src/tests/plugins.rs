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

/// Shows the open install approval in one window, frame after frame a quarter of a second apart,
/// until its OK takes input ([`dialogs::plugin::APPROVAL_DELAY`]), as a person sees it before
/// pressing OK.
pub(super) fn show_approval(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    for k in 0..12 {
        let input = egui::RawInput { time: Some(f64::from(k) * 0.25), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
        if app.pending_plugin_install.as_ref().is_some_and(|p| p.shown.ready()) {
            return;
        }
    }
    panic!("the approval's OK never took input");
}

pub(super) type Window = egui_kittest::Harness<'static, VectorcraftApp>;

/// The dialog layer of `app` in a window, each frame drawn at the time [`at`] gives it. Building it
/// draws frames at times of its own, so a test opens the dialog after.
pub(super) fn window(app: VectorcraftApp) -> Window {
    let mut fonts = false;
    egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 800.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !std::mem::replace(&mut fonts, true) {
                theme::install_fonts(ui.ctx());
                return;
            }
            dialogs::show(app, ui.ctx());
        },
        app,
    )
}

/// Draws a frame of `h` at `time` seconds.
pub(super) fn at(h: &mut Window, time: f64) {
    h.input_mut().time = Some(time);
    h.step();
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

/// The approval is open and plug-in `id` neither installed nor granted.
pub(super) fn waiting(app: &VectorcraftApp, id: &str) -> bool {
    app.ui.dialog.as_ref().is_some_and(|d| d.kind == dialogs::plugin::INSTALL_KIND)
        && vectorcraft_plugins::registry::get(id).is_none()
        && vectorcraft_plugins::registry::granted(id).is_none()
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
    let mut app = app();
    let id = "test.ui.permissions";
    // U+202E would show the text after it reversed, and U+2066 isolates it; a manifest's name may
    // hold them, though not control characters, which its version may.
    let name = "Tidy\u{202E}fdp.exe\u{2066}";
    let bytes = asking(id, name, r#"["doc.read","doc.write"]"#);
    let data = vectorcraft_format::base64_encode(&bytes);
    let r = app.run("ui.installPlugin", json!({"dataBase64": data, "name": "tidy.wasm"})).unwrap();
    assert_eq!(r["pending"], dialogs::plugin::INSTALL_KIND);
    let pending = app.pending_plugin_install.as_ref().unwrap();
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
    dialogs::cancel(&mut app);
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "Cancel installs nothing");
    assert!(app.pending_plugin_install.is_none(), "Cancel drops what it asked about");
    // File › Open asks too, and OK installs it with what it lists granted.
    io::open_bytes(&mut app, "tidy.wasm", &bytes, None).unwrap();
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(dialogs::plugin::INSTALL_KIND));
    show_approval(&mut app);
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert!(vectorcraft_plugins::registry::get(id).is_some());
    assert_eq!(vectorcraft_plugins::registry::granted(id).map(|g| g.hash), Some(hash));
    // The same module again: installed without asking.
    app.run("ui.installPlugin", json!({"dataBase64": data})).unwrap();
    assert!(app.ui.dialog.is_none(), "the same bytes don't ask again");
    // Other bytes with the same permissions ask for them, from both ways in; Cancel keeps the one
    // installed.
    let again = asking(id, "Tidy 2", r#"["doc.write","doc.read"]"#);
    let r = app.run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&again)})).unwrap();
    assert_eq!(r["pending"], dialogs::plugin::INSTALL_KIND, "other bytes ask again");
    assert_eq!(app.pending_plugin_install.as_ref().unwrap().message, "Install “Tidy 2”, which asks for these permissions?");
    dialogs::cancel(&mut app);
    assert_eq!(vectorcraft_plugins::registry::get(id).map(|p| p.manifest().name.clone()), Some(name.into()), "Cancel keeps the one installed");
    io::open_bytes(&mut app, "tidy2.wasm", &again, None).unwrap();
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(dialogs::plugin::INSTALL_KIND), "File › Open asks too");
    // The same dialog says it takes the installed one's place, and OK replaces it; Cancel keeps the
    // one installed and records no grant.
    let pending = app.pending_plugin_install.as_ref().unwrap();
    let again_hash = vectorcraft_plugins::permissions::sha256_hex(&again);
    assert!(pending.detail.contains(&format!("File SHA-256: {again_hash}")), "{}", pending.detail);
    assert!(
        pending.detail.ends_with("\n\nThis file is not the plug-in installed before with this ID. It gets the settings and cache kept for that ID."),
        "{}",
        pending.detail
    );
    assert_eq!(pending.params["replace"], json!(true), "OK replaces the one installed");
    dialogs::cancel(&mut app);
    assert_eq!(vectorcraft_plugins::registry::get(id).map(|p| p.manifest().name.clone()), Some(name.into()), "Cancel keeps the one installed");
    assert_ne!(vectorcraft_plugins::registry::granted(id).map(|g| g.hash), Some(again_hash), "Cancel records no grant");
    app.run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&again)})).unwrap();
    show_approval(&mut app);
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none(), "one question");
    assert_eq!(vectorcraft_plugins::registry::get(id).map(|p| p.manifest().name.clone()), Some("Tidy 2".into()));
    let new_hash = vectorcraft_plugins::permissions::sha256_hex(&again);
    assert_eq!(vectorcraft_plugins::registry::granted(id).map(|g| g.hash), Some(new_hash), "OK records the new module's hash");
    // One that asks for another permission asks again.
    let more = asking(id, "Tidy 3", r#"["doc.read","doc.write","state"]"#);
    let r = app.run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&more)})).unwrap();
    assert_eq!(r["pending"], dialogs::plugin::INSTALL_KIND);
    assert!(app.pending_plugin_install.as_ref().unwrap().detail.contains("• Keep data of its own with the document"));
    dialogs::cancel(&mut app);
    assert_eq!(vectorcraft_plugins::registry::get(id).map(|p| p.manifest().name.clone()), Some("Tidy 2".into()), "Cancel keeps the one installed");
    // OK keeps the `replace` it was asked with: `false` leaves the installed one in place.
    app.run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&more), "replace": false})).unwrap();
    show_approval(&mut app);
    let _ = dialogs::confirm(&mut app);
    assert_eq!(vectorcraft_plugins::registry::get(id).map(|p| p.manifest().name.clone()), Some("Tidy 2".into()), "replace: false is kept");
    app.run("plugin.remove", json!({"id": id})).unwrap();
}

/// The approval's OK takes no click, Enter or Space, and a confirm installs nothing, until its
/// question has been on screen for [`dialogs::plugin::APPROVAL_DELAY`]; then a click installs it.
/// Cancel works on the first frame.
#[test]
fn the_install_approval_takes_ok_only_once_its_question_was_shown_for_a_moment() {
    use egui_kittest::kittest::Queryable;
    let id = "test.ui.approval.delay";
    let module = json!({"dataBase64": vectorcraft_format::base64_encode(&asking(id, "Delay", r#"["doc.read"]"#))});
    let mut h = window(app());
    h.state_mut().run("ui.installPlugin", module.clone()).unwrap();
    assert_eq!(dialogs::confirm(h.state_mut()).unwrap(), json!({"pending": dialogs::plugin::INSTALL_KIND}), "not drawn yet");
    assert!(waiting(h.state(), id));
    at(&mut h, 10.0);
    at(&mut h, 10.0);
    assert!(!ok_enabled(&h), "disabled when it first shows");
    click_ok(&mut h, 10.1);
    assert!(waiting(h.state(), id), "a click");
    for (key, time) in [(egui::Key::Enter, 10.2), (egui::Key::Space, 10.3)] {
        h.get_by_label("OK").focus();
        at(&mut h, time);
        h.key_press(key);
        at(&mut h, time + 0.05);
        assert!(waiting(h.state(), id), "{key:?}");
    }
    at(&mut h, 10.49);
    assert!(!ok_enabled(&h), "still disabled just before the delay");
    assert_eq!(dialogs::confirm(h.state_mut()).unwrap()["pending"], dialogs::plugin::INSTALL_KIND, "a confirm");
    assert!(waiting(h.state(), id));
    at(&mut h, 10.5);
    assert!(ok_enabled(&h), "enabled once shown for the delay");
    click_ok(&mut h, 10.6);
    assert!(h.state().ui.dialog.is_none(), "{}", h.state().ui.status);
    assert!(vectorcraft_plugins::registry::get(id).is_some());
    h.state_mut().run("plugin.remove", json!({"id": id})).unwrap();

    h.state_mut().run("ui.installPlugin", module).unwrap();
    at(&mut h, 20.0);
    at(&mut h, 20.0);
    h.get_by_label("Cancel").click();
    at(&mut h, 20.0);
    assert!(h.state().ui.dialog.is_none() && h.state().pending_plugin_install.is_none(), "Cancel doesn't wait");
    assert!(vectorcraft_plugins::registry::get(id).is_none());
}

/// The wait starts over for each question: one that takes the place of the question shown just
/// before a click lands, which the click doesn't approve; the same question asked again; and the
/// question shown when the window gets the focus back.
#[test]
fn the_install_approval_waits_again_for_a_new_question_and_after_the_focus_comes_back() {
    let (a, b) = ("test.ui.approval.swap.a", "test.ui.approval.swap.b");
    let module = |id: &str| json!({"dataBase64": vectorcraft_format::base64_encode(&asking(id, "Same", r#"["doc.read"]"#))});
    let mut h = window(app());
    h.state_mut().run("ui.installPlugin", module(a)).unwrap();
    at(&mut h, 10.0);
    at(&mut h, 10.0);
    at(&mut h, 10.6);
    assert!(ok_enabled(&h));
    // Between two frames, a request closes the question and asks about another module.
    dialogs::cancel(h.state_mut());
    h.state_mut().run("ui.installPlugin", module(b)).unwrap();
    click_ok(&mut h, 10.7);
    assert!(waiting(h.state(), b) && vectorcraft_plugins::registry::get(a).is_none(), "the click approves neither");
    assert!(!ok_enabled(&h));
    at(&mut h, 11.19);
    assert!(!ok_enabled(&h));
    at(&mut h, 11.2);
    assert!(ok_enabled(&h), "the new question waits its own delay");
    // The same question again, from the same bytes.
    dialogs::cancel(h.state_mut());
    h.state_mut().run("ui.installPlugin", module(b)).unwrap();
    at(&mut h, 11.3);
    assert!(!ok_enabled(&h), "asked again, it waits again");
    at(&mut h, 11.8);
    assert!(ok_enabled(&h));
    // The window loses the focus and gets it back.
    h.input_mut().focused = false;
    at(&mut h, 11.9);
    assert!(!ok_enabled(&h), "not while the window doesn't have the focus");
    assert_eq!(dialogs::confirm(h.state_mut()).unwrap()["pending"], dialogs::plugin::INSTALL_KIND);
    h.input_mut().focused = true;
    at(&mut h, 12.0);
    at(&mut h, 12.49);
    assert!(!ok_enabled(&h), "back in front, it waits again");
    at(&mut h, 12.5);
    click_ok(&mut h, 12.6);
    assert!(h.state().ui.dialog.is_none(), "{}", h.state().ui.status);
    assert!(vectorcraft_plugins::registry::get(b).is_some() && vectorcraft_plugins::registry::get(a).is_none());
    h.state_mut().run("plugin.remove", json!({"id": b})).unwrap();
}

/// A press on OK that began before OK took input approves nothing when it is let go after; a new
/// click does.
#[test]
fn a_press_on_the_install_approval_from_before_ok_took_input_approves_nothing() {
    use egui_kittest::kittest::Queryable;
    let id = "test.ui.approval.held";
    let mut h = window(app());
    let module = json!({"dataBase64": vectorcraft_format::base64_encode(&asking(id, "Held", r#"["doc.read"]"#))});
    h.state_mut().run("ui.installPlugin", module).unwrap();
    at(&mut h, 10.0);
    at(&mut h, 10.0);
    let pos = h.get_by_label("OK").rect().center();
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    h.input_mut().events.extend([egui::Event::PointerMoved(pos), button(true)]);
    at(&mut h, 10.1);
    at(&mut h, 10.6);
    assert!(ok_enabled(&h));
    h.input_mut().events.push(button(false));
    at(&mut h, 10.7);
    assert!(waiting(h.state(), id), "the press began before OK took input");
    click_ok(&mut h, 10.8);
    assert!(h.state().ui.dialog.is_none(), "{}", h.state().ui.status);
    assert!(vectorcraft_plugins::registry::get(id).is_some());
    h.state_mut().run("plugin.remove", json!({"id": id})).unwrap();
}

/// A press of another button after OK took input doesn't let a primary press from before it
/// approve when that press is let go of.
#[test]
fn a_later_press_on_the_install_approval_keeps_an_early_press_from_approving() {
    use egui_kittest::kittest::Queryable;
    let id = "test.ui.approval.held.two";
    let mut h = window(app());
    let module = json!({"dataBase64": vectorcraft_format::base64_encode(&asking(id, "Two", r#"["doc.read"]"#))});
    h.state_mut().run("ui.installPlugin", module).unwrap();
    at(&mut h, 10.0);
    at(&mut h, 10.0);
    let pos = h.get_by_label("OK").rect().center();
    let button = |button, pressed| egui::Event::PointerButton { pos, button, pressed, modifiers: Default::default() };
    // The primary button goes down away from any widget and moves over OK.
    let away = egui::pos2(2.0, 2.0);
    h.input_mut().events.extend([
        egui::Event::PointerMoved(away),
        egui::Event::PointerButton { pos: away, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() },
    ]);
    at(&mut h, 10.1);
    h.input_mut().events.push(egui::Event::PointerMoved(pos));
    at(&mut h, 10.2);
    at(&mut h, 10.5);
    assert!(ok_enabled(&h));
    h.input_mut().events.push(button(egui::PointerButton::Secondary, true));
    at(&mut h, 10.55);
    h.input_mut().events.push(button(egui::PointerButton::Primary, false));
    at(&mut h, 10.6);
    assert!(waiting(h.state(), id), "the primary press began before OK took input");
    h.input_mut().events.push(button(egui::PointerButton::Secondary, false));
    at(&mut h, 10.65);
    click_ok(&mut h, 10.8);
    assert!(h.state().ui.dialog.is_none(), "{}", h.state().ui.status);
    h.state_mut().run("plugin.remove", json!({"id": id})).unwrap();
}

/// A press on the OK of one question that is still held when another question takes its place
/// doesn't approve the new one when it is let go of.
#[test]
fn a_press_held_from_the_question_before_approves_nothing() {
    use egui_kittest::kittest::Queryable;
    let (a, b) = ("test.ui.approval.carried.a", "test.ui.approval.carried.b");
    let module = |id: &str| json!({"dataBase64": vectorcraft_format::base64_encode(&asking(id, "Carried", r#"["doc.read"]"#))});
    let mut h = window(app());
    h.state_mut().run("ui.installPlugin", module(a)).unwrap();
    at(&mut h, 10.0);
    at(&mut h, 10.0);
    at(&mut h, 10.6);
    assert!(ok_enabled(&h));
    let pos = h.get_by_label("OK").rect().center();
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    h.input_mut().events.extend([egui::Event::PointerMoved(pos), button(true)]);
    at(&mut h, 10.65);
    // While the button is down, a request closes the question and asks about another module.
    dialogs::cancel(h.state_mut());
    h.state_mut().run("ui.installPlugin", module(b)).unwrap();
    at(&mut h, 10.7);
    at(&mut h, 11.0);
    at(&mut h, 11.2);
    assert!(ok_enabled(&h));
    h.input_mut().events.push(button(false));
    at(&mut h, 11.25);
    assert!(waiting(h.state(), b) && vectorcraft_plugins::registry::get(a).is_none(), "the press began on the question before");
    click_ok(&mut h, 11.4);
    assert!(h.state().ui.dialog.is_none(), "{}", h.state().ui.status);
    assert!(vectorcraft_plugins::registry::get(b).is_some());
    h.state_mut().run("plugin.remove", json!({"id": b})).unwrap();
}

/// Enter or Space held down while OK is disabled doesn't approve through its key repeat once OK
/// has the keyboard focus; a new press after it is let go of does.
#[test]
fn a_key_held_on_the_install_approval_from_before_ok_took_input_approves_nothing() {
    use egui_kittest::kittest::Queryable;
    for (n, key) in [egui::Key::Enter, egui::Key::Space].into_iter().enumerate() {
        let id = format!("test.ui.approval.key.{n}");
        let mut h = window(app());
        let module = json!({"dataBase64": vectorcraft_format::base64_encode(&asking(&id, "Key", r#"["doc.read"]"#))});
        h.state_mut().run("ui.installPlugin", module).unwrap();
        at(&mut h, 10.0);
        at(&mut h, 10.0);
        let event = |pressed, repeat| egui::Event::Key { key, physical_key: Some(key), pressed, repeat, modifiers: Default::default() };
        h.input_mut().events.push(event(true, false));
        at(&mut h, 10.1);
        at(&mut h, 10.6);
        assert!(ok_enabled(&h));
        h.get_by_label("OK").focus();
        at(&mut h, 10.65);
        h.input_mut().events.push(event(true, true));
        at(&mut h, 10.7);
        assert!(waiting(h.state(), &id), "{key:?}: the key went down before OK took input");
        h.input_mut().events.push(event(false, false));
        at(&mut h, 10.75);
        h.get_by_label("OK").focus();
        at(&mut h, 10.8);
        h.key_press(key);
        at(&mut h, 10.85);
        assert!(h.state().ui.dialog.is_none(), "{key:?}: {}", h.state().ui.status);
        h.state_mut().run("plugin.remove", json!({"id": id})).unwrap();
    }
}

/// The wait starts over when the window loses the focus and gets it back between two frames.
#[test]
fn the_install_approval_waits_again_when_the_focus_leaves_and_comes_back_between_frames() {
    let id = "test.ui.approval.blink";
    let mut h = window(app());
    let module = json!({"dataBase64": vectorcraft_format::base64_encode(&asking(id, "Blink", r#"["doc.read"]"#))});
    h.state_mut().run("ui.installPlugin", module).unwrap();
    at(&mut h, 10.0);
    at(&mut h, 10.0);
    at(&mut h, 10.6);
    assert!(ok_enabled(&h));
    h.input_mut().events.extend([egui::Event::WindowFocused(false), egui::Event::WindowFocused(true)]);
    at(&mut h, 10.7);
    assert!(!ok_enabled(&h), "the focus left since the frame before");
    click_ok(&mut h, 10.8);
    assert!(waiting(h.state(), id));
    at(&mut h, 11.19);
    assert!(!ok_enabled(&h));
    at(&mut h, 11.2);
    assert!(ok_enabled(&h), "back in front, it waits again");
    click_ok(&mut h, 11.3);
    assert!(h.state().ui.dialog.is_none(), "{}", h.state().ui.status);
    h.state_mut().run("plugin.remove", json!({"id": id})).unwrap();
}

/// A module with other bytes than the one installed with its id asks before taking its place, in
/// the approval dialog, with its id and file hash, also when it asks for a heavy budget.
#[test]
fn installing_a_plugin_over_another_module_asks_first() {
    let mut app = app();
    let id = "test.ui.replaces";
    let plain = |output: &str| wat::parse_str(tpl::returning(&format!(r#"{{"id":"{id}","name":"Plain","kind":"filter"}}"#), output)).unwrap();
    let module = |wasm: &[u8]| json!({"dataBase64": vectorcraft_format::base64_encode(wasm)});
    let takes_place = "This file is not the plug-in installed before with this ID. It gets the settings and cache kept for that ID.";
    app.run("ui.installPlugin", module(&plain(r#"{"objects":[]}"#))).unwrap();
    assert!(app.ui.dialog.is_none(), "nothing to take the place of");

    let other = plain(r#"{"objects":[],"error":null}"#);
    let other_hash = vectorcraft_plugins::permissions::sha256_hex(&other);
    let r = app.run("ui.installPlugin", module(&other)).unwrap();
    assert_eq!(r, json!({"pending": dialogs::plugin::INSTALL_KIND}));
    let pending = app.pending_plugin_install.as_ref().unwrap();
    assert_eq!(pending.message, "Install “Plain” in place of the plug-in with the same ID?");
    assert_eq!(pending.detail, format!("ID: {id}\nFile SHA-256: {other_hash}\n\n{takes_place}"));
    assert_eq!(pending.params["replace"], json!(true), "OK replaces the one installed");
    assert_eq!(dialogs::confirm(&mut app).unwrap()["pending"], dialogs::plugin::INSTALL_KIND, "it waits like any other question");
    show_approval(&mut app);
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(vectorcraft_plugins::registry::get(id).map(|p| p.hash().to_string()), Some(other_hash));

    let manifest = format!(r#"{{"id":"{id}","name":"Heavy","kind":"filter","budget":"heavy"}}"#);
    let heavy = wat::parse_str(tpl::spinning(&manifest, 1, r#"{"objects":[]}"#)).unwrap();
    let heavy_hash = vectorcraft_plugins::permissions::sha256_hex(&heavy);
    let r = app.run("ui.installPlugin", module(&heavy)).unwrap();
    assert_eq!(r["pending"], dialogs::plugin::INSTALL_KIND);
    let pending = app.pending_plugin_install.as_ref().unwrap();
    assert!(pending.detail.ends_with(&format!("\n\nID: {id}\nFile SHA-256: {heavy_hash}\n\n{takes_place}")), "{}", pending.detail);
    assert_eq!(pending.params["replace"], json!(true), "OK replaces the one installed");
    show_approval(&mut app);
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(vectorcraft_plugins::registry::get(id).map(|p| p.hash().to_string()), Some(heavy_hash));
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
    assert!(crate::palette::items().iter().any(|(name, id, _)| *id == low(bee) && name.ends_with("Low Area (Bee)")), "in the command palette");

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
    fn find<'a>(items: &'a [menus::Item], label: &str) -> Option<&'a [menus::Item]> {
        items.iter().find_map(|it| match it {
            menus::Item::Sub(l, children) if *l == label => Some(children.as_slice()),
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
                    menus::Item::Sub(l, children) if l == segment => Some(children.as_slice()),
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
                menus::Item::Cmd(l, id, p) if !menus::lists_plugin(id, p) => *l,
                menus::Item::Sub(l, children) if !menus::holds_plugin_commands(children) => *l,
                menus::Item::Todo(l, _) => *l,
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

/// A filter at API level 2 named Ctx whose commands are `commands` (a JSON list): each adds a
/// triangle when it is given an object named Target, and leaves its input as it is otherwise.
fn context_plugin(id: &str, commands: &str) -> String {
    let manifest = format!(r#"{{"id":"{id}","name":"Ctx","api":2,"version":"1","kind":"filter","commands":{commands}}}"#);
    let triangle = r#"{"objects":[{"path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[5,0]},{"p":[5,5]}]}]}}]}"#;
    vectorcraft_format::base64_encode(&wat::parse_str(tpl::when_given(&manifest, tpl::Given::Input, r#""name":"Target""#, triangle)).unwrap())
}

/// The items of the canvas context menu's Plug-ins submenu: (plug-in, label, command, params).
fn canvas_plugin_items(app: &VectorcraftApp) -> Vec<(String, String, String, Value)> {
    let items = menus::context_items(app);
    let Some(menus::Item::Sub(_, plugins)) = items.iter().find(|it| matches!(it, menus::Item::Sub("Plug-ins", _))) else { return Vec::new() };
    let mut out = Vec::new();
    for p in plugins {
        let menus::Item::Sub(name, commands) = p else { panic!("a plug-in's group is a submenu: {p:?}") };
        for c in commands {
            let menus::Item::Cmd(label, id, params) = c else { panic!("a plug-in group holds commands only: {c:?}") };
            out.push((name.to_string(), label.to_string(), id.to_string(), params.clone()));
        }
    }
    out
}

/// A command whose manifest's `menu.context` names the canvas or the Layers panel is listed in that
/// context menu, in a Plug-ins submenu holding a submenu named after the plug-in; a command without
/// it is not. An item whose `when` fails is greyed out. Chosen, it runs its command on the
/// selection (canvas) or on the clicked rows (Layers panel, whatever the selection) as one undo
/// step named after the command; one with parameters opens its dialog first, and OK runs it on
/// those objects. Nothing is listed while nothing is selected, and nothing once the plug-in is
/// removed.
#[test]
fn plugin_commands_are_listed_in_the_canvas_and_layers_context_menus() {
    use dialogs::plugin::{ContextCommand, context_commands};
    use vectorcraft_plugins::ContextMenu;
    let mut app = app();
    let id = "test.ui.context";
    let command = |c: &str| vectorcraft_engine::cmd::plugin::command_id(id, c);
    let commands = r#"[{"id":"tidy","label":"Tidy","menu":{"context":[{"menu":"canvas"},{"menu":"layers","when":"oneObject"}]}},
        {"id":"text","label":"Text Only","menu":{"context":[{"menu":"canvas","when":"typeObject"},{"menu":"layers","when":"layerRow"}]}},
        {"id":"size","label":"Size","menu":{"context":[{"menu":"canvas"},{"menu":"layers"}]},"params":{"by":{"type":"number","min":0,"max":10,"default":2}}},
        {"id":"plain","label":"Plain"}]"#;
    app.run("plugin.install", json!({"dataBase64": context_plugin(id, commands)})).unwrap();
    let target = red_rect(&mut app);
    app.run("layer.setProps", json!({"ids": [target.0], "name": "Target"})).unwrap();
    let other = red_rect(&mut app);
    let layer = app.session.doc().unwrap().doc.layers[0].id;

    app.run("select.none", json!({})).unwrap();
    assert!(canvas_plugin_items(&app).is_empty(), "nothing selected: no Plug-ins submenu");

    app.run("select.set", json!({"ids": [target.0]})).unwrap();
    let ids = json!({"ids": [target.0]});
    let listed = canvas_plugin_items(&app);
    let expected = [("Ctx", "Tidy", command("tidy")), ("Ctx", "Text Only", command("text")), ("Ctx", "Size…", command("size"))];
    assert_eq!(listed.iter().map(|(p, l, c, _)| (p.as_str(), l.as_str(), c.clone())).collect::<Vec<_>>(), expected);
    assert!(listed.iter().all(|(.., p)| *p == ids), "{listed:?}");
    let shown = |app: &VectorcraftApp, c: &str| menus::shown_state(app, c, &ids).map(|(enabled, _)| enabled);
    assert_eq!((shown(&app, &command("tidy")), shown(&app, &command("text"))), (Some(true), Some(false)), "no type object is selected");

    let doc = app.session.doc().unwrap().doc.clone();
    let rows = |ids: &[u64]| context_commands(ContextMenu::Layers, &doc, ids);
    let item = |label: &'static str, c: &str, enabled| ContextCommand { label, command: vectorcraft_plugins::registry::intern(&command(c)), enabled };
    assert_eq!(rows(&[target.0]), vec![("Ctx", vec![item("Tidy", "tidy", true), item("Text Only", "text", false), item("Size…", "size", true)])]);
    assert_eq!(rows(&[layer.0])[0].1[..2], [item("Tidy", "tidy", false), item("Text Only", "text", true)], "a layer row");
    assert_eq!(rows(&[target.0, other.0])[0].1[0], item("Tidy", "tidy", false), "two rows are not one object");

    // The canvas item runs on the selection as one undo step.
    // The plug-in's output, a triangle in place of what it was given, replaces Target only when
    // Target is what it was given.
    let undo = app.session.doc().unwrap().history.undo.len();
    menus::invoke(&mut app, &command("tidy"), ids.clone());
    let st = app.session.doc().unwrap();
    assert_eq!((st.history.undo.len(), st.history.undo.last().unwrap().label.as_str()), (undo + 1, "Tidy"));
    assert!(st.doc.node(target).is_none() && st.doc.node(other).is_some(), "the plug-in was given Target alone");
    app.run("edit.undo", json!({})).unwrap();

    // Clicking the canvas item of a command with parameters opens the plug-in's own dialog for the
    // selection, not the generic parameter dialog.
    let (_, label, c, p) = listed.iter().find(|(_, l, ..)| l == "Size…").unwrap();
    let (cmd, p) = menus::click_target(label, c, p);
    menus::invoke(&mut app, &cmd, p);
    let d = app.ui.dialog.take().expect("Size opens its dialog from the canvas");
    assert_eq!((d.kind.as_str(), d.str("__command"), d.fields.get("__ids")), (dialogs::plugin::KIND, command("size"), Some(&json!([target.0]))));

    // A row's item runs on that row's object, not on the selection; one with parameters asks first.
    app.run("select.set", json!({"ids": [other.0]})).unwrap();
    let undo = app.session.doc().unwrap().history.undo.len();
    menus::invoke(&mut app, &command("size"), ids.clone());
    let d = app.ui.dialog.as_ref().expect("Size opens its dialog");
    assert_eq!((d.str("__command"), d.fields.get("__ids")), (command("size"), Some(&json!([target.0]))));
    frame(&mut app);
    dialogs::confirm(&mut app).unwrap();
    let st = app.session.doc().unwrap();
    assert_eq!((st.history.undo.len(), st.history.undo.last().unwrap().label.as_str()), (undo + 1, "Size"));
    assert!(st.doc.node(target).is_none() && st.doc.node(other).is_some(), "the plug-in was given the row's object, not the selection");

    let list = app.run("plugin.list", json!({})).unwrap();
    let entry = list["plugins"].as_array().unwrap().iter().find(|p| p["id"] == id).unwrap().clone();
    assert_eq!(entry["commands"][0]["menu"], json!({"context": [{"menu": "canvas"}, {"menu": "layers", "when": "oneObject"}]}));
    assert!(entry["commands"][3].get("menu").is_none(), "Plain lists no menu");

    app.run("plugin.remove", json!({"id": id})).unwrap();
    assert!(canvas_plugin_items(&app).is_empty(), "removed: nothing listed");
    assert!(rows(&[other.0]).is_empty());
}

/// A command dialog opened for objects of one document changes no document once another document
/// is the active one, even one holding an object with the same id: drawn, the dialog closes and
/// leaves that document's edit in progress, and OK closes it. On its own document, OK runs the
/// command on those objects.
#[test]
fn a_plugin_command_dialog_stays_with_its_document() {
    let mut app = app();
    let id = "test.ui.context.doc";
    let size = vectorcraft_engine::cmd::plugin::command_id(id, "size");
    let commands = r#"[{"id":"size","label":"Size","params":{"by":{"type":"number","min":0,"max":10,"default":2}}}]"#;
    app.run("plugin.install", json!({"dataBase64": context_plugin(id, commands)})).unwrap();
    let target = red_rect(&mut app);
    app.run("layer.setProps", json!({"ids": [target.0], "name": "Target"})).unwrap();
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let twin = red_rect(&mut app);
    app.run("layer.setProps", json!({"ids": [twin.0], "name": "Target"})).unwrap();
    assert_eq!(twin, target, "the second document's object has the first one's id");
    let ids = json!({"ids": [target.0]});
    let undos = |app: &VectorcraftApp| app.session.documents().iter().map(|st| st.history.undo.len()).collect::<Vec<_>>();
    let kept = |app: &VectorcraftApp| app.session.documents().iter().map(|st| st.doc.node(target).is_some()).collect::<Vec<_>>();

    // Drawn while the other document is active, the dialog closes.
    app.run("document.activate", json!({"index": 0})).unwrap();
    menus::invoke(&mut app, &size, ids.clone());
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.str("__command")), Some(size.clone()));
    let before = undos(&app);
    frame(&mut app);
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.begin_interaction("Edit").unwrap();
    frame(&mut app);
    assert!(app.ui.dialog.is_none(), "the dialog closes on another document");
    assert!(app.session.in_interaction(), "closing it leaves the other document's edit in progress");
    app.session.cancel_interaction().unwrap();
    assert_eq!((undos(&app), kept(&app)), (before.clone(), vec![true, true]));

    // OK while the other document is active closes the dialog and changes neither document.
    app.run("document.activate", json!({"index": 0})).unwrap();
    menus::invoke(&mut app, &size, ids.clone());
    frame(&mut app);
    app.run("document.activate", json!({"index": 1})).unwrap();
    assert_eq!(dialogs::confirm(&mut app).unwrap(), json!({}));
    assert!(app.ui.dialog.is_none());
    assert_eq!((undos(&app), kept(&app)), (before.clone(), vec![true, true]));

    // On its own document, OK runs the command on the objects it opened for.
    app.run("document.activate", json!({"index": 0})).unwrap();
    menus::invoke(&mut app, &size, ids);
    frame(&mut app);
    dialogs::confirm(&mut app).unwrap();
    assert_eq!((undos(&app), kept(&app)), (vec![before[0] + 1, before[1]], vec![false, true]));
    app.run("plugin.remove", json!({"id": id})).unwrap();
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

/// The `grant` and `allowHeavy` that `ui.installPlugin` is given approve nothing: it asks all the
/// same, and what OK would approve comes from the manifest. A module with nothing to approve
/// installs without either.
#[test]
fn ui_install_plugin_takes_no_grant_or_heavy_budget_from_its_params() {
    let mut app = app();
    let id = "test.ui.approval.params";
    let manifest = format!(r#"{{"id":"{id}","name":"Asks","kind":"filter","permissions":["doc.read"]}}"#);
    let asks = wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    let given = |wasm: &[u8]| json!({"dataBase64": vectorcraft_format::base64_encode(wasm), "grant": ["doc.read", "doc.write"], "allowHeavy": true});
    let r = app.run("ui.installPlugin", given(&asks)).unwrap();
    assert_eq!(r["pending"], dialogs::plugin::INSTALL_KIND);
    assert!(vectorcraft_plugins::registry::get(id).is_none() && vectorcraft_plugins::registry::granted(id).is_none());
    let pending = app.pending_plugin_install.as_ref().unwrap();
    assert_eq!(pending.approval.permissions, ["doc.read"], "the permissions the manifest lists");
    assert_eq!(pending.params["grant"], json!(["doc.read"]));
    assert!(!pending.approval.heavy && pending.params.get("allowHeavy").is_none(), "no heavy budget was asked about");
    dialogs::cancel(&mut app);
    assert!(vectorcraft_plugins::registry::get(id).is_none());

    let heavy_id = "test.ui.approval.params.heavy";
    let manifest = format!(r#"{{"id":"{heavy_id}","name":"Heavy","kind":"filter","budget":"heavy"}}"#);
    let heavy = wat::parse_str(tpl::spinning(&manifest, 1, r#"{"objects":[]}"#)).unwrap();
    let r = app.run("ui.installPlugin", given(&heavy)).unwrap();
    assert_eq!(r["pending"], dialogs::plugin::INSTALL_KIND, "allowHeavy skips no question");
    assert!(vectorcraft_plugins::registry::get(heavy_id).is_none());
    dialogs::cancel(&mut app);

    let plain_id = "test.ui.approval.params.plain";
    let manifest = format!(r#"{{"id":"{plain_id}","name":"Plain","kind":"filter"}}"#);
    let plain = wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    let r = app.run("ui.installPlugin", given(&plain)).unwrap();
    assert!(r.get("pending").is_none() && app.ui.dialog.is_none(), "{r}");
    assert!(vectorcraft_plugins::registry::get(plain_id).is_some());
    assert!(vectorcraft_plugins::registry::granted(plain_id).is_none(), "the grant it was given isn't recorded");
    app.run("plugin.remove", json!({"id": plain_id})).unwrap();
}

/// An action's step can't grant what only a person approves: a played `plugin.install` with `grant`
/// is refused at Accelerated and at Step by Step, and nothing is installed.
#[test]
fn an_action_step_cannot_grant_a_plugin_its_permissions() {
    use crate::panels::actions::{Action, ActionSet, PlaybackSpeed, play};
    let mut app = app();
    let id = "test.ui.approval.action";
    let manifest = format!(r#"{{"id":"{id}","name":"Asks","kind":"filter","permissions":["doc.read"]}}"#);
    let wasm = wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    let step = json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": ["doc.read"]});
    app.ui.action_sets = vec![ActionSet::new("Test", vec![Action::new("Install", vec![("plugin.install".into(), step)])])];
    for speed in [PlaybackSpeed::Accelerated, PlaybackSpeed::Step] {
        app.ui.action_playback.speed = speed;
        let e = play(&mut app, 0, 0).unwrap_err();
        assert!(e.contains("take effect only in a session started with --allow-plugin-grants"), "{e}");
        assert!(vectorcraft_plugins::registry::get(id).is_none() && vectorcraft_plugins::registry::granted(id).is_none());
    }
}

/// A plug-in of API level 2 that lists `progress` names it in plain words when it asks.
#[test]
fn installing_a_plugin_that_reports_progress_says_so() {
    let mut app = app();
    let id = "test.ui.permissions.progress";
    let manifest = format!(r#"{{"id":"{id}","name":"Count","kind":"filter","api":2,"permissions":["doc.read","progress"]}}"#);
    let bytes = wat::parse_str(tpl::reporting(&manifest, "(call $vc_progress (i64.const 1) (i64.const 2))", r#"{"objects":[]}"#)).unwrap();
    let r = app.run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap();
    assert_eq!(r["pending"], dialogs::plugin::INSTALL_KIND);
    let detail = app.pending_plugin_install.as_ref().unwrap().detail.clone();
    assert!(detail.ends_with("• Read the objects it is given\n• Show its progress, see Cancel and write log lines"), "{detail}");
    show_approval(&mut app);
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(vectorcraft_plugins::registry::granted(id).map(|g| g.permissions.len()), Some(2));
    app.run("plugin.remove", json!({"id": id})).unwrap();
}
