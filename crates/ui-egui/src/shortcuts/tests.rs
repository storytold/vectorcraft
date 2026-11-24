//! Key dispatch through the whole app frame: chord → command → effect.

use egui::{Key, Modifiers};
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use crate::VectorcraftApp;
use crate::tests_synthetic::key_frame;

/// What egui reports for the command key: Cmd on a Mac, Ctrl elsewhere. The keymap reads a
/// Mac's Ctrl as a key of its own, so Ctrl+L there is not Cmd+L.
fn command_key() -> Modifiers {
    if crate::keymap::platform().is_mac() { Modifiers::MAC_CMD | Modifiers::COMMAND } else { Modifiers::CTRL | Modifiers::COMMAND }
}

/// Cmd as egui reports it on this platform.
fn cmd() -> Modifiers {
    command_key()
}

/// A press of `key` with `m` held.
fn press(key: Key, m: Modifiers) -> Vec<egui::Event> {
    vec![egui::Event::ModifiersChanged(m), egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: m }]
}

/// A press and release of `key` with `m` held.
fn tap(key: Key, m: Modifiers) -> Vec<egui::Event> {
    let mut v = press(key, m);
    v.push(egui::Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers: m });
    v.push(egui::Event::ModifiersChanged(Modifiers::NONE));
    v
}

/// A press of `key` that also types `text`.
fn typed(key: Key, m: Modifiers, text: &str) -> Vec<egui::Event> {
    let mut v = press(key, m);
    v.push(egui::Event::Text(text.into()));
    v
}

/// F pressed without a modifier.
fn press_f() -> Vec<egui::Event> {
    typed(Key::F, Modifiers::NONE, "f")
}

/// A context that has run its first pass, which builds the board's keymap.
fn ready(app: &mut VectorcraftApp) -> egui::Context {
    let ctx = crate::testing::ui_context();
    key_frame(app, &ctx, vec![]);
    ctx
}

/// A context whose last frame had the Type tool's editor focused, as a window that has drawn a while has: an Alt
/// chord's text is held back for a binding around the focus of the last pass, and the editor takes the focus in its
/// first.
fn typing_ready(app: &mut VectorcraftApp) -> egui::Context {
    let ctx = ready(app);
    key_frame(app, &ctx, vec![]);
    ctx
}

/// One whole app frame delivering `events`, in a context of its own.
fn frame(app: &mut VectorcraftApp, events: Vec<egui::Event>) {
    let ctx = ready(app);
    key_frame(app, &ctx, events);
}

/// Gives the current document's canvas the keyboard, as a press on it does, and runs the frame that applies it: the
/// keys a tool claims reach the canvas that has the keyboard, and these tests drive the gesture past the canvas.
fn focus_canvas(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let win = crate::doc_windows::current(app).expect("a document window");
    craft::ui::runtime::focus::request(ctx, craft::ui::runtime::focus::FocusTarget::Id(crate::canvas::canvas_id(win)));
    key_frame(app, ctx, vec![]);
}

/// `n` empty frames, `0.05` s each.
fn idle(app: &mut VectorcraftApp, ctx: &egui::Context, n: usize) {
    for _ in 0..n {
        key_frame(app, ctx, vec![]);
    }
}

/// An app with a 200 × 200 document whose window has drawn its first frame, which builds the keymap the shortcut
/// commands edit.
fn new_app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    ready(&mut app);
    app
}

fn app_with_a_selected_rectangle() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 30})).unwrap();
    ready(&mut app);
    app
}

/// The shortcut the menu item running `id` with `params` shows, as of the last frame's keymap.
fn item_shortcut(app: &VectorcraftApp, id: &str, params: Value) -> Option<String> {
    crate::menus::item_shortcut(app, id, &params).map(|c| c.to_string())
}

/// Gives `id` the chord `shortcut`, taking it from any command that holds it.
fn bind(app: &mut VectorcraftApp, id: &str, shortcut: &str) {
    app.run("shortcuts.set", json!({"id": id, "shortcut": shortcut, "force": true})).unwrap();
}

/// An app with a selected rectangle where `id` has the shortcut `shortcut` and F runs nothing else.
fn app_with_shortcut(id: &str, shortcut: &str) -> VectorcraftApp {
    let mut app = app_with_a_selected_rectangle();
    bind(&mut app, "view.screenMode", "");
    bind(&mut app, id, shortcut);
    app.ui.status.clear();
    app
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

fn zoom(app: &mut VectorcraftApp) -> f64 {
    app.view_mut().unwrap().zoom
}

/// The key of the window's top dialog, else the command of what opened in its place: the framework's Find and
/// Replace dialog, which has no key, or the Smooth or Simplify slider, which the app shows over the canvas.
fn dialog_kind(app: &VectorcraftApp) -> Option<String> {
    let find = || app.dialogs().is_some_and(|d| d.with(|h| h.is_open::<craft::text::ui::find::FindReplaceDialog>()));
    let strip = || {
        [(crate::dialogs::smooth_slider::showing(app), "object.path.smooth"), (crate::dialogs::simplify_slider::showing(app), "object.path.simplify")]
            .into_iter()
            .find_map(|(on, id)| on.then(|| id.to_string()))
    };
    match crate::tests::picks::open_dialogs(app).pop() {
        Some(d) if d.key.is_some() => d.key,
        Some(_) if find() => Some(crate::dialogs::find_replace::COMMAND.to_string()),
        Some(_) => None,
        None => strip(),
    }
}

/// The effect of the open Effect dialog, when the window's top dialog is it (its key is `effect`;
/// the effect is the app's, not a field).
fn open_effect(app: &VectorcraftApp) -> Option<String> {
    (dialog_kind(app).as_deref() == Some("effect")).then(|| app.effect_dialogs.effect.as_ref().map(|o| o.cx.id.clone()))?
}

/// Whether the window's top dialog is plug-in `plugin`'s dialog: its key is `plugin`, its instance the plug-in.
fn plugin_open(app: &VectorcraftApp, plugin: &str) -> bool {
    let want = craft::ui::dialog::dialog_id((crate::dialogs::plugin::KIND, plugin));
    crate::tests::picks::open_dialogs(app).pop().is_some_and(|d| d.id == want)
}

/// A field of the window's top dialog.
fn dialog_field(app: &VectorcraftApp, key: &str) -> Option<Value> {
    crate::tests::picks::open_dialogs(app).pop().and_then(|d| d.fields?.get(key).cloned())
}

/// OK (`ok`) or Cancel in the window's top dialog, and the app's pump over the answer.
pub(super) fn close_top_dialog(app: &mut VectorcraftApp, ok: bool) {
    let ctx = app.ui_ctx.clone().expect("a window context");
    // A dialog opened after the window's pass shows in its next one, and only a shown dialog takes an answer.
    key_frame(app, &ctx, vec![]);
    let id = crate::tests::picks::open_dialogs(app).pop().expect("a dialog is open").id;
    if ok {
        // A dialog that holds its OK closes once the pump accepted the answer.
        crate::tests::picks::answer_dialog(app, id, &[], true);
    } else {
        app.dialogs().expect("the window's dialog host").with(|h| h.cancel_by_id(&ctx, id));
        crate::dialogs::host::pump(app, &ctx);
    }
    assert!(crate::tests::picks::open_dialogs(app).iter().all(|d| d.id != id), "the dialog stayed open");
    // The window's next pass draws it gone: until then its layer, last drawn, still holds the keys.
    key_frame(app, &ctx, vec![]);
}

/// Sets `field` of the window's top dialog as a person editing it would.
fn set_top_field(app: &mut VectorcraftApp, field: &str, value: Value) {
    let ctx = app.ui_ctx.clone().expect("a window context");
    let id = crate::tests::picks::open_dialogs(app).pop().expect("a dialog is open").id;
    let host = app.dialogs().expect("the window's dialog host");
    host.with(|h| h.set_json_field(&ctx, id, field, value)).unwrap_or_else(|e| panic!("{field}: {e}"));
}

/// The ids of the first layer's objects.
fn ids(app: &VectorcraftApp) -> Vec<u64> {
    app.session.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id.0).collect()
}

/// The text of the type object `id`, and the document's layer count.
fn text_and_layers(app: &mut VectorcraftApp, id: u64) -> (String, usize) {
    let r = app.session.execute("text.getRange", &json!({"id": id})).unwrap();
    let text = r["runs"].as_array().unwrap().iter().filter_map(|run| run["text"].as_str()).collect();
    (text, app.session.active().unwrap().doc.layers.len())
}

/// The `field` of the style of the last run of the type object `id`.
fn last_style(app: &mut VectorcraftApp, id: u64, field: &str) -> Value {
    let r = app.session.execute("text.getRange", &json!({"id": id})).unwrap();
    r["runs"].as_array().unwrap().last().unwrap()["style"][field].clone()
}

/// A document with the type object "Hi", edited with the Type tool and the caret after "Hi".
fn editing_hi() -> (VectorcraftApp, u64) {
    let mut app = new_app();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    edit(&mut app, 2, 2);
    (app, id)
}

/// Starts editing the selected type object with the Type tool and selects `start..end`.
fn edit(app: &mut VectorcraftApp, start: u64, end: u64) {
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": start, "end": end}));
    assert!(app.session.tool_wants_text());
}

// ---- T1: panels ----------------------------------------------------------------------------

/// Each default panel chord toggles its panel (`window.panel {panel, toggle}`).
#[test]
fn named_panel_default_chords_toggle_their_panels() {
    let (shift, c) = (Modifiers::SHIFT, cmd());
    for (panel, key, mods) in [
        ("brushes", Key::F5, Modifiers::NONE),
        ("color", Key::F6, Modifiers::NONE),
        ("layers", Key::F7, Modifiers::NONE),
        ("graphicStyles", Key::F5, shift),
        ("appearance", Key::F6, shift),
        ("align", Key::F7, shift),
        ("transform", Key::F8, shift),
        ("info", Key::F8, c),
        ("gradient", Key::F9, c),
        ("stroke", Key::F10, c),
        ("attributes", Key::F11, c),
    ] {
        let mut app = new_app();
        let ctx = ready(&mut app);
        let before = crate::panel_docking::showing(&app.ui, panel);
        key_frame(&mut app, &ctx, tap(key, mods));
        let after = crate::panel_docking::showing(&app.ui, panel);
        assert_ne!(before, after, "{panel}: {key:?} with {mods:?} toggles it");
    }
}

/// A shortcut on a panel without a function key of its own runs `window.panel` with the params it has: no toggle is
/// added, as production checked.
#[test]
fn other_panel_shortcuts_keep_plain_panel_params() {
    for panel in ["character", "swatches"] {
        let params = super::shortcut_params(&craft::keymap::CommandRef::with_params("window.panel", json!({"panel": panel})));
        assert!(params.get("toggle").is_none(), "{panel} gained toggle");
        assert!(params.get("toggleGroup").is_none(), "{panel} gained toggleGroup");
    }
}

