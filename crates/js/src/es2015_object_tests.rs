//! Object literals with computed names, shorthand properties and methods, and the `name` and
//! `length` of functions, with [`Options::es2015`].

use proptest::prelude::*;

use crate::{JsValue, LimitKind, Limits, Options, Realm, RunError, RunErrorKind};

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

/// Runs `src` with a collection at every heap reservation, and checks that it gives `true`.
fn truth_collecting(src: &str) {
    let mut realm = es();
    crate::interp::COLLECT_ON_RESERVE.with(|c| c.set(true));
    let r = realm.run(src);
    crate::interp::COLLECT_ON_RESERVE.with(|c| c.set(false));
    assert_eq!(r, Ok(JsValue::Bool(true)), "{src}");
    assert!(realm.interp().heap.collections > 0, "{src}");
}

/// A script that checks that `f`'s own `name` is `value`, not writable, not enumerable and
/// configurable.
fn name_is(f: &str, value: &str) -> String {
    format!(
        "(function (f) {{ var d = Object.getOwnPropertyDescriptor(f, 'name'); \
         return !!d && d.value === '{value}' && !d.writable && !d.enumerable && d.configurable; }})({f})"
    )
}

// ---- object literals ----

#[test]
fn a_shorthand_property_reads_its_binding() {
    truth("var a = 1, get = 2, set = 3; var o = { a, get, set }; o.a === 1 && o.get === 2 && o.set === 3");
    truth("var a = 1; var d = Object.getOwnPropertyDescriptor({ a }, 'a'); d.writable && d.enumerable && d.configurable");
    truth("let x = 'x'; const y = 'y'; var o = { x, y, z: 3 }; Object.keys(o).join() === 'x,y,z'");
    throws("({ missing })", "ReferenceError");
    throws("({ later }); let later = 1;", "ReferenceError");
}

#[test]
fn methods_are_enumerable_functions_that_cannot_be_constructed() {
    truth(
        "var o = { k: 1, m(x) { return x + this.k; }, get() { return 'g'; }, set(v) { return v; } }; o.m(1) === 2 && o.get() === 'g' && o.set(4) === 4",
    );
    truth("var o = { m() {} }; var d = Object.getOwnPropertyDescriptor(o, 'm'); d.enumerable && d.writable && d.configurable");
    truth("var o = { m() {} }; !o.m.hasOwnProperty('prototype') && o.m.length === 0");
    truth("var o = { m(a, b = 1, ...r) { return arguments.length; } }; o.m.length === 1 && o.m(1, 2, 3) === 3");
    throws("var o = { m() {} }; new o.m();", "TypeError");
    throws("var o = { get g() { return 1; } }; new (Object.getOwnPropertyDescriptor(o, 'g').get)();", "TypeError");
    syntax("({ m(a, a) {} })");
    syntax("({ m() { 'use strict'; } , n(a = 1) { 'use strict'; } })");
}

#[test]
fn computed_names_are_evaluated_and_converted_in_order() {
    truth("var k = 'a'; var o = { [k + 'b']: 1, [1 + 1]: 2, [k]() { return 3; } }; o.ab === 1 && o[2] === 2 && o.a() === 3");
    truth("var v = 0; var o = { get ['x' + 1]() { return v; }, set ['x' + 1](n) { v = n; } }; o.x1 = 5; o.x1 === 5");
    truth("var o = { [{ toString() { return 'p'; } }]: 1 }; o.p === 1");
    // Each name is converted to a property name before its value is evaluated, entry by entry.
    truth(
        "var log = []; var key = (n) => ({ toString() { log.push('key' + n); return 'k' + n; } }); \
         var val = (n) => { log.push('value' + n); return n; }; \
         var o = { [key(1)]: val(1), [key(2)]: val(2) }; \
         log.join() === 'key1,value1,key2,value2' && o.k1 === 1 && o.k2 === 2",
    );
    // A later definition of the same name replaces the earlier one.
    truth("var o = { a: 1, ['a']: 2 }; o.a === 2 && Object.keys(o).length === 1");
    truth("var o = { get ['a']() { return 1; }, a: 2 }; o.a === 2");
}

