//! The app lifecycle through the framework's frame entry: input is taken once per frame, a discarded rerun draws again
//! without replaying it, and the Type tool's text part feeds the engine tool from the framework's IME engine.

use egui::Event;
use serde_json::json;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;

/// Build a new 400 by 300 document with one selected rectangle for lifecycle fixtures.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 40, "height": 30})).unwrap();
    app.session.execute("select.all", &json!({})).unwrap();
    app
}

/// Count child objects across every layer of the active document.
fn objects(app: &VectorcraftApp) -> usize {
    app.session.active().map_or(0, |st| st.doc.layers.iter().map(|l| l.children().map_or(0, |c| c.len())).sum())
}

/// Return the number of undo entries in the active document's history.
fn undo(app: &VectorcraftApp) -> usize {
    app.session.active().map_or(0, |st| st.history.undo.len())
}

/// A realistic window for headless frames: `RawInput::default()`'s viewport is huge enough for the
/// canvas texture to exceed egui's 2048-point limit.
fn screen() -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))
}

/// Foreign SVG: a paste of it adds one object through the internal clipboard, unlike plain text
/// (which only pastes when the app copied text itself).
const PASTE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>"#;

/// One frame through the production entry: the framework hands the app what it accumulated, then the app ticks and
/// draws. `rerun` asks egui to discard the first pass, as the multi-pass widgets do; returns the passes run and the
/// IME descriptor of the last pass.
fn run_frame(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    events: Vec<Event>,
    between: impl FnOnce(&mut VectorcraftApp),
    rerun: bool,
) -> (usize, Option<egui::output::IMEOutput>) {
    // PLAN: assumed `craft::ui::shell::embed::before_input` and `VectorcraftApp::frame_input(FrameOutput) -> FrameInput`
    // (sh.S7, hub-w1); the shell crate's name is not in the bundle.
    craft::ui::shell::embed::before_input(ctx, |output| app.frame_input(output));
    let raw = egui::RawInput { events, screen_rect: Some(screen()), ..Default::default() };
    let mut between = Some(between);
    let mut passes = 0;
    let mut out = ctx.run_ui(raw, |ui| {
        passes += 1;
        app.tick(ui.ctx());
        if let Some(between) = between.take() {
            between(app);
        }
        app.ui(ui);
        if rerun && passes == 1 {
            ui.ctx().request_discard("lifecycle test");
        }
    });
    // The commands the frame fired run as the host's next frame begins (`testing::app_frame`).
    craft::ui::shell::embed::before_input(ctx, |output| app.frame_input(output));
    out.textures_delta.clear();
    (passes, out.platform_output.ime)
}

/// A paste in a frame whose pass reruns runs once: one object and one undo step.
#[test]
fn a_discarded_rerun_runs_a_duplicate_once() {
    let mut app = app();
    let ctx = crate::testing::ui_context();
    // A paste event is the paste chord; its text is on the clipboard service, as the web shell offers it there.
    app.clipboard.offer_paste(vec![craft::platform::clipboard::Flavor::text(PASTE_SVG)]);
    // Fonts install on the first pass and are ready on the second.
    run_frame(&mut app, &ctx, vec![], |_| {}, false);
    run_frame(&mut app, &ctx, vec![], |_| {}, false);
    let (before_objects, before_undo) = (objects(&app), undo(&app));
    let (passes, _) = run_frame(&mut app, &ctx, vec![Event::Paste(PASTE_SVG.into())], |_| {}, true);
    assert!(passes >= 2, "the frame reran after the discard, got {passes} passes");
    assert_eq!(objects(&app), before_objects + 1, "the duplicate ran exactly once");
    assert_eq!(undo(&app), before_undo + 1, "one undo step");
}

/// While the selection moves every frame, as in a drag, each frame runs its UI once: the canvas's overlays are placed
/// over the canvas and keep the layout its measuring run recorded, so no frame is discarded and run again.
#[test]
fn a_moving_selection_runs_each_frame_once() {
    let mut app = app();
    let ctx = crate::testing::ui_context();
    for _ in 0..3 {
        run_frame(&mut app, &ctx, vec![], |_| {}, false);
    }
    let frames = 8;
    let passes: usize = (0..frames)
        .map(|_| {
            let step = |app: &mut VectorcraftApp| {
                app.session.execute("object.move", &json!({"dx": 2, "dy": 1})).unwrap();
            };
            run_frame(&mut app, &ctx, vec![], step, false).0
        })
        .sum();
    assert_eq!(passes, frames, "{passes} passes for {frames} frames");
}

/// One frame with an optional command applied between the tick and the draw (as a control request drained by `tick`
/// does); returns the IME descriptor of the last pass.
fn ime_frame(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    events: Vec<Event>,
    between: impl FnOnce(&mut VectorcraftApp),
) -> Option<egui::output::IMEOutput> {
    run_frame(app, ctx, events, between, false).1
}

/// Create a blank 400 by 300 document with one text object and return the app and node id;
/// placeholder text is disabled so the object is the only text.
fn app_with_text(text: &str) -> (VectorcraftApp, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.prefs.placeholder_text = false;
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = app.run("text.create", json!({"x": 100, "y": 100, "text": text, "size": 24})).unwrap()["id"].as_u64().unwrap();
    (app, id)
}

/// Read the plain text of the given text node in these lifecycle fixtures.
fn plain(app: &VectorcraftApp, id: u64) -> String {
    match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not text"),
    }
}

