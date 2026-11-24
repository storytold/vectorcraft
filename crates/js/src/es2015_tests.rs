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
    syntax("let [a];");
    syntax("let [let] = [1];");
    syntax("let [a, a] = [1, 2];");
    syntax("try {} catch ([e]) { let e; }");
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
    // Each declaration is judged on its own: one inside a `let` of the same name leaves the variable alone.
    truth("function f() { { function g() { return 1; } } { let g; { function g() { return 2; } } } return g(); } f() === 1");
    truth("{ function t() { return 1; } } { let t; { function t() { return 2; } } } t() === 1");
}

#[test]
fn a_block_function_inside_another_of_its_name_leaves_the_variable_alone() {
    truth("var f; { function f() { return 'outer'; } { function f() { return 'inner'; } } } f() === 'outer'");
    truth("function g() { { function f() { return 1; } { function f() { return 2; } } } return f(); } g() === 1");
}

#[test]
fn a_labelled_function_in_a_block_is_made_when_the_block_is_entered() {
    truth("let f = 0; { var value = f(); label: function f() { return 7; } } value === 7 && f === 0");
    truth("{ var early = g(); a: b: function g() { return 8; } } early === 8 && g() === 8");
    truth("label: function top() { return 1; } top() === 1");
    syntax("let f; label: function f() {}");
    syntax("label: function f() {} let f;");
    syntax("{ let f; label: function f() {} }");
}

