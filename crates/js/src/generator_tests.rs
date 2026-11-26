//! Generator functions, `yield`, `yield*` and the generator objects, in a realm with
//! [`Options::es2015`].

use crate::{JsValue, LimitKind, Limits, Options, Realm, RunError, RunErrorKind};

fn es2015(limits: Limits) -> Realm {
    Realm::with_options(Options { es2015: true, ..Options::default() }, limits)
}

fn run(src: &str) -> JsValue {
    match es2015(Limits::default()).run(src) {
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

/// The run throws an error whose message starts with `name`.
fn throws(src: &str, name: &str) {
    match es2015(Limits::default()).run(src) {
        Err(RunError { kind: RunErrorKind::Thrown, message, .. }) => assert!(message.starts_with(name), "{src}: {message}"),
        other => panic!("{src}: {other:?}"),
    }
}

/// The script does not parse in a realm with `options`.
fn syntax_with(src: &str, options: Options) {
    match Realm::with_options(options, Limits::default()).run(src) {
        Err(RunError { kind: RunErrorKind::Syntax, .. }) => {}
        other => panic!("{src}: {other:?}"),
    }
}

fn syntax(src: &str) {
    syntax_with(src, Options { es2015: true, ..Options::default() });
}

#[test]
fn promise_jobs_resume_generator_callbacks() {
    let mut realm = es2015(Limits::default());
    realm
        .run(
            "var log = []; var it = (function* () { log.push('start'); var value = yield 7; \
             log.push('resume:' + value); return 9; })(); \
             Promise.resolve().then(it.next.bind(it)).then(function (result) { \
             log.push(result.value + ':' + result.done); return it.next(5); \
             }).then(function (result) { log.push(result.value + ':' + result.done); });",
        )
        .unwrap();
    assert_eq!(realm.run("log.join(',')").unwrap(), JsValue::String("start,7:false,resume:5,9:true".into()));
}

#[test]
fn promise_static_methods_reject_generator_constructors() {
    truth(
        "var g = function* () {}, count = 0; [g, g.bind(null)].forEach(function (C) { \
         try { Promise.resolve.call(C, 1); } catch (error) { if (error instanceof TypeError) count++; } \
         try { Promise.reject.call(C, 1).catch(function () {}); } \
         catch (error) { if (error instanceof TypeError) count++; } }); count === 4",
    );
}

#[test]
fn a_generator_runs_its_body_one_yield_at_a_time() {
    assert_eq!(
        string(
            "var log = []; function* g(a, b) { log.push('start'); var x = yield a; log.push(x); yield b + x; return 'end'; } \
             var it = g(1, 2); log.push('made'); var r = [it.next('ignored'), it.next(10), it.next(20), it.next()]; \
             log.join() + ' ' + r.map(function (o) { return o.value + ':' + o.done; }).join()"
        ),
        "made,start,10 1:false,12:false,end:true,undefined:true"
    );
    // The parameters' default values run at the call, the body at the first `next`.
    assert_eq!(string("var log = []; function* g(a = log.push('d')) { log.push('b'); } var it = g(); log.push('c'); it.next(); log.join()"), "d,c,b");
    truth("function* g() { yield this; } var o = { g: g }; o.g().next().value === o");
    truth("function* g() { return arguments.length; } g(1, 2, 3).next().value === 3");
    truth("var it = (function* () {})(); var r = it.next(); r.value === undefined && r.done === true");
    truth("function* g() { yield 1; } var a = g(), b = g(); a.next(); a.next().done && !b.next().done");
}

#[test]
fn yield_takes_an_operand_only_on_the_same_line() {
    assert_eq!(string("function* g() { yield\n1; } var it = g(); String(it.next().value) + ',' + it.next().done"), "undefined,true");
    truth("function* g() { var x = yield; return x; } var it = g(); it.next(); it.next(7).value === 7");
    truth("function* g() { yield /a/g; } g().next().value instanceof RegExp");
    truth("function* g() { yield yield 1; } var it = g(); it.next().value === 1 && it.next(2).value === 2");
    truth("function* g() { var a = [yield 1, yield 2]; return a.join(); } var it = g(); it.next(); it.next('x'); it.next('y').value === 'x,y'");
}

#[test]
fn return_runs_the_finally_blocks_around_the_yield() {
    truth(
        "var log = []; function* g() { try { yield 1; yield 2; } finally { log.push('f'); } } \
         var it = g(); it.next(); var r = it.return(9); r.value === 9 && r.done === true && log.join() === 'f' && it.next().done",
    );
    // A `finally` block that yields suspends the return, which ends at the next resumption.
    assert_eq!(
        string(
            "function* g() { try { yield 1; } finally { yield 'cleanup'; } } var it = g(); it.next(); \
             var a = it.return(5), b = it.next(), c = it.next(); [a.value, a.done, b.value, b.done, c.done].join()"
        ),
        "cleanup,false,5,true,true"
    );
    truth("function* g() { try { yield 1; } finally { return 'override'; } } var it = g(); it.next(); it.return(5).value === 'override'");
    // Before the body starts, and after it ends, `return` only completes the generator.
    truth("var ran = false; function* g() { ran = true; } var it = g(); var r = it.return(3); r.value === 3 && r.done && !ran && it.next().done");
    truth("function* g() {} var it = g(); it.next(); var r = it.return(4); r.value === 4 && r.done");
}

#[test]
fn throw_goes_into_a_suspended_generator() {
    truth("function* g() { try { yield 1; } catch (e) { yield 'caught ' + e; } } var it = g(); it.next(); it.throw('x').value === 'caught x'");
    truth(
        "function* g() { yield 1; yield 2; } var it = g(); it.next(); var e; try { it.throw(new RangeError('r')); } catch (x) { e = x; } \
         e instanceof RangeError && it.next().done",
    );
    // A generator that has not started is completed by `throw`, which its body never sees.
    truth(
        "var ran = false; function* g() { ran = true; } var it = g(); var e; try { it.throw(1); } catch (x) { e = x; } e === 1 && !ran && it.next().done",
    );
    throws("function* g() {} var it = g(); it.next(); it.throw(new TypeError('after the end'));", "TypeError: after the end");
}

#[test]
fn resuming_a_running_generator_is_a_type_error() {
    truth(
        "var it; function* g() { try { it.next(); } catch (e) { yield e instanceof TypeError; } } it = g(); \
         it.next().value === true",
    );
    truth("var it; function* g() { it.next(); } it = g(); var e; try { it.next(); } catch (x) { e = x; } e instanceof TypeError && it.next().done");
    throws("var g = function* () {}; g.prototype.next.call({});", "TypeError");
    throws("(function* () {}).prototype.next.call(1);", "TypeError");
}

#[test]
fn yield_star_delegates_next_throw_and_return() {
    truth(
        "function* g() { yield* [1, 2]; yield* 'ab'; } var r = []; var it = g(), s; while (!(s = it.next()).done) r.push(s.value); r.join() === '1,2,a,b'",
    );
    truth(
        "function* inner() { yield 1; return 'r'; } function* outer() { var v = yield* inner(); yield v; } var it = outer(); it.next().value === 1 && it.next().value === 'r'",
    );
    // The values `next` sends go to the delegate.
    truth(
        "function* inner() { var x = yield 1; yield x * 2; } function* outer() { yield* inner(); } var it = outer(); it.next(); it.next(21).value === 42",
    );
    truth(
        "var log = []; function* inner() { try { yield 1; } finally { log.push('inner'); } } function* outer() { try { yield* inner(); } finally { log.push('outer'); } } \
         var it = outer(); it.next(); var r = it.return(7); r.value === 7 && r.done && log.join() === 'inner,outer'",
    );
    truth(
        "function* inner() { try { yield 1; } catch (e) { yield 'in ' + e; } } function* outer() { yield* inner(); } var it = outer(); it.next(); it.throw('t').value === 'in t'",
    );
    // The delegate's own results are passed on as they are.
    truth(
        "var r = { done: false, value: 1, extra: 2 }; var o = {}; o[Symbol.iterator] = function () { return { next: function () { return r; } }; }; function* g() { yield* o; } g().next() === r",
    );
    // A delegate without `throw` is closed, and the generator gets a TypeError.
    truth(
        "var closed = false; var o = {}; o[Symbol.iterator] = function () { return { next: function () { return { done: false }; }, return: function () { closed = true; return {}; } }; }; \
         function* g() { yield* o; } var it = g(); it.next(); var e; try { it.throw(1); } catch (x) { e = x; } e instanceof TypeError && closed",
    );
    throws("function* g() { yield* 5; } g().next();", "TypeError");
    throws(
        "var o = {}; o[Symbol.iterator] = function () { return { next: function () { return 1; } }; }; function* g() { yield* o; } g().next();",
        "TypeError",
    );
}

#[test]
fn generator_methods_and_the_generator_objects() {
    truth("var o = { *g(a) { yield this === o && a; } }; o.g(5).next().value === 5");
    truth("var o = { *'s'() { yield 1; }, *2() { yield 2; } }; o.s().next().value + o[2]().next().value === 3");
    throws("function* g() {} new g();", "TypeError");
    throws("var o = { *m() {} }; new o.m();", "TypeError");
    // A generator method has a `prototype` that its generator objects inherit from, unlike other
    // methods, and takes a computed name and the method's `name`.
    truth("var o = { *m() {} }; var it = o.m(); Object.getPrototypeOf(it) === o.m.prototype && !o.m.prototype.hasOwnProperty('constructor')");
    truth("var k = 'c'; var o = { *[k + 'd']() { yield 4; } }; o.cd().next().value === 4 && o.cd.name === 'cd'");
    truth("function* g() {} var GP = Object.getPrototypeOf(g); GP === Object.getPrototypeOf(function* () {}) && GP !== Function.prototype");
    truth("function* g() {} Object.getPrototypeOf(g.prototype) === Object.getPrototypeOf(g).prototype && !g.prototype.hasOwnProperty('constructor')");
    truth("function* g() {} var it = g(); it[Symbol.iterator]() === it && Object.prototype.toString.call(it) === '[object Generator]'");
    truth("function* g() {} Object.prototype.toString.call(g) === '[object GeneratorFunction]' && g instanceof Function && typeof g === 'function'");
    // %GeneratorFunction.prototype% is an ordinary object, not a function (section 25.2.3).
    truth("var GP = Object.getPrototypeOf(function* () {}); typeof GP === 'object' && Object.getPrototypeOf(GP) === Function.prototype");
    throws("Object.getPrototypeOf(function* () {})();", "TypeError");
    truth(
        "var GF = Object.getPrototypeOf(function* () {}).constructor; var f = GF('a', 'yield a; yield a * 2;'); var it = f(3); it.next().value + it.next().value === 9",
    );
    truth(
        "var GF = Object.getPrototypeOf(function* () {}).constructor; Object.getPrototypeOf(GF) === Function && GF.prototype === Object.getPrototypeOf(function* () {})",
    );
    truth(
        "var GF = Object.getPrototypeOf(function* () {}).constructor; typeof this.GeneratorFunction === 'undefined' && new GF('yield 1')().next().value === 1",
    );
    truth("function* g() { yield 1 } String(g) === 'function* g() { yield 1 }'");
    // A generator function's own `prototype` is used when it is an object.
    truth("function* g() {} var p = { mark: 1 }; g.prototype = p; Object.getPrototypeOf(g()) === p");
    truth("function* g() {} g.prototype = 1; Object.getPrototypeOf(g()) === Object.getPrototypeOf(function* () {}).prototype");
}

#[test]
fn a_bound_generator_function_inherits_from_the_targets_prototype() {
    truth("function* g() {} var b = g.bind(null); Object.getPrototypeOf(b) === Object.getPrototypeOf(g) && b.constructor !== Function");
    truth("function* g(a) { yield a; } g.bind(null, 3)().next().value === 3");
    truth("function f() {} Object.getPrototypeOf(f.bind(null)) === Function.prototype");
}

#[test]
fn yield_is_an_identifier_outside_generators_in_sloppy_code() {
    truth("var yield = 1; yield + 1 === 2");
    truth("function f(yield) { return yield; } f(3) === 3");
    truth("function* g() { function f() { var yield = 4; return yield; } yield f(); } g().next().value === 4");
    truth("function* g() { var o = { yield: 5 }; yield o.yield; } g().next().value === 5");
    truth("function* yield() { yield 6; } yield().next().value === 6");
    syntax("'use strict'; var yield = 1;");
}

#[test]
fn bad_generators_are_syntax_errors() {
    for src in [
        "function* g() { var yield; }",
        "function* g(yield) {}",
        "function* g(a = yield) {}",
        "function* g() { function* h(a = yield 1) {} }",
        "var f = function* yield() {};",
        "function* g() { (yield) => 1; }",
        "function* g() { yield = 1; }",
        "function* g() { yi\\u0065ld 1; }",
        "function* g() { yield\n* 1; }",
        "function* g() { yield: 1; }",
        "if (true) function* g() {}",
        "L: function* g() {}",
        "{ function* g() {} function* g() {} }",
        "function* g() { yield* ; }",
        "var o = { *m(a, a) {} };",
        "function* g() { (a = yield) => 1; }",
    ] {
        syntax(src);
    }
    // A `yield` in the body of a function in a generator's parameters belongs to that function.
    truth("function* g(a = function* () { yield 1; }) { yield* a(); } g().next().value === 1");
    truth("function* g(a = function* () { yield 2; }, b = (c = function* () { yield 3; }) => c) { yield* b()(); } g().next().value === 3");
}

#[test]
fn realms_without_es2015_have_no_generators() {
    let es5 = Options::default();
    let extendscript = Options { e4x: true, preprocessor_lines: true, ..Options::default() };
    for options in [es5, extendscript] {
        for src in ["function* g() {}", "var g = function* () {};", "var o = { *m() {} };"] {
            syntax_with(src, options.clone());
        }
        let mut realm = Realm::with_options(options, Limits::default());
        assert_eq!(realm.run("function f() { var yield = 2; return yield; } f()"), Ok(JsValue::Number(2.0)));
        // Reading the syntax later does not add the built-ins, so a call is a TypeError.
        realm.set_es2015(true);
        match realm.run("function* g() { yield 1; } g();") {
            Err(RunError { kind: RunErrorKind::Thrown, message, .. }) => assert!(message.starts_with("TypeError"), "{message}"),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn a_suspended_generator_keeps_its_frame_across_a_collection() {
    let mut realm = es2015(Limits::default());
    let setup = "function* g() { var local = { v: 'left' }; var r = [{ w: 'held' }, yield 1]; yield local.v + r[0].w + r[1]; } \
                 var it = g(); it.next(); it.next('!').value";
    assert_eq!(realm.run(setup), Ok(JsValue::String("leftheld!".into())));
    // The operands on the stack at the `yield` (the array being built) survive a collection.
    assert_eq!(realm.run("var it3 = g(); it3.next(); 'ok'"), Ok(JsValue::String("ok".into())));
    realm.collect_garbage();
    assert_eq!(realm.run("it3.next('#').value"), Ok(JsValue::String("leftheld#".into())));
}

#[test]
fn a_frame_taken_out_of_its_generator_leaves_the_heap_total() {
    let mut realm = es2015(Limits::default());
    // At the `yield`, the 500 arguments before it are operands in the saved frame.
    let args = vec!["0"; 500].join(", ");
    let setup = format!("function f() {{}} function* g() {{ while (true) f({args}, yield 1); }} var it = g(); it.next(); 0");
    assert_eq!(realm.run(&setup), Ok(JsValue::Number(0.0)));
    let mut gaps = Vec::new();
    for resumes in [10, 210] {
        realm.collect_garbage();
        assert_eq!(realm.run(&format!("for (var i = 0; i < {resumes}; i++) it.next(); 0")), Ok(JsValue::Number(0.0)));
        let counted = realm.interp().heap.bytes;
        gaps.push(counted.saturating_sub(realm.collect_garbage()));
    }
    // 200 more resumptions leave far less than one saved frame's worth each.
    assert!(gaps[1] < gaps[0] + 200 * 1024, "{gaps:?}");
}

#[test]
fn an_unreachable_suspended_generator_is_freed() {
    let mut realm = es2015(Limits::default());
    let setup = "function* g() { var big = []; for (var i = 0; i < 20000; i++) big.push({ i: i }); yield big.length; } var it = g(); it.next().value";
    assert_eq!(realm.run(setup), Ok(JsValue::Number(20000.0)));
    let held = realm.collect_garbage();
    assert_eq!(realm.run("it = null; 0"), Ok(JsValue::Number(0.0)));
    let freed = realm.collect_garbage();
    assert!(freed + 1_000_000 < held, "held {held}, after {freed}");
}

/// The smallest heap limit, to within 256 bytes, under which `script` runs after `setup`.
fn smallest_heap_limit(setup: &str, script: &str) -> usize {
    let fits = |limit: usize| {
        let mut realm = es2015(Limits::default());
        assert_eq!(realm.run(setup), Ok(JsValue::Number(0.0)));
        realm.collect_garbage();
        realm.set_limits(Limits { heap_bytes: limit, ..Limits::default() });
        realm.run(script).is_ok()
    };
    let (mut low, mut high) = (0, 64 << 20);
    assert!(fits(high), "{script}");
    while high - low > 256 {
        let mid = low + (high - low) / 2;
        if fits(mid) {
            high = mid;
        } else {
            low = mid;
        }
    }
    high
}

#[test]
fn a_yield_reserves_only_what_its_frame_keeps() {
    // At the `yield` in `inside`, the 1,001 frames under the generator's are in `finally` blocks
    // that a throw entered. Their entries stay with those frames, so the yield needs no more room
    // than the one in `before`, which comes before those frames are pushed.
    let setup = "function* g() { yield 1; } var it = g(); 0";
    let inside = "function f(n) { try { throw 0; } finally { if (n) f(n - 1); else it.next(); } } try { f(1000); } catch (e) {} 0";
    let before = "it.next(); function f(n) { try { throw 0; } finally { if (n) f(n - 1); } } try { f(1000); } catch (e) {} 0";
    let (inside, before) = (smallest_heap_limit(setup, inside), smallest_heap_limit(setup, before));
    assert!(inside < before + 4096, "inside {inside}, before {before}");
}

#[test]
fn generators_stay_within_the_limits() {
    let mut realm = es2015(Limits { fuel: 20_000, ..Limits::default() });
    match realm.run("function* g() { while (true) yield 1; } var it = g(); while (true) it.next();") {
        Err(RunError { kind: RunErrorKind::Limit(LimitKind::Fuel), .. }) => {}
        other => panic!("{other:?}"),
    }
    // Each resumption takes fuel even when the generator has completed: 1,000 calls of its `next`
    // take at least 1,000 more than the same calls of a native method that takes none.
    let mut realm = es2015(Limits::default());
    let setup = "var it = (function* () {})(); it.next(); var control = { next: Object.prototype.valueOf }; 0";
    assert_eq!(realm.run(setup), Ok(JsValue::Number(0.0)));
    let mut used = Vec::new();
    for callee in ["control", "it"] {
        assert_eq!(realm.run(&format!("for (var i = 0; i < 1000; i++) {callee}.next(); 0")), Ok(JsValue::Number(0.0)));
        used.push(Limits::default().fuel - realm.fuel_left());
    }
    assert!(used[1] >= used[0] + 1000, "{used:?}");
    // A deep `yield*` chain stops at the frame limit, and the realm can run again.
    let mut realm = es2015(Limits { frames: 200, ..Limits::default() });
    match realm.run("function* d(n) { if (n) yield* d(n - 1); else yield 1; } d(1000).next();") {
        Err(RunError { kind: RunErrorKind::Limit(LimitKind::Frames), .. }) => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(realm.run("function* d(n) { if (n) yield* d(n - 1); else yield 1; } d(20).next().value"), Ok(JsValue::Number(1.0)));
    // A generator whose body was running when a limit ended the run cannot be resumed.
    let mut realm = es2015(Limits { fuel: 20_000, ..Limits::default() });
    match realm.run("var it = (function* () { while (true) {} })(); it.next();") {
        Err(RunError { kind: RunErrorKind::Limit(LimitKind::Fuel), .. }) => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(realm.run("var r = it.next(); r.done && r.value === undefined"), Ok(JsValue::Bool(true)));
}