#[test]
fn e4x_values_as_computed_names_become_property_names() {
    let options = Options { es2015: true, e4x: true, ..Options::default() };
    let mut realm = Realm::with_options(options, Limits::default());
    let src = "var o; with (<x a='k' b='m'/>) { o = { [@a]: 1, [@b]() { return 2; }, [<y>n</y>]: 3 }; } \
               Object.keys(o).join() === 'k,m,n' && o.k === 1 && o.m() === 2 && o.m.name === 'm'";
    assert_eq!(realm.run(src), Ok(JsValue::Bool(true)));
}

#[test]
fn a_computed_name_that_throws_stops_the_literal() {
    truth(
        "var log = []; try { ({ a: log.push('a'), [(() => { throw new Error('k'); })()]: log.push('b'), c: log.push('c') }); } \
         catch (e) { log.push(e.message); } log.join() === 'a,k'",
    );
    throws("({ [{ toString() { throw new TypeError('no name'); } }]: 1 })", "TypeError: no name");
    throws("({ [undefinedName]: 1 })", "ReferenceError");
}

#[test]
fn bad_object_literal_forms_are_syntax_errors() {
    syntax("({ m() {} } = {});");
    syntax("var a; [{ m() {} }] = [];");
    syntax("for ({ m() {} } of []) ;");
    for src in ["({ if })", "({ this })", "({ 1 })", "({ 'a' })", "({ [a] })", "({ [a }: 1 })", "({ get [a]: 1 })", "({ a() })"] {
        syntax(src);
    }
    syntax("'use strict'; ({ yield })");
    syntax("({ \\u0069f })");
    syntax("({ get [a](x) {} })");
    syntax("({ set [a]() {} })");
}

#[test]
fn proto_sets_the_prototype_once() {
    truth("var p = { x: 1 }; var o = { __proto__: p }; Object.getPrototypeOf(o) === p && !o.hasOwnProperty('__proto__') && o.x === 1");
    truth("var o = { '__proto__': null }; Object.getPrototypeOf(o) === null");
    truth("var o = { __proto__: 1 }; Object.getPrototypeOf(o) === Object.prototype && !o.hasOwnProperty('__proto__')");
    truth("var o = { ['__proto__']: 1 }; o.hasOwnProperty('__proto__') && Object.getPrototypeOf(o) === Object.prototype");
    truth("var __proto__ = 2; var o = { __proto__ }; o.hasOwnProperty('__proto__') && Object.getPrototypeOf(o) === Object.prototype");
    truth("var o = { __proto__() { return 3; } }; o.hasOwnProperty('__proto__') && o.__proto__() === 3");
    truth("var __proto__ = 2; var o = { __proto__: null, ['__proto__']: 1, __proto__ }; Object.getPrototypeOf(o) === null");
    syntax("({ __proto__: null, __proto__: null })");
    syntax("({ __proto__: null, '__proto__': null })");
}

#[test]
fn a_destructuring_pattern_may_name_proto_twice() {
    // The value read is an own property named `__proto__`.
    truth("var v = { ['__proto__']: 1 }; var a, b; ({ __proto__: a, __proto__: b } = v); a === 1 && b === 1");
    truth("var v = { ['__proto__']: 1 }; var a, b; [{ __proto__: a, __proto__: b }] = [v]; a === 1 && b === 1");
    truth("var f = ({ __proto__: a, __proto__: b }) => a + b; f({ ['__proto__']: 1 }) === 2");
    truth("var r; for ({ __proto__: r, __proto__: r } of [{ ['__proto__']: 3 }]) ; r === 3");
    truth("var { __proto__: x, __proto__: y } = { ['__proto__']: 4 }; x === 4 && y === 4");
    // The same literal is still an error where it stays a literal, nested or not.
    for src in [
        "var a, b; ({ __proto__: a, __proto__: b });",
        "[{ __proto__: 1, __proto__: 2 }];",
        "var b; ({ a: b = { __proto__: 1, __proto__: 2 } } = {});",
        "var b; ({ [{ __proto__: 1, __proto__: 2 }]: b } = {});",
        "var a; ({ __proto__: a, __proto__: a } = { b: { __proto__: 1, __proto__: 2 } });",
    ] {
        match es().run(src) {
            Err(RunError { kind: RunErrorKind::Syntax, message, .. }) => assert!(message.contains("'__proto__' twice"), "{src}: {message}"),
            other => panic!("{src}: {other:?}"),
        }
    }
}