#[test]
fn rebound_layers_chord_toggles_and_plain_panel_command_reveals() {
    let mut app = new_app();
    bind(&mut app, "panel:layers", "Cmd+Alt+Shift+F7");
    let ctx = ready(&mut app);
    app.run("window.panel", json!({"panel": "layers"})).unwrap();
    let checked = |app: &VectorcraftApp| crate::menus::checked(app, "window.panel", &json!({"panel": "layers"}));
    key_frame(&mut app, &ctx, tap(Key::F7, Modifiers::NONE));
    assert_eq!(checked(&app), Some(true), "the old F7 is free");
    let chord = cmd() | Modifiers::ALT | Modifiers::SHIFT;
    key_frame(&mut app, &ctx, tap(Key::F7, chord));
    assert_eq!(checked(&app), Some(false));
    key_frame(&mut app, &ctx, tap(Key::F7, chord));
    assert_eq!(checked(&app), Some(true));
    app.run("window.panel", json!({"panel": "layers"})).unwrap();
    assert_eq!(checked(&app), Some(true), "a plain command still reveals");
}

// ---- T2: tools -----------------------------------------------------------------------------

/// A tool key chooses its tool.
#[test]
fn tool_keys_choose_their_tools() {
    for (key, mods, text, tool) in [
        (Key::P, Modifiers::NONE, "p", "pen"),
        (Key::V, Modifiers::NONE, "v", "selection"),
        (Key::A, Modifiers::NONE, "a", "directSelection"),
        (Key::Equals, Modifiers::SHIFT, "+", "addAnchor"),
        (Key::Minus, Modifiers::NONE, "-", "deleteAnchor"),
    ] {
        let mut app = new_app();
        app.select_tool(if tool == "selection" { "rectangle" } else { "selection" });
        frame(&mut app, typed(key, mods, text));
        assert_eq!(app.session.tool_id(), tool, "{key:?} with {mods:?}");
    }
}

/// A tool bound to a key that types nothing, a panel and a command, and a held Cmd+= zooms on.
#[test]
fn shortcuts_run_tools_panels_and_commands() {
    let mut app = new_app();
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap();
    app.session.execute("select.set", &json!({"ids": []})).unwrap();
    bind(&mut app, "tool:pen", "F2");
    let ctx = ready(&mut app);
    key_frame(&mut app, &ctx, tap(Key::F2, Modifiers::NONE));
    assert_eq!(app.session.tool_id(), "pen", "a tool on a key that types nothing");
    app.select_tool("selection");
    key_frame(&mut app, &ctx, tap(Key::F7, Modifiers::SHIFT));
    assert_eq!(app.ui.open_panel.as_deref(), Some("align"));
    key_frame(&mut app, &ctx, tap(Key::A, cmd()));
    assert_eq!(app.session.active().unwrap().selection.objects.len(), 2, "Select All");
    key_frame(&mut app, &ctx, tap(Key::Equals, cmd()));
    let once = zoom(&mut app);
    let mut repeat = press(Key::Equals, cmd());
    repeat.push(egui::Event::Key { key: Key::Equals, physical_key: Some(Key::Equals), pressed: true, repeat: true, modifiers: cmd() });
    key_frame(&mut app, &ctx, repeat);
    assert!(zoom(&mut app) > once, "a key repeat zooms again");
}

/// A tool's own chord ends its session and keeps the tool chosen.
#[test]
fn the_blend_tool_shortcut_finishes_the_blend() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = new_app();
    for x in [0, 200] {
        app.session.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 40, "height": 40})).unwrap();
    }
    app.select_tool("blend");
    let click = |app: &mut VectorcraftApp, x: f64| {
        let v = app.view_info();
        app.session.pointer(&PointerEvent::new(PointerKind::Down, x, 20.0), v).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Up, x, 20.0), v).unwrap();
    };
    click(&mut app, 20.0);
    assert!(app.session.tool_busy(), "the first rectangle is picked");
    let layers = app.session.active().unwrap().doc.layers.clone();
    frame(&mut app, typed(Key::P, Modifiers::NONE, "p"));
    assert_eq!(app.session.tool_id(), "blend", "the Pen's shortcut does nothing while the pick is held");
    assert!(app.session.tool_busy());
    frame(&mut app, typed(Key::W, Modifiers::NONE, "w"));
    assert_eq!(app.session.tool_id(), "blend");
    assert!(!app.session.tool_busy(), "W drops the pick");
    click(&mut app, 220.0);
    assert!(app.session.tool_busy(), "the second rectangle is a new first pick");
    assert_eq!(app.session.active().unwrap().doc.layers, layers, "no blend was made");
}

/// P pressed between clicks ends the path being drawn; the next click starts a new path.
#[test]
fn the_pen_shortcut_ends_the_path_being_drawn() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
    ready(&mut app);
    app.select_tool("pen");
    let click = |app: &mut VectorcraftApp, x: f64| {
        let v = app.view_info();
        app.session.pointer(&PointerEvent::new(PointerKind::Down, x, 100.0), v).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Up, x, 100.0), v).unwrap();
    };
    let anchors = |app: &VectorcraftApp, id| {
        let path = app.session.active().unwrap().doc.node(id).and_then(|n| n.path_data());
        path.map(|pd| pd.subpaths.iter().map(|sp| sp.anchors.len()).collect::<Vec<_>>())
    };
    click(&mut app, 100.0);
    click(&mut app, 200.0);
    let first = app.session.active().unwrap().selection.objects[0];
    frame(&mut app, typed(Key::P, Modifiers::NONE, "p"));
    assert_eq!(app.session.tool_id(), "pen");
    click(&mut app, 300.0);
    assert_eq!(anchors(&app, first), Some(vec![2]), "the path keeps its two anchors");
    let second = app.session.active().unwrap().selection.objects[0];
    assert_ne!(second, first, "the click starts a new path");
    assert_eq!(anchors(&app, second), Some(vec![1]));
}

/// A Blend chord that arrives only as text (no key press) also finishes the blend.
#[test]
fn a_text_only_blend_shortcut_finishes_the_blend() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = new_app();
    bind(&mut app, "tool:curvature", "");
    bind(&mut app, "tool:blend", "Shift+~");
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap();
    app.select_tool("blend");
    let v = app.view_info();
    app.session.pointer(&PointerEvent::new(PointerKind::Down, 20.0, 20.0), v).unwrap();
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 20.0, 20.0), v).unwrap();
    assert!(app.session.tool_busy(), "the rectangle is picked");
    frame(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::SHIFT), egui::Event::Text("W".into())]);
    assert!(app.session.tool_busy(), "Shift+W is another tool's shortcut and keeps the pick");
    frame(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::SHIFT), egui::Event::Text("~".into())]);
    assert_eq!(app.session.tool_id(), "blend");
    assert!(!app.session.tool_busy(), "Shift+~ drops the pick");
}

/// Ctrl+` chooses the last selection tool on Windows and Linux; macOS has no default.
#[test]
fn ctrl_backtick_chooses_the_last_selection_tool() {
    let mut app = new_app();
    let mac = crate::keymap::platform().is_mac();
    app.select_tool("directSelection");
    app.select_tool("pen");
    frame(&mut app, tap(Key::Backtick, cmd()));
    assert_eq!(app.session.tool_id(), if mac { "pen" } else { "directSelection" });
}

// ---- T3: commands --------------------------------------------------------------------------

/// A chord, the app it runs in, and what must hold afterwards.
struct Row {
    name: &'static str,
    setup: fn(&mut VectorcraftApp),
    events: Vec<egui::Event>,
    check: fn(&mut VectorcraftApp) -> bool,
}

/// Two rectangles at x = 10 and x = 50, the first selected.
fn two_rectangles(app: &mut VectorcraftApp) {
    app.session.execute("shape.rectangle", &json!({"x": 50, "y": 10, "width": 20, "height": 20})).unwrap();
    let first = ids(app)[0];
    app.session.execute("select.set", &json!({"ids": [first]})).unwrap();
}

#[test]
fn chords_run_their_commands() {
    let all = cmd() | Modifiers::ALT | Modifiers::SHIFT;
    let rows = [
        Row {
            name: "Cmd+A selects all",
            setup: |app| {
                two_rectangles(app);
                app.session.execute("select.none", &json!({})).unwrap();
            },
            events: tap(Key::A, cmd()),
            check: |app| app.session.active().unwrap().selection.objects.len() == 2,
        },
        Row {
            name: "Cmd+Alt+Shift+3 hides unselected artwork",
            setup: two_rectangles,
            events: tap(Key::Num3, all),
            check: |app| {
                let doc = &app.session.doc().unwrap().doc;
                let shown = |i: usize| doc.node(NodeId(ids(app)[i])).unwrap().visible;
                shown(0) && !shown(1)
            },
        },
        Row {
            name: "Cmd+Alt+Shift+2 locks unselected artwork",
            setup: two_rectangles,
            events: tap(Key::Num2, all),
            check: |app| {
                let doc = &app.session.doc().unwrap().doc;
                let locked = |i: usize| doc.node(NodeId(ids(app)[i])).unwrap().locked;
                !locked(0) && locked(1)
            },
        },
        Row {
            name: "Cmd+Shift+O creates outlines",
            setup: |app| {
                app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Hi"})).unwrap();
            },
            events: tap(Key::O, cmd() | Modifiers::SHIFT),
            check: |app| {
                let id = app.session.active().unwrap().selection.objects.first().copied().unwrap();
                !matches!(app.session.doc().unwrap().doc.node(id).map(|n| &n.kind), Some(vectorcraft_doc::NodeKind::Text(_)))
                    && dialog_kind(app).is_none()
            },
        },
        Row {
            name: "Cmd+Shift+X swaps nothing on a rectangle but F on Perspective Snap toggles",
            setup: |app| bind(app, "perspective.grid.snap", "F"),
            events: tap(Key::F, Modifiers::NONE),
            check: |app| crate::menus::checked(app, "perspective.grid.snap", &Value::Null) == Some(false),
        },
        Row {
            name: "Page Down goes to the next artboard",
            setup: |app| {
                app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
                bind(app, "view.nextArtboard", "PageDown");
            },
            events: tap(Key::PageDown, Modifiers::NONE),
            check: |app| app.view().unwrap().artboard == 1,
        },
        Row {
            name: "a saved override on a paint key runs its new command",
            setup: |app| {
                app.session.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
                app.session.execute("paint.setFill", &json!({"color": "#336699"})).unwrap();
                bind(app, "paint.lastGradient", ",");
            },
            events: typed(Key::Comma, Modifiers::NONE, ","),
            check: |app| matches!(crate::panels::current_paints(app).0, vectorcraft_color::Paint::Gradient(_)),
        },
    ];
    for row in rows {
        let mut app = app_with_a_selected_rectangle();
        (row.setup)(&mut app);
        frame(&mut app, row.events);
        assert!((row.check)(&mut app), "{}: {}", row.name, app.ui.status);
    }
}

/// `+` zooms in however it is typed, and `-` zooms out.
#[test]
fn plus_typed_any_way_zooms_in() {
    let cmd_shift = cmd() | Modifiers::SHIFT;
    for (key, m) in [(Key::Equals, cmd()), (Key::Equals, cmd_shift), (Key::Plus, cmd_shift), (Key::Plus, cmd())] {
        let mut app = new_app();
        let ctx = ready(&mut app);
        let before = zoom(&mut app);
        key_frame(&mut app, &ctx, tap(key, m));
        assert!(zoom(&mut app) > before, "{key:?} with {m:?} zooms in");
    }
    let mut app = new_app();
    let ctx = ready(&mut app);
    let before = zoom(&mut app);
    key_frame(&mut app, &ctx, tap(Key::Minus, cmd()));
    assert!(zoom(&mut app) < before, "Cmd+- zooms out");
}

