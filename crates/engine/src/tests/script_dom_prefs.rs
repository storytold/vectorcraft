//! The script object model: `app.preferences`, the matrix functions of `app` and `app.copy`,
//! `app.cut` and `app.paste`.

use serde_json::{Value, json};

use crate::script::dom::preferences::{MAX_KEYS, MAX_STRING_BYTES, MAX_TOTAL_BYTES, WARNING_OFF_NEEDS_A_PERSON, WARNING_OFF_QUESTION};
use crate::script::{DialogAnswer, DialogKind, ScriptDialog};
use crate::*;

fn run(s: &mut Session, source: &str) -> Result<Value> {
    s.execute("script.run", &json!({"source": source, "name": "t.js"}))
}

/// What the run logged.
fn log(s: &mut Session, source: &str) -> Vec<Value> {
    let r = run(s, source).unwrap_or_else(|e| panic!("{source}: {e}"));
    r["log"].as_array().cloned().unwrap_or_default()
}

/// The message of the run's failure.
fn failure(s: &mut Session, source: &str) -> String {
    run(s, source).map(|r| panic!("{source} ran: {r}")).unwrap_err().to_string()
}

fn stored(s: &Session, key: &str) -> Option<ScriptPreference> {
    s.prefs.script_preferences.get(key).cloned()
}

#[test]
fn show_external_jsx_warning_is_the_external_script_warning() {
    let mut s = Session::new();
    assert!(s.prefs.show_external_script_warning);
    let journal = s.journal.len();
    let out = log(
        &mut s,
        "var p = app.preferences; vc.log(p.typename); vc.log(p.getBooleanPreference('ShowExternalJSXWarning'));\
         vc.log(p.getIntegerPreference('ShowExternalJSXWarning')); p.setBooleanPreference('ShowExternalJSXWarning', true);",
    );
    assert_eq!(out, vec![json!("Preferences"), json!(true), json!(1)]);
    // A preference is not a command, so the journal has no entry for the run.
    assert_eq!(s.journal.len(), journal);
    assert!(s.prefs.script_preferences.is_empty());
    assert!(failure(&mut s, "app.preferences.setIntegerPreference('ShowExternalJSXWarning', 1);").contains("setBooleanPreference"));
    // Turned off by the person, a script reads it as off and can turn it back on, or remove it.
    s.prefs.show_external_script_warning = false;
    let out = log(
        &mut s,
        "var p = app.preferences; p.setBooleanPreference('ShowExternalJSXWarning', false);\
         vc.log(p.getBooleanPreference('ShowExternalJSXWarning')); p.setBooleanPreference('ShowExternalJSXWarning', true);",
    );
    assert_eq!(out, vec![json!(false)]);
    assert!(s.prefs.show_external_script_warning);
    s.prefs.show_external_script_warning = false;
    log(&mut s, "app.preferences.removePreference('ShowExternalJSXWarning');");
    assert!(s.prefs.show_external_script_warning);
}

/// A run with nobody to ask cannot turn the warning off, even with a Yes in `answers`.
#[test]
fn a_run_without_a_person_cannot_turn_the_warning_off() {
    let mut s = Session::new();
    let off = "app.preferences.setBooleanPreference('ShowExternalJSXWarning', false);";
    assert!(failure(&mut s, off).contains(WARNING_OFF_NEEDS_A_PERSON));
    assert!(s.prefs.show_external_script_warning);
    let r = s.execute("script.run", &json!({"source": off, "name": "t.js", "answers": [{"kind": "confirm", "value": true}]}));
    assert!(r.as_ref().is_err_and(|e| e.to_string().contains(WARNING_OFF_NEEDS_A_PERSON)), "{r:?}");
    assert!(s.prefs.show_external_script_warning);
    let caught = log(&mut s, &format!("try {{ {off} }} catch (e) {{ vc.log(e.message); }}"));
    assert_eq!(caught, vec![json!(WARNING_OFF_NEEDS_A_PERSON)]);
    assert!(s.prefs.show_external_script_warning);
}

