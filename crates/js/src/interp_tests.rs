use std::rc::Rc;

use proptest::prelude::*;

use crate::bytecode::{Code, Handler, Op, Source, verify};
use crate::{JsValue, LimitKind, Limits, Options, Realm, RunError, RunErrorKind};

fn run(src: &str) -> JsValue {
    match Realm::new().run(src) {
        Ok(v) => v,
        Err(e) => panic!("{src}\n{e:?}"),
    }
}

fn num(src: &str) -> f64 {
    match run(src) {
        JsValue::Number(n) => n,
        other => panic!("{src}: {other:?}"),
    }
}

fn string(src: &str) -> String {
    match run(src) {
        JsValue::String(s) => s,
        other => panic!("{src}: {other:?}"),
    }
}

fn truth(src: &str) {
    assert_eq!(run(src), JsValue::Bool(true), "{src}");
}

fn fail(src: &str) -> RunError {
    match Realm::new().run(src) {
        Ok(v) => panic!("{src} completed with {v:?}"),
        Err(e) => e,
    }
}

fn small(limits: Limits) -> Realm {
    Realm::with_options(Options::default(), limits)
}

// ---- language ----

#[test]
fn assignments_resolve_the_variable_before_the_right_hand_side() {
    // Section 11.13.1: the reference is resolved first, so a binding the right-hand side adds
    // does not catch the assignment.
    truth("var x = 0; var scope = {}; with (scope) { x = (scope.x = 2, 1); } scope.x === 2 && x === 1");
    truth("var x = 0; var scope = {}; with (scope) { x += (scope.x = 2, 1); } scope.x === 2 && x === 1");
    truth("var x = 5; function f() { var r = (x = (eval('var x;'), 1)); return [x, r]; } var a = f(); a[0] === undefined && a[1] === 1 && x === 1");
    // Section 11.3.1: the operand's reference is resolved before ToNumber runs.
    truth(
        "var x = 0; var scope = {x: {valueOf: function(){ delete scope.x; return 2; }}}; \
         with (scope) { x++; } scope.x === 3 && x === 0",
    );
}

#[test]
fn arithmetic_comparison_and_conversions() {
    assert_eq!(num("1 + 2 * 3 - 4 / 2"), 5.0);
    assert_eq!(num("7 % 3 + (-7 % 3)"), 0.0);
    assert_eq!(string("'a' + 1 + 2"), "a12");
    assert_eq!(num("1 + 2 + '3' - 0"), 33.0);
    assert_eq!(num("(1 << 31) >> 31"), -1.0);
    assert_eq!(num("-1 >>> 28"), 15.0);
    assert_eq!(num("~5 ^ 3 | 8 & 12"), -7.0);
    truth("'10' == 10 && null == undefined && !(null == 0) && '' == 0 && '1e1' == 10");
    truth("NaN !== NaN && 0 === -0 && 1/-0 === -Infinity");
    truth("'b' > 'a' && 'B' < 'a' && !(NaN < 1) && !(NaN >= 1) && 2 >= 2 && '10' < '9' && 10 > '9'");
    truth(
        "typeof 1 === 'number' && typeof 'x' === 'string' && typeof null === 'object' && typeof undefined_name === 'undefined' && typeof function(){} === 'function'",
    );
    truth("({valueOf: function(){ return 4; }}) * 2 === 8 && ({toString: function(){ return 'x'; }}) + 'y' === 'xy'");
    truth("+'0x10' === 16 && +' 12 ' === 12 && isNaNish(+'1x'); function isNaNish(n) { return n !== n; }");
}

#[test]
fn variables_functions_and_closures() {
    assert_eq!(num("var x = 1; function f() { return x + 1; } f()"), 2.0);
    assert_eq!(num("function counter() { var n = 0; return function() { return ++n; }; } var c = counter(); c(); c(); c()"), 3.0);
    assert_eq!(num("f(); function f() { return 5; }"), 5.0);
    assert_eq!(num("var g = function fact(n) { return n <= 1 ? 1 : n * fact(n - 1); }; g(10)"), 3_628_800.0);
    assert_eq!(num("function f(a, b) { return arguments.length * 10 + (b === undefined ? 1 : 0); } f(1)"), 11.0);
    assert_eq!(num("function f(a) { arguments[0] = 9; return a; } f(1)"), 9.0);
    assert_eq!(num("function f(a) { 'use strict'; arguments[0] = 9; return a; } f(1)"), 1.0);
    assert_eq!(num("function f(a) { a = 4; return arguments[0]; } f(1)"), 4.0);
    truth("var o = { m: function() { return this; } }; o.m() === o");
    truth("function f() { return this; } f() === this");
    truth("function f() { 'use strict'; return this; } f() === undefined");
    assert_eq!(num("function P(x) { this.x = x; } P.prototype.get = function() { return this.x; }; new P(7).get()"), 7.0);
    truth("function P() { return {a: 1}; } new P().a === 1 && new P() instanceof Object");
    truth("function P() {} var p = new P(); p instanceof P && p.constructor === P && P.prototype.isPrototypeOf(p)");
    assert_eq!(num("(function(a, b, c) {}).length"), 3.0);
}

#[test]
fn call_and_apply_set_this_and_arguments() {
    assert_eq!(num("function f(a, b) { return this.k + a + b; } f.call({k: 1}, 2, 3)"), 6.0);
    assert_eq!(num("function f(a, b) { return this.k + a + b; } f.apply({k: 1}, [2, 3])"), 6.0);
    assert_eq!(num("function f() { return arguments.length; } f.apply(null, undefined)"), 0.0);
    assert_eq!(num("Function.prototype.call.call(function(a) { return a; }, null, 8)"), 8.0);
    let e = fail("Function.prototype.call.call(1)");
    assert_eq!(e.kind, RunErrorKind::Thrown);
    assert!(e.message.starts_with("TypeError"), "{}", e.message);
}

