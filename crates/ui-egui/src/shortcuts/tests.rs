use super::*;

/// One headless frame delivering `events` to the shortcut handler.
fn frame(app: &mut VectorcraftApp, events: Vec<egui::Event>) -> egui::FullOutput {
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
        // The production path: `logic` takes this host iteration's input once through
        // `handle_input_once`, which runs the shortcut handler when the fonts are ready. A
        // direct call covers the passes before those fonts are installed; the frame key makes
        // it a no-op on the passes that already took this frame.
        app.logic(ui.ctx());
        handle(app, ui.ctx());
    });
    out.textures_delta.clear();
    out
}

#[test]
fn option_and_altgr_still_type_but_cmd_and_ctrl_chords_do_not() {
    use egui::Modifiers as M;
    assert!(is_shortcut_chord(M::COMMAND | M::SHIFT));
    assert!(is_shortcut_chord(M::CTRL));
    assert!(!is_shortcut_chord(M::NONE));
    assert!(!is_shortcut_chord(M::SHIFT));
    assert!(!is_shortcut_chord(M::ALT), "Option+key types é, ø on macOS");
    assert!(!is_shortcut_chord(M::CTRL | M::ALT), "AltGr types @ on a German keyboard");
}

/// A press of `key` with Ctrl and Alt held (as Firefox on Windows reports AltGr), followed by
/// the text it typed when `typed` is given.
fn ctrl_alt_press(key: Key, typed: Option<&str>) -> Vec<egui::Event> {
    let m = Modifiers::CTRL | Modifiers::ALT | Modifiers::COMMAND;
    let mut v = vec![egui::Event::ModifiersChanged(m), egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: m }];
    v.extend(typed.map(|t| egui::Event::Text(t.into())));
    v
}

/// The text of the type object `id`, and the document's layer count.
fn text_and_layers(app: &mut VectorcraftApp, id: u64) -> (String, usize) {
    let r = app.session.execute("text.getRange", &json!({"id": id})).unwrap();
    let text = r["runs"].as_array().unwrap().iter().filter_map(|run| run["text"].as_str()).collect();
    (text, app.session.active().unwrap().doc.layers.len())
}

/// While the Type tool edits text, a character typed with AltGr (Ctrl+Alt in Firefox on
/// Windows) is typed and runs no command: not New Layer Options… (Cmd+Alt+L), Reset Tracking
/// (Cmd+Alt+Q), Export for Screens (Cmd+Alt+E), Select All on Active Artboard (Cmd+Alt+A) or
/// Undo (Cmd+Z).
#[test]
fn altgr_characters_typed_in_text_run_no_command() {
    let mut wrong = vec![];
    for (key, typed) in [(Key::L, "ł"), (Key::Q, "@"), (Key::E, "€"), (Key::A, "ą"), (Key::Z, "ż")] {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
        app.session.execute("text.setRangeStyle", &json!({"id": id, "tracking": 200, "kerning": -40})).unwrap();
        app.select_tool("type");
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        app.session.set_tool_option("select", &json!({"start": 2, "end": 2}));
        assert!(app.session.tool_wants_text());
        let layers = text_and_layers(&mut app, id).1;
        let selected = |app: &VectorcraftApp| app.session.active().unwrap().selection.objects.len();
        let before = selected(&app);
        frame(&mut app, ctrl_alt_press(key, Some(typed)));
        let r = app.session.execute("text.getRange", &json!({"id": id})).unwrap();
        let spaced =
            r["runs"].as_array().unwrap().iter().all(|run| run["style"]["tracking"] == json!(200.0) && run["style"]["kerning"] == json!(-40.0));
        let got = (text_and_layers(&mut app, id), spaced, selected(&app) == before, app.ui.dialog.is_none(), app.session.tool_wants_text());
        if got != ((format!("Hi{typed}"), layers), true, true, true, true) {
            wrong.push(format!("{key:?}: (text, layers), tracking and kerning kept, selection kept, no dialog, still editing = {got:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// What egui reports for the command key: Cmd on a Mac, Ctrl elsewhere. The keymap reads a
/// Mac's Ctrl as a key of its own, so Ctrl+L there is not Cmd+L.
fn command_key() -> Modifiers {
    if crate::keymap::platform().is_mac() { Modifiers::MAC_CMD | Modifiers::COMMAND } else { Modifiers::CTRL | Modifiers::COMMAND }
}

/// Ctrl and Ctrl+Alt chords (Cmd and Cmd+Alt on a Mac) that type no text still run their
/// commands while the Type tool edits, and with the Selection tool a Ctrl+Alt press runs its
/// chord as before. Ctrl+L is New Layer, and Ctrl+Alt+L is New Layer Options…, which opens
/// Layer Options for a new layer.
#[test]
fn ctrl_alt_chords_without_typed_text_still_run() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    assert!(app.session.tool_wants_text());
    let layers = text_and_layers(&mut app, id).1;
    let new_layer_options =
        |app: &VectorcraftApp| app.ui.dialog.as_ref().is_some_and(|d| d.kind == "layerOptions" && d.fields.get("mode") == Some(&json!("new")));
    let m = command_key();
    frame(&mut app, press(Key::L, m));
    assert_eq!(text_and_layers(&mut app, id), ("Hi".into(), layers + 1), "Ctrl+L while editing: New Layer");
    assert!(app.ui.dialog.is_none(), "Ctrl+L opens no dialog");
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    assert!(app.session.tool_wants_text());
    frame(&mut app, press(Key::L, m | Modifiers::ALT));
    assert!(new_layer_options(&app), "Ctrl+Alt+L with no text typed while editing: New Layer Options…");
    assert_eq!(text_and_layers(&mut app, id), ("Hi".into(), layers + 1), "no layer is added until OK");
    crate::dialogs::cancel(&mut app);
    app.select_tool("selection");
    let mut typed = press(Key::L, m | Modifiers::ALT);
    typed.push(egui::Event::Text("ł".into()));
    frame(&mut app, typed);
    assert!(new_layer_options(&app), "Selection tool: Ctrl+Alt+L runs New Layer Options… as before");
    assert_eq!(text_and_layers(&mut app, id), ("Hi".into(), layers + 1), "no layer is added until OK");
}

/// Cmd as egui reports it on this platform: the Command key on macOS, Ctrl elsewhere.
fn cmd() -> Modifiers {
    if crate::keymap::platform().is_mac() { Modifiers::MAC_CMD | Modifiers::COMMAND } else { Modifiers::CTRL | Modifiers::COMMAND }
}

/// The web build keeps from the browser the presses the keymap binds: the View menu's zoom
/// chords, Undo and Redo, whatever the browser reports as the key typed and its position.
#[test]
fn presses_the_keymap_binds_run_a_shortcut() {
    crate::shortcut_editor::sync(&crate::state::UiState::default());
    let (cmd, shift, alt) = (cmd(), Modifiers::SHIFT, Modifiers::ALT);
    for (key, code, mods) in [
        ("=", "Equal", cmd),
        ("+", "Equal", cmd | shift),
        ("+", "NumpadAdd", cmd),
        ("-", "Minus", cmd),
        ("0", "Digit0", cmd),
        ("º", "Digit0", cmd | alt),
        ("z", "KeyZ", cmd),
        ("Z", "KeyZ", cmd | shift),
        ("[", "BracketLeft", cmd),
        // German's `+` key, where US has `]`: the character decides.
        ("+", "BracketRight", cmd),
    ] {
        assert!(runs_a_shortcut(key, code, mods), "{key} ({code}) {mods:?}");
    }
    // Keys the browser has no name egui knows for are matched by their position.
    assert!(runs_a_shortcut("Dead", "KeyZ", cmd), "a dead key at Z");
    for (key, code, mods) in [("Insert", "Numpad0", cmd), ("F13", "F13", cmd), ("Unidentified", "", cmd)] {
        assert!(!runs_a_shortcut(key, code, mods), "{key} ({code}) runs nothing");
    }
}

/// The user's shortcuts decide: a chord given to another command runs that one, and a chord
/// taken from its command runs nothing.
#[test]
fn presses_follow_the_users_shortcuts() {
    let mut ui = crate::state::UiState::default();
    crate::keymap::set_override(&mut ui.shortcut_overrides, "view.zoomIn", "");
    crate::keymap::set_override(&mut ui.shortcut_overrides, "view.grid", "Cmd+Alt+Shift+F9");
    crate::shortcut_editor::sync(&ui);
    let zoom_in = runs_a_shortcut("=", "Equal", cmd());
    let grid = runs_a_shortcut("F9", "F9", cmd() | Modifiers::ALT | Modifiers::SHIFT);
    crate::shortcut_editor::sync(&crate::state::UiState::default());
    assert!(!zoom_in, "Zoom In has no shortcut");
    assert!(grid, "Show Grid's own chord");
    assert!(runs_a_shortcut("=", "Equal", cmd()), "the defaults again");
}

/// #2094: Cmd+Shift+O is Type › Create Outlines (not File › Open's Cmd+O, not the Artboard
/// tool's Shift+O), with the Selection tool or while the Type tool edits the text.
#[test]
fn cmd_shift_o_creates_outlines() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    assert_eq!(crate::menus::shortcut_of("type.createOutlines"), Some("Cmd+Shift+O"));
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let chord = egui::Event::Key { key: Key::O, physical_key: None, pressed: true, repeat: false, modifiers: cmd_shift };
    let is_text = |app: &VectorcraftApp, id: u64| {
        matches!(app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).map(|n| &n.kind), Some(vectorcraft_doc::NodeKind::Text(_)))
    };
    let group_kids = |app: &VectorcraftApp| -> usize {
        let id = app.session.active().unwrap().selection.objects.first().copied().unwrap();
        match &app.session.doc().unwrap().doc.node(id).unwrap().kind {
            vectorcraft_doc::NodeKind::Group { children, .. } => children.len(),
            _ => 0,
        }
    };
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    assert!(is_text(&app, id));
    frame(&mut app, vec![chord.clone()]);
    assert!(!is_text(&app, id), "Selection tool: the chord outlines the text");
    assert_eq!(group_kids(&app), 2, "H and i");
    assert_ne!(app.session.tool_id(), "artboard", "not the Artboard tool's Shift+O");
    assert!(app.ui.dialog.is_none(), "not File › Open");
    // While the Type tool edits, the same chord outlines; a Text event with Cmd held must not type.
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 80, "text": "Yo"})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    assert!(app.session.tool_wants_text());
    frame(&mut app, vec![egui::Event::ModifiersChanged(cmd_shift), chord, egui::Event::Text("o".into())]);
    assert!(!is_text(&app, id), "Type tool: the chord outlines the text being edited");
    assert_eq!(group_kids(&app), 2, "Yo only: the stray Text with Cmd was not typed");
    assert!(app.ui.dialog.is_none(), "not File › Open while typing");
}

