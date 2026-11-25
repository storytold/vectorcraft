use proptest::prelude::*;

use crate::{JsValue, Limits, Options, Realm, RunError, RunErrorKind};

fn es_options() -> Options {
    Options { es2015: true, ..Options::default() }
}

fn es() -> Realm {
    Realm::with_options(es_options(), Limits::default())
}

fn truth(src: &str) {
    assert_eq!(es().run(src), Ok(JsValue::Bool(true)), "{src}");
}

/// The script throws an error whose message starts with `name`.
fn throws(src: &str, name: &str) {
    match es().run(src) {
        Err(RunError { kind: RunErrorKind::Thrown, message, .. }) => assert!(message.starts_with(name), "{src}: {message}"),
        other => panic!("{src}: {other:?}"),
    }
}

/// The script does not parse.
fn syntax(src: &str) {
    match es().run(src) {
        Err(RunError { kind: RunErrorKind::Syntax, .. }) => {}
        other => panic!("{src}: {other:?}"),
    }
}

// ---- optional chains ----

#[test]
fn an_optional_chain_stops_at_undefined_or_null() {
    truth("var a = null; a?.b.c === undefined");
    truth("var a; a?.b === undefined && a?.[x] === undefined && a?.() === undefined && a?.b.c(d).e === undefined");
    truth("var o = { b: { c: 1 } }; o?.b.c === 1 && o.b?.c === 1 && o?.['b']?.c === 1 && o.x?.c === undefined");
    // Values that are falsy but neither undefined nor null go on.
    truth("(0)?.toFixed(1) === '0.0' && ''?.length === 0 && false?.toString() === 'false'");
    // A short circuit skips every key and argument of the rest of the chain.
    truth("var n = 0, a = null; a?.[n++]; a?.b(n++); a?.b[n++].c(n++); a?.b?.[n++]; n === 0");
    // The base is evaluated once.
    truth("var n = 0; function f() { n++; return { b: 2 }; } f()?.b === 2 && n === 1");
}

#[test]
fn only_an_optional_step_skips_the_rest() {
    // Parentheses end the chain.
    throws("var a = null; (a?.b).c", "TypeError");
    // A plain access after a step that found a value still throws on undefined.
    throws("var a = {}; a?.b.c", "TypeError");
    throws("var a = {}; a?.b()", "TypeError");
    // A chain inside a key or an argument is a chain of its own.
    truth("var o = { u: 3 }, k = null; o?.[k?.x ?? 'u'] === 3");
}

#[test]
fn an_optional_call_keeps_its_this() {
    let o = "var o = { v: 7, f: function () { return this.v; } };";
    for call in ["o?.f()", "o.f?.()", "o?.f?.()", "(o?.f)()", "(o.f)?.()", "(o?.f)?.()", "o?.['f']()", "o['f']?.()"] {
        truth(&format!("{o} {call} === 7"));
    }
    truth(&format!("{o} o.g?.() === undefined && o?.g?.(x) === undefined"));
    throws("var o = { g: 1 }; o.g?.()", "TypeError");
    truth("function h() { return this; } h?.() === this");
    truth("var r = []; var f = function (a, b) { r.push(a, b); return r.length; }; f?.(1, 2) === 2 && r[1] === 2");
}

#[test]
fn an_optional_call_of_eval_is_an_indirect_eval() {
    truth("var x = 'global'; function f() { var x = 'local'; return eval('x') + ' ' + eval?.('x'); } f() === 'local global'");
}

#[test]
fn delete_of_an_optional_chain() {
    truth("var a = null, n = 0; delete a?.b === true && delete a?.[n++] === true && n === 0");
    truth("var o = { b: 1, c: 2 }; delete o?.b && delete o?.['c'] && !('b' in o) && !('c' in o)");
    throws("'use strict'; var o = {}; Object.defineProperty(o, 'b', { value: 1 }); delete o?.b", "TypeError");
    truth("var o = {}; Object.defineProperty(o, 'b', { value: 1 }); delete o?.b === false");
}

#[test]
fn optional_chains_where_new_and_tags_can_use_them() {
    truth("function F() { this.v = 2; } new F()?.v === 2");
    truth("var o = { F: function () { this.v = 3; } }; new (o?.F)().v === 3");
    truth("var o = { t: function (s) { return s[0]; } }; (o?.t)`x` === 'x'");
}

#[test]
fn a_question_mark_before_a_number_is_a_conditional() {
    truth("var a = true; (a?.5:1) === 0.5");
    truth("var a = false; (a?.5:1) === 1");
    truth("var a = true; (a ?.5 : 1) === 0.5");
}

