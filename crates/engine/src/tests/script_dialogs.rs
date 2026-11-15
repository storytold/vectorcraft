//! `alert`, `confirm` and `prompt` in a script: their arguments and results, the answers a run with
//! nobody to ask takes from `answers`, the wait in the app, Stop, and the report's `dialogs`.

use serde_json::{Value, json};

use crate::script::{DialogAnswer, DialogKind, ScriptDialog, dialogs};
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
    s.execute("script.run", &json!({"source": source, "name": "d.js", "answers": answers}))
}

/// A run in the app of `source`, one slice in.
fn framed(s: &mut Session, source: &str) {
    s.script_frames = true;
    assert_eq!(s.execute("script.run", &json!({"source": source, "name": "d.js"})).unwrap()["running"], true);
    assert!(s.script_pump().is_none());
}

/// Pumps the run in the app until it ends or waits at a dialog (`None`).
fn pump(s: &mut Session) -> Option<Result<Value>> {
    for _ in 0..10_000 {
        if s.script_dialog().is_some() {
            return None;
        }
        if let Some(r) = s.script_pump() {
            return Some(r);
        }
    }
    panic!("the run neither ended nor waited");
}

fn answers(list: &[Value]) -> Value {
    Value::Array(list.to_vec())
}

#[test]
fn notification_signatures_and_returns() {
    let mut s = one_doc();
    let source = "vc.log(typeof alert('a')); alert(42, 'Title', 1);\
        vc.log(confirm('c')); vc.log(confirm('n', true, undefined));\
        vc.log(prompt('p', 'pre')); vc.log(prompt({toString: function () { return 'own'; }}, '', 'T'));";
    let r = headless(
        &mut s,
        source,
        answers(&[
            json!({"kind": "alert", "value": "ok"}),
            json!({"kind": "alert", "value": "ok"}),
            json!({"kind": "confirm", "value": true}),
            json!({"kind": "confirm", "value": false}),
            json!({"kind": "prompt", "value": "typed"}),
            json!({"kind": "prompt", "value": null}),
        ]),
    )
    .unwrap();
    assert_eq!(r["log"], json!(["undefined", true, false, "typed", null]), "{r}");
    assert_eq!(
        r["dialogs"],
        json!([
            {"kind": "alert", "message": "a", "title": "Script Alert", "errorIcon": false, "answer": "ok"},
            {"kind": "alert", "message": "42", "title": "Title", "errorIcon": true, "answer": "ok"},
            {"kind": "confirm", "message": "c", "title": "Script Alert", "noAsDefault": false, "answer": true},
            {"kind": "confirm", "message": "n", "title": "Script Alert", "noAsDefault": true, "answer": false},
            {"kind": "prompt", "message": "p", "title": "Script Alert", "preset": "pre", "answer": "typed"},
            {"kind": "prompt", "message": "own", "title": "T", "preset": "", "answer": null},
        ])
    );
    assert_eq!((r["dialogCount"].clone(), r["dialogsTruncated"].clone()), (json!(6), json!(false)));
    let has = headless(&mut s, "vc.log(vc.api.has('dialogs'));", json!([])).unwrap();
    assert_eq!(has["log"], json!([true]));
}

#[test]
fn headless_answers_follow_call_order() {
    let mut s = one_doc();
    let source = "var out = []; [1, 2].forEach(function (i) { out.push(confirm('q' + i)); }); out.push(prompt('r', 'x')); vc.log(out.join());";
    let r = headless(
        &mut s,
        source,
        answers(&[json!({"kind": "confirm", "value": false}), json!({"kind": "confirm", "value": true}), json!({"kind": "prompt", "value": "z"})]),
    )
    .unwrap();
    assert_eq!(r["log"], json!(["false,true,z"]), "a callback of a built-in takes its answer too: {r}");
    assert_eq!(headless(&mut s, "1", json!({})).unwrap()["dialogCount"], 0, "an empty object is an empty list");
}

