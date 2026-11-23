//! ScriptUI palettes in a script: a named engine to show them in, the events that wait for a run
//! and run one at a time after it, the events that take a waiting one's place, `close()` and the
//! close box, the engine's end, and the limits.

use std::time::Duration;

use serde_json::{Value, json};

use crate::script::palettes::{MAX_PALETTES, MAX_WAITING};
use crate::script::scriptui::PALETTE_NEEDS_ENGINE;
use crate::script::{DialogAnswer, TEST_ELAPSED, UiEvent, Waiting};
use crate::*;

/// One open document with one rectangle.
fn one_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s
}

fn run(s: &mut Session, source: &str) -> Result<Value> {
    s.execute("script.run", &json!({"source": source, "name": "p.js"}))
}

/// Pumps the run in the app until it ends, or until it waits at a dialog (`None`).
fn pump(s: &mut Session) -> Option<Result<Value>> {
    for _ in 0..10_000 {
        if s.script_dialog().is_some() || s.script_window().is_some() {
            return None;
        }
        if let Some(r) = s.script_pump() {
            return Some(r);
        }
    }
    panic!("the run neither ended nor waited");
}

/// Runs `source` in the app to its end.
fn run_in_app(s: &mut Session, source: &str) -> Value {
    s.script_frames = true;
    assert_eq!(run(s, source).unwrap()["running"], true);
    pump(s).expect("the run ended").unwrap()
}

/// A palette in the engine `tools` with a button (control 1) whose handler counts its clicks and
/// adds a rectangle, and a field (control 2).
const TOOLS: &str = "#targetengine tools\n\
    var clicks = 0; var p = new Window('palette', 'Tools'); var go = p.add('button', undefined, 'Go'); \
    var field = p.add('edittext', undefined, ''); \
    go.onClick = function () { clicks++; vc.run('shape.rectangle', {x: 20, y: 0, width: 5, height: 5}); vc.log('click ' + clicks); }; \
    field.onChange = function () { vc.log('change ' + field.text); }; p.show(); vc.log('shown');";

fn only_palette(s: &Session) -> u64 {
    let list = s.script_palettes();
    assert_eq!(list.len(), 1, "{list:?}");
    list[0].id
}

#[test]
fn show_returns_at_once_and_the_palette_stays_after_the_run() {
    let mut s = one_doc();
    let r = run(&mut s, TOOLS).unwrap();
    assert_eq!(r["log"], json!(["shown"]), "show() did not wait: {r}");
    let id = only_palette(&s);
    let listed = s.execute("script.palettes", &json!({})).unwrap();
    assert_eq!(listed["palettes"][0]["engine"], "tools");
    assert_eq!(listed["palettes"][0]["title"], "Tools");
    assert_eq!(listed["palettes"][0]["controls"][0]["text"], "Go");
    let clicked = s.execute("script.palette.event", &json!({"palette": id, "event": "click", "control": "Go"})).unwrap();
    assert_eq!(clicked["log"], json!(["click 1"]));
    assert_eq!(s.documents()[0].history.undo.last().unwrap().label, "Run Script", "a handler's run is one undo step");
    let changed = s.execute("script.palette.event", &json!({"palette": id, "event": "change", "control": 2, "value": "abc"})).unwrap();
    assert_eq!(changed["log"], json!(["change abc"]));
}

#[test]
fn update_in_a_palette_s_handler_runs_and_the_palette_shows_the_last_value() {
    let mut s = one_doc();
    run(
        &mut s,
        "#targetengine progress\nvar p = new Window('palette'); var bar = p.add('progressbar', undefined, 0, 10); \
         var go = p.add('button', undefined, 'Go'); \
         go.onClick = function () { for (var k = 1; k <= 10; k++) { bar.value = k; p.update(); } vc.log('done'); }; p.show();",
    )
    .unwrap();
    let id = only_palette(&s);
    let clicked = s.execute("script.palette.event", &json!({"palette": id, "event": "click", "control": "Go"})).unwrap();
    assert_eq!(clicked["log"], json!(["done"]), "update() in the handler does not throw");
    let listed = s.execute("script.palettes", &json!({})).unwrap();
    assert_eq!(listed["palettes"][0]["controls"][0]["value"].as_f64(), Some(10.0), "{listed}");
}

