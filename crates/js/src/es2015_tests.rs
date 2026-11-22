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
    // A labeled function in a block declares its name in the block.
    syntax("{ let f; label: function f() {} }");
    truth("function g() { var f; L: function f() {} return typeof f; } g() === 'function'");
    // A method's parameter names are unique outside strict mode code too.
    syntax("({ m(x, x) {} })");
    truth("function f(x, x) { return x; } f(1, 2) === 2");
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
    // Only a `let` or `const` around the declaration, a function declared in a block around it, or
    // a parameter keeps the name from being a variable.
    truth("var f; { function f() { return 1; } { function f() { return 2; } } } f() === 1");
    // A function standing alone as an `if` branch is in a block of its own.
    truth("var f; { function f() { return 1; } if (true) function f() { return 2; } } f() === 1");
    truth("var f; if (true) function f() { return 2; } else function f() { return 3; } f() === 2");
    // A labeled function in a block is made when the block is entered.
    truth("{ var result = typeof lf; L: function lf() {} } result === 'function'");
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
fn a_script_stopped_at_a_limit_before_it_runs_leaves_no_global_lexical_bindings() {
    // The 200 functions take a unit of fuel each after the `let` is declared, so each of these
    // fuel limits stops the script before it runs. A second run with the default limits then
    // declares `pending` again; a thrown error once the script runs keeps the binding.
    let functions: String = (0..200).map(|k| format!("function f{k}() {{}} ")).collect();
    let src = format!("let pending = 1; {functions} pending");
    for fuel in (2..150).step_by(7) {
        let mut realm = Realm::with_options(es_options(), Limits { fuel, ..Limits::default() });
        let e = realm.run(&src).unwrap_err();
        assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Fuel), "fuel {fuel}: {e:?}");
        realm.set_limits(Limits::default());
        assert_eq!(realm.run(&src), Ok(JsValue::Number(1.0)), "fuel {fuel}");
    }
    let mut realm = es();
    assert!(realm.run("let thrown = 1; throw 0;").is_err());
    let again = realm.run("let thrown = 2;");
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
fn a_global_variable_that_cannot_be_declared_declares_nothing() {
    // A global object that is not extensible takes no new variable (ECMA-262 2015 sections 15.1.8
    // and 18.2.1.2, CanDeclareGlobalVar), and nothing else the code declares is declared.
    let mut realm = es();
    assert_eq!(realm.run("var kept = 1; Object.preventExtensions(this); 0"), Ok(JsValue::Number(0.0)));
    for decl in ["let pending = 1; var missing;", "eval('var missing;')", "(0, eval)('var missing;')"] {
        let failed = realm.run(decl);
        assert!(
            matches!(&failed, Err(RunError { kind: RunErrorKind::Thrown, message, .. }) if message.starts_with("TypeError")),
            "{decl}: {failed:?}"
        );
    }
    assert_eq!(realm.run("typeof pending === 'undefined' && !this.hasOwnProperty('missing')"), Ok(JsValue::Bool(true)));
    assert_eq!(realm.run("var kept = 2; kept"), Ok(JsValue::Number(2.0)));
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
    // Without default values the parameters and the body share one environment, so a `var` of a
    // parameter's name, in the body or in direct eval, is the parameter.
    truth("function f(...r) { return eval('var r; r[0]'); } f(3) === 3");
    truth("function f(a, ...r) { eval('var a = 5; var r = 6'); return a + r; } f(1, 2) === 11");
    truth("function f(...r) { var r; return r[0]; } f(4) === 4");
    // The arguments object still does not follow the parameters, and its callee throws.
    truth("function f(a, ...r) { a = 9; return arguments[0]; } f(1) === 1");
    throws("function f(...r) { return arguments.callee; } f();", "TypeError");
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
    // A negative, NaN or fractional length is read as ToLength reads it.
    truth("[...{ length: -1, 0: 'x' }].length === 0 && [...{ length: NaN }].length === 0");
    truth("var b = [...{ length: 1.9, 0: 'x', 1: 'y' }]; b.join() === 'x'");
    truth("function f() { return arguments.length; } f(...{ length: -4294967295, 0: 1 }) === 0");
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
    // An object that is not an array keeps the length it had when spread began.
    truth("var o = { length: 2, get 0() { o.length = 1; return 'v'; }, 1: 'w' }; [...o].join() === 'v,w'");
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
    // An array no getter reads is checked when spread begins, without reading its elements.
    let mut realm = Realm::with_options(es_options(), Limits { fuel: 10_000, ..Limits::default() });
    let long = realm.run("var a = []; a.length = 4294967295; [...a]");
    assert!(is_thrown(&long, "RangeError"), "{long:?}");
    // An object that is not an array is still checked against the length it has when spread
    // begins.
    throws("[0, ...{ length: 4294967295 }];", "RangeError");
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
fn spreading_a_rope_counts_its_flattened_length_first() {
    // `mk` returns a rope whose left half is a flat string of about half the heap, held by
    // nothing else. Its flattened copy is the other half, so the two don't fit together. Making
    // the left half takes about 16,400 fuel, and the elements read before the heap fills would
    // take more than the rest, so only the check before the copy ends the run with the heap limit.
    let build = "var s = 'x'; for (var i = 0; i < 18; i++) s = s + s; function f(c) { return s + c; } \
                 function mk() { var a = f('a'); a[0]; return a + 'b'; }";
    let limits = Limits { heap_bytes: 4 * 262_146, fuel: 20_000, ..Limits::default() };
    for spread in ["[...mk()]", "Math.max(...mk())"] {
        let e = Realm::with_options(es_options(), limits.clone()).run(&format!("{build} {spread}; 0")).unwrap_err();
        assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{spread}: {e:?}");
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
fn calls_and_strict_eval_survive_collections_while_their_scopes_are_made() {
    // Heaps just big enough make the collector run while a call's or a strict eval's scopes are
    // made; the receiver, the arguments and eval's own environment stay reachable.
    let src = "var total = 0; \
        for (var i = 0; i < 300; i++) { \
        total += (function () { 'use strict'; let x = i; return this.marker + x - i; }).call({ marker: 1 }); \
        total += (function (a) { let y = a; return arguments.length + y.v; })({ v: 1 }, {}, {}); \
        total += eval(\"'use strict'; var x = 7; let y = 1; function g() { return x; } x + y + g() + i - i\"); \
        total += (0, eval)(\"'use strict'; var x = 2; let y = 1; function g() { return x; } x + y + g() + i - i\"); } total";
    let mut completed = 0;
    for kib in 16..400 {
        let mut realm = Realm::with_options(es_options(), Limits { heap_bytes: kib << 10, ..Limits::default() });
        match realm.run(src) {
            Ok(v) => {
                assert_eq!(v, JsValue::Number(7500.0), "{kib} KiB");
                completed += 1;
            }
            Err(e) => assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{kib} KiB: {e:?}"),
        }
    }
    assert!(completed > 0);
}

#[test]
fn strict_eval_keeps_its_environment_through_a_collection_while_its_scope_is_made() {
    // The scope of 500 names is the biggest piece eval code makes, so the collector runs while it
    // is made at many heap sizes; eval's own environment, with `x` in it, stays.
    let lets: Vec<String> = (0..500).map(|k| format!("l{k}")).collect();
    let src = format!(
        "var total = 0; var src = \"'use strict'; var x = 7; let {}; function g() {{ return x; }} x + g() + i - i\"; \
         for (var i = 0; i < 10; i++) {{ total += eval(src) + (0, eval)(src); }} total",
        lets.join(", ")
    );
    let mut completed = 0;
    for kib in (64..1024).step_by(2) {
        let mut realm = Realm::with_options(es_options(), Limits { heap_bytes: kib << 10, ..Limits::default() });
        match realm.run(&src) {
            Ok(v) => {
                assert_eq!(v, JsValue::Number(280.0), "{kib} KiB");
                completed += 1;
            }
            Err(e) => assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{kib} KiB: {e:?}"),
        }
    }
    assert!(completed > 0);
}

#[test]
fn spread_checks_the_heap_before_its_elements_are_added() {
    // The list of 25,000 elements read fits in the room left over, and the array's 25,000 new
    // properties do not, so the spread ends with the heap limit before the estimate passes it.
    let setup = "var big = { length: 25000 }; 0";
    let mut measure = Realm::with_options(es_options(), Limits::default());
    assert_eq!(measure.run(setup), Ok(JsValue::Number(0.0)));
    measure.interp().collect_garbage();
    let heap_bytes = measure.interp().heap.bytes + (2 << 20);
    let mut realm = Realm::with_options(es_options(), Limits { heap_bytes, ..Limits::default() });
    assert_eq!(realm.run(setup), Ok(JsValue::Number(0.0)));
    assert_eq!(realm.run("[...{ length: 10 }].length"), Ok(JsValue::Number(10.0)));
    let e = realm.run("[...big].length").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{e:?}");
    assert!(realm.interp().heap.bytes <= heap_bytes, "{} > {heap_bytes}", realm.interp().heap.bytes);
}

#[test]
fn spread_counts_its_list_of_elements_while_the_array_grows() {
    // The list of elements read (64 bytes each) fits in the room left over, and so do the array's
    // new properties (116 bytes each), but the two together do not. The list is still held while
    // the properties are added, so the spread ends with the heap limit.
    let cases = [
        ("var big = { length: 25000 }; 0", "[...big].length", 3584 << 10),
        ("var big = 'x'; for (var i = 0; i < 15; i++) big += big; big[0]; 0", "[...big].length", 4608 << 10),
    ];
    for (setup, spread, room) in cases {
        let mut measure = Realm::with_options(es_options(), Limits::default());
        assert_eq!(measure.run(setup), Ok(JsValue::Number(0.0)), "{setup}");
        measure.interp().collect_garbage();
        let heap_bytes = measure.interp().heap.bytes + room;
        let mut realm = Realm::with_options(es_options(), Limits { heap_bytes, ..Limits::default() });
        assert_eq!(realm.run(setup), Ok(JsValue::Number(0.0)), "{setup}");
        assert_eq!(realm.run("[...'abc', ...{ length: 10 }].length"), Ok(JsValue::Number(13.0)), "{setup}");
        let e = realm.run(spread).unwrap_err();
        assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{setup}: {e:?}");
        assert!(realm.interp().heap.bytes <= heap_bytes, "{setup}: {} > {heap_bytes}", realm.interp().heap.bytes);
    }
}

#[test]
fn template_objects_go_when_their_code_does() {
    // Two sites in one function share its code; both strings objects go once the function does.
    let mut realm = es();
    let src = "function tag(s) { return s; } (function () { tag`a`; tag`b`; })(); (function () { tag`c`; })(); 0";
    assert_eq!(realm.run(src), Ok(JsValue::Number(0.0)));
    realm.interp().collect_garbage();
    realm.interp().collect_garbage();
    assert_eq!(realm.interp().template_objects.len(), 0);
}

#[test]
fn a_directive_after_a_template_over_lines_must_start_its_line() {
    let options = Options { preprocessor_lines: true, ..es_options() };
    let p = crate::parse("var s = `${1}\nx` //@include \"missing.jsx\"\nvar t = `a\nb` //@include \"other.jsx\"", &options).unwrap();
    assert!(p.directives.is_empty(), "{:?}", p.directives);
    assert!(crate::parse("var s = `${1}\nx` #include x", &options).is_err());
    assert_eq!(crate::parse("var s = `${1}\nx`\n//@include \"a.jsx\"", &options).unwrap().directives.len(), 1);
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

// ---- classes ----

#[test]
fn classes_make_constructors_with_methods_on_their_prototype() {
    truth("class A { constructor(x) { this.x = x; } get() { return this.x; } } new A(3).get() === 3");
    truth(
        "class A {} typeof A === 'function' && Object.getPrototypeOf(A) === Function.prototype && Object.getPrototypeOf(A.prototype) === Object.prototype",
    );
    truth("class A { m() {} } var d = Object.getOwnPropertyDescriptor(A.prototype, 'm'); d.writable && !d.enumerable && d.configurable");
    truth("class A {} var d = Object.getOwnPropertyDescriptor(A, 'prototype'); !d.writable && !d.enumerable && !d.configurable");
    truth("class A {} var d = Object.getOwnPropertyDescriptor(A.prototype, 'constructor'); d.value === A && !d.enumerable");
    truth("class A { m() {} } !A.prototype.m.hasOwnProperty('prototype')");
    truth("var k = 'dyn'; class A { [k + 1]() { return 1; } } new A().dyn1() === 1");
    truth("var C = class Named { who() { return Named; } }; new C().who() === C && typeof Named === 'undefined'");
    truth("class A { ;; m() { return 2; }; } new A().m() === 2");
    assert!(string("class A { m() { return 1; } } String(A)").starts_with("class A"));
    // A class body is strict mode code.
    truth("class A { m() { return this; } } A.prototype.m.call(undefined) === undefined");
    throws("class A { m() { undeclared = 1; } } new A().m();", "ReferenceError");
}

#[test]
fn static_and_accessor_members_go_on_the_constructor_and_prototype() {
    truth("class A { static make() { return new A(); } } A.make() instanceof A");
    truth("class A { static get n() { return 4; } } A.n === 4 && !A.prototype.hasOwnProperty('n')");
    truth(
        "class A { constructor() { this._v = 1; } get v() { return this._v; } set v(x) { this._v = x * 2; } } var a = new A(); a.v = 5; a.v === 10",
    );
    truth(
        "class A { get v() { return 1; } set v(x) {} } var d = Object.getOwnPropertyDescriptor(A.prototype, 'v'); typeof d.get === 'function' && typeof d.set === 'function' && !d.enumerable",
    );
    truth(
        "class A { static() { return 's'; } get() { return 'g'; } set() { return 't'; } } var a = new A(); a.static() + a.get() + a.set() === 'sgt'",
    );
    throws("class A { static ['proto' + 'type']() {} }", "TypeError");
    syntax("class A { static prototype() {} }");
}

#[test]
fn subclasses_inherit_and_reach_the_superclass_through_super() {
    truth(
        "class A { constructor(x) { this.x = x; } m() { return 'A' + this.x; } } \
           class B extends A { constructor(x) { super(x + 1); } m() { return 'B' + super.m(); } } \
           var b = new B(1); b.m() === 'BA2' && b instanceof A && Object.getPrototypeOf(B) === A",
    );
    truth("class A { constructor(...a) { this.n = a.length; } } class B extends A {} new B(1, 2, 3).n === 3");
    truth("class A { static s() { return 1; } } class B extends A { static s() { return super.s() + 1; } } B.s() === 2");
    truth("class A {} class B extends A { m() { super.x = 5; return this.x; } } var b = new B(); b.m() === 5 && !A.prototype.hasOwnProperty('x')");
    truth(
        "class A { get v() { return this.w; } } class B extends A { constructor() { super(); this.w = 7; } get v() { return super.v + 1; } } new B().v === 8",
    );
    // `super.n` reads the prototype's property and writes the instance's.
    truth(
        "class A {} A.prototype.n = 1; class B extends A { m() { super.n += 2; var a = this.n; super['n']++; return a * 10 + this.n; } } new B().m() === 32 && A.prototype.n === 1",
    );
    truth("class A { constructor() { this.made = new.target; } } class B extends A {} new B().made === B && new A().made === A");
    truth("class A {} class B extends A { constructor() { var f = () => super(); f(); this.ok = true; } } new B().ok");
    truth("function F() { this.f = 1; } class B extends F {} new B().f === 1");
    truth("class E extends Error { constructor(m) { super(m); } } var e = new E('x'); e instanceof E && e instanceof Error && e.message === 'x'");
    truth("class L extends Array {} var l = new L(); l.push(1); l instanceof L && l.length === 1");
    throws("class A {} class B extends A { m() { delete super.x; } } new B().m();", "ReferenceError");
    // The constructor's home object is the prototype (section 14.5.14, step 12), so `super.x` in
    // it reads the superclass's prototype, not the superclass.
    truth(
        "class A { m() { return 'p'; } static m() { return 's'; } } \
           class B extends A { constructor() { super(); this.r = super.m(); } } new B().r === 'p'",
    );
}

#[test]
fn object_literal_methods_are_enumerable_and_reach_super() {
    truth("var o = { m(x) { return x + 1; } }; o.m(1) === 2 && Object.keys(o).join() === 'm'");
    truth("var o = { m() {} }; !o.m.hasOwnProperty('prototype')");
    throws("var o = { m() {} }; new o.m();", "TypeError");
    // `super` reads the home object's prototype, `Object.prototype`, as it is at the access.
    truth(
        "Object.prototype.hi = function () { return 'base ' + this.n; }; var o = { n: 1, hi() { return 'o ' + super.hi(); } }; o.hi() === 'o base 1'",
    );
    truth("Object.prototype.v = 1; var o = { v: 9, m() { return super.v; } }; var a = o.m(); Object.prototype.v = 2; a === 1 && o.m() === 2");
    truth("Object.prototype.v = 3; var o = { v: 9, get g() { return super.v; } }; o.g === 3");
    truth("Object.prototype.v = 4; var o = { v: 9, m() { return (() => super.v)(); } }; o.m() === 4");
    truth("var o = { get: 1, set: 2, get() { return 3; } }; o.get() === 3 && o.set === 2");
    // A `super` write that cannot be made does nothing outside strict mode code.
    truth("var o = { m() { super.x = 1; Object.freeze(o); super.y = 2; return 1; } }; o.m() === 1 && o.x === 1 && !('y' in o)");
    throws("var o = { m() { 'use strict'; Object.freeze(o); super.y = 2; } }; o.m();", "TypeError");
    // So does a realm's strict writes setting.
    let mut realm = es();
    realm.set_strict_writes(true);
    let failed = realm.run("var o = { m() { Object.freeze(o); super.y = 2; } }; o.m();");
    assert!(matches!(&failed, Err(RunError { kind: RunErrorKind::Thrown, message, .. }) if message.starts_with("TypeError")), "{failed:?}");
    syntax("var o = { m() { super(); } };");
    syntax("var o = { f: function () { return super.x; } };");
    let r = Realm::new().run("var o = { m() {} };");
    assert!(matches!(r, Err(RunError { kind: RunErrorKind::Syntax, .. })), "{r:?}");
}

#[test]
fn this_before_super_is_a_reference_error() {
    throws("class A {} class B extends A { constructor() { this.x = 1; super(); } } new B();", "ReferenceError");
    throws("class A {} class B extends A { constructor() { var f = () => this; f(); super(); } } new B();", "ReferenceError");
    throws("class A {} class B extends A { constructor() {} } new B();", "ReferenceError");
    throws("class A {} class B extends A { constructor() { super(); super(); } } new B();", "ReferenceError");
    throws("class A {} class B extends A { constructor() { return 1; } } new B();", "TypeError");
    truth("class A {} class B extends A { constructor() { return { o: 1 }; } } new B().o === 1");
    truth("class A {} class B extends A { constructor() { var f = () => this; super(); this.f = f; } } var b = new B(); b.f() === b");
    // A handler in the constructor does not catch what its return throws.
    throws("class A {} class B extends A { constructor() { try { return; } catch (e) {} } } new B();", "ReferenceError");
    // Direct eval reads the constructor's `this` binding when it runs.
    throws("class A {} class B extends A { constructor() { eval('this'); super(); } } new B();", "ReferenceError");
    truth("class A {} class B extends A { constructor() { eval('1'); super(); this.ok = eval('this') === this; } } new B().ok");
    truth("class A {} class B extends A { constructor() { var f = eval('() => this'); super(); this.f = f; } } var b = new B(); b.f() === b");
    truth(
        "var seen; class A {} class B extends A { constructor() { var o = { g() { return eval('this'); } }; seen = o.g() === o; super(); } } new B(); seen",
    );
}

#[test]
fn extends_null_and_bad_heritage() {
    truth("class N extends null {} Object.getPrototypeOf(N.prototype) === null && Object.getPrototypeOf(N) === Function.prototype");
    truth("class N extends null { constructor() { return Object.create(N.prototype); } } new N() instanceof N");
    throws("class N extends null {} new N();", "TypeError");
    throws("class B extends 3 {}", "TypeError");
    throws("var f = () => {}; class B extends f {}", "TypeError");
    throws("function F() {} F.prototype = 3; class B extends F {}", "TypeError");
    throws("class B extends B {}", "ReferenceError");
    throws("class A { m() {} } class B extends A.prototype.m {}", "TypeError");
}

#[test]
fn class_heritage_and_computed_keys_run_as_strict_mode_code() {
    throws("var o = {}; Object.defineProperty(o, 'x', { value: 1 }); class C { [(o.x = 2)]() {} }", "TypeError");
    throws("class C extends (undeclared = Object) {}", "ReferenceError");
    throws("var C = class { static [(undeclared = 'k')]() {} };", "ReferenceError");
    // The code around the class stays sloppy, after the class and in a handler of what it throws.
    truth("class C extends Object {} after = 1; after === 1");
    truth("try { class C extends (function () { throw 1; })() {} } catch (e) { caught = e; } caught === 1");
    truth("try { class C { [(function () { throw 2; })()]() {} } } catch (e) { caught = e; } caught === 2");
}

#[test]
fn a_class_extends_other_built_ins_but_new_on_it_throws() {
    let e4x = || Realm::with_options(Options { e4x: true, ..es_options() }, Limits::default());
    assert_eq!(e4x().run("class X extends XML {} typeof X === 'function' && Object.getPrototypeOf(X) === XML"), Ok(JsValue::Bool(true)));
    for src in ["class X extends XML {} new X('<a/>');", "class X extends XML { constructor() { super('<a/>'); } } new X();"] {
        match e4x().run(src) {
            Err(RunError { kind: RunErrorKind::Thrown, message, .. }) => assert!(message.starts_with("TypeError"), "{src}: {message}"),
            other => panic!("{src}: {other:?}"),
        }
    }
    truth("class E extends RangeError {} var e = new E('m'); e instanceof E && e instanceof RangeError && e.message === 'm'");
}

#[test]
fn a_class_is_called_only_with_new() {
    throws("class A {} A();", "TypeError");
    throws("class A {} A.call({});", "TypeError");
    throws("class A { m() {} } new A.prototype.m();", "TypeError");
    throws("class A { static m() {} } new A.m();", "TypeError");
    throws("class A {} class B extends A {} B();", "TypeError");
    truth("class A {} var Bound = A.bind(null); new Bound() instanceof A");
}

#[test]
fn class_bindings_follow_let_rules() {
    throws("new A(); class A {}", "ReferenceError");
    truth("class A {} { class A { m() { return 2; } } } typeof A.prototype.m === 'undefined'");
    throws("class A { m() { A = 1; } } new A().m();", "TypeError");
    // The inner name is initialized only after every method is defined (section 14.5.14, steps
    // 21 and 23), so a computed key that reads it is a `ReferenceError`.
    throws("class A { [A]() {} }", "ReferenceError");
    throws("var C = class D { static [D]() {} };", "ReferenceError");
    truth("class A {} A = 1; A === 1");
    syntax("class A {} class A {}");
    syntax("if (1) class A {}");
    syntax("class {}");
    syntax("class A { constructor() {} constructor() {} }");
    syntax("class A { get constructor() {} }");
    syntax("class A { *g() {} }");
    syntax("class A { m() { super(); } }");
    syntax("class A extends Object { m() { super(); } }");
    syntax("function f() { super.x; }");
    syntax("super.x");
    syntax("new.target");
    syntax("class A { constructor() { var o = { f: function () { super.x; } }; } }");
}

#[test]
fn new_target_names_the_constructor_new_was_applied_to() {
    truth("function F() { return new.target; } new F() === F && F() === undefined");
    truth("function F() { return (() => new.target)(); } new F() === F");
    truth("function F() { this.t = new.target; } var B = F.bind(null); new B().t === F");
}

#[test]
fn deep_class_chains_stay_within_the_limits() {
    let src = "var C = class {}; for (var i = 0; i < 2000; i++) { C = class extends C {}; } new C();";
    let e = Realm::with_options(es_options(), Limits { frames: 500, ..Limits::default() }).run(src).unwrap_err();
    assert!(matches!(e.kind, RunErrorKind::Limit(_)), "{e:?}");
    let mut realm = Realm::with_options(es_options(), Limits { fuel: 50_000, ..Limits::default() });
    let e = realm.run("class A { constructor() { new B(); } } class B extends A {} new B();").unwrap_err();
    assert!(matches!(e.kind, RunErrorKind::Limit(_)), "{e:?}");
}

#[test]
fn classes_survive_collections_while_they_are_made() {
    let src = "var total = 0; for (var i = 0; i < 200; i++) { \
        class A { constructor(x) { this.x = x; } m() { return this.x; } static s() { return 1; } get g() { return 1; } } \
        class B extends A { constructor() { super(1); } m() { return super.m() + 1; } } \
        total += new B().m() + B.s() + new B().g; } total";
    let mut completed = 0;
    for kib in (16..400).step_by(4) {
        let mut realm = Realm::with_options(es_options(), Limits { heap_bytes: kib << 10, ..Limits::default() });
        match realm.run(src) {
            Ok(v) => {
                assert_eq!(v, JsValue::Number(800.0), "{kib} KiB");
                completed += 1;
            }
            Err(e) => assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{kib} KiB: {e:?}"),
        }
    }
    assert!(completed > 0);
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
        "class A {}",
        "var C = class {};",
        "function F() { return new.target; }",
    ] {
        let r = Realm::new().run(src);
        assert!(matches!(r, Err(RunError { kind: RunErrorKind::Syntax, .. })), "{src}: {r:?}");
        let r = Realm::with_options(Options { e4x: true, ..Options::default() }, Limits::default()).run(src);
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
    "class ",
    " extends ",
    "super",
    "static ",
    "get ",
    "constructor",
    "new.target",
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
