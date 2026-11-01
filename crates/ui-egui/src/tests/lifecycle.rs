//! The 0.37 lifecycle: `raw_input_hook` (one host iteration) → `tick` in `App::logic` → `ui` in
//! `App::ui`. Input is taken once, on the first UI pass; a discarded rerun draws again without
//! replaying it, and a logic-only iteration ticks without reading the stale events.

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

/// A discarded rerun must execute a duplicate paste exactly once while the host iteration still
/// ticks once.
#[test]
fn a_discarded_rerun_ticks_once_and_runs_a_duplicate_once() {
    let mut app = app();
    let ctx = egui::Context::default();
    let frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
        let raw = egui::RawInput { events, screen_rect: Some(screen()), ..Default::default() };
        app.begin_host_iteration();
        let mut out = ctx.run_ui(raw, |ui| {
            app.tick(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
        out
    };
    // Fonts install on the first pass and are ready on the second.
    frame(&mut app, vec![]);
    frame(&mut app, vec![]);
    let (before_objects, before_undo, before_iteration) = (objects(&app), undo(&app), app.host_iteration);
    // A paste adds one object and one undo step: a second handling of the same event would add
    // two of each.
    let paste = Event::Paste(PASTE_SVG.into());
    let mut passes = 0;
    let raw = egui::RawInput { events: vec![paste], screen_rect: Some(screen()), ..Default::default() };
    app.begin_host_iteration();
    let mut out = ctx.run_ui(raw, |ui| {
        passes += 1;
        app.tick(ui.ctx());
        app.ui(ui);
        if passes == 1 {
            // Force the multi-pass path the new Grid widgets trigger in production.
            ui.ctx().request_discard("lifecycle test");
        }
    });
    out.textures_delta.clear();
    assert!(passes >= 2, "the frame reran after the discard, got {passes} passes");
    assert_eq!(app.host_iteration, before_iteration + 1, "one host iteration despite {passes} passes");
    assert_eq!(app.tick_iteration, Some(app.host_iteration), "tick once for the iteration");
    assert_eq!(objects(&app), before_objects + 1, "the duplicate ran exactly once");
    assert_eq!(undo(&app), before_undo + 1, "one undo step");
}

/// Logic-only iterations advance the host clock but never read the stale raw events a hidden window
/// replays.
#[test]
fn logic_only_iterations_tick_without_replaying_input() {
    let mut app = app();
    let ctx = egui::Context::default();
    // Two full frames: fonts, then a drawn frame.
    for _ in 0..2 {
        app.begin_host_iteration();
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen()), ..Default::default() }, |ui| {
            app.tick(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
    }
    let (before_objects, before_undo) = (objects(&app), undo(&app));
    let iterations = app.host_iteration;
    // The same duplicate event stays in this raw input, as eframe keeps the last shown frame's
    // events for a hidden window. `tick` must not read them.
    let stale = egui::RawInput { events: vec![Event::Paste(PASTE_SVG.into())], screen_rect: Some(screen()), ..Default::default() };
    for _ in 0..3 {
        app.begin_host_iteration();
        let _ = ctx.run_logic(&stale, |ctx| app.tick(ctx));
    }
    assert_eq!(app.host_iteration, iterations + 3);
    assert_eq!(app.tick_iteration, Some(app.host_iteration), "the last hidden iteration ticked");
    assert_eq!(objects(&app), before_objects, "no stale input was replayed");
    assert_eq!(undo(&app), before_undo, "no command ran hidden");
}

/// A visible frame consumes a paste once; later logic-only iterations with the same raw input must
/// not replay it.
#[test]
fn a_consumed_event_is_not_replayed_by_a_later_logic_iteration() {
    let mut app = app();
    let ctx = egui::Context::default();
    for _ in 0..2 {
        app.begin_host_iteration();
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen()), ..Default::default() }, |ui| {
            app.tick(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
    }
    let before = undo(&app);
    let raw = egui::RawInput { events: vec![Event::Paste(PASTE_SVG.into())], screen_rect: Some(screen()), ..Default::default() };
    app.begin_host_iteration();
    let mut out = ctx.run_ui(raw, |ui| {
        app.tick(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    assert_eq!(undo(&app), before + 1, "the visible frame ran it");
    // eframe hands the same events to `run_logic` for a hidden window; tick ignores them.
    let stale = egui::RawInput { events: vec![Event::Paste(PASTE_SVG.into())], screen_rect: Some(screen()), ..Default::default() };
    for _ in 0..2 {
        app.begin_host_iteration();
        let _ = ctx.run_logic(&stale, |ctx| app.tick(ctx));
    }
    assert_eq!(undo(&app), before + 1, "consumed input stays consumed");
}

/// One host iteration through the production lifecycle (`tick` then `ui`), with an optional
/// command applied between them (as a control request drained by `tick` does); returns the IME
/// descriptor of the last pass.
fn ime_frame(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    events: Vec<Event>,
    between: impl FnOnce(&mut VectorcraftApp),
) -> Option<egui::output::IMEOutput> {
    VectorcraftApp::install_ime_reset_hooks(ctx);
    let raw = egui::RawInput { events, screen_rect: Some(screen()), ..Default::default() };
    app.begin_host_iteration();
    let mut between = Some(between);
    let mut out = ctx.run_ui(raw, |ui| {
        app.tick(ui.ctx());
        if let Some(between) = between.take() {
            between(app);
        }
        app.ui(ui);
    });
    out.textures_delta.clear();
    out.platform_output.ime
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

/// Send a pointer move and a primary press/release pair through the production tick then UI
/// lifecycle.
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
    let xf = crate::canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let at = xf.to_screen(center);
    click(app, ctx, at);
    assert!(app.session.tool_wants_text(), "the click starts editing the text");
    at
}

/// Build an IME pre-edit event whose active clause is an empty character range at `chars`.
fn preedit(t: &str, chars: usize) -> Event {
    Event::Ime(egui::ImeEvent::Preedit { text: t.into(), active_range_chars: Some(chars..chars) })
}

/// Aim a real pointer at the already-drawn caret1, then start the composition at caret4.
fn interruption_editor() -> (VectorcraftApp, egui::Context, u64, egui::Pos2) {
    let (mut app, id) = app_with_text("abcd");
    let ctx = egui::Context::default();
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![], |_| {});
    begin_typing(&mut app, &ctx, id);
    app.session.set_tool_option("select", &json!({"start": 1, "end": 1}));
    let point = ime_frame(&mut app, &ctx, vec![], |_| {}).expect("caret1 is drawn").cursor_rect.center();
    assert!(app.canvas_rect.unwrap().contains(point));
    app.session.set_tool_option("select", &json!({"start": 4, "end": 4}));
    ime_frame(&mut app, &ctx, vec![], |_| {});
    (app, ctx, id, point)
}

/// Build a one-character or one-byte IME delete-surrounding event according to `bytes`.
fn surrounding_delete(bytes: bool) -> Event {
    Event::Ime(if bytes {
        egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }
    } else {
        egui::ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 0 }
    })
}

/// Build the reset-completed event for the app's current pending IME request.
fn reset_marker(app: &VectorcraftApp, ctx: &egui::Context) -> Event {
    Event::ImeResetCompleted { request_id: crate::ime::pending_request(app, ctx).expect("a real interruption requested a fence") }
}

/// Click the given point, assert the resulting custom descriptor publishes the real reset request,
/// and return that request.
fn pointer_reset(app: &mut VectorcraftApp, ctx: &egui::Context, point: egui::Pos2) -> egui::ImeResetId {
    let press = Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() };
    let output = ime_frame(app, ctx, vec![Event::PointerMoved(point), press], |_| {}).expect("the destination still edits text");
    let request = output.reset_request.expect("the final destination publishes its real request");
    let release = Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() };
    assert!(ime_frame(app, ctx, vec![release], |_| {}).unwrap().reset_request.is_none(), "an unchanged pass does not request again");
    assert_eq!(crate::ime::pending_request(app, ctx), Some(request));
    request
}

/// Build a pressed, non-repeat key event with the given key and modifiers.
fn pressed(key: egui::Key, modifiers: egui::Modifiers) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

/// A replacement fence admits an immediate fresh suffix without an old terminal, for empty and non-
/// empty pre-edits and for same- and next-batch ordering.
#[test]
fn reset_fence_without_an_old_terminal_admits_the_immediate_fresh_suffix() {
    for terminal_pending in [false, true] {
        for next_batch in [false, true] {
            let (mut app, ctx, id, point) = interruption_editor();
            ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
            if terminal_pending {
                ime_frame(&mut app, &ctx, vec![preedit("", 0)], |_| {});
            }
            let request = pointer_reset(&mut app, &ctx, point);
            let marker = Event::ImeResetCompleted { request_id: request };
            let mut events = if next_batch {
                ime_frame(&mut app, &ctx, vec![marker], |_| {});
                vec![]
            } else {
                vec![marker]
            };
            events.extend([preedit("が", 1), Event::Ime(egui::ImeEvent::Commit("が".into()))]);
            ime_frame(&mut app, &ctx, events, |_| {});
            assert_eq!(plain(&app, id), if terminal_pending { "aがbcd" } else { "aがbcdé" });
            assert!(!crate::ime::blocked(&app, &ctx));
        }
    }
}

