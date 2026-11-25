//! A script's ScriptUI palette in the app: it stays open on the dialog host after the run, follows
//! the session's palette, and a click on it waits in line and starts its handler's run on the next
//! frame.

use serde_json::json;
use vectorcraft_engine::script::{UiEvent, UiKind, UiValue};

use super::script_dialogs::{app, is_open};
use crate::dialogs::script_run::ScriptNotice;
use crate::script_palettes::{self, PaletteWindow, with_palette};
use crate::{VectorcraftApp, menus, script_runs};

const TOOLS: &str = "#targetengine tools\nvar p = new Window('palette', 'Tools'); var go = p.add('button', undefined, 'Go'); \
                     go.onClick = function () { vc.log('clicked'); }; p.show();";

fn start(app: &mut VectorcraftApp, source: &str) {
    menus::run_ui_command(app, "ui.runScript", &json!({"source": source, "name": "p.js"})).unwrap().unwrap();
}

fn run_to_end(app: &mut VectorcraftApp) {
    for _ in 0..10_000 {
        if !app.session.script_running() {
            return;
        }
        script_runs::poll(app);
    }
    panic!("the script did not end");
}

/// Runs `source`, then follows its palettes into the dialog host. → the first palette's id.
fn shown(app: &mut VectorcraftApp, source: &str) -> u64 {
    start(app, source);
    run_to_end(app);
    follow(app);
    app.session.script_palettes()[0].id
}

/// The frame's follow of the session's palettes into the dialog host.
fn follow(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().unwrap();
    script_palettes::show(app, &ctx);
}

/// One headless pass in which `f` runs. → how long until the pass asked for the next one;
/// `Duration::MAX` when it asked for none.
fn frame_delay(app: &mut VectorcraftApp, mut f: impl FnMut(&mut VectorcraftApp, &egui::Context)) -> std::time::Duration {
    let ctx = app.ui_ctx.clone().unwrap();
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0)));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect, ..Default::default() }, |ui| f(app, ui.ctx()));
    out.textures_delta.clear();
    out.viewport_output.get(&egui::ViewportId::ROOT).map_or(std::time::Duration::MAX, |v| v.repaint_delay)
}

/// Runs passes of `f` until one asks for no frame at once. → whether one did.
fn settle(app: &mut VectorcraftApp, mut f: impl FnMut(&mut VectorcraftApp, &egui::Context)) -> bool {
    (0..10).any(|_| !frame_delay(app, &mut f).is_zero())
}

fn give(app: &VectorcraftApp, id: u64, event: UiEvent) {
    with_palette(app, id, |w| w.give(event)).unwrap();
}

#[test]
fn a_shown_palette_opens_on_the_dialog_host_and_closes_when_hidden() {
    let mut app = app();
    let id = shown(&mut app, TOOLS);
    assert!(is_open::<PaletteWindow>(&app), "the palette stays open after the run");
    start(&mut app, "#targetengine tools\np.hide();");
    run_to_end(&mut app);
    follow(&mut app);
    assert!(with_palette(&app, id, |_| ()).is_none(), "a hidden palette's dialog closes");
    start(&mut app, "#targetengine tools\np.show();");
    run_to_end(&mut app);
    follow(&mut app);
    assert!(with_palette(&app, id, |_| ()).is_some(), "showing it again opens it");
}

#[test]
fn a_script_dialog_opens_while_a_palette_is_open() {
    let mut app = app();
    shown(&mut app, TOOLS);
    start(&mut app, "vc.log(confirm('Go?'));");
    for _ in 0..10_000 {
        if is_open::<ScriptNotice>(&app) {
            break;
        }
        assert!(app.session.script_running(), "the run waits at its confirm");
        script_runs::poll(&mut app);
    }
    assert!(is_open::<ScriptNotice>(&app), "the palette beside the documents does not keep the confirm from opening");
    assert!(is_open::<PaletteWindow>(&app));
}

