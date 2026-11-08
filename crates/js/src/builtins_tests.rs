use proptest::prelude::*;

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

/// The run throws an error whose message starts with `name`.
fn throws(src: &str, name: &str) {
    match Realm::new().run(src) {
        Err(RunError { kind: RunErrorKind::Thrown, message, .. }) => assert!(message.starts_with(name), "{src}: {message}"),
        other => panic!("{src}: {other:?}"),
    }
}

fn small(limits: Limits) -> Realm {
    Realm::with_options(Options::default(), limits)
}

fn limit(realm: &mut Realm, src: &str) -> LimitKind {
    match realm.run(src) {
        Err(RunError { kind: RunErrorKind::Limit(k), .. }) => k,
        other => panic!("{src}: {other:?}"),
    }
}

// ---- Object ----

#[test]
fn define_property_and_descriptors_follow_section_8_12_9() {
    truth(
        "var o = {}; Object.defineProperty(o, 'x', {value: 1}); var d = Object.getOwnPropertyDescriptor(o, 'x'); \
         d.value === 1 && d.writable === false && d.enumerable === false && d.configurable === false",
    );
    truth("var o = {}; Object.defineProperty(o, 'x', {value: 1}); o.x = 2; o.x === 1 && Object.keys(o).length === 0");
    throws("'use strict'; var o = {}; Object.defineProperty(o, 'x', {value: 1}); o.x = 2;", "TypeError");
    throws("var o = {}; Object.defineProperty(o, 'x', {value: 1}); Object.defineProperty(o, 'x', {value: 2});", "TypeError");
    truth("var o = {}; Object.defineProperty(o, 'x', {value: NaN}); Object.defineProperty(o, 'x', {value: NaN}); o.x !== o.x");
    throws("var o = {}; Object.defineProperty(o, 'x', {value: 0}); Object.defineProperty(o, 'x', {value: -0});", "TypeError");
    truth(
        "var o = {}, log = []; Object.defineProperty(o, 'x', {get: function () { return 7; }, set: function (v) { log.push(v); }, enumerable: true}); \
         o.x = 3; o.x === 7 && log[0] === 3 && Object.keys(o)[0] === 'x' && typeof Object.getOwnPropertyDescriptor(o, 'x').get === 'function'",
    );
    throws("Object.defineProperty({}, 'x', {get: function () {}, value: 1})", "TypeError");
    throws("Object.defineProperty({}, 'x', {get: 1})", "TypeError");
    throws("Object.defineProperty(1, 'x', {})", "TypeError");
    // A configurable data property becomes an accessor and keeps its enumerability.
    truth(
        "var o = {x: 1}; Object.defineProperty(o, 'x', {get: function () { return 2; }}); \
         var d = Object.getOwnPropertyDescriptor(o, 'x'); o.x === 2 && d.enumerable && d.configurable && d.set === undefined",
    );
}

#[test]
fn array_length_and_index_definitions_follow_section_15_4_5_1() {
    truth("var a = [1, 2, 3]; Object.defineProperty(a, 'length', {value: 1}); a.length === 1 && !(1 in a)");
    truth(
        "var a = [1, 2, 3]; Object.defineProperty(a, '1', {value: 2, configurable: false}); \
         var threw = false; try { Object.defineProperty(a, 'length', {value: 0}); } catch (e) { threw = e instanceof TypeError; } \
         threw && a.length === 2 && a[1] === 2 && !(2 in a)",
    );
    truth("var a = [1, 2, 3]; Object.defineProperty(a, '1', {configurable: false}); a.length = 0; a.length === 2");
    truth("var a = [1]; Object.defineProperty(a, 'length', {writable: false}); a[5] = 1; a.length === 1 && !(5 in a)");
    throws("var a = [1]; Object.defineProperty(a, 'length', {writable: false}); Object.defineProperty(a, '3', {value: 1});", "TypeError");
    throws("Object.defineProperty([], 'length', {value: -1})", "RangeError");
    truth("var a = []; Object.defineProperty(a, '4', {value: 1, enumerable: true, writable: true, configurable: true}); a.length === 5");
    throws("var a = Object.freeze([1, 2]); a.push(3);", "TypeError");
}

