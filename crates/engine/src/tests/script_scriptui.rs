//! ScriptUI dialogs in a script: the description the engine checks and lays out, the `window`
//! answers a run with nobody to ask takes from `answers`, the wait in the app, Stop, and the
//! report's `dialogs`.

use serde_json::{Value, json};
use vectorcraft_js::{Data, HostError};

use crate::script::scriptui::{self, MAX_CONTROLS, MAX_DEPTH, MAX_EXTENT, MAX_TEXT_BYTES};
use crate::script::{ScriptUiDialog, UiEvent, UiKind, UiValue};
use crate::*;

/// One open document with one rectangle.
fn one_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s
}

fn art(s: &Session) -> (String, usize) {
    let d = &s.documents()[0];
    (format!("{:?}", d.doc.layers), d.history.undo.len())
}

fn headless(s: &mut Session, source: &str, answers: Value) -> Result<Value> {
    s.execute("script.run", &json!({"source": source, "name": "w.js", "answers": answers}))
}

/// A run in the app of `source`, one slice in.
fn framed(s: &mut Session, source: &str) {
    s.script_frames = true;
    assert_eq!(s.execute("script.run", &json!({"source": source, "name": "w.js"})).unwrap()["running"], true);
    assert!(s.script_pump().is_none());
}

/// Pumps the run in the app until it ends or waits at a ScriptUI dialog (`None`).
fn pump(s: &mut Session) -> Option<Result<Value>> {
    for _ in 0..10_000 {
        if s.script_window().is_some() {
            return None;
        }
        if let Some(r) = s.script_pump() {
            return Some(r);
        }
    }
    panic!("the run neither ended nor waited");
}

fn window(steps: Value) -> Value {
    json!([{"kind": "window", "value": steps}])
}

const EDIT: &str = "vc.run('shape.rectangle', {x: 20, y: 0, width: 5, height: 5});";

/// A dialog with a field, a check box, a slider, a list and OK and Cancel.
const FORM: &str = "var w = new Window('dialog', 'Settings'); var g = w.add('group'); \
    g.add('statictext', undefined, 'Name:'); var name = g.add('edittext', undefined, 'first', {name: 'name'}); \
    var bold = w.add('checkbox', undefined, 'Bold'); var size = w.add('slider', undefined, 5, 0, 10); size.name = 'size'; \
    var pick = w.add('dropdownlist', undefined, ['a', 'b', 'c']); pick.name = 'pick'; \
    var ok = w.add('button', undefined, 'OK'); w.add('button', undefined, 'Cancel');";

#[test]
fn a_run_with_nobody_to_ask_takes_the_steps_of_its_window_answer() {
    let mut s = one_doc();
    let source = format!(
        "{EDIT} {FORM} bold.onClick = function () {{ name.text = 'bolded'; }}; var r = w.show(); vc.log([r, name.text, bold.value, size.value, pick.selection.text]);"
    );
    let steps = json!([
        {"control": "Bold", "event": "click"},
        {"control": "size", "event": "change", "value": 99},
        {"control": "pick", "event": "change", "value": 2},
        {"control": "OK", "event": "click"},
    ]);
    let r = headless(&mut s, &source, window(steps)).unwrap();
    assert_eq!(r["log"], json!([[1, "bolded", true, 10, "c"]]), "the slider's value is clamped to its range: {r}");
    assert_eq!(r["dialogs"], json!([{"kind": "window", "title": "Settings", "controls": 8, "events": 4, "answer": 1}]));
    let cancel = headless(&mut s, &format!("{FORM} vc.log(w.show());"), window(json!([{"control": "Cancel", "event": "click"}]))).unwrap();
    assert_eq!(cancel["log"], json!([2]));
    let closed = headless(&mut s, &format!("{FORM} vc.log(w.show());"), window(json!([{"event": "close"}]))).unwrap();
    assert_eq!(closed["log"], json!([0]));
}