/// Send a pointer move and a primary press/release pair through the production lifecycle.
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, at: egui::Pos2) {
    let button =
        |pos: egui::Pos2, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    ime_frame(app, ctx, vec![egui::Event::PointerMoved(at), button(at, true)], |_| {});
    ime_frame(app, ctx, vec![button(at, false)], |_| {});
}

/// Start editing `id` with the Type tool and leave the caret where a click into its bounds lands.
fn begin_typing(app: &mut VectorcraftApp, ctx: &egui::Context, id: u64) -> egui::Pos2 {
    app.select_tool("type");
    ime_frame(app, ctx, vec![], |_| {});
    let center = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().center();
    let at = app.canvas_view().unwrap().to_screen(center);
    click(app, ctx, at);
    assert!(app.session.tool_wants_text(), "the click starts editing the text");
    at
}

/// Build an IME pre-edit event whose active clause is an empty character range at `chars`.
fn preedit(t: &str, chars: usize) -> Event {
    Event::Ime(egui::ImeEvent::Preedit { text: t.into(), active_range_chars: Some(chars..chars) })
}

/// Home takes the marked text down: the tool composes no more, the text stays as typed and no editor remains.
#[test]
fn home_takes_down_a_live_composition() {
    let (mut app, id) = app_with_text("Same");
    let ctx = crate::testing::ui_context();
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![], |_| {});
    begin_typing(&mut app, &ctx, id);
    ime_frame(&mut app, &ctx, vec![preedit("が", 1)], |_| {});
    assert!(app.ime_marked.is_some());
    app.run("app.home", json!({})).unwrap();
    // The framework interrupts the old owner's composition in the pass it loses focus; no pass after names an owner.
    let home = ime_frame(&mut app, &ctx, vec![], |_| {});
    assert!(home.is_none_or(|ime| ime.should_interrupt_composition), "Home only interrupts the composition");
    assert!(ime_frame(&mut app, &ctx, vec![], |_| {}).is_none(), "Home has no IME owner");
    assert!(app.ime_marked.is_none(), "Home takes the marked text down");
    assert!(!app.session.tool_composing(), "no stale composition survives Home");
    assert!(plain(&app, id).contains('が'), "the marked text stays as typed");
}

/// Closing the active document leaves no marked text for the next document and no composition.
#[test]
fn closing_the_document_takes_down_a_live_composition() {
    let (mut app, id) = app_with_text("Same");
    let ctx = crate::testing::ui_context();
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![], |_| {});
    begin_typing(&mut app, &ctx, id);
    ime_frame(&mut app, &ctx, vec![preedit("が", 1)], |_| {});
    assert!(app.ime_marked.is_some());
    assert!(app.session.close_document(0), "the document closes");
    let closed = ime_frame(&mut app, &ctx, vec![], |_| {});
    assert!(closed.is_none_or(|ime| ime.should_interrupt_composition), "closing only interrupts the composition");
    assert!(ime_frame(&mut app, &ctx, vec![], |_| {}).is_none(), "no document, no IME owner");
    assert!(app.ime_marked.is_none(), "closing takes the marked text down");
    assert!(!app.session.tool_composing());
}

/// Smart-quote replacement examines the character before the pre-edit: after `a` the committed quote is the closing one.
#[test]
fn an_ime_quote_commit_uses_the_character_before_its_pre_edit() {
    let (mut app, id) = app_with_text("a");
    // The pre-edit ends in an opening quote, but replacement starts after the letter `a`.
    app.run("document.setup", json!({"quotes": {"double": ["\"", "”"]}})).unwrap();
    let ctx = crate::testing::ui_context();
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![], |_| {});
    begin_typing(&mut app, &ctx, id);
    app.session.set_tool_option("select", &json!({"start": 1, "end": 1}));
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![preedit("\"", 1)], |_| {});
    ime_frame(&mut app, &ctx, vec![Event::Ime(egui::ImeEvent::Commit("\"".into()))], |_| {});
    assert_eq!(plain(&app, id), "a”", "the document inserts its closing quote");
}

/// An outside command can end the typing session (`endTyping`) while the Type tool still edits. The IME's following
/// `DeleteSurrounding` and `Commit` edit the text once, in order, and a later commit after a caret move edits again.
#[test]
fn ime_delete_surrounding_then_commit_edit_the_type_tool_once_in_order() {
    let (mut app, id) = app_with_text("abc");
    let ctx = crate::testing::ui_context();
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![], |_| {});
    begin_typing(&mut app, &ctx, id);
    app.session.set_tool_option("select", &json!({"start": 3, "end": 3}));
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![Event::Text("x".into())], |_| {});
    assert_eq!(plain(&app, id), "abcx");
    ime_frame(&mut app, &ctx, vec![], |app| {
        app.session.commit_interaction().expect("the typing preview is committed");
        app.session.set_tool_option("endTyping", &json!(true));
    });
    // The IME deletes the typed character and commits its replacement in the new session.
    ime_frame(
        &mut app,
        &ctx,
        vec![Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }), Event::Ime(egui::ImeEvent::Commit("Z".into()))],
        |_| {},
    );
    assert_eq!(plain(&app, id), "abcZ");
    let left = egui::Event::Key { key: egui::Key::ArrowLeft, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    ime_frame(&mut app, &ctx, vec![left], |_| {});
    ime_frame(&mut app, &ctx, vec![Event::Ime(egui::ImeEvent::Commit("Q".into()))], |_| {});
    assert_eq!(plain(&app, id), "abcQZ");
}
