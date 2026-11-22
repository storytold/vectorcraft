use proptest::prelude::*;

use crate::{JsValue, LimitKind, Limits, Options, Realm, RunError, RunErrorKind};

fn es_options() -> Options {
    Options { es2015: true, ..Options::default() }
}

fn es() -> Realm {
    Realm::with_options(es_options(), Limits::default())
}

fn run(src: &str) -> JsValue {
    match es().run(src) {
        Ok(v) => v,
        Err(e) => panic!("{src}\n{e:?}"),
    }
}

fn truth(src: &str) {
    assert_eq!(run(src), JsValue::Bool(true), "{src}");
}

fn string(src: &str) -> String {
    match run(src) {
        JsValue::String(s) => s,
        other => panic!("{src}: {other:?}"),
    }
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

// ---- let and const ----

#[test]
fn let_and_const_are_scoped_to_their_block() {
    truth("let x = 1; { let x = 2; } x === 1");
    truth("var r = []; { const c = 3; r.push(c); } r[0] === 3 && typeof c === 'undefined'");
    truth("function f() { if (true) { let y = 5; } return typeof y; } f() === 'undefined'");
    truth("let a = 1, b = a + 1; b === 2");
    truth("switch (0) { case 0: let s = 6; case 1: s += 1; } typeof s === 'undefined'");
    // The cases share one scope, so a case that skips the declaration finds the binding uninitialized.
    throws("switch (1) { case 0: let s = 0; break; case 1: s = 7; }", "ReferenceError");
    truth("try { throw 1 } catch (e) { let e2 = e + 1; } typeof e2 === 'undefined'");
}

#[test]
fn a_binding_is_unusable_before_its_declaration_runs() {
    throws("x; let x = 1;", "ReferenceError");
    throws("{ x = 2; let x; }", "ReferenceError");
    throws("typeof x; const x = 1;", "ReferenceError");
    throws("function f() { return v; } f(); let v = 1;", "ReferenceError");
    throws("let q = q;", "ReferenceError");
    throws("for (let k in k) {}", "ReferenceError");
    // A declaration without an initializer initializes to undefined where it stands.
    truth("let u; u === undefined");
    // A function that runs after the declaration sees the binding.
    truth("function g() { return w; } let w = 4; g() === 4");
}

#[test]
fn assigning_a_const_is_a_type_error_in_any_code() {
    throws("const c = 1; c = 2;", "TypeError");
    throws("'use strict'; const c = 1; c = 2;", "TypeError");
    throws("const c = 1; c++;", "TypeError");
    throws("const c = 1; c += 1;", "TypeError");
    throws("for (const i = 0; i < 2; i++) {}", "TypeError");
    truth("const o = {}; o.x = 1; o.x === 1");
}

#[test]
fn redeclarations_and_bad_declarations_are_syntax_errors() {
    syntax("let a; let a;");
    syntax("let a; var a;");
    syntax("var a; let a;");
    syntax("const a = 1; function a() {}");
    syntax("{ let a; { var a; } }");
    syntax("function f(p) { let p; }");
    syntax("try {} catch (e) { let e; }");
    syntax("const c;");
    syntax("let let = 1;");
    syntax("let [a] = [1];");
    syntax("'use strict'; { function f() {} function f() {} }");
    syntax("for (let x = 1 in {}) {}");
    syntax("if (1) let x = 1;");
    // Outside strict mode code `let` is still a name where no declaration can start.
    truth("var let = 3; let === 3");
    truth("{ function f() { return 1; } function f() { return 2; } } f() === 2");
}

#[test]
fn a_let_loop_gives_each_iteration_its_own_binding() {
    truth("var fs = []; for (let i = 0; i < 3; i++) { fs.push(function() { return i; }); } fs[0]() === 0 && fs[1]() === 1 && fs[2]() === 2");
    truth("var fs = []; for (let i = 0; i < 3; i++) { fs.push(() => i); i += 0; } fs.map(function(f) { return f(); }).join() === '0,1,2'");
    truth("var fs = []; for (let k in {a: 1, b: 2}) { fs.push(function() { return k; }); } fs[0]() === 'a' && fs[1]() === 'b'");
    truth("var fs = []; for (const k in {a: 1, b: 2}) { fs.push(() => k); } fs[0]() + fs[1]() === 'ab'");
    // `continue` and `break` leave the iteration's environment.
    truth("var s = 0; for (let i = 0; i < 5; i++) { if (i === 1) continue; if (i === 3) break; s += i; } s === 2");
    truth("var s = ''; for (let k in {a: 1, b: 2, c: 3}) { if (k === 'b') continue; s += k; } s === 'ac'");
    truth("var i = 'outer'; for (let i = 0; i < 1; i++) {} i === 'outer'");
}

#[test]
fn functions_declared_in_blocks_are_scoped_to_the_block() {
    truth("'use strict'; { function f() { return 1; } } typeof f === 'undefined'");
    // Outside strict mode code the name is also a variable, assigned when the declaration runs.
    truth("var before = typeof f; { function f() { return 1; } } before === 'undefined' && f() === 1");
    truth("{ let x = 2; function g() { return x; } } g() === 2");
    truth("function h() { if (true) { function inner() { return 'i'; } } return inner(); } h() === 'i'");
    // Only a `let` or `const` around the declaration, or a parameter, keeps the name from being a variable.
    truth("function f() { { let g; } { function g() {} } return typeof g; } f() === 'function'");
    truth("function f() { let g = 1; { function g() {} } return g; } f() === 1");
    truth("function f() { { let g = 1; { function g() {} } } return typeof g; } f() === 'undefined'");
    truth("function f() { for (let g = 0; g < 1; g++) { function g() {} } return typeof g; } f() === 'undefined'");
    truth("function f(g) { { function g() {} } return g; } f(5) === 5");
}

#[test]
fn global_lexical_bindings_stay_across_runs_and_cannot_be_redeclared() {
    let mut realm = es();
    assert_eq!(realm.run("let kept = 5; const fixed = 1; kept"), Ok(JsValue::Number(5.0)));
    assert_eq!(realm.run("kept + fixed"), Ok(JsValue::Number(6.0)));
    assert_eq!(realm.run("this.kept === undefined"), Ok(JsValue::Bool(true)));
    let again = realm.run("let kept = 6;");
    assert!(matches!(&again, Err(RunError { kind: RunErrorKind::Thrown, message, .. }) if message.starts_with("SyntaxError")), "{again:?}");
    let var = realm.run("var fixed;");
    assert!(matches!(&var, Err(RunError { kind: RunErrorKind::Thrown, message, .. }) if message.starts_with("SyntaxError")), "{var:?}");
    assert_eq!(realm.run("kept"), Ok(JsValue::Number(5.0)));
    let restricted = realm.run("let undefined = 1;");
    assert!(restricted.is_err(), "{restricted:?}");
}

#[test]
fn eval_code_keeps_its_let_and_const_to_itself() {
    truth("eval('let e = 1; e') === 1 && typeof e === 'undefined'");
    truth("function f() { eval('var v = 2; let l = 3;'); return v + typeof l; } f() === '2undefined'");
    truth("let outer = 4; eval('outer') === 4");
    truth("(0, eval)('let g = 5; g') === 5");
    // A variable eval code declares cannot take the name of a `let` or `const` around it.
    throws("let x; eval('var x;');", "SyntaxError");
    throws("let x; (0, eval)('var x;');", "SyntaxError");
    throws("function f() { let y; { eval('var y;'); } } f();", "SyntaxError");
    throws("{ let z; eval('function z() {}'); }", "SyntaxError");
    throws("{ function w() {} eval('var w;'); }", "SyntaxError");
    truth("function f() { let y; return eval(\"'use strict'; var y = 1; y\"); } f() === 1");
    truth("try { throw 1; } catch (e) { eval('var e = 2;'); } true");
    truth("function f() { { let v; } eval('var v = 3;'); return v; } f() === 3");
    // Nor, in a default parameter's value, the name of a parameter; the body can.
    throws("var n = 0; function f(a = eval('var a = 1')) { n++; } try { f(); } finally { n === 0 || ok; }", "SyntaxError");
    truth("function f(a = eval('var b = 1; b')) { return a; } f() === 1");
    truth("function f(a = 0) { eval('var a = 2;'); return a; } f() === 2");
    // Nothing is declared when the check fails.
    truth("let a; try { eval('var b; var a;'); } catch (e) {} typeof b === 'undefined'");
}

// ---- arrow functions ----

#[test]
fn arrow_functions_take_this_and_arguments_from_around_them() {
    truth("var o = { v: 1, f: function() { return () => this.v; } }; o.f()() === 1");
    truth("var o = { v: 1, f: function() { return () => () => this.v; } }; o.f()()() === 1");
    truth("function f() { return (() => arguments[0])(); } f(7) === 7");
    truth("var o = { v: 2, f: function() { var g = () => this.v; return g.call({ v: 3 }); } }; o.f() === 2");
    truth("var o = { f: function() { return () => eval('this'); } }; o.f()() === o");
    truth("(() => this)() === this");
}

#[test]
fn arrow_functions_have_expression_and_block_bodies() {
    truth("var f = x => x * 2; f(4) === 8");
    truth("var f = (a, b) => a + b; f(1, 2) === 3");
    truth("var f = () => { return 3; }; f() === 3");
    truth("var f = () => ({ a: 1 }); f().a === 1");
    truth("var f = (a = 2, ...r) => a + r.length; f(undefined, 1, 2) === 4");
    truth("var f = x => y => x + y; f(1)(2) === 3");
    truth("var t = true ? () => 1 : () => 2; t() === 1");
    truth("[1, 2, 3].map(x => x + 1).join() === '2,3,4'");
    truth("var f = (x) => x in { a: 1 }; f('a')");
    truth("var f = a => {}; f() === undefined");
}

#[test]
fn arrow_functions_cannot_be_constructed() {
    truth("var f = () => 1; f.prototype === undefined && f.length === 0 && !f.hasOwnProperty('prototype')");
    throws("var f = () => 1; new f();", "TypeError");
    throws("var f = () => 1; var b = f.bind(null); new b();", "TypeError");
    truth("((a, b) => 0).length === 2");
}

#[test]
fn bad_arrow_functions_are_syntax_errors() {
    syntax("var f = x\n=> x;");
    syntax("var f = (x, x) => 1;");
    syntax("var f = (a, ...b, c) => 1;");
    syntax("var f = ((a)) => 1;");
    syntax("var f = (a + 1) => 1;");
    syntax("var f = (...a);");
    syntax("var f = ();");
    syntax("'use strict'; var f = eval => 1;");
    syntax("var f = (a = 1) => { 'use strict'; };");
    syntax("1 + () => 2;");
}

// ---- template literals ----

#[test]
fn template_literals_join_their_pieces_and_substitutions() {
    assert_eq!(string("`a${1 + 1}b${'c'}`"), "a2bc");
    assert_eq!(string("``"), "");
    assert_eq!(string("`${'x'}`"), "x");
    assert_eq!(string("var o = { toString: function() { return 'T'; }, valueOf: function() { return 'V'; } }; `${o}`"), "T");
    assert_eq!(string("`a${`b${'c'}d`}e`"), "abcde");
    assert_eq!(string("`line\nnext`"), "line\nnext");
    assert_eq!(string("`cr\r\nlf`"), "cr\nlf");
    assert_eq!(string("`\\u{41}\\x42\\u0043`"), "ABC");
    assert_eq!(string("`a\\\nb`"), "ab");
    assert_eq!(string("`{${1}}`"), "{1}");
    assert_eq!(string("`$`"), "$");
    assert_eq!(string("`\\${1}`"), "${1}");
    // Substitutions run left to right.
    assert_eq!(string("var s = ''; `${s += 'a'}${s += 'b'}`; s"), "ab");
}

#[test]
fn tagged_templates_get_one_frozen_strings_object_for_each_site() {
    truth("function tag(s, a, b) { return s.length === 3 && s[0] === 'x' && s[1] === '' && s[2] === 'y' && a === 1 && b === 2; } tag`x${1}${2}y`");
    truth("function tag(s) { return s.raw[0]; } tag`a\\nb` === 'a\\\\nb'");
    truth("function tag(s) { return s[0]; } tag`a\\nb` === 'a\\nb'");
    truth("function tag(s) { return s; } function f() { return tag`k`; } f() === f()");
    truth("function tag(s) { return s; } tag`k` !== tag`k`");
    truth(
        "function tag(s) { return Object.isFrozen(s) && Object.isFrozen(s.raw) && !Object.getOwnPropertyDescriptor(s, 'raw').enumerable; } tag`a${0}b`",
    );
    truth("var o = { v: 1, m: function(s) { return this.v; } }; o.m`x` === 1");
    truth("function tag(s) { return s.raw.join('|'); } tag`a\r\nb${0}\\\r\nc` === 'a\\nb|\\\\\\nc'");
}

#[test]
fn bad_template_literals_are_syntax_errors() {
    syntax("`abc");
    syntax("`${1`");
    syntax("`\\01`");
    syntax("`\\1`");
    syntax("`\\xZ`");
    syntax("`\\u{110000}`");
    syntax("function t() {} t`\\u`");
}

// ---- default and rest parameters ----

#[test]
fn default_parameters_apply_to_undefined_arguments_in_order() {
    truth("function f(a, b = a + 1) { return b; } f(1) === 2 && f(1, 5) === 5 && f(1, undefined) === 2 && f(1, null) === null");
    truth("var n = 0; function f(a = ++n, b = ++n) { return [a, b].join(); } f() === '1,2' && f(9) === '9,3'");
    throws("function f(a = b, b) {} f();", "ReferenceError");
    throws("function f(a = a) {} f();", "ReferenceError");
    truth("function f(a, b = 1, c) {} f.length === 1");
    truth("function f(a, ...r) {} f.length === 1");
    // A function with defaults has an arguments object that does not follow the parameters.
    truth("function f(a = 0) { a = 9; return arguments[0]; } f(1) === 1");
    truth("function f(a = 0) { arguments[0] = 9; return a; } f(1) === 1");
    // The body's variables start with the parameter's value and are kept apart from it.
    truth("function f(a = 1) { var a; return a; } f() === 1");
    truth("function f(a = 1, g = () => a) { var a = 2; return g(); } f() === 1");
    truth("var x = 'outer'; function f(a = () => x) { var x = 'inner'; return a(); } f() === 'outer'");
}

#[test]
fn rest_parameters_collect_the_remaining_arguments() {
    truth("function f(a, ...r) { return Array.isArray(r) && r.length === 2 && r[0] === 2 && r[1] === 3; } f(1, 2, 3)");
    truth("function f(...r) { return r.length; } f() === 0");
    truth("function f(...r) { r.push(1); return r.length; } f(0) === 2");
    syntax("function f(...r, a) {}");
    syntax("function f(...r = []) {}");
    syntax("function f(a, a = 1) {}");
    syntax("function f(...a) { 'use strict'; }");
    // Already strict mode code too.
    syntax("'use strict'; function f(a = 1) { 'use strict'; }");
    syntax("'use strict'; var f = (...a) => { 'use strict'; };");
    syntax("var o = { set s(...v) {} };");
}

// ---- spread ----

#[test]
fn spread_expands_arrays_strings_and_array_like_objects() {
    truth("var a = [1, 2]; var b = [0, ...a, 3]; b.join() === '0,1,2,3' && b.length === 4");
    truth("var b = [...'a\\uD83D\\uDE00b']; b.length === 3 && b[1] === '\\uD83D\\uDE00'");
    truth("var b = [...{ length: 2, 0: 'x', 1: 'y' }]; b.join() === 'x,y'");
    truth("var b = [...[1, , 3]]; b.length === 3 && 1 in b && b[1] === undefined");
    truth("var b = [, ...[1]]; b.length === 2 && !(0 in b)");
    truth("function f() { return arguments.length; } f(...[1, 2], 3, ...[]) === 3");
    truth("function f(a, b) { return a + b; } f(...[1, 2]) === 3");
    truth("var o = { v: 1, m: function(x) { return this.v + x; } }; o.m(...[1]) === 2");
    truth("function C(a, b) { this.s = a + b; } new C(...[1, 2]).s === 3");
    truth("Math.max(...[1, 5, 2]) === 5");
    truth("var s = ''; function f() {} f((s += 'a', 0), ...(s += 'b', []), (s += 'c', 0)); s === 'abc'");
    truth("function f() { return arguments; } var a = f(...'ab'); a[0] === 'a' && a[1] === 'b'");
    truth("eval(...['1 + 1']) === 2");
}

#[test]
fn spreading_other_values_is_a_type_error() {
    throws("[...1];", "TypeError");
    throws("[...null];", "TypeError");
    throws("[...undefined];", "TypeError");
    throws("[...{}];", "TypeError");
    throws("Math.max(...true);", "TypeError");
    throws("new Array(...{ a: 1 });", "TypeError");
}

#[test]
fn spread_and_templates_stay_within_the_limits() {
    let mut realm = Realm::with_options(es_options(), Limits { fuel: 10_000, ..Limits::default() });
    let e = realm.run("[...{ length: 4294967294 }]").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Fuel), "{e:?}");
    let mut realm = Realm::with_options(es_options(), Limits { heap_bytes: 4 << 20, ..Limits::default() });
    let e = realm.run("var s = 'x'; for (var i = 0; i < 22; i++) s += s; [...s]").unwrap_err();
    assert!(matches!(e.kind, RunErrorKind::Limit(_)), "{e:?}");
    throws("function f() {} f(...{ length: 100000 });", "RangeError");
    throws("var a = []; a.length = 4294967294; [0, ...a];", "RangeError");
}