#[test]
fn missing_or_mismatched_answer_rolls_back() {
    let mut s = one_doc();
    let before = art(&s);
    let edit = "vc.run('shape.rectangle', {x: 20, y: 0, width: 5, height: 5});";
    for (source, given, says) in [
        (format!("{edit} try {{ alert('x'); }} catch (e) {{ vc.log('caught'); }}"), json!([]), "no answer left"),
        (format!("{edit} alert('x');"), answers(&[json!({"kind": "confirm", "value": true})]), "is for confirm, not alert"),
        (format!("{edit} alert('x');"), answers(&[json!({"kind": "alert", "value": "ok"}), json!({"kind": "alert", "value": "ok"})]), "1 answer"),
    ] {
        let e = headless(&mut s, &source, given).unwrap_err().to_string();
        assert!(e.contains(says), "{e}");
        assert_eq!(art(&s), before, "{source}");
        let report = s.script_report().unwrap();
        assert_eq!(report["kind"], "limit", "{report}");
        assert_eq!(report["log"], json!([]), "a missing answer cannot be caught");
        assert_eq!(report["dialogCount"], 1);
    }
    assert_eq!(s.script_report().unwrap()["dialogs"][0]["answer"], "ok");
    headless(&mut s, "alert('x')", json!([])).unwrap_err();
    assert_eq!(s.script_report().unwrap()["dialogs"][0]["answer"], "missing");
}

#[test]
fn invalid_answers_are_rejected_before_run() {
    let mut s = one_doc();
    let before = art(&s);
    let many: Vec<Value> = (0..=dialogs::MAX_ANSWERS).map(|_| json!({"kind": "alert", "value": "ok"})).collect();
    let big: Vec<Value> = (0..80).map(|_| json!({"kind": "prompt", "value": "x".repeat(16_000)})).collect();
    for given in [
        json!({"a": 1}),
        json!("ok"),
        json!([1]),
        json!([{"kind": "alert"}]),
        json!([{"kind": "alert", "value": true}]),
        json!([{"kind": "confirm", "value": "yes"}]),
        json!([{"kind": "prompt", "value": 3}]),
        json!([{"kind": "prompt", "value": "x".repeat(dialogs::MAX_PROMPT_BYTES + 1)}]),
        json!([{"kind": "other", "value": "ok"}]),
        json!([{"kind": "alert", "value": "ok", "extra": 1}]),
        Value::Array(many),
        Value::Array(big),
    ] {
        let source = "vc.run('shape.rectangle', {x: 20, y: 0, width: 5, height: 5});";
        let r = s.execute("script.run", &json!({"source": source, "answers": given}));
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "{r:?}");
        assert_eq!(art(&s), before);
    }
    s.script_frames = true;
    let r = s.execute("script.run", &json!({"source": "1", "answers": [{"kind": "alert", "value": "ok"}]}));
    assert!(matches!(r, Err(EngineError::BadParams { .. })), "a run in the app takes no answers: {r:?}");
    assert!(!s.script_running());
}

#[test]
fn a_framed_run_waits_at_each_dialog_and_takes_the_persons_answer() {
    let mut s = one_doc();
    framed(
        &mut s,
        "vc.run('shape.rectangle', {x: 20, y: 0, width: 5, height: 5}); var a = confirm('Go?', true, 'Ask'); var b = prompt('Name', 'n'); vc.log([a, b]);",
    );
    let want = ScriptDialog {
        kind: DialogKind::Confirm,
        message: "Go?".into(),
        title: Some("Ask".into()),
        error_icon: false,
        no_as_default: true,
        preset: String::new(),
    };
    assert_eq!(s.script_dialog(), Some(want));
    assert!(!s.script_answer(DialogAnswer::Ok), "an answer for another kind of dialog is refused");
    assert!(s.script_answer(DialogAnswer::Confirm(true)));
    assert!(!s.script_answer(DialogAnswer::Confirm(true)), "nothing waits until the run goes on");
    assert!(pump(&mut s).is_none());
    assert_eq!(s.script_dialog().unwrap().preset, "n");
    assert!(!s.script_answer(DialogAnswer::Prompt(Some("x".repeat(dialogs::MAX_PROMPT_BYTES + 1)))));
    assert!(s.script_answer(DialogAnswer::Prompt(Some("Ann".into()))));
    let r = pump(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!([[true, "Ann"]]));
    assert_eq!(s.documents()[0].history.undo.last().unwrap().label, "Run Script", "a finished run is one undo step");
    assert_eq!(r["dialogs"][1]["answer"], "Ann");
}