/// Cmd+Alt+Shift+3 is Object › Hide › Unselected Artwork: it hides the object that is not
/// selected. Cmd+Alt+3 still shows all, and Cmd+3 still hides the selection. In isolation mode
/// the item is disabled.
#[test]
fn cmd_alt_shift_3_hides_unselected_artwork() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    assert_eq!(crate::menus::shortcut_of("object.hide.unselected"), Some("Cmd+Alt+Shift+3"));
    let rect = |app: &mut VectorcraftApp, x: f64| {
        let r = app.session.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap();
        vectorcraft_doc::NodeId(r["id"].as_u64().unwrap())
    };
    let (a, b) = (rect(&mut app, 10.0), rect(&mut app, 50.0));
    let shown = |app: &VectorcraftApp| [a, b].map(|id| app.session.doc().unwrap().doc.node(id).unwrap().visible);
    let three = |modifiers| egui::Event::Key { key: Key::Num3, physical_key: None, pressed: true, repeat: false, modifiers };
    app.session.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    frame(&mut app, vec![three(Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT)]);
    assert_eq!(shown(&app), [true, false]);
    frame(&mut app, vec![three(Modifiers::COMMAND | Modifiers::ALT)]);
    assert_eq!(shown(&app), [true, true], "Cmd+Alt+3 shows all");
    app.session.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    frame(&mut app, vec![three(Modifiers::COMMAND)]);
    assert_eq!(shown(&app), [false, true], "Cmd+3 hides the selection");
    // In isolation mode the menu item is off, and the shortcut hides nothing.
    app.session.execute("object.showAll", &json!({})).unwrap();
    app.session.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let g = app.session.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("object.isolate", &json!({"id": g})).unwrap();
    app.session.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    assert!(!crate::menus::enabled(&app, "object.hide.unselected"));
    frame(&mut app, vec![three(Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT)]);
    assert_eq!(shown(&app), [true, true], "nothing is hidden in isolation mode");
}

/// Cmd+Alt+Shift+J opens Average & Join's dialog, Points: Corner the first time. Cancel changes
/// nothing; OK averages the selected lines' nearest ends and joins them there; the dialog opens
/// again on the choice confirmed last.
#[test]
fn cmd_alt_shift_j_opens_average_and_join() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    assert_eq!(crate::menus::shortcut_of("path.averageJoin"), Some("Cmd+Alt+Shift+J"));
    let line = |app: &mut VectorcraftApp, x1: f64, x2: f64| {
        app.session.execute("shape.line", &json!({"x1": x1, "y1": 10, "x2": x2, "y2": 10})).unwrap()["id"].as_u64().unwrap()
    };
    let (a, b) = (line(&mut app, 10.0, 50.0), line(&mut app, 60.0, 100.0));
    app.session.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    let j = || egui::Event::Key {
        key: Key::J,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT,
    };
    frame(&mut app, vec![j()]);
    let d = app.ui.dialog.as_ref().expect("the dialog opened");
    assert_eq!((d.kind.as_str(), d.str("to").as_str()), ("averageJoin", "corner"));
    crate::dialogs::cancel(&mut app);
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.doc().unwrap().doc, before, "Cancel changes nothing");
    frame(&mut app, vec![j()]);
    app.ui.dialog.as_mut().expect("the dialog opened again").fields.insert("to".into(), json!("smooth"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let doc = &app.session.doc().unwrap().doc;
    assert!(doc.node(vectorcraft_doc::NodeId(b)).is_none(), "joined into one path");
    let path = doc.node(vectorcraft_doc::NodeId(a)).unwrap().path_data().unwrap();
    let pts: Vec<(f64, f64)> = path.subpaths[0].anchors.iter().map(|x| (x.p.x, x.p.y)).collect();
    assert_eq!(pts, [(10.0, 10.0), (55.0, 10.0), (100.0, 10.0)]);
    frame(&mut app, vec![j()]);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.str("to")).as_deref(), Some("smooth"), "the choice confirmed last");
}

/// Cmd+Shift+B while the Type tool edits text is Type › Bold, not Hide Bounding Box (#724).
#[test]
fn cmd_shift_b_while_typing_is_bold_not_the_bounding_box() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    app.select_tool("type");
    let view = app.view_info();
    for kind in [vectorcraft_tools::PointerKind::Down, vectorcraft_tools::PointerKind::Up] {
        app.session.pointer(&vectorcraft_tools::PointerEvent::new(kind, 50.0, 50.0), view).unwrap();
    }
    frame(&mut app, vec![egui::Event::Text("Bold".into())]);
    assert!(app.session.tool_wants_text());
    let chord = egui::Event::Key { key: Key::B, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND | Modifiers::SHIFT };
    frame(&mut app, vec![chord.clone()]);
    assert!(app.ui.view.bounding_box, "the bounding box stays");
    // The chord reached Bold: the text is in a bold face, or its family has none and says so.
    let face = crate::panels::character::text_style(&app).map(|(s, _)| s.font_style).unwrap_or_default();
    assert!(face.contains("Bold") || app.ui.status.contains("has no Bold style"), "{face:?} {:?}", app.ui.status);
    // Out of the text, the chord hides the bounding box as before.
    app.select_tool("selection");
    frame(&mut app, vec![chord]);
    assert!(!app.ui.view.bounding_box);
}

/// Cmd+Shift+L, C, R, J and F align paragraphs (#1111): every paragraph of the selected type,
/// or while the Type tool edits text, those its selection touches.
#[test]
fn cmd_shift_letters_align_paragraphs() {
    use vectorcraft_doc::Justify;
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One\nTwo"})).unwrap()["id"].as_u64().unwrap();
    let chord = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND | Modifiers::SHIFT };
    let paras = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => (0..t.paragraph_count()).map(|i| t.para_at(i).justify).collect::<Vec<_>>(),
        _ => panic!("text"),
    };
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    for (key, want) in
        [(Key::C, Justify::Center), (Key::R, Justify::Right), (Key::J, Justify::JustifyLeft), (Key::F, Justify::JustifyAll), (Key::L, Justify::Left)]
    {
        frame(&mut app, vec![chord(key)]);
        assert_eq!(paras(&app), [want, want], "{key:?}");
    }
    // With the Type tool, only the paragraph its selection touches.
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 5, "end": 6}));
    assert!(app.session.tool_wants_text());
    frame(&mut app, vec![chord(Key::R)]);
    assert_eq!(paras(&app), [Justify::Left, Justify::Right]);
    // Nothing selected: nothing to align.
    app.select_tool("selection");
    app.session.execute("select.none", &json!({})).unwrap();
    assert!(!crate::menus::enabled(&app, "type.alignCenter"));
}

/// Cmd+Alt+Q resets the tracking and kerning of the selected type (tracking 0, kerning Auto),
/// and of the Type tool's selected text while it edits.
#[test]
fn cmd_alt_q_resets_tracking_and_kerning() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One Two"})).unwrap()["id"].as_u64().unwrap();
    let spaced = |app: &mut VectorcraftApp| app.session.execute("text.setRangeStyle", &json!({"id": id, "tracking": 200, "kerning": -40})).unwrap();
    let spacing = |app: &mut VectorcraftApp| {
        let r = app.session.execute("text.getRange", &json!({"id": id})).unwrap();
        r["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|run| (run["text"].clone(), run["style"]["tracking"].clone(), run["style"]["kerning"].clone()))
            .collect::<Vec<_>>()
    };
    let chord = egui::Event::Key { key: Key::Q, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND | Modifiers::ALT };
    spaced(&mut app);
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    frame(&mut app, vec![chord.clone()]);
    assert_eq!(spacing(&mut app), [(json!("One Two"), json!(0.0), json!(null))]);
    // While the Type tool edits: only its selection.
    spaced(&mut app);
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 4, "end": 7}));
    assert!(app.session.tool_wants_text());
    frame(&mut app, vec![chord]);
    assert_eq!(spacing(&mut app), [(json!("One "), json!(200.0), json!(-40.0)), (json!("Two"), json!(0.0), json!(null))]);
}