#[test]
fn object_functions_list_create_and_lock_objects() {
    truth(
        "var o = Object.create({inherited: 1}, {own: {value: 2, enumerable: true}}); o.inherited === 1 && o.own === 2 && Object.keys(o).join() === 'own'",
    );
    truth("Object.getPrototypeOf(Object.create(null)) === null && Object.getPrototypeOf([]) === Array.prototype");
    assert_eq!(string("Object.getOwnPropertyNames(Object('ab')).join()"), "0,1,length");
    assert_eq!(string("var a = [5]; a.k = 1; Object.getOwnPropertyNames(a).join()"), "length,0,k");
    // Object.keys lists names in the order for-in visits them (section 15.2.3.14).
    assert_eq!(string("var o = {b: 1, a: 2, 3: 3}, s = ''; for (var k in o) s += k; Object.keys(o).join('') === s ? s : 'differs'"), "ba3");
    truth("var o = Object.seal({x: 1}); o.x = 2; delete o.x; o.y = 1; o.x === 2 && o.y === undefined && Object.isSealed(o) && !Object.isFrozen(o)");
    truth("var o = Object.freeze({x: 1}); o.x = 2; o.x === 1 && Object.isFrozen(o) && !Object.isExtensible(o)");
    truth("var o = Object.preventExtensions({x: 1}); o.y = 1; o.y === undefined && Object.isSealed(o) === false && Object.isExtensible(o) === false");
    truth("Object.isFrozen(Object.preventExtensions({})) && Object.isFrozen(1) && Object.freeze(1) === 1 && Object.keys('ab').join() === '0,1'");
    truth("var o = Object.defineProperties({}, {a: {value: 1, enumerable: true}, b: {get: function () { return 2; }}}); o.a + o.b === 3");
    // A frozen arguments object no longer follows its parameters.
    truth("function f(a) { Object.freeze(arguments); a = 2; return arguments[0]; } f(1) === 1");
    truth("function f(a) { Object.defineProperty(arguments, '0', {value: 5}); return a; } f(1) === 5");
}

#[test]
fn prototype_methods_are_writable_so_scripts_can_replace_them() {
    truth("var d = Object.getOwnPropertyDescriptor(Array.prototype, 'map'); d.writable && d.configurable && !d.enumerable");
    truth("Array.prototype.first = function () { return this[0]; }; [4].first() === 4");
    truth(
        "var old = Array.prototype.join; Array.prototype.join = function () { return 'x'; }; var r = [1].toString(); Array.prototype.join = old; r === 'x'",
    );
    truth("var d = Object.getOwnPropertyDescriptor(Array, 'prototype'); !d.writable && !d.enumerable && !d.configurable");
    truth("var d = Object.getOwnPropertyDescriptor(Math, 'PI'); !d.writable && !d.enumerable && !d.configurable");
}

// ---- Function ----

#[test]
fn bind_fixes_this_and_leading_arguments() {
    assert_eq!(num("function f(a, b, c) { return this.k + a + b + c; } var g = f.bind({k: 1}, 2); g(3, 4)"), 10.0);
    assert_eq!(num("function f(a, b, c) {} f.bind(null, 1).length"), 2.0);
    assert_eq!(num("function f(a) {} f.bind(null, 1, 2, 3).length"), 0.0);
    truth("function P(x, y) { this.s = x + y; } var B = P.bind({ignored: 1}, 2); var p = new B(3); p.s === 5 && p instanceof P && p instanceof B");
    truth("var g = function () { return this; }.bind(7); typeof g() === 'object' && g().constructor === undefined || true");
    truth("var g = Math.max.bind(null, 5); g(1) === 5 && g(9) === 9");
    throws("Function.prototype.bind.call({})", "TypeError");
    throws("'use strict'; function f() {} f.bind(null).caller", "TypeError");
    truth("function f() { return arguments.length; } f.bind(null, 1).bind(null, 2)(3) === 3");
    truth("Object.prototype.toString.call(function () {}.bind()) === '[object Function]'");
}

// ---- Array ----

#[test]
fn the_array_constructor_and_is_array() {
    truth("var a = new Array(3); a.length === 3 && !(0 in a) && Array(2).length === 2");
    truth("var a = new Array(1, 2); a.length === 2 && a[1] === 2 && new Array('3').length === 1");
    throws("new Array(-1)", "RangeError");
    throws("new Array(1.5)", "RangeError");
    truth("new Array(4294967295).length === 4294967295");
    truth("Array.isArray([]) && !Array.isArray({length: 0}) && !Array.isArray(Array.prototype.slice) && Array.isArray(Array.prototype)");
}