#[test]
fn a_missing_wrong_or_unused_step_ends_the_run_and_rolls_it_back() {
    let mut s = one_doc();
    let before = art(&s);
    let source = format!("{EDIT} {FORM} try {{ w.show(); }} catch (e) {{ vc.log('caught'); }}");
    for (given, says) in [
        (json!([]), "`answers` has no answer left for this dialog"),
        (json!([{"kind": "alert", "value": "ok"}]), "is for alert, not window"),
        (window(json!([])), "has no step left"),
        (window(json!([{"control": "Nothing", "event": "click"}])), "the dialog has no control named Nothing"),
        (window(json!([{"control": "Name:", "event": "click"}])), "cannot be clicked"),
        (window(json!([{"control": "pick", "event": "change", "value": 3}])), "has no item 3"),
        (window(json!([{"control": "OK", "event": "click"}, {"event": "close"}])), "has 1 step the dialog did not use"),
    ] {
        let e = headless(&mut s, &source, given.clone()).unwrap_err().to_string();
        assert!(e.contains(says), "{given}: {e}");
        assert_eq!(art(&s), before, "{given}");
        let report = s.script_report().unwrap();
        assert_eq!(report["log"], json!([]), "a missing answer cannot be caught: {report}");
        assert_eq!(report["dialogs"][0]["answer"], "missing", "{report}");
    }
}

#[test]
fn window_answers_that_do_not_parse_are_refused_before_the_run() {
    let mut s = one_doc();
    let bad = |i: usize| format!("`answers[0]` has a step {i} that is not a valid click, change or close");
    for (steps, says) in [
        (json!("click"), "`answers[0]` is a window's answer, whose `value` must be a list of steps".to_string()),
        (json!([1]), bad(0)),
        (json!([{"event": "close"}, {"event": "tap", "control": "OK"}]), bad(1)),
        (json!([{"event": "click"}]), bad(0)),
        (json!([{"event": "click", "control": "OK", "value": 1}]), bad(0)),
        (json!([{"event": "change", "control": "x"}]), bad(0)),
        (json!([{"event": "change", "control": "x", "value": [1]}]), bad(0)),
        (json!([{"event": "change", "control": "x".repeat(MAX_TEXT_BYTES + 1), "value": 1}]), bad(0)),
        (json!([{"event": "close", "control": "x"}]), bad(0)),
        (json!([{"event": "close", "extra": 1}]), bad(0)),
    ] {
        let e = headless(&mut s, "1", window(steps.clone())).unwrap_err().to_string();
        assert!(e.contains(&says), "{steps}: {e}");
    }
}

#[test]
fn a_framed_run_waits_at_show_and_runs_each_handler_with_the_persons_events() {
    let mut s = one_doc();
    let undo_before = art(&s).1;
    framed(
        &mut s,
        &format!("{EDIT} {FORM} bold.onClick = function () {{ size.enabled = !bold.value; }}; var r = w.show(); vc.log([r, name.text, size.value]);"),
    );
    let d = s.script_window().expect("show waits");
    assert_eq!(d.title(), "Settings");
    assert_eq!(d.nodes.len(), 9);
    assert_eq!((d.nodes[3].kind, d.nodes[3].text.as_str()), (UiKind::EditText, "first"));
    let bold = d.find("Bold").unwrap();
    let size = d.find("size").unwrap();
    assert!(s.script_window_answer(UiEvent::Click(d.find("Name:").unwrap())).is_err(), "static text takes no click");
    assert!(s.script_window_answer(UiEvent::Change(3, UiValue::Text("x".repeat(MAX_TEXT_BYTES + 1)))).is_err());
    assert!(s.script_window_answer(UiEvent::Click(99)).is_err());
    s.script_window_answer(UiEvent::Click(bold)).unwrap();
    assert!(s.script_window_answer(UiEvent::Close).is_err(), "nothing waits until the run goes on");
    assert!(pump(&mut s).is_none());
    let d = s.script_window().unwrap();
    assert!(d.nodes[bold].checked);
    assert!(!d.nodes[size].enabled, "the handler disabled the slider");
    assert!(s.script_window_answer(UiEvent::Change(size, UiValue::Number(3.0))).is_err(), "a disabled control takes no events");
    s.script_window_answer(UiEvent::Change(3, UiValue::Text("Ann".into()))).unwrap();
    assert!(pump(&mut s).is_none());
    s.script_window_answer(UiEvent::Click(d.default_element.unwrap())).unwrap();
    let r = pump(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!([[1, "Ann", 5]]));
    assert_eq!(r["dialogs"][0]["events"], 3);
    assert_eq!(art(&s).1, undo_before + 1, "a finished run is one undo step");
    assert_eq!(s.documents()[0].history.undo.last().unwrap().label, "Run Script");
}