/// Cmd+Alt+K highlights the Character panel's Kerning field and Cmd+Alt+Shift+F highlights its
/// font name field for the selected type, and Cmd+Alt+K also does so while the Type tool edits
/// text. Cmd+Alt+K does not open Preferences (Cmd+K).
#[test]
fn cmd_alt_k_and_cmd_alt_shift_f_highlight_the_character_fields() {
    use crate::panels::character::Field;
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One Two"})).unwrap()["id"].as_u64().unwrap();
    let chord = |key, m| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: m };
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    frame(&mut app, vec![chord(Key::K, Modifiers::COMMAND | Modifiers::ALT)]);
    assert_eq!(app.character_focus, Some(Field::Kerning));
    assert!(app.ui.dialog.is_none(), "not Preferences");
    assert!(crate::panel_docking::showing(&app.ui, "character"));
    frame(&mut app, vec![chord(Key::F, Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT)]);
    assert_eq!(app.character_focus, Some(Field::FontName));
    // While the Type tool edits text.
    app.character_focus = None;
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 4, "end": 7}));
    assert!(app.session.tool_wants_text());
    frame(&mut app, vec![chord(Key::K, Modifiers::COMMAND | Modifiers::ALT)]);
    assert_eq!(app.character_focus, Some(Field::Kerning));
    assert!(app.ui.dialog.is_none(), "not Preferences");
    // Keyboard Shortcuts lists all three under Type; Highlight Tracking has no default key.
    for id in ["type.highlightFontName", "type.highlightKerning", "type.highlightTracking"] {
        assert_eq!(crate::shortcut_editor::entry(id).map(|e| e.group.as_str()), Some("Type"), "{id}");
    }
    assert_eq!(crate::menus::shortcut_of("type.highlightTracking"), None);
}

/// Cmd+Shift+X resets the horizontal scale of the selected type to 100%, and of the Type
/// tool's selected text while it edits. egui reports the chord as a Cut with Shift held, and
/// the type object is not cut.
#[test]
fn cmd_shift_x_resets_horizontal_scale() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One Two"})).unwrap()["id"].as_u64().unwrap();
    let stretched = |app: &mut VectorcraftApp| app.session.execute("text.setRangeStyle", &json!({"id": id, "hScale": 150})).unwrap();
    let scales = |app: &mut VectorcraftApp| {
        let r = app.session.execute("text.getRange", &json!({"id": id})).unwrap();
        r["runs"].as_array().unwrap().iter().map(|run| (run["text"].clone(), run["style"]["h_scale"].clone())).collect::<Vec<_>>()
    };
    let chord = vec![egui::Event::ModifiersChanged(Modifiers::COMMAND | Modifiers::SHIFT), egui::Event::Cut];
    stretched(&mut app);
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    frame(&mut app, chord.clone());
    assert_eq!(scales(&mut app), [(json!("One Two"), json!(100.0))]);
    // While the Type tool edits: only its selection, and the text stays.
    stretched(&mut app);
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 4, "end": 7}));
    assert!(app.session.tool_wants_text());
    frame(&mut app, chord);
    assert_eq!(scales(&mut app), [(json!("One "), json!(150.0)), (json!("Two"), json!(100.0))]);
}

/// Cmd+Shift+U underlines the selected type and removes the underline again, Cmd+Alt+Shift+C
/// switches its paragraphs between the Single-line and the Every-line Composer, and
/// Cmd+Alt+Shift+H turns their hyphenation on and off. While the Type tool edits text, they act on
/// its selection and the paragraphs it touches. Smart Guides (Cmd+U), Copy (Cmd+C) and Hide
/// Artboards (Cmd+Shift+H) stay as they are.
#[test]
fn cmd_shift_u_and_cmd_alt_shift_c_and_h_toggle_type_formats() {
    use vectorcraft_doc::Composer;
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One\nTwo"})).unwrap()["id"].as_u64().unwrap();
    let underlines = |app: &mut VectorcraftApp| {
        let r = app.session.execute("text.getRange", &json!({"id": id})).unwrap();
        r["runs"].as_array().unwrap().iter().map(|run| (run["text"].clone(), run["style"]["underline"].clone())).collect::<Vec<_>>()
    };
    let paras = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => (0..t.paragraph_count()).map(|i| (t.para_at(i).composer, t.para_at(i).hyphenate)).collect::<Vec<_>>(),
        _ => panic!("text"),
    };
    let chord =
        |key, m: Modifiers| vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND | m }];
    let (shift, alt_shift) = (Modifiers::SHIFT, Modifiers::ALT | Modifiers::SHIFT);
    let (guides, artboards) = (app.ui.view.smart_guides, app.ui.view.artboards);
    assert!(app.session.clipboard.is_empty());
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let hyphenate = paras(&app)[0].1;
    frame(&mut app, chord(Key::U, shift));
    assert_eq!(underlines(&mut app), [(json!("One\nTwo"), json!(true))]);
    frame(&mut app, chord(Key::U, shift));
    assert_eq!(underlines(&mut app), [(json!("One\nTwo"), json!(false))]);
    frame(&mut app, chord(Key::C, alt_shift));
    assert_eq!(paras(&app), [(Composer::SingleLine, hyphenate); 2]);
    frame(&mut app, chord(Key::C, alt_shift));
    assert_eq!(paras(&app), [(Composer::EveryLine, hyphenate); 2]);
    frame(&mut app, chord(Key::H, alt_shift));
    assert_eq!(paras(&app), [(Composer::EveryLine, !hyphenate); 2]);
    frame(&mut app, chord(Key::H, alt_shift));
    assert_eq!(paras(&app), [(Composer::EveryLine, hyphenate); 2]);
    assert_eq!((app.ui.view.smart_guides, app.ui.view.artboards, app.session.clipboard.is_empty()), (guides, artboards, true));
    // While the Type tool edits: its selected text, and the paragraph that text is in.
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 4, "end": 7}));
    assert!(app.session.tool_wants_text());
    for (key, m) in [(Key::U, shift), (Key::C, alt_shift), (Key::H, alt_shift)] {
        frame(&mut app, chord(key, m));
    }
    assert_eq!(underlines(&mut app), [(json!("One\n"), json!(false)), (json!("Two"), json!(true))]);
    assert_eq!(paras(&app), [(Composer::EveryLine, hyphenate), (Composer::SingleLine, !hyphenate)]);
    // Paragraphs with different composers and hyphenation take the Every-line Composer and
    // hyphenation, as the Hyphenate checkbox turns it on for them.
    app.select_tool("selection");
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    for (key, m) in [(Key::C, alt_shift), (Key::H, alt_shift)] {
        frame(&mut app, chord(key, m));
    }
    assert_eq!(paras(&app), [(Composer::EveryLine, true); 2]);
    // Nothing selected: nothing to change.
    app.session.execute("select.none", &json!({})).unwrap();
    for id in ["type.underline", "type.lineComposer", "type.hyphenate"] {
        assert!(!crate::menus::enabled(&app, id), "{id}");
    }
}

/// Tab shows and hides the panels, and in the Type tool it types a tab, without moving the
/// keyboard focus on to a field: the keys after it (a tool letter, the next letter typed) still
/// work.
#[test]
fn tab_leaves_the_keyboard_to_the_shortcuts_and_the_type_tool() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 850.0));
    let run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
    };
    let press = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    for _ in 0..3 {
        run(&mut app, vec![]);
    }
    assert!(app.ui.dock && app.ui.toolbar);
    run(&mut app, vec![press(Key::Tab)]);
    run(&mut app, vec![]);
    assert!(!app.ui.dock && !app.ui.toolbar, "Tab hid the panels");
    run(&mut app, vec![press(Key::Tab)]);
    run(&mut app, vec![]);
    assert!(app.ui.dock && app.ui.toolbar, "a second Tab shows them again");
    run(&mut app, vec![press(Key::P), egui::Event::Text("p".into())]);
    assert_eq!(app.session.tool_id(), "pen", "and a tool letter after it works");
    // The Type tool: a, Tab, b is one text.
    app.select_tool("type");
    let view = app.view_info();
    for kind in [vectorcraft_tools::PointerKind::Down, vectorcraft_tools::PointerKind::Up] {
        app.session.pointer(&vectorcraft_tools::PointerEvent::new(kind, 50.0, 50.0), view).unwrap();
    }
    run(&mut app, vec![egui::Event::Text("a".into())]);
    run(&mut app, vec![press(Key::Tab)]);
    run(&mut app, vec![egui::Event::Text("b".into())]);
    let doc = app.session.execute("document.inspect", &json!({})).unwrap();
    assert_eq!(doc["layers"][0]["children"][0]["name"], "a\tb");
}

/// egui counts frames per viewport: two viewports of one context can both be on frame 0. Each
/// one must take its shortcut, not lose the second to a dedupe key that omits the viewport.
#[test]
fn two_viewports_on_the_same_frame_number_each_take_their_shortcut() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let aux = egui::ViewportId::from_hash_of("vectorcraft-shortcuts-aux");
    let tab = egui::Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    let mut frames = vec![];
    // The auxiliary viewport goes first so the root one has not advanced past frame 0.
    for viewport in [aux, egui::ViewportId::ROOT] {
        let mut raw = egui::RawInput { events: vec![tab.clone()], screen_rect: Some(screen), viewport_id: viewport, ..Default::default() };
        raw.viewports.entry(viewport).or_default();
        let mut out = ctx.run_ui(raw, |ui| {
            frames.push(ui.ctx().cumulative_frame_nr_for(viewport));
            handle(&mut app, ui.ctx());
        });
        out.textures_delta.clear();
    }
    assert_eq!(frames, [0, 0], "both viewports are on frame 0");
    assert!(app.ui.dock, "two Tabs in two viewports toggled the panels twice (the second was not a duplicate)");
}

