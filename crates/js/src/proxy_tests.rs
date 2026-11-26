//! `Proxy` and `Reflect` in a realm with `Options::es2015`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::{Data, HostFunction, HostItem, JsValue, LimitKind, Limits, Options, Realm, RunError, RunErrorKind};

fn es_options() -> Options {
    Options { es2015: true, ..Options::default() }
}

fn es() -> Realm {
    Realm::with_options(es_options(), Limits::default())
}

/// `type_error(f)` is true when calling `f` throws a `TypeError`.
const PRELUDE: &str = "function type_error(f) { try { f(); } catch (e) { return e instanceof TypeError; } return false; }\n";

fn truth(src: &str) {
    let script = format!("{PRELUDE}{src}");
    match es().run(&script) {
        Ok(v) => assert_eq!(v, JsValue::Bool(true), "{src}"),
        Err(e) => panic!("{src}\n{e:?}"),
    }
}

/// Each script, run after [`PRELUDE`], throws a `TypeError` from its last statement.
fn type_errors(scripts: &[&str]) {
    for src in scripts {
        let script = format!("{PRELUDE}{src}");
        match es().run(&script) {
            Err(RunError { kind: RunErrorKind::Thrown, message, .. }) => assert!(message.starts_with("TypeError"), "{src}: {message}"),
            other => panic!("{src}: {other:?}"),
        }
    }
}

fn limit(src: &str, limits: Limits) -> LimitKind {
    match Realm::with_options(es_options(), limits).run(src) {
        Err(RunError { kind: RunErrorKind::Limit(kind), .. }) => kind,
        other => panic!("{src}: {other:?}"),
    }
}

// ---- the realm gate ----

#[test]
fn only_a_realm_that_ran_es2015_code_has_proxy_and_reflect() {
    let names = "typeof Proxy + ' ' + typeof Reflect";
    assert_eq!(es().run(names), Ok(JsValue::String("function object".into())));
    assert_eq!(Realm::new().run(names), Ok(JsValue::String("undefined undefined".into())));
    let e4x = Options { e4x: true, ..Options::default() };
    assert_eq!(Realm::with_options(e4x, Limits::default()).run(names), Ok(JsValue::String("undefined undefined".into())));
    // A realm made without the option gets both at its first run with it, once, and keeps them.
    let mut later = Realm::new();
    assert_eq!(later.run(names), Ok(JsValue::String("undefined undefined".into())));
    later.set_es2015(true);
    assert_eq!(later.run(&format!("var first = Proxy, firstReflect = Reflect; {names}")), Ok(JsValue::String("function object".into())));
    later.set_es2015(false);
    assert_eq!(later.run(names), Ok(JsValue::String("function object".into())));
    later.set_es2015(true);
    assert_eq!(later.run("Proxy === first && Reflect === firstReflect"), Ok(JsValue::Bool(true)));
    // A global of either name that an earlier run defined stays as that run left it.
    let mut taken = Realm::new();
    assert_eq!(taken.run("var Proxy = 1; Object.defineProperty(this, 'Reflect', { value: 2 }); 0"), Ok(JsValue::Number(0.0)));
    taken.set_es2015(true);
    assert_eq!(taken.run("Proxy + Reflect"), Ok(JsValue::Number(3.0)));
}

// ---- Proxy ----

#[test]
fn the_constructor_needs_new_and_two_objects() {
    truth("Proxy.length === 2 && Proxy.revocable.length === 2 && Proxy.prototype === undefined");
    type_errors(&["Proxy({}, {})", "new Proxy(1, {})", "new Proxy({}, null)", "new Proxy({})"]);
}

#[test]
fn traps_get_the_target_key_and_receiver_with_the_handler_as_this() {
    truth(
        "var target = { a: 1 }, seen = [];
         var handler = { get: function (t, k, r) { seen.push(this === handler, t === target, k, r === p); return 42; } };
         var p = new Proxy(target, handler);
         p.a === 42 && seen.join() === 'true,true,a,true'",
    );
    truth(
        "var log = [];
         var p = new Proxy({}, { set: function (t, k, v, r) { log.push(k, v, r === p); t[k] = v * 2; return true; } });
         p.x = 3;
         log.join() === 'x,3,true' && p.x === 6",
    );
    truth(
        "var p = new Proxy({}, { has: function (t, k) { return k === 'yes'; } });
         ('yes' in p) && !('no' in p)",
    );
    truth(
        "var p = new Proxy({ a: 1, b: 2 }, { deleteProperty: function (t, k) { delete t[k]; return k === 'a'; } });
         delete p.a && !Reflect.deleteProperty(p, 'b')",
    );
}