#[test]
fn a_wide_pattern_may_repeat_proto_with_defaults_inside() {
    let props = vec!["__proto__: { x = 0 }"; 2_000].join(", ");
    truth(&format!("var x; ({{ {props} }} = {{ ['__proto__']: {{}} }}); x === 0"));
    // In a literal, the first of the errors in source order is the one reported.
    for (src, twice) in [
        ("({ __proto__: 1, __proto__: 2, a: { b = 1 }, __proto__: 3 });", true),
        ("({ a: { b = 1 }, __proto__: 1, __proto__: 2 });", false),
        ("({ __proto__: 1, a: { b = 1 }, __proto__: 2 });", false),
    ] {
        let e = es().run(src).unwrap_err();
        assert_eq!((&e.kind, e.message.contains("'__proto__' twice")), (&RunErrorKind::Syntax, twice), "{src}: {e:?}");
    }
}

#[test]
fn computed_names_and_methods_stay_reachable_while_they_are_defined() {
    truth_collecting(
        "var o = { ['a' + 1]: [1], ['b' + 2]() { return 2; }, get ['c' + 3]() { return 3; } }; o.a1[0] === 1 && o.b2() === 2 && o.c3 === 3",
    );
    truth_collecting("var o = { ['m' + 1]: () => 1 }; o.m1.name === 'm1'");
    truth_collecting("function t() {} var b = t.bind(); b.name === 'bound t'");
}

#[test]
fn computed_names_stay_within_the_limits() {
    let mut realm = Realm::with_options(es_options(), Limits { fuel: 10_000, ..Limits::default() });
    let e = realm.run("({ [{ toString() { for (;;) {} } }]: 1 })").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Fuel), "{e:?}");
    let mut realm = Realm::with_options(es_options(), Limits { heap_bytes: 4 << 20, ..Limits::default() });
    let e = realm.run("var s = 'x'; for (var i = 0; i < 20; i++) s += s; ({ get [s]() {} })").unwrap_err();
    assert!(matches!(e.kind, RunErrorKind::Limit(_)), "{e:?}");
}

