//! The paste chords (egui reports every Cmd+V chord as a paste, not as its key) and the default
//! shortcuts (no chord does two things).

use egui::{Event, Key, Modifiers};
use serde_json::json;
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, shortcut_editor, shortcuts};

/// One headless frame of the shortcut handler (and the app's logic, which publishes copies) with
/// `modifiers` held and `events`.
fn frame(app: &mut VectorcraftApp, modifiers: Modifiers, events: Vec<Event>) -> egui::FullOutput {
    let events = std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect();
    let mut out = egui::Context::default().run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
        shortcuts::handle(app, ui.ctx());
        app.logic(ui.ctx());
    });
    out.textures_delta.clear();
    out
}

fn last_object_bounds(app: &VectorcraftApp) -> (f64, f64) {
    let st = app.session.active().unwrap();
    let id = st.doc.layers[0].children().unwrap().last().unwrap().id;
    let b = st.doc.node(id).unwrap().geometric_bounds().unwrap();
    (b.x0, b.y0)
}

#[test]
fn cmd_shift_v_pastes_in_place() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 30, "height": 30})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let out = frame(&mut app, Modifiers::COMMAND, vec![Event::Copy]);
    let svg = out
        .platform_output
        .commands
        .iter()
        .find_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
        .unwrap();
    // The text egui read from the clipboard comes with the chord still held.
    frame(&mut app, Modifiers::COMMAND | Modifiers::SHIFT, vec![Event::Paste(svg.clone())]);
    assert_eq!(app.session.active().unwrap().doc.layers[0].children().unwrap().len(), 2);
    assert_eq!(last_object_bounds(&app), (10.0, 20.0), "Cmd+Shift+V pastes in place");
    frame(&mut app, Modifiers::COMMAND, vec![Event::Paste(svg)]);
    assert_eq!(app.session.active().unwrap().doc.layers[0].children().unwrap().len(), 3);
    assert_ne!(last_object_bounds(&app), (10.0, 20.0), "Cmd+V pastes offset or centred");
}

#[test]
fn a_paste_takes_the_command_of_the_v_chord_held() {
    let cmd = Modifiers::COMMAND;
    assert_eq!(shortcuts::paste_command(cmd), "edit.paste");
    assert_eq!(shortcuts::paste_command(cmd | Modifiers::SHIFT), "edit.pasteInPlace");
    assert_eq!(shortcuts::paste_command(cmd | Modifiers::ALT), "edit.pasteWithoutFormatting");
    assert_eq!(shortcuts::paste_command(cmd | Modifiers::ALT | Modifiers::SHIFT), "edit.pasteOnAllArtboards");
    // The Paste key, and Shift+Insert, paste.
    assert_eq!(shortcuts::paste_command(Modifiers::NONE), "edit.paste");
    assert_eq!(shortcuts::paste_command(Modifiers::SHIFT), "edit.paste");
    // What isn't text: only the key's release is reported, with the chord's modifiers.
    let mut chord = shortcuts::PasteChord::default();
    let release = Event::Key { key: Key::V, physical_key: None, pressed: false, repeat: false, modifiers: cmd | Modifiers::SHIFT };
    assert_eq!(chord.textless_paste(&[release]), Some(cmd | Modifiers::SHIFT));
}

#[test]
fn no_two_commands_tools_or_panels_share_a_default_shortcut() {
    assert_eq!(shortcut_editor::all_conflicts(&craft_keymap::Overrides::new()), vec![]);
}

/// egui-winit reports every Cmd+C chord as a Copy, Shift and Alt held too, never as its key: the
/// Cmd+Shift+C alignment fired on a Mac (the menu's key equivalent) but copied elsewhere (#1940).
#[test]
fn a_copy_with_more_modifiers_is_the_c_chord_held() {
    use vectorcraft_doc::{Justify, NodeId, NodeKind};
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let id = app.run("text.create", json!({"x": 20, "y": 50, "text": "One\nTwo"})).unwrap()["id"].as_u64().unwrap();
    let justify = |app: &VectorcraftApp| match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (0..t.paragraph_count()).map(|i| t.para_at(i).justify).collect::<Vec<_>>(),
        _ => panic!("text"),
    };
    let copied = |out: &egui::FullOutput| out.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_)));
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let out = frame(&mut app, Modifiers::COMMAND | Modifiers::SHIFT, vec![Event::Copy]);
    assert_eq!(justify(&app), [Justify::Center, Justify::Center]);
    assert!(!copied(&out), "Cmd+Shift+C doesn't copy");
    // Cmd+C still copies.
    let out = frame(&mut app, Modifiers::COMMAND, vec![Event::Copy]);
    assert!(copied(&out));
    // While the Type tool edits text, it aligns the paragraph the caret is in.
    app.select_tool("type");
    app.session.apply_actions(vec![vectorcraft_tools::Action::Notify("text.editNew".into())]).unwrap();
    app.session.set_tool_option("select", &json!({"start": 5, "end": 5}));
    assert!(app.session.tool_wants_text());
    frame(
        &mut app,
        Modifiers::COMMAND | Modifiers::SHIFT,
        vec![Event::Key { key: Key::R, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND | Modifiers::SHIFT }],
    );
    assert_eq!(justify(&app), [Justify::Center, Justify::Right]);
    frame(&mut app, Modifiers::COMMAND | Modifiers::SHIFT, vec![Event::Copy]);
    assert_eq!(justify(&app), [Justify::Center, Justify::Center]);
}
