//! The `$` object's members besides `$.write` and `$.writeln`, the global `localize` and
//! `toSource`, and what of them one run keeps to itself.

use std::time::Duration;

use serde_json::{Value, json};

use crate::cancel::{self, CancelToken};
use crate::script::{TEST_ELAPSED, TEST_NO_CLOCK};
use crate::*;

/// The values `source` gives `vc.log`, in a run of its text.
fn logged(s: &mut Session, source: &str) -> Value {
    s.execute("script.run", &json!({"source": source, "name": "t.js"})).unwrap_or_else(|e| panic!("{source}: {e}"))["log"].clone()
}

/// The one value `expr` gives, logged.
fn value(s: &mut Session, expr: &str) -> Value {
    logged(s, &format!("vc.log({expr});"))[0].clone()
}

/// The name of the error `statement` throws and its message, as `Name: message`.
fn thrown(s: &mut Session, statement: &str) -> String {
    value(s, &format!("(function () {{ try {{ {statement}; return 'no error'; }} catch (e) {{ return e.name + ': ' + e.message; }} }})()"))
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn each_member_has_its_value() {
    let mut s = Session::new();
    let version = env!("CARGO_PKG_VERSION");
    let os = if cfg!(target_arch = "wasm32") {
        "Web"
    } else if cfg!(windows) {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else if cfg!(target_os = "freebsd") {
        "FreeBSD"
    } else {
        "Unix"
    };
    for (expr, want) in [
        ("$.about()", json!(format!("VectorCraft script engine {version}"))),
        ("$.appEncoding", json!("UTF-8")),
        ("$.build", json!(version)),
        ("$.buildDate", json!(null)),
        ("$.decimalPoint", json!(".")),
        ("$.engineName", json!("VectorCraft")),
        ("$.error", json!(null)),
        ("$.flags", json!(0)),
        ("$.gc() === undefined", json!(true)),
        ("$.global === this", json!(true)),
        ("typeof $.hiresTimer === 'number' && $.hiresTimer >= 0", json!(true)),
        ("$.level", json!(0)),
        ("\n$.line", json!(2)),
        ("$.locale", json!("en")),
        ("$.localize", json!(false)),
        ("$.memCache", json!(0)),
        ("$.os", json!(os)),
        ("$.screens.length", json!(0)),
        ("$.stack", json!("[global]@t.js:1\n")),
        ("$.strict", json!(false)),
        ("$.version", json!(version)),
        ("typeof $.getenv + typeof $.setenv + typeof $.sleep", json!("functionfunctionfunction")),
        ("vc.api.has('dollar')", json!(true)),
    ] {
        assert_eq!(value(&mut s, expr), want, "{expr}");
    }
    assert_eq!(thrown(&mut s, "$.bp()"), "Error: $.bp: scripts here have no debugger");
    assert_eq!(thrown(&mut s, "$.colorPicker(0)"), "Error: $.colorPicker: scripts cannot show a dialog yet");
}

#[test]
fn read_only_members_keep_their_values_and_settable_ones_check_what_they_are_given() {
    let mut s = Session::new();
    assert_eq!(value(&mut s, "($.build = 'x', $.level = 3, $.os = 'y', [$.build === 'x', $.level, $.os === 'y'])"), json!([false, 0, false]));
    assert!(thrown(&mut s, "(function () { 'use strict'; $.version = '9'; })()").starts_with("TypeError"));
    assert!(thrown(&mut s, "(function () { 'use strict'; $.error = null; })()").starts_with("TypeError"));
    assert_eq!(value(&mut s, "($.appEncoding = 'utf-8', $.appEncoding)"), json!("UTF-8"));
    assert!(thrown(&mut s, "$.appEncoding = 'ISO-8859-1'").starts_with("RangeError"));
    assert!(thrown(&mut s, "$.appEncoding = 8").starts_with("TypeError"));
    assert!(thrown(&mut s, "$.memCache = 1024").starts_with("Error"));
    assert_eq!(value(&mut s, "($.flags = 512, [$.flags, $.localize])"), json!([512, true]));
    assert_eq!(value(&mut s, "($.flags = 512, $.flags = 0, [$.flags, $.localize])"), json!([0, false]));
    for bad in ["$.flags = 2", "$.flags = 513", "$.flags = -512"] {
        assert!(thrown(&mut s, bad).starts_with("RangeError"), "{bad}");
    }
    assert!(thrown(&mut s, "$.flags = '512'").starts_with("TypeError"));
    assert!(thrown(&mut s, "$.localize = 1").starts_with("TypeError"));
}

#[test]
fn the_locale_starts_as_the_interfaces_and_a_change_lasts_one_run() {
    let mut s = Session::new();
    for (ui, want) in [("de", "de"), ("pt-br", "pt_BR"), ("zh-hant", "zh_TW"), ("zh-hans", "zh_CN"), ("auto", "en"), ("x", "en")] {
        s.ui_language = Some(ui.into());
        assert_eq!(value(&mut s, "$.locale"), json!(want), "{ui}");
    }
    s.ui_language = None;
    s.prefs.interface_language = "fr".into();
    assert_eq!(value(&mut s, "$.locale"), json!("fr"), "without a language from the app, the preference");
    s.ui_language = Some("de".into());
    let lookup = "localize({en: 'e', de: 'd', fr: 'f', fr_CA: 'c'})";
    assert_eq!(value(&mut s, &format!("[{lookup}, $.decimalPoint]")), json!(["d", ","]));
    let changed = format!(
        "$.locale = 'FR-ca'; var a = [$.locale, {lookup}]; $.locale = 'fr'; a.push({lookup}); $.locale = null; a.push($.locale, {lookup}); a"
    );
    assert_eq!(value(&mut s, &format!("(function () {{ {changed}; return a; }})()")), json!(["fr_CA", "c", "f", "de", "d"]));
    assert_eq!(value(&mut s, "($.locale = '', $.locale)"), json!("de"), "an empty locale is the interface's");
    assert_eq!(value(&mut s, "($.locale = 'en_US', $.locale)"), json!("en_US"));
    assert_eq!(value(&mut s, "$.locale"), json!("de"), "the next run starts with the interface's again");
    for bad in ["$.locale = 'german'", "$.locale = 'de_DE_x'", "$.locale = 'd'"] {
        assert!(thrown(&mut s, bad).starts_with("RangeError"), "{bad}");
    }
    assert!(thrown(&mut s, "$.locale = 5").starts_with("TypeError"));
}

#[test]
fn localized_to_string_is_on_only_while_the_run_turns_it_on() {
    let mut s = Session::new();
    s.ui_language = Some("de".into());
    let text = "String({en: 'Open', de: 'Öffnen'})";
    assert_eq!(value(&mut s, text), json!("[object Object]"));
    assert_eq!(value(&mut s, &format!("($.localize = true, {text})")), json!("Öffnen"));
    assert_eq!(value(&mut s, text), json!("[object Object]"), "the next run starts with it off");
    assert_eq!(value(&mut s, "localize('%1 of %2', 3, 7)"), json!("3 of 7"), "localize works with it off");
}

#[test]
fn to_source_writes_plain_data() {
    let mut s = Session::new();
    assert_eq!(value(&mut s, "toSource({a: [1, 'x'], b: null})"), json!(r#"({"a":[1, "x"], "b":null})"#));
    assert_eq!(value(&mut s, "[true, 2].toSource()"), json!("[true, 2]"));
    assert!(thrown(&mut s, "var o = {}; o.o = o; toSource(o)").starts_with("TypeError"));
    assert!(thrown(&mut s, "toSource(vc)").starts_with("TypeError"), "a host object is not plain data");
}

#[test]
fn getenv_reads_only_its_names_and_setenv_changes_only_the_run() {
    let mut s = Session::new();
    for name in ["HOME", "PATH", "USERPROFILE", "TEMP", "lang", ""] {
        assert_eq!(value(&mut s, &format!("$.getenv('{name}')")), json!(null), "{name}");
    }
    let process = std::env::var("OS").ok();
    let before = value(&mut s, "$.getenv('OS')");
    if let Some(os) = process.as_deref().filter(|v| v.len() <= 64 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-@".contains(&b))) {
        assert_eq!(before, json!(os));
    }
    assert_eq!(value(&mut s, "($.setenv('OS', 'Test-1'), $.getenv('OS'))"), json!("Test-1"));
    assert_eq!(std::env::var("OS").ok(), process, "the process's environment stays as it was");
    assert_eq!(value(&mut s, "$.getenv('OS')"), before, "the next run reads the process's value again");
    assert!(thrown(&mut s, "$.setenv('HOME', 'x')").starts_with("Error"));
    assert!(thrown(&mut s, "$.setenv('LANG', 'a b')").starts_with("RangeError"));
    assert!(thrown(&mut s, &format!("$.setenv('LANG', '{}')", "a".repeat(65))).starts_with("RangeError"));
    assert!(thrown(&mut s, "$.setenv('LANG')").starts_with("TypeError"));
    assert!(thrown(&mut s, "$.getenv(1)").starts_with("TypeError"));
}

#[test]
fn error_line_and_stack_follow_the_running_code() {
    let mut s = Session::new();
    let source = "try { null.x; } catch (e) {}\nfunction where() {\n  return [$.line, $.stack, $.error instanceof TypeError];\n}\nvc.log(where());";
    assert_eq!(logged(&mut s, source)[0], json!([3, "[global]@t.js:5\nwhere@t.js:3\n", true]));
    assert_eq!(value(&mut s, "$.error"), json!(null), "each run starts without one");
}

#[test]
fn sleep_waits_between_slices_until_its_time_has_passed() {
    let mut s = Session::new();
    s.script_frames = true;
    TEST_ELAPSED.set(Some(Duration::ZERO));
    s.execute("script.run", &json!({"source": "vc.log(1); $.sleep(250); vc.log(2);", "name": "t.js"})).unwrap();
    let logs =
        |s: &mut Session| s.execute("script.console", &json!({})).unwrap()["lines"].as_array().unwrap().iter().filter(|l| l["kind"] == "log").count();
    assert!(s.script_pump().is_none());
    assert_eq!(logs(&mut s), 1);
    TEST_ELAPSED.set(Some(Duration::from_millis(249)));
    assert!(s.script_pump().is_none());
    assert!(s.script_pump().is_none());
    assert_eq!(logs(&mut s), 1, "the run waits");
    assert_eq!(s.script_sleep_left(), Some(Duration::from_millis(1)), "the app can wait this long before the next frame");
    TEST_ELAPSED.set(Some(Duration::from_millis(250)));
    let r = s.script_pump().unwrap().unwrap();
    assert_eq!(r["log"], json!([1, 2]));
    TEST_ELAPSED.set(None);
}

#[test]
fn a_sleeping_run_stops_when_asked() {
    let mut s = Session::new();
    s.script_frames = true;
    TEST_ELAPSED.set(Some(Duration::ZERO));
    s.execute("script.run", &json!({"source": "$.sleep(1000); vc.log(2);", "name": "t.js"})).unwrap();
    assert!(s.script_pump().is_none());
    assert_eq!(s.script_sleep_left(), Some(Duration::from_secs(1)));
    s.execute("script.stop", &json!({})).unwrap();
    assert_eq!(s.script_sleep_left(), None, "a run asked to stop does not wait");
    assert!(s.script_pump().unwrap().is_err());
    assert_eq!(s.script_report().unwrap()["kind"], "stopped");
    TEST_ELAPSED.set(None);
}

#[test]
fn a_sleeping_run_stops_at_the_next_slice_once_its_work_is_canceled() {
    for (asleep_until, when) in
        [(None, "before its first slice"), (Some(Duration::ZERO), "while it sleeps"), (Some(Duration::from_secs(1)), "at its wake time")]
    {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let before = (format!("{:?}", s.documents()[0].doc.layers), s.documents()[0].history.undo.len());
        s.script_frames = true;
        TEST_ELAPSED.set(Some(Duration::ZERO));
        let source = "vc.run('shape.rectangle', {x: 0, y: 0, width: 5, height: 5}); $.sleep(1000); vc.log('woke'); \
                      vc.run('shape.rectangle', {x: 9, y: 0, width: 5, height: 5});";
        s.execute("script.run", &json!({"source": source, "name": "t.js"})).unwrap();
        let token = CancelToken::new();
        if let Some(at) = asleep_until {
            assert!(cancel::scope(&token, None, || s.script_pump()).is_none(), "{when}");
            assert_eq!(s.script_sleep_left(), Some(Duration::from_secs(1)), "{when}");
            TEST_ELAPSED.set(Some(at));
        }
        token.cancel();
        let ended = cancel::scope(&token, None, || s.script_pump());
        TEST_ELAPSED.set(None);
        assert!(ended.unwrap_or_else(|| panic!("{when}: the run goes on waiting")).is_err(), "{when}");
        let report = s.script_report().unwrap();
        assert_eq!((report["kind"].as_str(), &report["log"]), (Some("stopped"), &json!([])), "{when}: {report}");
        assert!(!s.script_running(), "{when}");
        assert_eq!((format!("{:?}", s.documents()[0].doc.layers), s.documents()[0].history.undo.len()), before, "{when}");
    }
}

#[test]
fn sleep_takes_milliseconds_from_0_to_1000() {
    let mut s = Session::new();
    assert_eq!(value(&mut s, "($.sleep(0), $.sleep(1), 'woke')"), json!("woke"));
    for bad in ["$.sleep(-1)", "$.sleep(1001)"] {
        assert!(thrown(&mut s, bad).starts_with("RangeError"), "{bad}");
    }
    for bad in ["$.sleep()", "$.sleep('5')", "$.sleep(NaN)", "$.sleep(Infinity)"] {
        assert!(thrown(&mut s, bad).starts_with("TypeError"), "{bad}");
    }
}

#[test]
fn sleep_throws_where_the_run_cannot_wait_instead_of_running_on() {
    let mut s = Session::new();
    s.script_frames = true;
    TEST_ELAPSED.set(Some(Duration::ZERO));
    let source = "var o = {}; Object.defineProperty(o, 'x', { get: function () { $.sleep(250); vc.log('after'); return 1; } });
        try { o.x; } catch (e) { vc.log(e.name + ': ' + e.message); }
        try { [1].map(function () { $.sleep(250); vc.log('after'); }); } catch (e) { vc.log(e.name + ': ' + e.message); }
        [1].map(function () { $.sleep(0); });
        vc.log('done');";
    s.execute("script.run", &json!({"source": source, "name": "t.js"})).unwrap();
    let r = s.script_pump().expect("the run does not wait").unwrap();
    TEST_ELAPSED.set(None);
    let message = "Error: $.sleep: a run cannot wait inside a callback of a built-in function, a getter, a conversion or a file $.evalFile runs; call it from the script's own code";
    assert_eq!(r["log"], json!([message, message, "done"]));
}

#[test]
fn sleep_throws_where_the_run_cannot_wait_without_a_clock_too() {
    let mut s = Session::new();
    TEST_NO_CLOCK.set(true);
    let source = "var o = {}; Object.defineProperty(o, 'x', { get: function () { $.sleep(250); return 1; } });
        try { o.x; vc.log('no error'); } catch (e) { vc.log(e.name); }
        $.sleep(250);
        vc.log('done');";
    let log = logged(&mut s, source);
    TEST_NO_CLOCK.set(false);
    assert_eq!(log, json!(["Error", "done"]), "the script's own code does not wait without a clock");
}