#[test]
fn objects_arrays_and_accessors() {
    assert_eq!(num("var o = {a: 1, 'b': 2, 3: 4}; o.a + o['b'] + o[3]"), 7.0);
    assert_eq!(num("var a = [1, , 3]; a.length"), 3.0);
    assert_eq!(num("var a = []; a[5] = 1; a.length"), 6.0);
    assert_eq!(num("var a = [1, 2, 3, 4]; a.length = 1; a[2] === undefined ? a.length : -1"), 1.0);
    assert_eq!(num("var o = { get x() { return this.y * 2; }, set x(v) { this.y = v; }, y: 1 }; o.x = 5; o.x"), 10.0);
    truth("var o = {a: 1}; delete o.a && !('a' in o) && o.a === undefined");
    truth("'length' in [] && !delete [].length");
    truth("'abc'.length === 3 && 'abc'[1] === 'b' && 'abc'.x === undefined");
    truth("var o = {}; o.hasOwnProperty('x') === false && ({x: 1}).hasOwnProperty('x')");
    truth("Object.prototype.toString.call([]) === '[object Array]' && Object.prototype.toString.call(null) === '[object Null]'");
    // Error.prototype and each NativeError prototype are Error objects (sections 15.11.4 and 15.11.7.7).
    truth(
        "var s = Object.prototype.toString, all = [Error, EvalError, RangeError, ReferenceError, SyntaxError, TypeError, URIError], ok = true;\
         for (var i = 0; i < all.length; i++) ok = ok && s.call(all[i].prototype) === '[object Error]'; ok",
    );
    let e = fail("var a = []; a.length = -1");
    assert!(e.message.starts_with("RangeError"), "{}", e.message);
    let e = fail("'use strict'; var o = { get x() { return 1; } }; o.x = 2;");
    assert!(e.message.starts_with("TypeError"), "{}", e.message);
    let e = fail("null.x");
    assert!(e.message.starts_with("TypeError"), "{}", e.message);
}

#[test]
fn for_in_lists_own_properties_in_insertion_order_then_inherited_ones() {
    assert_eq!(
        string("function P() { this.z = 1; this.a = 2; } P.prototype.m = 3; P.prototype.a = 9; var s = ''; for (var k in new P()) s += k; s"),
        "zam"
    );
    assert_eq!(string("var o = {a: 1, b: 2, c: 3}, s = ''; for (var k in o) { delete o.b; s += k; } s"), "ac");
    assert_eq!(string("var s = ''; for (var k in 'xyz') s += k; s"), "012");
    assert_eq!(num("var n = 0; for (var k in null) n++; for (k in undefined) n++; n"), 0.0);
    assert_eq!(string("var o = {p: 1}, t = {}, s = ''; for (t.k in o) s += t.k; s"), "p");
    assert_eq!(string("var a = ['x', 'y'], s = ''; for (var i in a) s += i + a[i]; s"), "0x1y");
}

#[test]
fn control_flow_with_labels_switch_and_finally() {
    assert_eq!(
        num(
            "var n = 0; outer: for (var i = 0; i < 5; i++) { for (var j = 0; j < 5; j++) { if (j == 2) continue outer; if (i == 3) break outer; n++; } } n"
        ),
        6.0
    );
    assert_eq!(num("var r = 0; switch (3) { case 1: r = 1; case 3: r += 3; case 4: r += 4; break; default: r = 100; } r"), 7.0);
    assert_eq!(num("var r = 0; switch ('z') { case 'a': r = 1; break; default: r = 2; case 'b': r += 10; } r"), 12.0);
    assert_eq!(num("var log = 0; function f() { try { return 1; } finally { log = 5; } } f() + log"), 6.0);
    assert_eq!(num("function f() { try { return 1; } finally { return 2; } } f()"), 2.0);
    assert_eq!(num("var n = 0; for (var i = 0; i < 3; i++) { try { if (i == 1) continue; n += 10; } finally { n += 1; } } n"), 23.0);
    assert_eq!(num("var n = 0; while (true) { try { try { break; } finally { n += 1; } } finally { n += 10; } } n"), 11.0);
    assert_eq!(num("var n = 0; try { throw 3; } catch (e) { n = e; } finally { n *= 2; } n"), 6.0);
    assert_eq!(num("var n = 0; try { try { throw 1; } finally { n = 7; } } catch (e) { n += e; } n"), 8.0);
    assert_eq!(num("function f() { for (var k in {a: 1}) { try { return 4; } finally { } } } f()"), 4.0);
    assert_eq!(num("var x = 0; lbl: { x = 1; break lbl; x = 2; } x"), 1.0);
    assert_eq!(num("var i = 0; do { i++; } while (i < 10); i"), 10.0);
    assert_eq!(num("var e = 1; try { throw 2; } catch (e) { e = 3; } e"), 1.0);
    assert_eq!(num("function f() { var r = 0; with ({a: 1}) { try { return a; } finally { r = 1; } } } f()"), 1.0);
}

#[test]
fn a_finally_block_keeps_the_completion_value_unless_it_ends_abruptly() {
    // Section 12.14: a finally block that ends normally leaves the try or catch block's value.
    let cases = [
        ("try { 1; } finally { 2; }", 1.0),
        ("try { throw 0; } catch (e) { 1; } finally { 2; }", 1.0),
        ("eval('try { 1; } finally { 2; }')", 1.0),
        ("try { try { 1; } finally { 2; } } finally { 3; }", 1.0),
        ("do { try { 1; } finally { 2; break; } } while (false)", 2.0),
        ("do { try { 1; } finally { 2; continue; } } while (false)", 2.0),
    ];
    let wrong: Vec<(&str, f64, f64)> = cases.iter().map(|&(src, want)| (src, want, num(src))).filter(|&(_, want, got)| want != got).collect();
    assert!(wrong.is_empty(), "{wrong:?}");
}