#[test]
fn a_function_standing_alone_as_an_if_branch_has_a_block_of_its_own() {
    truth("var f; if (true) function f() { return f; } var saved = f; f = 0; saved() === saved");
    truth("var g; if (false) ; else function g() { return g; } var kept = g; g = 0; kept() === kept");
    truth("if (true) function h() { return 3; } h() === 3");
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
fn a_var_an_inherited_property_satisfied_does_not_stop_a_later_let() {
    let mut realm = Realm::new();
    assert_eq!(realm.run("Object.prototype.x = 1; true"), Ok(JsValue::Bool(true)));
    let first = realm.run("var x; delete Object.prototype.x && !this.hasOwnProperty('x')");
    assert_eq!(first, Ok(JsValue::Bool(true)));
    realm.set_es2015(true);
    assert_eq!(realm.run("let x = 2; x"), Ok(JsValue::Number(2.0)));
    // A `var` that made an own property still stops one.
    assert_eq!(realm.run("var y; true"), Ok(JsValue::Bool(true)));
    let again = realm.run("let y = 1;");
    assert!(matches!(&again, Err(RunError { kind: RunErrorKind::Thrown, message, .. }) if message.starts_with("SyntaxError")), "{again:?}");
}

#[test]
fn functions_from_earlier_runs_see_later_global_lexical_bindings() {
    let mut realm = Realm::new();
    let first = realm.run("function read() { return x; } function call(f) { return f(); }");
    assert!(first.is_ok(), "{first:?}");
    let made = realm.run("(0, eval)('function viaEval() { return x; }'); var made = new Function('return x');");
    assert!(made.is_ok(), "{made:?}");
    realm.set_es2015(true);
    assert_eq!(realm.run("let x = 7; read() + call(() => x) + viaEval() + made()"), Ok(JsValue::Number(28.0)));
    realm.set_es2015(false);
    assert_eq!(realm.run("function later() { return x; } later() + read()"), Ok(JsValue::Number(14.0)));
}

#[test]
fn a_function_eval_declares_on_the_global_object_sees_global_lexical_bindings() {
    // Without the option, eval code's functions close over its variable environment; on the
    // global object that is the global scope, which holds the scripts' `let` and `const` names.
    let mut realm = es();
    assert_eq!(realm.run("let x = 7; 0"), Ok(JsValue::Number(0.0)));
    realm.set_es2015(false);
    assert_eq!(realm.run("with ({}) { eval('function read() { return x; }'); } read()"), Ok(JsValue::Number(7.0)));
    assert_eq!(realm.run("function g() { eval('function inner() { return x; }'); return inner(); } g()"), Ok(JsValue::Number(7.0)));
}

#[test]
fn a_global_var_makes_its_own_property_over_an_inherited_one() {
    // ECMA-262 2015 section 8.1.1.4.17 reads the global object's own property, so `var` makes a
    // new one even when the global object inherits the name.
    let mut realm = es();
    assert_eq!(realm.run("Object.prototype.inherited = 1; Object.prototype.viaEval = 2; 0"), Ok(JsValue::Number(0.0)));
    let own = "var inherited; (0, eval)('var viaEval'); this.hasOwnProperty('inherited') && inherited === undefined \
               && this.hasOwnProperty('viaEval') && viaEval === undefined";
    assert_eq!(realm.run(own), Ok(JsValue::Bool(true)));
    // Without the option, the inherited property counts as declared (ECMA-262 5.1 section 10.5).
    let mut realm = Realm::new();
    assert_eq!(realm.run("Object.prototype.inherited = 1; 0"), Ok(JsValue::Number(0.0)));
    assert_eq!(realm.run("var inherited; !this.hasOwnProperty('inherited') && inherited === 1"), Ok(JsValue::Bool(true)));
}

#[test]
fn a_global_function_that_cannot_be_declared_declares_nothing() {
    let mut realm = es();
    let failed = realm.run("let retained = 1; function undefined() {}");
    assert!(matches!(&failed, Err(RunError { kind: RunErrorKind::Thrown, message, .. }) if message.starts_with("TypeError")), "{failed:?}");
    assert_eq!(realm.run("typeof retained"), Ok(JsValue::String("undefined".into())));
    assert_eq!(realm.run("let retained = 2; retained"), Ok(JsValue::Number(2.0)));
}

#[test]
fn a_global_function_is_judged_by_the_global_objects_own_property() {
    // ECMA-262 2015 sections 8.1.1.4.16 and 8.1.1.4.18 read the global object's own property, so
    // a fixed property the global object inherits does not stop the declaration.
    let mut realm = es();
    let setup = "Object.defineProperty(Object.prototype, 'fixed', { value: 1 }); \
                 Object.defineProperty(Object.prototype, 'getter', { get: function () { return 1; } }); 0";
    assert_eq!(realm.run(setup), Ok(JsValue::Number(0.0)));
    assert_eq!(realm.run("function fixed() { return 2; } function getter() { return 3; } fixed() + getter()"), Ok(JsValue::Number(5.0)));
    let own = "var d = Object.getOwnPropertyDescriptor(this, 'fixed'); d.writable && d.enumerable && !d.configurable && ({}).fixed === 1";
    assert_eq!(realm.run(own), Ok(JsValue::Bool(true)));
    // A global object that is not extensible takes no new function, inherited name or not, and
    // a function declared next to one it cannot take is not declared either.
    let mut realm = es();
    assert_eq!(realm.run(&format!("{setup}; var kept = 1; Object.preventExtensions(this); 0")), Ok(JsValue::Number(0.0)));
    for decl in [
        "function fixed() {}",
        "function fresh() {}",
        "(0, eval)('function fixed() {}')",
        "(0, eval)('function fresh() {}')",
        "(0, eval)('function kept() {} function fresh() {}')",
    ] {
        let failed = realm.run(decl);
        assert!(
            matches!(&failed, Err(RunError { kind: RunErrorKind::Thrown, message, .. }) if message.starts_with("TypeError")),
            "{decl}: {failed:?}"
        );
    }
    assert_eq!(realm.run("this.hasOwnProperty('fixed') || this.hasOwnProperty('fresh') || kept !== 1"), Ok(JsValue::Bool(false)));
}

fn is_thrown(result: &Result<JsValue, RunError>, name: &str) -> bool {
    matches!(result, Err(RunError { kind: RunErrorKind::Thrown, message, .. }) if message.starts_with(name))
}

#[test]
fn a_global_var_stops_a_later_let_even_on_a_configurable_property() {
    // ECMA-262 2015 section 15.1.8 checks the names global `var` declarations made, not only the
    // property attributes.
    let mut realm = es();
    assert_eq!(realm.run("this.x = 1; this.y = 1; 0"), Ok(JsValue::Number(0.0)));
    assert_eq!(realm.run("var x; 0"), Ok(JsValue::Number(0.0)));
    let again = realm.run("let x = 2;");
    assert!(is_thrown(&again, "SyntaxError"), "{again:?}");
    // A property no `var` declared still takes a `let`.
    assert_eq!(realm.run("let y = 2; y"), Ok(JsValue::Number(2.0)));
    // A variable eval code declared can be deleted, and then its name takes a `let`.
    assert_eq!(realm.run("(0, eval)('var z = 1'); 0"), Ok(JsValue::Number(0.0)));
    assert!(is_thrown(&realm.run("let z;"), "SyntaxError"));
    assert_eq!(realm.run("delete z"), Ok(JsValue::Bool(true)));
    assert_eq!(realm.run("let z = 3; z"), Ok(JsValue::Number(3.0)));
    assert_eq!(realm.run("(0, eval)('var w = 1'); delete this.w"), Ok(JsValue::Bool(true)));
    assert_eq!(realm.run("let w = 4; w"), Ok(JsValue::Number(4.0)));
}

#[test]
fn a_global_var_that_cannot_be_declared_declares_nothing() {
    let mut realm = es();
    assert_eq!(realm.run("var kept = 1; Object.preventExtensions(this); 0"), Ok(JsValue::Number(0.0)));
    for decl in ["let x = 1; var y;", "(0, eval)('var y;');", "(0, eval)('let w; var y;');"] {
        let failed = realm.run(decl);
        assert!(is_thrown(&failed, "TypeError"), "{decl}: {failed:?}");
    }
    assert_eq!(realm.run("this.hasOwnProperty('y')"), Ok(JsValue::Bool(false)));
    // `x` was not declared, and a `var` the global object already has stays allowed.
    assert_eq!(realm.run("let x = 2; var kept; x + kept"), Ok(JsValue::Number(3.0)));
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
fn template_objects_go_once_their_code_cannot_run() {
    // Each eval makes code with two template sites; once it has run, nothing can run that code.
    let mut realm = es();
    let src = "function tag(s) { return s; } for (var i = 0; i < 50; i++) eval('tag`a`; tag`b`;'); 0";
    assert_eq!(realm.run(src), Ok(JsValue::Number(0.0)));
    realm.collect_garbage();
    assert_eq!(realm.interp().template_objects.len(), 0);
    // While the code can run, both of its sites keep their objects.
    assert_eq!(realm.run("var sites = function () { return [tag`a`, tag`b`]; }; var first = sites(); 0"), Ok(JsValue::Number(0.0)));
    realm.collect_garbage();
    assert_eq!(realm.interp().template_objects.len(), 2);
    assert_eq!(realm.run("var again = sites(); again[0] === first[0] && again[1] === first[1]"), Ok(JsValue::Bool(true)));
}

#[test]
fn a_preprocessor_line_does_not_start_after_a_template_that_spans_lines() {
    let options = Options { es2015: true, preprocessor_lines: true, ..Options::default() };
    assert!(crate::parse("var t = `${1}\n` #strict on", &options).is_err());
    let p = crate::parse("var t = `${1}\n`\n#strict on", &options);
    assert!(p.as_ref().is_ok_and(|p| p.directives.len() == 1), "{p:?}");
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
fn spread_expands_what_an_iterator_gives() {
    truth("var a = [1, 2]; var b = [0, ...a, 3]; b.join() === '0,1,2,3' && b.length === 4");
    truth("var b = [...'a\\uD83D\\uDE00b']; b.length === 3 && b[1] === '\\uD83D\\uDE00'");
    truth("function f() { return [...arguments]; } f(1, 2).join() === '1,2'");
    // Any object with an @@iterator method spreads, in the order its iterator gives.
    truth(
        "var n = 0; var it = { [Symbol.iterator]: function () { return { next: function () { n++; return { value: n, done: n > 3 }; } }; } }; \
         var b = [...it]; b.join() === '1,2,3' && n === 4",
    );
    truth("var a = [1, 2]; a[Symbol.iterator] = function () { return [9][Symbol.iterator](); }; [...a].join() === '9'");
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
fn spread_reads_a_string_object_by_code_point() {
    truth("var b = [...new String('a\\uD83D\\uDE00')]; b.length === 2 && b[0] === 'a' && b[1] === '\\uD83D\\uDE00'");
    truth("function f() { return arguments.length; } f(...new String('\\uD83D\\uDE00\\uD83D\\uDE00')) === 2");
}

#[test]
fn spread_reads_an_array_s_length_again_before_each_element() {
    let shrink = "var a = [1, 2]; Object.defineProperty(a, 0, { get: function () { a.length = 1; return 'v'; } }); \
                  var b = [...a]; b.length === 1 && b[0] === 'v'";
    truth(shrink);
    let grow = "var a = [1]; Object.defineProperty(a, 0, { get: function () { a.push(2); return 'v'; } }); \
                var b = [...a]; b.join() === 'v,2'";
    truth(grow);
    // An object without an iterator is not spread by its length.
    throws("var o = { length: 2, 0: 'v', 1: 'w' }; [...o];", "TypeError");
}

#[test]
fn an_array_s_spread_is_checked_against_the_elements_it_reads() {
    // The array starts at the longest length, and its first element's getter shrinks it, so
    // only one element is read and the result fits.
    let shrink = "var a = []; a.length = 4294967295; Object.defineProperty(a, 0, { get: function () { a.length = 1; return 7; } }); \
                  var b = [...a]; b.length === 1 && b[0] === 7";
    truth(shrink);
    let then_more = "var a = []; a.length = 4294967295; Object.defineProperty(a, 0, { get: function () { a.length = 1; return 7; } }); \
                     var b = [...a, ...[1, 2]]; b.join() === '7,1,2'";
    truth(then_more);
    // An array's iterator steps through every index, so a long array ends at the fuel limit.
    let mut realm = Realm::with_options(es_options(), Limits { fuel: 10_000, ..Limits::default() });
    let long = realm.run("var a = []; a.length = 4294967295; [...a]");
    assert!(matches!(&long, Err(e) if e.kind == RunErrorKind::Limit(LimitKind::Fuel)), "{long:?}");
    // An object without an iterator is not spread by its length.
    throws("[0, ...{ length: 4294967295 }];", "TypeError");
}

#[test]
fn spread_counts_each_index_name_against_the_heap() {
    // The elements' properties, with their index names and a string's characters, fit in the
    // heap or the run ends with the heap limit, at every limit in the range.
    for spread in ["[...a]", "[...s]"] {
        let setup = "var a = Array(4000); var s = new Array(4001).join('x'); 0";
        let mut measure = Realm::with_options(es_options(), Limits::default());
        assert_eq!(measure.run(setup), Ok(JsValue::Number(0.0)));
        measure.interp().collect_garbage();
        let base = measure.interp().heap.bytes;
        for extra in (0..640usize).map(|k| k * 1024) {
            let heap_bytes = base + extra;
            let mut realm = Realm::with_options(es_options(), Limits { heap_bytes, ..Limits::default() });
            if realm.run(setup).is_err() {
                continue;
            }
            match realm.run(&format!("var b = {spread}; b.length")) {
                Ok(v) => assert_eq!(v, JsValue::Number(4000.0), "{spread} at {heap_bytes}"),
                Err(e) => assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{spread} at {heap_bytes}: {e:?}"),
            }
            // A collection counts what is live, without the bytes the spread held while it ran.
            realm.interp().collect_garbage();
            assert!(realm.interp().heap.bytes <= heap_bytes, "{spread}: {} > {heap_bytes}", realm.interp().heap.bytes);
        }
    }
}

#[test]
fn spreading_other_values_is_a_type_error() {
    throws("[...1];", "TypeError");
    throws("[...null];", "TypeError");
    throws("[...undefined];", "TypeError");
    throws("[...{}];", "TypeError");
    // An object with a length is not iterable for that.
    throws("[...{ length: 2, 0: 'x', 1: 'y' }];", "TypeError");
    throws("[...{ [Symbol.iterator]: function () { return 1; } }];", "TypeError");
    throws("[...{ [Symbol.iterator]: function () { return { next: function () { return 1; } }; } }];", "TypeError");
    throws("Math.max(...true);", "TypeError");
    throws("new Array(...{ a: 1 });", "TypeError");
}

/// Runs `setup`, which gives `true`, and collects its garbage, then `call` once to measure the
/// bytes it adds. Then, for every room from none to one byte less than that, in steps of 8
/// bytes, it runs `call` in a new realm with that room: the run ends with the heap limit or a
/// collection makes room, and the heap does not pass the limit either way.
fn stays_within_every_limit(setup: &str, call: &str) {
    let ready = || {
        let mut realm = es();
        assert_eq!(realm.run(setup), Ok(JsValue::Bool(true)), "{setup}");
        realm.collect_garbage();
        realm
    };
    let mut realm = ready();
    let before = realm.interp().heap.bytes;
    assert!(realm.run(call).is_ok(), "{call}");
    let added = realm.interp().heap.bytes.saturating_sub(before);
    for room in (0..added).rev().step_by(8) {
        let mut realm = ready();
        let limit = realm.interp().heap.bytes + room;
        realm.set_limits(Limits { heap_bytes: limit, ..Limits::default() });
        let r = realm.run(call);
        let used = realm.interp().heap.bytes;
        assert!(r.is_ok() || matches!(&r, Err(e) if e.kind == RunErrorKind::Limit(LimitKind::Heap)), "{call} with {room}: {r:?}");
        assert!(used <= limit, "{call} with {room}: {used} of {limit}");
    }
}

#[test]
fn iterator_results_and_template_objects_stay_within_the_heap_limit() {
    stays_within_every_limit("var it = [1, 2][Symbol.iterator](), r; true", "r = it.next(); true");
    stays_within_every_limit("var it = [][Symbol.iterator](), r; true", "r = it.next(); true");
    let parts = (0..60).map(|i| format!("part{i}${{{i}}}")).collect::<String>();
    stays_within_every_limit(&format!("var r; function tag(s) {{ return s; }} function go() {{ r = tag`{parts}end`; return true; }} true"), "go()");
}

#[test]
fn declarations_stay_within_the_heap_limit() {
    let lets = (0..40).map(|i| format!("let a_long_global_lexical_name_{i} = {i};")).collect::<String>();
    stays_within_every_limit("true", &format!("{lets} const fixed = 1; true"));
    let body = (0..30).map(|i| format!("let a_long_body_lexical_name_{i} = {i}; function a_long_body_function_name_{i}() {{}}")).collect::<String>();
    stays_within_every_limit(&format!("function f(a = 1) {{ {body} return true; }} true"), "f()");
}

#[test]
fn code_stopped_by_the_frame_limit_declares_nothing() {
    let mut realm = es();
    realm.set_limits(Limits { frames: 0, ..Limits::default() });
    let e = realm.run("let kept = 1; var also = 2;").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Frames), "{e:?}");
    realm.set_limits(Limits { frames: 1, ..Limits::default() });
    let e = realm.run("eval('var leaked = 1; let inner = 2;')").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Frames), "{e:?}");
    realm.set_limits(Limits::default());
    assert_eq!(realm.run("let kept = 3; typeof also === 'undefined' && typeof leaked === 'undefined' && kept === 3"), Ok(JsValue::Bool(true)));
}