#[test]
fn an_unused_step_after_a_handler_threw_ends_the_run_and_rolls_it_back() {
    let mut s = one_doc();
    let before = art(&s);
    let source = format!(
        "{EDIT} var w = new Window('dialog'); var f = w.add('button', undefined, 'Fail'); f.onClick = function () {{ throw new Error('boom'); }}; \
         try {{ w.show(); }} catch (e) {{ vc.log('caught'); }}"
    );
    let steps = window(json!([{"control": "Fail", "event": "click"}, {"event": "close"}]));
    let e = headless(&mut s, &source, steps).unwrap_err().to_string();
    assert!(e.contains("has 1 step the dialog did not use"), "{e}");
    assert_eq!(art(&s), before, "the run is rolled back");
}

#[test]
fn a_list_index_written_as_a_whole_float_is_an_index_and_a_fraction_is_refused() {
    let mut s = one_doc();
    let source = format!("{FORM} vc.log([w.show(), pick.selection.text]);");
    let r = headless(&mut s, &source, window(json!([{"control": "pick", "event": "change", "value": 1.0}, {"control": "OK", "event": "click"}])))
        .unwrap();
    assert_eq!(r["log"], json!([[1, "b"]]));
    let e = headless(&mut s, &source, window(json!([{"control": "pick", "event": "change", "value": 1.5}]))).unwrap_err().to_string();
    assert!(e.contains("a list's `value` must be an item index or null"), "{e}");
}

#[test]
fn a_progress_bar_s_fraction_stays_finite_over_the_widest_range() {
    let bar = |min: f64, max: f64, value: f64| {
        let n =
            with(with(with(node("progressbar", 0.0), "minvalue", Data::Number(min)), "maxvalue", Data::Number(max)), "value", Data::Number(value));
        scriptui::parse(&description(vec![node("dialog", -1.0), n])).unwrap().nodes[1].fraction()
    };
    assert_eq!(bar(-1e308, 1e308, 1e308), 1.0);
    assert_eq!(bar(-1e308, 1e308, -1e308), 0.0);
    assert_eq!(bar(-1e308, 1e308, 0.0), 0.5);
    assert_eq!(bar(0.0, 10.0, 3.0), 0.3);
    assert_eq!(bar(5.0, 5.0, 5.0), 0.0, "an empty range shows nothing done");
}

#[test]
fn stop_while_show_waits_ends_the_run_and_rolls_it_back() {
    let mut s = one_doc();
    let before = art(&s);
    framed(&mut s, &format!("{EDIT} {FORM} w.show(); vc.log('after');"));
    assert!(s.script_window().is_some());
    assert_eq!(s.execute("script.stop", &json!({})).unwrap()["stopped"], true);
    assert_eq!(s.script_window(), None, "a run asked to stop no longer waits");
    assert!(s.script_window_answer(UiEvent::Close).is_err());
    assert!(s.script_pump().unwrap().is_err());
    let report = s.script_report().unwrap();
    assert_eq!((report["kind"].as_str(), &report["log"]), (Some("stopped"), &json!([])), "{report}");
    assert_eq!(report["dialogs"][0]["answer"], "stop");
    assert_eq!(art(&s), before);
    framed(&mut s, &format!("{EDIT} {FORM} w.show();"));
    assert!(s.script_abandon(), "quitting while a dialog waits abandons the run");
    assert_eq!(art(&s), before);
}

#[test]
fn a_handler_that_loops_ends_the_run_at_its_fuel_and_rolls_it_back() {
    let mut s = one_doc();
    let before = art(&s);
    let source = format!("{EDIT} {FORM} bold.onClick = function () {{ for (;;) {{}} }}; w.show();");
    let e = headless(&mut s, &source, window(json!([{"control": "Bold", "event": "click"}]))).unwrap_err().to_string();
    assert!(e.contains("limit") || e.contains("fuel") || e.contains("instructions"), "{e}");
    assert_eq!(art(&s), before);
    assert_eq!(s.script_report().unwrap()["dialogs"][0]["answer"], "error");
}