#[test]
fn new_scopes_survive_collections_while_they_are_made() {
    // Heaps just big enough make the collector run while function bodies and blocks set up their
    // scopes; each run gives the right total or stops at the heap limit.
    let src = "var total = 0; \
        function f(a = 1, ...r) { let x = a + r.length; const y = 1; function g() { return x + y; } function h() { return g(); } var junk = [a, a, a]; return h(); } \
        for (var i = 0; i < 300; i++) { { let b = i; function k() { return b; } function m() { return k(); } total += f(1, 2) + m() - i; } } total";
    let mut completed = 0;
    for kib in (16..400).step_by(2) {
        let mut realm = Realm::with_options(es_options(), Limits { heap_bytes: kib << 10, ..Limits::default() });
        match realm.run(src) {
            Ok(v) => {
                assert_eq!(v, JsValue::Number(900.0), "{kib} KiB");
                completed += 1;
            }
            Err(e) => assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{kib} KiB: {e:?}"),
        }
    }
    assert!(completed > 0);
}

#[test]
fn nested_arrows_and_templates_stay_within_the_depth_limit() {
    let deep = format!("{}0", "x => ".repeat(400));
    let e = es().run(&deep).unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Syntax, "{e:?}");
    let deep = format!("{}1{}", "`${".repeat(300), "}`".repeat(300));
    let e = es().run(&deep).unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Syntax, "{e:?}");
    assert_eq!(string(&format!("{}1{}", "`${".repeat(50), "}`".repeat(50))), "1");
}

