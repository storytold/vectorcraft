use std::cell::RefCell;
use std::rc::Rc;

use craft::ui::dialog::UnsavedChangesDialog;
use craft::ui::shell::embed::{self, CloseDecision};
use craft::ui::testing::Fixture;
use serde_json::json;
use vectorcraft_engine::Session;

use crate::{Services, VectorcraftApp};

/// An app with one Letter document open.
fn app_with_a_document() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.session.add_document(vectorcraft_doc::Document::new(612.0, 792.0), None);
    app.sync_views();
    app
}

/// The smoke trial: a debug assert or a panic anywhere in the whole frame (the shell, the dock,
/// the canvas, the dialogs and the messages) shows only when the whole window runs, which no
/// component trial does.
#[test]
fn the_whole_window_shows_for_three_frames() {
    let app = Rc::new(RefCell::new(app_with_a_document()));
    let shown = Rc::clone(&app);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().ui(ui));
    crate::testing::install(fixture.ctx());
    let host = app.borrow().host.clone();
    fixture.room(egui::vec2(1440.0, 900.0)).host(host);
    for _ in 0..3 {
        app.borrow_mut().tick(fixture.ctx());
        fixture.frame(Vec::new());
    }
    let area = app.borrow().canvas.windows.area;
    assert!(area.is_some_and(|r| r.width() > 0.0 && r.height() > 0.0), "the document got no room in the window: {area:?}");
}

/// One frame in which the OS asks the root window to close: the app draws its pass, then the
/// close path of `embed::logic` asks it what to do. Gives the decision the app made, `None` when
/// the close path did not ask.
fn close_frame(app: &mut VectorcraftApp, ctx: &egui::Context) -> Option<CloseDecision> {
    let mut raw = egui::RawInput::default();
    raw.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    let mut decision = None;
    ctx.run_ui(raw, |ui| {
        app.ui(ui);
        embed::logic(ui.ctx(), || {
            let answer = app.close_decision();
            decision = Some(answer);
            answer
        });
    })
    .textures_delta
    .clear();
    decision
}

/// Whether the window's dialog host shows the Unsaved Changes question.
// PLAN: assumed shell-w15's `unsaved::close_all` opens `UnsavedChangesDialog` itself on the window's host (the sheet's Tests row names it).
fn asks_about_unsaved(app: &VectorcraftApp) -> bool {
    app.dialogs().is_some_and(|dialogs| dialogs.with(|host| host.get::<UnsavedChangesDialog>().is_some()))
}

/// The window's close asks about unsaved documents instead of closing the window: with a modified
/// document the window stays while the Unsaved Changes question shows; with nothing to ask it
/// closes.
#[test]
fn the_close_caption_asks_about_unsaved_documents() {
    let ctx = crate::testing::ui_context();
    let mut app = app_with_a_document();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    assert_eq!(close_frame(&mut app, &ctx), Some(CloseDecision::Hold));
    assert!(asks_about_unsaved(&app), "the close asks about the modified document");

    let ctx = crate::testing::ui_context();
    let mut app = app_with_a_document();
    assert_eq!(close_frame(&mut app, &ctx), Some(CloseDecision::Close));
    assert!(!asks_about_unsaved(&app), "nothing unsaved, nothing asked");
}