#[test]
fn an_absent_trap_forwards_each_operation_to_the_target() {
    truth(
        "var target = { a: 1 };
         var p = new Proxy(target, {});
         p.b = 2;
         Object.defineProperty(p, 'c', { value: 3, enumerable: false });
         var ok = p.a === 1 && target.b === 2 && ('c' in p) && Object.keys(p).join() === 'a,b';
         ok = ok && Reflect.ownKeys(p).join() === 'a,b,c' && delete p.a && !('a' in target);
         ok = ok && Object.getPrototypeOf(p) === Object.prototype && Reflect.setPrototypeOf(p, null) && Object.getPrototypeOf(target) === null;
         Object.preventExtensions(p);
         ok && !Object.isExtensible(target) && Object.isFrozen(Object.freeze(p)) && Object.isFrozen(target)",
    );
}

#[test]
fn callable_and_constructable_proxies() {
    truth(
        "function add(a, b) { return a + b; }
         var p = new Proxy(add, {});
         var q = new Proxy(add, { apply: function (t, self, args) { return [self, args.length, t(args[0], args[1])]; } });
         var r = q.call('me', 2, 3);
         typeof p === 'function' && p(1, 2) === 3 && r[0] === 'me' && r[1] === 2 && r[2] === 5 && p.apply(null, [4, 5]) === 9",
    );
    truth(
        "function Point(x) { this.x = x; }
         var P = new Proxy(Point, {});
         var made = new P(7);
         var C = new Proxy(Point, { construct: function (t, args, nt) { return { args: args.length, nt: nt === C }; } });
         var c = new C(1, 2);
         made.x === 7 && made instanceof Point && c.args === 2 && c.nt",
    );
    type_errors(&[
        "new Proxy({}, {})()",
        "new (new Proxy(function () {}, { construct: function () { return 1; } }))()",
        "new (new Proxy(Math.max, {}))()",
        "var arrow = new Proxy(() => 1, {}); new arrow()",
    ]);
    truth("typeof new Proxy({}, {}) === 'object' && Object.prototype.toString.call(new Proxy(function () {}, {})) === '[object Function]'");
}

#[test]
fn a_revoked_proxy_refuses_every_operation() {
    truth(
        "var r = Proxy.revocable(function () {}, {});
         var p = r.proxy;
         r.revoke();
         r.revoke();
         var ops = [
             function () { return p.x; }, function () { p.x = 1; }, function () { return 'x' in p; }, function () { delete p.x; },
             function () { Object.keys(p); }, function () { Object.getPrototypeOf(p); }, function () { Object.isExtensible(p); },
             function () { Object.preventExtensions(p); }, function () { Object.defineProperty(p, 'x', {}); },
             function () { Object.getOwnPropertyDescriptor(p, 'x'); }, function () { p(); }, function () { new p(); },
             function () { Array.isArray(p); }, function () { for (var k in p) {} }, function () { JSON.stringify(p); },
             function () { Object.prototype.toString.call(p); }, function () { ({}) instanceof p; },
         ];
         var all = true;
         for (var i = 0; i < ops.length; i++) all = all && type_error(ops[i]);
         all && typeof p === 'function' && r.revoke.length === 0 && r.revoke() === undefined",
    );
    // A trap that revokes its own proxy still has its target for the checks that follow.
    truth(
        "var target = {};
         Object.defineProperty(target, 'k', { value: 5 });
         var r = Proxy.revocable(target, { get: function () { r.revoke(); return 5; } });
         r.proxy.k === 5 && type_error(function () { return r.proxy.k; })",
    );
}

// ---- invariants ----