#[test]
fn spread_and_templates_stay_within_the_limits() {
    let mut realm = Realm::with_options(es_options(), Limits { fuel: 10_000, ..Limits::default() });
    let endless = "{ [Symbol.iterator]: function () { return { next: function () { return { value: 1, done: false }; } }; } }";
    let e = realm.run(&format!("[...{endless}]")).unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Fuel), "{e:?}");
    let mut realm = Realm::with_options(es_options(), Limits { fuel: 10_000, ..Limits::default() });
    let e = realm.run("var a = []; a.length = 4294967294; [0, ...a];").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Fuel), "{e:?}");
    let mut realm = Realm::with_options(es_options(), Limits { heap_bytes: 4 << 20, ..Limits::default() });
    let e = realm.run("var s = 'x'; for (var i = 0; i < 22; i++) s += s; [...s]").unwrap_err();
    assert!(matches!(e.kind, RunErrorKind::Limit(_)), "{e:?}");
    throws("function f() {} var a = []; a.length = 100000; f(...a);", "RangeError");
}

#[test]
fn spreading_a_rope_counts_its_flattened_length_first() {
    // `mk` returns a rope whose left half is a flat string of about half the heap, held by
    // nothing else. Its flattened copy is the other half, so the two don't fit together. Making
    // the left half takes about 16,400 fuel, and the elements read before the heap fills would
    // take more than the rest, so only the check before the copy ends the run with the heap limit.
    let build = "var s = 'x'; for (var i = 0; i < 18; i++) s = s + s; function f(c) { return s + c; } \
                 function mk() { var a = f('a'); a[0]; return a + 'b'; }";
    let limits = Limits { heap_bytes: 4 * 262_146, fuel: 20_000, ..Limits::default() };
    for spread in ["[...mk()]", "Math.max(...mk())", "var [c] = mk()", "for (var c of mk()) break"] {
        let e = Realm::with_options(es_options(), limits.clone()).run(&format!("{build} {spread}; 0")).unwrap_err();
        assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{spread}: {e:?}");
        // A realm without `Symbol.iterator` walks the string, and flattens it the same way.
        let mut walking = Realm::with_options(Options::default(), limits.clone());
        walking.set_es2015(true);
        let e = walking.run(&format!("{build} {spread}; 0")).unwrap_err();
        assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "walking {spread}: {e:?}");
    }
    let made = Realm::with_options(es_options(), limits).run(&format!("{build} mk(); 0"));
    assert_eq!(made, Ok(JsValue::Number(0.0)), "the rope is made within the fuel");
}