/// Shift+Tab shows and hides the panels and leaves the toolbar and the Control bar as they are,
/// while Tab hides all three. Hiding the panels closes a panel popped out of the icon column.
/// In a focused field neither key shows or hides anything.
#[test]
fn shift_tab_hides_the_panels_but_not_the_toolbar_or_the_control_bar() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 850.0));
    let run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
    };
    let press = |modifiers| egui::Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers };
    let shown = |app: &VectorcraftApp| (app.ui.dock, app.ui.toolbar, app.ui.control_bar);
    app.ui.control_bar = true;
    for _ in 0..3 {
        run(&mut app, vec![]);
    }
    assert_eq!(shown(&app), (true, true, true));
    app.run("window.panel", json!({"panel": "color"})).unwrap();
    run(&mut app, vec![press(Modifiers::SHIFT)]);
    run(&mut app, vec![]);
    assert_eq!(shown(&app), (false, true, true), "Shift+Tab hid the panels only");
    assert_eq!(app.ui.open_panel, None, "and the panel popped out of the icon column");
    run(&mut app, vec![press(Modifiers::SHIFT)]);
    run(&mut app, vec![]);
    assert_eq!(shown(&app), (true, true, true), "a second Shift+Tab shows them again");
    app.run("window.panel", json!({"panel": "color"})).unwrap();
    run(&mut app, vec![press(Modifiers::NONE)]);
    run(&mut app, vec![]);
    assert_eq!(shown(&app), (false, false, false), "Tab hides the toolbar and the Control bar too");
    assert_eq!(app.ui.open_panel, None);
    run(&mut app, vec![press(Modifiers::NONE)]);
    run(&mut app, vec![]);
    assert_eq!(shown(&app), (true, true, true));
    // A field of the Color panel has the keyboard focus: Shift+Tab and Tab do not show or hide anything.
    app.run("window.panel", json!({"panel": "color"})).unwrap();
    for _ in 0..3 {
        run(&mut app, vec![]);
    }
    let fields: Vec<(egui::Id, egui::Rect)> = ctx.viewport(|vp| {
        vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.sense.senses_click()).map(|w| (w.id, w.rect)).collect()
    });
    let (id, rect) =
        fields.into_iter().find(|&(id, _)| egui::TextEdit::load_state(&ctx, id).is_some()).unwrap_or_else(|| panic!("no field on screen"));
    let click =
        |pressed| egui::Event::PointerButton { pos: rect.center(), button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    run(&mut app, vec![egui::Event::PointerMoved(rect.center()), click(true)]);
    run(&mut app, vec![click(false)]);
    assert!(ctx.memory(|m| m.has_focus(id)), "the field has the keyboard");
    for modifiers in [Modifiers::SHIFT, Modifiers::NONE] {
        run(&mut app, vec![press(modifiers)]);
        run(&mut app, vec![]);
        assert_eq!(shown(&app), (true, true, true), "{modifiers:?}");
        assert_eq!(app.ui.open_panel.as_deref(), Some("color"), "{modifiers:?}");
    }
}

/// The second Tab shows again only the bars the first Tab hid: a Control bar that was off stays
/// off, and a toolbar hidden with Window › Tools stays hidden, after a restart too. Shift+Tab
/// between two Tabs, and Tab between two Shift+Tabs, leave a state the same key goes back from.
/// After a switch of workspace, Tab hides and shows that workspace's bars.
#[test]
fn tab_shows_again_only_the_bars_it_hid() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 850.0));
    let run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
    };
    let press = |modifiers| egui::Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers };
    let tab = |app: &mut VectorcraftApp, modifiers| {
        run(app, vec![press(modifiers)]);
        run(app, vec![]);
        (app.ui.dock, app.ui.toolbar, app.ui.control_bar)
    };
    for _ in 0..3 {
        run(&mut app, vec![]);
    }
    assert_eq!((app.ui.dock, app.ui.toolbar, app.ui.control_bar), (true, true, false), "the Control bar starts off");
    assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
    assert_eq!(tab(&mut app, Modifiers::NONE), (true, true, false), "the second Tab leaves the Control bar off");
    app.run("window.toolbar", json!({})).unwrap();
    assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
    assert_eq!(tab(&mut app, Modifiers::NONE), (true, false, false), "a toolbar hidden with Window › Tools stays hidden");
    app.run("window.toolbar", json!({})).unwrap();
    // Hidden by Tab when the app quits: the next Tab after a restart shows the same bars.
    assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
    app.ui = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
    assert_eq!(tab(&mut app, Modifiers::NONE), (true, true, false), "after a restart");
    // Shift+Tab after Tab shows the panels, and a second Shift+Tab hides them again; Tab then
    // shows what the first Tab hid.
    assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
    assert_eq!(tab(&mut app, Modifiers::SHIFT), (true, false, false));
    assert_eq!(tab(&mut app, Modifiers::SHIFT), (false, false, false));
    assert_eq!(tab(&mut app, Modifiers::SHIFT), (true, false, false));
    assert_eq!(tab(&mut app, Modifiers::NONE), (true, true, false), "Tab after Shift+Tab");
    // Tab after Shift+Tab hides the toolbar, and a second Tab goes back to Shift+Tab's state.
    assert_eq!(tab(&mut app, Modifiers::SHIFT), (false, true, false));
    assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
    assert_eq!(tab(&mut app, Modifiers::NONE), (false, true, false), "Shift+Tab's state");
    // Panels shown with Shift+Tab between two Tabs stay shown.
    assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
    assert_eq!(tab(&mut app, Modifiers::SHIFT), (true, false, false));
    assert_eq!(tab(&mut app, Modifiers::NONE), (true, true, false), "the panels Shift+Tab showed");
    // A bar shown another way after Tab: the next Tab hides it, and the Tab after that shows it again.
    assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
    app.run("window.control", json!({})).unwrap();
    assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
    assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, true), "only the Control bar");
    // Nothing shows and Tab hid nothing: Tab shows all three.
    app.run("window.control", json!({})).unwrap();
    assert_eq!((app.ui.dock, app.ui.toolbar, app.ui.control_bar), (false, false, false));
    assert_eq!(tab(&mut app, Modifiers::NONE), (true, true, true));
    // After a switch of workspace, Tab hides that workspace's bars and shows them again.
    app.run("window.toolbar", json!({})).unwrap();
    app.run("window.control", json!({})).unwrap();
    app.run("window.workspace.new", json!({"name": "Panels Only"})).unwrap();
    app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
    assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false));
    app.run("window.workspace", json!({"name": "Panels Only"})).unwrap();
    assert_eq!((app.ui.dock, app.ui.toolbar, app.ui.control_bar), (true, false, false));
    assert_eq!(tab(&mut app, Modifiers::NONE), (false, false, false), "Tab after a switch of workspace");
    assert_eq!(tab(&mut app, Modifiers::NONE), (true, false, false));
}

#[test]
fn copy_and_paste_events_use_the_system_clipboard() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].clone();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    // Copy publishes SVG to the system clipboard (egui's CopyText output command).
    let out = frame(&mut app, vec![egui::Event::Copy]);
    let copied = out.platform_output.commands.iter().find_map(|c| match c {
        egui::OutputCommand::CopyText(t) => Some(t.clone()),
        _ => None,
    });
    let svg = copied.expect("copy publishes SVG");
    assert!(svg.contains("<svg"));
    // The copy is handed to egui once, in the pass that took it: no second command follows.
    let again = frame(&mut app, vec![]);
    assert!(!again.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_))));
    // Pasting our own SVG back uses the internal clipboard; foreign SVG replaces it.
    frame(&mut app, vec![egui::Event::Paste(svg)]);
    assert_eq!(app.session.doc().unwrap().doc.layers[0].children().unwrap().len(), 2);
    let foreign = r##"<svg xmlns="http://www.w3.org/2000/svg"><circle cx="5" cy="5" r="5"/><circle cx="20" cy="5" r="5"/><circle cx="35" cy="5" r="5"/></svg>"##;
    frame(&mut app, vec![egui::Event::Paste(foreign.into())]);
    assert_eq!(app.session.doc().unwrap().doc.layers[0].children().unwrap().len(), 5);
    // Plain text is not art: nothing is pasted from it (the internal clipboard is reused).
    frame(&mut app, vec![egui::Event::Paste("hello".into())]);
    assert_eq!(app.session.doc().unwrap().doc.layers[0].children().unwrap().len(), 8);
}