#[test]
fn a_trap_result_that_breaks_an_invariant_is_a_type_error() {
    let fixed = "var t = {}; Object.defineProperty(t, 'k', { value: 1 });";
    let sealed = "var t = { k: 1 }; Object.preventExtensions(t);";
    type_errors(&[
        &format!("{fixed} new Proxy(t, {{ get: function () {{ return 2; }} }}).k"),
        "var t = {}; Object.defineProperty(t, 'k', { set: function () {} }); new Proxy(t, { get: function () { return 1; } }).k",
        &format!("{fixed} 'use strict'; new Proxy(t, {{ set: function () {{ return true; }} }}).k = 2"),
        "var t = {}; Object.defineProperty(t, 'k', { get: function () {} }); Reflect.set(new Proxy(t, { set: function () { return true; } }), 'k', 1)",
        &format!("{fixed} 'k' in new Proxy(t, {{ has: function () {{ return false; }} }})"),
        &format!("{sealed} 'k' in new Proxy(t, {{ has: function () {{ return false; }} }})"),
        &format!("{fixed} delete new Proxy(t, {{ deleteProperty: function () {{ return true; }} }}).k"),
        &format!("{sealed} delete new Proxy(t, {{ deleteProperty: function () {{ return true; }} }}).k"),
        "var t = {}; Object.preventExtensions(t); Object.defineProperty(new Proxy(t, { defineProperty: function () { return true; } }), 'n', { value: 1 })",
        "Object.defineProperty(new Proxy({}, { defineProperty: function () { return true; } }), 'n', { value: 1, configurable: false })",
        "var t = { k: 1 }; Object.defineProperty(new Proxy(t, { defineProperty: function () { return true; } }), 'k', { configurable: false })",
        "var t = {}; Object.defineProperty(t, 'k', { value: 1, writable: true }); \
         Reflect.defineProperty(new Proxy(t, { defineProperty: function () { return true; } }), 'k', { writable: false })",
        &format!("{fixed} Object.getOwnPropertyDescriptor(new Proxy(t, {{ getOwnPropertyDescriptor: function () {{ return undefined; }} }}), 'k')"),
        &format!("{sealed} Object.getOwnPropertyDescriptor(new Proxy(t, {{ getOwnPropertyDescriptor: function () {{ return undefined; }} }}), 'k')"),
        "Object.getOwnPropertyDescriptor(new Proxy({}, { getOwnPropertyDescriptor: function () { return { value: 1 }; } }), 'k')",
        "Object.getOwnPropertyDescriptor(new Proxy({}, { getOwnPropertyDescriptor: function () { return 1; } }), 'k')",
        "var t = {}; Object.defineProperty(t, 'k', { value: 1, writable: true }); \
         Object.getOwnPropertyDescriptor(new Proxy(t, { getOwnPropertyDescriptor: function () { return { value: 1, writable: false }; } }), 'k')",
        "var t = {}; Object.preventExtensions(t); Object.getPrototypeOf(new Proxy(t, { getPrototypeOf: function () { return Array.prototype; } }))",
        "Object.getPrototypeOf(new Proxy({}, { getPrototypeOf: function () { return 1; } }))",
        "var t = {}; Object.preventExtensions(t); Reflect.setPrototypeOf(new Proxy(t, { setPrototypeOf: function () { return true; } }), Array.prototype)",
        "Object.isExtensible(new Proxy({}, { isExtensible: function () { return false; } }))",
        "Object.preventExtensions(new Proxy({}, { preventExtensions: function () { return true; } }))",
        "Object.preventExtensions(new Proxy({}, { preventExtensions: function () { return false; } }))",
        "new Proxy({}, { get: 1 }).k",
    ]);
}

#[test]
fn an_own_keys_result_is_checked_against_the_target() {
    type_errors(&[
        "Object.keys(new Proxy({}, { ownKeys: function () { return ['a', 'a']; } }))",
        "Object.keys(new Proxy({}, { ownKeys: function () { return ['a', 1]; } }))",
        "Object.keys(new Proxy({}, { ownKeys: function () { return 'ab'; } }))",
        "var t = {}; Object.defineProperty(t, 'k', { value: 1 }); Reflect.ownKeys(new Proxy(t, { ownKeys: function () { return []; } }))",
        "var t = { a: 1 }; Object.preventExtensions(t); Reflect.ownKeys(new Proxy(t, { ownKeys: function () { return []; } }))",
        "var t = { a: 1 }; Object.preventExtensions(t); Reflect.ownKeys(new Proxy(t, { ownKeys: function () { return ['a', 'b']; } }))",
    ]);
    truth(
        "var t = { a: 1 }; Object.preventExtensions(t);
         var keys = Reflect.ownKeys(new Proxy(t, { ownKeys: function () { return ['a']; } }));
         var listed = Reflect.ownKeys(new Proxy({}, { ownKeys: function () { return { length: 2, 0: 'x', 1: 'y' }; } }));
         keys.join() === 'a' && listed.join() === 'x,y'",
    );
    let mut realm = es();
    let e = realm.run("Reflect.ownKeys(new Proxy({}, { ownKeys: function () { return { length: 1e9 }; } }))").unwrap_err();
    assert!(e.message.starts_with("RangeError"), "{e:?}");
}