#[test]
fn a_click_on_a_palette_asks_for_the_frame_that_starts_its_run() {
    let mut app = app();
    let id = shown(&mut app, TOOLS);
    assert!(settle(&mut app, |_, _| {}), "an idle pass asks for no other");
    give(&app, id, UiEvent::Click(1));
    assert!(frame_delay(&mut app, script_palettes::show).is_zero(), "the click asks for the next frame");
    assert_eq!(app.session.script_palette_events_waiting(), 1);
    script_runs::poll(&mut app);
    run_to_end(&mut app);
    assert_eq!(app.session.script_report().unwrap()["log"], json!(["clicked"]));
    assert_eq!(app.session.script_palette_events_waiting(), 0);
}

#[test]
fn a_click_waiting_behind_a_run_at_its_dialog_asks_for_no_frames() {
    let mut app = app();
    let id = shown(&mut app, TOOLS);
    start(&mut app, "vc.log(confirm('Go?'));");
    for _ in 0..10_000 {
        if app.session.script_dialog().is_some() {
            break;
        }
        script_runs::poll(&mut app);
    }
    assert!(app.session.script_dialog().is_some(), "the run waits at its confirm");
    give(&app, id, UiEvent::Click(1));
    follow(&mut app);
    assert_eq!(app.session.script_palette_events_waiting(), 1);
    assert!(settle(&mut app, |app, ctx| app.tick(ctx)), "the waiting click asks for no frames while the run waits for its answer");
    assert_eq!(app.session.script_palette_events_waiting(), 1, "the click still waits");
}

#[test]
fn a_click_waiting_for_an_open_undo_group_asks_for_no_frames_and_runs_after_it() {
    let mut app = app();
    let id = shown(&mut app, TOOLS);
    app.run("edit.undoGroup.begin", json!({})).unwrap();
    give(&app, id, UiEvent::Click(1));
    follow(&mut app);
    assert_eq!(app.session.script_palette_events_waiting(), 1);
    let mut delay = std::time::Duration::ZERO;
    for _ in 0..5 {
        delay = frame_delay(&mut app, |app, ctx| app.tick(ctx));
        if delay == std::time::Duration::MAX {
            break;
        }
    }
    assert_eq!(delay, std::time::Duration::MAX, "the waiting click asks for no frames, now or later, while the undo group is open");
    assert_eq!(app.session.script_palette_events_waiting(), 1, "the click still waits");
    app.run("edit.undoGroup.end", json!({})).unwrap();
    frame_delay(&mut app, |app, ctx| app.tick(ctx));
    assert_eq!(app.session.script_palette_events_waiting(), 0, "with the group ended the next frame starts the click's run");
    run_to_end(&mut app);
    assert_eq!(app.session.script_report().unwrap()["log"], json!(["clicked"]));
}

#[test]
fn a_typed_text_stays_in_its_field_when_a_run_adds_a_control_before_it() {
    let mut app = app();
    let source = "#targetengine places\nvar p = new Window('palette'); var g = p.add('group'); g.add('button', undefined, 'First'); \
                  var t = p.add('edittext', undefined, ''); p.show();";
    let id = shown(&mut app, source);
    assert_eq!(app.session.script_palettes()[0].dialog.nodes[3].kind, UiKind::EditText);
    with_palette(&app, id, |w| w.type_text(3, "typed")).unwrap();
    start(&mut app, "#targetengine places\ng.add('button', undefined, 'Added');");
    run_to_end(&mut app);
    follow(&mut app);
    assert_eq!(app.session.script_palettes()[0].dialog.nodes[4].kind, UiKind::EditText, "the added button comes before the field");
    assert_eq!(with_palette(&app, id, |w| w.typed(4).map(str::to_string)).unwrap().as_deref(), Some("typed"));
    assert_eq!(with_palette(&app, id, |w| w.typed(3).map(str::to_string)).unwrap(), None);
}