/// While a grid is dragged out, ↑/↓ change its rows and ←/→ its columns.
#[test]
fn arrows_while_dragging_a_grid_change_its_rows_and_columns() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("rectangularGrid");
    let view = app.view_info();
    // Drag a grid out, pressing `keys` on the way: its (horizontal, vertical) dividers.
    let grid = |app: &mut VectorcraftApp, keys: &[Key]| {
        app.session.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 80.0), view).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Drag, 300.0, 220.0), view).unwrap();
        for &key in keys {
            let press = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
            frame(app, vec![press]);
        }
        app.session.pointer(&PointerEvent::new(PointerKind::Up, 300.0, 220.0), view).unwrap();
        let d = &app.session.doc().unwrap().doc;
        let g = d.layers[0].children().unwrap().last().unwrap().clone();
        let vectorcraft_doc::NodeKind::Group { children, .. } = &g.kind else { panic!("not a grid: {:?}", g.kind) };
        let bounds: Vec<_> = children.iter().filter_map(|c| c.geometric_bounds()).collect();
        (bounds.iter().filter(|b| b.height() < 0.5).count(), bounds.iter().filter(|b| b.width() < 0.5).count())
    };
    let (rows, columns) = grid(&mut app, &[]);
    assert_eq!(grid(&mut app, &[Key::ArrowRight, Key::ArrowRight, Key::ArrowUp]), (rows + 1, columns + 2));
    assert_eq!(grid(&mut app, &[Key::ArrowLeft, Key::ArrowDown]), (rows, columns + 1));
}

/// A press of `key` with `m` held.
fn press(key: Key, m: Modifiers) -> Vec<egui::Event> {
    vec![egui::Event::ModifiersChanged(m), egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: m }]
}

/// Binds Show Grid to `chord`, or with `None` leaves every command its default shortcut.
fn bind_grid(app: &mut VectorcraftApp, chord: Option<&str>) {
    app.ui.shortcut_overrides.clear();
    if let Some(c) = chord {
        app.ui.shortcut_overrides.insert("view.grid", vec![c.to_string()]);
    }
    crate::shortcut_editor::sync(&app.ui);
}

/// Clicks the active tool at (`x`, 200) and returns how many objects the first layer holds.
fn click_at(app: &mut VectorcraftApp, x: f64) -> usize {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let view = app.view_info();
    for kind in [PointerKind::Down, PointerKind::Up] {
        app.session.pointer(&PointerEvent::new(kind, x, 200.0), view).unwrap();
    }
    app.session.doc().unwrap().doc.layers[0].children().unwrap().len()
}

/// A command bound to Cmd+Enter runs without the active tool's Enter: the Selection tool opens
/// no Move dialog, the Type tool types no line break, and the Pen tool's path stays open. With
/// nothing bound to it, Cmd+Enter goes to the tool as Enter does.
#[test]
fn a_command_on_cmd_enter_runs_without_the_tools_enter() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 30})).unwrap()["id"].clone();
    let kind = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| d.kind.clone());
    let grid = app.ui.view.grid;
    app.select_tool("selection");
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    frame(&mut app, press(Key::Enter, Modifiers::COMMAND));
    assert_eq!((kind(&app).as_deref(), app.ui.view.grid), (Some("move"), grid), "nothing bound: Cmd+Enter opens Move as Enter does");
    app.ui.dialog = None;
    bind_grid(&mut app, Some("Cmd+Enter"));
    frame(&mut app, press(Key::Enter, Modifiers::COMMAND));
    assert_eq!((kind(&app), app.ui.view.grid), (None, !grid), "Cmd+Enter runs Show Grid only");
    frame(&mut app, press(Key::Enter, Modifiers::NONE));
    assert_eq!((kind(&app).as_deref(), app.ui.view.grid), (Some("move"), !grid), "Enter still opens Move");
    app.ui.dialog = None;
    let text = app.session.execute("text.create", &json!({"x": 20, "y": 250, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    assert!(app.session.tool_wants_text());
    frame(&mut app, press(Key::Enter, Modifiers::COMMAND));
    let typed = text_and_layers(&mut app, text).0;
    assert_eq!((typed.as_str(), app.ui.view.grid), ("Hi", grid), "Cmd+Enter runs Show Grid and types no line break");
    app.session.execute("select.none", &json!({})).unwrap();
    app.select_tool("pen");
    click_at(&mut app, 100.0);
    let objects = click_at(&mut app, 150.0);
    frame(&mut app, press(Key::Enter, Modifiers::COMMAND));
    assert_eq!((click_at(&mut app, 200.0), app.ui.view.grid), (objects, !grid), "Cmd+Enter runs Show Grid and the path stays open");
    bind_grid(&mut app, None);
    frame(&mut app, press(Key::Enter, Modifiers::COMMAND));
    assert_eq!(click_at(&mut app, 250.0), objects + 1, "nothing bound: Cmd+Enter ends the path as Enter does");
}

/// A command bound to Cmd+Escape runs without the active tool's Escape: the Pen tool's path
/// stays open, isolation mode stays on, and the Type tool goes on editing its text. With nothing
/// bound to it, Cmd+Escape goes to the tool as Escape does. Windows keeps Cmd+Escape (Ctrl+Esc)
/// for the Start menu, so no command can be bound to it there and it goes to the tool.
#[test]
fn a_command_on_cmd_escape_runs_without_the_tools_escape() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 30})).unwrap()["id"].clone();
    let grid = app.ui.view.grid;
    let cmd_escape = || press(Key::Escape, Modifiers::COMMAND);
    let bindable = crate::keymap::platform() != craft_keymap::Platform::Windows;
    bind_grid(&mut app, Some("Cmd+Escape"));
    app.select_tool("pen");
    click_at(&mut app, 100.0);
    let objects = click_at(&mut app, 150.0);
    frame(&mut app, cmd_escape());
    if bindable {
        assert_eq!((click_at(&mut app, 200.0), app.ui.view.grid), (objects, !grid), "Cmd+Escape runs Show Grid and the path stays open");
    } else {
        assert_eq!((click_at(&mut app, 200.0), app.ui.view.grid), (objects + 1, grid), "Windows: Cmd+Escape stays unbound and ends the path");
    }
    let text = app.session.execute("text.create", &json!({"x": 20, "y": 250, "text": "Hi"})).unwrap()["id"].clone();
    let edit = |app: &mut VectorcraftApp| {
        app.session.execute("select.set", &json!({"ids": [text]})).unwrap();
        app.select_tool("type");
        app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
        assert!(app.session.tool_wants_text());
    };
    if bindable {
        edit(&mut app);
        frame(&mut app, cmd_escape());
        assert_eq!((app.session.tool_wants_text(), app.ui.view.grid), (true, grid), "Cmd+Escape runs Show Grid and the Type tool goes on editing");
    }
    app.select_tool("selection");
    app.session.execute("object.isolate", &json!({"id": id})).unwrap();
    let isolated = |app: &VectorcraftApp| app.session.active().unwrap().isolation.is_some();
    if bindable {
        frame(&mut app, cmd_escape());
        assert_eq!((isolated(&app), app.ui.view.grid), (true, !grid), "Cmd+Escape runs Show Grid and isolation mode stays on");
    }
    bind_grid(&mut app, None);
    frame(&mut app, cmd_escape());
    assert!(!isolated(&app), "nothing bound: Cmd+Escape leaves isolation mode as Escape does");
    edit(&mut app);
    frame(&mut app, cmd_escape());
    assert!(!app.session.tool_wants_text(), "nothing bound: Cmd+Escape ends the editing as Escape does");
    app.session.execute("select.none", &json!({})).unwrap();
    app.select_tool("pen");
    let objects = click_at(&mut app, 300.0);
    frame(&mut app, cmd_escape());
    assert_eq!(click_at(&mut app, 350.0), objects + 1, "nothing bound: Cmd+Escape ends the path as Escape does");
}

/// Escape while an opacity mask is edited leaves mask editing, as Stop Editing Opacity Mask does:
/// the editing layer is removed, nothing is isolated, and the masked object is selected.
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
    assert_eq!(st.selection.objects, vec![vectorcraft_doc::NodeId(obj)]);
}

/// Enter opens the tool's dialog: the Move dialog for the selection tools, the tool's own for
/// Rotate, Scale, Reflect and Shear. The Enter that opens it isn't also its OK.
#[test]
fn enter_opens_the_tools_dialog_and_leaves_it_open() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    let enter = || egui::Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    // A whole app frame: the shortcuts first, then the UI, where a dialog takes its Enter.
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0)));
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect, ..Default::default() }, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
    };
    let kind = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| d.kind.clone());
    // The undo steps and the journal entries (the dialog's OK runs its command, which the journal records).
    let steps = |app: &VectorcraftApp| (app.session.active().unwrap().history.undo.len(), app.session.journal.len());
    app.select_tool("selection");
    frame(&mut app, vec![enter()]);
    assert_eq!(kind(&app), None, "nothing is selected");
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 30})).unwrap();
    for (tool, dialog) in [
        ("selection", "move"),
        ("directSelection", "move"),
        ("groupSelection", "move"),
        ("rotate", "rotate"),
        ("scale", "scale"),
        ("reflect", "reflect"),
        ("shear", "shear"),
    ] {
        app.select_tool(tool);
        let before = steps(&app);
        frame(&mut app, vec![enter()]);
        assert_eq!(kind(&app).as_deref(), Some(dialog), "{tool}: Enter opens it");
        frame(&mut app, vec![]);
        assert_eq!(kind(&app).as_deref(), Some(dialog), "{tool}: and it stays open");
        assert_eq!(steps(&app), before, "{tool}: the Enter that opened it didn't confirm it");
        // The next Enter is its OK. With its default values only Reflect changes the art, so only
        // Reflect records an undo step.
        frame(&mut app, vec![enter()]);
        let after = (before.0 + usize::from(dialog == "reflect"), before.1 + 1);
        assert_eq!((kind(&app), steps(&app)), (None, after), "{tool}: Enter confirms");
    }
    app.select_tool("rectangle");
    frame(&mut app, vec![enter()]);
    assert_eq!(kind(&app), None, "another tool has its own Enter");
}

