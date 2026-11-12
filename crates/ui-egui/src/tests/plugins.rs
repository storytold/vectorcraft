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

type Window = egui_kittest::Harness<'static, VectorcraftApp>;

/// The dialog layer of `app` in a window, each frame drawn at the time [`at`] gives it. Building it
/// draws frames at times of its own, so a test opens the dialog after.
fn window(app: VectorcraftApp) -> Window {
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
fn at(h: &mut Window, time: f64) {
    h.input_mut().time = Some(time);
    h.step();
}

/// Whether the approval's OK is enabled, as a screen reader reads it.
fn ok_enabled(h: &Window) -> bool {
    use egui_kittest::kittest::{NodeT, Queryable};
    !h.get_by_label("OK").accesskit_node().is_disabled()
}

/// Clicks the approval's OK as a person does, on a frame at `time` seconds.
fn click_ok(h: &mut Window, time: f64) {
    use egui_kittest::kittest::Queryable;
    h.get_by_label("OK").click();
    at(h, time);
}

/// The approval is open and plug-in `id` neither installed nor granted.
fn waiting(app: &VectorcraftApp, id: &str) -> bool {
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
fn asking(id: &str, name: &str, permissions: &str) -> Vec<u8> {
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

/// A module with other bytes than the one installed with its id asks before taking its place: in
/// the ordinary confirm when it needs nothing that only a person can approve, and in the approval
/// dialog, with its id and file hash, when it also asks for a heavy budget.
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
    assert_eq!(r["pending"], dialogs::confirm::KIND);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.str("message"), "Install “Plain” in place of the plug-in with the same ID?");
    assert_eq!(d.str("detail"), format!("ID: {id}\nFile SHA-256: {other_hash}\n\n{takes_place}"));
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