// ---- limits ----

#[test]
fn traps_stay_within_the_run_limits() {
    let fuel = Limits { fuel: 200_000, ..Limits::default() };
    assert_eq!(limit("new Proxy({}, { get: function () { while (true) {} } }).x", fuel.clone()), LimitKind::Fuel);
    // A trap that reads its own proxy, a chain of proxies without traps and a handler that is a proxy
    // of itself end at the nested-call limit rather than exhausting the native stack.
    assert_eq!(limit("var p = new Proxy({}, { get: function (t, k, r) { return r[k]; } }); p.x", Limits::default()), LimitKind::Reentry);
    assert_eq!(limit("var p = {}; for (var i = 0; i < 100000; i++) p = new Proxy(p, {}); p.x", Limits::default()), LimitKind::Reentry);
    assert_eq!(
        limit("var h = {}; var p = new Proxy({}, h); h.__proto__ = p; Reflect.setPrototypeOf(h, p); p.x", Limits::default()),
        LimitKind::Reentry
    );
    // A `getPrototypeOf` trap that gives the proxy back makes a chain without end, which the fuel ends.
    let chain = "var p = new Proxy({}, { getPrototypeOf: function () { return p; } });";
    assert_eq!(limit(&format!("{chain} for (var k in p) {{}}"), fuel.clone()), LimitKind::Fuel);
    assert_eq!(limit(&format!("{chain} ({{}}) instanceof function () {{}}; Object.prototype.isPrototypeOf.call({{}}, p)"), fuel), LimitKind::Fuel);
}

// ---- Reflect ----

#[test]
fn reflect_is_a_plain_object_of_functions() {
    truth(
        "var names = Object.getOwnPropertyNames(Reflect).sort().join();
         names === 'apply,construct,defineProperty,deleteProperty,get,getOwnPropertyDescriptor,getPrototypeOf,has,isExtensible,ownKeys,preventExtensions,set,setPrototypeOf'
             && Object.getPrototypeOf(Reflect) === Object.prototype && Reflect.apply.length === 3 && Reflect.construct.length === 2
             && Reflect.set.length === 3",
    );
    type_errors(&["Reflect()", "new Reflect()", "Reflect.get(1, 'x')", "Reflect.ownKeys('abc')", "Reflect.apply(function () {}, null, 1)"]);
}

#[test]
fn reflect_reads_an_argument_list_length_with_to_length() {
    truth(
        "function count() { return arguments.length; }
         function Made() { this.n = arguments.length; }
         Reflect.apply(count, null, { length: -1 }) === 0 && Reflect.apply(count, null, { length: 2.5 }) === 2
             && Reflect.apply(count, null, { length: NaN }) === 0 && new Made().n === 0
             && Reflect.apply(count, null, { length: 65536.5 }) === 65536
             && Reflect.construct(Made, { length: -5 }).n === 0
             && (function () { try { Reflect.apply(count, null, { length: 4294967296 }); } catch (e) { return e instanceof RangeError; } return false; })()",
    );
}

#[test]
fn reflect_methods_answer_with_booleans_and_receivers() {
    truth(
        "var o = {}; Object.defineProperty(o, 'fixed', { value: 1 }); Object.preventExtensions(o);
         !Reflect.defineProperty(o, 'n', { value: 1 }) && !Reflect.deleteProperty(o, 'fixed') && !Reflect.set(o, 'fixed', 2)
             && !Reflect.set(o, 'n', 1) && Reflect.has(o, 'fixed') && !Reflect.isExtensible(o) && Reflect.preventExtensions(o)",
    );
    truth(
        "var base = { get who() { return this.name; }, set who(v) { this.got = v; } };
         var me = { name: 'me' };
         Reflect.get(base, 'who', me) === 'me' && Reflect.set(base, 'who', 5, me) && me.got === 5 && !('got' in base)",
    );
    truth(
        "var data = { v: 1 }, target = {};
         Reflect.set(data, 'v', 2, target) && data.v === 1 && target.v === 2 && Reflect.getOwnPropertyDescriptor(target, 'v').enumerable",
    );
    truth(
        "var a = {}, b = Object.create(a);
         !Reflect.setPrototypeOf(a, b) && !Reflect.setPrototypeOf(Object.prototype, {}) && Reflect.setPrototypeOf(Object.prototype, null)
             && Reflect.setPrototypeOf(b, null) && Reflect.getPrototypeOf(b) === null && type_error(function () { Reflect.setPrototypeOf(b, 1); })",
    );
    truth(
        "function F(x) { this.x = x; } F.prototype.kind = 'F';
         function G() {} G.prototype.kind = 'G';
         var f = Reflect.construct(F, [3]), g = Reflect.construct(F, [4], G);
         f.x === 3 && f.kind === 'F' && g.x === 4 && g.kind === 'G' && Reflect.apply(Math.max, null, [1, 5, 2]) === 5
             && type_error(function () { Reflect.construct(F, [], Math.max); }) && type_error(function () { Reflect.construct(Math.max, []); })",
    );
    truth(
        "var d = Reflect.getOwnPropertyDescriptor({ a: 1 }, 'a');
         d.value === 1 && d.writable && Reflect.getOwnPropertyDescriptor({}, 'a') === undefined && Reflect.ownKeys([1]).join() === '0,length'",
    );
}