/// A drag with the Selection tool, then Enter: the Move dialog opens on the drag's offset, and
/// Enter is its OK. With Copy ticked once, each Enter, Enter after that makes another copy at
/// the same spacing.
#[test]
fn enter_after_a_drag_repeats_its_offset() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 800, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let ctx = egui::Context::default();
    let enter = || egui::Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0)));
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect, ..Default::default() }, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
    };
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
    // Keys reach the canvas from the second frame on.
    frame(&mut app, vec![]);
    frame(&mut app, vec![enter()]);
    let d = app.ui.dialog.as_mut().expect("Enter opens Move");
    assert_eq!((d.kind.as_str(), d.f64("dx", f64::NAN).round(), d.f64("dy", f64::NAN)), ("move", step, 0.0));
    d.fields.insert("copy".into(), json!(true));
    frame(&mut app, vec![enter()]);
    for _ in 0..2 {
        frame(&mut app, vec![enter()]);
        frame(&mut app, vec![enter()]);
    }
    assert!(app.ui.dialog.is_none());
    assert_eq!(lefts(&app), (0..4).map(|i| moved + step * f64::from(i)).collect::<Vec<_>>());
    // With Preview on too, each Enter, Enter still makes one copy, and one undo step.
    frame(&mut app, vec![enter()]);
    app.ui.dialog.as_mut().expect("Move").fields.insert("preview".into(), json!(true));
    frame(&mut app, vec![enter()]);
    let steps = app.session.active().unwrap().history.undo.len();
    for _ in 0..2 {
        frame(&mut app, vec![enter()]);
        assert!(app.ui.dialog.as_ref().is_some_and(|d| d.bool("preview") && d.bool("copy")) && app.session.in_interaction());
        frame(&mut app, vec![enter()]);
        assert!(app.ui.dialog.is_none() && !app.session.in_interaction());
    }
    assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 2);
    assert_eq!(lefts(&app), (0..7).map(|i| moved + step * f64::from(i)).collect::<Vec<_>>());
}

/// #1661: typed `[` and `]` reach the tool that takes them: the Blob Brush's and the Eraser's
/// size (Option or AltGr brackets too), the stroke weight left as it is; other tools leave them
/// alone.
#[test]
fn brackets_size_the_blob_brush_and_the_eraser() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("stroke.set", &json!({"weight": 3})).unwrap();
    let size = |app: &mut VectorcraftApp| app.session.tool_options()["size"].as_f64().unwrap();
    let text = |t: &str| egui::Event::Text(t.into());
    for tool in ["blobBrush", "eraser"] {
        app.select_tool(tool);
        let before = size(&mut app);
        frame(&mut app, vec![text("]")]);
        frame(&mut app, vec![text("]")]);
        assert_eq!(size(&mut app), before + 2.0, "{tool}");
        // Option-8 types `[` on a Turkish Mac keyboard.
        let option_8 = egui::Event::Key { key: Key::Num8, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::ALT };
        frame(&mut app, vec![option_8, text("[")]);
        assert_eq!(size(&mut app), before + 1.0, "{tool}: a bracket typed with Option");
    }
    assert_eq!(app.session.paint.stroke_width, 3.0, "the stroke weight stays");
    app.select_tool("rectangle");
    frame(&mut app, vec![text("[")]);
    assert_eq!(app.session.paint.stroke_width, 3.0);
}

#[test]
fn comma_and_period_reapply_the_last_colour_and_gradient() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    app.session.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
    app.session.execute("paint.setFill", &json!({"color": "#336699"})).unwrap();
    let fill = |app: &VectorcraftApp| crate::panels::current_paints(app).0;
    frame(&mut app, vec![egui::Event::Text(".".into())]);
    assert!(matches!(fill(&app), vectorcraft_color::Paint::Gradient(_)), "`.` applies the last gradient");
    frame(&mut app, vec![egui::Event::Text(",".into())]);
    assert_eq!(fill(&app).color().unwrap().to_hex(), "#336699", "`,` applies the last colour");
}

#[test]
fn shifted_punctuation_shortcuts_work_as_typed() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let a = app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].clone();
    let b = app.session.execute("shape.ellipse", &json!({"x": 20, "y": 20, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
    let order = |app: &VectorcraftApp| -> Vec<u64> { app.session.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id.0).collect() };
    let (a, b) = (a.as_u64().unwrap(), b.as_u64().unwrap());
    assert_eq!(order(&app), [a, b]);
    let press = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    // Recorded in the Keyboard Shortcuts editor, they read as the keys (and so conflict).
    assert_eq!(crate::shortcut_editor::chord_from_event(Key::CloseCurlyBracket, cmd_shift).as_deref(), Some("Cmd+Shift+]"));
    assert_eq!(crate::shortcut_editor::chord_from_event(Key::Questionmark, cmd_shift).as_deref(), Some("Cmd+Shift+/"));
    // Cmd+Shift+] arrives as Cmd+Shift+`}`, Cmd+Shift+[ as Cmd+Shift+`{`.
    frame(&mut app, vec![press(Key::CloseCurlyBracket, cmd_shift)]);
    assert_eq!(order(&app), [b, a], "Bring to Front");
    frame(&mut app, vec![press(Key::OpenCurlyBracket, cmd_shift)]);
    assert_eq!(order(&app), [a, b], "Send to Back");
    // Cmd+Shift+/ arrives as Cmd+Shift+`?`: Search Commands.
    frame(&mut app, vec![press(Key::Questionmark, cmd_shift)]);
    assert!(app.ui.palette_open, "Search Commands");
    app.ui.palette_open = false;
    // The Curvature tool's Shift+~ arrives as the text `~` with Shift.
    frame(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::SHIFT), egui::Event::Text("~".into())]);
    assert_eq!(app.session.tool_id(), "curvature");
}

/// Cmd/Ctrl+Alt+Shift+. and , step the font size by five Size/Leading increments, and
/// Cmd/Ctrl+Shift+. still steps it by one: the chord with Alt does not reach the one-step
/// command.
#[test]
fn cmd_alt_shift_period_and_comma_step_the_font_size_by_five() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One Two", "size": 12})).unwrap()["id"].as_u64().unwrap();
    let sizes = |app: &mut VectorcraftApp| {
        let r = app.session.execute("text.getRange", &json!({"id": id})).unwrap();
        r["runs"].as_array().unwrap().iter().map(|run| (run["text"].clone(), run["style"]["size"].clone())).collect::<Vec<_>>()
    };
    let press = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    let five = Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT;
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    frame(&mut app, vec![press(Key::Period, five)]);
    assert_eq!(sizes(&mut app), [(json!("One Two"), json!(22.0))]);
    frame(&mut app, vec![press(Key::Period, Modifiers::COMMAND | Modifiers::SHIFT)]);
    assert_eq!(sizes(&mut app), [(json!("One Two"), json!(24.0))], "one step");
    frame(&mut app, vec![press(Key::Comma, five)]);
    assert_eq!(sizes(&mut app), [(json!("One Two"), json!(14.0))]);
    // While the Type tool edits: only its selection.
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 4, "end": 7}));
    assert!(app.session.tool_wants_text());
    frame(&mut app, vec![press(Key::Period, five)]);
    assert_eq!(sizes(&mut app), [(json!("One "), json!(14.0)), (json!("Two"), json!(24.0))]);
}

/// #3786: Alt+arrows step a selected text object's spacing instead of copying and nudging it.
#[test]
fn alt_arrows_step_selected_type_object() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 20, "y": 50, "text": "One Two", "size": 20})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("selection");
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();

    let press = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    frame(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::ALT), press(Key::ArrowRight, Modifiers::ALT)]);
    let r = app.session.execute("text.getRange", &json!({"id": id})).unwrap();
    assert_eq!(r["runs"][0]["style"]["tracking"], json!(20.0));
    assert_eq!(app.session.active().unwrap().selection.objects, [vectorcraft_doc::NodeId(id)]);

    frame(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::ALT), press(Key::ArrowDown, Modifiers::ALT)]);
    let r = app.session.execute("text.getRange", &json!({"id": id})).unwrap();
    assert_eq!(r["runs"][0]["style"]["leading"], json!(26.0));

    let alt_shift = Modifiers::ALT | Modifiers::SHIFT;
    frame(&mut app, vec![egui::Event::ModifiersChanged(alt_shift), press(Key::ArrowUp, alt_shift)]);
    let r = app.session.execute("text.getRange", &json!({"id": id})).unwrap();
    assert_eq!(r["runs"][0]["style"]["baseline_shift"], json!(2.0));

    // Shift+Alt+Left/Right have no text-spacing action; they must still be consumed instead of
    // falling through to Alt+arrow's copy-and-nudge behaviour.
    for key in [Key::ArrowLeft, Key::ArrowRight] {
        frame(&mut app, vec![egui::Event::ModifiersChanged(alt_shift), press(key, alt_shift)]);
        assert_eq!(app.session.active().unwrap().selection.objects, [vectorcraft_doc::NodeId(id)]);
    }

    // Other objects keep Alt+arrow's copy-and-nudge shortcut.
    let rect = app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [rect]})).unwrap();
    frame(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::ALT), press(Key::ArrowRight, Modifiers::ALT)]);
    let selected = &app.session.active().unwrap().selection.objects;
    assert_eq!(selected.len(), 1);
    assert_ne!(selected[0], vectorcraft_doc::NodeId(rect), "a non-type object is still copied by Alt+arrow");
}

