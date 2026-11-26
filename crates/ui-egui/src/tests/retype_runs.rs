//! Retype in the app ([`crate::retype_runs`]): Type › Retype and the Retype panel's Find Matching
//! Fonts search the installed fonts off the UI thread, then run their command, unless the document
//! or its selection changed meanwhile; Cancel stops the search and changes nothing; its progress
//! window shows it.

use serde_json::json;
use vectorcraft_engine::{EngineError, Session};

use crate::export_progress::{self, DELAY};
use crate::{VectorcraftApp, retype_runs};

/// An app whose searches run off the UI thread, with "Retype me" set in Source Sans 3, outlined and
/// selected → the app and the outlines' group.
fn outlined() -> (VectorcraftApp, u64) {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let id = app.session.execute("text.create", &json!({"x": 40, "y": 100, "text": "Retype me", "size": 30, "font": "Source Sans 3"})).unwrap()["id"]
        .as_u64()
        .unwrap();
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    let g = app.session.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
    app.session.execute("select.set", &json!({"ids": [g]})).unwrap();
    app.retype_runs.off_the_ui_thread = true;
    (app, g)
}

fn undos(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

fn has_type(app: &VectorcraftApp) -> bool {
    let mut found = false;
    app.session.doc().unwrap().doc.walk(|n| found |= matches!(n.kind, vectorcraft_doc::NodeKind::Text(_)));
    found
}

/// Draws the progress window at `time` seconds in a frame as the hosts run it → the texts it drew.
fn progress(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64) -> String {
    let out = crate::testing::part_frame(app, ctx, egui::RawInput { time: Some(time), ..Default::default() }, export_progress::show);
    let mut texts = String::new();
    for shape in out.shapes {
        if let egui::epaint::Shape::Text(t) = shape.shape {
            texts.push_str(t.galley.text());
            texts.push('\n');
        }
    }
    texts
}

#[test]
fn type_retype_searches_off_the_ui_thread_then_retypes_as_one_undo_step() {
    let (mut app, _) = outlined();
    let before = undos(&app);
    crate::menus::invoke(&mut app, "type.retype", json!({}));
    assert!(app.retype_runs.running.is_some(), "the search runs in the background");
    assert_eq!(app.ui.status, "Finding matching fonts…");
    assert_eq!(undos(&app), before, "nothing yet");
    retype_runs::wait(&mut app);
    assert!(app.retype_runs.running.is_none());
    assert_eq!(undos(&app), before + 1, "one undo step: {}", app.ui.status);
    assert!(has_type(&app), "the outlines are type again");
}

/// Starting a search asks for a frame, so its result is picked up without input.
#[test]
fn starting_a_search_asks_for_a_frame() {
    let (mut app, _) = outlined();
    let ctx = crate::testing::ui_context();
    for _ in 0..5 {
        crate::testing::part_frame(&mut app, &ctx, Default::default(), |_, _| {});
        if !ctx.has_requested_repaint() {
            break;
        }
    }
    assert!(!ctx.has_requested_repaint(), "the context settled");
    crate::menus::invoke(&mut app, "type.retype", json!({}));
    assert!(app.retype_runs.running.is_some());
    assert!(ctx.has_requested_repaint());
    retype_runs::wait(&mut app);
}

#[test]
fn cancel_changes_nothing() {
    let (mut app, _) = outlined();
    let before = undos(&app);
    crate::menus::invoke(&mut app, "type.retype", json!({}));
    assert_eq!(app.run("retype.cancelSearch", json!({})).unwrap(), json!({"cancelled": 1}));
    assert_eq!(app.run("retype.cancelSearch", json!({})).unwrap(), json!({"cancelled": 0}), "already canceled");
    retype_runs::wait(&mut app);
    assert_eq!(undos(&app), before, "canceled: nothing changes, even when the search ended first");
    assert!(!has_type(&app));
    assert_eq!(app.ui.status, EngineError::Cancelled.to_string());
}

/// The progress window opens after its delay with Cancel and no Continue in Background; its Cancel
/// stops the search, which then changes nothing.
#[test]
fn the_progress_window_shows_a_search() {
    let (mut app, _) = outlined();
    let ctx = crate::testing::ui_context();
    let before = undos(&app);
    crate::menus::invoke(&mut app, "type.retype", json!({}));
    assert!(app.retype_runs.running.is_some());
    assert!(!progress(&mut app, &ctx, 0.0).contains("Finding matching fonts"), "not before the delay");
    // The delay counts on the monotonic clock: the search started that long ago.
    if let Some(r) = app.retype_runs.running.as_mut() {
        r.started -= DELAY;
    }
    progress(&mut app, &ctx, DELAY + 0.05); // a new window measures itself unseen first
    let shown = progress(&mut app, &ctx, DELAY + 0.1);
    assert!(shown.contains("Finding matching fonts…") && shown.contains("Reading the installed fonts…"), "{shown}");
    assert!(shown.contains("Cancel") && !shown.contains("Continue in Background"), "{shown}");
    assert_eq!(app.run("retype.cancelSearch", json!({})).unwrap(), json!({"cancelled": 1}));
    assert!(!progress(&mut app, &ctx, DELAY + 1.0).contains("Finding matching fonts"), "a canceled search isn't shown");
    retype_runs::wait(&mut app);
    assert_eq!(undos(&app), before, "canceled: nothing changes");
}

#[test]
fn a_search_whose_selection_changed_meanwhile_changes_nothing() {
    let (mut app, g) = outlined();
    let before = undos(&app);
    crate::menus::invoke(&mut app, "type.retype", json!({}));
    app.session.execute("select.set", &json!({"ids": []})).unwrap();
    app.session.execute("select.set", &json!({"ids": [g]})).unwrap();
    // The same objects selected again: it still applies.
    retype_runs::wait(&mut app);
    assert_eq!(undos(&app), before + 1);
    // Another selection when it is done: nothing changes.
    app.run("edit.undo", json!({})).unwrap();
    app.session.execute("select.set", &json!({"ids": [g]})).unwrap();
    let before = undos(&app);
    crate::menus::invoke(&mut app, "type.retype", json!({}));
    app.session.execute("select.set", &json!({"ids": []})).unwrap();
    retype_runs::wait(&mut app);
    assert_eq!(undos(&app), before);
    assert_eq!(app.ui.status, "Retype finished, but the document or its selection changed meanwhile: run it again");
}

#[test]
fn find_matching_fonts_lists_the_fonts_once_the_search_is_done() {
    let (mut app, _) = outlined();
    let r = retype_runs::start(&mut app, "type.retypeFonts", json!({"count": 96}), true).unwrap();
    assert_eq!(r["background"], json!(true), "{r}");
    assert!(!app.ui.retype.searched, "nothing listed yet");
    retype_runs::wait(&mut app);
    assert!(app.ui.retype.searched);
    // Other tests load renamed copies of the bundled fonts, which tie with it: it may be among the
    // fonts Find more font matches adds.
    let v = &app.ui.retype;
    assert!(!v.fonts.is_empty());
    assert!(v.fonts.iter().chain(&v.rest).any(|f| f.family == "Source Sans 3"), "{:?} {:?}", v.fonts, v.rest);
    assert!(crate::panel_docking::showing(&app.ui, crate::panels::retype::ID), "the panel shows");
    // From a menu or the command palette, retype.find searches in the background too; with the
    // installed fonts left out it lists at once.
    app.run("retype.reset", json!({})).unwrap();
    crate::menus::invoke(&mut app, "retype.find", json!({}));
    assert!(app.retype_runs.running.is_some() && !app.ui.retype.searched);
    retype_runs::wait(&mut app);
    assert!(app.ui.retype.searched);
    // Find more font matches before any search is a first search, in the background too.
    app.run("retype.reset", json!({})).unwrap();
    crate::menus::invoke(&mut app, "retype.find", json!({"more": true}));
    assert!(app.retype_runs.running.is_some() && !app.ui.retype.searched);
    retype_runs::wait(&mut app);
    assert!(app.ui.retype.searched);
    // After a search it adds to the list at once.
    crate::menus::invoke(&mut app, "retype.find", json!({"more": true}));
    assert!(app.retype_runs.running.is_none());
    app.run("retype.reset", json!({})).unwrap();
    crate::menus::invoke(&mut app, "retype.find", json!({"installed": false}));
    assert!(app.retype_runs.running.is_none() && app.ui.retype.searched);
}

#[test]
fn reset_retype_panel_stops_the_panel_s_search() {
    let (mut app, _) = outlined();
    retype_runs::start(&mut app, "type.retypeFonts", json!({"count": 96}), true).unwrap();
    app.run("retype.reset", json!({})).unwrap();
    assert!(app.retype_runs.running.is_none(), "the search is stopped");
    retype_runs::wait(&mut app);
    assert!(!app.ui.retype.searched, "the list stays cleared");
    assert!(app.ui.status.is_empty(), "{}", app.ui.status);
    // A new search the command runs at once stops it too, so it does not replace the new list.
    retype_runs::start(&mut app, "type.retypeFonts", json!({"count": 96}), true).unwrap();
    let listed = app.run("retype.find", json!({"installed": false})).unwrap();
    assert!(app.retype_runs.running.is_none(), "the older search is stopped");
    retype_runs::wait(&mut app);
    assert_eq!(app.run("retype.list", json!({})).unwrap(), listed);
    // Retype's own search goes on.
    crate::menus::invoke(&mut app, "type.retype", json!({}));
    app.run("retype.reset", json!({})).unwrap();
    assert!(app.retype_runs.running.is_some());
    retype_runs::wait(&mut app);
}

#[test]
fn without_a_thread_or_with_the_installed_fonts_left_out_the_command_runs_at_once() {
    let (mut app, _) = outlined();
    let r = retype_runs::start(&mut app, "type.retype", json!({"installed": false}), false).unwrap();
    assert!(r["ids"].is_array(), "the command's own result: {r}");
    assert!(app.retype_runs.running.is_none());
    let (mut app, _) = outlined();
    app.retype_runs.off_the_ui_thread = false;
    let r = retype_runs::start(&mut app, "type.retypeFonts", json!({"count": 96}), true).unwrap();
    assert_eq!(r["searched"], json!(true), "what retype.find returns: {r}");
    assert!(app.retype_runs.running.is_none());
    // A request that runs at once stops the search before it, which does not apply later.
    let (mut app, _) = outlined();
    let before = undos(&app);
    crate::menus::invoke(&mut app, "type.retype", json!({}));
    assert!(app.retype_runs.running.is_some());
    // A request with bad params leaves it running.
    assert!(retype_runs::start(&mut app, "type.retype", json!({"installed": 1}), false).is_err());
    assert!(app.retype_runs.running.is_some(), "the search goes on");
    retype_runs::start(&mut app, "type.retype", json!({"installed": false}), false).unwrap();
    assert!(app.retype_runs.running.is_none(), "the older search is stopped");
    assert_eq!(undos(&app), before + 1, "the new request retyped once");
    // Bad params fail before anything runs.
    let (mut app, _) = outlined();
    assert!(retype_runs::start(&mut app, "type.retype", json!({"installed": 1}), false).is_err());
    assert!(crate::panels::retype::find_in_background(&mut app, &json!({"more": "yes"})).is_err());
    assert!(crate::panels::retype::find_in_background(&mut app, &json!({"installed": "yes"})).is_err());
    crate::menus::invoke(&mut app, "retype.find", json!({"more": 1}));
    assert!(app.retype_runs.running.is_none() && !app.ui.retype.searched);
    assert!(app.ui.status.contains("`more` must be true or false"), "{}", app.ui.status);
}