#[test]
fn reflect_construct_gives_a_built_in_object_the_prototype_of_new_target() {
    truth(
        "function F() {} F.prototype.kind = 'F';
         var a = Reflect.construct(Array, [1, 2], F), e = Reflect.construct(Error, ['m'], F), d = Reflect.construct(Date, [0], F);
         Object.getPrototypeOf(a) === F.prototype && a.length === 2 && Array.isArray(a) && a.kind === 'F'
             && Object.getPrototypeOf(e) === F.prototype && e.message === 'm' && Object.getPrototypeOf(d) === F.prototype",
    );
    // A `newTarget` whose `prototype` is not an object leaves the built-in's own. `Object` with a
    // different `newTarget` makes a new object and leaves its argument alone.
    truth(
        "function G() {} G.prototype = 1; function F() {} var o = {};
         var made = Reflect.construct(Object, [o], F), plain = Reflect.construct(Object, [o], G);
         Object.getPrototypeOf(Reflect.construct(Array, [], G)) === Array.prototype
             && made !== o && Object.getPrototypeOf(made) === F.prototype && Object.getPrototypeOf(o) === Object.prototype
             && plain !== o && Object.getPrototypeOf(plain) === Object.prototype && Reflect.construct(Object, [o]) === o",
    );
}

#[test]
fn reflect_construct_reads_new_target_prototype_after_number_and_string_convert() {
    // `Number`, `String`, `Date` and `Function` convert their arguments before they read
    // `newTarget.prototype`; `Array` and the error types read it first.
    let made = "var log = [];
         var nt = new Proxy(function () {}, { get: function (t, k) { log.push(k); return t[k]; } });
         var arg = { valueOf: function () { log.push('valueOf'); return 3; }, toString: function () { log.push('toString'); return 's'; } };
         var n = Reflect.construct(Number, [arg], nt), numberLog = log.join(); log = [];
         var s = Reflect.construct(String, [arg], nt), stringLog = log.join(); log = [];
         var d = Reflect.construct(Date, [arg], nt), dateLog = log.join(); log = [];
         var e = Reflect.construct(Error, [arg], nt), errorLog = log.join(); log = [];
         var proto = nt.prototype;";
    assert_eq!(
        es().run(&format!("{made} [numberLog, stringLog, dateLog, errorLog].join(' | ')")),
        Ok(JsValue::String("valueOf,prototype | toString,prototype | valueOf,prototype | prototype,toString".into()))
    );
    truth(&format!(
        "{made} Object.getPrototypeOf(n) === proto && Number.prototype.valueOf.call(n) === 3
             && Object.getPrototypeOf(s) === proto && String.prototype.toString.call(s) === 's'
             && Object.getPrototypeOf(d) === proto && Date.prototype.getTime.call(d) === 3 && Object.getPrototypeOf(e) === proto"
    ));
    // A conversion that throws reads nothing from `newTarget`.
    truth(
        "var log = [];
         var nt = new Proxy(function () {}, { get: function (t, k) { log.push(k); return t[k]; } });
         var bad = { valueOf: function () { throw 1; }, toString: function () { throw 1; } };
         var thrown = 0;
         try { Reflect.construct(Number, [bad], nt); } catch (e) { thrown++; }
         try { Reflect.construct(String, [bad], nt); } catch (e) { thrown++; }
         thrown === 2 && log.length === 0",
    );
    // `Proxy` does not read `newTarget.prototype`.
    truth(
        "var nt = new Proxy(function () {}, { get: function (t, k) { if (k === 'prototype') throw new Error('read'); return t[k]; } });
         var p = Reflect.construct(Proxy, [{ a: 1 }, {}], nt);
         p.a === 1",
    );
}