/// #793: with a layout without Latin letters the one-key shortcuts go by the key's position:
/// the P key typing Persian `ح` chooses the Pen, the V key typing Russian `м` the Selection
/// tool. A Latin layout keeps the letter typed: AZERTY's A (the Q key's place) is A.
#[test]
fn one_key_shortcuts_follow_the_key_on_layouts_without_latin_letters() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let typed = |key, physical, text: &str| {
        vec![
            egui::Event::Key { key, physical_key: Some(physical), pressed: true, repeat: false, modifiers: Modifiers::NONE },
            egui::Event::Text(text.into()),
        ]
    };
    frame(&mut app, typed(Key::P, Key::P, "ح"));
    assert_eq!(app.session.tool_id(), "pen");
    frame(&mut app, typed(Key::V, Key::V, "м"));
    assert_eq!(app.session.tool_id(), "selection");
    frame(&mut app, typed(Key::A, Key::Q, "a"));
    assert_eq!(app.session.tool_id(), "directSelection", "AZERTY: the letter typed");
}

#[test]
fn plus_typed_any_way_zooms_in() {
    assert_eq!(parse("Cmd++"), parse("Cmd+="), "`+` and `=` are one chord key");
    assert_eq!(crate::shortcut_editor::chord_from_event(Key::Plus, Modifiers::COMMAND).as_deref(), Some("Cmd+="), "a recorded `+` too");
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let zoom = |app: &mut VectorcraftApp| app.view_mut().unwrap().zoom;
    let press = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    // `=`, Shift+`=` (reported as `=` or as `+`), and the numpad `+` / a layout's own `+` key.
    for (key, m) in [(Key::Equals, Modifiers::COMMAND), (Key::Equals, cmd_shift), (Key::Plus, cmd_shift), (Key::Plus, Modifiers::COMMAND)] {
        let before = zoom(&mut app);
        frame(&mut app, vec![press(key, m)]);
        assert!(zoom(&mut app) > before, "{key:?} with {m:?} zooms in");
    }
    let before = zoom(&mut app);
    frame(&mut app, vec![press(Key::Minus, Modifiers::COMMAND)]);
    assert!(zoom(&mut app) < before, "Cmd+- zooms out");
}

/// `+` selects the Add Anchor Point tool, as `-` selects Delete Anchor Point (#2142): typed as
/// Shift+`=` on a US layout, or by a layout's own `+` key or the numpad's.
#[test]
fn plus_selects_the_add_anchor_point_tool() {
    assert_eq!(parse("+"), parse("="), "a bare `+` is the `=` key");
    assert_eq!(crate::shortcut_editor::tool_for_key("+"), Some("addAnchor"));
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    for m in [Modifiers::SHIFT, Modifiers::NONE] {
        app.select_tool("selection");
        frame(&mut app, vec![egui::Event::ModifiersChanged(m), egui::Event::Text("+".into())]);
        assert_eq!(app.session.tool_id(), "addAnchor", "`+` with {m:?}");
    }
    frame(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::NONE), egui::Event::Text("-".into())]);
    assert_eq!(app.session.tool_id(), "deleteAnchor");
}

#[test]
fn digit_keys_reach_a_dragging_tool_once_per_press() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    app.session.execute("perspective.grid.preset", &json!({"kind": 2})).unwrap();
    let id = app.session.execute("shape.rectangle", &json!({"x": 450, "y": 380, "width": 60, "height": 60})).unwrap()["id"].clone();
    app.session.execute("perspective.attach", &json!({"ids": [id], "plane": "right"})).unwrap();
    app.select_tool("perspectiveSelection");
    let v = app.view_info();
    let c = app.session.doc().unwrap().doc.node(vectorcraft_engine::doc::NodeId(id.as_u64().unwrap())).unwrap().geometric_bounds().unwrap().center();
    app.session.pointer(&PointerEvent::new(PointerKind::Down, c.x, c.y), v).unwrap();
    app.session.pointer(&PointerEvent::new(PointerKind::Drag, c.x + 30.0, c.y), v).unwrap();
    let key = |repeat| egui::Event::Key { key: Key::Num5, physical_key: None, pressed: true, repeat, modifiers: Modifiers::NONE };
    // A held key repeats: only its first press toggles.
    frame(&mut app, vec![key(false), key(true)]);
    let preview = app.session.doc().unwrap().interaction.as_ref().unwrap().preview.clone().unwrap();
    assert_eq!((preview.0.as_str(), &preview.1["perpendicular"]), ("perspective.move", &json!(true)));
    assert_eq!(digit_of(Key::Num0), Some(0));
    assert_eq!(digit_of(Key::A), None);
}

/// E pressed while the Selection tool drags a marquee makes it select only the objects inside
/// it: once per press, without Cmd, and from the key at E's position on a layout without Latin
/// letters.
#[test]
fn e_reaches_a_dragged_marquee_once_per_press() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let inside = app.session.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].clone();
    let touched = app.session.execute("shape.rectangle", &json!({"x": 300, "y": 100, "width": 100, "height": 100})).unwrap()["id"].clone();
    app.select_tool("selection");
    let e = |key, repeat, modifiers| egui::Event::Key { key, physical_key: Some(Key::E), pressed: true, repeat, modifiers };
    let mut marquee = |events: Vec<egui::Event>| {
        let v = app.view_info();
        app.session.pointer(&PointerEvent::new(PointerKind::Down, 50.0, 50.0), v).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Drag, 350.0, 250.0), v).unwrap();
        frame(&mut app, events);
        app.session.pointer(&PointerEvent::new(PointerKind::Up, 350.0, 250.0), v).unwrap();
        let ids = app.session.doc().unwrap().selection.objects.iter().map(|i| json!(i.0)).collect::<Vec<_>>();
        assert_eq!(app.session.tool_id(), "selection");
        ids
    };
    assert_eq!(marquee(vec![]), vec![inside.clone(), touched.clone()]);
    // A held key repeats: only its first press switches.
    assert_eq!(marquee(vec![e(Key::E, false, Modifiers::NONE), e(Key::E, true, Modifiers::NONE)]), vec![inside.clone()]);
    let up = egui::Event::Key { key: Key::E, physical_key: Some(Key::E), pressed: false, repeat: false, modifiers: Modifiers::NONE };
    let twice = vec![e(Key::E, false, Modifiers::NONE), up, e(Key::E, false, Modifiers::NONE)];
    assert_eq!(marquee(twice), vec![inside.clone(), touched.clone()], "pressed twice: what it touches");
    // A Russian layout: egui-winit reports the key at E's position, and the text it types.
    assert_eq!(marquee(vec![e(Key::E, false, Modifiers::NONE), egui::Event::Text("у".into())]), vec![inside.clone()]);
    assert_eq!(marquee(vec![e(Key::E, false, Modifiers::COMMAND)]), vec![inside, touched], "Cmd+E is not E");
    assert_eq!(letter_of(Key::E), Some('e'));
    assert_eq!(letter_of(Key::Num1), None);
    assert_eq!(letter_of(Key::Escape), None);
}

/// W, the Blend tool's shortcut, pressed while the Blend tool holds a first pick finishes the
/// blend: the tool stays chosen, and the next click picks a new first object. Another tool's
/// shortcut does nothing while the pick is held.
#[test]
fn the_blend_tool_shortcut_finishes_the_blend() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
    for x in [0, 200] {
        app.session.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 40, "height": 40})).unwrap();
    }
    app.select_tool("blend");
    let click = |app: &mut VectorcraftApp, x: f64| {
        let v = app.view_info();
        app.session.pointer(&PointerEvent::new(PointerKind::Down, x, 20.0), v).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Up, x, 20.0), v).unwrap();
    };
    let press = |key| egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::NONE };
    click(&mut app, 20.0);
    assert!(app.session.tool_busy(), "the first rectangle is picked");
    let layers = app.session.active().unwrap().doc.layers.clone();
    frame(&mut app, vec![press(Key::P), egui::Event::Text("p".into())]);
    assert_eq!(app.session.tool_id(), "blend", "the Pen's shortcut does nothing while the pick is held");
    assert!(app.session.tool_busy());
    frame(&mut app, vec![press(Key::W), egui::Event::Text("w".into())]);
    assert_eq!(app.session.tool_id(), "blend");
    assert!(!app.session.tool_busy(), "W drops the pick");
    click(&mut app, 220.0);
    assert!(app.session.tool_busy(), "the second rectangle is a new first pick");
    assert_eq!(app.session.active().unwrap().doc.layers, layers, "no blend was made");
}