/// F2, F3 and F4 cut, copy and paste where the platform gives the aliases, and leave edited text alone.
#[test]
fn edit_function_keys_dispatch_outside_text_editing_only() {
    if crate::keymap::platform().is_mac() {
        return;
    }
    let mut app = app_with_a_selected_rectangle();
    let shape = ids(&app)[0];
    app.session.execute("select.set", &json!({"ids": [shape]})).unwrap();
    let ctx = ready(&mut app);
    key_frame(&mut app, &ctx, tap(Key::F3, Modifiers::NONE));
    assert_eq!(app.session.clipboard.nodes.len(), 1, "F3 copies the selection");
    key_frame(&mut app, &ctx, tap(Key::F2, Modifiers::NONE));
    assert!(app.session.active().unwrap().selection.objects.is_empty(), "F2 cuts the selection");
    key_frame(&mut app, &ctx, tap(Key::F4, Modifiers::NONE));
    assert_eq!(app.session.active().unwrap().selection.objects.len(), 1, "F4 pastes the copied object");

    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    edit(&mut app, 2, 2);
    for key in [Key::F2, Key::F3, Key::F4] {
        key_frame(&mut app, &ctx, tap(key, Modifiers::NONE));
        assert_eq!(text_and_layers(&mut app, id).0, "Hi", "{key:?} leaves edited text alone");
        assert!(app.session.tool_wants_text(), "{key:?} keeps editing");
    }
}

/// With Cmd+Enter bound to an effect, the key opens the effect and not the tool's Move dialog.
#[test]
fn cmd_enter_bound_to_an_effect_runs_the_effect_not_the_tool() {
    let mut app = app_with_a_selected_rectangle();
    app.select_tool("selection");
    bind(&mut app, "effect:distort.transform", "Cmd+Enter");
    frame(&mut app, tap(Key::Enter, cmd()));
    assert_eq!(open_effect(&app).as_deref(), Some("distort.transform"), "the effect's, not Move: {:?}", dialog_kind(&app));
}

/// Cmd+, opens Preferences on a Mac only, and a user binding on the chord wins.
#[test]
fn cmd_comma_opens_settings_on_a_mac() {
    let on = |os, setup: fn(&mut VectorcraftApp)| {
        let mut app = new_app();
        setup(&mut app);
        let ctx = crate::testing::ui_context();
        ctx.set_os(os);
        key_frame(&mut app, &ctx, vec![]);
        key_frame(&mut app, &ctx, tap(Key::Comma, Modifiers::COMMAND));
        crate::prefs_dialog::is_open(&app)
    };
    let (mac, linux) = (egui::os::OperatingSystem::Mac, egui::os::OperatingSystem::Nix);
    assert!(on(mac, |_| {}));
    assert!(!on(linux, |_| {}), "not elsewhere");
    assert!(!on(mac, |app| bind(app, "select.all", "Cmd+,")), "a shortcut that has it keeps it");
}

/// Every shortcut the command specs and the UI commands name is a default of the catalogue.
#[test]
fn all_registered_shortcuts_parse() {
    let snapshot = crate::shortcut_editor::snapshot();
    for platform in [craft::keymap::Platform::Windows, craft::keymap::Platform::Mac, craft::keymap::Platform::Linux] {
        let defs = crate::keymap::catalog_defs(platform, &snapshot);
        // By entry key: Tab's `window.dock` runs the framework's Panels toggle.
        let has = |id: &str| defs.iter().any(|d| crate::keymap::entry_key(&d.command) == id);
        for c in vectorcraft_engine::command_specs() {
            if let Some(s) = c.shortcut
                && !["D", "X", "Shift+X", "/"].contains(&s)
            {
                assert!(has(c.id), "{} has no default for {s} on {platform:?}", c.id);
            }
        }
        for c in crate::menus::UI_COMMANDS {
            if !c.2.is_empty() && c.2 != "F" && c.2 != "Shift+F" {
                assert!(has(c.0), "{} has no default for {} on {platform:?}", c.0, c.2);
            }
        }
    }
}

/// A command run by its chord leaves the status its menu item leaves, with no document open.
#[test]
fn a_single_key_shortcut_reports_what_its_chord_reports() {
    let status = |shortcut: &str, events: Vec<egui::Event>| {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        ready(&mut app);
        bind(&mut app, "view.screenMode", "");
        bind(&mut app, "paint.default", shortcut);
        app.ui.status.clear();
        frame(&mut app, events);
        (dialog_kind(&app), app.ui.status.clone())
    };
    let by_key = status("F", press_f());
    let by_chord = status("Cmd+Alt+Shift+F9", tap(Key::F9, cmd() | Modifiers::ALT | Modifiers::SHIFT));
    assert_eq!(by_key, by_chord);
    assert!(!by_key.1.is_empty(), "the chord reports something: {by_chord:?}");
}

/// A shortcut on a menu item with params runs what a click on that item runs.
#[test]
fn a_shortcut_runs_what_its_menu_item_runs() {
    let chord = tap(Key::F9, cmd() | Modifiers::ALT | Modifiers::SHIFT);
    for id in ["object.path.cleanUp", "addon.color.printFixes", "edit.findReplace"] {
        let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Single.session(), Default::default());
        ready(&mut app);
        bind(&mut app, id, "Cmd+Alt+Shift+F9");
        app.ui.status.clear();
        frame(&mut app, chord.clone());
        assert_eq!(dialog_kind(&app).as_deref(), Some(id), "{id}: {}", app.ui.status);
    }
}

/// A saved plain paint override is restored by the stored form of the user's shortcuts.
#[test]
fn saved_plain_paint_override_runs_its_new_command() {
    let mut app = app_with_a_selected_rectangle();
    app.session.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
    app.session.execute("paint.setFill", &json!({"color": "#336699"})).unwrap();
    bind(&mut app, "paint.lastGradient", ",");
    bind(&mut app, "paint.lastColor", ".");
    let ctx = ready(&mut app);
    key_frame(&mut app, &ctx, typed(Key::Comma, Modifiers::NONE, ","));
    assert!(matches!(crate::panels::current_paints(&app).0, vectorcraft_color::Paint::Gradient(_)));
    key_frame(&mut app, &ctx, typed(Key::Period, Modifiers::NONE, "."));
    assert_eq!(crate::panels::current_paints(&app).0.color().map(|c| c.to_hex()), Some("#336699".into()));
}

/// Cmd/Ctrl+Alt+Shift+. steps the size of the selected type by five increments, Cmd+Shift+. by one.
#[test]
fn cmd_alt_shift_period_and_comma_step_the_font_size_by_five() {
    let mut app = new_app();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One Two", "size": 12})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let five = cmd() | Modifiers::ALT | Modifiers::SHIFT;
    let ctx = ready(&mut app);
    key_frame(&mut app, &ctx, tap(Key::Period, five));
    assert_eq!(last_style(&mut app, id, "size"), json!(22.0));
    key_frame(&mut app, &ctx, tap(Key::Period, cmd() | Modifiers::SHIFT));
    assert_eq!(last_style(&mut app, id, "size"), json!(24.0), "one step");
    key_frame(&mut app, &ctx, tap(Key::Comma, five));
    assert_eq!(last_style(&mut app, id, "size"), json!(14.0));
}

// ---- T4: dialog openers --------------------------------------------------------------------

#[test]
fn a_shortcut_on_export_selection_opens_export_for_screens() {
    let mut app = app_with_shortcut("file.exportSelection", "F");
    frame(&mut app, press_f());
    assert_eq!(dialog_kind(&app).as_deref(), Some("exportForScreens"), "{}", app.ui.status);
}

/// Commands whose menu item opens a slider or a dialog before anything changes.
const MENU_DIALOG_COMMANDS: [&str; 12] = [
    "object.path.smooth",
    "object.path.simplify",
    "object.path.offsetPath",
    "object.move",
    "object.rotate",
    "object.scale",
    "object.reflect",
    "object.shear",
    "path.average",
    "object.path.splitIntoGrid",
    "object.path.cleanUp",
    "edit.findReplace",
];