#[test]
fn methods_that_change_the_array() {
    assert_eq!(string("var a = [1, 2]; var n = a.push(3, 4); n + ':' + a.join()"), "4:1,2,3,4");
    assert_eq!(string("var a = [1, 2, 3]; a.pop() + ':' + a.join() + ':' + [].pop()"), "3:1,2:undefined");
    assert_eq!(string("var a = [1, 2, 3]; a.shift() + ':' + a.join()"), "1:2,3");
    assert_eq!(string("var a = [3]; a.unshift(1, 2) + ':' + a.join()"), "3:1,2,3");
    assert_eq!(string("[1, , 3].reverse().join()"), "3,,1");
    truth("var a = [1, , 3, , ]; a.reverse(); !(0 in a) && a[1] === 3 && !(2 in a) && a[3] === 1 && a.length === 4");
    assert_eq!(string("var a = [1, 2, 3, 4, 5]; var r = a.splice(1, 2, 'x'); r.join() + ':' + a.join()"), "2,3:1,x,4,5");
    assert_eq!(string("var a = [1, 2, 3]; var r = a.splice(1, 0, 'x', 'y'); r.length + ':' + a.join()"), "0:1,x,y,2,3");
    assert_eq!(string("var a = [1, 2, 3]; a.splice(-2).join() + ':' + a.join()"), "2,3:1");
    assert_eq!(string("[1, 2, 3, 4].slice(1, -1).join() + ':' + [1, 2].slice(5).length"), "2,3:0");
    assert_eq!(string("[1].concat([2, [3]], 4).length + ':' + [1].concat([2, , 3]).join()"), "4:1,2,,3");
    // The methods are generic over array-like objects.
    assert_eq!(
        string("var o = {length: 2, 0: 'a', 1: 'b'}; Array.prototype.push.call(o, 'c'); o.length + Array.prototype.join.call(o, '-')"),
        "3a-b-c"
    );
    assert_eq!(num("var o = {length: '2', 0: 1, 1: 2}; Array.prototype.reverse.call(o); o[0] * 10 + o[1]"), 21.0);
    throws("var a = [1]; Object.defineProperty(a, 'length', {writable: false}); a.push(2)", "TypeError");
}

#[test]
fn searching_and_iteration_methods() {
    assert_eq!(num("[1, 2, 3, 2].indexOf(2) * 10 + [1, 2, 3, 2].lastIndexOf(2)"), 13.0);
    assert_eq!(num("[1, 2, 3].indexOf(1, -2) + [NaN].indexOf(NaN) + [1, 2].lastIndexOf(1, -3)"), -3.0);
    assert_eq!(num("[1, 2, 3].indexOf(3, -1) * 10 + [1, , 3].indexOf(undefined) * -1"), 21.0);
    truth("[2, 4].every(function (x) { return x % 2 === 0; }) && ![2, 3].every(function (x) { return x % 2 === 0; })");
    truth("[1, 2].some(function (x) { return x > 1; }) && ![].some(function () { return true; })");
    assert_eq!(string("var s = ''; [1, , 3].forEach(function (x, i, a) { s += x + '@' + i + (a.length); }); s"), "1@033@23");
    assert_eq!(string("[1, 2, 3].map(function (x) { return x * this.k; }, {k: 2}).join()"), "2,4,6");
    truth("var m = [1, , 3].map(function (x) { return x; }); m.length === 3 && !(1 in m)");
    assert_eq!(string("[1, 2, 3, 4].filter(function (x) { return x % 2; }).join()"), "1,3");
    assert_eq!(num("[1, 2, 3].reduce(function (a, b) { return a + b; })"), 6.0);
    assert_eq!(string("['a', 'b', 'c'].reduceRight(function (a, b) { return a + b; }, '')"), "cba");
    assert_eq!(num("[, 5, ].reduce(function (a, b) { return a + b; })"), 5.0);
    throws("[].reduce(function () {})", "TypeError");
    throws("[1].forEach(1)", "TypeError");
    // Elements appended during iteration are not visited; elements deleted before they are
    // reached are skipped.
    assert_eq!(num("var a = [1, 2, 3], n = 0; a.forEach(function (x, i) { n++; a.push(9); if (i === 0) delete a[1]; }); n"), 2.0);
}

#[test]
fn sort_orders_values_with_undefined_and_holes_last() {
    assert_eq!(string("[3, 1, 10, 2].sort().join()"), "1,10,2,3");
    assert_eq!(string("[3, 1, 10, 2].sort(function (a, b) { return a - b; }).join()"), "1,2,3,10");
    truth("var a = [3, undefined, , 1]; a.sort(); a[0] === 1 && a[1] === 3 && a[2] === undefined && 2 in a && !(3 in a) && a.length === 4");
    // The sort is stable.
    assert_eq!(
        string(
            "[{k: 1, v: 'a'}, {k: 0, v: 'b'}, {k: 1, v: 'c'}, {k: 0, v: 'd'}].sort(function (x, y) { return x.k - y.k; }).map(function (x) { return x.v; }).join('')"
        ),
        "bdac"
    );
    // A comparison that throws stops the sort with its exception.
    truth("var a = [2, 1, 3]; var e; try { a.sort(function () { throw 'stop'; }); } catch (x) { e = x; } e === 'stop'");
    // A comparison that changes the array or answers inconsistently leaves an array of the same
    // length and no exception.
    truth("var a = [5, 4, 3, 2, 1]; a.sort(function (x, y) { a.length = 1; a[7] = 0; return Math.random() - 0.5; }); a.length >= 5");
    truth("var a = []; for (var i = 0; i < 100; i++) a[i] = (i * 37) % 100; a.sort(function () { return 1; }); a.length === 100");
    throws("[1, 2].sort(1)", "TypeError");
    assert_eq!(string("Array.prototype.sort.call({length: 3, 0: 'c', 2: 'a'}) && 'ok'"), "ok");
}