#[test]
fn a_show_in_a_callback_throws_in_the_app() {
    let mut s = one_doc();
    s.script_frames = true;
    let source = "var w = new Window('dialog'); w.add('button', undefined, 'OK'); var e = ''; try { [1].forEach(function () { w.show(); }); } catch (x) { e = x.message; } vc.log(e);";
    assert_eq!(s.execute("script.run", &json!({"source": source, "name": "w.js"})).unwrap()["running"], true);
    let r = pump(&mut s).expect("show does not wait in a callback").unwrap();
    assert!(r["log"][0].as_str().unwrap().contains("cannot open inside a callback"), "{r}");
}

fn node(kind: &str, parent: f64) -> Data {
    Data::Object(vec![
        ("kind".into(), Data::String(kind.into())),
        ("parent".into(), Data::Number(parent)),
        ("name".into(), Data::String(String::new())),
        ("text".into(), Data::String("t".into())),
        ("enabled".into(), Data::Bool(true)),
        ("visible".into(), Data::Bool(true)),
    ])
}

fn description(nodes: Vec<Data>) -> Data {
    Data::Object(vec![
        ("nodes".into(), Data::Array(nodes)),
        ("defaultElement".into(), Data::Number(-1.0)),
        ("cancelElement".into(), Data::Number(-1.0)),
    ])
}

fn with(n: Data, name: &str, value: Data) -> Data {
    let Data::Object(mut fields) = n else { panic!("a node is an object") };
    fields.retain(|(k, _)| k != name);
    fields.push((name.into(), value));
    Data::Object(fields)
}

fn refused(d: &Data) -> String {
    match scriptui::parse(d) {
        Err(HostError::Type(m) | HostError::Range(m) | HostError::Error(m)) => m,
        other => panic!("not refused: {other:?}"),
    }
}

#[test]
fn a_description_out_of_order_too_large_too_deep_or_too_long_is_refused() {
    let dialog = node("dialog", -1.0);
    assert!(refused(&description(vec![])).contains("has no dialog"));
    assert!(refused(&description(vec![node("group", -1.0)])).contains("does not start with the dialog"));
    assert!(refused(&description(vec![dialog.clone(), node("button", 2.0), node("group", 0.0)])).contains("before its container"));
    assert!(refused(&description(vec![dialog.clone(), node("button", 1.0)])).contains("before its container"), "a control cannot hold itself");
    assert!(refused(&description(vec![dialog.clone(), node("button", 0.0), node("button", 1.0)])).contains("holds none"));
    assert!(refused(&description(vec![dialog.clone(), node("dialog", 0.0)])).contains("before its container"));
    let many: Vec<Data> = std::iter::once(dialog.clone()).chain((0..=MAX_CONTROLS).map(|_| node("button", 0.0))).collect();
    assert!(refused(&description(many)).contains(&format!("at most {MAX_CONTROLS} controls")));
    let deep: Vec<Data> = std::iter::once(dialog.clone()).chain((0..=MAX_DEPTH).map(|k| node("group", k as f64))).collect();
    assert!(refused(&description(deep)).contains(&format!("at most {MAX_DEPTH} deep")));
    let long = with(node("statictext", 0.0), "text", Data::String("x".repeat(MAX_TEXT_BYTES + 1)));
    assert!(refused(&description(vec![dialog.clone(), long])).contains("longer than 16 KiB"));
    let slider = with(with(node("slider", 0.0), "minvalue", Data::Number(5.0)), "maxvalue", Data::Number(1.0));
    assert!(refused(&description(vec![dialog.clone(), slider])).contains("minvalue is larger"));
    let nan = with(node("button", 0.0), "bounds", Data::Array(vec![Data::Number(0.0), Data::Number(f64::NAN), Data::Number(1.0), Data::Number(1.0)]));
    assert!(refused(&description(vec![dialog.clone(), nan])).contains("not a number"));
    let orient = with(dialog.clone(), "orientation", Data::String("diagonal".into()));
    assert!(refused(&description(vec![orient])).contains("row, column or stack"));
    let align = with(node("button", 0.0), "alignment", Data::String("sideways".into()));
    assert!(refused(&description(vec![dialog, align])).contains("no alignment sideways"));
}