/// The smallest heap limit at which a new ECMAScript 2015 realm runs `src` without an error.
fn smallest_heap(src: &str) -> usize {
    let runs = |heap_bytes: usize| Realm::with_options(es_options(), Limits { heap_bytes, ..Limits::default() }).run(src).is_ok();
    let (mut lo, mut hi) = (0usize, 64usize << 20);
    assert!(runs(hi), "{src}");
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if runs(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

#[test]
fn a_repeated_computed_name_needs_no_more_heap_than_one() {
    // The two scripts differ only in `twice`, so they compile to code of the same size.
    let script = |twice: u8| {
        format!(
            "var twice = {twice}; var k = 'x'; for (var i = 0; i < 16; i++) k += k; \
             var o = twice ? {{ [k]: 1, [k]: 2 }} : {{ [k]: 1 }}; o[k] === 1 + twice"
        )
    };
    let once = smallest_heap(&script(0));
    let mut realm = Realm::with_options(es_options(), Limits { heap_bytes: once, ..Limits::default() });
    assert_eq!(realm.run(&script(1)), Ok(JsValue::Bool(true)));
}

#[test]
fn naming_a_function_needs_no_more_heap_than_making_it() {
    let script = |named: u8| format!("var named = {named}, f, g; if (named) f = function () {{}}; else g = (0, function () {{}}); true");
    let (unnamed, named) = (smallest_heap(&script(0)), smallest_heap(&script(1)));
    // A new property takes at least `PROP_BYTES`; giving the made function its name replaces one.
    assert!(named < unnamed + crate::heap::PROP_BYTES, "{named} {unnamed}");
}

#[test]
fn bind_stays_within_the_heap_limit() {
    for options in [Options::default(), es_options()] {
        // The bytes a second run that binds a function adds, measured without a limit.
        let ready = || {
            let mut realm = Realm::with_options(options.clone(), Limits::default());
            assert_eq!(realm.run("function t() {} var b; function go() { b = t.bind(); return true; } true"), Ok(JsValue::Bool(true)));
            realm
        };
        let mut realm = ready();
        let before = realm.interp().heap.bytes;
        assert_eq!(realm.run("go()"), Ok(JsValue::Bool(true)));
        let added = realm.interp().heap.bytes - before;
        // With one byte less than that, the run ends at the limit, or a collection makes room,
        // and the heap does not pass the limit either way.
        let mut realm = ready();
        let limit = realm.interp().heap.bytes + added - 1;
        realm.set_limits(Limits { heap_bytes: limit, ..Limits::default() });
        let r = realm.run("go()");
        let used = realm.interp().heap.bytes;
        assert!(r.is_ok() || matches!(&r, Err(e) if e.kind == RunErrorKind::Limit(LimitKind::Heap)), "{options:?}: {r:?}");
        assert!(used <= limit, "{options:?}: {used} of {limit}");
    }
}

#[test]
fn computed_keys_stay_within_the_heap_limit() {
    let setup = "var ks = [], ss = [], out; for (var i = 0; i < 64; i++) { ks.push('key' + i); ss.push(Symbol()); } true";
    let literal = |keys: &str| (0..64).map(|i| format!("[{keys}[{i}]]: {i}")).collect::<Vec<_>>().join(", ");
    for go in [
        format!("function go() {{ out = {{ {} }}; return true; }} true", literal("ks")),
        format!("function go() {{ out = {{ {} }}; return true; }} true", literal("ss")),
    ] {
        // The bytes the object a run of `go` keeps in `out` holds, measured without a limit.
        let ready = || {
            let mut realm = es();
            assert_eq!(realm.run(setup), Ok(JsValue::Bool(true)));
            assert_eq!(realm.run(&go), Ok(JsValue::Bool(true)));
            realm.collect_garbage();
            realm
        };
        let mut realm = ready();
        let before = realm.interp().heap.bytes;
        assert_eq!(realm.run("go()"), Ok(JsValue::Bool(true)));
        realm.collect_garbage();
        let kept = realm.interp().heap.bytes - before;
        // With one byte less than that, the run ends at the limit and the heap stays within it.
        let mut realm = ready();
        let limit = realm.interp().heap.bytes + kept - 1;
        realm.set_limits(Limits { heap_bytes: limit, ..Limits::default() });
        let e = realm.run("go()").unwrap_err();
        let used = realm.interp().heap.bytes;
        assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{e:?}");
        assert!(used <= limit, "{used} of {limit}");
    }
}

#[test]
fn a_built_in_made_after_the_names_are_set_checks_the_heap_for_its_name() {
    fn nothing(
        _: &mut crate::interp::Interp,
        _: crate::value::Value,
        _: &[crate::value::Value],
        _: bool,
    ) -> crate::interp::JsResult<crate::value::Value> {
        Ok(crate::value::Value::Undefined)
    }
    let mut realm = es();
    assert_eq!(realm.run("Math.max.name === 'max'"), Ok(JsValue::Bool(true)));
    let limit = realm.collect_garbage();
    realm.set_limits(Limits { heap_bytes: limit, ..Limits::default() });
    let made = crate::builtins::native(realm.interp(), "later", nothing, 0, crate::heap::NativeKind::Plain, false);
    assert!(matches!(made, Err(crate::interp::Abrupt::Limit(LimitKind::Heap))), "{}", made.is_ok());
}

#[test]
fn built_ins_no_script_can_reach_need_no_room_for_names() {
    let drop = "[Math, String.prototype, Array.prototype].forEach(function (o) { \
                Object.getOwnPropertyNames(o).forEach(function (k) { if (k !== 'constructor') delete o[k]; }); }); true";
    // The heap the first ECMAScript 2015 run needs once the dropped built-ins are collected.
    let mut realm = Realm::new();
    assert_eq!(realm.run(drop), Ok(JsValue::Bool(true)));
    realm.collect_garbage();
    realm.set_es2015(true);
    assert_eq!(realm.run("true"), Ok(JsValue::Bool(true)));
    // 4 KiB more covers what compiling the script reserves; the dropped functions' names would
    // need several times that.
    let limit = realm.interp().heap.bytes + 4096;
    let mut realm = Realm::new();
    assert_eq!(realm.run(drop), Ok(JsValue::Bool(true)));
    realm.set_limits(Limits { heap_bytes: limit, ..Limits::default() });
    realm.set_es2015(true);
    assert_eq!(realm.run("true"), Ok(JsValue::Bool(true)));
}

#[test]
fn built_in_names_do_not_follow_what_earlier_scripts_did_with_them() {
    let mut realm = Realm::new();
    let setup = "Object.defineProperty(this, 'max', { get: Math.max, configurable: true }); \
                 Object.defineProperty(Math.min, 'name', { value: 'mine', configurable: true }); true";
    assert_eq!(realm.run(setup), Ok(JsValue::Bool(true)));
    realm.set_es2015(true);
    let check = "Math.max.name === 'max' && Math.min.name === 'mine' && \
                 Object.getOwnPropertyDescriptor(Math.min, 'length').configurable";
    assert_eq!(realm.run(check), Ok(JsValue::Bool(true)));
}

#[test]
fn frozen_built_ins_keep_their_properties_when_the_names_are_set() {
    let mut realm = Realm::new();
    assert_eq!(realm.run("var f = Object.freeze(Math.max), g = Object.preventExtensions(Math.min); true"), Ok(JsValue::Bool(true)));
    realm.set_es2015(true);
    let check = "f === Math.max && Object.isFrozen(f) && !f.hasOwnProperty('name') && \
                 !Object.getOwnPropertyDescriptor(f, 'length').configurable && \
                 !g.hasOwnProperty('name') && !Object.getOwnPropertyDescriptor(g, 'length').configurable && \
                 Math.abs.name === 'abs'";
    assert_eq!(realm.run(check), Ok(JsValue::Bool(true)));
    realm.set_es2015(false);
    assert_eq!(realm.run("Object.isFrozen(Math.max) && !Math.max.hasOwnProperty('name')"), Ok(JsValue::Bool(true)));
}

#[test]
fn built_in_names_that_do_not_fit_the_heap_end_the_run_and_wait_for_room() {
    let mut realm = Realm::new();
    let used = realm.interp().heap.bytes;
    realm.set_limits(Limits { heap_bytes: used + 4096, ..Limits::default() });
    realm.set_es2015(true);
    let e = realm.run("1").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{e:?}");
    assert!(realm.interp().heap.bytes <= used + 4096);
    realm.set_limits(Limits::default());
    assert_eq!(realm.run("Math.max.name === 'max' && Function.prototype.name === ''"), Ok(JsValue::Bool(true)));
}

#[test]
fn a_new_realm_names_the_built_ins_under_its_own_heap_limit() {
    let used = es().interp().heap.bytes;
    let limits = Limits { heap_bytes: used + 4096, ..Limits::default() };
    let mut realm = Realm::with_options(es_options(), limits);
    assert_eq!(realm.interp().heap.bytes, used);
    let e = realm.run("1").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Heap), "{e:?}");
    assert!(realm.interp().heap.bytes <= used + 4096);
    realm.set_limits(Limits::default());
    assert_eq!(realm.run("Math.max.name === 'max' && Function.prototype.name === ''"), Ok(JsValue::Bool(true)));
}