/// Only the newest reset request for this viewport, context and destination completes; old, foreign
/// and duplicate markers cannot retire it or admit input.
#[test]
fn newest_full_reset_id_rejects_old_foreign_and_duplicate_controls() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let first = pointer_reset(&mut app, &ctx, point);
    app.session.set_tool_option("select", &json!({"start": 2, "end": 2}));
    let second = ime_frame(&mut app, &ctx, vec![], |_| {}).unwrap().reset_request.expect("a second caret interruption supersedes R1");
    assert_ne!(first, second);
    let foreign = ctx.request_ime_reset(egui::ViewportId::from_hash_of("foreign-reset-owner")).unwrap();
    let foreign_context = egui::Context::default().request_ime_reset(egui::ViewportId::ROOT).unwrap();
    let text = plain(&app, id);
    let document = app.session.active().unwrap().doc.clone();
    let steps = undo(&app);
    ime_frame(
        &mut app,
        &ctx,
        vec![
            Event::ImeResetCompleted { request_id: first },
            Event::ImeResetCompleted { request_id: foreign },
            Event::ImeResetCompleted { request_id: foreign_context },
            preedit("late", 4),
            preedit("", 0),
            surrounding_delete(false),
            surrounding_delete(true),
            Event::Ime(egui::ImeEvent::Commit("X".into())),
        ],
        |_| {},
    );
    assert_eq!(crate::ime::pending_request(&app, &ctx), Some(second));
    assert_eq!(plain(&app, id), text);
    assert!(std::sync::Arc::ptr_eq(&document, &app.session.active().unwrap().doc));
    assert_eq!(undo(&app), steps);
    ime_frame(
        &mut app,
        &ctx,
        vec![
            Event::ImeResetCompleted { request_id: second },
            preedit("Q", 1),
            Event::ImeResetCompleted { request_id: second },
            preedit("QR", 2),
            Event::Ime(egui::ImeEvent::Commit("QR".into())),
        ],
        |_| {},
    );
    assert_eq!(plain(&app, id), "abQRcdé", "a duplicate marker cannot retire the new live preview");
}

/// A pressed navigation key retires the old fence, moves the caret once, and the final destination
/// publishes a replacement fence before its suffix is accepted.
#[test]
fn pressed_navigation_retires_the_old_fence_until_the_real_destination_is_known() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let first = pointer_reset(&mut app, &ctx, point);
    let output = ime_frame(
        &mut app,
        &ctx,
        vec![
            pressed(egui::Key::ArrowRight, egui::Modifiers::NONE),
            Event::ImeResetCompleted { request_id: first },
            surrounding_delete(true),
            Event::Ime(egui::ImeEvent::Commit("X".into())),
            preedit("late", 4),
        ],
        |_| {},
    )
    .unwrap();
    assert_eq!(app.session.tool_options()["caret"], 2, "the actual pressed key moved the caret once");
    assert_eq!(plain(&app, id), "abcdé", "the deferred handler's raw suffix stays closed");
    let second = output.reset_request.expect("the final caret publishes a replacement fence");
    assert_ne!(first, second);
    ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: second }, preedit("が", 1)], |_| {});
    assert_eq!(plain(&app, id), "abがcdé");
}

/// A real permitted command key still runs while composing or after retirement; the cached
/// composing flag must not swallow Cmd+R.
#[test]
fn a_pressed_command_runs_after_composition_retirement_and_after_its_fence() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let rulers = app.ui.view.rulers;
    let press = Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() };
    ime_frame(&mut app, &ctx, vec![Event::PointerMoved(point), press, pressed(egui::Key::R, egui::Modifiers::COMMAND)], |_| {});
    assert_eq!(app.ui.view.rulers, !rulers, "the cached composing flag must not swallow a real permitted command press");
    assert_eq!(plain(&app, id), "abcdé");

    // A separate unchanged-layout editor proves the command remains available after F too.
    let (mut app, ctx, _, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let request = pointer_reset(&mut app, &ctx, point);
    let rulers = app.ui.view.rulers;
    ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: request }, pressed(egui::Key::R, egui::Modifiers::COMMAND)], |_| {});
    assert_eq!(app.ui.view.rulers, !rulers);
}

/// A request allocated in a discarded pass is retired; an unavailable final pass leaves no token,
/// and the same caret re-arms once when visible again.
#[test]
fn discarded_request_then_hidden_output_rearms_the_same_caret_once() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let previous = pointer_reset(&mut app, &ctx, point);
    app.session.set_tool_option("select", &json!({"start": 2, "end": 2}));
    let selection = app.session.tool_options();
    let raw = egui::RawInput { screen_rect: Some(screen()), ..Default::default() };
    let mut passes = 0;
    let mut discarded = None;
    app.begin_host_iteration();
    let mut out = ctx.run_ui(raw, |ui| {
        passes += 1;
        if passes > 1 {
            // A real reported occlusion removes this pass's custom producer without changing A.
            ui.ctx().input_mut(|input| input.raw.viewports.entry(egui::ViewportId::ROOT).or_default().occluded = Some(true));
        }
        app.tick(ui.ctx());
        app.ui(ui);
        if passes == 1 {
            // Core attaches the token after the callback. This is the adapter's final ID for
            // the actual custom descriptor, after all deferred handlers and UI writers ran.
            assert!(ui.ctx().output(|output| output.ime.is_some()));
            discarded = crate::ime::pending_request(&app, ui.ctx());
            ui.ctx().request_discard("reset descriptor discarded before an unavailable final pass");
        }
    });
    out.textures_delta.clear();
    assert!(passes >= 2);
    let first = discarded.expect("the discarded pass had a new request for its final destination");
    assert_ne!(previous, first);
    assert!(out.platform_output.ime.is_none());
    assert!(crate::ime::pending_request(&app, &ctx).is_none(), "R1 is retired, not left awaiting an impossible acknowledgement");
    assert!(crate::ime::blocked(&app, &ctx));
    assert_eq!(app.session.tool_options(), selection);
    let second = ime_frame(&mut app, &ctx, vec![], |_| {}).unwrap().reset_request.expect("the same A/caret reactivates with R2");
    assert_ne!(first, second);
    assert!(ime_frame(&mut app, &ctx, vec![], |_| {}).unwrap().reset_request.is_none());
    ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: first }, preedit("late", 4)], |_| {});
    assert_eq!(plain(&app, id), "abcdé");
    ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: second }, preedit("が", 1)], |_| {});
    assert_eq!(plain(&app, id), "abがcdé");
}

/// Stock text remains editable while custom fences retire; returning from stock focus and losing
/// window focus each allocate a fresh fence.
#[test]
fn stock_field_and_focus_loss_retire_custom_fences_without_consuming_stock_text() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let first = pointer_reset(&mut app, &ctx, point);
    app.ui.palette_open = true;
    ime_frame(&mut app, &ctx, vec![], |_| {});
    let stock = ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: first }, Event::Text("xyz".into())], |_| {})
        .expect("the palette's stock TextEdit publishes its descriptor");
    assert!(app.ui.palette_query.contains("xyz"), "controls-only routing leaves stock text intact");
    let view = app.view_info();
    assert_ne!(stock.owner, crate::ime::target(&mut app.session, view).unwrap().owner);
    assert!(stock.reset_request.is_none(), "the stock descriptor never carries the custom fence");
    assert!(crate::ime::pending_request(&app, &ctx).is_none());
    assert_eq!(plain(&app, id), "abcdé");
    app.ui.palette_open = false;
    ctx.memory_mut(|memory| {
        if let Some(field) = memory.focused() {
            memory.surrender_focus(field);
        }
    });
    let second = ime_frame(&mut app, &ctx, vec![], |_| {}).unwrap().reset_request.expect("returning from the stock field rearms");
    assert_ne!(first, second);
    let raw = egui::RawInput {
        focused: false,
        screen_rect: Some(screen()),
        events: vec![Event::ImeResetCompleted { request_id: second }],
        ..Default::default()
    };
    app.begin_host_iteration();
    let mut out = ctx.run_ui(raw, |ui| {
        app.tick(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    assert!(crate::ime::pending_request(&app, &ctx).is_none());
    let third = ime_frame(&mut app, &ctx, vec![], |_| {}).unwrap().reset_request.expect("focus reactivation needs a new fence");
    assert_ne!(second, third);
    ime_frame(
        &mut app,
        &ctx,
        vec![Event::ImeResetCompleted { request_id: second }, preedit("late", 4), Event::ImeResetCompleted { request_id: third }, preedit("が", 1)],
        |_| {},
    );
    assert_eq!(plain(&app, id), "aがbcdé");
}

/// Text, paste and cut events before a deferred pointer's real handler are dropped; the pointer
/// still moves the caret and publishes a replacement fence.
#[test]
fn a_deferred_pointer_drops_text_and_clipboard_until_its_real_handler_and_fence() {
    for paste in [false, true] {
        let (mut app, ctx, id, point) = interruption_editor();
        ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
        let first = pointer_reset(&mut app, &ctx, point);
        let text = plain(&app, id);
        let steps = undo(&app);
        let mut events = vec![Event::PointerMoved(point), primary_button(point, true)];
        if paste {
            events.push(Event::Paste("P".into()));
        } else {
            events.push(Event::Text("x".into()));
            events.push(Event::Cut);
        }
        // The old marker arrives as a suffix, after the application text/clipboard event.
        events.push(Event::ImeResetCompleted { request_id: first });
        let output = ime_frame(&mut app, &ctx, events, |_| {}).expect("the pointer destination still edits text");
        assert_eq!(plain(&app, id), text, "the stale prefix mutates neither the old caret nor a new destination");
        assert_eq!(undo(&app), steps, "dropped text and clipboard events create no undo step");
        assert_eq!(app.session.tool_options()["caret"], 1, "the real pointer handler still moves the caret once");
        let second = output.reset_request.expect("the actual destination publishes its replacement fence");
        assert_ne!(first, second);
        assert_eq!(crate::ime::pending_request(&app, &ctx), Some(second));
        // The retired marker and its suffix stay inert; the newest fence admits fresh input.
        ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: first }, preedit("late", 4)], |_| {});
        assert_eq!(plain(&app, id), text);
        ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: second }, preedit("が", 1)], |_| {});
        assert_eq!(plain(&app, id), "aがbcdé");
    }
}