#[test]
fn sizes_and_places_are_clamped_and_the_layout_follows_the_containers() {
    let dialog = with(with(node("dialog", -1.0), "orientation", Data::String("column".into())), "margins", Data::Array(vec![Data::Number(10.0); 4]));
    let row = with(with(node("group", 0.0), "orientation", Data::String("row".into())), "spacing", Data::Number(5.0));
    let a = with(node("button", 1.0), "size", Data::Array(vec![Data::Number(100.0), Data::Number(20.0)]));
    let b = with(node("button", 1.0), "size", Data::Array(vec![Data::Number(50.0), Data::Number(30.0)]));
    let huge =
        with(node("statictext", 0.0), "bounds", Data::Array(vec![Data::Number(-1e12), Data::Number(0.0), Data::Number(1e12), Data::Number(1e300)]));
    let d: ScriptUiDialog = scriptui::parse(&description(vec![dialog, row, a, b, huge])).unwrap();
    assert_eq!((d.nodes[1].rect.w, d.nodes[1].rect.h), (155.0, 30.0), "a row adds widths and spacing, and takes the tallest height");
    assert_eq!((d.nodes[2].rect.x, d.nodes[3].rect.x), (10.0, 115.0));
    assert_eq!(d.nodes[2].rect.y, 15.0, "a shorter control is centered in its row");
    for n in &d.nodes {
        for v in [n.rect.x, n.rect.y, n.rect.w, n.rect.h] {
            assert!(v.is_finite() && v.abs() <= MAX_EXTENT, "{:?}", n.rect);
        }
    }
    let (w, h) = d.size();
    assert!(w <= MAX_EXTENT && h <= MAX_EXTENT);
}

#[test]
fn a_control_s_size_keeps_within_its_minimum_and_maximum() {
    let dialog = node("dialog", -1.0);
    let pair = |w: f64, h: f64| Data::Array(vec![Data::Number(w), Data::Number(h)]);
    let wide = with(with(node("button", 0.0), "size", pair(300.0, -1.0)), "max", pair(120.0, -1.0));
    let tall = with(with(node("button", 0.0), "size", pair(-1.0, -1.0)), "min", pair(-1.0, 60.0));
    let d: ScriptUiDialog = scriptui::parse(&description(vec![dialog.clone(), wide, tall])).unwrap();
    assert_eq!(d.nodes[1].rect.w, 120.0, "the maximum width holds over a larger size");
    assert_eq!(d.nodes[2].rect.h, 60.0, "the minimum height holds over the estimate");
    let plain: ScriptUiDialog = scriptui::parse(&description(vec![dialog, node("button", 0.0)])).unwrap();
    assert!(plain.nodes[1].rect.h < 60.0 && plain.nodes[1].rect.w > 0.0, "a size of -1 leaves the side to the estimate");
    let column = with(with(node("dialog", -1.0), "orientation", Data::String("column".into())), "size", pair(300.0, -1.0));
    let fill = |n: Data| with(n, "alignment", Data::String("fill".into()));
    let capped = with(fill(node("button", 0.0)), "max", pair(90.0, -1.0));
    let d: ScriptUiDialog = scriptui::parse(&description(vec![column, capped, fill(node("button", 0.0))])).unwrap();
    assert!(d.nodes[2].rect.w > 90.0, "a control that fills its column takes the column's width");
    assert_eq!(d.nodes[1].rect.w, 90.0, "a control that fills its column stays within its maximum width");
}

#[test]
fn window_s_alert_confirm_and_prompt_open_the_script_dialogs() {
    let mut s = one_doc();
    let answers = json!([{"kind": "alert", "value": "ok"}, {"kind": "confirm", "value": false}, {"kind": "prompt", "value": "typed"}]);
    let r = headless(&mut s, "vc.log([typeof Window.alert('Hi'), Window.confirm('Go?'), Window.prompt('Name', 'x')]);", answers).unwrap();
    assert_eq!(r["log"], json!([["undefined", false, "typed"]]), "{r}");
    assert_eq!(r["dialogCount"], 3);
}