/// Runs `src` with a collection at every heap reservation, and checks that it gives `true`.
fn truth_collecting(src: &str) {
    let mut realm = es();
    crate::interp::COLLECT_ON_RESERVE.with(|c| c.set(true));
    let r = realm.run(&format!("{PRELUDE}{src}"));
    crate::interp::COLLECT_ON_RESERVE.with(|c| c.set(false));
    assert_eq!(r, Ok(JsValue::Bool(true)), "{src}");
    assert!(realm.interp().heap.collections > 0, "{src}");
}

#[test]
fn a_descriptor_a_trap_makes_stays_reachable_while_it_is_read() {
    let trap = "var p = new Proxy({}, { getOwnPropertyDescriptor: function () { return { value: { v: 7 }, configurable: true }; } });";
    let accessor = "var q = new Proxy({}, { getOwnPropertyDescriptor: function () {
                        return { get: function () { return 8; }, configurable: true }; } });";
    truth_collecting(&format!(
        "{trap} Object.getOwnPropertyDescriptor(p, 'x').value.v === 7 && Reflect.getOwnPropertyDescriptor(p, 'x').value.v === 7"
    ));
    truth_collecting(&format!("{accessor} Object.getOwnPropertyDescriptor(q, 'x').get() === 8"));
}

// ---- proxies elsewhere in the language ----

#[test]
fn a_proxy_works_as_a_prototype_and_a_with_scope() {
    truth(
        "var p = new Proxy({}, { get: function (t, k, r) { return k === 'hi' ? 'from proxy' : undefined; },
                                 has: function (t, k) { return k === 'hi'; } });
         var o = Object.create(p);
         o.own = 1;
         var r;
         with (p) { r = hi; }
         o.hi === 'from proxy' && ('hi' in o) && r === 'from proxy' && p.isPrototypeOf === undefined",
    );
    truth(
        "var log = [];
         var p = new Proxy({}, { set: function (t, k, v, r) { log.push(k, r === o); return true; } });
         var o = Object.create(p);
         o.x = 1;
         log.join() === 'x,true' && !o.hasOwnProperty('x')",
    );
    truth("var o = {}; Object.prototype.isPrototypeOf.call(Object.prototype, new Proxy(o, {}))");
}

#[test]
fn typeof_reads_the_binding_a_with_scope_reported_once() {
    truth(
        "var asked = 0;
         var p = new Proxy({}, { has: function (t, k) { return k === 'x' && asked++ === 0; },
                                 get: function (t, k) { return k === 'x' ? function () {} : undefined; } });
         var r;
         with (p) { r = typeof x; }
         r === 'function' && asked === 1",
    );
}

#[test]
fn arrays_json_for_in_and_keys_read_a_proxy_through_its_traps() {
    truth(
        "var p = new Proxy([1, 2], {});
         Array.isArray(p) && JSON.stringify(p) === '[1,2]' && JSON.stringify({ a: p }) === '{\"a\":[1,2]}'
             && Object.prototype.toString.call(p) === '[object Array]' && [0].concat(p).length === 3",
    );
    truth(
        "var p = new Proxy({}, {
             ownKeys: function () { return ['b', 'a', 'hidden']; },
             getOwnPropertyDescriptor: function (t, k) { return { value: k, enumerable: k !== 'hidden', configurable: true }; },
             get: function (t, k) { return k.toUpperCase(); }
         });
         var seen = [];
         for (var k in p) seen.push(k);
         seen.join() === 'b,a' && Object.keys(p).join() === 'b,a' && JSON.stringify(p) === '{\"b\":\"B\",\"a\":\"A\"}'",
    );
    // A proxy on the prototype chain lists its names, and its `has` trap does not hide them.
    truth(
        "var p = new Proxy({ x: 1 }, { has: function () { return false; } });
         var seen = [];
         for (var k in Object.create(p)) seen.push(k);
         seen.join() === 'x'",
    );
    // A name deleted or made non-enumerable during the loop is not visited, a name from the
    // proxy's prototype still is, and the `has` trap is not asked.
    truth(
        "var t = { a: 1, b: 2, c: 3 }, asked = false;
         var p = new Proxy(t, { has: function () { asked = true; return true; } });
         Reflect.setPrototypeOf(t, { d: 4 });
         var seen = [];
         for (var k in p) { seen.push(k); if (k === 'a') { delete t.b; Object.defineProperty(t, 'c', { enumerable: false }); } }
         seen.join() === 'a,d' && !asked",
    );
    // A proxy on an array's prototype chain answers for indices the array's own properties do not
    // show, so a long array method visits every index rather than only the stored ones.
    truth(
        "var p = new Proxy({}, { has: function (t, k) { return k === '5000'; }, get: function (t, k) { return k === '5000' ? 'x' : undefined; } });
         var a = []; a.length = 20000; Reflect.setPrototypeOf(a, p);
         Array.prototype.indexOf.call(a, 'x') === 5000",
    );
    // A new prototype set during a long array method brings its indices with it.
    truth(
        "var proto = Object.create(Array.prototype); proto[19999] = 'z';
         var a = []; a.length = 20000; a[0] = 0;
         var seen = [];
         a.forEach(function (v, i) { seen.push(i); if (i === 0) Reflect.setPrototypeOf(a, proto); });
         seen.join() === '0,19999'",
    );
}