#[test]
fn with_eval_and_new_function() {
    assert_eq!(num("var o = {a: 2}; with (o) { a = 3; } o.a"), 3.0);
    truth("var o = {f: function() { return this; }}; with (o) { f() === o }");
    assert_eq!(num("var x = 1; function f() { var x = 2; return eval('x'); } f()"), 2.0);
    assert_eq!(num("var x = 1; function f() { var x = 2; return (0, eval)('x'); } f()"), 1.0);
    assert_eq!(num("function f() { eval('var y = 4'); return y; } f()"), 4.0);
    truth("function f() { 'use strict'; eval('var y = 4'); return typeof y; } f() === 'undefined'");
    assert_eq!(num("eval('1; 2; if (false) {}')"), 2.0);
    assert_eq!(num("new Function('a', 'b', 'return a * b;')(6, 7)"), 42.0);
    assert_eq!(num("Function('return 3')()"), 3.0);
    assert_eq!(num("var anonymous = 42; Function('return anonymous')()"), 42.0);
    let e = fail("new Function('a){}; (function(', 'return 1')");
    assert!(e.message.starts_with("SyntaxError"), "{}", e.message);
    let e = fail("new Function('}); (function() {')");
    assert!(e.message.starts_with("SyntaxError"), "{}", e.message);
    let e = fail("eval('var = 1')");
    assert!(e.message.starts_with("SyntaxError"), "{}", e.message);
    truth("try { eval('a b') } catch (e) { e instanceof SyntaxError }");
    assert_eq!(string("(function f(a) { return a; }).toString()"), "function f(a) { return a; }");
}

#[test]
fn strict_mode_assignments_and_errors() {
    let e = fail("'use strict'; undeclared = 1;");
    assert!(e.message.starts_with("ReferenceError"), "{}", e.message);
    assert_eq!(num("undeclared = 1; undeclared"), 1.0);
    let e = fail("'use strict'; (function f() { f = 1; })()");
    assert!(e.message.starts_with("TypeError"), "{}", e.message);
    assert_eq!(num("var g = function f() { f = 1; return typeof f === 'function' ? 1 : 0; }; g()"), 1.0);
    let e = fail("'use strict'; NaN = 1;");
    assert!(e.message.starts_with("TypeError"), "{}", e.message);
    let e = fail("'use strict'; (function() {}).caller");
    assert!(e.message.starts_with("TypeError"), "{}", e.message);
    truth("var o = {}; try { o.x.y; false } catch (e) { e instanceof TypeError && e.constructor === TypeError }");
    truth(
        "try { nope; } catch (e) { e instanceof ReferenceError && e.name === 'ReferenceError' && String_(e) === 'ReferenceError: nope is not defined' } function String_(v) { return '' + v; }",
    );
    truth(
        "new Error('m').message === 'm' && Error('m') instanceof Error && new TypeError().message === '' && new RangeError('r').toString() === 'RangeError: r'",
    );
}

#[test]
fn errors_carry_line_and_column() {
    let e = fail("var a = 1;\nvar b = 2;\n  nope();");
    assert_eq!((e.kind, e.line, e.column), (RunErrorKind::Thrown, 3, 3));
    assert_eq!(e.message, "ReferenceError: nope is not defined");
    let e = fail("function f() {\n  throw new TypeError('bad');\n}\nf();");
    assert_eq!((e.line, e.column), (2, 3));
    assert_eq!(e.message, "TypeError: bad");
    let e = fail("var x = ;");
    assert_eq!((e.kind, e.line), (RunErrorKind::Syntax, 1));
    let e = fail("throw 42");
    assert_eq!(e.message, "42");
}

#[test]
fn host_functions_receive_arguments_and_return_values() {
    let mut realm = Realm::new();
    realm.define_function("add", |args| match args {
        [JsValue::Number(a), JsValue::Number(b)] => Ok(JsValue::Number(a + b)),
        _ => Err("add takes two numbers".into()),
    });
    assert_eq!(realm.run("add(2, 3)"), Ok(JsValue::Number(5.0)));
    let e = realm.run("add('x')").unwrap_err();
    assert_eq!(e.message, "Error: add takes two numbers");
    assert_eq!(realm.run("try { add(); } catch (e) { e.message }"), Ok(JsValue::String("add takes two numbers".into())));
    assert_eq!(realm.run("var kept = 1; 'next'"), Ok(JsValue::String("next".into())));
    assert_eq!(realm.run("kept"), Ok(JsValue::Number(1.0)), "a realm keeps its globals between runs");
}

// ---- limits ----

fn limit(realm: &mut Realm, src: &str) -> LimitKind {
    match realm.run(src) {
        Err(RunError { kind: RunErrorKind::Limit(k), .. }) => k,
        other => panic!("{src}: {other:?}"),
    }
}

fn with_big(limits: Limits) -> Realm {
    let mut realm = small(limits);
    realm.define_function("big", |args| match args {
        [JsValue::Number(n)] => Ok(JsValue::String("x".repeat(*n as usize))),
        _ => Err("big takes a length".into()),
    });
    realm.define_function("fail", |args| match args {
        [JsValue::Number(n)] => Err("x".repeat(*n as usize)),
        _ => Err("fail takes a length".into()),
    });
    realm
}

#[test]
fn error_messages_from_host_functions_count_toward_the_heap() {
    let limits = Limits { heap_bytes: 1 << 20, fuel: 100_000, ..Limits::default() };
    let mut realm = with_big(limits.clone());
    assert_eq!(realm.run("try { fail(1000); } catch (e) { e.message.length }"), Ok(JsValue::Number(1000.0)));
    assert_eq!(limit(&mut with_big(limits.clone()), "try { fail(1000000); } catch (e) {}"), LimitKind::Heap);
    assert_eq!(limit(&mut with_big(limits), "var a = []; for (var i = 0; ; i++) try { fail(100000); } catch (e) { a[i] = e; }"), LimitKind::Heap);
}

#[test]
fn names_from_source_count_toward_the_heap_when_they_are_bound() {
    // Without a collection in between, the estimate grows by at least each new name's code units:
    // a global variable (an object property), a binding made by direct eval, and a parameter of a
    // function made by the Function constructor.
    // Each case runs in a new realm, so the charges for the code itself do not add up to a
    // collection.
    let name = "n".repeat(50_000);
    let cases = [
        format!("var {name} = 1;"),
        format!("(function () {{ eval('var {name}2 = 1; {name}2'); }})();"),
        format!("Function('{name}3', 'return {name}3')(1);"),
    ];
    let mut short = Vec::new();
    for (i, src) in cases.iter().enumerate() {
        let mut realm = small(Limits::default());
        let collections = realm.interp().heap.collections;
        let before = realm.interp().heap.bytes;
        realm.run(src).unwrap();
        assert_eq!(realm.interp().heap.collections, collections, "a collection ran in case {i}");
        let grown = realm.interp().heap.bytes - before;
        if grown < crate::heap::string_bytes(50_000) {
            short.push((i, grown));
        }
    }
    assert!(short.is_empty(), "cases that grew by less than the name: {short:?}");
}