/// A key or a chord opens the dialog a click on the command's menu item opens, and changes nothing.
#[test]
fn a_single_key_shortcut_opens_the_dialog_its_menu_item_opens() {
    let chord = tap(Key::F9, cmd() | Modifiers::ALT | Modifiers::SHIFT);
    let mut failures = vec![];
    for id in MENU_DIALOG_COMMANDS {
        let mut opened = |shortcut: &str, events: Vec<egui::Event>| {
            let mut app = app_with_shortcut(id, shortcut);
            let before = undo_len(&app);
            frame(&mut app, events);
            if undo_len(&app) != before {
                failures.push(format!("{id} on {shortcut} changed the document: {}", app.ui.status));
            }
            dialog_kind(&app)
        };
        let by_key = opened("F", press_f());
        let by_chord = opened("Cmd+Alt+Shift+F9", chord.clone());
        let mut app = app_with_a_selected_rectangle();
        let items: Vec<_> = crate::menus::menu_entries(&app).into_iter().filter(|e| e.command.as_deref() == Some(id)).collect();
        let item = items.iter().find(|e| e.params.as_object().is_none_or(|o| o.is_empty())).or(items.first());
        let (menu_id, p) = item.map_or_else(|| (id.to_string(), json!({})), |item| crate::menus::click_target(&item.label, id, &item.params));
        crate::menus::invoke(&mut app, &menu_id, p);
        let by_menu = dialog_kind(&app);
        if by_key.is_none() || by_key != by_chord || by_chord != by_menu {
            failures.push(format!("{id}: F opens {by_key:?}, the chord {by_chord:?}, the menu item {by_menu:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Smooth on F opens its slider without smoothing the path; Enter keeps the result as one undo step.
#[test]
fn smooth_on_a_single_key_waits_for_its_slider() {
    let mut app = app_with_shortcut("object.path.smooth", "F");
    app.session.execute("edit.clear", &json!({})).unwrap();
    let anchors: Vec<_> =
        (0..=60).map(|i| json!({"x": 20 + i * 5, "y": 150.0 + 40.0 * (f64::from(i) / 10.0).sin() + f64::from(i % 2) * 3.0})).collect();
    app.session.execute("path.create", &json!({"anchors": anchors})).unwrap();
    let before = undo_len(&app);
    let ctx = ready(&mut app);
    key_frame(&mut app, &ctx, press_f());
    idle(&mut app, &ctx, 1);
    assert!(crate::dialogs::smooth_slider::showing(&app), "{}", app.ui.status);
    assert_eq!(undo_len(&app), before, "the path is not smoothed yet");
    key_frame(&mut app, &ctx, press(Key::Enter, Modifiers::NONE));
    idle(&mut app, &ctx, 1);
    assert!(!crate::dialogs::smooth_slider::showing(&app));
    assert_eq!(undo_len(&app), before + 1, "the slider's result is one undo step");
}

/// A command without a dialog runs at once on a key without a modifier.
#[test]
fn a_single_key_shortcut_without_a_dialog_runs_at_once() {
    let mut app = app_with_shortcut("object.group", "F");
    let before = undo_len(&app);
    frame(&mut app, press_f());
    assert_eq!((dialog_kind(&app), undo_len(&app)), (None, before + 1), "F groups: {}", app.ui.status);
}

/// Cmd+Alt+Shift+J opens Average & Join; Cancel changes nothing and OK joins the lines.
#[test]
fn cmd_alt_shift_j_opens_average_and_join() {
    let mut app = new_app();
    let line = |app: &mut VectorcraftApp, x1: f64, x2: f64| {
        app.session.execute("shape.line", &json!({"x1": x1, "y1": 10, "x2": x2, "y2": 10})).unwrap()["id"].as_u64().unwrap()
    };
    let (a, b) = (line(&mut app, 10.0, 50.0), line(&mut app, 60.0, 100.0));
    app.session.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    let j = || tap(Key::J, cmd() | Modifiers::ALT | Modifiers::SHIFT);
    frame(&mut app, j());
    // A dialog is keyed by the command its OK runs.
    assert_eq!(dialog_kind(&app).as_deref(), Some("path.averageJoin"));
    close_top_dialog(&mut app, false);
    assert!(dialog_kind(&app).is_none());
    assert_eq!(app.session.doc().unwrap().doc, before, "Cancel changes nothing");
    frame(&mut app, j());
    close_top_dialog(&mut app, true);
    assert!(app.session.doc().unwrap().doc.node(NodeId(b)).is_none(), "joined into one path");
}

/// A key assigned to Color Picker… opens it for the proxy in front.
#[test]
fn a_key_assigned_to_the_color_picker_opens_it_for_the_proxy_in_front() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    ready(&mut app);
    let listed = app.run("shortcuts.list", json!({"query": "ui.colorPicker"})).unwrap();
    let rows: Vec<_> = listed.as_array().expect("rows").iter().map(|r| r["label"].clone()).collect();
    assert_eq!(rows, [json!("Color Picker…")], "Keyboard Shortcuts lists it");
    bind(&mut app, "ui.colorPicker", "Cmd+Alt+Shift+F12");
    let ctx = ready(&mut app);
    let key = || tap(Key::F12, cmd() | Modifiers::ALT | Modifiers::SHIFT);
    let opened = |app: &mut VectorcraftApp| {
        // The framework's picker is keyed by the paint it edits.
        let stroke = app.dialogs()?.with(|h| h.get::<craft::color_ui::ui::ColorPickerDialog>().map(|p| p.key() == "stroke"))?;
        close_top_dialog(app, false);
        Some(stroke)
    };
    key_frame(&mut app, &ctx, key());
    assert_eq!(opened(&mut app), Some(false), "no document: the fill; {}", app.ui.status);
    app.run("paint.toggleActive", json!({"fill": false})).unwrap();
    key_frame(&mut app, &ctx, key());
    assert_eq!(opened(&mut app), Some(true), "the Stroke proxy in front: the stroke");
}

// ---- Alt characters and typing -------------------------------------------------------------

/// The Alt chords that type a character while text is edited on Windows and Linux: the key, Shift,
/// the text the key types without Alt, and the character.
const ALT_CHARACTER_CHORDS: [(Key, bool, &str, &str); 15] = [
    (Key::Num8, false, "8", "\u{2022}"),
    (Key::G, false, "g", "\u{00A9}"),
    (Key::Semicolon, false, ";", "\u{2026}"),
    (Key::Num7, false, "7", "\u{00B6}"),
    (Key::Num6, false, "6", "\u{00A7}"),
    (Key::Num2, false, "2", "\u{2122}"),
    (Key::R, false, "r", "\u{00AE}"),
    (Key::Minus, true, "_", "\u{2014}"),
    (Key::Minus, false, "-", "\u{2013}"),
    (Key::OpenBracket, false, "[", "\u{201C}"),
    (Key::OpenBracket, true, "{", "\u{201D}"),
    (Key::OpenCurlyBracket, true, "{", "\u{201D}"),
    (Key::CloseBracket, false, "]", "\u{2018}"),
    (Key::CloseBracket, true, "}", "\u{2019}"),
    (Key::CloseCurlyBracket, true, "}", "\u{2019}"),
];

/// An Alt press of `key`, with Shift when `shift`, followed by `typed` when given.
fn alt_press(key: Key, shift: bool, typed: Option<&str>) -> Vec<egui::Event> {
    let m = if shift { Modifiers::ALT | Modifiers::SHIFT } else { Modifiers::ALT };
    let mut v = press(key, m);
    v.extend(typed.map(|t| egui::Event::Text(t.into())));
    v
}

/// Each Alt chord types its character at the caret as one undo step, with or without the key's own
/// character coming with the press, and Undo takes it out.
#[test]
fn alt_chords_type_special_characters_while_editing_text() {
    if crate::keymap::platform().is_mac() {
        return;
    }
    let mut wrong = vec![];
    for (key, shift, own, character) in ALT_CHARACTER_CHORDS {
        for typed in [None, Some(own), Some(own.to_uppercase().as_str())] {
            let (mut app, id) = editing_hi();
            let ctx = typing_ready(&mut app);
            key_frame(&mut app, &ctx, alt_press(key, shift, typed));
            let case = format!("{key:?} shift={shift} text={typed:?}");
            let text = text_and_layers(&mut app, id).0;
            if text != format!("Hi{character}") {
                wrong.push(format!("{case}: text = {text:?}"));
                continue;
            }
            app.session.execute("edit.undo", &json!({})).unwrap();
            let undone = text_and_layers(&mut app, id).0;
            if undone != "Hi" {
                wrong.push(format!("{case}: after Undo = {undone:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// An Alt chord replaces the selected text, and a held chord types its character once per press.
#[test]
fn alt_chord_replaces_selected_text_and_repeats() {
    if crate::keymap::platform().is_mac() {
        return;
    }
    let (mut app, id) = editing_hi();
    app.session.set_tool_option("select", &json!({"start": 0, "end": 2}));
    let ctx = typing_ready(&mut app);
    key_frame(&mut app, &ctx, alt_press(Key::G, false, Some("g")));
    assert_eq!(text_and_layers(&mut app, id).0, "\u{00A9}");
    let mut held = alt_press(Key::Num8, false, Some("8"));
    held.push(egui::Event::Key { key: Key::Num8, physical_key: Some(Key::Num8), pressed: true, repeat: true, modifiers: Modifiers::ALT });
    held.push(egui::Event::Text("8".into()));
    key_frame(&mut app, &ctx, held);
    assert_eq!(text_and_layers(&mut app, id).0, "\u{00A9}\u{2022}\u{2022}");
}

/// Text the layout or an input method types with Alt is typed once, as it came.
#[test]
fn alt_chord_text_from_the_layout_is_typed_once() {
    let mut ime_commit = alt_press(Key::G, false, None);
    ime_commit.push(egui::Event::Ime(egui::ImeEvent::Commit("\u{011F}".into())));
    for (events, expected) in [
        (ime_commit, "Hi\u{011F}"),
        (alt_press(Key::G, false, Some("\u{00A9}")), "Hi\u{00A9}"),
        (alt_press(Key::G, false, Some("\u{011F}")), "Hi\u{011F}"),
        (alt_press(Key::Num8, false, Some("ab")), "Hiab"),
    ] {
        let (mut app, id) = editing_hi();
        frame(&mut app, events.clone());
        assert_eq!(text_and_layers(&mut app, id).0, expected, "{events:?}");
    }
}

/// Outside text editing the Alt chords type nothing.
#[test]
fn alt_chords_outside_text_editing_type_nothing() {
    for tool in ["selection", "type"] {
        let mut app = new_app();
        let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
        app.select_tool(tool);
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        assert!(!app.session.tool_wants_text(), "{tool}");
        let ctx = ready(&mut app);
        for (key, shift, own, _) in ALT_CHARACTER_CHORDS {
            key_frame(&mut app, &ctx, alt_press(key, shift, Some(own)));
            key_frame(&mut app, &ctx, alt_press(key, shift, None));
        }
        assert_eq!(text_and_layers(&mut app, id).0, "Hi", "{tool}");
    }
}

/// Alt+P chooses the Quick Pen and is no Alt tap; a lone Alt tap then switches its action, and Alt
/// let go after a click does not.
#[test]
fn alt_p_chooses_the_quick_pen_and_an_alt_tap_switches_its_action() {
    let mut app = new_app();
    let ctx = ready(&mut app);
    let held = |app: &mut VectorcraftApp, m: Modifiers, mut events: Vec<egui::Event>| {
        events.insert(0, egui::Event::ModifiersChanged(m));
        key_frame(app, &ctx, events);
    };
    held(&mut app, Modifiers::ALT, alt_press(Key::P, false, Some("\u{3c0}")));
    assert_eq!(app.session.tool_id(), vectorcraft_tools::quickpen::ID);
    held(&mut app, Modifiers::NONE, vec![]);
    assert_eq!(app.session.tool_options()["action"], "line", "Alt+P is no tap");
    held(&mut app, Modifiers::ALT, vec![]);
    held(&mut app, Modifiers::NONE, vec![]);
    assert_eq!(app.session.tool_options()["action"], "continuousArc");
    held(&mut app, Modifiers::ALT, vec![]);
    let click =
        egui::Event::PointerButton { pos: egui::pos2(5.0, 5.0), button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::ALT };
    held(&mut app, Modifiers::ALT, vec![click]);
    held(&mut app, Modifiers::NONE, vec![]);
    assert_eq!(app.session.tool_options()["action"], "continuousArc", "a click in between");
}

/// While the Type tool edits, a type chord runs the type command and not the object command.
#[test]
fn typing_chords_run_type_commands_not_object_commands() {
    use vectorcraft_doc::Justify;
    type Check = fn(&mut VectorcraftApp, u64) -> bool;
    let (c, shift) = (cmd(), Modifiers::SHIFT);
    let rows: [(&str, Vec<egui::Event>, Check); 7] = [
        ("Cmd+Shift+B is bold, not the bounding box", tap(Key::B, c | shift), |app, _| {
            let face = crate::panels::character::text_style(app).map(|(s, _)| s.font_style).unwrap_or_default();
            app.ui.view.bounding_box && (face.contains("Bold") || app.ui.status.contains("has no Bold style"))
        }),
        ("Cmd+Shift+R aligns right", tap(Key::R, c | shift), |app, id| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            vectorcraft_doc::NodeKind::Text(t) => t.para_at(0).justify == Justify::Right,
            _ => false,
        }),
        ("Cmd+Alt+Q resets tracking and kerning", tap(Key::Q, c | Modifiers::ALT), |app, id| {
            last_style(app, id, "tracking") == json!(0.0) && last_style(app, id, "kerning").is_null()
        }),
        ("Cmd+Alt+K highlights the Kerning field", tap(Key::K, c | Modifiers::ALT), |app, _| {
            app.character_focus == Some(crate::panels::character::Field::Kerning) && dialog_kind(app).is_none()
        }),
        ("Cmd+Shift+X resets the horizontal scale", vec![egui::Event::ModifiersChanged(c | shift), egui::Event::Cut], |app, id| {
            last_style(app, id, "h_scale") == json!(100.0)
        }),
        ("Cmd+Shift+U underlines", tap(Key::U, c | shift), |app, id| last_style(app, id, "underline") == json!(true)),
        ("Cmd+Shift+= is superscript", tap(Key::Equals, c | shift), |app, id| {
            let p = last_style(app, id, "position");
            p.get("kind").unwrap_or(&p) == &json!("superscript")
        }),
    ];
    for (name, events, check) in rows {
        let mut app = new_app();
        let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One Two"})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("text.setRangeStyle", &json!({"id": id, "tracking": 200, "kerning": -40, "hScale": 150})).unwrap();
        edit(&mut app, 4, 7);
        frame(&mut app, events);
        assert!(check(&mut app, id), "{name}: {}", app.ui.status);
    }
}

/// A Type tool command the user gives another chord runs on that chord while the tool types, and no longer on its old
/// one.
#[test]
fn a_type_command_moved_to_another_chord_runs_on_it_while_typing() {
    use vectorcraft_doc::Justify;
    let mut app = new_app();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One Two"})).unwrap()["id"].as_u64().unwrap();
    let right = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => t.para_at(0).justify == Justify::Right,
        _ => false,
    };
    let ctx = ready(&mut app);
    edit(&mut app, 4, 7);
    app.run("shortcuts.set", json!({"id": "type.alignRight", "shortcut": "Cmd+Shift+F13"})).unwrap();
    key_frame(&mut app, &ctx, vec![]);
    key_frame(&mut app, &ctx, tap(Key::R, cmd() | Modifiers::SHIFT));
    assert!(!right(&app), "the old chord");
    key_frame(&mut app, &ctx, tap(Key::F13, cmd() | Modifiers::SHIFT));
    assert!(right(&app), "the new chord: {}", app.ui.status);
}

// ---- Nudge table (kb.C23) ------------------------------------------------------------------

/// The nudge of `dir` with `mods` on the selection.
fn nudge(app: &mut VectorcraftApp, dir: egui::Vec2, mods: Modifiers) {
    super::nudge(app, dir, craft::keymap::Mods::from_egui(&mods, crate::keymap::platform()), false);
}

const UP: egui::Vec2 = egui::vec2(0.0, -1.0);
const DOWN: egui::Vec2 = egui::vec2(0.0, 1.0);
const LEFT: egui::Vec2 = egui::vec2(-1.0, 0.0);
const RIGHT: egui::Vec2 = egui::vec2(1.0, 0.0);

/// Alt+arrows step the spacing of selected vertical type, Alt+Shift the baseline and Alt+Cmd by five.
#[test]
fn alt_arrows_rotate_for_selected_vertical_type() {
    let mut app = new_app();
    let vertical = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One Two", "size": 20, "vertical": true})).unwrap()["id"]
        .as_u64()
        .unwrap();
    app.select_tool("selection");
    app.session.execute("select.set", &json!({"ids": [vertical]})).unwrap();
    let (alt, alt_shift, alt_cmd) = (Modifiers::ALT, Modifiers::ALT | Modifiers::SHIFT, Modifiers::ALT | Modifiers::COMMAND);
    for (dir, mods, field, value) in [
        (DOWN, alt, "tracking", 20.0),
        (UP, alt, "tracking", 0.0),
        (LEFT, alt, "leading", 26.0),
        (RIGHT, alt, "leading", 24.0),
        (RIGHT, alt_shift, "baseline_shift", 2.0),
        (LEFT, alt_shift, "baseline_shift", 0.0),
        (DOWN, alt_cmd, "tracking", 100.0),
        (UP, alt_cmd, "tracking", 0.0),
    ] {
        nudge(&mut app, dir, mods);
        assert_eq!(last_style(&mut app, vertical, field), json!(value), "{dir:?} {mods:?}");
        assert_eq!(app.session.active().unwrap().selection.objects, [NodeId(vertical)]);
    }
}

/// A mixed selection takes the horizontal spacing step, in both selection orders.
#[test]
fn mixed_type_objects_take_horizontal_spacing_chords_in_both_orders() {
    for vertical_first in [true, false] {
        let mut app = new_app();
        let vertical = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One", "size": 20, "vertical": true})).unwrap()["id"]
            .as_u64()
            .unwrap();
        let horizontal = app.session.execute("text.create", &json!({"x": 80, "y": 50, "text": "Two", "size": 20})).unwrap()["id"].as_u64().unwrap();
        let order = if vertical_first { [vertical, horizontal] } else { [horizontal, vertical] };
        app.select_tool("type");
        app.session.execute("select.set", &json!({"ids": order})).unwrap();
        let before = undo_len(&app);
        nudge(&mut app, DOWN, Modifiers::ALT);
        for id in order {
            assert_eq!(last_style(&mut app, id, "leading"), json!(26.0));
        }
        assert_eq!(undo_len(&app), before + 1, "one undo step");
        nudge(&mut app, RIGHT, Modifiers::ALT);
        for id in order {
            assert_eq!(last_style(&mut app, id, "tracking"), json!(20.0));
        }
        assert_eq!(app.session.active().unwrap().selection.objects.len(), 2);
    }
}

#[test]
fn selected_vertical_area_and_path_type_use_vertical_spacing_chords() {
    let mut app = new_app();
    let area_frame = app.session.execute("path.create", &json!({"d": "M10 10 L100 10 L100 100 L10 100 Z"})).unwrap()["id"].as_u64().unwrap();
    let path = app.session.execute("path.create", &json!({"d": "M150 10 L250 10"})).unwrap()["id"].as_u64().unwrap();
    let area =
        app.session.execute("text.createInPath", &json!({"path": area_frame, "mode": "area", "text": "Area", "vertical": true, "size": 20})).unwrap()
            ["id"]
            .as_u64()
            .unwrap();
    let on_path = app
        .session
        .execute("text.createInPath", &json!({"path": path, "mode": "onPath", "text": "Path", "vertical": true, "size": 20}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    app.select_tool("selection");
    app.session.execute("select.set", &json!({"ids": [area, on_path]})).unwrap();
    nudge(&mut app, DOWN, Modifiers::ALT);
    for id in [area, on_path] {
        assert_eq!(last_style(&mut app, id, "tracking"), json!(20.0));
    }
}

/// Alt+arrows step a selected type object instead of copying it; a rectangle with Alt is copied.
#[test]
fn alt_arrows_step_selected_type_object() {
    let mut app = new_app();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One Two", "size": 20})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("selection");
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    nudge(&mut app, RIGHT, Modifiers::ALT);
    assert_eq!(last_style(&mut app, id, "tracking"), json!(20.0));
    nudge(&mut app, DOWN, Modifiers::ALT);
    assert_eq!(last_style(&mut app, id, "leading"), json!(26.0));
    nudge(&mut app, UP, Modifiers::ALT | Modifiers::SHIFT);
    assert_eq!(last_style(&mut app, id, "baseline_shift"), json!(2.0));
    // Alt+Shift+Left/Right have no spacing action but must not fall through to copy-and-nudge.
    for dir in [LEFT, RIGHT] {
        nudge(&mut app, dir, Modifiers::ALT | Modifiers::SHIFT);
        assert_eq!(app.session.active().unwrap().selection.objects, [NodeId(id)]);
    }
    let rect = app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [rect]})).unwrap();
    nudge(&mut app, RIGHT, Modifiers::ALT);
    let selected = &app.session.active().unwrap().selection.objects;
    assert_eq!(selected.len(), 1);
    assert_ne!(selected[0], NodeId(rect), "a non-type object is still copied by Alt+arrow");
}

// ---- What each tool takes ------------------------------------------------------------------

/// While a grid is dragged out, up and down change its rows and left and right its columns.
#[test]
fn arrows_while_dragging_a_grid_change_its_rows_and_columns() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = new_app();
    app.select_tool("rectangularGrid");
    let view = app.view_info();
    let ctx = ready(&mut app);
    let grid = |app: &mut VectorcraftApp, keys: &[Key]| {
        app.session.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 80.0), view).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Drag, 300.0, 220.0), view).unwrap();
        focus_canvas(app, &ctx);
        for &key in keys {
            key_frame(app, &ctx, press(key, Modifiers::NONE));
        }
        app.session.pointer(&PointerEvent::new(PointerKind::Up, 300.0, 220.0), view).unwrap();
        let g = app.session.doc().unwrap().doc.layers[0].children().unwrap().last().unwrap().clone();
        let vectorcraft_doc::NodeKind::Group { children, .. } = &g.kind else { panic!("not a grid: {:?}", g.kind) };
        let bounds: Vec<_> = children.iter().filter_map(|c| c.geometric_bounds()).collect();
        (bounds.iter().filter(|b| b.height() < 0.5).count(), bounds.iter().filter(|b| b.width() < 0.5).count())
    };
    let (rows, columns) = grid(&mut app, &[]);
    assert_eq!(grid(&mut app, &[Key::ArrowRight, Key::ArrowRight, Key::ArrowUp]), (rows + 1, columns + 2));
    assert_eq!(grid(&mut app, &[Key::ArrowLeft, Key::ArrowDown]), (rows, columns + 1));
}

/// Grid letters change the stored skews once per press while a polar grid is dragged, plain or with
/// Shift; with Alt, Ctrl or Cmd they do not reach the grid.
#[test]
fn grid_drag_letters_take_plain_and_shift_presses() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = new_app();
    app.select_tool("polarGrid");
    let view = app.view_info();
    let ctx = ready(&mut app);
    app.session.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 80.0), view).unwrap();
    app.session.pointer(&PointerEvent::new(PointerKind::Drag, 300.0, 220.0), view).unwrap();
    focus_canvas(&mut app, &ctx);
    let skews = |app: &VectorcraftApp| (app.session.tool_options()["radialSkew"].clone(), app.session.tool_options()["concentricSkew"].clone());
    key_frame(&mut app, &ctx, press(Key::V, Modifiers::NONE));
    assert_eq!(skews(&app), (json!(10.0), json!(0.0)));
    key_frame(&mut app, &ctx, press(Key::F, Modifiers::SHIFT));
    assert_eq!(skews(&app), (json!(0.0), json!(0.0)));
    key_frame(&mut app, &ctx, press(Key::V, Modifiers::SHIFT));
    assert_eq!(skews(&app), (json!(10.0), json!(0.0)));
    key_frame(&mut app, &ctx, press(Key::X, Modifiers::SHIFT));
    assert_eq!(skews(&app), (json!(10.0), json!(-10.0)));
    key_frame(&mut app, &ctx, press(Key::C, Modifiers::SHIFT));
    assert_eq!(skews(&app), (json!(10.0), json!(0.0)));
    key_frame(&mut app, &ctx, press(Key::X, Modifiers::ALT));
    assert_eq!(skews(&app), (json!(10.0), json!(0.0)));
    for key in [Key::F, Key::X] {
        key_frame(&mut app, &ctx, press(key, command_key()));
        assert_eq!(skews(&app), (json!(10.0), json!(0.0)), "{key:?} with Ctrl/Cmd");
    }
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 300.0, 220.0), view).unwrap();
    assert_eq!(skews(&app), (json!(10.0), json!(0.0)), "the options stay after release");
}

