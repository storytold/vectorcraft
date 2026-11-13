//! Document tabs dragged along the tab strip move their documents (#5663); dragged off it, they
//! still float them.

use egui::{Event, PointerButton, Pos2};
use serde_json::json;
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, chrome};

const SIZE: egui::Vec2 = egui::vec2(1000.0, 700.0);
const NAMES: [&str; 4] = ["Alpha", "Bravo", "Charlie", "Delta"];

/// Run a tab-strip frame; → the tabs' titles with their rects, and the frame's output.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<Event>) -> (Vec<(String, egui::Rect)>, egui::FullOutput) {
    let mut out = ctx.run_ui(
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, SIZE)), time: Some(time), events, ..Default::default() },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| chrome::doc_tabs(app, ui));
        },
    );
    out.textures_delta.clear();
    fn walk(s: &egui::Shape, v: &mut Vec<(String, egui::Rect)>) {
        match s {
            egui::Shape::Text(t) => v.push((t.galley.text().to_owned(), egui::Rect::from_min_size(t.pos, t.galley.size()))),
            egui::Shape::Vec(list) => list.iter().for_each(|s| walk(s, v)),
            _ => {}
        }
    }
    let mut texts = vec![];
    for c in &out.shapes {
        walk(&c.shape, &mut texts);
    }
    (texts, out)
}

fn button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// An app with documents Alpha, Bravo, Charlie and Delta, Delta current, and a context that has
/// drawn the strip once.
fn app() -> (VectorcraftApp, egui::Context) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    for name in NAMES {
        app.run("file.new", json!({"width": 100, "height": 100, "name": name})).unwrap();
    }
    app.sync_views();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, 0.0, vec![]);
    (app, ctx)
}

fn order(app: &VectorcraftApp) -> Vec<String> {
    app.session.documents().iter().map(|d| d.title()).collect()
}

fn current(app: &VectorcraftApp) -> String {
    app.session.active().map(|d| d.title()).unwrap_or_default()
}

/// The middle of the title of `name`'s tab.
fn tab(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, name: &str) -> Pos2 {
    let (texts, _) = frame(app, ctx, time, vec![]);
    texts.iter().find(|(t, _)| t.starts_with(name)).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("{name}'s tab: {texts:?}"))
}

/// Press at `from`, move to `to` in steps, and release there; → the output of the frame before
/// the release (while the tab is still dragged).
fn drag(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, from: Pos2, to: Pos2) -> egui::FullOutput {
    frame(app, ctx, time, vec![Event::PointerMoved(from)]);
    frame(app, ctx, time + 0.05, vec![button(from, true)]);
    let mut last = None;
    for k in 1..=8 {
        let p = from + (to - from) * (k as f32 / 8.0);
        last = Some(frame(app, ctx, time + 0.05 + 0.02 * f64::from(k), vec![Event::PointerMoved(p)]).1);
    }
    frame(app, ctx, time + 0.3, vec![button(to, false)]);
    frame(app, ctx, time + 0.35, vec![]);
    last.unwrap()
}

#[test]
fn a_tab_dragged_along_the_strip_moves_its_document_and_the_current_one_stays() {
    let (mut app, ctx) = app();
    assert_eq!(current(&app), "Delta");
    let alpha = tab(&mut app, &ctx, 0.1, "Alpha");
    let delta = tab(&mut app, &ctx, 0.15, "Delta");
    let during = drag(&mut app, &ctx, 0.2, alpha, delta + egui::vec2(80.0, 0.0));
    assert_eq!(during.platform_output.cursor_icon, egui::CursorIcon::Grabbing, "the tab is held while it moves");
    assert_eq!(order(&app), ["Bravo", "Charlie", "Delta", "Alpha"]);
    assert_eq!(current(&app), "Delta", "a move is not a switch");
    // The tabs are drawn in the new order.
    let xs: Vec<f32> = NAMES.iter().map(|n| tab(&mut app, &ctx, 1.0, n).x).collect();
    assert!(xs[1] < xs[2] && xs[2] < xs[3] && xs[3] < xs[0], "Bravo, Charlie, Delta, Alpha: {xs:?}");

    // And back to the start, before Bravo.
    let alpha = tab(&mut app, &ctx, 1.1, "Alpha");
    let bravo = tab(&mut app, &ctx, 1.15, "Bravo");
    drag(&mut app, &ctx, 1.2, alpha, bravo - egui::vec2(80.0, 0.0));
    assert_eq!(order(&app), NAMES);
    assert_eq!(current(&app), "Delta");
}

#[test]
fn a_document_keeps_its_own_zoom_after_its_tab_moves() {
    let (mut app, ctx) = app();
    for (k, v) in app.views.iter_mut().enumerate() {
        v.zoom = 1.0 + k as f64;
    }
    let bravo = tab(&mut app, &ctx, 0.1, "Bravo");
    let delta = tab(&mut app, &ctx, 0.15, "Delta");
    drag(&mut app, &ctx, 0.2, bravo, delta + egui::vec2(80.0, 0.0));
    assert_eq!(order(&app), ["Alpha", "Charlie", "Delta", "Bravo"]);
    let zooms: Vec<f64> = app.views.iter().map(|v| v.zoom).collect();
    assert_eq!(zooms, [1.0, 3.0, 4.0, 2.0], "each view went with its document");
    assert_eq!(app.view().map(|v| v.zoom), Some(4.0), "the current document's view is still Delta's");
}