// ---- function names ----

#[test]
fn declarations_and_expressions_have_their_own_name() {
    truth(&name_is("function () {}", ""));
    truth(&format!("function decl() {{}} {}", name_is("decl", "decl")));
    truth(&name_is("function expr() {}", "expr"));
    truth(&name_is("() => {}", ""));
    truth("var g = function inner() { return inner; }; g() === g && g.name === 'inner'");
    truth(&name_is("Function('a', 'return a')", "anonymous"));
    truth(&name_is("new Function()", "anonymous"));
    truth("Object.getOwnPropertyNames(function f() {}).slice(0, 2).join() === 'length,name'");
}

#[test]
fn an_anonymous_function_takes_the_name_it_is_first_bound_to() {
    truth(&format!("var v = function () {{}}; {}", name_is("v", "v")));
    truth(&format!("let l = () => {{}}; {}", name_is("l", "l")));
    truth(&format!("const c = (function () {{}}); {}", name_is("c", "c")));
    truth(&format!("var a; a = () => 1; {}", name_is("a", "a")));
    truth(&format!("var a, b; a = b = function () {{}}; {}", name_is("b", "b")));
    truth(&format!("function f(p = () => {{}}) {{ return p; }} {}", name_is("f()", "p")));
    truth("function f(p = () => {}) { return p; } var given = (0, function () {}); f(given) === given && given.name === ''");
    // A function that already has a name, a function in a comma expression, a parenthesized
    // target and a property target keep the name they have.
    truth("var n = function m() {}; n.name === 'm'");
    truth("var s = (0, function () {}); s.name === ''");
    truth("var t; (t) = function () {}; t.name === ''");
    truth("var o = {}; o.p = function () {}; o.p.name === ''");
    truth("var q; q += function () {}; typeof q === 'string'");
}