/// A hidden route allocates no reset; an old marker and pointer suffix are inert and cannot lend a
/// token to a later stock field.
#[test]
fn hidden_pointer_and_marker_leave_no_token_for_a_later_stock_field() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let first = pointer_reset(&mut app, &ctx, point);
    let _ = crate::ime::take_request_count(&ctx);
    // The route is already known ineligible: hidden and with an old marker followed by a pointer
    // and navigation notice. No new request may be allocated for a descriptor this pass cannot
    // publish.
    let mut raw = egui::RawInput {
        events: vec![
            Event::PointerMoved(point),
            primary_button(point, true),
            Event::Ime(egui::ImeEvent::Commit("X".into())),
            Event::ImeResetCompleted { request_id: first },
            pressed(egui::Key::ArrowRight, egui::Modifiers::NONE),
        ],
        screen_rect: Some(screen()),
        ..Default::default()
    };
    raw.viewports.entry(egui::ViewportId::ROOT).or_default().occluded = Some(true);
    app.begin_host_iteration();
    let mut out = ctx.run_ui(raw, |ui| {
        app.tick(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    assert_eq!(crate::ime::take_request_count(&ctx), 0, "an ineligible route allocates no reset");
    assert!(crate::ime::pending_request(&app, &ctx).is_none(), "the old fence is retired, not rearmed");
    assert!(crate::ime::blocked(&app, &ctx), "the guard stays suspended until a real custom destination");
    assert_eq!(plain(&app, id), "abcdé", "the hidden marker and pointer suffix mutate nothing");
    app.ui.palette_open = true;
    // A newly shown TextEdit requests focus after it has already sampled
    // `owns_ime_events`; its stock descriptor is published on the next pass.
    assert!(ime_frame(&mut app, &ctx, vec![], |_| {}).is_none(), "the palette's focus pass publishes no descriptor or reset token");
    let stock = ime_frame(&mut app, &ctx, vec![], |_| {}).expect("the palette's stock TextEdit publishes its descriptor");
    assert!(stock.reset_request.is_none(), "no hidden-route token reaches a stock field");
    assert!(crate::ime::pending_request(&app, &ctx).is_none());
}

/// A hidden logic pass settles a changed composition without allocating, leaving no token for a
/// later stock field.
#[test]
fn hidden_logic_with_a_changed_composition_leaves_no_token_for_a_later_stock_field() {
    let (mut app, ctx, id, _) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    // The application changed the composition's destination while the window is hidden; a logic
    // pass settles it, but has no custom descriptor to consume a request.
    app.session.set_tool_option("select", &json!({"start": 1, "end": 1}));
    let _ = crate::ime::take_request_count(&ctx);
    let raw = egui::RawInput { screen_rect: Some(screen()), ..Default::default() };
    app.begin_host_iteration();
    let _ = ctx.run_logic(&raw, |ctx| {
        app.tick(ctx);
        app.finish_ime_logic(ctx);
    });
    assert_eq!(crate::ime::take_request_count(&ctx), 0, "hidden logic allocates no reset");
    assert!(crate::ime::pending_request(&app, &ctx).is_none());
    assert!(!app.session.tool_composing(), "the changed preview settled at its original position");
    assert_eq!(plain(&app, id), "abcdé");
    app.ui.palette_open = true;
    // A newly shown TextEdit requests focus after it has already sampled
    // `owns_ime_events`; its stock descriptor is published on the next pass.
    assert!(ime_frame(&mut app, &ctx, vec![], |_| {}).is_none(), "the palette's focus pass publishes no descriptor or reset token");
    let stock = ime_frame(&mut app, &ctx, vec![], |_| {}).expect("the palette's stock TextEdit publishes its descriptor");
    assert!(stock.reset_request.is_none(), "hidden logic leaves no token for the stock field");
}

/// Home suspends before input, so an interrupted commit allocates no reset and its old marker
/// cannot rearm or leak into a stock field.
#[test]
fn home_with_an_interrupted_commit_allocates_no_reset_and_leaves_no_stock_token() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let first = pointer_reset(&mut app, &ctx, point);
    // R1 is live and the incoming latch is Stale. A fresh preedit/commit here is not reachable:
    // `shortcuts.rs` drops IME mutations while Stale. Application text still is: `Text` changes
    // the committed stamp and caret without being an IME mutation. Home is already known at
    // pass entry, so route suspension retires R1 before text/ArrowRight handling. No new reset
    // may be allocated for this pass's non-custom route.
    let _ = crate::ime::take_request_count(&ctx);
    let output = ime_frame(&mut app, &ctx, vec![Event::Text("x".into()), pressed(egui::Key::ArrowRight, egui::Modifiers::NONE)], |app| {
        // Chosen after `tick`, before this pass's UI/input handling: the same route ordering
        // as the original regression, with Home already known when the notice is processed.
        app.run("app.home", json!({})).unwrap();
        assert!(crate::menus::home_showing(app));
    });
    assert!(output.is_none(), "Home publishes no custom descriptor");
    assert_eq!(crate::ime::take_request_count(&ctx), 0, "Home suspends the old fence before input; no new reset is allocated");
    assert!(crate::ime::pending_request(&app, &ctx).is_none(), "the old fence is retired, not rearmed");
    assert_eq!(plain(&app, id), "axbcdé", "the application text lands at the fenced caret; Home changes no text");
    app.ui.palette_open = true;
    // A newly shown TextEdit requests focus after it has already sampled
    // `owns_ime_events`; its stock descriptor is published on the next pass.
    assert!(ime_frame(&mut app, &ctx, vec![], |_| {}).is_none(), "the palette's focus pass publishes no descriptor or reset token");
    let stock = ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: first }], |_| {})
        .expect("the palette's stock TextEdit publishes its descriptor");
    assert!(stock.reset_request.is_none(), "Home's route suspension leaves no token for the stock field");
    assert!(crate::ime::pending_request(&app, &ctx).is_none(), "the old marker cannot rearm a retired fence");
}

/// A command batch that opens the palette allocates no speculative reset; the palette's stock field
/// receives no custom token.
#[test]
fn command_palette_opened_by_a_commit_batch_gets_no_speculative_reset() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let _ = pointer_reset(&mut app, &ctx, point);
    // R1 is live and Stale. The application Text below is accepted while Stale and changes the
    // committed context and caret. The following Cmd+Shift+/ is a command notice whose palette
    // only opens later in this same pass: binding at the notice would outlive the palette's
    // stock descriptor.
    let _ = crate::ime::take_request_count(&ctx);
    let output = ime_frame(
        &mut app,
        &ctx,
        vec![Event::Text("x".into()), pressed(egui::Key::Questionmark, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT)],
        |_| {},
    );
    assert!(app.ui.palette_open, "the batch opened the command palette");
    assert_eq!(crate::ime::take_request_count(&ctx), 0, "the pre-UI command settlement allocates no reset");
    assert!(crate::ime::pending_request(&app, &ctx).is_none(), "no token awaits the palette");
    assert!(output.is_none(), "the opening batch only takes stock focus; the descriptor is next pass");
    let stock = ime_frame(&mut app, &ctx, vec![], |_| {}).expect("the palette's stock TextEdit publishes its descriptor");
    assert!(stock.reset_request.is_none(), "the palette's stock field gets no custom token");
    assert_eq!(plain(&app, id), "axbcdé", "the application text lands at the fenced caret");
}

/// Presses that end on an invalid destination release no custom descriptor and allocate no reset
/// for the stock field.
#[test]
fn multiple_pointer_presses_ending_on_an_invalid_destination_leave_no_stock_token() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let text = plain(&app, id);
    // Clicking the edited caret, then clicking an empty part of the canvas, ends the editing
    // session. A token allocated on the empty click's preflight notice would outlive the
    // now-ineligible route, so the finalizer must be the only allocator.
    let xf = crate::canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    let empty = xf.to_screen(vectorcraft_geom::Point::new(300.0, 250.0));
    click(&mut app, &ctx, point);
    assert!(app.session.tool_wants_text(), "the first press edits the caret");
    let _ = ime_frame(&mut app, &ctx, vec![Event::PointerMoved(empty), primary_button(empty, true)], |_| {});
    assert!(app.session.tool_wants_text(), "the empty press ends the edit only on release");
    // The release publishes no custom descriptor and must allocate nothing for the stock field.
    let _ = crate::ime::take_request_count(&ctx);
    let output = ime_frame(&mut app, &ctx, vec![primary_button(empty, false)], |_| {});
    assert!(output.is_none(), "the final release ended the custom editor");
    assert_eq!(crate::ime::take_request_count(&ctx), 0, "the invalidated final destination allocates no reset");
    assert!(crate::ime::pending_request(&app, &ctx).is_none());
    assert_eq!(plain(&app, id), text, "the presses mutate no text");
    app.ui.palette_open = true;
    // A newly shown TextEdit requests focus after it has already sampled
    // `owns_ime_events`; its stock descriptor is published on the next pass.
    assert!(ime_frame(&mut app, &ctx, vec![], |_| {}).is_none(), "the palette's focus pass publishes no descriptor or reset token");
    let stock = ime_frame(&mut app, &ctx, vec![], |_| {}).expect("the palette's stock TextEdit publishes its descriptor");
    assert!(stock.reset_request.is_none(), "the invalidated final pointer leaves no stock token");
}

