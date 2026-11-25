//! A script's ScriptUI palette in the app: it stays open after the run, and a click on it waits in
//! line and starts its handler's run on the next frame.

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_engine::script::{UiEvent, UiValue};

use crate::dialogs::script_window;
use crate::state::Dialog;
use crate::{VectorcraftApp, menus, script_palettes, script_runs};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    app.session.prefs.show_external_script_warning = false;
    app
}

/// No text typed in a palette's fields.
fn drafts() -> Dialog {
    Dialog::new(script_window::KIND, json!({}))
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

/// One headless frame in which `f` runs. → whether the frame asked for the next one at once.
fn frame(ctx: &egui::Context, f: impl FnMut(&mut egui::Ui)) -> bool {
    frame_delay(ctx, f).is_zero()
}

/// One headless frame in which `f` runs. → how long until the frame asked for the next one;
/// `Duration::MAX` when it asked for none.
fn frame_delay(ctx: &egui::Context, f: impl FnMut(&mut egui::Ui)) -> std::time::Duration {
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0)));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect, ..Default::default() }, f);
    out.textures_delta.clear();
    out.viewport_output.get(&egui::ViewportId::ROOT).map_or(std::time::Duration::MAX, |v| v.repaint_delay)
}

#[test]
fn a_click_on_a_palette_asks_for_the_frame_that_starts_its_run() {
    let mut app = app();
    let source = "#targetengine tools\nvar p = new Window('palette', 'Tools'); var go = p.add('button', undefined, 'Go'); \
                  go.onClick = function () { vc.log('clicked'); }; p.show();";
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": source, "name": "p.js"})).unwrap().unwrap();
    run_to_end(&mut app);
    let id = app.session.script_palettes()[0].id;
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&ctx, |_| {});
    }
    assert!(!frame(&ctx, |_| {}), "an idle frame asks for no other");
    assert!(
        frame(&ctx, |ui| script_palettes::give(&mut app, ui.ctx(), id, &mut drafts(), vec![UiEvent::Click(1)])),
        "the click asks for the next frame"
    );
    assert_eq!(app.session.script_palette_events_waiting(), 1);
    script_runs::poll(&mut app);
    run_to_end(&mut app);
    assert_eq!(app.session.script_report().unwrap()["log"], json!(["clicked"]));
    assert_eq!(app.session.script_palette_events_waiting(), 0);
}

#[test]
fn a_click_waiting_behind_a_run_at_its_dialog_asks_for_no_frames() {
    let mut app = app();
    let source = "#targetengine tools\nvar p = new Window('palette', 'Tools'); var go = p.add('button', undefined, 'Go'); \
                  go.onClick = function () { vc.log('clicked'); }; p.show();";
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": source, "name": "p.js"})).unwrap().unwrap();
    run_to_end(&mut app);
    let id = app.session.script_palettes()[0].id;
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": "vc.log(confirm('Go?'));", "name": "d.js"})).unwrap().unwrap();
    for _ in 0..10_000 {
        if app.session.script_dialog().is_some() {
            break;
        }
        script_runs::poll(&mut app);
    }
    assert!(app.session.script_dialog().is_some(), "the run waits at its confirm");
    let ctx = egui::Context::default();
    frame(&ctx, |ui| script_palettes::give(&mut app, ui.ctx(), id, &mut drafts(), vec![UiEvent::Click(1)]));
    assert_eq!(app.session.script_palette_events_waiting(), 1);
    let mut asks = true;
    for _ in 0..5 {
        app.begin_host_iteration();
        asks = frame(&ctx, |ui| app.tick(ui.ctx()));
        if !asks {
            break;
        }
    }
    assert!(!asks, "the waiting click asks for no frames while the run waits for its answer");
    assert_eq!(app.session.script_palette_events_waiting(), 1, "the click still waits");
}

#[test]
fn a_click_waiting_for_an_open_undo_group_asks_for_no_frames_and_runs_after_it() {
    let mut app = app();
    let source = "#targetengine tools\nvar p = new Window('palette', 'Tools'); var go = p.add('button', undefined, 'Go'); \
                  go.onClick = function () { vc.log('clicked'); }; p.show();";
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": source, "name": "p.js"})).unwrap().unwrap();
    run_to_end(&mut app);
    let id = app.session.script_palettes()[0].id;
    app.run("edit.undoGroup.begin", json!({})).unwrap();
    let ctx = egui::Context::default();
    frame(&ctx, |ui| script_palettes::give(&mut app, ui.ctx(), id, &mut drafts(), vec![UiEvent::Click(1)]));
    assert_eq!(app.session.script_palette_events_waiting(), 1);
    let mut delay = std::time::Duration::ZERO;
    for _ in 0..5 {
        app.begin_host_iteration();
        delay = frame_delay(&ctx, |ui| app.tick(ui.ctx()));
        if delay == std::time::Duration::MAX {
            break;
        }
    }
    assert_eq!(delay, std::time::Duration::MAX, "the waiting click asks for no frames, now or later, while the undo group is open");
    assert_eq!(app.session.script_palette_events_waiting(), 1, "the click still waits");
    app.run("edit.undoGroup.end", json!({})).unwrap();
    app.begin_host_iteration();
    frame(&ctx, |ui| app.tick(ui.ctx()));
    assert_eq!(app.session.script_palette_events_waiting(), 0, "with the group ended the next frame starts the click's run");
    run_to_end(&mut app);
    assert_eq!(app.session.script_report().unwrap()["log"], json!(["clicked"]));
    assert_eq!(app.session.script_palette_events_waiting(), 0);
}