#[test]
fn object_literal_functions_are_named_after_their_property() {
    truth(&name_is("({ p: function () {} }).p", "p"));
    truth(&name_is("({ 'str key': () => {} })['str key']", "str key"));
    truth(&name_is("({ 1.5: () => {} })['1.5']", "1.5"));
    truth(&name_is("({ m() {} }).m", "m"));
    truth(&name_is("({ ['c' + 'k']: function () {} }).ck", "ck"));
    truth(&name_is("({ [1 + 1]() {} })[2]", "2"));
    truth(&name_is("Object.getOwnPropertyDescriptor({ get g() {} }, 'g').get", "get g"));
    truth(&name_is("Object.getOwnPropertyDescriptor({ set s(v) {} }, 's').set", "set s"));
    truth(&name_is("Object.getOwnPropertyDescriptor({ get ['x' + 1]() {} }, 'x1').get", "get x1"));
    truth(&name_is("Object.getOwnPropertyDescriptor({ set ['x' + 1](v) {} }, 'x1').set", "set x1"));
    truth("var o = { p: function named() {} }; o.p.name === 'named'");
    truth("var f = function () {}; var o = { f }; o.f.name === 'f' && ({ g: f }).g.name === 'f'");
    truth("var o = { xs: (0, function () {}) }; o.xs.name === ''");
}

#[test]
fn symbol_keys_name_functions_after_their_description() {
    truth(&format!("var s = Symbol('s'); var o = {{ [s]: () => 0 }}; {}", name_is("o[s]", "[s]")));
    truth(&format!("var s = Symbol(''); var o = {{ [s]() {{}} }}; {}", name_is("o[s]", "[]")));
    truth(&format!("var e = Symbol(); var o = {{ [e]: function () {{}} }}; {}", name_is("o[e]", "")));
    let accessors = |key: &str| format!("var k = {key}; var d = Object.getOwnPropertyDescriptor({{ get [k]() {{}}, set [k](v) {{}} }}, k);");
    truth(&format!("{} {} && {}", accessors("Symbol('a')"), name_is("d.get", "get [a]"), name_is("d.set", "set [a]")));
    truth(&format!("{} {} && {}", accessors("Symbol()"), name_is("d.get", "get "), name_is("d.set", "set ")));
    truth(&format!("var o = {{ [Symbol.iterator]() {{}} }}; {}", name_is("o[Symbol.iterator]", "[Symbol.iterator]")));
}