#[test]
fn strings_from_host_functions_count_toward_the_heap() {
    let limits = Limits { heap_bytes: 1 << 20, fuel: 100_000, ..Limits::default() };
    let mut realm = with_big(limits.clone());
    assert_eq!(realm.run("big(1000).length"), Ok(JsValue::Number(1000.0)));
    assert_eq!(realm.run("var a = []; for (var i = 0; i < 4; i++) a[i] = big(50000); a[3].length"), Ok(JsValue::Number(50000.0)));
    assert_eq!(limit(&mut with_big(limits.clone()), "var a = []; for (var i = 0; ; i++) a[i] = big(100000);"), LimitKind::Heap);
    assert_eq!(limit(&mut with_big(limits), "big(1000000).length"), LimitKind::Heap);
}

#[test]
fn functions_kept_across_runs_count_their_code_toward_the_heap() {
    // Each run keeps a function whose source holds a 64 Ki-unit comment, and drops the string the
    // source was made from. The kept code fills the heap after a few runs.
    let limits = Limits { heap_bytes: 1 << 20, ..Limits::default() };
    let mut realm = small(limits);
    assert_eq!(realm.run("var keep = []; keep.length"), Ok(JsValue::Number(0.0)));
    let step = "var s = 'x'; for (var i = 0; i < 16; i++) s += s; keep[keep.length] = Function('/*' + s + '*/'); s = null; keep.length";
    let mut ended = None;
    for run in 1..=20 {
        match realm.run(step) {
            Ok(v) => assert_eq!(v, JsValue::Number(f64::from(run))),
            Err(RunError { kind: RunErrorKind::Limit(k), .. }) => {
                ended = Some((run, k));
                break;
            }
            Err(e) => panic!("run {run}: {e:?}"),
        }
    }
    let Some((run, kind)) = ended else { panic!("20 kept functions fit in a 1 MiB heap") };
    assert_eq!(kind, LimitKind::Heap);
    assert!(run > 2, "the heap filled on run {run}");
}

#[test]
fn function_source_text_counts_toward_the_heap() {
    let limits = Limits { heap_bytes: 1 << 20, ..Limits::default() };
    let comment = "x".repeat(300_000);
    let src = format!("var f = function () {{ /*{comment}*/ }}; f.toString().length");
    assert_eq!(limit(&mut small(limits.clone()), &src), LimitKind::Heap);
    let mut realm = small(limits);
    assert_eq!(realm.run("(function f(a) { return a; }).toString()"), Ok(JsValue::String("function f(a) { return a; }".into())));
}

#[test]
fn for_in_names_take_fuel_and_heap_under_later_limits() {
    // The first run, under the default limits, builds an object with many properties. Later runs
    // with smaller limits list its names only within their own fuel and heap.
    let mut realm = small(Limits::default());
    assert_eq!(realm.run("var o = {}; for (var i = 0; i < 100000; i++) o['k' + i] = i; o.k99999"), Ok(JsValue::Number(99999.0)));
    realm.set_limits(Limits { fuel: 1000, ..Limits::default() });
    assert_eq!(limit(&mut realm, "for (var k in o) break;"), LimitKind::Fuel);
    realm.set_limits(Limits::default());
    assert_eq!(realm.run("var n = 0; for (var k in o) n++; n"), Ok(JsValue::Number(100_000.0)));
    realm.interp().collect_garbage();
    let live = realm.interp().heap.bytes;
    // The list of names takes more than 3 MB (at least 32 + 2 * 2 bytes each), so 1 MiB more than
    // the live heap holds the object but not the list.
    let heap_bytes = live + (1 << 20);
    realm.set_limits(Limits { heap_bytes, ..Limits::default() });
    assert_eq!(realm.run("o.k0"), Ok(JsValue::Number(0.0)));
    assert_eq!(limit(&mut realm, "for (var k in o) break;"), LimitKind::Heap);
    // The run ends before the list is made, so the estimate stays within the limit.
    assert!(realm.interp().heap.bytes <= heap_bytes, "{} > {heap_bytes}", realm.interp().heap.bytes);
}

#[test]
fn shrinking_an_array_takes_fuel_for_each_property() {
    let mut realm = small(Limits::default());
    assert_eq!(realm.run("var a = []; for (var i = 0; i < 100000; i++) a[i] = i; a.length"), Ok(JsValue::Number(100_000.0)));
    realm.set_limits(Limits { fuel: 1000, ..Limits::default() });
    assert_eq!(limit(&mut realm, "a.length = 0;"), LimitKind::Fuel);
    realm.set_limits(Limits::default());
    // The run stopped before anything was removed.
    assert_eq!(realm.run("a.length + a[99999]"), Ok(JsValue::Number(199_999.0)));
    assert_eq!(realm.run("a.length = 1; a.length + a[0] + (99999 in a ? 1 : 0)"), Ok(JsValue::Number(1.0)));
}

#[test]
fn apply_takes_fuel_for_each_argument_it_reads() {
    let mut realm = small(Limits { fuel: 1000, ..Limits::default() });
    assert_eq!(limit(&mut realm, "(function () {}).apply(null, {length: 65536});"), LimitKind::Fuel);
    assert_eq!(realm.run("(function () { return arguments.length; }).apply(null, {length: 100})"), Ok(JsValue::Number(100.0)));
}