#[test]
fn for_each_takes_let_and_const_with_e4x() {
    let mut realm = Realm::with_options(Options { e4x: true, ..es_options() }, Limits::default());
    let src = "var fs = []; for each (let v in [1, 2]) { fs.push(() => v); } \
               var s = 0; for each (const w in { a: 3, b: 4 }) { s += w; } \
               fs[0]() + fs[1]() + s === 10 && typeof v === 'undefined'";
    assert_eq!(realm.run(src), Ok(JsValue::Bool(true)));
    let e = Realm::with_options(Options { e4x: true, ..es_options() }, Limits::default()).run("for each (let a = 1 in []) {}").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Syntax, "{e:?}");
}

// ---- the language mode ----

#[test]
fn without_the_option_the_new_forms_are_syntax_errors() {
    for src in ["let x = 1; x", "const c = 1;", "var f = x => x;", "`t`", "function f(a = 1) {}", "function f(...r) {}", "f(...a)", "[...a]"] {
        let r = Realm::new().run(src);
        assert!(matches!(r, Err(RunError { kind: RunErrorKind::Syntax, .. })), "{src}: {r:?}");
    }
    // `let` stays a name outside strict mode code.
    assert_eq!(Realm::new().run("var let = 1; let"), Ok(JsValue::Number(1.0)));
}