#[test]
fn enclosing_lasso_escape_cancels_and_q_returns_to_anchor_lasso() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = new_app();
    let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.select_tool(vectorcraft_tools::addons::enclosing_lasso::ID);
    let view = app.view_info();
    let ctx = ready(&mut app);
    app.session.pointer(&PointerEvent::new(PointerKind::Down, 90.0, 90.0), view).unwrap();
    app.session.pointer(&PointerEvent::new(PointerKind::Drag, 210.0, 90.0), view).unwrap();
    assert!(app.session.tool_busy());
    key_frame(&mut app, &ctx, tap(Key::Escape, Modifiers::NONE));
    assert!(!app.session.tool_busy());
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 90.0, 210.0), view).unwrap();
    assert_eq!(app.session.active().unwrap().selection.objects, vec![NodeId(id)]);
    key_frame(&mut app, &ctx, typed(Key::Q, Modifiers::NONE, "q"));
    assert_eq!(app.session.tool_id(), "lasso");
}

/// While a Polygonal Lasso loop is open, Backspace and Delete remove its last vertex, Enter closes
/// it, and Escape drops it and leaves the selection.
#[test]
fn polygonal_lasso_takes_backspace_delete_enter_and_escape_while_a_loop_is_open() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = new_app();
    let id = NodeId(app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    app.select_tool(vectorcraft_tools::addons::polygonal_lasso::ID);
    let view = app.view_info();
    let ctx = ready(&mut app);
    let click = |app: &mut VectorcraftApp, x: f64, y: f64| {
        app.session.pointer(&PointerEvent::new(PointerKind::Down, x, y), view).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Up, x, y), view).unwrap();
    };
    for (x, y) in [(80.0, 80.0), (140.0, 80.0), (300.0, 300.0)] {
        click(&mut app, x, y);
    }
    focus_canvas(&mut app, &ctx);
    key_frame(&mut app, &ctx, tap(Key::Backspace, Modifiers::NONE));
    assert!(app.session.active().unwrap().doc.node(id).is_some(), "Backspace leaves the selected art");
    click(&mut app, 300.0, 300.0);
    key_frame(&mut app, &ctx, tap(Key::Delete, Modifiers::NONE));
    assert!(app.session.active().unwrap().doc.node(id).is_some(), "Delete leaves the selected art");
    assert!(app.session.tool_ends_on_escape(view));
    key_frame(&mut app, &ctx, tap(Key::Escape, Modifiers::NONE));
    assert!(!app.session.tool_ends_on_escape(view));
    assert_eq!(app.session.active().unwrap().selection.objects, vec![id]);
    for (x, y) in [(80.0, 80.0), (140.0, 80.0), (300.0, 300.0)] {
        click(&mut app, x, y);
    }
    focus_canvas(&mut app, &ctx);
    key_frame(&mut app, &ctx, tap(Key::Backspace, Modifiers::NONE));
    click(&mut app, 80.0, 140.0);
    key_frame(&mut app, &ctx, tap(Key::Enter, Modifiers::NONE));
    assert!(!app.session.tool_ends_on_escape(view));
    let sel = &app.session.active().unwrap().selection;
    assert_eq!(sel.objects, vec![id]);
    assert_eq!(sel.anchors.get(&id).map(|a| a.len()), Some(1), "only the corner inside the loop");
}