/// A move from outside the strip (MCP, the control channel, a recording) keeps the views with
/// their documents too.
#[test]
fn views_follow_a_move_made_by_the_engine_command() {
    let (mut app, _ctx) = app();
    for (k, v) in app.views.iter_mut().enumerate() {
        v.zoom = 1.0 + k as f64;
    }
    app.session.execute("document.move", &json!({"from": 3, "to": 0})).unwrap();
    app.sync_views();
    assert_eq!(order(&app), ["Delta", "Alpha", "Bravo", "Charlie"]);
    assert_eq!(app.views.iter().map(|v| v.zoom).collect::<Vec<_>>(), [4.0, 1.0, 2.0, 3.0]);
    // Closing a document still leaves the others' views with them.
    app.run("file.close", json!({"index": 1})).unwrap();
    app.sync_views();
    assert_eq!(order(&app), ["Delta", "Bravo", "Charlie"]);
    assert_eq!(app.views.iter().map(|v| v.zoom).collect::<Vec<_>>(), [4.0, 2.0, 3.0]);
}

/// One batch that closes documents and moves others (a script, a recording) leaves every view with
/// its own document; the views are put right once, after the whole batch.
#[test]
fn views_follow_closes_and_moves_made_in_one_batch() {
    let (mut app, _ctx) = app();
    let zooms = |app: &VectorcraftApp| app.views.iter().map(|v| v.zoom).collect::<Vec<_>>();
    for (k, v) in app.views.iter_mut().enumerate() {
        v.zoom = 1.0 + k as f64;
    }
    // A close alone, in the middle: the documents after it keep their own views.
    app.run("command.batch", json!({"commands": [{"command": "file.close", "params": {"index": 1}}]})).unwrap();
    assert_eq!(order(&app), ["Alpha", "Charlie", "Delta"]);
    assert_eq!(zooms(&app), [1.0, 3.0, 4.0], "Charlie and Delta keep theirs, not Bravo's and Charlie's");
    // A close and a move together.
    app.run(
        "command.batch",
        json!({"commands": [
            {"command": "file.close", "params": {"index": 0}},
            {"command": "document.move", "params": {"from": 1, "to": 0}},
        ]}),
    )
    .unwrap();
    assert_eq!(order(&app), ["Delta", "Charlie"]);
    assert_eq!(zooms(&app), [4.0, 3.0], "Delta does not take the closed Alpha's view");
    // Closing other documents by index leaves the current one (Delta) current; it shows its own view.
    assert_eq!((current(&app), app.view().map(|v| v.zoom)), ("Delta".into(), Some(4.0)));
    // A document opened afterwards starts with a view of its own.
    app.run("file.new", json!({"width": 100, "height": 100, "name": "Echo"})).unwrap();
    app.sync_views();
    assert_eq!(order(&app), ["Delta", "Charlie", "Echo"]);
    assert_eq!(app.views.len(), 3);
    assert_eq!(&zooms(&app)[..2], [4.0, 3.0]);
}

#[test]
fn a_tab_dragged_off_the_strip_still_floats_and_moves_nothing() {
    let (mut app, ctx) = app();
    let alpha = tab(&mut app, &ctx, 0.1, "Alpha");
    drag(&mut app, &ctx, 0.2, alpha, alpha + egui::vec2(150.0, 300.0));
    assert_eq!(order(&app), NAMES, "the documents keep their order");
    let uid = app.session.documents()[0].uid;
    assert!(app.ui.floating.iter().any(|w| w.uid == uid), "Alpha floats in a window");
}

#[test]
fn next_document_steps_through_the_tabs_in_their_new_order() {
    let (mut app, ctx) = app();
    let delta = tab(&mut app, &ctx, 0.1, "Delta");
    let alpha = tab(&mut app, &ctx, 0.15, "Alpha");
    drag(&mut app, &ctx, 0.2, delta, alpha - egui::vec2(80.0, 0.0));
    assert_eq!(order(&app), ["Delta", "Alpha", "Bravo", "Charlie"]);
    assert_eq!(current(&app), "Delta");
    app.run("window.nextDocument", json!({})).unwrap();
    assert_eq!(current(&app), "Alpha", "after Delta, now first, comes Alpha");
    app.run("window.previousDocument", json!({})).unwrap();
    app.run("window.previousDocument", json!({})).unwrap();
    assert_eq!(current(&app), "Charlie", "before the first tab comes the last");
}

#[test]
fn a_click_on_a_tab_still_switches_to_it() {
    let (mut app, ctx) = app();
    let bravo = tab(&mut app, &ctx, 0.1, "Bravo");
    frame(&mut app, &ctx, 0.2, vec![Event::PointerMoved(bravo)]);
    frame(&mut app, &ctx, 0.25, vec![button(bravo, true)]);
    frame(&mut app, &ctx, 0.3, vec![button(bravo, false)]);
    assert_eq!(current(&app), "Bravo");
    assert_eq!(order(&app), NAMES);
}