#[test]
fn for_in_reads_ordinary_and_proxy_names_by_the_same_rules() {
    // A name made non-enumerable during the loop is skipped on an ordinary object, as on a proxy.
    truth(
        "var o = { a: 1, b: 2, c: 3 }, seen = [];
         for (var k in o) { seen.push(k); if (k === 'a') Object.defineProperty(o, 'b', { enumerable: false }); }
         var p = new Proxy({ a: 1, b: 2 }, {}), proxySeen = [];
         for (var k in Object.create(p)) { proxySeen.push(k); if (k === 'a') Object.defineProperty(p, 'b', { enumerable: false }); }
         seen.join() === 'a,c' && proxySeen.join() === 'a'",
    );
    // A name the `ownKeys` trap lists without a descriptor does not hide an enumerable one further
    // up the chain; a non-enumerable own property does.
    truth(
        "var p = new Proxy(Object.create({ x: 1, y: 2 }), {
             ownKeys: function () { return ['x', 'y']; },
             getOwnPropertyDescriptor: function (t, k) { return k === 'y' ? { value: 0, enumerable: false, configurable: true } : undefined; }
         });
         var seen = [];
         for (var k in p) seen.push(k);
         seen.join() === 'x'",
    );
}

/// A `getPrototypeOf` trap that gives a new proxy of a large array each time it is called, 40
/// times, and then `null`: the chain it makes holds far more than the heap limit of
/// [`walk_limits`] at once.
const FRESH_CHAIN: &str = "var hops = 0;
     function big() { var t = []; for (var i = 0; i < 3000; i++) t[i] = i; return t; }
     var tail = { getPrototypeOf: function () { hops++; return hops > 40 ? null : new Proxy(big(), tail); } };";

fn walk_limits() -> Limits {
    Limits { heap_bytes: 4 << 20, ..Limits::default() }
}

#[test]
fn a_prototype_walk_keeps_only_the_prototype_it_has_reached() {
    // Declaring a function without the option reads the global object's inherited property.
    let mut realm = Realm::with_options(es_options(), walk_limits());
    assert_eq!(realm.run(&format!("{FRESH_CHAIN} Reflect.setPrototypeOf(this,new Proxy({{}}, tail)); 0")), Ok(JsValue::Number(0.0)));
    realm.set_es2015(false);
    assert_eq!(realm.run("function declared() { return 5; } declared() + hops"), Ok(JsValue::Number(46.0)));
    // `for-in` reads the chain again for a name deleted during the loop.
    let mut realm = Realm::with_options(es_options(), walk_limits());
    let src = format!(
        "{FRESH_CHAIN}
         var armed = false;
         var head = new Proxy({{}}, {{ getPrototypeOf: function () {{ return armed ? new Proxy(big(), tail) : null; }} }});
         var o = Object.create(head); o.a = 1; o.b = 2;
         var seen = [];
         for (var k in o) {{ seen.push(k); if (k === 'a') {{ armed = true; delete o.b; }} }}
         seen.join() + ' ' + hops"
    );
    assert_eq!(realm.run(&src), Ok(JsValue::String("a 41".into())));
}