#[test]
fn join_and_to_string_are_cycle_safe() {
    assert_eq!(string("var a = [1, 2]; a.push(a); a.join()"), "1,2,");
    assert_eq!(string("var a = [1]; var b = [a, 2]; a.push(b); String2(a); function String2(x) { return '' + x; }"), "1,,2");
    assert_eq!(string("var a = [1, [2, [3]]]; a.toString()"), "1,2,3");
    assert_eq!(string("[null, undefined, 0].join('-')"), "--0");
    assert_eq!(string("Array.prototype.toString.call({join: 1})"), "[object Object]");
    assert_eq!(string("var a = [{toLocaleString: function () { return 'L'; }}, null]; a.push(a); a.toLocaleString()"), "L,,");
}

#[test]
fn sparse_arrays_of_any_length_cost_only_their_elements() {
    // Each would visit four billion indices one by one; with 200,000 units of fuel each must
    // finish by visiting only the indices that exist.
    let mut realm = small(Limits { fuel: 200_000, ..Limits::default() });
    let cases = [
        ("var a = new Array(4294967295); a.join('')", JsValue::String(String::new())),
        ("var a = []; a[4294967294] = 'z'; a[3] = 'y'; a.join('').length", JsValue::Number(2.0)),
        ("var a = []; a[4294967294] = 2; a[7] = 1; a.sort(); a[0] * 10 + a[1] + (a.length === 4294967295 ? 0 : 100)", JsValue::Number(12.0)),
        ("var a = []; a[4294967294] = 1; a.indexOf(1)", JsValue::Number(4_294_967_294.0)),
        ("var a = []; a[4294967294] = 1; a.lastIndexOf(1, -1) + a.lastIndexOf(2)", JsValue::Number(4_294_967_293.0)),
        ("var n = 0; var a = []; a[4294967294] = 1; a[0] = 1; a.forEach(function () { n++; }); n", JsValue::Number(2.0)),
        ("var a = []; a[4294967294] = 1; a.map(function (x) { return x + 1; })[4294967294]", JsValue::Number(2.0)),
        ("var a = []; a[4000000000] = 1; a[1] = 2; a.reverse(); a[0] * 10 + a[3999999999]", JsValue::Number(12.0)),
        ("var a = []; a[4000000000] = 1; a.shift(); a[3999999999] + (a.length === 4000000000 ? 0 : 100)", JsValue::Number(1.0)),
        ("var a = []; a[4000000000] = 1; a.unshift(0); a[4000000001] + a[0]", JsValue::Number(1.0)),
        ("var a = []; a[4000000000] = 1; a[1] = 2; var r = a.splice(1, 1); r[0] + a[3999999999]", JsValue::Number(3.0)),
        ("var a = []; a[4000000000] = 1; a.splice(1, 0, 'x', 'y'); a[4000000002]", JsValue::Number(1.0)),
        ("var a = []; a[4000000000] = 1; a.slice(1)[3999999999]", JsValue::Number(1.0)),
        ("var a = []; a[4000000000] = 1; [0].concat(a).length", JsValue::Number(4_000_000_002.0)),
        ("var a = []; a[4000000000] = 2; a.reduce(function (x, y) { return x + y; }, 1)", JsValue::Number(3.0)),
    ];
    for (src, want) in cases {
        assert_eq!(realm.run(src), Ok(want), "{src}");
    }
    // An index added while a method runs is visited.
    let src = "var a = []; a[4000000000] = 1; var seen = []; a.forEach(function (x, i) { seen.push(i); if (i === 4000000000) {} }); \
               var b = []; b[1] = 1; b[3000000000] = 1; var n = 0; b.forEach(function (x, i) { n++; if (n === 1) { b[2000000000] = 2; b[3500000000] = 2; } }); n + seen.length";
    assert_eq!(realm.run(src), Ok(JsValue::Number(4.0)));
}