#[test]
fn declarations_check_the_heap_before_they_are_bound() {
    // Each script declares more than fits in the heap. The run ends with the heap limit before
    // the bindings are made, so the estimate never goes past the limit.
    // Short names keep the source strings well inside the heap while the bindings are not.
    let limits = Limits { heap_bytes: 2 << 20, ..Limits::default() };
    let short = |mut i: usize| {
        let mut name = String::from("_");
        loop {
            name.push(char::from(b'a' + (i % 26) as u8));
            i /= 26;
            if i == 0 {
                return name;
            }
        }
    };
    let vars = (0..40_000).map(short).collect::<Vec<_>>().join(",");
    let funs = (0..20_000).map(|i| format!("function f{i}() {{}}")).collect::<String>();
    let cases = [
        format!("var {vars};"),
        funs.clone(),
        format!("(function () {{ eval('var {vars};'); }})();"),
        format!("(function () {{ eval('{funs}'); }})();"),
        format!("(0, eval)('\"use strict\"; var {vars};');"),
        format!("Function('var {vars};')();"),
        format!("Function('{funs}')();"),
    ];
    let mut wrong = Vec::new();
    for (i, src) in cases.iter().enumerate() {
        let mut realm = small(limits.clone());
        let r = realm.run(src).map_err(|e| e.kind);
        let bytes = realm.interp().heap.bytes;
        if r != Err(RunErrorKind::Limit(LimitKind::Heap)) || bytes > limits.heap_bytes {
            wrong.push((i, r, bytes));
        }
    }
    assert!(wrong.is_empty(), "cases that bound past the limit: {wrong:?}");
    let few = (0..1000).map(|i| format!("b{i}")).collect::<Vec<_>>().join(",");
    let mut realm = small(limits);
    assert_eq!(realm.run(&format!("var {few}; b999 = 1; Function('var {few}; return 2')() + b999")), Ok(JsValue::Number(3.0)));
}

#[test]
fn declarations_take_fuel() {
    let vars = (0..10_000).map(|i| format!("a{i}")).collect::<Vec<_>>().join(",");
    let mut realm = small(Limits { fuel: 1000, ..Limits::default() });
    assert_eq!(limit(&mut realm, &format!("var {vars};")), LimitKind::Fuel);
    assert_eq!(limit(&mut realm, &format!("Function('var {vars};')();")), LimitKind::Fuel);
}

#[test]
fn a_lower_heap_limit_applies_to_what_is_already_live() {
    // A heap already past the new limit ends the next run.
    let mut realm = small(Limits::default());
    realm.run("var keep = []; for (var i = 0; i < 100000; i++) keep[i] = {i: i};").unwrap();
    realm.set_limits(Limits { heap_bytes: 1 << 20, ..Limits::default() });
    assert_eq!(limit(&mut realm, "for (var j = 0; j < 1000; j++) ({j: j});"), LimitKind::Heap);
    realm.set_limits(Limits::default());
    realm.run("keep = null;").unwrap();
    realm.set_limits(Limits { heap_bytes: 1 << 20, ..Limits::default() });
    assert_eq!(realm.run("for (var j = 0; j < 1000; j++) ({j: j}); j"), Ok(JsValue::Number(1000.0)));
    // A heap below the new limit cannot grow past it: the next collection runs by the new limit,
    // not by when it would have run under the old, higher one.
    let mut realm = small(Limits::default());
    realm.run("var keep = []; for (var i = 0; i < 100000; i++) keep[i] = {i: i};").unwrap();
    let live = realm.collect_garbage();
    realm.set_limits(Limits { heap_bytes: live + (1 << 20), ..Limits::default() });
    assert_eq!(limit(&mut realm, "var more = []; for (var j = 0; j < 20000; j++) more[j] = {j: j};"), LimitKind::Heap);
}

#[test]
fn apply_keeps_its_arguments_alive_while_a_native_function_runs() {
    // The first argument's toString makes 20,000 objects, more than a 1 MiB heap holds, so a
    // collection runs before it returns. The second argument is reachable only from the list
    // that apply read from the array-like object.
    let mut realm = small(Limits { heap_bytes: 1 << 20, ..Limits::default() });
    let src = "var list = {length: 2, \
        get 0() { return {toString: function () { for (var i = 0; i < 20000; i++) ({i: i}); return 'a'; }}; }, \
        get 1() { return {toString: function () { return 'return a * 2'; }}; }}; \
        Function.apply(null, list)(21)";
    assert_eq!(realm.run(src), Ok(JsValue::Number(42.0)));
}

#[test]
fn to_number_skips_the_white_space_the_lexer_skips() {
    let mut realm = small(Limits::default());
    assert_eq!(realm.run("var n = +'\\u180E'; n !== n"), Ok(JsValue::Bool(true)), "U+180E is not white space");
    assert_eq!(realm.run("var m = +'\\u180E1'; m !== m"), Ok(JsValue::Bool(true)));
    assert_eq!(realm.run("+' \\t\\n\\u2028\\u3000\\uFEFF 1 \\u00A0'"), Ok(JsValue::Number(1.0)));
    assert_eq!(realm.run("+'   1 '"), Ok(JsValue::Number(1.0)));
}

#[test]
fn an_infinite_loop_ends_with_the_fuel_limit() {
    let mut realm = small(Limits { fuel: 100_000, ..Limits::default() });
    assert_eq!(limit(&mut realm, "for (;;) {}"), LimitKind::Fuel);
    assert_eq!(limit(&mut realm, "while (true) { try { for (;;) {} } catch (e) {} }"), LimitKind::Fuel);
    assert_eq!(limit(&mut realm, "var n = 0; try { for (;;) {} } finally { n = 1; } "), LimitKind::Fuel);
    assert_eq!(
        limit(&mut realm, "function f() { try { for (;;) {} } finally { for (;;) {} } } while (true) { try { f(); } catch (e) {} }"),
        LimitKind::Fuel
    );
    assert_eq!(realm.run("n"), Ok(JsValue::Number(0.0)), "the finally block did not run");
    assert_eq!(realm.run("1 + 1"), Ok(JsValue::Number(2.0)), "each run gets fresh fuel");
}

#[test]
fn deep_recursion_ends_with_the_frame_limit_on_a_small_stack() {
    let handle = std::thread::Builder::new()
        .stack_size(256 << 10)
        .spawn(|| {
            let mut realm = Realm::new();
            let r = realm.run("function f(n) { return n === 0 ? 0 : 1 + f(n - 1); } f(1000000)");
            let ok = realm.run("function g(n) { return n === 0 ? 0 : 1 + g(n - 1); } g(5000)");
            let caught = realm.run("function h(n) { try { return h(n + 1); } catch (e) { return -1; } } h(0)");
            (r, ok, caught)
        })
        .unwrap();
    let (r, ok, caught) = handle.join().unwrap();
    assert_eq!(r.map_err(|e| e.kind), Err(RunErrorKind::Limit(LimitKind::Frames)));
    assert_eq!(ok, Ok(JsValue::Number(5000.0)));
    assert_eq!(caught.map_err(|e| e.kind), Err(RunErrorKind::Limit(LimitKind::Frames)), "catch cannot swallow the frame limit");
}