#[test]
fn code_made_at_run_time_reads_the_realm_s_language() {
    truth("eval('let a = 1; `${a}`') === '1'");
    truth("new Function('a = 2', 'return a')() === 2");
    truth("Function('...r', 'return r.length')(1, 2) === 2");
    let legacy = Realm::new().run("eval('let a = 1;')");
    assert!(matches!(&legacy, Err(RunError { kind: RunErrorKind::Thrown, message, .. }) if message.starts_with("SyntaxError")), "{legacy:?}");
}

// ---- robustness ----

const FRAGMENTS: &[&str] = &[
    "let ",
    "const ",
    "var ",
    "x",
    "y",
    "=",
    "=>",
    "(",
    ")",
    "{",
    "}",
    "[",
    "]",
    "...",
    ",",
    ";",
    "`",
    "${",
    "a",
    "1",
    "+",
    "function ",
    "f",
    "return ",
    "for (",
    " in ",
    "if (",
    "this",
    "arguments",
    "new ",
    "\n",
    "'use strict';",
    ".",
    "?",
    ":",
    "\\",
    "switch (",
    "case ",
    "default:",
    "break;",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_es2015_fragment_programs_never_panic(parts in proptest::collection::vec(0..FRAGMENTS.len(), 0..60)) {
        let src: String = parts.iter().map(|&i| FRAGMENTS[i]).collect();
        let mut realm = Realm::with_options(es_options(), Limits { fuel: 20_000, heap_bytes: 1 << 20, frames: 200, reentry: 8 });
        if let Err(e) = realm.run(&src) {
            prop_assert!(e.kind != RunErrorKind::Internal, "{src:?}: {e:?}");
        }
    }

    #[test]
    fn arbitrary_es2015_text_never_panics(src in "[`$\\{\\}=>.()a-c1 ;,\\[\\]\\\\\n]{0,80}") {
        let mut realm = Realm::with_options(es_options(), Limits { fuel: 20_000, heap_bytes: 1 << 20, frames: 200, reentry: 8 });
        if let Err(e) = realm.run(&src) {
            prop_assert!(e.kind != RunErrorKind::Internal, "{src:?}: {e:?}");
        }
    }
}