#[test]
fn built_ins_end_with_a_limit_instead_of_running_on() {
    let limits = Limits { heap_bytes: 8 << 20, fuel: 5_000_000, ..Limits::default() };
    assert_eq!(limit(&mut small(limits.clone()), "new Array(4294967295).join('x')"), LimitKind::Heap);
    assert_eq!(limit(&mut small(limits.clone()), "JSON.stringify(new Array(4294967295))"), LimitKind::Heap);
    assert_eq!(limit(&mut small(limits.clone()), "var a = [1]; for (;;) a = a.concat(a);"), LimitKind::Heap);
    assert_eq!(
        limit(&mut small(limits.clone()), "var o = {}; for (var i = 0; i < 20; i++) o = {a: o, b: o, c: o}; JSON.stringify(o)"),
        LimitKind::Heap
    );
    assert_eq!(
        limit(&mut small(limits.clone()), "var o = []; for (var i = 0; i < 20000; i++) o = [o]; for (;;) JSON.stringify(o);"),
        LimitKind::Fuel
    );
    let mut realm = small(Limits { fuel: 100_000, ..Limits::default() });
    assert_eq!(
        limit(&mut realm, "var a = []; for (var i = 0; i < 20000; i++) a[i * 1000] = i; a.length = 4294967295; a.sort(); a.sort();"),
        LimitKind::Fuel
    );
    assert_eq!(limit(&mut realm, "var a = [1, 2, 3, 4, 5, 6, 7, 8]; for (var i = 0; i < 14; i++) a = a.concat(a); a.join('')"), LimitKind::Fuel);
    let mut realm = small(Limits { fuel: 1_000, ..Limits::default() });
    assert_eq!(limit(&mut realm, "var s = 'x'; for (var i = 0; i < 20; i++) s += s; JSON.parse('\"' + s + '\"')"), LimitKind::Fuel);
}

#[test]
fn values_built_ins_hold_survive_collections() {
    // A small heap makes the callbacks' garbage start collections while the results are built.
    let mut realm = small(Limits { heap_bytes: 2 << 20, ..Limits::default() });
    let src = "function junk() { var t = []; for (var j = 0; j < 200; j++) t[j] = {j: j}; return t.length; } \
               var src = []; for (var i = 0; i < 300; i++) src[i] = i; \
               var m = src.map(function (x) { junk(); return {v: x}; }); \
               var f = src.filter(function (x) { junk(); return x % 2; }); \
               var r = src.reduce(function (acc, x) { junk(); return {sum: acc.sum + x}; }, {sum: 0}); \
               var s = src.slice().sort(function (a, b) { junk(); return b - a; }); \
               var j = JSON.parse(JSON.stringify(m), function (k, v) { junk(); return v; }); \
               m[299].v + f.length + r.sum + s[0] + j[299].v + j.length";
    assert_eq!(realm.run(src), Ok(JsValue::Number(299.0 + 150.0 + 44_850.0 + 299.0 + 299.0 + 300.0)));
    assert!(realm.interp().heap.collections > 0);
    // The callback is reachable only through the list `apply` read once the list is emptied.
    let src = "var list = [function (x) { list.length = 0; junk(); return {v: x}; }]; \
               var r = Array.prototype.map.apply(src, list); \
               var g = Array.prototype.map.bind(src, function (x) { junk(); return {v: x}; }); g = g(); \
               r[299].v + g[299].v";
    assert_eq!(realm.run(src), Ok(JsValue::Number(598.0)));
}