#[test]
fn update_outside_a_handler_pauses_the_run_so_the_app_draws_the_palette_at_most_once_per_interval() {
    TEST_ELAPSED.set(Some(Duration::ZERO));
    let mut s = one_doc();
    s.script_frames = true;
    let source = "#targetengine progress\nvar p = new Window('palette'); var bar = p.add('progressbar', undefined, 0, 3); p.show(); \
                  for (var k = 1; k <= 3; k++) { bar.value = k; p.update(); }";
    assert_eq!(run(&mut s, source).unwrap()["running"], true);
    let value = |s: &Session| s.script_palettes().first().map_or(0.0, |p| p.dialog.nodes[1].value);
    for _ in 0..100 {
        assert!(s.script_pump().is_none(), "the run pauses at the first update()");
        if value(&s) > 0.0 {
            break;
        }
    }
    let first = value(&s);
    assert!(first > 0.0);
    TEST_ELAPSED.set(Some(Duration::from_millis(16)));
    assert!(s.script_pump().is_none(), "an update() a frame later pauses the run too");
    let second = value(&s);
    assert!(second > first, "the palette shows the second update()");
    assert!(s.script_pump().expect("an update() within the same frame does not pause the run").is_ok());
    assert!(value(&s) > second);
    TEST_ELAPSED.set(None);
}

#[test]
fn update_hands_a_click_given_during_the_run_to_its_handler_so_cancel_stops_the_loop() {
    TEST_ELAPSED.set(Some(Duration::ZERO));
    let mut s = one_doc();
    s.script_frames = true;
    let source = "#targetengine progress\nvar p = new Window('palette'); var bar = p.add('progressbar', undefined, 0, 100000); \
                  var cancel = p.add('button', undefined, 'Cancel'); var stopped = false; cancel.onClick = function () { stopped = true; }; \
                  p.show(); var k = 0; while (!stopped && k < 100000) { k++; bar.value = k; p.update(); } vc.log(stopped ? 'canceled' : 'ran out');";
    assert_eq!(run(&mut s, source).unwrap()["running"], true);
    for _ in 0..100 {
        if !s.script_palettes().is_empty() {
            break;
        }
        assert!(s.script_pump().is_none(), "the run pauses at the first update()");
    }
    let id = only_palette(&s);
    assert_eq!(s.script_palette_event(id, UiEvent::Click(2)), Ok(Waiting::Added));
    let report = pump(&mut s).expect("the run ended").unwrap();
    assert_eq!(report["log"], json!(["canceled"]), "{report}");
    assert_eq!(s.script_palette_events_waiting(), 0, "the click was handled in the run, not left for a run of its own");
    TEST_ELAPSED.set(None);
}

#[test]
fn update_in_a_handler_does_not_pause_the_run() {
    TEST_ELAPSED.set(Some(Duration::ZERO));
    let mut s = one_doc();
    run_in_app(
        &mut s,
        "#targetengine progress\nvar p = new Window('palette'); var bar = p.add('progressbar', undefined, 0, 3); \
         var go = p.add('button', undefined, 'Go'); \
         go.onClick = function () { for (var k = 1; k <= 3; k++) { bar.value = k; p.update(); } vc.log('done'); }; p.show();",
    );
    let id = only_palette(&s);
    assert_eq!(s.script_palette_event(id, UiEvent::Click(2)), Ok(Waiting::Added));
    assert!(s.script_palette_start().expect("the click starts its run").is_ok());
    let report = s.script_pump().expect("the handler's run ends without a pause after the handler").unwrap();
    assert_eq!(report["log"], json!(["done"]));
    TEST_ELAPSED.set(None);
}

#[test]
fn many_update_calls_within_one_interval_give_the_controls_once() {
    TEST_ELAPSED.set(Some(Duration::ZERO));
    let mut s = one_doc();
    let report = run_in_app(
        &mut s,
        "#targetengine progress\nvar names = []; for (var n = 0; n < 500; n++) { names.push('a long list item name ' + n); } \
         var p = new Window('palette'); var list = p.add('listbox', undefined, names); var bar = p.add('progressbar', undefined, 0, 1000); \
         p.show(); for (var k = 1; k <= 1000; k++) { bar.value = k; p.update(); }",
    );
    let calls = report["hostCalls"].as_u64().unwrap();
    assert!(calls < 1000, "an update() the app does not draw costs no host call: {calls}");
    let bar = s.script_palettes()[0].dialog.nodes[2].value;
    assert_eq!(bar, 1000.0, "the palette shows the last value once the run ends");
    TEST_ELAPSED.set(None);
}