#[test]
fn a_typed_text_stays_in_its_field_when_a_run_adds_a_control_before_it() {
    let mut app = app();
    let source = "#targetengine places\nvar p = new Window('palette'); var g = p.add('group'); g.add('button', undefined, 'First'); \
                  var t = p.add('edittext', undefined, ''); p.show();";
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": source, "name": "p.js"})).unwrap().unwrap();
    run_to_end(&mut app);
    let before = app.session.script_palettes()[0].clone();
    assert_eq!(before.dialog.nodes[3].kind, vectorcraft_engine::script::UiKind::EditText);
    let ctx = egui::Context::default();
    let mut drafts = script_palettes::drafts(&ctx, &before);
    drafts.fields.insert("text3".into(), json!("typed"));
    script_palettes::keep_drafts(&ctx, &before, drafts);
    let added = "#targetengine places\ng.add('button', undefined, 'Added');";
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": added, "name": "a.js"})).unwrap().unwrap();
    run_to_end(&mut app);
    let after = app.session.script_palettes()[0].clone();
    assert_eq!(after.dialog.nodes[4].kind, vectorcraft_engine::script::UiKind::EditText, "the added button comes before the field");
    let drafts = script_palettes::drafts(&ctx, &after);
    assert_eq!(drafts.fields.get("text4"), Some(&json!("typed")), "{:?}", drafts.fields);
    assert!(!drafts.fields.contains_key("text3"));
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
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": source, "name": "p.js"})).unwrap().unwrap();
    run_to_end(&mut app);
    let id = app.session.script_palettes()[0].id;
    let ctx = egui::Context::default();
    let fill: Vec<UiEvent> = (3..3 + MAX_WAITING).map(UiEvent::Click).collect();
    frame(&ctx, |ui| script_palettes::give(&mut app, ui.ctx(), id, &mut drafts(), fill.clone()));
    assert_eq!(app.session.script_palette_events_waiting(), MAX_WAITING, "the line is full");
    let mut typed = drafts();
    let events = vec![UiEvent::Change(1, UiValue::Text("typed".into())), UiEvent::Click(2)];
    frame(&ctx, |ui| script_palettes::give(&mut app, ui.ctx(), id, &mut typed, events.clone()));
    assert_eq!(typed.fields.get("text1"), Some(&json!("typed")), "the refused text goes back in its field: {:?}", typed.fields);
    assert_eq!(app.session.script_palette_events_waiting(), MAX_WAITING, "neither the text nor the click is kept");
}

#[test]
fn a_typed_text_the_script_refuses_keeps_the_palette_click_out_of_line() {
    let mut app = app();
    let source = "#targetengine texts\nvar p = new Window('palette'); var t = p.add('edittext', undefined, 'a'); \
                  var go = p.add('button', undefined, 'Go'); go.onClick = function () { vc.log(t.text); }; p.show();";
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": source, "name": "p.js"})).unwrap().unwrap();
    run_to_end(&mut app);
    let p = app.session.script_palettes()[0].clone();
    let long = "x".repeat(16 * 1024 + 1);
    let mut drafts = Dialog::new(script_window::KIND, json!({}));
    drafts.fields.insert("text1".into(), json!(long));
    let mut events = vec![UiEvent::Change(1, UiValue::Text(long.clone())), UiEvent::Click(2)];
    script_palettes::drafted(&mut app, &p, &mut drafts, &mut events);
    assert!(events.is_empty(), "neither the long text nor the click is given: {events:?}");
    assert_eq!(drafts.fields.get("text1"), Some(&json!(long)), "the refused text stays in its field");
    drafts.fields.insert("text1".into(), json!("fine"));
    let mut events = vec![UiEvent::Click(2)];
    script_palettes::drafted(&mut app, &p, &mut drafts, &mut events);
    assert_eq!(events, vec![UiEvent::Change(1, UiValue::Text("fine".into())), UiEvent::Click(2)]);
    assert!(!drafts.fields.contains_key("text1"), "the text is given once");
}