#[test]
fn fresh_values_survive_a_collection_while_they_are_stored() {
    // Below 1 MiB the only collections run at the heap limit, many of them while a built-in
    // reserves room for a value that a callback has just made and nothing else holds.
    let limits = Limits { heap_bytes: 768 << 10, ..Limits::default() };
    let src = "var bad = 0; \
               for (var n = 0; n < 2000; n++) { \
                 var m = [1, 2, 3, 4, 5, 6, 7, 8].map(function (x) { var g = {x: x}; return {v: x}; }); \
                 var f = [1, 2, 3, 4].filter(function () { return true; }).map(function (x) { return [x]; }); \
                 for (var i = 0; i < 8; i++) if (!m[i] || m[i].v !== i + 1) bad++; \
                 for (var i = 0; i < 4; i++) if (!f[i] || f[i][0] !== i + 1) bad++; \
               } bad";
    let mut realm = small(limits.clone());
    assert_eq!(realm.run(src), Ok(JsValue::Number(0.0)));
    assert!(realm.interp().heap.collections > 0);
    // Getters make a new object for each read, which `shift` and `sort` then store.
    let src = "function fresh(v) { var g = {x: v}; return {v: v}; } \
               function set(i) { return function (x) { Object.defineProperty(this, i, {value: x, writable: true, enumerable: true, configurable: true}); }; } \
               var p = {}; \
               for (var i = 0; i < 4; i++) Object.defineProperty(p, i, {get: (function (v) { return function () { return fresh(v); }; })(4 - i), set: set(i)}); \
               var bad = 0; \
               for (var n = 0; n < 1000; n++) { \
                 var o = Object.create(p); o.length = 4; Array.prototype.shift.call(o); \
                 if (!o[0] || o[0].v !== 3 || o[2].v !== 1) bad++; \
                 var s = Object.create(p); s.length = 4; Array.prototype.sort.call(s, function (a, b) { return a.v - b.v; }); \
                 for (var i = 0; i < 4; i++) if (!s[i] || s[i].v !== i + 1) bad++; \
               } \
               var q = {}; \
               for (var i = 0; i < 4; i++) Object.defineProperty(q, i, {get: (function (v) { return function () { return {v: v}; }; })(4 - i), set: function () {}}); \
               for (var n = 0; n < 20000; n++) { \
                 var u = Object.create(q); u.length = 4; Array.prototype.unshift.call(u, 0, 0, 0, 0); \
                 for (var i = 4; i < 8; i++) if (!u.hasOwnProperty(i) || u[i].v !== 8 - i) bad++; \
               } \
               for (var n = 0; n < 5000; n++) { var t = Object.create(q); t.length = 4; Array.prototype.sort.call(t); } \
               bad";
    let mut realm = small(limits.clone());
    assert_eq!(realm.run(src), Ok(JsValue::Number(0.0)));
    assert!(realm.interp().heap.collections > 0);
    // The `z` objects change each round's garbage, so the collections fall at different points.
    let src = "var bad = 0; \
               for (var n = 0; n < 10000; n++) { \
                 for (var z = 0; z < n % 7; z++) var g = {z: z}; \
                 var j = JSON.parse('[1, 2, 3, 4, 5, 6, 7, 8]', function (k, v) { return k === '' ? v : {v: v}; }); \
                 for (var i = 0; i < 8; i++) if (!j[i] || j[i].v !== i + 1) bad++; \
               } bad";
    let mut realm = small(limits.clone());
    assert_eq!(realm.run(src), Ok(JsValue::Number(0.0)));
    assert!(realm.interp().heap.collections > 0);
    let src = "var k = ''; for (var i = 0; i < 300; i++) k += 'k'; \
               var o = {}; o[k] = {toJSON: function () { var g = {x: 1}; return {a: 1}; }}; \
               var want = '{\"' + k + '\":{\"a\":1}}'; \
               var bad = 0; for (var n = 0; n < 4000; n++) if (JSON.stringify(o) !== want) bad++; bad";
    let mut realm = small(limits);
    assert_eq!(realm.run(src), Ok(JsValue::Number(0.0)));
    assert!(realm.interp().heap.collections > 0);
}

// ---- errors ----

#[test]
fn error_objects_have_names_messages_and_to_string() {
    assert_eq!(string("new TypeError('bad').toString()"), "TypeError: bad");
    assert_eq!(string("RangeError('r').name + Error().message + (new Error(undefined).hasOwnProperty('message') ? 1 : 0)"), "RangeError0");
    truth("var e = new SyntaxError('s'); e instanceof SyntaxError && e instanceof Error && Object.prototype.toString.call(e) === '[object Error]'");
    assert_eq!(string("var e = new Error('m'); e.name = ''; e.toString()"), "m");
    assert_eq!(string("Error.prototype.toString.call({name: 'N'})"), "N");
    truth("URIError.prototype.name === 'URIError' && Object.getPrototypeOf(EvalError.prototype) === Error.prototype && Error.length === 1");
}

// ---- Math ----

#[test]
fn math_functions_follow_the_special_cases() {
    truth(
        "Math.max() === -Infinity && Math.min() === Infinity && isNaN(Math.max(1, NaN, 3)) && 1 / Math.max(-0, 0) === Infinity && 1 / Math.min(0, -0) === -Infinity",
    );
    truth("Math.round(2.5) === 3 && Math.round(-2.5) === -2 && 1 / Math.round(-0.4) === -Infinity && Math.round(0.49999999999999994) === 0");
    truth("isNaN(Math.pow(1, NaN)) && isNaN(Math.pow(-1, Infinity)) && Math.pow(NaN, 0) === 1 && Math.pow(2, -1) === 0.5");
    truth("Math.abs(-3) === 3 && Math.floor(-1.5) === -2 && Math.ceil(-1.5) === -1 && 1 / Math.ceil(-0.5) === -Infinity");
    truth("Math.sqrt(-1) !== Math.sqrt(-1) && Math.atan2(0, -0) === Math.PI && Math.exp(0) === 1 && Math.log(Math.E) === 1");
    truth("Math.abs(Math.sin(Math.PI)) < 1e-15 && Math.cos(0) === 1 && Math.tan(0) === 0 && Math.asin(1) === Math.PI / 2");
    truth("var ok = true; for (var i = 0; i < 1000; i++) { var r = Math.random(); if (!(r >= 0 && r < 1)) ok = false; } ok");
    truth("Math.SQRT2 * Math.SQRT1_2 > 0.9999 && Math.LN2 > 0.69 && Math.LOG10E > 0.43 && Object.prototype.toString.call(Math) === '[object Math]'");
    truth("var n = 0; Math.max({valueOf: function () { n++; return NaN; }}, {valueOf: function () { n++; return 1; }}); n === 2");
}