/// Cmd+X and Cmd+V in a deferred command batch are dropped until the replacement fence, leaving the
/// text and undo history unchanged.
#[test]
fn deferred_command_key_forms_drop_cmd_x_and_cmd_v_until_the_fence() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let first = pointer_reset(&mut app, &ctx, point);
    app.session.set_tool_option("select", &json!({"start": 0, "end": 2}));
    let text = plain(&app, id);
    let steps = undo(&app);
    // Cmd+R is a command notice that leaves the selection alone; the following key-form Cmd+X and
    // Cmd+V must not cut or paste at the old caret before the final destination is known.
    let output = ime_frame(
        &mut app,
        &ctx,
        vec![
            pressed(egui::Key::R, egui::Modifiers::COMMAND),
            pressed(egui::Key::X, egui::Modifiers::COMMAND),
            pressed(egui::Key::V, egui::Modifiers::COMMAND),
            Event::ImeResetCompleted { request_id: first },
        ],
        |_| {},
    )
    .expect("the destination still edits text");
    assert_eq!(plain(&app, id), text, "deferred Cmd+X cuts nothing at the old caret");
    assert_eq!(undo(&app), steps, "deferred Cmd+X creates no undo step");
    let second = output.reset_request.expect("the actual destination publishes its replacement fence");
    assert_ne!(first, second);
}

/// A live composition alone requests no reset; a stock field publishes no custom token, and
/// returning to Type accepts fresh composition.
#[test]
fn a_stock_field_with_an_active_composition_but_no_guard_gets_no_reset_request() {
    let (mut app, ctx, id, _) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    assert!(crate::ime::pending_request(&app, &ctx).is_none(), "a live composition alone requests nothing");
    app.session.set_tool_option("select", &json!({"start": 1, "end": 1}));
    app.ui.palette_open = true;
    let foreign = egui::Context::default().request_ime_reset(egui::ViewportId::ROOT).unwrap();
    // A newly shown TextEdit requests focus after it has already sampled
    // `owns_ime_events`; its stock descriptor is published on the next pass.
    assert!(ime_frame(&mut app, &ctx, vec![], |_| {}).is_none(), "the palette's focus pass publishes no descriptor or reset token");
    let stock = ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: foreign }], |_| {})
        .expect("the palette's stock TextEdit publishes its descriptor");
    assert!(stock.reset_request.is_none(), "a control-only pass with no custom guard allocates no request");
    assert!(crate::ime::pending_request(&app, &ctx).is_none());
    assert_eq!(plain(&app, id), "abcdé", "the interrupted preview settles at its original position");
    app.ui.palette_open = false;
    // The palette's stock TextEdit was focused last pass. It disappears this pass, but egui's
    // focus dead-man switch only runs at end_pass, so surrender it before the Type tool can own
    // the keyboard again. This fixture has no guard to acknowledge; the fresh preedit below is
    // blocked only by that stale focus, not by a Stale latch.
    ctx.memory_mut(|memory| {
        if let Some(field) = memory.focused() {
            memory.surrender_focus(field);
        }
    });
    let returned = ime_frame(&mut app, &ctx, vec![], |_| {}).expect("the Type tool owns the IME again");
    assert!(returned.reset_request.is_none(), "no guard existed to re-arm");
    ime_frame(&mut app, &ctx, vec![preedit("が", 1)], |_| {});
    assert_eq!(plain(&app, id), "aがbcdé", "a clear adapter accepts fresh composition without a stale latch");
}

/// Opening a modal after the canvas suspends the fence; returning to the same caret allocates a new
/// fence and old input stays stale.
#[test]
fn a_modal_dialog_suspends_a_live_fence_and_the_same_caret_rearms() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let first = pointer_reset(&mut app, &ctx, point);
    let caret = app.session.tool_options()["caret"].clone();
    let during = ime_frame(&mut app, &ctx, vec![], |app| {
        app.ui.told.push(crate::state::Dialog::new(crate::dialogs::confirm::MESSAGE, json!({"message": "A message", "detail": ""})));
    });
    assert!(app.ui.dialog.is_some(), "the modal dialog opened after the canvas drew");
    assert!(during.is_none(), "a modal covering the canvas is not a custom IME owner");
    let returned = ime_frame(&mut app, &ctx, vec![], |app| {
        app.ui.dialog = None;
    })
    .expect("the Type tool owns the IME again");
    assert_eq!(app.session.tool_options()["caret"], caret, "returning to the same caret");
    let second = returned.reset_request.expect("the same caret re-arms after the modal");
    assert_ne!(first, second);
    ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: first }, preedit("late", 4)], |_| {});
    assert_eq!(plain(&app, id), "abcdé");
    ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: second }, preedit("が", 1)], |_| {});
    assert_eq!(plain(&app, id), "aがbcdé");
}

/// An absent pass starts a new context generation for the same owner and publishes a replacement
/// fence on return.
#[test]
fn a_same_owner_absent_pass_gets_a_new_generation_and_replacement_fence() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let first = pointer_reset(&mut app, &ctx, point);
    let before = ime_frame(&mut app, &ctx, vec![], |_| {}).expect("the Type tool owns the IME");
    app.run("app.home", json!({})).unwrap();
    assert!(ime_frame(&mut app, &ctx, vec![], |_| {}).is_none(), "Home has no custom IME owner");
    app.ui.home = None;
    let after = ime_frame(&mut app, &ctx, vec![], |_| {}).expect("the same Type tool owner returns");
    assert_eq!(after.owner, before.owner, "the model owner did not change");
    assert_ne!(after.context_generation, before.context_generation, "an absent pass starts a new context generation");
    let second = after.reset_request.expect("the new generation publishes a replacement fence");
    assert_ne!(first, second);
    ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: first }, preedit("late", 4)], |_| {});
    assert_eq!(plain(&app, id), "abcdé");
    ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: second }, preedit("が", 1)], |_| {});
    assert_eq!(plain(&app, id), "aがbcdé");
}

/// An occluded pass leaves the marker inert and keeps the guard suspended until a visible pass re-
/// arms it.
#[test]
fn geometry_loss_before_a_marker_keeps_the_fence_suspended_until_reactivation() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let first = pointer_reset(&mut app, &ctx, point);
    let text = plain(&app, id);
    // A hidden/occluded pass has no custom descriptor. Its marker must not complete R1 before the
    // route has an editor again.
    let mut raw = egui::RawInput { events: vec![Event::ImeResetCompleted { request_id: first }], screen_rect: Some(screen()), ..Default::default() };
    raw.viewports.entry(egui::ViewportId::ROOT).or_default().occluded = Some(true);
    app.begin_host_iteration();
    let mut out = ctx.run_ui(raw, |ui| {
        app.tick(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    assert!(crate::ime::pending_request(&app, &ctx).is_none(), "the occluded marker is inert");
    assert!(crate::ime::blocked(&app, &ctx), "the guard stays suspended, not completed");
    let second = ime_frame(&mut app, &ctx, vec![], |_| {}).unwrap().reset_request.expect("visible reactivation re-arms");
    assert_ne!(first, second);
    ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: first }, preedit("late", 4)], |_| {});
    assert_eq!(plain(&app, id), text);
    ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: second }, preedit("が", 1)], |_| {});
    assert_eq!(plain(&app, id), "aがbcdé");
}

/// Settling a preview through an application selection change allocates exactly one fence for the
/// post-settlement destination, with no preflight retry.
#[test]
fn settling_a_preview_without_a_pointer_allocates_one_request_after_the_commit() {
    let (mut app, ctx, id, _) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let _ = crate::ime::take_request_count(&ctx);
    let output = ime_frame(&mut app, &ctx, vec![], |app| {
        // An application selection/caret change ends the live preview without a pointer or
        // navigation event. The fence must describe the post-settlement destination directly.
        app.session.set_tool_option("select", &json!({"start": 1, "end": 1}));
    })
    .expect("the Type tool still owns the IME");
    assert_eq!(crate::ime::take_request_count(&ctx), 1, "settlement allocates one fence, not a pre-commit preflight");
    let request = output.reset_request.expect("the settled destination publishes its request");
    assert_eq!(crate::ime::pending_request(&app, &ctx), Some(request));
    let _ = crate::ime::take_request_count(&ctx);
    assert!(ime_frame(&mut app, &ctx, vec![], |_| {}).unwrap().reset_request.is_none(), "an unchanged pass keeps the request");
    assert_eq!(crate::ime::take_request_count(&ctx), 0, "no replacement request on an unchanged frame");
    ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: request }, preedit("が", 1)], |_| {});
    assert_eq!(plain(&app, id), "aがbcdé");
}

/// Marked text left without a recorded composition triggers an interrupting descriptor whose
/// request-bound fence admits only fresh input.
#[test]
fn an_untokened_marked_text_interruption_binds_a_request() {
    let (mut app, ctx, id, _) = interruption_editor();
    // The defensive end-of-pass branch: marked text survives without a recorded composition or a
    // live tool composition (for example a pre-edit accepted before geometry was observed).
    app.ime_marked = Some("x".into());
    let output = ime_frame(&mut app, &ctx, vec![], |_| {}).expect("the Type tool owns the IME");
    assert!(output.should_interrupt_composition, "the tool ended the composition itself");
    let request = output.reset_request.expect("an interruption cannot publish without a request-bound fence");
    assert_eq!(crate::ime::pending_request(&app, &ctx), Some(request));
    assert!(app.ime_marked.is_none());
    ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: request }, preedit("が", 1)], |_| {});
    assert_eq!(plain(&app, id), "abcdが");
}

/// Logic inside a full UI pass preserves the fence; repeated standalone logic keeps it blocked and
/// the first UI return re-arms once.
#[test]
fn full_pass_logic_keeps_its_fence_but_standalone_logic_rearms_once() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let first = pointer_reset(&mut app, &ctx, point);
    let raw = egui::RawInput { screen_rect: Some(screen()), ..Default::default() };
    let mut out = ctx.run_ui(raw.clone(), |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    assert_eq!(crate::ime::pending_request(&app, &ctx), Some(first), "logic inside a full UI pass is not publication loss");
    assert!(out.platform_output.ime.unwrap().reset_request.is_none());
    for _ in 0..3 {
        app.begin_host_iteration();
        let _ = ctx.run_logic(&raw, |ctx| {
            app.tick(ctx);
            app.finish_ime_logic(ctx);
        });
        assert!(crate::ime::blocked(&app, &ctx));
        assert!(crate::ime::pending_request(&app, &ctx).is_none());
    }
    let second = ime_frame(&mut app, &ctx, vec![], |_| {}).unwrap().reset_request.expect("the first returning UI pass rearms once");
    assert_ne!(first, second);
    for _ in 0..3 {
        assert!(ime_frame(&mut app, &ctx, vec![], |_| {}).unwrap().reset_request.is_none());
    }
    ime_frame(
        &mut app,
        &ctx,
        vec![
            Event::ImeResetCompleted { request_id: first },
            Event::Ime(egui::ImeEvent::Commit("X".into())),
            Event::ImeResetCompleted { request_id: second },
            preedit("が", 1),
        ],
        |_| {},
    );
    assert_eq!(plain(&app, id), "aがbcdé");
}