#[test]
fn prompt_cancel_returns_null() {
    let mut s = one_doc();
    framed(&mut s, "vc.log(prompt('Name', '') === null);");
    assert!(s.script_answer(DialogAnswer::Prompt(None)));
    assert_eq!(pump(&mut s).unwrap().unwrap()["log"], json!([true]));
}

#[test]
fn dialog_loop_stops_or_exhausts() {
    let mut s = one_doc();
    let before = art(&s);
    let source = "vc.run('shape.rectangle', {x: 20, y: 0, width: 5, height: 5}); while (1) { alert('x'); }";
    let three: Vec<Value> = (0..3).map(|_| json!({"kind": "alert", "value": "ok"})).collect();
    let e = headless(&mut s, source, Value::Array(three)).unwrap_err().to_string();
    assert!(e.contains("no answer left"), "{e}");
    assert_eq!(s.script_report().unwrap()["dialogCount"], 4);
    assert_eq!(art(&s), before);
    framed(&mut s, source);
    for _ in 0..3 {
        assert!(s.script_answer(DialogAnswer::Ok));
        assert!(pump(&mut s).is_none());
    }
    assert_eq!(s.execute("script.stop", &json!({})).unwrap()["stopped"], true);
    assert_eq!(s.script_dialog(), None, "a run asked to stop no longer waits");
    assert!(!s.script_answer(DialogAnswer::Ok));
    assert!(s.script_pump().unwrap().is_err());
    let report = s.script_report().unwrap();
    assert_eq!((report["kind"].as_str(), report["dialogCount"].as_u64()), (Some("stopped"), Some(4)), "{report}");
    assert_eq!(report["dialogs"][3]["answer"], "stop");
    assert_eq!(art(&s), before, "Stop at the first or any later dialog leaves no undo step");
    framed(&mut s, source);
    assert!(s.script_abandon(), "quitting while a dialog waits abandons the run");
    assert_eq!(s.script_report().unwrap()["dialogs"][0]["answer"], "stop");
    assert_eq!(art(&s), before);
}

#[test]
fn a_run_waiting_at_a_dialog_stops_at_the_next_slice_once_its_work_is_canceled() {
    let mut s = one_doc();
    let before = art(&s);
    let token = crate::cancel::CancelToken::new();
    s.script_frames = true;
    let source = "vc.run('shape.rectangle', {x: 20, y: 0, width: 5, height: 5}); alert('x'); vc.log('answered');";
    assert_eq!(s.execute("script.run", &json!({"source": source, "name": "d.js"})).unwrap()["running"], true);
    assert!(crate::cancel::scope(&token, None, || s.script_pump()).is_none());
    assert!(s.script_dialog().is_some());
    token.cancel();
    let ended = crate::cancel::scope(&token, None, || s.script_pump());
    assert!(ended.expect("the run goes on waiting for an answer").is_err());
    let report = s.script_report().unwrap();
    assert_eq!((report["kind"].as_str(), &report["log"]), (Some("stopped"), &json!([])), "{report}");
    assert_eq!(report["dialogs"][0]["answer"], "stop");
    assert_eq!(s.script_dialog(), None);
    assert_eq!(art(&s), before);
}