/// Typed `[` and `]` size the Blob Brush and the Eraser, and leave the stroke weight alone.
#[test]
fn brackets_size_the_blob_brush_and_the_eraser() {
    let mut app = new_app();
    app.session.execute("stroke.set", &json!({"weight": 3})).unwrap();
    let size = |app: &mut VectorcraftApp| app.session.tool_options()["size"].as_f64().unwrap();
    let text = |t: &str| egui::Event::Text(t.into());
    let ctx = ready(&mut app);
    for tool in ["blobBrush", "eraser"] {
        app.select_tool(tool);
        // The tool takes its keys from the first frame that shows it.
        key_frame(&mut app, &ctx, vec![]);
        let before = size(&mut app);
        key_frame(&mut app, &ctx, vec![text("]")]);
        key_frame(&mut app, &ctx, vec![text("]")]);
        assert_eq!(size(&mut app), before + 2.0, "{tool}");
        // Option-8 types `[` on a Turkish Mac keyboard, so the step runs on a macOS shell: elsewhere Alt+digit starts an Alt code.
        let mac = crate::testing::ui_context();
        craft::ui::testing::windows::set_os(&mac, craft::host::target::Os::Mac);
        key_frame(&mut app, &mac, vec![]);
        let option_8 = egui::Event::Key { key: Key::Num8, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::ALT };
        key_frame(&mut app, &mac, vec![option_8, text("[")]);
        assert_eq!(size(&mut app), before + 1.0, "{tool}: a bracket typed with Option");
    }
    assert_eq!(app.session.paint.stroke_width, 3.0, "the stroke weight stays");
    app.select_tool("rectangle");
    key_frame(&mut app, &ctx, vec![text("[")]);
    assert_eq!(app.session.paint.stroke_width, 3.0);
}

/// A typed `[` reaches the Quick Pen while a press drags a sharp-corner arc.
#[test]
fn brackets_size_the_quick_pen_arc_being_dragged() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = new_app();
    app.select_tool(vectorcraft_tools::quickpen::ID);
    app.session.set_tool_option_cmd(&json!({"tool": "quickPen", "key": "action", "value": "sharpArc"})).unwrap();
    let v = vectorcraft_engine::ViewInfo { smart_guides: false, ..Default::default() };
    app.session.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 150.0), v).unwrap();
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 100.0, 150.0), v).unwrap();
    app.session.pointer(&PointerEvent::new(PointerKind::Down, 180.0, 150.0), v).unwrap();
    app.session.pointer(&PointerEvent::new(PointerKind::Drag, 190.0, 150.0), v).unwrap();
    assert!(app.session.tool_busy());
    let points = |app: &VectorcraftApp| {
        let mut v = vec![];
        app.session.doc().unwrap().doc.walk(|n| {
            if let Some(pd) = n.path_data() {
                v.extend(pd.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p)));
            }
        });
        v
    };
    let before = points(&app);
    assert_eq!(before.len(), 3, "the arc being dragged is drawn");
    frame(&mut app, vec![egui::Event::Text("[".into())]);
    assert_eq!(app.session.tool_options()["turn"], 165.0);
    let after = points(&app);
    assert_eq!(after.len(), 3);
    assert_ne!(after[1], before[1], "the arc being dragged is drawn again with the new size");
}

/// With the Live Paint Bucket a plain Right picks the next swatch; Alt+Right and Shift+Right do not.
#[test]
fn live_paint_bucket_takes_only_plain_arrows_for_swatches() {
    let mut app = new_app();
    let rect = app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [rect]})).unwrap();
    let first = vectorcraft_tools::builder::color_swatches(&app.session.active().unwrap().doc)[0].name.clone();
    app.session.execute("paint.setFill", &json!({"swatch": first, "ids": []})).unwrap();
    app.select_tool("livePaintBucket");
    let fill = app.session.paint.fill.clone();
    let ctx = ready(&mut app);
    key_frame(&mut app, &ctx, tap(Key::ArrowRight, Modifiers::ALT));
    assert_eq!(app.session.paint.fill, fill, "Alt+Right picks no swatch");
    assert_ne!(app.session.active().unwrap().selection.objects.as_slice(), [NodeId(rect)], "Alt+Right copies and nudges the selection");
    key_frame(&mut app, &ctx, tap(Key::ArrowRight, Modifiers::SHIFT));
    assert_eq!(app.session.paint.fill, fill, "Shift+Right picks no swatch");
    key_frame(&mut app, &ctx, tap(Key::ArrowRight, Modifiers::NONE));
    assert_ne!(app.session.paint.fill, fill, "a plain Right picks the next swatch");
}

/// Up steps the corner kind only while a corner widget is pressed; otherwise the window nudges.
#[test]
fn a_plain_up_nudges_unless_a_corner_widget_is_pressed() {
    use vectorcraft_doc::{LiveShape, NodeKind};
    use vectorcraft_geom::shapes::CornerKind;
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = new_app();
    let id =
        NodeId(app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 120, "height": 80})).unwrap()["id"].as_u64().unwrap());
    app.select_tool("selection");
    app.session.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let state = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        let b = st.doc.bounds_of(&[id], false).unwrap();
        let NodeKind::Path { live: Some(l), .. } = &st.doc.node(id).unwrap().kind else { panic!("not live") };
        let LiveShape::Rectangle { kinds, .. } = &**l else { panic!("not a rectangle") };
        ((b.x0, b.y0), kinds[0])
    };
    let ctx = ready(&mut app);
    let ((x, y), kind) = state(&app);
    key_frame(&mut app, &ctx, tap(Key::ArrowUp, Modifiers::NONE));
    let ((x1, y1), kind1) = state(&app);
    assert_eq!(x1, x, "no sideways move");
    assert!((y1 - y).abs() > 0.5, "a plain Up nudges the rectangle: {y} → {y1}");
    assert_eq!(kind1, kind, "and leaves its corners alone");
    let view = app.view_info();
    let st = app.session.active().unwrap();
    let w = vectorcraft_tools::corners::CornerWidgets::showing(&st.doc, &st.selection, view.zoom, false, true, 180.0).expect("corner widgets show");
    let at = w.visible().next().expect("a widget");
    app.session.pointer(&PointerEvent::new(PointerKind::Down, at.x, at.y), view).unwrap();
    key_frame(&mut app, &ctx, tap(Key::ArrowUp, Modifiers::NONE));
    app.session.pointer(&PointerEvent::new(PointerKind::Up, at.x, at.y), view).unwrap();
    let ((x2, y2), kind2) = state(&app);
    assert_eq!((x2, y2), (x1, y1), "no nudge while the widget is pressed");
    assert_eq!(kind2, CornerKind::InvertedRound, "Up stepped the kind");
}