#[test]
fn a_rest_parameter_checks_the_heap_before_its_array_is_made() {
    // The 20,000 arguments fit in the room left over, and the rest array of them does not, so
    // the call ends with the heap limit before the heap estimate passes it.
    let setup = "function f(...r) { return r.length; } var big = Array(20000); 0";
    let mut measure = Realm::with_options(es_options(), Limits::default());
    assert_eq!(measure.run(setup), Ok(JsValue::Number(0.0)));
    measure.interp().collect_garbage();
    let heap_bytes = measure.interp().heap.bytes + (512 << 10);
    let mut realm = Realm::with_options(es_options(), Limits { heap_bytes, ..Limits::default() });
    assert_eq!(realm.run(setup), Ok(JsValue::Number(0.0)));
    assert_eq!(realm.run("f.apply(null, Array(10))"), Ok(JsValue::Number(10.0)));
    let e = realm.run("f.apply(null, big)").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{e:?}");
    assert!(realm.interp().heap.bytes <= heap_bytes, "{} > {heap_bytes}", realm.interp().heap.bytes);
}

#[test]
fn a_body_after_default_parameters_checks_the_heap_before_its_scope_is_made() {
    // The default value fills the heap a little more each run, so at some count the room left
    // after it is smaller than the body's long variable names need. The call then ends with the
    // heap limit before the heap estimate passes it.
    let vars = (0..200).map(|i| format!("v{i:0>60}")).collect::<Vec<_>>().join(", ");
    let setup = format!(
        "var keep = []; function fill(k) {{ for (var i = 0; i < k; i++) keep.push(i); }} function f(a = fill(count)) {{ var {vars}; return 1; }} 0"
    );
    let mut measure = Realm::with_options(es_options(), Limits::default());
    assert_eq!(measure.run(&setup), Ok(JsValue::Number(0.0)));
    measure.interp().collect_garbage();
    let heap_bytes = measure.interp().heap.bytes + (256 << 10);
    let mut stopped = 0;
    for count in (0..4000).step_by(20) {
        let mut realm = Realm::with_options(es_options(), Limits { heap_bytes, ..Limits::default() });
        assert_eq!(realm.run(&setup), Ok(JsValue::Number(0.0)));
        match realm.run(&format!("var count = {count}; f()")) {
            Ok(v) => assert_eq!(v, JsValue::Number(1.0), "{count}"),
            Err(e) => {
                assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{count}: {e:?}");
                stopped += 1;
            }
        }
        assert!(realm.interp().heap.bytes <= heap_bytes, "{count}: {} > {heap_bytes}", realm.interp().heap.bytes);
    }
    assert!(stopped > 0, "no count reached the heap limit");
}