#[test]
fn reentry_through_getters_ends_with_the_reentry_limit() {
    let handle = std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(|| {
            let mut realm = Realm::new();
            let getter = realm.run("var o = { get x() { return this.x; } }; o.x");
            let conversion = realm.run("var v = { valueOf: function() { return v * 1; } }; v * 1");
            let eval = realm.run("function e() { return (0, eval)('e()'); } e()");
            (getter, conversion, eval)
        })
        .unwrap();
    let (getter, conversion, eval) = handle.join().unwrap();
    for r in [getter, conversion, eval] {
        assert_eq!(r.map_err(|e| e.kind), Err(RunErrorKind::Limit(LimitKind::Reentry)));
    }
}

#[test]
fn the_script_itself_does_not_count_against_the_reentry_limit() {
    let mut realm = small(Limits { reentry: 0, ..Limits::default() });
    assert_eq!(realm.run("1 + 1"), Ok(JsValue::Number(2.0)));
    assert_eq!(limit(&mut realm, "var o = { get x() { return 1; } }; o.x"), LimitKind::Reentry);
    realm.set_limits(Limits { reentry: 1, ..Limits::default() });
    assert_eq!(realm.run("o.x"), Ok(JsValue::Number(1.0)));
    assert_eq!(realm.run("(0, eval)('2 + 2')"), Ok(JsValue::Number(4.0)));
    assert_eq!(limit(&mut realm, "var p = { get x() { return o.x; } }; p.x"), LimitKind::Reentry);
}

#[test]
fn memory_bombs_end_with_the_heap_limit() {
    let limits = Limits { heap_bytes: 4 << 20, ..Limits::default() };
    for src in [
        "var a = null; for (;;) a = {next: a, v: [a, a]};",
        "var s = 'x'; for (;;) s = s + s;",
        "var a = []; for (var i = 0; ; i++) a[i] = 'item' + i;",
        "var s = 'x'; for (;;) { s = s + s; var o = {}; o[s] = 1; }",
    ] {
        let mut realm = small(limits.clone());
        assert_eq!(limit(&mut realm, src), LimitKind::Heap, "{src}");
        // What the bomb built is still reachable from the globals; raising the limit lets the
        // realm drop it.
        realm.set_limits(Limits::default());
        assert_eq!(realm.run("a = s = null; 1"), Ok(JsValue::Number(1.0)));
        realm.set_limits(limits.clone());
        assert_eq!(realm.run("2"), Ok(JsValue::Number(2.0)));
    }
}

#[test]
fn garbage_is_collected_cycles_included() {
    let mut realm = small(Limits { heap_bytes: 2 << 20, ..Limits::default() });
    let src = "for (var i = 0; i < 200000; i++) { var a = {}, b = {a: a, s: 'x' + i}; a.b = b; } i";
    assert_eq!(realm.run(src), Ok(JsValue::Number(200_000.0)));
    assert!(realm.interp().heap.collections > 0);
    let before = realm.collect_garbage();
    realm.run("var keep = []; for (var j = 0; j < 1000; j++) keep[j] = {j: j};").unwrap();
    let live = realm.collect_garbage();
    realm.run("keep = null;").unwrap();
    let after = realm.collect_garbage();
    assert!(live > before && after < live, "{before} {live} {after}");
    let cells = realm.interp().heap.live_cells();
    assert!(cells < 200, "{cells} cells are still live");
}

#[test]
fn a_million_appends_flatten_without_recursion() {
    let handle = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let mut realm = Realm::new();
            realm.run("var s = '', t = ''; for (var i = 0; i < 1000000; i++) { s += 'ab'; t = t + 'ab'; } s.length === 2000000 && s === t && s[1999999] === 'b'")
        })
        .unwrap();
    assert_eq!(handle.join().unwrap(), Ok(JsValue::Bool(true)));
}

#[test]
fn the_interrupt_flag_stops_a_run() {
    let mut realm = Realm::new();
    let flag = realm.interrupt();
    flag.set();
    assert_eq!(limit(&mut realm, "1 + 1"), LimitKind::Interrupted);
    flag.clear();
    assert_eq!(realm.run("1 + 1"), Ok(JsValue::Number(2.0)));
    let from_host = realm.interrupt();
    realm.define_function("stop", move |_| {
        from_host.set();
        Ok(JsValue::Undefined)
    });
    assert_eq!(limit(&mut realm, "for (var i = 0; ; i++) { if (i === 1000) stop(); }"), LimitKind::Interrupted);
    assert!(flag.is_set());
    assert_eq!(limit(&mut realm, "i"), LimitKind::Interrupted, "the flag stays set");
    flag.clear();
    assert_eq!(realm.run("i"), Ok(JsValue::Number(1000.0)));
}

#[test]
fn eval_and_function_source_counts_against_the_source_limit() {
    let mut realm = Realm::with_options(Options { max_source_units: 200, ..Options::default() }, Limits::default());
    assert_eq!(realm.run("eval('1 + 1')"), Ok(JsValue::Number(2.0)));
    let e = realm.run("var s = 'x'; for (var i = 0; i < 8; i++) s += s; eval(s)").unwrap_err();
    assert!(e.message.contains(crate::TOO_LONG), "{e:?}");
    let e = realm.run("for (var i = 0; i < 20; i++) eval('1 + 1 + 1 + 1 + 1 + 1 + 1 + 1');").unwrap_err();
    assert!(e.message.contains(crate::TOO_LONG), "the sources of one run count together: {e:?}");
    let e = realm.run("Function('return 1;' + '                                                                                                                                                                                                       ')").unwrap_err();
    assert!(e.message.contains(crate::TOO_LONG), "{e:?}");
}