/// While a blend widget is pressed, an arrow steps its value by one and Shift with an arrow by ten.
#[test]
fn shift_arrows_step_a_pressed_blend_widget_by_ten() {
    use vectorcraft_doc::NodeKind;
    use vectorcraft_tools::blendwidget::BlendWidget;
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = new_app();
    let rect = |app: &mut VectorcraftApp, x: f64| {
        app.session.execute("shape.rectangle", &json!({"x": x, "y": 90, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap()
    };
    let (a, b) = (rect(&mut app, 30.0), rect(&mut app, 150.0));
    app.session.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let id = NodeId(app.session.execute("object.blend.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    app.select_tool("selection");
    app.session.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let shift = |app: &VectorcraftApp| {
        let NodeKind::Blend { spec, .. } = &app.session.active().unwrap().doc.node(id).unwrap().kind else { panic!("not a blend") };
        spec.color_shift
    };
    let view = app.view_info();
    let ctx = ready(&mut app);
    let st = app.session.active().unwrap();
    let at = BlendWidget::of(&st.doc, &st.selection, view.zoom, vectorcraft_geom::Vec2::new(0.0, -1.0))[0].at;
    app.session.pointer(&PointerEvent::new(PointerKind::Down, at.x, at.y), view).unwrap();
    key_frame(&mut app, &ctx, tap(Key::ArrowUp, Modifiers::SHIFT));
    assert_eq!(shift(&app), 10.0, "Shift+Up steps by ten");
    key_frame(&mut app, &ctx, tap(Key::ArrowUp, Modifiers::NONE));
    assert_eq!(shift(&app), 11.0, "Up steps by one");
    app.session.pointer(&PointerEvent::new(PointerKind::Up, at.x, at.y), view).unwrap();
    assert_eq!(shift(&app), 11.0);
}

// ---- Hold bindings -------------------------------------------------------------------------

/// Frames enough to pass the hold delay.
const HOLD_FRAMES: usize = 30;

/// Period taps its own command, and a `Hold .` binding runs Deselect when held.
#[test]
fn the_period_key_taps_its_own_command_and_holds_to_deselect() {
    let mut app = app_with_a_selected_rectangle();
    app.session.prefs.hold_shortcuts = true;
    app.session.execute("paint.setFill", &json!({"color": "#336699"})).unwrap();
    bind(&mut app, "select.none", "Hold .");
    let ctx = ready(&mut app);
    key_frame(&mut app, &ctx, typed(Key::Period, Modifiers::NONE, "."));
    key_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Key { key: Key::Period, physical_key: Some(Key::Period), pressed: false, repeat: false, modifiers: Modifiers::NONE }],
    );
    assert!(matches!(crate::panels::current_paints(&app).0, vectorcraft_color::Paint::Gradient(_)), "a tap runs its own command");
    assert!(!app.session.active().unwrap().selection.objects.is_empty(), "a tap deselects nothing");
    key_frame(&mut app, &ctx, press(Key::Period, Modifiers::NONE));
    idle(&mut app, &ctx, HOLD_FRAMES);
    assert!(app.session.active().unwrap().selection.objects.is_empty(), "a hold deselects");
}

/// A key with a hold binding beats `holdToolKeys`: it starts no held tool.
#[test]
fn a_hold_shortcut_takes_a_tool_key_from_hold_tool_keys() {
    let mut app = app_with_a_selected_rectangle();
    app.session.prefs.hold_shortcuts = true;
    // PLAN: assumed the `holdToolKeys` setting is `app.session.prefs.hold_tool_keys`.
    app.session.prefs.hold_tool_keys = true;
    app.select_tool("selection");
    bind(&mut app, "select.none", "Hold P");
    let ctx = ready(&mut app);
    key_frame(&mut app, &ctx, press(Key::P, Modifiers::NONE));
    idle(&mut app, &ctx, HOLD_FRAMES);
    assert_ne!(app.session.tool_id(), "pen", "the hold binding took the key");
    assert!(app.session.active().unwrap().selection.objects.is_empty(), "and ran its command");
}

/// A tool key with no hold binding still starts a held tool.
#[test]
fn other_tool_keys_keep_hold_tool_keys() {
    let mut app = app_with_a_selected_rectangle();
    app.session.prefs.hold_shortcuts = true;
    app.session.prefs.hold_tool_keys = true;
    app.select_tool("selection");
    let ctx = ready(&mut app);
    key_frame(&mut app, &ctx, press(Key::P, Modifiers::NONE));
    idle(&mut app, &ctx, HOLD_FRAMES);
    assert_eq!(app.session.tool_id(), "pen", "held: the tool is chosen");
    key_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Key { key: Key::P, physical_key: Some(Key::P), pressed: false, repeat: false, modifiers: Modifiers::NONE }],
    );
    assert_eq!(app.session.tool_id(), "selection", "let go: the held tool ends");
}

// ---- The app's Enter and Escape decisions --------------------------------------------------

/// Escape while an opacity mask is edited leaves mask editing.
#[test]
fn escape_while_an_opacity_mask_is_edited_leaves_mask_editing() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let obj = app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let m = app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 100})).unwrap()["id"].clone();
    app.session.execute("select.set", &json!({"ids": [obj, m]})).unwrap();
    app.session.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    app.session.execute("transparency.editOpacityMask", &json!({})).unwrap();
    app.select_tool("selection");
    frame(&mut app, press(Key::Escape, Modifiers::NONE));
    let st = app.session.active().unwrap();
    assert!(st.doc.mask_edit.is_none() && st.isolation.is_none(), "Escape leaves mask editing");
    assert_eq!(st.doc.layers.len(), 1, "the editing layer is removed");
    assert_eq!(st.selection.objects, vec![NodeId(obj)]);
}

/// Enter opens the tool's dialog and the Enter that opened it isn't also its OK.
#[test]
fn enter_opens_the_tools_dialog_and_leaves_it_open() {
    let mut app = new_app();
    let ctx = ready(&mut app);
    let enter = || tap(Key::Enter, Modifiers::NONE);
    let steps = |app: &VectorcraftApp| (app.session.active().unwrap().history.undo.len(), app.session.journal.len());
    app.select_tool("selection");
    key_frame(&mut app, &ctx, enter());
    assert_eq!(dialog_kind(&app), None, "nothing is selected");
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 30})).unwrap();
    for (tool, dialog) in [
        ("selection", "object.move"),
        ("directSelection", "object.move"),
        ("groupSelection", "object.move"),
        ("rotate", "object.rotate"),
        ("scale", "object.scale"),
        ("reflect", "object.reflect"),
        ("shear", "object.shear"),
    ] {
        app.select_tool(tool);
        let before = steps(&app);
        key_frame(&mut app, &ctx, enter());
        assert_eq!(dialog_kind(&app).as_deref(), Some(dialog), "{tool}: Enter opens it");
        key_frame(&mut app, &ctx, vec![]);
        assert_eq!(dialog_kind(&app).as_deref(), Some(dialog), "{tool}: and it stays open");
        assert_eq!(steps(&app), before, "{tool}: the Enter that opened it didn't confirm it");
        // With its default values only Reflect changes the art. A held OK closes its dialog on the window's next pass,
        // and its field gives up the keyboard in the frame after that.
        key_frame(&mut app, &ctx, enter());
        key_frame(&mut app, &ctx, vec![]);
        key_frame(&mut app, &ctx, vec![]);
        let after = (before.0 + usize::from(dialog == "object.reflect"), before.1 + 1);
        assert_eq!((dialog_kind(&app), steps(&app)), (None, after), "{tool}: Enter confirms");
    }
    app.select_tool("rectangle");
    key_frame(&mut app, &ctx, enter());
    assert_eq!(dialog_kind(&app), None, "another tool has its own Enter");
}

/// A drag with the Selection tool, then Enter: Move opens on the drag's offset and repeats it.
#[test]
fn enter_after_a_drag_repeats_its_offset() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 800, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let enter = || tap(Key::Enter, Modifiers::NONE);
    let lefts = |app: &VectorcraftApp| {
        let kids = app.session.doc().unwrap().doc.layers[0].children().unwrap();
        kids.iter().map(|n| n.geometric_bounds().unwrap().x0.round()).collect::<Vec<_>>()
    };
    app.select_tool("selection");
    let view = app.view_info();
    for (kind, x, y) in
        [(PointerKind::Down, 20.0, 20.0), (PointerKind::Drag, 40.0, 20.0), (PointerKind::Drag, 60.0, 20.0), (PointerKind::Up, 60.0, 20.0)]
    {
        app.session.pointer(&PointerEvent::new(kind, x, y), view).unwrap();
    }
    let [moved] = lefts(&app)[..] else { panic!("one rectangle") };
    let step = moved - 10.0;
    assert!(step > 0.0);
    // The window's first frame comes after the drag, which the view before it measures: one fitted to the window
    // would put the press on a corner handle.
    let ctx = ready(&mut app);
    key_frame(&mut app, &ctx, enter());
    assert_eq!(dialog_kind(&app).as_deref(), Some("object.move"), "Enter opens Move");
    assert_eq!(dialog_field(&app, "dx").and_then(|v| v.as_f64()).map(f64::round), Some(step));
    set_top_field(&mut app, "copy", json!(true));
    key_frame(&mut app, &ctx, enter());
    // A held OK closes its dialog on the window's next pass.
    key_frame(&mut app, &ctx, vec![]);
    for _ in 0..2 {
        key_frame(&mut app, &ctx, enter());
        key_frame(&mut app, &ctx, enter());
        key_frame(&mut app, &ctx, vec![]);
    }
    assert!(dialog_kind(&app).is_none());
    assert_eq!(lefts(&app), (0..4).map(|i| moved + step * f64::from(i)).collect::<Vec<_>>());
}

// ---- Clipboard -----------------------------------------------------------------------------

/// The text a frame copied to the system clipboard (egui's CopyText output command).
fn copied(out: &egui::FullOutput) -> Option<String> {
    out.platform_output.commands.iter().find_map(|c| match c {
        egui::OutputCommand::CopyText(t) => Some(t.clone()),
        _ => None,
    })
}

/// [`key_frame`] with the frame's output.
fn output_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    let time = ctx.input(|i| i.time) + 0.05;
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
    crate::testing::app_frame(app, ctx, egui::RawInput { screen_rect, focused: true, time: Some(time), events, ..Default::default() })
}

#[test]
fn copy_and_paste_events_use_the_system_clipboard() {
    let mut app = new_app();
    let id = app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let ctx = ready(&mut app);
    let count = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].children().unwrap().len();
    // Copy publishes SVG to the system clipboard (egui's CopyText output command): the Copy the event fired runs as
    // the next frame begins, and that frame hands the text over.
    let svg =
        [vec![egui::Event::Copy], vec![]].into_iter().find_map(|events| copied(&output_frame(&mut app, &ctx, events))).expect("copy publishes SVG");
    assert!(svg.contains("<svg"));
    // The copy is handed to egui once: no second command follows.
    assert!(copied(&output_frame(&mut app, &ctx, vec![])).is_none());
    // A paste event is the paste chord; its text is on the clipboard service, as the web shell offers it there.
    let paste = |app: &mut VectorcraftApp, text: &str| {
        app.clipboard.offer_paste(vec![craft::platform::clipboard::Flavor::text(text)]);
        key_frame(app, &ctx, vec![egui::Event::Paste(text.into())]);
    };
    // Pasting our own SVG back uses the internal clipboard; foreign SVG replaces it.
    paste(&mut app, &svg);
    assert_eq!(count(&app), 2);
    let foreign = r##"<svg xmlns="http://www.w3.org/2000/svg"><circle cx="5" cy="5" r="5"/><circle cx="20" cy="5" r="5"/><circle cx="35" cy="5" r="5"/></svg>"##;
    paste(&mut app, foreign);
    assert_eq!(count(&app), 5);
    // Plain text is not art: nothing is pasted from it (the internal clipboard is reused).
    paste(&mut app, "hello");
    assert_eq!(count(&app), 8);
}

// ---- Plug-in and effect rows ---------------------------------------------------------------

/// The `(id, shortcut)` pairs `shortcuts.list` returns for `query`.
fn listed(app: &mut VectorcraftApp, query: &str) -> Vec<(String, Value)> {
    let list = app.run("shortcuts.list", json!({ "query": query })).unwrap();
    list.as_array().unwrap().iter().map(|e| (e["id"].as_str().unwrap_or_default().to_string(), e["shortcut"].clone())).collect()
}

