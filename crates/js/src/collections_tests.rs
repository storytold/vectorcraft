use crate::heap::{Class, key};
use crate::value::Value;
use crate::{JsValue, LimitKind, Limits, Options, Realm, RunError, RunErrorKind};

fn es2015() -> Options {
    Options { es2015: true, ..Options::default() }
}

fn realm() -> Realm {
    Realm::with_options(es2015(), Limits::default())
}

fn run(src: &str) -> JsValue {
    match realm().run(src) {
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
    match realm().run(src) {
        Err(RunError { kind: RunErrorKind::Thrown, message, .. }) => assert!(message.starts_with(name), "{src}: {message}"),
        other => panic!("{src}: {other:?}"),
    }
}

fn limit(limits: &Limits, src: &str) -> LimitKind {
    match Realm::with_options(es2015(), limits.clone()).run(src) {
        Err(RunError { kind: RunErrorKind::Limit(k), .. }) => k,
        other => panic!("{src}: {other:?}"),
    }
}

/// The object a global variable holds.
fn global_object(realm: &mut Realm, name: &str) -> crate::heap::ObjId {
    let interp = realm.interp();
    let global = interp.global;
    match interp.heap.object(global).and_then(|g| g.props.get(&key(name))) {
        Some(crate::heap::Prop::Data { value: Value::Object(o), .. }) => *o,
        _ => panic!("{name} is not an object"),
    }
}

// ---- realms ----

#[test]
fn realms_without_the_option_keep_the_es5_globals() {
    for options in [Options::default(), Options { e4x: true, ..Options::default() }] {
        let mut realm = Realm::with_options(options, Limits::default());
        let names = "Map Set WeakMap WeakSet ArrayBuffer DataView Int8Array Uint8Array Uint8ClampedArray Int16Array Uint16Array Int32Array \
                     Uint32Array Float32Array Float64Array";
        for name in names.split_whitespace() {
            assert_eq!(realm.run(&format!("typeof {name}")).unwrap(), JsValue::String("undefined".into()), "{name}");
        }
    }
    truth("typeof Map === 'function' && typeof Float64Array === 'function' && typeof TypedArray === 'undefined'");
}

// ---- Map and Set ----

#[test]
fn keys_compare_with_same_value_zero() {
    truth("var m = new Map(); m.set(NaN, 1); m.get(NaN) === 1 && m.has(0 / 0) && m.size === 1");
    truth("var m = new Map(); m.set(-0, 'a'); m.get(0) === 'a' && m.has(-0) && 1 / m.keys().next().value === Infinity");
    truth("var s = new Set([0, -0, NaN, NaN]); s.size === 2 && 1 / s.values().next().value === Infinity");
    truth("var m = new Map(); m.set('1', 'string').set(1, 'number'); m.size === 2 && m.get('1') === 'string' && m.get(1) === 'number'");
    truth("var a = {}, b = {}; var m = new Map([[a, 1], [b, 2]]); m.get(a) === 1 && m.get(b) === 2 && m.get({}) === undefined");
    truth("var m = new Map(); m.set('ab' + 'c', 1); m.get('abc') === 1");
}

#[test]
fn symbol_keys_compare_by_identity() {
    truth(
        "var a = Symbol('k'), b = Symbol('k'); var m = new Map([[a, 1], [b, 2]]); \
         m.size === 2 && m.get(a) === 1 && m.get(b) === 2 && m.get(Symbol('k')) === undefined && m.keys().next().value === a",
    );
    truth("var r = Symbol.for('k'); var s = new Set([r, Symbol.for('k')]); s.size === 1 && s.has(r) && s.delete(r) && s.size === 0");
}

#[test]
fn entries_keep_the_order_they_were_added_in() {
    assert_eq!(
        string(
            "var m = new Map([['a', 1], ['b', 2], ['c', 3]]); m.set('a', 9); m.delete('b'); m.set('b', 4); \
             var out = []; m.forEach(function (v, k, map) { out.push(k + v + (map === m)); }); out.join()"
        ),
        "a9true,c3true,b4true"
    );
    assert_eq!(string("var s = new Set(['x', 'y']); var out = []; s.forEach(function (v, k) { out.push(v + k); }); out.join()"), "xx,yy");
}

#[test]
fn for_each_skips_deleted_entries_and_visits_added_ones() {
    assert_eq!(
        string(
            "var m = new Map([[1, 'a'], [2, 'b'], [3, 'c']]); var out = []; \
             m.forEach(function (v, k) { out.push(v); if (k === 1) { m.delete(2); m.set(4, 'd'); } }); out.join()"
        ),
        "a,c,d"
    );
    assert_eq!(
        string("var s = new Set([1, 2]); var out = []; s.forEach(function (v) { out.push(v); if (v < 4) s.add(v + 2); }); out.join()"),
        "1,2,3,4,5"
    );
    assert_eq!(string("var s = new Set([1, 2, 3]); var out = []; s.forEach(function (v) { out.push(v); s.clear(); }); out.join()"), "1");
}

#[test]
fn iterators_step_through_live_entries() {
    truth(
        "var m = new Map([['a', 1], ['b', 2], ['c', 3]]); var it = m.entries(); var first = it.next(); m.delete('b'); m.set('d', 4); \
         var second = it.next(), third = it.next(), last = it.next(); \
         first.value.join() === 'a,1' && !first.done && second.value.join() === 'c,3' && third.value.join() === 'd,4' && \
         last.done && last.value === undefined && it.next().done",
    );
    truth("var s = new Set(['p']); var e = s.entries().next().value; e[0] === 'p' && e[1] === 'p' && s.keys === s.values");
    truth("Object.prototype.toString.call(new Map().keys()) === '[object Map Iterator]'");
    throws("var it = new Map().keys(); it.next.call(new Set().values());", "TypeError");
}

#[test]
fn constructors_read_arrays_strings_and_array_likes() {
    truth("var m = new Map([[1, 'a'], [2, 'b']]); m.size === 2 && m.get(2) === 'b'");
    truth("new Set('ab\u{1F600}b').size === 3");
    truth("new Set({length: 3, 0: 'a', 1: 'a', 2: 'b'}).size === 2");
    truth("new Map(null).size === 0 && new Set(undefined).size === 0");
    truth("Object.prototype.toString.call(new Set()) === '[object Set]' && Object.prototype.toString.call(new WeakMap()) === '[object WeakMap]'");
    throws("Map()", "TypeError");
    throws("Set([])", "TypeError");
    throws("new Map([1])", "TypeError");
    throws("new Set(5)", "TypeError");
    throws("new Map({})", "TypeError");
    throws("new WeakSet({})", "TypeError");
    truth("Uint8Array.from({}).length === 0 && new Uint8Array({}).length === 0 && DataView.length === 1 && Uint8Array.length === 3");
    // `from` reads each element of an array-like source just before mapping it (section 22.2.2.1).
    truth("var src = {length: 2, 0: 1, 1: 1}; Uint8Array.from(src, function (v, i) { src[1] = 7; return v; }).join() === '1,7'");
    truth("var src = [1, 1]; Uint8Array.from(src, function (v, i) { src[1] = 7; return v * 2; }).join() === '2,14'");
    truth("Uint8Array.from('ab', function (v) { return v.charCodeAt(0); }).join() === '97,98'");
    throws("Map.prototype.get.call({}, 1)", "TypeError");
    throws("Object.getOwnPropertyDescriptor(Map.prototype, 'size').get.call(new Set())", "TypeError");
    // The constructor adds through the prototype's method, as section 23.1.1.1 says.
    truth(
        "var calls = 0; var set = Map.prototype.set; Map.prototype.set = function (k, v) { calls++; return set.call(this, k, v); }; \
           new Map([[1, 1], [2, 2]]); Map.prototype.set = set; calls === 2",
    );
    truth("JSON.stringify(new Map([[1, 2]])) === '{}' && JSON.stringify(new Set([1])) === '{}'");
}

// ---- WeakMap and WeakSet ----

#[test]
fn weak_collections_take_object_keys_only() {
    truth("var k = {}; var w = new WeakMap([[k, 1]]); w.get(k) === 1 && w.has(k) && w.delete(k) && !w.has(k)");
    truth("var w = new WeakMap(); w.get(1) === undefined && !w.has('a') && !w.delete(null)");
    truth("var s = new WeakSet(); var o = {}; s.add(o) === s && s.has(o) && !s.has(1) && !s.delete(1)");
    throws("new WeakMap().set(1, 2)", "TypeError");
    throws("new WeakSet().add('x')", "TypeError");
    throws("new WeakMap([[1, 2]])", "TypeError");
    truth("new WeakMap().size === undefined && typeof WeakMap.prototype.forEach === 'undefined'");
}

#[test]
fn a_weak_map_entry_does_not_keep_its_key_alive() {
    let mut realm = realm();
    realm
        .run(
            "var wm = new WeakMap(), ws = new WeakSet(), kept = {};
             (function () { for (var i = 0; i < 50; i++) { var k = {}; wm.set(k, {back: k}); ws.add(k); } })();
             wm.set(kept, {back: kept}); ws.add(kept);",
        )
        .unwrap();
    let wm = global_object(&mut realm, "wm");
    let ws = global_object(&mut realm, "ws");
    let before = realm.interp().heap.live_cells();
    realm.interp().collect_garbage();
    let after = realm.interp().heap.live_cells();
    let count = |realm: &mut Realm, o| match realm.interp().heap.object(o).map(|o| &o.class) {
        Some(Class::WeakMap(w) | Class::WeakSet(w)) => w.entries.len(),
        _ => panic!("not a weak collection"),
    };
    assert_eq!(count(&mut realm, wm), 1);
    assert_eq!(count(&mut realm, ws), 1);
    // Each dropped key and its value were freed.
    assert!(before - after >= 100, "{before} -> {after}");
    assert_eq!(realm.run("wm.get(kept).back === kept && ws.has(kept)").unwrap(), JsValue::Bool(true));
}

#[test]
fn a_weak_map_value_reachable_through_another_key_is_kept() {
    let mut realm = realm();
    realm.run("var wm = new WeakMap(), a = {}; (function () { var b = {}; wm.set(a, b); wm.set(b, {n: 7}); })();").unwrap();
    realm.interp().collect_garbage();
    assert_eq!(realm.run("wm.get(wm.get(a)).n").unwrap(), JsValue::Number(7.0));
}

#[test]
fn weak_maps_of_dropped_keys_stay_within_the_heap() {
    let limits = Limits { heap_bytes: 4 << 20, ..Limits::default() };
    let mut realm = Realm::with_options(es2015(), limits);
    let r = realm.run("var wm = new WeakMap(); for (var i = 0; i < 100000; i++) wm.set({}, i); 'done'");
    assert_eq!(r.unwrap(), JsValue::String("done".into()));
    assert!(realm.interp().heap.collections > 0);
}

#[test]
fn collections_of_millions_end_at_the_limits() {
    let heap = Limits { heap_bytes: 4 << 20, ..Limits::default() };
    assert_eq!(limit(&heap, "var m = new Map(); for (var i = 0; ; i++) m.set(i, i);"), LimitKind::Heap);
    assert_eq!(limit(&heap, "var s = new Set(); for (var i = 0; ; i++) s.add('k' + i);"), LimitKind::Heap);
    let fuel = Limits { fuel: 100_000, ..Limits::default() };
    assert_eq!(limit(&fuel, "new Set({length: 1e15})"), LimitKind::Fuel);
    let m = "var m = new Map(); for (var i = 0; i < 1000; i++) m.set(i, i); m.forEach(function () { for (;;) {} });";
    assert_eq!(limit(&fuel, m), LimitKind::Fuel);
}

#[test]
fn a_collection_removes_holes_and_moves_live_iterators() {
    let mut realm = realm();
    realm
        .run(
            "var m = new Map(); for (var i = 0; i < 100; i++) m.set(i, i * 2);
             var it = m.values(); it.next(); it.next();
             for (var j = 0; j < 90; j++) m.delete(j);",
        )
        .unwrap();
    let m = global_object(&mut realm, "m");
    realm.interp().collect_garbage();
    match realm.interp().heap.object(m).map(|o| &o.class) {
        Some(Class::Map(c)) => assert_eq!((c.len(), c.slots()), (10, 10)),
        _ => panic!("not a Map"),
    }
    let rest = realm.run("var out = []; for (var r = it.next(); !r.done; r = it.next()) out.push(r.value); out.join()").unwrap();
    assert_eq!(rest, JsValue::String("180,182,184,186,188,190,192,194,196,198".into()));
}

#[test]
fn a_collection_releases_the_storage_of_removed_entries() {
    let mut realm = realm();
    realm
        .run(
            "var m = new Map(), wm = new WeakMap(), kept = {};
             for (var i = 0; i < 1000; i++) m.set(i, i);
             m.clear(); m.set('a', 1);
             (function () { for (var i = 0; i < 1000; i++) wm.set({}, i); })();
             wm.set(kept, 1);",
        )
        .unwrap();
    let m = global_object(&mut realm, "m");
    let wm = global_object(&mut realm, "wm");
    realm.interp().collect_garbage();
    match realm.interp().heap.object(m).map(|o| &o.class) {
        Some(Class::Map(c)) => {
            let (entries, index) = c.capacity();
            assert_eq!(c.slots(), 1);
            assert!(entries < 64 && index < 64, "{entries} {index}");
        }
        _ => panic!("not a Map"),
    }
    match realm.interp().heap.object(wm).map(|o| &o.class) {
        Some(Class::WeakMap(w)) => assert!(w.entries.len() == 1 && w.entries.capacity() < 64, "{}", w.entries.capacity()),
        _ => panic!("not a WeakMap"),
    }
    assert_eq!(realm.run("m.get('a') + wm.get(kept)").unwrap(), JsValue::Number(2.0));
}

// ---- ArrayBuffer ----

#[test]
fn array_buffers_are_zeroed_and_sliced() {
    truth("var b = new ArrayBuffer(8); b.byteLength === 8 && new Uint8Array(b)[7] === 0");
    truth("var u = new Uint8Array([1, 2, 3, 4]); var s = u.buffer.slice(1, -1); s.byteLength === 2 && new Uint8Array(s).join() === '2,3'");
    truth("ArrayBuffer.isView(new DataView(new ArrayBuffer(1))) && ArrayBuffer.isView(new Int8Array(1)) && !ArrayBuffer.isView(new ArrayBuffer(1))");
    truth("new ArrayBuffer().byteLength === 0 && new ArrayBuffer(2.9).byteLength === 2");
    throws("ArrayBuffer(1)", "TypeError");
    throws("new ArrayBuffer(-1)", "RangeError");
}

#[test]
fn a_huge_array_buffer_is_refused_before_it_is_allocated() {
    throws("new ArrayBuffer(1e15)", "RangeError");
    throws("new ArrayBuffer(Math.pow(2, 53))", "RangeError");
    throws("new Float64Array(Math.pow(2, 40))", "RangeError");
    truth("var caught; try { new Uint8Array(1e12); } catch (e) { caught = e instanceof RangeError; } caught");
    // Larger than the realm's heap limit: a catchable RangeError, not an allocation.
    let mut small = Realm::with_options(es2015(), Limits { heap_bytes: 8 << 20, ..Limits::default() });
    assert_eq!(small.run("try { new ArrayBuffer(16 << 20); 'made' } catch (e) { e.name }").unwrap(), JsValue::String("RangeError".into()));
    // Within the limit but not within what is left of it: the heap limit.
    assert_eq!(
        limit(&Limits { heap_bytes: 8 << 20, ..Limits::default() }, "var keep = []; for (;;) keep.push(new ArrayBuffer(1 << 20));"),
        LimitKind::Heap
    );
}

// ---- DataView ----

#[test]
fn data_views_read_and_write_both_byte_orders() {
    truth("var v = new DataView(new ArrayBuffer(4)); v.setUint16(0, 0x1234); v.getUint8(0) === 0x12 && v.getUint8(1) === 0x34");
    truth("var v = new DataView(new ArrayBuffer(4)); v.setUint16(0, 0x1234, true); v.getUint8(0) === 0x34 && v.getUint16(0, true) === 0x1234");
    truth("var v = new DataView(new ArrayBuffer(4)); v.setInt32(0, -2); v.getInt32(0) === -2 && v.getUint32(0) === 4294967294");
    truth("var v = new DataView(new ArrayBuffer(8)); v.setFloat64(0, Math.PI); v.getFloat64(0) === Math.PI && v.getUint8(0) === 0x40");
    truth("var v = new DataView(new ArrayBuffer(4)); v.setFloat32(0, 1.5, true); v.getFloat32(0, true) === 1.5 && v.getUint8(3) === 0x3f");
    truth("var v = new DataView(new ArrayBuffer(4)); v.setInt8(0, 200); v.getInt8(0) === -56 && v.getUint8(0) === 200");
    truth("var b = new ArrayBuffer(8); var v = new DataView(b, 2, 4); v.byteOffset === 2 && v.byteLength === 4 && v.buffer === b");
    throws("new DataView(new ArrayBuffer(4)).getUint32(1)", "RangeError");
    throws("new DataView(new ArrayBuffer(4)).getInt8(-1)", "RangeError");
    throws("new DataView(new ArrayBuffer(4), 5)", "RangeError");
    throws("new DataView(new ArrayBuffer(4), 2, 3)", "RangeError");
    throws("new DataView({})", "TypeError");
    throws("DataView(new ArrayBuffer(1))", "TypeError");
}

// ---- typed arrays ----

#[test]
fn uint8_clamped_rounds_half_to_even() {
    assert_eq!(string("new Uint8ClampedArray([300, -5, 1.5, 2.5, 0.5, NaN, 254.5, 254.6, Infinity]).join()"), "255,0,2,2,0,0,254,255,255");
    assert_eq!(string("var u = new Uint8ClampedArray(1); u[0] = 3.5; u[0] + ',' + new Uint8Array([300])[0]"), "4,44");
}

#[test]
fn element_types_convert_their_values() {
    assert_eq!(string("new Int8Array([127, 128, -129, 1.9]).join()"), "127,-128,127,1");
    assert_eq!(string("new Uint16Array([-1, 65536]).join()"), "65535,0");
    assert_eq!(string("new Int32Array([4294967295]).join()"), "-1");
    assert_eq!(string("new Uint32Array([-1]).join()"), "4294967295");
    assert_eq!(string("new Float32Array([0.1])[0].toFixed(8)"), "0.10000000");
    assert_eq!(string("new Float64Array([0.1, NaN]).join()"), "0.1,NaN");
    truth("Int16Array.BYTES_PER_ELEMENT === 2 && new Float64Array(1).BYTES_PER_ELEMENT === 8");
    truth("Object.prototype.toString.call(new Uint8Array(1)) === '[object Uint8Array]'");
}

#[test]
fn views_share_their_buffer_little_endian() {
    truth("var b = new ArrayBuffer(8); var u8 = new Uint8Array(b), u32 = new Uint32Array(b); u32[0] = 0x01020304; u8[0] === 4 && u8[3] === 1");
    truth("var a = new Uint8Array([1, 2, 3, 4]); var s = a.subarray(1, 3); s[0] = 9; a[1] === 9 && s.length === 2 && s.byteOffset === 1");
    truth("var b = new ArrayBuffer(8); var a = new Int16Array(b, 2, 2); a.length === 2 && a.byteOffset === 2 && a.byteLength === 4");
    throws("new Int32Array(new ArrayBuffer(8), 2)", "RangeError");
    throws("new Int32Array(new ArrayBuffer(6))", "RangeError");
    throws("new Int16Array(new ArrayBuffer(8), 4, 4)", "RangeError");
}

#[test]
fn numeric_names_are_elements_or_nothing() {
    truth("var a = new Int8Array(2); a[5] = 1; a[5] === undefined && !('5' in a) && Object.keys(a).join() === '0,1'");
    truth(
        "var a = new Int8Array(2); a['-0'] = 1; a[1.5] = 1; a['-0'] === undefined && a[1.5] === undefined && Object.getOwnPropertyNames(a).length === 2",
    );
    truth("Int8Array.prototype[3] = 'p'; var a = new Int8Array(1); var r = a[3] === undefined && a[0] === 0; delete Int8Array.prototype[3]; r");
    truth("var a = new Int8Array(1); a.foo = 1; a['01'] = 2; a.foo === 1 && a['01'] === 2");
    // The value is converted before the index is checked (section 9.4.5.9), so an ignored write still converts it.
    truth(
        "var n = 0; var a = new Int8Array(1); a[5] = {valueOf: function () { n++; return 1; }}; a['-0'] = {valueOf: function () { n++; }}; n === 2",
    );
    truth("var a = new Int8Array([200]); a[0] === -56 && delete a[1] && !delete a[0]");
    throws("'use strict'; var a = new Int8Array(1); delete a[0];", "TypeError");
    // An out-of-range write is ignored without an error in strict mode too, as later editions define [[Set]].
    truth("'use strict'; var a = new Uint8Array(1); a[2] = 7; a['-1'] = 7; a[0.5] = 7; a.length === 1 && a[2] === undefined");
    truth("var d = Object.getOwnPropertyDescriptor(new Uint8Array([7]), '0'); d.value === 7 && d.writable && d.enumerable && d.configurable");
    truth("var a = new Uint8Array(1); Object.defineProperty(a, '0', {value: 5}); a[0] === 5");
    throws("Object.defineProperty(new Uint8Array(1), '0', {value: 5, configurable: false})", "TypeError");
    throws("Object.defineProperty(new Uint8Array(1), '3', {value: 5})", "TypeError");
    throws("Object.freeze(new Uint8Array(1))", "TypeError");
    truth("var keys = []; for (var k in new Uint8Array(3)) keys.push(k); keys.join() === '0,1,2'");
    truth("JSON.stringify(new Uint8Array([1, 2])) === '{\"0\":1,\"1\":2}'");
}

#[test]
fn array_methods_visit_every_element_of_a_long_typed_array() {
    truth("Array.prototype.join.call(new Uint8Array(5000)).length === 9999");
    truth("var a = new Uint8Array(10000); a[9999] = 1; Array.prototype.indexOf.call(a, 1) === 9999");
    // The element count, not a `length` property defined on the array, bounds the methods.
    truth(
        "var a = new Uint8Array([1, 2, 3]); Object.defineProperty(a, 'length', {value: 0}); var n = 0; \
         a.forEach(function () { n++; }); n === 3 && a.join() === '1,2,3' && a.every(function (v) { return v > 0; }) && !a.every(function (v) { return v > 1; })",
    );
    // A generic array method called on the typed array reads its `length` property.
    truth(
        "var a = new Uint8Array([1, 2, 3]); Object.defineProperty(a, 'length', {value: 1}); \
         Array.prototype.join.call(a) === '1' && a.join() === '1,2,3' && Array.prototype.join.call(new Uint8Array([4, 5])) === '4,5'",
    );
}

#[test]
fn typed_array_methods() {
    assert_eq!(string("new Float64Array([3, NaN, -0, 0, -1, 2]).sort().join()"), "-1,0,0,2,3,NaN");
    truth("var a = new Float64Array([0, -0]).sort(); 1 / a[0] === -Infinity && 1 / a[1] === Infinity");
    assert_eq!(string("new Int8Array([1, 2, 3]).sort(function (a, b) { return b - a; }).join()"), "3,2,1");
    assert_eq!(string("new Int8Array([1, 2, 3, 4]).map(function (v) { return v * 100; }).join()"), "100,-56,44,-112");
    assert_eq!(string("new Int8Array([1, 2, 3, 4]).filter(function (v) { return v % 2; }).join()"), "1,3");
    assert_eq!(string("var a = new Uint8Array([1, 2, 3, 4, 5]); a.set(a.subarray(0, 3), 2); a.join()"), "1,2,1,2,3");
    assert_eq!(string("var a = new Uint8Array(4); a.set([7, '8'], 1); a.join()"), "0,7,8,0");
    throws("new Uint8Array(2).set([1, 2, 3])", "RangeError");
    throws("new Uint8Array(2).set([1], -1)", "RangeError");
    assert_eq!(string("new Uint8Array([1, 2, 3, 4, 5]).copyWithin(0, 3).join()"), "4,5,3,4,5");
    assert_eq!(string("new Uint8Array(4).fill(7, 1, -1).join()"), "0,7,7,0");
    assert_eq!(string("new Int8Array([5, 6, 7]).slice(-2).join()"), "6,7");
    truth("var a = new Int8Array([5, 6, 7]); a.find(function (v) { return v > 5; }) === 6 && a.findIndex(function (v) { return v > 9; }) === -1");
    truth("new Int8Array([1, 2, 1]).indexOf(1, 1) === 2 && new Int8Array([1, 2, 1]).lastIndexOf(2) === 1");
    truth("new Int8Array([1, 2, 3]).reduce(function (s, v) { return s + v; }) === 6 && new Int8Array([1, 2]).reverse().join() === '2,1'");
    truth(
        "var it = new Int8Array([4, 5]).entries(); var e = it.next().value; e[0] === 0 && e[1] === 4 && it.next().value[1] === 5 && it.next().done",
    );
    truth("new Int8Array([9]).keys().next().value === 0 && new Int8Array([9]).values().next().value === 9");
    truth("Uint8Array.from({length: 2, 0: 1, 1: 2}, function (v, i) { return v * 10 + i; }).join() === '10,21'");
    truth("Int16Array.from('12').join() === '1,2' && Float32Array.of(1, 2.5).join() === '1,2.5'");
    truth("new Int8Array([1, 2]).toString === Array.prototype.toString && String(new Int8Array([1, 2])) === '1,2'");
    // A typed array calls each element's toLocaleString on the number (section 22.2.3.27); an array
    // calls it on the element's object (section 15.4.4.3).
    assert_eq!(
        string(
            "Number.prototype.toLocaleString = function () { 'use strict'; return typeof this; }; new Uint8Array([7, 8]).toLocaleString() + ';' + [7].toLocaleString()"
        ),
        "number,number;object"
    );
    // A getter for the method gets the number as `this` too (Invoke, section 7.3.18).
    assert_eq!(
        string(
            "Object.defineProperty(Number.prototype, 'toLocaleString', {get: function () { 'use strict'; var t = typeof this; \
             return function () { return t; }; }}); new Uint8Array([7]).toLocaleString() + ';' + [7].toLocaleString()"
        ),
        "number;object"
    );
    truth(
        "Object.getPrototypeOf(Int8Array) === Object.getPrototypeOf(Float64Array) && Object.getPrototypeOf(Int8Array.prototype) !== Object.prototype",
    );
    throws("Object.getPrototypeOf(Int8Array)()", "TypeError");
    throws("Int8Array(1)", "TypeError");
    throws("Object.getPrototypeOf(Int8Array.prototype).fill.call([1], 0)", "TypeError");
    throws("Int8Array.from.call(Object, [1])", "TypeError");
    throws("Int8Array.from.call({}, [1])", "TypeError");
    // The source's length is read before the constructor is checked to be a typed array's.
    throws("Object.getPrototypeOf(Int8Array).from({get length() { throw new RangeError('first'); }})", "RangeError");
    // A receiver that is not a constructor is refused before the source is read (section 22.2.2.1 step 2).
    throws("Int8Array.from.call(ArrayBuffer.isView, {get length() { throw new RangeError('first'); }})", "TypeError");
    // A constructor that is not a typed array's is refused (AllocateTypedArray, section 22.2.1.2.1),
    // a function that returns a typed array and a bound typed array constructor included.
    throws("Int8Array.from.call(function () { return new Int8Array(2); }, [1, 2])", "TypeError");
    throws("Int8Array.of.call(Int8Array.bind(null), 1)", "TypeError");
    truth("var a = new Int8Array([1, 2]); var b = new Uint8Array(a); b[0] = 9; a[0] === 1 && b.join() === '9,2'");
    // A copy to the same element type keeps every bit, a signaling NaN's included.
    truth(
        "var f = new Float32Array(2); new DataView(f.buffer).setUint32(4, 0x7f800001, true); \
         function bits(t) { return new DataView(t.buffer, t.byteOffset).getUint32(4, true); } \
         var g = new Float32Array(3); g.set(f, 1); var h = new Float32Array(2); h.set(f); \
         bits(f.slice()) === 0x7f800001 && bits(new Float32Array(f)) === 0x7f800001 && bits(h) === 0x7f800001 && \
         new DataView(g.buffer).getUint32(8, true) === 0x7f800001",
    );
    assert_eq!(string("var a = new Uint8Array([1, 2, 3, 4, 5]); a.set(a.subarray(2), 1); a.join()"), "1,3,4,5,5");
    assert_eq!(
        string("var b = new ArrayBuffer(4); var a = new Int8Array(b); a.set([1, 2]); new Uint16Array(b).set(a.subarray(0, 2)); a.join()"),
        "1,0,2,0"
    );
    assert_eq!(string("var a = new Int8Array(3); a.set(new Uint8Array([200, 1]), 1); a.join()"), "0,-56,1");
}

#[test]
fn derived_buffers_and_typed_arrays_read_the_constructor_first() {
    // SpeciesConstructor (section 7.3.20) reads `constructor` before the result is made, and
    // before filter's callback runs.
    for call in ["a.slice()", "a.subarray(0)", "a.map(function (v) { return v; })", "a.filter(function () { n++; })", "a.buffer.slice(0)"] {
        let getter = format!(
            "var n = 0; var a = new Int8Array([1, 2]); var g = {{get: function () {{ throw new RangeError('c'); }}}}; \
             Object.defineProperty(a, 'constructor', g); Object.defineProperty(a.buffer, 'constructor', g); {call}"
        );
        throws(&getter, "RangeError");
        throws(&format!("var n = 0; var a = new Int8Array([1, 2]); a.constructor = 1; a.buffer.constructor = null; {call}"), "TypeError");
    }
    truth("var n = 0; var a = new Int8Array([1, 2]); a.constructor = 1; try { a.filter(function () { n++; }); } catch (e) {} n === 0");
    truth(
        "var a = new Int8Array([1, 2]); a.constructor = undefined; a.buffer.constructor = undefined; a.slice().join() === '1,2' && a.subarray(1)[0] === 2 && a.buffer.slice(1).byteLength === 1",
    );
    truth("var a = new Int8Array([1, 2]); a.constructor = Array; a.map(function (v) { return v; }) instanceof Int8Array");
}

#[test]
fn typed_array_work_takes_fuel() {
    let fuel = Limits { fuel: 50_000, ..Limits::default() };
    assert_eq!(limit(&fuel, "new Uint8Array(1 << 20).fill(1)"), LimitKind::Fuel);
    assert_eq!(limit(&fuel, "new Uint8Array(1 << 20).copyWithin(0, 1)"), LimitKind::Fuel);
    assert_eq!(limit(&fuel, "new Uint8Array(1 << 20).slice()"), LimitKind::Fuel);
    assert_eq!(limit(&fuel, "new ArrayBuffer(1 << 20).slice(0)"), LimitKind::Fuel);
    assert_eq!(limit(&fuel, "new Uint8Array({length: 1 << 20})"), LimitKind::Fuel);
    assert_eq!(limit(&fuel, "new Float64Array(1 << 16).sort(function (a, b) { return a - b; })"), LimitKind::Fuel);
    // A sort of 2^16 equal elements makes 2^19 comparisons, and reading and writing back its
    // elements takes 2^17 units more.
    let sort = "new Uint8Array(1 << 16).sort(); 'sorted'";
    assert_eq!(limit(&Limits { fuel: 600_000, ..Limits::default() }, sort), LimitKind::Fuel);
    let mut enough = Realm::with_options(es2015(), Limits { fuel: 700_000, ..Limits::default() });
    assert_eq!(enough.run(sort).unwrap(), JsValue::String("sorted".into()));
}

#[test]
fn a_copy_between_buffers_holds_no_snapshot() {
    let mut realm = Realm::with_options(es2015(), Limits { heap_bytes: 6 << 20, ..Limits::default() });
    let r = realm.run("var a = new Uint8Array(1 << 20), b = new Int8Array(1 << 20); a[5] = 200; b.set(a); b[5]");
    assert_eq!(r.unwrap(), JsValue::Number(-56.0));
}