/// A description of arbitrary kinds, containers in any order, numbers of any size (infinite and
/// not-a-number ones included) and texts of any length.
fn any_description() -> impl proptest::strategy::Strategy<Value = Data> {
    use proptest::prelude::*;
    let kind = prop_oneof![
        Just("dialog"),
        Just("group"),
        Just("panel"),
        Just("statictext"),
        Just("edittext"),
        Just("button"),
        Just("checkbox"),
        Just("radiobutton"),
        Just("dropdownlist"),
        Just("listbox"),
        Just("slider"),
        Just("progressbar"),
        Just("wobble"),
    ];
    let number = prop_oneof![1 => Just(f64::NAN), 1 => Just(f64::INFINITY), 1 => Just(-1e300), 1 => any::<f64>(), 6 => -50.0..400.0f64];
    let text = prop_oneof![2 => Just(String::new()), 8 => "[ -~]{0,40}", 1 => Just("x".repeat(MAX_TEXT_BYTES + 1))];
    let bounds = proptest::option::of(proptest::collection::vec(number.clone(), 4..=4));
    let size = proptest::option::of(proptest::collection::vec(number.clone(), 2..=2));
    // The container is a share of the controls before this one, or with a share past 1, one after it.
    let fields = (kind, 0.0..1.1f64, text, bounds, size, 0..4usize, number);
    proptest::collection::vec(fields, 0..40).prop_map(|nodes| {
        let mut containers = vec![0usize];
        let mut out = Vec::new();
        for (i, (kind, share, text, bounds, size, items, value)) in nodes.into_iter().enumerate() {
            let kind = if i == 0 { "dialog" } else { kind };
            let parent = match i {
                0 => -1.0,
                _ if share >= 1.0 => (i + 1) as f64,
                _ => containers[((share * containers.len() as f64) as usize).min(containers.len() - 1)] as f64,
            };
            if matches!(kind, "group" | "panel") {
                containers.push(i);
            }
            let numbers = |v: Option<Vec<f64>>| v.map_or(Data::Null, |v| Data::Array(v.into_iter().map(Data::Number).collect()));
            let mut n = with(with(node(kind, parent), "text", Data::String(text)), "bounds", numbers(bounds));
            n = with(n, "size", numbers(size));
            n = with(n, "items", Data::Array((0..items).map(|k| Data::String(k.to_string())).collect()));
            n = with(n, "selection", Data::Number(value));
            out.push(with(with(n, "minvalue", Data::Number(0.0)), "value", Data::Number(value)));
        }
        description(out)
    })
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

    /// Any description is refused with an error or laid out with every place and size finite and
    /// within the limit.
    #[test]
    fn any_description_is_refused_or_laid_out_within_its_limits(d in any_description()) {
        if let Ok(dialog) = scriptui::parse(&d) {
            for n in &dialog.nodes {
                for v in [n.rect.x, n.rect.y, n.rect.w, n.rect.h] {
                    proptest::prop_assert!(v.is_finite() && v.abs() <= MAX_EXTENT, "{:?}", n.rect);
                }
                proptest::prop_assert!(n.value.is_finite() && n.selection.is_none_or(|k| k < n.items.len()));
            }
        }
    }
}

#[test]
fn an_event_is_checked_against_the_dialog() {
    let d = scriptui::parse(&description(vec![
        node("dialog", -1.0),
        node("button", 0.0),
        with(node("edittext", 0.0), "readonly", Data::Bool(true)),
        with(with(with(node("slider", 0.0), "minvalue", Data::Number(0.0)), "maxvalue", Data::Number(10.0)), "value", Data::Number(4.0)),
        with(node("listbox", 0.0), "items", Data::Array(vec![Data::String("a".into())])),
        with(node("button", 0.0), "visible", Data::Bool(false)),
    ]))
    .unwrap();
    assert_eq!(scriptui::check_event(&d, UiEvent::Click(1)), Ok(UiEvent::Click(1)));
    assert!(scriptui::check_event(&d, UiEvent::Click(0)).is_err(), "the dialog itself is no control");
    assert!(scriptui::check_event(&d, UiEvent::Change(2, UiValue::Text("x".into()))).is_err(), "a read-only field takes no text");
    assert_eq!(scriptui::check_event(&d, UiEvent::Change(3, UiValue::Number(-5.0))), Ok(UiEvent::Change(3, UiValue::Number(0.0))));
    assert!(scriptui::check_event(&d, UiEvent::Change(3, UiValue::Number(f64::NAN))).is_err());
    assert!(scriptui::check_event(&d, UiEvent::Change(4, UiValue::Index(Some(1)))).is_err());
    assert_eq!(scriptui::check_event(&d, UiEvent::Change(4, UiValue::Index(None))), Ok(UiEvent::Change(4, UiValue::Index(None))));
    assert!(scriptui::check_event(&d, UiEvent::Click(5)).is_err(), "a hidden control takes no events");
    assert_eq!(scriptui::check_event(&d, UiEvent::Close), Ok(UiEvent::Close));
}