/// An application edit that preserves the caret but changes committed text invalidates the fence
/// before its raw suffix runs.
#[test]
fn same_caret_external_edit_invalidates_a_fence_before_its_raw_suffix() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let first = pointer_reset(&mut app, &ctx, point);
    let caret = app.session.tool_options()["caret"].clone();
    let second = ime_frame(
        &mut app,
        &ctx,
        vec![Event::ImeResetCompleted { request_id: first }, surrounding_delete(false), Event::Ime(egui::ImeEvent::Commit("X".into()))],
        |app| {
            app.run("text.editRange", json!({"id": id, "start": 0, "end": 1, "insert": "A"})).unwrap();
        },
    )
    .unwrap()
    .reset_request
    .expect("the changed committed context gets a new fence");
    assert_ne!(first, second);
    assert_eq!(app.session.tool_options()["caret"], caret, "the caret itself did not change");
    assert_eq!(plain(&app, id), "Abcdé", "F1 cannot admit its suffix into a changed context");
    ime_frame(&mut app, &ctx, vec![Event::ImeResetCompleted { request_id: second }, preedit("が", 1)], |_| {});
    assert_eq!(plain(&app, id), "Aがbcdé");
}

/// An auxiliary viewport observes only its own guard; a replacement egui context rejects the old
/// request and publishes a new one.
#[test]
fn a_replaced_context_and_auxiliary_ui_cannot_reuse_the_root_fence() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let first = pointer_reset(&mut app, &ctx, point);
    let aux = egui::ViewportId::from_hash_of("vectorcraft-ime-mode-aux");
    let mut raw = egui::RawInput { viewport_id: aux, screen_rect: Some(screen()), ..Default::default() };
    raw.viewports.entry(aux).or_default();
    app.begin_host_iteration();
    let mut out = ctx.run_ui(raw, |ui| {
        app.tick(ui.ctx());
        app.finish_ime_logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    assert_eq!(crate::ime::pending_request(&app, &ctx), Some(first), "auxiliary UI observes only its own viewport's mode and guard");
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen()), ..Default::default() }, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    assert_eq!(crate::ime::pending_request(&app, &ctx), Some(first));

    let replacement = egui::Context::default();
    // The real app adopts and restyles this Context. Old F must be rejected even before its
    // first descriptor appears, and the final returned descriptor must carry a new request.
    let first_output = ime_frame(&mut app, &replacement, vec![Event::ImeResetCompleted { request_id: first }, preedit("late", 4)], |_| {});
    assert_eq!(plain(&app, id), "abcdé");
    let second = first_output
        .and_then(|output| output.reset_request)
        .or_else(|| ime_frame(&mut app, &replacement, vec![], |_| {}).and_then(|output| output.reset_request))
        .expect("the restored custom editor publishes a request belonging to the replacement Context");
    assert_ne!(first, second);
    ime_frame(&mut app, &replacement, vec![Event::ImeResetCompleted { request_id: second }, preedit("が", 1)], |_| {});
    assert_eq!(plain(&app, id), "aがbcdé");
}

/// A hidden logic pass neither interrupts an intact composition nor retries acknowledgement; non-
/// empty and old-terminal input stays rejected.
#[test]
fn hidden_logic_preserves_an_uninterrupted_composition_and_no_ack_stays_closed() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    let raw = egui::RawInput { screen_rect: Some(screen()), ..Default::default() };
    app.begin_host_iteration();
    let _ = ctx.run_logic(&raw, |ctx| {
        app.tick(ctx);
        app.finish_ime_logic(ctx);
    });
    assert!(app.session.tool_composing());
    assert!(!crate::ime::blocked(&app, &ctx), "a hidden iteration alone is not a new interruption");
    let request = pointer_reset(&mut app, &ctx, point);
    for _ in 0..4 {
        let output =
            ime_frame(&mut app, &ctx, vec![preedit("late", 4), preedit("", 0), Event::Ime(egui::ImeEvent::Commit("X".into()))], |_| {}).unwrap();
        assert!(output.reset_request.is_none(), "unsupported/no-ack does not retry every idle frame");
        assert_eq!(crate::ime::pending_request(&app, &ctx), Some(request));
        assert_eq!(plain(&app, id), "abcdé", "old terminals and nonempty text never stand in for acknowledgement");
    }
}

/// A real pointer interruption rejects following-batch IME deletes and commits without previewing
/// them, then admits fresh input only after its fence.
#[test]
fn pointer_interruption_rejects_next_batch_ime_mutations_and_allows_fresh_input() {
    for bytes in [false, true] {
        for terminal_pending in [false, true] {
            let (mut app, ctx, id, point) = interruption_editor();
            let view = app.view_info();
            let owner = app.session.tool_ime_target(view);
            ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
            if terminal_pending {
                ime_frame(&mut app, &ctx, vec![preedit("", 0)], |_| {});
            }
            click(&mut app, &ctx, point);
            assert_eq!(app.session.tool_options()["caret"], 1, "the real pointer moved the caret");
            assert_eq!(app.session.tool_ime_target(view), owner, "ordinary caret moves preserve the immutable owner/session");
            let text = if terminal_pending { "abcd" } else { "abcdé" };
            assert_eq!(plain(&app, id), text, "interrupted preview remains at its original position");
            let document = app.session.active().unwrap().doc.clone();
            let steps = undo(&app);
            ime_frame(&mut app, &ctx, vec![surrounding_delete(bytes), Event::Ime(egui::ImeEvent::Commit("X".into()))], |_| {});
            assert_eq!(plain(&app, id), text);
            assert_eq!(app.session.tool_options()["caret"], 1);
            assert!(std::sync::Arc::ptr_eq(&document, &app.session.active().unwrap().doc), "rejection cannot preview a mutation");
            assert_eq!(undo(&app), steps, "rejection creates no undo entry");
            let marker = reset_marker(&app, &ctx);
            ime_frame(&mut app, &ctx, vec![marker, preedit("Q", 1), Event::Ime(egui::ImeEvent::Commit("Q".into()))], |_| {});
            assert_eq!(plain(&app, id), if terminal_pending { "aQbcd" } else { "aQbcdé" });
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(plain(&app, id), text, "only fresh input needs undo");
        }
    }
}

/// Late composition input after clicking another text object cannot mutate either object; the
/// replacement target's fence admits only its own commit.
#[test]
fn target_switch_rejects_late_composition_input_before_mutating_the_new_text() {
    for bytes in [false, true] {
        for terminal_pending in [false, true] {
            let (mut app, ctx, first, _) = interruption_editor();
            let second = app.run("text.create", json!({"x": 300, "y": 250, "text": "WXYZ", "size": 24})).unwrap()["id"].as_u64().unwrap();
            // Creating a text object selects it; return to the first through the real viewer.
            begin_typing(&mut app, &ctx, first);
            ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
            if terminal_pending {
                ime_frame(&mut app, &ctx, vec![preedit("", 0)], |_| {});
            }
            let old_text = plain(&app, first);
            let bounds = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(second)).unwrap().geometric_bounds().unwrap();
            let xf = crate::canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
            click(&mut app, &ctx, xf.to_screen(bounds.center()));
            assert_eq!(app.session.tool_options()["editing"], second, "the pointer edits the replacement target");
            let caret = app.session.tool_options()["caret"].as_u64().unwrap() as usize;
            let document = app.session.active().unwrap().doc.clone();
            let steps = undo(&app);
            ime_frame(&mut app, &ctx, vec![surrounding_delete(bytes), Event::Ime(egui::ImeEvent::Commit("X".into()))], |_| {});
            assert_eq!(plain(&app, first), old_text);
            assert_eq!(plain(&app, second), "WXYZ");
            assert!(std::sync::Arc::ptr_eq(&document, &app.session.active().unwrap().doc));
            assert_eq!(undo(&app), steps);
            // The actual fence, rather than the dropped old terminal, admits the next commit.
            let marker = reset_marker(&app, &ctx);
            ime_frame(&mut app, &ctx, vec![marker, Event::Ime(egui::ImeEvent::Commit("Y".into()))], |_| {});
            let mut expected = "WXYZ".to_string();
            expected.insert(caret, 'Y');
            assert_eq!(plain(&app, second), expected);
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(plain(&app, second), "WXYZ");
        }
    }
}

/// A composition's own delete plus commit, and a non-composing pointer commit, remain accepted
/// single undoable edits.
#[test]
fn valid_composition_delete_commit_and_noncomposing_pointer_commit_stay_accepted() {
    for bytes in [false, true] {
        for terminal_pending in [false, true] {
            let (mut app, ctx, id, _) = interruption_editor();
            ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
            if terminal_pending {
                ime_frame(&mut app, &ctx, vec![preedit("", 0)], |_| {});
            }
            ime_frame(&mut app, &ctx, vec![surrounding_delete(bytes), Event::Ime(egui::ImeEvent::Commit("X".into()))], |_| {});
            assert_eq!(plain(&app, id), "abcX", "the IME's own delete does not interrupt its commit");
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(plain(&app, id), "abcd", "valid replacement is one undo step");
        }
    }
    let (mut app, ctx, id, point) = interruption_editor();
    click(&mut app, &ctx, point);
    assert_eq!(app.session.tool_options()["caret"], 1);
    ime_frame(&mut app, &ctx, vec![Event::Ime(egui::ImeEvent::Commit("Y".into()))], |_| {});
    assert_eq!(plain(&app, id), "aYbcd", "no old composition means no stale guard");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(plain(&app, id), "abcd");
}