/// A run in the app, one slice in.
fn framed(s: &mut Session, source: &str) {
    s.script_frames = true;
    assert_eq!(run(s, source).unwrap()["running"], true);
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

const ASK_OFF: &str = "var p = app.preferences; var r = p.setBooleanPreference('ShowExternalJSXWarning', false);\
                       vc.log([r === undefined, p.getBooleanPreference('ShowExternalJSXWarning')]);";

#[test]
fn yes_to_the_confirm_turns_the_warning_off() {
    let mut s = Session::new();
    framed(&mut s, ASK_OFF);
    let want = ScriptDialog {
        kind: DialogKind::Confirm,
        message: WARNING_OFF_QUESTION.into(),
        title: None,
        error_icon: false,
        no_as_default: true,
        preset: String::new(),
    };
    assert_eq!(s.script_dialog(), Some(want));
    assert!(s.prefs.show_external_script_warning, "nothing changes while the confirm waits");
    assert!(s.script_answer(DialogAnswer::Confirm(true)));
    let r = pump(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!([[true, false]]));
    assert_eq!(r["dialogs"][0]["answer"], true);
    assert!(!s.prefs.show_external_script_warning);
}

#[test]
fn turning_the_warning_on_after_yes_keeps_it_on() {
    let mut s = Session::new();
    framed(
        &mut s,
        &format!("{ASK_OFF} p.setBooleanPreference('ShowExternalJSXWarning', true); vc.log(p.getBooleanPreference('ShowExternalJSXWarning'));"),
    );
    assert!(s.script_answer(DialogAnswer::Confirm(true)));
    let r = pump(&mut s).unwrap().unwrap();
    assert_eq!(r["log"], json!([[true, false], true]), "the script sees the answer at once");
    assert!(s.prefs.show_external_script_warning, "the script's last write is what the run keeps");
}

#[test]
fn no_to_the_confirm_or_stop_leaves_the_warning_on() {
    let mut s = Session::new();
    framed(&mut s, ASK_OFF);
    assert!(s.script_answer(DialogAnswer::Confirm(false)));
    assert_eq!(pump(&mut s).unwrap().unwrap()["log"], json!([[true, true]]));
    assert!(s.prefs.show_external_script_warning);
    framed(&mut s, ASK_OFF);
    assert_eq!(s.execute("script.stop", &json!({})).unwrap()["stopped"], true);
    assert!(!s.script_answer(DialogAnswer::Confirm(true)));
    assert!(s.script_pump().unwrap().is_err());
    assert!(s.prefs.show_external_script_warning);
    // Yes in a run that then fails is put back with the rest of the run.
    framed(&mut s, &format!("{ASK_OFF} throw new Error('stop');"));
    assert!(s.script_answer(DialogAnswer::Confirm(true)));
    assert!(pump(&mut s).unwrap().is_err());
    assert!(s.prefs.show_external_script_warning);
}

/// Asking to show no dialogs does not skip the confirm, and a call from a callback, where no dialog
/// can open, throws.
#[test]
fn the_confirm_cannot_be_skipped() {
    let mut s = Session::new();
    framed(&mut s, &format!("try {{ app.userInteractionLevel = 'DONTDISPLAYALERTS'; }} catch (e) {{}} {ASK_OFF}"));
    assert_eq!(s.script_dialog().map(|d| d.message), Some(WARNING_OFF_QUESTION.to_string()));
    assert!(s.script_answer(DialogAnswer::Confirm(false)));
    assert!(pump(&mut s).unwrap().is_ok());
    assert!(s.prefs.show_external_script_warning);
    let callback = "[1].forEach(function () { app.preferences.setBooleanPreference('ShowExternalJSXWarning', false); });";
    assert_eq!(run(&mut s, callback).unwrap()["running"], true);
    let e = pump(&mut s).unwrap().unwrap_err().to_string();
    assert!(e.contains(WARNING_OFF_NEEDS_A_PERSON), "{e}");
    assert!(s.prefs.show_external_script_warning);
}

#[test]
fn other_keys_keep_their_type_across_runs_and_a_restart() {
    let mut s = Session::new();
    log(
        &mut s,
        "var p = app.preferences; p.setBooleanPreference('my/flag', true); p.setIntegerPreference('count', -7);\
         p.setRealPreference('ratio', 0.25); p.setStringPreference('name', 'Ünïcode \\u2713');",
    );
    assert_eq!(stored(&s, "my/flag"), Some(ScriptPreference::Boolean(true)));
    assert_eq!(stored(&s, "count"), Some(ScriptPreference::Integer(-7)));
    assert_eq!(stored(&s, "ratio"), Some(ScriptPreference::Real(0.25)));
    assert_eq!(stored(&s, "name"), Some(ScriptPreference::String("Ünïcode \u{2713}".into())));
    // The preferences as the app saves them, read into a new session.
    let saved = serde_json::to_string(&s.prefs).unwrap();
    let mut t = Session::new();
    t.prefs = serde_json::from_str(&saved).unwrap();
    let out = log(
        &mut t,
        "var p = app.preferences; vc.log(p.getBooleanPreference('my/flag')); vc.log(p.getIntegerPreference('count'));\
         vc.log(p.getRealPreference('ratio')); vc.log(p.getStringPreference('name'));",
    );
    assert_eq!(out, vec![json!(true), json!(-7), json!(0.25), json!("Ünïcode \u{2713}")]);
}

#[test]
fn missing_keys_read_as_empty_values_and_getters_convert() {
    let mut s = Session::new();
    let mut out = log(
        &mut s,
        "var p = app.preferences; vc.log([p.getBooleanPreference('none'), p.getIntegerPreference('none'),\
         p.getRealPreference('none'), p.getStringPreference('none')]);\
         p.setRealPreference('r', -2.75); p.setStringPreference('t', ' 12.5 '); p.setStringPreference('x', 'abc');\
         p.setBooleanPreference('b', true); p.setIntegerPreference('i', 0);\
         vc.log([p.getIntegerPreference('r'), p.getStringPreference('r'), p.getBooleanPreference('r')]);\
         vc.log([p.getRealPreference('t'), p.getIntegerPreference('t'), p.getBooleanPreference('t')]);\
         vc.log([isNaN(p.getRealPreference('x')), p.getIntegerPreference('x')]);\
         vc.log([p.getStringPreference('b'), p.getRealPreference('b'), p.getBooleanPreference('i'), p.getStringPreference('i')]);\
         var n = []; ['0x10', '0B101', '0o17', '-0x10', '0x', ' -Infinity ', '1e3', '.5', 'inf', '1_0'].forEach(function (t) {\
         p.setStringPreference('n', t); n.push(String(p.getRealPreference('n'))); }); vc.log(n.join(' '));",
    );
    assert_eq!(out.pop(), Some(json!("16 5 15 NaN NaN -Infinity 1000 0.5 NaN NaN")));
    assert_eq!(
        out,
        vec![json!([false, 0, 0, ""]), json!([-2, "-2.75", true]), json!([12.5, 12, true]), json!([true, 0]), json!(["true", 1, false, "0"])]
    );
}

#[test]
fn bad_arguments_throw_and_store_nothing() {
    let mut s = Session::new();
    let mut wrong = Vec::new();
    for (source, message) in [
        ("app.preferences.setIntegerPreference('k', 1.5);", "whole number"),
        ("app.preferences.setIntegerPreference('k', 1e300);", "whole number"),
        ("app.preferences.setIntegerPreference('k', '3');", "whole number"),
        ("app.preferences.setRealPreference('k', Infinity);", "finite number"),
        ("app.preferences.setRealPreference('k', NaN);", "finite number"),
        ("app.preferences.setBooleanPreference('k', 1);", "true or false"),
        ("app.preferences.setStringPreference('k', 5);", "a string"),
        ("app.preferences.setStringPreference(5, 'v');", "key must be a string"),
        ("app.preferences.getBooleanPreference();", "key must be a string"),
        ("app.preferences.setStringPreference('', 'v');", "1 to 256"),
        ("app.preferences.setStringPreference(new Array(258).join('k'), 'v');", "1 to 256"),
        // 129 characters outside the Basic Multilingual Plane are 258 UTF-16 code units.
        ("app.preferences.setStringPreference(new Array(130).join('\\uD83D\\uDE00'), 'v');", "1 to 256"),
        ("app.preferences.setStringPreference('a\\u0000b', 'v');", "control characters"),
        ("app.preferences.setStringPreference('k', new Array(65538).join('v'));", "at most 65536 bytes"),
        ("var f = app.preferences.getBooleanPreference; f.call({}, 'k');", "not a Preferences"),
        ("Preferences.prototype.setBooleanPreference('k', true);", "not a Preferences"),
    ] {
        match run(&mut s, source) {
            Err(e) if e.to_string().contains(message) => {}
            r => wrong.push(format!("{source}: {r:?}")),
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    assert!(s.prefs.script_preferences.is_empty());
    // A key of exactly 256 code units and a value of exactly the limit are taken.
    log(&mut s, "app.preferences.setStringPreference(new Array(257).join('k'), new Array(65537).join('v'));");
    assert_eq!(stored(&s, &"k".repeat(256)), Some(ScriptPreference::String("v".repeat(MAX_STRING_BYTES))));
    log(&mut s, "app.preferences.setStringPreference(new Array(129).join('\\uD83D\\uDE00'), 'v');");
    assert_eq!(stored(&s, &"\u{1F600}".repeat(128)), Some(ScriptPreference::String("v".into())));
}

#[test]
fn the_store_limits_its_keys() {
    let mut s = Session::new();
    for i in 0..MAX_KEYS {
        s.prefs.script_preferences.insert(format!("k{i}"), ScriptPreference::Integer(1));
    }
    assert!(failure(&mut s, "app.preferences.setIntegerPreference('one more', 1);").contains("at most 4096 keys"));
    assert_eq!(s.prefs.script_preferences.len(), MAX_KEYS);
    // A key already stored can still change.
    log(&mut s, "app.preferences.setIntegerPreference('k0', 2);");
    assert_eq!(stored(&s, "k0"), Some(ScriptPreference::Integer(2)));
}

#[test]
fn the_store_limits_its_bytes() {
    let mut s = Session::new();
    let big = "v".repeat(MAX_STRING_BYTES);
    for i in 0..MAX_TOTAL_BYTES / MAX_STRING_BYTES - 1 {
        s.prefs.script_preferences.insert(format!("{i:02}"), ScriptPreference::String(big.clone()));
    }
    let e = failure(&mut s, "app.preferences.setStringPreference('last', new Array(65537).join('v'));");
    assert!(e.contains("at most 1048576 bytes"), "{e}");
    assert_eq!(stored(&s, "last"), None);
    // Replacing a stored value counts the new value instead of the old one.
    log(&mut s, "app.preferences.setStringPreference('00', new Array(65537).join('w'));");
    assert_eq!(stored(&s, "00"), Some(ScriptPreference::String("w".repeat(MAX_STRING_BYTES))));
}

#[test]
fn a_failed_run_puts_the_store_back_and_removing_the_last_key_empties_it() {
    let mut s = Session::new();
    log(&mut s, "app.preferences.setIntegerPreference('keep', 1);");
    let e = failure(&mut s, "app.preferences.setIntegerPreference('keep', 2); app.preferences.setRealPreference('new', 1); throw new Error('stop');");
    assert!(e.contains("stop"));
    assert_eq!(stored(&s, "keep"), Some(ScriptPreference::Integer(1)));
    assert_eq!(stored(&s, "new"), None);
    let out = log(
        &mut s,
        "var p = app.preferences; p.removePreference('missing'); p.removePreference('keep');\
         vc.log(p.getIntegerPreference('keep'));",
    );
    assert_eq!(out, vec![json!(0)]);
    assert!(s.prefs.script_preferences.is_empty());
}

#[test]
fn a_run_merges_its_keys_with_keys_the_app_changed_meanwhile() {
    let mut s = Session::new();
    s.prefs.script_preferences.insert("gone".into(), ScriptPreference::Integer(1));
    s.prefs.script_preferences.insert("kept".into(), ScriptPreference::Integer(1));
    s.script_frames = true;
    let source = "var p = app.preferences; p.removePreference('gone'); p.setIntegerPreference('mine', 2);\
                  for (var i = 0; i < 2000000; i++) {}";
    assert_eq!(run(&mut s, source).unwrap()["running"], true);
    assert!(s.script_pump().is_none());
    assert!(s.script_running());
    s.prefs.script_preferences.insert("theirs".into(), ScriptPreference::Boolean(true));
    s.prefs.script_preferences.insert("kept".into(), ScriptPreference::Integer(3));
    while s.script_pump().is_none() {}
    let keys: Vec<(&str, &ScriptPreference)> = s.prefs.script_preferences.iter().map(|(k, v)| (k.as_str(), v)).collect();
    assert_eq!(
        keys,
        vec![("kept", &ScriptPreference::Integer(3)), ("mine", &ScriptPreference::Integer(2)), ("theirs", &ScriptPreference::Boolean(true))]
    );
}

#[test]
fn matrix_functions_give_new_records() {
    let mut s = Session::new();
    let out = log(
        &mut s,
        "function v(m) { return [m.typename, m.mValueA, m.mValueB, m.mValueC, m.mValueD, m.mValueTX, m.mValueTY]; }\
         vc.log(v(app.getIdentityMatrix())); vc.log(v(app.getTranslationMatrix(100, 200))); vc.log(v(app.getTranslationMatrix()));\
         vc.log(v(app.getScaleMatrix(60))); vc.log(v(app.getScaleMatrix(50, 200))); vc.log(v(app.getScaleMatrix()));\
         vc.log(v(app.getRotationMatrix(90))); vc.log(v(app.getRotationMatrix(-270))); vc.log(v(app.getRotationMatrix()));",
    );
    assert_eq!(
        out,
        vec![
            json!(["Matrix", 1, 0, 0, 1, 0, 0]),
            json!(["Matrix", 1, 0, 0, 1, 100, 200]),
            json!(["Matrix", 1, 0, 0, 1, 0, 0]),
            json!(["Matrix", 0.6, 0, 0, 0.6, 0, 0]),
            json!(["Matrix", 0.5, 0, 0, 2, 0, 0]),
            json!(["Matrix", 1, 0, 0, 1, 0, 0]),
            json!(["Matrix", 0, 1, -1, 0, 0, 0]),
            json!(["Matrix", 0, 1, -1, 0, 0, 0]),
            json!(["Matrix", 1, 0, 0, 1, 0, 0]),
        ]
    );
    let r = log(&mut s, "var m = app.getRotationMatrix(30); vc.log([m.mValueA, m.mValueB, m.mValueC, m.mValueD]);");
    let (c, sn) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
    assert_eq!(r, vec![json!([c, sn, -sn, c])]);
}

#[test]
fn concatenation_applies_the_first_matrix_first() {
    let mut s = Session::new();
    // As PDF `cm` operators compose (ISO 32000-1, 8.3.4): [x y 1] × M × N.
    // M = [2 0 0 3 5 7] then N = [0 1 -1 0 10 20]: (x, y) → (2x + 5, 3y + 7) → (-(3y + 7) + 10, 2x + 5 + 20).
    let out = log(
        &mut s,
        "function v(m) { return [m.mValueA, m.mValueB, m.mValueC, m.mValueD, m.mValueTX, m.mValueTY]; }\
         var m = {mValueA: 2, mValueB: 0, mValueC: 0, mValueD: 3, mValueTX: 5, mValueTY: 7};\
         var n = {mValueA: 0, mValueB: 1, mValueC: -1, mValueD: 0, mValueTX: 10, mValueTY: 20};\
         var c = app.concatenateMatrix(m, n); vc.log(v(c)); vc.log(v(m)); vc.log(c === m);\
         vc.log(v(app.concatenateTranslationMatrix(m, 1, 2))); vc.log(v(app.concatenateScaleMatrix(m, 50)));\
         vc.log(v(app.concatenateRotationMatrix(m, 90)));\
         vc.log(app.isEqualMatrix(app.concatenateRotationMatrix(m, 90), app.concatenateMatrix(m, app.getRotationMatrix(90))));",
    );
    assert_eq!(
        out,
        vec![
            json!([0, 2, -3, 0, 3, 25]),
            json!([2, 0, 0, 3, 5, 7]),
            json!(false),
            json!([2, 0, 0, 3, 6, 9]),
            json!([1, 0, 0, 1.5, 2.5, 3.5]),
            json!([0, 2, -3, 0, -7, 5]),
            json!(true),
        ]
    );
}

#[test]
fn concatenation_keeps_a_finite_result_whose_products_leave_the_range_of_a_number() {
    let mut s = Session::new();
    let out = log(
        &mut s,
        "function v(m) { return [m.mValueA, m.mValueB, m.mValueC, m.mValueD, m.mValueTX, m.mValueTY]; }\
         function mat(v) { return {mValueA: v[0], mValueB: v[1], mValueC: v[2], mValueD: v[3], mValueTX: v[4], mValueTY: v[5]}; }\
         var x = Math.pow(2, 600), y = Math.pow(2, -500), z = Math.pow(2, -575);\
         var c = app.concatenateMatrix(mat([x, x, 0, 1, x, x]), mat([x, 0, -x, 1, 0, 0]));\
         vc.log([c.mValueA, c.mValueB === x, c.mValueC === -x, c.mValueD, c.mValueTX, c.mValueTY === x]);\
         vc.log(app.concatenateMatrix(mat([y, y, 0, 1, 0, 0]), mat([z, 0, z, 1, 0, 0])).mValueA === Number.MIN_VALUE);\
         var p = 2.0543170765449965e-57, q = 6.201533916984909e-252;\
         vc.log(app.concatenateMatrix(mat([p, 0, 0, 1, 0, 0]), mat([q, 0, 0, 1, 0, 0])).mValueA === p * q);\
         vc.log(v(app.concatenateMatrix(mat([-2, 0, 0, -3, 0, 0]), mat([0, 1, -1, 0, 0, 0]))));\
         var big = mat([1, 0, 0, 1, Number.MAX_VALUE, 0]), up = 1 + Math.pow(2, -52);\
         try { app.concatenateMatrix(big, mat([up, 0, 0, 1, -Math.pow(2, 971), 0])); vc.log('finite'); }\
         catch (e) { vc.log(e.name); }\
         vc.log(app.concatenateMatrix(big, mat([up, 0, 0, 1, -Math.pow(2, 972), 0])).mValueTX === Number.MAX_VALUE);\
         var h = Math.pow(2, 500), far = mat([1, 0, 0, 1, Math.pow(2, 600), -Math.pow(2, 600)]);\
         vc.log(app.concatenateMatrix(far, mat([h, 0, h, 1, 1, 0])).mValueTX);\
         var g = Math.pow(2, 100), farther = mat([1, 0, 0, 1, Math.pow(2, 1000), -Math.pow(2, 1000)]);\
         vc.log(app.concatenateMatrix(farther, mat([g, 0, g, 1, Math.pow(2, -100), 0])).mValueTX === Math.pow(2, -100));\
         var tiny = mat([1, 0, 0, 1, y, 0]);\
         vc.log(app.concatenateMatrix(tiny, mat([z, 0, 0, 1, Number.MIN_VALUE, 0])).mValueTX === 2 * Number.MIN_VALUE);\
         var least = mat([1, 0, 0, 1, Number.MIN_VALUE, 0]);\
         vc.log(app.concatenateMatrix(least, mat([0.5 - Math.pow(2, -54), 0, 0, 1, Number.MIN_VALUE, 0])).mValueTX === Number.MIN_VALUE);\
         vc.log(app.concatenateMatrix(least, mat([0.5 + Math.pow(2, -53), 0, 0, 1, Number.MIN_VALUE, 0])).mValueTX === 2 * Number.MIN_VALUE);",
    );
    assert_eq!(
        out,
        vec![
            json!([0, true, true, 1, 0, true]),
            json!(true),
            json!(true),
            json!([0, -2, 3, 0, 0, 0]),
            json!("RangeError"),
            json!(true),
            json!(1),
            json!(true),
            json!(true),
            json!(true),
            json!(true)
        ]
    );
}

#[test]
fn a_turn_just_short_of_a_negative_quarter_turn_is_not_rounded_to_it() {
    let mut s = Session::new();
    let out = log(
        &mut s,
        "function v(m) { return [m.mValueA, m.mValueB, m.mValueC, m.mValueD]; }\
         var m = app.getRotationMatrix(-89.99999999999999); vc.log(m.mValueA > 0); vc.log(m.mValueB);\
         vc.log(v(app.getRotationMatrix(-90))); vc.log(v(app.getRotationMatrix(-180))); vc.log(v(app.getRotationMatrix(-450)));",
    );
    assert_eq!(out, vec![json!(true), json!(-1), json!([0, -1, 1, 0]), json!([-1, 0, 0, -1]), json!([0, -1, 1, 0])]);
}

#[test]
fn inverse_singular_and_equal_matrices() {
    let mut s = Session::new();
    let out = log(
        &mut s,
        "var m = app.concatenateTranslationMatrix(app.concatenateRotationMatrix(app.getScaleMatrix(40, 250), 33), 17, -4);\
         var i = app.invertMatrix(m), back = app.concatenateMatrix(m, i), id = app.getIdentityMatrix();\
         var d = [back.mValueA - 1, back.mValueB, back.mValueC, back.mValueD - 1, back.mValueTX, back.mValueTY];\
         vc.log(Math.max.apply(null, d.map(Math.abs)) < 1e-12);\
         vc.log(app.isSingularMatrix(app.getScaleMatrix(0, 100))); vc.log(app.isSingularMatrix(m));\
         vc.log(app.isEqualMatrix(id, app.getRotationMatrix(360))); vc.log(app.isEqualMatrix(id, app.getTranslationMatrix(0, 1e-300)));\
         var tiny = app.getScaleMatrix(1e-198, 1e202), t = app.invertMatrix(tiny);\
         function near(v, w) { return Math.abs(v / w - 1) < 1e-12; }\
         vc.log([app.isSingularMatrix(tiny), near(t.mValueA, 1e200), near(t.mValueD, 1e-200), t.mValueB, t.mValueC]);\
         vc.log(near(app.invertMatrix(app.getScaleMatrix(1e202)).mValueA, 1e-200));\
         function mat(v) { return {mValueA: v[0], mValueB: v[1], mValueC: v[2], mValueD: v[3], mValueTX: 0, mValueTY: 0}; }\
         var wide = mat([1e308, 0, 1e308, 1e-100]), w = app.invertMatrix(wide);\
         vc.log([app.isSingularMatrix(wide), near(w.mValueA, 1e-308), near(w.mValueC, -1e100), near(w.mValueD, 1e100)]);\
         var thin = app.invertMatrix(mat([1e300, 0, 1e300, 5e-24]));\
         vc.log([near(thin.mValueA, 1e-300), near(thin.mValueC, -2e23), near(thin.mValueD, 2e23)]);\
         var close = mat([7207, 7440, 382, 394.34993756070486]), ci = app.invertMatrix(close);\
         vc.log([app.isSingularMatrix(close), app.isSingularMatrix(mat([6, 4, 3, 2])), near(ci.mValueD, 7207 / -8.009237717487849e-11)]);\
         var far = mat([1e-100, 1e-100, 1e-100, 1e-100 + 1e-115]); far.mValueTX = 1e200; far.mValueTY = 1e200;\
         var fi = app.invertMatrix(far); vc.log([Math.abs(fi.mValueTX / -1e300 - 1) < 1e-6, fi.mValueTY]);",
    );
    assert_eq!(out[..5], [json!(true), json!(true), json!(false), json!(true), json!(false)]);
    assert_eq!(out[5], json!([false, true, true, 0, 0]));
    assert_eq!(out[6], json!(true));
    assert_eq!(out[7], json!([false, true, true, true]));
    assert_eq!(out[8], json!([true, true, true]));
    assert_eq!(out[9], json!([false, true, true]));
    assert_eq!(out[10], json!([true, 0]));
    let mut wrong = Vec::new();
    for (source, message) in [
        ("app.invertMatrix(app.getScaleMatrix(100, 0));", "singular"),
        ("app.getTranslationMatrix(NaN);", "not a finite number"),
        ("app.getScaleMatrix('50');", "scaleX must be a finite number"),
        ("app.getRotationMatrix(Infinity);", "not a finite number"),
        ("app.getRotationMatrix(null);", "angle must be a finite number"),
        ("app.concatenateMatrix(app.getIdentityMatrix(), {mValueA: 1});", "secondMatrix must be a Matrix"),
        (
            "app.concatenateMatrix(app.getIdentityMatrix(), {mValueA: 1, mValueB: 0, mValueC: 0, mValueD: 1, mValueTX: '0', mValueTY: 0});",
            "secondMatrix must be a Matrix",
        ),
        ("app.invertMatrix(5);", "matrix must be a Matrix"),
        ("var m = app.getIdentityMatrix(); m.mValueTX = Infinity; app.isSingularMatrix(m);", "not a finite number"),
        ("app.concatenateScaleMatrix(app.getScaleMatrix(1e300), 1e300);", "too large"),
    ] {
        match run(&mut s, source) {
            Err(e) if e.to_string().contains(message) => {}
            r => wrong.push(format!("{source}: {r:?}")),
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn copy_and_paste_run_as_one_undo_step() {
    let mut s = Session::new();
    assert!(failure(&mut s, "app.paste();").contains("No active document"));
    log(&mut s, "app.copy(); app.cut();");
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let a = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let steps = s.active().unwrap().history.undo.len();
    // Nothing selected and the clipboard empty: nothing happens and no step is added.
    log(&mut s, "app.copy(); app.cut(); app.paste();");
    assert_eq!(s.active().unwrap().history.undo.len(), steps);
    assert!(s.clipboard.is_empty());
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let steps = s.active().unwrap().history.undo.len();
    let out = log(&mut s, "app.copy(); app.paste(); app.paste(); vc.log(app.activeDocument.pageItems.length);");
    assert_eq!(out, vec![json!(3)]);
    assert_eq!(s.active().unwrap().history.undo.len(), steps + 1);
    // A cut in a run that fails is put back.
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let steps = s.active().unwrap().history.undo.len();
    assert!(failure(&mut s, "app.cut(); throw new Error('stop');").contains("stop"));
    assert!(s.active().unwrap().doc.node(NodeId(a)).is_some());
    assert_eq!(s.active().unwrap().history.undo.len(), steps);
    log(&mut s, "app.cut();");
    assert!(s.active().unwrap().doc.node(NodeId(a)).is_none());
}

#[test]
fn only_copies_that_fill_the_clipboard_and_are_kept_are_counted() {
    let mut s = Session::new();
    s.clipboard_copied = true;
    s.note_copy();
    assert_eq!(s.copies, 0);
    assert!(!s.clipboard_copied);
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let a = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    assert_eq!(s.copies, 1);
    assert!(s.clipboard_copied);
    // A batch that copies and then fails puts the count back with the clipboard.
    s.execute("clipboard.importSvg", &json!({"svg": "<svg xmlns=\"http://www.w3.org/2000/svg\"><rect width=\"5\" height=\"5\"/></svg>"})).unwrap();
    assert!(!s.clipboard_copied);
    let batch = json!({"commands": [{"command": "edit.copy", "params": {}}, {"command": "no.such", "params": {}}]});
    assert!(s.execute("command.batch", &batch).is_err());
    assert_eq!(s.copies, 1);
    assert!(!s.clipboard_copied);
}

#[test]
fn a_copy_or_cut_the_run_puts_back_is_not_reported() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let a = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    // The clipboard holds a copy from before the run.
    s.execute("edit.copy", &json!({})).unwrap();
    let copies = s.copies;
    // No command result reaches the script, so each command below is put back.
    crate::script::TEST_RESULT_BYTES.with(|b| b.set(Some(1)));
    let r = run(
        &mut s,
        "try { vc.run('command.batch', {commands: [{command: 'edit.copy', params: {}}]}); } catch (e) { vc.log(e.name); }\
         try { vc.run('edit.cut'); } catch (e) { vc.log(e.name); }\
         try { vc.run('edit.copy'); } catch (e) { vc.log(e.name); }\
         try { vc.run('artboard.copy', {indices: [0]}); } catch (e) { vc.log(e.name); }",
    );
    crate::script::TEST_RESULT_BYTES.with(|b| b.set(None));
    let r = r.unwrap();
    assert_eq!(r["log"], json!(["RangeError", "RangeError", "RangeError", "RangeError"]), "{r}");
    assert_eq!(r.get("copied"), None, "{r}");
    assert_eq!(s.copies, copies);
    assert!(s.clipboard_copied);
    assert!(s.active().unwrap().doc.node(NodeId(a)).is_some());
}