#[test]
fn the_function_constructor_checks_the_source_limit_before_reading_its_arguments() {
    // A rope argument that is too long is refused before it is flattened.
    let mut realm = Realm::with_options(Options { max_source_units: 1000, ..Options::default() }, Limits::default());
    let e = realm.run("var s = 'return 1;'; for (var i = 0; i < 12; i++) s += s; Function('a', s)").unwrap_err();
    assert!(e.message.contains(crate::TOO_LONG), "{e:?}");
    let interp = realm.interp();
    match interp.find_property(interp.global, &crate::heap::key("s")) {
        Ok(Some(crate::heap::Prop::Data { value: crate::value::Value::String(s), .. })) => assert!(s.is_rope(), "the argument was flattened"),
        other => panic!("{other:?}"),
    }
    // The source is read as code units, so a lone surrogate in an argument is kept.
    let mut realm = Realm::new();
    assert_eq!(realm.run("Function('return \"\\ud800\"')() === '\\ud800'"), Ok(JsValue::Bool(true)));
}

#[test]
fn a_script_longer_than_the_source_limit_is_a_syntax_error() {
    let mut realm = Realm::with_options(Options { max_source_units: 10, ..Options::default() }, Limits::default());
    let e = realm.run("1 + 2 + 3 + 4").unwrap_err();
    assert_eq!((&e.kind, e.message.contains(crate::TOO_LONG)), (&RunErrorKind::Syntax, true), "{e:?}");
    // Ten UTF-16 code units in 18 bytes of UTF-8 fit.
    assert_eq!(realm.run("'\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}'"), Ok(JsValue::String("\u{e9}".repeat(8))));
    let mut realm = Realm::with_options(Options { max_source_units: 40, ..Options::default() }, Limits::default());
    let e = realm.run("Function('return 1;')").unwrap_err();
    assert_eq!((&e.kind, e.message.contains(crate::TOO_LONG)), (&RunErrorKind::Thrown, true), "{e:?}");
}

#[test]
fn bind_last_thrown_binds_undefined_after_a_run_that_did_not_throw() {
    let mut realm = Realm::new();
    assert_eq!(realm.run("throw 'first'").map_err(|e| e.kind), Err(RunErrorKind::Thrown));
    realm.bind_last_thrown("caught");
    assert_eq!(realm.run("caught"), Ok(JsValue::String("first".into())));
    assert_eq!(realm.run("1 + 1"), Ok(JsValue::Number(2.0)));
    realm.bind_last_thrown("caught");
    assert_eq!(realm.run("caught"), Ok(JsValue::Undefined));
    let _ = realm.run("throw 'second'");
    assert_eq!(realm.run("var").map_err(|e| e.kind), Err(RunErrorKind::Syntax));
    realm.bind_last_thrown("caught");
    assert_eq!(realm.run("caught"), Ok(JsValue::Undefined), "a run that does not parse throws nothing");
    realm.set_limits(Limits { fuel: 100, ..Limits::default() });
    let _ = realm.run("throw 'third'");
    assert_eq!(limit(&mut realm, "for (;;) {}"), LimitKind::Fuel);
    realm.bind_last_thrown("caught");
    realm.set_limits(Limits::default());
    assert_eq!(realm.run("caught"), Ok(JsValue::Undefined), "a run that reached a limit threw nothing");
}

#[test]
fn a_strict_realm_runs_scripts_as_strict_mode_code() {
    let mut realm = Realm::with_options(Options { strict: true, ..Options::default() }, Limits::default());
    assert_eq!(realm.run("with ({}) {}").map_err(|e| e.kind), Err(RunErrorKind::Syntax));
    assert_eq!(realm.run("(function () { return this; })()"), Ok(JsValue::Undefined));
    assert_eq!(realm.run("(function () { return typeof this; })()"), Ok(JsValue::String("undefined".into())));
    let mut realm = Realm::new();
    assert_eq!(realm.run("with ({}) {} 1"), Ok(JsValue::Number(1.0)));
    assert_eq!(realm.run("(function () { return typeof this; })()"), Ok(JsValue::String("object".into())));
}

#[test]
fn an_apply_that_throws_while_reading_its_arguments_keeps_nothing_alive() {
    // Each call reads a fresh array at index 0 before index 1 throws; the arrays must not stay
    // reachable after the call ends.
    let mut realm = small(Limits { heap_bytes: 2 << 20, ..Limits::default() });
    let src = "function f() {} var caught = 0; \
        var list = {length: 2, get 0() { var a = []; for (var k = 0; k < 100; k++) a[k] = k; return a; }, get 1() { throw 'stop'; }}; \
        for (var i = 0; i < 5000; i++) { try { f.apply(null, list); } catch (e) { caught++; } } caught";
    assert_eq!(realm.run(src), Ok(JsValue::Number(5000.0)));
}

#[test]
fn for_in_over_a_long_string_checks_the_heap_before_listing_its_indices() {
    // The string fits in the heap and its 131,072 index names do not. The fuel runs out partway
    // through listing them, so only the check before the listing ends the run with the heap limit.
    let mut realm = small(Limits { heap_bytes: 1 << 20, fuel: 50_000, ..Limits::default() });
    assert_eq!(limit(&mut realm, "var s = 'x'; while (s.length < 100000) s += s; for (var k in s) {}"), LimitKind::Heap);
    assert_eq!(realm.run("var n = ''; for (var k in 'abc') n += k; n"), Ok(JsValue::String("012".into())));
}

#[test]
fn long_property_names_count_toward_the_heap_after_their_strings_are_collected() {
    // Each name is a 16,385-unit buffer that only the object's key or the function's binding
    // holds once the string that made it is garbage, so about 32 of them fill the heap well before
    // the fuel runs out. In the second script the property lookup flattens a long string each
    // time, so collections run.
    let mut realm = small(Limits { heap_bytes: 1 << 20, fuel: 20_000, ..Limits::default() });
    let src = "var s = 'x'; while (s.length < 10000) s += s; var o = {}; for (var i = 0; ; i++) o[s + i] = 1;";
    assert_eq!(limit(&mut realm, src), LimitKind::Heap);
    let src = "var s = 'x'; while (s.length < 10000) s += s; \
        (function () { for (var i = 0; ; i++) { eval('var ' + s + i + ' = 1'); ({})[s + i]; } })();";
    let mut realm = small(Limits { heap_bytes: 1 << 20, fuel: 20_000, ..Limits::default() });
    assert_eq!(limit(&mut realm, src), LimitKind::Heap);
}