/// A stale non-empty pre-edit stays rejected across idle frames until its matching fence; only
/// post-fence input changes text.
#[test]
fn a_stale_nonempty_preedit_stays_rejected_until_its_matching_fence() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    click(&mut app, &ctx, point);
    let text = plain(&app, id);
    let document = app.session.active().unwrap().doc.clone();
    let steps = undo(&app);
    ime_frame(&mut app, &ctx, vec![preedit("late", 4)], |_| {});
    for _ in 0..3 {
        ime_frame(&mut app, &ctx, vec![], |_| {});
    }
    let marker = reset_marker(&app, &ctx);
    ime_frame(&mut app, &ctx, vec![Event::Ime(egui::ImeEvent::Commit("X".into())), marker, preedit("Q", 1)], |_| {});
    // Old input did not change committed text; only Q after the explicit fence did.
    assert_eq!(plain(&app, id), "aQbcdé");
    assert_eq!(undo(&app), steps);
    assert!(!std::sync::Arc::ptr_eq(&document, &app.session.active().unwrap().doc), "the fresh preview is a real model change");
    ime_frame(&mut app, &ctx, vec![Event::Ime(egui::ImeEvent::Commit("Q".into()))], |_| {});
    assert_eq!(plain(&app, id), "aQbcdé");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(plain(&app, id), text);
}

/// An application caret change settles the preview at its old position and rejects old input
/// without changing the document.
#[test]
fn an_external_caret_change_keeps_preview_at_its_old_position_and_rejects_old_input() {
    let (mut app, ctx, id, _) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
    app.session.set_tool_option("select", &json!({"start": 1, "end": 1}));
    ime_frame(&mut app, &ctx, vec![], |_| {});
    assert_eq!(plain(&app, id), "abcdé", "settling the old preview must not reinsert it at caret1");
    assert_eq!(app.session.tool_options()["caret"], 1);
    let document = app.session.active().unwrap().doc.clone();
    let steps = undo(&app);
    ime_frame(&mut app, &ctx, vec![surrounding_delete(false), Event::Ime(egui::ImeEvent::Commit("X".into()))], |_| {});
    assert_eq!(plain(&app, id), "abcdé");
    assert_eq!(app.session.tool_options()["caret"], 1);
    assert!(std::sync::Arc::ptr_eq(&document, &app.session.active().unwrap().doc));
    assert_eq!(undo(&app), steps);
}

/// An application edit while typing makes the payload and following commit `Other` and publishes
/// the document's current text, not the stale session.
#[test]
fn external_edit_while_typing_is_other_and_ime_commit_uses_the_document_text() {
    let (mut app, id) = app_with_text("Same");
    let ctx = egui::Context::default();
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![], |_| {});
    begin_typing(&mut app, &ctx, id);
    ime_frame(&mut app, &ctx, vec![Event::Text("x".into())], |_| {});
    let typed = plain(&app, id);
    assert!(typed.contains('x'), "{typed}");

    // A control/menu/panel edit between passes (here inside the pass, as `tick` drains it): the
    // typing session's runs are now stale. The outgoing payload must describe the document, not
    // the stale session.
    let edited = ime_frame(&mut app, &ctx, vec![], |app| {
        app.run("text.editRange", json!({"id": id, "start": 0, "end": typed.len(), "insert": "XYZ"})).unwrap();
    })
    .expect("the Type tool still owns the IME");
    assert_eq!(plain(&app, id), "XYZ");
    assert_eq!(edited.change_cause, egui::output::ImeChangeCause::Other);
    let payload = edited.surrounding_text.available().expect("the document text is available");
    assert_eq!(payload.text.as_str(), "XYZ", "the stale session text is never published");

    // The commit replaces the stale session's text as an application-side reset; its receipt
    // cannot continue the observed baseline, so the change is `Other`, never `InputMethod`.
    let committed = ime_frame(&mut app, &ctx, vec![Event::Ime(egui::ImeEvent::Commit("y".into()))], |_| {}).expect("still editing");
    assert_eq!(plain(&app, id), "XYZy");
    assert_eq!(committed.change_cause, egui::output::ImeChangeCause::Other);
    let payload = committed.surrounding_text.available().expect("the committed text is available");
    assert_eq!((payload.text.as_str(), payload.cursor, payload.anchor), ("XYZy", 4, 4));
}

/// A dialog opened after the canvas takes IME ownership away from the Type tool on that pass.
#[test]
fn a_dialog_opened_after_the_canvas_takes_the_ime_away_from_the_type_tool() {
    let (mut app, id) = app_with_text("Same");
    let ctx = egui::Context::default();
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![], |_| {});
    begin_typing(&mut app, &ctx, id);
    let owner = egui::Id::unique(("vectorcraft-type-ime", app.session.active().unwrap().uid, id));
    let before = ime_frame(&mut app, &ctx, vec![], |_| {}).expect("the Type tool owns the IME");
    assert_eq!(before.owner, owner);
    // A queued dialog opens in `dialogs::show`, after the canvas drew: a producer that observed
    // during the canvas would still publish the Type tool as the owner, though the modal now
    // covers it.
    let during = ime_frame(&mut app, &ctx, vec![], |app| {
        app.ui.told.push(crate::state::Dialog::new(crate::dialogs::confirm::MESSAGE, json!({"message": "A message", "detail": ""})));
    });
    assert!(app.ui.dialog.is_some(), "the dialog opened after the canvas drew");
    assert!(during.is_none(), "the canvas must not own the IME while a dialog opened after it: {during:?}");
}

/// Home removes the Type tool owner, clears marked text and asks the system IME to discard its
/// composition.
#[test]
fn home_takes_down_a_live_composition() {
    let (mut app, id) = app_with_text("Same");
    let ctx = egui::Context::default();
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![], |_| {});
    begin_typing(&mut app, &ctx, id);
    let composing = ime_frame(&mut app, &ctx, vec![preedit("が", 1)], |_| {}).expect("the composition has an owner");
    assert!(composing.surrounding_text.available().is_some());
    assert!(app.ime_marked.is_some());
    app.run("app.home", json!({})).unwrap();
    let home = ime_frame(&mut app, &ctx, vec![], |_| {});
    assert!(home.is_none(), "Home has no IME owner");
    assert!(app.ime_marked.is_none(), "Home takes the marked text down");
    assert!(app.take_ime_discard(), "the system IME is told to drop its marked text");
    assert!(!app.session.tool_composing(), "no stale composition survives Home");
}

/// Closing the active document removes the IME owner, clears marked text and asks the system IME to
/// discard its composition.
#[test]
fn closing_the_document_takes_down_a_live_composition() {
    let (mut app, id) = app_with_text("Same");
    let ctx = egui::Context::default();
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![], |_| {});
    begin_typing(&mut app, &ctx, id);
    ime_frame(&mut app, &ctx, vec![preedit("が", 1)], |_| {});
    assert!(app.ime_marked.is_some());
    assert!(app.session.close_document(0), "the document closes");
    let closed = ime_frame(&mut app, &ctx, vec![], |_| {});
    assert!(closed.is_none(), "no document, no IME owner");
    assert!(app.ime_marked.is_none(), "closing takes the marked text down");
    assert!(app.take_ime_discard(), "the system IME is told to drop its marked text");
}

/// A commit whose text the document rewrites through smart quotes or control filtering is reported
/// as `Other`, not `InputMethod`.
#[test]
fn an_ime_commit_the_document_rewrites_is_other() {
    let (mut app, id) = app_with_text("Same");
    let ctx = egui::Context::default();
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![], |_| {});
    begin_typing(&mut app, &ctx, id);
    // Typographer's Quotes is on by default: the straight quote the IME accepted is rewritten by
    // the document, so the change is application-origin.
    let quoted = ime_frame(&mut app, &ctx, vec![Event::Ime(egui::ImeEvent::Commit("\"".into()))], |_| {}).expect("still editing");
    let text = plain(&app, id);
    assert!(text.contains('”'), "the document rewrote the quote: {text}");
    assert_eq!(quoted.change_cause, egui::output::ImeChangeCause::Other, "a rewritten commit is not InputMethod");

    // A filtered control character is another application transformation of the same commit.
    let filtered = ime_frame(&mut app, &ctx, vec![Event::Ime(egui::ImeEvent::Commit("a\u{7}b".into()))], |_| {}).expect("still editing");
    let text = plain(&app, id);
    assert!(text.contains("ab") && !text.contains('\u{7}'), "the control character is filtered: {text}");
    assert_eq!(filtered.change_cause, egui::output::ImeChangeCause::Other, "filtering is not an IME insertion");
}

/// Smart-quote replacement examines the character before the pre-edit, so the rewritten commit is
/// reported as `Other`.
#[test]
fn an_ime_quote_commit_uses_the_character_before_its_pre_edit() {
    let (mut app, id) = app_with_text("a");
    // The pre-edit ends in an opening quote, but replacement starts after the letter `a`.
    app.run("document.setup", json!({"quotes": {"double": ["\"", "”"]}})).unwrap();
    let ctx = egui::Context::default();
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![], |_| {});
    begin_typing(&mut app, &ctx, id);
    app.session.set_tool_option("select", &json!({"start": 1, "end": 1}));
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![preedit("\"", 1)], |_| {});
    let committed = ime_frame(&mut app, &ctx, vec![Event::Ime(egui::ImeEvent::Commit("\"".into()))], |_| {}).expect("still editing");
    assert_eq!(plain(&app, id), "a”", "the document inserts its closing quote");
    assert_eq!(committed.change_cause, egui::output::ImeChangeCause::Other, "a rewritten quote is not an unchanged IME insertion");
}