#[test]
fn a_function_stays_reachable_while_a_trap_runs_as_it_is_declared() {
    let mut realm = es();
    let proto = "Reflect.setPrototypeOf(this, new Proxy(Object.prototype, {
                     getOwnPropertyDescriptor: function (t, k) { var junk = [[1], [2], [3]]; return Reflect.getOwnPropertyDescriptor(t, k); } })); 0";
    assert_eq!(realm.run(proto), Ok(JsValue::Number(0.0)));
    realm.set_es2015(false);
    crate::interp::COLLECT_ON_RESERVE.with(|c| c.set(true));
    let r = realm.run("function declared() { return 5; } declared()");
    crate::interp::COLLECT_ON_RESERVE.with(|c| c.set(false));
    assert_eq!(r, Ok(JsValue::Number(5.0)));
    assert!(realm.interp().heap.collections > 0);
}

#[test]
fn spread_lets_a_proxy_shrink_the_array_it_reads() {
    // A proxy's `get` trap answers for the indices the array does not hold, so the length is not
    // refused before the elements are read.
    truth(
        "var a = []; a.length = 4294967295;
         Reflect.setPrototypeOf(a, new Proxy({}, { get: function (t, k) { if (k === '0') a.length = 1; return 'v'; } }));
         var r = [...a];
         r.length === 1 && r[0] === 'v'",
    );
    // A proxy of an array is spread as an array: its length is read again before each element.
    truth(
        "var t = [1, 2, 3];
         var p = new Proxy(t, { get: function (o, k) { if (k === '0') t.length = 1; return o[k]; } });
         var r = [...p];
         r.length === 1 && r[0] === 1",
    );
    // A proxy that its `length` read revokes spreads as the empty array that length gives.
    truth(
        "var made = Proxy.revocable([], { get: function (t, k) { if (k === 'length') { made.revoke(); return 0; } return t[k]; } });
         [...made.proxy].length === 0",
    );
    // A proxy of an array is not asked whether it has a `length`.
    truth(
        "var asked = false;
         var p = new Proxy([1, 2], { has: function (t, k) { asked = true; return k in t; } });
         [...p].join() === '1,2' && !asked",
    );
    // Without a proxy or a getter, the length is refused at once.
    let src = "var a = []; a.length = 4294967295; [...a]";
    match es().run(src) {
        Err(RunError { kind: RunErrorKind::Thrown, message, .. }) => assert!(message.starts_with("RangeError"), "{src}: {message}"),
        other => panic!("{src}: {other:?}"),
    }
}

#[test]
fn instanceof_and_typeof_read_a_proxy() {
    truth(
        "function F() {}
         var f = new F();
         var PF = new Proxy(F, { get: function (t, k) { return k === 'prototype' ? Array.prototype : t[k]; } });
         var viaProto = new Proxy({}, { getPrototypeOf: function () { return F.prototype; } });
         [] instanceof PF && !(f instanceof PF) && viaProto instanceof F && typeof PF === 'function'",
    );
}

// ---- host functions ----

#[test]
fn a_host_function_refuses_a_proxy_and_names_the_argument() {
    let mut realm = es();
    let seen: Rc<RefCell<Vec<Data>>> = Rc::default();
    let log = seen.clone();
    let f: HostFunction = Rc::new(move |mut args| {
        let v = if args.is_empty() { Data::Undefined } else { args.remove(0) };
        log.borrow_mut().push(v.clone());
        Ok(v)
    });
    realm.define_host_object("host", vec![("take".into(), HostItem::Function(f))]).unwrap();
    for (src, place) in
        [("host.take(new Proxy({}, {}))", "argument[0] is a proxy"), ("host.take({ a: [new Proxy([], {})] })", "argument[0].a[0] is a proxy")]
    {
        match realm.run(src) {
            Err(RunError { kind: RunErrorKind::Thrown, message, .. }) => {
                assert!(message.starts_with("TypeError") && message.contains(place), "{src}: {message}");
            }
            other => panic!("{src}: {other:?}"),
        }
    }
    assert!(seen.borrow().is_empty(), "the host function did not run");
    // The traps did not run either.
    let mut realm = es();
    let f: HostFunction = Rc::new(|_| Ok(Data::Undefined));
    realm.define_host_object("host", vec![("take".into(), HostItem::Function(f))]).unwrap();
    let r = realm.run("var ran = false; try { host.take(new Proxy({}, { ownKeys: function () { ran = true; return []; } })); } catch (e) {} ran");
    assert_eq!(r, Ok(JsValue::Bool(false)));
}