#[test]
fn dialog_text_limits_and_controls() {
    let mut s = one_doc();
    let source = format!(
        "var out = [];\
         function t(f) {{ try {{ f(); out.push('opened'); }} catch (e) {{ out.push(e.name); }} }}\
         t(function () {{ alert('x'); }});\
         t(function () {{ alert(new Array({m}).join('m')); }});\
         t(function () {{ alert('m', new Array({t}).join('t')); }});\
         t(function () {{ prompt('m', new Array({m}).join('p')); }});\
         t(function () {{ alert(); }});\
         t(function () {{ prompt('m'); }});\
         t(function () {{ alert({{toString: function () {{ throw new Error('no'); }}}}); }});\
         t(function () {{ alert(undefined); }});\
         vc.log(out.join());",
        m = dialogs::MAX_MESSAGE_BYTES + 2,
        t = dialogs::MAX_TITLE_BYTES + 2,
    );
    let r = headless(&mut s, &source, answers(&[json!({"kind": "alert", "value": "ok"}), json!({"kind": "alert", "value": "ok"})])).unwrap();
    assert_eq!(r["log"], json!(["opened,RangeError,RangeError,RangeError,TypeError,TypeError,Error,opened"]), "{r}");
    assert_eq!(r["dialogs"][1]["message"], "undefined", "an explicit undefined message is a message");
    assert_eq!(dialogs::shown_text("a\r\nb\rc\nd\te\u{7}\u{7f}", true), "a\nb\nc\nd\u{2409}e\u{2407}\u{2421}");
    assert_eq!(dialogs::shown_text("a\nb", false), "a\u{240a}b", "a title stays on one line");
    assert_eq!(dialogs::cut_to("aé", 2), "a", "a cut keeps whole characters");
}

#[test]
fn dialog_transcript_on_success_and_failure() {
    let mut s = one_doc();
    let r = headless(&mut s, "alert('one'); throw new Error('later');", answers(&[json!({"kind": "alert", "value": "ok"})])).unwrap_err();
    assert!(r.to_string().contains("later"));
    let report = s.script_report().unwrap();
    assert_eq!(report["dialogs"], json!([{"kind": "alert", "message": "one", "title": "Script Alert", "errorIcon": false, "answer": "ok"}]));
    let ok = headless(&mut s, "1", json!([])).unwrap();
    assert_eq!((ok["dialogs"].clone(), ok["dialogCount"].clone()), (json!([]), json!(0)));
}

#[test]
fn dialog_wait_does_not_spend_fuel() {
    let mut s = one_doc();
    framed(&mut s, "alert('x'); 1");
    for _ in 0..20 {
        assert!(s.script_pump().is_none(), "the run waits");
    }
    assert!(s.script_answer(DialogAnswer::Ok));
    let r = pump(&mut s).unwrap().unwrap();
    let mut t = one_doc();
    let plain = headless(&mut t, "alert('x'); 1", answers(&[json!({"kind": "alert", "value": "ok"})])).unwrap();
    assert_eq!((r["fuelUsed"].clone(), r["hostCalls"].clone()), (plain["fuelUsed"].clone(), plain["hostCalls"].clone()), "{r}");
}

#[test]
fn second_run_is_busy_while_dialog_waits() {
    let mut s = one_doc();
    framed(&mut s, "alert('x');");
    assert!(s.execute("script.run", &json!({"source": "1"})).is_err());
    assert!(s.script_dialog().is_some(), "the first run still waits");
    assert!(s.execute("shape.rectangle", &json!({"x": 1, "y": 1, "width": 1, "height": 1})).is_err(), "edits are refused while it waits");
}

#[test]
fn a_dialog_in_a_callback_throws_in_the_app() {
    let mut s = one_doc();
    s.script_frames = true;
    let source = "var m = ''; [1].forEach(function () { try { alert('x'); } catch (e) { m = e.message; } }); vc.log(m);";
    s.execute("script.run", &json!({"source": source})).unwrap();
    let r = pump(&mut s).unwrap().unwrap();
    assert!(r["log"][0].as_str().unwrap().contains("callback"), "{r}");
    assert_eq!(r["dialogCount"], 0);
}