#[test]
fn a_later_run_in_the_engine_changes_what_the_palette_shows() {
    let mut s = one_doc();
    run(&mut s, TOOLS).unwrap();
    let id = only_palette(&s);
    run(&mut s, "#targetengine tools\ngo.enabled = false; go.text = 'Wait';").unwrap();
    let listed = s.execute("script.palettes", &json!({})).unwrap();
    assert_eq!(listed["palettes"][0]["controls"][0]["enabled"], false, "{listed}");
    assert_eq!(listed["palettes"][0]["controls"][0]["text"], "Wait");
    assert!(s.execute("script.palette.event", &json!({"palette": id, "event": "click", "control": 1})).is_err(), "a disabled button takes no click");
}

#[test]
fn a_handler_keeps_the_file_grants_of_the_script_that_showed_the_palette() {
    let base = std::env::temp_dir().join(format!("vc-script-palette-grants-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("main")).unwrap();
    std::fs::create_dir_all(base.join("picked")).unwrap();
    let base = std::fs::canonicalize(&base).unwrap();
    let data = base.join("picked/data.txt");
    std::fs::write(&data, "hello").unwrap();
    let path = serde_json::to_string(&data.to_string_lossy()).unwrap();
    let main = base.join("main/main.jsx");
    let source = format!(
        "#targetengine grants\nif (typeof picked == 'undefined') {{ var picked = File.openDialog('p'); }} \
         var p = new Window('palette'); var b = p.add('button', undefined, 'Read'); \
         b.onClick = function () {{ var f = new File({path}); vc.log(f.open('r') ? f.read() : f.error); }}; p.show();"
    );
    std::fs::write(&main, source).unwrap();
    let mut s = Session::new();
    s.attach_script_data(base.join("settings/Script Data"));
    s.script_frames = true;
    assert_eq!(s.execute("script.run", &json!({"path": main.to_string_lossy()})).unwrap()["running"], true);
    for _ in 0..10_000 {
        if s.script_picker().is_some() {
            break;
        }
        assert!(s.script_pump().is_none(), "the run waits at its file dialog");
    }
    let picked = data.to_string_lossy().into_owned();
    assert!(s.script_pick(crate::script::files::PickAnswer::Allow { paths: vec![picked], always: true }));
    pump(&mut s).expect("the run ended").unwrap();
    let id = only_palette(&s);
    assert_eq!(s.script_palette_event(id, UiEvent::Click(1)), Ok(Waiting::Added));
    assert!(s.script_palette_start().expect("the click starts its run").is_ok());
    assert_eq!(pump(&mut s).expect("the handler ended").unwrap()["log"], json!(["hello"]));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_failed_run_in_the_engine_still_changes_what_the_palette_shows() {
    let mut s = one_doc();
    run(&mut s, TOOLS).unwrap();
    assert!(run(&mut s, "#targetengine tools\ngo.enabled = false; throw new Error('after');").is_err());
    let listed = s.execute("script.palettes", &json!({})).unwrap();
    assert_eq!(listed["palettes"][0]["controls"][0]["enabled"], false, "{listed}");
}

#[test]
fn a_list_change_that_waited_while_the_list_changed_ends_its_run() {
    let mut s = one_doc();
    run_in_app(
        &mut s,
        "#targetengine lists\nvar p = new Window('palette'); var l = p.add('listbox', undefined, ['a', 'b']); \
         l.onChange = function () { vc.log(String(l.selection)); }; p.show();",
    );
    let id = only_palette(&s);
    assert_eq!(run(&mut s, "#targetengine lists\nconfirm('Go?'); l.removeAll(); l.add('item', 'a'); l.add('item', 'b');").unwrap()["running"], true);
    assert!(pump(&mut s).is_none());
    let pick = UiEvent::Change(1, crate::script::UiValue::Index(Some(1)));
    assert_eq!(s.script_palette_event(id, pick.clone()), Ok(Waiting::Added));
    assert!(s.script_answer(DialogAnswer::Confirm(true)));
    pump(&mut s).unwrap().unwrap();
    assert!(s.script_palette_start().expect("the change starts its run").is_ok());
    let e = pump(&mut s).unwrap().unwrap_err().to_string();
    assert!(e.contains("a palette's list changed before the person's choice was taken"), "{e}");
    assert_eq!(s.script_palette_event(id, pick), Ok(Waiting::Added), "a change given after it is for the new items");
    assert!(s.script_palette_start().unwrap().is_ok());
    assert_eq!(pump(&mut s).unwrap().unwrap()["log"], json!(["b"]));
}

#[test]
fn a_list_change_for_an_item_a_run_removed_since_ends_its_run() {
    let mut s = one_doc();
    run_in_app(
        &mut s,
        "#targetengine shrink\nvar p = new Window('palette'); var l = p.add('listbox', undefined, ['a', 'b']); \
         l.onChange = function () { vc.log(String(l.selection)); }; p.show();",
    );
    let id = only_palette(&s);
    assert_eq!(run(&mut s, "#targetengine shrink\nconfirm('Go?'); l.removeAll(); l.add('item', 'a');").unwrap()["running"], true);
    assert!(pump(&mut s).is_none());
    assert_eq!(s.script_palette_event(id, UiEvent::Change(1, crate::script::UiValue::Index(Some(1)))), Ok(Waiting::Added));
    assert!(s.script_answer(DialogAnswer::Confirm(true)));
    pump(&mut s).unwrap().unwrap();
    assert!(s.script_palette_start().expect("the change starts its run").is_ok());
    let e = pump(&mut s).unwrap().unwrap_err().to_string();
    assert!(e.contains("a palette's list changed before the person's choice was taken"), "{e}");
    assert_eq!(run(&mut s, "#targetengine shrink\nconfirm('Go?'); l.add('item', 'c'); l.enabled = false;").unwrap()["running"], true);
    assert!(pump(&mut s).is_none());
    assert_eq!(s.script_palette_event(id, UiEvent::Change(1, crate::script::UiValue::Index(Some(0)))), Ok(Waiting::Added));
    assert!(s.script_answer(DialogAnswer::Confirm(true)));
    pump(&mut s).unwrap().unwrap();
    assert!(s.script_palette_start().is_none(), "a change of a list a run changed and disabled since is dropped");
    assert_eq!(s.script_palette_events_waiting(), 0);
}

#[test]
fn a_click_that_waited_while_a_run_added_a_control_before_it_reaches_its_button() {
    let mut s = one_doc();
    run_in_app(
        &mut s,
        "#targetengine places\nvar p = new Window('palette'); var g = p.add('group'); g.add('button', undefined, 'First'); \
         var last = p.add('button', undefined, 'Last'); last.onClick = function () { vc.log('last'); }; p.show();",
    );
    let id = only_palette(&s);
    assert_eq!(s.script_palettes()[0].dialog.find("Last"), Some(3));
    let added = "#targetengine places\nconfirm('Go?'); g.add('button', undefined, 'Added').onClick = function () { vc.log('added'); };";
    assert_eq!(run(&mut s, added).unwrap()["running"], true);
    assert!(pump(&mut s).is_none());
    assert_eq!(s.script_palette_event(id, UiEvent::Click(3)), Ok(Waiting::Added));
    assert!(s.script_answer(DialogAnswer::Confirm(true)));
    pump(&mut s).unwrap().unwrap();
    assert_eq!(s.script_palettes()[0].dialog.find("Last"), Some(4), "the added button comes before Last in the list of controls");
    assert!(s.script_palette_start().expect("the click starts its run").is_ok());
    assert_eq!(pump(&mut s).unwrap().unwrap()["log"], json!(["last"]));
}

#[test]
fn a_waiting_click_on_a_button_a_handler_disabled_is_dropped() {
    let mut s = one_doc();
    run_in_app(
        &mut s,
        "#targetengine switches\nvar p = new Window('palette'); var off = p.add('button', undefined, 'Off'); \
         var go = p.add('button', undefined, 'Go'); off.onClick = function () { go.enabled = false; vc.log('off'); }; \
         go.onClick = function () { vc.log('go'); }; p.show();",
    );
    let id = only_palette(&s);
    assert_eq!(run(&mut s, "confirm('Go?');").unwrap()["running"], true);
    assert!(pump(&mut s).is_none());
    assert_eq!(s.script_palette_event(id, UiEvent::Click(1)), Ok(Waiting::Added));
    assert_eq!(s.script_palette_event(id, UiEvent::Click(2)), Ok(Waiting::Added));
    assert!(s.script_answer(DialogAnswer::Confirm(true)));
    pump(&mut s).unwrap().unwrap();
    assert!(s.script_palette_start().expect("the first click starts its run").is_ok());
    assert_eq!(pump(&mut s).unwrap().unwrap()["log"], json!(["off"]));
    assert!(s.script_palette_start().is_none(), "the click on the button the handler disabled does not run");
    assert_eq!(s.script_palette_events_waiting(), 0);
}

#[test]
fn a_palette_click_during_another_run_runs_after_it() {
    let mut s = one_doc();
    run_in_app(&mut s, TOOLS);
    let id = only_palette(&s);
    assert_eq!(run(&mut s, "var a = confirm('Go?'); vc.log('busy done');").unwrap()["running"], true);
    assert!(pump(&mut s).is_none(), "the run waits at its confirm");
    assert_eq!(s.script_palette_event(id, UiEvent::Click(1)), Ok(Waiting::Added));
    assert!(s.script_palette_start().is_none(), "the click waits while another run holds the lease");
    assert_eq!(s.script_palette_events_waiting(), 1);
    assert!(s.script_answer(DialogAnswer::Confirm(true)));
    assert_eq!(pump(&mut s).unwrap().unwrap()["log"], json!(["busy done"]));
    assert!(s.script_palette_start().expect("the click starts its run").is_ok());
    assert_eq!(pump(&mut s).unwrap().unwrap()["log"], json!(["click 1"]));
    assert_eq!(s.script_palette_events_waiting(), 0);
}

#[test]
fn double_clicks_while_queued_run_once() {
    let mut s = one_doc();
    run_in_app(&mut s, TOOLS);
    let id = only_palette(&s);
    assert_eq!(run(&mut s, "confirm('Go?');").unwrap()["running"], true);
    assert!(pump(&mut s).is_none());
    assert_eq!(s.script_palette_event(id, UiEvent::Click(1)), Ok(Waiting::Added));
    assert_eq!(s.script_palette_event(id, UiEvent::Click(1)), Ok(Waiting::Merged));
    let text = |t: &str| UiEvent::Change(2, crate::script::UiValue::Text(t.into()));
    assert_eq!(s.script_palette_event(id, text("a")), Ok(Waiting::Added));
    assert_eq!(s.script_palette_event(id, text("ab")), Ok(Waiting::Merged), "a later change takes the waiting one's place");
    assert_eq!(s.script_palette_events_waiting(), 2);
    assert!(s.script_answer(DialogAnswer::Confirm(true)));
    pump(&mut s).unwrap().unwrap();
    let mut logs = vec![];
    while let Some(started) = s.script_palette_start() {
        started.unwrap();
        logs.push(pump(&mut s).unwrap().unwrap()["log"].clone());
    }
    assert_eq!(logs, [json!(["click 1"]), json!(["change ab"])]);
}

#[test]
fn a_list_change_takes_a_whole_float_as_its_item_index() {
    let mut s = one_doc();
    run(
        &mut s,
        "#targetengine floats\nvar p = new Window('palette'); var l = p.add('listbox', undefined, ['a', 'b']); \
         l.onChange = function () { vc.log(String(l.selection)); }; p.show();",
    )
    .unwrap();
    let id = only_palette(&s);
    let picked = s.execute("script.palette.event", &json!({"palette": id, "event": "change", "control": 1, "value": 1.0})).unwrap();
    assert_eq!(picked["log"], json!(["b"]));
    for value in [json!(0.5), json!(-1)] {
        let given = s.execute("script.palette.event", &json!({"palette": id, "event": "change", "control": 1, "value": value}));
        assert!(given.is_err(), "{value}");
    }
}

#[test]
fn with_nobody_to_ask_an_event_while_an_undo_group_is_open_is_refused_and_not_kept() {
    let mut s = one_doc();
    run(&mut s, TOOLS).unwrap();
    let id = only_palette(&s);
    s.execute("edit.undoGroup.begin", &json!({})).unwrap();
    let e = s.execute("script.palette.event", &json!({"palette": id, "event": "click", "control": 1})).unwrap_err().to_string();
    assert!(e.contains("undo group is open"), "{e}");
    assert_eq!(s.script_palette_events_waiting(), 0, "the refused click does not wait");
    s.execute("edit.undoGroup.end", &json!({})).unwrap();
    let clicked = s.execute("script.palette.event", &json!({"palette": id, "event": "click", "control": 1})).unwrap();
    assert_eq!(clicked["log"], json!(["click 1"]), "only the click given after the group ran");
}

#[test]
fn with_nobody_to_ask_an_event_whose_run_cannot_begin_ends_with_its_error_and_the_next_waits() {
    let mut s = one_doc();
    run(&mut s, TOOLS).unwrap();
    let id = only_palette(&s);
    assert_eq!(s.script_palette_event(id, UiEvent::Change(2, crate::script::UiValue::Text("a".into()))), Ok(Waiting::Added));
    // The document has changes, and the session has nowhere to keep a recovery copy of it.
    s.script_copy_first = true;
    let e = s.execute("script.palette.event", &json!({"palette": id, "event": "click", "control": 1})).unwrap_err().to_string();
    assert!(e.contains("recovery copy"), "{e}");
    assert_eq!(s.script_palette_events_waiting(), 1, "the click after the change whose run could not begin still waits");
    s.script_copy_first = false;
    assert_eq!(s.script_palette_run_waiting().unwrap()["log"], json!(["click 1"]));
}

#[test]
fn a_handler_s_error_keeps_its_file_when_a_palette_getter_throws_after_it() {
    let base = std::env::temp_dir().join(format!("vc-script-palette-place-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let base = std::fs::canonicalize(&base).unwrap();
    let first = base.join("first.jsx");
    std::fs::write(
        &first,
        "#targetengine places\nvar armed = false; var p = new Window('palette'); var b = p.add('button', undefined, 'Go');\n\
         Object.defineProperty(p, 'text', {get: function () { if (armed) { throw new Error('getter'); } return 'P'; }});\np.show();",
    )
    .unwrap();
    let second = base.join("second.jsx");
    std::fs::write(&second, "#targetengine places\n\n\nb.onClick = function () { armed = true; throw new Error('handler'); };").unwrap();
    let mut s = one_doc();
    for file in [&first, &second] {
        s.execute("script.run", &json!({"path": file.to_string_lossy()})).unwrap();
    }
    let id = only_palette(&s);
    let e = s.execute("script.palette.event", &json!({"palette": id, "event": "click", "control": 1})).unwrap_err().to_string();
    assert!(e.contains("handler"), "{e}");
    let report = s.script_report().unwrap();
    assert!(report["file"].as_str().is_some_and(|f| f.ends_with("second.jsx")), "{report}");
    assert_eq!(report["line"], 4, "{report}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_palette_from_a_script_without_a_named_engine_is_refused() {
    let mut s = one_doc();
    for source in ["", "#targetengine main\n"] {
        let r = run(&mut s, &format!("{source}try {{ new Window('palette'); }} catch (e) {{ vc.log(e.message); }}")).unwrap();
        assert_eq!(r["log"], json!([PALETTE_NEEDS_ENGINE]), "{source:?}");
    }
    assert!(s.script_palettes().is_empty());
}

#[test]
fn close_fires_on_close_and_frees_the_window() {
    let mut s = one_doc();
    run(&mut s, &format!("{TOOLS} p.onClose = function () {{ vc.log('closing'); }};")).unwrap();
    let id = only_palette(&s);
    let closed = s.execute("script.palette.event", &json!({"palette": id, "event": "close"})).unwrap();
    assert_eq!(closed["log"], json!(["closing"]));
    assert!(s.script_palettes().is_empty(), "the close box freed the palette");
    let e = s.execute("script.palette.event", &json!({"palette": id, "event": "click", "control": 1})).unwrap_err().to_string();
    assert!(e.contains(&format!("there is no script palette {id}")), "{e}");

    run(&mut s, TOOLS).unwrap();
    let r =
        run(&mut s, "#targetengine tools\nvar asked = 0; p.onClose = function () { asked++; return asked > 1; }; p.close(); vc.log(asked);").unwrap();
    assert_eq!(r["log"], json!([1]));
    assert_eq!(s.script_palettes().len(), 1, "onClose returned false: the palette stays open");
    run(&mut s, "#targetengine tools\np.close();").unwrap();
    assert!(s.script_palettes().is_empty(), "close() freed the palette");
}

#[test]
fn dropping_the_engine_closes_its_palettes() {
    let mut s = one_doc();
    run_in_app(&mut s, TOOLS);
    run_in_app(&mut s, "#targetengine other\nnew Window('palette').show();");
    let id = s.script_palettes()[0].id;
    s.script_palette_event(id, UiEvent::Click(1)).unwrap();
    s.execute("script.engine.drop", &json!({"name": "tools"})).unwrap();
    let left: Vec<&str> = s.script_palettes().iter().map(|p| p.engine.as_str()).collect();
    assert_eq!(left, ["other"]);
    assert_eq!(s.script_palette_events_waiting(), 0, "the dropped palette's event no longer waits");
    assert!(s.script_palette_start().is_none());
}

#[test]
fn an_erroring_handler_ends_its_run_and_leaves_the_palette_open() {
    let mut s = one_doc();
    let before = s.documents()[0].history.undo.len();
    run(
        &mut s,
        "#targetengine tools\nvar p = new Window('palette'); var b = p.add('button', undefined, 'Go'); \
        b.onClick = function () { vc.run('shape.rectangle', {x: 20, y: 0, width: 5, height: 5}); throw new Error('broken'); }; p.show();",
    )
    .unwrap();
    let id = only_palette(&s);
    let e = s.execute("script.palette.event", &json!({"palette": id, "event": "click", "control": 1})).unwrap_err().to_string();
    assert!(e.contains("broken"), "{e}");
    assert_eq!(s.documents()[0].history.undo.len(), before, "the failed handler's rectangle was rolled back");
    assert_eq!(only_palette(&s), id);
}

#[test]
fn a_dialog_in_a_handler_throws_in_the_app() {
    let mut s = one_doc();
    run_in_app(
        &mut s,
        "#targetengine tools\nvar p = new Window('palette'); var b = p.add('button', undefined, 'Go'); \
         b.onClick = function () { try { confirm('Sure?'); } catch (e) { vc.log('confirm'); } \
         try { var d = new Window('dialog'); d.add('button', undefined, 'OK'); d.show(); } catch (e) { vc.log('dialog'); } }; p.show();",
    );
    let id = only_palette(&s);
    assert_eq!(s.script_palette_event(id, UiEvent::Click(1)), Ok(Waiting::Added));
    assert!(s.script_palette_start().expect("the click starts its run").is_ok());
    assert_eq!(pump(&mut s).expect("no dialog waits").unwrap()["log"], json!(["confirm", "dialog"]));
    assert_eq!(only_palette(&s), id);
}

#[test]
fn a_handler_with_nobody_to_ask_ends_at_its_dialog() {
    let mut s = one_doc();
    run(
        &mut s,
        "#targetengine tools\nvar p = new Window('palette'); p.add('button', undefined, 'Go').onClick = function () { alert('hi'); }; p.show();",
    )
    .unwrap();
    let id = only_palette(&s);
    let e = s.execute("script.palette.event", &json!({"palette": id, "event": "click", "control": 1})).unwrap_err().to_string();
    assert!(e.contains("no answer left"), "{e}");
    assert_eq!(only_palette(&s), id, "the palette stays open");
}

#[test]
fn palettes_and_waiting_events_have_limits() {
    let mut s = one_doc();
    let r = run(
        &mut s,
        "#targetengine many\nfor (var i = 0; i < 40; i++) { try { new Window('palette').show(); } catch (e) { vc.log([i, e.name]); break; } }",
    )
    .unwrap();
    assert_eq!(r["log"], json!([[MAX_PALETTES, "RangeError"]]));
    s.execute("script.engine.drop", &json!({"name": "many"})).unwrap();
    assert!(s.script_palettes().is_empty());

    run_in_app(
        &mut s,
        "#targetengine tools\nvar p = new Window('palette'); for (var i = 0; i < 70; i++) p.add('button', undefined, 'B' + i); p.show();",
    );
    let id = only_palette(&s);
    assert_eq!(run(&mut s, "confirm('Go?');").unwrap()["running"], true);
    assert!(pump(&mut s).is_none());
    for c in 1..=MAX_WAITING {
        assert_eq!(s.script_palette_event(id, UiEvent::Click(c)), Ok(Waiting::Added));
    }
    let e = s.script_palette_event(id, UiEvent::Click(MAX_WAITING + 1)).unwrap_err();
    assert!(e.contains("this one was not kept"), "{e}");
    assert_eq!(s.script_palette_event(id, UiEvent::Click(1)), Ok(Waiting::Merged), "a waiting control still takes a click");
}