/// An outside command can end the typing session (`endTyping`) while the Type tool still owns
/// the IME. The IME's following `DeleteSurrounding` starts a new typing session; that session
/// must clear the ended marker, so after its commit and a caret move the next ordinary IME
/// commit is an accepted `InputMethod` mutation, not an application edit.
#[test]
fn ime_delete_surrounding_after_end_typing_keeps_the_next_commit_input_method() {
    let (mut app, id) = app_with_text("abc");
    let ctx = egui::Context::default();
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![], |_| {});
    begin_typing(&mut app, &ctx, id);
    app.session.set_tool_option("select", &json!({"start": 3, "end": 3}));
    ime_frame(&mut app, &ctx, vec![], |_| {});
    // A real typing session, settled by an outside command as `settle_interaction` does.
    ime_frame(&mut app, &ctx, vec![Event::Text("x".into())], |_| {}).expect("typing owns the IME");
    assert_eq!(plain(&app, id), "abcx");
    ime_frame(&mut app, &ctx, vec![], |app| {
        app.session.commit_interaction().expect("the typing preview is committed");
        app.session.set_tool_option("endTyping", &json!(true));
    });
    // The IME deletes the typed character and commits its replacement in the new session.
    let deleted = ime_frame(
        &mut app,
        &ctx,
        vec![Event::Ime(egui::ImeEvent::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }), Event::Ime(egui::ImeEvent::Commit("Z".into()))],
        |_| {},
    )
    .expect("the Type tool still owns the IME");
    assert_eq!(plain(&app, id), "abcZ");
    assert_eq!(deleted.change_cause, egui::output::ImeChangeCause::InputMethod);
    // A caret move ends the deletion's session. The following IME commit starts a fresh session;
    // because no outside command ended that session, the commit is InputMethod.
    let left = egui::Event::Key { key: egui::Key::ArrowLeft, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    ime_frame(&mut app, &ctx, vec![left], |_| {});
    let committed =
        ime_frame(&mut app, &ctx, vec![Event::Ime(egui::ImeEvent::Commit("Q".into()))], |_| {}).expect("the Type tool still owns the IME");
    assert_eq!(plain(&app, id), "abcQZ");
    assert_eq!(committed.change_cause, egui::output::ImeChangeCause::InputMethod, "a caret move is not an application-side reset");
}

/// Reloading a document keeps its uid and text-node id but changes the edit-session replacement
/// epoch.
#[test]
fn a_reloaded_document_gets_a_new_ime_replacement_epoch() {
    let (mut app, id) = app_with_text("Same");
    let ctx = egui::Context::default();
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![], |_| {});
    begin_typing(&mut app, &ctx, id);
    ime_frame(&mut app, &ctx, vec![], |_| {});
    let view = app.view_info();
    let first = app.session.tool_ime_target(view).expect("editing has a target");
    assert_eq!(first.0, app.session.active().unwrap().uid);
    assert_eq!(first.1, id);
    // File › Revert: the document is replaced in its tab, keeping its uid and node ids, and the
    // Type tool is recreated.
    let doc = app.session.active().unwrap().doc.clone();
    assert!(app.session.replace_document(0, (*doc).clone()));
    ime_frame(&mut app, &ctx, vec![], |_| {});
    let center = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().center();
    let xf = crate::canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap());
    click(&mut app, &ctx, xf.to_screen(center));
    let second = app.session.tool_ime_target(view).expect("re-entering has a target");
    assert_eq!((second.0, second.1), (first.0, first.1), "the same document uid and text node");
    assert_ne!(second.2, first.2, "the replacement epoch must not be reusable after a reload");
}

/// Frame keys compare the whole (context, viewport, frame) set, so root frame 0 → auxiliary frame
/// 0 → root rerun frame 0 does not deliver the root frame twice.
#[test]
fn frame_keys_are_exact_once_across_interleaved_viewports() {
    let aux = egui::ViewportId::from_hash_of("vectorcraft-lifecycle-aux");
    let mut seen = vec![];
    assert!(crate::take_frame_once(&mut seen, (1, egui::ViewportId::ROOT, 0)));
    assert!(crate::take_frame_once(&mut seen, (1, aux, 0)));
    assert!(!crate::take_frame_once(&mut seen, (1, egui::ViewportId::ROOT, 0)), "root's rerun is a duplicate");
    assert!(!crate::take_frame_once(&mut seen, (1, aux, 0)), "the auxiliary pass is a duplicate too");
}