#[test]
fn a_default_value_takes_the_name_of_its_target() {
    truth(&format!("var {{ a = function () {{}} }} = {{}}; {}", name_is("a", "a")));
    truth(&format!("let [b = () => 0] = []; {}", name_is("b", "b")));
    truth(&format!("const {{ x: y = function () {{}} }} = {{}}; {}", name_is("y", "y")));
    truth(&format!("var c; ({{ c = function () {{}} }} = {{}}); {}", name_is("c", "c")));
    truth(&format!("var d; [d = () => 0] = []; {}", name_is("d", "d")));
    truth(&format!("var e; ({{ k: e = function () {{}} }} = {{}}); {}", name_is("e", "e")));
    truth(&format!("function f({{ p = () => 0 }}) {{ return p; }} {}", name_is("f({})", "p")));
    truth(&format!("var got; for (const [q = function () {{}}] of [[]]) got = q; {}", name_is("got", "q")));
    truth(&format!("var got; try {{ throw {{}}; }} catch ({{ r = () => 0 }}) {{ got = r; }} {}", name_is("got", "r")));
    // A parenthesized or property target, a function with its own name and a value that is
    // given keep the names they have.
    truth("var t; [(t) = function () {}] = []; t.name === ''");
    truth("var o = {}; [o.p = function () {}] = []; o.p.name === ''");
    truth("var { g = function h() {} } = {}; g.name === 'h'");
    truth("var given = (0, function () {}); var { k = function () {} } = { k: given }; k === given && k.name === ''");
}

#[test]
fn bound_and_built_in_functions_have_names() {
    truth(&name_is("function target() {}.bind()", "bound target"));
    truth(&name_is("function target() {}.bind().bind()", "bound bound target"));
    truth(&name_is("Object.defineProperty(function () {}, 'name', { value: 7 }).bind()", "bound "));
    throws("Object.defineProperty(function () {}, 'name', { get() { throw new RangeError('name'); } }).bind()", "RangeError");
    truth(&name_is("Array.prototype.map", "map"));
    truth(&name_is("Object", "Object"));
    truth(&name_is("Function.prototype", ""));
    truth(&name_is("Function.prototype.bind", "bind"));
    truth("Object.getOwnPropertyNames(Function).slice(0, 2).join() === 'length,name'");
}

#[test]
fn bind_reads_only_an_own_length() {
    truth(
        "var read = 0; function t(a, b) {} delete t.length; \
         Object.defineProperty(Function.prototype, 'length', { get: function () { read++; return 5; }, configurable: true }); \
         var b = t.bind(); read === 0 && b.length === 0",
    );
    truth("function t(a, b) {} t.bind(null, 1).length === 1");
}

#[test]
fn bind_charges_fuel_for_a_long_name() {
    let mut realm = es();
    let setup = "var s = 'x'; for (var i = 0; i < 20; i++) s += s; function t() {} function u() {} \
                 Object.defineProperty(t, 'name', { value: s }); true";
    assert_eq!(realm.run(setup), Ok(JsValue::Bool(true)));
    realm.set_limits(Limits { fuel: 10_000, ..Limits::default() });
    assert_eq!(realm.run("u.bind().name === 'bound u'"), Ok(JsValue::Bool(true)));
    let e = realm.run("t.bind(); true").unwrap_err();
    assert_eq!(e.kind, RunErrorKind::Limit(LimitKind::Fuel), "{e:?}");
}