#[test]
fn a_click_given_before_a_run_removes_a_control_before_it_reaches_its_button() {
    let mut app = app();
    let source = "#targetengine moves\nvar p = new Window('palette'); var a = p.add('button', undefined, 'A'); \
                  var b = p.add('button', undefined, 'B'); a.onClick = function () { vc.log('a'); }; \
                  b.onClick = function () { vc.log('b'); }; p.show();";
    let id = shown(&mut app, source);
    give(&app, id, UiEvent::Click(2));
    // A run removes A before the app takes the click, so B is now the palette's first control.
    start(&mut app, "#targetengine moves\np.remove(a);");
    run_to_end(&mut app);
    follow(&mut app);
    assert_eq!(app.session.script_palette_events_waiting(), 1, "the click waits for its run");
    script_runs::poll(&mut app);
    run_to_end(&mut app);
    assert_eq!(app.session.script_report().unwrap()["log"], json!(["b"]), "the click reaches the button it was given to");
}

#[test]
fn a_typed_text_the_full_line_refuses_stays_in_its_field() {
    use vectorcraft_engine::script::palettes::MAX_WAITING;
    let mut app = app();
    let source = format!(
        "#targetengine full\nvar p = new Window('palette'); var t = p.add('edittext', undefined, 'a'); \
         var go = p.add('button', undefined, 'Go'); go.onClick = function () {{ vc.log(t.text); }}; \
         for (var k = 0; k < {MAX_WAITING}; k++) {{ p.add('button', undefined, 'B' + k); }} p.show();"
    );
    let id = shown(&mut app, &source);
    for c in 3..3 + MAX_WAITING {
        app.session.script_palette_event(id, UiEvent::Click(c)).unwrap();
    }
    assert_eq!(app.session.script_palette_events_waiting(), MAX_WAITING, "the line is full");
    give(&app, id, UiEvent::Change(1, UiValue::Text("typed".into())));
    give(&app, id, UiEvent::Click(2));
    follow(&mut app);
    assert_eq!(
        with_palette(&app, id, |w| w.typed(1).map(str::to_string)).unwrap().as_deref(),
        Some("typed"),
        "the refused text goes back in its field"
    );
    assert_eq!(app.session.script_palette_events_waiting(), MAX_WAITING, "neither the text nor the click is kept");
}

#[test]
fn a_typed_text_the_script_refuses_keeps_the_palette_click_out_of_line() {
    let mut app = app();
    let source = "#targetengine texts\nvar p = new Window('palette'); var t = p.add('edittext', undefined, 'a'); \
                  var go = p.add('button', undefined, 'Go'); go.onClick = function () { vc.log(t.text); }; p.show();";
    let id = shown(&mut app, source);
    let long = "x".repeat(16 * 1024 + 1);
    with_palette(&app, id, |w| w.type_text(1, &long)).unwrap();
    give(&app, id, UiEvent::Change(1, UiValue::Text(long.clone())));
    give(&app, id, UiEvent::Click(2));
    follow(&mut app);
    assert_eq!(app.session.script_palette_events_waiting(), 0, "neither the long text nor the click is given");
    assert_eq!(with_palette(&app, id, |w| w.typed(1).map(str::to_string)).unwrap(), Some(long), "the refused text stays in its field");
    with_palette(&app, id, |w| w.type_text(1, "fine")).unwrap();
    give(&app, id, UiEvent::Click(2));
    follow(&mut app);
    assert_eq!(app.session.script_palette_events_waiting(), 2, "the typed text, then the click");
    assert_eq!(with_palette(&app, id, |w| w.typed(1).map(str::to_string)).unwrap(), None, "the text is given once");
    // The change's run, then the click's.
    for _ in 0..2 {
        script_runs::poll(&mut app);
        run_to_end(&mut app);
    }
    assert_eq!(app.session.script_palette_events_waiting(), 0);
    assert_eq!(app.session.script_report().unwrap()["log"], json!(["fine"]), "the click's handler reads the given text");
}