#[test]
fn new_scopes_survive_collections_while_they_are_made() {
    // Heaps just big enough make the collector run while function bodies and blocks set up their
    // scopes; each run gives the right total or stops at the heap limit.
    let src = "var total = 0; \
        function f(a = 1, ...r) { let x = a + r.length; const y = 1; function g() { return x + y; } function h() { return g(); } var junk = [a, a, a]; return h(); } \
        function p(c) { let x = c; function q() { return x + arguments.length; } return q(); } \
        for (var i = 0; i < 300; i++) { { let b = i; function k() { return b; } function m() { return k(); } total += f(1, 2) + m() - i + p(1) - 1; } } total";
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

/// Runs `src` with a collection at every heap reservation, and checks that it gives `true`.
fn truth_collecting(src: &str) {
    let mut realm = es();
    crate::interp::COLLECT_ON_RESERVE.with(|c| c.set(true));
    let r = realm.run(src);
    crate::interp::COLLECT_ON_RESERVE.with(|c| c.set(false));
    assert_eq!(r, Ok(JsValue::Bool(true)), "{src}");
    assert!(realm.interp().heap.collections > 0, "{src}");
}

#[test]
fn values_a_call_enters_with_stay_reachable_while_its_scopes_are_made() {
    truth_collecting("function C() { let x = 1; this.x = x; } var o = new C(); o.x === 1 && o instanceof C");
    truth_collecting("function f(a) { let x = 1; return arguments[1].v; } f(0, { v: 3 }) === 3");
    truth_collecting("function g() { 'use strict'; let x = 1; return this.v + x; } g.call({ v: 4 }) === 5");
}

#[test]
fn strict_eval_environments_stay_reachable_while_their_scopes_are_made() {
    truth_collecting("(function () { 'use strict'; var v = 5; return eval('let x = 1; var y = 2; x + y + v'); })() === 8");
    truth_collecting("(0, eval)(\"'use strict'; let x = 1; var y = 2; x + y\") === 3");
}

#[test]
fn new_scopes_survive_a_collection_at_each_reservation() {
    // The sweep below makes collections fall where the heap size puts them; this one runs one at
    // every reservation, so every scope is made across a collection.
    truth_collecting(
        "var total = 0; \
         function f(a = 1, ...r) { let x = a + r.length; const y = 1; function g() { return x + y; } function h() { return g(); } return h(); } \
         for (var i = 0; i < 20; i++) { { let b = i; function k() { return b; } total += f(1, 2) + k() - i; } } total === 60",
    );
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

#[test]
fn for_each_takes_an_assignment_pattern_with_e4x() {
    let mut realm = Realm::with_options(Options { e4x: true, ..es_options() }, Limits::default());
    let src = "var x, y; for each ([x] in { a: [7] }); for each ({ p: y } in [{ p: 8 }]); x + y";
    assert_eq!(realm.run(src), Ok(JsValue::Number(15.0)));
    let legacy = Realm::with_options(Options { e4x: true, ..Options::default() }, Limits::default()).run("var x; for each ([x] in {});");
    assert!(matches!(&legacy, Err(e) if e.kind == RunErrorKind::Syntax), "{legacy:?}");
}

// ---- for-of and destructuring ----

/// An iterable whose iterator gives 1 to `n` and counts the calls to its `return` method in
/// `closed`.
const COUNTED: &str = "var closed = 0; function it(n) { var i = 0; return { [Symbol.iterator]: function () { return this; }, \
     next: function () { i++; return { value: i, done: i > n }; }, return: function () { closed++; return {}; } }; } ";

fn counted(src: &str) {
    truth(&format!("{COUNTED}{src}"));
}

#[test]
fn for_of_runs_the_body_for_each_value() {
    truth("var s = ''; for (var x of ['a', 'b']) s += x; s === 'ab' && x === 'b'");
    truth("var s = []; for (var c of 'a\\uD83D\\uDE00') s.push(c); s.length === 2 && s[1] === '\\uD83D\\uDE00'");
    truth("var n = 0; for (var x of []) n++; n === 0");
    truth("var o = {}; for (o.p of [1, 2]); o.p === 2");
    truth("var r = []; for (var [a, b] of [[1, 2], [3, 4]]) r.push(a + b); r.join() === '3,7'");
    truth("var a, b; for ([a, { b }] of [[1, { b: 2 }]]); a === 1 && b === 2");
    truth("var r = []; outer: for (var i = 0; i < 2; i++) { for (var x of [1, 2]) { if (x === 2) continue outer; r.push(i); } } r.join() === '0,1'");
    truth("var s = 0; for (var x of [1, 2, 3]) { if (x === 2) continue; s += x; } s === 4");
    truth("eval('1; for (var x of []) {}') === undefined && eval('1; for (var x of [2]) { break; }') === undefined");
    truth("eval('1; for (var x of [2, 3]) { x; }') === 3");
}

#[test]
fn loops_whose_body_never_completes_give_undefined() {
    for src in [
        "42; for (let i = 0; i < 0; i++) {}",
        "42; for (const k in {}) {}",
        "42; for (let k in {}) {}",
        "42; for (var i = 0; i < 0; i++) {}",
        "42; for (var k in {}) {}",
        "42; while (false) {}",
        "42; do { break; } while (true)",
    ] {
        assert_eq!(es().run(src), Ok(JsValue::Undefined), "{src}");
    }
    assert_eq!(es().run("42; for (let i = 0; i < 2; i++) { i; }"), Ok(JsValue::Number(1.0)));
    assert_eq!(es().run("42; for (let k in { a: 1 }) { k; }"), Ok(JsValue::String("a".into())));
    // ECMAScript 5.1 code keeps the value before the loop.
    assert_eq!(Realm::new().run("42; for (var i = 0; i < 0; i++) {}"), Ok(JsValue::Number(42.0)));
    assert_eq!(Realm::new().run("42; while (false) {}"), Ok(JsValue::Number(42.0)));
}

#[test]
fn if_switch_with_and_try_without_a_value_give_undefined() {
    for src in [
        "for (let i = 0; i < 2; i++) { if (i === 0) 7; }",
        "42; if (false) 7;",
        "42; if (true) {}",
        "42; switch (1) { case 2: 7; }",
        "42; with ({}) {}",
        "42; try {} finally {}",
        "42; try { 7; throw 1; } catch (e) {}",
        "42; do { 7; if (true) break; } while (false)",
    ] {
        assert_eq!(es().run(src), Ok(JsValue::Undefined), "{src}");
        assert_eq!(es().run(&format!("eval({src:?})")), Ok(JsValue::Undefined), "{src}");
    }
    assert_eq!(es().run("42; ;"), Ok(JsValue::Number(42.0)));
    assert_eq!(es().run("42; if (true) 7;"), Ok(JsValue::Number(7.0)));
    assert_eq!(es().run("42; try { 7; } catch (e) {}"), Ok(JsValue::Number(7.0)));
    // ECMAScript 5.1 code keeps the earlier value.
    assert_eq!(Realm::new().run("42; if (false) 7;"), Ok(JsValue::Number(42.0)));
    assert_eq!(Realm::new().run("42; try { 7; throw 1; } catch (e) {}"), Ok(JsValue::Number(7.0)));
}

#[test]
fn for_of_let_and_const_get_a_binding_for_each_iteration() {
    truth("var fs = []; for (let x of [1, 2]) fs.push(function () { return x; }); fs[0]() === 1 && fs[1]() === 2");
    truth("var fs = []; for (const [k, v] of [[1, 2], [3, 4]]) fs.push(() => k + v); fs[0]() === 3 && fs[1]() === 7");
    truth("for (let x of [1]) {} typeof x === 'undefined'");
    throws("for (const x of [1]) x = 2;", "TypeError");
    // The iterable is read where the head's names are not yet initialized.
    throws("let x = [1]; for (let x of x) {}", "ReferenceError");
}

#[test]
fn for_of_closes_the_iterator_when_the_loop_is_left_early() {
    counted("for (var x of it(5)) break; closed === 1 && x === 1");
    counted("function f() { for (var x of it(5)) return x * 10; } f() === 10 && closed === 1");
    counted("try { for (var x of it(5)) throw 1; } catch (e) {} closed === 1");
    counted("outer: for (var i = 0; i < 2; i++) for (var x of it(5)) continue outer; closed === 2");
    counted("for (var x of it(5)) for (var y of it(5)) break; closed === 5");
    counted("try { for (var x of it(5)) { try { break; } finally { closed += 10; } } } finally {} closed === 11");
    counted(
        "var t = it(5); t.next = function () { return { value: [], done: false }; }; \
         var r; try { for (var [a = (function () { throw 7; })()] of t); } catch (e) { r = e; } r === 7 && closed === 1",
    );
    // An iterator that finishes, or whose own next method throws, is not closed.
    counted("for (var x of it(2)); closed === 0");
    counted("var t = it(5); t.next = function () { throw 3; }; try { for (var x of t); } catch (e) {} closed === 0");
    // Nor is one whose result's `value` getter throws (sections 13.7.5.13 and 13.3.3.6).
    let bad_value = "var t = it(5); t.next = function () { return { done: false, get value() { throw 4; } }; };";
    counted(&format!("{bad_value} var r; try {{ for (var x of t); }} catch (e) {{ r = e; }} r === 4 && closed === 0"));
    counted(&format!("{bad_value} var r; try {{ var [a] = t; }} catch (e) {{ r = e; }} r === 4 && closed === 0"));
}

#[test]
fn closing_an_iterator_reports_a_bad_return_unless_an_exception_is_on_its_way() {
    let bad = "var bad = { [Symbol.iterator]: function () { return this; }, next: function () { return { value: 1, done: false }; }, \
               return: function () { return 1; } }; ";
    throws(&format!("{bad} for (var x of bad) break;"), "TypeError");
    truth(&format!("{bad} var r; try {{ for (var x of bad) throw new RangeError('x'); }} catch (e) {{ r = e; }} r instanceof RangeError"));
    let throwing = "var t = { [Symbol.iterator]: function () { return this; }, next: function () { return { value: 1, done: false }; }, \
                    return: function () { throw new SyntaxError('r'); } }; ";
    throws(&format!("{throwing} for (var x of t) break;"), "SyntaxError");
    truth(&format!("{throwing} var r; try {{ for (var x of t) throw 5; }} catch (e) {{ r = e; }} r === 5"));
    // A close that throws when `return` or a jump to an outer statement leaves the loop reaches
    // the enclosing catch clause. Inner iterators are closed first, and a throwing inner close
    // still closes the outer iterators.
    let caught = |src: &str| format!("{COUNTED}{throwing} var r; try {{ {src} }} catch (e) {{ r = e; }} r instanceof SyntaxError");
    for src in [
        "(function () { for (var x of t) return 1; })();",
        "(function () { for (var x of [1]) for (var y of t) return 1; })();",
        "outer: for (var i = 0; i < 2; i++) for (var x of t) continue outer;",
        "outer: for (var x of t) for (var y of [1, 2]) break outer;",
        "outer: for (var x of t) for (var y of it(5)) for (var z of it(5)) break outer;",
    ] {
        truth(&format!("{} && closed === {}", caught(src), src.matches("it(5)").count()));
    }
    truth(&format!("{} && closed === 1", caught("outer: for (var x of it(5)) for (var y of t) break outer;")));
    truth(&format!("{} && closed === 1", caught("(function () { for (var x of it(5)) for (var y of t) return 1; })();")));
}

#[test]
fn an_exception_that_closes_an_iterator_keeps_where_it_was_thrown() {
    let src = "var t = { [Symbol.iterator]: function () { return this; }, next: function () { return { value: 1, done: false }; }, \
               return: function () { return {}; } };\nfor (var x of t) {\n  if (x) throw 1;\n  x = String(x);\n}";
    // A collection runs at each reservation, so one runs while `return` is called.
    let mut realm = es();
    crate::interp::COLLECT_ON_RESERVE.with(|c| c.set(true));
    let r = realm.run(src);
    crate::interp::COLLECT_ON_RESERVE.with(|c| c.set(false));
    let e = r.unwrap_err();
    assert_eq!((&e.kind, e.line, e.column), (&RunErrorKind::Thrown, 3, 10), "{e:?}");
    // A `return` getter that throws does not replace the exception (ECMA-262 2023 section 7.4.8).
    truth(
        "var t = { [Symbol.iterator]: function () { return this; }, next: function () { return { value: 1, done: false }; }, \
         get return() { throw 2; } }; var r; try { for (var x of t) throw 1; } catch (e) { r = e; } r === 1",
    );
}

#[test]
fn array_patterns_step_the_iterator_and_close_it() {
    truth("var [a, , b = 3, ...c] = [1, 2, undefined, 4, 5]; a === 1 && b === 3 && c.join() === '4,5'");
    truth("var [a, b] = 'xy'; a + b === 'xy'");
    truth("var [a, b] = [1]; a === 1 && b === undefined");
    truth("let [[a], [b]] = [[1], [2]]; a + b === 3");
    truth("var [...[a, b]] = [1, 2]; a + b === 3");
    counted("var [a] = it(5); closed === 1 && a === 1");
    counted("var [a, ...r] = it(2); closed === 0 && r.join() === '2'");
    counted("var [] = it(5); closed === 1");
    counted("var [a, b] = it(1); closed === 0 && b === undefined");
    // A hole steps the iterator without reading the result's `value`.
    let bad_value = "var t = it(5); t.next = function () { return { done: false, get value() { throw 4; } }; };";
    counted(&format!("{bad_value} var [,] = t; closed === 1"));
    counted(&format!("{bad_value} [,] = t; for (let [,] of [t]); closed === 2"));
    counted("try { var [a = (function () { throw 1; })()] = it(0); } catch (e) {} closed === 0");
    counted("try { var [a, b = (function () { throw 1; })()] = [1]; } catch (e) {} closed === 0");
    counted("var o = { set p(v) { throw 2; } }; try { [o.p] = it(5); } catch (e) {} closed === 1");
    throws("var [a] = {};", "TypeError");
    throws("var [a] = 1;", "TypeError");
}

#[test]
fn object_patterns_read_properties_by_name() {
    truth("let { x, y: z = 2, ['w' + 1]: w } = { x: 1, w1: 4 }; x === 1 && z === 2 && w === 4");
    truth("const [{ a: [b] }] = [{ a: [7] }]; b === 7");
    truth("var { length } = 'abc'; length === 3");
    truth("var s = Symbol(); var { [s]: v } = { [s]: 9 }; v === 9");
    truth("var { 0: a, 'b c': b } = { 0: 1, 'b c': 2 }; a + b === 3");
    truth(
        "var order = []; var o = { get a() { order.push('a'); return 1; }, get b() { order.push('b'); return 2; } }; \
           var { b, a } = o; order.join() === 'b,a'",
    );
    // A `var` pattern's names are resolved before their values are read (sections 13.3.3.6
    // and 13.3.3.7).
    truth(
        "var o = { x: 0 }; var t = { [Symbol.iterator]: function () { return this; }, \
         next: function () { delete o.x; return { value: 1, done: false }; }, return: function () { return {}; } }; \
         with (o) { var [x] = t; } o.x === 1 && x === undefined",
    );
    truth("var o = { y: 0 }; with (o) { var { a: y } = { get a() { delete o.y; return 2; } }; } o.y === 2 && y === undefined");
}

#[test]
fn destructuring_assignment_writes_to_any_assignment_target() {
    truth("var a = 1, b = 2; [a, b] = [b, a]; a === 2 && b === 1");
    truth("var a; var v = [1, 2]; var r = ([a] = v); r === v && a === 1");
    truth("var o = {}; [o.a, o['b']] = [1, 2]; ({ c: o.c } = { c: 3 }); o.a + o.b + o.c === 6");
    truth("var a, b; ({ a, b = 5 } = { a: 1 }); a === 1 && b === 5");
    truth("var x, y; [x = 1, [y] = [2]] = []; x === 1 && y === 2");
    truth("var r = []; var a; [a = r.push('default')] = [undefined]; a === 1 && r.length === 1");
}

#[test]
fn destructured_parameters_and_catch_clauses() {
    truth("function f([a, b], { c } = { c: 3 }) { return a + b + c; } f([1, 2]) === 6");
    truth("(([a], { b }) => a + b)([1], { b: 2 }) === 3");
    truth("function f(...[a, b]) { return a + b; } f(1, 2) === 3");
    truth("function f(a, [b], c = 1) {} f.length === 2");
    truth("function f({ a }, b) {} f.length === 2");
    truth("function f([a], b = a) { return b; } f([4]) === 4");
    truth("function f({ a }) { var a; return a; } f({ a: 6 }) === 6");
    truth("var r; try { throw { m: 'x', d: [1] }; } catch ({ m, d: [n] }) { r = m + n; } r === 'x1'");
    truth("var r; try { throw [1, 2]; } catch ([a, b]) { r = function () { return a + b; }; } r() === 3");
    // A function in a block is not also made a `var` when that `var` would clash with a
    // destructured catch parameter (ECMA-262 11th edition sections B.3.3 and B.3.5).
    truth("function g() { try { throw [1]; } catch ([f]) { { function f() {} } } return typeof f; } g() === 'undefined'");
    truth("try { throw {}; } catch ({ h }) { { function h() {} } } typeof h === 'undefined'");
    truth("function g() { try { throw 1; } catch (f) { { function f() {} } } return typeof f; } g() === 'function'");
    throws("function f({ a }) {} f();", "TypeError");
    throws("try { throw null; } catch ({ a }) {}", "TypeError");
    throws("function f([a], b = a) {} f([]); f(undefined);", "TypeError");
}

#[test]
fn defaults_run_only_for_undefined() {
    truth(
        "var n = 0; function d() { n++; return 5; } var [a = d()] = [null]; var { b = d() } = { b: 0 }; var [c = d()] = []; \
         var { e = d() } = { e: undefined }; a === null && b === 0 && c === 5 && e === 5 && n === 2",
    );
    truth("var n = 0; function f([a = n++] = [], { b = n++ } = {}) { return a + b; } f([3], { b: 4 }) === 7 && n === 0");
}

#[test]
fn destructuring_undefined_or_null_is_a_type_error() {
    throws("var { a } = null;", "TypeError");
    throws("var {} = undefined;", "TypeError");
    throws("var [a] = null;", "TypeError");
    throws("({ a } = undefined);", "TypeError");
    throws("for (var { a } of [null]);", "TypeError");
    throws("let { a: { b } } = { a: null };", "TypeError");
    truth("var { toFixed } = 1; typeof toFixed === 'function'");
}

#[test]
fn object_literals_take_shorthand_and_computed_names() {
    truth("var a = 1; var o = { a, ['b' + 1]: 2 }; o.a === 1 && o.b1 === 2");
    truth("var s = Symbol('k'); var o = { [s]: 3 }; o[s] === 3 && Object.keys(o).length === 0");
    truth("var o = { get ['x' + 1]() { return 4; }, set ['x' + 1](v) { this.y = v; } }; o.x1 = 5; o.x1 === 4 && o.y === 5");
    truth("var order = []; var o = { [(order.push(1), 'a')]: order.push(2) }; order.join() === '1,2' && o.a === 2");
}

#[test]
fn bad_patterns_are_syntax_errors() {
    for src in [
        "({ a = 1 });",
        "var o = { a = 1 };",
        "[...a, b] = [];",
        "[...a,] = [];",
        "var [...a, b] = [];",
        "({ get a() { return 1; } } = {});",
        "([a]) = [];",
        "[(a = 1)] = [];",
        "[a()] = [];",
        "[a + 1] = [];",
        "'use strict'; [eval] = [];",
        "'use strict'; var { arguments } = {};",
        "var [a];",
        "const { a };",
        "for (var x = 1 of []) ;",
        "for (let x = 1 of []) ;",
        "for (var [x] = 1 in {}) ;",
        "for (x of [], []) ;",
        "for (let.x of []) ;",
        "for (async of []) ;",
        "for (var x of []) let\n[a] = 0;",
        "if (1) let [a] = [];",
        "for (a() of []) ;",
        "function f([a]) { 'use strict'; }",
        "function f([a, a]) {}",
        "function f(a, [a]) {}",
        "([a, a]) => 1;",
        "try {} catch ([e, e]) {}",
        "try {} catch ([e]) { for (var e of []); }",
        "({ a: 1 } = {});",
        "[...[a] = []] = [];",
        "var a; [a = { b = 1 }] = [];",
        "var a; ({ a = { b = 1 } } = {});",
        "var a; ({ [{ b = 1 }]: a } = {});",
        "for ([a = { b = 1 }] of []) ;",
        "(a = { b = 1 }) => 0;",
        "([a = { b = 1 }]) => 0;",
    ] {
        syntax(src);
    }
    // A shorthand property with an initializer is a target only in the pattern that holds it.
    truth("var a, b; [{ b = 1 }] = [{}]; ({ a: { b = 2 } = {} } = {}); b === 2");
    truth("var f = ({ a = 1 }, [{ b = 2 }] = [{}]) => a + b; f({}) === 3");
    // `of` stays a name.
    truth("var of = [1]; var n = 0; for (var x of of) n += x; n === 1");
}

#[test]
fn large_patterns_take_their_shorthand_defaults() {
    let defaults = (0..20_000).map(|i| format!("a{i} = {i}")).collect::<Vec<_>>().join(", ");
    truth(&format!("({{ {defaults} }} = {{}}); a0 === 0 && a19999 === 19999"));
    truth(&format!("var f = ({{ {defaults} }}) => a19999; f({{}}) === 19999"));
    // A shorthand default inside a default value or a computed name is still an error.
    syntax(&format!("({{ {defaults}, b = {{ c = 1 }} }} = {{}});"));
    syntax(&format!("({{ [{{ c = 1 }}]: b, {defaults} }} = {{}});"));
    let small = "[{ b = 1 }] = [{}]; ".repeat(20_000);
    truth(&format!("var b; {small} b === 1"));
    syntax(&format!("var b; {small} ({{ c = 1 }});"));
}

#[test]
fn a_realm_without_the_later_built_ins_walks_arrays_arguments_and_strings() {
    let mut realm = Realm::new();
    realm.set_es2015(true);
    let src = "var s = ''; for (var x of [1, 2]) s += x; for (var c of 'a\\uD83D\\uDE00') s += c.length; \
               (function () { for (var a of arguments) s += a; })(3); var [p, ...q] = 'xyz'; \
               s === '1212' + '3' && [...[4, 5]].length === 2 && p === 'x' && q.join('') === 'yz'";
    assert_eq!(realm.run(src), Ok(JsValue::Bool(true)));
    let boxed = "var b = new String('ab'); b.toString = function () { return 'cd'; }; \
                 [...new String('ab')].join() === 'a,b' && [...b].join() === 'c,d'";
    assert_eq!(realm.run(boxed), Ok(JsValue::Bool(true)));
    let r = realm.run("for (var x of {}) {}");
    assert!(matches!(&r, Err(RunError { kind: RunErrorKind::Thrown, message, .. }) if message.starts_with("TypeError")), "{r:?}");
}

#[test]
fn for_of_and_patterns_stay_within_the_limits() {
    let endless = "{ [Symbol.iterator]: function () { return { next: function () { return { value: 1, done: false }; } }; } }";
    for src in [format!("for (var x of {endless});"), format!("var [...r] = {endless};")] {
        let mut realm = Realm::with_options(es_options(), Limits { fuel: 10_000, ..Limits::default() });
        let e = realm.run(&src).unwrap_err();
        assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Fuel), "{src}: {e:?}");
    }
    // Spread and a rest element reserve each new element's whole size, its index name included,
    // so the run ends at the heap limit without passing it, wherever the limit falls. A realm
    // without `Symbol.iterator` walks the array by index and makes no result objects.
    let walking = |limits: Limits| {
        let mut realm = Realm::with_options(Options::default(), limits);
        realm.set_es2015(true);
        realm
    };
    let setup = "var a = []; for (var i = 0; i < 2000; i++) a.push(i); 0";
    let mut measure = walking(Limits::default());
    assert_eq!(measure.run(setup), Ok(JsValue::Number(0.0)));
    measure.interp().collect_garbage();
    let base = measure.interp().heap.bytes + 100_000;
    let mut wrong = Vec::new();
    for src in ["var b = [...a]; 0", "var [...b] = a; 0"] {
        for heap_bytes in (base..base + 128).step_by(2) {
            let mut realm = walking(Limits { heap_bytes, ..Limits::default() });
            assert_eq!(realm.run(setup), Ok(JsValue::Number(0.0)));
            let r = realm.run(src).map_err(|e| e.kind);
            let bytes = realm.interp().heap.bytes;
            if r != Err(RunErrorKind::Limit(LimitKind::Heap)) || bytes > heap_bytes {
                wrong.push((src, heap_bytes, r, bytes));
            }
        }
    }
    assert!(wrong.is_empty(), "runs that ended past the heap limit: {wrong:?}");
    // Patterns nested past the nesting limit are a SyntaxError.
    for src in [format!("var {}a{} = 1;", "[".repeat(5000), "]".repeat(5000)), format!("({}a{} = 1);", "[".repeat(5000), "]".repeat(5000))] {
        let e = es().run(&src).unwrap_err();
        assert_eq!(e.kind, RunErrorKind::Syntax, "{e:?}");
    }
}