#[test]
fn function_lengths_can_be_redefined() {
    truth("function f(a, b) {} delete f.length && !f.hasOwnProperty('length')");
    truth("var g = (a) => a; Object.defineProperty(g, 'length', { value: 5 }); g.length === 5");
    truth("function t(a, b) {} var b = t.bind(); Object.getOwnPropertyDescriptor(b, 'length').configurable");
    truth("delete Math.max.length && Math.max.hasOwnProperty('length') === false");
}

// ---- the language mode ----

#[test]
fn without_the_option_the_new_forms_are_syntax_errors() {
    for options in [Options::default(), Options { e4x: true, ..Options::default() }] {
        for src in ["var a = 1; ({ a })", "({ [1]: 2 })", "({ m() {} })", "({ get [a]() {} })", "({ a = 1 })"] {
            let r = Realm::with_options(options.clone(), Limits::default()).run(src);
            assert!(matches!(r, Err(RunError { kind: RunErrorKind::Syntax, .. })), "{src}: {r:?}");
        }
    }
}

#[test]
fn without_the_option_functions_keep_their_names_and_lengths() {
    for options in [Options::default(), Options { e4x: true, ..Options::default() }] {
        let mut realm = Realm::with_options(options, Limits::default());
        let src = "function f(a) {} var v = function () {}; var o = { p: function () {}, __proto__: null }; \
                   !f.hasOwnProperty('name') && !v.hasOwnProperty('name') && !o.p.hasOwnProperty('name') && \
                   !Object.hasOwnProperty('name') && !Function.prototype.bind.hasOwnProperty('name') && \
                   !f.bind().hasOwnProperty('name') && !(delete f.length) && f.hasOwnProperty('length') && \
                   !Object.getOwnPropertyDescriptor(Math.max, 'length').configurable && o.hasOwnProperty('__proto__') && \
                   ({ __proto__: 1, __proto__: 2 }).__proto__ === 2 && f.hasOwnProperty('prototype') && \
                   Object.getOwnPropertyDescriptor({ get g() {} }, 'g').get.hasOwnProperty('prototype')";
        assert_eq!(realm.run(src), Ok(JsValue::Bool(true)));
    }
}

#[test]
fn turning_the_option_on_names_the_built_ins_once() {
    let mut realm = Realm::new();
    assert_eq!(realm.run("Array.prototype.map.hasOwnProperty('name')"), Ok(JsValue::Bool(false)));
    realm.set_es2015(true);
    assert_eq!(
        realm.run("Array.prototype.map.name === 'map' && Object.getOwnPropertyDescriptor(Math.max, 'length').configurable"),
        Ok(JsValue::Bool(true))
    );
    // The built-ins keep the names after it is turned off; the code read then gets none.
    realm.set_es2015(false);
    assert_eq!(realm.run("Array.prototype.map.name === 'map' && !(function f() {}).hasOwnProperty('name')"), Ok(JsValue::Bool(true)));
}

// ---- robustness ----

const FRAGMENTS: &[&str] = &[
    "({",
    "})",
    "{",
    "}",
    "[",
    "]",
    "(",
    ")",
    ",",
    ":",
    ";",
    "a",
    "b",
    "get ",
    "set ",
    "__proto__",
    "'k'",
    "1",
    "=",
    "=>",
    "function ",
    "...",
    "this",
    "x",
    "+",
    " ",
    "\n",
    "'use strict';",
    "var ",
    "let ",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_object_literal_programs_never_panic(parts in proptest::collection::vec(0..FRAGMENTS.len(), 0..60)) {
        let src: String = parts.iter().map(|&i| FRAGMENTS[i]).collect();
        let mut realm = Realm::with_options(es_options(), Limits { fuel: 20_000, heap_bytes: 1 << 20, frames: 200, reentry: 8 });
        if let Err(e) = realm.run(&src) {
            prop_assert!(e.kind != RunErrorKind::Internal, "{src:?}: {e:?}");
        }
    }
}
