//! The paste chords (egui reports every Cmd+V chord as a paste, not as its key).

use egui::{Event, Modifiers};
use serde_json::json;
use vectorcraft_engine::Session;

use super::synthetic::key_frame_output;
use crate::VectorcraftApp;

/// One whole frame with `modifiers` held and `events` → the text it copied.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, modifiers: Modifiers, events: Vec<Event>) -> Option<String> {
    let events = std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect();
    let out = key_frame_output(app, ctx, events);
    out.platform_output.commands.into_iter().find_map(|c| if let egui::OutputCommand::CopyText(t) = c { Some(t) } else { None })
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
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, Modifiers::NONE, vec![]);
    // With no clipboard of the host's, the copy's text goes out through egui, which hands it to the system clipboard. The
    // chord's command runs after its pass, so the text may go out with the next one.
    let copied = frame(&mut app, &ctx, Modifiers::COMMAND, vec![Event::Copy]);
    let svg = copied.or_else(|| frame(&mut app, &ctx, Modifiers::COMMAND, vec![])).expect("copy publishes SVG");
    // The text egui read from the clipboard comes with the chord still held.
    frame(&mut app, &ctx, Modifiers::COMMAND | Modifiers::SHIFT, vec![Event::Paste(svg.clone())]);
    assert_eq!(app.session.active().unwrap().doc.layers[0].children().unwrap().len(), 2);
    assert_eq!(last_object_bounds(&app), (10.0, 20.0), "Cmd+Shift+V pastes in place");
    frame(&mut app, &ctx, Modifiers::COMMAND, vec![Event::Paste(svg)]);
    assert_eq!(app.session.active().unwrap().doc.layers[0].children().unwrap().len(), 3);
    assert_ne!(last_object_bounds(&app), (10.0, 20.0), "Cmd+V pastes offset or centred");
}