#[test]
fn for_in_over_a_long_string_takes_fuel_for_each_index() {
    let mut realm = small(Limits { fuel: 10_000, ..Limits::default() });
    assert_eq!(limit(&mut realm, "var s = 'x'; while (s.length < 100000) s += s; for (var k in s) break;"), LimitKind::Fuel);
}

#[test]
fn wasm_builds_have_smaller_default_limits() {
    let limits = Limits::default();
    #[cfg(target_arch = "wasm32")]
    assert_eq!(limits, Limits { fuel: 250_000_000, heap_bytes: 128 << 20, frames: 2_000, reentry: 16 });
    #[cfg(not(target_arch = "wasm32"))]
    assert_eq!(limits, Limits { fuel: 1_000_000_000, heap_bytes: 512 << 20, frames: 10_000, reentry: 32 });
}

#[test]
fn deep_nesting_compiles_on_a_small_stack() {
    let n = crate::DEFAULT_MAX_DEPTH - 5;
    let cases = [
        format!("x = {}1{};", "[".repeat(n), "]".repeat(n)),
        format!("x = {}1{};", "{a:".repeat(n), "}".repeat(n)),
        format!("{}{}", "function f(){".repeat(n), "}".repeat(n)),
        format!("{}x = 1;", "if (1) ".repeat(n)),
        format!("x = {}1;", "1 ? 1 : ".repeat(n)),
        format!("x = 0; {}x++;{}", "while (x < 1) { try { ".repeat(n / 2), " } finally { } }".repeat(n / 2)),
    ];
    let handle = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || cases.iter().map(|src| Realm::new().run(src).map(|_| ()).map_err(|e| e.kind)).collect::<Vec<_>>())
        .unwrap();
    for r in handle.join().unwrap() {
        assert!(r.is_ok() || r == Err(RunErrorKind::Syntax), "{r:?}");
    }
    let long = [
        format!("x = 1{};", " + 1".repeat(100_000)),
        format!("{}x = 1;", "a = ".repeat(100_000)),
        format!("x = {{}}; x.y = x; x{};", ".y".repeat(100_000)),
    ];
    for src in &long {
        Realm::new().run(src).unwrap();
    }
    assert_eq!(num(&format!("var a; {}1; a", "a = ".repeat(100_000))), 1.0);
    assert_eq!(num(&format!("x = 1{}", " + 1".repeat(100_000))), 100_001.0);
}

// ---- the verifier ----

fn code(ops: Vec<Op>) -> Code {
    let source = Rc::new(Source { lines: crate::LineTable::new(&[]), text: Rc::from(Vec::<u16>::new()) });
    Code {
        positions: vec![0; ops.len()],
        ops,
        numbers: vec![1.0],
        strings: Vec::new(),
        names: Vec::new(),
        functions: Vec::new(),
        regexps: Vec::new(),
        handlers: Vec::new(),
        finally_tables: Vec::new(),
        max_stack: 0,
        strict: false,
        self_name: None,
        params: Vec::new(),
        vars: Vec::new(),
        fun_decls: Vec::new(),
        uses_arguments: false,
        source,
        span: (0, 0),
    }
}

#[test]
fn the_verifier_rejects_bad_bytecode() {
    assert_eq!(verify(&code(vec![Op::Number(0), Op::Return])), Ok(1));
    let bad: Vec<(Vec<Op>, &str)> = vec![
        (vec![Op::Jump(9)], "past the end"),
        (vec![Op::Pop, Op::Undefined, Op::Return], "pops below"),
        (vec![Op::Number(5), Op::Return], "number 5 out of range"),
        (vec![Op::GetVar(0), Op::Return], "name 0 out of range"),
        (vec![Op::Closure(0), Op::Return], "function 0 out of range"),
        (vec![Op::Undefined], "past its last instruction"),
        (vec![Op::True, Op::JumpIfFalse(3), Op::Undefined, Op::Undefined, Op::Return], "depths"),
        (vec![Op::Undefined, Op::Undefined, Op::EndFinally(0)], "finally table 0"),
        (vec![], "no instructions"),
    ];
    for (ops, want) in bad {
        let e = verify(&code(ops.clone())).unwrap_err();
        assert!(e.contains(want), "{ops:?}: {e}");
    }
    let mut c = code(vec![Op::Undefined, Op::Return]);
    c.handlers.push(Handler { start: 0, end: 5, target: 0, depth: 0, env_depth: 0 });
    assert!(verify(&c).unwrap_err().contains("handler range"));
}

#[test]
fn every_compiled_function_passes_the_verifier() {
    // Each code is verified as it is compiled; a failure would surface as an internal error.
    let src = "function f(a) { try { for (var k in a) { switch (k) { case 'x': continue; default: break; } } } catch (e) { return e; } finally { a = 0; } \
               with (a) { lbl: do { if (b) break lbl; } while (0); } return function() { return arguments; }; } 1";
    assert_eq!(run(src), JsValue::Number(1.0));
}

// ---- robustness ----

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_fragment_programs_never_panic(parts in proptest::collection::vec(0..crate::tests::FRAGMENTS.len(), 0..60)) {
        let src: String = parts.iter().map(|&i| crate::tests::FRAGMENTS[i]).collect();
        let mut realm = small(Limits { fuel: 20_000, heap_bytes: 1 << 20, frames: 200, reentry: 8 });
        if let Err(e) = realm.run(&src) {
            prop_assert!(e.kind != RunErrorKind::Internal, "{src:?}: {e:?}");
        }
    }

    #[test]
    fn arbitrary_text_never_panics(src in ".{0,120}") {
        let mut realm = small(Limits { fuel: 20_000, heap_bytes: 1 << 20, frames: 200, reentry: 8 });
        if let Err(e) = realm.run(&src) {
            prop_assert!(e.kind != RunErrorKind::Internal, "{src:?}: {e:?}");
        }
    }
}