/// A Blend tool shortcut that arrives only as text (Shift+~ types `~`) also finishes the blend,
/// while another tool's shifted shortcut does not.
#[test]
fn a_text_only_blend_shortcut_finishes_the_blend() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "tool:curvature", "");
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "tool:blend", "Shift+~");
    app.session.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap();
    app.select_tool("blend");
    let v = app.view_info();
    app.session.pointer(&PointerEvent::new(PointerKind::Down, 20.0, 20.0), v).unwrap();
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 20.0, 20.0), v).unwrap();
    assert!(app.session.tool_busy(), "the rectangle is picked");
    frame(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::SHIFT), egui::Event::Text("W".into())]);
    assert!(app.session.tool_busy(), "Shift+W is another tool's shortcut and keeps the pick");
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "tool:width", "");
    frame(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::SHIFT), egui::Event::Text("W".into())]);
    assert!(app.session.tool_busy(), "an unbound Shift+W is not the Blend tool's W");
    let shift_w = egui::Event::Key { key: Key::W, physical_key: Some(Key::W), pressed: true, repeat: false, modifiers: Modifiers::SHIFT };
    let release = egui::Event::ModifiersChanged(Modifiers::NONE);
    frame(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::SHIFT), shift_w, release, egui::Event::Text("W".into())]);
    assert!(app.session.tool_busy(), "Shift released after Shift+W in the same frame keeps the pick");
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "tool:blend", "=");
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "tool:pen", "Shift+=");
    let shift_eq = egui::Event::Key { key: Key::Equals, physical_key: Some(Key::Equals), pressed: true, repeat: false, modifiers: Modifiers::SHIFT };
    frame(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::SHIFT), shift_eq, egui::Event::Text("+".into())]);
    assert!(app.session.tool_busy(), "Shift+= is the Pen's shortcut, not the Blend tool's =");
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "tool:blend", "Shift+~");
    frame(&mut app, vec![egui::Event::ModifiersChanged(Modifiers::SHIFT), egui::Event::Text("~".into())]);
    assert_eq!(app.session.tool_id(), "blend");
    assert!(!app.session.tool_busy(), "Shift+~ drops the pick");
}

/// A repeated key's text is no new press, whether it comes after its key or before it.
#[test]
fn a_repeated_keys_text_is_no_shortcut_press() {
    let key = |repeat| egui::Event::Key { key: Key::Backtick, physical_key: Some(Key::Backtick), pressed: true, repeat, modifiers: Modifiers::SHIFT };
    let text = || egui::Event::Text("~".into());
    assert!(shortcut_presses(&[key(true), text()], Modifiers::SHIFT).is_empty());
    assert!(shortcut_presses(&[text(), key(true)], Modifiers::SHIFT).is_empty());
    let fresh = shortcut_presses(&[key(false), text()], Modifiers::SHIFT);
    assert_eq!(fresh.len(), 2);
    assert_eq!(fresh[1].0.first().map(String::as_str), Some("Shift+Backtick"), "the text tries its key's name first");
    assert!(fresh[1].0.iter().any(|n| n == "Shift+~"));
    let only_text = shortcut_presses(&[text()], Modifiers::SHIFT);
    assert_eq!(only_text.len(), 1);
}

/// Shortcuts reach tools, panels (`window.panel {panel}`) and commands through the keymap,
/// and a held Cmd+= keeps zooming in.
#[test]
fn shortcuts_run_tools_panels_and_commands() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap();
    app.session.execute("select.set", &json!({"ids": []})).unwrap();
    let press = |key, modifiers, repeat| egui::Event::Key { key, physical_key: None, pressed: true, repeat, modifiers };
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "tool:pen", "F2");
    crate::shortcut_editor::sync(&app.ui);
    frame(&mut app, vec![press(Key::F2, Modifiers::NONE, false)]);
    assert_eq!(app.session.tool_id(), "pen", "a tool on a key that types nothing");
    app.select_tool("selection");
    frame(&mut app, vec![press(Key::F7, Modifiers::SHIFT, false)]);
    assert_eq!(app.ui.open_panel.as_deref(), Some("align"));
    frame(&mut app, vec![press(Key::A, Modifiers::COMMAND, false)]);
    assert_eq!(app.session.active().unwrap().selection.objects.len(), 2, "Select All");
    let zoom = |app: &mut VectorcraftApp| app.view_mut().unwrap().zoom;
    frame(&mut app, vec![press(Key::Equals, Modifiers::COMMAND, false)]);
    let once = zoom(&mut app);
    frame(&mut app, vec![press(Key::Equals, Modifiers::COMMAND, true)]);
    assert!(zoom(&mut app) > once, "a key repeat zooms again");
}

/// Cmd+, opens Settings on a Mac only, beside the frame's other shortcuts, and never takes a
/// chord a shortcut already has.
#[test]
fn cmd_comma_opens_settings_on_a_mac() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let press = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    let on = |app: &mut VectorcraftApp, os, events| {
        app.ui.dialog = None;
        app.session.execute("select.set", &json!({"ids": []})).unwrap();
        let ctx = egui::Context::default();
        ctx.set_os(os);
        ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| handle(app, ui.ctx())).textures_delta.clear();
        let settings = app.ui.dialog.as_ref().is_some_and(|d| d.kind == "preferences");
        (settings, app.session.active().unwrap().selection.objects.len())
    };
    let (mac, linux) = (egui::os::OperatingSystem::Mac, egui::os::OperatingSystem::Nix);
    let comma = press(Key::Comma, Modifiers::COMMAND);
    let select_all = press(Key::A, Modifiers::COMMAND);
    assert_eq!(on(&mut app, mac, vec![comma.clone()]), (true, 0));
    assert_eq!(on(&mut app, mac, vec![select_all.clone(), comma.clone()]), (true, 1), "with another shortcut in the frame");
    assert_eq!(on(&mut app, linux, vec![select_all, comma.clone()]), (false, 1), "not elsewhere");
    crate::keymap::set_override(&mut app.ui.shortcut_overrides, "select.all", "Cmd+,");
    crate::shortcut_editor::sync(&app.ui);
    assert_eq!(on(&mut app, mac, vec![comma]), (false, 1), "a shortcut that has it keeps it");
}

#[test]
fn parses() {
    let s = parse("Cmd+Shift+]").unwrap();
    assert_eq!(s.logical_key, Key::CloseBracket);
    assert!(s.modifiers.shift && s.modifiers.command);
    assert_eq!(parse("Cmd+=").unwrap().logical_key, Key::Equals);
    assert_eq!(parse("Cmd+Alt+2").unwrap().logical_key, Key::Num2);
    assert_eq!(parse("F12").unwrap().logical_key, Key::F12);
    assert!(parse("").is_none());
}

/// An IME replacement on the Type tool arrives as `DeleteSurroundingBytes` + `Commit` events
/// and is taken once, in order: the marked text is dropped, the committed text replaces it, and
/// the events never reach `route_type_input` as typed characters.
#[test]
fn ime_delete_surrounding_bytes_edits_the_type_tool_once() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let v = vectorcraft_engine::ViewInfo::default();
    app.session.select_tool("type", v).unwrap();
    app.session.pointer(&PointerEvent::new(PointerKind::Down, 100.0, 100.0), v).unwrap();
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 100.0, 100.0), v).unwrap();
    app.session.tool_text("Héllo", v).unwrap();
    let ctx = egui::Context::default();
    // Fonts install on the first logic pass and are ready on the second.
    for _ in 0..2 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.logic(ui.ctx()));
        out.textures_delta.clear();
    }
    let events = vec![
        egui::Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 3, after_bytes: 0 }),
        egui::Event::Ime(egui::ImeEvent::Commit("y".into())),
    ];
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| app.logic(ui.ctx()));
    out.textures_delta.clear();
    let mut text = String::new();
    if let Some(st) = app.session.active() {
        st.doc.walk(|node| {
            if let vectorcraft_doc::NodeKind::Text(t) = &node.kind {
                text = t.plain_text();
            }
        });
    }
    assert_eq!(text, "Héy");
    assert!(app.session.tool_wants_text(), "editing continues after the replacement");
}

/// X11/Wayland PRIMARY middle-click after a press in a panel field: `handle` runs before the
/// fields are drawn, and must leave the event for egui's text field (it pastes even without
/// focus). Deleting it here made PRIMARY paste dead outside dialogs.
#[test]
fn middle_click_paste_reaches_an_unfocused_text_field() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 200.0));
    let id = egui::Id::unique("middle-click-paste-test");
    let mut buf = String::new();
    let mut at = egui::pos2(40.0, 40.0);
    for step in 0..2 {
        let events =
            if step == 1 { vec![egui::Event::PointerMoved(at), egui::Event::MiddleClickPaste { pos: at, text: "x".into() }] } else { vec![] };
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| {
            handle(&mut app, ui.ctx());
            let resp = ui.add(egui::TextEdit::singleline(&mut buf).id(id));
            at = resp.rect.center();
        });
        out.textures_delta.clear();
    }
    assert_eq!(buf, "x");
}

/// A shortcut is taken once per frame, but only within the context that frame belongs to: a
/// new context starts again at frame 0 (a restarted host, or a test's fresh context) and its
/// input is new, not a rerun of the old context's frame 0.
#[test]
fn a_new_context_takes_its_own_frame_zero() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    let press = |app: &mut VectorcraftApp| {
        let ctx = egui::Context::default();
        let ev = egui::Event::Key { key: Key::N, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND | Modifiers::ALT };
        let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| handle(app, ui.ctx()));
        out.textures_delta.clear();
    };
    press(&mut app);
    assert_eq!(app.session.documents().len(), 1, "New Without Dialog makes a document");
    press(&mut app);
    assert_eq!(app.session.documents().len(), 2, "a second context's frame 0 is new input");
}

#[test]
fn all_registered_shortcuts_parse() {
    for c in vectorcraft_engine::command_specs() {
        if let Some(s) = c.shortcut
            && s != "D"
            && s != "X"
            && s != "Shift+X"
            && s != "/"
        {
            assert!(parse(s).is_some(), "{} has unparsable shortcut {s}", c.id);
        }
    }
    for c in crate::menus::UI_COMMANDS {
        if !c.2.is_empty() && c.2 != "F" && c.2 != "Shift+F" {
            assert!(parse(c.2).is_some(), "{} has unparsable shortcut {}", c.0, c.2);
        }
    }
}