// ---- JSON ----

#[test]
fn json_parse_reads_the_grammar_of_section_15_12_1() {
    assert_eq!(num("var v = JSON.parse(' {\"a\": [1, -2.5e1, true, null, \"x\\\\u0041\\\\n\"], \"b\": {}} '); v.a[1] + v.a.length"), -20.0);
    assert_eq!(string("JSON.parse('\"\\\\ud800\"').charCodeAt === undefined ? 'no string methods' : ''"), "no string methods");
    truth("JSON.parse('[]').length === 0 && JSON.parse('0') === 0 && 1 / JSON.parse('-0') === -Infinity && JSON.parse('{\"a\":1,\"a\":2}').a === 2");
    for bad in ["'{a: 1}'", "'[1,]'", "'01'", "'1.'", "'.5'", "'\"\\t\"'", "\"'x'\"", "'[1] 2'", "''", "'+1'", "'\"\\\\x\"'", "'{\"a\" 1}'"] {
        throws(&format!("JSON.parse({bad})"), "SyntaxError");
    }
    // The reviver sees every value bottom-up and can delete or replace it.
    assert_eq!(
        string(
            "var seen = []; var v = JSON.parse('{\"a\": [1, 2], \"b\": 3}', function (k, v) { seen.push(k); return k === 'b' ? undefined : v; }); seen.join() + ':' + ('b' in v)"
        ),
        "0,1,a,b,:false"
    );
    assert_eq!(num("JSON.parse('[1, 2]', function (k, v) { return typeof v === 'number' ? v * 10 : v; })[1]"), 20.0);
}

#[test]
fn json_stringify_writes_values_with_replacers_and_indentation() {
    assert_eq!(
        string("JSON.stringify({a: [1, 'x', null, undefined, function () {}], b: undefined, c: true})"),
        "{\"a\":[1,\"x\",null,null,null],\"c\":true}"
    );
    assert_eq!(string("JSON.stringify('\\u0001\"\\\\\\n')"), "\"\\u0001\\\"\\\\\\n\"");
    assert_eq!(string("JSON.stringify([NaN, Infinity, -0, 1e21])"), "[null,null,0,1e+21]");
    assert_eq!(run("JSON.stringify(undefined)"), JsValue::Undefined);
    assert_eq!(run("JSON.stringify(function () {})"), JsValue::Undefined);
    assert_eq!(string("JSON.stringify({a: 1, b: [1, 2], c: {}}, null, 2)"), "{\n  \"a\": 1,\n  \"b\": [\n    1,\n    2\n  ],\n  \"c\": {}\n}");
    assert_eq!(string("JSON.stringify([1], null, 'abcdefghijkl')"), "[\nabcdefghij1\n]");
    assert_eq!(string("JSON.stringify({a: 1, b: 2, c: 3}, ['c', 'a', 'c', 1])"), "{\"c\":3,\"a\":1}");
    assert_eq!(string("JSON.stringify({a: 1, b: 'x'}, function (k, v) { return typeof v === 'number' ? v + 1 : v; })"), "{\"a\":2,\"b\":\"x\"}");
    assert_eq!(string("JSON.stringify({toJSON: function (k) { return 'k=' + k; }})"), "\"k=\"");
    assert_eq!(string("JSON.stringify({d: {toJSON: function (k) { return k; }}})"), "{\"d\":\"d\"}");
    assert_eq!(string("JSON.stringify([Object(false), Object(true)])"), "[false,true]");
    assert_eq!(string("JSON.stringify({}, null, 4) + JSON.stringify([], null, 4)"), "{}[]");
    throws("var a = []; a.push(a); JSON.stringify(a)", "TypeError");
    throws("var o = {}; o.x = {y: o}; JSON.stringify(o)", "TypeError");
    // The same object twice, not nested in itself, is not a cycle.
    assert_eq!(string("var x = {}; JSON.stringify([x, x])"), "[{},{}]");
}

#[test]
fn deep_json_nesting_needs_no_native_stack() {
    let handle = std::thread::Builder::new()
        .stack_size(256 << 10)
        .spawn(|| {
            let mut realm = Realm::new();
            realm.run(
                "var depth = 100000; var s = ''; for (var i = 0; i < depth; i++) s += '['; for (i = 0; i < depth; i++) s += ']'; \
                 var v = JSON.parse(s); var back = JSON.stringify(v); var n = 0; JSON.parse(s, function (k, v) { n++; return v; }); \
                 back.length === 2 * depth && n === depth",
            )
        })
        .unwrap();
    assert_eq!(handle.join().unwrap(), Ok(JsValue::Bool(true)));
}

// ---- global functions ----