#[test]
fn an_optional_chain_cannot_be_assigned_constructed_or_tagged() {
    for src in [
        "a?.b = 1",
        "a?.b += 1",
        "a?.[0] = 1",
        "(a?.b) = 1",
        "a?.b++",
        "--a?.b",
        "for (a?.b in {}) {}",
        "a?.b`x`",
        "a?.b\n`x`",
        "a?.`x`",
        "new a?.b()",
        "new a?.()",
        "a?.",
        "a?.1",
    ] {
        syntax(src);
    }
}

#[test]
fn a_long_chain_and_deep_parentheses_are_bounded() {
    let src = format!("var o = {{}}; o.o = o; o{} === o", "?.o".repeat(20_000));
    truth(&src);
    syntax(&format!("var a; {}a?.b{}", "(".repeat(500), ")".repeat(500)));
}

// ---- nullish coalescing ----

#[test]
fn coalesce_takes_the_right_side_only_for_undefined_or_null() {
    truth("(0 ?? 1) === 0 && (false ?? 1) === false && ('' ?? 1) === '' && isNaN(NaN ?? 1)");
    truth("(null ?? 1) === 1 && (undefined ?? 2) === 2 && (void 0 ?? null) === null");
    truth("var n = 0; 1 ?? n++; null ?? n++; n === 1");
    truth("(null ?? undefined ?? 3) === 3 && (null ?? 4 ?? 5) === 4");
    truth("var o = {}; (o.x ?? 'd') === 'd' && (o?.x?.y ?? 'e') === 'e'");
}

#[test]
fn coalesce_binds_looser_than_bitwise_and_tighter_than_the_conditional() {
    truth("(1 ?? 2 | 4) === 1");
    truth("(1 ?? 0 ? 'a' : 'b') === 'a'");
    truth("var x = null ?? 7, y = 1 ?? 8; x === 7 && y === 1");
    truth("((null || 1) ?? 2) === 1 && (null ?? (0 && 1)) === 0 && ((0 ?? 1) || 2) === 2 && ((null && 1) ?? 2) === 2");
}

#[test]
fn coalesce_mixed_with_and_or_or_needs_parentheses() {
    for src in ["a || b ?? c", "a ?? b || c", "a && b ?? c", "a ?? b && c", "a ?? b ?? c || d", "a ??= 1", "a ?? "] {
        syntax(src);
    }
}

// ---- the language mode ----

#[test]
fn es5_and_extendscript_realms_do_not_read_the_new_operators() {
    let legacy = [Options::default(), Options { e4x: true, preprocessor_lines: true, ..Options::default() }];
    for options in legacy {
        for src in ["var a; a?.b", "var a; a?.[0]", "var f; f?.()", "null ?? 1"] {
            let r = Realm::with_options(options.clone(), Limits::default()).run(src);
            assert!(matches!(r, Err(RunError { kind: RunErrorKind::Syntax, .. })), "{src}: {r:?}");
        }
        // The conditional with a number after `?` reads as it always has.
        let r = Realm::with_options(options.clone(), Limits::default()).run("var a = true; a?.5:1");
        assert_eq!(r, Ok(JsValue::Number(0.5)));
    }
}

#[test]
fn code_made_at_run_time_reads_the_new_operators() {
    truth("eval('null ?? 4') === 4 && eval('var q = null; q?.r') === undefined");
    truth("new Function('a', 'return a?.b ?? 5')(null) === 5");
}

// ---- robustness ----

const FRAGMENTS: &[&str] = &[
    "a",
    "b",
    "f",
    "1",
    ".5",
    "?.",
    "??",
    "?",
    ":",
    ".",
    "(",
    ")",
    "[",
    "]",
    "`",
    "=",
    "++",
    "delete ",
    "new ",
    "&&",
    "||",
    ",",
    ";",
    "null",
    "this",
    "\n",
    "=>",
    "...",
    "{",
    "}",
    "'use strict';",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_es2020_fragment_programs_never_panic(parts in proptest::collection::vec(0..FRAGMENTS.len(), 0..60)) {
        let src: String = parts.iter().map(|&i| FRAGMENTS[i]).collect();
        let mut realm = Realm::with_options(es_options(), Limits { fuel: 20_000, heap_bytes: 1 << 20, frames: 200, reentry: 8 });
        if let Err(e) = realm.run(&src) {
            prop_assert!(e.kind != RunErrorKind::Internal, "{src:?}: {e:?}");
        }
    }
}