/// The production `ui` path (not `shortcuts::handle` directly) takes input once for each viewport
/// in one host iteration: two Tabs from two viewports toggle the panels twice, and the root
/// viewport's discarded rerun toggles them no third time.
#[test]
fn two_viewports_take_their_input_once_through_the_ui_path() {
    let mut app = app();
    let ctx = egui::Context::default();
    let aux = egui::ViewportId::from_hash_of("vectorcraft-lifecycle-input-aux");
    let screen = screen();
    let tab = Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let run = |app: &mut VectorcraftApp, viewport: egui::ViewportId, events: Vec<Event>, discard: bool| {
        let mut raw = egui::RawInput { events, screen_rect: Some(screen), viewport_id: viewport, ..Default::default() };
        raw.viewports.entry(viewport).or_default();
        app.begin_host_iteration();
        let mut first = true;
        let mut passes = 0;
        let mut out = ctx.run_ui(raw, |ui| {
            passes += 1;
            app.tick(ui.ctx());
            app.ui(ui);
            if discard && std::mem::replace(&mut first, false) {
                ui.ctx().request_discard("interleaved viewports");
            }
        });
        out.textures_delta.clear();
        passes
    };
    // Fonts and a drawn frame for each viewport before the input frame.
    for viewport in [aux, egui::ViewportId::ROOT] {
        for _ in 0..2 {
            run(&mut app, viewport, vec![], false);
        }
    }
    let shown = app.ui.dock;
    assert!(shown, "the panels start shown");
    // One host iteration: the auxiliary viewport first, then the root viewport with a discarded
    // rerun. Both viewports must take their Tab; the rerun must not take the root's again.
    let mut raw = egui::RawInput { events: vec![tab.clone()], screen_rect: Some(screen), viewport_id: aux, ..Default::default() };
    raw.viewports.entry(aux).or_default();
    app.begin_host_iteration();
    let mut out = ctx.run_ui(raw, |ui| {
        app.tick(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    let mut raw = egui::RawInput { events: vec![tab], screen_rect: Some(screen), ..Default::default() };
    raw.viewports.entry(egui::ViewportId::ROOT).or_default();
    let mut first = true;
    let mut passes = 0;
    let mut out = ctx.run_ui(raw, |ui| {
        passes += 1;
        app.tick(ui.ctx());
        app.ui(ui);
        if std::mem::replace(&mut first, false) {
            ui.ctx().request_discard("interleaved viewports");
        }
    });
    out.textures_delta.clear();
    assert!(passes >= 2, "the root reran after the discard, got {passes} passes");
    assert_eq!(app.ui.dock, shown, "two viewports took their Tab exactly once; the rerun took none");
}

/// A minimized window's `run_logic` sees egui's last shown `i.time` (the same value forever here).
/// Background polling must still advance on the monotonic host clock.
#[test]
fn a_hidden_tick_advances_on_monotonic_time_not_the_stale_pass_clock() {
    let mut app = app();
    let ctx = egui::Context::default();
    let stale = || egui::RawInput { time: Some(0.0), screen_rect: Some(screen()), ..Default::default() };
    app.begin_host_iteration();
    let _ = ctx.run_logic(&stale(), |ctx| app.tick(ctx));
    let first = app.system_paste_at;
    std::thread::sleep(std::time::Duration::from_millis(300));
    app.begin_host_iteration();
    let _ = ctx.run_logic(&stale(), |ctx| app.tick(ctx));
    assert!(app.system_paste_at > first, "the clipboard poll used the replayed i.time: {first} → {}", app.system_paste_at);
}

/// A new graphics context starts a new IME context generation: the old native IME context must not
/// be treated as the new owner's.
#[test]
fn a_new_graphics_context_invalidates_the_ime_context() {
    let (mut app, id) = app_with_text("Same");
    let first_ctx = egui::Context::default();
    ime_frame(&mut app, &first_ctx, vec![], |_| {});
    ime_frame(&mut app, &first_ctx, vec![], |_| {});
    begin_typing(&mut app, &first_ctx, id);
    let before = ime_frame(&mut app, &first_ctx, vec![], |_| {}).expect("the Type tool owns the IME");
    let second_ctx = egui::Context::default();
    // The first pass installs the new context's fonts; the second draws and publishes the IME.
    let _ = ime_frame(&mut app, &second_ctx, vec![], |_| {});
    let after = ime_frame(&mut app, &second_ctx, vec![], |_| {}).expect("the Type tool still owns the IME");
    assert_ne!(before.context_generation, after.context_generation, "a new context needs a new IME generation");
    assert_eq!(before.owner, after.owner, "the same Type tool owner, in a new context");
}

/// An unavailable surrounding-text query (a selection wider than the 4,000-byte limit) still carries
/// a document revision, so an application edit between two unavailable boundaries is detectable.
#[test]
fn an_unavailable_ime_boundary_carries_the_document_revision() {
    let (mut app, id) = app_with_text("Same");
    let ctx = egui::Context::default();
    ime_frame(&mut app, &ctx, vec![], |_| {});
    ime_frame(&mut app, &ctx, vec![], |_| {});
    begin_typing(&mut app, &ctx, id);
    // Grow the edited text past the 4,000-byte surrounding-text limit while keeping the typing
    // session alive, then select all of it.
    let long = "x".repeat(5_000);
    app.run("text.editRange", json!({"id": id, "start": 0, "end": 4, "insert": long})).unwrap();
    app.run("tool.setOption", json!({"key": "selectAll", "value": true})).unwrap();
    let view = app.view_info();
    let before = crate::ime::stamp(&mut app.session, view).expect("the Type tool is editing");
    assert!(before.surrounding_text().available().is_none(), "the 5,000-byte selection is unavailable");
    let revision = before.unavailable_revision().expect("an unavailable stamp carries a revision");
    app.run("text.editRange", json!({"id": id, "start": 0, "end": 1, "insert": "y"})).unwrap();
    let after = crate::ime::stamp(&mut app.session, view).expect("still editing after the application edit");
    assert_ne!(before, after, "an application edit changes the unavailable boundary");
    assert!(after.unavailable_revision().is_some_and(|after| after > revision), "the document revision advanced");
}

/// Build a primary-pointer button event at `at` with the requested press state.
fn primary_button(at: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// Build a left-arrow key event marked as a physical key press or release.
fn left_key(pressed: bool) -> Event {
    Event::Key { key: egui::Key::ArrowLeft, physical_key: Some(egui::Key::ArrowLeft), pressed, repeat: false, modifiers: Default::default() }
}

/// The press and old terminal arrive in one raw batch, rather than finishing the pointer frame
/// first. The guard must run at the press's position before the queued old mutations execute.
#[test]
fn a_pointer_before_same_batch_terminal_input_preserves_the_interrupted_text() {
    for bytes in [false, true] {
        for terminal_pending in [false, true] {
            let (mut app, ctx, id, point) = interruption_editor();
            ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
            if terminal_pending {
                ime_frame(&mut app, &ctx, vec![preedit("", 0)], |_| {});
            }
            let expected = if terminal_pending { "abcd" } else { "abcdé" };
            let document = app.session.active().unwrap().doc.clone();
            let steps = undo(&app);
            let view = app.view_info();
            let owner = app.session.tool_ime_target(view);
            let interrupted = ime_frame(
                &mut app,
                &ctx,
                vec![
                    Event::PointerMoved(point),
                    primary_button(point, true),
                    surrounding_delete(bytes),
                    Event::Ime(egui::ImeEvent::Commit("X".into())),
                ],
                |_| {},
            )
            .expect("the pointer destination still edits text");
            assert_eq!(plain(&app, id), expected, "old same-batch input cannot delete or insert at either caret");
            assert_eq!(app.session.tool_options()["caret"], 1, "the retained real pointer still moves the caret");
            assert_eq!(app.session.tool_ime_target(view), owner);
            assert!(std::sync::Arc::ptr_eq(&document, &app.session.active().unwrap().doc), "reject before previewing an old mutation");
            assert_eq!(undo(&app), steps + usize::from(!terminal_pending), "only settling an existing live preview commits a step");
            let fence = interrupted.reset_request.expect("the retained pointer interruption publishes its replacement fence");
            assert_eq!(crate::ime::pending_request(&app, &ctx), Some(fence));
            ime_frame(&mut app, &ctx, vec![primary_button(point, false)], |_| {});
            // The guard must still fence fresh-looking input until the native reset completion.
            ime_frame(&mut app, &ctx, vec![preedit("Q", 1), Event::Ime(egui::ImeEvent::Commit("Q".into()))], |_| {});
            assert_eq!(plain(&app, id), expected, "input before the reset ack stays stale");
            ime_frame(
                &mut app,
                &ctx,
                vec![Event::ImeResetCompleted { request_id: fence }, preedit("Q", 1), Event::Ime(egui::ImeEvent::Commit("Q".into()))],
                |_| {},
            );
            assert_eq!(plain(&app, id), if terminal_pending { "aQbcd" } else { "aQbcdé" });
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(plain(&app, id), expected, "fresh composition after rejection remains one undoable edit");
        }
    }
}

/// Application navigation before a same-batch pending terminal is applied once; the old
/// delete/commit suffix is rejected and only input after the fence is admitted.
#[test]
fn application_navigation_before_same_batch_pending_terminal_rejects_its_mutations() {
    for bytes in [false, true] {
        let (mut app, ctx, id, _) = interruption_editor();
        ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
        ime_frame(&mut app, &ctx, vec![preedit("", 0)], |_| {});
        let document = app.session.active().unwrap().doc.clone();
        let steps = undo(&app);
        ime_frame(&mut app, &ctx, vec![left_key(true), surrounding_delete(bytes), Event::Ime(egui::ImeEvent::Commit("X".into()))], |_| {});
        assert_eq!(plain(&app, id), "abcd");
        assert_eq!(app.session.tool_options()["caret"], 3, "the application navigation is not consumed by the marker");
        assert!(std::sync::Arc::ptr_eq(&document, &app.session.active().unwrap().doc));
        assert_eq!(undo(&app), steps, "rejected input cannot create an Undo edit");
        let marker = reset_marker(&app, &ctx);
        ime_frame(&mut app, &ctx, vec![left_key(false), marker, Event::Ime(egui::ImeEvent::Commit("Y".into()))], |_| {});
        assert_eq!(plain(&app, id), "abcYd", "a later direct commit stays valid");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(plain(&app, id), "abcd");
    }
}

/// A later press must not retroactively interrupt an earlier valid terminal. The initial move
/// also precedes the terminal while the final batch state is held: only the actual press starts
/// the pointer interruption, not InputState's final held flag.
#[test]
fn terminal_before_a_same_batch_pointer_is_accepted_in_both_composition_states() {
    for bytes in [false, true] {
        for terminal_pending in [false, true] {
            let (mut app, ctx, id, point) = interruption_editor();
            ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
            if terminal_pending {
                ime_frame(&mut app, &ctx, vec![preedit("", 0)], |_| {});
            }
            ime_frame(
                &mut app,
                &ctx,
                vec![
                    Event::PointerMoved(point),
                    surrounding_delete(bytes),
                    Event::Ime(egui::ImeEvent::Commit("X".into())),
                    primary_button(point, true),
                ],
                |_| {},
            );
            assert_eq!(plain(&app, id), "abcX", "a later press cannot cancel an earlier terminal");
            assert_eq!(app.session.tool_options()["caret"], 1);
            ime_frame(&mut app, &ctx, vec![primary_button(point, false)], |_| {});
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(plain(&app, id), "abcd", "the accepted replacement remains one Undo edit");
        }
    }
}

/// A terminal followed by application navigation keeps the valid commit and runs the navigation; an
/// active IME's own navigation does not interrupt it.
#[test]
fn terminal_before_navigation_is_accepted_and_active_ime_navigation_stays_owned() {
    for bytes in [false, true] {
        let (mut app, ctx, id, _) = interruption_editor();
        ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
        ime_frame(&mut app, &ctx, vec![preedit("", 0)], |_| {});
        ime_frame(&mut app, &ctx, vec![surrounding_delete(bytes), Event::Ime(egui::ImeEvent::Commit("X".into())), left_key(true)], |_| {});
        assert_eq!(plain(&app, id), "abcX");
        assert_eq!(app.session.tool_options()["caret"], 3, "a later application navigation still executes");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(plain(&app, id), "abcd");

        let (mut app, ctx, id, _) = interruption_editor();
        ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
        ime_frame(&mut app, &ctx, vec![left_key(true), surrounding_delete(bytes), Event::Ime(egui::ImeEvent::Commit("X".into()))], |_| {});
        assert_eq!(plain(&app, id), "abcX", "an active IME owns its arrow; it does not interrupt the valid terminal");
        assert_eq!(app.session.tool_options()["caret"], 4);
    }
}

/// A pointer marker without an observed old composition does not reject a direct commit; terminal
/// text still executes before pointer placement.
#[test]
fn a_pointer_marker_without_an_old_composition_does_not_reject_a_direct_commit() {
    let (mut app, ctx, id, point) = interruption_editor();
    ime_frame(&mut app, &ctx, vec![Event::PointerMoved(point), primary_button(point, true), Event::Ime(egui::ImeEvent::Commit("Y".into()))], |_| {});
    // Keep the existing dispatch contract: terminal text executes before canvas pointer placement.
    assert_eq!(plain(&app, id), "abcdY", "no observed old composition means no stale rejection");
    assert_eq!(app.session.tool_options()["caret"], 1);
    ime_frame(&mut app, &ctx, vec![primary_button(point, false)], |_| {});
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(plain(&app, id), "abcd");
}

/// A move within an already-held gesture before a same-batch terminal interrupts the old
/// composition; the retained move reaches the tool and only the live preview commits.
#[test]
fn a_held_pointer_move_before_same_batch_terminal_also_interrupts_the_old_composition() {
    for bytes in [false, true] {
        let (mut app, ctx, id, point) = interruption_editor();
        let start = ime_frame(&mut app, &ctx, vec![], |_| {}).expect("caret4 is drawn").cursor_rect.center();
        ime_frame(&mut app, &ctx, vec![Event::PointerMoved(start), primary_button(start, true)], |_| {});
        ime_frame(&mut app, &ctx, vec![preedit("é", 1)], |_| {});
        let document = app.session.active().unwrap().doc.clone();
        let steps = undo(&app);
        ime_frame(
            &mut app,
            &ctx,
            vec![Event::PointerMoved(point), surrounding_delete(bytes), Event::Ime(egui::ImeEvent::Commit("X".into()))],
            |_| {},
        );
        assert_eq!(plain(&app, id), "abcdé", "a move in an already-held gesture precedes the old terminal");
        assert_eq!(app.session.tool_options()["caret"], 1, "the retained real move reaches the tool");
        assert!(std::sync::Arc::ptr_eq(&document, &app.session.active().unwrap().doc));
        assert_eq!(undo(&app), steps + 1, "only the interrupted live preview commits");
        ime_frame(&mut app, &ctx, vec![primary_button(point, false)], |_| {});
    }
}