/// #3296: an Effect menu item takes a shortcut of its own, and pressing it opens that effect's dialog.
#[test]
fn an_effect_menu_item_takes_a_shortcut_of_its_own() {
    let mut app = app_with_a_selected_rectangle();
    assert!(!listed(&mut app, "effect:distort.transform").is_empty(), "Transform is listed");
    bind(&mut app, "effect:distort.transform", "Cmd+Alt+Shift+F11");
    frame(&mut app, tap(Key::F11, cmd() | Modifiers::ALT | Modifiers::SHIFT));
    assert_eq!(open_effect(&app).as_deref(), Some("distort.transform"), "{:?}", dialog_kind(&app));
    assert_eq!(item_shortcut(&app, "effect.dialog", json!({"effect": "distort.transform"})).as_deref(), Some("Cmd+Alt+Shift+F11"));
}

/// A live-effect plug-in installed later takes a shortcut of its own, and the key applies the effect.
#[test]
fn a_live_effect_plugin_installed_later_takes_a_shortcut_of_its_own() {
    use vectorcraft_plugins::wat as tpl;
    let (plugin, effect, key) = ("test.ui.keyedfx", "plugin.test.ui.keyedfx", "effect:plugin.test.ui.keyedfx");
    let install = |params: &str| {
        let src = tpl::module(&tpl::manifest(plugin, "effect", params), tpl::ECHO, "", 1);
        vectorcraft_plugins::registry::install_bytes(&wat::parse_str(src).unwrap()).unwrap();
    };
    let mut app = app_with_a_selected_rectangle();
    let node = NodeId(ids(&app)[0]);
    let effects =
        |app: &VectorcraftApp| app.session.doc().unwrap().doc.node(node).unwrap().appearance.effects.iter().filter(|e| e.id == effect).count();
    let chord = "Cmd+Alt+Shift+F6";
    let f6 = || tap(Key::F6, cmd() | Modifiers::ALT | Modifiers::SHIFT);
    assert_eq!(listed(&mut app, effect), vec![]);
    install("{}");
    // The window's keymap takes the plug-in's command in the frame after the install.
    frame(&mut app, vec![]);
    app.run("shortcuts.set", json!({"id": key, "shortcut": chord})).unwrap();
    assert_eq!(listed(&mut app, effect), vec![(key.to_string(), json!(chord))]);
    frame(&mut app, f6());
    assert_eq!(effects(&app), 1, "the key applies the effect");
    assert_eq!(item_shortcut(&app, "effect.apply", json!({ "effect": effect })).as_deref(), Some(chord));
    // Removed: no row and no key; the saved shortcut is the effect's again when it comes back.
    vectorcraft_plugins::registry::remove(plugin);
    assert_eq!(listed(&mut app, effect), vec![]);
    install(r#"{"wiggle":{"type":"bool"}}"#);
    assert_eq!(listed(&mut app, effect), vec![(key.to_string(), json!(chord))]);
    frame(&mut app, f6());
    // The effect is applied already: its dialog first asks whether to edit it or add another.
    let asked = app.effect_dialogs.effect.as_ref().map(|o| o.cx.id.clone());
    assert!(app.any_dialog_open() && asked.as_deref() == Some(effect), "{asked:?}: {:?}", dialog_kind(&app));
    assert_eq!(item_shortcut(&app, "effect.dialog", json!({ "effect": effect })).as_deref(), Some(chord));
    vectorcraft_plugins::registry::remove(plugin);
}

/// Each filter plug-in's Object › Plug-ins item has a row of its own with no default shortcut; a key
/// given to one opens that filter's dialog and the other stays unbound.
#[test]
fn each_filter_plugin_takes_a_shortcut_of_its_own() {
    use vectorcraft_plugins::wat as tpl;
    let (one, two) = ("test.UI.key_filter-one", "test.UI.key_filter-two");
    let (key_one, key_two) = (format!("plugin:{one}"), format!("plugin:{two}"));
    let install = |id: &str, name: &str, params: &str| {
        let manifest = format!(r#"{{"id":"{id}","name":"{name}","version":"1","kind":"filter","params":{params}}}"#);
        vectorcraft_plugins::registry::install_bytes(&wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap()).unwrap();
    };
    let mut app = app_with_a_selected_rectangle();
    let chord = "Cmd+Alt+Shift+F7";
    let f7 = || tap(Key::F7, cmd() | Modifiers::ALT | Modifiers::SHIFT);
    install(one, "Same Name", "{}");
    install(two, "Same Name", r#"{"wiggle":{"type":"bool"}}"#);
    frame(&mut app, vec![]);
    assert_eq!(listed(&mut app, "test.ui.key_filter-"), vec![(key_one.clone(), Value::Null), (key_two.clone(), Value::Null)]);
    app.run("shortcuts.set", json!({"id": key_two, "shortcut": chord})).unwrap();
    assert_eq!(listed(&mut app, "test.ui.key_filter-"), vec![(key_one.clone(), Value::Null), (key_two.clone(), json!(chord))]);
    frame(&mut app, f7());
    assert!(plugin_open(&app, two), "the filter's dialog opens");
    assert_eq!(item_shortcut(&app, "plugin.dialog", json!({ "id": two })).as_deref(), Some(chord));
    assert_eq!(item_shortcut(&app, "plugin.dialog", json!({ "id": one })), None);
    close_top_dialog(&mut app, false);
    // Removed: no row and no key; back under another name and without parameters it runs at once.
    vectorcraft_plugins::registry::remove(two);
    assert_eq!(listed(&mut app, "test.ui.key_filter-"), vec![(key_one.clone(), Value::Null)]);
    install(two, "Renamed", "{}");
    assert_eq!(listed(&mut app, "test.ui.key_filter-"), vec![(key_one.clone(), Value::Null), (key_two.clone(), json!(chord))]);
    let undo = undo_len(&app);
    frame(&mut app, f7());
    assert!(dialog_kind(&app).is_none(), "no dialog: {:?}", dialog_kind(&app));
    assert_eq!(undo_len(&app), undo + 1, "the filter ran as one undo step");
    vectorcraft_plugins::registry::remove(one);
    vectorcraft_plugins::registry::remove(two);
}

/// A filter plug-in replaced under its id, or reinstalled by Reload Plug-ins, keeps its shortcut.
#[test]
fn a_filter_plugin_keeps_its_shortcut_when_replaced_or_reloaded() {
    use vectorcraft_plugins::wat as tpl;
    let id = "test.UI.replaced_filter";
    let key = format!("plugin:{id}");
    let wasm = |name: &str, params: &str| {
        let manifest = format!(r#"{{"id":"{id}","name":"{name}","version":"1","kind":"filter","params":{params}}}"#);
        wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap()
    };
    let with_params = r#"{"wiggle":{"type":"bool"}}"#;
    let chord = "Cmd+Alt+Shift+F6";
    let f6 = || tap(Key::F6, cmd() | Modifiers::ALT | Modifiers::SHIFT);
    let mut app = new_app();
    vectorcraft_plugins::registry::install_bytes(&wasm("Original", "{}")).unwrap();
    frame(&mut app, vec![]);
    app.run("shortcuts.set", json!({"id": key, "shortcut": chord})).unwrap();
    vectorcraft_plugins::registry::install_bytes(&wasm("Replaced", with_params)).unwrap();
    frame(&mut app, f6());
    assert!(plugin_open(&app, id), "the replaced filter has parameters");
    assert_eq!(item_shortcut(&app, "plugin.dialog", json!({ "id": id })).as_deref(), Some(chord));
    close_top_dialog(&mut app, false);
    let folder = std::env::temp_dir().join(format!("vectorcraft-ui-filter-shortcut-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("filter.wasm"), wasm("Reloaded", with_params)).unwrap();
    app.run("plugin.reload", json!({ "path": folder.display().to_string() })).unwrap();
    std::fs::remove_dir_all(&folder).ok();
    frame(&mut app, vec![]);
    assert_eq!(item_shortcut(&app, "plugin.dialog", json!({ "id": id })).as_deref(), Some(chord));
    let mut empty = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    ready(&mut empty);
    empty.run("shortcuts.set", json!({"id": key, "shortcut": chord})).unwrap();
    frame(&mut empty, f6());
    assert!(dialog_kind(&empty).is_none(), "no document: no dialog");
    vectorcraft_plugins::registry::remove(id);
}

/// The key of a heavy filter does what a click on its item does.
#[test]
fn a_heavy_filter_plugin_runs_from_its_shortcut_as_from_its_menu_item() {
    use vectorcraft_plugins::wat as tpl;
    let mut app = new_app();
    app.session.allow_plugin_grants_from_requests();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let (at_once, asks) = ("test.UI.heavy_filter-at-once", "test.UI.heavy_filter-asks");
    for (id, params, chord) in [(at_once, "{}", "Cmd+Alt+Shift+F7"), (asks, r#"{"wiggle":{"type":"bool"}}"#, "Cmd+Alt+Shift+F8")] {
        let manifest = format!(r#"{{"id":"{id}","name":"Heavy {id}","version":"1","kind":"filter","budget":"heavy","params":{params}}}"#);
        let wasm = wat::parse_str(tpl::spinning(&manifest, 1 << 40, r#"{"objects":[]}"#)).unwrap();
        app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "allowHeavy": true})).unwrap();
        frame(&mut app, vec![]);
        app.run("shortcuts.set", json!({"id": format!("plugin:{id}"), "shortcut": chord})).unwrap();
    }
    frame(&mut app, tap(Key::F7, cmd() | Modifiers::ALT | Modifiers::SHIFT));
    assert_eq!(app.plugin_runs.running.len(), 1, "started in the background");
    assert_eq!(app.ui.status, format!("Running “Heavy {at_once}”…"));
    frame(&mut app, tap(Key::F8, cmd() | Modifiers::ALT | Modifiers::SHIFT));
    assert!(plugin_open(&app, asks), "the heavy filter with parameters opens its dialog");
    assert!(dialog_field(&app, "preview").is_none(), "no Preview");
    close_top_dialog(&mut app, false);
    assert_eq!(app.run("plugin.cancelRuns", json!({})).unwrap()["cancelled"], 1);
    vectorcraft_plugins::registry::remove(at_once);
    vectorcraft_plugins::registry::remove(asks);
}

/// Keyboard Shortcuts translates a menu path one segment at a time and the label of a built-in
/// item. A filter plug-in's name stays as the plug-in gives it, even when it reads like a command.
#[test]
fn a_filter_plugin_row_translates_its_path_and_not_its_name() {
    let zh = <crate::i18n::Lang as crate::i18n::LangExt>::from_code("zh-hant").unwrap();
    crate::i18n::set_current(zh);
    let entry = |key: &str| crate::shortcut_editor::Entry {
        key: key.into(),
        label: "Group".into(),
        group: "Object › Plug-ins".into(),
        is_tool: false,
        shows_key: false,
    };
    let path = format!("{} › {}", crate::i18n::tr(zh, "Object"), crate::i18n::tr(zh, "Plug-ins"));
    let (filter, command) = (crate::shortcut_editor::shown_name(&entry("plugin:a.b")), crate::shortcut_editor::shown_name(&entry("object.group")));
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_ne!(path, "Object › Plug-ins");
    assert_eq!(filter, format!("{path} › Group"));
    assert_eq!(command, format!("{path} › {}", crate::i18n::tr(zh, "Group")));
}
