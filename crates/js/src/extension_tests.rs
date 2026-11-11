//! The script extensions: `localize`, `toSource`, host properties with setters and the items that
//! read the interpreter's own state.

use std::cell::RefCell;
use std::rc::Rc;

use super::*;

fn realm() -> Realm {
    let mut r = Realm::new();
    r.add_script_extensions().unwrap();
    r
}

fn text(r: &mut Realm, source: &str) -> String {
    match r.run(source) {
        Ok(JsValue::String(s)) => s,
        other => panic!("{source}: {other:?}"),
    }
}

fn thrown(r: &mut Realm, source: &str) -> String {
    match r.run(source) {
        Err(e) if e.kind == RunErrorKind::Thrown => e.message,
        other => panic!("{source}: {other:?}"),
    }
}

#[test]
fn localize_looks_up_the_locale_with_its_platform_then_its_language_then_english() {
    let mut r = realm();
    let l = r.localization();
    l.set_locale("de_DE");
    l.set_platform(Some("Win"));
    assert_eq!(l.keys(), ["de_DE_Win", "de_DE", "de_Win", "de", "en"]);
    let all = "({en: 'e', de: 'd', de_Win: 'dw', de_DE: 'dd', de_DE_Win: 'ddw'})";
    assert_eq!(text(&mut r, &format!("localize({all})")), "ddw");
    assert_eq!(text(&mut r, "localize({en: 'e', de: 'd', de_Win: 'dw', de_DE: 'dd'})"), "dd");
    assert_eq!(text(&mut r, "localize({en: 'e', de: 'd', de_Win: 'dw'})"), "dw");
    assert_eq!(text(&mut r, "localize({en: 'e', de: 'd', DE: 'x'})"), "d", "names are case sensitive");
    assert_eq!(text(&mut r, "localize({en: 'e', fr: 'f'})"), "e");
    l.set_platform(None);
    assert_eq!(text(&mut r, &format!("localize({all})")), "dd");
    l.set_locale("en");
    assert_eq!(l.keys(), ["en"]);
}

#[test]
fn localize_replaces_numbered_values_once_and_leaves_the_rest() {
    let mut r = realm();
    assert_eq!(text(&mut r, "localize({en: '%2 before %1, %1 again'}, 'a', 'b')"), "b before a, a again");
    assert_eq!(text(&mut r, "localize('%1%% %3 %0 %', 'x', '%1')"), "x%% %3 %0 %");
    assert_eq!(text(&mut r, "localize('%2', 'x', '%1')"), "%1", "a value is not replaced again");
    assert_eq!(text(&mut r, "localize('%1', {toString: function () { return 'o'; }})"), "o");
    assert_eq!(text(&mut r, "var n = 0; var v = {toString: function () { n++; return 'v'; }}; localize('%1%1', v) + n"), "vv1");
}

#[test]
fn localize_gives_back_an_object_without_the_locale_and_runs_no_getter() {
    let mut r = realm();
    r.localization().set_locale("fr_FR");
    assert_eq!(r.run("var o = {de: 'd'}; localize(o) === o").unwrap(), JsValue::Bool(true));
    let source = "var hit = false; var o = {get fr() { hit = true; return 'g'; }, en: 'e'}; localize(o) + ',' + hit";
    assert_eq!(text(&mut r, source), "e,false");
    assert_eq!(text(&mut r, "localize({fr: 1, en: 'e'})"), "e", "a value that is not a string is passed over");
    for bad in ["localize()", "localize(1)", "localize(null)"] {
        assert!(thrown(&mut r, bad).starts_with("TypeError"), "{bad}");
    }
}

#[test]
fn to_string_localizes_an_object_only_while_localization_is_on() {
    let mut r = realm();
    r.localization().set_locale("ja");
    let source = "String({en: 'Hello', ja: 'こんにちは'}) + '|' + String({x: 1})";
    assert_eq!(text(&mut r, source), "[object Object]|[object Object]");
    r.localization().set_enabled(true);
    assert_eq!(text(&mut r, source), "こんにちは|[object Object]");
}