// ---- the language mode ----

#[test]
fn without_the_option_the_new_forms_are_syntax_errors() {
    for src in [
        "let x = 1; x",
        "const c = 1;",
        "var f = x => x;",
        "`t`",
        "function f(a = 1) {}",
        "function f(...r) {}",
        "f(...a)",
        "[...a]",
        "for (var x of []) {}",
        "var [a] = [1];",
        "var { a } = {};",
        "({ a } = {});",
        "function f([a]) {}",
        "try {} catch ({ a }) {}",
        "var o = { ['a']: 1 };",
    ] {
        let r = Realm::new().run(src);
        assert!(matches!(r, Err(RunError { kind: RunErrorKind::Syntax, .. })), "{src}: {r:?}");
        let extendscript = Options { e4x: true, preprocessor_lines: true, ..Options::default() };
        let r = Realm::with_options(extendscript, Limits::default()).run(src);
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

/// Starts `src` with the option `start`, turns it to `!start` once the run has paused, and gives
/// the run's value.
fn run_switching_language(src: &str, start: bool) -> Result<JsValue, RunError> {
    let mut realm = Realm::with_options(Options { es2015: start, ..Options::default() }, Limits::default());
    realm.start(src)?;
    assert_eq!(realm.advance(20)?, crate::Progress::Paused);
    realm.set_es2015(!start);
    loop {
        if let crate::Progress::Done(v) = realm.advance(1000)? {
            assert_eq!(realm.interp().options.es2015, !start, "the new language applies once the run ends");
            return Ok(v);
        }
    }
}

#[test]
fn a_run_keeps_its_language_for_the_code_it_evaluates() {
    let src = "for (var i = 0; i < 200; i++) {} ";
    assert_eq!(run_switching_language(&format!("{src} eval('let a = 1; a')"), true), Ok(JsValue::Number(1.0)));
    let legacy = run_switching_language(&format!("{src} var r; try {{ eval('let a = 1; a'); }} catch (e) {{ r = e.name; }} r"), false);
    assert_eq!(legacy, Ok(JsValue::String("SyntaxError".into())));
}

// ---- robustness ----

const FRAGMENTS: &[&str] = &[
    " of ",
    "...",
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