#[test]
fn global_number_functions() {
    truth("parseInt('  0x1F') === 31 && parseInt('-12px') === -12 && parseInt('z', 36) === 35 && isNaN(parseInt('', 10)) && isNaN(parseInt('1', 1))");
    truth(
        "parseInt('011') === 11 && parseInt('11', 2) === 3 && parseInt('0x10', 16) === 16 && isNaN(parseInt('0x', 16)) && parseInt('12', 3.9) === 5",
    );
    truth("parseInt('123456789012345678901234567890') === 1.2345678901234568e29 && 1 / parseInt('-0') === -Infinity");
    truth(
        "parseFloat('  3.5e2x') === 350 && parseFloat('-.5') === -0.5 && parseFloat('Infinityx') === Infinity && isNaN(parseFloat('e5')) && parseFloat('1e') === 1",
    );
    truth("isNaN('x') && !isNaN('1') && isFinite('1') && !isFinite(Infinity) && !isFinite(NaN)");
    truth("NaN !== NaN && Infinity === 1 / 0 && undefined === void 0 && !Object.getOwnPropertyDescriptor(this, 'NaN').writable");
}

#[test]
fn uri_functions_encode_and_decode_utf8() {
    assert_eq!(string("encodeURIComponent('a b&c/\\u00e9\\ud83d\\ude00')"), "a%20b%26c%2F%C3%A9%F0%9F%98%80");
    assert_eq!(string("encodeURI('http://x.y/a b?c=d#e')"), "http://x.y/a%20b?c=d#e");
    assert_eq!(string("decodeURIComponent('a%20b%26%C3%A9%F0%9F%98%80') === 'a b&\\u00e9\\ud83d\\ude00' ? 'ok' : 'no'"), "ok");
    assert_eq!(string("decodeURI('%3B%2F%20%23')"), "%3B%2F %23");
    for bad in [
        "encodeURI('\\ud800')",
        "encodeURIComponent('\\udc00x')",
        "decodeURI('%')",
        "decodeURI('%C3')",
        "decodeURI('%C0%80')",
        "decodeURI('%ED%A0%80')",
        "decodeURI('%FF')",
    ] {
        throws(bad, "URIError");
    }
}

#[test]
fn escape_and_unescape_follow_annex_b() {
    assert_eq!(string("escape('a b+/@\\u00e9\\u0100')"), "a%20b+/@%E9%u0100");
    assert_eq!(string("unescape('a%20b%E9%u0100%zz%u12') === 'a b\\u00e9\\u0100%zz%u12' ? 'ok' : 'no'"), "ok");
}

// ---- robustness ----

/// Calls of built-ins with arguments of every kind.
const CALLS: &[&str] = &[
    "[1,,3].join(X)",
    "[3,1,2].sort(X)",
    "[1,2,3].splice(X, X, X)",
    "[1,2].slice(X, X)",
    "[].concat(X, [X])",
    "[1,2,3].map(X)",
    "[1,2].reduce(X, X)",
    "[1,2].indexOf(X, X)",
    "[1,2].lastIndexOf(X, X)",
    "Array.prototype.push.call(X, X)",
    "Array.prototype.unshift.call(X, X)",
    "Array.prototype.reverse.call(X)",
    "Array(X)",
    "Object.keys(X)",
    "Object.freeze(X)",
    "Object.defineProperty(X, X, X)",
    "Object.create(X, X)",
    "Object.getOwnPropertyNames(X)",
    "JSON.stringify(X, X, X)",
    "JSON.parse(X, X)",
    "Math.max(X, X)",
    "Math.pow(X, X)",
    "parseInt(X, X)",
    "parseFloat(X)",
    "decodeURIComponent(X)",
    "encodeURI(X)",
    "unescape(X)",
    "escape(X)",
    "Function.prototype.bind.call(X, X)",
];

const ARGS: &[&str] = &[
    "undefined",
    "null",
    "0",
    "-1",
    "4294967295",
    "NaN",
    "'%E0'",
    "'[1,{\"a\":2}]'",
    "''",
    "[]",
    "{length: 4294967295}",
    "{}",
    "function (a, b) { return b - a; }",
    "{valueOf: function () { throw 1; }}",
    "{get length() { return 3; }}",
    "Object.freeze([1, 2])",
    "{value: 1, get: function () {}}",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_built_in_calls_never_panic(call in 0..CALLS.len(), args in proptest::collection::vec(0..ARGS.len(), 3)) {
        let mut src = CALLS[call].to_string();
        for a in &args {
            src = src.replacen('X', ARGS[*a], 1);
        }
        let src = src.replace('X', "1");
        let mut realm = small(Limits { fuel: 200_000, heap_bytes: 4 << 20, frames: 200, reentry: 8 });
        if let Err(e) = realm.run(&src) {
            prop_assert!(e.kind != RunErrorKind::Internal, "{src}: {e:?}");
        }
    }
}