#[test]
fn to_source_writes_plain_data_as_an_expression_that_makes_it_again() {
    let mut r = realm();
    let source = r#"toSource({a: [1, "x\n\"", null, true, -0, 1.5e300], "b c": {d: undefined}, e: {}, f: []})"#;
    let want = r#"({"a":[1, "x\n\"", null, true, -0, 1.5e+300], "b c":({"d":(void 0)}), "e":({}), "f":[]})"#;
    assert_eq!(text(&mut r, source), want);
    assert_eq!(text(&mut r, "[1, , 3].toSource() + '|' + [1, ,].toSource() + '|' + [,].toSource()"), "[1, , 3]|[1, ,]|[,]");
    assert_eq!(
        text(&mut r, "toSource(NaN) + toSource(-Infinity) + toSource() + toSource('\\u2028\\ud800\\x7f')"),
        "NaN-Infinity(void 0)\"\\u2028\\uD800\\u007F\""
    );
    assert_eq!(text(&mut r, "(5).toSource() + 'a'.toSource() + true.toSource()"), "5\"a\"true");
    // Parsed back, the text makes the same value: holes, -0 and undefined included.
    let round = r#"var v = {a: [1, , "q\u0001'\\"], b: {"0": -0, "": undefined, "x y": [[]]}};
        var w = eval(toSource(v));
        [JSON.stringify(w) === JSON.stringify(v), w.a.length, 1 in w.a, 1 / w.b[0], "" in w.b].join()"#;
    assert_eq!(text(&mut r, round), "true,3,false,-Infinity,true");
    let shared = "var s = {k: 1}; toSource([s, s])";
    assert_eq!(text(&mut r, shared), r#"[({"k":1}), ({"k":1})]"#, "a value seen twice that is not its own ancestor is written twice");
}

#[test]
fn to_source_refuses_what_is_not_plain_data() {
    let mut r = realm();
    for (source, kind) in [
        ("var o = {}; o.o = o; toSource(o)", "TypeError"),
        ("var a = [1]; a.push([a]); toSource(a)", "TypeError"),
        ("toSource({f: function () {}})", "TypeError"),
        ("toSource({get g() { return 1; }})", "TypeError"),
        ("toSource(new Date(0))", "TypeError"),
        ("toSource([/x/])", "TypeError"),
        ("toSource(new Error('e'))", "TypeError"),
        ("toSource(new String('s'))", "TypeError"),
        ("var a = [1]; a.extra = 2; toSource(a)", "TypeError"),
        ("toSource(Object.create(Array.prototype))", "TypeError"),
        ("var o = {}; Object.defineProperty(o, 'hidden', {value: 1}); toSource(o)", "TypeError"),
        ("var a = [0]; Object.defineProperty(a, '0', {value: 1, enumerable: false}); toSource(a)", "TypeError"),
        ("toSource(Object.create({a: 1}))", "TypeError"),
        ("toSource(Object.create(null))", "TypeError"),
        ("function F() { this.a = 1; } toSource(new F())", "TypeError"),
        ("var d = []; for (var i = 0; i < 70; i++) d = [d]; toSource(d)", "RangeError"),
        ("var a = []; a.length = 2000000; toSource(a)", "RangeError"),
    ] {
        assert!(thrown(&mut r, source).starts_with(kind), "{source}");
    }
    assert!(thrown(&mut r, "toSource({get g() { throw 'ran'; }})").starts_with("TypeError"), "the getter does not run");
}

#[test]
fn to_source_of_a_string_with_many_escapes_throws_a_range_error_before_the_heap_limit() {
    let mut r = realm();
    // A run's heap in the web build. 2^23 control characters, each written as six, pass a quarter
    // of it, which is all one string may take.
    r.set_limits(Limits { heap_bytes: 128 << 20, ..Limits::default() });
    let source = "var s = '\\x01'; for (var i = 0; i < 23; i++) s = s + s;
        try { toSource(s); 'no error'; } catch (e) { e.name + ': ' + e.message; }";
    assert_eq!(text(&mut r, source), "RangeError: toSource: the source is too long");
    assert_eq!(text(&mut r, "toSource('a\\x01a\\x01a\\x01a\\x01').length + ''"), "30", "short strings are written whole");
}

#[test]
fn to_source_stops_at_the_fuel_limit() {
    let mut r = realm();
    r.set_limits(Limits { fuel: 200_000, ..Limits::default() });
    let e = r.run("var a = []; for (var i = 0; i < 1000; i++) a.push(i); while (true) toSource(a);").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Fuel));
}

#[test]
fn script_items_read_the_global_object_the_last_error_the_line_and_the_stack() {
    let mut r = realm();
    let items = ["global", "error", "line", "stack"]
        .into_iter()
        .zip([ScriptItem::Global, ScriptItem::LastError, ScriptItem::Line, ScriptItem::Stack])
        .map(|(n, i)| (n.to_string(), HostItem::Script(i)))
        .collect();
    r.define_host_object("h", items).unwrap();
    assert_eq!(r.run("var later = 2; h.global === this && h.global.later === 2").unwrap(), JsValue::Bool(true));
    let source = "var before = h.error; try { null.x; } catch (e) {} var caught = h.error; try { throw 'text'; } catch (e) {}
        [before === null, caught instanceof TypeError, h.error === caught].join()";
    assert_eq!(text(&mut r, source), "true,true,true");
    assert_eq!(r.run("h.error").unwrap(), JsValue::Null, "each run starts without one");
    assert_eq!(r.run("\n\n  h.line").unwrap(), JsValue::Number(3.0));
    let source = "function outer() {\n  return inner();\n}\nfunction inner() {\n  return h.stack;\n}\nvar f = function () { return outer(); };\nf()";
    assert_eq!(text(&mut r, source), "[global]:8\n[anonymous]:7\nouter:2\ninner:5\n");
    let deep = "function d(n) { return n ? d(n - 1) : h.stack; } var s = d(100).split('\\n'); s.length + ',' + s[0]";
    let lines = text(&mut r, deep);
    assert_eq!(lines, "66,…", "64 frames, the line that some are left out, and the last line's end");
    assert!(thrown(&mut r, "'use strict'; h.line = 1").starts_with("TypeError"), "the items cannot be assigned");
}

#[test]
fn a_host_property_reads_its_getter_and_gives_assignments_to_its_setter() {
    let mut r = Realm::new();
    let seen: Rc<RefCell<Vec<Data>>> = Rc::default();
    let log = seen.clone();
    let get: HostFunction = Rc::new(move |_| Ok(Data::String("got".into())));
    let set: HostFunction = Rc::new(move |args| match args.first() {
        Some(Data::Number(n)) if *n < 0.0 => Err(HostError::Range("no negatives".into())),
        Some(d) => {
            log.borrow_mut().push(d.clone());
            Ok(Data::Undefined)
        }
        None => Ok(Data::Undefined),
    });
    r.define_host_object("h", vec![("p".into(), HostItem::Property { get, set })]).unwrap();
    assert_eq!(r.run("h.p = 5; h.p = {a: [1]}; h.p").unwrap(), JsValue::String("got".into()));
    assert_eq!(*seen.borrow(), [Data::Number(5.0), Data::Object(vec![("a".into(), Data::Array(vec![Data::Number(1.0)]))])]);
    let e = r.run("try { h.p = -1; } catch (e) { e instanceof RangeError && e.message }").unwrap();
    assert_eq!(e, JsValue::String("no negatives".into()));
}
